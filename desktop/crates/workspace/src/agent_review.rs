//! Local review comments addressed to the agent: draft on a diff, send as a prompt,
//! then track whether the agent's edits addressed or outdated each comment.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use editor::{Editor, ReviewComment, ReviewCommentCreateRequest, ReviewCommentSide};
use gfm_markdown_viewer::SuggestionContext;
use gpui::{App, Entity};

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

pub(crate) fn agent_review_comment_spans_a_range(comment: &LocalAgentReviewComment) -> bool {
  comment
    .start_line
    .is_some_and(|start_line| start_line != comment.line)
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

/// An addressed comment has nothing left to say; an outdated one still does,
/// marked as such.
pub(crate) fn agent_review_comment_is_shown_in_diff(comment: &LocalAgentReviewComment) -> bool {
  !matches!(comment.state, LocalAgentReviewCommentState::Addressed)
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

/// Owns the draft comments for one page: creation, edition, deletion and the
/// state machine that marks them copied, addressed or outdated.
#[derive(Default)]
pub(crate) struct AgentReviewComments {
  comments: Vec<LocalAgentReviewComment>,
  next_id: u64,
}

impl AgentReviewComments {
  pub(crate) fn new() -> Self {
    Self {
      comments: Vec::new(),
      next_id: 1,
    }
  }

  #[cfg(test)]
  pub(crate) fn is_empty(&self) -> bool {
    self.comments.is_empty()
  }

  #[cfg(test)]
  pub(crate) fn all(&self) -> &[LocalAgentReviewComment] {
    &self.comments
  }

  pub(crate) fn copyable_count(&self) -> usize {
    self
      .comments
      .iter()
      .filter(|comment| agent_review_comment_is_copyable(comment))
      .count()
  }

  pub(crate) fn clear(&mut self) {
    self.comments.clear();
  }

  /// Returns the created id, or the reason the comment could not be anchored.
  pub(crate) fn create(
    &mut self,
    request: &ReviewCommentCreateRequest,
    selected_file: Option<&Path>,
    original: (Option<usize>, Vec<String>),
  ) -> Result<u64, Arc<str>> {
    let parent_path = request.in_reply_to_id.and_then(|parent_id| {
      self
        .comments
        .iter()
        .find(|comment| comment.id == parent_id)
        .map(|comment| comment.path.clone())
    });
    let Some(path) = parent_path.or_else(|| selected_file.map(Path::to_path_buf)) else {
      return Err(Arc::from("No selected file"));
    };

    let (original_start_line, original_lines) = if request.in_reply_to_id.is_some() {
      (None, Vec::new())
    } else {
      original
    };

    let id = self.next_id.max(1);
    self.next_id = id.saturating_add(1);
    self.comments.push(LocalAgentReviewComment {
      id,
      in_reply_to_id: request.in_reply_to_id,
      path,
      line: request.line,
      side: request.side,
      start_line: request.start_line,
      start_side: request.start_side,
      body: request.body.clone(),
      original_start_line,
      original_lines,
      state: LocalAgentReviewCommentState::Draft,
    });
    Ok(id)
  }

  pub(crate) fn update(&mut self, comment_id: u64, body: Arc<str>) -> bool {
    let Some(comment) = self
      .comments
      .iter_mut()
      .find(|comment| comment.id == comment_id)
    else {
      return false;
    };
    comment.body = body;
    true
  }

  /// Deleting a thread root takes its replies with it.
  pub(crate) fn delete(&mut self, comment_id: u64) {
    let removed_root = self.root_id(comment_id);
    let removed_ids = self
      .comments
      .iter()
      .filter(|comment| {
        comment.id == comment_id
          || comment.in_reply_to_id == Some(comment_id)
          || (comment_id == removed_root && self.root_id(comment.id) == removed_root)
      })
      .map(|comment| comment.id)
      .collect::<HashSet<_>>();
    self
      .comments
      .retain(|comment| !removed_ids.contains(&comment.id));
  }

  pub(crate) fn root_id(&self, comment_id: u64) -> u64 {
    let mut root_id = comment_id;
    let mut current_id = Some(comment_id);
    for _ in 0..MAX_REPLY_DEPTH {
      let Some(id) = current_id else {
        break;
      };
      let Some(comment) = self.comments.iter().find(|comment| comment.id == id) else {
        break;
      };
      let Some(parent_id) = comment.in_reply_to_id else {
        root_id = comment.id;
        break;
      };
      root_id = parent_id;
      current_id = Some(parent_id);
    }
    root_id
  }

  /// Recomputes the state of the comments anchored in `selected_file` against
  /// its current content. Returns whether anything changed.
  pub(crate) fn refresh_states(&mut self, selected_file: &Path, file_lines: &[String]) -> bool {
    let mut changed = false;
    for comment in &mut self.comments {
      if comment.path != selected_file {
        continue;
      }
      let next_state = next_agent_review_comment_state(comment, file_lines);
      if comment.state != next_state {
        comment.state = next_state;
        changed = true;
      }
    }
    changed
  }

  pub(crate) fn export(&self) -> String {
    format_agent_review_export(&self.comments)
  }

  /// Marks every copyable comment as copied and returns how many were marked.
  pub(crate) fn mark_copyable_as_copied(&mut self) -> usize {
    let mut marked = 0;
    for comment in &mut self.comments {
      if agent_review_comment_is_copyable(comment) {
        comment.state = LocalAgentReviewCommentState::Copied;
        marked += 1;
      }
    }
    marked
  }

  fn editor_comments(&self, selected_file: &Path) -> Vec<ReviewComment> {
    self
      .comments
      .iter()
      .filter(|comment| comment.path == selected_file)
      .filter(|comment| agent_review_comment_is_shown_in_diff(comment))
      .map(to_editor_comment)
      .collect()
  }
}

const MAX_REPLY_DEPTH: usize = 32;

fn to_editor_comment(comment: &LocalAgentReviewComment) -> ReviewComment {
  let suggestion_context = if comment.original_lines.is_empty() {
    None
  } else {
    Some(SuggestionContext {
      original_start_line: comment.original_start_line,
      suggested_start_line: comment.original_start_line,
      original_lines: comment.original_lines.clone(),
      path: Arc::from(comment.path.to_string_lossy().as_ref()),
    })
  };

  ReviewComment {
    id: comment.id,
    in_reply_to_id: comment.in_reply_to_id,
    line: comment.line,
    side: comment.side,
    author: Arc::from(""),
    avatar_url: None,
    // The anchor line is visible in the diff; only a range needs spelling out.
    line_label: agent_review_comment_spans_a_range(comment)
      .then(|| Arc::<str>::from(agent_review_line_label(comment))),
    body: comment.body.clone(),
    suggestion_context,
    created_at: Arc::from(""),
    thread_id: None,
    is_resolved: false,
    is_outdated: matches!(comment.state, LocalAgentReviewCommentState::Outdated),
    viewer_can_resolve: false,
    viewer_can_unresolve: false,
    is_pending: false,
  }
}

/// The file content the comments are anchored against, newline endings stripped.
pub(crate) fn editor_file_lines(editor: &Entity<Editor>, cx: &App) -> Vec<String> {
  let document = editor.read(cx).document().clone();
  let document = document.read(cx);
  (0..document.len_lines())
    .filter_map(|line_ix| {
      document
        .line_content(line_ix)
        .map(|line| line.trim_end_matches(['\r', '\n']).to_string())
    })
    .collect()
}

/// Snapshot of the lines a new comment is anchored to, used later to tell
/// whether the agent addressed it. Only right-side comments carry one.
pub(crate) fn original_lines_for_request(
  editor: Option<&Entity<Editor>>,
  request: &ReviewCommentCreateRequest,
  cx: &App,
) -> (Option<usize>, Vec<String>) {
  if request.side != ReviewCommentSide::Right {
    return (None, Vec::new());
  }
  let Some(editor) = editor else {
    return (None, Vec::new());
  };

  let anchor = request.start_line.unwrap_or(request.line);
  let start = anchor.min(request.line);
  let end = anchor.max(request.line);
  let document = editor.read(cx).document().clone();
  let document = document.read(cx);

  let lines = (start..=end)
    .filter_map(|line_ix| {
      document
        .line_content(line_ix)
        .map(|line| line.trim_end_matches(['\r', '\n']).to_string())
    })
    .collect::<Vec<_>>();

  if lines.is_empty() {
    (None, Vec::new())
  } else {
    (Some(start.saturating_add(1)), lines)
  }
}

/// Pushes the comments of `selected_file` into the editor, refreshing their
/// state against the current content first.
pub(crate) fn sync_comments_to_editor(
  comments: &mut AgentReviewComments,
  editor: Option<&Entity<Editor>>,
  selected_file: Option<&Path>,
  cx: &mut App,
) {
  let Some(editor) = editor else {
    return;
  };
  let Some(selected_file) = selected_file else {
    editor.update(cx, |editor, cx| {
      editor.set_review_comments(Vec::new(), cx);
      editor.set_editable_review_comment_ids(std::iter::empty::<u64>(), cx);
    });
    return;
  };

  let file_lines = editor_file_lines(editor, cx);
  comments.refresh_states(selected_file, &file_lines);

  let editor_comments = comments.editor_comments(selected_file);
  let editable_ids = editor_comments
    .iter()
    .map(|comment| comment.id)
    .collect::<Vec<_>>();

  editor.update(cx, |editor, cx| {
    editor.set_editable_review_comment_ids(editable_ids, cx);
    editor.set_review_comments(editor_comments, cx);
  });
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
  fn only_a_range_carries_a_line_label_into_the_diff() {
    let single = comment(1, 12, "fix", LocalAgentReviewCommentState::Draft);
    assert!(!agent_review_comment_spans_a_range(&single));
    assert!(to_editor_comment(&single).line_label.is_none());

    let range = LocalAgentReviewComment {
      start_line: Some(10),
      ..comment(2, 12, "fix", LocalAgentReviewCommentState::Draft)
    };
    assert!(agent_review_comment_spans_a_range(&range));
    assert_eq!(
      to_editor_comment(&range).line_label.as_deref(),
      Some("L11-L13")
    );
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

  fn create_request(line: usize, body: &str) -> ReviewCommentCreateRequest {
    ReviewCommentCreateRequest {
      line,
      side: ReviewCommentSide::Right,
      start_line: None,
      start_side: None,
      body: Arc::from(body),
      in_reply_to_id: None,
      mode: editor::ReviewCommentMode::SingleComment,
    }
  }

  #[test]
  fn create_refuses_a_comment_without_a_selected_file() {
    let mut comments = AgentReviewComments::new();

    let created = comments.create(&create_request(3, "extract this"), None, (None, Vec::new()));

    assert!(created.is_err());
    assert!(comments.is_empty());
  }

  #[test]
  fn create_anchors_replies_on_the_parent_path() {
    let mut comments = AgentReviewComments::new();
    let parent = comments
      .create(
        &create_request(3, "extract this"),
        Some(Path::new("src/main.rs")),
        (Some(4), vec!["let value = custom();".to_string()]),
      )
      .expect("create parent");

    let mut reply = create_request(9, "agreed");
    reply.in_reply_to_id = Some(parent);
    // Another file is selected, the reply must still follow its parent.
    comments
      .create(&reply, Some(Path::new("src/other.rs")), (Some(1), vec![]))
      .expect("create reply");

    let stored = comments.all();
    assert_eq!(stored.len(), 2);
    assert_eq!(stored[1].path, PathBuf::from("src/main.rs"));
    assert!(stored[1].original_lines.is_empty());
  }

  #[test]
  fn deleting_a_thread_root_removes_its_replies() {
    let mut comments = AgentReviewComments::new();
    let root = comments
      .create(
        &create_request(3, "extract this"),
        Some(Path::new("src/main.rs")),
        (None, Vec::new()),
      )
      .expect("create root");
    let mut reply = create_request(3, "agreed");
    reply.in_reply_to_id = Some(root);
    comments
      .create(&reply, Some(Path::new("src/main.rs")), (None, Vec::new()))
      .expect("create reply");

    comments.delete(root);

    assert!(comments.is_empty());
  }

  #[test]
  fn refresh_states_only_touches_comments_of_the_given_file() {
    let mut comments = AgentReviewComments::new();
    comments
      .create(
        &create_request(0, "extract this"),
        Some(Path::new("src/main.rs")),
        (Some(1), vec!["let value = custom();".to_string()]),
      )
      .expect("create comment");
    comments
      .create(
        &create_request(0, "other file"),
        Some(Path::new("src/other.rs")),
        (Some(1), vec!["let value = custom();".to_string()]),
      )
      .expect("create comment");

    // Drafts are frozen until they are sent, so send them first.
    comments.mark_copyable_as_copied();

    let changed = comments.refresh_states(
      Path::new("src/main.rs"),
      &["let value = replaced();".to_string()],
    );

    assert!(changed);
    let stored = comments.all();
    assert_eq!(stored[0].state, LocalAgentReviewCommentState::Outdated);
    assert_eq!(stored[1].state, LocalAgentReviewCommentState::Copied);
  }

  #[test]
  fn marking_as_copied_covers_every_copyable_comment() {
    let mut comments = AgentReviewComments::new();
    comments
      .create(
        &create_request(0, "extract this"),
        Some(Path::new("src/main.rs")),
        (None, Vec::new()),
      )
      .expect("create comment");

    assert_eq!(comments.copyable_count(), 1);
    assert_eq!(comments.mark_copyable_as_copied(), 1);
    assert_eq!(
      comments.all()[0].state,
      LocalAgentReviewCommentState::Copied
    );
    // Copied comments stay copyable until the agent addresses them.
    assert_eq!(comments.copyable_count(), 1);
  }

  #[test]
  fn update_reports_whether_the_comment_exists() {
    let mut comments = AgentReviewComments::new();
    let id = comments
      .create(
        &create_request(3, "extract this"),
        Some(Path::new("src/main.rs")),
        (None, Vec::new()),
      )
      .expect("create comment");

    assert!(comments.update(id, Arc::from("extract this helper")));
    assert!(!comments.update(id + 100, Arc::from("ghost")));
    assert_eq!(comments.all()[0].body.as_ref(), "extract this helper");
  }

  #[test]
  fn deleting_a_reply_keeps_its_thread_root() {
    let mut comments = AgentReviewComments::new();
    let root = comments
      .create(
        &create_request(3, "extract this"),
        Some(Path::new("src/main.rs")),
        (None, Vec::new()),
      )
      .expect("create root");
    let mut reply = create_request(3, "agreed");
    reply.in_reply_to_id = Some(root);
    let reply_id = comments
      .create(&reply, Some(Path::new("src/main.rs")), (None, Vec::new()))
      .expect("create reply");

    comments.delete(reply_id);

    let stored = comments.all();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].id, root);
  }

  #[test]
  fn editor_comments_keep_only_the_pending_ones_of_the_open_file() {
    let mut comments = AgentReviewComments::new();
    comments
      .create(
        &create_request(0, "extract this"),
        Some(Path::new("src/main.rs")),
        (Some(1), vec!["let value = custom();".to_string()]),
      )
      .expect("create comment");
    comments
      .create(
        &create_request(0, "other file"),
        Some(Path::new("src/other.rs")),
        (None, Vec::new()),
      )
      .expect("create comment");

    let rendered = comments.editor_comments(Path::new("src/main.rs"));
    assert_eq!(rendered.len(), 1);
    assert_eq!(rendered[0].body.as_ref(), "extract this");
    // The snapshot travels as a suggestion context so the diff can render it.
    let suggestion = rendered[0]
      .suggestion_context
      .as_ref()
      .expect("suggestion context");
    assert_eq!(suggestion.original_start_line, Some(1));
    assert_eq!(suggestion.original_lines, vec!["let value = custom();"]);
  }

  #[test]
  fn an_outdated_comment_stays_in_the_diff_marked_outdated() {
    let mut comments = AgentReviewComments::new();
    comments
      .create(
        &create_request(0, "rename it\n\n```suggestion\nlet total = custom();\n```"),
        Some(Path::new("src/main.rs")),
        (Some(1), vec!["let value = custom();".to_string()]),
      )
      .expect("create comment");
    comments.mark_copyable_as_copied();

    // The agent rewrote the line into something else: the comment lost its anchor.
    comments.refresh_states(
      Path::new("src/main.rs"),
      &["let value = other();".to_string()],
    );

    assert_eq!(
      comments.all()[0].state,
      LocalAgentReviewCommentState::Outdated
    );
    let rendered = comments.editor_comments(Path::new("src/main.rs"));
    assert_eq!(
      rendered.len(),
      1,
      "an outdated comment still has something to say"
    );
    assert!(rendered[0].is_outdated);
    // Outdated is not sendable: the agent would get a comment about gone code.
    assert_eq!(comments.copyable_count(), 0);
  }

  #[test]
  fn an_addressed_suggestion_leaves_the_diff() {
    let mut comments = AgentReviewComments::new();
    comments
      .create(
        &create_request(0, "rename it\n\n```suggestion\nlet total = custom();\n```"),
        Some(Path::new("src/main.rs")),
        (Some(1), vec!["let value = custom();".to_string()]),
      )
      .expect("create comment");
    comments.mark_copyable_as_copied();

    // The agent applied the suggestion.
    comments.refresh_states(
      Path::new("src/main.rs"),
      &["let total = custom();".to_string()],
    );

    assert_eq!(
      comments.all()[0].state,
      LocalAgentReviewCommentState::Addressed
    );
    assert!(
      comments
        .editor_comments(Path::new("src/main.rs"))
        .is_empty()
    );
    assert_eq!(comments.copyable_count(), 0);
  }
}
