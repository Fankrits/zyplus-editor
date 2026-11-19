//! Pure keyboard-navigation and file-open rules for the sidebar tree and the
//! find bar. No GPUI types, so the index math and the open policy can be
//! unit-tested. The window code in `main.rs` applies the results.

use std::ops::Range;

/// The row shape the tree keys need: depth, and whether a row is a folder
/// and open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowShape {
    pub depth: usize,
    pub is_folder: bool,
    pub is_open: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeKey {
    Up,
    Down,
    Left,
    Right,
}

/// What an arrow key does to the tree (react-arborist's defaults).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeMove {
    Nothing,
    /// Move the cursor and selection to this visible row.
    Select(usize),
    /// Open the folder at this row.
    Expand(usize),
    /// Close the folder at this row.
    Collapse(usize),
}

/// Up/Down step through the visible rows; with no cursor yet, Down picks the
/// first row and Up the last. Right opens a closed folder, or steps into an
/// open one (onto its first child). Left closes an open folder, or moves to
/// the parent row. Files ignore Left and Right beyond moving to the parent.
pub fn tree_move(rows: &[RowShape], current: Option<usize>, key: TreeKey) -> TreeMove {
    if rows.is_empty() {
        return TreeMove::Nothing;
    }
    let Some(i) = current.filter(|i| *i < rows.len()) else {
        return match key {
            TreeKey::Down => TreeMove::Select(0),
            TreeKey::Up => TreeMove::Select(rows.len() - 1),
            TreeKey::Left | TreeKey::Right => TreeMove::Nothing,
        };
    };
    let row = rows[i];
    match key {
        TreeKey::Up => {
            if i > 0 {
                TreeMove::Select(i - 1)
            } else {
                TreeMove::Nothing
            }
        }
        TreeKey::Down => {
            if i + 1 < rows.len() {
                TreeMove::Select(i + 1)
            } else {
                TreeMove::Nothing
            }
        }
        TreeKey::Left => {
            if row.is_folder && row.is_open {
                return TreeMove::Collapse(i);
            }
            // The nearest row above with a smaller depth is the parent.
            match (0..i).rev().find(|&j| rows[j].depth < row.depth) {
                Some(parent) => TreeMove::Select(parent),
                None => TreeMove::Nothing,
            }
        }
        TreeKey::Right => {
            if !row.is_folder {
                return TreeMove::Nothing;
            }
            if !row.is_open {
                return TreeMove::Expand(i);
            }
            match rows.get(i + 1) {
                Some(next) if next.depth > row.depth => TreeMove::Select(i + 1),
                _ => TreeMove::Nothing,
            }
        }
    }
}

/// Extensions of the Markdown/plain-text files the tree opens (`isMarkdownFile`).
const MARKDOWN_TREE_EXTENSIONS: [&str; 3] = [".md", ".markdown", ".txt"];

/// Extensions the Code & Config Files extension edits (its `fileExtensions`
/// in `extensions/catalog.ts`).
const CODE_EXTENSIONS: [&str; 30] = [
    ".yaml",
    ".yml",
    ".toml",
    ".xml",
    ".sh",
    ".bash",
    ".zsh",
    ".ini",
    ".env",
    ".css",
    ".html",
    ".js",
    ".jsx",
    ".ts",
    ".tsx",
    ".py",
    ".rs",
    ".go",
    ".sql",
    ".rb",
    ".php",
    ".c",
    ".cpp",
    ".h",
    ".java",
    ".kt",
    ".swift",
    ".dockerfile",
    "dockerfile",
    ".gitignore",
];

/// Whether activating a file in the tree opens it as a tab. Only Markdown and
/// plain text, and files an enabled extension edits (`csv`, `json`,
/// `codefiles`), open. Anything else, such as images, archives or binaries, is
/// ignored, as in `FileTree.tsx`. A disabled extension leaves its files
/// unopened too.
pub fn tree_opens(name: &str, enabled: impl Fn(&str) -> bool) -> bool {
    let lower = name.to_ascii_lowercase();
    let ends = |exts: &[&str]| exts.iter().any(|ext| lower.ends_with(ext));
    if ends(&MARKDOWN_TREE_EXTENSIONS) {
        return true;
    }
    if ends(&[".csv", ".tsv"]) {
        return enabled("csv");
    }
    if ends(&[".json"]) {
        return enabled("json");
    }
    if ends(&CODE_EXTENSIONS) {
        return enabled("codefiles");
    }
    false
}

