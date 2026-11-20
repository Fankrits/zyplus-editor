//! The WYSIWYG editor behind Rich mode: live preview over the markdown source.
//!
//! The document stays one markdown source string. Every edit is a plain text
//! edit on it, so the file round-trips exactly. What is drawn is derived from
//! the source by `rich_model`:
//!
//! * block markers never appear as text: headings lose their `#`, list items
//!   get bullets, numbers or checkboxes, quotes get bars, rules are lines, code
//!   fences become a panel and callout markers become a label;
//! * inline markers (`**`, `*`, `~~`, backticks, link brackets) are concealed
//!   unless the caret or selection touches the element they belong to.
//!
//! Layout works per source line, the same way `editor` works per line of the
//! plain editor: only lines that intersect the viewport are shaped (cached by
//! content in `wrap_layout::ShapeCache`), and line heights are measured lazily
//! and cached until the next edit. A line's height includes its block margins.
//!
//! Extension point: `RichEditor::plan` decides how a line kind is drawn. A
//! widget that needs a whole block (a table grid, an image, a chart, a
//! mermaid diagram) takes over the block's lines there, and sets its own row
//! height and `Deco`/panel. Until then Table, Html and FrontMatter lines are
//! drawn as monospace source.

#![allow(dead_code)]

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    actions, div, fill, point, px, quad, size, transparent_black, App, BorderStyle, Bounds,
    ClipboardItem, Context, CursorStyle, Element, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, Focusable, Font, FontStyle, FontWeight, GlobalElementId, Hsla,
    InspectorElementId, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PaintQuad, Pixels, Point, Rgba, ScrollWheelEvent, StrikethroughStyle, Style, TextRun,
    UTF16Selection, UnderlineStyle, Window, WindowTextSystem,
};

use crate::editor::{self, mono_font, Scroll};
use crate::highlight::{self, TokenKind};
use crate::rich_blocks::{self, Action, Button, Hover, Widget, WidgetKind};
use crate::rich_model::{
    self, Block, BlockKind, CalloutKind, DisplayLine, InlineKind, InlineStyle, Line, LineKind,
    Model,
};
use crate::rich_ops::{self, Edit, InlineMark, ListType};
use crate::theme::Theme;
use crate::wrap_layout::{ShapeCache, WrappedText};

// ---------------------------------------------------------------- actions

actions!(
    rich_editor,
    [
        ToggleBold,
        ToggleItalic,
        ToggleStrike,
        ToggleCode,
        InsertLink,
        SetParagraph,
        Heading1,
        Heading2,
        Heading3,
        Heading4,
        Heading5,
        Heading6,
        BulletList,
        OrderedList,
        TaskList,
        BlockQuote,
        Indent,
        Outdent,
    ]
);

/// The key context every binding below is registered under.
pub const KEY_CONTEXT: &str = "RichEditor";

// Floating widgets (toolbar, slash menu, link popover, placeholder) live in
// their own file as a child module, so they can use this module's state.
#[path = "rich_overlay.rs"]
mod overlay;
use overlay::{OverlayKey, Overlays};

// ---------------------------------------------------------------- metrics

pub const BODY_SIZE: f32 = 15.5;
/// 1.65 times the body size.
pub const BODY_LH: f32 = 25.6;
const MONO_SIZE: f32 = 13.0;
const MONO_LH: f32 = 20.0;
const LABEL_SIZE: f32 = 11.0;
const LABEL_LH: f32 = 16.0;
/// Callout label icon: square size, and the gap before the label text.
const LABEL_ICON: f32 = 12.0;
const LABEL_ICON_GAP: f32 = 18.0;
/// H1..H6.
const HEADING_SIZE: [f32; 6] = [30.0, 24.0, 20.0, 17.0, 15.0, 14.0];
const COLUMN_MAX: f32 = 760.0;
const SIDE_PAD: f32 = 36.0;
const TOP_PAD: f32 = 24.0;
const BOTTOM_PAD: f32 = 40.0;
/// Height of a blank source line.
const BLANK_H: f32 = 12.0;
const RULE_H: f32 = 24.0;
/// Fence lines with the caret away: padding above and below a code panel.
const FENCE_PAD: f32 = 8.0;
/// A fence with a language keeps room for its label at the top right.
const FENCE_LABEL_H: f32 = 22.0;
const LIST_STEP: f32 = 24.0;
const QUOTE_STEP: f32 = 16.0;
const CALLOUT_PAD: f32 = 18.0;
const CODE_PAD_X: f32 = 14.0;
const CHECK_SIZE: f32 = 14.0;
const BAR_W: f32 = 3.0;
const CARET_WIDTH: f32 = 1.5;
/// Width of the cell drawn for a selected line break.
const NEWLINE_W: f32 = 8.0;
const CODE_CACHE_LIMIT: usize = 128;

type Tokens = Vec<(Range<usize>, TokenKind)>;
type Snapshot = (String, Range<usize>);

/// Timing for the performance notes: set `ZYPLUS_RICH_TIMING=1` and every 60
/// frames the rich editor prints the average and worst time of each stage to
/// stderr. Off by default; the cost when off is one `Option` check.
mod timing {
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::sync::OnceLock;
    use std::time::Instant;

    type Stats = BTreeMap<&'static str, (u64, f64, f64)>;

    thread_local! {
        static STATS: RefCell<Stats> = const { RefCell::new(BTreeMap::new()) };
    }

    pub fn start() -> Option<Instant> {
        static ON: OnceLock<bool> = OnceLock::new();
        let on = *ON.get_or_init(|| std::env::var_os("ZYPLUS_RICH_TIMING").is_some());
        on.then(Instant::now)
    }

    /// Records one stage. `frame` is the stage that closes a frame and prints.
    pub fn record(label: &'static str, started: Option<Instant>) {
        let Some(started) = started else { return };
        let us = started.elapsed().as_secs_f64() * 1e6;
        STATS.with(|stats| {
            let mut stats = stats.borrow_mut();
            let entry = stats.entry(label).or_insert((0, 0.0, 0.0));
            entry.0 += 1;
            entry.1 += us;
            entry.2 = entry.2.max(us);
            if label == "frame" && entry.0 % 60 == 0 {
                let mut line = String::from("[rich-timing]");
                for (name, (n, total, max)) in stats.iter() {
                    line.push_str(&format!(
                        " {name}: avg {:.0}us max {:.0}us (n={n});",
                        total / *n as f64,
                        max
                    ));
                }
                eprintln!("{line}");
            }
        });
    }
}

// ---------------------------------------------------------------- pure helpers

/// A rectangle in element-local px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// The checkbox of a task item. `check_x` is column-relative, `text_top` is the
/// element-local top of the item's first text row.
pub fn checkbox_rect(col_left: f32, check_x: f32, text_top: f32, line_height: f32) -> Rect {
    Rect {
        x: col_left + check_x,
        y: text_top + (line_height - CHECK_SIZE) / 2.0,
        w: CHECK_SIZE,
        h: CHECK_SIZE,
    }
}

/// Points along the tick mark inside a checkbox, in element-local px.
pub fn tick_points(rect: Rect) -> Vec<(f32, f32)> {
    let at = |fx: f32, fy: f32| (rect.x + rect.w * fx, rect.y + rect.h * fy);
    let (a, b, c) = (at(0.22, 0.52), at(0.42, 0.72), at(0.78, 0.30));
    let mut out = Vec::new();
    for (from, to) in [(a, b), (b, c)] {
        let steps = 6;
        for k in 0..=steps {
            let t = k as f32 / steps as f32;
            out.push((from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t));
        }
    }
    out
}

/// Top of each row when stacked from `top0`: `tops[i] = top0 + sum(heights[..i])`.
pub fn cumulative_tops(top0: f32, heights: &[f32]) -> Vec<f32> {
    let mut out = Vec::with_capacity(heights.len());
    let mut y = top0;
    for h in heights {
        out.push(y);
        y += h;
    }
    out
}

/// The row whose `(top, height)` span holds `y`. Points above the first row
/// give row 0 and points below the last give the last row.
pub fn row_for_y(spans: &[(f32, f32)], y: f32) -> Option<usize> {
    if spans.is_empty() {
        return None;
    }
    let k = spans.partition_point(|&(top, h)| top + h <= y);
    Some(k.min(spans.len() - 1))
}

/// Whether a caret or selection touches `r`. Both ends count, the same rule
/// `rich_model::display_line` uses to decide what to reveal.
pub fn touches(sel: &Range<usize>, r: &Range<usize>) -> bool {
    let (a, z) = (sel.start.min(sel.end), sel.start.max(sel.end));
    a <= r.end && r.start <= z
}

/// Moves an offset that landed inside a line's block-marker bytes (`# `, `- `,
/// `> `) out of them: forward goes to the content start, backward to the end
/// of the line above.
pub fn clamp_out_of_marker(model: &Model, offset: usize, forward: bool) -> usize {
    if model.lines.is_empty() {
        return offset;
    }
    let i = model.line_index_at(offset);
    let line = &model.lines[i];
    if line.marker.is_empty() || offset < line.marker.start || offset >= line.content_start {
        return offset;
    }
    if forward || i == 0 {
        line.content_start
    } else {
        model.lines[i - 1].range.end
    }
}

/// Vertical gap after a line, in px. Blank lines supply the paragraph gap
/// themselves, so a gap only appears where two blocks touch.
pub fn gap_after(kind: &LineKind, next: Option<&LineKind>) -> f32 {
    let Some(next) = next else { return 0.0 };
    if *next == LineKind::Blank {
        return 0.0;
    }
    if family(kind) == family(next) {
        return match kind {
            LineKind::ListItem { .. } | LineKind::ListContinuation { .. } => 3.0,
            _ => 0.0,
        };
    }
    8.0
}

/// Which visual family a line belongs to, for `gap_after`.
fn family(kind: &LineKind) -> u8 {
    match kind {
        LineKind::Blank => 0,
        LineKind::Paragraph | LineKind::SetextUnderline => 1,
        LineKind::Heading(_) => 2,
        LineKind::Quote { callout: None, .. } => 3,
        LineKind::Quote {
            callout: Some(_), ..
        } => 4,
        LineKind::ListItem { .. } | LineKind::ListContinuation { .. } => 5,
        LineKind::FenceOpen { .. }
        | LineKind::CodeLine
        | LineKind::FenceClose
        | LineKind::IndentedCode => 6,
        LineKind::Rule => 7,
        LineKind::TableHeader | LineKind::TableDelimiter | LineKind::TableRow => 8,
        LineKind::Html | LineKind::FrontMatter => 9,
    }
}

/// Splits `0..len` into consecutive pieces, each with the span that covers it
/// (`None` where no span does). Spans must be sorted and disjoint.
pub fn cover<T: Clone>(len: usize, spans: &[(Range<usize>, T)]) -> Vec<(usize, Option<T>)> {
    let mut out = Vec::new();
    let mut pos = 0;
    for (r, value) in spans {
        let s = r.start.max(pos);
        let e = r.end.min(len);
        if s >= e {
            continue;
        }
        if s > pos {
            out.push((s - pos, None));
        }
        out.push((e - s, Some(value.clone())));
        pos = e;
    }
    if pos < len {
        out.push((len - pos, None));
    }
    out
}

/// Line endings to LF, as pasted text is inserted.
pub fn normalise_newlines(text: &str) -> String {
    if text.contains('\r') {
        text.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        text.to_string()
    }
}

fn label_text(kind: CalloutKind) -> &'static str {
    match kind {
        CalloutKind::Note => "Note",
        CalloutKind::Tip => "Tip",
        CalloutKind::Important => "Important",
        CalloutKind::Warning => "Warning",
        CalloutKind::Caution => "Caution",
    }
}

/// Icon drawn before a callout's label (GitHub-alert icons).
fn label_icon(kind: CalloutKind) -> &'static str {
    match kind {
        CalloutKind::Note | CalloutKind::Tip => "information-circle",
        CalloutKind::Important => "alert-circle",
        CalloutKind::Warning | CalloutKind::Caution => "alert01",
    }
}

fn callout_colour(kind: CalloutKind, t: &Theme) -> Rgba {
    match kind {
        CalloutKind::Note | CalloutKind::Tip | CalloutKind::Important => t.accent,
        CalloutKind::Warning | CalloutKind::Caution => t.danger,
    }
}

fn tint(colour: Rgba, alpha: f32) -> Hsla {
    let mut h: Hsla = colour.into();
    h.a = alpha;
    h
}

fn heading_size(level: u8) -> f32 {
    HEADING_SIZE[(level.clamp(1, 6) - 1) as usize]
}

/// Width in px of a single-row paragraph.
fn text_width(p: &WrappedText) -> f32 {
    f32::from(p.position_for_index(p.len(), false).x)
}

fn hash_of(parts: &[&str]) -> u64 {
    let mut h = DefaultHasher::new();
    for p in parts {
        p.hash(&mut h);
    }
    h.finish()
}

// ---------------------------------------------------------------- layout types

#[derive(Clone, Copy, Debug, PartialEq)]
enum Role {
    Body,
    Heading(u8),
    Muted,
    Code,
    Quote,
    Label(CalloutKind),
}

fn role_colour(role: Role, t: &Theme) -> Rgba {
    match role {
        Role::Body => t.fg,
        Role::Heading(_) => t.heading,
        Role::Muted => t.muted,
        Role::Code => t.code,
        Role::Quote => t.quote,
        Role::Label(kind) => callout_colour(kind, t),
    }
}

/// A decoration drawn by a row, besides its text.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Deco {
    None,
    /// A horizontal rule across the column.
    Rule,
    /// Quote bars, one per nesting level.
    Quote(u8),
    /// The line under an H1 or H2.
    HeadingRule,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ListMark {
    None,
    /// Nesting level.
    Bullet(usize),
    /// Nesting level and the source number.
    Number(usize, u64),
    /// Nesting level and checked state.
    Check(usize, bool),
}

