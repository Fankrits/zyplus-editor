//! Block widgets of the rich editor: GFM table grids, images, chart, mermaid
//! and math cards, and the copy button of code fences.
//!
//! A widget covers one or more source lines (`Widget::lines`). While the caret
//! or selection touches those lines the source is drawn raw instead (see
//! `RichEditor::widget_rendered`), so a widget is only ever a view of the text
//! and always editable by moving into it. Everything here is in element-local
//! px, the same space `rich_editor` uses for its rows.
//!
//! Pure parts (column widths, cell mapping, table edits, image detection) are
//! unit-tested at the bottom of this file.

#![allow(dead_code)]

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, fill, img, point, px, quad, size, transparent_black, AnyElement, App, AvailableSpace,
    Bounds, BorderStyle, Font, FontStyle, FontWeight, Hsla, Pixels, Point, Rgba, SharedString,
    StrikethroughStyle, TextRun, UnderlineStyle, Window, WindowTextSystem,
};

use crate::chart;
use crate::rich_editor::{cover, Rect};
use crate::rich_model::{
    self, Block, BlockKind, DisplayLine, InlineKind, InlineStyle, Line, LineKind, Model,
};
use crate::rich_ops::{self, Align, Edit};
use crate::theme::Theme;
use crate::wrap_layout::{ShapeCache, WrappedText};

// ---------------------------------------------------------------- metrics

const CELL_SIZE: f32 = 14.0;
const CELL_LH: f32 = 20.0;
const CELL_PAD_X: f32 = 8.0;
const CELL_PAD_Y: f32 = 5.0;
const MIN_COL: f32 = 56.0;
/// Space right of a table for the row and column controls.
const STRIP_W: f32 = 24.0;
const TABLE_TOP: f32 = 6.0;
/// Space under a table for the column controls and the add-row button.
const CONTROLS_H: f32 = 26.0;
const BUTTON_W: f32 = 18.0;
const WIDGET_BOTTOM: f32 = 4.0;
const IMG_TOP: f32 = 6.0;
const IMG_MAX_H: f32 = 560.0;
const URL_IMG_W: f32 = 640.0;
const URL_IMG_H: f32 = 240.0;
const FALLBACK_W: f32 = 420.0;
const FALLBACK_H: f32 = 34.0;
const CAPTION_SIZE: f32 = 11.0;
const CAPTION_LH: f32 = 16.0;
const CARD_TOP: f32 = 6.0;
const CARD_PAD: f32 = 12.0;
const CARD_LABEL_LH: f32 = 16.0;
const CODE_SIZE: f32 = 12.5;
const CODE_LH: f32 = 18.0;
const FENCE_BUTTON_H: f32 = 17.0;
const WIDGET_GAP: f32 = 8.0;

pub const MERMAID_LABEL: &str = "Mermaid diagram — not rendered in the native app";
pub const MATH_LABEL: &str = "Math — not rendered in the native app";

// ---------------------------------------------------------------- classification

/// The widget kinds. A block of one of these kinds is drawn as a widget while
/// the caret is away from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidgetKind {
    Table,
    Image,
    Chart,
    Mermaid,
    Math,
}

/// Which widget (if any) owns source line `i`, as its line range. `charts`
/// mirrors the chart extension: with it off, a chart fence stays a code block.
pub fn widget_span(
    model: &Model,
    blocks: &[Block],
    block_of: &[usize],
    content: &str,
    charts: bool,
    i: usize,
) -> Option<(Range<usize>, WidgetKind)> {
    let line = model.lines.get(i)?;
    if line.kind == LineKind::Paragraph && image_line(content, line).is_some() {
        return Some((i..i + 1, WidgetKind::Image));
    }
    let block = blocks.get(*block_of.get(i)?)?;
    let kind = match &block.kind {
        BlockKind::Table => WidgetKind::Table,
        BlockKind::CodeBlock { lang } => match lang.to_ascii_lowercase().as_str() {
            "mermaid" => WidgetKind::Mermaid,
            "math" | "latex" | "katex" => WidgetKind::Math,
            "chart" | "chartjs" if charts => WidgetKind::Chart,
            _ => return None,
        },
        _ => return None,
    };
    Some((block.lines.clone(), kind))
}

/// `Some((alt, url))` when the line holds nothing but `![alt](url)`.
pub fn image_line(content: &str, line: &Line) -> Option<(String, String)> {
    if line.inlines.len() != 1 {
        return None;
    }
    let inl = &line.inlines[0];
    let InlineKind::Image { url } = &inl.kind else {
        return None;
    };
    let trimmed_end = line.range.start + content[line.range.clone()].trim_end().len();
    if inl.range.start != line.content_start || inl.range.end != trimmed_end {
        return None;
    }
    if url.is_empty() || inl.close.is_empty() {
        return None;
    }
    let alt = content[inl.open.end..inl.close.start].to_string();
    Some((alt, content[url.clone()].to_string()))
}

/// Where an image comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageSource {
    File(PathBuf),
    Url(String),
}

/// A local path is relative to the markdown file; remote URLs are handed to
/// gpui; data URLs and missing files give `None` (the alt-text box is drawn).
pub fn resolve_image(url: &str, base: Option<&Path>) -> Option<ImageSource> {
    if url.starts_with("http://") || url.starts_with("https://") {
        return Some(ImageSource::Url(url.to_string()));
    }
    if url.is_empty() || url.starts_with("data:") {
        return None;
    }
    let trimmed = url.split(['#', '?']).next().unwrap_or(url);
    let decoded = trimmed.replace("%20", " ");
    let path = Path::new(&decoded);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base?.parent()?.join(path)
    };
    path.exists().then_some(ImageSource::File(path))
}

/// Size of an image drawn in a column of `col_w`: its natural size, shrunk to
/// the column width and to `IMG_MAX_H` tall, keeping the aspect ratio.
pub fn image_size(natural: (f32, f32), col_w: f32) -> (f32, f32) {
    let (nw, nh) = natural;
    if nw <= 0.0 || nh <= 0.0 {
        return (col_w.min(URL_IMG_W), URL_IMG_H);
    }
    let mut w = nw.min(col_w);
    let mut h = w * nh / nw;
    if h > IMG_MAX_H {
        h = IMG_MAX_H;
        w = h * nw / nh;
    }
    (w, h)
}

