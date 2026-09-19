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
    if tab.kind == CenterTabKind::Terminal {
      self.focus_terminal_tab(tab, window, cx);
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
