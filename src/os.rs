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
//! primary. Later launches forward their file arguments to the primary and
//! exit: over a Unix domain socket on macOS/Linux, and over an authenticated
//! loopback TCP listener on Windows. The primary's `Receiver` yields one
//! `Vec<String>` per later launch; an empty `Vec` means "that launch had no
//! files, just bring the window forward". Poll it from a background timer, for
//! example every 100 ms with `try_recv`.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::platform::Os;

/// Bundle identifier of the app; LaunchServices keys default-app claims on it.
#[allow(dead_code)] // Read by the default-app functions below, which the Settings UI calls.
pub const BUNDLE_ID: &str = "com.fankrits.zyplus-editor";

/// Uniform Type Identifiers for the file types Zyplus can claim as its default.
#[allow(dead_code)] // Used by the Settings → Default app section.
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
/// is returned without an existence check. The caller decides what to do with a
/// file that has since disappeared.
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
/// Accepts `file:///path`, `file://localhost/path` and `file:/path`. Remote
/// hosts — including Windows UNC URLs such as `file://server/share/a.md` — are
/// rejected on all platforms so external URLs cannot trigger unintended remote
/// or SMB lookups (Windows UNC paths are still supported when passed directly
/// on the command line). Percent-escapes are decoded; an invalid escape such as
/// `%ZZ` stays literal, and a percent-encoded byte sequence that is not valid
/// UTF-8 is rejected.
pub fn file_url_to_path(url: &str) -> Option<String> {
    file_url_to_path_for(url, Os::current())
}

