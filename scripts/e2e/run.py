#!/usr/bin/env python3
"""End-to-end test runner for Zyplus GPUI using ZYPLUS_TEST_SCRIPT."""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import xml.etree.ElementTree as ET
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[2]
E2E_ROOT = Path(__file__).resolve().parent
FIXTURES_DIR = E2E_ROOT / "fixtures"
SCENARIOS_DIR = E2E_ROOT / "scenarios"


@dataclass
class ScenarioResult:
    name: str
    description: str
    status: str  # PASS, FAIL, XFAIL, XPASS, SKIP
    duration_s: float
    message: str = ""
    xfail_reason: str | None = None


def current_platform() -> str:
    if sys.platform == "darwin":
        return "macos"
    if sys.platform.startswith("linux"):
        return "linux"
    if sys.platform == "win32":
        return "windows"
    return sys.platform


def app_data_for(platform: str, home_dir: Path) -> tuple[Path, dict[str, str]]:
    """Matches src/paths.rs::app_data_dir_for and returns isolation env vars."""
    if platform == "macos":
        return home_dir / "Library" / "Application Support", {}
    if platform == "windows":
        data = home_dir / "AppData" / "Roaming"
        return data, {"APPDATA": str(data), "USERPROFILE": str(home_dir)}
    data = home_dir / ".local" / "share"
    return data, {"XDG_DATA_HOME": str(data)}


def parse_state_file(path: Path) -> dict[str, str]:
    data: dict[str, str] = {}
    for raw in path.read_text(encoding="utf-8").splitlines():
        if "=" in raw:
            k, v = raw.split("=", 1)
            data[k] = v
    return data


def substitute(text: str, vars_map: dict[str, str]) -> str:
    for k, v in vars_map.items():
        text = text.replace("{" + k + "}", v)
    return text


def substitute_obj(obj: Any, vars_map: dict[str, str]) -> Any:
    if isinstance(obj, str):
        return substitute(obj, vars_map)
    if isinstance(obj, list):
        return [substitute_obj(item, vars_map) for item in obj]
    if isinstance(obj, dict):
        return {k: substitute_obj(v, vars_map) for k, v in obj.items()}
    return obj


def prepare_script(
    template_path: Path,
    dest_path: Path,
    vars_map: dict[str, str],
    shots_enabled: bool,
    shots_dir: Path,
) -> None:
    lines: list[str] = []
    shot_idx = 0
    for raw in template_path.read_text(encoding="utf-8").splitlines():
        stripped = raw.strip()
        if stripped.startswith("shot "):
            if not shots_enabled:
                continue
            shots_dir.mkdir(parents=True, exist_ok=True)
            shot_arg = substitute(stripped[5:].strip(), vars_map)
            if not os.path.isabs(shot_arg):
                shot_idx += 1
                shot_arg = str(shots_dir / f"{shot_idx:02d}_{shot_arg}")
            lines.append(f"shot {shot_arg}")
        else:
            lines.append(substitute(raw, vars_map))
    dest_path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def write_open_shim(bin_dir: Path, home_dir: Path, tmp_dir: Path) -> None:
    bin_dir.mkdir(parents=True, exist_ok=True)
    shim = bin_dir / "open"
    shim.write_text(
        "#!/bin/sh\n"
        "unset ZYPLUS_TEST_SCRIPT ZYPLUS_TEST_WINID\n"
        f'exec /usr/bin/open --env HOME="{home_dir}" --env TMPDIR="{tmp_dir}" "$@"\n',
        encoding="utf-8",
    )
    shim.chmod(0o755)


