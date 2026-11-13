//! Webview memory and CPU tuning.
//!
//! Most of the app's memory is the OS webview's own processes, so the levers worth trying are
//! the webview's.
//!
//! **Linux (WebKitGTK):** JavaScriptCore's two top JIT tiers (DFG and FTL) are turned off. They
//! pay off for long-running compute, which an editor does not do, and they cost memory and CPU
//! to compile code that short interactive bursts never reuse. Measured on the real desktop app
//! (docs/performance.md): about 8 MB less at idle and 100-120 MB less after typing in a big note,
//! with about 17% less CPU per key. The baseline JIT stays on. A value already in the
//! environment (`JSC_useDFGJIT=true`, or `JSC_useJIT=false` for no JIT at all) is left alone.
//!
//! **Windows (WebView2):** it can be told the app is idle and should use less memory. Microsoft's
//! guidance is Low while inactive and Normal when active; the effect is best effort, and some of
//! the webview's memory may be swapped out meanwhile. Scripts keep running either way. This is
//! opt-in (`ZYPLUS_WEBVIEW_LOWMEM=1`) because it has not been measured on Windows yet: measure
//! with the variable unset and set, and make it the default only if it wins.
//!
//! macOS has neither lever.

/// JavaScriptCore options set for the web process, which reads them from its environment.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const JSC_OPTIONS: &[(&str, &str)] = &[("JSC_useDFGJIT", "false"), ("JSC_useFTLJIT", "false")];

/// The options to set, given which variables the environment already defines.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn jsc_options_to_set(is_defined: impl Fn(&str) -> bool) -> Vec<(&'static str, &'static str)> {
    JSC_OPTIONS
        .iter()
        .filter(|(name, _)| !is_defined(name))
        .copied()
        .collect()
}

/// Sets the JavaScriptCore options. Call it first thing in `run`, before the webview is created:
/// its processes inherit the environment of the app that starts them.
pub fn configure_javascript_engine() {
    #[cfg(target_os = "linux")]
    for (name, value) in jsc_options_to_set(|name| std::env::var_os(name).is_some()) {
        std::env::set_var(name, value);
    }
}

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
    use std::sync::mpsc::{channel, RecvTimeoutError};

    if !enabled(std::env::var("ZYPLUS_WEBVIEW_LOWMEM").ok().as_deref()) {
        return;
    }

    // One thread for the window's life, fed focus changes by the window's listener. The listener
    // owns only the sender, so it holds nothing alive; the thread ends when the window's
    // listeners are dropped and the channel closes.
    let (focus_changes, receiver) = channel::<bool>();
    let target = window.clone();
    std::thread::spawn(move || {
        let mut unfocused = false;
        loop {
            // Unfocused: wait for either focus to return or the idle time to pass. Otherwise
            // (focused, or already Low) there is nothing to time, so wait for the next change.
            let change = if unfocused {
                receiver.recv_timeout(IDLE_AFTER)
            } else {
                receiver.recv().map_err(|_| RecvTimeoutError::Disconnected)
            };
            match change {
                Ok(true) => {
                    unfocused = false;
                    set_memory_target(&target, false);
                }
                // Focus lost again (or still lost): the idle clock restarts.
                Ok(false) => unfocused = true,
                Err(RecvTimeoutError::Timeout) => {
                    unfocused = false;
                    set_memory_target(&target, true);
                }
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    });
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::Focused(focused) = event {
            let _ = focus_changes.send(*focused);
        }
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
    use super::{enabled, jsc_options_to_set};

    #[test]
    fn sets_the_jit_options_the_environment_does_not_define() {
        let all = jsc_options_to_set(|_| false);
        assert_eq!(
            all,
            [("JSC_useDFGJIT", "false"), ("JSC_useFTLJIT", "false")]
        );
        // A user's own choice, either way, is never overridden.
        let left = jsc_options_to_set(|name| name == "JSC_useDFGJIT");
        assert_eq!(left, [("JSC_useFTLJIT", "false")]);
        assert!(jsc_options_to_set(|_| true).is_empty());
    }

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
