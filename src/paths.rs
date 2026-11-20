//! Path string helpers.
//!
//! These stay free of GPUI so they can be tested without opening a window.
//! Windows' filename rules are enforced on every platform, because a note
//! created on a Mac with `a:b.md` cannot be copied to a Windows machine at all.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use crate::platform::Os;

pub const MARKDOWN_EXTENSIONS: [&str; 3] = [".md", ".markdown", ".txt"];

/// Files a tab can be opened for without an extension's editor.
pub fn is_markdown_file(name: &str) -> bool {
    let lower = name.to_lowercase();
    MARKDOWN_EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
}

/// Last segment of a path, either separator. An empty trailing segment falls
/// back to the whole path, matching `basenameOf`.
pub fn basename_of(path: &str) -> String {
    match path.rsplit(['/', '\\']).next() {
        Some(last) if !last.is_empty() => last.to_string(),
        Some(_) => path.to_string(),
        None => path.to_string(),
    }
}

/// Parent of a path, or `/` for a path with no parent.
pub fn dirname_of(path: &str) -> String {
    match Path::new(path).parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_string_lossy().into_owned(),
        _ => "/".to_string(),
    }
}

/// Joins a parent and one child name using the separator the parent already
/// uses: Windows hands back `C:\Users\me\Documents`, and pasting a `/` onto that
/// renders a path the user has never seen.
pub fn display_join(parent: &str, name: &str) -> String {
    let sep = if parent.contains('\\') && !parent.contains('/') {
        '\\'
    } else {
        '/'
    };
    if parent.ends_with(sep) {
        format!("{parent}{name}")
    } else {
        format!("{parent}{sep}{name}")
    }
}

/// Windows' rules, enforced on every platform.
const RESERVED_ON_WINDOWS: [&str; 9] = [
    "con", "prn", "aux", "nul", "com1", "lpt1", "com2", "lpt2", "com3",
];

/// Why `name` cannot be a filename, or `None` if it can be.
pub fn invalid_name_reason(name: &str) -> Option<String> {
    if name.is_empty() {
        return Some("Enter a name.".into());
    }
    if name == "." || name == ".." {
        return Some("Choose a different name.".into());
    }
    if name.contains(['\\', '/']) {
        return Some("A name cannot contain \\ or /.".into());
    }
    if name.contains(['<', '>', ':', '"', '|', '?', '*']) {
        return Some("A name cannot contain < > : \" | ? *".into());
    }
    if name.chars().any(|c| c.is_control()) {
        return Some("A name cannot contain control characters.".into());
    }
    if name.ends_with('.') {
        return Some("A name cannot end with a period.".into());
    }
    if name.ends_with(' ') {
        return Some("A name cannot end with a space.".into());
    }
    // `con`, `con.txt`, `COM1.md`, ...
    let stem = name.split('.').next().unwrap_or(name);
    let stem_lower = stem.to_lowercase();
    let reserved = RESERVED_ON_WINDOWS.contains(&stem_lower.as_str())
        || (stem_lower.len() == 4
            && (stem_lower.starts_with("com") || stem_lower.starts_with("lpt"))
            && stem_lower.as_bytes()[3].is_ascii_digit()
            && stem_lower.as_bytes()[3] != b'0');
    if reserved {
        return Some(format!("\"{name}\" is a name Windows reserves."));
    }
    if name.len() > 255 {
        return Some("That name is too long.".into());
    }
    None
}

/// `path` is `prefix` itself or something inside it.
pub fn is_under(path: &str, prefix: &str) -> bool {
    if path == prefix {
        return true;
    }
    if prefix.ends_with('/') || prefix.ends_with('\\') {
        return path.starts_with(prefix);
    }
    path.starts_with(&format!("{prefix}/")) || path.starts_with(&format!("{prefix}\\"))
}

/// `title` with its extension swapped for `ext` (".md", ".pdf"). A leading dot
/// is part of the name, not an extension: ".gitignore" becomes ".gitignore.md".
pub fn with_extension(title: &str, ext: &str) -> String {
    match title.rfind('.') {
        Some(dot) if dot > 0 => format!("{}{ext}", &title[..dot]),
        _ => format!("{title}{ext}"),
    }
}