fn file_url_to_path_for(url: &str, os: Os) -> Option<String> {
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
    if os == Os::Windows {
        let b = decoded.as_bytes();
        // Windows: `file:///C:/notes/a.md` (or `file:///C%3A/notes/a.md`) leaves
        // `/C:/notes/a.md`; strip the leading slash and normalize `/` to `\`.
        if b.len() >= 3
            && b[0] == b'/'
            && b[1].is_ascii_alphabetic()
            && b[2] == b':'
            && (b.len() == 3 || b[3] == b'/')
        {
            return Some(decoded[1..].replace('/', "\\"));
        }
        return Some(decoded.replace('/', "\\"));
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
/// the sidebar, which skips dotfiles, and uses a distinct name for the native
/// single-instance protocol.
const SOCKET_NAME: &str = ".gpui-single-instance.sock";

/// Port/token file name inside `<app data>/Zyplus/` for the loopback TCP
/// single-instance transport (used on Windows). Leading dot keeps it out of the
/// sidebar.
#[cfg_attr(unix, allow(dead_code))]
const PORT_FILE_NAME: &str = ".gpui-single-instance.port";

/// Short acknowledgement written by the TCP listener once a request passes
/// token verification and has been queued to the primary.
#[cfg_attr(unix, allow(dead_code))]
const TCP_ACK: &[u8] = b"ok\n";

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
    /// app runs: dropping it removes the socket or port file.
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
    cleanup_path: Option<PathBuf>,
    /// When set, `cleanup_path` is a TCP port file and is only removed if it
    /// still carries this primary's token.
    cleanup_token: Option<String>,
    _keepalive: Option<Sender<Vec<String>>>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(path) = self.cleanup_path.take() {
            match self.cleanup_token.take() {
                Some(token) => {
                    if read_port_file(&path)
                        .is_some_and(|ep| constant_time_eq(ep.token.as_bytes(), token.as_bytes()))
                    {
                        let _ = std::fs::remove_file(path);
                    }
                }
                None => {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
    }
}

/// Decides whether this process is the primary instance.
///
/// Forwarded launches send `cli_file_args()` to the primary and return
/// `Forwarded`. The primary starts listening and returns `Primary`. A stale
/// socket or port file left behind by a crash is detected and replaced. If the
/// listener cannot be started at all, the app still starts as primary without
/// forwarding, and the reason goes to stderr.
pub fn acquire_single_instance() -> SingleInstance {
    acquire_single_instance_with(cli_file_args())
}

#[cfg(unix)]
fn acquire_single_instance_with(files: Vec<String>) -> SingleInstance {
    acquire_at(&socket_path(), files)
}

#[cfg(not(unix))]
fn acquire_single_instance_with(files: Vec<String>) -> SingleInstance {
    acquire_tcp(&port_file_path(), files)
}

/// Where the single-instance socket lives: `<app data>/Zyplus/.gpui-single-instance.sock`.
/// Falls back to the temp directory when that path is too long to bind.
pub fn socket_path() -> PathBuf {
    let preferred = crate::paths::app_data_dir().map(|d| d.join("Zyplus").join(SOCKET_NAME));
    pick_socket_path(preferred, std::env::temp_dir())
}

fn sanitize_user(raw: Option<&str>) -> String {
    let clean: String = raw
        .unwrap_or("user")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    if clean.is_empty() {
        "user".into()
    } else {
        clean
    }
}

fn pick_socket_path(preferred: Option<PathBuf>, temp_dir: PathBuf) -> PathBuf {
    match preferred {
        Some(p) if p.as_os_str().len() < MAX_SOCKET_PATH => p,
        _ => {
            let user_env = std::env::var("USER").ok();
            let user = sanitize_user(user_env.as_deref());
            temp_dir.join(format!("zyplus-gpui-{user}.sock"))
        }
    }
}

/// Where the Windows single-instance port/token file lives:
/// `<app data>/Zyplus/.gpui-single-instance.port`, falling back to temp dir
/// when the app data directory is unavailable.
#[cfg_attr(unix, allow(dead_code))]
fn port_file_path() -> PathBuf {
    let preferred = crate::paths::app_data_dir().map(|d| d.join("Zyplus").join(PORT_FILE_NAME));
    let user = std::env::var("USERNAME")
        .ok()
        .or_else(|| std::env::var("USER").ok());
    pick_port_file_path(preferred, std::env::temp_dir(), user.as_deref())
}

#[cfg_attr(unix, allow(dead_code))]
fn pick_port_file_path(
    preferred: Option<PathBuf>,
    temp_dir: PathBuf,
    user: Option<&str>,
) -> PathBuf {
    match preferred {
        Some(p) => p,
        None => temp_dir.join(format!("zyplus-gpui-{}.port", sanitize_user(user))),
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
                    cleanup_path: Some(socket.to_path_buf()),
                    cleanup_token: None,
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
                    cleanup_path: None,
                    cleanup_token: None,
                    _keepalive: Some(tx),
                },
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Loopback TCP single-instance transport (used on Windows; compiled and tested everywhere)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct TcpEndpoint {
    port: u16,
    token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct TcpRequest {
    token: String,
    files: Vec<String>,
}

static TOKEN_SEQ: AtomicU64 = AtomicU64::new(0);

/// Generates an unpredictable 256-bit hex token using OS-seeded `RandomState`
/// plus process/time/sequence entropy (std only).
#[cfg_attr(unix, allow(dead_code))]
fn generate_token() -> String {
    let pid = std::process::id();
    let now_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let instant = Instant::now();
    let mut parts = [0u64; 4];
    for (idx, slot) in parts.iter_mut().enumerate() {
        let seq = TOKEN_SEQ.fetch_add(1, Ordering::Relaxed);
        let mut h = RandomState::new().build_hasher();
        h.write_u64(seq);
        h.write_usize(idx);
        h.write_u32(pid);
        h.write_u128(now_nanos);
        h.write_u128(instant.elapsed().as_nanos());
        *slot = h.finish();
    }
    format!(
        "{:016x}{:016x}{:016x}{:016x}",
        parts[0], parts[1], parts[2], parts[3]
    )
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn read_port_file(path: &Path) -> Option<TcpEndpoint> {
    let text = std::fs::read_to_string(path).ok()?;
    let ep: TcpEndpoint = serde_json::from_str(text.trim()).ok()?;
    if ep.port == 0 || ep.token.is_empty() {
        return None;
    }
    Some(ep)
}

#[cfg_attr(unix, allow(dead_code))]
fn write_port_file(path: &Path, endpoint: &TcpEndpoint) -> io::Result<()> {
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    std::fs::create_dir_all(dir)?;

    let body = serde_json::to_string(endpoint)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let seq = TOKEN_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(
        ".gpui-single-instance.{}.{seq}.tmp",
        std::process::id()
    ));

    let write_res = (|| -> io::Result<()> {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut file = opts.open(&tmp)?;
        file.write_all(body.as_bytes())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    })();

    if write_res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    write_res
}

/// Loopback TCP single-instance acquisition. Compiled on all platforms so unit
/// tests exercise every edge case here; used by `acquire_single_instance()` on
/// Windows.
#[cfg_attr(unix, allow(dead_code))]
fn acquire_tcp(port_file: &Path, files: Vec<String>) -> SingleInstance {
    if let Some(dir) = port_file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }

    // Try forwarding to an existing primary. Only treat it as live when the
    // peer accepts our token and replies with TCP_ACK; connection refused,
    // corrupt file, or an unrelated service reusing the port falls through and
    // lets this process take over.
    if let Some(endpoint) = read_port_file(port_file) {
        if tcp_send_request(&endpoint, &files).is_ok() {
            return SingleInstance::Forwarded;
        }
    }

    let (tx, rx) = mpsc::channel();
    let listener = match TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))) {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!(
                "single-instance: could not bind loopback listener: {e}; not forwarding launches"
            );
            return SingleInstance::Primary(Primary {
                requests: rx,
                _guard: Guard {
                    cleanup_path: None,
                    cleanup_token: None,
                    _keepalive: Some(tx),
                },
            });
        }
    };

    let port = match listener.local_addr() {
        Ok(addr) => addr.port(),
        Err(e) => {
            eprintln!(
                "single-instance: could not read listener address: {e}; not forwarding launches"
            );
            return SingleInstance::Primary(Primary {
                requests: rx,
                _guard: Guard {
                    cleanup_path: None,
                    cleanup_token: None,
                    _keepalive: Some(tx),
                },
            });
        }
    };

    let token = generate_token();
    let endpoint = TcpEndpoint {
        port,
        token: token.clone(),
    };
    if let Err(e) = write_port_file(port_file, &endpoint) {
        eprintln!(
            "single-instance: could not write {}: {e}; not forwarding launches",
            port_file.display()
        );
        return SingleInstance::Primary(Primary {
            requests: rx,
            _guard: Guard {
                cleanup_path: None,
                cleanup_token: None,
                _keepalive: Some(tx),
            },
        });
    }

    let listener_token = token.clone();
    std::thread::Builder::new()
        .name("single-instance".into())
        .spawn(move || tcp_accept_loop(listener, listener_token, tx))
        .expect("could not start the single-instance listener thread");

    SingleInstance::Primary(Primary {
        requests: rx,
        _guard: Guard {
            cleanup_path: Some(port_file.to_path_buf()),
            cleanup_token: Some(token),
            _keepalive: None,
        },
    })
}

