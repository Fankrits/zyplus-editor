use std::sync::Mutex;

use tauri::Emitter;
use tauri::Manager;
use tauri_plugin_fs::FsExt;

/// Files the OS asked us to open before the webview was ready to listen.
#[derive(Default)]
struct PendingFiles(Mutex<Vec<String>>);

/// The document waiting to be printed, served to the print window by `zyprint://`,
/// plus the channel that window uses to report it has laid out and is printable.
#[derive(Default)]
struct PrintDoc {
    html: Mutex<String>,
    ready: Mutex<Option<tauri::async_runtime::Sender<()>>>,
}

const PRINT_WINDOW: &str = "print";

fn allow_paths_in_scope<R: tauri::Runtime, M: tauri::Manager<R>>(manager: &M, paths: &[String]) {
    let fs_scope = manager.fs_scope();
    for path in paths {
        let p = std::path::Path::new(path);
        if p.is_file() {
            let _ = fs_scope.allow_file(p);
        } else if p.is_dir() {
            let _ = fs_scope.allow_directory(p, true);
        }
    }
}

#[tauri::command]
fn take_pending_files(app: tauri::AppHandle, pending: tauri::State<PendingFiles>) -> Vec<String> {
    let files = std::mem::take(&mut *pending.0.lock().unwrap());
    allow_paths_in_scope(&app, &files);
    files
}

/// Starts a silent print job on a WKWebView, writing a paginated PDF to `path`.
///
/// Returns as soon as the job is *started* — see `export_pdf`, which waits for
/// the file. `runOperation()` cannot be used here: it blocks the main thread,
/// and WKWebView's pagination needs that thread free to talk to the web content
/// process, so it never terminates and writes pages until the disk fills.
/// The generic `NSView` print path does terminate, but a WKWebView renders out
/// of process, so it yields pages with no text on them at all.
#[cfg(target_os = "macos")]
fn start_print_to_pdf(
    webview: *mut std::ffi::c_void,
    ns_window: *mut std::ffi::c_void,
    path: &str,
) -> Result<(), String> {
    use objc2::runtime::{AnyObject, ProtocolObject};
    use objc2::AnyThread;
    use objc2_app_kit::{
        NSPrintInfo, NSPrintJobDisposition, NSPrintJobSavingURL, NSPrintSaveJob,
        NSPrintingPaginationMode, NSWindow,
    };
    use objc2_foundation::{NSMutableDictionary, NSString, NSURL};
    use objc2_web_kit::WKWebView;

    if webview.is_null() || ns_window.is_null() {
        return Err("no webview to print".into());
    }
    // Both are owned by the window this job runs against, which outlives the job.
    let webview: &WKWebView = unsafe { &*(webview as *const WKWebView) };
    let window: &NSWindow = unsafe { &*(ns_window as *const NSWindow) };

    let url = NSURL::fileURLWithPath(&NSString::from_str(path));
    let info = unsafe {
        // A save disposition plus a destination URL is what suppresses the panel.
        let dict = NSMutableDictionary::<NSString, AnyObject>::new();
        dict.setObject_forKey(
            url.as_ref() as &AnyObject,
            ProtocolObject::from_ref(NSPrintJobSavingURL),
        );
        dict.setObject_forKey(
            NSPrintSaveJob.as_ref() as &AnyObject,
            ProtocolObject::from_ref(NSPrintJobDisposition),
        );
        let info = NSPrintInfo::initWithDictionary(NSPrintInfo::alloc(), &dict);
        // The document's own `@page` rule is ignored on this path, so the
        // printable area is set here instead. 48pt = 2/3in.
        info.setTopMargin(MARGIN);
        info.setBottomMargin(MARGIN);
        info.setLeftMargin(MARGIN);
        info.setRightMargin(MARGIN);
        info.setVerticalPagination(NSPrintingPaginationMode::Automatic);
        info.setHorizontallyCentered(false);
        info.setVerticallyCentered(false);
        info
    };

    unsafe {
        let op = webview.printOperationWithPrintInfo(&info);
        op.setShowsPrintPanel(false);
        op.setShowsProgressPanel(false);
        // No delegate: completion is observed by watching the file appear.
        op.runOperationModalForWindow_delegate_didRunSelector_contextInfo(
            window,
            None,
            None,
            std::ptr::null_mut(),
        );
    }
    Ok(())
}

/// Printable-area inset, in points (72pt = 1in).
#[cfg(target_os = "macos")]
const MARGIN: f64 = 48.0;

