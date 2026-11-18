//! Zyplus for the desktop: the workspace shell around the editor.
//!
//! The React app splits its state into a reducer, a store and a handful of
//! components. Here the same shape lives in one entity: the project tree, the
//! open folders, the tabs and which one is active. Each tab owns an `Editor`
//! entity, so switching tabs is a swap and typing never re-reads the file.
//!
//! Everything the web build does on the desktop is ported here: session
//! restore, the ten palettes, find/replace, a rendered markdown mode, context
//! menus, settings, autosave and the keyboard shortcuts. The two pieces that
//! cannot cross over are noted where they would live: PDF export needs a print
//! stack, and extensions are downloaded JavaScript bundles.

mod app_icon;
mod chart;
mod csv_view;
mod editor;
mod export_pdf;
mod highlight;
mod json_view;
mod os;
mod paths;
mod preview;
mod rich_blocks;
mod rich_editor;
mod rich_model;
mod rich_ops;
mod session;
mod theme;
mod tree;
#[cfg(debug_assertions)]
mod test_script;
mod updater;
mod wrap_layout;
mod view_kind;

use std::collections::BTreeSet;
use std::ops::Range;
use std::path::Path;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    actions, div, px, rgb, rgba, size, AnyElement, App, Application, Bounds, ClipboardItem, Context,
    Div, Entity, FocusHandle, Focusable, FontWeight, KeyBinding, MouseButton, PathPromptOptions,
    Rgba, SharedString, Stateful, TitlebarOptions, Window, WindowBounds, WindowControlArea,
    WindowOptions,
};

use csv_view::CsvView;
use editor::Editor;
use rich_editor::RichEditor;
use json_view::JsonView;
use theme::Theme;
use tree::TreeNode;
use view_kind::ViewKind;

actions!(
    zyplus,
    [
        Save,
        ExportMd,
        ExportPdf,
        NewFile,
        NewFolder,
        CloseTab,
        NextTab,
        PrevTab,
        ToggleSidebar,
        ToggleMode,
        OpenFile,
        OpenFolder,
        Find,
        Replace,
        Settings,
        Shortcuts,
        CopyMarkdown,
        CopyPath,
        DeleteSelection,
        CloseOverlay,
        GoToTab1,
        GoToTab2,
        GoToTab3,
        GoToTab4,
        GoToTab5,
        GoToTab6,
        GoToTab7,
        GoToTab8,
        GoToTab9
    ]
);

const WELCOME_NOTE_NAME: &str = "Welcome.md";
const VERSION: &str = "0.2.3";
/// Space the macOS traffic lights take at the top-left of the window.
const TRAFFIC_LIGHT_INSET: f32 = 84.0;

const WELCOME_NOTE: &str = r#"# Welcome to Zyplus

**A markdown editor that gets out of the way.** This note lives in your Zyplus
folder. Edit it, rename it, or delete it — it won't come back.

## The basics

- **Your notes are plain files.** Everything is a `.md` file in a real folder on
  disk, with no database and no lock-in.
- **New files land here.** This folder is the default; open another with
  `Cmd-O`.
- **Open a single file** with `Cmd-Shift-O`.

## Handy shortcuts

| Action | Shortcut |
|---|---|
| New file | `Cmd-N` |
| Save | `Cmd-S` |
| Close tab | `Cmd-W` |
| Toggle sidebar | `Cmd-B` |
| Open folder | `Cmd-O` |
| Open file | `Cmd-Shift-O` |

Happy writing!
"#;

struct Tab {
    path: String,
    title: String,
    /// The last content known to be on disk.
    saved: String,
    dirty: bool,
    /// The view mode, as persisted: rich/plain, text/tree/table or table/text.
    mode: String,
    /// Mirror of the editor buffer, for preview and autosave.
    content: String,
    kind: ViewKind,
    view: TabView,
    /// The WYSIWYG editor shown in Rich mode, created on first use. Both
    /// editors report their edits through `tab_edited`, so `content` is the
    /// one source of truth whichever one is visible.
    rich: Option<Entity<RichEditor>>,
}

/// The widget a tab shows. Each one reports its full text back through `on_change`.
#[derive(Clone)]
enum TabView {
    Text(Entity<Editor>),
    Csv(Entity<CsvView>),
    Json(Entity<JsonView>),
}

impl Tab {
    /// Whether the visible view is the rich (WYSIWYG) editor.
    fn is_rich(&self) -> bool {
        matches!(self.kind, ViewKind::Markdown { .. }) && self.mode == "rich"
    }

    /// The editor Find and Replace act on, when the visible view shows text.
    fn searchable(&self, cx: &App) -> Option<Searchable> {
        if self.is_rich() {
            return self.rich.clone().map(Searchable::Rich);
        }
        if !view_kind::shows_text(self.kind, &self.mode) {
            return None;
        }
        match &self.view {
            TabView::Text(editor) => Some(Searchable::Plain(editor.clone())),
            TabView::Csv(view) => Some(Searchable::Plain(view.read(cx).text_editor())),
            TabView::Json(view) => Some(Searchable::Plain(view.read(cx).text_editor())),
        }
    }

    /// Where keyboard focus goes for the visible view.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if let (true, Some(rich)) = (self.is_rich(), &self.rich) {
            return rich.read(cx).focus_handle(cx);
        }
        match &self.view {
            TabView::Text(editor) => editor.read(cx).focus_handle(cx),
            TabView::Csv(view) => view.read(cx).focus_handle(cx),
            TabView::Json(view) => view.read(cx).focus_handle(cx),
        }
    }
}

/// What Find and Replace act on: the plain editor, or the rich editor in Rich mode.
#[derive(Clone)]
enum Searchable {
    Plain(Entity<Editor>),
    Rich(Entity<RichEditor>),
}

impl Searchable {
    fn find_ranges(&self, query: &str, cx: &App) -> Vec<Range<usize>> {
        match self {
            Searchable::Plain(editor) => editor.read(cx).find_ranges(query),
            Searchable::Rich(editor) => editor.read(cx).find_ranges(query),
        }
    }

    fn set_search(&self, ranges: Vec<Range<usize>>, active: Option<Range<usize>>, cx: &mut App) {
        match self {
            Searchable::Plain(editor) => {
                editor.update(cx, |editor, cx| editor.set_search(ranges, active, cx))
            }
            Searchable::Rich(editor) => {
                editor.update(cx, |editor, cx| editor.set_search(ranges, active, cx))
            }
        }
    }

    fn select(&self, range: Range<usize>, cx: &mut App) {
        match self {
            Searchable::Plain(editor) => editor.update(cx, |editor, cx| editor.select(range, cx)),
            Searchable::Rich(editor) => editor.update(cx, |editor, cx| editor.select(range, cx)),
        }
    }

    fn replace_at(&self, range: Range<usize>, text: &str, cx: &mut App) {
        match self {
            Searchable::Plain(editor) => {
                editor.update(cx, |editor, cx| editor.replace_at(range, text, cx))
            }
            Searchable::Rich(editor) => {
                editor.update(cx, |editor, cx| editor.replace_at(range, text, cx))
            }
        }
    }

    fn replace_all(&self, query: &str, replacement: &str, cx: &mut App) -> usize {
        match self {
            Searchable::Plain(editor) => {
                editor.update(cx, |editor, cx| editor.replace_all_matches(query, replacement, cx))
            }
            Searchable::Rich(editor) => {
                editor.update(cx, |editor, cx| editor.replace_all_matches(query, replacement, cx))
            }
        }
    }
}

/// Sends a viewer's edits to the tab for `path`. Deferred, because the edit
/// can come from inside a workspace update (switching modes, replacing text),
/// and the workspace cannot be updated while it is already being updated.
fn edit_reporter(workspace: Entity<Zyplus>, path: String) -> Box<dyn Fn(&str, &mut App)> {
    Box::new(move |text, cx| {
        let text = text.to_string();
        let workspace = workspace.clone();
        let path = path.clone();
        cx.defer(move |cx| {
            let _ = workspace.update(cx, |this, cx| this.tab_edited(&path, text, cx));
        });
    })
}

#[derive(Clone)]
enum MenuAction {
    NewFile(String),
    NewFolder(String),
    Rename(String, bool),
    Delete(String, bool),
    Duplicate(String),
    CopyPath(String),
    Reveal(String),
    CloseProject(String),
    Open(String),
    Save(String),
    Close(String),
    CloseOthers(String),
    CloseAll,
    CopyMarkdown(String),
    ExportMd(String),
    ExportPdf(String),
    Find,
    Replace,
    ToggleMode,
    Settings,
    OpenFolder,
}

#[derive(Clone)]
struct MenuItem {
    label: &'static str,
    action: MenuAction,
    danger: bool,
    disabled: bool,
    /// Hugeicon name (kebab-case, see `app_icon`) drawn left of the label.
    icon: Option<&'static str>,
}

impl MenuItem {
    fn with_icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }
}

fn item(label: &'static str, action: MenuAction) -> MenuItem {
    MenuItem {
        label,
        action,
        danger: false,
        disabled: false,
        icon: None,
    }
}

fn danger_item(label: &'static str, action: MenuAction) -> MenuItem {
    MenuItem {
        label,
        action,
        danger: true,
        disabled: false,
        icon: None,
    }
}

#[derive(Clone)]
struct MenuState {
    x: f32,
    y: f32,
    items: Vec<MenuItem>,
}

#[derive(Clone, Copy, PartialEq)]
enum SettingsSection {
    General,
    Appearance,
    Extensions,
    Shortcuts,
    About,
}

#[derive(Clone)]
enum Modal {
    Rename {
        path: String,
        is_folder: bool,
        input: Entity<Editor>,
    },
    NewFolder {
        dir: String,
        input: Entity<Editor>,
    },
    Delete {
        paths: Vec<String>,
        is_folder: bool,
    },
    Unsaved {
        path: String,
    },
    Settings {
        section: SettingsSection,
    },
}

struct FindState {
    query_text: String,
    query: Entity<Editor>,
    replace_text: String,
    replace: Entity<Editor>,
    show_replace: bool,
    matches: Vec<Range<usize>>,
    index: usize,
    /// The buffer changed since the matches were computed.
    dirty: bool,
}

/// A tab being dragged across the tab strip.
#[derive(Clone)]
struct DragTab {
    path: String,
    title: String,
}

/// One or more sidebar rows being dragged onto a folder.
#[derive(Clone)]
struct DragPath {
    paths: Vec<String>,
    label: String,
}

/// What follows the pointer while a tab or row is dragged.
struct DragGhost {
    label: SharedString,
    theme: Theme,
}

impl Render for DragGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .h(px(24.0))
            .flex()
            .items_center()
            .rounded(px(5.0))
            .bg(self.theme.surface)
            .border_1()
            .border_color(self.theme.accent)
            .text_size(px(12.0))
            .text_color(self.theme.fg)
            .child(self.label.clone())
    }
}

struct Zyplus {
    focus: FocusHandle,
    roots: Vec<String>,
    tree: Vec<TreeNode>,
    open_folders: BTreeSet<String>,
    tabs: Vec<Tab>,
    active: Option<String>,
    sidebar_collapsed: bool,
    default_folder: Option<String>,
    error: Option<SharedString>,
    focus_placed: bool,
    theme_name: String,
    autosave: bool,
    autosave_tick: u64,
    find: Option<FindState>,
    /// Set when the find bar closes without a window (from its own Escape
    /// handler); the next render gives keyboard focus back to the editor.
    refocus_editor: bool,
    menu: Option<MenuState>,
    modal: Option<Modal>,
    dialog_error: Option<SharedString>,
    /// The rows selected in the sidebar, for Delete and drag affordances.
    selected: Vec<String>,
    /// The row a range selection extends from.
    selection_anchor: Option<String>,
    /// Which row was clicked and when, to tell a double-click from two clicks.
    last_row_click: Option<(String, Instant)>,
    /// First run before a notes folder has been chosen or skipped.
    welcome: bool,
    /// Built-in extensions turned off in Settings → Extensions.
    disabled_extensions: BTreeSet<String>,
    /// A short status line (export progress and results), cleared by a timer.
    notice: Option<SharedString>,
    notice_tick: u64,
    /// Where the update check and download stand, for the sidebar and About.
    update_status: updater::Status,
    /// Which apps open each file type, read from the OS when Settings opens.
    default_apps: Option<DefaultApps>,
}

/// LaunchServices claims for the Settings → General rows, in `DEFAULT_APPS` order.
#[derive(Clone, Default)]
struct DefaultApps {
    claimed: [bool; 3],
    error: [Option<SharedString>; 3],
}

/// The "Default … app" rows: label, UTI, and the hint on macOS and elsewhere.
const DEFAULT_APPS: [(&str, &str, &str, &str); 3] = [
    (
        "Default Markdown app",
        os::uti::MARKDOWN,
        "Open .md files with Zyplus when you double-click them.",
        "Set Zyplus as the handler for .md in your system's default apps settings.",
    ),
    (
        "Default CSV app",
        os::uti::CSV,
        "Open .csv files with Zyplus when you double-click them.",
        "Set Zyplus as the handler for .csv in your system's default apps settings.",
    ),
    (
        "Default JSON app",
        os::uti::JSON,
        "Open .json files with Zyplus when you double-click them.",
        "Set Zyplus as the handler for .json in your system's default apps settings.",
    ),
];

/// Built-in extensions for Settings → Extensions:
/// (id, toggle element id, name, description, applies to). Descriptions are
/// the web catalog's (`src/extensions/catalog.ts`).
const EXTENSIONS: [(&str, &str, &str, &str, &str); 4] = [
    (
        "json",
        "ext-toggle-json",
        "JSON Editor",
        "Opens .json files in a tree, table or text view. Edit values, format, sort, search and repair, with big numbers kept exact.",
        "Applies to .json files.",
    ),
    (
        "csv",
        "ext-toggle-csv",
        "CSV / TSV Editor",
        "Opens .csv and .tsv files in a table or text view. Edit rows and columns with lossless round-tripping for ragged rows.",
        "Applies to .csv and .tsv files.",
    ),
    (
        "chart",
        "ext-toggle-chart",
        "Charts",
        "Renders bar, line, pie, doughnut, and scatter charts from JSON configuration.",
        "Applies to ```chart blocks in markdown.",
    ),
    (
        "codefiles",
        "ext-toggle-codefiles",
        "Code & Config Files",
        "Syntax highlighting and basic editor for various code, config and script files.",
        "Applies to code and config file types, such as .yaml, .rs and .ts.",
    ),
];

/// Catalog extensions the native build cannot run yet: (toggle id, name, description).
const UNAVAILABLE_EXTENSIONS: [(&str, &str, &str); 2] = [
    (
        "ext-toggle-mermaid",
        "Mermaid Diagrams",
        "Renders rich interactive diagrams, sequence charts, git graphs, and flowcharts directly in markdown preview.",
    ),
    (
        "ext-toggle-katex",
        "LaTeX Math (KaTeX)",
        "Fast math typesetting engine that transforms LaTeX equations into formatted mathematical expressions.",
    ),
];

/// Keyboard shortcuts for the sheet: groups in order, each row with its keys.
/// Kept in step with `src/lib/shortcuts.ts`.
const SHORTCUT_GROUPS: [(&str, &[(&str, &[&str])]); 4] = [
    (
        "File",
        &[
            ("Save", &["⌘", "S"]),
            ("Export as .md", &["⌘", "⇧", "S"]),
            ("Export as PDF", &["⌘", "P"]),
            ("New file", &["⌘", "N"]),
            ("New folder", &["⌘", "⇧", "N"]),
            ("Open folder", &["⌘", "O"]),
            ("Open file", &["⌘", "⇧", "O"]),
        ],
    ),
    (
        "Tabs",
        &[
            ("Close tab", &["⌘", "W"]),
            ("Next tab", &["⌘", "⌥", "→"]),
            ("Previous tab", &["⌘", "⌥", "←"]),
            ("Go to tab 1–9", &["⌘", "1–9"]),
        ],
    ),
    (
        "View",
        &[
            ("Toggle sidebar", &["⌘", "B"]),
            ("Toggle rich / plain text", &["⌘", "E"]),
            ("Settings", &["⌘", ","]),
            ("Keyboard shortcuts", &["⌘", "/"]),
        ],
    ),
    (
        "Editing",
        &[
            ("Find in document", &["⌘", "F"]),
            ("Find and replace", &["⌘", "⌥", "F"]),
            ("Copy document markdown", &["⌘", "⇧", "C"]),
            ("Copy file path", &["⌘", "⌥", "C"]),
        ],
    ),
];

fn is_untitled(title: &str) -> bool {
    if title == "Untitled.md" {
        return true;
    }
    title
        .strip_prefix("Untitled ")
        .and_then(|rest| rest.strip_suffix(".md"))
        .map(|number| number.trim().parse::<u32>().is_ok())
        .unwrap_or(false)
}

impl Zyplus {
    fn new(cx: &mut Context<Self>) -> Self {
        let stored = session::load();
        let had_session = stored.is_some();
        let mut this = Self {
            focus: cx.focus_handle(),
            roots: Vec::new(),
            tree: Vec::new(),
            open_folders: BTreeSet::new(),
            tabs: Vec::new(),
            active: None,
            sidebar_collapsed: false,
            default_folder: None,
            error: None,
            focus_placed: false,
            theme_name: "system".to_string(),
            autosave: false,
            autosave_tick: 0,
            find: None,
            refocus_editor: false,
            menu: None,
            modal: None,
            dialog_error: None,
            selected: Vec::new(),
            selection_anchor: None,
            last_row_click: None,
            welcome: false,
            disabled_extensions: BTreeSet::new(),
            notice: None,
            notice_tick: 0,
            update_status: updater::Status::default(),
            default_apps: None,
        };

        let mut restored = false;
        if let Some(stored) = stored {
            restored = !stored.roots.is_empty() || !stored.tabs.is_empty();
            if theme::THEMES.iter().any(|option| option.id == stored.theme) {
                this.theme_name = stored.theme;
            }
            this.autosave = stored.autosave;
            this.sidebar_collapsed = stored.sidebar_collapsed;
            this.default_folder = stored.default_folder;
            this.disabled_extensions = stored.disabled_extensions.into_iter().collect();

            let open: BTreeSet<String> = stored.open_folders.into_iter().collect();
            for root in &stored.roots {
                match tree::read_project_node(root, &open) {
                    Ok(node) => {
                        this.roots.push(root.clone());
                        this.tree.push(node);
                    }
                    Err(err) => this.error = Some(err.into()),
                }
            }
            this.open_folders = open;

            let mut wanted_active = stored.active_file_path.clone();
            for tab in stored.tabs {
                match tree::read_text_file(&tab.file_path) {
                    Ok(content) => this.push_tab(&tab.file_path, &tab.mode, content, cx),
                    Err(err) => this.error = Some(err.into()),
                }
            }
            if let Some(path) = wanted_active.take() {
                if this.tabs.iter().any(|tab| tab.path == path) {
                    this.active = Some(path);
                }
            }
        }

        // First run shows the welcome screen, where the notes folder can be
        // created or skipped. A stored session that opens nothing means the
        // user already went through it, so it is not shown again.
        if !restored && !had_session {
            this.welcome = true;
        }

        if !restored {
            this.persist();
        }
        this.start_startup_update_check(cx);
        this
    }

