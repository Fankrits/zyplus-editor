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
cargo test                               # unit tests (paths, file URLs, the single-instance socket,
                                         # theme palettes, atomic saves, session, updater, PDF export)
cargo build --release                    # target/release/zyplus-gpui
```

## Command-line arguments

- `zyplus-gpui <path> [<path> ...]` opens each existing file as a tab and each folder
  as a project in the sidebar. Relative paths resolve against the current directory.
  Anything starting with `-` (for example Finder's `-psn_…`) is ignored.
- A second launch forwards its paths to the running instance and exits. The running
  window gets them (files as tabs, folders in the sidebar) and comes to the front. A
  launch with no paths only raises the window. The socket is
  `<app data>/Zyplus/.gpui-single-instance.sock`, which is hidden from the sidebar
  because the sidebar skips dotfiles. If that path is too long for a Unix socket, the
  socket goes in the temp directory instead. A socket left by a crash is replaced.
- On macOS, Finder's double-click and "Open With" send `file://` URLs to the
  running app (or launch it). They open the same way. A folder the OS passes in is
  added to the sidebar instead.

## File types

- The bundle's `Info.plist` (see `bundle/macos-document-types.plist`) declares
  Markdown (`md`, `markdown`, `mdown`, `mkd`), plain text (`txt`), CSV, TSV and JSON.
- Inside the app, `.md`, `.markdown`, `.mkd` and `.txt` open as Markdown. `.mdown` is
  declared for Finder but is not recognised here, so it opens as plain text. That is
  a known gap, not an intended difference.
- PDF export (⌘P) is offered for `.md`, `.markdown` and `.txt` only.

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

## Saving and session files

- Saves through `tree::write_text_file` (file saves, duplicates, markdown export, the
  welcome note) are atomic. The new content goes to a hidden temp file in the same
  folder, is fsynced, and is renamed over the target (`tree::write_atomic`). The file
  keeps its permission bits, and a symlink keeps pointing at the same file. A
  read-only file is refused, not overwritten. A failed save leaves the old file in
  place and removes the temp file. PDF export has its own atomic write in
  `export_pdf.rs`.
- The session (open folders, tabs, theme, settings) is saved atomically to
  `<app data>/Zyplus-session/session.json`, outside the notes folder. Only paths and
  preferences are stored, never document text. A session file from an older build is
  moved out of the notes folder once, on launch.

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
| Markdown editing (plain mode with highlighting and soft wrap) | Ported |
| Rich/Plain toggle (⌘E) | Ported |
| WYSIWYG rich editing: live preview over the markdown source, tables, images, charts, selection toolbar, `/` menu, link popover | Ported (no block drag handle) |
| Tabs, project folders, session restore, find/replace, autosave | Ported |
| Settings (General, Theme, Extensions, Shortcuts, About), shortcuts sheet, ten palettes | Ported. Theme cards and base colours follow the web's HeroUI tokens |
| CSV and JSON views, `chart` fences | Ported |
| PDF export (⌘P) | Ported, macOS system fonts only (Arial, Courier New, Arial Unicode, Thonburi) |
| File associations, Open With, single instance, menu bar, Cmd-Q | Ported (macOS; see File types above) |
| Update check and download | Ported |
| Update install (replace the running `.app`, relaunch) | Ported, macOS only |
| Mermaid diagrams | Ported. Rendered with `mermaid-svg` in rich edit and preview modes, enabled via Settings → Extensions |
| KaTeX math | Ported. Rendered with RaTeX in rich edit and preview modes, enabled via Settings → Extensions |
| Extensions installed as JavaScript bundles | Not ported. The built-in extensions (CSV, JSON, Charts, Mermaid Diagrams, LaTeX Math (KaTeX), Code & Config Files) are native |

Check the table against the merged source before a release: the viewers, PDF export
and updater were integrated in parallel, and their status changes as they land.
