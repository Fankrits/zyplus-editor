//! A multi-line plain-text editor drawn with GPUI.
//!
//! GPUI has no text editor widget, so this is the port's replacement for
//! CodeMirror: a text buffer, a selection, a caret, undo history and its own
//! element that lays out and paints the visible lines. It implements
//! `EntityInputHandler`, which is how GPUI routes real keyboard and IME input
//! into a custom element.
//!
//! Multi-line mode soft-wraps every source line at the viewport width (see
//! `wrap_layout`). The view is a (line, px offset into that line) pair, so
//! only the lines on screen are ever shaped; heights above or below the view
//! are measured when a scroll needs them and cached until the next edit.
//! Single-line mode never wraps and never scrolls.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    actions, div, fill, point, px, relative, rgba, size, App, Bounds, ClipboardItem, Context,
    CursorStyle, Element, ElementId, ElementInputHandler, Entity, EntityInputHandler, FocusHandle,
    Focusable, Font, FontStyle, FontWeight, GlobalElementId, Hsla, InspectorElementId, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point,
    ScrollWheelEvent, SharedString, Style, TextRun, UTF16Selection, Window, WindowTextSystem,
};

use crate::highlight::{self, LineCache, TokenKind};
use crate::history::History;
use crate::rich_ops;
use crate::theme::Theme;
use crate::wrap_layout::{ShapeCache, WrappedText};

pub const FONT_SIZE: f32 = 14.0;
pub const LINE_HEIGHT: f32 = 22.0;
pub const PAD_X: f32 = 20.0;
pub const PAD_Y: f32 = 14.0;
/// Monospace families to look for, best first. Menlo ships on every Mac. gpui
/// has no generic `monospace` family off macOS and falls back to a PROPORTIONAL
/// sans font for an unknown name, which breaks column-aligned code, so the
/// installed family is picked once at startup (`init_mono_font`).
#[cfg(target_os = "macos")]
const MONO_CANDIDATES: &[&str] = &["Menlo", "Monaco", "Courier New"];
#[cfg(target_os = "windows")]
const MONO_CANDIDATES: &[&str] = &["Consolas", "Cascadia Mono", "Courier New"];
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const MONO_CANDIDATES: &[&str] = &[
    "DejaVu Sans Mono",
    "Liberation Mono",
    "Noto Sans Mono",
    "Ubuntu Mono",
    "Cousine",
    "Hack",
    "Source Code Pro",
    "Fira Mono",
];

static MONO_FONT_CHOICE: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();

/// The monospace family used for code, tables and the plain editor.
pub fn mono_font() -> &'static str {
    MONO_FONT_CHOICE
        .get()
        .copied()
        .unwrap_or(MONO_CANDIDATES[0])
}

/// First candidate present in `available` (family names from the text
/// system); the platform's first choice when none is installed.
pub fn pick_mono_font(available: &[String]) -> &'static str {
    MONO_CANDIDATES
        .iter()
        .copied()
        .find(|candidate| available.iter().any(|name| name == candidate))
        .unwrap_or(MONO_CANDIDATES[0])
}

/// Picks the monospace family once; call at startup with
/// `cx.text_system().all_font_names()`.
pub fn init_mono_font(available: &[String]) {
    let _ = MONO_FONT_CHOICE.set(pick_mono_font(available));
}

/// Width of the caret bar.
const CARET_WIDTH: f32 = 1.5;
/// Width drawn for a selected line break, after the last glyph of its line.
const NEWLINE_MARK_WIDTH: f32 = FONT_SIZE * 0.6;
/// Rows of tokens fetched beyond the view, so a layout pass that runs a little
/// past the bottom still finds them in the prefetched window.
const TOKEN_SLACK_ROWS: usize = 2;

actions!(
    editor,
    [
        Backspace,
        Delete,
        Enter,
        Left,
        Right,
        Up,
        Down,
        Home,
        End,
        DocStart,
        DocEnd,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        SelectHome,
        SelectEnd,
        SelectDocStart,
        SelectDocEnd,
        WordLeft,
        WordRight,
        SelectWordLeft,
        SelectWordRight,
        SelectAll,
        Paste,
        Cut,
        Copy,
        Undo,
        Redo,
        Cancel,
    ]
);

/// Key bindings for the navigation keys both editors share: Cmd-Up/Down and
/// Cmd-Left/Right on macOS, or Ctrl-Home/End off macOS. Single-line inputs
/// keep only the horizontal line-edge actions.
pub fn bind_nav_keys(cx: &mut App) {
    cx.bind_keys(crate::keymap::editor_nav_bindings(
        crate::platform::Os::current(),
    ));
}

type Tokens = Vec<Vec<(Range<usize>, TokenKind)>>;

pub struct Editor {
    focus: FocusHandle,
    content: String,
    /// Byte offset where each line starts. Always holds at least `0`; rebuilt on every edit.
    line_starts: Vec<usize>,
    /// Byte offsets into `content`, always on a char boundary.
    selection: Range<usize>,
    reversed: bool,
    marked: Option<Range<usize>>,
    /// The view: its first line and how many px of that line sit above the top.
    scroll: Scroll,
    /// Scroll requests for the next paint, which is where layout is known:
    /// bring the caret into view, then move by `scroll_px` (positive is down).
    follow_caret: bool,
    scroll_px: f32,
    view_height: f32,
    /// The width text wrapped to at the last paint. None in single-line mode and before the first paint.
    wrap_width: Option<f32>,
    /// The font and size the last paint shaped with. Key handlers run outside
    /// paint, so they must shape with these, not the window's default style.
    paint_style: Option<(Font, Pixels)>,
    /// Row-local x the Up/Down run keeps, in px.
    desired_x: Option<f32>,
    /// The caret sits at the end of the row above a soft wrap, not the start of the next.
    at_row_end: bool,
    history: History,
    last_edit: Option<(Instant, EditKind)>,
    last_bounds: Option<Bounds<Pixels>>,
    /// The lines the last paint laid out, top to bottom, for hit-testing and IME.
    last_lines: Vec<VisibleLine>,
    /// Ranges the find bar has matched, painted behind the text. Sorted by start.
    search_ranges: Vec<Range<usize>>,
    search_active: Option<Range<usize>>,
    selecting: bool,
    single_line: bool,
    /// Where the text starts inside the element; compact inputs use smaller values.
    pad_x: f32,
    pad_y: f32,
    /// Palette id from `theme::THEMES`; "system" follows the OS appearance.
    pub theme_name: String,
    /// The language the buffer is coloured as; `None` keeps the line-level Markdown colouring.
    language: Option<&'static str>,
    /// Built on first use and kept across repaints; see `highlight::LineCache`.
    highlighter: Option<LineCache>,
    layout: LayoutState,
    placeholder: SharedString,
    pub on_submit: Option<Box<dyn Fn(&mut App)>>,
    pub on_cancel: Option<Box<dyn Fn(&mut App)>>,
    /// Called with the whole buffer after every edit, so the tab it belongs to
    /// can track whether it differs from what is on disk.
    pub on_change: Option<Box<dyn Fn(&str, &mut App)>>,
}

