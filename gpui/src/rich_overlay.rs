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

use std::path::Path;
use gpui::{AnyElement, ClickEvent, Div, PathPromptOptions, SharedString, Stateful, Subscription};

use crate::editor::Editor;
use crate::rich_model::Inline;

// ---------------------------------------------------------------- metrics

const TB_W: f32 = 360.0;
const TB_H: f32 = 32.0;
/// Where the heading button's centre sits, from the bar's left edge.
const HEADING_BTN_CENTRE: f32 = 199.0;
const DD_W: f32 = 150.0;
const DD_ROW: f32 = 28.0;
const SLASH_W: f32 = 268.0;
const SLASH_ROW: f32 = 30.0;
/// Rows the slash menu shows at once; the list scrolls to keep the selection.
const SLASH_VISIBLE: usize = 10;
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
    Mermaid,
    Math,
}

pub struct SlashItem {
    pub label: &'static str,
    /// Hugeicons id (see `app_icon::icon`).
    pub icon: &'static str,
    /// Extra words the query also matches, space separated.
    pub keys: &'static str,
    pub action: SlashAction,
}

pub const SLASH_ITEMS: [SlashItem; 20] = [
    SlashItem { label: "Text", icon: "text-font", keys: "text paragraph plain normal p", action: SlashAction::Paragraph },
    SlashItem { label: "Heading 1", icon: "heading01", keys: "heading h1 title big", action: SlashAction::Heading(1) },
    SlashItem { label: "Heading 2", icon: "heading02", keys: "heading h2 subtitle", action: SlashAction::Heading(2) },
    SlashItem { label: "Heading 3", icon: "heading03", keys: "heading h3", action: SlashAction::Heading(3) },
    SlashItem { label: "Bullet list", icon: "left-to-right-list-bullet", keys: "bullet list ul unordered dash", action: SlashAction::List(ListType::Bullet) },
    SlashItem { label: "Ordered list", icon: "left-to-right-list-number", keys: "ordered numbered list ol number", action: SlashAction::List(ListType::Ordered) },
    SlashItem { label: "Task list", icon: "check-list", keys: "task todo checkbox check list", action: SlashAction::List(ListType::Task) },
    SlashItem { label: "Quote", icon: "quote-down", keys: "quote blockquote citation", action: SlashAction::Quote },
    SlashItem { label: "Code block", icon: "source-code", keys: "code block fence snippet pre", action: SlashAction::CodeBlock },
    SlashItem { label: "Table", icon: "table01", keys: "table grid 3x3", action: SlashAction::Table },
    SlashItem { label: "Divider", icon: "minus-sign", keys: "divider rule hr line separator ---", action: SlashAction::Rule },
    SlashItem { label: "Image", icon: "image01", keys: "image img picture pic photo", action: SlashAction::Image },
    SlashItem { label: "Callout: Note", icon: "information-circle", keys: "callout note alert admonition info", action: SlashAction::Callout(CalloutKind::Note) },
    SlashItem { label: "Callout: Tip", icon: "information-circle", keys: "callout tip hint", action: SlashAction::Callout(CalloutKind::Tip) },
    SlashItem { label: "Callout: Important", icon: "alert-circle", keys: "callout important", action: SlashAction::Callout(CalloutKind::Important) },
    SlashItem { label: "Callout: Warning", icon: "alert01", keys: "callout warning caution alert", action: SlashAction::Callout(CalloutKind::Warning) },
    SlashItem { label: "Callout: Caution", icon: "alert01", keys: "callout caution danger", action: SlashAction::Callout(CalloutKind::Caution) },
    SlashItem { label: "Chart", icon: "chart01", keys: "chart graph bar line pie data", action: SlashAction::Chart },
    SlashItem { label: "Mermaid diagram", icon: "list-tree", keys: "mermaid diagram flowchart graph sequence", action: SlashAction::Mermaid },
    SlashItem { label: "Math block", icon: "multiplication-sign", keys: "math latex katex equation formula tex", action: SlashAction::Math },
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
    // Don't open inside inline code or link destination / autolink.
    if line.inlines.iter().any(|inl| inl.range.contains(&p) && matches!(inl.kind, InlineKind::Code | InlineKind::Link { .. } | InlineKind::AutoLink)) {
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

/// Destination for an image: relative to `doc_path`'s parent folder when under it,
/// otherwise absolute. Escaped with `<>` via `link_dest` when containing spaces or parentheses.
pub fn image_dest_path(image_path: &Path, doc_path: Option<&Path>) -> String {
    let rel = doc_path
        .and_then(|doc| doc.parent())
        .and_then(|parent| image_path.strip_prefix(parent).ok());
    let path_str = match rel {
        Some(r) => r.to_string_lossy(),
        None => image_path.to_string_lossy(),
    };
    link_dest(&path_str)
}

/// Applies an image path to `content`: if `sel` is currently selecting "url" in `![image](url)`
/// (as left by slash 'Image'), replaces "url" with the destination and positions the caret
/// right after the closing ')'. Otherwise inserts `![image](<dest>)` at `sel`.
pub fn apply_image_path_edit(
    content: &str,
    sel: Range<usize>,
    image_path: &Path,
    doc_path: Option<&Path>,
) -> Edit {
    let dest = image_dest_path(image_path, doc_path);
    if sel.end <= content.len() && content.get(sel.clone()) == Some("url") {
        let after = &content[sel.end..];
        let caret = if after.starts_with(')') {
            sel.start + dest.len() + 1
        } else {
            sel.start + dest.len()
        };
        Edit {
            range: sel,
            insert: dest,
            selection: caret..caret,
        }
    } else {
        let body = format!("![image]({dest})");
        let n = body.len();
        rich_ops::block_edit(content, sel.start, sel.end, &body, n)
    }
}

/// The whole document after running `action` on the line with `/query` at
/// `start..caret`. The query is removed first, so the result is one edit and
/// one undo step.
pub fn slash_edit(text: &str, start: usize, caret: usize, action: SlashAction) -> Edit {
    let caret = caret.clamp(start, text.len());
    let (ls, le) = rich_ops::line_bounds(text, start);
    let p = rich_ops::line_prefix(text, start);

    // If the line consists only of block markers (> , - , # ) before the slash
    // and nothing but whitespace after the query, the action replaces the whole
    // line so no empty list bullets, quotes or headings are left dangling.
    let prefix_only = start == p.content_start
        || (start > p.content_start && text[p.content_start..start].trim().is_empty());
    let rest_empty = text[caret..le].trim().is_empty();
    let empty_line_query = prefix_only && rest_empty;

    let is_block = matches!(
        action,
        SlashAction::CodeBlock
            | SlashAction::Table
            | SlashAction::Rule
            | SlashAction::Image
            | SlashAction::Callout(_)
            | SlashAction::Chart
            | SlashAction::Mermaid
            | SlashAction::Math
    );

    if is_block {
        let (s, e) = if empty_line_query {
            (ls, le)
        } else {
            // Trim trailing spaces/tabs on the text before '/' so the block has clean spacing.
            let before = &text[..start];
            let trimmed = before.trim_end_matches([' ', '\t']);
            let s = if trimmed.len() >= ls { trimmed.len() } else { start };
            (s, caret)
        };
        match action {
            SlashAction::CodeBlock => {
                let head = "```\n";
                let body = format!("{head}\n```");
                rich_ops::block_edit(text, s, e, &body, head.len())
            }
            SlashAction::Table => {
                let (rendered, pos) = rich_ops::table_template(3, 3);
                rich_ops::block_edit(text, s, e, &rendered, pos[0][0])
            }
            SlashAction::Rule => rich_ops::block_edit(text, s, e, "---", 4),
            SlashAction::Image => {
                // `url` is selected so typing replaces it.
                let body = "![image](url)";
                let ed = rich_ops::block_edit(text, s, e, body, 9);
                let c = ed.selection.start;
                Edit {
                    selection: c..c + 3,
                    ..ed
                }
            }
            SlashAction::Callout(kind) => {
                let body = format!("> [!{}]\n> ", callout_name(kind));
                let n = body.len();
                rich_ops::block_edit(text, s, e, &body, n)
            }
            SlashAction::Chart => {
                let head = "```chart\n";
                let starter = "{\n  \"type\": \"bar\",\n  \"data\": {\n    \"labels\": [\"A\", \"B\", \"C\"],\n    \"datasets\": [{ \"label\": \"Series 1\", \"data\": [3, 5, 2] }]\n  }\n}";
                let body = format!("{head}{starter}\n```");
                let ed = rich_ops::block_edit(text, s, e, &body, head.len());
                let c = ed.selection.start;
                Edit {
                    selection: c..c + starter.len(),
                    ..ed
                }
            }
            SlashAction::Mermaid => {
                let head = "```mermaid\n";
                let starter = "flowchart LR\n  A[Start] --> B[End]";
                let body = format!("{head}{starter}\n```");
                let ed = rich_ops::block_edit(text, s, e, &body, head.len());
                let c = ed.selection.start;
                Edit {
                    selection: c..c + starter.len(),
                    ..ed
                }
            }
            SlashAction::Math => {
                let head = "```math\n";
                let starter = "E = mc^2";
                let body = format!("{head}{starter}\n```");
                let ed = rich_ops::block_edit(text, s, e, &body, head.len());
                let c = ed.selection.start;
                Edit {
                    selection: c..c + starter.len(),
                    ..ed
                }
            }
            _ => unreachable!(),
        }
    } else {
        if empty_line_query {
            let (insert, caret_offset) = match action {
                SlashAction::Heading(n) => (format!("{} ", "#".repeat(n.clamp(1, 6) as usize)), n as usize + 1),
                SlashAction::List(ListType::Bullet) => ("- ".to_string(), 2),
                SlashAction::List(ListType::Ordered) => ("1. ".to_string(), 3),
                SlashAction::List(ListType::Task) => ("- [ ] ".to_string(), 6),
                SlashAction::Quote => ("> ".to_string(), 2),
                SlashAction::Paragraph => (String::new(), 0),
                _ => unreachable!(),
            };
            let sel = ls + caret_offset;
            Edit {
                range: ls..le,
                insert,
                selection: sel..sel,
            }
        } else {
            let stripped = format!("{}{}", &text[..start], &text[caret..]);
            let s = start;
            let op: Edit = match action {
                SlashAction::Paragraph => rich_ops::set_paragraph(&stripped, s..s),
                SlashAction::Heading(n) => rich_ops::set_heading(&stripped, s..s, Some(n)),
                SlashAction::List(kind) => rich_ops::toggle_list(&stripped, s..s, kind),
                SlashAction::Quote => rich_ops::toggle_quote(&stripped, s..s),
                _ => unreachable!(),
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

/// An open slash menu: the `/` offset, the highlighted row, and how far the
/// list is scrolled (`top`, in pixels, so a trackpad can scroll smoothly).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SlashMenu {
    pub start: usize,
    pub selected: usize,
    pub top: i32,
}

impl SlashMenu {
    /// Largest scroll offset for a list of `n` rows.
    fn max_top(n: usize) -> i32 {
        (n.saturating_sub(SLASH_VISIBLE) as f32 * SLASH_ROW) as i32
    }

    /// `top` clamped to the list, then moved just far enough to show the selected row.
    fn reveal(mut self, n: usize) -> Self {
        let row = SLASH_ROW as i32;
        let sel_top = self.selected as i32 * row;
        let view = SLASH_VISIBLE as i32 * row;
        if sel_top < self.top {
            self.top = sel_top;
        } else if sel_top + row > self.top + view {
            self.top = sel_top + row - view;
        }
        self.top = self.top.clamp(0, Self::max_top(n));
        self
    }
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
        self.overlay_sync_slash();
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
        self.overlay_sync_slash();
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
                    self.overlays.slash = Some(SlashMenu { selected: next, ..menu }.reveal(n));
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
        let action = item.action;
        let edit = slash_edit(&self.content, menu.start, self.cursor(), action);
        self.apply_edit(edit, cx);
        if action == SlashAction::Image {
            self.prompt_for_image(cx);
        }
    }

    /// Replaces the image url with `image_path` (relative to the active document if possible).
    pub fn apply_image_path(&mut self, image_path: &Path, cx: &mut Context<Self>) {
        let doc_path = self.widgets.file_path.as_deref();
        let edit = apply_image_path_edit(&self.content, self.selection.clone(), image_path, doc_path);
        self.apply_edit(edit, cx);
    }

    /// Prompts for an image using the native file picker, falling back to leaving `url` selected.
    pub fn prompt_for_image(&mut self, cx: &mut Context<Self>) {
        if std::env::var_os("ZYPLUS_TEST_SCRIPT").is_some() {
            return;
        }
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose an image".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await {
                if let Some(path) = paths.into_iter().next() {
                    let _ = this.update(cx, |editor, cx| {
                        editor.apply_image_path(&path, cx);
                    });
                }
            }
        })
        .detach();
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
                    self.overlays.slash = Some(SlashMenu { selected, ..menu }.reveal(n));
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
                    self.overlays.slash = Some(SlashMenu { start, selected: 0, top: 0 });
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
                let (x, y) = place_popup(anchor, (LINK_W, LINK_H), view, true);
                out.push(self.link_el(&t, (x, y), pop.target.is_some(), cx));
            }
        } else if let Some(inl) = self.tooltip_target() {
            if let Some(anchor) = self.caret_rect(self.cursor(), self.at_row_end) {
                let url = link_target_text(&self.content, &inl);
                // Above the link text, so it never covers the table header or the
                // line above; flips below only when there is no room above.
                let (x, y) = place_popup(anchor, (TIP_W, TIP_H), view, true);
                out.push(self.tooltip_el(&t, (x, y), url, cx));
            }
        }
        out
    }

    fn toolbar_el(&self, t: &Theme, at: (f32, f32), marks: Marks, cx: &mut Context<Self>) -> AnyElement {
        let heading_label = match marks.heading {
            Some(n) if n > 0 => format!("H{n}"),
            _ => "Text".to_string(),
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
            .child(icon_button(OverlayCmd::Bold, "text-bold", marks.bold, t, cx))
            .child(icon_button(OverlayCmd::Italic, "text-italic", marks.italic, t, cx))
            .child(icon_button(OverlayCmd::Strike, "text-strikethrough", marks.strike, t, cx))
            .child(icon_button(OverlayCmd::Code, "source-code", marks.code, t, cx))
            .child(divider(t))
            .child(icon_button(OverlayCmd::Link, "link01", marks.link, t, cx))
            .child(divider(t))
            .child(dropdown_button(OverlayCmd::Dropdown, &heading_label, heading_active, t, cx))
            .child(divider(t))
            .child(icon_button(OverlayCmd::Bullet, "left-to-right-list-bullet", list(ListType::Bullet), t, cx))
            .child(icon_button(OverlayCmd::Ordered, "left-to-right-list-number", list(ListType::Ordered), t, cx))
            .child(icon_button(OverlayCmd::Task, "check-list", list(ListType::Task), t, cx))
            .child(icon_button(OverlayCmd::Quote, "quote-down", marks.quote, t, cx))
            .into_any_element()
    }

    fn heading_menu_el(&self, t: &Theme, at: (f32, f32), current: Option<u8>, cx: &mut Context<Self>) -> AnyElement {
        let rows = [("Paragraph", "text-font", 0u8), ("Heading 1", "heading01", 1), ("Heading 2", "heading02", 2), ("Heading 3", "heading03", 3)];
        let mut panel = panel(t).left(px(at.0)).top(px(at.1)).w(px(DD_W)).p_1();
        for (label, icon, level) in rows {
            let active = current.unwrap_or(0) == level;
            panel = panel.child(
                menu_row(
                    format!("rich-ov-heading-{level}"),
                    label,
                    Some(icon),
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
        let menu = self.overlays.slash;
        let selected = menu.map(|m| m.selected).unwrap_or(0);
        let top = menu.map(|m| m.top).unwrap_or(0).clamp(0, SlashMenu::max_top(ranked.len()));
        let first = (top as f32 / SLASH_ROW).floor() as usize;
        let mut panel = panel(t)
            .left(px(at.0))
            .top(px(at.1))
            .w(px(SLASH_W))
            .p_1()
            .on_scroll_wheel(cx.listener(|this, event: &gpui::ScrollWheelEvent, _, cx| {
                // The panel blocks the mouse, so the wheel scrolls the list, not the page.
                let n = this.slash_ranked().len();
                if let Some(menu) = this.overlays.slash {
                    let delta = f32::from(event.delta.pixel_delta(px(SLASH_ROW)).y);
                    let top = (menu.top - delta as i32).clamp(0, SlashMenu::max_top(n));
                    if top != menu.top {
                        this.overlays.slash = Some(SlashMenu { top, ..menu });
                        cx.notify();
                    }
                }
                cx.stop_propagation();
            }));
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
                menu_row(format!("rich-slash-{index}"), item.label, Some(item.icon), active, t)
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
                    .child(labeled_icon_button(OverlayCmd::Open, "external-link", "Open", 64.0, t, cx))
                    .child(labeled_icon_button(OverlayCmd::Edit, "pencil-edit01", "Edit", 60.0, t, cx))
                    .child(labeled_icon_button(OverlayCmd::Unlink, "unlink01", "Remove", 80.0, t, cx)),
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
        .rounded_full()
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

/// A square toolbar button showing a Hugeicons glyph.
fn icon_button(cmd: OverlayCmd, icon: &str, active: bool, t: &Theme, cx: &mut Context<RichEditor>) -> AnyElement {
    let hover = t.hover;
    let color = if active { t.accent } else { t.fg };
    let mut b = div()
        .id(SharedString::from(format!("rich-ov-{cmd:?}")))
        .w(px(26.0))
        .h(px(24.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .cursor_pointer()
        .hover(move |s| s.bg(hover));
    if active {
        b = b.bg(t.accent_soft);
    }
    b.child(crate::app_icon::icon(icon, 15.0).text_color(color))
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.overlay_command(cmd, window, cx)
        }))
        .into_any_element()
}

/// A button with a glyph and a label side by side.
fn labeled_icon_button(
    cmd: OverlayCmd,
    icon: &str,
    label: &str,
    w: f32,
    t: &Theme,
    cx: &mut Context<RichEditor>,
) -> AnyElement {
    let hover = t.hover;
    div()
        .id(SharedString::from(format!("rich-ov-{cmd:?}")))
        .w(px(w))
        .h(px(24.0))
        .flex()
        .items_center()
        .justify_center()
        .gap_1()
        .rounded_full()
        .text_size(px(12.0))
        .text_color(t.fg)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(crate::app_icon::icon(icon, 14.0).text_color(t.fg))
        .child(label.to_string())
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.overlay_command(cmd, window, cx)
        }))
        .into_any_element()
}

/// The heading picker: its label and a chevron.
fn dropdown_button(cmd: OverlayCmd, label: &str, active: bool, t: &Theme, cx: &mut Context<RichEditor>) -> AnyElement {
    let hover = t.hover;
    let color = if active { t.accent } else { t.fg };
    let mut b = div()
        .id(SharedString::from(format!("rich-ov-{cmd:?}")))
        .w(px(64.0))
        .h(px(24.0))
        .flex()
        .items_center()
        .justify_center()
        .gap_0p5()
        .rounded_full()
        .text_size(px(12.0))
        .text_color(color)
        .cursor_pointer()
        .hover(move |s| s.bg(hover));
    if active {
        b = b.bg(t.accent_soft);
    }
    b.child(label.to_string())
        .child(crate::app_icon::icon("arrow-down01", 12.0).text_color(color))
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.overlay_command(cmd, window, cx)
        }))
        .into_any_element()
}

