//! The "rich" half of a tab: the markdown drawn as a document rather than as
//! source. The web build edits through Milkdown here; a native port has no
//! WYSIWYG editor, so this view renders the same buffer read-only and Cmd-E
//! swaps back to the text editor to change it.
//!
//! Markdown is parsed with `pulldown-cmark` (CommonMark + GFM), so headings,
//! emphasis, inline code, links, nested and task lists, quotes, tables, rules
//! and images all keep their structure instead of being read as plain text.

use std::cell::{Cell, RefCell};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::path::{Path, PathBuf};

use gpui::prelude::*;
use gpui::{
    div, img, px, AnyElement, Div, FontStyle, FontWeight, HighlightStyle, InteractiveText,
    SharedString, Stateful, StrikethroughStyle, StyledText, TextAlign, UnderlineStyle,
};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

use crate::chart;
use crate::editor::MONO_FONT;
use crate::highlight::{self, TokenKind};
use crate::theme::Theme;

// ---------------------------------------------------------------------------
// The document model
// ---------------------------------------------------------------------------

/// A run of text that shares one inline style.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    pub link: bool,
    /// The destination of the link this text is in, when `link` is set.
    pub href: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Callout {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ListItem {
    /// `Some` for a task list item, `None` for a plain bullet or number.
    pub checked: Option<bool>,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Heading {
        level: u8,
        spans: Vec<Span>,
    },
    Paragraph {
        spans: Vec<Span>,
    },
    Code {
        lang: String,
        text: String,
    },
    Quote {
        kind: Option<Callout>,
        blocks: Vec<Block>,
    },
    List {
        ordered: bool,
        start: u64,
        items: Vec<ListItem>,
    },
    Table {
        aligns: Vec<Align>,
        header: Vec<Vec<Span>>,
        rows: Vec<Vec<Vec<Span>>>,
    },
    Rule,
    Image {
        src: String,
        alt: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Document {
    pub blocks: Vec<Block>,
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
struct InlineStyle {
    bold: u32,
    italic: u32,
    strike: u32,
    link: u32,
}

#[derive(Debug)]
struct TableBuild {
    aligns: Vec<Align>,
    header: Vec<Vec<Span>>,
    rows: Vec<Vec<Vec<Span>>>,
    row: Vec<Vec<Span>>,
    in_head: bool,
}

#[derive(Debug)]
enum Frame {
    Quote {
        kind: Option<Callout>,
        blocks: Vec<Block>,
    },
    List {
        ordered: bool,
        start: u64,
        items: Vec<ListItem>,
        item: Option<ListItem>,
    },
    Table(TableBuild),
}

struct Builder {
    doc: Vec<Block>,
    stack: Vec<Frame>,
    spans: Vec<Span>,
    style: InlineStyle,
    heading: Option<u8>,
    paragraph: bool,
    code: Option<(String, String)>,
    image: Option<(String, Vec<Span>)>,
    cell: Option<Vec<Span>>,
    /// Destinations of the links being read, innermost last.
    links: Vec<String>,
}

impl Builder {
    fn new() -> Self {
        Self {
            doc: Vec::new(),
            stack: Vec::new(),
            spans: Vec::new(),
            style: InlineStyle::default(),
            heading: None,
            paragraph: false,
            code: None,
            image: None,
            cell: None,
            links: Vec::new(),
        }
    }

    fn push_block(&mut self, block: Block) {
        match self.stack.last_mut() {
            None => self.doc.push(block),
            Some(Frame::Quote { blocks, .. }) => blocks.push(block),
            Some(Frame::List { item, .. }) => {
                item.get_or_insert_with(|| ListItem {
                    checked: None,
                    blocks: Vec::new(),
                })
                .blocks
                .push(block);
            }
            // A block inside a table cell has nowhere sensible to go.
            Some(Frame::Table(_)) => {}
        }
    }

    fn push_span(&mut self, text: &str, code: bool) {
        if text.is_empty() {
            return;
        }
        let span = Span {
            text: text.to_string(),
            bold: self.style.bold > 0,
            italic: self.style.italic > 0,
            strike: self.style.strike > 0,
            code,
            link: self.style.link > 0,
            href: self.links.last().cloned(),
        };
        if let Some(cell) = &mut self.cell {
            append_span(cell, span);
            return;
        }
        if let Some((_, alt)) = &mut self.image {
            append_span(alt, span);
            return;
        }
        append_span(&mut self.spans, span);
    }

    /// Text events: code blocks and image alt text collect their own text.
    fn text(&mut self, text: &str) {
        if let Some((_, body)) = &mut self.code {
            body.push_str(text);
            return;
        }
        self.push_span(text, false);
    }

    fn take_spans(&mut self) -> Option<Vec<Span>> {
        let mut spans = std::mem::take(&mut self.spans);
        while spans
            .last()
            .map(|span| span.text.trim().is_empty() && !span.code)
            .unwrap_or(false)
        {
            spans.pop();
        }
        if let Some(last) = spans.last_mut() {
            let trimmed = last.text.trim_end_matches([' ', '\t']).len();
            last.text.truncate(trimmed);
        }
        spans.retain(|span| !span.text.is_empty());
        if spans.is_empty() {
            None
        } else {
            Some(spans)
        }
    }

    fn flush_paragraph(&mut self) {
        if let Some(spans) = self.take_spans() {
            self.push_block(Block::Paragraph { spans });
        }
    }

    fn flush_heading(&mut self, level: u8) {
        if let Some(spans) = self.take_spans() {
            self.push_block(Block::Heading { level, spans });
        }
    }

    fn start(&mut self, tag: Tag) {
        // A tight list item has no Paragraph around its text. Whatever is
        // pending belongs to the item before a nested block starts, or it
        // would be merged into that block's first entry.
        if matches!(
            tag,
            Tag::List(_) | Tag::BlockQuote(_) | Tag::CodeBlock(_) | Tag::Table(_)
        ) {
            self.flush_paragraph();
        }
        match tag {
            Tag::Paragraph => {
                self.paragraph = true;
                self.spans.clear();
            }
            Tag::Heading { level, .. } => {
                self.heading = Some(level as u8);
                self.spans.clear();
            }
            Tag::CodeBlock(kind) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => info
                        .split_whitespace()
                        .next()
                        .unwrap_or_default()
                        .to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some((lang, String::new()));
            }
            Tag::BlockQuote(kind) => {
                let kind = kind.map(|kind| match kind {
                    pulldown_cmark::BlockQuoteKind::Note => Callout::Note,
                    pulldown_cmark::BlockQuoteKind::Tip => Callout::Tip,
                    pulldown_cmark::BlockQuoteKind::Important => Callout::Important,
                    pulldown_cmark::BlockQuoteKind::Warning => Callout::Warning,
                    pulldown_cmark::BlockQuoteKind::Caution => Callout::Caution,
                });
                self.stack.push(Frame::Quote {
                    kind,
                    blocks: Vec::new(),
                });
            }
            Tag::List(start) => {
                self.stack.push(Frame::List {
                    ordered: start.is_some(),
                    start: start.unwrap_or(1),
                    items: Vec::new(),
                    item: None,
                });
            }
            Tag::Item => {
                if let Some(Frame::List { item, .. }) = self.stack.last_mut() {
                    *item = Some(ListItem {
                        checked: None,
                        blocks: Vec::new(),
                    });
                }
            }
            Tag::Table(aligns) => {
                self.stack.push(Frame::Table(TableBuild {
                    aligns: aligns
                        .into_iter()
                        .map(|align| match align {
                            pulldown_cmark::Alignment::Center => Align::Center,
                            pulldown_cmark::Alignment::Right => Align::Right,
                            _ => Align::Left,
                        })
                        .collect(),
                    header: Vec::new(),
                    rows: Vec::new(),
                    row: Vec::new(),
                    in_head: false,
                }));
            }
            Tag::TableHead => {
                if let Some(Frame::Table(table)) = self.stack.last_mut() {
                    table.in_head = true;
                    table.row.clear();
                }
            }
            Tag::TableRow => {
                if let Some(Frame::Table(table)) = self.stack.last_mut() {
                    table.row.clear();
                }
            }
            Tag::TableCell => self.cell = Some(Vec::new()),
            Tag::Emphasis => self.style.italic += 1,
            Tag::Strong => self.style.bold += 1,
            Tag::Strikethrough => self.style.strike += 1,
            Tag::Link { dest_url, .. } => {
                self.style.link += 1;
                self.links.push(dest_url.to_string());
            }
            Tag::Image { dest_url, .. } => {
                // Keep the document order: text before an image is its own paragraph.
                if self.paragraph && !self.spans.is_empty() {
                    self.flush_paragraph();
                }
                self.image = Some((dest_url.to_string(), Vec::new()));
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.paragraph = false;
                self.flush_paragraph();
            }
            TagEnd::Heading(level) => {
                self.heading = None;
                self.flush_heading(level as u8);
            }
            TagEnd::CodeBlock => {
                if let Some((lang, mut body)) = self.code.take() {
                    while body.ends_with('\n') {
                        body.pop();
                    }
                    self.push_block(Block::Code { lang, text: body });
                }
            }
            TagEnd::BlockQuote(_) => {
                if let Some(Frame::Quote { kind, blocks }) = self.stack.pop() {
                    self.push_block(Block::Quote { kind, blocks });
                }
            }
            TagEnd::List(_) => {
                if let Some(Frame::List {
                    ordered,
                    start,
                    items,
                    ..
                }) = self.stack.pop()
                {
                    self.push_block(Block::List {
                        ordered,
                        start,
                        items,
                    });
                }
            }
            TagEnd::Item => {
                // A tight list item has no Paragraph events; flush its text now.
                if !self.spans.is_empty() {
                    self.flush_paragraph();
                }
                if let Some(Frame::List { item, items, .. }) = self.stack.last_mut() {
                    if let Some(item) = item.take() {
                        items.push(item);
                    }
                }
            }
            TagEnd::Table => {
                if let Some(Frame::Table(table)) = self.stack.pop() {
                    let mut rows = table.rows;
                    if !table.row.is_empty() && !table.header.is_empty() {
                        rows.push(table.row);
                    }
                    self.push_block(Block::Table {
                        aligns: table.aligns,
                        header: table.header,
                        rows,
                    });
                }
            }
            TagEnd::TableHead => {
                if let Some(Frame::Table(table)) = self.stack.last_mut() {
                    table.header = std::mem::take(&mut table.row);
                    table.in_head = false;
                }
            }
            TagEnd::TableRow => {
                if let Some(Frame::Table(table)) = self.stack.last_mut() {
                    if !table.in_head {
                        let row = std::mem::take(&mut table.row);
                        table.rows.push(row);
                    }
                }
            }
            TagEnd::TableCell => {
                if let (Some(cell), Some(Frame::Table(table))) =
                    (self.cell.take(), self.stack.last_mut())
                {
                    table.row.push(cell);
                }
            }
            TagEnd::Emphasis => self.style.italic = self.style.italic.saturating_sub(1),
            TagEnd::Strong => self.style.bold = self.style.bold.saturating_sub(1),
            TagEnd::Strikethrough => self.style.strike = self.style.strike.saturating_sub(1),
            TagEnd::Link => {
                self.style.link = self.style.link.saturating_sub(1);
                self.links.pop();
            }
            TagEnd::Image => {
                if let Some((src, alt)) = self.image.take() {
                    let alt = alt.into_iter().map(|span| span.text).collect::<String>();
                    self.push_block(Block::Image { src, alt });
                }
            }
            _ => {}
        }
    }

    fn task_marker(&mut self, done: bool) {
        if let Some(Frame::List {
            item: Some(item), ..
        }) = self.stack.last_mut()
        {
            item.checked = Some(done);
        }
    }
}

fn append_span(spans: &mut Vec<Span>, span: Span) {
    if span.text.is_empty() {
        return;
    }
    if let Some(last) = spans.last_mut() {
        if last.bold == span.bold
            && last.italic == span.italic
            && last.strike == span.strike
            && last.code == span.code
            && last.link == span.link
            && last.href == span.href
        {
            last.text.push_str(&span.text);
            return;
        }
    }
    spans.push(span);
}

/// Parses markdown into the blocks this view draws.
pub fn parse(content: &str) -> Document {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_GFM);

    let mut builder = Builder::new();
    for event in Parser::new_ext(content, options) {
        match event {
            Event::Start(tag) => builder.start(tag),
            Event::End(tag) => builder.end(tag),
            Event::Text(text) => builder.text(&text),
            Event::Code(code) => builder.push_span(&code, true),
            Event::SoftBreak => builder.push_span(" ", false),
            Event::HardBreak => builder.push_span("\n", false),
            Event::Rule => builder.push_block(Block::Rule),
            Event::InlineMath(text) | Event::DisplayMath(text) => builder.push_span(&text, true),
            Event::TaskListMarker(done) => builder.task_marker(done),
            Event::FootnoteReference(label) => {
                builder.push_span(&format!("[^{label}]"), false);
            }
            // Raw HTML has no native renderer; the text inside it still arrives
            // as ordinary text events.
            Event::Html(_) | Event::InlineHtml(_) => {}
        }
    }
    Document {
        blocks: builder.doc,
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Draws a markdown document for reading, resolving relative image paths
/// against the file that holds it. `charts`, `mermaid`, `katex` turn those
/// fences into rendered views; off, they stay code.
pub fn render_preview(
    content: &str,
    file: &Path,
    theme: &Theme,
    charts: bool,
    mermaid: bool,
    katex: bool,
) -> Stateful<Div> {
    let doc = parse(content);
    let ctx = Ctx {
        file,
        theme,
        charts,
        mermaid,
        katex,
        next_id: Cell::new(0),
    };
    let mut inner = div().w_full().max_w(px(760.0)).flex().flex_col();
    for block in &doc.blocks {
        inner = inner.child(render_block(block, &ctx));
    }

    div()
        .id("preview-scroll")
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.0))
        .overflow_y_scroll()
        .bg(theme.bg)
        .text_size(px(14.0))
        .text_color(theme.fg)
        .child(
            div()
                .w_full()
                .flex()
                .flex_col()
                .items_center()
                .px(px(28.0))
                .py(px(18.0))
                .child(inner),
        )
}

/// What a block needs while it is drawn.
struct Ctx<'a> {
    file: &'a Path,
    theme: &'a Theme,
    charts: bool,
    mermaid: bool,
    katex: bool,
    /// Gives every link its own element id within one frame.
    next_id: Cell<usize>,
}

impl Ctx<'_> {
    fn next_id(&self) -> usize {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        id
    }
}