/// Whether an edit inserts text or only removes it. Quick edits of one kind
/// fold into one undo step; switching kind starts a new step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditKind {
    Insert,
    Delete,
}

/// Whether an edit of `kind` at `now` folds into the undo step opened by `last`.
fn folds_into_undo(last: Option<(Instant, EditKind)>, now: Instant, kind: EditKind) -> bool {
    match last {
        Some((at, prev)) => prev == kind && now.duration_since(at) < Duration::from_millis(650),
        None => false,
    }
}

/// Layout state kept across paints. Edits clear what depends on line indices;
/// the shaped paragraphs survive because they are keyed by their content.
#[derive(Default)]
struct LayoutState {
    shapes: ShapeCache,
    /// Hash of the style `heights` were measured with; a change clears them.
    style_key: u64,
    /// Measured height in px of each line, by line index.
    heights: Vec<Option<f32>>,
    /// Whether each line ends inside a code fence, from `fence_marks`.
    fences: Vec<bool>,
}

/// One laid-out line of the view.
struct VisibleLine {
    index: usize,
    /// Byte offset of the line's first byte.
    start: usize,
    /// Element-local y of the line's first row; includes the top padding.
    top: f32,
    height: f32,
    para: Rc<WrappedText>,
}

/// The view's scroll position. Line heights come from the caller, so the
/// arithmetic here is pure and tested on its own.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Scroll {
    pub line: usize,
    /// Px of `line` scrolled above the top of the view.
    pub offset: f32,
}

impl Scroll {
    /// Moves down by `px` through `count` lines, stopping at the bottom of the last one.
    pub fn down(&mut self, px: f32, count: usize, mut height: impl FnMut(usize) -> f32) {
        let mut left = px;
        loop {
            let h = height(self.line);
            if self.offset + left < h || self.line + 1 >= count {
                self.offset = (self.offset + left).min(h.max(0.0));
                return;
            }
            left -= h - self.offset;
            self.line += 1;
            self.offset = 0.0;
        }
    }

    /// Moves up by `px` through the lines above, stopping at the top of line 0.
    pub fn up(&mut self, px: f32, mut height: impl FnMut(usize) -> f32) {
        let mut left = px;
        loop {
            if self.offset >= left {
                self.offset -= left;
                return;
            }
            left -= self.offset;
            if self.line == 0 {
                self.offset = 0.0;
                return;
            }
            self.line -= 1;
            self.offset = height(self.line);
        }
    }
}

/// What one layout pass shapes with: the text style, and the tokens of the
/// window it starts from.
struct Frame {
    ts: Arc<WindowTextSystem>,
    font: Font,
    font_size: Pixels,
    theme: Theme,
    wrap: Option<f32>,
    highlighting: bool,
    tokens_first: usize,
    tokens: Tokens,
}

/// The quads painted over and under the text for one frame.
#[derive(Default)]
struct Overlay {
    search: Vec<PaintQuad>,
    selection: Vec<PaintQuad>,
    caret: Option<PaintQuad>,
}

