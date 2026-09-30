use std::sync::Mutex;

use tauri::Emitter;
use tauri::Manager;

/// Files the OS asked us to open before the webview was ready to listen.
#[derive(Default)]
struct PendingFiles(Mutex<Vec<String>>);

/// The document waiting to be printed, served to the print window by `zyprint://`,
/// plus the channel that window uses to report it has laid out and is printable.
#[derive(Default)]
struct PrintDoc {
    html: Mutex<String>,
    ready: Mutex<Option<std::sync::mpsc::SyncSender<()>>>,
}

const PRINT_WINDOW: &str = "print";

/// How long the print window gets to load and lay out before the export gives up.
/// Without a limit, a page that never signalled left `export_pdf` waiting forever
/// and, with the frontend's one-export-at-a-time guard, every later export too.
const READY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// Queues files the OS handed us, then raises the main window.
///
/// Queue first, then nudge: at launch the webview may not be listening yet, so
/// the event carries no payload and the frontend always drains the queue.
/// The window is raised either way — a second launch carrying no files at all
/// is just the user asking for the app they already have running.
fn queue_open<R: tauri::Runtime, M: tauri::Manager<R> + Emitter<R>>(app: &M, paths: Vec<String>) {
    if !paths.is_empty() {
        app.state::<PendingFiles>().0.lock().unwrap().extend(paths);
        let _ = app.emit("open-files", ());
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[tauri::command]
fn take_pending_files(pending: tauri::State<PendingFiles>) -> Vec<String> {
    std::mem::take(&mut *pending.0.lock().unwrap())
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
    wait_for_pdf_within(path, Duration::from_millis(100), Duration::from_secs(120))
}

fn wait_for_pdf_within(
    path: &std::path::Path,
    poll: std::time::Duration,
    timeout: std::time::Duration,
) -> Result<(), String> {
    const SETTLED_FOR: u32 = 3; // consecutive unchanged polls

    let deadline = std::time::Instant::now() + timeout;
    let mut last = None;
    let mut stable = 0;

    while std::time::Instant::now() < deadline {
        std::thread::sleep(poll);
        let size = std::fs::metadata(path).ok().map(|m| m.len());
        // An empty file is the job having created its output and not yet written
        // to it (or a stale leftover), never a finished PDF.
        if size.is_some_and(|s| s > 0) && size == last {
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
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<()>(1);
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
        tauri::async_runtime::spawn_blocking(move || ready_rx.recv_timeout(READY_TIMEOUT))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|_| "the document never finished loading".to_string())?;

        let out = std::path::PathBuf::from(&path);
        // A leftover file at the target would look like a finished export, so a
        // removal that fails for any reason but "nothing there" fails the export.
        match std::fs::remove_file(&out) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("could not replace {}: {e}", out.display())),
        }

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

    if let Err(e) = window.close() {
        eprintln!("could not close the print window: {e}");
    }
    printed
}

/// LaunchServices bindings for claiming this app as the default handler for a
/// Uniform Type Identifier (UTI) — the OS's per-file-type content-type id
/// (e.g. `net.daringfireball.markdown`, `public.json`).
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

    /// Claims a content type (UTI) for this app bundle.
    pub fn claim(bundle_id: &str, uti: &str) -> Result<(), String> {
        let handler = CFString::new(bundle_id);
        let uti_ref = CFString::new(uti);

        // Set both editor and all-roles handler
        let _ = unsafe {
            LSSetDefaultRoleHandlerForContentType(
                uti_ref.as_concrete_TypeRef(),
                ROLE_EDITOR,
                handler.as_concrete_TypeRef(),
            )
        };
        let status = unsafe {
            LSSetDefaultRoleHandlerForContentType(
                uti_ref.as_concrete_TypeRef(),
                ROLE_ALL,
                handler.as_concrete_TypeRef(),
            )
        };

        if status == 0 && is_default(bundle_id, uti) {
            return Ok(());
        }

        if status != 0 {
            Err(format!(
                "macOS LaunchServices failed to set {bundle_id} as the default app for {uti} (status {status})."
            ))
        } else {
            Err(format!(
                "macOS did not set {bundle_id} as the default app for {uti}. \
                 Please ensure Zyplus is installed in /Applications."
            ))
        }
    }

    pub fn is_default(bundle_id: &str, uti: &str) -> bool {
        let uti_ref = CFString::new(uti);
        for &role in &[ROLE_ALL, ROLE_EDITOR] {
            let current = unsafe {
                LSCopyDefaultRoleHandlerForContentType(uti_ref.as_concrete_TypeRef(), role)
            };
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

/// Registers this app as the system handler for a Uniform Type Identifier (UTI),
/// e.g. `net.daringfireball.markdown` or `public.json`.
/// macOS only — Windows/Linux have no API for this, the user picks it in OS settings.
#[tauri::command]
fn set_default_app_for(app: tauri::AppHandle, uti: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        launch_services::claim(&app.config().identifier, &uti)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, uti);
        Err("Set Zyplus as the default app for this file type in your system settings.".into())
    }
}

/// Whether the given UTI already opens in this app, so the UI can say so up front.
#[tauri::command]
fn is_default_app_for(app: tauri::AppHandle, uti: String) -> bool {
    #[cfg(target_os = "macos")]
    {
        launch_services::is_default(&app.config().identifier, &uti)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, uti);
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
    // Shadowing rather than a cfg'd `return`, which reads as a needless one
    // on the very platform the branch exists for.
    #[cfg(windows)]
    let s = strip_verbatim_prefix(&s);
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
            queue_open(app, paths);
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
                    let _ = tx.try_send(());
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
            set_default_app_for,
            is_default_app_for,
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
                queue_open(app, paths);
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

    /// Deliberately not part of the default run.
    ///
    /// LaunchServices can only hand the Markdown UTI to a bundle id it knows
    /// about, so this fails on any machine where Zyplus is not installed — a
    /// clean CI runner included. It also has a real side effect: it reassigns
    /// whoever runs it a new default Markdown app, which `bun run check`
    /// should not do to a developer's Mac.
    ///
    /// Run it deliberately, on a machine with Zyplus in /Applications:
    ///   cargo test --manifest-path src-tauri/Cargo.toml -- --ignored
    #[test]
    #[ignore = "mutates the machine's LaunchServices bindings; needs Zyplus installed"]
    fn test_claim_markdown_and_is_default() {
        let bundle_id = "com.fankrits.zyplus-editor";
        let uti = "net.daringfireball.markdown";
        let res = claim(bundle_id, uti);
        assert!(res.is_ok(), "claim failed: {:?}", res.err());
        assert!(
            is_default(bundle_id, uti),
            "is_default returned false after successful claim"
        );
    }
}

#[cfg(test)]
mod pdf_wait_tests {
    use super::wait_for_pdf_within;
    use std::time::Duration;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("zyplus-pdf-wait-test");
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn an_empty_file_is_never_a_finished_pdf() {
        let path = temp("empty.pdf");
        std::fs::write(&path, b"").unwrap();
        let r = wait_for_pdf_within(&path, Duration::from_millis(5), Duration::from_millis(100));
        assert!(r.is_err(), "a 0-byte file must not count as written");
    }

    #[test]
    fn a_settled_file_is_done() {
        let path = temp("done.pdf");
        std::fs::write(&path, b"%PDF-1.4").unwrap();
        let r = wait_for_pdf_within(&path, Duration::from_millis(5), Duration::from_secs(5));
        assert!(r.is_ok());
    }

    #[test]
    fn a_missing_file_times_out() {
        let path = temp("never.pdf");
        let _ = std::fs::remove_file(&path);
        let r = wait_for_pdf_within(&path, Duration::from_millis(5), Duration::from_millis(60));
        assert!(r.is_err());
    }
}
