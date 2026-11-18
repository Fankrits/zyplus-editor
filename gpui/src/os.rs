//! OS integration for the desktop app: files from the command line and from
//! macOS "Open With" / double-click, single-instance forwarding, and claiming
//! Zyplus as the default app for a file type on macOS.
//!
//! # How macOS delivers "Open With" and double-clicked files
//!
//! LaunchServices calls `application:openURLs:` on the app delegate. gpui 0.2.2
//! turns that into `file://` URL strings (`platform/mac/platform.rs`, `open_urls`,
//! using `NSURL.absoluteString`) and hands them to the callback registered with
//! `App::on_open_urls` (`app.rs`). Register it inside the `Application::new().run`
//! closure, forward the strings through a channel, and drain that channel from a
//! foreground task:
//!
//! ```ignore
//! let (tx, rx) = std::sync::mpsc::channel::<Vec<String>>();
//! cx.on_open_urls(move |urls| {
//!     let _ = tx.send(os::urls_to_paths(&urls));
//! });
//! ```
//!
//! The callback runs on the main thread but gets no `App`, so it cannot touch
//! entities directly. Launch files still come from `cli_file_args()`, which is
//! taken once at startup and merged with the channel's contents.
//!
//! # Single instance
//!
//! `acquire_single_instance()` decides at startup whether this process is the
//! primary. Later launches forward their file arguments over a Unix socket and
//! exit. The primary's `Receiver` yields one `Vec<String>` per later launch; an
//! empty `Vec` means "that launch had no files, just bring the window forward".
//! Poll it from a background timer, for example every 100 ms with `try_recv`.
//!
//! Non-Unix platforms are always primary, so the receiver never yields.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};

/// Bundle identifier of the app; LaunchServices keys default-app claims on it.
#[allow(dead_code)] // Read by the default-app functions below, which the Settings UI calls.
pub const BUNDLE_ID: &str = "com.fankrits.zyplus-editor";