// ---------------------------------------------------------------- table model

/// A GFM table read from the source: cell text ranges, absolute source offsets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SrcTable {
    pub aligns: Vec<Align>,
    /// `rows[0]` is the header, the rest are body rows. Every row has
    /// `aligns.len()` cells; a cell is the trimmed source range of its text
    /// (empty, at the end of its line, when the row is short).
    pub rows: Vec<Vec<Range<usize>>>,
}

impl SrcTable {
    pub fn cols(&self) -> usize {
        self.aligns.len()
    }
}

/// Reads the lines of one table block (header, delimiter, body rows).
pub fn parse_table(content: &str, lines: &[Line]) -> SrcTable {
    let delim = lines.get(1).map(|l| &content[l.range.clone()]);
    let mut aligns = delim.and_then(rich_ops::delim_aligns).unwrap_or_default();
    let mut raw: Vec<(Vec<Range<usize>>, usize)> = Vec::new();
    for (k, line) in lines.iter().enumerate() {
        if k == 1 {
            continue;
        }
        let text = &content[line.range.clone()];
        let base = line.range.start;
        let cells = rich_ops::cell_spans_rel(text)
            .into_iter()
            .map(|sp| {
                let (a, b) = rich_ops::trim_span(text, sp);
                base + a..base + b
            })
            .collect();
        raw.push((cells, line.range.end));
    }
    let n = aligns
        .len()
        .max(raw.iter().map(|(cells, _)| cells.len()).max().unwrap_or(0))
        .max(1);
    aligns.resize(n, Align::Default);
    let rows = raw
        .into_iter()
        .map(|(mut cells, end)| {
            cells.resize(n, end..end);
            cells
        })
        .collect();
    SrcTable { aligns, rows }
}

/// The (row, column) of the cell whose source range holds `offset`.
pub fn cell_for(t: &SrcTable, offset: usize) -> Option<(usize, usize)> {
    for (r, row) in t.rows.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            if cell.start <= offset && offset <= cell.end {
                return Some((r, c));
            }
        }
    }
    None
}

/// A structural table command, as the widget's controls issue them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableOp {
    /// After the last body row.
    AddRow,
    /// After the last column.
    AddCol,
    /// Body row `r` (`r >= 1`).
    DelRow(usize),
    /// Column `c`.
    DelCol(usize),
}

/// The `rich_ops` edit for a table command. The caret is put in the cell the
/// command works on, because `rich_ops` finds the table from the caret.
pub fn table_op_edit(content: &str, t: &SrcTable, op: TableOp) -> Option<Edit> {
    let last_row = t.rows.len().checked_sub(1)?;
    let (r, c) = match op {
        TableOp::AddRow => (last_row, 0),
        TableOp::AddCol => (0, t.cols().checked_sub(1)?),
        TableOp::DelRow(r) if r >= 1 && r <= last_row => (r, 0),
        TableOp::DelRow(_) => return None,
        TableOp::DelCol(c) if c < t.cols() => (0, c),
        TableOp::DelCol(_) => return None,
    };
    let at = t.rows.get(r)?.get(c)?.start;
    let sel = at..at;
    match op {
        TableOp::AddRow => rich_ops::table_add_row(content, sel),
        TableOp::AddCol => rich_ops::table_add_col(content, sel),
        TableOp::DelRow(_) => rich_ops::table_delete_row(content, sel),
        TableOp::DelCol(_) => rich_ops::table_delete_col(content, sel),
    }
}

// ---------------------------------------------------------------- pure layout

/// Column widths for `natural` content widths. Columns are at least `min`.
/// Narrower content is stretched to fill `avail`; wider content shrinks
/// proportionally to its room above `min`. When even the minimum does not fit,
/// the columns stay at `min` and the table overflows.
pub fn column_widths(natural: &[f32], min: f32, avail: f32) -> Vec<f32> {
    let mut w: Vec<f32> = natural.iter().map(|&n| n.max(min)).collect();
    if w.is_empty() {
        return w;
    }
    let sum: f32 = w.iter().sum();
    if sum > avail {
        let excess = sum - avail;
        let room: f32 = w.iter().map(|x| x - min).sum();
        if room > 0.0 {
            let k = (excess / room).min(1.0);
            for x in w.iter_mut() {
                *x -= (*x - min) * k;
            }
        }
    } else if sum > 0.0 && sum < avail {
        let k = avail / sum;
        for x in w.iter_mut() {
            *x *= k;
        }
    }
    w
}

/// Index of the cell that holds `v` between `edges` (ascending, `n + 1` edges
/// for `n` cells). Points outside the edges give `None`.
pub fn cell_index(edges: &[f32], v: f32) -> Option<usize> {
    if edges.len() < 2 || v < edges[0] || v >= edges[edges.len() - 1] {
        return None;
    }
    Some(edges.partition_point(|&e| e <= v) - 1)
}

/// Where a widget sits when the pointer is over it: the table row and column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hover {
    /// First source line of the widget.
    pub line: usize,
    pub row: Option<usize>,
    pub col: Option<usize>,
}

// ---------------------------------------------------------------- widgets

/// A piece of text drawn at an element-local origin.
pub struct Label {
    pub para: Rc<WrappedText>,
    pub x: f32,
    pub y: f32,
}

pub struct TableCell {
    /// Trimmed source range; the display text is the source text of this range.
    pub src: Range<usize>,
    pub disp: DisplayLine,
    pub para: Rc<WrappedText>,
    /// Element-local origin of the text.
    pub x: f32,
    pub y: f32,
}

pub struct TableBody {
    pub frame: Rect,
    /// Element-local x of each column edge (`cols + 1` values).
    pub col_edges: Vec<f32>,
    /// Element-local y of each row edge (`rows + 1` values).
    pub row_edges: Vec<f32>,
    pub cells: Vec<Vec<TableCell>>,
    pub hover_row: Option<usize>,
}

