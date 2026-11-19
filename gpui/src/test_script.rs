//! Debug-only scripted input for the Zyplus window.
//!
//! Start a debug build with `ZYPLUS_TEST_SCRIPT=/path/to/script.txt` and the
//! window runs the script once it has started. Release builds do not contain
//! this module. One command per line; `#` starts a comment; blank lines are
//! skipped.
//!
//! Keys and mouse events go through gpui's own dispatch path
//! (`Window::dispatch_keystroke` and `Window::dispatch_event`), so keymaps,
//! actions, focus, and the input handler run exactly as they do for a user.
//! Nothing is poked into editor state directly, except the read-only dumps.
//!
//! Commands:
//!
//! ```text
//! type <text>               one keystroke per character; \n presses enter, \t presses tab
//! key <keystroke>           e.g. key cmd-b, key shift-left, key enter, key cmd-alt-1
//! click <x> <y>             left click at window-local logical px
//! dblclick <x> <y>          double click
//! triple <x> <y>            triple click
//! shiftclick <x> <y>        click with shift held
//! cmdclick <x> <y>          click with cmd held (opens a link)
//! drag <x1> <y1> <x2> <y2>  press, move, release
//! scroll <x> <y> <dy>       wheel by dy px; positive scrolls down
//! clip <text>               sets the clipboard (\n = newline)
//! wait <ms>                 pause before the next command
//! shot <file.png>           window capture (see below)
//! dump <file>               writes the active tab's text (rich source in Rich mode)
//! dumpclip <file>           writes the clipboard text
//! state <file>              writes key=value lines: path, mode, focus, rich state
//! echo <text>               logs a marker line
//! focus <editor|query|replace|tree>  moves keyboard focus (find bar fields, sidebar tree)
//! rich-action <name>        runs a rich overlay command (bold, slash-N, etc.)
//! image-path <path>         applies an image path to the active rich editor
//! click-row <path>          a single click on a sidebar row (select, then open or toggle)
//! menu-run <label>          runs an item of the open context menu (disabled items do nothing)
//! replace-all               runs the find bar's Replace All (same call as its button)
//! menu <x> <y> actions      opens the tab bar's ⋯ menu at x, y
//! menu <x> <y> tab <path>   opens a tab's right-click menu
//! menu <x> <y> row <path>   opens a sidebar row's right-click menu
//! menu <x> <y> tree         opens the empty-tree-area menu
//! settings <section>        opens Settings on general|appearance|extensions|shortcuts|about
//! dialog delete <path>      opens the delete confirmation for a path
//! dialog unsaved <path>     opens the unsaved-changes dialog for a tab
//! update check|download|action  the About update button's calls (action presses the shown button)
//! fullscreen open           presses the first chart or Mermaid diagram's full-screen button (a real click)
//! fullscreen wheel <x> <y> <lines> [ctrl]  zoom by wheel lines (ctrl: trackpad pixels with Ctrl held)
//! fullscreen scroll <dx> <dy>    two-finger scroll (pan) over the viewer
//! fullscreen drag <x1> <y1> <x2> <y2>  drag in the viewer
//! fullscreen dblclick <x> <y>    double click in the viewer
//! quit                      exits the app
//! ```
//!
//! Each command is logged to stderr as `[script] line N: <command>`, so the log
//! shows how far a run got.
//!
//! `shot` captures one window with `screencapture -l <id>`. The id comes from a
//! helper named by `ZYPLUS_TEST_WINID`; the helper gets the process id as its
//! only argument and prints the window number (`tools/winid.swift` in the
//! project is one). Without the helper, `shot` captures the whole screen.

use std::process::Command;
use std::time::Duration;

use gpui::{
    px, point, App, AppContext, AsyncApp, ClipboardItem, Context, Entity, Focusable, Keystroke, Modifiers, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ScrollDelta, ScrollWheelEvent, TouchPhase,
    WindowHandle,
};

use crate::rich_editor::RichEditor;
use crate::{Modal, SettingsSection, Zyplus};

