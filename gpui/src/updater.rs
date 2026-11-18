//! Update check and download for the GPUI build.
//!
//! Replaces `tauri-plugin-updater`. The release workflow still publishes the
//! Tauri `latest.json` manifest (`includeUpdaterJson: true`), so this reads the
//! same file: `version`, `notes`, and `platforms.<os>-<arch>.url`.
//!
//! Everything here blocks. Call `check` and `download` from a background thread.
//! The manifest's `signature` fields (minisign, key in tauri.conf.json) are not
//! verified: downloads are only checked for completeness.

use std::cmp::Ordering;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{mpsc, Arc};
use std::time::Duration;

const REPO_URL: &str = "https://github.com/Fankrits/zyplus-editor";
const MANIFEST_URL: &str =
    "https://github.com/Fankrits/zyplus-editor/releases/latest/download/latest.json";
const MAX_MANIFEST_BYTES: u64 = 256 * 1024;
const FALLBACK_FILE_NAME: &str = "zyplus-update";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInfo {
    /// Version without a leading `v`, e.g. `0.3.0`.
    pub version: String,
    /// Release notes from the manifest; empty when the manifest has none.
    pub notes: String,
    /// Artifact URL for this platform, taken from the manifest as-is.
    pub download_url: String,
    /// Human-facing release page for this version.
    pub page_url: String,
}

type Triple = (u64, u64, u64);

/// Splits `[v]MAJOR.MINOR.PATCH[-pre][+build]` into the numeric core and the
/// optional pre-release tag. Build metadata is ignored.
fn split_version(s: &str) -> Option<(Triple, Option<&str>)> {
    let s = s.trim();
    let s = s.strip_prefix(['v', 'V']).unwrap_or(s);
    let without_build = s.split('+').next()?;
    let (core, pre) = match without_build.split_once('-') {
        Some((_, "")) => return None,
        Some((core, pre)) => (core, Some(pre)),
        None => (without_build, None),
    };
    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    Some(((parse_num(parts[0])?, parse_num(parts[1])?, parse_num(parts[2])?), pre))
}

fn parse_num(s: &str) -> Option<u64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// Semver-style pre-release ordering: dot-separated identifiers, numeric ones
/// compare as numbers and rank below alphanumeric ones; more identifiers rank higher.
fn compare_prerelease(a: &str, b: &str) -> Ordering {
    let mut ai = a.split('.');
    let mut bi = b.split('.');
    loop {
        let ord = match (ai.next(), bi.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => match (parse_num(x), parse_num(y)) {
                // Length first so huge numbers cannot overflow.
                (Some(x), Some(y)) => x.to_string().len().cmp(&y.to_string().len()).then(x.cmp(&y)),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => x.cmp(y),
            },
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
}

/// Parses `[v]MAJOR.MINOR.PATCH`, ignoring any pre-release or build suffix.
#[cfg(test)]
pub fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    split_version(s).map(|(triple, _)| triple)
}

/// True when `candidate` is a newer release than `current`. Unparseable input
/// on either side is never newer. A pre-release sorts below its release
/// (`0.3.0-beta.1` < `0.3.0`), and is still newer than any older core version.
pub fn is_newer(current: &str, candidate: &str) -> bool {
    let (Some((cur, cur_pre)), Some((cand, cand_pre))) =
        (split_version(current), split_version(candidate))
    else {
        return false;
    };
    if cand != cur {
        return cand > cur;
    }
    match (cur_pre, cand_pre) {
        (None, _) => false,
        (Some(_), None) => true,
        (Some(c), Some(n)) => compare_prerelease(n, c) == Ordering::Greater,
    }
}

/// Tauri manifest key for an OS/arch pair (`std::env::consts` values).
pub fn platform_key_for(os: &str, arch: &str) -> Option<&'static str> {
    Some(match (os, arch) {
        ("macos", "aarch64") => "darwin-aarch64",
        ("macos", "x86_64") => "darwin-x86_64",
        ("linux", "x86_64") => "linux-x86_64",
        ("linux", "aarch64") => "linux-aarch64",
        ("windows", "x86_64") => "windows-x86_64",
        ("windows", "aarch64") => "windows-aarch64",
        _ => return None,
    })
}

/// Manifest key for the running build, or `None` on an unsupported platform.
pub fn platform_key() -> Option<&'static str> {
    platform_key_for(std::env::consts::OS, std::env::consts::ARCH)
}

