# Extensions roadmap

Ten features that build on the JSON editor extension, each reusing open-source
libraries. Written 2026-09-29. Every library claim below was checked (version,
license, and a scan for `eval` / `new Function` / WebAssembly); anything not yet
measured says so.

## 1. Goals and non-goals

**Goals.** Make more of what lives in a notes folder openable and renderable, with
no new trust surface and no CSP weakening. Every feature looks like Zyplus (HeroUI
tokens only) and is covered by tests and a real-browser check.

**Non-goals.** Running code from notes. Network features (map tiles, remote
dictionaries). Anything that needs `'unsafe-eval'` or `'wasm-unsafe-eval'` in the CSP.

## 2. Constraints learned from the JSON extension

These are hard requirements for every phase.

1. **CSP is the second security layer** (`tauri.conf.json`): `script-src 'self' blob:`,
   no `'unsafe-eval'`, no `'wasm-unsafe-eval'`. So: no `eval`, no `new Function`, no
   WebAssembly on any path a bundle can reach. A dead fallback branch is acceptable
   only if shown to be unreachable in a modern webview. Library options that switch
   eval off (`isEvalSupported: false`, `useWasm: false`, expression interpreters) must
   be set.
2. **Extensions are one blob-imported ES module.** No code-splitting, no lazy chunks,
   no bare-specifier imports at runtime. Everything is bundled by `Bun.build` into
   `extensions/dist/<id>.js` (+ optional `.css`).
3. **Integrity.** Bundles are SHA-256 pinned in `src/extensions/checksums.json`, fetched
   from the release tag, and installed copies are refreshed when the pinned version
   changes (`extensionManager.ensureCurrent`). `bun run build:extensions` must be
   committed together with `dist` and be byte-identical on a rebuild (CI gate, bun
   floats to latest on CI: currently 1.4.2, same as local).
4. **Hooks that exist today:** `renderCodeBlockPreview` (by manifest `languages`) and
   `mountFileEditor` (by `fileExtensions`, with `fileModes` shown in the tab bar).
   The host only hands a file editor **text** (`initialValue`).
