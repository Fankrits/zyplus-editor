//! Platform-specific window chrome, decorations, caption controls, and
//! lifecycle decisions.
//!
//! Every platform decision is expressed as a pure function of [`Os`] and
//! [`WindowDecorationState`] so that the macOS, Windows, and Linux (both
//! server-side and client-side decoration) branches compile and are unit-tested
//! on every host.
//!
//! - **macOS**: transparent titlebar with native traffic lights at `(16, 15)`
//!   and an 84 px left inset; no app-drawn caption buttons; app stays running
//!   when its last window closes.
//! - **Windows**: transparent titlebar (`hide_title_bar = true`) with app-drawn
//!   minimise / maximise-restore / close buttons at the top-right tagged with
//!   [`WindowControlArea::Min`], [`WindowControlArea::Max`], and
//!   [`WindowControlArea::Close`] so Windows natively hit-tests them (including
//!   Snap Layouts and `WM_CLOSE` -> `on_window_should_close`), plus
//!   [`WindowControlArea::Drag`] on non-interactive top-bar regions.
//! - **Linux (server-side decorations — X11 WM, KDE/Wayland, etc.)**: native
//!   titlebar (`appears_transparent = false`), zero left inset, no app-drawn
//!   caption buttons, no resize border.
//! - **Linux (client-side decorations — GNOME/Wayland, etc.)**: app-drawn
//!   caption buttons wired to `minimize_window` / `zoom_window` / close
//!   callback, drag-to-move (`start_window_move`), double-click maximize,
//!   right-click window menu (`show_window_menu`), and outer shadow / resize
//!   edges (`start_window_resize`) with `set_client_inset`.

use crate::app_icon;
use crate::platform::Os;
use crate::theme::Theme;
use gpui::{
    canvas, div, point, prelude::*, px, rgb, size, AnyElement, Bounds, CursorStyle, Decorations,
    Div, HitboxBehavior, Hsla, MouseButton, Pixels, Point, ResizeEdge, Rgba, SharedString, Size,
    Tiling, TitlebarOptions, Window, WindowBounds, WindowControlArea, WindowControls,
    WindowDecorations, WindowOptions,
};

/// Space the macOS traffic lights take at the top-left of the window.
pub const MAC_TRAFFIC_LIGHT_INSET: f32 = 84.0;
pub const MAC_TRAFFIC_LIGHT_X: f32 = 16.0;
pub const MAC_TRAFFIC_LIGHT_Y: f32 = 15.0;

/// Desktop application identifier matching `StartupWMClass` in
/// `packaging/linux/zyplus.desktop`.
pub const APP_ID: &str = "zyplus-gpui";

/// Height of the sidebar header, tab bar, and fullscreen top bar.
pub const TOP_BAR_HEIGHT: f32 = 44.0;

/// Initial and minimum window dimensions.
pub const DEFAULT_WINDOW_WIDTH: f32 = 1180.0;
pub const DEFAULT_WINDOW_HEIGHT: f32 = 800.0;
pub const MIN_WINDOW_WIDTH: f32 = 560.0;
pub const MIN_WINDOW_HEIGHT: f32 = 360.0;

/// Linux client-side decoration frame geometry (matching gpui's `window_shadow`
/// example).
pub const CSD_SHADOW_SIZE: f32 = 10.0;
pub const CSD_CORNER_RADIUS: f32 = 10.0;
pub const CSD_BORDER_WIDTH: f32 = 1.0;

/// Caption button widths.
pub const WIN_CONTROL_WIDTH: f32 = 46.0;
pub const LINUX_CONTROL_WIDTH: f32 = 38.0;

/// Default left padding on top bars when no traffic-light inset is needed.
pub const SIDEBAR_DEFAULT_LEFT_PAD: f32 = 12.0;
pub const TAB_BAR_DEFAULT_LEFT_PAD: f32 = 8.0;
pub const FULLSCREEN_DEFAULT_LEFT_PAD: f32 = 12.0;

/// A window caption button drawn by the app when the OS does not draw one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Minimize,
    /// Maximizes when windowed, restores when maximized.
    Maximize,
    Close,
}

/// Snapshot of a platform window's decoration and control state, decoupled from
/// [`Window`] so every OS x decoration combination can be unit-tested.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowDecorationState {
    pub decorations: Decorations,
    pub controls: WindowControls,
    pub is_maximized: bool,
    pub is_fullscreen: bool,
}

impl Default for WindowDecorationState {
    fn default() -> Self {
        Self {
            decorations: Decorations::Server,
            controls: WindowControls::default(),
            is_maximized: false,
            is_fullscreen: false,
        }
    }
}

impl WindowDecorationState {
    pub fn from_window(window: &Window) -> Self {
        Self {
            decorations: window.window_decorations(),
            controls: window.window_controls(),
            is_maximized: window.is_maximized(),
            is_fullscreen: window.is_fullscreen(),
        }
    }
}

