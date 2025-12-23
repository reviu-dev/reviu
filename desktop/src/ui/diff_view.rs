//! Custom high-performance diff view with virtualization
//! Uses GPUI's uniform_list for efficient rendering of large files
//! Displays changed lines with CONTEXT_OFFSET lines around them
//! Supports both local git (V1) and GitHub PR diffs (V2)

use crate::state::{FileDiff, LineOrigin};
use gpui::{
  div, prelude::*, px, uniform_list, AnyElement, Context, IntoElement, ParentElement, Render,
  Styled, UniformListScrollHandle, Window,
};
use std::sync::Arc;

use crate::ui::Colors;

const CONTEXT_OFFSET: usize = 3; // Show 3 lines of context before and after changes

/// Represents a display line - either a real line or an expansion button
#[derive(Debug, Clone)]
enum DisplayLine {
  /// A real line from the file
  Line {
    line_idx: usize,
    line_num: usize,
    content: String,
    origin: Option<LineOrigin>,
  },
  /// An expansion button
  ExpandButton {
    hidden_lines: usize,
    region_id: usize,
    label: String,
  },
}

/// Custom diff view with perfect context handling and virtualization
/// Works with both local git files (V1) and GitHub PR files (V2)
pub struct DiffView {
  /// File metadata
  file_diff: Arc<FileDiff>,
  /// All lines from the new file content
  all_lines: Vec<String>,
  /// Line change information (line_num -> LineOrigin)
  /// Index corresponds to 0-based line index
  line_changes: Vec<Option<LineOrigin>>,
  /// Expanded context regions
  expanded_regions: Vec<usize>,
  /// Scroll handle for virtualization
  scroll_handle: UniformListScrollHandle,
  /// Flattened display lines (cached for rendering)
  display_lines: Vec<DisplayLine>,
}

impl DiffView {
  pub fn new(file_diff: Arc<FileDiff>, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
    // Load content - works for both V1 (local) and V2 (GitHub)
    let content = file_diff.new_content.as_deref().unwrap_or("");
    let all_lines: Vec<String> = if content.is_empty() {
      Vec::new()
    } else {
      content.lines().map(|s| s.to_string()).collect()
    };

    // Handle empty files (new files might have content but old files don't)
    if all_lines.is_empty() && file_diff.status.as_str() == "Added" {
      return Self::from_hunks_only(file_diff);
    }

    // Build line change map from hunks
    let mut line_changes = vec![None; all_lines.len()];

    for hunk in &file_diff.hunks {
      for line in &hunk.lines {
        if let Some(new_lineno) = line.new_lineno {
          let idx = (new_lineno as usize).saturating_sub(1);
          if idx < line_changes.len() {
            line_changes[idx] = Some(line.origin);
          }
        }
      }
    }

    let mut view = Self {
      file_diff,
      all_lines,
      line_changes,
      expanded_regions: Vec::new(),
      scroll_handle: UniformListScrollHandle::new(),
      display_lines: Vec::new(),
    };

    // Calculate initial display lines
    view.recalculate_display_lines();
    view
  }

  /// Fallback constructor when only hunks are available (for new files)
  fn from_hunks_only(file_diff: Arc<FileDiff>) -> Self {
    let mut all_lines = Vec::new();
    let mut line_changes = Vec::new();

    // Extract lines from hunks (additions only for new files)
    for hunk in &file_diff.hunks {
      for line in &hunk.lines {
        if matches!(line.origin, LineOrigin::Addition) {
          all_lines.push(line.content.clone());
          line_changes.push(Some(LineOrigin::Addition));
        }
      }
    }

    let mut view = Self {
      file_diff,
      all_lines,
      line_changes,
      expanded_regions: Vec::new(),
      scroll_handle: UniformListScrollHandle::new(),
      display_lines: Vec::new(),
    };

    view.recalculate_display_lines();
    view
  }

  /// Get the number of hunks
  pub fn hunk_count(&self) -> usize {
    self.file_diff.hunks.len()
  }

  /// Check if a region is expanded
  fn is_region_expanded(&self, region_id: usize) -> bool {
    self.expanded_regions.contains(&region_id)
  }

