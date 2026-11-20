# Migration from the Tauri + React app

Zyplus moved from a Tauri 2 + React/TypeScript shell (`src/` + `src-tauri/`) to a native Rust desktop app built with [GPUI](https://crates.io/crates/gpui) (`zyplus-gpui`) at the repository root.

## What changed

- **Single native Rust codebase:** The TypeScript/React frontend, Vite/Bun build tooling, and `src-tauri` wrapper were removed. Everything now lives in `Cargo.toml` and `src/*.rs`.
- **Git history preserved:** The last commit containing the Tauri + React application is `c3587de3` (`gpui/` subfolder alongside the web app). You can inspect the previous sources via `git show c3587de3:src-tauri/...` or `git show c3587de3:src/...`.
- **Platform focus:** macOS is the primary target and receives release builds and E2E verification. Windows (setup .exe) and Linux (.deb, AppImage) x86_64 installers are attached to releases as experimental downloads (build, unit-test and launch checks in CI only, no auto-update). Intel Macs are no longer supported.

## Continuity with previous releases

- **Notes folder:** Your notes remain in `<app data>/Zyplus` (`~/Library/Application Support/Zyplus` on macOS).
- **Session file:** Previously stored inside the notes folder (`<app data>/Zyplus/session.json`), which caused it to appear in the sidebar. On launch, `src/session.rs` automatically migrates an existing `Zyplus/session.json` once to `<app data>/Zyplus-session/session.json` (`~/Library/Application Support/Zyplus-session/session.json` on macOS).
- **Bundle identifier:** Remains `com.fankrits.zyplus-editor`, preserving macOS LaunchServices associations and folder permissions (when signed with the release certificate).
- **In-app updater:** `src/updater.rs` reads the same Tauri-format `latest.json` manifest and verifies downloads against the same minisign public key (`PUBKEY_B64`) used by earlier releases, so existing installations can update seamlessly. It also checks `CFBundleExecutable` (`zyplus-gpui`) in the macOS `.app` bundle's `Info.plist` before replacing an installation.

## Intentionally not ported

- **Downloadable JavaScript-bundle extensions:** The GPUI app has no JavaScript runtime or webview. The six built-in extensions (JSON Editor, CSV / TSV Editor, Charts, Mermaid Diagrams, LaTeX Math (KaTeX), and Code & Config Files) are implemented natively in Rust and toggled in **Settings → Extensions**.
- **Rich-editor block drag handle:** The WYSIWYG rich editor supports live preview, inline editing, tables, images, code blocks, callouts, charts, Mermaid diagrams, math blocks, a selection toolbar, a `/` slash menu, and a link popover, without a block drag handle.
- **`.mdown` rich rendering:** `.mdown` is declared in `Info.plist` for Finder file associations, but opens as plain text inside the app (`.md`, `.markdown`, `.mkd`, and `.txt` open as Markdown).
- **Cross-platform PDF fonts:** PDF export (`⌘P`) is built in Rust using `pdf-writer` and `rustybuzz` and relies on macOS system fonts (`Arial`, `Courier New`, `Arial Unicode`, `Thonburi`).
