//! Pure edit commands for the WYSIWYG markdown editor.
//!
//! The document is a markdown source string. Each command takes the current
//! text and a selection (a byte range; an empty range is a caret) and returns
//! an [`Edit`]: one replacement the editor applies as a single undo step, plus
//! the selection in the NEW text. `None` means "not applicable here": let the
//! default key behaviour run.
//!
//! Command contracts (all offsets are bytes on char boundaries):
//! - `enter`: splits paragraphs with a blank line; continues list items (same
//!   marker, numbers +1, tasks unchecked); Enter on an empty item outdents one
//!   level or removes the marker; continues quotes (`>` line between paragraphs,
//!   empty quote line exits); splits headings (mid-heading splits, end-of-heading
//!   opens a paragraph, empty heading becomes a paragraph); inside fenced code
//!   inserts `\n` plus the line's indentation; in tables moves to the same column
//!   of the next row or appends a row. A non-empty selection is deleted first.
//! - `backspace_at_line_start`: only at a block's content start. Removes a
//!   list marker (outdent when nested), drops one `>`, or removes a heading's
//!   `#`s. Anywhere else (plain paragraph, mid-line, selection) returns `None`.
//! - `tab(shift)`: list items (with nested children) indent under the previous
//!   sibling (by the parent's content column) or outdent under the nearest
//!   shallower item. An item that cannot move returns an identity edit so focus
//!   stays put. Code lines insert/remove up to 2 spaces. Tables and paragraphs
//!   return `None`.
//! - `toggle_inline`: Bold/Italic/Strike/Code. Wraps selection; unwraps when the
//!   markers sit just outside the selection or the selection includes them.
//!   A caret strictly inside a word wraps the word and keeps the caret in place;
//!   any other caret inserts a marker pair with the caret between. Wrapping a
//!   selection strips inner markers of the same kind first (no `****`).
//!   Run-aware: `***x***` is bold+italic, italic toggles the odd layer.
//! - `insert_link` / `paste_url_over_selection`: `[sel](url)`; without url the
//!   caret goes into the `()`. `smart_paste`: URL over selection becomes a link;
//!   multi-line clipboard inside a list item keeps the item's indentation.
//! - `set_heading(level)`, `toggle_list`, `toggle_quote`, `set_paragraph`: apply
//!   to every touched line, keep enclosing containers (quote, list), and toggle
//!   off when all touched lines already match.
//! - `insert_code_block`, `insert_rule`, `insert_table`: block insertion, which
//!   separates from neighbouring text with blank lines. `insert_image` is inline.
//! - `toggle_task`: flips `[ ]`/`[x]` on the line containing the offset.
//! - Table helpers (`table_next_cell`, `table_add_row/col`, `table_delete_row/col`,
//!   `table_reformat`) work on the GFM table containing the caret. Navigation
//!   never reflows; structural edits re-render the whole table with aligned
//!   columns. Widths use [`display_width`] (CJK and emoji wide, Thai combining
//!   marks zero width).
//!
//! Implementation note: commands work on the whole text and the resulting
//! [`Edit`] is the minimal prefix/suffix replacement, so `edit.apply(text)`
//! always equals the intended new document.

#![allow(dead_code)]

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

// ---------------------------------------------------------------- public types

/// One replacement of `range` in the old text by `insert`. `selection` is in
/// the NEW text (after applying the edit).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub range: Range<usize>,
    pub insert: String,
    pub selection: Range<usize>,
}

