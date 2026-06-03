use std::collections::HashSet;
use std::ops::Range;

const MAX_QUERY_LEN: usize = 128;
const MAX_FILE_RESULTS: usize = 8;
const MAX_DIFF_LINES: usize = 1500;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum DiffMention {
  Working,
  Staged,
  Branch,
}

impl DiffMention {
  pub(crate) fn keyword(self) -> &'static str {
    match self {
      DiffMention::Working => "diff",
      DiffMention::Staged => "staged",
      DiffMention::Branch => "branch",
    }
  }

  pub(crate) fn description(self) -> &'static str {
    match self {
      DiffMention::Working => "Uncommitted changes",
      DiffMention::Staged => "Staged changes",
      DiffMention::Branch => "Changes vs base branch",
    }
  }

  fn all() -> [DiffMention; 3] {
    [
      DiffMention::Working,
      DiffMention::Staged,
      DiffMention::Branch,
    ]
  }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MentionCandidate {
  Diff(DiffMention),
  File(String),
}

impl MentionCandidate {
  /// Text inserted into the input when the candidate is picked (trailing space included).
  pub(crate) fn token(&self) -> String {
    match self {
      MentionCandidate::Diff(d) => format!("@{} ", d.keyword()),
      MentionCandidate::File(path) => format!("@{path} "),
    }
  }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MentionTrigger {
  /// Byte range of `@query` (including the `@`) in the source text.
  pub range: Range<usize>,
  pub query: String,
}

pub(crate) fn mention_trigger_at_cursor(text: &str, cursor: usize) -> Option<MentionTrigger> {
  if cursor > text.len() || !text.is_char_boundary(cursor) {
    return None;
  }

  let before_cursor = &text[..cursor];
  for (ix, ch) in before_cursor.char_indices().rev() {
    if ch == '@' {
      let query = &text[ix + ch.len_utf8()..cursor];
      if query.len() > MAX_QUERY_LEN || !query.chars().all(is_query_char) {
        return None;
      }

      let previous = before_cursor[..ix].chars().next_back();
      if !is_boundary(previous) {
        return None;
      }

      return Some(MentionTrigger {
        range: ix..cursor,
        query: query.to_string(),
      });
    }

    if !is_query_char(ch) {
      return None;
    }
  }

  None
}

fn is_query_char(ch: char) -> bool {
  ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/')
}

fn is_boundary(ch: Option<char>) -> bool {
  ch.is_none_or(|ch| !is_query_char(ch))
}

pub(crate) fn matching_mentions(
  query: &str,
  files: &[String],
  max: usize,
) -> Vec<MentionCandidate> {
  let query = query.to_ascii_lowercase();
  let mut out = Vec::new();

  for diff in DiffMention::all() {
    if query.is_empty() || diff.keyword().starts_with(query.as_str()) {
      out.push(MentionCandidate::Diff(diff));
    }
  }

  let mut scored = files
    .iter()
    .filter_map(|path| file_score(path, query.as_str()).map(|score| (score, path)))
    .collect::<Vec<_>>();
  scored.sort_by(|(left_score, left), (right_score, right)| {
    left_score
      .cmp(right_score)
      .then_with(|| left.len().cmp(&right.len()))
      .then_with(|| left.as_str().cmp(right.as_str()))
  });
  for (_, path) in scored.into_iter().take(MAX_FILE_RESULTS) {
    out.push(MentionCandidate::File(path.clone()));
  }

  out.truncate(max);
  out
}

fn file_score(path: &str, query: &str) -> Option<u8> {
  if query.is_empty() {
    return Some(2);
  }

  let lower = path.to_ascii_lowercase();
  let file_name = lower.rsplit('/').next().unwrap_or(lower.as_str());
  if file_name == query {
    Some(0)
  } else if file_name.starts_with(query) {
    Some(1)
  } else if file_name.contains(query) {
    Some(2)
  } else if lower.contains(query) {
    Some(3)
  } else {
    None
  }
}

/// A mention resolved against the known repo file list, ready to become a content block.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ResolvedMention {
  Diff(DiffMention),
  File(String),
}

fn diff_from_keyword(word: &str) -> Option<DiffMention> {
  match word {
    "diff" => Some(DiffMention::Working),
    "staged" => Some(DiffMention::Staged),
    "branch" => Some(DiffMention::Branch),
    _ => None,
  }
}

fn token_after_at(text: &str, at_ix: usize) -> Option<&str> {
  if !is_boundary(text[..at_ix].chars().next_back()) {
    return None;
  }
  let rest = &text[at_ix + 1..];
  let end = rest.find(|c: char| !is_query_char(c)).unwrap_or(rest.len());
  if end == 0 {
    return None;
  }
  Some(&rest[..end])
}

/// Diff and file mentions in submitted text, in order, de-duplicated. File tokens count only when
/// they match a known repo path, so prose like `@everyone` is ignored.
pub(crate) fn resolve_mentions(text: &str, files: &[String]) -> Vec<ResolvedMention> {
  let known = files.iter().map(String::as_str).collect::<HashSet<_>>();
  let mut out = Vec::new();
  let mut seen = HashSet::new();
  for (ix, _) in text.match_indices('@') {
    let Some(token) = token_after_at(text, ix) else {
      continue;
    };
    let mention = if let Some(diff) = diff_from_keyword(token) {
      ResolvedMention::Diff(diff)
    } else if known.contains(token) {
      ResolvedMention::File(token.to_string())
    } else {
      continue;
    };
    if seen.insert(mention.clone()) {
      out.push(mention);
    }
  }
  out
}