/// Uniform Type Identifiers for the file types Zyplus can claim as its default.
/// These match the web settings (`DEFAULT_APP_UTI` in `src/lib/fs.ts`).
#[allow(dead_code)] // Used by the Settings → Default app section (merged separately).
pub mod uti {
    pub const MARKDOWN: &str = "net.daringfireball.markdown";
    pub const CSV: &str = "public.comma-separated-values-text";
    pub const JSON: &str = "public.json";
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// A canonicalized path as the rest of the app should see it.
///
/// `canonicalize` returns extended-length paths on Windows (`\\?\C:\notes\a.md`,
/// or `\\?\UNC\server\share` for a network share). They are valid, but they
/// become the tab's id and title and are matched against the fs scope, so the
/// prefix is stripped back to the path the user actually typed or clicked.
pub fn display_path(path: PathBuf) -> String {
    let s = path.to_string_lossy().into_owned();
    // Only Windows produces these, and only Windows should have them removed:
    // on Unix `\\?\` is an ordinary (if bizarre) filename, not a prefix.
    if cfg!(windows) {
        strip_verbatim_prefix(&s)
    } else {
        s
    }
}

/// `\\?\C:\notes` becomes `C:\notes`, and `\\?\UNC\server\share` becomes
/// `\\server\share`. Compiled everywhere so the tests run off Windows too.
pub fn strip_verbatim_prefix(s: &str) -> String {
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    match s.strip_prefix(r"\\?\") {
        Some(rest) => rest.to_string(),
        None => s.to_string(),
    }
}

/// Resolves a command-line path to an absolute path string. Relative paths are
/// joined onto `cwd`. Nothing is checked on disk: the caller sorts folders,
/// files and missing paths, so a bad name is reported instead of dropped.
pub fn resolve_arg(arg: &Path, cwd: Option<&Path>) -> String {
    let candidate = match cwd {
        Some(cwd) if !arg.is_absolute() => cwd.join(arg),
        _ => arg.to_path_buf(),
    };
    display_path(candidate.canonicalize().unwrap_or(candidate))
}

/// Paths passed on the command line (files and folders, existing or not),
/// resolved against the working directory. Flags such as `-psn_0_12345` (sent
/// by Finder) are dropped.
///
/// `args_os`, not `args`: `args` panics on an argument that is not valid
/// Unicode. On Unix that is any filename the filesystem happens to hold, and
/// the app would die on launch instead of opening the file.
pub fn cli_file_args() -> Vec<String> {
    let cwd = std::env::current_dir().ok();
    std::env::args_os()
        .skip(1)
        .filter(|a| !a.to_string_lossy().starts_with('-'))
        .map(|a| resolve_arg(Path::new(&a), cwd.as_deref()))
        .collect()
}

// ---------------------------------------------------------------------------
// file:// URLs (macOS "Open With", via gpui's on_open_urls)
// ---------------------------------------------------------------------------

/// Turns the URL strings from `App::on_open_urls` into local paths.
///
/// Non-`file://` URLs are dropped. Each path is canonicalized when it exists and
/// is returned without an existence check, the same as the Tauri app's
/// `RunEvent::Opened` handling. The caller decides what to do with a file that
/// has since disappeared.
pub fn urls_to_paths(urls: &[String]) -> Vec<String> {
    urls.iter()
        .filter_map(|u| file_url_to_path(u))
        .map(|p| {
            let path = PathBuf::from(p);
            display_path(path.canonicalize().unwrap_or(path))
        })
        .collect()
}

/// Decodes one `file://` URL to a path, or `None` if it is not a local file URL.
///
/// Accepts `file:///path`, `file://localhost/path` and `file:/path`. A remote
/// host is rejected. Percent-escapes are decoded; an invalid escape such as
/// `%ZZ` stays literal, and a percent-encoded byte sequence that is not valid
/// UTF-8 is rejected.
pub fn file_url_to_path(url: &str) -> Option<String> {
    let scheme_len = "file:".len();
    if url.len() < scheme_len || !url[..scheme_len].eq_ignore_ascii_case("file:") {
        return None;
    }
    let rest = &url[scheme_len..];

    let path = match rest.strip_prefix("//") {
        Some(after) => {
            let (host, path) = match after.find('/') {
                Some(i) => (&after[..i], &after[i..]),
                None => (after, ""),
            };
            if !host.is_empty() && !host.eq_ignore_ascii_case("localhost") {
                return None;
            }
            path
        }
        None => rest,
    };
    if !path.starts_with('/') {
        return None;
    }

    let decoded = percent_decode(path)?;
    // Windows: `file:///C:/notes/a.md` leaves `/C:/notes/a.md`; the leading slash goes.
    if cfg!(windows) {
        let b = decoded.as_bytes();
        if b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':' {
            return Some(decoded[1..].to_string());
        }
    }
    Some(decoded)
}

/// Decodes `%XX` escapes. Returns `None` if the decoded bytes are not UTF-8.
fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let (hi, lo) = (bytes[i + 1], bytes[i + 2]);
            if hi.is_ascii_hexdigit() && lo.is_ascii_hexdigit() {
                out.push((hex_val(hi) << 4) | hex_val(lo));
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).ok()
}

fn hex_val(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        b'A'..=b'F' => b - b'A' + 10,
        _ => 0,
    }
}

// ---------------------------------------------------------------------------
// Single instance
// ---------------------------------------------------------------------------

/// Socket file name inside `<app data>/Zyplus/`. The leading dot keeps it out of
/// the sidebar, which skips dotfiles. Deliberately not the Tauri
/// build's name: the two builds speak different protocols.
const SOCKET_NAME: &str = ".gpui-single-instance.sock";

/// `sun_path` is 104 bytes on macOS and 108 on Linux, NUL included. Anything
/// longer cannot be bound, so the socket falls back to the temp directory.
const MAX_SOCKET_PATH: usize = 100;

/// Upper bound on one forwarded request. Real requests are a few paths.
const MAX_REQUEST_BYTES: u64 = 1 << 20;

