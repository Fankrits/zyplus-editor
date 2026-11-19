#![allow(dead_code)]

//! Pure markdown model behind the rich (WYSIWYG) editor.
//!
//! The document stays one markdown source string. This module answers three
//! questions about it without touching the text:
//!
//! * what kind of line each source line is (heading, list item, quote, code…),
//!   with the byte range of its block markers and of its visible content;
//! * which inline spans (emphasis, links, code…) exist, with their delimiter
//!   ranges, so the editor can conceal the delimiters while the caret is away;
//! * what a line looks like on screen (`display_line`), with a two-way offset
//!   map between the displayed text and the source.
//!
//! Every offset is a byte offset into the source on a char boundary. Lines are
//! split on `\n`, `\r\n` and lone `\r` (the terminator is never in `Line.range`).
//! A trailing newline produces a final empty line, so every offset in the
//! document, including `src.len()`, belongs to some line.

use std::cmp::Reverse;
use std::collections::HashMap;
use std::ops::Range;

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, LinkType, Options, Parser, Tag};

// ---------------------------------------------------------------------------
// Public model
// ---------------------------------------------------------------------------

/// GitHub-style alert kinds (`> [!NOTE]` and friends).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CalloutKind {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

/// What a source line is. Block markers are not part of the visible text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LineKind {
    Blank,
    Paragraph,
    /// ATX (`# x`) or setext heading text line, level 1..=6.
    Heading(u8),
    /// The `===` / `---` line that closes a setext heading.
    SetextUnderline,
    /// `depth` is the number of enclosing block quotes. `callout` is set on
    /// every line of a quote whose first line is `[!KIND]`.
    Quote {
        depth: u8,
        callout: Option<CalloutKind>,
    },
    /// The first line of a list item. `indent` is the list nesting level
    /// (0 for a top-level item). `number` is the source number for ordered
    /// items and 0 for bullets.
    ListItem {
        indent: usize,
        ordered: bool,
        number: u64,
        task: Option<bool>,
    },
    /// A later line of a list item (paragraph continuation, lazy line, …).
    ListContinuation {
        indent: usize,
    },
    /// The opening fence. `lang` is the first word of the info string.
    FenceOpen {
        lang: String,
    },
    CodeLine,
    FenceClose,
    IndentedCode,
    Rule,
    /// The header row of a GFM table.
    TableHeader,
    TableDelimiter,
    TableRow,
    Html,
    /// YAML / TOML front matter, including its `---` delimiters.
    FrontMatter,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InlineKind {
    Strong,
    Emphasis,
    Strike,
    Code,
    /// `url` is the destination inside `](…)`. For reference links it is an
    /// empty range at the closing bracket.
    Link {
        url: Range<usize>,
    },
    /// Same `url` convention as `Link`. Its alt text is the content.
    Image {
        url: Range<usize>,
    },
    /// `<https://…>` or `<a@b.c>`. Content is the address.
    AutoLink,
    /// A single raw HTML tag such as `<br>` or `</b>`. Never concealed.
    HtmlInline,
}

/// An inline span. Ranges are absolute source offsets.
///
/// `range` covers everything. `open` and `close` are the delimiter bytes at
/// each end (empty ranges when there is none). The visible content is the
/// bytes between them: `open.end..close.start`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inline {
    pub kind: InlineKind,
    pub range: Range<usize>,
    pub open: Range<usize>,
    pub close: Range<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// The line's bytes without its terminator.
    pub range: Range<usize>,
    /// First source byte of visible text after block markers and indentation.
    /// Always within `range` (equal to `range.end` when nothing is visible).
    pub content_start: usize,
    /// Block-marker bytes: from the first non-blank byte up to `content_start`.
    /// Empty when the line has no marker.
    pub marker: Range<usize>,
    pub kind: LineKind,
    /// Inlines that intersect this line, sorted by start (outer first on ties).
    /// A multi-line inline appears on every line it touches.
    pub inlines: Vec<Inline>,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Model {
    pub lines: Vec<Line>,
    /// `true` for setext heading text lines (the lines above a `===`/`---`).
    /// Kept private so block grouping can tell them apart from ATX headings.
    setext: Vec<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockKind {
    Paragraph,
    Heading(u8),
    Quote,
    ListItem,
    CodeBlock { lang: String },
    Table,
    Rule,
    Html,
    Callout(CalloutKind),
    Blank,
    FrontMatter,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub kind: BlockKind,
    /// Indices into `Model.lines`.
    pub lines: Range<usize>,
}

/// Style of a displayed byte range. Nested inlines combine as flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InlineStyle {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    pub link: bool,
    /// A revealed marker (the delimiter the caret is touching). Draw muted.
    pub marker: bool,
}

/// What a line looks like on screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayLine {
    /// Visible text, with concealed delimiters removed.
    pub text: String,
    /// `to_source[k]` is the source byte offset of display byte `k`.
    /// Length is `text.len() + 1`; the last entry is the source offset just
    /// after the last visible byte (or the content start when empty).
    /// Non-decreasing (strictly increasing for visible bytes).
    pub to_source: Vec<usize>,
    /// Non-default styles as display byte ranges. Bytes not listed are plain.
    pub styles: Vec<(Range<usize>, InlineStyle)>,
}

impl Model {
    /// Groups consecutive lines of the same construct.
    pub fn blocks(&self) -> Vec<Block> {
        let lines = &self.lines;
        let n = lines.len();
        let mut out = Vec::new();
        let mut i = 0;
        while i < n {
            let (kind, end) = match &lines[i].kind {
                LineKind::Blank => (BlockKind::Blank, run(lines, i, |k| *k == LineKind::Blank)),
                LineKind::Paragraph | LineKind::SetextUnderline => (
                    BlockKind::Paragraph,
                    run(lines, i, |k| *k == LineKind::Paragraph),
                ),
                LineKind::Heading(level) => {
                    if self.setext.get(i) == Some(&true) {
                        let mut j = i;
                        while j < n && self.setext[j] {
                            j += 1;
                        }
                        let end = if j < n && lines[j].kind == LineKind::SetextUnderline {
                            j + 1
                        } else {
                            j
                        };
                        (BlockKind::Heading(*level), end)
                    } else {
                        (BlockKind::Heading(*level), i + 1)
                    }
                }
                LineKind::Quote { callout, .. } => {
                    let callout = *callout;
                    let mut j = i + 1;
                    while j < n {
                        match &lines[j].kind {
                            LineKind::Quote { callout: c, .. } if *c == callout => j += 1,
                            _ => break,
                        }
                    }
                    let kind = match callout {
                        Some(k) => BlockKind::Callout(k),
                        None => BlockKind::Quote,
                    };
                    (kind, j)
                }
                LineKind::ListItem { .. } | LineKind::ListContinuation { .. } => (
                    BlockKind::ListItem,
                    run(lines, i + 1, |k| {
                        matches!(k, LineKind::ListContinuation { .. })
                    }),
                ),
                LineKind::FenceOpen { lang } => {
                    let lang = lang.clone();
                    let mut j = i + 1;
                    while j < n && lines[j].kind == LineKind::CodeLine {
                        j += 1;
                    }
                    let end = if j < n && lines[j].kind == LineKind::FenceClose {
                        j + 1
                    } else {
                        j
                    };
                    (BlockKind::CodeBlock { lang }, end)
                }
                LineKind::IndentedCode => (
                    BlockKind::CodeBlock {
                        lang: String::new(),
                    },
                    run(lines, i, |k| *k == LineKind::IndentedCode),
                ),
                LineKind::CodeLine | LineKind::FenceClose => (
                    BlockKind::CodeBlock {
                        lang: String::new(),
                    },
                    i + 1,
                ),
                LineKind::Rule => (BlockKind::Rule, i + 1),
                LineKind::TableHeader | LineKind::TableDelimiter | LineKind::TableRow => (
                    BlockKind::Table,
                    run(lines, i, |k| {
                        matches!(
                            k,
                            LineKind::TableHeader | LineKind::TableDelimiter | LineKind::TableRow
                        )
                    }),
                ),
                LineKind::Html => (BlockKind::Html, run(lines, i, |k| *k == LineKind::Html)),
                LineKind::FrontMatter => (
                    BlockKind::FrontMatter,
                    run(lines, i, |k| *k == LineKind::FrontMatter),
                ),
            };
            let end = end.max(i + 1).min(n);
            out.push(Block {
                kind,
                lines: i..end,
            });
            i = end;
        }
        out
    }

    /// Index of the line containing byte `offset`. Offsets past the end map
    /// to the last line. An offset on a terminator belongs to the line it ends.
    pub fn line_index_at(&self, offset: usize) -> usize {
        if self.lines.is_empty() {
            return 0;
        }
        self.lines
            .partition_point(|l| l.range.start <= offset)
            .saturating_sub(1)
    }
}