impl Editor {
    pub fn new(single_line: bool, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            content: String::new(),
            line_starts: vec![0],
            selection: 0..0,
            reversed: false,
            marked: None,
            scroll: Scroll::default(),
            follow_caret: false,
            scroll_px: 0.0,
            view_height: 0.0,
            wrap_width: None,
            paint_style: None,
            desired_x: None,
            at_row_end: false,
            history: History::default(),
            last_edit: None,
            last_bounds: None,
            last_lines: Vec::new(),
            search_ranges: Vec::new(),
            search_active: None,
            selecting: false,
            single_line,
            pad_x: PAD_X,
            pad_y: PAD_Y,
            theme_name: "system".to_string(),
            language: None,
            highlighter: None,
            layout: LayoutState::default(),
            placeholder: SharedString::default(),
            on_submit: None,
            on_cancel: None,
            on_change: None,
        }
    }

    pub fn with_placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// Smaller insets, for a one-line field in a toolbar or dialog.
    pub fn with_padding(mut self, pad_x: f32, pad_y: f32) -> Self {
        self.pad_x = pad_x;
        self.pad_y = pad_y;
        self
    }

    pub fn set_theme(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.theme_name != name {
            self.theme_name = name.to_string();
            cx.notify();
        }
    }

    /// Colours the buffer as `lang` (a `highlight` language id), or back to the
    /// line-level Markdown colouring with `None`. Single-line fields ignore it.
    /// Does not notify: call `cx.notify()` after changing it.
    pub fn set_language(&mut self, lang: Option<&'static str>) {
        if self.language != lang {
            self.language = lang;
            self.highlighter = None;
        }
    }

    fn highlighting(&self) -> bool {
        self.language.is_some() && !self.single_line && self.content.len() <= highlight::MAX_BYTES
    }

    /// Token ranges for lines `first..last`, relative to each line's start. None
    /// when the buffer is not highlighted (no language, a single-line field, or
    /// a buffer over `highlight::MAX_BYTES`).
    fn highlight_range(&mut self, first: usize, last: usize) -> Option<Tokens> {
        if !self.highlighting() {
            self.highlighter = None;
            return None;
        }
        let language = self.language?;
        if self.highlighter.is_none() {
            self.highlighter = LineCache::new(language);
        }
        let cache = self.highlighter.as_mut()?;
        Some(cache.lines(&self.content, first, last))
    }

    /// Token ranges for the lines the view starts with, enough for one view.
    pub fn highlight_visible(&mut self, view_height: f32) -> Option<Tokens> {
        let first = self.scroll.line;
        let last = (first + rows_in(view_height) + TOKEN_SLACK_ROWS).min(self.line_count());
        self.highlight_range(first, last)
    }

    pub fn text(&self) -> &str {
        &self.content
    }

    /// The selected byte range (the caret when empty). Find steps from it.
    pub fn selection(&self) -> Range<usize> {
        self.selection.clone()
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

    /// Every case-insensitive, non-overlapping match of `query` in the buffer.
    /// Offsets stay byte-accurate even when lowercasing a character changes its
    /// length: the folded copy is only used when it has the same byte count.
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
        self.set_selection(start..end, cx);
    }

    /// Replaces one range; used by the find bar's "Replace" button.
    pub fn replace_at(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) {
        self.replace_range(range, text, cx);
    }

    /// Replaces every match in one undo step.
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
        self.push_undo(EditKind::Insert);
        let mut next = String::with_capacity(self.content.len());
        let mut last = 0;
        for range in &ranges {
            next.push_str(&self.content[last..range.start]);
            next.push_str(replacement);
            last = range.end;
        }
        next.push_str(&self.content[last..]);
        self.content = next;
        self.selection = self.content.len()..self.content.len();
        self.reversed = false;
        self.at_row_end = false;
        self.search_ranges.clear();
        self.search_active = None;
        self.desired_x = None;
        self.reindex();
        self.follow_caret = true;
        self.notify_changed(cx);
        cx.notify();
        ranges.len()
    }

    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.content = if self.single_line {
            text.replace(['\n', '\r'], " ")
        } else {
            text.to_string()
        };
        self.selection = 0..0;
        self.reversed = false;
        self.marked = None;
        self.at_row_end = false;
        self.desired_x = None;
        self.history.clear();
        self.scroll = Scroll::default();
        self.scroll_px = 0.0;
        self.follow_caret = false;
        self.reindex();
        cx.notify();
    }

    fn cursor(&self) -> usize {
        if self.reversed {
            self.selection.start
        } else {
            self.selection.end
        }
    }

    fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    fn line_index_of(&self, offset: usize) -> usize {
        let offset = offset.min(self.content.len());
        self.line_starts.partition_point(|&s| s <= offset) - 1
    }

    /// Byte offset of the line's end, before its line break.
    fn line_end_at(&self, line: usize) -> usize {
        match self.line_starts.get(line + 1) {
            Some(&next) => next - 1,
            None => self.content.len(),
        }
    }

    fn prev_boundary(&self, offset: usize) -> usize {
        rich_ops::prev_grapheme(&self.content, offset)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        rich_ops::next_grapheme(&self.content, offset)
    }

    /// Rebuilds everything derived from the text. Call after every content change.
    fn reindex(&mut self) {
        self.line_starts = line_starts(&self.content);
        self.layout.heights.clear();
        self.layout.fences.clear();
    }

    /// Records a restore point, folding quick consecutive edits of the same kind
    /// into one step so typing a sentence is a single undo. Switching between
    /// inserting and deleting starts a new step.
    fn push_undo(&mut self, kind: EditKind) {
        let now = Instant::now();
        if !folds_into_undo(self.last_edit, now, kind) {
            self.history
                .record((self.content.clone(), self.selection.clone()));
        }
        self.last_edit = Some((now, kind));
    }

    fn notify_changed(&mut self, cx: &mut Context<Self>) {
        if let Some(on_change) = self.on_change.as_ref() {
            on_change(&self.content, cx);
        }
    }

    fn replace_range(&mut self, range: Range<usize>, new_text: &str, cx: &mut Context<Self>) {
        let new_text = if self.single_line {
            new_text.replace(['\n', '\r'], " ")
        } else {
            new_text.to_string()
        };
        self.push_undo(if new_text.is_empty() {
            EditKind::Delete
        } else {
            EditKind::Insert
        });
        self.content.replace_range(range.clone(), &new_text);
        let caret = range.start + new_text.len();
        self.selection = caret..caret;
        self.reversed = false;
        self.marked = None;
        self.at_row_end = false;
        self.desired_x = None;
        self.reindex();
        self.follow_caret = true;
        self.notify_changed(cx);
        cx.notify();
    }

    fn insert(&mut self, text: &str, cx: &mut Context<Self>) {
        self.replace_range(self.selection.clone(), text, cx);
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if let Some(on_submit) = self.on_submit.as_ref() {
            on_submit(cx);
        }
    }

    /// Moves the caret to `offset`, or extends the selection to it when
    /// `extend`. `at_row_end` says which side of a soft wrap the caret sits on.
    /// `keep_x` keeps the Up/Down column (only vertical moves do that).
    fn place(
        &mut self,
        offset: usize,
        at_row_end: bool,
        extend: bool,
        keep_x: bool,
        cx: &mut Context<Self>,
    ) {
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
        self.follow_caret = true;
        cx.notify();
    }

    fn set_selection(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        self.selection = range;
        self.reversed = false;
        self.at_row_end = false;
        self.desired_x = None;
        self.follow_caret = true;
        cx.notify();
    }

    fn line_range(&self, offset: usize) -> Range<usize> {
        let line = self.line_index_of(offset);
        self.line_starts[line]..self.line_end_at(line)
    }

    // ---- layout --------------------------------------------------------

    /// The per-pass style: font, colours, wrap width, and the token window.
    /// Layout runs in prepaint, where the window's text style is the editor's.
    fn frame(&mut self, window: &Window, view_height: f32, wrap: Option<f32>) -> Frame {
        let style = window.text_style();
        let font = style.font();
        let font_size = style.font_size.to_pixels(window.rem_size());
        self.frame_with(window, (font, font_size), view_height, wrap)
    }

    /// The frame for caret movement. Key handlers run outside paint, where the
    /// window's text style is the default (system font, 16px), not the editor's
    /// (Menlo 14px). Shaping with that breaks rows where the screen does not
    /// show them, so reuse the style of the last paint.
    fn navigation_frame(&mut self, window: &Window) -> Frame {
        let style = match self.paint_style.clone() {
            Some(style) => style,
            None => {
                let style = window.text_style();
                (style.font(), style.font_size.to_pixels(window.rem_size()))
            }
        };
        let (view_height, wrap) = (self.view_height, self.wrap_width);
        self.frame_with(window, style, view_height, wrap)
    }

    fn frame_with(
        &mut self,
        window: &Window,
        (font, font_size): (Font, Pixels),
        view_height: f32,
        wrap: Option<f32>,
    ) -> Frame {
        let theme = Theme::for_name(&self.theme_name, window.appearance());
        let highlighting = self.highlighting();
        let tokens_first = self.scroll.line;
        let tokens = self.highlight_visible(view_height).unwrap_or_default();
        Frame {
            ts: window.text_system().clone(),
            font,
            font_size,
            theme,
            wrap,
            highlighting,
            tokens_first,
            tokens,
        }
    }

    /// The shaped paragraph for `line`. Shaped once per (text, runs, width), so
    /// a repeat costs a lookup.
    fn paragraph(&mut self, frame: &mut Frame, line: usize) -> Rc<WrappedText> {
        let start = self.line_starts[line];
        let end = self.line_end_at(line);
        let prefetched = line
            .checked_sub(frame.tokens_first)
            .and_then(|i| frame.tokens.get(i))
            .cloned();
        let tokens = if !frame.highlighting {
            None
        } else if prefetched.is_some() {
            prefetched
        } else {
            self.highlight_range(line, line + 1)
                .and_then(|mut lines| lines.pop())
        };
        let in_code = tokens.is_none() && self.code_state(line);
        let text = &self.content[start..end];
        let runs: Vec<TextRun> = if text.is_empty() {
            Vec::new()
        } else if let Some(tokens) = &tokens {
            token_runs(text, tokens, &frame.font, &frame.theme)
        } else {
            vec![TextRun {
                len: text.len(),
                font: frame.font.clone(),
                color: line_colour(text, in_code, &frame.theme),
                background_color: None,
                underline: None,
                strikethrough: None,
            }]
        };
        self.layout.shapes.get_or_shape(
            &frame.ts,
            text,
            frame.font_size,
            &runs,
            frame.wrap.map(px),
            px(LINE_HEIGHT),
        )
    }

    /// Whether `line` is inside a code fence, counting the fence lines themselves
    /// the way `line_colour` expects.
    fn code_state(&mut self, line: usize) -> bool {
        if self.layout.fences.is_empty() {
            self.layout.fences = fence_marks(&self.content);
        }
        self.layout.fences.get(line).copied().unwrap_or(false)
    }

    /// Height of `line` in px, from the cache or by laying it out.
    fn line_height_px(&mut self, frame: &mut Frame, line: usize) -> f32 {
        if let Some(Some(height)) = self.layout.heights.get(line) {
            return *height;
        }
        let height = self.paragraph(frame, line).height();
        if let Some(slot) = self.layout.heights.get_mut(line) {
            *slot = Some(height);
        }
        height
    }

    fn scroll_down(&mut self, frame: &mut Frame, px: f32) {
        let count = self.line_count();
        let mut scroll = self.scroll;
        scroll.down(px, count, |line| self.line_height_px(frame, line));
        self.scroll = scroll;
    }

    fn scroll_up(&mut self, frame: &mut Frame, px: f32) {
        let mut scroll = self.scroll;
        scroll.up(px, |line| self.line_height_px(frame, line));
        self.scroll = scroll;
    }

    /// Scrolls the minimum that brings the caret's row into view.
    fn follow_caret_now(&mut self, frame: &mut Frame) {
        let cursor = self.cursor();
        let line = self.line_index_of(cursor);
        let para = self.paragraph(frame, line);
        let (row, _) = para.caret_row_x(cursor - self.line_starts[line], self.at_row_end);
        let row_top = row as f32 * LINE_HEIGHT;
        if line < self.scroll.line || (line == self.scroll.line && row_top < self.scroll.offset) {
            self.scroll = Scroll {
                line,
                offset: row_top,
            };
            return;
        }
        // Distance from the top of the view to the bottom of the caret's row.
        let mut below = row_top + LINE_HEIGHT - self.scroll.offset;
        for above in self.scroll.line..line {
            below += self.line_height_px(frame, above);
        }
        let limit = self.view_height - self.pad_y;
        if below > limit {
            self.scroll_down(frame, below - limit);
        }
    }

    /// The visible lines, top to bottom, from the scroll position down to the
    /// bottom of the view. Also returns the bottom edge of the last one, and
    /// whether the document ended before the view was full.
    fn walk(&mut self, frame: &mut Frame) -> (Vec<VisibleLine>, f32, bool) {
        let mut lines = Vec::new();
        let mut line = self.scroll.line;
        let mut top = self.pad_y - self.scroll.offset;
        loop {
            let start = self.line_starts[line];
            let para = self.paragraph(frame, line);
            let height = para.height();
            if let Some(slot) = self.layout.heights.get_mut(line) {
                *slot = Some(height);
            }
            lines.push(VisibleLine {
                index: line,
                start,
                top,
                height,
                para,
            });
            top += height;
            if top >= self.view_height {
                return (lines, top, false);
            }
            if line + 1 >= self.line_count() {
                return (lines, top, true);
            }
            line += 1;
        }
    }

    /// Lays out one frame: applies the pending scroll requests, then walks the
    /// visible lines. Runs in prepaint, where the window can shape text.
    fn layout_frame(
        &mut self,
        window: &mut Window,
        width: f32,
        view_height: f32,
    ) -> Vec<VisibleLine> {
        self.view_height = view_height;
        let wrap = if self.single_line {
            None
        } else {
            Some((width - 2.0 * self.pad_x).max(16.0))
        };
        self.wrap_width = wrap;
        let mut frame = self.frame(window, view_height, wrap);
        self.paint_style = Some((frame.font.clone(), frame.font_size));

        let key = self.style_key(&frame);
        if self.layout.style_key != key {
            self.layout.style_key = key;
            self.layout.heights.clear();
        }
        self.layout.heights.resize(self.line_count(), None);

        if self.single_line {
            self.scroll = Scroll::default();
            self.scroll_px = 0.0;
            self.follow_caret = false;
        } else {
            let last = self.line_count() - 1;
            if self.scroll.line > last {
                self.scroll = Scroll {
                    line: last,
                    offset: 0.0,
                };
            }
            if std::mem::take(&mut self.follow_caret) {
                self.follow_caret_now(&mut frame);
            }
            let delta = std::mem::take(&mut self.scroll_px);
            if delta > 0.0 {
                self.scroll_down(&mut frame, delta);
            } else if delta < 0.0 {
                self.scroll_up(&mut frame, -delta);
            }
            let top_height = self.line_height_px(&mut frame, self.scroll.line);
            self.scroll.offset = self.scroll.offset.clamp(0.0, top_height);
        }

        let (lines, bottom, ended) = self.walk(&mut frame);
        // Near the end of the document the view can show empty space below the
        // last line. Pull the top back so the last line sits on the bottom padding.
        let slack = self.view_height - self.pad_y - bottom;

        if ended && slack > 0.0 && (self.scroll.line > 0 || self.scroll.offset > 0.0) {
            self.scroll_up(&mut frame, slack);
            self.walk(&mut frame).0
        } else {
            lines
        }
    }

    fn style_key(&self, frame: &Frame) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.language.hash(&mut hasher);
        frame.font.family.as_ref().hash(&mut hasher);
        f32::from(frame.font_size).to_bits().hash(&mut hasher);
        frame.wrap.map(f32::to_bits).hash(&mut hasher);
        hasher.finish()
    }

    /// Selection, search and caret quads for the laid-out lines.
    fn overlay(&self, lines: &[VisibleLine], bounds: Bounds<Pixels>, theme: &Theme) -> Overlay {
        let mut out = Overlay::default();
        let caret = self.cursor();
        for line in lines {
            let origin = point(bounds.left() + px(self.pad_x), bounds.top() + px(line.top));
            let end = line.start + line.para.len();

            let first = self
                .search_ranges
                .partition_point(|found| found.end <= line.start);
            for found in &self.search_ranges[first..] {
                if found.start >= end {
                    break;
                }
                let local =
                    found.start.max(line.start) - line.start..found.end.min(end) - line.start;
                let colour = if self.search_active.as_ref() == Some(found) {
                    rgba(0xf5a52480)
                } else {
                    rgba(0xf5a52433)
                };
                out.search
                    .extend(line.para.range_quads(origin, local, colour, px(1.0)));
            }

            if !self.selection.is_empty() {
                let s = self.selection.start.max(line.start);
                let e = self.selection.end.min(end);
                if s < e {
                    out.selection.extend(line.para.range_quads(
                        origin,
                        s - line.start..e - line.start,
                        theme.selection,
                        px(1.0),
                    ));
                }
                // A selected line break shows as a cell after the line's last glyph.
                let break_selected = self.selection.start <= end && self.selection.end > end;
                if break_selected && end < self.content.len() {
                    let at = line.para.position_for_index(line.para.len(), false);
                    out.selection.push(fill(
                        Bounds::new(
                            point(origin.x + at.x, origin.y + at.y),
                            size(px(NEWLINE_MARK_WIDTH), px(LINE_HEIGHT)),
                        ),
                        theme.selection,
                    ));
                }
            }

            if out.caret.is_none() && caret >= line.start && caret <= end {
                out.caret = Some(line.para.caret_quad(
                    origin,
                    caret - line.start,
                    self.at_row_end,
                    theme.cursor,
                    px(CARET_WIDTH),
                ));
            }
        }
        out
    }

    // ---- caret movement ------------------------------------------------

    /// The caret one visual row up or down, keeping the column. None at the
    /// first or last row of the document.
    fn vertical_target(&mut self, window: &Window, delta: isize) -> Option<(usize, bool)> {
        let cursor = self.cursor();
        let line = self.line_index_of(cursor);
        let start = self.line_starts[line];
        let mut frame = self.navigation_frame(window);
        let para = self.paragraph(&mut frame, line);
        let (row, x) = para.caret_row_x(cursor - start, self.at_row_end);
        let x = *self.desired_x.get_or_insert(x);

        if delta < 0 {
            if row > 0 {
                let (index, at_end) = para.index_at_row(row - 1, x);
                return Some((start + index, at_end));
            }
            if line == 0 {
                return None;
            }
            let prev_start = self.line_starts[line - 1];
            let prev = self.paragraph(&mut frame, line - 1);
            let (index, at_end) = prev.index_at_row(prev.row_count() - 1, x);
            Some((prev_start + index, at_end))
        } else {
            if row + 1 < para.row_count() {
                let (index, at_end) = para.index_at_row(row + 1, x);
                return Some((start + index, at_end));
            }
            if line + 1 >= self.line_count() {
                return None;
            }
            let next_start = self.line_starts[line + 1];
            let next = self.paragraph(&mut frame, line + 1);
            let (index, at_end) = next.index_at_row(0, x);
            Some((next_start + index, at_end))
        }
    }

    fn vertical(&mut self, window: &Window, delta: isize, extend: bool, cx: &mut Context<Self>) {
        if self.single_line {
            return;
        }
        if let Some((offset, at_end)) = self.vertical_target(window, delta) {
            self.place(offset, at_end, extend, true, cx);
        }
    }

    /// Start or end of the visual row the caret is on.
    fn visual_edge(&mut self, window: &Window, end: bool, extend: bool, cx: &mut Context<Self>) {
        let cursor = self.cursor();
        let line = self.line_index_of(cursor);
        let start = self.line_starts[line];
        let mut frame = self.navigation_frame(window);
        let para = self.paragraph(&mut frame, line);
        let (row, _) = para.caret_row_x(cursor - start, self.at_row_end);
        let (target, at_row_end) = if !end {
            (start + para.rows().start(row), false)
        } else if row + 1 < para.row_count() {
            (start + para.rows().end(row), true)
        } else {
            (start + para.len(), false)
        };
        self.place(target, at_row_end, extend, false, cx);
    }

    // ---- actions -------------------------------------------------------

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        if self.selection.is_empty() {
            let start = rich_ops::backspace_start(&self.content, self.cursor());
            self.selection = start..self.cursor();
        }
        self.replace_range(self.selection.clone(), "", cx);
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        if self.selection.is_empty() {
            let cursor = self.cursor();
            let next = self.next_boundary(cursor);
            self.selection = cursor..next;
        }
        self.replace_range(self.selection.clone(), "", cx);
    }

    fn enter(&mut self, _: &Enter, _: &mut Window, cx: &mut Context<Self>) {
        if self.single_line {
            self.submit(cx);
        } else {
            self.insert("\n", cx);
        }
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(on_cancel) = self.on_cancel.as_ref() {
            on_cancel(cx);
        }
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selection.is_empty() {
            let prev = self.prev_boundary(self.cursor());
            self.place(prev, false, false, false, cx);
        } else {
            let start = self.selection.start;
            self.place(start, false, false, false, cx);
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selection.is_empty() {
            let next = self.next_boundary(self.cursor());
            self.place(next, false, false, false, cx);
        } else {
            let end = self.selection.end;
            self.place(end, false, false, false, cx);
        }
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        let prev = self.prev_boundary(self.cursor());
        self.place(prev, false, true, false, cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        let next = self.next_boundary(self.cursor());
        self.place(next, false, true, false, cx);
    }

    fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let target = prev_word(&self.content, self.cursor());
        self.place(target, false, false, false, cx);
    }

    fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        let target = next_word(&self.content, self.cursor());
        self.place(target, false, false, false, cx);
    }

    fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let target = prev_word(&self.content, self.cursor());
        self.place(target, false, true, false, cx);
    }

    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        let target = next_word(&self.content, self.cursor());
        self.place(target, false, true, false, cx);
    }

    fn up(&mut self, _: &Up, window: &mut Window, cx: &mut Context<Self>) {
        self.vertical(window, -1, false, cx);
    }

    fn down(&mut self, _: &Down, window: &mut Window, cx: &mut Context<Self>) {
        self.vertical(window, 1, false, cx);
    }

    fn select_up(&mut self, _: &SelectUp, window: &mut Window, cx: &mut Context<Self>) {
        self.vertical(window, -1, true, cx);
    }

    fn select_down(&mut self, _: &SelectDown, window: &mut Window, cx: &mut Context<Self>) {
        self.vertical(window, 1, true, cx);
    }

    fn home(&mut self, _: &Home, window: &mut Window, cx: &mut Context<Self>) {
        self.visual_edge(window, false, false, cx);
    }

    fn end(&mut self, _: &End, window: &mut Window, cx: &mut Context<Self>) {
        self.visual_edge(window, true, false, cx);
    }

    fn select_home(&mut self, _: &SelectHome, window: &mut Window, cx: &mut Context<Self>) {
        self.visual_edge(window, false, true, cx);
    }

    fn select_end(&mut self, _: &SelectEnd, window: &mut Window, cx: &mut Context<Self>) {
        self.visual_edge(window, true, true, cx);
    }

    /// Cmd-Up/Down. Multi-line only: single-line inputs keep their own keys.
    fn doc_start(&mut self, _: &DocStart, _: &mut Window, cx: &mut Context<Self>) {
        if !self.single_line {
            self.place(0, false, false, false, cx);
        }
    }

    fn doc_end(&mut self, _: &DocEnd, _: &mut Window, cx: &mut Context<Self>) {
        if !self.single_line {
            self.place(self.content.len(), false, false, false, cx);
        }
    }

    fn select_doc_start(&mut self, _: &SelectDocStart, _: &mut Window, cx: &mut Context<Self>) {
        if !self.single_line {
            self.place(0, false, true, false, cx);
        }
    }

    fn select_doc_end(&mut self, _: &SelectDocEnd, _: &mut Window, cx: &mut Context<Self>) {
        if !self.single_line {
            self.place(self.content.len(), false, true, false, cx);
        }
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selection = 0..self.content.len();
        self.reversed = false;
        self.at_row_end = false;
        self.desired_x = None;
        cx.notify();
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.insert(&text, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selection.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selection.clone()].to_string(),
            ));
        }
    }

    fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selection.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selection.clone()].to_string(),
            ));
            self.replace_range(self.selection.clone(), "", cx);
        }
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if self.history.undo_len() == 0 {
            return;
        }
        let current = (std::mem::take(&mut self.content), self.selection.clone());
        if let Some((content, selection)) = self.history.undo(current) {
            self.restore(content, selection, cx);
        }
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if self.history.redo_len() == 0 {
            return;
        }
        let current = (std::mem::take(&mut self.content), self.selection.clone());
        if let Some((content, selection)) = self.history.redo(current) {
            self.restore(content, selection, cx);
        }
    }

    fn restore(&mut self, content: String, selection: Range<usize>, cx: &mut Context<Self>) {
        self.content = content;
        self.selection = selection;
        self.reversed = false;
        self.at_row_end = false;
        self.desired_x = None;
        self.last_edit = None;
        self.reindex();
        self.follow_caret = true;
        self.notify_changed(cx);
        cx.notify();
    }

    // ---- mouse and wheel -----------------------------------------------

    /// The caret position under a window-space point, and its row-end flag,
    /// using the lines the last paint laid out.
    fn hit(&self, position: Point<Pixels>) -> (usize, bool) {
        let Some(bounds) = self.last_bounds else {
            return (self.cursor(), false);
        };
        let y = f32::from(position.y - bounds.top());
        let x = f32::from(position.x - bounds.left()) - self.pad_x;
        let line = self
            .last_lines
            .iter()
            .find(|line| y < line.top + line.height)
            .or(self.last_lines.last());
        let Some(line) = line else {
            return (self.cursor(), false);
        };
        let (index, at_row_end) = line.para.hit_test(point(px(x), px(y - line.top)));
        (line.start + index, at_row_end)
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus);
        self.selecting = true;
        let (offset, at_row_end) = self.hit(event.position);
        if event.click_count >= 3 {
            let range = self.line_range(offset);
            self.set_selection(range, cx);
        } else if event.click_count == 2 {
            let range = word_range(&self.content, offset);
            self.set_selection(range, cx);
        } else {
            self.place(offset, at_row_end, event.modifiers.shift, false, cx);
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.selecting {
            let (offset, at_row_end) = self.hit(event.position);
            self.place(offset, at_row_end, true, false, cx);
        }
    }

    fn on_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.single_line {
            return;
        }
        let delta = f32::from(event.delta.pixel_delta(px(LINE_HEIGHT)).y);
        self.scroll_px -= delta;
        cx.notify();
    }

    // ---- UTF-16 bridging ----------------------------------------------

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_count = 0;
        for ch in self.content.chars() {
            if utf16_count >= offset {
                break;
            }
            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }
        utf8_offset
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        let mut utf8_count = 0;
        for ch in self.content.chars() {
            if utf8_count >= offset {
                break;
            }
            utf8_count += ch.len_utf8();
            utf16_offset += ch.len_utf16();
        }
        utf16_offset
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range_utf16.start)..self.offset_from_utf16(range_utf16.end)
    }
}

