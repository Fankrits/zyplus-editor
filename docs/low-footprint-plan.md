# Low-footprint plan: lowest RAM and CPU for Zyplus

Status: proposal. Nothing in here is implemented yet.
Written against commit `0fec639` (v0.4.0).

## 0. Verdict

**Stay on Tauri + React and attack the webview, the big-document paths, and the idle behaviour.**
Do not rewrite in egui, Slint, iced or gpui.

Why, from the project's own measurements (`docs/performance.md`):

- Desktop peak RSS is ~375-420 MB for a tiny project or the Welcome note, and the doc calls that
  "webview-bound". The JS heap is only ~10-14 MB. The rest is the OS webview's own processes.
- Earlier work (lazy file tree, 28 highlight languages instead of ~110, lazy editors/extensions) already
  took the big JS-side wins. What is left in JS is a few percent of idle RSS. What is left outside JS
  (webview process model, caches, GPU/compositor) is most of it.
- No Rust UI toolkit has a rich-text markdown editor, Mermaid, KaTeX, charts, tables or a JS extension
  system. A native rewrite is a different product, not an optimization.

Realistic expectation (stated plainly): per-OS webview tuning and idle policies can plausibly trim tens
of MB from idle and a lot more in the worst cases (huge files, many tabs, Mermaid-heavy notes). They will
not take a WebKitGTK/WebView2 app to a 50 MB native footprint. Every number below is a hypothesis until
Phase 0 produces a baseline.

## 1. What the code analysis found

### Already good (leave alone)
- One editor mounted at a time (`EditorPane.tsx`); editors are lazy chunks; a build guard
  (`guardBundle` in `vite.config.ts`) enforces a 200 KB brotli startup budget and keeps extension
  libraries (Mermaid, KaTeX, Chart.js, Tabulator, JSON editor) out of the app bundle.
- File tree reads one level at a time; store split into four contexts so typing doesn't re-render the shell.
- Rust side is tiny and size-tuned (`opt-level="z"`, LTO, strip, `panic=abort`); no devtools feature;
  PDF export's second webview is closed after each export.
- Idle CPU is ~0% unfocused, 1-2% with an editor focused (caret blink only). No polling timers.
- The DOM-observer rewrite and keystroke-path rewrite were measured as no-ops; don't repeat them.

### Findings (new)

| # | Finding | Where | Why it matters |
|---|---|---|---|
| F1 | **No file-size guard anywhere** (no size/limit check in `src/lib/fs.ts` or `openFile`). A multi-MB file opens straight into Milkdown/ProseMirror. | `workspaceStore.tsx` `openFile`, `fs.ts` | Worst-case RAM and CPU spike by far; a rich-text DOM for a big doc is ~10-100x the file size. |
| F2 | **CodeMirror source mode calls `onChange` with the whole document string on every keystroke** (`@uiw/react-codemirror` does `doc.toString()`), then the reducer stores it and compares it to `savedContent`. | `PlainTextEditor.tsx`, `workspaceReducer.ts:194` | O(document size) work and a fresh string allocation per keystroke. Invisible on small notes, painful and GC-heavy on large ones. |
| F3 | Every open tab keeps `content` plus `savedContent` strings in the store (two copies once dirty), forever. | `workspaceReducer.ts` | Grows linearly with tabs x file size; nothing evicts background tabs. |
| F4 | Session-persist effect and autosave effect both depend on `state.tabs`, so they run on every keystroke (a `JSON.stringify` of the session, and an array filter). | `workspaceStore.tsx:223,252` | Small, constant CPU per keystroke. Cheap to fix. |
| F5 | **The perf harness is stale.** `scripts/perf/web.mjs` waits for a `file-name.md` input that the v0.4.0 "New File creates an Untitled.md at once" change removed; it times out. | `scripts/perf/web.mjs:79` | No regression gate exists today. Phase 0 must fix this first. |
| F6 | `scripts/perf/desktop.mjs` reports summed **RSS** across the process tree. RSS double-counts pages shared between the app and WebKit processes. | `scripts/perf/desktop.mjs` | Numbers overstate real usage and can't be compared to what Task Manager/Activity Monitor show per process. Needs PSS (Linux) and per-process breakdown. |
| F7 | Caret blink (`prosemirror-virtual-cursor-blink`, `cm-blink`) is a perpetual CSS animation while an editor has focus. | measured in `docs/performance.md` | Source of the only non-zero idle CPU; it keeps the compositor waking up on a laptop battery. |
| F8 | Extension renderers (Mermaid/KaTeX/Chart.js) are loaded via Blob `import()` and, once loaded, stay resident for the session; chart/diagram instances for off-screen blocks aren't released. | `extensions/loader.ts`, `codeBlockPreview.ts` | Mermaid is the heaviest thing a user can load; it is never unloaded. |
| F9 | HeroUI v3 (react-aria based) contributes a 41 KB gz shared chunk plus Modal/Drawer/Dropdown/Toast code for 9 import sites; Hugeicons adds a 28 KB gz chunk for 18 call sites. | `dist/assets/useFormValidation-*.js`, `HugeiconsIcon-*.js` | Startup parse/JIT cost and heap. Real but modest; do last. |