/// Manifest keys to try for a platform, best first.
fn manifest_keys(key: &str) -> Vec<&str> {
    match key {
        "windows-x86_64" => vec!["windows-x86_64-nsis", "windows-x86_64"],
        "windows-aarch64" => vec![
            "windows-aarch64-nsis",
            "windows-aarch64",
            "windows-x86_64-nsis",
            "windows-x86_64",
        ],
        _ => vec![key],
    }
}

/// Pure check against manifest text. `Ok(None)` when `current` is up to date.
/// `platform` is `None` when this OS/arch has no key at all.
///
/// Errors when the manifest is malformed, or when a newer version has no
/// artifact for this platform. Platform gaps are only errors for newer
/// versions, so an old manifest with missing entries still reads as up to date.
pub fn parse_manifest(
    body: &str,
    current: &str,
    platform: Option<&str>,
) -> Result<Option<UpdateInfo>, String> {
    let manifest: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| format!("Update manifest is not valid JSON: {e}"))?;
    let version = manifest
        .get("version")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or("Update manifest has no version")?;
    if split_version(version).is_none() {
        return Err(format!("Update manifest version \"{version}\" is not a release version"));
    }
    if split_version(current).is_none() {
        return Err(format!("Current version \"{current}\" is not a release version"));
    }
    if !is_newer(current, version) {
        return Ok(None);
    }

    let version = version.trim_start_matches(['v', 'V']).to_string();
    let no_build = || format!("No Zyplus {version} build for this platform. See {REPO_URL}/releases");
    let key = platform.ok_or_else(no_build)?;

    let platforms = manifest.get("platforms");
    // The bare windows keys point at the MSI, but install.ps1 installs the NSIS
    // setup.exe. Taking the MSI would leave two installs side by side. Keep the
    // install.ps1 order, including its arm64 -> x64 fallback.
    let url = manifest_keys(key)
        .into_iter()
        .find_map(|k| {
            platforms
                .and_then(|p| p.get(k))
                .and_then(|e| e.get("url"))
                .and_then(|u| u.as_str())
                .filter(|u| !u.is_empty())
        })
        .ok_or_else(|| format!("No Zyplus {version} build for {key}. See {REPO_URL}/releases"))?;
    if !url.starts_with("https://") {
        return Err(format!("Update URL is not HTTPS: {url}"));
    }

    let notes = manifest
        .get("notes")
        .and_then(|n| n.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    Ok(Some(UpdateInfo {
        page_url: format!("{REPO_URL}/releases/tag/v{version}"),
        version,
        notes,
        download_url: url.to_string(),
    }))
}

fn user_agent() -> String {
    format!("zyplus-gpui/{}", env!("CARGO_PKG_VERSION"))
}

/// Agent builder with the TLS backend this platform uses. ureq's `native-tls`
/// feature only compiles the adapter; the connector must be handed to the agent,
/// or every https request fails with "no TLS backend is configured". On Linux the
/// default `tls` (rustls) feature is picked up without configuration.
fn agent_builder() -> Result<ureq::AgentBuilder, String> {
    #[cfg(not(target_os = "linux"))]
    {
        let connector = ureq::native_tls::TlsConnector::new()
            .map_err(|e| format!("Could not start the system TLS stack: {e}"))?;
        Ok(ureq::AgentBuilder::new().tls_connector(Arc::new(connector)))
    }
    #[cfg(target_os = "linux")]
    {
        Ok(ureq::AgentBuilder::new())
    }
}

/// Blocking. Fetches the manifest (10s timeout, 256 KiB cap) and compares it
/// with `current`. Never panics.
pub fn check(current: &str) -> Result<Option<UpdateInfo>, String> {
    let agent = agent_builder()?.timeout(Duration::from_secs(10)).build();
    let resp = agent
        .get(MANIFEST_URL)
        .set("User-Agent", &user_agent())
        .set("Accept", "application/json")
        .call()
        .map_err(|e| format!("Update check failed: {e}"))?;
    let mut body = String::new();
    resp.into_reader()
        .take(MAX_MANIFEST_BYTES)
        .read_to_string(&mut body)
        .map_err(|e| format!("Could not read the update manifest: {e}"))?;
    parse_manifest(&body, current, platform_key())
}

