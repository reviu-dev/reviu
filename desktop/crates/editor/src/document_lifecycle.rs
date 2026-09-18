use super::*;
use buffer::BufferVersion;
use std::io::{self, Write as _};

#[cfg(test)]
#[path = "document_lifecycle_tests.rs"]
mod tests;

#[derive(Clone, Debug)]
pub(crate) struct SelectionSnapshot {
  pub range: Range<usize>,
  pub reversed: bool,
}

#[derive(Debug)]
enum FileWrite {
  Written(Option<SystemTime>),
  Conflict(Option<Arc<str>>),
}

fn read_disk(path: &Path) -> io::Result<Option<Arc<str>>> {
  match std::fs::read_to_string(path) {
    Ok(contents) => Ok(Some(Arc::from(contents))),
    Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
    Err(error) => Err(error),
  }
}

fn write_file(path: &Path, expected: Option<&str>, contents: &str) -> io::Result<FileWrite> {
  let current = read_disk(path)?;
  if current.as_deref() != expected {
    return Ok(FileWrite::Conflict(current));
  }
  let target = match std::fs::canonicalize(path) {
    Ok(target) => target,
    Err(error) if error.kind() == io::ErrorKind::NotFound => {
      if std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_symlink()) {
        return Err(io::Error::other("The file's symbolic link has no target"));
      }
      path.to_path_buf()
    }
    Err(error) => return Err(error),
  };
  let parent = target
    .parent()
    .ok_or_else(|| io::Error::other("File has no parent directory"))?;
  static NEXT_TEMPORARY: AtomicUsize = AtomicUsize::new(0);
  let unique = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
  let temporary = parent.join(format!(".reviu-save-{}-{unique}.tmp", std::process::id()));
  let mut file = std::fs::OpenOptions::new()
    .write(true)
    .create_new(true)
    .open(&temporary)?;
  let result = (|| {
    if let Ok(metadata) = std::fs::metadata(&target) {
      // Atomic replacement must not bypass the original file's write permissions.
      let permissions = metadata.permissions();
      if permissions.readonly() {
        return Err(io::Error::new(
          io::ErrorKind::PermissionDenied,
          "The file is read-only",
        ));
      }
      std::fs::OpenOptions::new().write(true).open(&target)?;
      file.set_permissions(permissions)?;
    }
    file.write_all(contents.as_bytes())?;
    file.sync_all()?;
    let modified = file.metadata()?.modified().ok();
    drop(file);
    let current = read_disk(path)?;
    if current.as_deref() != expected {
      return Ok(FileWrite::Conflict(current));
    }
    if expected.is_some() && std::fs::canonicalize(path)? != target {
      return Err(io::Error::other(
        "The file's symbolic link changed while saving",
      ));
    }
    std::fs::rename(&temporary, &target)?;
    Ok(FileWrite::Written(modified))
  })();
  if !matches!(result, Ok(FileWrite::Written(_)))
    && let Err(error) = std::fs::remove_file(&temporary)
    && error.kind() != io::ErrorKind::NotFound
  {
    log::warn!("Could not remove temporary editor file: {error}");
  }
  result
}

impl Editor {
  pub(crate) fn selection_snapshot(&self, cx: &App) -> SelectionSnapshot {
    SelectionSnapshot {
      range: self.clamp_range_to_doc_len(self.selected_range.clone(), cx),
      reversed: self.selection_reversed && !self.selected_range.is_empty(),
    }
  }

  pub(crate) fn finalize_transaction(&mut self, cx: &mut Context<Self>) {
    self
      .document
      .update(cx, |document, _| document.buffer.finalize_transaction());
  }

  pub(crate) fn refresh_dirty(&mut self, cx: &App) {
    self.is_dirty =
      self.document.read(cx).buffer.is_dirty() || self.git_state.index_dirty || self.disk_conflict;
  }

