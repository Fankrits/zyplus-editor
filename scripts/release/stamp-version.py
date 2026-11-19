#!/usr/bin/env python3
"""Stamp a semver release version into Cargo.toml and Cargo.lock."""

from __future__ import annotations

import argparse
import os
import re
import sys
from pathlib import Path

SEMVER_RE = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$")
CARGO_TOML_VERSION_RE = re.compile(r'^(version\s*=\s*)"[^"]*"(.*)$')
CARGO_LOCK_PKG_NAME = "zyplus-gpui"


def normalize_version(raw: str) -> str:
    """Strip an optional leading v/V and validate semver."""
    version = raw.strip()
    if version.startswith(("v", "V")):
        version = version[1:]
    if not SEMVER_RE.match(version):
        raise ValueError(f"Not a valid semver version: {raw!r}")
    return version


def update_cargo_toml_text(text: str, version: str) -> tuple[str, list[str]]:
    """Update version in [package] and [package.metadata.bundle]."""
    target_sections = {"[package]", "[package.metadata.bundle]"}
    updated_sections: list[str] = []
    current_section: str | None = None
    out_lines: list[str] = []

    for line in text.splitlines(keepends=True):
        stripped = line.strip()
        if stripped.startswith("[") and not stripped.startswith("[["):
            current_section = stripped.split("#", 1)[0].strip()
        elif (
            current_section in target_sections
            and current_section not in updated_sections
        ):
            match = CARGO_TOML_VERSION_RE.match(line.rstrip("\r\n"))
            if match:
                newline = "\r\n" if line.endswith("\r\n") else ("\n" if line.endswith("\n") else "")
                line = f'{match.group(1)}"{version}"{match.group(2)}{newline}'
                updated_sections.append(current_section)
        out_lines.append(line)

    missing = [sec for sec in ("[package]", "[package.metadata.bundle]") if sec not in updated_sections]
    if missing:
        raise ValueError(f"Cargo.toml is missing version field in section(s): {', '.join(missing)}")

    return "".join(out_lines), updated_sections


def update_cargo_lock_text(text: str, version: str, package_name: str = CARGO_LOCK_PKG_NAME) -> str:
    """Update the version line for the target [[package]] entry in Cargo.lock."""
    in_package = False
    matched_package = False
    updated_count = 0
    out_lines: list[str] = []

    for line in text.splitlines(keepends=True):
        stripped = line.strip()
        if stripped == "[[package]]":
            in_package = True
            matched_package = False
        elif stripped.startswith("[") and stripped != "[[package]]":
            in_package = False
            matched_package = False
        elif in_package:
            if stripped == f'name = "{package_name}"':
                matched_package = True
            elif matched_package:
                match = CARGO_TOML_VERSION_RE.match(line.rstrip("\r\n"))
                if match:
                    newline = "\r\n" if line.endswith("\r\n") else ("\n" if line.endswith("\n") else "")
                    line = f'{match.group(1)}"{version}"{match.group(2)}{newline}'
                    updated_count += 1
                    matched_package = False
        out_lines.append(line)

    if updated_count != 1:
        raise ValueError(
            f"Expected exactly 1 {package_name!r} entry in Cargo.lock, updated {updated_count}"
        )

    return "".join(out_lines)


def write_atomic(path: Path, content: str) -> None:
    """Write text atomically when replacing an existing file."""
    tmp = path.with_name(f".{path.name}.tmp.{os.getpid()}")
    tmp.write_text(content, encoding="utf-8")
    tmp.replace(path)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Stamp release version into Cargo.toml and Cargo.lock.")
    parser.add_argument("version", help="Release version (with or without leading 'v').")
    parser.add_argument(
        "--root",
        type=Path,
        default=Path(__file__).resolve().parents[2],
        help="Repository root containing Cargo.toml and Cargo.lock.",
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="Verify files already match the target version without modifying them.",
    )
    args = parser.parse_args(argv)

    try:
        version = normalize_version(args.version)
        cargo_toml = args.root / "Cargo.toml"
        cargo_lock = args.root / "Cargo.lock"

        toml_before = cargo_toml.read_text(encoding="utf-8")
        toml_after, _ = update_cargo_toml_text(toml_before, version)

        lock_before = cargo_lock.read_text(encoding="utf-8")
        lock_after = update_cargo_lock_text(lock_before, version)
    except (OSError, ValueError) as exc:
        print(f"stamp-version.py: {exc}", file=sys.stderr)
        return 1

    if args.check:
        if toml_before != toml_after or lock_before != lock_after:
            print(f"stamp-version.py: files are not at version {version}", file=sys.stderr)
            return 1
        print(f"Verified Cargo.toml and Cargo.lock are at {version}")
        return 0

    if toml_before != toml_after:
        write_atomic(cargo_toml, toml_after)
    if lock_before != lock_after:
        write_atomic(cargo_lock, lock_after)

    print(f"Stamped version {version} into Cargo.toml and Cargo.lock")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
