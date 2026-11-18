//! PDF export for the rich (preview) view.
//!
//! The markdown is parsed with `preview::parse` and laid out on A4 pages with
//! the print rules from `src/lib/exportPdf.ts`: 11pt body, 18mm/16mm margins,
//! grey code blocks, bordered tables with a shaded header, blockquote bars,
//! horizontal rules. Text is set in subset-embedded TrueType fonts shaped with
//! rustybuzz, so Thai and other non-Latin scripts render with their marks in
//! place. Nothing here uses a webview or an external binary.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::fs::File;
use std::io::Write as _;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use flate2::write::ZlibEncoder;
use flate2::Compression;
use pdf_writer::types::{CidFontType, FontFlags, SystemInfo};
use pdf_writer::{Filter, Name, Pdf, Rect, Ref, Str, TextStr};
use rustybuzz::ttf_parser::{self, GlyphId};
use subsetter::GlyphRemapper;

use crate::preview::{self, Align, Block, Callout, ListItem, Span};

// ---------------------------------------------------------------------------
// Page geometry and print styling (mirrors src/lib/exportPdf.ts)
// ---------------------------------------------------------------------------

const MM: f32 = 72.0 / 25.4;
const PAGE_W: f32 = 595.28;
const PAGE_H: f32 = 841.89;
const MARGIN_X: f32 = 16.0 * MM;
const MARGIN_Y: f32 = 18.0 * MM;
const CONTENT_W: f32 = PAGE_W - 2.0 * MARGIN_X;
const CONTENT_H: f32 = PAGE_H - 2.0 * MARGIN_Y;
const TOP: f32 = MARGIN_Y;
const BOTTOM: f32 = PAGE_H - MARGIN_Y;

const BODY: f32 = 11.0;
/// Body line height as a multiple of the font size.
const LINE: f32 = 1.3;
/// Headings use `line-height: 1.25`.
const HEAD_LINE: f32 = 1.25;
/// `code { font-size: 0.88em }`.
const CODE_SCALE: f32 = 0.88;
/// Code blocks and inline code use a monospace face with this line height.
const CODE_LINE: f32 = 1.3;
/// `p, ul, ol, blockquote, table, pre { margin: 0 0 0.85em }`.
const GAP: f32 = 0.85 * BODY;
/// `li { margin: 0.2em 0 }`.
const LI_GAP: f32 = 0.2 * BODY;
const LIST_INDENT: f32 = 22.0;
const QUOTE_BAR: f32 = 3.0;
const QUOTE_PAD: f32 = BODY;
/// `table { font-size: 0.95em }`, `th, td { padding: 0.4em 0.6em }`.
const TABLE_SCALE: f32 = 0.95;
const CELL_PAD_X: f32 = 0.6 * TABLE_SCALE * BODY;
const CELL_PAD_Y: f32 = 0.4 * TABLE_SCALE * BODY;
/// `pre { padding: 0.7em 0.9em }`.
const PRE_PAD_Y: f32 = 0.7 * BODY;
const PRE_PAD_X: f32 = 0.9 * BODY;
const MAX_IMAGE_H: f32 = 600.0;
/// Images are decoded at no more than this many pixels per side.
const MAX_IMAGE_PX: u32 = 2000;
/// CSS pixels to PDF points.
const PX_TO_PT: f32 = 0.75;

const BLACK: [f32; 3] = [0.0, 0.0, 0.0];
const GREY_TEXT: [f32; 3] = [0.2, 0.2, 0.2];
const MUTED: [f32; 3] = [0.4, 0.4, 0.4];
const CODE_BG: [f32; 3] = [0.965, 0.965, 0.965];
const PRE_BORDER: [f32; 3] = [0.878, 0.878, 0.878];
const INLINE_BG: [f32; 3] = [0.949, 0.949, 0.949];
const HEAD_BG: [f32; 3] = [0.949, 0.949, 0.949];
const TABLE_BORDER: [f32; 3] = [0.733, 0.733, 0.733];
const RULE: [f32; 3] = [0.8, 0.8, 0.8];
const QUOTE_BORDER: [f32; 3] = [0.8, 0.8, 0.8];

const EPS: f32 = 0.01;
/// Horizontal skew used to fake italic when no italic face exists.
const FAKE_ITALIC: f32 = 0.21;

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Renders `content` (markdown) to a paginated A4 PDF at `out`.
///
/// `base_file` is the markdown file the content came from; relative image
/// paths resolve against its directory. The file is written atomically: a
/// temporary sibling is renamed into place only after the PDF validates.
pub fn export(content: &str, base_file: &Path, out: &Path) -> Result<(), String> {
    let bytes = render(content, base_file)?;
    if bytes.is_empty() {
        return Err("PDF export produced no data".into());
    }
    write_atomic(out, &bytes)
}

/// Renders `content` to PDF bytes without touching the filesystem (apart from
/// reading images and system fonts). Never panics: an internal failure comes
/// back as `Err`.
pub fn render(content: &str, base_file: &Path) -> Result<Vec<u8>, String> {
    build_guarded(content, base_file).map(|(bytes, _)| bytes)
}

/// Whether a tab's file can be exported to PDF. Only markdown and plain text:
/// the rest of the file kinds have their own viewers and no printable form.
pub fn supports(path: &str) -> bool {
    crate::paths::is_markdown_file(path)
}

/// The name a PDF export is saved under: the title with its extension swapped
/// for `.pdf` (`Notes.md` becomes `Notes.pdf`).
pub fn suggested_file_name(title: &str) -> String {
    crate::paths::with_extension(title, ".pdf")
}

fn build_guarded(content: &str, base_file: &Path) -> Result<(Vec<u8>, usize), String> {
    match std::panic::catch_unwind(AssertUnwindSafe(|| build(content, base_file))) {
        Ok(result) => result,
        Err(_) => Err("PDF export failed unexpectedly".into()),
    }
}

fn write_atomic(out: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = match out.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let name = out
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("export.pdf");
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    if let Err(err) = write_file(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, out)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("could not write {}: {err}", out.display()));
    }
    Ok(())
}

fn write_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn build(content: &str, base_file: &Path) -> Result<(Vec<u8>, usize), String> {
    let doc = preview::parse(content);
    let fonts = load_fonts()?;
    let mut layout = Layout::new(&fonts, base_file)?;
    layout.blocks(
        &doc.blocks,
        Ctx {
            x: MARGIN_X,
            width: CONTENT_W,
            color: BLACK,
        },
    );
    let title = base_file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("Untitled")
        .to_string();
    let pages = layout.pages.len();
    let bytes = layout.write_pdf(&title)?;
    if !bytes.starts_with(b"%PDF") || !bytes.trim_ascii_end().ends_with(b"%%EOF") {
        return Err("PDF export produced an invalid file".into());
    }
    Ok((bytes, pages))
}

// ---------------------------------------------------------------------------
// Fonts
// ---------------------------------------------------------------------------

const FONT_DIRS: [&str; 3] = [
    "/System/Library/Fonts/Supplemental",
    "/Library/Fonts",
    "/System/Library/Fonts",
];
/// Sans family: regular, bold, italic, bold italic.
const SANS: [&str; 4] = [
    "Arial.ttf",
    "Arial Bold.ttf",
    "Arial Italic.ttf",
    "Arial Bold Italic.ttf",
];
/// Monospace family: regular, bold, italic, bold italic.
const MONO: [&str; 4] = [
    "Courier New.ttf",
    "Courier New Bold.ttf",
    "Courier New Italic.ttf",
    "Courier New Bold Italic.ttf",
];
/// Broad-coverage fallbacks for glyphs the Latin faces lack (Thai, CJK, symbols).
const UNICODE_FALLBACK: &str = "Arial Unicode.ttf";
const THAI_FALLBACK: &str = "Thonburi.ttc";

struct FontFile {
    name: String,
    bytes: Vec<u8>,
    index: u32,
    bold: bool,
    italic: bool,
}

struct Fonts {
    files: Vec<FontFile>,
    /// `family[0]` is sans, `family[1]` is mono; each holds
    /// `[regular, bold, italic, bold italic]` as indices into `files`.
    family: [[usize; 4]; 2],
    /// Consulted in order when the chosen face lacks a character.
    fallback: Vec<usize>,
}

fn load_file(
    files: &mut Vec<FontFile>,
    name: &str,
    index: u32,
    bold: bool,
    italic: bool,
) -> Option<usize> {
    if let Some(pos) = files.iter().position(|file| file.name == name) {
        return Some(pos);
    }
    let path = FONT_DIRS
        .iter()
        .map(|dir| Path::new(dir).join(name))
        .find(|path| path.is_file())?;
    let bytes = std::fs::read(path).ok()?;
    ttf_parser::Face::parse(&bytes, index).ok()?;
    files.push(FontFile {
        name: name.to_string(),
        bytes,
        index,
        bold,
        italic,
    });
    Some(files.len() - 1)
}

