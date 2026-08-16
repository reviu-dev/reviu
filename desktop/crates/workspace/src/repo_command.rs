//! Git commands as data, independent of the page running them.

use std::path::{Path, PathBuf};

use git::{
  BranchKind, BranchRef, MergeBranchOutcome, PullOutcome, RebaseBranchOutcome, RepoStatusKind,
  abort_merge, abort_rebase, apply_stash, checkout_detached_target, cherry_pick_commits,
  continue_rebase, create_branch, create_branch_from, create_stash, current_branch_status,
  current_rebase_commit_message, delete_branch, drop_stash, fetch, list_repo_status, merge_branch,
  pop_stash, pull, push, rebase_branch, skip_rebase, stage_all, switch_branch, undo_last_commit,
  unstage_all,
};
use gpui::SharedString;
use ui::{CommandPaletteBranch, CommandPaletteBranchKind};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RepoCommand {
  StageAll,
  UnstageAll,
  Push,
  ForcePush,
  Pull,
  Fetch,
  UndoLastCommit,
  CheckoutDetached {
    target: String,
  },
  SwitchBranch(BranchRef),
  CreateBranch {
    name: String,
  },
  CreateBranchFrom {
    name: String,
    base: BranchRef,
  },
  DeleteBranch(BranchRef),
  MergeBranch(BranchRef),
  AbortMerge,
  RebaseBranch(BranchRef),
  ContinueRebase,
  SkipRebase,
  AbortRebase,
  Stash {
    include_untracked: bool,
    message: Option<String>,
  },
  ApplyStash {
    index: usize,
    name: String,
  },
  PopStash {
    index: usize,
    name: String,
  },
  DropStash {
    index: usize,
    name: String,
  },
  CherryPick {
    commit_hashes: Vec<String>,
  },
}

/// What the caller has to react to once the command ran.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RepoCommandOutcome {
  Done {
    message: SharedString,
  },
  /// Nothing to do: the branch was already in sync.
  UpToDate {
    message: SharedString,
  },
  /// The command stopped on conflicts; the file is waiting to be resolved.
  Conflicted {
    path: PathBuf,
    commit_message: Option<String>,
    /// Kept for telemetry: git failed, even though the user only sees conflicts.
    error: String,
  },
}

impl RepoCommandOutcome {
  fn done(message: impl Into<SharedString>) -> Self {
    Self::Done {
      message: message.into(),
    }
  }
}