def run_app_once(
    binary: Path,
    home_dir: Path,
    tmp_dir: Path,
    shim_bin_dir: Path | None,
    script_path: Path | None,
    cli_args: list[str],
    log_path: Path,
    timeout_s: int,
    winid_bin: Path | None,
    extra_env: dict[str, str] | None = None,
    second_launch: dict[str, Any] | None = None,
    vars_map: dict[str, str] | None = None,
) -> tuple[int, str, list[str]]:
    env = os.environ.copy()
    env["HOME"] = str(home_dir)
    env["TMPDIR"] = str(tmp_dir)
    if extra_env:
        env.update(extra_env)
    if shim_bin_dir is not None:
        env["PATH"] = f"{shim_bin_dir}:{env.get('PATH', '')}"
    if script_path is not None:
        env["ZYPLUS_TEST_SCRIPT"] = str(script_path)
    else:
        env.pop("ZYPLUS_TEST_SCRIPT", None)
    if winid_bin is not None:
        env["ZYPLUS_TEST_WINID"] = str(winid_bin)
    else:
        env.pop("ZYPLUS_TEST_WINID", None)

    cmd = ["perl", "-e", "alarm shift; exec @ARGV", str(timeout_s), str(binary), *cli_args]
    extra_errors: list[str] = []

    log_file = log_path.open("w", encoding="utf-8")
    proc = subprocess.Popen(
        cmd,
        cwd=str(home_dir),
        env=env,
        stdout=log_file,
        stderr=subprocess.STDOUT,
    )
    try:
        if second_launch is not None:
            delay_s = float(second_launch.get("delay_ms", 1200)) / 1000.0
            time.sleep(delay_s)
            if proc.poll() is not None:
                extra_errors.append(
                    f"primary exited early ({proc.returncode}) before second launch"
                )
            else:
                raw_args = second_launch.get("args", [])
                s_args = [substitute(a, vars_map or {}) for a in raw_args]
                env2 = env.copy()
                env2.pop("ZYPLUS_TEST_SCRIPT", None)
                env2.pop("ZYPLUS_TEST_WINID", None)
                s_cmd = [
                    "perl",
                    "-e",
                    "alarm shift; exec @ARGV",
                    "15",
                    str(binary),
                    *s_args,
                ]
                res2 = subprocess.run(
                    s_cmd,
                    cwd=str(home_dir),
                    env=env2,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.STDOUT,
                    text=True,
                )
                expected_exit = int(second_launch.get("expect_exit", 0))
                if res2.returncode != expected_exit:
                    extra_errors.append(
                        f"second launch exited {res2.returncode} != {expected_exit}: {res2.stdout.strip()}"
                    )

        rc = proc.wait(timeout=timeout_s + 5)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait()
        extra_errors.append(f"timed out after {timeout_s + 5}s (killed pid {proc.pid})")
        rc = -1
    finally:
        if proc.poll() is None:
            proc.kill()
            proc.wait()
        log_file.close()

    log_text = log_path.read_text(encoding="utf-8", errors="replace")
    return rc, log_text, extra_errors