fn inline(spans: &[Span], ctx: &Ctx) -> AnyElement {
    let theme = ctx.theme;
    let mut text = String::new();
    let mut highlights = Vec::new();
    let mut links: Vec<(Range<usize>, String)> = Vec::new();
    for span in spans {
        let start = text.len();
        text.push_str(&span.text);
        let end = text.len();
        let mut style = HighlightStyle::default();
        let mut styled = false;
        if span.code {
            style.color = Some(theme.code.into());
            style.background_color = Some(theme.hover.into());
            styled = true;
        }
        if span.link {
            style.color = Some(theme.accent.into());
            style.underline = Some(UnderlineStyle {
                thickness: px(1.0),
                color: Some(theme.accent.into()),
                wavy: false,
            });
            styled = true;
            if let Some(href) = &span.href {
                links.push((start..end, href.clone()));
            }
        }
        if span.bold {
            style.font_weight = Some(FontWeight::SEMIBOLD);
            styled = true;
        }
        if span.italic {
            style.font_style = Some(FontStyle::Italic);
            styled = true;
        }
        if span.strike {
            style.strikethrough = Some(StrikethroughStyle {
                thickness: px(1.0),
                color: Some(theme.muted.into()),
            });
            styled = true;
        }
        if styled {
            highlights.push((start..end, style));
        }
    }
    let text = StyledText::new(SharedString::from(text));
    let text = if highlights.is_empty() {
        text
    } else {
        text.with_highlights(highlights)
    };
    if links.is_empty() {
        return text.into_any_element();
    }
    // Clicks report the index of the range they hit, so the URLs are kept in
    // the same order as the ranges.
    let (ranges, urls): (Vec<Range<usize>>, Vec<String>) = links.into_iter().unzip();
    let id = ctx.next_id();
    InteractiveText::new(SharedString::from(format!("md-inline-{id}")), text)
        .on_click(ranges, move |index, _window, cx| {
            if let Some(url) = urls.get(index) {
                cx.open_url(url);
            }
        })
        .into_any_element()
}