impl RepoCommand {
  /// Blocking: run it off the main thread.
  pub(crate) fn run(&self, repo_root: &Path) -> anyhow::Result<RepoCommandOutcome> {
    match self {
      Self::StageAll => {
        stage_all(repo_root).map(|()| RepoCommandOutcome::done("Staged all changes"))
      }
      Self::UnstageAll => {
        unstage_all(repo_root).map(|()| RepoCommandOutcome::done("Unstaged all changes"))
      }
      Self::Push => {
        push(repo_root, false).map(|()| RepoCommandOutcome::done("Pushed to the remote branch"))
      }
      Self::ForcePush => push(repo_root, true)
        .map(|()| RepoCommandOutcome::done("Force pushed to the remote branch")),
      Self::Pull => pull(repo_root).map(|outcome| match outcome {
        PullOutcome::AlreadyUpToDate => RepoCommandOutcome::UpToDate {
          message: "Already up to date".into(),
        },
        PullOutcome::Pulled => RepoCommandOutcome::done("Pulled from the remote branch"),
      }),
      Self::Fetch => fetch(repo_root).map(|()| RepoCommandOutcome::done("Fetched from remotes")),
      Self::UndoLastCommit => {
        undo_last_commit(repo_root).map(|()| RepoCommandOutcome::done("Undid the last commit"))
      }
      Self::CheckoutDetached { target } => checkout_detached_target(repo_root, target)
        .map(|()| RepoCommandOutcome::done(format!("Checked out {target}"))),
      Self::SwitchBranch(branch) => switch_branch(repo_root, branch)
        .map(|()| RepoCommandOutcome::done(format!("Switched to {}", branch.name))),
      Self::CreateBranch { name } => {
        let created = BranchRef {
          name: name.clone(),
          kind: BranchKind::Local,
        };
        create_branch(repo_root, name)
          .and_then(|()| switch_branch(repo_root, &created))
          .map(|()| RepoCommandOutcome::done(format!("Created branch {name}")))
      }
      Self::CreateBranchFrom { name, base } => {
        let created = BranchRef {
          name: name.clone(),
          kind: BranchKind::Local,
        };
        create_branch_from(repo_root, name, base)
          .and_then(|()| switch_branch(repo_root, &created))
          .map(|()| RepoCommandOutcome::done(format!("Created branch {name}")))
      }
      Self::DeleteBranch(branch) => delete_branch(repo_root, branch)
        .map(|()| RepoCommandOutcome::done(format!("Deleted branch {}", branch.name))),
      Self::MergeBranch(branch) => match merge_branch(repo_root, branch) {
        Ok(MergeBranchOutcome::AlreadyUpToDate) => Ok(RepoCommandOutcome::UpToDate {
          message: format!("Already up to date with {}", branch.name).into(),
        }),
        Ok(MergeBranchOutcome::Merged) => {
          Ok(RepoCommandOutcome::done(format!("Merged {}", branch.name)))
        }
        Err(error) => {
          conflicted_or_error(repo_root, error, merge_conflict_message(repo_root, branch))
        }
      },
      Self::AbortMerge => {
        abort_merge(repo_root).map(|()| RepoCommandOutcome::done("Aborted merge"))
      }
      Self::RebaseBranch(branch) => match rebase_branch(repo_root, branch) {
        Ok(RebaseBranchOutcome::AlreadyUpToDate) => Ok(RepoCommandOutcome::UpToDate {
          message: format!("Already up to date with {}", branch.name).into(),
        }),
        Ok(RebaseBranchOutcome::Rebased) => Ok(RepoCommandOutcome::done(format!(
          "Rebased onto {}",
          branch.name
        ))),
        Err(error) => conflicted_or_error(repo_root, error, rebase_conflict_message(repo_root)),
      },
      Self::ContinueRebase => match continue_rebase(repo_root) {
        Ok(()) => Ok(RepoCommandOutcome::done("Continued the rebase")),
        Err(error) => conflicted_or_error(repo_root, error, rebase_conflict_message(repo_root)),
      },
      Self::SkipRebase => match skip_rebase(repo_root) {
        Ok(()) => Ok(RepoCommandOutcome::done("Skipped the commit")),
        Err(error) => conflicted_or_error(repo_root, error, rebase_conflict_message(repo_root)),
      },
      Self::AbortRebase => {
        abort_rebase(repo_root).map(|()| RepoCommandOutcome::done("Aborted rebase"))
      }
      Self::Stash {
        include_untracked,
        message,
      } => create_stash(repo_root, *include_untracked, message.as_deref())
        .map(|()| RepoCommandOutcome::done("Stashed changes")),
      Self::ApplyStash { index, name } => apply_stash(repo_root, *index)
        .map(|()| RepoCommandOutcome::done(format!("Applied stash {name}"))),
      Self::PopStash { index, name } => pop_stash(repo_root, *index)
        .map(|()| RepoCommandOutcome::done(format!("Popped stash {name}"))),
      Self::DropStash { index, name } => drop_stash(repo_root, *index)
        .map(|()| RepoCommandOutcome::done(format!("Dropped stash {name}"))),
      Self::CherryPick { commit_hashes } => {
        let count = commit_hashes.len();
        cherry_pick_commits(repo_root, commit_hashes).map(|()| {
          let label = if count == 1 { "commit" } else { "commits" };
          RepoCommandOutcome::done(format!("Cherry-picked {count} {label}"))
        })
      }
    }
  }

