//! Custom high-performance diff view with virtualization
//! Uses GPUI's uniform_list for efficient rendering of large files
//! Displays changed lines with CONTEXT_OFFSET lines around them
//! Supports both local git (V1) and GitHub PR diffs (V2)

use crate::state::{FileDiff, LineOrigin};
use crate::syntax_highlighter::{SupportedLanguage, SyntaxHighlighter};
use gpui::{
  div, prelude::*, px, relative, uniform_list, AnyElement, Context, IntoElement, ParentElement,
  Render, Styled, StyledText, UniformListScrollHandle, Window,
};
use std::sync::Arc;

use crate::ui::Colors;

/// Represents a display line or expand button
#[derive(Debug, Clone)]
enum DisplayLine {
  /// A real line from a hunk
  Line {
    line_idx: usize,
    old_lineno: Option<u32>,
    new_lineno: Option<u32>,
    content: String,
    origin: Option<LineOrigin>,
  },
  /// An expand button to show hidden lines between hunks
  ExpandButton {
    region_id: usize,
    hidden_lines: usize,
    start_old_lineno: u32,
    start_new_lineno: u32,
  },
}

/// Line info stored for rendering
#[derive(Debug, Clone)]
struct LineInfo {
  content: String,
  origin: LineOrigin,
  old_lineno: Option<u32>,
  new_lineno: Option<u32>,
}

/// Custom diff view with virtualization and expandable gaps
/// Works with both local git files (V1) and GitHub PR files (V2)
/// Displays hunks with expand buttons for gaps between them
pub struct DiffView {
  /// File metadata
  file_diff: Arc<FileDiff>,
  /// All lines from hunks (includes context, additions, deletions)
  all_lines: Vec<LineInfo>,
  /// Expanded regions (gap IDs that are currently expanded)
  expanded_regions: Vec<usize>,
  /// Scroll handle for virtualization
  scroll_handle: UniformListScrollHandle,
  /// Flattened display lines (cached for rendering)
  display_lines: Vec<DisplayLine>,
  /// Syntax highlighter for this file (if language is supported)
  syntax_highlighter: Option<SyntaxHighlighter>,
}