/// How long a single connection may stall before it is dropped.
const IO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Result of `acquire_single_instance`.
pub enum SingleInstance {
    /// This process owns the app. Keep the `Primary` alive for as long as the
    /// app runs: dropping it removes the socket file.
    Primary(Primary),
    /// Another instance is running and has taken this launch's files. The
    /// caller should exit right away.
    Forwarded,
}

/// The primary instance's handle.
pub struct Primary {
    /// One message per launch forwarded from elsewhere, holding the absolute
    /// paths it was asked to open. An empty `Vec` means "just raise the window".
    pub requests: Receiver<Vec<String>>,
    _guard: Guard,
}

/// Cleans up on drop and keeps `requests` connected when nothing is listening.
/// Without the kept-alive sender, `try_recv` would return `Disconnected`
/// instead of `Empty`.
struct Guard {
    socket: Option<PathBuf>,
    _keepalive: Option<Sender<Vec<String>>>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(path) = self.socket.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Decides whether this process is the primary instance.
///
/// Forwarded launches send `cli_file_args()` to the primary and return
/// `Forwarded`. The primary starts listening and returns `Primary`. A socket
/// file left behind by a crash is detected (nothing accepts on it) and
/// replaced. If the socket cannot be bound at all, the app still starts as
/// primary without forwarding, and the reason goes to stderr.
pub fn acquire_single_instance() -> SingleInstance {
    acquire_at(&socket_path(), cli_file_args())
}

/// Where the single-instance socket lives: `<app data>/Zyplus/.gpui-single-instance.sock`.
/// Falls back to the temp directory when that path is too long to bind.
pub fn socket_path() -> PathBuf {
    let preferred = crate::paths::app_data_dir().map(|d| d.join("Zyplus").join(SOCKET_NAME));
    pick_socket_path(preferred, std::env::temp_dir())
}

fn pick_socket_path(preferred: Option<PathBuf>, temp_dir: PathBuf) -> PathBuf {
    match preferred {
        Some(p) if p.as_os_str().len() < MAX_SOCKET_PATH => p,
        _ => {
            let user: String = std::env::var("USER")
                .unwrap_or_else(|_| "user".into())
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .collect();
            temp_dir.join(format!("zyplus-gpui-{user}.sock"))
        }
    }
}

/// `acquire_single_instance` with an explicit socket path, so tests can use
/// a temporary one.
#[cfg(unix)]
fn acquire_at(socket: &Path, files: Vec<String>) -> SingleInstance {
    use std::os::unix::net::{UnixListener, UnixStream};

    if let Some(dir) = socket.parent() {
        let _ = std::fs::create_dir_all(dir);
    }

    // A live primary accepts the connection. Anything else (refused, or no
    // file at all) means the socket is stale or absent.
    if let Ok(stream) = UnixStream::connect(socket) {
        if let Err(e) = send_request(stream, &files) {
            eprintln!("single-instance: could not reach the running instance: {e}");
        }
        return SingleInstance::Forwarded;
    }

    match std::fs::remove_file(socket) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => eprintln!(
            "single-instance: could not remove {}: {e}",
            socket.display()
        ),
    }

    let (tx, rx) = mpsc::channel();
    match UnixListener::bind(socket) {
        Ok(listener) => {
            std::thread::Builder::new()
                .name("single-instance".into())
                .spawn(move || accept_loop(listener, tx))
                .expect("could not start the single-instance listener thread");
            SingleInstance::Primary(Primary {
                requests: rx,
                _guard: Guard {
                    socket: Some(socket.to_path_buf()),
                    _keepalive: None,
                },
            })
        }
        Err(e) => {
            // Two launches raced for the same stale socket. The app still starts,
            // but this instance will not forward later launches to the other one.
            eprintln!(
                "single-instance: could not listen on {}: {e}; not forwarding launches",
                socket.display()
            );
            SingleInstance::Primary(Primary {
                requests: rx,
                _guard: Guard {
                    socket: None,
                    _keepalive: Some(tx),
                },
            })
        }
    }
}

#[cfg(not(unix))]
fn acquire_at(_socket: &Path, _files: Vec<String>) -> SingleInstance {
    // No forwarding on this platform: every launch is its own primary.
    let (tx, rx) = mpsc::channel();
    SingleInstance::Primary(Primary {
        requests: rx,
        _guard: Guard {
            socket: None,
            _keepalive: Some(tx),
        },
    })
}

