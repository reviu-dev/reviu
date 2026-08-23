//! Where a repository's review batch lives on disk, next to its conversations.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use agent_chat_panel::AgentChatPanel;
use editor::ReviewCommentSide;

use crate::agent_review::{LocalAgentReviewComment, LocalAgentReviewCommentState};
use app_log::ResultExt;

/// Bump when the on-disk shape changes; readers dispatch on it.
const REVIEW_FORMAT_VERSION: u32 = 1;

const REVIEW_FILE_NAME: &str = "review.json";

pub(crate) struct StoredReview {
  pub comments: Vec<LocalAgentReviewComment>,
  pub next_id: u64,
}

/// The batch sits in the same per-repo directory as that repo's conversations.
pub(crate) fn review_path_for_repo(state_dir: &Path, repo: &Path) -> PathBuf {
  AgentChatPanel::state_dir_for_repo(state_dir, repo).join(REVIEW_FILE_NAME)
}

pub(crate) fn read_review(path: &Path) -> Option<StoredReview> {
  let raw = std::fs::read_to_string(path).ok()?;
  let parsed: PersistedReview = serde_json::from_str(&raw).ok()?;
  if parsed.version != REVIEW_FORMAT_VERSION {
    return None;
  }
  let comments = parsed
    .comments
    .into_iter()
    .filter_map(PersistedComment::into_comment)
    .collect::<Vec<_>>();
  // A file written by a newer id counter than its own comments is fine; the
  // other way round would hand out an id that is already taken.
  let next_id = comments
    .iter()
    .map(|comment| comment.id.saturating_add(1))
    .max()
    .unwrap_or(1)
    .max(parsed.next_id);
  Some(StoredReview { comments, next_id })
}