  /// Sentry key for the failures of this command.
  pub(crate) fn telemetry_key(&self) -> &'static str {
    match self {
      Self::StageAll => "git.stage_all",
      Self::UnstageAll => "git.unstage_all",
      Self::Push => "git.push",
      Self::ForcePush => "git.force_push",
      Self::Pull => "git.pull",
      Self::Fetch => "git.fetch",
      Self::UndoLastCommit => "git.undo_last_commit",
      Self::CheckoutDetached { .. } => "git.checkout_detached",
      Self::SwitchBranch(_) => "git.switch_branch",
      Self::CreateBranch { .. } | Self::CreateBranchFrom { .. } => "git.create_branch",
      Self::DeleteBranch(_) => "git.delete_branch",
      Self::MergeBranch(_) => "git.merge",
      Self::AbortMerge => "git.merge.abort",
      Self::RebaseBranch(_) => "git.rebase",
      Self::ContinueRebase => "git.rebase.continue",
      Self::SkipRebase => "git.rebase.skip",
      Self::AbortRebase => "git.rebase.abort",
      Self::Stash { .. } => "git.stash",
      Self::ApplyStash { .. } => "git.stash.apply",
      Self::PopStash { .. } => "git.stash.pop",
      Self::DropStash { .. } => "git.stash.drop",
      Self::CherryPick { .. } => "git.cherry_pick",
    }
  }

  /// Breadcrumb name, used as "<label> started" / "<label> failed".
  pub(crate) fn label(&self) -> &'static str {
    match self {
      Self::StageAll => "Stage all",
      Self::UnstageAll => "Unstage all",
      Self::Push => "Push",
      Self::ForcePush => "Force push",
      Self::Pull => "Pull",
      Self::Fetch => "Fetch",
      Self::UndoLastCommit => "Undo last commit",
      Self::CheckoutDetached { .. } => "Checkout detached",
      Self::SwitchBranch(_) => "Switch branch",
      Self::CreateBranch { .. } | Self::CreateBranchFrom { .. } => "Create branch",
      Self::DeleteBranch(_) => "Delete branch",
      Self::MergeBranch(_) => "Merge",
      Self::AbortMerge => "Abort merge",
      Self::RebaseBranch(_) => "Rebase",
      Self::ContinueRebase => "Continue rebase",
      Self::SkipRebase => "Skip rebase",
      Self::AbortRebase => "Abort rebase",
      Self::Stash { .. } => "Stash",
      Self::ApplyStash { .. } => "Apply stash",
      Self::PopStash { .. } => "Pop stash",
      Self::DropStash { .. } => "Drop stash",
      Self::CherryPick { .. } => "Cherry-pick",
    }
  }

  pub(crate) fn analytics_event(&self) -> Option<&'static str> {
    match self {
      Self::Fetch => Some("fetch_done"),
      Self::RebaseBranch(_) => Some("rebase_done"),
      Self::Stash { .. } => Some("stash_created"),
      Self::CherryPick { .. } => Some("cherry_pick_done"),
      _ => None,
    }
  }
}

/// Conflicts are an expected stop, not a failure: report the file to resolve.
fn conflicted_or_error(
  repo_root: &Path,
  error: anyhow::Error,
  commit_message: Option<String>,
) -> anyhow::Result<RepoCommandOutcome> {
  match first_conflicted_path(repo_root) {
    Some(path) => Ok(RepoCommandOutcome::Conflicted {
      path,
      commit_message,
      error: error.to_string(),
    }),
    None => Err(error),
  }
}

pub(crate) fn first_conflicted_path(repo_root: &Path) -> Option<PathBuf> {
  list_repo_status(repo_root)
    .ok()?
    .into_iter()
    .find(|entry| entry.status == RepoStatusKind::Conflicted)
    .map(|entry| entry.path)
}

pub(crate) fn merge_commit_message(source_branch: &str, target_branch: &str) -> String {
  format!("Merge branch '{source_branch}' into {target_branch}")
}

fn merge_conflict_message(repo_root: &Path, branch: &BranchRef) -> Option<String> {
  let target = current_branch_status(repo_root)
    .ok()
    .map(|status| status.name)
    .unwrap_or_else(|| "HEAD".to_string());
  Some(merge_commit_message(branch.name.as_str(), target.as_str()))
}

fn rebase_conflict_message(repo_root: &Path) -> Option<String> {
  current_rebase_commit_message(repo_root).ok().flatten()
}

