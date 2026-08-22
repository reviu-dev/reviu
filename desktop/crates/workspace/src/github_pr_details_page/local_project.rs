//! The local repository behind a pull request: browsing its files read-only,
//! and moving its branch to match the pull request.

use super::*;

impl GithubPrDetailsPage {
  pub(super) fn pr_source_repository(pull_request: &GithubPullRequestDetails) -> &GithubRepository {
    pull_request
      .head_repository
      .as_ref()
      .unwrap_or(&pull_request.repository)
  }

  pub(super) fn active_local_repo_for_pull_request(&self, cx: &App) -> Option<ActiveLocalRepo> {
    let pull_request = self.pull_request.as_ref()?;
    let local_repo = ActiveLocalRepoStore::get(cx)?;
    local_repo_matches_pull_request(pull_request, &local_repo).then_some(local_repo)
  }

  pub(super) fn effective_local_repo_for_pull_request(&self, cx: &App) -> Option<ActiveLocalRepo> {
    self
      .active_local_repo_for_pull_request(cx)
      .or_else(|| self.resolved_local_repo.clone())
  }

  pub(super) fn local_project_availability_for_repo(
    pull_request: &GithubPullRequestDetails,
    local_repo: ActiveLocalRepo,
  ) -> GithubPrLocalProjectAvailability {
    if local_repo.current_branch.as_deref() != Some(pull_request.head_ref_name.as_str()) {
      return GithubPrLocalProjectAvailability::NeedsBranchSwitch {
        repo_root: local_repo.repo_root,
        current_branch: local_repo.current_branch,
        has_uncommitted_changes: local_repo.has_uncommitted_changes,
      };
    }

    let Some(local_head_sha) = local_repo.head_sha.as_deref() else {
      return GithubPrLocalProjectAvailability::Hidden;
    };
    if local_head_sha == pull_request.head_sha {
      return GithubPrLocalProjectAvailability::Ready {
        repo_root: local_repo.repo_root,
      };
    }

    if local_repo.has_uncommitted_changes {
      GithubPrLocalProjectAvailability::Dirty {
        repo_root: local_repo.repo_root,
      }
    } else {
      GithubPrLocalProjectAvailability::NeedsUpdate {
        repo_root: local_repo.repo_root,
      }
    }
  }

  pub(super) fn maybe_refresh_resolved_local_repo_match(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.selected_commit_sha.is_some() || self.pull_request.is_none() {
      return;
    }

    if self.active_local_repo_for_pull_request(cx).is_some()
      || self.resolved_local_repo_scan_complete
      || self.resolved_local_repo_task.is_some()
    {
      return;
    }

    cx.defer_in(window, |this, _window, cx| {
      this.refresh_resolved_local_repo_match(cx);
    });
  }

  pub(super) fn refresh_resolved_local_repo_match(&mut self, cx: &mut Context<Self>) {
    if self.resolved_local_repo_task.is_some() {
      return;
    }

    let Some(pull_request) = self.pull_request.clone() else {
      self.resolved_local_repo = None;
      self.resolved_local_repo_task = None;
      self.resolved_local_repo_scan_complete = false;
      return;
    };

    if self.active_local_repo_for_pull_request(cx).is_some() {
      self.resolved_local_repo = None;
      self.resolved_local_repo_task = None;
      self.resolved_local_repo_scan_complete = false;
      return;
    }

    self.resolved_local_repo_generation = self.resolved_local_repo_generation.wrapping_add(1);
    let generation = self.resolved_local_repo_generation;
    self.resolved_local_repo = None;
    self.resolved_local_repo_scan_complete = false;

    let excluded_repo_root = ActiveLocalRepoStore::get(cx).map(|repo| repo.repo_root);
    let task = cx.spawn(async move |this, cx| {
      let pull_request_for_scan = pull_request.clone();
      let excluded_repo_root_for_scan = excluded_repo_root.clone();
      let snapshot = cx
        .background_spawn(async move {
          find_matching_recent_local_repo(
            &pull_request_for_scan,
            excluded_repo_root_for_scan.as_deref(),
          )
        })
        .await;

      let _ = this.update(cx, |this, cx| {
        if this.resolved_local_repo_generation != generation {
          return;
        }

        this.resolved_local_repo_task = None;
        this.resolved_local_repo_scan_complete = true;
        this.resolved_local_repo = snapshot;

        cx.notify();
      });
    });

    self.resolved_local_repo_task = Some(task);
  }

