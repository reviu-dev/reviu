use super::*;
use crate::session_page::center_layout::CenterNode;
use crate::session_page::test_support::{
  add_session_page_window_from_config, isolate_config_store_for_test,
};
use crate::test_support::TempDir;
use gpui::{InputEvent, TestAppContext, VisualTestContext};

fn setup(cx: &mut TestAppContext) -> (TempDir, Entity<SessionPage>, &mut VisualTestContext) {
  isolate_config_store_for_test();
  let project = TempDir::new("center-workspace");
  ConfigStore::persist_recent_project(&project.path);
  let (page, cx) = add_session_page_window_from_config(cx);
  (project, page, cx)
}

fn draft(page: &Entity<SessionPage>, text: &str, cx: &mut VisualTestContext) -> CenterTab {
  page.update_in(cx, |page, window, cx| {
    page.new_untitled_file_action(&crate::NewFile, window, cx);
    let tab = page.shown_editor_tab().expect("draft tab").clone();
    let editor = page.shown_editor().expect("draft editor");
    editor.update(cx, |editor, cx| {
      editor
        .document
        .update(cx, |document, cx| document.replace_all(text, cx));
      editor.is_dirty = !text.is_empty();
    });
    tab
  })
}

fn group(
  page: &Entity<SessionPage>,
  first: &CenterTab,
  second: &CenterTab,
  cx: &mut VisualTestContext,
) -> center_layout::CenterGroupId {
  page.update(cx, |page, _| {
    page.save_active_center_layout();
    page.center_layout = CenterLayout::single(CenterSurface::from_tab(first.clone()));
    assert!(page.center_layout.split_active(
      CenterSurface::from_tab(second.clone()),
      CenterSplitDirection::Right
    ));
    page.remember_center_layout_tab(first.clone());
    page.center_layout.id()
  })
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
  cx.run_until_parked();
  cx.update(|window, cx| window.draw(cx).clear(cx));
  let bounds = cx.debug_bounds(selector).expect("control bounds");
  // Keep both events on the same frame while the dialog is animating.
  cx.update(|window, cx| {
    window.dispatch_event(
      gpui::MouseDownEvent {
        position: bounds.center(),
        button: gpui::MouseButton::Left,
        modifiers: gpui::Modifiers::default(),
        click_count: 1,
        first_mouse: false,
      }
      .to_platform_input(),
      cx,
    );
    window.dispatch_event(
      gpui::MouseUpEvent {
        position: bounds.center(),
        button: gpui::MouseButton::Left,
        modifiers: gpui::Modifiers::default(),
        click_count: 1,
      }
      .to_platform_input(),
      cx,
    );
  });
  cx.run_until_parked();
}

#[gpui::test]
fn cancelling_group_close_keeps_every_dirty_editor_and_draft(cx: &mut TestAppContext) {
  let (project, page, cx) = setup(cx);
  let first = draft(&page, "first unsaved", cx);
  let second = draft(&page, "second unsaved", cx);
  let id = group(&page, &first, &second, cx);
  page.update_in(cx, |page, window, cx| {
    page.close_center_tab(first.clone(), window, cx)
  });
  assert!(cx.update(|window, cx| window.has_active_dialog(cx)));
  click(cx, UNSAVED_EDITOR_DISCARD_DEBUG_SELECTOR);
  assert!(cx.update(|window, cx| window.has_active_dialog(cx)));
  click(cx, UNSAVED_EDITOR_CANCEL_DEBUG_SELECTOR);
  page.read_with(cx, |page, cx| {
    assert_eq!(page.center_layout.id(), id);
    for tab in [&first, &second] {
      assert!(
        page
          .editor_states
          .get(tab)
          .expect("editor state")
          .editor
          .as_ref()
          .expect("editor")
          .read(cx)
          .is_dirty
      );
      assert!(
        page
          .untitled_buffers
          .contains_key(&tab.untitled_id().expect("draft id"))
      );
    }
  });
  assert_eq!(
    ConfigStore::load_untitled_buffers(&SessionPage::canonical_repo(&project.path))
      .expect("drafts")
      .len(),
    2
  );
  page.update_in(cx, |page, window, cx| {
    page.close_center_tab(first.clone(), window, cx)
  });
  click(cx, UNSAVED_EDITOR_DISCARD_DEBUG_SELECTOR);
  click(cx, UNSAVED_EDITOR_DISCARD_DEBUG_SELECTOR);
  assert!(!cx.update(|window, cx| window.has_active_dialog(cx)));
  page.read_with(cx, |page, _| {
    assert!(page.center_group_tab(id).is_none());
    assert!(!page.editor_states.contains_key(&first));
    assert!(!page.editor_states.contains_key(&second));
  });
  assert!(
    ConfigStore::load_untitled_buffers(&SessionPage::canonical_repo(&project.path))
      .expect("drafts")
      .is_empty()
  );
}

