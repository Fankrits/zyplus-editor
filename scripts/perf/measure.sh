#!/bin/bash
# Runs every perf scenario in the Linux CI container. Usage: scripts/perf/measure.sh [label]
# Needs podman, the zyplus-linux-ci image and a release build in the zyplus-target-perf volume:
#   podman run --rm -v $PWD:/src:ro -v zyplus-target-perf:/target -v zyplus-cargo:/root/.cargo/registry \
#     -e CARGO_TARGET_DIR=/target -e CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true -w /src zyplus-linux-ci cargo build --release
set -eu
cd "$(dirname "$0")"
LABEL=${1:-run}
OUT=${PERF_OUT:-/tmp/zyplus-perf}; mkdir -p "$OUT"
python3 gen_fixtures.py "$OUT/fx"
podman run --rm -v "$PWD:/p:ro" -v "$OUT/fx:/fx:ro" -v "${TARGET_VOLUME:-zyplus-target-perf}":/target:ro zyplus-linux-ci bash -c '
tabs=""; for i in 0 1 2 3 4 5 6 7 8 9; do tabs="$tabs /fx/tab$i.md"; done
run() { echo "### $1"; shift; /p/run.sh "$@"; }
run 1_welcome_idle /p/idle.txt
run 2_typical_md   /p/idle.txt /fx/typical.md
run 3_ten_tabs     /p/idle.txt $tabs
run 4_big_2.8MB    /p/idle.txt /fx/big.md
run 5_typing_typical /p/typing.txt /fx/typical.md
run 6_typing_big   /p/typing.txt /fx/big.md
' | tee "$OUT/$LABEL.log"