/// How one source line is drawn. Built from the line kind, the caret and the
/// fence/block state by `RichEditor::plan`.
#[derive(Clone, Debug)]
struct Plan {
    size: f32,
    lh: f32,
    mt: f32,
    mb: f32,
    mono: bool,
    bold: bool,
    role: Role,
    /// Text x inside the column.
    x: f32,
    /// Fixed row height for rows that show no text.
    collapsed: Option<f32>,
    /// Show the source line instead of the rendered text.
    raw: bool,
    /// Show a callout label instead of the (concealed) marker line.
    label: Option<CalloutKind>,
    deco: Deco,
    /// Text takes syntax colours from its code block.
    code: bool,
    list: ListMark,
}

impl Plan {
    fn body(gap: f32, x: f32) -> Self {
        Plan {
            size: BODY_SIZE,
            lh: BODY_LH,
            mt: 0.0,
            mb: gap,
            mono: false,
            bold: false,
            role: Role::Body,
            x,
            collapsed: None,
            raw: false,
            label: None,
            deco: Deco::None,
            code: false,
            list: ListMark::None,
        }
    }

    fn raw_mono(gap: f32, role: Role) -> Self {
        Plan {
            size: MONO_SIZE,
            lh: MONO_LH,
            mono: true,
            role,
            raw: true,
            ..Plan::body(gap, 0.0)
        }
    }
}

/// A shaped label drawn beside a row: a bullet, a number, a fence language.
struct Mark {
    para: Rc<WrappedText>,
    /// Element-local origin.
    x: f32,
    y: f32,
}