impl Focusable for Editor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EntityInputHandler for Editor {
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
        let line_index = self.line_index_of(range.start);
        let line = self
            .last_lines
            .iter()
            .find(|line| line.index == line_index)?;
        let line_end = line.start + line.para.len();
        let from = line
            .para
            .position_for_index(range.start - line.start, false);
        let to_offset = range.end.min(line_end) - line.start;
        let to = line.para.position_for_index(to_offset, false);
        // A range that crosses a soft wrap is measured to the start of its row.
        let right = if to.y == from.y { to.x } else { from.x };
        let top = bounds.top() + px(line.top) + from.y;
        let left = bounds.left() + px(self.pad_x);
        Some(Bounds::from_corners(
            point(left + from.x, top),
            point(left + right, top + px(LINE_HEIGHT)),
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

// ---- pure helpers ------------------------------------------------------

/// Byte offset where each line of `content` starts, always beginning with 0.
pub fn line_starts(content: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(content.match_indices('\n').map(|(i, _)| i + 1))
        .collect()
}

/// For each line, whether the line ends inside a code fence. A line that opens
/// or closes a fence counts as inside it, the way the colouring reads it.
pub fn fence_marks(content: &str) -> Vec<bool> {
    let mut in_code = false;
    content
        .split('\n')
        .map(|line| {
            if line.trim_start().starts_with("```") {
                in_code = !in_code;
            }
            in_code
        })
        .collect()
}

/// Rows of `height` px the view spans, rounded up.
fn rows_in(height: f32) -> usize {
    (height / LINE_HEIGHT).ceil().max(0.0) as usize
}

/// Which class a character belongs to for word selection and word motion.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CharClass {
    Word,
    Space,
    Other,
    LineBreak,
}

fn is_word_char(c: char) -> bool {
    // Thai has no spaces between words, so the whole Thai block counts as one
    // word. That selects a run of Thai text, not a dictionary word.
    c.is_alphanumeric() || c == '_' || ('\u{0E00}'..='\u{0E7F}').contains(&c)
}

fn char_class(c: char) -> CharClass {
    if c == '\n' {
        CharClass::LineBreak
    } else if is_word_char(c) {
        CharClass::Word
    } else if c.is_whitespace() {
        CharClass::Space
    } else {
        CharClass::Other
    }
}

/// The run of same-class characters around `offset`, for double-click selection.
/// Prefers the character after the caret, then the one before, so a click just
/// past a word's last letter still selects that word.
pub fn word_range(text: &str, offset: usize) -> Range<usize> {
    let offset = offset.min(text.len());
    let after = text[offset..].chars().next();
    let before = text[..offset].chars().next_back();
    let picked = if after.is_some_and(is_word_char) {
        after
    } else if before.is_some_and(is_word_char) {
        before
    } else {
        after.or(before)
    };
    let Some(picked) = picked else {
        return offset..offset;
    };
    let class = char_class(picked);
    if class == CharClass::LineBreak {
        return offset..offset;
    }
    let mut start = offset;
    while let Some(c) = text[..start].chars().next_back() {
        if char_class(c) != class {
            break;
        }
        start -= c.len_utf8();
    }
    let mut end = offset;
    while let Some(c) = text[end..].chars().next() {
        if char_class(c) != class {
            break;
        }
        end += c.len_utf8();
    }
    start..end
}

/// Where Alt-Left lands: the start of the word before `offset`. Spaces are
/// skipped first, and a line break counts as one stop.
pub fn prev_word(text: &str, offset: usize) -> usize {
    let mut i = offset.min(text.len());
    while let Some(c) = text[..i].chars().next_back() {
        if char_class(c) != CharClass::Space {
            break;
        }
        i -= c.len_utf8();
    }
    if let Some(c) = text[..i].chars().next_back() {
        let class = char_class(c);
        while let Some(c) = text[..i].chars().next_back() {
            if char_class(c) != class {
                break;
            }
            i -= c.len_utf8();
            if class == CharClass::LineBreak {
                break;
            }
        }
    }
    rich_ops::cluster_floor(text, i)
}

/// Where Alt-Right lands: the end of the word after `offset`. Spaces are
/// skipped first, and a line break counts as one stop.
pub fn next_word(text: &str, offset: usize) -> usize {
    let mut i = offset.min(text.len());
    while let Some(c) = text[i..].chars().next() {
        if char_class(c) != CharClass::Space {
            break;
        }
        i += c.len_utf8();
    }
    if let Some(c) = text[i..].chars().next() {
        let class = char_class(c);
        while let Some(c) = text[i..].chars().next() {
            if char_class(c) != class {
                break;
            }
            i += c.len_utf8();
            if class == CharClass::LineBreak {
                break;
            }
        }
    }
    rich_ops::cluster_ceil(text, i)
}

/// Which colour a line is painted in, from the line's own Markdown shape. This
/// is the port's stand-in for CodeMirror's highlighter: line-level, not
/// inline, but enough to make a note readable.
fn line_colour(line: &str, in_code_block: bool, theme: &Theme) -> Hsla {
    let colour = if in_code_block {
        theme.code
    } else {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            theme.heading
        } else if trimmed.starts_with('>') {
            theme.quote
        } else if trimmed.starts_with("```") {
            theme.code
        } else {
            theme.fg
        }
    };
    colour.into()
}

/// The text runs for one line from its highlight tokens. Gaps between tokens
/// are plain, so the runs always add up to the line's length.
fn token_runs(
    line: &str,
    tokens: &[(Range<usize>, TokenKind)],
    font: &Font,
    theme: &Theme,
) -> Vec<TextRun> {
    let mut runs = Vec::with_capacity(tokens.len() * 2 + 1);
    let mut pos = 0;
    for (range, kind) in tokens {
        let start = range.start.max(pos);
        if start >= range.end || range.end > line.len() {
            continue;
        }
        if start > pos {
            runs.push(text_run(start - pos, TokenKind::Plain, font, theme));
        }
        runs.push(text_run(range.end - start, *kind, font, theme));
        pos = range.end;
    }
    if pos < line.len() {
        runs.push(text_run(line.len() - pos, TokenKind::Plain, font, theme));
    }
    runs
}

fn text_run(len: usize, kind: TokenKind, font: &Font, theme: &Theme) -> TextRun {
    let (bold, italic) = highlight::emphasis(kind);
    let mut font = font.clone();
    if bold {
        font.weight = FontWeight::BOLD;
    }
    if italic {
        font.style = FontStyle::Italic;
    }
    TextRun {
        len,
        font,
        color: highlight::color(kind, theme),
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}

// ---- element -------------------------------------------------------------

struct EditorElement {
    editor: Entity<Editor>,
}

struct Prepaint {
    lines: Vec<VisibleLine>,
    overlay: Overlay,
    bounds: Bounds<Pixels>,
    pad_x: f32,
    view_height: f32,
}

impl IntoElement for EditorElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for EditorElement {
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
        style.size.width = relative(1.0).into();
        style.size.height = relative(1.0).into();
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
        let view_height = f32::from(bounds.size.height);
        let (lines, overlay, pad_x) = self.editor.update(cx, |editor, _| {
            let lines = editor.layout_frame(window, width, view_height);
            let theme = Theme::for_name(&editor.theme_name, window.appearance());
            let overlay = editor.overlay(&lines, bounds, &theme);
            (lines, overlay, editor.pad_x)
        });
        Prepaint {
            lines,
            overlay,
            bounds,
            pad_x,
            view_height,
        }
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
        let focus_handle = self.editor.read(cx).focus.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.editor.clone()),
            cx,
        );

