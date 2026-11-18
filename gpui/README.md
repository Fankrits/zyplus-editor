# Zyplus (GPUI build)

The native desktop build of Zyplus, written with [GPUI](https://crates.io/crates/gpui)
0.2.2. The web/Tauri app lives in the parent repository (`../src`, `../src-tauri`).
This crate is the GPUI port. It is macOS-first; other platforms compile but get less
testing.

## Build and run

Requires a Rust toolchain (stable) and, on macOS, the Xcode command line tools.

```sh
cargo run                                # debug build, opens the welcome screen
cargo run -- notes/todo.md              # open a file on start
cargo test                               # unit tests (path, URL and single-instance helpers)
cargo build --release                    # target/release/zyplus-gpui
```

## Command-line arguments

- `zyplus-gpui <path> [<path> ...]` opens each existing file as a tab. A folder
  argument is ignored, because a folder is not a document. Relative paths resolve
  against the current directory. Anything starting with `-` (for example
  Finder's `-psn_…`) is ignored.
- A second launch forwards its files to the running instance and exits. The
  running window gets the files as tabs and comes to the front. A launch with no
  files only raises the window. The socket is
  `<app data>/Zyplus/.gpui-single-instance.sock`, which is hidden from the sidebar
  because the sidebar skips dotfiles. A socket left by a crash is replaced.
- On macOS, Finder's double-click and "Open With" send `file://` URLs to the
  running app (or launch it). They open the same way. A folder the OS passes in is
  added to the sidebar instead.

## Window and quit behaviour

- Cmd-Q (also the Quit item) quits the app. If any tab has unsaved edits, the
  Save-changes prompt opens for the first one and the app stays open. Save or
  discard, then quit again. Tabs that autosave, and untitled tabs, are saved
  without asking, as they are when a tab closes.
- Closing the window with unsaved edits opens the same prompt and keeps the window.
- On macOS, closing the last window keeps the app running. Clicking the Dock icon
  opens a new window. On other platforms, closing the last window quits.
- Menus: Zyplus (About, Settings, Hide, Hide Others, Quit), File, Edit, View,
  Window (Minimize, Zoom). Menu items send their action to the focused window, so
  the same handlers run as for the shortcuts.

## Bundle a macOS app

```sh
./bundle-macos.sh                        # <target>/release/bundle/osx/Zyplus.app
./bundle-macos.sh /Applications          # or choose the output directory
```

The script runs a release build, writes `Info.plist` with the document types
(Markdown, plain text, CSV, TSV, JSON), signs the bundle, and checks it with
`codesign --verify --strict`. Environment overrides:

| Variable | Default | Purpose |
| --- | --- | --- |
| `CARGO_TARGET_DIR` | `<crate>/target` | where the binary is built |
| `ZYPLUS_ICONS_DIR` | `<crate>/../src-tauri/icons` | folder holding `icon.icns` |
| `SIGN_IDENTITY` | `-` (ad-hoc) | codesign identity |
| `SKIP_BUILD=1` | unset | reuse the release binary |

An ad-hoc signed app runs on the machine that built it. To claim a default app in
Settings, LaunchServices needs Zyplus in `/Applications`.

## Feature parity with the web/Tauri app

| Feature | Status |
| --- | --- |
| Markdown editing (plain mode with highlighting) | Ported |
| Rich/Plain toggle (⌘E) | Ported |
| Tabs, project folders, session restore, find/replace, autosave | Ported |
| Settings modal, shortcuts sheet, ten palettes | Ported |
| File associations, Open With, single instance, menu bar, Cmd-Q | Ported |
| WYSIWYG rich editing: live-preview over the markdown source, tables, images, charts, selection toolbar, `/` menu, link popover | Ported (no block drag handle) |
| Mermaid diagrams | Not ported |
| KaTeX math | Not ported |
| Extensions installed as JavaScript bundles | Not ported |
| Signed auto-install updater | Not ported |

Check the table against the merged source before a release: the viewers, PDF export
and updater are being integrated in parallel, and their status changes as they land.
