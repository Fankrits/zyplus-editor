# Performance

How to measure Zyplus's runtime cost, what the numbers were, and the budgets that keep it light.
Nothing here runs in `bun run check`; it is opt-in tooling in `scripts/perf/`.

## Measuring

| What | Command |
|---|---|
| Chromium: boot, keystroke latency (small + 120-code-block note), heap, long tasks, idle CPU | `bun run build && bun run perf` (`--dist <dir>` for another build, `--json out.json`) |
| What is in the bundle, brotli sizes (the size Tauri embeds) | `bun run analyze` -> `bundle-report.html` |
| Desktop RSS / CPU of the app + WebKit processes, headless | `node scripts/perf/desktop.mjs <release-binary> --seconds 30` |
| Same, comparing frontends on one binary, with a big project restored | `node scripts/perf/desktop.mjs <debug-binary> --dist <build-dir> --project <folder>` |
| A big project folder to open | `node scripts/perf/fixture.mjs <dir> 4000` (13,000 files: notes, a `node_modules`, a `target`) |

`web.mjs` needs Playwright with a Chromium (`npm i -g playwright`); it is not a project dependency.
`desktop.mjs` needs `Xvfb` and `dbus-run-session`, and uses a fresh `HOME` per run so each is a first run.
Numbers from Xvfb use software rendering: compare runs against each other, not against a real desktop.
The `--dist` mode exists because a folder passed on the command line first fails to open as a file,
which raises a blocking error dialog.

## Baseline (before the runtime work)

Linux x86_64, commit `7c26e42`.

| | |
|---|---|
| Release binary | 7,288,304 B |
| Embedded frontend (brotli) | 926 KB; startup JS 183 KB |
| Desktop, first run (Welcome note open): peak RSS / boot CPU / idle CPU | 394 MB / 2.8 s / 0.3% |
| Desktop, tiny project restored (no editor): peak RSS / boot CPU | 376 MB / 0.85 s |
| Desktop, 13,000-file project restored: peak RSS / boot CPU | 488-493 MB / 2.4-2.9 s |
| Chromium: boot to first note | 0.7-0.9 s, JS heap 9.7 MB |
| Chromium: keystroke p50 / p95, small note | 10 / 17 ms (up to 1 long task) |
| Chromium: 120-code-block note, mount / heap / DOM nodes | 0.4 s / 14 MB / 5,076 |
| Chromium: keystroke p50 / p95, that note | 14-15 / 20 ms (1-2 long tasks, ~100 ms) |
| Chromium: idle main thread busy | 1.5% |

## Budgets

- Editors, and the libraries only extensions use, never reach the startup path (same plugin).