  /// Toggle region expansion
  fn toggle_region(&mut self, region_id: usize, cx: &mut Context<Self>) {
    if let Some(pos) = self.expanded_regions.iter().position(|&id| id == region_id) {
      self.expanded_regions.remove(pos);
    } else {
      self.expanded_regions.push(region_id);
    }
    self.recalculate_display_lines();
    cx.notify();
  }

  /// Find all ranges of changed lines
  fn find_change_ranges(&self) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut start: Option<usize> = None;

    for (idx, change) in self.line_changes.iter().enumerate() {
      match (change, start) {
        (Some(_), None) => {
          start = Some(idx);
        }
        (None, Some(s)) => {
          ranges.push((s, idx - 1));
          start = None;
        }
        _ => {}
      }
    }

    if let Some(s) = start {
      ranges.push((s, self.all_lines.len().saturating_sub(1)));
    }

    ranges
  }

  /// Recalculate flattened display lines
  fn recalculate_display_lines(&mut self) {
    self.display_lines.clear();
    let change_ranges = self.find_change_ranges();

    if change_ranges.is_empty() {
      return;
    }

    let mut region_id = 0;

    for (i, (change_start, change_end)) in change_ranges.iter().enumerate() {
      let is_first = i == 0;
      let is_last = i == change_ranges.len() - 1;

      let visible_start = change_start.saturating_sub(CONTEXT_OFFSET);

      // Handle gap before this change block
      if is_first {
        if visible_start > 0 {
          let expanded = self.is_region_expanded(region_id);
          if expanded {
            for line_idx in 0..visible_start {
              self.push_line(line_idx);
            }
          } else {
            self.display_lines.push(DisplayLine::ExpandButton {
              hidden_lines: visible_start,
              region_id,
              label: "Show more".to_string(),
            });
          }
          region_id += 1;
        }
      } else {
        let (_, prev_end) = change_ranges[i - 1];
        let prev_visible_end =
          (prev_end + CONTEXT_OFFSET).min(self.all_lines.len().saturating_sub(1));

        if visible_start > prev_visible_end + 1 {
          let gap_start = prev_visible_end + 1;
          let gap_end = visible_start - 1;

          let expanded = self.is_region_expanded(region_id);
          if expanded {
            for line_idx in gap_start..=gap_end {
              self.push_line(line_idx);
            }
          } else {
            self.display_lines.push(DisplayLine::ExpandButton {
              hidden_lines: gap_end - gap_start + 1,
              region_id,
              label: "Show more".to_string(),
            });
          }
          region_id += 1;
        }
      }

      // Show the change block with context
      let visible_end = (change_end + CONTEXT_OFFSET).min(self.all_lines.len().saturating_sub(1));
      for line_idx in visible_start..=visible_end {
        self.push_line(line_idx);
      }

      // Handle after last block
      if is_last && visible_end < self.all_lines.len().saturating_sub(1) {
        let expanded = self.is_region_expanded(region_id);

        if expanded {
          for line_idx in (visible_end + 1)..self.all_lines.len() {
            self.push_line(line_idx);
          }
        } else {
          let remaining_lines = self.all_lines.len().saturating_sub(1) - visible_end;
          self.display_lines.push(DisplayLine::ExpandButton {
            hidden_lines: remaining_lines,
            region_id,
            label: "Show more".to_string(),
          });
        }
      }
    }
  }

  /// Helper to push a line to display_lines
  fn push_line(&mut self, line_idx: usize) {
    let content = self
      .all_lines
      .get(line_idx)
      .map(|s| s.clone())
      .unwrap_or_default();
    let origin = self.line_changes.get(line_idx).copied().flatten();
    let line_num = line_idx + 1;

    self.display_lines.push(DisplayLine::Line {
      line_idx,
      line_num,
      content,
      origin,
    });
  }

  /// Render a single display line
  fn render_display_line(&self, display_line: &DisplayLine, cx: &mut Context<Self>) -> AnyElement {
    match display_line {
      DisplayLine::Line {
        line_num,
        content,
        origin,
        ..
      } => self
        .render_line(*line_num, content, *origin)
        .into_any_element(),
      DisplayLine::ExpandButton {
        hidden_lines,
        region_id,
        label,
      } => self
        .render_expand_button(*region_id, *hidden_lines, label, cx)
        .into_any_element(),
    }
  }

  /// Render a single line
  fn render_line(
    &self,
    line_num: usize,
    line_content: &str,
    line_origin: Option<LineOrigin>,
  ) -> impl IntoElement {
    let (bg_color, fg_color, prefix) = match line_origin {
      Some(LineOrigin::Addition) => (Colors::diff_addition_bg(), Colors::diff_addition_fg(), "+"),
      Some(LineOrigin::Deletion) => (Colors::diff_deletion_bg(), Colors::diff_deletion_fg(), "-"),
      _ => (Colors::diff_context_bg(), Colors::text_muted(), " "),
    };

    let line_numbers = match line_origin {
      Some(LineOrigin::Deletion) => format!("{:>4}    ", line_num),
      _ => format!("{:>4} {:>4}", line_num, line_num),
    };

    div()
      .flex()
      .items_start()
      .w_full()
      .bg(bg_color)
      .child(
        div()
          .w(px(80.0))
          .px(px(8.0))
          .py(px(2.0))
          .text_xs()
          .font_family("monospace")
          .text_color(Colors::text_muted())
          .child(line_numbers),
      )
      .child(
        div()
          .flex_1()
          .px(px(8.0))
          .py(px(2.0))
          .text_xs()
          .font_family("monospace")
          .text_color(fg_color)
          .child(format!("{} {}", prefix, line_content)),
      )
  }

  /// Render an expand button
  fn render_expand_button(
    &self,
    region_id: usize,
    hidden_lines: usize,
    label: &str,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    div()
      .id(format!("expand-{}", region_id))
      .flex()
      .items_center()
      .justify_center()
      .w_full()
      .h(px(28.0))
      .bg(Colors::bg_secondary())
      .border_y_1()
      .border_color(Colors::border_primary())
      .cursor_pointer()
      .hover(|this| this.bg(Colors::hover()))
      .on_mouse_down(
        gpui::MouseButton::Left,
        cx.listener(move |view, _event, _window, cx| {
          view.toggle_region(region_id, cx);
        }),
      )
      .child(
        div()
          .flex()
          .items_center()
          .gap_2()
          .child(div().text_xs().text_color(Colors::text_muted()).child("⋯"))
          .child(
            div()
              .text_xs()
              .text_color(Colors::text_secondary())
              .child(format!("{} ({} lines)", label, hidden_lines)),
          ),
      )
  }
}