/// Token colours for a code block, or nothing when the fence names no
/// language the highlighter knows. Byte ranges are into `text`.
fn code_tokens(lang: &str, text: &str) -> Vec<(Range<usize>, TokenKind)> {
    let Some(language) = highlight::language_for_fence(lang) else {
        return Vec::new();
    };
    // Blocks are repainted every frame, so the same block is only parsed once.
    let key = {
        let mut hasher = DefaultHasher::new();
        language.hash(&mut hasher);
        text.hash(&mut hasher);
        hasher.finish()
    };
    if let Some(tokens) = CODE_TOKENS.with(|cache| cache.borrow().get(&key).cloned()) {
        return tokens;
    }
    let tokens = highlight::highlight(language, text);
    CODE_TOKENS.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() >= CODE_CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(key, tokens.clone());
    });
    tokens
}

const CODE_CACHE_LIMIT: usize = 256;

thread_local! {
    static CODE_TOKENS: RefCell<HashMap<u64, Vec<(Range<usize>, TokenKind)>>> =
        RefCell::new(HashMap::new());
}

/// A fenced block drawn as its source with a label, for diagram and maths
/// fences or error reporting.
fn source_card(label: impl Into<SharedString>, text: &str, theme: &Theme) -> Div {
    div()
        .w_full()
        .my_2()
        .rounded(px(6.0))
        .border_1()
        .border_color(theme.border)
        .bg(theme.surface)
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(
            div()
                .px_3()
                .pt(px(6.0))
                .text_size(px(11.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.muted)
                .child(label.into()),
        )
        .child(
            div()
                .w_full()
                .p_3()
                .font_family(MONO_FONT)
                .text_size(px(12.5))
                .text_color(theme.code)
                .child(SharedString::from(if text.is_empty() {
                    " ".to_string()
                } else {
                    text.to_string()
                })),
        )
}

fn render_preview_diagram(kind: crate::diagram_cache::DiagramKind, text: &str, ctx: &Ctx) -> Div {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        let msg = match kind {
            crate::diagram_cache::DiagramKind::Mermaid => "the Mermaid block is empty",
            crate::diagram_cache::DiagramKind::Math => "Empty math block",
        };
        return source_card(msg, text, ctx.theme);
    }
    match crate::diagram_cache::get_or_render(kind, text, ctx.theme) {
        Ok(diagram) => {
            let (w, h) = crate::diagram_cache::scale_diagram_size(diagram.size, 760.0);
            div()
                .w_full()
                .my_2()
                .flex()
                .justify_center()
                .child(img(diagram.path).w(px(w)).h(px(h)).rounded(px(6.0)))
        }
        Err(err) => {
            let label = format!(
                "{} error: {}",
                match kind {
                    crate::diagram_cache::DiagramKind::Mermaid => "Mermaid",
                    crate::diagram_cache::DiagramKind::Math => "LaTeX",
                },
                err
            );
            source_card(label, text, ctx.theme)
        }
    }
}

