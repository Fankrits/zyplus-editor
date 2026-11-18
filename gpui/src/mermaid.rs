//! Native Mermaid rendering for ```mermaid fences.
//!
//! The web app draws fences with mermaid.js in a webview. The native app has no
//! JS engine, so the source is rendered here with `mermaid-svg` (pure Rust, MIT,
//! one dependency, no browser, no network) into a standalone SVG string themed to
//! the diagram palette the web build uses (`--diagram-*` in `App.css`).
//!
//! gpui's `svg()` element is a monochrome alpha mask, so colour diagrams go through
//! `img(path)` instead: [`to_image_path`] writes the SVG to a private temp file and
//! gpui rasterises it with resvg (system fonts come from gpui's fontdb).
//!
//! Nothing here panics on bad input. Parse errors, oversized sources and renders
//! that take too long come back as `Err(String)` with a message for the user.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use mermaid_svg::Theme as SvgTheme;

use crate::theme::Theme;

/// Largest accepted diagram source, in bytes.
pub const MAX_SOURCE_BYTES: usize = 64 * 1024;
/// Largest accepted node count (see [`count_nodes`] for how it is estimated).
pub const MAX_NODES: usize = 500;
/// Wall-clock budget for one render. The render runs on a helper thread; if it
/// overruns, the caller gets an error and the thread finishes in the background.
const RENDER_TIMEOUT: Duration = Duration::from_secs(5);
/// Entries kept in the render cache before it is cleared.
const CACHE_CAPACITY: usize = 128;

/// Font stack for diagram labels. All of these resolve on macOS, so usvg's
/// fontdb (inside gpui) picks a real face instead of falling back to a box.
const FONT_FAMILY: &str = "Helvetica Neue, Helvetica, Arial, sans-serif";

/// Render `source` to a standalone SVG string.
///
/// `dark` selects the dark or light diagram palette. `bg` is the background
/// colour as `#rgb`, `#rrggbb` or `#rrggbbaa`; `None` uses the palette's own
/// diagram background. Results are cached by (source hash, palette key).
pub fn render(source: &str, dark: bool, bg: Option<&str>) -> Result<String, String> {
    if let Some(bg) = bg {
        if !is_hex_colour(bg) {
            return Err(format!("invalid diagram background colour: {bg}"));
        }
    }
    let key = (hash64(source.as_bytes()), palette_key(dark, bg));
    if let Some(hit) = cache().lock().ok().and_then(|c| c.get(&key).cloned()) {
        return hit;
    }
    let result = render_uncached(source, dark, bg);
    if let Ok(mut c) = cache().lock() {
        if c.len() >= CACHE_CAPACITY {
            c.clear();
        }
        c.insert(key, result.clone());
    }
    result
}

/// [`render`] with the palette taken from an app [`Theme`]: dark when the window
/// background is dark, and the diagram background set to the app background so the
/// diagram sits on the editor canvas.
pub fn render_for_theme(source: &str, theme: &Theme) -> Result<String, String> {
    let bg = to_hex(theme.bg);
    render(source, is_dark(theme), Some(&bg))
}

/// Parse an SVG produced by [`render`] for its intrinsic size in px.
///
/// Reads `width`/`height` on the root `<svg>`, falling back to the `viewBox`.
/// Percentage or missing sizes give `None`.
pub fn svg_size(svg: &str) -> Option<(f32, f32)> {
    let start = svg.find("<svg")?;
    let tag_end = start + svg[start..].find('>')?;
    let tag = &svg[start..tag_end];

    let w = attr(tag, "width").and_then(parse_px);
    let h = attr(tag, "height").and_then(parse_px);
    if let (Some(w), Some(h)) = (w, h) {
        return positive(w, h);
    }
    let vb = attr(tag, "viewBox")?;
    let nums: Vec<f32> = vb
        .split(|c: char| c == ' ' || c == ',')
        .filter(|s| !s.is_empty())
        .map(str::parse::<f32>)
        .collect::<Result<_, _>>()
        .ok()?;
    if nums.len() != 4 {
        return None;
    }
    positive(nums[2], nums[3])
}

