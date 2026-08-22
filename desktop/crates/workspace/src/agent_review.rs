//! Local review comments addressed to the agent: draft on a diff, send as a
//! prompt, and let a completed turn consume them. They are instructions, not a
//! record like a pull request's comments, so they do not outlive the turn they
//! were sent for.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use editor::{Editor, ReviewComment, ReviewCommentCreateRequest, ReviewCommentSide};
use gfm_markdown_viewer::SuggestionContext;
use gpui::{App, Entity};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LocalAgentReviewCommentState {
  Draft,
  /// Handed to the agent, waiting for the turn to end. A completed turn takes
  /// it away.
  Sent,
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

/// Only what has not left yet: a sent comment is already with the agent.
pub(crate) fn agent_review_state_is_sendable(state: &LocalAgentReviewCommentState) -> bool {
  matches!(state, LocalAgentReviewCommentState::Draft)
}

pub(crate) fn agent_review_comment_is_sendable(comment: &LocalAgentReviewComment) -> bool {
  agent_review_state_is_sendable(&comment.state)
}

/// Which comments a send covers. The panel's empty selection means the whole
/// batch: `Send` never sends nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ReviewSend {
  WholeBatch,
  Only(HashSet<u64>),
}

impl ReviewSend {
  pub(crate) fn from_selection(selection: HashSet<u64>) -> Self {
    if selection.is_empty() {
      Self::WholeBatch
    } else {
      Self::Only(selection)
    }
  }

  pub(crate) fn one(comment_id: u64) -> Self {
    Self::Only(HashSet::from([comment_id]))
  }

  fn covers(&self, comment: &LocalAgentReviewComment) -> bool {
    match self {
      Self::WholeBatch => true,
      Self::Only(ids) => ids.contains(&comment.id),
    }
  }
}

pub(crate) fn format_agent_review_export(
  comments: &[LocalAgentReviewComment],
  send: &ReviewSend,
) -> String {
  let mut comments = comments
    .iter()
    .filter(|comment| agent_review_comment_is_sendable(comment) && send.covers(comment))
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
    // The new side is the ordinary case: only a comment on removed code needs
    // saying which side its line number belongs to.
    if comment.side == ReviewCommentSide::Left {
      output.push_str(" (old side)");
    }
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
  /// Set by every change, taken by whoever writes the batch to disk.
  dirty: bool,
}

impl AgentReviewComments {
  pub(crate) fn new() -> Self {
    Self {
      comments: Vec::new(),
      next_id: 1,
      dirty: false,
    }
  }

  /// A batch read back from disk starts clean: nothing to write yet.
  pub(crate) fn restored(comments: Vec<LocalAgentReviewComment>, next_id: u64) -> Self {
    Self {
      comments,
      next_id: next_id.max(1),
      dirty: false,
    }
  }

  pub(crate) fn next_id(&self) -> u64 {
    self.next_id.max(1)
  }

  pub(crate) fn take_dirty(&mut self) -> bool {
    std::mem::take(&mut self.dirty)
  }

  #[cfg(test)]
  pub(crate) fn is_empty(&self) -> bool {
    self.comments.is_empty()
  }

  pub(crate) fn all(&self) -> &[LocalAgentReviewComment] {
    &self.comments
  }

  /// What a plain `Send` would carry, and what the rail badge counts.
  pub(crate) fn draft_count(&self) -> usize {
    self
      .comments
      .iter()
      .filter(|comment| agent_review_comment_is_sendable(comment))
      .count()
  }

  pub(crate) fn sendable_count(&self, send: &ReviewSend) -> usize {
    self
      .comments
      .iter()
      .filter(|comment| agent_review_comment_is_sendable(comment) && send.covers(comment))
      .count()
  }

