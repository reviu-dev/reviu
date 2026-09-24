use super::*;
use crate::session_page::test_support::{
  add_session_page_window_from_config, isolate_config_store_for_test,
};
use crate::test_support::TempDir;
use gpui::{TestAppContext, VisualTestContext};

fn setup(cx: &mut TestAppContext) -> (TempDir, Entity<SessionPage>, &mut VisualTestContext) {
  isolate_config_store_for_test();
  let project = TempDir::new("untitled-persistence");
  ConfigStore::persist_recent_project(&project.path);
  let (page, cx) = add_session_page_window_from_config(cx);
  (project, page, cx)
}

fn new_buffer(page: &Entity<SessionPage>, text: &str, cx: &mut VisualTestContext) -> CenterTab {
  page.update_in(cx, |page, window, cx| {
    page.new_untitled_file_action(&crate::NewFile, window, cx);
    let tab = page.shown_editor_tab().expect("untitled tab").clone();
    let editor = page.shown_editor().expect("untitled editor");
    editor.update(cx, |editor, cx| {
      editor
        .document()
        .clone()
        .update(cx, |document, cx| document.replace_all(text, cx));
      editor.is_dirty = !text.is_empty();
      editor.selections.primary_mut().range = 1.min(text.chars().count())..text.chars().count();
      editor.selections.primary_mut().reversed = true;
      cx.notify();
    });
    tab
  })
}

fn fresh_page(cx: &mut VisualTestContext) -> Entity<SessionPage> {
  cx.update(|window, cx| cx.new(|cx| SessionPage::new(window, cx)))
}

fn stored(project: &Path) -> Vec<(u64, UntitledSnapshot)> {
  ConfigStore::load_untitled_buffers(&SessionPage::canonical_repo(project))
    .expect("load drafts")
    .into_iter()
    .map(|(id, state)| (id, serde_json::from_str(&state).expect("draft state")))
    .collect()
}

fn finish_debounce(cx: &mut VisualTestContext) {
  cx.run_until_parked();
  cx.background_executor.advance_clock(WRITE_DELAY * 2);
  cx.run_until_parked();
}

#[gpui::test]
fn untitled_crash_restart_restores_multiple_buffers_and_split_selection(cx: &mut TestAppContext) {
  let (project, page, cx) = setup(cx);
  let first = new_buffer(&page, "ébauche 🦀", cx);
  let second = new_buffer(&page, "second draft", cx);
  let empty = new_buffer(&page, "", cx);
  page.update(cx, |page, cx| {
    page.center_layout = CenterLayout::single(CenterSurface::from_tab(first.clone()));
    assert!(page.center_layout.split_active(
      CenterSurface::from_tab(second.clone()),
      CenterSplitDirection::Right
    ));
    page.remember_center_layout_tab(first.clone());
    page.persist_current_center_workspace(cx);
  });
  assert!(
    stored(&project.path)
      .iter()
      .all(|(_, snapshot)| snapshot.content.is_empty())
  );
  finish_debounce(cx);
  assert_eq!(stored(&project.path).len(), 3);
  page.update(cx, |page, _| page.untitled_buffers.clear());

  let restored = fresh_page(cx);
  restored.read_with(cx, |page, cx| {
    assert_eq!(
      page.center_tabs,
      vec![CenterTab::chat(), first.clone(), empty.clone()]
    );
    assert_eq!(page.active_center_tab.as_ref(), Some(&first));
    assert_eq!(page.center_layout.surface_count(), 2);
    assert_eq!(page.center_layout.active_tab(), &second);
    for (tab, text) in [
      (&first, "ébauche 🦀"),
      (&second, "second draft"),
      (&empty, ""),
    ] {
      let editor = page
        .editor_states
        .get(tab)
        .and_then(|state| state.editor.as_ref())
        .expect("restored editor")
        .read(cx);
      let snapshot = UntitledSnapshot::capture(editor, cx);
      assert_eq!(snapshot.content, text);
      assert_eq!(snapshot.dirty, !text.is_empty());
      assert_eq!(
        snapshot.selection,
        1.min(text.chars().count())..text.chars().count()
      );
      assert!(snapshot.selection_reversed);
      assert!(editor.is_untitled());
    }
  });
  let next = new_buffer(&restored, "", cx);
  assert!(![first, second, empty].contains(&next));
  assert_eq!(
    std::fs::read_dir(&project.path)
      .expect("project directory")
      .count(),
    0
  );
}

#[gpui::test]
fn untitled_quit_flushes_pending_edits_without_a_save_prompt(cx: &mut TestAppContext) {
  let (project, page, cx) = setup(cx);
  let tab = new_buffer(&page, "last keystroke", cx);
  page.update_in(cx, |page, window, cx| page.request_quit(window, cx));
  assert_eq!(
    stored(&project.path).first().expect("draft").1.content,
    "last keystroke"
  );
  assert!(!cx.update(|window, cx| window.has_active_dialog(cx)));
  let restored = fresh_page(cx);
  assert!(restored.read_with(cx, |page, _| page.editor_states.contains_key(&tab)));
}