fn load_fonts() -> Result<Fonts, String> {
    let mut files = Vec::new();
    let mut loaded: [[Option<usize>; 4]; 2] = [[None; 4]; 2];
    for (fam, names) in [SANS, MONO].iter().enumerate() {
        for (style, name) in names.iter().enumerate() {
            loaded[fam][style] = load_file(&mut files, name, 0, style & 1 != 0, style & 2 != 0);
        }
    }
    let unicode = load_file(&mut files, UNICODE_FALLBACK, 0, false, false);
    let thai = load_file(&mut files, THAI_FALLBACK, 0, false, false);

    let mut fallback = Vec::new();
    for file in [unicode, thai, loaded[0][0]].into_iter().flatten() {
        if !fallback.contains(&file) {
            fallback.push(file);
        }
    }
    let general = fallback
        .first()
        .copied()
        .or(loaded[1][0])
        .ok_or("no usable system font was found")?;

    // A missing face falls back to its family's regular face, then to the
    // general-coverage face, so every style resolves to something.
    let mut family = [[0usize; 4]; 2];
    for fam in 0..2 {
        let regular = loaded[fam][0].unwrap_or(general);
        for style in 0..4 {
            family[fam][style] = loaded[fam][style].unwrap_or(regular);
        }
    }
    Ok(Fonts {
        files,
        family,
        fallback,
    })
}

// ---------------------------------------------------------------------------
// Shaped glyphs, pieces and lines
// ---------------------------------------------------------------------------

/// A shaped glyph in em units, independent of font size.
#[derive(Clone)]
struct Raw {
    file: u16,
    gid: u16,
    ch: char,
    /// Advance after shaping (includes kerning and mark positioning).
    adv: f32,
    /// Advance as stored in the font's hmtx; what the PDF widths will say.
    w: f32,
    off: f32,
    yoff: f32,
    sb: bool,
    si: bool,
}

