use super::*;

impl SessionPage {
  #[cfg(any(test, feature = "test-support"))]
  #[doc(hidden)]
  pub fn git_state_for_driver(&self, cx: &App) -> serde_json::Value {
    let repo_root = self.checkout_root(cx);
    let panel = self.dock_panel.read(cx);
    let head_status = panel.head_status();
    let commit_message = panel.commit_message(cx).to_string();

    let Some(repo_root) = repo_root else {
      return serde_json::json!({
        "has_repo": false,
        "command_in_flight": self.repo_command_in_flight.is_some(),
      });
    };

    let palette_commands = self
      .palette_commands(self.palette_repositories().len(), cx)
      .into_iter()
      .map(|command| command.id.as_str())
      .collect::<Vec<_>>();
    let status_entries = git::list_repo_status(&repo_root).unwrap_or_default();
    let branch_status = git::current_branch_status(&repo_root).ok();
    let branches = git::list_branches(&repo_root).unwrap_or_default();
    let upstream_branch = git::current_branch_upstream(&repo_root).ok().flatten();
    let default_branch = git::default_remote_branch(&repo_root).ok().flatten();
    let stashes = git::list_stashes(&repo_root).unwrap_or_default();
    let merge_in_progress = git::is_merge_in_progress(&repo_root).unwrap_or(false);
    let rebase_in_progress = git::is_rebase_in_progress(&repo_root).unwrap_or(false);

    serde_json::json!({
      "has_repo": true,
      "repo_root": repo_root.display().to_string(),
      "selected_file": self.selected_file.as_ref().map(|path| path.display().to_string()),
      "center": format!("{:?}", self.center),
      "command_in_flight": self.repo_command_in_flight.is_some(),
      "merge_in_progress": merge_in_progress,
      "rebase_in_progress": rebase_in_progress,
      "head_status": {
        "has_head_commit": head_status.has_head_commit,
        "can_undo_last_commit": head_status.can_undo_last_commit,
      },
      "commit_message": commit_message,
      "palette_commands": palette_commands,
      "branch_status": branch_status.map(|status| serde_json::json!({
        "name": status.name,
        "ahead": status.ahead,
        "behind": status.behind,
        "has_upstream": status.has_upstream,
      })),
      "upstream_branch": upstream_branch.map(driver_branch_json),
      "default_branch": default_branch.map(driver_branch_json),
      "branches": branches.into_iter().map(driver_branch_json).collect::<Vec<_>>(),
      "status_entries": status_entries.into_iter().map(driver_status_json).collect::<Vec<_>>(),
      "stashes": stashes.into_iter().map(|stash| serde_json::json!({
        "index": stash.index,
        "name": stash.name,
        "oid": stash.oid,
      })).collect::<Vec<_>>(),
    })
  }