/// Complete layout and behaviour description for the window chrome on one frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Chrome {
    pub os: Os,
    /// Left inset to clear macOS traffic lights (`84.0` on macOS, `0.0` elsewhere).
    pub left_inset: f32,
    /// Caption buttons rendered flush on the top-right, left-to-right.
    pub right_controls: Vec<Control>,
    /// Whether the window is currently maximized (switches Maximize glyph to Restore).
    pub is_maximized: bool,
    /// Whether caption buttons rely on Windows `WindowControlArea` hit-testing
    /// rather than GPUI `on_click` handlers.
    pub native_hit_test_controls: bool,
    /// Whether top-bar drag areas initiate compositor moves (`start_window_move`)
    /// on Linux client-side decorations.
    pub client_drag: bool,
    /// Whether right-clicking a Linux CSD titlebar should open the compositor's
    /// window menu (`show_window_menu`).
    pub supports_window_menu: bool,
    /// Whether double-clicking a Linux CSD titlebar can toggle maximize.
    pub supports_maximize: bool,
    /// Whether to wrap the root view in a client-side shadow/border/resize frame.
    pub draw_resize_edges: bool,
    /// Edge tiling state when `draw_resize_edges` is active.
    pub tiling: Tiling,
    /// Client inset passed to `window.set_client_inset` (shadow margin on Linux CSD).
    pub client_inset: f32,
}

impl Chrome {
    pub fn shows_window_controls(&self) -> bool {
        !self.right_controls.is_empty()
    }

    pub fn control_button_width(&self) -> f32 {
        match self.os {
            Os::Windows => WIN_CONTROL_WIDTH,
            Os::Linux | Os::Mac => LINUX_CONTROL_WIDTH,
        }
    }

    #[cfg(test)]
    pub fn right_controls_width(&self) -> f32 {
        self.right_controls.len() as f32 * self.control_button_width()
    }

    pub fn sidebar_left_pad(&self) -> f32 {
        if self.left_inset > 0.0 {
            self.left_inset
        } else {
            SIDEBAR_DEFAULT_LEFT_PAD
        }
    }

    pub fn collapsed_tab_bar_left_pad(&self) -> f32 {
        if self.left_inset > 0.0 {
            self.left_inset
        } else {
            TAB_BAR_DEFAULT_LEFT_PAD
        }
    }

    pub fn fullscreen_left_pad(&self) -> f32 {
        if self.left_inset > 0.0 {
            self.left_inset
        } else {
            FULLSCREEN_DEFAULT_LEFT_PAD
        }
    }
}

/// Computes the window chrome configuration for an OS and decoration state.
pub fn chrome_for(os: Os, state: WindowDecorationState) -> Chrome {
    match os {
        Os::Mac => Chrome {
            os,
            left_inset: MAC_TRAFFIC_LIGHT_INSET,
            right_controls: Vec::new(),
            is_maximized: state.is_maximized,
            native_hit_test_controls: false,
            client_drag: false,
            supports_window_menu: false,
            supports_maximize: false,
            draw_resize_edges: false,
            tiling: Tiling::default(),
            client_inset: 0.0,
        },
        Os::Windows => Chrome {
            os,
            left_inset: 0.0,
            right_controls: vec![Control::Minimize, Control::Maximize, Control::Close],
            is_maximized: state.is_maximized,
            native_hit_test_controls: true,
            client_drag: false,
            supports_window_menu: false,
            supports_maximize: true,
            draw_resize_edges: false,
            tiling: Tiling::default(),
            client_inset: 0.0,
        },
        Os::Linux => match state.decorations {
            Decorations::Server => Chrome {
                os,
                left_inset: 0.0,
                right_controls: Vec::new(),
                is_maximized: state.is_maximized,
                native_hit_test_controls: false,
                client_drag: false,
                supports_window_menu: false,
                supports_maximize: false,
                draw_resize_edges: false,
                tiling: Tiling::default(),
                client_inset: 0.0,
            },
            Decorations::Client { tiling } => {
                let mut right_controls = Vec::with_capacity(3);
                if state.controls.minimize {
                    right_controls.push(Control::Minimize);
                }
                if state.controls.maximize {
                    right_controls.push(Control::Maximize);
                }
                right_controls.push(Control::Close);

                let effective_tiling = if state.is_maximized || state.is_fullscreen {
                    Tiling::tiled()
                } else {
                    tiling
                };

                Chrome {
                    os,
                    left_inset: 0.0,
                    right_controls,
                    is_maximized: state.is_maximized,
                    native_hit_test_controls: false,
                    client_drag: true,
                    supports_window_menu: state.controls.window_menu,
                    supports_maximize: state.controls.maximize,
                    draw_resize_edges: true,
                    tiling: effective_tiling,
                    client_inset: CSD_SHADOW_SIZE,
                }
            }
        },
    }
}

