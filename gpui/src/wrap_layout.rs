//! Soft-wrapped text for GPUI: a paragraph shaped to a width, the geometry an
//! editor needs over it (rows, carets, hit-testing, selection rectangles), and
//! the paint calls. The plain editor uses it; the rich editor is meant to.
//!
//! Two layers, so the geometry is testable without a window:
//!
//! - [`Rows`] and [`GlyphMetrics`] are pure f32 geometry: where each visual row
//!   starts (a byte offset) and where it starts in unwrapped x.
//! - [`WrappedText`] shapes a paragraph with GPUI, implements [`GlyphMetrics`]
//!   over the shaped glyphs, and adds painting and quad helpers.
//!
//! Coordinates are paragraph-local: x = 0 at the left edge of the text, y = 0 at
//! the top of the first row, rows `line_height` apart. A paragraph holds no
//! newlines; splitting lines is the caller's job.
//!
//! A soft-wrap point is one byte index that ends one row and starts the next,
//! so a caret there is ambiguous. Callers pass `at_row_end`: `false` draws the
//! caret at the start of the next row (the default), `true` at the end of the
//! row above. End, and a click past the end of a wrapped row, produce `true`.

#![allow(dead_code)]

use std::borrow::Cow;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    fill, point, px, size, App, Background, Bounds, FontStyle, Hsla, PaintQuad, Pixels, Point,
    SharedString, TextAlign, TextRun, Window, WindowTextSystem, WrappedLine,
};

/// Glyph positions of one paragraph in unwrapped (single-row) coordinates.
pub trait GlyphMetrics {
    /// x of the character boundary at byte `index`; the line width past the end.
    fn x_for_index(&self, index: usize) -> f32;
    /// The character boundary nearest unwrapped `x`, clamped to the text.
    fn index_for_x(&self, x: f32) -> usize;
}

/// The visual rows of a paragraph. Row `i` covers `start(i)..end(i)`, and its
/// glyphs begin at `origin(i)` in unwrapped x.
#[derive(Clone, Debug, PartialEq)]
pub struct Rows {
    len: usize,
    starts: Vec<usize>,
    origins: Vec<f32>,
}

impl Rows {
    /// One row covering all `len` bytes.
    pub fn single(len: usize) -> Self {
        Self {
            len,
            starts: vec![0],
            origins: vec![0.0],
        }
    }

    /// Rows from their start offsets and origins. Input that is not a valid
    /// split falls back to one row: starts must begin at 0, strictly increase,
    /// stay within `len`, and pair one-to-one with the origins.
    pub fn new(len: usize, starts: Vec<usize>, origins: Vec<f32>) -> Self {
        let valid = starts.first() == Some(&0)
            && starts.len() == origins.len()
            && starts.windows(2).all(|pair| pair[0] < pair[1])
            && starts.last().map_or(true, |&last| last <= len);
        if valid {
            Self {
                len,
                starts,
                origins,
            }
        } else {
            Self::single(len)
        }
    }