#[derive(Clone, Copy)]
struct Glyph {
    file: u16,
    gid: u16,
    ch: char,
    adv: f32,
    w: f32,
    off: f32,
    yoff: f32,
    sb: bool,
    si: bool,
    size: f32,
    color: [f32; 3],
    code: bool,
    underline: bool,
    strike: bool,
    /// Pen position from the start of the line, in points.
    x: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct TStyle {
    size: f32,
    color: [f32; 3],
    bold: bool,
    italic: bool,
    mono: bool,
    code: bool,
    underline: bool,
    strike: bool,
}

impl TStyle {
    fn plain(size: f32, color: [f32; 3]) -> Self {
        Self {
            size,
            color,
            bold: false,
            italic: false,
            mono: false,
            code: false,
            underline: false,
            strike: false,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Word,
    Space,
    Break,
}

struct Piece {
    kind: Kind,
    glyphs: Vec<Glyph>,
    width: f32,
    /// A line may wrap before this piece.
    brk: bool,
}

struct Pending {
    kind: Kind,
    text: String,
    brk: bool,
    style: TStyle,
}

#[derive(Clone)]
struct Line {
    glyphs: Vec<Glyph>,
    width: f32,
    height: f32,
    /// Distance from the top of the line to the baseline.
    base: f32,
}

// ---------------------------------------------------------------------------
// Drawing operations (top-down coordinates, points)
// ---------------------------------------------------------------------------

enum Op {
    Text {
        x: f32,
        baseline: f32,
        glyphs: Vec<Glyph>,
    },
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        rgb: [f32; 3],
    },
    Frame {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        lw: f32,
        rgb: [f32; 3],
    },
    Poly {
        pts: Vec<[f32; 2]>,
        lw: f32,
        rgb: [f32; 3],
    },
    Image {
        idx: usize,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
}

struct ImageData {
    w: u32,
    h: u32,
    rgb: Vec<u8>,
}

#[derive(Clone, Copy)]
struct Ctx {
    x: f32,
    width: f32,
    color: [f32; 3],
}

#[derive(Clone, Copy)]
struct BoxOpen {
    page: usize,
    top: f32,
    index: usize,
}

#[derive(Default)]
struct BoxStyle {
    fill: Option<[f32; 3]>,
    border: Option<[f32; 3]>,
    bar: Option<([f32; 3], f32)>,
}

#[derive(Clone)]
struct Row {
    head: bool,
    cells: Vec<Vec<Line>>,
    height: f32,
}

enum Marker {
    Bullet,
    Number(String),
    Task(bool),
}

// ---------------------------------------------------------------------------
// Text breaking
// ---------------------------------------------------------------------------

fn is_thai(c: char) -> bool {
    matches!(c as u32, 0x0E01..=0x0E5B)
}

/// Characters that start a new Thai syllable (consonants, digits, leading vowels).
fn is_thai_base(c: char) -> bool {
    matches!(c as u32, 0x0E01..=0x0E2E | 0x0E40..=0x0E44 | 0x0E50..=0x0E59)
}

fn is_leading_vowel(c: char) -> bool {
    matches!(c as u32, 0x0E40..=0x0E44)
}

fn is_cjk(c: char) -> bool {
    matches!(
        c as u32,
        0x2E80..=0x2FDF
            | 0x3040..=0x30FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xAC00..=0xD7AF
            | 0xF900..=0xFAFF
            | 0xFF00..=0xFFEF
    )
}

fn is_zero_width(c: char) -> bool {
    matches!(c as u32, 0x200B..=0x200D | 0x2060 | 0xFEFF)
}

/// Whether a line may break between `prev` and `next` (no space between them).
fn break_between(prev: char, next: char) -> bool {
    if is_cjk(prev) || is_cjk(next) {
        return true;
    }
    if is_thai(prev) && is_thai_base(next) && !is_leading_vowel(prev) {
        return true;
    }
    prev == '-' && next.is_alphanumeric()
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

struct Layout<'a> {
    fonts: &'a Fonts,
    faces: Vec<rustybuzz::Face<'a>>,
    tt: Vec<ttf_parser::Face<'a>>,
    units: Vec<f32>,
    /// (ascender, descender) per font file, in em, descender positive.
    metrics: Vec<(f32, f32)>,
    cache: HashMap<(String, bool, bool, bool), Rc<Vec<Raw>>>,
    pages: Vec<Vec<Op>>,
    y: f32,
    images: Vec<ImageData>,
    base_dir: PathBuf,
}

impl<'a> Layout<'a> {
    fn new(fonts: &'a Fonts, base_file: &Path) -> Result<Self, String> {
        let mut faces = Vec::with_capacity(fonts.files.len());
        let mut tt = Vec::with_capacity(fonts.files.len());
        let mut units = Vec::with_capacity(fonts.files.len());
        let mut metrics = Vec::with_capacity(fonts.files.len());
        for file in &fonts.files {
            let face = rustybuzz::Face::from_slice(&file.bytes, file.index)
                .ok_or("a system font could not be shaped")?;
            let parsed = ttf_parser::Face::parse(&file.bytes, file.index)
                .map_err(|_| "a system font could not be parsed".to_string())?;
            let upem = f32::from(parsed.units_per_em().max(1));
            metrics.push((
                f32::from(parsed.ascender()) / upem,
                (-f32::from(parsed.descender())).max(0.0) / upem,
            ));
            units.push(upem);
            faces.push(face);
            tt.push(parsed);
        }
        Ok(Self {
            fonts,
            faces,
            tt,
            units,
            metrics,
            cache: HashMap::new(),
            pages: vec![Vec::new()],
            y: TOP,
            images: Vec::new(),
            base_dir: base_file
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default(),
        })
    }

    // -- pages and cursor ---------------------------------------------------

    fn new_page(&mut self) {
        self.pages.push(Vec::new());
        self.y = TOP;
    }

    /// Starts a new page when `h` more points would run past the bottom margin.
    /// Returns true when a break happened.
    fn need(&mut self, h: f32) -> bool {
        if self.y + h > BOTTOM && self.y > TOP + 0.5 {
            self.new_page();
            true
        } else {
            false
        }
    }

    fn at_top(&self) -> bool {
        self.y <= TOP + 0.5
    }

    fn current_page(&self) -> usize {
        self.pages.len().saturating_sub(1)
    }

    fn push(&mut self, op: Op) {
        let page = self.current_page();
        self.push_to(page, op);
    }

    fn push_to(&mut self, page: usize, op: Op) {
        if let Some(ops) = self.pages.get_mut(page) {
            ops.push(op);
        }
    }

    /// Records where a box starts so its background can be inserted behind
    /// the content drawn inside it.
    fn open_box(&self) -> BoxOpen {
        let page = self.current_page();
        BoxOpen {
            page,
            top: self.y,
            index: self.pages.get(page).map_or(0, Vec::len),
        }
    }

    /// Draws the background, border and bar for a box across every page it
    /// touched, placing them behind the box's content.
    fn close_box(&mut self, open: BoxOpen, x: f32, w: f32, style: BoxStyle) {
        let last = self.current_page();
        for page in open.page..=last {
            let top = if page == open.page { open.top } else { TOP };
            let bottom = if page == last { self.y } else { BOTTOM };
            let h = bottom - top;
            if h <= 0.0 {
                continue;
            }
            let mut ops = Vec::new();
            if let Some(rgb) = style.fill {
                ops.push(Op::Rect { x, y: top, w, h, rgb });
            }
            if let Some(rgb) = style.border {
                ops.push(Op::Frame {
                    x,
                    y: top,
                    w,
                    h,
                    lw: 0.5,
                    rgb,
                });
            }
            if let Some((rgb, bar_w)) = style.bar {
                ops.push(Op::Rect {
                    x,
                    y: top,
                    w: bar_w,
                    h,
                    rgb,
                });
            }
            let at = if page == open.page {
                open.index.min(self.pages[page].len())
            } else {
                0
            };
            for (offset, op) in ops.into_iter().enumerate() {
                self.pages[page].insert(at + offset, op);
            }
        }
    }

    // -- shaping ------------------------------------------------------------

    fn has_face(&self, file: usize, ch: char) -> bool {
        self.tt
            .get(file)
            .is_some_and(|face| face.glyph_index(ch).is_some())
    }

    /// The font file that draws `ch`, plus whether bold/italic must be faked.
    fn pick(&self, ch: char, mono: bool, bold: bool, italic: bool) -> Option<(usize, bool, bool)> {
        let fam = usize::from(mono);
        let style = usize::from(bold) + 2 * usize::from(italic);
        let primary = self.fonts.family[fam][style];
        let regular = self.fonts.family[fam][0];
        let file = [primary, regular]
            .into_iter()
            .chain(self.fonts.fallback.iter().copied())
            .chain(0..self.tt.len())
            .find(|&file| self.has_face(file, ch))?;
        let spec = self.fonts.files.get(file)?;
        Some((file, bold && !spec.bold, italic && !spec.italic))
    }

    fn raws(&mut self, text: &str, mono: bool, bold: bool, italic: bool) -> Rc<Vec<Raw>> {
        let key = (text.to_string(), mono, bold, italic);
        if let Some(hit) = self.cache.get(&key) {
            return Rc::clone(hit);
        }

        let mut out: Vec<Raw> = Vec::new();
        // Consecutive characters that share one font file are shaped together.
        let mut runs: Vec<(usize, usize, usize, bool, bool)> = Vec::new();
        let mut clean = String::with_capacity(text.len());
        for ch in text.chars() {
            let ch = if self.pick(ch, mono, bold, italic).is_some() {
                ch
            } else {
                '?'
            };
            clean.push(ch);
        }
        for (i, ch) in clean.char_indices() {
            let Some((file, sb, si)) = self.pick(ch, mono, bold, italic) else {
                continue;
            };
            let end = i + ch.len_utf8();
            match runs.last_mut() {
                Some(run) if run.0 == file && run.3 == sb && run.4 == si => run.2 = end,
                _ => runs.push((file, i, end, sb, si)),
            }
        }

        for (file, start, end, sb, si) in runs {
            let Some(segment) = clean.get(start..end) else {
                continue;
            };
            let mut buffer = rustybuzz::UnicodeBuffer::new();
            buffer.push_str(segment);
            let shaped = rustybuzz::shape(&self.faces[file], &[], buffer);
            let upem = self.units[file];
            let face = &self.tt[file];
            for (info, pos) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
                let ch = segment
                    .get(info.cluster as usize..)
                    .and_then(|rest| rest.chars().next())
                    .unwrap_or('?');
                let gid = info.glyph_id as u16;
                let hmtx = face.glyph_hor_advance(GlyphId(gid)).map_or(0.0, f32::from);
                out.push(Raw {
                    file: file as u16,
                    gid,
                    ch,
                    adv: pos.x_advance as f32 / upem,
                    w: hmtx / upem,
                    off: pos.x_offset as f32 / upem,
                    yoff: pos.y_offset as f32 / upem,
                    sb,
                    si,
                });
            }
        }

        let rc = Rc::new(out);
        self.cache.insert(key, Rc::clone(&rc));
        rc
    }

    fn glyphs(&mut self, text: &str, style: TStyle) -> Vec<Glyph> {
        let raws = self.raws(text, style.mono, style.bold, style.italic);
        raws.iter()
            .map(|raw| Glyph {
                file: raw.file,
                gid: raw.gid,
                ch: raw.ch,
                adv: raw.adv,
                w: raw.w,
                off: raw.off,
                yoff: raw.yoff,
                sb: raw.sb,
                si: raw.si,
                size: style.size,
                color: style.color,
                code: style.code,
                underline: style.underline,
                strike: style.strike,
                x: 0.0,
            })
            .collect()
    }

    fn piece(&mut self, kind: Kind, text: &str, brk: bool, style: TStyle) -> Piece {
        let glyphs = self.glyphs(text, style);
        let width = glyphs.iter().map(|g| g.adv * g.size).sum::<f32>();
        Piece {
            kind,
            glyphs,
            width,
            brk,
        }
    }

    fn flush(&mut self, pending: &mut Option<Pending>, out: &mut Vec<Piece>) {
        if let Some(p) = pending.take() {
            let piece = self.piece(p.kind, &p.text, p.brk, p.style);
            out.push(piece);
        }
    }

    /// Splits spans into word, space and break pieces. In `preserve` mode
    /// (code) every character is its own piece and spaces are kept.
    fn spans_pieces(&mut self, spans: &[Span], base: TStyle, preserve: bool) -> Vec<Piece> {
        let mut out = Vec::new();
        let mut pending: Option<Pending> = None;
        let mut last: Option<char> = None;

        for span in spans {
            self.flush(&mut pending, &mut out);
            let mut style = base;
            style.bold |= span.bold;
            style.italic |= span.italic;
            style.strike |= span.strike;
            style.underline |= span.link;
            if span.code {
                style.mono = true;
                style.code = true;
                style.size = base.size * CODE_SCALE;
            }

            for ch in span.text.chars() {
                if ch == '\n' {
                    self.flush(&mut pending, &mut out);
                    out.push(Piece {
                        kind: Kind::Break,
                        glyphs: Vec::new(),
                        width: 0.0,
                        brk: false,
                    });
                    last = None;
                    continue;
                }
                if preserve {
                    if ch == '\r' || (ch.is_control() && ch != '\t') {
                        continue;
                    }
                    // Each character is its own piece; a tab becomes four spaces.
                    let count = if ch == '\t' { 4 } else { 1 };
                    for _ in 0..count {
                        let piece = self.piece(Kind::Word, &ch.to_string(), true, style);
                        out.push(piece);
                    }
                    continue;
                }
                if ch.is_control() || is_zero_width(ch) {
                    continue;
                }
                let ch = if ch.is_whitespace() && ch != '\u{a0}' {
                    ' '
                } else {
                    ch
                };
                if ch == ' ' {
                    let extend = matches!(&pending, Some(p) if p.kind == Kind::Space);
                    if extend {
                        if let Some(p) = pending.as_mut() {
                            p.text.push(' ');
                        }
                    } else {
                        self.flush(&mut pending, &mut out);
                        pending = Some(Pending {
                            kind: Kind::Space,
                            text: " ".to_string(),
                            brk: false,
                            style,
                        });
                    }
                } else {
                    let brk = last.is_none_or(|prev| prev == ' ' || break_between(prev, ch));
                    let extend = matches!(&pending, Some(p) if p.kind == Kind::Word && !brk);
                    if extend {
                        if let Some(p) = pending.as_mut() {
                            p.text.push(ch);
                        }
                    } else {
                        self.flush(&mut pending, &mut out);
                        pending = Some(Pending {
                            kind: Kind::Word,
                            text: ch.to_string(),
                            brk,
                            style,
                        });
                    }
                }
                last = Some(ch);
            }
        }
        self.flush(&mut pending, &mut out);
        out
    }

    // -- line breaking ------------------------------------------------------

    fn line(&self, glyphs: Vec<Glyph>, width: f32, size: f32, factor: f32) -> Line {
        let mut asc = 0.0f32;
        let mut desc = 0.0f32;
        for g in &glyphs {
            let (a, d) = self
                .metrics
                .get(g.file as usize)
                .copied()
                .unwrap_or((0.905, 0.212));
            asc = asc.max(a * g.size);
            desc = desc.max(d * g.size);
        }
        if glyphs.is_empty() {
            asc = size * 0.905;
            desc = size * 0.212;
        }
        let height = (asc + desc).max(size * factor);
        let base = (height - (asc + desc)) / 2.0 + asc;
        Line {
            glyphs,
            width,
            height,
            base,
        }
    }

    /// Greedy line filling. Spaces at a line end are dropped. A word that is
    /// wider than the line is split between glyphs. In `preserve` mode nothing
    /// is dropped and nothing collapses.
    fn wrap(&self, pieces: &[Piece], avail: f32, preserve: bool, factor: f32, size: f32) -> Vec<Line> {
        let mut lines = Vec::new();
        let mut cur: Vec<Glyph> = Vec::new();
        let mut cur_w = 0.0f32;
        let mut pend: Vec<Glyph> = Vec::new();
        let mut pend_w = 0.0f32;

        for piece in pieces {
            match piece.kind {
                Kind::Break => {
                    lines.push(self.line(std::mem::take(&mut cur), cur_w, size, factor));
                    cur_w = 0.0;
                    pend.clear();
                    pend_w = 0.0;
                }
                Kind::Space if !preserve => {
                    if !cur.is_empty() {
                        pend.extend_from_slice(&piece.glyphs);
                        pend_w += piece.width;
                    }
                }
                _ => {
                    if !cur.is_empty() && piece.brk && cur_w + pend_w + piece.width > avail + EPS {
                        lines.push(self.line(std::mem::take(&mut cur), cur_w, size, factor));
                        cur_w = 0.0;
                        pend.clear();
                    }
                    for mut g in pend.drain(..) {
                        g.x = cur_w;
                        cur_w += g.adv * g.size;
                        cur.push(g);
                    }
                    pend_w = 0.0;
                    for glyph in &piece.glyphs {
                        let mut g = *glyph;
                        let gw = g.adv * g.size;
                        if !cur.is_empty() && cur_w + gw > avail + EPS {
                            lines.push(self.line(std::mem::take(&mut cur), cur_w, size, factor));
                            cur_w = 0.0;
                        }
                        g.x = cur_w;
                        cur_w += gw;
                        cur.push(g);
                    }
                }
            }
        }
        if !cur.is_empty() || lines.is_empty() {
            lines.push(self.line(cur, cur_w, size, factor));
        }
        lines
    }

    /// Places a line at `y`, aligned inside `avail` starting at `x`.
    fn place(&mut self, line: Line, x: f32, avail: f32, align: Align, y: f32) {
        let dx = match align {
            Align::Left => 0.0,
            Align::Center => ((avail - line.width) / 2.0).max(0.0),
            Align::Right => (avail - line.width).max(0.0),
        };
        let baseline = y + line.base;
        self.push(Op::Text {
            x: x + dx,
            baseline,
            glyphs: line.glyphs,
        });
    }

    /// Emits lines from the cursor, breaking pages between lines. Returns the
    /// page and baseline of the first line.
    fn emit(&mut self, lines: Vec<Line>, x: f32, avail: f32, align: Align) -> Option<(usize, f32)> {
        let mut first = None;
        for line in lines {
            self.need(line.height);
            let y = self.y;
            if first.is_none() {
                first = Some((self.current_page(), y + line.base));
            }
            let height = line.height;
            self.place(line, x, avail, align, y);
            self.y += height;
        }
        first
    }

    // -- blocks -------------------------------------------------------------

    fn blocks(&mut self, blocks: &[Block], ctx: Ctx) {
        for block in blocks {
            self.block(block, ctx);
        }
    }

    fn block(&mut self, block: &Block, ctx: Ctx) {
        match block {
            Block::Heading { level, spans } => self.heading(*level, spans, ctx),
            Block::Paragraph { spans } => {
                self.paragraph(spans, ctx, true);
            }
            Block::Code { text, .. } => self.code(text, ctx),
            Block::Quote { kind, blocks } => self.quote(*kind, blocks, ctx),
            Block::List {
                ordered,
                start,
                items,
            } => self.list(*ordered, *start, items, ctx),
            Block::Table {
                aligns,
                header,
                rows,
            } => self.table(aligns, header, rows, ctx),
            Block::Rule => self.rule(ctx),
            Block::Image { src, alt } => self.image(src, alt, ctx),
        }
    }

    fn heading(&mut self, level: u8, spans: &[Span], ctx: Ctx) {
        let size = heading_size(level);
        if !self.at_top() {
            self.y += 1.4 * size;
        }
        // Keep the heading with at least the start of the next paragraph.
        self.need(size * HEAD_LINE + 2.0 * BODY * LINE);
        let style = TStyle {
            bold: true,
            ..TStyle::plain(size, ctx.color)
        };
        let pieces = self.spans_pieces(spans, style, false);
        if pieces.iter().all(|p| p.glyphs.is_empty()) {
            return;
        }
        let lines = self.wrap(&pieces, ctx.width, false, HEAD_LINE, size);
        self.emit(lines, ctx.x, ctx.width, Align::Left);
        self.y += 0.5 * size;
    }

    fn paragraph(&mut self, spans: &[Span], ctx: Ctx, gap: bool) -> Option<(usize, f32)> {
        let pieces = self.spans_pieces(spans, TStyle::plain(BODY, ctx.color), false);
        if pieces.iter().all(|p| p.glyphs.is_empty()) {
            return None;
        }
        let lines = self.wrap(&pieces, ctx.width, false, LINE, BODY);
        let first = self.emit(lines, ctx.x, ctx.width, Align::Left);
        if gap {
            self.y += GAP;
        }
        first
    }

    fn code(&mut self, text: &str, ctx: Ctx) {
        let style = TStyle {
            mono: true,
            ..TStyle::plain(BODY * CODE_SCALE, BLACK)
        };
        let inner_w = (ctx.width - 2.0 * PRE_PAD_X).max(10.0);
        let mut lines = Vec::new();
        for source in text.split('\n') {
            let span = Span {
                text: source.to_string(),
                ..Default::default()
            };
            let pieces = self.spans_pieces(&[span], style, true);
            lines.extend(self.wrap(&pieces, inner_w, true, CODE_LINE, style.size));
        }

        // Keep a code block whole when it fits on one page.
        let body: f32 = lines.iter().map(|line| line.height).sum();
        if body + 2.0 * PRE_PAD_Y <= CONTENT_H {
            self.need(body + 2.0 * PRE_PAD_Y);
        }
        let open = self.open_box();
        self.y += PRE_PAD_Y;
        for line in lines {
            self.need(line.height);
            let y = self.y;
            let height = line.height;
            self.place(line, ctx.x + PRE_PAD_X, inner_w, Align::Left, y);
            self.y += height;
        }
        self.y += PRE_PAD_Y;
        self.close_box(
            open,
            ctx.x,
            ctx.width,
            BoxStyle {
                fill: Some(CODE_BG),
                border: Some(PRE_BORDER),
                bar: None,
            },
        );
        self.y += GAP;
    }

    fn quote(&mut self, kind: Option<Callout>, blocks: &[Block], ctx: Ctx) {
        let bar_colour = kind.map_or(QUOTE_BORDER, callout_colour);
        let inner = Ctx {
            x: ctx.x + QUOTE_BAR + QUOTE_PAD,
            width: ctx.width - QUOTE_BAR - QUOTE_PAD,
            color: GREY_TEXT,
        };
        let open = self.open_box();
        if let Some(kind) = kind {
            let label = Span {
                text: callout_label(kind).to_string(),
                bold: true,
                ..Default::default()
            };
            self.paragraph(&[label], Ctx { color: bar_colour, ..inner }, false);
        }
        self.blocks(blocks, inner);
        self.close_box(
            open,
            ctx.x,
            ctx.width,
            BoxStyle {
                bar: Some((bar_colour, QUOTE_BAR)),
                ..BoxStyle::default()
            },
        );
    }

    fn list(&mut self, ordered: bool, start: u64, items: &[ListItem], ctx: Ctx) {
        let inner = Ctx {
            x: ctx.x + LIST_INDENT,
            width: ctx.width - LIST_INDENT,
            color: ctx.color,
        };
        for (index, item) in items.iter().enumerate() {
            let marker = match item.checked {
                Some(done) => Marker::Task(done),
                None if ordered => Marker::Number(format!("{}.", start + index as u64)),
                None => Marker::Bullet,
            };
            self.need(BODY * LINE);

            let mut first_pos = None;
            let mut skip = 0;
            if let Some(Block::Paragraph { spans }) = item.blocks.first() {
                skip = 1;
                first_pos = self.paragraph(spans, inner, false);
            }
            if first_pos.is_none() {
                // The item opens with a nested block: give the marker its own line.
                let page = self.current_page();
                let baseline = self.y + self.line(Vec::new(), 0.0, BODY, LINE).base;
                self.y += BODY * LINE;
                first_pos = Some((page, baseline));
            }
            if let Some((page, baseline)) = first_pos {
                self.draw_marker(page, baseline, inner.x, &marker);
            }
            for block in item.blocks.iter().skip(skip) {
                self.block(block, inner);
            }
            self.y += LI_GAP;
        }
        self.y += GAP;
    }

    fn draw_marker(&mut self, page: usize, baseline: f32, inner_x: f32, marker: &Marker) {
        let right = inner_x - 6.0;
        match marker {
            Marker::Task(done) => {
                let size = 8.0;
                let left = right - size;
                let top = baseline - size + 0.6;
                self.push_to(
                    page,
                    Op::Frame {
                        x: left,
                        y: top,
                        w: size,
                        h: size,
                        lw: 0.8,
                        rgb: BLACK,
                    },
                );
                if *done {
                    self.push_to(
                        page,
                        Op::Poly {
                            pts: vec![
                                [left + 1.8, top + 4.3],
                                [left + 3.4, top + 6.1],
                                [left + 6.4, top + 1.9],
                            ],
                            lw: 1.1,
                            rgb: BLACK,
                        },
                    );
                }
            }
            Marker::Bullet | Marker::Number(_) => {
                let text = match marker {
                    Marker::Number(text) => text.as_str(),
                    _ => "\u{2022}",
                };
                let mut glyphs = self.glyphs(text, TStyle::plain(BODY, BLACK));
                let mut pen = 0.0f32;
                for g in &mut glyphs {
                    g.x = pen;
                    pen += g.adv * g.size;
                }
                let x = right - pen;
                self.push_to(
                    page,
                    Op::Text {
                        x,
                        baseline,
                        glyphs,
                    },
                );
            }
        }
    }

    fn table(&mut self, aligns: &[Align], header: &[Vec<Span>], rows: &[Vec<Vec<Span>>], ctx: Ctx) {
        let cols = aligns
            .len()
            .max(header.len())
            .max(rows.iter().map(Vec::len).max().unwrap_or(0));
        if cols == 0 {
            return;
        }
        let size = TABLE_SCALE * BODY;
        let head_style = TStyle {
            bold: true,
            ..TStyle::plain(size, ctx.color)
        };
        let body_style = TStyle::plain(size, ctx.color);

        let mut grid: Vec<(bool, Vec<Vec<Piece>>)> = Vec::new();
        if !header.is_empty() {
            let cells = self.row_pieces(header, cols, head_style);
            grid.push((true, cells));
        }
        for row in rows {
            let cells = self.row_pieces(row, cols, body_style);
            grid.push((false, cells));
        }

        let mut natural = vec![0.0f32; cols];
        let mut minimum = vec![0.0f32; cols];
        for (_, cells) in &grid {
            for (c, cell) in cells.iter().enumerate() {
                let total: f32 = cell.iter().map(|p| p.width).sum();
                let longest = cell
                    .iter()
                    .filter(|p| p.kind == Kind::Word)
                    .map(|p| p.width)
                    .fold(0.0, f32::max);
                natural[c] = natural[c].max(total + 2.0 * CELL_PAD_X);
                minimum[c] = minimum[c].max(longest + 2.0 * CELL_PAD_X);
            }
        }
        let widths = column_widths(&natural, &minimum, ctx.width);

        let mut laid: Vec<Row> = Vec::with_capacity(grid.len());
        for (head, cells) in &grid {
            let mut lines_per_cell = Vec::with_capacity(cols);
            let mut content: f32 = 0.0;
            for (c, cell) in cells.iter().enumerate() {
                let inner = (widths[c] - 2.0 * CELL_PAD_X).max(1.0);
                let lines = self.wrap(cell, inner, false, LINE, size);
                let height: f32 = lines.iter().map(|line| line.height).sum();
                content = content.max(height);
                lines_per_cell.push(lines);
            }
            laid.push(Row {
                head: *head,
                cells: lines_per_cell,
                height: content + 2.0 * CELL_PAD_Y,
            });
        }

        let head_row = laid.first().filter(|row| row.head).cloned();
        for row in &laid {
            let broke = self.need(row.height);
            if broke && !row.head {
                if let Some(head) = &head_row {
                    self.draw_row(head, ctx.x, &widths, aligns);
                }
            }
            self.draw_row(row, ctx.x, &widths, aligns);
        }
        self.y += GAP;
    }

    fn row_pieces(&mut self, cells: &[Vec<Span>], cols: usize, style: TStyle) -> Vec<Vec<Piece>> {
        let mut out = Vec::with_capacity(cols);
        for c in 0..cols {
            let pieces = match cells.get(c) {
                Some(cell) => self.spans_pieces(cell, style, false),
                None => Vec::new(),
            };
            out.push(pieces);
        }
        out
    }

    fn draw_row(&mut self, row: &Row, x0: f32, widths: &[f32], aligns: &[Align]) {
        let top = self.y;
        let page = self.current_page();
        let mut x = x0;
        for (c, width) in widths.iter().enumerate() {
            let width = *width;
            if row.head {
                self.push_to(
                    page,
                    Op::Rect {
                        x,
                        y: top,
                        w: width,
                        h: row.height,
                        rgb: HEAD_BG,
                    },
                );
            }
            self.push_to(
                page,
                Op::Frame {
                    x,
                    y: top,
                    w: width,
                    h: row.height,
                    lw: 0.5,
                    rgb: TABLE_BORDER,
                },
            );
            let align = aligns.get(c).copied().unwrap_or(Align::Left);
            let mut y = top + CELL_PAD_Y;
            if let Some(lines) = row.cells.get(c) {
                for line in lines {
                    let inner = width - 2.0 * CELL_PAD_X;
                    let dx = match align {
                        Align::Left => 0.0,
                        Align::Center => ((inner - line.width) / 2.0).max(0.0),
                        Align::Right => (inner - line.width).max(0.0),
                    };
                    self.push_to(
                        page,
                        Op::Text {
                            x: x + CELL_PAD_X + dx,
                            baseline: y + line.base,
                            glyphs: line.glyphs.clone(),
                        },
                    );
                    y += line.height;
                }
            }
            x += width;
        }
        self.y = top + row.height;
    }

    fn rule(&mut self, ctx: Ctx) {
        let gap = 1.5 * BODY;
        self.need(2.0 * gap + 1.0);
        self.y += gap;
        let y = self.y;
        self.push(Op::Rect {
            x: ctx.x,
            y,
            w: ctx.width,
            h: 0.75,
            rgb: RULE,
        });
        self.y += 0.75 + gap;
    }

    fn image(&mut self, src: &str, alt: &str, ctx: Ctx) {
        match self.load_image(src) {
            Some((idx, w, h)) => {
                let max_h = MAX_IMAGE_H.min(CONTENT_H * 0.9);
                let scale = (ctx.width / w).min(max_h / h).min(1.0);
                let (dw, dh) = (w * scale, h * scale);
                self.need(dh);
                let y = self.y;
                self.push(Op::Image {
                    idx,
                    x: ctx.x,
                    y,
                    w: dw,
                    h: dh,
                });
                self.y += dh + GAP;
            }
            None => {
                let label = if alt.is_empty() {
                    format!("[image] {src}")
                } else {
                    format!("[image] {alt}")
                };
                let span = Span {
                    text: label,
                    ..Default::default()
                };
                self.paragraph(
                    &[span],
                    Ctx {
                        color: MUTED,
                        ..ctx
                    },
                    true,
                );
            }
        }
    }

    /// Decodes a local PNG or JPEG, flattened onto white. Returns the image
    /// index and its size in points.
    fn load_image(&mut self, src: &str) -> Option<(usize, f32, f32)> {
        let path = resolve_image(src, &self.base_dir)?;
        let decoded = image::ImageReader::open(&path)
            .ok()?
            .with_guessed_format()
            .ok()?
            .decode()
            .ok()?;
        let (w, h) = (decoded.width(), decoded.height());
        if w == 0 || h == 0 {
            return None;
        }
        let decoded = if w > MAX_IMAGE_PX || h > MAX_IMAGE_PX {
            decoded.thumbnail(MAX_IMAGE_PX, MAX_IMAGE_PX)
        } else {
            decoded
        };
        let rgba = decoded.to_rgba8();
        let (tw, th) = rgba.dimensions();
        let mut rgb = Vec::with_capacity(tw as usize * th as usize * 3);
        for pixel in rgba.pixels() {
            let [r, g, b, a] = pixel.0;
            let alpha = u32::from(a);
            let blend = |c: u8| ((u32::from(c) * alpha + 255 * (255 - alpha)) / 255) as u8;
            rgb.extend_from_slice(&[blend(r), blend(g), blend(b)]);
        }
        let idx = self.images.len();
        self.images.push(ImageData { w: tw, h: th, rgb });
        Some((idx, w as f32 * PX_TO_PT, h as f32 * PX_TO_PT))
    }

    // -- PDF output ---------------------------------------------------------

    fn write_pdf(self, title: &str) -> Result<Vec<u8>, String> {
        let files = &self.fonts.files;

        // Which glyphs each font file draws, and a representative character for each.
        let mut used: Vec<BTreeMap<u16, char>> = vec![BTreeMap::new(); files.len()];
        for ops in &self.pages {
            for op in ops {
                if let Op::Text { glyphs, .. } = op {
                    for g in glyphs {
                        if let Some(map) = used.get_mut(g.file as usize) {
                            map.entry(g.gid).or_insert(g.ch);
                        }
                    }
                }
            }
        }

        let mut remaps: Vec<Option<GlyphRemapper>> = Vec::with_capacity(files.len());
        let mut subsets: Vec<Option<Vec<u8>>> = Vec::with_capacity(files.len());
        for (f, map) in used.iter().enumerate() {
            if map.is_empty() {
                remaps.push(None);
                subsets.push(None);
                continue;
            }
            let mut remap = GlyphRemapper::new();
            for gid in map.keys() {
                remap.remap(*gid);
            }
            let file = &files[f];
            let subset = subsetter::subset(&file.bytes, file.index, &remap)
                .map_err(|err| format!("could not subset {}: {err:?}", file.name))?;
            remaps.push(Some(remap));
            subsets.push(Some(subset));
        }

        let mut next = 1;
        let mut alloc = move || {
            let id = Ref::new(next);
            next += 1;
            id
        };
        let catalog_id = alloc();
        let tree_id = alloc();
        let info_id = alloc();

        let mut pdf = Pdf::new();
        pdf.catalog(catalog_id).pages(tree_id);
        pdf.document_info(info_id)
            .title(TextStr(title))
            .producer(TextStr("Zyplus"));

        // Fonts: one Type0 / CIDFontType2 per used file.
        struct FontRefs {
            file: usize,
            type0: Ref,
        }
        let mut font_refs: Vec<FontRefs> = Vec::new();
        let mut name_index = 0usize;
        for (f, map) in used.iter().enumerate() {
            let (Some(remap), Some(subset)) = (&remaps[f], &subsets[f]) else {
                continue;
            };
            let file = &files[f];
            let face = &self.tt[f];
            let upem = self.units[f];
            let scale = 1000.0 / upem;

            let type0_id = alloc();
            let cid_id = alloc();
            let desc_id = alloc();
            let data_id = alloc();
            let cmap_id = alloc();

            // Widths indexed by new glyph id, from the font's own advances.
            let mut widths: Vec<f32> = Vec::new();
            let mut cmap_entries: Vec<(u16, char)> = Vec::new();
            for (old, ch) in map {
                let Some(new) = remap.get(*old) else {
                    continue;
                };
                let new = usize::from(new);
                if widths.len() <= new {
                    widths.resize(new + 1, 0.0);
                }
                let hmtx = face.glyph_hor_advance(GlyphId(*old)).map_or(0.0, f32::from);
                widths[new] = hmtx * scale;
                cmap_entries.push((new as u16, *ch));
            }
            let otf = subset.starts_with(b"OTTO");

            let tag = format!("ZYP{name_index:03}");
            name_index += 1;
            let base: String = file.name.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
            let name = format!("{tag}+{base}");

            let compressed = deflate(subset);
            let mut stream = pdf.stream(data_id, &compressed);
            stream.filter(Filter::FlateDecode);
            if otf {
                stream.pair(Name(b"Subtype"), Name(b"OpenType"));
            } else {
                stream.pair(Name(b"Length1"), subset.len() as i32);
            }
            drop(stream);

            let cmap = to_unicode(&cmap_entries);
            let compressed_cmap = deflate(cmap.as_bytes());
            let mut cmap_stream = pdf.stream(cmap_id, &compressed_cmap);
            cmap_stream.filter(Filter::FlateDecode);
            drop(cmap_stream);

            let ascent = f32::from(face.ascender()) * scale;
            let descent = f32::from(face.descender()) * scale;
            let cap = f32::from(face.capital_height().unwrap_or(700)) * scale;
            let bbox = face.global_bounding_box();
            let mut descriptor = pdf.font_descriptor(desc_id);
            descriptor
                .name(Name(name.as_bytes()))
                .flags(FontFlags::NON_SYMBOLIC)
                .bbox(Rect::new(
                    f32::from(bbox.x_min) * scale,
                    f32::from(bbox.y_min) * scale,
                    f32::from(bbox.x_max) * scale,
                    f32::from(bbox.y_max) * scale,
                ))
                .italic_angle(face.italic_angle())
                .ascent(ascent)
                .descent(descent)
                .cap_height(cap)
                .stem_v(80.0);
            if otf {
                descriptor.font_file3(data_id);
            } else {
                descriptor.font_file2(data_id);
            }
            drop(descriptor);

            let mut cid = pdf.cid_font(cid_id);
            cid.subtype(if otf {
                CidFontType::Type0
            } else {
                CidFontType::Type2
            });
            cid.base_font(Name(name.as_bytes()));
            cid.system_info(SystemInfo {
                registry: Str(b"Adobe"),
                ordering: Str(b"Identity"),
                supplement: 0,
            });
            cid.font_descriptor(desc_id);
            cid.default_width(0.0);
            cid.widths().consecutive(0, widths.iter().copied());
            drop(cid);

            let mut type0 = pdf.type0_font(type0_id);
            type0.base_font(Name(name.as_bytes()));
            type0.encoding_predefined(Name(b"Identity-H"));
            type0.descendant_font(cid_id);
            type0.to_unicode(cmap_id);
            drop(type0);

            font_refs.push(FontRefs {
                file: f,
                type0: type0_id,
            });
        }

        // Images.
        let mut image_refs: Vec<Ref> = Vec::with_capacity(self.images.len());
        for image in &self.images {
            let id = alloc();
            let compressed = deflate(&image.rgb);
            let mut xobject = pdf.image_xobject(id, &compressed);
            xobject.filter(Filter::FlateDecode);
            xobject.width(image.w as i32);
            xobject.height(image.h as i32);
            xobject.color_space().device_rgb();
            xobject.bits_per_component(8);
            drop(xobject);
            image_refs.push(id);
        }

        // Pages and their content streams.
        let page_ids: Vec<(Ref, Ref)> = self.pages.iter().map(|_| (alloc(), alloc())).collect();
        pdf.pages(tree_id)
            .kids(page_ids.iter().map(|(page, _)| *page))
            .count(page_ids.len() as i32);

        for (index, ops) in self.pages.iter().enumerate() {
            let (page_id, content_id) = page_ids[index];
            let content = page_content(ops, &remaps);
            let compressed = deflate(content.as_bytes());
            let mut stream = pdf.stream(content_id, &compressed);
            stream.filter(Filter::FlateDecode);
            drop(stream);

            let mut page = pdf.page(page_id);
            page.parent(tree_id)
                .media_box(Rect::new(0.0, 0.0, PAGE_W, PAGE_H))
                .contents(content_id);
            let mut resources = page.resources();
            {
                let mut fonts = resources.fonts();
                for font in &font_refs {
                    let label = format!("F{}", font.file);
                    fonts.pair(Name(label.as_bytes()), font.type0);
                }
            }
            {
                let mut xobjects = resources.x_objects();
                for (idx, id) in image_refs.iter().enumerate() {
                    let label = format!("Im{idx}");
                    xobjects.pair(Name(label.as_bytes()), *id);
                }
            }
            drop(resources);
            drop(page);
        }

        let bytes = pdf.finish();
        if bytes.is_empty() {
            return Err("PDF writer produced no data".into());
        }
        Ok(bytes)
    }
}

fn heading_size(level: u8) -> f32 {
    match level {
        1 => 1.9 * BODY,
        2 => 1.5 * BODY,
        3 => 1.25 * BODY,
        4 => BODY,
        5 => 10.0,
        _ => 9.5,
    }
}

fn callout_colour(kind: Callout) -> [f32; 3] {
    match kind {
        Callout::Note => [0.035, 0.408, 0.855],
        Callout::Tip => [0.102, 0.498, 0.216],
        Callout::Important => [0.510, 0.310, 0.875],
        Callout::Warning => [0.604, 0.404, 0.0],
        Callout::Caution => [0.812, 0.133, 0.180],
    }
}

fn callout_label(kind: Callout) -> &'static str {
    match kind {
        Callout::Note => "NOTE",
        Callout::Tip => "TIP",
        Callout::Important => "IMPORTANT",
        Callout::Warning => "WARNING",
        Callout::Caution => "CAUTION",
    }
}

/// Splits the content width between columns. Columns grow to fill the width
/// when everything fits; otherwise they shrink toward their longest word.
fn column_widths(natural: &[f32], minimum: &[f32], avail: f32) -> Vec<f32> {
    let n = natural.len().max(1) as f32;
    let nat_sum: f32 = natural.iter().sum();
    let min_sum: f32 = minimum.iter().sum();
    if nat_sum > 0.0 && nat_sum <= avail {
        natural.iter().map(|w| w * avail / nat_sum).collect()
    } else if min_sum > 0.0 && min_sum <= avail {
        let spare = avail - min_sum;
        let flex = nat_sum - min_sum;
        natural
            .iter()
            .zip(minimum)
            .map(|(nat, min)| {
                min + if flex > 0.0 {
                    (nat - min) * spare / flex
                } else {
                    spare / n
                }
            })
            .collect()
    } else if min_sum > 0.0 {
        minimum.iter().map(|m| m * avail / min_sum).collect()
    } else {
        vec![avail / n; natural.len()]
    }
}

fn resolve_image(src: &str, base_dir: &Path) -> Option<PathBuf> {
    if src.is_empty()
        || src.starts_with("http://")
        || src.starts_with("https://")
        || src.starts_with("data:")
    {
        return None;
    }
    let trimmed = src.split(['#', '?']).next().unwrap_or(src);
    let decoded = trimmed.replace("%20", " ");
    let path = Path::new(&decoded);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    };
    path.is_file().then_some(path)
}

