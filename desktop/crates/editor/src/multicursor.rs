use super::editing::{TextEdit, map_offset};
use super::*;
use crate::boundaries;

#[cfg(test)]
#[path = "multicursor_tests.rs"]
mod tests;

pub(super) struct EditPlan {
  edits: Vec<TextEdit>,
  selection: SelectionSnapshot,
}

impl EditPlan {
  pub(super) fn new(edits: Vec<TextEdit>, selection: SelectionSnapshot) -> Self {
    Self { edits, selection }
  }

  pub(super) fn replace(range: Range<usize>, text: String) -> Self {
    let cursor = range.start + text.chars().count();
    Self::new(
      vec![TextEdit { range, text }],
      SelectionSnapshot {
        range: cursor..cursor,
        reversed: false,
      },
    )
  }
}

fn merged_edits(plans: &[(u64, EditPlan)]) -> Option<Vec<TextEdit>> {
  let mut edits: Vec<_> = plans
    .iter()
    .flat_map(|(_, plan)| plan.edits.iter().cloned())
    .collect();
  edits.sort_by_key(|edit| (edit.range.start, edit.range.end));
  let mut merged: Vec<TextEdit> = Vec::new();
  for edit in edits {
    if let Some(previous) = merged.last_mut() {
      if *previous == edit {
        continue;
      }
      if previous.text.is_empty() && edit.text.is_empty() && edit.range.start <= previous.range.end
      {
        previous.range.end = previous.range.end.max(edit.range.end);
        continue;
      }
      if edit.range.start < previous.range.end
        || (edit.range.is_empty() && previous.range == edit.range)
      {
        return None;
      }
    }
    merged.push(edit);
  }
  Some(merged)
}

fn rebase_offset(offset: usize, own: &[TextEdit], edits: &[TextEdit]) -> usize {
  let mut removed = 0;
  let mut inserted = 0;
  for edit in own {
    let start = edit.range.start - removed + inserted;
    let end = start + edit.text.chars().count();
    if offset < start {
      break;
    }
    if offset <= end {
      if let Some(index) = edits.iter().position(|candidate| *candidate == *edit) {
        let mut base = edit.range.start;
        for previous in edits.iter().take(index) {
          base = base - previous.range.len() + previous.text.chars().count();
        }
        return base + offset - start;
      }
      return map_offset(edit.range.start, edits);
    }
    removed += edit.range.len();
    inserted += edit.text.chars().count();
  }
  map_offset(offset + removed - inserted, edits)
}

impl Editor {
  pub(crate) fn retain_primary_selection(&mut self, cx: &mut Context<Self>) -> bool {
    if self.selections.len() == 1 {
      return false;
    }
    self.finalize_transaction(cx);
    self.selections.single();
    self.composition_ranges = None;
    self.ensure_cursor_visible_when_hidden(cx);
    cx.notify();
    true
  }

  pub(super) fn edit_selections(
    &mut self,
    cx: &mut Context<Self>,
    plan: impl Fn(&Self, &Selection, &Context<Self>) -> EditPlan,
  ) {
    if !self.can_edit_text(cx) {
      return;
    }
    self.selections.normalize();
    let plans = self
      .selections
      .iter()
      .map(|selection| (selection.id, plan(self, selection, cx)))
      .collect();
    self.apply_selection_plans(plans, cx);
  }

  pub(super) fn apply_selection_plans(
    &mut self,
    plans: Vec<(u64, EditPlan)>,
    cx: &mut Context<Self>,
  ) {
    if !self.can_edit_text(cx) {
      return;
    }
    let Some(edits) = merged_edits(&plans) else {
      let message: Arc<str> =
        "These selections produce overlapping edits. Adjust the selections and try again.".into();
      log::warn!("{message}");
      cx.emit(EditorEvent::EditFailed { message });
      return;
    };
    let mut after = self.selections.clone();
    for selection in after.iter_mut() {
      if let Some((_, plan)) = plans.iter().find(|(id, _)| *id == selection.id) {
        selection.range = rebase_offset(plan.selection.range.start, &plan.edits, &edits)
          ..rebase_offset(plan.selection.range.end, &plan.edits, &edits);
        selection.reversed = plan.selection.reversed;
      } else {
        selection.range =
          map_offset(selection.range.start, &edits)..map_offset(selection.range.end, &edits);
      }
      selection.goal = None;
    }
    self.apply_selection_edits(edits, after, cx);
  }