def evaluate_checks(
    checks: list[dict[str, Any]],
    vars_map: dict[str, str],
    log_text: str,
) -> list[str]:
    errors: list[str] = []
    out_dir = Path(vars_map["out"])

    for chk in checks:
        ctype = chk["type"]
        if ctype == "state_equals":
            state_path = out_dir / chk["file"]
            if not state_path.is_file():
                errors.append(f"missing state file: {state_path}")
                continue
            actual = parse_state_file(state_path)
            expected: dict[str, str] = substitute_obj(chk["expected"], vars_map)
            for k, exp_val in expected.items():
                act_val = actual.get(k)
                if k == "modal" and exp_val in ("", "none"):
                    if act_val not in ("", "none", None):
                        errors.append(f"{chk['file']} modal={act_val!r}, expected closed")
                elif act_val != exp_val:
                    errors.append(
                        f"{chk['file']} {k}={act_val!r}, expected {exp_val!r}"
                    )
        elif ctype == "state_selection_bytes":
            state_path = out_dir / chk["file"]
            if not state_path.is_file():
                errors.append(f"missing state file: {state_path}")
                continue
            actual = parse_state_file(state_path)
            expected_len = len(chk["text"].encode("utf-8"))
            expected_sel = f"{expected_len}..{expected_len}"
            if actual.get("selection") != expected_sel:
                errors.append(
                    f"{chk['file']} selection={actual.get('selection')!r}, expected {expected_sel!r}"
                )
        elif ctype == "file_exists":
            p = Path(substitute(chk["path"], vars_map))
            if not p.is_file():
                errors.append(f"expected file to exist: {p}")
        elif ctype == "file_not_exists":
            p = Path(substitute(chk["path"], vars_map))
            if p.exists():
                errors.append(f"expected path not to exist: {p}")
        elif ctype == "dir_exists":
            p = Path(substitute(chk["path"], vars_map))
            if not p.is_dir():
                errors.append(f"expected directory to exist: {p}")
        elif ctype == "file_equals":
            p = Path(substitute(chk["path"], vars_map))
            if not p.is_file():
                errors.append(f"missing file for equality check: {p}")
                continue
            actual_text = p.read_text(encoding="utf-8")
            expected_text = substitute(chk["expected"], vars_map)
            if actual_text != expected_text:
                errors.append(
                    f"{p.name} content mismatch:\n  actual:   {actual_text!r}\n  expected: {expected_text!r}"
                )
        elif ctype == "file_contains":
            p = Path(substitute(chk["path"], vars_map))
            if not p.is_file():
                errors.append(f"missing file for substring check: {p}")
                continue
            actual_text = p.read_text(encoding="utf-8")
            substr = substitute(chk["substr"], vars_map)
            if substr not in actual_text:
                errors.append(
                    f"{p.name} missing {substr!r}; got {actual_text!r}"
                )
        elif ctype == "file_not_contains":
            p = Path(substitute(chk["path"], vars_map))
            if not p.is_file():
                errors.append(f"missing file for negative substring check: {p}")
                continue
            actual_text = p.read_text(encoding="utf-8")
            substr = substitute(chk["substr"], vars_map)
            if substr in actual_text:
                errors.append(f"{p.name} unexpectedly contains {substr!r}")
        elif ctype == "json_valid":
            p = Path(substitute(chk["path"], vars_map))
            if not p.is_file():
                errors.append(f"missing JSON file: {p}")
                continue
            try:
                parsed = json.loads(p.read_text(encoding="utf-8"))
            except json.JSONDecodeError as err:
                errors.append(f"invalid JSON in {p}: {err}")
                continue
            for key in chk.get("expected_keys", []):
                if not isinstance(parsed, dict) or key not in parsed:
                    errors.append(f"JSON {p.name} missing key {key!r}")
        elif ctype == "log_not_contains":
            substr = chk["substr"]
            if substr in log_text:
                errors.append(f"stderr log unexpectedly contains {substr!r}")
        else:
            errors.append(f"unknown check type: {ctype!r}")

    return errors


def discover_scenarios(filter_substr: str | None) -> list[Path]:
    if not SCENARIOS_DIR.is_dir():
        return []
    dirs = sorted(
        d for d in SCENARIOS_DIR.iterdir() if d.is_dir() and (d / "spec.json").is_file()
    )
    if filter_substr:
        dirs = [d for d in dirs if filter_substr in d.name]
    return dirs


