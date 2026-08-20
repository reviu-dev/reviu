use agent_client_protocol::schema::{ContentBlock, ToolCallContent};
use syntax::HighlightSpan;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct DiffSummary {
  pub path: String,
  pub added: u32,
  pub removed: u32,
  #[serde(default)]
  pub lines: Vec<DiffLine>,
  #[serde(skip)]
  pub expanded: bool,
}

/// Visible diff lines per block when collapsed. Beyond this, an expander button is shown.
pub(crate) const MAX_DIFF_LINES_COLLAPSED: usize = 40;

/// Visible tool output lines per block when collapsed. Beyond this, an expander button is shown.
pub(crate) const MAX_TOOL_OUTPUT_LINES_COLLAPSED: usize = 20;

/// Extracts text content blocks from tool call output. Returns one entry per `Content` block.
pub(crate) fn extract_outputs(content: &[ToolCallContent]) -> Vec<crate::ToolOutput> {
  content
    .iter()
    .filter_map(|c| match c {
      ToolCallContent::Content(text_content) => match &text_content.content {
        ContentBlock::Text(t) => Some(crate::ToolOutput {
          text: crate::strip_markdown_code_fence(&t.text).to_string(),
          expanded: false,
          syntax_spans: Vec::new(),
        }),
        _ => None,
      },
      _ => None,
    })
    .collect()
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

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub(crate) enum DiffLineKind {
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

pub(crate) const WORD_DIFF_MAX_COMBINED_BYTES: usize = 2_048;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IdentifierCharKind {
  Lower,
  Upper,
  Digit,
  Underscore,
  Other,
}

fn identifier_char_kind(ch: char) -> IdentifierCharKind {
  if ch == '_' {
    IdentifierCharKind::Underscore
  } else if ch.is_lowercase() {
    IdentifierCharKind::Lower
  } else if ch.is_uppercase() {
    IdentifierCharKind::Upper
  } else if ch.is_numeric() {
    IdentifierCharKind::Digit
  } else {
    IdentifierCharKind::Other
  }
}

fn split_identifier_token_ranges(segment: &str) -> Vec<std::ops::Range<usize>> {
  let chars: Vec<_> = segment.char_indices().collect();
  if chars.is_empty() {
    return Vec::new();
  }
  let mut ranges = Vec::new();
  let mut start = 0usize;
  for idx in 1..chars.len() {
    let (byte_offset, current) = chars[idx];
    let (_, previous) = chars[idx - 1];
    let previous_kind = identifier_char_kind(previous);
    let current_kind = identifier_char_kind(current);
    let next_kind = chars
      .get(idx + 1)
      .map(|(_, next)| identifier_char_kind(*next));
    let should_split = match (previous_kind, current_kind) {
      (IdentifierCharKind::Underscore, _) | (_, IdentifierCharKind::Underscore) => true,
      (IdentifierCharKind::Digit, IdentifierCharKind::Digit) => false,
      (IdentifierCharKind::Digit, _) | (_, IdentifierCharKind::Digit) => true,
      (IdentifierCharKind::Lower, IdentifierCharKind::Upper) => true,
      (IdentifierCharKind::Upper, IdentifierCharKind::Upper) => {
        next_kind == Some(IdentifierCharKind::Lower)
      }
      _ => false,
    };
    if should_split {
      ranges.push(start..byte_offset);
      start = byte_offset;
    }
  }
  ranges.push(start..segment.len());
  ranges
}

fn word_tokens(text: &str, include_whitespace: bool) -> Vec<&str> {
  use unicode_segmentation::UnicodeSegmentation;
  let mut tokens = Vec::new();
  for (_idx, segment) in text.split_word_bound_indices() {
    if !include_whitespace && segment.trim().is_empty() {
      continue;
    }
    let subranges = split_identifier_token_ranges(segment);
    if subranges.is_empty() {
      tokens.push(segment);
      continue;
    }
    for subrange in subranges {
      tokens.push(&segment[subrange]);
    }
  }
  tokens
}

pub(crate) fn word_diff_spans(old: &str, new: &str) -> (Vec<InlineSpan>, Vec<InlineSpan>) {
  if old == new {
    let same_old = vec![InlineSpan {
      kind: InlineSpanKind::Same,
      text: old.to_string(),
    }];
    let same_new = vec![InlineSpan {
      kind: InlineSpanKind::Same,
      text: new.to_string(),
    }];
    return (same_old, same_new);
  }
  if old.len().saturating_add(new.len()) > WORD_DIFF_MAX_COMBINED_BYTES {
    return (Vec::new(), Vec::new());
  }
  word_diff_spans_impl(old, new)
}

fn word_diff_spans_impl(old: &str, new: &str) -> (Vec<InlineSpan>, Vec<InlineSpan>) {
  let a = word_tokens(old, true);
  let b = word_tokens(new, true);
  let n = a.len();
  let m = b.len();
  let mut dp = vec![vec![0u32; m + 1]; n + 1];
  for i in 0..n {
    for j in 0..m {
      dp[i + 1][j + 1] = if a[i] == b[j] {
        dp[i][j] + 1
      } else {
        dp[i + 1][j].max(dp[i][j + 1])
      };
    }
  }
  let mut old_spans: Vec<InlineSpan> = Vec::new();
  let mut new_spans: Vec<InlineSpan> = Vec::new();
  let mut i = n;
  let mut j = m;
  while i > 0 || j > 0 {
    if i > 0 && j > 0 && a[i - 1] == b[j - 1] {
      old_spans.push(InlineSpan {
        kind: InlineSpanKind::Same,
        text: a[i - 1].to_string(),
      });
      new_spans.push(InlineSpan {
        kind: InlineSpanKind::Same,
        text: b[j - 1].to_string(),
      });
      i -= 1;
      j -= 1;
    } else if j > 0 && (i == 0 || dp[i][j - 1] >= dp[i - 1][j]) {
      new_spans.push(InlineSpan {
        kind: InlineSpanKind::Diff,
        text: b[j - 1].to_string(),
      });
      j -= 1;
    } else {
      old_spans.push(InlineSpan {
        kind: InlineSpanKind::Diff,
        text: a[i - 1].to_string(),
      });
      i -= 1;
    }
  }
  old_spans.reverse();
  new_spans.reverse();
  (
    merge_adjacent_spans(old_spans),
    merge_adjacent_spans(new_spans),
  )
}

fn merge_adjacent_spans(spans: Vec<InlineSpan>) -> Vec<InlineSpan> {
  let mut out: Vec<InlineSpan> = Vec::new();
  for s in spans {
    if let Some(last) = out.last_mut()
      && last.kind == s.kind
    {
      last.text.push_str(&s.text);
      continue;
    }
    out.push(s);
  }
  out
}

pub(crate) fn build_diff_lines(old: &str, new: &str) -> Vec<DiffLine> {
  use imara_diff::{Algorithm, Diff, InternedInput};
  let input = InternedInput::new(old, new);
  let diff = Diff::compute(Algorithm::Histogram, &input);
  let old_lines: Vec<&str> = old.lines().collect();
  let new_lines: Vec<&str> = new.lines().collect();
  let mut out = Vec::new();
  for hunk in diff.hunks() {
    let removed: Vec<&str> = hunk
      .before
      .clone()
      .filter_map(|i| old_lines.get(i as usize).copied())
      .collect();
    let added: Vec<&str> = hunk
      .after
      .clone()
      .filter_map(|i| new_lines.get(i as usize).copied())
      .collect();
    let paired = removed.len().min(added.len());
    for k in 0..paired {
      let (old_spans, new_spans) = word_diff_spans(removed[k], added[k]);
      out.push(DiffLine {
        kind: DiffLineKind::Removed,
        old_line: Some(hunk.before.start + k as u32 + 1),
        new_line: None,
        text: removed[k].to_string(),
        spans: old_spans,
        syntax_spans: Vec::new(),
      });
      out.push(DiffLine {
        kind: DiffLineKind::Added,
        old_line: None,
        new_line: Some(hunk.after.start + k as u32 + 1),
        text: added[k].to_string(),
        spans: new_spans,
        syntax_spans: Vec::new(),
      });
    }
    for (offset, line) in removed.iter().enumerate().skip(paired) {
      out.push(DiffLine {
        kind: DiffLineKind::Removed,
        old_line: Some(hunk.before.start + offset as u32 + 1),
        new_line: None,
        text: (*line).to_string(),
        spans: Vec::new(),
        syntax_spans: Vec::new(),
      });
    }
    for (offset, line) in added.iter().enumerate().skip(paired) {
      out.push(DiffLine {
        kind: DiffLineKind::Added,
        old_line: None,
        new_line: Some(hunk.after.start + offset as u32 + 1),
        text: (*line).to_string(),
        spans: Vec::new(),
        syntax_spans: Vec::new(),
      });
    }
  }
  out
}

pub(crate) fn backfill_legacy_line_numbers(summary: &mut DiffSummary, start_line: Option<u32>) {
  if summary
    .lines
    .iter()
    .any(|line| line.old_line.is_some() || line.new_line.is_some())
  {
    return;
  }
  let Some(start_line) = start_line else {
    return;
  };

  let mut old_line = start_line;
  let mut new_line = start_line;
  for line in &mut summary.lines {
    match line.kind {
      DiffLineKind::Removed => {
        line.old_line = Some(old_line);
        old_line += 1;
      }
      DiffLineKind::Added => {
        line.new_line = Some(new_line);
        new_line += 1;
      }
    }
  }
}

pub(crate) fn diff_line_counts(old_text: Option<&str>, new_text: &str) -> (u32, u32) {
  use imara_diff::{Algorithm, Diff, InternedInput};
  let old = old_text.unwrap_or("");
  let input = InternedInput::new(old, new_text);
  let diff = Diff::compute(Algorithm::Histogram, &input);
  let mut added = 0u32;
  let mut removed = 0u32;
  for hunk in diff.hunks() {
    added += hunk.after.end - hunk.after.start;
    removed += hunk.before.end - hunk.before.start;
  }
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
    let old = "x".repeat(WORD_DIFF_MAX_COMBINED_BYTES);
    let new = "y".repeat(WORD_DIFF_MAX_COMBINED_BYTES);
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
  fn backfill_legacy_line_numbers_uses_the_tool_location_as_a_start() {
    let mut summary = DiffSummary {
      path: "src/main.rs".to_string(),
      added: 2,
      removed: 1,
      lines: vec![
        DiffLine {
          kind: DiffLineKind::Removed,
          old_line: None,
          new_line: None,
          text: "old".to_string(),
          spans: Vec::new(),
          syntax_spans: Vec::new(),
        },
        DiffLine {
          kind: DiffLineKind::Added,
          old_line: None,
          new_line: None,
          text: "new".to_string(),
          spans: Vec::new(),
          syntax_spans: Vec::new(),
        },
        DiffLine {
          kind: DiffLineKind::Added,
          old_line: None,
          new_line: None,
          text: "newer".to_string(),
          spans: Vec::new(),
          syntax_spans: Vec::new(),
        },
      ],
      expanded: false,
    };

    backfill_legacy_line_numbers(&mut summary, Some(12));

    assert_eq!(summary.lines[0].old_line, Some(12));
    assert_eq!(summary.lines[0].new_line, None);
    assert_eq!(summary.lines[1].old_line, None);
    assert_eq!(summary.lines[1].new_line, Some(12));
    assert_eq!(summary.lines[2].old_line, None);
    assert_eq!(summary.lines[2].new_line, Some(13));
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
    let outs = extract_outputs(&content);
    assert_eq!(outs.len(), 2);
    assert_eq!(outs[0].text, "hello stdout");
    assert_eq!(outs[1].text, "second block");
    assert!(!outs[0].expanded);
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
    assert_eq!(diffs[0].added, 1);
    assert_eq!(diffs[1].added, 1);
    assert_eq!(diffs[1].removed, 1);
  }
}
