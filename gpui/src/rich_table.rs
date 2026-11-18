//! Rich table editing, navigation, and rendering support.
//!
//! Provides Typora/Notion-like table editing:
//! - Table widgets stay rendered while the caret/selection is inside the table.
//! - Active cell caret and selection painting inside the grid.
//! - Cell navigation: Tab/Shift-Tab, Enter, Up/Down, Left/Right, and Escape.
//! - Backspace/Delete guards preventing eating table delimiters or merging cells.
//! - Multi-cell selection editing (clearing cell contents while preserving delimiters).
//! - Text escaping (unescaped `|` escaped as `\|`, newlines sanitized).

use std::ops::Range;

use gpui::{point, px, Background, Bounds, PaintQuad, Pixels};

use crate::rich_blocks::{self, cell_index, SrcTable, TableBody};
use crate::rich_model::{self, Line};
use crate::rich_ops::{self, Edit};
use crate::theme::Theme;

pub const CARET_WIDTH: f32 = 2.0;

/// Escapes any unescaped `|` characters in `s` as `\|`.
/// An existing `\|` is kept as `\|` and not double-escaped.
pub fn escape_pipes(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    let mut backslashes = 0;
    for ch in s.chars() {
        if ch == '\\' {
            backslashes += 1;
            out.push('\\');
        } else if ch == '|' {
            if backslashes % 2 == 0 {
                out.push('\\');
            }
            out.push('|');
            backslashes = 0;
        } else {
            backslashes = 0;
            out.push(ch);
        }
    }
    out
}

/// Sanitizes text inserted into a table cell:
/// - Replaces newlines (`\r\n`, `\n`, `\r`) and tabs (`\t`) with spaces.
/// - Escapes unescaped pipe characters `|` as `\|`.
pub fn sanitize_cell_text(s: &str) -> String {
    let mut s2 = String::with_capacity(s.len());
    let mut prev_cr = false;
    for ch in s.chars() {
        if ch == '\r' {
            s2.push(' ');
            prev_cr = true;
        } else if ch == '\n' {
            if !prev_cr {
                s2.push(' ');
            }
            prev_cr = false;
        } else if ch == '\t' {
            s2.push(' ');
            prev_cr = false;
        } else {
            s2.push(ch);
            prev_cr = false;
        }
    }
    escape_pipes(&s2)
}

/// Whether a table widget should remain rendered as a grid given the table's
/// byte range and the current editor selection.
///
/// It stays rendered when the caret is away from the table, or when the entire
/// selection is within the table. It falls back to raw pipe text only when the
/// selection extends outside the table range (e.g. whole-document selection).
pub fn is_table_rendered(table_range: Range<usize>, sel: &Range<usize>) -> bool {
    let (s, e) = (sel.start.min(sel.end), sel.start.max(sel.end));
    let touches = s <= table_range.end && table_range.start <= e;
    if !touches {
        return true;
    }
    s >= table_range.start && e <= table_range.end
}

/// Finds the (row, col) cell in `table` corresponding to `offset`.
///
/// If `offset` falls strictly within a cell's source range, returns that cell.
/// Otherwise, finds the row corresponding to `offset` and selects the nearest cell.
pub fn cell_at_offset(table: &SrcTable, lines: &[Line], offset: usize) -> Option<(usize, usize)> {
    if let Some(rc) = rich_blocks::cell_for(table, offset) {
        return Some(rc);
    }
    let line_idx = lines.iter().position(|l| {
        let ls = l.range.start;
        let le = l.range.end + 1;
        ls <= offset && offset <= le
    })?;
    let r = if line_idx == 0 {
        0
    } else if line_idx == 1 {
        // Delimiter line: map to first body row if available, else header
        if table.rows.len() > 1 { 1 } else { 0 }
    } else {
        (line_idx - 1).min(table.rows.len().saturating_sub(1))
    };
    let row = table.rows.get(r)?;
    if row.is_empty() {
        return None;
    }
    let c = row
        .iter()
        .enumerate()
        .min_by_key(|(_, cell)| {
            if offset < cell.start {
                cell.start - offset
            } else if offset > cell.end {
                offset - cell.end
            } else {
                0
            }
        })
        .map(|(c, _)| c)
        .unwrap_or(0);
    Some((r, c))
}