/// Whether a path exists, for free-path selection.
pub fn path_exists(path: &str) -> bool {
    Path::new(path).exists()
}

/// `dir/name`, or `dir/name<label> 2`, `… 3` and so on until nothing is there.
pub fn free_path(dir: &str, name: &str, label: &str) -> String {
    let dot = name.rfind('.');
    let (stem, ext) = match dot {
        Some(dot) if dot > 0 => (&name[..dot], &name[dot..]),
        _ => (name, ""),
    };
    let mut path = display_join(dir, &format!("{stem}{label}{ext}"));
    let mut n = 2;
    while path_exists(&path) {
        path = display_join(dir, &format!("{stem}{label} {n}{ext}"));
        n += 1;
    }
    path
}

/// The notes folder: `<app data>/Zyplus`, with no OS-guarded parent on the way.
pub fn default_folder_path() -> Option<PathBuf> {
    default_folder_path_for(Os::current(), &|key| std::env::var_os(key))
}

/// This platform's per-user application data directory.
pub fn app_data_dir() -> Option<PathBuf> {
    app_data_dir_for(Os::current(), &|key| std::env::var_os(key))
}

/// This platform's per-user home directory.
pub fn home_dir() -> Option<PathBuf> {
    home_for(Os::current(), &|key| std::env::var_os(key))
}

fn non_empty_env(get: &impl Fn(&str) -> Option<OsString>, key: &str) -> Option<OsString> {
    get(key).filter(|v| !v.is_empty())
}

/// XDG Base Directory Specification: relative paths in `XDG_*` variables are
/// invalid and must be ignored. Checked by leading `/` so Linux rules can be
/// unit-tested on any host OS.
fn is_posix_absolute(path: &OsStr) -> bool {
    path.as_encoded_bytes().starts_with(b"/")
}

fn home_for(os: Os, get: &impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    match os {
        Os::Mac | Os::Linux => non_empty_env(get, "HOME").map(PathBuf::from),
        Os::Windows => non_empty_env(get, "USERPROFILE")
            .map(PathBuf::from)
            .or_else(|| {
                let mut drive = non_empty_env(get, "HOMEDRIVE")?;
                let path = non_empty_env(get, "HOMEPATH")?;
                drive.push(path);
                Some(PathBuf::from(drive))
            }),
    }
}

fn app_data_dir_for(os: Os, get: &impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    match os {
        Os::Mac => home_for(Os::Mac, get).map(|home| home.join("Library/Application Support")),
        Os::Windows => non_empty_env(get, "APPDATA")
            .map(PathBuf::from)
            .or_else(|| {
                home_for(Os::Windows, get).map(|home| home.join("AppData").join("Roaming"))
            }),
        Os::Linux => non_empty_env(get, "XDG_DATA_HOME")
            .filter(|v| is_posix_absolute(v))
            .map(PathBuf::from)
            .or_else(|| home_for(Os::Linux, get).map(|home| home.join(".local/share"))),
    }
}

fn default_folder_path_for(os: Os, get: &impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    app_data_dir_for(os, get).map(|base| base.join("Zyplus"))
}

/// Turns a raw OS error into something the user can act on.
pub fn explain_fs_error(err: &str, path: &str) -> String {
    explain_fs_error_for(err, path, Os::current())
}

