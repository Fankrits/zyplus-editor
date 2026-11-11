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

`desktop.mjs` reports PSS (proportional set size) next to RSS, and a per-process table. RSS counts a
page shared between processes in full in each of them, and the app and WebKit's processes share the
GTK/WebKit libraries, so a summed RSS overstates what the app costs (here 391 MB RSS for 248 MB PSS).
Every RSS figure in the baseline above overstates by the same kind of margin: compare runs by PSS.

> **Build the binary with `bun run tauri build --no-bundle`.** A plain `cargo build --release` does not
> embed the frontend: the app loads the dev server on `localhost:1420` and shows "Could not connect to
> localhost: Connection refused". Such a binary boots in a quarter of a CPU-second and measures the
> webview's empty floor (~165 MB), not the app. An earlier round of numbers in this file's history came
> from exactly that mistake and was thrown away; `desktop.mjs` now warns when a run used under 0.8
> CPU-seconds. Take a screenshot (`import -window root`) before trusting a new kind of run.

Linux x86_64, release binary, first run with the Welcome note open in the rich editor (Xvfb, software
rendering), median of three 60 s runs, idle window at the end:

| | PSS | RSS |
|---|---|---|
| Total | 247.6 MB | 390.7 MB |
| `zyplus-editor` (Rust + GTK + the libraries it maps) | 72.9 MB | |
| `WebKitWebProcess` | 153.4 MB | |
| `WebKitNetworkProcess` | 21.3 MB | |

The harness forces software rendering (`LIBGL_ALWAYS_SOFTWARE`, compositing and DMABUF off), which
inflates the app process on a headless box; on a real desktop the split differs. Use these numbers to
compare runs of the script against each other, not against Task Manager or `top`.

### Where the web process's memory goes

Same binary and method, with the frontend swapped through the debug binary's `--dist` mode (web-process
PSS, two runs each):

| page | web process | step |
|---|---|---|
| blank page | 76.5 MB | the floor |
| + stylesheet only | ~81 MB | +4.5 |
| + JavaScript only, no CSS | ~112 MB | **+35** |
| + both, no editor mounted | ~120 MB | +8 |
| + the rich editor (Welcome note) | ~152 MB | **+33** |

So about 76 MB of the 153 MB is the webview's floor and the rest is the app's JavaScript and the editor.
The JavaScript is where the engine's own overhead shows (the Chromium JS heap for the same page is ~10
MB): see the next section.

## JavaScriptCore's JIT tiers: the lever that paid

WebKit's engine compiles hot JavaScript through four tiers (interpreter, baseline JIT, DFG, FTL). The two
top tiers pay off in long-running compute, which an editor does not do, and they cost memory and CPU to
compile code that short interactive bursts never reuse. The options are read from the environment
(`JSC_useDFGJIT`, `JSC_useFTLJIT`, `JSC_useJIT`), so the app can set them before it starts the webview.

Same binary, idle with the Welcome note, three 25 s runs each (median PSS):

| setting | PSS | peak PSS |
|---|---|---|
| default | 246.7 MB | 261 |
| FTL off | 246.1 | 260 |
| **FTL and DFG off** (shipped) | **238.8** (-3.2%) | 251 |
| JIT off entirely | 225.4 (-8.7%) | 229 |
| `MALLOC_ARENA_MAX=1` / `=2` | 245.7 / 246.0 | |

The gap widens under load. Real keystrokes (`xdotool`, 544 keys at 15 ms) in the real app, one note
open, CPU of every process in the tree:

| | default | FTL + DFG off | JIT off |
|---|---|---|---|
| Welcome note: PSS after typing | 289 MB | 270 MB | 236 MB |
| Welcome note: CPU per key | 10.4 ms | 9.9 ms | 10.0 ms |
| 600 KB note: PSS after typing | 452 MB | 332-356 MB | 245-250 MB |
| 600 KB note: CPU per key | 14.3 ms | 11.8 ms | 11.6 ms |
| 8 MB note: PSS after typing | 506 MB | 385 MB | 253 MB |
| 8 MB note: CPU per key | 14.6 ms | 11.9 ms | 12.3 ms |
| 8 MB note: CPU to open | 1.00 s | 0.88 s | 1.20 s |

Turning the top two tiers off lowers memory and CPU in every case measured, so it is what the app now
does on Linux (`tuning.rs`, applied before the webview starts; a `JSC_*` value already in the
environment is left alone). Turning the JIT off entirely saves another 20-130 MB but opens the 8 MB note
20% slower, and it would slow JavaScript-heavy extensions (Mermaid layout, a large CSV or JSON), which
this harness cannot drive, so it stays an opt-in for the user: `JSC_useJIT=false zyplus`.
macOS's WKWebView and Windows' WebView2 take no such options from the environment, so this is Linux only.

The original and the shipped binary, same method, three 60 s runs interleaved, Welcome note idle:

| | PSS | peak PSS | web process | boot CPU | idle CPU |
|---|---|---|---|---|---|
| before (`0fec639`) | 247.6 MB | 262.2 | 153.4 | 1.49 s | 0% |
| after the JIT change | **239.3 MB** (-8.3) | **250.9** (-11) | 145.1 | 1.36 s | 0% |
| after, with the on-demand UI lazy (a later interleaved run: 247.8 / 239.2 / **234.3**) | **234.3 MB** (-13.5) | **244.9** (-18) | 140.2 | 1.36 s | 0% |

and with a 600 KB note, real keystrokes, two runs each:

