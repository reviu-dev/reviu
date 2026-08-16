//! Git fixtures shared by the tests of every page and component.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use git2::{Repository, Signature};

pub(crate) struct TempRepo {
  pub(crate) path: PathBuf,
}

impl TempRepo {
  pub(crate) fn init(prefix: &str) -> Self {
    let path = temp_path(prefix);
    std::fs::create_dir_all(&path).expect("create temp dir");
    Repository::init(&path).expect("init git repository");
    Self { path }
  }
}

impl Drop for TempRepo {
  fn drop(&mut self) {
    let _ = std::fs::remove_dir_all(&self.path);
  }
}

pub(crate) fn temp_path(prefix: &str) -> PathBuf {
  let mut path = std::env::temp_dir();
  let nanos = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .expect("system clock before unix epoch")
    .as_nanos();
  path.push(format!("reviu-{prefix}-{}-{nanos}", std::process::id()));
  path
}

pub(crate) fn commit_text_file(
  repo_root: &Path,
  rel_path: &Path,
  contents: &str,
  message: &str,
) -> git2::Oid {
  let repo = Repository::open(repo_root).expect("open repo");
  if let Some(parent_dir) = rel_path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
    std::fs::create_dir_all(repo_root.join(parent_dir)).expect("create parent dir");
  }
  std::fs::write(repo_root.join(rel_path), contents).expect("write worktree file");

  let mut index = repo.index().expect("open index");
  index.add_path(rel_path).expect("stage file");
  index.write().expect("write index");
  let tree_id = index.write_tree().expect("write tree");
  let tree = repo.find_tree(tree_id).expect("find tree");
  let signature = Signature::now("Reviu Tests", "tests@reviu.local").expect("signature");
  let parent = repo.head().ok().and_then(|head| head.peel_to_commit().ok());
  let parents: Vec<_> = parent.iter().collect();

  repo
    .commit(
      Some("HEAD"),
      &signature,
      &signature,
      message,
      &tree,
      &parents,
    )
    .expect("commit")
}
