//! Floating widgets over the rich editor: the selection toolbar, the slash
//! menu, the link popover and link tooltip, and the empty-document placeholder.
//!
//! This is a child module of `rich_editor` (declared with `#[path]` at the end
//! of that file), so it reaches the editor's private state while staying in its
//! own file. Pure logic (ranking, trigger detection, mark derivation, placement,
//! edit construction) is at the top and unit tested; the `impl RichEditor`
//! blocks below wire it to input and paint.
//!
//! Layering: every widget is a sibling of the editor's own element, not a
//! child of its key-context div. So the editor's action listeners never see
//! keys typed into the link field, and a widget that takes a click is
//! `occlude`d so the editor under it does not also move the caret.
//!
//! Geometry comes from the rows of the last paint (`last_rows`). A widget is
//! therefore placed from the layout one frame behind an edit, and hidden when
//! its anchor line is not on screen.

use super::*;

use gpui::{AnyElement, ClickEvent, Div, SharedString, Stateful, Subscription};

use crate::editor::Editor;
use crate::rich_model::Inline;

// ---------------------------------------------------------------- metrics

const TB_W: f32 = 404.0;
const TB_H: f32 = 32.0;
/// Where the heading button's centre sits, from the bar's left edge.
const HEADING_BTN_CENTRE: f32 = 222.0;
const DD_W: f32 = 150.0;
const DD_ROW: f32 = 28.0;
const SLASH_W: f32 = 268.0;
const SLASH_ROW: f32 = 30.0;
/// Rows the slash menu shows at once; the list scrolls to keep the selection.
const SLASH_VISIBLE: usize = 8;
const LINK_W: f32 = 340.0;
const LINK_H: f32 = 156.0;
const TIP_W: f32 = 380.0;
const TIP_H: f32 = 66.0;
const POPUP_GAP: f32 = 6.0;
const POPUP_MARGIN: f32 = 6.0;
/// Longest query the slash menu keeps open for.
const SLASH_QUERY_MAX: usize = 40;

// ---------------------------------------------------------------- slash menu

/// What a slash item does to the line it was typed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlashAction {
    Paragraph,
    Heading(u8),
    List(ListType),
    Quote,
    CodeBlock,
    Table,
    Rule,
    Image,
    Callout(CalloutKind),
    Chart,
}

pub struct SlashItem {
    pub label: &'static str,
    /// Extra words the query also matches, space separated.
    pub keys: &'static str,
    pub action: SlashAction,
}

pub const SLASH_ITEMS: [SlashItem; 18] = [
    SlashItem { label: "Text", keys: "text paragraph plain normal p", action: SlashAction::Paragraph },
    SlashItem { label: "Heading 1", keys: "heading h1 title big", action: SlashAction::Heading(1) },
    SlashItem { label: "Heading 2", keys: "heading h2 subtitle", action: SlashAction::Heading(2) },
    SlashItem { label: "Heading 3", keys: "heading h3", action: SlashAction::Heading(3) },
    SlashItem { label: "Bullet list", keys: "bullet list ul unordered dash", action: SlashAction::List(ListType::Bullet) },
    SlashItem { label: "Ordered list", keys: "ordered numbered list ol number", action: SlashAction::List(ListType::Ordered) },
    SlashItem { label: "Task list", keys: "task todo checkbox check list", action: SlashAction::List(ListType::Task) },
    SlashItem { label: "Quote", keys: "quote blockquote citation", action: SlashAction::Quote },
    SlashItem { label: "Code block", keys: "code block fence snippet pre", action: SlashAction::CodeBlock },
    SlashItem { label: "Table", keys: "table grid 3x3", action: SlashAction::Table },
    SlashItem { label: "Divider", keys: "divider rule hr line separator ---", action: SlashAction::Rule },
    SlashItem { label: "Image", keys: "image img picture photo", action: SlashAction::Image },
    SlashItem { label: "Callout: Note", keys: "callout note alert admonition info", action: SlashAction::Callout(CalloutKind::Note) },
    SlashItem { label: "Callout: Tip", keys: "callout tip hint", action: SlashAction::Callout(CalloutKind::Tip) },
    SlashItem { label: "Callout: Important", keys: "callout important", action: SlashAction::Callout(CalloutKind::Important) },
    SlashItem { label: "Callout: Warning", keys: "callout warning caution alert", action: SlashAction::Callout(CalloutKind::Warning) },
    SlashItem { label: "Callout: Caution", keys: "callout caution danger", action: SlashAction::Callout(CalloutKind::Caution) },
    SlashItem { label: "Chart", keys: "chart graph bar line pie data", action: SlashAction::Chart },
];

/// Indices into `SLASH_ITEMS` that match `query`, best first. An empty query
/// matches everything in list order. Every whitespace-separated term must
/// match; a term scores by how well it matches (label prefix, label word,
/// keyword word, substring), and the item's score is its worst term.
pub fn rank_slash(query: &str) -> Vec<usize> {
    let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if terms.is_empty() {
        return (0..SLASH_ITEMS.len()).collect();
    }
    let mut scored: Vec<(u8, usize)> = Vec::new();
    for (i, item) in SLASH_ITEMS.iter().enumerate() {
        let label = item.label.to_lowercase();
        let keys = item.keys.to_lowercase();
        let mut worst = 0u8;
        let mut all = true;
        for term in &terms {
            match term_score(&label, &keys, term) {
                Some(score) => worst = worst.max(score),
                None => {
                    all = false;
                    break;
                }
            }
        }
        if all {
            scored.push((worst, i));
        }
    }
    scored.sort();
    scored.into_iter().map(|(_, i)| i).collect()
}

fn term_score(label: &str, keys: &str, term: &str) -> Option<u8> {
    let word_starts = |s: &str| {
        s.split(|c: char| !c.is_alphanumeric())
            .any(|w| !w.is_empty() && w.starts_with(term))
    };
    if label.starts_with(term) {
        Some(0)
    } else if word_starts(label) {
        Some(1)
    } else if word_starts(keys) {
        Some(2)
    } else if label.contains(term) || keys.contains(term) {
        Some(3)
    } else {
        None
    }
}

/// Whether a line of this kind may open the slash menu.
fn slash_allowed(kind: &LineKind) -> bool {
    matches!(
        kind,
        LineKind::Blank
            | LineKind::Paragraph
            | LineKind::Heading(_)
            | LineKind::Quote { .. }
            | LineKind::ListItem { .. }
            | LineKind::ListContinuation { .. }
    )
}

