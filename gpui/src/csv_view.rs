//! CSV and TSV editing for the GPUI port.
//!
//! Two layers live here:
//!
//! * [`CsvTable`] is the data model. It parses RFC 4180-style text (quoted
//!   fields, doubled quotes, embedded newlines, ragged rows, BOM, CRLF) and
//!   writes it back. Every record keeps its source text until one of its cells
//!   changes, so opening and saving without edits is byte-identical, and an
//!   edit only rewrites the record it touched.
//! * [`CsvView`] is the GPUI entity: a sticky header, virtualised rows,
//!   keyboard navigation, inline cell editing, row/column actions and a Text
//!   view over the raw file.
//!
//! Record 0 of the model is the header, as in the web extension. Row numbers
//! shown in the view are record indices, so the first data row is row 1.

use std::cmp::Ordering;
use std::fmt;

use gpui::prelude::*;
use gpui::{
    point, px, uniform_list, AnyElement, App, ClickEvent, Context, Div, Entity, FocusHandle,
    Focusable, FontWeight, KeyDownEvent, MouseButton, MouseDownEvent, ScrollHandle, ScrollStrategy,
    SharedString, Stateful, UniformListScrollHandle, Window, div,
};

use crate::editor::{Editor, FONT_SIZE, MONO_FONT};
use crate::theme::Theme;

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

/// How many characters and lines the delimiter sniffer looks at.
const SNIFF_CHARS: usize = 16 * 1024;
const SNIFF_LINES: usize = 20;

/// The line terminator after a record. `None` is a final record with no newline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Term {
    None,
    Lf,
    CrLf,
    Cr,
}

impl Term {
    fn as_str(self) -> &'static str {
        match self {
            Term::None => "",
            Term::Lf => "\n",
            Term::CrLf => "\r\n",
            Term::Cr => "\r",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Record {
    cells: Vec<String>,
    /// The record's exact source text, kept until a cell changes. `None` means
    /// the record must be re-serialised from `cells`.
    raw: Option<String>,
    term: Term,
}

/// A parsed CSV/TSV document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CsvTable {
    bom: bool,
    delimiter: u8,
    /// Terminator for records the editor inserts.
    newline: Term,
    records: Vec<Record>,
}

/// Picks the delimiter for a file. `.tsv` is always tab. Otherwise the most
/// frequent candidate outside quotes in the first lines wins, with comma as
/// the default, so `.csv` files stay comma-delimited unless they clearly are not.
pub fn detect_delimiter(path: &str, text: &str) -> char {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".tsv") || lower.ends_with(".tab") {
        return '\t';
    }
    const CANDIDATES: [char; 4] = [',', '\t', ';', '|'];
    let mut counts = [0usize; 4];
    let mut in_quotes = false;
    let mut lines = 0;
    for ch in text.chars().take(SNIFF_CHARS) {
        match ch {
            '"' => in_quotes = !in_quotes,
            '\n' if !in_quotes => {
                lines += 1;
                if lines >= SNIFF_LINES {
                    break;
                }
            }
            _ if in_quotes => {}
            _ => {
                if let Some(i) = CANDIDATES.iter().position(|&c| c == ch) {
                    counts[i] += 1;
                }
            }
        }
    }
    if lower.ends_with(".csv") && counts[0] > 0 {
        return ',';
    }
    let mut best = 0;
    for i in 1..counts.len() {
        if counts[i] > counts[best] {
            best = i;
        }
    }
    if counts[best] == 0 {
        ','
    } else {
        CANDIDATES[best]
    }
}

fn is_field_end(byte: u8, delim: u8) -> bool {
    byte == delim || byte == b'\r' || byte == b'\n'
}

fn line_of(bytes: &[u8], pos: usize) -> usize {
    bytes[..pos].iter().filter(|&&b| b == b'\n').count() + 1
}

fn longest_line(text: &str) -> usize {
    // Multi-line cells are drawn flattened onto one line, so the width is the
    // whole text's length, not its longest line.
    text.chars().count()
}

