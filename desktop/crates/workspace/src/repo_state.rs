//! What the repository allows right now, independent of the page asking.

use git::{BranchStatus, RepoStatusEntry, RepoStatusKind};

use crate::changes_list::{all_entries_staged, can_stage, can_unstage, has_conflicted_entries};

/// The git commands a palette can offer. Not every one maps to a
/// [`crate::repo_command::RepoCommand`]: some open a dialog first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PaletteCommand {
  Commit,
  ContinueRebase,
  SkipRebase,
  Push,
  ForcePush,
  UndoLastCommit,
  Amend,
  CheckoutDetached,
  InteractiveRebase,
  Pull,
  MergeBranch,
  RebaseBranch,
  CherryPick,
  StageAll,
  UnstageAll,
  RestoreAll,
  Stash,
  StashWithUntracked,
  StageSelectedFile,
  UnstageSelectedFile,
}

/// A snapshot of the repository, read once when the rules are evaluated.
pub(crate) struct RepoState<'a> {
  pub(crate) has_repo: bool,
  pub(crate) merge_in_progress: bool,
  pub(crate) rebase_in_progress: bool,
  pub(crate) has_head_commit: bool,
  pub(crate) can_push: bool,
  pub(crate) can_force_push: bool,
  pub(crate) can_undo_last_commit: bool,
  pub(crate) branch_status: Option<&'a BranchStatus>,
  pub(crate) status_entries: &'a [RepoStatusEntry],
  pub(crate) selected_entry: Option<&'a RepoStatusEntry>,
  pub(crate) commit_message: &'a str,
}

impl RepoState<'_> {
  pub(crate) fn allows(&self, command: PaletteCommand) -> bool {
    match command {
      PaletteCommand::Commit => {
        !self.rebase_in_progress
          && self.has_repo
          && !self.commit_message.trim().is_empty()
          && !self.status_entries.is_empty()
          && !has_conflicted_entries(self.status_entries)
      }
      PaletteCommand::ContinueRebase => {
        self.rebase_in_progress && self.has_repo && !has_conflicted_entries(self.status_entries)
      }
      PaletteCommand::SkipRebase => self.rebase_in_progress && self.has_repo,
      PaletteCommand::Push => !self.rebase_in_progress && self.has_repo && self.can_push,
      PaletteCommand::ForcePush => !self.rebase_in_progress && self.has_repo && self.can_force_push,
      PaletteCommand::UndoLastCommit => {
        !self.rebase_in_progress && self.has_repo && self.can_undo_last_commit
      }
      PaletteCommand::Amend => !self.rebase_in_progress && self.has_repo && self.has_head_commit,
      PaletteCommand::CheckoutDetached => {
        self.has_repo
          && self.has_head_commit
          && !self.merge_in_progress
          && !self.rebase_in_progress
          && self.branch_status.is_some()
          && !self.is_detached_head()
      }
      PaletteCommand::InteractiveRebase => {
        self.no_operation_in_progress()
          && self.has_repo
          && self.has_head_commit
          && self.status_entries.is_empty()
          && !self.is_detached_head()
      }
      PaletteCommand::Pull => {
        self.no_operation_in_progress()
          && self.has_repo
          && self.branch_status.is_some_and(|status| status.has_upstream)
      }
      PaletteCommand::MergeBranch | PaletteCommand::RebaseBranch | PaletteCommand::CherryPick => {
        self.no_operation_in_progress() && self.has_repo
      }
      PaletteCommand::StageAll => {
        !self.status_entries.is_empty() && !all_entries_staged(self.status_entries)
      }
      PaletteCommand::UnstageAll => has_staged_entries(self.status_entries),
      // Same rule as the button it replaces: something to discard, in a repository.
      PaletteCommand::RestoreAll => self.has_repo && !self.status_entries.is_empty(),
      PaletteCommand::Stash => has_tracked_entries(self.status_entries),
      PaletteCommand::StashWithUntracked => {
        has_tracked_entries(self.status_entries) || has_untracked_entries(self.status_entries)
      }
      PaletteCommand::StageSelectedFile => {
        self.has_repo
          && self
            .selected_entry
            .is_some_and(|entry| can_stage(entry.stage))
      }
      PaletteCommand::UnstageSelectedFile => {
        self.has_repo
          && self
            .selected_entry
            .is_some_and(|entry| can_unstage(entry.stage))
      }
    }
  }

  pub(crate) fn is_detached_head(&self) -> bool {
    is_detached_head(self.branch_status)
  }

  fn no_operation_in_progress(&self) -> bool {
    !self.rebase_in_progress && !self.merge_in_progress
  }
}

