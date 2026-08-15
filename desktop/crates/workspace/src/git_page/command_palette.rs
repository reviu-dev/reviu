//! Command palette: contents, dispatch and the file search palette.

use super::*;

impl GitPage {
  pub(super) fn branch_pull_request_palette_command(
    &self,
    cx: &App,
  ) -> Option<CommandPaletteCommand> {
    match self.current_branch_pr_button_state(cx) {
      GitBranchPullRequestButtonState::Create => Some(CommandPaletteCommand::create_pull_request()),
      GitBranchPullRequestButtonState::OpenExisting { number, .. } => {
        Some(CommandPaletteCommand::open_pull_request(number))
      }
      GitBranchPullRequestButtonState::Checking => Some(
        CommandPaletteCommand::create_pull_request().disabled("Checking for an open pull request"),
      ),
      GitBranchPullRequestButtonState::Hidden
      | GitBranchPullRequestButtonState::LockedPro
      | GitBranchPullRequestButtonState::PublishAndCreate => None,
    }
  }

  pub(super) fn command_palette_error_notification_title(
    action: &CommandPaletteAction,
  ) -> Option<&'static str> {
    match action {
      CommandPaletteAction::CheckoutDetached { .. } => Some("Checkout failed"),
      CommandPaletteAction::SwitchBranch(_) => Some("Switch branch failed"),
      CommandPaletteAction::CreateBranch { .. } => Some("Create branch failed"),
      CommandPaletteAction::CreateBranchFrom { .. } => Some("Create branch failed"),
      CommandPaletteAction::DeleteBranch(_) => Some("Delete branch failed"),
      CommandPaletteAction::MergeBranch { .. } => Some("Merge failed"),
      CommandPaletteAction::AbortMerge => Some("Abort merge failed"),
      CommandPaletteAction::RebaseBranch { .. } => Some("Rebase failed"),
      CommandPaletteAction::AbortRebase => Some("Abort rebase failed"),
      CommandPaletteAction::Stash { .. } => Some("Stash failed"),
      CommandPaletteAction::ApplyStash(_) => Some("Apply stash failed"),
      CommandPaletteAction::DropStash(_) => Some("Drop stash failed"),
      CommandPaletteAction::PopStash(_) => Some("Pop stash failed"),
      CommandPaletteAction::CherryPick { .. } => Some("Cherry-pick failed"),
      _ => None,
    }
  }

  pub(super) fn handle_command_palette_operation_error(
    &mut self,
    action: &CommandPaletteAction,
    err: anyhow::Error,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let message: SharedString = err.to_string().into();
    let title = Self::command_palette_error_notification_title(action).unwrap_or("Action failed");
    self.push_git_action_error_notification_in_window(title, message, window, cx);
  }

  pub(super) fn show_command_palette_action(
    &mut self,
    _: &ShowCommandPalette,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.open_command_palette(window, cx, None);
  }

  pub(super) fn show_file_search_action(
    &mut self,
    _: &ShowFileSearch,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.open_file_search_palette(window, cx);
  }

  pub(super) fn open_command_palette(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
    initial_screen: Option<CommandPaletteInitialScreen>,
  ) {
    crate::analytics::track(cx, "command_palette_opened");
    let mut palette_repositories = ConfigStore::load_recent_repositories()
      .into_iter()
      .map(|repo| CommandPaletteRepository {
        path: repo.path.to_string_lossy().replace(['\n', '\r'], "").into(),
      })
      .collect::<Vec<_>>();

    if let Some(selected_repo) = self.selected_repo.as_ref() {
      let selected_repo_path = selected_repo.to_string_lossy().replace(['\n', '\r'], "");
      if !palette_repositories
        .iter()
        .any(|repo| repo.path.as_ref() == selected_repo_path)
      {
        palette_repositories.insert(
          0,
          CommandPaletteRepository {
            path: selected_repo_path.into(),
          },
        );
      }
    }

    let GitCommandPaletteContents {
      commands,
      branches: palette_branches,
      rebase_branches: palette_rebase_branches,
      delete_branches: palette_delete_branches,
      stashes: palette_stashes,
      default_stash_message: palette_default_stash_message,
    } = self.build_command_palette_contents(palette_repositories.len(), cx);

    let view = cx.entity();
    let handler: CommandPaletteHandler = Arc::new(move |action, window, cx| {
      view.update(cx, |view, cx| {
        view.handle_command_palette_action(action, window, cx)
      })
    });

    let mut config = CommandPaletteConfig::new(palette_branches, commands, handler)
      .with_repositories(palette_repositories)
      .with_rebase_branches(palette_rebase_branches)
      .with_delete_branches(palette_delete_branches)
      .with_stashes(palette_stashes);
    if let Some(default_stash_message) = palette_default_stash_message {
      config = config.with_default_stash_message(default_stash_message);
    }
    if let Some(initial_screen) = initial_screen {
      config = config.with_initial_screen(initial_screen);
    }

    let palette = cx.new(|cx| CommandPalette::new(window, cx, config));
    ui::open_palette_dialog(palette, window, cx);
  }

  pub(super) fn command_palette_branch(branch: &BranchRef) -> CommandPaletteBranch {
    CommandPaletteBranch {
      name: branch.name.clone().into(),
      kind: match branch.kind {
        BranchKind::Local => CommandPaletteBranchKind::Local,
        BranchKind::Remote => CommandPaletteBranchKind::Remote,
      },
    }
  }

  pub(super) fn command_palette_rebase_branches(
    branches: &[BranchRef],
    current_branch_name: Option<&str>,
    upstream_branch: Option<&BranchRef>,
    default_branch: Option<&BranchRef>,
  ) -> Vec<CommandPaletteBranch> {
    let mut candidates = branches
      .iter()
      .enumerate()
      .filter(|(_, branch)| {
        !matches!(
          (branch.kind, current_branch_name),
          (BranchKind::Local, Some(current_branch_name)) if branch.name == current_branch_name
        )
      })
      .map(|(index, branch)| {
        (
          index,
          Self::rebase_branch_priority(branch, upstream_branch, default_branch),
          branch,
        )
      })
      .collect::<Vec<_>>();

    candidates.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    candidates
      .into_iter()
      .map(|(_, _, branch)| Self::command_palette_branch(branch))
      .collect()
  }

  pub(super) fn rebase_branch_priority(
    branch: &BranchRef,
    upstream_branch: Option<&BranchRef>,
    default_branch: Option<&BranchRef>,
  ) -> usize {
    if upstream_branch.is_some_and(|upstream| upstream == branch) {
      return 0;
    }
    if default_branch.is_some_and(|default| default == branch) {
      return 1;
    }
    match (branch.kind, branch.name.as_str()) {
      (BranchKind::Local, "main" | "master") => 2,
      (
        BranchKind::Remote,
        "origin/main" | "origin/master" | "upstream/main" | "upstream/master",
      ) => 2,
      _ => 3,
    }
  }

  pub(super) fn build_command_palette_contents(
    &self,
    palette_repositories_len: usize,
    cx: &App,
  ) -> GitCommandPaletteContents {
    let include_github = self.auth_state.has_github_access();
    let mut commands = Vec::new();
    let mut stashes = Vec::new();
    let mut default_stash_message_value = None;
    let mut branches = Vec::new();
    let mut rebase_branches = Vec::new();
    let mut delete_branches = Vec::new();

    if let Some(root_path) = self.selected_repo.clone() {
      let commit_message = self.commit_input.read(cx).value().to_string();
      if self.should_show_commit_palette_command(&commit_message) {
        commands.push(CommandPaletteCommand::commit());
      }
      if self.should_show_continue_rebase_palette_command() {
        commands.push(CommandPaletteCommand::continue_rebase());
      } else if let Some(reason) = self.continue_rebase_disabled_reason() {
        commands.push(CommandPaletteCommand::continue_rebase().disabled(reason));
      }
      if self.should_show_skip_rebase_palette_command() {
        commands.push(CommandPaletteCommand::skip_rebase());
      }
      if self.should_show_push_palette_command() {
        let push_label = Self::push_action_label(self.branch_status.as_ref(), self.has_head_commit);
        commands.push(CommandPaletteCommand::push(push_label));
      }
      if self.should_show_force_push_palette_command() {
        commands.push(CommandPaletteCommand::force_push());
      }
      if self.should_show_undo_last_commit_palette_command() {
        commands.push(CommandPaletteCommand::undo_last_commit());
      }
      if self.should_show_amend_palette_command() {
        commands.push(CommandPaletteCommand::amend());
      }
      if self.should_show_checkout_detached_palette_command() {
        commands.push(CommandPaletteCommand::checkout_detached());
      }

      if Self::should_show_stage_all_command(&self.status_entries) {
        commands.push(CommandPaletteCommand::stage_all());
      }
      if Self::should_show_unstage_all_palette_command(&self.status_entries) {
        commands.push(CommandPaletteCommand::unstage_all());
      }
      if self.should_show_unstage_selected_file_palette_command() {
        commands.push(CommandPaletteCommand::unstage_selected_file());
      } else if self.should_show_stage_selected_file_palette_command() {
        commands.push(CommandPaletteCommand::stage_selected_file());
      }
      if self.should_show_accept_all_conflicts_palette_commands(cx) {
        commands.push(CommandPaletteCommand::accept_all_current_conflicts());
        commands.push(CommandPaletteCommand::accept_all_incoming_conflicts());
      }
      if self.should_show_pull_palette_command() {
        commands.push(CommandPaletteCommand::pull());
      } else if self.selected_repo.is_some()
        && self
          .branch_status
          .as_ref()
          .is_some_and(|status| status.has_upstream)
        && let Some(reason) = self.operation_in_progress_disabled_reason()
      {
        commands.push(CommandPaletteCommand::pull().disabled(reason));
      }
      commands.push(CommandPaletteCommand::fetch());
      if self.should_show_cherry_pick_palette_command() {
        commands.push(CommandPaletteCommand::cherry_pick());
      } else if self.selected_repo.is_some()
        && let Some(reason) = self.operation_in_progress_disabled_reason()
      {
        commands.push(CommandPaletteCommand::cherry_pick().disabled(reason));
      }

      let (show_stash, show_stash_with_untracked) = Self::stash_command_flags(&self.status_entries);

      if show_stash {
        commands.push(CommandPaletteCommand::stash());
      }

      if show_stash_with_untracked {
        commands.push(CommandPaletteCommand::stash_with_untracked());
        default_stash_message_value = default_stash_message(&root_path).ok().map(Into::into);
      }

      if let Ok(repo_stashes) = list_stashes(&root_path) {
        stashes = repo_stashes
          .into_iter()
          .map(|stash| CommandPaletteStash {
            index: stash.index,
            name: stash.name.into(),
            oid: stash.oid.into(),
          })
          .collect();

        if !stashes.is_empty() {
          commands.push(CommandPaletteCommand::apply_stash());
          commands.push(CommandPaletteCommand::drop_stash());
          commands.push(CommandPaletteCommand::pop_stash());
        }
      }

      if let Ok(repo_branches) = list_branches(&root_path) {
        let current_branch_name = self
          .branch_status
          .as_ref()
          .map(|status| status.name.clone())
          .or_else(|| {
            current_branch_status(&root_path)
              .ok()
              .map(|status| status.name)
          });
        delete_branches = repo_branches
          .iter()
          .filter(|branch| match branch.kind {
            BranchKind::Local => current_branch_name
              .as_ref()
              .is_none_or(|current_branch_name| branch.name != *current_branch_name),
            BranchKind::Remote => true,
          })
          .map(|branch| CommandPaletteBranch {
            name: branch.name.clone().into(),
            kind: match branch.kind {
              BranchKind::Local => CommandPaletteBranchKind::Local,
              BranchKind::Remote => CommandPaletteBranchKind::Remote,
            },
          })
          .collect::<Vec<_>>();
        let upstream_branch = current_branch_upstream(&root_path).ok().flatten();
        let default_branch = default_remote_branch(&root_path).ok().flatten();
        rebase_branches = Self::command_palette_rebase_branches(
          &repo_branches,
          current_branch_name.as_deref(),
          upstream_branch.as_ref(),
          default_branch.as_ref(),
        );
        branches = repo_branches
          .iter()
          .map(Self::command_palette_branch)
          .collect::<Vec<_>>();
        commands.push(CommandPaletteCommand::switch_branch());
        if !delete_branches.is_empty() {
          commands.push(CommandPaletteCommand::delete_branch());
        }
        if self.should_show_merge_branch_palette_command() {
          commands.push(CommandPaletteCommand::merge_branch());
        } else if let Some(reason) = self.operation_in_progress_disabled_reason() {
          commands.push(CommandPaletteCommand::merge_branch().disabled(reason));
        }
        if self.merge_in_progress {
          commands.push(CommandPaletteCommand::abort_merge());
        }
        if self.should_show_rebase_branch_palette_command() && !rebase_branches.is_empty() {
          commands.push(CommandPaletteCommand::rebase_branch());
        } else if let Some(reason) = self.operation_in_progress_disabled_reason() {
          commands.push(CommandPaletteCommand::rebase_branch().disabled(reason));
        }
        if self.should_show_interactive_rebase_palette_command() {
          commands.push(CommandPaletteCommand::interactive_rebase());
        } else if let Some(reason) = self.interactive_rebase_disabled_reason() {
          commands.push(CommandPaletteCommand::interactive_rebase().disabled(reason));
        }
        if self.rebase_in_progress {
          commands.push(CommandPaletteCommand::abort_rebase());
        }
      }

      if let Some(command) = self.branch_pull_request_palette_command(cx) {
        commands.push(command);
      }
    }

    if palette_repositories_len > 1 {
      commands.push(CommandPaletteCommand::switch_repository());
    }
    if palette_repositories_len > 0 {
      commands.push(CommandPaletteCommand::forget_repository());
    }
    commands.push(CommandPaletteCommand::open_repository());
    commands.extend(CommandPaletteCommand::default_global_commands(
      CommandPalettePage::Git,
      include_github,
    ));

    GitCommandPaletteContents {
      commands,
      branches,
      rebase_branches,
      delete_branches,
      stashes,
      default_stash_message: default_stash_message_value,
    }
  }

  pub(super) fn open_file_search_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.selected_repo.is_none() {
      return;
    }

    let entries = self.git_file_search_entries();
    if entries.is_empty() {
      return;
    }

    let view = cx.entity();
    let handler: SearchFileHandler = Arc::new(move |path, window, cx| {
      view.update(cx, |view, cx| {
        view.open_file(path, cx);
      });

      let view_for_focus = view.clone();
      window.on_next_frame(move |window, cx| {
        if let Some(editor) = view_for_focus.read(cx).editor.clone() {
          let focus_handle: FocusHandle = editor.read(cx).focus_handle(cx);
          window.focus(&focus_handle, cx);
        } else {
          let focus_handle = view_for_focus.read(cx).focus_handle(cx);
          window.focus(&focus_handle, cx);
        }
      });
      Ok(())
    });
    open_shared_file_search_palette(window, cx, entries, handler, false);
  }

  pub(super) fn git_file_search_entries(&self) -> Vec<SearchFileEntry> {
    let Some(root_path) = self.selected_repo.as_ref() else {
      return Vec::new();
    };

    let changed_entries = self
      .status_entries
      .iter()
      .map(|entry| {
        let file_label = entry.path.to_string_lossy();
        let file_label = file_label.replace(['\n', '\r'], "");
        SearchFileEntry::new(entry.path.clone(), file_label).grouped("Changed")
      })
      .collect::<Vec<_>>();

    let mut changed_paths = HashSet::new();
    for entry in &self.status_entries {
      changed_paths.insert(entry.path.clone());
      if let Some(old_path) = entry.old_path.as_ref() {
        changed_paths.insert(old_path.clone());
      }
    }

    let unchanged_entries = list_repo_head_files(root_path)
      .unwrap_or_default()
      .into_iter()
      .filter(|path| !changed_paths.contains(path))
      .map(|path| {
        let file_label = path.to_string_lossy();
        let file_label = file_label.replace(['\n', '\r'], "");
        SearchFileEntry::new(path, file_label).grouped("Unchanged")
      });

    changed_entries
      .into_iter()
      .chain(unchanged_entries)
      .collect()
  }

  pub(super) fn handle_command_palette_action(
    &mut self,
    action: CommandPaletteAction,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    let mut should_post_action_refresh = true;
    let action_for_error = action.clone();
    let result = match action {
      CommandPaletteAction::OpenRepository => {
        self.start_open_repository(window, cx);
        Ok(())
      }
      CommandPaletteAction::OpenSessionPage => {
        NavigationHistory::navigate("/session", cx);
        Ok(())
      }
      CommandPaletteAction::OpenGitPage => {
        NavigationHistory::navigate("/git", cx);
        Ok(())
      }
      CommandPaletteAction::OpenGithubPrDetails {
        owner,
        repo,
        number,
        open_changes_tab,
        review_comment_id,
      } => {
        GithubPrDetailsPageHandle::show_with_open_target(
          owner.into(),
          repo.into(),
          number,
          open_changes_tab,
          review_comment_id,
          cx,
        );
        Ok(())
      }
      CommandPaletteAction::OpenGithubRepoDetails {
        owner,
        repo,
        tab,
        issue_number,
        issue_comment_id,
      } => {
        open_repo_target(owner, repo, tab, issue_number, issue_comment_id, cx);
        Ok(())
      }
      CommandPaletteAction::OpenGithubCommitDetails { owner, repo, sha } => {
        open_commit_target(owner, repo, sha, cx);
        Ok(())
      }
      CommandPaletteAction::OpenGithubProfile { login } => {
        open_profile_target(login, cx);
        Ok(())
      }
      CommandPaletteAction::SwitchToPrBranch
      | CommandPaletteAction::CopyPrBranch
      | CommandPaletteAction::ToggleUnchangedFiles
      | CommandPaletteAction::OpenPrMergePopover
      | CommandPaletteAction::OpenPrReviewPopover
      | CommandPaletteAction::TogglePrCommitByCommit => {
        Err(anyhow::anyhow!("Command not available."))
      }
      CommandPaletteAction::CreatePullRequest => {
        let branch_context = self
          .create_pull_request_branch_context(cx)
          .ok_or_else(|| SharedString::from("Command not available."))?;
        let api = self.api.clone();
        let window_handle = self.window_handle;
        let on_created = git_page_created_handler(cx.entity().downgrade());
        window.on_next_frame(move |window, cx| {
          open_create_pull_request_dialog(
            api,
            window_handle,
            on_created,
            branch_context,
            window,
            cx,
          );
        });
        Ok(())
      }
      CommandPaletteAction::OpenPullRequest => {
        let GitBranchPullRequestButtonState::OpenExisting {
          owner,
          repo,
          number,
        } = self.current_branch_pr_button_state(cx)
        else {
          return Err("No open pull request found for this branch.".into());
        };
        GithubPrDetailsPageHandle::show_with_open_target(
          owner.into(),
          repo.into(),
          number,
          false,
          None,
          cx,
        );
        Ok(())
      }
      CommandPaletteAction::OpenSettingsPage => {
        NavigationHistory::navigate("/settings", cx);
        Ok(())
      }
      CommandPaletteAction::OpenBillingPage => {
        NavigationHistory::navigate("/billing", cx);
        Ok(())
      }
      CommandPaletteAction::OpenAboutPage => {
        NavigationHistory::navigate("/about", cx);
        Ok(())
      }
      CommandPaletteAction::OpenGitConfigPage => {
        NavigationHistory::navigate("/git-config", cx);
        Ok(())
      }
      CommandPaletteAction::SendFeedback => {
        should_post_action_refresh = false;
        crate::feedback_dialog::open_feedback_dialog(window, cx);
        Ok(())
      }
      CommandPaletteAction::CheckoutDetached { target } => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        if !self.should_show_checkout_detached_palette_command() {
          return Err("Checkout detached is currently disabled.".into());
        }
        self.advance_status_refresh_generation();
        checkout_detached_target(&root_path, &target)
      }
      CommandPaletteAction::Commit => {
        if self.selected_repo.is_none() {
          return Err("No repository selected.".into());
        }
        let commit_message = self.commit_input.read(cx).value().to_string();
        if !self.should_show_commit_palette_command(&commit_message) {
          return Err("Commit command is currently disabled.".into());
        }
        should_post_action_refresh = false;
        self.commit_changes_inner(window, cx);
        Ok(())
      }
      CommandPaletteAction::ContinueRebase => {
        if self.selected_repo.is_none() {
          return Err("No repository selected.".into());
        }
        if !self.should_show_continue_rebase_palette_command() {
          return Err("Rebase continue is currently disabled.".into());
        }
        should_post_action_refresh = false;
        self.continue_rebase_inner(cx);
        Ok(())
      }
      CommandPaletteAction::SkipRebase => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        if !self.should_show_skip_rebase_palette_command() {
          return Err("No rebase in progress.".into());
        }
        self.add_git_breadcrumb("Skip rebase started", Map::new());
        match skip_rebase(&root_path) {
          Ok(()) => {
            if !is_rebase_in_progress(&root_path).unwrap_or(false) {
              self.force_push_after_rebase = true;
            }
            self.add_git_breadcrumb("Skip rebase succeeded", Map::new());
            Ok(())
          }
          Err(err) => {
            let err_text = err.to_string();
            if let Some(path) = Self::first_conflicted_path(&root_path) {
              let mut data = Map::new();
              data.insert("error".into(), err_text.into());
              data.insert(
                "file".into(),
                path.to_string_lossy().replace(['\n', '\r'], "").into(),
              );
              self.record_git_expected_error("git.rebase.skip", "conflict", data.clone());
              self.add_git_breadcrumb("Skip rebase blocked by conflicts", data);
              if let Some(rebase_message) = current_rebase_commit_message(&root_path).ok().flatten()
              {
                self
                  .commit_input
                  .update(cx, |input, cx| input.set_value(&rebase_message, window, cx));
              }
              self.set_sidebar_mode(GitSidebarMode::Changes, window, cx);
              self.open_file_revealing_first_conflict(path, cx);
              Ok(())
            } else {
              let mut data = Map::new();
              data.insert("error".into(), err_text.clone().into());
              self.add_git_breadcrumb("Skip rebase failed", data.clone());
              self.record_git_unexpected_error("git.rebase.skip", err_text.as_str(), data);
              Err(err)
            }
          }
        }
      }
      CommandPaletteAction::Push => {
        if self.selected_repo.is_none() {
          return Err("No repository selected.".into());
        }
        if !self.should_show_push_palette_command() {
          return Err("Push command is currently disabled.".into());
        }
        should_post_action_refresh = false;
        self.push_changes_action(cx);
        Ok(())
      }
      CommandPaletteAction::ForcePush => {
        if self.selected_repo.is_none() {
          return Err("No repository selected.".into());
        }
        if !self.should_show_force_push_palette_command() {
          return Err("Force push command is currently disabled.".into());
        }
        should_post_action_refresh = false;
        self.force_push_changes_action(cx);
        Ok(())
      }
      CommandPaletteAction::UndoLastCommit => {
        if self.selected_repo.is_none() {
          return Err("No repository selected.".into());
        }
        if !self.should_show_undo_last_commit_palette_command() {
          return Err("Undo last commit command is currently disabled.".into());
        }
        should_post_action_refresh = false;
        self.undo_last_commit_action(cx);
        Ok(())
      }
      CommandPaletteAction::Amend => {
        if self.selected_repo.is_none() {
          return Err("No repository selected.".into());
        }
        if !self.should_show_amend_palette_command() {
          return Err("Amend command is currently disabled.".into());
        }
        should_post_action_refresh = false;
        self.commit_amend_changes(window, cx);
        Ok(())
      }
      CommandPaletteAction::StageSelectedFile => {
        if self.selected_repo.is_none() {
          return Err("No repository selected.".into());
        }
        if !self.should_show_stage_selected_file_palette_command() {
          return Err("Stage file command is currently disabled.".into());
        }
        let Some(selected_entry) = self.selected_file_entry().cloned() else {
          return Err("Stage file command is currently disabled.".into());
        };
        should_post_action_refresh = false;
        let has_unresolved_conflict_markers = self.editor.as_ref().is_none_or(|editor| {
          editor.read_with(cx, |editor, cx| editor.has_unresolved_conflict_markers(cx))
        });
        if Self::should_confirm_stage_for_status(
          Some(selected_entry.status),
          has_unresolved_conflict_markers,
        ) {
          self.confirm_stage_conflicted_file_action(window, selected_entry.path.clone(), cx);
        } else {
          self.stage_file_action(selected_entry.path.clone(), cx);
        }
        Ok(())
      }
      CommandPaletteAction::UnstageSelectedFile => {
        if self.selected_repo.is_none() {
          return Err("No repository selected.".into());
        }
        if !self.should_show_unstage_selected_file_palette_command() {
          return Err("Unstage file command is currently disabled.".into());
        }
        let Some(selected_entry) = self.selected_file_entry().cloned() else {
          return Err("Unstage file command is currently disabled.".into());
        };
        should_post_action_refresh = false;
        self.unstage_file_action(selected_entry.path.clone(), cx);
        Ok(())
      }
      CommandPaletteAction::AcceptAllCurrentConflicts => {
        if self.selected_repo.is_none() {
          return Err("No repository selected.".into());
        }
        if !self.should_show_accept_all_conflicts_palette_commands(cx) {
          return Err("Accept all current conflicts is currently disabled.".into());
        }
        should_post_action_refresh = false;
        self.resolve_all_conflicts_in_editor(ConflictResolution::Current, cx);
        Ok(())
      }
      CommandPaletteAction::AcceptAllIncomingConflicts => {
        if self.selected_repo.is_none() {
          return Err("No repository selected.".into());
        }
        if !self.should_show_accept_all_conflicts_palette_commands(cx) {
          return Err("Accept all incoming conflicts is currently disabled.".into());
        }
        should_post_action_refresh = false;
        self.resolve_all_conflicts_in_editor(ConflictResolution::Incoming, cx);
        Ok(())
      }
      CommandPaletteAction::SwitchRepository(repository) => {
        let repo_root = PathBuf::from(repository.path.as_ref());
        if !repo_root.is_dir() {
          let message: SharedString =
            format!("Repository not found: {}", repo_root.display()).into();
          return Err(message);
        }
        self.set_selected_repo(repo_root, cx);
        Ok(())
      }
      CommandPaletteAction::ForgetRepository(repository) => {
        should_post_action_refresh = false;
        let repo_root = PathBuf::from(repository.path.as_ref());
        let forgetting_selected = self.selected_repo.as_deref() == Some(repo_root.as_path());
        ConfigStore::forget_recent_repository(&repo_root);

        if forgetting_selected {
          let next_repo = ConfigStore::load_recent_repositories()
            .into_iter()
            .map(|repo| repo.path)
            .find(|path| path != &repo_root);
          match next_repo {
            Some(next) => self.set_selected_repo(next, cx),
            None => self.clear_selected_repo(cx),
          }
        } else {
          self.refresh_repo_select(cx);
        }
        Ok(())
      }
      CommandPaletteAction::SwitchBranch(branch) => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        let branch_ref = BranchRef {
          name: branch.name.to_string(),
          kind: match branch.kind {
            CommandPaletteBranchKind::Local => BranchKind::Local,
            CommandPaletteBranchKind::Remote => BranchKind::Remote,
          },
        };
        switch_branch(&root_path, &branch_ref)
      }
      CommandPaletteAction::CreateBranch { name } => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        let branch_ref = BranchRef {
          name: name.clone(),
          kind: BranchKind::Local,
        };
        create_branch(&root_path, &name).and_then(|_| switch_branch(&root_path, &branch_ref))
      }
      CommandPaletteAction::CreateBranchFrom { name, base } => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        let branch_ref = BranchRef {
          name: base.name.to_string(),
          kind: match base.kind {
            CommandPaletteBranchKind::Local => BranchKind::Local,
            CommandPaletteBranchKind::Remote => BranchKind::Remote,
          },
        };
        let new_branch = BranchRef {
          name: name.clone(),
          kind: BranchKind::Local,
        };
        create_branch_from(&root_path, &name, &branch_ref)
          .and_then(|_| switch_branch(&root_path, &new_branch))
      }
      CommandPaletteAction::DeleteBranch(branch) => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        let branch_ref = BranchRef {
          name: branch.name.to_string(),
          kind: match branch.kind {
            CommandPaletteBranchKind::Local => BranchKind::Local,
            CommandPaletteBranchKind::Remote => BranchKind::Remote,
          },
        };
        let result = delete_branch(&root_path, &branch_ref);
        if result.is_ok() {
          window.push_notification(
            Notification::success(format!("Deleted branch {}", branch_ref.name)),
            cx,
          );
        }
        result
      }
      CommandPaletteAction::MergeBranch { name } => {
        if self.selected_repo.is_none() {
          return Err("No repository selected.".into());
        }
        if !self.should_show_merge_branch_palette_command() {
          return Err("Merge command is currently disabled.".into());
        }
        let branch_ref = BranchRef {
          name: name.name.to_string(),
          kind: match name.kind {
            CommandPaletteBranchKind::Local => BranchKind::Local,
            CommandPaletteBranchKind::Remote => BranchKind::Remote,
          },
        };
        self.merge_branch_action(branch_ref, Some(window), true, cx)
      }
      CommandPaletteAction::AbortMerge => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        let result = abort_merge(&root_path);
        if result.is_ok() {
          self
            .commit_input
            .update(cx, |input, cx| input.set_value("", window, cx));
          window.push_notification(Notification::success("Aborted merge"), cx);
        }
        result
      }
      CommandPaletteAction::RebaseBranch { name } => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        if !self.should_show_rebase_branch_palette_command() {
          return Err("Rebase command is currently disabled.".into());
        }
        let branch_ref = BranchRef {
          name: name.name.to_string(),
          kind: match name.kind {
            CommandPaletteBranchKind::Local => BranchKind::Local,
            CommandPaletteBranchKind::Remote => BranchKind::Remote,
          },
        };
        let mut start_data = Map::new();
        start_data.insert("target_branch".into(), branch_ref.name.clone().into());
        self.add_git_breadcrumb("Rebase started", start_data);
        crate::analytics::track(cx, "rebase_done");
        match rebase_branch(&root_path, &branch_ref) {
          Ok(outcome) => {
            let mut data = Map::new();
            data.insert("target_branch".into(), branch_ref.name.clone().into());
            match outcome {
              RebaseBranchOutcome::AlreadyUpToDate => {
                self.add_git_breadcrumb("Rebase already up to date", data);
                window.push_notification(
                  Notification::info(format!("Already up to date with {}", branch_ref.name)),
                  cx,
                );
              }
              RebaseBranchOutcome::Rebased => {
                self.force_push_after_rebase = true;
                self.add_git_breadcrumb("Rebase succeeded", data);
                window.push_notification(
                  Notification::success(format!("Rebased onto {}", branch_ref.name)),
                  cx,
                );
              }
            }
            Ok(())
          }
          Err(err) => {
            let err_text = err.to_string();
            if let Some(path) = Self::first_conflicted_path(&root_path) {
              let mut data = Map::new();
              data.insert("target_branch".into(), branch_ref.name.clone().into());
              data.insert(
                "file".into(),
                path.to_string_lossy().replace(['\n', '\r'], "").into(),
              );
              data.insert("error".into(), err_text.into());
              self.record_git_expected_error("git.rebase", "conflict", data.clone());
              self.add_git_breadcrumb("Rebase has conflicts", data);
              if let Some(rebase_message) = current_rebase_commit_message(&root_path).ok().flatten()
              {
                self
                  .commit_input
                  .update(cx, |input, cx| input.set_value(&rebase_message, window, cx));
              }
              self.set_sidebar_mode(GitSidebarMode::Changes, window, cx);
              self.open_file_revealing_first_conflict(path, cx);
              Ok(())
            } else {
              let mut data = Map::new();
              data.insert("target_branch".into(), branch_ref.name.clone().into());
              data.insert("error".into(), err_text.clone().into());
              self.add_git_breadcrumb("Rebase failed", data.clone());
              self.record_git_unexpected_error("git.rebase", err_text.as_str(), data);
              Err(err)
            }
          }
        }
      }
      CommandPaletteAction::InteractiveRebaseBranch { ref name }
      | CommandPaletteAction::InteractiveRebaseEditBranch { ref name } => {
        if self.selected_repo.is_none() {
          return Err("No repository selected.".into());
        }
        if !self.should_show_interactive_rebase_palette_command() {
          return Err("Interactive rebase is currently disabled.".into());
        }
        should_post_action_refresh = false;
        let branch_ref = BranchRef {
          name: name.name.to_string(),
          kind: match name.kind {
            CommandPaletteBranchKind::Local => BranchKind::Local,
            CommandPaletteBranchKind::Remote => BranchKind::Remote,
          },
        };
        let target = if matches!(
          action,
          CommandPaletteAction::InteractiveRebaseEditBranch { .. }
        ) {
          InteractiveRebaseTarget::BranchInPlace(branch_ref)
        } else {
          InteractiveRebaseTarget::Branch(branch_ref)
        };
        let preview = self.prepare_interactive_rebase_commits(&target)?;
        self.dispatch_interactive_rebase_target(target, preview, window, cx);
        Ok(())
      }
      CommandPaletteAction::InteractiveRebaseHeadCount { count } => {
        if self.selected_repo.is_none() {
          return Err("No repository selected.".into());
        }
        if !self.should_show_interactive_rebase_palette_command() {
          return Err("Interactive rebase is currently disabled.".into());
        }
        should_post_action_refresh = false;
        let target = InteractiveRebaseTarget::HeadCount(count);
        let preview = self.prepare_interactive_rebase_commits(&target)?;
        self.dispatch_interactive_rebase_target(target, preview, window, cx);
        Ok(())
      }
      CommandPaletteAction::AbortRebase => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        let result = abort_rebase(&root_path);
        if result.is_ok() {
          self.force_push_after_rebase = false;
          self
            .commit_input
            .update(cx, |input, cx| input.set_value("", window, cx));
          window.push_notification(Notification::success("Aborted rebase"), cx);
        }
        result
      }
      CommandPaletteAction::StageAll => {
        if self.selected_repo.is_none() {
          return Err("No repository selected.".into());
        }
        should_post_action_refresh = false;
        if Self::should_confirm_stage_all(self.selected_repo.as_ref(), &self.status_entries) {
          self.confirm_stage_all_conflicted_action(window, cx);
        } else {
          self.stage_all_action(cx);
        }
        Ok(())
      }
      CommandPaletteAction::UnstageAll => {
        if self.selected_repo.is_none() {
          return Err("No repository selected.".into());
        }
        should_post_action_refresh = false;
        self.unstage_all_action(cx);
        Ok(())
      }
      CommandPaletteAction::Pull => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        if !self.should_show_pull_palette_command() {
          return Err("Pull command is currently disabled.".into());
        }
        should_post_action_refresh = false;
        self.pull_repository(root_path, cx);
        Ok(())
      }
      CommandPaletteAction::Fetch => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        should_post_action_refresh = false;
        self.fetch_repository(root_path, cx);
        Ok(())
      }
      CommandPaletteAction::Stash {
        include_untracked,
        message,
      } => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        crate::analytics::track(cx, "stash_created");
        let result = create_stash(&root_path, include_untracked, message.as_deref());
        if result.is_ok() {
          window.push_notification(Notification::success("Stashed changes"), cx);
        }
        result
      }
      CommandPaletteAction::ApplyStash(stash) => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        let result = apply_stash(&root_path, stash.index);
        if result.is_ok() {
          window.push_notification(
            Notification::success(format!("Applied stash {}", stash.name)),
            cx,
          );
        }
        result
      }
      CommandPaletteAction::DropStash(stash) => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        let result = drop_stash(&root_path, stash.index);
        if result.is_ok() {
          window.push_notification(
            Notification::success(format!("Dropped stash {}", stash.name)),
            cx,
          );
        }
        result
      }
      CommandPaletteAction::PopStash(stash) => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        let result = pop_stash(&root_path, stash.index);
        if result.is_ok() {
          window.push_notification(
            Notification::success(format!("Popped stash {}", stash.name)),
            cx,
          );
        }
        result
      }
      CommandPaletteAction::CherryPick { commit_hashes } => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        if !self.should_show_cherry_pick_palette_command() {
          return Err("Cherry-pick command is currently disabled.".into());
        }
        let count = commit_hashes.len();
        crate::analytics::track(cx, "cherry_pick_done");
        let result = cherry_pick_commits(&root_path, &commit_hashes);
        if result.is_ok() {
          let label = if count == 1 { "commit" } else { "commits" };
          window.push_notification(
            Notification::success(format!("Cherry-picked {count} {label}")),
            cx,
          );
        }
        result
      }
    };

    if let Err(err) = result {
      self.handle_command_palette_operation_error(&action_for_error, err, window, cx);
    }

    if should_post_action_refresh {
      self.reload_status(cx);
      self.refresh_branches(cx);
      if let Some(editor) = self.editor.clone() {
        editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
      }
    }

    Ok(())
  }

  pub(super) fn should_show_commit_palette_command(&self, commit_message: &str) -> bool {
    !self.rebase_in_progress && self.commit_primary_action_enabled(commit_message)
  }

  pub(super) fn should_show_continue_rebase_palette_command(&self) -> bool {
    self.rebase_in_progress && self.can_continue_rebase_command()
  }

  pub(super) fn should_show_skip_rebase_palette_command(&self) -> bool {
    self.rebase_in_progress && self.selected_repo.is_some()
  }

  pub(super) fn should_show_push_palette_command(&self) -> bool {
    !self.rebase_in_progress && self.selected_repo.is_some() && self.can_push
  }

  pub(super) fn should_show_force_push_palette_command(&self) -> bool {
    !self.rebase_in_progress && self.selected_repo.is_some() && self.can_force_push
  }

  pub(super) fn should_show_undo_last_commit_palette_command(&self) -> bool {
    !self.rebase_in_progress && self.selected_repo.is_some() && self.can_undo_last_commit
  }

  pub(super) fn should_show_amend_palette_command(&self) -> bool {
    !self.rebase_in_progress && self.selected_repo.is_some() && self.has_head_commit
  }

  pub(super) fn should_show_checkout_detached_palette_command(&self) -> bool {
    self.selected_repo.is_some()
      && self.has_head_commit
      && !self.merge_in_progress
      && !self.rebase_in_progress
      && self.branch_status.is_some()
      && !Self::is_detached_head(self.branch_status.as_ref())
  }

  pub(super) fn should_show_interactive_rebase_palette_command(&self) -> bool {
    !self.rebase_in_progress
      && !self.merge_in_progress
      && self.selected_repo.is_some()
      && self.has_head_commit
      && self.status_entries.is_empty()
      && !Self::is_detached_head(self.branch_status.as_ref())
  }

  pub(super) fn should_show_pull_palette_command(&self) -> bool {
    !self.rebase_in_progress
      && !self.merge_in_progress
      && self.selected_repo.is_some()
      && self
        .branch_status
        .as_ref()
        .is_some_and(|status| status.has_upstream)
  }

  pub(super) fn should_show_merge_branch_palette_command(&self) -> bool {
    !self.rebase_in_progress && !self.merge_in_progress && self.selected_repo.is_some()
  }

  pub(super) fn should_show_rebase_branch_palette_command(&self) -> bool {
    !self.rebase_in_progress && !self.merge_in_progress && self.selected_repo.is_some()
  }

  pub(super) fn should_show_cherry_pick_palette_command(&self) -> bool {
    !self.rebase_in_progress && !self.merge_in_progress && self.selected_repo.is_some()
  }

  pub(super) fn should_show_stage_selected_file_palette_command(&self) -> bool {
    self.selected_repo.is_some()
      && self
        .selected_file_entry()
        .is_some_and(|entry| Self::selected_file_can_stage(entry.stage))
  }

  pub(super) fn should_show_unstage_selected_file_palette_command(&self) -> bool {
    self.selected_repo.is_some()
      && self
        .selected_file_entry()
        .is_some_and(|entry| Self::selected_file_can_unstage(entry.stage))
  }

  pub(super) fn should_show_accept_all_conflicts_palette_commands(&self, cx: &App) -> bool {
    let selected_status = self.selected_file_status();
    self.editor.as_ref().is_some_and(|editor| {
      editor.read_with(cx, |editor, cx| {
        Self::can_accept_all_conflicts(
          selected_status,
          editor.is_read_only,
          editor.has_unresolved_conflict_markers(cx),
        )
      })
    })
  }

  pub(super) fn stash_command_flags(entries: &[RepoStatusEntry]) -> (bool, bool) {
    let show_stash = Self::has_tracked_entries(entries);
    let show_stash_with_untracked = show_stash || Self::has_untracked_entries(entries);
    (show_stash, show_stash_with_untracked)
  }

  pub(super) fn should_show_unstage_all_palette_command(entries: &[RepoStatusEntry]) -> bool {
    Self::has_staged_changes(entries)
  }
}

