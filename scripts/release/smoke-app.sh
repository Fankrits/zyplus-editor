#!/usr/bin/env bash
#
# Launches a bundled Zyplus.app (the artifact that ships, a release build with
# no scripted-input runner) in an isolated HOME/TMPDIR and checks that it keeps
# running for a few seconds, then exits cleanly when asked to.
#
#   scripts/release/smoke-app.sh path/to/Zyplus.app [seconds] [--arch x86_64|arm64]
#
# --arch runs one slice of a universal binary (x86_64 needs Rosetta on Apple
# silicon). Exits 0 on success, 1 on failure, 77 when the requested slice
# cannot run on this machine (so callers can treat it as a skip).
set -euo pipefail

APP="${1:?usage: smoke-app.sh Zyplus.app [seconds] [--arch x86_64|arm64]}"
SECS="${2:-6}"
ARCH=""
if [ "${3:-}" = "--arch" ]; then ARCH="${4:?--arch needs a value}"; fi

BIN="$APP/Contents/MacOS/zyplus-gpui"
[ -x "$BIN" ] || { echo "smoke: executable not found: $BIN" >&2; exit 1; }

# Keep both short: the single-instance socket path must stay under ~100 bytes.
ROOT="$(mktemp -d "${TMPDIR:-/tmp}/zs.XXXXXX")"
mkdir -p "$ROOT/home" "$ROOT/tmp"
LOG="$ROOT/log"
PID=""
cleanup() {
  [ -n "$PID" ] && kill -KILL "$PID" 2>/dev/null || true
  rm -rf "$ROOT"
}
trap cleanup EXIT

RUN=("$BIN")
if [ -n "$ARCH" ]; then
  if ! lipo -archs "$BIN" | tr ' ' '\n' | grep -qx "$ARCH"; then
    echo "smoke: binary has no $ARCH slice ($(lipo -archs "$BIN"))" >&2
    exit 1
  fi
  if ! arch "-$ARCH" /usr/bin/true 2>/dev/null; then
    echo "smoke: cannot run $ARCH on this machine (no Rosetta?); skipping" >&2
    exit 77
  fi
  RUN=(arch "-$ARCH" "$BIN")
fi

set +m
HOME="$ROOT/home" TMPDIR="$ROOT/tmp" "${RUN[@]}" >"$LOG" 2>&1 &
PID=$!

for _ in $(seq 1 "$SECS"); do
  sleep 1
  if ! kill -0 "$PID" 2>/dev/null; then
    wait "$PID" || true
    echo "smoke: app exited early; log:" >&2
    sed -n '1,40p' "$LOG" >&2
    exit 1
  fi
done

# Still alive after $SECS seconds: ask it to quit and make sure it does.
kill -TERM "$PID" 2>/dev/null || true
for _ in 1 2 3 4 5; do
  kill -0 "$PID" 2>/dev/null || break
  sleep 1
done
if kill -0 "$PID" 2>/dev/null; then
  echo "smoke: app ignored SIGTERM" >&2
  exit 1
fi
{ wait "$PID"; } 2>/dev/null || true
PID=""
echo "smoke: ok ($APP${ARCH:+, $ARCH}, ${SECS}s)"