// ---------------------------------------------------------------------------
// PDF content and objects
// ---------------------------------------------------------------------------

fn deflate(data: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    let _ = encoder.write_all(data);
    encoder.finish().unwrap_or_default()
}

fn num(v: f32) -> String {
    if v.is_finite() {
        format!("{v:.3}")
    } else {
        "0".to_string()
    }
}

fn rgb_op(rgb: [f32; 3]) -> String {
    format!("{} {} {}", num(rgb[0]), num(rgb[1]), num(rgb[2]))
}

fn gid_of(glyph: &Glyph, remaps: &[Option<GlyphRemapper>]) -> u16 {
    remaps
        .get(glyph.file as usize)
        .and_then(Option::as_ref)
        .and_then(|remap| remap.get(glyph.gid))
        .unwrap_or(0)
}

fn page_content(ops: &[Op], remaps: &[Option<GlyphRemapper>]) -> String {
    let mut s = String::new();
    for op in ops {
        match op {
            Op::Rect { x, y, w, h, rgb } => {
                let _ = writeln!(
                    s,
                    "{} rg {} {} {} {} re f",
                    rgb_op(*rgb),
                    num(*x),
                    num(PAGE_H - y - h),
                    num(*w),
                    num(*h)
                );
            }
            Op::Frame {
                x,
                y,
                w,
                h,
                lw,
                rgb,
            } => {
                let _ = writeln!(
                    s,
                    "{} RG {} w {} {} {} {} re S",
                    rgb_op(*rgb),
                    num(*lw),
                    num(*x),
                    num(PAGE_H - y - h),
                    num(*w),
                    num(*h)
                );
            }
            Op::Poly { pts, lw, rgb } => {
                let _ = write!(s, "{} RG {} w ", rgb_op(*rgb), num(*lw));
                for (i, [x, y]) in pts.iter().enumerate() {
                    let op = if i == 0 { "m" } else { "l" };
                    let _ = write!(s, "{} {} {op} ", num(*x), num(PAGE_H - y));
                }
                s.push_str("S\n");
            }
            Op::Image { idx, x, y, w, h } => {
                let _ = writeln!(
                    s,
                    "q {} 0 0 {} {} {} cm /Im{idx} Do Q",
                    num(*w),
                    num(*h),
                    num(*x),
                    num(PAGE_H - y - h)
                );
            }
            Op::Text {
                x,
                baseline,
                glyphs,
            } => text_op(&mut s, *x, *baseline, glyphs, remaps),
        }
    }
    s
}

