//! Repository fixtures shared by the tests of every module of the crate.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use git2::{BranchType, Cred, Oid, PushOptions, RemoteCallbacks, Repository, Signature};

/// Two fixtures created in the same clock tick would otherwise share a
/// directory and fight over its `.lock` files.
static TEMP_PATH_COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_dir_name(prefix: &str, process_id: u32, nanos: u128, unique: u64) -> String {
  format!("reviu-{prefix}-{process_id}-{nanos}-{unique}")
}

pub fn temp_path(prefix: &str) -> PathBuf {
  let nanos = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .expect("system clock before unix epoch")
    .as_nanos();
  let unique = TEMP_PATH_COUNTER.fetch_add(1, Ordering::Relaxed);
  let mut path = std::env::temp_dir();
  path.push(temp_dir_name(prefix, std::process::id(), nanos, unique));
  path
}

pub struct TempDir {
  pub path: PathBuf,
}

impl TempDir {
  pub fn new(prefix: &str) -> Self {
    let path = temp_path(prefix);
    std::fs::create_dir_all(&path).expect("create temp dir");
    Self { path }
  }
}

impl Drop for TempDir {
  fn drop(&mut self) {
    let _ = std::fs::remove_dir_all(&self.path);
  }
}

pub struct TempRepo {
  pub path: PathBuf,
}

impl TempRepo {
  pub fn init(prefix: &str) -> Self {
    let path = temp_path(prefix);
    std::fs::create_dir_all(&path).expect("create temp dir");
    // macOS puts temp dirs behind the /var -> /private/var symlink; hand out the
    // canonical path so comparisons with git's resolved workdir hold.
    let path = path.canonicalize().expect("canonicalize temp dir");
    let repo = Repository::init(&path).expect("init git repository");
    let mut config = repo.config().expect("open git config");
    config
      .set_str("user.name", "Reviu Tests")
      .expect("set git user.name");
    config
      .set_str("user.email", "tests@reviu.local")
      .expect("set git user.email");
    Self { path }
  }
}

impl Drop for TempRepo {
  fn drop(&mut self) {
    let _ = std::fs::remove_dir_all(&self.path);
  }
}

pub struct TempBareRepo {
  pub path: PathBuf,
}

impl TempBareRepo {
  pub fn init(prefix: &str) -> Self {
    let path = temp_path(&format!("{prefix}-bare"));
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

pub fn commit_text_file(repo_root: &Path, rel_path: &Path, contents: &str, message: &str) -> Oid {
  let repo = Repository::open(repo_root).expect("open repo");
  if let Some(parent) = rel_path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
    std::fs::create_dir_all(repo_root.join(parent)).expect("create parent dirs");
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

pub fn push_branch_to_remote(repo_root: &Path, branch_name: &str, remote_name: &str) {
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

pub fn set_upstream(repo_root: &Path, local_branch: &str, upstream_branch: &str) {
  let repo = Repository::open(repo_root).expect("open repo");
  let mut branch = repo
    .find_branch(local_branch, BranchType::Local)
    .expect("find local branch");
  branch
    .set_upstream(Some(upstream_branch))
    .expect("set upstream");
}

pub fn set_remote_head(remote_root: &Path, branch_name: &str) {
  let refname = format!("refs/heads/{branch_name}");
  Repository::open(remote_root)
    .expect("open remote")
    .set_head(&refname)
    .expect("set remote HEAD");
}

pub fn head_oid(repo_root: &Path) -> Oid {
  Repository::open(repo_root)
    .expect("open repo")
    .head()
    .and_then(|head| head.peel_to_commit())
    .expect("read head")
    .id()
}

pub fn remote_branch_oid(remote_root: &Path, branch_name: &str) -> Oid {
  let refname = format!("refs/heads/{branch_name}");
  Repository::open(remote_root)
    .expect("open remote")
    .refname_to_id(&refname)
    .expect("read remote branch oid")
}

#[cfg(test)]
mod tests {
  use super::{TempRepo, temp_dir_name, temp_path};

  #[test]
  fn two_fixtures_never_land_in_the_same_directory() {
    assert_ne!(
      temp_dir_name("git", 42, 1_000, 0),
      temp_dir_name("git", 42, 1_000, 1),
      "the same clock tick still gives each fixture its own directory"
    );

    assert_ne!(temp_path("git"), temp_path("git"));

    let repo = TempRepo::init("git");
    let other = TempRepo::init("git");
    assert_ne!(repo.path, other.path);
  }
}
