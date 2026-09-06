use std::sync::Mutex;

use tauri::{Emitter, Manager};

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

/// Registers this app as the system handler for Markdown files.
/// macOS only — Windows/Linux have no API for this, the user picks it in OS settings.
#[tauri::command]
fn set_default_markdown_app(app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use core_foundation::base::TCFType;
        use core_foundation::string::{CFString, CFStringRef};

        #[link(name = "CoreServices", kind = "framework")]
        extern "C" {
            fn LSSetDefaultRoleHandlerForContentType(
                content_type: CFStringRef,
                role: u32,
                handler_bundle_id: CFStringRef,
            ) -> i32;
        }
        const K_LS_ROLES_EDITOR: u32 = 2;

        let bundle_id = CFString::new(&app.config().identifier);
        // .md maps to either UTI depending on macOS version; claim both.
        for uti in ["public.markdown", "net.daringfireball.markdown"] {
            let status = unsafe {
                LSSetDefaultRoleHandlerForContentType(
                    CFString::new(uti).as_concrete_TypeRef(),
                    K_LS_ROLES_EDITOR,
                    bundle_id.as_concrete_TypeRef(),
                )
            };
            if status != 0 {
                return Err(format!("LaunchServices refused {uti} (status {status})"));
            }
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Err("Set Zyplus as the default app for .md in your system settings.".into())
    }
}

/// Paths passed on the command line (Windows/Linux double-click).
fn cli_file_args() -> Vec<String> {
    std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with('-') && std::path::Path::new(a).is_file())
        .collect()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
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
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                if paths.is_empty() {
                    return;
                }
                // Queue first, then nudge: the webview may not be listening yet at launch,
                // so the frontend always drains the queue rather than trusting the payload.
                app.state::<PendingFiles>().0.lock().unwrap().extend(paths);
                let _ = app.emit("open-files", ());
            }
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            {
                let _ = (app, event);
            }
        });
}