/// Reads the live [`Window`] state and computes [`Chrome`] for the current OS.
pub fn chrome_for_window(window: &Window) -> Chrome {
    chrome_for(Os::current(), WindowDecorationState::from_window(window))
}

/// Per-OS `TitlebarOptions` for [`WindowOptions`].
pub fn titlebar_options(os: Os) -> TitlebarOptions {
    match os {
        Os::Mac => TitlebarOptions {
            title: Some("Zyplus".into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(MAC_TRAFFIC_LIGHT_X), px(MAC_TRAFFIC_LIGHT_Y))),
        },
        Os::Windows => TitlebarOptions {
            title: Some("Zyplus".into()),
            appears_transparent: true,
            traffic_light_position: None,
        },
        Os::Linux => TitlebarOptions {
            title: Some("Zyplus".into()),
            appears_transparent: false,
            traffic_light_position: None,
        },
    }
}

/// Initial [`WindowDecorations`] request on Linux.
///
/// Defaults to [`WindowDecorations::Server`] (so X11 window managers, KDE, and
/// wlroots compositors draw their native titlebar). On GNOME/Wayland, Mutter
/// does not advertise `zxdg_decoration_manager_v1`; in gpui 0.2.2
/// (`platform/linux/wayland/window.rs`), calling `request_decorations(Server)`
/// without a decoration manager still overwrites `state.decorations = Server`
/// and never receives a configure event to flip it back to `Client`. Requesting
/// [`WindowDecorations::Client`] when running under GNOME on Wayland avoids
/// that bug so CSD chrome is drawn.
pub fn preferred_linux_decorations(
    wayland_display: Option<&str>,
    xdg_current_desktop: Option<&str>,
) -> WindowDecorations {
    let on_wayland = wayland_display.is_some_and(|v| !v.trim().is_empty());
    let is_gnome = xdg_current_desktop.is_some_and(|desktop| {
        desktop
            .split(':')
            .any(|part| part.trim().eq_ignore_ascii_case("gnome"))
    });
    if on_wayland && is_gnome {
        WindowDecorations::Client
    } else {
        WindowDecorations::Server
    }
}

/// Pure [`WindowOptions`] builder with explicit environment inputs for testing.
pub fn window_options_with_env(
    os: Os,
    bounds: Bounds<Pixels>,
    wayland_display: Option<&str>,
    xdg_current_desktop: Option<&str>,
) -> WindowOptions {
    let window_decorations = match os {
        Os::Linux => Some(preferred_linux_decorations(
            wayland_display,
            xdg_current_desktop,
        )),
        Os::Mac | Os::Windows => None,
    };
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(titlebar_options(os)),
        app_id: Some(APP_ID.to_string()),
        window_min_size: Some(size(px(MIN_WINDOW_WIDTH), px(MIN_WINDOW_HEIGHT))),
        window_decorations,
        ..Default::default()
    }
}

/// Builds [`WindowOptions`] for the main window on `os`.
pub fn main_window_options(os: Os, bounds: Bounds<Pixels>) -> WindowOptions {
    let wayland_display = (os == Os::Linux)
        .then(|| std::env::var("WAYLAND_DISPLAY").ok())
        .flatten();
    let xdg_current_desktop = (os == Os::Linux)
        .then(|| std::env::var("XDG_CURRENT_DESKTOP").ok())
        .flatten();
    window_options_with_env(
        os,
        bounds,
        wayland_display.as_deref(),
        xdg_current_desktop.as_deref(),
    )
}

/// Windows and Linux quit when the last window closes; macOS stays running.
pub const fn should_quit_when_all_windows_closed(os: Os) -> bool {
    !matches!(os, Os::Mac)
}

/// Only macOS implements app hiding (`cx.hide()` / `cx.hide_other_apps()`).
/// On Windows, gpui 0.2.2's `hide_other_apps()` calls `unimplemented!()`.
pub const fn supports_app_hide(os: Os) -> bool {
    matches!(os, Os::Mac)
}

/// Determines which outer resize edge or corner `pos` lies in, ignoring any
/// edge that is currently tiled (or all edges when maximized/fullscreen).
pub fn resize_edge(
    pos: Point<Pixels>,
    border: Pixels,
    size: Size<Pixels>,
    tiling: Tiling,
) -> Option<ResizeEdge> {
    if size.width <= px(0.0) || size.height <= px(0.0) || border <= px(0.0) {
        return None;
    }
    let top = !tiling.top && pos.y < border;
    let bottom = !tiling.bottom && pos.y > size.height - border;
    let left = !tiling.left && pos.x < border;
    let right = !tiling.right && pos.x > size.width - border;
    match (top, bottom, left, right) {
        (true, _, true, _) => Some(ResizeEdge::TopLeft),
        (true, _, _, true) => Some(ResizeEdge::TopRight),
        (true, _, _, _) => Some(ResizeEdge::Top),
        (_, true, true, _) => Some(ResizeEdge::BottomLeft),
        (_, true, _, true) => Some(ResizeEdge::BottomRight),
        (_, true, _, _) => Some(ResizeEdge::Bottom),
        (_, _, true, _) => Some(ResizeEdge::Left),
        (_, _, _, true) => Some(ResizeEdge::Right),
        _ => None,
    }
}