| | before | after |
|---|---|---|
| CPU to open | 4.3 s | **0.6 s** |
| PSS after opening | 471-546 MB | **261 MB** |
| PSS after typing | 477-612 MB | **344-361 MB** |
| CPU per key | 13.0 ms | **11.8 ms** |
| Welcome note: PSS after typing | 291 MB | **270 MB** |

(The 600 KB note opens in the source editor now because of the large-file policy below; before, it opened in
rich.)

## What is left, and what it would cost

With the JIT tiers off and the on-demand UI lazy, the idle app is 234 MB PSS: the webview's floor (a blank
page, 165 MB in total, 76.5 MB of it web process) plus about 70 MB that is the app's own. Measured by swapping entry points
(web-process PSS, two runs each, same JIT setting as shipped):

| what is loaded | web process | step |
|---|---|---|
| blank page | 76.5 MB | |
| React 19 alone | 86 MB | +9.6 |
| + one HeroUI `Button` | 90.4 | +4.2 |
| + one icon | 92.5 | +2.0 |
| the whole shell (no editor) | 113 | +20.5 (the other components, the stylesheet, the app) |
| + the rich editor | ~145 | +32 |

What was tried or sized and not done:

- **More JavaScriptCore options**, on the shipped binary: mini-VM mode -1.5 MB for +0.15 s boot CPU,
  concurrent GC off, concurrent JIT off, WebAssembly off: all within noise. Nothing beyond the JIT tiers.
- **A longer batch window for big-note edits** (150 ms -> 1.5 s): typing in a 600 KB note ended at 347
  MB instead of 356 MB, so the ~90 MB it grows while typing is CodeMirror and the webview, not the
  reporting.
- **Dropping HeroUI from the startup path.** Sized first: with every HeroUI part stubbed the web process
  was 13 MB lower, and keeping only `Button` eager 8.5 MB lower. The second is what was done: the
  create-folder, unsaved-changes and delete dialogs, the mobile drawer, the right-click and document-actions
  menus, the inline rename field and the toast region are lazy chunks that mount the first time they are
  needed (`OnDemand`, `ToastHost`, `showToast`). Startup JavaScript fell from 686 KB to 538 KB raw (187 to
  148 KB brotli) and idle PSS from 239.2 to 234.3 MB (three runs each, interleaved). The cost is that the
  first open of each loads its chunk (a frame or two from the local protocol); after that it stays loaded.
- **React -> Preact.** React alone is ~10 MB, so the ceiling is a few MB, and HeroUI's react-aria
  would have to work under `preact/compat`. Not tried.
- **Trimming the editor.** It is +32 MB for ProseMirror, remark, Vue (Milkdown's UI) and CodeMirror
  (for code blocks). Each Crepe feature is a product feature; removing one is a decision about the
  product, not a tuning.

So, within this stack, the app is within about 75 MB of the empty-webview floor at idle and the levers
that measured have been taken. Going meaningfully lower means replacing a dependency (HeroUI, React or
Milkdown) or dropping a feature.

## Other webview settings, measured on the real app

The WebKitGTK settings a markdown editor does not need were tried through `with_webview` (cache model
`DocumentViewer`, page cache off, WebGL off, media stream and MSE off, WebAudio off), each alone and all
together, two fresh-`HOME` runs each, 25 s idle (PSS):

| setting | PSS |
|---|---|
| none | 247.7 / 248.0 MB |
| all five | 250.0 / 250.3 |
| cache model alone | 249.9 / 250.5 |
| page cache alone | 247.6 / 247.8 |
| WebGL alone | 247.2 / 247.8 |
| media alone | 247.3 / 248.5 |
| WebAudio alone | 245.4 / 245.9 |

The cache model makes it **worse** (+2 MB), page cache, WebGL and media are noise, and WebAudio off is
-2 MB (~1%), which is not worth a new native dependency and a per-platform code path. None was shipped.

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
| before | 59-60 | 92-98% of a core (GPU process alone: 46-48%, renderer 4%, browser 56-59%) |
| after, still blinking (5 s after the last key) | 3.7 | 8% |
| after, settled (25 s after the last key) | 2.1 | 4.7% (GPU process alone: 2%, renderer 1%, browser 2%) |
| editor not focused (floor of the harness) | 0.07 | 0.9% |

The CPU percentages include the cost of Chromium's screencast capturing every frame, so they overstate a
real window; the frame rate is the figure to trust. The main-thread-only figure that the baseline above
tracked (`web.mjs`, editor focused) went from 1.4-1.6% to 0.4%. What remains once the blink stops is the browser's own
native caret, which every focused text field has. The desktop harness shows no difference here (idle CPU
0% before and after) because there the Welcome note's editor is not focused and rendering is software.

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

- GPU use on real hardware. There is no GPU here: the Chromium GPU process runs software GL, so its
  CPU time (46-48% of a core before, ~2% after, with a focused editor idle) and the compositor frame
  rate are what is measured. The work is the same, but the cost of a real GPU is not.
- Windows and macOS memory and CPU, since the JIT-tier setting is Linux only.
- The memory saved by evicting background tabs was verified for correctness (tests plus a browser run
  that saves and reopens), not for size.
- Windows low-memory mode.

## Budgets

- Startup JS is capped at `STARTUP_BUDGET_KB` (200, brotli) in `vite.config.ts`; the build fails
  above it. `bun run analyze` shows what joined the startup path.
- Editors, and the libraries only extensions use, never reach the startup path (same plugin).