#[cfg(test)]
mod tests {
  use super::super::test_support::*;
  use super::*;
  use git2::{BranchType, Repository};
  use gpui::TestAppContext;
  use ui::CommandPaletteCommandId;

  #[gpui::test]
  fn palette_commit_and_rebase_commands_follow_commit_button_rules(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-palette-commit-rules");
    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update_in(cx, |this, _window, _cx| {
      this.selected_repo = Some(repo.path.clone());
      this.branch_status = Some(make_branch_status("main", 0, 0, true));
      this.rebase_in_progress = false;
      this.status_entries = vec![make_status_entry("README.md", RepoStage::Unstaged)];
      this.selected_file = Some(PathBuf::from("README.md"));
      this.has_head_commit = true;
      this.can_undo_last_commit = true;
      this.can_push = true;
      this.can_force_push = true;
      assert!(this.should_show_commit_palette_command("feat: commit"));
      assert!(!this.should_show_commit_palette_command("   "));
      assert!(!this.should_show_continue_rebase_palette_command());
      assert!(!this.should_show_skip_rebase_palette_command());
      assert!(this.should_show_push_palette_command());
      assert!(this.should_show_force_push_palette_command());
      assert!(this.should_show_undo_last_commit_palette_command());
      assert!(this.should_show_amend_palette_command());
      assert!(this.should_show_checkout_detached_palette_command());
      assert!(!this.should_show_interactive_rebase_palette_command());
      assert!(this.should_show_stage_selected_file_palette_command());
      assert!(!this.should_show_unstage_selected_file_palette_command());

      this.status_entries = vec![make_status_entry("README.md", RepoStage::Staged)];
      assert!(!this.should_show_stage_selected_file_palette_command());
      assert!(this.should_show_unstage_selected_file_palette_command());

      this.status_entries = vec![make_status_entry("README.md", RepoStage::PartiallyStaged)];
      assert!(!this.should_show_stage_selected_file_palette_command());
      assert!(this.should_show_unstage_selected_file_palette_command());

      this.status_entries.clear();
      assert!(this.should_show_checkout_detached_palette_command());
      assert!(this.should_show_interactive_rebase_palette_command());
      this.branch_status = Some(make_branch_status("HEAD", 0, 0, false));
      assert!(!this.should_show_checkout_detached_palette_command());
      assert!(!this.should_show_interactive_rebase_palette_command());
      this.branch_status = Some(make_branch_status("main", 0, 0, true));

      this.selected_file = None;
      assert!(!this.should_show_stage_selected_file_palette_command());
      assert!(!this.should_show_unstage_selected_file_palette_command());

      this.rebase_in_progress = true;
      assert!(!this.should_show_commit_palette_command("feat: commit"));
      assert!(this.should_show_continue_rebase_palette_command());
      assert!(this.should_show_skip_rebase_palette_command());
      assert!(!this.should_show_push_palette_command());
      assert!(!this.should_show_force_push_palette_command());
      assert!(!this.should_show_undo_last_commit_palette_command());
      assert!(!this.should_show_amend_palette_command());
      assert!(!this.should_show_checkout_detached_palette_command());
      assert!(!this.should_show_interactive_rebase_palette_command());
      assert!(!this.should_show_stage_selected_file_palette_command());
      assert!(!this.should_show_unstage_selected_file_palette_command());

      this.status_entries = vec![RepoStatusEntry {
        path: PathBuf::from("README.md"),
        old_path: None,
        status: RepoStatusKind::Conflicted,
        stage: RepoStage::Unstaged,
      }];
      assert!(!this.should_show_continue_rebase_palette_command());
      assert!(this.should_show_skip_rebase_palette_command());

      this.rebase_in_progress = false;
      this.merge_in_progress = true;
      this.status_entries = vec![RepoStatusEntry {
        path: PathBuf::from("README.md"),
        old_path: None,
        status: RepoStatusKind::Conflicted,
        stage: RepoStage::Unstaged,
      }];
      assert!(!this.should_show_commit_palette_command("Merge branch 'feature' into main"));

      this.status_entries = vec![RepoStatusEntry {
        path: PathBuf::from("README.md"),
        old_path: None,
        status: RepoStatusKind::Modified,
        stage: RepoStage::Staged,
      }];
      assert!(this.should_show_commit_palette_command("Merge branch 'feature' into main"));

      this.merge_in_progress = false;
      this.selected_repo = None;
      assert!(!this.should_show_push_palette_command());
      assert!(!this.should_show_force_push_palette_command());
      assert!(!this.should_show_undo_last_commit_palette_command());
      assert!(!this.should_show_amend_palette_command());
      assert!(!this.should_show_checkout_detached_palette_command());
      assert!(!this.should_show_interactive_rebase_palette_command());
      assert!(!this.should_show_stage_selected_file_palette_command());
      assert!(!this.should_show_unstage_selected_file_palette_command());
    });
  }

