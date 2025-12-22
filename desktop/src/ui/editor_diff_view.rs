use crate::state::FileDiff;
use buffer_diff::{BufferDiff, BufferDiffSnapshot};
use gpui::{AppContext, Context, Entity, IntoElement, Render, Task, Window};
use language::Buffer;
use std::sync::Arc;

pub struct EditorDiffView {
  /// The editor displaying the diff
  editor: Entity<editor::Editor>,
  /// Old buffer (base)
  _old_buffer: Entity<Buffer>,
  /// New buffer (modified)
  _new_buffer: Entity<Buffer>,
  /// Background task for calculating diff
  _diff_task: Task<anyhow::Result<()>>,
}

impl EditorDiffView {
  /// Create a new EditorDiffView from old and new content
  pub fn new(file_diff: Arc<FileDiff>, window: &mut Window, cx: &mut Context<Self>) -> Self {
    // Extract old and new content from the file_diff
    let old_content = file_diff.old_content.clone().unwrap_or_default();
    let new_content = file_diff.new_content.clone().unwrap_or_default();

    // Create buffers for old and new content
    let old_buffer = cx.new(|cx| Buffer::local(old_content, cx));
    let new_buffer = cx.new(|cx| Buffer::local(new_content, cx));

    // Create BufferDiff for tracking changes
    let new_snapshot = new_buffer.read(cx).snapshot();
    let diff_buffer = cx.new(|cx| BufferDiff::new(&new_snapshot.text, cx));

    // Create MultiBuffer with the new buffer and add diff
    let multibuffer = cx.new(|cx| {
      let mut multibuffer = editor::MultiBuffer::singleton(new_buffer.clone(), cx);
      multibuffer.add_diff(diff_buffer.clone(), cx);
      multibuffer
    });

    // Create Editor for the MultiBuffer (read-only for diff viewing)
    let editor = cx.new(|cx| {
      let mut editor = editor::Editor::for_multibuffer(multibuffer.clone(), None, window, cx);

      // Configure editor for diff display
      editor.set_expand_all_diff_hunks(cx);
      editor.set_render_diff_hunk_controls(
        Arc::new(|_, _, _, _, _, _, _, _| gpui::Empty.into_any_element()),
        cx,
      );
      editor.set_read_only(true); // Read-only for diff viewing

      editor
    });

    // Start background task to calculate diff
    let diff_task = Self::calculate_diff(
      diff_buffer.clone(),
      old_buffer.clone(),
      new_buffer.clone(),
      cx,
    );

    Self {
      editor,
      _old_buffer: old_buffer,
      _new_buffer: new_buffer,
      _diff_task: diff_task,
    }
  }

  /// Calculate diff in the background
  fn calculate_diff(
    diff_buffer: Entity<BufferDiff>,
    old_buffer: Entity<Buffer>,
    new_buffer: Entity<Buffer>,
    cx: &mut Context<Self>,
  ) -> Task<anyhow::Result<()>> {
    cx.spawn(async move |_view, cx| {
      let new_snapshot = new_buffer.read_with(cx, |buffer, _| buffer.snapshot())?;
      let old_snapshot = old_buffer.read_with(cx, |buffer, _| buffer.snapshot())?;
      let old_text = old_snapshot.text();

      // Calculate diff asynchronously
      let diff_snapshot = cx
        .update(|cx| {
          BufferDiffSnapshot::new_with_base_buffer(
            new_snapshot.text.clone(),
            Some(Arc::new(old_text)),
            old_snapshot,
            cx,
          )
        })?
        .await;

      // Update the diff buffer with the calculated snapshot
      diff_buffer.update(cx, |diff, cx| {
        diff.set_snapshot(diff_snapshot, &new_snapshot.text, cx);
      })?;

      Ok(())
    })
  }
}

impl Render for EditorDiffView {
  fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
    // Simply render the editor - it handles everything (line numbers, diff colors, scrolling, etc.)
    self.editor.clone()
  }
}