  pub(super) fn apply_selection_edits(
    &mut self,
    edits: Vec<TextEdit>,
    after: Selections,
    cx: &mut Context<Self>,
  ) {
    let before = self.selection_snapshot(cx);
    let composing = self.composition_update;
    let continuing = composing && self.composition_ranges.is_some();
    if !composing {
      self.finalize_transaction(cx);
    }
    let document = self.document.read(cx);
    if edits
      .iter()
      .all(|edit| document.slice_to_string(edit.range.clone()) == edit.text)
    {
      self.restore_selections(after, cx);
      cx.notify();
      return;
    }
    let start_line = document.char_to_line(edits.first().map_or(0, |edit| edit.range.start));
    let end_line = document.char_to_line(edits.last().map_or(0, |edit| edit.range.end));
    self.auto_pairs.prepare(document.buffer.version(), &edits);
    self.maybe_optimistic_unstage_for_edit(start_line, end_line, cx);
    let id = self.document.update(cx, |document, cx| {
      let apply = |buffer: &mut buffer::TextBuffer,
                   transaction: &mut buffer::TransactionContext| {
        for edit in edits.iter().rev() {
          buffer.replace(transaction, edit.range.clone(), &edit.text);
        }
      };
      let id = if continuing {
        document.buffer.continue_transaction(Instant::now(), apply)
      } else {
        document.buffer.transaction(Instant::now(), apply)
      };
      cx.notify();
      id
    });
    self.restore_selections(after, cx);
    self.record_transaction(id, before, cx);
    if !composing {
      self.finalize_transaction(cx);
    }
    self.invalidate_after_history_edit(cx);
    self.ensure_cursor_visible_when_hidden(cx);
  }

  pub(super) fn replace_multiple_composition(
    &mut self,
    range_utf16: Option<Range<usize>>,
    text: &str,
    selected_utf16: Option<Range<usize>>,
    marked: bool,
    cx: &mut Context<Self>,
  ) {
    if !self.can_edit_text(cx) {
      return;
    }
    let ranges = self
      .composition_ranges
      .clone()
      .unwrap_or_else(|| self.selections.clone());
    let primary = ranges.primary().range.clone();
    let explicit = range_utf16
      .as_ref()
      .map(|range| self.range_from_utf16(range, cx));
    let plans = ranges
      .iter()
      .map(|selection| {
        let range = explicit.as_ref().map_or_else(
          || selection.range.clone(),
          |range| {
            let translate = |offset: usize| {
              selection
                .range
                .start
                .saturating_add_signed(offset as isize - primary.start as isize)
                .min(self.document.read(cx).len())
            };
            translate(range.start)..translate(range.end)
          },
        );
        (selection.id, EditPlan::replace(range, text.to_string()))
      })
      .collect();
    if self.composition_ranges.is_none() {
      self.finalize_transaction(cx);
    }
    self.composition_update = true;
    self.apply_selection_plans(plans, cx);
    self.composition_update = false;
    if marked && !text.is_empty() {
      let mut ranges = self.selections.clone();
      let selected = selected_utf16
        .as_ref()
        .map(|range| Self::utf16_range_to_char_range_in_text(text, range));
      for range in ranges.iter_mut() {
        range.range.start = range.range.end.saturating_sub(text.chars().count());
      }
      if let Some(selected) = selected {
        for selection in self.selections.iter_mut() {
          if let Some(range) = ranges.iter().find(|range| range.id == selection.id) {
            selection.range = range.range.start + selected.start..range.range.start + selected.end;
          }
        }
      }
      self.marked_range = Some(ranges.primary().range.clone());
      self.composition_ranges = Some(ranges);
    } else {
      self.marked_range = None;
      self.composition_ranges = None;
      self.finalize_transaction(cx);
    }
    if let Some(transaction) = self.undo_stack.back_mut() {
      transaction.selection_after = self.selections.clone();
    }
    cx.notify();
  }

