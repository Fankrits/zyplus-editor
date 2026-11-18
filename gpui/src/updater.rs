//! Update check and download for the GPUI build.
//!
//! Replaces `tauri-plugin-updater`. The release workflow still publishes the
//! Tauri `latest.json` manifest (`includeUpdaterJson: true`), so this reads the
//! same file: `version`, `notes`, and `platforms.<os>-<arch>.url`.
//!
//! Everything here blocks. Call `check` and `start_update` from background
//! threads. A download is only used after its minisign signature (the manifest's
//! `signature`, checked against the Tauri updater key) verifies. On macOS, a
//! verified `.app.tar.gz` is installed over the running bundle and relaunched.

use std::cmp::Ordering;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use base64::Engine;

const REPO_URL: &str = "https://github.com/Fankrits/zyplus-editor";
const MANIFEST_URL: &str =
    "https://github.com/Fankrits/zyplus-editor/releases/latest/download/latest.json";
/// Tauri updater public key: base64 of the minisign public key file. Copied from
/// `plugins.updater.pubkey` in src-tauri/tauri.conf.json. Keep the two in sync
/// when the signing key rotates.
const PUBKEY_B64: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDNEMDQzOThGREZGNDA0RDAKUldUUUJQVGZqemtFUFRkbnYxMVpwZllBY0dhT2pjN1lCVXlBcjB6Z2tHOVlGdlN3akJJQ3lXcG4K";
/// Bundle identifier the update must carry (tauri.conf.json `identifier`).
const BUNDLE_ID: &str = "com.fankrits.zyplus-editor";
const APP_BUNDLE_NAME: &str = "Zyplus.app";
/// Debug-build overrides for testing against a local fixture release.
const DEV_URL_ENV: &str = "ZYPLUS_UPDATE_URL";
const DEV_PUBKEY_ENV: &str = "ZYPLUS_UPDATE_PUBKEY";
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
    /// The manifest's `signature` for this platform: base64 of a minisign `.sig` file.
    pub signature: String,
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
    // install.ps1 order, including its arm64 -> x64 fallback. The signature must
    // come from the same entry as the URL.
    let entry = manifest_keys(key)
        .into_iter()
        .filter_map(|k| platforms.and_then(|p| p.get(k)))
        .find(|e| e.get("url").and_then(|u| u.as_str()).is_some_and(|u| !u.is_empty()))
        .ok_or_else(|| format!("No Zyplus {version} build for {key}. See {REPO_URL}/releases"))?;
    let url = entry.get("url").and_then(|u| u.as_str()).unwrap_or_default();
    if !url_allowed(url) {
        return Err(format!("Update URL is not HTTPS: {url}"));
    }
    let signature = entry
        .get("signature")
        .and_then(|s| s.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("The update manifest has no signature for {key}, so the update is refused"))?;

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
        signature: signature.to_string(),
    }))
}

/// `https://` always. Plain `http://` only for loopback URLs, and only in a debug
/// build with `ZYPLUS_UPDATE_URL` set (the local fixture release).
fn url_allowed(url: &str) -> bool {
    url.starts_with("https://")
        || (dev_env(DEV_URL_ENV).is_some()
            && (url.starts_with("http://127.0.0.1:") || url.starts_with("http://localhost:")))
}

/// Reads a debug-only override. Release builds never read the environment here,
/// so a user's shell cannot redirect or re-key updates.
fn dev_env(name: &str) -> Option<String> {
    if cfg!(debug_assertions) {
        std::env::var(name).ok().filter(|v| !v.trim().is_empty())
    } else {
        None
    }
}

fn manifest_url() -> String {
    dev_env(DEV_URL_ENV).unwrap_or_else(|| MANIFEST_URL.to_string())
}

fn pubkey_b64() -> String {
    dev_env(DEV_PUBKEY_ENV).unwrap_or_else(|| PUBKEY_B64.to_string())
}

// ---------------------------------------------------------------------------
// Signature verification (minisign, Tauri's key and signature encoding)
// ---------------------------------------------------------------------------

fn decode_b64_text(b64: &str) -> Result<String, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .map_err(|e| e.to_string())?;
    String::from_utf8(bytes).map_err(|e| e.to_string())
}

/// Checks `bytes` against a manifest `signature`. Both arguments use Tauri's
/// encoding: base64 of the minisign file text. The check covers the artifact
/// bytes and the signed trusted comment (the global signature). Only pre-hashed
/// (`ED`) signatures are accepted; the legacy `Ed` form is refused.
pub fn verify_signature(bytes: &[u8], signature_b64: &str, pubkey_b64: &str) -> Result<(), String> {
    let key = decode_b64_text(pubkey_b64)
        .map_err(|e| format!("The update public key is invalid: {e}"))
        .and_then(|text| {
            minisign_verify::PublicKey::decode(&text)
                .map_err(|e| format!("The update public key is invalid: {e}"))
        })?;
    let sig = decode_b64_text(signature_b64)
        .map_err(|e| format!("The update signature is invalid: {e}"))
        .and_then(|text| {
            minisign_verify::Signature::decode(&text)
                .map_err(|e| format!("The update signature is invalid: {e}"))
        })?;
    key.verify(bytes, &sig, false)
        .map_err(|e| format!("the signature does not match this release ({e})"))
}

/// The archive listing must hold only entries under `Zyplus.app/`, with no
/// absolute paths and no `..` parts. Checked before anything is extracted.
pub fn check_archive_entries(entries: &[&str]) -> Result<(), String> {
    if entries.is_empty() {
        return Err("The update archive is empty".to_string());
    }
    for entry in entries {
        let name = entry.trim_start_matches("./").trim_end_matches('/');
        let inside = name == APP_BUNDLE_NAME || name.starts_with(&format!("{APP_BUNDLE_NAME}/"));
        let unsafe_part = name.starts_with('/') || name.split('/').any(|part| part == "..");
        if !inside || unsafe_part {
            return Err(format!("The update archive has an unexpected entry: {entry}"));
        }
    }
    Ok(())
}

/// Rejects symlinks that are absolute or contain `..`. Every other symlink then
/// resolves inside the bundle, because no path can climb out of it.
fn check_links_stay_inside(dir: &Path) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("Could not read {}: {e}", dir.display()))?;
    for entry in entries {
        let path = entry.map_err(|e| e.to_string())?.path();
        let meta = fs::symlink_metadata(&path).map_err(|e| format!("Could not read {}: {e}", path.display()))?;
        if meta.file_type().is_symlink() {
            let target = fs::read_link(&path).map_err(|e| e.to_string())?;
            let climbs = target.components().any(|c| matches!(c, Component::ParentDir));
            if target.is_absolute() || climbs {
                return Err(format!("The update contains a link that leaves the app: {}", path.display()));
            }
        } else if meta.is_dir() {
            check_links_stay_inside(&path)?;
        }
    }
    Ok(())
}

