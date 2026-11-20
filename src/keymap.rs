//! Keybinding tables and shortcut display labels per platform.
//!
//! Every builder takes an [`Os`] value instead of reading `cfg!(target_os)`, so
//! the macOS, Windows and Linux tables all compile and are unit-tested on any
//! host. Call sites pass [`Os::current`].

use gpui::KeyBinding;

use crate::platform::Os;
use crate::{
    editor, rich_editor, CloseOverlay, CloseTab, CopyMarkdown, CopyPath, DeleteSelection, ExportMd,
    ExportPdf, Find, FindNext, FindPrev, GoToTab1, GoToTab2, GoToTab3, GoToTab4, GoToTab5,
    GoToTab6, GoToTab7, GoToTab8, GoToTab9, HideApp, HideOthers, Minimize, NewFile, NewFolder,
    NextTab, OpenFile, OpenFolder, PrevTab, Quit, Replace, Save, Settings, Shortcuts, ToggleMode,
    ToggleSidebar, TreeActivate, TreeDown, TreeLeft, TreeRename, TreeRight, TreeUp,
};

/// Prefixes `rest` with the platform's primary shortcut modifier: `cmd-` on
/// macOS, `ctrl-` on Windows and Linux.
///
/// We emit explicit `cmd-` / `ctrl-` rather than gpui's `secondary-` because
/// gpui resolves `secondary-` via `cfg!(target_os = "macos")` at parse time;
/// explicit prefixes produce identical `Keystroke`s at runtime on every OS and
/// let unit tests on macOS exercise real Windows/Linux `ctrl-` keystrokes.
fn primary(os: Os, rest: &str) -> String {
    match os {
        Os::Mac => format!("cmd-{rest}"),
        Os::Windows | Os::Linux => format!("ctrl-{rest}"),
    }
}

/// Global (`None`), shell (`Zyplus`), and sidebar tree (`ZyplusTree`) bindings.
pub fn app_bindings(os: Os) -> Vec<KeyBinding> {
    let mut out = Vec::with_capacity(48);

    // Global (no context): on macOS includes standard app-lifecycle shortcuts;
    // off macOS only Quit is bound (Ctrl+H / Ctrl+M collide with common editing
    // keys, and Hide/Minimize menu items exist on macOS only; Windows also
    // leaves Alt+F4 to the OS).
    out.push(KeyBinding::new(&primary(os, "q"), Quit, None));
    if os == Os::Mac {
        out.push(KeyBinding::new("cmd-h", HideApp, None));
        out.push(KeyBinding::new("cmd-alt-h", HideOthers, None));
        out.push(KeyBinding::new("cmd-m", Minimize, None));
    }

    let z = Some("Zyplus");
    out.extend([
        KeyBinding::new(&primary(os, "s"), Save, z),
        KeyBinding::new(&primary(os, "shift-s"), ExportMd, z),
        KeyBinding::new(&primary(os, "p"), ExportPdf, z),
        KeyBinding::new(&primary(os, "n"), NewFile, z),
        KeyBinding::new(&primary(os, "shift-n"), NewFolder, z),
        KeyBinding::new(&primary(os, "w"), CloseTab, z),
        KeyBinding::new(&primary(os, "b"), ToggleSidebar, z),
        KeyBinding::new(&primary(os, "e"), ToggleMode, z),
        KeyBinding::new(&primary(os, "o"), OpenFolder, z),
        KeyBinding::new(&primary(os, "shift-o"), OpenFile, z),
        KeyBinding::new(&primary(os, "f"), Find, z),
        KeyBinding::new(&primary(os, "alt-f"), Replace, z),
        KeyBinding::new(&primary(os, ","), Settings, z),
        KeyBinding::new(&primary(os, "/"), Shortcuts, z),
        KeyBinding::new(&primary(os, "shift-c"), CopyMarkdown, z),
        KeyBinding::new(&primary(os, "alt-c"), CopyPath, z),
    ]);

    if os == Os::Mac {
        out.extend([
            KeyBinding::new("cmd-alt-right", NextTab, z),
            KeyBinding::new("cmd-alt-left", PrevTab, z),
        ]);
    } else {
        out.extend([
            KeyBinding::new("ctrl-tab", NextTab, z),
            KeyBinding::new("ctrl-shift-tab", PrevTab, z),
            KeyBinding::new("ctrl-pagedown", NextTab, z),
            KeyBinding::new("ctrl-pageup", PrevTab, z),
        ]);
    }

    out.extend([
        KeyBinding::new(&primary(os, "shift-]"), NextTab, z),
        KeyBinding::new(&primary(os, "shift-["), PrevTab, z),
        KeyBinding::new(&primary(os, "1"), GoToTab1, z),
        KeyBinding::new(&primary(os, "2"), GoToTab2, z),
        KeyBinding::new(&primary(os, "3"), GoToTab3, z),
        KeyBinding::new(&primary(os, "4"), GoToTab4, z),
        KeyBinding::new(&primary(os, "5"), GoToTab5, z),
        KeyBinding::new(&primary(os, "6"), GoToTab6, z),
        KeyBinding::new(&primary(os, "7"), GoToTab7, z),
        KeyBinding::new(&primary(os, "8"), GoToTab8, z),
        KeyBinding::new(&primary(os, "9"), GoToTab9, z),
    ]);

    let tree = Some("ZyplusTree");
    out.extend([
        KeyBinding::new("delete", DeleteSelection, tree),
        KeyBinding::new("backspace", DeleteSelection, tree),
        KeyBinding::new("up", TreeUp, tree),
        KeyBinding::new("down", TreeDown, tree),
        KeyBinding::new("left", TreeLeft, tree),
        KeyBinding::new("right", TreeRight, tree),
        KeyBinding::new("enter", TreeActivate, tree),
        KeyBinding::new("f2", TreeRename, tree),
    ]);

    out.extend([
        KeyBinding::new(&primary(os, "g"), FindNext, z),
        KeyBinding::new(&primary(os, "shift-g"), FindPrev, z),
        KeyBinding::new("shift-enter", FindPrev, z),
        KeyBinding::new("escape", CloseOverlay, z),
    ]);

    out
}

