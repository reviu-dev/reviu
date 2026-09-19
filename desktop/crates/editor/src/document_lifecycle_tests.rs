use super::*;
use crate::editor::tests::EditorTestContext;
use gpui::{TestAppContext, VisualTestContext};
use gpui_component::Root;

fn setup<'a>(
  cx: &'a mut TestAppContext,
  text: &str,
) -> (Entity<Editor>, &'a mut VisualTestContext) {
  cx.update(gpui_component::init);
  let editor = EditorTestContext::with_text(cx.clone(), text).editor;
  let view = editor.clone();
  let (_, cx) = cx.add_window_view(move |window, cx| Root::new(view, window, cx));
  (editor, cx)
}

fn text(editor: &Entity<Editor>, cx: &VisualTestContext) -> String {
  editor.read_with(cx, |editor, cx| {
    editor
      .document
      .read(cx)
      .slice_to_string(0..editor.document.read(cx).len())
  })
}

fn type_text(editor: &Entity<Editor>, input: &str, cx: &mut VisualTestContext) {
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      editor.replace_text_in_range(None, input, window, cx)
    })
  });
}

fn undo(editor: &Entity<Editor>, redo: bool, cx: &mut VisualTestContext) {
  cx.update(|window, cx| editor.update(cx, |editor, cx| editor.undo_edit(redo, window, cx)));
}

struct Fixture(PathBuf);
impl Fixture {
  fn new(contents: &str) -> Self {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
      "reviu-editor-lifecycle-{}-{}",
      std::process::id(),
      NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&path).expect("fixture directory");
    let file = path.join("file.txt");
    std::fs::write(&file, contents).expect("fixture file");
    Self(file)
  }
}
impl Drop for Fixture {
  fn drop(&mut self) {
    if let Some(parent) = self.0.parent() {
      std::fs::remove_dir_all(parent).expect("remove fixture");
    }
  }
}

#[gpui::test]
fn unicode_undo_redo_restores_text_selection_and_saved_state(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "hello");
  type_text(&editor, "é", cx);
  assert_eq!(text(&editor, cx), "éhello");
  undo(&editor, false, cx);
  assert_eq!(text(&editor, cx), "hello");
  assert!(!editor.read_with(cx, |editor, _| editor.is_dirty));
  undo(&editor, true, cx);
  assert_eq!(text(&editor, cx), "éhello");
  assert!(editor.read_with(cx, |editor, _| editor.is_dirty));
  editor.update(cx, |editor, _| {
    editor.selections.primary_mut().range = 0..2;
    editor.selections.primary_mut().reversed = true;
  });
  type_text(&editor, "🙂", cx);
  undo(&editor, false, cx);
  assert_eq!(text(&editor, cx), "éhello");
  editor.read_with(cx, |editor, _| {
    assert_eq!(editor.selections.primary().range, 0..2);
    assert!(editor.selections.primary().reversed);
    assert!(editor.display_selection.is_none());
  });
}

#[gpui::test]
fn external_edits_preserve_dirty_buffer_and_explicit_reload_is_undoable(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "hello");
  editor.update(cx, |editor, _| {
    editor.disk_contents = Some(Arc::from("hello"))
  });
  type_text(&editor, "local ", cx);
  editor.update(cx, |editor, cx| {
    editor.observe_disk_contents(Some(Arc::from("agent text")), None, cx)
  });
  assert_eq!(text(&editor, cx), "local hello");
  assert!(editor.read_with(cx, |editor, _| editor.disk_conflict));
  editor.update(cx, |editor, cx| editor.reload_changed_file(cx));
  assert_eq!(text(&editor, cx), "agent text");
  assert!(!editor.read_with(cx, |editor, _| editor.is_dirty));
  undo(&editor, false, cx);
  assert_eq!(text(&editor, cx), "local hello");
  assert!(editor.read_with(cx, |editor, _| editor.is_dirty));
  undo(&editor, true, cx);
  assert_eq!(text(&editor, cx), "agent text");
  assert!(!editor.read_with(cx, |editor, _| editor.is_dirty));
}

#[gpui::test]
fn clean_reload_keeps_selection_and_viewport_instead_of_resetting(cx: &mut TestAppContext) {
  let original = (0..100)
    .map(|line| format!("line {line}\n"))
    .collect::<String>();
  let (editor, cx) = setup(cx, &original);
  editor.update(cx, |editor, cx| {
    editor.disk_contents = Some(Arc::from(original.as_str()));
    editor.selections.primary_mut().range = 40..44;
    editor.selections.primary_mut().reversed = true;
    editor.scroll_offset_y = 20.0;
    editor.observe_disk_contents(Some(Arc::from(format!("{original}extra\n"))), None, cx);
    assert_eq!(editor.selections.primary().range, 40..44);
    assert!(editor.selections.primary().reversed);
    assert_eq!(editor.scroll_offset_y, 20.0);
    assert!(!editor.is_dirty);
  });
}

