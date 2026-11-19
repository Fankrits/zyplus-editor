<div align="center">

<img src="docs/logo.svg" alt="Zyplus" width="96" />

# Zyplus

**A markdown editor that gets out of the way.**

Write in rendered rich text or raw source — same file, one keystroke apart.
Built natively in Rust with [GPUI](https://crates.io/crates/gpui).

[![License: MIT](https://img.shields.io/badge/License-MIT-3b82f6.svg?style=flat-square)](LICENSE)
[![Downloads](https://img.shields.io/github/downloads/Fankrits/zyplus-editor/total?style=flat-square&color=3b82f6)](https://github.com/Fankrits/zyplus-editor/releases)

[Features](#features) ·
[Install](#install) ·
[Build & Run](#build-and-run) ·
[Testing](#testing) ·
[Layout](#project-layout) ·
[Releases](#releases)

</div>

---

Zyplus is a macOS-first native desktop application (`zyplus-gpui`). Linux and Windows are not release targets; cross-platform compilation is checked only as an informational CI job.

## Features

- **Two markdown modes (`⌘E`):** WYSIWYG rich editing (live preview over markdown source, tables, images, code blocks, callouts, charts, Mermaid diagrams, math blocks, selection toolbar, `/` slash menu, and link popover) or plain markdown with syntax highlighting and soft wrap.
- **Built-in native viewers & diagrams (Settings → Extensions):**
  - **JSON Editor** (`.json`): tree, table, and text views.
  - **CSV / TSV Editor** (`.csv`, `.tsv`): table and text views.
  - **Charts** (` ```chart `): bar, line, pie, doughnut, and scatter charts rendered natively on GPUI canvas, with fullscreen zoom/pan.
  - **Mermaid Diagrams** (` ```mermaid `): rendered natively via `mermaid-svg`, with fullscreen zoom/pan.
  - **LaTeX Math** (` ```latex `, ` ```math `, ` ```katex `, ` ```tex `): rendered natively via RaTeX.
  - **Code & Config Files:** `syntect`-based syntax highlighting for YAML, TOML, XML, shell, JS/TS, Python, Rust, Go, SQL, and more.
- **Workspace & editing:** Multiple project folders, file tree, tabs, find & replace (`⌘F` / `⌘⌥F`), optional autosave, and atomic file saves.
- **Themes & UI:** 10 palettes (System, Light, Dark, Catppuccin Latte/Frappé/Macchiato/Mocha, Tokyo Night/Storm/Day), native menus, and shortcuts sheet (`⌘/`).
- **PDF export (`⌘P`):** Pure-Rust paginated A4 PDF export for `.md`, `.markdown`, and `.txt` files.
- **In-app updater:** Checks signed GitHub releases (`latest.json` + minisign verification) and replaces/relaunches `Zyplus.app` on macOS.

### Known gaps

- **No downloadable JS-bundle extensions:** There is no JavaScript engine or webview; all six extensions above are compiled natively into the binary.
- **`.mdown` opens as plain text:** `.mdown` is declared in `Info.plist` for Finder associations, while inside the app `.md`, `.markdown`, `.mkd`, and `.txt` open as Markdown.
- **PDF export uses macOS system fonts only:** `Arial`, `Courier New`, `Arial Unicode`, and `Thonburi`.
- **macOS-only release target:** Linux and Windows builds are not packaged, released, or verified with E2E tests.
- **No block drag handle** in the rich editor.

## Install

Zyplus currently distributes macOS universal builds (Apple Silicon + Intel) from [GitHub Releases](https://github.com/Fankrits/zyplus-editor/releases/latest).

### Homebrew (macOS)

```sh
brew tap fankrits/zyplus https://github.com/Fankrits/zyplus-editor
brew trust fankrits/zyplus     # Homebrew requires this for third-party casks
brew install --cask fankrits/zyplus/zyplus
```

This installs `/Applications/Zyplus.app` and links the `zyplus` CLI binary (`Zyplus.app/Contents/MacOS/zyplus-gpui`). The cask definition in [`Casks/zyplus.rb`](Casks/zyplus.rb) is updated automatically on stable releases. Uninstall with `brew uninstall --cask zyplus`.

### Install script (macOS)

```sh
curl -fsSL https://raw.githubusercontent.com/Fankrits/zyplus-editor/main/install.sh | sh
```

Downloads the latest `.dmg`, installs `/Applications/Zyplus.app`, and clears the `com.apple.quarantine` attribute. Pin a specific release with `ZYPLUS_VERSION=0.2.3 sh install.sh`.

### Manual

Download `Zyplus_<version>_universal.dmg` from [GitHub Releases](https://github.com/Fankrits/zyplus-editor/releases/latest) and drag `Zyplus.app` into `/Applications`. If the build is not notarized with a Developer ID, right-click → **Open** on first launch (or clear quarantine with `xattr -dr com.apple.quarantine /Applications/Zyplus.app`).

## Build and Run

Requires a stable [Rust toolchain](https://rustup.rs) and, on macOS, the Xcode Command Line Tools.

```sh
cargo run                                # Debug build, opens welcome screen or restored session
cargo run -- notes/todo.md               # Open a file or folder on start
cargo test                               # Run Rust unit tests
cargo build --release                    # Build target/release/zyplus-gpui
```

### Bundle a macOS `.app`

Run from the repository root:

```sh
./scripts/bundle-macos.sh                # Outputs target/release/bundle/osx/Zyplus.app
./scripts/bundle-macos.sh /Applications  # Or choose the output directory
```

`scripts/bundle-macos.sh` runs `cargo build --release`, writes `Contents/Info.plist` with document types matching [`bundle/macos-document-types.plist`](bundle/macos-document-types.plist), signs the executable and bundle, and verifies with `codesign --verify --strict`.

| Variable | Default | Purpose |
| --- | --- | --- |
| `CARGO_TARGET_DIR` | `<repo>/target` | Where Cargo writes build artifacts |
| `ZYPLUS_ICONS_DIR` | `<repo>/assets/app-icons` | Directory containing `icon.icns` |
| `SIGN_IDENTITY` | `-` (ad-hoc) | `codesign` identity |
| `SKIP_BUILD=1` | unset | Reuse the existing release binary |

An ad-hoc signed app runs on the machine that built it. LaunchServices only accepts default-app claims (**Settings → General → Default app**) for a bundle installed where macOS can index it (such as `/Applications`).

## Behaviour Reference

### Command-line arguments & single instance

- `zyplus-gpui <path> [<path> ...]` opens each existing file as a tab and each folder as a project in the sidebar. Relative paths resolve against the current working directory. Arguments starting with `-` (such as Finder's `-psn_…`) are ignored.
- On Unix, a second launch forwards its paths to the running primary instance over a Unix socket and exits. The running window opens them (files as tabs, folders in the sidebar) and comes to the front; a launch with no paths raises the window.
- The single-instance socket lives at `<app data>/Zyplus/.gpui-single-instance.sock` (a dotfile so the sidebar skips it). If that path reaches the Unix `sun_path` limit (100+ bytes), it falls back to `$TMPDIR/zyplus-gpui-<user>.sock`. Stale sockets left by a crash are replaced automatically.
- On macOS, Finder double-click and **Open With** deliver `file://` URLs through `Application::on_open_urls`.

### File types

- `Info.plist` (`bundle/macos-document-types.plist` and `scripts/bundle-macos.sh`) declares Markdown (`md`, `markdown`, `mdown`, `mkd`), Plain Text (`txt`), CSV (`csv`), TSV (`tsv`), and JSON (`json`).
- Inside the app (`src/view_kind.rs`): `.md`, `.markdown`, `.mkd`, and `.txt` open as Markdown (`rich` / `plain`); `.csv` and `.tsv` open in the CSV viewer (`table` / `text`); `.json` opens in the JSON viewer (`text` / `tree` / `table`); other files open in the syntax-highlighted text editor.
- PDF export (`⌘P`) is enabled for `.md`, `.markdown`, and `.txt`.

### Window & quit behaviour

- `⌘Q` (or **Zyplus → Quit Zyplus**) quits the app. Autosave-enabled and untitled dirty tabs are written automatically; any remaining dirty tab opens the **Save changes** prompt and pauses quit until decided.
- Closing a window with unsaved non-autosaved edits opens the same prompt and keeps the window open.
- On macOS, closing the last window keeps the app running (clicking the Dock icon opens a new window); on other platforms, closing the last window quits.
- Menu bar items (**Zyplus**, **File**, **Edit**, **View**, **Window**) dispatch actions to the focused window.

### Saving & session files

- File saves, duplicates, Markdown exports, and the welcome note go through `tree::write_text_file` → `tree::write_atomic`: contents are written to a hidden temp file in the same directory, fsynced, and renamed over the target, preserving permissions and symlink targets while refusing read-only files. PDF export uses its own atomic write in `src/export_pdf.rs`.
- The session (roots, default folder, open tabs and modes, active file, collapsed/open folders, autosave flag, theme, disabled extensions) is saved atomically to `<app data>/Zyplus-session/session.json` (`~/Library/Application Support/Zyplus-session/session.json` on macOS), outside the notes folder (`<app data>/Zyplus`). Only paths and preferences are stored — never document text. An older `Zyplus/session.json` is migrated out once on launch (`src/session.rs`).

## Testing

The unified test runner is [`scripts/test.sh`](scripts/test.sh):

```sh
scripts/test.sh quick                    # fmt + lint + unit
scripts/test.sh unit                     # cargo test
scripts/test.sh e2e                      # debug build + scripts/e2e/run.py (macOS)
scripts/test.sh all                      # fmt + lint + unit + build + bundle + e2e
```

Available stages:

| Stage | What it runs |
| --- | --- |
| `fmt` | `cargo fmt --check` |
| `lint` | `cargo clippy --all-targets -- -D warnings` |
| `unit` | `cargo test` |
| `build` | `cargo build --release` and verifies `zyplus-gpui` is executable |
| `bundle` | `scripts/bundle-macos.sh`, verifies `Info.plist` and `codesign --verify --strict` (macOS only) |
| `e2e` | `cargo build` and runs `scripts/e2e/run.py --bin <target>/debug/zyplus-gpui` (macOS only) |
| `quick` | `fmt`, `lint`, `unit` |
| `all` | `fmt`, `lint`, `unit`, `build`, `bundle`, `e2e` |

Extra flags after `e2e` (or `--`) are forwarded to `scripts/e2e/run.py` (for example `--list`, `--filter <substr>`, `--jobs <n>`, `--shots`, `--keep`, `--report <path>`).

### Scripted-input runner (`ZYPLUS_TEST_SCRIPT`)

In debug builds (`#[cfg(debug_assertions)]`), `src/test_script.rs` runs scripted UI scenarios when `ZYPLUS_TEST_SCRIPT=/path/to/script.txt` is set (excluded from release builds). It drives keystrokes, editor pointer events, sidebar/menu/dialog/settings/updater actions, dumps (`dump`, `dumpclip`, `state`), and optional window screenshots (`shot` using `ZYPLUS_TEST_WINID` compiled from `tools/winid.swift`). See `scripts/e2e/run.py` for isolated `HOME`/`TMPDIR` orchestration.

## Project Layout

```text
Cargo.toml / Cargo.lock        zyplus-gpui crate and bundle metadata
src/                           Rust application source
  main.rs                      Workspace shell (sidebar, tabs, menus, modals, settings)
  editor.rs / wrap_layout.rs   Plain text / code editor and soft-wrap layout
  rich_*.rs / preview.rs       WYSIWYG rich markdown editor, blocks, tables, overlays
  chart.rs / mermaid.rs / math.rs / diagram_cache.rs / fullscreen_view.rs
  csv_view.rs / json_view.rs   CSV/TSV and JSON viewers
  export_pdf.rs                Native A4 PDF exporter
  highlight.rs / theme.rs      syntect highlighting and 10 palettes
  paths.rs / tree.rs / nav.rs / session.rs / view_kind.rs
  os.rs / app_icon.rs          Single instance, Open With, LaunchServices, embedded assets
  updater.rs                   GitHub release check, minisign verify, macOS install/relaunch
  test_script.rs               Debug-only scripted input runner
assets/                        logo.svg, icon.png, icons/*.svg, app-icons/*
bundle/                        macos-document-types.plist
scripts/
  test.sh                      Unified test runner
  e2e/                         Python stdlib E2E runner and fixtures
  bundle-macos.sh              macOS Zyplus.app packager & codesign helper
  make-signing-cert.sh         Self-signed macOS code-signing secret generator
  release/                     stamp-version.py, make-dmg.sh, make-manifest.py, signer/
  export-gpui-icons.ts         Dev-only SVG icon exporter
tools/winid.swift              macOS window-id lookup helper for screenshots
docs/                          Logo and migration notes (docs/migration-from-tauri.md)
Casks/zyplus.rb                Homebrew cask
install.sh                     macOS DMG install script
.github/workflows/             ci.yml and release.yml
```

## CI and Releases

- **CI ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)):** Runs on every push, pull request, and manual dispatch. The required `macos` job runs `scripts/test.sh` stages (`fmt`, `lint`, `unit`, `build`, `bundle`, `e2e`) and uploads `Zyplus.app.zip` and `e2e-report.json`. An informational (`continue-on-error: true`) `cross-platform` job checks `cargo check --all-targets` on `ubuntu-22.04` and `windows-latest`, and `ci-ok` gates branch protection on `macos`.
- **Releases ([`.github/workflows/release.yml`](.github/workflows/release.yml)):** Triggered by pushing a `vX.Y.Z` tag (or `workflow_dispatch` with optional `dry_run` / `tag`). Builds a universal macOS binary (`aarch64-apple-darwin` + `x86_64-apple-darwin` via `lipo`), packages `Zyplus_<version>_universal.dmg` and `Zyplus.app.tar.gz`, signs the updater tarball with `scripts/release/signer`, generates `latest.json` via `scripts/release/make-manifest.py`, publishes the GitHub release, and updates `Casks/zyplus.rb`, `Cargo.toml`, and `Cargo.lock` on `main` (for stable releases).
  - **Required / optional GitHub Secrets:**
    - `TAURI_SIGNING_PRIVATE_KEY` (required for real releases; optional `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`) — minisign private key matching `PUBKEY_B64` in `src/updater.rs` (kept in Tauri-compatible format for updater continuity; see [`docs/migration-from-tauri.md`](docs/migration-from-tauri.md)).
    - `MACOS_SIGNING_CERT` and `MACOS_SIGNING_CERT_PASSWORD` (optional; falls back to ad-hoc `-` signing when unset) — base64-encoded `.p12` certificate (`Zyplus Self-Signed`, created with `scripts/make-signing-cert.sh`) so macOS preserves Files & Folders permissions across updates.

## Migration

See [`docs/migration-from-tauri.md`](docs/migration-from-tauri.md) for details on the migration from the earlier Tauri + React app.

## License

[MIT](LICENSE) © 2026 Fankrits