fn number(text: &str) -> Option<f64> {
    text.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Ordering for sorting: empty cells always last, then numbers, then text
/// (case-insensitive). Descending flips the non-empty comparison only.
fn compare_cells(a: &str, b: &str, ascending: bool) -> Ordering {
    let (a, b) = (a.trim(), b.trim());
    match (a.is_empty(), b.is_empty()) {
        (true, true) => return Ordering::Equal,
        (true, false) => return Ordering::Greater,
        (false, true) => return Ordering::Less,
        _ => {}
    }
    let ord = match (number(a), number(b)) {
        (Some(x), Some(y)) => x.total_cmp(&y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => a
            .to_lowercase()
            .cmp(&b.to_lowercase())
            .then_with(|| a.cmp(b)),
    };
    if ascending { ord } else { ord.reverse() }
}

fn cell_of(record: &Record, col: usize) -> &str {
    record.cells.get(col).map(String::as_str).unwrap_or("")
}

fn needs_quotes(cell: &str, delim: u8) -> bool {
    cell.bytes()
        .any(|b| b == delim || b == b'"' || b == b'\n' || b == b'\r')
}

fn write_record(f: &mut fmt::Formatter<'_>, cells: &[String], delim: u8) -> fmt::Result {
    // A single empty field must not look like a blank line.
    if cells.len() == 1 && cells[0].is_empty() {
        return f.write_str("\"\"");
    }
    for (i, cell) in cells.iter().enumerate() {
        if i > 0 {
            write!(f, "{}", delim as char)?;
        }
        if needs_quotes(cell, delim) {
            f.write_str("\"")?;
            f.write_str(&cell.replace('"', "\"\""))?;
            f.write_str("\"")?;
        } else {
            f.write_str(cell)?;
        }
    }
    Ok(())
}

impl CsvTable {
    /// An empty table with no records.
    pub fn empty(delimiter: char) -> Self {
        Self {
            bom: false,
            delimiter: u8::try_from(delimiter).unwrap_or(b','),
            newline: Term::Lf,
            records: Vec::new(),
        }
    }

    /// Parses `text` with the given delimiter. Errors are for input that is
    /// not well-formed CSV (an unclosed quote, or text after a closing quote),
    /// which the view answers by opening the Text view.
    pub fn parse(text: &str, delimiter: char) -> Result<Self, String> {
        let delim = match u8::try_from(delimiter) {
            Ok(b) if b.is_ascii() && !matches!(b, b'"' | b'\r' | b'\n') => b,
            _ => return Err(format!("unsupported delimiter {delimiter:?}")),
        };
        let (bom, body) = match text.strip_prefix('\u{feff}') {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        let bytes = body.as_bytes();
        let len = bytes.len();
        let mut records = Vec::new();
        let mut newline = None;
        let mut i = 0;
        while i < len {
            let start = i;
            let mut cells = Vec::new();
            loop {
                let mut field = Vec::new();
                if i < len && bytes[i] == b'"' {
                    let quote_at = i;
                    i += 1;
                    loop {
                        if i >= len {
                            return Err(format!(
                                "unclosed quote on line {}",
                                line_of(bytes, quote_at)
                            ));
                        }
                        if bytes[i] == b'"' {
                            if i + 1 < len && bytes[i + 1] == b'"' {
                                field.push(b'"');
                                i += 2;
                                continue;
                            }
                            i += 1;
                            break;
                        }
                        field.push(bytes[i]);
                        i += 1;
                    }
                    if i < len && !is_field_end(bytes[i], delim) {
                        return Err(format!(
                            "text after a closing quote on line {}",
                            line_of(bytes, i)
                        ));
                    }
                } else {
                    while i < len && !is_field_end(bytes[i], delim) {
                        field.push(bytes[i]);
                        i += 1;
                    }
                }
                cells.push(String::from_utf8_lossy(&field).into_owned());
                if i < len && bytes[i] == delim {
                    i += 1;
                    continue;
                }
                break;
            }
            let content_end = i;
            let term = if i < len && bytes[i] == b'\r' {
                if i + 1 < len && bytes[i + 1] == b'\n' {
                    i += 2;
                    Term::CrLf
                } else {
                    i += 1;
                    Term::Cr
                }
            } else if i < len && bytes[i] == b'\n' {
                i += 1;
                Term::Lf
            } else {
                Term::None
            };
            if newline.is_none() && term != Term::None {
                newline = Some(term);
            }
            records.push(Record {
                cells,
                raw: Some(body[start..content_end].to_string()),
                term,
            });
        }
        Ok(Self {
            bom,
            delimiter: delim,
            newline: newline.unwrap_or(Term::Lf),
            records,
        })
    }

    pub fn delimiter(&self) -> char {
        self.delimiter as char
    }

    /// Records, including the header at index 0.
    pub fn row_count(&self) -> usize {
        self.records.len()
    }

    /// The widest record. Shorter (ragged) records read as empty past their end.
    pub fn col_count(&self) -> usize {
        self.records
            .iter()
            .map(|r| r.cells.len())
            .max()
            .unwrap_or(0)
    }

    pub fn cell(&self, row: usize, col: usize) -> &str {
        self.records
            .get(row)
            .and_then(|r| r.cells.get(col))
            .map(String::as_str)
            .unwrap_or("")
    }

    pub fn row(&self, row: usize) -> Option<&[String]> {
        self.records.get(row).map(|r| r.cells.as_slice())
    }

    /// Sets one cell, padding a ragged record as needed. Returns `false` when
    /// nothing changed, including when the value is already there.
    pub fn set_cell(&mut self, row: usize, col: usize, value: &str) -> bool {
        let Some(record) = self.records.get_mut(row) else {
            return false;
        };
        match record.cells.get(col) {
            Some(existing) if existing == value => return false,
            None if value.is_empty() => return false,
            _ => {}
        }
        if record.cells.len() <= col {
            record.cells.resize(col + 1, String::new());
        }
        record.cells[col] = value.to_string();
        record.raw = None;
        true
    }

    /// Inserts a record before `at`. `at == row_count()` appends. When the
    /// file had no final newline, the new last record keeps that.
    pub fn insert_row(&mut self, at: usize, cells: Vec<String>) -> bool {
        let len = self.records.len();
        if at > len {
            return false;
        }
        let mut term = self.newline;
        if at == len && len > 0 && self.records[len - 1].term == Term::None {
            self.records[len - 1].term = self.newline;
            term = Term::None;
        }
        self.records.insert(
            at,
            Record {
                cells,
                raw: None,
                term,
            },
        );
        true
    }

    /// Removes the record at `at`. Removing the last record hands its
    /// terminator to the new last record, so the file ends the same way.
    pub fn delete_row(&mut self, at: usize) -> bool {
        let len = self.records.len();
        if at >= len {
            return false;
        }
        let removed = self.records.remove(at);
        if at == len - 1 {
            if let Some(last) = self.records.last_mut() {
                last.term = removed.term;
            }
        }
        true
    }

    /// Inserts an empty cell at `col` in every record. Short records are
    /// padded to `col` first, so the column lines up.
    pub fn insert_col(&mut self, col: usize) -> bool {
        if self.records.is_empty() {
            self.records.push(Record {
                cells: Vec::new(),
                raw: None,
                term: Term::None,
            });
        }
        for record in &mut self.records {
            if record.cells.len() < col {
                record.cells.resize(col, String::new());
            }
            record.cells.insert(col, String::new());
            record.raw = None;
        }
        true
    }

    /// Removes column `col` from every record that has it.
    pub fn delete_col(&mut self, col: usize) -> bool {
        let mut changed = false;
        for record in &mut self.records {
            if col < record.cells.len() {
                record.cells.remove(col);
                record.raw = None;
                changed = true;
            }
        }
        changed
    }

    /// Sorts the data rows by `col`. The header stays first. Terminators stay
    /// with their positions, so the end-of-file state survives a sort.
    pub fn sort_rows(&mut self, col: usize, ascending: bool) -> bool {
        if self.records.len() < 3 {
            return false;
        }
        let mut data = self.records.split_off(1);
        let terms: Vec<Term> = data.iter().map(|r| r.term).collect();
        let mut order: Vec<usize> = (0..data.len()).collect();
        order.sort_by(|&a, &b| {
            compare_cells(cell_of(&data[a], col), cell_of(&data[b], col), ascending)
        });
        if order.iter().enumerate().all(|(pos, &idx)| pos == idx) {
            self.records.extend(data);
            return false;
        }
        let mut slots: Vec<Option<Record>> = data.drain(..).map(Some).collect();
        for (pos, idx) in order.into_iter().enumerate() {
            if let Some(mut record) = slots[idx].take() {
                record.term = terms[pos];
                self.records.push(record);
            }
        }
        true
    }

    /// Width of each column in characters, as displayed (multi-line cells are flattened).
    pub fn column_widths(&self) -> Vec<usize> {
        let mut widths = vec![0; self.col_count()];
        for record in &self.records {
            for (col, cell) in record.cells.iter().enumerate() {
                widths[col] = widths[col].max(longest_line(cell));
            }
        }
        widths
    }
}

impl fmt::Display for CsvTable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.bom {
            f.write_str("\u{feff}")?;
        }
        for record in &self.records {
            match &record.raw {
                Some(raw) => f.write_str(raw)?,
                None => write_record(f, &record.cells, self.delimiter)?,
            }
            f.write_str(record.term.as_str())?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

const TOOLBAR_H: f32 = 36.0;
const ROW_H: f32 = 28.0;
const NUM_W: f32 = 56.0;
/// Advance width of the mono face at `FONT_SIZE`.
const CHAR_W: f32 = 8.4;
const CELL_PAD: f32 = 20.0;
const MIN_COL_W: f32 = 80.0;
const MAX_COL_W: f32 = 320.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Table,
    Text,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToolAction {
    AddRow,
    DeleteRow,
    AddColumn,
    DeleteColumn,
    SortAsc,
    SortDesc,
}

/// The CSV/TSV editor entity.
///
/// `on_change` receives the full serialised text after every edit. It runs
/// while this entity is being updated, so it must not read the `CsvView`.
pub struct CsvView {
    focus: FocusHandle,
    table: CsvTable,
    mode: Mode,
    /// Why the text does not parse. Shown in Text mode, and blocks the switch
    /// back to Table.
    parse_error: Option<String>,
    /// The current full text: the raw buffer in Text mode, and the serialised
    /// table in Table mode.
    content: String,
    theme_name: String,
    /// (record, column). Record 0 is the header.
    selected: (usize, usize),
    editing: Option<(usize, usize)>,
    /// The cell's value before the edit started. An unchanged cell is not
    /// written back, so a multi-line cell is not flattened by an Enter.
    editing_original: String,
    cell_editor: Entity<Editor>,
    text_editor: Entity<Editor>,
    list_scroll: UniformListScrollHandle,
    hscroll: ScrollHandle,
    cols: usize,
    col_widths: Vec<f32>,
    pub on_change: Option<Box<dyn Fn(&str, &mut App)>>,
}

impl CsvView {
    /// Builds the view for a file. `path` picks the delimiter. Text that does
    /// not parse opens in Text mode with the error shown.
    pub fn new(path: &str, content: &str, theme_name: &str, cx: &mut Context<Self>) -> Self {
        let delimiter = detect_delimiter(path, content);
        let (table, mode, parse_error) = match CsvTable::parse(content, delimiter) {
            Ok(table) => (table, Mode::Table, None),
            Err(err) => (CsvTable::empty(delimiter), Mode::Text, Some(err)),
        };
        let weak = cx.weak_entity();
        let theme_name = theme_name.to_string();

        let cell_editor = cx.new(|cx| {
            let mut editor = Editor::new(true, cx).with_padding(6.0, 2.0);
            editor.theme_name = theme_name.clone();
            editor
        });
        let weak_submit = weak.clone();
        let weak_cancel = weak.clone();
        cell_editor.update(cx, |editor, _| {
            // Deferred: the editor is still being updated while its callbacks run.
            editor.on_submit = Some(Box::new(move |cx: &mut App| {
                let weak = weak_submit.clone();
                cx.defer(move |cx| {
                    let _ = weak.update(cx, |this, cx| this.finish_edit(true, None, cx));
                });
            }));
            editor.on_cancel = Some(Box::new(move |cx: &mut App| {
                let weak = weak_cancel.clone();
                cx.defer(move |cx| {
                    let _ = weak.update(cx, |this, cx| this.finish_edit(false, None, cx));
                });
            }));
        });

        let text_editor = cx.new(|cx| {
            let mut editor = Editor::new(false, cx);
            editor.theme_name = theme_name.clone();
            editor
        });
        let weak_text = weak.clone();
        text_editor.update(cx, |editor, cx| {
            editor.set_text(content, cx);
            editor.on_change = Some(Box::new(move |text: &str, cx: &mut App| {
                let _ = weak_text.update(cx, |this, cx| this.on_text_edit(text, cx));
            }));
        });

        let mut this = Self {
            focus: cx.focus_handle(),
            table,
            mode,
            parse_error,
            content: content.to_string(),
            theme_name,
            selected: (0, 0),
            editing: None,
            editing_original: String::new(),
            cell_editor,
            text_editor,
            list_scroll: UniformListScrollHandle::new(),
            hscroll: ScrollHandle::new(),
            cols: 0,
            col_widths: Vec::new(),
            on_change: None,
        };
        this.refresh_layout();
        this.selected = if this.table.row_count() > 1 {
            (1, 0)
        } else {
            (0, 0)
        };
        this
    }

    pub fn set_theme(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.theme_name == name {
            return;
        }
        self.theme_name = name.to_string();
        self.cell_editor.update(cx, |editor, cx| editor.set_theme(name, cx));
        self.text_editor.update(cx, |editor, cx| editor.set_theme(name, cx));
        cx.notify();
    }

    // ---- change reporting -------------------------------------------------

    fn emit(&mut self, cx: &mut Context<Self>) {
        if self.mode == Mode::Table {
            self.content = self.table.to_string();
        }
        if let Some(on_change) = self.on_change.as_ref() {
            on_change(&self.content, cx);
        }
    }

    /// Called by the text editor after each keystroke. Re-parses the table when
    /// the text is valid. Otherwise keeps the last good table and shows the error.
    fn on_text_edit(&mut self, text: &str, cx: &mut Context<Self>) {
        self.content = text.to_string();
        match CsvTable::parse(text, self.table.delimiter()) {
            Ok(table) => {
                self.table = table;
                self.parse_error = None;
                self.refresh_layout();
                self.clamp_selection();
            }
            Err(err) => self.parse_error = Some(err),
        }
        self.emit(cx);
        cx.notify();
    }

    fn refresh_layout(&mut self) {
        self.cols = self.table.col_count();
        self.col_widths = self
            .table
            .column_widths()
            .into_iter()
            .map(|chars| (chars as f32 * CHAR_W + CELL_PAD).clamp(MIN_COL_W, MAX_COL_W))
            .collect();
    }

    fn total_width(&self) -> f32 {
        NUM_W + self.col_widths.iter().sum::<f32>()
    }

    fn clamp_selection(&mut self) {
        let rows = self.table.row_count();
        if rows == 0 {
            self.selected = (0, 0);
            return;
        }
        let row = self.selected.0.min(rows - 1);
        let col = if self.cols == 0 {
            0
        } else {
            self.selected.1.min(self.cols - 1)
        };
        self.selected = (row, col);
    }

    // ---- editing ----------------------------------------------------------

    fn start_edit(
        &mut self,
        row: usize,
        col: usize,
        initial: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.mode != Mode::Table || row >= self.table.row_count() || col >= self.cols {
            return;
        }
        if self.editing.is_some() {
            self.finish_edit(true, Some(window), cx);
        }
        self.selected = (row, col);
        let original = self.table.cell(row, col).to_string();
        let value = initial.unwrap_or_else(|| one_line(&original));
        self.cell_editor.update(cx, |editor, cx| {
            editor.set_text(&value, cx);
            let end = editor.text().len();
            editor.select(end..end, cx);
        });
        self.editing = Some((row, col));
        self.editing_original = original;
        let focus = self.cell_editor.read(cx).focus_handle(cx);
        window.focus(&focus);
        self.scroll_to_selection();
        cx.notify();
    }

    /// Ends the inline edit. With `commit`, the editor's text is written to the
    /// cell if it changed. Without a window (a deferred callback), focus goes
    /// back to the active window.
    fn finish_edit(&mut self, commit: bool, window: Option<&mut Window>, cx: &mut Context<Self>) {
        let Some((row, col)) = self.editing.take() else {
            return;
        };
        if commit {
            let text = self.cell_editor.read(cx).text().to_string();
            if text != one_line(&self.editing_original) && self.table.set_cell(row, col, &text) {
                self.refresh_layout();
                self.emit(cx);
            }
        }
        self.editing_original.clear();
        let focus = self.focus.clone();
        match window {
            Some(window) => window.focus(&focus),
            None => {
                if let Some(handle) = cx.active_window() {
                    let _ = handle.update(cx, |_, window, _| window.focus(&focus));
                }
            }
        }
        cx.notify();
    }

    fn on_cell_mouse_down(
        &mut self,
        row: usize,
        col: usize,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.mode != Mode::Table || self.editing == Some((row, col)) {
            return;
        }
        if event.click_count >= 2 {
            self.start_edit(row, col, None, window, cx);
            return;
        }
        self.finish_edit(true, Some(window), cx);
        self.selected = (row, col);
        window.focus(&self.focus);
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode != Mode::Table {
            return;
        }
        let keystroke = &event.keystroke;
        if keystroke.modifiers.platform || keystroke.modifiers.control || keystroke.modifiers.function
        {
            return;
        }
        let shift = keystroke.modifiers.shift;
        let key = keystroke.key.as_str();

        if self.editing.is_some() {
            // Keys the cell editor does not consume reach here. Enter and
            // Escape are handled through the editor's callbacks.
            match key {
                "tab" => {
                    self.finish_edit(true, Some(window), cx);
                    self.tab(shift);
                }
                "up" | "down" => {
                    self.finish_edit(true, Some(window), cx);
                    self.step(if key == "up" { -1 } else { 1 }, 0);
                }
                "escape" => self.finish_edit(false, Some(window), cx),
                _ => return,
            }
            cx.stop_propagation();
            self.scroll_to_selection();
            cx.notify();
            return;
        }

        let mut handled = true;
        match key {
            "up" => self.step(-1, 0),
            "down" => self.step(1, 0),
            "left" => self.step(0, -1),
            "right" => self.step(0, 1),
            "tab" => self.tab(shift),
            "enter" | "f2" => {
                let (row, col) = self.selected;
                self.start_edit(row, col, None, window, cx);
            }
            _ => handled = false,
        }
        if !handled {
            // Typing on a selected cell replaces its value, like a spreadsheet.
            if let Some(text) = keystroke.key_char.as_ref() {
                if !text.is_empty() && !text.chars().any(char::is_control) {
                    let (row, col) = self.selected;
                    self.start_edit(row, col, Some(text.clone()), window, cx);
                    handled = true;
                }
            }
        }
        if handled {
            cx.stop_propagation();
            self.scroll_to_selection();
            cx.notify();
        }
    }

    fn step(&mut self, dr: isize, dc: isize) {
        let rows = self.table.row_count();
        if rows == 0 || self.cols == 0 {
            return;
        }
        let row = (self.selected.0 as isize + dr).clamp(0, rows as isize - 1) as usize;
        let col = (self.selected.1 as isize + dc).clamp(0, self.cols as isize - 1) as usize;
        self.selected = (row, col);
    }

    /// Tab moves right, wrapping to the next row. Shift-Tab goes left.
    fn tab(&mut self, backwards: bool) {
        let rows = self.table.row_count();
        if rows == 0 || self.cols == 0 {
            return;
        }
        let (row, col) = self.selected;
        self.selected = if !backwards {
            if col + 1 < self.cols {
                (row, col + 1)
            } else if row + 1 < rows {
                (row + 1, 0)
            } else {
                (row, col)
            }
        } else if col > 0 {
            (row, col - 1)
        } else if row > 0 {
            (row - 1, self.cols - 1)
        } else {
            (row, col)
        };
    }

    /// Keeps the selection on screen. Vertical scrolling uses the list's
    /// non-strict scroll, so it moves only when needed. Horizontal scrolling is
    /// done by hand from the measured viewport.
    fn scroll_to_selection(&self) {
        let (row, col) = self.selected;
        let index = row.saturating_sub(1);
        self.list_scroll.scroll_to_item(index, ScrollStrategy::Top);

        let viewport = f32::from(self.hscroll.bounds().size.width);
        if viewport <= 0.0 || col >= self.col_widths.len() {
            return;
        }
        let offset = self.hscroll.offset();
        let shown = -f32::from(offset.x);
        let left = NUM_W + self.col_widths[..col].iter().sum::<f32>();
        let right = left + self.col_widths[col];
        let target = if left < shown {
            Some(left)
        } else if right > shown + viewport {
            Some(right - viewport)
        } else {
            None
        };
        if let Some(target) = target {
            self.hscroll.set_offset(point(px(-target), offset.y));
        }
    }

    // ---- structure and mode -----------------------------------------------

    fn run_action(&mut self, action: ToolAction, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode != Mode::Table {
            return;
        }
        self.finish_edit(true, Some(window), cx);
        let (row, col) = self.selected;
        let changed = match action {
            ToolAction::AddRow => {
                let rows = self.table.row_count();
                let at = if rows == 0 { 0 } else { (row + 1).max(1) };
                let cells = vec![String::new(); self.cols];
                let inserted = self.table.insert_row(at, cells);
                if inserted {
                    self.selected = (at, col);
                }
                inserted
            }
            // The header is not deleted from the toolbar.
            ToolAction::DeleteRow => row >= 1 && self.table.delete_row(row),
            ToolAction::AddColumn => {
                let at = if self.cols == 0 { 0 } else { col + 1 };
                let inserted = self.table.insert_col(at);
                if inserted {
                    self.selected = (self.selected.0, at);
                }
                inserted
            }
            ToolAction::DeleteColumn => self.cols > 0 && self.table.delete_col(col),
            ToolAction::SortAsc => self.table.sort_rows(col, true),
            ToolAction::SortDesc => self.table.sort_rows(col, false),
        };
        if changed {
            self.refresh_layout();
            self.clamp_selection();
            self.emit(cx);
        }
        self.scroll_to_selection();
        window.focus(&self.focus);
        cx.notify();
    }

    /// Switches to `id` ("table" or "text"). Table is refused while the text
    /// does not parse. `window` is only needed to move focus.
    pub fn select_mode(&mut self, id: &str, window: Option<&mut Window>, cx: &mut Context<Self>) {
        let mode = match id {
            "table" => Mode::Table,
            "text" => Mode::Text,
            _ => return,
        };
        self.set_mode(mode, window, cx);
    }

    /// "table" or "text": the view the file is showing.
    pub fn mode(&self) -> &'static str {
        match self.mode {
            Mode::Table => "table",
            Mode::Text => "text",
        }
    }

    /// The raw-text editor, for Find and Replace while the Text view is shown.
    pub fn text_editor(&self) -> Entity<Editor> {
        self.text_editor.clone()
    }

    fn set_mode(&mut self, mode: Mode, mut window: Option<&mut Window>, cx: &mut Context<Self>) {
        if mode == self.mode {
            return;
        }
        self.finish_edit(true, window.as_deref_mut(), cx);
        match mode {
            Mode::Text => {
                let text = self.table.to_string();
                self.content = text.clone();
                self.text_editor.update(cx, |editor, cx| editor.set_text(&text, cx));
                self.parse_error = None;
                self.mode = Mode::Text;
                if let Some(window) = window {
                    let focus = self.text_editor.read(cx).focus_handle(cx);
                    window.focus(&focus);
                }
            }
            Mode::Table => {
                // Only leave Text mode when the text parses. Otherwise the
                // error stays on screen.
                let text = self.content.clone();
                match CsvTable::parse(&text, self.table.delimiter()) {
                    Ok(table) => {
                        self.table = table;
                        self.parse_error = None;
                        self.mode = Mode::Table;
                        self.refresh_layout();
                        self.clamp_selection();
                        if let Some(window) = window {
                            window.focus(&self.focus);
                        }
                    }
                    Err(err) => self.parse_error = Some(err),
                }
            }
        }
        cx.notify();
    }

    // ---- rendering --------------------------------------------------------

    fn render_toolbar(&self, theme: Theme, cx: &mut Context<Self>) -> Div {
        let table_mode = self.mode == Mode::Table;
        let data_rows = self.table.row_count().saturating_sub(1);
        let status = format!("{data_rows} rows · {} columns", self.cols);
        let sort_ok = table_mode && self.table.row_count() > 2 && self.cols > 0;
        let mut bar = div()
            .flex_none()
            .h(px(TOOLBAR_H))
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .bg(theme.sidebar_bg)
            .border_b_1()
            .border_color(theme.border)
            .child(self.tool_button("csv-add-row", "Add row", ToolAction::AddRow, false, table_mode, theme, cx))
            .child(self.tool_button("csv-del-row", "Delete row", ToolAction::DeleteRow, false, table_mode, theme, cx))
            .child(self.tool_button("csv-add-col", "Add column", ToolAction::AddColumn, false, table_mode, theme, cx))
            .child(self.tool_button("csv-del-col", "Delete column", ToolAction::DeleteColumn, false, table_mode, theme, cx))
            .child(self.tool_button("csv-sort-asc", "Sort A→Z", ToolAction::SortAsc, false, sort_ok, theme, cx))
            .child(self.tool_button("csv-sort-desc", "Sort Z→A", ToolAction::SortDesc, false, sort_ok, theme, cx));
        bar = bar.child(div().flex_1());
        if table_mode {
            bar = bar.child(
                div()
                    .text_size(px(12.0))
                    .text_color(theme.muted)
                    .child(status),
            );
        }
        bar
    }

    #[allow(clippy::too_many_arguments)]
    fn tool_button(
        &self,
        id: &'static str,
        label: &'static str,
        action: ToolAction,
        active: bool,
        enabled: bool,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let mut button = div()
            .id(id)
            .flex()
            .items_center()
            .h(px(24.0))
            .px_2()
            .rounded_md()
            .text_size(px(12.0))
            .font_family(MONO_FONT);
        if active {
            button = button.bg(theme.accent_soft).text_color(theme.accent);
        } else if enabled {
            button = button
                .text_color(theme.fg)
                .cursor_pointer()
                .hover(|style| style.bg(theme.hover));
        } else {
            button = button.text_color(theme.muted);
        }
        if enabled {
            button = button.on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.run_action(action, window, cx);
            }));
        }
        button.child(label)
    }

    fn render_table(&self, theme: Theme, cx: &mut Context<Self>) -> AnyElement {
        if self.table.row_count() == 0 {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.muted)
                .child("Empty file. Use Add row to start.")
                .into_any_element();
        }
        let total_w = self.total_width();
        let data_rows = self.table.row_count() - 1;
        let view = cx.entity();
        let list = uniform_list("csv-rows", data_rows, move |range, _window, cx| {
            view.update(cx, |this, cx| {
                range
                    .map(|index| this.render_record(index + 1, theme, total_w, cx))
                    .collect::<Vec<_>>()
            })
        })
        .track_scroll(self.list_scroll.clone())
        .flex_1()
        .min_h_0()
        .w(px(total_w));

        // The header sits outside the list, so it stays put while rows scroll.
        // The whole block scrolls horizontally together.
        let header = self.render_record(0, theme, total_w, cx);
        div()
            .id("csv-hscroll")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_x_scroll()
            .track_scroll(&self.hscroll)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .w(px(total_w))
                    .child(header)
                    .child(list),
            )
            .into_any_element()
    }

    fn render_record(&self, record: usize, theme: Theme, total_w: f32, cx: &mut Context<Self>) -> Div {
        let is_header = record == 0;
        let mut row = div()
            .flex()
            .flex_none()
            .h(px(ROW_H))
            .w(px(total_w))
            .bg(if is_header { theme.sidebar_bg } else { theme.bg })
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex_none()
                    .w(px(NUM_W))
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_end()
                    .pr_2()
                    .border_r_1()
                    .border_color(theme.border)
                    .text_size(px(12.0))
                    .text_color(theme.muted)
                    .child(if is_header {
                        String::new()
                    } else {
                        record.to_string()
                    }),
            );
        for col in 0..self.cols {
            row = row.child(self.render_cell(record, col, theme, cx));
        }
        row
    }

    fn render_cell(&self, row: usize, col: usize, theme: Theme, cx: &mut Context<Self>) -> Div {
        let width = self.col_widths.get(col).copied().unwrap_or(MIN_COL_W);
        let mut cell = div()
            .flex_none()
            .w(px(width))
            .h_full()
            .overflow_hidden()
            .flex()
            .items_center()
            .border_r_1()
            .border_color(theme.border)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.on_cell_mouse_down(row, col, event, window, cx);
                }),
            );
        if row == 0 {
            cell = cell.font_weight(FontWeight::SEMIBOLD);
        }
        if self.editing == Some((row, col)) {
            cell = cell
                .border_1()
                .border_color(theme.accent)
                .child(self.cell_editor.clone());
        } else {
            let text = if row == 0 && col >= self.table.row(0).map_or(0, |r| r.len()) {
                format!("Column {}", col + 1)
            } else {
                display_text(self.table.cell(row, col))
            };
            cell = cell.px_2().child(
                div()
                    .w_full()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(SharedString::from(text)),
            );
            if self.selected == (row, col) {
                cell = cell.border_1().border_color(theme.accent).bg(theme.accent_soft);
            }
        }
        cell
    }

    fn render_text(&self) -> AnyElement {
        div()
            .flex_1()
            .min_h_0()
            .child(self.text_editor.clone())
            .into_any_element()
    }
}

