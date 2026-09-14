//! Turning project entries into the tree the file lists render. Folders come
//! before files, and the first file is what a fresh tree selects.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

use gpui_component::tree::TreeItem;

use crate::project_files::{ProjectEntry, ProjectEntryKind};

/// The items to render, the files by path, and the row to select with its id.
pub(crate) type FileTreeBuildResult<T> = (
  Vec<TreeItem>,
  HashMap<String, Rc<T>>,
  Option<usize>,
  Option<String>,
);

#[derive(Default)]
struct FileTreeNode {
  name: String,
  path: String,
  children: BTreeMap<String, FileTreeNode>,
  file: Option<()>,
  directory: Option<()>,
}

impl FileTreeNode {
  fn new(name: String, path: String) -> Self {
    Self {
      name,
      path,
      children: BTreeMap::new(),
      file: None,
      directory: None,
    }
  }

  fn is_folder(&self) -> bool {
    !self.children.is_empty() || self.directory.is_some()
  }
}

/// `expanded_folder_paths` names the folders to open; `None` opens every one.
pub(crate) fn build_project_tree_items_with_expansion(
  entries: &[Rc<ProjectEntry>],
  expanded_folder_paths: Option<&HashSet<String>>,
) -> FileTreeBuildResult<ProjectEntry> {
  fn insert_node(
    map: &mut BTreeMap<String, FileTreeNode>,
    parts: &[&str],
    prefix: &str,
    kind: ProjectEntryKind,
  ) {
    let Some((head, tail)) = parts.split_first() else {
      return;
    };

    let path = if prefix.is_empty() {
      head.to_string()
    } else {
      format!("{}/{}", prefix, head)
    };

    let node = map
      .entry(head.to_string())
      .or_insert_with(|| FileTreeNode::new(head.to_string(), path.clone()));

    if tail.is_empty() {
      match kind {
        ProjectEntryKind::File => node.file = Some(()),
        ProjectEntryKind::Directory => node.directory = Some(()),
      }
      return;
    }

    let node_path = node.path.clone();
    insert_node(&mut node.children, tail, &node_path, kind);
  }

  let mut root: BTreeMap<String, FileTreeNode> = BTreeMap::new();
  let mut file_lookup: HashMap<String, Rc<ProjectEntry>> = HashMap::new();

  for entry in entries {
    let path = entry.path_id();
    if entry.is_file() {
      file_lookup.insert(path.clone(), entry.clone());
    }
    let parts: Vec<&str> = path.split('/').collect();
    insert_node(&mut root, &parts, "", entry.kind);
  }

  let mut order = Vec::new();
  let mut first_file_id: Option<String> = None;

  let mut root_nodes: Vec<FileTreeNode> = root.into_values().collect();
  root_nodes.sort_by(sort_nodes);

  let items = root_nodes
    .into_iter()
    .filter_map(|node| build_tree_item(node, &mut order, &mut first_file_id, expanded_folder_paths))
    .collect::<Vec<_>>();

  let selected_index = first_file_id
    .as_ref()
    .and_then(|id| order.iter().position(|candidate| candidate == id));

  (items, file_lookup, selected_index, first_file_id)
}

fn build_tree_item(
  node: FileTreeNode,
  order: &mut Vec<String>,
  first_file_id: &mut Option<String>,
  expanded_folder_paths: Option<&HashSet<String>>,
) -> Option<TreeItem> {
  let mut child_nodes: Vec<FileTreeNode> = node.children.into_values().collect();
  child_nodes.sort_by(sort_nodes);

  let children = child_nodes
    .into_iter()
    .filter_map(|child| build_tree_item(child, order, first_file_id, expanded_folder_paths))
    .collect::<Vec<_>>();

  if node.file.is_none() && children.is_empty() {
    return None;
  }

  let mut item = TreeItem::new(node.path.clone(), node.name.clone());
  if !children.is_empty() {
    let is_expanded = expanded_folder_paths
      .map(|paths| paths.contains(&node.path))
      .unwrap_or(true);
    item = item.children(children).expanded(is_expanded);
  }

  order.push(node.path.clone());
  if node.file.is_some() && first_file_id.is_none() {
    *first_file_id = Some(node.path.clone());
  }

  Some(item)
}