  pub(super) fn sync_active_local_repo_store_snapshot(
    &self,
    snapshot: &ActiveLocalRepo,
    cx: &mut Context<Self>,
  ) {
    if ActiveLocalRepoStore::get(cx)
      .as_ref()
      .map(|repo| repo.repo_root.as_path())
      == Some(snapshot.repo_root.as_path())
    {
      ActiveLocalRepoStore::set(cx, Some(snapshot.clone()));
    }
  }

  pub(super) fn sync_resolved_local_repo_snapshot(&mut self, snapshot: &ActiveLocalRepo) {
    let Some(pull_request) = self.pull_request.as_ref() else {
      self.resolved_local_repo = None;
      self.resolved_local_repo_scan_complete = false;
      return;
    };

    if local_repo_matches_pull_request(pull_request, snapshot) {
      self.resolved_local_repo = Some(snapshot.clone());
      self.resolved_local_repo_scan_complete = true;
    } else {
      self.resolved_local_repo = None;
      self.resolved_local_repo_scan_complete = false;
    }
  }

  pub(super) fn local_project_availability(&self, cx: &App) -> GithubPrLocalProjectAvailability {
    if self.selected_commit_sha.is_some() {
      return GithubPrLocalProjectAvailability::Hidden;
    }

    let Some(pull_request) = self.pull_request.as_ref() else {
      return GithubPrLocalProjectAvailability::Hidden;
    };
    let Some(local_repo) = self.effective_local_repo_for_pull_request(cx) else {
      return GithubPrLocalProjectAvailability::Hidden;
    };

    Self::local_project_availability_for_repo(pull_request, local_repo)
  }

  pub(super) fn effective_local_repo_has_uncommitted_changes(&self, cx: &App) -> bool {
    self
      .effective_local_repo_for_pull_request(cx)
      .is_some_and(|repo| repo.has_uncommitted_changes)
  }

  pub(super) fn local_project_mode_active(&self, cx: &App) -> bool {
    self.show_local_project_files
      && matches!(
        self.local_project_availability(cx),
        GithubPrLocalProjectAvailability::Ready { .. }
      )
  }

  pub(super) fn sync_local_project_tree_state(&mut self, cx: &mut Context<Self>) {
    self.sync_changes_tree_state(cx);
  }

  pub(super) fn maybe_load_local_project_files_if_needed(
    &mut self,
    repo_root: &Path,
    cx: &mut Context<Self>,
  ) {
    if self.local_project_loaded_repo_root.as_deref() == Some(repo_root) {
      if self.local_project_tree_loading || self.local_project_files_task.is_some() {
        return;
      }
      if !self.local_project_lookup.is_empty() || self.local_project_tree_error.is_some() {
        self.sync_local_project_tree_state(cx);
        return;
      }
    }

    self.load_local_project_files(repo_root.to_path_buf(), cx);
  }