    /// Length of the paragraph in bytes.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn count(&self) -> usize {
        self.starts.len()
    }

    pub fn start(&self, row: usize) -> usize {
        self.starts[row]
    }

    /// Exclusive end of `row`: the next row's start, or the paragraph end.
    pub fn end(&self, row: usize) -> usize {
        self.starts.get(row + 1).copied().unwrap_or(self.len)
    }

    pub fn range(&self, row: usize) -> Range<usize> {
        self.start(row)..self.end(row)
    }

    /// Where `row` begins in unwrapped x.
    pub fn origin(&self, row: usize) -> f32 {
        self.origins[row]
    }

    /// The row a caret at `index` is drawn on.
    pub fn row_of(&self, index: usize, at_row_end: bool) -> usize {
        let index = index.min(self.len);
        // starts[0] is 0, so at least one start is <= index.
        let row = self.starts.partition_point(|&s| s <= index) - 1;
        if at_row_end && row > 0 && self.starts[row] == index {
            row - 1
        } else {
            row
        }
    }

    /// The row a row-local y (px) falls in, clamped to the paragraph.
    pub fn row_at_y(&self, y: f32, line_height: f32) -> usize {
        let row = (y / line_height).floor();
        if row > 0.0 {
            (row as usize).min(self.count() - 1)
        } else {
            0
        }
    }

    /// The row and row-local x of a caret at `index`.
    pub fn caret<G: GlyphMetrics + ?Sized>(
        &self,
        glyphs: &G,
        index: usize,
        at_row_end: bool,
    ) -> (usize, f32) {
        let index = index.min(self.len);
        let row = self.row_of(index, at_row_end);
        (row, glyphs.x_for_index(index) - self.origins[row])
    }

    /// The caret position nearest row-local `x` on `row`, and whether it sits at
    /// the end of that row. x left of the row gives its start; x past it gives
    /// its end, so a click anywhere past a row lands on that row.
    pub fn index_at<G: GlyphMetrics + ?Sized>(
        &self,
        glyphs: &G,
        row: usize,
        x: f32,
    ) -> (usize, bool) {
        let row = row.min(self.count() - 1);
        let (start, end) = (self.start(row), self.end(row));
        let abs = self.origins[row] + x.max(0.0);
        let index = glyphs.index_for_x(abs).clamp(start, end);
        if row + 1 < self.count() && index >= end {
            (end, true)
        } else {
            (index, false)
        }
    }

    /// The caret one row up (`delta < 0`) or down (`delta > 0`), at row-local
    /// `x`. None past the first or last row of this paragraph.
    pub fn step<G: GlyphMetrics + ?Sized>(
        &self,
        glyphs: &G,
        index: usize,
        at_row_end: bool,
        x: f32,
        delta: isize,
    ) -> Option<(usize, bool)> {
        let row = self.row_of(index, at_row_end) as isize + delta;
        if row < 0 || row as usize >= self.count() {
            return None;
        }
        Some(self.index_at(glyphs, row as usize, x))
    }

    /// One `(row, x0, x1)` per row that `range` touches, with row-local x
    /// extents. A row's extent runs to the next row's origin when the range
    /// covers its trailing break, so a selected space paints its width.
    pub fn segments<G: GlyphMetrics + ?Sized>(
        &self,
        glyphs: &G,
        range: Range<usize>,
    ) -> Vec<(usize, f32, f32)> {
        let range = range.start.min(self.len)..range.end.min(self.len);
        let mut out = Vec::new();
        if range.is_empty() {
            return out;
        }
        for row in self.row_of(range.start, false)..self.count() {
            if self.start(row) >= range.end {
                break;
            }
            let s = range.start.max(self.start(row));
            let e = range.end.min(self.end(row));
            if s < e {
                let origin = self.origins[row];
                out.push((
                    row,
                    glyphs.x_for_index(s) - origin,
                    glyphs.x_for_index(e) - origin,
                ));
            }
        }
        out
    }
}

/// A paragraph shaped to a width: its text, its rows, and the GPUI glyphs.
pub struct WrappedText {
    text: SharedString,
    shaped: Option<WrappedLine>,
    rows: Rows,
    line_height: f32,
    has_backgrounds: bool,
}