/// Cursor style corresponding to a client-side resize edge.
pub const fn resize_cursor(edge: ResizeEdge) -> CursorStyle {
    match edge {
        ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
        ResizeEdge::Left | ResizeEdge::Right => CursorStyle::ResizeLeftRight,
        ResizeEdge::TopLeft | ResizeEdge::BottomRight => CursorStyle::ResizeUpLeftDownRight,
        ResizeEdge::TopRight | ResizeEdge::BottomLeft => CursorStyle::ResizeUpRightDownLeft,
    }
}

/// Synchronises GPUI's client inset with the active chrome (Linux CSD shadow
/// margin). No-op on macOS, Windows, and Linux server-side decorations unless
/// clearing a previous inset.
pub fn apply_client_inset(window: &mut Window, chrome: &Chrome) {
    if chrome.client_inset > 0.0 {
        window.set_client_inset(px(chrome.client_inset));
    } else if window.client_inset().is_some_and(|v| v != px(0.0)) {
        window.set_client_inset(px(0.0));
    }
}

/// Marks a non-interactive top-bar region as a window drag surface.
///
/// - macOS / Windows: sets [`WindowControlArea::Drag`] (hit-tested as
///   `HTCAPTION` on Windows for native drag, Aero Snap, double-click maximize,
///   and system menu).
/// - Linux CSD: binds left-drag (`start_window_move`), double-click maximize
///   (`zoom_window`), and right-click window menu (`show_window_menu`).
/// - Linux SSD: returns `el` untouched (the WM titlebar owns dragging).
pub fn drag_area(el: Div, chrome: &Chrome) -> Div {
    let el = if chrome.os == Os::Mac || chrome.native_hit_test_controls {
        el.window_control_area(WindowControlArea::Drag)
    } else {
        el
    };
    // Windows turns a press on a Drag area into WM_NCLBUTTONDOWN, and gpui first
    // dispatches that press as an ordinary MouseDown to everything under the
    // pointer. If any of it calls `stop_propagation` or `prevent_default` the press
    // counts as handled and the move never starts. The root view tracks focus, and
    // gpui's focus handler calls `prevent_default`, so without this the window
    // could not be dragged at all. Occluding hides everything beneath the area.
    let el = if drag_area_occludes(chrome) {
        el.occlude()
    } else {
        el
    };
    if !chrome.client_drag {
        return el;
    }
    let can_maximize = chrome.supports_maximize;
    let el = el.on_mouse_down(MouseButton::Left, move |event, window, _| {
        if event.click_count >= 2 && can_maximize {
            window.zoom_window();
        } else {
            window.start_window_move();
        }
    });
    if chrome.supports_window_menu {
        el.on_mouse_down(MouseButton::Right, |event, window, _| {
            window.show_window_menu(event.position);
        })
    } else {
        el
    }
}

/// Whether a drag area must hide the elements beneath it from mouse events
/// (see [`drag_area`]). Only Windows routes the press through the native caption
/// hit test; macOS and Linux client-side moves do not need it.
pub fn drag_area_occludes(chrome: &Chrome) -> bool {
    chrome.native_hit_test_controls
}