impl Edit {
    /// Applies the edit, returning the new document.
    pub fn apply(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len() + self.insert.len());
        out.push_str(&text[..self.range.start]);
        out.push_str(&self.insert);
        out.push_str(&text[self.range.end..]);
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineMark {
    Bold,
    Italic,
    Strike,
    Code,
}

impl InlineMark {
    fn marker(self) -> &'static str {
        match self {
            InlineMark::Bold => "**",
            InlineMark::Italic => "*",
            InlineMark::Strike => "~~",
            InlineMark::Code => "`",
        }
    }
    fn ch(self) -> u8 {
        self.marker().as_bytes()[0]
    }
    /// Marker length of the layer this mark owns.
    fn layer(self) -> usize {
        self.marker().len()
    }
    /// Whether a marker run of `run` chars (min of both sides) contains this layer.
    /// Italic shares `*` with bold, so it is the odd layer of a `*` run.
    fn active(self, run: usize) -> bool {
        match self {
            InlineMark::Italic => run % 2 == 1,
            _ => run >= self.layer(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListType {
    Bullet,
    Ordered,
    Task,
}

/// A list marker on a line. Offsets are absolute byte offsets in the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListItem {
    /// Spaces between the enclosing quote markers and the list marker.
    pub indent: usize,
    pub ordered: bool,
    /// Ordered items only (0 for bullets).
    pub number: u64,
    /// Bullet char `-`/`*`/`+`, or ordered delimiter `.`/`)`.
    pub delim: char,
    /// `Some(checked)` when a `[ ]`/`[x]` checkbox follows the marker.
    pub task: Option<bool>,
    pub marker_start: usize,
    /// After the marker and its following space.
    pub marker_end: usize,
    /// After the checkbox (if any): where the item's text starts.
    pub content_start: usize,
    /// Offset of the `[` of the checkbox.
    pub task_pos: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Default,
    Left,
    Center,
    Right,
}

/// Structural analysis of one source line. Produced by [`line_prefix`].
///
/// Layout of a line: `[line_start .. quote_end]` quote markers, then `indent`
/// spaces, then optionally a list item, then optionally a heading marker, then
/// the text. `inner_start` is where the heading (or text) begins after the
/// list item; `content_start` is where the text begins after every marker.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Prefix {
    pub line_start: usize,
    pub line_end: usize,
    pub quote_depth: usize,
    /// End of all `>` markers, including one optional following space.
    pub quote_end: usize,
    /// Spaces between the quote markers and the first non-space char.
    pub indent: usize,
    pub item: Option<ListItem>,
    pub heading: Option<u8>,
    pub inner_start: usize,
    pub content_start: usize,
    /// Nothing but whitespace after `content_start`.
    pub content_empty: bool,
    /// Inside a fenced code block (the opening fence counts; the closing fence does not).
    pub in_code: bool,
    /// This line is a fence delimiter (opening or closing).
    pub fence: bool,
    /// Thematic break (`---`, `***`, `___`).
    pub rule: bool,
    /// Line belongs to a GFM table.
    pub table: bool,
    /// The table's delimiter row (`|---|`).
    pub table_delim: bool,
}

// ---------------------------------------------------------------- byte helpers

fn norm(text: &str, sel: Range<usize>) -> (usize, usize) {
    let mut a = sel.start.min(sel.end).min(text.len());
    let mut b = sel.start.max(sel.end).min(text.len());
    while !text.is_char_boundary(a) {
        a -= 1;
    }
    while !text.is_char_boundary(b) {
        b -= 1;
    }
    (a, b)
}

/// Start and end (exclusive of `\n`) of the line containing byte `pos`.
pub fn line_bounds(text: &str, pos: usize) -> (usize, usize) {
    let pos = pos.min(text.len());
    let ls = text[..pos].rfind('\n').map_or(0, |i| i + 1);
    let le = text[pos..].find('\n').map_or(text.len(), |i| pos + i);
    (ls, le)
}

/// Every line touched by the selection `s..e`. A selection ending exactly at a
/// line start does not include that line.
fn touched_lines(text: &str, s: usize, e: usize) -> Vec<(usize, usize)> {
    let (first, _) = line_bounds(text, s);
    let last = if e > s && text.as_bytes()[e - 1] == b'\n' { e - 1 } else { e };
    let mut out = Vec::new();
    let mut cur = first;
    loop {
        let (ls, le) = line_bounds(text, cur);
        out.push((ls, le));
        if le >= last || le >= text.len() {
            break;
        }
        cur = le + 1;
    }
    out
}

/// Minimal replacement turning `old` into `new`, with `sel` (in `new`) kept.
fn finish(old: &str, new: String, sel: Range<usize>) -> Edit {
    let a = old.as_bytes();
    let b = new.as_bytes();
    let mut pre = a.iter().zip(b.iter()).take_while(|(x, y)| x == y).count();
    while !old.is_char_boundary(pre) || !new.is_char_boundary(pre) {
        pre -= 1;
    }
    let max_suf = a.len().min(b.len()) - pre;
    let mut suf = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take(max_suf)
        .take_while(|(x, y)| x == y)
        .count();
    while !old.is_char_boundary(a.len() - suf) || !new.is_char_boundary(b.len() - suf) {
        suf -= 1;
    }
    let (ns, ne) = norm(&new, sel);
    Edit {
        range: pre..a.len() - suf,
        insert: new[pre..b.len() - suf].to_string(),
        selection: ns..ne,
    }
}

/// Replaces `old_len` bytes starting at `at` by `new_prefix`. Edits must be sorted.
#[derive(Debug, Clone)]
struct LineEdit {
    at: usize,
    old_len: usize,
    new_prefix: String,
}

/// Maps an old offset to the new text. `start` marks a selection start: an
/// insertion exactly at that offset stays before the inserted text, so the
/// selection keeps covering the new marker. Carets (and selection ends) move
/// after it.
fn map_pos(p: usize, edits: &[LineEdit], start: bool) -> usize {
    let mut shift: isize = 0;
    for ed in edits {
        if p < ed.at {
            break;
        }
        let after = if start {
            p > ed.at + ed.old_len || (p == ed.at + ed.old_len && ed.old_len > 0)
        } else {
            p >= ed.at + ed.old_len
        };
        if after {
            shift += ed.new_prefix.len() as isize - ed.old_len as isize;
        } else {
            let o = (p - ed.at).min(ed.new_prefix.len());
            return (ed.at as isize + shift) as usize + o;
        }
    }
    (p as isize + shift) as usize
}

fn apply_edits(text: &str, sel: (usize, usize), edits: &[LineEdit]) -> (String, Range<usize>) {
    let mut out = String::with_capacity(text.len() + 16);
    let mut cur = 0;
    for ed in edits {
        out.push_str(&text[cur..ed.at]);
        out.push_str(&ed.new_prefix);
        cur = ed.at + ed.old_len;
    }
    out.push_str(&text[cur..]);
    let s = if sel.0 == sel.1 {
        map_pos(sel.0, edits, false)
    } else {
        map_pos(sel.0, edits, true)
    };
    (out, s..map_pos(sel.1, edits, false))
}

fn replace_line(text: &str, ls: usize, le: usize, seg: &str) -> String {
    format!("{}{}{}", &text[..ls], seg, &text[le..])
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric()
        || c == '_'
        || matches!(c as u32, 0x0300..=0x036F | 0x0E31 | 0x0E34..=0x0E3A | 0x0E47..=0x0E4E)
}

fn word_bounds(text: &str, pos: usize) -> (usize, usize) {
    let mut ws = pos;
    for (i, c) in text[..pos].char_indices().rev() {
        if is_word_char(c) {
            ws = i;
        } else {
            break;
        }
    }
    let mut we = pos;
    for c in text[pos..].chars() {
        if is_word_char(c) {
            we += c.len_utf8();
        } else {
            break;
        }
    }
    (ws, we)
}

// ---------------------------------------------------------------- graphemes

/// Start of the line containing byte `pos` (the grapheme window never crosses a newline).
fn window_start(text: &str, pos: usize) -> usize {
    text[..pos].rfind('\n').map_or(0, |i| i + 1)
}

/// Start of the extended grapheme cluster before `pos` (0 at the text start).
/// Emoji ZWJ sequences, flags and Thai marks are one cluster.
pub fn prev_grapheme(text: &str, pos: usize) -> usize {
    let pos = norm(text, pos..pos).0;
    let ws = window_start(text, pos);
    if ws == pos {
        // Just after a newline: the newline is the cluster before the caret.
        return text[..pos].char_indices().next_back().map_or(0, |(i, _)| i);
    }
    text[ws..pos]
        .grapheme_indices(true)
        .next_back()
        .map_or(ws, |(i, _)| ws + i)
}

/// End of the extended grapheme cluster after `pos` (text end at the end).
pub fn next_grapheme(text: &str, pos: usize) -> usize {
    let pos = norm(text, pos..pos).0;
    let we = text[pos..].find('\n').map_or(text.len(), |i| pos + i + 1);
    text[pos..we]
        .graphemes(true)
        .next()
        .map_or(pos, |g| pos + g.len())
}

/// The grapheme boundary at or before `pos`: `pos` itself when it is a boundary,
/// else the start of the cluster it falls inside.
pub fn cluster_floor(text: &str, pos: usize) -> usize {
    let pos = norm(text, pos..pos).0;
    if pos == text.len() {
        return pos;
    }
    let ws = window_start(text, pos);
    let mut best = ws;
    for (i, _) in text[ws..].grapheme_indices(true) {
        if ws + i > pos {
            break;
        }
        best = ws + i;
    }
    best
}

/// The grapheme boundary at or after `pos` (the end of the cluster `pos` is in).
pub fn cluster_ceil(text: &str, pos: usize) -> usize {
    let pos = norm(text, pos..pos).0;
    let floor = cluster_floor(text, pos);
    if floor == pos {
        pos
    } else {
        next_grapheme(text, floor)
    }
}

/// Where Backspace starts deleting from `pos`. Emoji and other clusters go
/// whole; Thai removes one code point at a time, the way macOS text fields do
/// (`กิ` then `ก`).
pub fn backspace_start(text: &str, pos: usize) -> usize {
    let start = prev_grapheme(text, pos);
    let pos = norm(text, pos..pos).0;
    let cluster = &text[start..pos];
    if cluster.chars().any(is_thai) {
        if let Some(last) = cluster.chars().next_back() {
            return pos - last.len_utf8();
        }
    }
    start
}

fn is_thai(c: char) -> bool {
    ('\u{0E00}'..='\u{0E7F}').contains(&c)
}

/// Terminal column width of one char: 0 for combining/zero-width, 2 for CJK,
/// Hangul, fullwidth and emoji blocks, 1 otherwise. Thai consonants and vowels
/// are width 1; Thai combining marks (above/below vowels, tone marks) are 0.
pub fn char_width(c: char) -> usize {
    let u = c as u32;
    let zero = matches!(
        u,
        0x0300..=0x036F
            | 0x0E31
            | 0x0E34..=0x0E3A
            | 0x0E47..=0x0E4E
            | 0x1AB0..=0x1AFF
            | 0x20D0..=0x20FF
            | 0x200B..=0x200F
            | 0xFE00..=0xFE0F
            | 0xFEFF
    );
    if zero {
        return 0;
    }
    let wide = matches!(
        u,
        0x1100..=0x115F
            | 0x2E80..=0x303E
            | 0x3041..=0x33FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xA000..=0xA4CF
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE30..=0xFE4F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x1F300..=0x1F64F
            | 0x1F900..=0x1F9FF
            | 0x20000..=0x3FFFD
    );
    if wide {
        2
    } else {
        1
    }
}

/// Display width of a string (sum of [`char_width`]).
pub fn display_width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

// ---------------------------------------------------------------- line scanning

/// Quote markers at the start of `text[ls..le]`: (depth, end offset).
fn scan_quotes(text: &str, ls: usize, le: usize) -> (usize, usize) {
    let b = text.as_bytes();
    let mut pos = ls;
    let mut depth = 0;
    loop {
        let mut q = pos;
        while q < le && q - pos < 3 && b[q] == b' ' {
            q += 1;
        }
        if q < le && b[q] == b'>' {
            depth += 1;
            q += 1;
            if q < le && b[q] == b' ' {
                q += 1;
            }
            pos = q;
        } else {
            break;
        }
    }
    (depth, pos)
}

/// (quote_end, indent) of a line: `indent` spaces after the quote markers.
fn line_indent(text: &str, ls: usize, le: usize) -> (usize, usize) {
    let (_, qe) = scan_quotes(text, ls, le);
    let b = text.as_bytes();
    let mut pos = qe;
    while pos < le && b[pos] == b' ' {
        pos += 1;
    }
    (qe, pos - qe)
}

/// A list marker starting at `pos`, requiring a following space or line end.
fn parse_item(text: &str, pos: usize, le: usize) -> Option<ListItem> {
    if pos >= le {
        return None;
    }
    let b = text.as_bytes();
    let (ordered, number, delim, mlen) = if matches!(b[pos], b'-' | b'*' | b'+')
        && (pos + 1 == le || b[pos + 1] == b' ')
    {
        (false, 0u64, b[pos] as char, 1)
    } else {
        let mut d = pos;
        while d < le && d - pos < 9 && b[d].is_ascii_digit() {
            d += 1;
        }
        if d == pos || d >= le || !(b[d] == b'.' || b[d] == b')') {
            return None;
        }
        if !(d + 1 == le || b[d + 1] == b' ') {
            return None;
        }
        let number: u64 = text[pos..d].parse().ok()?;
        (true, number, b[d] as char, d + 1 - pos)
    };
    let mut marker_end = pos + mlen;
    if marker_end < le {
        marker_end += 1;
    }
    let mut task = None;
    let mut task_pos = None;
    let mut content_start = marker_end;
    if marker_end + 3 <= le
        && b[marker_end] == b'['
        && b[marker_end + 2] == b']'
        && matches!(b[marker_end + 1], b' ' | b'x' | b'X')
        && (marker_end + 3 == le || b[marker_end + 3] == b' ')
    {
        task = Some(b[marker_end + 1] != b' ');
        task_pos = Some(marker_end);
        content_start = marker_end + 3;
        if content_start < le {
            content_start += 1;
        }
    }
    Some(ListItem {
        indent: 0,
        ordered,
        number,
        delim,
        task,
        marker_start: pos,
        marker_end,
        content_start,
        task_pos,
    })
}

/// Fenced code state carried between lines: (fence char, opening run length).
type Fence = Option<(u8, usize)>;

/// For a line body (after quotes and indent): (fence char, run length, rest blank).
fn fence_run(body: &str) -> Option<(u8, usize, bool)> {
    let b = body.as_bytes();
    let c = *b.first()?;
    if c != b'`' && c != b'~' {
        return None;
    }
    let n = b.iter().take_while(|&&x| x == c).count();
    if n < 3 {
        return None;
    }
    let rest = &body[n..];
    if c == b'`' && rest.contains('`') {
        return None;
    }
    Some((c, n, rest.trim().is_empty()))
}

fn is_rule(body: &str) -> bool {
    let t: Vec<char> = body.chars().filter(|c| !c.is_whitespace()).collect();
    t.len() >= 3 && matches!(t[0], '-' | '*' | '_') && t.iter().all(|&c| c == t[0])
}

/// Fence state after consuming `line`.
fn step_fence(state: Fence, line: &str) -> Fence {
    let (_, qe) = scan_quotes(line, 0, line.len());
    let rest = &line[qe..];
    let ind = rest.len() - rest.trim_start_matches(' ').len();
    if ind > 3 {
        return state;
    }
    match (state, fence_run(&rest[ind..])) {
        (None, Some((c, n, _))) => Some((c, n)),
        (Some((c, n)), Some((c2, n2, true))) if c == c2 && n2 >= n => None,
        (st, _) => st,
    }
}

/// Fence state at the start of the line starting at `ls` (a line start).
fn fence_before(text: &str, ls: usize) -> Fence {
    let mut st = None;
    let mut cur = 0;
    while cur < ls {
        let (a, b) = line_bounds(text, cur);
        st = step_fence(st, &text[a..b]);
        if b >= text.len() {
            break;
        }
        cur = b + 1;
    }
    st
}

fn parse_line(text: &str, ls: usize, le: usize, fence: Fence) -> Prefix {
    let b = text.as_bytes();
    let (quote_depth, quote_end) = scan_quotes(text, ls, le);
    let mut pos = quote_end;
    while pos < le && b[pos] == b' ' {
        pos += 1;
    }
    let indent = pos - quote_end;
    let mut p = Prefix {
        line_start: ls,
        line_end: le,
        quote_depth,
        quote_end,
        indent,
        inner_start: pos,
        content_start: pos,
        ..Default::default()
    };
    let body = &text[pos..le];
    let fence_line = if indent <= 3 { fence_run(body) } else { None };
    if let Some((fc, fnum)) = fence {
        p.in_code = true;
        if let Some((c, n, true)) = fence_line {
            if c == fc && n >= fnum {
                p.in_code = false;
                p.fence = true;
            }
        }
    } else if fence_line.is_some() {
        p.fence = true;
        p.in_code = true;
    } else if is_rule(body) {
        p.rule = true;
    } else {
        let mut inner = pos;
        if let Some(mut it) = parse_item(text, pos, le) {
            it.indent = indent;
            inner = it.content_start;
            p.inner_start = inner;
            p.content_start = inner;
            p.item = Some(it);
        }
        let hashes = b[inner..le].iter().take_while(|&&c| c == b'#').count();
        if (1..=6).contains(&hashes) && (inner + hashes == le || b[inner + hashes] == b' ') {
            p.heading = Some(hashes as u8);
            let mut cs = inner + hashes;
            if cs < le {
                cs += 1;
            }
            p.content_start = cs;
        }
    }
    p.content_empty = text[p.content_start..le].trim().is_empty();
    p
}

fn is_pipe_line(line: &str) -> bool {
    !line.trim().is_empty() && line.contains('|')
}

fn is_delim(line: &str) -> bool {
    delim_aligns(line).is_some()
}

/// Parses a GFM delimiter row into per-column alignments.
pub(crate) fn delim_aligns(line: &str) -> Option<Vec<Align>> {
    let t = line.trim();
    if !t.contains('|') || !t.contains('-') {
        return None;
    }
    if !t.chars().all(|c| matches!(c, '|' | ':' | '-' | ' ' | '\t')) {
        return None;
    }
    let mut out = Vec::new();
    for (a, b) in cell_spans_rel(t) {
        let cell = t[a..b].trim();
        if cell.is_empty() || !cell.chars().all(|c| c == '-' || c == ':') || !cell.contains('-') {
            return None;
        }
        out.push(match (cell.starts_with(':'), cell.ends_with(':')) {
            (true, true) => Align::Center,
            (true, false) => Align::Left,
            (false, true) => Align::Right,
            _ => Align::Default,
        });
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Start/end of the table block (its first and last line) containing `ls`,
/// when that block is a GFM table (second line is a delimiter row).
fn table_block(text: &str, ls: usize) -> Option<(usize, usize)> {
    let (_, le) = line_bounds(text, ls);
    if !is_pipe_line(&text[ls..le]) {
        return None;
    }
    let mut start = ls;
    while start > 0 {
        let (pls, ple) = line_bounds(text, start - 1);
        if is_pipe_line(&text[pls..ple]) {
            start = pls;
        } else {
            break;
        }
    }
    let mut end = le;
    while end < text.len() {
        let (nls, nle) = line_bounds(text, end + 1);
        if is_pipe_line(&text[nls..nle]) {
            end = nle;
        } else {
            break;
        }
    }
    let (_, h_e) = line_bounds(text, start);
    if h_e >= end {
        return None;
    }
    let (d_s, d_e) = line_bounds(text, h_e + 1);
    if is_delim(&text[d_s..d_e]) {
        Some((start, end))
    } else {
        None
    }
}

/// Full structural analysis of the line `ls..le`, given the fence state before it.
fn analyze(text: &str, ls: usize, le: usize, fence: Fence) -> Prefix {
    let mut p = parse_line(text, ls, le, fence);
    let line = &text[ls..le];
    if !p.in_code && is_pipe_line(line) && table_block(text, ls).is_some() {
        p.table = true;
        p.table_delim = is_delim(line);
    }
    p
}

/// Structural analysis of the line containing `offset`. Used by the editor to
/// decide list/quote/heading context for rendering and key handling.
pub fn line_prefix(text: &str, offset: usize) -> Prefix {
    let off = norm(text, offset..offset).0;
    let (ls, le) = line_bounds(text, off);
    analyze(text, ls, le, fence_before(text, ls))
}

/// Analyses every line touched by `s..e`. The bool is true for multi-line
/// selections.
fn cands(text: &str, s: usize, e: usize) -> (bool, Vec<Prefix>) {
    let lines = touched_lines(text, s, e);
    let multi = lines.len() > 1;
    let mut fence = fence_before(text, lines[0].0);
    let mut out = Vec::with_capacity(lines.len());
    for (ls, le) in lines {
        out.push(analyze(text, ls, le, fence));
        fence = step_fence(fence, &text[ls..le]);
    }
    (multi, out)
}

/// Lines a block-level command may rewrite (not code, fences, tables or rules).
fn usable(p: &Prefix) -> bool {
    !(p.in_code || p.fence || p.table || p.rule)
}

fn marker_text(it: &ListItem) -> String {
    let mut s = if it.ordered {
        format!("{}{} ", it.number, it.delim)
    } else {
        format!("{} ", it.delim)
    };
    match it.task {
        Some(true) => s.push_str("[x] "),
        Some(false) => s.push_str("[ ] "),
        None => {}
    }
    s
}

/// Marker for the item created by Enter: next number, unchecked task.
fn next_marker(it: &ListItem) -> String {
    let mut s = if it.ordered {
        format!("{}{} ", it.number + 1, it.delim)
    } else {
        format!("{} ", it.delim)
    };
    if it.task.is_some() {
        s.push_str("[ ] ");
    }
    s
}

/// Indent of the nearest previous item shallower than `indent`, else 0.
/// Stops at a top-level non-item line.
fn parent_indent(text: &str, ls: usize, indent: usize) -> usize {
    let mut cur = ls;
    while cur > 0 {
        let (pls, ple) = line_bounds(text, cur - 1);
        cur = pls;
        if text[pls..ple].trim().is_empty() {
            continue;
        }
        let (qe, ind) = line_indent(text, pls, ple);
        if parse_item(text, qe + ind, ple).is_some() {
            if ind < indent {
                return ind;
            }
        } else if ind == 0 {
            return 0;
        }
    }
    0
}

/// Content column (indent + marker width) that an item at `indent` gets when
/// nested under the previous sibling or parent. `None` if there is none.
fn prev_sibling_col(text: &str, ls: usize, indent: usize) -> Option<usize> {
    let mut cur = ls;
    while cur > 0 {
        let (pls, ple) = line_bounds(text, cur - 1);
        cur = pls;
        if text[pls..ple].trim().is_empty() {
            continue;
        }
        let (qe, ind) = line_indent(text, pls, ple);
        match parse_item(text, qe + ind, ple) {
            Some(it) => {
                if ind <= indent {
                    return Some(ind + it.marker_end - it.marker_start);
                }
            }
            None => {
                if ind == 0 {
                    return None;
                }
            }
        }
    }
    None
}

/// The item line at `ls..le` plus following lines indented deeper than `indent`.
fn subtree(text: &str, ls: usize, le: usize, indent: usize) -> Vec<(usize, usize)> {
    let mut out = vec![(ls, le)];
    let mut cur = le;
    while cur < text.len() {
        let (nls, nle) = line_bounds(text, cur + 1);
        if text[nls..nle].trim().is_empty() {
            cur = nle;
            continue;
        }
        let (_, ind) = line_indent(text, nls, nle);
        if ind <= indent {
            break;
        }
        out.push((nls, nle));
        cur = nle;
    }
    out
}

// ---------------------------------------------------------------- Enter / Backspace / Tab

/// Enter at the caret (or replacing the selection). See the module docs.
pub fn enter(text: &str, sel: Range<usize>) -> Option<Edit> {
    let (s, e) = norm(text, sel);
    let base = format!("{}{}", &text[..s], &text[e..]);
    let (new, caret) = enter_core(&base, s)?;
    Some(finish(text, new, caret..caret))
}

fn enter_core(base: &str, c: usize) -> Option<(String, usize)> {
    let (ls, le) = line_bounds(base, c);
    let p = analyze(base, ls, le, fence_before(base, ls));
    if p.table {
        return table_enter(base, c, &p);
    }
    if p.in_code {
        if c < p.content_start {
            return None;
        }
        // Enter at the end of an opening fence with no closing fence after it:
        // create the empty block with its closing fence, caret inside.
        if p.fence && base[c..le].trim().is_empty() {
            let fence_state = step_fence(fence_before(base, ls), &base[ls..le]);
            if !closed_after(base, le, fence_state) {
                if let Some((fc, fnum, _)) = fence_run(&base[p.inner_start..le]) {
                    let lead = base[ls..p.inner_start].to_string();
                    let closing = (fc as char).to_string().repeat(fnum);
                    let seg = format!("{}\n{}\n{}{}", &base[ls..c], lead, lead, closing);
                    let caret = ls + (c - ls) + 1 + lead.len();
                    return Some((replace_line(base, ls, le, &seg), caret));
                }
            }
        }
        let lead = base[ls..p.content_start].to_string();
        let seg = format!("{}\n{}{}", &base[ls..c], lead, &base[c..le]);
        let caret = ls + (c - ls) + 1 + lead.len();
        return Some((replace_line(base, ls, le, &seg), caret));
    }
    if p.quote_depth > 0 && p.content_empty && p.item.is_none() && p.heading.is_none() {
        if c < p.quote_end {
            return None;
        }
        let last = ls + base[ls..p.quote_end].rfind('>')?;
        let prefix = &base[ls..last];
        if prefix.trim().is_empty() && ls > 0 {
            // Enter inside a quote leaves a bare `>` separator above the new
            // line. Exiting drops it too, so the quote ends with one blank line.
            let (pls, ple) = line_bounds(base, ls - 1);
            if base[pls..ple].trim() == ">" {
                let seg = if le == base.len() { "\n" } else { "" };
                let new = format!("{}{}{}", &base[..pls], seg, &base[le..]);
                return Some((new, pls + seg.len()));
            }
        }
        return Some(exit_block(base, ls, le, prefix));
    }
    if p.heading.is_some() {
        if c < p.content_start {
            return None;
        }
        if p.content_empty {
            let seg = base[ls..p.inner_start].to_string();
            let caret = ls + seg.len();
            return Some((replace_line(base, ls, le, &seg), caret));
        }
        let cont = match &p.item {
            Some(it) => it.indent + it.marker_end - it.marker_start,
            None => p.indent,
        };
        return Some(split_block(base, ls, le, c, &p, cont));
    }
    if let Some(it) = p.item.clone() {
        if c < p.content_start {
            return None;
        }
        let qp = &base[ls..p.quote_end];
        if p.content_empty {
            if it.indent > 0 {
                let ni = parent_indent(base, ls, it.indent);
                let seg = format!("{}{}{}", qp, " ".repeat(ni), marker_text(&it));
                let caret = ls + seg.len();
                return Some((replace_line(base, ls, le, &seg), caret));
            }
            return Some(exit_block(base, ls, le, qp));
        }
        let before = &base[ls..c];
        let after = base[c..le].trim_start();
        let cont = format!("{}{}{}", qp, " ".repeat(it.indent), next_marker(&it));
        let seg = format!("{}\n{}{}", before, cont, after);
        let caret = ls + before.len() + 1 + cont.len();
        // Ordered siblings below shift up by one: renumber them so the list stays 1, 2, 3.
        let mut edits = vec![LineEdit {
            at: ls,
            old_len: le - ls,
            new_prefix: seg,
        }];
        if it.ordered {
            edits.extend(renumber_edits(base, le, qp, it.indent, it.delim, it.number + 2));
        }
        let (new, _) = apply_edits(base, (c, c), &edits);
        return Some((new, caret));
    }
    if p.content_empty && p.quote_depth == 0 && p.indent == 0 {
        // Empty plain line: just open another line.
        return Some((replace_line(base, ls, le, "\n"), ls + 1));
    }
    Some(split_block(base, ls, le, c, &p, p.indent))
}

/// Whether the line before the one starting at `ls` has text. A block that
/// starts right after text would be read as its lazy continuation, so it needs
/// a blank line between them.
fn needs_blank_above(text: &str, ls: usize) -> bool {
    if ls == 0 {
        return false;
    }
    let (pls, ple) = line_bounds(text, ls - 1);
    !text[pls..ple].trim().is_empty()
}

/// Whether the fence opened with `state` (the state after its opening line)
/// is closed by some line after `le`.
fn closed_after(text: &str, le: usize, mut state: Fence) -> bool {
    let mut cur = le;
    while cur < text.len() {
        let (ls, nle) = line_bounds(text, cur + 1);
        state = step_fence(state, &text[ls..nle]);
        if state.is_none() {
            return true;
        }
        cur = nle;
    }
    false
}

/// Replaces the empty line `ls..le` with `prefix` (the container left after
/// an exit: a paragraph, or a quote line). At the end of the document, a
/// paragraph that follows text gets a blank line first. Without the blank
/// line, typing the next paragraph would continue the list or quote above.
fn exit_block(base: &str, ls: usize, le: usize, prefix: &str) -> (String, usize) {
    let seg = if le == base.len() && needs_blank_above(base, ls) {
        format!("{}\n{}", prefix.trim_end(), prefix)
    } else {
        prefix.to_string()
    };
    let caret = ls + seg.len();
    (replace_line(base, ls, le, &seg), caret)
}

/// Splits a paragraph-like line at `c` into two blocks separated by a blank
/// line. Quotes get a bare `>` between the two parts; the continuation is
/// indented by `cont_indent` after the quote markers.
fn split_block(base: &str, ls: usize, le: usize, c: usize, p: &Prefix, cont_indent: usize) -> (String, usize) {
    let qp = &base[ls..p.quote_end];
    let before = &base[ls..c];
    let after = base[c..le].trim_start();
    let blank = qp.trim_end();
    let cont = format!("{}{}", qp, " ".repeat(cont_indent));
    let seg = format!("{}\n{}\n{}{}", before, blank, cont, after);
    let caret = ls + before.len() + 1 + blank.len() + 1 + cont.len();
    (replace_line(base, ls, le, &seg), caret)
}

/// Numbers of the ordered siblings below the item whose line ends at `le`.
/// They must follow `next`, `next + 1`, ... Only items of the same list (same
/// quote, indent and delimiter) are renumbered; nested content is skipped and
/// the first line that leaves the list ends the scan.
fn renumber_edits(text: &str, le: usize, qp: &str, indent: usize, delim: char, mut next: u64) -> Vec<LineEdit> {
    let mut out = Vec::new();
    let mut cur = le;
    while cur < text.len() {
        let (ls, nle) = line_bounds(text, cur + 1);
        cur = nle;
        if text[ls..nle].trim().is_empty() {
            continue;
        }
        let (qe, ind) = line_indent(text, ls, nle);
        if text[ls..qe] != *qp {
            break;
        }
        if ind > indent {
            continue;
        }
        if ind < indent {
            break;
        }
        let pos = qe + ind;
        match parse_item(text, pos, nle) {
            Some(it) if it.ordered && it.delim == delim => {
                if it.number != next {
                    let digits = text[pos..nle].bytes().take_while(u8::is_ascii_digit).count();
                    out.push(LineEdit {
                        at: pos,
                        old_len: digits,
                        new_prefix: next.to_string(),
                    });
                }
                next += 1;
            }
            _ => break,
        }
    }
    out
}

/// Enter inside a GFM table: move to the same column of the next row, or
/// append a row after the last one.
fn table_enter(base: &str, c: usize, p: &Prefix) -> Option<(String, usize)> {
    let ctx = table_ctx(base, c)?;
    let mut cur_le = p.line_end;
    while cur_le < ctx.end {
        let (nls, nle) = line_bounds(base, cur_le + 1);
        let line = &base[nls..nle];
        if !is_delim(line) {
            let spans = cell_spans_rel(line);
            if spans.is_empty() {
                return None;
            }
            let (a, _) = trim_span(line, spans[ctx.raw_col.min(spans.len() - 1)]);
            return Some((base.to_string(), nls + a));
        }
        cur_le = nle;
    }
    let mut t = ctx.table.clone();
    let n = t.align.len();
    t.body.push(vec![String::new(); n]);
    let target = Some(t.body.len() - 1);
    let edit = rebuild(ctx.start, ctx.end, &t, target, ctx.col, 0, 0);
    Some((edit.apply(base), edit.selection.start))
}

/// Backspace when the caret is at a block's content start. See the module docs.
pub fn backspace_at_line_start(text: &str, sel: Range<usize>) -> Option<Edit> {
    let (s, e) = norm(text, sel);
    if s != e {
        return None;
    }
    let p = line_prefix(text, s);
    if p.in_code && !p.fence && s == p.quote_end && p.line_start > 0 {
        // First body line of a fence: joining it onto the opening line would
        // corrupt the info string (```` ```js ````). Stay at the info line's end.
        let (pls, ple) = line_bounds(text, p.line_start - 1);
        let above = analyze(text, pls, ple, fence_before(text, pls));
        if above.fence && above.in_code {
            return Some(Edit {
                range: s..s,
                insert: String::new(),
                selection: ple..ple,
            });
        }
    }
    if p.in_code || p.fence || p.table || p.rule {
        return None;
    }
    let (ls, le) = (p.line_start, p.line_end);
    if p.heading.is_some() {
        if s != p.content_start {
            return None;
        }
        let seg = format!("{}{}", &text[ls..p.inner_start], &text[p.content_start..le]);
        let c = ls + (p.inner_start - ls);
        return Some(finish(text, replace_line(text, ls, le, &seg), c..c));
    }
    if let Some(it) = &p.item {
        if s != p.content_start {
            return None;
        }
        let qp = &text[ls..p.quote_end];
        if it.indent == 0 && needs_blank_above(text, ls) {
            // Lifting the item to a paragraph (or quote line) right after text:
            // keep a blank line so it does not continue the list above.
            let blank = qp.trim_end();
            let seg = format!("{}\n{}{}", blank, qp, &text[p.content_start..le]);
            let c = ls + blank.len() + 1 + qp.len();
            return Some(finish(text, replace_line(text, ls, le, &seg), c..c));
        }
        let lead = if it.indent > 0 {
            format!("{}{}{}", qp, " ".repeat(parent_indent(text, ls, it.indent)), marker_text(it))
        } else {
            qp.to_string()
        };
        let seg = format!("{}{}", lead, &text[p.content_start..le]);
        let c = ls + lead.len();
        return Some(finish(text, replace_line(text, ls, le, &seg), c..c));
    }
    if p.quote_depth > 0 && s == p.quote_end {
        let last = ls + text[ls..p.quote_end].rfind('>')?;
        let prefix = &text[ls..last];
        if prefix.trim().is_empty() && needs_blank_above(text, ls) {
            let seg = format!("\n{}", &text[p.quote_end..le]);
            let c = ls + 1;
            return Some(finish(text, replace_line(text, ls, le, &seg), c..c));
        }
        let seg = format!("{}{}", prefix, &text[p.quote_end..le]);
        return Some(finish(text, replace_line(text, ls, le, &seg), last..last));
    }
    None
}


/// Tab (indent) or Shift-Tab (outdent). See the module docs for the contract.
pub fn tab(text: &str, sel: Range<usize>, shift: bool) -> Option<Edit> {
    let (s, e) = norm(text, sel);
    let lines = touched_lines(text, s, e);
    let mut fence = fence_before(text, lines[0].0);
    let mut infos = Vec::with_capacity(lines.len());
    for &(ls, le) in &lines {
        let p = analyze(text, ls, le, fence);
        fence = step_fence(fence, &text[ls..le]);
        if p.table {
            return None;
        }
        infos.push(p);
    }
    let any_item = infos.iter().any(|p| p.item.is_some() && !p.in_code);
    let any_code = infos.iter().any(|p| p.in_code);
    if !any_item && !any_code {
        return None;
    }
    let mut edits: Vec<LineEdit> = Vec::new();
    if any_item {
        let mut covered = 0usize;
        for p in &infos {
            if p.line_start < covered || p.in_code {
                continue;
            }
            let it = match &p.item {
                Some(it) => it.clone(),
                None => continue,
            };
            let sub = subtree(text, p.line_start, p.line_end, it.indent);
            covered = sub.last().map_or(p.line_end, |x| x.1);
            if shift {
                if it.indent == 0 {
                    continue;
                }
                let ni = parent_indent(text, p.line_start, it.indent);
                if ni >= it.indent {
                    continue;
                }
                let out = it.indent - ni;
                for &(ls, le) in &sub {
                    let (qe, ind) = line_indent(text, ls, le);
                    let k = out.min(ind);
                    if k > 0 {
                        edits.push(LineEdit { at: qe, old_len: k, new_prefix: String::new() });
                    }
                }
            } else {
                let target = match prev_sibling_col(text, p.line_start, it.indent) {
                    Some(t) if t > it.indent => t,
                    _ => continue,
                };
                let add = target - it.indent;
                for &(ls, le) in &sub {
                    let (qe, _) = line_indent(text, ls, le);
                    edits.push(LineEdit { at: qe, old_len: 0, new_prefix: " ".repeat(add) });
                }
            }
        }
    } else {
        for p in &infos {
            if !p.in_code {
                continue;
            }
            if shift {
                let k = p.indent.min(2);
                if k > 0 {
                    edits.push(LineEdit { at: p.quote_end, old_len: k, new_prefix: String::new() });
                }
            } else {
                let at = if s == e { s.max(p.quote_end) } else { p.quote_end };
                edits.push(LineEdit { at, old_len: 0, new_prefix: "  ".to_string() });
            }
        }
    }
    let (new, nsel) = apply_edits(text, (s, e), &edits);
    Some(finish(text, new, nsel))
}

// ---------------------------------------------------------------- inline marks

/// Toggles an inline mark on the selection (or the word at the caret).
///
/// A caret strictly inside a word (word characters on both sides) wraps that
/// word and keeps the caret where it was. A caret anywhere else inserts an
/// empty marker pair with the caret between the markers. Wrapping a selection
/// that already holds the same mark strips the inner markers first, so the
/// result never nests `****`.
pub fn toggle_inline(text: &str, sel: Range<usize>, mark: InlineMark) -> Option<Edit> {
    let (mut s, mut e) = norm(text, sel);
    let m = mark.marker();
    let l = mark.layer();
    let ch = mark.ch();
    let b = text.as_bytes();
    let mut caret_in_word: Option<usize> = None;
    if s == e {
        if let Some((o, c)) = caret_pair(text, s, mark) {
            // The caret sits in, or right against, an existing span: toggle it off.
            let new = format!("{}{}{}", &text[..o], &text[o + l..c], &text[c + l..]);
            let mut caret = s;
            if s > o {
                caret -= l;
            }
            if s > c {
                caret -= l;
            }
            return Some(finish(text, new, caret..caret));
        }
        let inside = match (text[..s].chars().next_back(), text[s..].chars().next()) {
            (Some(before), Some(after)) => is_word_char(before) && is_word_char(after),
            _ => false,
        };
        if inside {
            let (ws, we) = word_bounds(text, s);
            caret_in_word = Some(s - ws);
            s = ws;
            e = we;
        } else {
            let new = format!("{}{}{}{}", &text[..s], m, m, &text[s..]);
            let c = s + m.len();
            return Some(finish(text, new, c..c));
        }
    }
    let before = b[..s].iter().rev().take_while(|&&x| x == ch).count();
    let after = b[e..].iter().take_while(|&&x| x == ch).count();
    let mut edit = if mark.active(before.min(after)) {
        let new = format!("{}{}{}", &text[..s - l], &text[s..e], &text[e + l..]);
        finish(text, new, s - l..e - l)
    } else {
        let lead = b[s..e].iter().take_while(|&&x| x == ch).count();
        let trail = b[s..e].iter().rev().take_while(|&&x| x == ch).count();
        if e - s > 2 * l && mark.active(lead.min(trail)) {
            let new = format!("{}{}{}", &text[..s], &text[s + l..e - l], &text[e..]);
            finish(text, new, s..e - 2 * l)
        } else {
            let inner = strip_inner_marks(mark, &text[s..e]);
            let new = format!("{}{}{}{}{}", &text[..s], m, inner, m, &text[e..]);
            let start = s + l;
            finish(text, new, start..start + inner.len())
        }
    };
    if let Some(rel) = caret_in_word {
        let c = edit.selection.start + rel;
        edit.selection = c..c;
    }
    Some(edit)
}

/// Inner markers of the same kind, removed before wrapping a selection. Only
/// an even count is stripped (they pair up as old formatting); a lone marker
/// is literal text and stays. Italic only strips single `*` runs, so bold
/// inside italic text nests as `***x***`.
fn strip_inner_marks(mark: InlineMark, inner: &str) -> String {
    match mark {
        InlineMark::Italic => {
            let singles: Vec<usize> = {
                let b = inner.as_bytes();
                let mut out = Vec::new();
                let mut i = 0;
                while i < b.len() {
                    if b[i] == b'*' {
                        let run_start = i;
                        while i < b.len() && b[i] == b'*' {
                            i += 1;
                        }
                        if i - run_start == 1 {
                            out.push(run_start);
                        }
                    } else {
                        i += 1;
                    }
                }
                out
            };
            if singles.len() < 2 || singles.len() % 2 != 0 {
                return inner.to_string();
            }
            let mut out = String::with_capacity(inner.len());
            let mut last = 0;
            for at in singles {
                out.push_str(&inner[last..at]);
                last = at + 1;
            }
            out.push_str(&inner[last..]);
            out
        }
        _ => {
            let m = mark.marker();
            let count = inner.matches(m).count();
            if count >= 2 && count % 2 == 0 {
                inner.replace(m, "")
            } else {
                inner.to_string()
            }
        }
    }
}


fn escape_label(s: &str) -> String {
    s.replace('[', "\\[").replace(']', "\\]")
}

/// The marker pair of `mark` on the caret's line that contains the caret
/// (or touches it: directly after the closing or before the opening marker).
/// Markers are runs of exactly the mark's length, paired left to right.
fn caret_pair(text: &str, s: usize, mark: InlineMark) -> Option<(usize, usize)> {
    let (ls, le) = line_bounds(text, s);
    let l = mark.layer();
    let ch = mark.ch();
    let b = text.as_bytes();
    let mut runs = Vec::new();
    let mut i = ls;
    while i < le {
        if b[i] == ch {
            let st = i;
            while i < le && b[i] == ch {
                i += 1;
            }
            if i - st == l {
                runs.push(st);
            }
        } else {
            i += 1;
        }
    }
    runs.chunks_exact(2)
        .map(|pair| (pair[0], pair[1]))
        .find(|&(o, c)| o <= s && s <= c + l)
}

/// Joins a paragraph with the paragraph after one blank line when the caret
/// is at the end of the first (forward Delete). Both newlines go, as in
/// ProseMirror, so `ab⏎⏎cd` becomes `abcd`. Other blocks keep the default.
pub fn join_paragraphs_forward(text: &str, sel: Range<usize>) -> Option<Edit> {
    let (s, e) = norm(text, sel);
    if s != e {
        return None;
    }
    let (_, le) = line_bounds(text, s);
    if s != le || !text[le..].starts_with("\n\n") {
        return None;
    }
    let next = le + 2;
    let (nls, nle) = line_bounds(text, next);
    if nls != next || nle == nls {
        return None;
    }
    if !is_plain_paragraph(&line_prefix(text, s)) || !is_plain_paragraph(&line_prefix(text, next)) {
        return None;
    }
    let new = format!("{}{}", &text[..le], &text[le + 2..]);
    Some(finish(text, new, le..le))
}

fn is_plain_paragraph(p: &Prefix) -> bool {
    p.quote_depth == 0
        && p.item.is_none()
        && p.heading.is_none()
        && !(p.in_code || p.fence || p.rule || p.table)
        && p.indent == 0
        && !p.content_empty
}

/// When the document ends with a closing fence, a rule or a table, appends a
/// newline so a line after the block exists for the caret. `None` otherwise.
pub fn ensure_trailing_line(text: &str) -> Option<Edit> {
    if text.is_empty() || text.ends_with('\n') {
        return None;
    }
    let p = line_prefix(text, text.len());
    let closing_fence = p.fence && !p.in_code;
    if !(closing_fence || p.rule || p.table) {
        return None;
    }
    let end = text.len();
    Some(Edit {
        range: end..end,
        insert: "\n".to_string(),
        selection: end + 1..end + 1,
    })
}

/// End of the line at `pos` when that line is a closing fence or a rule. Text
/// typed on such a line must go on a new line after it, never onto the fence.
pub fn block_guard_line_end(text: &str, pos: usize) -> Option<usize> {
    let p = line_prefix(text, pos);
    ((p.fence && !p.in_code) || p.rule).then_some(p.line_end)
}

fn link_dest(u: &str) -> String {
    if u.chars().any(|c| c.is_whitespace() || c == '(' || c == ')') {
        format!("<{}>", u)
    } else {
        u.to_string()
    }
}

/// `[sel](url)`. With `url == None` the caret goes into the empty `()`.
/// With a selection and a url, the link text is selected afterwards.
pub fn insert_link(text: &str, sel: Range<usize>, url: Option<&str>) -> Option<Edit> {
    let (mut s, mut e) = norm(text, sel);
    // Whitespace at the edges of the selection stays outside the link text.
    let inner = &text[s..e];
    if !inner.trim().is_empty() {
        let lead = inner.len() - inner.trim_start().len();
        let trail = inner.len() - inner.trim_end().len();
        s += lead;
        e -= trail;
    }
    let url = url.map(str::trim).filter(|u| !u.is_empty());
    // With no selection the URL itself is the link text.
    let label = match url {
        Some(u) if s == e => escape_label(u),
        _ => escape_label(&text[s..e]),
    };
    match url {
        Some(u) => {
            let new = format!("{}[{}]({}){}", &text[..s], label, link_dest(u), &text[e..]);
            Some(finish(text, new, s + 1..s + 1 + label.len()))
        }
        None => {
            let new = format!("{}[{}](){}", &text[..s], label, &text[e..]);
            let c = s + 1 + label.len() + 2;
            Some(finish(text, new, c..c))
        }
    }
}

/// Whether `s` is a link-able URL: http(s):// or mailto: with no whitespace.
pub fn is_url(s: &str) -> bool {
    let schemes = ["http://", "https://", "mailto:"];
    schemes.iter().any(|p| s.len() > p.len() && s.starts_with(p)) && !s.chars().any(char::is_whitespace)
}

/// Pasting a URL over a non-empty selection makes it a link. Otherwise `None`.
pub fn paste_url_over_selection(text: &str, sel: Range<usize>, clipboard: &str) -> Option<Edit> {
    let (s, e) = norm(text, sel);
    let u = clipboard.trim();
    if s == e || !is_url(u) {
        return None;
    }
    insert_link(text, s..e, Some(u))
}

// ---------------------------------------------------------------- block commands

/// Sets the ATX heading level on every touched line (`None` removes headings).
/// If every touched line already has `level`, it is removed (toggle). Enclosing
/// quotes and list markers stay.
pub fn set_heading(text: &str, sel: Range<usize>, level: Option<u8>) -> Option<Edit> {
    if let Some(n) = level {
        if !(1..=6).contains(&n) {
            return None;
        }
    }
    let (s, e) = norm(text, sel);
    let (multi, cs) = cands(text, s, e);
    let lines: Vec<Prefix> = cs
        .into_iter()
        .filter(|p| usable(p) && !(multi && p.content_empty))
        .collect();
    if lines.is_empty() {
        return None;
    }
    let remove = match level {
        None => true,
        Some(n) => lines.iter().all(|p| p.heading == Some(n)),
    };
    let edits: Vec<LineEdit> = lines
        .iter()
        .map(|p| {
            let keep = &text[p.line_start..p.inner_start];
            let new_prefix = match (remove, level) {
                (false, Some(n)) => format!("{}{} ", keep, "#".repeat(n as usize)),
                _ => keep.to_string(),
            };
            LineEdit {
                at: p.line_start,
                old_len: p.content_start - p.line_start,
                new_prefix,
            }
        })
        .collect();
    let (new, nsel) = apply_edits(text, (s, e), &edits);
    Some(finish(text, new, nsel))
}

/// Converts touched lines to a list of `kind`, or removes the list markers when
/// every touched line already is one. Ordered items are numbered 1, 2, 3…
pub fn toggle_list(text: &str, sel: Range<usize>, kind: ListType) -> Option<Edit> {
    let (s, e) = norm(text, sel);
    let (multi, cs) = cands(text, s, e);
    let lines: Vec<Prefix> = cs
        .into_iter()
        .filter(|p| usable(p) && !(multi && p.content_empty))
        .collect();
    if lines.is_empty() {
        return None;
    }
    let same = |p: &Prefix| match (&p.item, kind) {
        (Some(it), ListType::Bullet) => !it.ordered && it.task.is_none(),
        (Some(it), ListType::Ordered) => it.ordered,
        (Some(it), ListType::Task) => it.task.is_some(),
        _ => false,
    };
    let remove = lines.iter().all(same);
    let mut n = 0u64;
    let mut edits = Vec::with_capacity(lines.len());
    for p in &lines {
        let qp = &text[p.line_start..p.quote_end];
        if remove {
            if let Some(it) = &p.item {
                edits.push(LineEdit {
                    at: p.line_start,
                    old_len: it.content_start - p.line_start,
                    new_prefix: format!("{}{}", qp, " ".repeat(it.indent)),
                });
            }
        } else {
            n += 1;
            let marker = match kind {
                ListType::Bullet => "- ".to_string(),
                ListType::Ordered => format!("{}. ", n),
                ListType::Task => "- [ ] ".to_string(),
            };
            let (indent, old_len) = match &p.item {
                Some(it) => (it.indent, it.content_start - p.line_start),
                None => (p.indent, p.content_start - p.line_start),
            };
            edits.push(LineEdit {
                at: p.line_start,
                old_len,
                new_prefix: format!("{}{}{}", qp, " ".repeat(indent), marker),
            });
        }
    }
    let (new, nsel) = apply_edits(text, (s, e), &edits);
    Some(finish(text, new, nsel))
}

/// Adds a `> ` quote to every touched line, or removes the outer `>` when all
/// non-blank touched lines are already quoted. Blank lines inside a multi-line
/// quote get a bare `>` so the quote stays one block.
pub fn toggle_quote(text: &str, sel: Range<usize>) -> Option<Edit> {
    let (s, e) = norm(text, sel);
    let (multi, cs) = cands(text, s, e);
    let lines: Vec<Prefix> = cs
        .into_iter()
        .filter(|p| !(p.in_code || p.fence || p.table))
        .collect();
    if lines.is_empty() {
        return None;
    }
    let judged: Vec<&Prefix> = if multi {
        lines.iter().filter(|p| !p.content_empty).collect()
    } else {
        lines.iter().collect()
    };
    let remove = !judged.is_empty() && judged.iter().all(|p| p.quote_depth > 0);
    let mut edits = Vec::with_capacity(lines.len());
    for p in &lines {
        if remove {
            if p.quote_depth == 0 {
                continue;
            }
            let gt = p.line_start + text[p.line_start..p.quote_end].find('>')?;
            let sp = usize::from(gt + 1 < p.quote_end && text.as_bytes()[gt + 1] == b' ');
            edits.push(LineEdit {
                at: p.line_start,
                old_len: gt + 1 + sp - p.line_start,
                new_prefix: text[p.line_start..gt].to_string(),
            });
        } else if p.content_empty {
            if p.quote_depth > 0 {
                continue;
            }
            let marker = if lines.len() == 1 { "> " } else { ">" };
            edits.push(LineEdit {
                at: p.line_start,
                old_len: p.line_end - p.line_start,
                new_prefix: marker.to_string(),
            });
        } else {
            edits.push(LineEdit {
                at: p.line_start,
                old_len: 0,
                new_prefix: "> ".to_string(),
            });
        }
    }
    if edits.is_empty() {
        return None;
    }
    let (new, nsel) = apply_edits(text, (s, e), &edits);
    Some(finish(text, new, nsel))
}

/// Strips every block container (quote, list marker, heading) from touched lines.
pub fn set_paragraph(text: &str, sel: Range<usize>) -> Option<Edit> {
    let (s, e) = norm(text, sel);
    let (_, cs) = cands(text, s, e);
    if !cs.iter().any(usable) {
        return None;
    }
    let edits: Vec<LineEdit> = cs
        .iter()
        .filter(|p| usable(p) && p.content_start > p.line_start)
        .map(|p| LineEdit {
            at: p.line_start,
            old_len: p.content_start - p.line_start,
            new_prefix: String::new(),
        })
        .collect();
    let (new, nsel) = apply_edits(text, (s, e), &edits);
    Some(finish(text, new, nsel))
}

/// Block insertion at the caret. A non-empty selection (`s..e`) is replaced.
/// Blank lines separate the block from neighbouring text. `caret_in_body` is
/// the caret offset inside `body`.
pub fn block_edit(text: &str, s: usize, e: usize, body: &str, caret_in_body: usize) -> Edit {
    let before = &text[..s];
    let lead = if before.is_empty() || before.ends_with("\n\n") {
        ""
    } else if before.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    let after = &text[e..];
    let trail = if after.is_empty() {
        "\n"
    } else if after.starts_with("\n\n") {
        ""
    } else if after.starts_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    let caret = s + lead.len() + caret_in_body;
    Edit {
        range: s..e,
        insert: format!("{}{}{}", lead, body, trail),
        selection: caret..caret,
    }
}

/// Fenced code block with `lang`. A selection becomes the block's body.
pub fn insert_code_block(text: &str, sel: Range<usize>, lang: &str) -> Option<Edit> {
    let (s, e) = norm(text, sel);
    let head = format!("```{}\n", lang.trim());
    let body = format!("{}{}\n```", head, &text[s..e]);
    Some(block_edit(text, s, e, &body, head.len()))
}

/// Thematic break `---` as its own block. The caret goes to the line after
/// it, so typing does not land on the rule.
pub fn insert_rule(text: &str, sel: Range<usize>) -> Option<Edit> {
    let (s, e) = norm(text, sel);
    Some(block_edit(text, s, e, "---", 4))
}

/// Renders a template GFM table with `rows` rows and `cols` columns.
pub fn table_template(rows: usize, cols: usize) -> (String, Vec<Vec<usize>>) {
    let t = Table {
        header: vec![String::new(); cols],
        align: vec![Align::Default; cols],
        body: vec![vec![String::new(); cols]; rows.saturating_sub(1)],
    };
    render(&t)
}

/// GFM table with `rows` rows (header included) and `cols` columns, empty cells,
/// caret in the first header cell.
pub fn insert_table(text: &str, sel: Range<usize>, rows: usize, cols: usize) -> Option<Edit> {
    if rows == 0 || cols == 0 {
        return None;
    }
    let (rendered, pos) = table_template(rows, cols);
    let (s, e) = norm(text, sel);
    Some(block_edit(text, s, e, &rendered, pos[0][0]))
}

/// Inline image `![alt](url)` replacing the selection. An empty `alt` uses the
/// selected text. The caret goes after the image.
pub fn insert_image(text: &str, sel: Range<usize>, alt: &str, url: &str) -> Option<Edit> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    let (s, e) = norm(text, sel);
    let alt_text: &str = if alt.is_empty() { &text[s..e] } else { alt };
    let img = format!("![{}]({})", escape_label(alt_text), link_dest(url));
    let new = format!("{}{}{}", &text[..s], img, &text[e..]);
    let c = s + img.len();
    Some(finish(text, new, c..c))
}

/// Flips the checkbox of the task item on the line containing `offset`.
pub fn toggle_task(text: &str, offset: usize) -> Option<Edit> {
    let off = norm(text, offset..offset).0;
    let p = line_prefix(text, off);
    if p.in_code {
        return None;
    }
    let tp = p.item.as_ref()?.task_pos?;
    let new = if text.as_bytes()[tp + 1] == b' ' { "x" } else { " " };
    Some(Edit {
        range: tp + 1..tp + 2,
        insert: new.to_string(),
        selection: off..off,
    })
}

/// Clipboard-aware paste. URL over a selection becomes a link. A multi-line
/// clipboard pasted inside a list item indents its continuation lines to the
/// item's content column. Everything else returns `None` (default paste).
pub fn smart_paste(text: &str, sel: Range<usize>, clipboard: &str) -> Option<Edit> {
    if let Some(edit) = paste_url_over_selection(text, sel.clone(), clipboard) {
        return Some(edit);
    }
    let clip = clipboard.replace("\r\n", "\n");
    if !clip.contains('\n') {
        return None;
    }
    let (s, e) = norm(text, sel);
    let p = line_prefix(text, s);
    if p.in_code || p.table {
        return None;
    }
    let it = p.item.as_ref()?;
    let cont = format!(
        "{}{}",
        &text[p.line_start..p.quote_end],
        " ".repeat(it.indent + it.marker_end - it.marker_start)
    );
    let mut pasted = String::with_capacity(clip.len() + cont.len() * 4);
    for (i, line) in clip.split('\n').enumerate() {
        if i > 0 {
            pasted.push('\n');
            if !line.trim().is_empty() {
                pasted.push_str(&cont);
            }
        }
        pasted.push_str(line);
    }
    let new = format!("{}{}{}", &text[..s], pasted, &text[e..]);
    let c = s + pasted.len();
    Some(finish(text, new, c..c))
}

// ---------------------------------------------------------------- tables

/// Byte spans (relative to `line`) of the cells of a table row. Cells are the
/// text between unescaped pipes; the empty segments outside a leading or
/// trailing pipe are dropped.
pub(crate) fn cell_spans_rel(line: &str) -> Vec<(usize, usize)> {
    let b = line.as_bytes();
    let mut pipes = Vec::new();
    for i in 0..b.len() {
        if b[i] == b'|' && (i == 0 || b[i - 1] != b'\\') {
            pipes.push(i);
        }
    }
    if pipes.is_empty() {
        return if line.trim().is_empty() {
            Vec::new()
        } else {
            vec![(0, line.len())]
        };
    }
    let mut segs = Vec::with_capacity(pipes.len() + 1);
    let mut start = 0;
    for &p in &pipes {
        segs.push((start, p));
        start = p + 1;
    }
    segs.push((start, b.len()));
    if line[..pipes[0]].trim().is_empty() {
        segs.remove(0);
    }
    if line[pipes[pipes.len() - 1] + 1..].trim().is_empty() {
        segs.pop();
    }
    segs
}

/// Trims a span's whitespace; returns the (start, end) of the trimmed text.
pub(crate) fn trim_span(line: &str, (a, b): (usize, usize)) -> (usize, usize) {
    let s = &line[a..b];
    let lead = s.len() - s.trim_start().len();
    (a + lead, a + lead + s.trim().len())
}

fn cells_of(line: &str) -> Vec<String> {
    cell_spans_rel(line)
        .into_iter()
        .map(|sp| {
            let (a, b) = trim_span(line, sp);
            line[a..b].to_string()
        })
        .collect()
}

/// A GFM table as a model: rendering it re-aligns the columns.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Table {
    header: Vec<String>,
    align: Vec<Align>,
    body: Vec<Vec<String>>,
}

impl Table {
    fn parse(lines: &[&str]) -> Option<Table> {
        if lines.len() < 2 {
            return None;
        }
        let mut align = delim_aligns(lines[1])?;
        let mut header = cells_of(lines[0]);
        let mut body: Vec<Vec<String>> = lines[2..].iter().map(|l| cells_of(l)).collect();
        let n = header
            .len()
            .max(align.len())
            .max(body.iter().map(Vec::len).max().unwrap_or(0));
        if n == 0 {
            return None;
        }
        align.resize(n, Align::Default);
        header.resize(n, String::new());
        for row in body.iter_mut() {
            row.resize(n, String::new());
        }
        Some(Table { header, align, body })
    }
}

fn delim_cell(w: usize, a: Align) -> String {
    match a {
        Align::Default => "-".repeat(w),
        Align::Left => format!(":{}", "-".repeat(w - 1)),
        Align::Right => format!("{}:", "-".repeat(w - 1)),
        Align::Center => format!(":{}:", "-".repeat(w - 2)),
    }
}

fn render_row(cells: &[String], widths: &[usize], align: &[Align], out: &mut String) -> Vec<usize> {
    let mut starts = Vec::with_capacity(cells.len());
    out.push('|');
    for (c, cell) in cells.iter().enumerate() {
        out.push(' ');
        let gap = widths[c] - display_width(cell);
        let (l, r) = match align[c] {
            Align::Right => (gap, 0),
            Align::Center => (gap / 2, gap - gap / 2),
            _ => (0, gap),
        };
        for _ in 0..l {
            out.push(' ');
        }
        starts.push(out.len());
        out.push_str(cell);
        for _ in 0..r {
            out.push(' ');
        }
        out.push_str(" |");
    }
    starts
}

/// Renders the table with aligned columns (min width 3). Returns the text and,
/// per row (0 = header, i+1 = body row i), the byte offset of each cell's text.
fn render(t: &Table) -> (String, Vec<Vec<usize>>) {
    let n = t.align.len();
    let mut widths = vec![3usize; n];
    for row in std::iter::once(&t.header).chain(t.body.iter()) {
        for c in 0..n {
            widths[c] = widths[c].max(display_width(&row[c]));
        }
    }
    let mut out = String::new();
    let mut pos = vec![render_row(&t.header, &widths, &t.align, &mut out)];
    out.push('\n');
    out.push('|');
    for c in 0..n {
        out.push(' ');
        out.push_str(&delim_cell(widths[c], t.align[c]));
        out.push_str(" |");
    }
    for row in &t.body {
        out.push('\n');
        pos.push(render_row(row, &widths, &t.align, &mut out));
    }
    (out, pos)
}

/// Caret/selection context inside a GFM table.
#[derive(Debug, Clone)]
struct TableCtx {
    start: usize,
    end: usize,
    table: Table,
    /// Line index within the table (0 header, 1 delimiter, 2.. body).
    line: usize,
    /// Body row index; `None` for the header.
    row: Option<usize>,
    /// Column index (clamped to the table width).
    col: usize,
    /// Column index in the raw line (may exceed the table width on ragged rows).
    raw_col: usize,
    /// Caret offset within the cell's trimmed text.
    off: usize,
}

fn table_ctx(text: &str, c: usize) -> Option<TableCtx> {
    let (ls, le) = line_bounds(text, c);
    let p = analyze(text, ls, le, fence_before(text, ls));
    if !p.table {
        return None;
    }
    let (start, end) = table_block(text, ls)?;
    let lines: Vec<&str> = text[start..end].split('\n').collect();
    let table = Table::parse(&lines)?;
    let line = text[start..ls].matches('\n').count();
    let line_text = &text[ls..le];
    let spans = cell_spans_rel(line_text);
    if spans.is_empty() {
        return None;
    }
    let rel = c - ls;
    let raw_col = spans
        .iter()
        .position(|&(a, b)| rel >= a && rel <= b)
        .unwrap_or(if rel < spans[0].0 { 0 } else { spans.len() - 1 });
    let (ta, tb) = trim_span(line_text, spans[raw_col]);
    let off = rel.clamp(ta, tb) - ta;
    let row = if line <= 1 { None } else { Some(line - 2) };
    let col = raw_col.min(table.align.len() - 1);
    Some(TableCtx {
        start,
        end,
        table,
        line,
        row,
        col,
        raw_col,
        off,
    })
}

/// Edit replacing the table block with the re-rendered `t`, selecting
/// `off_a..off_b` inside the given cell (offsets clamp to the cell text).
fn rebuild(start: usize, end: usize, t: &Table, target: Option<usize>, col: usize, off_a: usize, off_b: usize) -> Edit {
    let (rendered, pos) = render(t);
    let r = target.map_or(0, |i| i + 1);
    let cell_start = pos[r][col];
    let len = match target {
        None => t.header[col].len(),
        Some(i) => t.body[i][col].len(),
    };
    Edit {
        range: start..end,
        insert: rendered,
        selection: start + cell_start + off_a.min(len)..start + cell_start + off_b.min(len),
    }
}

/// Where Up/Down lands when the visual target `target` is inside a GFM table.
/// The delimiter row is skipped, and the caret goes into a cell's content:
/// the cell of `cursor` (the column is kept), or the cell under `target`
/// when the caret came from outside the table. Never before the first `|` or
/// after the last one. Targets outside tables are returned unchanged.
pub fn table_vertical_target(text: &str, target: usize, cursor: usize, delta: isize) -> usize {
    let target = norm(text, target..target).0;
    let (mut ls, mut le) = line_bounds(text, target);
    let p = analyze(text, ls, le, fence_before(text, ls));
    if !p.table {
        return target;
    }
    let mut rel = target - ls;
    if p.table_delim {
        // The delimiter row is not cell content: step over it.
        let (nls, nle) = if delta > 0 {
            if le >= text.len() {
                return target;
            }
            line_bounds(text, le + 1)
        } else {
            if ls == 0 {
                return target;
            }
            line_bounds(text, ls - 1)
        };
        let np = analyze(text, nls, nle, fence_before(text, nls));
        if !np.table || np.table_delim {
            return target;
        }
        ls = nls;
        le = nle;
        rel = rel.min(le - ls);
    }
    let line = &text[ls..le];
    let spans = cell_spans_rel(line);
    if spans.is_empty() {
        return target;
    }
    let col = match table_ctx(text, cursor) {
        Some(ctx) => ctx.raw_col,
        None => spans
            .iter()
            .position(|&(a, b)| rel >= a && rel <= b)
            .unwrap_or(if rel < spans[0].0 { 0 } else { spans.len() - 1 }),
    };
    let (a, b) = trim_span(line, spans[col.min(spans.len() - 1)]);
    ls + rel.clamp(a, b)
}

/// Moves to the next (or previous with `shift`) cell and selects its contents.
/// Moving past the last cell appends a body row. Navigation does not reflow.
pub fn table_next_cell(text: &str, sel: Range<usize>, shift: bool) -> Option<Edit> {
    let (s, _) = norm(text, sel);
    let ctx = table_ctx(text, s)?;
    let lines: Vec<&str> = text[ctx.start..ctx.end].split('\n').collect();
    let mut starts = Vec::with_capacity(lines.len());
    let mut acc = ctx.start;
    for l in &lines {
        starts.push(acc);
        acc += l.len() + 1;
    }
    let mut order: Vec<(usize, usize)> = Vec::new();
    for (li, l) in lines.iter().enumerate() {
        if li == 1 && is_delim(l) {
            continue;
        }
        for ci in 0..cell_spans_rel(l).len() {
            order.push((li, ci));
        }
    }
    let idx = order.iter().position(|&x| x == (ctx.line, ctx.raw_col))?;
    let target = if shift {
        if idx == 0 {
            return None;
        }
        Some(order[idx - 1])
    } else {
        order.get(idx + 1).copied()
    };
    match target {
        Some((li, ci)) => {
            let spans = cell_spans_rel(lines[li]);
            let (a, b) = trim_span(lines[li], spans[ci]);
            Some(Edit {
                range: s..s,
                insert: String::new(),
                selection: starts[li] + a..starts[li] + b,
            })
        }
        None => {
            let mut t = ctx.table.clone();
            let n = t.align.len();
            t.body.push(vec![String::new(); n]);
            let target = Some(t.body.len() - 1);
            Some(rebuild(ctx.start, ctx.end, &t, target, 0, 0, 0))
        }
    }
}

/// Inserts an empty body row below the caret's row (or as the first body row
/// when the caret is in the header).
pub fn table_add_row(text: &str, sel: Range<usize>) -> Option<Edit> {
    let (s, _) = norm(text, sel);
    let ctx = table_ctx(text, s)?;
    let mut t = ctx.table.clone();
    let at = ctx.row.map_or(0, |i| i + 1);
    let n = t.align.len();
    t.body.insert(at, vec![String::new(); n]);
    Some(rebuild(ctx.start, ctx.end, &t, Some(at), ctx.col, 0, 0))
}

/// Inserts an empty column after the caret's column.
pub fn table_add_col(text: &str, sel: Range<usize>) -> Option<Edit> {
    let (s, _) = norm(text, sel);
    let ctx = table_ctx(text, s)?;
    let mut t = ctx.table.clone();
    let at = ctx.col + 1;
    t.header.insert(at, String::new());
    t.align.insert(at, Align::Default);
    for row in t.body.iter_mut() {
        row.insert(at, String::new());
    }
    Some(rebuild(ctx.start, ctx.end, &t, ctx.row, at, 0, 0))
}

/// Deletes the caret's body row. The header cannot be deleted.
pub fn table_delete_row(text: &str, sel: Range<usize>) -> Option<Edit> {
    let (s, _) = norm(text, sel);
    let ctx = table_ctx(text, s)?;
    let i = ctx.row?;
    let mut t = ctx.table.clone();
    t.body.remove(i);
    let target = if t.body.is_empty() { None } else { Some(i.min(t.body.len() - 1)) };
    Some(rebuild(ctx.start, ctx.end, &t, target, ctx.col, 0, 0))
}

/// Deletes the caret's column (not allowed when it is the only column).
pub fn table_delete_col(text: &str, sel: Range<usize>) -> Option<Edit> {
    let (s, _) = norm(text, sel);
    let ctx = table_ctx(text, s)?;
    let n = ctx.table.align.len();
    if n <= 1 {
        return None;
    }
    let mut t = ctx.table.clone();
    t.header.remove(ctx.col);
    t.align.remove(ctx.col);
    for row in t.body.iter_mut() {
        row.remove(ctx.col);
    }
    let col = ctx.col.min(n - 2);
    Some(rebuild(ctx.start, ctx.end, &t, ctx.row, col, 0, 0))
}

/// Re-aligns the columns of the GFM table occupying `table_range`. The caret
/// goes to the start of the table.
pub fn table_reformat(text: &str, table_range: Range<usize>) -> Option<Edit> {
    let (s, e0) = norm(text, table_range);
    let e = s + text[s..e0].trim_end_matches('\n').len();
    let lines: Vec<&str> = text[s..e].split('\n').filter(|l| !l.trim().is_empty()).collect();
    let t = Table::parse(&lines)?;
    let (rendered, _) = render(&t);
    Some(Edit {
        range: s..e,
        insert: rendered,
        selection: s..s,
    })
}

/// Byte range of the GFM table containing `offset`, for [`table_reformat`].
/// `None` when the line is not part of a table (or is inside a code block).
pub fn table_range(text: &str, offset: usize) -> Option<Range<usize>> {
    let off = norm(text, offset..offset).0;
    let (ls, _) = line_bounds(text, off);
    let p = analyze(text, ls, line_bounds(text, off).1, fence_before(text, ls));
    if !p.table {
        return None;
    }
    table_block(text, ls).map(|(a, b)| a..b)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test input markers: `‸` caret, `«` selection start, `»` selection end.
    fn unmark(input: &str) -> (String, Range<usize>) {
        let mut text = String::new();
        let mut s = None;
        let mut e = None;
        for ch in input.chars() {
            match ch {
                '‸' => {
                    s = Some(text.len());
                    e = s;
                }
                '«' => s = Some(text.len()),
                '»' => e = Some(text.len()),
                c => text.push(c),
            }
        }
        let s = s.unwrap_or(0);
        let e = e.unwrap_or(s);
        (text, s.min(e)..s.max(e))
    }

    fn mark(text: &str, sel: Range<usize>) -> String {
        let mut out = String::new();
        out.push_str(&text[..sel.start]);
        if sel.start == sel.end {
            out.push('‸');
        } else {
            out.push('«');
            out.push_str(&text[sel.start..sel.end]);
            out.push('»');
        }
        out.push_str(&text[sel.end..]);
        out
    }

    /// Runs `f` on the marked input and compares the marked result.
    fn check(input: &str, expect: &str, f: impl Fn(&str, Range<usize>) -> Option<Edit>) {
        let (t, sel) = unmark(input);
        let e = f(&t, sel).unwrap_or_else(|| panic!("expected Some for {input:?}"));
        let out = e.apply(&t);
        assert_eq!(mark(&out, e.selection.clone()), expect, "input {input:?}");
    }

    fn check_none(input: &str, f: impl Fn(&str, Range<usize>) -> Option<Edit>) {
        let (t, sel) = unmark(input);
        assert!(f(&t, sel).is_none(), "expected None for {input:?}");
    }

    fn enter_f(t: &str, s: Range<usize>) -> Option<Edit> {
        enter(t, s)
    }
    fn back_f(t: &str, s: Range<usize>) -> Option<Edit> {
        backspace_at_line_start(t, s)
    }

    // ------------------------------------------------------------ Enter

    #[test]
    fn enter_paragraph_end_opens_new_paragraph() {
        check("hello‸", "hello\n\n‸", enter_f);
    }

    #[test]
    fn enter_paragraph_mid_splits() {
        check("hel‸lo", "hel\n\n‸lo", enter_f);
    }

    #[test]
    fn enter_thai_paragraph() {
        check("สวัสดี‸ครับ", "สวัสดี\n\n‸ครับ", enter_f);
    }

    #[test]
    fn enter_emoji_mid_paragraph() {
        check("a😀‸b", "a😀\n\n‸b", enter_f);
    }

    #[test]
    fn enter_bullet_continues() {
        check("- a‸", "- a\n- ‸", enter_f);
    }

    #[test]
    fn enter_bullet_mid_splits_item() {
        check("- he‸llo", "- he\n- ‸llo", enter_f);
    }

    #[test]
    fn enter_ordered_increments() {
        check("9. a‸", "9. a\n10. ‸", enter_f);
    }

    #[test]
    fn enter_ordered_paren_delim_kept() {
        check("1) a‸", "1) a\n2) ‸", enter_f);
    }

    #[test]
    fn enter_task_continues_unchecked() {
        check("- [x] done‸", "- [x] done\n- [ ] ‸", enter_f);
    }

    #[test]
    fn enter_empty_top_item_removes_marker() {
        check("- ‸", "‸", enter_f);
    }

    #[test]
    fn enter_empty_nested_item_outdents() {
        check("- a\n  - ‸", "- a\n- ‸", enter_f);
    }

    #[test]
    fn enter_empty_nested_ordered_outdents_to_parent() {
        check("1. a\n   1. ‸", "1. a\n1. ‸", enter_f);
    }

    #[test]
    fn enter_empty_task_removes_checkbox_at_top() {
        check("- [ ] ‸", "‸", enter_f);
    }

    #[test]
    fn enter_empty_item_inside_quote_becomes_quote_line() {
        check("> - ‸", "> ‸", enter_f);
    }

    #[test]
    fn enter_in_quote_paragraph_continues_quote() {
        check("> a‸", "> a\n>\n> ‸", enter_f);
    }

    #[test]
    fn enter_empty_quote_line_exits() {
        check("> ‸", "‸", enter_f);
    }

    #[test]
    fn enter_nested_empty_quote_drops_one_level() {
        check("> > ‸", "> ‸", enter_f);
    }

    #[test]
    fn enter_quote_list_continues_inside_quote() {
        check("> - a‸", "> - a\n> - ‸", enter_f);
    }

    #[test]
    fn enter_heading_end_opens_paragraph() {
        check("# Title‸", "# Title\n\n‸", enter_f);
    }

    #[test]
    fn enter_heading_mid_splits() {
        check("# Ti‸tle", "# Ti\n\n‸tle", enter_f);
    }

    #[test]
    fn enter_empty_heading_becomes_paragraph() {
        check("## ‸", "‸", enter_f);
    }

    #[test]
    fn enter_thai_heading_split() {
        check("## สวัส‸ดี", "## สวัส\n\n‸ดี", enter_f);
    }

    #[test]
    fn enter_fenced_code_keeps_indentation() {
        check("```\n    x‸\n```", "```\n    x\n    ‸\n```", enter_f);
    }

    #[test]
    fn enter_fenced_code_mid_line() {
        check("```rust\nlet a‸ = 1;\n```", "```rust\nlet a\n‸ = 1;\n```", enter_f);
    }

    #[test]
    fn enter_on_fence_opener_starts_code_line() {
        check("```‸", "```\n‸\n```", enter_f);
    }

    #[test]
    fn enter_on_closing_fence_follows_paragraph_rules() {
        check("```\ncode\n```‸", "```\ncode\n```\n\n‸", enter_f);
    }

    #[test]
    fn enter_inside_quoted_code_keeps_quote_marker() {
        check("> ```\n> x‸\n> ```", "> ```\n> x\n> ‸\n> ```", enter_f);
    }

    #[test]
    fn enter_table_row_appends_row() {
        check(
            "| a | b |\n|---|---|\n| 1 | 2 |‸",
            "| a   | b   |\n| --- | --- |\n| 1   | 2   |\n|     | ‸    |",
            enter_f,
        );
    }

    #[test]
    fn enter_table_moves_to_same_column_of_next_row() {
        check(
            "| a | b |\n|---|---|\n| 1 | ‸2 |\n| 3 | 4 |",
            "| a | b |\n|---|---|\n| 1 | 2 |\n| 3 | ‸4 |",
            |t, s| enter(t, s),
        );
    }

    #[test]
    fn enter_table_from_header_moves_past_delimiter() {
        check(
            "| a | b |\n|---|---|\n| 1 | 2 |",
            "| a | b |\n|---|---|\n| ‸1 | 2 |",
            enter_f,
        );
    }

    #[test]
    fn enter_replaces_selection_first() {
        check("hel«lo»", "hel\n\n‸", enter_f);
    }

    #[test]
    fn enter_selection_across_list_items() {
        check("- one\n- «two»", "- one\n\n‸", enter_f);
    }

    #[test]
    fn enter_blank_line_opens_new_line() {
        check("a\n‸\nb", "a\n\n‸\nb", enter_f);
    }

    #[test]
    fn enter_indented_continuation_paragraph_keeps_indent() {
        check("- a\n\n  b‸", "- a\n\n  b\n\n  ‸", enter_f);
    }

    #[test]
    fn enter_caret_inside_marker_is_not_applicable() {
        check_none("-‸ a", enter_f);
    }

    // ------------------------------------------------------------ Backspace

    #[test]
    fn backspace_removes_top_item_marker() {
        check("- ‸a", "‸a", back_f);
    }

    #[test]
    fn backspace_outdents_nested_item() {
        check("- a\n  - ‸b", "- a\n- ‸b", back_f);
    }

    #[test]
    fn backspace_drops_one_quote_level() {
        check("> > ‸a", "> ‸a", back_f);
    }

    #[test]
    fn backspace_drops_last_quote_marker() {
        check("> ‸a", "‸a", back_f);
    }

    #[test]
    fn backspace_heading_becomes_paragraph() {
        check("## ‸Title", "‸Title", back_f);
    }

    #[test]
    fn backspace_heading_inside_item_keeps_item() {
        check("- # ‸T", "- ‸T", back_f);
    }

    #[test]
    fn backspace_plain_paragraph_start_is_default() {
        check_none("ab‸c", back_f);
        check_none("‸abc", back_f);
    }

    #[test]
    fn backspace_selection_is_default() {
        check_none("- «a»", back_f);
    }

    #[test]
    fn backspace_thai_item_content_start() {
        check("- ‸สวัสดี", "‸สวัสดี", back_f);
    }

    // ------------------------------------------------------------ Tab

    fn tab_f(shift: bool) -> impl Fn(&str, Range<usize>) -> Option<Edit> {
        move |t, s| tab(t, s, shift)
    }

    #[test]
    fn tab_indents_item_under_previous_sibling() {
        check("- a\n- ‸b", "- a\n  - ‸b", tab_f(false));
    }

    #[test]
    fn tab_ordered_child_uses_parent_marker_width() {
        check("1. a\n2. ‸b", "1. a\n   2. ‸b", tab_f(false));
    }

    #[test]
    fn tab_moves_nested_children_along() {
        check("- a\n- ‸b\n  - c", "- a\n  - ‸b\n    - c", tab_f(false));
    }

    #[test]
    fn tab_first_item_without_sibling_is_identity() {
        check("- ‸a", "- ‸a", tab_f(false));
    }

    #[test]
    fn shift_tab_outdents_nested_item() {
        check("- a\n  - ‸b", "- a\n- ‸b", tab_f(true));
    }

    #[test]
    fn shift_tab_moves_children_too() {
        check("- a\n  - ‸b\n    - c", "- a\n- ‸b\n  - c", tab_f(true));
    }

    #[test]
    fn shift_tab_top_level_is_identity() {
        check("- ‸a", "- ‸a", tab_f(true));
    }

    #[test]
    fn tab_in_code_inserts_two_spaces_at_caret() {
        check("```\nab‸c\n```", "```\nab  ‸c\n```", tab_f(false));
    }

    #[test]
    fn shift_tab_in_code_removes_up_to_two_spaces() {
        check("```\n   ‸x\n```", "```\n ‸x\n```", tab_f(true));
    }

    #[test]
    fn tab_in_table_is_default() {
        check_none("| a | ‸b |\n|---|---|", tab_f(false));
    }

    #[test]
    fn tab_in_paragraph_is_default() {
        check_none("hello‸", tab_f(false));
    }

    #[test]
    fn tab_multi_item_selection_indents_each() {
        check("- a\n- «b\n- c»", "- a\n  - «b\n  - c»", tab_f(false));
    }

    // ------------------------------------------------------------ inline marks

    fn bold(t: &str, s: Range<usize>) -> Option<Edit> {
        toggle_inline(t, s, InlineMark::Bold)
    }
    fn italic(t: &str, s: Range<usize>) -> Option<Edit> {
        toggle_inline(t, s, InlineMark::Italic)
    }
    fn strike(t: &str, s: Range<usize>) -> Option<Edit> {
        toggle_inline(t, s, InlineMark::Strike)
    }
    fn code(t: &str, s: Range<usize>) -> Option<Edit> {
        toggle_inline(t, s, InlineMark::Code)
    }

    #[test]
    fn bold_wraps_selection() {
        check("hi «there»", "hi **«there»**", bold);
    }

    #[test]
    fn bold_unwraps_markers_outside_selection() {
        check("hi **«there»**", "hi «there»", bold);
    }

    #[test]
    fn bold_unwraps_when_selection_includes_markers() {
        check("«**there**»", "«there»", bold);
    }

    #[test]
    fn bold_caret_in_word_wraps_word() {
        check("hel‸lo", "**hel‸lo**", bold);
    }

    #[test]
    fn bold_caret_outside_word_inserts_pair() {
        check("a ‸ b", "a **‸** b", bold);
    }

    #[test]
    fn bold_caret_inside_bold_word_unwraps_it() {
        check("**hel‸lo**", "hel‸lo", bold);
    }

    #[test]
    fn italic_on_bold_adds_odd_layer() {
        check("**«x»**", "***«x»***", italic);
    }

    #[test]
    fn bold_on_italic_word_nests() {
        check("*«x»*", "***«x»***", bold);
    }

    #[test]
    fn italic_off_from_bold_italic() {
        check("***«x»***", "**«x»**", italic);
    }

    #[test]
    fn bold_off_from_bold_italic() {
        check("***«x»***", "*«x»*", bold);
    }

    #[test]
    fn italic_unwraps_selected_markers() {
        check("«*x*»", "«x»", italic);
    }

    #[test]
    fn italic_wraps_plain_text() {
        check("a «b» c", "a *«b»* c", italic);
    }

    #[test]
    fn strike_wraps_and_unwraps() {
        check("«gone»", "~~«gone»~~", strike);
        check("~~«gone»~~", "«gone»", strike);
    }

    #[test]
    fn code_wraps_and_unwraps() {
        check("run «ls»", "run `«ls»`", code);
        check("run `«ls»`", "run «ls»", code);
    }

    #[test]
    fn thai_word_expansion_bold() {
        check("สวัสดี‸ครับ", "**สวัสดี‸ครับ**", bold);
    }

    #[test]
    fn thai_selection_bold_roundtrip() {
        check("«สวัสดี»", "**«สวัสดี»**", bold);
        check("**«สวัสดี»**", "«สวัสดี»", bold);
    }

    #[test]
    fn emoji_selection_italic() {
        check("«😀»", "*«😀»*", italic);
    }

    // ------------------------------------------------------------ links

    #[test]
    fn link_with_url_selects_label() {
        check("see «docs»", "see [«docs»](https://x.y)", |t, s| insert_link(t, s, Some("https://x.y")));
    }

    #[test]
    fn link_without_url_puts_caret_in_parens() {
        check("see «docs»", "see [docs](‸)", |t, s| insert_link(t, s, None));
    }

    #[test]
    fn link_empty_selection_without_url() {
        check("a ‸b", "a [](‸)b", |t, s| insert_link(t, s, None));
    }

    #[test]
    fn link_url_with_space_is_angle_wrapped() {
        check("«x»", "[«x»](<a b>)", |t, s| insert_link(t, s, Some("a b")));
    }

    #[test]
    fn link_label_escapes_brackets() {
        check("«a]b»", "[«a\\]b»](u)", |t, s| insert_link(t, s, Some("u")));
    }

    #[test]
    fn paste_url_over_selection_makes_link() {
        check("«docs»", "[«docs»](https://a.b/c)", |t, s| {
            paste_url_over_selection(t, s, "https://a.b/c")
        });
    }

    #[test]
    fn paste_url_mailto_over_selection() {
        check("«me»", "[«me»](mailto:a@b.c)", |t, s| paste_url_over_selection(t, s, "mailto:a@b.c"));
    }

    #[test]
    fn paste_non_url_over_selection_is_none() {
        check_none("«docs»", |t, s| paste_url_over_selection(t, s, "just words"));
    }

    #[test]
    fn paste_url_with_empty_selection_is_none() {
        check_none("a ‸b", |t, s| paste_url_over_selection(t, s, "https://a.b"));
    }

    #[test]
    fn is_url_rules() {
        assert!(is_url("https://a.b"));
        assert!(is_url("http://x"));
        assert!(is_url("mailto:x@y.z"));
        assert!(!is_url("https://"));
        assert!(!is_url("https://a b"));
        assert!(!is_url("ftp://a"));
    }

    // ------------------------------------------------------------ block commands

    #[test]
    fn heading_applies_to_all_selected_lines() {
        check("«one\ntwo»", "«# one\n# two»", |t, s| set_heading(t, s, Some(1)));
    }

    #[test]
    fn heading_same_level_toggles_off() {
        check("# «one»", "«one»", |t, s| set_heading(t, s, Some(1)));
    }

    #[test]
    fn heading_level_change_replaces_marker() {
        check("## ‸one", "### ‸one", |t, s| set_heading(t, s, Some(3)));
    }

    #[test]
    fn heading_keeps_list_marker() {
        check("- ‸a", "- ## ‸a", |t, s| set_heading(t, s, Some(2)));
    }

    #[test]
    fn heading_none_removes() {
        check("### ‸a", "‸a", |t, s| set_heading(t, s, None));
    }

    #[test]
    fn heading_level_out_of_range_is_none() {
        check_none("a", |t, s| set_heading(t, s, Some(7)));
    }

    #[test]
    fn heading_skips_code_lines() {
        check_none("```\n‸x\n```", |t, s| set_heading(t, s, Some(1)));
    }

    #[test]
    fn list_bullet_on_paragraph_lines() {
        check("«a\n\nb»", "«- a\n\n- b»", |t, s| toggle_list(t, s, ListType::Bullet));
    }

    #[test]
    fn list_ordered_numbers_sequentially() {
        check("«a\nb\nc»", "«1. a\n2. b\n3. c»", |t, s| toggle_list(t, s, ListType::Ordered));
    }

    #[test]
    fn list_toggle_off_removes_markers() {
        check("«- a\n- b»", "«a\nb»", |t, s| toggle_list(t, s, ListType::Bullet));
    }

    #[test]
    fn list_task_marker() {
        check("‸hi", "- [ ] ‸hi", |t, s| toggle_list(t, s, ListType::Task));
    }

    #[test]
    fn list_bullet_converts_to_ordered() {
        check("- ‸a", "1. ‸a", |t, s| toggle_list(t, s, ListType::Ordered));
    }

    #[test]
    fn list_keeps_quote_container() {
        check("> ‸a", "> - ‸a", |t, s| toggle_list(t, s, ListType::Bullet));
    }

    #[test]
    fn list_on_heading_line_replaces_heading() {
        check("## ‸a", "- ‸a", |t, s| toggle_list(t, s, ListType::Bullet));
    }

    #[test]
    fn quote_adds_and_removes() {
        check("«a\nb»", "«> a\n> b»", |t, s| toggle_quote(t, s));
        check("«> a\n> b»", "«a\nb»", |t, s| toggle_quote(t, s));
    }

    #[test]
    fn quote_blank_line_inside_selection_gets_bare_marker() {
        check("«a\n\nb»", "«> a\n>\n> b»", |t, s| toggle_quote(t, s));
    }

    #[test]
    fn quote_single_caret_on_blank_line() {
        check("‸", "> ‸", |t, s| toggle_quote(t, s));
    }

    #[test]
    fn quote_nested_add_on_quote_line() {
        check("> ‸a", "‸a", |t, s| toggle_quote(t, s));
    }

    #[test]
    fn set_paragraph_strips_heading_quote_and_list() {
        check("> - ‸# a", "‸a", |t, s| set_paragraph(t, s));
    }

    #[test]
    fn set_paragraph_on_paragraph_is_identity() {
        check("‸a", "‸a", |t, s| set_paragraph(t, s));
    }

    #[test]
    fn code_block_empty_selection_caret_inside() {
        check("text‸", "text\n\n```rust\n‸\n```\n", |t, s| insert_code_block(t, s, "rust"));
    }

    #[test]
    fn code_block_wraps_selection() {
        check("«x = 1»", "```\n‸x = 1\n```\n", |t, s| insert_code_block(t, s, ""));
    }

    #[test]
    fn code_block_empty_doc() {
        check("‸", "```\n‸\n```\n", |t, s| insert_code_block(t, s, ""));
    }

    #[test]
    fn rule_block_between_paragraphs() {
        check("a‸\nb", "a\n\n---\n‸\nb", |t, s| insert_rule(t, s));
    }

    #[test]
    fn table_insert_three_rows_two_cols() {
        check(
            "‸",
            "| ‸    |     |\n| --- | --- |\n|     |     |\n|     |     |\n",
            |t, s| insert_table(t, s, 3, 2),
        );
    }

    #[test]
    fn table_insert_caret_in_header_cell() {
        let (t, s) = unmark("x‸");
        let e = insert_table(&t, s, 2, 1).unwrap();
        let out = e.apply(&t);
        assert_eq!(&out[e.selection.start..e.selection.start], "");
        assert!(out.starts_with("x\n\n| "), "{out:?}");
        assert_eq!(e.selection.start, out.find("| ").unwrap() + 2);
    }

    #[test]
    fn table_insert_zero_is_none() {
        check_none("a", |t, s| insert_table(t, s, 0, 2));
    }

    #[test]
    fn image_inline_replaces_selection() {
        check("see «x»", "see ![x](a.png)‸", |t, s| insert_image(t, s, "", "a.png"));
    }

    #[test]
    fn image_with_alt_and_spaced_url() {
        check("‸", "![logo](<my pic.png>)‸", |t, s| insert_image(t, s, "logo", "my pic.png"));
    }

    #[test]
    fn toggle_task_flips_checkbox() {
        let (t, s) = unmark("- [ ] ‸buy milk");
        let e = toggle_task(&t, s.start).unwrap();
        assert_eq!(e.apply(&t), "- [x] buy milk");
        assert_eq!(e.selection, s.start..s.start);
        let t2 = e.apply(&t);
        assert_eq!(toggle_task(&t2, 0).unwrap().apply(&t2), "- [ ] buy milk");
    }

    #[test]
    fn toggle_task_on_plain_line_is_none() {
        assert!(toggle_task("- plain", 3).is_none());
        assert!(toggle_task("para", 0).is_none());
    }

    #[test]
    fn toggle_task_inside_code_is_none() {
        assert!(toggle_task("```\n- [ ] x\n```", 8).is_none());
    }

    #[test]
    fn smart_paste_url_over_selection() {
        check("«site»", "[«site»](https://x.io)", |t, s| smart_paste(t, s, "https://x.io"));
    }

    #[test]
    fn smart_paste_multiline_in_item_keeps_indent() {
        check(
            "- a‸",
            "- a1\n  2\n  3‸",
            |t, s| smart_paste(t, s, "1\n2\n3"),
        );
    }

    #[test]
    fn smart_paste_multiline_nested_item_uses_item_column() {
        check("  1. x‸", "  1. x1\n     y‸", |t, s| smart_paste(t, s, "1\ny"));
    }

    #[test]
    fn smart_paste_empty_clipboard_line_gets_no_indent() {
        check("- ‸", "- a\n\n  b‸", |t, s| smart_paste(t, s, "a\n\nb"));
    }

    #[test]
    fn smart_paste_multiline_in_paragraph_is_default() {
        check_none("a‸", |t, s| smart_paste(t, s, "1\n2"));
    }

    #[test]
    fn smart_paste_single_line_is_default() {
        check_none("- a‸", |t, s| smart_paste(t, s, "plain"));
    }

    #[test]
    fn smart_paste_inside_code_is_default() {
        check_none("```\na‸\n```", |t, s| smart_paste(t, s, "1\n2"));
    }

    // ------------------------------------------------------------ tables

    fn sample() -> &'static str {
        "| Name | Qty |\n|:-----|----:|\n| apple | 3 |\n| fig | 12 |"
    }

    #[test]
    fn reformat_aligns_columns_and_keeps_alignments() {
        let t = "| a | bb |\n|:-|-:|\n| long cell | x |";
        let e = table_reformat(t, 0..t.len()).unwrap();
        let out = e.apply(t);
        assert_eq!(
            out,
            "| a         |  bb |\n| :-------- | --: |\n| long cell |   x |"
        );
    }

    #[test]
    fn reformat_cjk_width_counts_double() {
        let t = "| 中 | x |\n|---|---|\n| a | b |";
        let out = table_reformat(t, 0..t.len()).unwrap().apply(t);
        // "中" is 2 columns wide, so the first column is 3 wide (min) and the
        // header cell pads like any other.
        assert_eq!(out, "| 中  | x   |\n| --- | --- |\n| a   | b   |");
    }

    #[test]
    fn reformat_thai_combining_marks_are_zero_width() {
        // "ที่" = ท + ี + ่ : 1 consonant column wide, combining marks are free.
        assert_eq!(display_width("ที่"), 1);
        let t = "| ที่ |\n|-|\n| ab |";
        let out = table_reformat(t, 0..t.len()).unwrap().apply(t);
        assert_eq!(out, "| ที่   |\n| --- |\n| ab  |");
    }

    #[test]
    fn display_width_rules() {
        assert_eq!(display_width("abc"), 3);
        assert_eq!(display_width("中文"), 4);
        assert_eq!(display_width("😀"), 2);
        assert_eq!(display_width("สวัสดี"), 4);
        assert_eq!(display_width("e\u{0301}"), 1);
        assert_eq!(char_width('\u{200B}'), 0);
    }

    #[test]
    fn reformat_pads_ragged_rows() {
        let t = "| a | b |\n|-|-|\n| only |";
        let out = table_reformat(t, 0..t.len()).unwrap().apply(t);
        assert_eq!(out, "| a    | b   |\n| ---- | --- |\n| only |     |");
    }

    #[test]
    fn reformat_rejects_non_table() {
        assert!(table_reformat("a | b", 0..5).is_none());
    }

    #[test]
    fn table_next_cell_selects_next_cell_contents() {
        let t = sample();
        let caret = t.find("apple").unwrap();
        let e = table_next_cell(t, caret..caret, false).unwrap();
        assert_eq!(&e.apply(t)[e.selection.clone()], "3");
    }

    #[test]
    fn table_next_cell_crosses_row_boundary() {
        let t = sample();
        let caret = t.find("3 |").unwrap();
        let e = table_next_cell(t, caret..caret, false).unwrap();
        assert_eq!(&t[e.selection.clone()], "fig");
    }

    #[test]
    fn table_next_cell_shift_goes_back() {
        let t = sample();
        let caret = t.find("12").unwrap();
        let e = table_next_cell(t, caret..caret, true).unwrap();
        assert_eq!(&t[e.selection.clone()], "fig");
    }

    #[test]
    fn table_next_cell_from_first_shift_is_none() {
        let t = sample();
        let caret = t.find("Name").unwrap();
        assert!(table_next_cell(t, caret..caret, true).is_none());
    }

    #[test]
    fn table_next_cell_at_last_cell_appends_row() {
        let t = sample();
        let caret = t.find("12").unwrap();
        let e = table_next_cell(t, caret..caret, false).unwrap();
        let out = e.apply(t);
        assert!(out.ends_with("|       |     |"), "{out}");
        assert_eq!(out.lines().count(), 5);
        assert_eq!(e.selection.start, e.selection.end);
    }

    #[test]
    fn table_add_row_below_caret() {
        let t = sample();
        let caret = t.find("apple").unwrap();
        let out = table_add_row(t, caret..caret).unwrap().apply(t);
        assert_eq!(out.lines().count(), 5);
        assert!(out.lines().nth(3).unwrap().replace('|', "").trim().is_empty());
    }

    #[test]
    fn table_add_row_from_header_inserts_first_body_row() {
        let t = sample();
        let caret = t.find("Name").unwrap();
        let e = table_add_row(t, caret..caret).unwrap();
        let out = e.apply(t);
        assert_eq!(out.lines().nth(2).unwrap().trim_matches(|c| c == '|' || c == ' '), "");
    }

    #[test]
    fn table_add_col_after_caret_column() {
        let t = sample();
        let caret = t.find("Name").unwrap();
        let out = table_add_col(t, caret..caret).unwrap().apply(t);
        let first = out.lines().next().unwrap();
        assert_eq!(first.matches('|').count(), 4, "{out}");
    }

    #[test]
    fn table_delete_row_removes_body_row() {
        let t = sample();
        let caret = t.find("apple").unwrap();
        let out = table_delete_row(t, caret..caret).unwrap().apply(t);
        assert_eq!(out.lines().count(), 3);
        assert!(!out.contains("apple"));
    }

    #[test]
    fn table_delete_row_refuses_header() {
        let t = sample();
        let caret = t.find("Name").unwrap();
        assert!(table_delete_row(t, caret..caret).is_none());
    }

    #[test]
    fn table_delete_col_removes_column() {
        let t = sample();
        let caret = t.find("Qty").unwrap();
        let out = table_delete_col(t, caret..caret).unwrap().apply(t);
        assert!(!out.contains("Qty"));
        assert!(out.contains("Name"));
        assert_eq!(out.lines().next().unwrap().matches('|').count(), 2);
    }

    #[test]
    fn table_delete_only_column_is_none() {
        let t = "| a |\n|-|\n| b |";
        assert!(table_delete_col(t, 2..2).is_none());
    }

    #[test]
    fn table_ops_outside_table_are_none() {
        let t = "plain text\n\n| a |\n|-|";
        assert!(table_next_cell(t, 2..2, false).is_none());
        assert!(table_add_row(t, 2..2).is_none());
        assert!(table_range(t, 0).is_none());
        assert_eq!(table_range(t, t.len() - 1), Some(t.find("| a").unwrap()..t.len()));
    }

    #[test]
    fn table_ctx_on_code_block_is_none() {
        let t = "```\n| a |\n|-|\n```";
        assert!(table_add_row(t, 8..8).is_none());
    }

    // ------------------------------------------------------------ line prefix

    #[test]
    fn line_prefix_reports_nested_structure() {
        let t = "> - [x] task";
        let p = line_prefix(t, 5);
        assert_eq!(p.quote_depth, 1);
        assert_eq!(p.quote_end, 2);
        let it = p.item.unwrap();
        assert_eq!(it.task, Some(true));
        assert_eq!(it.content_start, 8);
        assert!(p.content_empty == false);
    }

    #[test]
    fn line_prefix_heading_and_code() {
        let t = "# head\n```\nx\n```";
        assert_eq!(line_prefix(t, 2).heading, Some(1));
        assert!(line_prefix(t, 10).in_code);
        assert!(line_prefix(t, 8).fence);
        assert!(!line_prefix(t, 15).in_code);
    }

    #[test]
    fn line_prefix_rule_and_table_flags() {
        assert!(line_prefix("***", 0).rule);
        assert!(!line_prefix("- x", 0).rule);
        let t = "| a |\n|---|\n| b |";
        assert!(line_prefix(t, 0).table);
        assert!(line_prefix(t, 6).table_delim);
        assert!(!line_prefix("a | b", 0).table);
    }

    #[test]
    fn line_prefix_ordered_item_fields() {
        let p = line_prefix("  12) x", 0);
        let it = p.item.unwrap();
        assert!(it.ordered);
        assert_eq!(it.number, 12);
        assert_eq!(it.delim, ')');
        assert_eq!(it.indent, 2);
    }

    #[test]
    fn edits_keep_char_boundaries_with_thai_and_emoji() {
        // Every command must produce valid UTF-8 slices.
        let t = "- สวัสดี 😀\n- ครับ";
        for off in 0..=t.len() {
            if !t.is_char_boundary(off) {
                continue;
            }
            let _ = enter(t, off..off);
            let _ = backspace_at_line_start(t, off..off);
            let _ = tab(t, off..off, false);
            let _ = toggle_inline(t, off..off, InlineMark::Bold);
            let _ = set_heading(t, off..off, Some(2));
            let _ = toggle_list(t, off..off, ListType::Ordered);
        }
    }

    #[test]
    fn finish_is_minimal_and_exact() {
        let e = enter("ab‸", 2..2).unwrap();
        let out = e.apply("ab");
        assert_eq!(out, "ab\n\n");
        assert_eq!(e.range, 2..2);
    }
}

/// Regression tests for the verified editor bugs: inline toggles, list/quote/
/// fence Enter and Backspace semantics, link text whitespace, grapheme clusters.
#[cfg(test)]
mod regression {
    use super::*;