/// The offset of the `/` that the caret is inside of, when the text from it to
/// the caret is a slash query: the `/` starts a word (line start, after a block
/// marker, or after whitespace), the query has no second `/`, and the line is
/// prose rather than code, a table, a rule or HTML.
pub fn slash_span(model: &Model, text: &str, caret: usize) -> Option<usize> {
    if model.lines.is_empty() || caret > text.len() || !text.is_char_boundary(caret) {
        return None;
    }
    let line = &model.lines[model.line_index_at(caret)];
    if !slash_allowed(&line.kind) {
        return None;
    }
    let head = &text[line.range.start..caret];
    let p = line.range.start + head.rfind('/')?;
    if caret - p - 1 > SLASH_QUERY_MAX {
        return None;
    }
    let at_word_start = p == line.range.start
        || p == line.content_start
        || text[line.range.start..p]
            .chars()
            .next_back()
            .is_some_and(char::is_whitespace);
    at_word_start.then_some(p)
}

/// The query typed after the slash starting at `start`.
pub fn slash_query(text: &str, start: usize, caret: usize) -> &str {
    text.get(start + 1..caret).unwrap_or("")
}

/// The whole document after running `action` on the line with `/query` at
/// `start..caret`. The query is removed first, so the result is one edit and
/// one undo step.
pub fn slash_edit(text: &str, start: usize, caret: usize, action: SlashAction) -> Edit {
    let caret = caret.clamp(start, text.len());
    let stripped = format!("{}{}", &text[..start], &text[caret..]);
    let s = start;
    let op: Edit = match action {
        SlashAction::Paragraph => rich_ops::set_paragraph(&stripped, s..s),
        SlashAction::Heading(n) => rich_ops::set_heading(&stripped, s..s, Some(n)),
        SlashAction::List(kind) => rich_ops::toggle_list(&stripped, s..s, kind),
        SlashAction::Quote => rich_ops::toggle_quote(&stripped, s..s),
        SlashAction::CodeBlock => rich_ops::insert_code_block(&stripped, s..s, ""),
        SlashAction::Table => rich_ops::insert_table(&stripped, s..s, 3, 3),
        SlashAction::Rule => rich_ops::insert_rule(&stripped, s..s),
        SlashAction::Image => {
            // `url` is selected so typing replaces it.
            let body = "![image](url)";
            let e = rich_ops::block_edit(&stripped, s, s, body, 9);
            let c = e.selection.start;
            Some(Edit {
                selection: c..c + 3,
                ..e
            })
        }
        SlashAction::Callout(kind) => {
            let body = format!("> [!{}]\n> ", callout_name(kind));
            let n = body.len();
            Some(rich_ops::block_edit(&stripped, s, s, &body, n))
        }
        SlashAction::Chart => {
            let head = "```chart\n";
            let body = format!(
                "{head}{{\n  \"type\": \"bar\",\n  \"data\": {{\n    \"labels\": [\"A\", \"B\", \"C\"],\n    \"datasets\": [{{ \"label\": \"Series 1\", \"data\": [3, 5, 2] }}]\n  }}\n}}\n```"
            );
            Some(rich_ops::block_edit(&stripped, s, s, &body, head.len()))
        }
    }
    .unwrap_or(Edit {
        range: s..s,
        insert: String::new(),
        selection: s..s,
    });
    Edit {
        range: 0..text.len(),
        insert: op.apply(&stripped),
        selection: op.selection,
    }
}

fn callout_name(kind: CalloutKind) -> &'static str {
    match kind {
        CalloutKind::Note => "NOTE",
        CalloutKind::Tip => "TIP",
        CalloutKind::Important => "IMPORTANT",
        CalloutKind::Warning => "WARNING",
        CalloutKind::Caution => "CAUTION",
    }
}

// ---------------------------------------------------------------- marks

/// What the selection's formatting is, for the toolbar's active states.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Marks {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    pub link: bool,
    /// `Some(n)` when every non-blank touched line is heading level `n`.
    pub heading: Option<u8>,
    /// `Some(kind)` when every non-blank touched line is that list kind.
    pub list: Option<ListType>,
    /// Every non-blank touched line is a quote.
    pub quote: bool,
}

/// The formatting that covers the whole selection. An inline counts when one
/// of its spans contains the selection; block state needs every touched line
/// to agree.
pub fn marks_at(model: &Model, sel: Range<usize>) -> Marks {
    let (a, z) = (sel.start.min(sel.end), sel.start.max(sel.end));
    let mut m = Marks::default();
    if model.lines.is_empty() {
        return m;
    }
    let first = model.line_index_at(a);
    let last = model.line_index_at(z).min(model.lines.len() - 1);
    let lines = &model.lines[first..=last];
    for line in lines {
        for inl in &line.inlines {
            if inl.range.start <= a && z <= inl.range.end {
                match inl.kind {
                    InlineKind::Strong => m.bold = true,
                    InlineKind::Emphasis => m.italic = true,
                    InlineKind::Strike => m.strike = true,
                    InlineKind::Code => m.code = true,
                    InlineKind::Link { .. } | InlineKind::AutoLink => m.link = true,
                    _ => {}
                }
            }
        }
    }
    let content: Vec<&LineKind> = lines
        .iter()
        .map(|l| &l.kind)
        .filter(|k| **k != LineKind::Blank)
        .collect();
    if content.is_empty() {
        return m;
    }
    m.heading = match content[0] {
        LineKind::Heading(n) if content.iter().all(|k| **k == LineKind::Heading(*n)) => Some(*n),
        _ => None,
    };
    m.quote = content
        .iter()
        .all(|k| matches!(k, LineKind::Quote { callout: None, .. }));
    let item_kind = |k: &LineKind| match k {
        LineKind::ListItem { ordered, task, .. } => Some(match (task, ordered) {
            (Some(_), _) => ListType::Task,
            (None, true) => ListType::Ordered,
            (None, false) => ListType::Bullet,
        }),
        _ => None,
    };
    if let Some(first) = item_kind(content[0]) {
        if content.iter().all(|k| item_kind(k) == Some(first)) {
            m.list = Some(first);
        }
    }
    m
}

/// Whether the selection touches a code block, where the toolbar is hidden.
pub fn selection_in_code(model: &Model, sel: Range<usize>) -> bool {
    if model.lines.is_empty() {
        return false;
    }
    let (a, z) = (sel.start.min(sel.end), sel.start.max(sel.end));
    let first = model.line_index_at(a);
    let last = model.line_index_at(z).min(model.lines.len() - 1);
    model.lines[first..=last].iter().any(|l| {
        matches!(
            l.kind,
            LineKind::FenceOpen { .. } | LineKind::CodeLine | LineKind::FenceClose | LineKind::IndentedCode
        )
    })
}

// ---------------------------------------------------------------- links