  pub(super) fn load_local_project_files(&mut self, repo_root: PathBuf, cx: &mut Context<Self>) {
    if self.local_project_loaded_repo_root.as_ref() == Some(&repo_root)
      && (self.local_project_tree_loading || self.local_project_files_task.is_some())
    {
      return;
    }

    self.local_project_loaded_repo_root = Some(repo_root.clone());
    self.local_project_tree_loading = true;
    self.local_project_tree_error = None;
    self.local_project_files_task = None;
    self.local_project_lookup.clear();
    self.selected_local_project_file = None;
    self.selected_local_project_tree_id = None;
    self.local_project_open_file_task = None;
    self.local_project_open_file_generation =
      self.local_project_open_file_generation.wrapping_add(1);

    if self.show_local_project_files {
      self.tree_state.update(cx, |state, cx| {
        state.set_items(Vec::new(), cx);
        state.set_selected_index(None, cx);
      });
    }

    let requested_repo_root = repo_root.clone();
    let task = cx.spawn(async move |this, cx| {
      let repo_root_for_load = requested_repo_root.clone();
      let result = cx
        .background_spawn(async move { list_repo_head_files(&repo_root_for_load) })
        .await;

      let _ = this.update(cx, |this, cx| {
        if this.local_project_loaded_repo_root.as_ref() != Some(&requested_repo_root) {
          return;
        }

        this.local_project_files_task = None;
        this.local_project_tree_loading = false;

        match result {
          Ok(paths) => {
            let files = paths
              .into_iter()
              .map(|path| {
                Rc::new(GithubPrLocalProjectFile {
                  path: path.to_string_lossy().replace(['\n', '\r'], "").into(),
                })
              })
              .collect::<Vec<_>>();
            let (_, lookup, _, _) = build_local_project_tree_items(&files);
            this.local_project_lookup = lookup;
            this.local_project_tree_error = None;
            this.refresh_tree_text_search(cx);
          }
          Err(error) => {
            this.local_project_lookup.clear();
            this.local_project_tree_error = Some(error.to_string().into());
            if this.local_project_mode_active(cx)
              && this.selected_local_project_file.is_some()
              && this.selected_file.is_none()
            {
              this.set_selected_local_project_file(None, cx);
            }
            this.refresh_tree_text_search(cx);
          }
        }

        cx.notify();
      });
    });

    self.local_project_files_task = Some(task);
    cx.notify();
  }

  pub(super) fn sync_local_project_tree_selection(&mut self, cx: &mut Context<Self>) {
    let Some(file) = self.selected_local_project_file.as_ref() else {
      return;
    };

    let key = file.path.as_ref().to_string();
    let tree_item = TreeItem::new(key.clone(), key.clone());
    self.tree_state.update(cx, |state, cx| {
      state.set_selected_item(Some(&tree_item), cx);
      if let Some(ix) = state.selected_index() {
        state.scroll_to_item(ix, gpui::ScrollStrategy::Top);
      }
    });
  }

  pub(super) fn load_local_project_snapshot_into_diff_editor(
    &mut self,
    file_path: PathBuf,
    contents: String,
    cx: &mut Context<Self>,
  ) {
    self.diff_editor = Self::build_detached_diff_editor(file_path, cx);
    self.diff_editor.update(cx, |editor, cx| {
      editor.load_readonly_snapshot(contents, None, cx);
      editor.reset_after_replace();
      editor.reset_selection(cx);
    });
  }

  pub(super) fn set_selected_local_project_file(
    &mut self,
    selected: Option<Rc<GithubPrLocalProjectFile>>,
    cx: &mut Context<Self>,
  ) {
    let current_id = self
      .selected_local_project_file
      .as_ref()
      .map(|file| file.path.clone());
    let next_id = selected.as_ref().map(|file| file.path.clone());
    if current_id == next_id {
      return;
    }

    self.selected_local_project_file = selected.clone();
    self.selected_local_project_tree_id = selected.as_ref().map(|file| file.path.to_string());
    self.selected_file = None;
    self.selected_tree_id = None;
    self.active_review_comment_id = None;
    self.selected_file_review_comment_ids.clear();
    self.sync_sentry_pr_context();
    if !self.selected_file_is_markdown() && !self.selected_file_is_svg() {
      self.show_markdown_preview = false;
    }
    self.binary_preview = None;
    self.svg_preview.update(cx, |preview, _| preview.clear());
    self.file_error = None;
    self.local_project_open_file_generation =
      self.local_project_open_file_generation.wrapping_add(1);
    self.local_project_open_file_task = None;

    let Some(file) = selected else {
      self.file_loading = false;
      self.clear_diff_editor(cx);
      self.sync_diff_view(cx);
      self.sync_review_comments(cx);
      cx.notify();
      return;
    };

    let Some(repo_root) = self.local_project_loaded_repo_root.clone() else {
      self.file_loading = false;
      self.file_error = Some("Local project unavailable".into());
      self.clear_diff_editor(cx);
      self.sync_review_comments(cx);
      cx.notify();
      return;
    };

    self.sync_local_project_tree_selection(cx);
    self.file_loading = true;
    self.file_error = None;
    self.clear_diff_editor(cx);

    let generation = self.local_project_open_file_generation;
    let requested_repo_root = repo_root.clone();
    let requested_rel_path = PathBuf::from(file.path.as_ref());
    let requested_key = file.path.to_string();
    let requested_absolute_path = requested_repo_root.join(&requested_rel_path);
    let task = cx.spawn(async move |this, cx| {
      let repo_root_for_load = requested_repo_root.clone();
      let absolute_path_for_load = requested_absolute_path.clone();
      let rel_path_for_load = requested_rel_path.clone();
      let (snapshot_contents, binary_bytes) = cx
        .background_spawn(async move {
          let loaded = Editor::load_file_for_editor(&repo_root_for_load, &absolute_path_for_load);
          let git_store = GitStore::new(repo_root_for_load.clone());
          let head_contents = git_store
            .load_bases(rel_path_for_load.as_path())
            .ok()
            .and_then(|bases| bases.head);
          let head_binary_bytes = git_store
            .load_binary_bases(rel_path_for_load.as_path())
            .ok()
            .and_then(|bases| bases.head);
          (
            head_contents.unwrap_or(loaded.content),
            head_binary_bytes.or(loaded.binary_bytes),
          )
        })
        .await;

      let _ = this.update(cx, move |this, cx| {
        if this.local_project_open_file_generation != generation {
          return;
        }
        if this.local_project_loaded_repo_root.as_ref() != Some(&requested_repo_root) {
          return;
        }
        if this
          .selected_local_project_file
          .as_ref()
          .map(|file| file.path.as_ref())
          != Some(requested_key.as_str())
        {
          return;
        }

        this.load_local_project_snapshot_into_diff_editor(
          requested_rel_path.clone(),
          snapshot_contents,
          cx,
        );
        this.binary_preview =
          Self::build_binary_preview(requested_rel_path.as_path(), binary_bytes.clone());
        this.file_loading = false;
        this.file_error = None;
        this.sync_diff_view(cx);
        cx.notify();
      });
    });
    self.local_project_open_file_task = Some(task);
    self.sync_review_comments(cx);
    cx.notify();
  }

