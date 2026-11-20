//! Native display math for ```latex, ```math, ```katex and ```tex fences.
//!
//! The web build hands the block to KaTeX in a webview. Here the same job is
//! done by RaTeX (`ratex-*` crates, MIT): a pure-Rust, KaTeX-compatible parser
//! and layout engine. The formula becomes a standalone SVG whose glyphs are
//! outlines from the KaTeX OFL fonts embedded in the binary, so the SVG needs
//! no installed fonts, no JavaScript and no network.
//!
//! Pipeline: LaTeX -> `ratex_parser::parse` -> `ratex_layout::layout` (display
//! style) -> `to_display_list` -> `ratex_svg` (`<path>` glyphs). The SVG's
//! `width`/`height` are in points, so `svg_size` reports CSS px (pt * 4/3),
//! which is the size gpui's `img()` rasteriser uses for SVG files.

#![allow(dead_code)] // Wired up by the rich-editor / preview integration.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Mutex;

use ratex_layout::{layout, to_display_list, validate_layout, LayoutOptions};
use ratex_parser::parser::parse;
use ratex_parser::ParseError;
use ratex_svg::{render_to_svg_with_color_syntax, SvgColorSyntax, SvgOptions};
use ratex_types::color::Color;
use ratex_types::math_style::MathStyle;

use crate::theme::Theme;

/// Largest accepted formula source, in bytes. Bigger inputs are rejected
/// before parsing.
pub const MAX_SOURCE_BYTES: usize = 16 * 1024;

/// One em in SVG user units. The SVG is in pt, so 12 pt is 16 CSS px, which
/// matches the body text size of the preview.
const EM_PT: f64 = 12.0;
/// Margin around the formula, in pt. It keeps italic overhangs inside the box.
const PAD_PT: f64 = 2.0;
/// Fraction rules and other stroked paths, in pt (about 0.04 em).
const STROKE_PT: f64 = 0.45;

/// Ink for `dark = false` (light backgrounds) and `dark = true`.
const LIGHT_INK: (f32, f32, f32) = (0.10, 0.10, 0.10);
const DARK_INK: (f32, f32, f32) = (0.90, 0.90, 0.90);

/// Maximum number of cached SVGs. Oldest entries are evicted first.
const CACHE_CAP: usize = 256;

type CacheKey = (u64, [u8; 4]);

struct Cache {
    map: HashMap<CacheKey, String>,
    order: VecDeque<CacheKey>,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

/// Render display-style math to a standalone SVG string. The ink colour is
/// chosen from `dark`. The background is transparent and the glyphs are paths.
pub fn render(latex: &str, dark: bool) -> Result<String, String> {
    let (r, g, b) = if dark { DARK_INK } else { LIGHT_INK };
    render_with_ink(latex, Color::new(r, g, b, 1.0))
}

/// Render using the theme's foreground colour as ink, so the formula matches
/// body text in every palette.
pub fn render_for_theme(latex: &str, theme: &Theme) -> Result<String, String> {
    let fg = theme.fg;
    render_with_ink(latex, Color::new(fg.r, fg.g, fg.b, fg.a))
}

fn render_with_ink(latex: &str, ink: Color) -> Result<String, String> {
    let source = latex.trim();
    if source.is_empty() {
        return Err("Empty math block".to_string());
    }
    if latex.len() > MAX_SOURCE_BYTES {
        return Err(format!(
            "Math block is {} bytes; the limit is {} bytes",
            latex.len(),
            MAX_SOURCE_BYTES
        ));
    }

    let key = cache_key(source, &ink);
    if let Some(hit) = cache_get(&key) {
        return Ok(hit);
    }

    // The parser and layout engine are third-party code. A panic here must
    // become an error the editor can show, not a crash.
    let svg = std::panic::catch_unwind(|| render_uncached(source, ink))
        .map_err(|_| "The math renderer failed on this formula".to_string())??;

    cache_put(key, svg.clone());
    Ok(svg)
}

fn render_uncached(source: &str, ink: Color) -> Result<String, String> {
    let ast = parse(source).map_err(|e| explain_parse_error(&e))?;
    let options = LayoutOptions::default()
        .with_style(MathStyle::Display)
        .with_color(ink);
    let root = layout(&ast, &options);
    validate_layout(&root).map_err(|e| format!("Math is too large to draw: {e}"))?;
    let list = to_display_list(&root);
    let svg_options = SvgOptions {
        font_size: EM_PT,
        padding: PAD_PT,
        stroke_width: STROKE_PT,
        embed_glyphs: true,
        font_dir: String::new(),
    };
    Ok(render_to_svg_with_color_syntax(
        &list,
        &svg_options,
        SvgColorSyntax::Rgb,
    ))
}

/// Turn a parser error into one line that names the problem. Unknown commands
/// are listed by name, so the author can see what to remove or rewrite.
fn explain_parse_error(error: &ParseError) -> String {
    const UNDEFINED: &str = "Undefined control sequence:";
    let message = error.message.trim();
    if let Some(index) = message.find(UNDEFINED) {
        let command = message[index + UNDEFINED.len()..].trim();
        return format!("Unsupported LaTeX command {command} in math block");
    }
    format!("LaTeX error: {message}")
}

/// Size in CSS px of a standalone SVG produced by [`render`]. Reads the root
/// `width`/`height` attributes and converts pt to px. Returns `None` when they
/// are missing or not positive.
pub fn svg_size(svg: &str) -> Option<(f32, f32)> {
    let root_end = svg.find('>')?;
    let root = &svg[..root_end];
    let width = to_px(attribute(root, "width")?)?;
    let height = to_px(attribute(root, "height")?)?;
    Some((width, height))
}

fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let pattern = format!(" {name}=\"");
    let start = tag.find(&pattern)? + pattern.len();
    let end = tag[start..].find('"')? + start;
    Some(&tag[start..end])
}