#[derive(Clone, Debug)]
enum Cmd {
    Type(String),
    Key(String),
    Click {
        x: f32,
        y: f32,
        count: usize,
        shift: bool,
        cmd: bool,
    },
    Drag {
        from: (f32, f32),
        to: (f32, f32),
    },
    Scroll {
        x: f32,
        y: f32,
        dy: f32,
    },
    Clip(String),
    Wait(u64),
    Shot(String),
    Dump(String),
    DumpClip(String),
    State(String),
    Echo(String),
    Focus(String),
    /// Runs an overlay command by name (see `rich_overlay::OverlayCmd::from_name`).
    RichAction(String),
    /// Replaces the active image url or inserts an image with the given path.
    ImagePath(String),
    /// Opens a context menu by name (`actions`, `tab <path>`, `row <path>`).
    Menu(String),
    ClickRow(String),
    MenuRun(String),
    Settings(String),
    Dialog(String),
    Update(String),
    ReplaceAll,
    /// Full-screen viewer commands (`open`, `wheel`, `scroll`, `drag`, `dblclick`).
    Fullscreen(String),
    Quit,
}

/// Parses a script. Errors name the line that failed.
fn parse(text: &str) -> Result<Vec<(usize, Cmd)>, String> {
    let mut out = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line_no = index + 1;
        let line = raw.trim_end();
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let (word, rest) = match trimmed.split_once(' ') {
            Some((word, rest)) => (word, rest),
            None => (trimmed, ""),
        };
        let num = |s: &str| -> Result<f32, String> {
            s.trim()
                .parse::<f32>()
                .map_err(|_| format!("line {line_no}: expected a number, got {s:?}"))
        };
        let pair = |rest: &str| -> Result<(f32, f32), String> {
            let mut it = rest.split_whitespace();
            let x = num(it.next().unwrap_or(""))?;
            let y = num(it.next().unwrap_or(""))?;
            Ok((x, y))
        };
        let cmd = match word {
            "type" => Cmd::Type(unescape(rest)),
            "key" => Cmd::Key(rest.trim().to_string()),
            "click" | "dblclick" | "triple" | "shiftclick" | "cmdclick" => {
                let (x, y) = pair(rest)?;
                Cmd::Click {
                    x,
                    y,
                    count: match word {
                        "dblclick" => 2,
                        "triple" => 3,
                        _ => 1,
                    },
                    shift: word == "shiftclick",
                    cmd: word == "cmdclick",
                }
            }
            "drag" => {
                let v: Vec<f32> = rest
                    .split_whitespace()
                    .map(num)
                    .collect::<Result<_, _>>()?;
                if v.len() != 4 {
                    return Err(format!("line {line_no}: drag takes four numbers"));
                }
                Cmd::Drag {
                    from: (v[0], v[1]),
                    to: (v[2], v[3]),
                }
            }
            "scroll" => {
                let v: Vec<f32> = rest
                    .split_whitespace()
                    .map(num)
                    .collect::<Result<_, _>>()?;
                if v.len() != 3 {
                    return Err(format!("line {line_no}: scroll takes x y dy"));
                }
                Cmd::Scroll {
                    x: v[0],
                    y: v[1],
                    dy: v[2],
                }
            }
            "clip" => Cmd::Clip(unescape(rest)),
            "wait" => Cmd::Wait(
                rest.trim()
                    .parse()
                    .map_err(|_| format!("line {line_no}: wait takes milliseconds"))?,
            ),
            "shot" => Cmd::Shot(rest.trim().to_string()),
            "dump" => Cmd::Dump(rest.trim().to_string()),
            "dumpclip" => Cmd::DumpClip(rest.trim().to_string()),
            "state" => Cmd::State(rest.trim().to_string()),
            "echo" => Cmd::Echo(rest.to_string()),
            "focus" => Cmd::Focus(rest.trim().to_string()),
            "rich-action" => Cmd::RichAction(rest.trim().to_string()),
            "image-path" => Cmd::ImagePath(rest.trim().to_string()),
            "menu" => Cmd::Menu(rest.trim().to_string()),
            "click-row" => Cmd::ClickRow(rest.trim().to_string()),
            "menu-run" => Cmd::MenuRun(rest.trim().to_string()),
            "settings" => Cmd::Settings(rest.trim().to_string()),
            "dialog" => Cmd::Dialog(rest.trim().to_string()),
            "update" => Cmd::Update(rest.trim().to_string()),
            "replace-all" => Cmd::ReplaceAll,
            "fullscreen" => Cmd::Fullscreen(rest.trim().to_string()),
            "quit" => Cmd::Quit,
            other => return Err(format!("line {line_no}: unknown command {other:?}")),
        };
        out.push((line_no, cmd));
    }
    Ok(out)
}