pub(crate) fn is_detached_head(branch_status: Option<&BranchStatus>) -> bool {
  branch_status.is_some_and(|status| status.name == "HEAD")
}

pub(crate) fn has_staged_entries(entries: &[RepoStatusEntry]) -> bool {
  entries.iter().any(|entry| {
    matches!(
      entry.stage,
      git::RepoStage::Staged | git::RepoStage::PartiallyStaged
    )
  })
}

pub(crate) fn has_untracked_entries(entries: &[RepoStatusEntry]) -> bool {
  entries
    .iter()
    .any(|entry| entry.status == RepoStatusKind::Untracked)
}

pub(crate) fn has_tracked_entries(entries: &[RepoStatusEntry]) -> bool {
  entries
    .iter()
    .any(|entry| entry.status != RepoStatusKind::Untracked)
}

/// A branch with no upstream has to be published before it can be pushed.
pub(crate) fn should_publish_branch(
  branch_status: Option<&BranchStatus>,
  has_head_commit: bool,
) -> bool {
  has_head_commit
    && matches!(
      branch_status,
      Some(status) if !status.has_upstream && !is_detached_head(Some(status))
    )
}

/// `(can_push, can_force_push)`: a diverged branch only moves with a force push.
pub(crate) fn push_flags(
  branch_status: Option<&BranchStatus>,
  has_head_commit: bool,
  force_push_after_rebase: bool,
) -> (bool, bool) {
  let Some(status) = branch_status else {
    return (false, false);
  };
  if should_publish_branch(Some(status), has_head_commit) {
    return (true, false);
  }
  if !status.has_upstream {
    return (false, false);
  }
  if force_push_after_rebase && status.ahead > 0 {
    return (false, true);
  }
  let can_push = status.ahead > 0 && status.behind == 0;
  let can_force_push = status.ahead > 0 && status.behind > 0;
  (can_push, can_force_push)
}

/// Accepting every conflict at once needs a writable file that still has markers.
pub(crate) fn can_accept_all_conflicts(
  selected_status: Option<RepoStatusKind>,
  is_read_only: bool,
  has_unresolved_conflict_markers: bool,
) -> bool {
  matches!(selected_status, Some(RepoStatusKind::Conflicted))
    && !is_read_only
    && has_unresolved_conflict_markers
}

#[cfg(test)]
mod tests {
  use super::*;
  use git::RepoStage;
  use std::path::PathBuf;

  fn entry(path: &str, status: RepoStatusKind, stage: RepoStage) -> RepoStatusEntry {
    RepoStatusEntry {
      path: PathBuf::from(path),
      old_path: None,
      status,
      stage,
    }
  }

  fn modified(path: &str, stage: RepoStage) -> RepoStatusEntry {
    entry(path, RepoStatusKind::Modified, stage)
  }

  fn branch(name: &str, has_upstream: bool) -> BranchStatus {
    tracking(name, 0, 0, has_upstream)
  }

  fn tracking(name: &str, ahead: usize, behind: usize, has_upstream: bool) -> BranchStatus {
    BranchStatus {
      name: name.to_string(),
      ahead,
      behind,
      has_upstream,
    }
  }

  /// A repository with one unstaged change, on a tracked branch, nothing running.
  struct StateBuilder {
    entries: Vec<RepoStatusEntry>,
    branch: Option<BranchStatus>,
    selected: Option<RepoStatusEntry>,
    commit_message: String,
    has_repo: bool,
    merge_in_progress: bool,
    rebase_in_progress: bool,
    has_head_commit: bool,
    can_push: bool,
    can_force_push: bool,
    can_undo_last_commit: bool,
  }