/// Wire format: one JSON array of path strings, newline-terminated.
#[cfg(unix)]
fn send_request(mut stream: std::os::unix::net::UnixStream, files: &[String]) -> io::Result<()> {
    // Best effort. Failing to set a timeout must not stop the write.
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    let mut line =
        serde_json::to_string(files).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    line.push('\n');
    stream.write_all(line.as_bytes())?;
    stream.flush()
}

#[cfg(unix)]
fn accept_loop(listener: std::os::unix::net::UnixListener, tx: Sender<Vec<String>>) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        if let Some(files) = read_request(stream) {
            if tx.send(files).is_err() {
                // The app has dropped its receiver; nothing left to serve.
                break;
            }
        }
    }
}

/// Reads one request. A malformed or stalled request is dropped silently so
/// it cannot take the listener down.
///
/// The read timeout is best effort. On macOS, `setsockopt` can fail on an
/// accepted socket whose client has already written and closed. The buffered
/// request is still readable in that case, so the error is ignored.
#[cfg(unix)]
fn read_request(stream: std::os::unix::net::UnixStream) -> Option<Vec<String>> {
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let mut line = String::new();
    BufReader::new(stream.take(MAX_REQUEST_BYTES))
        .read_line(&mut line)
        .ok()?;
    serde_json::from_str::<Vec<String>>(line.trim_end()).ok()
}

// ---------------------------------------------------------------------------
// macOS: default app claiming
// ---------------------------------------------------------------------------

// The default-app functions are called from the Settings modal, which lands in a
// separate change. Until it is merged they have no caller, so they are allowed
// to be unused here instead of being silenced for the whole module.

/// Makes Zyplus the default app for a UTI (for example `uti::MARKDOWN`), by
/// claiming both the Editor and All roles through LaunchServices.
///
/// On macOS, LaunchServices only accepts a handler it knows about, so this fails
/// unless Zyplus is installed (for example in /Applications). Other platforms
/// return `Err`: users pick the default in their system settings.
#[allow(dead_code)]
#[cfg(target_os = "macos")]
pub fn set_default_app_for(uti: &str) -> Result<(), String> {
    launch_services::claim(BUNDLE_ID, uti)
}

#[allow(dead_code)]
#[cfg(not(target_os = "macos"))]
pub fn set_default_app_for(_uti: &str) -> Result<(), String> {
    Err("Set Zyplus as the default app for this file type in your system settings.".into())
}

/// Whether Zyplus already opens this UTI by default. Always `false` off macOS.
#[allow(dead_code)]
#[cfg(target_os = "macos")]
pub fn is_default_app_for(uti: &str) -> bool {
    launch_services::is_default(BUNDLE_ID, uti)
}

#[allow(dead_code)]
#[cfg(not(target_os = "macos"))]
pub fn is_default_app_for(_uti: &str) -> bool {
    false
}