  pub(crate) fn delete_selections(
    &mut self,
    backward: bool,
    word: bool,
    line: bool,
    cx: &mut Context<Self>,
  ) {
    self.edit_selections(cx, |editor, selection, cx| {
      let cursor = selection.head();
      let mut range = selection.range.clone();
      if range.is_empty() {
        if backward {
          range.start = if line {
            let document = editor.document.read(cx);
            document.line_to_char(document.char_to_line(cursor))
          } else if word {
            boundaries::previous_word_boundary(editor, cursor, cx)
          } else {
            boundaries::previous_boundary(editor, cursor, cx)
          };
          if !word
            && !line
            && let Some(pair) = editor.auto_pair_deletion(cursor, cx)
          {
            range = pair;
          }
        } else {
          range.end = boundaries::next_boundary(editor, cursor, cx);
        }
      }
      EditPlan::replace(range, String::new())
    });
  }

  pub(crate) fn select_occurrences(
    &mut self,
    all: bool,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.selection_view == DiffElementView::SplitLeft || self.is_read_only_display_cursor(cx) {
      return;
    }
    self.finalize_transaction(cx);
    self.display_selection = None;
    let document = self.document.read(cx);
    let source = document.slice_to_string(0..document.len());
    let from_caret = self
      .selections
      .iter()
      .any(|selection| selection.range.is_empty());
    if from_caret {
      for selection in self
        .selections
        .iter_mut()
        .filter(|selection| selection.range.is_empty())
      {
        let cursor = selection.head();
        let row = document.char_to_line(cursor);
        let start = document.line_to_char(row);
        let text = document.line_content(row).unwrap_or_default();
        let (left, right) = word_range_in_text(&text, cursor - start);
        selection.range = start + left..start + right;
        selection.goal = None;
      }
      self.selections.normalize();
      self.occurrence_wordwise = true;
      if !all {
        cx.notify();
        return;
      }
    }
    let initial = self.selections.primary().clone();
    let query = document.slice_to_string(initial.range.clone());
    if query.is_empty() {
      return;
    }
    let wordwise = self.occurrence_wordwise;
    let mut previous_byte = 0;
    let mut start = 0;
    let query_length = query.chars().count();
    let matches: Vec<_> = source
      .match_indices(&query)
      .filter_map(|(byte, _)| {
        start += source[previous_byte..byte].chars().count();
        previous_byte = byte;
        let end = byte + query.len();
        let word_character = |character: char| character.is_alphanumeric() || character == '_';
        if wordwise
          && (source[..byte]
            .chars()
            .next_back()
            .is_some_and(word_character)
            || source[end..].chars().next().is_some_and(word_character))
        {
          return None;
        }
        let range = start..start + query_length;
        (!self
          .selections
          .iter()
          .any(|selection| selection.range.start < range.end && range.start < selection.range.end))
        .then_some(range)
      })
      .collect();
    if all {
      self.selections.extend(matches, initial.reversed);
      self.selections.set_primary(initial.id);
    } else if let Some(range) = matches
      .iter()
      .find(|range| range.start >= initial.range.end)
      .or(matches.first())
    {
      self.selections.add(range.clone(), initial.reversed);
    }
    self.reveal_selection_gaps(!all, cx);
    if !all {
      self.ensure_cursor_visible(window, cx);
    }
    cx.notify();
  }

  fn reveal_selection_gaps(&mut self, scroll: bool, cx: &mut Context<Self>) {
    let document = self.document.read(cx);
    let selected_rows: Vec<_> = self
      .selections
      .iter()
      .map(|selection| {
        document.char_to_line(selection.range.start)..document.char_to_line(selection.range.end) + 1
      })
      .collect();
    let mut changed = false;
    for block in self.block_map.gap_blocks() {
      let (Some(hidden), Some(gap)) = (&block.hidden_range, block.gap_id()) else {
        continue;
      };
      let Some(end) = selected_rows
        .iter()
        .filter(|rows| rows.start < hidden.end && hidden.start < rows.end)
        .map(|rows| rows.end.min(hidden.end))
        .max()
      else {
        continue;
      };
      let reveal = self.expanded_gaps.entry(gap).or_default();
      reveal.head = reveal
        .head
        .max(end.saturating_sub(gap.start).saturating_add(SCROLL_PADDING));
      changed = true;
    }
    if changed {
      self.pending_selection_reveal = scroll;
      self.rebuild_projection(cx);
    }
  }