fn text_op(s: &mut String, x: f32, baseline: f32, glyphs: &[Glyph], remaps: &[Option<GlyphRemapper>]) {
    // Inline code backgrounds go underneath the text.
    let mut i = 0;
    while i < glyphs.len() {
        if !glyphs[i].code {
            i += 1;
            continue;
        }
        let start = i;
        while i < glyphs.len() && glyphs[i].code {
            i += 1;
        }
        let first = glyphs[start];
        let last = glyphs[i - 1];
        let x0 = x + first.x;
        let x1 = x + last.x + last.adv * last.size;
        let top = baseline - 0.85 * first.size;
        let bottom = baseline + 0.25 * first.size;
        let _ = writeln!(
            s,
            "{} rg {} {} {} {} re f",
            rgb_op(INLINE_BG),
            num(x0 - 1.5),
            num(PAGE_H - bottom),
            num(x1 - x0 + 3.0),
            num(bottom - top)
        );
    }

    // Text: one BT block per run of glyphs sharing font, style, size, colour and rise.
    let mut i = 0;
    while i < glyphs.len() {
        let start = i;
        i += 1;
        while i < glyphs.len() && same_run(&glyphs[start], &glyphs[i]) {
            i += 1;
        }
        text_segment(s, x, baseline, &glyphs[start..i], remaps);
    }

    // Underline (links) and strikethrough.
    for (flag, offset, thickness_factor) in [(true, 0.11f32, 0.06f32), (false, -0.3, 0.06)] {
        let mut i = 0;
        while i < glyphs.len() {
            let on = |g: &Glyph| if flag { g.underline } else { g.strike };
            if !on(&glyphs[i]) {
                i += 1;
                continue;
            }
            let start = i;
            while i < glyphs.len() && on(&glyphs[i]) {
                i += 1;
            }
            let first = glyphs[start];
            let last = glyphs[i - 1];
            let x0 = x + first.x;
            let x1 = x + last.x + last.adv * last.size;
            let y = baseline + offset * first.size;
            let lw = (first.size * thickness_factor).max(0.5);
            let _ = writeln!(
                s,
                "{} RG {} w {} {} m {} {} l S",
                rgb_op(first.color),
                num(lw),
                num(x0),
                num(PAGE_H - y),
                num(x1),
                num(PAGE_H - y)
            );
        }
    }
}