#[gpui::test]
fn closing_inactive_groups_and_multiple_tabs_prompts_before_removing_any(cx: &mut TestAppContext) {
  let (_project, page, cx) = setup(cx);
  let first = draft(&page, "first", cx);
  let second = draft(&page, "second", cx);
  group(&page, &first, &second, cx);
  let third = draft(&page, "third", cx);
  let before = page.read_with(cx, |page, _| page.center_tabs.clone());
  page.update_in(cx, |page, window, cx| {
    page.request_close_center_tabs(vec![first.clone(), third.clone()], window, cx)
  });
  click(cx, UNSAVED_EDITOR_DISCARD_DEBUG_SELECTOR);
  click(cx, UNSAVED_EDITOR_DISCARD_DEBUG_SELECTOR);
  click(cx, UNSAVED_EDITOR_CANCEL_DEBUG_SELECTOR);
  page.read_with(cx, |page, _| {
    assert_eq!(page.center_tabs, before);
    for tab in [&first, &second, &third] {
      assert!(page.editor_states.contains_key(tab));
    }
  });
}

#[gpui::test]
async fn saving_a_group_representative_preserves_the_close_request(cx: &mut TestAppContext) {
  let (project, page, cx) = setup(cx);
  let first = draft(&page, "saved draft", cx);
  let second = draft(&page, "", cx);
  let id = group(&page, &first, &second, cx);
  let editor = page.read_with(cx, |page, _| {
    page
      .editor_states
      .get(&first)
      .expect("state")
      .editor
      .clone()
      .expect("editor")
  });
  page.update_in(cx, |page, window, cx| {
    page.close_center_tab(first.clone(), window, cx)
  });
  click(cx, UNSAVED_EDITOR_SAVE_DEBUG_SELECTOR);
  let prompt_task = page
    .update(cx, |page, _| page.save_as_task.take())
    .expect("save path task");
  let path = SessionPage::canonical_repo(&project.path).join("saved.txt");
  cx.simulate_new_path_selection(|_| Some(path.clone()));
  prompt_task.await;
  let task = editor
    .update(cx, |editor, _| editor.save_task.take())
    .expect("save task");
  task.await;
  cx.run_until_parked();
  assert_eq!(
    std::fs::read_to_string(path).expect("saved file"),
    "saved draft"
  );
  page.read_with(cx, |page, _| {
    assert!(page.center_group_tab(id).is_none());
    assert!(!page.editor_states.contains_key(&second));
    assert!(
      !page
        .center_tabs
        .iter()
        .any(|tab| tab.path() == Some(Path::new("saved.txt")))
    );
  });
}