  #[gpui::test]
  fn command_palette_keeps_temporarily_blocked_commands_visible_with_reasons(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-palette-disabled-reasons");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "initial\n", "initial");
    std::fs::write(repo.path.join(rel_path), "changed\n").expect("modify tracked file");
    let (git_page, cx) = add_git_page_window_with_root(cx);

    let dirty_worktree_reason = git_page.update(cx, |this, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.branch_status = Some(make_branch_status("main", 0, 0, true));
      this.has_head_commit = true;
      this.status_entries = list_repo_status(&repo.path).expect("list status");
      this
        .build_command_palette_contents(1, cx)
        .commands
        .into_iter()
        .find(|command| command.id == CommandPaletteCommandId::InteractiveRebase)
        .and_then(|command| command.disabled_reason)
    });
    assert_eq!(
      dirty_worktree_reason.as_ref().map(|reason| reason.as_ref()),
      Some("Commit or stash worktree changes first")
    );

    let rebase_continue_reason = git_page.update(cx, |this, cx| {
      this.rebase_in_progress = true;
      this.status_entries = vec![RepoStatusEntry {
        path: rel_path.to_path_buf(),
        old_path: None,
        status: RepoStatusKind::Conflicted,
        stage: RepoStage::Unstaged,
      }];
      this
        .build_command_palette_contents(1, cx)
        .commands
        .into_iter()
        .find(|command| command.id == CommandPaletteCommandId::ContinueRebase)
        .and_then(|command| command.disabled_reason)
    });
    assert_eq!(
      rebase_continue_reason
        .as_ref()
        .map(|reason| reason.as_ref()),
      Some("Resolve and stage conflicts first")
    );
  }

  #[test]
  fn unstage_all_palette_command_visibility_requires_any_staged_entry() {
    let unstaged_only_entries = vec![make_status_entry("src/main.rs", RepoStage::Unstaged)];
    let mixed_entries = vec![
      make_status_entry("src/main.rs", RepoStage::Staged),
      make_status_entry("src/lib.rs", RepoStage::Unstaged),
    ];
    let partial_entries = vec![make_status_entry(
      "src/editor.rs",
      RepoStage::PartiallyStaged,
    )];

    assert!(!GitPage::should_show_unstage_all_palette_command(&[]));
    assert!(!GitPage::should_show_unstage_all_palette_command(
      &unstaged_only_entries
    ));
    assert!(GitPage::should_show_unstage_all_palette_command(
      &mixed_entries
    ));
    assert!(GitPage::should_show_unstage_all_palette_command(
      &partial_entries
    ));
  }

  #[gpui::test]
  async fn command_palette_create_branch_creates_and_switches_to_branch(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-create-branch");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::CreateBranch {
          name: "feature".to_string(),
        },
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let status = current_branch_status(&repo.path).expect("read status");
    assert_eq!(status.name, "feature");
    assert!(
      list_branches(&repo.path)
        .expect("list branches")
        .iter()
        .any(|branch| branch.kind == BranchKind::Local && branch.name == "feature")
    );

    let selected_branch = git_page.read_with(cx, |this, _cx| selected_branch_from_dropdown(this));
    assert_eq!(
      selected_branch,
      Some(BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      })
    );
  }

  #[gpui::test]
  async fn command_palette_create_branch_shows_notification_only_when_branch_exists(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-create-branch-existing");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let base_branch = current_branch_status(&repo.path).expect("base status").name;
    create_branch(&repo.path, "feature").expect("create existing target branch");

    let mut mounted_git_page = None;
    let (root, cx) = cx.add_window_view(|window, cx| {
      let git_page = cx.new(|cx| GitPage::new_for_test(window, cx));
      mounted_git_page = Some(git_page.clone());
      gpui_component::Root::new(git_page, window, cx)
    });
    let git_page = mounted_git_page.expect("git page");
    cx.executor().allow_parking();
    cx.executor().allow_parking();

    let initial_notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(initial_notification_count, 0);

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::CreateBranch {
          name: "feature".to_string(),
        },
        _window,
        cx,
      )
    });
    assert!(
      result.is_ok(),
      "create branch failure should close palette and rely on notification feedback"
    );

    await_git_page_background_tasks(git_page.clone(), cx).await;
    cx.cx.run_until_parked();
    cx.run_until_parked();

    let notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(notification_count, 1);
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after failed create")
        .name,
      base_branch
    );
    let feature_count = list_branches(&repo.path)
      .expect("list branches after failed create")
      .iter()
      .filter(|branch| branch.kind == BranchKind::Local && branch.name == "feature")
      .count();
    assert_eq!(feature_count, 1);
  }

  #[gpui::test]
  async fn command_palette_switch_branch_switches_to_requested_branch(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-switch-branch");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    create_branch(&repo.path, "feature").expect("create feature branch");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::SwitchBranch(CommandPaletteBranch {
          name: "feature".into(),
          kind: CommandPaletteBranchKind::Local,
        }),
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let status = current_branch_status(&repo.path).expect("read status");
    assert_eq!(status.name, "feature");
  }

  #[gpui::test]
  async fn command_palette_delete_branch_deletes_requested_branch(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-delete-branch");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    create_branch(&repo.path, "feature").expect("create feature branch");
    let current_branch = current_branch_status(&repo.path)
      .expect("read current branch")
      .name;
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "v2-feature\n",
      "feature change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: current_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.branch_status = Some(make_branch_status(&current_branch, 0, 0, true));
      this.handle_command_palette_action(
        CommandPaletteAction::DeleteBranch(CommandPaletteBranch {
          name: "feature".into(),
          kind: CommandPaletteBranchKind::Local,
        }),
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert_eq!(
      current_branch_status(&repo.path)
        .expect("read status after delete")
        .name,
      current_branch
    );
    assert!(
      !list_branches(&repo.path)
        .expect("list branches after delete")
        .iter()
        .any(|branch| branch.kind == BranchKind::Local && branch.name == "feature")
    );
  }

  #[gpui::test]
  async fn command_palette_delete_branch_rejects_current_branch_with_notification_only(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-delete-current-branch");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let current_branch = current_branch_status(&repo.path)
      .expect("read current branch")
      .name;

    let mut mounted_git_page = None;
    let (root, cx) = cx.add_window_view(|window, cx| {
      let git_page = cx.new(|cx| GitPage::new_for_test(window, cx));
      mounted_git_page = Some(git_page.clone());
      gpui_component::Root::new(git_page, window, cx)
    });
    let git_page = mounted_git_page.expect("git page");
    cx.executor().allow_parking();

    let initial_notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(initial_notification_count, 0);

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.branch_status = Some(make_branch_status(&current_branch, 0, 0, true));
      this.handle_command_palette_action(
        CommandPaletteAction::DeleteBranch(CommandPaletteBranch {
          name: current_branch.clone().into(),
          kind: CommandPaletteBranchKind::Local,
        }),
        _window,
        cx,
      )
    });
    assert!(
      result.is_ok(),
      "delete current branch failure should close palette and rely on notification feedback"
    );

    await_git_page_background_tasks(git_page.clone(), cx).await;
    cx.cx.run_until_parked();
    cx.run_until_parked();

    let notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(notification_count, 1);
  }

  #[gpui::test]
  async fn command_palette_delete_remote_branch_deletes_requested_branch(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let remote = TempBareRepo::init("git-page-cmd-delete-remote-origin");
    let source = TempRepo::init("git-page-cmd-delete-remote-source");
    let clone_dir = TempDir::new("git-page-cmd-delete-remote-clone");

    let _ = commit_text_file(&source.path, Path::new("README.md"), "v1\n", "initial");
    let source_repo = Repository::open(&source.path).expect("open source repo");
    source_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add source origin");
    let base_branch = current_branch_status(&source.path)
      .expect("read source branch status")
      .name;
    push_branch_to_remote(&source.path, &base_branch, "origin");
    create_branch(&source.path, "feature").expect("create source feature branch");
    push_branch_to_remote(&source.path, "feature", "origin");

    let _clone_repo = Repository::clone(
      remote.path.to_str().expect("remote path utf8"),
      &clone_dir.path,
    )
    .expect("clone remote");
    let clone_branch = current_branch_status(&clone_dir.path)
      .expect("read clone branch status")
      .name;

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(clone_dir.path.clone());
      this.branch_status = Some(make_branch_status(&clone_branch, 0, 0, true));
      this.handle_command_palette_action(
        CommandPaletteAction::DeleteBranch(CommandPaletteBranch {
          name: "origin/feature".into(),
          kind: CommandPaletteBranchKind::Remote,
        }),
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(
      Repository::open(&remote.path)
        .expect("open remote")
        .refname_to_id("refs/heads/feature")
        .is_err()
    );
    assert!(
      !list_branches(&clone_dir.path)
        .expect("list clone branches after delete")
        .iter()
        .any(|branch| branch.kind == BranchKind::Remote && branch.name == "origin/feature")
    );
  }

  #[gpui::test]
  async fn command_palette_checkout_detached_detaches_head(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-checkout-detached");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.has_head_commit = true;
      this.branch_status = Some(make_branch_status("main", 0, 0, true));
      seed_repo_branch_state(this, &repo.path, cx);
      let target = head_oid(&repo.path).to_string();
      this.handle_command_palette_action(
        CommandPaletteAction::CheckoutDetached { target },
        window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let status = current_branch_status(&repo.path).expect("read status");
    assert_eq!(status.name, "HEAD");

    let selected_branch = git_page.read_with(cx, |this, _cx| selected_branch_from_dropdown(this));
    assert_eq!(
      selected_branch,
      Some(GitPage::detached_branch_select_value())
    );
  }

  #[gpui::test]
  async fn command_palette_switch_repository_updates_selected_repo_and_header_select(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo_a = TempRepo::init("git-page-cmd-switch-repo-a");
    let repo_b = TempRepo::init("git-page-cmd-switch-repo-b");
    let _ = commit_text_file(&repo_a.path, Path::new("README.md"), "a1\n", "initial");
    let _ = commit_text_file(&repo_b.path, Path::new("README.md"), "b1\n", "initial");

    ConfigStore::persist_recent_repository(&repo_a.path);
    ConfigStore::persist_recent_repository(&repo_b.path);

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo_a.path.clone());
      this.refresh_repo_select(cx);
      this.handle_command_palette_action(
        CommandPaletteAction::SwitchRepository(CommandPaletteRepository {
          path: repo_b.path.to_string_lossy().to_string().into(),
        }),
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (selected_repo, header_contains_repo) = git_page.read_with(cx, |this, _cx| {
      (
        this.selected_repo.clone(),
        this
          .repo_dropdown_items
          .iter()
          .any(|item| item.path == repo_b.path),
      )
    });
    assert_eq!(selected_repo, Some(repo_b.path.clone()));
    assert!(header_contains_repo);
  }

  #[gpui::test]
  async fn command_palette_forget_repository_removes_it_from_dropdown(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo_a = TempRepo::init("git-page-cmd-forget-repo-a");
    let repo_b = TempRepo::init("git-page-cmd-forget-repo-b");
    let _ = commit_text_file(&repo_a.path, Path::new("README.md"), "a1\n", "initial");
    let _ = commit_text_file(&repo_b.path, Path::new("README.md"), "b1\n", "initial");

    ConfigStore::persist_recent_repository(&repo_a.path);
    ConfigStore::persist_recent_repository(&repo_b.path);

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo_a.path.clone());
      this.refresh_repo_select(cx);
      this.handle_command_palette_action(
        CommandPaletteAction::ForgetRepository(CommandPaletteRepository {
          path: repo_b.path.to_string_lossy().to_string().into(),
        }),
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (selected_repo, still_contains_forgotten) = git_page.read_with(cx, |this, _cx| {
      (
        this.selected_repo.clone(),
        this
          .repo_dropdown_items
          .iter()
          .any(|item| item.path == repo_b.path),
      )
    });
    // The forgotten repo should be gone from the dropdown, but the selection is untouched.
    assert_eq!(selected_repo, Some(repo_a.path.clone()));
    assert!(!still_contains_forgotten);

    let persisted: Vec<PathBuf> = ConfigStore::load_recent_repositories()
      .into_iter()
      .map(|r| r.path)
      .collect();
    assert!(!persisted.contains(&repo_b.path));
    assert!(persisted.contains(&repo_a.path));
  }

  #[gpui::test]
  async fn command_palette_forget_selected_repository_switches_to_next_remaining(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo_a = TempRepo::init("git-page-cmd-forget-selected-a");
    let repo_b = TempRepo::init("git-page-cmd-forget-selected-b");
    let _ = commit_text_file(&repo_a.path, Path::new("README.md"), "a1\n", "initial");
    let _ = commit_text_file(&repo_b.path, Path::new("README.md"), "b1\n", "initial");

    ConfigStore::persist_recent_repository(&repo_a.path);
    ConfigStore::persist_recent_repository(&repo_b.path);

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo_b.path.clone());
      this.refresh_repo_select(cx);
      this.handle_command_palette_action(
        CommandPaletteAction::ForgetRepository(CommandPaletteRepository {
          path: repo_b.path.to_string_lossy().to_string().into(),
        }),
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (selected_repo, dropdown_has_b) = git_page.read_with(cx, |this, _cx| {
      (
        this.selected_repo.clone(),
        this
          .repo_dropdown_items
          .iter()
          .any(|item| item.path == repo_b.path),
      )
    });
    assert_eq!(selected_repo, Some(repo_a.path.clone()));
    assert!(!dropdown_has_b);
  }

  #[gpui::test]
  async fn command_palette_forget_last_repository_clears_selection(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-forget-last");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    ConfigStore::persist_recent_repository(&repo.path);

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.refresh_repo_select(cx);
      this.handle_command_palette_action(
        CommandPaletteAction::ForgetRepository(CommandPaletteRepository {
          path: repo.path.to_string_lossy().to_string().into(),
        }),
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (selected_repo, dropdown_items_len) = git_page.read_with(cx, |this, _cx| {
      (this.selected_repo.clone(), this.repo_dropdown_items.len())
    });
    assert_eq!(selected_repo, None);
    assert_eq!(dropdown_items_len, 0);
  }

  #[gpui::test]
  async fn command_palette_switch_repository_returns_error_for_missing_repository(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-switch-repo-missing");
    let missing_repo = repo.path.join("does-not-exist");

    let (git_page, cx) = add_git_page_window_with_root(cx);

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::SwitchRepository(CommandPaletteRepository {
          path: missing_repo.to_string_lossy().to_string().into(),
        }),
        _window,
        cx,
      )
    });

    let error = result.expect_err("switch repository should fail for a missing path");
    assert!(error.as_ref().starts_with("Repository not found:"));
    let selected_repo = git_page.read_with(cx, |this, _| this.selected_repo.clone());
    assert_eq!(selected_repo, Some(repo.path.clone()));
  }

  #[gpui::test]
  async fn command_palette_dialog_ignores_dialog_confirm_action(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-dialog-confirm");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let mut mounted_git_page = None;
    let (_root, cx) = cx.add_window_view(|window, cx| {
      let git_page = cx.new(|cx| GitPage::new_for_test(window, cx));
      mounted_git_page = Some(git_page.clone());
      gpui_component::Root::new(git_page, window, cx)
    });
    let git_page = mounted_git_page.expect("git page");
    cx.executor().allow_parking();

    git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.open_command_palette(window, cx, Some(CommandPaletteInitialScreen::SwitchBranch));
    });
    cx.cx.run_until_parked();
    cx.run_until_parked();

    let dialog_open_before_confirm =
      git_page.update_in(cx, |_this, window, cx| window.has_active_dialog(cx));
    assert!(dialog_open_before_confirm);

    git_page.update_in(cx, |_this, window, cx| {
      window.dispatch_action(
        Box::new(gpui_base::actions::Confirm { secondary: false }),
        cx,
      );
    });
    cx.cx.run_until_parked();
    cx.run_until_parked();

    let dialog_open_after_confirm =
      git_page.update_in(cx, |_this, window, cx| window.has_active_dialog(cx));
    assert!(dialog_open_after_confirm);
  }

  #[gpui::test]
  async fn command_palette_fetch_updates_remote_tracking_refs(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let remote = TempBareRepo::init("git-page-cmd-fetch-origin");
    let source = TempRepo::init("git-page-cmd-fetch-source");
    let clone_dir = TempDir::new("git-page-cmd-fetch-clone");

    let _ = commit_text_file(&source.path, Path::new("README.md"), "v1\n", "initial");
    let source_repo = Repository::open(&source.path).expect("open source repo");
    source_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add source origin");
    let base_branch = current_branch_status(&source.path)
      .expect("read source branch status")
      .name;
    push_branch_to_remote(&source.path, &base_branch, "origin");

    let _clone_repo = Repository::clone(
      remote.path.to_str().expect("remote path utf8"),
      &clone_dir.path,
    )
    .expect("clone remote");
    let tracking_ref = format!("refs/remotes/origin/{base_branch}");
    let before = Repository::open(&clone_dir.path)
      .expect("open clone")
      .refname_to_id(&tracking_ref)
      .expect("read remote tracking ref before fetch");

    let _ = commit_text_file(
      &source.path,
      Path::new("README.md"),
      "v2\n",
      "source update",
    );
    push_branch_to_remote(&source.path, &base_branch, "origin");
    let expected = remote_branch_oid(&remote.path, &base_branch);
    assert_ne!(
      before, expected,
      "expected remote branch to advance after push"
    );

    let clone_repo = Repository::open(&clone_dir.path).expect("open clone");
    clone_repo
      .reference(
        &tracking_ref,
        before,
        true,
        "force stale remote tracking ref",
      )
      .expect("force stale remote tracking ref");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(clone_dir.path.clone());
      this.handle_command_palette_action(CommandPaletteAction::Fetch, window, cx)
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let after = Repository::open(&clone_dir.path)
      .expect("open clone")
      .refname_to_id(&tracking_ref)
      .expect("read remote tracking ref after fetch");
    assert_eq!(after, expected);
  }

  #[gpui::test]
  async fn command_palette_fetch_toggles_loading_state(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let remote = TempBareRepo::init("git-page-cmd-fetch-loading-origin");
    let source = TempRepo::init("git-page-cmd-fetch-loading-source");
    let clone_dir = TempDir::new("git-page-cmd-fetch-loading-clone");

    let _ = commit_text_file(&source.path, Path::new("README.md"), "v1\n", "initial");
    let source_repo = Repository::open(&source.path).expect("open source repo");
    source_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add source origin");
    let base_branch = current_branch_status(&source.path)
      .expect("read source branch status")
      .name;
    push_branch_to_remote(&source.path, &base_branch, "origin");

    let _clone_repo = Repository::clone(
      remote.path.to_str().expect("remote path utf8"),
      &clone_dir.path,
    )
    .expect("clone remote");
    let _ = commit_text_file(
      &source.path,
      Path::new("README.md"),
      "v2\n",
      "source update",
    );
    push_branch_to_remote(&source.path, &base_branch, "origin");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(clone_dir.path.clone());
      this.handle_command_palette_action(CommandPaletteAction::Fetch, window, cx)
    });
    assert!(result.is_ok());
    assert!(git_page.read_with(cx, |this, _| this.fetch_in_progress));

    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(!git_page.read_with(cx, |this, _| this.fetch_in_progress));
  }

  #[gpui::test]
  async fn command_palette_create_branch_from_local_creates_and_switches(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-create-from");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    create_branch(&repo.path, "feature").expect("create feature branch");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let feature_head = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "v2-feature\n",
      "feature change",
    );

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::CreateBranchFrom {
          name: "feature-copy".to_string(),
          base: CommandPaletteBranch {
            name: "feature".into(),
            kind: CommandPaletteBranchKind::Local,
          },
        },
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let status = current_branch_status(&repo.path).expect("read status");
    assert_eq!(status.name, "feature-copy");

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let created = repo_handle
      .find_branch("feature-copy", BranchType::Local)
      .expect("find feature-copy branch");
    assert_eq!(created.get().target(), Some(feature_head));
    assert!(created.upstream().is_err());
  }

  #[gpui::test]
  async fn command_palette_create_branch_from_shows_notification_only_when_branch_exists(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-create-from-existing");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let base_branch = current_branch_status(&repo.path).expect("base status").name;
    create_branch(&repo.path, "feature").expect("create feature branch");
    create_branch(&repo.path, "feature-copy").expect("create existing target branch");

    let mut mounted_git_page = None;
    let (root, cx) = cx.add_window_view(|window, cx| {
      let git_page = cx.new(|cx| GitPage::new_for_test(window, cx));
      mounted_git_page = Some(git_page.clone());
      gpui_component::Root::new(git_page, window, cx)
    });
    let git_page = mounted_git_page.expect("git page");
    cx.executor().allow_parking();

    let initial_notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(initial_notification_count, 0);

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::CreateBranchFrom {
          name: "feature-copy".to_string(),
          base: CommandPaletteBranch {
            name: "feature".into(),
            kind: CommandPaletteBranchKind::Local,
          },
        },
        _window,
        cx,
      )
    });
    assert!(
      result.is_ok(),
      "create branch from failure should close palette and rely on notification feedback"
    );

    await_git_page_background_tasks(git_page.clone(), cx).await;
    cx.cx.run_until_parked();
    cx.run_until_parked();

    let notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(notification_count, 1);
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after failed create from")
        .name,
      base_branch
    );
    let feature_copy_count = list_branches(&repo.path)
      .expect("list branches after failed create from")
      .iter()
      .filter(|branch| branch.kind == BranchKind::Local && branch.name == "feature-copy")
      .count();
    assert_eq!(feature_copy_count, 1);
  }

  #[gpui::test]
  async fn command_palette_switch_remote_branch_creates_local_branch_with_upstream(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let remote = TempBareRepo::init("git-page-cmd-switch-remote-origin");
    let source = TempRepo::init("git-page-cmd-switch-remote-source");
    let clone_dir = TempDir::new("git-page-cmd-switch-remote-clone");

    let _ = commit_text_file(&source.path, Path::new("README.md"), "v1\n", "initial");
    let source_repo = Repository::open(&source.path).expect("open source repo");
    source_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add source origin");

    let base_branch = current_branch_status(&source.path)
      .expect("source branch status")
      .name;
    push_branch_to_remote(&source.path, &base_branch, "origin");
    set_remote_head(&remote.path, &base_branch);

    create_branch(&source.path, "feature").expect("create source feature branch");
    switch_branch(
      &source.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch source to feature");
    let _ = commit_text_file(
      &source.path,
      Path::new("README.md"),
      "v2-feature\n",
      "feature change",
    );
    push_branch_to_remote(&source.path, "feature", "origin");

    let _clone_repo = Repository::clone(
      remote.path.to_str().expect("remote path utf8"),
      &clone_dir.path,
    )
    .expect("clone remote");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();
    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(clone_dir.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::SwitchBranch(CommandPaletteBranch {
          name: "origin/feature".into(),
          kind: CommandPaletteBranchKind::Remote,
        }),
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let status = current_branch_status(&clone_dir.path).expect("status after remote switch");
    assert_eq!(status.name, "feature");
    assert!(status.has_upstream);

    let clone_repo = Repository::open(&clone_dir.path).expect("open clone repo");
    let local_feature = clone_repo
      .find_branch("feature", BranchType::Local)
      .expect("find local feature branch");
    let upstream = local_feature
      .upstream()
      .expect("feature upstream")
      .name()
      .expect("upstream name")
      .expect("non-empty upstream")
      .to_string();
    assert_eq!(upstream, "origin/feature");
  }

  #[gpui::test]
  async fn command_palette_switch_remote_branch_shows_notification_only_when_remote_branch_missing(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-switch-remote-missing");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let base_branch = current_branch_status(&repo.path).expect("base status").name;

    let mut mounted_git_page = None;
    let (root, cx) = cx.add_window_view(|window, cx| {
      let git_page = cx.new(|cx| GitPage::new_for_test(window, cx));
      mounted_git_page = Some(git_page.clone());
      gpui_component::Root::new(git_page, window, cx)
    });
    let git_page = mounted_git_page.expect("git page");
    cx.executor().allow_parking();

    let initial_notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(initial_notification_count, 0);

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::SwitchBranch(CommandPaletteBranch {
          name: "origin/missing".into(),
          kind: CommandPaletteBranchKind::Remote,
        }),
        _window,
        cx,
      )
    });
    assert!(
      result.is_ok(),
      "switch branch failure should close palette and rely on notification feedback"
    );

    await_git_page_background_tasks(git_page.clone(), cx).await;
    cx.cx.run_until_parked();
    cx.run_until_parked();

    let notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(notification_count, 1);
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after failed remote switch")
        .name,
      base_branch
    );
  }

  #[gpui::test]
  async fn command_palette_create_branch_from_remote_hides_pr_until_unique_commit(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let remote = TempBareRepo::init("git-page-cmd-create-from-remote-origin");
    let source = TempRepo::init("git-page-cmd-create-from-remote-source");
    let clone_dir = TempDir::new("git-page-cmd-create-from-remote-clone");

    let _ = commit_text_file(&source.path, Path::new("README.md"), "v1\n", "initial");
    let source_repo = Repository::open(&source.path).expect("open source repo");
    source_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add source origin");

    let base_branch = current_branch_status(&source.path)
      .expect("source branch status")
      .name;
    push_branch_to_remote(&source.path, &base_branch, "origin");
    set_remote_head(&remote.path, &base_branch);

    create_branch(&source.path, "feature").expect("create source feature branch");
    push_branch_to_remote(&source.path, "feature", "origin");

    let _clone_repo = Repository::clone(
      remote.path.to_str().expect("remote path utf8"),
      &clone_dir.path,
    )
    .expect("clone remote");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();
    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(clone_dir.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::CreateBranchFrom {
          name: "my-feature".to_string(),
          base: CommandPaletteBranch {
            name: "origin/feature".into(),
            kind: CommandPaletteBranchKind::Remote,
          },
        },
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let status = current_branch_status(&clone_dir.path).expect("status after create from remote");
    assert_eq!(status.name, "my-feature");
    assert!(!status.has_upstream);
    assert!(!branch_has_unpublished_commits(&clone_dir.path).expect("unpublished commit state"));
    assert_eq!(
      GitPage::push_action_label(Some(&status), true),
      "Push (Publish branch)"
    );
    let (can_push, has_unpublished_branch_commits) = git_page.read_with(cx, |this, _| {
      (this.can_push, this.has_unpublished_branch_commits)
    });
    assert!(can_push);
    assert!(!has_unpublished_branch_commits);

    let clone_repo = Repository::open(&clone_dir.path).expect("open clone repo");
    let created = clone_repo
      .find_branch("my-feature", BranchType::Local)
      .expect("find created branch");
    assert!(created.upstream().is_err());

    let _ = commit_text_file(
      &clone_dir.path,
      Path::new("README.md"),
      "v2-feature\n",
      "feature change",
    );
    let reload_task = git_page.update_in(cx, |this, _window, cx| {
      this.reload_status(cx);
      this.status_task.take().expect("reload status task")
    });
    reload_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(branch_has_unpublished_commits(&clone_dir.path).expect("unpublished commit state"));
    let (can_push, has_unpublished_branch_commits) = git_page.read_with(cx, |this, _| {
      (this.can_push, this.has_unpublished_branch_commits)
    });
    assert!(can_push);
    assert!(has_unpublished_branch_commits);
  }

  #[test]
  fn command_palette_create_branch_from_remote_uses_notification_feedback() {
    assert_eq!(
      GitPage::command_palette_error_notification_title(&CommandPaletteAction::CreateBranchFrom {
        name: "my-feature".to_string(),
        base: CommandPaletteBranch {
          name: "origin/missing".into(),
          kind: CommandPaletteBranchKind::Remote,
        },
      }),
      Some("Create branch failed")
    );
  }

  #[test]
  fn command_palette_delete_branch_uses_notification_feedback() {
    assert_eq!(
      GitPage::command_palette_error_notification_title(&CommandPaletteAction::DeleteBranch(
        CommandPaletteBranch {
          name: "feature".into(),
          kind: CommandPaletteBranchKind::Local,
        }
      )),
      Some("Delete branch failed")
    );
  }

  #[gpui::test]
  async fn command_palette_commit_stages_all_when_needed(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-commit");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("write unstaged change");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();
    let result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.status_entries = list_repo_status(&repo.path).expect("list status before commit");
      this.commit_input.update(cx, |input, cx| {
        input.set_value("feat: command palette commit", window, cx)
      });
      this.handle_command_palette_action(CommandPaletteAction::Commit, window, cx)
    });
    assert!(result.is_ok());
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(
      list_repo_status(&repo.path)
        .expect("list status after commit")
        .is_empty()
    );
    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let head = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read head commit");
    assert_eq!(head.summary(), Some("feat: command palette commit"));
  }

  #[gpui::test]
  async fn command_palette_commit_returns_error_when_command_is_disabled(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-commit-disabled");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("write unstaged change");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    let result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.status_entries = list_repo_status(&repo.path).expect("list status before commit");
      this
        .commit_input
        .update(cx, |input, cx| input.set_value("   ", window, cx));
      this.handle_command_palette_action(CommandPaletteAction::Commit, window, cx)
    });
    let error = result.expect_err("disabled commit should return an error");
    assert_eq!(error.as_ref(), "Commit command is currently disabled.");
  }

  #[gpui::test]
  async fn command_palette_push_pushes_to_remote_when_allowed(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let source = TempRepo::init("git-page-cmd-push-success-source");
    let remote = TempBareRepo::init("git-page-cmd-push-success-remote");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&source.path, rel_path, "v1\n", "initial");

    let source_repo = Repository::open(&source.path).expect("open source");
    source_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add origin remote");
    let branch_name = current_branch_status(&source.path)
      .expect("source branch status")
      .name;
    push_branch_to_remote(&source.path, &branch_name, "origin");
    set_upstream(&source.path, &branch_name, &format!("origin/{branch_name}"));
    set_remote_head(&remote.path, &branch_name);

    let _ = commit_text_file(&source.path, rel_path, "v2-source\n", "source change");
    let expected_head = head_oid(&source.path);

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let push_result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(source.path.clone());
      this.can_push = true;
      this.handle_command_palette_action(CommandPaletteAction::Push, window, cx)
    });
    assert!(push_result.is_ok());
    assert!(git_page.read_with(cx, |this, _| this.push_pull_in_progress));

    let push_task = git_page.update_in(cx, |this, _window, _| this.status_task.take());
    push_task.expect("push task should exist").await;
    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    assert_eq!(remote_branch_oid(&remote.path, &branch_name), expected_head);
    assert!(!git_page.read_with(cx, |this, _| this.push_pull_in_progress));
  }

  #[gpui::test]
  async fn command_palette_force_push_force_pushes_when_allowed(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let source = TempRepo::init("git-page-cmd-force-push-source");
    let remote = TempBareRepo::init("git-page-cmd-force-push-remote");
    let peer = TempDir::new("git-page-cmd-force-push-peer");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&source.path, rel_path, "v1\n", "initial");

    let source_repo = Repository::open(&source.path).expect("open source");
    source_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add origin remote");
    let branch_name = current_branch_status(&source.path)
      .expect("source branch status")
      .name;
    push_branch_to_remote(&source.path, &branch_name, "origin");
    set_upstream(&source.path, &branch_name, &format!("origin/{branch_name}"));
    set_remote_head(&remote.path, &branch_name);

    let _ = Repository::clone(remote.path.to_str().expect("remote path utf8"), &peer.path)
      .expect("clone remote into peer");

    let _ = commit_text_file(&source.path, rel_path, "v2-source\n", "source change");
    let expected_head = head_oid(&source.path);

    let _ = commit_text_file(&peer.path, rel_path, "v2-peer\n", "peer change");
    push_branch_to_remote(&peer.path, &branch_name, "origin");

    let non_force = push(&source.path, false).err();
    assert!(non_force.is_some(), "non-force push should fail");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let push_result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(source.path.clone());
      this.can_force_push = true;
      this.handle_command_palette_action(CommandPaletteAction::ForcePush, window, cx)
    });
    assert!(push_result.is_ok());
    assert!(git_page.read_with(cx, |this, _| this.push_pull_in_progress));

    let force_task = git_page.update_in(cx, |this, _window, _| this.status_task.take());
    force_task.expect("force push task should exist").await;
    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    assert_eq!(remote_branch_oid(&remote.path, &branch_name), expected_head);
    assert!(!git_page.read_with(cx, |this, _| this.push_pull_in_progress));
  }

  #[gpui::test]
  async fn command_palette_undo_last_commit_moves_head_when_allowed(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-undo-success");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "first");
    let _ = commit_text_file(&repo.path, rel_path, "v2\n", "second");

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let expected_parent = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("head before undo")
      .parent(0)
      .expect("parent")
      .id();

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let undo_result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.can_undo_last_commit = true;
      this.handle_command_palette_action(CommandPaletteAction::UndoLastCommit, window, cx)
    });
    assert!(undo_result.is_ok());

    let undo_task = git_page.update_in(cx, |this, _window, _| this.status_task.take());
    undo_task.expect("undo task should exist").await;
    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    let head_after = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("head after undo")
      .id();
    assert_eq!(head_after, expected_parent);
  }

  #[gpui::test]
  async fn command_palette_amend_updates_head_message_when_allowed(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-amend-success");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let amend_result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.has_head_commit = true;
      this
        .commit_input
        .update(cx, |input, cx| input.set_value("feat: amended", window, cx));
      this.handle_command_palette_action(CommandPaletteAction::Amend, window, cx)
    });
    assert!(amend_result.is_ok());

    let amend_task = git_page.update_in(cx, |this, _window, _| this.status_task.take());
    amend_task.expect("amend task should exist").await;
    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let head = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read head after amend");
    assert_eq!(head.summary(), Some("feat: amended"));
  }

  #[gpui::test]
  fn command_palette_commit_menu_actions_return_error_when_disabled(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-commit-menu-disabled");
    let (git_page, cx) = add_git_page_window_with_root(cx);

    for (action, expected_error) in [
      (
        CommandPaletteAction::Push,
        "Push command is currently disabled.",
      ),
      (
        CommandPaletteAction::ForcePush,
        "Force push command is currently disabled.",
      ),
      (
        CommandPaletteAction::UndoLastCommit,
        "Undo last commit command is currently disabled.",
      ),
      (
        CommandPaletteAction::Amend,
        "Amend command is currently disabled.",
      ),
    ] {
      let result = git_page.update_in(cx, |this, window, cx| {
        this.selected_repo = Some(repo.path.clone());
        this.rebase_in_progress = true;
        this.can_push = true;
        this.can_force_push = true;
        this.can_undo_last_commit = true;
        this.has_head_commit = true;
        this.handle_command_palette_action(action.clone(), window, cx)
      });
      let error = result.expect_err("action should be disabled during rebase flow");
      assert_eq!(error.as_ref(), expected_error);
    }
  }

  #[gpui::test]
  fn command_palette_selected_file_stage_toggle_returns_error_when_disabled(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-selected-file-toggle-disabled");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("modify tracked file");

    let (git_page, cx) = add_git_page_window_with_root(cx);

    let stage_without_selection = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.selected_file = None;
      this.status_entries = list_repo_status(&repo.path).expect("list status");
      this.handle_command_palette_action(CommandPaletteAction::StageSelectedFile, window, cx)
    });
    assert_eq!(
      stage_without_selection
        .expect_err("stage selected file should be disabled without selection")
        .as_ref(),
      "Stage file command is currently disabled."
    );

    let unstage_without_staged_file = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.selected_file = Some(rel_path.to_path_buf());
      this.status_entries = list_repo_status(&repo.path).expect("list status");
      this.handle_command_palette_action(CommandPaletteAction::UnstageSelectedFile, window, cx)
    });
    assert_eq!(
      unstage_without_staged_file
        .expect_err("unstage selected file should be disabled when file is unstaged")
        .as_ref(),
      "Unstage file command is currently disabled."
    );
  }

  #[gpui::test]
  async fn command_palette_stage_selected_file_stages_selected_entry(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-stage-selected-file");
    let first = Path::new("a.txt");
    let second = Path::new("b.txt");
    let _ = commit_text_file(&repo.path, first, "a1\n", "first");
    let _ = commit_text_file(&repo.path, second, "b1\n", "second");
    std::fs::write(repo.path.join(first), "a2\n").expect("modify first");
    std::fs::write(repo.path.join(second), "b2\n").expect("modify second");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.status_entries = list_repo_status(&repo.path).expect("list status");
      this.selected_file = Some(first.to_path_buf());
      this.handle_command_palette_action(CommandPaletteAction::StageSelectedFile, window, cx)
    });
    assert!(result.is_ok());

    let stage_task = git_page.update_in(cx, |this, _window, _| this.status_task.take());
    stage_task.expect("stage selected file task").await;
    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    let entries = list_repo_status(&repo.path).expect("list status after stage selected file");
    let first_entry = entries
      .iter()
      .find(|entry| entry.path == first)
      .expect("first entry");
    let second_entry = entries
      .iter()
      .find(|entry| entry.path == second)
      .expect("second entry");
    assert_eq!(first_entry.stage, RepoStage::Staged);
    assert_eq!(second_entry.stage, RepoStage::Unstaged);
  }

  #[gpui::test]
  async fn command_palette_unstage_selected_file_unstages_selected_entry(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-unstage-selected-file");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("modify file");
    stage_file(&repo.path, rel_path).expect("stage file before command");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.status_entries = list_repo_status(&repo.path).expect("list status");
      this.selected_file = Some(rel_path.to_path_buf());
      this.handle_command_palette_action(CommandPaletteAction::UnstageSelectedFile, window, cx)
    });
    assert!(result.is_ok());

    let unstage_task = git_page.update_in(cx, |this, _window, _| this.status_task.take());
    unstage_task.expect("unstage selected file task").await;
    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    let entries = list_repo_status(&repo.path).expect("list status after unstage selected file");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, rel_path);
    assert_eq!(entries[0].stage, RepoStage::Unstaged);
  }

  #[gpui::test]
  async fn command_palette_accept_all_current_conflicts_resolves_editor_markers(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-accept-all-current");
    let rel_path = Path::new("README.md");
    let conflict_text = "before\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> branch\nafter\n";
    std::fs::write(repo.path.join(rel_path), conflict_text).expect("write conflict markers");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.status_entries = vec![RepoStatusEntry {
        path: rel_path.to_path_buf(),
        old_path: None,
        status: RepoStatusKind::Conflicted,
        stage: RepoStage::Unstaged,
      }];
      this.open_file(rel_path.to_path_buf(), cx);
    });
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let can_show_before = git_page.read_with(cx, |this, cx| {
      this.should_show_accept_all_conflicts_palette_commands(cx)
    });
    assert!(
      can_show_before,
      "command should be visible for conflicted file"
    );

    let result = git_page.update_in(cx, |this, window, cx| {
      this.handle_command_palette_action(
        CommandPaletteAction::AcceptAllCurrentConflicts,
        window,
        cx,
      )
    });
    assert!(result.is_ok());

    let (contents, can_show_after) = git_page.read_with(cx, |this, cx| {
      let contents = {
        let editor = this.editor.as_ref().expect("editor should exist").read(cx);
        let document = editor.document().read(cx);
        document.slice_to_string(0..document.len())
      };
      (
        contents,
        this.should_show_accept_all_conflicts_palette_commands(cx),
      )
    });
    assert_eq!(contents, "before\nours\nafter\n");
    assert!(
      !can_show_after,
      "commands should disappear once all conflict markers are resolved"
    );
  }

  #[gpui::test]
  async fn command_palette_accept_all_incoming_conflicts_resolves_editor_markers(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-accept-all-incoming");
    let rel_path = Path::new("README.md");
    let conflict_text = "before\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> branch\nafter\n";
    std::fs::write(repo.path.join(rel_path), conflict_text).expect("write conflict markers");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.status_entries = vec![RepoStatusEntry {
        path: rel_path.to_path_buf(),
        old_path: None,
        status: RepoStatusKind::Conflicted,
        stage: RepoStage::Unstaged,
      }];
      this.open_file(rel_path.to_path_buf(), cx);
    });
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let can_show_before = git_page.read_with(cx, |this, cx| {
      this.should_show_accept_all_conflicts_palette_commands(cx)
    });
    assert!(
      can_show_before,
      "command should be visible for conflicted file"
    );

    let result = git_page.update_in(cx, |this, window, cx| {
      this.handle_command_palette_action(
        CommandPaletteAction::AcceptAllIncomingConflicts,
        window,
        cx,
      )
    });
    assert!(result.is_ok());

    let (contents, can_show_after) = git_page.read_with(cx, |this, cx| {
      let contents = {
        let editor = this.editor.as_ref().expect("editor should exist").read(cx);
        let document = editor.document().read(cx);
        document.slice_to_string(0..document.len())
      };
      (
        contents,
        this.should_show_accept_all_conflicts_palette_commands(cx),
      )
    });
    assert_eq!(contents, "before\ntheirs\nafter\n");
    assert!(
      !can_show_after,
      "commands should disappear once all conflict markers are resolved"
    );
  }

  #[gpui::test]
  async fn command_palette_merge_branch_fast_forwards_current_branch(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-merge");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let base_branch = current_branch_status(&repo.path).expect("base status").name;
    create_branch(&repo.path, "feature").expect("create feature branch");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let feature_head = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "v2-feature\n",
      "feature change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base");
    force_checkout_head(&repo.path);

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::MergeBranch {
          name: CommandPaletteBranch {
            name: "feature".into(),
            kind: CommandPaletteBranchKind::Local,
          },
        },
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let head = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read head");
    assert_eq!(head.id(), feature_head);
    assert_eq!(
      std::fs::read_to_string(repo.path.join("README.md")).expect("read merged file"),
      "v2-feature\n"
    );
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after merge")
        .name,
      base_branch
    );
  }

  #[gpui::test]
  async fn command_palette_rebase_branch_fast_forwards_current_branch(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-rebase");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let base_branch = current_branch_status(&repo.path).expect("base status").name;
    create_branch(&repo.path, "feature").expect("create feature branch");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let feature_head = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "v2-feature\n",
      "feature change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base");
    force_checkout_head(&repo.path);

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::RebaseBranch {
          name: CommandPaletteBranch {
            name: "feature".into(),
            kind: CommandPaletteBranchKind::Local,
          },
        },
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let head = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read head");
    assert_eq!(head.id(), feature_head);
    assert_eq!(
      std::fs::read_to_string(repo.path.join("README.md")).expect("read rebased file"),
      "v2-feature\n"
    );
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after rebase")
        .name,
      base_branch
    );
  }

  #[gpui::test]
  async fn command_palette_rebase_branch_with_dirty_worktree_shows_notification_only(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-rebase-dirty-worktree");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let base_branch = current_branch_status(&repo.path).expect("base status").name;
    create_branch(&repo.path, "feature").expect("create feature branch");
    std::fs::write(repo.path.join("README.md"), "dirty change\n")
      .expect("write unstaged change before rebase");

    let mut mounted_git_page = None;
    let (root, cx) = cx.add_window_view(|window, cx| {
      let git_page = cx.new(|cx| GitPage::new_for_test(window, cx));
      mounted_git_page = Some(git_page.clone());
      gpui_component::Root::new(git_page, window, cx)
    });
    let git_page = mounted_git_page.expect("git page");
    cx.executor().allow_parking();

    let initial_notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(initial_notification_count, 0);

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::RebaseBranch {
          name: CommandPaletteBranch {
            name: "feature".into(),
            kind: CommandPaletteBranchKind::Local,
          },
        },
        _window,
        cx,
      )
    });
    assert!(
      result.is_ok(),
      "dirty worktree rebase failure should close palette and rely on notification feedback"
    );

    await_git_page_background_tasks(git_page.clone(), cx).await;
    cx.cx.run_until_parked();
    cx.run_until_parked();

    let notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(notification_count, 1);
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after failed rebase")
        .name,
      base_branch
    );
    assert!(
      !is_rebase_in_progress(&repo.path).expect("read rebase state after failed rebase"),
      "rebase should not start when the worktree is dirty"
    );
  }

  #[gpui::test]
  async fn command_palette_cherry_pick_applies_multiple_commits(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-cherry-pick");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let base_branch = current_branch_status(&repo.path).expect("base status").name;
    create_branch(&repo.path, "feature").expect("create feature branch");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");

    let first = commit_text_file(&repo.path, Path::new("README.md"), "v2\n", "feature 1");
    let second = commit_text_file(&repo.path, Path::new("extra.txt"), "extra\n", "feature 2");

    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base");
    force_checkout_head(&repo.path);

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();
    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::CherryPick {
          commit_hashes: vec![first.to_string(), second.to_string()],
        },
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let head = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read head");
    assert_eq!(head.message().unwrap_or_default(), "feature 2");
    let parent = head.parent(0).expect("head parent");
    assert_eq!(parent.message().unwrap_or_default(), "feature 1");
    assert_eq!(
      std::fs::read_to_string(repo.path.join("README.md")).expect("read cherry-picked README"),
      "v2\n"
    );
    assert_eq!(
      std::fs::read_to_string(repo.path.join("extra.txt")).expect("read cherry-picked extra file"),
      "extra\n"
    );
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after cherry-pick")
        .name,
      base_branch
    );
  }

  #[test]
  fn command_palette_cherry_pick_uses_notification_feedback() {
    assert_eq!(
      GitPage::command_palette_error_notification_title(&CommandPaletteAction::CherryPick {
        commit_hashes: vec!["deadbeef".to_string()],
      }),
      Some("Cherry-pick failed")
    );
  }

  #[gpui::test]
  async fn command_palette_stash_and_apply_restore_tracked_changes(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-stash-apply");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("write tracked change");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();
    let stash_result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::Stash {
          include_untracked: false,
          message: None,
        },
        window,
        cx,
      )
    });
    assert!(stash_result.is_ok());
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert_eq!(
      std::fs::read_to_string(repo.path.join(rel_path)).expect("read file after stash"),
      "v1\n"
    );
    let stash = list_stashes(&repo.path)
      .expect("list stashes after stash")
      .into_iter()
      .next()
      .expect("stash entry exists");

    let apply_result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::ApplyStash(CommandPaletteStash {
          index: stash.index,
          name: stash.name.clone().into(),
          oid: stash.oid.clone().into(),
        }),
        window,
        cx,
      )
    });
    assert!(apply_result.is_ok());
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert_eq!(
      std::fs::read_to_string(repo.path.join(rel_path)).expect("read file after apply stash"),
      "v2\n"
    );
    assert_eq!(
      list_stashes(&repo.path)
        .expect("list stashes after apply")
        .len(),
      1
    );
  }

  #[gpui::test]
  async fn command_palette_stash_with_untracked_and_pop_restores_untracked_file(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-stash-pop-untracked");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let rel_path = Path::new("notes.txt");
    std::fs::write(repo.path.join(rel_path), "notes\n").expect("write untracked file");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();
    let stash_result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::Stash {
          include_untracked: true,
          message: None,
        },
        window,
        cx,
      )
    });
    assert!(stash_result.is_ok());
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(
      !repo.path.join(rel_path).exists(),
      "untracked file should be removed after stash"
    );
    let stash = list_stashes(&repo.path)
      .expect("list stashes")
      .into_iter()
      .next()
      .expect("stash entry exists");

    let pop_result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::PopStash(CommandPaletteStash {
          index: stash.index,
          name: stash.name.clone().into(),
          oid: stash.oid.clone().into(),
        }),
        window,
        cx,
      )
    });
    assert!(pop_result.is_ok());
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert_eq!(
      std::fs::read_to_string(repo.path.join(rel_path)).expect("read restored untracked file"),
      "notes\n"
    );
    assert!(
      list_stashes(&repo.path)
        .expect("list stashes after pop")
        .is_empty()
    );
  }

  #[gpui::test]
  async fn command_palette_drop_stash_removes_entry(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-stash-drop");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("write tracked change");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();
    let stash_result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::Stash {
          include_untracked: false,
          message: None,
        },
        window,
        cx,
      )
    });
    assert!(stash_result.is_ok());
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let stash = list_stashes(&repo.path)
      .expect("list stashes")
      .into_iter()
      .next()
      .expect("stash entry exists");

    let drop_result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::DropStash(CommandPaletteStash {
          index: stash.index,
          name: stash.name.clone().into(),
          oid: stash.oid.clone().into(),
        }),
        window,
        cx,
      )
    });
    assert!(drop_result.is_ok());
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(
      list_stashes(&repo.path)
        .expect("list stashes after drop")
        .is_empty()
    );
    assert_eq!(
      std::fs::read_to_string(repo.path.join(rel_path)).expect("read file after drop stash"),
      "v1\n"
    );
  }

  #[gpui::test]
  async fn command_palette_branch_actions_require_selected_repo(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (git_page, cx) = add_git_page_window_with_root(cx);

    let actions = vec![
      CommandPaletteAction::CheckoutDetached {
        target: "deadbeef".to_string(),
      },
      CommandPaletteAction::Commit,
      CommandPaletteAction::ContinueRebase,
      CommandPaletteAction::SkipRebase,
      CommandPaletteAction::Push,
      CommandPaletteAction::ForcePush,
      CommandPaletteAction::UndoLastCommit,
      CommandPaletteAction::Amend,
      CommandPaletteAction::AcceptAllCurrentConflicts,
      CommandPaletteAction::AcceptAllIncomingConflicts,
      CommandPaletteAction::SwitchBranch(CommandPaletteBranch {
        name: "feature".into(),
        kind: CommandPaletteBranchKind::Local,
      }),
      CommandPaletteAction::CreateBranch {
        name: "feature".to_string(),
      },
      CommandPaletteAction::CreateBranchFrom {
        name: "feature-copy".to_string(),
        base: CommandPaletteBranch {
          name: "feature".into(),
          kind: CommandPaletteBranchKind::Local,
        },
      },
      CommandPaletteAction::DeleteBranch(CommandPaletteBranch {
        name: "feature".into(),
        kind: CommandPaletteBranchKind::Local,
      }),
      CommandPaletteAction::MergeBranch {
        name: CommandPaletteBranch {
          name: "feature".into(),
          kind: CommandPaletteBranchKind::Local,
        },
      },
      CommandPaletteAction::AbortMerge,
      CommandPaletteAction::RebaseBranch {
        name: CommandPaletteBranch {
          name: "feature".into(),
          kind: CommandPaletteBranchKind::Local,
        },
      },
      CommandPaletteAction::InteractiveRebaseBranch {
        name: CommandPaletteBranch {
          name: "feature".into(),
          kind: CommandPaletteBranchKind::Local,
        },
      },
      CommandPaletteAction::InteractiveRebaseEditBranch {
        name: CommandPaletteBranch {
          name: "feature".into(),
          kind: CommandPaletteBranchKind::Local,
        },
      },
      CommandPaletteAction::InteractiveRebaseHeadCount { count: 3 },
      CommandPaletteAction::AbortRebase,
      CommandPaletteAction::StageAll,
      CommandPaletteAction::UnstageAll,
      CommandPaletteAction::StageSelectedFile,
      CommandPaletteAction::UnstageSelectedFile,
      CommandPaletteAction::Fetch,
      CommandPaletteAction::Stash {
        include_untracked: false,
        message: None,
      },
      CommandPaletteAction::ApplyStash(CommandPaletteStash {
        index: 0,
        name: "stash@{0}".into(),
        oid: "deadbeef".into(),
      }),
      CommandPaletteAction::DropStash(CommandPaletteStash {
        index: 0,
        name: "stash@{0}".into(),
        oid: "deadbeef".into(),
      }),
      CommandPaletteAction::PopStash(CommandPaletteStash {
        index: 0,
        name: "stash@{0}".into(),
        oid: "deadbeef".into(),
      }),
      CommandPaletteAction::CherryPick {
        commit_hashes: vec!["deadbeef".to_string()],
      },
    ];

    for action in actions {
      let result = git_page.update_in(cx, |this, _window, cx| {
        this.selected_repo = None;
        this.handle_command_palette_action(action.clone(), _window, cx)
      });
      let error = result.expect_err("action should fail without selected repo");
      assert_eq!(error.as_ref(), "No repository selected.");
    }
  }

  #[gpui::test]
  fn command_palette_includes_delete_branch_root_command_when_available(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-command-palette-delete-available");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "initial\n", "initial");
    create_branch(&repo.path, "feature").expect("create feature branch");
    let current_branch = current_branch_status(&repo.path)
      .expect("read current branch")
      .name;

    let (git_page, cx) = add_git_page_window_with_root(cx);

    let (command_ids, delete_branches) = git_page.update(cx, |this, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.branch_status = Some(make_branch_status(&current_branch, 0, 0, true));
      let contents = this.build_command_palette_contents(1, cx);
      (
        contents
          .commands
          .into_iter()
          .map(|command| command.id)
          .collect::<Vec<_>>(),
        contents.delete_branches,
      )
    });

    assert!(command_ids.contains(&CommandPaletteCommandId::DeleteBranch));
    assert_eq!(
      delete_branches,
      vec![CommandPaletteBranch {
        name: "feature".into(),
        kind: CommandPaletteBranchKind::Local,
      }]
    );
  }

  #[test]
  fn command_palette_rebase_branches_exclude_current_branch_and_prioritize_base_branches() {
    let branches = vec![
      BranchRef {
        name: "feature".into(),
        kind: BranchKind::Local,
      },
      BranchRef {
        name: "topic".into(),
        kind: BranchKind::Local,
      },
      BranchRef {
        name: "main".into(),
        kind: BranchKind::Local,
      },
      BranchRef {
        name: "origin/main".into(),
        kind: BranchKind::Remote,
      },
      BranchRef {
        name: "origin/feature".into(),
        kind: BranchKind::Remote,
      },
    ];

    let rebase_branches = GitPage::command_palette_rebase_branches(
      &branches,
      Some("feature"),
      Some(&BranchRef {
        name: "origin/feature".into(),
        kind: BranchKind::Remote,
      }),
      Some(&BranchRef {
        name: "origin/main".into(),
        kind: BranchKind::Remote,
      }),
    );

    assert_eq!(
      rebase_branches,
      vec![
        CommandPaletteBranch {
          name: "origin/feature".into(),
          kind: CommandPaletteBranchKind::Remote,
        },
        CommandPaletteBranch {
          name: "origin/main".into(),
          kind: CommandPaletteBranchKind::Remote,
        },
        CommandPaletteBranch {
          name: "main".into(),
          kind: CommandPaletteBranchKind::Local,
        },
        CommandPaletteBranch {
          name: "topic".into(),
          kind: CommandPaletteBranchKind::Local,
        },
      ]
    );
  }

  #[gpui::test]
  fn command_palette_rebase_targets_do_not_include_current_local_branch(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-command-palette-rebase-targets");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "initial\n", "initial");
    let current_branch = current_branch_status(&repo.path)
      .expect("read current branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");

    let (git_page, cx) = add_git_page_window_with_root(cx);

    let (branches, rebase_branches) = git_page.update(cx, |this, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.branch_status = Some(make_branch_status(&current_branch, 0, 0, true));
      let contents = this.build_command_palette_contents(1, cx);
      (contents.branches, contents.rebase_branches)
    });

    assert!(branches.contains(&CommandPaletteBranch {
      name: current_branch.clone().into(),
      kind: CommandPaletteBranchKind::Local,
    }));
    assert!(!rebase_branches.contains(&CommandPaletteBranch {
      name: current_branch.into(),
      kind: CommandPaletteBranchKind::Local,
    }));
    assert!(rebase_branches.contains(&CommandPaletteBranch {
      name: "feature".into(),
      kind: CommandPaletteBranchKind::Local,
    }));
  }

  #[gpui::test]
  fn git_file_search_entries_group_changed_files_before_unchanged_project_files(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-file-search-groups");
    let _ = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "initial\n",
      "add readme",
    );
    std::fs::create_dir_all(repo.path.join("src")).expect("create src dir");
    let _ = commit_text_file(
      &repo.path,
      Path::new("src/main.rs"),
      "fn main() {}\n",
      "add main",
    );
    std::fs::write(repo.path.join("README.md"), "updated\n").expect("modify readme");

    let (git_page, cx) = add_git_page_window_with_root(cx);

    let entries = git_page.update(cx, |this, _cx| {
      this.selected_repo = Some(repo.path.clone());
      this.status_entries = list_repo_status(&repo.path).expect("list status");
      this.git_file_search_entries()
    });

    let labels = entries
      .iter()
      .map(|entry| {
        (
          entry.group.as_ref().map(|group| group.to_string()),
          entry.label.to_string(),
        )
      })
      .collect::<Vec<_>>();

    assert_eq!(
      labels,
      vec![
        (Some("Changed".to_string()), "README.md".to_string()),
        (Some("Unchanged".to_string()), "src/main.rs".to_string()),
      ]
    );
  }

  #[gpui::test]
  fn command_palette_includes_remote_branches_in_delete_candidates(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let remote = TempBareRepo::init("git-page-command-palette-delete-remote-origin");
    let source = TempRepo::init("git-page-command-palette-delete-remote-source");
    let clone_dir = TempDir::new("git-page-command-palette-delete-remote-clone");

    let _ = commit_text_file(&source.path, Path::new("README.md"), "initial\n", "initial");
    let source_repo = Repository::open(&source.path).expect("open source repo");
    source_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add source origin");
    let base_branch = current_branch_status(&source.path)
      .expect("read source branch status")
      .name;
    push_branch_to_remote(&source.path, &base_branch, "origin");
    create_branch(&source.path, "feature").expect("create source feature branch");
    push_branch_to_remote(&source.path, "feature", "origin");

    let _clone_repo = Repository::clone(
      remote.path.to_str().expect("remote path utf8"),
      &clone_dir.path,
    )
    .expect("clone remote");
    let clone_branch = current_branch_status(&clone_dir.path)
      .expect("read clone branch status")
      .name;

    let (git_page, cx) = add_git_page_window_with_root(cx);

    let (command_ids, delete_branches) = git_page.update(cx, |this, cx| {
      this.selected_repo = Some(clone_dir.path.clone());
      this.branch_status = Some(make_branch_status(&clone_branch, 0, 0, true));
      let contents = this.build_command_palette_contents(1, cx);
      (
        contents
          .commands
          .into_iter()
          .map(|command| command.id)
          .collect::<Vec<_>>(),
        contents.delete_branches,
      )
    });

    assert!(command_ids.contains(&CommandPaletteCommandId::DeleteBranch));
    assert!(delete_branches.contains(&CommandPaletteBranch {
      name: "origin/feature".into(),
      kind: CommandPaletteBranchKind::Remote,
    }));
  }

  #[gpui::test]
  fn command_palette_hides_delete_branch_root_command_without_candidates(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-command-palette-delete-hidden");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "initial\n", "initial");
    let current_branch = current_branch_status(&repo.path)
      .expect("read current branch")
      .name;

    let (git_page, cx) = add_git_page_window_with_root(cx);

    let (command_ids, delete_branches) = git_page.update(cx, |this, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.branch_status = Some(make_branch_status(&current_branch, 0, 0, true));
      let contents = this.build_command_palette_contents(1, cx);
      (
        contents
          .commands
          .into_iter()
          .map(|command| command.id)
          .collect::<Vec<_>>(),
        contents.delete_branches,
      )
    });

    assert!(!command_ids.contains(&CommandPaletteCommandId::DeleteBranch));
    assert!(delete_branches.is_empty());
  }

  #[gpui::test]
  fn command_palette_moves_open_commands_after_git_commands(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-command-palette-order");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "initial\n", "initial");

    let (git_page, cx) = add_git_page_window_with_root(cx);

    let command_ids = git_page.update(cx, |this, cx| {
      this.selected_repo = Some(repo.path.clone());
      this
        .build_command_palette_contents(2, cx)
        .commands
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>()
    });

    let is_open_command = |id: CommandPaletteCommandId| {
      matches!(
        id,
        CommandPaletteCommandId::OpenRepository
          | CommandPaletteCommandId::OpenSessionPage
          | CommandPaletteCommandId::OpenGitPage
          | CommandPaletteCommandId::OpenGithubFromUrl
          | CommandPaletteCommandId::OpenGitConfigPage
          | CommandPaletteCommandId::OpenSettingsPage
          | CommandPaletteCommandId::OpenBillingPage
          | CommandPaletteCommandId::OpenAboutPage
          | CommandPaletteCommandId::SendFeedback
      )
    };

    let first_open_ix = command_ids
      .iter()
      .position(|id| is_open_command(*id))
      .expect("should include open commands");
    let last_non_open_ix = command_ids
      .iter()
      .rposition(|id| !is_open_command(*id))
      .expect("should include git commands");

    assert!(
      first_open_ix > last_non_open_ix,
      "open commands should be listed after git commands: {command_ids:?}"
    );

    let switch_repository_ix = command_ids
      .iter()
      .position(|id| *id == CommandPaletteCommandId::SwitchRepository)
      .expect("should include switch repository");
    let forget_repository_ix = command_ids
      .iter()
      .position(|id| *id == CommandPaletteCommandId::ForgetRepository)
      .expect("should include forget repository");
    let open_repository_ix = command_ids
      .iter()
      .position(|id| *id == CommandPaletteCommandId::OpenRepository)
      .expect("should include open repository");

    assert_eq!(switch_repository_ix + 1, forget_repository_ix);
    assert_eq!(forget_repository_ix + 1, open_repository_ix);
  }

  #[gpui::test]
  async fn command_palette_merge_branch_opens_first_conflicted_file(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-merge-conflict");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "base\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");

    let _ = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "main change\n",
      "main change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "feature change\n",
      "feature change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base");
    force_checkout_head(&repo.path);

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();
    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::MergeBranch {
          name: CommandPaletteBranch {
            name: "feature".into(),
            kind: CommandPaletteBranchKind::Local,
          },
        },
        _window,
        cx,
      )
    });

    assert!(result.is_ok(), "merge conflict should be handled in-editor");
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let selected_file = git_page.read_with(cx, |this, _| this.selected_file.clone());
    assert_eq!(selected_file.as_deref(), Some(Path::new("README.md")));
    let commit_input_value = git_page.read_with(cx, |this, cx| {
      this.commit_input.read(cx).value().to_string()
    });
    assert_eq!(
      commit_input_value,
      format!("Merge branch 'feature' into {base_branch}")
    );
    let editor_text = git_page.read_with(cx, |this, cx| {
      let editor = this.editor.as_ref().expect("editor opened").clone();
      editor.read_with(cx, |editor, cx| {
        let doc = editor.document().read(cx);
        doc.slice_to_string(0..doc.len())
      })
    });
    assert!(
      editor_text.contains("<<<<<<<"),
      "expected conflict markers in opened editor file: {editor_text}"
    );
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after failed merge")
        .name,
      base_branch
    );
  }

  #[gpui::test]
  async fn command_palette_abort_merge_clears_merge_state(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-abort-merge");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "base\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");

    let _ = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "main change\n",
      "main change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "feature change\n",
      "feature change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base");
    force_checkout_head(&repo.path);

    let _ = merge_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect_err("merge should fail with conflicts");
    assert!(
      is_merge_in_progress(&repo.path).expect("read merge state"),
      "merge state should be active after conflict"
    );

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let reload_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.reload_status(cx);
      this.status_task.take().expect("reload status task")
    });
    reload_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(git_page.read_with(cx, |this, _| this.merge_in_progress));
    git_page.update_in(cx, |this, window, cx| {
      this.commit_input.update(cx, |input, cx| {
        input.set_value("Merge branch 'feature' into main", window, cx)
      });
    });

    let result = git_page.update_in(cx, |this, window, cx| {
      this.handle_command_palette_action(CommandPaletteAction::AbortMerge, window, cx)
    });
    assert!(
      result.is_ok(),
      "abort merge via command palette should succeed"
    );
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(
      !is_merge_in_progress(&repo.path).expect("read merge state after abort"),
      "merge state should be cleaned after abort"
    );
    assert_eq!(
      std::fs::read_to_string(repo.path.join("README.md")).expect("read README after abort"),
      "main change\n"
    );
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after abort merge")
        .name,
      base_branch
    );
    assert!(!git_page.read_with(cx, |this, _| this.merge_in_progress));
    assert_eq!(
      git_page.read_with(cx, |this, cx| this
        .commit_input
        .read(cx)
        .value()
        .to_string()),
      ""
    );
  }

  #[gpui::test]
  async fn command_palette_abort_rebase_clears_rebase_state(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-abort-rebase");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "base\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");

    let _ = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "main change\n",
      "main change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "feature change\n",
      "feature change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base");
    force_checkout_head(&repo.path);

    let _ = rebase_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect_err("rebase should fail with conflicts");
    assert!(
      is_rebase_in_progress(&repo.path).expect("read rebase state"),
      "rebase state should be active after conflict"
    );

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let reload_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.reload_status(cx);
      this.status_task.take().expect("reload status task")
    });
    reload_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(git_page.read_with(cx, |this, _| this.rebase_in_progress));
    git_page.update_in(cx, |this, window, cx| {
      this
        .commit_input
        .update(cx, |input, cx| input.set_value("main change", window, cx));
    });

    let result = git_page.update_in(cx, |this, window, cx| {
      this.handle_command_palette_action(CommandPaletteAction::AbortRebase, window, cx)
    });
    assert!(
      result.is_ok(),
      "abort rebase via command palette should succeed"
    );
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(
      !is_rebase_in_progress(&repo.path).expect("read rebase state after abort"),
      "rebase state should be cleaned after abort"
    );
    assert_eq!(
      std::fs::read_to_string(repo.path.join("README.md")).expect("read README after abort"),
      "main change\n"
    );
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after abort rebase")
        .name,
      base_branch
    );
    assert!(!git_page.read_with(cx, |this, _| this.rebase_in_progress));
    assert_eq!(
      git_page.read_with(cx, |this, cx| this
        .commit_input
        .read(cx)
        .value()
        .to_string()),
      ""
    );
  }

  #[gpui::test]
  async fn command_palette_continue_rebase_completes_rebase_after_conflict_resolution(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-continue-rebase");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "base\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");

    let _ = commit_text_file(&repo.path, rel_path, "main change\n", "main change");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(&repo.path, rel_path, "feature change\n", "feature change");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base");
    force_checkout_head(&repo.path);

    let _ = rebase_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect_err("rebase should fail with conflicts");

    std::fs::write(repo.path.join(rel_path), "resolved\n").expect("write resolved contents");
    stage_file(&repo.path, rel_path).expect("stage resolved file");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let reload_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.reload_status(cx);
      this.status_task.take().expect("reload status task")
    });
    reload_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let result = git_page.update_in(cx, |this, window, cx| {
      this.handle_command_palette_action(CommandPaletteAction::ContinueRebase, window, cx)
    });
    assert!(
      result.is_ok(),
      "continue rebase via command palette should succeed"
    );
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(
      !is_rebase_in_progress(&repo.path).expect("read rebase state after continue"),
      "rebase state should be cleaned after continue"
    );
    assert_eq!(
      std::fs::read_to_string(repo.path.join(rel_path)).expect("read README after continue"),
      "resolved\n"
    );
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after continue rebase")
        .name,
      base_branch
    );
    assert!(!git_page.read_with(cx, |this, _| this.rebase_in_progress));
  }

  #[gpui::test]
  async fn command_palette_skip_rebase_skips_conflicted_commit(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-skip-rebase");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "base\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");

    let _ = commit_text_file(&repo.path, rel_path, "main change\n", "main change");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(&repo.path, rel_path, "feature change\n", "feature change");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base");
    force_checkout_head(&repo.path);

    let _ = rebase_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect_err("rebase should fail with conflicts");
    assert!(
      is_rebase_in_progress(&repo.path).expect("read rebase state"),
      "rebase state should be active after conflict"
    );

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let reload_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.reload_status(cx);
      this.status_task.take().expect("reload status task")
    });
    reload_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let result = git_page.update_in(cx, |this, window, cx| {
      this.handle_command_palette_action(CommandPaletteAction::SkipRebase, window, cx)
    });
    assert!(
      result.is_ok(),
      "skip rebase via command palette should succeed"
    );
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(
      !is_rebase_in_progress(&repo.path).expect("read rebase state after skip"),
      "rebase state should be cleaned after skip"
    );
    assert_eq!(
      std::fs::read_to_string(repo.path.join(rel_path)).expect("read README after skip"),
      "feature change\n"
    );
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after skip rebase")
        .name,
      base_branch
    );
  }

  #[gpui::test]
  async fn command_palette_rebase_branch_opens_first_conflicted_file(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-rebase-conflict");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "base\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");

    let _ = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "main change\n",
      "main change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "feature change\n",
      "feature change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base");
    force_checkout_head(&repo.path);

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();
    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::RebaseBranch {
          name: CommandPaletteBranch {
            name: "feature".into(),
            kind: CommandPaletteBranchKind::Local,
          },
        },
        _window,
        cx,
      )
    });

    assert!(
      result.is_ok(),
      "rebase conflict should be handled in-editor"
    );
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let selected_file = git_page.read_with(cx, |this, _| this.selected_file.clone());
    assert_eq!(selected_file.as_deref(), Some(Path::new("README.md")));
    let commit_input_value = git_page.read_with(cx, |this, cx| {
      this.commit_input.read(cx).value().to_string()
    });
    assert_eq!(commit_input_value, "main change");
    let editor_text = git_page.read_with(cx, |this, cx| {
      let editor = this.editor.as_ref().expect("editor opened").clone();
      editor.read_with(cx, |editor, cx| {
        let doc = editor.document().read(cx);
        doc.slice_to_string(0..doc.len())
      })
    });
    assert!(
      editor_text.contains("<<<<<<<"),
      "expected conflict markers in opened editor file: {editor_text}"
    );
    assert!(
      is_rebase_in_progress(&repo.path).expect("read rebase state"),
      "rebase state should be active after conflict"
    );
  }
}