    fn unmark(input: &str) -> (String, Range<usize>) {
        let mut text = String::new();
        let mut s = None;
        let mut e = None;
        for ch in input.chars() {
            match ch {
                '‸' => {
                    s = Some(text.len());
                    e = s;
                }
                '«' => s = Some(text.len()),
                '»' => e = Some(text.len()),
                c => text.push(c),
            }
        }
        let s = s.unwrap_or(0);
        let e = e.unwrap_or(s);
        (text, s.min(e)..s.max(e))
    }

    fn mark(text: &str, sel: Range<usize>) -> String {
        let mut out = String::new();
        out.push_str(&text[..sel.start]);
        if sel.start == sel.end {
            out.push('‸');
        } else {
            out.push('«');
            out.push_str(&text[sel.start..sel.end]);
            out.push('»');
        }
        out.push_str(&text[sel.end..]);
        out
    }

    fn check(input: &str, expect: &str, f: impl Fn(&str, Range<usize>) -> Option<Edit>) {
        let (t, sel) = unmark(input);
        let e = f(&t, sel).unwrap_or_else(|| panic!("expected Some for {input:?}"));
        let out = e.apply(&t);
        assert_eq!(mark(&out, e.selection.clone()), expect, "input {input:?}");
    }

    fn enter_f(t: &str, s: Range<usize>) -> Option<Edit> {
        enter(t, s)
    }