/// Waits for an async print job to finish writing, by watching the file settle.
///
/// The job takes no completion delegate, so the file itself is the signal: it
/// is done once the size has stopped changing.
fn wait_for_pdf(path: &std::path::Path) -> Result<(), String> {
    use std::time::Duration;

    const POLL: Duration = Duration::from_millis(100);
    const SETTLED_FOR: u32 = 3; // consecutive unchanged polls
    const TIMEOUT: Duration = Duration::from_secs(120);

    let deadline = std::time::Instant::now() + TIMEOUT;
    let mut last = None;
    let mut stable = 0;

    while std::time::Instant::now() < deadline {
        std::thread::sleep(POLL);
        let size = std::fs::metadata(path).ok().map(|m| m.len());
        if size.is_some() && size == last {
            stable += 1;
            if stable >= SETTLED_FOR {
                return Ok(());
            }
        } else {
            stable = 0;
        }
        last = size;
    }
    Err("timed out waiting for the PDF to be written".into())
}

/// Writes the document to `path` as a PDF, with no print dialog.
///
/// The frontend renders the markdown to self-contained HTML and picks the path
/// with the same save dialog "Export as .md" uses. That HTML is loaded into an
/// offscreen webview and printed to disk once it has laid out. JS
/// `window.print()` is useless here: WKWebView routes it to the
/// `_webView:printFrame:` UI delegate, which wry does not implement.
#[tauri::command]
async fn export_pdf(app: tauri::AppHandle, html: String, path: String) -> Result<(), String> {
    let state = app.state::<PrintDoc>();
    *state.html.lock().unwrap() = html;

    // A previous export may still be around holding the stale document.
    if let Some(existing) = app.get_webview_window(PRINT_WINDOW) {
        existing.close().map_err(|e| e.to_string())?;
    }

    // The page signals it has laid out by fetching /ready off the same protocol
    // that served it; `on_page_load` fires too early to print against.
    let (ready_tx, mut ready_rx) = tauri::async_runtime::channel::<()>(1);
    *state.ready.lock().unwrap() = Some(ready_tx);

    let url = "zyprint://localhost/document.html"
        .parse()
        .map_err(|e| format!("{e}"))?;
    let window = tauri::WebviewWindowBuilder::new(
        &app,
        PRINT_WINDOW,
        tauri::WebviewUrl::CustomProtocol(url),
    )
    .title("Export as PDF")
    // Never shown, but it needs a real width to lay the document out against,
    // and on macOS an NSWindow is required for the print operation at all.
    .inner_size(816.0, 1056.0)
    .visible(false)
    .build()
    .map_err(|e| e.to_string())?;

    let printed = async {
        ready_rx
            .recv()
            .await
            .ok_or("the document never finished loading")?;

        let out = std::path::PathBuf::from(&path);
        let _ = std::fs::remove_file(&out);

        let (tx, mut rx) = tauri::async_runtime::channel::<Result<(), String>>(1);
        window
            .with_webview(move |platform| {
                #[cfg(target_os = "macos")]
                let result = start_print_to_pdf(platform.inner(), platform.ns_window(), &path);
                #[cfg(not(target_os = "macos"))]
                let result = {
                    let _ = (platform, &path);
                    Err("Exporting to PDF is only supported on macOS.".to_string())
                };
                let _ = tx.blocking_send(result);
            })
            .map_err(|e| e.to_string())?;
        rx.recv()
            .await
            .unwrap_or_else(|| Err("the export window went away before it printed".into()))?;

        tauri::async_runtime::spawn_blocking(move || wait_for_pdf(&out))
            .await
            .map_err(|e| e.to_string())?
    }
    .await;

    let _ = window.close();
    printed
}

/// LaunchServices bindings + the UTI a .md file carries (`net.daringfireball.markdown`).
#[cfg(target_os = "macos")]
mod launch_services {
    use core_foundation::base::TCFType;
    use core_foundation::string::{CFString, CFStringRef};

    #[link(name = "CoreServices", kind = "framework")]
    extern "C" {
        fn LSSetDefaultRoleHandlerForContentType(
            content_type: CFStringRef,
            role: u32,
            handler_bundle_id: CFStringRef,
        ) -> i32;
        fn LSCopyDefaultRoleHandlerForContentType(
            content_type: CFStringRef,
            role: u32,
        ) -> CFStringRef;
    }
    // LaunchServices role masks:
    // kLSRolesViewer = 0x00000002
    // kLSRolesEditor = 0x00000004
    // kLSRolesAll    = 0xFFFFFFFF
    const ROLE_ALL: u32 = 0xFFFF_FFFF;
    const ROLE_EDITOR: u32 = 0x0000_0004;
    const MARKDOWN_UTI: &str = "net.daringfireball.markdown";

