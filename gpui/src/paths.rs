//! Path string helpers, ported from `src/lib/fs.ts`.
//!
//! These stay free of GPUI so they can be tested without opening a window. The
//! rules match the web app's deliberately: Windows' filename rules are enforced
//! on every platform, because a note created on a Mac with `a:b.md` cannot be
//! copied to a Windows machine at all.

use std::path::{Path, PathBuf};

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
const RESERVED_ON_WINDOWS: [&str; 9] = ["con", "prn", "aux", "nul", "com1", "lpt1", "com2", "lpt2", "com3"];

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
    path == prefix
        || path.starts_with(&format!("{prefix}/"))
        || path.starts_with(&format!("{prefix}\\"))
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
    let base = app_data_dir()?;
    Some(base.join("Zyplus"))
}

/// This platform's per-user application data directory.
pub fn app_data_dir() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        home().map(|home| home.join("Library/Application Support"))
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| home().map(|home| home.join(".local/share")))
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Turns a raw OS error into something the user can act on.
pub fn explain_fs_error(err: &str, path: &str) -> String {
    let denied = ["os error 5", "EACCES", "EPERM", "permission denied", "access is denied"]
        .iter()
        .any(|needle| err.to_lowercase().contains(&needle.to_lowercase()));
    if !denied {
        return err.to_string();
    }
    if cfg!(windows) {
        return format!(
            "Windows blocked the change to \"{path}\".\n\n\
             This is usually Controlled folder access, which stops apps it does not \
             recognize from writing to Documents and Desktop.\n\n\
             Either choose a different location, or allow Zyplus: Windows Security → \
             Virus & threat protection → Ransomware protection → Allow an app through \
             Controlled folder access."
        );
    }
    if cfg!(target_os = "macos") {
        return format!(
            "macOS blocked the change to \"{path}\".\n\n\
             Zyplus does not have permission for this folder. Either choose a different \
             location, or grant access in System Settings → Privacy & Security → Files \
             and Folders."
        );
    }
    format!("\"{path}\" is not writable.\n\nCheck the folder's permissions, or choose a different location.")
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
}