    fn back_f(t: &str, s: Range<usize>) -> Option<Edit> {
        backspace_at_line_start(t, s)
    }

    // ---- inline toggles (bug 1: caret-only is never a selection; bug 4: no nesting)

    #[test]
    fn caret_after_word_inserts_pair_and_typing_follows() {
        check("a‸", "a**‸**", |t, s| toggle_inline(t, s, InlineMark::Bold));
        // The typed text lands between the markers, not over the word.
        let (t, s) = unmark("a‸");
        let e = toggle_inline(&t, s, InlineMark::Bold).unwrap();
        let typed = format!("{}b{}", &e.apply(&t)[..e.selection.start], &e.apply(&t)[e.selection.start..]);
        assert_eq!(typed, "a**b**");
    }

    #[test]
    fn caret_at_word_start_or_space_inserts_pair() {
        check("‸abc", "**‸**abc", |t, s| toggle_inline(t, s, InlineMark::Bold));
        check("a ‸b", "a **‸**b", |t, s| toggle_inline(t, s, InlineMark::Bold));
    }

    #[test]
    fn caret_inside_word_keeps_caret_relative() {
        check("hel‸lo", "**hel‸lo**", |t, s| toggle_inline(t, s, InlineMark::Bold));
        check("a‸bc", "*a‸bc*", |t, s| toggle_inline(t, s, InlineMark::Italic));
    }