fn to_px(value: &str) -> Option<f32> {
    let value = value.trim();
    let (number, scale) = if let Some(n) = value.strip_suffix("pt") {
        (n, 4.0_f32 / 3.0)
    } else if let Some(n) = value.strip_suffix("px") {
        (n, 1.0)
    } else {
        (value, 1.0)
    };
    let number: f32 = number.trim().parse().ok()?;
    if number.is_finite() && number > 0.0 {
        Some(number * scale)
    } else {
        None
    }
}

/// Write `svg` to `<temp>/zyplus-diagrams/<hash>.svg` and return the path, for
/// gpui's `img(path)`. The name is a hash of the content, so the same SVG
/// always maps to the same file, and an existing identical file is reused.
pub fn to_image_path(svg: &str) -> Result<PathBuf, String> {
    if svg.is_empty() {
        return Err("Cannot write an empty SVG".to_string());
    }
    let dir = std::env::temp_dir().join("zyplus-diagrams");
    let path = dir.join(format!("{:016x}.svg", hash_of(svg)));

    if let Ok(existing) = fs::read(&path) {
        if existing == svg.as_bytes() {
            return Ok(path);
        }
    }

    fs::create_dir_all(&dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    // Write to a per-process temp name, then rename, so readers never see a
    // half-written file.
    let staging = path.with_extension(format!("svg.{}.tmp", std::process::id()));
    fs::write(&staging, svg.as_bytes())
        .map_err(|e| format!("Cannot write {}: {e}", staging.display()))?;
    fs::rename(&staging, &path).map_err(|e| {
        let _ = fs::remove_file(&staging);
        format!("Cannot move SVG into place at {}: {e}", path.display())
    })?;
    Ok(path)
}

fn hash_of<T: Hash + ?Sized>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn cache_key(source: &str, ink: &Color) -> CacheKey {
    let quantise = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    (
        hash_of(source),
        [
            quantise(ink.r),
            quantise(ink.g),
            quantise(ink.b),
            quantise(ink.a),
        ],
    )
}

fn cache_get(key: &CacheKey) -> Option<String> {
    let guard = CACHE.lock().ok()?;
    guard.as_ref()?.map.get(key).cloned()
}

fn cache_put(key: CacheKey, svg: String) {
    let Ok(mut guard) = CACHE.lock() else {
        return;
    };
    let cache = guard.get_or_insert_with(|| Cache {
        map: HashMap::new(),
        order: VecDeque::new(),
    });
    if cache.map.insert(key, svg).is_none() {
        cache.order.push_back(key);
    }
    while cache.order.len() > CACHE_CAP {
        if let Some(old) = cache.order.pop_front() {
            cache.map.remove(&old);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Representative inputs: the forms people write in notes, from the
    /// quadratic formula to matrices, Greek letters and \text with Thai.
    const CORPUS: &[&str] = &[
        r"x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}",
        r"\frac{1}{2} + \frac{a+b}{c-d}",
        r"\sum_{i=1}^{n} i = \frac{n(n+1)}{2}",
        r"\int_0^\infty e^{-x^2}\,dx = \frac{\sqrt{\pi}}{2}",
        r"\sqrt[3]{x^2 + 1}",
        r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}",
        r"\begin{aligned} a &= b + c \\ d &= e \end{aligned}",
        r"\mathbb{R}^n \subseteq \mathbb{C}",
        r"\frac{\partial f}{\partial x} + \frac{\partial^2 f}{\partial y^2}",
        r"\alpha \beta \gamma \Gamma \Delta \lambda \omega \varepsilon",
        r"\text{if } x > 0 \text{ then } y = 1",
        r"\text{ภาษาไทย: สวัสดี} + x",
        r"\left( \frac{a}{b} \right)^{2}",
        r"\left\{ \begin{array}{ll} 1 & x>0 \\ 0 & \text{otherwise} \end{array} \right.",
        r"\lim_{x \to 0} \frac{\sin x}{x} = 1",
        r"e^{i\pi} + 1 = 0",
        r"\hat{x} \bar{y} \vec{v} \dot{z} \tilde{w}",
        r"\binom{n}{k} = \frac{n!}{k!(n-k)!}",
        r"\prod_{k=1}^{\infty} \left(1 - \frac{1}{2^k}\right)",
        r"\nabla \times \vec{B} = \mu_0 \vec{J} + \mu_0 \epsilon_0 \frac{\partial \vec{E}}{\partial t}",
    ];

    /// Rasterise an SVG with resvg on a white (light) or dark background and
    /// return the pixmap, so tests can check for ink.
    fn rasterise(
        svg: &str,
        scale: f32,
        background: resvg::tiny_skia::Color,
    ) -> resvg::tiny_skia::Pixmap {
        let options = resvg::usvg::Options::default();
        let tree = resvg::usvg::Tree::from_str(svg, &options).expect("resvg parses our SVG");
        let size = tree.size();
        let width = (size.width() * scale).ceil() as u32;
        let height = (size.height() * scale).ceil() as u32;
        let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height).expect("pixmap");
        pixmap.fill(background);
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_scale(scale, scale),
            &mut pixmap.as_mut(),
        );
        pixmap
    }

    /// Count pixels that differ noticeably from the background colour.
    fn inked_pixels(pixmap: &resvg::tiny_skia::Pixmap, background: [u8; 3]) -> usize {
        pixmap
            .pixels()
            .iter()
            .filter(|p| {
                let (r, g, b) = (p.red() as i32, p.green() as i32, p.blue() as i32);
                let d = (r - background[0] as i32).abs()
                    + (g - background[1] as i32).abs()
                    + (b - background[2] as i32).abs();
                d > 60
            })
            .count()
    }

    /// Whether RaTeX's system fallback font covers Thai glyphs (outlining them as
    /// `<path>` instead of falling back to `<text>`). True when a Thai-capable font
    /// is found (e.g. Arial Unicode on macOS); false on minimal Linux/Windows hosts
    /// whose RaTeX fallback font lacks Thai (at runtime GPUI's `SvgRenderer` loads
    /// the full system fontdb and renders `<text>` fallback glyphs).
    fn ratex_outlines_thai() -> bool {
        render(r"\text{ก}", false)
            .map(|svg| svg.contains("<path") && !svg.contains("<text"))
            .unwrap_or(false)
    }

    #[test]
    fn corpus_renders_and_draws() {
        let thai_outlined = ratex_outlines_thai();
        for src in CORPUS {
            let svg = render(src, false).unwrap_or_else(|e| panic!("{src}: {e}"));
            assert!(svg.starts_with("<svg"), "{src}: not an svg root");
            let has_thai = src.chars().any(|c| ('\u{0E00}'..='\u{0E7F}').contains(&c));
            if !has_thai || thai_outlined {
                assert!(
                    !svg.contains("<text"),
                    "{src}: glyphs must be paths, not text"
                );
            } else {
                // Without a Thai fallback font in RaTeX, only Thai glyphs may fall back
                // to `<text>`. The math glyphs in the same formula must still be paths.
                assert!(
                    !svg.contains(">x</text>") && !svg.contains(">+</text>"),
                    "{src}: math glyphs must still be paths"
                );
            }
            assert!(svg.contains("<path"), "{src}: no path glyphs emitted");

            let (w, h) = svg_size(&svg).unwrap_or_else(|| panic!("{src}: no size"));
            assert!(w > 0.0 && h > 0.0, "{src}: size {w}x{h}");
            let ratio = w / h;
            assert!(
                (0.05..=200.0).contains(&ratio),
                "{src}: insane aspect ratio {ratio} ({w}x{h})"
            );

            let pixmap = rasterise(&svg, 2.0, resvg::tiny_skia::Color::WHITE);
            let inked = inked_pixels(&pixmap, [255, 255, 255]);
            assert!(inked > 30, "{src}: rasterised blank ({inked} inked px)");
        }
    }

    #[test]
    fn fraction_is_taller_than_a_single_line() {
        let (_, h_frac) = svg_size(&render(r"\frac{a}{b}", false).unwrap()).unwrap();
        let (_, h_text) = svg_size(&render("a", false).unwrap()).unwrap();
        assert!(
            h_frac > h_text * 1.5,
            "fraction {h_frac} vs letter {h_text}"
        );
    }

    #[test]
    fn display_sum_has_limits_above_and_below() {
        // With \displaystyle limits, the sum is taller than an inline-style one.
        let display = svg_size(&render(r"\sum_{i=1}^{n} x_i", false).unwrap()).unwrap();
        let plain = svg_size(&render(r"x_i", false).unwrap()).unwrap();
        assert!(display.1 > plain.1 * 2.0, "sum {display:?} vs {plain:?}");
    }

    #[test]
    fn unsupported_command_is_named_in_the_error() {
        let err = render(r"\usepackage{amsmath} x", false).unwrap_err();
        assert!(
            err.contains(r"\usepackage"),
            "error should name the command: {err}"
        );
    }

    #[test]
    fn malformed_input_errors_without_panicking() {
        for src in [
            r"\frac{",
            "{{{",
            r"\left( x",
            r"\begin{pmatrix} a & b",
            r"\end{pmatrix}",
            r"\sqrt[",
            "$x$ and more",
            r"\text{unterminated",
            "\u{0}\u{1}\u{2}",
        ] {
            // Either a clean error or an SVG is acceptable. A panic is not.
            let _ = render(src, false);
        }
        assert!(render(r"\frac{", false).is_err());
    }

    #[test]
    fn thai_text_renders_without_panicking() {
        // Thai has no glyphs in KaTeX. RaTeX falls back to a system font, so
        // this must succeed with a usable size, even if a glyph is missing.
        let svg = render(r"\text{สวัสดีครับ} + x", false).expect("Thai \\text is not an error");
        let (w, h) = svg_size(&svg).expect("size");
        assert!(w > 0.0 && h > 0.0);
    }

    #[test]
    fn empty_and_oversized_inputs_are_rejected() {
        assert!(render("   \n ", false).is_err());
        let big = "x".repeat(MAX_SOURCE_BYTES + 1);
        let err = render(&big, false).unwrap_err();
        assert!(err.contains("limit"), "{err}");
        let at_limit = "x".repeat(MAX_SOURCE_BYTES);
        assert!(render(&at_limit, false).is_ok());
    }

    #[test]
    fn dark_and_light_use_different_ink() {
        let light = render(r"x^2", false).unwrap();
        let dark = render(r"x^2", true).unwrap();
        assert_ne!(light, dark);
        assert!(light.contains("rgb(26,26,26)"), "light ink missing");
        assert!(dark.contains("rgb(230,230,230)"), "dark ink missing");
    }

    #[test]
    fn theme_foreground_becomes_the_ink() {
        use gpui::WindowAppearance;
        let light = Theme::for_name("light", WindowAppearance::Light);
        let mocha = Theme::for_name("mocha", WindowAppearance::Light);
        let a = render_for_theme(r"\frac{a}{b}", &light).unwrap();
        let b = render_for_theme(r"\frac{a}{b}", &mocha).unwrap();
        assert_ne!(a, b, "different theme foregrounds must give different ink");
        assert!(svg_size(&a).is_some());
    }

    #[test]
    fn repeated_render_is_cached_and_identical() {
        let a = render(r"\frac{\pi}{4}", true).unwrap();
        let b = render(r"\frac{\pi}{4}", true).unwrap();
        assert_eq!(a, b);
        assert!(cache_get(&cache_key(
            r"\frac{\pi}{4}",
            &Color::new(DARK_INK.0, DARK_INK.1, DARK_INK.2, 1.0)
        ))
        .is_some());
    }

    #[test]
    fn cache_stays_bounded() {
        for i in 0..(CACHE_CAP + 20) {
            let _ = render(&format!("x_{{{i}}}"), false);
        }
        let guard = CACHE.lock().unwrap();
        let cache = guard.as_ref().unwrap();
        assert!(cache.map.len() <= CACHE_CAP);
        assert_eq!(cache.map.len(), cache.order.len());
    }

    #[test]
    fn svg_size_reads_pt_as_px() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 30 15" width="30pt" height="15pt"><path d=""/></svg>"#;
        let (w, h) = svg_size(svg).unwrap();
        assert!(
            (w - 40.0).abs() < 1e-3 && (h - 20.0).abs() < 1e-3,
            "{w} {h}"
        );
        assert_eq!(svg_size("<svg></svg>"), None);
        assert_eq!(svg_size(r#"<svg width="0pt" height="5pt">"#), None);
    }

    #[test]
    fn image_path_is_idempotent_and_holds_the_svg() {
        let svg = render(r"\sqrt{2}", false).unwrap();
        let first = to_image_path(&svg).unwrap();
        let second = to_image_path(&svg).unwrap();
        assert_eq!(first, second);
        assert!(first.starts_with(std::env::temp_dir().join("zyplus-diagrams")));
        assert_eq!(first.extension().and_then(|e| e.to_str()), Some("svg"));
        assert_eq!(fs::read_to_string(&first).unwrap(), svg);
        assert!(to_image_path("").is_err());
    }

    /// Writes PNG previews for a visual check, but only when
    /// `ZY_MATH_PREVIEW_DIR` is set. Otherwise it is a no-op.
    #[test]
    fn write_preview_pngs_when_requested() {
        let Some(dir) = std::env::var_os("ZY_MATH_PREVIEW_DIR") else {
            return;
        };
        let dir = PathBuf::from(dir);
        fs::create_dir_all(&dir).unwrap();
        let light_bg = [255u8, 255, 255];
        let dark_bg = [30u8, 30, 30];
        let mut panels: Vec<image::RgbaImage> = Vec::new();
        for (i, src) in CORPUS.iter().enumerate() {
            let dark = i % 2 == 1;
            let svg = render(src, dark).unwrap();
            let bg = if dark { dark_bg } else { light_bg };
            let pixmap = rasterise(
                &svg,
                2.0,
                resvg::tiny_skia::Color::from_rgba8(bg[0], bg[1], bg[2], 255),
            );
            let (w, h) = (pixmap.width(), pixmap.height());
            // tiny-skia stores premultiplied RGBA. The background is opaque, so the values are final.
            let mut rgba = Vec::with_capacity((w * h * 4) as usize);
            for p in pixmap.pixels() {
                rgba.extend_from_slice(&[p.red(), p.green(), p.blue(), 255]);
            }
            let img = image::RgbaImage::from_raw(w, h, rgba).unwrap();
            img.save(dir.join(format!("{i:02}.png"))).unwrap();
            panels.push(img);
        }

        // One contact sheet, two columns, each panel on its own background
        // so light and dark samples are both visible.
        let cell_w = panels.iter().map(|p| p.width()).max().unwrap_or(1) + 24;
        let cell_h = panels.iter().map(|p| p.height()).max().unwrap_or(1) + 24;
        let cols = 2u32;
        let rows = (panels.len() as u32).div_ceil(cols);
        let mut sheet = image::RgbaImage::from_pixel(
            cols * cell_w,
            rows * cell_h,
            image::Rgba([128, 128, 128, 255]),
        );
        for (k, panel) in panels.iter().enumerate() {
            let (cx, cy) = (
                (k as u32 % cols) * cell_w + 12,
                (k as u32 / cols) * cell_h + 12,
            );
            image::imageops::replace(&mut sheet, panel, cx as i64, cy as i64);
        }
        sheet.save(dir.join("sheet.png")).unwrap();
    }
}