pub enum Body {
    Table(TableBody),
    /// `source` is `None` for the alt-text box.
    Image {
        rect: Rect,
        source: Option<ImageSource>,
        labels: Vec<Label>,
        alt: String,
    },
    Card {
        rect: Rect,
        labels: Vec<Label>,
    },
    Chart {
        rect: Rect,
        src: String,
    },
}

/// A small control drawn on a widget or a code fence.
pub struct Button {
    pub rect: Rect,
    pub label: Rc<WrappedText>,
    /// Element-local origin of the label.
    pub at: (f32, f32),
    pub action: Action,
    /// A Hugeicons id drawn at `icon_rect` (element-local) next to or instead of the label.
    pub icon: Option<&'static str>,
    pub icon_rect: Rect,
    /// The label is not drawn (the icon says it all).
    pub icon_only: bool,
}

/// What a control does. Line numbers are the first source line of the widget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Copies the code of the fence that starts on this line.
    CopyCode(usize),
    Table(usize, TableOp),
}

pub struct Widget {
    pub kind: WidgetKind,
    pub lines: Range<usize>,
    /// The whole widget, controls included.
    pub rect: Rect,
    /// Caret offset when the widget is entered without a cell.
    pub enter: usize,
    pub body: Body,
    pub buttons: Vec<Button>,
}

impl Widget {
    /// The caret offset for a click at element-local `(x, y)`: the cell's
    /// source position for a table, otherwise the widget's entry offset.
    pub fn caret_at(&self, x: f32, y: f32) -> usize {
        if let Body::Table(tb) = &self.body {
            if let (Some(r), Some(c)) = (cell_index(&tb.row_edges, y), cell_index(&tb.col_edges, x)) {
                let cell = &tb.cells[r][c];
                let (idx, _) = cell
                    .para
                    .hit_test(point(px(x - cell.x), px(y - cell.y)));
                return cell.src.start + rich_model::display_to_source(&cell.disp, idx);
            }
        }
        self.enter
    }

    /// The (row, column) under a point, for hover. Rows are found by height
    /// alone so the row control in the right strip works; columns need `x`
    /// inside the table.
    pub fn cell_hover(&self, x: f32, y: f32) -> (Option<usize>, Option<usize>) {
        match &self.body {
            Body::Table(tb) => (cell_index(&tb.row_edges, y), cell_index(&tb.col_edges, x)),
            _ => (None, None),
        }
    }
}

// ---------------------------------------------------------------- state

/// Per-editor state the widgets read: the file (for relative images), the
/// chart extension switch, the hovered control target, the last copy.
#[derive(Default)]
pub struct BlockState {
    pub file_path: Option<PathBuf>,
    pub charts: bool,
    /// Natural size of each image file, by path. `None` when unreadable.
    pub dims: HashMap<String, Option<(f32, f32)>>,
    pub hover: Option<Hover>,
    /// The fence copied last and when; shows "Copied" for about a second.
    pub copied: Option<(usize, std::time::Instant)>,
}

// ---------------------------------------------------------------- building

/// Everything a widget needs to shape and measure itself.
pub struct Ctx<'a> {
    pub content: &'a str,
    pub lines: &'a [Line],
    pub blocks: &'a [Block],
    pub block_of: &'a [usize],
    pub ts: &'a WindowTextSystem,
    pub shapes: &'a mut ShapeCache,
    pub body: Font,
    pub mono: Font,
    pub theme: &'a Theme,
    pub col_left: f32,
    pub col_w: f32,
    /// The markdown file, for relative image paths.
    pub base: Option<&'a Path>,
    pub dims: &'a mut HashMap<String, Option<(f32, f32)>>,
    pub hover: Option<Hover>,
    /// First line of the code fence whose copy button shows "Copied".
    pub copied: Option<usize>,
}

fn text_width(p: &WrappedText) -> f32 {
    f32::from(p.position_for_index(p.len(), false).x)
}

fn tint(c: Rgba, alpha: f32) -> Hsla {
    let mut h: Hsla = c.into();
    h.a = alpha;
    h
}

/// Display of one table cell (or any inline text): its markers are concealed
/// the same way a paragraph's are. The text is `cell` on its own.
fn cell_display(cell: &str) -> DisplayLine {
    let model = rich_model::parse(cell);
    let inlines = model
        .lines
        .first()
        .map(|l| l.inlines.clone())
        .unwrap_or_default();
    let line = Line {
        range: 0..cell.len(),
        content_start: 0,
        marker: 0..0,
        kind: LineKind::Paragraph,
        inlines,
    };
    rich_model::display_line(cell, &line, None)
}

/// Text runs for a display line: inline styles over one base font and colour.
fn runs_for(disp: &DisplayLine, base: &Font, mono: &Font, colour: Rgba, bold: bool, t: &Theme) -> Vec<TextRun> {
    cover(disp.text.len(), &disp.styles)
        .into_iter()
        .map(|(len, st)| {
            let st: InlineStyle = st.unwrap_or_default();
            let mut font = if st.code { mono.clone() } else { base.clone() };
            if st.bold || bold {
                font.weight = FontWeight::BOLD;
            }
            if st.italic {
                font.style = FontStyle::Italic;
            }
            let c: Rgba = if st.marker {
                t.muted
            } else if st.code {
                t.code
            } else if st.link {
                t.accent
            } else {
                colour
            };
            TextRun {
                len,
                font,
                color: c.into(),
                background_color: st.code.then(|| t.hover.into()),
                underline: st.link.then(|| UnderlineStyle {
                    thickness: px(1.0),
                    color: Some(t.accent.into()),
                    wavy: false,
                }),
                strikethrough: st.strike.then(|| StrikethroughStyle {
                    thickness: px(1.0),
                    color: Some(t.muted.into()),
                }),
            }
        })
        .collect()
}