/// The link (or autolink) that the offset is inside of. Offsets on its outer
/// edges do not count, so a caret just before or after a link is not "in" it.
pub fn link_inline_at(model: &Model, offset: usize) -> Option<Inline> {
    if model.lines.is_empty() {
        return None;
    }
    let line = &model.lines[model.line_index_at(offset)];
    line.inlines
        .iter()
        .find(|inl| {
            matches!(inl.kind, InlineKind::Link { .. } | InlineKind::AutoLink)
                && inl.range.start < offset
                && offset < inl.range.end
        })
        .cloned()
}

/// The destination of a link, as text.
pub fn link_target_text(text: &str, inl: &Inline) -> String {
    match &inl.kind {
        InlineKind::Link { url } => text.get(url.clone()).unwrap_or("").to_string(),
        _ => text
            .get(inl.open.end..inl.close.start)
            .unwrap_or("")
            .to_string(),
    }
}

/// Markdown link destination: wrapped in `<>` when it holds spaces or parens.
fn link_dest(url: &str) -> String {
    if url.chars().any(|c| c.is_whitespace() || c == '(' || c == ')') {
        format!("<{url}>")
    } else {
        url.to_string()
    }
}

/// Replaces an existing link's destination, keeping its label source.
pub fn link_set_edit(text: &str, inl: &Inline, url: &str) -> Edit {
    let label = text.get(inl.open.end..inl.close.start).unwrap_or("");
    let new = format!("[{}]({})", label, link_dest(url));
    let c = inl.range.start + new.len();
    Edit {
        range: inl.range.clone(),
        insert: new,
        selection: c..c,
    }
}

/// Turns a link back into its plain label, with the label selected.
pub fn link_unwrap_edit(text: &str, inl: &Inline) -> Edit {
    let label = text.get(inl.open.end..inl.close.start).unwrap_or("").to_string();
    let s = inl.range.start;
    let e = s + label.len();
    Edit {
        range: inl.range.clone(),
        insert: label,
        selection: s..e,
    }
}

// ---------------------------------------------------------------- placement

/// Where a popup of `size` goes beside `anchor`: centred on it horizontally and
/// clamped inside the view. Vertically it goes above when `prefer_above` and
/// there is room, else below if there is room, else the side with more room,
/// clamped on screen.
pub fn place_popup(anchor: Rect, size: (f32, f32), view: (f32, f32), prefer_above: bool) -> (f32, f32) {
    let (w, h) = size;
    let (vw, vh) = view;
    let centre = anchor.x + anchor.w / 2.0;
    let x = (centre - w / 2.0).clamp(POPUP_MARGIN, (vw - w - POPUP_MARGIN).max(POPUP_MARGIN));
    let above = anchor.y - POPUP_GAP - h;
    let below = anchor.y + anchor.h + POPUP_GAP;
    let fits_above = above >= POPUP_MARGIN;
    let fits_below = below + h <= vh - POPUP_MARGIN;
    let y = if prefer_above {
        if fits_above {
            above
        } else if fits_below {
            below
        } else {
            above.max(POPUP_MARGIN)
        }
    } else if fits_below {
        below
    } else if fits_above {
        above
    } else {
        (vh - POPUP_MARGIN - h).max(POPUP_MARGIN)
    };
    (x, y)
}

// ---------------------------------------------------------------- state

/// An open slash menu: the `/` offset and the highlighted row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SlashMenu {
    pub start: usize,
    pub selected: usize,
}

/// An open link popover. `target` is the link being edited, or `None` for a
/// new link over `sel`.
#[derive(Clone, Debug)]
pub(super) struct LinkPopover {
    pub target: Option<Inline>,
    pub sel: Range<usize>,
}

/// Everything the overlays keep between frames.
pub struct Overlays {
    pub(super) slash: Option<SlashMenu>,
    /// Caret offset right after a typed `/`, consumed by the next sync.
    pub(super) slash_pending: Option<usize>,
    /// Selection the toolbar and link tooltip were dismissed at. They come back
    /// when the selection changes.
    pub(super) dismissed: Option<Range<usize>>,
    /// The selection the heading dropdown was opened on.
    pub(super) dropdown: Option<Range<usize>>,
    pub(super) link: Option<LinkPopover>,
    pub(super) link_input: Entity<Editor>,
    /// The URL field's text, mirrored through its `on_change`.
    pub(super) link_url: String,
    /// Give keyboard focus back to the editor on the next render.
    pub(super) refocus: bool,
    pub(super) blur_sub: Option<Subscription>,
}

impl Overlays {
    pub(super) fn new(cx: &mut Context<RichEditor>) -> Self {
        let weak = cx.entity().downgrade();
        let link_input = cx.new(|cx| {
            Editor::new(true, cx)
                .with_placeholder("https://example.com")
                // The field box supplies the horizontal inset; the text needs
                // only a little top room inside its 28px interior.
                .with_padding(0.0, 4.0)
        });
        link_input.update(cx, |e, _| {
            let w = weak.clone();
            e.on_submit = Some(Box::new(move |cx: &mut App| {
                let _ = w.update(cx, |this, cx| this.link_submit(cx));
            }));
            let w = weak.clone();
            e.on_cancel = Some(Box::new(move |cx: &mut App| {
                let _ = w.update(cx, |this, cx| this.link_cancel(cx));
            }));
            let w = weak.clone();
            e.on_change = Some(Box::new(move |text: &str, cx: &mut App| {
                let _ = w.update(cx, |this, _| this.overlays.link_url = text.to_string());
            }));
        });
        Self {
            slash: None,
            slash_pending: None,
            dismissed: None,
            dropdown: None,
            link: None,
            link_input,
            link_url: String::new(),
            refocus: false,
            blur_sub: None,
        }
    }
}

/// Keys the overlays react to. Other keys go to the editor unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OverlayKey {
    Up,
    Down,
    Enter,
    Tab,
    Escape,
}

/// The clickable commands of the overlays. Each is also reachable by name
/// through `RichEditor::overlay_action` (the debug script uses that).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayCmd {
    Bold,
    Italic,
    Strike,
    Code,
    Link,
    Dropdown,
    /// 0 is a paragraph.
    Heading(u8),
    Bullet,
    Ordered,
    Task,
    Quote,
    SlashItem(usize),
    LinkApply,
    LinkCancel,
    Open,
    Edit,
    Unlink,
}