/// LaunchServices bindings. Ported from `src-tauri/src/lib.rs`.
#[allow(dead_code)]
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

    /// Claims a content type (UTI) for the app bundle `bundle_id`.
    pub fn claim(bundle_id: &str, uti: &str) -> Result<(), String> {
        let handler = CFString::new(bundle_id);
        let uti_ref = CFString::new(uti);

        // The Editor role result is ignored, as in the Tauri app: the All-roles
        // call below is the one whose status is reported.
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

    /// Whether `bundle_id` is the current handler for `uti` in the All or Editor role.
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod path_tests {
    use super::{file_url_to_path, resolve_arg, strip_verbatim_prefix, urls_to_paths};
    use std::path::{Path, PathBuf};

    /// A fresh directory per test, under the temp dir. Keep the names short:
    /// socket paths in the same tree are limited to 104 bytes.
    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("zy-os-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

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
        let dir = scratch_dir("resolve");
        let file = dir.join("note.md");
        std::fs::write(&file, "# hi").unwrap();

        let resolved = resolve_arg(Path::new("note.md"), Some(&dir));
        assert!(resolved.ends_with("note.md"), "got {resolved}");
        assert!(
            !resolved.starts_with(r"\\?\"),
            "verbatim prefix leaked: {resolved}"
        );

        // A path that is not on disk still comes back, so it can be reported
        // as missing rather than vanishing.
        let missing = resolve_arg(Path::new("nope.md"), Some(&dir));
        assert!(missing.ends_with("nope.md"), "got {missing}");
        // A folder resolves too: it is added as a project root.
        let folder = resolve_arg(Path::new("."), Some(&dir));
        assert_eq!(
            std::fs::canonicalize(&folder).ok(),
            std::fs::canonicalize(&dir).ok()
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn decodes_file_urls_with_percent_escapes() {
        assert_eq!(
            file_url_to_path("file:///Users/me/My%20Notes/a%C3%A9.md").as_deref(),
            Some("/Users/me/My Notes/aé.md")
        );
        assert_eq!(
            file_url_to_path("file://localhost/tmp/x.md").as_deref(),
            Some("/tmp/x.md")
        );
        assert_eq!(
            file_url_to_path("file:/tmp/x.md").as_deref(),
            Some("/tmp/x.md")
        );
        assert_eq!(
            file_url_to_path("FILE:///tmp/x.md").as_deref(),
            Some("/tmp/x.md")
        );
        // A percent-encoded slash decodes to a real one.
        assert_eq!(
            file_url_to_path("file:///a%2Fb.md").as_deref(),
            Some("/a/b.md")
        );
        // An escape that is not hex stays literal.
        assert_eq!(
            file_url_to_path("file:///bad%ZZ").as_deref(),
            Some("/bad%ZZ")
        );
        // Bytes that are not UTF-8 cannot be a path string here.
        assert_eq!(file_url_to_path("file:///bad%E9.md"), None);
    }

    #[test]
    fn rejects_non_local_urls() {
        assert_eq!(file_url_to_path("file://otherhost/x.md"), None);
        assert_eq!(file_url_to_path("https://example.com/a.md"), None);
        assert_eq!(file_url_to_path("zyplus://open?x=1"), None);
        assert_eq!(file_url_to_path("file://localhost"), None);
        assert_eq!(file_url_to_path("file:relative/x.md"), None);
        assert_eq!(file_url_to_path(""), None);
    }

    #[test]
    fn urls_to_paths_keeps_only_file_urls_and_canonicalizes_existing_ones() {
        let dir = scratch_dir("urls");
        let file = dir.join("My Note.md");
        std::fs::write(&file, "# hi").unwrap();
        let url = format!("file://{}", file.to_string_lossy().replace(' ', "%20"));

        let urls = vec![
            url,
            "https://example.com/".to_string(),
            "file:///definitely/not/here%20.md".to_string(),
        ];
        let paths = urls_to_paths(&urls);
        assert_eq!(paths.len(), 2, "got {paths:?}");
        assert!(paths[0].ends_with("My Note.md"), "got {}", paths[0]);
        assert_eq!(paths[1], "/definitely/not/here .md");

        std::fs::remove_dir_all(&dir).ok();
    }
}

#[cfg(all(test, unix))]
mod single_instance_tests {
    use super::{acquire_at, pick_socket_path, SingleInstance};
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    fn socket_in(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("zy-si-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("s.sock");
        assert!(
            sock.as_os_str().len() < 100,
            "test socket path too long for sun_path: {}",
            sock.display()
        );
        sock
    }

    fn expect_primary(s: SingleInstance) -> super::Primary {
        match s {
            SingleInstance::Primary(p) => p,
            SingleInstance::Forwarded => panic!("expected to be primary"),
        }
    }

    fn expect_forwarded(s: SingleInstance) {
        if let SingleInstance::Primary(_) = s {
            panic!("expected to forward, but became primary");
        }
    }

    #[test]
    fn later_launch_forwards_its_files_to_the_primary() {
        let sock = socket_in("fwd");
        let primary = expect_primary(acquire_at(&sock, vec![]));

        expect_forwarded(acquire_at(
            &sock,
            vec!["/notes/a.md".into(), "/notes/b.csv".into()],
        ));
        let got = primary
            .requests
            .recv_timeout(Duration::from_secs(5))
            .expect("the primary should receive the forwarded request");
        assert_eq!(
            got,
            vec!["/notes/a.md".to_string(), "/notes/b.csv".to_string()]
        );

        // A launch with no files still reaches the primary, so it can raise the window.
        expect_forwarded(acquire_at(&sock, vec![]));
        let got = primary
            .requests
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert!(got.is_empty());

        // Nothing pending: the receiver is Empty, not Disconnected.
        assert!(matches!(
            primary.requests.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));

        drop(primary);
        assert!(!sock.exists(), "dropping the primary removes its socket");
    }

    #[test]
    fn a_stale_socket_from_a_crashed_instance_is_replaced() {
        let sock = socket_in("stale");
        // Bind then drop: the file stays behind, and nothing accepts on it.
        drop(std::os::unix::net::UnixListener::bind(&sock).unwrap());
        assert!(sock.exists());

        let primary = expect_primary(acquire_at(&sock, vec![]));
        expect_forwarded(acquire_at(&sock, vec!["/x.md".into()]));
        assert_eq!(
            primary
                .requests
                .recv_timeout(Duration::from_secs(5))
                .unwrap(),
            vec!["/x.md".to_string()]
        );
    }

    #[test]
    fn a_regular_file_left_at_the_socket_path_is_replaced() {
        let sock = socket_in("plain");
        std::fs::write(&sock, b"not a socket").unwrap();

        let _primary = expect_primary(acquire_at(&sock, vec![]));
        // The leftover is gone and the listener is in its place.
        expect_forwarded(acquire_at(&sock, vec![]));
    }

    #[test]
    fn missing_parent_directories_are_created() {
        let base = socket_in("nested");
        let sock = base
            .parent()
            .unwrap()
            .join("deeper")
            .join("er")
            .join("s.sock");
        assert!(sock.as_os_str().len() < 100);
        let _primary = expect_primary(acquire_at(&sock, vec![]));
        assert!(sock.exists());
    }

    #[test]
    fn long_app_data_paths_fall_back_to_the_temp_dir() {
        let short = PathBuf::from("/Users/me/Library/Application Support/Zyplus/s.sock");
        assert_eq!(
            pick_socket_path(Some(short.clone()), PathBuf::from("/tmp")),
            short
        );

        let long = PathBuf::from(format!("/Users/{}/Zyplus/s.sock", "x".repeat(120)));
        let picked = pick_socket_path(Some(long), PathBuf::from("/tmp"));
        assert!(picked.starts_with("/tmp"), "got {}", picked.display());

        let none = pick_socket_path(None, PathBuf::from("/tmp"));
        assert!(none.starts_with(Path::new("/tmp")));
    }
}

#[cfg(all(test, target_os = "macos"))]
mod default_app_tests {
    use super::{is_default_app_for, launch_services, BUNDLE_ID};

    /// Reads only. A bogus UTI has no handler, so this is `false` on any machine.
    #[test]
    fn an_unknown_uti_has_no_default_app() {
        assert!(!is_default_app_for(
            "com.fankrits.zyplus-test.not-a-real-uti"
        ));
    }

    /// Deliberately not part of the default run.
    ///
    /// It reassigns the machine's default Markdown app, and LaunchServices only
    /// accepts a bundle it knows about (Zyplus in /Applications). Run it on a
    /// Mac where that is intended:
    ///   cargo test -- --ignored claim_markdown
    #[test]
    #[ignore = "mutates the machine's LaunchServices bindings; needs Zyplus installed"]
    fn claim_markdown_and_is_default() {
        let uti = super::uti::MARKDOWN;
        let res = launch_services::claim(BUNDLE_ID, uti);
        assert!(res.is_ok(), "claim failed: {:?}", res.err());
        assert!(
            is_default_app_for(uti),
            "is_default returned false after a successful claim"
        );
    }
}
