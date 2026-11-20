//! Full-screen viewer for a ```chart or ```mermaid fence, drawn over the whole
//! window with zoom and pan.
//!
//! The zoom/pan arithmetic (`ZoomPan`) is pure and unit tested. The viewer is an
//! overlay owned by `Zyplus` (`fullscreen`). A chart is drawn from vectors, so it
//! stays sharp at any zoom; a Mermaid diagram is an SVG rasterised at a larger
//! size than the editor uses, so it stays sharp up to a few times its size.
//!
//! Controls: drag to pan; scroll (trackpad) to pan; ⌘/Ctrl + scroll, a mouse
//! wheel, the buttons or `+` / `-` to zoom around the pointer; `0` or the fit
//! button to reset; double-click toggles 2x; arrows pan; Esc closes.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::{
    canvas, px, Bounds, FocusHandle, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point,
    ScrollDelta, ScrollWheelEvent, WindowControlArea,
};

use super::*;
use crate::chart::{self, ChartSpec, Xf};
use crate::rich_blocks::WidgetKind;

pub const MIN_ZOOM: f32 = 0.25;
pub const MAX_ZOOM: f32 = 16.0;
/// Step of the zoom buttons and keys.
pub const ZOOM_STEP: f32 = 1.25;
/// Content that must stay inside the view when panning, in pixels.
const KEEP_VISIBLE: f32 = 64.0;
const PAN_KEY_STEP: f32 = 48.0;
const TOP_BAR_H: f32 = 44.0;

/// The zoom factor and offset of the chart, relative to the top left of the
/// view. At `zoom == 1` with no offset the chart fills the view ("fit").
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZoomPan {
    pub zoom: f32,
    pub dx: f32,
    pub dy: f32,
}

impl Default for ZoomPan {
    fn default() -> Self {
        ZoomPan {
            zoom: 1.0,
            dx: 0.0,
            dy: 0.0,
        }
    }
}

impl ZoomPan {
    pub fn xf(self) -> Xf {
        Xf {
            zoom: self.zoom,
            dx: self.dx,
            dy: self.dy,
        }
    }

    /// Multiplies the zoom by `factor`, keeping the content under view-local
    /// `(cx, cy)` where it is.
    pub fn zoom_at(&mut self, cx: f32, cy: f32, factor: f32) {
        let next = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let k = next / self.zoom;
        self.dx = cx - (cx - self.dx) * k;
        self.dy = cy - (cy - self.dy) * k;
        self.zoom = next;
    }

    pub fn pan_by(&mut self, ddx: f32, ddy: f32) {
        self.dx += ddx;
        self.dy += ddy;
    }

    /// Keeps at least `KEEP_VISIBLE` pixels of the chart (`view_w` x `view_h`
    /// at zoom 1) inside the view, so it cannot be flung out of sight.
    pub fn clamp(&mut self, view_w: f32, view_h: f32) {
        let keep_x = KEEP_VISIBLE.min(view_w * self.zoom).min(view_w);
        let keep_y = KEEP_VISIBLE.min(view_h * self.zoom).min(view_h);
        self.dx = self.dx.clamp(keep_x - view_w * self.zoom, view_w - keep_x);
        self.dy = self.dy.clamp(keep_y - view_h * self.zoom, view_h - keep_y);
    }

    /// Back to fit.
    pub fn reset(&mut self) {
        *self = ZoomPan::default();
    }

    /// Double-click: fit when zoomed, otherwise 2x around the pointer.
    pub fn toggle_at(&mut self, cx: f32, cy: f32) {
        if (self.zoom - 1.0).abs() > 0.01 {
            self.reset();
        } else {
            self.zoom_at(cx, cy, 2.0);
        }
    }

    pub fn percent(self) -> u32 {
        (self.zoom * 100.0).round() as u32
    }
}

/// Diagrams are never fitted larger than this multiple of their size.
const MAX_FIT_UPSCALE: f32 = 2.0;
/// Space kept around a fitted diagram.
const FIT_PAD: f32 = 24.0;
/// Longest side of the rasterised diagram, in pixels (the rasteriser adds 2x).
const MAX_RASTER_SIDE: f32 = 6000.0;