impl OverlayCmd {
    /// The name `rich-action` uses in a script.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "bold" => Self::Bold,
            "italic" => Self::Italic,
            "strike" => Self::Strike,
            "code" => Self::Code,
            "link" => Self::Link,
            "heading-menu" => Self::Dropdown,
            "paragraph" => Self::Heading(0),
            "h1" => Self::Heading(1),
            "h2" => Self::Heading(2),
            "h3" => Self::Heading(3),
            "bullet" => Self::Bullet,
            "ordered" => Self::Ordered,
            "task" => Self::Task,
            "quote" => Self::Quote,
            "link-apply" => Self::LinkApply,
            "link-cancel" => Self::LinkCancel,
            "open-link" => Self::Open,
            "edit-link" => Self::Edit,
            "unlink" => Self::Unlink,
            other => {
                let n = other.strip_prefix("slash-")?.parse::<usize>().ok()?;
                if n >= SLASH_ITEMS.len() {
                    return None;
                }
                Self::SlashItem(n)
            }
        })
    }
}

// ---------------------------------------------------------------- the editor

impl RichEditor {
    /// The caret rectangle of `offset`, element-local, from the last paint.
    pub(super) fn caret_rect(&self, offset: usize, at_row_end: bool) -> Option<Rect> {
        let line = self.line_of(offset);
        let row = self.last_rows.iter().find(|r| r.index == line)?;
        let idx = rich_model::source_to_display(&row.disp, offset);
        let at = row.para.position_for_index(idx, at_row_end);
        Some(Rect {
            x: row.text_x + f32::from(at.x),
            y: row.text_top + f32::from(at.y),
            w: 0.0,
            h: f32::from(row.para.line_height()),
        })
    }

    /// The area the toolbar sits above: from the selection's first caret to
    /// the bottom of its last. `None` when the start is off screen.
    fn selection_anchor(&self) -> Option<Rect> {
        let s = self.selection.clone();
        let start = self.caret_rect(s.start, false)?;
        let end = self.caret_rect(s.end, false).unwrap_or(start);
        let same_row = (end.y - start.y).abs() < 1.0;
        Some(Rect {
            x: if same_row { (start.x + end.x) / 2.0 } else { start.x },
            y: start.y,
            w: 0.0,
            h: end.y + end.h - start.y,
        })
    }

    /// Whether the link tooltip is shown for the caret: a caret inside a link,
    /// with no other popup open and not dismissed at this selection.
    fn tooltip_target(&self) -> Option<Inline> {
        if !self.selection.is_empty()
            || self.overlays.link.is_some()
            || self.overlays.slash.is_some()
            || self.overlays.dismissed.as_ref() == Some(&self.selection)
        {
            return None;
        }
        link_inline_at(&self.model, self.cursor())
    }

    fn toolbar_shown(&self) -> bool {
        !self.selection.is_empty()
            && !self.selecting
            && self.marked.is_none()
            && self.overlays.link.is_none()
            && self.overlays.slash.is_none()
            && self.overlays.dismissed.as_ref() != Some(&self.selection)
            && !selection_in_code(&self.model, self.selection.clone())
    }

    /// The slash items matching the query typed so far.
    pub(super) fn slash_ranked(&self) -> Vec<usize> {
        match self.overlays.slash {
            Some(menu) => {
                let query = slash_query(&self.content, menu.start, self.cursor());
                rank_slash(query)
            }
            None => Vec::new(),
        }
    }

    /// Closes every overlay. `refocus` returns keyboard focus to the editor,
    /// used when the user closed the popover; a blur must not steal focus back.
    fn overlays_close(&mut self, refocus: bool) {
        self.overlays.slash = None;
        self.overlays.dropdown = None;
        if self.overlays.link.take().is_some() && refocus {
            self.overlays.refocus = true;
        }
    }

    /// Called from `commit` after every edit. A typed `/` arms the slash menu;
    /// the next render decides whether it opens.
    pub(super) fn overlay_after_edit(&mut self, edit: &Edit, typing: bool) {
        if typing && edit.range.is_empty() && edit.insert == "/" {
            self.overlays.slash_pending = Some(edit.selection.end);
        }
    }

    /// Scrolling closes the popovers and hides the toolbar and tooltip until
    /// the selection changes.
    pub(super) fn overlay_on_scroll(&mut self) {
        let had_link = self.overlays.link.is_some();
        self.overlays_close(had_link);
        self.overlays.dismissed = Some(self.selection.clone());
    }

    /// Handles a key the overlays claim. Returns `true` when it was consumed.
    pub(super) fn overlay_key(&mut self, key: OverlayKey, cx: &mut Context<Self>) -> bool {
        if let Some(menu) = self.overlays.slash {
            let ranked = self.slash_ranked();
            let n = ranked.len();
            match key {
                OverlayKey::Up | OverlayKey::Down if n > 0 => {
                    let next = if key == OverlayKey::Up {
                        (menu.selected + n - 1) % n
                    } else {
                        (menu.selected + 1) % n
                    };
                    self.overlays.slash = Some(SlashMenu { selected: next, ..menu });
                    cx.notify();
                    return true;
                }
                OverlayKey::Enter | OverlayKey::Tab if n > 0 => {
                    let item = ranked[menu.selected.min(n - 1)];
                    self.slash_apply(item, cx);
                    return true;
                }
                OverlayKey::Escape => {
                    self.overlays.slash = None;
                    cx.notify();
                    return true;
                }
                _ => return false,
            }
        }
        if key == OverlayKey::Escape {
            if self.overlays.dropdown.is_some() {
                self.overlays.dropdown = None;
                cx.notify();
                return true;
            }
            if self.toolbar_shown() {
                self.overlays.dismissed = Some(self.selection.clone());
                cx.notify();
                return true;
            }
        }
        false
    }

    /// Applies slash item `index` in place of the `/query` text, as one edit.
    fn slash_apply(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(menu) = self.overlays.slash.take() else {
            return;
        };
        let Some(item) = SLASH_ITEMS.get(index) else {
            return;
        };
        let edit = slash_edit(&self.content, menu.start, self.cursor(), item.action);
        self.apply_edit(edit, cx);
    }

    /// Runs the slash-menu or popover logic that depends on the state after an
    /// edit or move: opens, closes or re-validates the slash menu.
    fn overlay_sync_slash(&mut self) {
        let cursor = self.cursor();
        if let Some(menu) = self.overlays.slash {
            let valid = self.marked.is_none()
                && self.selection.is_empty()
                && slash_span(&self.model, &self.content, cursor) == Some(menu.start);
            if !valid {
                self.overlays.slash = None;
            } else {
                let query = slash_query(&self.content, menu.start, cursor);
                let n = rank_slash(query).len();
                if n == 0 && query.ends_with(' ') {
                    self.overlays.slash = None;
                } else {
                    let selected = menu.selected.min(n.saturating_sub(1));
                    self.overlays.slash = Some(SlashMenu { selected, ..menu });
                }
            }
        }
        if let Some(pending) = self.overlays.slash_pending.take() {
            if self.overlays.slash.is_none()
                && pending == cursor
                && self.selection.is_empty()
                && self.marked.is_none()
                && self.overlays.link.is_none()
            {
                if let Some(start) = slash_span(&self.model, &self.content, cursor) {
                    self.overlays.slash = Some(SlashMenu { start, selected: 0 });
                }
            }
        }
    }

