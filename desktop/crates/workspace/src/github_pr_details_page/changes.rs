//! The Changes tab: which files the pull request touches, the tree that lists
//! them, and getting the selected one into the diff editor.

use super::*;

impl GithubPrDetailsPage {
  pub(super) fn build_detached_diff_editor(
    path: impl Into<PathBuf>,
    cx: &mut Context<Self>,
  ) -> Entity<Editor> {
    let editor_path = path.into();
    let load_root = PathBuf::from(".");
    let load_path = PathBuf::from(".reviu-github-pr-preview").join(&editor_path);
    let loaded = Editor::load_file_for_editor(&load_root, &load_path);
    let detached_root = PathBuf::from(".reviu-github-pr-editor-root");

    cx.new(move |cx| {
      let mut editor = Editor::new_with_loaded_file(detached_root, editor_path, loaded, cx);
      editor.is_read_only = true;
      editor
    })
  }

  pub(super) fn tree_search_query_normalized(&self) -> Option<String> {
    let query = self.tree_search_query.trim();
    (!query.is_empty()).then(|| query.to_lowercase())
  }

  pub(super) fn search_scope_paths(&self, cx: &App) -> Vec<String> {
    if self.local_project_mode_active(cx) {
      let mut paths = BTreeSet::new();
      paths.extend(self.local_project_lookup.keys().cloned());
      paths.extend(self.file_lookup.keys().cloned());
      return paths.into_iter().collect();
    }

    let mut paths = self.file_lookup.keys().cloned().collect::<Vec<_>>();
    paths.sort();
    paths
  }

  pub(super) fn visible_tree_paths(&self, cx: &App) -> Vec<String> {
    let mut paths = self.search_scope_paths(cx);
    if self.tree_search_query_normalized().is_some()
      && let Some(matches) = self.tree_search_matches.as_ref()
    {
      paths.retain(|path| matches.contains(path));
    }
    paths
  }

  pub(super) fn active_file_count(&self, cx: &App) -> usize {
    self.visible_tree_paths(cx).len()
  }

  pub(super) fn active_file_search_entries(&self, cx: &App) -> Vec<SearchFileEntry> {
    let mut entries = self
      .visible_tree_paths(cx)
      .into_iter()
      .map(|path| build_search_file_entry(path.as_str()))
      .collect::<Vec<_>>();
    entries.sort_by(|a, b| a.label.as_ref().cmp(b.label.as_ref()));
    entries
  }

  pub(super) fn current_selected_tree_path(&self) -> Option<String> {
    self
      .selected_file
      .as_ref()
      .map(|file| file.path.to_string())
      .or_else(|| {
        self
          .selected_local_project_file
          .as_ref()
          .map(|file| file.path.to_string())
      })
  }

  pub(super) fn select_visible_tree_path(&mut self, path: &str, cx: &mut Context<Self>) {
    if let Some(file) = self.file_lookup.get(path).cloned() {
      self.set_selected_file(Some(file), cx);
      return;
    }

    if let Some(file) = self.local_project_lookup.get(path).cloned() {
      self.set_selected_local_project_file(Some(file), cx);
      return;
    }

    if self.selected_file.is_some() {
      self.set_selected_file(None, cx);
    } else if self.selected_local_project_file.is_some() {
      self.set_selected_local_project_file(None, cx);
    }
  }

  pub(super) fn set_tree_items_with_selection(
    &mut self,
    items: Vec<TreeItem>,
    preferred_id: Option<String>,
    fallback_index: Option<usize>,
    cx: &mut Context<Self>,
  ) -> Option<String> {
    let mut resolved_id = None;
    self.tree_state.update(cx, |state, cx| {
      state.set_items(items, cx);

      if let Some(preferred_id) = preferred_id.as_ref() {
        let tree_item = TreeItem::new(preferred_id.clone(), preferred_id.clone());
        state.set_selected_item(Some(&tree_item), cx);
        if let Some(ix) = state.selected_index() {
          state.scroll_to_item(ix, gpui::ScrollStrategy::Top);
          resolved_id = Some(preferred_id.clone());
          return;
        }
      }

      state.set_selected_index(fallback_index, cx);
      if let Some(ix) = state.selected_index() {
        state.scroll_to_item(ix, gpui::ScrollStrategy::Top);
        resolved_id = state
          .selected_entry()
          .map(|entry| entry.item().id.to_string());
      }
    });
    resolved_id
  }

