use std::sync::Mutex;

use tauri::{Emitter, Manager};

/// Files the OS asked us to open before the webview was ready to listen.
#[derive(Default)]
struct PendingFiles(Mutex<Vec<String>>);

#[tauri::command]
fn take_pending_files(pending: tauri::State<PendingFiles>) -> Vec<String> {
    std::mem::take(&mut *pending.0.lock().unwrap())
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
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(PendingFiles(Mutex::new(cli_file_args())))
        .invoke_handler(tauri::generate_handler![
            take_pending_files,
            set_default_markdown_app
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
