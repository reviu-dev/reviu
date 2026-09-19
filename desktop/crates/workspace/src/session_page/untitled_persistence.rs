use super::*;

#[cfg(test)]
mod tests;

const WRITE_DELAY: Duration = Duration::from_millis(500);

#[derive(Default, serde::Deserialize, serde::Serialize)]
struct UntitledSnapshot {
  content: String,
  dirty: bool,
  selection: std::ops::Range<usize>,
  selection_reversed: bool,
}

pub(super) struct UntitledBuffer {
  pub(super) checkout_root: PathBuf,
  write_task: Option<Task<()>>,
  _subscriptions: Vec<gpui::Subscription>,
}

impl UntitledSnapshot {
  fn capture(editor: &Editor, cx: &App) -> Self {
    let document = editor.document().read(cx);
    Self {
      content: document.slice_to_string(0..document.len()),
      dirty: editor.is_dirty,
      selection: editor.selections.primary().range.clone(),
      selection_reversed: editor.selections.primary().reversed,
    }
  }
}

impl SessionPage {
  pub(super) fn create_untitled_buffer(&self, checkout_root: &Path) -> anyhow::Result<u64> {
    ConfigStore::create_untitled_buffer(
      &Self::canonical_repo(checkout_root),
      &serde_json::to_string(&UntitledSnapshot::default())?,
    )
  }

  pub(super) fn track_untitled_buffer(
    &mut self,
    tab: &CenterTab,
    checkout_root: PathBuf,
    editor: &Entity<Editor>,
    cx: &mut Context<Self>,
  ) {
    let Some(id) = tab.untitled_id() else { return };
    let document = editor.read(cx).document().clone();
    let document_subscription = cx.observe(&document, move |this, _, cx| {
      this.schedule_untitled_write(id, cx);
    });
    let mut selection = {
      let editor = editor.read(cx);
      (
        editor.selections.primary().range.clone(),
        editor.selections.primary().reversed,
        editor.is_dirty,
      )
    };
    let editor_subscription = cx.observe(editor, move |this, editor, cx| {
      let editor = editor.read(cx);
      let current = (
        editor.selections.primary().range.clone(),
        editor.selections.primary().reversed,
        editor.is_dirty,
      );
      if selection != current {
        selection = current;
        this.schedule_untitled_write(id, cx);
      }
    });
    self.untitled_buffers.insert(
      id,
      UntitledBuffer {
        checkout_root: Self::canonical_repo(&checkout_root),
        write_task: None,
        _subscriptions: vec![document_subscription, editor_subscription],
      },
    );
  }

  fn schedule_untitled_write(&mut self, id: u64, cx: &mut Context<Self>) {
    let Some(buffer) = self.untitled_buffers.get_mut(&id) else {
      return;
    };
    buffer.write_task = Some(cx.spawn(async move |this, cx| {
      cx.background_executor().timer(WRITE_DELAY).await;
      let _ = this.update(cx, |this, cx| {
        if let Err(error) = this.persist_untitled_buffer(id, cx) {
          this.report_untitled_error(error, cx);
        }
      });
    }));
  }

  fn persist_untitled_buffer(&self, id: u64, cx: &App) -> anyhow::Result<()> {
    let Some(editor) = self
      .editor_states
      .get(&CenterTab::untitled(id))
      .and_then(|state| state.editor.as_ref())
    else {
      return Ok(());
    };
    let snapshot = UntitledSnapshot::capture(editor.read(cx), cx);
    ConfigStore::persist_untitled_buffer(id, &serde_json::to_string(&snapshot)?)
  }

  pub(super) fn flush_untitled_buffers(&mut self, cx: &App) -> anyhow::Result<()> {
    for buffer in self.untitled_buffers.values_mut() {
      buffer.write_task = None;
    }
    for id in self.untitled_buffers.keys() {
      self.persist_untitled_buffer(*id, cx)?;
    }
    self.persist_current_center_workspace(cx);
    Ok(())
  }

  pub(super) fn remove_untitled_buffer(&mut self, tab: &CenterTab, cx: &mut Context<Self>) {
    let Some(id) = tab.untitled_id() else { return };
    self.untitled_buffers.remove(&id);
    if let Err(error) = ConfigStore::forget_untitled_buffer(id) {
      self.report_untitled_error(error, cx);
    }
  }

  pub(super) fn report_untitled_error(&self, error: anyhow::Error, cx: &mut Context<Self>) {
    log::error!("Untitled buffer persistence failed: {error:#}");
    let window_handle = self.window_handle;
    cx.defer(move |cx| {
      let _ = cx.update_window(window_handle, |_, window, cx| {
        window.push_notification(
          Notification::error(format!("Could not update local drafts: {error}")),
          cx,
        );
      });
    });
  }

  pub(super) fn restore_untitled_buffers(
    &mut self,
    checkout_root: &Path,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Vec<CenterTab> {
    let rows = match ConfigStore::load_untitled_buffers(&Self::canonical_repo(checkout_root)) {
      Ok(rows) => rows,
      Err(error) => {
        self.report_untitled_error(error, cx);
        return Vec::new();
      }
    };
    let mut tabs = Vec::new();
    for (id, state) in rows {
      let tab = CenterTab::untitled(id);
      if self.editor_states.contains_key(&tab) {
        tabs.push(tab);
        continue;
      }
      let snapshot = match serde_json::from_str::<UntitledSnapshot>(&state) {
        Ok(snapshot) => snapshot,
        Err(error) => {
          self.report_untitled_error(error.into(), cx);
          continue;
        }
      };
      let editor = cx.new(|cx| {
        let mut editor =
          Editor::new_untitled_with_content(checkout_root.to_path_buf(), snapshot.content, cx);
        let length = editor.document().read(cx).len();
        let start = snapshot.selection.start.min(length);
        editor.selections.primary_mut().range = start..snapshot.selection.end.clamp(start, length);
        editor.selections.primary_mut().reversed = snapshot.selection_reversed;
        editor.is_dirty = snapshot.dirty;
        editor.document.update(cx, |document, _| {
          if snapshot.dirty {
            document.buffer.mark_unsaved();
          } else {
            document.buffer.mark_saved(document.buffer.version());
          }
        });
        editor.set_git_diff_enabled(false, cx);
        editor
      });
      self.subscribe_untitled_editor(&editor, window, cx);
      self.track_untitled_buffer(&tab, checkout_root.to_path_buf(), &editor, cx);
      self.editor_states.insert(
        tab.clone(),
        CenterEditorState {
          selected_file: None,
          file_modified: None,
          editor: Some(editor),
          binary_preview: None,
          opened_snapshot: None,
        },
      );
      tabs.push(tab);
    }
    tabs
  }
}