/// `\n` and `\t` escapes, so a script can type multi-line text on one line.
fn unescape(s: &str) -> String {
    s.replace("\\n", "\n").replace("\\t", "\t")
}

/// Starts the script if `ZYPLUS_TEST_SCRIPT` is set. Called once, after the
/// main window opens.
pub fn spawn(handle: WindowHandle<Zyplus>, cx: &mut App) {
    let Ok(path) = std::env::var("ZYPLUS_TEST_SCRIPT") else {
        return;
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) => {
            eprintln!("[script] cannot read {path}: {err}");
            return;
        }
    };
    let commands = match parse(&text) {
        Ok(commands) => commands,
        Err(err) => {
            eprintln!("[script] {err}");
            return;
        }
    };
    eprintln!("[script] {} commands from {path}", commands.len());
    cx.spawn(async move |cx| {
        cx.background_executor()
            .timer(Duration::from_millis(900))
            .await;
        let _ = handle.update(cx, |root, window, cx| root.focus_editor(window, cx));
        for (line, cmd) in commands {
            eprintln!("[script] line {line}: {cmd:?}");
            let quit = matches!(cmd, Cmd::Quit);
            execute(&cmd, handle, cx).await;
            if quit {
                return;
            }
            // Let a frame render between commands, as a user's events would be spaced.
            cx.background_executor()
                .timer(Duration::from_millis(40))
                .await;
        }
        eprintln!("[script] done");
    })
    .detach();
}

