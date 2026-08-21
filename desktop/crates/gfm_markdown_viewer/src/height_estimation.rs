use crate::constants::*;
use crate::image::{
  InlineImageData, MarkdownImageDimension, inline_contains_image, inline_image_data,
  parse_markdown_image_dimension, single_inline_image_data, split_inlines_by_hard_breaks,
};
use crate::parse::inline_to_plain_text_with_soft_break;
use crate::parsed_cache::parse_markdown_for_render;
use crate::types::*;

/// How wide a run of text is, in the same column unit as the wrap budget: the
/// caller divides its measured pixels by whatever it called a column.
pub type MarkdownTextWidthFn<'a> = &'a dyn Fn(&str) -> f32;

/// The wrap budget, and how to measure text against it. Counting characters makes
/// a run of `i` wrap as late as a run of `M`; a measured width does not.
#[derive(Clone, Copy)]
pub struct MarkdownTextMetrics<'a> {
  wrap_columns: usize,
  width_of: Option<MarkdownTextWidthFn<'a>>,
}

impl<'a> MarkdownTextMetrics<'a> {
  /// Falls back to counting characters, one column each.
  pub fn columns(wrap_columns: usize) -> Self {
    Self {
      wrap_columns: wrap_columns.max(MARKDOWN_MIN_WRAP_COLUMNS),
      width_of: None,
    }
  }

  pub fn measured(wrap_columns: usize, width_of: MarkdownTextWidthFn<'a>) -> Self {
    Self {
      wrap_columns: wrap_columns.max(MARKDOWN_MIN_WRAP_COLUMNS),
      width_of: Some(width_of),
    }
  }

  pub(crate) fn wrap_columns(&self) -> usize {
    self.wrap_columns
  }

  pub(crate) fn width(&self, text: &str) -> f32 {
    match self.width_of {
      Some(width_of) => width_of(text),
      None => text.chars().count() as f32,
    }
  }

  pub(crate) fn with_indent(self, indent: usize) -> Self {
    Self {
      wrap_columns: wrap_columns_for_indent(self.wrap_columns, indent),
      ..self
    }
  }

  pub(crate) fn with_wrap_columns(self, wrap_columns: usize) -> Self {
    Self {
      wrap_columns: wrap_columns.max(MARKDOWN_MIN_WRAP_COLUMNS),
      ..self
    }
  }
}

pub fn estimate_markdown_height_px_with_suggestion_context(
  source: &str,
  metrics: MarkdownTextMetrics<'_>,
  line_height_px: f32,
  suggestion_context: Option<&SuggestionContext>,
) -> f32 {
  let parsed = parse_markdown_for_render(source);
  estimate_parsed_markdown_height_px_with_suggestion_context(
    &parsed,
    metrics,
    line_height_px,
    suggestion_context,
  )
}

pub fn estimate_parsed_markdown_height_px_with_suggestion_context(
  parsed: &ParsedMarkdown,
  metrics: MarkdownTextMetrics<'_>,
  line_height_px: f32,
  suggestion_context: Option<&SuggestionContext>,
) -> f32 {
  estimate_blocks_height_px(
    parsed.blocks.as_ref(),
    metrics,
    line_height_px.max(1.0),
    0,
    suggestion_context,
  )
}

pub(crate) fn estimate_blocks_height_px(
  blocks: &[Block],
  metrics: MarkdownTextMetrics<'_>,
  line_height_px: f32,
  indent: usize,
  suggestion_context: Option<&SuggestionContext>,
) -> f32 {
  if blocks.is_empty() {
    return line_height_px;
  }

  let mut total = 0.0f32;
  for (ix, block) in blocks.iter().enumerate() {
    if ix > 0 {
      total += MARKDOWN_BASE_BLOCK_GAP_PX;
      if matches!(block, Block::Heading { .. }) {
        total += MARKDOWN_HEADING_EXTRA_TOP_MARGIN_PX;
      }
    }
    total += estimate_block_height_px(block, metrics, line_height_px, indent, suggestion_context);
  }

  total.max(line_height_px)
}