def run_scenario(
    idx: int,
    scenario_dir: Path,
    session_root: Path,
    binary: Path,
    shots_enabled: bool,
    winid_bin: Path | None,
    keep: bool,
    clipboard_lock: threading.Lock,
) -> ScenarioResult:
    t0 = time.monotonic()
    spec = json.loads((scenario_dir / "spec.json").read_text(encoding="utf-8"))
    name = scenario_dir.name
    description = spec.get("description", "")
    xfail_reason = spec.get("xfail")
    platform = current_platform()

    platforms = spec.get("platforms")
    if platforms is not None and platform not in platforms:
        return ScenarioResult(
            name=name,
            description=description,
            status="SKIP",
            duration_s=0.0,
            message=f"only runs on {', '.join(platforms)} (current: {platform})",
        )

    # Keep scratch path short so $TMPDIR/zyplus-gpui-<user>.sock stays < 100 bytes.
    run_dir = session_root / f"s{idx}"
    home_dir = run_dir / "h"
    tmp_dir = run_dir / "t"
    ws_dir = run_dir / "w"
    out_dir = run_dir / "o"
    shots_dir = out_dir / "shots"
    app_data, extra_env = app_data_for(platform, home_dir)

    for d in (home_dir, tmp_dir, ws_dir, out_dir, app_data):
        d.mkdir(parents=True, exist_ok=True)

    shim_dir: Path | None = None
    if platform == "macos":
        shim_dir = run_dir / "b"
        write_open_shim(shim_dir, home_dir, tmp_dir)

    vars_map = {
        "home": str(home_dir),
        "tmp": str(tmp_dir),
        "ws": str(ws_dir),
        "out": str(out_dir),
        "app_data": str(app_data),
        "app_support": str(app_data),
    }

    # Copy fixtures into workspace.
    for fix in spec.get("fixtures", []):
        src = FIXTURES_DIR / fix["src"]
        dst_rel = fix.get("dst", fix["src"])
        dst = ws_dir if dst_rel in ("", ".") else ws_dir / dst_rel
        if src.is_dir():
            shutil.copytree(src, dst, dirs_exist_ok=True)
        else:
            dst.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(src, dst)

    # Optional pre-created files (e.g., blocking Zyplus dir for welcome fallback).
    for item in spec.get("pre_create_files", []):
        p = Path(substitute(item["path"], vars_map))
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(substitute(item.get("content", ""), vars_map), encoding="utf-8")

    # Optional initial session.json.
    session_template = scenario_dir / "session.json"
    if session_template.is_file():
        session_dir = app_data / "Zyplus-session"
        session_dir.mkdir(parents=True, exist_ok=True)
        session_text = substitute(
            session_template.read_text(encoding="utf-8"), vars_map
        )
        (session_dir / "session.json").write_text(session_text, encoding="utf-8")

    script_file = run_dir / "script.txt"
    prepare_script(
        scenario_dir / "script.txt",
        script_file,
        vars_map,
        shots_enabled,
        shots_dir,
    )

    cli_args = [substitute(a, vars_map) for a in spec.get("cli_args", [])]
    timeout_s = int(spec.get("timeout_s", 60))
    uses_clipboard = bool(spec.get("uses_clipboard", False))

    def _run_all() -> list[str]:
        errs: list[str] = []
        rc, log_text, extra = run_app_once(
            binary=binary,
            home_dir=home_dir,
            tmp_dir=tmp_dir,
            shim_bin_dir=shim_dir,
            script_path=script_file,
            cli_args=cli_args,
            log_path=out_dir / "run1.log",
            timeout_s=timeout_s,
            winid_bin=winid_bin,
            extra_env=extra_env,
            second_launch=spec.get("second_launch"),
            vars_map=vars_map,
        )
        errs.extend(extra)
        expected_rc = int(spec.get("expect_exit", 0))
        if rc != expected_rc:
            errs.append(f"exit code {rc} != {expected_rc}")
        if "panicked at" in log_text:
            errs.append("app panicked (see run1.log)")
        errs.extend(evaluate_checks(spec.get("checks", []), vars_map, log_text))

        relaunch_template = scenario_dir / "relaunch.txt"
        if relaunch_template.is_file() and not errs:
            relaunch_script = run_dir / "relaunch.txt"
            prepare_script(
                relaunch_template,
                relaunch_script,
                vars_map,
                shots_enabled,
                shots_dir,
            )
            rc2, log2, extra2 = run_app_once(
                binary=binary,
                home_dir=home_dir,
                tmp_dir=tmp_dir,
                shim_bin_dir=shim_dir,
                script_path=relaunch_script,
                cli_args=[],
                log_path=out_dir / "run2.log",
                timeout_s=timeout_s,
                winid_bin=winid_bin,
                extra_env=extra_env,
            )
            errs.extend(extra2)
            if rc2 != 0:
                errs.append(f"relaunch exit code {rc2} != 0")
            if "panicked at" in log2:
                errs.append("app panicked on relaunch (see run2.log)")
            errs.extend(
                evaluate_checks(spec.get("relaunch_checks", []), vars_map, log2)
            )
        return errs

    if uses_clipboard:
        with clipboard_lock:
            errors = _run_all()
    else:
        errors = _run_all()

    duration = time.monotonic() - t0
    if errors:
        if xfail_reason:
            status = "XFAIL"
            msg = f"{xfail_reason} ({'; '.join(errors)})"
        else:
            status = "FAIL"
            msg = "; ".join(errors) + f" [scratch: {run_dir}]"
    else:
        if xfail_reason:
            status = "XPASS"
            msg = f"unexpected pass (xfail was: {xfail_reason})"
        else:
            status = "PASS"
            msg = ""

    if not keep and status in ("PASS", "XFAIL"):
        shutil.rmtree(run_dir, ignore_errors=True)

    return ScenarioResult(
        name=name,
        description=description,
        status=status,
        duration_s=duration,
        message=msg,
        xfail_reason=xfail_reason,
    )


