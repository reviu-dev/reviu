//! Turning a flat list of paths into the tree the file lists render. Folders
//! come before files, and the first file is what a fresh tree selects.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

use gpui_component::tree::TreeItem;

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
}

impl FileTreeNode {
  fn new(name: String, path: String) -> Self {
    Self {
      name,
      path,
      children: BTreeMap::new(),
      file: None,
    }
  }

  fn is_folder(&self) -> bool {
    !self.children.is_empty()
  }
}

/// `expanded_folder_paths` names the folders to open; `None` opens every one.
pub(crate) fn build_path_tree_items_with_expansion<T, F>(
  files: &[Rc<T>],
  path_for: F,
  expanded_folder_paths: Option<&HashSet<String>>,
) -> FileTreeBuildResult<T>
where
  F: Fn(&T) -> &str,
{
  fn insert_node(
    map: &mut BTreeMap<String, FileTreeNode>,
    parts: &[&str],
    prefix: &str,
    has_file: bool,
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
      if has_file {
        node.file = Some(());
      }
      return;
    }

    let node_path = node.path.clone();
    insert_node(&mut node.children, tail, &node_path, has_file);
  }

  let mut root: BTreeMap<String, FileTreeNode> = BTreeMap::new();
  let mut file_lookup: HashMap<String, Rc<T>> = HashMap::new();

  for file in files {
    let path = path_for(file.as_ref());
    file_lookup.insert(path.to_string(), file.clone());
    let parts: Vec<&str> = path.split('/').collect();
    insert_node(&mut root, &parts, "", true);
  }

  let mut order = Vec::new();
  let mut first_file_id: Option<String> = None;

  let mut root_nodes: Vec<FileTreeNode> = root.into_values().collect();
  root_nodes.sort_by(|a, b| {
    b.is_folder()
      .cmp(&a.is_folder())
      .then_with(|| a.name.cmp(&b.name))
  });

  let items = root_nodes
    .into_iter()
    .map(|node| build_tree_item(node, &mut order, &mut first_file_id, expanded_folder_paths))
    .collect::<Vec<_>>();

  let selected_index = first_file_id
    .as_ref()
    .and_then(|id| order.iter().position(|candidate| candidate == id));

  (items, file_lookup, selected_index, first_file_id)
}

/// Every folder on the way to one of these paths, so a tree can open just the
/// branches that lead somewhere.
pub(crate) fn expanded_folder_paths_for_changed_files<'a, I>(paths: I) -> HashSet<String>
where
  I: IntoIterator<Item = &'a str>,
{
  let mut expanded = HashSet::new();
  for path in paths {
    let mut prefix = String::new();
    let parts = path.split('/').collect::<Vec<_>>();
    for folder in parts.iter().take(parts.len().saturating_sub(1)) {
      if prefix.is_empty() {
        prefix.push_str(folder);
      } else {
        prefix.push('/');
        prefix.push_str(folder);
      }
      expanded.insert(prefix.clone());
    }
  }
  expanded
}

fn build_tree_item(
  node: FileTreeNode,
  order: &mut Vec<String>,
  first_file_id: &mut Option<String>,
  expanded_folder_paths: Option<&HashSet<String>>,
) -> TreeItem {
  let mut child_nodes: Vec<FileTreeNode> = node.children.into_values().collect();
  child_nodes.sort_by(|a, b| {
    b.is_folder()
      .cmp(&a.is_folder())
      .then_with(|| a.name.cmp(&b.name))
  });

  let mut item = TreeItem::new(node.path.clone(), node.name.clone());
  if !child_nodes.is_empty() {
    let children = child_nodes
      .into_iter()
      .map(|child| build_tree_item(child, order, first_file_id, expanded_folder_paths))
      .collect::<Vec<_>>();
    let is_expanded = expanded_folder_paths
      .map(|paths| paths.contains(&node.path))
      .unwrap_or(true);
    item = item.children(children).expanded(is_expanded);
  }

  order.push(node.path.clone());
  if node.file.is_some() && first_file_id.is_none() {
    *first_file_id = Some(node.path.clone());
  }

  item
}

#[cfg(test)]
mod tests {
  use super::*;

  struct TestFile {
    path: String,
  }

  fn files(paths: &[&str]) -> Vec<Rc<TestFile>> {
    paths
      .iter()
      .map(|path| {
        Rc::new(TestFile {
          path: (*path).to_string(),
        })
      })
      .collect()
  }

  fn build(paths: &[&str]) -> FileTreeBuildResult<TestFile> {
    build_path_tree_items_with_expansion(&files(paths), |file| file.path.as_str(), None)
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
    // `src` holds a file and a folder: the folder still comes first.
    let (items, _, _, _) = build(&["src/main.rs", "src/nested/deep.rs"]);

    assert_eq!(items.len(), 1);
    let children = &items[0].children;
    assert_eq!(children[0].label.as_ref(), "nested");
    assert_eq!(children[1].label.as_ref(), "main.rs");
  }

  #[test]
  fn without_an_expansion_set_every_folder_is_open() {
    let (items, _, _, _) = build(&["src/nested/deep.rs"]);

    assert!(items[0].is_expanded());
    assert!(items[0].children[0].is_expanded());
  }

  #[test]
  fn an_expansion_set_opens_only_the_branches_it_names() {
    let expanded =
      expanded_folder_paths_for_changed_files(["src/changed.rs", "src/nested/also_changed.rs"]);
    let all = files(&[
      "src/changed.rs",
      "src/nested/also_changed.rs",
      "tests/helper.rs",
      "README.md",
    ]);

    let (items, _, selected_index, selected_id) =
      build_path_tree_items_with_expansion(&all, |file| file.path.as_str(), Some(&expanded));

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

  #[test]
  fn a_path_without_a_folder_expands_nothing() {
    assert!(expanded_folder_paths_for_changed_files(["README.md"]).is_empty());
  }
}
