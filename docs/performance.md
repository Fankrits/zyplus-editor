# Performance

How to measure Zyplus's runtime cost, what the numbers were, and the budgets that keep it light.
Nothing here runs in `bun run check`; it is opt-in tooling in `scripts/perf/`.

## Measuring

| What | Command |
|---|---|
| Chromium: boot, keystroke latency (small + 120-code-block note), heap, long tasks, idle CPU | `bun run build && bun run perf` (`--dist <dir>` for another build, `--json out.json`) |
| What opening one big note costs, and typing at the end of it (editor chosen, time, heap, DOM, keystroke latency) | `bun run build && bun run perf:bigfile` (`--kb 1000`) |
| What a focused but untouched editor costs while idle (compositor frames/s, CPU of every Chromium process) | `bun run build && node scripts/perf/idle.mjs` (`--settle 25 --window 15`) |
| What is in the bundle, brotli sizes (the size Tauri embeds) | `bun run analyze` -> `bundle-report.html` |
| Desktop PSS / RSS / CPU of the app + WebKit processes, per process, headless (`--max-pss <MB>` fails above a budget) | `node scripts/perf/desktop.mjs <release-binary> --seconds 30` |
| The same, in CI | Actions tab -> Perf -> Run workflow (`.github/workflows/perf.yml`, manual only) |
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
| Chromium: idle main thread busy, editor focused / not | 1-2% / 0% |

## After

Same machine and method, same debug binary for the desktop rows so only the frontend differs.

| | before | after |
|---|---|---|
| Desktop, 13,000-file project restored: peak RSS | 503 MB | 377 MB (-25%) |
| Desktop, 13,000-file project restored: boot CPU | 2.4 s | 0.9 s (-63%) |
| Desktop, tiny project / first run with Welcome note: peak RSS | 376 / 425 MB | 375 / 419 MB (webview-bound, unchanged) |
| Embedded frontend (brotli) | 926 KB in 145 files | 737 KB in 50 files (-20%) |
| Startup JS (brotli) | 183 KB | 184 KB |
| Release binary, Linux | 7,288,304 B | 7,057,776 B (-3.2%; -10.0% since 7,845,104) |
| Chromium typing, idle | | unchanged |

What did it: the file tree is read one level at a time (folders start collapsed; a project root
opens to its first level), and code fences highlight 28 common languages instead of ~110.
macOS and Windows also drop the updater's bundled TLS for the OS's; that is not measurable from
Linux, where nothing changes.

## Desktop memory is PSS, not RSS

`desktop.mjs` now reports PSS (proportional set size) next to RSS, and a per-process table. RSS counts a
page shared between processes in full in each of them, and the app and WebKit's processes share the
GTK/WebKit libraries, so a summed RSS overstates what the app costs. Every RSS figure above, and in
older commits, overstates by the same kind of margin: compare runs by PSS.

Linux x86_64, release binary, first run with the Welcome note open (Xvfb, software rendering):

| | PSS | RSS |
|---|---|---|
| Total | 174 MB | 311 MB |
| `zyplus-editor` (Rust + GTK + the libraries it maps) | 66 MB | 122 MB |
| `WebKitWebProcess` | 81 MB | 135 MB |
| `WebKitNetworkProcess` | 27 MB | 54 MB |

So the web process, the part the frontend can influence, is under half of the total. The harness forces
software rendering (`LIBGL_ALWAYS_SOFTWARE`, compositing and DMABUF off), which inflates the app process
on a headless box; on a real desktop the split differs. Use these numbers to compare runs of the script
against each other, not against Task Manager or `top`.

## Webview levers, measured on Linux: no effect

The WebKitGTK settings a markdown editor does not need were tried through `with_webview`
(cache model `DocumentViewer`, page cache off, WebGL off, media stream and MSE off, WebAudio off).
Each was applied (read back from the settings object) and measured alone, plus all together, three
fresh-`HOME` runs each, 25 s idle with the Welcome note open:

| setting | final PSS, median of 3 |
|---|---|
| none | 174.4 MB |
| all five | 174.4 MB |
| cache model alone | 174.6 MB |
| page cache alone | 174.5 MB |
| WebGL alone | 174.4 MB |
| media alone | 174.5 MB |
| WebAudio alone | 174.5 MB |

Every difference is under 0.3 MB and inside the run-to-run noise (about 0.4 MB). Switching off a feature
that is never used saves nothing, and the cache model only matters once resources are loaded and evicted,
which a local-protocol app barely does. They were not shipped. This was an idle measurement; a long
session may behave differently, but there is no evidence for these levers.

