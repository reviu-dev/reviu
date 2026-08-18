//! Branch-side git state for the shell: branch status, branches and stashes,
//! loaded in the background and served to the palette and the sidebar.

use std::path::PathBuf;

use gpui::{Context, EventEmitter, SharedString, Task, prelude::*};

pub struct RepoSnapshot {
  repo_root: Option<PathBuf>,
  branch_status: Option<git::BranchStatus>,
  branches: Vec<git::BranchRef>,
  upstream_branch: Option<git::BranchRef>,
  default_branch: Option<git::BranchRef>,
  stashes: Vec<git::StashEntry>,
  default_stash_message: Option<SharedString>,
  _refresh_task: Option<Task<()>>,
}

pub enum RepoSnapshotEvent {
  Refreshed,
}

impl EventEmitter<RepoSnapshotEvent> for RepoSnapshot {}

impl RepoSnapshot {
  pub fn new(repo_root: Option<PathBuf>) -> Self {
    Self {
      repo_root,
      branch_status: None,
      branches: Vec::new(),
      upstream_branch: None,
      default_branch: None,
      stashes: Vec::new(),
      default_stash_message: None,
      _refresh_task: None,
    }
  }

  /// Switching repositories drops the previous state at once: stale branches
  /// must not reach the palette while the new ones load.
  pub fn set_repo_root(&mut self, repo_root: Option<PathBuf>, cx: &mut Context<Self>) {
    self.repo_root = repo_root;
    self.branch_status = None;
    self.branches = Vec::new();
    self.upstream_branch = None;
    self.default_branch = None;
    self.stashes = Vec::new();
    self.default_stash_message = None;
    cx.notify();
  }

  pub fn refresh(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.repo_root.clone() else {
      return;
    };
    let task = cx.spawn(async move |this, cx| {
      let load_root = repo_root.clone();
      let (status, branches, upstream, default_branch, stashes, default_stash_message) = cx
        .background_spawn(async move {
          (
            git::current_branch_status(&load_root),
            git::list_branches(&load_root),
            git::current_branch_upstream(&load_root),
            git::default_remote_branch(&load_root),
            git::list_stashes(&load_root),
            git::default_stash_message(&load_root),
          )
        })
        .await;
      let _ = this.update(cx, |this, cx| {
        // A repository switch mid-flight wins over what we just read.
        if this.repo_root.as_deref() != Some(repo_root.as_path()) {
          return;
        }
        this.branch_status = status.ok();
        this.branches = branches.unwrap_or_default();
        this.upstream_branch = upstream.ok().flatten();
        this.default_branch = default_branch.ok().flatten();
        this.stashes = stashes.unwrap_or_default();
        this.default_stash_message = default_stash_message.ok().map(Into::into);
        cx.emit(RepoSnapshotEvent::Refreshed);
        cx.notify();
      });
    });
    self._refresh_task = Some(task);
  }

  pub fn branch_status(&self) -> Option<&git::BranchStatus> {
    self.branch_status.as_ref()
  }

  pub fn branches(&self) -> &[git::BranchRef] {
    &self.branches
  }

  pub fn upstream_branch(&self) -> Option<&git::BranchRef> {
    self.upstream_branch.as_ref()
  }

  pub fn default_branch(&self) -> Option<&git::BranchRef> {
    self.default_branch.as_ref()
  }

  pub fn stashes(&self) -> &[git::StashEntry] {
    &self.stashes
  }

  pub fn default_stash_message(&self) -> Option<SharedString> {
    self.default_stash_message.clone()
  }

  pub fn current_branch_name(&self) -> Option<&str> {
    self
      .branch_status
      .as_ref()
      .map(|status| status.name.as_str())
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::test_support::{TempRepo, commit_text_file};
  use gpui::TestAppContext;
  use std::path::Path;

  #[gpui::test]
  async fn refresh_loads_the_branch_side_state(cx: &mut TestAppContext) {
    let repo = TempRepo::init("repo-snapshot-refresh");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let snapshot = cx.new(|_| RepoSnapshot::new(Some(repo.path.clone())));
    snapshot.update(cx, |snapshot, cx| snapshot.refresh(cx));
    cx.run_until_parked();

    snapshot.read_with(cx, |snapshot, _| {
      assert!(snapshot.branch_status().is_some());
      assert!(!snapshot.branches().is_empty());
      assert_eq!(
        snapshot.current_branch_name(),
        snapshot.branch_status().map(|status| status.name.as_str())
      );
    });
  }

  #[gpui::test(iterations = 10)]
  async fn switching_repo_mid_refresh_drops_the_stale_read(cx: &mut TestAppContext) {
    let repo = TempRepo::init("repo-snapshot-stale");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let snapshot = cx.new(|_| RepoSnapshot::new(Some(repo.path.clone())));
    snapshot.update(cx, |snapshot, cx| {
      snapshot.refresh(cx);
      // The switch lands before the refresh resolves.
      snapshot.set_repo_root(None, cx);
    });
    cx.run_until_parked();

    snapshot.read_with(cx, |snapshot, _| {
      assert!(snapshot.branch_status().is_none());
      assert!(snapshot.branches().is_empty());
    });
  }
}