  pub(super) fn sync_changes_tree_state(&mut self, cx: &mut Context<Self>) {
    let visible_paths = self.visible_tree_paths(cx);
    let expanded_folder_paths = self.local_project_mode_active(cx).then(|| {
      expanded_folder_paths_for_changed_files(self.file_lookup.keys().map(|path| path.as_str()))
    });
    let (items, fallback_index, fallback_id) =
      build_tree_items_from_paths(&visible_paths, expanded_folder_paths.as_ref());
    let preferred_id = self
      .saved_pr_selected_tree_id
      .clone()
      .or_else(|| self.current_selected_tree_path())
      .filter(|id| visible_paths.contains(id))
      .or(fallback_id);
    let resolved_id = self.set_tree_items_with_selection(items, preferred_id, fallback_index, cx);
    self.saved_pr_selected_tree_id = None;
    match resolved_id {
      Some(path) => self.select_visible_tree_path(path.as_str(), cx),
      None => {
        self.selected_tree_id = None;
        self.selected_local_project_tree_id = None;
        if self.selected_file.is_some() {
          self.set_selected_file(None, cx);
        } else if self.selected_local_project_file.is_some() {
          self.set_selected_local_project_file(None, cx);
        }
      }
    }
  }

  pub(super) fn sync_tree_selection(&mut self, cx: &mut Context<Self>) {
    if self.show_local_project_files && self.selected_local_project_file.is_some() {
      self.sync_local_project_tree_selection(cx);
      return;
    }

    let Some(file) = self.selected_file.as_ref() else {
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

  pub(super) fn refresh_tree_text_search(&mut self, cx: &mut Context<Self>) {
    self.tree_search_generation = self.tree_search_generation.wrapping_add(1);
    let generation = self.tree_search_generation;
    self.tree_search_task = None;
    self.tree_search_error = None;

    let Some(query) = self.tree_search_query_normalized() else {
      self.tree_search_loading = false;
      self.tree_search_matches = None;
      self.sync_changes_tree_state(cx);
      cx.notify();
      return;
    };

    let scope_paths = self.search_scope_paths(cx);
    if scope_paths.is_empty() {
      self.tree_search_loading = false;
      self.tree_search_matches = Some(HashSet::new());
      self.sync_changes_tree_state(cx);
      cx.notify();
      return;
    }

    let pr_files = scope_paths
      .iter()
      .filter_map(|path| {
        self
          .file_lookup
          .get(path)
          .map(|file| (path.clone(), file.as_ref().clone()))
      })
      .collect::<HashMap<_, _>>();
    let cached_file_contents = self.file_contents.clone();
    let diff_refs = self.resolve_diff_refs();
    let api = self.api.clone();
    let local_repo_root = self
      .local_project_mode_active(cx)
      .then(|| self.local_project_loaded_repo_root.clone())
      .flatten();
    let previous_matches = self.tree_search_matches.clone();

    self.tree_search_loading = true;
    if let Some(previous_matches) = previous_matches {
      self.tree_search_matches = Some(previous_matches);
    }
    self.sync_changes_tree_state(cx);
    cx.notify();

    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move {
          perform_tree_text_search(
            &query,
            &scope_paths,
            &pr_files,
            &cached_file_contents,
            diff_refs.as_ref(),
            &api,
            local_repo_root.as_deref(),
          )
        })
        .await;

      let _ = this.update(cx, |this, cx| {
        if generation != this.tree_search_generation {
          return;
        }

        this.tree_search_task = None;
        this.tree_search_loading = false;
        this.tree_search_error = result.error.map(Into::into);
        for (path, contents) in result.updated_file_contents {
          this.file_contents.entry(path).or_insert(contents);
        }
        this.tree_search_matches = Some(result.matches);
        this.sync_changes_tree_state(cx);
        cx.notify();
      });
    });