/// Facts from a bundle's Info.plist that the update relies on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleFacts {
    pub identifier: String,
    pub version: String,
    pub executable: String,
}

/// The text of `<key>name</key><string>value</string>` in an XML plist. Only
/// top-level string values are read; that covers everything the update needs.
pub fn plist_string(xml: &str, key: &str) -> Option<String> {
    let (_, after) = xml.split_once(&format!("<key>{key}</key>"))?;
    let rest = after.trim_start().strip_prefix("<string>")?;
    let (value, _) = rest.split_once("</string>")?;
    Some(value.trim().to_string())
}

/// Checks the new bundle's Info.plist: same bundle id, a version newer than
/// `current`, and the same executable name as the running build. The executable
/// check stops a different app (for example a Tauri build) from replacing this one.
pub fn validate_bundle_plist(xml: &str, current: &str, running_exe: &str) -> Result<BundleFacts, String> {
    let identifier = plist_string(xml, "CFBundleIdentifier")
        .ok_or("The update's Info.plist has no CFBundleIdentifier")?;
    if identifier != BUNDLE_ID {
        return Err(format!("The update is for {identifier}, not {BUNDLE_ID}"));
    }
    let version = plist_string(xml, "CFBundleShortVersionString")
        .or_else(|| plist_string(xml, "CFBundleVersion"))
        .filter(|v| !v.is_empty())
        .ok_or("The update's Info.plist has no version")?;
    if !is_newer(current, &version) {
        return Err(format!("The update is version {version}, which is not newer than {current}"));
    }
    let executable = plist_string(xml, "CFBundleExecutable")
        .filter(|e| !e.is_empty() && !e.contains('/'))
        .ok_or("The update's Info.plist has no CFBundleExecutable")?;
    if executable != running_exe {
        return Err(format!(
            "The update contains a different app ({executable}), so it was not installed"
        ));
    }
    Ok(BundleFacts {
        identifier,
        version,
        executable,
    })
}

/// `<X>.app` when `exe` is `<X>.app/Contents/MacOS/<bin>`, otherwise `None`
/// (cargo builds and any other location).
pub fn bundle_for_exe(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    let is_macos_dir = macos.file_name()? == "MacOS";
    let is_contents_dir = contents.file_name()? == "Contents";
    let is_app = app.extension().is_some_and(|e| e == "app");
    (is_macos_dir && is_contents_dir && is_app).then(|| app.to_path_buf())
}

/// The running `.app` bundle, when this process was started from one. Only
/// macOS installs; other platforms download and reveal.
pub fn installable_bundle() -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    std::env::current_exe().ok().and_then(|exe| bundle_for_exe(&exe))
}

fn running_exe_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default()
}

/// Why an install stopped. `reveal` means the user has to move the new app into
/// place by hand; the downloaded archive is kept so it can be revealed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallError {
    pub message: String,
    pub reveal: bool,
}

fn plain(message: String) -> InstallError {
    InstallError {
        message,
        reveal: false,
    }
}

fn is_permission(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::PermissionDenied || matches!(e.raw_os_error(), Some(1 | 13 | 30))
}

/// Renames `app` to `backup`, moves `new_app` into its place, and restores
/// `app` from `backup` if the second move fails. Both moves stay inside one
/// directory, so each is a rename and nothing is copied.
pub fn replace_bundle(app: &Path, new_app: &Path, backup: &Path) -> Result<(), InstallError> {
    if backup.exists() {
        return Err(plain(format!("{} already exists", backup.display())));
    }
    if let Err(e) = fs::rename(app, backup) {
        return Err(InstallError {
            message: move_by_hand_message(app, &e),
            reveal: is_permission(&e),
        });
    }
    if let Err(e) = fs::rename(new_app, app) {
        let restored = fs::rename(backup, app);
        let message = match restored {
            Ok(()) => format!("Could not put the new version in place ({e}). The previous version was restored."),
            Err(r) => format!(
                "Could not put the new version in place ({e}), and the previous version could not be restored ({r}). It is at {}.",
                backup.display()
            ),
        };
        return Err(plain(message));
    }
    Ok(())
}

fn move_by_hand_message(app: &Path, e: &io::Error) -> String {
    if is_permission(e) {
        format!(
            "Zyplus cannot replace {} ({e}). Open the downloaded file and move the new Zyplus.app into place by hand.",
            app.display()
        )
    } else {
        format!("Could not move {} aside ({e}). Nothing was changed.", app.display())
    }
}