/// A full-width menu row with a hover tint. The caller adds the click handler.
fn menu_row(id: String, label: &str, icon: Option<&str>, active: bool, t: &Theme) -> Stateful<Div> {
    let hover = t.hover;
    let mut row = div()
        .id(SharedString::from(id))
        .h(px(DD_ROW))
        .px_2()
        .flex()
        .items_center()
        .gap_2()
        .rounded(px(5.0))
        .text_size(px(12.0))
        .text_color(if active { t.accent } else { t.fg })
        .cursor_pointer()
        .hover(move |s| s.bg(hover));
    if active {
        row = row.bg(t.accent_soft);
    }
    row.children(icon.map(|name| {
        crate::app_icon::icon(name, 15.0).text_color(if active { t.accent } else { t.muted })
    }))
    .child(label.to_string())
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

        // Natural queries required by brief:
        assert_eq!(labels("code").first(), Some(&"Code block"));
        assert_eq!(labels("img").first(), Some(&"Image"));
        assert_eq!(labels("table").first(), Some(&"Table"));
        assert_eq!(labels("pic").first(), Some(&"Image"));
        assert_eq!(labels("fence").first(), Some(&"Code block"));
        assert_eq!(labels("snippet").first(), Some(&"Code block"));
        assert_eq!(labels("grid").first(), Some(&"Table"));
        assert_eq!(labels("photo").first(), Some(&"Image"));

        // Step 3 new items queries:
        assert_eq!(labels("mermaid"), vec!["Mermaid diagram"]);
        assert_eq!(labels("flowchart"), vec!["Mermaid diagram"]);
        assert!(labels("diagram").contains(&"Mermaid diagram"));
        assert!(labels("graph").contains(&"Mermaid diagram"));
        assert!(labels("graph").contains(&"Chart"));
        assert_eq!(labels("sequence"), vec!["Mermaid diagram"]);

        assert_eq!(labels("math"), vec!["Math block"]);
        assert_eq!(labels("latex"), vec!["Math block"]);
        assert_eq!(labels("katex"), vec!["Math block"]);
        assert_eq!(labels("equation"), vec!["Math block"]);
        assert_eq!(labels("formula"), vec!["Math block"]);
        assert!(labels("tex").contains(&"Math block"));
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
        assert_eq!(span("` /`", 2), None, "inside inline code");
        assert_eq!(span("[link](http://x)", 10), None, "inside link destination");
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
        assert!(&out[e.selection.clone()].starts_with("{\n"));
    }

    #[test]
    fn slash_mermaid_and_math_templates() {
        let e = slash_edit("/mermaid", 0, 8, SlashAction::Mermaid);
        let out = e.apply("/mermaid");
        assert!(out.starts_with("```mermaid\nflowchart LR\n  A[Start] --> B[End]\n```"), "{out:?}");
        assert_eq!(&out[e.selection.clone()], "flowchart LR\n  A[Start] --> B[End]");

        let e = slash_edit("/math", 0, 5, SlashAction::Math);
        let out = e.apply("/math");
        assert!(out.starts_with("```math\nE = mc^2\n```"), "{out:?}");
        assert_eq!(&out[e.selection.clone()], "E = mc^2");
    }

    #[test]
    fn slash_image_selects_its_url() {
        let e = slash_edit("/img", 0, 4, SlashAction::Image);
        let out = e.apply("/img");
        assert_eq!(out.trim_end(), "![image](url)");
        assert_eq!(&out[e.selection.clone()], "url");
    }

    #[test]
    fn slash_image_picker_path_logic() {
        use std::path::PathBuf;
        let doc = PathBuf::from("/Users/test/notes/my_note.md");

        // 1. Relative under folder
        let img1 = PathBuf::from("/Users/test/notes/photo.png");
        assert_eq!(image_dest_path(&img1, Some(&doc)), "photo.png");

        let img2 = PathBuf::from("/Users/test/notes/sub/pic.png");
        assert_eq!(image_dest_path(&img2, Some(&doc)), "sub/pic.png");

        // 2. Spaces and parens escaped via link_dest
        let img_spaces = PathBuf::from("/Users/test/notes/my photo (1).png");
        assert_eq!(image_dest_path(&img_spaces, Some(&doc)), "<my photo (1).png>");

        // 3. Outside folder: absolute
        let img_other = PathBuf::from("/Users/test/other/outside.png");
        assert_eq!(image_dest_path(&img_other, Some(&doc)), "/Users/test/other/outside.png");

        // 4. Replacing "url" in ![image](url) and placing caret after ')'
        let text = "![image](url)\n";
        let edit = apply_image_path_edit(text, 9..12, &img1, Some(&doc));
        let out = edit.apply(text);
        assert_eq!(out, "![image](photo.png)\n");
        // Caret right after ')'
        assert_eq!(edit.selection, 19..19);
        assert_eq!(&out[..edit.selection.start], "![image](photo.png)");
    }

    #[test]
    fn slash_edit_contexts_a_through_g() {
        // (a) an empty document
        let e = slash_edit("/code", 0, 5, SlashAction::CodeBlock);
        let out = e.apply("/code");
        assert_eq!(out, "```\n\n```\n");
        assert_eq!(e.selection, 4..4);

        // (b) an empty line between paragraphs
        let text = "para 1\n\n/code\n\npara 2";
        let e = slash_edit(text, 8, 13, SlashAction::CodeBlock);
        let out = e.apply(text);
        assert_eq!(out, "para 1\n\n```\n\n```\n\npara 2");

        // (c) end of a non-empty paragraph line after a space
        let text = "hello /code";
        let e = slash_edit(text, 6, 11, SlashAction::CodeBlock);
        let out = e.apply(text);
        assert_eq!(out, "hello\n\n```\n\n```\n");

        // (d1) inside an empty list item: replaces list marker cleanly without dangling '- '
        let text = "- /code";
        let e = slash_edit(text, 2, 7, SlashAction::CodeBlock);
        let out = e.apply(text);
        assert_eq!(out, "```\n\n```\n");

        // (d2) inside a list item with text: inserts block after text
        let text = "- item /code";
        let e = slash_edit(text, 7, 12, SlashAction::CodeBlock);
        let out = e.apply(text);
        assert_eq!(out, "- item\n\n```\n\n```\n");

        // (e1) inside an empty quote: replaces quote marker cleanly without dangling '> '
        let text = "> /code";
        let e = slash_edit(text, 2, 7, SlashAction::CodeBlock);
        let out = e.apply(text);
        assert_eq!(out, "```\n\n```\n");

        // (e2) inside a quote with text: inserts block after text
        let text = "> quote /code";
        let e = slash_edit(text, 8, 13, SlashAction::CodeBlock);
        let out = e.apply(text);
        assert_eq!(out, "> quote\n\n```\n\n```\n");

        // (f) last line of the file with no trailing newline
        let text = "first line\n/code";
        let e = slash_edit(text, 11, 16, SlashAction::CodeBlock);
        let out = e.apply(text);
        assert_eq!(out, "first line\n\n```\n\n```\n");

        // (g) document start
        let text = "/code\nsecond line";
        let e = slash_edit(text, 0, 5, SlashAction::CodeBlock);
        let out = e.apply(text);
        assert_eq!(out, "```\n\n```\n\nsecond line");
    }

    #[test]
    fn slash_edit_table_and_image_contexts() {
        // Table in empty list item
        let e = slash_edit("- /table", 2, 8, SlashAction::Table);
        let out = e.apply("- /table");
        assert!(out.starts_with('|'), "{out:?}");
        assert!(out.contains("| --- |"), "{out:?}");
        assert!(!out.starts_with("- "), "{out:?}");
        assert!(!out.contains("- \n"), "{out:?}");

        // Image in empty quote
        let e = slash_edit("> /img", 2, 6, SlashAction::Image);
        let out = e.apply("> /img");
        assert_eq!(out.trim_end(), "![image](url)");
        assert_eq!(&out[e.selection.clone()], "url");

        // Mermaid in empty line between paragraphs
        let text = "p1\n\n/mermaid\n\np2";
        let e = slash_edit(text, 4, 12, SlashAction::Mermaid);
        let out = e.apply(text);
        assert!(out.contains("```mermaid\nflowchart LR"));

        // Math at document start
        let text = "/math\nrest";
        let e = slash_edit(text, 0, 5, SlashAction::Math);
        let out = e.apply(text);
        assert!(out.starts_with("```math\nE = mc^2\n```\n\nrest"));
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

#[cfg(test)]
mod slash_scroll_tests {
    use super::*;

    #[test]
    fn reveal_keeps_the_selected_row_in_view() {
        let n = SLASH_ITEMS.len();
        let row = SLASH_ROW as i32;
        let menu = SlashMenu { start: 0, selected: 0, top: 0 };
        assert_eq!(SlashMenu { selected: SLASH_VISIBLE - 1, ..menu }.reveal(n).top, 0);
        assert_eq!(SlashMenu { selected: SLASH_VISIBLE, ..menu }.reveal(n).top, row);
        // Table is the 10th item: reachable and visible with the keyboard.
        let table = SLASH_ITEMS.iter().position(|i| i.label == "Table").unwrap();
        let shown = SlashMenu { selected: table, ..menu }.reveal(n);
        let first = (shown.top / row) as usize;
        assert!(table >= first && table < first + SLASH_VISIBLE);
        // The last item is the last row of the window, and scrolling cannot go past it.
        let last = SlashMenu { selected: n - 1, top: 10_000, start: 0 }.reveal(n);
        assert_eq!(last.top, SlashMenu::max_top(n));
        assert_eq!(SlashMenu { selected: 0, top: 500, start: 0 }.reveal(n).top, 0);
    }
}