impl WrappedText {
    /// Shapes `text` with `runs` and breaks it to `wrap_width`; `None` gives one
    /// row that never wraps. The runs should cover the text in order. Runs
    /// that are short or long are cut or extended to fit, and an empty text
    /// is laid out as one empty row with no glyphs.
    pub fn shape(
        text_system: &WindowTextSystem,
        text: SharedString,
        font_size: Pixels,
        runs: &[TextRun],
        wrap_width: Option<Pixels>,
        line_height: Pixels,
    ) -> Self {
        let len = text.len();
        let has_backgrounds = runs.iter().any(|run| run.background_color.is_some());
        let shaped = if text.is_empty() || runs.is_empty() {
            None
        } else {
            let runs = fit_runs(runs, len);
            text_system
                .shape_text(text.clone(), font_size, &runs, wrap_width, None)
                .ok()
                .and_then(|lines| lines.into_iter().next())
        };
        let rows = match &shaped {
            Some(line) => rows_of(line),
            None => Rows::single(len),
        };
        Self {
            text,
            shaped,
            rows,
            line_height: f32::from(line_height),
            has_backgrounds,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Length of the paragraph in bytes.
    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn rows(&self) -> &Rows {
        &self.rows
    }

    pub fn row_count(&self) -> usize {
        self.rows.count()
    }

    pub fn line_height(&self) -> Pixels {
        px(self.line_height)
    }

    /// Height of the whole paragraph in px.
    pub fn height(&self) -> f32 {
        self.rows.count() as f32 * self.line_height
    }

    /// The row and row-local x of a caret at `index`.
    pub fn caret_row_x(&self, index: usize, at_row_end: bool) -> (usize, f32) {
        self.rows.caret(self, index, at_row_end)
    }

    /// The paragraph-local top-left of the caret at `index`.
    pub fn position_for_index(&self, index: usize, at_row_end: bool) -> Point<Pixels> {
        let (row, x) = self.rows.caret(self, index, at_row_end);
        point(px(x), px(row as f32 * self.line_height))
    }

    /// The caret position nearest a paragraph-local point, and whether it sits
    /// at the end of its row. Points past the text or below the last row clamp.
    pub fn hit_test(&self, position: Point<Pixels>) -> (usize, bool) {
        let row = self
            .rows
            .row_at_y(f32::from(position.y), self.line_height);
        self.rows.index_at(self, row, f32::from(position.x))
    }

    pub fn index_for_position(&self, position: Point<Pixels>) -> usize {
        self.hit_test(position).0
    }

    /// The caret nearest row-local `x` on `row` (see [`Rows::index_at`]).
    pub fn index_at_row(&self, row: usize, x: f32) -> (usize, bool) {
        self.rows.index_at(self, row, x)
    }

    /// The caret one row up or down from `index` at row-local `x`. None when
    /// there is no such row in this paragraph.
    pub fn step(
        &self,
        index: usize,
        at_row_end: bool,
        x: f32,
        delta: isize,
    ) -> Option<(usize, bool)> {
        self.rows.step(self, index, at_row_end, x, delta)
    }

    /// Byte range of the visual row that holds the caret at `index`.
    pub fn row_range(&self, index: usize, at_row_end: bool) -> Range<usize> {
        self.rows.range(self.rows.row_of(index, at_row_end))
    }

    /// Paints the text (and run backgrounds) with its first row at `origin`.
    pub fn paint(&self, window: &mut Window, cx: &mut App, origin: Point<Pixels>) {
        let Some(line) = &self.shaped else {
            return;
        };
        let line_height = self.line_height();
        if self.has_backgrounds {
            let _ = line.paint_background(origin, line_height, TextAlign::Left, None, window, cx);
        }
        let _ = line.paint(origin, line_height, TextAlign::Left, None, window, cx);
    }

    /// A `width`-px caret bar one row tall at `index`, offset by `origin`.
    pub fn caret_quad(
        &self,
        origin: Point<Pixels>,
        index: usize,
        at_row_end: bool,
        color: impl Into<Background>,
        width: Pixels,
    ) -> PaintQuad {
        let at = self.position_for_index(index, at_row_end);
        fill(
            Bounds::new(
                point(origin.x + at.x, origin.y + at.y),
                size(width, self.line_height()),
            ),
            color,
        )
    }

    /// One rectangle per visual row that `range` covers, each at least
    /// `min_width` wide, offset by `origin`.
    pub fn range_quads(
        &self,
        origin: Point<Pixels>,
        range: Range<usize>,
        color: impl Into<Background> + Clone,
        min_width: Pixels,
    ) -> Vec<PaintQuad> {
        let line_height = self.line_height();
        let min = f32::from(min_width);
        self.rows
            .segments(self, range)
            .into_iter()
            .map(|(row, x0, x1)| {
                let width = (x1 - x0).max(min);
                fill(
                    Bounds::new(
                        point(
                            origin.x + px(x0),
                            origin.y + px(row as f32 * self.line_height),
                        ),
                        size(px(width), line_height),
                    ),
                    color.clone(),
                )
            })
            .collect()
    }
}

impl GlyphMetrics for WrappedText {
    fn x_for_index(&self, index: usize) -> f32 {
        self.shaped
            .as_ref()
            .map_or(0.0, |line| f32::from(line.unwrapped_layout.x_for_index(index)))
    }

    fn index_for_x(&self, x: f32) -> usize {
        self.shaped.as_ref().map_or(0, |line| {
            line.unwrapped_layout
                .closest_index_for_x(px(x))
                .min(self.len())
        })
    }
}

/// Rows of a shaped line. Each wrap boundary names the glyph that starts a row.
fn rows_of(line: &WrappedLine) -> Rows {
    let mut starts = vec![0];
    let mut origins = vec![0.0];
    for boundary in line.wrap_boundaries() {
        let glyph = &line.unwrapped_layout.runs[boundary.run_ix].glyphs[boundary.glyph_ix];
        starts.push(glyph.index);
        origins.push(f32::from(glyph.position.x));
    }
    drop_mark_breaks(&line.text, &mut starts, &mut origins);
    Rows::new(line.len(), starts, origins)
}

/// Whether `c` is a combining mark with no advance of its own (Thai vowels and
/// tone marks, and the common combining diacritics).
pub fn is_zero_width_mark(c: char) -> bool {
    matches!(
        c,
        '\u{0300}'..='\u{036F}' | '\u{0E31}' | '\u{0E34}'..='\u{0E3A}' | '\u{0E47}'..='\u{0E4E}'
    )
}

/// Removes row starts that fall on a zero-width mark. gpui breaks before every
/// non-Latin character, so a mark can begin a row; it has no width, so keeping it
/// on the row above costs nothing. `starts` and `origins` stay paired.
pub fn drop_mark_breaks(text: &str, starts: &mut Vec<usize>, origins: &mut Vec<f32>) {
    let mut kept_starts = Vec::with_capacity(starts.len());
    let mut kept_origins = Vec::with_capacity(origins.len());
    for (row, (&start, &origin)) in starts.iter().zip(origins.iter()).enumerate() {
        let starts_on_mark = text
            .get(start..)
            .and_then(|rest| rest.chars().next())
            .is_some_and(is_zero_width_mark);
        if row > 0 && starts_on_mark {
            continue;
        }
        kept_starts.push(start);
        kept_origins.push(origin);
    }
    *starts = kept_starts;
    *origins = kept_origins;
}

/// `runs` cut or extended so they cover exactly `len` bytes.
fn fit_runs(runs: &[TextRun], len: usize) -> Cow<'_, [TextRun]> {
    let total: usize = runs.iter().map(|run| run.len).sum();
    if total == len {
        return Cow::Borrowed(runs);
    }
    let mut fitted: Vec<TextRun> = Vec::with_capacity(runs.len() + 1);
    let mut covered = 0;
    for run in runs {
        if covered >= len {
            break;
        }
        let take = run.len.min(len - covered);
        fitted.push(TextRun {
            len: take,
            ..run.clone()
        });
        covered += take;
    }
    if covered < len {
        if let Some(last) = fitted.last().cloned() {
            fitted.push(TextRun {
                len: len - covered,
                ..last
            });
        }
    }
    Cow::Owned(fitted)
}

/// Hash of everything that shapes a paragraph: text, font size, wrap width and
/// the run styles (family, weight, italic, colours, decorations).
pub fn paragraph_key(
    text: &str,
    runs: &[TextRun],
    font_size: Pixels,
    wrap_width: Option<Pixels>,
) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    f32::from(font_size).to_bits().hash(&mut hasher);
    wrap_width
        .map(|width| f32::from(width).to_bits())
        .hash(&mut hasher);
    for run in runs {
        run.len.hash(&mut hasher);
        (*run.font.family).hash(&mut hasher);
        run.font.weight.0.to_bits().hash(&mut hasher);
        matches!(run.font.style, FontStyle::Italic).hash(&mut hasher);
        hsla_bits(run.color).hash(&mut hasher);
        run.background_color.map(hsla_bits).hash(&mut hasher);
        if let Some(underline) = &run.underline {
            format!("{underline:?}").hash(&mut hasher);
        }
        if let Some(strike) = &run.strikethrough {
            format!("{strike:?}").hash(&mut hasher);
        }
    }
    hasher.finish()
}