    fn finish_welcome(&mut self, cx: &mut Context<Self>) {
        self.welcome = false;
        self.focus_placed = false;
        self.persist();
        cx.notify();
    }

    fn skip_welcome(&mut self, cx: &mut Context<Self>) {
        self.default_folder = Some(String::new());
        self.finish_welcome(cx);
    }

    fn create_default_folder(&mut self, cx: &mut Context<Self>) {
        if self.create_notes_folder(cx) {
            self.finish_welcome(cx);
        }
    }

    fn create_notes_folder(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(folder) = paths::default_folder_path() else {
            self.error = Some("Could not find the application data folder.".into());
            cx.notify();
            return false;
        };
        let dir = folder.to_string_lossy().into_owned();
        if let Err(err) = tree::ensure_folder(&dir) {
            self.error = Some(err.into());
            cx.notify();
            return false;
        }
        let note = paths::display_join(&dir, WELCOME_NOTE_NAME);
        if !paths::path_exists(&note) {
            let _ = tree::write_text_file(&note, WELCOME_NOTE);
        }
        match tree::read_project_node(&dir, &BTreeSet::new()) {
            Ok(node) => {
                self.roots.push(dir.clone());
                self.tree.push(node);
                self.open_folders.insert(dir.clone());
                self.default_folder = Some(dir);
                self.open_document(&note, None, cx);
                self.persist();
                true
            }
            Err(err) => {
                self.error = Some(err.into());
                cx.notify();
                false
            }
        }
    }

    fn active_index(&self) -> Option<usize> {
        let active = self.active.as_deref()?;
        self.tabs.iter().position(|tab| tab.path == active)
    }

    fn active_tab(&self) -> Option<&Tab> {
        self.active_index().map(|index| &self.tabs[index])
    }

    /// What Find and Replace act on for the active tab. `None` when the visible
    /// view shows no text (JSON tree, CSV table).
    fn active_searchable(&self, cx: &App) -> Option<Searchable> {
        self.active_tab()?.searchable(cx)
    }

    /// Creates the rich editor of tab `index` the first time it is needed, and
    /// reports its edits to that tab.
    fn ensure_rich(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.tabs[index].rich.is_some() {
            return;
        }
        let content = self.tabs[index].content.clone();
        let path = self.tabs[index].path.clone();
        let theme_name = self.theme_name.clone();
        let charts = self.extension_enabled("chart");
        let workspace = cx.entity();
        let rich = cx.new(|cx| {
            let mut editor = RichEditor::new(cx);
            editor.theme_name = theme_name;
            editor.set_file_path(Some(std::path::PathBuf::from(&path)), cx);
            editor.set_charts_enabled(charts, cx);
            editor.set_text(&content, cx);
            editor
        });
        rich.update(cx, |editor, _| {
            editor.on_change = Some(edit_reporter(workspace, path));
        });
        self.tabs[index].rich = Some(rich);
    }

    fn theme(&self, window: &Window) -> Theme {
        Theme::for_name(&self.theme_name, window.appearance())
    }