def write_report(report_path: Path, results: list[ScenarioResult], total_s: float) -> None:
    report_path.parent.mkdir(parents=True, exist_ok=True)
    if report_path.suffix.lower() == ".xml":
        failures = sum(1 for r in results if r.status in ("FAIL", "XPASS"))
        skipped = sum(1 for r in results if r.status in ("XFAIL", "SKIP"))
        suite = ET.Element(
            "testsuite",
            name="zyplus-e2e",
            tests=str(len(results)),
            failures=str(failures),
            skipped=str(skipped),
            time=f"{total_s:.3f}",
        )
        for r in results:
            tc = ET.SubElement(
                suite, "testcase", name=r.name, time=f"{r.duration_s:.3f}"
            )
            if r.status in ("FAIL", "XPASS"):
                fail = ET.SubElement(tc, "failure", message=r.status)
                fail.text = r.message
            elif r.status == "XFAIL":
                sk = ET.SubElement(tc, "skipped", message=r.xfail_reason or "xfail")
                sk.text = r.message
            elif r.status == "SKIP":
                sk = ET.SubElement(tc, "skipped", message=r.message or "skipped")
                sk.text = r.message
        tree = ET.ElementTree(suite)
        ET.indent(tree, space="  ")
        tree.write(report_path, encoding="utf-8", xml_declaration=True)
    else:
        payload = {
            "platform": current_platform(),
            "total": len(results),
            "passed": sum(1 for r in results if r.status == "PASS"),
            "failed": sum(1 for r in results if r.status in ("FAIL", "XPASS")),
            "xfailed": sum(1 for r in results if r.status == "XFAIL"),
            "skipped": sum(1 for r in results if r.status == "SKIP"),
            "duration_s": round(total_s, 3),
            "scenarios": [
                {
                    "name": r.name,
                    "description": r.description,
                    "status": r.status,
                    "duration_s": round(r.duration_s, 3),
                    "message": r.message,
                    "xfail_reason": r.xfail_reason,
                }
                for r in results
            ],
        }
        report_path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")