  pub(crate) fn add_selection_vertical(
    &mut self,
    direction: i32,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.selection_view == DiffElementView::SplitLeft || self.is_read_only_display_cursor(cx) {
      return;
    }
    let Some(cursor) = self.current_display_cursor(cx) else {
      return;
    };
    let Some(layout) = self.navigation_layout(cursor.line, window, cx) else {
      return;
    };
    let primary = self.selections.primary().clone();
    let goal = primary.goal.unwrap_or_else(|| {
      layout.x_for_index(char_offset_to_byte_offset(&layout.text, cursor.column))
    });
    let anchor_goal = self
      .current_display_anchor(cx)
      .and_then(|anchor| {
        self
          .navigation_layout(anchor.line, window, cx)
          .map(|layout| layout.x_for_index(char_offset_to_byte_offset(&layout.text, anchor.column)))
      })
      .unwrap_or(goal);
    let mut line = cursor.line;
    while let Some(next) = self.next_selectable_display_line(line, direction, cx) {
      line = next;
      if self.is_removed_display_line(line, cx) {
        continue;
      }
      let Some(layout) = self.navigation_layout(line, window, cx) else {
        continue;
      };
      if !primary.range.is_empty() && layout.width < goal.max(anchor_goal) {
        continue;
      }
      let byte = layout
        .text
        .grapheme_indices(true)
        .map(|(byte, _)| byte)
        .chain(std::iter::once(layout.text.len()))
        .min_by(|left, right| {
          (layout.x_for_index(*left) - goal)
            .abs()
            .partial_cmp(&(layout.x_for_index(*right) - goal).abs())
            .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or(0);
      let target = DisplayCursor {
        line,
        column: byte_offset_to_char_offset(&layout.text, byte),
      };
      let Some(offset) = self.doc_offset_for_display_cursor(target, cx) else {
        continue;
      };
      let anchor = if primary.range.is_empty() {
        offset
      } else {
        let byte = layout.closest_index_for_x(anchor_goal);
        let byte = layout
          .text
          .grapheme_indices(true)
          .map(|(byte, _)| byte)
          .chain(std::iter::once(layout.text.len()))
          .min_by_key(|candidate| candidate.abs_diff(byte))
          .unwrap_or(0);
        self
          .doc_offset_for_display_cursor(
            DisplayCursor {
              line,
              column: byte_offset_to_char_offset(&layout.text, byte),
            },
            cx,
          )
          .unwrap_or(offset)
      };
      let range = anchor.min(offset)..anchor.max(offset);
      if self.selections.iter().any(|selection| {
        selection.range == range
          || (selection.range.start < range.end && range.start < selection.range.end)
      }) {
        continue;
      }
      self.finalize_transaction(cx);
      self.display_selection = None;
      self.selections.add(range, primary.reversed);
      self.selections.primary_mut().goal = Some(goal);
      self.occurrence_wordwise = false;
      self.ensure_cursor_visible(window, cx);
      cx.notify();
      break;
    }
  }

  pub(crate) fn navigate_selections(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
    navigate: impl Fn(&mut Self, &mut Window, &mut Context<Self>),
  ) -> bool {
    if self.selections.len() == 1 {
      return false;
    }
    let mut moved = self.selections.clone();
    let primary = moved.primary().id;
    let scroll = self.scroll_offset_y;
    let mut primary_scroll = scroll;
    for selection in moved.iter_mut() {
      self.scroll_offset_y = scroll;
      self.selections = Selections::default();
      *self.selections.primary_mut() = Selection {
        id: 0,
        ..selection.clone()
      };
      self.display_selection = None;
      self.vertical_goal_x = selection.goal;
      navigate(self, window, cx);
      if !self.is_read_only_display_cursor(cx) {
        selection.range = self.selections.primary().range.clone();
        selection.reversed = self.selections.primary().reversed;
      }
      selection.goal = self.vertical_goal_x;
      if selection.id == primary {
        primary_scroll = self.scroll_offset_y;
      }
    }
    self.scroll_offset_y = primary_scroll;
    moved.normalize();
    self.selections = moved;
    self.display_selection = None;
    self.occurrence_wordwise = false;
    self.ensure_cursor_visible(window, cx);
    cx.notify();
    true
  }
}