    fn focus_editor(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab) = self.active_tab() {
            let handle = tab.focus_handle(cx);
            window.focus(&handle);
        } else {
            window.focus(&self.focus);
        }
    }

    /// Whether a built-in extension (json, csv, chart, codefiles) is on.
    fn extension_enabled(&self, id: &str) -> bool {
        !self.disabled_extensions.contains(id)
    }

    fn persist(&mut self) {
        let stored = session::Session {
            roots: self.roots.clone(),
            default_folder: self.default_folder.clone(),
            tabs: self
                .tabs
                .iter()
                .map(|tab| session::PersistedTab {
                    file_path: tab.path.clone(),
                    mode: tab.mode.clone(),
                })
                .collect(),
            active_file_path: self.active.clone(),
            sidebar_collapsed: self.sidebar_collapsed,
            autosave: self.autosave,
            open_folders: self.open_folders.iter().cloned().collect(),
            theme: self.theme_name.clone(),
            disabled_extensions: self.disabled_extensions.iter().cloned().collect(),
        };
        let _ = session::save(&stored);
    }

    /// Opens `path` as a tab in the view its extension picks. `mode` is the
    /// persisted view mode; a mode the view does not offer falls back to the
    /// view's default.
    fn push_tab(&mut self, path: &str, mode: &str, content: String, cx: &mut Context<Self>) {
        if self.tabs.iter().any(|tab| tab.path == path) {
            self.active = Some(path.to_string());
            return;
        }
        let kind = view_kind::kind_for_path(path, |id| self.extension_enabled(id));
        let wanted = view_kind::normalize_mode(kind, mode);
        let theme_name = self.theme_name.clone();
        let workspace = cx.entity();
        let tab_path = path.to_string();

        let (view, mode) = match kind {
            ViewKind::Csv => {
                let csv = cx.new(|cx| CsvView::new(path, &content, &theme_name, cx));
                csv.update(cx, |view, cx| {
                    view.on_change = Some(edit_reporter(workspace.clone(), tab_path.clone()));
                    if let Some(wanted) = wanted {
                        view.select_mode(wanted, None, cx);
                    }
                });
                let mode = csv.read(cx).mode();
                (TabView::Csv(csv), mode)
            }
            ViewKind::Json => {
                let json = cx.new(|cx| JsonView::new(&content, &theme_name, cx));
                json.update(cx, |view, cx| {
                    view.on_change = Some(edit_reporter(workspace.clone(), tab_path.clone()));
                    if let Some(wanted) = wanted {
                        view.set_mode(wanted, cx);
                    }
                });
                let mode = json.read(cx).mode();
                (TabView::Json(json), mode)
            }
            ViewKind::Markdown { language } | ViewKind::Text { language } => {
                let editor = cx.new(|cx| {
                    let mut editor = Editor::new(false, cx);
                    editor.set_text(&content, cx);
                    editor.theme_name = theme_name.clone();
                    editor
                });
                editor.update(cx, |editor, cx| {
                    editor.on_change = Some(edit_reporter(workspace.clone(), tab_path.clone()));
                    editor.set_language(language);
                    cx.notify();
                });
                (TabView::Text(editor), wanted.unwrap_or("plain"))
            }
        };

        self.tabs.push(Tab {
            path: path.to_string(),
            title: paths::basename_of(path),
            saved: content.clone(),
            dirty: false,
            mode: mode.to_string(),
            content,
            kind,
            view,
            rich: None,
        });
        if let Some(index) = self.tabs.iter().position(|tab| tab.path == path) {
            if self.tabs[index].is_rich() {
                self.ensure_rich(index, cx);
            }
        }
        self.active = Some(path.to_string());
        self.error = None;
    }

    fn tab_edited(&mut self, path: &str, text: String, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.path == path) {
            tab.dirty = text != tab.saved;
            tab.content = text;
        }
        if let Some(find) = self.find.as_mut() {
            find.dirty = true;
        }
        self.schedule_autosave(cx);
        cx.notify();
    }

    fn open_document(&mut self, path: &str, window: Option<&mut Window>, cx: &mut Context<Self>) {
        self.welcome = false;
        if self.tabs.iter().any(|tab| tab.path == path) {
            self.active = Some(path.to_string());
            if let Some(window) = window {
                self.focus_editor(window, cx);
            }
            self.persist();
            cx.notify();
            return;
        }

        let content = match tree::read_text_file(path) {
            Ok(content) => content,
            Err(err) => {
                self.error = Some(err.into());
                cx.notify();
                return;
            }
        };

        self.push_tab(path, "plain", content, cx);

        if let Some(folder) = self
            .roots
            .iter()
            .find(|root| paths::is_under(path, root))
            .cloned()
        {
            self.open_folders.insert(folder);
        }

        if let Some(window) = window {
            self.focus_editor(window, cx);
        }
        self.persist();
        cx.notify();
    }

    fn save_path(&mut self, path: &str, cx: &mut Context<Self>) -> bool {
        let Some(index) = self.tabs.iter().position(|tab| tab.path == path) else {
            return false;
        };
        let content = self.tabs[index].content.clone();
        match tree::write_text_file(path, &content) {
            Ok(()) => {
                self.tabs[index].saved = content;
                self.tabs[index].dirty = false;
                self.error = None;
                self.persist();
                cx.notify();
                true
            }
            Err(err) => {
                self.error = Some(err.into());
                cx.notify();
                false
            }
        }
    }

    fn save_active(&mut self, cx: &mut Context<Self>) -> bool {
        match self.active.clone() {
            Some(path) => self.save_path(&path, cx),
            None => false,
        }
    }

    fn close_tab_now(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.tabs.iter().position(|tab| tab.path == path) else {
            return;
        };
        self.tabs.remove(index);
        if self.active.as_deref() == Some(path) {
            self.active = self
                .tabs
                .get(index.saturating_sub(1))
                .or_else(|| self.tabs.last())
                .map(|tab| tab.path.clone());
        }
        self.focus_editor(window, cx);
        self.persist();
        cx.notify();
    }

    fn request_close(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter().find(|tab| tab.path == path) else {
            return;
        };
        let dirty = tab.dirty;
        let title = tab.title.clone();
        if !dirty {
            self.close_tab_now(path, window, cx);
            return;
        }
        if self.autosave || is_untitled(&title) {
            if self.save_path(path, cx) {
                self.close_tab_now(path, window, cx);
            }
            return;
        }
        self.dialog_error = None;
        self.modal = Some(Modal::Unsaved {
            path: path.to_string(),
        });
        cx.notify();
    }

    fn close_others(&mut self, keep: &str, window: &mut Window, cx: &mut Context<Self>) {
        let remove: Vec<String> = self
            .tabs
            .iter()
            .filter(|tab| tab.path != keep && !tab.dirty)
            .map(|tab| tab.path.clone())
            .collect();
        for path in remove {
            self.close_tab_now(&path, window, cx);
        }
        self.active = Some(keep.to_string());
        self.focus_editor(window, cx);
        self.persist();
        cx.notify();
    }

    fn close_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let remove: Vec<String> = self
            .tabs
            .iter()
            .filter(|tab| !tab.dirty)
            .map(|tab| tab.path.clone())
            .collect();
        for path in remove {
            self.close_tab_now(&path, window, cx);
        }
        self.focus_editor(window, cx);
        cx.notify();
    }

    /// Rewrites the tabs, the active path and the open folders after `old`
    /// changed its name or moved somewhere else.
    fn remap_paths(&mut self, old: &str, new: &str) {
        for tab in &mut self.tabs {
            if paths::is_under(&tab.path, old) {
                tab.path = format!("{}{}", new, &tab.path[old.len()..]);
                tab.title = paths::basename_of(&tab.path);
            }
        }
        if let Some(active) = self.active.clone() {
            if paths::is_under(&active, old) {
                self.active = Some(format!("{}{}", new, &active[old.len()..]));
            }
        }
        self.selected = self
            .selected
            .iter()
            .map(|selected| {
                if paths::is_under(selected, old) {
                    format!("{}{}", new, &selected[old.len()..])
                } else {
                    selected.clone()
                }
            })
            .collect();
        if let Some(selected) = self.selection_anchor.clone() {
            if paths::is_under(&selected, old) {
                self.selection_anchor = Some(format!("{}{}", new, &selected[old.len()..]));
            }
        }
        self.open_folders = self
            .open_folders
            .iter()
            .map(|id| {
                if paths::is_under(id, old) {
                    format!("{}{}", new, &id[old.len()..])
                } else {
                    id.clone()
                }
            })
            .collect();
    }

    /// Moves dragged rows into `dest_dir`, the way dragging in the web tree did.
    fn move_into(&mut self, paths_to_move: &[String], dest_dir: &str, cx: &mut Context<Self>) {
        let mut moved = false;
        for src in paths_to_move {
            if self.roots.iter().any(|root| root == src) || src == dest_dir {
                continue;
            }
            // A folder cannot be dropped inside itself.
            if paths::is_under(dest_dir, src) {
                continue;
            }
            let new_path = paths::display_join(dest_dir, &paths::basename_of(src));
            if new_path == *src {
                continue;
            }
            match tree::rename_path(src, &new_path) {
                Ok(()) => {
                    self.remap_paths(src, &new_path);
                    moved = true;
                }
                Err(err) => {
                    self.error = Some(err.into());
                    cx.notify();
                    return;
                }
            }
        }
        if moved {
            self.error = None;
            self.refresh_tree(cx);
            self.persist();
            cx.notify();
        }
    }

    /// Applies a click on a sidebar row: plain replaces the selection, the
    /// command/alt key toggles one row, shift extends from the last anchor.
    fn select_row(&mut self, id: &str, modifiers: &gpui::Modifiers, cx: &mut Context<Self>) {
        if modifiers.shift {
            let rows: Vec<String> = tree::visible_rows(&self.tree, &self.open_folders)
                .into_iter()
                .map(|row| row.id)
                .collect();
            let anchor = self.selection_anchor.clone().unwrap_or_else(|| id.to_string());
            let from = rows.iter().position(|row| row == &anchor);
            let to = rows.iter().position(|row| row == id);
            match (from, to) {
                (Some(from), Some(to)) => {
                    let (start, end) = if from <= to { (from, to) } else { (to, from) };
                    self.selected = rows[start..=end].to_vec();
                }
                _ => self.selected = vec![id.to_string()],
            }
        } else if modifiers.secondary() || modifiers.alt {
            if let Some(index) = self.selected.iter().position(|path| path == id) {
                self.selected.remove(index);
            } else {
                self.selected.push(id.to_string());
                self.selection_anchor = Some(id.to_string());
            }
        } else {
            self.selected = vec![id.to_string()];
            self.selection_anchor = Some(id.to_string());
        }
        cx.notify();
    }

    /// Drops a dragged tab in front of `target`, or at the end for `None`.
    fn reorder_tab(&mut self, id: &str, target: Option<&str>, cx: &mut Context<Self>) {
        if target == Some(id) {
            return;
        }
        let Some(from) = self.tabs.iter().position(|tab| tab.path == id) else {
            return;
        };
        let tab = self.tabs.remove(from);
        let to = match target {
            Some(target) => self
                .tabs
                .iter()
                .position(|tab| tab.path == target)
                .unwrap_or(self.tabs.len()),
            None => self.tabs.len(),
        };
        self.tabs.insert(to, tab);
        self.persist();
        cx.notify();
    }

    /// True when this is the second click on the same row within a moment.
    fn register_row_click(&mut self, id: &str) -> bool {
        let now = Instant::now();
        let double = matches!(
            &self.last_row_click,
            Some((last, at)) if last == id && now.duration_since(*at) < Duration::from_millis(400)
        );
        self.last_row_click = if double {
            None
        } else {
            Some((id.to_string(), now))
        };
        double
    }

    fn delete_selected(&mut self, cx: &mut Context<Self>) {
        // Project roots are closed, never deleted from disk.
        let paths: Vec<String> = self
            .selected
            .iter()
            .filter(|path| !self.roots.iter().any(|root| root == *path))
            .filter(|path| tree::find_tree_node(&self.tree, path).is_some())
            .cloned()
            .collect();
        if paths.is_empty() {
            return;
        }
        let is_folder = paths.len() == 1
            && tree::find_tree_node(&self.tree, &paths[0])
                .map(|node| node.is_folder)
                .unwrap_or(false);
        self.dialog_error = None;
        self.modal = Some(Modal::Delete { paths, is_folder });
        cx.notify();
    }

    fn creation_dir(&self) -> Option<String> {
        if let Some(tab) = self.active_tab() {
            let dir = paths::dirname_of(&tab.path);
            if Path::new(&dir).is_dir() {
                return Some(dir);
            }
        }
        self.default_folder
            .clone()
            .filter(|dir| !dir.is_empty())
            .or_else(|| self.roots.first().cloned())
    }

    fn new_file_in(&mut self, dir: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !Path::new(dir).is_dir() {
            self.error = Some("That folder is no longer there.".into());
            cx.notify();
            return;
        }
        let path = paths::free_path(dir, "Untitled.md", "");
        if let Err(err) = tree::create_file(&path) {
            self.error = Some(err.into());
            cx.notify();
            return;
        }
        self.refresh_tree(cx);
        self.reveal_dir(dir);
        self.open_document(&path, Some(window), cx);
    }

    fn new_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dir) = self.creation_dir() else {
            self.error = Some("Open a folder first.".into());
            cx.notify();
            return;
        };
        self.new_file_in(&dir, window, cx);
    }

    fn new_folder_in(&mut self, dir: &str, cx: &mut Context<Self>) {
        if !Path::new(dir).is_dir() {
            self.error = Some("That folder is no longer there.".into());
            cx.notify();
            return;
        }
        let theme_name = self.theme_name.clone();
        let input = cx.new(|cx| {
            let mut editor = Editor::new(true, cx)
                .with_placeholder("folder-name")
                .with_padding(8.0, 5.0);
            editor.theme_name = theme_name;
            editor
        });
        let this = cx.entity();
        let for_submit = this.clone();
        let for_cancel = this.clone();
        input.update(cx, |editor, _| {
            editor.on_submit = Some(Box::new(move |cx| {
                let this = for_submit.clone();
                cx.defer(move |cx| {
                    let _ = this.update(cx, |this, cx| this.submit_new_folder(cx));
                });
            }));
            editor.on_cancel = Some(Box::new(move |cx| {
                let this = for_cancel.clone();
                cx.defer(move |cx| {
                    let _ = this.update(cx, |this, cx| {
                        this.modal = None;
                        this.dialog_error = None;
                        cx.notify();
                    });
                });
            }));
        });
        self.dialog_error = None;
        self.modal = Some(Modal::NewFolder {
            dir: dir.to_string(),
            input,
        });
        cx.notify();
    }

    fn submit_new_folder(&mut self, cx: &mut Context<Self>) {
        let Some(Modal::NewFolder { dir, input }) = self.modal.clone() else {
            return;
        };
        let name = input.read(cx).text().trim().to_string();
        if let Some(reason) = paths::invalid_name_reason(&name) {
            self.dialog_error = Some(reason.into());
            cx.notify();
            return;
        }
        let path = paths::display_join(&dir, &name);
        match tree::create_folder(&path) {
            Ok(()) => {
                self.modal = None;
                self.dialog_error = None;
                self.error = None;
                self.refresh_tree(cx);
                self.reveal_dir(&dir);
                self.persist();
                cx.notify();
            }
            Err(err) => {
                self.dialog_error = Some(err.into());
                cx.notify();
            }
        }
    }

    fn new_folder(&mut self, cx: &mut Context<Self>) {
        let Some(dir) = self.creation_dir() else {
            self.error = Some("Open a folder first.".into());
            cx.notify();
            return;
        };
        self.new_folder_in(&dir, cx);
    }

    fn refresh_tree(&mut self, cx: &mut Context<Self>) {
        if self.roots.is_empty() {
            return;
        }
        let mut loaded = BTreeSet::new();
        tree::loaded_folder_ids(&self.tree, &mut loaded);
        let mut next = Vec::new();
        for root in self.roots.clone() {
            match tree::read_project_node(&root, &loaded) {
                Ok(node) => next.push(node),
                Err(err) => {
                    self.error = Some(err.into());
                    if let Some(old) = self.tree.iter().find(|node| node.id == root) {
                        next.push(old.clone());
                    }
                }
            }
        }
        self.tree = next;
        cx.notify();
    }

    fn toggle_folder(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.open_folders.remove(id) {
            self.persist();
            cx.notify();
            return;
        }
        if tree::find_tree_node(&self.tree, id)
            .map(|node| node.is_unloaded())
            .unwrap_or(false)
        {
            match tree::read_children(id) {
                Ok(children) => {
                    tree::set_children(&mut self.tree, id, children);
                }
                Err(err) => {
                    self.error = Some(err.into());
                    cx.notify();
                    return;
                }
            }
        }
        self.open_folders.insert(id.to_string());
        self.persist();
        cx.notify();
    }

    /// Opens every folder between a project root and `dir`, reading the ones
    /// that have not been read yet, so a new or moved file is visible.
    fn reveal_dir(&mut self, dir: &str) {
        let Some(root) = self
            .roots
            .iter()
            .find(|root| paths::is_under(dir, root))
            .cloned()
        else {
            return;
        };
        let mut chain = vec![dir.to_string()];
        let mut current = dir.to_string();
        while current != root {
            let parent = paths::dirname_of(&current);
            if parent == current || !paths::is_under(&parent, &root) {
                break;
            }
            chain.push(parent.clone());
            current = parent;
        }
        for id in chain.into_iter().rev() {
            if tree::find_tree_node(&self.tree, &id)
                .map(|node| node.is_unloaded())
                .unwrap_or(false)
            {
                if let Ok(children) = tree::read_children(&id) {
                    tree::set_children(&mut self.tree, &id, children);
                }
            }
            self.open_folders.insert(id);
        }
    }

    fn add_folder(&mut self, dir: String, cx: &mut Context<Self>) {
        self.welcome = false;
        if self.roots.iter().any(|root| root == &dir) {
            self.open_folders.insert(dir);
            self.persist();
            cx.notify();
            return;
        }
        match tree::read_project_node(&dir, &BTreeSet::new()) {
            Ok(node) => {
                self.roots.push(dir.clone());
                self.tree.push(node);
                self.open_folders.insert(dir.clone());
                if self.default_folder.is_none() {
                    self.default_folder = Some(dir);
                }
                self.error = None;
                self.persist();
                cx.notify();
            }
            Err(err) => {
                self.error = Some(err.into());
                cx.notify();
            }
        }
    }

    fn close_project(&mut self, dir: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .tabs
            .iter()
            .any(|tab| tab.dirty && paths::is_under(&tab.path, dir))
        {
            self.error = Some("Save or close the edited files in this folder first.".into());
            cx.notify();
            return;
        }
        self.roots.retain(|root| root != dir);
        self.tree.retain(|node| node.id != dir);
        self.open_folders.retain(|id| !paths::is_under(id, dir));
        let remove: Vec<String> = self
            .tabs
            .iter()
            .filter(|tab| paths::is_under(&tab.path, dir))
            .map(|tab| tab.path.clone())
            .collect();
        for path in remove {
            self.close_tab_now(&path, window, cx);
        }
        self.persist();
        cx.notify();
    }

    fn pick_folder(&mut self, cx: &mut Context<Self>, as_default: bool) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open a folder".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await {
                if let Some(path) = paths.into_iter().next() {
                    let dir = path.to_string_lossy().into_owned();
                    this.update(cx, |this, cx| {
                        if as_default {
                            this.default_folder = Some(dir.clone());
                        }
                        this.add_folder(dir, cx);
                    })
                    .ok();
                }
            }
        })
        .detach();
    }

    fn pick_file(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open a file".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await {
                if let Some(path) = paths.into_iter().next() {
                    let path = path.to_string_lossy().into_owned();
                    this.update(cx, |this, cx| this.open_document(&path, None, cx))
                        .ok();
                }
            }
        })
        .detach();
    }

    // ---- find and replace ----------------------------------------------

    fn open_find(&mut self, replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        let theme_name = self.theme_name.clone();
        if self.find.is_none() {
            let this = cx.entity();
            let query = cx.new(|cx| {
                Editor::new(true, cx)
                    .with_placeholder("Find")
                    .with_padding(8.0, 5.0)
            });
            let replace_input = cx.new(|cx| {
                Editor::new(true, cx)
                    .with_placeholder("Replace with")
                    .with_padding(8.0, 5.0)
            });

            let for_change = this.clone();
            let for_submit = this.clone();
            let for_cancel = this.clone();
            query.update(cx, |editor, _| {
                editor.theme_name = theme_name.clone();
                editor.on_change = Some(Box::new(move |text, cx| {
                    let text = text.to_string();
                    for_change.update(cx, |this, cx| {
                        if let Some(find) = this.find.as_mut() {
                            find.query_text = text;
                            find.dirty = true;
                            find.index = 0;
                        }
                        this.refresh_find(cx);
                        cx.notify();
                    });
                }));
                editor.on_submit = Some(Box::new(move |cx| {
                    let _ = for_submit.update(cx, |this, cx| this.step_find(1, cx));
                }));
                editor.on_cancel = Some(Box::new(move |cx| {
                    // Tearing the find bar down drops this very input, so wait
                    // until the editor has finished dispatching the key.
                    let this = for_cancel.clone();
                    cx.defer(move |cx| {
                        let _ = this.update(cx, |this, cx| this.close_find(None, cx));
                    });
                }));
            });

            let for_change = this.clone();
            let for_submit = this.clone();
            let for_cancel = this.clone();
            replace_input.update(cx, |editor, _| {
                editor.theme_name = theme_name.clone();
                editor.on_change = Some(Box::new(move |text, cx| {
                    let text = text.to_string();
                    for_change.update(cx, |this, cx| {
                        if let Some(find) = this.find.as_mut() {
                            find.replace_text = text;
                        }
                        cx.notify();
                    });
                }));
                editor.on_submit = Some(Box::new(move |cx| {
                    let _ = for_submit.update(cx, |this, cx| this.replace_current(cx));
                }));
                editor.on_cancel = Some(Box::new(move |cx| {
                    let this = for_cancel.clone();
                    cx.defer(move |cx| {
                        let _ = this.update(cx, |this, cx| this.close_find(None, cx));
                    });
                }));
            });

            self.find = Some(FindState {
                query_text: String::new(),
                query,
                replace_text: String::new(),
                replace: replace_input,
                show_replace: replace,
                matches: Vec::new(),
                index: 0,
                dirty: false,
            });
        } else if let Some(find) = self.find.as_mut() {
            find.show_replace = replace;
        }
        self.find.as_mut().unwrap().dirty = true;
        self.refresh_find(cx);
        let handle = self.find.as_ref().unwrap().query.read(cx).focus_handle(cx);
        window.focus(&handle);
        cx.notify();
    }

    fn close_find(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        self.find = None;
        if let Some(target) = self.active_searchable(cx) {
            target.set_search(Vec::new(), None, cx);
        }
        match window {
            Some(window) => self.focus_editor(window, cx),
            None => self.refocus_editor = true,
        }
        cx.notify();
    }

    fn refresh_find(&mut self, cx: &mut Context<Self>) {
        let Some(find) = self.find.as_ref() else {
            return;
        };
        if !find.dirty {
            return;
        }
        let query = find.query_text.clone();
        let matches = match self.active_searchable(cx) {
            Some(target) if !query.is_empty() => target.find_ranges(&query, cx),
            _ => Vec::new(),
        };
        let find = self.find.as_mut().unwrap();
        find.dirty = false;
        find.matches = matches;
        if find.index >= find.matches.len() {
            find.index = 0;
        }
        let active_range = find.matches.get(find.index).cloned();
        let matches = find.matches.clone();
        if let Some(target) = self.active_searchable(cx) {
            target.set_search(matches, active_range, cx);
        }
    }

    fn step_find(&mut self, delta: isize, cx: &mut Context<Self>) {
        self.refresh_find(cx);
        let Some(find) = self.find.as_mut() else {
            return;
        };
        if find.matches.is_empty() {
            return;
        }
        let len = find.matches.len() as isize;
        find.index = ((find.index as isize + delta).rem_euclid(len)) as usize;
        let index = find.index;
        let find = self.find.as_ref().unwrap();
        let matches = find.matches.clone();
        let Some(range) = find.matches.get(index).cloned() else {
            return;
        };
        let active = Some(range.clone());
        if let Some(target) = self.active_searchable(cx) {
            target.select(range, cx);
            target.set_search(matches, active, cx);
        }
        cx.notify();
    }

    fn replace_current(&mut self, cx: &mut Context<Self>) {
        self.refresh_find(cx);
        let Some(find) = self.find.as_ref() else {
            return;
        };
        let Some(range) = find.matches.get(find.index).cloned() else {
            return;
        };
        let replacement = find.replace_text.clone();
        if let Some(target) = self.active_searchable(cx) {
            target.replace_at(range, &replacement, cx);
        }
        if let Some(find) = self.find.as_mut() {
            find.dirty = true;
        }
        self.refresh_find(cx);
        cx.notify();
    }

    fn replace_all(&mut self, cx: &mut Context<Self>) {
        let Some(find) = self.find.as_ref() else {
            return;
        };
        let query = find.query_text.clone();
        let replacement = find.replace_text.clone();
        if let Some(target) = self.active_searchable(cx) {
            target.replace_all(&query, &replacement, cx);
        }
        if let Some(find) = self.find.as_mut() {
            find.dirty = true;
        }
        self.refresh_find(cx);
        cx.notify();
    }

    // ---- tabs, mode, clipboard, export ----------------------------------

    /// Cmd-E: moves the active tab to the next mode its view offers.
    fn toggle_mode(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        let Some(next) = view_kind::next_mode(tab.kind, &tab.mode) else {
            return;
        };
        self.set_mode(next, window, cx);
    }

    /// Switches the active tab to `mode`, one of the modes its view offers.
    fn set_mode(&mut self, mode: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.active_index() else {
            return;
        };
        let kind = self.tabs[index].kind;
        if self.tabs[index].mode == mode || !view_kind::modes(kind).iter().any(|m| *m == mode) {
            return;
        }
        // Rich mode is a WYSIWYG editor over the same text. Both editors report
        // their edits to the tab, so the tab's content is the source of truth.
        let markdown = matches!(kind, ViewKind::Markdown { .. });
        let before = self.tabs[index].searchable(cx);
        if markdown && mode == "rich" {
            self.ensure_rich(index, cx);
        }
        // Viewers may refuse the switch (a CSV that does not parse, a JSON
        // that is not valid), so the tab keeps whatever the view settled on.
        let applied = match self.tabs[index].view.clone() {
            TabView::Text(_) => mode.to_string(),
            TabView::Csv(view) => {
                view.update(cx, |view, cx| view.select_mode(mode, Some(&mut *window), cx));
                view.read(cx).mode().to_string()
            }
            TabView::Json(view) => {
                view.update(cx, |view, cx| view.set_mode(mode, cx));
                view.read(cx).mode().to_string()
            }
        };
        if markdown {
            let content = self.tabs[index].content.clone();
            if applied == "rich" {
                if let Some(rich) = self.tabs[index].rich.clone() {
                    rich.update(cx, |editor, cx| {
                        if editor.text() != content.as_str() {
                            editor.set_text(&content, cx);
                        }
                    });
                }
            } else if let TabView::Text(editor) = self.tabs[index].view.clone() {
                editor.update(cx, |editor, cx| {
                    if editor.text() != content.as_str() {
                        editor.set_text(&content, cx);
                    }
                });
            }
        }
        let shows_text = view_kind::shows_text(kind, &applied) || applied == "rich";
        self.tabs[index].mode = applied;
        if shows_text {
            self.focus_editor(window, cx);
        }
        if self.find.is_some() {
            if let Some(old) = before {
                old.set_search(Vec::new(), None, cx);
            }
            if let Some(find) = self.find.as_mut() {
                find.dirty = true;
            }
            self.refresh_find(cx);
        }
        self.persist();
        cx.notify();
    }

    fn copy_markdown(&mut self, path: &str, cx: &mut Context<Self>) {
        let content = self
            .tabs
            .iter()
            .find(|tab| tab.path == path)
            .map(|tab| tab.content.clone())
            .or_else(|| tree::read_text_file(path).ok());
        match content {
            Some(content) => {
                cx.write_to_clipboard(ClipboardItem::new_string(content));
                self.error = None;
            }
            None => self.error = Some("Could not read that file.".into()),
        }
        cx.notify();
    }

    fn export_md(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter().find(|tab| tab.path == path) else {
            return;
        };
        let dir = paths::dirname_of(path);
        let suggested = paths::with_extension(&tab.title, ".md");
        let content = tab.content.clone();
        let receiver = cx.prompt_for_new_path(Path::new(&dir), Some(&suggested));
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(target))) = receiver.await {
                let target = target.to_string_lossy().into_owned();
                match tree::write_text_file(&target, &content) {
                    Ok(()) => {
                        this.update(cx, |this, cx| {
                            this.error = None;
                            cx.notify();
                        })
                        .ok();
                    }
                    Err(err) => {
                        this.update(cx, |this, cx| {
                            this.error = Some(err.into());
                            cx.notify();
                        })
                        .ok();
                    }
                }
            }
        })
        .detach();
    }

    fn export_pdf(&mut self, path: &str, cx: &mut Context<Self>) {
        if !export_pdf::supports(path) {
            self.error = Some("PDF export works for markdown and text files.".into());
            cx.notify();
            return;
        }
        let Some(tab) = self.tabs.iter().find(|tab| tab.path == path) else {
            return;
        };
        let dir = paths::dirname_of(path);
        let suggested = export_pdf::suggested_file_name(&tab.title);
        let content = tab.content.clone();
        let source = std::path::PathBuf::from(path);
        let receiver = cx.prompt_for_new_path(Path::new(&dir), Some(&suggested));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(target))) = receiver.await else {
                return;
            };
            let name = paths::basename_of(&target.to_string_lossy());
            this.update(cx, |this, cx| {
                this.error = None;
                this.set_notice(format!("Exporting {name}…"), false, cx);
            })
            .ok();
            // Layout and font shaping take a while on long documents: keep them
            // off the UI thread, and report the outcome when they finish.
            let outcome = cx
                .background_executor()
                .spawn(async move { export_pdf::export(&content, &source, &target).map(|()| name) })
                .await;
            this.update(cx, |this, cx| match outcome {
                Ok(name) => this.set_notice(format!("Exported {name}"), true, cx),
                Err(err) => {
                    this.notice = None;
                    this.error = Some(err.into());
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Shows `text` in the notice bar. Timed notices clear themselves; untimed
    /// ones (work in progress) stay until the next notice replaces them.
    fn set_notice(&mut self, text: String, timed: bool, cx: &mut Context<Self>) {
        self.notice_tick += 1;
        let tick = self.notice_tick;
        self.notice = Some(text.into());
        cx.notify();
        if timed {
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_secs(4))
                    .await;
                this.update(cx, |this, cx| {
                    if this.notice_tick == tick {
                        this.notice = None;
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        }
    }

    fn toggle_extension(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.disabled_extensions.remove(id) {
            self.disabled_extensions.insert(id.to_string());
        }
        self.persist();
        cx.notify();
    }

    /// The OS default-app claims, read once per visit to Settings.
    fn default_app_claims(&mut self) -> DefaultApps {
        self.default_apps
            .get_or_insert_with(|| DefaultApps {
                claimed: DEFAULT_APPS.map(|(_, uti, _, _)| os::is_default_app_for(uti)),
                error: [None, None, None],
            })
            .clone()
    }

    fn make_default_app(&mut self, index: usize, cx: &mut Context<Self>) {
        let result = os::set_default_app_for(DEFAULT_APPS[index].1);
        let mut claims = self.default_app_claims();
        match result {
            Ok(()) => {
                claims.claimed[index] = true;
                claims.error[index] = None;
            }
            Err(err) => claims.error[index] = Some(err.into()),
        }
        self.default_apps = Some(claims);
        cx.notify();
    }

    /// Opens Settings on `section`, re-reading the OS default-app claims.
    fn open_settings(&mut self, section: SettingsSection, cx: &mut Context<Self>) {
        self.dialog_error = None;
        self.default_apps = None;
        self.modal = Some(Modal::Settings { section });
        cx.notify();
    }

    // ---- updates ----------------------------------------------------------

    /// Once per launch, a few seconds after startup. Failures stay silent.
    fn start_startup_update_check(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(3))
                .await;
            this.update(cx, |this, cx| this.check_for_updates(true, cx))
                .ok();
        })
        .detach();
    }

    /// Asks GitHub for a newer release. A silent check reports nothing when it
    /// fails or finds nothing; a manual one shows the result in About.
    fn check_for_updates(&mut self, silent: bool, cx: &mut Context<Self>) {
        if self.update_status.busy() {
            return;
        }
        if !silent {
            self.update_status = updater::Status::Checking;
            cx.notify();
        }
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async { updater::check(VERSION) })
                .await;
            this.update(cx, |this, cx| {
                this.update_status.after_check(result, silent);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Downloads the offered release in the background. The file is only
    /// revealed afterwards; nothing is installed automatically.
    fn download_update(&mut self, cx: &mut Context<Self>) {
        let Some(info) = self.update_status.begin_download() else {
            return;
        };
        let events = updater::start_download(info);
        cx.notify();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(120))
                    .await;
                let mut finished = false;
                let alive = this.update(cx, |this, cx| {
                    let mut changed = false;
                    loop {
                        match events.try_recv() {
                            Ok(updater::DownloadEvent::Progress(done, total)) => {
                                this.update_status.on_progress(done, total);
                                changed = true;
                            }
                            Ok(updater::DownloadEvent::Finished(result)) => {
                                this.update_status.on_download_finished(result);
                                finished = true;
                                break;
                            }
                            Err(std::sync::mpsc::TryRecvError::Empty) => break,
                            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                                this.update_status.on_download_finished(Err(
                                    "The download stopped unexpectedly.".into(),
                                ));
                                finished = true;
                                break;
                            }
                        }
                    }
                    if changed || finished {
                        cx.notify();
                    }
                });
                if alive.is_err() || finished {
                    break;
                }
            }
        })
        .detach();
    }

    fn reveal_update(&mut self, cx: &mut Context<Self>) {
        let downloaded = match &self.update_status {
            updater::Status::Downloaded { path, .. } => Some(path.clone()),
            _ => None,
        };
        if let Some(path) = downloaded {
            if let Err(err) = updater::reveal_or_open(&path) {
                self.error = Some(err.into());
                cx.notify();
            }
        }
    }

    fn schedule_autosave(&mut self, cx: &mut Context<Self>) {
        let any_untitled = self
            .tabs
            .iter()
            .any(|tab| tab.dirty && is_untitled(&tab.title));
        if !self.autosave && !any_untitled {
            return;
        }
        self.autosave_tick += 1;
        let tick = self.autosave_tick;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(800))
                .await;
            this.update(cx, |this, cx| {
                if this.autosave_tick == tick {
                    this.autosave_dirty(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn autosave_dirty(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        let autosave = self.autosave;
        for tab in &mut self.tabs {
            if !tab.dirty {
                continue;
            }
            if !autosave && !is_untitled(&tab.title) {
                continue;
            }
            match tree::write_text_file(&tab.path, &tab.content) {
                Ok(()) => {
                    tab.saved = tab.content.clone();
                    tab.dirty = false;
                    changed = true;
                }
                Err(err) => {
                    self.error = Some(err.into());
                    break;
                }
            }
        }
        if changed {
            self.error = None;
            self.persist();
            cx.notify();
        }
    }

    // ---- rename and delete ----------------------------------------------

    fn open_rename(&mut self, path: &str, is_folder: bool, cx: &mut Context<Self>) {
        let name = paths::basename_of(path);
        let theme_name = self.theme_name.clone();
        let input = cx.new(|cx| {
            let mut editor = Editor::new(true, cx)
                .with_placeholder("Name")
                .with_padding(8.0, 5.0);
            editor.set_text(&name, cx);
            editor.theme_name = theme_name;
            editor
        });
        let stem = name.rfind('.').filter(|dot| *dot > 0).unwrap_or(name.len());
        input.update(cx, |editor, cx| editor.select(0..stem, cx));
        let this = cx.entity();
        let for_submit = this.clone();
        let for_cancel = this.clone();
        input.update(cx, |editor, _| {
            editor.on_submit = Some(Box::new(move |cx| {
                let this = for_submit.clone();
                cx.defer(move |cx| {
                    let _ = this.update(cx, |this, cx| this.submit_rename(cx));
                });
            }));
            editor.on_cancel = Some(Box::new(move |cx| {
                let this = for_cancel.clone();
                cx.defer(move |cx| {
                    let _ = this.update(cx, |this, cx| {
                        this.modal = None;
                        this.dialog_error = None;
                        cx.notify();
                    });
                });
            }));
        });
        self.dialog_error = None;
        self.modal = Some(Modal::Rename {
            path: path.to_string(),
            is_folder,
            input,
        });
        cx.notify();
    }

    fn submit_rename(&mut self, cx: &mut Context<Self>) {
        let Some(Modal::Rename {
            path,
            is_folder,
            input,
        }) = self.modal.clone()
        else {
            return;
        };
        let name = input.read(cx).text().trim().to_string();
        if let Some(reason) = paths::invalid_name_reason(&name) {
            self.dialog_error = Some(reason.into());
            cx.notify();
            return;
        }
        let old_name = paths::basename_of(&path);
        if name == old_name {
            self.modal = None;
            cx.notify();
            return;
        }
        let new_path = paths::display_join(&paths::dirname_of(&path), &name);
        match tree::rename_path(&path, &new_path) {
            Ok(()) => {
                for tab in &mut self.tabs {
                    if paths::is_under(&tab.path, &path) {
                        tab.path = format!("{}{}", new_path, &tab.path[path.len()..]);
                        tab.title = paths::basename_of(&tab.path);
                    }
                }
                if let Some(active) = self.active.clone() {
                    if paths::is_under(&active, &path) {
                        self.active = Some(format!("{}{}", new_path, &active[path.len()..]));
                    }
                }
                self.open_folders = self
                    .open_folders
                    .iter()
                    .map(|id| {
                        if paths::is_under(id, &path) {
                            format!("{}{}", new_path, &id[path.len()..])
                        } else {
                            id.clone()
                        }
                    })
                    .collect();
                let _ = is_folder;
                self.modal = None;
                self.dialog_error = None;
                self.error = None;
                self.refresh_tree(cx);
                self.persist();
            }
            Err(err) => {
                self.dialog_error = Some(err.into());
                cx.notify();
            }
        }
    }

    fn confirm_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Modal::Delete { paths: doomed, .. }) = self.modal.clone() else {
            return;
        };
        for target in &doomed {
            let is_folder = tree::find_tree_node(&self.tree, target)
                .map(|node| node.is_folder)
                .unwrap_or(true);
            if let Err(err) = tree::delete_path(target, is_folder) {
                self.dialog_error = Some(err.into());
                cx.notify();
                return;
            }
        }
        let remove: Vec<String> = self
            .tabs
            .iter()
            .filter(|tab| doomed.iter().any(|path| paths::is_under(&tab.path, path)))
            .map(|tab| tab.path.clone())
            .collect();
        self.modal = None;
        for tab_path in remove {
            self.close_tab_now(&tab_path, window, cx);
        }
        self.open_folders
            .retain(|id| !doomed.iter().any(|path| paths::is_under(id, path)));
        self.selected
            .retain(|id| !doomed.iter().any(|path| paths::is_under(id, path)));
        self.selection_anchor = None;
        self.error = None;
        self.refresh_tree(cx);
        self.persist();
    }

    // ---- menus ----------------------------------------------------------

    fn row_menu(&self, path: &str, is_folder: bool, is_root: bool) -> Vec<MenuItem> {
        if is_folder {
            let mut items = vec![
                item("New File", MenuAction::NewFile(path.to_string())).with_icon("file-add"),
                item("New Folder", MenuAction::NewFolder(path.to_string())).with_icon("folder-add"),
            ];
            if !is_root {
                items.push(
                    item("Rename…", MenuAction::Rename(path.to_string(), true)).with_icon("pencil-edit01"),
                );
                items.push(
                    danger_item("Delete…", MenuAction::Delete(path.to_string(), true)).with_icon("trash"),
                );
            } else {
                items.push(item("Copy Path", MenuAction::CopyPath(path.to_string())).with_icon("clipboard"));
                items.push(
                    item("Show in File Manager", MenuAction::Reveal(path.to_string())).with_icon("folder-open"),
                );
                items.push(
                    danger_item("Close Project", MenuAction::CloseProject(path.to_string())).with_icon("cancel01"),
                );
            }
            items
        } else {
            vec![
                item("Open", MenuAction::Open(path.to_string())).with_icon("external-link"),
                item("Rename…", MenuAction::Rename(path.to_string(), false)).with_icon("pencil-edit01"),
                item("Duplicate", MenuAction::Duplicate(path.to_string())).with_icon("copy01"),
                item("Copy Path", MenuAction::CopyPath(path.to_string())).with_icon("clipboard"),
                item("Copy Markdown", MenuAction::CopyMarkdown(path.to_string())).with_icon("copy01"),
                item("Export Markdown…", MenuAction::ExportMd(path.to_string())).with_icon("file-export"),
                item("Show in File Manager", MenuAction::Reveal(path.to_string())).with_icon("folder-open"),
                danger_item("Delete…", MenuAction::Delete(path.to_string(), false)).with_icon("trash"),
            ]
        }
    }

    fn tab_menu(&self, path: &str) -> Vec<MenuItem> {
        let switch = self.tabs.iter().find(|tab| tab.path == path).and_then(|tab| {
            if view_kind::modes(tab.kind).len() < 2 {
                None
            } else if matches!(tab.kind, ViewKind::Markdown { .. }) {
                Some("Toggle Rich / Plain")
            } else {
                Some("Switch View")
            }
        });
        let mut items = vec![
            item("Save", MenuAction::Save(path.to_string())).with_icon("floppy-disk"),
            item("Close", MenuAction::Close(path.to_string())).with_icon("cancel01"),
            item("Close Others", MenuAction::CloseOthers(path.to_string())).with_icon("multiplication-sign"),
            item("Close All", MenuAction::CloseAll).with_icon("multiplication-sign"),
        ];
        items.extend(switch.map(|label| {
            let icon = if label == "Switch View" { "source-code" } else { "text-font" };
            item(label, MenuAction::ToggleMode).with_icon(icon)
        }));
        items.extend([
            item("Find…", MenuAction::Find).with_icon("search01"),
            item("Replace…", MenuAction::Replace).with_icon("pencil-edit01"),
            item("Copy Path", MenuAction::CopyPath(path.to_string())).with_icon("clipboard"),
            item("Copy Markdown", MenuAction::CopyMarkdown(path.to_string())).with_icon("copy01"),
            item("Export Markdown…", MenuAction::ExportMd(path.to_string())).with_icon("file-export"),
            item("Export as PDF…", MenuAction::ExportPdf(path.to_string())).with_icon("pdf01"),
        ]);
        items
    }

    /// The ⋯ menu at the end of the tab bar: the active document's actions, or
    /// the workspace's when no document is open.
    fn actions_menu(&self) -> Vec<MenuItem> {
        match self.active.clone() {
            Some(path) => self.tab_menu(&path),
            None => vec![
                item("New File", MenuAction::NewFile(String::new())).with_icon("file-add"),
                item("New Folder", MenuAction::NewFolder(String::new())).with_icon("folder-add"),
                item("Open Folder…", MenuAction::OpenFolder).with_icon("folder-open"),
                item("Settings…", MenuAction::Settings).with_icon("settings01"),
            ],
        }
    }

    fn open_menu(&mut self, x: f32, y: f32, items: Vec<MenuItem>, cx: &mut Context<Self>) {
        self.menu = Some(MenuState { x, y, items });
        cx.notify();
    }

    fn run_menu(&mut self, action: MenuAction, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        match action {
            MenuAction::NewFile(dir) if dir.is_empty() => self.new_file(window, cx),
            MenuAction::NewFile(dir) => self.new_file_in(&dir, window, cx),
            MenuAction::NewFolder(dir) if dir.is_empty() => self.new_folder(cx),
            MenuAction::NewFolder(dir) => self.new_folder_in(&dir, cx),
            MenuAction::OpenFolder => self.pick_folder(cx, false),
            MenuAction::Rename(path, is_folder) => self.open_rename(&path, is_folder, cx),
            MenuAction::Delete(path, is_folder) => {
                self.dialog_error = None;
                // Delete acts on the whole selection when the row is part of it.
                let paths = if self.selected.iter().any(|selected| selected == &path) {
                    self.selected.clone()
                } else {
                    vec![path]
                };
                self.modal = Some(Modal::Delete { paths, is_folder });
                cx.notify();
            }
            MenuAction::Duplicate(path) => match tree::duplicate_file(&path) {
                Ok(new_path) => {
                    self.refresh_tree(cx);
                    self.open_document(&new_path, Some(window), cx);
                }
                Err(err) => {
                    self.error = Some(err.into());
                    cx.notify();
                }
            },
            MenuAction::CopyPath(path) => {
                cx.write_to_clipboard(ClipboardItem::new_string(path));
                cx.notify();
            }
            MenuAction::Reveal(path) => cx.reveal_path(Path::new(&path)),
            MenuAction::CloseProject(dir) => self.close_project(&dir, window, cx),
            MenuAction::Open(path) => self.open_document(&path, Some(window), cx),
            MenuAction::Save(path) => {
                self.save_path(&path, cx);
            }
            MenuAction::Close(path) => self.request_close(&path, window, cx),
            MenuAction::CloseOthers(path) => self.close_others(&path, window, cx),
            MenuAction::CloseAll => self.close_all(window, cx),
            MenuAction::CopyMarkdown(path) => self.copy_markdown(&path, cx),
            MenuAction::ExportMd(path) => self.export_md(&path, cx),
            MenuAction::ExportPdf(path) => self.export_pdf(&path, cx),
            MenuAction::Find => self.open_find(false, window, cx),
            MenuAction::Replace => self.open_find(true, window, cx),
            MenuAction::ToggleMode => self.toggle_mode(window, cx),
            MenuAction::Settings => self.open_settings(SettingsSection::General, cx),
        }
    }

    fn apply_theme(&mut self, name: &str, cx: &mut Context<Self>) {
        self.theme_name = name.to_string();
        for tab in &self.tabs {
            if let Some(rich) = &tab.rich {
                rich.update(cx, |editor, cx| editor.set_theme(name, cx));
            }
            match &tab.view {
                TabView::Text(editor) => editor.update(cx, |editor, cx| editor.set_theme(name, cx)),
                TabView::Csv(view) => view.update(cx, |view, cx| view.set_theme(name, cx)),
                TabView::Json(view) => view.update(cx, |view, cx| view.set_theme(name, cx)),
            }
        }
        if let Some(find) = &self.find {
            find.query
                .update(cx, |editor, cx| editor.set_theme(name, cx));
            find.replace
                .update(cx, |editor, cx| editor.set_theme(name, cx));
        }
        if let Some(Modal::Rename { input, .. }) = &self.modal {
            input
                .update(cx, |editor, cx| editor.set_theme(name, cx));
        }
        self.persist();
        cx.notify();
    }

    // ---- actions -------------------------------------------------------

    fn save_action(&mut self, _: &Save, _: &mut Window, cx: &mut Context<Self>) {
        self.save_active(cx);
    }

    fn export_md_action(&mut self, _: &ExportMd, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.active.clone() {
            self.export_md(&path, cx);
        }
    }

    fn export_pdf_action(&mut self, _: &ExportPdf, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.active.clone() {
            self.export_pdf(&path, cx);
        }
    }

    fn new_file_action(&mut self, _: &NewFile, window: &mut Window, cx: &mut Context<Self>) {
        self.new_file(window, cx);
    }

    fn new_folder_action(&mut self, _: &NewFolder, _: &mut Window, cx: &mut Context<Self>) {
        self.new_folder(cx);
    }

    fn close_tab_action(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal.is_some() {
            self.modal = None;
            cx.notify();
            return;
        }
        if let Some(path) = self.active.clone() {
            self.request_close(&path, window, cx);
        }
    }

    fn close_overlay_action(
        &mut self,
        _: &CloseOverlay,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.menu.is_some() {
            self.menu = None;
            cx.notify();
            return;
        }
        if self.modal.is_some() {
            self.modal = None;
            self.dialog_error = None;
            cx.notify();
            return;
        }
        if self.find.is_some() {
            self.close_find(Some(window), cx);
        }
    }

    fn delete_selection_action(
        &mut self,
        _: &DeleteSelection,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_selected(cx);
    }

    fn toggle_sidebar_action(
        &mut self,
        _: &ToggleSidebar,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        self.persist();
        cx.notify();
    }

    fn toggle_mode_action(&mut self, _: &ToggleMode, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_mode(window, cx);
    }

    fn open_file_action(&mut self, _: &OpenFile, _: &mut Window, cx: &mut Context<Self>) {
        self.pick_file(cx);
    }

    fn open_folder_action(&mut self, _: &OpenFolder, _: &mut Window, cx: &mut Context<Self>) {
        self.pick_folder(cx, false);
    }

    fn find_action(&mut self, _: &Find, window: &mut Window, cx: &mut Context<Self>) {
        self.open_find(false, window, cx);
    }

    fn replace_action(&mut self, _: &Replace, window: &mut Window, cx: &mut Context<Self>) {
        self.open_find(true, window, cx);
    }

    fn settings_action(&mut self, _: &Settings, _: &mut Window, cx: &mut Context<Self>) {
        self.open_settings(SettingsSection::General, cx);
    }

    fn shortcuts_action(&mut self, _: &Shortcuts, _: &mut Window, cx: &mut Context<Self>) {
        self.open_settings(SettingsSection::Shortcuts, cx);
    }

    fn copy_markdown_action(
        &mut self,
        _: &CopyMarkdown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(path) = self.active.clone() {
            self.copy_markdown(&path, cx);
        }
    }

    fn copy_path_action(&mut self, _: &CopyPath, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.active.clone() {
            cx.write_to_clipboard(ClipboardItem::new_string(path));
            cx.notify();
        }
    }

    fn goto_tab(&mut self, number: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.is_empty() {
            return;
        }
        let index = if number >= 9 {
            self.tabs.len() - 1
        } else {
            (number - 1).min(self.tabs.len() - 1)
        };
        self.active = Some(self.tabs[index].path.clone());
        self.focus_editor(window, cx);
        self.persist();
        cx.notify();
    }

    fn next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        self.cycle_tab(1, window, cx);
    }

    fn prev_tab(&mut self, _: &PrevTab, window: &mut Window, cx: &mut Context<Self>) {
        self.cycle_tab(-1, window, cx);
    }

    fn cycle_tab(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.active_index() else {
            return;
        };
        let len = self.tabs.len() as isize;
        let next = (index as isize + delta).rem_euclid(len) as usize;
        self.active = Some(self.tabs[next].path.clone());
        self.focus_editor(window, cx);
        self.persist();
        cx.notify();
    }

    // ---- rendering -----------------------------------------------------

    fn render_welcome(&mut self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let folder = paths::default_folder_path()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        div()
            .flex()
            .flex_col()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .bg(theme.bg)
            .text_color(theme.fg)
            .child(
                gpui::svg()
                    .path(app_icon::LOGO_PATH)
                    .w(px(64.0))
                    .h(px(55.0))
                    .text_color(theme.accent),
            )
            .child(
                div()
                    .text_size(px(30.0))
                    .font_weight(FontWeight::BOLD)
                    .child("Zyplus"),
            )
            .child(
                div()
                    .text_size(px(15.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Welcome to Zyplus"),
            )
            .child(
                div()
                    .max_w(px(400.0))
                    .text_size(px(12.0))
                    .text_color(theme.muted)
                    .child(
                        "Zyplus can organize your notes in a dedicated folder, or you \
                         can open existing files directly.",
                    ),
            )
            .child(
                div()
                    .px_3()
                    .py_1()
                    .rounded(px(6.0))
                    .bg(theme.surface)
                    .text_size(px(11.0))
                    .text_color(theme.muted)
                    .child(folder),
            )
            .children(
                self.error
                    .clone()
                    .map(|error| div().text_size(px(11.0)).text_color(theme.danger).child(error)),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        primary_button(
                            theme,
                            "welcome-create",
                            icon_label("folder-add", 18.0, rgb(0xffffff), "Create folder"),
                        )
                        .on_click(
                            cx.listener(|this, _, _, cx| this.create_default_folder(cx)),
                        ),
                    )
                    .child(
                        secondary_button(theme, "welcome-open-folder", "Open project")
                            .on_click(cx.listener(|this, _, _, cx| this.pick_folder(cx, false))),
                    )
                    .child(
                        secondary_button(theme, "welcome-open-file", "Open file")
                            .on_click(cx.listener(|this, _, _, cx| this.pick_file(cx))),
                    )
                    .child(
                        secondary_button(theme, "welcome-skip", "Skip")
                            .on_click(cx.listener(|this, _, _, cx| this.skip_welcome(cx))),
                    ),
            )
    }

    fn render_sidebar(&mut self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let rows = tree::visible_rows(&self.tree, &self.open_folders);
        let active = self.active.clone();
        let selected_rows: BTreeSet<String> = self.selected.iter().cloned().collect();

        let update_pill = if self.update_status.info().is_some() {
            Some(
                div()
                    .id("update-pill")
                    .px_2()
                    .h(px(24.0))
                    .flex()
                    .items_center()
                    .rounded(px(5.0))
                    .bg(theme.accent_soft)
                    .text_size(px(11.0))
                    .text_color(theme.accent)
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.open_settings(SettingsSection::About, cx)
                    }))
                    .child("Update"),
            )
        } else {
            None
        };

        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .pl(px(TRAFFIC_LIGHT_INSET))
            .pr_3()
            .h(px(44.0))
            .flex_shrink_0()
            .window_control_area(WindowControlArea::Drag)
            .child(
                gpui::svg()
                    .path(app_icon::LOGO_PATH)
                    .w(px(18.0))
                    .h(px(16.0))
                    .flex_shrink_0()
                    .text_color(theme.accent),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .child(
                        icon_button(
                            theme,
                            "open-file",
                            app_icon::icon("file01", 16.0).text_color(theme.muted),
                            24.0,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.pick_file(cx))),
                    )
                    .child(
                        icon_button(
                            theme,
                            "open-folder",
                            app_icon::icon("folder-open", 16.0).text_color(theme.muted),
                            24.0,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.pick_folder(cx, false))),
                    )
                    .children(update_pill),
            );

        let actions_row = div()
            .flex()
            .flex_row()
            .gap_1()
            .px_2()
            .pb_2()
            .flex_shrink_0()
            .child(
                div()
                    .id("new-file")
                    .flex_1()
                    .h(px(28.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .bg(theme.accent)
                    .text_color(rgb(0xffffff))
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .cursor_pointer()
                    .hover(move |style| style.opacity(0.9))
                    .on_click(cx.listener(|this, _, window, cx| this.new_file(window, cx)))
                    .child(icon_label("file-add", 16.0, rgb(0xffffff), "New File")),
            )
            .child(
                div()
                    .id("new-folder")
                    .w(px(28.0))
                    .h(px(28.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .bg(theme.surface)
                    .border_1()
                    .border_color(theme.border)
                    .text_color(theme.fg)
                    .cursor_pointer()
                    .hover(move |style| style.bg(theme.hover))
                    .on_click(cx.listener(|this, _, _, cx| this.new_folder(cx)))
                    .child(app_icon::icon("folder-add", 16.0).text_color(theme.fg)),
            );

        let body = if self.roots.is_empty() {
            div()
                .flex()
                .flex_col()
                .flex_1()
                .items_center()
                .justify_center()
                .gap_2()
                .px_4()
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(theme.muted)
                        .child("No folder open"),
                )
                .child(
                    primary_button(
                        theme,
                        "sidebar-open-folder",
                        icon_label("folder-open", 18.0, rgb(0xffffff), "Open Folder"),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.pick_folder(cx, false))),
                )
                .into_any_element()
        } else {
            div()
                .id("tree-scroll")
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.0))
                .px_1()
                .py_1()
                .overflow_y_scroll()
                .children(rows.into_iter().map(|row| {
                    let is_active = active.as_deref() == Some(row.id.as_str());
                    let is_selected = selected_rows.contains(&row.id);
                    let is_folder = row.is_folder;
                    let is_root = row.is_root;
                    let row_id = row.id.clone();
                    let drop_dir = row.id.clone();
                    let name = if row.is_root {
                        row.name.to_uppercase()
                    } else {
                        row.name.clone()
                    };
                    let glyph = if row.is_folder {
                        if row.is_open {
                            "folder-open"
                        } else {
                            "folder01"
                        }
                    } else if paths::is_markdown_file(&row.name) {
                        "file-text"
                    } else {
                        "file01"
                    };
                    let chevron = if row.is_open { "arrow-down01" } else { "arrow-right01" };
                    let group = SharedString::from(format!("tree-{}", row.id));
                    let text_color = if is_active {
                        theme.accent
                    } else if row.is_folder {
                        theme.muted
                    } else {
                        theme.fg
                    };
                    let hover_bg = theme.hover;
                    let accent_soft = theme.accent_soft;
                    let menu_path = row.id.clone();
                    let new_file_dir = row.id.clone();
                    let new_folder_dir = row.id.clone();
                    let hover_group = group.clone();
                    div()
                        .id(SharedString::from(format!("row-{}", row.id)))
                        .group(group)
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .h(px(25.0))
                        .pl(px(8.0 + row.depth as f32 * 14.0))
                        .pr_1()
                        .rounded(px(5.0))
                        .cursor_pointer()
                        .text_size(px(12.0))
                        .text_color(text_color)
                        .when(row.is_root, |el| el.font_weight(FontWeight::SEMIBOLD))
                        .when(is_active, |el| el.bg(accent_soft))
                        .when(is_selected && !is_active, |el| el.bg(hover_bg))
                        .when(!is_active, |el| el.hover(move |style| style.bg(hover_bg)))
                        .when(!is_root, |el| {
                            let ghost = if is_selected && selected_rows.len() > 1 {
                                format!("{} items", selected_rows.len())
                            } else {
                                name.clone()
                            };
                            let drag_paths: Vec<String> = if is_selected && selected_rows.len() > 1 {
                                selected_rows.iter().cloned().collect()
                            } else {
                                vec![row_id.clone()]
                            };
                            let ghost_theme = theme.clone();
                            el.on_drag(
                                DragPath {
                                    paths: drag_paths,
                                    label: ghost,
                                },
                                move |drag: &DragPath, _offset, _window, cx| {
                                    let label = SharedString::from(drag.label.clone());
                                    let theme = ghost_theme.clone();
                                    cx.new(|_| DragGhost { label, theme })
                                },
                            )
                        })
                        .when(is_folder, |el| {
                            el.on_drop(cx.listener(
                                move |this, dragged: &DragPath, _, cx| {
                                    this.move_into(&dragged.paths, &drop_dir, cx);
                                },
                            ))
                        })
                        .on_mouse_down(MouseButton::Left, {
                            let select_id = row_id.clone();
                            cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                                this.select_row(&select_id, &event.modifiers, cx);
                            })
                        })
                        .on_click(cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                            let modifiers = match event {
                                gpui::ClickEvent::Mouse(mouse) => mouse.down.modifiers,
                                gpui::ClickEvent::Keyboard(_) => gpui::Modifiers::default(),
                            };
                            if modifiers.shift || modifiers.secondary() || modifiers.alt {
                                return;
                            }
                            let double = this.register_row_click(&row_id);
                            if is_folder {
                                if double && !is_root {
                                    this.open_rename(&row_id, true, cx);
                                } else {
                                    this.toggle_folder(&row_id, cx);
                                }
                            } else if double {
                                this.open_rename(&row_id, false, cx);
                            } else {
                                this.open_document(&row_id, Some(window), cx);
                            }
                            cx.notify();
                        }))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                                if !this.selected.iter().any(|path| path == &menu_path) {
                                    this.selected = vec![menu_path.clone()];
                                    this.selection_anchor = Some(menu_path.clone());
                                }
                                let items = this.row_menu(&menu_path, is_folder, is_root);
                                this.open_menu(
                                    f32::from(event.position.x),
                                    f32::from(event.position.y),
                                    items,
                                    cx,
                                );
                            }),
                        )
                        .child(
                            div()
                                .w(px(14.0))
                                .h(px(14.0))
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .opacity(0.6)
                                .children(
                                    row.is_folder
                                        .then(|| app_icon::icon(chevron, 13.0).text_color(text_color)),
                                ),
                        )
                        .child(app_icon::icon(glyph, 16.0).text_color(text_color).opacity(0.7))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(name),
                        )
                        .when(row.is_folder, |el| {
                            el.child(
                                div()
                                    .flex()
                                    .flex_shrink_0()
                                    .items_center()
                                    .gap_0p5()
                                    .opacity(0.0)
                                    .group_hover(hover_group, |style| style.opacity(1.0))
                                    .child(tree_action(
                                        theme,
                                        "new-file-here",
                                        new_file_dir.clone(),
                                        "file-add",
                                        MenuAction::NewFile(new_file_dir),
                                        cx,
                                    ))
                                    .child(tree_action(
                                        theme,
                                        "new-folder-here",
                                        new_folder_dir.clone(),
                                        "folder-add",
                                        MenuAction::NewFolder(new_folder_dir),
                                        cx,
                                    )),
                            )
                        })
                        .into_any_element()
                }))
                .into_any_element()
        };

        let footer = div()
            .flex_shrink_0()
            .mt_auto()
            .p_2()
            .border_t_1()
            .border_color(theme.border)
            .child(
                div()
                    .id("sidebar-settings")
                    .h(px(28.0))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded(px(5.0))
                    .cursor_pointer()
                    .text_size(px(12.0))
                    .text_color(theme.fg)
                    .hover(move |style| style.bg(theme.hover))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.open_settings(SettingsSection::General, cx)
                    }))
                    .child(app_icon::icon("settings01", 16.0).text_color(theme.fg))
                    .child("Settings"),
            );

        div()
            .flex()
            .flex_col()
            .w(px(264.0))
            .h_full()
            .flex_shrink_0()
            .border_r_1()
            .border_color(theme.border)
            .bg(theme.sidebar_bg)
            .child(header)
            .child(actions_row)
            .child(body)
            .child(footer)
    }

    fn render_tab_bar(&mut self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let active_switch = self
            .active_tab()
            .filter(|tab| view_kind::modes(tab.kind).len() > 1)
            .map(|tab| (tab.kind, tab.mode.clone()));
        let tabs = self
            .tabs
            .iter()
            .map(|tab| {
                let is_active = self.active.as_deref() == Some(tab.path.as_str());
                let path = tab.path.clone();
                let close_path = tab.path.clone();
                let menu_path = tab.path.clone();
                let accent = theme.accent;
                let accent_soft = theme.accent_soft;
                let muted = theme.muted;
                let hover = theme.hover;
                let drag_path = tab.path.clone();
                let drag_title = tab.title.clone();
                let drop_path = tab.path.clone();
                let ghost_theme = theme.clone();
                div()
                    .id(SharedString::from(format!("tab-{}", tab.path)))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .h(px(28.0))
                    .px_2()
                    .rounded(px(6.0))
                    .flex_shrink_0()
                    .cursor_pointer()
                    .text_size(px(12.0))
                    .text_color(if is_active { accent } else { muted })
                    .when(is_active, |el| {
                        el.bg(accent_soft).font_weight(FontWeight::MEDIUM)
                    })
                    .when(!is_active, |el| el.hover(move |style| style.bg(hover)))
                    .on_drag(
                        DragTab {
                            path: drag_path,
                            title: drag_title,
                        },
                        move |drag: &DragTab, _offset, _window, cx| {
                            let label = SharedString::from(drag.title.clone());
                            let theme = ghost_theme.clone();
                            cx.new(|_| DragGhost { label, theme })
                        },
                    )
                    .on_drop(cx.listener(move |this, dragged: &DragTab, _, cx| {
                        this.reorder_tab(&dragged.path, Some(&drop_path), cx);
                    }))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.active = Some(path.clone());
                        this.focus_editor(window, cx);
                        this.persist();
                        cx.notify();
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                            let items = this.tab_menu(&menu_path);
                            this.open_menu(
                                f32::from(event.position.x),
                                f32::from(event.position.y),
                                items,
                                cx,
                            );
                        }),
                    )
                    .children(if tab.dirty {
                        Some(
                            div()
                                .w(px(6.0))
                                .h(px(6.0))
                                .rounded(px(3.0))
                                .flex_shrink_0()
                                .bg(if is_active { accent } else { muted }),
                        )
                    } else {
                        None
                    })
                    .child(
                        div()
                            .max_w(px(180.0))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(tab.title.clone()),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("close-{}", tab.path)))
                            .w(px(16.0))
                            .h(px(16.0))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.0))
                            .text_color(muted)
                            .cursor_pointer()
                            .hover(move |style| style.bg(hover))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.request_close(&close_path, window, cx);
                            }))
                            .child(app_icon::icon("cancel01", 13.0).text_color(muted)),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();

        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .h(px(44.0))
            .flex_shrink_0()
            .pr_2()
            .when(self.sidebar_collapsed, |el| el.pl(px(TRAFFIC_LIGHT_INSET)))
            .when(!self.sidebar_collapsed, |el| el.pl_2())
            .window_control_area(WindowControlArea::Drag)
            .bg(theme.surface)
            .border_b_1()
            .border_color(theme.border)
            .child(
                icon_button(
                    theme,
                    "toggle-sidebar",
                    app_icon::icon("sidebar-left", 16.0).text_color(if self.sidebar_collapsed {
                        theme.muted
                    } else {
                        theme.accent
                    }),
                    28.0,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.sidebar_collapsed = !this.sidebar_collapsed;
                    this.persist();
                    cx.notify();
                })),
            )
            .child(
                div()
                    .id("tab-strip")
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_x_scroll()
                    .children(tabs)
                    .child(
                        icon_button(
                            theme,
                            "tab-new-file",
                            app_icon::icon("plus-sign", 16.0).text_color(theme.muted),
                            26.0,
                        )
                        .on_click(cx.listener(|this, _, window, cx| this.new_file(window, cx))),
                    )
                    .child(
                        // Empty strip after the last tab: dropping here moves the
                        // tab to the end. A sibling, not a parent, so a drop on a
                        // tab is not also seen by it.
                        div()
                            .id("tab-strip-end")
                            .flex_1()
                            .min_w(px(12.0))
                            .h(px(28.0))
                            .on_drop(cx.listener(|this, dragged: &DragTab, _, cx| {
                                this.reorder_tab(&dragged.path, None, cx);
                            })),
                    ),
            )
            .children(active_switch.map(|(kind, mode)| {
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .flex_shrink_0()
                    .children(view_kind::modes(kind).iter().map(|&option| {
                        mode_button(
                            theme,
                            mode_button_id(option),
                            mode_icon(option),
                            view_kind::mode_label(option),
                            mode == option,
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.set_mode(option, window, cx)
                        }))
                    }))
            }))
            .child(
                div()
                    .id("tab-actions")
                    .w(px(28.0))
                    .h(px(28.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.0))
                    .flex_shrink_0()
                    .cursor_pointer()
                    .text_size(px(14.0))
                    .text_color(theme.muted)
                    .hover(move |style| style.bg(theme.hover))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                            let items = this.actions_menu();
                            this.open_menu(
                                f32::from(event.position.x) - 180.0,
                                f32::from(event.position.y) + 8.0,
                                items,
                                cx,
                            );
                        }),
                    )
                    .child(app_icon::icon("more-horizontal", 16.0).text_color(theme.muted)),
            )
    }

    fn render_find_bar(&mut self, theme: &Theme, cx: &mut Context<Self>) -> Option<Div> {
        let find = self.find.as_ref()?;
        let query = find.query.clone();
        let replace = find.replace.clone();
        let show_replace = find.show_replace;
        let index = find.index;
        let total = find.matches.len();
        let searchable = self.active_searchable(cx).is_some();
        let count = if find.query_text.is_empty() {
            String::new()
        } else if !searchable {
            "Text view only".to_string()
        } else if total == 0 {
            "No results".to_string()
        } else {
            format!("{} of {}", index + 1, total)
        };

        let field = |input: Entity<Editor>, width: f32| {
            div()
                .h(px(28.0))
                .w(px(width))
                .flex_shrink_0()
                .rounded(px(6.0))
                .border_1()
                .border_color(theme.border)
                .bg(theme.bg)
                .overflow_hidden()
                .child(input)
        };

        let first_row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .child(field(query, 220.0))
            .child(
                div()
                    .w(px(64.0))
                    .flex_shrink_0()
                    .text_size(px(11.0))
                    .text_color(theme.muted)
                    .child(count),
            )
            .child(
                icon_button(
                    theme,
                    "find-prev",
                    app_icon::icon("arrow-up01", 15.0).text_color(theme.muted),
                    28.0,
                )
                .on_click(cx.listener(|this, _, _, cx| this.step_find(-1, cx))),
            )
            .child(
                icon_button(
                    theme,
                    "find-next",
                    app_icon::icon("arrow-down01", 15.0).text_color(theme.muted),
                    28.0,
                )
                .on_click(cx.listener(|this, _, _, cx| this.step_find(1, cx))),
            )
            .child(
                icon_button(
                    theme,
                    "find-toggle-replace",
                    // The web turns the chevron to point down while the replace row is open.
                    app_icon::icon("arrow-right01", 15.0)
                        .text_color(theme.muted)
                        .with_transformation(gpui::Transformation::rotate(gpui::radians(
                            if show_replace { std::f32::consts::FRAC_PI_2 } else { 0.0 },
                        ))),
                    28.0,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(find) = this.find.as_mut() {
                        find.show_replace = !find.show_replace;
                    }
                    cx.notify();
                })),
            )
            .child(div().flex_1())
            .child(
                icon_button(
                    theme,
                    "find-close",
                    app_icon::icon("cancel01", 15.0).text_color(theme.muted),
                    28.0,
                )
                .on_click(cx.listener(|this, _, window, cx| this.close_find(Some(window), cx))),
            );

        let second_row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .child(field(replace, 220.0))
            .child(
                toolbar_button(theme, "replace-one", "Replace").on_click(cx.listener(
                    |this, _, _, cx| this.replace_current(cx),
                )),
            )
            .child(
                toolbar_button(theme, "replace-all", "Replace All").on_click(cx.listener(
                    |this, _, _, cx| this.replace_all(cx),
                )),
            )
            .child(div().flex_1());

        Some(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .flex_shrink_0()
                .px_3()
                .py_2()
                .bg(theme.surface)
                .border_b_1()
                .border_color(theme.border)
                .child(first_row)
                .when(show_replace, |el| el.child(second_row)),
        )
    }

    fn render_status(
        &mut self,
        theme: &Theme,
        path: String,
        dirty: bool,
        kind: ViewKind,
        mode: String,
        cx: &mut Context<Self>,
    ) -> Div {
        let switchable = view_kind::modes(kind).len() > 1;
        // Markdown reads as Rich or Markdown; other views name their mode.
        let (toggle_label, toggle_lit) = match kind {
            ViewKind::Markdown { .. } => {
                let rich = mode == "rich";
                (if rich { "Rich" } else { "Markdown" }, rich)
            }
            _ => (view_kind::mode_label(&mode), true),
        };
        let words = self
            .active_tab()
            .map(|tab| tab.content.split_whitespace().count())
            .unwrap_or(0);
        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap_2()
            .h(px(26.0))
            .flex_shrink_0()
            .px_3()
            .bg(theme.surface)
            .border_t_1()
            .border_color(theme.border)
            .text_size(px(11.0))
            .text_color(theme.muted)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(path),
            )
            .children(switchable.then(|| {
                div()
                    .id("mode-toggle")
                    .px_2()
                    .h(px(18.0))
                    .flex()
                    .items_center()
                    .rounded(px(4.0))
                    .cursor_pointer()
                    .text_color(if toggle_lit { theme.accent } else { theme.muted })
                    .hover(move |style| style.bg(theme.hover))
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_mode(window, cx)))
                    .child(toggle_label)
            }))
            .child(div().flex_shrink_0().child(format!("{words} words")))
            .child(
                div()
                    .flex_shrink_0()
                    .child(if dirty { "Edited" } else { "Saved" }),
            )
    }

    fn render_body(&mut self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let Some(index) = self.active_index() else {
            return div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_1()
                .bg(theme.bg)
                .child(
                    div()
                        .text_size(px(14.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.fg)
                        .child("Zyplus"),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(theme.muted)
                        .child("Open a folder and select a markdown file to start editing."),
                );
        };

        let (path, dirty, kind, mode, content, view) = {
            let tab = &self.tabs[index];
            (
                tab.path.clone(),
                tab.dirty,
                tab.kind,
                tab.mode.clone(),
                tab.content.clone(),
                tab.view.clone(),
            )
        };

        let find_bar = self.render_find_bar(theme, cx);
        let rich = matches!(kind, ViewKind::Markdown { .. }) && mode == "rich";
        let rich_view = if rich {
            self.ensure_rich(index, cx);
            let charts = self.extension_enabled("chart");
            if let Some(editor) = self.tabs[index].rich.clone() {
                editor.update(cx, |editor, cx| editor.set_charts_enabled(charts, cx));
            }
            self.tabs[index].rich.clone()
        } else {
            None
        };
        let main = if let Some(rich_view) = rich_view {
            div()
                .flex_1()
                .min_h(px(0.0))
                .overflow_hidden()
                .child(rich_view)
                .into_any_element()
        } else if rich {
            let charts = self.extension_enabled("chart");
            preview::render_preview(&content, std::path::Path::new(&path), theme, charts)
                .into_any_element()
        } else {
            let child: AnyElement = match view {
                TabView::Text(editor) => editor.into_any_element(),
                TabView::Csv(view) => view.into_any_element(),
                TabView::Json(view) => view.into_any_element(),
            };
            div()
                .flex_1()
                .min_h(px(0.0))
                .overflow_hidden()
                .child(child)
                .into_any_element()
        };
        let status = self.render_status(theme, path, dirty, kind, mode, cx);

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .children(find_bar)
            .child(main)
            .child(status)
    }

    fn render_menu(&mut self, theme: &Theme, cx: &mut Context<Self>) -> Option<Div> {
        let menu = self.menu.clone()?;
        let items = menu
            .items
            .iter()
            .map(|item| {
                let action = item.action.clone();
                let mut row = div()
                    .id(SharedString::from(format!("menu-{}", item.label)))
                    .h(px(28.0))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(12.0))
                    .text_color(if item.danger { theme.danger } else { theme.fg })
                    .children(item.icon.map(|icon| {
                        app_icon::icon(icon, 16.0).text_color(if item.danger { theme.danger } else { theme.fg })
                    }))
                    .child(item.label);
                if item.disabled {
                    row = row.opacity(0.4);
                } else {
                    row = row
                        .cursor_pointer()
                        .hover(move |style| style.bg(theme.hover))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.run_menu(action.clone(), window, cx);
                        }));
                }
                row.into_any_element()
            })
            .collect::<Vec<_>>();

        let panel = div()
            .absolute()
            .left(px(menu.x))
            .top(px(menu.y))
            .w(px(220.0))
            .py_1()
            .flex()
            .flex_col()
            .rounded(px(8.0))
            .bg(theme.surface)
            .border_1()
            .border_color(theme.border)
            .children(items);

        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .child(
                    div()
                        .id("menu-backdrop")
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                            this.menu = None;
                            cx.notify();
                        }))
                        .on_mouse_down(MouseButton::Right, cx.listener(|this, _, _, cx| {
                            this.menu = None;
                            cx.notify();
                        })),
                )
                .child(panel),
        )
    }

    fn render_modal(&mut self, theme: &Theme, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let modal = self.modal.clone()?;
        let card: Div = match modal {
            Modal::Rename {
                path,
                is_folder,
                input,
            } => {
                let what = if is_folder { "folder" } else { "file" };
                div()
                    .flex()
                    .flex_col()
                    .w(px(420.0))
                    .p_4()
                    .gap_3()
                    .rounded(px(12.0))
                    .bg(theme.surface)
                    .border_1()
                    .border_color(theme.border)
                    .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                        cx.stop_propagation()
                    })
                    .child(
                        div()
                            .text_size(px(15.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.fg)
                            .child(format!("Rename {what}")),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(theme.muted)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(path.clone()),
                    )
                    .child(
                        div()
                            .h(px(32.0))
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(theme.border)
                            .bg(theme.bg)
                            .overflow_hidden()
                            .child(input),
                    )
                    .children(self.dialog_error.clone().map(|error| {
                        div()
                            .text_size(px(11.0))
                            .text_color(theme.danger)
                            .child(error)
                    }))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_end()
                            .gap_2()
                            .child(
                                secondary_button(theme, "rename-cancel", "Cancel").on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.modal = None;
                                        this.dialog_error = None;
                                        cx.notify();
                                    }),
                                ),
                            )
                            .child(
                                primary_button(theme, "rename-submit", "Rename").on_click(
                                    cx.listener(|this, _, _, cx| this.submit_rename(cx)),
                                ),
                            ),
                    )
            }
            Modal::NewFolder { dir, input } => {
                div()
                    .flex()
                    .flex_col()
                    .w(px(420.0))
                    .p_4()
                    .gap_3()
                    .rounded(px(12.0))
                    .bg(theme.surface)
                    .border_1()
                    .border_color(theme.border)
                    .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                        cx.stop_propagation()
                    })
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_3()
                            .child(dialog_badge(
                                "folder-add",
                                32.0,
                                16.0,
                                theme.accent,
                                soft(theme.accent),
                            ))
                            .child(
                                div()
                                    .text_size(px(15.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.fg)
                                    .child("New Folder"),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(theme.muted)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(dir.clone()),
                    )
                    .child(
                        div()
                            .h(px(32.0))
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(theme.border)
                            .bg(theme.bg)
                            .overflow_hidden()
                            .child(input),
                    )
                    .children(self.dialog_error.clone().map(|error| {
                        div()
                            .text_size(px(11.0))
                            .text_color(theme.danger)
                            .child(error)
                    }))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_end()
                            .gap_2()
                            .child(
                                secondary_button(theme, "new-folder-cancel", "Cancel").on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.modal = None;
                                        this.dialog_error = None;
                                        cx.notify();
                                    }),
                                ),
                            )
                            .child(
                                primary_button(theme, "new-folder-submit", "Create").on_click(
                                    cx.listener(|this, _, _, cx| this.submit_new_folder(cx)),
                                ),
                            ),
                    )
            }
            Modal::Delete { paths, is_folder } => {
                let what = if paths.len() > 1 {
                    format!("{} items", paths.len())
                } else if is_folder {
                    "folder".to_string()
                } else {
                    "file".to_string()
                };
                let names = paths
                    .iter()
                    .map(|path| paths::basename_of(path))
                    .collect::<Vec<_>>()
                    .join(", ");
                div()
                    .flex()
                    .flex_col()
                    .w(px(420.0))
                    .p_4()
                    .gap_3()
                    .rounded(px(12.0))
                    .bg(theme.surface)
                    .border_1()
                    .border_color(theme.border)
                    .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                        cx.stop_propagation()
                    })
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_3()
                            .child(dialog_badge("alert01", 32.0, 18.0, theme.danger, soft(theme.danger)))
                            .child(
                                div()
                                    .text_size(px(15.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.fg)
                                    .child(format!("Delete {what}?")),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(theme.muted)
                            .child(if paths.len() == 1 {
                                format!(
                                    "\"{}\" will be deleted from disk. This cannot be undone.",
                                    names
                                )
                            } else {
                                format!(
                                    "{} will be deleted from disk. This cannot be undone.",
                                    names
                                )
                            }),
                    )
                    .children(self.dialog_error.clone().map(|error| {
                        div()
                            .text_size(px(11.0))
                            .text_color(theme.danger)
                            .child(error)
                    }))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_end()
                            .gap_2()
                            .child(
                                secondary_button(theme, "delete-cancel", "Cancel").on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.modal = None;
                                        this.dialog_error = None;
                                        cx.notify();
                                    }),
                                ),
                            )
                            .child(
                                danger_button(theme, "delete-confirm", "Delete").on_click(
                                    cx.listener(|this, _, window, cx| {
                                        this.confirm_delete(window, cx)
                                    }),
                                ),
                            ),
                    )
            }
            Modal::Unsaved { path } => {
                let discard_path = path.clone();
                let save_path = path.clone();
                div()
                .flex()
                .flex_col()
                .w(px(440.0))
                .p_4()
                .gap_3()
                .rounded(px(12.0))
                .bg(theme.surface)
                .border_1()
                .border_color(theme.border)
                .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                    cx.stop_propagation()
                })
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .child(dialog_badge(
                            "alert01",
                            32.0,
                            18.0,
                            theme.warning,
                            soft(theme.warning),
                        ))
                        .child(
                            div()
                                .text_size(px(15.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.fg)
                                .child("Save changes?"),
                        ),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(theme.muted)
                        .child(format!(
                            "\"{}\" has unsaved changes.",
                            paths::basename_of(&path)
                        )),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_end()
                        .gap_2()
                        .child(
                            secondary_button(theme, "unsaved-cancel", "Cancel").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.modal = None;
                                    cx.notify();
                                }),
                            ),
                        )
                        .child(
                            secondary_button(theme, "unsaved-discard", "Discard").on_click(
                                cx.listener(move |this, _, window, cx| {
                                    this.modal = None;
                                    this.close_tab_now(&discard_path, window, cx);
                                }),
                            ),
                        )
                        .child(
                            primary_button(theme, "unsaved-save", "Save").on_click(cx.listener(
                                move |this, _, window, cx| {
                                    if this.save_path(&save_path, cx) {
                                        this.modal = None;
                                        this.close_tab_now(&save_path, window, cx);
                                    }
                                },
                            )),
                        ),
                )
            }
            Modal::Settings { section } => self.render_settings_card(theme, section, cx),
        };

        Some(
            div()
                .id("modal-backdrop")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(rgba(0x00000040))
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                    this.modal = None;
                    this.dialog_error = None;
                    cx.notify();
                }))
                .child(card),
        )
    }

    fn render_settings_card(
        &mut self,
        theme: &Theme,
        section: SettingsSection,
        cx: &mut Context<Self>,
    ) -> Div {
        let nav = div()
            .flex()
            .flex_col()
            .gap_1()
            .w(px(130.0))
            .flex_shrink_0()
            .p_2()
            .bg(theme.sidebar_bg)
            .border_r_1()
            .border_color(theme.border)
            .child(settings_nav_item(
                theme,
                cx,
                "General",
                "settings01",
                SettingsSection::General,
                section == SettingsSection::General,
            ))
            .child(settings_nav_item(
                theme,
                cx,
                "Appearance",
                "paint-board",
                SettingsSection::Appearance,
                section == SettingsSection::Appearance,
            ))
            .child(settings_nav_item(
                theme,
                cx,
                "Extensions",
                "puzzle",
                SettingsSection::Extensions,
                section == SettingsSection::Extensions,
            ))
            .child(settings_nav_item(
                theme,
                cx,
                "Shortcuts",
                "keyboard",
                SettingsSection::Shortcuts,
                section == SettingsSection::Shortcuts,
            ))
            .child(settings_nav_item(
                theme,
                cx,
                "About",
                "information-circle",
                SettingsSection::About,
                section == SettingsSection::About,
            ));

        let content = match section {
            SettingsSection::General => {
                let notes = self
                    .default_folder
                    .clone()
                    .filter(|dir| !dir.is_empty())
                    .unwrap_or_else(|| "No folder selected".to_string());
                let default_rows = self.default_app_rows(theme, cx);
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_4()
                    .child(settings_heading(theme, "General"))
                    .children(default_rows)
                    .child(settings_row(
                        theme,
                        "Autosave",
                        Some("Write tabs shortly after you stop typing."),
                        Some(
                            toggle_switch(theme, "autosave-toggle", self.autosave)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.autosave = !this.autosave;
                                    this.persist();
                                    cx.notify();
                                }))
                                .into_any_element(),
                        ),
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme.fg)
                                    .child("Notes folder"),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(theme.muted)
                                    .child("Where Cmd-N creates new notes."),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(app_icon::icon("folder-open", 16.0).text_color(theme.muted))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w(px(0.0))
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .text_ellipsis()
                                            .text_size(px(11.0))
                                            .text_color(theme.fg)
                                            .child(notes),
                                    )
                                    .child(
                                        secondary_button(theme, "notes-folder", "Change")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.pick_folder(cx, true)
                                            })),
                                    ),
                            ),
                    )
            }
            SettingsSection::Appearance => {
                let mut groups: Vec<(String, Vec<Stateful<Div>>)> = Vec::new();
                for option in theme::THEMES.iter() {
                    let selected = self.theme_name == option.id;
                    let id = option.id;
                    let label = option.label;
                    let group = option.group.to_string();
                    let row = div()
                        .id(SharedString::from(format!("theme-{}", id)))
                        .h(px(26.0))
                        .px_2()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .rounded(px(5.0))
                        .cursor_pointer()
                        .text_size(px(12.0))
                        .text_color(if selected { theme.accent } else { theme.fg })
                        .when(selected, |el| el.bg(theme.accent_soft))
                        .when(!selected, |el| {
                            el.hover(move |style| style.bg(theme.hover))
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.apply_theme(id, cx);
                        }))
                        .child(div().w(px(12.0)).child(if selected { "✓" } else { "" }))
                        .child(label);
                    match groups.last_mut() {
                        Some((name, rows)) if name == &group => rows.push(row),
                        _ => groups.push((group, vec![row])),
                    }
                }
                let mut body = div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_4()
                    .child(settings_heading(theme, "Appearance"));
                for (group, rows) in groups {
                    body = body
                        .child(
                            div()
                                .mt_2()
                                .text_size(px(10.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.muted)
                                .child(group.to_uppercase()),
                        )
                        .children(rows);
                }
                body
            }
            SettingsSection::Shortcuts => {
                let groups = SHORTCUT_GROUPS.iter().map(|(group, rows)| {
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_size(px(10.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.muted)
                                .child(group.to_uppercase()),
                        )
                        .children(rows.iter().map(|(label, keys)| {
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .gap_3()
                                .h(px(26.0))
                                .border_b_1()
                                .border_color(theme.border)
                                .text_size(px(12.0))
                                .child(div().text_color(theme.fg).child(*label))
                                .child(keycaps(theme, keys))
                        }))
                });
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .p_4()
                    .child(settings_heading(theme, "Shortcuts"))
                    .children(groups)
            }
            SettingsSection::Extensions => {
                let mut rows = div().flex().flex_col();
                for (id, toggle_id, name, description, applies) in EXTENSIONS {
                    let on = self.extension_enabled(id);
                    let toggle = toggle_switch(theme, toggle_id, on).on_click(cx.listener(
                        move |this, _, _, cx| this.toggle_extension(id, cx),
                    ));
                    rows = rows.child(extension_row(
                        theme,
                        name,
                        description,
                        applies,
                        toggle,
                        false,
                    ));
                }
                let mut unavailable = div().flex().flex_col();
                for (toggle_id, name, description) in UNAVAILABLE_EXTENSIONS {
                    let toggle = toggle_switch(theme, toggle_id, false).opacity(0.4);
                    unavailable = unavailable.child(extension_row(
                        theme,
                        name,
                        description,
                        "Needs the web build's JavaScript renderer, which this native build does not include.",
                        toggle,
                        true,
                    ));
                }
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_4()
                    .child(settings_heading(theme, "Extensions"))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(theme.muted)
                            .child("Built-in extensions. Turn one off to stop it handling its file types. Nothing is downloaded."),
                    )
                    .child(rows)
                    .child(
                        div()
                            .mt_3()
                            .text_size(px(10.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.muted)
                            .child("NOT AVAILABLE IN THE NATIVE APP YET"),
                    )
                    .child(unavailable)
            }
            SettingsSection::About => div()
                .flex()
                .flex_col()
                .gap_2()
                .p_4()
                .child(settings_heading(theme, "About"))
                .child(
                    div()
                        .text_size(px(20.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.fg)
                        .child("Zyplus"),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(theme.muted)
                        .child(format!("Version {VERSION} — native GPUI build")),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(theme.fg)
                        .child("A markdown editor that gets out of the way. Your notes stay plain files in a folder you choose."),
                )
                .child(self.update_panel(theme, cx)),
        };

        div()
            .flex()
            .flex_col()
            .w(px(620.0))
            .h(px(470.0))
            .rounded(px(12.0))
            .overflow_hidden()
            .bg(theme.surface)
            .border_1()
            .border_color(theme.border)
            .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                cx.stop_propagation()
            })
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .h(px(42.0))
                    .px_3()
                    .flex_shrink_0()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .text_size(px(13.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.fg)
                            .child("Settings"),
                    )
                    .child(
                        div()
                            .id("settings-close")
                            .w(px(24.0))
                            .h(px(24.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(5.0))
                            .cursor_pointer()
                            .text_color(theme.muted)
                            .hover(move |style| style.bg(theme.hover))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.modal = None;
                                cx.notify();
                            }))
                            .child(app_icon::icon("cancel01", 14.0).text_color(theme.muted)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(nav)
                    .child(
                        div()
                            .id("settings-content")
                            .flex_1()
                            .min_w(px(0.0))
                            .min_h(px(0.0))
                            .overflow_y_scroll()
                            .child(content),
                    ),
            )
    }

    /// The "Default … app" rows for Settings → General. The OS claims come from
    /// `default_app_claims`, read once per visit rather than every frame.
    fn default_app_rows(&mut self, theme: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let claims = self.default_app_claims();
        let mac = cfg!(target_os = "macos");
        let ids = ["default-app-md", "default-app-csv", "default-app-json"];
        DEFAULT_APPS
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let (label, _, mac_hint, other_hint) = *row;
                let claimed = claims.claimed[index];
                let button = secondary_button(
                    theme,
                    ids[index],
                    if claimed {
                        "Zyplus is the default"
                    } else {
                        "Make default"
                    },
                );
                // Only macOS can claim a file type; elsewhere the hint says where to go.
                let button = if !mac || claimed {
                    button.opacity(0.4)
                } else {
                    button.on_click(cx.listener(move |this, _, _, cx| {
                        this.make_default_app(index, cx)
                    }))
                };
                let mut control = div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(button)
                    .when(claimed, |el| {
                        el.child(app_icon::icon("checkmark-circle02", 16.0).text_color(theme.accent))
                    });
                if let Some(error) = claims.error[index].clone() {
                    control = control.child(
                        div()
                            .text_size(px(11.0))
                            .text_color(theme.danger)
                            .child(error),
                    );
                }
                settings_row(
                    theme,
                    label,
                    Some(if mac { mac_hint } else { other_hint }),
                    Some(control.into_any_element()),
                )
                .into_any_element()
            })
            .collect()
    }

    /// The update block in Settings → About: the check button and what it found.
    fn update_panel(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let status = &self.update_status;
        let check = secondary_button(theme, "check-updates", "Check for updates");
        let check = if status.busy() {
            check.opacity(0.4)
        } else {
            check.on_click(cx.listener(|this, _, _, cx| this.check_for_updates(false, cx)))
        };

        let (text, color) = match status {
            updater::Status::Idle => (String::new(), theme.muted),
            updater::Status::Checking => ("Checking for updates…".to_string(), theme.muted),
            updater::Status::UpToDate => ("You are up to date.".to_string(), theme.muted),
            updater::Status::Available(info) => (
                format!("Version {} is available.", info.version),
                theme.fg,
            ),
            updater::Status::Downloading { info, done, total } => (
                format!(
                    "Downloading {}… {}",
                    info.version,
                    updater::progress_text(*done, *total)
                ),
                theme.muted,
            ),
            updater::Status::Downloaded { info, path } => (
                format!(
                    "Version {} was downloaded to {}. Zyplus does not install updates itself; open the file to install it.",
                    info.version,
                    path.display()
                ),
                theme.fg,
            ),
            updater::Status::Failed { message, .. } => (message.clone(), theme.danger),
        };

        let action = match status {
            updater::Status::Available(_) | updater::Status::Failed { info: Some(_), .. } => Some(
                primary_button(theme, "update-download", "Download").on_click(cx.listener(
                    |this, _, _, cx| this.download_update(cx),
                )),
            ),
            updater::Status::Downloaded { .. } => Some(
                secondary_button(theme, "update-reveal", "Reveal").on_click(cx.listener(
                    |this, _, _, cx| this.reveal_update(cx),
                )),
            ),
            _ => None,
        };

        let mut panel = div()
            .flex()
            .flex_col()
            .items_start()
            .gap_2()
            .mt_3()
            .max_w(px(520.0))
            .child(check);
        if !text.is_empty() {
            panel = panel.child(div().text_size(px(11.0)).text_color(color).child(text));
        }
        if let Some(action) = action {
            panel = panel.child(action);
        }
        panel
    }

    fn render_error(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<Div> {
        // Errors win; a notice (export progress or result) uses the calm style.
        let (text, is_error) = match (&self.error, &self.notice) {
            (Some(error), _) => (error.clone(), true),
            (None, Some(notice)) => (notice.clone(), false),
            (None, None) => return None,
        };
        let color = if is_error { theme.danger } else { theme.muted };
        Some(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .px_3()
                .py_2()
                .flex_shrink_0()
                .bg(if is_error { theme.accent_soft } else { theme.surface })
                .border_t_1()
                .border_color(if is_error { theme.danger } else { theme.border })
                .text_size(px(11.0))
                .text_color(color)
                .children(is_error.then(|| {
                    app_icon::icon("alert01", 14.0)
                        .flex_shrink_0()
                        .text_color(color)
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(SharedString::from(text.to_string())),
                )
                .child(
                    div()
                        .id("dismiss-error")
                        .px_1()
                        .rounded(px(3.0))
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.error = None;
                            this.notice = None;
                            cx.notify();
                        }))
                        .child(app_icon::icon("cancel01", 12.0).text_color(color)),
                ),
        )
    }
}