    /// Claims Markdown UTI for this app bundle.
    pub fn claim_markdown(bundle_id: &str) -> Result<(), String> {
        let handler = CFString::new(bundle_id);
        let uti = CFString::new(MARKDOWN_UTI);

        // Set both editor and all-roles handler
        let _ = unsafe {
            LSSetDefaultRoleHandlerForContentType(
                uti.as_concrete_TypeRef(),
                ROLE_EDITOR,
                handler.as_concrete_TypeRef(),
            )
        };
        let status = unsafe {
            LSSetDefaultRoleHandlerForContentType(
                uti.as_concrete_TypeRef(),
                ROLE_ALL,
                handler.as_concrete_TypeRef(),
            )
        };

        if status == 0 && is_default(bundle_id) {
            return Ok(());
        }

        if status != 0 {
            Err(format!(
                "macOS LaunchServices failed to set {bundle_id} as default Markdown app (status {status})."
            ))
        } else {
            Err(format!(
                "macOS did not set {bundle_id} as default Markdown app. \
                 Please ensure Zyplus is installed in /Applications."
            ))
        }
    }

    pub fn is_default(bundle_id: &str) -> bool {
        let uti = CFString::new(MARKDOWN_UTI);
        for &role in &[ROLE_ALL, ROLE_EDITOR] {
            let current =
                unsafe { LSCopyDefaultRoleHandlerForContentType(uti.as_concrete_TypeRef(), role) };
            if !current.is_null() {
                let current_str = unsafe { CFString::wrap_under_create_rule(current).to_string() };
                if current_str.eq_ignore_ascii_case(bundle_id) {
                    return true;
                }
            }
        }
        false
    }
}

/// Registers this app as the system handler for Markdown files.
/// macOS only — Windows/Linux have no API for this, the user picks it in OS settings.
#[tauri::command]
fn set_default_markdown_app(app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        launch_services::claim_markdown(&app.config().identifier)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Err("Set Zyplus as the default app for .md in your system settings.".into())
    }
}

/// Whether Markdown already opens in this app, so the UI can say so up front.
#[tauri::command]
fn is_default_markdown_app(app: tauri::AppHandle) -> bool {
    #[cfg(target_os = "macos")]
    {
        launch_services::is_default(&app.config().identifier)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        false
    }
}

/// A canonicalized path as the rest of the app should see it.
///
/// `canonicalize` returns extended-length paths on Windows (`\\?\\C:\\notes\\a.md`,
/// or `\\\\?\\UNC\\server\\share` for a network share). They are valid, but they
/// become the tab's id and title and are matched against the fs scope, so the
/// prefix is stripped back off to the path the user actually typed or clicked.
fn display_path(path: std::path::PathBuf) -> String {
    let s = path.to_string_lossy().into_owned();
    // Only Windows produces these, and only Windows should have them removed:
    // on Unix `\\?\` is an ordinary (if bizarre) filename, not a prefix.
    #[cfg(windows)]
    return strip_verbatim_prefix(&s);
    #[cfg(not(windows))]
    s
}

/// `\\?\C:\notes` → `C:\notes`, `\\?\UNC\server\share` → `\\server\share`.
/// Compiled everywhere so the tests below cover it off a Windows runner too.
#[cfg_attr(not(windows), allow(dead_code))]
fn strip_verbatim_prefix(s: &str) -> String {
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    match s.strip_prefix(r"\\?\") {
        Some(rest) => rest.to_string(),
        None => s.to_string(),
    }
}

/// Canonicalizes a path argument relative to `cwd` if relative.
fn resolve_file_arg(arg: &std::path::Path, cwd: Option<&std::path::Path>) -> Option<String> {
    let candidate = if arg.is_absolute() {
        arg.to_path_buf()
    } else if let Some(cwd) = cwd {
        cwd.join(arg)
    } else {
        arg.to_path_buf()
    };
    if candidate.is_file() || candidate.is_dir() {
        Some(display_path(candidate.canonicalize().unwrap_or(candidate)))
    } else {
        None
    }
}

