use std::{collections::HashMap, ops::Range, sync::Arc};

use gpui::{Bounds, Context, PaintQuad, Pixels, ShapedLine, TextRun, TextStyle, Window, point, px};
use gpui_component::ActiveTheme as _;
use unicode_segmentation::UnicodeSegmentation;

use crate::{
  editor::{DisplayCursor, Editor},
  editor_element::{DiffElementView, display_line_text_for_view},
  projection::DisplayLine,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct WrapBoundary {
  pub byte: usize,
  pub column: usize,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct WrapMap {
  starts: Vec<usize>,
  lines: HashMap<usize, [Arc<Vec<WrapBoundary>>; 2]>,
}

impl WrapMap {
  pub fn row(&self, line: usize) -> usize {
    self.starts.get(line).copied().unwrap_or(line)
  }

  pub fn line(&self, row: usize) -> usize {
    if self.starts.is_empty() {
      row
    } else {
      self
        .starts
        .partition_point(|start| *start <= row)
        .saturating_sub(1)
        .min(self.starts.len().saturating_sub(2))
    }
  }

  pub fn count(&self, logical_count: usize) -> usize {
    self.starts.last().copied().unwrap_or(logical_count)
  }

  pub fn logical_range(&self, rows: Range<usize>) -> Range<usize> {
    if rows.is_empty() {
      return 0..0;
    }
    self.line(rows.start)..self.line(rows.end - 1) + 1
  }

  pub fn boundaries(&self, line: usize, view: DiffElementView) -> &[WrapBoundary] {
    self
      .lines
      .get(&line)
      .map(|sides| sides[usize::from(view != DiffElementView::SplitLeft)].as_slice())
      .unwrap_or(&[WrapBoundary { byte: 0, column: 0 }])
  }

  pub fn cursor_row(&self, line: usize, column: usize, view: DiffElementView) -> usize {
    self.row(line)
      + self
        .boundaries(line, view)
        .partition_point(|boundary| boundary.column <= column)
        .saturating_sub(1)
  }

  pub fn cursor_row_with_affinity(
    &self,
    cursor: DisplayCursor,
    view: DiffElementView,
    upstream: bool,
  ) -> usize {
    let row = self.cursor_row(cursor.line, cursor.column, view);
    if upstream && row > self.row(cursor.line) && self.boundary(row, view).column == cursor.column {
      row - 1
    } else {
      row
    }
  }

  pub fn boundary(&self, row: usize, view: DiffElementView) -> WrapBoundary {
    let line = self.line(row);
    self
      .boundaries(line, view)
      .get(row.saturating_sub(self.row(line)))
      .copied()
      .or_else(|| self.boundaries(line, view).last().copied())
      .unwrap_or_default()
  }

  pub fn fragment_bytes(
    &self,
    line: usize,
    fragment: usize,
    view: DiffElementView,
    length: usize,
  ) -> Option<Range<usize>> {
    let boundaries = self.boundaries(line, view);
    let start = boundaries.get(fragment)?.byte;
    Some(
      start
        ..boundaries
          .get(fragment + 1)
          .map_or(length, |boundary| boundary.byte),
    )
  }

  pub fn block_quad(
    &self,
    mut quad: PaintQuad,
    top: Pixels,
    height: Pixels,
    scroll: f32,
  ) -> PaintQuad {
    let start = scroll + (quad.bounds.top() - top) / height;
    let end = scroll + (quad.bounds.bottom() - top) / height;
    let map = |position: f32| {
      let line = (position + 0.0001).floor().max(0.0) as usize;
      let fraction = (position - line as f32).max(0.0);
      self.row(line) as f32
        + fraction * self.row(line + 1).saturating_sub(self.row(line)).max(1) as f32
    };
    let new_start = map(start);
    let new_end = map(end);
    if quad.bounds.size.height <= px(1.0) {
      let at_end = start.fract() > 0.5;
      quad.bounds.origin.y = top + height * ((if at_end { new_end } else { new_start }) - scroll)
        - if at_end {
          quad.bounds.size.height
        } else {
          px(0.0)
        };
    } else {
      quad.bounds.origin.y = top + height * (new_start - scroll);
      quad.bounds.size.height = height * (new_end - new_start);
    }
    quad
  }

  pub fn visible_fragments(
    &self,
    line: usize,
    view: DiffElementView,
    scroll: f32,
    height: Pixels,
    viewport_height: Pixels,
  ) -> Range<usize> {
    let start = (scroll.floor().max(0.0) as usize).saturating_sub(self.row(line));
    let end =
      ((scroll + viewport_height / height).ceil().max(0.0) as usize).saturating_sub(self.row(line));
    let count = self.boundaries(line, view).len();
    start.min(count)..end.min(count)
  }

  pub fn text_quads(
    &self,
    quad: &PaintQuad,
    line: usize,
    shaped: &ShapedLine,
    bounds: Bounds<Pixels>,
    height: Pixels,
    scroll: f32,
    view: DiffElementView,
    caret: bool,
    upstream: bool,
  ) -> Vec<PaintQuad> {
    let start = quad.bounds.left() - bounds.left();
    let end = quad.bounds.right() - bounds.left();
    let boundaries = self.boundaries(line, view);
    let mut result = Vec::new();
    for fragment in self.visible_fragments(line, view, scroll, height, bounds.size.height) {
      let Some(boundary) = boundaries.get(fragment) else {
        continue;
      };
      let left = shaped.x_for_index(boundary.byte);
      let right = boundaries
        .get(fragment + 1)
        .map(|boundary| shaped.x_for_index(boundary.byte));
      if caret
        && if upstream {
          start < left
            || (fragment > 0 && start == left)
            || right.is_some_and(|right| start > right)
        } else {
          start < left || right.is_some_and(|right| start >= right)
        }
      {
        continue;
      }
      let clipped_start = start.max(left);
      let clipped_end = right.map_or(end, |right| end.min(right));
      if !caret && clipped_start >= clipped_end {
        continue;
      }
      let mut mapped = quad.clone();
      mapped.bounds.origin = point(
        bounds.left() + clipped_start - left,
        bounds.top() + height * ((self.row(line) + fragment) as f32 - scroll),
      );
      mapped.bounds.size.width = if caret {
        quad.bounds.size.width
      } else {
        clipped_end - clipped_start
      };
      result.push(mapped);
    }
    result
  }
}

#[derive(PartialEq)]
struct WrapKey {
  version: buffer::BufferVersion,
  projection: usize,
  width: Pixels,
  font: gpui::Font,
  font_size: Pixels,
  indentation: usize,
}

#[derive(Default)]
pub(crate) struct SoftWrap {
  pub enabled: bool,
  pub map: Arc<WrapMap>,
  pub upstream_cursor: Option<DisplayCursor>,
  pub upstream_selections: HashMap<u64, DisplayCursor>,
  key: Option<WrapKey>,
  projection: Option<Arc<crate::projection::Projection>>,
  cache: HashMap<blake3::Hash, Arc<Vec<WrapBoundary>>>,
  changed_lines: Option<(buffer::BufferVersion, Range<usize>)>,
}

impl SoftWrap {
  pub fn cursor_row(&self, cursor: DisplayCursor, view: DiffElementView) -> usize {
    self
      .map
      .cursor_row_with_affinity(cursor, view, self.upstream_cursor == Some(cursor))
  }

  pub(super) fn record_edit(
    &mut self,
    before: buffer::BufferVersion,
    after: buffer::BufferVersion,
    lines: Range<usize>,
  ) {
    if let Some((version, changed)) = &mut self.changed_lines
      && *version == before
    {
      *version = after;
      changed.start = changed.start.min(lines.start);
      changed.end = changed.end.max(lines.end);
    } else {
      self.changed_lines = self
        .key
        .as_ref()
        .filter(|key| key.version == before)
        .map(|_| (after, lines));
    }
  }
}

fn wrap_line(
  text: &str,
  width: Pixels,
  tab_width: Pixels,
  measure: impl FnMut(&str) -> Pixels,
) -> Vec<WrapBoundary> {
  if text.is_ascii() {
    wrap_graphemes(
      text
        .char_indices()
        .map(|(byte, _)| (byte, text.get(byte..byte + 1).unwrap_or_default())),
      width,
      tab_width,
      measure,
    )
  } else {
    wrap_graphemes(text.grapheme_indices(true), width, tab_width, measure)
  }
}

fn wrap_graphemes<'a>(
  graphemes: impl Iterator<Item = (usize, &'a str)>,
  width: Pixels,
  tab_width: Pixels,
  mut measure: impl FnMut(&str) -> Pixels,
) -> Vec<WrapBoundary> {
  let mut boundaries = vec![WrapBoundary::default()];
  let mut absolute_x = px(0.0);
  let mut row_x = px(0.0);
  let mut column = 0;
  let mut candidate = None;
  let mut previous_whitespace = false;
  for (byte, grapheme) in graphemes {
    let whitespace = grapheme.chars().all(char::is_whitespace);
    if previous_whitespace && !whitespace {
      candidate = Some((WrapBoundary { byte, column }, absolute_x));
    }
    let advance = if grapheme == "\t" {
      tab_width * ((absolute_x / tab_width).floor() + 1.0) - absolute_x
    } else {
      measure(grapheme)
    };
    if absolute_x + advance - row_x > width
      && boundaries.last().is_some_and(|last| byte > last.byte)
    {
      let (boundary, next_x) = candidate
        .take()
        .filter(|(boundary, _)| {
          boundaries
            .last()
            .is_some_and(|last| boundary.byte > last.byte)
        })
        .unwrap_or((WrapBoundary { byte, column }, absolute_x));
      boundaries.push(boundary);
      row_x = next_x;
    }
    absolute_x += advance;
    column += grapheme.chars().count();
    previous_whitespace = whitespace;
  }
  boundaries
}

impl Editor {
  pub(crate) fn set_wrap_affinity(
    &mut self,
    upstream: Option<DisplayCursor>,
    cx: &mut Context<Self>,
  ) {
    self
      .soft_wrap
      .upstream_selections
      .remove(&self.selections.primary().id);
    if self.soft_wrap.upstream_cursor != upstream {
      self.soft_wrap.upstream_cursor = upstream;
      cx.notify();
    }
  }

  pub fn soft_wrap_enabled(&self) -> bool {
    self.soft_wrap.enabled
  }

  pub fn visual_row_for_display_line(&self, display_line: usize) -> usize {
    self.soft_wrap.map.row(display_line)
  }

  pub fn visual_line_count(&self, doc_line_count: usize) -> usize {
    self
      .soft_wrap
      .map
      .count(self.display_line_count(doc_line_count))
  }

  pub fn toggle_soft_wrap(&mut self, window: &Window, cx: &mut Context<Self>) {
    self.set_soft_wrap(!self.soft_wrap.enabled, cx);
    self.sync_soft_wrap(window, cx);
    self.ensure_cursor_visible(window, cx);
  }

  pub fn set_soft_wrap(&mut self, enabled: bool, cx: &mut Context<Self>) {
    if self.soft_wrap.enabled == enabled {
      return;
    }
    let line = self
      .soft_wrap
      .map
      .line(self.scroll_offset_y.floor().max(0.0) as usize);
    self.soft_wrap = SoftWrap {
      enabled,
      ..SoftWrap::default()
    };
    self.scroll_offset_y = line as f32;
    self.find_scroll_epoch = self.find_scroll_epoch.saturating_add(1);
    self.review_comment_scroll_epoch = self.review_comment_scroll_epoch.saturating_add(1);
    self.vertical_goal_x = None;
    self.selection_position_map = None;
    self.reset_horizontal_scroll_state();
    cx.notify();
  }

  pub(crate) fn sync_soft_wrap(&mut self, window: &Window, cx: &mut Context<Self>) {
    if !self.soft_wrap.enabled || self.viewport_width <= px(0.0) {
      return;
    }
    let document = self.document.read(cx);
    let style = TextStyle {
      font_family: cx.theme().mono_font_family.clone(),
      font_size: cx.theme().mono_font_size.into(),
      ..TextStyle::default()
    };
    let width = (self.viewport_width.min(self.horizontal_viewport_width()) - px(4.0)).max(px(1.0));
    let font_size = style.font_size.to_pixels(window.rem_size());
    let font = style.font();
    let key = WrapKey {
      version: document.buffer.version(),
      projection: self
        .projection
        .as_ref()
        .map_or(0, |projection| Arc::as_ptr(projection) as usize),
      width,
      font: font.clone(),
      font_size,
      indentation: document.indentation.width,
    };
    if self.soft_wrap.key.as_ref() == Some(&key) {
      return;
    }
    if self.soft_wrap.key.as_ref().is_none_or(|previous| {
      previous.width != width
        || previous.font != font
        || previous.font_size != font_size
        || previous.indentation != document.indentation.width
    }) {
      self.soft_wrap.cache.clear();
    }
    let reveal_cursor = self
      .soft_wrap
      .key
      .as_ref()
      .is_some_and(|previous| previous.version != key.version)
      && self.focus_handle.is_focused(window);
    let anchor_row = self.scroll_offset_y.floor().max(0.0) as usize;
    let anchor_line = self.soft_wrap.map.line(anchor_row);
    let anchor_column = self
      .soft_wrap
      .map
      .boundary(anchor_row, self.selection_view)
      .column;
    let mut widths = HashMap::new();
    let mut ascii_widths = [None; 128];
    let mut measure = |text: &str| -> Pixels {
      let ascii = text
        .as_bytes()
        .first()
        .copied()
        .filter(|byte| text.len() == 1 && byte.is_ascii());
      if let Some(width) = ascii
        .and_then(|byte| ascii_widths.get(usize::from(byte)))
        .copied()
        .flatten()
      {
        return width;
      }
      if let Some(width) = widths.get(text) {
        return *width;
      }
      let width = window
        .text_system()
        .shape_line(
          text.to_owned().into(),
          font_size,
          &[TextRun {
            len: text.len(),
            font: font.clone(),
            color: style.color,
            background_color: None,
            underline: None,
            strikethrough: None,
          }],
          None,
        )
        .width;
      if let Some(slot) = ascii.and_then(|byte| ascii_widths.get_mut(usize::from(byte))) {
        *slot = Some(width);
      } else {
        widths.insert(text.to_owned(), width);
      }
      width
    };
    let tab_width = measure(" ").max(px(1.0)) * document.indentation.width.max(1) as f32;
    let count = self.display_line_count(document.len_lines());
    let changed_lines = self
      .soft_wrap
      .changed_lines
      .take()
      .filter(|(version, _)| *version == key.version)
      .filter(|_| {
        self.soft_wrap.key.as_ref().is_some_and(|previous| {
          previous.projection == key.projection
            && previous.width == key.width
            && previous.font == key.font
            && previous.font_size == key.font_size
            && previous.indentation == key.indentation
        })
      })
      .filter(|_| self.soft_wrap.map.starts.len() == count + 1)
      .map(|(_, lines)| lines);
    let incremental = changed_lines.is_some();
    let lines = changed_lines
      .map(|lines| {
        let first = self.doc_to_display_line(lines.start).unwrap_or(0);
        let last = self
          .doc_to_display_line(lines.end.saturating_sub(1))
          .map_or(count, |line| line + 1);
        first..last
      })
      .unwrap_or(0..count);
    let mut map = if incremental {
      self.soft_wrap.map.as_ref().clone()
    } else {
      WrapMap::default()
    };
    let mut cache = if incremental {
      std::mem::take(&mut self.soft_wrap.cache)
    } else {
      HashMap::new()
    };
    if cache.len() > count.saturating_mul(2).max(128) {
      cache.clear();
    }
    for line in lines {
      map.lines.remove(&line);
      let Some(display) = self.display_line(line, document.len_lines()) else {
        continue;
      };
      if !matches!(
        display,
        DisplayLine::Doc { .. } | DisplayLine::Modified { .. } | DisplayLine::Removed { .. }
      ) {
        continue;
      }
      let [left, right] = [DiffElementView::SplitLeft, DiffElementView::SplitRight].map(|view| {
        let text = display_line_text_for_view(&display, view, document);
        let hash = blake3::hash(text.as_bytes());
        cache
          .entry(hash)
          .or_insert_with(|| {
            self
              .soft_wrap
              .cache
              .get(&hash)
              .cloned()
              .unwrap_or_else(|| Arc::new(wrap_line(&text, width, tab_width, &mut measure)))
          })
          .clone()
      });
      if left.len() > 1 || right.len() > 1 {
        map.lines.insert(line, [left, right]);
      }
    }
    map.starts.clear();
    map.starts.reserve(count + 1);
    let mut row = 0;
    for line in 0..count {
      map.starts.push(row);
      row += map
        .lines
        .get(&line)
        .map_or(1, |[left, right]| left.len().max(right.len()));
    }
    map.starts.push(row);
    self.scroll_offset_y = map.cursor_row(anchor_line, anchor_column, self.selection_view) as f32
      + self.scroll_offset_y.fract();
    self.soft_wrap.map = Arc::new(map);
    self.soft_wrap.upstream_cursor = None;
    self.soft_wrap.upstream_selections.clear();
    self.soft_wrap.cache = cache;
    self.soft_wrap.key = Some(key);
    self.soft_wrap.projection = self.projection.clone();
    self.find_scroll_epoch = self.find_scroll_epoch.saturating_add(1);
    self.review_comment_scroll_epoch = self.review_comment_scroll_epoch.saturating_add(1);
    if reveal_cursor {
      self.ensure_cursor_visible_when_hidden(cx);
    }
  }
}

#[cfg(test)]
#[path = "soft_wrap_tests.rs"]
mod integration_tests;

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn wraps_words_tabs_and_graphemes_without_changing_offsets() {
    let text = "ab cd\te\u{301}👩‍💻z";
    let boundaries = wrap_line(text, px(4.0), px(4.0), |_| px(1.0));
    assert_eq!(
      boundaries
        .iter()
        .map(|boundary| boundary.column)
        .collect::<Vec<_>>(),
      vec![0, 3, 5, 6]
    );
    let reconstructed = boundaries
      .iter()
      .enumerate()
      .map(|(index, boundary)| {
        &text[boundary.byte
          ..boundaries
            .get(index + 1)
            .map_or(text.len(), |next| next.byte)]
      })
      .collect::<String>();
    assert_eq!(reconstructed, text);
    assert!(boundaries.iter().all(|boundary| {
      text
        .grapheme_indices(true)
        .any(|(byte, _)| byte == boundary.byte)
    }));
  }

  #[test]
  fn narrow_width_always_makes_progress() {
    let boundaries = wrap_line("a👩‍💻b", px(0.1), px(4.0), |_| px(10.0));
    assert_eq!(
      boundaries
        .iter()
        .map(|boundary| boundary.column)
        .collect::<Vec<_>>(),
      vec![0, 1, 4]
    );
    assert_eq!(wrap_line("", px(1.0), px(4.0), |_| px(1.0)).len(), 1);
  }
}