/// Core text-editing bindings shared by `Editor` and `RichEditor`.
fn text_editing_bindings(os: Os, context: &'static str, escape_early: bool) -> Vec<KeyBinding> {
    let c = Some(context);
    let mut out = Vec::with_capacity(28);
    out.extend([
        KeyBinding::new("backspace", editor::Backspace, c),
        KeyBinding::new("delete", editor::Delete, c),
        KeyBinding::new("enter", editor::Enter, c),
    ]);
    if escape_early {
        out.push(KeyBinding::new("escape", editor::Cancel, c));
    }
    out.extend([
        KeyBinding::new("left", editor::Left, c),
        KeyBinding::new("right", editor::Right, c),
        KeyBinding::new("up", editor::Up, c),
        KeyBinding::new("down", editor::Down, c),
        KeyBinding::new("home", editor::Home, c),
        KeyBinding::new("end", editor::End, c),
        KeyBinding::new("shift-left", editor::SelectLeft, c),
        KeyBinding::new("shift-right", editor::SelectRight, c),
        KeyBinding::new("shift-up", editor::SelectUp, c),
        KeyBinding::new("shift-down", editor::SelectDown, c),
        KeyBinding::new("shift-home", editor::SelectHome, c),
        KeyBinding::new("shift-end", editor::SelectEnd, c),
    ]);
    if os == Os::Mac {
        out.extend([
            KeyBinding::new("alt-left", editor::WordLeft, c),
            KeyBinding::new("alt-right", editor::WordRight, c),
            KeyBinding::new("shift-alt-left", editor::SelectWordLeft, c),
            KeyBinding::new("shift-alt-right", editor::SelectWordRight, c),
        ]);
    } else {
        out.extend([
            KeyBinding::new("ctrl-left", editor::WordLeft, c),
            KeyBinding::new("ctrl-right", editor::WordRight, c),
            KeyBinding::new("shift-ctrl-left", editor::SelectWordLeft, c),
            KeyBinding::new("shift-ctrl-right", editor::SelectWordRight, c),
        ]);
    }
    out.extend([
        KeyBinding::new(&primary(os, "a"), editor::SelectAll, c),
        KeyBinding::new(&primary(os, "c"), editor::Copy, c),
        KeyBinding::new(&primary(os, "x"), editor::Cut, c),
        KeyBinding::new(&primary(os, "v"), editor::Paste, c),
        KeyBinding::new(&primary(os, "z"), editor::Undo, c),
        KeyBinding::new(&primary(os, "shift-z"), editor::Redo, c),
    ]);
    if os != Os::Mac {
        out.push(KeyBinding::new("ctrl-y", editor::Redo, c));
    }
    if !escape_early {
        out.push(KeyBinding::new("escape", editor::Cancel, c));
    }
    out
}

/// Plain / code / single-line editor bindings under the `Editor` context.
pub fn editor_bindings(os: Os) -> Vec<KeyBinding> {
    text_editing_bindings(os, "Editor", false)
}