fn shape_plain(
    ctx: &mut Ctx,
    text: &str,
    font: Font,
    size_px: f32,
    lh: f32,
    colour: Rgba,
    wrap: Option<f32>,
) -> Rc<WrappedText> {
    let runs = if text.is_empty() {
        Vec::new()
    } else {
        vec![TextRun {
            len: text.len(),
            font,
            color: colour.into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        }]
    };
    ctx.shapes.get_or_shape(ctx.ts, text, px(size_px), &runs, wrap.map(|w| px(w)), px(lh))
}

fn cell_para(ctx: &mut Ctx, disp: &DisplayLine, header: bool, wrap: Option<f32>) -> Rc<WrappedText> {
    let runs = runs_for(disp, &ctx.body, &ctx.mono, ctx.theme.fg, header, ctx.theme);
    ctx.shapes.get_or_shape(
        ctx.ts,
        &disp.text,
        px(CELL_SIZE),
        &runs,
        wrap.map(|w| px(w)),
        px(CELL_LH),
    )
}

/// Builds the widget for `kind` over `lines`, placed at element-local `top`.
pub fn build(ctx: &mut Ctx, kind: WidgetKind, lines: Range<usize>, top: f32) -> Widget {
    let hover = ctx.hover.filter(|h| h.line == lines.start);
    match kind {
        WidgetKind::Table => build_table(ctx, lines, top, hover),
        WidgetKind::Image => build_image(ctx, lines, top),
        WidgetKind::Chart => build_chart(ctx, lines, top),
        WidgetKind::Mermaid => build_card(ctx, lines, top, WidgetKind::Mermaid),
        WidgetKind::Math => build_card(ctx, lines, top, WidgetKind::Math),
    }
}

fn button(
    ctx: &mut Ctx,
    label: &str,
    rect: Rect,
    action: Action,
    size_px: f32,
) -> Button {
    let para = shape_plain(ctx, label, ctx.body.clone(), size_px, rect.h, ctx.theme.fg, None);
    let tw = text_width(&para);
    // Table controls are glyphs, like the web editor's.
    let icon = match action {
        Action::Table(_, TableOp::AddCol) => Some("column-insert"),
        Action::Table(_, TableOp::AddRow) => Some("row-insert"),
        Action::Table(_, TableOp::DelRow(_)) => Some("row-delete"),
        Action::Table(_, TableOp::DelCol(_)) => Some("column-delete"),
        _ => None,
    };
    let g = (rect.w.min(rect.h) - 6.0).max(10.0);
    Button {
        rect,
        label: para,
        at: (rect.x + (rect.w - tw) / 2.0, rect.y + (rect.h - size_px * 1.2) / 2.0),
        action,
        icon,
        icon_rect: Rect { x: rect.x + (rect.w - g) / 2.0, y: rect.y + (rect.h - g) / 2.0, w: g, h: g },
        icon_only: icon.is_some(),
    }
}

fn build_table(ctx: &mut Ctx, lines: Range<usize>, top: f32, hover: Option<Hover>) -> Widget {
    let src = parse_table(ctx.content, &ctx.lines[lines.clone()]);
    let n = src.cols();
    let x0 = ctx.col_left;
    let table_w = (ctx.col_w - STRIP_W).max(MIN_COL);
    let first_line = lines.start;

    // Measure every cell unwrapped to get the natural column widths.
    let mut disps: Vec<Vec<DisplayLine>> = Vec::with_capacity(src.rows.len());
    let mut natural = vec![MIN_COL; n];
    for (r, row) in src.rows.iter().enumerate() {
        let mut drow = Vec::with_capacity(n);
        for c in 0..n {
            let text = &ctx.content[row[c].clone()];
            let disp = cell_display(text);
            let para = cell_para(ctx, &disp, r == 0, None);
            natural[c] = natural[c].max(text_width(&para) + 2.0 * CELL_PAD_X);
            drow.push(disp);
        }
        disps.push(drow);
    }
    let widths = column_widths(&natural, MIN_COL, table_w);
    let mut col_edges = Vec::with_capacity(n + 1);
    let mut x = x0;
    col_edges.push(x);
    for w in &widths {
        x += w;
        col_edges.push(x);
    }

    // Wrap each cell to its column and size each row to its tallest cell.
    let mut paras: Vec<Vec<Rc<WrappedText>>> = Vec::with_capacity(src.rows.len());
    let mut row_edges = Vec::with_capacity(src.rows.len() + 1);
    let mut y = top + TABLE_TOP;
    row_edges.push(y);
    for (r, drow) in disps.iter().enumerate() {
        let mut prow = Vec::with_capacity(n);
        let mut tallest = CELL_LH;
        for c in 0..n {
            let wrap = (widths[c] - 2.0 * CELL_PAD_X).max(8.0);
            let para = cell_para(ctx, &drow[c], r == 0, Some(wrap));
            tallest = tallest.max(para.height());
            prow.push(para);
        }
        y += tallest + 2.0 * CELL_PAD_Y;
        row_edges.push(y);
        paras.push(prow);
    }
    let table_h = y - (top + TABLE_TOP);

    let mut cells: Vec<Vec<TableCell>> = Vec::with_capacity(src.rows.len());
    for (r, prow) in paras.into_iter().enumerate() {
        let mut crow = Vec::with_capacity(n);
        for (c, para) in prow.into_iter().enumerate() {
            let inner = widths[c] - 2.0 * CELL_PAD_X;
            let tw = if para.row_count() == 1 { text_width(&para) } else { 0.0 };
            let off = match src.aligns[c] {
                Align::Right => (inner - tw).max(0.0),
                Align::Center => ((inner - tw) / 2.0).max(0.0),
                _ => 0.0,
            };
            crow.push(TableCell {
                src: src.rows[r][c].clone(),
                disp: disps[r][c].clone(),
                para,
                x: col_edges[c] + CELL_PAD_X + off,
                y: row_edges[r] + CELL_PAD_Y,
            });
        }
        cells.push(crow);
    }

    let height = TABLE_TOP + table_h + CONTROLS_H + WIDGET_BOTTOM;
    let rect = Rect { x: x0, y: top, w: ctx.col_w, h: height };
    let frame = Rect { x: x0, y: top + TABLE_TOP, w: table_w, h: table_h };
    let hover_row = hover.and_then(|h| h.row);
    let hovered = hover.is_some();

    // Controls: a column at the right for rows and the header, a strip under
    // the table for columns and the add-row button.
    let mut buttons = Vec::new();
    let strip_x = x0 + table_w + (STRIP_W - BUTTON_W) / 2.0;
    let bottom_y = *row_edges.last().unwrap_or(&y) + 4.0;
    if hovered {
        let add_col = Action::Table(first_line, TableOp::AddCol);
        buttons.push(button(
            ctx,
            "+",
            Rect { x: strip_x, y: row_edges[0] + 3.0, w: BUTTON_W, h: BUTTON_W },
            add_col,
            13.0,
        ));
        let add_row = Action::Table(first_line, TableOp::AddRow);
        buttons.push(button(
            ctx,
            "+",
            Rect { x: strip_x, y: bottom_y, w: BUTTON_W, h: BUTTON_W },
            add_row,
            13.0,
        ));
    }
    if let Some(r) = hover.and_then(|h| h.row) {
        if r >= 1 && r < src.rows.len() {
            let cy = (row_edges[r] + row_edges[r + 1]) / 2.0 - BUTTON_W / 2.0;
            buttons.push(button(
                ctx,
                "×",
                Rect { x: strip_x, y: cy, w: BUTTON_W, h: BUTTON_W },
                Action::Table(first_line, TableOp::DelRow(r)),
                13.0,
            ));
        }
    }
    if let Some(c) = hover.and_then(|h| h.col) {
        if n > 1 && c < n {
            let cx = (col_edges[c] + col_edges[c + 1]) / 2.0 - BUTTON_W / 2.0;
            buttons.push(button(
                ctx,
                "×",
                Rect { x: cx, y: bottom_y, w: BUTTON_W, h: BUTTON_W },
                Action::Table(first_line, TableOp::DelCol(c)),
                13.0,
            ));
        }
    }

    let enter = src.rows.first().and_then(|r| r.first()).map(|r| r.start).unwrap_or(ctx.lines[first_line].range.start);
    Widget {
        kind: WidgetKind::Table,
        lines,
        rect,
        enter,
        body: Body::Table(TableBody {
            frame,
            col_edges,
            row_edges,
            cells,
            hover_row,
        }),
        buttons,
    }
}