fn settings_nav_item(
    theme: &Theme,
    cx: &mut Context<Zyplus>,
    label: &'static str,
    icon: &'static str,
    target: SettingsSection,
    active: bool,
) -> Stateful<Div> {
    div()
        .id(SharedString::from(format!("settings-nav-{label}")))
        .h(px(28.0))
        .px_2()
        .flex()
        .items_center()
        .gap_2()
        .rounded(px(5.0))
        .cursor_pointer()
        .text_size(px(12.0))
        .text_color(if active { theme.accent } else { theme.fg })
        .when(active, |el| el.bg(theme.accent_soft))
        .when(!active, |el| el.hover(move |style| style.bg(theme.hover)))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.modal = Some(Modal::Settings { section: target });
            cx.notify();
        }))
        .child(app_icon::icon(icon, 16.0).text_color(if active { theme.accent } else { theme.fg }))
        .child(label)
}

/// One row of Settings → Extensions: name, description and a note, with the toggle on the right.
fn extension_row(
    theme: &Theme,
    name: &'static str,
    description: &'static str,
    note: &'static str,
    toggle: Stateful<Div>,
    disabled: bool,
) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_3()
        .py_2()
        .border_b_1()
        .border_color(theme.border)
        .child(
            div()
                .flex()
                .flex_col()
                .gap_0p5()
                .flex_1()
                .min_w(px(0.0))
                .child(
                    div()
                        .text_size(px(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(if disabled { theme.muted } else { theme.fg })
                        .child(name),
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(theme.muted)
                        .child(description),
                )
                .child(
                    div()
                        .text_size(px(10.0))
                        .text_color(theme.muted)
                        .child(note),
                ),
        )
        .child(toggle)
}

/// Shortcut keys drawn as small keycaps: `⌘`, `⇧`, `S`.
fn keycaps(theme: &Theme, keys: &[&'static str]) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_1()
        .flex_shrink_0()
        .children(keys.iter().map(|key| {
            div()
                .h(px(18.0))
                .min_w(px(18.0))
                .px_1()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.0))
                .border_1()
                .border_color(theme.border)
                .text_size(px(10.0))
                .text_color(theme.muted)
                .child(*key)
        }))
}