/// Navigation bindings shared by `Editor` and `RichEditor`:
/// - macOS: Cmd-Up/Down for doc start/end, Cmd-Left/Right for visual row start/end.
/// - Windows/Linux: Ctrl-Home/End for doc start/end (plain Home/End already move
///   to visual row start/end).
pub fn editor_nav_bindings(os: Os) -> Vec<KeyBinding> {
    let mut out = Vec::with_capacity(16);
    for context in ["Editor", rich_editor::KEY_CONTEXT] {
        let c = Some(context);
        if os == Os::Mac {
            out.extend([
                KeyBinding::new("cmd-up", editor::DocStart, c),
                KeyBinding::new("cmd-down", editor::DocEnd, c),
                KeyBinding::new("shift-cmd-up", editor::SelectDocStart, c),
                KeyBinding::new("shift-cmd-down", editor::SelectDocEnd, c),
                KeyBinding::new("cmd-left", editor::Home, c),
                KeyBinding::new("cmd-right", editor::End, c),
                KeyBinding::new("shift-cmd-left", editor::SelectHome, c),
                KeyBinding::new("shift-cmd-right", editor::SelectEnd, c),
            ]);
        } else {
            out.extend([
                KeyBinding::new("ctrl-home", editor::DocStart, c),
                KeyBinding::new("ctrl-end", editor::DocEnd, c),
                KeyBinding::new("shift-ctrl-home", editor::SelectDocStart, c),
                KeyBinding::new("shift-ctrl-end", editor::SelectDocEnd, c),
            ]);
        }
    }
    out
}

/// WYSIWYG rich editor bindings under [`rich_editor::KEY_CONTEXT`].
pub fn rich_bindings(os: Os) -> Vec<KeyBinding> {
    let c = Some(rich_editor::KEY_CONTEXT);
    let mut out = text_editing_bindings(os, rich_editor::KEY_CONTEXT, true);
    out.extend([
        KeyBinding::new("tab", rich_editor::Indent, c),
        KeyBinding::new("shift-tab", rich_editor::Outdent, c),
        KeyBinding::new(&primary(os, "b"), rich_editor::ToggleBold, c),
        KeyBinding::new(&primary(os, "i"), rich_editor::ToggleItalic, c),
        KeyBinding::new(&primary(os, "shift-x"), rich_editor::ToggleStrike, c),
        KeyBinding::new(&primary(os, "shift-m"), rich_editor::ToggleCode, c),
        KeyBinding::new(&primary(os, "k"), rich_editor::InsertLink, c),
        // Note: on European keyboard layouts, AltGr sends Ctrl+Alt, so
        // `ctrl-alt-<digit>` off macOS can collide with AltGr-composed
        // characters (such as `@`, `²`, `³`, `{`, `[`, `]`, `}`).
        KeyBinding::new(&primary(os, "alt-0"), rich_editor::SetParagraph, c),
        KeyBinding::new(&primary(os, "alt-1"), rich_editor::Heading1, c),
        KeyBinding::new(&primary(os, "alt-2"), rich_editor::Heading2, c),
        KeyBinding::new(&primary(os, "alt-3"), rich_editor::Heading3, c),
        KeyBinding::new(&primary(os, "alt-4"), rich_editor::Heading4, c),
        KeyBinding::new(&primary(os, "alt-5"), rich_editor::Heading5, c),
        KeyBinding::new(&primary(os, "alt-6"), rich_editor::Heading6, c),
        KeyBinding::new(&primary(os, "shift-8"), rich_editor::BulletList, c),
        KeyBinding::new(&primary(os, "shift-7"), rich_editor::OrderedList, c),
        KeyBinding::new(&primary(os, "shift-9"), rich_editor::TaskList, c),
        KeyBinding::new(&primary(os, "shift-."), rich_editor::BlockQuote, c),
    ]);
    out
}

/// Every key binding registered at startup for `os`, in registration order.
#[cfg(test)]
pub fn all_bindings(os: Os) -> Vec<KeyBinding> {
    let mut out = app_bindings(os);
    out.extend(editor_bindings(os));
    out.extend(rich_bindings(os));
    out.extend(editor_nav_bindings(os));
    out
}

/// One group of the Settings → Shortcuts sheet: `(group_title, &[(label, keycaps)])`.
pub type ShortcutGroup = (
    &'static str,
    &'static [(&'static str, &'static [&'static str])],
);