#[gpui::test]
fn reload_rebases_every_selection_and_undo_restores_their_anchors(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "abc def ghi");
  editor.update(cx, |editor, cx| {
    editor.selections.primary_mut().range = 4..7;
    editor.selections.add(8..11, true);
    editor.apply_disk_contents("abc NEW def ghi", cx);
    assert_eq!(
      editor
        .selections
        .iter()
        .map(|selection| selection.range.clone())
        .collect::<Vec<_>>(),
      vec![8..11, 12..15]
    );
    assert!(editor.selections.primary().reversed);
  });
  undo(&editor, false, cx);
  assert_eq!(text(&editor, cx), "abc def ghi");
  editor.read_with(cx, |editor, _| {
    assert_eq!(
      editor
        .selections
        .iter()
        .map(|selection| selection.range.clone())
        .collect::<Vec<_>>(),
      vec![4..7, 8..11]
    );
    assert!(editor.selections.primary().reversed);
  });
}

#[gpui::test]
fn typing_during_save_remains_dirty_and_does_not_run_close_callback(cx: &mut TestAppContext) {
  let fixture = Fixture::new("hello");
  let (editor, cx) = setup(cx, "hello");
  let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
  let closed_callback = closed.clone();
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      editor.workdir_path = Some(fixture.0.clone());
      editor.disk_contents = Some(Arc::from("hello"));
      editor.replace_text_in_range(None, "a", window, cx);
      editor.save_with_completion(
        cx,
        Some(Box::new(move |_| {
          closed_callback.store(true, Ordering::SeqCst);
        })),
      );
      editor.replace_text_in_range(None, "b", window, cx);
    })
  });
  cx.run_until_parked();
  assert_eq!(
    std::fs::read_to_string(&fixture.0).expect("saved text"),
    "ahello"
  );
  assert_eq!(text(&editor, cx), "abhello");
  assert!(editor.read_with(cx, |editor, _| editor.is_dirty));
  assert!(!closed.load(Ordering::SeqCst));
  undo(&editor, false, cx);
  assert_eq!(text(&editor, cx), "ahello");
  assert!(!editor.read_with(cx, |editor, _| editor.is_dirty));
}

#[gpui::test]
fn saving_detects_disk_changes_even_before_polling(cx: &mut TestAppContext) {
  let fixture = Fixture::new("agent text");
  let (editor, cx) = setup(cx, "hello");
  editor.update(cx, |editor, _| {
    editor.workdir_path = Some(fixture.0.clone());
    editor.disk_contents = Some(Arc::from("hello"));
  });
  type_text(&editor, "local ", cx);
  editor.update(cx, |editor, cx| editor.save(cx));
  cx.run_until_parked();
  assert_eq!(
    std::fs::read_to_string(&fixture.0).expect("disk"),
    "agent text"
  );
  assert_eq!(text(&editor, cx), "local hello");
  assert!(editor.read_with(cx, |editor, _| editor.disk_conflict));
  editor.update(cx, |editor, cx| editor.overwrite_changed_file(cx));
  cx.run_until_parked();
  assert_eq!(
    std::fs::read_to_string(&fixture.0).expect("disk"),
    "local hello"
  );
  assert!(!editor.read_with(cx, |editor, _| editor.is_dirty));
}

#[gpui::test]
fn disk_deletion_keeps_the_buffer_and_requires_explicit_recreation(cx: &mut TestAppContext) {
  let fixture = Fixture::new("hello");
  let (editor, cx) = setup(cx, "hello");
  std::fs::remove_file(&fixture.0).expect("delete file");
  editor.update(cx, |editor, cx| {
    editor.workdir_path = Some(fixture.0.clone());
    editor.disk_contents = Some(Arc::from("hello"));
    editor.observe_disk_contents(None, None, cx);
    editor.save(cx);
  });
  cx.run_until_parked();
  assert!(!fixture.0.exists());
  assert_eq!(text(&editor, cx), "hello");
  editor.update(cx, |editor, cx| editor.overwrite_changed_file(cx));
  cx.run_until_parked();
  assert_eq!(
    std::fs::read_to_string(&fixture.0).expect("recreated"),
    "hello"
  );
}

