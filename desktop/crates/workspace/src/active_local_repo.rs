use std::{
  path::{Path, PathBuf},
  sync::{Arc, Mutex},
};

use git::{current_github_remote_repo, current_head_sha};
use gpui::{App, Global};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveLocalRepo {
  pub repo_root: PathBuf,
  pub github_owner: Option<String>,
  pub github_repo: Option<String>,
  pub current_branch: Option<String>,
  pub head_sha: Option<String>,
  pub has_uncommitted_changes: bool,
}

#[derive(Clone, Default)]
pub struct ActiveLocalRepoStore {
  state: Arc<Mutex<Option<ActiveLocalRepo>>>,
}

impl Global for ActiveLocalRepoStore {}

impl ActiveLocalRepoStore {
  pub fn get(cx: &App) -> Option<ActiveLocalRepo> {
    cx.try_global::<Self>()
      .and_then(|store| store.state.lock().ok())
      .and_then(|state| state.clone())
  }

  /// Written from a poll, so a missing store is a no-op rather than a crash.
  pub fn set(cx: &mut App, repo: Option<ActiveLocalRepo>) {
    let Some(store) = cx.try_global::<Self>() else {
      return;
    };
    if let Ok(mut state) = store.state.lock() {
      *state = repo;
    }
  }
}

/// What the pull request page needs to know about the repository open in the
/// shell: reads git, so it belongs on a background task.
pub(crate) fn active_local_repo_snapshot(
  repo_root: &Path,
  current_branch: Option<String>,
  has_uncommitted_changes: bool,
) -> ActiveLocalRepo {
  let github_remote = current_github_remote_repo(repo_root).ok().flatten();
  ActiveLocalRepo {
    repo_root: repo_root.to_path_buf(),
    github_owner: github_remote.as_ref().map(|remote| remote.owner.clone()),
    github_repo: github_remote.as_ref().map(|remote| remote.repo.clone()),
    current_branch,
    head_sha: current_head_sha(repo_root).ok().flatten(),
    has_uncommitted_changes,
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::test_support::{TempRepo, commit_text_file};

  #[test]
  fn a_snapshot_carries_the_github_remote_and_the_head() {
    let repo = TempRepo::init("active-local-repo");
    let head = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    git2::Repository::open(&repo.path)
      .expect("open repo")
      .remote("origin", "git@github.com:acme/widget.git")
      .expect("add remote");

    let snapshot = active_local_repo_snapshot(&repo.path, Some("feature".to_string()), true);

    assert_eq!(snapshot.repo_root, repo.path);
    assert_eq!(snapshot.github_owner.as_deref(), Some("acme"));
    assert_eq!(snapshot.github_repo.as_deref(), Some("widget"));
    assert_eq!(snapshot.current_branch.as_deref(), Some("feature"));
    assert_eq!(
      snapshot.head_sha.as_deref(),
      Some(head.to_string().as_str())
    );
    assert!(snapshot.has_uncommitted_changes);
  }

  #[test]
  fn a_repository_without_a_github_remote_still_makes_a_snapshot() {
    let repo = TempRepo::init("active-local-repo-no-remote");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let snapshot = active_local_repo_snapshot(&repo.path, None, false);

    assert_eq!(snapshot.github_owner, None);
    assert_eq!(snapshot.github_repo, None);
    assert!(
      snapshot.head_sha.is_some(),
      "the pull request page still needs the head to compare against"
    );
    assert!(!snapshot.has_uncommitted_changes);
  }
}
