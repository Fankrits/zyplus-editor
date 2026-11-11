//! Webview memory tuning. Windows only, and off unless asked for.
//!
//! Most of the app's memory is the OS webview's own processes, so the levers worth trying are
//! the webview's. On Linux they were measured and turned out to do nothing (see
//! docs/performance.md), so nothing is applied there. WebView2 on Windows is different: it can
//! be told the app is idle and should use less memory. Microsoft's guidance is Low while
//! inactive and Normal when active; the effect is best effort, and some of the webview's memory
//! may be swapped out meanwhile. Scripts keep running either way.
//!
//! This is opt-in (`ZYPLUS_WEBVIEW_LOWMEM=1`) because it has not been measured on Windows yet.
//! Measure with the variable unset and set, and make it the default only if it wins.

/// Whether the opt-in is on for this value of `ZYPLUS_WEBVIEW_LOWMEM`.
#[cfg_attr(not(windows), allow(dead_code))]
fn enabled(setting: Option<&str>) -> bool {
    matches!(
        setting.map(|s| s.trim().to_ascii_lowercase()).as_deref(),
        Some("1" | "true" | "on" | "yes")
    )
}

/// A window that has stayed unfocused this long gets the Low target; focus restores Normal at
/// once. Switching between windows therefore never pays for it.
#[cfg(windows)]
const IDLE_AFTER: std::time::Duration = std::time::Duration::from_secs(30);

#[cfg(windows)]
pub fn apply(window: &tauri::WebviewWindow) {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    if !enabled(std::env::var("ZYPLUS_WEBVIEW_LOWMEM").ok().as_deref()) {
        return;
    }

    // Each focus change bumps this; a pending "go low" only fires if nothing changed since.
    let generation = Arc::new(AtomicU64::new(0));
    let target = window.clone();
    window.on_window_event(move |event| {
        let tauri::WindowEvent::Focused(focused) = event else {
            return;
        };
        let now = generation.fetch_add(1, Ordering::SeqCst) + 1;
        if *focused {
            set_memory_target(&target, false);
            return;
        }
        let (generation, target) = (generation.clone(), target.clone());
        std::thread::spawn(move || {
            std::thread::sleep(IDLE_AFTER);
            if generation.load(Ordering::SeqCst) == now {
                set_memory_target(&target, true);
            }
        });
    });
}

#[cfg(windows)]
fn set_memory_target(window: &tauri::WebviewWindow, low: bool) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2_19, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW,
        COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL,
    };
    use windows::core::Interface;

    let level = if low {
        COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW
    } else {
        COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL
    };
    let result = window.with_webview(move |platform| unsafe {
        // Older WebView2 runtimes lack ICoreWebView2_19: the cast fails and nothing changes.
        if let Ok(core) = platform.controller().CoreWebView2() {
            if let Ok(core) = core.cast::<ICoreWebView2_19>() {
                let _ = core.SetMemoryUsageTargetLevel(level);
            }
        }
    });
    if let Err(e) = result {
        eprintln!("could not set the webview memory target: {e}");
    }
}

#[cfg(not(windows))]
pub fn apply(_window: &tauri::WebviewWindow) {}

#[cfg(test)]
mod tests {
    use super::enabled;

    #[test]
    fn off_unless_asked_for() {
        assert!(!enabled(None));
        assert!(!enabled(Some("")));
        assert!(!enabled(Some("0")));
        assert!(!enabled(Some("off")));
    }

    #[test]
    fn on_for_the_usual_spellings() {
        for v in ["1", "true", "TRUE", " on ", "yes"] {
            assert!(enabled(Some(v)), "{v}");
        }
    }
}