    #[test]
    fn bold_over_range_with_bold_strips_inner_markers() {
        check("«**hello** world»", "**«hello world»**", |t, s| toggle_inline(t, s, InlineMark::Bold));
    }

    #[test]
    fn bold_toggled_twice_round_trips() {
        check("**«hello world»**", "«hello world»", |t, s| toggle_inline(t, s, InlineMark::Bold));
    }

    #[test]
    fn strike_and_code_over_marked_range_strip_inner() {
        check("«~~a~~ b»", "~~«a b»~~", |t, s| toggle_inline(t, s, InlineMark::Strike));
        check("«`x` y»", "`«x y»`", |t, s| toggle_inline(t, s, InlineMark::Code));
    }

    #[test]
    fn italic_over_italic_range_strips_inner_single_stars() {
        check("«*a* b»", "*«a b»*", |t, s| toggle_inline(t, s, InlineMark::Italic));
    }

    #[test]
    fn lone_marker_in_selection_is_literal_text() {
        check("«a ** b»", "**«a ** b»**", |t, s| toggle_inline(t, s, InlineMark::Bold));
    }

    // ---- list / quote exits (bug 2: a blank line separates the exited block)

    #[test]
    fn exit_bullet_at_end_of_doc_gets_blank_line() {
        check("- a\n- b\n- ‸", "- a\n- b\n\n‸", enter_f);
    }