async fn execute(cmd: &Cmd, handle: WindowHandle<Zyplus>, cx: &mut AsyncApp) {
    match cmd {
        Cmd::Type(text) => {
            for ch in text.chars() {
                match ch {
                    '\n' => key(handle, cx, "enter"),
                    '\t' => key(handle, cx, "tab"),
                    ch => {
                        let stroke = Keystroke {
                            modifiers: Modifiers::default(),
                            key: ch.to_string(),
                            key_char: Some(ch.to_string()),
                        };
                        dispatch_key(handle, cx, stroke);
                    }
                }
                cx.background_executor()
                    .timer(Duration::from_millis(4))
                    .await;
            }
        }
        Cmd::Key(spec) => key(handle, cx, spec),
        Cmd::Click {
            x,
            y,
            count,
            shift,
            cmd,
        } => {
            let modifiers = Modifiers {
                shift: *shift,
                platform: *cmd,
                ..Modifiers::default()
            };
            let position = point(px(*x), px(*y));
            for n in 1..=*count {
                mouse_down(handle, cx, position, n, modifiers);
                mouse_up(handle, cx, position, n, modifiers);
            }
        }
        Cmd::Drag { from, to } => {
            let p0 = point(px(from.0), px(from.1));
            let p1 = point(px(to.0), px(to.1));
            mouse_down(handle, cx, p0, 1, Modifiers::default());
            cx.background_executor()
                .timer(Duration::from_millis(16))
                .await;
            rich_mouse(
                handle,
                cx,
                Mouse::Move(MouseMoveEvent {
                    position: p1,
                    pressed_button: Some(MouseButton::Left),
                    modifiers: Modifiers::default(),
                }),
            );
            cx.background_executor()
                .timer(Duration::from_millis(16))
                .await;
            mouse_up(handle, cx, p1, 1, Modifiers::default());
        }
        Cmd::Scroll { x, y, dy } => rich_mouse(
            handle,
            cx,
            Mouse::Wheel(ScrollWheelEvent {
                position: point(px(*x), px(*y)),
                // gpui reports a downward wheel as negative y; the script says dy > 0 is down.
                delta: ScrollDelta::Pixels(point(px(0.0), px(-*dy))),
                modifiers: Modifiers::default(),
                touch_phase: TouchPhase::Moved,
            }),
        ),
        Cmd::Clip(text) => {
            let text = text.clone();
            let _ = cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string(text)));
        }
        Cmd::Wait(ms) => {
            cx.background_executor()
                .timer(Duration::from_millis(*ms))
                .await;
        }
        Cmd::Shot(path) => {
            // Let the last edit reach the screen first.
            cx.background_executor()
                .timer(Duration::from_millis(250))
                .await;
            shot(path);
        }
        Cmd::Dump(path) => {
            let text = handle
                .update(cx, |root, _, cx| root.test_source(cx))
                .ok()
                .flatten()
                .unwrap_or_default();
            write_file(path, &text);
        }
        Cmd::DumpClip(path) => {
            let text = cx
                .update(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
                .ok()
                .flatten()
                .unwrap_or_default();
            write_file(path, &text);
        }
        Cmd::State(path) => {
            let text = handle
                .update(cx, |root, window, cx| root.test_state(window, cx))
                .unwrap_or_else(|err| format!("error={err}\n"));
            write_file(path, &text);
        }
        Cmd::Echo(text) => eprintln!("[script] echo: {text}"),
        Cmd::Focus(target) => {
            let target = target.clone();
            let _ = cx.update_window(handle.into(), |view, window, cx| {
                if let Ok(root) = view.downcast::<Zyplus>() {
                    root.update(cx, |root, cx| root.test_focus(&target, window, cx));
                }
            });
        }
        Cmd::Fullscreen(spec) => {
            if spec == "open" {
                let target = cx
                    .update_window(handle.into(), |view, _window, cx| {
                        let root = view.downcast::<Zyplus>().ok()?;
                        let rich = root.read(cx).test_rich()?;
                        rich.read(cx).test_fullscreen_button()
                    })
                    .ok()
                    .flatten();
                match target {
                    Some(position) => mouse_down(handle, cx, position, 1, Modifiers::default()),
                    None => eprintln!("[script] fullscreen open: no full-screen button on screen"),
                }
            } else {
                let spec = spec.clone();
                let _ = handle.update(cx, |root, _, cx| root.test_fullscreen(&spec, cx));
            }
        }
        Cmd::ReplaceAll => {
            let _ = handle.update(cx, |root, _, cx| root.replace_all(cx));
        }
        Cmd::Menu(spec) => {
            let spec = spec.clone();
            let _ = handle.update(cx, |root, _, cx| root.test_menu(&spec, cx));
        }
        Cmd::ClickRow(path) => {
            let path = path.clone();
            let _ = cx.update_window(handle.into(), |view, window, cx| {
                if let Ok(root) = view.downcast::<Zyplus>() {
                    root.update(cx, |root, cx| root.test_click_row(&path, window, cx));
                }
            });
        }
        Cmd::MenuRun(label) => {
            let label = label.clone();
            let _ = cx.update_window(handle.into(), |view, window, cx| {
                if let Ok(root) = view.downcast::<Zyplus>() {
                    root.update(cx, |root, cx| root.test_menu_run(&label, window, cx));
                }
            });
        }
        Cmd::Settings(name) => {
            let name = name.clone();
            let _ = handle.update(cx, |root, _, cx| root.test_settings(&name, cx));
        }
        Cmd::Dialog(spec) => {
            let spec = spec.clone();
            let _ = handle.update(cx, |root, _, cx| root.test_dialog(&spec, cx));
        }
        Cmd::Update(sub) => {
            let sub = sub.clone();
            let _ = handle.update(cx, |root, _, cx| root.test_update(&sub, cx));
        }
        Cmd::RichAction(name) => {
            let name = name.clone();
            let _ = cx.update_window(handle.into(), |view, window, cx| {
                let rich = view
                    .downcast::<Zyplus>()
                    .ok()
                    .and_then(|root| root.read(cx).test_rich());
                if let Some(rich) = rich {
                    rich.update(cx, |editor, cx| {
                        if !editor.overlay_action(&name, window, cx) {
                            eprintln!("[script] unknown rich action {name:?}");
                        }
                    });
                }
            });
        }
        Cmd::ImagePath(path_str) => {
            let path = std::path::PathBuf::from(path_str);
            let _ = cx.update_window(handle.into(), |view, _window, cx| {
                let rich = view
                    .downcast::<Zyplus>()
                    .ok()
                    .and_then(|root| root.read(cx).test_rich());
                if let Some(rich) = rich {
                    rich.update(cx, |editor, cx| {
                        editor.apply_image_path(&path, cx);
                    });
                }
            });
        }
        Cmd::Quit => {
            let _ = cx.update(|cx| cx.quit());
        }
    }
}