#[gpui::test]
fn reordering_preserves_active_group_history_editors_and_persisted_order(cx: &mut TestAppContext) {
  let (_project, page, cx) = setup(cx);
  let first = draft(&page, "one", cx);
  let second = draft(&page, "two", cx);
  let group_id = group(&page, &first, &second, cx);
  let third = draft(&page, "three", cx);
  let fourth = draft(&page, "four", cx);
  page.update(cx, |page, cx| {
    let third_id = page.center_group_id(&third).expect("third group");
    let fourth_id = page.center_group_id(&fourth).expect("fourth group");
    let active = page.active_center_tab.clone();
    let history = page.center_tab_history.clone();
    let editor_ids = page
      .editor_states
      .values()
      .filter_map(|state| state.editor.as_ref().map(Entity::entity_id))
      .collect::<HashSet<_>>();
    page.reorder_center_tab(group_id, fourth_id, true, cx);
    assert_eq!(page.center_tabs.last(), Some(&first));
    page.reorder_center_tab(fourth_id, third_id, false, cx);
    page.reorder_center_tab(group_id, fourth_id, false, cx);
    assert_eq!(
      page.center_tabs,
      vec![
        CenterTab::chat(),
        first.clone(),
        fourth.clone(),
        third.clone()
      ]
    );
    assert_eq!(page.active_center_tab, active);
    assert_eq!(page.center_tab_history, history);
    assert_eq!(
      page
        .center_group_layout(&first)
        .expect("group")
        .surface_count(),
      2
    );
    assert_eq!(page.center_group_id(&first), Some(group_id));
    assert_eq!(
      page
        .editor_states
        .values()
        .filter_map(|state| state.editor.as_ref().map(Entity::entity_id))
        .collect::<HashSet<_>>(),
      editor_ids
    );
    page.flush_untitled_buffers(cx).expect("persist drafts");
  });
  let restored = cx.update(|window, cx| cx.new(|cx| SessionPage::new(window, cx)));
  restored.read_with(cx, |page, _| {
    assert_eq!(
      page.center_tabs,
      vec![CenterTab::chat(), first, fourth.clone(), third]
    );
    assert_eq!(page.active_center_tab, Some(fourth));
  });
}

#[gpui::test]
fn closing_or_separating_the_representative_keeps_group_identity_and_position(
  cx: &mut TestAppContext,
) {
  let (_project, page, cx) = setup(cx);
  let first = draft(&page, "", cx);
  let second = draft(&page, "", cx);
  let third = draft(&page, "", cx);
  let id = group(&page, &first, &second, cx);
  page.update_in(cx, |page, window, cx| {
    let index = page
      .center_tabs
      .iter()
      .position(|tab| tab == &first)
      .expect("group position");
    page.separate_center_surface(first.clone(), window, cx);
    assert_eq!(page.center_tabs.get(index), Some(&second));
    assert_eq!(page.center_tabs.get(index + 1), Some(&first));
    assert_eq!(page.center_group_id(&second), Some(id));
    page.activate_center_tab(second.clone(), OpenIntent::Open, window, cx);
    assert_eq!(page.center_layout.id(), id);
    let CenterNode::Pane(pane) = page.center_layout.root() else {
      panic!("single pane")
    };
    assert!(page.center_layout.split_pane(
      pane.id(),
      CenterSurface::from_tab(first.clone()),
      CenterSplitDirection::Right
    ));
    page.remember_center_layout_tab(second.clone());
    page.close_center_surface(second.clone(), window, cx);
    assert_eq!(page.center_layout.id(), id);
    assert_eq!(page.center_tabs.get(index), Some(&first));
    assert!(page.center_tabs.contains(&third));
  });
}

#[gpui::test]
fn mouse_drag_reorders_tabs_without_changing_the_active_editor(cx: &mut TestAppContext) {
  let (_project, page, cx) = setup(cx);
  let first = draft(&page, "one", cx);
  let second = draft(&page, "two", cx);
  let third = draft(&page, "three", cx);
  cx.run_until_parked();
  let selector = |tab: &CenterTab| {
    format!(
      "session-center-tab-file-untitled-{}",
      tab.untitled_id().expect("draft id")
    )
  };
  let from = cx
    .debug_bounds(selector(&first).leak())
    .expect("first tab")
    .center();
  let target = cx
    .debug_bounds(selector(&second).leak())
    .expect("second tab");
  let to = gpui::point(target.right() - gpui::px(2.0), target.center().y);
  let active_editor = page.read_with(cx, |page, _| {
    page.shown_editor().expect("active editor").entity_id()
  });
  cx.simulate_mouse_down(from, gpui::MouseButton::Left, gpui::Modifiers::default());
  cx.simulate_mouse_move(
    gpui::point(from.x + gpui::px(10.0), from.y),
    Some(gpui::MouseButton::Left),
    gpui::Modifiers::default(),
  );
  cx.simulate_mouse_move(
    to,
    Some(gpui::MouseButton::Left),
    gpui::Modifiers::default(),
  );
  cx.run_until_parked();
  cx.simulate_mouse_up(to, gpui::MouseButton::Left, gpui::Modifiers::default());
  cx.run_until_parked();
  page.read_with(cx, |page, _| {
    assert_eq!(
      page.center_tabs,
      vec![CenterTab::chat(), second, first, third.clone()]
    );
    assert_eq!(page.active_center_tab.as_ref(), Some(&third));
    assert_eq!(
      page.shown_editor().expect("same editor").entity_id(),
      active_editor
    );
  });
}