  pub(super) fn restore_selection(&mut self, selection: SelectionSnapshot, cx: &mut Context<Self>) {
    self.selected_range = self.clamp_range_to_doc_len(selection.range, cx);
    self.selection_reversed = selection.reversed;
    self.display_selection = None;
    self.marked_range = None;
    self.mouse_selection = None;
    self.is_selecting = false;
    self.selection_autoscroll_task = None;
    self.target_column = None;
    self.selection_view = match self.diff_view_mode {
      DiffViewMode::Inline => DiffElementView::Inline,
      DiffViewMode::Split => DiffElementView::SplitRight,
    };
    self
      .cursor_blink
      .update(cx, |blink, cx| blink.pause_blinking(cx));
  }

  pub(super) fn invalidate_after_history_edit(&mut self, cx: &mut Context<Self>) {
    self.invalidate_projection_builds();
    self.mark_conflict_cache_dirty();
    self.line_layouts.clear();
    self.virtual_line_layouts.clear();
    self.word_diff_cache = WordDiffCache::default();
    self
      .document
      .update(cx, |document, cx| document.schedule_initial_highlights(cx));
    self.refresh_dirty(cx);
    self.refresh_find_matches_after_document_edit(cx);
    self.schedule_diff_recompute(cx);
    cx.notify();
  }

  pub(crate) fn undo_edit(&mut self, redo: bool, window: &mut Window, cx: &mut Context<Self>) {
    if self.selection_is_read_only() {
      return;
    }
    let stack = if redo {
      &mut self.redo_stack
    } else {
      &mut self.undo_stack
    };
    let Some(transaction) = stack.pop_back() else {
      return;
    };
    let result = self.document.update(cx, |document, cx| {
      if redo {
        document.redo(cx)
      } else {
        document.undo(cx)
      }
    });
    if result != Some(transaction.id) {
      log::error!("Editor selection history does not match buffer history");
      let stack = if redo {
        &mut self.redo_stack
      } else {
        &mut self.undo_stack
      };
      stack.push_back(transaction);
      return;
    }
    let selection = if redo {
      transaction.selection_after.clone()
    } else {
      transaction.selection_before.clone()
    };
    self.auto_pairs.restore(
      self.document.read(cx).buffer.version(),
      if redo {
        transaction.pairs_after.clone()
      } else {
        transaction.pairs_before.clone()
      },
    );
    self.pending_reload_scroll_anchor = None;
    self.restore_selection(selection, cx);
    if redo {
      self.undo_stack.push_back(transaction);
    } else {
      self.redo_stack.push_back(transaction);
    }
    self.invalidate_after_history_edit(cx);
    self.ensure_cursor_visible(window, cx);
  }

  fn save_error(&mut self, message: impl Into<Arc<str>>, cx: &mut Context<Self>) {
    let message = message.into();
    log::warn!("[editor] {message}");
    cx.emit(EditorEvent::SaveFailed { message });
    cx.notify();
  }

  pub fn save(&mut self, cx: &mut Context<Self>) {
    self.save_with_completion(cx, None);
  }

  pub fn save_with_completion(&mut self, cx: &mut Context<Self>, on_saved: Option<SaveCompletion>) {
    if self.is_read_only {
      return;
    }
    let Some(path) = self.workdir_path.clone() else {
      self.pending_untitled_save_completion = on_saved;
      cx.emit(EditorEvent::SavePathRequested);
      return;
    };
    if self.disk_conflict {
      self.save_error(
        "The file changed on disk. Choose Reload from disk or Overwrite disk before saving.",
        cx,
      );
      return;
    }
    self.save_to_path(path, None, on_saved, cx);
  }

  pub fn save_as(&mut self, repo_root: PathBuf, path: PathBuf, cx: &mut Context<Self>) {
    if self.is_read_only {
      return;
    }
    if self.workdir_path.as_ref() == Some(&path) && self.disk_conflict {
      self.save_error(
        "The file changed on disk. Resolve the disk conflict before saving to this path.",
        cx,
      );
      return;
    }
    let on_saved = self.pending_untitled_save_completion.take();
    self.save_to_path(path, Some(repo_root), on_saved, cx);
  }