/// Renders the top-right window caption buttons (minimise / maximise-restore /
/// close) when required by `chrome`. Returns `None` on macOS and Linux
/// server-side decorations.
///
/// Must be placed as a **sibling** (not a child) of any drag area so Windows
/// `WM_NCHITTEST` never matches `HTCAPTION` ahead of `HTMINBUTTON` /
/// `HTMAXBUTTON` / `HTCLOSE`.
pub fn window_controls(
    chrome: &Chrome,
    theme: &Theme,
    id_prefix: &'static str,
    on_close: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> Option<Div> {
    if chrome.right_controls.is_empty() {
        return None;
    }
    let width = chrome.control_button_width();
    let is_maximized = chrome.is_maximized;
    let native_hit_test = chrome.native_hit_test_controls;
    let fg = theme.fg;
    let hover = theme.hover;
    let active_bg = theme.border;
    let close_hover = theme.danger;
    let close_active = darken(theme.danger, 0.85);
    let mut on_close = Some(on_close);

    let mut row = div()
        .flex()
        .flex_row()
        .items_center()
        .h_full()
        .flex_shrink_0();

    for &control in &chrome.right_controls {
        let btn_id = SharedString::from(format!(
            "{id_prefix}-win-{}",
            match control {
                Control::Minimize => "min",
                Control::Maximize => "max",
                Control::Close => "close",
            }
        ));
        let close_group = SharedString::from(format!("{id_prefix}-win-close-grp"));

        let base = div()
            .id(btn_id)
            .w(px(width))
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .flex_shrink_0()
            .occlude();

        let styled = match control {
            Control::Minimize => base
                .hover(move |s| s.bg(hover))
                .active(move |s| s.bg(active_bg))
                .child(minimize_glyph(fg)),
            Control::Maximize => base
                .hover(move |s| s.bg(hover))
                .active(move |s| s.bg(active_bg))
                .child(maximize_glyph(fg, is_maximized)),
            Control::Close => base
                .group(close_group.clone())
                .hover(move |s| s.bg(close_hover))
                .active(move |s| s.bg(close_active))
                .child(
                    app_icon::icon("cancel01", 14.0)
                        .text_color(fg)
                        .group_hover(close_group, |s| s.text_color(rgb(0xffffff))),
                ),
        };

        let wired = if native_hit_test {
            // Windows handles clicks on HTMINBUTTON / HTMAXBUTTON / HTCLOSE in
            // `WM_NCLBUTTONUP` (including maximize/restore toggle, Win11 Snap
            // Layouts on hover, and WM_CLOSE -> on_window_should_close).
            let area = match control {
                Control::Minimize => WindowControlArea::Min,
                Control::Maximize => WindowControlArea::Max,
                Control::Close => WindowControlArea::Close,
            };
            styled.window_control_area(area)
        } else {
            let clickable = styled
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
            match control {
                Control::Minimize => clickable.on_click(|_, window, _| window.minimize_window()),
                Control::Maximize => clickable.on_click(|_, window, _| window.zoom_window()),
                Control::Close => {
                    let close_fn = on_close.take();
                    clickable.on_click(move |_, window, cx| {
                        if let Some(f) = &close_fn {
                            f(window, cx);
                        }
                    })
                }
            }
        };

        row = row.child(wired);
    }

    Some(row)
}

/// Wraps the root shell element with Linux client-side resize edges, drop
/// shadow, border, and rounded corners when `chrome.draw_resize_edges` is true.
/// Returns `content` untouched on macOS, Windows, and Linux server-side
/// decorations.
pub fn decorate_window(content: AnyElement, chrome: &Chrome, theme: &Theme) -> AnyElement {
    if !chrome.draw_resize_edges {
        return content;
    }

    let shadow_size = px(CSD_SHADOW_SIZE);
    let rounding = px(CSD_CORNER_RADIUS);
    let border_size = px(CSD_BORDER_WIDTH);
    let tiling = chrome.tiling;
    let border_color = theme.border;

    div()
        .id("window-csd-backdrop")
        .size_full()
        .bg(gpui::transparent_black())
        .child(
            canvas(
                |bounds, window, _cx| window.insert_hitbox(bounds, HitboxBehavior::Normal),
                move |bounds, hitbox, window, _cx| {
                    let local = window.mouse_position() - bounds.origin;
                    let Some(edge) = resize_edge(local, shadow_size, bounds.size, tiling) else {
                        return;
                    };
                    window.set_cursor_style(resize_cursor(edge), &hitbox);
                },
            )
            .size_full()
            .absolute(),
        )
        .when(!tiling.top, |el| el.pt(shadow_size))
        .when(!tiling.bottom, |el| el.pb(shadow_size))
        .when(!tiling.left, |el| el.pl(shadow_size))
        .when(!tiling.right, |el| el.pr(shadow_size))
        .on_mouse_move(|_e, window, _cx| window.refresh())
        .on_mouse_down(MouseButton::Left, move |e, window, _cx| {
            let size = window.viewport_size();
            if let Some(edge) = resize_edge(e.position, shadow_size, size, tiling) {
                window.start_window_resize(edge);
            }
        })
        .child(
            div()
                .cursor(CursorStyle::Arrow)
                .size_full()
                .overflow_hidden()
                .border_color(border_color)
                .when(!(tiling.top || tiling.left), |el| el.rounded_tl(rounding))
                .when(!(tiling.top || tiling.right), |el| el.rounded_tr(rounding))
                .when(!(tiling.bottom || tiling.left), |el| {
                    el.rounded_bl(rounding)
                })
                .when(!(tiling.bottom || tiling.right), |el| {
                    el.rounded_br(rounding)
                })
                .when(!tiling.top, |el| el.border_t(border_size))
                .when(!tiling.bottom, |el| el.border_b(border_size))
                .when(!tiling.left, |el| el.border_l(border_size))
                .when(!tiling.right, |el| el.border_r(border_size))
                .when(!tiling.is_tiled(), |el| {
                    el.shadow(vec![gpui::BoxShadow {
                        color: Hsla {
                            h: 0.0,
                            s: 0.0,
                            l: 0.0,
                            a: 0.35,
                        },
                        blur_radius: shadow_size / 2.0,
                        spread_radius: px(0.0),
                        offset: point(px(0.0), px(0.0)),
                    }])
                })
                .on_mouse_move(|_e, _, cx| {
                    cx.stop_propagation();
                })
                .child(content),
        )
        .into_any_element()
}

fn minimize_glyph(color: Rgba) -> Div {
    div().w(px(10.0)).h(px(1.0)).bg(color)
}

fn maximize_glyph(color: Rgba, is_maximized: bool) -> Div {
    if is_maximized {
        // Overlapping double-square restore icon (10x10 bounding box).
        div()
            .w(px(10.0))
            .h(px(10.0))
            .relative()
            .child(
                div()
                    .absolute()
                    .top_0()
                    .right_0()
                    .w(px(8.0))
                    .h(px(8.0))
                    .border_t(px(1.0))
                    .border_r(px(1.0))
                    .border_color(color),
            )
            .child(
                div()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .w(px(8.0))
                    .h(px(8.0))
                    .border_1()
                    .border_color(color),
            )
    } else {
        div().w(px(10.0)).h(px(10.0)).border_1().border_color(color)
    }
}

fn darken(color: Rgba, factor: f32) -> Rgba {
    Rgba {
        r: (color.r * factor).clamp(0.0, 1.0),
        g: (color.g * factor).clamp(0.0, 1.0),
        b: (color.b * factor).clamp(0.0, 1.0),
        a: color.a,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_chrome_is_unchanged_regardless_of_decoration_state() {
        for decorations in [
            Decorations::Server,
            Decorations::Client {
                tiling: Tiling::default(),
            },
        ] {
            let chrome = chrome_for(
                Os::Mac,
                WindowDecorationState {
                    decorations,
                    ..Default::default()
                },
            );
            assert_eq!(chrome.left_inset, 84.0);
            assert_eq!(chrome.sidebar_left_pad(), 84.0);
            assert_eq!(chrome.collapsed_tab_bar_left_pad(), 84.0);
            assert_eq!(chrome.fullscreen_left_pad(), 84.0);
            assert!(chrome.right_controls.is_empty());
            assert!(!chrome.shows_window_controls());
            assert!(!chrome.native_hit_test_controls);
            assert!(!chrome.client_drag);
            assert!(!chrome.draw_resize_edges);
            assert_eq!(chrome.client_inset, 0.0);
        }
    }

    #[test]
    fn only_windows_drag_areas_hide_the_elements_beneath_them() {
        // Windows dispatches the caption press as a MouseDown first; the root's
        // focus handler would `prevent_default` it and cancel the window move.
        let state = WindowDecorationState::default();
        assert!(drag_area_occludes(&chrome_for(Os::Windows, state)));
        assert!(!drag_area_occludes(&chrome_for(Os::Mac, state)));
        assert!(!drag_area_occludes(&chrome_for(Os::Linux, state)));
    }

    #[test]
    fn windows_chrome_draws_native_hit_tested_caption_controls_without_left_inset() {
        let windowed = chrome_for(Os::Windows, WindowDecorationState::default());
        assert_eq!(windowed.left_inset, 0.0);
        assert_eq!(windowed.sidebar_left_pad(), SIDEBAR_DEFAULT_LEFT_PAD);
        assert_eq!(
            windowed.collapsed_tab_bar_left_pad(),
            TAB_BAR_DEFAULT_LEFT_PAD
        );
        assert_eq!(windowed.fullscreen_left_pad(), FULLSCREEN_DEFAULT_LEFT_PAD);
        assert_eq!(
            windowed.right_controls,
            vec![Control::Minimize, Control::Maximize, Control::Close]
        );
        assert!(windowed.shows_window_controls());
        assert_eq!(windowed.control_button_width(), WIN_CONTROL_WIDTH);
        assert_eq!(windowed.right_controls_width(), 3.0 * WIN_CONTROL_WIDTH);
        assert!(windowed.native_hit_test_controls);
        assert!(!windowed.client_drag);
        assert!(!windowed.draw_resize_edges);
        assert!(!windowed.is_maximized);
        assert_eq!(windowed.client_inset, 0.0);

        let maximized = chrome_for(
            Os::Windows,
            WindowDecorationState {
                is_maximized: true,
                ..Default::default()
            },
        );
        assert!(maximized.is_maximized);
        assert_eq!(
            maximized.right_controls,
            vec![Control::Minimize, Control::Maximize, Control::Close]
        );
    }

    #[test]
    fn linux_server_decorations_draw_no_controls_and_no_insets() {
        let chrome = chrome_for(
            Os::Linux,
            WindowDecorationState {
                decorations: Decorations::Server,
                ..Default::default()
            },
        );
        assert_eq!(chrome.left_inset, 0.0);
        assert_eq!(chrome.sidebar_left_pad(), SIDEBAR_DEFAULT_LEFT_PAD);
        assert_eq!(
            chrome.collapsed_tab_bar_left_pad(),
            TAB_BAR_DEFAULT_LEFT_PAD
        );
        assert_eq!(chrome.fullscreen_left_pad(), FULLSCREEN_DEFAULT_LEFT_PAD);
        assert!(chrome.right_controls.is_empty());
        assert!(!chrome.shows_window_controls());
        assert!(!chrome.native_hit_test_controls);
        assert!(!chrome.client_drag);
        assert!(!chrome.supports_window_menu);
        assert!(!chrome.draw_resize_edges);
        assert_eq!(chrome.client_inset, 0.0);
    }

    #[test]
    fn linux_client_decorations_draw_csd_controls_drag_and_resize_edges() {
        let chrome = chrome_for(
            Os::Linux,
            WindowDecorationState {
                decorations: Decorations::Client {
                    tiling: Tiling::default(),
                },
                controls: WindowControls::default(),
                is_maximized: false,
                is_fullscreen: false,
            },
        );
        assert_eq!(chrome.left_inset, 0.0);
        assert_eq!(
            chrome.right_controls,
            vec![Control::Minimize, Control::Maximize, Control::Close]
        );
        assert!(chrome.shows_window_controls());
        assert_eq!(chrome.control_button_width(), LINUX_CONTROL_WIDTH);
        assert_eq!(chrome.right_controls_width(), 3.0 * LINUX_CONTROL_WIDTH);
        assert!(!chrome.native_hit_test_controls);
        assert!(chrome.client_drag);
        assert!(chrome.supports_window_menu);
        assert!(chrome.supports_maximize);
        assert!(chrome.draw_resize_edges);
        assert_eq!(chrome.tiling, Tiling::default());
        assert_eq!(chrome.client_inset, CSD_SHADOW_SIZE);
    }

    #[test]
    fn linux_csd_respects_compositor_window_controls_capabilities() {
        // e.g. GNOME default often disables minimize or maximize via wm_capabilities.
        let close_only = chrome_for(
            Os::Linux,
            WindowDecorationState {
                decorations: Decorations::Client {
                    tiling: Tiling::default(),
                },
                controls: WindowControls {
                    fullscreen: false,
                    maximize: false,
                    minimize: false,
                    window_menu: false,
                },
                is_maximized: false,
                is_fullscreen: false,
            },
        );
        assert_eq!(close_only.right_controls, vec![Control::Close]);
        assert!(!close_only.supports_window_menu);
        assert!(!close_only.supports_maximize);
        assert_eq!(close_only.right_controls_width(), LINUX_CONTROL_WIDTH);

        let max_and_close = chrome_for(
            Os::Linux,
            WindowDecorationState {
                decorations: Decorations::Client {
                    tiling: Tiling::default(),
                },
                controls: WindowControls {
                    fullscreen: true,
                    maximize: true,
                    minimize: false,
                    window_menu: true,
                },
                is_maximized: true,
                is_fullscreen: false,
            },
        );
        assert_eq!(
            max_and_close.right_controls,
            vec![Control::Maximize, Control::Close]
        );
        assert!(max_and_close.is_maximized);
        assert_eq!(max_and_close.tiling, Tiling::tiled());
    }

    #[test]
    fn window_options_configure_titlebar_app_id_and_decorations_per_os() {
        let bounds = Bounds::new(
            point(px(0.0), px(0.0)),
            size(px(DEFAULT_WINDOW_WIDTH), px(DEFAULT_WINDOW_HEIGHT)),
        );

        let mac = window_options_with_env(Os::Mac, bounds, None, None);
        let mac_tb = mac.titlebar.expect("mac titlebar");
        assert!(mac_tb.appears_transparent);
        assert_eq!(
            mac_tb.traffic_light_position,
            Some(point(px(16.0), px(15.0)))
        );
        assert_eq!(mac.app_id.as_deref(), Some(APP_ID));
        assert_eq!(mac.window_decorations, None);

        let win = window_options_with_env(Os::Windows, bounds, None, None);
        let win_tb = win.titlebar.expect("windows titlebar");
        assert!(win_tb.appears_transparent);
        assert_eq!(win_tb.traffic_light_position, None);
        assert_eq!(win.app_id.as_deref(), Some(APP_ID));
        assert_eq!(win.window_decorations, None);

        let kde_wayland =
            window_options_with_env(Os::Linux, bounds, Some("wayland-0"), Some("KDE"));
        let kde_tb = kde_wayland.titlebar.expect("linux titlebar");
        assert!(!kde_tb.appears_transparent);
        assert_eq!(kde_tb.traffic_light_position, None);
        assert_eq!(kde_wayland.app_id.as_deref(), Some(APP_ID));
        assert_eq!(
            kde_wayland.window_decorations,
            Some(WindowDecorations::Server)
        );

        let gnome_x11 = window_options_with_env(Os::Linux, bounds, None, Some("ubuntu:GNOME"));
        assert_eq!(
            gnome_x11.window_decorations,
            Some(WindowDecorations::Server)
        );

        let gnome_wayland =
            window_options_with_env(Os::Linux, bounds, Some("wayland-0"), Some("ubuntu:GNOME"));
        assert_eq!(
            gnome_wayland.window_decorations,
            Some(WindowDecorations::Client)
        );
    }

    #[test]
    fn lifecycle_policies_match_each_os() {
        assert!(!should_quit_when_all_windows_closed(Os::Mac));
        assert!(should_quit_when_all_windows_closed(Os::Windows));
        assert!(should_quit_when_all_windows_closed(Os::Linux));

        assert!(supports_app_hide(Os::Mac));
        assert!(!supports_app_hide(Os::Windows));
        assert!(!supports_app_hide(Os::Linux));
    }

    #[test]
    fn resize_edge_detects_all_edges_corners_interior_and_tiling() {
        let sz = size(px(800.0), px(600.0));
        let b = px(CSD_SHADOW_SIZE);
        let none = Tiling::default();

        assert_eq!(
            resize_edge(point(px(2.0), px(2.0)), b, sz, none),
            Some(ResizeEdge::TopLeft)
        );
        assert_eq!(
            resize_edge(point(px(795.0), px(2.0)), b, sz, none),
            Some(ResizeEdge::TopRight)
        );
        assert_eq!(
            resize_edge(point(px(400.0), px(3.0)), b, sz, none),
            Some(ResizeEdge::Top)
        );
        assert_eq!(
            resize_edge(point(px(3.0), px(595.0)), b, sz, none),
            Some(ResizeEdge::BottomLeft)
        );
        assert_eq!(
            resize_edge(point(px(795.0), px(595.0)), b, sz, none),
            Some(ResizeEdge::BottomRight)
        );
        assert_eq!(
            resize_edge(point(px(400.0), px(595.0)), b, sz, none),
            Some(ResizeEdge::Bottom)
        );
        assert_eq!(
            resize_edge(point(px(3.0), px(300.0)), b, sz, none),
            Some(ResizeEdge::Left)
        );
        assert_eq!(
            resize_edge(point(px(795.0), px(300.0)), b, sz, none),
            Some(ResizeEdge::Right)
        );
        assert_eq!(resize_edge(point(px(400.0), px(300.0)), b, sz, none), None);

        // Maximized / fully tiled window has no resize edges anywhere.
        let tiled = Tiling::tiled();
        assert_eq!(resize_edge(point(px(2.0), px(2.0)), b, sz, tiled), None);
        assert_eq!(resize_edge(point(px(795.0), px(595.0)), b, sz, tiled), None);

        // Half-tiled on left/top/bottom: only the untiled right edge resizes.
        let left_half = Tiling {
            top: true,
            bottom: true,
            left: true,
            right: false,
        };
        assert_eq!(resize_edge(point(px(2.0), px(2.0)), b, sz, left_half), None);
        assert_eq!(
            resize_edge(point(px(795.0), px(2.0)), b, sz, left_half),
            Some(ResizeEdge::Right)
        );

        // Degenerate zero size returns None.
        assert_eq!(
            resize_edge(point(px(0.0), px(0.0)), b, size(px(0.0), px(0.0)), none),
            None
        );
    }

    #[test]
    fn resize_cursor_maps_every_edge() {
        assert_eq!(resize_cursor(ResizeEdge::Top), CursorStyle::ResizeUpDown);
        assert_eq!(resize_cursor(ResizeEdge::Bottom), CursorStyle::ResizeUpDown);
        assert_eq!(
            resize_cursor(ResizeEdge::Left),
            CursorStyle::ResizeLeftRight
        );
        assert_eq!(
            resize_cursor(ResizeEdge::Right),
            CursorStyle::ResizeLeftRight
        );
        assert_eq!(
            resize_cursor(ResizeEdge::TopLeft),
            CursorStyle::ResizeUpLeftDownRight
        );
        assert_eq!(
            resize_cursor(ResizeEdge::BottomRight),
            CursorStyle::ResizeUpLeftDownRight
        );
        assert_eq!(
            resize_cursor(ResizeEdge::TopRight),
            CursorStyle::ResizeUpRightDownLeft
        );
        assert_eq!(
            resize_cursor(ResizeEdge::BottomLeft),
            CursorStyle::ResizeUpRightDownLeft
        );
    }
}
