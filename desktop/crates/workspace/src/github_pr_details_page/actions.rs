//! The keyboard actions of the page and the palettes they open.

use super::*;

impl GithubPrDetailsPage {
  pub(super) fn show_file_search_action(
    &mut self,
    _: &ShowFileSearch,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.active_tab_ix != PR_TAB_CHANGES_IX {
      return;
    }

    self.open_file_search_palette(window, cx);
  }

  pub(super) fn show_command_palette_action(
    &mut self,
    _: &ShowCommandPalette,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.open_command_palette(window, cx);
  }

  pub(super) fn switch_to_pr_branch_action(
    &mut self,
    _: &crate::SwitchToPrBranch,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.prompt_or_switch_local_branch_to_pr_branch(window, cx);
    cx.stop_propagation();
  }

  pub(super) fn previous_annotation_action(
    &mut self,
    _: &crate::PreviousAnnotation,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.active_tab_ix != PR_TAB_CHANGES_IX {
      return;
    }

    self.diff_editor.update(cx, |editor, cx| {
      editor.navigate_hunk(HunkNavigationDirection::Previous, cx);
    });
    cx.stop_propagation();
  }

  pub(super) fn next_annotation_action(
    &mut self,
    _: &crate::NextAnnotation,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.active_tab_ix != PR_TAB_CHANGES_IX {
      return;
    }

    self.diff_editor.update(cx, |editor, cx| {
      editor.navigate_hunk(HunkNavigationDirection::Next, cx);
    });
    cx.stop_propagation();
  }

  pub(super) fn previous_review_comment_action(
    &mut self,
    _: &crate::PreviousReviewComment,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.active_tab_ix != PR_TAB_CHANGES_IX {
      return;
    }
    self.navigate_review_comment(ReviewCommentNavigationDirection::Previous, cx);
    cx.stop_propagation();
  }

  pub(super) fn next_review_comment_action(
    &mut self,
    _: &crate::NextReviewComment,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.active_tab_ix != PR_TAB_CHANGES_IX {
      return;
    }
    self.navigate_review_comment(ReviewCommentNavigationDirection::Next, cx);
    cx.stop_propagation();
  }

  pub(super) fn toggle_diff_view_action(
    &mut self,
    _: &crate::ToggleDiffView,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.active_tab_ix != PR_TAB_CHANGES_IX {
      return;
    }

    self.toggle_diff_view(cx);
    cx.stop_propagation();
  }

  pub(super) fn toggle_commit_by_commit_action(
    &mut self,
    _: &crate::ToggleCommitByCommit,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.active_tab_ix != PR_TAB_CHANGES_IX {
      return;
    }
    if self.commits.is_empty() || self.commits_loading || self.commits_error.is_some() {
      return;
    }
    if self.selected_commit_sha.is_some() {
      self.exit_commit_by_commit_review(cx);
    } else {
      self.enter_commit_by_commit_review(cx);
    }
    cx.stop_propagation();
  }

  pub(super) fn previous_pr_commit_action(
    &mut self,
    _: &crate::PreviousPrCommit,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.active_tab_ix != PR_TAB_CHANGES_IX {
      return;
    }
    if self.selected_commit_sha.is_none() {
      return;
    }
    self.navigate_commit_by_commit(CommitNavigationDirection::Previous, cx);
    cx.stop_propagation();
  }

  pub(super) fn next_pr_commit_action(
    &mut self,
    _: &crate::NextPrCommit,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.active_tab_ix != PR_TAB_CHANGES_IX {
      return;
    }
    if self.selected_commit_sha.is_none() {
      return;
    }
    self.navigate_commit_by_commit(CommitNavigationDirection::Next, cx);
    cx.stop_propagation();
  }

  pub(super) fn comment_hunk_action(
    &mut self,
    _: &crate::CommentHunk,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.active_tab_ix != PR_TAB_CHANGES_IX {
      return;
    }
    if self.selected_commit_sha.is_some() {
      return;
    }
    let editor = self.diff_editor.clone();
    if editor.update(cx, |editor, cx| {
      editor.start_review_comment_for_active_hunk(window, cx)
    }) {
      cx.stop_propagation();
    }
  }