The Windows equivalent (WebView2's `MemoryUsageTargetLevel`, Low while the window sits unfocused) is
implemented in `src-tauri/src/tuning.rs` but **off unless `ZYPLUS_WEBVIEW_LOWMEM=1`**: it has not been
measured, because it needs a Windows machine. It compiles for the Windows target (`cargo check --target
x86_64-pc-windows-msvc`). Measure with and without the variable and make it the default only if it wins.

## Idle with a focused editor: 60 frames a second to about 2

ProseMirror draws its caret as an element faded by a CSS animation with a `linear` timing function and no
iteration limit, so for as long as a rich editor has focus the compositor produces a frame every 16 ms,
even when nothing else on the page changes. The previous idle figure (1-2% of the main thread) hid this:
it counted main-thread time, not the compositor work behind it. Now the caret steps on and off (as
CodeMirror's always has) and stops blinking a few seconds after the last keystroke. Chromium, focused rich
editor, 15 s window after 25 s of no input (`node scripts/perf/idle.mjs`), two runs each:

| | compositor frames/s | CPU of all Chromium processes |
|---|---|---|
| before | 59-60 | 92-98% of a core |
| after, still blinking (5 s after the last key) | 3.7 | 8% |
| after, settled (25 s after the last key) | 2.1 | 4.7% |
| editor not focused (floor of the harness) | 0.07 | 0.9% |

The CPU percentages include the cost of Chromium's screencast capturing every frame, so they overstate a
real window; the frame rate is the figure to trust. The main-thread-only figure that the baseline above
tracked (`web.mjs`, editor focused) went from 1.4-1.6% to 0.4%. What remains once the blink stops is the browser's own
native caret, which every focused text field has. The desktop harness shows no difference here (idle CPU
0% before and after) because there the Welcome note's editor is not focused and rendering is software.

## Desktop memory: nothing in the app moves the floor

Release binaries built from `0fec639` (before) and from this branch (after), three 60 s runs each,
interleaved, Welcome note open, idle window at the end:

| | PSS | web process | network process | boot CPU | idle CPU |
|---|---|---|---|---|---|
| before | 168.2 MB | 76.3 MB | 26.2 MB | 0.28 s | 0% |
| after | 168.2 MB | 76.3 MB | 26.3 MB | 0.25 s | 0% |

Identical. The work in this file lowers memory and CPU in the cases that scale with what the user does
(big notes, many tabs, a focused editor left alone), not the idle floor, which is the webview. Process-level
settings were tried on the same binary to see whether any of them moves that floor (three 25 s runs each,
median PSS): `MALLOC_ARENA_MAX=1` 167.7 MB, `=2` 168.1 MB, JavaScriptCore with FTL off 168.3 MB, with DFG
and FTL off 168.4 MB, with the JIT off 166.4 MB, against 168.5 MB. The best of them (JIT off, -1.2%) makes
every script slower, so none was adopted.

## Large files

Before, a file of any size opened in the rich editor, which builds a ProseMirror document and a DOM node
per block. Now a file over 200 KB opens in the source editor (the user can still switch to rich) and
one over 20 MB is refused, with the size checked by `stat` before the file is read. The check sits in
front of every path that reads a document into an editor: opening, restoring the session at launch (a
file that grew while the app was closed is dropped without being read) and reading an evicted tab back.
Sizes are bytes everywhere. A restored tab keeps the editor it was left in, so a big note the user
switched to rich stays rich.

One note of about 1 MB, reopened from the tree and typed in (`bun run perf:bigfile`, Chromium):

| | opens in | time to usable | heap growth | DOM nodes | keystroke p50 / p95 | main thread per key |
|---|---|---|---|---|---|---|
| before (`0fec639`) | rich | 3.76 s | +22.4 MB | 103,835 | 363 / 614 ms | 152 ms |
| after | source | 0.10 s | -0.4 MB | 1,012 | 13 / 19 ms | 11 ms |

The 200 KB threshold comes from measuring the rich editor's keystroke latency (p50, same harness):

| note size | 60 KB | 120 KB | 200 KB | 300 KB | 1 MB |
|---|---|---|---|---|---|
| rich | 15 ms | 20 ms | 28 ms | 41 ms | 363 ms |
| source | | | | | 13 ms |

and the source editor stays usable up to the 20 MB cap: p50 21 ms at 5 MB, 33 ms at 15 MB, 0.1-0.2 s to open.
Notes between 200 KB and 1 MB were already open in rich before and after; only the default editor changed.

Above 200,000 characters the source editor also reports edits to the store at most every 150 ms instead
of copying the whole document out on every keystroke; `getState()` settles a waiting batch first, so
save, export, copy and close always see the text on screen. Big, unmodified tabs that are not showing
let go of their text and read it back from disk when focused.

## Findings that led to no change

Measured, then left alone, so they are not re-investigated:

- **Editor DOM observer** (`RichTextEditor.tsx` rescans every code block on each DOM change).
  Rewriting it to look only at touched blocks, once per frame, changed nothing measurable in a
  120-code-block note (keystroke p50 12-16 ms against 14-15 ms before, p95 unchanged); it only
  starts to matter with thousands of blocks.
- **Keystroke path.** p95 is 17-21 ms: one frame. The ~100 ms long task per typing burst is layout
  of the big note, not the store or the observer.
- **Chart.js and the file tree.** Charts are rasterized offscreen and destroyed straight away, and the
  tree reads one folder at a time, so neither holds memory worth releasing.
- **Harness trap.** Playwright's `keyboard.insertText` of a megabyte into CodeMirror takes ~100 s in the
  original and the current build alike (a DOM-diffing path). A real paste event takes 0.1 s. Big-document
  scripts must paste, as `bigfile.mjs` does.
- **Idle.** Nothing but the caret's blink animation runs (`prosemirror-virtual-cursor-blink`,
  `cm-blink`), and only while an editor has focus. Unfocused the main thread is 0.01% busy.

## Not measured yet

- GPU use on real hardware. There is no GPU here; compositor frames per second is the proxy.
- The memory saved by evicting background tabs was verified for correctness (tests plus a browser run
  that saves and reopens), not for size.
- Windows low-memory mode, and anything on macOS.

## Budgets

- Startup JS is capped at `STARTUP_BUDGET_KB` (200, brotli) in `vite.config.ts`; the build fails
  above it. `bun run analyze` shows what joined the startup path.
- Editors, and the libraries only extensions use, never reach the startup path (same plugin).
