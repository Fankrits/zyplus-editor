#!/usr/bin/env bash
#
# Unified test and verification runner for Zyplus (GPUI).
#
# Usage:
#   scripts/test.sh <stage> [e2e-args...]
#   scripts/test.sh --help
#
# Stages:
#   fmt     Check Rust formatting (cargo fmt --check)
#   lint    Run clippy across all targets (cargo clippy --all-targets -- -D warnings)
#   unit    Run Rust unit tests (cargo test)
#   build   Build release binary and verify it is executable
#   bundle  Assemble Zyplus.app and verify Info.plist + codesign (macOS only)
#   e2e     Build debug binary and run scripts/e2e/run.py (macOS only)
#   quick   Run: fmt lint unit
#   all     Run: fmt lint unit build bundle e2e

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

TARGET_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/target}"
OS_NAME="$(uname -s)"

if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
  C_RESET=$'\033[0m'
  C_BOLD=$'\033[1m'
  C_CYAN=$'\033[36m'
  C_GREEN=$'\033[32m'
  C_RED=$'\033[31m'
  C_YELLOW=$'\033[33m'
else
  C_RESET=""
  C_BOLD=""
  C_CYAN=""
  C_GREEN=""
  C_RED=""
  C_YELLOW=""
fi

usage() {
  cat <<'EOF'
Usage: scripts/test.sh <stage>... [-- <e2e-args...>]
       scripts/test.sh --help

Stages:
  fmt     Check Rust formatting (cargo fmt --check)
  lint    Run clippy across all targets (cargo clippy --all-targets -- -D warnings)
  unit    Run Rust unit tests (cargo test)
  build   Build release binary and verify it exists/is executable
  bundle  Run scripts/bundle-macos.sh and verify Info.plist & codesign (macOS only)
  e2e     Build debug binary and run scripts/e2e/run.py (macOS only)
  quick   Run: fmt lint unit
  all     Run: fmt lint unit build bundle e2e

Environment:
  CARGO_TARGET_DIR   Override cargo target directory (default: ./target)
  NO_COLOR           Disable ANSI colors when set
EOF
}

if [ $# -eq 0 ]; then
  usage >&2
  exit 2
fi

STAGES=()
E2E_ARGS=()
PARSING_E2E_ARGS=0

for arg in "$@"; do
  if [ "$PARSING_E2E_ARGS" -eq 1 ]; then
    E2E_ARGS+=("$arg")
    continue
  fi
  case "$arg" in
    -h|--help)
      usage
      exit 0
      ;;
    --)
      PARSING_E2E_ARGS=1
      ;;
    -*)
      # Allow passing --filter / --keep / etc. directly after `e2e`.
      if [ "${#STAGES[@]}" -gt 0 ] && [ "${STAGES[${#STAGES[@]}-1]}" = "e2e" ]; then
        E2E_ARGS+=("$arg")
        PARSING_E2E_ARGS=1
      else
        echo "Unknown option: $arg" >&2
        usage >&2
        exit 2
      fi
      ;;
    quick)
      STAGES+=(fmt lint unit)
      ;;
    all)
      STAGES+=(fmt lint unit build bundle e2e)
      ;;
    fmt|lint|unit|build|bundle|e2e)
      STAGES+=("$arg")
      ;;
    *)
      echo "Unknown stage: $arg" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [ "${#STAGES[@]}" -eq 0 ]; then
  usage >&2
  exit 2
fi

RESULTS=()
OVERALL_STATUS=0

header() {
  local stage="$1"
  local desc="$2"
  printf '\n%s==> [%s] %s%s\n' "${C_BOLD}${C_CYAN}" "$stage" "$desc" "$C_RESET"
}

record_result() {
  local stage="$1"
  local status="$2"
  RESULTS+=("$stage:$status")
  if [ "$status" = "FAIL" ]; then
    OVERALL_STATUS=1
  fi
}

run_fmt() {
  header "fmt" "cargo fmt --check"
  if cargo fmt --check; then
    record_result "fmt" "PASS"
  else
    record_result "fmt" "FAIL"
  fi
}

run_lint() {
  header "lint" "cargo clippy --all-targets -- -D warnings"
  if cargo clippy --all-targets -- -D warnings; then
    record_result "lint" "PASS"
  else
    record_result "lint" "FAIL"
  fi
}

run_unit() {
  header "unit" "cargo test"
  if cargo test; then
    record_result "unit" "PASS"
  else
    record_result "unit" "FAIL"
  fi
}

