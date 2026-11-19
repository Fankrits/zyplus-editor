#!/usr/bin/env python3
"""Unit tests for make-manifest.py and stamp-version.py."""

from __future__ import annotations

import base64
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest

RELEASE_DIR = Path(__file__).resolve().parent


def load_module(name: str, filename: str):
    spec = importlib.util.spec_from_file_location(name, RELEASE_DIR / filename)
    assert spec is not None and spec.loader is not None
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


make_manifest = load_module("make_manifest", "make-manifest.py")
stamp_version = load_module("stamp_version", "stamp-version.py")


def fake_sig_b64(trusted: str = "timestamp:1700000000\tfile:Zyplus.app.tar.gz") -> str:
    raw = (
        "untrusted comment: signature from minisign secret key\n"
        "RUTQBPTfjzkEPTdnAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\n"
        f"trusted comment: {trusted}\n"
        "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB==\n"
    )
    return base64.b64encode(raw.encode("utf-8")).decode("ascii")


class MakeManifestTests(unittest.TestCase):
    def test_generates_universal_macos_manifest_deterministically(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            sig_path = tmp_path / "Zyplus.app.tar.gz.sig"
            sig_b64 = fake_sig_b64()
            sig_path.write_text(sig_b64 + "\n", encoding="utf-8")

            out1 = tmp_path / "latest1.json"
            out2 = tmp_path / "latest2.json"

            args = [
                "--version",
                "v0.2.4",
                "--url-base",
                "https://github.com/Fankrits/zyplus-editor/releases/download/v0.2.4/",
                "--sig",
                str(sig_path),
                "--notes",
                "  Bug fixes and polish.  ",
                "--pub-date",
                "2026-11-19T12:00:00Z",
            ]
            self.assertEqual(make_manifest.main([*args, "--output", str(out1)]), 0)
            self.assertEqual(make_manifest.main([*args, "--output", str(out2)]), 0)
            self.assertEqual(out1.read_text("utf-8"), out2.read_text("utf-8"))

            parsed = json.loads(out1.read_text("utf-8"))
            self.assertEqual(parsed["version"], "0.2.4")
            self.assertEqual(parsed["notes"], "Bug fixes and polish.")
            self.assertEqual(parsed["pub_date"], "2026-11-19T12:00:00Z")
            self.assertEqual(
                list(parsed["platforms"].keys()),
                ["darwin-aarch64", "darwin-x86_64"],
            )
            expected_url = (
                "https://github.com/Fankrits/zyplus-editor/releases/download/"
                "v0.2.4/Zyplus.app.tar.gz"
            )
            for key in ("darwin-aarch64", "darwin-x86_64"):
                self.assertEqual(parsed["platforms"][key]["url"], expected_url)
                self.assertEqual(parsed["platforms"][key]["signature"], sig_b64)

    def test_accepts_prerelease_and_source_date_epoch(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            sig_path = tmp_path / "app.sig"
            sig_path.write_text(fake_sig_b64(), encoding="utf-8")
            notes_path = tmp_path / "notes.txt"
            notes_path.write_text("Beta release\n", encoding="utf-8")
            out = tmp_path / "latest.json"

            old_epoch = os.environ.get("SOURCE_DATE_EPOCH")
            os.environ["SOURCE_DATE_EPOCH"] = "1700000000"
            try:
                rc = make_manifest.main(
                    [
                        "--version",
                        "0.3.0-beta.1",
                        "--url-base",
                        "https://github.com/Fankrits/zyplus-editor/releases/download/v0.3.0-beta.1",
                        "--sig",
                        str(sig_path),
                        "--notes-file",
                        str(notes_path),
                        "--output",
                        str(out),
                    ]
                )
            finally:
                if old_epoch is None:
                    os.environ.pop("SOURCE_DATE_EPOCH", None)
                else:
                    os.environ["SOURCE_DATE_EPOCH"] = old_epoch

            self.assertEqual(rc, 0)
            parsed = json.loads(out.read_text("utf-8"))
            self.assertEqual(parsed["version"], "0.3.0-beta.1")
            self.assertEqual(parsed["notes"], "Beta release")
            self.assertEqual(parsed["pub_date"], "2023-11-14T22:13:20Z")

    def test_rejects_invalid_inputs(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            valid_sig = tmp_path / "valid.sig"
            valid_sig.write_text(fake_sig_b64(), encoding="utf-8")
            raw_sig = tmp_path / "raw.sig"
            raw_sig.write_text(
                "untrusted comment: not base64 wrapped\nabc\ntrusted comment: x\ndef\n",
                encoding="utf-8",
            )
            empty_sig = tmp_path / "empty.sig"
            empty_sig.write_text("   \n", encoding="utf-8")

            # Invalid versions
            for bad_ver in ("1.2", "latest", "v", "0.2.3-"):
                self.assertNotEqual(
                    make_manifest.main(
                        [
                            "--version",
                            bad_ver,
                            "--url-base",
                            "https://example.com/r",
                            "--sig",
                            str(valid_sig),
                        ]
                    ),
                    0,
                )

            # Non-HTTPS URL
            self.assertNotEqual(
                make_manifest.main(
                    [
                        "--version",
                        "0.2.4",
                        "--url-base",
                        "http://example.com/r",
                        "--sig",
                        str(valid_sig),
                    ]
                ),
                0,
            )

            # Raw (non-base64) or empty signature
            for bad_sig in (raw_sig, empty_sig, tmp_path / "missing.sig"):
                self.assertNotEqual(
                    make_manifest.main(
                        [
                            "--version",
                            "0.2.4",
                            "--url-base",
                            "https://example.com/r",
                            "--sig",
                            str(bad_sig),
                        ]
                    ),
                    0,
                )

    def test_allows_loopback_http_when_flag_is_set(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            sig_path = tmp_path / "app.sig"
            sig_path.write_text(fake_sig_b64(), encoding="utf-8")
            out = tmp_path / "latest.json"
            rc = make_manifest.main(
                [
                    "--version",
                    "0.2.4",
                    "--url-base",
                    "http://127.0.0.1:8080",
                    "--sig",
                    str(sig_path),
                    "--allow-http-localhost",
                    "--output",
                    str(out),
                ]
            )
            self.assertEqual(rc, 0)
            parsed = json.loads(out.read_text("utf-8"))
            self.assertEqual(
                parsed["platforms"]["darwin-aarch64"]["url"],
                "http://127.0.0.1:8080/Zyplus.app.tar.gz",
            )


class StampVersionTests(unittest.TestCase):
    SAMPLE_TOML = (
        '[package]\n'
        'name = "zyplus-gpui"\n'
        'version = "0.2.3"\n'
        '\n'
        '[dependencies]\n'
        'serde = { version = "1", features = ["derive"] }\n'
        '\n'
        '[package.metadata.bundle]\n'
        'name = "Zyplus"\n'
        'version = "0.2.3"\n'
    )

    SAMPLE_LOCK = (
        '[[package]]\n'
        'name = "other-pkg"\n'
        'version = "1.0.0"\n'
        '\n'
        '[[package]]\n'
        'name = "zyplus-gpui"\n'
        'version = "0.2.3"\n'
        'dependencies = []\n'
    )

    def test_stamps_only_target_sections_and_lock_entry(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "Cargo.toml").write_text(self.SAMPLE_TOML, encoding="utf-8")
            (root / "Cargo.lock").write_text(self.SAMPLE_LOCK, encoding="utf-8")

            self.assertEqual(
                stamp_version.main(["0.2.4", "--root", str(root), "--check"]),
                1,
            )
            self.assertEqual(
                stamp_version.main(["v0.2.4", "--root", str(root)]),
                0,
            )
            self.assertEqual(
                stamp_version.main(["0.2.4", "--root", str(root), "--check"]),
                0,
            )

            toml_after = (root / "Cargo.toml").read_text("utf-8")
            self.assertEqual(toml_after.count('version = "0.2.4"'), 2)
            self.assertIn('serde = { version = "1", features = ["derive"] }', toml_after)

            lock_after = (root / "Cargo.lock").read_text("utf-8")
            self.assertIn('name = "other-pkg"\nversion = "1.0.0"', lock_after)
            self.assertIn('name = "zyplus-gpui"\nversion = "0.2.4"', lock_after)

    def test_fails_when_bundle_metadata_version_missing(self) -> None:
        with self.assertRaises(ValueError):
            stamp_version.update_cargo_toml_text(
                '[package]\nname = "zyplus-gpui"\nversion = "0.2.3"\n',
                "0.2.4",
            )


if __name__ == "__main__":
    unittest.main()