    #[test]
    fn enter_before_space_keeps_the_word_and_drops_only_the_gap() {
        check("Hello‸ world", "Hello\n\n‸world", enter_f);
    }

    #[test]
    fn exit_ordered_and_task_at_end_of_doc_get_blank_line() {
        check("1. a\n2. ‸", "1. a\n\n‸", enter_f);
        check("- [ ] a\n- [ ] ‸", "- [ ] a\n\n‸", enter_f);
    }

    #[test]
    fn exit_quote_at_end_of_doc_gets_blank_line() {
        check("> a\n> ‸", "> a\n\n‸", enter_f);
    }

    #[test]
    fn exit_item_before_more_text_is_already_separated() {
        check("- a\n- ‸\nafter", "- a\n‸\nafter", enter_f);
    }

    #[test]
    fn exit_first_item_of_doc_needs_no_blank() {
        check("- ‸", "‸", enter_f);
    }

    #[test]
    fn backspace_lifts_item_after_text_with_blank_line() {
        check("- a\n- ‸b", "- a\n\n‸b", back_f);
    }

    #[test]
    fn backspace_drops_last_quote_marker_after_text_with_blank_line() {
        check("> a\n> ‸b", "> a\n\n‸b", back_f);
    }

    #[test]
    fn backspace_lifted_item_inside_quote_keeps_quote_separator() {
        check("> x\n> - ‸b", "> x\n>\n> ‸b", back_f);
    }