/// Replaces a multi-cell selection `sel` with `insert`.
///
/// Clears the covered text in all cells between `sel.start` and `sel.end`,
/// inserts `insert` in the first cell, and leaves all `|` delimiters, spacing,
/// and line breaks intact. Returns an `Edit` that keeps the table structure valid.
pub fn multi_cell_replace(
    content: &str,
    table: &SrcTable,
    sel: Range<usize>,
    insert: &str,
) -> Edit {
    let (s, e) = (sel.start.min(sel.end), sel.start.max(sel.end));
    let mut deletions: Vec<(usize, usize)> = Vec::new();
    for row in &table.rows {
        for cell in row {
            if cell.start >= cell.end {
                continue;
            }
            let cs = cell.start.max(s);
            let ce = cell.end.min(e);
            if cs < ce {
                deletions.push((cs, ce));
            }
        }
    }

    if deletions.is_empty() {
        // No text characters in cells were selected (e.g. empty cells or delimiters only).
        // Insert at clamped s.
        return Edit {
            range: s..s,
            insert: insert.to_string(),
            selection: s + insert.len()..s + insert.len(),
        };
    }

    deletions.sort_by_key(|&(a, _)| a);

    let edit_start = deletions[0].0;
    let edit_end = deletions.last().unwrap().1;

    let mut new_slice = String::new();
    let mut cursor = edit_start;
    for (i, &(da, db)) in deletions.iter().enumerate() {
        if da > cursor {
            new_slice.push_str(&content[cursor..da]);
        }
        if i == 0 {
            new_slice.push_str(insert);
        }
        cursor = db;
    }
    if cursor < edit_end {
        new_slice.push_str(&content[cursor..edit_end]);
    }

    let caret = edit_start + insert.len();
    Edit {
        range: edit_start..edit_end,
        insert: new_slice,
        selection: caret..caret,
    }
}

/// Backspace handler inside a cell.
///
/// If `cursor <= cell.src.start`, returns `None` (preventing deleting `|` or merging cells).
/// If cursor is after `\|`, deletes both bytes `\|`.
/// Otherwise, deletes the grapheme cluster before `cursor`, clamped to `>= cell.src.start`.
pub fn cell_backspace(
    content: &str,
    table: &SrcTable,
    r: usize,
    c: usize,
    cursor: usize,
) -> Option<Edit> {
    let cell_src = table.rows.get(r)?.get(c)?.clone();
    if cursor <= cell_src.start {
        return None;
    }
    if cursor >= cell_src.start + 2 && &content[cursor - 2..cursor] == "\\|" {
        let start = cursor - 2;
        return Some(Edit {
            range: start..cursor,
            insert: String::new(),
            selection: start..start,
        });
    }
    let prev = rich_ops::backspace_start(content, cursor);
    let start = prev.max(cell_src.start);
    Some(Edit {
        range: start..cursor,
        insert: String::new(),
        selection: start..start,
    })
}

/// Delete (forward delete) handler inside a cell.
///
/// If `cursor >= cell.src.end`, returns `None` (preventing deleting `|` or merging cells).
/// If cursor is before `\|`, deletes both bytes `\|`.
/// Otherwise, deletes the grapheme cluster after `cursor`, clamped to `<= cell.src.end`.
pub fn cell_delete(
    content: &str,
    table: &SrcTable,
    r: usize,
    c: usize,
    cursor: usize,
) -> Option<Edit> {
    let cell_src = table.rows.get(r)?.get(c)?.clone();
    if cursor >= cell_src.end {
        return None;
    }
    if cursor + 2 <= cell_src.end && &content[cursor..cursor + 2] == "\\|" {
        let end = cursor + 2;
        return Some(Edit {
            range: cursor..end,
            insert: String::new(),
            selection: cursor..cursor,
        });
    }
    let next = rich_ops::next_grapheme(content, cursor);
    let end = next.min(cell_src.end);
    Some(Edit {
        range: cursor..end,
        insert: String::new(),
        selection: cursor..cursor,
    })
}

/// Action to perform for table keyboard navigation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NavAction {
    /// Move caret to offset without modifying document.
    Move(usize),
    /// Leave table to offset before the table.
    LeaveBefore(usize),
    /// Leave table to offset after the table.
    LeaveAfter(usize),
    /// Insert an empty line before the table and move to it.
    InsertLineBefore(usize),
    /// Insert an empty line after the table and move to it.
    InsertLineAfter(usize),
}

