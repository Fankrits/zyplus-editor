//! The workspace tree and the file operations behind it, ported from
//! `src/state/workspaceReducer.ts` and the desktop half of `src/lib/fs.ts`.
//!
//! Folders are read one level at a time: a subfolder comes back with
//! `children: None` and is read when the user opens it. Reading a whole project
//! up front cost one round trip per folder, and a `node_modules` or a build
//! output made that tens of thousands.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::paths::{basename_of, display_join, invalid_name_reason, is_under, free_path};

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
pub fn set_children(nodes: &mut Vec<TreeNode>, id: &str, children: Vec<TreeNode>) -> bool {
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
    pub unloaded: bool,
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
                unloaded: node.is_unloaded(),
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
    fs::write(path, content).map_err(|e| io_err(e, path))
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
        return Err(format!("Cannot move \"{}\" into itself", basename_of(old_path)));
    }
    let finds_itself = new_path == old_path
        || (new_path.to_lowercase() == old_path.to_lowercase()
            && Path::new(old_path).exists()
            && cfg!(any(target_os = "macos", target_os = "windows")));
    if !finds_itself && Path::new(new_path).exists() {
        return Err(format!("\"{}\" already exists there", basename_of(new_path)));
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
        assert!(closed.iter().find(|r| r.name == "nested").unwrap().is_open == false);
        assert!(closed.iter().any(|r| r.name == "b.md"));

        let open: BTreeSet<String> = [dir.clone(), nested_id].into_iter().collect();
        assert_eq!(visible_rows(&roots, &open).len(), 4, "a.md appears");

        // Collapsing the root hides its whole first level.
        assert_eq!(visible_rows(&roots, &BTreeSet::new()).len(), 1, "just the root");

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
        assert_eq!(create_folder(&display_join(&dir, "a:b")).unwrap_err(), "A name cannot contain < > : \" | ? *");

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
}