fn run(lines: &[Line], from: usize, pred: impl Fn(&LineKind) -> bool) -> usize {
    let mut j = from;
    while j < lines.len() && pred(&lines[j].kind) {
        j += 1;
    }
    j
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContKind {
    Quote,
    Item,
}

#[derive(Clone, Copy, Debug, Default)]
struct ItemInfo {
    /// Columns of indentation a continuation line may have, measured from the
    /// start of the item's own text (after any enclosing quote markers).
    content_indent: usize,
    ordered: bool,
    number: u64,
    task: Option<bool>,
}

struct Container {
    kind: ContKind,
    range: Range<usize>,
}

enum LeafKind {
    Paragraph,
    Heading(u8),
    Fenced(String),
    Indented,
    Table,
    Html,
    Rule,
    FrontMatter,
}

struct Leaf {
    range: Range<usize>,
    kind: LeafKind,
    first: usize,
    last: usize,
}

enum RawKind {
    Emphasis,
    Strong,
    Strike,
    Code,
    Html,
    Link(LinkType),
    Image(LinkType),
}

struct Raw {
    kind: RawKind,
    range: Range<usize>,
}

struct Frame {
    kind: ContKind,
    end: usize,
    start_line: usize,
    callout: Option<CalloutKind>,
    item: ItemInfo,
}

/// Parses markdown into lines, kinds, markers, inlines.
pub fn parse(src: &str) -> Model {
    let ranges = split_lines(src);
    let n = ranges.len();
    let starts: Vec<usize> = ranges.iter().map(|r| r.start).collect();
    let bytes = src.as_bytes();

    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_GFM);
    options.insert(Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);
    options.insert(Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS);

    let mut leaves: Vec<Leaf> = Vec::new();
    let mut containers: Vec<Container> = Vec::new();
    let mut raws: Vec<Raw> = Vec::new();
    let mut tasks: Vec<(usize, bool)> = Vec::new();

    for (event, range) in Parser::new_ext(src, options).into_offset_iter() {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => leaves.push(leaf_of(range, LeafKind::Paragraph)),
                Tag::Heading { level, .. } => {
                    leaves.push(leaf_of(range, LeafKind::Heading(heading_level(level))))
                }
                Tag::BlockQuote(_) => containers.push(Container {
                    kind: ContKind::Quote,
                    range,
                }),
                Tag::Item => containers.push(Container {
                    kind: ContKind::Item,
                    range,
                }),
                Tag::CodeBlock(CodeBlockKind::Fenced(info)) => {
                    let lang = info.split_whitespace().next().unwrap_or("").to_string();
                    leaves.push(leaf_of(range, LeafKind::Fenced(lang)))
                }
                Tag::CodeBlock(CodeBlockKind::Indented) => {
                    leaves.push(leaf_of(range, LeafKind::Indented))
                }
                Tag::Table(_) => leaves.push(leaf_of(range, LeafKind::Table)),
                Tag::HtmlBlock => leaves.push(leaf_of(range, LeafKind::Html)),
                Tag::MetadataBlock(_) => leaves.push(leaf_of(range, LeafKind::FrontMatter)),
                Tag::Emphasis => raws.push(Raw {
                    kind: RawKind::Emphasis,
                    range,
                }),
                Tag::Strong => raws.push(Raw {
                    kind: RawKind::Strong,
                    range,
                }),
                Tag::Strikethrough => raws.push(Raw {
                    kind: RawKind::Strike,
                    range,
                }),
                Tag::Link { link_type, .. } => raws.push(Raw {
                    kind: RawKind::Link(link_type),
                    range,
                }),
                Tag::Image { link_type, .. } => raws.push(Raw {
                    kind: RawKind::Image(link_type),
                    range,
                }),
                _ => {}
            },
            Event::Rule => leaves.push(leaf_of(range, LeafKind::Rule)),
            Event::Code(_) => raws.push(Raw {
                kind: RawKind::Code,
                range,
            }),
            Event::InlineHtml(_) => raws.push(Raw {
                kind: RawKind::Html,
                range,
            }),
            Event::TaskListMarker(done) => tasks.push((range.start, done)),
            _ => {}
        }
    }
    containers.sort_by_key(|c| c.range.start);
    tasks.sort_by_key(|t| t.0);

    // Map each line to the leaf block that covers it (leaves never overlap).
    let mut leaf_of_line: Vec<Option<usize>> = vec![None; n];
    for (k, leaf) in leaves.iter_mut().enumerate() {
        let first = line_index(&starts, leaf.range.start);
        let last = line_index(
            &starts,
            leaf.range.end.saturating_sub(1).max(leaf.range.start),
        )
        .max(first);
        leaf.first = first;
        leaf.last = last;
        for slot in leaf_of_line.iter_mut().take(last + 1).skip(first) {
            *slot = Some(k);
        }
    }

    // Sweep the lines once, keeping the stack of containers that cover each line.
    let mut fence_open: Vec<Option<(u8, usize)>> = vec![None; leaves.len()];
    let mut setext = vec![false; n];
    let mut stack: Vec<Frame> = Vec::new();
    let mut next_c = 0usize;
    let mut lines: Vec<Line> = Vec::with_capacity(n);

    for i in 0..n {
        let rs = ranges[i].start;
        let re = ranges[i].end;

        stack.retain(|f| f.end > rs);
        while next_c < containers.len() && containers[next_c].range.start <= re {
            let c = &containers[next_c];
            next_c += 1;
            if c.range.end > rs {
                stack.push(Frame {
                    kind: c.kind,
                    end: c.range.end,
                    start_line: i,
                    callout: None,
                    item: ItemInfo::default(),
                });
            }
        }

        // Walk the container markers that start this line, outermost first.
        let mut pos = rs;
        let mut last_quote_end = rs;
        let mut alert_end: Option<usize> = None;
        let mut item_here: Option<ItemInfo> = None;
        for idx in 0..stack.len() {
            let started = stack[idx].start_line == i;
            match stack[idx].kind {
                ContKind::Quote => {
                    let p = skip_ws(bytes, pos, re);
                    if p < re && bytes[p] == b'>' {
                        let mut q = p + 1;
                        if q < re && (bytes[q] == b' ' || bytes[q] == b'\t') {
                            q += 1;
                        }
                        pos = q;
                        last_quote_end = q;
                        if started {
                            if let Some((kind, end)) = match_alert(bytes, q, re) {
                                stack[idx].callout = Some(kind);
                                alert_end = Some(end);
                            }
                        }
                    } else {
                        // Lazy continuation: the quote's marker is absent here.
                        break;
                    }
                }
                ContKind::Item => {
                    let p = skip_ws(bytes, pos, re);
                    if started {
                        let Some((ordered, number, after)) = parse_list_marker(bytes, p, re) else {
                            break;
                        };
                        let mut q = skip_ws(bytes, after, re);
                        let mut task = None;
                        if let Ok(t) = tasks.binary_search_by_key(&q, |t| t.0) {
                            if task_box_at(bytes, q, re) {
                                task = Some(tasks[t].1);
                                q = skip_ws(bytes, q + 3, re);
                            }
                        }
                        let info = ItemInfo {
                            content_indent: q.saturating_sub(last_quote_end),
                            ordered,
                            number,
                            task,
                        };
                        stack[idx].item = info;
                        item_here = Some(info);
                        pos = q;
                    } else {
                        pos = p;
                    }
                }
            }
        }
        let text_start = skip_ws(bytes, pos, re);

        let depth = stack
            .iter()
            .filter(|f| f.kind == ContKind::Quote)
            .count()
            .min(255) as u8;
        let items = stack.iter().filter(|f| f.kind == ContKind::Item).count();
        let innermost = stack.last().map(|f| f.kind);
        let callout = stack.iter().rev().find_map(|f| {
            if f.kind == ContKind::Quote {
                f.callout
            } else {
                None
            }
        });

        let leaf_idx = leaf_of_line[i];
        // The opening fence's run is needed to recognise the closing fence,
        // even when the opening line is also a list item line.
        if let Some(k) = leaf_idx {
            if i == leaves[k].first && matches!(leaves[k].kind, LeafKind::Fenced(_)) {
                fence_open[k] = fence_run(bytes, text_start, re);
            }
        }
        let (kind, cstart) = if let Some(info) = item_here {
            (
                LineKind::ListItem {
                    indent: items.saturating_sub(1),
                    ordered: info.ordered,
                    number: info.number,
                    task: info.task,
                },
                text_start,
            )
        } else if let Some(k) = leaf_idx {
            let leaf = &leaves[k];
            let multi = leaf.last > leaf.first;
            match &leaf.kind {
                LeafKind::Heading(level) => {
                    if multi && i == leaf.last {
                        (LineKind::SetextUnderline, text_start)
                    } else if multi {
                        setext[i] = true;
                        (LineKind::Heading(*level), text_start)
                    } else {
                        (
                            LineKind::Heading(*level),
                            atx_content(bytes, text_start, re),
                        )
                    }
                }
                LeafKind::Fenced(lang) => {
                    if i == leaf.first {
                        (LineKind::FenceOpen { lang: lang.clone() }, text_start)
                    } else {
                        let closing = i == leaf.last
                            && match fence_open[k] {
                                Some((ch, len)) => is_closing_fence(bytes, text_start, re, ch, len),
                                None => false,
                            };
                        if closing {
                            (LineKind::FenceClose, text_start)
                        } else {
                            (LineKind::CodeLine, code_walk(bytes, rs, re, &stack))
                        }
                    }
                }
                LeafKind::Indented => {
                    let p = strip_cols(bytes, code_walk(bytes, rs, re, &stack), re, 4);
                    (LineKind::IndentedCode, p)
                }
                LeafKind::Table => {
                    let kind = if i == leaf.first {
                        LineKind::TableHeader
                    } else if i == leaf.first + 1 {
                        LineKind::TableDelimiter
                    } else {
                        LineKind::TableRow
                    };
                    (kind, rs)
                }
                LeafKind::Html => (LineKind::Html, text_start),
                LeafKind::Rule => (LineKind::Rule, text_start),
                LeafKind::FrontMatter => (LineKind::FrontMatter, rs),
                LeafKind::Paragraph => {
                    paragraph_like(innermost, depth, items, callout, alert_end, text_start)
                }
            }
        } else if text_start == re && depth == 0 {
            (LineKind::Blank, re)
        } else {
            paragraph_like(innermost, depth, items, callout, alert_end, text_start)
        };

        let first_nonws = skip_ws(bytes, rs, re);
        let cstart = cstart.clamp(rs, re);
        let marker = if first_nonws <= cstart {
            first_nonws..cstart
        } else {
            cstart..cstart
        };
        lines.push(Line {
            range: rs..re,
            content_start: cstart,
            marker,
            kind,
            inlines: Vec::new(),
        });
    }

    // Inlines: build spans once, then attach each to the lines it touches.
    let inlines = finish_inlines(src, &raws);
    for inl in inlines {
        let first = line_index(&starts, inl.range.start);
        let last = line_index(
            &starts,
            inl.range.end.saturating_sub(1).max(inl.range.start),
        )
        .min(n - 1);
        for line in lines.iter_mut().take(last + 1).skip(first) {
            line.inlines.push(inl.clone());
        }
    }

    Model { lines, setext }
}

fn leaf_of(range: Range<usize>, kind: LeafKind) -> Leaf {
    Leaf {
        range,
        kind,
        first: 0,
        last: 0,
    }
}