/// Where downloads go: `~/Downloads` when it exists, otherwise the temp dir.
pub fn downloads_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|home| PathBuf::from(home).join("Downloads"))
        .filter(|dir| dir.is_dir())
        .unwrap_or_else(std::env::temp_dir)
}

/// Last path segment of the URL, reduced to `[A-Za-z0-9._-]` with no leading dots.
/// This keeps a hostile manifest from writing outside the target directory.
fn file_name_from_url(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or("");
    let segment = path.rsplit('/').next().unwrap_or("");
    let clean: String = segment
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        .collect();
    let clean = clean.trim_start_matches('.');
    if clean.is_empty() {
        FALLBACK_FILE_NAME.to_string()
    } else {
        clean.to_string()
    }
}

/// Blocking. Streams the artifact into `~/Downloads` (or the temp dir).
/// `progress(bytes_done, total)` is called on start and after each chunk.
pub fn download(info: &UpdateInfo, progress: &dyn Fn(u64, Option<u64>)) -> Result<PathBuf, String> {
    download_into(&downloads_dir(), info, progress)
}

/// Same as `download`, into an explicit directory.
pub fn download_into(
    dir: &Path,
    info: &UpdateInfo,
    progress: &dyn Fn(u64, Option<u64>),
) -> Result<PathBuf, String> {
    if !info.download_url.starts_with("https://") {
        return Err(format!("Update URL is not HTTPS: {}", info.download_url));
    }
    fs::create_dir_all(dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;

    let name = file_name_from_url(&info.download_url);
    let dest = dir.join(&name);
    let part = dir.join(format!("{name}.part"));

    // No global timeout: a large artifact can take longer than any fixed bound.
    let agent = agent_builder()?
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        .timeout_write(Duration::from_secs(30))
        .build();
    let resp = agent
        .get(&info.download_url)
        .set("User-Agent", &user_agent())
        .call()
        .map_err(|e| format!("Download failed: {e}"))?;
    let total = resp
        .header("Content-Length")
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&n| n > 0);

    let mut reader = resp.into_reader();
    if let Err(e) = stream_to(&mut reader, &part, total, progress) {
        let _ = fs::remove_file(&part);
        return Err(e);
    }
    // Rename is atomic on the same volume, so `dest` is never half-written.
    fs::rename(&part, &dest).map_err(|e| {
        let _ = fs::remove_file(&part);
        format!("Could not save {}: {e}", dest.display())
    })?;
    Ok(dest)
}

fn stream_to(
    reader: &mut dyn Read,
    part: &Path,
    total: Option<u64>,
    progress: &dyn Fn(u64, Option<u64>),
) -> Result<(), String> {
    let mut file = File::create(part).map_err(|e| format!("Could not create {}: {e}", part.display()))?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut done: u64 = 0;
    progress(0, total);
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(format!("Download interrupted after {done} bytes: {e}")),
        };
        file.write_all(&buf[..n])
            .map_err(|e| format!("Could not write the update file: {e}"))?;
        done += n as u64;
        progress(done, total);
    }
    file.sync_all()
        .map_err(|e| format!("Could not flush the update file: {e}"))?;
    if done == 0 {
        return Err("Download was empty".to_string());
    }
    if let Some(total) = total {
        if done != total {
            return Err(format!("Download incomplete: got {done} of {total} bytes"));
        }
    }
    Ok(())
}

/// macOS: mounts a `.dmg`, or reveals anything else (the `.app.tar.gz`) in Finder.
#[cfg(target_os = "macos")]
pub fn reveal_or_open(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("{} is missing", path.display()));
    }
    let is_dmg = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("dmg"));
    let mut cmd = Command::new("open");
    if !is_dmg {
        cmd.arg("-R");
    }
    cmd.arg(path)
        .spawn()
        .map(drop)
        .map_err(|e| format!("Could not open {}: {e}", path.display()))
}

/// Windows: selects the file in Explorer. The installer is not run from here.
#[cfg(target_os = "windows")]
pub fn reveal_or_open(path: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    if !path.exists() {
        return Err(format!("{} is missing", path.display()));
    }
    Command::new("explorer")
        .raw_arg(format!("/select,\"{}\"", path.display()))
        .spawn()
        .map(drop)
        .map_err(|e| format!("Could not open {}: {e}", path.display()))
}