/// Left arrow navigation inside a table cell.
pub fn nav_left(
    content: &str,
    table: &SrcTable,
    lines: Range<usize>,
    r: usize,
    c: usize,
    cursor: usize,
) -> NavAction {
    let cell_src = &table.rows[r][c];
    if cursor > cell_src.start {
        let prev = rich_ops::prev_grapheme(content, cursor).max(cell_src.start);
        return NavAction::Move(prev);
    }
    // At cell start: move to previous cell
    if c > 0 {
        return NavAction::Move(table.rows[r][c - 1].end);
    }
    if r > 0 {
        let last_c = table.cols().saturating_sub(1);
        return NavAction::Move(table.rows[r - 1][last_c].end);
    }
    // First cell of table: exit above
    if lines.start == 0 {
        NavAction::InsertLineBefore(0)
    } else {
        NavAction::LeaveBefore(lines.start - 1)
    }
}

/// Right arrow navigation inside a table cell.
pub fn nav_right(
    content: &str,
    table: &SrcTable,
    lines: Range<usize>,
    total_lines: usize,
    r: usize,
    c: usize,
    cursor: usize,
) -> NavAction {
    let cell_src = &table.rows[r][c];
    if cursor < cell_src.end {
        let next = rich_ops::next_grapheme(content, cursor).min(cell_src.end);
        return NavAction::Move(next);
    }
    // At cell end: move to next cell
    if c + 1 < table.cols() {
        return NavAction::Move(table.rows[r][c + 1].start);
    }
    if r + 1 < table.rows.len() {
        return NavAction::Move(table.rows[r + 1][0].start);
    }
    // Last cell of table: exit below
    if lines.end >= total_lines {
        NavAction::InsertLineAfter(content.len())
    } else {
        NavAction::LeaveAfter(lines.end)
    }
}

/// Up arrow navigation inside a table.
pub fn nav_up(
    _content: &str,
    table: &SrcTable,
    tb: Option<&TableBody>,
    lines: Range<usize>,
    r: usize,
    c: usize,
    cursor: usize,
    desired_x: Option<f32>,
) -> NavAction {
    if r > 0 {
        let target_r = r - 1;
        if let (Some(tb), Some(x)) = (tb, desired_x) {
            let cols = tb.col_edges.len().saturating_sub(1);
            let clamped_x = x.clamp(tb.col_edges[0], tb.col_edges[cols] - 0.1);
            let target_c = cell_index(&tb.col_edges, clamped_x).unwrap_or(c).min(cols - 1);
            let cell = &tb.cells[target_r][target_c];
            let (idx, _) = cell.para.hit_test(point(px(x - cell.x), px(0.0)));
            let off = cell.src.start + rich_model::display_to_source(&cell.disp, idx);
            return NavAction::Move(off);
        }
        let target_c = c.min(table.cols().saturating_sub(1));
        let cell_src = &table.rows[target_r][target_c];
        let off_in_cell = cursor.saturating_sub(table.rows[r][c].start).min(cell_src.len());
        return NavAction::Move(cell_src.start + off_in_cell);
    }
    // Header row: leave table above
    if lines.start == 0 {
        NavAction::InsertLineBefore(0)
    } else {
        NavAction::LeaveBefore(lines.start - 1)
    }
}

/// Down arrow navigation inside a table.
pub fn nav_down(
    content: &str,
    table: &SrcTable,
    tb: Option<&TableBody>,
    lines: Range<usize>,
    total_lines: usize,
    r: usize,
    c: usize,
    cursor: usize,
    desired_x: Option<f32>,
) -> NavAction {
    if r + 1 < table.rows.len() {
        let target_r = r + 1;
        if let (Some(tb), Some(x)) = (tb, desired_x) {
            let cols = tb.col_edges.len().saturating_sub(1);
            let clamped_x = x.clamp(tb.col_edges[0], tb.col_edges[cols] - 0.1);
            let target_c = cell_index(&tb.col_edges, clamped_x).unwrap_or(c).min(cols - 1);
            let cell = &tb.cells[target_r][target_c];
            let (idx, _) = cell.para.hit_test(point(px(x - cell.x), px(0.0)));
            let off = cell.src.start + rich_model::display_to_source(&cell.disp, idx);
            return NavAction::Move(off);
        }
        let target_c = c.min(table.cols().saturating_sub(1));
        let cell_src = &table.rows[target_r][target_c];
        let off_in_cell = cursor.saturating_sub(table.rows[r][c].start).min(cell_src.len());
        return NavAction::Move(cell_src.start + off_in_cell);
    }
    // Last row: leave table below
    if lines.end >= total_lines {
        NavAction::InsertLineAfter(content.len())
    } else {
        NavAction::LeaveAfter(lines.end)
    }
}