impl DiffView {
  pub fn new(file_diff: Arc<FileDiff>, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
    // Build display lines directly from hunks to include deletions
    // This preserves the original line order: context, deletions, additions
    let mut all_lines = Vec::new();

    for hunk in &file_diff.hunks {
      for line in &hunk.lines {
        all_lines.push(LineInfo {
          content: line.content.clone(),
          origin: line.origin,
          old_lineno: line.old_lineno,
          new_lineno: line.new_lineno,
        });
      }
    }

    // Try to create syntax highlighter based on file extension
    let syntax_highlighter = SupportedLanguage::from_path(&file_diff.path)
      .and_then(|lang| SyntaxHighlighter::new(lang).ok());

    let mut view = Self {
      file_diff,
      all_lines,
      expanded_regions: Vec::new(),
      scroll_handle: UniformListScrollHandle::new(),
      display_lines: Vec::new(),
      syntax_highlighter,
    };

    // Calculate initial display lines
    view.recalculate_display_lines();
    view
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

  /// Recalculate display lines by showing hunks and gaps between them
  fn recalculate_display_lines(&mut self) {
    self.display_lines.clear();

    if self.file_diff.hunks.is_empty() {
      return;
    }

    // Clone hunks to avoid borrow checker issues
    let hunks = self.file_diff.hunks.clone();
    let mut line_offset = 0; // Track position in all_lines
    let mut region_id = 0;

    for (hunk_idx, hunk) in hunks.iter().enumerate() {
      let is_first = hunk_idx == 0;
      let is_last = hunk_idx == hunks.len() - 1;

      // Handle gap before first hunk
      if is_first && hunk.new_start > 1 {
        let gap_lines = (hunk.new_start - 1) as usize;
        if self.is_region_expanded(region_id) {
          // Load and show lines from new_content
          self.load_gap_lines(1, hunk.new_start - 1, 1, hunk.old_start - 1);
        } else {
          self.display_lines.push(DisplayLine::ExpandButton {
            region_id,
            hidden_lines: gap_lines,
            start_old_lineno: 1,
            start_new_lineno: 1,
          });
        }
        region_id += 1;
      }

      // Show all lines from this hunk
      for i in 0..hunk.lines.len() {
        self.push_line(line_offset + i);
      }
      line_offset += hunk.lines.len();

      // Handle gap between this hunk and the next
      if !is_last {
        let next_hunk = &hunks[hunk_idx + 1];
        let current_end = hunk.new_start + hunk.new_lines;
        let next_start = next_hunk.new_start;

        if next_start > current_end {
          let gap_lines = (next_start - current_end) as usize;
          if self.is_region_expanded(region_id) {
            // Load and show lines from new_content
            let old_start_line = hunk.old_start + hunk.old_lines;
            let old_end_line = next_hunk.old_start - 1;
            self.load_gap_lines(current_end, next_start - 1, old_start_line, old_end_line);
          } else {
            self.display_lines.push(DisplayLine::ExpandButton {
              region_id,
              hidden_lines: gap_lines,
              start_old_lineno: hunk.old_start + hunk.old_lines,
              start_new_lineno: current_end,
            });
          }
          region_id += 1;
        }
      }
    }
  }

  /// Load and display lines from a gap between hunks (unchanged lines)
  fn load_gap_lines(
    &mut self,
    new_start_line: u32,
    new_end_line: u32,
    old_start_line: u32,
    old_end_line: u32,
  ) {
    // Get lines from new_content
    if let Some(new_content) = &self.file_diff.new_content {
      let all_lines: Vec<&str> = new_content.lines().collect();

      // Convert to 0-based indices
      let start_idx = (new_start_line - 1) as usize;
      let end_idx = new_end_line as usize;

      for (i, line_idx) in (start_idx..end_idx).enumerate() {
        if let Some(line_content) = all_lines.get(line_idx) {
          let new_lineno = new_start_line + i as u32;
          let old_lineno = old_start_line + i as u32;

          self.display_lines.push(DisplayLine::Line {
            line_idx: 0, // Not from all_lines array
            old_lineno: Some(old_lineno),
            new_lineno: Some(new_lineno),
            content: line_content.to_string(),
            origin: Some(LineOrigin::Context),
          });
        }
      }
    }
  }

  /// Helper to push a line to display_lines
  fn push_line(&mut self, line_idx: usize) {
    if let Some(line_info) = self.all_lines.get(line_idx) {
      self.display_lines.push(DisplayLine::Line {
        line_idx,
        old_lineno: line_info.old_lineno,
        new_lineno: line_info.new_lineno,
        content: line_info.content.clone(),
        origin: Some(line_info.origin),
      });
    }
  }

  /// Render a single display line
  fn render_display_line(
    &mut self,
    display_line: &DisplayLine,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    match display_line {
      DisplayLine::Line {
        old_lineno,
        new_lineno,
        content,
        origin,
        ..
      } => self
        .render_line(*old_lineno, *new_lineno, content, *origin)
        .into_any_element(),
      DisplayLine::ExpandButton {
        region_id,
        hidden_lines,
        start_old_lineno,
        start_new_lineno,
      } => self
        .render_expand_button(
          *region_id,
          *hidden_lines,
          *start_old_lineno,
          *start_new_lineno,
          cx,
        )
        .into_any_element(),
    }
  }

  /// Render a single line
  fn render_line(
    &mut self,
    old_lineno: Option<u32>,
    new_lineno: Option<u32>,
    line_content: &str,
    line_origin: Option<LineOrigin>,
  ) -> impl IntoElement {
    let (bg_color, fg_color) = match line_origin {
      Some(LineOrigin::Addition) => (Colors::diff_addition_bg(), Colors::text_primary()),
      Some(LineOrigin::Deletion) => (Colors::diff_deletion_bg(), Colors::text_primary()),
      _ => (Colors::bg_primary(), Colors::text_primary()),
    };

    let old_num = old_lineno.map_or("".to_string(), |n| format!("{}", n));
    let new_num = new_lineno.map_or("".to_string(), |n| format!("{}", n));

    // Helper to render plain text without highlighting
    let plain_text = || {
      div()
        .text_color(fg_color)
        .child(line_content.to_string())
        .into_any_element()
    };

    // Try to highlight the line with syntax highlighting
    let content_element = if let Some(highlighter) = &mut self.syntax_highlighter {
      let runs = highlighter.highlight_line(line_content, fg_color);
      if runs.len() > 1 || (runs.len() == 1 && runs[0].color != fg_color) {
        // We have syntax highlighting
        StyledText::new(line_content.to_string())
          .with_runs(runs)
          .into_any_element()
      } else {
        // Fallback to plain text
        plain_text()
      }
    } else {
      // No highlighter, use plain text
      plain_text()
    };

    div()
      .flex()
      .items_center()
      .w_full()
      .bg(bg_color)
      .h(px(24.0))
      .text_xs()
      .font_family("monospace")
      .child(
        div()
          .flex()
          .items_center()
          .text_color(Colors::text_muted())
          .child(
            div()
              .flex()
              .items_center()
              .justify_center()
              .w(px(40.0))
              .child(old_num),
          )
          .child(
            div()
              .flex()
              .items_center()
              .justify_center()
              .w(px(40.0))
              .child(new_num),
          ),
      )
      .child(
        div()
          .px(px(8.0))
          .line_height(relative(0.0))
          .child(content_element),
      )
  }

  /// Render an expand button for gap between hunks
  fn render_expand_button(
    &self,
    region_id: usize,
    hidden_lines: usize,
    start_old_lineno: u32,
    start_new_lineno: u32,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    div()
      .id(format!("expand-gap-{}", region_id))
      .flex()
      .items_center()
      .justify_between()
      .w_full()
      .h(px(24.0))
      .px(px(16.0))
      .bg(Colors::bg_primary())
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
      .text_xs()
      .child(
        div()
          .flex()
          .items_center()
          .gap_2()
          .child(div().text_color(Colors::text_muted()).child("⋯"))
          .child(
            div()
              .text_color(Colors::text_muted())
              .child(format!("{} hidden lines", hidden_lines)),
          ),
      )
      .child(div().text_color(Colors::text_muted()).child(format!(
        "Lines {}-{}",
        start_new_lineno,
        start_new_lineno + hidden_lines as u32 - 1
      )))
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

    let total_display_lines = self.display_lines.len();

    // Calculate additions and deletions
    let mut additions = 0;
    let mut deletions = 0;
    for line in &self.all_lines {
      match line.origin {
        LineOrigin::Addition => additions += 1,
        LineOrigin::Deletion => deletions += 1,
        _ => {}
      }
    }

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
          .py(px(8.0))
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
              .when(additions > 0, |this| {
                this.child(
                  div()
                    .text_xs()
                    .text_color(Colors::success())
                    .child(format!("+{}", additions)),
                )
              })
              .when(deletions > 0, |this| {
                this.child(
                  div()
                    .text_xs()
                    .text_color(Colors::destructive())
                    .child(format!("-{}", deletions)),
                )
              }),
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