  fn save_to_path(
    &mut self,
    path: PathBuf,
    repo_root: Option<PathBuf>,
    on_saved: Option<SaveCompletion>,
    cx: &mut Context<Self>,
  ) {
    if self.save_in_flight {
      self.save_error(
        "A save is already in progress. Please wait before saving again.",
        cx,
      );
      return;
    }
    self.finalize_transaction(cx);
    let document = self.document.read(cx);
    let version = document.buffer.version();
    let contents: Arc<str> = Arc::from(document.slice_to_string(0..document.len()));
    let same_path = self.workdir_path.as_ref() == Some(&path);
    let expected = self.disk_contents.clone();
    let repo_file = self.repo_file.clone();
    let index_text = self
      .git_state
      .bases
      .as_ref()
      .and_then(|bases| bases.index.clone());
    let needs_index_write = same_path && self.git_state.index_dirty;
    let saved_index_text = index_text.clone();
    let saved_path = path.clone();
    let saved_contents = contents.clone();
    self.save_in_flight = true;
    cx.notify();
    self.save_task = Some(cx.spawn(async move |this, cx| {
      let result = cx.background_spawn(async move {
        let expected = if same_path { expected } else { read_disk(&path)? };
        let outcome = write_file(&path, expected.as_deref(), &contents)?;
        let mut index_mtime = None;
        let mut index_error = None;
        if matches!(outcome, FileWrite::Written(_)) && needs_index_write
          && let (Some(repo_file), Some(index_text)) = (repo_file, index_text)
        {
          match git::write_index_content(&repo_file, &index_text) {
            Ok(()) => index_mtime = std::fs::metadata(git::index_path(&repo_file.repo_root)).and_then(|metadata| metadata.modified()).ok(),
            Err(error) => index_error = Some(error.to_string()),
          }
        }
        Ok::<_, io::Error>((outcome, index_mtime, index_error))
      }).await;
      let _ = this.update(cx, |editor, cx| {
        editor.save_in_flight = false;
        match result {
          Ok((FileWrite::Written(file_mtime), index_mtime, index_error)) => {
            if let Some(repo_root) = repo_root {
              editor.workdir_path = Some(saved_path.clone());
              editor.repo_file = RepoFile::new(&repo_root, &saved_path).ok();
              editor.git_store = editor.repo_file.as_ref().map(|file| GitStore::new(file.repo_root.clone()));
              let hint = Self::language_hint_for_path(&saved_path);
              editor.document.update(cx, |document, cx| document.set_language_hint(hint.as_deref(), cx));
              editor.start_polling(cx);
              cx.emit(EditorEvent::SavedAs { path: saved_path });
            }
            editor.complete_saved_version(version, saved_contents, file_mtime, cx);
            if needs_index_write && index_error.is_none() {
              let current = editor.git_state.bases.as_ref().and_then(|bases| bases.index.as_ref());
              if current == saved_index_text.as_ref() {
                editor.git_state.index_dirty = false;
                editor.optimistic_unstaged_groups.clear();
              }
              editor.index_mtime = index_mtime.or(editor.index_mtime);
              if let Some(store) = &editor.git_store { store.bump_op(); editor.git_state.op_id = store.op_id(); }
            }
            editor.refresh_dirty(cx);
            if let Some(error) = index_error {
              editor.save_error(format!("File saved, but the index could not be updated: {error}"), cx);
              return;
            }
            cx.emit(EditorEvent::Saved);
            if editor.git_diff_enabled {
              editor.reload_git_bases(cx);
              editor.schedule_diff_recompute(cx);
            }
            if let Some(on_saved) = on_saved {
              if editor.is_dirty {
                editor.save_error("The file was saved, but newer edits are still unsaved. Save again before closing.", cx);
              } else { on_saved(cx); }
            }
            cx.notify();
          }
          Ok((FileWrite::Conflict(contents), _, _)) => {
            if same_path {
              editor.disk_contents = contents;
              editor.disk_conflict = true;
              editor.disk_generation += 1;
              editor.refresh_dirty(cx);
            }
            editor.save_error("The file changed on disk while saving. Nothing was overwritten.", cx);
          }
          Err(error) => editor.save_error(format!("Could not save file: {error}"), cx),
        }
      });
    }));
  }