fn settings_heading(theme: &Theme, label: &'static str) -> Div {
    div()
        .text_size(px(13.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.fg)
        .child(label)
}

fn settings_row(
    theme: &Theme,
    label: &'static str,
    hint: Option<&'static str>,
    control: Option<AnyElement>,
) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.0))
                .gap_0p5()
                .child(
                    div()
                        .text_size(px(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.fg)
                        .child(label),
                )
                .children(hint.map(|hint| {
                    div()
                        .text_size(px(11.0))
                        .text_color(theme.muted)
                        .child(hint)
                })),
        )
        .children(control)
}

fn toggle_switch(theme: &Theme, id: &'static str, on: bool) -> Stateful<Div> {
    div()
        .id(id)
        .w(px(34.0))
        .h(px(20.0))
        .p(px(2.0))
        .flex()
        .flex_row()
        .items_center()
        .rounded(px(10.0))
        .cursor_pointer()
        .bg(if on { theme.accent } else { theme.border })
        .child(
            div()
                .w(px(16.0))
                .h(px(16.0))
                .rounded(px(8.0))
                .bg(rgb(0xffffff))
                .when(on, |el| el.ml(px(14.0))),
        )
}

fn mode_button(
    theme: &Theme,
    id: &'static str,
    icon: &'static str,
    label: &'static str,
    selected: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .px_2()
        .h(px(22.0))
        .flex()
        .items_center()
        .gap_1()
        .rounded(px(5.0))
        .text_size(px(11.0))
        .cursor_pointer()
        .text_color(if selected { theme.accent } else { theme.muted })
        .when(selected, |el| el.bg(theme.accent_soft))
        .when(!selected, |el| el.hover(move |style| style.bg(theme.hover)))
        .child(app_icon::icon(icon, 15.0).text_color(if selected { theme.accent } else { theme.muted }))
        .child(label)
}

