use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

pub const WORD_DIFF_MAX_COMBINED_BYTES: usize = 2_048;
pub const DEFAULT_CONTEXT_LINES: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffRowKind {
  Context,
  Gap,
  Added,
  Removed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffRow {
  pub kind: DiffRowKind,
  pub old_line: Option<u32>,
  pub new_line: Option<u32>,
  pub text: String,
  pub word_diff_ranges: Vec<Range<usize>>,
  pub no_newline: bool,
}

#[derive(Clone, Debug)]
struct WordToken {
  text: String,
  range: Range<usize>,
}

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

fn split_identifier_token_ranges(segment: &str) -> Vec<Range<usize>> {
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

fn word_tokens(text: &str, include_whitespace: bool) -> Vec<WordToken> {
  let mut tokens = Vec::new();
  for (idx, segment) in text.split_word_bound_indices() {
    if !include_whitespace && segment.trim().is_empty() {
      continue;
    }
    let subranges = split_identifier_token_ranges(segment);
    if subranges.is_empty() {
      tokens.push(WordToken {
        text: segment.to_string(),
        range: idx..idx + segment.len(),
      });
      continue;
    }

    for subrange in subranges {
      tokens.push(WordToken {
        text: segment[subrange.clone()].to_string(),
        range: idx + subrange.start..idx + subrange.end,
      });
    }
  }
  tokens
}

pub fn merge_ranges(mut ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
  ranges.sort_by_key(|range| range.start);
  let mut merged: Vec<Range<usize>> = Vec::new();
  for range in ranges {
    if let Some(last) = merged.last_mut()
      && range.start <= last.end
    {
      last.end = last.end.max(range.end);
      continue;
    }
    merged.push(range);
  }
  merged
}

pub fn word_diff_ranges(old_text: &str, new_text: &str) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
  if old_text == new_text {
    return (Vec::new(), Vec::new());
  }

  if old_text.len().saturating_add(new_text.len()) > WORD_DIFF_MAX_COMBINED_BYTES {
    return (Vec::new(), Vec::new());
  }

  let (removed, added) = word_diff_ranges_impl(old_text, new_text, false);
  if removed.is_empty() && added.is_empty() && old_text != new_text {
    return word_diff_ranges_impl(old_text, new_text, true);
  }
  (removed, added)
}

fn word_diff_ranges_impl(
  old_text: &str,
  new_text: &str,
  include_whitespace: bool,
) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
  let old_tokens = word_tokens(old_text, include_whitespace);
  let new_tokens = word_tokens(new_text, include_whitespace);

  let old_len = old_tokens.len();
  let new_len = new_tokens.len();
  if old_len == 0 && new_len == 0 {
    return (Vec::new(), Vec::new());
  }

  let mut dp = vec![vec![0usize; new_len + 1]; old_len + 1];
  for i in 0..old_len {
    for j in 0..new_len {
      if old_tokens[i].text == new_tokens[j].text {
        dp[i + 1][j + 1] = dp[i][j] + 1;
      } else {
        dp[i + 1][j + 1] = dp[i][j + 1].max(dp[i + 1][j]);
      }
    }
  }

  let mut matched_old = vec![false; old_len];
  let mut matched_new = vec![false; new_len];
  let mut i = old_len;
  let mut j = new_len;
  while i > 0 && j > 0 {
    if old_tokens[i - 1].text == new_tokens[j - 1].text {
      matched_old[i - 1] = true;
      matched_new[j - 1] = true;
      i -= 1;
      j -= 1;
    } else if dp[i - 1][j] >= dp[i][j - 1] {
      i -= 1;
    } else {
      j -= 1;
    }
  }

  let removed = old_tokens
    .iter()
    .enumerate()
    .filter_map(|(idx, token)| (!matched_old[idx]).then_some(token.range.clone()))
    .collect();
  let added = new_tokens
    .iter()
    .enumerate()
    .filter_map(|(idx, token)| (!matched_new[idx]).then_some(token.range.clone()))
    .collect();

  (merge_ranges(removed), merge_ranges(added))
}