fn run_tar(args: &[&str], archive: &Path, dest: Option<&Path>) -> Result<String, InstallError> {
    let mut cmd = Command::new("/usr/bin/tar");
    cmd.args(args).arg(archive);
    if let Some(dest) = dest {
        cmd.arg("-C").arg(dest);
    }
    let out = cmd
        .output()
        .map_err(|e| plain(format!("Could not run tar: {e}")))?;
    if !out.status.success() {
        return Err(plain(format!(
            "The update archive could not be unpacked: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Reads an Info.plist as XML text. `plutil` also accepts binary plists.
fn read_plist_xml(plist: &Path) -> Result<String, String> {
    let out = Command::new("/usr/bin/plutil")
        .args(["-convert", "xml1", "-o", "-"])
        .arg(plist)
        .output()
        .map_err(|e| format!("Could not run plutil: {e}"))?;
    if !out.status.success() {
        return Err(format!("The update's Info.plist could not be read: {}", plist.display()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Unpacks a verified archive next to the app, validates it, and replaces the
/// running bundle. Only ever call this after `verify_signature` has passed.
pub fn install_archive(
    archive: &Path,
    app: &Path,
    current: &str,
    running_exe: &str,
) -> Result<(), InstallError> {
    let parent = app
        .parent()
        .ok_or_else(|| plain(format!("{} has no parent folder", app.display())))?;
    let app_name = app
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| APP_BUNDLE_NAME.to_string());
    let staging = parent.join(format!(".zyplus-update-{}", std::process::id()));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir(&staging).map_err(|e| InstallError {
        message: format!(
            "Could not prepare the update in {} ({e}). Open the downloaded file and move the new Zyplus.app into place by hand.",
            parent.display()
        ),
        reveal: is_permission(&e),
    })?;

    let result = unpack_and_replace(archive, app, &app_name, &staging, current, running_exe);
    let _ = fs::remove_dir_all(&staging);
    result
}

fn unpack_and_replace(
    archive: &Path,
    app: &Path,
    app_name: &str,
    staging: &Path,
    current: &str,
    running_exe: &str,
) -> Result<(), InstallError> {
    let listing = run_tar(&["-tzf"], archive, None)?;
    let entries: Vec<&str> = listing.lines().filter(|l| !l.is_empty()).collect();
    check_archive_entries(&entries).map_err(plain)?;
    run_tar(&["-xzf"], archive, Some(staging))?;

    let new_app = staging.join(APP_BUNDLE_NAME);
    if !new_app.is_dir() {
        return Err(plain(format!("The update archive does not contain {APP_BUNDLE_NAME}")));
    }
    check_links_stay_inside(&new_app).map_err(plain)?;
    let xml = read_plist_xml(&new_app.join("Contents/Info.plist")).map_err(plain)?;
    let facts = validate_bundle_plist(&xml, current, running_exe).map_err(plain)?;
    if !new_app.join("Contents/MacOS").join(&facts.executable).is_file() {
        return Err(plain(format!("The update has no {} executable", facts.executable)));
    }

    // Next to the app, so the rename stays on one volume.
    let backup = app.with_file_name(format!("{app_name}.old-{}", std::process::id()));
    replace_bundle(app, &new_app, &backup)?;
    // The old bundle is no longer needed. Failing to delete it leaves a stray
    // folder, not a broken install, so the error is ignored.
    let _ = fs::remove_dir_all(&backup);
    Ok(())
}

/// Starts a detached helper that waits for `pid` to exit, then opens `app` as a
/// new instance. The app quits right after, so the old process is gone by then.
pub fn spawn_relaunch(app: &Path, pid: u32) -> io::Result<()> {
    Command::new("/bin/sh")
        .arg("-c")
        .arg(r#"i=0; while kill -0 "$1" 2>/dev/null && [ "$i" -lt 100 ]; do sleep 0.2; i=$((i+1)); done; sleep 0.3; exec open -n "$2""#)
        .arg("zyplus-relaunch")
        .arg(pid.to_string())
        .arg(app)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
}

/// The app to relaunch once the process quits. Set by the Restart button and
/// cleared if the quit is cancelled.
static PENDING_RELAUNCH: Mutex<Option<PathBuf>> = Mutex::new(None);

pub fn request_relaunch(app: PathBuf) {
    *PENDING_RELAUNCH.lock().unwrap_or_else(|e| e.into_inner()) = Some(app);
}

pub fn cancel_relaunch() {
    *PENDING_RELAUNCH.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Called right before the process exits. Starts the helper when a relaunch was requested.
pub fn relaunch_on_exit() {
    let app = PENDING_RELAUNCH.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(app) = app {
        if let Err(e) = spawn_relaunch(&app, std::process::id()) {
            eprintln!("zyplus: could not restart the app: {e}");
        }
    }
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
        .get(&manifest_url())
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

/// Same as `download`, into an explicit directory.
pub fn download_into(
    dir: &Path,
    info: &UpdateInfo,
    progress: &dyn Fn(u64, Option<u64>),
) -> Result<PathBuf, String> {
    if !url_allowed(&info.download_url) {
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

/// Where the check, download, verification and install stand. Transitions are
/// plain methods, so the window code only forwards results into them.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Status {
    /// No check has run yet.
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
    /// The download is being checked against the release signature.
    Verifying { info: UpdateInfo },
    /// The verified bundle is being moved into place (macOS app bundles only).
    Installing { info: UpdateInfo },
    /// Verified and saved, but not installed by Zyplus (Windows, Linux, cargo builds).
    Downloaded { info: UpdateInfo, path: PathBuf },
    /// Installed over the running bundle. Restart relaunches `app`.
    ReadyToRestart { info: UpdateInfo, app: PathBuf },
    /// `info` is kept after a failed download so the offer can be retried.
    /// `file` is set when the user has to move the new app into place by hand.
    Failed {
        message: String,
        info: Option<UpdateInfo>,
        file: Option<PathBuf>,
    },
}

/// Intermediate steps reported by a running update.
#[derive(Debug, Clone, PartialEq)]
pub enum Phase {
    Progress(u64, Option<u64>),
    Verifying,
    Installing,
}

/// What a successful update left behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finished {
    /// Verified and saved at this path.
    Downloaded(PathBuf),
    /// Verified and installed over this app bundle.
    Installed(PathBuf),
}

/// A failed update. `file` is set when the user must move the new app by hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub message: String,
    pub file: Option<PathBuf>,
}

/// Messages from the update thread to the UI.
pub enum DownloadEvent {
    Phase(Phase),
    Finished(Result<Finished, Failure>),
}

/// What the About panel's single button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Check,
    Download,
    Restart,
    Reveal,
}

/// What the About panel shows: a line of text, an optional progress bar, and one button.
#[derive(Debug, Clone, PartialEq)]
pub struct PanelView {
    pub text: String,
    pub danger: bool,
    /// 0.0..=1.0 while the download size is known.
    pub progress: Option<f32>,
    pub button: String,
    /// `None` when the button is shown disabled.
    pub action: Option<Action>,
}

/// Fraction of `total` that `done` covers, or `None` when the total is unknown or zero.
pub fn fraction(done: u64, total: Option<u64>) -> Option<f32> {
    match total {
        Some(total) if total > 0 => Some((done as f32 / total as f32).clamp(0.0, 1.0)),
        _ => None,
    }
}

fn reveal_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "Reveal in Finder"
    } else if cfg!(target_os = "windows") {
        "Show in Explorer"
    } else {
        "Show in folder"
    }
}

fn finish(info: UpdateInfo, result: Result<Finished, Failure>) -> Status {
    match result {
        Ok(Finished::Downloaded(path)) => Status::Downloaded { info, path },
        Ok(Finished::Installed(app)) => Status::ReadyToRestart { info, app },
        Err(failure) => Status::Failed {
            message: failure.message,
            info: Some(info),
            file: failure.file,
        },
    }
}

impl Status {
    /// The release on offer, if there is one.
    pub fn info(&self) -> Option<&UpdateInfo> {
        match self {
            Status::Available(info)
            | Status::Downloading { info, .. }
            | Status::Verifying { info }
            | Status::Installing { info }
            | Status::Downloaded { info, .. }
            | Status::ReadyToRestart { info, .. } => Some(info),
            Status::Failed { info, .. } => info.as_ref(),
            Status::Idle | Status::Checking | Status::UpToDate => None,
        }
    }

    /// True while a check or an update runs; the button is disabled then.
    pub fn busy(&self) -> bool {
        matches!(
            self,
            Status::Checking
                | Status::Downloading { .. }
                | Status::Verifying { .. }
                | Status::Installing { .. }
        )
    }

    /// The installed app waiting for a restart, if any.
    pub fn installed_app(&self) -> Option<&Path> {
        match self {
            Status::ReadyToRestart { app, .. } => Some(app),
            _ => None,
        }
    }

    /// Short name for the test script's `state` output.
    pub fn kind(&self) -> &'static str {
        match self {
            Status::Idle => "idle",
            Status::Checking => "checking",
            Status::UpToDate => "uptodate",
            Status::Available(_) => "available",
            Status::Downloading { .. } => "downloading",
            Status::Verifying { .. } => "verifying",
            Status::Installing { .. } => "installing",
            Status::Downloaded { .. } => "downloaded",
            Status::ReadyToRestart { .. } => "ready",
            Status::Failed { .. } => "failed",
        }
    }

    /// Applies a check result.
    pub fn after_check(&mut self, result: Result<Option<UpdateInfo>, String>) {
        *self = match result {
            Ok(Some(info)) => Status::Available(info),
            Ok(None) => Status::UpToDate,
            Err(message) => Status::Failed {
                message,
                info: None,
                file: None,
            },
        };
    }

    /// Starts an update for the release on offer, or retries a failed download.
    /// Returns the release, or `None` when there is nothing to start.
    pub fn begin_download(&mut self) -> Option<UpdateInfo> {
        let info = match self {
            Status::Available(info) => info.clone(),
            Status::Failed {
                info: Some(info),
                file: None,
                ..
            } => info.clone(),
            _ => return None,
        };
        *self = Status::Downloading {
            info: info.clone(),
            done: 0,
            total: None,
        };
        Some(info)
    }

    /// Applies one event from the update thread. Events that do not fit the
    /// current state are ignored.
    pub fn apply(&mut self, event: DownloadEvent) {
        let current = std::mem::take(self);
        *self = match (current, event) {
            (Status::Downloading { info, .. }, DownloadEvent::Phase(Phase::Progress(done, total))) => {
                Status::Downloading { info, done, total }
            }
            (Status::Downloading { info, .. }, DownloadEvent::Phase(Phase::Verifying)) => {
                Status::Verifying { info }
            }
            (Status::Verifying { info }, DownloadEvent::Phase(Phase::Installing)) => {
                Status::Installing { info }
            }
            (
                Status::Downloading { info, .. } | Status::Verifying { info } | Status::Installing { info },
                DownloadEvent::Finished(result),
            ) => finish(info, result),
            (other, _) => other,
        };
    }

    /// What the About panel shows right now. Exactly one button in every state.
    pub fn panel_view(&self) -> PanelView {
        let plain = |text: String, button: &str, action: Option<Action>| PanelView {
            text,
            danger: false,
            progress: None,
            button: button.to_string(),
            action,
        };
        match self {
            Status::Idle => plain(String::new(), "Check for updates", Some(Action::Check)),
            Status::Checking => plain("Checking for updates…".into(), "Checking…", None),
            Status::UpToDate => plain("You are up to date.".into(), "Check for updates", Some(Action::Check)),
            Status::Available(info) => plain(
                format!("Version {} is available.", info.version),
                "Download & install",
                Some(Action::Download),
            ),
            Status::Downloading { info, done, total } => PanelView {
                text: format!("Downloading {}… {}", info.version, progress_text(*done, *total)),
                danger: false,
                progress: fraction(*done, *total),
                button: "Downloading…".into(),
                action: None,
            },
            Status::Verifying { info } => plain(
                format!("Verifying the signature of {}…", info.version),
                "Verifying…",
                None,
            ),
            Status::Installing { info } => plain(format!("Installing {}…", info.version), "Installing…", None),
            Status::Downloaded { info, path } => plain(
                format!(
                    "Version {} was downloaded to {} and its signature is verified. Open it to install.",
                    info.version,
                    path.display()
                ),
                reveal_label(),
                Some(Action::Reveal),
            ),
            Status::ReadyToRestart { info, .. } => plain(
                format!("Version {} is installed. Restart Zyplus to finish updating.", info.version),
                "Restart to update",
                Some(Action::Restart),
            ),
            Status::Failed { message, info, file } => {
                let (button, action) = if file.is_some() {
                    (reveal_label(), Action::Reveal)
                } else if info.is_some() {
                    ("Retry", Action::Download)
                } else {
                    ("Retry", Action::Check)
                };
                PanelView {
                    text: message.clone(),
                    danger: true,
                    progress: None,
                    button: button.to_string(),
                    action: Some(action),
                }
            }
        }
    }

    /// Label for the sidebar pill, or `None` when no update needs attention.
    /// The header has room for about six characters, so labels stay short;
    /// the About panel has the full wording.
    pub fn pill_label(&self) -> Option<String> {
        Some(match self {
            Status::Available(_) | Status::Downloaded { .. } => "Update".to_string(),
            Status::Downloading { done, total, .. } => match fraction(*done, *total) {
                Some(f) => format!("{}%", (f * 100.0).round() as u32),
                None => "…".to_string(),
            },
            Status::Verifying { .. } | Status::Installing { .. } => "…".to_string(),
            Status::ReadyToRestart { .. } => "Restart".to_string(),
            Status::Failed { info: Some(_), .. } => "Failed".to_string(),
            _ => return None,
        })
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

/// Blocking. Downloads the release, checks its signature, then either installs
/// it over `install` (a macOS `.app` bundle) or leaves it in the downloads
/// folder. A download that fails the signature check is deleted, never opened.
pub fn run_update(
    info: &UpdateInfo,
    current: &str,
    install: Option<&Path>,
    report: &dyn Fn(Phase),
) -> Result<Finished, Failure> {
    let path = download_into(&downloads_dir(), info, &|done, total| {
        report(Phase::Progress(done, total))
    })
    .map_err(|message| Failure { message, file: None })?;

    report(Phase::Verifying);
    let checked = fs::read(&path)
        .map_err(|e| format!("Could not read the download: {e}"))
        .and_then(|bytes| verify_signature(&bytes, &info.signature, &pubkey_b64()));
    if let Err(reason) = checked {
        let _ = fs::remove_file(&path);
        return Err(Failure {
            message: format!("The download was discarded: {reason}. Nothing was installed."),
            file: None,
        });
    }

    let Some(app) = install else {
        return Ok(Finished::Downloaded(path));
    };
    report(Phase::Installing);
    match install_archive(&path, app, current, &running_exe_name()) {
        Ok(()) => Ok(Finished::Installed(app.to_path_buf())),
        Err(e) => Err(Failure {
            message: e.message,
            file: e.reveal.then_some(path),
        }),
    }
}

/// Runs `run_update` on its own thread. Phases and the final result arrive on
/// the returned channel, so the UI can poll it without blocking.
pub fn start_update(info: UpdateInfo, current: &'static str) -> mpsc::Receiver<DownloadEvent> {
    let (tx, rx) = mpsc::channel();
    let install = installable_bundle();
    std::thread::spawn(move || {
        let phase_tx = tx.clone();
        let result = run_update(&info, current, install.as_deref(), &move |phase| {
            let _ = phase_tx.send(DownloadEvent::Phase(phase));
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
  "windows-x86_64": { "signature": "sig", "url": "https://github.com/x/releases/download/v0.3.0/Zyplus_0.3.0_x64_en-US.msi" },
  "windows-x86_64-msi": { "signature": "sig", "url": "https://github.com/x/releases/download/v0.3.0/Zyplus_0.3.0_x64_en-US.msi" },
  "windows-x86_64-nsis": { "signature": "sig", "url": "https://github.com/x/releases/download/v0.3.0/Zyplus_0.3.0_x64-setup.exe" }
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
            r#"{ "windows-x86_64": { "signature": "sig", "url": "https://x/Zyplus_0.3.0_x64_en-US.msi" } }"#,
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
        let body = r#"{ "version": "0.2.4", "platforms": { "darwin-aarch64": { "url": "https://x/y.tar.gz", "signature": "sig" } } }"#;
        let info = parse_manifest(body, CURRENT, Some("darwin-aarch64")).unwrap().unwrap();
        assert_eq!(info.notes, "");
    }

    #[test]
    fn offer_without_signature_is_refused() {
        let body = manifest("0.2.4", r#"{ "darwin-aarch64": { "url": "https://x/y.tar.gz" } }"#);
        let err = parse_manifest(&body, CURRENT, Some("darwin-aarch64")).unwrap_err();
        assert!(err.contains("no signature"), "{err}");
        let blank = manifest("0.2.4", r#"{ "darwin-aarch64": { "url": "https://x/y.tar.gz", "signature": "  " } }"#);
        assert!(parse_manifest(&blank, CURRENT, Some("darwin-aarch64")).is_err());
    }

    #[test]
    fn signature_comes_from_the_same_entry_as_the_url() {
        let body = manifest(
            "0.2.4",
            r#"{ "darwin-aarch64": { "url": "https://x/a.tar.gz", "signature": "from-a" },
                 "darwin-x86_64": { "url": "https://x/b.tar.gz", "signature": "from-b" } }"#,
        );
        let info = parse_manifest(&body, CURRENT, Some("darwin-x86_64")).unwrap().unwrap();
        assert_eq!(info.download_url, "https://x/b.tar.gz");
        assert_eq!(info.signature, "from-b");
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
            signature: "sig".into(),
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
            download_url: "https://example.com/Zyplus.app.tar.gz".to_string(),
            page_url: "https://example.com/release".to_string(),
            signature: "sig".to_string(),
        }
    }

    #[test]
    fn check_result_maps_to_status() {
        let mut status = Status::Checking;
        status.after_check(Ok(Some(info("0.3.0"))));
        assert_eq!(status.info().map(|i| i.version.as_str()), Some("0.3.0"));

        let mut status = Status::Checking;
        status.after_check(Ok(None));
        assert_eq!(status, Status::UpToDate);
        assert!(status.info().is_none());

        let mut status = Status::Checking;
        status.after_check(Err("offline".into()));
        assert_eq!(
            status,
            Status::Failed {
                message: "offline".into(),
                info: None,
                file: None
            }
        );
    }

    #[test]
    fn install_path_runs_through_every_phase() {
        let mut status = Status::Available(info("0.3.0"));
        let started = status.begin_download().expect("offer exists");
        assert_eq!(started.version, "0.3.0");
        assert!(status.busy());
        assert!(status.begin_download().is_none(), "a second click does nothing");

        status.apply(DownloadEvent::Phase(Phase::Progress(512, Some(2048))));
        assert_eq!(
            status,
            Status::Downloading {
                info: info("0.3.0"),
                done: 512,
                total: Some(2048)
            }
        );
        status.apply(DownloadEvent::Phase(Phase::Verifying));
        assert_eq!(status, Status::Verifying { info: info("0.3.0") });
        assert!(status.busy());
        status.apply(DownloadEvent::Phase(Phase::Installing));
        assert_eq!(status, Status::Installing { info: info("0.3.0") });
        status.apply(DownloadEvent::Finished(Ok(Finished::Installed(PathBuf::from(
            "/Applications/Zyplus.app",
        )))));
        assert!(!status.busy());
        assert_eq!(status.installed_app(), Some(Path::new("/Applications/Zyplus.app")));
        assert_eq!(status.info().map(|i| i.version.as_str()), Some("0.3.0"));
    }

    #[test]
    fn download_only_path_reveals_the_file() {
        let mut status = Status::Available(info("0.3.0"));
        status.begin_download();
        status.apply(DownloadEvent::Phase(Phase::Verifying));
        status.apply(DownloadEvent::Finished(Ok(Finished::Downloaded(PathBuf::from(
            "/tmp/Z.tar.gz",
        )))));
        assert_eq!(
            status,
            Status::Downloaded {
                info: info("0.3.0"),
                path: PathBuf::from("/tmp/Z.tar.gz")
            }
        );
        assert!(status.installed_app().is_none());
        assert_eq!(status.panel_view().action, Some(Action::Reveal));
    }

    #[test]
    fn failed_download_keeps_the_offer_for_retry() {
        let mut status = Status::Available(info("0.3.0"));
        status.begin_download();
        status.apply(DownloadEvent::Finished(Err(Failure {
            message: "network dropped".into(),
            file: None,
        })));
        assert_eq!(
            status,
            Status::Failed {
                message: "network dropped".into(),
                info: Some(info("0.3.0")),
                file: None
            }
        );
        assert_eq!(status.panel_view().action, Some(Action::Download));
        assert!(status.begin_download().is_some(), "retry is allowed");
    }

    #[test]
    fn manual_move_failure_offers_reveal_not_retry() {
        let mut status = Status::Available(info("0.3.0"));
        status.begin_download();
        status.apply(DownloadEvent::Phase(Phase::Verifying));
        status.apply(DownloadEvent::Phase(Phase::Installing));
        status.apply(DownloadEvent::Finished(Err(Failure {
            message: "move it by hand".into(),
            file: Some(PathBuf::from("/Users/me/Downloads/Zyplus_universal.app.tar.gz")),
        })));
        let view = status.panel_view();
        assert_eq!(view.action, Some(Action::Reveal));
        assert!(view.danger);
        assert!(status.begin_download().is_none(), "a manual move is not retried");
    }

    #[test]
    fn stray_events_are_ignored() {
        let mut status = Status::Idle;
        status.apply(DownloadEvent::Phase(Phase::Progress(1, None)));
        status.apply(DownloadEvent::Phase(Phase::Verifying));
        status.apply(DownloadEvent::Finished(Ok(Finished::Downloaded(PathBuf::from("/tmp/x")))));
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

    #[test]
    fn fraction_needs_a_known_total() {
        assert_eq!(fraction(50, Some(200)), Some(0.25));
        assert_eq!(fraction(300, Some(200)), Some(1.0));
        assert_eq!(fraction(5, None), None);
        assert_eq!(fraction(5, Some(0)), None);
    }

    #[test]
    fn every_state_shows_exactly_one_button() {
        let states = vec![
            Status::Idle,
            Status::Checking,
            Status::UpToDate,
            Status::Available(info("0.3.0")),
            Status::Downloading {
                info: info("0.3.0"),
                done: 1,
                total: Some(4),
            },
            Status::Verifying { info: info("0.3.0") },
            Status::Installing { info: info("0.3.0") },
            Status::Downloaded {
                info: info("0.3.0"),
                path: PathBuf::from("/tmp/x"),
            },
            Status::ReadyToRestart {
                info: info("0.3.0"),
                app: PathBuf::from("/Applications/Zyplus.app"),
            },
            Status::Failed {
                message: "x".into(),
                info: None,
                file: None,
            },
            Status::Failed {
                message: "x".into(),
                info: Some(info("0.3.0")),
                file: None,
            },
            Status::Failed {
                message: "x".into(),
                info: Some(info("0.3.0")),
                file: Some(PathBuf::from("/tmp/x")),
            },
        ];
        for status in &states {
            let view = status.panel_view();
            assert!(!view.button.is_empty(), "{status:?}");
            // The button is disabled exactly while work runs.
            assert_eq!(view.action.is_none(), status.busy(), "{status:?}");
        }
    }

    #[test]
    fn panel_actions_match_the_state() {
        let failed = |info, file| Status::Failed {
            message: "x".into(),
            info,
            file,
        };
        assert_eq!(Status::Idle.panel_view().action, Some(Action::Check));
        assert_eq!(Status::UpToDate.panel_view().action, Some(Action::Check));
        assert_eq!(
            Status::Available(info("0.3.0")).panel_view().action,
            Some(Action::Download)
        );
        assert_eq!(failed(None, None).panel_view().action, Some(Action::Check));
        assert_eq!(
            failed(Some(info("0.3.0")), None).panel_view().action,
            Some(Action::Download)
        );
        assert_eq!(
            failed(Some(info("0.3.0")), Some(PathBuf::from("/tmp/x")))
                .panel_view()
                .action,
            Some(Action::Reveal)
        );
        let ready = Status::ReadyToRestart {
            info: info("0.3.0"),
            app: PathBuf::from("/Applications/Zyplus.app"),
        };
        assert_eq!(ready.panel_view().action, Some(Action::Restart));
        assert_eq!(ready.panel_view().button, "Restart to update");
    }

    #[test]
    fn pill_labels_follow_the_work() {
        assert_eq!(Status::Idle.pill_label(), None);
        assert_eq!(Status::UpToDate.pill_label(), None);
        assert_eq!(Status::Available(info("0.3.0")).pill_label().as_deref(), Some("Update"));
        let downloading = |done, total| Status::Downloading {
            info: info("0.3.0"),
            done,
            total,
        };
        assert_eq!(downloading(50, Some(200)).pill_label().as_deref(), Some("25%"));
        assert_eq!(downloading(50, None).pill_label().as_deref(), Some("…"));
        assert_eq!(
            Status::Verifying { info: info("0.3.0") }.pill_label().as_deref(),
            Some("…")
        );
        assert_eq!(
            Status::ReadyToRestart {
                info: info("0.3.0"),
                app: PathBuf::from("/Applications/Zyplus.app")
            }
            .pill_label()
            .as_deref(),
            Some("Restart")
        );
        let failed = |info| Status::Failed {
            message: "x".into(),
            info,
            file: None,
        };
        assert_eq!(failed(None).pill_label(), None, "a failed check has no pill");
        assert_eq!(failed(Some(info("0.3.0"))).pill_label().as_deref(), Some("Failed"));
    }

    #[test]
    fn requested_relaunch_can_be_cancelled() {
        request_relaunch(PathBuf::from("/tmp/Zyplus.app"));
        assert!(PENDING_RELAUNCH.lock().unwrap().is_some());
        cancel_relaunch();
        assert!(PENDING_RELAUNCH.lock().unwrap().is_none());
    }
}

#[cfg(test)]
mod signature_tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;
    use std::io::Cursor;

    /// A fresh keypair per test, with the public key encoded the way tauri.conf.json stores it.
    fn keypair() -> (minisign::KeyPair, String) {
        let kp = minisign::KeyPair::generate_unencrypted_keypair().expect("generate keypair");
        let pk_text = kp.pk.to_box().expect("public key box").into_string();
        (kp, STANDARD.encode(pk_text))
    }

    /// Signs `data` as the release pipeline does. Returns the manifest form:
    /// base64 of the `.sig` file text.
    fn sign(kp: &minisign::KeyPair, data: &[u8], trusted: &str) -> String {
        let sig = minisign::sign(None, &kp.sk, Cursor::new(data), Some(trusted), None)
            .expect("sign");
        STANDARD.encode(sig.into_string())
    }

    const DATA: &[u8] = b"zyplus release bytes";
    const TRUSTED: &str = "timestamp:1700000000\tfile:Zyplus_universal.app.tar.gz";

    #[test]
    fn embedded_key_is_the_tauri_updater_key() {
        let text = decode_b64_text(PUBKEY_B64).unwrap();
        assert!(
            text.starts_with("untrusted comment: minisign public key: 3D04398FDFF404D0\n"),
            "{text}"
        );
        assert!(minisign_verify::PublicKey::decode(&text).is_ok());
    }

    #[test]
    fn valid_signature_verifies() {
        let (kp, pk) = keypair();
        let sig = sign(&kp, DATA, TRUSTED);
        assert_eq!(verify_signature(DATA, &sig, &pk), Ok(()));
    }

    #[test]
    fn tampered_content_is_refused() {
        let (kp, pk) = keypair();
        let sig = sign(&kp, DATA, TRUSTED);
        let mut bad = DATA.to_vec();
        bad[0] ^= 1;
        let err = verify_signature(&bad, &sig, &pk).unwrap_err();
        assert!(err.contains("does not match"), "{err}");
    }

    #[test]
    fn wrong_key_is_refused() {
        let (kp, _) = keypair();
        let (_, other_pk) = keypair();
        let sig = sign(&kp, DATA, TRUSTED);
        assert!(verify_signature(DATA, &sig, &other_pk).is_err());
    }

    #[test]
    fn truncated_signature_is_refused() {
        let (kp, pk) = keypair();
        let text = String::from_utf8(STANDARD.decode(sign(&kp, DATA, TRUSTED)).unwrap()).unwrap();
        for cut in [30, text.len() / 2, text.len() - 20] {
            let truncated = STANDARD.encode(&text[..cut]);
            assert!(verify_signature(DATA, &truncated, &pk).is_err(), "cut at {cut}");
        }
        assert!(verify_signature(DATA, "%%% not base64 %%%", &pk).is_err());
    }

    #[test]
    fn trusted_comment_tamper_is_refused() {
        let (kp, pk) = keypair();
        let text = String::from_utf8(STANDARD.decode(sign(&kp, DATA, TRUSTED)).unwrap()).unwrap();
        let tampered = text.replace(TRUSTED, "timestamp:1700000000\tfile:other.tar.gz");
        assert_ne!(tampered, text, "the fixture must contain the trusted comment");
        assert!(
            verify_signature(DATA, &STANDARD.encode(tampered), &pk).is_err(),
            "a changed trusted comment must fail the global signature"
        );
    }

    #[test]
    fn invalid_public_key_is_refused() {
        let (kp, _) = keypair();
        let sig = sign(&kp, DATA, TRUSTED);
        let err = verify_signature(DATA, &sig, &STANDARD.encode("not a key")).unwrap_err();
        assert!(err.contains("public key is invalid"), "{err}");
    }
}

#[cfg(test)]
mod bundle_tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::{symlink, PermissionsExt};

    const PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleExecutable</key>
	<string>zyplus-gpui</string>
	<key>CFBundleIdentifier</key>
	<string>com.fankrits.zyplus-editor</string>
	<key>CFBundleShortVersionString</key>
	<string>0.2.4</string>
	<key>CFBundleVersion</key>
	<string>0.2.4</string>
</dict>
</plist>"#;

    /// A fresh scratch directory for one test.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("zyplus-updater-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn plist_values_are_read() {
        assert_eq!(
            plist_string(PLIST, "CFBundleIdentifier").as_deref(),
            Some("com.fankrits.zyplus-editor")
        );
        assert_eq!(plist_string(PLIST, "CFBundleShortVersionString").as_deref(), Some("0.2.4"));
        assert_eq!(plist_string(PLIST, "Missing"), None);
    }

    #[test]
    fn valid_bundle_passes() {
        let facts = validate_bundle_plist(PLIST, "0.2.3", "zyplus-gpui").unwrap();
        assert_eq!(facts.version, "0.2.4");
        assert_eq!(facts.executable, "zyplus-gpui");
    }

    #[test]
    fn wrong_identifier_is_refused() {
        let xml = PLIST.replace("com.fankrits.zyplus-editor", "com.example.other");
        let err = validate_bundle_plist(&xml, "0.2.3", "zyplus-gpui").unwrap_err();
        assert!(err.contains("com.example.other"), "{err}");
    }

    #[test]
    fn same_or_older_version_is_refused() {
        let err = validate_bundle_plist(PLIST, "0.2.4", "zyplus-gpui").unwrap_err();
        assert!(err.contains("not newer"), "{err}");
        assert!(validate_bundle_plist(PLIST, "0.3.0", "zyplus-gpui").is_err());
    }

    #[test]
    fn different_executable_is_refused() {
        // A Tauri build must not replace the gpui build.
        let err = validate_bundle_plist(PLIST, "0.2.3", "zyplus-editor").unwrap_err();
        assert!(err.contains("different app"), "{err}");
    }

    #[test]
    fn version_falls_back_to_cfbundleversion() {
        let xml = PLIST.replace("CFBundleShortVersionString", "Other");
        assert_eq!(validate_bundle_plist(&xml, "0.2.3", "zyplus-gpui").unwrap().version, "0.2.4");
    }

    #[test]
    fn missing_version_is_refused() {
        let xml = PLIST
            .replace("CFBundleShortVersionString", "X")
            .replace("CFBundleVersion", "Y");
        assert!(validate_bundle_plist(&xml, "0.2.3", "zyplus-gpui").is_err());
    }

    #[test]
    fn bundle_is_found_only_for_app_executables() {
        assert_eq!(
            bundle_for_exe(Path::new("/Applications/Zyplus.app/Contents/MacOS/zyplus-gpui")),
            Some(PathBuf::from("/Applications/Zyplus.app"))
        );
        assert_eq!(bundle_for_exe(Path::new("/Users/me/dev/gpui/target/debug/zyplus-gpui")), None);
        assert_eq!(bundle_for_exe(Path::new("/tmp/Zyplus/Contents/MacOS/zyplus-gpui")), None);
        assert_eq!(
            bundle_for_exe(Path::new("/Applications/Zyplus.app/Contents/Resources/zyplus-gpui")),
            None
        );
        assert_eq!(bundle_for_exe(Path::new("zyplus-gpui")), None);
    }

    #[test]
    fn archive_entries_must_stay_under_the_app() {
        assert!(check_archive_entries(&[
            "Zyplus.app/",
            "Zyplus.app/Contents/Info.plist",
            "./Zyplus.app/Contents/MacOS/zyplus-gpui",
        ])
        .is_ok());
        for bad in [
            vec!["../Zyplus.app/x"],
            vec!["/etc/passwd"],
            vec!["Zyplus.app/../../etc/x"],
            vec!["Other.app/Contents/Info.plist"],
            vec!["Zyplus.app-evil/x"],
            vec!["Zyplus.app/ok", "README"],
            vec![],
        ] {
            assert!(check_archive_entries(&bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[cfg(unix)]
    #[test]
    fn links_inside_the_bundle_are_allowed() {
        let root = scratch("links-ok");
        fs::create_dir_all(root.join("Contents/Frameworks/A")).unwrap();
        symlink("A", root.join("Contents/Frameworks/Current")).unwrap();
        symlink("Frameworks/A", root.join("Contents/alias")).unwrap();
        assert!(check_links_stay_inside(&root).is_ok());
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn links_that_leave_the_bundle_are_refused() {
        for (name, target) in [
            ("abs", "/etc"),
            ("climb", "../../../../etc/passwd"),
            ("mid", "sub/../../x"),
        ] {
            let root = scratch(&format!("links-{name}"));
            fs::create_dir_all(root.join("Contents")).unwrap();
            symlink(target, root.join("Contents/evil")).unwrap();
            assert!(check_links_stay_inside(&root).is_err(), "{target} should be refused");
            let _ = fs::remove_dir_all(&root);
        }
    }

    fn fake_bundle(dir: &Path, marker: &str) -> PathBuf {
        fs::create_dir_all(dir.join("Contents")).unwrap();
        fs::write(dir.join("Contents/marker"), marker).unwrap();
        dir.to_path_buf()
    }

    fn marker(app: &Path) -> String {
        fs::read_to_string(app.join("Contents/marker")).unwrap()
    }

    #[test]
    fn replace_swaps_the_bundle() {
        let root = scratch("replace-ok");
        let app = fake_bundle(&root.join("Zyplus.app"), "old");
        let new = fake_bundle(&root.join("new/Zyplus.app"), "new");
        let backup = root.join("Zyplus.app.old-1");
        replace_bundle(&app, &new, &backup).unwrap();
        assert_eq!(marker(&app), "new");
        assert_eq!(marker(&backup), "old");
        assert!(!new.exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn failed_move_rolls_back_to_the_old_bundle() {
        let root = scratch("replace-rollback");
        let app = fake_bundle(&root.join("Zyplus.app"), "old");
        let missing_new = root.join("does-not-exist/Zyplus.app");
        let backup = root.join("Zyplus.app.old-2");
        let err = replace_bundle(&app, &missing_new, &backup).unwrap_err();
        assert!(err.message.contains("previous version was restored"), "{}", err.message);
        assert!(!err.reveal);
        assert_eq!(marker(&app), "old", "the old bundle is back in place");
        assert!(!backup.exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn existing_backup_is_never_overwritten() {
        let root = scratch("replace-backup");
        let app = fake_bundle(&root.join("Zyplus.app"), "old");
        let new = fake_bundle(&root.join("new/Zyplus.app"), "new");
        let backup = fake_bundle(&root.join("Zyplus.app.old-3"), "keep");
        assert!(replace_bundle(&app, &new, &backup).is_err());
        assert_eq!(marker(&app), "old");
        assert_eq!(marker(&backup), "keep");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_app_changes_nothing() {
        let root = scratch("replace-noapp");
        let new = fake_bundle(&root.join("new/Zyplus.app"), "new");
        let err = replace_bundle(&root.join("Zyplus.app"), &new, &root.join("b")).unwrap_err();
        assert!(!err.reveal);
        assert_eq!(marker(&new), "new", "the new bundle is left where it was");
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn permission_denied_asks_for_a_manual_move() {
        let root = scratch("replace-perm");
        let parent = root.join("locked");
        let app = fake_bundle(&parent.join("Zyplus.app"), "old");
        let new = fake_bundle(&root.join("new/Zyplus.app"), "new");
        let set_mode = |mode: u32| {
            let mut perms = fs::metadata(&parent).unwrap().permissions();
            perms.set_mode(mode);
            fs::set_permissions(&parent, perms).unwrap();
        };
        set_mode(0o555);
        // Root ignores directory permissions, so the failure cannot be produced there.
        if fs::write(parent.join("probe"), b"").is_ok() {
            set_mode(0o755);
            eprintln!("skipped: the test needs a non-root user");
            let _ = fs::remove_dir_all(&root);
            return;
        }
        let result = replace_bundle(&app, &new, &parent.join("b"));
        set_mode(0o755);
        let err = result.unwrap_err();
        assert!(err.reveal, "{}", err.message);
        assert!(err.message.contains("by hand"), "{}", err.message);
        assert_eq!(marker(&app), "old");
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(target_os = "macos")]
    mod install {
        use super::*;

        /// Packs a `<name>.app` folder with the given Info.plist into `<root>/<archive>`.
        fn pack(root: &Path, archive: &str, app_name: &str, plist: &str, exe: &str) -> PathBuf {
            let src = root.join(format!("pack-{archive}"));
            let app = src.join(app_name);
            fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
            fs::write(app.join("Contents/Info.plist"), plist).unwrap();
            fs::write(app.join("Contents/MacOS").join(exe), b"#!/bin/sh\n").unwrap();
            let out = root.join(archive);
            let status = Command::new("/usr/bin/tar")
                .arg("-czf")
                .arg(&out)
                .arg("-C")
                .arg(&src)
                .arg(app_name)
                .status()
                .expect("run tar");
            assert!(status.success());
            out
        }

        fn installed_app(root: &Path) -> PathBuf {
            let app = root.join("Apps/Zyplus.app");
            fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
            fs::write(app.join("Contents/Info.plist"), PLIST.replace("0.2.4", "0.2.3")).unwrap();
            fs::write(app.join("Contents/MacOS/zyplus-gpui"), b"old").unwrap();
            app
        }

        fn leftovers(parent: &Path) -> Vec<String> {
            fs::read_dir(parent)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|n| n.starts_with(".zyplus-update-") || n.contains(".old-"))
                .collect()
        }

        #[test]
        fn valid_archive_replaces_the_app() {
            let root = scratch("install-ok");
            let app = installed_app(&root);
            let archive = pack(&root, "update.tar.gz", "Zyplus.app", PLIST, "zyplus-gpui");
            install_archive(&archive, &app, "0.2.3", "zyplus-gpui").expect("install");
            let plist = fs::read_to_string(app.join("Contents/Info.plist")).unwrap();
            assert!(plist.contains("<string>0.2.4</string>"), "{plist}");
            assert!(app.join("Contents/MacOS/zyplus-gpui").is_file());
            assert_eq!(leftovers(app.parent().unwrap()), Vec::<String>::new());
            let _ = fs::remove_dir_all(&root);
        }

        #[test]
        fn wrong_identifier_leaves_the_app_alone() {
            let root = scratch("install-id");
            let app = installed_app(&root);
            let plist = PLIST.replace("com.fankrits.zyplus-editor", "com.example.other");
            let archive = pack(&root, "update.tar.gz", "Zyplus.app", &plist, "zyplus-gpui");
            let err = install_archive(&archive, &app, "0.2.3", "zyplus-gpui").unwrap_err();
            assert!(err.message.contains("com.example.other"), "{}", err.message);
            assert_eq!(fs::read(app.join("Contents/MacOS/zyplus-gpui")).unwrap(), b"old");
            assert_eq!(leftovers(app.parent().unwrap()), Vec::<String>::new());
            let _ = fs::remove_dir_all(&root);
        }

        #[test]
        fn foreign_archive_entries_are_refused_before_extraction() {
            let root = scratch("install-foreign");
            let app = installed_app(&root);
            let archive = pack(&root, "update.tar.gz", "Evil.app", PLIST, "zyplus-gpui");
            let err = install_archive(&archive, &app, "0.2.3", "zyplus-gpui").unwrap_err();
            assert!(err.message.contains("unexpected entry"), "{}", err.message);
            assert_eq!(fs::read(app.join("Contents/MacOS/zyplus-gpui")).unwrap(), b"old");
            let _ = fs::remove_dir_all(&root);
        }

        #[test]
        fn same_version_archive_is_refused() {
            let root = scratch("install-same");
            let app = installed_app(&root);
            let archive = pack(&root, "update.tar.gz", "Zyplus.app", PLIST, "zyplus-gpui");
            let err = install_archive(&archive, &app, "0.2.4", "zyplus-gpui").unwrap_err();
            assert!(err.message.contains("not newer"), "{}", err.message);
            let _ = fs::remove_dir_all(&root);
        }
    }
}