5. **Theming.** Only HeroUI tokens (`var(--accent)`, `--background`, `--overlay`,
   `--default`, `--field-radius`, ...). No color literals, in CSS or JS. Popups and
   modals render beside the editor root, so root-prefixed selectors do not reach
   them. Fields inside overlays use `--default` (HeroUI's secondary input), since
   `--field-background` equals `--overlay` in the dark theme. Metrics come from the
   app's own HeroUI menus: popover radius 24px, menu padding 6px / gap 2px, item
   16px radius, buttons are pills, focus is a ring not a border.
6. **Keyboard.** Widgets that swallow keydown must have app shortcuts forwarded out
   (`ExtensionFileEditor` already does this in the capture phase; find/replace stay
   with the editor).
7. **Data fidelity.** Never rewrite what the user did not touch: keep numbers exact,
   keep cells as strings, keep indentation, quoting, delimiters, BOM, line endings and
   the trailing newline. Say plainly where a mode re-serializes.
8. **Cross-platform.** CI runs macOS, Ubuntu and Windows. Use `new URL(..., import.meta.url)`
   for file access in tests, exact-case imports, no CRLF-sensitive assertions.
9. **Web build.** The web filesystem is text-only (`Map<string, string>`). Features that
   need bytes must degrade gracefully there (hide, or say "desktop only").

## 3. Decision record: extension or built-in?

| Kind | Where it lives | Why |
|---|---|---|
| Renders a fenced block, or opens a whole file type | **Extension** | Fits the two existing hooks, downloads on demand, isolated |
| Changes the editor itself (syntax, panels, indexes, spellcheck) | **Built-in, opt-in in Settings, lazy-loaded chunk** | An extension bundles its own ProseMirror/CodeMirror; two copies break `instanceof` and keyed plugins. Sharing the host's modules with blob bundles is a large, risky API project for little gain. The app already ships a built-in Milkdown plugin (`githubAlerts.ts`) as the pattern. |

Revisit only if a third party ever needs to write editor plugins.

## 4. Feature specs

Sizes: S < 1 day, M 1-3 days, L > 3 days (of focused work).

### E1. CSV / TSV editor (M)
- **What.** Opens `.csv` / `.tsv` with **Table** and **Text** views (tab bar, like JSON),
  and renders ```` ```csv ```` / ```` ```tsv ```` fences as a table.
- **Hooks.** `mountFileEditor` + `renderCodeBlockPreview`. No host changes.
- **Libraries.** Papa Parse 5.7.0 (MIT, 264 KB unpacked) for parsing; Tabulator 6.5.3
  (MIT, 28.9 MB unpacked because it ships many builds; **bundle size unmeasured**)
  for the grid. Both scan clean (0 eval/new Function, 0 WebAssembly).
- **Design.** Cells stay strings (no type inference). Detect and preserve delimiter,
  quote style, BOM and line endings; edits re-serialize only touched rows where
  practical, otherwise say so. Virtualized rendering for large files. Text view is
  CodeMirror with no reformatting.
- **Acceptance.** Round-trip test: open, edit one cell, everything else byte-identical
  (fixtures with quotes, embedded newlines, BOM, CRLF, ragged rows). 50k-row file stays
  responsive. Theme audit test green.

### E2. Code and config files (M)
- **What.** Opens common non-markdown text files (`.yaml .yml .toml .xml .sh .env
  .ini .css .html .js .ts .py .rs .go .sql ...`, **not** `.json`, which stays with the
  JSON extension) in CodeMirror with syntax highlighting, line numbers, no markdown parsing.
- **Hook.** `mountFileEditor`; `fileModes` may be empty (single view).
- **Libraries.** CodeMirror 6 and its language packages are already dependencies
  (`@codemirror/language-data`, `lang-yaml`, `lang-xml`, `@codemirror/legacy-modes`).
  Add `yaml` 2.9.1 (ISC) and `smol-toml` 1.9.0 (BSD-3-Clause) for lint gutters only.
  Both scan clean.
- **Design.** A blob bundle cannot lazy-load languages, so bundle a **curated fixed set**
  and report the size. First catalog match wins for a file type, so catalog order
  matters: JSON before this. Files the user opens that are unknown fall back to plain text.
- **Acceptance.** Each listed extension opens and highlights; YAML/TOML syntax errors
  show in the gutter; saving preserves bytes; shortcuts still work.

### E3. Charts in markdown (S-M)
- **What.** ```` ```chart ```` fence with a JSON config (bar, line, pie, scatter, ...)
  renders a chart; optional `data:` from an adjacent ```` ```csv ```` is out of scope for v1.
- **Hook.** `renderCodeBlockPreview`.
- **Libraries.** Chart.js 4.5.1 (MIT; last release Oct 2025, stable) is the pick.
  Alternative if richer charts are wanted: ECharts 6.1.0 (Apache-2.0; its only
  `new Function` is a fallback used when `JSON.parse` is missing, which never runs in
  a modern webview). Both have no WebAssembly.
- **Design.** Colors come from the theme tokens read at render time; re-render on theme
  change (as the Mermaid extension does through `context.isDarkTheme`). Output must
  survive PDF export (`exportPdf.ts`): render to an image or SVG, verify.
  Invalid JSON shows the raw block, never a crash.
- **Acceptance.** Renders in light, dark and a palette; exports to PDF; invalid config
  degrades to the code block.

### P1. Platform: binary reads (M) — prerequisite for E4 and E5
- **What.** Give file editors bytes: `FileEditorOptions.readBytes(): Promise<Uint8Array>`,
  `filePath`, and a `binary` flag on the manifest so the host does not read the file as
  text (a text read of a PNG corrupts it and, worse, an autosave would overwrite it).
- **Host work.** `fs:allow-read-file` in `src-tauri/capabilities/default.json`;
  `readBytes` in `src/lib/fs.ts` (Tauri `readFile`); binary tabs are **read-only**
  (no dirty state, no autosave, no write path); web build reports "desktop only".
  File tree opens claimed binary types; the open dialog lists them.
- **Acceptance.** A binary file is never written by the app. Test asserts that opening
  and closing an image leaves it byte-identical, including with autosave on.

### E4. Image viewer (S, after P1)
- **What.** Opens `.png .jpg .jpeg .gif .webp .svg .avif` with fit / 100% / zoom and pan.
- **Libraries.** Native `<img>` from a `blob:` URL (CSP `img-src` allows it) and
  `@panzoom/panzoom` 4.6.2 (MIT; scans clean). SVG only ever through `<img>`, never
  inlined, so its scripts cannot run.
- **Acceptance.** Each format renders; huge images do not freeze the UI; SVG with a
  `<script>` does nothing.

### E5. PDF viewer (M-L, after P1) — risk item
- **What.** Opens `.pdf`: paged view, zoom, text selection, find.
- **Library.** `pdfjs-dist` 6.3.289 (Apache-2.0). Scan: no eval, but **WebAssembly in 11
  files** (image decoders). The CSP forbids WASM, so it must run with `useWasm: false`
  and its worker loaded from a `blob:` URL (allowed via `script-src`).
- **Gate.** First task is a spike under the production CSP proving it renders text and
  common images. If some image types (JPEG2000/JBIG2) fail without WASM, document the
  limit; if core rendering fails, **stop and report** rather than loosen the CSP.
  Old pdf.js versions had an eval-related CVE; pin a current version and keep
  `isEvalSupported: false` where the option exists.

### E6. Graphviz / DOT (M-L) — best effort
- **What.** ```` ```dot ```` / ```` ```graphviz ```` fences render as SVG.
- **Approach (no CSP change).** Pure JS: a DOT parser (`graphlib-dot` 0.6.4, MIT, last
  release 2022, or a small purpose-written parser if it does not fit) + `@dagrejs/dagre`
  3.1.1 (MIT, active) for layout + our own SVG renderer. All scan clean. The WASM route
  (`@viz-js/viz`, `d3-graphviz`) is **rejected** because it needs `'wasm-unsafe-eval'`.
- **Scope.** digraph/graph, node shapes (box, ellipse, circle, diamond, plaintext),
  labels, edge labels, `rankdir`, colors, clusters. Anything unsupported degrades to the
  code block with a one-line notice, never a wrong picture.
- **Acceptance.** A fixture set of ~10 DOT graphs renders sensibly; unsupported syntax
  degrades; looks right in every theme.

### F1. Wiki-links `[[note]]` (M-L) — built-in
- **What.** In both editors: `[[Target]]` and `[[Target|alias]]` render as links, click
  opens the note (or offers to create it), typing `[[` autocompletes note names.
- **Rich mode.** Milkdown custom node via `$remark` + `$node` + `$inputRule` from
  `@milkdown/kit/utils` (see `githubAlerts.ts`); parsing logic modeled on
  `remark-wiki-link` 2.0.1 (MIT; last release 2023, so treat as reference and check
  compatibility with the app's remark/unified versions before depending on it).
- **Plain mode.** CodeMirror decoration + `@codemirror/autocomplete` (already installed).
- **Design.** Resolution is case-insensitive by note title/filename, across all open
  project folders; ambiguity picks nearest folder then alphabetical, and says so.
  Markdown round-trips exactly: `[[a|b]]` in, `[[a|b]]` out.
- **Acceptance.** Round-trip fixtures; unresolved links styled distinctly; rename of a
  note offers to update links (v2, may defer).

### F2. Notes index, backlinks and global search (L) — built-in, after F1
- **What.** "Backlinks" panel for the open note, and a project-wide search (⌘⇧F).
- **Library.** MiniSearch 7.2.0 (MIT, ~6 kB gzipped, pure JS, scans clean). FlexSearch
  is faster at 100k+ documents but heavier; a notes folder does not need it.
- **Design.** Build the index lazily in a worker-free background task on first use,
  update incrementally on save/rename/delete (the app already refreshes the tree on
  change). Never read files outside open project folders. Index lives in memory; no
  cache on disk in v1. Results show a snippet, open at the match.
- **Acceptance.** 5k notes index in a few seconds without blocking typing; backlinks
  update after save; deleted notes disappear.

### F3. Front-matter properties (M) — built-in
- **What.** YAML front matter (`---` block) shown as a collapsible properties panel
  instead of raw text in rich mode; editable key/value with type-aware inputs.
- **Libraries.** `yaml` 2.9.1 (ISC) using its **Document API so comments and key order
  survive an edit**; `remark-frontmatter` 5.0.0 (MIT) is the reference for parsing the
  block. (`gray-matter` is unmaintained since 2023 and drops comments; not used.)
- **Acceptance.** Editing one property changes only that line; comments preserved;
  invalid YAML shows the raw block with an error, never loses text.

### F4. Spellcheck (S) — built-in
- **What.** Toggle in Settings for the webview's native spellcheck in rich and plain modes.
- **Design.** `spellcheck` attribute on the ProseMirror root and
  `EditorView.contentAttributes` in CodeMirror; no dictionary library, no network.
  Verify that the Tauri webviews (WKWebView, WebView2, WebKitGTK) honor it; where one
  does not, hide the toggle there.

## 5. Order, waves and dependencies

```
Wave 1 (parallel, independent):   E1 CSV/TSV   E2 Code+config   E3 Charts
Wave 2 (serial):                  P1 binary reads  ->  E4 Images  ->  E5 PDF (spike first)
Wave 3 (serial):                  F4 Spellcheck  ->  F1 Wiki-links  ->  F2 Index/backlinks/search  ->  F3 Front-matter
Wave 4:                           E6 DOT
```

Shared files that will conflict between parallel branches (resolve by regeneration, not
by hand): `src/extensions/catalog.ts` and `scripts/build-extensions.ts` (append-only),
`package.json` (adjacent lines), `bun.lock`, `extensions/dist/**`,
`src/extensions/checksums.json`, `README.md`. Rule: **feature branches never commit
`dist`, `checksums.json` or `bun.lock`.** The integrator regenerates them once after
merging.

## 6. Verification gates (definition of done for every feature)

1. `bun test` and `bunx tsc --noEmit` green, with new tests (fidelity/round-trip first).
2. Extension features: `bun run build:extensions` succeeds; built bundle scanned for
   `new Function`, `eval(` and `WebAssembly`, and every hit explained.
3. Extension CSS: no color literals; `tests/jsonTheme.test.ts`-style guards (every
   library variable themed or allow-listed) for any library with a themable UI.
4. **Real-browser check under the production CSP** (harness page with the CSP from
   `tauri.conf.json`): install through Settings, exercise the feature, zero CSP
   violations, checked in light, dark and one palette.
5. Data-fidelity test for anything that edits files.
6. Integration only: `bun run check:release`; the Linux container run; the CRLF
   simulation; `build:extensions` byte-identical; `git status` clean after commit.
7. The user's `tauri dev` server is never touched (see section 7).

## 7. Delegation to the `agy` CLI

`agy` (v1.2.12) is an agent CLI. Facts established by testing it:

- Print mode: `agy -p "<prompt>" --mode accept-edits --sandbox --model <id>`.
  It edits files and runs shell commands, including the network, without stopping to ask.
- **`--sandbox` blocks reading and writing anything outside its workspace and `/tmp`**
  (verified: home directory, a sibling folder, this repo's `package.json` and
  `~/.ssh` were all refused). Consequence: it cannot damage the main checkout, but it
  also cannot use a `git worktree`, whose `.git` points outside the workspace.
- Models available include `gemini-3.1-pro-high`, `claude-opus-4-6-thinking`,
  `gemini-3.8-flash-high`. Default choice: `gemini-3.1-pro-high`.

**Protocol per feature**
1. Create an isolated copy: `git archive HEAD` into
   `~/dev/zyplus-agy/<slug>/`, `git init` + a `base` commit, and an APFS-cloned
   `node_modules` (`cp -cR`) so its `bun add` cannot touch the real one.
2. Run agy in that directory with a self-contained prompt: goal, constraints from
   section 2, the spec from section 4, files to read first (`src/extensions/bundles/json.*`,
   `tests/jsonTheme.test.ts`, `src/components/Editor/ExtensionFileEditor.tsx`,
   `src/extensions/catalog.ts`), what not to touch, and the gates from section 6.
   It must not create commits, and must end with a short report.
3. **Review everything.** Read the diff, run the gates myself (agy's claim that tests
   pass is not evidence), do the real-browser check on a private port from the copy.
4. Export the patch (source and tests only), apply it in the integration clone
   `~/dev/zyplus-agy/integration`, regenerate `bun.lock`, `dist` and `checksums.json`,
   run the full gates, commit one feature per commit.
5. Fast-forward the main `auth` branch from the integration clone once per wave.
   Nothing is pushed.

**Never**: start a dev server on port 1420 or inside the main checkout (the user's
`tauri dev` owns it; a second Vite wipes its dependency cache), edit files in the main
checkout while it is running, or pass `--dangerously-skip-permissions`.

## 8. Risks and decisions for the user

| Item | Risk / decision |
|---|---|
| E5 PDF | WASM decoders are blocked by the CSP. `useWasm: false` may drop some image types. If the spike fails, PDF is dropped, not the CSP. |
| E6 DOT | Pure-JS output will not match Graphviz pixel for pixel. If exact fidelity matters, the only route is allowing `'wasm-unsafe-eval'`, which is a security decision for you. |
| E1 Tabulator | 28.9 MB unpacked; the real bundle size is unknown until built. Budget: report it, and switch to a lighter grid if it exceeds ~1.5 MB minified (Mermaid is 3.5 MB, for reference). |
| Built-ins vs extensions | F1-F4 are built-in by design (section 3). They add to the main bundle unless lazy-loaded; each must be a lazy chunk behind its Settings toggle. |
| Third-party staleness | `remark-wiki-link` (2023), `graphlib-dot` (2022), `remark-frontmatter` (2023) are stable but unmaintained; prefer small local code over a stale dependency where it is cheap. |
| Cost of delegation | agy output is untrusted until reviewed; budget review time per feature. |

## 9. Status

| Feature | State |
|---|---|
| E1 CSV/TSV | planned |
| E2 Code + config | planned |
| E3 Charts | planned |
| P1 Binary reads | planned |
| E4 Images | planned |
| E5 PDF | planned |
| E6 DOT | planned |
| F1 Wiki-links | planned |
| F2 Index / backlinks / search | planned |
| F3 Front-matter | planned |
| F4 Spellcheck | planned |
