use gpui::{
  App, Bounds, ContentMask, DispatchPhase, ElementId, ElementInputHandler, Entity, GlobalElementId,
  Hitbox, HitboxBehavior, InspectorElementId, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent,
  MouseUpEvent, PaintQuad, Path, PathBuilder, Pixels, Point, ScrollDelta, ScrollWheelEvent,
  ShapedLine, Style, TextAlign, TextRun, TextStyle, Window, fill, point, prelude::*, px, relative,
  size,
};
use std::{
  collections::{HashMap, HashSet},
  ops::Range,
  rc::Rc,
  sync::Arc,
  time::{Duration, Instant},
};

use git::DiffLineKind;

use crate::{
  document::Document,
  editor::{DEFAULT_MAX_LINE_WIDTH, DisplayCursor, Editor, GroupOverlay, ScrollAxis},
  projection::{
    ChangeKind, DisplayLine, GAP_MARKER_TEXT, HunkState, NO_NEWLINE_MARKER_TEXT, Projection,
  },
};
use syntax::{HighlightSpan, Theme};
use gpui_component::ActiveTheme as _;

// Visual width for empty line selection indicator
const NEWLINE_SELECTION_WIDTH: f32 = 4.0;
// Scroll sensitivity for pixel-based scrolling (trackpad)
const PIXEL_SCROLL_DIVISOR: f32 = 20.0;
// Scroll sensitivity for line-based scrolling (mouse wheel)
const LINE_SCROLL_MULTIPLIER: f32 = 3.0;
const SCROLL_AXIS_RATIO: f32 = 1.1;
const SCROLL_AXIS_SWITCH_RATIO: f32 = 1.4;
const SCROLL_AXIS_TIMEOUT_MS: u64 = 150;
const DIAGONAL_STRIPE_SPACING: f32 = 6.0;
const DIAGONAL_STRIPE_WIDTH: f32 = 1.0;

/// Encapsulates layout information for mouse position -> text offset conversion
#[derive(Clone)]
pub struct PositionMap {
  pub shaped_lines: Vec<(usize, Arc<ShapedLine>)>,
  pub bounds: Bounds<Pixels>,
  pub line_height: Pixels,
  pub viewport: Range<usize>,
  pub scroll_offset: f32,
  pub projection: Option<Arc<Projection>>,
}

impl PositionMap {
  pub fn display_line_for_position(
    &self,
    position: Point<Pixels>,
  ) -> Option<usize> {
    if !self.bounds.contains(&position) {
      return None;
    }

    let y_offset = position.y - self.bounds.top();
    let line_float = self.scroll_offset + (y_offset / self.line_height);
    if line_float.is_sign_negative() {
      return None;
    }
    let mut display_line = line_float.floor() as usize;
    if display_line >= self.viewport.end {
      display_line = self.viewport.end.saturating_sub(1);
    }
    Some(display_line)
  }

  pub fn display_cursor_for_position(
    &self,
    position: Point<Pixels>,
  ) -> Option<DisplayCursor> {
    if !self.bounds.contains(&position) {
      return None;
    }

    let y_offset = position.y - self.bounds.top();
    let line_float = self.scroll_offset + (y_offset / self.line_height);
    if line_float.is_sign_negative() {
      return None;
    }
    let mut actual_row = line_float.floor() as usize;
    if actual_row >= self.viewport.end {
      actual_row = self.viewport.end.saturating_sub(1);
    }

    let x_offset = position.x - self.bounds.left();
    let column = self
      .shaped_lines
      .iter()
      .find(|(idx, _)| *idx == actual_row)
      .map(|(_, shaped)| shaped.closest_index_for_x(x_offset))
      .unwrap_or(0);

    Some(DisplayCursor {
      line: actual_row,
      column,
    })
  }

  pub fn point_for_position(&self, position: Point<Pixels>, document: &Document) -> Option<usize> {
    if !self.bounds.contains(&position) {
      return None;
    }

    if document.is_empty() {
      return Some(0);
    }

    let y_offset = position.y - self.bounds.top();
    let line_float = self.scroll_offset + (y_offset / self.line_height);
    if line_float.is_sign_negative() {
      return None;
    }
    let mut actual_row = line_float.floor() as usize;
    if actual_row >= self.viewport.end {
      actual_row = self.viewport.end.saturating_sub(1);
    }

    let doc_line = if let Some(projection) = &self.projection {
      projection.display_to_doc_line(actual_row)?
    } else {
      actual_row
    };

    if doc_line >= document.len_lines() {
      return Some(document.len());
    }

    let shaped = self
      .shaped_lines
      .iter()
      .find(|(idx, _)| *idx == actual_row)
      .map(|(_, s)| s)?;

    let x_offset = position.x - self.bounds.left();
    let column = shaped.closest_index_for_x(x_offset);

    let line_start = document.line_to_char(doc_line);
    Some(line_start + column)
  }
}