fn strip_line_ending(line: &str) -> &str {
  let without_lf = line.strip_suffix('\n').unwrap_or(line);
  without_lf.strip_suffix('\r').unwrap_or(without_lf)
}

fn line_has_no_newline(line: &str) -> bool {
  !line.ends_with('\n')
}

pub fn split_lines_preserving_newline(text: &str) -> Vec<&str> {
  let mut lines = Vec::new();
  let mut rest = text;
  while !rest.is_empty() {
    match rest.find('\n') {
      Some(pos) => {
        lines.push(&rest[..=pos]);
        rest = &rest[pos + 1..];
      }
      None => {
        lines.push(rest);
        break;
      }
    }
  }
  lines
}

pub fn line_hunks(old: &str, new: &str) -> Vec<LineHunk> {
  use imara_diff::{Algorithm, Diff, InternedInput};

  let input = InternedInput::new(old, new);
  let mut diff = Diff::compute(Algorithm::Histogram, &input);
  diff.postprocess_lines(&input);
  diff
    .hunks()
    .map(|hunk| LineHunk {
      old: hunk.before,
      new: hunk.after,
    })
    .collect()
}

pub fn line_diff_counts(old: &str, new: &str) -> (u32, u32) {
  let mut added = 0u32;
  let mut removed = 0u32;
  for hunk in line_hunks(old, new) {
    added += hunk.new.end - hunk.new.start;
    removed += hunk.old.end - hunk.old.start;
  }
  (added, removed)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineHunk {
  pub old: Range<u32>,
  pub new: Range<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineHunkGroup {
  pub old_context_start: usize,
  pub new_context_start: usize,
  pub old_context_end: usize,
  pub new_context_end: usize,
  pub hunks: Vec<LineHunk>,
}

impl LineHunkGroup {
  fn new(
    before: Range<u32>,
    after: Range<u32>,
    old_len: usize,
    new_len: usize,
    context_lines: usize,
  ) -> Self {
    Self {
      old_context_start: (before.start as usize).saturating_sub(context_lines),
      new_context_start: (after.start as usize).saturating_sub(context_lines),
      old_context_end: (before.end as usize)
        .saturating_add(context_lines)
        .min(old_len),
      new_context_end: (after.end as usize)
        .saturating_add(context_lines)
        .min(new_len),
      hunks: vec![LineHunk {
        old: before,
        new: after,
      }],
    }
  }

  fn overlaps(&self, before: &Range<u32>, after: &Range<u32>, context_lines: usize) -> bool {
    (before.start as usize).saturating_sub(context_lines) <= self.old_context_end
      || (after.start as usize).saturating_sub(context_lines) <= self.new_context_end
  }

  fn push(
    &mut self,
    before: Range<u32>,
    after: Range<u32>,
    old_len: usize,
    new_len: usize,
    context_lines: usize,
  ) {
    self.old_context_end = (before.end as usize)
      .saturating_add(context_lines)
      .min(old_len);
    self.new_context_end = (after.end as usize)
      .saturating_add(context_lines)
      .min(new_len);
    self.hunks.push(LineHunk {
      old: before,
      new: after,
    });
  }
}

pub fn diff_rows(old: &str, new: &str) -> Vec<DiffRow> {
  diff_rows_with_context(old, new, DEFAULT_CONTEXT_LINES)
}

pub fn line_hunk_groups_with_context(
  old: &str,
  new: &str,
  context_lines: usize,
) -> Vec<LineHunkGroup> {
  let hunks = line_hunks(old, new);
  if hunks.is_empty() {
    return Vec::new();
  }

  let old_line_count = split_lines_preserving_newline(old).len();
  let new_line_count = split_lines_preserving_newline(new).len();
  let mut groups: Vec<LineHunkGroup> = Vec::new();

  for hunk in hunks {
    let before = hunk.old;
    let after = hunk.new;
    if let Some(group) = groups.last_mut()
      && group.overlaps(&before, &after, context_lines)
    {
      group.push(before, after, old_line_count, new_line_count, context_lines);
      continue;
    }
    groups.push(LineHunkGroup::new(
      before,
      after,
      old_line_count,
      new_line_count,
      context_lines,
    ));
  }

  groups
}

pub fn diff_rows_with_context(old: &str, new: &str, context_lines: usize) -> Vec<DiffRow> {
  let old_lines = split_lines_preserving_newline(old);
  let new_lines = split_lines_preserving_newline(new);
  let mut rows = Vec::new();
  for group in line_hunk_groups_with_context(old, new, context_lines) {
    if !rows.is_empty() {
      rows.push(DiffRow {
        kind: DiffRowKind::Gap,
        old_line: None,
        new_line: None,
        text: "...".to_string(),
        word_diff_ranges: Vec::new(),
        no_newline: false,
      });
    }
    push_group_rows(&group, &old_lines, &new_lines, &mut rows);
  }
  rows
}

fn push_context_rows(
  old_cursor: &mut usize,
  new_cursor: &mut usize,
  old_end: usize,
  new_end: usize,
  old_lines: &[&str],
  rows: &mut Vec<DiffRow>,
) {
  while *old_cursor < old_end && *new_cursor < new_end {
    let old_line = *old_cursor + 1;
    let new_line = *new_cursor + 1;
    rows.push(DiffRow {
      kind: DiffRowKind::Context,
      old_line: Some(old_line as u32),
      new_line: Some(new_line as u32),
      text: strip_line_ending(old_lines.get(*old_cursor).copied().unwrap_or_default()).to_string(),
      word_diff_ranges: Vec::new(),
      no_newline: old_lines
        .get(*old_cursor)
        .is_some_and(|line| line_has_no_newline(line)),
    });
    *old_cursor += 1;
    *new_cursor += 1;
  }
}

fn push_group_rows(
  group: &LineHunkGroup,
  old_lines: &[&str],
  new_lines: &[&str],
  rows: &mut Vec<DiffRow>,
) {
  let mut old_cursor = group.old_context_start;
  let mut new_cursor = group.new_context_start;

  for hunk in &group.hunks {
    push_context_rows(
      &mut old_cursor,
      &mut new_cursor,
      hunk.old.start as usize,
      hunk.new.start as usize,
      old_lines,
      rows,
    );

    let removed: Vec<(&str, bool)> = hunk
      .old
      .clone()
      .filter_map(|idx| {
        old_lines
          .get(idx as usize)
          .copied()
          .map(|line| (strip_line_ending(line), line_has_no_newline(line)))
      })
      .collect();
    let added: Vec<(&str, bool)> = hunk
      .new
      .clone()
      .filter_map(|idx| {
        new_lines
          .get(idx as usize)
          .copied()
          .map(|line| (strip_line_ending(line), line_has_no_newline(line)))
      })
      .collect();
    let paired = removed.len().min(added.len());
    let word_pairs: Vec<_> = (0..paired)
      .map(|idx| word_diff_ranges(removed[idx].0, added[idx].0))
      .collect();

    for (offset, (line, no_newline)) in removed.iter().enumerate() {
      rows.push(DiffRow {
        kind: DiffRowKind::Removed,
        old_line: Some(hunk.old.start + offset as u32 + 1),
        new_line: None,
        text: (*line).to_string(),
        word_diff_ranges: word_pairs
          .get(offset)
          .map(|(removed, _)| removed.clone())
          .unwrap_or_default(),
        no_newline: *no_newline,
      });
    }

    for (offset, (line, no_newline)) in added.iter().enumerate() {
      rows.push(DiffRow {
        kind: DiffRowKind::Added,
        old_line: None,
        new_line: Some(hunk.new.start + offset as u32 + 1),
        text: (*line).to_string(),
        word_diff_ranges: word_pairs
          .get(offset)
          .map(|(_, added)| added.clone())
          .unwrap_or_default(),
        no_newline: *no_newline,
      });
    }

    old_cursor = hunk.old.end as usize;
    new_cursor = hunk.new.end as usize;
  }

  push_context_rows(
    &mut old_cursor,
    &mut new_cursor,
    group.old_context_end,
    group.new_context_end,
    old_lines,
    rows,
  );
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn split_lines_preserving_newline_matches_diff_line_tokens() {
    assert_eq!(split_lines_preserving_newline(""), Vec::<&str>::new());
    assert_eq!(split_lines_preserving_newline("a"), vec!["a"]);
    assert_eq!(split_lines_preserving_newline("a\n"), vec!["a\n"]);
    assert_eq!(split_lines_preserving_newline("a\n\n"), vec!["a\n", "\n"]);
  }

  #[test]
  fn word_tokens_split_identifier_subwords() {
    let tokens = word_tokens("getLastDataNotification_v2", false)
      .into_iter()
      .map(|token| token.text)
      .collect::<Vec<_>>();

    assert_eq!(
      tokens,
      vec!["get", "Last", "Data", "Notification", "_", "v", "2"]
    );
  }

  #[test]
  fn word_tokens_preserve_acronym_boundaries() {
    let tokens = word_tokens("getHTTPServerResponse", false)
      .into_iter()
      .map(|token| token.text)
      .collect::<Vec<_>>();

    assert_eq!(tokens, vec!["get", "HTTP", "Server", "Response"]);
  }

  #[test]
  fn word_diff_highlights_changed_identifier_segment() {
    let old_text = "const getLastNotification = value;";
    let new_text = "const getLastDataNotification = value;";

    let (removed, added) = word_diff_ranges(old_text, new_text);

    assert!(removed.is_empty());
    assert_eq!(&new_text[added[0].clone()], "Data");
  }

  #[test]
  fn word_diff_falls_back_to_whitespace() {
    let (removed, added) = word_diff_ranges("  let x = 1;", "    let x = 1;");

    assert!(!removed.is_empty() || !added.is_empty());
  }

  #[test]
  fn line_counts_ignore_context() {
    assert_eq!(line_diff_counts("same\nold\n", "same\nnew\n"), (1, 1));
  }

  #[test]
  fn diff_rows_mark_lines_without_trailing_newlines() {
    let rows = diff_rows_with_context("same\nold", "same\nnew\n", 1);
    let removed = rows
      .iter()
      .find(|row| row.kind == DiffRowKind::Removed)
      .expect("removed row");
    let added = rows
      .iter()
      .find(|row| row.kind == DiffRowKind::Added)
      .expect("added row");

    assert!(removed.no_newline);
    assert!(!added.no_newline);
  }

  #[test]
  fn diff_rows_include_context_and_line_numbers() {
    let rows = diff_rows_with_context("a\nb\nc\nd\ne\n", "a\nB\nc\nd\nE\n", 1);

    assert_eq!(rows[0].kind, DiffRowKind::Context);
    assert_eq!(rows[0].old_line, Some(1));
    assert_eq!(rows[0].new_line, Some(1));
    assert!(
      rows
        .iter()
        .any(|row| row.kind == DiffRowKind::Removed && row.text == "b")
    );
    assert!(
      rows
        .iter()
        .any(|row| row.kind == DiffRowKind::Added && row.text == "B")
    );
    assert!(
      rows
        .iter()
        .any(|row| row.kind == DiffRowKind::Context && row.text == "c")
    );
  }

  #[test]
  fn separated_hunks_get_gap_rows() {
    let rows = diff_rows_with_context("a\nb\nc\nd\ne\n", "A\nb\nc\nd\nE\n", 0);

    assert!(rows.iter().any(|row| row.kind == DiffRowKind::Gap));
  }

  #[test]
  fn nearby_hunks_are_merged_by_context() {
    let rows = diff_rows_with_context("a\nb\nc\nd\ne\n", "A\nb\nc\nD\ne\n", 2);
    let context_c_count = rows
      .iter()
      .filter(|row| row.kind == DiffRowKind::Context && row.text == "c")
      .count();

    assert_eq!(context_c_count, 1);
  }
}