/// Linux: opens the containing folder.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn reveal_or_open(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("{} is missing", path.display()));
    }
    let dir = path.parent().unwrap_or(path);
    Command::new("xdg-open")
        .arg(dir)
        .spawn()
        .map(drop)
        .map_err(|e| format!("Could not open {}: {e}", dir.display()))
}

// ---------------------------------------------------------------------------
// Update state for the UI (sidebar pill, Settings → About)
// ---------------------------------------------------------------------------

/// Where the update check and download stand. Transitions are plain methods
/// so the window code only has to forward results into them.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Status {
    /// No check has run, or a silent check found nothing to report.
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(UpdateInfo),
    Downloading {
        info: UpdateInfo,
        done: u64,
        total: Option<u64>,
    },
    Downloaded {
        info: UpdateInfo,
        path: PathBuf,
    },
    /// `info` is kept after a failed download so the offer can be retried.
    Failed {
        message: String,
        info: Option<UpdateInfo>,
    },
}

/// Messages from the download thread to the UI.
pub enum DownloadEvent {
    Progress(u64, Option<u64>),
    Finished(Result<PathBuf, String>),
}

impl Status {
    /// The release on offer, if there is one.
    pub fn info(&self) -> Option<&UpdateInfo> {
        match self {
            Status::Available(info)
            | Status::Downloading { info, .. }
            | Status::Downloaded { info, .. } => Some(info),
            Status::Failed { info, .. } => info.as_ref(),
            Status::Idle | Status::Checking | Status::UpToDate => None,
        }
    }

    /// True while a check or download is running; the UI disables its buttons.
    pub fn busy(&self) -> bool {
        matches!(self, Status::Checking | Status::Downloading { .. })
    }

    /// Applies a check result. A silent check (the one run at startup) reports
    /// nothing when it fails or finds no update.
    pub fn after_check(&mut self, result: Result<Option<UpdateInfo>, String>, silent: bool) {
        *self = match result {
            Ok(Some(info)) => Status::Available(info),
            Ok(None) if silent => Status::Idle,
            Ok(None) => Status::UpToDate,
            Err(_) if silent => Status::Idle,
            Err(message) => Status::Failed {
                message,
                info: None,
            },
        };
    }

    /// Starts downloading the release on offer. Returns it, or `None` when
    /// there is no offer or a download is already running.
    pub fn begin_download(&mut self) -> Option<UpdateInfo> {
        if matches!(self, Status::Downloading { .. }) {
            return None;
        }
        let info = self.info()?.clone();
        *self = Status::Downloading {
            info: info.clone(),
            done: 0,
            total: None,
        };
        Some(info)
    }

    /// Records download progress; ignored unless a download is running.
    pub fn on_progress(&mut self, done: u64, total: Option<u64>) {
        if let Status::Downloading {
            done: current,
            total: current_total,
            ..
        } = self
        {
            *current = done;
            *current_total = total;
        }
    }

    /// Applies the download result. Ignored unless a download is running.
    pub fn on_download_finished(&mut self, result: Result<PathBuf, String>) {
        let current = std::mem::take(self);
        *self = match (current, result) {
            (Status::Downloading { info, .. }, Ok(path)) => Status::Downloaded { info, path },
            (Status::Downloading { info, .. }, Err(message)) => Status::Failed {
                message,
                info: Some(info),
            },
            (other, _) => other,
        };
    }
}

/// Human-readable download progress: `"2.1 MB of 5.4 MB"`, or just the amount
/// done when the total is unknown.
pub fn progress_text(done: u64, total: Option<u64>) -> String {
    match total {
        Some(total) if total > 0 => format!("{} of {}", human_size(done), human_size(total)),
        _ => human_size(done),
    }
}

fn human_size(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if (bytes as f64) < MB {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / MB)
    }
}