fn key(handle: WindowHandle<Zyplus>, cx: &mut AsyncApp, spec: &str) {
    match Keystroke::parse(spec) {
        Ok(stroke) => dispatch_key(handle, cx, stroke),
        Err(err) => eprintln!("[script] bad keystroke {spec:?}: {err}"),
    }
}

/// Sends a key through the window's real dispatch. This goes through
/// `update_window`, not `WindowHandle::update`, because dispatch updates the
/// root view itself and that view must not already be leased.
fn dispatch_key(handle: WindowHandle<Zyplus>, cx: &mut AsyncApp, stroke: Keystroke) {
    let _ = cx.update_window(handle.into(), |_, window, cx| {
        window.dispatch_keystroke(stroke, cx);
    });
}

/// A mouse event for the rich editor. gpui's own `dispatch_event` returns a
/// crate-private type, so the rich editor's element handlers are called
/// directly with the same event values the platform would deliver.
enum Mouse {
    Down(MouseDownEvent),
    Up(MouseUpEvent),
    Move(MouseMoveEvent),
    Wheel(ScrollWheelEvent),
}

fn rich_mouse(handle: WindowHandle<Zyplus>, cx: &mut AsyncApp, event: Mouse) {
    let result = cx.update_window(handle.into(), |view, window, cx| {
        let Ok(root) = view.downcast::<Zyplus>() else {
            return false;
        };
        let Some(rich) = root.read(cx).test_rich() else {
            return false;
        };
        rich.update(cx, |editor, cx| match &event {
            Mouse::Down(e) => editor.on_mouse_down(e, window, cx),
            Mouse::Up(e) => editor.on_mouse_up(e, window, cx),
            Mouse::Move(e) => editor.on_mouse_move(e, window, cx),
            Mouse::Wheel(e) => editor.on_scroll_wheel(e, window, cx),
        });
        true
    });
    if !matches!(result, Ok(true)) {
        eprintln!("[script] mouse event ignored: the active tab is not in Rich mode");
    }
}

fn mouse_down(
    handle: WindowHandle<Zyplus>,
    cx: &mut AsyncApp,
    position: gpui::Point<gpui::Pixels>,
    click_count: usize,
    modifiers: Modifiers,
) {
    rich_mouse(
        handle,
        cx,
        Mouse::Down(MouseDownEvent {
            button: MouseButton::Left,
            position,
            modifiers,
            click_count,
            first_mouse: false,
        }),
    );
}

fn mouse_up(
    handle: WindowHandle<Zyplus>,
    cx: &mut AsyncApp,
    position: gpui::Point<gpui::Pixels>,
    click_count: usize,
    modifiers: Modifiers,
) {
    rich_mouse(
        handle,
        cx,
        Mouse::Up(MouseUpEvent {
            button: MouseButton::Left,
            position,
            modifiers,
            click_count,
        }),
    );
}

fn shot(path: &str) {
    let pid = std::process::id().to_string();
    let window_id = std::env::var("ZYPLUS_TEST_WINID")
        .ok()
        .and_then(|helper| Command::new(helper).arg(&pid).output().ok())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|id| !id.is_empty());
    let mut cmd = Command::new("screencapture");
    cmd.arg("-x").arg("-o");
    match &window_id {
        Some(id) => {
            cmd.arg("-l").arg(id);
        }
        None => eprintln!("[script] no ZYPLUS_TEST_WINID; capturing the full screen"),
    }
    match cmd.arg(path).status() {
        Ok(status) if status.success() => eprintln!("[script] shot {path} (window {window_id:?})"),
        Ok(status) => eprintln!("[script] screencapture failed: {status}"),
        Err(err) => eprintln!("[script] screencapture did not start: {err}"),
    }
}

fn write_file(path: &str, text: &str) {
    if let Err(err) = std::fs::write(path, text) {
        eprintln!("[script] cannot write {path}: {err}");
    }
}