pub(crate) fn branch_ref_from_palette(branch: &CommandPaletteBranch) -> BranchRef {
  BranchRef {
    name: branch.name.to_string(),
    kind: match branch.kind {
      CommandPaletteBranchKind::Local => BranchKind::Local,
      CommandPaletteBranchKind::Remote => BranchKind::Remote,
    },
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use git::{RepoStage, list_repo_status};
  use std::fs;

  use crate::test_support::{TempRepo, commit_text_file};

  fn local(name: &str) -> BranchRef {
    BranchRef {
      name: name.to_string(),
      kind: BranchKind::Local,
    }
  }

  fn run(repo_root: &Path, command: RepoCommand) -> RepoCommandOutcome {
    command
      .run(repo_root)
      .unwrap_or_else(|error| panic!("{} failed: {error}", command.label()))
  }

  #[test]
  fn staging_and_unstaging_move_the_whole_worktree() {
    let repo = TempRepo::init("repo-command-stage-all");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");
    fs::write(repo.path.join("a.txt"), "v2\n").expect("update file");

    let outcome = run(&repo.path, RepoCommand::StageAll);
    assert_eq!(
      outcome,
      RepoCommandOutcome::done("Staged all changes"),
      "the message goes straight to the notification"
    );
    let staged = list_repo_status(&repo.path).expect("status");
    assert!(staged.iter().all(|entry| entry.stage == RepoStage::Staged));

    run(&repo.path, RepoCommand::UnstageAll);
    let unstaged = list_repo_status(&repo.path).expect("status");
    assert!(
      unstaged
        .iter()
        .all(|entry| entry.stage == RepoStage::Unstaged)
    );
  }

  #[test]
  fn branch_commands_create_switch_and_delete() {
    let repo = TempRepo::init("repo-command-branches");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");
    let initial = current_branch_status(&repo.path)
      .expect("branch status")
      .name;

    run(
      &repo.path,
      RepoCommand::CreateBranch {
        name: "feature".to_string(),
      },
    );
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("branch status")
        .name,
      "feature",
      "creating a branch switches to it"
    );

    run(&repo.path, RepoCommand::SwitchBranch(local(&initial)));
    run(&repo.path, RepoCommand::DeleteBranch(local("feature")));
    let branches = git::list_branches(&repo.path).expect("branches");
    assert!(!branches.iter().any(|branch| branch.name == "feature"));
  }

  #[test]
  fn creating_a_branch_from_a_base_starts_at_that_base() {
    let repo = TempRepo::init("repo-command-branch-from");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");
    let base = current_branch_status(&repo.path)
      .expect("branch status")
      .name;
    run(
      &repo.path,
      RepoCommand::CreateBranch {
        name: "other".to_string(),
      },
    );
    commit_text_file(&repo.path, Path::new("b.txt"), "only here\n", "second");

    run(
      &repo.path,
      RepoCommand::CreateBranchFrom {
        name: "from-base".to_string(),
        base: local(&base),
      },
    );

    assert_eq!(
      current_branch_status(&repo.path)
        .expect("branch status")
        .name,
      "from-base"
    );
    assert!(
      !repo.path.join("b.txt").exists(),
      "the branch starts from the base, not from the current branch"
    );
  }

  #[test]
  fn merging_reports_up_to_date_separately_from_a_merge() {
    let repo = TempRepo::init("repo-command-merge");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");
    let base = current_branch_status(&repo.path)
      .expect("branch status")
      .name;
    run(
      &repo.path,
      RepoCommand::CreateBranch {
        name: "feature".to_string(),
      },
    );
    commit_text_file(&repo.path, Path::new("b.txt"), "feature\n", "feature work");
    run(&repo.path, RepoCommand::SwitchBranch(local(&base)));

    let merged = run(&repo.path, RepoCommand::MergeBranch(local("feature")));
    assert_eq!(merged, RepoCommandOutcome::done("Merged feature"));
    assert!(repo.path.join("b.txt").exists());

    let again = run(&repo.path, RepoCommand::MergeBranch(local("feature")));
    assert_eq!(
      again,
      RepoCommandOutcome::UpToDate {
        message: "Already up to date with feature".into()
      }
    );
  }

  #[test]
  fn a_conflicting_merge_names_the_file_and_the_commit_message() {
    let repo = TempRepo::init("repo-command-merge-conflict");
    commit_text_file(&repo.path, Path::new("a.txt"), "base\n", "initial");
    let base = current_branch_status(&repo.path)
      .expect("branch status")
      .name;
    run(
      &repo.path,
      RepoCommand::CreateBranch {
        name: "feature".to_string(),
      },
    );
    commit_text_file(&repo.path, Path::new("a.txt"), "feature\n", "feature work");
    run(&repo.path, RepoCommand::SwitchBranch(local(&base)));
    commit_text_file(&repo.path, Path::new("a.txt"), "main\n", "main work");

    let outcome = run(&repo.path, RepoCommand::MergeBranch(local("feature")));
    // Conflicts are a stop to resolve, not a failure.
    let RepoCommandOutcome::Conflicted {
      path,
      commit_message,
      error,
    } = outcome
    else {
      panic!("the merge should stop on the conflicted file");
    };
    assert_eq!(path, PathBuf::from("a.txt"));
    assert_eq!(commit_message, Some(merge_commit_message("feature", &base)));
    assert!(!error.is_empty(), "the git error is kept for telemetry");
  }

  #[test]
  fn a_conflicting_rebase_hands_back_the_rebase_commit_message() {
    let repo = TempRepo::init("repo-command-rebase-conflict");
    commit_text_file(&repo.path, Path::new("a.txt"), "base\n", "initial");
    let base = current_branch_status(&repo.path)
      .expect("branch status")
      .name;
    run(
      &repo.path,
      RepoCommand::CreateBranch {
        name: "feature".to_string(),
      },
    );
    commit_text_file(&repo.path, Path::new("a.txt"), "feature\n", "feature work");
    run(&repo.path, RepoCommand::SwitchBranch(local(&base)));
    commit_text_file(&repo.path, Path::new("a.txt"), "main\n", "main work");
    run(&repo.path, RepoCommand::SwitchBranch(local("feature")));

    let outcome = run(&repo.path, RepoCommand::RebaseBranch(local(&base)));
    let RepoCommandOutcome::Conflicted {
      path,
      commit_message,
      ..
    } = outcome
    else {
      panic!("the rebase should stop on the conflicted file");
    };
    assert_eq!(path, PathBuf::from("a.txt"));
    assert_eq!(commit_message.as_deref(), Some("feature work"));

    run(&repo.path, RepoCommand::AbortRebase);
    assert!(!git::is_rebase_in_progress(&repo.path).expect("rebase state"));
  }

  #[test]
  fn a_failure_without_conflicts_stays_an_error() {
    let repo = TempRepo::init("repo-command-missing-branch");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");

    let error = RepoCommand::SwitchBranch(local("does-not-exist"))
      .run(&repo.path)
      .expect_err("switching to a missing branch fails");
    assert!(!error.to_string().is_empty());
  }

  #[test]
  fn stashing_puts_the_changes_aside_and_pop_brings_them_back() {
    let repo = TempRepo::init("repo-command-stash");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");
    fs::write(repo.path.join("a.txt"), "v2\n").expect("update file");

    run(
      &repo.path,
      RepoCommand::Stash {
        include_untracked: false,
        message: Some("wip".to_string()),
      },
    );
    assert_eq!(
      fs::read_to_string(repo.path.join("a.txt")).expect("read file"),
      "v1\n"
    );

    let popped = run(
      &repo.path,
      RepoCommand::PopStash {
        index: 0,
        name: "wip".to_string(),
      },
    );
    assert_eq!(popped, RepoCommandOutcome::done("Popped stash wip"));
    assert_eq!(
      fs::read_to_string(repo.path.join("a.txt")).expect("read file"),
      "v2\n"
    );
  }

  #[test]
  fn cherry_pick_names_how_many_commits_landed() {
    let repo = TempRepo::init("repo-command-cherry-pick");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");
    let base = current_branch_status(&repo.path)
      .expect("branch status")
      .name;
    run(
      &repo.path,
      RepoCommand::CreateBranch {
        name: "feature".to_string(),
      },
    );
    commit_text_file(&repo.path, Path::new("b.txt"), "feature\n", "feature work");
    let sha = git::current_head_sha(&repo.path)
      .expect("head sha")
      .expect("head sha");
    run(&repo.path, RepoCommand::SwitchBranch(local(&base)));

    let outcome = run(
      &repo.path,
      RepoCommand::CherryPick {
        commit_hashes: vec![sha],
      },
    );
    assert_eq!(outcome, RepoCommandOutcome::done("Cherry-picked 1 commit"));
    assert!(repo.path.join("b.txt").exists());
  }

  #[test]
  fn undoing_the_last_commit_keeps_the_work_in_the_worktree() {
    let repo = TempRepo::init("repo-command-undo");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");
    commit_text_file(&repo.path, Path::new("b.txt"), "v1\n", "second");

    run(&repo.path, RepoCommand::UndoLastCommit);

    assert!(repo.path.join("b.txt").exists());
    let entries = list_repo_status(&repo.path).expect("status");
    assert!(
      entries
        .iter()
        .any(|entry| entry.path == PathBuf::from("b.txt"))
    );
  }

  #[test]
  fn checking_out_detached_leaves_the_branch_behind() {
    let repo = TempRepo::init("repo-command-detached");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");
    let sha = git::current_head_sha(&repo.path)
      .expect("head sha")
      .expect("head sha");

    run(&repo.path, RepoCommand::CheckoutDetached { target: sha });

    assert_eq!(
      current_branch_status(&repo.path)
        .expect("branch status")
        .name,
      "HEAD"
    );
  }

  #[test]
  fn the_palette_branch_kind_maps_to_the_git_one() {
    let remote = branch_ref_from_palette(&CommandPaletteBranch {
      name: "origin/main".into(),
      kind: CommandPaletteBranchKind::Remote,
    });
    assert_eq!(remote.kind, BranchKind::Remote);
    assert_eq!(remote.name, "origin/main");

    let local_branch = branch_ref_from_palette(&CommandPaletteBranch {
      name: "main".into(),
      kind: CommandPaletteBranchKind::Local,
    });
    assert_eq!(local_branch.kind, BranchKind::Local);
  }
}