/// Write `svg` to `<temp>/zyplus-diagrams/<hash>.svg` and return the path, for
/// `gpui::img(path)`.
///
/// The name is a hash of the content, so the same SVG always maps to the same
/// file and repeated calls rewrite nothing. The directory is created 0700 so
/// other users cannot read or replace diagrams. Writes go through a temp file and
/// a rename, so gpui never reads a half-written SVG.
pub fn to_image_path(svg: &str) -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join("zyplus-diagrams");
    create_private_dir(&dir)?;

    let name = format!("{:016x}.svg", hash64(svg.as_bytes()));
    let path = dir.join(&name);
    if fs::read(&path).map(|old| old == svg.as_bytes()).unwrap_or(false) {
        return Ok(path);
    }

    // Unique per call, so two threads writing the same diagram never share a temp file.
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(".{name}.{}.{seq}.tmp", std::process::id()));
    fs::write(&tmp, svg).map_err(|e| format!("writing diagram: {e}"))?;
    fs::rename(&tmp, &path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("placing diagram: {e}")
    })?;
    Ok(path)
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render_uncached(source: &str, dark: bool, bg: Option<&str>) -> Result<String, String> {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        return Err("the Mermaid block is empty".into());
    }
    if source.len() > MAX_SOURCE_BYTES {
        return Err(format!(
            "the diagram source is larger than {} KB",
            MAX_SOURCE_BYTES / 1024
        ));
    }
    let nodes = count_nodes(trimmed);
    if nodes > MAX_NODES {
        return Err(format!(
            "the diagram has about {nodes} nodes; the limit is {MAX_NODES}"
        ));
    }

    let theme = svg_theme(dark, bg);
    let src = trimmed.to_owned();
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("zyplus-mermaid".into())
        .spawn(move || {
            let out = panic::catch_unwind(AssertUnwindSafe(|| {
                mermaid_svg::render_with(&src, &theme).map_err(|e| e.to_string())
            }))
            .unwrap_or_else(|_| Err("the diagram renderer failed unexpectedly".into()));
            let _ = tx.send(out);
        })
        .map_err(|e| format!("could not start the diagram renderer: {e}"))?;

    match rx.recv_timeout(RENDER_TIMEOUT) {
        Ok(Ok(svg)) => Ok(svg),
        Ok(Err(msg)) => {
            let msg = msg.strip_prefix("parse error: ").unwrap_or(&msg);
            Err(format!("Mermaid syntax error: {msg}"))
        }
        Err(_) => Err(format!(
            "the diagram took longer than {} s to render",
            RENDER_TIMEOUT.as_secs()
        )),
    }
}

/// The `mermaid-svg` theme for a palette. Colours follow the web diagram tokens
/// (`--diagram-*` in `App.css`, and `getMermaidConfig` in `mermaid.ts`).
fn svg_theme(dark: bool, bg: Option<&str>) -> SvgTheme {
    let p = Palette::new(dark);
    let bg: Cow<'static, str> = match bg {
        Some(c) => Cow::Owned(c.to_owned()),
        None => Cow::Borrowed(p.bg),
    };
    let st = |s: &'static str| Cow::Borrowed(s);
    let list = |items: &[&'static str]| -> Cow<'static, [Cow<'static, str>]> {
        Cow::Owned(items.iter().map(|s| Cow::Borrowed(*s)).collect())
    };
    // The categorical scale is the six `--diagram-N` colours, cycled per series.
    let scale = list(&p.scale);

    SvgTheme {
        fg: st(p.text),
        fg_muted: st(p.edge_text),
        bg: bg.clone(),
        actor_fill: bg.clone(),
        actor_stroke: st(p.accent),
        actor_text_color: Some(st(p.accent)),
        lifeline: st(p.border),
        arrow_stroke: st(p.line),
        signal_text_color: Some(st(p.text)),
        note_fill: bg.clone(),
        note_stroke: st(p.scale[3]),
        activation_fill: bg.clone(),
        activation_stroke: st(p.accent),
        frame_label_fill: bg.clone(),
        title_color: Some(st(p.text)),
        flow_node_fill: bg.clone(),
        flow_node_stroke: st(p.accent),
        flow_edge_stroke: st(p.line),
        flow_label_bg: bg.clone(),
        flow_cluster_fill: bg.clone(),
        flow_cluster_stroke: st(p.border),
        cscale_palette: scale.clone(),
        pie_palette: scale.clone(),
        git_palette: scale.clone(),
        xychart_palette: scale,
        pie_stroke: Some(bg.clone()),
        pie_opacity: None,
        commit_label_color: Some(st(p.edge_text)),
        tag_label_color: Some(st(p.text)),
        quadrant_fills: [None, None, None, None],
        quadrant_default_fills: p.quadrant_fills,
        font_family: st(FONT_FAMILY),
        font_size: 14.0,
        // Fixed width/height, not `width="100%"`: gpui's rasteriser needs a
        // concrete size to lay the image out.
        responsive: false,
    }
}

