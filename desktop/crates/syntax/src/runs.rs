use std::sync::Arc;

use gpui::{Font, Hsla, TextRun};

use crate::{HighlightSpan, LanguageConfig, SyntaxHighlighter, SyntaxTheme};

pub fn clamp_to_char_boundary(text: &str, byte_offset: usize) -> usize {
  let mut byte_offset = byte_offset.min(text.len());
  while byte_offset > 0 && !text.is_char_boundary(byte_offset) {
    byte_offset -= 1;
  }
  byte_offset
}

pub fn compute_line_bounds(text: &str) -> Vec<(usize, usize)> {
  let bytes = text.as_bytes();
  let mut bounds = Vec::new();
  let mut line_start = 0;

  for (idx, byte) in bytes.iter().enumerate() {
    if *byte == b'\n' {
      let mut line_end = idx;
      if line_end > line_start && bytes[line_end - 1] == b'\r' {
        line_end -= 1;
      }
      bounds.push((line_start, line_end));
      line_start = idx + 1;
    }
  }

  let mut line_end = bytes.len();
  if line_end > line_start && bytes[line_end - 1] == b'\r' {
    line_end -= 1;
  }
  bounds.push((line_start, line_end));

  bounds
}

pub fn line_index_for_byte(line_starts: &[usize], offset: usize) -> usize {
  match line_starts.binary_search(&offset) {
    Ok(idx) => idx,
    Err(idx) => idx.saturating_sub(1),
  }
}

pub fn highlight_text_to_line_spans(
  text: &str,
  config: &'static LanguageConfig,
) -> Result<Vec<Arc<[HighlightSpan]>>, String> {
  let line_bounds = compute_line_bounds(text);
  let line_starts: Vec<usize> = line_bounds.iter().map(|(start, _)| *start).collect();
  let mut line_spans: Vec<Vec<HighlightSpan>> = vec![Vec::new(); line_bounds.len()];

  let mut highlighter = SyntaxHighlighter::new(config);
  highlighter.highlight_text_stream(
    text,
    |_| true,
    |span| {
      let start_line = line_index_for_byte(&line_starts, span.byte_range.start);
      let end_offset = span.byte_range.end.saturating_sub(1);
      let end_line = line_index_for_byte(&line_starts, end_offset);

      for line_idx in start_line..=end_line {
        let (line_start, line_end) = line_bounds[line_idx];
        let local_start = span.byte_range.start.max(line_start) - line_start;
        let local_end = span.byte_range.end.min(line_end) - line_start;
        if local_end > local_start {
          line_spans[line_idx].push(HighlightSpan {
            byte_range: local_start..local_end,
            token_type: span.token_type,
          });
        }
      }
      true
    },
  )?;

  Ok(line_spans.into_iter().map(Arc::from).collect())
}

pub fn highlights_to_text_runs(
  highlights: &[HighlightSpan],
  line_text: &str,
  base_color: Hsla,
  base_font: Font,
  syntax_theme: &SyntaxTheme,
) -> Vec<TextRun> {
  let mut runs = Vec::new();
  let line_len = line_text.len();
  let mut current_pos = 0;

  let mut line_highlights: Vec<_> = highlights
    .iter()
    .filter_map(|h| {
      let start = clamp_to_char_boundary(line_text, h.byte_range.start.min(line_len));
      let end = clamp_to_char_boundary(line_text, h.byte_range.end.min(line_len));
      (end > start).then_some((start..end, h.token_type))
    })
    .collect();
  line_highlights.sort_by_key(|(range, _)| range.start);

  for (range, token_type) in line_highlights {
    if range.start > current_pos {
      runs.push(TextRun {
        len: range.start - current_pos,
        font: base_font.clone(),
        color: base_color,
        background_color: None,
        underline: None,
        strikethrough: None,
      });
    }
    runs.push(TextRun {
      len: range.len(),
      font: base_font.clone(),
      color: syntax_theme.color_for_token(token_type),
      background_color: None,
      underline: None,
      strikethrough: None,
    });
    current_pos = range.end;
  }

  if current_pos < line_len {
    runs.push(TextRun {
      len: line_len - current_pos,
      font: base_font.clone(),
      color: base_color,
      background_color: None,
      underline: None,
      strikethrough: None,
    });
  }

  runs
}
