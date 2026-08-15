//! Local review comments addressed to the agent: draft on a diff, send as a prompt,
//! then track whether the agent's edits addressed or outdated each comment.

use std::path::PathBuf;
use std::sync::Arc;

use editor::ReviewCommentSide;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LocalAgentReviewCommentState {
  Draft,
  Copied,
  Addressed,
  Outdated,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LocalAgentReviewComment {
  pub id: u64,
  pub in_reply_to_id: Option<u64>,
  pub path: PathBuf,
  pub line: usize,
  pub side: ReviewCommentSide,
  pub start_line: Option<usize>,
  pub start_side: Option<ReviewCommentSide>,
  pub body: Arc<str>,
  pub original_start_line: Option<usize>,
  pub original_lines: Vec<String>,
  pub state: LocalAgentReviewCommentState,
}

pub(crate) fn agent_review_line_label(comment: &LocalAgentReviewComment) -> String {
  let line = comment.line.saturating_add(1);
  let Some(start_line) = comment.start_line.map(|line| line.saturating_add(1)) else {
    return format!("L{line}");
  };
  if start_line == line {
    format!("L{line}")
  } else {
    let start = start_line.min(line);
    let end = start_line.max(line);
    format!("L{start}-L{end}")
  }
}

pub(crate) fn agent_review_side_label(side: ReviewCommentSide) -> &'static str {
  match side {
    ReviewCommentSide::Left => "old",
    ReviewCommentSide::Right => "new",
  }
}

pub(crate) fn agent_review_comment_is_copyable(comment: &LocalAgentReviewComment) -> bool {
  matches!(
    comment.state,
    LocalAgentReviewCommentState::Draft | LocalAgentReviewCommentState::Copied
  )
}

fn lines_match_at(lines: &[String], start_line: usize, expected: &[String]) -> bool {
  if expected.is_empty() || start_line == 0 {
    return false;
  }
  let start_ix = start_line - 1;
  lines
    .get(start_ix..start_ix.saturating_add(expected.len()))
    .is_some_and(|lines| lines == expected)
}

fn contains_line_sequence(lines: &[String], expected: &[String]) -> bool {
  if expected.is_empty() || expected.len() > lines.len() {
    return false;
  }
  lines
    .windows(expected.len())
    .any(|window| window == expected)
}

fn extract_first_suggestion_lines(body: &str) -> Vec<String> {
  let mut in_suggestion = false;
  let mut lines = Vec::new();

  for line in body.lines() {
    let trimmed = line.trim();
    if !in_suggestion {
      if trimmed.starts_with("```suggestion") {
        in_suggestion = true;
      }
      continue;
    }

    if trimmed == "```" {
      return lines;
    }

    lines.push(line.to_string());
  }

  Vec::new()
}

pub(crate) fn next_agent_review_comment_state(
  comment: &LocalAgentReviewComment,
  current_file_lines: &[String],
) -> LocalAgentReviewCommentState {
  if matches!(comment.state, LocalAgentReviewCommentState::Draft) {
    return LocalAgentReviewCommentState::Draft;
  }

  let suggested_lines = extract_first_suggestion_lines(comment.body.as_ref());
  if contains_line_sequence(current_file_lines, &suggested_lines) {
    return LocalAgentReviewCommentState::Addressed;
  }

  if let Some(original_start_line) = comment.original_start_line
    && !lines_match_at(
      current_file_lines,
      original_start_line,
      &comment.original_lines,
    )
  {
    return LocalAgentReviewCommentState::Outdated;
  }

  LocalAgentReviewCommentState::Copied
}