  fn complete_saved_version(
    &mut self,
    version: BufferVersion,
    contents: Arc<str>,
    modified: Option<SystemTime>,
    cx: &mut Context<Self>,
  ) {
    self
      .document
      .update(cx, |document, _| document.buffer.mark_saved(version));
    self.disk_contents = Some(contents);
    self.file_mtime = modified;
    self.disk_conflict = false;
    self.disk_generation += 1;
    self.refresh_dirty(cx);
  }

  pub(crate) fn overwrite_changed_file(&mut self, cx: &mut Context<Self>) {
    if !self.disk_conflict || self.save_in_flight {
      return;
    }
    if let Some(path) = self.workdir_path.clone() {
      self.save_to_path(path, None, None, cx);
    }
  }

  pub(super) fn render_disk_conflict(&self, cx: &mut Context<Self>) -> impl IntoElement {
    h_flex()
      .gap_2()
      .px_3()
      .py_2()
      .border_b_1()
      .border_color(cx.theme().border)
      .bg(cx.theme().muted)
      .text_sm()
      .child(div().flex_1().child(if self.disk_contents.is_some() {
        "File changed on disk. Your edits are kept. Reload can be undone."
      } else {
        "File removed on disk. Your buffer is kept."
      }))
      .child(
        Button::new("reload-editor-from-disk")
          .label("Reload from disk")
          .small()
          .disabled(self.save_in_flight || self.disk_contents.is_none())
          .on_click(cx.listener(|editor, _, _, cx| editor.reload_changed_file(cx))),
      )
      .child(
        Button::new("overwrite-editor-disk")
          .label(if self.disk_contents.is_some() {
            "Overwrite disk"
          } else {
            "Recreate file"
          })
          .small()
          .disabled(self.save_in_flight)
          .on_click(cx.listener(|editor, _, _, cx| editor.overwrite_changed_file(cx))),
      )
  }

  pub(crate) fn reload_changed_file(&mut self, cx: &mut Context<Self>) {
    if self.save_in_flight || !self.disk_conflict {
      return;
    }
    if let Some(contents) = self.disk_contents.clone() {
      self.apply_disk_contents(&contents, cx);
      self.disk_conflict = false;
      self.disk_generation += 1;
      self.refresh_dirty(cx);
      cx.notify();
    }
  }

  pub(super) fn observe_disk_contents(
    &mut self,
    contents: Option<Arc<str>>,
    modified: Option<SystemTime>,
    cx: &mut Context<Self>,
  ) {
    self.file_mtime = modified;
    if contents == self.disk_contents {
      return;
    }
    self.disk_contents = contents.clone();
    self.disk_generation += 1;
    let document = self.document.read(cx);
    if let Some(contents) = contents.as_deref()
      && document.slice_to_string(0..document.len()) == contents
    {
      let version = document.buffer.version();
      self
        .document
        .update(cx, |document, _| document.buffer.mark_saved(version));
      self.disk_conflict = false;
      self.refresh_dirty(cx);
      cx.notify();
      return;
    }
    if self.is_dirty || document.buffer.is_dirty() || contents.is_none() {
      let notify_conflict = !self.disk_conflict;
      self.disk_conflict = true;
      self.refresh_dirty(cx);
      if notify_conflict {
        self.save_error("The file changed on disk. Your buffer has been kept; resolve the conflict before saving.", cx);
      }
      cx.notify();
    } else if let Some(contents) = contents {
      self.apply_disk_contents(&contents, cx);
    }
  }