fn sort_nodes(a: &FileTreeNode, b: &FileTreeNode) -> std::cmp::Ordering {
  b.is_folder()
    .cmp(&a.is_folder())
    .then_with(|| a.name.cmp(&b.name))
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::path::PathBuf;

  fn files(paths: &[&str]) -> Vec<Rc<ProjectEntry>> {
    paths
      .iter()
      .map(|path| Rc::new(ProjectEntry::file(PathBuf::from(path))))
      .collect()
  }

  fn entries(paths: &[(&str, ProjectEntryKind)]) -> Vec<Rc<ProjectEntry>> {
    paths
      .iter()
      .map(|(path, kind)| {
        Rc::new(ProjectEntry {
          path: PathBuf::from(path),
          kind: *kind,
          is_gitignored: false,
          is_hidden: false,
        })
      })
      .collect()
  }

  fn build(paths: &[&str]) -> FileTreeBuildResult<ProjectEntry> {
    build_project_tree_items_with_expansion(&files(paths), None)
  }

  #[test]
  fn folders_come_first_and_the_first_file_is_selected() {
    let (items, lookup, selected_index, selected_id) =
      build(&["README.md", "src/lib.rs", "src/main.rs"]);

    assert_eq!(items.len(), 2);
    assert_eq!(items[0].label.as_ref(), "src");
    assert_eq!(items[0].children.len(), 2);
    assert_eq!(items[0].children[0].label.as_ref(), "lib.rs");
    assert_eq!(items[0].children[1].label.as_ref(), "main.rs");
    assert_eq!(items[1].label.as_ref(), "README.md");

    assert_eq!(selected_id.as_deref(), Some("src/lib.rs"));
    assert_eq!(selected_index, Some(0));
    assert!(lookup.contains_key("src/lib.rs"));
    assert!(lookup.contains_key("README.md"));
  }

  #[test]
  fn nothing_to_show_selects_nothing() {
    let (items, lookup, selected_index, selected_id) = build(&[]);

    assert!(items.is_empty());
    assert!(lookup.is_empty());
    assert_eq!(selected_index, None);
    assert_eq!(selected_id, None);
  }

  #[test]
  fn a_folder_shared_by_a_file_and_a_folder_keeps_both() {
    let (items, _, _, _) = build(&["src/main.rs", "src/nested/deep.rs"]);

    assert_eq!(items.len(), 1);
    let children = &items[0].children;
    assert_eq!(children[0].label.as_ref(), "nested");
    assert_eq!(children[1].label.as_ref(), "main.rs");
  }

  #[test]
  fn directory_entries_are_rendered_when_they_have_children() {
    let (items, lookup, _, _) = build_project_tree_items_with_expansion(
      &entries(&[
        ("src", ProjectEntryKind::Directory),
        ("src/main.rs", ProjectEntryKind::File),
      ]),
      None,
    );

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].label.as_ref(), "src");
    assert!(items[0].is_folder());
    assert!(lookup.contains_key("src/main.rs"));
    assert!(!lookup.contains_key("src"));
  }

  #[test]
  fn empty_directory_entries_are_skipped_until_the_tree_can_render_them_as_folders() {
    let (items, lookup, selected_index, selected_id) = build_project_tree_items_with_expansion(
      &entries(&[("empty", ProjectEntryKind::Directory)]),
      None,
    );

    assert!(items.is_empty());
    assert!(lookup.is_empty());
    assert_eq!(selected_index, None);
    assert_eq!(selected_id, None);
  }

  #[test]
  fn without_an_expansion_set_every_folder_is_open() {
    let (items, _, _, _) = build(&["src/nested/deep.rs"]);

    assert!(items[0].is_expanded());
    assert!(items[0].children[0].is_expanded());
  }

  #[test]
  fn an_expansion_set_opens_only_the_branches_it_names() {
    let expanded = HashSet::from(["src".to_string(), "src/nested".to_string()]);
    let all = files(&[
      "src/changed.rs",
      "src/nested/also_changed.rs",
      "tests/helper.rs",
      "README.md",
    ]);

    let (items, _, selected_index, selected_id) =
      build_project_tree_items_with_expansion(&all, Some(&expanded));

    assert_eq!(items.len(), 3);
    assert_eq!(items[0].label.as_ref(), "src");
    assert!(items[0].is_expanded());
    assert_eq!(items[0].children[0].label.as_ref(), "nested");
    assert!(items[0].children[0].is_expanded());
    assert_eq!(items[1].label.as_ref(), "tests");
    assert!(!items[1].is_expanded());
    assert_eq!(items[2].label.as_ref(), "README.md");
    assert_eq!(selected_id.as_deref(), Some("src/nested/also_changed.rs"));
    assert_eq!(selected_index, Some(0));
  }
}