fn build_image(ctx: &mut Ctx, lines: Range<usize>, top: f32) -> Widget {
    let line = ctx.lines[lines.start].clone();
    let (alt, url) = image_line(ctx.content, &line).unwrap_or_default();
    let x0 = ctx.col_left;
    let source = resolve_image(&url, ctx.base);
    let natural = match &source {
        Some(ImageSource::File(p)) => {
            let key = p.display().to_string();
            *ctx.dims.entry(key).or_insert_with(|| {
                image::image_dimensions(p)
                    .ok()
                    .map(|(w, h)| (w as f32, h as f32))
            })
        }
        _ => None,
    };
    let mut labels = Vec::new();
    let (rect, source) = match (source, natural) {
        (Some(ImageSource::File(p)), Some(n)) => {
            let (w, h) = image_size(n, ctx.col_w);
            (Rect { x: x0, y: top + IMG_TOP, w, h }, Some(ImageSource::File(p)))
        }
        (Some(ImageSource::Url(u)), _) => (
            Rect {
                x: x0,
                y: top + IMG_TOP,
                w: ctx.col_w.min(URL_IMG_W),
                h: URL_IMG_H,
            },
            Some(ImageSource::Url(u)),
        ),
        _ => {
            let shown = if alt.is_empty() { url.clone() } else { alt.clone() };
            let rect = Rect {
                x: x0,
                y: top + IMG_TOP,
                w: ctx.col_w.min(FALLBACK_W),
                h: FALLBACK_H,
            };
            let text = format!("[image] {shown}");
            let para = shape_plain(ctx, &text, ctx.body.clone(), 12.0, 16.0, ctx.theme.muted, None);
            labels.push(Label {
                para,
                x: rect.x + 10.0,
                y: rect.y + (FALLBACK_H - 16.0) / 2.0,
            });
            (rect, None)
        }
    };
    let mut bottom = rect.y + rect.h;
    if source.is_some() && !alt.is_empty() {
        let para = shape_plain(ctx, &alt, ctx.body.clone(), CAPTION_SIZE, CAPTION_LH, ctx.theme.muted, Some(ctx.col_w));
        labels.push(Label { para, x: x0, y: bottom + 4.0 });
        bottom += 4.0 + CAPTION_LH;
    }
    let height = bottom - top + WIDGET_GAP;
    Widget {
        kind: WidgetKind::Image,
        lines: lines.clone(),
        rect: Rect { x: x0, y: top, w: ctx.col_w, h: height },
        enter: ctx.lines[lines.start].range.start,
        body: Body::Image { rect, source, labels, alt: alt.clone() },
        buttons: Vec::new(),
    }
}