#[gpui::test]
fn keyboard_reordering_keeps_focus_and_stops_at_the_edges(cx: &mut TestAppContext) {
  cx.update(crate::shortcuts::install_workspace_shortcuts);
  let (_project, page, cx) = setup(cx);
  let first = draft(&page, "one", cx);
  let second = draft(&page, "two", cx);
  cx.run_until_parked();
  page.update_in(cx, |page, window, cx| {
    window.focus(&page.focus_handle(cx), cx)
  });
  cx.run_until_parked();
  let focus = cx.update(|window, cx| window.focused(cx));
  assert!(focus.is_some());
  cx.simulate_keystrokes("ctrl-shift-pageup");
  cx.run_until_parked();
  page.read_with(cx, |page, _| {
    assert_eq!(
      page.center_tabs,
      vec![CenterTab::chat(), second.clone(), first.clone()]
    );
    assert_eq!(page.active_center_tab.as_ref(), Some(&second));
  });
  cx.update(|window, cx| assert_eq!(window.focused(cx), focus));
  cx.simulate_keystrokes("ctrl-shift-pagedown ctrl-shift-pagedown");
  page.read_with(cx, |page, _| {
    assert_eq!(page.center_tabs, vec![CenterTab::chat(), first, second])
  });
}

#[gpui::test]
fn a_group_can_be_closed_while_its_empty_chat_surface_is_focused(cx: &mut TestAppContext) {
  let (_project, page, cx) = setup(cx);
  let file = draft(&page, "", cx);
  let chat = CenterTab::chat();
  let id = group(&page, &chat, &file, cx);
  page.update_in(cx, |page, window, cx| {
    page.activate_center_surface(&chat, window, cx)
  });
  cx.run_until_parked();
  cx.update(|window, cx| window.draw(cx).clear(cx));
  page.read_with(cx, |page, _| {
    assert_eq!(page.active_center_tab.as_ref(), Some(&file));
    assert!(page.center_group_is_closeable(&file));
  });
  assert!(cx.debug_bounds("session-center-close-File-None-").is_some());
  page.update_in(cx, |page, window, cx| {
    page.close_active_center_tab_action(&crate::CloseCenterTab, window, cx)
  });
  page.read_with(cx, |page, _| {
    assert!(page.center_group_tab(id).is_none());
    assert!(!page.editor_states.contains_key(&file));
  });
}

#[gpui::test]
fn rendering_multiple_editors_keeps_the_focused_surface(cx: &mut TestAppContext) {
  let (_project, page, cx) = setup(cx);
  let first = draft(&page, "one", cx);
  let second = draft(&page, "two", cx);
  group(&page, &first, &second, cx);
  page.update_in(cx, |page, window, cx| {
    page.activate_center_surface(&second, window, cx)
  });
  cx.run_until_parked();
  cx.update(|window, cx| window.draw(cx).clear(cx));
  page.read_with(cx, |page, cx| {
    assert_eq!(page.active_center_tab.as_ref(), Some(&first));
    assert_eq!(page.shown_editor_tab(), Some(&second));
    let editor = page
      .editor_states
      .get(&second)
      .expect("state")
      .editor
      .as_ref()
      .expect("editor");
    assert_eq!(
      page.shown_editor().expect("shown editor").entity_id(),
      editor.entity_id()
    );
    assert_eq!(page.focus_handle(cx), editor.read(cx).focus_handle(cx));
  });
}