    /// Per-render bookkeeping: focus-loss handling, the refocus request, and
    /// the slash menu state. Runs before the frame is built, so it may change
    /// state but not notify.
    pub(super) fn overlay_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlays.blur_sub.is_none() {
            let sub = cx.on_focus_out(&self.focus, window, |_this, _event, _window, cx| {
                cx.notify();
            });
            self.overlays.blur_sub = Some(sub);
        }
        if std::mem::take(&mut self.overlays.refocus) {
            window.focus(&self.focus);
        }
        let link_focused = self
            .overlays
            .link_input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        if !self.focus.is_focused(window) && !link_focused {
            self.overlays_close(false);
            self.overlays.dismissed = None;
        }
        if self
            .overlays
            .dropdown
            .as_ref()
            .is_some_and(|sel| *sel != self.selection)
        {
            self.overlays.dropdown = None;
        }
        self.overlay_sync_slash();
    }

    /// Runs one overlay command: a toolbar click, a popover button, or a
    /// named command from the debug script.
    pub(super) fn overlay_command(&mut self, cmd: OverlayCmd, window: &mut Window, cx: &mut Context<Self>) {
        match cmd {
            OverlayCmd::Bold => self.toggle_bold(cx),
            OverlayCmd::Italic => self.toggle_italic(cx),
            OverlayCmd::Strike => self.toggle_strike(cx),
            OverlayCmd::Code => self.toggle_code(cx),
            OverlayCmd::Link => self.open_link_popover(window, cx),
            OverlayCmd::Dropdown => {
                self.overlays.dropdown = match self.overlays.dropdown {
                    Some(_) => None,
                    None => Some(self.selection.clone()),
                };
                cx.notify();
            }
            OverlayCmd::Heading(level) => {
                self.overlays.dropdown = None;
                self.set_heading(level, cx);
            }
            OverlayCmd::Bullet => self.toggle_list(ListType::Bullet, cx),
            OverlayCmd::Ordered => self.toggle_list(ListType::Ordered, cx),
            OverlayCmd::Task => self.toggle_list(ListType::Task, cx),
            OverlayCmd::Quote => self.toggle_quote(cx),
            OverlayCmd::SlashItem(index) => self.slash_apply(index, cx),
            OverlayCmd::LinkApply => self.link_submit(cx),
            OverlayCmd::LinkCancel => self.link_cancel(cx),
            OverlayCmd::Open => {
                if let Some(inl) = link_inline_at(&self.model, self.cursor()) {
                    let url = link_target_text(&self.content, &inl);
                    cx.open_url(&url);
                }
            }
            OverlayCmd::Edit => self.open_link_popover(window, cx),
            OverlayCmd::Unlink => {
                if let Some(inl) = link_inline_at(&self.model, self.cursor()) {
                    let edit = link_unwrap_edit(&self.content, &inl);
                    self.apply_edit(edit, cx);
                }
            }
        }
        cx.notify();
    }

    /// Runs the named command. `false` for an unknown name.
    pub(crate) fn overlay_action(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match OverlayCmd::from_name(name) {
            Some(cmd) => {
                self.overlay_command(cmd, window, cx);
                true
            }
            None => false,
        }
    }

    /// Cmd-K: opens the URL popover, prefilled when the caret is in a link.
    pub(super) fn open_link_popover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = if self.selection.is_empty() {
            link_inline_at(&self.model, self.cursor())
        } else {
            None
        };
        let url = target
            .as_ref()
            .map(|inl| link_target_text(&self.content, inl))
            .unwrap_or_default();
        self.overlays.dropdown = None;
        self.overlays.slash = None;
        self.overlays.link_url = url.clone();
        self.overlays.link = Some(LinkPopover {
            target,
            sel: self.selection.clone(),
        });
        let theme_name = self.theme_name.clone();
        let input = self.overlays.link_input.clone();
        input.update(cx, |e, cx| {
            // `set_text` reports a change; the mirror is already set above, so
            // keep the hook out of this one update (the editor is leased here).
            let hook = e.on_change.take();
            e.set_theme(&theme_name, cx);
            e.set_text(&url, cx);
            e.on_change = hook;
            let len = e.text().len();
            e.select(0..len, cx);
        });
        let handle = input.read(cx).focus_handle(cx);
        window.focus(&handle);
        cx.notify();
    }

    /// Enter in the URL field: applies the URL, or removes the link when the
    /// field is empty. Runs without a window, from the field's callback.
    pub(super) fn link_submit(&mut self, cx: &mut Context<Self>) {
        let Some(pop) = self.overlays.link.take() else {
            return;
        };
        self.overlays.refocus = true;
        let url = self.overlays.link_url.trim().to_string();
        let edit = match (&pop.target, url.is_empty()) {
            (Some(inl), true) => Some(link_unwrap_edit(&self.content, inl)),
            (Some(inl), false) => Some(link_set_edit(&self.content, inl, &url)),
            (None, true) => None,
            (None, false) => rich_ops::insert_link(&self.content, pop.sel.clone(), Some(&url)),
        };
        match edit {
            Some(edit) => self.apply_edit(edit, cx),
            None => cx.notify(),
        }
    }

    /// Escape in the URL field, or the Cancel button.
    pub(super) fn link_cancel(&mut self, cx: &mut Context<Self>) {
        if self.overlays.link.take().is_some() {
            self.overlays.refocus = true;
        }
        cx.notify();
    }

    // ---- drawing -------------------------------------------------------

    /// Wraps the editor in a container with the overlays as siblings of it.
    pub(super) fn overlay_wrap(&mut self, root: Div, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let layers = self.overlay_layers(window, cx);
        div().relative().size_full().child(root).children(layers)
    }

    /// Every visible overlay, back to front.
    fn overlay_layers(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let t = Theme::for_name(&self.theme_name, window.appearance());
        let view = (self.view_width, self.view_height);
        let mut out = Vec::new();

        if self.content.is_empty() && self.marked.is_none() {
            let col_w = (self.view_width - 2.0 * SIDE_PAD).clamp(120.0, COLUMN_MAX);
            let col_left = ((self.view_width - col_w) / 2.0).max(0.0);
            out.push(
                div()
                    .absolute()
                    .left(px(col_left))
                    .top(px(TOP_PAD))
                    .text_size(px(BODY_SIZE))
                    .line_height(px(BODY_LH))
                    .text_color(t.muted)
                    .child("Start writing…")
                    .into_any_element(),
            );
        }

        if self.toolbar_shown() {
            if let Some(anchor) = self.selection_anchor() {
                let (x, y) = place_popup(anchor, (TB_W, TB_H), view, true);
                let dropdown_open = self.overlays.dropdown.is_some();
                let marks = marks_at(&self.model, self.selection.clone());
                out.push(self.toolbar_el(&t, (x, y), marks, cx));
                if dropdown_open {
                    // Under the bar, aligned near the heading button; flips
                    // above it when there is no room below.
                    let dd_h = 4.0 * DD_ROW + 8.0;
                    let bar = Rect { x: x + HEADING_BTN_CENTRE, y, w: 0.0, h: TB_H };
                    let (dx, dy) = place_popup(bar, (DD_W, dd_h), view, false);
                    out.push(self.heading_menu_el(&t, (dx, dy), marks.heading, cx));
                }
            }
        }

        if self.overlays.slash.is_some() {
            if let Some(anchor) = self.caret_rect(self.cursor(), self.at_row_end) {
                let ranked = self.slash_ranked();
                let rows = ranked.len().clamp(1, SLASH_VISIBLE);
                let h = rows as f32 * SLASH_ROW + 12.0;
                let (x, y) = place_popup(anchor, (SLASH_W, h), view, false);
                out.push(self.slash_el(&t, (x, y), &ranked, cx));
            }
        }

        if let Some(pop) = self.overlays.link.clone() {
            if let Some(anchor) = self.caret_rect(self.cursor(), self.at_row_end) {
                let (x, y) = place_popup(anchor, (LINK_W, LINK_H), view, false);
                out.push(self.link_el(&t, (x, y), pop.target.is_some(), cx));
            }
        } else if let Some(inl) = self.tooltip_target() {
            if let Some(anchor) = self.caret_rect(self.cursor(), self.at_row_end) {
                let url = link_target_text(&self.content, &inl);
                let (x, y) = place_popup(anchor, (TIP_W, TIP_H), view, false);
                out.push(self.tooltip_el(&t, (x, y), url, cx));
            }
        }
        out
    }

    fn toolbar_el(&self, t: &Theme, at: (f32, f32), marks: Marks, cx: &mut Context<Self>) -> AnyElement {
        let heading_label = match marks.heading {
            Some(n) if n > 0 => format!("H{n} ▾"),
            _ => "Text ▾".to_string(),
        };
        let heading_active = marks.heading.is_some_and(|n| n > 0);
        let list = |k: ListType| marks.list == Some(k);
        div()
            .absolute()
            .left(px(at.0))
            .top(px(at.1))
            .w(px(TB_W))
            .h(px(TB_H))
            .px_1()
            .flex()
            .flex_row()
            .items_center()
            .gap_0p5()
            .rounded(px(8.0))
            .bg(t.surface)
            .border_1()
            .border_color(t.border)
            .occlude()
            .child(button(OverlayCmd::Bold, "B", marks.bold, 26.0, t, cx))
            .child(button(OverlayCmd::Italic, "I", marks.italic, 26.0, t, cx))
            .child(button(OverlayCmd::Strike, "S", marks.strike, 26.0, t, cx))
            .child(button(OverlayCmd::Code, "</>", marks.code, 34.0, t, cx))
            .child(divider(t))
            .child(button(OverlayCmd::Link, "Link", marks.link, 40.0, t, cx))
            .child(divider(t))
            .child(button(OverlayCmd::Dropdown, &heading_label, heading_active, 64.0, t, cx))
            .child(divider(t))
            .child(button(OverlayCmd::Bullet, "•", list(ListType::Bullet), 26.0, t, cx))
            .child(button(OverlayCmd::Ordered, "1.", list(ListType::Ordered), 26.0, t, cx))
            .child(button(OverlayCmd::Task, "☑", list(ListType::Task), 26.0, t, cx))
            .child(button(OverlayCmd::Quote, "❝", marks.quote, 26.0, t, cx))
            .into_any_element()
    }

    fn heading_menu_el(&self, t: &Theme, at: (f32, f32), current: Option<u8>, cx: &mut Context<Self>) -> AnyElement {
        let rows = [("Paragraph", 0u8), ("Heading 1", 1), ("Heading 2", 2), ("Heading 3", 3)];
        let mut panel = panel(t).left(px(at.0)).top(px(at.1)).w(px(DD_W)).p_1();
        for (label, level) in rows {
            let active = current.unwrap_or(0) == level;
            panel = panel.child(
                menu_row(
                    format!("rich-ov-heading-{level}"),
                    label,
                    active,
                    t,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.overlay_command(OverlayCmd::Heading(level), window, cx)
                })),
            );
        }
        panel.into_any_element()
    }

    fn slash_el(&self, t: &Theme, at: (f32, f32), ranked: &[usize], cx: &mut Context<Self>) -> AnyElement {
        let selected = self.overlays.slash.map(|m| m.selected).unwrap_or(0);
        let first = if selected >= SLASH_VISIBLE {
            selected + 1 - SLASH_VISIBLE
        } else {
            0
        };
        let mut panel = panel(t).left(px(at.0)).top(px(at.1)).w(px(SLASH_W)).p_1();
        if ranked.is_empty() {
            panel = panel.child(
                div()
                    .h(px(SLASH_ROW))
                    .px_2()
                    .flex()
                    .items_center()
                    .text_size(px(12.0))
                    .text_color(t.muted)
                    .child("No matching blocks"),
            );
        }
        for (pos, &index) in ranked.iter().enumerate().skip(first).take(SLASH_VISIBLE) {
            let item = &SLASH_ITEMS[index];
            let active = pos == selected;
            panel = panel.child(
                menu_row(format!("rich-slash-{index}"), item.label, active, t)
                    .h(px(SLASH_ROW))
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if *hovered {
                            if let Some(menu) = this.overlays.slash {
                                if menu.selected != pos {
                                    this.overlays.slash = Some(SlashMenu { selected: pos, ..menu });
                                    cx.notify();
                                }
                            }
                        }
                    }))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.overlay_command(OverlayCmd::SlashItem(index), window, cx)
                    })),
            );
        }
        panel.into_any_element()
    }

    fn link_el(&self, t: &Theme, at: (f32, f32), editing: bool, cx: &mut Context<Self>) -> AnyElement {
        let field = div()
            .h(px(30.0))
            .px_2()
            .flex()
            .items_center()
            .rounded(px(6.0))
            .border_1()
            .border_color(t.border)
            .bg(t.bg)
            .overflow_hidden()
            .child(self.overlays.link_input.clone());
        panel(t)
            .left(px(at.0))
            .top(px(at.1))
            .w(px(LINK_W))
            .p_3()
            .gap_2()
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(t.muted)
                    .child(if editing { "Edit link" } else { "Insert link" }),
            )
            .child(field)
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(t.muted)
                    .child("Enter applies · Esc cancels · empty removes the link"),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_end()
                    .gap_2()
                    .child(button(OverlayCmd::LinkCancel, "Cancel", false, 72.0, t, cx))
                    .child(button(OverlayCmd::LinkApply, "Apply", true, 72.0, t, cx)),
            )
            .into_any_element()
    }

    fn tooltip_el(&self, t: &Theme, at: (f32, f32), url: String, cx: &mut Context<Self>) -> AnyElement {
        panel(t)
            .left(px(at.0))
            .top(px(at.1))
            .w(px(TIP_W))
            .p_2()
            .gap_1()
            .child(
                div()
                    .w_full()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(12.0))
                    .text_color(t.accent)
                    .child(url),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_1()
                    .child(button(OverlayCmd::Open, "Open", false, 56.0, t, cx))
                    .child(button(OverlayCmd::Edit, "Edit", false, 56.0, t, cx))
                    .child(button(OverlayCmd::Unlink, "Remove", false, 72.0, t, cx)),
            )
            .into_any_element()
    }
}