/// An empty batch leaves nothing behind rather than an empty file.
pub(crate) fn write_review(path: &Path, comments: &[LocalAgentReviewComment], next_id: u64) {
  if comments.is_empty() {
    std::fs::remove_file(path).log_err_context("clearing the review file");
    return;
  }
  if let Some(dir) = path.parent()
    && std::fs::create_dir_all(dir)
      .log_err_context("creating the review dir")
      .is_none()
  {
    return;
  }
  let file = PersistedReview {
    version: REVIEW_FORMAT_VERSION,
    next_id,
    comments: comments
      .iter()
      .map(PersistedComment::from_comment)
      .collect(),
  };
  if let Some(json) = serde_json::to_string(&file).log_err_context("serializing the review") {
    std::fs::write(path, json).log_err_context("writing the review");
  }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PersistedReview {
  version: u32,
  next_id: u64,
  comments: Vec<PersistedComment>,
}

/// `LocalAgentReviewComment` cannot derive serde: its body is an `Arc<str>` and
/// its side comes from the editor crate.
#[derive(serde::Serialize, serde::Deserialize)]
struct PersistedComment {
  id: u64,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  in_reply_to_id: Option<u64>,
  path: PathBuf,
  line: usize,
  side: PersistedSide,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  start_line: Option<usize>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  start_side: Option<PersistedSide>,
  body: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  original_start_line: Option<usize>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  original_lines: Vec<String>,
  state: PersistedState,
}

impl PersistedComment {
  fn from_comment(comment: &LocalAgentReviewComment) -> Self {
    Self {
      id: comment.id,
      in_reply_to_id: comment.in_reply_to_id,
      path: comment.path.clone(),
      line: comment.line,
      side: PersistedSide::from(comment.side),
      start_line: comment.start_line,
      start_side: comment.start_side.map(PersistedSide::from),
      body: comment.body.to_string(),
      original_start_line: comment.original_start_line,
      original_lines: comment.original_lines.clone(),
      state: PersistedState::from(&comment.state),
    }
  }

  fn into_comment(self) -> Option<LocalAgentReviewComment> {
    let state = self.state.into_state()?;
    Some(LocalAgentReviewComment {
      id: self.id,
      in_reply_to_id: self.in_reply_to_id,
      path: self.path,
      line: self.line,
      side: self.side.into(),
      start_line: self.start_line,
      start_side: self.start_side.map(Into::into),
      body: Arc::from(self.body.as_str()),
      original_start_line: self.original_start_line,
      original_lines: self.original_lines,
      state,
    })
  }
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum PersistedSide {
  Old,
  New,
}

impl From<ReviewCommentSide> for PersistedSide {
  fn from(side: ReviewCommentSide) -> Self {
    match side {
      ReviewCommentSide::Left => Self::Old,
      ReviewCommentSide::Right => Self::New,
    }
  }
}

impl From<PersistedSide> for ReviewCommentSide {
  fn from(side: PersistedSide) -> Self {
    match side {
      PersistedSide::Old => Self::Left,
      PersistedSide::New => Self::Right,
    }
  }
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum PersistedState {
  Draft,
  Copied,
  Addressed,
  Outdated,
}

impl From<&LocalAgentReviewCommentState> for PersistedState {
  fn from(state: &LocalAgentReviewCommentState) -> Self {
    match state {
      LocalAgentReviewCommentState::Draft => Self::Draft,
      LocalAgentReviewCommentState::Sent => Self::Copied,
    }
  }
}

impl PersistedState {
  /// `addressed` and `outdated` come from the state machine that used to watch
  /// the diff. Those comments were dealt with, so a reload drops them.
  fn into_state(self) -> Option<LocalAgentReviewCommentState> {
    match self {
      Self::Draft => Some(LocalAgentReviewCommentState::Draft),
      Self::Copied => Some(LocalAgentReviewCommentState::Sent),
      Self::Addressed | Self::Outdated => None,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn temp_dir(name: &str) -> PathBuf {
    let dir =
      std::env::temp_dir().join(format!("reviu-review-store-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
  }

  /// `app_log::init` fires once per process, so this test owns the sink for the whole binary.
  #[test]
  fn a_failed_review_write_lands_in_the_log() {
    let dir = temp_dir("write-failure");
    let log_path = dir.join("reviu.log");
    app_log::init(Some(log_path.clone()));

    let blocker = dir.join("blocker");
    std::fs::write(&blocker, b"not a directory").expect("seed blocker file");

    write_review(
      &blocker.join("review.json"),
      &[comment(1, LocalAgentReviewCommentState::Draft)],
      2,
    );

    let logged = std::fs::read_to_string(&log_path).expect("log file should exist");
    assert!(logged.contains("creating the review dir"), "got {logged:?}");
  }

  fn comment(id: u64, state: LocalAgentReviewCommentState) -> LocalAgentReviewComment {
    LocalAgentReviewComment {
      id,
      in_reply_to_id: None,
      path: PathBuf::from("src/main.rs"),
      line: 3,
      side: ReviewCommentSide::Right,
      start_line: Some(1),
      start_side: Some(ReviewCommentSide::Right),
      body: Arc::from("extract this"),
      original_start_line: Some(2),
      original_lines: vec!["let value = custom();".to_string()],
      state,
    }
  }

  #[test]
  fn a_batch_survives_a_round_trip() {
    let dir = temp_dir("round-trip");
    let path = dir.join(REVIEW_FILE_NAME);

    let comments = vec![
      comment(1, LocalAgentReviewCommentState::Draft),
      LocalAgentReviewComment {
        side: ReviewCommentSide::Left,
        start_line: None,
        start_side: None,
        original_start_line: None,
        original_lines: Vec::new(),
        ..comment(2, LocalAgentReviewCommentState::Sent)
      },
    ];
    write_review(&path, &comments, 3);

    let stored = read_review(&path).expect("read back");
    assert_eq!(stored.comments, comments);
    assert_eq!(stored.next_id, 3);

    let _ = std::fs::remove_dir_all(&dir);
  }

  #[test]
  fn the_id_counter_never_comes_back_behind_the_comments() {
    let dir = temp_dir("next-id");
    let path = dir.join(REVIEW_FILE_NAME);

    // A counter left behind by a truncated write must not hand out id 7 twice.
    write_review(&path, &[comment(7, LocalAgentReviewCommentState::Draft)], 1);

    assert_eq!(read_review(&path).expect("read back").next_id, 8);

    let _ = std::fs::remove_dir_all(&dir);
  }

  #[test]
  fn an_emptied_batch_leaves_no_file_behind() {
    let dir = temp_dir("empty");
    let path = dir.join(REVIEW_FILE_NAME);

    write_review(&path, &[comment(1, LocalAgentReviewCommentState::Draft)], 2);
    assert!(path.exists());

    write_review(&path, &[], 2);

    assert!(!path.exists());
    assert!(read_review(&path).is_none());

    let _ = std::fs::remove_dir_all(&dir);
  }

  #[test]
  fn a_batch_written_by_the_old_state_machine_keeps_only_what_still_means_something() {
    let dir = temp_dir("legacy-states");
    let path = dir.join(REVIEW_FILE_NAME);

    // Written before comments became temporary: those two were dealt with.
    std::fs::write(
      &path,
      serde_json::to_string(&PersistedReview {
        version: REVIEW_FORMAT_VERSION,
        next_id: 5,
        comments: vec![
          persisted(1, PersistedState::Draft),
          persisted(2, PersistedState::Copied),
          persisted(3, PersistedState::Addressed),
          persisted(4, PersistedState::Outdated),
        ],
      })
      .expect("serialize"),
    )
    .expect("write file");

    let stored = read_review(&path).expect("read back");

    let states = stored
      .comments
      .iter()
      .map(|comment| (comment.id, comment.state.clone()))
      .collect::<Vec<_>>();
    assert_eq!(
      states,
      vec![
        (1, LocalAgentReviewCommentState::Draft),
        (2, LocalAgentReviewCommentState::Sent),
      ]
    );

    let _ = std::fs::remove_dir_all(&dir);
  }

  fn persisted(id: u64, state: PersistedState) -> PersistedComment {
    let mut persisted =
      PersistedComment::from_comment(&comment(id, LocalAgentReviewCommentState::Draft));
    persisted.state = state;
    persisted
  }

  #[test]
  fn a_file_from_another_format_is_ignored_rather_than_fatal() {
    let dir = temp_dir("version");
    let path = dir.join(REVIEW_FILE_NAME);

    std::fs::write(
      &path,
      format!(
        "{{\"version\":{},\"next_id\":1,\"comments\":[]}}",
        REVIEW_FORMAT_VERSION + 1
      ),
    )
    .expect("write file");
    assert!(read_review(&path).is_none());

    std::fs::write(&path, "not json at all").expect("write file");
    assert!(read_review(&path).is_none());

    let _ = std::fs::remove_dir_all(&dir);
  }

  #[test]
  fn a_repo_keeps_its_batch_next_to_its_conversations() {
    let state_dir = PathBuf::from("/state");
    let repo = std::env::temp_dir().join("reviu-review-store-path");

    let path = review_path_for_repo(&state_dir, &repo);

    assert_eq!(
      path.file_name().and_then(|n| n.to_str()),
      Some("review.json")
    );
    assert_eq!(
      path.parent(),
      Some(AgentChatPanel::state_dir_for_repo(&state_dir, &repo).as_path())
    );
  }
}
