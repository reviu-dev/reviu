use crate::constants::*;
use crate::image::inline_contains_image;
use crate::parse::inline_to_plain_text;
use crate::parsed_cache::parse_markdown_for_render;
use crate::types::*;

pub fn estimate_markdown_height_px(source: &str, wrap_columns: usize, line_height_px: f32) -> f32 {
  let parsed = parse_markdown_for_render(source);
  estimate_parsed_markdown_height_px(&parsed, wrap_columns, line_height_px)
}

pub fn estimate_parsed_markdown_height_px(
  parsed: &ParsedMarkdown,
  wrap_columns: usize,
  line_height_px: f32,
) -> f32 {
  estimate_blocks_height_px(
    parsed.blocks.as_ref(),
    wrap_columns.max(MARKDOWN_MIN_WRAP_COLUMNS),
    line_height_px.max(1.0),
    0,
  )
}

pub(crate) fn estimate_blocks_height_px(
  blocks: &[Block],
  wrap_columns: usize,
  line_height_px: f32,
  indent: usize,
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
    total += estimate_block_height_px(block, wrap_columns, line_height_px, indent);
  }

  total.max(line_height_px)
}

pub(crate) fn estimate_block_height_px(
  block: &Block,
  wrap_columns: usize,
  line_height_px: f32,
  indent: usize,
) -> f32 {
  match block {
    Block::Paragraph(inlines) => {
      estimate_inline_lines(inlines, wrap_columns_for_indent(wrap_columns, indent)) as f32
        * line_height_px
    }
    Block::Heading { level, content } => {
      let lines =
        estimate_inline_lines(content, wrap_columns_for_indent(wrap_columns, indent)).max(1);
      let scale = match level {
        1 => 1.35,
        2 => 1.2,
        3 => 1.05,
        _ => 1.0,
      };
      lines as f32 * line_height_px * scale
    }
    Block::List(list) => estimate_list_height_px(list, wrap_columns, line_height_px, indent),
    Block::CodeBlock(code) => {
      let code_lines = code.value.lines().count().max(1) as f32;
      (code_lines * line_height_px * MARKDOWN_CODE_LINE_HEIGHT_SCALE
        + MARKDOWN_CODE_BLOCK_VERTICAL_CHROME_PX)
        .min(MARKDOWN_CODE_BLOCK_MAX_HEIGHT_PX)
    }
    Block::BlockQuote(children) => {
      estimate_blocks_height_px(children, wrap_columns, line_height_px, indent + 1)
    }
    Block::ThematicBreak => 1.0,
    Block::Table(table) => estimate_table_height_px(table, line_height_px),
    Block::Details(details) => {
      estimate_details_height_px(details, wrap_columns, line_height_px, indent)
    }
    Block::Aligned { blocks, .. } => {
      estimate_blocks_height_px(blocks, wrap_columns, line_height_px, indent)
    }
  }
}

pub(crate) fn estimate_list_height_px(
  list: &List,
  wrap_columns: usize,
  line_height_px: f32,
  indent: usize,
) -> f32 {
  if list.items.is_empty() {
    return line_height_px;
  }

  let indent_cols =
    ((LIST_LEFT_PADDING_PX + LIST_MARKER_GAP_PX + 14.0) / MARKDOWN_CHAR_WIDTH_PX).ceil() as usize;
  let item_wrap_columns = wrap_columns_for_indent(wrap_columns, indent)
    .saturating_sub(indent_cols)
    .max(MARKDOWN_MIN_WRAP_COLUMNS);

  let mut total = 0.0f32;
  for (ix, item) in list.items.iter().enumerate() {
    if ix > 0 {
      total += MARKDOWN_LIST_ITEM_GAP_PX;
    }
    total += estimate_blocks_height_px(&item.blocks, item_wrap_columns, line_height_px, indent + 1);
  }

  total.max(line_height_px)
}

pub(crate) fn estimate_details_height_px(
  details: &Details,
  wrap_columns: usize,
  line_height_px: f32,
  indent: usize,
) -> f32 {
  let summary_cols = wrap_columns_for_indent(wrap_columns, indent)
    .saturating_sub(3)
    .max(MARKDOWN_MIN_WRAP_COLUMNS);
  let summary_height =
    estimate_inline_lines(&details.summary, summary_cols).max(1) as f32 * line_height_px;

  if !details.open {
    return summary_height;
  }

  let body = estimate_blocks_height_px(&details.blocks, wrap_columns, line_height_px, indent + 1);
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

pub(crate) fn wrap_columns_for_indent(base_wrap_columns: usize, indent: usize) -> usize {
  let indent_columns =
    ((indent as f32 * MARKDOWN_INDENT_PER_LEVEL_PX) / MARKDOWN_CHAR_WIDTH_PX).ceil() as usize;
  base_wrap_columns
    .saturating_sub(indent_columns)
    .max(MARKDOWN_MIN_WRAP_COLUMNS)
}

pub(crate) fn estimate_inline_lines(inlines: &[Inline], wrap_columns: usize) -> usize {
  let text = inline_to_plain_text(inlines);
  if text.is_empty() {
    return 1;
  }

  let mut lines = 0usize;
  for line in text.split('\n') {
    lines = lines.saturating_add(estimate_wrapped_text_lines(line, wrap_columns));
  }
  lines.max(1)
}

pub(crate) fn estimate_wrapped_text_lines(line: &str, wrap_columns: usize) -> usize {
  let wrap_columns = wrap_columns.max(1);
  if line.is_empty() {
    return 1;
  }

  let mut wrapped_lines = 0usize;
  let mut current_len = 0usize;
  let mut has_word = false;

  for word in line.split_whitespace() {
    has_word = true;
    let mut word_len = word.chars().count();
    if word_len == 0 {
      continue;
    }

    if current_len == 0 {
      if word_len <= wrap_columns {
        current_len = word_len;
      } else {
        wrapped_lines = wrapped_lines.saturating_add(word_len / wrap_columns);
        word_len %= wrap_columns;
        current_len = word_len;
      }
      continue;
    }

    if current_len + 1 + word_len <= wrap_columns {
      current_len += 1 + word_len;
      continue;
    }

    wrapped_lines = wrapped_lines.saturating_add(1);
    if word_len <= wrap_columns {
      current_len = word_len;
    } else {
      wrapped_lines = wrapped_lines.saturating_add(word_len / wrap_columns);
      word_len %= wrap_columns;
      current_len = word_len;
    }
  }

  if !has_word {
    return 1;
  }

  if current_len > 0 {
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