fn hsla_bits(color: Hsla) -> [u32; 4] {
    [
        color.h.to_bits(),
        color.s.to_bits(),
        color.l.to_bits(),
        color.a.to_bits(),
    ]
}

/// Shaped paragraphs keyed by [`paragraph_key`]. A paragraph is shaped once and
/// reused until its text, runs, font size or wrap width changes, so a repaint
/// or a keystroke in another line costs a map lookup per visible line.
#[derive(Default)]
pub struct ShapeCache {
    entries: HashMap<u64, Rc<WrappedText>>,
}

/// Entries kept before the cache starts over. A screenful is a few dozen
/// paragraphs, so this bounds memory while scrolling a long document.
const MAX_CACHED: usize = 4096;

impl ShapeCache {
    pub fn get_or_shape(
        &mut self,
        text_system: &WindowTextSystem,
        text: &str,
        font_size: Pixels,
        runs: &[TextRun],
        wrap_width: Option<Pixels>,
        line_height: Pixels,
    ) -> Rc<WrappedText> {
        let key = paragraph_key(text, runs, font_size, wrap_width);
        if let Some(hit) = self.entries.get(&key) {
            if hit.text() == text {
                return hit.clone();
            }
        }
        if self.entries.len() >= MAX_CACHED {
            self.entries.clear();
        }
        let shaped = Rc::new(WrappedText::shape(
            text_system,
            SharedString::from(text.to_string()),
            font_size,
            runs,
            wrap_width,
            line_height,
        ));
        self.entries.insert(key, shaped.clone());
        shaped
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CELL: f32 = 10.0;

    /// Fixed-width metrics: every character is `CELL` px wide.
    struct Mono<'a>(&'a str);