/// Paths passed on the command line. Resolves relative paths against the process working directory.
///
/// `args_os`, not `args`: the latter panics on an argument that is not valid
/// Unicode, which on Linux and macOS is any filename the filesystem happens to
/// hold — the app would die on launch rather than open the file.
fn cli_file_args() -> Vec<String> {
    let cwd = std::env::current_dir().ok();
    std::env::args_os()
        .skip(1)
        .filter(|a| !a.to_string_lossy().starts_with('-'))
        .filter_map(|a| resolve_file_arg(std::path::Path::new(&a), cwd.as_deref()))
        .collect()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            let cwd_path = std::path::Path::new(&cwd);
            let paths: Vec<String> = argv
                .iter()
                .skip(1)
                .filter(|a| !a.starts_with('-'))
                .filter_map(|a| resolve_file_arg(std::path::Path::new(a), Some(cwd_path)))
                .collect();

            if !paths.is_empty() {
                allow_paths_in_scope(app, &paths);
                app.state::<PendingFiles>().0.lock().unwrap().extend(paths);
                let _ = app.emit("open-files", ());
            }

            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(PendingFiles(Mutex::new(cli_file_args())))
        .manage(PrintDoc::default())
        .register_uri_scheme_protocol("zyprint", |ctx, request| {
            let state = ctx.app_handle().state::<PrintDoc>();
            if request.uri().path() == "/ready" {
                if let Some(tx) = state.ready.lock().unwrap().take() {
                    let _ = tx.blocking_send(());
                }
                return tauri::http::Response::builder()
                    .status(204)
                    .body(Vec::new())
                    .unwrap();
            }
            let html = state.html.lock().unwrap().clone();
            tauri::http::Response::builder()
                .header("Content-Type", "text/html; charset=utf-8")
                .body(html.into_bytes())
                .unwrap()
        })
        .invoke_handler(tauri::generate_handler![
            take_pending_files,
            set_default_markdown_app,
            is_default_markdown_app,
            export_pdf
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app, event| {
            // macOS delivers double-clicked/"Open With" files here, at any time.
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            if let tauri::RunEvent::Opened { urls } = event {
                let paths: Vec<String> = urls
                    .iter()
                    .filter_map(|u| u.to_file_path().ok())
                    .map(|p| display_path(p.canonicalize().unwrap_or(p)))
                    .collect();
                if paths.is_empty() {
                    return;
                }
                allow_paths_in_scope(app, &paths);
                // Queue first, then nudge: the webview may not be listening yet at launch,
                // so the frontend always drains the queue rather than trusting the payload.
                app.state::<PendingFiles>().0.lock().unwrap().extend(paths);

                let _ = app.emit("open-files", ());
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.unminimize();
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            {
                let _ = (app, event);
            }
        });
}

#[cfg(test)]
mod path_tests {
    use super::{resolve_file_arg, strip_verbatim_prefix};

    #[test]
    fn strips_windows_extended_length_prefixes() {
        assert_eq!(
            strip_verbatim_prefix(r"\\?\C:\notes\a.md"),
            r"C:\notes\a.md"
        );
        assert_eq!(
            strip_verbatim_prefix(r"\\?\UNC\server\share\a.md"),
            r"\\server\share\a.md"
        );
        // Anything without the prefix is passed through untouched.
        assert_eq!(strip_verbatim_prefix(r"C:\notes\a.md"), r"C:\notes\a.md");
        assert_eq!(strip_verbatim_prefix("/home/me/a.md"), "/home/me/a.md");
    }

    #[test]
    fn resolves_a_relative_arg_against_the_working_directory() {
        let dir = std::env::temp_dir().join("zyplus-resolve-arg-test");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("note.md");
        std::fs::write(&file, "# hi").unwrap();

        let resolved = resolve_file_arg(std::path::Path::new("note.md"), Some(&dir))
            .expect("a file that exists should resolve");
        assert!(resolved.ends_with("note.md"), "got {resolved}");
        assert!(
            !resolved.starts_with(r"\\?\"),
            "verbatim prefix leaked: {resolved}"
        );

        // A path that is not on disk is not a file to open.
        assert!(resolve_file_arg(std::path::Path::new("nope.md"), Some(&dir)).is_none());

        std::fs::remove_dir_all(&dir).ok();
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::launch_services::*;

    #[test]
    fn test_claim_markdown_and_is_default() {
        let bundle_id = "com.fankrits.zyplus-editor";
        let res = claim_markdown(bundle_id);
        assert!(res.is_ok(), "claim_markdown failed: {:?}", res.err());
        assert!(
            is_default(bundle_id),
            "is_default returned false after successful claim"
        );
    }
}
