//! The comments of an unsubmitted pull request review, as the Review panel
//! shows them. GitHub owns them, so they are read back from the API rather than
//! kept next to the local batch.

use std::path::PathBuf;

use crate::api::GithubPullRequestReviewComment;
use crate::review_list::{
  ReviewPanelComment, ReviewRowStatus, ReviewSection, review_comment_excerpt,
  sort_review_panel_comments,
};

/// The node id of the viewer's unsubmitted review. Nothing else on the pull
/// request names it, so it is read off any of its comments.
pub(crate) fn pending_review_id(comments: &[GithubPullRequestReviewComment]) -> Option<String> {
  comments
    .iter()
    .filter(|comment| comment.is_pending)
    .find_map(|comment| comment.pull_request_review_node_id.clone())
}

pub(crate) fn pending_review_comment_node_id(
  comments: &[GithubPullRequestReviewComment],
  id: u64,
) -> Option<String> {
  comments
    .iter()
    .find(|comment| comment.id == id && !comment.node_id.is_empty())
    .map(|comment| comment.node_id.clone())
}

/// The line the file numbers this comment on. A comment the diff moved under
/// keeps the line it was written against, which is all there is left to show.
fn comment_line(comment: &GithubPullRequestReviewComment) -> usize {
  comment
    .line
    .or(comment.original_line)
    .unwrap_or(1)
    .max(1)
    .unsigned_abs() as usize
}

fn comment_line_label(comment: &GithubPullRequestReviewComment) -> String {
  let line = comment_line(comment);
  let Some(start_line) = comment
    .start_line
    .or(comment.original_start_line)
    .map(|start| start.max(1).unsigned_abs() as usize)
  else {
    return format!("L{line}");
  };
  if start_line >= line {
    return format!("L{line}");
  }
  format!("L{start_line}-L{line}")
}

pub(crate) fn pending_review_rows(
  comments: &[GithubPullRequestReviewComment],
) -> Vec<ReviewPanelComment> {
  let mut rows = comments
    .iter()
    .filter(|comment| comment.is_pending)
    .map(|comment| ReviewPanelComment {
      id: comment.id,
      section: ReviewSection::PullRequest,
      path: PathBuf::from(comment.path.as_str()),
      line: comment_line(comment),
      line_label: comment_line_label(comment),
      excerpt: review_comment_excerpt(comment.body.as_str()),
      status: if comment.is_outdated {
        ReviewRowStatus::Outdated
      } else {
        ReviewRowStatus::Pending
      },
      // GitHub submits a review whole: there is no subset to tick.
      sendable: false,
    })
    .collect::<Vec<_>>();
  sort_review_panel_comments(&mut rows);
  rows
}

#[cfg(test)]
pub(crate) fn pending_comment_fixture(
  id: u64,
  path: &str,
  line: Option<i64>,
  body: &str,
) -> GithubPullRequestReviewComment {
  GithubPullRequestReviewComment {
    node_id: format!("node-{id}"),
    is_outdated: false,
    thread_id: format!("thread-{id}"),
    is_resolved: false,
    is_collapsed: false,
    viewer_can_resolve: true,
    viewer_can_unresolve: false,
    id,
    pull_request_review_id: Some(7),
    diff_hunk: String::new(),
    path: path.to_string(),
    position: None,
    original_position: None,
    commit_id: "head".to_string(),
    original_commit_id: "head".to_string(),
    in_reply_to_id: None,
    user: crate::api::GithubPullRequestReviewCommentUser {
      login: "octocat".to_string(),
      avatar_url: None,
    },
    body: body.to_string(),
    created_at: "2026-08-22T10:00:00Z".to_string(),
    updated_at: "2026-08-22T10:00:00Z".to_string(),
    start_line: None,
    original_start_line: None,
    start_side: None,
    line,
    original_line: None,
    side: Some("RIGHT".to_string()),
    is_pending: true,
    pull_request_review_node_id: Some("review-node".to_string()),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn comment(id: u64, path: &str, line: Option<i64>, body: &str) -> GithubPullRequestReviewComment {
    pending_comment_fixture(id, path, line, body)
  }

  #[test]
  fn only_the_unsubmitted_comments_reach_the_panel() {
    let mut published = comment(2, "src/b.rs", Some(4), "already on GitHub");
    published.is_pending = false;

    let rows = pending_review_rows(&[comment(1, "src/a.rs", Some(9), "still a draft"), published]);

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, 1);
    assert_eq!(rows[0].section, ReviewSection::PullRequest);
    assert_eq!(rows[0].excerpt, "still a draft");
    assert!(!rows[0].sendable);
  }

  #[test]
  fn rows_read_in_file_order() {
    let rows = pending_review_rows(&[
      comment(3, "src/b.rs", Some(4), "third"),
      comment(1, "src/a.rs", Some(9), "second"),
      comment(2, "src/a.rs", Some(2), "first"),
    ]);

    assert_eq!(
      rows
        .iter()
        .map(|row| row.excerpt.as_str())
        .collect::<Vec<_>>(),
      vec!["first", "second", "third"]
    );
  }

  #[test]
  fn a_comment_the_diff_moved_under_keeps_the_line_it_was_written_against() {
    let mut moved = comment(1, "src/a.rs", None, "stale anchor");
    moved.original_line = Some(12);
    moved.is_outdated = true;

    let rows = pending_review_rows(&[moved]);

    assert_eq!(rows[0].line, 12);
    assert_eq!(rows[0].line_label, "L12");
    assert_eq!(rows[0].status, ReviewRowStatus::Outdated);
  }

  #[test]
  fn a_multi_line_comment_shows_its_range() {
    let mut ranged = comment(1, "src/a.rs", Some(12), "range");
    ranged.start_line = Some(10);

    let rows = pending_review_rows(&[ranged]);

    assert_eq!(rows[0].line_label, "L10-L12");
    // The row opens on the last line of the range, the one GitHub anchors to.
    assert_eq!(rows[0].line, 12);
  }

  #[test]
  fn the_review_is_named_by_any_of_its_comments() {
    let mut published = comment(2, "src/b.rs", Some(4), "already on GitHub");
    published.is_pending = false;
    published.pull_request_review_node_id = Some("other-review".to_string());

    assert_eq!(
      pending_review_id(&[published.clone(), comment(1, "src/a.rs", Some(9), "draft")]),
      Some("review-node".to_string())
    );
    // Nothing pending: the decision would go out on its own.
    assert_eq!(pending_review_id(&[published]), None);
  }

  #[test]
  fn a_comment_is_deleted_by_its_node_id() {
    let comments = [comment(1, "src/a.rs", Some(9), "draft")];

    assert_eq!(
      pending_review_comment_node_id(&comments, 1),
      Some("node-1".to_string())
    );
    assert_eq!(pending_review_comment_node_id(&comments, 2), None);
  }
}