fn same_run(a: &Glyph, b: &Glyph) -> bool {
    a.file == b.file
        && a.sb == b.sb
        && a.si == b.si
        && a.size == b.size
        && a.color == b.color
        && a.yoff == b.yoff
}

fn text_segment(s: &mut String, x: f32, baseline: f32, seg: &[Glyph], remaps: &[Option<GlyphRemapper>]) {
    let first = seg[0];
    let rise = first.yoff * first.size;
    let anchor_x = x + first.x;
    let anchor_y = baseline - rise;
    let skew = if first.si { FAKE_ITALIC } else { 0.0 };

    let _ = writeln!(s, "{} rg", rgb_op(first.color));
    s.push_str("BT\n");
    let _ = writeln!(s, "/F{} {} Tf", first.file, num(first.size));
    if first.sb {
        let _ = writeln!(s, "2 Tr {} w", num(first.size * 0.03));
    }
    let _ = writeln!(
        s,
        "1 0 {} 1 {} {} Tm",
        num(skew),
        num(anchor_x),
        num(PAGE_H - anchor_y)
    );

    // Glyph positions: each glyph is placed at its shaped pen position plus
    // its mark offset. Adjustments go in the TJ array in thousandths of an em,
    // measured against the PDF's own advance widths.
    let mut parts: Vec<String> = Vec::with_capacity(seg.len() * 2);
    let mut prev: Option<&Glyph> = None;
    for g in seg {
        let adjust = match prev {
            None => -g.off * 1000.0,
            Some(p) => -((p.adv - p.w) + (g.off - p.off)) * 1000.0,
        };
        if adjust.abs() >= 0.05 {
            parts.push(format!("{:.2}", adjust));
        }
        parts.push(format!("<{:04X}>", gid_of(g, remaps)));
        prev = Some(g);
    }
    let _ = writeln!(s, "[{}] TJ", parts.join(" "));
    if first.sb {
        s.push_str("0 Tr\n");
    }
    s.push_str("ET\n");
}