/// The Hugeicon on a view-mode button: rich text, plain text, table, tree.
fn mode_icon(mode: &str) -> &'static str {
    match mode {
        "rich" => "text-font",
        "table" => "table01",
        "tree" => "list-tree",
        _ => "source-code",
    }
}

/// A square icon-only control (tab bar, sidebar header, find bar). The glyph
/// carries its own colour, so callers pick it per state.
fn icon_button(theme: &Theme, id: &'static str, glyph: gpui::Svg, size: f32) -> Stateful<Div> {
    let hover = theme.hover;
    div()
        .id(id)
        .w(px(size))
        .h(px(size))
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .rounded(px(5.0))
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(glyph)
}

/// A folder row's hover button ("New file here", "New folder here"). It stops
/// its mouse-down and click here, so the row is not selected or toggled.
fn tree_action(
    theme: &Theme,
    id_prefix: &'static str,
    dir: String,
    icon: &'static str,
    action: MenuAction,
    cx: &Context<Zyplus>,
) -> Stateful<Div> {
    let hover = theme.hover;
    div()
        .id(SharedString::from(format!("{id_prefix}-{dir}")))
        .w(px(24.0))
        .h(px(24.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.0))
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .on_mouse_down(MouseButton::Left, |_event, _window, cx| cx.stop_propagation())
        .on_click(cx.listener(move |this, _, window, cx| {
            cx.stop_propagation();
            this.run_menu(action.clone(), window, cx);
        }))
        .child(app_icon::icon(icon, 14.0).text_color(theme.muted))
}