pub(crate) fn format_agent_review_export(comments: &[LocalAgentReviewComment]) -> String {
  let mut comments = comments
    .iter()
    .filter(|comment| agent_review_comment_is_copyable(comment))
    .collect::<Vec<_>>();
  comments.sort_by(|a, b| {
    a.path
      .cmp(&b.path)
      .then_with(|| a.line.cmp(&b.line))
      .then_with(|| a.id.cmp(&b.id))
  });

  let mut output = String::new();

  for comment in comments {
    if !output.is_empty() {
      output.push('\n');
    }
    output.push_str("### ");
    output.push_str(&comment.path.to_string_lossy().replace(['\n', '\r'], ""));
    output.push(':');
    output.push_str(&agent_review_line_label(comment));
    output.push_str(" (");
    output.push_str(agent_review_side_label(comment.side));
    output.push_str(" side)");
    output.push('\n');
    output.push_str(comment.body.trim());
    output.push('\n');
  }

  output
}

#[cfg(test)]
mod tests {
  use super::*;

  fn comment(
    id: u64,
    line: usize,
    body: &str,
    state: LocalAgentReviewCommentState,
  ) -> LocalAgentReviewComment {
    LocalAgentReviewComment {
      id,
      in_reply_to_id: None,
      path: PathBuf::from("src/main.rs"),
      line,
      side: ReviewCommentSide::Right,
      start_line: None,
      start_side: None,
      body: Arc::from(body),
      original_start_line: Some(line.saturating_add(1)),
      original_lines: vec!["let value = custom();".to_string()],
      state,
    }
  }

  #[test]
  fn agent_review_line_label_formats_ranges() {
    let comment = LocalAgentReviewComment {
      start_line: Some(10),
      start_side: Some(ReviewCommentSide::Right),
      ..comment(
        1,
        12,
        "Please simplify this.",
        LocalAgentReviewCommentState::Draft,
      )
    };

    assert_eq!(agent_review_line_label(&comment), "L11-L13");
  }

  #[test]
  fn format_agent_review_export_groups_and_keeps_suggestions() {
    let comments = vec![
      LocalAgentReviewComment {
        path: PathBuf::from("src/lib.rs"),
        ..comment(
          2,
          4,
          "Use the shared helper.",
          LocalAgentReviewCommentState::Draft,
        )
      },
      comment(
        1,
        1,
        "Replace with:\n\n```suggestion\nlet value = shared();\n```",
        LocalAgentReviewCommentState::Draft,
      ),
    ];

    let export = format_agent_review_export(&comments);

    assert!(export.contains("### src/main.rs:L2 (new side)"));
    assert!(export.contains("```suggestion\nlet value = shared();\n```"));
    assert!(export.contains("### src/lib.rs:L5 (new side)"));
    assert!(export.find("src/lib.rs") < export.find("src/main.rs"));
  }

  #[test]
  fn next_agent_review_comment_state_marks_copied_suggestion_as_addressed() {
    let comment = comment(
      1,
      1,
      "Use this:\n\n```suggestion\nlet value = shared();\n```",
      LocalAgentReviewCommentState::Copied,
    );
    let current_lines = vec![
      "fn main() {".to_string(),
      "let value = shared();".to_string(),
      "}".to_string(),
    ];

    assert_eq!(
      next_agent_review_comment_state(&comment, &current_lines),
      LocalAgentReviewCommentState::Addressed
    );
  }

  #[test]
  fn next_agent_review_comment_state_marks_copied_mismatch_as_outdated() {
    let comment = comment(
      1,
      1,
      "Please simplify this.",
      LocalAgentReviewCommentState::Copied,
    );
    let current_lines = vec![
      "fn main() {".to_string(),
      "let value = changed();".to_string(),
      "}".to_string(),
    ];

    assert_eq!(
      next_agent_review_comment_state(&comment, &current_lines),
      LocalAgentReviewCommentState::Outdated
    );
  }

  #[test]
  fn format_agent_review_export_skips_addressed_and_outdated_comments() {
    let comments = vec![
      comment(1, 1, "Still active.", LocalAgentReviewCommentState::Copied),
      comment(
        2,
        3,
        "Already fixed.",
        LocalAgentReviewCommentState::Addressed,
      ),
      comment(3, 5, "Stale.", LocalAgentReviewCommentState::Outdated),
    ];

    let export = format_agent_review_export(&comments);

    assert!(export.contains("Still active."));
    assert!(!export.contains("Already fixed."));
    assert!(!export.contains("Stale."));
  }
}