  impl StateBuilder {
    fn new() -> Self {
      Self {
        entries: vec![modified("a.rs", RepoStage::Unstaged)],
        branch: Some(branch("main", true)),
        selected: None,
        commit_message: "a message".to_string(),
        has_repo: true,
        merge_in_progress: false,
        rebase_in_progress: false,
        has_head_commit: true,
        can_push: true,
        can_force_push: true,
        can_undo_last_commit: true,
      }
    }

    fn state(&self) -> RepoState<'_> {
      RepoState {
        has_repo: self.has_repo,
        merge_in_progress: self.merge_in_progress,
        rebase_in_progress: self.rebase_in_progress,
        has_head_commit: self.has_head_commit,
        can_push: self.can_push,
        can_force_push: self.can_force_push,
        can_undo_last_commit: self.can_undo_last_commit,
        branch_status: self.branch.as_ref(),
        status_entries: &self.entries,
        selected_entry: self.selected.as_ref(),
        commit_message: &self.commit_message,
      }
    }
  }

  const EVERY_COMMAND: [PaletteCommand; 20] = [
    PaletteCommand::Commit,
    PaletteCommand::ContinueRebase,
    PaletteCommand::SkipRebase,
    PaletteCommand::Push,
    PaletteCommand::ForcePush,
    PaletteCommand::UndoLastCommit,
    PaletteCommand::Amend,
    PaletteCommand::CheckoutDetached,
    PaletteCommand::InteractiveRebase,
    PaletteCommand::Pull,
    PaletteCommand::MergeBranch,
    PaletteCommand::RebaseBranch,
    PaletteCommand::CherryPick,
    PaletteCommand::StageAll,
    PaletteCommand::UnstageAll,
    PaletteCommand::RestoreAll,
    PaletteCommand::Stash,
    PaletteCommand::StashWithUntracked,
    PaletteCommand::StageSelectedFile,
    PaletteCommand::UnstageSelectedFile,
  ];

  #[test]
  fn without_a_repository_nothing_git_is_offered() {
    let mut builder = StateBuilder::new();
    builder.has_repo = false;
    builder.entries = Vec::new();
    builder.branch = None;
    let state = builder.state();

    for command in EVERY_COMMAND {
      assert!(
        !state.allows(command),
        "{command:?} should be hidden without a repository"
      );
    }
  }

  #[test]
  fn a_rebase_in_progress_leaves_only_its_own_commands() {
    let mut builder = StateBuilder::new();
    builder.rebase_in_progress = true;
    let state = builder.state();

    assert!(state.allows(PaletteCommand::ContinueRebase));
    assert!(state.allows(PaletteCommand::SkipRebase));
    for command in [
      PaletteCommand::Commit,
      PaletteCommand::Push,
      PaletteCommand::ForcePush,
      PaletteCommand::UndoLastCommit,
      PaletteCommand::Amend,
      PaletteCommand::CheckoutDetached,
      PaletteCommand::InteractiveRebase,
      PaletteCommand::Pull,
      PaletteCommand::MergeBranch,
      PaletteCommand::RebaseBranch,
      PaletteCommand::CherryPick,
    ] {
      assert!(!state.allows(command), "{command:?} during a rebase");
    }
  }

  #[test]
  fn an_unresolved_conflict_blocks_committing_and_continuing() {
    let mut builder = StateBuilder::new();
    builder.entries = vec![entry(
      "a.rs",
      RepoStatusKind::Conflicted,
      RepoStage::Unstaged,
    )];
    assert!(!builder.state().allows(PaletteCommand::Commit));

    builder.rebase_in_progress = true;
    assert!(!builder.state().allows(PaletteCommand::ContinueRebase));
    assert!(
      builder.state().allows(PaletteCommand::SkipRebase),
      "skipping is how you get out of a conflict you do not want"
    );
  }

  #[test]
  fn committing_needs_a_message_and_something_to_commit() {
    let mut builder = StateBuilder::new();
    assert!(builder.state().allows(PaletteCommand::Commit));

    builder.commit_message = "   ".to_string();
    assert!(!builder.state().allows(PaletteCommand::Commit));

    builder.commit_message = "a message".to_string();
    builder.entries = Vec::new();
    assert!(!builder.state().allows(PaletteCommand::Commit));
  }

  #[test]
  fn a_merge_in_progress_blocks_the_commands_that_would_start_another_one() {
    let mut builder = StateBuilder::new();
    builder.merge_in_progress = true;
    let state = builder.state();

    for command in [
      PaletteCommand::MergeBranch,
      PaletteCommand::RebaseBranch,
      PaletteCommand::CherryPick,
      PaletteCommand::InteractiveRebase,
      PaletteCommand::Pull,
      PaletteCommand::CheckoutDetached,
    ] {
      assert!(!state.allows(command), "{command:?} during a merge");
    }
    assert!(
      state.allows(PaletteCommand::Commit),
      "committing is how a merge ends"
    );
  }

  #[test]
  fn a_detached_head_hides_the_commands_that_need_a_branch() {
    let mut builder = StateBuilder::new();
    builder.branch = Some(branch("HEAD", false));
    builder.entries = Vec::new();
    let state = builder.state();

    assert!(state.is_detached_head());
    assert!(!state.allows(PaletteCommand::CheckoutDetached));
    assert!(!state.allows(PaletteCommand::InteractiveRebase));
  }

  #[test]
  fn pulling_needs_an_upstream() {
    let mut builder = StateBuilder::new();
    assert!(builder.state().allows(PaletteCommand::Pull));

    builder.branch = Some(branch("main", false));
    assert!(!builder.state().allows(PaletteCommand::Pull));

    builder.branch = None;
    assert!(!builder.state().allows(PaletteCommand::Pull));
  }

  #[test]
  fn an_interactive_rebase_needs_a_clean_worktree_and_a_commit() {
    let mut builder = StateBuilder::new();
    assert!(
      !builder.state().allows(PaletteCommand::InteractiveRebase),
      "uncommitted changes would be rewritten under the user"
    );

    builder.entries = Vec::new();
    assert!(builder.state().allows(PaletteCommand::InteractiveRebase));

    builder.has_head_commit = false;
    assert!(!builder.state().allows(PaletteCommand::InteractiveRebase));
  }

  #[test]
  fn the_remote_commands_follow_their_capability_flags() {
    let mut builder = StateBuilder::new();
    builder.can_push = false;
    builder.can_force_push = false;
    builder.can_undo_last_commit = false;
    let state = builder.state();

    assert!(!state.allows(PaletteCommand::Push));
    assert!(!state.allows(PaletteCommand::ForcePush));
    assert!(!state.allows(PaletteCommand::UndoLastCommit));
    assert!(
      state.allows(PaletteCommand::Amend),
      "amending only needs a commit to amend"
    );
  }

  #[test]
  fn staging_commands_follow_what_is_left_to_stage() {
    let mut builder = StateBuilder::new();
    assert!(builder.state().allows(PaletteCommand::StageAll));
    assert!(!builder.state().allows(PaletteCommand::UnstageAll));

    builder.entries = vec![modified("a.rs", RepoStage::Staged)];
    assert!(!builder.state().allows(PaletteCommand::StageAll));
    assert!(builder.state().allows(PaletteCommand::UnstageAll));

    // A partially staged file still has something to stage and to unstage.
    builder.entries = vec![modified("a.rs", RepoStage::PartiallyStaged)];
    assert!(builder.state().allows(PaletteCommand::StageAll));
    assert!(builder.state().allows(PaletteCommand::UnstageAll));

    builder.entries = Vec::new();
    assert!(!builder.state().allows(PaletteCommand::StageAll));
    assert!(!builder.state().allows(PaletteCommand::UnstageAll));
  }

  #[test]
  fn restoring_everything_needs_something_to_discard() {
    let mut builder = StateBuilder::new();
    assert!(builder.state().allows(PaletteCommand::RestoreAll));

    // Staged changes are still changes to discard.
    builder.entries = vec![modified("a.rs", RepoStage::Staged)];
    assert!(builder.state().allows(PaletteCommand::RestoreAll));

    builder.entries = Vec::new();
    assert!(
      !builder.state().allows(PaletteCommand::RestoreAll),
      "a clean working tree has nothing to restore"
    );

    builder.entries = vec![modified("a.rs", RepoStage::Unstaged)];
    builder.has_repo = false;
    assert!(!builder.state().allows(PaletteCommand::RestoreAll));
  }

  #[test]
  fn the_selected_file_commands_follow_its_stage() {
    let mut builder = StateBuilder::new();
    assert!(
      !builder.state().allows(PaletteCommand::StageSelectedFile),
      "nothing is selected"
    );

    builder.selected = Some(modified("a.rs", RepoStage::Unstaged));
    assert!(builder.state().allows(PaletteCommand::StageSelectedFile));
    assert!(!builder.state().allows(PaletteCommand::UnstageSelectedFile));

    builder.selected = Some(modified("a.rs", RepoStage::Staged));
    assert!(!builder.state().allows(PaletteCommand::StageSelectedFile));
    assert!(builder.state().allows(PaletteCommand::UnstageSelectedFile));
  }

  #[test]
  fn stashing_untracked_only_work_needs_the_untracked_variant() {
    let mut builder = StateBuilder::new();
    builder.entries = vec![entry(
      "new.rs",
      RepoStatusKind::Untracked,
      RepoStage::Unstaged,
    )];
    assert!(!builder.state().allows(PaletteCommand::Stash));
    assert!(builder.state().allows(PaletteCommand::StashWithUntracked));

    builder.entries = vec![modified("a.rs", RepoStage::Unstaged)];
    assert!(builder.state().allows(PaletteCommand::Stash));
    assert!(builder.state().allows(PaletteCommand::StashWithUntracked));

    builder.entries = Vec::new();
    assert!(!builder.state().allows(PaletteCommand::Stash));
    assert!(!builder.state().allows(PaletteCommand::StashWithUntracked));
  }

  #[test]
  fn push_flags_respect_upstream_and_divergence() {
    // No upstream: the branch has to be published, and only if it has a commit.
    let no_upstream = tracking("main", 3, 0, false);
    assert_eq!(push_flags(Some(&no_upstream), false, false), (false, false));
    assert_eq!(push_flags(Some(&no_upstream), true, false), (true, false));

    let clean_ahead = tracking("main", 2, 0, true);
    assert_eq!(push_flags(Some(&clean_ahead), true, false), (true, false));

    let diverged = tracking("main", 1, 2, true);
    assert_eq!(push_flags(Some(&diverged), true, false), (false, true));

    let behind_only = tracking("main", 0, 2, true);
    assert_eq!(push_flags(Some(&behind_only), true, false), (false, false));

    assert_eq!(push_flags(None, true, false), (false, false));
  }

  #[test]
  fn a_rebase_turns_the_next_push_into_a_force_push() {
    let clean_ahead = tracking("main", 2, 0, true);
    assert_eq!(push_flags(Some(&clean_ahead), true, true), (false, true));
    assert_eq!(push_flags(Some(&clean_ahead), true, false), (true, false));

    // Nothing ahead: a rebase that changed nothing needs no force push.
    let no_ahead = tracking("main", 0, 0, true);
    assert_eq!(push_flags(Some(&no_ahead), true, true), (false, false));
  }

  #[test]
  fn a_detached_head_is_never_published() {
    let detached = tracking("HEAD", 0, 0, false);
    assert!(!should_publish_branch(Some(&detached), true));
    assert!(should_publish_branch(
      Some(&tracking("feature", 0, 0, false)),
      true
    ));
    assert!(!should_publish_branch(None, true));
  }

  #[test]
  fn accepting_every_conflict_needs_a_writable_file_with_markers() {
    assert!(can_accept_all_conflicts(
      Some(RepoStatusKind::Conflicted),
      false,
      true
    ));
    assert!(!can_accept_all_conflicts(
      Some(RepoStatusKind::Conflicted),
      true,
      true
    ));
    assert!(!can_accept_all_conflicts(
      Some(RepoStatusKind::Conflicted),
      false,
      false
    ));
    assert!(!can_accept_all_conflicts(
      Some(RepoStatusKind::Modified),
      false,
      true
    ));
    assert!(!can_accept_all_conflicts(None, false, true));
  }
}