/// One visible source line, laid out.
struct Row {
    index: usize,
    /// Element-local top of the line box (margins included).
    top: f32,
    height: f32,
    /// Element-local top of the text.
    text_top: f32,
    /// Element-local x of the text origin.
    text_x: f32,
    col_left: f32,
    col_w: f32,
    para: Rc<WrappedText>,
    /// Display text and its map back to the source.
    disp: DisplayLine,
    deco: Deco,
    /// Task checkbox state and rect, element-local.
    check: Option<(bool, Rect)>,
    marks: Vec<Mark>,
    block: usize,
    /// A block widget that starts on this line (rendered, not raw).
    widget: Option<Widget>,
    /// The widget's element (chart, picture), prepainted in prepaint.
    embed: Option<gpui::AnyElement>,
    /// Controls drawn on this row (a code fence's copy button).
    buttons: Vec<Button>,
    /// A later line of a rendered widget: takes no height, draws nothing.
    widget_part: bool,
    /// Callout kind of a label row: its icon is painted before the text.
    icon: Option<CalloutKind>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum PanelKind {
    Code,
    Callout(CalloutKind),
    FrontMatter,
}

/// A block background (code panel, callout card), element-local.
#[derive(Clone, Copy, Debug)]
struct Panel {
    kind: PanelKind,
    rect: Rect,
}

#[derive(Default)]
struct Quads {
    panels: Vec<PaintQuad>,
    search: Vec<PaintQuad>,
    selection: Vec<PaintQuad>,
    decor: Vec<PaintQuad>,
    caret: Option<PaintQuad>,
}

/// What one layout pass shapes with.
struct Frame {
    ts: Arc<WindowTextSystem>,
    body: Font,
    mono: Font,
    theme: Theme,
    col_left: f32,
    col_w: f32,
    /// Syntax tokens per code block, as absolute source ranges.
    code: HashMap<usize, Rc<Tokens>>,
}

#[derive(Default)]
struct LayoutState {
    shapes: ShapeCache,
    /// Hash of the column width and font the heights were measured with.
    key: u64,
    /// Measured height of each line, by line index.
    heights: Vec<Option<f32>>,
    /// Tokens of whole code blocks, by hash of language and text.
    code: HashMap<u64, Rc<Tokens>>,
}

/// Prepaint output: the rows, the quads under the text and the palette the
/// row controls are painted with.
struct Prepaint {
    rows: Vec<Row>,
    quads: Quads,
    theme: Theme,
}

// ---------------------------------------------------------------- the entity

pub struct RichEditor {
    focus: FocusHandle,
    content: String,
    model: Model,
    blocks: Vec<Block>,
    /// Block index of every line.
    block_of: Vec<usize>,
    /// Byte offsets into `content`, always on a char boundary.
    selection: Range<usize>,
    reversed: bool,
    marked: Option<Range<usize>>,
    scroll: Scroll,
    follow_caret: bool,
    scroll_px: f32,
    view_height: f32,
    view_width: f32,
    /// Column x of the caret kept by Up/Down, in element-local px.
    desired_x: Option<f32>,
    /// The caret sits at the end of the row above a soft wrap.
    at_row_end: bool,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    last_edit: Option<Instant>,
    /// The last edit was plain typing, so the next typed character may join
    /// its undo step. Commands, pastes and caret moves end the run.
    typing_run: bool,
    last_bounds: Option<Bounds<Pixels>>,
    /// The rows the last paint drew, for hit-testing and IME.
    last_rows: Vec<Row>,
    /// Find matches, sorted by start, in source offsets.
    search_ranges: Vec<Range<usize>>,
    search_active: Option<Range<usize>>,
    selecting: bool,
    /// Palette id from `theme::THEMES`.
    pub theme_name: String,
    layout: LayoutState,
    /// Widget state: file path, chart switch, hover, last copy.
    widgets: rich_blocks::BlockState,
    /// Called with the whole source after every edit.
    pub on_change: Option<Box<dyn Fn(&str, &mut App)>>,
    pub on_cancel: Option<Box<dyn Fn(&mut App)>>,
    /// Called with a chart or Mermaid fence's kind and source when its full-screen button is pressed.
    pub on_fullscreen: Option<Box<dyn Fn(WidgetKind, &str, &mut App)>>,
    overlays: Overlays,
}

impl RichEditor {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            focus: cx.focus_handle(),
            content: String::new(),
            model: Model::default(),
            blocks: Vec::new(),
            block_of: Vec::new(),
            selection: 0..0,
            reversed: false,
            marked: None,
            scroll: Scroll::default(),
            follow_caret: false,
            scroll_px: 0.0,
            view_height: 0.0,
            view_width: 0.0,
            desired_x: None,
            at_row_end: false,
            undo: Vec::new(),
            redo: Vec::new(),
            last_edit: None,
            typing_run: false,
            last_bounds: None,
            last_rows: Vec::new(),
            search_ranges: Vec::new(),
            search_active: None,
            selecting: false,
            theme_name: "system".to_string(),
            layout: LayoutState::default(),
            widgets: rich_blocks::BlockState::default(),
            on_change: None,
            on_cancel: None,
            on_fullscreen: None,
            overlays: Overlays::new(cx),
        };
        this.reparse();
        this
    }

    /// Registers every key binding of this editor under [`KEY_CONTEXT`]. The
    /// navigation and clipboard keys reuse the plain editor's actions.
    pub fn bind_keys(cx: &mut App) {
        cx.bind_keys(crate::keymap::rich_bindings(crate::platform::Os::current()));
    }

    // ---- public surface (same shape as `Editor`) -------------------------

    pub fn text(&self) -> &str {
        &self.content
    }

    /// Facts for the debug script's `state` dump, one `key=value` per line.
    pub fn state_summary(&self) -> String {
        let cursor = self.cursor();
        format!(
            "selection={}..{}\ncursor={}\nlen={}\nlines={}\nscroll_line={}\nscroll_offset={:.1}\nundo={}\nredo={}\n",
            self.selection.start,
            self.selection.end,
            cursor,
            self.content.len(),
            self.model.lines.len(),
            self.scroll.line,
            self.scroll.offset,
            self.undo.len(),
            self.redo.len(),
        )
    }

    /// The caret or selection, in source offsets.
    pub fn selection(&self) -> Range<usize> {
        self.selection.clone()
    }

    pub fn set_theme(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.theme_name != name {
            self.theme_name = name.to_string();
            cx.notify();
        }
    }

    /// The markdown file this document belongs to. Relative image paths
    /// resolve against its folder.
    pub fn set_file_path(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.widgets.file_path != path {
            self.widgets.file_path = path;
            self.layout.heights.clear();
            cx.notify();
        }
    }

    /// Whether ```chart fences draw as charts (the chart extension).
    pub fn set_charts_enabled(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.widgets.charts != on {
            self.widgets.charts = on;
            self.layout.heights.clear();
            cx.notify();
        }
    }

    pub fn charts_enabled(&self) -> bool {
        self.widgets.charts
    }

    /// Whether ```mermaid fences draw as diagrams (the mermaid extension).
    pub fn set_mermaid_enabled(&mut self, on: bool, cx: &mut Context<Self>) {
        rich_blocks::set_mermaid_enabled(on);
        if self.widgets.mermaid != on {
            self.widgets.mermaid = on;
            self.layout.heights.clear();
            cx.notify();
        }
    }

    pub fn mermaid_enabled(&self) -> bool {
        self.widgets.mermaid
    }

    /// Whether ```latex, ```math, ```katex, ```tex fences draw as math (the katex extension).
    pub fn set_katex_enabled(&mut self, on: bool, cx: &mut Context<Self>) {
        rich_blocks::set_katex_enabled(on);
        if self.widgets.katex != on {
            self.widgets.katex = on;
            self.layout.heights.clear();
            cx.notify();
        }
    }

    pub fn katex_enabled(&self) -> bool {
        self.widgets.katex
    }

    /// Replaces the document. Resets undo and does not report a change.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.content = text.to_string();
        self.selection = 0..0;
        self.reversed = false;
        self.marked = None;
        self.at_row_end = false;
        self.desired_x = None;
        self.undo.clear();
        self.redo.clear();
        self.scroll = Scroll::default();
        self.scroll_px = 0.0;
        self.follow_caret = false;
        self.reparse();
        cx.notify();
    }

    pub fn set_search(
        &mut self,
        mut ranges: Vec<Range<usize>>,
        active: Option<Range<usize>>,
        cx: &mut Context<Self>,
    ) {
        ranges.sort_by_key(|range| range.start);
        self.search_ranges = ranges;
        self.search_active = active;
        cx.notify();
    }

    /// Every case-insensitive, non-overlapping match of `query`, in source offsets.
    pub fn find_ranges(&self, query: &str) -> Vec<Range<usize>> {
        if query.is_empty() {
            return Vec::new();
        }
        let haystack_owned;
        let haystack: &str = {
            let folded = self.content.to_lowercase();
            if folded.len() == self.content.len() {
                haystack_owned = folded;
                &haystack_owned
            } else {
                &self.content
            }
        };
        let needle = query.to_lowercase();
        let mut ranges = Vec::new();
        let mut from = 0;
        while let Some(found) = haystack[from..].find(&needle) {
            let start = from + found;
            let end = start + needle.len();
            ranges.push(start..end);
            from = end.max(start + 1);
        }
        ranges
    }

    /// Puts the selection on `range` and scrolls it into view.
    pub fn select(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        let start = range.start.min(self.content.len());
        let end = range.end.min(self.content.len());
        self.selection = start..end;
        self.reversed = false;
        self.at_row_end = false;
        self.desired_x = None;
        self.follow_caret = true;
        cx.notify();
    }

    /// Replaces one range, as an undoable edit.
    pub fn replace_at(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) {
        self.splice(range, text, false, cx);
    }

    /// Replaces every match of `query` in one undo step. Returns the count.
    pub fn replace_all_matches(
        &mut self,
        query: &str,
        replacement: &str,
        cx: &mut Context<Self>,
    ) -> usize {
        let ranges = self.find_ranges(query);
        if ranges.is_empty() {
            return 0;
        }
        let mut next = String::with_capacity(self.content.len());
        let mut last = 0;
        for range in &ranges {
            next.push_str(&self.content[last..range.start]);
            next.push_str(replacement);
            last = range.end;
        }
        next.push_str(&self.content[last..]);
        let caret = next.len();
        self.apply_edit(
            Edit {
                range: 0..self.content.len(),
                insert: next,
                selection: caret..caret,
            },
            cx,
        );
        self.search_ranges.clear();
        self.search_active = None;
        ranges.len()
    }

    /// Applies a command from `rich_ops` as one undoable edit.
    pub fn apply_edit(&mut self, edit: Edit, cx: &mut Context<Self>) {
        self.commit(edit, false, cx);
    }

    /// Applies an edit. `typing` edits may join the undo step of the typing
    /// run before them; everything else starts a step of its own.
    fn commit(&mut self, edit: Edit, typing: bool, cx: &mut Context<Self>) {
        let started = timing::start();
        let next = edit.apply(&self.content);
        if next == self.content {
            // An identity edit (Tab on an item that cannot move) keeps focus put.
            let len = self.content.len();
            self.selection = edit.selection.start.min(len)..edit.selection.end.min(len);
            self.reversed = false;
            self.desired_x = None;
            self.follow_caret = true;
            cx.notify();
            return;
        }
        self.push_undo(typing);
        self.content = next;
        let len = self.content.len();
        self.selection = edit.selection.start.min(len)..edit.selection.end.min(len);
        self.reversed = false;
        self.marked = None;
        self.at_row_end = false;
        self.desired_x = None;
        self.reparse();
        timing::record("edit", started);
        self.overlay_after_edit(&edit, typing);
        self.follow_caret = true;
        self.notify_changed(cx);
        cx.notify();
    }

    pub fn toggle_bold(&mut self, cx: &mut Context<Self>) {
        self.mark(InlineMark::Bold, cx);
    }

    pub fn toggle_italic(&mut self, cx: &mut Context<Self>) {
        self.mark(InlineMark::Italic, cx);
    }

    pub fn toggle_strike(&mut self, cx: &mut Context<Self>) {
        self.mark(InlineMark::Strike, cx);
    }

    pub fn toggle_code(&mut self, cx: &mut Context<Self>) {
        self.mark(InlineMark::Code, cx);
    }

    pub fn insert_link(&mut self, cx: &mut Context<Self>) {
        let edit = rich_ops::insert_link(&self.content, self.selection.clone(), None);
        self.run(edit, cx);
    }

    /// Heading of `level` 1..=6, or a paragraph for 0. Applying the level a
    /// line already has removes it.
    pub fn set_heading(&mut self, level: u8, cx: &mut Context<Self>) {
        let edit = if level == 0 {
            rich_ops::set_paragraph(&self.content, self.selection.clone())
        } else {
            rich_ops::set_heading(&self.content, self.selection.clone(), Some(level))
        };
        self.run(edit, cx);
    }

    pub fn toggle_list(&mut self, kind: ListType, cx: &mut Context<Self>) {
        let edit = rich_ops::toggle_list(&self.content, self.selection.clone(), kind);
        self.run(edit, cx);
    }

    pub fn toggle_quote(&mut self, cx: &mut Context<Self>) {
        let edit = rich_ops::toggle_quote(&self.content, self.selection.clone());
        self.run(edit, cx);
    }

    pub fn insert_table(&mut self, rows: usize, cols: usize, cx: &mut Context<Self>) {
        let edit = rich_ops::insert_table(&self.content, self.selection.clone(), rows, cols);
        self.run(edit, cx);
    }

    pub fn insert_code_block(&mut self, lang: &str, cx: &mut Context<Self>) {
        let edit = rich_ops::insert_code_block(&self.content, self.selection.clone(), lang);
        self.run(edit, cx);
    }

    pub fn insert_rule(&mut self, cx: &mut Context<Self>) {
        let edit = rich_ops::insert_rule(&self.content, self.selection.clone());
        self.run(edit, cx);
    }

    pub fn undo(&mut self, cx: &mut Context<Self>) {
        if let Some((content, selection)) = self.undo.pop() {
            self.redo
                .push((self.content.clone(), self.selection.clone()));
            self.restore(content, selection, cx);
        }
    }

    pub fn redo(&mut self, cx: &mut Context<Self>) {
        if let Some((content, selection)) = self.redo.pop() {
            self.undo
                .push((self.content.clone(), self.selection.clone()));
            self.restore(content, selection, cx);
        }
    }

    // ---- document state ---------------------------------------------------

    fn reparse(&mut self) {
        let mut model = rich_model::parse(&self.content);
        if model.lines.is_empty() {
            model.lines.push(Line {
                range: 0..0,
                content_start: 0,
                marker: 0..0,
                kind: LineKind::Blank,
                inlines: Vec::new(),
            });
        }
        self.blocks = model.blocks();
        self.block_of = vec![0; model.lines.len()];
        for (b, block) in self.blocks.iter().enumerate() {
            for i in block.lines.clone() {
                if let Some(slot) = self.block_of.get_mut(i) {
                    *slot = b;
                }
            }
        }
        self.model = model;
        self.layout.heights.clear();
        let len = self.content.len();
        self.selection = self.selection.start.min(len)..self.selection.end.min(len);
    }

    fn mark(&mut self, mark: InlineMark, cx: &mut Context<Self>) {
        let edit = rich_ops::toggle_inline(&self.content, self.selection.clone(), mark);
        self.run(edit, cx);
    }

    fn run(&mut self, edit: Option<Edit>, cx: &mut Context<Self>) {
        if let Some(edit) = edit {
            self.apply_edit(edit, cx);
        }
    }

    fn cursor(&self) -> usize {
        if self.reversed {
            self.selection.start
        } else {
            self.selection.end
        }
    }

    fn line_of(&self, offset: usize) -> usize {
        self.model.line_index_at(offset)
    }

    fn prev_boundary(&self, offset: usize) -> usize {
        rich_ops::prev_grapheme(&self.content, offset)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        rich_ops::next_grapheme(&self.content, offset)
    }

    /// Records a restore point. A typed character joins the step of the typing
    /// run before it when the run is unbroken and quick; any other edit starts
    /// a new step.
    fn push_undo(&mut self, typing: bool) {
        let now = Instant::now();
        let quick = match self.last_edit {
            Some(last) => now.duration_since(last) < Duration::from_millis(650),
            None => false,
        };
        let coalesce = typing && self.typing_run && quick;
        if !coalesce {
            self.undo
                .push((self.content.clone(), self.selection.clone()));
            if self.undo.len() > 250 {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
        self.last_edit = Some(now);
        self.typing_run = typing;
    }

    fn restore(&mut self, content: String, selection: Range<usize>, cx: &mut Context<Self>) {
        self.content = content;
        self.selection = selection;
        self.reversed = false;
        self.at_row_end = false;
        self.desired_x = None;
        self.last_edit = None;
        self.typing_run = false;
        self.reparse();
        self.follow_caret = true;
        self.notify_changed(cx);
        cx.notify();
    }

    fn notify_changed(&mut self, cx: &mut Context<Self>) {
        if let Some(on_change) = self.on_change.as_ref() {
            on_change(&self.content, cx);
        }
    }

    /// Typed text, deletions and IME input: these may join a typing run.
    fn replace_range(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) {
        if let Some((lines, table, byte_range)) = self.table_info_at(range.start) {
            if range.end <= byte_range.end {
                let clean_text = crate::rich_table::sanitize_cell_text(text);
                let start_cell = rich_blocks::cell_for(&table, range.start);
                let end_cell = rich_blocks::cell_for(&table, range.end);
                if start_cell.is_some() && end_cell.is_some() && start_cell != end_cell {
                    let edit = crate::rich_table::multi_cell_replace(
                        &self.content,
                        &table,
                        range,
                        &clean_text,
                    );
                    self.commit(edit, true, cx);
                    return;
                }
                if let Some((r, c)) = start_cell.or_else(|| {
                    crate::rich_table::cell_at_offset(&table, &self.model.lines[lines], range.start)
                }) {
                    let cell_src = table.rows[r][c].clone();
                    let clamped_start = range.start.clamp(cell_src.start, cell_src.end);
                    let clamped_end = range.end.clamp(cell_src.start, cell_src.end);
                    self.splice(clamped_start..clamped_end, &clean_text, true, cx);
                    return;
                }
            }
        }
        // Text typed on a closing fence or a rule goes on a new line after the
        // block, never onto its delimiter line.
        if range.is_empty() && !text.is_empty() {
            if let Some(le) = rich_ops::block_guard_line_end(&self.content, range.start) {
                let typed = format!("\n{text}");
                self.splice(le..le, &typed, true, cx);
                return;
            }
        }
        self.splice(range, text, true, cx);
    }

    fn splice(&mut self, range: Range<usize>, text: &str, typing: bool, cx: &mut Context<Self>) {
        let caret = range.start + text.len();
        self.commit(
            Edit {
                range,
                insert: text.to_string(),
                selection: caret..caret,
            },
            typing,
            cx,
        );
    }

    /// Moves the caret to `offset`, or extends the selection to it when
    /// `extend`. `keep_x` keeps the Up/Down column (only vertical moves do).
    fn place(
        &mut self,
        offset: usize,
        at_row_end: bool,
        extend: bool,
        keep_x: bool,
        cx: &mut Context<Self>,
    ) {
        let offset = if extend {
            offset
        } else {
            self.leave_table(offset..offset, cx).start
        };
        if extend {
            if self.reversed {
                self.selection.start = offset;
            } else {
                self.selection.end = offset;
            }
            if self.selection.end < self.selection.start {
                self.reversed = !self.reversed;
                self.selection = self.selection.end..self.selection.start;
            }
        } else {
            self.selection = offset..offset;
            self.reversed = false;
        }
        self.at_row_end = at_row_end;
        if !keep_x {
            self.desired_x = None;
        }
        self.typing_run = false;
        self.follow_caret = true;
        cx.notify();
    }

    fn set_selection(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        let range = self.leave_table(range, cx);
        self.typing_run = false;
        self.selection = range;
        self.reversed = false;
        self.at_row_end = false;
        self.desired_x = None;
        self.follow_caret = true;
        cx.notify();
    }

    /// Whether the caret or selection touches any line of block `block`.
    fn block_touched(&self, block: usize) -> bool {
        let Some(b) = self.blocks.get(block) else {
            return false;
        };
        let (Some(first), Some(last)) = (
            self.model.lines.get(b.lines.start),
            self.model.lines.get(b.lines.end.max(b.lines.start + 1) - 1),
        ) else {
            return false;
        };
        touches(&self.selection, &(first.range.start..last.range.end))
    }

    // ---- planning and shaping ---------------------------------------------

    /// How line `i` is drawn. Block markers are never text: the plan decides
    /// the decoration instead.
    fn plan(&self, i: usize, line: &Line, touched: bool, block_touched: bool) -> Plan {
        let next = self.model.lines.get(i + 1).map(|l| &l.kind);
        let gap = gap_after(&line.kind, next);
        match &line.kind {
            LineKind::Blank => Plan {
                collapsed: Some(BLANK_H),
                ..Plan::body(0.0, 0.0)
            },
            LineKind::Paragraph => Plan::body(gap, 0.0),
            LineKind::Heading(level) => {
                let level = (*level).clamp(1, 6);
                let size = heading_size(level);
                Plan {
                    size,
                    lh: size * 1.3,
                    mt: if level <= 2 { 10.0 } else { 6.0 },
                    mb: 8.0,
                    bold: true,
                    role: Role::Heading(level),
                    deco: if level <= 2 {
                        Deco::HeadingRule
                    } else {
                        Deco::None
                    },
                    ..Plan::body(0.0, 0.0)
                }
            }
            LineKind::SetextUnderline => {
                if touched {
                    Plan::raw_mono(gap, Role::Muted)
                } else {
                    Plan {
                        collapsed: Some(0.0),
                        ..Plan::body(0.0, 0.0)
                    }
                }
            }
            LineKind::Quote {
                depth,
                callout: None,
            } => {
                let depth = *depth;
                Plan {
                    role: Role::Quote,
                    deco: Deco::Quote(depth),
                    ..Plan::body(gap, depth as f32 * QUOTE_STEP + 6.0)
                }
            }
            LineKind::Quote {
                callout: Some(kind),
                ..
            } => {
                let first = self.blocks.get(self.block_of[i]).map(|b| b.lines.start) == Some(i);
                let no_title = rich_model::display_line(&self.content, line, None)
                    .text
                    .trim()
                    .is_empty();
                if first && no_title {
                    if touched {
                        Plan::raw_mono(gap, Role::Muted)
                    } else {
                        Plan {
                            size: LABEL_SIZE,
                            lh: LABEL_LH,
                            mt: 8.0,
                            mb: 2.0,
                            bold: true,
                            role: Role::Label(*kind),
                            label: Some(*kind),
                            ..Plan::body(0.0, CALLOUT_PAD)
                        }
                    }
                } else {
                    Plan::body(gap, CALLOUT_PAD)
                }
            }
            LineKind::ListItem {
                indent,
                ordered,
                number,
                task,
            } => {
                let level = *indent;
                Plan {
                    list: match task {
                        Some(done) => ListMark::Check(level, *done),
                        None if *ordered => ListMark::Number(level, *number),
                        None => ListMark::Bullet(level),
                    },
                    ..Plan::body(gap, level as f32 * LIST_STEP + LIST_STEP)
                }
            }
            LineKind::ListContinuation { indent } => {
                Plan::body(gap, *indent as f32 * LIST_STEP + LIST_STEP)
            }
            LineKind::FenceOpen { lang: _ } => {
                if block_touched {
                    Plan {
                        role: Role::Muted,
                        x: CODE_PAD_X,
                        code: false,
                        ..Plan::raw_mono(gap, Role::Muted)
                    }
                } else {
                    // Room for the language label and the copy button.
                    Plan {
                        collapsed: Some(FENCE_LABEL_H),
                        ..Plan::body(0.0, CODE_PAD_X)
                    }
                }
            }
            LineKind::FenceClose => {
                if block_touched {
                    Plan::raw_mono(gap, Role::Muted)
                } else {
                    Plan {
                        collapsed: Some(FENCE_PAD),
                        ..Plan::body(0.0, CODE_PAD_X)
                    }
                }
            }
            LineKind::CodeLine | LineKind::IndentedCode => Plan {
                size: MONO_SIZE,
                lh: MONO_LH,
                mono: true,
                role: Role::Code,
                code: true,
                ..Plan::body(gap, CODE_PAD_X)
            },
            LineKind::Rule => {
                if touched {
                    Plan::raw_mono(gap, Role::Muted)
                } else {
                    Plan {
                        collapsed: Some(RULE_H),
                        deco: Deco::Rule,
                        ..Plan::body(0.0, 0.0)
                    }
                }
            }
            // Block widgets (tables, images, cards) replace these arms. Until
            // then their source is shown in monospace.
            LineKind::TableHeader | LineKind::TableDelimiter | LineKind::TableRow => {
                Plan::raw_mono(gap, Role::Code)
            }
            LineKind::Html => Plan::raw_mono(gap, Role::Muted),
            // Front matter sits on a bordered panel, so its text is inset.
            LineKind::FrontMatter => Plan {
                x: CODE_PAD_X,
                ..Plan::raw_mono(gap, Role::Muted)
            },
        }
    }

    /// The text a line displays, with the source offsets it maps to.
    fn display_for(&self, line: &Line, plan: &Plan, caret: Option<Range<usize>>) -> DisplayLine {
        if let Some(kind) = plan.label {
            let text = label_text(kind).to_string();
            let n = text.len();
            return DisplayLine {
                text,
                to_source: vec![line.content_start; n + 1],
                styles: Vec::new(),
            };
        }
        if plan.raw {
            let r = line.range.clone();
            return DisplayLine {
                text: self.content[r.clone()].to_string(),
                to_source: (r.start..=r.end).collect(),
                styles: Vec::new(),
            };
        }
        if plan.collapsed.is_some() || plan.deco == Deco::Rule {
            return DisplayLine {
                text: String::new(),
                to_source: vec![line.content_start],
                styles: Vec::new(),
            };
        }
        rich_model::display_line(&self.content, line, caret)
    }

    fn inline_runs(&self, disp: &DisplayLine, plan: &Plan, f: &Frame) -> Vec<TextRun> {
        cover(disp.text.len(), &disp.styles)
            .into_iter()
            .map(|(len, st)| inline_run(len, st.unwrap_or_default(), plan, f))
            .collect()
    }

    /// Runs for a code row, coloured from its block's syntax tokens.
    fn code_runs(
        &mut self,
        f: &mut Frame,
        block: usize,
        plan: &Plan,
        disp: &DisplayLine,
    ) -> Vec<TextRun> {
        let len = disp.text.len();
        if len == 0 {
            return Vec::new();
        }
        let base = disp.to_source.first().copied().unwrap_or(0);
        let tokens = self.block_tokens(f, block);
        let mut spans: Vec<(Range<usize>, TokenKind)> = Vec::new();
        let lo = tokens.partition_point(|(r, _)| r.end <= base);
        for (r, kind) in &tokens[lo..] {
            if r.start >= base + len {
                break;
            }
            if r.end <= base {
                continue;
            }
            let s = r.start.saturating_sub(base);
            let e = (r.end - base).min(len);
            spans.push((s..e, *kind));
        }
        cover(len, &spans)
            .into_iter()
            .map(|(n, kind)| code_run(n, kind.unwrap_or(TokenKind::Plain), plan, f))
            .collect()
    }

    /// Syntax tokens of a code block, as absolute source ranges. Cached by
    /// hash of language and text, and per frame.
    fn block_tokens(&mut self, f: &mut Frame, block: usize) -> Rc<Tokens> {
        if let Some(hit) = f.code.get(&block) {
            return hit.clone();
        }
        let empty: Rc<Tokens> = Rc::new(Vec::new());
        let Some(b) = self.blocks.get(block) else {
            return empty;
        };
        let BlockKind::CodeBlock { lang } = &b.kind else {
            return empty;
        };
        let lang = lang.clone();
        let mut first = None;
        let mut last = None;
        for l in &self.model.lines[b.lines.clone()] {
            if matches!(l.kind, LineKind::CodeLine | LineKind::IndentedCode) {
                first.get_or_insert(l.range.start);
                last = Some(l.range.end);
            }
        }
        let tokens: Rc<Tokens> = match (first, last, highlight::language_for_fence(&lang)) {
            (Some(a), Some(z), Some(language)) => {
                let text = &self.content[a..z];
                let key = hash_of(&[language, text]);
                match self.layout.code.get(&key) {
                    Some(hit) => hit.clone(),
                    None => {
                        let shifted: Tokens = highlight::highlight(language, text)
                            .into_iter()
                            .map(|(r, k)| (r.start + a..r.end + a, k))
                            .collect();
                        if self.layout.code.len() >= CODE_CACHE_LIMIT {
                            self.layout.code.clear();
                        }
                        let rc = Rc::new(shifted);
                        self.layout.code.insert(key, rc.clone());
                        rc
                    }
                }
            }
            _ => empty,
        };
        f.code.insert(block, tokens.clone());
        tokens
    }

    /// Builds one row. `top` is its element-local top; measuring a row for
    /// heights uses `top = 0`.
    fn build_row(&mut self, f: &mut Frame, i: usize, top: f32) -> Row {
        if let Some((lines, kind)) = self.rendered_widget(i) {
            return self.widget_row(f, i, lines, kind, top);
        }
        let line = self.model.lines[i].clone();
        let touched = touches(&self.selection, &line.range);
        let block = self.block_of[i];
        let block_touched = self.block_touched(block);
        let plan = self.plan(i, &line, touched, block_touched);
        let caret = touched.then(|| self.selection.clone());
        let disp = self.display_for(&line, &plan, caret);
        let runs = if plan.code {
            self.code_runs(f, block, &plan, &disp)
        } else {
            self.inline_runs(&disp, &plan, f)
        };
        let width = (f.col_w - plan.x).max(24.0);
        let para = self.layout.shapes.get_or_shape(
            &f.ts,
            &disp.text,
            px(plan.size),
            &runs,
            Some(px(width)),
            px(plan.lh),
        );
        let height = match plan.collapsed {
            Some(h) => h,
            None => plan.mt + para.height() + plan.mb,
        };
        let text_top = top + plan.mt;
        let text_x = f.col_left + plan.x;

        let mut marks = Vec::new();
        let mut check = None;
        let mut buttons: Vec<Button> = Vec::new();
        match plan.list {
            ListMark::None => {}
            ListMark::Bullet(level) => {
                let glyph = match level {
                    0 => "•",
                    1 => "◦",
                    _ => "▪",
                };
                let p = self.shape_mark(f, glyph, BODY_SIZE, BODY_LH, Role::Muted);
                let x = f.col_left
                    + level as f32 * LIST_STEP
                    + (LIST_STEP - 4.0 - text_width(&p)) / 2.0;
                marks.push(Mark {
                    para: p,
                    x,
                    y: text_top,
                });
            }
            ListMark::Number(level, number) => {
                let p = self.shape_mark(f, &format!("{number}."), BODY_SIZE, BODY_LH, Role::Muted);
                let x = f.col_left + level as f32 * LIST_STEP + LIST_STEP - 4.0 - text_width(&p);
                marks.push(Mark {
                    para: p,
                    x,
                    y: text_top,
                });
            }
            ListMark::Check(level, done) => {
                let rect = checkbox_rect(
                    f.col_left,
                    level as f32 * LIST_STEP + 2.0,
                    text_top,
                    plan.lh,
                );
                check = Some((done, rect));
            }
        }
        if let LineKind::FenceOpen { lang } = &line.kind {
            if plan.collapsed.is_some() {
                if !lang.is_empty() {
                    let p = self.shape_mark(f, lang, LABEL_SIZE, LABEL_LH, Role::Muted);
                    marks.push(Mark {
                        para: p,
                        x: f.col_left + CODE_PAD_X,
                        y: top + 4.0,
                    });
                }
                let hover = self.widgets.hover;
                let copy = {
                    let mut ctx = self.block_ctx(f, hover);
                    rich_blocks::copy_button(&mut ctx, i, top)
                };
                buttons.push(copy);
            }
        }

        Row {
            index: i,
            top,
            height,
            text_top,
            text_x,
            col_left: f.col_left,
            col_w: f.col_w,
            para,
            disp,
            deco: plan.deco,
            check,
            marks,
            block,
            widget: None,
            embed: None,
            buttons,
            widget_part: false,
            icon: plan.label,
        }
    }

    /// The widget that draws line `i`, when the caret is away from it. Its
    /// lines and kind; `None` when the lines are shown as source.
    fn rendered_widget(&self, i: usize) -> Option<(Range<usize>, WidgetKind)> {
        let (lines, kind) = rich_blocks::widget_span(
            &self.model,
            &self.blocks,
            &self.block_of,
            &self.content,
            self.widgets.charts,
            i,
        )?;
        let first = self.model.lines[lines.start].range.start;
        let last = self.model.lines[lines.end - 1].range.end;
        if !touches(&self.selection, &(first..last)) {
            return Some((lines, kind));
        }
        if kind == WidgetKind::Table
            && crate::rich_table::is_table_rendered(first..last, &self.selection)
        {
            return Some((lines, kind));
        }
        None
    }

    /// Row(s) for a rendered widget. Its first line carries the widget; the
    /// other lines of its block are empty rows that take no height.
    fn widget_row(
        &mut self,
        f: &mut Frame,
        i: usize,
        lines: Range<usize>,
        kind: WidgetKind,
        top: f32,
    ) -> Row {
        let line = self.model.lines[i].clone();
        let block = self.block_of[i];
        let para = self.shape_mark(f, "", BODY_SIZE, BODY_LH, Role::Body);
        if i != lines.start {
            return Row {
                index: i,
                top,
                height: 0.0,
                text_top: top,
                text_x: f.col_left,
                col_left: f.col_left,
                col_w: f.col_w,
                para,
                disp: DisplayLine {
                    text: String::new(),
                    to_source: vec![line.range.end],
                    styles: Vec::new(),
                },
                deco: Deco::None,
                check: None,
                marks: Vec::new(),
                block,
                widget: None,
                embed: None,
                buttons: Vec::new(),
                widget_part: true,
                icon: None,
            };
        }
        let hover = self.widgets.hover;
        let widget = {
            let mut ctx = self.block_ctx(f, hover);
            rich_blocks::build(&mut ctx, kind, lines, top)
        };
        Row {
            index: i,
            top,
            height: widget.rect.h,
            text_top: top,
            text_x: f.col_left,
            col_left: f.col_left,
            col_w: f.col_w,
            para,
            // Entering a widget from above or below puts the caret on this
            // line's start: that makes the block raw, so it is editable.
            disp: DisplayLine {
                text: String::new(),
                to_source: vec![line.content_start],
                styles: Vec::new(),
            },
            deco: Deco::None,
            check: None,
            marks: Vec::new(),
            block,
            widget: Some(widget),
            embed: None,
            buttons: Vec::new(),
            widget_part: false,
            icon: None,
        }
    }

    /// Context for the block widgets over the current frame.
    fn block_ctx<'a>(&'a mut self, f: &'a Frame, hover: Option<Hover>) -> rich_blocks::Ctx<'a> {
        let copied = self
            .widgets
            .copied
            .filter(|(_, at)| at.elapsed() < Duration::from_millis(1000))
            .map(|(line, _)| line);
        rich_blocks::Ctx {
            content: &self.content,
            lines: &self.model.lines,
            blocks: &self.blocks,
            block_of: &self.block_of,
            ts: &f.ts,
            shapes: &mut self.layout.shapes,
            body: f.body.clone(),
            mono: f.mono.clone(),
            theme: &f.theme,
            col_left: f.col_left,
            col_w: f.col_w,
            base: self.widgets.file_path.as_deref(),
            dims: &mut self.widgets.dims,
            hover,
            copied,
            selection: self.selection.clone(),
        }
    }

    fn shape_mark(
        &mut self,
        f: &Frame,
        text: &str,
        size: f32,
        lh: f32,
        role: Role,
    ) -> Rc<WrappedText> {
        let colour = role_colour(role, &f.theme);
        let run = TextRun {
            len: text.len(),
            font: f.body.clone(),
            color: colour.into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        self.layout
            .shapes
            .get_or_shape(&f.ts, text, px(size), &[run], None, px(lh))
    }

    fn frame(&self, window: &Window, width: f32) -> Frame {
        let col_w = (width - 2.0 * SIDE_PAD).clamp(120.0, COLUMN_MAX);
        Frame {
            ts: window.text_system().clone(),
            body: window.text_style().font(),
            mono: gpui::font(mono_font()),
            theme: Theme::for_name(&self.theme_name, window.appearance()),
            col_left: ((width - col_w) / 2.0).max(0.0),
            col_w,
            code: HashMap::new(),
        }
    }

    /// Height of line `line` in px, from the cache or by building its row.
    fn line_height_px(&mut self, f: &mut Frame, line: usize) -> f32 {
        if let Some(Some(height)) = self.layout.heights.get(line) {
            return *height;
        }
        let height = self.build_row(f, line, 0.0).height;
        if let Some(slot) = self.layout.heights.get_mut(line) {
            *slot = Some(height);
        }
        height
    }

    fn scroll_down(&mut self, f: &mut Frame, px: f32) {
        let count = self.model.lines.len();
        let mut scroll = self.scroll;
        scroll.down(px, count, |line| self.line_height_px(f, line));
        self.scroll = scroll;
    }

    fn scroll_up(&mut self, f: &mut Frame, px: f32) {
        let mut scroll = self.scroll;
        scroll.up(px, |line| self.line_height_px(f, line));
        self.scroll = scroll;
    }

    /// Scrolls the least that brings the caret's row into view.
    fn follow_caret_now(&mut self, f: &mut Frame) {
        let cursor = self.cursor();
        let line = self.line_of(cursor);
        let row = self.build_row(f, line, 0.0);
        let idx = rich_model::source_to_display(&row.disp, cursor);
        let (r, _) = row.para.caret_row_x(idx, self.at_row_end);
        let lh = f32::from(row.para.line_height());
        let caret_top = row.text_top + r as f32 * lh;
        let caret_bottom = caret_top + lh;
        if line < self.scroll.line || (line == self.scroll.line && caret_top < self.scroll.offset) {
            self.scroll = Scroll {
                line,
                offset: caret_top,
            };
            return;
        }
        let mut below = TOP_PAD + caret_bottom - self.scroll.offset;
        for above in self.scroll.line..line {
            below += self.line_height_px(f, above);
        }
        let limit = self.view_height - BOTTOM_PAD;
        if below > limit {
            self.scroll_down(f, below - limit);
        }
    }

    /// The visible rows from the scroll position down to the bottom of the view.
    /// Also returns the bottom edge of the last row, and whether the document
    /// ended before the view was full.
    fn walk(&mut self, f: &mut Frame) -> (Vec<Row>, f32, bool) {
        let mut rows = Vec::new();
        let mut i = self.scroll.line;
        let mut top = TOP_PAD - self.scroll.offset;
        loop {
            let row = self.build_row(f, i, top);
            if let Some(slot) = self.layout.heights.get_mut(i) {
                *slot = Some(row.height);
            }
            top += row.height;
            rows.push(row);
            if top >= self.view_height {
                return (rows, top, false);
            }
            if i + 1 >= self.model.lines.len() {
                return (rows, top, true);
            }
            i += 1;
        }
    }

    /// Lays out one frame: applies pending scroll requests, then walks the
    /// visible lines. Runs in prepaint, where the window can shape text.
    fn layout(&mut self, window: &mut Window, width: f32, height: f32) -> (Vec<Row>, Vec<Panel>) {
        self.view_height = height;
        self.view_width = width;
        let mut f = self.frame(window, width);
        let width_key = format!("{:.2}", f.col_w);
        let key = hash_of(&[width_key.as_str(), f.body.family.as_ref()]);
        if self.layout.key != key {
            self.layout.key = key;
            self.layout.heights.clear();
        }
        let n = self.model.lines.len();
        self.layout.heights.resize(n, None);

        let last = n - 1;
        if self.scroll.line > last {
            self.scroll = Scroll {
                line: last,
                offset: 0.0,
            };
        }
        if std::mem::take(&mut self.follow_caret) {
            self.follow_caret_now(&mut f);
        }
        let delta = std::mem::take(&mut self.scroll_px);
        if delta > 0.0 {
            self.scroll_down(&mut f, delta);
        } else if delta < 0.0 {
            self.scroll_up(&mut f, -delta);
        }
        let top_height = self.line_height_px(&mut f, self.scroll.line);
        self.scroll.offset = self.scroll.offset.clamp(0.0, top_height);

        let (rows, bottom, ended) = self.walk(&mut f);
        // Near the end the view can show empty space below the last line; pull
        // the top back so the document ends on the bottom padding.
        let slack = self.view_height - BOTTOM_PAD - bottom;
        let rows = if ended && slack > 0.0 && (self.scroll.line > 0 || self.scroll.offset > 0.0) {
            self.scroll_up(&mut f, slack);
            self.walk(&mut f).0
        } else {
            rows
        };
        let panels = self.panels(&rows, &f);
        (rows, panels)
    }

    /// One panel per run of consecutive rows in a code block or callout.
    fn panels(&self, rows: &[Row], f: &Frame) -> Vec<Panel> {
        let mut out = Vec::new();
        let mut open: Option<(usize, PanelKind, f32, f32)> = None;
        let flush = |open: &mut Option<(usize, PanelKind, f32, f32)>, out: &mut Vec<Panel>| {
            if let Some((_, kind, top, bottom)) = open.take() {
                out.push(Panel {
                    kind,
                    rect: Rect {
                        x: f.col_left,
                        y: top,
                        w: f.col_w,
                        h: bottom - top,
                    },
                });
            }
        };
        for row in rows {
            if row.widget_part || row.widget.is_some() {
                // A widget draws its own frame; it neither joins nor ends a panel.
                flush(&mut open, &mut out);
                continue;
            }
            let kind = match self.blocks.get(row.block).map(|b| &b.kind) {
                Some(BlockKind::CodeBlock { .. }) => Some(PanelKind::Code),
                Some(BlockKind::Callout(k)) => Some(PanelKind::Callout(*k)),
                Some(BlockKind::FrontMatter) => Some(PanelKind::FrontMatter),
                _ => None,
            };
            match kind {
                Some(kind) => {
                    let same = matches!(open, Some((block, ..)) if block == row.block);
                    if same {
                        if let Some((_, _, _, bottom)) = open.as_mut() {
                            *bottom = row.top + row.height;
                        }
                    } else {
                        flush(&mut open, &mut out);
                        open = Some((row.block, kind, row.top, row.top + row.height));
                    }
                }
                None => flush(&mut open, &mut out),
            }
        }
        flush(&mut open, &mut out);
        out
    }

    /// Selection, search, decoration and caret quads for the laid-out rows.
    fn quads(&self, rows: &[Row], panels: &[Panel], bounds: Bounds<Pixels>, t: &Theme) -> Quads {
        let mut q = Quads::default();
        let at = |x: f32, y: f32| point(bounds.left() + px(x), bounds.top() + px(y));
        let rect_bounds = |r: Rect| Bounds::new(at(r.x, r.y), size(px(r.w), px(r.h)));

        for p in panels {
            match p.kind {
                PanelKind::Code => q.panels.push(quad(
                    rect_bounds(p.rect),
                    px(6.0),
                    t.hover,
                    px(0.0),
                    transparent_black(),
                    BorderStyle::default(),
                )),
                PanelKind::FrontMatter => q.panels.push(quad(
                    rect_bounds(p.rect),
                    px(6.0),
                    t.surface,
                    px(1.0),
                    t.border,
                    BorderStyle::default(),
                )),
                PanelKind::Callout(kind) => {
                    let colour = callout_colour(kind, t);
                    q.panels.push(quad(
                        rect_bounds(p.rect),
                        px(6.0),
                        tint(colour, 0.10),
                        px(0.0),
                        transparent_black(),
                        BorderStyle::default(),
                    ));
                    q.panels.push(fill(
                        Bounds::new(at(p.rect.x, p.rect.y), size(px(BAR_W), px(p.rect.h))),
                        colour,
                    ));
                }
            }
        }

        let sel = self.selection.clone();
        let n = self.model.lines.len();
        for row in rows {
            let origin = at(row.text_x, row.text_top);
            let lr = self.model.lines[row.index].range.clone();
            if !sel.is_empty() {
                let s = sel.start.max(lr.start);
                let e = sel.end.min(lr.end);
                if s < e {
                    let ds = rich_model::source_to_display(&row.disp, s);
                    let de = rich_model::source_to_display(&row.disp, e);
                    if ds < de {
                        q.selection.extend(row.para.range_quads(
                            origin,
                            ds..de,
                            t.selection,
                            px(1.0),
                        ));
                    }
                }
                let breaks = sel.start <= lr.end && sel.end > lr.end && row.index + 1 < n;
                if breaks {
                    let at_end = row.para.position_for_index(row.para.len(), false);
                    q.selection.push(fill(
                        Bounds::new(
                            point(origin.x + at_end.x, origin.y + at_end.y),
                            size(px(NEWLINE_W), row.para.line_height()),
                        ),
                        t.selection,
                    ));
                }
            }

            let first = self
                .search_ranges
                .partition_point(|found| found.end <= lr.start);
            for found in &self.search_ranges[first..] {
                if found.start >= lr.end {
                    break;
                }
                let s = found.start.max(lr.start);
                let e = found.end.min(lr.end);
                if s >= e {
                    continue;
                }
                let colour = if self.search_active.as_ref() == Some(found) {
                    rgba_u32(0xf5a52480)
                } else {
                    rgba_u32(0xf5a52433)
                };
                let ds = rich_model::source_to_display(&row.disp, s);
                let de = rich_model::source_to_display(&row.disp, e);
                if ds < de {
                    q.search
                        .extend(row.para.range_quads(origin, ds..de, colour, px(1.0)));
                }
            }

            match row.deco {
                Deco::None => {}
                Deco::Rule => q.decor.push(fill(
                    Bounds::new(
                        at(row.col_left, row.top + RULE_H / 2.0 - 0.5),
                        size(px(row.col_w), px(1.0)),
                    ),
                    t.border,
                )),
                Deco::Quote(depth) => {
                    for d in 0..depth {
                        q.decor.push(fill(
                            Bounds::new(
                                at(row.col_left + d as f32 * QUOTE_STEP, row.top),
                                size(px(BAR_W), px(row.height)),
                            ),
                            t.accent,
                        ));
                    }
                }
                Deco::HeadingRule => q.decor.push(fill(
                    Bounds::new(
                        at(row.col_left, row.text_top + row.para.height() + 3.0),
                        size(px(row.col_w), px(1.0)),
                    ),
                    t.border,
                )),
            }

            if let Some((checked, rect)) = row.check {
                let boxed = rect_bounds(rect);
                if checked {
                    q.decor.push(quad(
                        boxed,
                        px(3.0),
                        t.accent,
                        px(1.0),
                        t.accent,
                        BorderStyle::default(),
                    ));
                    for (x, y) in tick_points(rect) {
                        q.decor.push(fill(
                            Bounds::new(at(x - 0.9, y - 0.9), size(px(1.8), px(1.8))),
                            t.bg,
                        ));
                    }
                } else {
                    q.decor.push(quad(
                        boxed,
                        px(3.0),
                        transparent_black(),
                        px(1.0),
                        t.muted,
                        BorderStyle::default(),
                    ));
                }
            }
        }

        let cursor = self.cursor();
        for row in rows {
            if let Some(w) = &row.widget {
                if let rich_blocks::Body::Table(tb) = &w.body {
                    crate::rich_table::table_quads(
                        bounds,
                        tb,
                        &self.selection,
                        cursor,
                        self.at_row_end,
                        t,
                        &mut q.caret,
                        &mut q.selection,
                    );
                }
            }
        }

        let caret_line = self.line_of(cursor);
        if q.caret.is_none() {
            if let Some(row) = rows
                .iter()
                .find(|r| r.index == caret_line && r.widget.is_none() && !r.widget_part)
            {
                let idx = rich_model::source_to_display(&row.disp, cursor);
                q.caret = Some(row.para.caret_quad(
                    at(row.text_x, row.text_top),
                    idx,
                    self.at_row_end,
                    t.cursor,
                    px(CARET_WIDTH),
                ));
            }
        }
        q
    }

    // ---- caret movement -----------------------------------------------------

    /// Where the caret goes one visual row up or down, keeping the column.
    fn vertical_target(&mut self, window: &Window, delta: isize) -> Option<(usize, bool)> {
        let cursor = self.cursor();
        let line = self.line_of(cursor);
        let mut f = self.frame(window, self.view_width);
        let row = self.build_row(&mut f, line, 0.0);
        let idx = rich_model::source_to_display(&row.disp, cursor);
        let (r, x_local) = row.para.caret_row_x(idx, self.at_row_end);
        let x = *self.desired_x.get_or_insert(row.text_x + x_local);

        if delta < 0 {
            if r > 0 {
                let (i2, end) = row.para.index_at_row(r - 1, x - row.text_x);
                return Some((self.source_of(&row, i2), end));
            }
            if line == 0 {
                return None;
            }
            let prev = self.build_row(&mut f, line - 1, 0.0);
            if let Some(w) = &prev.widget {
                if let rich_blocks::Body::Table(tb) = &w.body {
                    let last_r = tb.cells.len().saturating_sub(1);
                    let cols = tb.col_edges.len().saturating_sub(1);
                    if cols > 0 && !tb.cells[last_r].is_empty() {
                        let clamped_x = x.clamp(tb.col_edges[0], tb.col_edges[cols] - 0.1);
                        let c = rich_blocks::cell_index(&tb.col_edges, clamped_x)
                            .unwrap_or(0)
                            .min(tb.cells[last_r].len() - 1);
                        let cell = &tb.cells[last_r][c];
                        let (i2, end) = cell.para.hit_test(point(px(x - cell.x), px(0.0)));
                        let target = cell.src.start + rich_model::display_to_source(&cell.disp, i2);
                        return Some((target, end));
                    }
                }
            }
            let last = prev.para.row_count() - 1;
            let (i2, end) = prev.para.index_at_row(last, x - prev.text_x);
            Some((self.source_of(&prev, i2), end))
        } else {
            if r + 1 < row.para.row_count() {
                let (i2, end) = row.para.index_at_row(r + 1, x - row.text_x);
                return Some((self.source_of(&row, i2), end));
            }
            if line + 1 >= self.model.lines.len() {
                return None;
            }
            let next = self.build_row(&mut f, line + 1, 0.0);
            if let Some(w) = &next.widget {
                if let rich_blocks::Body::Table(tb) = &w.body {
                    let cols = tb.col_edges.len().saturating_sub(1);
                    if cols > 0 && !tb.cells[0].is_empty() {
                        let clamped_x = x.clamp(tb.col_edges[0], tb.col_edges[cols] - 0.1);
                        let c = rich_blocks::cell_index(&tb.col_edges, clamped_x)
                            .unwrap_or(0)
                            .min(tb.cells[0].len() - 1);
                        let cell = &tb.cells[0][c];
                        let (i2, end) = cell.para.hit_test(point(px(x - cell.x), px(0.0)));
                        let target = cell.src.start + rich_model::display_to_source(&cell.disp, i2);
                        return Some((target, end));
                    }
                }
            }
            let (i2, end) = next.para.index_at_row(0, x - next.text_x);
            Some((self.source_of(&next, i2), end))
        }
    }

    fn source_of(&self, row: &Row, display_index: usize) -> usize {
        rich_model::display_to_source(&row.disp, display_index)
    }

    fn vertical(&mut self, window: &Window, delta: isize, extend: bool, cx: &mut Context<Self>) {
        if !extend {
            if let Some((lines, table, _)) = self.table_info_at(self.cursor()) {
                if let Some((r, c)) = crate::rich_table::cell_at_offset(
                    &table,
                    &self.model.lines[lines.clone()],
                    self.cursor(),
                ) {
                    let mut f = self.frame(window, self.view_width);
                    let widget_row = self.build_row(&mut f, lines.start, 0.0);
                    let tb_opt = match &widget_row.widget {
                        Some(w) => match &w.body {
                            rich_blocks::Body::Table(tb) => Some(tb),
                            _ => None,
                        },
                        None => None,
                    };
                    let x = self.desired_x;
                    let action = if delta < 0 {
                        crate::rich_table::nav_up(
                            &self.content,
                            &table,
                            tb_opt,
                            lines.clone(),
                            r,
                            c,
                            self.cursor(),
                            x,
                        )
                    } else {
                        crate::rich_table::nav_down(
                            &self.content,
                            &table,
                            tb_opt,
                            lines.clone(),
                            self.model.lines.len(),
                            r,
                            c,
                            self.cursor(),
                            x,
                        )
                    };
                    match action {
                        crate::rich_table::NavAction::Move(target) => {
                            self.place(target, false, false, true, cx);
                            return;
                        }
                        crate::rich_table::NavAction::LeaveBefore(target_line) => {
                            let target = self.model.lines[target_line].range.end;
                            self.place(target, false, false, false, cx);
                            return;
                        }
                        crate::rich_table::NavAction::LeaveAfter(target_line) => {
                            let target = self.model.lines[target_line].range.start;
                            self.place(target, false, false, false, cx);
                            return;
                        }
                        crate::rich_table::NavAction::InsertLineBefore(at) => {
                            self.commit(
                                Edit {
                                    range: at..at,
                                    insert: "\n".to_string(),
                                    selection: at..at,
                                },
                                false,
                                cx,
                            );
                            return;
                        }
                        crate::rich_table::NavAction::InsertLineAfter(at) => {
                            self.commit(
                                Edit {
                                    range: at..at,
                                    insert: "\n".to_string(),
                                    selection: at + 1..at + 1,
                                },
                                false,
                                cx,
                            );
                            return;
                        }
                    }
                }
            }
        }
        let target = self.vertical_target(window, delta);
        if delta > 0 && !extend {
            // Down onto a closing fence or rule that ends the document: give
            // it a line after, and move there.
            if let Some(edit) = rich_ops::ensure_trailing_line(&self.content) {
                let last_start = self.content.rfind('\n').map_or(0, |i| i + 1);
                let exits = match target {
                    None => true,
                    Some((o, _)) => {
                        let p = rich_ops::line_prefix(&self.content, o);
                        o >= last_start && ((p.fence && !p.in_code) || p.rule)
                    }
                };
                if exits {
                    self.apply_edit(edit, cx);
                    let len = self.content.len();
                    self.place(len, false, false, false, cx);
                    return;
                }
            }
        }
        if let Some((offset, at_end)) = target {
            // Inside a table the caret lands in a cell's content, never on a `|`.
            let cursor = self.cursor();
            let snapped = rich_ops::table_vertical_target(&self.content, offset, cursor, delta);
            let at_end = at_end && snapped == offset;
            let offset = clamp_out_of_marker(&self.model, snapped, delta > 0);
            self.place(offset, at_end, extend, true, cx);
        }
    }

    /// Start or end of the visual row the caret is on.
    fn visual_edge(&mut self, window: &Window, end: bool, extend: bool, cx: &mut Context<Self>) {
        let cursor = self.cursor();
        let line = self.line_of(cursor);
        let mut f = self.frame(window, self.view_width);
        let row = self.build_row(&mut f, line, 0.0);
        let idx = rich_model::source_to_display(&row.disp, cursor);
        let (r, _) = row.para.caret_row_x(idx, self.at_row_end);
        let (target_disp, at_row_end) = if !end {
            (row.para.rows().start(r), false)
        } else if r + 1 < row.para.row_count() {
            (row.para.rows().end(r), true)
        } else {
            (row.para.len(), false)
        };
        let target = self.source_of(&row, target_disp);
        self.place(target, at_row_end, extend, false, cx);
    }

    // ---- actions ----------------------------------------------------------

    fn act_backspace(&mut self, _: &editor::Backspace, _: &mut Window, cx: &mut Context<Self>) {
        if self.selection.is_empty() {
            if let Some((lines, table, _)) = self.table_info_at(self.cursor()) {
                if let Some((r, c)) = crate::rich_table::cell_at_offset(
                    &table,
                    &self.model.lines[lines],
                    self.cursor(),
                ) {
                    if let Some(edit) = crate::rich_table::cell_backspace(
                        &self.content,
                        &table,
                        r,
                        c,
                        self.cursor(),
                    ) {
                        self.apply_edit(edit, cx);
                    }
                    return;
                }
            }
            if let Some(edit) =
                rich_ops::backspace_at_line_start(&self.content, self.selection.clone())
            {
                self.apply_edit(edit, cx);
                return;
            }
            let start = rich_ops::backspace_start(&self.content, self.cursor());
            self.selection = start..self.cursor();
        }
        self.replace_range(self.selection.clone(), "", cx);
    }

    fn act_delete(&mut self, _: &editor::Delete, _: &mut Window, cx: &mut Context<Self>) {
        if self.selection.is_empty() {
            if let Some((lines, table, _)) = self.table_info_at(self.cursor()) {
                if let Some((r, c)) = crate::rich_table::cell_at_offset(
                    &table,
                    &self.model.lines[lines],
                    self.cursor(),
                ) {
                    if let Some(edit) =
                        crate::rich_table::cell_delete(&self.content, &table, r, c, self.cursor())
                    {
                        self.apply_edit(edit, cx);
                    }
                    return;
                }
            }
            if let Some(edit) =
                rich_ops::join_paragraphs_forward(&self.content, self.selection.clone())
            {
                self.apply_edit(edit, cx);
                return;
            }
            let cursor = self.cursor();
            let next = self.next_boundary(cursor);
            self.selection = cursor..next;
        }
        self.replace_range(self.selection.clone(), "", cx);
    }

    fn act_enter(&mut self, _: &editor::Enter, _: &mut Window, cx: &mut Context<Self>) {
        if self.overlay_key(OverlayKey::Enter, cx) {
            return;
        }
        match rich_ops::enter(&self.content, self.selection.clone()) {
            Some(edit) => self.apply_edit(edit, cx),
            None => self.splice(self.selection.clone(), "\n", false, cx),
        }
    }

    fn act_cancel(&mut self, _: &editor::Cancel, _: &mut Window, cx: &mut Context<Self>) {
        if self.overlay_key(OverlayKey::Escape, cx) {
            return;
        }
        if let Some((lines, _, _)) = self.table_info_at(self.cursor()) {
            let action =
                crate::rich_table::nav_escape(&self.content, lines, self.model.lines.len());
            match action {
                crate::rich_table::NavAction::LeaveAfter(target_line) => {
                    let target = self.model.lines[target_line].range.start;
                    self.place(target, false, false, false, cx);
                    return;
                }
                crate::rich_table::NavAction::InsertLineAfter(at) => {
                    self.commit(
                        Edit {
                            range: at..at,
                            insert: "\n".to_string(),
                            selection: at + 1..at + 1,
                        },
                        false,
                        cx,
                    );
                    return;
                }
                _ => {}
            }
        }
        if let Some(on_cancel) = self.on_cancel.as_ref() {
            on_cancel(cx);
        }
    }

    fn act_indent(&mut self, _: &Indent, _: &mut Window, cx: &mut Context<Self>) {
        self.indent(false, cx);
    }

    fn act_outdent(&mut self, _: &Outdent, _: &mut Window, cx: &mut Context<Self>) {
        self.indent(true, cx);
    }

    fn indent(&mut self, shift: bool, cx: &mut Context<Self>) {
        if !shift && self.overlay_key(OverlayKey::Tab, cx) {
            return;
        }
        if self.in_table() {
            // Tab and Shift-Tab move between cells; they never insert a tab.
            if let Some(edit) =
                rich_ops::table_next_cell(&self.content, self.selection.clone(), shift)
            {
                self.apply_edit(edit, cx);
            }
            return;
        }
        // Tab in a paragraph is consumed without inserting a tab character:
        // the rich document is markdown, where a literal tab is not a
        // paragraph indent (Milkdown ignores it too).
        if let Some(edit) = rich_ops::tab(&self.content, self.selection.clone(), shift) {
            self.apply_edit(edit, cx)
        }
    }

    fn act_left(&mut self, _: &editor::Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selection.is_empty() {
            if let Some((lines, table, _)) = self.table_info_at(self.cursor()) {
                if let Some((r, c)) = crate::rich_table::cell_at_offset(
                    &table,
                    &self.model.lines[lines.clone()],
                    self.cursor(),
                ) {
                    let action = crate::rich_table::nav_left(
                        &self.content,
                        &table,
                        lines,
                        r,
                        c,
                        self.cursor(),
                    );
                    match action {
                        crate::rich_table::NavAction::Move(target) => {
                            self.place(target, false, false, false, cx);
                            return;
                        }
                        crate::rich_table::NavAction::LeaveBefore(target_line) => {
                            let target = self.model.lines[target_line].range.end;
                            self.place(target, false, false, false, cx);
                            return;
                        }
                        crate::rich_table::NavAction::InsertLineBefore(at) => {
                            self.commit(
                                Edit {
                                    range: at..at,
                                    insert: "\n".to_string(),
                                    selection: at..at,
                                },
                                false,
                                cx,
                            );
                            return;
                        }
                        _ => {}
                    }
                }
            }
            let prev = self.prev_boundary(self.cursor());
            let target = clamp_out_of_marker(&self.model, prev, false);
            self.place(target, false, false, false, cx);
        } else {
            let start = self.selection.start;
            self.place(start, false, false, false, cx);
        }
    }

    fn act_right(&mut self, _: &editor::Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selection.is_empty() {
            if let Some((lines, table, _)) = self.table_info_at(self.cursor()) {
                if let Some((r, c)) = crate::rich_table::cell_at_offset(
                    &table,
                    &self.model.lines[lines.clone()],
                    self.cursor(),
                ) {
                    let action = crate::rich_table::nav_right(
                        &self.content,
                        &table,
                        lines,
                        self.model.lines.len(),
                        r,
                        c,
                        self.cursor(),
                    );
                    match action {
                        crate::rich_table::NavAction::Move(target) => {
                            self.place(target, false, false, false, cx);
                            return;
                        }
                        crate::rich_table::NavAction::LeaveAfter(target_line) => {
                            let target = self.model.lines[target_line].range.start;
                            self.place(target, false, false, false, cx);
                            return;
                        }
                        crate::rich_table::NavAction::InsertLineAfter(at) => {
                            self.commit(
                                Edit {
                                    range: at..at,
                                    insert: "\n".to_string(),
                                    selection: at + 1..at + 1,
                                },
                                false,
                                cx,
                            );
                            return;
                        }
                        _ => {}
                    }
                }
            }
            let next = self.next_boundary(self.cursor());
            let target = clamp_out_of_marker(&self.model, next, true);
            self.place(target, false, false, false, cx);
        } else {
            let end = self.selection.end;
            self.place(end, false, false, false, cx);
        }
    }

    fn act_select_left(&mut self, _: &editor::SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((lines, table, _)) = self.table_info_at(self.cursor()) {
            if let Some((r, c)) = crate::rich_table::cell_at_offset(
                &table,
                &self.model.lines[lines.clone()],
                self.cursor(),
            ) {
                let cell_src = &table.rows[r][c];
                let cursor = self.cursor();
                if cursor > cell_src.start {
                    let prev = self.prev_boundary(cursor).max(cell_src.start);
                    self.place(prev, false, true, false, cx);
                    return;
                } else if c > 0 {
                    self.place(table.rows[r][c - 1].end, false, true, false, cx);
                    return;
                } else if r > 0 {
                    let last_c = table.cols().saturating_sub(1);
                    self.place(table.rows[r - 1][last_c].end, false, true, false, cx);
                    return;
                }
            }
        }
        let prev = self.prev_boundary(self.cursor());
        let target = clamp_out_of_marker(&self.model, prev, false);
        self.place(target, false, true, false, cx);
    }

    fn act_select_right(
        &mut self,
        _: &editor::SelectRight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((lines, table, _)) = self.table_info_at(self.cursor()) {
            if let Some((r, c)) = crate::rich_table::cell_at_offset(
                &table,
                &self.model.lines[lines.clone()],
                self.cursor(),
            ) {
                let cell_src = &table.rows[r][c];
                let cursor = self.cursor();
                if cursor < cell_src.end {
                    let next = self.next_boundary(cursor).min(cell_src.end);
                    self.place(next, false, true, false, cx);
                    return;
                } else if c + 1 < table.cols() {
                    self.place(table.rows[r][c + 1].start, false, true, false, cx);
                    return;
                } else if r + 1 < table.rows.len() {
                    self.place(table.rows[r + 1][0].start, false, true, false, cx);
                    return;
                }
            }
        }
        let next = self.next_boundary(self.cursor());
        let target = clamp_out_of_marker(&self.model, next, true);
        self.place(target, false, true, false, cx);
    }

    fn act_word_left(&mut self, _: &editor::WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let target = editor::prev_word(&self.content, self.cursor());
        let target = clamp_out_of_marker(&self.model, target, false);
        self.place(target, false, false, false, cx);
    }

    fn act_word_right(&mut self, _: &editor::WordRight, _: &mut Window, cx: &mut Context<Self>) {
        let target = editor::next_word(&self.content, self.cursor());
        let target = clamp_out_of_marker(&self.model, target, true);
        self.place(target, false, false, false, cx);
    }

    fn act_select_word_left(
        &mut self,
        _: &editor::SelectWordLeft,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = editor::prev_word(&self.content, self.cursor());
        let target = clamp_out_of_marker(&self.model, target, false);
        self.place(target, false, true, false, cx);
    }

    fn act_select_word_right(
        &mut self,
        _: &editor::SelectWordRight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = editor::next_word(&self.content, self.cursor());
        let target = clamp_out_of_marker(&self.model, target, true);
        self.place(target, false, true, false, cx);
    }

    fn act_up(&mut self, _: &editor::Up, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay_key(OverlayKey::Up, cx) {
            return;
        }
        self.vertical(window, -1, false, cx);
    }

    fn act_down(&mut self, _: &editor::Down, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay_key(OverlayKey::Down, cx) {
            return;
        }
        self.vertical(window, 1, false, cx);
    }

    fn act_select_up(&mut self, _: &editor::SelectUp, window: &mut Window, cx: &mut Context<Self>) {
        self.vertical(window, -1, true, cx);
    }

    fn act_select_down(
        &mut self,
        _: &editor::SelectDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.vertical(window, 1, true, cx);
    }

    fn act_home(&mut self, _: &editor::Home, window: &mut Window, cx: &mut Context<Self>) {
        self.visual_edge(window, false, false, cx);
    }

    fn act_end(&mut self, _: &editor::End, window: &mut Window, cx: &mut Context<Self>) {
        self.visual_edge(window, true, false, cx);
    }

    fn act_select_home(
        &mut self,
        _: &editor::SelectHome,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.visual_edge(window, false, true, cx);
    }

    fn act_select_end(
        &mut self,
        _: &editor::SelectEnd,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.visual_edge(window, true, true, cx);
    }

    fn act_doc_start(&mut self, _: &editor::DocStart, _: &mut Window, cx: &mut Context<Self>) {
        self.place(0, false, false, false, cx);
    }

    fn act_doc_end(&mut self, _: &editor::DocEnd, _: &mut Window, cx: &mut Context<Self>) {
        // A closing fence, rule or table at the end gets a line after it, so
        // the caret never lands on the block's last line.
        if let Some(edit) = rich_ops::ensure_trailing_line(&self.content) {
            self.apply_edit(edit, cx);
        }
        self.place(self.content.len(), false, false, false, cx);
    }

    fn act_select_doc_start(
        &mut self,
        _: &editor::SelectDocStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.place(0, false, true, false, cx);
    }

    fn act_select_doc_end(
        &mut self,
        _: &editor::SelectDocEnd,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.place(self.content.len(), false, true, false, cx);
    }

    fn act_select_all(&mut self, _: &editor::SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selection = 0..self.content.len();
        self.reversed = false;
        self.at_row_end = false;
        self.desired_x = None;
        cx.notify();
    }

    fn act_paste(&mut self, _: &editor::Paste, _: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let text = normalise_newlines(&text);
        let sel = self.selection.clone();
        if self.in_table() {
            let clean = crate::rich_table::sanitize_cell_text(&text);
            self.replace_range(sel, &clean, cx);
            return;
        }
        match rich_ops::smart_paste(&self.content, sel.clone(), &text) {
            Some(edit) => self.apply_edit(edit, cx),
            None => self.splice(sel, &text, false, cx),
        }
    }

    /// Copies the selected source markdown, not the rendered text.
    fn act_copy(&mut self, _: &editor::Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selection.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selection.clone()].to_string(),
            ));
        }
    }

    fn act_cut(&mut self, _: &editor::Cut, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selection.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selection.clone()].to_string(),
            ));
            self.splice(self.selection.clone(), "", false, cx);
        }
    }

    fn act_undo(&mut self, _: &editor::Undo, _: &mut Window, cx: &mut Context<Self>) {
        self.undo(cx);
    }

    fn act_redo(&mut self, _: &editor::Redo, _: &mut Window, cx: &mut Context<Self>) {
        self.redo(cx);
    }

    fn act_bold(&mut self, _: &ToggleBold, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_bold(cx);
    }

    fn act_italic(&mut self, _: &ToggleItalic, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_italic(cx);
    }

    fn act_strike(&mut self, _: &ToggleStrike, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_strike(cx);
    }

    fn act_code(&mut self, _: &ToggleCode, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_code(cx);
    }

    fn act_link(&mut self, _: &InsertLink, window: &mut Window, cx: &mut Context<Self>) {
        self.open_link_popover(window, cx);
    }

    fn act_paragraph(&mut self, _: &SetParagraph, _: &mut Window, cx: &mut Context<Self>) {
        self.set_heading(0, cx);
    }

    fn act_h1(&mut self, _: &Heading1, _: &mut Window, cx: &mut Context<Self>) {
        self.set_heading(1, cx);
    }

    fn act_h2(&mut self, _: &Heading2, _: &mut Window, cx: &mut Context<Self>) {
        self.set_heading(2, cx);
    }

    fn act_h3(&mut self, _: &Heading3, _: &mut Window, cx: &mut Context<Self>) {
        self.set_heading(3, cx);
    }

    fn act_h4(&mut self, _: &Heading4, _: &mut Window, cx: &mut Context<Self>) {
        self.set_heading(4, cx);
    }

    fn act_h5(&mut self, _: &Heading5, _: &mut Window, cx: &mut Context<Self>) {
        self.set_heading(5, cx);
    }

    fn act_h6(&mut self, _: &Heading6, _: &mut Window, cx: &mut Context<Self>) {
        self.set_heading(6, cx);
    }

    fn act_bullets(&mut self, _: &BulletList, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_list(ListType::Bullet, cx);
    }

    fn act_ordered(&mut self, _: &OrderedList, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_list(ListType::Ordered, cx);
    }

    fn act_tasks(&mut self, _: &TaskList, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_list(ListType::Task, cx);
    }

    fn act_quote(&mut self, _: &BlockQuote, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_quote(cx);
    }

    // ---- mouse and wheel --------------------------------------------------

    /// Element-local point of a window-space position.
    fn local(&self, position: Point<Pixels>) -> Option<(f32, f32)> {
        let b = self.last_bounds?;
        Some((
            f32::from(position.x - b.left()),
            f32::from(position.y - b.top()),
        ))
    }

    /// The caret position under a window-space point, and its row-end flag.
    fn hit(&self, position: Point<Pixels>) -> (usize, bool) {
        let Some((x, y)) = self.local(position) else {
            return (self.cursor(), false);
        };
        let spans: Vec<(f32, f32)> = self.last_rows.iter().map(|r| (r.top, r.height)).collect();
        let Some(k) = row_for_y(&spans, y) else {
            return (self.cursor(), false);
        };
        let row = &self.last_rows[k];
        if let Some(w) = &row.widget {
            return (w.caret_at(x, y), false);
        }
        let (idx, at_end) = row
            .para
            .hit_test(point(px(x - row.text_x), px(y - row.text_top)));
        (self.source_of(row, idx), at_end)
    }

    /// The control under a window-space point, if the pointer is on one.
    fn button_at(&self, position: Point<Pixels>) -> Option<Action> {
        let (x, y) = self.local(position)?;
        self.last_rows.iter().find_map(|row| {
            let widget_buttons: &[Button] = row
                .widget
                .as_ref()
                .map(|w| w.buttons.as_slice())
                .unwrap_or(&[]);
            row.buttons
                .iter()
                .chain(widget_buttons.iter())
                .find(|b| b.rect.contains(x, y))
                .map(|b| b.action)
        })
    }

    /// The widget and the table row/column under a window-space point.
    fn hover_at(&self, position: Point<Pixels>) -> Option<Hover> {
        let (x, y) = self.local(position)?;
        self.last_rows.iter().find_map(|row| {
            let w = row.widget.as_ref()?;
            if !w.rect.contains(x, y) {
                return None;
            }
            let (r, c) = w.cell_hover(x, y);
            Some(Hover {
                line: w.lines.start,
                row: r,
                col: c,
            })
        })
    }

    /// Window-space centre of the first chart's full-screen button (scripted tests).
    #[cfg(debug_assertions)]
    pub(crate) fn test_fullscreen_button(&self) -> Option<Point<Pixels>> {
        let bounds = self.last_bounds?;
        self.last_rows.iter().find_map(|row| {
            let b = row
                .widget
                .as_ref()?
                .buttons
                .iter()
                .find(|b| matches!(b.action, Action::Fullscreen(_)))?;
            Some(point(
                bounds.left() + px(b.rect.x + b.rect.w / 2.0),
                bounds.top() + px(b.rect.y + b.rect.h / 2.0),
            ))
        })
    }

    /// Runs a widget control: copies a code fence, or edits a table.
    fn run_action(&mut self, action: Action, cx: &mut Context<Self>) {
        match action {
            Action::CopyCode(line) => {
                let range = self.blocks[self.block_of[line]].lines.clone();
                let text = rich_blocks::fence_text(&self.content, &self.model.lines, range);
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                self.widgets.copied = Some((line, Instant::now()));
                // Repaint once the "Copied" label has run its second.
                cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(1000))
                        .await;
                    this.update(cx, |_, cx| cx.notify()).ok();
                })
                .detach();
                cx.notify();
            }
            Action::Fullscreen(line) => {
                let range = self.blocks[self.block_of[line]].lines.clone();
                let text = rich_blocks::fence_text(&self.content, &self.model.lines, range);
                if let (Some(open), Some((_, kind))) =
                    (&self.on_fullscreen, self.rendered_widget(line))
                {
                    open(kind, &text, cx);
                }
            }
            Action::Table(line, op) => {
                let range = self.blocks[self.block_of[line]].lines.clone();
                let table = rich_blocks::parse_table(&self.content, &self.model.lines[range]);
                let edit = rich_blocks::table_op_edit(&self.content, &table, op);
                self.run(edit, cx);
            }
        }
    }

    /// Whether the caret is on a line of a GFM table (drawn raw).
    fn in_table(&self) -> bool {
        matches!(
            self.model.lines[self.line_of(self.cursor())].kind,
            LineKind::TableHeader | LineKind::TableDelimiter | LineKind::TableRow
        )
    }

    /// Information about a rendered table containing `offset`: line range, table model, and byte range.
    fn table_info_at(
        &self,
        offset: usize,
    ) -> Option<(Range<usize>, rich_blocks::SrcTable, Range<usize>)> {
        let line = self.line_of(offset);
        let block_idx = *self.block_of.get(line)?;
        let block = self.blocks.get(block_idx)?;
        if block.kind != BlockKind::Table {
            return None;
        }
        let lines_range = block.lines.clone();
        let (lines, kind) = self.rendered_widget(lines_range.start)?;
        if kind != WidgetKind::Table {
            return None;
        }
        let first = self.model.lines[lines.start].range.start;
        let last = self.model.lines[lines.end - 1].range.end;
        let table = rich_blocks::parse_table(&self.content, &self.model.lines[lines.clone()]);
        Some((lines, table, first..last))
    }

    /// Leaving a table edited as source: when the caret moves to `target`
    /// outside it, the table is re-aligned first. Returns `target` shifted
    /// past the change.
    fn leave_table(&mut self, target: Range<usize>, cx: &mut Context<Self>) -> Range<usize> {
        if !self.in_table() {
            return target;
        }
        let Some(table) = rich_ops::table_range(&self.content, self.cursor()) else {
            return target;
        };
        if touches(&target, &table) {
            return target;
        }
        let Some(edit) = rich_ops::table_reformat(&self.content, table) else {
            return target;
        };
        if edit.insert == self.content[edit.range.clone()] {
            return target;
        }
        let delta = edit.insert.len() as isize - edit.range.len() as isize;
        let end = edit.range.end;
        let shift = |o: usize| {
            if o >= end {
                (o as isize + delta) as usize
            } else {
                o
            }
        };
        let moved = shift(target.start)..shift(target.end);
        self.commit(edit, false, cx);
        moved
    }

    /// The task checkbox under a window-space point, as its line's offset.
    fn checkbox_at(&self, position: Point<Pixels>) -> Option<usize> {
        let (x, y) = self.local(position)?;
        self.last_rows.iter().find_map(|row| {
            let (_, rect) = row.check?;
            rect.contains(x, y)
                .then(|| self.model.lines[row.index].range.start)
        })
    }

    /// The link destination under a source offset, if it sits in one.
    fn link_at(&self, offset: usize) -> Option<String> {
        let line = &self.model.lines[self.line_of(offset)];
        line.inlines.iter().find_map(|inl| {
            if !inl.range.contains(&offset) {
                return None;
            }
            match &inl.kind {
                InlineKind::Link { url } => Some(self.content[url.clone()].to_string()),
                InlineKind::AutoLink => {
                    Some(self.content[inl.open.end..inl.close.start].to_string())
                }
                _ => None,
            }
        })
    }

    pub(crate) fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus);
        let modified = event.modifiers.shift || event.modifiers.alt || event.modifiers.secondary();
        if event.click_count == 1 && !modified {
            if let Some(line_start) = self.checkbox_at(event.position) {
                self.run(rich_ops::toggle_task(&self.content, line_start), cx);
                return;
            }
            if let Some(action) = self.button_at(event.position) {
                self.run_action(action, cx);
                return;
            }
        }
        let (offset, at_row_end) = self.hit(event.position);
        if event.click_count == 1 && event.modifiers.secondary() {
            if let Some(url) = self.link_at(offset) {
                cx.open_url(&url);
                return;
            }
        }
        self.selecting = true;
        if event.click_count >= 3 {
            if let Some((lines, table, _)) = self.table_info_at(offset) {
                if let Some((r, c)) =
                    crate::rich_table::cell_at_offset(&table, &self.model.lines[lines], offset)
                {
                    let range = table.rows[r][c].clone();
                    self.set_selection(range, cx);
                    return;
                }
            }
            let range = self.model.lines[self.line_of(offset)].range.clone();
            self.set_selection(range, cx);
        } else if event.click_count == 2 {
            if let Some((lines, table, _)) = self.table_info_at(offset) {
                if let Some((r, c)) =
                    crate::rich_table::cell_at_offset(&table, &self.model.lines[lines], offset)
                {
                    let cell_src = table.rows[r][c].clone();
                    let raw_range = editor::word_range(&self.content, offset);
                    let range = raw_range.start.clamp(cell_src.start, cell_src.end)
                        ..raw_range.end.clamp(cell_src.start, cell_src.end);
                    self.set_selection(range, cx);
                    return;
                }
            }
            let range = editor::word_range(&self.content, offset);
            self.set_selection(range, cx);
        } else {
            self.place(offset, at_row_end, event.modifiers.shift, false, cx);
        }
    }

    pub(crate) fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.selecting = false;
        cx.notify();
    }

    pub(crate) fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selecting {
            let (offset, at_row_end) = self.hit(event.position);
            self.place(offset, at_row_end, true, false, cx);
        }
        let hover = self.hover_at(event.position);
        if hover != self.widgets.hover {
            self.widgets.hover = hover;
            cx.notify();
        }
    }

    pub(crate) fn on_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delta = f32::from(event.delta.pixel_delta(px(BODY_LH)).y);
        self.scroll_px -= delta;
        self.overlay_on_scroll();
        cx.notify();
    }

    // ---- UTF-16 bridging --------------------------------------------------

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8 = 0;
        let mut utf16 = 0;
        for ch in self.content.chars() {
            if utf16 >= offset {
                break;
            }
            utf16 += ch.len_utf16();
            utf8 += ch.len_utf8();
        }
        utf8
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16 = 0;
        let mut utf8 = 0;
        for ch in self.content.chars() {
            if utf8 >= offset {
                break;
            }
            utf8 += ch.len_utf8();
            utf16 += ch.len_utf16();
        }
        utf16
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }
}