### Built bundle, measured today (`bun run build`)

| Chunk | Raw | gzip |
|---|---|---|
| `editor-core` (Milkdown, ProseMirror, remark, Vue) | 745 KB | 235 KB |
| `codemirror` | 435 KB | 140 KB |
| `react-vendor` | 219 KB | 68 KB |
| `index` (app) | 192 KB | 56 KB |
| `useFormValidation` (HeroUI/react-aria) | 132 KB | 42 KB |
| `HugeiconsIcon` | 84 KB | 28 KB |

Build passes the startup budget. 51 asset files, `dist` is 12 MB for the web build (includes
`extensions-dist`; the desktop build skips that).

### Research (sources at the bottom)
- **Windows / WebView2:** `ICoreWebView2_19::MemoryUsageTargetLevel` lets the host say "use less memory".
  Microsoft's guidance is Low when the app is inactive, Normal when active. Best effort, can cause some
  process memory to be swapped out, scripts keep running. Don't mix it with TrySuspend/Resume.
- **Linux / WebKitGTK:** There is no env var for the cache model; it's set via
  `webkit_web_context_set_cache_model()`. Docs describe `DOCUMENT_VIEWER` as the lowest-memory model for
  apps that aren't browsers, but the documentation is inconsistent about what it caches, so the saving
  must be measured. WebKit bug reports describe a WebKitWebProcess that grows over long sessions and that
  does not give memory back when content is closed. `WEBKIT_DISABLE_DMABUF_RENDERER` /
  `WEBKIT_DISABLE_COMPOSITING_MODE` are compatibility workarounds for NVIDIA/blank-window bugs, not memory
  tools; a Feb 2026 WebKit bug says both crash MiniBrowser in one build, so don't ship them as defaults.
- **macOS / WKWebView:** the lightest of the three in published comparisons; no equivalent per-view
  memory knob found. Focus on not holding memory in the page.
- **Tauri vs Electron:** Tauri's advantage is clearest on macOS and Linux; on Windows both are Chromium
  based so the gap is small. Headline "90% less" blog numbers are unverified.
- **Hide vs destroy:** keeping a hidden webview warm costs RAM; destroying and recreating costs a cold
  start (WebKit process init, React mount, first paint). One Tauri issue hints WebView2 memory
  accumulates only while a webview is attached. No tested recipe was found; treat as a spike.

## 2. Targets (provisional, finalize after Phase 0)

Measured as **PSS summed over the app + webview process tree**, release build, median of 5 runs.

| Scenario | Metric | Target |
|---|---|---|
| S1 cold boot, no file open | PSS, boot CPU-seconds | -10% vs baseline |
| S2 Welcome note open, idle 60 s | PSS, idle CPU % | -15% PSS, idle CPU < 0.5% |
| S3 window minimized/hidden 5 min | PSS | -30% vs S2 on Windows; no worse than S2 elsewhere |
| S4 13,000-file project restored | PSS | no regression vs current (377 MB debug) |
| S5 10 tabs of 200 KB notes | PSS growth per tab | <= 1.5x file size per background tab |
| S6 open a 5 MB `.md` | peak PSS, UI stays responsive | no more than +150 MB, never freezes (guarded fallback) |
| S7 Mermaid note, then close it | PSS returns within 15% of S2 | within 15% |
| S8 typing 60 s in a 1 MB source file | CPU-seconds, p95 key latency | -50% CPU, p95 < 25 ms |

## 3. Plan

Effort: S = under a day, M = 1-3 days, L = a week+. Order matters: do Phase 0 first, then 1 and 2 in
parallel, then 3. Each task lists how it is verified.