impl Zyplus {
    /// Opens a context menu at window-local `x y`: `actions`, `tab <path>` or
    /// `row <path>`. Mirrors the mouse handlers, which scripted clicks can't reach.
    fn test_menu(&mut self, spec: &str, cx: &mut Context<Self>) {
        let mut parts = spec.split_whitespace();
        let (Some(x), Some(y), Some(kind)) = (parts.next(), parts.next(), parts.next()) else {
            eprintln!("[script] menu takes: x y actions|tab <path>|row <path>");
            return;
        };
        let rest = parts.collect::<Vec<_>>().join(" ");
        let (Ok(x), Ok(y)) = (x.parse::<f32>(), y.parse::<f32>()) else {
            eprintln!("[script] menu position must be numbers");
            return;
        };
        let items = match kind {
            "actions" => self.actions_menu(),
            "tab" => self.tab_menu(&rest),
            "tree" => self.tree_area_menu(),
            "row" => {
                let is_folder = std::path::Path::new(&rest).is_dir();
                let is_root = self.roots.iter().any(|root| root == &rest);
                self.row_menu(&rest, is_folder, is_root)
            }
            other => {
                eprintln!("[script] unknown menu {other:?}");
                return;
            }
        };
        self.open_menu(x, y, items, cx);
    }

    /// A single left click on a sidebar row: focus the tree, select the row, then
    /// open a file (when the tree opens that type) or toggle a folder.
    fn test_click_row(&mut self, path: &str, window: &mut gpui::Window, cx: &mut Context<Self>) {
        let Some(row) = crate::tree::visible_rows(&self.tree, &self.open_folders)
            .into_iter()
            .find(|row| row.id == path)
        else {
            eprintln!("[script] click-row: no visible row {path:?}");
            return;
        };
        self.refocus_editor = false;
        window.focus(&self.tree_focus);
        self.select_row(path, &gpui::Modifiers::default(), cx);
        if row.is_folder {
            self.toggle_folder(path, cx);
        } else if crate::nav::tree_opens(&row.name, |id| self.extension_enabled(id)) {
            self.open_document(path, Some(window), cx);
        }
        cx.notify();
    }

    /// Runs the open context menu's item with this label, like a click on it.
    /// Disabled items do nothing, as they do on screen.
    fn test_menu_run(&mut self, label: &str, window: &mut gpui::Window, cx: &mut Context<Self>) {
        let Some(item) = self
            .menu
            .as_ref()
            .and_then(|menu| menu.items.iter().find(|item| item.label == label).cloned())
        else {
            eprintln!("[script] menu-run: no open menu item {label:?}");
            return;
        };
        if item.disabled {
            eprintln!("[script] menu-run: {label:?} is disabled");
            return;
        }
        self.run_menu(item.action, window, cx);
    }

    /// Opens a confirmation dialog: `delete <path>` or `unsaved <path>`.
    fn test_dialog(&mut self, spec: &str, cx: &mut Context<Self>) {
        let (kind, path) = spec.split_once(' ').unwrap_or((spec, ""));
        self.dialog_error = None;
        self.modal = match kind {
            "delete" => Some(Modal::Delete {
                paths: vec![path.to_string()],
                is_folder: false,
            }),
            "unsaved" => Some(Modal::Unsaved {
                path: path.to_string(),
            }),
            other => {
                eprintln!("[script] unknown dialog {other:?}");
                return;
            }
        };
        cx.notify();
    }

    /// Opens Settings on the named section (the sidebar and tab bar buttons do the same).
    fn test_settings(&mut self, name: &str, cx: &mut Context<Self>) {
        let section = match name {
            "general" => SettingsSection::General,
            "appearance" => SettingsSection::Appearance,
            "extensions" => SettingsSection::Extensions,
            "shortcuts" => SettingsSection::Shortcuts,
            "about" => SettingsSection::About,
            other => {
                eprintln!("[script] unknown settings section {other:?}");
                return;
            }
        };
        self.open_settings(section, cx);
    }

    /// `update check|download|action`: the same calls the About panel makes.
    /// `action` presses whatever single button the panel shows now.
    fn test_update(&mut self, sub: &str, cx: &mut Context<Self>) {
        match sub {
            "check" => self.check_for_updates(cx),
            "download" => self.download_update(cx),
            "action" => self.update_action(cx),
            other => eprintln!("[script] unknown update command {other:?}"),
        }
    }