pub(crate) fn estimate_block_height_px(
  block: &Block,
  metrics: MarkdownTextMetrics<'_>,
  line_height_px: f32,
  indent: usize,
  suggestion_context: Option<&SuggestionContext>,
) -> f32 {
  match block {
    Block::Paragraph(inlines) => {
      estimate_inline_content_height_px(inlines, metrics.with_indent(indent), line_height_px)
    }
    Block::Heading { level, content } => {
      let lines = estimate_inline_lines(content, metrics.with_indent(indent)).max(1);
      let scale = match level {
        1 => 1.35,
        2 => 1.2,
        3 => 1.05,
        _ => 1.0,
      };
      lines as f32 * line_height_px * scale
    }
    Block::List(list) => {
      estimate_list_height_px(list, metrics, line_height_px, indent, suggestion_context)
    }
    Block::CodeBlock(code) => {
      if code.lang.as_deref() == Some("suggestion") {
        return estimate_suggestion_block_height_px(code, line_height_px, suggestion_context);
      }

      let code_lines = code.value.lines().count().max(1) as f32;
      (code_lines * line_height_px * MARKDOWN_CODE_LINE_HEIGHT_SCALE
        + MARKDOWN_CODE_BLOCK_VERTICAL_CHROME_PX)
        .min(MARKDOWN_CODE_BLOCK_MAX_HEIGHT_PX)
    }
    Block::BlockQuote(children) => estimate_blocks_height_px(
      children,
      metrics,
      line_height_px,
      indent + 1,
      suggestion_context,
    ),
    Block::ThematicBreak => 1.0,
    Block::Table(table) => estimate_table_height_px(table, line_height_px),
    Block::Details(details) => {
      estimate_details_height_px(details, metrics, line_height_px, indent, suggestion_context)
    }
    Block::Aligned { blocks, .. } => {
      estimate_blocks_height_px(blocks, metrics, line_height_px, indent, suggestion_context)
    }
  }
}

fn estimate_suggestion_block_height_px(
  code: &CodeBlock,
  line_height_px: f32,
  suggestion_context: Option<&SuggestionContext>,
) -> f32 {
  let suggested_value = code.value.strip_suffix('\n').unwrap_or(code.value.as_str());
  let suggested_line_count = if suggested_value.is_empty() {
    0
  } else {
    suggested_value.split('\n').count()
  };
  let original_line_count = suggestion_context
    .map(|context| context.original_lines.len())
    .unwrap_or_default();
  let diff_line_count = (original_line_count + suggested_line_count).max(1) as f32;

  MARKDOWN_SUGGESTION_BLOCK_HEADER_PX
    + MARKDOWN_SUGGESTION_BLOCK_BORDER_PX
    + diff_line_count * line_height_px * MARKDOWN_CODE_LINE_HEIGHT_SCALE
}

pub(crate) fn estimate_list_height_px(
  list: &List,
  metrics: MarkdownTextMetrics<'_>,
  line_height_px: f32,
  indent: usize,
  suggestion_context: Option<&SuggestionContext>,
) -> f32 {
  if list.items.is_empty() {
    return line_height_px;
  }

  let indent_cols =
    ((LIST_LEFT_PADDING_PX + LIST_MARKER_GAP_PX + 14.0) / MARKDOWN_CHAR_WIDTH_PX).ceil() as usize;
  let item_wrap_columns = wrap_columns_for_indent(metrics.wrap_columns(), indent)
    .saturating_sub(indent_cols)
    .max(MARKDOWN_MIN_WRAP_COLUMNS);

  let mut total = 0.0f32;
  for (ix, item) in list.items.iter().enumerate() {
    if ix > 0 {
      total += MARKDOWN_LIST_ITEM_GAP_PX;
    }
    total += estimate_blocks_height_px(
      &item.blocks,
      metrics.with_wrap_columns(item_wrap_columns),
      line_height_px,
      indent + 1,
      suggestion_context,
    );
  }

  total.max(line_height_px)
}

pub(crate) fn estimate_details_height_px(
  details: &Details,
  metrics: MarkdownTextMetrics<'_>,
  line_height_px: f32,
  indent: usize,
  suggestion_context: Option<&SuggestionContext>,
) -> f32 {
  let summary_cols = wrap_columns_for_indent(metrics.wrap_columns(), indent)
    .saturating_sub(3)
    .max(MARKDOWN_MIN_WRAP_COLUMNS);
  let summary_height =
    estimate_inline_lines(&details.summary, metrics.with_wrap_columns(summary_cols)).max(1) as f32
      * line_height_px;

  if !details.open {
    return summary_height;
  }

  let body = estimate_blocks_height_px(
    &details.blocks,
    metrics,
    line_height_px,
    indent + 1,
    suggestion_context,
  );
  summary_height + MARKDOWN_BASE_BLOCK_GAP_PX + body
}

