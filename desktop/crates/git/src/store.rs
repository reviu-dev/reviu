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