run_build() {
  header "build" "cargo build --release"
  if ! cargo build --release; then
    record_result "build" "FAIL"
    return
  fi
  local bin="$TARGET_DIR/release/zyplus-gpui"
  if [ "$OS_NAME" != "Darwin" ] && [ "$OS_NAME" != "Linux" ] && [ -f "${bin}.exe" ]; then
    bin="${bin}.exe"
  fi
  if [ -f "$bin" ] && [ -x "$bin" ]; then
    echo "Verified release binary: $bin"
    ls -lh "$bin"
    record_result "build" "PASS"
  else
    echo "Release binary missing or not executable: $bin" >&2
    record_result "build" "FAIL"
  fi
}

run_bundle() {
  header "bundle" "macOS app bundle & verification"
  if [ "$OS_NAME" != "Darwin" ]; then
    echo "SKIP: bundle stage only runs on macOS (current OS: $OS_NAME)"
    record_result "bundle" "SKIP"
    return
  fi

  if ! "$REPO_ROOT/scripts/bundle-macos.sh"; then
    record_result "bundle" "FAIL"
    return
  fi

  local app="$TARGET_DIR/release/bundle/osx/Zyplus.app"
  local plist="$app/Contents/Info.plist"
  local expected_version
  expected_version="$(awk -F'"' '/^\[/{sec=$0} /^version = /{ if (sec == "[package]") { print $2; exit } }' "$REPO_ROOT/Cargo.toml")"

  if [ ! -d "$app" ] || [ ! -f "$plist" ]; then
    echo "Bundle or Info.plist missing: $app" >&2
    record_result "bundle" "FAIL"
    return
  fi

  if ! plutil -lint "$plist"; then
    record_result "bundle" "FAIL"
    return
  fi

  if ! plutil -convert json -o - "$plist" | python3 -c '
import json, sys

expected_version = sys.argv[1]
data = json.load(sys.stdin)

errors = []
bid = data.get("CFBundleIdentifier")
short_ver = data.get("CFBundleShortVersionString")
bundle_ver = data.get("CFBundleVersion")
if bid != "com.fankrits.zyplus-editor":
    errors.append(f"CFBundleIdentifier={bid!r}")
if short_ver != expected_version:
    errors.append(f"CFBundleShortVersionString={short_ver!r} != {expected_version!r}")
if bundle_ver != expected_version:
    errors.append(f"CFBundleVersion={bundle_ver!r} != {expected_version!r}")

doc_types = data.get("CFBundleDocumentTypes") or []
exts = set()
for dt in doc_types:
    for ext in dt.get("CFBundleTypeExtensions") or []:
        exts.add(ext)

required_exts = {"md", "txt", "csv", "tsv", "json"}
missing = sorted(required_exts - exts)
if missing:
    errors.append(f"Missing document type extensions: {missing}")

if errors:
    for e in errors:
        print("Info.plist check failed:", e, file=sys.stderr)
    sys.exit(1)

print(
    f"Verified Info.plist: id={bid}, "
    f"version={expected_version}, doc_exts={sorted(exts)}"
)
' "$expected_version"; then
    record_result "bundle" "FAIL"
    return
  fi

  if ! codesign --verify --strict --verbose=2 "$app"; then
    record_result "bundle" "FAIL"
    return
  fi

  record_result "bundle" "PASS"
}

run_e2e() {
  header "e2e" "Debug build + scripts/e2e/run.py"
  if [ "$OS_NAME" != "Darwin" ]; then
    echo "SKIP: e2e stage only runs on macOS (current OS: $OS_NAME)"
    record_result "e2e" "SKIP"
    return
  fi

  if ! cargo build; then
    record_result "e2e" "FAIL"
    return
  fi

  local debug_bin="$TARGET_DIR/debug/zyplus-gpui"
  if python3 "$REPO_ROOT/scripts/e2e/run.py" --bin "$debug_bin" ${E2E_ARGS[@]+"${E2E_ARGS[@]}"}; then
    record_result "e2e" "PASS"
  else
    record_result "e2e" "FAIL"
  fi
}

for stage in "${STAGES[@]}"; do
  case "$stage" in
    fmt) run_fmt ;;
    lint) run_lint ;;
    unit) run_unit ;;
    build) run_build ;;
    bundle) run_bundle ;;
    e2e) run_e2e ;;
  esac
done

printf '\n%s=== Summary ===%s\n' "$C_BOLD" "$C_RESET"
for entry in "${RESULTS[@]}"; do
  stage="${entry%%:*}"
  status="${entry#*:}"
  case "$status" in
    PASS) color="$C_GREEN" ;;
    SKIP) color="$C_YELLOW" ;;
    FAIL) color="$C_RED" ;;
    *) color="$C_RESET" ;;
  esac
  printf '  %-8s %s%s%s\n' "$stage" "$color" "$status" "$C_RESET"
done

exit "$OVERALL_STATUS"