/// Where a diagram of `natural` size sits when fitted into an `area`: `(x, y, w, h)`
/// in view space, centred, never enlarged past `MAX_FIT_UPSCALE`.
pub fn fit_rect(natural: (f32, f32), area: (f32, f32)) -> (f32, f32, f32, f32) {
    let (nw, nh) = (natural.0.max(1.0), natural.1.max(1.0));
    let (aw, ah) = (area.0.max(1.0), area.1.max(1.0));
    let scale = ((aw - 2.0 * FIT_PAD).max(1.0) / nw)
        .min((ah - 2.0 * FIT_PAD).max(1.0) / nh)
        .min(MAX_FIT_UPSCALE);
    let (w, h) = (nw * scale, nh * scale);
    ((aw - w) / 2.0, (ah - h) / 2.0, w, h)
}

/// A rendered diagram: the SVG file and its natural size.
#[derive(Clone)]
struct DiagramImage {
    path: PathBuf,
    size: (f32, f32),
}

/// What the viewer shows.
enum Content {
    Chart(ChartSpec),
    /// The source; `image` is rendered on the first frame, when the theme is known.
    Mermaid {
        source: String,
        image: Option<DiagramImage>,
    },
}

/// An open full-screen chart or diagram.
pub struct FullscreenView {
    content: Content,
    pub title: String,
    pub view: ZoomPan,
    /// While the pointer is held: where it went down and the pan it started from.
    drag: Option<(Point<Pixels>, f32, f32)>,
    /// Bounds of the drawing area from the last frame, for pointer maths.
    area: Rc<Cell<Bounds<Pixels>>>,
    pub focus: FocusHandle,
}

impl FullscreenView {
    fn new(content: Content, title: String, focus: FocusHandle) -> Self {
        FullscreenView {
            content,
            title,
            view: ZoomPan::default(),
            drag: None,
            area: Rc::new(Cell::new(Bounds::default())),
            focus,
        }
    }

    pub fn chart(spec: ChartSpec, focus: FocusHandle) -> Self {
        let title = spec.title.clone().unwrap_or_else(|| "Chart".to_string());
        Self::new(Content::Chart(spec), title, focus)
    }

    pub fn mermaid(source: String, focus: FocusHandle) -> Self {
        Self::new(
            Content::Mermaid {
                source,
                image: None,
            },
            "Mermaid diagram".to_string(),
            focus,
        )
    }

    /// Renders a Mermaid diagram on first use. The error is the renderer's message.
    fn ensure_image(&mut self, theme: &Theme) -> Result<(), String> {
        let Content::Mermaid { source, image } = &mut self.content else {
            return Ok(());
        };
        if image.is_some() {
            return Ok(());
        }
        let svg = crate::mermaid::render_for_theme(source, theme)?;
        let size = crate::mermaid::svg_size(&svg).unwrap_or((400.0, 200.0));
        let factor = (MAX_RASTER_SIDE / (2.0 * size.0.max(size.1).max(1.0))).clamp(1.0, 4.0);
        let path = crate::mermaid::to_image_path(&crate::mermaid::scale_svg(&svg, factor))?;
        *image = Some(DiagramImage { path, size });
        Ok(())
    }

    /// The pointer went up: the drag ends.
    pub(crate) fn release(&mut self) {
        self.drag = None;
    }

    fn area(&self) -> Bounds<Pixels> {
        self.area.get()
    }

    /// A window point as view-local pixels.
    fn local(&self, p: Point<Pixels>) -> (f32, f32) {
        let a = self.area();
        (f32::from(p.x - a.origin.x), f32::from(p.y - a.origin.y))
    }

    fn clamp(&mut self) {
        let a = self.area();
        let (w, h) = (f32::from(a.size.width), f32::from(a.size.height));
        if w > 0.0 && h > 0.0 {
            self.view.clamp(w, h);
        }
    }

    fn zoom_centre(&mut self, factor: f32) {
        let a = self.area();
        let (w, h) = (f32::from(a.size.width), f32::from(a.size.height));
        self.view.zoom_at(w / 2.0, h / 2.0, factor);
        self.clamp();
    }
}

/// Pixels of pointer travel that double the zoom while ⌘/Ctrl-scrolling a trackpad.
const SCROLL_ZOOM_PX: f32 = 220.0;
/// Zoom factor per line of a notched mouse wheel.
const WHEEL_STEP: f32 = 1.12;

