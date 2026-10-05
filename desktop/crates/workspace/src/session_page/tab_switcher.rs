//! The `ctrl-tab` switcher: the open center tabs, most recently used first.

use std::time::Duration;

use gpui::{
  AnyElement, App, Context, EventEmitter, FocusHandle, Focusable, InteractiveElement as _,
  IntoElement, Modifiers, ModifiersChangedEvent, ParentElement as _, Render, ScrollHandle,
  SharedString, StatefulInteractiveElement as _, Styled as _, Task, Window, actions, div,
  prelude::FluentBuilder, px,
};
use gpui_component::{ActiveTheme as _, h_flex, v_flex};

use super::center_tab::{CenterTab, CenterTabIcon};

actions!(tab_switcher, [SelectNext, SelectPrevious, Confirm, Dismiss]);

pub(crate) const TAB_SWITCHER_CONTEXT: &str = "TabSwitcher";

const TAB_SWITCHER_WIDTH_PX: f32 = 448.0;
const TAB_SWITCHER_MAX_HEIGHT_PX: f32 = 400.0;
// A quick press-and-release switches tabs without the list ever flashing.
const REVEAL_DELAY: Duration = Duration::from_millis(300);

pub(super) struct TabSwitcherEntry {
  pub(super) tab: CenterTab,
  pub(super) label: SharedString,
  pub(super) detail: Option<SharedString>,
  pub(super) icon: Option<CenterTabIcon>,
}

pub(super) enum TabSwitcherEvent {
  Confirmed(CenterTab),
  Dismissed,
}

pub(super) struct TabSwitcher {
  focus_handle: FocusHandle,
  entries: Vec<TabSwitcherEntry>,
  selected_index: usize,
  /// The modifiers held when the switcher opened; releasing them confirms.
  init_modifiers: Option<Modifiers>,
  visible: bool,
  scroll_handle: ScrollHandle,
  _reveal_task: Option<Task<()>>,
}

impl EventEmitter<TabSwitcherEvent> for TabSwitcher {}

impl Focusable for TabSwitcher {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

impl TabSwitcher {
  /// `entries` are ordered most recently used first, so the first one is the
  /// tab already on screen.
  pub(super) fn new(
    entries: Vec<TabSwitcherEntry>,
    select_last: bool,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    let selected_index = if select_last {
      entries.len().saturating_sub(1)
    } else {
      1.min(entries.len().saturating_sub(1))
    };
    let init_modifiers = window.modifiers().modified().then_some(window.modifiers());
    let reveal_task = init_modifiers.is_some().then(|| {
      cx.spawn(async move |this, cx| {
        cx.background_executor().timer(REVEAL_DELAY).await;
        let _ = this.update(cx, |this, cx| {
          this.visible = true;
          this.scroll_handle.scroll_to_item(this.selected_index);
          cx.notify();
        });
      })
    });
    let scroll_handle = ScrollHandle::new();
    scroll_handle.scroll_to_item(selected_index);
    Self {
      focus_handle: cx.focus_handle(),
      entries,
      selected_index,
      init_modifiers,
      visible: init_modifiers.is_none(),
      scroll_handle,
      _reveal_task: reveal_task,
    }
  }

  #[cfg(test)]
  pub(super) fn selected_tab(&self) -> Option<&CenterTab> {
    self
      .entries
      .get(self.selected_index)
      .map(|entry| &entry.tab)
  }

  #[cfg(test)]
  pub(super) fn tabs(&self) -> Vec<CenterTab> {
    self.entries.iter().map(|entry| entry.tab.clone()).collect()
  }

  #[cfg(test)]
  pub(super) fn is_visible(&self) -> bool {
    self.visible
  }

  pub(super) fn select_next(&mut self, cx: &mut Context<Self>) {
    self.select_offset(1, cx);
  }

  pub(super) fn select_previous(&mut self, cx: &mut Context<Self>) {
    self.select_offset(-1, cx);
  }

  fn select_offset(&mut self, offset: isize, cx: &mut Context<Self>) {
    if self.entries.is_empty() {
      return;
    }
    let count = self.entries.len() as isize;
    self.selected_index = (self.selected_index as isize + offset).rem_euclid(count) as usize;
    self.scroll_handle.scroll_to_item(self.selected_index);
    cx.notify();
  }

  fn confirm(&mut self, cx: &mut Context<Self>) {
    match self.entries.get(self.selected_index) {
      Some(entry) => cx.emit(TabSwitcherEvent::Confirmed(entry.tab.clone())),
      None => cx.emit(TabSwitcherEvent::Dismissed),
    }
  }