fn rgba_u32(v: u32) -> Rgba {
    gpui::rgba(v)
}

/// Run for a span of inline style.
fn inline_run(len: usize, st: InlineStyle, plan: &Plan, f: &Frame) -> TextRun {
    let t = &f.theme;
    let mut font = if plan.mono || st.code {
        f.mono.clone()
    } else {
        f.body.clone()
    };
    if st.bold || plan.bold {
        font.weight = if matches!(plan.role, Role::Heading(_)) {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::BOLD
        };
    }
    if st.italic {
        font.style = FontStyle::Italic;
    }
    let colour = if st.marker {
        t.muted
    } else if st.code {
        t.code
    } else if st.link {
        t.accent
    } else {
        role_colour(plan.role, t)
    };
    let background: Option<Hsla> = st.code.then(|| t.hover.into());
    let underline = st.link.then(|| UnderlineStyle {
        thickness: px(1.0),
        color: Some(t.accent.into()),
        wavy: false,
    });
    let strikethrough = st.strike.then(|| StrikethroughStyle {
        thickness: px(1.0),
        color: Some(t.muted.into()),
    });
    TextRun {
        len,
        font,
        color: colour.into(),
        background_color: background,
        underline,
        strikethrough,
    }
}

/// Run for a span of code, coloured by its syntax token.
fn code_run(len: usize, kind: TokenKind, plan: &Plan, f: &Frame) -> TextRun {
    let _ = plan;
    let (bold, italic) = highlight::emphasis(kind);
    let mut font = f.mono.clone();
    if bold {
        font.weight = FontWeight::SEMIBOLD;
    }
    if italic {
        font.style = FontStyle::Italic;
    }
    let colour: Hsla = if kind == TokenKind::Plain {
        f.theme.code.into()
    } else {
        highlight::color(kind, &f.theme)
    };
    TextRun {
        len,
        font,
        color: colour,
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}

impl Focusable for RichEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EntityInputHandler for RichEditor {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selection),
            reversed: self.reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked.as_ref().map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or(self.marked.clone())
            .unwrap_or(self.selection.clone());
        self.replace_range(range, new_text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or(self.marked.clone())
            .unwrap_or(self.selection.clone());
        self.replace_range(range.clone(), new_text, cx);
        if !new_text.is_empty() {
            self.marked = Some(range.start..range.start + new_text.len());
        }
        if let Some(new_range) = new_selected_range_utf16 {
            let start = self.offset_from_utf16(new_range.start);
            let end = self.offset_from_utf16(new_range.end);
            self.selection = start..end;
            self.reversed = false;
        }
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.range_from_utf16(&range_utf16);
        let line = self.line_of(range.start);
        let row = self.last_rows.iter().find(|r| r.index == line)?;
        let idx = rich_model::source_to_display(&row.disp, range.start);
        let at = row.para.position_for_index(idx, false);
        let left = bounds.left() + px(row.text_x) + at.x;
        let top = bounds.top() + px(row.text_top) + at.y;
        Some(Bounds::new(
            point(left, top),
            size(px(2.0), row.para.line_height()),
        ))
    }

    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.offset_to_utf16(self.hit(position).0))
    }
}