/// A Hugeicon followed by a label, for buttons. `color` is the button's text
/// colour: an SVG only paints with its own colour, so it can't inherit it.
fn icon_label(icon: &'static str, size: f32, color: Rgba, label: &'static str) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .child(app_icon::icon(icon, size).text_color(color))
        .child(label)
}

/// The round badge at the start of a dialog title, like the web's `Modal.Icon`.
fn dialog_badge(icon: &'static str, size: f32, glyph: f32, color: Rgba, background: Rgba) -> Div {
    div()
        .w(px(size))
        .h(px(size))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(size / 2.0))
        .bg(background)
        .text_color(color)
        .child(app_icon::icon(icon, glyph).text_color(color))
}

/// A translucent version of a colour, for badge backgrounds.
fn soft(color: Rgba) -> Rgba {
    Rgba { a: 0.14, ..color }
}

/// The element id of a view-mode button in the tab bar.
fn mode_button_id(mode: &str) -> &'static str {
    match mode {
        "rich" => "mode-rich",
        "plain" => "mode-plain",
        "table" => "mode-table",
        "text" => "mode-text",
        "tree" => "mode-tree",
        _ => "mode-other",
    }
}

fn toolbar_button(theme: &Theme, id: &'static str, label: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(26.0))
        .px_2()
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .rounded(px(5.0))
        .cursor_pointer()
        .text_size(px(11.0))
        .text_color(theme.fg)
        .hover(move |style| style.bg(theme.hover))
        .child(label)
}

fn primary_button(theme: &Theme, id: &'static str, label: impl IntoElement) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(30.0))
        .px_3()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .cursor_pointer()
        .bg(theme.accent)
        .text_color(rgb(0xffffff))
        .text_size(px(12.0))
        .font_weight(FontWeight::MEDIUM)
        .hover(move |style| style.opacity(0.9))
        .child(label)
}

fn secondary_button(theme: &Theme, id: &'static str, label: impl IntoElement) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(30.0))
        .px_3()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .cursor_pointer()
        .bg(theme.surface)
        .border_1()
        .border_color(theme.border)
        .text_color(theme.fg)
        .text_size(px(12.0))
        .hover(move |style| style.bg(theme.hover))
        .child(label)
}

fn danger_button(theme: &Theme, id: &'static str, label: impl IntoElement) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(30.0))
        .px_3()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .cursor_pointer()
        .bg(theme.danger)
        .text_color(rgb(0xffffff))
        .text_size(px(12.0))
        .font_weight(FontWeight::MEDIUM)
        .hover(move |style| style.opacity(0.9))
        .child(label)
}

impl Focusable for Zyplus {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Zyplus {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_shell(window, cx)
    }
}

impl Zyplus {
    fn render_shell(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if std::mem::take(&mut self.refocus_editor) {
            self.focus_editor(window, cx);
        }
        let theme = self.theme(window);

        // Drawn over the empty shell instead of replacing it, so the global
        // shortcuts stay bound while the welcome screen is up.
        let welcome = if self.welcome {
            Some(
                self.render_welcome(&theme, cx)
                    .absolute()
                    .left(px(0.0))
                    .top(px(0.0))
                    .into_any_element(),
            )
        } else {
            None
        };

        if self.find.as_ref().map(|find| find.dirty).unwrap_or(false) {
            self.refresh_find(cx);
        }

        if !self.focus_placed {
            self.focus_placed = true;
            self.focus_editor(window, cx);
        }

        let title = match self.active_tab() {
            Some(tab) => format!(
                "{}{} — Zyplus",
                if tab.dirty { "• " } else { "" },
                tab.title
            ),
            None => "Zyplus".to_string(),
        };
        if window.window_title().to_string() != title {
            window.set_window_title(&title);
        }

        let sidebar = if self.sidebar_collapsed {
            None
        } else {
            Some(self.render_sidebar(&theme, cx).into_any_element())
        };
        let tab_bar = self.render_tab_bar(&theme, cx);
        let body = self.render_body(&theme, cx);
        let error_banner = self.render_error(&theme, cx);
        let menu = self.render_menu(&theme, cx);
        let modal = self.render_modal(&theme, cx);

        div()
            .key_context("Zyplus")
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.bg)
            .text_color(theme.fg)
            .text_size(px(13.0))
            .on_action(cx.listener(Self::save_action))
            .on_action(cx.listener(Self::export_md_action))
            .on_action(cx.listener(Self::export_pdf_action))
            .on_action(cx.listener(Self::new_file_action))
            .on_action(cx.listener(Self::new_folder_action))
            .on_action(cx.listener(Self::close_tab_action))
            .on_action(cx.listener(Self::delete_selection_action))
            .on_action(cx.listener(Self::close_overlay_action))
            .on_action(cx.listener(Self::toggle_sidebar_action))
            .on_action(cx.listener(Self::toggle_mode_action))
            .on_action(cx.listener(Self::open_file_action))
            .on_action(cx.listener(Self::open_folder_action))
            .on_action(cx.listener(Self::find_action))
            .on_action(cx.listener(Self::replace_action))
            .on_action(cx.listener(Self::settings_action))
            .on_action(cx.listener(Self::shortcuts_action))
            .on_action(cx.listener(Self::copy_markdown_action))
            .on_action(cx.listener(Self::copy_path_action))
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::prev_tab))
            .on_action(cx.listener(Self::go_to_tab_1))
            .on_action(cx.listener(Self::go_to_tab_2))
            .on_action(cx.listener(Self::go_to_tab_3))
            .on_action(cx.listener(Self::go_to_tab_4))
            .on_action(cx.listener(Self::go_to_tab_5))
            .on_action(cx.listener(Self::go_to_tab_6))
            .on_action(cx.listener(Self::go_to_tab_7))
            .on_action(cx.listener(Self::go_to_tab_8))
            .on_action(cx.listener(Self::go_to_tab_9))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(px(0.0))
                    .children(sidebar)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w(px(0.0))
                            .min_h(px(0.0))
                            .child(tab_bar)
                            .child(body),
                    ),
            )
            .children(error_banner)
            .children(menu)
            .children(modal)
            .children(welcome)
            .into_any_element()
    }
}

