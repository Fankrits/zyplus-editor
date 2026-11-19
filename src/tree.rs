//! The workspace tree and the file operations behind it, ported from
//! `src/state/workspaceReducer.ts` and the desktop half of `src/lib/fs.ts`.
//!
//! Folders are read one level at a time: a subfolder comes back with
//! `children: None` and is read when the user opens it. Reading a whole project
//! up front cost one round trip per folder, and a `node_modules` or a build
//! output made that tens of thousands.

use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::paths::{basename_of, display_join, free_path, invalid_name_reason, is_under};

#[derive(Clone, Debug, PartialEq)]
pub struct TreeNode {
    pub id: String,
    pub name: String,
    pub is_folder: bool,
    /// `None` means a folder whose contents have not been read yet; `Some(vec![])`
    /// is a folder that was read and is empty.
    pub children: Option<Vec<TreeNode>>,
}

impl TreeNode {
    pub fn is_unloaded(&self) -> bool {
        self.is_folder && self.children.is_none()
    }
}

fn io_err(err: std::io::Error, path: &str) -> String {
    crate::paths::explain_fs_error(&err.to_string(), path)
}

/// Lists one folder, skipping dotfiles, folders first.
pub fn read_children(dir: &str) -> Result<Vec<TreeNode>, String> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| io_err(e, dir))? {
        let entry = entry.map_err(|e| io_err(e, dir))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let is_folder = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        entries.push((name, is_folder));
    }
    entries.sort_by(|(a_name, a_dir), (b_name, b_dir)| {
        b_dir
            .cmp(a_dir)
            .then_with(|| a_name.to_lowercase().cmp(&b_name.to_lowercase()))
            .then_with(|| a_name.cmp(b_name))
    });
    Ok(entries
        .into_iter()
        .map(|(name, is_folder)| TreeNode {
            id: display_join(dir, &name),
            name,
            is_folder,
            children: if is_folder { None } else { Some(Vec::new()) },
        })
        .collect())
}

/// Reads a folder as a project: one collapsible top-level node holding its first
/// level. `loaded` is the folders the user had already opened, so they come back
/// showing their contents.
pub fn read_project_node(dir: &str, loaded: &BTreeSet<String>) -> Result<TreeNode, String> {
    let children = read_level(dir, loaded)?;
    Ok(TreeNode {
        id: dir.to_string(),
        name: basename_of(dir),
        is_folder: true,
        children: Some(children),
    })
}

fn read_level(dir: &str, loaded: &BTreeSet<String>) -> Result<Vec<TreeNode>, String> {
    let nodes = read_children(dir)?;
    Ok(nodes
        .into_iter()
        .map(|node| {
            if !loaded.contains(&node.id) {
                return node;
            }
            // Gone or unreadable since it was listed: leave it to be read (and
            // reported) if it is opened again.
            match read_level(&node.id, loaded) {
                Ok(children) => TreeNode {
                    children: Some(children),
                    ..node
                },
                Err(_) => node,
            }
        })
        .collect())
}

/// Replaces one folder's contents, sharing every subtree it did not touch.
pub fn set_children(nodes: &mut [TreeNode], id: &str, children: Vec<TreeNode>) -> bool {
    for node in nodes.iter_mut() {
        if node.id == id {
            node.children = Some(children);
            return true;
        }
        if let Some(inner) = node.children.as_mut() {
            if is_under(id, &node.id) && set_children(inner, id, children.clone()) {
                return true;
            }
        }
    }
    false
}

/// A folder by id, looking only where it can be: through the folders on the way.
pub fn find_tree_node<'a>(nodes: &'a [TreeNode], id: &str) -> Option<&'a TreeNode> {
    for node in nodes {
        if node.id == id {
            return Some(node);
        }
        if let Some(children) = node.children.as_ref() {
            if is_under(id, &node.id) {
                if let Some(found) = find_tree_node(children, id) {
                    return Some(found);
                }
            }
        }
    }
    None
}

/// Every folder whose contents are on screen, for a refresh to read again.
pub fn loaded_folder_ids(nodes: &[TreeNode], into: &mut BTreeSet<String>) {
    for node in nodes {
        if !node.is_folder || node.is_unloaded() {
            continue;
        }
        into.insert(node.id.clone());
        if let Some(children) = node.children.as_ref() {
            loaded_folder_ids(children, into);
        }
    }
}

/// A row of the flattened, currently visible sidebar.
#[derive(Clone, Debug)]
pub struct Row {
    pub id: String,
    pub name: String,
    pub depth: usize,
    pub is_folder: bool,
    pub is_open: bool,
    pub is_root: bool,
}