### Phase 0: measurement you can trust (do first)
| Task | Effort |
|---|---|
| **0.1** Fix `scripts/perf/web.mjs` for the Untitled.md flow (F5). Verify: script runs green on `main`. | S |
| **0.2** Make `scripts/perf/desktop.mjs` report **PSS per process** (`/proc/<pid>/smaps_rollup`) and a per-process table (app, WebKitWebProcess, WebKitNetworkProcess, GPU) (F6). Add equivalents for Windows (WebView2 child processes by working set + private bytes) and macOS (`footprint`/`vmmap -summary`). | M |
| **0.3** Add scenarios S1-S8 as fixtures (big files: 1 MB, 5 MB; 10-tab session; Mermaid note). | M |
| **0.4** Record baselines per OS in `docs/performance.md`, replacing the single Linux column. | S |
| **0.5** Add a memory/CPU budget check beside the existing `STARTUP_BUDGET_KB`, run on release tags (not every push; it needs a display). | M |

Exit gate: baselines committed; targets in section 2 finalized from real numbers.

### Phase 1: webview-level levers (biggest lever on idle RAM; each is a measured spike)
Each task is an A/B: ship only if it beats noise (>= 5% PSS) with no visible regression.

| Task | Platform | Detail | Effort | Risk |
|---|---|---|---|---|
| **1.1** Low-memory mode when inactive | Windows | On window blur/minimize/hide, set `MemoryUsageTargetLevel = Low`; Normal on focus. Reach `ICoreWebView2_19` through Tauri's `with_webview` platform handle. Don't also use TrySuspend. | M | Low. Best effort; may cause swap-out and a slight wake-up lag. |
| **1.2** WebKitGTK cache model + settings | Linux | Through `with_webview`: `set_cache_model(DocumentViewer)`; disable features a markdown editor doesn't use (page cache, WebGL, media/MSE/WebRTC, offline app cache, spellcheck if unused). Verify the CSP-allowed features aren't needed first. | M | Low/Medium. Docs conflict on what the model drops; measure. |
| **1.3** Hardware-acceleration policy experiment | Linux | Compare default vs on-demand vs never on PSS **and** scroll smoothness. Keep default if scrolling degrades. | S | Medium: rendering bugs on some GPUs. Ship only as an opt-in setting if at all. |
| **1.4** WebView2 launch flags experiment | Windows | `additionalBrowserArgs` candidates: renderer process limit, V8 heap cap, GPU off. Measure each individually. | S | Medium; flags can hurt rendering. Drop anything that isn't a clear win. |
| **1.5** Idle recycle spike | all | After N minutes hidden with **no dirty tabs**, reload the page (session already restores tabs). Tests whether the web process actually returns memory (bug reports say it often doesn't). Also test destroy+recreate of the main window on Windows. | M | Medium: loses transient UI state; must never run with dirty tabs or an in-flight export. Kill it if gains aren't real. |
| **1.6** Confirm no extra processes/features in release | all | Ensure no `devtools` feature, one webview at steady state, print window closed after export. | S | None |

### Phase 2: app-level memory and CPU
| Task | Fixes | Detail | Effort |
|---|---|---|---|
| **2.1** Large-file policy | F1 | Thresholds (tune from S6): above ~512 KB open in source mode by default with a banner; above ~10 MB open read-only/plain with no syntax parsing, or offer "open in default app". Reuse `tryFs` error reporting. Include a size check in `readTextFile` using `stat` so the file isn't read before the decision. | M |
| **2.2** Drive CodeMirror directly, debounce reads | F2 | Replace `@uiw/react-codemirror` with a thin `EditorView` wrapper; push changes with a debounced `updateListener` (~150 ms) that reads `doc.toString()` once per pause and tracks dirtiness by a change counter, not string comparison. Also removes a dependency. | M |
| **2.3** Evict background tab content | F3 | For tabs that are clean and inactive for > N minutes, drop `content`/`savedContent` and re-read from disk on activation. Dirty tabs are never evicted. Keep one copy (`savedContent` alias) when clean. | M |
| **2.4** Stop per-keystroke effects | F4 | Persist the session only when its inputs change (derive a path/mode key, not `state.tabs`); autosave via a `dirtyIds` selector. | S |
| **2.5** Stop caret blink after idle | F7 | Limit blink to ~N cycles after the last input (CSS `animation-iteration-count` reset on input), honor `prefers-reduced-motion`. | S |
| **2.6** Release extension resources | F8 | Render Mermaid/Chart blocks only when visible (IntersectionObserver) and destroy Chart.js instances on unmount; cache SVG strings, not live objects. Add "unload when no open tab uses it" with a delayed teardown. Check Blob URL revocation (already done) and KaTeX CSS/font retention. | M-L |
| **2.7** Image handling | new | Lazy-load and async-decode images in rich view; cap decoded size for very large images (decoded bitmaps dwarf file size). | S |
| **2.8** File tree ignore + cap | new | Verify `node_modules`, `.git`, `target`, `dist` are not expanded eagerly and that huge folders paginate. The 13,000-file fixture already covers this. | S |
| **2.9** Trim UI libraries (only if Phase 0 shows startup heap/parse matters) | F9 | Replace the 9 HeroUI sites with small local components / native `<dialog>`; swap Hugeicons for inline SVGs. Expected savings: ~70 KB gz on startup-adjacent chunks. | L |

### Phase 3: protect the gains
| Task | Effort |
|---|---|
| **3.1** CI job (release tags) running Phase 0 scenarios on Linux + Windows runners; fail on regression beyond a threshold. | M |
| **3.2** Keep `STARTUP_BUDGET_KB`, add a budget for each lazy editor chunk. | S |
| **3.3** Document each shipped lever and its measured effect in `docs/performance.md`, including the ones that didn't work, so nobody re-investigates. | S |

### Phase 4: only if the targets still aren't met
1. **Ship a "Lite" mode:** source-only editing (CodeMirror, no Milkdown/Vue), no extensions. This cuts
   `editor-core` (235 KB gz) from the process entirely and is the cheapest large step.
2. **Separate native "Zyplus Lite"** (Rust: a text-buffer crate + a GPU/native toolkit) for plain
   markdown source editing. Real footprint wins, but a second product with no WYSIWYG, diagrams or
   extensions. Only do this if users ask for it.
3. Not recommended: Electron/CEF (heavier), Servo/Blitz embedding (not production-ready for this).

## 4. Suggested order and expected payoff

| Order | Tasks | Payoff |
|---|---|---|
| 1 | 0.1-0.5 | Reliable numbers; unlocks everything else |
| 2 | 2.1, 2.2, 2.4, 2.5 | Removes worst-case spikes and wasted per-keystroke CPU; low risk |
| 3 | 1.1, 1.2 | Largest likely idle/background RAM savings on Windows/Linux |
| 4 | 2.3, 2.6, 2.7 | Memory that scales with usage (tabs, diagrams, images) |
| 5 | 1.3, 1.4, 1.5 | Higher-risk experiments; keep only measured wins |
| 6 | 3.x, then 2.9 / Phase 4 if needed | Guard rails, then deeper cuts |

## 5. Risks and non-goals
- Webview tuning is platform-specific and best effort; each lever needs a measurement and a revert path.
- RAM that the OS can reclaim (cached/shared pages) is not the same as a leak; judge by PSS/private bytes.
- Not in scope: changing the editor stack (Milkdown/CodeMirror), dropping features, a native rewrite.

## Sources
- [WebView2 ICoreWebView2_19 (MemoryUsageTargetLevel)](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2_19)
- [CoreWebView2.MemoryUsageTargetLevel](https://learn.microsoft.com/en-us/dotnet/api/microsoft.web.webview2.core.corewebview2.memoryusagetargetlevel)
- [WebKitGTK CacheModel](https://webkitgtk.org/reference/webkit2gtk/stable/enum.CacheModel.html)
- [Tauri: Linux graphics issues (WebKitGTK env vars)](https://v2.tauri.app/develop/debug/linux-graphics/)
- [WebKit bug 319590 (WebKitWebProcess memory growth)](https://www2.webkit.org/show_bug.cgi?id=319590)
- [WebKit bug 306993 (DMABUF/compositing vars crash in one build)](https://bugs.webkit.org/show_bug.cgi?id=306993)
- [Tauri issue 5889 (memory, Tauri vs Electron)](https://github.com/tauri-apps/tauri/issues/5889)
- [Tauri cold launch write-up (hide vs recreate)](https://dev.to/tinyforge/tauri-cold-launch-was-1s-heres-how-i-got-it-to-feel-instant-32lh)
- [Tauri vs Electron comparison (macOS numbers)](https://dev.to/gethopp/tauri-vs-electron-performance-bundle-size-and-the-real-trade-offs-1el4)