pub(crate) fn estimate_table_height_px(table: &Table, line_height_px: f32) -> f32 {
  let header_content_height = estimate_table_row_content_height_px(&table.headers, line_height_px);
  let header_row_height = header_content_height + 16.0;
  let mut total = header_row_height + 2.0;

  for row in &table.rows {
    let content_height = estimate_table_row_content_height_px(row, line_height_px);
    total += content_height + 16.0 + 1.0;
  }

  total.max(line_height_px)
}

pub(crate) fn estimate_table_row_content_height_px(
  cells: &[Vec<Inline>],
  line_height_px: f32,
) -> f32 {
  let mut max_height = line_height_px;
  for cell in cells {
    let has_image = cell.iter().any(inline_contains_image);
    let cell_height = if has_image { 18.0 } else { line_height_px };
    max_height = max_height.max(cell_height);
  }
  max_height
}

pub(crate) fn estimate_inline_content_height_px(
  inlines: &[Inline],
  metrics: MarkdownTextMetrics<'_>,
  line_height_px: f32,
) -> f32 {
  if !inlines.iter().any(inline_contains_image) {
    return estimate_inline_lines(inlines, metrics).max(1) as f32 * line_height_px;
  }

  if let Some(image_data) = single_inline_image_data(inlines) {
    return estimate_image_height_px(&image_data, metrics, line_height_px);
  }

  let rows = split_inlines_by_hard_breaks(inlines);
  let mut total = 0.0f32;

  for (row_ix, row) in rows.iter().enumerate() {
    total += estimate_inline_row_height_px(row, metrics, line_height_px);
    if row_ix + 1 < rows.len() {
      total += MARKDOWN_IMAGE_HARD_BREAK_SPACER_PX;
    }
  }

  total.max(line_height_px)
}

fn estimate_inline_row_height_px(
  inlines: &[Inline],
  metrics: MarkdownTextMetrics<'_>,
  line_height_px: f32,
) -> f32 {
  if inlines.is_empty() {
    return line_height_px;
  }

  let mut total = 0.0f32;
  let mut row_height = 0.0f32;
  let mut text_chunk: Vec<Inline> = Vec::new();

  for inline in inlines {
    if let Some(image_data) = inline_image_data(inline) {
      if !text_chunk.is_empty() {
        let text_height =
          estimate_inline_lines(&text_chunk, metrics).max(1) as f32 * line_height_px;
        row_height = row_height.max(text_height);
        text_chunk.clear();
      }

      let image_height = estimate_image_height_px(&image_data, metrics, line_height_px);
      if image_data.is_block_sized() {
        total += row_height;
        total += image_height;
        row_height = 0.0;
      } else {
        row_height = row_height.max(image_height);
      }
    } else {
      text_chunk.push(inline.clone());
    }
  }

  if !text_chunk.is_empty() {
    let text_height = estimate_inline_lines(&text_chunk, metrics).max(1) as f32 * line_height_px;
    row_height = row_height.max(text_height);
  }

  (total + row_height).max(line_height_px)
}

fn estimate_image_height_px(
  image_data: &InlineImageData,
  metrics: MarkdownTextMetrics<'_>,
  line_height_px: f32,
) -> f32 {
  let line_height_px = line_height_px.max(1.0);
  let available_width_px =
    (metrics.wrap_columns() as f32 * MARKDOWN_CHAR_WIDTH_PX).max(line_height_px);

  let width_hint = parse_markdown_image_dimension(image_data.width_hint.as_deref());
  let height_hint = parse_markdown_image_dimension(image_data.height_hint.as_deref());

  let estimated = match (width_hint, height_hint) {
    (Some(MarkdownImageDimension::Pixels(width)), Some(MarkdownImageDimension::Pixels(height))) => {
      let clamped_width = width.max(1.0).min(available_width_px);
      height.max(1.0) * (clamped_width / width.max(1.0))
    }
    (_, Some(MarkdownImageDimension::Pixels(height))) => height.max(1.0),
    (Some(MarkdownImageDimension::Pixels(width)), _) if image_data.is_block_sized() => {
      let clamped_width = width.max(1.0).min(available_width_px);
      clamped_width * 9.0 / 16.0
    }
    _ if image_data.is_block_sized() => available_width_px * 9.0 / 16.0,
    _ => 18.0,
  };

  estimated
    .max(line_height_px)
    .min(MARKDOWN_INLINE_IMAGE_MAX_HEIGHT_PX)
}

