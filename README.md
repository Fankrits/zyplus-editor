<div align="center">

<img src="docs/logo.svg" alt="Zyplus" width="96" />

# Zyplus

**A markdown editor that gets out of the way.**

Write in rendered rich text or raw source — same file, one keystroke apart.
Built with Tauri&nbsp;2, React&nbsp;19 and TypeScript.

[![License: MIT](https://img.shields.io/badge/License-MIT-3b82f6.svg?style=flat-square)](LICENSE)
[![Tauri 2](https://img.shields.io/badge/Tauri-2-24C8DB?style=flat-square&logo=tauri&logoColor=white)](https://tauri.app)
[![React 19](https://img.shields.io/badge/React-19-61DAFB?style=flat-square&logo=react&logoColor=black)](https://react.dev)
[![Downloads](https://img.shields.io/github/downloads/Fankrits/zyplus-editor/total?style=flat-square&color=3b82f6)](https://github.com/Fankrits/zyplus-editor/releases)

[Download](https://github.com/Fankrits/zyplus-editor/releases/latest) ·
[Development](#development) ·
[Layout](#layout)

</div>

---

## Features

| | |
|---|---|
| ✍️ **Two modes** | WYSIWYG rich text (Milkdown/Crepe) or raw markdown (CodeMirror) — `⌘E` to switch, per tab |
| 🎨 **Rich rendering** | GitHub alert callouts, Mermaid diagrams, KaTeX math, syntax-highlighted code |
| 🗂️ **Real workspace** | Multiple project folders, tabs, file tree, find & replace, optional autosave |
| 📄 **PDF export** | Genuinely paginated PDFs through the native print pipeline — not `window.print()` |
| 🖱️ **OS integration** | Registered editor for `.md`, `.markdown`, `.mdown`, `.mkd`, `.txt` — double-click opens a tab |
| 🌗 **Themes** | Light, dark, system, plus Catppuccin and Tokyo Night variants |
| ⬆️ **Self-updating** | In-app updater backed by signed GitHub releases |

Your session — open folders, tabs, active file, sidebar state — comes back
exactly as you left it.

## Install

Grab an installer for macOS, Linux or Windows from the
[latest release](https://github.com/Fankrits/zyplus-editor/releases/latest).

## Development

**Requirements:** [Bun](https://bun.sh), a [Rust toolchain](https://rustup.rs),
and the [Tauri system prerequisites](https://tauri.app/start/prerequisites/)
for your platform.

```sh
bun install
bun run tauri dev     # desktop app (Vite on :1420)
bun run dev           # browser only — no filesystem, for UI work
bun test              # test suite (bun:test + happy-dom)
bun run build         # typecheck + build the frontend
bun run tauri build   # installers into src-tauri/target/release/bundle
```

## Layout

```
src/
  components/Editor/   RichTextEditor (Milkdown), PlainTextEditor (CodeMirror),
                       mermaid / katex / alert rendering, search bar
  components/Sidebar/  file tree over the open project folders
  components/Tabs/     tab bar
  lib/                 fs, shortcuts, theme, PDF export, session persistence
  state/               workspace store + reducer (tabs, roots, dirty state)
src-tauri/src/lib.rs   Rust side: file associations, pending-file queue,
                       silent print-to-PDF
tests/                 unit tests for lib/ and state/
```

Keyboard shortcuts live in exactly one place — `src/lib/shortcuts.ts`. The key
bindings and the in-app shortcut sheet (`⌘/`) read the same table.

## Releases

Push a `v*` tag and [`release.yml`](.github/workflows/release.yml) stamps the
version into `package.json`, `tauri.conf.json` and `Cargo.toml`, builds signed
bundles for macOS (universal), Linux and Windows, and publishes them alongside
the `latest.json` the in-app updater reads.

## License

[MIT](LICENSE) © Fankrits