/// A popup panel. The caller adds its position.
fn panel(t: &Theme) -> Div {
    div()
        .absolute()
        .flex()
        .flex_col()
        .rounded(px(8.0))
        .bg(t.surface)
        .border_1()
        .border_color(t.border)
        .occlude()
}

fn divider(t: &Theme) -> Div {
    div().w(px(1.0)).h(px(16.0)).mx_1().bg(t.border)
}

/// A toolbar or popover button that runs `cmd` on click.
fn button(cmd: OverlayCmd, label: &str, active: bool, w: f32, t: &Theme, cx: &mut Context<RichEditor>) -> AnyElement {
    let hover = t.hover;
    let mut b = div()
        .id(SharedString::from(format!("rich-ov-{cmd:?}")))
        .w(px(w))
        .h(px(24.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.0))
        .text_size(px(12.0))
        .text_color(if active { t.accent } else { t.fg })
        .cursor_pointer()
        .hover(move |s| s.bg(hover));
    if active {
        b = b.bg(t.accent_soft);
    }
    b.child(label.to_string())
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.overlay_command(cmd, window, cx)
        }))
        .into_any_element()
}

/// A full-width menu row with a hover tint. The caller adds the click handler.
fn menu_row(id: String, label: &str, active: bool, t: &Theme) -> Stateful<Div> {
    let hover = t.hover;
    let mut row = div()
        .id(SharedString::from(id))
        .h(px(DD_ROW))
        .px_2()
        .flex()
        .items_center()
        .rounded(px(5.0))
        .text_size(px(12.0))
        .text_color(if active { t.accent } else { t.fg })
        .cursor_pointer()
        .hover(move |s| s.bg(hover));
    if active {
        row = row.bg(t.accent_soft);
    }
    row.child(label.to_string())
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rich_model::parse;

    #[test]
    fn slash_filter_matches_labels_and_keywords() {
        let labels = |q: &str| -> Vec<&'static str> {
            rank_slash(q).into_iter().map(|i| SLASH_ITEMS[i].label).collect()
        };
        assert_eq!(labels("").len(), SLASH_ITEMS.len(), "empty query lists everything in order");
        assert_eq!(labels("").first(), Some(&"Text"));
        assert_eq!(labels("head"), vec!["Heading 1", "Heading 2", "Heading 3"]);
        assert_eq!(labels("h2"), vec!["Heading 2"], "keyword match");
        assert_eq!(labels("note").first(), Some(&"Callout: Note"));
        assert_eq!(labels("callout").len(), 5);
        assert_eq!(labels("task"), vec!["Task list"]);
        assert_eq!(labels("code"), vec!["Code block"]);
        assert!(labels("zzz").is_empty());
        assert_eq!(labels("heading 1"), vec!["Heading 1"], "every term must match");
    }

    #[test]
    fn label_prefix_outranks_keyword_match() {
        // "quote" is a label prefix for Quote; "blockquote" only a keyword of it.
        let r = rank_slash("quo");
        assert_eq!(SLASH_ITEMS[r[0]].label, "Quote");
    }

    fn span(src: &str, caret: usize) -> Option<usize> {
        slash_span(&parse(src), src, caret)
    }

    #[test]
    fn slash_opens_at_word_starts_only() {
        assert_eq!(span("/", 1), Some(0));
        assert_eq!(span("/head", 5), Some(0));
        assert_eq!(span("text /", 6), Some(5), "after a space");
        assert_eq!(span("a/b", 3), None, "mid word");
        assert_eq!(span("http://x", 8), None, "URL");
        assert_eq!(span("", 0), None);
        assert_eq!(span("/a/b", 4), None, "a second slash ends the query");
        assert_eq!(span("- /", 3), Some(2), "after a list marker");
        assert_eq!(span("# /hi", 5), Some(2), "after a heading marker");
    }

    #[test]
    fn slash_stays_out_of_code() {
        let src = "```\n/x\n```";
        assert_eq!(span(src, 6), None);
        let src = "| a | /b |\n| - | - |";
        assert_eq!(span(src, 8), None, "table row");
    }

    #[test]
    fn slash_query_is_text_after_the_slash() {
        assert_eq!(slash_query("/head", 0, 5), "head");
        assert_eq!(slash_query("x /", 2, 3), "");
    }

    #[test]
    fn slash_heading_removes_query_and_sets_level() {
        let e = slash_edit("/head", 0, 5, SlashAction::Heading(1));
        assert_eq!(e.insert, "# ");
        assert_eq!(e.selection, 2..2);
        assert_eq!(e.range, 0..5);
    }

    #[test]
    fn slash_keeps_surrounding_lines() {
        let text = "a\n/\nb";
        let e = slash_edit(text, 2, 3, SlashAction::List(ListType::Bullet));
        assert_eq!(e.apply(text), "a\n- \nb");
        assert_eq!(e.selection, 4..4);
    }

    #[test]
    fn slash_inline_text_becomes_a_block_after_it() {
        let text = "text /";
        let e = slash_edit(text, 5, 6, SlashAction::Rule);
        let out = e.apply(text);
        assert!(out.starts_with("text"), "{out:?}");
        assert!(out.contains("\n\n---"), "{out:?}");
        assert!(!out.contains('/'), "query removed: {out:?}");
    }

    #[test]
    fn slash_table_is_three_by_three() {
        let e = slash_edit("/table", 0, 6, SlashAction::Table);
        let out = e.apply("/table");
        // Header, delimiter, then two body rows: three cells tall.
        assert_eq!(out.lines().filter(|l| l.starts_with('|')).count(), 4, "{out:?}");
    }

    #[test]
    fn slash_callout_and_chart_templates() {
        let out = slash_edit("/", 0, 1, SlashAction::Callout(CalloutKind::Warning)).apply("/");
        assert!(out.starts_with("> [!WARNING]\n> "), "{out:?}");
        let e = slash_edit("/", 0, 1, SlashAction::Chart);
        let out = e.apply("/");
        assert!(out.starts_with("```chart\n{"), "{out:?}");
        assert!(out.contains("\"datasets\""));
        assert!(out.trim_end().ends_with("```"));
        assert_eq!(e.selection.start, "```chart\n".len());
    }

    #[test]
    fn slash_image_selects_its_url() {
        let e = slash_edit("/img", 0, 4, SlashAction::Image);
        let out = e.apply("/img");
        assert_eq!(out.trim_end(), "![image](url)");
        assert_eq!(&out[e.selection.clone()], "url");
    }

    #[test]
    fn marks_report_inline_and_block_state() {
        let src = "**bold** and *it* ~~st~~ `c` [l](https://x.y)";
        let m = parse(src);
        let at = |needle: &str| {
            let s = src.find(needle).unwrap();
            s..s + needle.len()
        };
        let bold = marks_at(&m, at("bold"));
        assert!(bold.bold && !bold.italic);
        assert!(marks_at(&m, at("it")).italic);
        assert!(marks_at(&m, at("st")).strike);
        assert!(marks_at(&m, at("c")).code);
        let lp = src.find("[l]").unwrap() + 1;
        assert!(marks_at(&m, lp..lp + 1).link);
        assert!(!marks_at(&m, at("and")).bold);
    }

    #[test]
    fn marks_report_block_state_over_lines() {
        let src = "## One\n## Two\n\n- a\n- b\n\n> q";
        let m = parse(src);
        let heading = marks_at(&m, 0..src.find("\n\n").unwrap());
        assert_eq!(heading.heading, Some(2));
        assert_eq!(marks_at(&m, 0..src.find("\n##").unwrap()).heading, Some(2));
        let list_start = src.find("- a").unwrap();
        let list_end = src.find("- b").unwrap() + 3;
        assert_eq!(marks_at(&m, list_start..list_end).list, Some(ListType::Bullet));
        let mixed = marks_at(&m, 0..list_end);
        assert_eq!(mixed.heading, None);
        assert_eq!(mixed.list, None);
        assert!(marks_at(&m, src.find("> q").unwrap()..src.len()).quote);
    }

    #[test]
    fn selection_in_a_code_block_hides_the_toolbar() {
        let src = "text\n\n```\ncode\n```";
        let m = parse(src);
        assert!(!selection_in_code(&m, 0..4));
        assert!(selection_in_code(&m, 0..src.len()));
        assert!(selection_in_code(&m, src.find("code").unwrap()..src.find("code").unwrap() + 2));
    }

    #[test]
    fn link_lookup_and_edits() {
        let src = "see [a](https://x.y) now";
        let m = parse(src);
        let inside = src.find('a').unwrap();
        let link = link_inline_at(&m, inside).expect("caret inside the label");
        assert_eq!(link_target_text(src, &link), "https://x.y");
        assert!(link_inline_at(&m, 0).is_none());
        assert!(link_inline_at(&m, src.find(" now").unwrap()).is_none(), "after the link");

        let e = link_set_edit(src, &link, "https://new.example");
        assert_eq!(e.apply(src), "see [a](https://new.example) now");

        let e = link_unwrap_edit(src, &link);
        assert_eq!(e.apply(src), "see a now");
        assert_eq!(&e.apply(src)[e.selection.clone()], "a");
    }

    #[test]
    fn link_destination_with_spaces_is_bracketed() {
        let src = "[a](https://x.y)";
        let m = parse(src);
        let link = link_inline_at(&m, 1).unwrap();
        let e = link_set_edit(src, &link, "https://x.y/a b");
        assert_eq!(e.apply(src), "[a](<https://x.y/a b>)");
    }

    fn anchor(x: f32, y: f32, h: f32) -> Rect {
        Rect { x, y, w: 0.0, h }
    }

    #[test]
    fn popup_centres_and_clamps_horizontally() {
        let view = (800.0, 600.0);
        let (x, _) = place_popup(anchor(400.0, 300.0, 20.0), (200.0, 30.0), view, true);
        assert_eq!(x, 300.0, "centred on the anchor");
        let (x, _) = place_popup(anchor(100.0, 300.0, 20.0), (200.0, 30.0), view, true);
        assert_eq!(x, POPUP_MARGIN, "left edge clamps inside");
        let (x, _) = place_popup(anchor(790.0, 300.0, 20.0), (200.0, 30.0), view, true);
        assert_eq!(x, 800.0 - 200.0 - POPUP_MARGIN, "right edge clamps inside");
    }

    #[test]
    fn popup_flips_when_there_is_no_room_above() {
        let view = (800.0, 600.0);
        let (_, y) = place_popup(anchor(400.0, 300.0, 20.0), (200.0, 30.0), view, true);
        assert_eq!(y, 300.0 - POPUP_GAP - 30.0, "above when it fits");
        let (_, y) = place_popup(anchor(400.0, 2.0, 20.0), (200.0, 30.0), view, true);
        assert_eq!(y, 2.0 + 20.0 + POPUP_GAP, "below when the top is too tight");
        let (_, y) = place_popup(anchor(400.0, 300.0, 20.0), (200.0, 30.0), view, false);
        assert_eq!(y, 300.0 + 20.0 + POPUP_GAP, "below when preferred");
    }

    #[test]
    fn popup_stays_on_screen_at_the_bottom() {
        let view = (800.0, 600.0);
        let (_, y) = place_popup(anchor(400.0, 590.0, 5.0), (200.0, 30.0), view, false);
        assert!(y + 30.0 <= 600.0 - POPUP_MARGIN + 1e-3, "y {y} overflows");
    }

    #[test]
    fn overlay_command_names_resolve() {
        assert_eq!(OverlayCmd::from_name("bold"), Some(OverlayCmd::Bold));
        assert_eq!(OverlayCmd::from_name("h2"), Some(OverlayCmd::Heading(2)));
        assert_eq!(OverlayCmd::from_name("paragraph"), Some(OverlayCmd::Heading(0)));
        assert_eq!(OverlayCmd::from_name("slash-3"), Some(OverlayCmd::SlashItem(3)));
        assert_eq!(OverlayCmd::from_name("slash-99"), None);
        assert_eq!(OverlayCmd::from_name("nope"), None);
    }
}