  fn apply_disk_contents(&mut self, contents: &str, cx: &mut Context<Self>) {
    let before = self.selection_snapshot(cx);
    let document = self.document.read(cx);
    let scroll_anchor = self
      .pending_reload_scroll_anchor
      .clone()
      .or_else(|| self.capture_scroll_anchor(document.len_lines()))
      .map(|anchor| {
        let offset =
          document.line_to_char(anchor.doc_line.min(document.len_lines().saturating_sub(1)));
        (anchor, offset)
      });
    let previous = document.slice_to_string(0..document.len());
    let prefix = previous
      .chars()
      .zip(contents.chars())
      .take_while(|(left, right)| left == right)
      .count();
    let previous_len = previous.chars().count();
    let next_len = contents.chars().count();
    let suffix = previous
      .chars()
      .rev()
      .zip(contents.chars().rev())
      .take(previous_len.min(next_len).saturating_sub(prefix))
      .take_while(|(left, right)| left == right)
      .count();
    let old_range = prefix..previous_len - suffix;
    let new_end = next_len - suffix;
    let replacement = contents
      .chars()
      .skip(prefix)
      .take(new_end - prefix)
      .collect::<String>();
    self.finalize_transaction(cx);
    let id = self.document.update(cx, |document, cx| {
      document.replace(old_range.clone(), &replacement, cx)
    });
    let map_offset = |offset: usize| {
      if offset <= old_range.start {
        offset
      } else if offset >= old_range.end {
        new_end + offset - old_range.end
      } else {
        offset.min(new_end)
      }
    };
    let selection = SelectionSnapshot {
      range: map_offset(before.range.start)..map_offset(before.range.end),
      reversed: before.reversed,
    };
    self.restore_selection(selection, cx);
    self.record_transaction(id, before, self.selected_range.clone(), cx);
    if let Some(transaction) = self.undo_stack.back_mut()
      && transaction.id == id
    {
      transaction.selection_after.reversed = self.selection_reversed;
    }
    self.document.update(cx, |document, _| {
      document.buffer.mark_saved(document.buffer.version());
      document.redetect_indentation();
    });
    if let Some((mut anchor, offset)) = scroll_anchor {
      let document = self.document.read(cx);
      anchor.doc_line = document.char_to_line(map_offset(offset).min(document.len()));
      let line_count = document.len_lines();
      if self.projection.is_some() {
        self.pending_reload_scroll_anchor = Some(anchor);
      } else {
        self.restore_scroll_anchor(anchor, line_count, line_count);
      }
    }
    self.invalidate_after_history_edit(cx);
  }

  pub(super) fn start_polling(&mut self, cx: &mut Context<Self>) {
    if self.poll_task.is_some() || self.workdir_path.is_none() {
      return;
    }
    self.poll_task = Some(cx.spawn(async move |this, cx| {
      loop {
        cx.background_executor()
          .timer(Duration::from_millis(POLL_INTERVAL_MS))
          .await;
        let state = this
          .update(cx, |editor, _| {
            (!editor.save_in_flight && !editor.is_read_only).then(|| {
              (
                editor.workdir_path.clone(),
                editor.repo_file.clone(),
                editor.file_mtime,
                editor.index_mtime,
                editor.disk_generation,
              )
            })
          })
          .ok();
        let Some(state) = state else {
          return;
        };
        let Some((Some(path), repo_file, previous_mtime, previous_index, generation)) = state
        else {
          continue;
        };
        let result = cx
          .background_spawn(async move {
            let modified = match std::fs::metadata(&path) {
              Ok(metadata) => metadata.modified().ok(),
              Err(error) if error.kind() == io::ErrorKind::NotFound => None,
              Err(error) => return Err(error),
            };
            let index_modified = repo_file
              .and_then(|file| std::fs::metadata(git::index_path(&file.repo_root)).ok())
              .and_then(|metadata| metadata.modified().ok());
            let contents = if modified != previous_mtime {
              Some(read_disk(&path)?)
            } else {
              None
            };
            Ok((contents, modified, index_modified))
          })
          .await;
        let _ = this.update(cx, |editor, cx| {
          if editor.save_in_flight || editor.disk_generation != generation || editor.is_read_only {
            return;
          }
          match result {
            Ok((contents, modified, index_modified)) => {
              let file_changed = contents.is_some();
              if let Some(contents) = contents {
                editor.observe_disk_contents(contents, modified, cx);
              }
              if index_modified != previous_index {
                editor.index_mtime = index_modified;
                if editor.git_diff_enabled {
                  editor.reload_git_bases(cx);
                }
              } else if file_changed && editor.git_diff_enabled {
                editor.schedule_diff_recompute(cx);
              }
            }
            Err(error) => {
              if !editor.disk_conflict {
                editor.disk_conflict = true;
                editor.refresh_dirty(cx);
                editor.save_error(
                  format!("Could not reload file; your buffer has been kept: {error}"),
                  cx,
                );
              }
            }
          }
        });
      }
    }));
  }
}