    impl GlyphMetrics for Mono<'_> {
        fn x_for_index(&self, index: usize) -> f32 {
            self.0
                .char_indices()
                .take_while(|&(i, _)| i < index)
                .count() as f32
                * CELL
        }

        fn index_for_x(&self, x: f32) -> usize {
            let count = self.0.chars().count();
            let k = ((x / CELL).round().max(0.0) as usize).min(count);
            self.0.char_indices().nth(k).map_or(self.0.len(), |(i, _)| i)
        }
    }

    /// "aaaa bbbb cccc" wrapped after each space: rows at bytes 0, 5, 10.
    const TEXT: &str = "aaaa bbbb cccc";

    fn three_rows() -> Rows {
        Rows::new(TEXT.len(), vec![0, 5, 10], vec![0.0, 50.0, 100.0])
    }

    #[test]
    fn row_of_splits_soft_wrap_points_by_affinity() {
        let rows = three_rows();
        assert_eq!(rows.row_of(0, false), 0);
        assert_eq!(rows.row_of(4, false), 0);
        assert_eq!(rows.row_of(5, false), 1);
        assert_eq!(rows.row_of(5, true), 0);
        assert_eq!(rows.row_of(10, true), 1);
        assert_eq!(rows.row_of(14, false), 2);
        assert_eq!(rows.row_of(999, false), 2);
    }

    #[test]
    fn caret_positions_are_row_local() {
        let rows = three_rows();
        let text = Mono(TEXT);
        assert_eq!(rows.caret(&text, 5, false), (1, 0.0));
        assert_eq!(rows.caret(&text, 5, true), (0, 50.0));
        assert_eq!(rows.caret(&text, 9, false), (1, 40.0));
        assert_eq!(rows.caret(&text, 14, false), (2, 40.0));
    }

    #[test]
    fn hit_testing_clamps_and_marks_row_ends() {
        let rows = three_rows();
        let text = Mono(TEXT);
        assert_eq!(rows.index_at(&text, 0, 0.0), (0, false));
        assert_eq!(rows.index_at(&text, 0, 24.0), (2, false));
        assert_eq!(rows.index_at(&text, 0, 26.0), (3, false));
        assert_eq!(rows.index_at(&text, 0, 46.0), (5, true));
        assert_eq!(rows.index_at(&text, 0, 1000.0), (5, true));
        assert_eq!(rows.index_at(&text, 0, -20.0), (0, false));
        assert_eq!(rows.index_at(&text, 1, -5.0), (5, false));
        assert_eq!(rows.index_at(&text, 2, 1000.0), (14, false));
    }

    #[test]
    fn vertical_steps_keep_the_row_local_x() {
        let rows = three_rows();
        let text = Mono(TEXT);
        assert_eq!(rows.step(&text, 2, false, 30.0, 1), Some((8, false)));
        assert_eq!(rows.step(&text, 8, false, 30.0, -1), Some((3, false)));
        assert_eq!(rows.step(&text, 2, false, 30.0, -1), None);
        assert_eq!(rows.step(&text, 12, false, 0.0, 1), None);
        assert_eq!(rows.step(&text, 7, false, 0.0, -1), Some((0, false)));
    }

    #[test]
    fn wrapped_stepping_lands_on_row_starts_and_row_ends() {
        // Row starts as the verifier painted them; every char 10 px wide.
        let body = "x".repeat(430);
        let text = Mono(&body);
        let starts = vec![0, 18, 117, 216, 316, 418];
        let origins: Vec<f32> = starts.iter().map(|&s| s as f32 * CELL).collect();
        let rows = Rows::new(430, starts, origins);

        // Column 0: each Down lands exactly on the next row start.
        let mut at = 0;
        let mut landed = Vec::new();
        for _ in 0..5 {
            let (next, at_end) = rows.step(&text, at, false, 0.0, 1).expect("row below");
            assert!(!at_end, "column 0 lands at a row start");
            landed.push(next);
            at = next;
        }
        assert_eq!(landed, vec![18, 117, 216, 316, 418]);

        // A non-zero column keeps its row-local x: 30 px into each row.
        assert_eq!(rows.step(&text, 18, false, 30.0, 1), Some((120, false)));
        assert_eq!(rows.step(&text, 120, false, 30.0, -1), Some((21, false)));

        // A column past the row's width lands on that row's end, at_row_end.
        assert_eq!(rows.step(&text, 18, false, 2000.0, 1), Some((216, true)));

        // End after two Downs (caret at 117, row 2) reaches the end of that row.
        let row = rows.row_of(117, false);
        assert_eq!(row, 2);
        assert_eq!(rows.end(row), 216);
        assert_eq!(rows.caret(&text, 216, true), (2, 990.0));
    }

    #[test]
    fn segments_split_a_range_across_rows() {
        let rows = three_rows();
        let text = Mono(TEXT);
        assert_eq!(rows.segments(&text, 3..7), vec![(0, 30.0, 50.0), (1, 0.0, 20.0)]);
        assert_eq!(
            rows.segments(&text, 0..14),
            vec![(0, 0.0, 50.0), (1, 0.0, 50.0), (2, 0.0, 40.0)]
        );
        assert!(rows.segments(&text, 5..5).is_empty());
    }

    #[test]
    fn multibyte_text_never_splits_a_character() {
        // Four 3-byte Thai letters, wrapped after the second.
        let text = "กขคง";
        let rows = Rows::new(text.len(), vec![0, 6], vec![0.0, 20.0]);
        let glyphs = Mono(text);
        assert_eq!(rows.row_of(6, false), 1);
        assert_eq!(rows.index_at(&glyphs, 1, 0.0), (6, false));
        assert_eq!(rows.index_at(&glyphs, 0, 15.0), (6, true));
        assert_eq!(rows.caret(&glyphs, 4, false), (0, 20.0));
    }

    #[test]
    fn invalid_row_splits_fall_back_to_one_row() {
        assert_eq!(Rows::new(5, vec![0, 3, 2], vec![0.0, 1.0, 2.0]), Rows::single(5));
        assert_eq!(Rows::new(5, vec![1, 3], vec![0.0, 1.0]), Rows::single(5));
        assert_eq!(Rows::new(5, vec![0, 3], vec![0.0]), Rows::single(5));
        assert_eq!(Rows::new(5, vec![0, 9], vec![0.0, 1.0]), Rows::single(5));
    }

    #[test]
    fn empty_paragraph_is_one_row() {
        let rows = Rows::single(0);
        let glyphs = Mono("");
        assert_eq!(rows.count(), 1);
        assert_eq!(rows.row_of(0, true), 0);
        assert_eq!(rows.index_at(&glyphs, 0, 50.0), (0, false));
        assert!(rows.segments(&glyphs, 0..0).is_empty());
    }

    #[test]
    fn y_maps_to_rows_and_clamps() {
        let rows = three_rows();
        assert_eq!(rows.row_at_y(-5.0, 22.0), 0);
        assert_eq!(rows.row_at_y(21.9, 22.0), 0);
        assert_eq!(rows.row_at_y(22.0, 22.0), 1);
        assert_eq!(rows.row_at_y(50.0, 22.0), 2);
        assert_eq!(rows.row_at_y(1000.0, 22.0), 2);
    }

    #[test]
    fn breaks_never_start_a_row_on_a_zero_width_mark() {
        // "กิน": base, vowel mark (zero width), base. A break before the mark is dropped.
        let text = "กิน";
        let mut starts = vec![0, 3, 6];
        let mut origins = vec![0.0, 10.0, 20.0];
        drop_mark_breaks(text, &mut starts, &mut origins);
        assert_eq!(starts, vec![0, 6]);
        assert_eq!(origins, vec![0.0, 20.0]);
        // A break before a base character stays.
        let mut starts = vec![0, 6];
        let mut origins = vec![0.0, 20.0];
        drop_mark_breaks(text, &mut starts, &mut origins);
        assert_eq!(starts, vec![0, 6]);
    }

    #[test]
    fn paragraph_key_follows_text_width_and_colour() {
        let font = gpui::font("Menlo");
        let run = |color: Hsla| TextRun {
            len: 4,
            font: font.clone(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let black = gpui::black();
        let white = gpui::white();
        let size = px(14.0);
        let wrap = Some(px(300.0));
        let base = paragraph_key("abcd", &[run(black)], size, wrap);
        assert_eq!(base, paragraph_key("abcd", &[run(black)], size, wrap));
        assert_ne!(base, paragraph_key("abcd", &[run(black)], size, Some(px(301.0))));
        assert_ne!(base, paragraph_key("abcd", &[run(black)], size, None));
        assert_ne!(base, paragraph_key("abcd", &[run(white)], size, wrap));
        assert_ne!(base, paragraph_key("abce", &[run(black)], size, wrap));
    }
}