        let (theme, is_empty, placeholder) = {
            let editor = self.editor.read(cx);
            (
                Theme::for_name(&editor.theme_name, window.appearance()),
                editor.content.is_empty(),
                editor.placeholder.clone(),
            )
        };
        let overlay = std::mem::take(&mut prepaint.overlay);
        for quad in overlay.search {
            window.paint_quad(quad);
        }
        for quad in overlay.selection {
            window.paint_quad(quad);
        }

        if is_empty && !placeholder.is_empty() {
            let style = window.text_style();
            let font_size = style.font_size.to_pixels(window.rem_size());
            let text: SharedString = placeholder;
            let runs = vec![TextRun {
                len: text.len(),
                font: style.font(),
                color: theme.muted.into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            }];
            let shaped = window
                .text_system()
                .shape_line(text, font_size, &runs, None);
            let _ = shaped.paint(
                Bounds::new(
                    point(
                        prepaint.bounds.left() + px(prepaint.pad_x),
                        prepaint.bounds.top(),
                    ),
                    prepaint.bounds.size,
                )
                .origin,
                px(LINE_HEIGHT),
                window,
                cx,
            );
        }

        let lines = std::mem::take(&mut prepaint.lines);
        for line in &lines {
            let origin = point(
                bounds.left() + px(prepaint.pad_x),
                bounds.top() + px(line.top),
            );
            line.para.paint(window, cx, origin);
        }