pub(crate) fn wrap_columns_for_indent(base_wrap_columns: usize, indent: usize) -> usize {
  let indent_columns =
    ((indent as f32 * MARKDOWN_INDENT_PER_LEVEL_PX) / MARKDOWN_CHAR_WIDTH_PX).ceil() as usize;
  base_wrap_columns
    .saturating_sub(indent_columns)
    .max(MARKDOWN_MIN_WRAP_COLUMNS)
}

pub(crate) fn estimate_inline_lines(inlines: &[Inline], metrics: MarkdownTextMetrics<'_>) -> usize {
  // Review comments render in hardbreaks mode, so a soft break costs a line.
  let text = inline_to_plain_text_with_soft_break(inlines, '\n');
  if text.is_empty() {
    return 1;
  }

  let mut lines = 0usize;
  for line in text.split('\n') {
    lines = lines.saturating_add(estimate_wrapped_text_lines(line, metrics));
  }
  lines.max(1)
}

pub(crate) fn estimate_wrapped_text_lines(line: &str, metrics: MarkdownTextMetrics<'_>) -> usize {
  if line.is_empty() {
    return 1;
  }

  let wrap = metrics.wrap_columns() as f32;
  let space = metrics.width(" ").max(f32::EPSILON);
  let mut wrapped_lines = 0usize;
  let mut current = 0.0f32;
  let mut has_word = false;

  for word in line.split_whitespace() {
    has_word = true;
    let mut word_width = metrics.width(word);
    if word_width <= 0.0 {
      continue;
    }

    if current > 0.0 {
      if current + space + word_width <= wrap {
        current += space + word_width;
        continue;
      }
      wrapped_lines = wrapped_lines.saturating_add(1);
    }

    // A word wider than the line is broken as many times as it overflows.
    let overflow_lines = (word_width / wrap).floor();
    wrapped_lines = wrapped_lines.saturating_add(overflow_lines as usize);
    word_width -= overflow_lines * wrap;
    current = word_width;
  }

  if !has_word {
    return 1;
  }

  if current > 0.0 {
    wrapped_lines = wrapped_lines.saturating_add(1);
  }
  wrapped_lines.max(1)
}

pub fn estimate_github_code_reference_preview_height_px(
  snippet_line_count: usize,
  row_height_px: f32,
) -> f32 {
  let row_height_px = row_height_px.max(1.0);
  let snippet_line_count = snippet_line_count.max(1) as f32;
  let snippet_rows_height_px = row_height_px * snippet_line_count
    + MARKDOWN_CODE_REFERENCE_SNIPPET_ROW_GAP_PX * (snippet_line_count - 1.0).max(0.0);
  let snippet_scroll_height_px = snippet_rows_height_px.min(MARKDOWN_CODE_BLOCK_MAX_HEIGHT_PX);

  row_height_px * 2.0
    + MARKDOWN_CODE_REFERENCE_CARD_PADDING_Y_PX * 4.0
    + MARKDOWN_CODE_REFERENCE_CARD_MARGIN_Y_PX * 2.0
    + 3.0
    + snippet_scroll_height_px
}

pub(crate) fn estimate_code_reference_preview_min_content_width_px(
  preview: &GithubCodeReferencePreview,
) -> f32 {
  let widest_row_columns = if preview.snippets.is_empty() {
    preview.start_line.to_string().chars().count()
  } else {
    preview
      .snippets
      .iter()
      .enumerate()
      .map(|(offset, snippet)| {
        let line_number_columns = (preview.start_line + offset).to_string().chars().count();
        line_number_columns + 2 + snippet.chars().count()
      })
      .max()
      .unwrap_or(0)
  } as f32;

  (widest_row_columns * MARKDOWN_CODE_BLOCK_APPROX_CHAR_WIDTH_PX
    + MARKDOWN_CODE_REFERENCE_ROW_GAP_PX)
    .ceil()
}