// ---------------------------------------------------------------- element

struct RichElement {
    editor: Entity<RichEditor>,
}

impl IntoElement for RichElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for RichElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = gpui::relative(1.0).into();
        style.size.height = gpui::relative(1.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let width = f32::from(bounds.size.width);
        let height = f32::from(bounds.size.height);
        let started = timing::start();
        let prepaint = self.editor.update(cx, |editor, ecx| {
            let layout_started = timing::start();
            let (mut rows, panels) = editor.layout(window, width, height);
            timing::record("layout", layout_started);
            let theme = Theme::for_name(&editor.theme_name, window.appearance());
            let quads = editor.quads(&rows, &panels, bounds, &theme);
            for row in rows.iter_mut() {
                if let Some(w) = &row.widget {
                    row.embed = rich_blocks::embed(w, bounds, &theme, window, ecx);
                }
            }
            Prepaint { rows, quads, theme }
        });
        timing::record("frame", started);
        prepaint
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.editor.read(cx).focus.clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.editor.clone()),
            cx,
        );

        let quads = std::mem::take(&mut prepaint.quads);
        for q in quads.panels {
            window.paint_quad(q);
        }
        for q in quads.search {
            window.paint_quad(q);
        }
        for q in quads.selection {
            window.paint_quad(q);
        }
        for q in quads.decor {
            window.paint_quad(q);
        }

        let rows_theme = prepaint.theme;
        let mut rows = std::mem::take(&mut prepaint.rows);
        let origin = |x: f32, y: f32| point(bounds.left() + px(x), bounds.top() + px(y));
        for row in rows.iter_mut() {
            if let Some(w) = &row.widget {
                rich_blocks::paint_widget(w, bounds, &rows_theme, window, cx);
            }
            for b in &row.buttons {
                rich_blocks::paint_button(b, bounds, &rows_theme, window, cx);
            }
            if let Some(kind) = row.icon {
                let at = origin(row.text_x, row.text_top + (LABEL_LH - LABEL_ICON) / 2.0);
                let colour: Hsla = callout_colour(kind, &rows_theme).into();
                let _ = window.paint_svg(
                    Bounds::new(at, size(px(LABEL_ICON), px(LABEL_ICON))),
                    gpui::SharedString::from(format!("icons/{}.svg", label_icon(kind))),
                    gpui::TransformationMatrix::default(),
                    colour,
                    cx,
                );
            }
            let text_x = row.text_x
                + if row.icon.is_some() {
                    LABEL_ICON_GAP
                } else {
                    0.0
                };
            row.para.paint(window, cx, origin(text_x, row.text_top));
            for mark in &row.marks {
                mark.para.paint(window, cx, origin(mark.x, mark.y));
            }
            if let Some(el) = row.embed.as_mut() {
                el.paint(window, cx);
            }
            // A chart's controls sit on top of the chart, so they paint after it.
            if let Some(w) = &row.widget {
                rich_blocks::paint_embed_buttons(w, bounds, &rows_theme, window, cx);
            }
        }

        if focus.is_focused(window) {
            if let Some(caret) = quads.caret {
                window.paint_quad(caret);
            }
        }

        let view_height = f32::from(bounds.size.height);
        self.editor.update(cx, |editor, _| {
            editor.last_bounds = Some(bounds);
            editor.last_rows = rows;
            editor.view_height = view_height;
        });
    }
}