/// Colours for one light or dark palette.
struct Palette {
    bg: &'static str,
    text: &'static str,
    border: &'static str,
    line: &'static str,
    edge_text: &'static str,
    /// `--diagram-1` ... `--diagram-6`.
    scale: [&'static str; 6],
    quadrant_fills: [Cow<'static, str>; 4],
    accent: &'static str,
}

impl Palette {
    fn new(dark: bool) -> Self {
        if dark {
            Self {
                bg: "#18181b",
                text: "#e4e4e7",
                border: "#3f3f46",
                line: "#a1a1aa",
                edge_text: "#d4d4d8",
                scale: ["#63b3ff", "#c084fc", "#34d399", "#fbbf24", "#fb7185", "#22d3ee"],
                quadrant_fills: [
                    Cow::Borrowed("#1e2a3a"),
                    Cow::Borrowed("#2a1f3a"),
                    Cow::Borrowed("#1a2e25"),
                    Cow::Borrowed("#33280f"),
                ],
                accent: "#63b3ff",
            }
        } else {
            Self {
                bg: "#ffffff",
                text: "#0f172a",
                border: "#cbd5e1",
                line: "#64748b",
                edge_text: "#334155",
                scale: ["#006fee", "#7828c8", "#17c964", "#f5a524", "#f31260", "#06b6d4"],
                quadrant_fills: [
                    Cow::Borrowed("#eef5ff"),
                    Cow::Borrowed("#f4efff"),
                    Cow::Borrowed("#eefbf3"),
                    Cow::Borrowed("#fff7e8"),
                ],
                accent: "#006fee",
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Guards and helpers
// ---------------------------------------------------------------------------

/// Estimate the node count from the source.
///
/// Counts distinct identifiers left after removing quoted text, bracketed labels,
/// `|edge labels|`, `%%` comments and everything after a `:` (message, task and
/// pie labels). For flowcharts, class, state and ER diagrams this tracks the real
/// node count closely. It is a guard against huge inputs, not an exact figure.
fn count_nodes(source: &str) -> usize {
    let mut ids: HashSet<String> = HashSet::new();
    for line in source.lines() {
        let line = match line.find("%%") {
            Some(i) => &line[..i],
            None => line,
        };
        let mut cleaned = String::with_capacity(line.len());
        let mut depth = 0usize;
        let mut in_quote = false;
        let mut in_pipe = false;
        for ch in line.chars() {
            if in_quote {
                if ch == '"' {
                    in_quote = false;
                }
                cleaned.push(' ');
                continue;
            }
            if in_pipe {
                if ch == '|' {
                    in_pipe = false;
                }
                cleaned.push(' ');
                continue;
            }
            match ch {
                '"' => {
                    in_quote = true;
                    cleaned.push(' ');
                }
                '[' | '(' | '{' => {
                    depth += 1;
                    cleaned.push(' ');
                }
                ']' | ')' | '}' => {
                    depth = depth.saturating_sub(1);
                    cleaned.push(' ');
                }
                '|' if depth == 0 => {
                    in_pipe = true;
                    cleaned.push(' ');
                }
                ':' if depth == 0 => break,
                _ if depth > 0 => cleaned.push(' '),
                _ => cleaned.push(ch),
            }
        }
        for tok in cleaned.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
            if tok.starts_with(|c: char| c.is_alphabetic() || c == '_') {
                ids.insert(tok.to_owned());
            }
        }
    }
    ids.len()
}

fn is_hex_colour(s: &str) -> bool {
    let Some(hex) = s.strip_prefix('#') else {
        return false;
    };
    matches!(hex.len(), 3 | 4 | 6 | 8) && hex.bytes().all(|b| b.is_ascii_hexdigit())
}

fn palette_key(dark: bool, bg: Option<&str>) -> String {
    format!(
        "{}|{}",
        if dark { "dark" } else { "light" },
        bg.unwrap_or("default")
    )
}

fn cache() -> &'static Mutex<HashMap<(u64, String), Result<String, String>>> {
    static CACHE: OnceLock<Mutex<HashMap<(u64, String), Result<String, String>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// FNV-1a, 64-bit. Stable across runs, unlike `DefaultHasher`, so file names are
/// deterministic.
fn hash64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

fn is_dark(theme: &Theme) -> bool {
    let c = theme.bg;
    0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b < 0.5
}

fn to_hex(c: gpui::Rgba) -> String {
    let ch = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", ch(c.r), ch(c.g), ch(c.b))
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!("{name}=\"");
    let mut from = 0;
    while let Some(i) = tag[from..].find(&needle) {
        let at = from + i;
        // Require a whitespace before the name so `stroke-width` does not match `width`.
        let preceded_by_space = tag[..at].chars().last().is_some_and(char::is_whitespace);
        let value_start = at + needle.len();
        if preceded_by_space {
            let end = tag[value_start..].find('"')?;
            return Some(&tag[value_start..value_start + end]);
        }
        from = value_start;
    }
    None
}

fn parse_px(v: &str) -> Option<f32> {
    let v = v.trim().trim_end_matches("px").trim();
    v.parse::<f32>().ok()
}

fn positive(w: f32, h: f32) -> Option<(f32, f32)> {
    (w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0).then_some((w, h))
}

fn create_private_dir(dir: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        if dir.is_dir() {
            return Ok(());
        }
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(|e| format!("creating {}: {e}", dir.display()))
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLOWCHART: &str = "flowchart TD\n    Start[Start here] --> Decide{Is it ready?}\n    Decide -->|yes| Ship[Ship it]\n    Decide -->|no| Wait[Wait longer]";
    const SEQUENCE: &str = "sequenceDiagram\n    participant Alice\n    participant Bob\n    Alice->>Bob: Hello Bob\n    Bob-->>Alice: Hi Alice";
    const CLASS: &str = "classDiagram\n    class Animal {\n      +String name\n    }\n    class Dog\n    Animal <|-- Dog";
    const STATE: &str = "stateDiagram-v2\n    [*] --> Idle\n    Idle --> Running : start\n    Running --> [*]";
    const ER: &str = "erDiagram\n    CUSTOMER ||--o{ ORDER : places\n    ORDER {\n      int id PK\n    }";
    const GANTT: &str = "gantt\n    title Release plan\n    dateFormat YYYY-MM-DD\n    section Build\n    Design :a1, 2026-01-01, 5d\n    Code   :after a1, 7d";
    const PIE: &str = "pie title Pets\n    \"Dogs\" : 386\n    \"Cats\" : 85";
    const GITGRAPH: &str = "gitGraph\n    commit id: \"init\"\n    branch develop\n    commit id: \"feat\"\n    checkout main\n    merge develop";
    const JOURNEY: &str = "journey\n    title My day\n    section Morning\n      Wake up: 3: Me\n      Coffee: 5: Me";
    const MINDMAP: &str = "mindmap\n  root((Editor))\n    Writing\n    Preview";

    const TIMELINE: &str = "timeline\n    title History\n    section Start\n      2024 : Kickoff\n      2025 : Beta";
    const QUADRANT: &str = "quadrantChart\n    title Reach\n    x-axis Low --> High\n    y-axis Low --> High\n    quadrant-1 Quick wins\n    Point A: [0.3, 0.6]";
    const XYCHART: &str = "xychart-beta\n    title \"Sales\"\n    x-axis [jan, feb, mar]\n    bar [5, 6, 7]";
    const RADAR: &str = "radar-beta\n    axis a[\"Speed\"], b[\"Power\"], c[\"Range\"]\n    curve k[\"Mine\"]{1, 2, 3}";
    const SANKEY: &str = "sankey-beta\n\nCoal,Electricity,30\nElectricity,Homes,20";
    const KANBAN: &str = "kanban\n  todo[Backlog]\n    t1[Write docs]";
    const REQUIREMENT: &str = "requirementDiagram\n    requirement r1 {\n    id: 1\n    text: the system shall\n    risk: high\n    verifymethod: test\n    }\n    element e1 {\n    type: simulation\n    }\n    e1 - satisfies -> r1";
    const C4: &str = "C4Context\n    Person(customer, \"Customer\", \"A user\")\n    System(app, \"App\", \"The app\")\n    Rel(customer, app, \"Uses\")";
    const PACKET: &str = "packet-beta\n0-15: \"Flags\"\n16-31: \"Length\"";
    const TREEMAP: &str = "treemap-beta\n\"Root\"\n    \"Frontend\": 40\n    \"Backend\": 60";
    const BLOCK: &str = "block-beta\n  columns 2\n  Compute[\"Compute\"] Store[\"Store\"]";
    const ARCHITECTURE: &str = "architecture-beta\n    service db(database)[Database] in cloud\n    group cloud(cloud)[Cloud]";

    fn all_types() -> Vec<(&'static str, &'static str, &'static str)> {
        vec![
            ("flowchart", FLOWCHART, "Is it ready"),
            ("sequence", SEQUENCE, "Hello Bob"),
            ("class", CLASS, "Animal"),
            ("state", STATE, "Running"),
            ("er", ER, "CUSTOMER"),
            ("gantt", GANTT, "Release plan"),
            ("pie", PIE, "Dogs"),
            ("gitgraph", GITGRAPH, "develop"),
            ("journey", JOURNEY, "Wake up"),
            ("mindmap", MINDMAP, "Editor"),
            ("timeline", TIMELINE, "Kickoff"),
            ("quadrant", QUADRANT, "Quick wins"),
            ("xychart", XYCHART, "Sales"),
            ("radar", RADAR, "Speed"),
            ("sankey", SANKEY, "Electricity"),
            ("kanban", KANBAN, "Backlog"),
            ("requirement", REQUIREMENT, "r1"),
            ("c4", C4, "Customer"),
            ("packet", PACKET, "Flags"),
            ("treemap", TREEMAP, "Frontend"),
            ("block", BLOCK, "Compute"),
            ("architecture", ARCHITECTURE, "Database"),
        ]
    }

    #[test]
    fn every_supported_type_renders_svg_with_labels() {
        let mut failures = Vec::new();
        for (name, src, label) in all_types() {
            let svg = match render(src, false, None) {
                Ok(svg) => svg,
                Err(e) => {
                    failures.push(format!("{name}: {e}"));
                    continue;
                }
            };
            if !svg.starts_with("<svg") || !svg.contains("</svg>") {
                failures.push(format!("{name}: not a complete svg"));
            }
            if !svg.contains(label) {
                failures.push(format!("{name}: label {label:?} missing from output"));
            }
            if svg_size(&svg).is_none() {
                failures.push(format!("{name}: no intrinsic size"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn invalid_source_is_a_readable_error() {
        let err = render("this is not mermaid at all {{{", false, None).unwrap_err();
        assert!(err.starts_with("Mermaid syntax error"), "{err}");
        assert!(err.len() < 400, "error too long: {err}");
        assert!(render("   \n  ", false, None).is_err());
    }

    #[test]
    fn dark_and_light_differ() {
        let light = render(FLOWCHART, false, None).unwrap();
        let dark = render(FLOWCHART, true, None).unwrap();
        assert_ne!(light, dark);
        assert!(light.contains("#006fee"), "light accent missing");
        assert!(dark.contains("#63b3ff"), "dark accent missing");
        assert!(dark.contains("#18181b"), "dark background missing");
        assert!(!dark.contains("#ffffff"), "dark output still has white fills");
    }

    #[test]
    fn background_override_is_used_and_validated() {
        let svg = render(FLOWCHART, false, Some("#eff1f5")).unwrap();
        assert!(svg.contains("#eff1f5"));
        let err = render(FLOWCHART, false, Some("red\" onload=\"x")).unwrap_err();
        assert!(err.contains("invalid diagram background colour"), "{err}");
    }

    #[test]
    fn render_for_theme_follows_app_background() {
        let mut theme = crate::theme::Theme::for_name("dark", gpui::WindowAppearance::Light);
        let dark_svg = render_for_theme(FLOWCHART, &theme).unwrap();
        assert!(dark_svg.contains("#63b3ff"));
        assert!(dark_svg.contains(&to_hex(theme.bg)));
        theme = crate::theme::Theme::for_name("light", gpui::WindowAppearance::Dark);
        let light_svg = render_for_theme(FLOWCHART, &theme).unwrap();
        assert!(light_svg.contains("#006fee"));
    }

    #[test]
    fn cache_returns_identical_output_and_hits() {
        let src = "flowchart LR\n  cacheprobe_a --> cacheprobe_b";
        let first = render(src, true, None).unwrap();
        let key = (hash64(src.as_bytes()), palette_key(true, None));
        assert!(cache().lock().unwrap().contains_key(&key), "result not cached");
        let second = render(src, true, None).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn size_guard_rejects_huge_sources() {
        let big = format!("flowchart TD\n{}", "A --> B\n".repeat(MAX_SOURCE_BYTES / 8));
        assert!(big.len() > MAX_SOURCE_BYTES);
        let err = render(&big, false, None).unwrap_err();
        assert!(err.contains("larger than"), "{err}");
    }

    #[test]
    fn node_guard_rejects_too_many_nodes() {
        let mut src = String::from("flowchart LR\n");
        for i in 0..=MAX_NODES {
            src.push_str(&format!("n{i}[Node {i}] --> n{}\n", i + 1));
        }
        assert!(count_nodes(&src) > MAX_NODES);
        let err = render(&src, false, None).unwrap_err();
        assert!(err.contains("nodes"), "{err}");
    }

    #[test]
    fn node_count_ignores_labels() {
        // Start, Decide, Ship, Wait, plus the `flowchart` and `TD` header tokens.
        assert_eq!(count_nodes(FLOWCHART), 6);
        assert_eq!(count_nodes("flowchart LR\n  A-->B"), 4);
    }

    #[test]
    fn large_diagram_renders_within_budget() {
        let mut src = String::from("flowchart LR\n");
        for i in 0..300 {
            src.push_str(&format!("n{i}[Step {i}] --> n{}\n", i + 1));
        }
        let start = std::time::Instant::now();
        let svg = render(&src, true, None).expect("300-node flowchart should render");
        let took = start.elapsed();
        assert!(svg.contains("Step 299"));
        assert!(took < RENDER_TIMEOUT, "took {took:?}");
    }

    #[test]
    fn source_cannot_break_out_of_svg_attributes() {
        let src = "%%{init: {\"themeVariables\": {\"primaryColor\": \"\\\"/><script>alert(1)</script>\"}}}%%\nflowchart TD\n  A[\"<img src=x onerror=alert(1)>\"] --> B";
        match render(src, false, None) {
            Ok(svg) => {
                assert!(!svg.contains("<script"), "script tag leaked into svg");
                assert!(!svg.contains("onerror=alert"), "event attribute leaked into svg");
            }
            Err(e) => assert!(!e.is_empty()),
        }
    }

    #[test]
    fn svg_size_reads_width_height_then_viewbox() {
        assert_eq!(svg_size(r#"<svg width="120" height="80" viewBox="0 0 120 80">"#), Some((120.0, 80.0)));
        assert_eq!(svg_size(r#"<svg width="10px" height="20px">"#), Some((10.0, 20.0)));
        assert_eq!(svg_size(r#"<svg viewBox="0 0 300 150">"#), Some((300.0, 150.0)));
        assert_eq!(svg_size(r#"<svg width="100%" viewBox="0 0 50 60">"#), Some((50.0, 60.0)));
        assert_eq!(svg_size(r#"<svg stroke-width="3" width="7" height="9">"#), Some((7.0, 9.0)));
        assert_eq!(svg_size("<svg>"), None);
        assert_eq!(svg_size("not svg"), None);
    }

    #[test]
    fn to_image_path_is_idempotent_and_content_addressed() {
        let svg = render(FLOWCHART, false, None).unwrap();
        let a = to_image_path(&svg).unwrap();
        let b = to_image_path(&svg).unwrap();
        assert_eq!(a, b);
        assert!(a.starts_with(std::env::temp_dir().join("zyplus-diagrams")));
        assert_eq!(fs::read_to_string(&a).unwrap(), svg);
        let other = to_image_path(&format!("{svg}<!-- x -->")).unwrap();
        assert_ne!(a, other);
    }
}