  pub(super) fn selected_local_project_file_is_markdown(&self) -> bool {
    self
      .selected_local_project_file
      .as_ref()
      .map(|file| is_markdown_path(Path::new(file.path.as_ref())))
      .unwrap_or(false)
  }

  pub(super) fn selected_local_project_file_is_svg(&self) -> bool {
    self
      .selected_local_project_file
      .as_ref()
      .map(|file| is_svg_path(Path::new(file.path.as_ref())))
      .unwrap_or(false)
  }

  pub(super) fn set_show_local_project_files(&mut self, enabled: bool, cx: &mut Context<Self>) {
    if self.show_local_project_files == enabled {
      return;
    }

    let previous_selection = self.current_selected_tree_path();

    if enabled {
      let GithubPrLocalProjectAvailability::Ready { repo_root } =
        self.local_project_availability(cx)
      else {
        return;
      };
      self.saved_pr_selected_tree_id = previous_selection;
      self.show_local_project_files = true;
      self.local_project_update_error = None;
      self.maybe_load_local_project_files_if_needed(repo_root.as_path(), cx);
      if !self.local_project_tree_loading {
        self.refresh_tree_text_search(cx);
      }
      cx.notify();
      return;
    }

    self.saved_pr_selected_tree_id = previous_selection;
    self.show_local_project_files = false;
    self.local_project_open_file_task = None;
    self.local_project_open_file_generation =
      self.local_project_open_file_generation.wrapping_add(1);
    self.file_loading = false;
    self.file_error = None;
    self.binary_preview = None;
    self.refresh_tree_text_search(cx);
    cx.notify();
  }

  pub(super) fn confirm_switch_local_branch_with_stash(
    &mut self,
    post_action: Option<GithubPrLocalProjectPostAction>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(pull_request) = self.pull_request.as_ref() else {
      return;
    };
    let branch_name = pull_request.head_ref_name.clone();
    let title: SharedString = "Stash changes before switching branches?".into();
    let message: SharedString = format!(
      "Create a stash with tracked and untracked files, then switch to {}?",
      branch_name
    )
    .into();
    let view = cx.entity();

    window.open_alert_dialog(cx, move |alert, _, _| {
      let view = view.clone();
      let post_action = post_action.clone();
      ConfirmDialog::new(title.clone(), div().child(message.clone()))
        .confirm_text("Stash and switch")
        .cancel_text("Cancel")
        .on_confirm(move |_, window, cx| {
          let post_action = post_action.clone();
          view.update(cx, |this, cx| {
            this.switch_local_branch_to_pr_branch(true, post_action, window, cx);
          });
          true
        })
        .build(alert)
    });
  }