fn paragraph_like(
    innermost: Option<ContKind>,
    depth: u8,
    items: usize,
    callout: Option<CalloutKind>,
    alert_end: Option<usize>,
    text_start: usize,
) -> (LineKind, usize) {
    match innermost {
        Some(ContKind::Quote) => (
            LineKind::Quote { depth, callout },
            alert_end.unwrap_or(text_start),
        ),
        Some(ContKind::Item) => (
            LineKind::ListContinuation {
                indent: items.saturating_sub(1),
            },
            text_start,
        ),
        None => (LineKind::Paragraph, text_start),
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// Splits on `\n`, `\r\n` and lone `\r`. Always returns at least one line; a
/// trailing terminator yields a final empty line.
fn split_lines(src: &str) -> Vec<Range<usize>> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\n' => {
                out.push(start..i);
                start = i + 1;
            }
            b'\r' => {
                out.push(start..i);
                if i + 1 < b.len() && b[i + 1] == b'\n' {
                    i += 1;
                }
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out.push(start..b.len());
    out
}

/// Index of the line containing `offset`, given sorted line starts.
fn line_index(starts: &[usize], offset: usize) -> usize {
    starts.partition_point(|&s| s <= offset).saturating_sub(1)
}

fn skip_ws(b: &[u8], mut p: usize, end: usize) -> usize {
    while p < end && (b[p] == b' ' || b[p] == b'\t') {
        p += 1;
    }
    p
}

/// Skips up to `cols` columns of indentation (tab = 4 columns).
fn strip_cols(b: &[u8], mut p: usize, end: usize, cols: usize) -> usize {
    let mut col = 0;
    while p < end && col < cols {
        match b[p] {
            b' ' => {
                p += 1;
                col += 1;
            }
            b'\t' if col + 4 <= cols => {
                p += 1;
                col += 4;
            }
            _ => break,
        }
    }
    p
}

/// Start of the code text on a code line: quote markers and list-item
/// indentation are removed, leading spaces of the code itself are kept.
fn code_walk(b: &[u8], rs: usize, re: usize, stack: &[Frame]) -> usize {
    let mut pos = rs;
    for f in stack {
        match f.kind {
            ContKind::Quote => {
                let p = skip_ws(b, pos, re);
                if p < re && b[p] == b'>' {
                    pos = p + 1;
                    if pos < re && (b[pos] == b' ' || b[pos] == b'\t') {
                        pos += 1;
                    }
                } else {
                    break;
                }
            }
            ContKind::Item => pos = strip_cols(b, pos, re, f.item.content_indent),
        }
    }
    pos
}

fn atx_content(b: &[u8], text_start: usize, re: usize) -> usize {
    let mut p = text_start;
    while p < re && b[p] == b'#' {
        p += 1;
    }
    skip_ws(b, p, re)
}

/// Fence character and run length if a fence starts at `p`.
fn fence_run(b: &[u8], p: usize, re: usize) -> Option<(u8, usize)> {
    if p >= re || (b[p] != b'`' && b[p] != b'~') {
        return None;
    }
    let ch = b[p];
    let mut q = p;
    while q < re && b[q] == ch {
        q += 1;
    }
    Some((ch, q - p))
}

fn is_closing_fence(b: &[u8], p: usize, re: usize, ch: u8, min_len: usize) -> bool {
    match fence_run(b, p, re) {
        Some((c, len)) => c == ch && len >= min_len && skip_ws(b, p + len, re) == re,
        None => false,
    }
}

fn task_box_at(b: &[u8], q: usize, re: usize) -> bool {
    q + 3 <= re
        && b[q] == b'['
        && (b[q + 1] == b' ' || b[q + 1] == b'x' || b[q + 1] == b'X')
        && b[q + 2] == b']'
}

/// Parses `-`, `+`, `*`, `12.` or `12)` followed by a space or the line end.
/// Returns (ordered, number, offset after the marker char).
fn parse_list_marker(b: &[u8], p: usize, re: usize) -> Option<(bool, u64, usize)> {
    if p >= re {
        return None;
    }
    let follows = |k: usize| k >= re || b[k] == b' ' || b[k] == b'\t';
    match b[p] {
        b'-' | b'+' | b'*' if follows(p + 1) => Some((false, 0, p + 1)),
        b'0'..=b'9' => {
            let mut q = p;
            let mut num: u64 = 0;
            while q < re && b[q].is_ascii_digit() && q - p < 9 {
                num = num * 10 + u64::from(b[q] - b'0');
                q += 1;
            }
            if q < re && (b[q] == b'.' || b[q] == b')') && follows(q + 1) {
                Some((true, num, q + 1))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// `[!NOTE]`-style marker at `p` (case-insensitive), with trailing blanks.
/// Returns the kind and the offset after the marker and its blanks.
fn match_alert(b: &[u8], p: usize, re: usize) -> Option<(CalloutKind, usize)> {
    let p = skip_ws(b, p, re);
    if p + 2 > re || &b[p..p + 2] != b"[!" {
        return None;
    }
    const KINDS: [(&str, CalloutKind); 5] = [
        ("NOTE", CalloutKind::Note),
        ("TIP", CalloutKind::Tip),
        ("IMPORTANT", CalloutKind::Important),
        ("WARNING", CalloutKind::Warning),
        ("CAUTION", CalloutKind::Caution),
    ];
    for (name, kind) in KINDS {
        let after = p + 2 + name.len();
        if after < re && b[p + 2..after].eq_ignore_ascii_case(name.as_bytes()) && b[after] == b']' {
            return Some((kind, skip_ws(b, after + 1, re)));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Inlines
// ---------------------------------------------------------------------------

/// Turns the raw pulldown spans into `Inline`s with delimiter ranges, sorted
/// by start (outer first on ties).
fn finish_inlines(src: &str, raws: &[Raw]) -> Vec<Inline> {
    let b = src.as_bytes();

    // Largest end for each start: lets bracket matching skip nested spans.
    let mut span_end: HashMap<usize, usize> = HashMap::new();
    for r in raws {
        let e = span_end.entry(r.range.start).or_insert(r.range.end);
        if r.range.end > *e {
            *e = r.range.end;
        }
    }

    // Nested emphasis can share a boundary (`***x***`). Each span takes the
    // delimiter bytes not already claimed by an outer span at that position.
    let mut open_claimed: HashMap<usize, usize> = HashMap::new();
    let mut close_claimed: HashMap<usize, usize> = HashMap::new();

    let mut out = Vec::with_capacity(raws.len());
    for r in raws {
        let s = r.range.start;
        let e = r.range.end.min(b.len());
        if s >= e {
            continue;
        }
        let (kind, open, close) = match &r.kind {
            RawKind::Emphasis | RawKind::Strong | RawKind::Strike => {
                let delim = if matches!(r.kind, RawKind::Strike) {
                    b'~'
                } else {
                    b[s]
                };
                let d = match r.kind {
                    RawKind::Emphasis => 1,
                    RawKind::Strong => 2,
                    _ => {
                        if s + 1 < e && b[s + 1] == b'~' {
                            2
                        } else {
                            1
                        }
                    }
                };
                let oc = open_claimed.get(&s).copied().unwrap_or(0);
                let cc = close_claimed.get(&e).copied().unwrap_or(0);
                let os = s + oc;
                let oe = os + d;
                let ce = e.checked_sub(cc);
                let cs = ce.and_then(|ce| ce.checked_sub(d));
                let ok = match (ce, cs) {
                    (Some(ce), Some(cs)) => {
                        oe <= e
                            && cs >= oe
                            && b[os..oe].iter().all(|&c| c == delim)
                            && b[cs..ce].iter().all(|&c| c == delim)
                    }
                    _ => false,
                };
                let kind = match r.kind {
                    RawKind::Emphasis => InlineKind::Emphasis,
                    RawKind::Strong => InlineKind::Strong,
                    _ => InlineKind::Strike,
                };
                if ok {
                    *open_claimed.entry(s).or_insert(0) += d;
                    *close_claimed.entry(e).or_insert(0) += d;
                    (kind, os..oe, cs.unwrap()..ce.unwrap())
                } else {
                    (kind, s..s, e..e)
                }
            }
            RawKind::Code => {
                let mut n = 0;
                while s + n < e && b[s + n] == b'`' {
                    n += 1;
                }
                if n == 0 || 2 * n > e - s {
                    (InlineKind::Code, s..s, e..e)
                } else {
                    let mut open = s..s + n;
                    let mut close = e - n..e;
                    let inner = open.end..close.start;
                    let all_space = b[inner.clone()].iter().all(|&c| c == b' ');
                    if inner.len() >= 2
                        && b[inner.start] == b' '
                        && b[inner.end - 1] == b' '
                        && !all_space
                    {
                        open.end += 1;
                        close.start -= 1;
                    }
                    (InlineKind::Code, open, close)
                }
            }
            RawKind::Html => (InlineKind::HtmlInline, s..e, e..e),
            RawKind::Link(lt) | RawKind::Image(lt) => {
                let is_image = matches!(r.kind, RawKind::Image(_));
                let autolink = matches!(lt, LinkType::Autolink | LinkType::Email);
                if autolink {
                    (InlineKind::AutoLink, s..s + 1, e - 1..e)
                } else {
                    let open_len = if is_image { 2 } else { 1 };
                    let opens_ok = (is_image && s + 1 < e && b[s] == b'!' && b[s + 1] == b'[')
                        || (!is_image && b[s] == b'[');
                    let text_from = s + open_len;
                    let close_at = if opens_ok {
                        find_close_bracket(b, text_from, e, &span_end)
                    } else {
                        None
                    };
                    match close_at {
                        None => {
                            let kind = if is_image {
                                InlineKind::Image { url: e..e }
                            } else {
                                InlineKind::Link { url: e..e }
                            };
                            (kind, s..text_from.min(e), e..e)
                        }
                        Some(c) => {
                            let url = if b.get(c + 1) == Some(&b'(') {
                                parse_dest(b, c + 2, e)
                                    .map(|(u, _)| u)
                                    .unwrap_or(c + 2..c + 2)
                            } else {
                                c..c
                            };
                            let kind = if is_image {
                                InlineKind::Image { url }
                            } else {
                                InlineKind::Link { url }
                            };
                            (kind, s..text_from, c..e)
                        }
                    }
                }
            }
        };
        out.push(Inline {
            kind,
            range: s..e,
            open,
            close,
        });
    }
    out.sort_by_key(|i| (i.range.start, Reverse(i.range.end)));
    out
}

/// Finds the `]` that closes a link text starting at `from` (depth 1).
/// Nested spans (code, emphasis, inner images) are skipped whole.
fn find_close_bracket(
    b: &[u8],
    from: usize,
    end: usize,
    span_end: &HashMap<usize, usize>,
) -> Option<usize> {
    let mut p = from;
    let mut depth = 1usize;
    while p < end {
        if let Some(&ce) = span_end.get(&p) {
            if ce > p && ce <= end {
                p = ce;
                continue;
            }
        }
        match b[p] {
            b'\\' => {
                p += if p + 1 < end && b[p + 1].is_ascii_punctuation() {
                    2
                } else {
                    1
                };
            }
            b'[' => {
                depth += 1;
                p += 1;
            }
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(p);
                }
                p += 1;
            }
            _ => p += 1,
        }
    }
    None
}

/// Parses `dest ["title"])` starting after `(`. Returns the destination range
/// and the offset after `)`.
fn parse_dest(b: &[u8], start: usize, end: usize) -> Option<(Range<usize>, usize)> {
    let ws = |c: u8| c == b' ' || c == b'\t' || c == b'\n' || c == b'\r';
    let mut p = start;
    while p < end && ws(b[p]) {
        p += 1;
    }
    if p >= end {
        return None;
    }
    let url;
    if b[p] == b'<' {
        let mut q = p + 1;
        while q < end && b[q] != b'>' && b[q] != b'\n' {
            q += 1;
        }
        if q >= end || b[q] != b'>' {
            return None;
        }
        url = p + 1..q;
        p = q + 1;
    } else {
        let st = p;
        let mut depth = 0usize;
        while p < end {
            let c = b[p];
            if ws(c) {
                break;
            }
            if c == b'\\' && p + 1 < end {
                p += 2;
                continue;
            }
            if c == b'(' {
                depth += 1;
            } else if c == b')' {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            p += 1;
        }
        url = st..p;
    }
    while p < end && ws(b[p]) {
        p += 1;
    }
    if p < end && (b[p] == b'"' || b[p] == b'\'' || b[p] == b'(') {
        let close = if b[p] == b'(' { b')' } else { b[p] };
        p += 1;
        while p < end && b[p] != close {
            if b[p] == b'\\' {
                p += 1;
            }
            p += 1;
        }
        if p >= end {
            return None;
        }
        p += 1;
        while p < end && ws(b[p]) {
            p += 1;
        }
    }
    if p < end && b[p] == b')' {
        Some((url, p + 1))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Display
// ---------------------------------------------------------------------------

/// Builds what a line looks like with the caret (or selection) at `caret`.
///
/// Concealed: the delimiters of every inline the caret does not touch, and
/// backslashes before ASCII punctuation (unless the caret touches the escape).
/// Revealed delimiters are kept with `marker: true`. Raw line kinds (code,
/// tables, rules, HTML, front matter, blank, setext underline) return their
/// source text unchanged.
pub fn display_line(src: &str, line: &Line, caret: Option<Range<usize>>) -> DisplayLine {
    let b = src.as_bytes();
    let end = line.range.end.min(b.len());
    let base = line.content_start.min(end);

    let inline_kind = matches!(
        line.kind,
        LineKind::Paragraph
            | LineKind::Heading(_)
            | LineKind::Quote { .. }
            | LineKind::ListItem { .. }
            | LineKind::ListContinuation { .. }
    );
    if !inline_kind {
        let text = src[base..end].to_string();
        let to_source = (base..=end).collect();
        return DisplayLine {
            text,
            to_source,
            styles: Vec::new(),
        };
    }

    let caret = caret.map(|c| (c.start.min(c.end), c.start.max(c.end)));
    let touched = |r: &Range<usize>| match caret {
        Some((a, z)) => r.start <= z && a <= r.end,
        None => false,
    };

    let mut hidden: Vec<Range<usize>> = Vec::new();
    let mut shown_markers: Vec<Range<usize>> = Vec::new();
    let mut no_escape: Vec<Range<usize>> = Vec::new();
    let mut contents: Vec<(Range<usize>, InlineStyle)> = Vec::new();

    for inl in &line.inlines {
        for m in [&inl.open, &inl.close] {
            let m = clip(m, base, end);
            if m.is_empty() {
                continue;
            }
            no_escape.push(m.clone());
            let show = matches!(inl.kind, InlineKind::HtmlInline) || touched(&inl.range);
            if show {
                shown_markers.push(m);
            } else {
                hidden.push(m);
            }
        }
        let cs = clip(&(inl.open.end..inl.close.start), base, end);
        if !cs.is_empty() {
            let mut st = InlineStyle::default();
            match inl.kind {
                InlineKind::Strong => st.bold = true,
                InlineKind::Emphasis => st.italic = true,
                InlineKind::Strike => st.strike = true,
                InlineKind::Code => {
                    st.code = true;
                    no_escape.push(cs.clone());
                }
                InlineKind::Link { .. } | InlineKind::Image { .. } | InlineKind::AutoLink => {
                    st.link = true
                }
                InlineKind::HtmlInline => {}
            }
            contents.push((cs, st));
        }
    }

    // Backslash escapes: hide the backslash unless the caret touches it.
    no_escape.sort_by_key(|r| r.start);
    let mut ne: Vec<Range<usize>> = Vec::with_capacity(no_escape.len());
    for r in no_escape {
        match ne.last_mut() {
            Some(l) if r.start <= l.end => l.end = l.end.max(r.end),
            _ => ne.push(r),
        }
    }
    let mut i = base;
    while i + 1 < end {
        if b[i] == b'\\'
            && b[i + 1].is_ascii_punctuation()
            && !covered(&ne, i)
            && !covered(&ne, i + 1)
        {
            let esc = i..i + 2;
            if touched(&esc) {
                shown_markers.push(i..i + 1);
            } else {
                hidden.push(i..i + 1);
            }
            i += 2;
        } else {
            i += 1;
        }
    }

    // Merge hidden ranges into sorted, disjoint spans.
    hidden.sort_by_key(|r| r.start);
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(hidden.len());
    for h in hidden {
        match merged.last_mut() {
            Some(last) if h.start <= last.end => last.end = last.end.max(h.end),
            _ => merged.push(h),
        }
    }

    // Visible text and the offset map.
    let mut text = String::with_capacity(end - base);
    let mut to_source: Vec<usize> = Vec::with_capacity(end - base + 1);
    let push_visible = |text: &mut String, to_source: &mut Vec<usize>, a: usize, z: usize| {
        if a < z {
            text.push_str(&src[a..z]);
            to_source.extend(a..z);
        }
    };
    let mut cursor = base;
    for h in &merged {
        push_visible(&mut text, &mut to_source, cursor, h.start);
        cursor = cursor.max(h.end);
    }
    push_visible(&mut text, &mut to_source, cursor, end);
    let last = to_source.last().map_or(base, |&k| k + 1);
    to_source.push(last);

    // Display index of a source offset: its position minus the hidden bytes
    // before it. `merged` is sorted, so binary search plus prefix sums keep
    // each lookup O(log n).
    let mut pre: Vec<usize> = Vec::with_capacity(merged.len() + 1);
    pre.push(0);
    for h in &merged {
        let total = *pre.last().unwrap_or(&0);
        pre.push(total + (h.end - h.start));
    }
    let disp = |s: usize| -> usize {
        let k = merged.partition_point(|h| h.start < s);
        let gone = if k == 0 {
            0
        } else {
            let h = &merged[k - 1];
            pre[k - 1] + (h.end.min(s) - h.start)
        };
        (s - base) - gone
    };

    // One sweep over the display: each span adds its flags at its start and
    // removes them at its end, so every window between cut points has a fixed
    // style. Flag order: bold, italic, strike, code, link, marker.
    let mut spans: Vec<(usize, usize, [bool; 6])> =
        Vec::with_capacity(contents.len() + shown_markers.len());
    for (r, st) in &contents {
        let (d0, d1) = (disp(r.start), disp(r.end));
        if d0 < d1 {
            spans.push((
                d0,
                d1,
                [st.bold, st.italic, st.strike, st.code, st.link, false],
            ));
        }
    }
    for m in &shown_markers {
        let (d0, d1) = (disp(m.start), disp(m.end));
        if d0 < d1 {
            spans.push((d0, d1, [false, false, false, false, false, true]));
        }
    }
    let mut events: Vec<(usize, usize, i32)> = Vec::with_capacity(spans.len() * 4);
    for (d0, d1, flags) in &spans {
        for (f, on) in flags.iter().enumerate() {
            if *on {
                events.push((*d0, f, 1));
                events.push((*d1, f, -1));
            }
        }
    }
    events.sort_unstable_by_key(|e| e.0);

    let mut styles: Vec<(Range<usize>, InlineStyle)> = Vec::new();
    let mut counts = [0i32; 6];
    let mut prev: Option<usize> = None;
    let mut i = 0;
    while i < events.len() {
        let pos = events[i].0;
        if let Some(p) = prev {
            if p < pos {
                let st = InlineStyle {
                    bold: counts[0] > 0,
                    italic: counts[1] > 0,
                    strike: counts[2] > 0,
                    code: counts[3] > 0,
                    link: counts[4] > 0,
                    marker: counts[5] > 0,
                };
                if st != InlineStyle::default() {
                    match styles.last_mut() {
                        Some((r, last)) if r.end == p && *last == st => r.end = pos,
                        _ => styles.push((p..pos, st)),
                    }
                }
            }
        }
        while i < events.len() && events[i].0 == pos {
            counts[events[i].1] += events[i].2;
            i += 1;
        }
        prev = Some(pos);
    }

    DisplayLine {
        text,
        to_source,
        styles,
    }
}

fn clip(r: &Range<usize>, base: usize, end: usize) -> Range<usize> {
    let s = r.start.max(base);
    let e = r.end.min(end);
    if s < e {
        s..e
    } else {
        s..s
    }
}

/// `ranges` must be sorted and disjoint.
fn covered(ranges: &[Range<usize>], p: usize) -> bool {
    let k = ranges.partition_point(|r| r.start <= p);
    k > 0 && p < ranges[k - 1].end
}

/// Source offset for display index `idx`. Indices past the end clamp.
pub fn display_to_source(d: &DisplayLine, idx: usize) -> usize {
    d.to_source[idx.min(d.text.len())]
}

/// Display index for a source offset. Offsets inside concealed bytes snap
/// forward to the next visible byte. Offsets past the end clamp.
pub fn source_to_display(d: &DisplayLine, src_off: usize) -> usize {
    d.to_source
        .partition_point(|&x| x < src_off)
        .min(d.text.len())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<LineKind> {
        parse(src).lines.into_iter().map(|l| l.kind).collect()
    }

    fn line(src: &str, i: usize) -> Line {
        parse(src).lines[i].clone()
    }

    /// Display text of line `i` with no caret (everything concealed).
    fn text(src: &str, i: usize) -> String {
        let m = parse(src);
        display_line(src, &m.lines[i], None).text
    }

    fn disp(src: &str, i: usize, caret: Range<usize>) -> DisplayLine {
        let m = parse(src);
        display_line(src, &m.lines[i], Some(caret))
    }

    fn none(src: &str, i: usize) -> DisplayLine {
        let m = parse(src);
        display_line(src, &m.lines[i], None)
    }

    /// Combined style of display byte `k`.
    fn style_at(d: &DisplayLine, k: usize) -> InlineStyle {
        d.styles
            .iter()
            .filter(|(r, _)| r.start <= k && k < r.end)
            .fold(InlineStyle::default(), |mut acc, (_, s)| {
                acc.bold |= s.bold;
                acc.italic |= s.italic;
                acc.strike |= s.strike;
                acc.code |= s.code;
                acc.link |= s.link;
                acc.marker |= s.marker;
                acc
            })
    }

    /// Structural invariants that must hold for every document and caret.
    pub(super) fn check_model(src: &str) {
        let m = parse(src);
        let n = m.lines.len();
        assert!(n >= 1, "always at least one line");
        let mut prev_end = 0;
        for (i, l) in m.lines.iter().enumerate() {
            assert!(l.range.start >= prev_end, "lines are ordered, line {i}");
            assert!(l.range.start <= l.range.end && l.range.end <= src.len());
            assert!(src.is_char_boundary(l.range.start) && src.is_char_boundary(l.range.end));
            assert!(
                l.range.start <= l.content_start && l.content_start <= l.range.end,
                "content in range, line {i}"
            );
            assert_eq!(
                l.marker.end, l.content_start,
                "marker ends at content, line {i}"
            );
            assert!(
                !src[l.range.clone()].contains('\n'),
                "no newline inside a line"
            );
            assert!(!src[l.range.clone()].contains('\r'), "no CR inside a line");
            for inl in &l.inlines {
                assert!(inl.range.start <= inl.open.start && inl.open.end <= inl.close.start);
                assert!(inl.close.end <= inl.range.end, "close inside span, {inl:?}");
                assert!(
                    src.is_char_boundary(inl.open.start) && src.is_char_boundary(inl.close.end)
                );
            }
            let starts: Vec<usize> = l.inlines.iter().map(|x| x.range.start).collect();
            assert!(starts.windows(2).all(|w| w[0] <= w[1]), "inlines sorted");
            prev_end = l.range.end;
        }
        assert_eq!(m.line_index_at(src.len()), n - 1);
        let mut expect = 0;
        for b in m.blocks() {
            assert_eq!(b.lines.start, expect, "blocks are contiguous");
            assert!(b.lines.end > b.lines.start);
            expect = b.lines.end;
        }
        assert_eq!(expect, n, "blocks cover every line");
        for (i, l) in m.lines.iter().enumerate() {
            let mut carets: Vec<Option<Range<usize>>> = vec![None];
            for c in [l.range.start, l.content_start, l.range.end] {
                carets.push(Some(c..c));
            }
            for c in carets {
                let d = display_line(src, l, c.clone());
                check_display(src, &d, &c, i);
            }
        }
    }

    fn check_display(src: &str, d: &DisplayLine, caret: &Option<Range<usize>>, line: usize) {
        assert_eq!(
            d.to_source.len(),
            d.text.len() + 1,
            "map length, line {line} {caret:?}"
        );
        for w in d.to_source.windows(2) {
            assert!(
                w[0] < w[1],
                "to_source strictly increasing, line {line} {caret:?}: {:?}",
                d.to_source
            );
        }
        let sb = src.as_bytes();
        for (k, &b) in d.text.as_bytes().iter().enumerate() {
            assert_eq!(
                sb[d.to_source[k]], b,
                "visible byte maps to the same source byte"
            );
        }
        let mut last_end = 0;
        for (r, _) in &d.styles {
            assert!(r.start < r.end && r.end <= d.text.len(), "style in bounds");
            assert!(r.start >= last_end, "styles sorted and disjoint");
            assert!(d.text.is_char_boundary(r.start) && d.text.is_char_boundary(r.end));
            last_end = r.end;
        }
        for k in 0..=d.text.len() {
            if d.text.is_char_boundary(k) {
                let s = display_to_source(d, k);
                assert_eq!(source_to_display(d, s), k, "round trip at {k}");
            }
        }
    }

    // --- empty and basic structure -----------------------------------------

    #[test]
    fn empty_document_is_one_blank_line() {
        let m = parse("");
        assert_eq!(m.lines.len(), 1);
        assert_eq!(m.lines[0].kind, LineKind::Blank);
        assert_eq!(m.lines[0].range, 0..0);
        check_model("");
    }

    #[test]
    fn trailing_newline_makes_final_empty_line() {
        let m = parse("a\n");
        assert_eq!(m.lines.len(), 2);
        assert_eq!(m.lines[1].range, 2..2);
        assert_eq!(m.line_index_at(2), 1);
        assert_eq!(m.line_index_at(1), 0, "newline belongs to the line it ends");
        check_model("a\n");
    }

    #[test]
    fn paragraph_line() {
        let l = line("hello world", 0);
        assert_eq!(l.kind, LineKind::Paragraph);
        assert_eq!(l.content_start, 0);
        assert_eq!(l.marker, 0..0);
    }

    #[test]
    fn atx_heading_levels_and_markers() {
        let src = "# a\n## b\n###### f";
        assert_eq!(
            kinds(src),
            vec![
                LineKind::Heading(1),
                LineKind::Heading(2),
                LineKind::Heading(6)
            ]
        );
        let l = line(src, 1);
        assert_eq!(
            l.content_start, 7,
            "offsets are absolute: line 1 starts at 4"
        );
        assert_eq!(l.marker, 4..7);
        check_model(src);
    }

    #[test]
    fn empty_atx_heading_has_no_content() {
        let src = "#";
        let l = line(src, 0);
        assert_eq!(l.kind, LineKind::Heading(1));
        assert_eq!(l.content_start, 1);
        check_model(src);
    }

    #[test]
    fn hashtag_without_space_is_paragraph() {
        assert_eq!(kinds("#tag"), vec![LineKind::Paragraph]);
    }

    #[test]
    fn setext_headings_and_underline() {
        let src = "Title\n===\n\nSub\n---";
        assert_eq!(
            kinds(src),
            vec![
                LineKind::Heading(1),
                LineKind::SetextUnderline,
                LineKind::Blank,
                LineKind::Heading(2),
                LineKind::SetextUnderline,
            ]
        );
        let blocks = parse(src).blocks();
        assert_eq!(blocks[0].kind, BlockKind::Heading(1));
        assert_eq!(blocks[0].lines, 0..2);
        assert_eq!(blocks[2].kind, BlockKind::Heading(2));
        check_model(src);
    }

    #[test]
    fn multi_line_setext_heading_is_one_block() {
        let src = "one\ntwo\n===";
        let b = parse(src).blocks();
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].kind, BlockKind::Heading(1));
        assert_eq!(b[0].lines, 0..3);
        check_model(src);
    }

    #[test]
    fn atx_heading_followed_by_setext_text_stays_separate() {
        let src = "# a\nb\n===";
        assert_eq!(
            kinds(src),
            vec![
                LineKind::Heading(1),
                LineKind::Heading(1),
                LineKind::SetextUnderline
            ]
        );
        let b = parse(src).blocks();
        assert_eq!(b.len(), 2);
        assert_eq!(b[0].lines, 0..1);
        assert_eq!(b[1].lines, 1..3);
        check_model(src);
    }

    #[test]
    fn consecutive_atx_headings_are_separate_blocks() {
        assert_eq!(parse("# a\n# b").blocks().len(), 2);
    }

    #[test]
    fn rules_of_three_kinds() {
        assert_eq!(
            kinds("a\n\n---\n\n***\n\n___"),
            vec![
                LineKind::Paragraph,
                LineKind::Blank,
                LineKind::Rule,
                LineKind::Blank,
                LineKind::Rule,
                LineKind::Blank,
                LineKind::Rule,
            ]
        );
    }

    // --- quotes and callouts ---------------------------------------------

    #[test]
    fn quote_depth_one_and_content() {
        let src = "> a";
        assert_eq!(
            line(src, 0).kind,
            LineKind::Quote {
                depth: 1,
                callout: None
            }
        );
        assert_eq!(line(src, 0).content_start, 2);
        assert_eq!(text(src, 0), "a");
        check_model(src);
    }

    #[test]
    fn nested_quote_depth_two() {
        let src = "> > a";
        assert_eq!(
            line(src, 0).kind,
            LineKind::Quote {
                depth: 2,
                callout: None
            }
        );
        assert_eq!(line(src, 0).content_start, 4);
        check_model(src);
    }

    #[test]
    fn lazy_quote_continuation_keeps_quote_kind() {
        let src = "> a\nb";
        let l = line(src, 1);
        assert_eq!(
            l.kind,
            LineKind::Quote {
                depth: 1,
                callout: None
            }
        );
        assert_eq!(
            l.content_start, 4,
            "lazy line has no marker: content is its line start"
        );
        assert_eq!(text(src, 1), "b");
        check_model(src);
    }

    #[test]
    fn bare_quote_marker_line_is_empty_quote() {
        let src = ">";
        let l = line(src, 0);
        assert_eq!(
            l.kind,
            LineKind::Quote {
                depth: 1,
                callout: None
            }
        );
        assert_eq!(l.content_start, 1);
        assert_eq!(text(src, 0), "");
        check_model(src);
    }

    #[test]
    fn callout_note_marks_every_quote_line() {
        let src = "> [!NOTE]\n> text\n> more\n\nafter";
        let m = parse(src);
        let note = Some(CalloutKind::Note);
        assert_eq!(
            m.lines[0].kind,
            LineKind::Quote {
                depth: 1,
                callout: note
            }
        );
        assert_eq!(m.lines[0].content_start, 9, "marker line displays empty");
        assert_eq!(text(src, 0), "");
        assert_eq!(
            m.lines[1].kind,
            LineKind::Quote {
                depth: 1,
                callout: note
            }
        );
        assert_eq!(
            m.lines[2].kind,
            LineKind::Quote {
                depth: 1,
                callout: note
            }
        );
        let b = m.blocks();
        assert_eq!(b[0].kind, BlockKind::Callout(CalloutKind::Note));
        assert_eq!(b[0].lines, 0..3);
        check_model(src);
    }

    #[test]
    fn callout_marker_with_text_and_lowercase() {
        let src = "> [!warning] careful now";
        let l = line(src, 0);
        assert_eq!(
            l.kind,
            LineKind::Quote {
                depth: 1,
                callout: Some(CalloutKind::Warning)
            }
        );
        assert_eq!(text(src, 0), "careful now");
        assert_eq!(
            l.marker,
            0..13,
            "quote marker and alert marker incl. trailing blank"
        );
        check_model(src);
    }

    #[test]
    fn all_five_callout_kinds() {
        for (name, kind) in [
            ("NOTE", CalloutKind::Note),
            ("TIP", CalloutKind::Tip),
            ("IMPORTANT", CalloutKind::Important),
            ("WARNING", CalloutKind::Warning),
            ("CAUTION", CalloutKind::Caution),
        ] {
            let src = format!("> [!{name}]\n> x");
            assert_eq!(
                line(&src, 0).kind,
                LineKind::Quote {
                    depth: 1,
                    callout: Some(kind)
                }
            );
        }
    }

    #[test]
    fn callout_must_be_first_line_and_known_name() {
        assert_eq!(
            line("> a\n> [!TIP]", 1).kind,
            LineKind::Quote {
                depth: 1,
                callout: None
            }
        );
        assert_eq!(
            line("> [!FOO]", 0).kind,
            LineKind::Quote {
                depth: 1,
                callout: None
            }
        );
        assert_eq!(
            line("> [!NOTEX] a", 0).kind,
            LineKind::Quote {
                depth: 1,
                callout: None
            }
        );
    }

    // --- lists --------------------------------------------------------------

    #[test]
    fn bullet_item_marker_and_content() {
        let src = "- a";
        let l = line(src, 0);
        assert_eq!(
            l.kind,
            LineKind::ListItem {
                indent: 0,
                ordered: false,
                number: 0,
                task: None
            }
        );
        assert_eq!(l.marker, 0..2);
        assert_eq!(l.content_start, 2);
        check_model(src);
    }

    #[test]
    fn ordered_items_keep_source_numbers() {
        let src = "12. x\n3) y";
        assert_eq!(
            kinds(src),
            vec![
                LineKind::ListItem {
                    indent: 0,
                    ordered: true,
                    number: 12,
                    task: None
                },
                LineKind::ListItem {
                    indent: 0,
                    ordered: true,
                    number: 3,
                    task: None
                },
            ]
        );
        assert_eq!(line(src, 0).marker, 0..4);
        check_model(src);
    }

    #[test]
    fn nested_items_indent_from_the_parser() {
        let src = "- a\n  - b\n    - c";
        assert_eq!(
            kinds(src),
            vec![
                LineKind::ListItem {
                    indent: 0,
                    ordered: false,
                    number: 0,
                    task: None
                },
                LineKind::ListItem {
                    indent: 1,
                    ordered: false,
                    number: 0,
                    task: None
                },
                LineKind::ListItem {
                    indent: 2,
                    ordered: false,
                    number: 0,
                    task: None
                },
            ]
        );
        assert_eq!(
            line(src, 1).marker,
            6..8,
            "absolute offsets: line 1 starts at 4"
        );
        assert_eq!(text(src, 2), "c");
        check_model(src);
    }

    #[test]
    fn task_boxes_checked_and_unchecked() {
        let src = "- [ ] a\n- [x] b\n- [X] c";
        let m = parse(src);
        assert_eq!(
            m.lines[0].kind,
            LineKind::ListItem {
                indent: 0,
                ordered: false,
                number: 0,
                task: Some(false)
            }
        );
        assert_eq!(
            m.lines[1].kind,
            LineKind::ListItem {
                indent: 0,
                ordered: false,
                number: 0,
                task: Some(true)
            }
        );
        assert_eq!(
            m.lines[2].kind,
            LineKind::ListItem {
                indent: 0,
                ordered: false,
                number: 0,
                task: Some(true)
            }
        );
        assert_eq!(m.lines[0].content_start, 6, "content after the box");
        assert_eq!(text(src, 0), "a");
        check_model(src);
    }

    #[test]
    fn task_box_needs_a_space_after_it() {
        match line("- [x]y", 0).kind {
            LineKind::ListItem { task: None, .. } => {}
            other => panic!("expected plain item, got {other:?}"),
        }
    }

    #[test]
    fn item_continuation_and_lazy_line() {
        let src = "- a\n  b\nlazy";
        assert_eq!(kinds(src)[1], LineKind::ListContinuation { indent: 0 });
        assert_eq!(kinds(src)[2], LineKind::ListContinuation { indent: 0 });
        check_model(src);
    }

    #[test]
    fn blank_line_between_items_is_blank() {
        let src = "- a\n\n- b";
        assert_eq!(
            kinds(src),
            vec![
                LineKind::ListItem {
                    indent: 0,
                    ordered: false,
                    number: 0,
                    task: None
                },
                LineKind::Blank,
                LineKind::ListItem {
                    indent: 0,
                    ordered: false,
                    number: 0,
                    task: None
                },
            ]
        );
    }

    #[test]
    fn empty_items_have_empty_content() {
        let src = "-\n- ";
        let m = parse(src);
        assert_eq!(m.lines[0].content_start, 1);
        assert_eq!(m.lines[1].content_start, 4, "absolute: '- ' at 2..4");
        check_model(src);
    }

    #[test]
    fn item_with_quote_inside() {
        let src = "- > q\n  > r";
        let m = parse(src);
        assert!(matches!(m.lines[0].kind, LineKind::ListItem { .. }));
        assert_eq!(m.lines[0].content_start, 4);
        assert_eq!(
            m.lines[1].kind,
            LineKind::Quote {
                depth: 1,
                callout: None
            }
        );
        assert_eq!(text(src, 1), "r");
        check_model(src);
    }

    #[test]
    fn quote_containing_list_item() {
        let src = "> - a\n>   b";
        let m = parse(src);
        assert_eq!(
            m.lines[0].kind,
            LineKind::ListItem {
                indent: 0,
                ordered: false,
                number: 0,
                task: None
            }
        );
        assert_eq!(m.lines[0].content_start, 4);
        assert_eq!(m.lines[1].kind, LineKind::ListContinuation { indent: 0 });
        assert_eq!(m.lines[1].content_start, 10);
        check_model(src);
    }

    // --- code ----------------------------------------------------------------

    #[test]
    fn fenced_code_keeps_raw_lines_and_lang() {
        let src = "```rust\nlet x = 1;\n\n  indented\n```";
        assert_eq!(
            kinds(src),
            vec![
                LineKind::FenceOpen {
                    lang: "rust".into()
                },
                LineKind::CodeLine,
                LineKind::CodeLine,
                LineKind::CodeLine,
                LineKind::FenceClose,
            ]
        );
        assert_eq!(text(src, 3), "  indented", "code keeps its leading spaces");
        assert_eq!(text(src, 2), "");
        let b = parse(src).blocks();
        assert_eq!(b.len(), 1);
        assert_eq!(
            b[0].kind,
            BlockKind::CodeBlock {
                lang: "rust".into()
            }
        );
        check_model(src);
    }

    #[test]
    fn code_block_inner_markup_is_not_processed() {
        let src = "```\n*not em* [x](y)\n```";
        let d = disp(src, 1, 0..0);
        assert_eq!(d.text, "*not em* [x](y)");
        assert!(d.styles.is_empty());
    }

    #[test]
    fn fence_opened_on_a_list_item_line_closes_properly() {
        let src = "- ```\n  code\n  ```\n\nafter";
        let m = parse(src);
        assert!(matches!(m.lines[0].kind, LineKind::ListItem { .. }));
        assert_eq!(m.lines[1].kind, LineKind::CodeLine);
        assert_eq!(
            m.lines[2].kind,
            LineKind::FenceClose,
            "closing fence recognised"
        );
        assert_eq!(m.lines[4].kind, LineKind::Paragraph);
        check_model(src);
    }

    #[test]
    fn tilde_fence_and_longer_closing_fence() {
        let src = "~~~\nx\n~~~~";
        assert_eq!(
            kinds(src),
            vec![
                LineKind::FenceOpen {
                    lang: String::new()
                },
                LineKind::CodeLine,
                LineKind::FenceClose
            ]
        );
        let src2 = "````\n```\nin\n````";
        assert_eq!(
            kinds(src2),
            vec![
                LineKind::FenceOpen {
                    lang: String::new()
                },
                LineKind::CodeLine,
                LineKind::CodeLine,
                LineKind::FenceClose
            ]
        );
        check_model(src);
        check_model(src2);
    }

    #[test]
    fn unclosed_fence_runs_to_end_of_document() {
        let src = "```\na\nb";
        assert_eq!(
            kinds(src),
            vec![
                LineKind::FenceOpen {
                    lang: String::new()
                },
                LineKind::CodeLine,
                LineKind::CodeLine
            ]
        );
        assert_eq!(parse(src).blocks()[0].lines, 0..3);
        check_model(src);
    }

    #[test]
    fn fenced_code_inside_list_item_strips_item_indent() {
        let src = "1. a\n\n   ```\n   code\n     x\n   ```";
        let m = parse(src);
        assert_eq!(m.lines[3].kind, LineKind::CodeLine);
        assert_eq!(text(src, 3), "code");
        assert_eq!(text(src, 4), "  x", "extra indentation kept");
        assert_eq!(m.lines[5].kind, LineKind::FenceClose);
        check_model(src);
    }

    #[test]
    fn fenced_code_inside_quote_strips_marker() {
        let src = "> ```\n> x  y\n> ```";
        let m = parse(src);
        assert_eq!(m.lines[1].kind, LineKind::CodeLine);
        assert_eq!(text(src, 1), "x  y");
        check_model(src);
    }

    #[test]
    fn indented_code_strips_four_columns() {
        let src = "    code\n      more";
        let m = parse(src);
        assert_eq!(m.lines[0].kind, LineKind::IndentedCode);
        assert_eq!(m.lines[0].content_start, 4);
        assert_eq!(text(src, 1), "  more");
        assert_eq!(
            m.blocks()[0].kind,
            BlockKind::CodeBlock {
                lang: String::new()
            }
        );
        check_model(src);
    }

    // --- tables, html, front matter ------------------------------------------

    #[test]
    fn table_lines_are_raw_and_classified() {
        let src = "| a | b |\n|---|:-:|\n| 1 | 2 |\n\nafter";
        assert_eq!(
            kinds(src),
            vec![
                LineKind::TableHeader,
                LineKind::TableDelimiter,
                LineKind::TableRow,
                LineKind::Blank,
                LineKind::Paragraph,
            ]
        );
        assert_eq!(text(src, 2), "| 1 | 2 |", "table rows are shown whole");
        let b = parse(src).blocks();
        assert_eq!(b[0].kind, BlockKind::Table);
        assert_eq!(b[0].lines, 0..3);
        check_model(src);
    }

    #[test]
    fn html_block_lines() {
        let src = "<div>\nx\n</div>\n\nafter";
        let k = kinds(src);
        assert_eq!(k[..3], [LineKind::Html, LineKind::Html, LineKind::Html]);
        assert_eq!(parse(src).blocks()[0].kind, BlockKind::Html);
        check_model(src);
    }

    #[test]
    fn front_matter_block() {
        let src = "---\ntitle: x\n---\n# h";
        let m = parse(src);
        assert_eq!(m.lines[0].kind, LineKind::FrontMatter);
        assert_eq!(m.lines[1].kind, LineKind::FrontMatter);
        assert_eq!(m.lines[2].kind, LineKind::FrontMatter);
        assert_eq!(m.lines[3].kind, LineKind::Heading(1));
        assert_eq!(m.blocks()[0].kind, BlockKind::FrontMatter);
        check_model(src);
    }

    #[test]
    fn dashes_mid_document_are_a_rule_not_front_matter() {
        assert_eq!(kinds("a\n\n---\nb")[2], LineKind::Rule);
    }

    // --- line endings --------------------------------------------------------

    #[test]
    fn crlf_ranges_exclude_carriage_return() {
        let src = "a\r\nb\r\n";
        let m = parse(src);
        assert_eq!(m.lines[0].range, 0..1);
        assert_eq!(m.lines[1].range, 3..4);
        assert_eq!(m.lines[2].range, 6..6);
        assert_eq!(text(src, 1), "b");
        check_model(src);
    }

    #[test]
    fn crlf_fence_and_setext() {
        let src = "```rust\r\ncode\r\n```\r\n\r\nTitle\r\n===";
        let m = parse(src);
        assert_eq!(m.lines[1].kind, LineKind::CodeLine);
        assert_eq!(text(src, 1), "code");
        assert_eq!(m.lines[4].kind, LineKind::Heading(1));
        assert_eq!(m.lines[5].kind, LineKind::SetextUnderline);
        check_model(src);
    }

    #[test]
    fn lone_cr_is_a_line_break() {
        let src = "a\rb";
        assert_eq!(parse(src).lines.len(), 2);
        check_model(src);
    }

    #[test]
    fn hard_break_with_trailing_spaces_and_backslash() {
        let src = "a  \nb\\\nc";
        let m = parse(src);
        assert_eq!(m.lines.len(), 3);
        assert_eq!(text(src, 0), "a  ");
        check_model(src);
    }

    // --- inlines and concealment -------------------------------------------

    #[test]
    fn strong_concealed_until_touched() {
        let src = "**a** b";
        assert_eq!(text(src, 0), "a b");
        let d = disp(src, 0, 6..6);
        assert_eq!(d.text, "a b", "caret after the space does not reveal");
        assert!(style_at(&d, 0).bold);
        check_model(src);
    }

    #[test]
    fn caret_just_after_closing_marker_reveals() {
        let src = "**a** b";
        let d = disp(src, 0, 5..5);
        assert_eq!(d.text, "**a** b");
        assert!(style_at(&d, 0).marker);
        assert!(style_at(&d, 2).bold);
        check_model(src);
    }

    #[test]
    fn caret_just_before_opening_marker_reveals() {
        let src = "x **a**";
        assert_eq!(text(src, 0), "x a");
        assert_eq!(disp(src, 0, 2..2).text, "x **a**");
        assert_eq!(
            disp(src, 0, 1..1).text,
            "x a",
            "caret before the space does not reveal"
        );
        check_model(src);
    }

    #[test]
    fn caret_at_document_start_reveals_leading_strong() {
        let src = "**a**";
        assert_eq!(disp(src, 0, 0..0).text, "**a**");
        assert_eq!(none(src, 0).text, "a");
    }

    #[test]
    fn selection_touching_inline_reveals_it() {
        let src = "a *b* c";
        assert_eq!(
            disp(src, 0, 0..2).text,
            "a *b* c",
            "selection reaching the inline reveals it"
        );
        assert_eq!(disp(src, 0, 0..1).text, "a b c");
        check_model(src);
    }

    #[test]
    fn emphasis_strike_code_and_plain_flags() {
        let src = "*i* ~~s~~ `c` plain";
        let d = none(src, 0);
        assert_eq!(d.text, "i s c plain");
        assert!(style_at(&d, 0).italic);
        assert!(style_at(&d, 2).strike);
        assert!(style_at(&d, 4).code);
        assert_eq!(style_at(&d, 6), InlineStyle::default());
        check_model(src);
    }

    #[test]
    fn nested_emphasis_combines_flags() {
        let src = "**a *b* c**";
        let d = none(src, 0);
        assert_eq!(d.text, "a b c");
        assert!(style_at(&d, 0).bold && !style_at(&d, 0).italic);
        assert!(
            style_at(&d, 2).bold && style_at(&d, 2).italic,
            "b is bold and italic"
        );
        assert!(style_at(&d, 4).bold && !style_at(&d, 4).italic);
        check_model(src);
    }

    #[test]
    fn triple_star_nests_emphasis_outside_strong() {
        let src = "***x*** tail";
        assert_eq!(none(src, 0).text, "x tail");
        assert!(style_at(&none(src, 0), 0).bold && style_at(&none(src, 0), 0).italic);
        let d = disp(src, 0, 0..0);
        assert_eq!(
            d.text, "*x* tail",
            "outer markers revealed, inner still hidden"
        );
        check_model(src);
    }

    #[test]
    fn link_text_is_visible_and_url_hidden() {
        let src = "see [docs](https://x.y/z) now";
        let d = none(src, 0);
        assert_eq!(d.text, "see docs now");
        assert!(style_at(&d, 4).link);
        assert!(!style_at(&d, 0).link);
        let shown = disp(src, 0, 6..6);
        assert_eq!(shown.text, "see [docs](https://x.y/z) now");
        assert!(
            style_at(&shown, 5).link,
            "link text keeps link style when markers are shown"
        );
        assert!(style_at(&shown, 4).marker, "revealed '[' is a muted marker");
        check_model(src);
    }

    #[test]
    fn link_url_range_and_title_forms() {
        let src = "[a](u \"t\") [b](<u v>)";
        let m = parse(src);
        let inl: Vec<&Inline> = m.lines[0].inlines.iter().collect();
        assert_eq!(inl.len(), 2);
        match &inl[0].kind {
            InlineKind::Link { url } => assert_eq!(&src[url.clone()], "u"),
            other => panic!("{other:?}"),
        }
        match &inl[1].kind {
            InlineKind::Link { url } => assert_eq!(&src[url.clone()], "u v"),
            other => panic!("{other:?}"),
        }
        assert_eq!(&src[inl[0].close.clone()], "](u \"t\")");
        check_model(src);
    }

    #[test]
    fn reference_link_has_empty_url_and_shows_text() {
        let src = "[a][r] and [b]\n\n[r]: /x\n[b]: /y";
        let m = parse(src);
        let first = &m.lines[0].inlines[0];
        match &first.kind {
            InlineKind::Link { url } => assert!(url.is_empty()),
            other => panic!("{other:?}"),
        }
        assert_eq!(text(src, 0), "a and b");
        check_model(src);
    }

    #[test]
    fn unknown_shortcut_is_plain_text() {
        let src = "[a] plain";
        assert!(parse(src).lines[0].inlines.is_empty());
        assert_eq!(text(src, 0), "[a] plain");
        check_model(src);
    }

    #[test]
    fn image_shows_alt_text_as_link() {
        let src = "![alt *x*](src.png) end";
        let d = none(src, 0);
        assert_eq!(d.text, "alt x end");
        assert!(style_at(&d, 0).link);
        assert!(style_at(&d, 4).italic);
        match &parse(src).lines[0].inlines[0].kind {
            InlineKind::Image { url } => assert_eq!(&src[url.clone()], "src.png"),
            other => panic!("{other:?}"),
        }
        check_model(src);
    }

    #[test]
    fn autolink_shows_address() {
        let src = "<https://a.b> x";
        let d = none(src, 0);
        assert_eq!(d.text, "https://a.b x");
        assert!(style_at(&d, 0).link);
        check_model(src);
    }

    #[test]
    fn raw_html_tags_stay_visible_as_markers() {
        let src = "a <b>x</b> c";
        let d = none(src, 0);
        assert_eq!(d.text, "a <b>x</b> c", "raw html is never concealed");
        assert!(style_at(&d, 2).marker);
        check_model(src);
    }

    #[test]
    fn code_span_padding_and_backticks() {
        let src = "`` a ` b `` end";
        let d = none(src, 0);
        assert_eq!(d.text, "a ` b end");
        assert!(style_at(&d, 0).code);
        assert!(!style_at(&d, 8).code);
        check_model(src);
    }

    #[test]
    fn code_span_keeps_backslash_literal() {
        let src = "`\\*` x";
        assert_eq!(none(src, 0).text, "\\* x");
        check_model(src);
    }

    #[test]
    fn backslash_escape_hidden_and_revealed() {
        let src = "\\*not\\*";
        assert_eq!(none(src, 0).text, "*not*");
        let d = disp(src, 0, 0..0);
        assert_eq!(
            d.text, "\\*not*",
            "touching the first escape reveals only it"
        );
        check_model(src);
    }

    #[test]
    fn double_backslash_keeps_one() {
        let src = "\\\\*";
        assert_eq!(none(src, 0).text, "\\*");
        check_model(src);
    }

    #[test]
    fn strikethrough_single_and_double_tilde() {
        let src = "~a~ ~~b~~";
        let d = none(src, 0);
        assert_eq!(d.text, "a b");
        assert!(style_at(&d, 0).strike && style_at(&d, 2).strike);
        check_model(src);
    }

    #[test]
    fn multi_line_emphasis_appears_on_both_lines() {
        let src = "*a\nb*";
        let m = parse(src);
        assert_eq!(m.lines[0].inlines.len(), 1);
        assert_eq!(m.lines[1].inlines.len(), 1);
        assert_eq!(text(src, 0), "a");
        assert_eq!(text(src, 1), "b");
        assert!(style_at(&none(src, 1), 0).italic);
        check_model(src);
    }

    #[test]
    fn inline_in_heading_and_task_item() {
        let src = "## Hi *there*";
        let d = none(src, 0);
        assert_eq!(d.text, "Hi there");
        assert!(style_at(&d, 3).italic);
        let src2 = "- [x] **done** item";
        let d2 = none(src2, 0);
        assert_eq!(d2.text, "done item");
        assert!(style_at(&d2, 0).bold);
        check_model(src);
        check_model(src2);
    }

    #[test]
    fn cjk_text_with_emphasis_round_trips() {
        let src = "中文 **粗体** 文字 *斜* 和 `代码`";
        let d = none(src, 0);
        assert_eq!(d.text, "中文 粗体 文字 斜 和 代码");
        let bold = d.text.find("粗体").unwrap();
        assert!(style_at(&d, bold).bold);
        let shown = disp(src, 0, 9..9);
        assert_eq!(shown.text, "中文 **粗体** 文字 斜 和 代码");
        check_model(src);
    }

    #[test]
    fn thai_and_emoji_offsets_are_byte_exact() {
        let src = "ไทย **ทดสอบ** 😀 *ok*";
        let d = none(src, 0);
        assert_eq!(d.text, "ไทย ทดสอบ 😀 ok");
        let bold_start = d.text.find("ทดสอบ").unwrap();
        assert!(style_at(&d, bold_start).bold);
        let shown = disp(src, 0, 12..12);
        assert_eq!(shown.text, "ไทย **ทดสอบ** 😀 ok");
        check_model(src);
    }

    #[test]
    fn thai_heading_content_start_is_char_boundary() {
        let src = "# สวัสดี **โลก**";
        let l = line(src, 0);
        assert!(src.is_char_boundary(l.content_start));
        assert_eq!(none(src, 0).text, "สวัสดี โลก");
        check_model(src);
    }

    // --- display mapping -----------------------------------------------------

    #[test]
    fn source_offsets_map_through_concealed_text() {
        let src = "a **bold** c";
        let d = none(src, 0);
        assert_eq!(d.text, "a bold c");
        assert_eq!(display_to_source(&d, 2), 4, "'b' is at source 4");
        assert_eq!(display_to_source(&d, 6), 10, "space after bold");
        assert_eq!(source_to_display(&d, 4), 2);
        assert_eq!(
            source_to_display(&d, 2),
            2,
            "offset inside concealed '**' snaps forward"
        );
        assert_eq!(
            display_to_source(&d, 999),
            d.to_source[d.text.len()],
            "index clamps"
        );
        assert_eq!(source_to_display(&d, 999), d.text.len(), "offset clamps");
        check_model(src);
    }

    #[test]
    fn to_source_is_monotonic_on_a_rich_document() {
        let src = "# Title **x**\n\n> [!TIP] *hi*\n> more `c`\n\n- [ ] [l](u) ~~s~~\n  - nested \\* esc\n\n1. one\n2. two\n\n| a | b |\n|---|---|\n| ไทย | 😀 |\n\n```rust\ncode\n```\n\n---\n<div>x</div>\n";
        check_model(src);
        let m = parse(src);
        for i in 0..m.lines.len() {
            let d = none(src, i);
            for w in d.to_source.windows(2) {
                assert!(w[0] <= w[1]);
            }
        }
    }

    #[test]
    fn blank_line_displays_empty() {
        assert_eq!(none("a\n\nb", 1).text, "");
        assert_eq!(none("a\n   \nb", 1).text, "");
    }

    // --- blocks, indices and robustness --------------------------------------

    #[test]
    fn block_grouping_of_a_mixed_document() {
        let src = "# H\ntext\n\n- a\n- b\n\n> q\n> q2\n\n```\nc\n```\n\n---";
        let kinds: Vec<BlockKind> = parse(src).blocks().into_iter().map(|b| b.kind).collect();
        assert_eq!(
            kinds,
            vec![
                BlockKind::Heading(1),
                BlockKind::Paragraph,
                BlockKind::Blank,
                BlockKind::ListItem,
                BlockKind::ListItem,
                BlockKind::Blank,
                BlockKind::Quote,
                BlockKind::Blank,
                BlockKind::CodeBlock {
                    lang: String::new()
                },
                BlockKind::Blank,
                BlockKind::Rule,
            ]
        );
    }

    #[test]
    fn line_index_at_handles_terminators_and_past_end() {
        let src = "ab\r\ncd";
        let m = parse(src);
        assert_eq!(m.line_index_at(0), 0);
        assert_eq!(m.line_index_at(2), 0, "CR belongs to line 0");
        assert_eq!(m.line_index_at(3), 0, "LF belongs to line 0");
        assert_eq!(m.line_index_at(4), 1);
        assert_eq!(m.line_index_at(src.len() + 10), 1);
    }

    #[test]
    fn caret_outside_document_does_not_panic() {
        let src = "**a**";
        let d = disp(src, 0, 100..100);
        assert_eq!(d.text, "a");
        check_display(src, &d, &Some(100..100), 0);
    }

    #[test]
    fn odd_inputs_do_not_panic() {
        for src in [
            "[",
            "](",
            "**",
            "`",
            "\\",
            "<",
            "![",
            "~~",
            "> ",
            "- ",
            "1.",
            "\r",
            "\r\n\r\n",
            "[a](",
            "[a](b",
            "*a",
            "_",
            "```",
            "> > > >",
            "- - - -",
            "|",
            "| a |\n|",
            "[^1]",
            "[x]: y",
            "<!-- c -->",
            "&amp;",
            "\u{feff}# bom",
            "é\u{301}*x*",
            "😀😀**😀**",
        ] {
            check_model(src);
        }
    }

    #[test]
    fn every_prefix_of_a_rich_document_parses() {
        let doc = "---\ntitle: t\n---\n# Title **x** ~~y~~\nSetext\n===\n\n> [!NOTE] *hi*\n> body `c`\n> > deep\nlazy quote\n\n- [ ] [l](https://u) \\* ![i](s)\n  - nested <b>x</b>\n    1. ordered\n\n12) paren\n\n| a | b |\n|:-|-:|\n| ไทย | 😀 |\n\n    indented\n\n```rust\nfn main() {}\n\n  x\n```\n\n~~~\ntilde\n~~~\n\n---\n<div>\nhtml\n</div>\n\n[ref]: http://x\nfoot[^1] and [a][ref]\r\nCRLF line\r\n";
        let mut count = 0;
        for end in 0..=doc.len() {
            if !doc.is_char_boundary(end) {
                continue;
            }
            let prefix = &doc[..end];
            check_model(prefix);
            count += 1;
        }
        assert!(count > 200);
    }

    /// Small deterministic generator (no external crates).
    fn lcg(seed: &mut u64) -> u64 {
        *seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        *seed >> 33
    }

    #[test]
    fn random_markdown_soup_keeps_invariants() {
        let alphabet: Vec<&str> = vec![
            "#", " ", " ", "\n", "\n", "\r\n", "-", "*", "_", "`", "~", "[", "]", "(", ")", "!",
            "<", ">", "\\", "a", "b", "ไ", "😀", "1", ".", "|", ":", "+", "\"", "'", "\t", "e",
            "h", "[!NOTE]", "```", "---", "===", "    ", "[ ] ", "x ", "**", "![", "](",
        ];
        let mut seed = 0x5eed_u64;
        for _ in 0..1500 {
            let len = (lcg(&mut seed) % 60) as usize;
            let mut s = String::new();
            for _ in 0..len {
                let k = (lcg(&mut seed) as usize) % alphabet.len();
                s.push_str(alphabet[k]);
            }
            check_model(&s);
        }
    }

    #[test]
    fn large_document_parses_quickly_enough() {
        let unit = "# Heading **bold** *em* `code` [link](https://x.y)\n\n- item with ~~strike~~\n  continued text\n\n> quote [!NOTE]\n\n```rust\nlet x = 1;\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n";
        let mut big = String::new();
        while big.len() < 1_000_000 {
            big.push_str(unit);
        }
        let start = std::time::Instant::now();
        let m = parse(&big);
        let elapsed = start.elapsed();
        assert!(m.lines.len() > 1000);
        assert!(elapsed.as_secs_f64() < 20.0, "debug parse took {elapsed:?}");
        eprintln!(
            "rich_model: parsed {} bytes, {} lines in {elapsed:?} (debug)",
            big.len(),
            m.lines.len()
        );
    }
}