/// Flattens the tree into the rows the sidebar shows, skipping closed folders.
pub fn visible_rows(tree: &[TreeNode], open: &BTreeSet<String>) -> Vec<Row> {
    fn walk(nodes: &[TreeNode], depth: usize, open: &BTreeSet<String>, out: &mut Vec<Row>) {
        for node in nodes {
            let is_open = open.contains(&node.id);
            out.push(Row {
                id: node.id.clone(),
                name: node.name.clone(),
                depth,
                is_folder: node.is_folder,
                is_open,
                is_root: depth == 0,
            });
            if node.is_folder && is_open {
                if let Some(children) = node.children.as_ref() {
                    walk(children, depth + 1, open, out);
                }
            }
        }
    }
    let mut rows = Vec::new();
    walk(tree, 0, open, &mut rows);
    rows
}

fn assert_name_is_usable(path: &str) -> Result<(), String> {
    match invalid_name_reason(&basename_of(path)) {
        Some(reason) => Err(reason),
        None => Ok(()),
    }
}

pub fn read_text_file(path: &str) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| io_err(e, path))
}

pub fn write_text_file(path: &str, content: &str) -> Result<(), String> {
    write_atomic(Path::new(path), content.as_bytes()).map_err(|e| io_err(e, path))
}

/// Errno for a rename that crossed filesystems (`EXDEV`; `ERROR_NOT_SAME_DEVICE` on Windows).
fn is_cross_device(err: &std::io::Error) -> bool {
    #[cfg(windows)]
    const EXDEV: i32 = 17;
    #[cfg(not(windows))]
    const EXDEV: i32 = 18;
    err.raw_os_error() == Some(EXDEV)
}

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Replaces the file at `path` with `bytes` so that a crash, a full disk or a
/// failed write leaves either the old file or the new one, never a truncated mix.
///
/// The bytes go to a hidden temp file in the same directory (the sidebar skips
/// dotfiles, so a leftover never shows up as a note), are fsynced, and are then
/// renamed over the target. The existing file's permission bits are copied to
/// the temp file first. A symlink is followed, so the link keeps pointing at the
/// same file. A read-only existing file is refused, as `fs::write` would refuse
/// it. The temp file is removed on every error path.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let target = match fs::canonicalize(path) {
        Ok(real) => real,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => path.to_path_buf(),
        Err(err) => return Err(err),
    };
    let permissions = match fs::metadata(&target) {
        Ok(meta) if meta.permissions().readonly() => {
            return Err(std::io::ErrorKind::PermissionDenied.into());
        }
        Ok(meta) => Some(meta.permissions()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return Err(err),
    };

    let dir = match target.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(".{name}.{}.{seq}.tmp", std::process::id()));

    let staged = (|| -> std::io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        if let Some(permissions) = permissions {
            file.set_permissions(permissions)?;
        }
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, &target)
    })();

    match staged {
        Ok(()) => {
            sync_dir(dir);
            Ok(())
        }
        Err(err) if is_cross_device(&err) => {
            // Only reachable if the temp file somehow landed on another
            // filesystem. A direct write is the best that is left.
            let _ = fs::remove_file(&tmp);
            fs::write(&target, bytes)
        }
        Err(err) => {
            let _ = fs::remove_file(&tmp);
            Err(err)
        }
    }
}

/// Makes a completed rename durable. Best effort: not every platform lets a
/// directory be opened and synced, and the rename has already happened.
fn sync_dir(dir: &Path) {
    if let Ok(handle) = fs::File::open(dir) {
        let _ = handle.sync_all();
    }
}

pub fn create_file(path: &str) -> Result<(), String> {
    assert_name_is_usable(path)?;
    if Path::new(path).exists() {
        return Err(format!("\"{path}\" already exists"));
    }
    write_text_file(path, "")
}

pub fn create_folder(path: &str) -> Result<(), String> {
    assert_name_is_usable(path)?;
    if Path::new(path).exists() {
        return Err(format!("\"{path}\" already exists"));
    }
    fs::create_dir_all(path).map_err(|e| io_err(e, path))
}

/// Creates a folder and any missing parents. Unlike `create_folder`, existing is fine.
pub fn ensure_folder(path: &str) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|e| io_err(e, path))
}