// The nine tab actions share a body; the macro keeps them from drifting apart.
macro_rules! go_to_tab_actions {
    ($($method:ident, $action:ident => $number:expr);* $(;)?) => {
        impl Zyplus {
            $(
                fn $method(&mut self, _: &$action, window: &mut Window, cx: &mut Context<Self>) {
                    self.goto_tab($number, window, cx);
                }
            )*
        }
    };
}

go_to_tab_actions!(
    go_to_tab_1, GoToTab1 => 1;
    go_to_tab_2, GoToTab2 => 2;
    go_to_tab_3, GoToTab3 => 3;
    go_to_tab_4, GoToTab4 => 4;
    go_to_tab_5, GoToTab5 => 5;
    go_to_tab_6, GoToTab6 => 6;
    go_to_tab_7, GoToTab7 => 7;
    go_to_tab_8, GoToTab8 => 8;
    go_to_tab_9, GoToTab9 => 9;
);

// ---- OS integration: external open requests, quit, menus -----------------
//
// Files reach the app three ways: the command line (`os::cli_file_args`), a
// second launch forwarded over the single-instance socket, and macOS "Open
// With" / double-click (`App::on_open_urls`). The first is opened when the
// window is built. The other two are queued and drained by a poll loop in
// `main`, which hands each batch to `open_external` on the main thread.

actions!(zyplus_os, [Quit, HideApp, HideOthers, Minimize, Zoom]);

/// Local paths from one open request, sorted by what they are on disk.
#[derive(Debug, Default, PartialEq)]
struct OpenPlan {
    folders: Vec<String>,
    files: Vec<String>,
    missing: Vec<String>,
}

fn plan_open(paths: Vec<String>) -> OpenPlan {
    let mut plan = OpenPlan::default();
    for path in paths {
        let on_disk = Path::new(&path);
        if on_disk.is_dir() {
            plan.folders.push(path);
        } else if on_disk.is_file() {
            plan.files.push(path);
        } else {
            plan.missing.push(path);
        }
    }
    plan
}

/// The main window, when one is open.
fn zyplus_window(cx: &App) -> Option<gpui::WindowHandle<Zyplus>> {
    cx.windows()
        .into_iter()
        .find_map(|handle| handle.downcast::<Zyplus>())
}

fn main_window_options(cx: &App) -> WindowOptions {
    let bounds = Bounds::centered(None, size(px(1180.0), px(800.0)), cx);
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        // No system titlebar: the app draws its own chrome and the
        // traffic lights float over the sidebar / tab bar.
        titlebar: Some(TitlebarOptions {
            title: Some("Zyplus".into()),
            appears_transparent: true,
            traffic_light_position: Some(gpui::point(px(16.0), px(15.0))),
        }),
        window_min_size: Some(size(px(560.0), px(360.0))),
        ..Default::default()
    }
}

/// Opens the main window. `paths` open as tabs, which also skips the welcome
/// screen. Closing the window with unsaved edits asks first.
fn open_main_window(paths: Vec<String>, cx: &mut App) -> Option<gpui::WindowHandle<Zyplus>> {
    cx.open_window(main_window_options(cx), |window, cx| {
        let view = cx.new(Zyplus::new);
        if !paths.is_empty() {
            view.update(cx, |this, cx| this.open_paths(paths, window, cx));
        }
        let guard = view.downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            guard
                .update(cx, |this, cx| this.confirm_quit(window, cx))
                .unwrap_or(true)
        });
        view
    })
    .ok()
}

/// Opens paths in the main window, or a new one if it was closed. An empty
/// list only brings the window forward.
fn open_external(paths: Vec<String>, cx: &mut App) {
    match zyplus_window(cx) {
        Some(handle) => {
            let _ = handle.update(cx, |this, window, cx| {
                this.open_paths(paths, window, cx);
                window.activate_window();
            });
        }
        None => {
            if open_main_window(paths, cx).is_none() {
                eprintln!("zyplus: could not open a window");
            }
        }
    }
    cx.activate(true);
}

/// Runs an action on the active window, if there is one.
fn with_active_window(cx: &mut App, f: impl FnOnce(&mut Window)) {
    if let Some(handle) = cx.active_window() {
        let _ = handle.update(cx, |_, window, _| f(window));
    }
}

/// Cmd-Q and the Quit menu item. Quits once no window has unsaved edits left
/// to decide about; otherwise the first dirty tab gets the Save-changes prompt.
fn request_quit(cx: &mut App) {
    let handles: Vec<gpui::WindowHandle<Zyplus>> = cx
        .windows()
        .into_iter()
        .filter_map(|handle| handle.downcast::<Zyplus>())
        .collect();
    for handle in handles {
        let may_quit = handle
            .update(cx, |this, window, cx| this.confirm_quit(window, cx))
            .unwrap_or(true);
        if !may_quit {
            return;
        }
    }
    cx.quit();
}

/// The menu bar. Items dispatch their action to the focused window, so the
/// Zyplus handlers and the editor's own actions both see them.
fn app_menus() -> Vec<gpui::Menu> {
    vec![
        gpui::Menu {
            name: "Zyplus".into(),
            items: vec![
                // The About section lives in Settings, so both items open it.
                gpui::MenuItem::action("About Zyplus", Settings),
                gpui::MenuItem::separator(),
                gpui::MenuItem::action("Settings…", Settings),
                gpui::MenuItem::separator(),
                gpui::MenuItem::action("Hide Zyplus", HideApp),
                gpui::MenuItem::action("Hide Others", HideOthers),
                gpui::MenuItem::separator(),
                gpui::MenuItem::action("Quit Zyplus", Quit),
            ],
        },
        gpui::Menu {
            name: "File".into(),
            items: vec![
                gpui::MenuItem::action("New File", NewFile),
                gpui::MenuItem::action("New Folder", NewFolder),
                gpui::MenuItem::separator(),
                gpui::MenuItem::action("Open File…", OpenFile),
                gpui::MenuItem::action("Open Folder…", OpenFolder),
                gpui::MenuItem::separator(),
                gpui::MenuItem::action("Save", Save),
                gpui::MenuItem::action("Export as .md", ExportMd),
                gpui::MenuItem::separator(),
                gpui::MenuItem::action("Close Tab", CloseTab),
            ],
        },
        gpui::Menu {
            name: "Edit".into(),
            items: vec![
                gpui::MenuItem::action("Undo", editor::Undo),
                gpui::MenuItem::action("Redo", editor::Redo),
                gpui::MenuItem::separator(),
                gpui::MenuItem::action("Cut", editor::Cut),
                gpui::MenuItem::action("Copy", editor::Copy),
                gpui::MenuItem::action("Paste", editor::Paste),
                gpui::MenuItem::separator(),
                gpui::MenuItem::action("Select All", editor::SelectAll),
                gpui::MenuItem::separator(),
                gpui::MenuItem::action("Find", Find),
                gpui::MenuItem::action("Replace", Replace),
            ],
        },
        gpui::Menu {
            name: "View".into(),
            items: vec![
                gpui::MenuItem::action("Toggle Sidebar", ToggleSidebar),
                gpui::MenuItem::action("Toggle Rich / Plain", ToggleMode),
            ],
        },
        gpui::Menu {
            name: "Window".into(),
            items: vec![
                gpui::MenuItem::action("Minimize", Minimize),
                gpui::MenuItem::action("Zoom", Zoom),
            ],
        },
    ]
}

impl Zyplus {
    /// Opens files and folders the OS or the command line asked for. A file
    /// outside every open folder still opens as a tab, as Open File does.
    fn open_paths(&mut self, paths: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        if paths.is_empty() {
            return;
        }
        self.welcome = false;
        let plan = plan_open(paths);
        for dir in plan.folders {
            self.add_folder(dir, cx);
        }
        for file in plan.files {
            self.open_document(&file, Some(window), cx);
        }
        if let Some(first) = plan.missing.first() {
            let message = if plan.missing.len() == 1 {
                format!("Could not find {}.", paths::basename_of(first))
            } else {
                format!("Could not find {} items.", plan.missing.len())
            };
            self.error = Some(message.into());
            cx.notify();
        }
    }

    /// Whether the app may close now. Edits that autosave (or are untitled)
    /// are written first. Any other dirty tab gets the Save-changes prompt
    /// and the caller says no; the user repeats the close after deciding.
    fn confirm_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.autosave_dirty(cx);
        let Some(path) = self
            .tabs
            .iter()
            .find(|tab| tab.dirty)
            .map(|tab| tab.path.clone())
        else {
            return true;
        };
        window.activate_window();
        self.dialog_error = None;
        self.modal = Some(Modal::Unsaved { path });
        cx.notify();
        false
    }
}

fn main() {
    // A second launch hands its files to the running instance and exits here,
    // before GPUI starts.
    let primary = match os::acquire_single_instance() {
        os::SingleInstance::Primary(primary) => primary,
        os::SingleInstance::Forwarded => std::process::exit(0),
    };
    let startup_files = os::cli_file_args();

    // Double-click and "Open With" on macOS. These callbacks run before
    // `run` starts and have no App, so URLs are queued for the poll loop.
    let (url_tx, url_rx) = std::sync::mpsc::channel::<Vec<String>>();
    let app = Application::new().with_assets(app_icon::Assets);
    app.on_open_urls(move |urls| {
        let _ = url_tx.send(os::urls_to_paths(&urls));
    });
    // Clicking the Dock icon with no window open brings one back.
    app.on_reopen(|cx| {
        if zyplus_window(cx).is_none() {
            open_main_window(Vec::new(), cx);
        }
        cx.activate(true);
    });

    app.run(move |cx: &mut App| {
        app_icon::set_dock_icon();
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-h", HideApp, None),
            KeyBinding::new("cmd-alt-h", HideOthers, None),
            KeyBinding::new("cmd-m", Minimize, None),
            KeyBinding::new("cmd-s", Save, Some("Zyplus")),
            KeyBinding::new("cmd-shift-s", ExportMd, Some("Zyplus")),
            KeyBinding::new("cmd-p", ExportPdf, Some("Zyplus")),
            KeyBinding::new("cmd-n", NewFile, Some("Zyplus")),
            KeyBinding::new("cmd-shift-n", NewFolder, Some("Zyplus")),
            KeyBinding::new("cmd-w", CloseTab, Some("Zyplus")),
            KeyBinding::new("cmd-b", ToggleSidebar, Some("Zyplus")),
            KeyBinding::new("cmd-e", ToggleMode, Some("Zyplus")),
            KeyBinding::new("cmd-o", OpenFolder, Some("Zyplus")),
            KeyBinding::new("cmd-shift-o", OpenFile, Some("Zyplus")),
            KeyBinding::new("cmd-f", Find, Some("Zyplus")),
            KeyBinding::new("cmd-alt-f", Replace, Some("Zyplus")),
            KeyBinding::new("cmd-,", Settings, Some("Zyplus")),
            KeyBinding::new("cmd-/", Shortcuts, Some("Zyplus")),
            KeyBinding::new("cmd-shift-c", CopyMarkdown, Some("Zyplus")),
            KeyBinding::new("cmd-alt-c", CopyPath, Some("Zyplus")),
            KeyBinding::new("cmd-alt-right", NextTab, Some("Zyplus")),
            KeyBinding::new("cmd-alt-left", PrevTab, Some("Zyplus")),
            KeyBinding::new("cmd-shift-]", NextTab, Some("Zyplus")),
            KeyBinding::new("cmd-shift-[", PrevTab, Some("Zyplus")),
            KeyBinding::new("cmd-1", GoToTab1, Some("Zyplus")),
            KeyBinding::new("cmd-2", GoToTab2, Some("Zyplus")),
            KeyBinding::new("cmd-3", GoToTab3, Some("Zyplus")),
            KeyBinding::new("cmd-4", GoToTab4, Some("Zyplus")),
            KeyBinding::new("cmd-5", GoToTab5, Some("Zyplus")),
            KeyBinding::new("cmd-6", GoToTab6, Some("Zyplus")),
            KeyBinding::new("cmd-7", GoToTab7, Some("Zyplus")),
            KeyBinding::new("cmd-8", GoToTab8, Some("Zyplus")),
            KeyBinding::new("cmd-9", GoToTab9, Some("Zyplus")),
            KeyBinding::new("delete", DeleteSelection, Some("Zyplus")),
            KeyBinding::new("escape", CloseOverlay, Some("Zyplus")),

            // The editor draws its own text, so it needs its own key map; GPUI
            // only turns keystrokes into actions. Without these, typing worked
            // but deleting, moving the caret, undo and the clipboard did not.
            KeyBinding::new("backspace", editor::Backspace, Some("Editor")),
            KeyBinding::new("delete", editor::Delete, Some("Editor")),
            KeyBinding::new("enter", editor::Enter, Some("Editor")),
            KeyBinding::new("left", editor::Left, Some("Editor")),
            KeyBinding::new("right", editor::Right, Some("Editor")),
            KeyBinding::new("up", editor::Up, Some("Editor")),
            KeyBinding::new("down", editor::Down, Some("Editor")),
            KeyBinding::new("home", editor::Home, Some("Editor")),
            KeyBinding::new("end", editor::End, Some("Editor")),
            KeyBinding::new("shift-left", editor::SelectLeft, Some("Editor")),
            KeyBinding::new("shift-right", editor::SelectRight, Some("Editor")),
            KeyBinding::new("shift-up", editor::SelectUp, Some("Editor")),
            KeyBinding::new("shift-down", editor::SelectDown, Some("Editor")),
            KeyBinding::new("shift-home", editor::SelectHome, Some("Editor")),
            KeyBinding::new("shift-end", editor::SelectEnd, Some("Editor")),
            KeyBinding::new("alt-left", editor::WordLeft, Some("Editor")),
            KeyBinding::new("alt-right", editor::WordRight, Some("Editor")),
            KeyBinding::new("shift-alt-left", editor::SelectWordLeft, Some("Editor")),
            KeyBinding::new("shift-alt-right", editor::SelectWordRight, Some("Editor")),
            KeyBinding::new("cmd-a", editor::SelectAll, Some("Editor")),
            KeyBinding::new("cmd-c", editor::Copy, Some("Editor")),
            KeyBinding::new("cmd-x", editor::Cut, Some("Editor")),
            KeyBinding::new("cmd-v", editor::Paste, Some("Editor")),
            KeyBinding::new("cmd-z", editor::Undo, Some("Editor")),
            KeyBinding::new("cmd-shift-z", editor::Redo, Some("Editor")),
            KeyBinding::new("escape", editor::Cancel, Some("Editor")),
        ]);
        RichEditor::bind_keys(cx);

        cx.set_menus(app_menus());
        cx.on_action(|_: &Quit, cx| request_quit(cx));
        cx.on_action(|_: &HideApp, cx| cx.hide());
        cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
        cx.on_action(|_: &Minimize, cx| with_active_window(cx, |window| window.minimize_window()));
        cx.on_action(|_: &Zoom, cx| with_active_window(cx, |window| window.zoom_window()));

        // Other platforms quit with their last window. macOS keeps the app
        // running, as its apps do.
        cx.on_window_closed(|cx| {
            if cfg!(not(target_os = "macos")) && cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        #[cfg_attr(not(debug_assertions), allow(unused_variables))]
        let Some(main_window) = open_main_window(startup_files, cx) else {
            eprintln!("zyplus: could not open the main window");
            cx.quit();
            return;
        };
        #[cfg(debug_assertions)]
        test_script::spawn(main_window, cx);

        // Drains files from later launches and from the OS. `primary` is moved
        // in so the single-instance socket lives as long as this task does.
        cx.spawn(async move |cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;
            let mut batches: Vec<Vec<String>> = Vec::new();
            while let Ok(batch) = primary.requests.try_recv() {
                batches.push(batch);
            }
            while let Ok(batch) = url_rx.try_recv() {
                batches.push(batch);
            }
            for batch in batches {
                if cx.update(|cx| open_external(batch, cx)).is_err() {
                    return;
                }
            }
        })
        .detach();

        cx.activate(true);
    });
}

#[cfg(test)]
mod open_request_tests {
    use super::{plan_open, OpenPlan};
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("zy-open-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn sorts_request_paths_into_folders_files_and_missing() {
        let dir = scratch("plan");
        let file = dir.join("a.md");
        std::fs::write(&file, "# a").unwrap();
        let sub = dir.join("notes");
        std::fs::create_dir_all(&sub).unwrap();
        let gone = dir.join("gone.md");

        let f = file.to_string_lossy().into_owned();
        let d = sub.to_string_lossy().into_owned();
        let m = gone.to_string_lossy().into_owned();
        let plan = plan_open(vec![f.clone(), d.clone(), m.clone()]);
        assert_eq!(
            plan,
            OpenPlan {
                folders: vec![d],
                files: vec![f],
                missing: vec![m],
            }
        );
        assert_eq!(plan_open(Vec::new()), OpenPlan::default());

        std::fs::remove_dir_all(&dir).ok();
    }
}
