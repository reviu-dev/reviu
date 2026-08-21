//! The shell's command palette: what it offers and what each action does.

use super::*;

impl SessionPage {
  pub(super) fn palette_repositories(&self) -> Vec<CommandPaletteRepository> {
    let mut repositories = ConfigStore::load_recent_repositories()
      .into_iter()
      .map(|repo| CommandPaletteRepository {
        path: repo.path.to_string_lossy().replace(['\n', '\r'], "").into(),
      })
      .collect::<Vec<_>>();

    if let Some(selected_repo) = self.selected_repo.as_ref() {
      let selected = selected_repo.to_string_lossy().replace(['\n', '\r'], "");
      if !repositories
        .iter()
        .any(|repo| repo.path.as_ref() == selected)
      {
        repositories.insert(
          0,
          CommandPaletteRepository {
            path: selected.into(),
          },
        );
      }
    }

    repositories
  }

  pub(super) fn palette_commands(
    &self,
    repositories_len: usize,
    cx: &App,
  ) -> Vec<CommandPaletteCommand> {
    let mut commands = Vec::new();
    if repositories_len > 1 {
      commands.push(CommandPaletteCommand::switch_repository());
    }
    commands.push(CommandPaletteCommand::open_repository());
    if repositories_len > 0 {
      commands.push(CommandPaletteCommand::forget_repository());
    }

    if self.selected_repo.is_some() {
      let commit_message = self.dock_panel.read(cx).commit_message(cx);
      let state = self.repo_state(&commit_message, cx);
      if state.allows(PaletteCommand::Commit) {
        commands.push(CommandPaletteCommand::commit());
      }
      if self.can_accept_all_conflicts(cx) {
        commands.push(CommandPaletteCommand::accept_all_current_conflicts());
        commands.push(CommandPaletteCommand::accept_all_incoming_conflicts());
      }
      if state.allows(PaletteCommand::ContinueRebase) {
        commands.push(CommandPaletteCommand::continue_rebase());
      }
      if state.allows(PaletteCommand::SkipRebase) {
        commands.push(CommandPaletteCommand::skip_rebase());
      }
      if state.rebase_in_progress {
        commands.push(CommandPaletteCommand::abort_rebase());
      }
      if state.merge_in_progress {
        commands.push(CommandPaletteCommand::abort_merge());
      }
      if state.allows(PaletteCommand::StageAll) {
        commands.push(CommandPaletteCommand::stage_all());
      }
      if state.allows(PaletteCommand::UnstageAll) {
        commands.push(CommandPaletteCommand::unstage_all());
      }
      if state.allows(PaletteCommand::RestoreAll) {
        commands.push(CommandPaletteCommand::restore_all());
      }
      if let Some(command) = self.dock_panel.read(cx).branch_pull_request_command() {
        commands.push(command);
      }
      if state.allows(PaletteCommand::Push) {
        commands.push(CommandPaletteCommand::push("Push"));
      }
      if state.allows(PaletteCommand::ForcePush) {
        commands.push(CommandPaletteCommand::force_push());
      }
      if state.allows(PaletteCommand::Amend) {
        commands.push(CommandPaletteCommand::amend());
      }
      if state.allows(PaletteCommand::UndoLastCommit) {
        commands.push(CommandPaletteCommand::undo_last_commit());
      }
      if state.allows(PaletteCommand::CheckoutDetached) {
        commands.push(CommandPaletteCommand::checkout_detached());
      }
      if state.allows(PaletteCommand::StageSelectedFile) {
        commands.push(CommandPaletteCommand::stage_selected_file());
      }
      if state.allows(PaletteCommand::UnstageSelectedFile) {
        commands.push(CommandPaletteCommand::unstage_selected_file());
      }
      if state.allows(PaletteCommand::InteractiveRebase) {
        commands.push(CommandPaletteCommand::interactive_rebase());
      }
      let has_branches = !self.repo_snapshot.read(cx).branches().is_empty();
      if has_branches {
        commands.push(CommandPaletteCommand::switch_branch());
        if !self.delete_branch_targets(cx).is_empty() {
          commands.push(CommandPaletteCommand::delete_branch());
        }
      }
      if state.allows(PaletteCommand::MergeBranch) && has_branches {
        commands.push(CommandPaletteCommand::merge_branch());
      }
      if state.allows(PaletteCommand::RebaseBranch) && !self.rebase_branch_targets(cx).is_empty() {
        commands.push(CommandPaletteCommand::rebase_branch());
      }
      if state.allows(PaletteCommand::CherryPick) {
        commands.push(CommandPaletteCommand::cherry_pick());
      }
      if state.allows(PaletteCommand::Stash) {
        commands.push(CommandPaletteCommand::stash());
      }
      if state.allows(PaletteCommand::StashWithUntracked) {
        commands.push(CommandPaletteCommand::stash_with_untracked());
      }
      if !self.repo_snapshot.read(cx).stashes().is_empty() {
        commands.push(CommandPaletteCommand::apply_stash());
        commands.push(CommandPaletteCommand::pop_stash());
        commands.push(CommandPaletteCommand::drop_stash());
      }
      if state.allows(PaletteCommand::Pull) {
        commands.push(CommandPaletteCommand::pull());
      }
      commands.push(CommandPaletteCommand::fetch());
    }

    commands.extend(CommandPaletteCommand::default_global_commands(
      CommandPalettePage::Session,
      AuthStateStore::has_github_access(cx),
    ));
    commands
  }

