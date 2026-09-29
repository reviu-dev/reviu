use super::*;

#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct CenterCheckoutState {
  pub(super) tabs: Vec<CenterTab>,
  pub(super) active_tab: Option<CenterTab>,
  pub(super) history: Vec<CenterTab>,
  pub(super) layouts: HashMap<CenterTab, CenterLayout>,
}

impl SessionPage {
  pub(super) fn activate_center_surface(
    &mut self,
    tab: &CenterTab,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if !self.center_layout.contains_tab(tab) {
      return;
    }
    self
      .center_layout
      .set_active_surface(CenterSurface::from_tab(tab.clone()));
    if let Some(id) = tab.conversation_id() {
      self.activate_session_panel(id, window, cx);
    }
    self.center = Self::center_view_for_tab(tab);
    if matches!(tab.kind, CenterTabKind::File | CenterTabKind::Diff) {
      self.editor_tab = Some(tab.clone());
    }
    match tab.kind {
      CenterTabKind::ProjectSearch => self.focus_project_search_on_next_frame(window, cx),
      CenterTabKind::Terminal => self.focus_terminal_tab(tab, window, cx),
      _ => {}
    }
    self.reveal_center_tab_in_files_panel(tab, cx);
    self.save_active_center_layout();
    self.persist_current_center_workspace(cx);
    cx.notify();
  }

  pub(super) fn resize_center_split(
    &mut self,
    group: center_layout::CenterGroupId,
    split: center_layout::CenterSplitId,
    sizes: Vec<gpui::Pixels>,
    cx: &mut Context<Self>,
  ) {
    if self.center_layout.id() != group {
      return;
    }
    let [first, second] = sizes.as_slice() else {
      return;
    };
    let total = f32::from(*first + *second);
    if !total.is_finite() || total <= 0.0 {
      return;
    }
    let fraction = (f32::from(*first) / total * 10_000.0).round() as u16;
    if self.center_layout.resize_split(split, fraction) {
      self.save_active_center_layout();
      self.persist_current_center_workspace(cx);
      cx.notify();
    }
  }