  pub(crate) fn clear(&mut self) {
    if self.comments.is_empty() {
      return;
    }
    self.comments.clear();
    self.dirty = true;
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
    self.dirty = true;
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
    self.dirty = true;
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
    self.dirty |= !removed_ids.is_empty();
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

  pub(crate) fn export(&self, send: &ReviewSend) -> String {
    format_agent_review_export(&self.comments, send)
  }

  /// Marks what just left and returns how many. What stayed behind is still a
  /// draft, and goes with the next send.
  pub(crate) fn mark_as_sent(&mut self, send: &ReviewSend) -> usize {
    let mut marked = 0;
    for comment in &mut self.comments {
      if agent_review_comment_is_sendable(comment) && send.covers(comment) {
        comment.state = LocalAgentReviewCommentState::Sent;
        marked += 1;
      }
    }
    self.dirty |= marked > 0;
    marked
  }

  /// A completed turn consumes the comments it was sent: they were instructions,
  /// and the work is over. Returns how many were dropped.
  pub(crate) fn clear_sent(&mut self) -> usize {
    let before = self.comments.len();
    self.comments.retain(agent_review_comment_is_sendable);
    let dropped = before - self.comments.len();
    self.dirty |= dropped > 0;
    dropped
  }

  /// Which of the open file's comments still have somewhere to go.
  fn sendable_ids_in(&self, selected_file: &Path) -> Vec<u64> {
    self
      .comments
      .iter()
      .filter(|comment| comment.path == selected_file)
      .filter(|comment| agent_review_comment_is_sendable(comment))
      .map(|comment| comment.id)
      .collect()
  }

  fn editor_comments(&self, selected_file: &Path) -> Vec<ReviewComment> {
    self
      .comments
      .iter()
      .filter(|comment| comment.path == selected_file)
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
    // Nothing local goes stale any more: a turn takes its comments with it.
    is_outdated: false,
    viewer_can_resolve: false,
    viewer_can_unresolve: false,
    is_pending: false,
  }
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
  comments: &AgentReviewComments,
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
      editor.set_sendable_review_comment_ids(std::iter::empty::<u64>(), cx);
    });
    return;
  };

  let editor_comments = comments.editor_comments(selected_file);
  let editable_ids = editor_comments
    .iter()
    .map(|comment| comment.id)
    .collect::<Vec<_>>();
  let sendable_ids = comments.sendable_ids_in(selected_file);

  editor.update(cx, |editor, cx| {
    editor.set_editable_review_comment_ids(editable_ids, cx);
    editor.set_sendable_review_comment_ids(sendable_ids, cx);
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

    let export = format_agent_review_export(&comments, &ReviewSend::WholeBatch);

    assert!(export.contains("### src/main.rs:L2\n"));
    assert!(export.contains("```suggestion\nlet value = shared();\n```"));
    assert!(export.contains("### src/lib.rs:L5\n"));
    assert!(export.find("src/lib.rs") < export.find("src/main.rs"));
  }

  #[test]
  fn only_a_comment_on_removed_code_spells_out_its_side() {
    let old_side = LocalAgentReviewComment {
      side: ReviewCommentSide::Left,
      ..comment(
        1,
        1,
        "This line went away.",
        LocalAgentReviewCommentState::Draft,
      )
    };

    let export = format_agent_review_export(&[old_side], &ReviewSend::WholeBatch);

    assert!(export.contains("### src/main.rs:L2 (old side)"));
  }

  #[test]
  fn a_sent_comment_does_not_go_out_twice() {
    let comments = vec![
      comment(1, 1, "Still waiting.", LocalAgentReviewCommentState::Draft),
      comment(2, 3, "Already gone.", LocalAgentReviewCommentState::Sent),
    ];

    let export = format_agent_review_export(&comments, &ReviewSend::WholeBatch);

    assert!(export.contains("Still waiting."));
    assert!(!export.contains("Already gone."));
  }

  #[test]
  fn a_selection_sends_only_what_it_names() {
    let comments = vec![
      comment(1, 1, "First.", LocalAgentReviewCommentState::Draft),
      comment(2, 3, "Second.", LocalAgentReviewCommentState::Draft),
    ];

    let export = format_agent_review_export(&comments, &ReviewSend::one(2));

    assert!(!export.contains("First."));
    assert!(export.contains("Second."));
  }

  #[test]
  fn an_empty_selection_stands_for_the_whole_batch() {
    let comments = vec![
      comment(1, 1, "First.", LocalAgentReviewCommentState::Draft),
      comment(2, 3, "Second.", LocalAgentReviewCommentState::Draft),
    ];

    let send = ReviewSend::from_selection(HashSet::new());

    assert_eq!(send, ReviewSend::WholeBatch);
    let export = format_agent_review_export(&comments, &send);
    assert!(export.contains("First."));
    assert!(export.contains("Second."));
  }

  #[test]
  fn what_stayed_behind_is_still_a_draft() {
    let mut comments = AgentReviewComments::new();
    let first = comments
      .create(
        &create_request(0, "first"),
        Some(Path::new("src/main.rs")),
        (None, Vec::new()),
      )
      .expect("create first");
    comments
      .create(
        &create_request(2, "second"),
        Some(Path::new("src/main.rs")),
        (None, Vec::new()),
      )
      .expect("create second");

    let send = ReviewSend::one(first);
    assert_eq!(comments.sendable_count(&send), 1);
    assert_eq!(comments.mark_as_sent(&send), 1);

    let stored = comments.all();
    assert_eq!(stored[0].state, LocalAgentReviewCommentState::Sent);
    assert_eq!(stored[1].state, LocalAgentReviewCommentState::Draft);
    // Only the one left behind can still go.
    assert_eq!(comments.draft_count(), 1);
  }

  #[test]
  fn a_completed_turn_takes_away_what_it_was_sent() {
    let mut comments = AgentReviewComments::new();
    let sent = comments
      .create(
        &create_request(0, "sent with the turn"),
        Some(Path::new("src/main.rs")),
        (None, Vec::new()),
      )
      .expect("create sent");
    comments.mark_as_sent(&ReviewSend::WholeBatch);
    // Written while the agent was working: it never left, so it stays.
    comments
      .create(
        &create_request(2, "written meanwhile"),
        Some(Path::new("src/main.rs")),
        (None, Vec::new()),
      )
      .expect("create draft");

    assert_eq!(comments.clear_sent(), 1);

    let stored = comments.all();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].body.as_ref(), "written meanwhile");
    assert_ne!(stored[0].id, sent);
    assert_eq!(comments.draft_count(), 1);
  }

  #[test]
  fn clearing_a_turn_that_sent_nothing_changes_nothing() {
    let mut comments = AgentReviewComments::new();
    comments
      .create(
        &create_request(0, "still a draft"),
        Some(Path::new("src/main.rs")),
        (None, Vec::new()),
      )
      .expect("create comment");

    assert_eq!(comments.clear_sent(), 0);
    assert_eq!(comments.all().len(), 1);
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
  fn the_diff_shows_the_comments_of_the_open_file_only() {
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
  fn a_sent_comment_stays_in_the_diff_without_a_send_action() {
    let mut comments = AgentReviewComments::new();
    comments
      .create(
        &create_request(0, "extract this"),
        Some(Path::new("src/main.rs")),
        (None, Vec::new()),
      )
      .expect("create comment");
    comments.mark_as_sent(&ReviewSend::WholeBatch);

    // Visible while the agent works on it, but it has nowhere left to go.
    assert_eq!(comments.editor_comments(Path::new("src/main.rs")).len(), 1);
    assert!(
      comments
        .sendable_ids_in(Path::new("src/main.rs"))
        .is_empty()
    );
  }
}