fn explain_fs_error_for(err: &str, path: &str, os: Os) -> String {
    let denied = [
        "os error 5",
        "EACCES",
        "EPERM",
        "permission denied",
        "access is denied",
    ]
    .iter()
    .any(|needle| err.to_lowercase().contains(&needle.to_lowercase()));
    if !denied {
        return err.to_string();
    }
    match os {
        Os::Windows => format!(
            "Windows blocked the change to \"{path}\".\n\n\
             This is usually Controlled folder access, which stops apps it does not \
             recognize from writing to Documents and Desktop.\n\n\
             Either choose a different location, or allow Zyplus: Windows Security → \
             Virus & threat protection → Ransomware protection → Allow an app through \
             Controlled folder access."
        ),
        Os::Mac => format!(
            "macOS blocked the change to \"{path}\".\n\n\
             Zyplus does not have permission for this folder. Either choose a different \
             location, or grant access in System Settings → Privacy & Security → Files \
             and Folders."
        ),
        Os::Linux => format!(
            "\"{path}\" is not writable.\n\nCheck the folder's permissions, or choose a different location."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basenames_and_dirnames() {
        assert_eq!(basename_of("/notes/a.md"), "a.md");
        assert_eq!(basename_of(r"C:\notes\a.md"), "a.md");
        assert_eq!(basename_of("/notes/"), "/notes/");
        assert_eq!(dirname_of("/notes/a.md"), "/notes");
        assert_eq!(dirname_of("a.md"), "/");
    }

    #[test]
    fn joins_with_the_separator_the_parent_uses() {
        assert_eq!(display_join("/notes", "a.md"), "/notes/a.md");
        assert_eq!(display_join("/notes/", "a.md"), "/notes/a.md");
        assert_eq!(display_join(r"C:\notes", "a.md"), r"C:\notes\a.md");
    }

    #[test]
    fn rejects_names_the_filesystem_cannot_hold() {
        assert_eq!(invalid_name_reason("").as_deref(), Some("Enter a name."));
        assert!(invalid_name_reason("..").is_some());
        assert!(invalid_name_reason("a/b").is_some());
        assert!(invalid_name_reason("a:b").is_some());
        assert!(invalid_name_reason("trailing.").is_some());
        assert!(invalid_name_reason("trailing ").is_some());
        assert!(invalid_name_reason("CON").is_some());
        assert!(invalid_name_reason("com1.md").is_some());
        assert!(invalid_name_reason("nul.txt").is_some());
        assert!(invalid_name_reason(&"x".repeat(256)).is_some());
        assert_eq!(invalid_name_reason("notes 2.md"), None);
        assert_eq!(invalid_name_reason("console.log"), None);
        assert_eq!(invalid_name_reason(".gitignore"), None);
    }

    #[test]
    fn under_uses_path_boundaries_not_prefixes() {
        assert!(is_under("/notes/a.md", "/notes"));
        assert!(is_under("/notes", "/notes"));
        assert!(!is_under("/notes2/a.md", "/notes"));
        assert!(is_under("/a.md", "/"));
        assert!(is_under(r"C:\notes\a.md", r"C:\notes"));
        assert!(!is_under(r"C:\notes2\a.md", r"C:\notes"));
        assert!(is_under(r"C:\a.md", r"C:\"));
    }

    #[test]
    fn extension_swaps_but_leading_dots_are_part_of_the_name() {
        assert_eq!(with_extension("a.md", ".md"), "a.md");
        assert_eq!(with_extension("a", ".md"), "a.md");
        assert_eq!(with_extension(".gitignore", ".md"), ".gitignore.md");
    }

    #[test]
    fn markdown_is_recognised_case_insensitively() {
        assert!(is_markdown_file("A.MD"));
        assert!(is_markdown_file("notes.txt"));
        assert!(is_markdown_file("x.markdown"));
        assert!(!is_markdown_file("data.json"));
    }

    fn env_map(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let owned: Vec<(String, OsString)> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), OsString::from(v)))
            .collect();
        move |key| owned.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    }

    #[test]
    fn home_resolves_per_platform() {
        assert_eq!(
            home_for(Os::Mac, &env_map(&[("HOME", "/Users/me")])),
            Some(PathBuf::from("/Users/me"))
        );
        assert_eq!(home_for(Os::Mac, &env_map(&[("HOME", "")])), None);

        assert_eq!(
            home_for(Os::Linux, &env_map(&[("HOME", "/home/me")])),
            Some(PathBuf::from("/home/me"))
        );

        assert_eq!(
            home_for(
                Os::Windows,
                &env_map(&[
                    ("USERPROFILE", r"C:\Users\me"),
                    ("HOMEDRIVE", "D:"),
                    ("HOMEPATH", r"\fallback"),
                ])
            ),
            Some(PathBuf::from(r"C:\Users\me"))
        );
        assert_eq!(
            home_for(
                Os::Windows,
                &env_map(&[
                    ("USERPROFILE", ""),
                    ("HOMEDRIVE", "C:"),
                    ("HOMEPATH", r"\Users\me"),
                ])
            ),
            Some(PathBuf::from(r"C:\Users\me"))
        );
        assert_eq!(
            home_for(Os::Windows, &env_map(&[("HOMEDRIVE", "C:")])),
            None
        );
        assert_eq!(
            home_for(Os::Windows, &env_map(&[("HOME", "/ignored")])),
            None
        );
    }

    #[test]
    fn app_data_and_default_folder_resolve_per_platform() {
        let mac_env = env_map(&[("HOME", "/Users/me")]);
        assert_eq!(
            app_data_dir_for(Os::Mac, &mac_env),
            Some(PathBuf::from("/Users/me/Library/Application Support"))
        );
        assert_eq!(
            default_folder_path_for(Os::Mac, &mac_env),
            Some(PathBuf::from(
                "/Users/me/Library/Application Support/Zyplus"
            ))
        );

        let win_appdata = env_map(&[
            ("APPDATA", r"C:\Users\me\AppData\Roaming"),
            ("USERPROFILE", r"C:\Users\other"),
        ]);
        assert_eq!(
            app_data_dir_for(Os::Windows, &win_appdata),
            Some(PathBuf::from(r"C:\Users\me\AppData\Roaming"))
        );
        assert_eq!(
            default_folder_path_for(Os::Windows, &win_appdata),
            Some(PathBuf::from(r"C:\Users\me\AppData\Roaming").join("Zyplus"))
        );

        // Windows falls back to <USERPROFILE>/AppData/Roaming when APPDATA is unset.
        let win_profile = env_map(&[("APPDATA", ""), ("USERPROFILE", r"C:\Users\me")]);
        assert_eq!(
            app_data_dir_for(Os::Windows, &win_profile),
            Some(
                PathBuf::from(r"C:\Users\me")
                    .join("AppData")
                    .join("Roaming")
            )
        );

        // Linux honours XDG_DATA_HOME only when absolute (XDG spec).
        let linux_xdg = env_map(&[("XDG_DATA_HOME", "/srv/data"), ("HOME", "/home/me")]);
        assert_eq!(
            app_data_dir_for(Os::Linux, &linux_xdg),
            Some(PathBuf::from("/srv/data"))
        );
        assert_eq!(
            default_folder_path_for(Os::Linux, &linux_xdg),
            Some(PathBuf::from("/srv/data/Zyplus"))
        );

        for bad_xdg in ["", "relative/data", "./data", "../data"] {
            let env = env_map(&[("XDG_DATA_HOME", bad_xdg), ("HOME", "/home/me")]);
            assert_eq!(
                app_data_dir_for(Os::Linux, &env),
                Some(PathBuf::from("/home/me/.local/share")),
                "relative or empty XDG_DATA_HOME={bad_xdg:?} must fall back to ~/.local/share"
            );
        }
        assert_eq!(
            app_data_dir_for(Os::Linux, &env_map(&[("XDG_DATA_HOME", "relative")])),
            None
        );
    }

    #[test]
    fn explain_fs_error_tailors_permission_messages_per_os() {
        assert!(
            explain_fs_error_for("Permission denied (os error 13)", "/a", Os::Mac)
                .contains("System Settings")
        );
        assert!(
            explain_fs_error_for("Access is denied. (os error 5)", r"C:\a", Os::Windows)
                .contains("Controlled folder access")
        );
        assert!(explain_fs_error_for("EACCES", "/a", Os::Linux).contains("not writable"));
        assert_eq!(
            explain_fs_error_for("No such file or directory", "/a", Os::Linux),
            "No such file or directory"
        );
    }
}