    // ---- fenced code (bug 3: Enter after an opening fence creates the block)

    #[test]
    fn enter_after_bare_fence_creates_empty_block() {
        check("```‸", "```\n‸\n```", enter_f);
    }

    #[test]
    fn enter_after_fence_with_language_creates_empty_block() {
        check("```rust‸", "```rust\n‸\n```", enter_f);
    }

    #[test]
    fn enter_after_fence_inside_quote_keeps_marker() {
        check("> ```‸", "> ```\n> ‸\n> ```", enter_f);
    }

    #[test]
    fn enter_after_fence_with_existing_closer_only_adds_a_line() {
        check("```‸\nx\n```", "```\n‸\nx\n```", enter_f);
    }

    // ---- link text (bug 8: whitespace stays outside the link)

    #[test]
    fn link_selection_trailing_newline_stays_outside() {
        check("«word\n»", "[«word»](https://x.y)\n", |t, s| {
            insert_link(t, s, Some("https://x.y"))
        });
    }

    #[test]
    fn link_selection_leading_space_stays_outside() {
        check(" «word»", " [«word»](https://x.y)", |t, s| {
            insert_link(t, s, Some("https://x.y"))
        });
    }

    // ---- grapheme clusters (bug 6)

    #[test]
    fn thai_marks_are_stepped_by_cluster_and_backspace_by_code_point() {
        let t = "กิจ"; // ก + ิ (U+0E34) + จ
        assert_eq!(next_grapheme(t, 0), 6, "ก+ิ is one cluster");
        assert_eq!(next_grapheme(t, 6), 9);
        assert_eq!(prev_grapheme(t, 9), 6);
        assert_eq!(prev_grapheme(t, 6), 0);
        assert_eq!(backspace_start(t, 9), 6, "จ alone");
        assert_eq!(backspace_start(t, 6), 3, "Thai backspace removes only the mark");
    }

