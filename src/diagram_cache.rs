//! In-memory render cache for Mermaid diagrams and LaTeX math blocks.
//!
//! Diagrams are rendered to standalone SVGs and saved to temp files for gpui's
//! `img(PathBuf)`. Render results (image path and natural px size, or error message)
//! are cached in memory keyed by (source hash, theme), ensuring that a layout/paint
//! frame never re-renders an existing block.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Mutex;

use crate::math;
use crate::mermaid;
use crate::theme::Theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DiagramKind {
    Mermaid,
    Math,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RenderedDiagram {
    pub path: PathBuf,
    pub size: (f32, f32),
}

pub type RenderResult = Result<RenderedDiagram, String>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ThemeKey {
    dark: bool,
    bg: [u8; 4],
    fg: [u8; 4],
}

impl ThemeKey {
    fn from_theme(theme: &Theme) -> Self {
        let to_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        let lum = 0.299 * theme.bg.r + 0.587 * theme.bg.g + 0.114 * theme.bg.b;
        Self {
            dark: lum < 0.5,
            bg: [
                to_u8(theme.bg.r),
                to_u8(theme.bg.g),
                to_u8(theme.bg.b),
                to_u8(theme.bg.a),
            ],
            fg: [
                to_u8(theme.fg.r),
                to_u8(theme.fg.g),
                to_u8(theme.fg.b),
                to_u8(theme.fg.a),
            ],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct CacheKey {
    source_hash: u64,
    kind: DiagramKind,
    theme: ThemeKey,
}

const CACHE_CAPACITY: usize = 256;

struct Cache {
    entries: HashMap<CacheKey, RenderResult>,
    order: VecDeque<CacheKey>,
}

impl Cache {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn get(&self, key: &CacheKey) -> Option<RenderResult> {
        self.entries.get(key).cloned()
    }

    fn insert(&mut self, key: CacheKey, val: RenderResult) {
        if let std::collections::hash_map::Entry::Occupied(mut e) = self.entries.entry(key) {
            let _ = e.insert(val);
            return;
        }
        if self.entries.len() >= CACHE_CAPACITY {
            if let Some(old) = self.order.pop_front() {
                self.entries.remove(&old);
            }
        }
        self.order.push_back(key);
        self.entries.insert(key, val);
    }

    #[cfg(test)]
    fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

fn with_cache<R>(f: impl FnOnce(&mut Cache) -> R) -> R {
    let mut lock = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let cache = lock.get_or_insert_with(Cache::new);
    f(cache)
}

/// Computes the 64-bit hash of diagram source text.
pub fn hash_source(source: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    hasher.finish()
}

/// Scale a diagram's natural size to column width without upscaling.
/// Keeps aspect ratio intact.
pub fn scale_diagram_size(natural: (f32, f32), col_w: f32) -> (f32, f32) {
    let (mut nw, mut nh) = natural;
    if nw <= 0.0 || nh <= 0.0 {
        nw = 400.0;
        nh = 100.0;
    }
    if col_w > 0.0 && nw > col_w {
        let scale = col_w / nw;
        (col_w, nh * scale)
    } else {
        (nw, nh)
    }
}

/// Retrieve a cached render or render uncached and store it.
pub fn get_or_render(kind: DiagramKind, source: &str, theme: &Theme) -> RenderResult {
    let key = CacheKey {
        source_hash: hash_source(source),
        kind,
        theme: ThemeKey::from_theme(theme),
    };

    if let Some(hit) = with_cache(|c| c.get(&key)) {
        return hit;
    }

    let rendered = render_uncached(kind, source, theme);
    with_cache(|c| c.insert(key, rendered.clone()));
    rendered
}

fn render_uncached(kind: DiagramKind, source: &str, theme: &Theme) -> RenderResult {
    match kind {
        DiagramKind::Mermaid => {
            let svg = mermaid::render_for_theme(source, theme)?;
            let size = mermaid::svg_size(&svg).unwrap_or((400.0, 200.0));
            let path = mermaid::to_image_path(&svg)?;
            Ok(RenderedDiagram { path, size })
        }
        DiagramKind::Math => {
            let svg = math::render_for_theme(source, theme)?;
            let size = math::svg_size(&svg).unwrap_or((300.0, 50.0));
            let path = math::to_image_path(&svg)?;
            Ok(RenderedDiagram { path, size })
        }
    }
}

#[cfg(test)]
pub fn clear_cache() {
    with_cache(|c| c.clear());
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::WindowAppearance;

    fn test_theme(dark: bool) -> Theme {
        Theme::for_name(
            "system",
            if dark {
                WindowAppearance::Dark
            } else {
                WindowAppearance::Light
            },
        )
    }

    #[test]
    fn scale_diagram_size_never_upscales() {
        // Smaller than column width -> kept as natural size
        assert_eq!(scale_diagram_size((200.0, 100.0), 600.0), (200.0, 100.0));
        assert_eq!(scale_diagram_size((400.0, 200.0), 400.0), (400.0, 200.0));

        // Larger than column width -> downscaled preserving aspect ratio
        let (w, h) = scale_diagram_size((1000.0, 500.0), 500.0);
        assert_eq!(w, 500.0);
        assert!((h - 250.0).abs() < 1e-3);

        // Degenerate sizes fall back to sensible defaults
        let (w, h) = scale_diagram_size((0.0, 0.0), 600.0);
        assert_eq!((w, h), (400.0, 100.0));
    }

    #[test]
    fn get_or_render_caches_successful_diagrams() {
        clear_cache();
        let theme = test_theme(true);
        let src = "flowchart LR\n    A --> B";
        let res1 = get_or_render(DiagramKind::Mermaid, src, &theme).expect("valid mermaid");
        assert!(res1.path.exists());
        assert!(res1.size.0 > 0.0 && res1.size.1 > 0.0);

        let res2 = get_or_render(DiagramKind::Mermaid, src, &theme).expect("cached mermaid");
        assert_eq!(res1, res2);
    }

    #[test]
    fn get_or_render_caches_successful_math() {
        clear_cache();
        let theme = test_theme(false);
        let src = "e^{i\\pi} + 1 = 0";
        let res1 = get_or_render(DiagramKind::Math, src, &theme).expect("valid math");
        assert!(res1.path.exists());
        assert!(res1.size.0 > 0.0 && res1.size.1 > 0.0);

        let res2 = get_or_render(DiagramKind::Math, src, &theme).expect("cached math");
        assert_eq!(res1, res2);
    }

    #[test]
    fn get_or_render_caches_render_errors() {
        clear_cache();
        let theme = test_theme(true);
        let src = "\\nonexistentcommand{xyz}";
        let res1 = get_or_render(DiagramKind::Math, src, &theme);
        assert!(res1.is_err());
        let err_msg1 = res1.unwrap_err();

        let res2 = get_or_render(DiagramKind::Math, src, &theme);
        assert!(res2.is_err());
        assert_eq!(res2.unwrap_err(), err_msg1);
    }
}