/// The single-line editor shows a cell with its line breaks flattened, the
/// same way `Editor::set_text` does.
fn one_line(text: &str) -> String {
    text.replace(['\r', '\n'], " ")
}

/// Cell text for display. Control characters (tabs, stray breaks) show as spaces.
fn display_text(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

impl Focusable for CsvView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match self.mode {
            Mode::Table => self.focus.clone(),
            Mode::Text => self.text_editor.read(cx).focus_handle(cx),
        }
    }
}

impl Render for CsvView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::for_name(&self.theme_name, window.appearance());
        let body = match self.mode {
            Mode::Table => self.render_table(theme, cx),
            Mode::Text => self.render_text(),
        };
        let error = self.parse_error.clone();
        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key_down))
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            .bg(theme.bg)
            .text_color(theme.fg)
            .text_size(px(FONT_SIZE))
            .font_family(MONO_FONT)
            .child(self.render_toolbar(theme, cx))
            .when_some(error, |this, message| {
                this.child(
                    div()
                        .flex_none()
                        .px_3()
                        .py_1()
                        .text_size(px(12.0))
                        .text_color(theme.danger)
                        .child(format!(
                            "Not a valid table: {message}. Fix it in the Text view to switch back."
                        )),
                )
            })
            .child(body)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> CsvTable {
        CsvTable::parse(text, ',').expect("parses")
    }

    fn rt(text: &str, delim: char) {
        let table = CsvTable::parse(text, delim).expect("parses");
        assert_eq!(table.to_string(), text, "round trip of {text:?}");
    }

    #[test]
    fn round_trips_byte_for_byte() {
        let cases = [
            "a,b,c\n1,2,3",
            "a,\"b,c\",d\n1,2,3",
            "a,\"b\nc\",d\n1,2,3",
            "\u{feff}a,b,c\n1,2,3",
            "a,b,c\r\n1,2,3",
            "a,b,c\n1,2,3",
            "a,b,c\n1,2,3\n",
            "a,b,c\n1,2\n3,4,5,6",
            "a,\"b\"\r\n1,2\r\n3,4",
            "a\n\nb",
            "a,\"say \"\"hi\"\"\"",
            "\"\"",
            "",
            "\n",
            "\n\n",
            "a,b,\n,,\n",
            "\"\u{feff}x\",ü,日本\n",
        ];
        for text in cases {
            rt(text, ',');
        }
        rt("a\tb\tc\n1\t2\t3", '\t');
        rt("a\tb\r\n\"x\ty\"\t2", '\t');
    }

    #[test]
    fn parses_quotes_embedded_newlines_and_escapes() {
        let table = parse("a,\"b,c\",d\n1,\"x\ny\",\"say \"\"hi\"\"\"");
        assert_eq!(table.row_count(), 2);
        assert_eq!(table.row(0), Some(&["a".to_string(), "b,c".to_string(), "d".to_string()][..]));
        assert_eq!(table.cell(1, 1), "x\ny");
        assert_eq!(table.cell(1, 2), "say \"hi\"");
    }

    #[test]
    fn ragged_rows_read_empty_past_their_end() {
        let table = parse("a,b,c\n1,2\n3,4,5,6");
        assert_eq!(table.row_count(), 3);
        assert_eq!(table.col_count(), 4);
        assert_eq!(table.cell(1, 2), "");
        assert_eq!(table.cell(2, 3), "6");
        assert_eq!(table.cell(9, 9), "");
    }

    #[test]
    fn bom_and_crlf_are_kept() {
        let table = parse("\u{feff}a,b\r\n1,2\r\n");
        assert_eq!(table.cell(0, 0), "a");
        assert_eq!(table.to_string(), "\u{feff}a,b\r\n1,2\r\n");
    }

    #[test]
    fn unclosed_quote_and_text_after_quote_are_errors() {
        assert!(CsvTable::parse("a,b,\"c\n1,2,3", ',').is_err());
        assert!(CsvTable::parse("\"a\"b,c", ',').is_err());
        assert!(CsvTable::parse("ab\"c,d", ',').is_ok(), "stray quote is literal");
        assert!(CsvTable::parse("a,b", '"').is_err(), "quote is not a delimiter");
    }

    #[test]
    fn detects_delimiter() {
        assert_eq!(detect_delimiter("data.tsv", "a,b\n1,2"), '\t');
        // A .csv with no commas at all is sniffed as tab, like Papa's auto-detect.
        assert_eq!(detect_delimiter("data.csv", "a\tb\n1\t2"), '\t');
        assert_eq!(detect_delimiter("data.csv", "a,b\n1,2"), ',');
        assert_eq!(detect_delimiter("data.txt", "a;b;c\n1;2;3"), ';');
        assert_eq!(detect_delimiter("data.txt", "a\tb\tc\n1\t2\t3"), '\t');
        assert_eq!(detect_delimiter("data.txt", "plain"), ',');
        // Commas inside quotes must not outvote the real delimiter.
        assert_eq!(detect_delimiter("x.txt", "\"a,b,c,d\";x\n1;2"), ';');
    }

    #[test]
    fn set_cell_changes_only_its_record() {
        let mut table = parse("a,\"b\"\r\n1,2\r\n3,4");
        assert!(table.set_cell(1, 1, "9"));
        // Row 0 keeps its unnecessary quotes and CRLF. Row 2 stays as it was.
        assert_eq!(table.to_string(), "a,\"b\"\r\n1,9\r\n3,4");
    }

    #[test]
    fn set_cell_requotes_only_when_needed() {
        let mut table = parse("a,b\n1,2");
        let cases = [
            ("plain", "plain"),
            ("x,y", "\"x,y\""),
            ("say \"x\"", "\"say \"\"x\"\"\""),
            ("two\nlines", "\"two\nlines\""),
            ("", ""),
        ];
        for (value, expected) in cases {
            assert!(table.set_cell(1, 0, value) || value.is_empty());
            let out = table.to_string();
            assert_eq!(out, format!("a,b\n{expected},2"), "value {value:?}");
        }
    }

    #[test]
    fn set_cell_noops_and_bounds() {
        let mut table = parse("a,b\n1,2");
        assert!(!table.set_cell(1, 0, "1"), "same value is not a change");
        assert!(!table.set_cell(7, 0, "x"), "row out of range");
        assert!(!table.set_cell(1, 5, ""), "empty value past the end is not a change");
        assert_eq!(table.to_string(), "a,b\n1,2");
    }

    #[test]
    fn set_cell_pads_ragged_record() {
        let mut table = parse("a,b,c\n1,2");
        assert!(table.set_cell(1, 2, "x"));
        assert_eq!(table.to_string(), "a,b,c\n1,2,x");
    }

    #[test]
    fn insert_row_respects_missing_final_newline() {
        let mut table = parse("a\nb");
        assert!(table.insert_row(2, vec!["c".into()]));
        assert_eq!(table.to_string(), "a\nb\nc");

        let mut table = parse("a\nb\n");
        assert!(table.insert_row(1, vec!["x".into()]));
        assert_eq!(table.to_string(), "a\nx\nb\n");
    }

    #[test]
    fn insert_row_uses_file_newline_style() {
        let mut table = parse("a\r\nb\r\n");
        assert!(table.insert_row(2, vec!["c".into()]));
        assert_eq!(table.to_string(), "a\r\nb\r\nc\r\n");
    }

    #[test]
    fn insert_row_into_empty_and_bounds() {
        let mut table = CsvTable::empty(',');
        assert!(table.insert_row(0, vec!["a".into(), "b".into()]));
        assert_eq!(table.to_string(), "a,b\n");
        assert!(!table.insert_row(5, vec![]));
    }

    #[test]
    fn delete_row_keeps_end_of_file_state() {
        let mut table = parse("a\nb\nc");
        assert!(table.delete_row(2));
        assert_eq!(table.to_string(), "a\nb");

        let mut table = parse("a\nb\n");
        assert!(table.delete_row(1));
        assert_eq!(table.to_string(), "a\n");

        let mut table = parse("a\nb\nc");
        assert!(table.delete_row(0));
        assert_eq!(table.to_string(), "b\nc");
        assert!(!table.delete_row(9));
    }

    #[test]
    fn insert_and_delete_columns() {
        let mut table = parse("a,b\n1");
        assert!(table.insert_col(1));
        assert_eq!(table.to_string(), "a,,b\n1,");

        let mut table = parse("a,b,c\n1,2,3");
        assert!(table.delete_col(1));
        assert_eq!(table.to_string(), "a,c\n1,3");
        assert!(!table.delete_col(9));
    }

    #[test]
    fn single_empty_cell_is_not_a_blank_line() {
        let mut table = CsvTable::empty(',');
        assert!(table.insert_col(0));
        assert_eq!(table.to_string(), "\"\"");
        let back = parse(&table.to_string());
        assert_eq!(back.row_count(), 1);
        assert_eq!(back.cell(0, 0), "");
    }

    #[test]
    fn sort_orders_text_numbers_and_empties() {
        let mut table = parse("name,n\nbob,2\nalice,10\n,5\nCarl,x\n");
        assert!(table.sort_rows(0, true));
        assert_eq!(table.to_string(), "name,n\nalice,10\nbob,2\nCarl,x\n,5\n");

        let mut table = parse("name,n\nbob,2\nalice,10\n,5\nCarl,x\n");
        assert!(table.sort_rows(1, true));
        assert_eq!(table.to_string(), "name,n\nbob,2\n,5\nalice,10\nCarl,x\n");
    }

    #[test]
    fn sort_descending_and_already_sorted() {
        let mut table = parse("h\n1\n3\n2\n");
        assert!(table.sort_rows(0, false));
        assert_eq!(table.to_string(), "h\n3\n2\n1\n");
        assert!(!table.sort_rows(0, false), "already sorted");
    }

    #[test]
    fn sort_keeps_record_bytes_and_positional_terminators() {
        let mut table = parse("h\n\"b\"\r\na\n\"c\"");
        assert!(table.sort_rows(0, true));
        assert_eq!(table.to_string(), "h\na\r\n\"b\"\n\"c\"");
    }

    #[test]
    fn sort_needs_data_rows() {
        let mut table = parse("h\n1");
        assert!(!table.sort_rows(0, true));
        assert_eq!(table.to_string(), "h\n1");
    }

    #[test]
    fn column_widths_count_flattened_text() {
        // "x\nyyyy" is drawn as "x yyyy", six cells wide.
        let table = parse("a,b\n\"x\nyyyy\",2");
        assert_eq!(table.column_widths(), vec![6, 1]);
    }

    #[test]
    fn large_table_round_trips_and_edits_one_line() {
        let mut text = String::from("id,name,value\n");
        for i in 0..50_000 {
            text.push_str(&format!("{i},\"name {i}\",{}\n", i * 3));
        }
        let mut table = CsvTable::parse(&text, ',').expect("parses");
        assert_eq!(table.row_count(), 50_001);
        assert_eq!(table.to_string(), text);

        // Record 25_000 is line 25_000 (the header is record 0 / line 0).
        assert!(table.set_cell(25_000, 2, "x"));
        let out = table.to_string();
        let before: Vec<&str> = text.lines().collect();
        let after: Vec<&str> = out.lines().collect();
        assert_eq!(before.len(), after.len());
        let changed: Vec<usize> = (0..before.len()).filter(|&i| before[i] != after[i]).collect();
        assert_eq!(changed, vec![25_000]);
    }

    #[test]
    fn bom_survives_an_edit() {
        let mut table = parse("\u{feff}a,b\n1,2");
        assert!(table.set_cell(1, 0, "9"));
        assert_eq!(table.to_string(), "\u{feff}a,b\n9,2");
    }
}

