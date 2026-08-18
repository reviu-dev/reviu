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
      if !self.branches.is_empty() {
        commands.push(CommandPaletteCommand::switch_branch());
        if !self.delete_branch_targets().is_empty() {
          commands.push(CommandPaletteCommand::delete_branch());
        }
      }
      if state.allows(PaletteCommand::MergeBranch) && !self.branches.is_empty() {
        commands.push(CommandPaletteCommand::merge_branch());
      }
      if state.allows(PaletteCommand::RebaseBranch) && !self.rebase_branch_targets().is_empty() {
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
      if !self.stashes.is_empty() {
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

  pub(super) fn current_branch_name(&self) -> Option<&str> {
    self
      .branch_status
      .as_ref()
      .map(|status| status.name.as_str())
  }

  pub(super) fn rebase_branch_targets(&self) -> Vec<ui::CommandPaletteBranch> {
    rebase_branch_candidates(
      &self.branches,
      self.current_branch_name(),
      self.upstream_branch.as_ref(),
      self.default_branch.as_ref(),
    )
  }

  pub(super) fn delete_branch_targets(&self) -> Vec<ui::CommandPaletteBranch> {
    delete_branch_candidates(&self.branches, self.current_branch_name())
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

    let branches = self.branches.iter().map(palette_branch).collect::<Vec<_>>();
    let mut config = CommandPaletteConfig::new(branches, commands, handler)
      .with_repositories(repositories)
      .with_rebase_branches(self.rebase_branch_targets())
      .with_delete_branches(self.delete_branch_targets())
      .with_stashes(palette_stashes(&self.stashes));
    if let Some(message) = self.default_stash_message.clone() {
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
        self
          .dock_panel
          .read(cx)
          .changes_list()
          .update(cx, |list, cx| {
            list.confirm_restore_all(window, cx);
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
