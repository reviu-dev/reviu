use super::editing::TextEdit;
use super::*;
use gpui::ClipboardItem;

#[cfg(test)]
#[path = "clipboard_tests.rs"]
mod tests;

const LINEWISE_METADATA: &str = "reviu:editor:linewise:v1";

impl Editor {
  fn clipboard_selection_is_empty(&self) -> bool {
    self.display_selection.as_ref().map_or_else(
      || self.selections.primary().range.is_empty(),
      DisplaySelection::is_empty,
    )
  }

  fn can_edit_clipboard_selection(&self, cx: &App) -> bool {
    if !self.can_edit_text(cx) {
      return false;
    }
    let Some(selection) = &self.display_selection else {
      return true;
    };
    let (start, end) = selection.normalized();
    let last = if end.column == 0 && end.line > start.line {
      end.line - 1
    } else {
      end.line
    };
    // Removed rows map to nearby document offsets, not editable text.
    let line_count = self.document.read(cx).len_lines();
    (start.line..=last).any(|line| {
      matches!(
        self.display_line(line, line_count),
        Some(DisplayLine::Doc { .. } | DisplayLine::Modified { .. })
      )
    })
  }

  fn current_line_clipboard_item(&self, cx: &App) -> Option<ClipboardItem> {
    let cursor = self.current_display_cursor(cx)?;
    let mut text = self.selection_line_text(cursor.line, self.selection_view, cx)?;
    let document = self.document.read(cx);
    let doc_line = match self.display_line(cursor.line, document.len_lines())? {
      DisplayLine::Doc { doc_line, .. } => Some(doc_line),
      DisplayLine::Modified { doc_line, .. }
        if self.selection_view != DiffElementView::SplitLeft =>
      {
        Some(doc_line)
      }
      _ => None,
    };
    if let Some(doc_line) = doc_line {
      text = document.slice_to_string(document.line_range(doc_line)?);
    }
    if !text.ends_with('\n') {
      text.push_str(document.line_ending());
    }
    Some(ClipboardItem::new_string_with_metadata(
      text,
      LINEWISE_METADATA.to_string(),
    ))
  }

  fn multiple_clipboard_parts(&self, cx: &App) -> Vec<(String, bool)> {
    let document = self.document.read(cx);
    self
      .selections
      .iter()
      .map(|selection| {
        let linewise = selection.range.is_empty();
        let range = if linewise {
          document
            .line_range(document.char_to_line(selection.head()))
            .unwrap_or(selection.range.clone())
        } else {
          selection.range.clone()
        };
        let mut text = document.slice_to_string(range);
        if linewise && !text.ends_with('\n') {
          text.push_str(document.line_ending());
        }
        (text, linewise)
      })
      .collect()
  }