  pub(super) fn confirm_prepare_local_branch_with_stash(
    &mut self,
    repo_root: PathBuf,
    post_action: GithubPrLocalProjectPostAction,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let title: SharedString = "Stash changes before switching?".into();
    let message: SharedString =
      "Create a stash with tracked and untracked files, then prepare this PR branch in the workspace?"
        .into();
    let view = cx.entity();

    window.open_alert_dialog(cx, move |alert, _, _| {
      let view = view.clone();
      let repo_root = repo_root.clone();
      let post_action = post_action.clone();
      ConfirmDialog::new(title.clone(), div().child(message.clone()))
        .confirm_text("Stash and switch")
        .cancel_text("Cancel")
        .on_confirm(move |_, window, cx| {
          let repo_root = repo_root.clone();
          let post_action = post_action.clone();
          view.update(cx, |this, cx| {
            let _ = window;
            this.start_sync_local_branch_to_pr_head(repo_root, true, Some(post_action), cx);
          });
          true
        })
        .build(alert)
    });
  }

  pub(super) fn execute_local_project_post_action(
    &mut self,
    post_action: GithubPrLocalProjectPostAction,
    repo_root: PathBuf,
    cx: &mut Context<Self>,
  ) {
    match post_action {
      GithubPrLocalProjectPostAction::EnsurePrHeadThenMergeBaseInWorkspace { base_branch_name } => {
        let current_head = current_head_sha(&repo_root).ok().flatten();
        let pr_head = self.pull_request.as_ref().map(|pr| pr.head_sha.as_str());

        if current_head.as_deref() == pr_head {
          SessionPageHandle::show_repository_and_merge_base(repo_root, base_branch_name, cx);
        } else {
          self.start_sync_local_branch_to_pr_head(
            repo_root,
            false,
            Some(GithubPrLocalProjectPostAction::MergeBaseInWorkspace { base_branch_name }),
            cx,
          );
        }
      }
      GithubPrLocalProjectPostAction::MergeBaseInWorkspace { base_branch_name } => {
        SessionPageHandle::show_repository_and_merge_base(repo_root, base_branch_name, cx);
      }
    }
  }

  pub(super) fn prompt_or_switch_local_branch_to_pr_branch(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.local_branch_switch_loading {
      return;
    }

    let GithubPrLocalProjectAvailability::NeedsBranchSwitch {
      has_uncommitted_changes,
      ..
    } = self.local_project_availability(cx)
    else {
      return;
    };

    if has_uncommitted_changes {
      self.confirm_switch_local_branch_with_stash(None, window, cx);
    } else {
      self.switch_local_branch_to_pr_branch(false, None, window, cx);
    }
  }

  pub(super) fn start_switch_local_branch_to_pr_branch(
    &mut self,
    repo_root: PathBuf,
    stash_before_switch: bool,
    post_action: Option<GithubPrLocalProjectPostAction>,
    cx: &mut Context<Self>,
  ) {
    if self.local_branch_switch_loading {
      return;
    }

    let Some(pull_request) = self.pull_request.as_ref() else {
      return;
    };

    let branch_name = pull_request.head_ref_name.clone();
    self.local_branch_switch_loading = true;
    self.local_branch_switch_error = None;
    self.local_project_update_error = None;
    cx.notify();

    let task = cx.spawn(async move |this, cx| {
      let repo_root_for_action = repo_root.clone();
      let branch_name_for_action = branch_name.clone();
      let result = cx
        .background_spawn(async move {
          if stash_before_switch {
            let stash_message = default_stash_message(&repo_root_for_action).ok();
            create_stash(&repo_root_for_action, true, stash_message.as_deref())?;
          }
          switch_to_branch_name(&repo_root_for_action, &branch_name_for_action)?;
          Ok::<_, anyhow::Error>(local_repo_snapshot(
            &repo_root_for_action,
            Some(branch_name_for_action.as_str()),
          ))
        })
        .await;

      let _ = this.update(cx, |this, cx| {
        this.local_branch_switch_task = None;
        this.local_branch_switch_loading = false;

        match result {
          Ok(snapshot) => {
            this.local_branch_switch_error = None;
            if let Some(snapshot) = snapshot {
              this.sync_active_local_repo_store_snapshot(&snapshot, cx);
              this.sync_resolved_local_repo_snapshot(&snapshot);
            }
            this.load_local_project_files(repo_root.clone(), cx);
            if let Some(post_action) = post_action.clone() {
              this.execute_local_project_post_action(post_action, repo_root.clone(), cx);
            }
          }
          Err(error) => {
            this.local_branch_switch_error = Some(error.to_string().into());
          }
        }

        cx.notify();
      });
    });

    self.local_branch_switch_task = Some(task);
  }