#[cfg_attr(unix, allow(dead_code))]
fn tcp_send_request(endpoint: &TcpEndpoint, files: &[String]) -> io::Result<()> {
    let addr = SocketAddr::from(([127, 0, 0, 1], endpoint.port));
    let mut stream = TcpStream::connect_timeout(&addr, IO_TIMEOUT)?;
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));

    let req = TcpRequest {
        token: endpoint.token.clone(),
        files: files.to_vec(),
    };
    let mut line =
        serde_json::to_string(&req).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    line.push('\n');
    stream.write_all(line.as_bytes())?;
    stream.flush()?;

    let mut ack = [0u8; 3];
    stream.read_exact(&mut ack)?;
    if ack != TCP_ACK {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "single-instance peer rejected the request",
        ));
    }
    Ok(())
}

#[cfg_attr(unix, allow(dead_code))]
fn tcp_accept_loop(listener: TcpListener, expected_token: String, tx: Sender<Vec<String>>) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        if !tcp_handle_connection(stream, &expected_token, &tx) {
            break;
        }
    }
}

/// Handles one incoming TCP connection. Rejects wrong tokens, malformed JSON,
/// and oversized payloads without tearing down the listener. Returns `false`
/// only when the primary receiver has been dropped.
#[cfg_attr(unix, allow(dead_code))]
fn tcp_handle_connection(
    mut stream: TcpStream,
    expected_token: &str,
    tx: &Sender<Vec<String>>,
) -> bool {
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));

    let Some(req) = read_tcp_request(&mut stream) else {
        return true;
    };
    if !constant_time_eq(req.token.as_bytes(), expected_token.as_bytes()) {
        return true;
    }
    if tx.send(req.files).is_err() {
        // The app has dropped its receiver; nothing left to serve.
        return false;
    }
    let _ = stream.write_all(TCP_ACK);
    let _ = stream.flush();
    true
}