  fn handle_modifiers_changed(
    &mut self,
    event: &ModifiersChangedEvent,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(init_modifiers) = self.init_modifiers else {
      return;
    };
    if !event.modified() || !init_modifiers.is_subset_of(event) {
      self.init_modifiers = None;
      self.confirm(cx);
    }
  }

  fn render_entry(
    &self,
    index: usize,
    entry: &TabSwitcherEntry,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let theme = cx.theme().clone();
    let selected = index == self.selected_index;
    h_flex()
      .id(("tab-switcher-entry", index))
      .debug_selector(move || format!("tab-switcher-entry-{index}"))
      .flex_shrink_0()
      .items_center()
      .gap_2()
      .mx_1()
      .px_2()
      .py_1p5()
      .rounded_md()
      .text_sm()
      .cursor_pointer()
      .when(selected, |this| this.bg(theme.list_active))
      .when(!selected, |this| {
        this.hover(|this| this.bg(theme.list_hover))
      })
      .on_click(cx.listener(move |this, _, _, cx| {
        this.selected_index = index;
        this.confirm(cx);
      }))
      .children(entry.icon.as_ref().map(|icon| {
        h_flex()
          .flex_shrink_0()
          .items_center()
          .justify_center()
          .child(icon.render(cx))
      }))
      .child(
        div()
          .flex_shrink_0()
          .max_w_full()
          .truncate()
          .child(entry.label.clone()),
      )
      .children(entry.detail.clone().map(|detail| {
        div()
          .flex_1()
          .min_w_0()
          .truncate()
          .text_xs()
          .text_color(theme.muted_foreground)
          .child(detail)
      }))
      .into_any_element()
  }
}

impl Render for TabSwitcher {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let root = v_flex()
      .id("tab-switcher")
      .key_context(TAB_SWITCHER_CONTEXT)
      .track_focus(&self.focus_handle)
      .on_modifiers_changed(cx.listener(Self::handle_modifiers_changed))
      .on_action(cx.listener(|this, _: &crate::ToggleTabSwitcher, _, cx| this.select_next(cx)))
      .on_action(
        cx.listener(|this, _: &crate::ToggleTabSwitcherBackward, _, cx| this.select_previous(cx)),
      )
      .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.select_next(cx)))
      .on_action(cx.listener(|this, _: &SelectPrevious, _, cx| this.select_previous(cx)))
      .on_action(cx.listener(|this, _: &Confirm, _, cx| this.confirm(cx)))
      .on_action(cx.listener(|_, _: &Dismiss, _, cx| cx.emit(TabSwitcherEvent::Dismissed)));
    if !self.visible {
      return root.size_0().overflow_hidden();
    }
    root
      .debug_selector(|| "tab-switcher".to_string())
      .w(px(TAB_SWITCHER_WIDTH_PX))
      .max_h(px(TAB_SWITCHER_MAX_HEIGHT_PX))
      .py_1()
      .bg(theme.popover)
      .text_color(theme.popover_foreground)
      .border_1()
      .border_color(theme.border)
      .rounded(theme.radius_lg)
      .shadow_lg()
      .occlude()
      .overflow_y_scroll()
      .track_scroll(&self.scroll_handle)
      .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(TabSwitcherEvent::Dismissed)))
      .children(
        self
          .entries
          .iter()
          .enumerate()
          .map(|(index, entry)| self.render_entry(index, entry, cx))
          .collect::<Vec<_>>(),
      )
  }
}

#[cfg(test)]
mod tests {
  use super::super::test_support::*;
  use super::super::*;
  use crate::test_support::{TempRepo, commit_text_file};
  use gpui::{Modifiers, TestAppContext, VisualTestContext};

  async fn page_with_three_diffs(
    cx: &mut TestAppContext,
  ) -> (TempRepo, Entity<SessionPage>, &mut VisualTestContext) {
    let repo = TempRepo::init("session-page-tab-switcher");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    commit_text_file(&repo.path, Path::new("other.md"), "one\n", "second");
    commit_text_file(&repo.path, Path::new("third.md"), "one\n", "third");
    for path in ["README.md", "other.md", "third.md"] {
      std::fs::write(repo.path.join(path), "changed\n").expect("update file");
    }

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.update(|_, cx| {
      cx.bind_keys(crate::shortcuts::workspace_key_bindings());
      cx.bind_keys(crate::shortcuts::tab_switcher_key_bindings());
    });
    for path in ["README.md", "other.md", "third.md"] {
      page.update_in(cx, |page, window, cx| {
        page.open_diff(PathBuf::from(path), None, OpenIntent::Open, window, cx);
      });
      await_open_file(&page, cx).await;
    }
    cx.run_until_parked();
    (repo, page, cx)
  }

