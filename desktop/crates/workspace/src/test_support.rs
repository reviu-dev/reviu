//! Git fixtures shared by the tests of every page and component.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use git2::build::CheckoutBuilder;
use git2::{BranchType, Cred, PushOptions, RemoteCallbacks, Repository, Signature};

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

pub(crate) struct TempBareRepo {
  pub(crate) path: PathBuf,
}

impl TempBareRepo {
  pub(crate) fn init(prefix: &str) -> Self {
    let mut path = std::env::temp_dir();
    let nanos = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .expect("system clock before unix epoch")
      .as_nanos();
    path.push(format!(
      "reviu-{prefix}-bare-{}-{nanos}",
      std::process::id()
    ));
    std::fs::create_dir_all(&path).expect("create temp dir");
    Repository::init_bare(&path).expect("init bare git repository");
    Self { path }
  }
}

impl Drop for TempBareRepo {
  fn drop(&mut self) {
    let _ = std::fs::remove_dir_all(&self.path);
  }
}

pub(crate) fn push_branch_to_remote(repo_root: &Path, branch_name: &str, remote_name: &str) {
  let repo = Repository::open(repo_root).expect("open repo");
  let mut remote = repo.find_remote(remote_name).expect("find remote");
  let refspec = format!("refs/heads/{branch_name}:refs/heads/{branch_name}");
  let mut callbacks = RemoteCallbacks::new();
  callbacks.credentials(|_, _, _| Cred::default());
  let mut options = PushOptions::new();
  options.remote_callbacks(callbacks);
  remote
    .push(&[refspec], Some(&mut options))
    .expect("push branch");
}

pub(crate) fn set_upstream(repo_root: &Path, local_branch: &str, upstream_branch: &str) {
  let repo = Repository::open(repo_root).expect("open repo");
  let mut branch = repo
    .find_branch(local_branch, BranchType::Local)
    .expect("find local branch");
  branch
    .set_upstream(Some(upstream_branch))
    .expect("set upstream");
}

pub(crate) fn set_remote_head(remote_root: &Path, branch_name: &str) {
  let refname = format!("refs/heads/{branch_name}");
  Repository::open(remote_root)
    .expect("open remote")
    .set_head(&refname)
    .expect("set remote HEAD");
}

pub(crate) fn head_oid(repo_root: &Path) -> git2::Oid {
  Repository::open(repo_root)
    .expect("open repo")
    .head()
    .and_then(|head| head.peel_to_commit())
    .expect("read head")
    .id()
}

pub(crate) fn remote_branch_oid(remote_root: &Path, branch_name: &str) -> git2::Oid {
  let refname = format!("refs/heads/{branch_name}");
  Repository::open(remote_root)
    .expect("open remote")
    .refname_to_id(&refname)
    .expect("read remote branch oid")
}

pub(crate) fn force_checkout_head(repo_root: &Path) {
  let repo = Repository::open(repo_root).expect("open repo");
  let mut checkout = CheckoutBuilder::new();
  checkout.force();
  repo
    .checkout_head(Some(&mut checkout))
    .expect("force checkout HEAD");
}