  pub(super) fn rebase_branch_targets(&self, cx: &App) -> Vec<ui::CommandPaletteBranch> {
    let snapshot = self.repo_snapshot.read(cx);
    rebase_branch_candidates(
      snapshot.branches(),
      snapshot.current_branch_name(),
      snapshot.upstream_branch(),
      snapshot.default_branch(),
    )
  }

  pub(super) fn delete_branch_targets(&self, cx: &App) -> Vec<ui::CommandPaletteBranch> {
    let snapshot = self.repo_snapshot.read(cx);
    delete_branch_candidates(snapshot.branches(), snapshot.current_branch_name())
  }

  pub(super) fn open_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self.open_command_palette_with_screen(None, window, cx);
  }

  pub(super) fn open_command_palette_with_screen(
    &mut self,
    initial_screen: Option<CommandPaletteInitialScreen>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let repositories = self.palette_repositories();
    let commands = self.palette_commands(repositories.len(), cx);

    let view = cx.entity();
    let handler: CommandPaletteHandler = Arc::new(move |action, window, cx| {
      view.update(cx, |view, cx| {
        view.handle_command_palette_action(action, window, cx)
      })
    });

    let snapshot = self.repo_snapshot.read(cx);
    let branches = snapshot.branches().iter().map(palette_branch).collect();
    let stashes = palette_stashes(snapshot.stashes());
    let default_stash_message = snapshot.default_stash_message();
    let mut config = CommandPaletteConfig::new(branches, commands, handler)
      .with_repositories(repositories)
      .with_rebase_branches(self.rebase_branch_targets(cx))
      .with_delete_branches(self.delete_branch_targets(cx))
      .with_stashes(stashes);
    if let Some(message) = default_stash_message {
      config = config.with_default_stash_message(message);
    }
    if let Some(initial_screen) = initial_screen {
      config = config.with_initial_screen(initial_screen);
    }
    let palette = cx.new(|cx| CommandPalette::new(window, cx, config));
    ui::open_palette_dialog(palette, window, cx);
  }

  pub(super) fn handle_command_palette_action(
    &mut self,
    action: CommandPaletteAction,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    match action {
      CommandPaletteAction::Commit => {
        if self.selected_repo.is_none() {
          return Err("No repository selected.".into());
        }
        self.dock_panel.update(cx, |panel, cx| panel.commit(cx));
        Ok(())
      }
      CommandPaletteAction::AcceptAllCurrentConflicts => {
        self.resolve_all_conflicts(ConflictResolution::Current, cx);
        Ok(())
      }
      CommandPaletteAction::AcceptAllIncomingConflicts => {
        self.resolve_all_conflicts(ConflictResolution::Incoming, cx);
        Ok(())
      }
      CommandPaletteAction::ContinueRebase => {
        self.run_repo_command(RepoCommand::ContinueRebase, window, cx)
      }
      CommandPaletteAction::SkipRebase => {
        self.run_repo_command(RepoCommand::SkipRebase, window, cx)
      }
      CommandPaletteAction::AbortRebase => {
        self.run_repo_command(RepoCommand::AbortRebase, window, cx)
      }
      CommandPaletteAction::AbortMerge => {
        self.run_repo_command(RepoCommand::AbortMerge, window, cx)
      }
      CommandPaletteAction::StageAll => self.stage_all_with_confirmation(window, cx),
      CommandPaletteAction::UnstageAll => {
        self.run_repo_command(RepoCommand::UnstageAll, window, cx)
      }
      CommandPaletteAction::CreatePullRequest => {
        self
          .dock_panel
          .update(cx, |panel, cx| panel.create_branch_pull_request(window, cx));
        Ok(())
      }
      CommandPaletteAction::OpenPullRequest => {
        self
          .dock_panel
          .update(cx, |panel, cx| panel.open_branch_pull_request(cx));
        Ok(())
      }
      CommandPaletteAction::RestoreAll => {
        let changes_list = self.dock_panel.read(cx).changes_list();
        cx.defer_in(window, move |_, window, cx| {
          changes_list.update(cx, |list, cx| {
            list.confirm_restore_all(window, cx);
          });
        });
        Ok(())
      }
      CommandPaletteAction::Push => self.run_repo_command(RepoCommand::Push, window, cx),
      CommandPaletteAction::ForcePush => self.run_repo_command(RepoCommand::ForcePush, window, cx),
      CommandPaletteAction::Amend => self.amend_last_commit(window, cx),
      CommandPaletteAction::UndoLastCommit => {
        self.run_repo_command(RepoCommand::UndoLastCommit, window, cx)
      }
      CommandPaletteAction::CheckoutDetached { target } => {
        self.run_repo_command(RepoCommand::CheckoutDetached { target }, window, cx)
      }
      CommandPaletteAction::SwitchBranch(branch) => self.run_branch_command(
        RepoCommand::SwitchBranch(branch_ref_from_palette(&branch)),
        window,
        cx,
      ),
      CommandPaletteAction::CreateBranch { name } => {
        self.run_branch_command(RepoCommand::CreateBranch { name }, window, cx)
      }
      CommandPaletteAction::CreateBranchFrom { name, base } => self.run_branch_command(
        RepoCommand::CreateBranchFrom {
          name,
          base: branch_ref_from_palette(&base),
        },
        window,
        cx,
      ),
      CommandPaletteAction::DeleteBranch(branch) => self.run_repo_command(
        RepoCommand::DeleteBranch(branch_ref_from_palette(&branch)),
        window,
        cx,
      ),
      CommandPaletteAction::MergeBranch { name } => self.run_repo_command(
        RepoCommand::MergeBranch(branch_ref_from_palette(&name)),
        window,
        cx,
      ),
      CommandPaletteAction::RebaseBranch { name } => self.run_repo_command(
        RepoCommand::RebaseBranch(branch_ref_from_palette(&name)),
        window,
        cx,
      ),
      CommandPaletteAction::CherryPick { commit_hashes } => {
        self.run_repo_command(RepoCommand::CherryPick { commit_hashes }, window, cx)
      }
      CommandPaletteAction::Stash {
        include_untracked,
        message,
      } => self.run_repo_command(
        RepoCommand::Stash {
          include_untracked,
          message,
        },
        window,
        cx,
      ),
      CommandPaletteAction::ApplyStash(stash) => self.run_repo_command(
        RepoCommand::ApplyStash {
          index: stash.index,
          name: stash.name.to_string(),
        },
        window,
        cx,
      ),
      CommandPaletteAction::PopStash(stash) => self.run_repo_command(
        RepoCommand::PopStash {
          index: stash.index,
          name: stash.name.to_string(),
        },
        window,
        cx,
      ),
      CommandPaletteAction::DropStash(stash) => self.run_repo_command(
        RepoCommand::DropStash {
          index: stash.index,
          name: stash.name.to_string(),
        },
        window,
        cx,
      ),
      CommandPaletteAction::StageSelectedFile => self.stage_selected_file(window, cx),
      CommandPaletteAction::UnstageSelectedFile => self.unstage_selected_file(window, cx),
      CommandPaletteAction::InteractiveRebaseBranch { ref name } => self.start_interactive_rebase(
        InteractiveRebaseTarget::Branch(branch_ref_from_palette(name)),
        window,
        cx,
      ),
      CommandPaletteAction::InteractiveRebaseEditBranch { ref name } => self
        .start_interactive_rebase(
          InteractiveRebaseTarget::BranchInPlace(branch_ref_from_palette(name)),
          window,
          cx,
        ),
      CommandPaletteAction::InteractiveRebaseHeadCount { count } => {
        self.start_interactive_rebase(InteractiveRebaseTarget::HeadCount(count), window, cx)
      }
      CommandPaletteAction::Pull => self.run_repo_command(RepoCommand::Pull, window, cx),
      CommandPaletteAction::Fetch => self.run_repo_command(RepoCommand::Fetch, window, cx),
      CommandPaletteAction::OpenRepository => {
        self.start_open_repository(window, cx);
        Ok(())
      }
      CommandPaletteAction::SwitchRepository(repository) => {
        let repo_root = PathBuf::from(repository.path.as_ref());
        if !repo_root.is_dir() {
          return Err(format!("Repository not found: {}", repo_root.display()).into());
        }
        self.set_selected_repo(repo_root, window, cx)
      }
      CommandPaletteAction::ForgetRepository(repository) => {
        self.forget_repository(PathBuf::from(repository.path.as_ref()), window, cx)
      }
      other => crate::palette_actions::handle_global_command_palette_action(other, window, cx),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::super::test_support::*;
  use super::super::*;
  use crate::test_support::{TempRepo, commit_text_file};
  use gpui::TestAppContext;
  use std::path::Path;
  use ui::CommandPaletteCommandId;

  #[gpui::test]
  async fn branches_and_stashes_reach_the_palette(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-branches-stashes");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");
    let base = git::current_branch_status(&repo.path)
      .expect("branch status")
      .name;
    git::create_branch(&repo.path, "feature").expect("create branch");
    std::fs::write(repo.path.join("a.txt"), "v2\n").expect("update file");
    git::create_stash(&repo.path, false, Some("wip")).expect("stash");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let refresh = page.update(cx, |page, cx| {
      page.refresh_branch(cx);
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    await_branch_refresh(&page, cx).await;

    page.read_with(cx, |page, cx| {
      assert_eq!(
        page.repo_snapshot.read(cx).stashes().len(),
        1,
        "the stash was loaded"
      );
      let ids = page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>();
      assert!(ids.contains(&CommandPaletteCommandId::SwitchBranch));
      assert!(ids.contains(&CommandPaletteCommandId::DeleteBranch));
      assert!(ids.contains(&CommandPaletteCommandId::MergeBranch));
      assert!(ids.contains(&CommandPaletteCommandId::CherryPick));
      assert!(ids.contains(&CommandPaletteCommandId::ApplyStash));
      assert!(ids.contains(&CommandPaletteCommandId::PopStash));
      assert!(ids.contains(&CommandPaletteCommandId::DropStash));
      assert!(
        !ids.contains(&CommandPaletteCommandId::Stash),
        "the worktree is clean once stashed"
      );

      // The lists behind the screens: never the branch we are on.
      let targets = page.delete_branch_targets(cx);
      assert!(
        targets
          .iter()
          .any(|branch| branch.name.as_ref() == "feature")
      );
      assert!(!targets.iter().any(|branch| branch.name.as_ref() == base));
    });

    // Applying the stash brings the change back.
    page.update_in(cx, |page, window, cx| {
      page
        .handle_command_palette_action(
          CommandPaletteAction::PopStash(ui::CommandPaletteStash {
            index: 0,
            name: "wip".into(),
            oid: "".into(),
          }),
          window,
          cx,
        )
        .expect("pop the stash")
    });
    let command = page.update(cx, |page, _| {
      page._repo_command_task.take().expect("command task")
    });
    command.await;
    cx.run_until_parked();

    assert_eq!(
      std::fs::read_to_string(repo.path.join("a.txt")).expect("read file"),
      "v2\n"
    );
  }

  #[gpui::test]
  async fn palette_repositories_put_the_open_repository_first(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-palette-repos");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let other = TempRepo::init("session-page-palette-repos-other");
    commit_text_file(&other.path, Path::new("README.md"), "other\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    ConfigStore::persist_recent_repository(&other.path);

    page.read_with(cx, |page, _| {
      let repositories = page.palette_repositories();
      // The open repository is not in the recents yet, it still leads the list.
      assert_eq!(
        repositories.first().map(|repo| repo.path.to_string()),
        Some(repo.path.to_string_lossy().to_string())
      );
      assert_eq!(repositories.len(), 2);
    });

    // Once it is a recent too, it must not be listed twice. Order is left to the
    // recents, whose timestamps have a one-second granularity.
    ConfigStore::persist_recent_repository(&repo.path);
    page.read_with(cx, |page, _| {
      let repositories = page.palette_repositories();
      assert_eq!(repositories.len(), 2);
      assert_eq!(
        repositories
          .iter()
          .filter(|entry| entry.path.as_ref() == repo.path.to_string_lossy())
          .count(),
        1
      );
    });
  }

  #[gpui::test]
  async fn the_palette_reaches_the_pull_request_of_the_branch(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-pr-palette");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    // No GitHub access: the palette says nothing about pull requests.
    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.set_branch_pull_request_state(crate::dock_panel::BranchPrState::NoAccess, cx);
      });
    });
    page.read_with(cx, |page, cx| {
      let ids = page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>();
      assert!(!ids.contains(&CommandPaletteCommandId::CreatePullRequest));
      assert!(!ids.contains(&CommandPaletteCommandId::OpenPullRequest));
    });

    // A published branch with no pull request yet.
    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.set_branch_pull_request_state(
          crate::dock_panel::BranchPrState::Missing(GithubBranchContext {
            owner: "acme".to_string(),
            repo: "widget".to_string(),
            branch: "feature".to_string(),
          }),
          cx,
        );
        panel.set_branch_status(
          Some(git::BranchStatus {
            name: "feature".to_string(),
            ahead: 0,
            behind: 0,
            has_upstream: true,
          }),
          cx,
        );
      });
    });

    page.read_with(cx, |page, cx| {
      let ids = page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>();
      assert!(ids.contains(&CommandPaletteCommandId::CreatePullRequest));
    });

    page.update_in(cx, |page, window, cx| {
      page
        .handle_command_palette_action(CommandPaletteAction::CreatePullRequest, window, cx)
        .expect("create pull request is allowed");
    });
    cx.run_until_parked();

    assert!(
      cx.update(|window, cx| window.has_active_dialog(cx)),
      "the palette opens the same form as the tab"
    );

    // An existing pull request: the palette opens it instead of the form.
    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.set_branch_pull_request_state(
          crate::dock_panel::BranchPrState::Found(
            GithubBranchContext {
              owner: "acme".to_string(),
              repo: "widget".to_string(),
              branch: "feature".to_string(),
            },
            Box::new(
              serde_json::from_value(serde_json::json!({
                "number": 42,
                "title": "Add widgets",
                "state": "open",
                "draft": false,
                "repository": { "owner": "acme", "repo": "widget" }
              }))
              .expect("build pull request"),
            ),
          ),
          cx,
        );
      });
    });

    page.read_with(cx, |page, cx| {
      let ids = page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>();
      assert!(ids.contains(&CommandPaletteCommandId::OpenPullRequest));
      assert!(!ids.contains(&CommandPaletteCommandId::CreatePullRequest));
    });

    // The pull request page is not mounted here: opening it is a no-op, not a crash.
    page.update_in(cx, |page, window, cx| {
      page
        .handle_command_palette_action(CommandPaletteAction::OpenPullRequest, window, cx)
        .expect("open pull request is allowed");
    });
    cx.run_until_parked();
  }

  #[gpui::test]
  async fn restore_all_confirmation_survives_the_command_palette_closing(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-restore-all-palette-dialog");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");
    std::fs::write(repo.path.join("a.txt"), "v2\n").expect("modify file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let refresh = page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| page.open_command_palette(window, cx));
    cx.run_until_parked();
    cx.simulate_input("restore all");
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();

    assert!(cx.update(|window, cx| window.has_active_dialog(cx)));
    assert_eq!(
      std::fs::read_to_string(repo.path.join("a.txt")).expect("read file"),
      "v2\n"
    );
    cx.update(|window, cx| window.close_dialog(cx));
  }

  #[gpui::test]
  async fn the_palette_restores_every_change_after_confirmation(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-restore-all");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let refresh = page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    cx.run_until_parked();

    // Nothing changed yet: nothing to restore.
    page.read_with(cx, |page, cx| {
      let ids = page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>();
      assert!(!ids.contains(&CommandPaletteCommandId::RestoreAll));
    });

    std::fs::write(repo.path.join("a.txt"), "v2\n").expect("modify file");
    let refresh = page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    cx.run_until_parked();

    page.read_with(cx, |page, cx| {
      let ids = page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>();
      assert!(ids.contains(&CommandPaletteCommandId::RestoreAll));
    });

    // Destructive: the command asks first and touches nothing on its own.
    page.update_in(cx, |page, window, cx| {
      page
        .handle_command_palette_action(CommandPaletteAction::RestoreAll, window, cx)
        .expect("restore all is allowed");
    });
    cx.run_until_parked();

    assert!(cx.update(|window, cx| window.has_active_dialog(cx)));
    assert_eq!(
      std::fs::read_to_string(repo.path.join("a.txt")).expect("read file"),
      "v2\n",
      "the file is only discarded once the dialog is confirmed"
    );

    // What the dialog runs on confirmation.
    let restore = page.update_in(cx, |page, window, cx| {
      page
        .dock_panel
        .read(cx)
        .changes_list()
        .update(cx, |list, cx| {
          list.restore_all(window, cx);
          list._action_task.take().expect("restore all task")
        })
    });
    restore.await;
    cx.run_until_parked();
    let refresh = page.update(cx, |page, cx| {
      page
        .dock_panel
        .update(cx, |panel, _| panel._refresh_task.take())
    });
    if let Some(task) = refresh {
      task.await;
    }
    cx.run_until_parked();

    assert_eq!(
      std::fs::read_to_string(repo.path.join("a.txt")).expect("read file"),
      "v1\n"
    );
    page.read_with(cx, |page, cx| {
      assert!(
        page.dock_panel.read(cx).status_entries().is_empty(),
        "the changes list follows a discard without an explicit refresh"
      );
    });
  }

  #[gpui::test]
  async fn the_palette_only_offers_what_the_repository_allows(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-palette-rules");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| panel.refresh(cx));
      page.refresh_branch(cx);
    });
    await_branch_refresh(&page, cx).await;

    let ids = |page: &SessionPage, cx: &App| {
      page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>()
    };

    // Clean worktree, no upstream: nothing to commit, stage or sync.
    page.read_with(cx, |page, cx| {
      let ids = ids(page, cx);
      assert!(!ids.contains(&CommandPaletteCommandId::Commit));
      assert!(!ids.contains(&CommandPaletteCommandId::StageAll));
      assert!(!ids.contains(&CommandPaletteCommandId::UnstageAll));
      assert!(!ids.contains(&CommandPaletteCommandId::Pull));
      assert!(
        ids.contains(&CommandPaletteCommandId::Fetch),
        "fetching is always available"
      );
    });

    // A change and a message: committing and staging show up.
    std::fs::write(repo.path.join("a.txt"), "v2\n").expect("update file");
    let refresh = page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    cx.run_until_parked();
    page.update_in(cx, |page, window, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.set_commit_message("a message", window, cx)
      });
    });

    page.read_with(cx, |page, cx| {
      let ids = ids(page, cx);
      assert!(ids.contains(&CommandPaletteCommandId::Commit));
      assert!(ids.contains(&CommandPaletteCommandId::StageAll));
    });
  }

  #[gpui::test]
  async fn the_palette_offers_syncing_once_the_branch_tracks_a_remote(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-palette-sync");
    let remote = crate::test_support::TempBareRepo::init("session-page-palette-sync-remote");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");
    git2::Repository::open(&repo.path)
      .expect("open repo")
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add origin");
    let branch = git::current_branch_status(&repo.path)
      .expect("branch status")
      .name;
    crate::test_support::push_branch_to_remote(&repo.path, &branch, "origin");
    crate::test_support::set_upstream(&repo.path, &branch, &format!("origin/{branch}"));
    commit_text_file(
      &repo.path,
      Path::new("a.txt"),
      "v2\n",
      "ahead of the remote",
    );

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    page.update(cx, |page, cx| page.refresh_branch(cx));
    await_branch_refresh(&page, cx).await;

    page.read_with(cx, |page, cx| {
      let ids = page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>();
      assert!(
        ids.contains(&CommandPaletteCommandId::Push),
        "one commit ahead of the upstream is something to push"
      );
      assert!(
        ids.contains(&CommandPaletteCommandId::Pull),
        "a tracked branch can be pulled"
      );
      assert!(
        !ids.contains(&CommandPaletteCommandId::ForcePush),
        "nothing forces a push on a branch that only moved forward"
      );
    });

    // Rewriting a commit the remote already has diverges the branch.
    git::push(&repo.path, false).expect("push the commit first");
    git::undo_last_commit(&repo.path).expect("undo the last commit");
    commit_text_file(
      &repo.path,
      Path::new("a.txt"),
      "v2 rewritten\n",
      "rewritten",
    );
    page.update(cx, |page, cx| page.refresh_branch(cx));
    await_branch_refresh(&page, cx).await;

    page.read_with(cx, |page, cx| {
      let ids = page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>();
      assert!(ids.contains(&CommandPaletteCommandId::ForcePush));
      assert!(!ids.contains(&CommandPaletteCommandId::Push));
    });
  }
}
