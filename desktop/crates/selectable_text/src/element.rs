use std::ops::Range;

use gpui::{
  App, Bounds, ClipboardItem, CursorStyle, DispatchPhase, Element, ElementId, GlobalElementId,
  Hitbox, HitboxBehavior, Hsla, InspectorElementId, IntoElement, LayoutId, MouseButton,
  MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, SharedString, StyledText, TextRun, Window,
};
use gpui_component::ActiveTheme as _;

use crate::state::{ActiveSelection, SelectionMode, SelectionRegistry, normalize_range};
use crate::word::{clamp_to_char_boundary, line_range_at, word_range_at};

pub struct SelectableText {
  text: SharedString,
  runs: Vec<TextRun>,
  text_id: u64,
  registry: SelectionRegistry,
  selection_bg: Option<Hsla>,
  styled_text: StyledText,
}

impl SelectableText {
  pub fn new(
    text_id: u64,
    text: SharedString,
    runs: Vec<TextRun>,
    registry: SelectionRegistry,
  ) -> Self {
    let styled_text = if runs.is_empty() {
      StyledText::new(text.clone())
    } else {
      StyledText::new(text.clone()).with_runs(runs.clone())
    };
    Self {
      text,
      runs,
      text_id,
      registry,
      selection_bg: None,
      styled_text,
    }
  }

  pub fn selection_bg(mut self, color: Hsla) -> Self {
    self.selection_bg = Some(color);
    self
  }

  fn resolved_range(&self) -> Option<Range<usize>> {
    let active = self.registry.active_for(self.text_id)?;
    selection_range(&active, self.text.as_ref())
  }

  fn runs_with_selection(&self, window: &Window, cx: &App) -> Vec<TextRun> {
    let base_runs = if self.runs.is_empty() {
      let text_style = window.text_style();
      vec![TextRun {
        len: self.text.len(),
        font: text_style.font(),
        color: text_style.color,
        background_color: None,
        underline: None,
        strikethrough: None,
      }]
    } else {
      self.runs.clone()
    };

    let Some(range) = self.resolved_range() else {
      return base_runs;
    };
    let bg = self.selection_bg.unwrap_or_else(|| cx.theme().selection);
    apply_selection_to_runs(base_runs, range, bg)
  }
}

pub fn apply_selection_to_runs(
  runs: Vec<TextRun>,
  selection: Range<usize>,
  selection_color: Hsla,
) -> Vec<TextRun> {
  let mut updated = Vec::new();
  let mut offset = 0usize;
  for run in runs {
    let run_start = offset;
    let run_end = offset + run.len;
    offset = run_end;

    if selection.end <= run_start || selection.start >= run_end {
      updated.push(run);
      continue;
    }

    let overlap_start = selection.start.max(run_start);
    let overlap_end = selection.end.min(run_end);

    if overlap_start > run_start {
      let mut prefix = run.clone();
      prefix.len = overlap_start - run_start;
      updated.push(prefix);
    }

    let mut selected = run.clone();
    selected.len = overlap_end - overlap_start;
    selected.background_color = Some(selection_color);
    updated.push(selected);

    if overlap_end < run_end {
      let mut suffix = run.clone();
      suffix.len = run_end - overlap_end;
      updated.push(suffix);
    }
  }
  updated
}

pub fn selection_range(active: &ActiveSelection, text: &str) -> Option<Range<usize>> {
  let len = text.len();
  let mut range = normalize_range(active.anchor.min(len), active.head.min(len));
  range.start = clamp_to_char_boundary(text, range.start);
  range.end = clamp_to_char_boundary(text, range.end);
  if range.start >= range.end {
    None
  } else {
    Some(range)
  }
}

pub fn mode_for_click_count(count: usize) -> SelectionMode {
  match count {
    2 => SelectionMode::Word,
    3.. => SelectionMode::Line,
    _ => SelectionMode::Character,
  }
}

pub fn extend_selection(
  text: &str,
  mode: SelectionMode,
  anchor_word: Option<Range<usize>>,
  anchor_index: usize,
  head_index: usize,
) -> (usize, usize) {
  match mode {
    SelectionMode::Character => (anchor_index, head_index),
    SelectionMode::Word => {
      let anchor_word = anchor_word.unwrap_or(anchor_index..anchor_index);
      let head_word = word_range_at(text, head_index).unwrap_or(head_index..head_index);
      if head_index >= anchor_word.end {
        (anchor_word.start, head_word.end)
      } else if head_index < anchor_word.start {
        (anchor_word.end, head_word.start)
      } else {
        (anchor_word.start, anchor_word.end)
      }
    }
    SelectionMode::Line => {
      let anchor_line = line_range_at(text, anchor_index);
      let head_line = line_range_at(text, head_index);
      if head_index >= anchor_line.end {
        (anchor_line.start, head_line.end)
      } else if head_index < anchor_line.start {
        (anchor_line.end, head_line.start)
      } else {
        (anchor_line.start, anchor_line.end)
      }
    }
  }
}