  #[cfg(any(test, feature = "test-support"))]
  #[doc(hidden)]
  pub fn run_git_action_for_driver(
    &mut self,
    action: crate::DriverGitAction,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    use crate::DriverGitAction;

    match action {
      DriverGitAction::Commit { message } => {
        self.dock_panel.update(cx, |panel, cx| {
          panel.set_commit_message(&message, window, cx)
        });
        self.handle_command_palette_action(CommandPaletteAction::Commit, window, cx)
      }
      DriverGitAction::Amend { message } => {
        if let Some(message) = message {
          self.dock_panel.update(cx, |panel, cx| {
            panel.set_commit_message(&message, window, cx)
          });
        } else {
          self
            .dock_panel
            .update(cx, |panel, cx| panel.set_commit_message("", window, cx));
        }
        self.handle_command_palette_action(CommandPaletteAction::Amend, window, cx)
      }
      DriverGitAction::StageAll => {
        self.handle_command_palette_action(CommandPaletteAction::StageAll, window, cx)
      }
      DriverGitAction::UnstageAll => {
        self.handle_command_palette_action(CommandPaletteAction::UnstageAll, window, cx)
      }
      DriverGitAction::Push => {
        self.handle_command_palette_action(CommandPaletteAction::Push, window, cx)
      }
      DriverGitAction::ForcePush => {
        self.handle_command_palette_action(CommandPaletteAction::ForcePush, window, cx)
      }
      DriverGitAction::Pull => {
        self.handle_command_palette_action(CommandPaletteAction::Pull, window, cx)
      }
      DriverGitAction::Fetch => {
        self.handle_command_palette_action(CommandPaletteAction::Fetch, window, cx)
      }
      DriverGitAction::UndoLastCommit => {
        self.handle_command_palette_action(CommandPaletteAction::UndoLastCommit, window, cx)
      }
      DriverGitAction::CheckoutDetached { target } => self.handle_command_palette_action(
        CommandPaletteAction::CheckoutDetached { target },
        window,
        cx,
      ),
      DriverGitAction::SwitchBranch { branch } => self.handle_command_palette_action(
        CommandPaletteAction::SwitchBranch(driver_palette_branch(branch)),
        window,
        cx,
      ),
      DriverGitAction::CreateBranch { name } => {
        self.handle_command_palette_action(CommandPaletteAction::CreateBranch { name }, window, cx)
      }
      DriverGitAction::CreateBranchFrom { name, base } => self.handle_command_palette_action(
        CommandPaletteAction::CreateBranchFrom {
          name,
          base: driver_palette_branch(base),
        },
        window,
        cx,
      ),
      DriverGitAction::DeleteBranch { branch } => self.handle_command_palette_action(
        CommandPaletteAction::DeleteBranch(driver_palette_branch(branch)),
        window,
        cx,
      ),
      DriverGitAction::MergeBranch { branch } => self.handle_command_palette_action(
        CommandPaletteAction::MergeBranch {
          name: driver_palette_branch(branch),
        },
        window,
        cx,
      ),
      DriverGitAction::AbortMerge => {
        self.handle_command_palette_action(CommandPaletteAction::AbortMerge, window, cx)
      }
      DriverGitAction::RebaseBranch { branch } => self.handle_command_palette_action(
        CommandPaletteAction::RebaseBranch {
          name: driver_palette_branch(branch),
        },
        window,
        cx,
      ),
      DriverGitAction::ContinueRebase => {
        self.handle_command_palette_action(CommandPaletteAction::ContinueRebase, window, cx)
      }
      DriverGitAction::SkipRebase => {
        self.handle_command_palette_action(CommandPaletteAction::SkipRebase, window, cx)
      }
      DriverGitAction::AbortRebase => {
        self.handle_command_palette_action(CommandPaletteAction::AbortRebase, window, cx)
      }
      DriverGitAction::Stash {
        include_untracked,
        message,
      } => self.handle_command_palette_action(
        CommandPaletteAction::Stash {
          include_untracked,
          message,
        },
        window,
        cx,
      ),
      DriverGitAction::ApplyStash { index, name } => self.handle_command_palette_action(
        CommandPaletteAction::ApplyStash(driver_palette_stash(index, name)),
        window,
        cx,
      ),
      DriverGitAction::PopStash { index, name } => self.handle_command_palette_action(
        CommandPaletteAction::PopStash(driver_palette_stash(index, name)),
        window,
        cx,
      ),
      DriverGitAction::DropStash { index, name } => self.handle_command_palette_action(
        CommandPaletteAction::DropStash(driver_palette_stash(index, name)),
        window,
        cx,
      ),
      DriverGitAction::CherryPick { commit_hashes } => self.handle_command_palette_action(
        CommandPaletteAction::CherryPick { commit_hashes },
        window,
        cx,
      ),
      DriverGitAction::RestoreAll => {
        self.handle_command_palette_action(CommandPaletteAction::RestoreAll, window, cx)
      }
    }
  }
}

#[cfg(any(test, feature = "test-support"))]
fn driver_palette_branch(branch: crate::DriverBranchRef) -> ui::CommandPaletteBranch {
  ui::CommandPaletteBranch {
    name: branch.name.into(),
    kind: match branch.kind {
      crate::DriverBranchKind::Local => ui::CommandPaletteBranchKind::Local,
      crate::DriverBranchKind::Remote => ui::CommandPaletteBranchKind::Remote,
    },
  }
}

#[cfg(any(test, feature = "test-support"))]
fn driver_palette_stash(index: usize, name: String) -> ui::CommandPaletteStash {
  ui::CommandPaletteStash {
    index,
    name: name.into(),
    oid: "".into(),
  }
}

#[cfg(any(test, feature = "test-support"))]
fn driver_branch_json(branch: git::BranchRef) -> serde_json::Value {
  serde_json::json!({
    "name": branch.name,
    "kind": format!("{:?}", branch.kind),
  })
}

#[cfg(any(test, feature = "test-support"))]
fn driver_status_json(entry: git::RepoStatusEntry) -> serde_json::Value {
  serde_json::json!({
    "path": entry.path.display().to_string(),
    "old_path": entry.old_path.map(|path| path.display().to_string()),
    "status": format!("{:?}", entry.status),
    "stage": format!("{:?}", entry.stage),
  })
}