  fn selected_tab(page: &Entity<SessionPage>, cx: &mut VisualTestContext) -> Option<CenterTab> {
    page.read_with(cx, |page, cx| {
      let (switcher, _) = page.tab_switcher.as_ref()?;
      switcher.read(cx).selected_tab().cloned()
    })
  }

  #[gpui::test]
  async fn a_quick_ctrl_tab_toggles_between_the_two_most_recent_tabs(cx: &mut TestAppContext) {
    let (_repo, page, cx) = page_with_three_diffs(cx).await;
    let readme = CenterTab::diff(PathBuf::from("README.md"));
    let other = CenterTab::diff(PathBuf::from("other.md"));
    let third = CenterTab::diff(PathBuf::from("third.md"));

    cx.simulate_modifiers_change(Modifiers::control());
    cx.simulate_keystrokes("ctrl-tab");
    page.read_with(cx, |page, cx| {
      let (switcher, _) = page.tab_switcher.as_ref().expect("switcher should open");
      let switcher = switcher.read(cx);
      assert_eq!(
        switcher.tabs(),
        vec![third.clone(), other.clone(), readme.clone()]
      );
      assert_eq!(switcher.selected_tab(), Some(&other));
      assert!(!switcher.is_visible());
    });
    assert!(cx.debug_bounds("tab-switcher").is_none());

    cx.simulate_modifiers_change(Modifiers::none());
    await_open_file(&page, cx).await;
    page.read_with(cx, |page, _| {
      assert!(page.tab_switcher.is_none());
      assert_eq!(page.active_center_tab.as_ref(), Some(&other));
    });
    cx.update(|window, cx| window.simulate_next_frame(cx));
    cx.update(|window, cx| {
      assert!(page.read(cx).focus_handle(cx).is_focused(window));
    });

    cx.simulate_modifiers_change(Modifiers::control());
    cx.simulate_keystrokes("ctrl-tab");
    cx.simulate_modifiers_change(Modifiers::none());
    await_open_file(&page, cx).await;
    page.read_with(cx, |page, _| {
      assert_eq!(page.active_center_tab.as_ref(), Some(&third));
    });
  }

  #[gpui::test]
  async fn holding_ctrl_reveals_the_list_and_cycles_through_it(cx: &mut TestAppContext) {
    let (_repo, page, cx) = page_with_three_diffs(cx).await;
    let readme = CenterTab::diff(PathBuf::from("README.md"));
    let other = CenterTab::diff(PathBuf::from("other.md"));
    let third = CenterTab::diff(PathBuf::from("third.md"));

    cx.simulate_modifiers_change(Modifiers::control());
    cx.simulate_keystrokes("ctrl-shift-tab");
    assert_eq!(selected_tab(&page, cx), Some(readme.clone()));
    cx.simulate_keystrokes("ctrl-tab");
    assert_eq!(selected_tab(&page, cx), Some(third.clone()));
    cx.simulate_keystrokes("ctrl-tab");
    assert_eq!(selected_tab(&page, cx), Some(other.clone()));
    cx.simulate_keystrokes("ctrl-down");
    assert_eq!(selected_tab(&page, cx), Some(readme));
    cx.simulate_keystrokes("ctrl-shift-tab");
    assert_eq!(selected_tab(&page, cx), Some(other));

    cx.executor().advance_clock(super::REVEAL_DELAY);
    cx.run_until_parked();
    assert!(cx.debug_bounds("tab-switcher").is_some());
    assert!(cx.debug_bounds("tab-switcher-entry-2").is_some());

    cx.simulate_keystrokes("ctrl-escape");
    cx.run_until_parked();
    page.read_with(cx, |page, _| {
      assert!(page.tab_switcher.is_none());
      assert_eq!(page.active_center_tab.as_ref(), Some(&third));
    });
    assert!(cx.debug_bounds("tab-switcher").is_none());
    cx.update(|window, cx| window.simulate_next_frame(cx));
    cx.update(|window, cx| {
      assert!(page.read(cx).focus_handle(cx).is_focused(window));
    });
  }
}