impl Render for DiffView {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let file_name = self
      .file_diff
      .path
      .file_name()
      .and_then(|n| n.to_str())
      .unwrap_or("Unknown")
      .to_string();

    let status_text = self.file_diff.status.as_str().to_string();
    let hunk_count = self.file_diff.hunks.len();
    let total_display_lines = self.display_lines.len();

    // Clone data for the closure
    let display_lines = self.display_lines.clone();

    // Root container with fixed height
    div()
      .flex()
      .flex_col()
      .size_full()
      .overflow_hidden()
      .bg(Colors::bg_primary())
      // File header (fixed height)
      .child(
        div()
          .flex()
          .items_center()
          .justify_between()
          .px(px(16.0))
          .py(px(12.0))
          .bg(Colors::bg_secondary())
          .border_b_1()
          .border_color(Colors::border_primary())
          .child(
            div()
              .flex()
              .items_center()
              .gap_2()
              .child(
                div()
                  .text_sm()
                  .font_weight(gpui::FontWeight::BOLD)
                  .text_color(Colors::text_primary())
                  .child(file_name),
              )
              .child(
                div()
                  .text_xs()
                  .text_color(Colors::text_muted())
                  .child(format!(
                    "({}) • {} hunks • {} lines",
                    status_text,
                    hunk_count,
                    self.all_lines.len()
                  )),
              ),
          ),
      )
      // Virtualized scrollable content with uniform_list
      .child(
        uniform_list(
          "diff-lines",
          total_display_lines,
          cx.processor(move |this, range, _window, cx| {
            let mut items = Vec::new();
            for idx in range {
              if let Some(display_line) = display_lines.get(idx) {
                items.push(this.render_display_line(display_line, cx));
              }
            }
            items
          }),
        )
        .flex_1()
        .w_full()
        .track_scroll(&self.scroll_handle),
      )
  }
}