    /// The rich editor of the active tab, when that tab is in Rich mode.
    /// `fullscreen wheel|scroll|drag|dblclick ...`: drives the full-screen viewer's
    /// handlers with the same events the window would send.
    pub(crate) fn test_fullscreen(&mut self, spec: &str, cx: &mut Context<Self>) {
        let nums: Vec<f32> = spec.split_whitespace().skip(1).filter_map(|s| s.parse().ok()).collect();
        let word = spec.split_whitespace().next().unwrap_or("");
        let at = |x: f32, y: f32| point(px(x), px(y));
        let ctrl = spec.contains("ctrl");
        match (word, nums.as_slice()) {
            ("wheel", [x, y, lines, ..]) => {
                let (position, delta, modifiers) = if ctrl {
                    (at(*x, *y), ScrollDelta::Pixels(point(px(0.0), px(*lines))), Modifiers { control: true, ..Default::default() })
                } else {
                    (at(*x, *y), ScrollDelta::Lines(point(0.0, *lines)), Modifiers::default())
                };
                let event = ScrollWheelEvent { position, delta, modifiers, touch_phase: TouchPhase::Moved };
                self.fs_wheel(&event, cx);
            }
            ("scroll", [dx, dy, ..]) => {
                let event = ScrollWheelEvent {
                    position: at(400.0, 300.0),
                    delta: ScrollDelta::Pixels(point(px(*dx), px(*dy))),
                    modifiers: Modifiers::default(),
                    touch_phase: TouchPhase::Moved,
                };
                self.fs_wheel(&event, cx);
            }
            ("dblclick", [x, y, ..]) => {
                let event = MouseDownEvent {
                    button: MouseButton::Left,
                    position: at(*x, *y),
                    modifiers: Modifiers::default(),
                    click_count: 2,
                    first_mouse: false,
                };
                self.fs_mouse_down(&event, cx);
            }
            ("drag", [x1, y1, x2, y2, ..]) => {
                let down = MouseDownEvent {
                    button: MouseButton::Left,
                    position: at(*x1, *y1),
                    modifiers: Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                };
                self.fs_mouse_down(&down, cx);
                let mv = MouseMoveEvent {
                    position: at(*x2, *y2),
                    pressed_button: Some(MouseButton::Left),
                    modifiers: Modifiers::default(),
                };
                self.fs_mouse_move(&mv, cx);
                if let Some(viewer) = self.fullscreen.as_mut() {
                    viewer.release();
                }
            }
            _ => eprintln!("[script] unknown fullscreen command {spec:?}"),
        }
    }

    pub(crate) fn test_rich(&self) -> Option<Entity<RichEditor>> {
        let tab = self.active_tab()?;
        if tab.is_rich() {
            tab.rich.clone()
        } else {
            None
        }
    }

    /// The text of the active tab as the user sees it: the rich editor's source
    /// in Rich mode, otherwise the tab's mirrored content.
    pub(crate) fn test_source(&self, cx: &App) -> Option<String> {
        let tab = self.active_tab()?;
        if tab.is_rich() {
            if let Some(rich) = &tab.rich {
                return Some(rich.read(cx).text().to_string());
            }
        }
        Some(tab.content.clone())
    }

    /// Moves focus to one of the editors (or find-bar fields) by name.
    pub(crate) fn test_focus(&mut self, target: &str, window: &mut gpui::Window, cx: &mut Context<Self>) {
        self.refocus_editor = false;
        let field = match (target, &self.find) {
            ("replace", Some(find)) => Some(find.replace.read(cx).focus_handle(cx)),
            ("query", Some(find)) => Some(find.query.read(cx).focus_handle(cx)),
            ("tree", _) => Some(self.tree_focus.clone()),
            _ => None,
        };
        match field {
            Some(handle) => window.focus(&handle),
            None => self.focus_editor(window, cx),
        }
    }

