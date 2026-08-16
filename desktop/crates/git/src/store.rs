use std::{
  collections::HashSet,
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
pub struct GitFileBinaryBases {
  pub head: Option<Vec<u8>>,
  pub index: Option<Vec<u8>>,
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

  pub fn load_binary_bases(&self, rel_path: &Path) -> Result<GitFileBinaryBases> {
    let repo = Repository::open(&self.repo_root)
      .with_context(|| format!("open repo at {:?}", self.repo_root))?;
    let head = read_head_bytes(&repo, rel_path)?;
    let index = read_index_bytes(&repo, rel_path)?;
    Ok(GitFileBinaryBases { head, index })
  }
}

pub fn search_repo_head_contents(
  repo_root: &Path,
  rel_paths: &[PathBuf],
  query: &str,
) -> Result<HashSet<PathBuf>> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let head = match repo.head() {
    Ok(head) => head,
    Err(_) => return Ok(HashSet::new()),
  };
  let tree = match head.peel_to_tree() {
    Ok(tree) => tree,
    Err(_) => return Ok(HashSet::new()),
  };

  let query = query.to_lowercase();
  let mut matches = HashSet::new();
  for rel_path in rel_paths {
    if read_tree_content(&repo, &tree, rel_path)?
      .is_some_and(|contents| contents.to_lowercase().contains(query.as_str()))
    {
      matches.insert(rel_path.clone());
    }
  }

  Ok(matches)
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
  read_tree_content(repo, &tree, rel_path)
}

fn read_head_bytes(repo: &Repository, rel_path: &Path) -> Result<Option<Vec<u8>>> {
  let head = match repo.head() {
    Ok(head) => head,
    Err(_) => return Ok(None),
  };
  let tree = match head.peel_to_tree() {
    Ok(tree) => tree,
    Err(_) => return Ok(None),
  };
  read_tree_bytes(repo, &tree, rel_path)
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

fn read_index_bytes(repo: &Repository, rel_path: &Path) -> Result<Option<Vec<u8>>> {
  let index = repo.index()?;
  let entry = match index.get_path(rel_path, 0) {
    Some(entry) => entry,
    None => return Ok(None),
  };
  let blob = repo.find_blob(entry.id)?;
  Ok(Some(blob_to_bytes(&blob)))
}

fn read_tree_content(
  repo: &Repository,
  tree: &git2::Tree,
  rel_path: &Path,
) -> Result<Option<String>> {
  let entry = match tree.get_path(rel_path) {
    Ok(entry) => entry,
    Err(_) => return Ok(None),
  };
  let blob = repo.find_blob(entry.id())?;
  Ok(Some(blob_to_string(&blob)))
}

fn read_tree_bytes(
  repo: &Repository,
  tree: &git2::Tree,
  rel_path: &Path,
) -> Result<Option<Vec<u8>>> {
  let entry = match tree.get_path(rel_path) {
    Ok(entry) => entry,
    Err(_) => return Ok(None),
  };
  let blob = repo.find_blob(entry.id())?;
  Ok(Some(blob_to_bytes(&blob)))
}

fn blob_to_string(blob: &Blob) -> String {
  String::from_utf8_lossy(blob.content()).into_owned()
}

fn blob_to_bytes(blob: &Blob) -> Vec<u8> {
  blob.content().to_vec()
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::test_support::{TempRepo, commit_text_file as commit_file};
  use git2::Repository;

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

  #[test]
  fn search_repo_head_contents_matches_tracked_head_files_only() {
    let temp = TempRepo::init("store-search-head");
    commit_file(
      &temp.path,
      Path::new("tracked.txt"),
      "Needle in HEAD\n",
      "tracked",
    );
    std::fs::create_dir_all(temp.path.join("nested")).expect("create nested dir");
    commit_file(
      &temp.path,
      Path::new("nested/other.txt"),
      "different content\n",
      "nested",
    );
    std::fs::create_dir_all(temp.path.join("scratch")).expect("create scratch dir");
    std::fs::write(
      temp.path.join("scratch/untracked.txt"),
      "needle in worktree only\n",
    )
    .expect("write untracked file");

    let matches = search_repo_head_contents(
      &temp.path,
      &[
        PathBuf::from("tracked.txt"),
        PathBuf::from("nested/other.txt"),
        PathBuf::from("scratch/untracked.txt"),
      ],
      "needle",
    )
    .expect("search repo head contents");

    assert_eq!(matches, HashSet::from([PathBuf::from("tracked.txt")]));
  }
}