        if focus_handle.is_focused(window) {
            if let Some(quad) = overlay.caret {
                window.paint_quad(quad);
            }
        }

        let view_height = prepaint.view_height;
        self.editor.update(cx, |editor, _| {
            editor.last_bounds = Some(bounds);
            editor.last_lines = lines;
            editor.view_height = view_height;
        });
    }
}

impl Render for Editor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::for_name(&self.theme_name, window.appearance());
        div()
            .key_context("Editor")
            .track_focus(&self.focus)
            .cursor(CursorStyle::IBeam)
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            .bg(theme.bg)
            .text_size(px(FONT_SIZE))
            .line_height(px(LINE_HEIGHT))
            .font_family(mono_font())
            .text_color(theme.fg)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::enter))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::doc_start))
            .on_action(cx.listener(Self::doc_end))
            .on_action(cx.listener(Self::select_doc_start))
            .on_action(cx.listener(Self::select_doc_end))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_scroll_wheel(cx.listener(Self::on_scroll_wheel))
            .child(EditorElement {
                editor: cx.entity(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_steps_fold_only_quick_edits_of_one_kind() {
        let t0 = Instant::now();
        let soon = t0 + Duration::from_millis(100);
        let late = t0 + Duration::from_millis(900);
        assert!(folds_into_undo(
            Some((t0, EditKind::Insert)),
            soon,
            EditKind::Insert
        ));
        assert!(!folds_into_undo(
            Some((t0, EditKind::Insert)),
            soon,
            EditKind::Delete
        ));
        assert!(!folds_into_undo(
            Some((t0, EditKind::Delete)),
            soon,
            EditKind::Insert
        ));
        assert!(!folds_into_undo(
            Some((t0, EditKind::Insert)),
            late,
            EditKind::Insert
        ));
        assert!(!folds_into_undo(None, soon, EditKind::Insert));
    }

    #[test]
    fn scroll_down_moves_through_line_heights() {
        let heights = [22.0, 22.0, 44.0, 22.0];
        let mut scroll = Scroll::default();
        scroll.down(30.0, 4, |l| heights[l]);
        assert_eq!(
            scroll,
            Scroll {
                line: 1,
                offset: 8.0
            }
        );

        let mut scroll = Scroll::default();
        scroll.down(100.0, 4, |l| heights[l]);
        assert_eq!(
            scroll,
            Scroll {
                line: 3,
                offset: 12.0
            }
        );
    }

    #[test]
    fn scroll_down_stops_at_the_last_line() {
        let mut scroll = Scroll::default();
        scroll.down(1000.0, 2, |_| 22.0);
        assert_eq!(
            scroll,
            Scroll {
                line: 1,
                offset: 22.0
            }
        );
    }

    #[test]
    fn scroll_up_moves_back_and_stops_at_the_top() {
        let heights = [22.0, 22.0, 44.0, 22.0];
        let mut scroll = Scroll {
            line: 1,
            offset: 8.0,
        };
        scroll.up(10.0, |l| heights[l]);
        assert_eq!(
            scroll,
            Scroll {
                line: 0,
                offset: 20.0
            }
        );

        let mut scroll = Scroll {
            line: 0,
            offset: 5.0,
        };
        scroll.up(10.0, |l| heights[l]);
        assert_eq!(
            scroll,
            Scroll {
                line: 0,
                offset: 0.0
            }
        );
    }

    #[test]
    fn line_starts_follow_newlines() {
        assert_eq!(line_starts(""), vec![0]);
        assert_eq!(line_starts("a\nbc\n"), vec![0, 2, 5]);
    }

    #[test]
    fn fence_marks_include_the_fence_lines() {
        let marks = fence_marks("a\n```\ncode\n```\nb");
        assert_eq!(marks, vec![false, true, true, false, false]);
        assert_eq!(fence_marks("```rust\nfn x() {}"), vec![true, true]);
    }

    #[test]
    fn word_range_picks_the_word_under_or_before_the_caret() {
        assert_eq!(word_range("hello world", 7), 6..11);
        assert_eq!(word_range("hello world", 11), 6..11);
        assert_eq!(word_range("hello world", 5), 0..5);
        assert_eq!(word_range("a  b", 2), 1..3);
        assert_eq!(word_range("a\n\nb", 2), 2..2);
        assert_eq!(word_range("", 0), 0..0);
    }

    #[test]
    fn word_range_treats_thai_as_one_word() {
        let text = "สวัสดี ครับ";
        let first = "สวัสดี".len();
        assert_eq!(word_range(text, 0), 0..first);
        assert_eq!(word_range(text, first + 1), first + 1.."สวัสดี ครับ".len());
    }

    #[test]
    fn word_motion_stops_at_word_edges() {
        let text = "foo bar  baz";
        assert_eq!(next_word(text, 0), 3);
        assert_eq!(next_word(text, 3), 7);
        assert_eq!(prev_word(text, 12), 9);
        assert_eq!(prev_word(text, 9), 4);
        assert_eq!(prev_word(text, 0), 0);
        assert_eq!(next_word("ab\ncd", 2), 3);
        assert_eq!(prev_word("ab\ncd", 3), 2);
    }

    #[test]
    fn mono_font_prefers_the_first_installed_candidate() {
        let installed = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<Vec<_>>();
        // Nothing relevant installed: the platform's first choice.
        assert_eq!(pick_mono_font(&installed(&["Arial"])), MONO_CANDIDATES[0]);
        // Only a later candidate installed: that one wins.
        let last = *MONO_CANDIDATES.last().unwrap();
        assert_eq!(pick_mono_font(&installed(&["Arial", last])), last);
        // Both installed: candidate order, not font-list order.
        assert_eq!(
            pick_mono_font(&installed(&[last, MONO_CANDIDATES[0]])),
            MONO_CANDIDATES[0]
        );
    }
}
