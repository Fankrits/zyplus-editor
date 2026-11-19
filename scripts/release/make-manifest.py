#!/usr/bin/env python3
"""Generate Tauri-compatible latest.json update manifest for Zyplus GPUI."""

from __future__ import annotations

import argparse
import base64
import binascii
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import re
import sys

SEMVER_RE = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$")
DEFAULT_MAC_PLATFORMS = ("darwin-aarch64", "darwin-x86_64")
DEFAULT_ASSET = "Zyplus.app.tar.gz"


def normalize_version(raw: str) -> str:
    """Strip an optional leading v/V and validate semver."""
    version = raw.strip()
    if version.startswith(("v", "V")):
        version = version[1:]
    if not SEMVER_RE.match(version):
        raise ValueError(f"Invalid release version {raw!r}; expected MAJOR.MINOR.PATCH[-pre]")
    return version


def format_pub_date(raw: str | None) -> str:
    """Return a UTC RFC3339 timestamp string."""
    if raw is not None:
        text = raw.strip()
        iso = text[:-1] + "+00:00" if text.endswith("Z") else text
        try:
            dt = datetime.fromisoformat(iso)
        except ValueError as exc:
            raise ValueError(f"Invalid --pub-date {raw!r}: {exc}") from exc
        if dt.tzinfo is None:
            raise ValueError(f"Invalid --pub-date {raw!r}: timezone offset or 'Z' is required")
        return dt.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")

    epoch_env = os.environ.get("SOURCE_DATE_EPOCH", "").strip()
    if epoch_env:
        try:
            ts = int(epoch_env)
        except ValueError as exc:
            raise ValueError(f"Invalid SOURCE_DATE_EPOCH {epoch_env!r}: {exc}") from exc
        return datetime.fromtimestamp(ts, tz=timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")

    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def validate_url(url: str, *, allow_http_localhost: bool = False) -> str:
    """Validate that an update artifact URL is HTTPS (or loopback HTTP in test mode)."""
    clean = url.strip()
    if clean.startswith("https://"):
        return clean
    if allow_http_localhost and (
        clean.startswith("http://127.0.0.1:") or clean.startswith("http://localhost:")
    ):
        return clean
    raise ValueError(f"Update URL must use https://: {url!r}")


def read_signature(sig_path: Path) -> str:
    """Read and validate a Tauri-format base64-encoded minisign signature file."""
    if not sig_path.is_file():
        raise ValueError(f"Signature file not found: {sig_path}")
    sig_b64 = sig_path.read_text(encoding="utf-8").strip()
    if not sig_b64:
        raise ValueError(f"Signature file is empty: {sig_path}")
    try:
        decoded = base64.b64decode(sig_b64, validate=True).decode("utf-8")
    except (binascii.Error, UnicodeDecodeError) as exc:
        raise ValueError(
            f"Signature file {sig_path} must be base64 of a minisign .sig text file: {exc}"
        ) from exc

    lines = [line for line in decoded.splitlines() if line != ""]
    if (
        len(lines) < 4
        or not lines[0].startswith("untrusted comment: ")
        or not lines[2].startswith("trusted comment: ")
    ):
        raise ValueError(
            f"Decoded signature in {sig_path} is not a valid minisign signature box"
        )
    return sig_b64


def parse_platform_override(spec: str) -> tuple[str, str, Path]:
    """Parse KEY=URL=SIG_PATH."""
    parts = spec.split("=", 2)
    if len(parts) != 3 or not all(parts):
        raise ValueError(f"Invalid --platform spec {spec!r}; expected KEY=URL=SIG_PATH")
    return parts[0].strip(), parts[1].strip(), Path(parts[2].strip())


def build_manifest(
    *,
    version: str,
    notes: str,
    pub_date: str,
    platforms: dict[str, dict[str, str]],
) -> dict[str, object]:
    """Assemble a deterministic manifest dict matching src/updater.rs."""
    if not platforms:
        raise ValueError("Manifest must contain at least one platform entry")
    ordered_platforms = {
        key: {
            "signature": platforms[key]["signature"],
            "url": platforms[key]["url"],
        }
        for key in sorted(platforms)
    }
    return {
        "version": version,
        "notes": notes,
        "pub_date": pub_date,
        "platforms": ordered_platforms,
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Generate latest.json update manifest compatible with Zyplus src/updater.rs."
    )
    parser.add_argument("--version", required=True, help="Release semver version (e.g. 0.2.4 or v0.2.4).")
    parser.add_argument(
        "--url-base",
        help="Base download URL (e.g. https://github.com/Fankrits/zyplus-editor/releases/download/v0.2.4).",
    )
    parser.add_argument(
        "--asset",
        default=DEFAULT_ASSET,
        help=f"Universal macOS archive filename under --url-base (default: {DEFAULT_ASSET}).",
    )
    parser.add_argument(
        "--sig",
        type=Path,
        help="Path to base64-encoded minisign signature file for --asset.",
    )
    parser.add_argument(
        "--platforms",
        default=",".join(DEFAULT_MAC_PLATFORMS),
        help="Comma-separated platform keys populated from --url-base/--asset/--sig.",
    )
    parser.add_argument(
        "--platform",
        action="append",
        default=[],
        metavar="KEY=URL=SIG_PATH",
        help="Explicit platform entry override (repeatable).",
    )
    notes_group = parser.add_mutually_exclusive_group()
    notes_group.add_argument("--notes", default="", help="Release notes text.")
    notes_group.add_argument("--notes-file", type=Path, help="Read release notes from UTF-8 file.")
    parser.add_argument("--pub-date", help="Explicit RFC3339 publication timestamp.")
    parser.add_argument(
        "--allow-http-localhost",
        action="store_true",
        help="Allow http://127.0.0.1: or http://localhost: URLs for local fixture tests.",
    )
    parser.add_argument(
        "--output",
        default="-",
        help="Output path for latest.json, or '-' for stdout (default: '-').",
    )
    args = parser.parse_args(argv)

    try:
        version = normalize_version(args.version)
        pub_date = format_pub_date(args.pub_date)
        notes = (
            args.notes_file.read_text(encoding="utf-8")
            if args.notes_file is not None
            else args.notes
        ).strip()

        platforms: dict[str, dict[str, str]] = {}
        if args.sig is not None or args.url_base is not None:
            if args.sig is None or args.url_base is None:
                raise ValueError("--url-base and --sig must be provided together")
            base = args.url_base.rstrip("/")
            url = validate_url(
                f"{base}/{args.asset.lstrip('/')}",
                allow_http_localhost=args.allow_http_localhost,
            )
            signature = read_signature(args.sig)
            for key in (item.strip() for item in args.platforms.split(",") if item.strip()):
                platforms[key] = {"url": url, "signature": signature}

        for spec in args.platform:
            key, raw_url, sig_path = parse_platform_override(spec)
            url = validate_url(raw_url, allow_http_localhost=args.allow_http_localhost)
            signature = read_signature(sig_path)
            platforms[key] = {"url": url, "signature": signature}

        manifest = build_manifest(
            version=version,
            notes=notes,
            pub_date=pub_date,
            platforms=platforms,
        )
        rendered = json.dumps(manifest, indent=2, ensure_ascii=False) + "\n"
    except (OSError, ValueError) as exc:
        print(f"make-manifest.py: {exc}", file=sys.stderr)
        return 1

    if args.output == "-":
        sys.stdout.write(rendered)
    else:
        out_path = Path(args.output)
        out_path.parent.mkdir(parents=True, exist_ok=True)
        tmp_path = out_path.with_name(f".{out_path.name}.tmp.{os.getpid()}")
        tmp_path.write_text(rendered, encoding="utf-8")
        tmp_path.replace(out_path)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
