use std::{
  path::{Path, PathBuf},
  sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
  },
};

use anyhow::{Context, Result};
use git2::{Blob, Repository};

#[derive(Clone, Debug)]
pub struct GitFileBases {
  pub head: Option<String>,
  pub index: Option<String>,
}

#[derive(Clone, Debug)]
pub struct GitStore {
  repo_root: PathBuf,
  op_id: Arc<AtomicUsize>,
}

impl GitStore {
  pub fn new(repo_root: impl Into<PathBuf>) -> Self {
    Self {
      repo_root: repo_root.into(),
      op_id: Arc::new(AtomicUsize::new(0)),
    }
  }

  pub fn op_id(&self) -> usize {
    self.op_id.load(Ordering::Relaxed)
  }

  pub fn bump_op(&self) {
    self.op_id.fetch_add(1, Ordering::Relaxed);
  }

  pub fn load_bases(&self, rel_path: &Path) -> Result<GitFileBases> {
    let repo = Repository::open(&self.repo_root)
      .with_context(|| format!("open repo at {:?}", self.repo_root))?;
    let head = read_head_content(&repo, rel_path)?;
    let index = read_index_content(&repo, rel_path)?;
    Ok(GitFileBases { head, index })
  }
}

fn read_head_content(repo: &Repository, rel_path: &Path) -> Result<Option<String>> {
  let head = match repo.head() {
    Ok(head) => head,
    Err(_) => return Ok(None),
  };
  let tree = match head.peel_to_tree() {
    Ok(tree) => tree,
    Err(_) => return Ok(None),
  };
  let entry = match tree.get_path(rel_path) {
    Ok(entry) => entry,
    Err(_) => return Ok(None),
  };
  let blob = repo.find_blob(entry.id())?;
  Ok(Some(blob_to_string(&blob)))
}

fn read_index_content(repo: &Repository, rel_path: &Path) -> Result<Option<String>> {
  let index = repo.index()?;
  let entry = match index.get_path(rel_path, 0) {
    Some(entry) => entry,
    None => return Ok(None),
  };
  let blob = repo.find_blob(entry.id)?;
  Ok(Some(blob_to_string(&blob)))
}

fn blob_to_string(blob: &Blob) -> String {
  String::from_utf8_lossy(blob.content()).into_owned()
}

#[cfg(test)]
mod tests {
  use super::*;
  use git2::{Repository, Signature};
  use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
  };

  struct TempRepo {
    path: PathBuf,
  }

  impl TempRepo {
    fn init(prefix: &str) -> Self {
      let mut path = std::env::temp_dir();
      let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
      path.push(format!("reviu-{prefix}-{}-{nanos}", std::process::id()));
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

  fn commit_file(repo_root: &Path, rel_path: &Path, contents: &str, message: &str) {
    let repo = Repository::open(repo_root).expect("open repo");
    std::fs::write(repo_root.join(rel_path), contents).expect("write worktree file");

    let mut index = repo.index().expect("open index");
    index.add_path(rel_path).expect("stage file");
    index.write().expect("write index");
    let tree_id = index.write_tree().expect("write tree");
    let tree = repo.find_tree(tree_id).expect("find tree");
    let signature = Signature::now("Reviu Tests", "tests@reviu.local").expect("signature");
    let parent = repo.head().ok().and_then(|head| head.peel_to_commit().ok());

    match parent {
      Some(parent) => {
        repo
          .commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &[&parent],
          )
          .expect("commit with parent");
      }
      None => {
        repo
          .commit(Some("HEAD"), &signature, &signature, message, &tree, &[])
          .expect("initial commit");
      }
    }
  }

  #[test]
  fn op_id_starts_at_zero_and_increments() {
    let temp = TempRepo::init("store-op-id");
    let store = GitStore::new(&temp.path);
    assert_eq!(store.op_id(), 0);

    store.bump_op();
    store.bump_op();
    assert_eq!(store.op_id(), 2);
  }

  #[test]
  fn load_bases_returns_none_for_missing_file() {
    let temp = TempRepo::init("store-missing-file");
    let store = GitStore::new(&temp.path);
    let bases = store
      .load_bases(Path::new("missing.txt"))
      .expect("load bases");
    assert_eq!(bases.head, None);
    assert_eq!(bases.index, None);
  }

  #[test]
  fn load_bases_reads_head_and_index_versions() {
    let temp = TempRepo::init("store-head-index");
    let rel_path = Path::new("notes.txt");
    commit_file(&temp.path, rel_path, "head version\n", "initial");

    std::fs::write(temp.path.join(rel_path), "index version\n").expect("update worktree");
    let repo = Repository::open(&temp.path).expect("open repo");
    let mut index = repo.index().expect("open index");
    index.add_path(rel_path).expect("stage updated file");
    index.write().expect("write index");

    let store = GitStore::new(&temp.path);
    let bases = store.load_bases(rel_path).expect("load bases");
    assert_eq!(bases.head.as_deref(), Some("head version\n"));
    assert_eq!(bases.index.as_deref(), Some("index version\n"));
  }
}
