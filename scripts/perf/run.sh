#!/bin/bash
# Headless memory/CPU sampler for the Linux container (Xvfb). Nothing touches a real display.
#   run.sh <script.txt> [files to open...]
# Env: BIN (default /target/release/zyplus-gpui), SAMPLES (default 8, 5 s apart).
# Prints rss/anon MB, threads, CPU % and idle context switches per second for each sample,
# then the last ZYPLUS_RICH_TIMING line. Compare `anon`, not `rss`: rss includes the binary
# and the software GPU libraries.
set -u
BIN=${BIN:-/target/release/zyplus-gpui}
SAMPLES=${SAMPLES:-8}
export HOME=/tmp/h-$$ TMPDIR=/tmp/t-$$ ZYPLUS_RICH_TIMING=1
mkdir -p "$HOME" "$TMPDIR"
SCRIPT=$1; shift
ZYPLUS_TEST_SCRIPT=$SCRIPT xvfb-run -a -s "-screen 0 1280x800x24" "$BIN" "$@" > "$TMPDIR/app.log" 2>&1 &
XV=$!
sleep 6
PID=$(pgrep -n -f "^$BIN" || true)
if [ -z "$PID" ]; then echo "app not running"; tail -5 "$TMPDIR/app.log"; exit 1; fi
HZ=$(getconf CLK_TCK)
cpu() { awk '{print $14+$15}' /proc/$1/stat; }
ctx() { awk '/voluntary_ctxt_switches/{s+=$2} END{print s}' /proc/$1/status; }
for i in $(seq 1 "$SAMPLES"); do
  c0=$(cpu "$PID"); w0=$(ctx "$PID"); sleep 5; c1=$(cpu "$PID"); w1=$(ctx "$PID")
  rss=$(awk '/VmRSS/{print $2}' /proc/$PID/status 2>/dev/null) || break
  anon=$(awk '/RssAnon/{print $2}' /proc/$PID/status)
  thr=$(awk '/Threads/{print $2}' /proc/$PID/status)
  awk -v t=$((6+i*5)) -v rss="$rss" -v anon="$anon" -v thr="$thr" -v c0="$c0" -v c1="$c1" -v w0="$w0" -v w1="$w1" -v hz="$HZ" \
    'BEGIN{printf "t=%ds rss_kb=%d anon_kb=%d threads=%d cpu_pct=%.1f wakeups_per_s=%.1f\n", t, rss, anon, thr, (c1-c0)/hz/5*100, (w1-w0)/5}'
  kill -0 "$PID" 2>/dev/null || break
done
grep "rich-timing" "$TMPDIR/app.log" | tail -1 | cut -c1-400
kill "$PID" 2>/dev/null; wait "$XV" 2>/dev/null