/// Trim and line-cap a raw diff for embedding as resource content.
pub(crate) fn truncate_diff(diff: &str) -> String {
  let diff = diff.trim_end();
  let lines = diff.lines().collect::<Vec<_>>();
  if lines.len() > MAX_DIFF_LINES {
    format!(
      "{}\n... (truncated, {} lines total)",
      lines[..MAX_DIFF_LINES].join("\n"),
      lines.len()
    )
  } else {
    diff.to_string()
  }
}

pub(crate) fn byte_range_to_utf16_range(text: &str, range: Range<usize>) -> Range<usize> {
  byte_offset_to_utf16_offset(text, range.start)..byte_offset_to_utf16_offset(text, range.end)
}

fn byte_offset_to_utf16_offset(text: &str, offset: usize) -> usize {
  text[..offset].chars().map(char::len_utf16).sum()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn trigger_detects_query_after_at() {
    let text = "review @dif";
    let trigger = mention_trigger_at_cursor(text, text.len()).expect("trigger");
    assert_eq!(trigger.range, 7..11);
    assert_eq!(trigger.query, "dif");
  }

  #[test]
  fn trigger_allows_path_characters() {
    let text = "explain @src/foo/bar.rs";
    let trigger = mention_trigger_at_cursor(text, text.len()).expect("trigger");
    assert_eq!(trigger.query, "src/foo/bar.rs");
  }

  #[test]
  fn trigger_requires_boundary_before_at() {
    assert!(mention_trigger_at_cursor("user@host", "user@host".len()).is_none());
  }

  #[test]
  fn trigger_at_start_of_line() {
    let trigger = mention_trigger_at_cursor("@diff", 5).expect("trigger");
    assert_eq!(trigger.range, 0..5);
    assert_eq!(trigger.query, "diff");
  }

  #[test]
  fn matching_surfaces_diff_keywords_first() {
    let candidates = matching_mentions("d", &[], 8);
    assert_eq!(
      candidates,
      vec![MentionCandidate::Diff(DiffMention::Working)]
    );
  }

  #[test]
  fn matching_empty_query_lists_all_diff_kinds() {
    let candidates = matching_mentions("", &[], 8);
    assert_eq!(
      candidates,
      vec![
        MentionCandidate::Diff(DiffMention::Working),
        MentionCandidate::Diff(DiffMention::Staged),
        MentionCandidate::Diff(DiffMention::Branch),
      ]
    );
  }

  #[test]
  fn matching_ranks_files_by_filename_match() {
    let files = vec![
      "src/lib.rs".to_string(),
      "src/mention.rs".to_string(),
      "docs/mention_notes.md".to_string(),
    ];
    let candidates = matching_mentions("mention", &files, 8);
    assert_eq!(
      candidates,
      vec![
        MentionCandidate::File("src/mention.rs".to_string()),
        MentionCandidate::File("docs/mention_notes.md".to_string()),
      ]
    );
  }

  #[test]
  fn candidate_tokens_are_insertable() {
    assert_eq!(
      MentionCandidate::Diff(DiffMention::Staged).token(),
      "@staged "
    );
    assert_eq!(
      MentionCandidate::File("src/a.rs".to_string()).token(),
      "@src/a.rs "
    );
  }

  #[test]
  fn resolve_mentions_extracts_diff_keywords() {
    assert_eq!(
      resolve_mentions("please review @diff and @branch", &[]),
      vec![
        ResolvedMention::Diff(DiffMention::Working),
        ResolvedMention::Diff(DiffMention::Branch),
      ]
    );
  }

  #[test]
  fn resolve_mentions_ignores_partial_words_and_unknown_files() {
    assert_eq!(
      resolve_mentions("@diff @diffx @src/diff.rs", &[]),
      vec![ResolvedMention::Diff(DiffMention::Working)]
    );
  }

  #[test]
  fn resolve_mentions_requires_boundary() {
    assert!(resolve_mentions("email@diff", &[]).is_empty());
  }

  #[test]
  fn resolve_mentions_classifies_diffs_and_known_files() {
    let files = vec!["src/lib.rs".to_string()];
    let mentions = resolve_mentions("review @diff and @src/lib.rs and @everyone", &files);
    assert_eq!(
      mentions,
      vec![
        ResolvedMention::Diff(DiffMention::Working),
        ResolvedMention::File("src/lib.rs".to_string()),
      ]
    );
  }

  #[test]
  fn resolve_mentions_dedupes() {
    let files = vec!["a.rs".to_string()];
    let mentions = resolve_mentions("@a.rs @a.rs @diff @diff", &files);
    assert_eq!(
      mentions,
      vec![
        ResolvedMention::File("a.rs".to_string()),
        ResolvedMention::Diff(DiffMention::Working),
      ]
    );
  }

  #[test]
  fn truncate_diff_keeps_small_diff_verbatim() {
    assert_eq!(truncate_diff("+added\n-removed\n"), "+added\n-removed");
  }

  #[test]
  fn truncate_diff_caps_large_diff() {
    let diff = (0..(MAX_DIFF_LINES + 50))
      .map(|i| format!("+line {i}"))
      .collect::<Vec<_>>()
      .join("\n");
    let truncated = truncate_diff(&diff);
    assert!(truncated.contains(&format!("truncated, {} lines total", MAX_DIFF_LINES + 50)));
  }

  #[test]
  fn utf16_range_accounts_for_multibyte_prefix() {
    let text = "🚀 @diff";
    let trigger = mention_trigger_at_cursor(text, text.len()).expect("trigger");
    let range = byte_range_to_utf16_range(text, trigger.range);
    assert_eq!(range, 3..8);
  }
}