pub fn rename_path(old_path: &str, new_path: &str) -> Result<(), String> {
    // Only a name the user just typed gets validated. Moving a file that already
    // carries an awkward name into another folder must keep working.
    if basename_of(new_path) != basename_of(old_path) {
        assert_name_is_usable(new_path)?;
    }
    if is_under(new_path, old_path) && new_path != old_path {
        return Err(format!(
            "Cannot move \"{}\" into itself",
            basename_of(old_path)
        ));
    }
    let finds_itself = new_path == old_path
        || (new_path.to_lowercase() == old_path.to_lowercase()
            && Path::new(old_path).exists()
            && cfg!(any(target_os = "macos", target_os = "windows")));
    if !finds_itself && Path::new(new_path).exists() {
        return Err(format!(
            "\"{}\" already exists there",
            basename_of(new_path)
        ));
    }
    fs::rename(old_path, new_path).map_err(|e| io_err(e, new_path))
}

pub fn delete_path(path: &str, is_folder: bool) -> Result<(), String> {
    let result = if is_folder {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    result.map_err(|e| io_err(e, path))
}

/// Copies a file alongside itself as "name copy.ext", "name copy 2.ext", ...
pub fn duplicate_file(path: &str) -> Result<String, String> {
    let content = read_text_file(path)?;
    let new_path = free_path(&crate::paths::dirname_of(path), &basename_of(path), " copy");
    write_text_file(&new_path, &content)?;
    Ok(new_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> String {
        let dir = std::env::temp_dir().join(format!("zyplus-gpui-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir.to_string_lossy().into_owned()
    }

    #[test]
    fn atomic_overwrite_replaces_the_content_and_leaves_no_temp_file() {
        let dir = scratch("atomic-overwrite");
        let path = format!("{dir}/note.md");
        write_text_file(&path, "first version, quite a bit longer than the second").unwrap();
        write_text_file(&path, "second").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "second");
        let names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            vec!["note.md".to_string()],
            "no temp file may be left behind"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_failed_write_keeps_the_old_file_and_removes_its_temp_file() {
        let dir = scratch("atomic-missing");
        let missing = format!("{dir}/gone/note.md");
        let err = write_text_file(&missing, "x").unwrap_err();
        assert!(!err.is_empty(), "the error should say what went wrong");
        assert!(!Path::new(&format!("{dir}/gone")).exists());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_read_only_file_is_refused_and_left_alone() {
        let dir = scratch("atomic-readonly");
        let path = format!("{dir}/locked.md");
        fs::write(&path, "keep me").unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&path, perms).unwrap();

        assert!(write_text_file(&path, "replaced").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "keep me");

        let mut perms = fs::metadata(&path).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        fs::set_permissions(&path, perms).unwrap();
        fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn atomic_overwrite_keeps_the_permission_bits() {
        use std::os::unix::fs::PermissionsExt;

        let dir = scratch("atomic-perms");
        let path = format!("{dir}/shared.md");
        fs::write(&path, "old").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();

        write_text_file(&path, "new").unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o640);
        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
        fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_is_followed_and_stays_a_symlink() {
        let dir = scratch("atomic-symlink");
        let real = format!("{dir}/real.md");
        let link = format!("{dir}/link.md");
        fs::write(&real, "old").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();

        write_text_file(&link, "new").unwrap();
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read_to_string(&real).unwrap(), "new");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn subfolders_come_back_unread_and_folders_sort_first() {
        let dir = scratch("lazy");
        fs::create_dir_all(format!("{dir}/nested/deeper")).unwrap();
        fs::write(format!("{dir}/b.md"), "b").unwrap();
        fs::write(format!("{dir}/.hidden"), "x").unwrap();

        let children = read_children(&dir).unwrap();
        let names: Vec<_> = children.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, vec!["nested", "b.md"]);
        assert!(children[0].is_unloaded(), "a subfolder starts unread");
        assert!(!children[1].is_folder);

        let tree = read_project_node(&dir, &BTreeSet::new()).unwrap();
        assert_eq!(tree.children.as_ref().unwrap().len(), 2);
        assert!(tree.children.as_ref().unwrap()[0].is_unloaded());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn previously_open_folders_are_read_again() {
        let dir = scratch("restore");
        fs::create_dir_all(format!("{dir}/nested")).unwrap();
        fs::write(format!("{dir}/nested/a.md"), "a").unwrap();

        let nested = display_join(&dir, "nested");
        let open: BTreeSet<String> = [nested.clone()].into_iter().collect();
        let tree = read_project_node(&dir, &open).unwrap();
        let node = find_tree_node(tree.children.as_ref().unwrap(), &nested).unwrap();
        assert!(!node.is_unloaded());
        assert_eq!(node.children.as_ref().unwrap().len(), 1);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rows_skip_the_contents_of_closed_folders() {
        let dir = scratch("rows");
        fs::create_dir_all(format!("{dir}/nested")).unwrap();
        fs::write(format!("{dir}/nested/a.md"), "a").unwrap();
        fs::write(format!("{dir}/b.md"), "b").unwrap();

        // Read as if the user had opened `nested` last time, so it has contents
        // that can be hidden or revealed without another read.
        let nested_id = display_join(&dir, "nested");
        let loaded: BTreeSet<String> = [nested_id.clone()].into_iter().collect();
        let tree = read_project_node(&dir, &loaded).unwrap();
        let roots = vec![tree.clone()];

        // The project root is open (as it is on launch); `nested` is closed.
        let root_open: BTreeSet<String> = [dir.clone()].into_iter().collect();
        let closed = visible_rows(&roots, &root_open);
        assert_eq!(closed.len(), 3, "root + nested + b.md");
        assert!(!closed.iter().find(|r| r.name == "nested").unwrap().is_open);
        assert!(closed.iter().any(|r| r.name == "b.md"));

        let open: BTreeSet<String> = [dir.clone(), nested_id].into_iter().collect();
        assert_eq!(visible_rows(&roots, &open).len(), 4, "a.md appears");

        // Collapsing the root hides its whole first level.
        assert_eq!(
            visible_rows(&roots, &BTreeSet::new()).len(),
            1,
            "just the root"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn creating_duplicating_and_deleting() {
        let dir = scratch("ops");
        let file = display_join(&dir, "note.md");
        create_file(&file).unwrap();
        write_text_file(&file, "hello").unwrap();
        assert_eq!(read_text_file(&file).unwrap(), "hello");

        let copy = duplicate_file(&file).unwrap();
        assert_eq!(basename_of(&copy), "note copy.md");
        assert_eq!(read_text_file(&copy).unwrap(), "hello");

        // An invalid name is rejected before the filesystem sees it.
        assert_eq!(
            create_folder(&display_join(&dir, "a:b")).unwrap_err(),
            "A name cannot contain < > : \" | ? *"
        );

        delete_path(&copy, false).unwrap();
        assert!(!Path::new(&copy).exists());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn renaming_validates_only_a_new_name() {
        let dir = scratch("rename");
        let old = display_join(&dir, "awkward: name.md");
        fs::write(&old, "x").unwrap();
        fs::create_dir_all(display_join(&dir, "dest")).unwrap();

        // Same name in a new folder is fine even though Windows would refuse it.
        let moved = display_join(&display_join(&dir, "dest"), "awkward: name.md");
        rename_path(&old, &moved).unwrap();
        assert!(Path::new(&moved).exists());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn renaming_rejects_collisions_and_invalid_names_without_clobbering() {
        let dir = scratch("rename-errors");
        let a = display_join(&dir, "a.md");
        let b = display_join(&dir, "b.md");
        fs::write(&a, "alpha").unwrap();
        fs::write(&b, "beta").unwrap();

        // Collision: must fail and leave both files untouched.
        let err = rename_path(&a, &b).unwrap_err();
        assert!(err.contains("already exists"), "got {err}");
        assert_eq!(fs::read_to_string(&a).unwrap(), "alpha");
        assert_eq!(fs::read_to_string(&b).unwrap(), "beta");

        // Invalid names: empty, reserved, bad chars.
        assert!(rename_path(&a, &display_join(&dir, "")).is_err());
        assert!(rename_path(&a, &display_join(&dir, "CON.md")).is_err());
        assert!(rename_path(&a, &display_join(&dir, "bad:name.md")).is_err());
        assert_eq!(fs::read_to_string(&a).unwrap(), "alpha");

        // Folder cannot be moved inside itself.
        let folder = display_join(&dir, "sub");
        fs::create_dir_all(&folder).unwrap();
        let nested = display_join(&folder, "sub");
        let self_err = rename_path(&folder, &nested).unwrap_err();
        assert!(self_err.contains("into itself"), "got {self_err}");

        // Folder rename moves its contents on disk.
        let renamed_folder = display_join(&dir, "renamed-sub");
        fs::write(display_join(&folder, "note.md"), "inside").unwrap();
        rename_path(&folder, &renamed_folder).unwrap();
        assert!(!Path::new(&folder).exists());
        assert_eq!(
            fs::read_to_string(display_join(&renamed_folder, "note.md")).unwrap(),
            "inside"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn renaming_allows_case_only_change_on_macos() {
        let dir = scratch("rename-case");
        let lower = display_join(&dir, "note.md");
        let upper = display_join(&dir, "Note.md");
        fs::write(&lower, "hello").unwrap();

        rename_path(&lower, &upper).unwrap();
        let entries: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(entries, vec!["Note.md".to_string()]);

        fs::remove_dir_all(&dir).ok();
    }
}