impl Element for SelectableText {
  type RequestLayoutState = ();
  type PrepaintState = Hitbox;

  fn id(&self) -> Option<ElementId> {
    None
  }

  fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
    None
  }

  fn request_layout(
    &mut self,
    _id: Option<&GlobalElementId>,
    inspector_id: Option<&InspectorElementId>,
    window: &mut Window,
    cx: &mut App,
  ) -> (LayoutId, Self::RequestLayoutState) {
    let runs = self.runs_with_selection(window, cx);
    self.styled_text = StyledText::new(self.text.clone()).with_runs(runs);
    let (layout_id, _) = self
      .styled_text
      .request_layout(None, inspector_id, window, cx);
    (layout_id, ())
  }

  fn prepaint(
    &mut self,
    _id: Option<&GlobalElementId>,
    inspector_id: Option<&InspectorElementId>,
    bounds: Bounds<Pixels>,
    state: &mut Self::RequestLayoutState,
    window: &mut Window,
    cx: &mut App,
  ) -> Self::PrepaintState {
    self
      .styled_text
      .prepaint(None, inspector_id, bounds, state, window, cx);
    window.insert_hitbox(bounds, HitboxBehavior::Normal)
  }

  fn paint(
    &mut self,
    _id: Option<&GlobalElementId>,
    inspector_id: Option<&InspectorElementId>,
    bounds: Bounds<Pixels>,
    _request_layout: &mut Self::RequestLayoutState,
    hitbox: &mut Self::PrepaintState,
    window: &mut Window,
    cx: &mut App,
  ) {
    let text_layout = self.styled_text.layout().clone();
    let text_len = self.text.len();
    let text_for_event = self.text.clone();
    let registry = self.registry.clone();
    let text_id = self.text_id;

    if hitbox.is_hovered(window) {
      window.set_cursor_style(CursorStyle::IBeam, hitbox);
    }

    self
      .styled_text
      .paint(None, inspector_id, bounds, &mut (), &mut (), window, cx);

    let hitbox_down = hitbox.clone();
    let registry_down = registry.clone();
    let text_down = text_for_event.clone();
    let layout_down = text_layout.clone();
    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
      if phase != DispatchPhase::Bubble
        || event.button != MouseButton::Left
        || !hitbox_down.is_hovered(window)
      {
        return;
      }
      let index = clamp_to_char_boundary(
        text_down.as_ref(),
        layout_down
          .index_for_position(event.position)
          .unwrap_or_else(|ix| ix)
          .min(text_len),
      );
      let mode = mode_for_click_count(event.click_count);
      let anchor_word = match mode {
        SelectionMode::Word => word_range_at(text_down.as_ref(), index),
        SelectionMode::Line => Some(line_range_at(text_down.as_ref(), index)),
        SelectionMode::Character => None,
      };
      let (start, end) =
        extend_selection(text_down.as_ref(), mode, anchor_word.clone(), index, index);
      registry_down.set(text_id, start, end, true, mode, anchor_word);
      window.refresh();
      cx.stop_propagation();
    });

    let hitbox_move = hitbox.clone();
    let registry_move = registry.clone();
    let text_move = text_for_event.clone();
    let layout_move = text_layout.clone();
    window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, _cx| {
      if phase != DispatchPhase::Bubble {
        return;
      }
      let Some(active) = registry_move.active_for(text_id) else {
        return;
      };
      if !active.dragging {
        return;
      }
      let index = clamp_to_char_boundary(
        text_move.as_ref(),
        layout_move
          .index_for_position(event.position)
          .unwrap_or_else(|ix| ix)
          .min(text_len),
      );
      let (start, end) = extend_selection(
        text_move.as_ref(),
        active.mode,
        active.anchor_word.clone(),
        active.anchor,
        index,
      );
      registry_move.set(text_id, start, end, true, active.mode, active.anchor_word);
      window.refresh();
      let _ = hitbox_move;
    });

    let hitbox_up = hitbox.clone();
    let registry_up = registry.clone();
    let text_up = text_for_event.clone();
    let layout_up = text_layout.clone();
    window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
      if phase != DispatchPhase::Bubble {
        return;
      }
      let Some(active) = registry_up.active_for(text_id) else {
        return;
      };
      if !active.dragging {
        return;
      }
      let index = clamp_to_char_boundary(
        text_up.as_ref(),
        layout_up
          .index_for_position(event.position)
          .unwrap_or_else(|ix| ix)
          .min(text_len),
      );
      let (start, end) = extend_selection(
        text_up.as_ref(),
        active.mode,
        active.anchor_word.clone(),
        active.anchor,
        index,
      );
      registry_up.set(text_id, start, end, false, active.mode, active.anchor_word);

      if let Some(range) =
        selection_range(&registry_up.active_for(text_id).unwrap(), text_up.as_ref())
        && let Some(text) = text_up.as_ref().get(range)
      {
        cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
      }
      window.refresh();
      let _ = hitbox_up;
    });
  }
}

