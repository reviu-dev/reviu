use agent_client_protocol::schema::{ContentBlock, ToolCallContent};
use diff_core::{DiffRowKind, diff_rows, line_diff_counts};
use syntax::HighlightSpan;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct DiffSummary {
  pub path: String,
  pub old_text: Option<String>,
  pub new_text: String,
  pub added: u32,
  pub removed: u32,
  #[serde(default)]
  pub lines: Vec<DiffLine>,
  #[serde(skip)]
  pub expanded: bool,
}

impl DiffSummary {
  pub(crate) fn first_changed_line(&self) -> Option<u32> {
    self.lines.iter().find_map(|line| match line.kind {
      DiffLineKind::Added => line.snapshot_line(),
      DiffLineKind::Removed => line.snapshot_line(),
      DiffLineKind::Context | DiffLineKind::Gap => None,
    })
  }
}

/// Visible diff lines per block when collapsed. Beyond this, an expander button is shown.
pub(crate) const MAX_DIFF_LINES_COLLAPSED: usize = 40;

/// Visible tool output lines per block when collapsed. Beyond this, an expander button is shown.
pub(crate) const MAX_TOOL_OUTPUT_LINES_COLLAPSED: usize = 20;

#[derive(Clone, Debug, PartialEq, Eq)]
struct ParsedCatNumberedCode {
  code: String,
  first_number: u32,
}

fn strip_line_ending(line: &str) -> &str {
  let without_lf = line.strip_suffix('\n').unwrap_or(line);
  without_lf.strip_suffix('\r').unwrap_or(without_lf)
}

fn parse_cat_numbered_line(line: &str) -> Option<(u32, &str)> {
  let (prefix, text) = line.split_once('\t')?;
  let number = prefix.trim();
  if number.is_empty()
    || !prefix
      .chars()
      .all(|character| character == ' ' || character.is_ascii_digit())
  {
    return None;
  }
  Some((number.parse().ok()?, text))
}

fn parse_cat_numbered_code(code: &str) -> Option<ParsedCatNumberedCode> {
  if code.is_empty() {
    return None;
  }

  let mut output = String::with_capacity(code.len());
  let mut first_number = None;
  let mut expected_number = None;
  for (line_count, raw_line) in code.split_inclusive('\n').enumerate() {
    let line = strip_line_ending(raw_line);
    let (number, text) = parse_cat_numbered_line(line)?;
    if let Some(expected) = expected_number {
      if number != expected {
        return None;
      }
    } else {
      first_number = Some(number);
    }
    expected_number = number.checked_add(1);
    if line_count > 0 {
      output.push('\n');
    }
    output.push_str(text);
  }

  Some(ParsedCatNumberedCode {
    code: output,
    first_number: first_number?,
  })
}

fn normalize_output_text(
  text: &str,
  start_line: Option<u32>,
  strip_numbered_lines: bool,
) -> (String, Option<u32>) {
  let unfenced = crate::strip_markdown_code_fence(text);
  if strip_numbered_lines && let Some(parsed) = parse_cat_numbered_code(unfenced) {
    return (parsed.code, Some(parsed.first_number));
  }
  (unfenced.to_string(), start_line)
}

fn tool_output(
  text: &str,
  start_line: Option<u32>,
  strip_numbered_lines: bool,
) -> crate::ToolOutput {
  let (text, start_line) = normalize_output_text(text, start_line, strip_numbered_lines);
  crate::ToolOutput {
    text,
    start_line,
    expanded: false,
    syntax_spans: Vec::new(),
  }
}

fn raw_output_object_text(map: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
  if map.is_empty() {
    return None;
  }
  if map.get("type").and_then(|value| value.as_str()) == Some("text")
    && let Some(text) = map
      .get("text")
      .and_then(|value| value.as_str())
      .filter(|text| !text.trim().is_empty())
  {
    return Some(text.to_string());
  }

  for key in [
    "content",
    "formatted_output",
    "output",
    "result",
    "aggregatedOutput",
    "structuredContent",
  ] {
    if let Some(text) = map.get(key).and_then(raw_output_text) {
      return Some(text);
    }
  }

  let stream_output = ["stdout", "stderr"]
    .into_iter()
    .filter_map(|key| map.get(key).and_then(raw_output_text))
    .collect::<Vec<_>>();
  if !stream_output.is_empty() {
    return Some(stream_output.join("\n"));
  }

  serde_json::to_string_pretty(map).ok()
}