#[cfg(test)]
mod tests {
  use super::{
    MarkdownTextMetrics, estimate_parsed_markdown_height_px_with_suggestion_context,
    estimate_wrapped_text_lines,
  };
  use crate::parsed_cache::parse_markdown_for_render;
  use crate::types::SuggestionContext;
  use std::sync::Arc;

  #[test]
  fn explicit_image_dimensions_reserve_more_than_a_single_text_line() {
    let parsed = parse_markdown_for_render(
      r#"<img width="1159" height="272" alt="Image" src="https://github.com/user-attachments/assets/525e1fe3-1159-47ea-a1ac-8926a03c9cd1" />"#,
    );

    let height = estimate_parsed_markdown_height_px_with_suggestion_context(
      &parsed,
      MarkdownTextMetrics::columns(72),
      20.0,
      None,
    );

    assert!(
      height > 100.0,
      "image height should reserve more than a single line"
    );
  }

  #[test]
  fn bare_user_attachment_links_reserve_block_image_height() {
    let parsed = parse_markdown_for_render(
      "https://github.com/user-attachments/assets/4aa12d28-968a-490d-81ee-32bbbb595fc4",
    );

    let height = estimate_parsed_markdown_height_px_with_suggestion_context(
      &parsed,
      MarkdownTextMetrics::columns(72),
      20.0,
      None,
    );

    assert!(
      height > 100.0,
      "attachment links should reserve block image space"
    );
  }

  #[test]
  fn a_measured_run_wraps_on_its_own_width_not_its_length() {
    let source = "iiiiiiiiiiiiiiiiiiii iiiiiiiiiiiiiiiiiiii";
    let parsed = parse_markdown_for_render(source);
    let counted = estimate_parsed_markdown_height_px_with_suggestion_context(
      &parsed,
      MarkdownTextMetrics::columns(30),
      20.0,
      None,
    );

    // The same budget, told those glyphs are half a column wide each.
    let half = |text: &str| text.chars().count() as f32 / 2.0;
    let measured = estimate_parsed_markdown_height_px_with_suggestion_context(
      &parsed,
      MarkdownTextMetrics::measured(30, &half),
      20.0,
      None,
    );

    assert_eq!(counted, 40.0, "41 characters do not fit in 30 columns");
    assert_eq!(measured, 20.0, "half as wide, they fit on one line");
  }

  #[test]
  fn a_word_wider_than_the_line_is_broken_as_many_times_as_it_overflows() {
    let wide = |text: &str| text.chars().count() as f32 * 2.0;
    let metrics = MarkdownTextMetrics::measured(30, &wide);

    // 30 characters at two columns each: three lines of thirty columns.
    assert_eq!(estimate_wrapped_text_lines(&"a".repeat(30), metrics), 2);
    assert_eq!(estimate_wrapped_text_lines(&"a".repeat(45), metrics), 3);
  }

  #[test]
  fn a_soft_break_costs_a_line() {
    let height = |source: &str| {
      estimate_parsed_markdown_height_px_with_suggestion_context(
        &parse_markdown_for_render(source),
        MarkdownTextMetrics::columns(72),
        20.0,
        None,
      )
    };
    let one_line = height("one two");
    let two_lines = height("one\ntwo");

    assert_eq!(two_lines, one_line + 20.0);
  }

  #[test]
  fn suggestion_context_reserves_original_and_suggested_diff_rows() {
    let parsed = parse_markdown_for_render(
      r#"```suggestion
new line
```"#,
    );
    let default_height = estimate_parsed_markdown_height_px_with_suggestion_context(
      &parsed,
      MarkdownTextMetrics::columns(72),
      20.0,
      None,
    );
    let context = SuggestionContext {
      original_start_line: Some(10),
      suggested_start_line: Some(10),
      original_lines: vec!["old line".to_string(), "older line".to_string()],
      path: Arc::from("src/main.rs"),
    };

    let suggestion_height = estimate_parsed_markdown_height_px_with_suggestion_context(
      &parsed,
      MarkdownTextMetrics::columns(72),
      20.0,
      Some(&context),
    );

    assert!(
      suggestion_height > default_height,
      "suggestion height should include original diff rows"
    );
  }
}