  pub(super) fn switch_local_branch_to_pr_branch(
    &mut self,
    stash_before_switch: bool,
    post_action: Option<GithubPrLocalProjectPostAction>,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let GithubPrLocalProjectAvailability::NeedsBranchSwitch { repo_root, .. } =
      self.local_project_availability(cx)
    else {
      return;
    };

    self.start_switch_local_branch_to_pr_branch(repo_root, stash_before_switch, post_action, cx);
  }

  pub(super) fn start_sync_local_branch_to_pr_head(
    &mut self,
    repo_root: PathBuf,
    stash_before_update: bool,
    post_action: Option<GithubPrLocalProjectPostAction>,
    cx: &mut Context<Self>,
  ) {
    if self.local_project_update_loading {
      return;
    }

    let Some(pull_request) = self.pull_request.as_ref() else {
      return;
    };

    let branch_name = pull_request.head_ref_name.clone();
    let target_head_sha = pull_request.head_sha.clone();
    self.local_project_update_loading = true;
    self.local_project_update_error = None;
    cx.notify();

    let task = cx.spawn(async move |this, cx| {
      let repo_root_for_update = repo_root.clone();
      let branch_name_for_update = branch_name.clone();
      let target_head_sha_for_update = target_head_sha.clone();
      let result = cx
        .background_spawn(async move {
          if stash_before_update {
            let stash_message = default_stash_message(&repo_root_for_update).ok();
            create_stash(&repo_root_for_update, true, stash_message.as_deref())?;
          }
          sync_current_branch_to_head(
            &repo_root_for_update,
            &branch_name_for_update,
            &target_head_sha_for_update,
          )?;
          Ok::<_, anyhow::Error>(local_repo_snapshot(
            &repo_root_for_update,
            Some(branch_name_for_update.as_str()),
          ))
        })
        .await;

      let _ = this.update(cx, |this, cx| {
        this.local_project_update_task = None;
        this.local_project_update_loading = false;

        match result {
          Ok(snapshot) => {
            this.local_project_update_error = None;
            if let Some(snapshot) = snapshot {
              this.sync_active_local_repo_store_snapshot(&snapshot, cx);
              this.sync_resolved_local_repo_snapshot(&snapshot);
            }
            this.load_local_project_files(repo_root.clone(), cx);
            if let Some(post_action) = post_action.clone() {
              this.execute_local_project_post_action(post_action, repo_root.clone(), cx);
            }
          }
          Err(error) => {
            this.local_project_update_error = Some(error.to_string().into());
          }
        }

        cx.notify();
      });
    });

    self.local_project_update_task = Some(task);
  }

  pub(super) fn update_local_branch_to_pr_head(
    &mut self,
    stash_before_update: bool,
    post_action: Option<GithubPrLocalProjectPostAction>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let repo_root = match self.local_project_availability(cx) {
      GithubPrLocalProjectAvailability::NeedsUpdate { repo_root }
      | GithubPrLocalProjectAvailability::Dirty { repo_root }
      | GithubPrLocalProjectAvailability::Ready { repo_root } => repo_root,
      _ => return,
    };

    let _ = window;
    self.start_sync_local_branch_to_pr_head(repo_root, stash_before_update, post_action, cx);
  }

  pub(super) fn local_project_command_palette_commands(
    availability: &GithubPrLocalProjectAvailability,
  ) -> Vec<CommandPaletteCommand> {
    if matches!(
      availability,
      GithubPrLocalProjectAvailability::NeedsBranchSwitch { .. }
    ) {
      vec![CommandPaletteCommand::switch_to_pr_branch()]
    } else {
      Vec::new()
    }
  }