impl Zyplus {
    /// Opens the viewer for the body of a chart or Mermaid fence. A chart that
    /// does not parse is reported in the error banner instead.
    pub(crate) fn open_fullscreen(&mut self, kind: WidgetKind, src: &str, cx: &mut Context<Self>) {
        let view = match kind {
            WidgetKind::Chart => match chart::parse(src) {
                Ok(spec) => FullscreenView::chart(spec, cx.focus_handle()),
                Err(message) => {
                    self.error = Some(format!("Chart error: {message}").into());
                    cx.notify();
                    return;
                }
            },
            WidgetKind::Mermaid => FullscreenView::mermaid(src.to_string(), cx.focus_handle()),
            _ => return,
        };
        self.menu = None;
        self.fullscreen = Some(view);
        self.focus_fullscreen = true;
        cx.notify();
    }

    pub(crate) fn close_fullscreen(&mut self, cx: &mut Context<Self>) {
        if self.fullscreen.take().is_some() {
            self.refocus_editor = true;
        }
        cx.notify();
    }

    fn fs_key(&mut self, event: &gpui::KeyDownEvent, cx: &mut Context<Self>) {
        let Some(viewer) = self.fullscreen.as_mut() else {
            return;
        };
        let key = event.keystroke.key.as_str();
        let typed = event.keystroke.key_char.as_deref().unwrap_or(key);
        match (key, typed) {
            (_, "+") | ("=", _) | ("+", _) => viewer.zoom_centre(ZOOM_STEP),
            ("-", _) | (_, "-") | ("_", _) => viewer.zoom_centre(1.0 / ZOOM_STEP),
            ("0", _) => viewer.view.reset(),
            ("left", _) => viewer.view.pan_by(PAN_KEY_STEP, 0.0),
            ("right", _) => viewer.view.pan_by(-PAN_KEY_STEP, 0.0),
            ("up", _) => viewer.view.pan_by(0.0, PAN_KEY_STEP),
            ("down", _) => viewer.view.pan_by(0.0, -PAN_KEY_STEP),
            _ => return,
        }
        viewer.clamp();
        cx.stop_propagation();
        cx.notify();
    }

    pub(crate) fn fs_wheel(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let Some(viewer) = self.fullscreen.as_mut() else {
            return;
        };
        let (px_, py_) = viewer.local(event.position);
        match event.delta {
            ScrollDelta::Lines(lines) => {
                viewer.view.zoom_at(px_, py_, WHEEL_STEP.powf(lines.y));
            }
            ScrollDelta::Pixels(delta) => {
                if event.modifiers.secondary() || event.modifiers.control {
                    let factor = 2f32.powf(f32::from(delta.y) / SCROLL_ZOOM_PX);
                    viewer.view.zoom_at(px_, py_, factor);
                } else {
                    viewer.view.pan_by(f32::from(delta.x), f32::from(delta.y));
                }
            }
        }
        viewer.clamp();
        cx.stop_propagation();
        cx.notify();
    }

    pub(crate) fn fs_mouse_down(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let Some(viewer) = self.fullscreen.as_mut() else {
            return;
        };
        if event.click_count >= 2 {
            let (x, y) = viewer.local(event.position);
            viewer.view.toggle_at(x, y);
            viewer.clamp();
            viewer.drag = None;
        } else {
            viewer.drag = Some((event.position, viewer.view.dx, viewer.view.dy));
        }
        cx.stop_propagation();
        cx.notify();
    }