#[gpui::test]
fn untitled_window_close_flushes_and_a_failed_flush_keeps_the_window(cx: &mut TestAppContext) {
  let (project, page, cx) = setup(cx);
  new_buffer(&page, "keep me", cx);
  let database = ConfigStore::test_db_path();
  ConfigStore::set_test_db_path(Some(project.path.clone()));
  page.update_in(cx, |page, window, cx| page.request_close_window(window, cx));
  assert_eq!(cx.windows().len(), 1);
  ConfigStore::set_test_db_path(database);
  page.update_in(cx, |page, window, cx| page.request_close_window(window, cx));
  assert!(cx.windows().is_empty());
  assert_eq!(
    stored(&project.path).first().expect("draft").1.content,
    "keep me"
  );
}

#[gpui::test]
fn close_button_leaves_removing_the_window_to_the_platform(cx: &mut TestAppContext) {
  let (project, page, cx) = setup(cx);
  new_buffer(&page, "keep me", cx);
  assert!(page.update_in(cx, |page, window, cx| page.window_can_close(window, cx)));
  assert_eq!(cx.windows().len(), 1);
  assert_eq!(
    stored(&project.path).first().expect("draft").1.content,
    "keep me"
  );
}

#[gpui::test]
fn untitled_drafts_survive_missing_or_incompatible_layouts(cx: &mut TestAppContext) {
  let (project, page, cx) = setup(cx);
  let tab = new_buffer(&page, "recover independently", cx);
  finish_debounce(cx);
  let checkout = SessionPage::canonical_repo(&project.path);
  for state in [
    None,
    Some("not json"),
    Some(r#"{"version":255,"terminals":[],"tabs":[],"layouts":[],"active_tab":null}"#),
  ] {
    if let Some(state) = state {
      ConfigStore::persist_center_workspace(&checkout, &checkout, state);
    } else {
      ConfigStore::forget_center_workspace(&checkout);
    }
    let restored = fresh_page(cx);
    assert!(restored.read_with(cx, |page, _| page.center_tabs.contains(&tab)));
    assert_eq!(
      stored(&project.path).first().expect("draft").1.content,
      "recover independently"
    );
  }
}

#[gpui::test]
async fn untitled_save_as_after_restore_reuses_editor_and_cleans_up(cx: &mut TestAppContext) {
  let (project, page, cx) = setup(cx);
  let tab = new_buffer(&page, "fn main() {}", cx);
  finish_debounce(cx);
  page.update(cx, |page, _| page.untitled_buffers.clear());
  let restored = fresh_page(cx);
  let editor = restored.read_with(cx, |page, _| page.shown_editor().expect("restored editor"));
  let selection = editor.read_with(cx, |editor, _| editor.selections.primary().range.clone());
  editor.update(cx, |editor, cx| editor.save(cx));
  cx.run_until_parked();
  assert!(cx.did_prompt_for_new_path());
  let prompt_task = restored
    .update(cx, |page, _| page.save_as_task.take())
    .expect("save prompt");
  let destination = SessionPage::canonical_repo(&project.path).join("main.rs");
  cx.simulate_new_path_selection(|_| Some(destination.clone()));
  prompt_task.await;
  editor
    .update(cx, |editor, _| editor.save_task.take())
    .expect("save task")
    .await;
  finish_debounce(cx);
  assert_eq!(
    std::fs::read_to_string(destination).expect("saved content"),
    "fn main() {}"
  );
  restored.read_with(cx, |page, cx| {
    assert_eq!(page.shown_editor(), Some(editor.clone()));
    assert_eq!(editor.read(cx).selections.primary().range, selection);
    assert!(!editor.read(cx).is_untitled());
    assert!(!page.editor_states.contains_key(&tab));
    assert!(page.untitled_buffers.is_empty());
  });
  assert!(stored(&project.path).is_empty());
  assert!(!fresh_page(cx).read_with(cx, |page, _| {
    page.center_tabs.iter().any(CenterTab::is_untitled)
  }));
}

#[gpui::test]
fn untitled_close_and_discard_cancel_pending_writes(cx: &mut TestAppContext) {
  let (project, page, cx) = setup(cx);
  let empty = new_buffer(&page, "", cx);
  page.update_in(cx, |page, window, cx| {
    page.close_center_tab(empty, window, cx)
  });
  assert!(stored(&project.path).is_empty());
  let dirty = new_buffer(&page, "discard me", cx);
  cx.run_until_parked();
  page.update_in(cx, |page, window, cx| {
    page.close_center_tab(dirty.clone(), window, cx);
    assert!(window.has_active_dialog(cx));
    page.discard_unsaved_editor_for_test(
      UnsavedEditorAction::CloseCenterGroups {
        groups: vec![page.center_layout.id()],
        discarded: Vec::new(),
        tab: dirty,
      },
      window,
      cx,
    );
  });
  finish_debounce(cx);
  assert!(stored(&project.path).is_empty());
  assert!(!fresh_page(cx).read_with(cx, |page, _| {
    page.center_tabs.iter().any(CenterTab::is_untitled)
  }));
}

#[gpui::test]
fn untitled_buffers_are_scoped_to_their_checkout(cx: &mut TestAppContext) {
  let (project, page, cx) = setup(cx);
  let first = new_buffer(&page, "first checkout", cx);
  finish_debounce(cx);
  let other = TempDir::new("untitled-other-checkout");
  let snapshot = serde_json::to_string(&UntitledSnapshot {
    content: "other checkout".into(),
    dirty: true,
    ..Default::default()
  })
  .expect("snapshot");
  let other_checkout = SessionPage::canonical_repo(&other.path);
  let second = CenterTab::untitled(
    ConfigStore::create_untitled_buffer(&other_checkout, &snapshot).expect("other draft"),
  );
  page.update_in(cx, |page, window, cx| {
    page.project_root = Some(other_checkout.clone());
    page.sync_active_checkout(window, cx);
    assert!(page.center_tabs.contains(&second));
    assert!(!page.center_tabs.contains(&first));
    page.project_root = Some(SessionPage::canonical_repo(&project.path));
    page.sync_active_checkout(window, cx);
    assert!(page.center_tabs.contains(&first));
    assert!(!page.center_tabs.contains(&second));
    assert!(page.editor_states.contains_key(&first));
    assert!(page.editor_states.contains_key(&second));
  });
}

#[gpui::test]
fn untitled_recovery_keeps_splits_with_a_placeholder_chat(cx: &mut TestAppContext) {
  let (_project, page, cx) = setup(cx);
  let tab = new_buffer(&page, "beside chat", cx);
  page.update(cx, |page, cx| {
    page.center_layout = CenterLayout::single(CenterSurface::from_tab(CenterTab::chat()));
    assert!(page.center_layout.split_active(
      CenterSurface::from_tab(tab.clone()),
      CenterSplitDirection::Right
    ));
    page.remember_center_layout_tab(CenterTab::chat());
    page.persist_current_center_workspace(cx);
  });
  finish_debounce(cx);
  fresh_page(cx).read_with(cx, |page, _| {
    assert_eq!(page.center_layout.surface_count(), 2);
    assert_eq!(page.center_layout.active_tab(), &tab);
  });
}

#[gpui::test]
fn untitled_recovery_rescues_a_draft_from_a_missing_split_representative(cx: &mut TestAppContext) {
  let (project, page, cx) = setup(cx);
  let tab = new_buffer(&page, "still recoverable", cx);
  let path = project.path.join("removed.txt");
  std::fs::write(&path, "temporary").expect("create file");
  page.update(cx, |page, cx| {
    let file = CenterTab::file(PathBuf::from("removed.txt"));
    page.center_layout = CenterLayout::single(CenterSurface::from_tab(file.clone()));
    assert!(page.center_layout.split_active(
      CenterSurface::from_tab(tab.clone()),
      CenterSplitDirection::Right
    ));
    page.remember_center_layout_tab(file);
    page.persist_current_center_workspace(cx);
  });
  finish_debounce(cx);
  std::fs::remove_file(path).expect("remove file");
  assert!(fresh_page(cx).read_with(cx, |page, _| page.center_tabs.contains(&tab)));
}

#[gpui::test]
async fn untitled_cancelled_and_failed_save_as_after_restore_keep_the_backup(
  cx: &mut TestAppContext,
) {
  let (project, page, cx) = setup(cx);
  let tab = new_buffer(&page, "keep after failed save", cx);
  finish_debounce(cx);
  page.update(cx, |page, _| page.untitled_buffers.clear());
  let restored = fresh_page(cx);
  let editor = restored.read_with(cx, |page, _| page.shown_editor().expect("restored editor"));
  editor.update(cx, |editor, cx| editor.save(cx));
  cx.run_until_parked();
  let prompt = restored
    .update(cx, |page, _| page.save_as_task.take())
    .expect("save prompt");
  cx.simulate_new_path_selection(|_| None);
  prompt.await;
  assert_eq!(stored(&project.path).len(), 1);
  let checkout = SessionPage::canonical_repo(&project.path);
  editor.update(cx, |editor, cx| {
    editor.save_as(checkout.clone(), checkout.join("missing/file.rs"), cx)
  });
  editor
    .update(cx, |editor, _| editor.save_task.take())
    .expect("save task")
    .await;
  finish_debounce(cx);
  assert!(restored.read_with(cx, |page, _| page.editor_states.contains_key(&tab)));
  assert!(editor.read_with(cx, |editor, _| editor.is_untitled() && editor.is_dirty));
  assert_eq!(
    stored(&project.path).first().expect("draft").1.content,
    "keep after failed save"
  );
}

#[test]
fn untitled_deleted_records_cannot_be_recreated_by_a_stale_write() {
  isolate_config_store_for_test();
  let checkout = Path::new("/test/checkout");
  let id = ConfigStore::create_untitled_buffer(checkout, "draft").expect("create draft");
  ConfigStore::forget_untitled_buffer(id).expect("delete draft");
  assert!(ConfigStore::persist_untitled_buffer(id, "stale draft").is_err());
  let next = ConfigStore::create_untitled_buffer(checkout, "new draft").expect("new draft");
  assert_ne!(id, next);
}