  pub(super) fn render_local_project_controls(
    &self,
    theme: &gpui_component::Theme,
    cx: &mut Context<Self>,
    show_ready_toggle: bool,
  ) -> Option<AnyElement> {
    let availability = self.local_project_availability(cx);
    if matches!(availability, GithubPrLocalProjectAvailability::Hidden) {
      return None;
    }

    if matches!(availability, GithubPrLocalProjectAvailability::Ready { .. }) {
      if !show_ready_toggle {
        return None;
      }
      let local_project_mode = self.local_project_mode_active(cx);
      return Some(
        Switch::new("github-pr-local-project-switch")
          .label("Show unchanged files")
          .small()
          .checked(local_project_mode)
          .disabled(self.local_project_update_loading || self.local_branch_switch_loading)
          .on_click(cx.listener(move |this, checked, _, cx| {
            this.set_show_local_project_files(*checked, cx);
          }))
          .into_any_element(),
      );
    }

    let (status_text, action_button): (Option<String>, Option<Button>) = match &availability {
      GithubPrLocalProjectAvailability::NeedsBranchSwitch {
        current_branch,
        has_uncommitted_changes,
        ..
      } => {
        let text = match (current_branch, has_uncommitted_changes) {
          (Some(branch), true) => Some(format!("Local changes detected on {}.", branch)),
          (Some(_), false) => None,
          (None, true) => Some("Local changes detected.".to_string()),
          (None, false) => Some("Local repo is not on this PR branch.".to_string()),
        };
        let view = cx.entity();
        let button_label = if self.local_branch_switch_loading {
          "Switching..."
        } else if *has_uncommitted_changes {
          "Stash and switch to PR branch"
        } else {
          "Switch to PR branch"
        };
        let button = Button::new("github-pr-local-project-switch-branch")
          .label(button_label)
          .xsmall()
          .ghost()
          .disabled(self.local_branch_switch_loading || self.local_project_update_loading)
          .on_click(move |_, window, cx| {
            view.update(cx, |this, cx| {
              this.prompt_or_switch_local_branch_to_pr_branch(window, cx);
            });
          });
        (text, Some(button))
      }
      GithubPrLocalProjectAvailability::NeedsUpdate { .. } => {
        let view = cx.entity();
        let button = Button::new("github-pr-local-project-update")
          .label(if self.local_project_update_loading {
            "Updating..."
          } else {
            "Update to PR head"
          })
          .xsmall()
          .ghost()
          .disabled(self.local_project_update_loading)
          .on_click(move |_, window, cx| {
            view.update(cx, |this, cx| {
              this.update_local_branch_to_pr_head(false, None, window, cx);
            });
          });
        (
          Some("Local branch is not at this PR head.".to_string()),
          Some(button),
        )
      }
      GithubPrLocalProjectAvailability::Dirty { .. } => (
        Some("Local branch is not at this PR head and has local changes.".to_string()),
        None,
      ),
      _ => (None, None),
    };

    Some(
      h_flex()
        .items_center()
        .gap_2()
        .when_some(action_button, |this, button| this.child(button))
        .when_some(status_text, |this, text| {
          this.child(
            div()
              .text_xs()
              .text_color(theme.status_orange())
              .child(text),
          )
        })
        .into_any_element(),
    )
  }

  pub(super) fn render_local_project_file_header(
    &self,
    file: &GithubPrLocalProjectFile,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let path = Path::new(file.path.as_ref());
    let mut toolbar = DiffToolbar::new("pr-local-project")
      .filled(true)
      .title(render_file_title_with_status(path, None, None, false, cx));

    if is_markdown_path(path) || is_svg_path(path) {
      let view = cx.entity();
      toolbar = toolbar.preview(ToggleControl {
        active: self.show_markdown_preview,
        disabled: self.file_loading,
        debug_selector: PR_PREVIEW_TOGGLE_DEBUG_SELECTOR,
        on_toggle: Rc::new(move |_, cx| {
          view.update(cx, |this, cx| this.toggle_markdown_preview(cx));
        }),
      });
    }

    toolbar.render(cx)
  }
}