#[cfg_attr(unix, allow(dead_code))]
fn read_tcp_request(stream: &mut TcpStream) -> Option<TcpRequest> {
    let mut buf = Vec::new();
    let mut limited = BufReader::new(Read::by_ref(stream).take(MAX_REQUEST_BYTES + 1));
    let n = limited.read_until(b'\n', &mut buf).ok()?;
    if n == 0 || (n as u64) > MAX_REQUEST_BYTES || !buf.ends_with(b"\n") {
        return None;
    }
    let line = std::str::from_utf8(&buf).ok()?;
    serde_json::from_str::<TcpRequest>(line.trim_end()).ok()
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

/// LaunchServices bindings for default-app registration.
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

        // The Editor role result is ignored; the All-roles call below is the one
        // whose status is reported.
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
    use super::{
        file_url_to_path, file_url_to_path_for, resolve_arg, strip_verbatim_prefix, urls_to_paths,
    };
    use crate::platform::Os;
    use std::path::{Path, PathBuf};

    /// A fresh directory per test, under the temp dir. Keep the names short:
    /// socket paths in the same tree are limited to 104 bytes.
    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("zy-os-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Encodes a local path as a valid RFC 8089 `file:///` URL for `os`.
    fn path_to_file_url_for(path: &str, os: Os) -> String {
        let clean = if os == Os::Windows {
            strip_verbatim_prefix(path).replace('\\', "/")
        } else {
            path.to_string()
        };
        let mut encoded = String::with_capacity(clean.len() + 16);
        for &b in clean.as_bytes() {
            let keep =
                b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.' | b'~' | b':');
            if keep {
                encoded.push(b as char);
            } else {
                use std::fmt::Write;
                let _ = write!(encoded, "%{b:02X}");
            }
        }
        if encoded.starts_with('/') {
            format!("file://{encoded}")
        } else {
            format!("file:///{encoded}")
        }
    }

    fn path_to_file_url(path: &Path) -> String {
        path_to_file_url_for(&path.to_string_lossy(), Os::current())
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
        // Windows turns `/` into `\` for local paths; this test is about the
        // percent-decoding, so compare with `/` on every platform.
        let decode = |url: &str| file_url_to_path(url).map(|p| p.replace('\\', "/"));
        assert_eq!(
            decode("file:///Users/me/My%20Notes/a%C3%A9.md").as_deref(),
            Some("/Users/me/My Notes/aé.md")
        );
        assert_eq!(
            decode("file://localhost/tmp/x.md").as_deref(),
            Some("/tmp/x.md")
        );
        assert_eq!(decode("file:/tmp/x.md").as_deref(), Some("/tmp/x.md"));
        assert_eq!(decode("FILE:///tmp/x.md").as_deref(), Some("/tmp/x.md"));
        // A percent-encoded slash decodes to a real one.
        assert_eq!(decode("file:///a%2Fb.md").as_deref(), Some("/a/b.md"));
        // An escape that is not hex stays literal.
        assert_eq!(decode("file:///bad%ZZ").as_deref(), Some("/bad%ZZ"));
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
    fn decodes_windows_file_urls_and_rejects_unc_hosts() {
        assert_eq!(
            file_url_to_path_for("file:///C:/Users/me/My%20Notes/a.md", Os::Windows).as_deref(),
            Some(r"C:\Users\me\My Notes\a.md")
        );
        assert_eq!(
            file_url_to_path_for("file://localhost/C:/notes/x.md", Os::Windows).as_deref(),
            Some(r"C:\notes\x.md")
        );
        assert_eq!(
            file_url_to_path_for("file:/C:/notes/x.md", Os::Windows).as_deref(),
            Some(r"C:\notes\x.md")
        );
        // Percent-encoded drive colon and non-ASCII UTF-8 (accented + Thai).
        assert_eq!(
            file_url_to_path_for(
                "file:///C%3A/notes/a%C3%A9/%E0%B8%AA%E0%B8%A7%E0%B8%B1%E0%B8%AA%E0%B8%94%E0%B8%B5.md",
                Os::Windows
            )
            .as_deref(),
            Some(r"C:\notes\aé\สวัสดี.md")
        );
        // Remote/UNC hosts are rejected on Windows too (local-only policy).
        assert_eq!(
            file_url_to_path_for("file://server/share/a.md", Os::Windows),
            None
        );
        // Helper builds valid URLs for both Unix and Windows paths.
        assert_eq!(
            path_to_file_url_for(r"C:\Users\me\My Note.md", Os::Windows),
            "file:///C:/Users/me/My%20Note.md"
        );
        assert_eq!(
            path_to_file_url_for("/tmp/My Note.md", Os::Mac),
            "file:///tmp/My%20Note.md"
        );
    }

    #[test]
    fn urls_to_paths_keeps_only_file_urls_and_canonicalizes_existing_ones() {
        let dir = scratch_dir("urls");
        let file = dir.join("My Note.md");
        std::fs::write(&file, "# hi").unwrap();
        let url = path_to_file_url(&file);

        let (missing_url, expected_missing) = if cfg!(windows) {
            (
                "file:///C:/definitely/not/here%20.md".to_string(),
                r"C:\definitely\not\here .md",
            )
        } else {
            (
                "file:///definitely/not/here%20.md".to_string(),
                "/definitely/not/here .md",
            )
        };

        let urls = vec![url, "https://example.com/".to_string(), missing_url];
        let paths = urls_to_paths(&urls);
        assert_eq!(paths.len(), 2, "got {paths:?}");
        assert!(paths[0].ends_with("My Note.md"), "got {}", paths[0]);
        assert!(
            !paths[0].starts_with(r"\\?\"),
            "verbatim prefix leaked: {}",
            paths[0]
        );
        assert_eq!(paths[1], expected_missing);

        std::fs::remove_dir_all(&dir).ok();
    }
}

#[cfg(test)]
mod tcp_single_instance_tests {
    use super::{
        acquire_tcp, pick_port_file_path, read_port_file, tcp_send_request, write_port_file,
        Primary, SingleInstance, TcpEndpoint, MAX_REQUEST_BYTES,
    };
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    fn port_file_in(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("zy-tcp-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(".gpui-single-instance.port")
    }

    fn expect_primary(s: SingleInstance) -> Primary {
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
    fn tcp_later_launch_forwards_files_and_empty_raise_to_primary() {
        let port_file = port_file_in("fwd");
        let primary = expect_primary(acquire_tcp(&port_file, vec![]));
        assert!(port_file.exists());

        expect_forwarded(acquire_tcp(
            &port_file,
            vec![r"C:\notes\a.md".into(), r"C:\notes\b.csv".into()],
        ));
        let got = primary
            .requests
            .recv_timeout(Duration::from_secs(5))
            .expect("primary should receive forwarded files");
        assert_eq!(
            got,
            vec![r"C:\notes\a.md".to_string(), r"C:\notes\b.csv".to_string()]
        );

        // Empty launch still reaches the primary so it can raise its window.
        expect_forwarded(acquire_tcp(&port_file, vec![]));
        let got = primary
            .requests
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert!(got.is_empty());

        assert!(matches!(
            primary.requests.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));

        drop(primary);
        assert!(
            !port_file.exists(),
            "dropping the primary removes its port file"
        );
        let _ = std::fs::remove_dir_all(port_file.parent().unwrap());
    }

    #[test]
    fn tcp_rejects_wrong_token_and_stays_alive() {
        let port_file = port_file_in("wrong-token");
        let primary = expect_primary(acquire_tcp(&port_file, vec![]));
        let ep = read_port_file(&port_file).expect("valid port file");

        let bad = TcpEndpoint {
            port: ep.port,
            token: "0000000000000000000000000000000000000000000000000000000000000000".into(),
        };
        assert!(
            tcp_send_request(&bad, &[r"C:\evil.md".into()]).is_err(),
            "wrong token must be rejected"
        );
        assert!(matches!(
            primary.requests.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));

        // Listener is still healthy for a legitimate launch.
        expect_forwarded(acquire_tcp(&port_file, vec![r"C:\good.md".into()]));
        assert_eq!(
            primary
                .requests
                .recv_timeout(Duration::from_secs(5))
                .unwrap(),
            vec![r"C:\good.md".to_string()]
        );

        drop(primary);
        let _ = std::fs::remove_dir_all(port_file.parent().unwrap());
    }

    #[test]
    fn tcp_rejects_oversized_and_malformed_requests_without_crashing() {
        let port_file = port_file_in("bounds");
        let primary = expect_primary(acquire_tcp(&port_file, vec![]));
        let ep = read_port_file(&port_file).expect("valid port file");
        let addr = SocketAddr::from(([127, 0, 0, 1], ep.port));

        // Oversized payload (> MAX_REQUEST_BYTES).
        {
            let mut s = TcpStream::connect(addr).unwrap();
            let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
            let huge = format!(
                "{{\"token\":\"{}\",\"files\":[\"{}\"]}}\n",
                ep.token,
                "a".repeat(MAX_REQUEST_BYTES as usize + 64)
            );
            let _ = s.write_all(huge.as_bytes());
            let mut ack = Vec::new();
            let _ = s.read_to_end(&mut ack);
            assert!(ack.is_empty(), "oversized request must not be acked");
        }

        // Malformed JSON.
        {
            let mut s = TcpStream::connect(addr).unwrap();
            let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
            s.write_all(b"{not valid json\n").unwrap();
            let mut ack = Vec::new();
            let _ = s.read_to_end(&mut ack);
            assert!(ack.is_empty(), "malformed request must not be acked");
        }

        assert!(matches!(
            primary.requests.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));

        // Legitimate launch still succeeds afterwards.
        expect_forwarded(acquire_tcp(&port_file, vec!["/after.md".into()]));
        assert_eq!(
            primary
                .requests
                .recv_timeout(Duration::from_secs(5))
                .unwrap(),
            vec!["/after.md".to_string()]
        );

        drop(primary);
        let _ = std::fs::remove_dir_all(port_file.parent().unwrap());
    }

    #[test]
    fn tcp_takes_over_stale_and_corrupt_port_files() {
        // Bind and immediately drop to obtain an unused ephemeral port.
        let dead_port = {
            let l = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))).unwrap();
            l.local_addr().unwrap().port()
        };

        let port_file = port_file_in("stale");
        write_port_file(
            &port_file,
            &TcpEndpoint {
                port: dead_port,
                token: "stale-token".into(),
            },
        )
        .unwrap();

        let primary = expect_primary(acquire_tcp(&port_file, vec![]));
        let ep = read_port_file(&port_file).unwrap();
        assert_ne!(ep.token, "stale-token");
        expect_forwarded(acquire_tcp(&port_file, vec!["/recovered.md".into()]));
        assert_eq!(
            primary
                .requests
                .recv_timeout(Duration::from_secs(5))
                .unwrap(),
            vec!["/recovered.md".to_string()]
        );
        drop(primary);

        // Corrupt file content is also taken over cleanly.
        std::fs::write(&port_file, b"not json at all").unwrap();
        let primary2 = expect_primary(acquire_tcp(&port_file, vec![]));
        expect_forwarded(acquire_tcp(&port_file, vec!["/corrupt-ok.md".into()]));
        assert_eq!(
            primary2
                .requests
                .recv_timeout(Duration::from_secs(5))
                .unwrap(),
            vec!["/corrupt-ok.md".to_string()]
        );
        drop(primary2);
        let _ = std::fs::remove_dir_all(port_file.parent().unwrap());
    }

    #[test]
    fn tcp_takes_over_when_stale_port_is_reused_by_unrelated_service() {
        let port_file = port_file_in("reused");
        let decoy = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))).unwrap();
        let decoy_port = decoy.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            // Accepts one connection and closes immediately without sending TCP_ACK.
            let _ = decoy.accept();
        });

        write_port_file(
            &port_file,
            &TcpEndpoint {
                port: decoy_port,
                token: "old-token".into(),
            },
        )
        .unwrap();

        let primary = expect_primary(acquire_tcp(&port_file, vec![]));
        handle.join().unwrap();

        let ep = read_port_file(&port_file).unwrap();
        assert_ne!(ep.port, decoy_port);
        expect_forwarded(acquire_tcp(&port_file, vec!["/mine.md".into()]));
        assert_eq!(
            primary
                .requests
                .recv_timeout(Duration::from_secs(5))
                .unwrap(),
            vec!["/mine.md".to_string()]
        );
        drop(primary);
        let _ = std::fs::remove_dir_all(port_file.parent().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn tcp_port_file_is_created_user_only_on_unix() {
        use std::os::unix::fs::PermissionsExt;
        let port_file = port_file_in("perms");
        let primary = expect_primary(acquire_tcp(&port_file, vec![]));
        let mode = std::fs::metadata(&port_file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "token file must be user-only");
        drop(primary);
        let _ = std::fs::remove_dir_all(port_file.parent().unwrap());
    }

    #[test]
    fn tcp_missing_parent_dirs_and_path_selection() {
        let base = port_file_in("nested");
        let nested = base
            .parent()
            .unwrap()
            .join("a")
            .join("b")
            .join(".gpui-single-instance.port");
        let primary = expect_primary(acquire_tcp(&nested, vec![]));
        assert!(nested.exists());
        drop(primary);
        let _ = std::fs::remove_dir_all(base.parent().unwrap());

        let preferred =
            PathBuf::from(r"C:\Users\me\AppData\Roaming\Zyplus\.gpui-single-instance.port");
        assert_eq!(
            pick_port_file_path(
                Some(preferred.clone()),
                PathBuf::from(r"C:\Temp"),
                Some("me")
            ),
            preferred
        );
        let fallback = pick_port_file_path(None, PathBuf::from("/tmp"), Some("Alice Smith!"));
        assert_eq!(fallback, Path::new("/tmp/zyplus-gpui-AliceSmith.port"));
        let empty_user = pick_port_file_path(None, PathBuf::from("/tmp"), Some("!!!"));
        assert_eq!(empty_user, Path::new("/tmp/zyplus-gpui-user.port"));
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
