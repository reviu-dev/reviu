//! Preparing an interactive rebase: what it will touch, and what to say about it.

use std::path::Path;

use git::{InteractiveRebasePreview, InteractiveRebaseTarget, list_interactive_rebase_commits};
use gpui::SharedString;

pub(crate) fn success_message(target: &InteractiveRebaseTarget) -> String {
  match target {
    InteractiveRebaseTarget::Branch(branch) => {
      format!("Rebased interactively onto {}", branch.name)
    }
    InteractiveRebaseTarget::BranchInPlace(branch) => {
      format!("Edited commits since {}", branch.name)
    }
    InteractiveRebaseTarget::HeadCount(count) => {
      format!("Rebased last {count} commits")
    }
  }
}

/// The commits the rebase would replay, or why there is nothing to do.
pub(crate) fn prepare_commits(
  repo_root: &Path,
  target: &InteractiveRebaseTarget,
) -> Result<InteractiveRebasePreview, SharedString> {
  let preview = list_interactive_rebase_commits(repo_root, target)
    .map_err(|err| -> SharedString { format!("Action failed: {err}").into() })?;
  if preview.commits.is_empty() {
    return Err("No commits available for interactive rebase.".into());
  }
  Ok(preview)
}

/// Merge commits cannot be replayed as picks: the user has to agree to lose them.
pub(crate) fn dropped_merges_message(dropped_merge_count: usize) -> Option<SharedString> {
  match dropped_merge_count {
    0 => None,
    1 => Some(
      "1 merge commit will be dropped from the rebase. Its changes will be re-applied through the picked commits."
        .into(),
    ),
    count => Some(
      format!(
        "{count} merge commits will be dropped from the rebase. Their changes will be re-applied through the picked commits."
      )
      .into(),
    ),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::test_support::{TempRepo, commit_text_file};
  use git::{BranchKind, BranchRef};

  #[test]
  fn the_success_message_names_what_was_rebased() {
    let branch = BranchRef {
      name: "main".to_string(),
      kind: BranchKind::Local,
    };
    assert_eq!(
      success_message(&InteractiveRebaseTarget::Branch(branch.clone())),
      "Rebased interactively onto main"
    );
    assert_eq!(
      success_message(&InteractiveRebaseTarget::BranchInPlace(branch)),
      "Edited commits since main"
    );
    assert_eq!(
      success_message(&InteractiveRebaseTarget::HeadCount(3)),
      "Rebased last 3 commits"
    );
  }

  #[test]
  fn dropped_merges_are_announced_once_there_are_any() {
    assert!(dropped_merges_message(0).is_none());
    assert!(
      dropped_merges_message(1)
        .expect("message")
        .starts_with("1 merge commit will be dropped")
    );
    assert!(
      dropped_merges_message(4)
        .expect("message")
        .starts_with("4 merge commits will be dropped")
    );
  }

  #[test]
  fn preparing_lists_the_commits_and_refuses_an_empty_range() {
    let repo = TempRepo::init("interactive-rebase-prepare");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "first");
    commit_text_file(&repo.path, Path::new("a.txt"), "v2\n", "second");
    commit_text_file(&repo.path, Path::new("a.txt"), "v3\n", "third");

    let preview = prepare_commits(&repo.path, &InteractiveRebaseTarget::HeadCount(2))
      .expect("two commits to replay");
    assert_eq!(preview.commits.len(), 2);
    assert_eq!(preview.dropped_merge_count, 0);

    // Rebasing a branch onto itself replays nothing.
    let current = BranchRef {
      name: git::current_branch_status(&repo.path)
        .expect("branch status")
        .name,
      kind: BranchKind::Local,
    };
    let error = prepare_commits(&repo.path, &InteractiveRebaseTarget::Branch(current))
      .expect_err("nothing to replay");
    assert_eq!(
      error.as_ref(),
      "No commits available for interactive rebase."
    );

    // git refuses the range itself: the error travels as-is.
    let error = prepare_commits(&repo.path, &InteractiveRebaseTarget::HeadCount(0))
      .expect_err("an impossible range");
    assert!(error.starts_with("Action failed:"));
  }
}