const SHORTCUT_GROUPS_MAC: [ShortcutGroup; 4] = [
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

const SHORTCUT_GROUPS_NON_MAC: [ShortcutGroup; 4] = [
    (
        "File",
        &[
            ("Save", &["Ctrl", "S"]),
            ("Export as .md", &["Ctrl", "Shift", "S"]),
            ("Export as PDF", &["Ctrl", "P"]),
            ("New file", &["Ctrl", "N"]),
            ("New folder", &["Ctrl", "Shift", "N"]),
            ("Open folder", &["Ctrl", "O"]),
            ("Open file", &["Ctrl", "Shift", "O"]),
        ],
    ),
    (
        "Tabs",
        &[
            ("Close tab", &["Ctrl", "W"]),
            ("Next tab", &["Ctrl", "Tab"]),
            ("Previous tab", &["Ctrl", "Shift", "Tab"]),
            ("Go to tab 1–9", &["Ctrl", "1–9"]),
        ],
    ),
    (
        "View",
        &[
            ("Toggle sidebar", &["Ctrl", "B"]),
            ("Toggle rich / plain text", &["Ctrl", "E"]),
            ("Settings", &["Ctrl", ","]),
            ("Keyboard shortcuts", &["Ctrl", "/"]),
        ],
    ),
    (
        "Editing",
        &[
            ("Find in document", &["Ctrl", "F"]),
            ("Find and replace", &["Ctrl", "Alt", "F"]),
            ("Copy document markdown", &["Ctrl", "Shift", "C"]),
            ("Copy file path", &["Ctrl", "Alt", "C"]),
        ],
    ),
];

/// Keyboard shortcuts for the Settings → Shortcuts sheet, matching `os`.
pub fn shortcut_groups(os: Os) -> &'static [ShortcutGroup] {
    match os {
        Os::Mac => &SHORTCUT_GROUPS_MAC,
        Os::Windows | Os::Linux => &SHORTCUT_GROUPS_NON_MAC,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use gpui::{KeyBinding, Keystroke};

    use super::*;

    /// Canonical `(Keystroke, context_string, action_name)` tuple for a binding.
    fn binding_tuple(binding: &KeyBinding) -> (Keystroke, Option<String>, &'static str) {
        assert_eq!(
            binding.keystrokes().len(),
            1,
            "every binding must be a single keystroke: {binding:?}"
        );
        let stroke = binding.keystrokes()[0].inner().clone();
        let context = binding.predicate().map(|pred| pred.to_string());
        (stroke, context, binding.action().name())
    }

    fn golden_entry(
        spec: &str,
        context: Option<&str>,
        action: &'static str,
    ) -> (Keystroke, Option<String>, &'static str) {
        (
            Keystroke::parse(spec).unwrap_or_else(|e| panic!("bad golden spec {spec:?}: {e}")),
            context.map(str::to_string),
            action,
        )
    }

    /// Golden test: macOS bindings must match the pre-cross-platform tables
    /// identically (same keystroke, context, action, and order).
    #[test]
    fn macos_bindings_match_golden_table() {
        let actual: Vec<_> = all_bindings(Os::Mac).iter().map(binding_tuple).collect();
        let expected: Vec<_> = [
            // Global (None)
            ("cmd-q", None, "zyplus_os::Quit"),
            ("cmd-h", None, "zyplus_os::HideApp"),
            ("cmd-alt-h", None, "zyplus_os::HideOthers"),
            ("cmd-m", None, "zyplus_os::Minimize"),
            // Zyplus
            ("cmd-s", Some("Zyplus"), "zyplus::Save"),
            ("cmd-shift-s", Some("Zyplus"), "zyplus::ExportMd"),
            ("cmd-p", Some("Zyplus"), "zyplus::ExportPdf"),
            ("cmd-n", Some("Zyplus"), "zyplus::NewFile"),
            ("cmd-shift-n", Some("Zyplus"), "zyplus::NewFolder"),
            ("cmd-w", Some("Zyplus"), "zyplus::CloseTab"),
            ("cmd-b", Some("Zyplus"), "zyplus::ToggleSidebar"),
            ("cmd-e", Some("Zyplus"), "zyplus::ToggleMode"),
            ("cmd-o", Some("Zyplus"), "zyplus::OpenFolder"),
            ("cmd-shift-o", Some("Zyplus"), "zyplus::OpenFile"),
            ("cmd-f", Some("Zyplus"), "zyplus::Find"),
            ("cmd-alt-f", Some("Zyplus"), "zyplus::Replace"),
            ("cmd-,", Some("Zyplus"), "zyplus::Settings"),
            ("cmd-/", Some("Zyplus"), "zyplus::Shortcuts"),
            ("cmd-shift-c", Some("Zyplus"), "zyplus::CopyMarkdown"),
            ("cmd-alt-c", Some("Zyplus"), "zyplus::CopyPath"),
            ("cmd-alt-right", Some("Zyplus"), "zyplus::NextTab"),
            ("cmd-alt-left", Some("Zyplus"), "zyplus::PrevTab"),
            ("cmd-shift-]", Some("Zyplus"), "zyplus::NextTab"),
            ("cmd-shift-[", Some("Zyplus"), "zyplus::PrevTab"),
            ("cmd-1", Some("Zyplus"), "zyplus::GoToTab1"),
            ("cmd-2", Some("Zyplus"), "zyplus::GoToTab2"),
            ("cmd-3", Some("Zyplus"), "zyplus::GoToTab3"),
            ("cmd-4", Some("Zyplus"), "zyplus::GoToTab4"),
            ("cmd-5", Some("Zyplus"), "zyplus::GoToTab5"),
            ("cmd-6", Some("Zyplus"), "zyplus::GoToTab6"),
            ("cmd-7", Some("Zyplus"), "zyplus::GoToTab7"),
            ("cmd-8", Some("Zyplus"), "zyplus::GoToTab8"),
            ("cmd-9", Some("Zyplus"), "zyplus::GoToTab9"),
            // ZyplusTree
            ("delete", Some("ZyplusTree"), "zyplus::DeleteSelection"),
            ("backspace", Some("ZyplusTree"), "zyplus::DeleteSelection"),
            ("up", Some("ZyplusTree"), "zyplus::TreeUp"),
            ("down", Some("ZyplusTree"), "zyplus::TreeDown"),
            ("left", Some("ZyplusTree"), "zyplus::TreeLeft"),
            ("right", Some("ZyplusTree"), "zyplus::TreeRight"),
            ("enter", Some("ZyplusTree"), "zyplus::TreeActivate"),
            ("f2", Some("ZyplusTree"), "zyplus::TreeRename"),
            // Zyplus find / overlay
            ("cmd-g", Some("Zyplus"), "zyplus::FindNext"),
            ("cmd-shift-g", Some("Zyplus"), "zyplus::FindPrev"),
            ("shift-enter", Some("Zyplus"), "zyplus::FindPrev"),
            ("escape", Some("Zyplus"), "zyplus::CloseOverlay"),
            // Editor
            ("backspace", Some("Editor"), "editor::Backspace"),
            ("delete", Some("Editor"), "editor::Delete"),
            ("enter", Some("Editor"), "editor::Enter"),
            ("left", Some("Editor"), "editor::Left"),
            ("right", Some("Editor"), "editor::Right"),
            ("up", Some("Editor"), "editor::Up"),
            ("down", Some("Editor"), "editor::Down"),
            ("home", Some("Editor"), "editor::Home"),
            ("end", Some("Editor"), "editor::End"),
            ("shift-left", Some("Editor"), "editor::SelectLeft"),
            ("shift-right", Some("Editor"), "editor::SelectRight"),
            ("shift-up", Some("Editor"), "editor::SelectUp"),
            ("shift-down", Some("Editor"), "editor::SelectDown"),
            ("shift-home", Some("Editor"), "editor::SelectHome"),
            ("shift-end", Some("Editor"), "editor::SelectEnd"),
            ("alt-left", Some("Editor"), "editor::WordLeft"),
            ("alt-right", Some("Editor"), "editor::WordRight"),
            ("shift-alt-left", Some("Editor"), "editor::SelectWordLeft"),
            ("shift-alt-right", Some("Editor"), "editor::SelectWordRight"),
            ("cmd-a", Some("Editor"), "editor::SelectAll"),
            ("cmd-c", Some("Editor"), "editor::Copy"),
            ("cmd-x", Some("Editor"), "editor::Cut"),
            ("cmd-v", Some("Editor"), "editor::Paste"),
            ("cmd-z", Some("Editor"), "editor::Undo"),
            ("cmd-shift-z", Some("Editor"), "editor::Redo"),
            ("escape", Some("Editor"), "editor::Cancel"),
            // RichEditor
            ("backspace", Some("RichEditor"), "editor::Backspace"),
            ("delete", Some("RichEditor"), "editor::Delete"),
            ("enter", Some("RichEditor"), "editor::Enter"),
            ("escape", Some("RichEditor"), "editor::Cancel"),
            ("left", Some("RichEditor"), "editor::Left"),
            ("right", Some("RichEditor"), "editor::Right"),
            ("up", Some("RichEditor"), "editor::Up"),
            ("down", Some("RichEditor"), "editor::Down"),
            ("home", Some("RichEditor"), "editor::Home"),
            ("end", Some("RichEditor"), "editor::End"),
            ("shift-left", Some("RichEditor"), "editor::SelectLeft"),
            ("shift-right", Some("RichEditor"), "editor::SelectRight"),
            ("shift-up", Some("RichEditor"), "editor::SelectUp"),
            ("shift-down", Some("RichEditor"), "editor::SelectDown"),
            ("shift-home", Some("RichEditor"), "editor::SelectHome"),
            ("shift-end", Some("RichEditor"), "editor::SelectEnd"),
            ("alt-left", Some("RichEditor"), "editor::WordLeft"),
            ("alt-right", Some("RichEditor"), "editor::WordRight"),
            (
                "shift-alt-left",
                Some("RichEditor"),
                "editor::SelectWordLeft",
            ),
            (
                "shift-alt-right",
                Some("RichEditor"),
                "editor::SelectWordRight",
            ),
            ("cmd-a", Some("RichEditor"), "editor::SelectAll"),
            ("cmd-c", Some("RichEditor"), "editor::Copy"),
            ("cmd-x", Some("RichEditor"), "editor::Cut"),
            ("cmd-v", Some("RichEditor"), "editor::Paste"),
            ("cmd-z", Some("RichEditor"), "editor::Undo"),
            ("cmd-shift-z", Some("RichEditor"), "editor::Redo"),
            ("tab", Some("RichEditor"), "rich_editor::Indent"),
            ("shift-tab", Some("RichEditor"), "rich_editor::Outdent"),
            ("cmd-b", Some("RichEditor"), "rich_editor::ToggleBold"),
            ("cmd-i", Some("RichEditor"), "rich_editor::ToggleItalic"),
            (
                "cmd-shift-x",
                Some("RichEditor"),
                "rich_editor::ToggleStrike",
            ),
            ("cmd-shift-m", Some("RichEditor"), "rich_editor::ToggleCode"),
            ("cmd-k", Some("RichEditor"), "rich_editor::InsertLink"),
            ("cmd-alt-0", Some("RichEditor"), "rich_editor::SetParagraph"),
            ("cmd-alt-1", Some("RichEditor"), "rich_editor::Heading1"),
            ("cmd-alt-2", Some("RichEditor"), "rich_editor::Heading2"),
            ("cmd-alt-3", Some("RichEditor"), "rich_editor::Heading3"),
            ("cmd-alt-4", Some("RichEditor"), "rich_editor::Heading4"),
            ("cmd-alt-5", Some("RichEditor"), "rich_editor::Heading5"),
            ("cmd-alt-6", Some("RichEditor"), "rich_editor::Heading6"),
            ("cmd-shift-8", Some("RichEditor"), "rich_editor::BulletList"),
            (
                "cmd-shift-7",
                Some("RichEditor"),
                "rich_editor::OrderedList",
            ),
            ("cmd-shift-9", Some("RichEditor"), "rich_editor::TaskList"),
            ("cmd-shift-.", Some("RichEditor"), "rich_editor::BlockQuote"),
            // Shared editor nav keys
            ("cmd-up", Some("Editor"), "editor::DocStart"),
            ("cmd-down", Some("Editor"), "editor::DocEnd"),
            ("shift-cmd-up", Some("Editor"), "editor::SelectDocStart"),
            ("shift-cmd-down", Some("Editor"), "editor::SelectDocEnd"),
            ("cmd-left", Some("Editor"), "editor::Home"),
            ("cmd-right", Some("Editor"), "editor::End"),
            ("shift-cmd-left", Some("Editor"), "editor::SelectHome"),
            ("shift-cmd-right", Some("Editor"), "editor::SelectEnd"),
            ("cmd-up", Some("RichEditor"), "editor::DocStart"),
            ("cmd-down", Some("RichEditor"), "editor::DocEnd"),
            ("shift-cmd-up", Some("RichEditor"), "editor::SelectDocStart"),
            ("shift-cmd-down", Some("RichEditor"), "editor::SelectDocEnd"),
            ("cmd-left", Some("RichEditor"), "editor::Home"),
            ("cmd-right", Some("RichEditor"), "editor::End"),
            ("shift-cmd-left", Some("RichEditor"), "editor::SelectHome"),
            ("shift-cmd-right", Some("RichEditor"), "editor::SelectEnd"),
        ]
        .into_iter()
        .map(|(k, c, a)| golden_entry(k, c, a))
        .collect();

        assert_eq!(actual, expected);
    }

    #[test]
    fn per_platform_tables_parse_cleanly_with_no_context_conflicts() {
        for os in [Os::Mac, Os::Windows, Os::Linux] {
            let bindings = all_bindings(os);
            let mut seen: HashMap<(Keystroke, Option<String>), &'static str> = HashMap::new();
            let mut global_strokes: HashSet<Keystroke> = HashSet::new();
            let mut zyplus_strokes: HashMap<Keystroke, &'static str> = HashMap::new();

            for binding in &bindings {
                let (stroke, context, action) = binding_tuple(binding);
                assert!(
                    !stroke.key.is_empty(),
                    "{os:?}: empty key in binding for {action}"
                );
                let unparsed = stroke.unparse();
                let reparsed = Keystroke::parse(&unparsed).unwrap_or_else(|err| {
                    panic!("{os:?}: unparsed {unparsed:?} failed to reparse: {err}")
                });
                assert_eq!(reparsed, stroke, "{os:?}: round-trip mismatch for {action}");

                if let Some(prev) = seen.insert((stroke.clone(), context.clone()), action) {
                    panic!(
                        "{os:?}: duplicate binding for {stroke:?} in context {context:?}: {prev} vs {action}"
                    );
                }
                if context.is_none() {
                    global_strokes.insert(stroke);
                } else if context.as_deref() == Some("Zyplus") {
                    zyplus_strokes.insert(stroke, action);
                }
            }

            // Global (None) bindings take deepest-context precedence in gpui, so
            // they must never collide with any scoped context binding.
            for binding in &bindings {
                let (stroke, context, action) = binding_tuple(binding);
                if context.is_some() {
                    assert!(
                        !global_strokes.contains(&stroke),
                        "{os:?}: global binding collides with {context:?}::{action} on {stroke:?}"
                    );
                }
                // Check intentional vs accidental overrides of Zyplus in nested editors.
                if matches!(context.as_deref(), Some("Editor" | "RichEditor")) {
                    if let Some(&parent_action) = zyplus_strokes.get(&stroke) {
                        let allowed = (stroke.key == "escape"
                            && parent_action == "zyplus::CloseOverlay"
                            && action == "editor::Cancel")
                            || (context.as_deref() == Some("RichEditor")
                                && parent_action == "zyplus::ToggleSidebar"
                                && action == "rich_editor::ToggleBold");
                        assert!(
                            allowed,
                            "{os:?}: unexpected shadow of Zyplus::{parent_action} by {context:?}::{action} on {stroke:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn windows_and_linux_bindings_follow_platform_conventions() {
        for os in [Os::Windows, Os::Linux] {
            let bindings: Vec<_> = all_bindings(os).iter().map(binding_tuple).collect();
            let has = |spec: &str, ctx: Option<&str>, action: &str| {
                let want = Keystroke::parse(spec).unwrap();
                bindings
                    .iter()
                    .any(|(s, c, a)| s == &want && c.as_deref() == ctx && *a == action)
            };

            // No cmd/super/win modifier anywhere off macOS.
            assert!(
                bindings.iter().all(|(s, _, _)| !s.modifiers.platform),
                "{os:?}: platform modifier must not be bound off macOS"
            );

            // Quit is Ctrl+Q; macOS-only app lifecycle actions are absent.
            assert!(has("ctrl-q", None, "zyplus_os::Quit"));
            for forbidden in [
                "zyplus_os::HideApp",
                "zyplus_os::HideOthers",
                "zyplus_os::Minimize",
                "zyplus_os::Zoom",
            ] {
                assert!(
                    !bindings.iter().any(|(_, _, a)| *a == forbidden),
                    "{os:?}: {forbidden} must not be bound off macOS"
                );
            }

            // Tab switching: Ctrl+Tab / Ctrl+Shift+Tab, Ctrl+PageDown / Ctrl+PageUp,
            // Ctrl+Shift+] / [, and Ctrl+1..9.
            assert!(has("ctrl-tab", Some("Zyplus"), "zyplus::NextTab"));
            assert!(has("ctrl-shift-tab", Some("Zyplus"), "zyplus::PrevTab"));
            assert!(has("ctrl-pagedown", Some("Zyplus"), "zyplus::NextTab"));
            assert!(has("ctrl-pageup", Some("Zyplus"), "zyplus::PrevTab"));
            assert!(has("ctrl-shift-]", Some("Zyplus"), "zyplus::NextTab"));
            assert!(has("ctrl-shift-[", Some("Zyplus"), "zyplus::PrevTab"));
            assert!(has("ctrl-1", Some("Zyplus"), "zyplus::GoToTab1"));
            assert!(has("ctrl-9", Some("Zyplus"), "zyplus::GoToTab9"));

            // Word & document navigation + Redo in both Editor and RichEditor.
            for ctx in ["Editor", "RichEditor"] {
                assert!(has("ctrl-left", Some(ctx), "editor::WordLeft"));
                assert!(has("ctrl-right", Some(ctx), "editor::WordRight"));
                assert!(has("shift-ctrl-left", Some(ctx), "editor::SelectWordLeft"));
                assert!(has(
                    "shift-ctrl-right",
                    Some(ctx),
                    "editor::SelectWordRight"
                ));
                assert!(has("ctrl-home", Some(ctx), "editor::DocStart"));
                assert!(has("ctrl-end", Some(ctx), "editor::DocEnd"));
                assert!(has("shift-ctrl-home", Some(ctx), "editor::SelectDocStart"));
                assert!(has("shift-ctrl-end", Some(ctx), "editor::SelectDocEnd"));
                assert!(has("ctrl-shift-z", Some(ctx), "editor::Redo"));
                assert!(has("ctrl-y", Some(ctx), "editor::Redo"));

                // alt-left / alt-right are macOS-only word moves.
                let alt_left = Keystroke::parse("alt-left").unwrap();
                let alt_right = Keystroke::parse("alt-right").unwrap();
                assert!(
                    !bindings.iter().any(|(s, c, _)| c.as_deref() == Some(ctx)
                        && (s == &alt_left || s == &alt_right)),
                    "{os:?}: alt-left/right must not be bound in {ctx}"
                );
            }

            // Rich formatting uses Ctrl equivalents.
            assert!(has("ctrl-b", Some("RichEditor"), "rich_editor::ToggleBold"));
            assert!(has(
                "ctrl-i",
                Some("RichEditor"),
                "rich_editor::ToggleItalic"
            ));
            assert!(has("ctrl-k", Some("RichEditor"), "rich_editor::InsertLink"));
            assert!(has(
                "ctrl-alt-1",
                Some("RichEditor"),
                "rich_editor::Heading1"
            ));
        }
    }

    /// Converts one shortcut sheet keycap slice (e.g. `&["Ctrl", "Shift", "Tab"]`)
    /// into the `Keystroke`(s) it represents (expanding `"1–9"` into all 9 digits).
    fn keycaps_to_keystrokes(caps: &[&str]) -> Vec<Keystroke> {
        let mut mods = Vec::new();
        let mut key_tokens: Vec<&str> = Vec::new();
        for &cap in caps {
            match cap {
                "⌘" => mods.push("cmd"),
                "Ctrl" => mods.push("ctrl"),
                "⇧" | "Shift" => mods.push("shift"),
                "⌥" | "Alt" => mods.push("alt"),
                "→" => key_tokens.push("right"),
                "←" => key_tokens.push("left"),
                "Tab" => key_tokens.push("tab"),
                "1–9" => key_tokens.extend(["1", "2", "3", "4", "5", "6", "7", "8", "9"]),
                other => key_tokens.push(other),
            }
        }
        key_tokens
            .into_iter()
            .map(|key| {
                let mut parts = mods.clone();
                let lower = key.to_ascii_lowercase();
                parts.push(&lower);
                let spec = parts.join("-");
                Keystroke::parse(&spec)
                    .unwrap_or_else(|e| panic!("keycaps {caps:?} -> {spec:?} failed: {e}"))
            })
            .collect()
    }

    #[test]
    fn shortcut_sheet_labels_match_platform_and_real_bindings() {
        for os in [Os::Mac, Os::Windows, Os::Linux] {
            let bound: HashSet<Keystroke> = all_bindings(os)
                .iter()
                .map(|b| b.keystrokes()[0].inner().clone())
                .collect();

            for (group, rows) in shortcut_groups(os) {
                for (label, caps) in *rows {
                    if os != Os::Mac {
                        for text in std::iter::once(*label).chain(caps.iter().copied()) {
                            assert!(
                                !text.contains('⌘') && !text.contains('⇧') && !text.contains('⌥'),
                                "{os:?}: macOS glyph leaked into {group}/{label}: {text:?}"
                            );
                        }
                    }
                    let strokes = keycaps_to_keystrokes(caps);
                    assert!(
                        !strokes.is_empty(),
                        "{os:?}: empty keycaps for {group}/{label}"
                    );
                    for stroke in strokes {
                        assert!(
                            bound.contains(&stroke),
                            "{os:?}: displayed shortcut {group}/{label} ({caps:?} -> {stroke:?}) has no binding"
                        );
                    }
                }
            }
        }
    }
}