  pub(crate) fn copy_to_clipboard(&self, cx: &mut Context<Self>) {
    if self.selections.len() > 1 {
      let parts = self.multiple_clipboard_parts(cx);
      let text = parts
        .iter()
        .map(|(text, _)| text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
      match serde_json::to_string(&parts) {
        Ok(metadata) => cx.write_to_clipboard(ClipboardItem::new_string_with_metadata(
          text,
          format!("reviu:selections:v1:{metadata}"),
        )),
        Err(error) => log::error!("Could not serialize clipboard selections: {error}"),
      }
      return;
    }
    let item = if self.clipboard_selection_is_empty() {
      self.current_line_clipboard_item(cx)
    } else {
      self
        .selected_text_for_copy(cx)
        .map(ClipboardItem::new_string)
    };
    if let Some(item) = item {
      cx.write_to_clipboard(item);
    }
  }

  pub(crate) fn cut_to_clipboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if !self.can_edit_clipboard_selection(cx) {
      return;
    }
    if self.selections.len() > 1 {
      self.copy_to_clipboard(cx);
      self.edit_selections(cx, |editor, selection, cx| {
        let document = editor.document.read(cx);
        let range = if selection.range.is_empty() {
          document
            .line_range(document.char_to_line(selection.head()))
            .unwrap_or(selection.range.clone())
        } else {
          selection.range.clone()
        };
        multicursor::EditPlan::replace(range, String::new())
      });
      return;
    }
    if self.clipboard_selection_is_empty() {
      let Some(item) = self.current_line_clipboard_item(cx) else {
        return;
      };
      let document = self.document.read(cx);
      let Some(range) = document.line_range(document.char_to_line(self.cursor_offset())) else {
        return;
      };
      let cursor = range.start;
      cx.write_to_clipboard(item);
      self.apply_text_edits(
        vec![TextEdit {
          range,
          text: String::new(),
        }],
        SelectionSnapshot {
          range: cursor..cursor,
          reversed: false,
        },
        cx,
      );
    } else if !self.selections.primary().range.is_empty() {
      self.finalize_transaction(cx);
      self.vertical_goal_x = None;
      cx.write_to_clipboard(ClipboardItem::new_string(
        self
          .document
          .read(cx)
          .slice_to_string(self.selections.primary().range.clone()),
      ));
      self.replace_text_in_range(None, "", window, cx);
      self.finalize_transaction(cx);
    }
  }

  pub(crate) fn paste_from_clipboard(&mut self, cx: &mut Context<Self>) {
    if !self.can_edit_clipboard_selection(cx) {
      return;
    }
    let Some(item) = cx.read_from_clipboard() else {
      return;
    };
    let Some(text) = item.text() else {
      return;
    };
    if self.selections.len() > 1 {
      let parts = item
        .metadata()
        .and_then(|metadata| metadata.strip_prefix("reviu:selections:v1:"))
        .and_then(|metadata| serde_json::from_str::<Vec<(String, bool)>>(metadata).ok())
        .filter(|parts| parts.len() == self.selections.len());
      let lines: Vec<_> = text.split('\n').collect();
      let plans = self
        .selections
        .iter()
        .enumerate()
        .map(|(index, selection)| {
          let (text, linewise) = parts
            .as_ref()
            .and_then(|parts| parts.get(index))
            .cloned()
            .unwrap_or_else(|| {
              if lines.len() == self.selections.len() {
                (
                  lines.get(index).copied().unwrap_or_default().to_string(),
                  false,
                )
              } else {
                (
                  text.clone(),
                  item.metadata().map(String::as_str) == Some(LINEWISE_METADATA),
                )
              }
            });
          let plan = if linewise && selection.range.is_empty() {
            let document = self.document.read(cx);
            let start = document.line_to_char(document.char_to_line(selection.head()));
            let cursor = selection.head() + text.chars().count();
            multicursor::EditPlan::new(
              vec![TextEdit {
                range: start..start,
                text,
              }],
              SelectionSnapshot {
                range: cursor..cursor,
                reversed: false,
              },
            )
          } else {
            multicursor::EditPlan::replace(selection.range.clone(), text)
          };
          (selection.id, plan)
        })
        .collect();
      self.apply_selection_plans(plans, cx);
      return;
    }
    if item.metadata().map(String::as_str) == Some(LINEWISE_METADATA)
      && self.clipboard_selection_is_empty()
      && self.marked_range.is_none()
    {
      let document = self.document.read(cx);
      let cursor = self.cursor_offset();
      let start = document.line_to_char(document.char_to_line(cursor));
      let cursor = cursor + text.chars().count();
      self.apply_text_edits(
        vec![TextEdit {
          range: start..start,
          text,
        }],
        SelectionSnapshot {
          range: cursor..cursor,
          reversed: false,
        },
        cx,
      );
    } else {
      self.finalize_transaction(cx);
      self.vertical_goal_x = None;
      self.replace_literal_text_in_range(None, &text, cx);
      self.finalize_transaction(cx);
    }
  }
}
