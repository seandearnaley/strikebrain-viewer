//! [`Tree::from_paths`]: a tree from path strings, for tests and fixtures.

use std::collections::HashMap;
use std::path::Path;

use super::{Kind, RawTree, Tree, TreeError};

/// Builds a tree from `(path, size)` pairs. A trailing `/` marks a folder.
/// The root is the folder at `root`, named after its last part.
pub(super) fn build<'a>(
    root: &Path,
    paths: impl IntoIterator<Item = (&'a str, u64)>,
) -> Result<Tree, TreeError> {
    let root_name = root
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    let mut raw = RawTree::new(&root_name, true)?;
    // Every node so far by its path without the trailing slash; "" is the root.
    let mut nodes: HashMap<&'a str, u32> = HashMap::from([("", 0)]);
    for (path, size) in paths {
        let (body, is_folder) = match path.strip_suffix('/') {
            Some(body) => (body, true),
            None => (path, false),
        };
        if !is_valid(body) {
            return Err(TreeError::InvalidPath(path.to_owned()));
        }
        if let Some(&existing) = nodes.get(body) {
            if is_folder && raw.kind(existing) == Kind::Dir {
                continue;
            }
            return Err(TreeError::Conflict(path.to_owned()));
        }
        let parent = folder(&mut raw, &mut nodes, parent_of(body), path)?;
        let (kind, size) = if is_folder {
            (Kind::Dir, 0)
        } else {
            (Kind::File, size)
        };
        let node = raw.push(parent, name_of(body), kind)?;
        raw.set_size(node, size);
        nodes.insert(body, node);
    }
    raw.build(root.to_path_buf())
}

/// The raw index of the folder at `path`, creating it and any missing
/// ancestors. `given` is the input path, for the error.
fn folder<'a>(
    raw: &mut RawTree,
    nodes: &mut HashMap<&'a str, u32>,
    path: &'a str,
    given: &str,
) -> Result<u32, TreeError> {
    let mut missing = Vec::new();
    let mut current = path;
    let mut parent = loop {
        if let Some(&node) = nodes.get(current) {
            break node;
        }
        missing.push(current);
        current = parent_of(current);
    };
    if raw.kind(parent) != Kind::Dir {
        return Err(TreeError::Conflict(given.to_owned()));
    }
    for &prefix in missing.iter().rev() {
        parent = raw.push(parent, name_of(prefix), Kind::Dir)?;
        nodes.insert(prefix, parent);
    }
    Ok(parent)
}

/// Whether `path` is relative, non-empty and free of empty, `.`, `..` and
/// NUL-bearing components.
fn is_valid(path: &str) -> bool {
    !path.is_empty()
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != ".." && !part.contains('\0'))
}

/// Everything before the last `/`, or `""` at the top level.
fn parent_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(parent, _)| parent)
}

/// Everything after the last `/`.
fn name_of(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(_, name)| name)
}