fn code_block(lang: &str, text: &str, theme: &Theme) -> Div {
    let mut block = div()
        .w_full()
        .my_2()
        .rounded(px(6.0))
        .bg(theme.hover)
        .flex()
        .flex_col()
        .overflow_hidden();
    if !lang.is_empty() {
        block = block.child(
            div()
                .px_3()
                .pt(px(6.0))
                .text_size(px(11.0))
                .text_color(theme.muted)
                .child(SharedString::from(lang.to_string())),
        );
    }
    let shown = if text.is_empty() {
        " ".to_string()
    } else {
        text.to_string()
    };
    let mut highlights = Vec::new();
    for (range, kind) in code_tokens(lang, &shown) {
        if kind == TokenKind::Plain {
            continue;
        }
        let (bold, italic) = highlight::emphasis(kind);
        let mut style = HighlightStyle {
            color: Some(highlight::color(kind, theme)),
            ..Default::default()
        };
        if bold {
            style.font_weight = Some(FontWeight::SEMIBOLD);
        }
        if italic {
            style.font_style = Some(FontStyle::Italic);
        }
        highlights.push((range, style));
    }
    let mut body = StyledText::new(SharedString::from(shown));
    if !highlights.is_empty() {
        body = body.with_highlights(highlights);
    }
    block.child(
        div()
            .w_full()
            .p_3()
            .font_family(MONO_FONT)
            .text_size(px(12.5))
            .text_color(theme.code)
            .child(body),
    )
}