impl IntoElement for SelectableText {
  type Element = Self;

  fn into_element(self) -> Self::Element {
    self
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn mode_for_click_count_maps_to_modes() {
    assert_eq!(mode_for_click_count(1), SelectionMode::Character);
    assert_eq!(mode_for_click_count(2), SelectionMode::Word);
    assert_eq!(mode_for_click_count(3), SelectionMode::Line);
    assert_eq!(mode_for_click_count(7), SelectionMode::Line);
  }

  #[test]
  fn extend_character_returns_anchor_and_head() {
    let text = "hello world";
    let (start, end) = extend_selection(text, SelectionMode::Character, None, 0, 5);
    assert_eq!(start, 0);
    assert_eq!(end, 5);
  }

  #[test]
  fn extend_word_drags_forward_includes_word_end() {
    let text = "alpha beta gamma";
    let anchor_word = word_range_at(text, 2);
    let (start, end) = extend_selection(text, SelectionMode::Word, anchor_word, 2, 12);
    assert_eq!(&text[start..end], "alpha beta gamma");
  }

  #[test]
  fn extend_word_drags_backward_reverses_direction() {
    let text = "alpha beta gamma";
    let anchor_word = word_range_at(text, 12);
    let (start, end) = extend_selection(text, SelectionMode::Word, anchor_word, 12, 2);
    let lo = start.min(end);
    let hi = start.max(end);
    assert_eq!(&text[lo..hi], "alpha beta gamma");
  }

  #[test]
  fn extend_line_selects_full_lines() {
    let text = "first line\nsecond line\nthird line";
    let (start, end) = extend_selection(text, SelectionMode::Line, None, 5, 25);
    assert_eq!(&text[start..end], "first line\nsecond line\nthird line");
  }

  #[test]
  fn selection_range_clamps_to_text_bounds() {
    let text = "hello";
    let active = ActiveSelection {
      text_id: 0,
      anchor: 0,
      head: 99,
      dragging: false,
      mode: SelectionMode::Character,
      anchor_word: None,
    };
    let range = selection_range(&active, text).unwrap();
    assert_eq!(range, 0..5);
  }

  #[test]
  fn selection_range_returns_none_when_empty() {
    let text = "abc";
    let active = ActiveSelection {
      text_id: 0,
      anchor: 2,
      head: 2,
      dragging: false,
      mode: SelectionMode::Character,
      anchor_word: None,
    };
    assert!(selection_range(&active, text).is_none());
  }

  fn dummy_run(len: usize) -> TextRun {
    TextRun {
      len,
      font: gpui::Font {
        family: gpui::SharedString::from("monospace"),
        features: gpui::FontFeatures::default(),
        weight: gpui::FontWeight::NORMAL,
        style: gpui::FontStyle::Normal,
        fallbacks: None,
      },
      color: Hsla::default(),
      background_color: None,
      underline: None,
      strikethrough: None,
    }
  }

  #[test]
  fn apply_selection_splits_run_around_selection() {
    let bg = Hsla {
      h: 0.6,
      s: 0.8,
      l: 0.5,
      a: 1.0,
    };
    let runs = vec![dummy_run(10)];
    let updated = apply_selection_to_runs(runs, 3..7, bg);
    assert_eq!(updated.len(), 3);
    assert_eq!(updated[0].len, 3);
    assert!(updated[0].background_color.is_none());
    assert_eq!(updated[1].len, 4);
    assert_eq!(updated[1].background_color, Some(bg));
    assert_eq!(updated[2].len, 3);
    assert!(updated[2].background_color.is_none());
  }

  #[test]
  fn apply_selection_at_run_start_drops_prefix() {
    let bg = Hsla {
      h: 0.6,
      s: 0.8,
      l: 0.5,
      a: 1.0,
    };
    let runs = vec![dummy_run(10)];
    let updated = apply_selection_to_runs(runs, 0..4, bg);
    assert_eq!(updated.len(), 2);
    assert_eq!(updated[0].len, 4);
    assert_eq!(updated[0].background_color, Some(bg));
    assert_eq!(updated[1].len, 6);
  }

  #[test]
  fn apply_selection_spanning_multiple_runs_marks_each_overlap() {
    let bg = Hsla {
      h: 0.6,
      s: 0.8,
      l: 0.5,
      a: 1.0,
    };
    let runs = vec![dummy_run(4), dummy_run(4), dummy_run(4)];
    let updated = apply_selection_to_runs(runs, 2..10, bg);
    let selected_lens: Vec<usize> = updated
      .iter()
      .filter(|r| r.background_color == Some(bg))
      .map(|r| r.len)
      .collect();
    assert_eq!(selected_lens, vec![2, 4, 2]);
  }
}