fn raw_output_text(value: &serde_json::Value) -> Option<String> {
  match value {
    serde_json::Value::Null => None,
    serde_json::Value::String(text) => (!text.trim().is_empty()).then(|| text.clone()),
    serde_json::Value::Array(items) => {
      let parts = items.iter().filter_map(raw_output_text).collect::<Vec<_>>();
      if parts.is_empty() {
        serde_json::to_string_pretty(value).ok()
      } else {
        Some(parts.join("\n"))
      }
    }
    serde_json::Value::Object(map) => raw_output_object_text(map),
    serde_json::Value::Bool(_) | serde_json::Value::Number(_) => Some(value.to_string()),
  }
}

/// Extracts text content blocks from tool call output. Returns one entry per `Content` block.
pub(crate) fn extract_outputs(
  content: &[ToolCallContent],
  start_line: Option<u32>,
  strip_numbered_lines: bool,
) -> Vec<crate::ToolOutput> {
  content
    .iter()
    .filter_map(|c| match c {
      ToolCallContent::Content(text_content) => match &text_content.content {
        ContentBlock::Text(t) => Some(tool_output(&t.text, start_line, strip_numbered_lines)),
        _ => None,
      },
      _ => None,
    })
    .collect()
}

pub(crate) fn extract_outputs_with_fallback(
  content: &[ToolCallContent],
  raw_output: Option<&serde_json::Value>,
  start_line: Option<u32>,
  strip_numbered_lines: bool,
) -> Vec<crate::ToolOutput> {
  let outputs = extract_outputs(content, start_line, strip_numbered_lines);
  if !outputs.is_empty() {
    return outputs;
  }
  raw_output
    .and_then(raw_output_text)
    .map(|text| vec![tool_output(&text, start_line, strip_numbered_lines)])
    .unwrap_or_default()
}