/// The match Find moves to from `caret` (the selection). Forward picks the
/// first match that starts at or after the caret's end, wrapping to the first
/// match. Backward picks the last match that ends at or before the caret's
/// start, wrapping to the last. So stepping continues from wherever the caret
/// is, after an edit or a click, not from the old index. `matches` must not be
/// empty.
pub fn find_next_index(matches: &[Range<usize>], caret: Range<usize>, delta: isize) -> usize {
    debug_assert!(!matches.is_empty());
    if delta >= 0 {
        matches
            .iter()
            .position(|m| m.start >= caret.end)
            .unwrap_or(0)
    } else {
        matches
            .iter()
            .rposition(|m| m.end <= caret.start)
            .unwrap_or(matches.len() - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(depth: usize, is_folder: bool, is_open: bool) -> RowShape {
        RowShape {
            depth,
            is_folder,
            is_open,
        }
    }

    /// root (open folder)
    ///   docs (open folder)
    ///     a.md
    ///   notes (closed folder)
    ///   b.md
    fn sample() -> Vec<RowShape> {
        vec![
            row(0, true, true),
            row(1, true, true),
            row(2, false, false),
            row(1, true, false),
            row(1, false, false),
        ]
    }

    #[test]
    fn up_and_down_step_and_stop_at_the_ends() {
        let rows = sample();
        assert_eq!(
            tree_move(&rows, Some(0), TreeKey::Down),
            TreeMove::Select(1)
        );
        assert_eq!(tree_move(&rows, Some(4), TreeKey::Down), TreeMove::Nothing);
        assert_eq!(tree_move(&rows, Some(2), TreeKey::Up), TreeMove::Select(1));
        assert_eq!(tree_move(&rows, Some(0), TreeKey::Up), TreeMove::Nothing);
    }

    #[test]
    fn no_cursor_starts_at_the_first_or_last_row() {
        let rows = sample();
        assert_eq!(tree_move(&rows, None, TreeKey::Down), TreeMove::Select(0));
        assert_eq!(tree_move(&rows, None, TreeKey::Up), TreeMove::Select(4));
        assert_eq!(tree_move(&rows, None, TreeKey::Left), TreeMove::Nothing);
        assert_eq!(tree_move(&rows, None, TreeKey::Right), TreeMove::Nothing);
        assert_eq!(tree_move(&[], None, TreeKey::Down), TreeMove::Nothing);
        // A stale cursor past the end is treated as no cursor.
        assert_eq!(
            tree_move(&rows, Some(99), TreeKey::Down),
            TreeMove::Select(0)
        );
    }

    #[test]
    fn right_expands_a_closed_folder_then_steps_into_it() {
        let rows = sample();
        // notes (row 3) is closed: Right opens it.
        assert_eq!(
            tree_move(&rows, Some(3), TreeKey::Right),
            TreeMove::Expand(3)
        );
        // docs (row 1) is open with a child: Right steps onto a.md.
        assert_eq!(
            tree_move(&rows, Some(1), TreeKey::Right),
            TreeMove::Select(2)
        );
        // b.md is a file: Right does nothing.
        assert_eq!(tree_move(&rows, Some(4), TreeKey::Right), TreeMove::Nothing);
    }

    #[test]
    fn right_on_an_open_folder_without_children_does_nothing() {
        let rows = vec![row(0, true, true), row(0, false, false)];
        assert_eq!(tree_move(&rows, Some(0), TreeKey::Right), TreeMove::Nothing);
    }

    #[test]
    fn left_collapses_an_open_folder_or_moves_to_the_parent() {
        let rows = sample();
        // docs is open: Left closes it.
        assert_eq!(
            tree_move(&rows, Some(1), TreeKey::Left),
            TreeMove::Collapse(1)
        );
        // a.md is a file inside docs: Left moves to docs.
        assert_eq!(
            tree_move(&rows, Some(2), TreeKey::Left),
            TreeMove::Select(1)
        );
        // b.md at depth 1 moves to its parent, root.
        assert_eq!(
            tree_move(&rows, Some(4), TreeKey::Left),
            TreeMove::Select(0)
        );
        // A closed folder at depth 1 moves to root too.
        assert_eq!(
            tree_move(&rows, Some(3), TreeKey::Left),
            TreeMove::Select(0)
        );
        // The root has no parent: Left does nothing.
        assert_eq!(
            tree_move(&rows, Some(0), TreeKey::Left),
            TreeMove::Collapse(0)
        );
        let closed_root = vec![row(0, true, false)];
        assert_eq!(
            tree_move(&closed_root, Some(0), TreeKey::Left),
            TreeMove::Nothing
        );
    }

    #[test]
    fn markdown_and_text_files_open_from_the_tree() {
        let on = |_: &str| true;
        assert!(tree_opens("/p/Notes.md", on));
        assert!(tree_opens("/p/readme.MARKDOWN", on));
        assert!(tree_opens("/p/todo.txt", on));
        // Without any extension switch, these still open.
        assert!(tree_opens("/p/todo.txt", |_| false));
    }

    #[test]
    fn data_and_code_files_follow_their_extension_switch() {
        let all = |_: &str| true;
        let none = |_: &str| false;
        assert!(tree_opens("/p/data.csv", all));
        assert!(tree_opens("/p/data.TSV", all));
        assert!(!tree_opens("/p/data.csv", none));
        assert!(tree_opens("/p/data.json", all));
        assert!(!tree_opens("/p/data.json", |id: &str| id != "json"));
        assert!(tree_opens("/p/config.yaml", all));
        assert!(tree_opens("/p/main.rs", all));
        assert!(tree_opens("/p/Dockerfile", all));
        assert!(!tree_opens("/p/main.rs", none));
        // Code files are not affected by the JSON or CSV switches.
        assert!(!tree_opens("/p/main.rs", |id: &str| id == "json" || id == "csv"));
    }

    #[test]
    fn binaries_and_unknown_files_are_ignored() {
        let all = |_: &str| true;
        assert!(!tree_opens("/p/photo.png", all));
        assert!(!tree_opens("/p/archive.zip", all));
        assert!(!tree_opens("/p/tool.exe", all));
        assert!(!tree_opens("/p/Makefile", all));
        assert!(!tree_opens("/p/noext", all));
    }

    #[test]
    fn find_forward_continues_from_the_caret() {
        let m = vec![2..4, 10..12, 20..22];
        // Caret collapsed at 0: first match at or after it.
        assert_eq!(find_next_index(&m, 0..0, 1), 0);
        // Caret after an edit at 11: continues with the match at 20.
        assert_eq!(find_next_index(&m, 11..11, 1), 2);
        // Caret on match 10..12 (just stepped there): next is 20..22.
        assert_eq!(find_next_index(&m, 10..12, 1), 2);
        // Past the last match: wraps to the first.
        assert_eq!(find_next_index(&m, 25..25, 1), 0);
    }

    #[test]
    fn find_backward_continues_from_the_caret() {
        let m = vec![2..4, 10..12, 20..22];
        // Caret on match 10..12: previous is 2..4.
        assert_eq!(find_next_index(&m, 10..12, -1), 0);
        // Caret at 15: the last match ending before it is 10..12.
        assert_eq!(find_next_index(&m, 15..15, -1), 1);
        // Before the first match: wraps to the last.
        assert_eq!(find_next_index(&m, 0..0, -1), 2);
    }

    #[test]
    fn stepping_forward_then_back_returns_to_the_same_match() {
        let m = vec![2..4, 10..12, 20..22];
        // Forward from the start: 0, 1, 2 (each step selects the match it lands on).
        let mut index = find_next_index(&m, 0..0, 1);
        assert_eq!(index, 0);
        index = find_next_index(&m, m[index].clone(), 1);
        assert_eq!(index, 1);
        index = find_next_index(&m, m[index].clone(), 1);
        assert_eq!(index, 2);
        // One step back from the last match lands on the middle one.
        index = find_next_index(&m, m[index].clone(), -1);
        assert_eq!(index, 1);
    }
}