    #[test]
    fn zwj_family_and_flag_are_single_clusters() {
        let family = "👨\u{200d}👩\u{200d}👧";
        assert_eq!(next_grapheme(family, 0), family.len());
        assert_eq!(prev_grapheme(family, family.len()), 0);
        assert_eq!(backspace_start(family, family.len()), 0);
        let flag = "🇹🇭";
        assert_eq!(next_grapheme(flag, 0), flag.len());
        assert_eq!(backspace_start(flag, flag.len()), 0);
    }

    #[test]
    fn cluster_floor_snaps_into_a_cluster_start() {
        let t = "a👨\u{200d}👩b";
        let b_start = t.len() - 1;
        assert_eq!(cluster_floor(t, 5), 1);
        assert_eq!(cluster_floor(t, b_start), b_start);
    }

    #[test]
    fn grapheme_steps_never_cross_a_newline() {
        assert_eq!(next_grapheme("a\nb", 1), 2);
        assert_eq!(prev_grapheme("a\nb", 2), 1);
        assert_eq!(next_grapheme("ab", 2), 2);
        assert_eq!(prev_grapheme("ab", 0), 0);
    }

    #[test]
    fn word_motion_lands_on_cluster_boundaries() {
        let text = "ab 👨\u{200d}👩\u{200d}👧 cd กิจ 🇹🇭x";
        for p in 0..=text.len() {
            if !text.is_char_boundary(p) {
                continue;
            }
            for stop in [crate::editor::next_word(text, p), crate::editor::prev_word(text, p)] {
                assert_eq!(cluster_floor(text, stop), stop, "word stop {stop} splits a cluster (from {p})");
            }
        }
    }
}

#[cfg(test)]
mod fixes {
    use super::*;

    fn unmark(input: &str) -> (String, Range<usize>) {
        let mut text = String::new();
        let mut s = None;
        let mut e = None;
        for ch in input.chars() {
            match ch {
                '‸' => {
                    s = Some(text.len());
                    e = s;
                }
                '«' => s = Some(text.len()),
                '»' => e = Some(text.len()),
                c => text.push(c),
            }
        }
        let s = s.unwrap_or(0);
        let e = e.unwrap_or(s);
        (text, s.min(e)..s.max(e))
    }

    fn mark(text: &str, sel: Range<usize>) -> String {
        let mut out = String::new();
        out.push_str(&text[..sel.start]);
        if sel.start == sel.end {
            out.push('‸');
        } else {
            out.push('«');
            out.push_str(&text[sel.start..sel.end]);
            out.push('»');
        }
        out.push_str(&text[sel.end..]);
        out
    }

    fn check(input: &str, expect: &str, f: impl Fn(&str, Range<usize>) -> Option<Edit>) {
        let (t, sel) = unmark(input);
        let e = f(&t, sel).unwrap_or_else(|| panic!("expected Some for {input:?}"));
        let out = e.apply(&t);
        assert_eq!(mark(&out, e.selection.clone()), expect, "input {input:?}");
    }

    fn check_none(input: &str, f: impl Fn(&str, Range<usize>) -> Option<Edit>) {
        let (t, sel) = unmark(input);
        assert!(f(&t, sel).is_none(), "expected None for {input:?}");
    }

    fn enter_f(t: &str, s: Range<usize>) -> Option<Edit> {
        enter(t, s)
    }

    fn back_f(t: &str, s: Range<usize>) -> Option<Edit> {
        backspace_at_line_start(t, s)
    }

    fn bold(t: &str, s: Range<usize>) -> Option<Edit> {
        toggle_inline(t, s, InlineMark::Bold)
    }

    // ---- quote exit: the separator `>` line goes with the exit

    #[test]
    fn enter_twice_in_quote_leaves_one_blank_line() {
        check("> a‸", "> a\n>\n> ‸", enter_f);
        check("> a\n>\n> ‸", "> a\n\n‸", enter_f);
    }

    #[test]
    fn enter_exit_keeps_text_after_the_quote() {
        check("> a\n>\n> ‸\nafter", "> a\n‸\nafter", enter_f);
    }

    // ---- ordered lists renumber their following siblings

    #[test]
    fn enter_mid_ordered_list_renumbers_following_items() {
        check("1. a\n2. b‸\n3. c", "1. a\n2. b\n3. ‸\n4. c", enter_f);
    }

    #[test]
    fn renumber_skips_nested_items() {
        check(
            "1. a\n   - x\n2. b‸\n3. c",
            "1. a\n   - x\n2. b\n3. ‸\n4. c",
            enter_f,
        );
    }

    #[test]
    fn renumber_stops_at_a_paragraph() {
        check("1. a‸\n\ntext\n2. z", "1. a\n2. ‸\n\ntext\n2. z", enter_f);
    }

    #[test]
    fn renumber_keeps_bullets_out_of_the_count() {
        check("1. a‸\n- b\n2. c", "1. a\n2. ‸\n- b\n2. c", enter_f);
    }

    // ---- backspace at the first body line of a fence

    #[test]
    fn backspace_at_code_body_start_does_not_join_the_fence() {
        check("```\n‸ab", "```‸\nab", back_f);
    }

    #[test]
    fn backspace_at_code_body_start_keeps_info_string() {
        check("```js\n‸x", "```js‸\nx", back_f);
    }

    #[test]
    fn backspace_inside_code_body_is_default() {
        check_none("```\nab\n‸cd\n```", back_f);
        check("```\n‸", "```‸\n", back_f);
    }

    // ---- bold toggles off around the caret

    #[test]
    fn bold_caret_after_closing_marker_unwraps() {
        check("abc**X**‸", "abcX‸", bold);
    }

    #[test]
    fn bold_caret_before_opening_marker_unwraps() {
        check("abc‸**X**", "abc‸X", bold);
    }

    #[test]
    fn bold_caret_inside_span_with_spaces_unwraps() {
        check("**a ‸b**", "a ‸b", bold);
        check("**a** c‸ **d**", "**a** c**‸** **d**", bold);
    }

    #[test]
    fn bold_caret_inside_second_span_unwraps_only_that_span() {
        check("**a** **b ‸c**", "**a** b ‸c", bold);
    }

    // ---- link with no selection: the URL is the label

    #[test]
    fn link_without_selection_uses_url_as_label() {
        check("‸", "[«https://a.io»](https://a.io)", |t, s| {
            insert_link(t, s, Some("https://a.io"))
        });
    }

    // ---- forward delete joins paragraphs across a blank line

    #[test]
    fn delete_at_paragraph_end_joins_next_paragraph() {
        check("ab‸\n\ncd", "ab‸cd", |t, s| join_paragraphs_forward(t, s));
    }

    #[test]
    fn delete_join_needs_plain_paragraphs_on_both_sides() {
        check_none("- a‸\n\ncd", |t, s| join_paragraphs_forward(t, s));
        check_none("ab‸\n\n- cd", |t, s| join_paragraphs_forward(t, s));
        check_none("# h‸\n\ncd", |t, s| join_paragraphs_forward(t, s));
        check_none("ab‸\ncd", |t, s| join_paragraphs_forward(t, s));
        check_none("ab‸\n\n", |t, s| join_paragraphs_forward(t, s));
    }

    // ---- trailing blocks

    #[test]
    fn rule_insertion_puts_caret_on_the_line_after_it() {
        check("a‸", "a\n\n---\n‸", |t, s| insert_rule(t, s));
    }

    #[test]
    fn typing_on_a_closing_fence_is_guarded_by_line_end() {
        let t = "```js\nx\n```";
        assert_eq!(block_guard_line_end(t, t.len()), Some(t.len()));
        assert_eq!(block_guard_line_end(t, 0), None, "opening fence is not guarded");
        assert_eq!(block_guard_line_end("a\n---", 4), Some(5));
        assert_eq!(block_guard_line_end("a\nb", 2), None);
    }

    #[test]
    fn trailing_line_added_after_a_closing_fence_or_rule() {
        let t = "```js\nx\n```";
        let e = ensure_trailing_line(t).expect("fence at end");
        assert_eq!(e.apply(t), "```js\nx\n```\n");
        assert_eq!(e.selection, t.len() + 1..t.len() + 1);
        assert_eq!(ensure_trailing_line("a\n---").unwrap().apply("a\n---"), "a\n---\n");
        assert!(ensure_trailing_line("```js\nx\n```\n").is_none());
        assert!(ensure_trailing_line("a\nb").is_none());
        assert!(ensure_trailing_line("```js\nx").is_none(), "open code keeps its text");
    }

    // ---- table navigation

    #[test]
    fn table_vertical_skips_delimiter_and_stays_inside_the_cell() {
        let t = "| a | b |\n| - | - |\n| c | d |\n";
        // Down from the header's first `|` lands in the first body cell.
        assert_eq!(table_vertical_target(t, 10, 0, 1), 22);
        assert_eq!(&t[22..23], "c");
    }

    #[test]
    fn table_vertical_keeps_the_column_going_up() {
        let t = "| a | b |\n| - | - |\n| c | d |";
        let cursor = 26; // the `d`
        let target = table_vertical_target(t, 13, cursor, -1);
        assert_eq!(target, 6, "the `b` in the header");
        assert_eq!(&t[target..target + 1], "b");
    }

    #[test]
    fn table_vertical_never_lands_before_the_first_pipe_or_after_the_last() {
        let t = "| a | b |\n| - | - |\n| c | d |";
        assert_eq!(table_vertical_target(t, 0, 0, 1), 2, "before the first pipe clamps into the cell");
        // Past the end of the `d` cell's content (the ` |` after it is not content).
        assert_eq!(table_vertical_target(t, t.len(), 26, 1), 27);
        assert_eq!(&t[26..27], "d");
    }

    #[test]
    fn table_vertical_outside_table_is_identity() {
        let t = "text\n| a | b |\n| - | - |";
        assert_eq!(table_vertical_target(t, 2, 0, 1), 2);
    }
}