/// Helper to convert syntax highlights to TextRuns for rendering
pub(crate) fn highlights_to_text_runs(
  highlights: &[HighlightSpan],
  line_text: &str,
  theme: &Theme,
  base_style: &TextStyle,
) -> Vec<TextRun> {
  let mut runs = Vec::new();
  let line_len = line_text.len();
  let mut current_pos = 0;

  // Filter and clip highlights for this line
  let mut line_highlights: Vec<_> = highlights
    .iter()
    .filter_map(|h| {
      let start = h.byte_range.start.min(line_len);
      let end = h.byte_range.end.min(line_len);
      (end > start).then_some((start..end, h.token_type))
    })
    .collect();

  line_highlights.sort_by_key(|(range, _)| range.start);

  for (range, token_type) in line_highlights {
    // Gap before highlight (normal text)
    if range.start > current_pos {
      runs.push(TextRun {
        len: range.start - current_pos,
        font: base_style.font(),
        color: base_style.color,
        background_color: None,
        underline: None,
        strikethrough: None,
      });
    }

    // The highlighted span
    runs.push(TextRun {
      len: range.len(),
      font: base_style.font(),
      color: theme.syntax().color_for_token(token_type),
      background_color: None,
      underline: None,
      strikethrough: None,
    });

    current_pos = range.end;
  }

  // Final gap
  if current_pos < line_len {
    runs.push(TextRun {
      len: line_len - current_pos,
      font: base_style.font(),
      color: base_style.color,
      background_color: None,
      underline: None,
      strikethrough: None,
    });
  }

  runs
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditorElementRole {
  Primary,
  Secondary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DiffElementView {
  Inline,
  SplitLeft,
  SplitRight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineVisibility {
  Text,
  Blank,
}

pub struct EditorElement {
  editor: Entity<Editor>,
  diff_view: DiffElementView,
  role: EditorElementRole,
}

pub struct PrepaintState {
  shaped_lines: Vec<(usize, Arc<ShapedLine>)>,
  line_backgrounds: Vec<PaintQuad>,
  group_borders: Vec<PaintQuad>,
  diag_paths: Vec<Path<Pixels>>,
  cursor_quad: Option<PaintQuad>,
  selection_quads: Vec<PaintQuad>,
  viewport: Range<usize>,
  bounds: Bounds<Pixels>,
  line_height: Pixels,
  scroll_hitbox: Hitbox,
  projection: Option<Arc<Projection>>,
}

impl EditorElement {
  pub fn new(editor: Entity<Editor>) -> Self {
    Self {
      editor,
      diff_view: DiffElementView::Inline,
      role: EditorElementRole::Primary,
    }
  }

  fn diff_view(mut self, diff_view: DiffElementView) -> Self {
    self.diff_view = diff_view;
    self
  }

  fn role(mut self, role: EditorElementRole) -> Self {
    self.role = role;
    self
  }

  pub fn split_left(editor: Entity<Editor>) -> Self {
    Self::new(editor)
      .diff_view(DiffElementView::SplitLeft)
      .role(EditorElementRole::Secondary)
  }

  pub fn split_right(editor: Entity<Editor>) -> Self {
    Self::new(editor).diff_view(DiffElementView::SplitRight)
  }

  fn calculate_viewport(
    &self,
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    scroll_offset: f32,
    total_lines: usize,
  ) -> Range<usize> {
    let visible_line_count = ((bounds.size.height / line_height).ceil() as usize).max(1);

    let start_line = (scroll_offset.floor() as usize).min(total_lines.saturating_sub(1));
    let end_line = (start_line + visible_line_count).min(total_lines);

    start_line..end_line
  }

  fn is_primary(&self) -> bool {
    self.role == EditorElementRole::Primary
  }

  fn line_visibility(&self, display_line: &DisplayLine) -> LineVisibility {
    match self.diff_view {
      DiffElementView::Inline => LineVisibility::Text,
      DiffElementView::SplitLeft => match display_line {
        DisplayLine::Doc {
          change: Some(ChangeKind::Added),
          ..
        } => LineVisibility::Blank,
        _ => LineVisibility::Text,
      },
      DiffElementView::SplitRight => match display_line {
        DisplayLine::Removed { .. } => LineVisibility::Blank,
        _ => LineVisibility::Text,
      },
    }
  }
}

impl IntoElement for EditorElement {
  type Element = Self;

  fn into_element(self) -> Self::Element {
    self
  }
}

impl Element for EditorElement {
  type RequestLayoutState = ();
  type PrepaintState = PrepaintState;

  fn id(&self) -> Option<ElementId> {
    None
  }

  fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
    None
  }

  fn request_layout(
    &mut self,
    _id: Option<&GlobalElementId>,
    _inspector_id: Option<&InspectorElementId>,
    window: &mut Window,
    cx: &mut App,
  ) -> (LayoutId, Self::RequestLayoutState) {
    let mut style = Style::default();
    style.size.width = relative(1.).into();
    style.size.height = relative(1.).into();

    (window.request_layout(style, [], cx), ())
  }

  fn prepaint(
    &mut self,
    _id: Option<&GlobalElementId>,
    _inspector_id: Option<&InspectorElementId>,
    bounds: Bounds<Pixels>,
    _request_layout: &mut Self::RequestLayoutState,
    window: &mut Window,
    cx: &mut App,
  ) -> Self::PrepaintState {
    let scroll_hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
    let is_primary = self.is_primary();

    // Check if syntax highlights have been updated and invalidate cache if needed
    let (highlights_epoch, highlights_version, dirty_highlight_lines) = {
      let document = self.editor.read(cx).document().read(cx);
      let epoch = *document.highlights_epoch.read();
      let version = *document.highlights_version.read();
      let dirty = document.drain_dirty_highlight_lines();
      (epoch, version, dirty)
    };
    self.editor.update(cx, |editor, _| {
      if is_primary {
        editor.viewport_height = bounds.size.height;
        editor.viewport_width = bounds.size.width;
      }

      if editor.scroll_axis_lock == Some(ScrollAxis::Vertical)
        && editor.scroll_handle.offset().x != editor.last_scroll_x
      {
        editor
          .scroll_handle
          .set_offset(point(editor.last_scroll_x, px(0.0)));
      }

      if highlights_epoch > editor.last_highlights_epoch {
        editor.line_layouts.clear();
        editor.last_highlights_epoch = highlights_epoch;
        editor.last_highlights_version = highlights_version;
      } else if highlights_version > editor.last_highlights_version {
        for line_idx in &dirty_highlight_lines {
          editor.line_layouts.remove(line_idx);
        }
        editor.last_highlights_version = highlights_version;
      }
    });

    let (
      viewport,
      selected_range,
      display_selection,
      cursor_offset,
      mut shaped_lines,
      lines_to_shape,
      viewport_lines,
      projection,
    ) = {
      let editor = self.editor.read(cx);
      let document = editor.document().read(cx);
      let line_height = window.line_height();
      let scroll_offset = editor.scroll_offset_y;
      let doc_line_count = document.len_lines();
      let total_lines = editor.display_line_count(doc_line_count);

      let viewport = self.calculate_viewport(bounds, line_height, scroll_offset, total_lines);

      let mut lines_to_shape = Vec::new();
      let mut shaped_lines = Vec::new();
      let mut viewport_lines = Vec::new();
      let projection = editor.projection.clone();

      for display_idx in viewport.clone() {
        let Some(display_line) = editor.display_line(display_idx, doc_line_count) else {
          continue;
        };
        viewport_lines.push((display_idx, display_line.clone()));

        let doc_line_for_layout = match &display_line {
          DisplayLine::Doc { doc_line, .. } => Some(*doc_line),
          DisplayLine::Modified { doc_line, .. }
            if matches!(
              self.diff_view,
              DiffElementView::SplitRight | DiffElementView::Inline
            ) =>
          {
            Some(*doc_line)
          }
          _ => None,
        };

        if let Some(doc_line) = doc_line_for_layout {
          match editor.line_layouts.get(&doc_line) {
            Some(shaped) => {
              shaped_lines.push((display_idx, Arc::clone(shaped)));
            }
            None => {
              lines_to_shape.push((display_idx, display_line));
            }
          }
        } else {
          match editor.virtual_line_layouts.get(&display_idx) {
            Some(shaped) => {
              shaped_lines.push((display_idx, Arc::clone(shaped)));
            }
            None => {
              lines_to_shape.push((display_idx, display_line));
            }
          }
        }
      }

      (
        viewport,
        editor.selected_range.clone(),
        editor.display_selection.clone(),
        editor.cursor_offset(),
        shaped_lines,
        lines_to_shape,
        viewport_lines,
        projection,
      )
    };

    let style = window.text_style();
    let font_size = style.font_size.to_pixels(window.rem_size());
    let line_height = window.line_height();

    // Get theme for syntax highlighting colors
    let theme = self.editor.read(cx).theme.clone();

    let document = self.editor.read(cx).document().read(cx);
    let mut newly_shaped = Vec::new();
    for (display_idx, display_line) in lines_to_shape {
      let (mut line_text, doc_line, base_color, allow_highlights) = match &display_line {
        DisplayLine::Doc { doc_line, change, .. } => {
          let content = document
            .line_content(*doc_line)
            .map(|cow| cow.into_owned())
            .unwrap_or_default();
          let base_color = if matches!(change, Some(ChangeKind::Added)) {
            style.color
          } else {
            style.color
          };
          (content, Some(*doc_line), base_color, true)
        }
        DisplayLine::Modified {
          old_text,
          doc_line,
          ..
        } => match self.diff_view {
          DiffElementView::SplitLeft => {
            (old_text.clone(), None, theme.diff_removed_text(), false)
          }
          DiffElementView::SplitRight => {
            let content = document
              .line_content(*doc_line)
              .map(|cow| cow.into_owned())
              .unwrap_or_default();
            (content, Some(*doc_line), style.color, true)
          }
          DiffElementView::Inline => {
            let content = document
              .line_content(*doc_line)
              .map(|cow| cow.into_owned())
              .unwrap_or_default();
            (content, Some(*doc_line), style.color, true)
          }
        },
        DisplayLine::Removed { text, .. } => {
          let color = theme.diff_removed_text();
          (text.clone(), None, color, false)
        }
        DisplayLine::Gap { .. } => {
          (GAP_MARKER_TEXT.to_string(), None, cx.theme().muted_foreground, false)
        }
        DisplayLine::NoNewline { .. } => (
          NO_NEWLINE_MARKER_TEXT.to_string(),
          None,
          cx.theme().muted_foreground,
          false,
        ),
      };
      if line_text.contains('\n') || line_text.contains('\r') {
        line_text = line_text.replace(['\n', '\r'], "");
      }

      let runs = if allow_highlights {
        if let Some(doc_line) = doc_line
          && let Some(highlights) = document.get_highlights_for_line(doc_line)
        {
          highlights_to_text_runs(highlights.as_ref(), &line_text, &theme, &style)
        } else {
          vec![TextRun {
            len: line_text.len(),
            font: style.font(),
            color: base_color,
            background_color: None,
            underline: None,
            strikethrough: None,
          }]
        }
      } else {
        vec![TextRun {
          len: line_text.len(),
          font: style.font(),
          color: base_color,
          background_color: None,
          underline: None,
          strikethrough: None,
        }]
      };

      let shaped = window
        .text_system()
        .shape_line(line_text.into(), font_size, &runs, None);
      newly_shaped.push((display_idx, doc_line, shaped));
    }

    if !newly_shaped.is_empty() {
      self.editor.update(cx, |editor, _| {
        for (display_idx, doc_line, shaped) in newly_shaped {
          // Wrap in Arc for cheap cloning
          let shaped_arc = Arc::new(shaped);
          if let Some(doc_line) = doc_line {
            editor.line_layouts.insert(doc_line, shaped_arc.clone());
          } else {
            editor
              .virtual_line_layouts
              .insert(display_idx, shaped_arc.clone());
          }
          shaped_lines.push((display_idx, shaped_arc));
        }
        // Limit cache size to prevent memory issues with large files
        editor.ensure_cache_size(viewport.clone());
      });
    }

    // Calculate maximum line width for horizontal scrolling
    let max_width = shaped_lines
      .iter()
      .map(|(_, shaped)| shaped.width)
      .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
      .unwrap_or(px(DEFAULT_MAX_LINE_WIDTH));

    if is_primary {
      self.editor.update(cx, |editor, _| {
        editor.max_line_width = editor.max_line_width.max(max_width);
      });
    }

    let mut line_backgrounds = Vec::new();
    let mut group_borders = Vec::new();
    let mut diag_paths = Vec::new();
    let added_bg = theme.diff_added_background();
    let added_staged_bg = theme.diff_added_staged_background();
    let removed_bg = theme.diff_removed_background();
    let removed_staged_bg = theme.diff_removed_staged_background();
    let mut group_border_colors: HashMap<Arc<str>, (gpui::Hsla, gpui::Hsla)> = HashMap::new();
    if let Some(projection) = projection.as_ref() {
      for (group_id, group) in &projection.groups {
        if group.state != HunkState::Staged {
          continue;
        }
        let mut has_add = false;
        let mut has_remove = false;
        let mut first_kind: Option<DiffLineKind> = None;
        let mut last_kind: Option<DiffLineKind> = None;
        for line in &group.hunk.lines {
          match line.kind {
            DiffLineKind::Add => {
              has_add = true;
              if first_kind.is_none() {
                first_kind = Some(line.kind);
              }
              last_kind = Some(line.kind);
            }
            DiffLineKind::Remove => {
              has_remove = true;
              if first_kind.is_none() {
                first_kind = Some(line.kind);
              }
              last_kind = Some(line.kind);
            }
            DiffLineKind::Context => {}
          }
        }

        if has_add && has_remove {
          let removed = theme.diff_gutter_removed();
          let added = theme.diff_gutter_added();
          let (top_color, bottom_color) = match self.diff_view {
            DiffElementView::SplitLeft => (removed, removed),
            DiffElementView::SplitRight => (added, added),
            DiffElementView::Inline => (removed, added),
          };
          group_border_colors.insert(group_id.clone(), (top_color, bottom_color));
          continue;
        }

        let (Some(first_kind), Some(last_kind)) = (first_kind, last_kind) else {
          continue;
        };
        let top_color = match first_kind {
          DiffLineKind::Add => theme.diff_gutter_added(),
          DiffLineKind::Remove => theme.diff_gutter_removed(),
          DiffLineKind::Context => theme.diff_gutter_modified(),
        };
        let bottom_color = match last_kind {
          DiffLineKind::Add => theme.diff_gutter_added(),
          DiffLineKind::Remove => theme.diff_gutter_removed(),
          DiffLineKind::Context => theme.diff_gutter_modified(),
        };
        group_border_colors.insert(group_id.clone(), (top_color, bottom_color));
      }
    }

    let mut blank_line_set = HashSet::new();
    if !matches!(self.diff_view, DiffElementView::Inline) {
      for (display_idx, display_line) in &viewport_lines {
        if self.line_visibility(display_line) == LineVisibility::Blank {
          blank_line_set.insert(*display_idx);
        }
      }
    }

    for (display_idx, display_line) in &viewport_lines {
      let background = match display_line {
        DisplayLine::Doc {
          change: Some(ChangeKind::Added),
          secondary,
          ..
        } if !matches!(self.diff_view, DiffElementView::SplitLeft) => {
          Some(if *secondary { added_staged_bg } else { added_bg })
        }
        DisplayLine::Removed { secondary, .. }
          if !matches!(self.diff_view, DiffElementView::SplitRight) =>
        {
          Some(if *secondary { removed_staged_bg } else { removed_bg })
        }
        DisplayLine::Modified { secondary, .. } => match self.diff_view {
          DiffElementView::SplitLeft => Some(if *secondary {
            removed_staged_bg
          } else {
            removed_bg
          }),
          DiffElementView::SplitRight => Some(if *secondary {
            added_staged_bg
          } else {
            added_bg
          }),
          DiffElementView::Inline => None,
        },
        _ => None,
      };

      if let Some(color) = background {
        let y = bounds.top() + line_height * (*display_idx - viewport.start) as f32;
        line_backgrounds.push(fill(
          Bounds::new(point(bounds.left(), y), size(bounds.size.width, line_height)),
          color,
        ));
      }

      let group_id = match display_line {
        DisplayLine::Doc { group_id, .. } => group_id.as_ref(),
        DisplayLine::Modified { group_id, .. } => group_id.as_ref(),
        DisplayLine::Removed { group_id, .. } => group_id.as_ref(),
        DisplayLine::NoNewline { group_id, .. } => group_id.as_ref(),
        _ => None,
      };

      if let (Some(projection), Some(group_id)) = (projection.as_ref(), group_id) {
        if let Some((top_color, bottom_color)) = group_border_colors.get(group_id.as_ref()) {
          let prev_group = display_idx
            .checked_sub(1)
            .and_then(|idx| projection.lines.get(idx))
            .and_then(|line| match line {
              DisplayLine::Doc { group_id, .. } => group_id.as_ref(),
              DisplayLine::Modified { group_id, .. } => group_id.as_ref(),
              DisplayLine::Removed { group_id, .. } => group_id.as_ref(),
              DisplayLine::NoNewline { group_id, .. } => group_id.as_ref(),
              _ => None,
            });
          let next_group = projection
            .lines
            .get(display_idx + 1)
            .and_then(|line| match line {
              DisplayLine::Doc { group_id, .. } => group_id.as_ref(),
              DisplayLine::Modified { group_id, .. } => group_id.as_ref(),
              DisplayLine::Removed { group_id, .. } => group_id.as_ref(),
              DisplayLine::NoNewline { group_id, .. } => group_id.as_ref(),
              _ => None,
            });

          let is_top = prev_group.map(|id| id.as_ref()) != Some(group_id.as_ref());
          let is_bottom = next_group.map(|id| id.as_ref()) != Some(group_id.as_ref());
          let border_thickness = px(1.0);
          let y = bounds.top() + line_height * (*display_idx - viewport.start) as f32;

          if is_top {
            group_borders.push(fill(
              Bounds::new(point(bounds.left(), y), size(bounds.size.width, border_thickness)),
              *top_color,
            ));
          }

          if is_bottom {
            group_borders.push(fill(
              Bounds::new(
                point(bounds.left(), y + line_height - border_thickness),
                size(bounds.size.width, border_thickness),
              ),
              *bottom_color,
            ));
          }
        }
      }
    }

    if !blank_line_set.is_empty() {
      let mut blank_ranges = Vec::new();
      let mut current_start: Option<usize> = None;
      for (display_idx, _) in &viewport_lines {
        if blank_line_set.contains(display_idx) {
          if current_start.is_none() {
            current_start = Some(*display_idx);
          }
        } else if let Some(start) = current_start.take() {
          blank_ranges.push((start, display_idx.saturating_sub(1)));
        }
      }
      if let Some(start) = current_start.take() {
        if let Some((last_idx, _)) = viewport_lines.last() {
          blank_ranges.push((start, *last_idx));
        }
      }

      let stripe_spacing = px(DIAGONAL_STRIPE_SPACING);
      let stripe_width = px(DIAGONAL_STRIPE_WIDTH);
      for (start, end) in blank_ranges {
        let y = bounds.top() + line_height * (start - viewport.start) as f32;
        let height = line_height * (end - start + 1) as f32;
        let top = y;
        let bottom = y + height;
        let left = bounds.left();
        let right = bounds.right();
        let mut builder = PathBuilder::stroke(stripe_width);
        let start_n = ((left - height + bottom) / stripe_spacing).floor();
        let mut x = start_n * stripe_spacing - bottom;
        let end_x = right + height;
        while x < end_x {
          builder.move_to(point(x, bottom));
          builder.line_to(point(x + height, top));
          x += stripe_spacing;
        }
        if let Ok(path) = builder.build() {
          diag_paths.push(path);
        }
      }
    }

    if !blank_line_set.is_empty() {
      shaped_lines.retain(|(idx, _)| !blank_line_set.contains(idx));
    }

    if is_primary {
      let mut overlays = Vec::new();
      let mut seen = HashSet::new();
      for (display_idx, display_line) in &viewport_lines {
      let (group_id, state) = match display_line {
        DisplayLine::Doc {
          change: Some(ChangeKind::Added),
          hunk: Some(state),
          group_id: Some(id),
          ..
        } => (id.clone(), *state),
        DisplayLine::Modified {
          hunk,
          group_id: Some(id),
          ..
        } => (id.clone(), *hunk),
        DisplayLine::Removed {
          hunk,
          group_id: Some(id),
          ..
        } => (id.clone(), *hunk),
        _ => continue,
      };

        if !seen.insert(group_id.clone()) {
          continue;
        }

        let y = bounds.top() + line_height * (*display_idx - viewport.start) as f32;
        overlays.push(GroupOverlay {
          id: group_id,
          state,
          display_line: *display_idx,
          y,
        });
      }

      self.editor.update(cx, |editor, cx| {
        editor.visible_groups = overlays;

        if !editor.is_selecting {
          if let Some(position) = editor.last_mouse_position {
            if bounds.contains(&position) {
              let scroll_offset = editor.scroll_offset_y;
              let y_offset = position.y - bounds.top();
              let line_float = scroll_offset + (y_offset / line_height);
              if !line_float.is_sign_negative() {
                let mut display_line = line_float.floor() as usize;
                if display_line >= viewport.end {
                  display_line = viewport.end.saturating_sub(1);
                }
                let hovered = editor.group_id_for_modified_display_line(display_line);
                if editor.hovered_group_id.as_deref() != hovered.as_deref() {
                  editor.hovered_group_id = hovered;
                  cx.notify();
                }
              }
            }
          }
        }
      });
    }

    let document = self.editor.read(cx).document().read(cx);
    let display_selection = display_selection.clone();
    let display_cursor = display_selection.as_ref().map(|selection| selection.end);

    let cursor_quad = if let Some(display_cursor) = display_cursor
      && viewport.contains(&display_cursor.line)
    {
      let shaped_opt = shaped_lines
        .iter()
        .find(|(idx, _)| *idx == display_cursor.line)
        .map(|(_, shaped)| shaped);
      let line_text = match self.editor.read(cx).display_line(display_cursor.line, document.len_lines()) {
        Some(DisplayLine::Doc { doc_line, .. }) => document
          .line_content(doc_line)
          .map(|cow| cow.into_owned())
          .unwrap_or_default(),
        Some(DisplayLine::Modified { doc_line, .. }) => document
          .line_content(doc_line)
          .map(|cow| cow.into_owned())
          .unwrap_or_default(),
        Some(DisplayLine::Removed { text, .. }) => text,
        Some(DisplayLine::NoNewline { .. }) => NO_NEWLINE_MARKER_TEXT.to_string(),
        _ => String::new(),
      };
      if let Some(shaped) = shaped_opt {
        let line_len = line_text.len();
        let cursor_in_line = display_cursor.column.min(line_len);
        let cursor_x = shaped.x_for_index(cursor_in_line);
        let y = bounds.top() + line_height * (display_cursor.line - viewport.start) as f32;
        Some(fill(
          Bounds::new(
            point(bounds.left() + cursor_x, y),
            size(px(2.), line_height),
          ),
          theme.cursor(),
        ))
      } else {
        None
      }
    } else {
      let cursor_doc_line = document.char_to_line(cursor_offset);
      let cursor_display_line = self
        .editor
        .read(cx)
        .doc_to_display_line(cursor_doc_line);
      if let Some(cursor_line) = cursor_display_line
        && viewport.contains(&cursor_line)
      {
        let shaped_opt = shaped_lines
          .iter()
          .find(|(idx, _)| *idx == cursor_line)
          .map(|(_, shaped)| shaped);
        if let Some(shaped) = shaped_opt {
          let line_start = document.line_to_char(cursor_doc_line);
          let cursor_in_line = cursor_offset - line_start;
          let cursor_x = shaped.x_for_index(cursor_in_line);
          let y = bounds.top() + line_height * (cursor_line - viewport.start) as f32;
          Some(fill(
            Bounds::new(
              point(bounds.left() + cursor_x, y),
              size(px(2.), line_height),
            ),
            theme.cursor(),
          ))
        } else {
          None
        }
      } else {
        None
      }
    };

    let mut selection_quads = Vec::new();
    let display_selection = display_selection.filter(|selection| !selection.is_empty());
    if let Some(selection) = display_selection {
      let (start, end) = selection.normalized();
      let mut end_line = end.line;
      if end.column == 0 && end.line > start.line {
        end_line = end_line.saturating_sub(1);
      }

      for display_line in start.line..=end_line {
        if !viewport.contains(&display_line) {
          continue;
        }

        let line_text = match self.editor.read(cx).display_line(display_line, document.len_lines()) {
          Some(DisplayLine::Doc { doc_line, .. }) => document
            .line_content(doc_line)
            .map(|cow| cow.into_owned())
            .unwrap_or_default(),
          Some(DisplayLine::Modified { doc_line, .. }) => document
            .line_content(doc_line)
            .map(|cow| cow.into_owned())
            .unwrap_or_default(),
          Some(DisplayLine::Removed { text, .. }) => text,
          _ => continue,
        };

        let shaped_opt = shaped_lines
          .iter()
          .find(|(idx, _)| *idx == display_line)
          .map(|(_, shaped)| shaped);

        if let Some(shaped) = shaped_opt {
          let line_len = line_text.len();
          let line_start = if display_line == start.line {
            start.column.min(line_len)
          } else {
            0
          };
          let line_end = if display_line == end_line && display_line == end.line {
            end.column.min(line_len)
          } else {
            line_len
          };

          let x_start = shaped.x_for_index(line_start);
          let x_end = shaped.x_for_index(line_end);
          let y = bounds.top() + line_height * (display_line - viewport.start) as f32;

          let is_selecting_newline = display_line < end_line && x_start == x_end;
          let visual_x_end = if is_selecting_newline {
            x_end + px(NEWLINE_SELECTION_WIDTH)
          } else {
            x_end
          };

          selection_quads.push(fill(
            Bounds::from_corners(
              point(bounds.left() + x_start, y),
              point(bounds.left() + visual_x_end, y + line_height),
            ),
            theme.selection(),
          ));
        }
      }
    } else if !selected_range.is_empty() {
      let sel_start = selected_range.start;
      let sel_end = selected_range.end;
      let sel_start_line = document.char_to_line(sel_start);
      let sel_end_line = document.char_to_line(sel_end);

      for doc_line in sel_start_line..=sel_end_line {
        let display_line = self.editor.read(cx).doc_to_display_line(doc_line);
        let Some(display_line) = display_line else {
          continue;
        };
        if !viewport.contains(&display_line) {
          continue;
        }
        let line_range = document.line_range(doc_line).unwrap();
        let shaped_opt = shaped_lines
          .iter()
          .find(|(idx, _)| *idx == display_line)
          .map(|(_, shaped)| shaped);

        if let Some(shaped) = shaped_opt {
          let line_start = line_range.start;
          let line_end = line_range.end;
          let sel_line_start = sel_start.max(line_start) - line_start;
          let sel_line_end = sel_end.min(line_end) - line_start;
          let x_start = shaped.x_for_index(sel_line_start);
          let x_end = shaped.x_for_index(sel_line_end);
          let y = bounds.top() + line_height * (display_line - viewport.start) as f32;

          // If selection is empty on this line (selecting just the newline),
          // Only add width if we're actually selecting the newline character
          let is_selecting_newline = sel_line_end > sel_line_start && x_start == x_end;
          let visual_x_end = if is_selecting_newline {
            x_end + px(NEWLINE_SELECTION_WIDTH) // Small width to show newline selection
          } else {
            x_end
          };

          selection_quads.push(fill(
            Bounds::from_corners(
              point(bounds.left() + x_start, y),
              point(bounds.left() + visual_x_end, y + line_height),
            ),
            theme.selection(),
          ));
        }
      }
    }

    let cursor_quad = if is_primary { cursor_quad } else { None };
    let selection_quads = if is_primary {
      selection_quads
    } else {
      Vec::new()
    };

    PrepaintState {
      shaped_lines,
      line_backgrounds,
      group_borders,
      diag_paths,
      cursor_quad,
      selection_quads,
      viewport,
      bounds,
      line_height,
      scroll_hitbox,
      projection,
    }
  }

  fn paint(
    &mut self,
    _id: Option<&GlobalElementId>,
    _inspector_id: Option<&InspectorElementId>,
    bounds: Bounds<Pixels>,
    _request_layout: &mut Self::RequestLayoutState,
    prepaint: &mut Self::PrepaintState,
    window: &mut Window,
    cx: &mut App,
  ) {
    let is_primary = self.is_primary();
    let (focus_handle, is_focused) = {
      let editor = self.editor.read(cx);
      (
        editor.focus_handle.clone(),
        editor.focus_handle.is_focused(window),
      )
    };

    if is_primary {
      window.handle_input(
        &focus_handle,
        ElementInputHandler::new(bounds, self.editor.clone()),
        cx,
      );

      // Use Rc to avoid cloning PositionMap in closures
      let scroll_offset = self.editor.read(cx).scroll_offset_y;
      let position_map = Rc::new(PositionMap {
        shaped_lines: prepaint.shaped_lines.clone(),
        bounds: prepaint.bounds,
        line_height: prepaint.line_height,
        viewport: prepaint.viewport.clone(),
        scroll_offset,
        projection: prepaint.projection.clone(),
      });

      window.on_mouse_event({
        let editor = self.editor.clone();
        let position_map = Rc::clone(&position_map);
        move |event: &MouseDownEvent, phase, window, cx| {
          if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
            editor.update(cx, |editor, cx| {
              editor.mouse_left_down(event, &position_map, window, cx);
            });
          }
        }
      });

      window.on_mouse_event({
        let editor = self.editor.clone();
        move |event: &MouseUpEvent, phase, window, cx| {
          if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
            editor.update(cx, |editor, cx| {
              editor.mouse_left_up(event, window, cx);
            });
          }
        }
      });

      window.on_mouse_event({
        let editor = self.editor.clone();
        let position_map = Rc::clone(&position_map);
        move |event: &MouseMoveEvent, phase, window, cx| {
          if phase == DispatchPhase::Bubble {
            let is_selecting = editor.read(cx).is_selecting;
            if is_selecting {
              editor.update(cx, |editor, cx| {
                editor.mouse_dragged(event, &position_map, window, cx);
              });
            } else {
              editor.update(cx, |editor, cx| {
                editor.mouse_moved(event, &position_map, cx);
              });
            }
          }
        }
      });
    }

    // Handle mouse wheel scroll
    window.on_mouse_event({
      let editor = self.editor.clone();
      let scroll_hitbox = prepaint.scroll_hitbox.clone();
      move |event: &ScrollWheelEvent, phase, window, cx| {
        if phase == DispatchPhase::Bubble && scroll_hitbox.should_handle_scroll(window) {
          editor.update(cx, |editor, cx| {
            let document = editor.document().read(cx);
            let doc_line_count = document.len_lines();
            let total_lines = editor.display_line_count(doc_line_count);
            let now = Instant::now();
            let reset_lock = editor
              .last_scroll_time
              .map(|last| now.duration_since(last) > Duration::from_millis(SCROLL_AXIS_TIMEOUT_MS))
              .unwrap_or(true);
            if reset_lock {
              editor.scroll_axis_lock = None;
              editor.last_scroll_x = editor.scroll_handle.offset().x;
            }
            editor.last_scroll_time = Some(now);

            // Extract scroll delta (handle both pixel and line scrolling)
            // Note: Negative delta because scrolling down should increase scroll_offset
            let pixel_delta = event.delta.pixel_delta(window.line_height());
            let delta_x_px = pixel_delta.x;
            let delta_y_px = -pixel_delta.y;
            let delta_y = match event.delta {
              ScrollDelta::Pixels(point) => -(point.y / px(PIXEL_SCROLL_DIVISOR)),
              ScrollDelta::Lines(point) => -(point.y * LINE_SCROLL_MULTIPLIER),
            };
            let abs_x = delta_x_px.abs();
            let abs_y = delta_y_px.abs();
            let axis = match editor.scroll_axis_lock {
              None => {
                let axis = if abs_x > abs_y * SCROLL_AXIS_RATIO {
                  ScrollAxis::Horizontal
                } else {
                  ScrollAxis::Vertical
                };
                if axis == ScrollAxis::Horizontal {
                  editor.last_scroll_x = editor.scroll_handle.offset().x;
                }
                editor.scroll_axis_lock = Some(axis);
                axis
              }
              Some(axis) => {
                if axis == ScrollAxis::Vertical && abs_x > abs_y * SCROLL_AXIS_SWITCH_RATIO {
                  editor.last_scroll_x = editor.scroll_handle.offset().x;
                  editor.scroll_axis_lock = Some(ScrollAxis::Horizontal);
                  ScrollAxis::Horizontal
                } else if axis == ScrollAxis::Horizontal && abs_y > abs_x * SCROLL_AXIS_SWITCH_RATIO
                {
                  editor.scroll_axis_lock = Some(ScrollAxis::Vertical);
                  ScrollAxis::Vertical
                } else {
                  axis
                }
              }
            };

            if axis == ScrollAxis::Horizontal {
              let new_scroll_x = editor.scroll_handle.offset().x + delta_x_px;
              editor
                .scroll_handle
                .set_offset(point(new_scroll_x, px(0.0)));
              editor.last_scroll_x = new_scroll_x;
              cx.notify();
              return;
            }

            let new_scroll = (editor.scroll_offset_y + delta_y)
              .max(0.0)
              .min((total_lines.saturating_sub(1)) as f32);

            editor.scroll_offset_y = new_scroll;
            if editor.scroll_handle.offset().x != editor.last_scroll_x {
              editor
                .scroll_handle
                .set_offset(point(editor.last_scroll_x, px(0.0)));
            }
            let viewport = editor.viewport_range(window.line_height(), total_lines);
            let doc_viewport = editor.doc_range_for_display_viewport(viewport.clone());
            editor.document.update(cx, |doc, cx| {
              doc.schedule_viewport_highlights(
                doc_viewport.clone(),
                None,
                crate::document::VIEWPORT_HIGHLIGHT_MARGIN_LINES,
                cx,
              );
            });
            cx.notify();
          });
          cx.stop_propagation();
        }
      }
    });

    // Paint line backgrounds (diff highlights)
    for quad in &prepaint.line_backgrounds {
      window.paint_quad(quad.clone());
    }

    if !prepaint.diag_paths.is_empty() {
      let stripe_color = cx.theme().muted_foreground.opacity(0.35);
      let mask = ContentMask { bounds };
      window.with_content_mask(Some(mask), |window| {
        for path in &prepaint.diag_paths {
          window.paint_path(path.clone(), stripe_color);
        }
      });
    }

    // Paint staged group borders
    for quad in &prepaint.group_borders {
      window.paint_quad(quad.clone());
    }

    // Paint selection
    for quad in &prepaint.selection_quads {
      window.paint_quad(quad.clone());
    }

    // Paint text lines
    for (line_idx, shaped_line) in &prepaint.shaped_lines {
      let y = bounds.top() + prepaint.line_height * (*line_idx - prepaint.viewport.start) as f32;
      shaped_line
        .paint(
          point(bounds.left(), y),
          prepaint.line_height,
          TextAlign::Left,
          None,
          window,
          cx,
        )
        .ok();
    }

    // Paint cursor (if focused and visible from blink)
    let cursor_visible = self.editor.read(cx).cursor_blink.read(cx).visible();
    if is_primary
      && is_focused
      && cursor_visible
      && let Some(cursor_quad) = &prepaint.cursor_quad
    {
      window.paint_quad(cursor_quad.clone());
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use gpui::{TestAppContext, px, size};

  // Helper to create test bounds
  fn test_bounds(width: f32, height: f32) -> Bounds<Pixels> {
    Bounds::new(Point::default(), size(px(width), px(height)))
  }

  // ============================================================================
  // Viewport Calculation Tests
  // ============================================================================

  #[gpui::test]
  fn test_calculate_viewport_simple(cx: &mut TestAppContext) {
    let editor = cx.new(crate::editor::Editor::new);
    let element = EditorElement::new(editor);

    // 400px height, 20px line height = 20 visible lines
    let bounds = test_bounds(800.0, 400.0);
    let line_height = px(20.0);
    let scroll_offset = 0.0;
    let total_lines = 100;

    let viewport = element.calculate_viewport(bounds, line_height, scroll_offset, total_lines);

    assert_eq!(viewport, 0..20);
  }

  #[gpui::test]
  fn test_calculate_viewport_with_scroll(cx: &mut TestAppContext) {
    let editor = cx.new(crate::editor::Editor::new);
    let element = EditorElement::new(editor);

    let bounds = test_bounds(800.0, 400.0);
    let line_height = px(20.0);
    let scroll_offset = 10.0; // Scrolled down 10 lines
    let total_lines = 100;

    let viewport = element.calculate_viewport(bounds, line_height, scroll_offset, total_lines);

    assert_eq!(viewport, 10..30);
  }

  #[gpui::test]
  fn test_calculate_viewport_at_end(cx: &mut TestAppContext) {
    let editor = cx.new(crate::editor::Editor::new);
    let element = EditorElement::new(editor);

    let bounds = test_bounds(800.0, 400.0);
    let line_height = px(20.0);
    let scroll_offset = 90.0; // Near end
    let total_lines = 100;

    let viewport = element.calculate_viewport(bounds, line_height, scroll_offset, total_lines);

    // Should clamp to total_lines
    assert_eq!(viewport, 90..100);
  }

  #[gpui::test]
  fn test_calculate_viewport_short_document(cx: &mut TestAppContext) {
    let editor = cx.new(crate::editor::Editor::new);
    let element = EditorElement::new(editor);

    let bounds = test_bounds(800.0, 400.0);
    let line_height = px(20.0);
    let scroll_offset = 0.0;
    let total_lines = 5; // Document shorter than viewport

    let viewport = element.calculate_viewport(bounds, line_height, scroll_offset, total_lines);

    assert_eq!(viewport, 0..5);
  }

  #[gpui::test]
  fn test_calculate_viewport_fractional_scroll(cx: &mut TestAppContext) {
    let editor = cx.new(crate::editor::Editor::new);
    let element = EditorElement::new(editor);

    let bounds = test_bounds(800.0, 400.0);
    let line_height = px(20.0);
    let scroll_offset = 5.5; // Fractional scroll
    let total_lines = 100;

    let viewport = element.calculate_viewport(bounds, line_height, scroll_offset, total_lines);

    // Should floor scroll_offset
    assert_eq!(viewport, 5..25);
  }

  #[gpui::test]
  fn test_calculate_viewport_scroll_past_end(cx: &mut TestAppContext) {
    let editor = cx.new(crate::editor::Editor::new);
    let element = EditorElement::new(editor);

    let bounds = test_bounds(800.0, 400.0);
    let line_height = px(20.0);
    let scroll_offset = 150.0; // Way past end
    let total_lines = 100;

    let viewport = element.calculate_viewport(bounds, line_height, scroll_offset, total_lines);

    // Should clamp start_line to total_lines - 1
    assert_eq!(viewport, 99..100);
  }

  #[gpui::test]
  fn test_calculate_viewport_minimum_one_line(cx: &mut TestAppContext) {
    let editor = cx.new(crate::editor::Editor::new);
    let element = EditorElement::new(editor);

    let bounds = test_bounds(800.0, 10.0); // Very small height
    let line_height = px(20.0);
    let scroll_offset = 0.0;
    let total_lines = 100;

    let viewport = element.calculate_viewport(bounds, line_height, scroll_offset, total_lines);

    // Should show at least 1 line even if height is too small
    assert_eq!(viewport, 0..1);
  }

  #[gpui::test]
  fn test_calculate_viewport_large_line_height(cx: &mut TestAppContext) {
    let editor = cx.new(crate::editor::Editor::new);
    let element = EditorElement::new(editor);

    let bounds = test_bounds(800.0, 400.0);
    let line_height = px(40.0); // Large line height
    let scroll_offset = 0.0;
    let total_lines = 100;

    let viewport = element.calculate_viewport(bounds, line_height, scroll_offset, total_lines);

    // 400 / 40 = 10 visible lines
    assert_eq!(viewport, 0..10);
  }

  #[gpui::test]
  fn test_calculate_viewport_single_line_document(cx: &mut TestAppContext) {
    let editor = cx.new(crate::editor::Editor::new);
    let element = EditorElement::new(editor);

    let bounds = test_bounds(800.0, 400.0);
    let line_height = px(20.0);
    let scroll_offset = 0.0;
    let total_lines = 1;

    let viewport = element.calculate_viewport(bounds, line_height, scroll_offset, total_lines);

    assert_eq!(viewport, 0..1);
  }

  #[gpui::test]
  fn test_calculate_viewport_empty_document(cx: &mut TestAppContext) {
    let editor = cx.new(crate::editor::Editor::new);
    let element = EditorElement::new(editor);

    let bounds = test_bounds(800.0, 400.0);
    let line_height = px(20.0);
    let scroll_offset = 0.0;
    let total_lines = 0;

    let viewport = element.calculate_viewport(bounds, line_height, scroll_offset, total_lines);

    // Empty document edge case - start_line gets clamped to 0
    // This creates a 0..0 range which is valid but empty
    assert!(viewport.is_empty());
  }
}