/// Terminal ids embedded in the tool call's content.
pub(crate) fn extract_terminals(content: &[ToolCallContent]) -> Vec<String> {
  content
    .iter()
    .filter_map(|c| match c {
      ToolCallContent::Terminal(t) => Some(t.terminal_id.0.to_string()),
      _ => None,
    })
    .collect()
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct DiffLine {
  pub kind: DiffLineKind,
  #[serde(default)]
  pub old_line: Option<u32>,
  #[serde(default)]
  pub new_line: Option<u32>,
  pub text: String,
  #[serde(default)]
  pub spans: Vec<InlineSpan>,
  #[serde(skip)]
  pub syntax_spans: Vec<HighlightSpan>,
}

impl DiffLine {
  pub(crate) fn snapshot_line(&self) -> Option<u32> {
    match self.kind {
      DiffLineKind::Context | DiffLineKind::Added => self.new_line.or(self.old_line),
      DiffLineKind::Gap => None,
      DiffLineKind::Removed => self.old_line,
    }
  }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub(crate) enum DiffLineKind {
  Context,
  Gap,
  Added,
  Removed,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub(crate) enum InlineSpanKind {
  Same,
  Diff,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct InlineSpan {
  pub kind: InlineSpanKind,
  pub text: String,
}

fn spans_from_ranges(text: &str, ranges: &[std::ops::Range<usize>]) -> Vec<InlineSpan> {
  if text.is_empty() {
    return Vec::new();
  }

  let mut spans = Vec::new();
  let mut cursor = 0usize;
  for range in ranges {
    let start = range.start.min(text.len());
    let end = range.end.min(text.len());
    if start >= end {
      continue;
    }
    if cursor < start
      && let Some(slice) = text.get(cursor..start)
    {
      spans.push(InlineSpan {
        kind: InlineSpanKind::Same,
        text: slice.to_string(),
      });
    }
    if let Some(slice) = text.get(start..end) {
      spans.push(InlineSpan {
        kind: InlineSpanKind::Diff,
        text: slice.to_string(),
      });
    }
    cursor = end;
  }

  if cursor < text.len()
    && let Some(slice) = text.get(cursor..)
  {
    spans.push(InlineSpan {
      kind: InlineSpanKind::Same,
      text: slice.to_string(),
    });
  }

  if spans.is_empty() {
    spans.push(InlineSpan {
      kind: InlineSpanKind::Same,
      text: text.to_string(),
    });
  }
  spans
}

#[cfg(test)]
pub(crate) fn word_diff_spans(old: &str, new: &str) -> (Vec<InlineSpan>, Vec<InlineSpan>) {
  if old != new && old.len().saturating_add(new.len()) > diff_core::WORD_DIFF_MAX_COMBINED_BYTES {
    return (Vec::new(), Vec::new());
  }

  let (removed_ranges, added_ranges) = diff_core::word_diff_ranges(old, new);
  (
    spans_from_ranges(old, &removed_ranges),
    spans_from_ranges(new, &added_ranges),
  )
}

pub(crate) fn build_diff_lines(old: &str, new: &str) -> Vec<DiffLine> {
  diff_rows(old, new)
    .into_iter()
    .map(|row| {
      let kind = match row.kind {
        DiffRowKind::Context => DiffLineKind::Context,
        DiffRowKind::Gap => DiffLineKind::Gap,
        DiffRowKind::Added => DiffLineKind::Added,
        DiffRowKind::Removed => DiffLineKind::Removed,
      };
      let spans = if matches!(kind, DiffLineKind::Context | DiffLineKind::Gap)
        || row.word_diff_ranges.is_empty()
      {
        Vec::new()
      } else {
        spans_from_ranges(&row.text, &row.word_diff_ranges)
      };
      DiffLine {
        kind,
        old_line: row.old_line,
        new_line: row.new_line,
        text: row.text,
        spans,
        syntax_spans: Vec::new(),
      }
    })
    .collect()
}

pub(crate) fn diff_line_counts(old_text: Option<&str>, new_text: &str) -> (u32, u32) {
  let old = old_text.unwrap_or("");
  let (added, removed) = line_diff_counts(old, new_text);
  (added, removed)
}

pub(crate) fn extract_diffs(
  content: &[ToolCallContent],
  cwd: &std::path::Path,
) -> Vec<DiffSummary> {
  content
    .iter()
    .filter_map(|c| match c {
      ToolCallContent::Diff(d) => {
        let (added, removed) = diff_line_counts(d.old_text.as_deref(), &d.new_text);
        let lines = build_diff_lines(d.old_text.as_deref().unwrap_or(""), &d.new_text);
        Some(DiffSummary {
          path: relativize_path(&d.path, cwd),
          old_text: d.old_text.clone(),
          new_text: d.new_text.clone(),
          added,
          removed,
          lines,
          expanded: false,
        })
      }
      _ => None,
    })
    .collect()
}

pub(crate) fn relativize_path(path: &std::path::Path, cwd: &std::path::Path) -> String {
  if let Ok(stripped) = path.strip_prefix(cwd) {
    return stripped.display().to_string();
  }
  if let (Ok(canon_path), Ok(canon_cwd)) = (path.canonicalize(), cwd.canonicalize())
    && let Ok(stripped) = canon_path.strip_prefix(&canon_cwd)
  {
    return stripped.display().to_string();
  }
  path.display().to_string()
}

#[cfg(test)]
mod tests {
  use super::*;

  fn test_cwd() -> &'static std::path::Path {
    std::path::Path::new("/")
  }

  #[test]
  fn word_diff_highlights_only_changed_words() {
    let (old, new) = word_diff_spans(
      "export const VERSION = '3.0.0';",
      "export const VERSION = '2.0.0';",
    );
    let old_diff: String = old
      .iter()
      .filter(|s| s.kind == InlineSpanKind::Diff)
      .map(|s| s.text.clone())
      .collect();
    let new_diff: String = new
      .iter()
      .filter(|s| s.kind == InlineSpanKind::Diff)
      .map(|s| s.text.clone())
      .collect();
    assert_eq!(old_diff, "3");
    assert_eq!(new_diff, "2");
  }

  #[test]
  fn word_diff_splits_camel_case_identifier() {
    let (old, new) = word_diff_spans("foo getLastValue bar", "foo getFirstValue bar");
    let old_diff: String = old
      .iter()
      .filter(|s| s.kind == InlineSpanKind::Diff)
      .map(|s| s.text.clone())
      .collect();
    let new_diff: String = new
      .iter()
      .filter(|s| s.kind == InlineSpanKind::Diff)
      .map(|s| s.text.clone())
      .collect();
    assert_eq!(old_diff, "Last");
    assert_eq!(new_diff, "First");
  }

  #[test]
  fn word_diff_falls_back_to_whitespace_when_only_indent_changes() {
    let (old, new) = word_diff_spans("  let x = 1;", "    let x = 1;");
    let has_diff = old
      .iter()
      .chain(new.iter())
      .any(|s| s.kind == InlineSpanKind::Diff);
    assert!(
      has_diff,
      "indent-only change should still produce highlight"
    );
  }

  #[test]
  fn word_diff_skipped_above_byte_cap() {
    let old = "x".repeat(diff_core::WORD_DIFF_MAX_COMBINED_BYTES);
    let new = "y".repeat(diff_core::WORD_DIFF_MAX_COMBINED_BYTES);
    let (old_spans, new_spans) = word_diff_spans(&old, &new);
    assert!(old_spans.is_empty());
    assert!(new_spans.is_empty());
  }

  #[test]
  fn word_diff_empty_for_identical() {
    let (old, new) = word_diff_spans("same line", "same line");
    assert!(old.iter().all(|s| s.kind == InlineSpanKind::Same));
    assert!(new.iter().all(|s| s.kind == InlineSpanKind::Same));
  }

  #[test]
  fn diff_summary_reveals_the_first_changed_line() {
    let summary = DiffSummary {
      path: "src/main.rs".to_string(),
      old_text: Some("a\nb\n".to_string()),
      new_text: "a\nB\n".to_string(),
      added: 1,
      removed: 1,
      lines: vec![
        DiffLine {
          kind: DiffLineKind::Context,
          old_line: Some(1),
          new_line: Some(1),
          text: "a".to_string(),
          spans: Vec::new(),
          syntax_spans: Vec::new(),
        },
        DiffLine {
          kind: DiffLineKind::Removed,
          old_line: Some(2),
          new_line: None,
          text: "b".to_string(),
          spans: Vec::new(),
          syntax_spans: Vec::new(),
        },
        DiffLine {
          kind: DiffLineKind::Added,
          old_line: None,
          new_line: Some(2),
          text: "B".to_string(),
          spans: Vec::new(),
          syntax_spans: Vec::new(),
        },
      ],
      expanded: false,
    };

    assert_eq!(summary.first_changed_line(), Some(2));
  }

  #[test]
  fn diff_line_snapshot_line_targets_the_visible_snapshot_position() {
    let added = DiffLine {
      kind: DiffLineKind::Added,
      old_line: None,
      new_line: Some(4),
      text: "added".to_string(),
      spans: Vec::new(),
      syntax_spans: Vec::new(),
    };
    let removed = DiffLine {
      kind: DiffLineKind::Removed,
      old_line: Some(7),
      new_line: None,
      text: "removed".to_string(),
      spans: Vec::new(),
      syntax_spans: Vec::new(),
    };
    let gap = DiffLine {
      kind: DiffLineKind::Gap,
      old_line: None,
      new_line: None,
      text: "...".to_string(),
      spans: Vec::new(),
      syntax_spans: Vec::new(),
    };

    assert_eq!(added.snapshot_line(), Some(4));
    assert_eq!(removed.snapshot_line(), Some(7));
    assert_eq!(gap.snapshot_line(), None);
  }

  #[test]
  fn build_diff_lines_pairs_and_carries_spans_and_line_numbers() {
    let lines = build_diff_lines(
      "export const VERSION = '3.0.0';\n",
      "export const VERSION = '2.0.0';\n",
    );
    let removed = lines
      .iter()
      .filter(|l| l.kind == DiffLineKind::Removed)
      .count();
    let added = lines
      .iter()
      .filter(|l| l.kind == DiffLineKind::Added)
      .count();
    assert_eq!(removed, 1);
    assert_eq!(added, 1);
    assert_eq!(lines[0].old_line, Some(1));
    assert_eq!(lines[0].new_line, None);
    assert_eq!(lines[1].old_line, None);
    assert_eq!(lines[1].new_line, Some(1));
    for l in &lines {
      assert!(!l.spans.is_empty(), "paired lines should have spans");
    }
  }

  #[test]
  fn build_diff_lines_tracks_line_numbers_inside_the_file() {
    let lines = build_diff_lines("keep\nold\nkeep\n", "keep\nnew\nkeep\nadded\n");
    let removed = lines
      .iter()
      .find(|line| line.kind == DiffLineKind::Removed && line.text == "old")
      .expect("removed line");
    let added = lines
      .iter()
      .find(|line| line.kind == DiffLineKind::Added && line.text == "new")
      .expect("added line");
    let trailing = lines
      .iter()
      .find(|line| line.kind == DiffLineKind::Added && line.text == "added")
      .expect("trailing added line");

    assert_eq!(removed.old_line, Some(2));
    assert_eq!(removed.new_line, None);
    assert_eq!(added.old_line, None);
    assert_eq!(added.new_line, Some(2));
    assert_eq!(trailing.old_line, None);
    assert_eq!(trailing.new_line, Some(4));
  }

  #[test]
  fn build_diff_lines_includes_editor_style_context() {
    let lines = build_diff_lines("a\nb\nc\nd\ne\n", "a\nB\nc\nd\nE\n");

    assert_eq!(
      lines.first().map(|line| line.kind.clone()),
      Some(DiffLineKind::Context)
    );
    assert!(
      lines
        .iter()
        .any(|line| line.kind == DiffLineKind::Context && line.text == "c")
    );
  }

  #[test]
  fn diff_line_counts_full_replacement() {
    let (added, removed) = diff_line_counts(Some("a\nb\nc\n"), "x\ny\nz\n");
    assert_eq!(added, 3);
    assert_eq!(removed, 3);
  }

  #[test]
  fn diff_line_counts_pure_addition() {
    let (added, removed) = diff_line_counts(None, "new line\n");
    assert_eq!(added, 1);
    assert_eq!(removed, 0);
  }

  #[test]
  fn diff_line_counts_identical_is_zero() {
    let (added, removed) = diff_line_counts(Some("same\nlines\n"), "same\nlines\n");
    assert_eq!(added, 0);
    assert_eq!(removed, 0);
  }

  #[test]
  fn extract_outputs_collects_text_content_blocks() {
    use agent_client_protocol::schema::{
      Content as AcpContent, ContentBlock, TextContent, ToolCallContent,
    };
    let content = vec![
      ToolCallContent::Content(AcpContent::new(ContentBlock::Text(TextContent::new(
        "hello stdout",
      )))),
      ToolCallContent::Content(AcpContent::new(ContentBlock::Text(TextContent::new(
        "second block",
      )))),
    ];
    let outs = extract_outputs(&content, None, false);
    assert_eq!(outs.len(), 2);
    assert_eq!(outs[0].text, "hello stdout");
    assert_eq!(outs[1].text, "second block");
    assert_eq!(outs[0].start_line, None);
    assert!(!outs[0].expanded);
  }

  #[test]
  fn extract_outputs_carries_read_start_line() {
    use agent_client_protocol::schema::{
      Content as AcpContent, ContentBlock, TextContent, ToolCallContent,
    };
    let content = vec![ToolCallContent::Content(AcpContent::new(
      ContentBlock::Text(TextContent::new("first\nsecond")),
    ))];

    let outs = extract_outputs(&content, Some(42), true);

    assert_eq!(outs.len(), 1);
    assert_eq!(outs[0].start_line, Some(42));
  }

  #[test]
  fn extract_outputs_strips_cat_numbered_read_text() {
    use agent_client_protocol::schema::{
      Content as AcpContent, ContentBlock, TextContent, ToolCallContent,
    };
    let content = vec![ToolCallContent::Content(AcpContent::new(
      ContentBlock::Text(TextContent::new("   845\tlet a = 1;\n   846\tlet b = 2;\n")),
    ))];

    let outs = extract_outputs(&content, Some(1), true);

    assert_eq!(outs.len(), 1);
    assert_eq!(outs[0].text, "let a = 1;\nlet b = 2;");
    assert_eq!(outs[0].start_line, Some(845));
  }

  #[test]
  fn extract_outputs_strips_cat_numbered_read_text_without_a_default_line() {
    use agent_client_protocol::schema::{
      Content as AcpContent, ContentBlock, TextContent, ToolCallContent,
    };
    let content = vec![ToolCallContent::Content(AcpContent::new(
      ContentBlock::Text(TextContent::new("   845\tlet a = 1;\n   846\tlet b = 2;")),
    ))];

    let outs = extract_outputs(&content, None, true);

    assert_eq!(outs.len(), 1);
    assert_eq!(outs[0].text, "let a = 1;\nlet b = 2;");
    assert_eq!(outs[0].start_line, Some(845));
  }

  #[test]
  fn extract_outputs_strips_fenced_cat_numbered_read_text() {
    use agent_client_protocol::schema::{
      Content as AcpContent, ContentBlock, TextContent, ToolCallContent,
    };
    let content = vec![ToolCallContent::Content(AcpContent::new(
      ContentBlock::Text(TextContent::new(
        "```rust\n   845\tlet a = 1;\n   846\tlet b = 2;\n```",
      )),
    ))];

    let outs = extract_outputs(&content, Some(1), true);

    assert_eq!(outs.len(), 1);
    assert_eq!(outs[0].text, "let a = 1;\nlet b = 2;");
    assert_eq!(outs[0].start_line, Some(845));
  }

  #[test]
  fn extract_outputs_keeps_non_contiguous_numbered_text_verbatim() {
    use agent_client_protocol::schema::{
      Content as AcpContent, ContentBlock, TextContent, ToolCallContent,
    };
    let content = vec![ToolCallContent::Content(AcpContent::new(
      ContentBlock::Text(TextContent::new("   845\tlet a = 1;\n   847\tlet b = 2;")),
    ))];

    let outs = extract_outputs(&content, Some(1), true);

    assert_eq!(outs.len(), 1);
    assert_eq!(outs[0].text, "   845\tlet a = 1;\n   847\tlet b = 2;");
    assert_eq!(outs[0].start_line, Some(1));
  }

  #[test]
  fn extract_outputs_only_strips_numbered_text_for_reads() {
    use agent_client_protocol::schema::{
      Content as AcpContent, ContentBlock, TextContent, ToolCallContent,
    };
    let content = vec![ToolCallContent::Content(AcpContent::new(
      ContentBlock::Text(TextContent::new("   1\tstdout line")),
    ))];

    let outs = extract_outputs(&content, None, false);

    assert_eq!(outs.len(), 1);
    assert_eq!(outs[0].text, "   1\tstdout line");
    assert_eq!(outs[0].start_line, None);
  }

  #[test]
  fn raw_output_string_fills_empty_tool_output() {
    let raw_output = serde_json::json!("hello stdout");

    let outs = extract_outputs_with_fallback(&[], Some(&raw_output), None, false);

    assert_eq!(outs.len(), 1);
    assert_eq!(outs[0].text, "hello stdout");
    assert_eq!(outs[0].start_line, None);
  }

  #[test]
  fn raw_output_envelope_unwraps_known_text_fields() {
    let raw_output = serde_json::json!({
      "stdout": "build ok",
      "stderr": "warning: slow",
    });

    let outs = extract_outputs_with_fallback(&[], Some(&raw_output), None, false);

    assert_eq!(outs.len(), 1);
    assert_eq!(outs[0].text, "build ok\nwarning: slow");
  }

  #[test]
  fn raw_output_envelope_unwraps_formatted_output() {
    let raw_output = serde_json::json!({
      "formatted_output": "# Reviu\n\nA keyboard-first desktop Git client."
    });

    let outs = extract_outputs_with_fallback(&[], Some(&raw_output), Some(1), true);

    assert_eq!(outs.len(), 1);
    assert_eq!(
      outs[0].text,
      "# Reviu\n\nA keyboard-first desktop Git client."
    );
    assert_eq!(outs[0].start_line, Some(1));
  }

  #[test]
  fn raw_output_content_blocks_unwrap_text_items() {
    let raw_output = serde_json::json!({
      "content": [
        { "type": "text", "text": "first" },
        { "type": "text", "text": "second" }
      ]
    });

    let outs = extract_outputs_with_fallback(&[], Some(&raw_output), None, false);

    assert_eq!(outs.len(), 1);
    assert_eq!(outs[0].text, "first\nsecond");
  }

  #[test]
  fn unknown_raw_output_objects_fall_back_to_pretty_json() {
    let raw_output = serde_json::json!({ "ok": true, "count": 2 });

    let outs = extract_outputs_with_fallback(&[], Some(&raw_output), None, false);

    assert_eq!(outs.len(), 1);
    assert_eq!(outs[0].text, "{\n  \"ok\": true,\n  \"count\": 2\n}");
  }

  #[test]
  fn content_blocks_win_over_raw_output_fallback() {
    use agent_client_protocol::schema::{
      Content as AcpContent, ContentBlock, TextContent, ToolCallContent,
    };
    let content = vec![ToolCallContent::Content(AcpContent::new(
      ContentBlock::Text(TextContent::new("content text")),
    ))];
    let raw_output = serde_json::json!("raw text");

    let outs = extract_outputs_with_fallback(&content, Some(&raw_output), None, false);

    assert_eq!(outs.len(), 1);
    assert_eq!(outs[0].text, "content text");
  }

  #[test]
  fn raw_output_fallback_strips_numbered_read_text() {
    let raw_output = serde_json::json!({
      "output": "   845\tlet a = 1;\n   846\tlet b = 2;"
    });

    let outs = extract_outputs_with_fallback(&[], Some(&raw_output), None, true);

    assert_eq!(outs.len(), 1);
    assert_eq!(outs[0].text, "let a = 1;\nlet b = 2;");
    assert_eq!(outs[0].start_line, Some(845));
  }

  #[test]
  fn extract_diffs_collects_per_file() {
    use agent_client_protocol::schema::Diff;
    let content = vec![
      ToolCallContent::Diff(Diff::new("foo.rs", "new\n")),
      ToolCallContent::Diff(Diff::new("bar.rs", "after\n").old_text(Some("before\n".to_string()))),
    ];
    let diffs = extract_diffs(&content, test_cwd());
    assert_eq!(diffs.len(), 2);
    assert_eq!(diffs[0].path, "foo.rs");
    assert_eq!(diffs[0].old_text, None);
    assert_eq!(diffs[0].new_text, "new\n");
    assert_eq!(diffs[0].added, 1);
    assert_eq!(diffs[1].old_text.as_deref(), Some("before\n"));
    assert_eq!(diffs[1].new_text, "after\n");
    assert_eq!(diffs[1].added, 1);
    assert_eq!(diffs[1].removed, 1);
  }
}
