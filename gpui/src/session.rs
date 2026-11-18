//! The workspace session, saved beside the notes folder.
//!
//! The web build keeps this in `localStorage`; a native app has no such store,
//! so it lives in `<app data>/Zyplus/session.json`. Only paths, modes and
//! preferences are kept — never document content, which is already on disk.

use std::fs;
use std::path::PathBuf;

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

fn session_path() -> Option<PathBuf> {
    crate::paths::app_data_dir().map(|base| base.join("Zyplus").join("session.json"))
}

pub fn load() -> Option<Session> {
    let text = fs::read_to_string(session_path()?).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn save(session: &Session) -> Result<(), String> {
    let path = session_path().ok_or("Could not find the application data folder")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let json = serde_json::to_string_pretty(session).map_err(|err| err.to_string())?;
    fs::write(&path, json).map_err(|err| err.to_string())
}