impl Render for RichEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.overlay_sync(window, cx);
        let theme = Theme::for_name(&self.theme_name, window.appearance());
        let root = div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .cursor(CursorStyle::IBeam)
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            .bg(theme.bg)
            .text_size(px(BODY_SIZE))
            .line_height(px(BODY_LH))
            .text_color(theme.fg)
            .on_action(cx.listener(Self::act_backspace))
            .on_action(cx.listener(Self::act_delete))
            .on_action(cx.listener(Self::act_enter))
            .on_action(cx.listener(Self::act_cancel))
            .on_action(cx.listener(Self::act_indent))
            .on_action(cx.listener(Self::act_outdent))
            .on_action(cx.listener(Self::act_left))
            .on_action(cx.listener(Self::act_right))
            .on_action(cx.listener(Self::act_up))
            .on_action(cx.listener(Self::act_down))
            .on_action(cx.listener(Self::act_select_left))
            .on_action(cx.listener(Self::act_select_right))
            .on_action(cx.listener(Self::act_select_up))
            .on_action(cx.listener(Self::act_select_down))
            .on_action(cx.listener(Self::act_word_left))
            .on_action(cx.listener(Self::act_word_right))
            .on_action(cx.listener(Self::act_select_word_left))
            .on_action(cx.listener(Self::act_select_word_right))
            .on_action(cx.listener(Self::act_home))
            .on_action(cx.listener(Self::act_end))
            .on_action(cx.listener(Self::act_select_home))
            .on_action(cx.listener(Self::act_select_end))
            .on_action(cx.listener(Self::act_doc_start))
            .on_action(cx.listener(Self::act_doc_end))
            .on_action(cx.listener(Self::act_select_doc_start))
            .on_action(cx.listener(Self::act_select_doc_end))
            .on_action(cx.listener(Self::act_select_all))
            .on_action(cx.listener(Self::act_paste))
            .on_action(cx.listener(Self::act_copy))
            .on_action(cx.listener(Self::act_cut))
            .on_action(cx.listener(Self::act_undo))
            .on_action(cx.listener(Self::act_redo))
            .on_action(cx.listener(Self::act_bold))
            .on_action(cx.listener(Self::act_italic))
            .on_action(cx.listener(Self::act_strike))
            .on_action(cx.listener(Self::act_code))
            .on_action(cx.listener(Self::act_link))
            .on_action(cx.listener(Self::act_paragraph))
            .on_action(cx.listener(Self::act_h1))
            .on_action(cx.listener(Self::act_h2))
            .on_action(cx.listener(Self::act_h3))
            .on_action(cx.listener(Self::act_h4))
            .on_action(cx.listener(Self::act_h5))
            .on_action(cx.listener(Self::act_h6))
            .on_action(cx.listener(Self::act_bullets))
            .on_action(cx.listener(Self::act_ordered))
            .on_action(cx.listener(Self::act_tasks))
            .on_action(cx.listener(Self::act_quote))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_scroll_wheel(cx.listener(Self::on_scroll_wheel))
            .child(RichElement {
                editor: cx.entity(),
            });
        self.overlay_wrap(root, window, cx)
    }
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rich_model::parse;

    #[test]
    fn cumulative_tops_stack_heights() {
        assert_eq!(
            cumulative_tops(10.0, &[20.0, 5.0, 30.0]),
            vec![10.0, 30.0, 35.0]
        );
        assert!(cumulative_tops(0.0, &[]).is_empty());
    }

    #[test]
    fn row_for_y_finds_the_row_under_a_point() {
        let spans = [(0.0, 10.0), (10.0, 20.0), (30.0, 5.0)];
        assert_eq!(row_for_y(&spans, -5.0), Some(0));
        assert_eq!(row_for_y(&spans, 9.9), Some(0));
        assert_eq!(row_for_y(&spans, 10.0), Some(1));
        assert_eq!(row_for_y(&spans, 29.0), Some(1));
        assert_eq!(row_for_y(&spans, 31.0), Some(2));
        assert_eq!(row_for_y(&spans, 400.0), Some(2));
        assert_eq!(row_for_y(&[], 0.0), None);
    }

    #[test]
    fn caret_touch_includes_both_ends() {
        assert!(touches(&(5..5), &(0..5)));
        assert!(touches(&(5..5), &(5..9)));
        assert!(!touches(&(6..6), &(0..5)));
        assert!(touches(&(0..3), &(2..8)));
        let (hi, lo) = (8usize, 2usize);
        assert!(touches(&(hi..lo), &(0..3)), "reversed selections count too");
        assert!(!touches(&(10..12), &(0..5)));
    }

    #[test]
    fn arrows_never_land_inside_block_markers() {
        let src = "# Title\n- item\n\ntext";
        let m = parse(src);
        // "# " is bytes 0..2, so offset 1 is inside the marker.
        assert_eq!(clamp_out_of_marker(&m, 1, true), 2);
        assert_eq!(
            clamp_out_of_marker(&m, 1, false),
            2,
            "first line: no line above, go to content"
        );
        // Line 2 "- item": marker is 8..10.
        assert_eq!(clamp_out_of_marker(&m, 8, true), 10);
        // Backward from the content start goes to the end of the line above.
        assert_eq!(clamp_out_of_marker(&m, 9, false), m.lines[0].range.end);
        // Plain text is untouched.
        assert_eq!(clamp_out_of_marker(&m, 12, false), 12);
        assert_eq!(clamp_out_of_marker(&m, 2, false), 2);
    }

    #[test]
    fn checkbox_rect_hits_only_the_box() {
        let rect = checkbox_rect(100.0, 26.0, 40.0, 25.6);
        assert_eq!(rect.x, 126.0);
        assert!((rect.y - (40.0 + (25.6 - 14.0) / 2.0)).abs() < 1e-4);
        assert!(rect.contains(126.0, rect.y + 1.0));
        assert!(rect.contains(139.9, rect.y + 13.9));
        assert!(
            !rect.contains(140.0, rect.y + 1.0),
            "right edge is exclusive"
        );
        assert!(!rect.contains(125.9, rect.y + 1.0), "left of the box");
        assert!(!rect.contains(130.0, rect.y - 0.5), "above the box");
    }

    #[test]
    fn tick_mark_stays_inside_its_box() {
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 14.0,
            h: 14.0,
        };
        let pts = tick_points(rect);
        assert!(!pts.is_empty());
        assert!(pts.iter().all(|&(x, y)| rect.contains(x, y)));
    }

    #[test]
    fn gaps_separate_blocks_but_not_lines_of_one_block() {
        use LineKind::*;
        assert_eq!(gap_after(&Paragraph, Some(&Paragraph)), 0.0);
        assert_eq!(
            gap_after(&Paragraph, Some(&Blank)),
            0.0,
            "the blank line is the gap"
        );
        assert_eq!(gap_after(&Paragraph, None), 0.0);
        assert_eq!(gap_after(&Heading(1), Some(&Paragraph)), 8.0);
        assert_eq!(gap_after(&Paragraph, Some(&Heading(2))), 8.0);
        let item = ListItem {
            indent: 0,
            ordered: false,
            number: 0,
            task: None,
        };
        assert_eq!(gap_after(&item, Some(&item)), 3.0);
        assert_eq!(gap_after(&CodeLine, Some(&FenceClose)), 0.0);
    }

    #[test]
    fn cover_splits_spans_and_fills_gaps() {
        let spans = [(2..4, 'a'), (6..7, 'b')];
        let out = cover(10, &spans);
        assert_eq!(
            out,
            vec![
                (2, None),
                (2, Some('a')),
                (2, None),
                (1, Some('b')),
                (3, None)
            ]
        );
        assert_eq!(out.iter().map(|(n, _)| n).sum::<usize>(), 10);
        assert_eq!(cover(0, &spans), Vec::new());
        // Overlapping or out-of-range spans are clipped, never double counted.
        let odd = [(0..20, 'x'), (3..5, 'y')];
        let out = cover(6, &odd);
        assert_eq!(out.iter().map(|(n, _)| n).sum::<usize>(), 6);
    }

    #[test]
    fn pasted_newlines_become_lf() {
        assert_eq!(normalise_newlines("a\r\nb\rc\nd"), "a\nb\nc\nd");
        assert_eq!(normalise_newlines("plain"), "plain");
    }

    #[test]
    fn callout_and_heading_labels() {
        assert_eq!(label_text(CalloutKind::Warning), "Warning");
        assert_eq!(label_text(CalloutKind::Note), "Note");
        assert_eq!(label_icon(CalloutKind::Note), "information-circle");
        assert_eq!(label_icon(CalloutKind::Important), "alert-circle");
        assert_eq!(label_icon(CalloutKind::Caution), "alert01");
        assert_eq!(heading_size(1), 30.0);
        assert_eq!(heading_size(9), 14.0, "levels clamp to H6");
    }

    #[test]
    fn scroll_arithmetic_is_shared_with_the_plain_editor() {
        // The rich view reuses `editor::Scroll`; these cases pin the contract.
        let heights = [25.6, 25.6, 12.0, 8.0];
        let mut s = Scroll::default();
        s.down(30.0, heights.len(), |l| heights[l]);
        assert_eq!(s.line, 1);
        assert!((s.offset - 4.4).abs() < 1e-3);
    }
}