/// Runs `download` on its own thread. Progress and the final result arrive on
/// the returned channel, so the UI can poll it without blocking.
pub fn start_download(info: UpdateInfo) -> mpsc::Receiver<DownloadEvent> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let progress_tx = tx.clone();
        let result = download(&info, &move |done, total| {
            let _ = progress_tx.send(DownloadEvent::Progress(done, total));
        });
        let _ = tx.send(DownloadEvent::Finished(result));
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    const CURRENT: &str = "0.2.3";

    fn manifest(version: &str, platforms: &str) -> String {
        format!(
            r#"{{
  "version": "{version}",
  "notes": "  Faster startup.\nFixed tabs.  ",
  "pub_date": "2026-11-01T00:00:00Z",
  "platforms": {platforms}
}}"#
        )
    }

    fn mac_platforms(version: &str) -> String {
        format!(
            r#"{{
    "darwin-aarch64": {{ "signature": "sig", "url": "https://github.com/Fankrits/zyplus-editor/releases/download/v{version}/Zyplus_universal.app.tar.gz" }},
    "darwin-x86_64": {{ "signature": "sig", "url": "https://github.com/Fankrits/zyplus-editor/releases/download/v{version}/Zyplus_universal.app.tar.gz" }},
    "windows-x86_64": {{ "signature": "sig", "url": "https://github.com/Fankrits/zyplus-editor/releases/download/v{version}/Zyplus_{version}_x64-setup.exe" }}
  }}"#
        )
    }

    #[test]
    fn parse_version_accepts_v_prefix_and_suffixes() {
        assert_eq!(parse_version("0.2.3"), Some((0, 2, 3)));
        assert_eq!(parse_version("v1.10.0"), Some((1, 10, 0)));
        assert_eq!(parse_version("V2.0.1"), Some((2, 0, 1)));
        assert_eq!(parse_version(" 0.3.0-beta.1 "), Some((0, 3, 0)));
        assert_eq!(parse_version("1.2.3+build.5"), Some((1, 2, 3)));
        assert_eq!(parse_version("1.2.3-rc.1+build.5"), Some((1, 2, 3)));
    }

    #[test]
    fn parse_version_rejects_malformed() {
        for bad in ["", "v", "1.2", "1.2.3.4", "a.b.c", "1..3", "1.2.3-", "-1.2.3", "1.2.-3", "99999999999999999999.0.0"] {
            assert_eq!(parse_version(bad), None, "{bad:?} should not parse");
        }
    }

    #[test]
    fn is_newer_compares_numerically() {
        assert!(is_newer("0.2.3", "0.2.4"));
        assert!(is_newer("0.2.3", "0.10.0"), "10 > 2 numerically");
        assert!(is_newer("0.2.3", "v1.0.0"));
        assert!(!is_newer("0.2.3", "0.2.3"));
        assert!(!is_newer("0.2.3", "v0.2.2"));
        assert!(!is_newer("0.10.0", "0.9.9"));
    }

    #[test]
    fn is_newer_orders_prereleases() {
        assert!(!is_newer("0.3.0", "0.3.0-beta.1"), "pre-release is below its release");
        assert!(is_newer("0.3.0-beta.1", "0.3.0"));
        assert!(is_newer("0.3.0-beta.1", "0.3.0-beta.2"));
        assert!(is_newer("0.3.0-beta.2", "0.3.0-beta.10"), "numeric identifiers compare as numbers");
        assert!(is_newer("0.3.0-alpha", "0.3.0-beta"));
        assert!(is_newer("0.3.0-rc", "0.3.0-rc.1"), "more identifiers rank higher");
        assert!(!is_newer("0.3.0-rc.1", "0.3.0-rc"));
        assert!(!is_newer("0.3.0-beta.1", "0.3.0-beta.1"));
        assert!(is_newer("0.2.3", "0.3.0-beta.1"), "newer core beats any pre-release of an older core");
    }

    #[test]
    fn is_newer_is_false_for_garbage() {
        assert!(!is_newer("0.2.3", "not-a-version"));
        assert!(!is_newer("0.2.3", ""));
        assert!(!is_newer("garbage", "0.2.4"));
    }

    #[test]
    fn platform_keys_match_tauri_manifest() {
        assert_eq!(platform_key_for("macos", "aarch64"), Some("darwin-aarch64"));
        assert_eq!(platform_key_for("macos", "x86_64"), Some("darwin-x86_64"));
        assert_eq!(platform_key_for("linux", "x86_64"), Some("linux-x86_64"));
        assert_eq!(platform_key_for("windows", "x86_64"), Some("windows-x86_64"));
        assert_eq!(platform_key_for("windows", "aarch64"), Some("windows-aarch64"));
        assert_eq!(platform_key_for("freebsd", "x86_64"), None);
        assert_eq!(platform_key_for("macos", "riscv64"), None);
    }

    #[test]
    fn up_to_date_manifest_yields_none() {
        let body = manifest("0.2.3", &mac_platforms("0.2.3"));
        assert_eq!(parse_manifest(&body, CURRENT, Some("darwin-aarch64")), Ok(None));
    }

    #[test]
    fn up_to_date_ignores_missing_platform() {
        // An older manifest with no entry for this platform is not an error.
        let body = manifest("0.2.3", r#"{ "windows-x86_64": { "url": "https://x/y.exe" } }"#);
        assert_eq!(parse_manifest(&body, CURRENT, Some("darwin-aarch64")), Ok(None));
        assert_eq!(parse_manifest(&body, CURRENT, None), Ok(None));
    }

    #[test]
    fn newer_manifest_selects_platform_url() {
        let body = manifest("0.2.4", &mac_platforms("0.2.4"));
        let info = parse_manifest(&body, CURRENT, Some("darwin-aarch64"))
            .expect("parse")
            .expect("newer version offered");
        assert_eq!(info.version, "0.2.4");
        assert_eq!(info.notes, "Faster startup.\nFixed tabs.");
        assert_eq!(
            info.download_url,
            "https://github.com/Fankrits/zyplus-editor/releases/download/v0.2.4/Zyplus_universal.app.tar.gz"
        );
        assert_eq!(info.page_url, "https://github.com/Fankrits/zyplus-editor/releases/tag/v0.2.4");
    }

    #[test]
    fn windows_prefers_nsis_setup_over_msi() {
        let body = manifest(
            "0.3.0",
            r#"{
  "windows-x86_64": { "url": "https://github.com/x/releases/download/v0.3.0/Zyplus_0.3.0_x64_en-US.msi" },
  "windows-x86_64-msi": { "url": "https://github.com/x/releases/download/v0.3.0/Zyplus_0.3.0_x64_en-US.msi" },
  "windows-x86_64-nsis": { "url": "https://github.com/x/releases/download/v0.3.0/Zyplus_0.3.0_x64-setup.exe" }
}"#,
        );
        let info = parse_manifest(&body, CURRENT, Some("windows-x86_64"))
            .unwrap()
            .unwrap();
        assert!(info.download_url.ends_with("_x64-setup.exe"), "{}", info.download_url);
    }

    #[test]
    fn windows_msi_only_manifest_still_offers_msi() {
        let body = manifest(
            "0.3.0",
            r#"{ "windows-x86_64": { "url": "https://x/Zyplus_0.3.0_x64_en-US.msi" } }"#,
        );
        let info = parse_manifest(&body, CURRENT, Some("windows-x86_64")).unwrap().unwrap();
        assert!(info.download_url.ends_with(".msi"));
    }

    #[test]
    fn windows_arm_falls_back_to_x64_build() {
        let body = manifest("0.3.0", &mac_platforms("0.3.0"));
        let info = parse_manifest(&body, CURRENT, Some("windows-aarch64"))
            .expect("parse")
            .expect("offered");
        assert!(info.download_url.ends_with("Zyplus_0.3.0_x64-setup.exe"));
    }

    #[test]
    fn v_prefixed_manifest_version_is_stripped() {
        let body = manifest("v0.3.0", &mac_platforms("0.3.0"));
        let info = parse_manifest(&body, CURRENT, Some("darwin-x86_64"))
            .expect("parse")
            .expect("offered");
        assert_eq!(info.version, "0.3.0");
        assert_eq!(info.page_url, "https://github.com/Fankrits/zyplus-editor/releases/tag/v0.3.0");
    }

    #[test]
    fn malformed_json_is_an_error() {
        for body in ["", "not json", "{", "[1,2,3]", r#"{"version": "0.3.0", "platforms": "#] {
            let err = parse_manifest(body, CURRENT, Some("darwin-aarch64")).unwrap_err();
            assert!(err.contains("not valid JSON") || err.contains("no version"), "{body:?}: {err}");
        }
    }

    #[test]
    fn missing_or_bad_version_is_an_error() {
        let no_version = r#"{ "platforms": {} }"#;
        assert!(parse_manifest(no_version, CURRENT, Some("darwin-aarch64")).unwrap_err().contains("no version"));

        let empty = r#"{ "version": "  ", "platforms": {} }"#;
        assert!(parse_manifest(empty, CURRENT, Some("darwin-aarch64")).unwrap_err().contains("no version"));

        let junk = r#"{ "version": "latest", "platforms": {} }"#;
        assert!(parse_manifest(junk, CURRENT, Some("darwin-aarch64")).unwrap_err().contains("not a release version"));
    }

    #[test]
    fn newer_version_without_platform_entry_is_an_error() {
        let body = manifest("0.2.4", r#"{ "windows-x86_64": { "url": "https://x/y.exe" } }"#);
        let err = parse_manifest(&body, CURRENT, Some("darwin-aarch64")).unwrap_err();
        assert!(err.contains("darwin-aarch64") && err.contains("0.2.4"), "{err}");
    }

    #[test]
    fn newer_version_on_unsupported_platform_is_an_error() {
        let body = manifest("0.2.4", &mac_platforms("0.2.4"));
        let err = parse_manifest(&body, CURRENT, None).unwrap_err();
        assert!(err.contains("0.2.4"), "{err}");
    }

    #[test]
    fn platform_entry_without_url_is_an_error() {
        let body = manifest("0.2.4", r#"{ "darwin-aarch64": { "signature": "sig" } }"#);
        assert!(parse_manifest(&body, CURRENT, Some("darwin-aarch64")).is_err());
    }

    #[test]
    fn non_https_url_is_rejected() {
        let body = manifest("0.2.4", r#"{ "darwin-aarch64": { "url": "http://example.com/a.tar.gz" } }"#);
        let err = parse_manifest(&body, CURRENT, Some("darwin-aarch64")).unwrap_err();
        assert!(err.contains("not HTTPS"), "{err}");
    }

    #[test]
    fn prerelease_manifest_is_offered_only_when_newer() {
        let body = manifest("0.3.0-beta.1", &mac_platforms("0.3.0-beta.1"));
        let offered = parse_manifest(&body, CURRENT, Some("darwin-aarch64"))
            .expect("parse")
            .expect("0.3.0-beta.1 is newer than 0.2.3");
        assert_eq!(offered.version, "0.3.0-beta.1");

        assert_eq!(parse_manifest(&body, "0.3.0", Some("darwin-aarch64")), Ok(None));
        assert!(parse_manifest(&body, "0.3.0-beta.1", Some("darwin-aarch64")).unwrap().is_none());
    }

    #[test]
    fn missing_notes_become_empty() {
        let body = r#"{ "version": "0.2.4", "platforms": { "darwin-aarch64": { "url": "https://x/y.tar.gz" } } }"#;
        let info = parse_manifest(body, CURRENT, Some("darwin-aarch64")).unwrap().unwrap();
        assert_eq!(info.notes, "");
    }

    #[test]
    fn current_version_must_parse() {
        let body = manifest("0.2.4", &mac_platforms("0.2.4"));
        assert!(parse_manifest(&body, "dev", Some("darwin-aarch64")).is_err());
    }

    #[test]
    fn file_names_are_sanitized() {
        assert_eq!(
            file_name_from_url("https://github.com/x/releases/download/v1/Zyplus_0.3.0_universal.dmg?x=1"),
            "Zyplus_0.3.0_universal.dmg"
        );
        assert_eq!(file_name_from_url("https://h/a/Zyplus.app.tar.gz"), "Zyplus.app.tar.gz");
        assert_eq!(file_name_from_url("https://h/../../etc/passwd"), "passwd");
        assert_eq!(file_name_from_url("https://h/..%2F..%2Fevil"), "..2F..2Fevil".trim_start_matches('.'));
        assert_eq!(file_name_from_url("https://h/"), FALLBACK_FILE_NAME);
        assert_eq!(file_name_from_url("https://h/..."), FALLBACK_FILE_NAME);
    }

    #[test]
    fn download_rejects_non_https_before_touching_disk() {
        let info = UpdateInfo {
            version: "0.2.4".into(),
            notes: String::new(),
            download_url: "http://example.com/a.dmg".into(),
            page_url: String::new(),
        };
        let dir = std::env::temp_dir().join("zyplus-updater-test-never-created");
        let err = download_into(&dir, &info, &|_, _| {}).unwrap_err();
        assert!(err.contains("not HTTPS"));
        assert!(!dir.exists());
    }

    /// Hits GitHub. Run with: cargo test live_ -- --ignored
    #[test]
    #[ignore = "network: fetches the real latest.json"]
    fn live_manifest_offers_an_update_over_0_0_0() {
        let info = check("0.0.0")
            .expect("check failed")
            .expect("0.0.0 is older than any published release");
        assert!(info.download_url.starts_with("https://"), "{info:?}");
        assert!(!info.version.is_empty());
    }

    /// Downloads the real artifact. Set ZYPLUS_UPDATER_TEST_DIR to a scratch dir.
    #[test]
    #[ignore = "network: downloads the real artifact"]
    fn live_download_into_test_dir() {
        let Some(dir) = std::env::var_os("ZYPLUS_UPDATER_TEST_DIR") else {
            panic!("set ZYPLUS_UPDATER_TEST_DIR");
        };
        let info = check("0.0.0").expect("check").expect("offered");
        let last = std::cell::Cell::new(0u64);
        let path = download_into(Path::new(&dir), &info, &|done, _| last.set(done))
            .expect("download");
        let len = fs::metadata(&path).expect("stat").len();
        assert_eq!(len, last.get());
        assert!(len > 0);
        eprintln!("downloaded {} ({len} bytes)", path.display());
    }
}

#[cfg(test)]
mod state_tests {
    use super::*;

    fn info(version: &str) -> UpdateInfo {
        UpdateInfo {
            version: version.to_string(),
            notes: String::new(),
            download_url: "https://example.com/Zyplus.dmg".to_string(),
            page_url: "https://example.com/release".to_string(),
        }
    }

    #[test]
    fn check_result_maps_to_status() {
        let mut status = Status::Checking;
        status.after_check(Ok(Some(info("0.3.0"))), false);
        assert_eq!(status.info().map(|i| i.version.as_str()), Some("0.3.0"));

        let mut status = Status::Checking;
        status.after_check(Ok(None), false);
        assert_eq!(status, Status::UpToDate);
        assert!(status.info().is_none());

        // A silent check reports nothing, neither "up to date" nor errors.
        let mut status = Status::Idle;
        status.after_check(Ok(None), true);
        assert_eq!(status, Status::Idle);
        status.after_check(Err("offline".into()), true);
        assert_eq!(status, Status::Idle);

        let mut status = Status::Checking;
        status.after_check(Err("offline".into()), false);
        assert_eq!(
            status,
            Status::Failed {
                message: "offline".into(),
                info: None
            }
        );
    }

    #[test]
    fn download_success_path() {
        let mut status = Status::Available(info("0.3.0"));
        let started = status.begin_download().expect("offer exists");
        assert_eq!(started.version, "0.3.0");
        assert!(status.busy());
        // A second click while downloading does nothing.
        assert!(status.begin_download().is_none());

        status.on_progress(512, Some(2048));
        assert_eq!(
            status,
            Status::Downloading {
                info: info("0.3.0"),
                done: 512,
                total: Some(2048)
            }
        );

        status.on_download_finished(Ok(PathBuf::from("/tmp/Zyplus.dmg")));
        assert!(!status.busy());
        assert_eq!(
            status,
            Status::Downloaded {
                info: info("0.3.0"),
                path: PathBuf::from("/tmp/Zyplus.dmg")
            }
        );
        assert!(status.info().is_some(), "the offer stays visible after download");
    }

    #[test]
    fn failed_download_keeps_the_offer_for_retry() {
        let mut status = Status::Available(info("0.3.0"));
        status.begin_download();
        status.on_download_finished(Err("network dropped".into()));
        assert_eq!(
            status,
            Status::Failed {
                message: "network dropped".into(),
                info: Some(info("0.3.0"))
            }
        );
        assert!(status.begin_download().is_some(), "retry is allowed");
    }

    #[test]
    fn stray_events_are_ignored() {
        let mut status = Status::Idle;
        status.on_progress(1, None);
        status.on_download_finished(Ok(PathBuf::from("/tmp/x")));
        assert_eq!(status, Status::Idle);

        let mut status = Status::UpToDate;
        assert!(status.begin_download().is_none());
        assert_eq!(status, Status::UpToDate);
    }

    #[test]
    fn progress_text_formats_sizes() {
        assert_eq!(progress_text(0, None), "0 B");
        assert_eq!(progress_text(2048, Some(0)), "2 KB", "zero total means unknown");
        assert_eq!(progress_text(2048, None), "2 KB");
        assert_eq!(progress_text(2_200_000, Some(5_400_000)), "2.1 MB of 5.1 MB");
    }
}