fn render_block(block: &Block, ctx: &Ctx) -> Div {
    let theme = ctx.theme;
    match block {
        Block::Heading { level, spans } => {
            let size = match level {
                1 => 26.0,
                2 => 21.0,
                3 => 17.0,
                4 => 15.0,
                5 => 14.0,
                _ => 13.0,
            };
            div()
                .w_full()
                .mt(px(if *level <= 2 { 18.0 } else { 12.0 }))
                .mb(px(4.0))
                .text_size(px(size))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.heading)
                .child(inline(spans, ctx))
        }
        Block::Paragraph { spans } => div().w_full().py(px(3.0)).child(inline(spans, ctx)),
        Block::Code { lang, text } => match lang.to_ascii_lowercase().as_str() {
            "mermaid" if ctx.mermaid => {
                render_preview_diagram(crate::diagram_cache::DiagramKind::Mermaid, text, ctx)
            }
            "math" | "latex" | "katex" | "tex" if ctx.katex => {
                render_preview_diagram(crate::diagram_cache::DiagramKind::Math, text, ctx)
            }
            "chart" | "chartjs" if ctx.charts => {
                div().w_full().my_2().child(chart::render(text, theme))
            }
            _ => code_block(lang, text, theme),
        },
        Block::Quote { kind, blocks } => {
            let mut quote = div()
                .w_full()
                .my_2()
                .pl_3()
                .border_l_2()
                .border_color(theme.accent)
                .text_color(theme.quote)
                .flex()
                .flex_col();
            if let Some(kind) = kind {
                let (label, colour) = match kind {
                    Callout::Note => ("Note", theme.accent),
                    Callout::Tip => ("Tip", theme.accent),
                    Callout::Important => ("Important", theme.accent),
                    Callout::Warning => ("Warning", theme.danger),
                    Callout::Caution => ("Caution", theme.danger),
                };
                quote = quote.child(
                    div()
                        .pt(px(2.0))
                        .text_size(px(11.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(colour)
                        .child(label),
                );
            }
            for block in blocks {
                quote = quote.child(render_block(block, ctx));
            }
            quote
        }
        Block::List {
            ordered,
            start,
            items,
        } => {
            let mut list = div().w_full().my_1().flex().flex_col();
            for (index, item) in items.iter().enumerate() {
                let marker = if let Some(done) = item.checked {
                    if done {
                        "☑".to_string()
                    } else {
                        "☐".to_string()
                    }
                } else if *ordered {
                    format!("{}.", start + index as u64)
                } else {
                    "•".to_string()
                };
                let marker = div()
                    .flex_shrink_0()
                    .min_w(px(18.0))
                    .text_align(TextAlign::Right)
                    .text_color(if item.checked == Some(true) {
                        theme.accent
                    } else {
                        theme.muted
                    })
                    .child(SharedString::from(marker));

                let mut content = div().flex_1().min_w(px(0.0)).flex().flex_col();
                for block in &item.blocks {
                    content = content.child(render_block(block, ctx));
                }
                if item.blocks.is_empty() {
                    content = content.child(div().h(px(22.0)));
                }

                list = list.child(
                    div()
                        .w_full()
                        .flex()
                        .flex_row()
                        .items_start()
                        .gap(px(8.0))
                        .py(px(1.0))
                        .child(marker)
                        .child(content),
                );
            }
            list
        }
        Block::Table {
            aligns,
            header,
            rows,
        } => {
            let columns = aligns
                .len()
                .max(header.len())
                .max(rows.iter().map(Vec::len).max().unwrap_or(0));
            let mut table = div()
                .w_full()
                .my_3()
                .border_1()
                .border_color(theme.border)
                .rounded(px(6.0))
                .overflow_hidden()
                .flex()
                .flex_col();

            let push_row = |table: Div, cells: &[Vec<Span>], is_head: bool| {
                let mut row = div()
                    .w_full()
                    .flex()
                    .flex_row()
                    .bg(if is_head { theme.hover } else { theme.bg })
                    .border_b_1()
                    .border_color(theme.border);
                for index in 0..columns {
                    let empty = Vec::new();
                    let cell = cells.get(index).unwrap_or(&empty);
                    let align = aligns.get(index).copied().unwrap_or(Align::Left);
                    let mut block = div()
                        .flex_1()
                        .min_w(px(0.0))
                        .px_2()
                        .py(px(5.0))
                        .text_size(px(13.5))
                        .text_align(match align {
                            Align::Left => TextAlign::Left,
                            Align::Center => TextAlign::Center,
                            Align::Right => TextAlign::Right,
                        })
                        .when(is_head, |el| el.font_weight(FontWeight::SEMIBOLD))
                        .when(index + 1 < columns, |el| {
                            el.border_r_1().border_color(theme.border)
                        });
                    if cell.is_empty() {
                        block = block.child(SharedString::from(" "));
                    } else {
                        block = block.child(inline(cell, ctx));
                    }
                    row = row.child(block);
                }
                table.child(row)
            };

            if !header.is_empty() {
                table = push_row(table, header, true);
            }
            for row in rows {
                table = push_row(table, row, false);
            }
            table
        }
        Block::Rule => div().w_full().my_3().h(px(1.0)).bg(theme.border),
        Block::Image { src, alt } => match resolve_image(src, ctx.file) {
            Some(path) => div()
                .my_2()
                .child(gpui::img(path).max_w(px(720.0)).rounded(px(4.0))),
            None => div()
                .my_2()
                .px_3()
                .py_2()
                .rounded(px(6.0))
                .bg(theme.hover)
                .text_size(px(12.0))
                .text_color(theme.muted)
                .child(SharedString::from(if alt.is_empty() {
                    format!("[image] {src}")
                } else {
                    format!("[image] {alt}")
                })),
        },
    }
}

/// A local image path, relative to the markdown file. Remote images are not
/// fetched; the alt text stands in for them.
fn resolve_image(src: &str, file: &Path) -> Option<PathBuf> {
    if src.starts_with("http://") || src.starts_with("https://") || src.starts_with("data:") {
        return None;
    }
    let trimmed = src.split(['#', '?']).next().unwrap_or(src);
    let decoded = trimmed.replace("%20", " ");
    let path = Path::new(&decoded);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        file.parent()?.join(path)
    };
    path.exists().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(spans: &[Span]) -> String {
        spans.iter().map(|span| span.text.as_str()).collect()
    }

    #[test]
    fn headings_keep_their_level_and_inline_marks() {
        let doc = parse("# Title **bold** and *soft*");
        match &doc.blocks[0] {
            Block::Heading { level, spans } => {
                assert_eq!(*level, 1);
                assert_eq!(text_of(spans), "Title bold and soft");
                assert!(spans.iter().any(|span| span.bold && span.text == "bold"));
                assert!(spans.iter().any(|span| span.italic && span.text == "soft"));
            }
            other => panic!("expected a heading, got {other:?}"),
        }
    }

    #[test]
    fn inline_code_links_and_strikethrough_are_marked() {
        let doc = parse("Use `pv` and [docs](https://example.com) ~~old~~");
        match &doc.blocks[0] {
            Block::Paragraph { spans } => {
                assert_eq!(text_of(spans), "Use pv and docs old");
                assert!(spans.iter().any(|span| span.code && span.text == "pv"));
                assert!(spans.iter().any(|span| span.link && span.text == "docs"));
                assert!(spans.iter().any(|span| span.strike && span.text == "old"));
            }
            other => panic!("expected a paragraph, got {other:?}"),
        }
    }

    #[test]
    fn lists_are_ordered_nested_and_checked() {
        let doc = parse("- one\n- two\n  1. nested\n\n- [x] done\n- [ ] todo");
        let Block::List { ordered, items, .. } = &doc.blocks[0] else {
            panic!("expected a list, got {:?}", doc.blocks[0]);
        };
        assert!(!ordered);
        assert_eq!(items.len(), 4);
        assert!(matches!(
            items[1].blocks[1],
            Block::List { ordered: true, .. }
        ));
        assert_eq!(items[2].checked, Some(true));
        assert_eq!(items[3].checked, Some(false));
    }

    #[test]
    fn tables_keep_alignment_header_and_cells() {
        let doc = parse("| Name | Qty |\n| :--- | ---: |\n| Tea | 2 |\n| Jam | 10 |");
        let Block::Table {
            aligns,
            header,
            rows,
        } = &doc.blocks[0]
        else {
            panic!("expected a table, got {:?}", doc.blocks[0]);
        };
        assert_eq!(aligns, &vec![Align::Left, Align::Right]);
        assert_eq!(text_of(&header[0]), "Name");
        assert_eq!(rows.len(), 2);
        assert_eq!(text_of(&rows[1][1]), "10");
    }

    #[test]
    fn code_fences_keep_language_and_indentation() {
        let doc = parse("```rust\nfn main() {\n    run();\n}\n```");
        match &doc.blocks[0] {
            Block::Code { lang, text } => {
                assert_eq!(lang, "rust");
                assert_eq!(text, "fn main() {\n    run();\n}");
            }
            other => panic!("expected code, got {other:?}"),
        }
    }

    #[test]
    fn quotes_images_and_rules_are_blocks() {
        let doc = parse("> quoted **text**\n\n---\n\n![alt](/tmp/nowhere.png)");
        assert!(matches!(doc.blocks[0], Block::Quote { .. }));
        assert!(matches!(doc.blocks[1], Block::Rule));
        match &doc.blocks[2] {
            Block::Image { src, alt } => {
                assert_eq!(src, "/tmp/nowhere.png");
                assert_eq!(alt, "alt");
            }
            other => panic!("expected an image, got {other:?}"),
        }
    }

    #[test]
    fn tight_list_parent_text_is_not_merged_into_its_nested_list() {
        let doc = parse("- parent\n  - child one\n  - child two\n- sibling");
        let Block::List { items, .. } = &doc.blocks[0] else {
            panic!("expected a list, got {:?}", doc.blocks[0]);
        };
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].blocks.len(), 2);
        match &items[0].blocks[0] {
            Block::Paragraph { spans } => assert_eq!(text_of(spans), "parent"),
            other => panic!("expected the parent text first, got {other:?}"),
        }
        let Block::List { items: nested, .. } = &items[0].blocks[1] else {
            panic!("expected the nested list, got {:?}", items[0].blocks[1]);
        };
        assert_eq!(nested.len(), 2);
        match &nested[0].blocks[0] {
            Block::Paragraph { spans } => assert_eq!(text_of(spans), "child one"),
            other => panic!("expected the first child, got {other:?}"),
        }
        assert_eq!(text_of_item(&items[1]), "sibling");
    }

    fn text_of_item(item: &ListItem) -> String {
        match &item.blocks[0] {
            Block::Paragraph { spans } => text_of(spans),
            other => panic!("expected a paragraph, got {other:?}"),
        }
    }

    #[test]
    fn links_keep_their_destination_and_stay_apart_from_plain_text() {
        let doc = parse("See [docs](https://a.example) and [more](https://b.example).");
        let Block::Paragraph { spans } = &doc.blocks[0] else {
            panic!("expected a paragraph, got {:?}", doc.blocks[0]);
        };
        let docs = spans.iter().find(|span| span.text == "docs").unwrap();
        assert!(docs.link);
        assert_eq!(docs.href.as_deref(), Some("https://a.example"));
        let more = spans.iter().find(|span| span.text == "more").unwrap();
        assert_eq!(more.href.as_deref(), Some("https://b.example"));
        assert!(spans
            .iter()
            .filter(|span| !span.link)
            .all(|span| span.href.is_none()));
    }

    #[test]
    fn adjacent_links_with_different_targets_do_not_merge() {
        let doc = parse("[a](https://one.example)[b](https://two.example)");
        let Block::Paragraph { spans } = &doc.blocks[0] else {
            panic!("expected a paragraph");
        };
        let links: Vec<_> = spans.iter().filter(|span| span.link).collect();
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].href.as_deref(), Some("https://one.example"));
        assert_eq!(links[1].href.as_deref(), Some("https://two.example"));
    }

    #[test]
    fn fenced_code_is_tokenised_for_known_languages_only() {
        let tokens = code_tokens("rust", "fn main() {}");
        assert!(
            tokens
                .iter()
                .any(|(range, kind)| *kind == TokenKind::Keyword
                    && &"fn main() {}"[range.clone()] == "fn"),
            "expected `fn` to be a keyword, got {tokens:?}"
        );
        assert!(code_tokens("no-such-language", "x = 1").is_empty());
        assert!(code_tokens("", "plain").is_empty());
    }

    #[test]
    fn render_preview_with_diagrams_and_math() {
        let theme = Theme::for_name("system", gpui::WindowAppearance::Dark);
        let path = Path::new("/test/doc.md");
        let content = "# Title\n\n```mermaid\nflowchart LR\n    A --> B\n```\n\n```latex\ne^{i\\pi}+1=0\n```\n";

        // Enabled
        let _ = render_preview(content, path, &theme, true, true, true);

        // Disabled
        let _ = render_preview(content, path, &theme, false, false, false);
    }

    #[test]
    fn render_preview_with_broken_diagrams() {
        let theme = Theme::for_name("system", gpui::WindowAppearance::Dark);
        let path = Path::new("/test/doc.md");
        let content = "```mermaid\nthis is invalid mermaid\n```\n\n```latex\n\\invalidcmd\n```\n";
        let _ = render_preview(content, path, &theme, true, true, true);
    }
}