/// The source text of a fence's body (its code lines, joined).
pub fn fence_text(content: &str, lines: &[Line], range: Range<usize>) -> String {
    lines[range]
        .iter()
        .filter(|l| matches!(l.kind, LineKind::CodeLine | LineKind::IndentedCode))
        .map(|l| &content[l.range.clone()])
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_card(ctx: &mut Ctx, lines: Range<usize>, top: f32, kind: WidgetKind) -> Widget {
    let label = if kind == WidgetKind::Mermaid { MERMAID_LABEL } else { MATH_LABEL };
    let text = fence_text(ctx.content, ctx.lines, lines.clone());
    let shown = if text.is_empty() { " ".to_string() } else { text };
    let x0 = ctx.col_left;
    let inner = ctx.col_w - 2.0 * CARD_PAD;
    let label_para = shape_plain(ctx, label, ctx.body.clone(), CAPTION_SIZE, CARD_LABEL_LH, ctx.theme.muted, Some(inner));
    let code = shape_plain(ctx, &shown, ctx.mono.clone(), CODE_SIZE, CODE_LH, ctx.theme.code, Some(inner));
    let rect_y = top + CARD_TOP;
    let code_y = rect_y + 10.0 + CARD_LABEL_LH + 6.0;
    let rect_h = code_y + code.height() + 12.0 - rect_y;
    let rect = Rect { x: x0, y: rect_y, w: ctx.col_w, h: rect_h };
    Widget {
        kind,
        lines: lines.clone(),
        rect: Rect { x: x0, y: top, w: ctx.col_w, h: CARD_TOP + rect_h + WIDGET_GAP },
        enter: ctx.lines[lines.start].range.start,
        body: Body::Card {
            rect,
            labels: vec![
                Label { para: label_para, x: x0 + CARD_PAD, y: rect_y + 10.0 },
                Label { para: code, x: x0 + CARD_PAD, y: code_y },
            ],
        },
        buttons: Vec::new(),
    }
}

fn build_chart(ctx: &mut Ctx, lines: Range<usize>, top: f32) -> Widget {
    let src = fence_text(ctx.content, ctx.lines, lines.clone());
    let rect = Rect {
        x: ctx.col_left,
        y: top,
        w: ctx.col_w,
        h: chart::CHART_HEIGHT + 16.0,
    };
    Widget {
        kind: WidgetKind::Chart,
        lines: lines.clone(),
        rect: Rect { x: rect.x, y: top, w: rect.w, h: rect.h + WIDGET_GAP },
        enter: ctx.lines[lines.start].range.start,
        body: Body::Chart { rect, src },
        buttons: Vec::new(),
    }
}

/// The copy button at the top right of a code fence that starts on `line`.
/// `top` is the element-local top of that fence's first row.
pub fn copy_button(ctx: &mut Ctx, line: usize, top: f32) -> Button {
    let copied = ctx.copied == Some(line);
    let label = if copied { "Copied" } else { "Copy" };
    let para = shape_plain(ctx, label, ctx.body.clone(), CAPTION_SIZE, CAPTION_LH, ctx.theme.muted, None);
    const GLYPH: f32 = 12.0;
    let w = text_width(&para) + 16.0 + GLYPH + 4.0;
    let right = ctx.col_left + ctx.col_w - 8.0;
    let rect = Rect { x: right - w, y: top + 3.0, w, h: FENCE_BUTTON_H };
    Button {
        rect,
        label: para,
        at: (rect.x + 8.0 + GLYPH + 4.0, rect.y + (FENCE_BUTTON_H - CAPTION_LH) / 2.0),
        action: Action::CopyCode(line),
        icon: Some(if copied { "tick02" } else { "copy01" }),
        icon_rect: Rect { x: rect.x + 8.0, y: rect.y + (FENCE_BUTTON_H - GLYPH) / 2.0, w: GLYPH, h: GLYPH },
        icon_only: false,
    }
}

// ---------------------------------------------------------------- painting

fn at(bounds: Bounds<Pixels>, x: f32, y: f32) -> Point<Pixels> {
    point(bounds.left() + px(x), bounds.top() + px(y))
}

fn abs(bounds: Bounds<Pixels>, r: Rect) -> Bounds<Pixels> {
    Bounds::new(at(bounds, r.x, r.y), size(px(r.w), px(r.h)))
}

/// Draws a widget's own background, grid and text. Its embedded element (chart,
/// picture) is drawn separately from [`embed`].
pub fn paint_widget(w: &Widget, bounds: Bounds<Pixels>, t: &Theme, window: &mut Window, cx: &mut App) {
    match &w.body {
        Body::Table(tb) => {
            let frame = abs(bounds, tb.frame);
            let rows = tb.row_edges.len() - 1;
            let cols = tb.col_edges.len() - 1;
            if let Some(r) = tb.hover_row {
                if r < rows {
                    let y0 = tb.row_edges[r];
                    let h = tb.row_edges[r + 1] - y0;
                    window.paint_quad(fill(
                        Bounds::new(at(bounds, tb.frame.x, y0), size(px(tb.frame.w), px(h))),
                        tint(t.accent, 0.08),
                    ));
                }
            }
            // Header shading, then zebra striping on every second body row.
            let header_h = tb.row_edges[1] - tb.row_edges[0];
            window.paint_quad(fill(
                Bounds::new(at(bounds, tb.frame.x, tb.row_edges[0]), size(px(tb.frame.w), px(header_h))),
                t.hover,
            ));
            for r in (2..rows).step_by(2) {
                let y0 = tb.row_edges[r];
                let h = tb.row_edges[r + 1] - y0;
                window.paint_quad(fill(
                    Bounds::new(at(bounds, tb.frame.x, y0), size(px(tb.frame.w), px(h))),
                    t.surface,
                ));
            }
            for r in 1..rows {
                let y = tb.row_edges[r];
                window.paint_quad(fill(
                    Bounds::new(at(bounds, tb.frame.x, y), size(px(tb.frame.w), px(1.0))),
                    t.border,
                ));
            }
            for c in 1..cols {
                let x = tb.col_edges[c];
                window.paint_quad(fill(
                    Bounds::new(at(bounds, x, tb.frame.y), size(px(1.0), px(tb.frame.h))),
                    t.border,
                ));
            }
            window.paint_quad(quad(
                frame,
                px(6.0),
                transparent_black(),
                px(1.0),
                t.border,
                BorderStyle::default(),
            ));
            for row in &tb.cells {
                for cell in row {
                    cell.para.paint(window, cx, at(bounds, cell.x, cell.y));
                }
            }
        }
        Body::Image { rect, source, labels, .. } => {
            if source.is_none() {
                window.paint_quad(quad(
                    abs(bounds, *rect),
                    px(6.0),
                    t.hover,
                    px(0.0),
                    transparent_black(),
                    BorderStyle::default(),
                ));
            }
            for l in labels {
                l.para.paint(window, cx, at(bounds, l.x, l.y));
            }
        }
        Body::Card { rect, labels } => {
            window.paint_quad(quad(
                abs(bounds, *rect),
                px(6.0),
                t.surface,
                px(1.0),
                t.border,
                BorderStyle::default(),
            ));
            for l in labels {
                l.para.paint(window, cx, at(bounds, l.x, l.y));
            }
        }
        Body::Chart { .. } => {}
    }
    for b in &w.buttons {
        let r = abs(bounds, b.rect);
        window.paint_quad(quad(
            r,
            r.size.width.min(r.size.height) / 2.0,
            t.surface,
            px(1.0),
            t.border,
            BorderStyle::default(),
        ));
        paint_button_face(b, bounds, t, window, cx);
    }
}

/// The glyph and label of a control, on top of its already painted frame.
fn paint_button_face(b: &Button, bounds: Bounds<Pixels>, t: &Theme, window: &mut Window, cx: &mut App) {
    if !b.icon_only {
        b.label.paint(window, cx, at(bounds, b.at.0, b.at.1));
    }
    if let Some(name) = b.icon {
        let _ = window.paint_svg(
            abs(bounds, b.icon_rect),
            SharedString::from(format!("icons/{name}.svg")),
            gpui::TransformationMatrix::default(),
            t.muted.into(),
            cx,
        );
    }
}

/// Paints a code fence's copy button.
pub fn paint_button(b: &Button, bounds: Bounds<Pixels>, t: &Theme, window: &mut Window, cx: &mut App) {
    let r = abs(bounds, b.rect);
    window.paint_quad(quad(
        r,
        r.size.width.min(r.size.height) / 2.0,
        t.surface,
        px(1.0),
        t.border,
        BorderStyle::default(),
    ));
    paint_button_face(b, bounds, t, window, cx);
}

/// The element a widget draws with (a chart, a picture), laid out and
/// prepainted at the widget's place. `None` when the widget draws everything
/// itself. Called from prepaint; the result is painted with `.paint`.
pub fn embed(w: &Widget, bounds: Bounds<Pixels>, t: &Theme, window: &mut Window, cx: &mut App) -> Option<AnyElement> {
    let (rect, mut el) = match &w.body {
        Body::Chart { rect, src } => (*rect, chart::render(src, t)),
        Body::Image { rect, source: Some(source), alt, .. } => {
            let (rw, rh) = (rect.w, rect.h);
            let fallback_colour = t.hover;
            let muted = t.muted;
            let shown: SharedString = if alt.is_empty() {
                "[image]".into()
            } else {
                format!("[image] {alt}").into()
            };
            let el = match source {
                ImageSource::File(p) => img(p.clone()),
                ImageSource::Url(u) => img(u.clone()),
            };
            let el = el
                .w(px(rw))
                .h(px(rh))
                .rounded(px(6.0))
                .with_fallback(move || {
                    div()
                        .w(px(rw))
                        .h(px(rh))
                        .rounded(px(6.0))
                        .bg(fallback_colour)
                        .text_color(muted)
                        .text_size(px(12.0))
                        .px(px(10.0))
                        .pt(px(8.0))
                        .child(shown.clone())
                        .into_any_element()
                });
            (*rect, el.into_any_element())
        }
        _ => return None,
    };
    el.prepaint_as_root(
        at(bounds, rect.x, rect.y),
        size(
            AvailableSpace::Definite(px(rect.w)),
            AvailableSpace::Definite(px(rect.h)),
        ),
        window,
        cx,
    );
    Some(el)
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rich_editor::touches;
    use crate::rich_model::parse;

    fn lines_of(src: &str) -> (Model, Vec<Block>, Vec<usize>) {
        let m = parse(src);
        let blocks = m.blocks();
        let mut block_of = vec![0; m.lines.len()];
        for (b, block) in blocks.iter().enumerate() {
            for i in block.lines.clone() {
                block_of[i] = b;
            }
        }
        (m, blocks, block_of)
    }

    #[test]
    fn column_widths_fill_the_available_width() {
        let w = column_widths(&[100.0, 50.0], 56.0, 300.0);
        assert!((w.iter().sum::<f32>() - 300.0).abs() < 1e-3, "{w:?}");
        assert!(w[0] > w[1], "proportional to content");
    }

    #[test]
    fn column_widths_shrink_wide_columns_but_not_below_min() {
        let w = column_widths(&[400.0, 200.0], 56.0, 300.0);
        assert!((w.iter().sum::<f32>() - 300.0).abs() < 1e-3, "{w:?}");
        assert!(w.iter().all(|&x| x >= 56.0));
        let tight = column_widths(&[400.0, 400.0, 400.0], 56.0, 100.0);
        assert!(tight.iter().all(|&x| (x - 56.0).abs() < 1e-3), "min floor holds: {tight:?}");
        assert!(column_widths(&[], 56.0, 10.0).is_empty());
    }

    #[test]
    fn cell_index_maps_points_to_cells() {
        let edges = [0.0, 10.0, 25.0];
        assert_eq!(cell_index(&edges, 0.0), Some(0));
        assert_eq!(cell_index(&edges, 9.9), Some(0));
        assert_eq!(cell_index(&edges, 10.0), Some(1));
        assert_eq!(cell_index(&edges, 24.9), Some(1));
        assert_eq!(cell_index(&edges, 25.0), None, "right edge is outside");
        assert_eq!(cell_index(&edges, -1.0), None);
    }

    #[test]
    fn table_cells_map_to_trimmed_source_ranges() {
        let src = "| a | **Thai ไทย** | c |\n| :-- | --: | :-: |\n|x|y|\n";
        let (m, blocks, _) = lines_of(src);
        let block = blocks.iter().find(|b| b.kind == BlockKind::Table).unwrap();
        let t = parse_table(src, &m.lines[block.lines.clone()]);
        assert_eq!(t.aligns, vec![Align::Left, Align::Right, Align::Center]);
        assert_eq!(t.rows.len(), 2, "header and one body row");
        assert_eq!(&src[t.rows[0][1].clone()], "**Thai ไทย**");
        assert_eq!(&src[t.rows[1][2].clone()], "", "short row is padded with an empty cell");
        assert_eq!(&src[t.rows[1][0].clone()], "x");
        assert_eq!(cell_for(&t, t.rows[0][1].start), Some((0, 1)));
        assert_eq!(cell_for(&t, t.rows[1][1].start + 1), Some((1, 1)));
    }

    #[test]
    fn table_commands_produce_the_expected_source() {
        let src = "| a | b |\n| - | - |\n| 1 | 2 |\n";
        let (m, blocks, _) = lines_of(src);
        let block = blocks.iter().find(|b| b.kind == BlockKind::Table).unwrap();
        let t = parse_table(src, &m.lines[block.lines.clone()]);
        let e = table_op_edit(src, &t, TableOp::AddRow).unwrap();
        let out = e.apply(src);
        assert_eq!(out.lines().count(), 4, "{out:?}");
        let e = table_op_edit(src, &t, TableOp::DelRow(1)).unwrap();
        assert_eq!(e.apply(src).lines().count(), 2);
        assert!(table_op_edit(src, &t, TableOp::DelRow(0)).is_none(), "header is not a body row");
        let e = table_op_edit(src, &t, TableOp::DelCol(1)).unwrap();
        let out = e.apply(src);
        assert!(out.lines().all(|l| l.matches('|').count() == 2), "one column left: {out:?}");
        assert!(out.starts_with("| a"), "{out:?}");
        let e = table_op_edit(src, &t, TableOp::AddCol).unwrap();
        assert_eq!(e.apply(src).lines().next().unwrap().matches('|').count(), 4);
    }

    #[test]
    fn widget_spans_follow_block_kinds_and_the_chart_flag() {
        let src = "text\n\n| a |\n| - |\n| 1 |\n\n```chart\n{}\n```\n\n```mermaid\nflowchart\n```\n\n```rust\nfn x() {}\n```\n\n![pic](a.png)\n";
        let (m, blocks, block_of) = lines_of(src);
        let at_line = |needle: &str| {
            m.lines
                .iter()
                .position(|l| src[l.range.clone()].starts_with(needle))
                .unwrap()
        };
        let table = widget_span(&m, &blocks, &block_of, src, true, at_line("| a |"));
        assert_eq!(table.map(|(_, k)| k), Some(WidgetKind::Table));
        let chart_on = widget_span(&m, &blocks, &block_of, src, true, at_line("```chart"));
        assert_eq!(chart_on.map(|(_, k)| k), Some(WidgetKind::Chart));
        let chart_off = widget_span(&m, &blocks, &block_of, src, false, at_line("```chart"));
        assert!(chart_off.is_none(), "chart fence is a plain code block without the extension");
        let mermaid = widget_span(&m, &blocks, &block_of, src, false, at_line("flowchart") - 1);
        assert_eq!(mermaid.map(|(r, k)| (r.len(), k)), Some((3, WidgetKind::Mermaid)));
        assert!(widget_span(&m, &blocks, &block_of, src, true, at_line("fn x")).is_none());
        let img = widget_span(&m, &blocks, &block_of, src, true, at_line("![pic]"));
        assert_eq!(img.map(|(r, k)| (r, k)), Some((at_line("![pic]")..at_line("![pic]") + 1, WidgetKind::Image)));
    }

    #[test]
    fn image_lines_need_nothing_but_the_image() {
        let src = "![alt text](img/a.png)\n![x](b.png) trailing\nsee ![x](b.png)\n![](c.png)";
        let m = parse(src);
        assert_eq!(
            image_line(src, &m.lines[0]),
            Some(("alt text".to_string(), "img/a.png".to_string()))
        );
        assert_eq!(image_line(src, &m.lines[1]), None, "text after the image");
        assert_eq!(image_line(src, &m.lines[2]), None, "text before the image");
        assert_eq!(image_line(src, &m.lines[3]), Some((String::new(), "c.png".to_string())));
    }

    #[test]
    fn image_size_keeps_aspect_and_fits_the_column() {
        assert_eq!(image_size((800.0, 400.0), 760.0), (760.0, 380.0));
        assert_eq!(image_size((100.0, 50.0), 760.0), (100.0, 50.0), "no upscaling");
        let (w, h) = image_size((400.0, 2000.0), 760.0);
        assert_eq!(h, IMG_MAX_H);
        assert!((w - 112.0).abs() < 1e-3);
    }

    #[test]
    fn remote_and_missing_images_resolve_as_expected() {
        assert_eq!(
            resolve_image("https://x.test/a.png", None),
            Some(ImageSource::Url("https://x.test/a.png".to_string()))
        );
        assert_eq!(resolve_image("data:image/png;base64,AAAA", None), None);
        assert_eq!(resolve_image("missing-file-zyplus.png", Some(Path::new("/nonexistent/dir/doc.md"))), None);
        assert_eq!(resolve_image("a.png", None), None, "no file path: nothing to resolve against");
    }

    #[test]
    fn a_block_is_drawn_as_a_widget_only_while_the_caret_is_away() {
        let src = "para\n\n| a |\n| - |\n";
        let m = parse(src);
        let h = m.lines.iter().position(|l| l.kind == LineKind::TableHeader).unwrap();
        let (first, last) = (m.lines[h].range.start, m.lines[h + 1].range.end);
        let rendered = |sel: Range<usize>| !touches(&sel, &(first..last));
        assert!(rendered(0..0), "caret in the paragraph above");
        assert!(!rendered(first..first), "caret at the table's first cell");
        assert!(!rendered(last..last), "caret at the end of the table");
        assert!(!rendered(first - 2..first + 1), "a selection reaching into the table");
        assert!(rendered(last + 1..last + 1), "caret on the line after the table");
    }

    #[test]
    fn copy_fence_text_is_the_code_without_fences() {
        let src = "```rust\nfn a() {}\n\n  b\n```";
        let m = parse(src);
        let text = fence_text(src, &m.lines, 0..m.lines.len());
        assert_eq!(text, "fn a() {}\n\n  b");
    }
}