def main() -> int:
    default_target = Path(os.environ.get("CARGO_TARGET_DIR") or (REPO_ROOT / "target"))
    default_bin = default_target / "debug" / "zyplus-gpui"

    parser = argparse.ArgumentParser(description="Run Zyplus GPUI end-to-end scenarios.")
    parser.add_argument("--filter", help="Only run scenarios matching this substring.")
    parser.add_argument("--list", action="store_true", help="List available scenarios.")
    parser.add_argument("--keep", action="store_true", help="Keep scratch directories.")
    parser.add_argument(
        "--bin",
        type=Path,
        default=default_bin,
        help=f"Path to debug zyplus-gpui binary (default: {default_bin})",
    )
    parser.add_argument(
        "--jobs",
        type=int,
        default=1,
        help="Parallel worker count (default: 1; clipboard scenarios always serialize).",
    )
    parser.add_argument(
        "--shots",
        action="store_true",
        help="Enable window screenshots via tools/winid.swift (opt-in).",
    )
    parser.add_argument(
        "--report",
        type=Path,
        help="Write summary report (.json or JUnit .xml).",
    )
    args = parser.parse_args()

    scenarios = discover_scenarios(args.filter)
    if args.list:
        for s in scenarios:
            spec = json.loads((s / "spec.json").read_text(encoding="utf-8"))
            xf = f" [xfail: {spec['xfail']}]" if spec.get("xfail") else ""
            plat = (
                f" [platforms: {', '.join(spec['platforms'])}]"
                if spec.get("platforms")
                else ""
            )
            print(f"{s.name:<34} {spec.get('description', '')}{plat}{xf}")
        return 0

    if not scenarios:
        print("No scenarios matched.", file=sys.stderr)
        return 1

    src_bin = args.bin.resolve()
    if not src_bin.is_file() or not os.access(src_bin, os.X_OK):
        print(f"Debug binary not found or not executable: {src_bin}", file=sys.stderr)
        return 1

    # Use realpath so macOS /var -> /private/var matches rust canonicalize().
    base_tmp = Path(os.path.realpath(os.environ.get("TMPDIR") or tempfile.gettempdir()))
    session_root = base_tmp / f"z{os.getpid()}"
    session_root.mkdir(parents=True, exist_ok=True)

    # Copy binary cleanly (`rm -f dst && cp src dst`) to avoid SIGKILL on overwrite.
    copied_bin = session_root / "zyplus-gpui"
    if copied_bin.exists():
        copied_bin.unlink()
    shutil.copy2(src_bin, copied_bin)
    copied_bin.chmod(0o755)

    platform = current_platform()
    shots_enabled = args.shots
    winid_bin: Path | None = None
    if shots_enabled and platform != "macos":
        print(
            f"Note: --shots is macOS-only (uses tools/winid.swift and screencapture); "
            f"ignoring --shots on {platform}.",
            file=sys.stderr,
        )
        shots_enabled = False
    elif shots_enabled:
        winid_src = REPO_ROOT / "tools" / "winid.swift"
        if winid_src.is_file():
            winid_bin = session_root / "winid"
            res = subprocess.run(
                ["swiftc", "-O", str(winid_src), "-o", str(winid_bin)],
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
            )
            if res.returncode != 0:
                print(f"Warning: could not compile winid.swift: {res.stdout}", file=sys.stderr)
                winid_bin = None

    clipboard_lock = threading.Lock()
    t_start = time.monotonic()
    results: list[ScenarioResult] = []

    jobs = max(1, args.jobs)
    if jobs == 1:
        for i, sc in enumerate(scenarios, start=1):
            res = run_scenario(
                i, sc, session_root, copied_bin, shots_enabled, winid_bin, args.keep, clipboard_lock
            )
            results.append(res)
    else:
        with concurrent.futures.ThreadPoolExecutor(max_workers=jobs) as pool:
            futs = [
                pool.submit(
                    run_scenario,
                    i,
                    sc,
                    session_root,
                    copied_bin,
                    shots_enabled,
                    winid_bin,
                    args.keep,
                    clipboard_lock,
                )
                for i, sc in enumerate(scenarios, start=1)
            ]
            for fut in futs:
                results.append(fut.result())

    total_s = time.monotonic() - t_start

    print("\n=== Zyplus E2E Results ===")
    print(f"{'Scenario':<34} {'Status':<7} {'Time':>7}  Details")
    print("-" * 78)
    for r in results:
        print(f"{r.name:<34} {r.status:<7} {r.duration_s:>6.2f}s  {r.message}")
    print("-" * 78)

    passed = sum(1 for r in results if r.status == "PASS")
    failed = sum(1 for r in results if r.status in ("FAIL", "XPASS"))
    xfailed = sum(1 for r in results if r.status == "XFAIL")
    skipped = sum(1 for r in results if r.status == "SKIP")
    skip_part = f" | SKIP: {skipped}" if skipped else ""
    print(
        f"Total: {len(results)} | PASS: {passed} | FAIL: {failed} | XFAIL: {xfailed}{skip_part} | Time: {total_s:.2f}s"
    )

    if args.report:
        write_report(args.report, results, total_s)

    if not args.keep and failed == 0:
        shutil.rmtree(session_root, ignore_errors=True)
    elif args.keep or failed > 0:
        print(f"Scratch dir retained: {session_root}")

    return 1 if failed > 0 else 0


if __name__ == "__main__":
    sys.exit(main())