  pub(super) fn open_center_surface_in_split(
    &mut self,
    surface: CenterSurface,
    direction: CenterSplitDirection,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> bool {
    let tab = surface.tab().clone();
    let representative = self
      .active_center_tab
      .clone()
      .unwrap_or_else(|| self.center_layout.active_tab().clone());
    if !self.center_layout.split_active_pane(surface, direction) {
      cx.notify();
      return false;
    }

    self.center = Self::center_view_for_tab(&tab);
    if let Some(conversation_id) = tab.conversation_id() {
      self.activate_session_panel(conversation_id, window, cx);
    }
    if tab.kind == CenterTabKind::Terminal {
      self.focus_terminal_tab(&tab, window, cx);
    }
    self.ensure_center_layout_chat_panels(window, cx);
    self.remember_center_layout_tab(representative);
    self.reveal_center_tab_in_files_panel(&tab, cx);
    self.sync_agent_chat_close_control(cx);
    self.restore_visible_center_editors(cx);
    self.persist_current_center_workspace(cx);
    cx.notify();
    true
  }

  pub(super) fn scroll_center_tabs_during_drag(&self, window: &mut Window) {
    let bounds = self.center_tabs_scroll_handle.bounds();
    let pointer = window.mouse_position();
    if !bounds.contains(&pointer) {
      return;
    }
    let margin = gpui::px(32.0);
    let delta = if pointer.x < bounds.left() + margin {
      gpui::px(8.0)
    } else if pointer.x > bounds.right() - margin {
      gpui::px(-8.0)
    } else {
      return;
    };
    let offset = self.center_tabs_scroll_handle.offset();
    let maximum = self
      .center_tabs_scroll_handle
      .max_offset()
      .x
      .max(gpui::px(0.0));
    let next = (offset.x + delta).max(-maximum).min(gpui::px(0.0));
    if next != offset.x {
      self
        .center_tabs_scroll_handle
        .set_offset(gpui::point(next, offset.y));
      window.request_animation_frame();
    }
  }

  pub(super) fn center_group_is_closeable(&self, tab: &CenterTab) -> bool {
    tab.is_closeable()
      || self
        .center_group_layout(tab)
        .is_some_and(|layout| layout.tabs().iter().any(CenterTab::is_closeable))
  }

  pub(super) fn center_group_id(&self, tab: &CenterTab) -> Option<center_layout::CenterGroupId> {
    if self.active_center_tab.as_ref() == Some(tab) {
      return Some(self.center_layout.id());
    }
    self.center_layouts_by_tab.get(tab).map(CenterLayout::id)
  }

  pub(super) fn center_group_tab(&self, id: center_layout::CenterGroupId) -> Option<CenterTab> {
    self
      .center_tabs_for_navigation()
      .into_iter()
      .find(|tab| self.center_group_id(tab) == Some(id))
  }

  pub(super) fn center_group_layout(&self, tab: &CenterTab) -> Option<&CenterLayout> {
    if self.active_center_tab.as_ref() == Some(tab) {
      Some(&self.center_layout)
    } else {
      self.center_layouts_by_tab.get(tab)
    }
  }

  pub(super) fn move_center_tab_left_action(
    &mut self,
    _: &crate::MoveCenterTabLeft,
    _: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.move_active_center_tab(-1, cx);
  }

  pub(super) fn move_center_tab_right_action(
    &mut self,
    _: &crate::MoveCenterTabRight,
    _: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.move_active_center_tab(1, cx);
  }

  pub(super) fn split_pane_right_action(
    &mut self,
    _: &crate::SplitPaneRight,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let _ = self.split_pane_with_launcher(CenterSplitDirection::Right, window, cx);
    cx.stop_propagation();
  }

  pub(super) fn split_pane_down_action(
    &mut self,
    _: &crate::SplitPaneDown,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let _ = self.split_pane_with_launcher(CenterSplitDirection::Down, window, cx);
    cx.stop_propagation();
  }

  pub(super) fn open_file_in_split_right_action(
    &mut self,
    _: &crate::OpenFileInSplitRight,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.open_file_picker_in_split(CenterSplitDirection::Right, window, cx);
  }

  pub(super) fn open_file_in_split_down_action(
    &mut self,
    _: &crate::OpenFileInSplitDown,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.open_file_picker_in_split(CenterSplitDirection::Down, window, cx);
  }

  pub(super) fn open_diff_in_split_right_action(
    &mut self,
    _: &crate::OpenDiffInSplitRight,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.open_diff_picker_in_split(CenterSplitDirection::Right, window, cx);
  }

  pub(super) fn open_diff_in_split_down_action(
    &mut self,
    _: &crate::OpenDiffInSplitDown,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.open_diff_picker_in_split(CenterSplitDirection::Down, window, cx);
  }

  pub(super) fn move_center_pane_left_action(
    &mut self,
    _: &crate::MoveCenterPaneLeft,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.move_active_center_pane_to_edge(CenterSplitDirection::Left, window, cx);
  }

  pub(super) fn move_center_pane_right_action(
    &mut self,
    _: &crate::MoveCenterPaneRight,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.move_active_center_pane_to_edge(CenterSplitDirection::Right, window, cx);
  }

  pub(super) fn move_center_pane_up_action(
    &mut self,
    _: &crate::MoveCenterPaneUp,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.move_active_center_pane_to_edge(CenterSplitDirection::Up, window, cx);
  }

  pub(super) fn move_center_pane_down_action(
    &mut self,
    _: &crate::MoveCenterPaneDown,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.move_active_center_pane_to_edge(CenterSplitDirection::Down, window, cx);
  }

  pub(super) fn focus_center_pane_left_action(
    &mut self,
    _: &crate::FocusCenterPaneLeft,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.focus_center_pane(CenterSplitDirection::Left, window, cx);
  }

  pub(super) fn focus_center_pane_right_action(
    &mut self,
    _: &crate::FocusCenterPaneRight,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.focus_center_pane(CenterSplitDirection::Right, window, cx);
  }

  pub(super) fn focus_center_pane_up_action(
    &mut self,
    _: &crate::FocusCenterPaneUp,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.focus_center_pane(CenterSplitDirection::Up, window, cx);
  }

  pub(super) fn focus_center_pane_down_action(
    &mut self,
    _: &crate::FocusCenterPaneDown,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.focus_center_pane(CenterSplitDirection::Down, window, cx);
  }

  fn split_pane_with_launcher(
    &mut self,
    direction: CenterSplitDirection,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Option<CenterTab> {
    let tab = self.next_pane_launcher_tab();
    self
      .open_center_surface_in_split(CenterSurface::from_tab(tab.clone()), direction, window, cx)
      .then_some(tab)
  }

  fn next_pane_launcher_tab(&mut self) -> CenterTab {
    let id = self.next_pane_launcher_id;
    self.next_pane_launcher_id = self.next_pane_launcher_id.saturating_add(1);
    CenterTab::pane_launcher(id)
  }

  pub(super) fn open_file_picker_in_split(
    &mut self,
    direction: CenterSplitDirection,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.checkout_root(cx).is_none() {
      window.push_notification(
        Notification::warning("Open a project before opening a file."),
        cx,
      );
      cx.stop_propagation();
      return;
    }
    if let Some(tab) = self.split_pane_with_launcher(direction, window, cx) {
      self.open_file_picker_for_split_launcher(tab, window, cx);
    }
    cx.stop_propagation();
  }

  pub(super) fn open_diff_picker_in_split(
    &mut self,
    direction: CenterSplitDirection,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.checkout_root(cx).is_none() {
      window.push_notification(
        Notification::warning("Open a Git project before opening a diff."),
        cx,
      );
      cx.stop_propagation();
      return;
    }
    if self.dock_panel.read(cx).status_entries().is_empty() {
      window.push_notification(Notification::info("No changed files to diff."), cx);
      cx.stop_propagation();
      return;
    }
    if let Some(tab) = self.split_pane_with_launcher(direction, window, cx) {
      self.open_diff_picker_for_split_launcher(tab, window, cx);
    }
    cx.stop_propagation();
  }

  fn move_active_center_pane_to_edge(
    &mut self,
    direction: CenterSplitDirection,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let tab = self.center_layout.active_tab().clone();
    self.move_center_surface_to_edge(tab, direction, window, cx);
    cx.stop_propagation();
  }

  fn focus_center_pane(
    &mut self,
    direction: CenterSplitDirection,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if let Some(tab) = self
      .center_layout
      .adjacent_pane_info(direction)
      .map(|pane| pane.active_tab)
    {
      self.activate_center_surface(&tab, window, cx);
    }
    cx.stop_propagation();
  }

  pub(super) fn replace_split_launcher_with_surface(
    &mut self,
    launcher_tab: &CenterTab,
    tab: CenterTab,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> bool {
    if launcher_tab.kind != CenterTabKind::PaneLauncher
      || !self.center_layout.contains_tab(launcher_tab)
    {
      return false;
    }
    self.center_layout.replace_tab(launcher_tab, &tab);
    self
      .center_layout
      .set_active_surface(CenterSurface::from_tab(tab.clone()));
    self.center = Self::center_view_for_tab(&tab);
    if let Some(conversation_id) = tab.conversation_id() {
      self.activate_session_panel(conversation_id, window, cx);
    }
    self.ensure_center_layout_chat_panels(window, cx);
    self.remember_center_layout_tab(
      self
        .active_center_tab
        .clone()
        .unwrap_or_else(|| self.center_layout.active_tab().clone()),
    );
    self.reveal_center_tab_in_files_panel(&tab, cx);
    self.sync_agent_chat_close_control(cx);
    self.restore_visible_center_editors(cx);
    match self.center {
      CenterView::Conversation => self.focus_agent_input_on_next_frame(window, cx),
      CenterView::Diff => self.focus_editor_on_next_frame(window, cx),
      CenterView::InteractiveRebase => {}
      CenterView::ProjectSearch => self.focus_project_search_on_next_frame(window, cx),
      CenterView::Terminal => self.focus_terminal_tab(&tab, window, cx),
      CenterView::PaneLauncher => {}
    }
    self.persist_current_center_workspace(cx);
    cx.notify();
    true
  }

  pub(super) fn open_terminal_for_split_launcher(
    &mut self,
    launcher_tab: CenterTab,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(working_directory) = self.terminal_working_directory(cx, window) else {
      return;
    };
    let (project_root, checkout_root) = self.terminal_roots(&working_directory, cx);
    let tab = self.create_terminal_tab(working_directory, project_root, checkout_root, window, cx);
    if !self.replace_split_launcher_with_surface(&launcher_tab, tab.clone(), window, cx) {
      self.clear_terminal_tab(&tab);
    }
  }

  pub(super) fn open_project_search_for_split_launcher(
    &mut self,
    launcher_tab: CenterTab,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(repo_root) = self.checkout_root(cx) else {
      window.push_notification(
        Notification::warning("Open a Git project before starting project search."),
        cx,
      );
      return;
    };
    let tab = CenterTab::project_search();
    self.detach_project_search_from_layouts();
    self.ensure_project_search_view(repo_root, window, cx);
    self.center_layouts_by_tab.remove(&tab);
    self.replace_split_launcher_with_surface(&launcher_tab, tab, window, cx);
  }

  pub(super) fn open_file_picker_for_split_launcher(
    &mut self,
    launcher_tab: CenterTab,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(repo_root) = self.checkout_root(cx) else {
      window.push_notification(
        Notification::warning("Open a project before opening a file."),
        cx,
      );
      return;
    };

    let cached_paths = self
      .file_search_cache
      .as_ref()
      .filter(|cache| cache.checkout_root == repo_root)
      .map(|cache| cache.paths.clone());
    let cache_is_fresh = self.file_search_cache.as_ref().is_some_and(|cache| {
      cache.checkout_root == repo_root && cache.loaded_at.elapsed() < FILE_SEARCH_CACHE_TTL
    });
    let repository_paths = cached_paths.as_deref().map(Vec::as_slice).unwrap_or(&[]);
    let entries = self.file_search_entries(&repo_root, repository_paths, cx);

    let view = cx.entity();
    let handler: SearchFileHandler = Arc::new(move |request, window, cx| {
      view.update(cx, |view, cx| {
        view.replace_split_launcher_with_file(
          launcher_tab.clone(),
          request.path,
          false,
          window,
          cx,
        );
      });
      Ok(())
    });
    let palette = open_file_search_palette(window, cx, entries, handler, !cache_is_fresh);

    if cache_is_fresh {
      return;
    }

    let load_repo_root = repo_root.clone();
    let palette = palette.downgrade();
    self._file_search_task = Some(cx.spawn_in(window, async move |this, cx| {
      let result = cx
        .background_spawn({
          let repo_root = load_repo_root.clone();
          async move { list_project_files(&repo_root) }
        })
        .await;

      let _ = this.update_in(cx, |this, window, cx| match result {
        Ok(paths) => {
          if this.checkout_root(cx).as_deref() != Some(load_repo_root.as_path()) {
            return;
          }
          let paths = Arc::new(paths);
          this.file_search_cache = Some(FileSearchCache {
            checkout_root: load_repo_root.clone(),
            paths: paths.clone(),
            loaded_at: Instant::now(),
          });
          let entries = this.file_search_entries(&load_repo_root, paths.as_ref(), cx);
          let _ = palette.update(cx, |palette, cx| {
            palette.replace_entries(entries, window, cx);
          });
        }
        Err(error) => {
          log::error!("load files for split launcher: {error:#}");
          let _ = palette.update(cx, |palette, cx| {
            palette.set_loading_error("Could not load project files", cx);
          });
        }
      });
    }));
  }

  pub(super) fn open_diff_picker_for_split_launcher(
    &mut self,
    launcher_tab: CenterTab,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.checkout_root(cx).is_none() {
      window.push_notification(
        Notification::warning("Open a Git project before opening a diff."),
        cx,
      );
      return;
    }
    let entries = self
      .dock_panel
      .read(cx)
      .status_entries()
      .iter()
      .map(|entry| {
        let label = entry.path.to_string_lossy().replace(['\n', '\r'], "");
        SearchFileEntry::new(entry.path.clone(), label).in_group(SearchFileGroup::Changed)
      })
      .collect::<Vec<_>>();
    if entries.is_empty() {
      window.push_notification(Notification::info("No changed files to diff."), cx);
      return;
    }

    let view = cx.entity();
    let handler: SearchFileHandler = Arc::new(move |request, window, cx| {
      view.update(cx, |view, cx| {
        view.replace_split_launcher_with_file(launcher_tab.clone(), request.path, true, window, cx);
      });
      Ok(())
    });
    open_file_search_palette(window, cx, entries, handler, false);
  }

  fn replace_split_launcher_with_file(
    &mut self,
    launcher_tab: CenterTab,
    path: PathBuf,
    diff: bool,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(repo_root) = self.checkout_root(cx) else {
      return;
    };
    let tab = if diff {
      CenterTab::diff(path.clone())
    } else {
      CenterTab::file(path.clone())
    };
    self.show_preview = false;
    let app_settings = crate::config::AppSettings::get(cx);
    self.diff_view = if app_settings.split_diff_view {
      DiffViewMode::Split
    } else {
      DiffViewMode::Inline
    };
    if self.warm_selected_file().is_none() {
      self.hide_whitespace = app_settings.hide_whitespace;
    }
    self.switch_worktree_tab_mode(&tab, cx);
    if !self.replace_split_launcher_with_surface(&launcher_tab, tab.clone(), window, cx) {
      return;
    }
    self.center = CenterView::Diff;
    self.diff_chat_open = false;
    self.editor_tab = Some(tab.clone());
    self.record_recent_file(&repo_root, &path);
    if !self.editor_states.contains_key(&tab) {
      self.restore_persisted_center_editor(tab.clone(), path.clone(), repo_root, cx);
    }
    if diff {
      self
        .dock_panel
        .read(cx)
        .changes_list()
        .update(cx, |list, cx| {
          list.select_path(Some(path.as_path()), cx);
        });
    }
    self.sync_editor_unmerged_state(cx);
    self.sync_git_telemetry(cx);
    self.focus_editor_on_next_frame(window, cx);
    self.persist_current_center_workspace(cx);
    cx.notify();
  }

  fn move_active_center_tab(&mut self, direction: isize, cx: &mut Context<Self>) {
    let Some(active) = self.active_center_tab.as_ref() else {
      return;
    };
    let Some(index) = self.center_tabs.iter().position(|tab| tab == active) else {
      return;
    };
    let Some(target_index) = index.checked_add_signed(direction) else {
      return;
    };
    let Some(source) = self.center_group_id(active) else {
      return;
    };
    let Some(target) = self
      .center_tabs
      .get(target_index)
      .and_then(|tab| self.center_group_id(tab))
    else {
      return;
    };
    self.reorder_center_tab(source, target, direction > 0, cx);
    cx.stop_propagation();
  }

  pub(super) fn reorder_center_tab(
    &mut self,
    source: center_layout::CenterGroupId,
    target: center_layout::CenterGroupId,
    after: bool,
    cx: &mut Context<Self>,
  ) {
    self.center_drag_target = None;
    let Some(source_tab) = self.center_group_tab(source) else {
      return;
    };
    let Some(target_tab) = self.center_group_tab(target) else {
      return;
    };
    if source == target {
      return;
    }
    let Some(source_index) = self.center_tabs.iter().position(|tab| tab == &source_tab) else {
      return;
    };
    let Some(target_index) = self.center_tabs.iter().position(|tab| tab == &target_tab) else {
      return;
    };
    let insertion_index =
      target_index + usize::from(after) - usize::from(source_index < target_index);
    self.center_tabs.remove(source_index);
    self.center_tabs.insert(insertion_index, source_tab);
    self.center_tabs_revealed_tab.replace(None);
    self.persist_current_center_workspace(cx);
    cx.notify();
  }

  pub(super) fn replace_center_group_representative(&mut self, old: &CenterTab, new: &CenterTab) {
    file_viewer::replace_center_tabs(&mut self.center_tabs, old, new);
    file_viewer::replace_center_tabs(&mut self.center_tab_history, old, new);
    self.center_layouts_by_tab.remove(old);
    self.active_center_tab = Some(new.clone());
  }
}