/// Escape key navigation inside a table.
pub fn nav_escape(
    content: &str,
    lines: Range<usize>,
    total_lines: usize,
) -> NavAction {
    if lines.end >= total_lines {
        NavAction::InsertLineAfter(content.len())
    } else {
        NavAction::LeaveAfter(lines.end)
    }
}

/// Computes caret and selection paint quads for a rendered table body.
pub fn table_quads(
    bounds: Bounds<Pixels>,
    tb: &TableBody,
    sel: &Range<usize>,
    cursor: usize,
    at_row_end: bool,
    theme: &Theme,
    caret_out: &mut Option<PaintQuad>,
    selection_out: &mut Vec<PaintQuad>,
) {
    let at = |x: f32, y: f32| point(bounds.left() + px(x), bounds.top() + px(y));
    let (s, e) = (sel.start.min(sel.end), sel.start.max(sel.end));

    // 1. Caret quad in the active cell
    for row in &tb.cells {
        for cell in row {
            if cell.src.start <= cursor && cursor <= cell.src.end {
                let rel = cursor - cell.src.start;
                let idx = rich_model::source_to_display(&cell.disp, rel);
                let origin = at(cell.x, cell.y);
                *caret_out = Some(cell.para.caret_quad(
                    origin,
                    idx,
                    at_row_end,
                    theme.cursor,
                    px(CARET_WIDTH),
                ));
                break;
            }
        }
        if caret_out.is_some() {
            break;
        }
    }

    // 2. Selection highlights in covered cells
    if !sel.is_empty() {
        for (r, row) in tb.cells.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                let touches = cell.src.start <= e && s <= cell.src.end;
                if touches {
                    let cs = s.max(cell.src.start);
                    let ce = e.min(cell.src.end);
                    if cs < ce {
                        let ds = rich_model::source_to_display(&cell.disp, cs - cell.src.start);
                        let de = rich_model::source_to_display(&cell.disp, ce - cell.src.start);
                        if ds < de {
                            let origin = at(cell.x, cell.y);
                            selection_out.extend(cell.para.range_quads(
                                origin,
                                ds..de,
                                theme.selection,
                                px(1.0),
                            ));
                        }
                    } else if cell.src.start == cell.src.end && s <= cell.src.start && cell.src.end <= e {
                        // Empty cell covered by selection: subtle cell fill
                        let w = tb.col_edges[c + 1] - tb.col_edges[c];
                        let h = tb.row_edges[r + 1] - tb.row_edges[r];
                        selection_out.push(fill_quad(
                            Bounds::new(at(tb.col_edges[c] + 1.0, tb.row_edges[r] + 1.0), size(px(w - 2.0), px(h - 2.0))),
                            theme.selection,
                        ));
                    }
                }
            }
        }
    }
}

fn fill_quad(bounds: Bounds<Pixels>, color: impl Into<Background>) -> PaintQuad {
    gpui::fill(bounds, color)
}

