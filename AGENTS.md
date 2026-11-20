# AGENTS.md

Guidance for coding agents working in `zyplus-editor`.

## Stack & Layout

Pure Rust + [GPUI](https://crates.io/crates/gpui) (`0.2.2`, `macos-blade` feature) native desktop app (`zyplus-gpui`), macOS-first. No Node/Bun/Tauri runtime is used to build or run the app.

```text
Cargo.toml / Cargo.lock     Crate `zyplus-gpui`, binary `zyplus-gpui`, bundle metadata
src/                        Rust application source
assets/                     Embedded logo.svg, icons/*.svg, icon.png, app-icons/*
bundle/                     macos-document-types.plist (mirrors scripts/bundle-macos.sh)
scripts/
  test.sh                   Unified test runner (fmt|lint|unit|build|bundle|e2e|quick|all)
  e2e/                      Python stdlib E2E runner and fixtures (macOS)
  bundle-macos.sh           Builds and signs Zyplus.app without cargo-bundle
  make-signing-cert.sh      One-off helper to create self-signed macOS signing secrets
  release/                  Version stamping, DMG creation, manifest generation, signer
  export-gpui-icons.ts      Dev-only icon exporter (assets/icons/*.svg)
tools/winid.swift           Helper printing largest on-screen CGWindowNumber for a PID
docs/                       Logo and migration notes
Casks/zyplus.rb             Homebrew cask definition
install.sh / install.ps1    Install scripts
.github/workflows/          ci.yml (macOS gate + informational Linux/Windows build, test and launch), release.yml (macOS release + experimental Linux/Windows)
```

## Common Commands

```sh
cargo run                            # Debug build, opens welcome screen or restored session
cargo run -- path/to/file.md         # Open file/folder on start
cargo test                           # Run Rust unit tests
cargo build --release                # Build target/release/zyplus-gpui
scripts/test.sh quick                # Run fmt + lint (clippy) + unit
scripts/test.sh all                  # Run fmt + lint + unit + build + bundle + e2e
scripts/bundle-macos.sh              # Assemble target/release/bundle/osx/Zyplus.app
swiftc -O tools/winid.swift -o winid # Build window-id helper for E2E screenshots
```

## Module Roles (`src/`)

- `main.rs`: Root `Zyplus` workspace shell (sidebar tree, tabs, menus, keybindings, modals, settings, single-instance/URL handling).
- `paths.rs`, `tree.rs`, `nav.rs`, `session.rs`: Filename validation (Windows-compatible rules across all OSes), directory tree, atomic file writes (`tree::write_atomic`), and atomic session persistence (`Zyplus-session/session.json`).
- `os.rs`, `app_icon.rs`: CLI arg resolution, `file://` URL decoding, Unix-socket single instance, macOS LaunchServices default-app claims, embedded SVGs/Dock icon.
- `view_kind.rs`, `theme.rs`, `highlight.rs`, `wrap_layout.rs`: File-type routing (`rich`/`plain`, `text`/`tree`/`table`, `table`/`text`), 10 palettes (`THEMES`), `syntect` highlighting, soft-wrap layout.
- `editor.rs`: Plain/code text editor buffer, selection, undo/redo, find/replace.
- `preview.rs`, `rich_model.rs`, `rich_ops.rs`, `rich_blocks.rs`, `rich_table.rs`, `rich_overlay.rs`, `rich_editor.rs`: Markdown parsing (`pulldown-cmark`) and WYSIWYG rich editor with inline/block editing, tables, and overlays (selection toolbar, `/` slash menu, link popover).
- `chart.rs`, `mermaid.rs`, `math.rs`, `diagram_cache.rs`, `fullscreen_view.rs`: Native renderers for `chart`, `mermaid`, and `latex`/`math`/`katex`/`tex` fences plus fullscreen zoom/pan.
- `csv_view.rs`, `json_view.rs`: Native CSV/TSV table+text and JSON tree+table+text views.
- `export_pdf.rs`: Pure-Rust paginated A4 PDF export (`pdf-writer` + `rustybuzz` + `subsetter`) using macOS system fonts.
- `updater.rs`: Blocks on background threads to fetch `latest.json`, verify minisign signatures, and replace/relaunch `Zyplus.app` on macOS.
- `test_script.rs`: Debug-only (`#[cfg(debug_assertions)]`) scripted-input runner.

## Debug Scripted-Input Runner (`ZYPLUS_TEST_SCRIPT`)

Debug builds execute commands from `ZYPLUS_TEST_SCRIPT=/path/to/script.txt` after startup (compiled out of release builds; to profile optimized timing with the runner enabled, build with `CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true cargo build --release`). See `src/test_script.rs` for the full command list (`type`, `key`, `click`, `scroll`, `wait`, `shot`, `dump`, `dumpclip`, `state`, `focus`, `rich-action`, `click-row`, `menu`, `menu-run`, `settings`, `dialog`, `update`, `fullscreen`, `quit`).

Key limitations of the runner:
- `click` dispatches directly to the editor view handlers (`on_mouse_down`, etc.) and skips hit-testing; it cannot reach overlay widgets, dialogs, or context menus. Use `rich-action <name>`, `menu ...`, `menu-run <label>`, `settings <section>`, `dialog ...`, `click-row <path>`, `focus <target>`, and `key ...` instead.
- Script lines are right-trimmed (`type /foo ` drops trailing whitespace).
- `state <file>` reports `selection` offsets in **UTF-8 bytes**, not Unicode scalar counts.

## Operational Pitfalls (Running & Testing Locally)

- **Isolate `HOME` and `TMPDIR` on every test run.** Never let test runs touch the real `$HOME/Library/Application Support/Zyplus` or `$HOME/Library/Application Support/Zyplus-session/session.json`.
  - Note the session path moved: the app reads `$HOME/Library/Application Support/Zyplus-session/session.json` (any `Zyplus/session.json` is migrated once on launch, then removed).
  - The single-instance socket lives at `<HOME>/Library/Application Support/Zyplus/.gpui-single-instance.sock` only when that path is under 100 bytes; otherwise it falls back to `$TMPDIR/zyplus-gpui-$USER.sock`. Give every run a private `TMPDIR`, or a second launch will forward to an already-running instance and exit 0 immediately.
- **Avoid `open -n` inheritance loops on updater relaunches.** Put an `open` shim early on `PATH` that unsets `ZYPLUS_TEST_SCRIPT` and `ZYPLUS_TEST_WINID` before invoking `/usr/bin/open --env HOME=... --env TMPDIR=...`.
- **Replace binaries with `rm -f` before `cp`.** Overwriting a macOS executable in place with `cp` can trigger `Killed: 9` (exit 137) due to code-signing/vnode caching.
- **Record the child PID and kill only that PID.** Do not use broad `pkill -f` patterns (which can match your own shell or other runs). Wrap scripted runs with `perl -e 'alarm shift; exec @ARGV' <secs> ...` (macOS has no `timeout` binary).
- **Avoid shared `CARGO_TARGET_DIR` collisions** if building alongside other checkouts of the same package name (`zyplus-gpui`); use a private `CARGO_TARGET_DIR` when isolating builds.
- **macOS `screencapture` / `sed`:** `screencapture -l <winid>` requires Screen Recording permission and an unlocked display (black grabs or "could not create image from window" indicate permission/display state, not an app bug). Use `sed -i ''` (BSD sed) on macOS.