#[gpui::test]
fn keyboard_deletion_restores_the_original_caret_and_selection(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "hé🙂");
  editor.update(cx, |editor, cx| editor.move_to(3, cx));
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      crate::actions::backspace(editor, &crate::actions::Backspace, window, cx);
    })
  });
  assert_eq!(text(&editor, cx), "hé");
  undo(&editor, false, cx);
  assert_eq!(text(&editor, cx), "hé🙂");
  assert_eq!(
    editor.read_with(cx, |editor, _| editor.selections.primary().range.clone()),
    3..3
  );
  type_text(&editor, "!", cx);
  editor.update(cx, |editor, cx| editor.select_to(2, cx));
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      crate::actions::delete(editor, &crate::actions::Delete, window, cx);
    })
  });
  assert_eq!(text(&editor, cx), "hé");
  undo(&editor, false, cx);
  assert_eq!(text(&editor, cx), "hé🙂!");
  editor.read_with(cx, |editor, _| {
    assert_eq!(editor.selections.primary().range, 2..4);
    assert!(editor.selections.primary().reversed);
  });
}

#[gpui::test]
fn reload_preserves_reading_position_when_lines_are_inserted_above(cx: &mut TestAppContext) {
  let original = (0..40)
    .map(|line| format!("line {line}\n"))
    .collect::<String>();
  let updated = format!("é\nnew header\n{original}");
  let inserted = "é\nnew header\n".chars().count();
  for projected in [false, true] {
    let (editor, window_context) = setup(cx, &original);
    editor.update(window_context, |editor, cx| {
      editor.disk_contents = Some(Arc::from(original.as_str()));
      editor.is_dirty = false;
      editor.editor_line_height = px(20.0);
      editor.viewport_height = px(60.0);
      editor.scroll_offset_y = 5.25;
      let lines = editor.document.read(cx).len_lines();
      editor.set_projection(projected.then(|| Projection::full(lines)));
      let cursor = editor.document.read(cx).line_to_char(7) + 2;
      editor.selections.primary_mut().range = cursor..cursor;
      editor.observe_disk_contents(
        Some(Arc::from(updated.as_str())),
        Some(SystemTime::now()),
        cx,
      );
      if projected {
        editor.apply_projection_result(Projection::full(lines + 2), lines + 2, cx);
      }
      assert_eq!(editor.scroll_offset_y, 7.25);
      assert_eq!(
        editor.selections.primary().range,
        cursor + inserted..cursor + inserted
      );
    });
  }
}

#[gpui::test]
fn restored_untitled_content_stays_unsaved_after_edit_and_undo(cx: &mut TestAppContext) {
  let (_, cx) = setup(cx, "");
  let editor =
    cx.new(|cx| Editor::new_untitled_with_content(PathBuf::from("/tmp"), "draft".to_string(), cx));
  assert!(editor.read_with(cx, |editor, _| editor.is_dirty));
  type_text(&editor, "new ", cx);
  undo(&editor, false, cx);
  assert_eq!(text(&editor, cx), "draft");
  assert!(editor.read_with(cx, |editor, _| editor.is_dirty));
}

#[gpui::test]
fn navigation_and_paste_start_separate_undo_steps(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "hello");
  type_text(&editor, "a", cx);
  type_text(&editor, "b", cx);
  editor.update(cx, |editor, cx| editor.move_to(7, cx));
  type_text(&editor, "!", cx);
  undo(&editor, false, cx);
  assert_eq!(text(&editor, cx), "abhello");
  undo(&editor, false, cx);
  assert_eq!(text(&editor, cx), "hello");
  undo(&editor, true, cx);
  cx.update(|window, cx| {
    cx.write_to_clipboard(gpui::ClipboardItem::new_string("é".to_string()));
    editor.update(cx, |editor, cx| {
      crate::actions::paste(editor, &crate::actions::Paste, window, cx)
    });
  });
  undo(&editor, false, cx);
  assert_eq!(text(&editor, cx), "abhello");
}

#[gpui::test]
fn ime_composition_is_recorded_and_separate_from_following_typing(cx: &mut TestAppContext) {
  let (editor, cx) = setup(cx, "hello");
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      editor.document.update(cx, |document, _| {
        document.buffer.set_group_interval(Duration::ZERO)
      });
      editor.replace_and_mark_text_in_range(None, "e", Some(1..1), window, cx);
      editor.replace_and_mark_text_in_range(None, "e\u{301}", Some(2..2), window, cx);
      editor.replace_text_in_range(None, "é", window, cx);
      editor.replace_text_in_range(None, "!", window, cx);
    })
  });
  assert_eq!(text(&editor, cx), "é!hello");
  undo(&editor, false, cx);
  assert_eq!(text(&editor, cx), "éhello");
  undo(&editor, false, cx);
  assert_eq!(text(&editor, cx), "hello");
  assert!(!editor.read_with(cx, |editor, _| editor.is_dirty));
}