fn to_unicode(entries: &[(u16, char)]) -> String {
    let mut s = String::from(
        "/CIDInit /ProcSet findresource begin\n\
         12 dict begin\n\
         begincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def\n\
         /CMapType 2 def\n\
         1 begincodespacerange\n\
         <0000> <FFFF>\n\
         endcodespacerange\n",
    );
    for chunk in entries.chunks(100) {
        let _ = writeln!(s, "{} beginbfchar", chunk.len());
        for (gid, ch) in chunk {
            let mut units = [0u16; 2];
            let encoded = ch.encode_utf16(&mut units);
            let hex: String = encoded.iter().map(|u| format!("{u:04X}")).collect();
            let _ = writeln!(s, "<{gid:04X}> <{hex}>");
        }
        s.push_str("endbfchar\n");
    }
    s.push_str(
        "endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n",
    );
    s
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("zy-export-pdf-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn fonts_available() -> bool {
        load_fonts().is_ok()
    }

    fn assert_pdf(bytes: &[u8]) {
        assert!(bytes.starts_with(b"%PDF"), "missing %PDF header");
        assert!(
            bytes.trim_ascii_end().ends_with(b"%%EOF"),
            "missing %%EOF trailer"
        );
    }

    fn render_ok(content: &str, base: &Path) -> (Vec<u8>, usize) {
        build_guarded(content, base).expect("export should succeed")
    }

    #[test]
    fn empty_document_exports_one_page() {
        if !fonts_available() {
            return;
        }
        let (bytes, pages) = render_ok("", Path::new("/tmp/empty.md"));
        assert_pdf(&bytes);
        assert_eq!(pages, 1);
    }

    #[test]
    fn long_document_paginates() {
        if !fonts_available() {
            return;
        }
        let mut doc = String::new();
        for i in 0..200 {
            doc.push_str(&format!("## Section {i}\n\nParagraph {i} with **bold** and *italic* and `code` text that wraps across a line.\n\n"));
        }
        let (bytes, pages) = render_ok(&doc, Path::new("/tmp/long.md"));
        assert_pdf(&bytes);
        assert!(pages > 3, "expected several pages, got {pages}");
    }

    #[test]
    fn long_code_block_splits_across_pages() {
        if !fonts_available() {
            return;
        }
        let code: String = (0..200).map(|i| format!("let x{i} = {i};\n")).collect();
        let doc = format!("```rust\n{code}```\n");
        let (bytes, pages) = render_ok(&doc, Path::new("/tmp/code.md"));
        assert_pdf(&bytes);
        assert!(pages > 1, "expected the code block to span pages");
    }

    #[test]
    fn unicode_document_embeds_a_font() {
        if !fonts_available() {
            return;
        }
        let doc = "# สวัสดีครับ\n\nภาษาไทย: ทดสอบการแสดงผลของสระและวรรณยุกต์ เช่น น้ำ กิ่ง ปู่\n\n日本語のテキスト and emoji-free ascii.\n";
        let (bytes, pages) = render_ok(doc, Path::new("/tmp/unicode.md"));
        assert_pdf(&bytes);
        assert_eq!(pages, 1);
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("/CIDFontType2") || text.contains("/CIDFontType0"));
        assert!(text.contains("/Identity-H"));
        assert!(text.contains("/ToUnicode"));
    }

    #[test]
    fn table_and_image_document_exports() {
        if !fonts_available() {
            return;
        }
        let dir = tmp_dir("table-image");
        let png = dir.join("pixel.png");
        let mut img = image::RgbImage::new(64, 32);
        for (x, _y, px) in img.enumerate_pixels_mut() {
            *px = image::Rgb([(x * 4) as u8, 80, 200]);
        }
        img.save(&png).expect("write test png");

        let doc = "\
| Name | Qty | Note |
| :--- | ---: | :---: |
| Tea | 2 | a longer note that has to wrap inside its cell because it is long |
| Jam | 10 | ok |

![pixel](pixel.png)

![missing](nope.png)

> [!WARNING]
> Careful with this one.

- [x] done
- [ ] todo
  1. nested
";
        let (bytes, pages) = render_ok(doc, &dir.join("doc.md"));
        assert_pdf(&bytes);
        assert_eq!(pages, 1);
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("/Subtype /Image"), "image should be embedded");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_writes_the_file_atomically() {
        if !fonts_available() {
            return;
        }
        let dir = tmp_dir("atomic");
        let out = dir.join("out.pdf");
        export("# Hello\n\nworld", &dir.join("in.md"), &out).expect("export");
        let bytes = std::fs::read(&out).expect("output exists");
        assert_pdf(&bytes);
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .expect("read dir")
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temporary file left behind");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn odd_input_never_fails_or_panics() {
        if !fonts_available() {
            return;
        }
        let long_word = "x".repeat(3000);
        let inputs = [
            "|".to_string(),
            "```".to_string(),
            "> > > > >".to_string(),
            "- - - - - a".to_string(),
            "![x](nonexistent.png)".to_string(),
            "\u{0}\u{1}\u{7f}".to_string(),
            "#".repeat(50),
            long_word,
            "[link](http://example.com) <b>html</b>".to_string(),
            "a\n\n\n\n\nb\\\nc".to_string(),
        ];
        for input in &inputs {
            let result = build_guarded(input, Path::new("/nonexistent/dir/doc.md"));
            assert!(result.is_ok(), "export failed for {input:?}: {result:?}");
            assert_pdf(&result.unwrap().0);
        }
    }

    #[test]
    fn export_reports_an_unwritable_destination() {
        if !fonts_available() {
            return;
        }
        let out = Path::new("/nonexistent-zyplus-dir/out.pdf");
        assert!(export("text", Path::new("doc.md"), out).is_err());
    }

    #[test]
    fn suggested_name_swaps_the_extension() {
        assert_eq!(suggested_file_name("Untitled.md"), "Untitled.pdf");
        assert_eq!(suggested_file_name("notes.v2.markdown"), "notes.v2.pdf");
        assert_eq!(suggested_file_name("Untitled 2"), "Untitled 2.pdf");
    }

    #[test]
    fn only_markdown_and_text_are_exportable() {
        assert!(supports("/notes/a.md"));
        assert!(supports("/notes/B.TXT"));
        assert!(!supports("/data/sheet.csv"));
        assert!(!supports("/data/config.json"));
    }

    /// Writes a sample covering every block type for visual inspection:
    /// `ZY_PDF_SAMPLE=/path/out.pdf cargo test write_sample -- --ignored`.
    /// With `ZY_PDF_MD=/path/doc.md` the given markdown is rendered instead.
    #[test]
    #[ignore]
    fn write_sample() {
        let Ok(out) = std::env::var("ZY_PDF_SAMPLE") else {
            return;
        };
        if let Ok(md) = std::env::var("ZY_PDF_MD") {
            let md = PathBuf::from(md);
            let content = std::fs::read_to_string(&md).expect("read markdown");
            let (bytes, pages) = render_ok(&content, &md);
            write_file(Path::new(&out), &bytes).expect("write sample");
            eprintln!("wrote {out} ({} bytes, {pages} pages)", bytes.len());
            return;
        }
        let dir = tmp_dir("sample");
        let png = dir.join("chart.png");
        let mut img = image::RgbImage::new(240, 120);
        for (x, y, px) in img.enumerate_pixels_mut() {
            *px = image::Rgb([(x % 256) as u8, (y * 2 % 256) as u8, 160]);
        }
        img.save(&png).expect("write png");
        let doc = "\
# Zyplus export sample

Body text at 11pt with **bold**, *italic*, ~~strike~~, `inline code` and a [link](https://example.com). This paragraph is long enough that it has to wrap onto a second line so the justification and breaking can be checked by eye.

## Thai and mixed scripts

ภาษาไทยสำหรับทดสอบการแสดงผล น้ำ กิ่ง ปู่ เก้าอี้ ไข่เจียว — English mixed in, 日本語も.

### Lists

- First bullet
- Second bullet with a nested list:
  1. Nested one
  2. Nested two
- [x] Shipped task
- [ ] Open task

### Quotes and callouts

> A plain quote with some words to wrap across lines in the quote column.

> [!NOTE]
> Callouts carry a label and a coloured bar.

### Code

```rust
fn main() {
    println!(\"hello, world\");
}
```

### Table

| Item | Qty | Price |
| :--- | ---: | :---: |
| Green tea | 2 | 120 |
| Jasmine rice, long name here | 10 | 35 |

---

![chart](chart.png)
";
        let (bytes, pages) = render_ok(doc, &dir.join("sample.md"));
        write_file(Path::new(&out), &bytes).expect("write sample");
        eprintln!("wrote {out} ({} bytes, {pages} pages)", bytes.len());
    }
}