    /// Key=value lines describing what the window shows, for assertions.
    pub(crate) fn test_state(&self, window: &gpui::Window, cx: &App) -> String {
        let mut out = String::new();
        let tab = self.active_tab();
        out.push_str(&format!("active={}\n", tab.map_or("", |t| t.path.as_str())));
        out.push_str(&format!("mode={}\n", tab.map_or("", |t| t.mode.as_str())));
        out.push_str(&format!("dirty={}\n", tab.is_some_and(|t| t.dirty)));
        out.push_str(&format!("tabs={}\n", self.tabs.len()));
        if let Some(tab) = tab {
            if let Some(rich) = &tab.rich {
                let rich = rich.read(cx);
                out.push_str(&format!("rich_focused={}\n", rich.focus_handle(cx).is_focused(window)));
                out.push_str(&rich.state_summary());
            }
        }
        out.push_str(&format!("find_open={}\n", self.find.is_some()));
        out.push_str(&format!(
            "modal={}\n",
            match &self.modal {
                None => "none",
                Some(Modal::Rename { .. }) => "rename",
                Some(Modal::NewFolder { .. }) => "new-folder",
                Some(Modal::Delete { .. }) => "delete",
                Some(Modal::Unsaved { .. }) => "unsaved",
                Some(Modal::Settings { .. }) => "settings",
            }
        ));
        out.push_str(&format!("menu_open={}\n", self.menu.is_some()));
        out.push_str(&format!("welcome={}\n", self.welcome));
        out.push_str(&format!("quitting={}\n", self.quitting));
        out.push_str(&format!("roots={}\n", self.roots.len()));
        out.push_str(&format!(
            "error={}\n",
            self.error.as_ref().map_or(String::new(), |e| e.to_string().replace('\n', " "))
        ));
        out.push_str(&format!(
            "focus_on_shell={}\n",
            self.focus.is_focused(window)
        ));
        if let Some(find) = &self.find {
            out.push_str(&format!("find_query={}\n", find.query_text));
            out.push_str(&format!("find_index={}\n", find.index));
            out.push_str(&format!("find_total={}\n", find.matches.len()));
            out.push_str(&format!(
                "find_query_focused={}\n",
                find.query.read(cx).focus_handle(cx).is_focused(window)
            ));
            out.push_str(&format!(
                "find_replace_focused={}\n",
                find.replace.read(cx).focus_handle(cx).is_focused(window)
            ));
        }
        if let Some(searchable) = self.active_searchable(cx) {
            let sel = searchable.selection(cx);
            out.push_str(&format!("selection={}..{}\n", sel.start, sel.end));
        }
        out.push_str(&format!(
            "tab_focused={}\n",
            tab.is_some_and(|t| t.focus_handle(cx).is_focused(window))
        ));
        out.push_str(&format!("tree_focused={}\n", self.tree_focus.is_focused(window)));
        out.push_str(&format!(
            "tree_cursor={}\n",
            self.selection_anchor.clone().unwrap_or_default()
        ));
        out.push_str(&format!("tree_selected={}\n", self.selected.join("|")));
        let menu: Vec<String> = self
            .menu
            .as_ref()
            .map(|menu| {
                menu.items
                    .iter()
                    .map(|item| format!("{}{}", item.label, if item.disabled { "!" } else { "" }))
                    .collect()
            })
            .unwrap_or_default();
        out.push_str(&format!("menu={}\n", menu.join("|")));
        out.push_str(&format!("sidebar_collapsed={}\n", self.sidebar_collapsed));
        let modal = match &self.modal {
            Some(Modal::Rename { .. }) => "rename",
            Some(Modal::NewFolder { .. }) => "new-folder",
            Some(Modal::Delete { .. }) => "delete",
            Some(Modal::Unsaved { .. }) => "unsaved",
            Some(Modal::Settings { .. }) => "settings",
            None => "",
        };
        out.push_str(&format!("modal={modal}\n"));
        match &self.fullscreen {
            Some(viewer) => out.push_str(&format!(
                "fullscreen=open zoom={:.3} dx={:.1} dy={:.1}\n",
                viewer.view.zoom, viewer.view.dx, viewer.view.dy
            )),
            None => out.push_str("fullscreen=closed\n"),
        }
        let panel = self.update_status.panel_view();
        out.push_str(&format!("update={}\n", self.update_status.kind()));
        out.push_str(&format!("update_button={}\n", panel.button));
        out.push_str(&format!("update_text={}\n", panel.text.replace('\n', " ")));
        out.push_str(&format!(
            "update_pill={}\n",
            self.update_status.pill_label().unwrap_or_default()
        ));
        out
    }
}