  pub(super) fn focus_file_tree_action(
    &mut self,
    _: &crate::FocusFileTree,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.active_tab_ix == PR_TAB_CHANGES_IX {
      self.focus_changes_tree(window, cx);
    } else {
      self.set_active_tab(PR_TAB_CHANGES_IX, window, cx);
    }
    cx.stop_propagation();
  }

  pub(super) fn toggle_hide_whitespace_action(
    &mut self,
    _: &crate::ToggleHideWhitespace,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.active_tab_ix != PR_TAB_CHANGES_IX {
      return;
    }

    self.toggle_hide_whitespace(cx);
    cx.stop_propagation();
  }

  pub(super) fn previous_page_tab_action(
    &mut self,
    _: &crate::PreviousPageTab,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let next_ix = adjacent_pr_tab_ix(self.active_tab_ix, TabNavigationDirection::Previous);
    self.set_active_tab(next_ix, window, cx);
    cx.stop_propagation();
  }

  pub(super) fn next_page_tab_action(
    &mut self,
    _: &crate::NextPageTab,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let next_ix = adjacent_pr_tab_ix(self.active_tab_ix, TabNavigationDirection::Next);
    self.set_active_tab(next_ix, window, cx);
    cx.stop_propagation();
  }

  pub(super) fn find_action(&mut self, action: &Find, window: &mut Window, cx: &mut Context<Self>) {
    if self.active_tab_ix != PR_TAB_CHANGES_IX {
      return;
    }

    self.diff_editor.update(cx, |editor, cx| {
      editor::find(editor, action, window, cx);
    });
  }

  pub(super) fn close_find_action(
    &mut self,
    action: &CloseFind,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.active_tab_ix != PR_TAB_CHANGES_IX {
      return;
    }

    self.diff_editor.update(cx, |editor, cx| {
      editor::close_find(editor, action, window, cx);
    });
  }

  pub(super) fn open_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let commands = self.command_palette_commands(cx);

    let view = cx.entity();
    let handler: CommandPaletteHandler = Arc::new(move |action, window, cx| {
      view.update(cx, |view, cx| {
        view.handle_command_palette_action(action, window, cx)
      })
    });