#[gpui::test]
fn save_as_keeps_edits_made_after_its_snapshot(cx: &mut TestAppContext) {
  let fixture = Fixture::new("old target");
  let (editor, cx) = setup(cx, "hello");
  cx.update(|window, cx| {
    editor.update(cx, |editor, cx| {
      editor.save_as(
        fixture.0.parent().expect("parent").to_path_buf(),
        fixture.0.clone(),
        cx,
      );
      editor.replace_text_in_range(None, "new ", window, cx);
    })
  });
  cx.run_until_parked();
  assert_eq!(std::fs::read_to_string(&fixture.0).expect("disk"), "hello");
  assert_eq!(text(&editor, cx), "new hello");
  assert!(editor.read_with(cx, |editor, _| editor.is_dirty));
  undo(&editor, false, cx);
  assert!(!editor.read_with(cx, |editor, _| editor.is_dirty));
}

#[gpui::test]
fn failed_save_keeps_buffer_dirty_and_never_calls_completion(cx: &mut TestAppContext) {
  let fixture = Fixture::new("hello");
  let (editor, cx) = setup(cx, "hello");
  let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
  let callback = closed.clone();
  type_text(&editor, "new ", cx);
  editor.update(cx, |editor, cx| {
    editor.workdir_path = Some(fixture.0.join("missing/file.txt"));
    editor.save_with_completion(
      cx,
      Some(Box::new(move |_| callback.store(true, Ordering::SeqCst))),
    );
  });
  cx.run_until_parked();
  assert_eq!(text(&editor, cx), "new hello");
  assert!(editor.read_with(cx, |editor, _| editor.is_dirty));
  assert!(!editor.read_with(cx, |editor, _| editor.save_in_flight));
  assert!(!closed.load(Ordering::SeqCst));
}

#[gpui::test]
async fn polling_outside_git_protects_local_edits(cx: &mut TestAppContext) {
  let fixture = Fixture::new("hello");
  let (editor, cx) = setup(cx, "hello");
  editor.update(cx, |editor, cx| {
    editor.workdir_path = Some(fixture.0.clone());
    editor.disk_contents = Some(Arc::from("hello"));
    editor.start_polling(cx);
  });
  type_text(&editor, "local ", cx);
  std::fs::write(&fixture.0, "agent text").expect("external edit");
  cx.background_executor
    .timer(Duration::from_millis(POLL_INTERVAL_MS * 2))
    .await;
  cx.run_until_parked();
  assert_eq!(text(&editor, cx), "local hello");
  assert!(editor.read_with(cx, |editor, _| editor.disk_conflict));
  editor.update(cx, |editor, cx| editor.reload_changed_file(cx));
  assert_eq!(text(&editor, cx), "agent text");
}

#[cfg(unix)]
#[test]
fn atomic_save_respects_read_only_files_and_removes_its_temporary_file() {
  use std::os::unix::fs::PermissionsExt;
  let fixture = Fixture::new("protected");
  std::fs::set_permissions(&fixture.0, std::fs::Permissions::from_mode(0o444))
    .expect("permissions");
  let error =
    write_file(&fixture.0, Some("protected"), "replacement").expect_err("read-only target");
  assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
  assert_eq!(
    std::fs::read_to_string(&fixture.0).expect("disk"),
    "protected"
  );
  assert_eq!(
    std::fs::read_dir(fixture.0.parent().expect("parent"))
      .expect("directory")
      .count(),
    1
  );
}

#[test]
fn atomic_save_refuses_to_overwrite_unexpected_contents() {
  let fixture = Fixture::new("agent text");
  assert!(matches!(
    write_file(&fixture.0, Some("old"), "mine").expect("write outcome"),
    FileWrite::Conflict(_)
  ));
  assert_eq!(
    std::fs::read_to_string(&fixture.0).expect("disk"),
    "agent text"
  );
}

#[cfg(unix)]
#[test]
fn atomic_save_preserves_symlink_and_file_permissions() {
  use std::os::unix::fs::{PermissionsExt, symlink};
  let fixture = Fixture::new("old");
  std::fs::set_permissions(&fixture.0, std::fs::Permissions::from_mode(0o751))
    .expect("permissions");
  let link = fixture.0.with_file_name("link.txt");
  symlink(&fixture.0, &link).expect("symlink");
  assert!(matches!(
    write_file(&link, Some("old"), "new").expect("save"),
    FileWrite::Written(_)
  ));
  assert!(
    std::fs::symlink_metadata(&link)
      .expect("link metadata")
      .is_symlink()
  );
  assert_eq!(std::fs::read_to_string(&fixture.0).expect("target"), "new");
  assert_eq!(
    std::fs::metadata(&fixture.0)
      .expect("permissions")
      .permissions()
      .mode()
      & 0o777,
    0o751
  );
}