fn size(w: Pixels, h: Pixels) -> gpui::Size<Pixels> {
    gpui::size(w, h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rich_blocks::parse_table;
    use crate::rich_model::parse;

    #[test]
    fn pipe_escaping_handles_unescaped_and_already_escaped() {
        assert_eq!(escape_pipes("hello|world"), "hello\\|world");
        assert_eq!(escape_pipes("hello\\|world"), "hello\\|world");
        assert_eq!(escape_pipes("a|b|c"), "a\\|b\\|c");
        assert_eq!(escape_pipes("no pipes"), "no pipes");
        assert_eq!(escape_pipes("|start|"), "\\|start\\|");
    }

    #[test]
    fn sanitize_cell_text_strips_newlines_and_escapes_pipes() {
        assert_eq!(sanitize_cell_text("line1\nline2"), "line1 line2");
        assert_eq!(sanitize_cell_text("line1\r\nline2"), "line1 line2");
        assert_eq!(sanitize_cell_text("cell|with|pipes\nnew"), "cell\\|with\\|pipes new");
        assert_eq!(sanitize_cell_text("tab\tseparated"), "tab separated");
    }

    #[test]
    fn table_stays_rendered_while_selection_is_inside() {
        let table_range = 10..50;
        // Caret away from table
        assert!(is_table_rendered(table_range.clone(), &(0..0)));
        assert!(is_table_rendered(table_range.clone(), &(60..60)));

        // Caret inside table
        assert!(is_table_rendered(table_range.clone(), &(15..15)));
        assert!(is_table_rendered(table_range.clone(), &(10..10)));
        assert!(is_table_rendered(table_range.clone(), &(50..50)));

        // Selection inside table
        assert!(is_table_rendered(table_range.clone(), &(12..25)));
        assert!(is_table_rendered(table_range.clone(), &(10..50)));

        // Selection crossing boundary falls back to raw
        assert!(!is_table_rendered(table_range.clone(), &(5..15)));
        assert!(!is_table_rendered(table_range.clone(), &(45..55)));
        assert!(!is_table_rendered(table_range.clone(), &(0..100)));
    }

    #[test]
    fn backspace_at_cell_start_refuses_to_eat_delimiters() {
        let src = "| a | b |\n| - | - |\n| c | d |\n";
        let m = parse(src);
        let t = parse_table(src, &m.lines[0..3]);
        // Cell (0, 0) is "a", range 2..3
        assert_eq!(t.rows[0][0], 2..3);
        // Backspace at cell start (offset 2)
        let edit = cell_backspace(src, &t, 0, 0, 2);
        assert!(edit.is_none(), "backspace at start of cell must do nothing");

        // Backspace inside cell (offset 3) deletes "a"
        let edit = cell_backspace(src, &t, 0, 0, 3).unwrap();
        assert_eq!(edit.range, 2..3);
        assert_eq!(edit.insert, "");
    }

    #[test]
    fn delete_at_cell_end_refuses_to_eat_delimiters() {
        let src = "| a | b |\n| - | - |\n| c | d |\n";
        let m = parse(src);
        let t = parse_table(src, &m.lines[0..3]);
        // Cell (0, 0) is "a", range 2..3
        assert_eq!(t.rows[0][0], 2..3);
        // Delete at cell end (offset 3)
        let edit = cell_delete(src, &t, 0, 0, 3);
        assert!(edit.is_none(), "delete at end of cell must do nothing");

        // Delete at cell start (offset 2) deletes "a"
        let edit = cell_delete(src, &t, 0, 0, 2).unwrap();
        assert_eq!(edit.range, 2..3);
        assert_eq!(edit.insert, "");
    }

    #[test]
    fn backspace_over_escaped_pipe_deletes_both_bytes() {
        let src = "| a\\|b | c |\n| - | - |\n";
        let m = parse(src);
        let t = parse_table(src, &m.lines[0..2]);
        // Cell (0, 0) is "a\|b", start is 2, length 4 bytes: a (2), \ (3), | (4), b (5)
        // Cursor after "\|" is offset 5 (before "b")
        let edit = cell_backspace(src, &t, 0, 0, 5).unwrap();
        assert_eq!(edit.range, 3..5);
        assert_eq!(edit.insert, "");
    }

    #[test]
    fn multi_cell_replace_clears_contents_and_preserves_delimiters() {
        let src = "| hello | world |\n| --- | --- |\n| one | two |\n";
        let m = parse(src);
        let t = parse_table(src, &m.lines[0..3]);
        // Select from inside "hello" (offset 4) across delimiter to "world" (offset 13)
        let edit = multi_cell_replace(src, &t, 4..13, "");
        let applied = edit.apply(src);
        // "hello" (2..7) had 4..7 cleared -> "he"
        // "world" (10..15) had 10..13 cleared -> "ld"
        // The "| |" delimiters between them remain intact!
        assert_eq!(applied, "| he | ld |\n| --- | --- |\n| one | two |\n");

        let edit_type = multi_cell_replace(src, &t, 4..13, "XY");
        let applied_type = edit_type.apply(src);
        assert_eq!(applied_type, "| heXY | ld |\n| --- | --- |\n| one | two |\n");
    }

    #[test]
    fn nav_arrows_cross_cell_boundaries_and_leave_table() {
        let src = "| a | b |\n| - | - |\n| c | d |\n";
        let m = parse(src);
        let t = parse_table(src, &m.lines[0..3]);
        let lines = 0..3;
        let total_lines = 3;

        // Left at cell start moves to previous cell
        let act = nav_left(src, &t, lines.clone(), 0, 1, t.rows[0][1].start);
        assert_eq!(act, NavAction::Move(t.rows[0][0].end));

        // Right at cell end moves to next cell
        let act = nav_right(src, &t, lines.clone(), total_lines, 0, 0, t.rows[0][0].end);
        assert_eq!(act, NavAction::Move(t.rows[0][1].start));

        // Left at (0, 0) start leaves table above (creates line if at doc start)
        let act = nav_left(src, &t, lines.clone(), 0, 0, t.rows[0][0].start);
        assert_eq!(act, NavAction::InsertLineBefore(0));

        // Right at last cell end leaves table below (creates line if at doc end)
        let act = nav_right(src, &t, lines.clone(), total_lines, 1, 1, t.rows[1][1].end);
        assert_eq!(act, NavAction::InsertLineAfter(src.len()));
    }

    #[test]
    fn inline_formatting_in_cells_reveals_under_caret_and_conceals_away() {
        let text = "**bold**";
        // When caret is away (None): markers are concealed
        let d_away = rich_blocks::cell_display(text, None);
        assert_eq!(d_away.text, "bold");
        assert_eq!(rich_model::display_to_source(&d_away, 0), 2); // 'b' is at offset 2 in "**bold**"

        // When caret touches the inline marker (e.g. at offset 0): markers are revealed
        let d_touched = rich_blocks::cell_display(text, Some(0..0));
        assert_eq!(d_touched.text, "**bold**");
        assert_eq!(rich_model::display_to_source(&d_touched, 0), 0);
    }

    #[test]
    fn cjk_and_thai_text_in_cells() {
        let src = "| สวัสดี | 中文 |\n| :--- | :---: |\n| test | 😀 |\n";
        let m = parse(src);
        let t = parse_table(src, &m.lines[0..3]);
        assert_eq!(t.cols(), 2);
        assert_eq!(&src[t.rows[0][0].clone()], "สวัสดี");
        assert_eq!(&src[t.rows[0][1].clone()], "中文");
        assert_eq!(&src[t.rows[1][1].clone()], "😀");

        // Cell navigation and offsets work with multibyte characters
        let (r, c) = cell_at_offset(&t, &m.lines[0..3], t.rows[0][0].start).unwrap();
        assert_eq!((r, c), (0, 0));
        let (r, c) = cell_at_offset(&t, &m.lines[0..3], t.rows[0][1].start).unwrap();
        assert_eq!((r, c), (0, 1));
    }

    #[test]
    fn empty_cells_navigation_and_editing() {
        let src = "|  | b |\n| --- | --- |\n|  |  |\n";
        let m = parse(src);
        let t = parse_table(src, &m.lines[0..3]);
        // Cell (0, 0) is empty
        assert_eq!(t.rows[0][0].len(), 0);
        // Backspace and delete on empty cell return None
        assert!(cell_backspace(src, &t, 0, 0, t.rows[0][0].start).is_none());
        assert!(cell_delete(src, &t, 0, 0, t.rows[0][0].start).is_none());
    }

    #[test]
    fn nav_up_down_and_escape() {
        let src = "| h1 | h2 |\n| -- | -- |\n| b1 | b2 |\n";
        let m = parse(src);
        let t = parse_table(src, &m.lines[0..3]);
        let lines = 0..3;
        let total_lines = 3;

        // Up from header row (r=0) leaves table above
        let act = nav_up(src, &t, None, lines.clone(), 0, 0, t.rows[0][0].start, None);
        assert_eq!(act, NavAction::InsertLineBefore(0));

        // Down from header row (r=0) goes to body row (r=1)
        let act = nav_down(src, &t, None, lines.clone(), total_lines, 0, 0, t.rows[0][0].start, None);
        assert_eq!(act, NavAction::Move(t.rows[1][0].start));

        // Down from last body row leaves table below
        let act = nav_down(src, &t, None, lines.clone(), total_lines, 1, 0, t.rows[1][0].start, None);
        assert_eq!(act, NavAction::InsertLineAfter(src.len()));

        // Escape leaves table after
        let act = nav_escape(src, lines.clone(), total_lines);
        assert_eq!(act, NavAction::InsertLineAfter(src.len()));
    }
}