    let config = CommandPaletteConfig::new(Vec::new(), commands, handler);
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
      CommandPaletteAction::SwitchToPrBranch => {
        cx.defer_in(window, |this, window, cx| {
          this.prompt_or_switch_local_branch_to_pr_branch(window, cx);
        });
        Ok(())
      }
      CommandPaletteAction::CopyPrBranch => {
        let Some(pull_request) = self.pull_request.as_ref() else {
          return Err("No pull request loaded.".into());
        };
        let branch_name = pull_request.head_ref_name.to_string();
        cx.write_to_clipboard(ClipboardItem::new_string(branch_name.clone()));
        window.push_notification(
          Notification::success(format!("Copied branch name: {branch_name}")),
          cx,
        );
        Ok(())
      }
      CommandPaletteAction::OpenSessionPage => {
        NavigationHistory::navigate("/session", cx);
        Ok(())
      }
      CommandPaletteAction::OpenGithubPrDetails {
        owner,
        repo,
        number,
        open_changes_tab,
        review_comment_id,
      } => {
        self.load_pull_request(
          owner.clone(),
          repo.clone(),
          number,
          GithubPrOpenTarget {
            open_changes_tab,
            review_comment_id,
          },
          cx,
        );
        NavigationHistory::navigate(crate::navigation::build_pr_path(&owner, &repo, number), cx);
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
        crate::feedback_dialog::open_feedback_dialog(window, cx);
        Ok(())
      }
      CommandPaletteAction::ToggleUnchangedFiles => {
        let next = !self.show_local_project_files;
        self.set_show_local_project_files(next, cx);
        Ok(())
      }
      CommandPaletteAction::OpenPrMergePopover => {
        if self.pull_request.is_none() {
          return Err("No pull request loaded.".into());
        }
        if self.is_pull_request_merged() {
          return Err("Pull request already merged.".into());
        }
        if self
          .pull_request
          .as_ref()
          .is_some_and(|pull_request| pull_request.draft)
        {
          return Err("Pull request is a draft.".into());
        }
        self.merge_popover_open = true;
        if self.merge_form_reset_pending {
          self.reset_merge_form(window, cx);
        }
        let focus_handle = self.merge_method_focus_handle.clone();
        window.on_next_frame(move |window, cx| {
          window.focus(&focus_handle, cx);
        });
        cx.notify();
        Ok(())
      }
      CommandPaletteAction::OpenPrReviewPopover => {
        if self.pull_request.is_none() {
          return Err("No pull request loaded.".into());
        }
        if self.is_pull_request_merged() {
          return Err("Pull request already merged.".into());
        }
        self.review_popover_open = true;
        if self.review_form_reset_pending {
          self.reset_review_form(window, cx);
        }
        if self.is_current_user_pr_author(cx)
          && !matches!(self.review_decision, GithubPrReviewDecision::Comment)
        {
          self.review_decision = GithubPrReviewDecision::Comment;
        }
        self.focus_review_input(window);
        cx.notify();
        Ok(())
      }
      CommandPaletteAction::TogglePrCommitByCommit => {
        if self.commits.is_empty() || self.commits_loading || self.commits_error.is_some() {
          return Err("No commits available.".into());
        }
        if self.selected_commit_sha.is_some() {
          self.exit_commit_by_commit_review(cx);
        } else {
          self.enter_commit_by_commit_review(cx);
        }
        Ok(())
      }
      _ => Err("Command not available.".into()),
    }
  }

  pub(super) fn open_file_search_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let entries = self.active_file_search_entries(cx);
    if entries.is_empty() {
      return;
    }

    let view = cx.entity();
    let handler: SearchFileHandler = Arc::new(move |path, window, cx| {
      view.update(cx, |view, cx| {
        view.select_file_from_palette(&path, cx);
        view.refocus_page_shortcuts(window, cx);
      });

      Ok(())
    });
    open_shared_file_search_palette(window, cx, entries, handler, true);
  }

  pub(super) fn select_file_from_palette(&mut self, path: &Path, cx: &mut Context<Self>) {
    let key = path.to_string_lossy().to_string();

    let tree_item = TreeItem::new(key.clone(), key.clone());
    self.tree_state.update(cx, |state, cx| {
      state.set_selected_item(Some(&tree_item), cx);
      if let Some(ix) = state.selected_index() {
        state.scroll_to_item(ix, gpui::ScrollStrategy::Top);
      }
    });

    self.select_visible_tree_path(key.as_str(), cx);
  }

  pub(super) fn command_palette_commands(&self, cx: &App) -> Vec<CommandPaletteCommand> {
    let include_github = AuthStateStore::has_github_access(cx);
    let availability = self.local_project_availability(cx);
    let mut commands = Self::local_project_command_palette_commands(&availability);
    if matches!(availability, GithubPrLocalProjectAvailability::Ready { .. }) {
      commands.push(CommandPaletteCommand::toggle_unchanged_files(
        self.show_local_project_files,
      ));
    }
    if self.pull_request.is_some() {
      commands.push(CommandPaletteCommand::copy_pr_branch());
    }
    if !self.is_pull_request_merged() {
      if self
        .pull_request
        .as_ref()
        .is_some_and(|pull_request| !pull_request.draft)
      {
        commands.push(CommandPaletteCommand::open_pr_merge_popover());
      }
      if self.pull_request.is_some() {
        commands.push(CommandPaletteCommand::open_pr_review_popover());
      }
    }
    if !self.commits.is_empty() && !self.commits_loading && self.commits_error.is_none() {
      commands.push(CommandPaletteCommand::toggle_pr_commit_by_commit(
        self.selected_commit_sha.is_some(),
      ));
    }
    commands.extend(CommandPaletteCommand::default_global_commands(
      CommandPalettePage::GithubPrDetails,
      include_github,
    ));
    commands
  }

  pub(super) fn refocus_page_shortcuts(&self, window: &mut Window, cx: &mut Context<Self>) {
    let focus_handle = self.focus_handle.clone();
    window.focus(&focus_handle, cx);
    cx.on_next_frame(window, move |_, window, cx| {
      window.focus(&focus_handle, cx);
    });
  }

  pub(super) fn navigate_back(&self, cx: &mut Context<Self>) {
    NavigationHistory::navigate_back(cx);
  }
}