#[gpui::test]
fn dragging_near_the_tab_bar_edge_scrolls_overflowing_tabs(cx: &mut TestAppContext) {
  let (_project, page, cx) = setup(cx);
  let mut last = CenterTab::chat();
  for _ in 0..20 {
    last = draft(&page, "", cx);
  }
  cx.run_until_parked();
  let (before, bounds, tabs) = page.read_with(cx, |page, _| {
    (
      page.center_tabs_scroll_handle.offset(),
      page.center_tabs_scroll_handle.bounds(),
      page.center_tabs.clone(),
    )
  });
  assert!(before.x < gpui::px(0.0));
  let selector = format!(
    "session-center-tab-file-untitled-{}",
    last.untitled_id().expect("draft id")
  );
  let from = cx.debug_bounds(selector.leak()).expect("last tab").center();
  cx.simulate_mouse_down(from, gpui::MouseButton::Left, gpui::Modifiers::default());
  cx.simulate_mouse_move(
    gpui::point(from.x - gpui::px(10.0), from.y),
    Some(gpui::MouseButton::Left),
    gpui::Modifiers::default(),
  );
  let edge = gpui::point(bounds.left() + gpui::px(4.0), bounds.center().y);
  cx.simulate_mouse_move(
    edge,
    Some(gpui::MouseButton::Left),
    gpui::Modifiers::default(),
  );
  cx.run_until_parked();
  page.read_with(cx, |page, _| {
    assert!(page.center_tabs_scroll_handle.offset().x > before.x)
  });
  let outside = gpui::point(bounds.center().x, bounds.bottom() + gpui::px(100.0));
  cx.simulate_mouse_move(
    outside,
    Some(gpui::MouseButton::Left),
    gpui::Modifiers::default(),
  );
  cx.simulate_mouse_up(outside, gpui::MouseButton::Left, gpui::Modifiers::default());
  page.read_with(cx, |page, _| assert_eq!(page.center_tabs, tabs));
}

#[gpui::test]
fn checkouts_with_the_same_relative_paths_keep_separate_layouts(cx: &mut TestAppContext) {
  let (first_project, page, cx) = setup(cx);
  let second_project = TempDir::new("center-other-checkout");
  for project in [&first_project, &second_project] {
    for file in ["README.md", "one.txt", "two.txt"] {
      std::fs::write(project.path.join(file), "text").expect("file");
    }
  }
  let representative = CenterTab::file(PathBuf::from("README.md"));
  let first_file = CenterTab::file(PathBuf::from("one.txt"));
  let second_file = CenterTab::file(PathBuf::from("two.txt"));
  let first_id = group(&page, &representative, &first_file, cx);
  page.update_in(cx, |page, window, cx| {
    page
      .set_project_root_without_unsaved_prompt(second_project.path.clone(), window, cx)
      .expect("switch project");
    assert!(!page.center_layouts_by_tab.contains_key(&representative));
    page.forget_placeholder_chat_tab();
    let first_checkout = SessionPage::canonical_repo(&first_project.path);
    assert!(
      page
        .center_checkouts
        .get(&first_checkout)
        .expect("inactive checkout")
        .tabs
        .contains(&CenterTab::chat())
    );
  });
  let second_id = group(&page, &representative, &second_file, cx);
  page.update_in(cx, |page, window, cx| {
    page
      .set_project_root_without_unsaved_prompt(first_project.path.clone(), window, cx)
      .expect("switch back");
    assert_eq!(page.center_layout.id(), first_id);
    assert!(page.center_layout.contains_tab(&first_file));
    assert!(!page.center_layout.contains_tab(&second_file));
    page.handle_file_renamed(Path::new("README.md"), Path::new("renamed.md"), cx);
    page
      .set_project_root_without_unsaved_prompt(second_project.path.clone(), window, cx)
      .expect("switch again");
    assert_eq!(page.center_layout.id(), second_id);
    assert!(page.center_layout.contains_tab(&representative));
    assert!(page.center_layout.contains_tab(&second_file));
    assert!(!page.center_layout.contains_tab(&first_file));
  });
}