    self.tree_search_task = Some(task);
  }

  pub(super) fn selected_commit_index(&self) -> Option<usize> {
    let sha = self.selected_commit_sha.as_deref()?;
    self.commits.iter().position(|commit| commit.sha == sha)
  }

  pub(super) fn enter_commit_by_commit_review(&mut self, cx: &mut Context<Self>) {
    // Commits are sorted newest-first, so start at the oldest (last entry).
    let Some(first_sha) = self.commits.last().map(|commit| commit.sha.clone()) else {
      return;
    };
    self.select_commit_filter(Some(first_sha), cx);
  }

  pub(super) fn exit_commit_by_commit_review(&mut self, cx: &mut Context<Self>) {
    self.select_commit_filter(None, cx);
  }

  pub(super) fn navigate_commit_by_commit(
    &mut self,
    direction: CommitNavigationDirection,
    cx: &mut Context<Self>,
  ) {
    let Some(current_index) = self.selected_commit_index() else {
      return;
    };
    // Commits are newest-first; "Next" = newer (index - 1), "Previous" = older (index + 1).
    let new_index = match direction {
      CommitNavigationDirection::Next => current_index.checked_sub(1),
      CommitNavigationDirection::Previous => {
        let next = current_index + 1;
        if next < self.commits.len() {
          Some(next)
        } else {
          None
        }
      }
    };
    if let Some(new_index) = new_index
      && let Some(commit) = self.commits.get(new_index)
    {
      let sha = commit.sha.clone();
      self.select_commit_filter(Some(sha), cx);
    }
  }

  pub(super) fn select_commit_filter(
    &mut self,
    selected_commit_sha: Option<String>,
    cx: &mut Context<Self>,
  ) {
    if self.selected_commit_sha == selected_commit_sha {
      return;
    }
    let should_disable_local_project =
      selected_commit_sha.is_some() && self.show_local_project_files;
    self.selected_commit_sha = selected_commit_sha;
    if should_disable_local_project {
      self.set_show_local_project_files(false, cx);
    }
    self.sync_review_comment_handlers(cx);
    self.sync_sentry_pr_context();
    self.reload_files_for_current_pull_request(cx);
  }

  pub(super) fn set_active_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
    self.set_active_tab_inner(ix, window, cx, true);
  }

  pub(super) fn set_active_tab_inner(
    &mut self,
    ix: usize,
    window: &mut Window,
    cx: &mut Context<Self>,
    sync_url: bool,
  ) {
    self.active_tab_ix = ix;
    self.sync_sentry_pr_context();
    let mut data = Map::new();
    data.insert("active_tab".into(), ix.into());
    self.add_pr_breadcrumb("Changed PR tab", data);
    cx.notify();

    if sync_url && let Some(ctx) = &self.current_pr_context {
      let tab_segment = pr_tab_url_segment(ix);
      let path = if tab_segment.is_empty() {
        crate::navigation::build_pr_path(&ctx.owner, &ctx.repo, ctx.number)
      } else {
        crate::navigation::build_pr_tab_path(&ctx.owner, &ctx.repo, ctx.number, tab_segment)
      };
      NavigationHistory::navigate_replace(path, cx);
    }

    if ix == PR_TAB_CHANGES_IX {
      let app_settings = crate::config::AppSettings::get(cx);
      let saved_mode = if app_settings.split_diff_view {
        DiffViewMode::Split
      } else {
        DiffViewMode::Inline
      };
      if self.diff_view != saved_mode {
        self.diff_view = saved_mode;
        self.sync_diff_view(cx);
      }
      self.hide_whitespace = app_settings.hide_whitespace;
      self.sync_tree_selection(cx);
      self.focus_changes_tree(window, cx);
      cx.on_next_frame(window, |this, window, cx| {
        if this.active_tab_ix == PR_TAB_CHANGES_IX {
          this.focus_changes_tree(window, cx);
        }
      });
    } else {
      self.refocus_page_shortcuts(window, cx);
    }
  }

  pub(super) fn focus_changes_tree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self.tree_state.update(cx, |state, cx| {
      state.focus(window, cx);
    });
  }

  pub(super) fn focus_changes_tree_via_window_handle(&self, cx: &mut App) {
    if self.active_tab_ix != PR_TAB_CHANGES_IX {
      return;
    }
    let tree = self.tree_state.clone();
    let _ = cx.update_window(self.window_handle, move |_, window, cx| {
      tree.update(cx, |state, cx| {
        state.focus(window, cx);
      });
    });
  }

  pub(super) fn set_selected_file(
    &mut self,
    selected: Option<Rc<GithubPrFileDiff>>,
    cx: &mut Context<Self>,
  ) {
    let current_id = self.selected_file.as_ref().map(|file| file.path.clone());
    let next_id = selected.as_ref().map(|file| file.path.clone());
    if current_id == next_id {
      return;
    }

    self.selected_file = selected.clone();
    self.selected_tree_id = selected.as_ref().map(|file| file.path.to_string());
    self.selected_local_project_file = None;
    self.selected_local_project_tree_id = None;
    self.local_project_open_file_task = None;
    self.local_project_open_file_generation =
      self.local_project_open_file_generation.wrapping_add(1);
    self.active_review_comment_id = None;
    self.selected_file_review_comment_ids.clear();
    self.sync_sentry_pr_context();
    let mut data = Map::new();
    if let Some(file) = self.selected_file.as_ref() {
      data.insert("selected_file".into(), file.path.to_string().into());
    }
    self.add_pr_breadcrumb("Selected PR file changed", data);
    if !self.selected_file_is_markdown() && !self.selected_file_is_svg() {
      self.show_markdown_preview = false;
    }
    self.binary_preview = None;
    self.svg_preview.update(cx, |preview, _| preview.clear());

    if let Some(file) = selected {
      self.ensure_diff_editor_for_path(file.path.as_ref(), cx);
      self.sync_diff_view(cx);
      self.sync_tree_selection(cx);
      let key = file.path.to_string();
      let path = Path::new(file.path.as_ref());
      match file_preview_kind(path) {
        Some(FilePreviewKind::RasterImage(_)) => {
          self.file_error = None;
          self.clear_diff_editor(cx);
          if let Some(preview) = self.file_asset_previews.get(&key).cloned() {
            self.binary_preview = Some(preview);
            self.file_loading = false;
          } else {
            self.file_loading = true;
            self.maybe_fetch_file_asset(file, cx);
          }
        }
        Some(FilePreviewKind::UnsupportedBinary) => {
          self.file_loading = false;
          self.file_error = None;
          self.binary_preview = Some(GithubPrBinaryPreview::UnsupportedBinary);
          self.clear_diff_editor(cx);
        }
        _ => {
          let cached = self.file_contents.contains_key(&key);
          let in_flight = self.file_content_tasks.contains_key(&key);
          let _ = (cached, in_flight);
          if let Some(contents) = self.file_contents.get(&key).cloned() {
            if contents.base.is_none() && contents.head.is_none() {
              self.file_loading = false;
              self.file_error = Some("File contents unavailable".into());
              self.clear_diff_editor(cx);
            } else {
              self.file_loading = false;
              self.file_error = None;
              self.apply_full_diff(&file, &contents, cx);
            }
          } else {
            self.file_loading = true;
            self.file_error = None;
            self.clear_diff_editor(cx);
            self.maybe_fetch_file_contents(file, cx);
          }
        }
      }
    } else {
      self.file_loading = false;
      self.file_error = None;
      self.clear_diff_editor(cx);
    }

    self.sync_review_comments(cx);
    cx.notify();
  }

  pub(super) fn ensure_diff_editor_for_path(&mut self, path: &str, cx: &mut Context<Self>) {
    let desired_path = PathBuf::from(path);
    let mut current_path = None;
    self.diff_editor.update(cx, |editor, _| {
      current_path = Some(editor.workdir_path.clone());
    });
    if current_path.as_ref() == Some(&desired_path) {
      return;
    }

    self.diff_editor = Self::build_detached_diff_editor(desired_path, cx);
    self.install_diff_editor_review_comment_handlers(cx);
  }

  pub(super) fn clear_diff_editor(&mut self, cx: &mut Context<Self>) {
    self.diff_editor.update(cx, |editor, cx| {
      editor.document().update(cx, |doc, cx| {
        doc.replace_all("", cx);
      });
      editor.reset_after_replace();
      editor.reset_selection(cx);
      editor.set_diffs(None, cx);
      editor.is_read_only = true;
    });
  }

  pub(super) fn split_disabled_for_file(&self, file: &GithubPrFileDiff) -> bool {
    matches!(
      file.status,
      GithubPrFileStatus::Added | GithubPrFileStatus::Deleted
    )
  }

  pub(super) fn split_disabled_for_selected_file(&self) -> bool {
    if self.binary_preview.is_some() {
      return true;
    }

    if self.show_local_project_files && self.selected_local_project_file.is_some() {
      return true;
    }

    self
      .selected_file
      .as_ref()
      .is_some_and(|file| self.split_disabled_for_file(file))
  }

  pub(super) fn selected_file_is_markdown(&self) -> bool {
    if self.show_local_project_files && self.selected_local_project_file.is_some() {
      return self.selected_local_project_file_is_markdown();
    }

    self
      .selected_file
      .as_ref()
      .map(|file| is_markdown_path(Path::new(file.path.as_ref())))
      .unwrap_or(false)
  }

  pub(super) fn selected_file_is_svg(&self) -> bool {
    if self.show_local_project_files && self.selected_local_project_file.is_some() {
      return self.selected_local_project_file_is_svg();
    }

    self
      .selected_file
      .as_ref()
      .map(|file| is_svg_path(Path::new(file.path.as_ref())))
      .unwrap_or(false)
  }

  pub(super) fn build_binary_preview(
    path: &Path,
    binary_bytes: Option<Vec<u8>>,
  ) -> Option<GithubPrBinaryPreview> {
    if let Some(bytes) = binary_bytes {
      if let Some(image) = raster_image_from_bytes(path, bytes.clone()) {
        return Some(GithubPrBinaryPreview::RasterImage(image));
      }
      if should_show_unsupported_binary_placeholder(path, Some(bytes.as_slice())) {
        return Some(GithubPrBinaryPreview::UnsupportedBinary);
      }
      return None;
    }

    if matches!(
      file_preview_kind(path),
      Some(FilePreviewKind::UnsupportedBinary)
    ) {
      Some(GithubPrBinaryPreview::UnsupportedBinary)
    } else {
      None
    }
  }

  pub(super) fn render_binary_preview_content(
    &self,
    preview: &GithubPrBinaryPreview,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let theme = cx.theme().clone();

    match preview {
      GithubPrBinaryPreview::RasterImage(image) => {
        let loading_color = theme.muted_foreground;
        let error_color = theme.status_red();
        let image_el = img(image.clone())
          .max_w_full()
          .max_h_full()
          .object_fit(ObjectFit::Contain)
          .with_loading(move || {
            render_image_preview_status_message("Rendering image preview...", loading_color)
          })
          .with_fallback(move || {
            render_image_preview_status_message("Unable to render image preview", error_color)
          });

        div()
          .flex_1()
          .min_h_0()
          .min_w(px(0.0))
          .overflow_hidden()
          .bg(theme.background)
          .debug_selector(|| GITHUB_PR_BINARY_PREVIEW_RENDER_DEBUG_SELECTOR.to_string())
          .child(
            div().relative().size_full().child(
              div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .bottom_0()
                .p_4()
                .flex()
                .items_center()
                .justify_center()
                .child(image_el),
            ),
          )
          .into_any_element()
      }
      GithubPrBinaryPreview::UnsupportedBinary => div()
        .flex_1()
        .min_h_0()
        .min_w(px(0.0))
        .bg(theme.background)
        .debug_selector(|| GITHUB_PR_BINARY_PREVIEW_RENDER_DEBUG_SELECTOR.to_string())
        .child(
          v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .child(
              Icon::new(IconName::File)
                .size_6()
                .text_color(theme.muted_foreground),
            )
            .child(
              div()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child("Binary file preview is not available."),
            ),
        )
        .into_any_element(),
    }
  }

  pub(super) fn effective_diff_view(&self) -> DiffViewMode {
    effective_diff_view(DiffViewInputs {
      preferred: self.diff_view,
      binary_preview: self.binary_preview.is_some(),
      previewing: self.show_markdown_preview
        && (self.selected_file_is_markdown() || self.selected_file_is_svg()),
      whole_file_change: self.split_disabled_for_selected_file(),
    })
  }

  pub(super) fn sync_diff_view(&mut self, cx: &mut Context<Self>) {
    let diff_view = self.effective_diff_view();
    self
      .diff_editor
      .update(cx, |editor, cx| editor.set_diff_view_mode(diff_view, cx));
  }

  pub(super) fn toggle_diff_view(&mut self, cx: &mut Context<Self>) {
    if self.split_disabled_for_selected_file() {
      return;
    }
    if self.show_markdown_preview
      && (self.selected_file_is_markdown() || self.selected_file_is_svg())
    {
      return;
    }

    self.diff_view = match self.diff_view {
      DiffViewMode::Inline => DiffViewMode::Split,
      DiffViewMode::Split => DiffViewMode::Inline,
    };
    AppSettings::update(cx, |s| {
      s.split_diff_view = self.diff_view == DiffViewMode::Split
    });
    self.sync_diff_view(cx);
    cx.notify();
  }

  pub(super) fn toggle_hide_whitespace(&mut self, cx: &mut Context<Self>) {
    self.hide_whitespace = !self.hide_whitespace;
    // Recompute diff without resetting scroll or selection
    if let Some(path) = self.current_selected_tree_path()
      && let Some(file) = self.file_lookup.get(&path).cloned()
      && let Some(contents) = self.file_contents.get(&path).cloned()
    {
      let head = contents.head.as_deref().unwrap_or("");
      let base = contents.base.as_deref();
      if let Ok(diff) = compute_buffer_diff(
        DiffKind::Uncommitted,
        base,
        head,
        Path::new(file.path.as_ref()),
        self.hide_whitespace,
      ) {
        let diff_set = Some(DiffSet {
          uncommitted: diff,
          unstaged: FileDiff {
            kind: DiffKind::Unstaged,
            hunks: Vec::new(),
          },
          staged: FileDiff {
            kind: DiffKind::Staged,
            hunks: Vec::new(),
          },
        });
        self.diff_editor.update(cx, |editor, cx| {
          editor.set_diffs(diff_set, cx);
        });
      }
    }
    cx.notify();
  }

  pub(super) fn toggle_markdown_preview(&mut self, cx: &mut Context<Self>) {
    if !self.selected_file_is_markdown() && !self.selected_file_is_svg() {
      return;
    }

    self.show_markdown_preview = !self.show_markdown_preview;
    self.sync_diff_view(cx);
    cx.notify();
  }

  pub(super) fn apply_full_diff(
    &mut self,
    file: &GithubPrFileDiff,
    contents: &GithubPrFileContents,
    cx: &mut Context<Self>,
  ) {
    self.file_loading = false;
    self.file_error = None;
    let head = contents.head.as_deref().unwrap_or("");
    let base = contents.base.as_deref();
    let _ = (
      file.path.clone(),
      contents.base.as_ref().map(|value| value.len()),
      head.len(),
    );
    let diff = compute_buffer_diff(
      DiffKind::Uncommitted,
      base,
      head,
      Path::new(file.path.as_ref()),
      self.hide_whitespace,
    )
    .ok();
    let Some(diff) = diff else {
      self.file_error = Some("Unable to compute diff".into());
      self.file_loading = false;
      return;
    };
    let diff_set = Some(DiffSet {
      uncommitted: diff,
      unstaged: FileDiff {
        kind: DiffKind::Unstaged,
        hunks: Vec::new(),
      },
      staged: FileDiff {
        kind: DiffKind::Staged,
        hunks: Vec::new(),
      },
    });

    self.diff_editor.update(cx, |editor, cx| {
      let _ = (
        editor.document().read(cx).len(),
        editor.document().read(cx).len_lines(),
      );
      editor.document().update(cx, |doc, cx| {
        doc.replace_all(head, cx);
      });
      editor.reset_after_replace();
      let _ = (
        editor.document().read(cx).len(),
        editor.document().read(cx).len_lines(),
      );
      editor.reset_selection(cx);
      editor.set_diffs(diff_set, cx);
      editor.is_read_only = true;
    });
    self.sync_diff_view(cx);
    self.resolve_pending_review_comment_link(cx);
  }

  pub(super) fn selected_commit(&self) -> Option<&GithubPullRequestCommit> {
    self
      .selected_commit_sha
      .as_ref()
      .and_then(|sha| self.commit_lookup.get(sha))
  }

  pub(super) fn resolve_diff_refs(&self) -> Option<GithubPrDiffRefs> {
    let pull_request = self.pull_request.as_ref()?;
    let base_owner = pull_request.repository.owner.clone();
    let base_repo = pull_request.repository.repo.clone();
    let head_owner = pull_request
      .head_repository
      .as_ref()
      .map(|repo| repo.owner.clone())
      .unwrap_or_else(|| base_owner.clone());
    let head_repo = pull_request
      .head_repository
      .as_ref()
      .map(|repo| repo.repo.clone())
      .unwrap_or_else(|| base_repo.clone());
    let selected_commit = self.selected_commit();
    let (resolved_base_sha, resolved_head_sha) = resolve_diff_shas_for_context(
      pull_request.merge_base_sha.as_str(),
      pull_request.base_sha.as_str(),
      pull_request.head_sha.as_str(),
      selected_commit.map(|commit| commit.sha.as_str()),
      selected_commit.and_then(|commit| commit.parent_sha.as_deref()),
    )?;

    if selected_commit.is_some() {
      return Some(GithubPrDiffRefs {
        base_owner: head_owner.clone(),
        base_repo: head_repo.clone(),
        base_sha: resolved_base_sha,
        head_owner,
        head_repo,
        head_sha: resolved_head_sha,
      });
    }

    Some(GithubPrDiffRefs {
      base_owner,
      base_repo,
      base_sha: resolved_base_sha,
      head_owner,
      head_repo,
      head_sha: resolved_head_sha,
    })
  }

  pub(super) fn reset_files_state(&mut self, cx: &mut Context<Self>) {
    self.file_loading = false;
    self.file_error = None;
    self.files_error = None;
    self.tree_state.update(cx, |state, cx| {
      state.set_items(Vec::new(), cx);
    });
    self.file_lookup.clear();
    self.file_contents.clear();
    self.file_content_tasks.clear();
    self.file_asset_previews.clear();
    self.file_asset_tasks.clear();
    self.binary_preview = None;
    self.selected_tree_id = None;
    self.set_selected_file(None, cx);
    self.sync_review_comments(cx);
  }

  pub(super) fn reload_files_for_current_pull_request(&mut self, cx: &mut Context<Self>) {
    let Some(context) = self.current_pr_context.as_ref().cloned() else {
      return;
    };
    self.files_loading = true;
    self.reset_files_state(cx);
    self.fetch_pull_request_files_for_context(context.owner, context.repo, context.number, cx);
    cx.notify();
  }

  pub(super) fn fetch_pull_request_files_for_context(
    &mut self,
    owner: String,
    repo: String,
    number: u64,
    cx: &mut Context<Self>,
  ) {
    self.files_request_generation = self.files_request_generation.wrapping_add(1);
    let generation = self.files_request_generation;
    let files_api = self.api.clone();
    let commit_sha = self.selected_commit_sha.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move {
          files_api.fetch_pull_request_files(&owner, &repo, number, commit_sha.as_deref())
        })
        .await;

      let _ = this.update(cx, |this, cx| {
        if generation != this.files_request_generation {
          return;
        }
        match result {
          Ok(files) => {
            this.files_loading = false;
            this.files_error = None;
            let files = files_from_api(files);
            let (_, lookup, _, _) = build_tree_items(&files);
            this.file_lookup = lookup;
            this.refresh_tree_text_search(cx);
            this.prefetch_overview_root_review_comment_files(cx);
            this.add_pr_breadcrumb("Load PR files succeeded", Map::new());
          }
          Err(error) => {
            let error_message = error.to_string();
            this.files_loading = false;
            this.files_error = Some(error_message.clone().into());
            this.file_lookup.clear();
            this.file_contents.clear();
            this.file_content_tasks.clear();
            this.refresh_tree_text_search(cx);
            this.add_pr_breadcrumb("Load PR files failed", Map::new());
            this.record_pr_error("github.pr.files", error_message.as_str(), Map::new());
          }
        }
        cx.notify();
      });
    });
    self.files_task = Some(task);
  }

  pub(super) fn maybe_fetch_selected_file_contents(&mut self, cx: &mut Context<Self>) {
    if let Some(file) = self.selected_file.clone() {
      match file_preview_kind(Path::new(file.path.as_ref())) {
        Some(FilePreviewKind::RasterImage(_)) => self.maybe_fetch_file_asset(file, cx),
        Some(FilePreviewKind::UnsupportedBinary) => {}
        _ => self.maybe_fetch_file_contents(file, cx),
      }
    }
  }

  pub(super) fn maybe_fetch_file_asset(
    &mut self,
    file: Rc<GithubPrFileDiff>,
    cx: &mut Context<Self>,
  ) {
    let key = file.path.to_string();
    let key_for_task = key.clone();
    if self.file_asset_previews.contains_key(&key) || self.file_asset_tasks.contains_key(&key) {
      return;
    }

    let Some(diff_refs) = self.resolve_diff_refs() else {
      return;
    };
    let (owner, repo, reference, preview_path) = match file.status {
      GithubPrFileStatus::Deleted => (
        diff_refs.base_owner,
        diff_refs.base_repo,
        diff_refs.base_sha,
        file
          .old_path
          .as_ref()
          .map(|path| path.to_string())
          .unwrap_or_else(|| file.path.to_string()),
      ),
      _ => (
        diff_refs.head_owner,
        diff_refs.head_repo,
        diff_refs.head_sha,
        file.path.to_string(),
      ),
    };

    let api = self.api.clone();
    let preview_path_for_request = preview_path.clone();
    let task = cx.spawn(async move |this, cx| {
      let asset_result = cx
        .background_spawn(async move {
          api.fetch_github_file_asset(&owner, &repo, &preview_path_for_request, &reference)
        })
        .await;

      let _ = this.update(cx, |this, cx| {
        this.file_asset_tasks.remove(&key_for_task);
        let is_selected_file = this.selected_tree_id.as_deref() == Some(key_for_task.as_str());

        match asset_result {
          Ok(Some(bytes)) => {
            let preview = Self::build_binary_preview(Path::new(preview_path.as_str()), Some(bytes));
            let Some(preview) = preview else {
              if is_selected_file {
                this.file_loading = false;
                this.file_error = Some("Unable to render file preview".into());
              }
              cx.notify();
              return;
            };
            this
              .file_asset_previews
              .insert(key_for_task.clone(), preview.clone());
            if is_selected_file {
              this.binary_preview = Some(preview);
              this.file_loading = false;
              this.file_error = None;
            }
          }
          Ok(None) => {
            if is_selected_file {
              this.file_loading = false;
              this.file_error = Some("File preview unavailable".into());
            }
          }
          Err(error) => {
            if is_selected_file {
              this.file_loading = false;
              this.file_error = Some(error.to_string().into());
            }
          }
        }
        cx.notify();
      });
    });

    self.file_asset_tasks.insert(key, task);
  }

  pub(super) fn maybe_fetch_file_contents(
    &mut self,
    file: Rc<GithubPrFileDiff>,
    cx: &mut Context<Self>,
  ) {
    match file_preview_kind(Path::new(file.path.as_ref())) {
      Some(FilePreviewKind::RasterImage(_)) | Some(FilePreviewKind::UnsupportedBinary) => return,
      _ => {}
    }

    let key = file.path.to_string();
    let key_for_task = key.clone();
    if self.file_contents.contains_key(&key) || self.file_content_tasks.contains_key(&key) {
      return;
    }

    let Some(diff_refs) = self.resolve_diff_refs() else {
      return;
    };
    let base_owner = diff_refs.base_owner;
    let base_repo = diff_refs.base_repo;
    let base_sha = diff_refs.base_sha;
    let head_owner = diff_refs.head_owner;
    let head_repo = diff_refs.head_repo;
    let head_sha = diff_refs.head_sha;

    let base_path = match file.status {
      GithubPrFileStatus::Added => None,
      GithubPrFileStatus::Renamed => file
        .old_path
        .as_ref()
        .map(|path| path.to_string())
        .or_else(|| Some(file.path.to_string())),
      _ => Some(file.path.to_string()),
    };
    let head_path = match file.status {
      GithubPrFileStatus::Deleted => None,
      _ => Some(file.path.to_string()),
    };

    let api = self.api.clone();
    let task = cx.spawn(async move |this, cx| {
      let base_result = if let Some(path) = base_path.clone() {
        let api = api.clone();
        let owner = base_owner.clone();
        let repo = base_repo.clone();
        let base_sha = base_sha.clone();
        cx.background_spawn(async move {
          api.fetch_github_file_content(&owner, &repo, &path, &base_sha)
        })
        .await
      } else {
        Ok(None)
      };

      let head_result = if let Some(path) = head_path.clone() {
        let api = api.clone();
        let owner = head_owner.clone();
        let repo = head_repo.clone();
        let head_sha = head_sha.clone();
        cx.background_spawn(async move {
          api.fetch_github_file_content(&owner, &repo, &path, &head_sha)
        })
        .await
      } else {
        Ok(None)
      };

      let _ = this.update(cx, |this, cx| {
        this.file_content_tasks.remove(&key_for_task);
        let is_selected_file = this.selected_tree_id.as_deref() == Some(key_for_task.as_str());
        let (base, head) = match (base_result, head_result) {
          (Ok(base), Ok(head)) => (base, head),
          _ => {
            if is_selected_file {
              this.file_loading = false;
              this.file_error = Some("Failed to load file contents".into());
            }
            cx.notify();
            return;
          }
        };

        if base.is_none() && head.is_none() {
          if is_selected_file {
            this.file_loading = false;
            this.file_error = Some("File contents unavailable".into());
          }
          this
            .file_contents
            .insert(key_for_task.clone(), GithubPrFileContents { base, head });
          cx.notify();
          return;
        }

        this
          .file_contents
          .insert(key_for_task.clone(), GithubPrFileContents { base, head });

        if is_selected_file
          && let Some(file) = this.file_lookup.get(&key_for_task).cloned()
          && let Some(contents) = this.file_contents.get(&key_for_task).cloned()
        {
          this.apply_full_diff(&file, &contents, cx);
        }
        cx.notify();
      });
    });

    self.file_content_tasks.insert(key, task);
  }
}
