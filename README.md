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

[Install](#install) ·
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

Every method installs the same build from the same
[GitHub release](https://github.com/Fankrits/zyplus-editor/releases/latest),
and the app self-updates from then on regardless of how it got there.

| Platform | Terminal install |
|---|---|
| macOS — Apple silicon + Intel | Homebrew, or `install.sh` (universal build) |
| Linux — x86_64 + arm64 | `install.sh` (AppImage), or `.deb` / `.rpm` by hand |
| Windows — x64 + arm64 | `install.ps1` |

### Homebrew (macOS)

```sh
brew tap fankrits/zyplus https://github.com/Fankrits/zyplus-editor
brew trust fankrits/zyplus     # Homebrew requires this for third-party casks
brew install --cask zyplus
```

The repo is its own tap, so there is no separate `homebrew-tap` to add. The
cask is bumped automatically on every release. Uninstall with
`brew uninstall --cask zyplus`.

### Script (macOS, Linux)

```sh
curl -fsSL https://raw.githubusercontent.com/Fankrits/zyplus-editor/main/install.sh | sh
```

macOS installs `/Applications/Zyplus.app`. Linux drops the AppImage at
`~/.local/bin/zyplus` — override with `ZYPLUS_BIN_DIR`. Pin a build with
`ZYPLUS_VERSION=0.1.0`. Uninstall by deleting the app or the binary.

### PowerShell (Windows)

```powershell
irm https://raw.githubusercontent.com/Fankrits/zyplus-editor/main/install.ps1 | iex
```

Zyplus is not code-signed on Windows, which has two visible consequences:

- **SmartScreen** may warn when you run the installer by hand — "More info" →
  "Run anyway". The script above clears the download's Mark-of-the-Web, so the
  silent install is unaffected.
- **Controlled folder access** (Defender's ransomware protection) blocks
  unrecognized apps from writing to Documents and Desktop. If creating the
  default `Zyplus` folder fails, the app now says so and offers "Choose another
  location" — a folder you pick through the file dialog is exempt. Otherwise,
  allow Zyplus under Windows Security → Virus & threat protection → Ransomware
  protection → Allow an app through Controlled folder access.

Runs the NSIS installer silently, per user. Pin a build with
`$env:ZYPLUS_VERSION = '0.1.0'`. Windows on arm64 gets the x64
build under emulation — Bun publishes no Windows-arm64 release, so there is
nothing to build a native one with. Uninstall from Add or Remove Programs.

### Manual

Installers for every platform — `.dmg`, `.AppImage`, `.deb`, `.rpm`, `.msi`,
`.exe` — are attached to the
[latest release](https://github.com/Fankrits/zyplus-editor/releases/latest).

> The app is not code-signed or notarized yet. Homebrew and `install.sh` clear
> the macOS quarantine flag for you; a manually downloaded `.dmg` needs
> **right-click → Open** the first time.

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
bun run check         # everything CI runs: tests, tsc, fmt, clippy, cargo test
```

[`check.yml`](.github/workflows/check.yml) runs `bun run check` on macOS,
Linux and Windows for every push and pull request — platform-specific breakage
only shows up if the checks actually run on each platform.

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
bundles for macOS (universal), Linux (x86_64 + arm64) and Windows, publishes
them alongside the `latest.json` the in-app updater reads, and bumps
[`Casks/zyplus.rb`](Casks/zyplus.rb) to the new version and checksum.

## License

[MIT](LICENSE) © Fankrits
