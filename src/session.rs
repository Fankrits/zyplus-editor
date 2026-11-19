//! The workspace session, saved beside the notes folder.
//!
//! Stored in `<app data>/Zyplus-session/session.json`. Only paths, modes and
//! preferences are kept — never document content, which is already on disk.
//!
//! It used to sit at `<app data>/Zyplus/session.json`, which is also the notes
//! folder, so the file showed up in the sidebar. `migrate_in` moves it once.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersistedTab {
    pub file_path: String,
    #[serde(default = "default_mode")]
    pub mode: String,
}

fn default_mode() -> String {
    "plain".to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Session {
    pub roots: Vec<String>,
    pub default_folder: Option<String>,
    pub tabs: Vec<PersistedTab>,
    pub active_file_path: Option<String>,
    pub sidebar_collapsed: bool,
    /// Write dirty tabs shortly after typing stops.
    pub autosave: bool,
    /// Folders open in the sidebar, roots included.
    pub open_folders: Vec<String>,
    /// A [`crate::theme::THEMES`] id; "system" follows the OS appearance.
    pub theme: String,
    /// Built-in extensions the user turned off (ids: json, csv, chart, codefiles).
    pub disabled_extensions: Vec<String>,
}

/// Where the session lives under the app data directory `base`.
pub fn path_in(base: &Path) -> PathBuf {
    base.join("Zyplus-session").join("session.json")
}

/// Where older builds kept it: inside the notes folder.
fn legacy_path_in(base: &Path) -> PathBuf {
    base.join("Zyplus").join("session.json")
}

fn session_path() -> Option<PathBuf> {
    crate::paths::app_data_dir().map(|base| path_in(&base))
}

/// Moves a session written by an older build out of the notes folder. The new
/// file wins when both exist, and the old one is removed either way so it
/// stops appearing in the sidebar. Returns whether a file was moved.
pub fn migrate_in(base: &Path) -> io::Result<bool> {
    let old = legacy_path_in(base);
    if !old.is_file() {
        return Ok(false);
    }
    let new = path_in(base);
    if new.exists() {
        fs::remove_file(&old)?;
        return Ok(false);
    }
    if let Some(parent) = new.parent() {
        fs::create_dir_all(parent)?;
    }
    if fs::rename(&old, &new).is_err() {
        fs::copy(&old, &new)?;
        fs::remove_file(&old)?;
    }
    Ok(true)
}

/// Reads the session at `path`. `Ok(None)` when there is no file yet; `Err`
/// when the file exists but cannot be read or parsed, so the caller can keep
/// it instead of overwriting it.
fn read_path(path: &Path) -> Result<Option<Session>, String> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|err| err.to_string()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.to_string()),
    }
}

pub fn load() -> Result<Option<Session>, String> {
    let Some(base) = crate::paths::app_data_dir() else {
        return Ok(None);
    };
    if let Err(err) = migrate_in(&base) {
        eprintln!("zyplus: could not move the old session file: {err}");
    }
    read_path(&path_in(&base))
}

pub fn save(session: &Session) -> Result<(), String> {
    let path = session_path().ok_or("Could not find the application data folder")?;
    save_to(&path, session)
}

/// Writes the session to `path` atomically (see `tree::write_atomic`), so a
/// crash mid-save never leaves a truncated session file behind.
fn save_to(path: &Path, session: &Session) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let json = serde_json::to_string_pretty(session).map_err(|err| err.to_string())?;
    crate::tree::write_atomic(path, json.as_bytes()).map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("zy-session-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_session_in_the_notes_folder_moves_out_of_it() {
        let base = scratch("move");
        let old = legacy_path_in(&base);
        fs::create_dir_all(old.parent().unwrap()).unwrap();
        fs::write(&old, r#"{"roots":["/a"]}"#).unwrap();

        assert!(migrate_in(&base).unwrap());
        assert!(!old.exists(), "the notes folder must not keep the session");
        let moved = read_path(&path_in(&base)).unwrap().unwrap();
        assert_eq!(moved.roots, vec!["/a".to_string()]);

        // Nothing left to move the second time.
        assert!(!migrate_in(&base).unwrap());
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn an_existing_new_session_wins_over_the_old_one() {
        let base = scratch("both");
        let old = legacy_path_in(&base);
        fs::create_dir_all(old.parent().unwrap()).unwrap();
        fs::write(&old, r#"{"roots":["/old"]}"#).unwrap();
        let new = path_in(&base);
        fs::create_dir_all(new.parent().unwrap()).unwrap();
        fs::write(&new, r#"{"roots":["/new"]}"#).unwrap();

        assert!(!migrate_in(&base).unwrap());
        assert!(!old.exists());
        let kept = read_path(&new).unwrap().unwrap();
        assert_eq!(kept.roots, vec!["/new".to_string()]);
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn no_session_file_is_not_an_error_but_a_broken_one_is() {
        let base = scratch("missing");
        assert!(read_path(&path_in(&base)).unwrap().is_none());

        let path = path_in(&base);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{ not json").unwrap();
        assert!(read_path(&path).is_err());
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn save_replaces_the_file_atomically_and_leaves_no_temp_file() {
        let base = scratch("save");
        let path = path_in(&base);
        let first = Session {
            roots: vec!["/one".into()],
            theme: "dark".into(),
            ..Session::default()
        };
        save_to(&path, &first).unwrap();
        let second = Session {
            roots: vec!["/two".into()],
            theme: "latte".into(),
            ..Session::default()
        };
        save_to(&path, &second).unwrap();

        let kept = read_path(&path).unwrap().unwrap();
        assert_eq!(kept.roots, vec!["/two".to_string()]);
        assert_eq!(kept.theme, "latte");
        let names: Vec<String> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["session.json".to_string()]);
        fs::remove_dir_all(&base).ok();
    }
}