    pub(crate) fn fs_mouse_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(viewer) = self.fullscreen.as_mut() else {
            return;
        };
        let Some((from, dx, dy)) = viewer.drag else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            viewer.drag = None;
            return;
        }
        viewer.view.dx = dx + f32::from(event.position.x - from.x);
        viewer.view.dy = dy + f32::from(event.position.y - from.y);
        viewer.clamp();
        cx.notify();
    }

    /// The overlay: a top bar with the zoom controls over the canvas.
    pub(crate) fn render_fullscreen(
        &mut self,
        theme: &Theme,
        viewport: gpui::Size<Pixels>,
        chrome: &window_chrome::Chrome,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        if let Some(Err(message)) = self.fullscreen.as_mut().map(|v| v.ensure_image(theme)) {
            self.fullscreen = None;
            self.refocus_editor = true;
            self.error = Some(format!("Mermaid error: {message}").into());
            return None;
        }
        let viewer = self.fullscreen.as_ref()?;
        let (spec, diagram) = match &viewer.content {
            Content::Chart(spec) => (Some(spec.clone()), None),
            Content::Mermaid { image, .. } => (None, image.clone()),
        };
        let view = viewer.view;
        let area = viewer.area.clone();
        let focus = viewer.focus.clone();
        let title = viewer.title.clone();
        let panning = viewer.drag.is_some();
        let (fg, grid, muted) = (theme.fg, theme.border, theme.muted);

        let tool = |id: &'static str, icon: &'static str| {
            icon_button(
                theme,
                id,
                app_icon::icon(icon, 16.0).text_color(muted),
                28.0,
            )
        };
        let zoom_out =
            tool("chart-zoom-out", "search-minus").on_click(cx.listener(|this, _, _, cx| {
                if let Some(v) = this.fullscreen.as_mut() {
                    v.zoom_centre(1.0 / ZOOM_STEP);
                }
                cx.notify();
            }));
        let zoom_in =
            tool("chart-zoom-in", "search-add").on_click(cx.listener(|this, _, _, cx| {
                if let Some(v) = this.fullscreen.as_mut() {
                    v.zoom_centre(ZOOM_STEP);
                }
                cx.notify();
            }));
        let fit = tool("chart-fit", "center-focus").on_click(cx.listener(|this, _, _, cx| {
            if let Some(v) = this.fullscreen.as_mut() {
                v.view.reset();
            }
            cx.notify();
        }));
        let close = tool("chart-close", "cancel01")
            .on_click(cx.listener(|this, _, _, cx| this.close_fullscreen(cx)));

        let close_entity = cx.entity().downgrade();
        let win_controls =
            window_chrome::window_controls(chrome, theme, "fullscreen", move |window, cx| {
                let should_close = close_entity
                    .update(cx, |this, cx| {
                        let ok = this.confirm_quit(window, cx);
                        if !ok {
                            this.quitting = true;
                            this.fullscreen = None;
                        }
                        ok
                    })
                    .unwrap_or(true);
                if should_close {
                    window.remove_window();
                }
            });

        let bar = div()
            .h(px(TOP_BAR_H))
            .flex_shrink_0()
            .flex()
            .flex_row()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .when(chrome.os == platform::Os::Mac, |el| {
                el.window_control_area(WindowControlArea::Drag)
            })
            .child(
                window_chrome::drag_area(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .h_full()
                        .flex()
                        .items_center()
                        // Room for the traffic lights on macOS, normal padding elsewhere.
                        .pl(px(chrome.fullscreen_left_pad())),
                    chrome,
                )
                .text_size(px(13.0))
                .text_color(fg)
                .child(title),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .flex_shrink_0()
                    .pr_3()
                    .child(zoom_out)
                    .child(
                        div()
                            .w(px(48.0))
                            .flex()
                            .justify_center()
                            .text_size(px(12.0))
                            .text_color(muted)
                            .child(format!("{}%", view.percent())),
                    )
                    .child(zoom_in)
                    .child(fit)
                    .child(close),
            )
            .children(win_controls);

        let surface = div()
            .id("chart-viewer-canvas")
            .flex_1()
            .min_h(px(0.0))
            .relative()
            .overflow_hidden()
            .cursor(if panning { gpui::CursorStyle::ClosedHand } else { gpui::CursorStyle::OpenHand })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| this.fs_mouse_down(event, cx)),
            )
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| this.fs_wheel(event, cx)))
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds: Bounds<Pixels>, _, window, cx| {
                        area.set(bounds);
                        if let Some(spec) = &spec {
                            let scene = chart::build_scene(
                                spec,
                                f32::from(bounds.size.width),
                                f32::from(bounds.size.height),
                                fg,
                                grid,
                            );
                            chart::paint_scene_xf(&scene, bounds.origin, view.xf(), window, cx);
                        }
                    },
                )
                .size_full(),
            )
            .children(diagram.map(|d| {
                let (x, y, w, h) = fit_rect(
                    d.size,
                    (f32::from(viewport.width), f32::from(viewport.height) - TOP_BAR_H),
                );
                let z = view.zoom;
                gpui::img(d.path)
                    .absolute()
                    .left(px(x * z + view.dx))
                    .top(px(y * z + view.dy))
                    .w(px(w * z))
                    .h(px(h * z))
            }))
            .child(
                div()
                    .absolute()
                    .left(px(12.0))
                    .bottom(px(8.0))
                    .text_size(px(11.0))
                    .text_color(muted)
                    .child("Drag or scroll to pan · ⌘/Ctrl + scroll or +/− to zoom · double-click toggles 2x · Esc closes"),
            );

        Some(
            div()
                .id("chart-viewer")
                .key_context("FullscreenView")
                .track_focus(&focus)
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .flex_col()
                .bg(theme.bg)
                .text_color(theme.fg)
                .occlude()
                .on_key_down(
                    cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| this.fs_key(event, cx)),
                )
                .on_mouse_move(
                    cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                        this.fs_mouse_move(event, cx)
                    }),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        if let Some(v) = this.fullscreen.as_mut() {
                            v.release();
                        }
                        cx.notify();
                    }),
                )
                .child(bar)
                .child(surface),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    /// The scene point under a view point, before and after the same zoom.
    fn scene_at(v: ZoomPan, x: f32, y: f32) -> (f32, f32) {
        ((x - v.dx) / v.zoom, (y - v.dy) / v.zoom)
    }

    #[test]
    fn zoom_keeps_the_point_under_the_pointer_fixed() {
        let mut v = ZoomPan::default();
        v.pan_by(30.0, -12.0);
        let before = scene_at(v, 200.0, 150.0);
        v.zoom_at(200.0, 150.0, 2.5);
        let after = scene_at(v, 200.0, 150.0);
        assert!(close(before.0, after.0) && close(before.1, after.1));
        assert!(close(v.zoom, 2.5));
    }

    #[test]
    fn zoom_is_limited() {
        let mut v = ZoomPan::default();
        for _ in 0..60 {
            v.zoom_at(0.0, 0.0, 2.0);
        }
        assert!(close(v.zoom, MAX_ZOOM));
        for _ in 0..60 {
            v.zoom_at(0.0, 0.0, 0.5);
        }
        assert!(close(v.zoom, MIN_ZOOM));
    }

    #[test]
    fn pan_cannot_lose_the_chart() {
        let mut v = ZoomPan::default();
        v.pan_by(10_000.0, -10_000.0);
        v.clamp(800.0, 600.0);
        assert!(close(v.dx, 800.0 - KEEP_VISIBLE));
        assert!(close(v.dy, KEEP_VISIBLE - 600.0));
        // Zoomed in, the chart is bigger than the view: more room to pan left.
        let mut z = ZoomPan {
            zoom: 4.0,
            dx: -100_000.0,
            dy: 0.0,
        };
        z.clamp(800.0, 600.0);
        assert!(close(z.dx, KEEP_VISIBLE - 800.0 * 4.0));
        // A fitted chart stays put.
        let mut f = ZoomPan::default();
        f.clamp(800.0, 600.0);
        assert_eq!(f, ZoomPan::default());
    }

    #[test]
    fn double_click_toggles_between_fit_and_two_x() {
        let mut v = ZoomPan::default();
        v.toggle_at(100.0, 100.0);
        assert!(close(v.zoom, 2.0));
        v.toggle_at(100.0, 100.0);
        assert_eq!(v, ZoomPan::default());
    }

    #[test]
    fn diagrams_fit_centred_and_are_not_blown_up() {
        // Wider than the area: scaled down to the padded width, centred vertically.
        let (x, y, w, h) = fit_rect((2000.0, 500.0), (1000.0, 700.0));
        assert!(close(w, 1000.0 - 2.0 * FIT_PAD) && close(h, w / 4.0));
        assert!(close(x, FIT_PAD) && close(y, (700.0 - h) / 2.0));
        // A small diagram is enlarged at most MAX_FIT_UPSCALE times.
        let (_, _, w, h) = fit_rect((100.0, 50.0), (1000.0, 700.0));
        assert!(close(w, 200.0) && close(h, 100.0));
        // Degenerate sizes do not divide by zero.
        let (_, _, w, h) = fit_rect((0.0, 0.0), (0.0, 0.0));
        assert!(w.is_finite() && h.is_finite());
    }

    #[test]
    fn percent_reads_the_zoom() {
        assert_eq!(ZoomPan::default().percent(), 100);
        assert_eq!(
            ZoomPan {
                zoom: 1.25,
                dx: 0.0,
                dy: 0.0
            }
            .percent(),
            125
        );
    }
}
