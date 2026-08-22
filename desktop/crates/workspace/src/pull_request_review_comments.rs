//! The comments of an unsubmitted pull request review, as the Review panel
//! shows them. GitHub owns them, so they are read back from the API rather than
//! kept next to the local batch.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use editor::{ReviewComment, ReviewCommentSide};
use gfm_markdown_viewer::SuggestionContext;

use crate::api::GithubPullRequestReviewComment;
use crate::date_format::format_relative_time;
use crate::github_shared;
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

/// What GitHub calls each side of a diff.
pub(crate) fn review_comment_side_name(side: ReviewCommentSide) -> &'static str {
  match side {
    ReviewCommentSide::Left => "LEFT",
    ReviewCommentSide::Right => "RIGHT",
  }
}

fn positive_line_number(value: Option<i64>) -> Option<usize> {
  value.and_then(|value| (value > 0).then_some(value as usize))
}

fn line_range(start: Option<i64>, end: Option<i64>) -> Option<(usize, usize)> {
  let (start, end) = match (positive_line_number(start), positive_line_number(end)) {
    (Some(start), Some(end)) => (start, end),
    (Some(only), None) | (None, Some(only)) => (only, only),
    (None, None) => return None,
  };
  Some(if start <= end {
    (start, end)
  } else {
    (end, start)
  })
}

fn suggestion_context(comment: &GithubPullRequestReviewComment) -> Option<SuggestionContext> {
  let (_, end) = line_range(
    comment.start_line.or(comment.line),
    comment.line.or(comment.start_line),
  )
  .or_else(|| {
    line_range(
      comment.original_start_line.or(comment.original_line),
      comment.original_line.or(comment.original_start_line),
    )
  })?;
  let start_line = comment
    .start_line
    .or(comment.line)
    .or(comment.original_start_line)
    .or(comment.original_line);
  let original = github_shared::extract_original_line_range_from_diff_hunk(
    &comment.diff_hunk,
    start_line,
    end as i64,
  )?;
  Some(SuggestionContext {
    original_start_line: Some(original.start_line),
    suggested_start_line: Some(original.start_line),
    original_lines: original.lines,
    path: Arc::from(comment.path.as_str()),
  })
}

/// Where the card hangs in the diff. A reply carries no anchor of its own, so
/// it borrows the one of the comment it answers.
fn display_anchor(
  comment: &GithubPullRequestReviewComment,
  by_id: &HashMap<u64, &GithubPullRequestReviewComment>,
) -> Option<(usize, ReviewCommentSide, Option<i64>)> {
  let mut line = comment.line.or(comment.start_line);
  let mut side = comment.side.as_deref().or(comment.start_side.as_deref());
  let mut current = Some(comment);
  // A thread deeper than this is a cycle, not a conversation.
  for _ in 0..32 {
    if line.is_some() && side.is_some() {
      break;
    }
    let Some(parent) = current
      .and_then(|comment| comment.in_reply_to_id)
      .and_then(|id| by_id.get(&id).copied())
    else {
      break;
    };
    current = Some(parent);
    line = line.or(parent.line.or(parent.start_line));
    side = side.or(parent.side.as_deref().or(parent.start_side.as_deref()));
  }

  let anchor = positive_line_number(line)?.saturating_sub(1);
  let side = match side {
    Some("LEFT") => ReviewCommentSide::Left,
    _ => ReviewCommentSide::Right,
  };
  Some((anchor, side, line))
}

fn line_label(
  comment: &GithubPullRequestReviewComment,
  resolved_line: Option<i64>,
) -> Option<Arc<str>> {
  let label = match (comment.start_line, comment.line) {
    (Some(start), Some(end)) if start != end => format!("L{start}-{end}"),
    _ => format!(
      "L{}",
      comment.line.or(comment.start_line).or(resolved_line)?
    ),
  };
  Some(Arc::from(label.as_str()))
}

/// The comments of one file, as the diff hangs them: everything GitHub knows
/// about the file, submitted or not.
pub(crate) fn editor_review_comments(
  comments: &[GithubPullRequestReviewComment],
  path: &Path,
) -> Vec<ReviewComment> {
  let by_id = comments
    .iter()
    .map(|comment| (comment.id, comment))
    .collect::<HashMap<_, _>>();

  comments
    .iter()
    .filter(|comment| Path::new(comment.path.as_str()) == path)
    .filter_map(|comment| {
      let (line, side, resolved_line) = display_anchor(comment, &by_id)?;
      Some(ReviewComment {
        id: comment.id,
        in_reply_to_id: comment.in_reply_to_id,
        line,
        side,
        author: Arc::from(comment.user.login.as_str()),
        avatar_url: comment.user.avatar_url.as_deref().map(Arc::from),
        line_label: line_label(comment, resolved_line),
        body: Arc::from(comment.body.as_str()),
        suggestion_context: suggestion_context(comment),
        created_at: Arc::from(format_relative_time(&comment.created_at).to_string()),
        thread_id: (!comment.thread_id.is_empty())
          .then(|| Arc::<str>::from(comment.thread_id.as_str())),
        is_resolved: comment.is_resolved,
        is_outdated: comment.is_outdated,
        viewer_can_resolve: comment.viewer_can_resolve,
        viewer_can_unresolve: comment.viewer_can_unresolve,
        is_pending: comment.is_pending,
      })
    })
    .collect()
}

/// Only your own words are yours to change.
pub(crate) fn editable_comment_ids(
  comments: &[GithubPullRequestReviewComment],
  viewer_login: Option<&str>,
) -> HashSet<u64> {
  let Some(login) = viewer_login else {
    return HashSet::new();
  };
  comments
    .iter()
    .filter(|comment| {
      github_shared::logins_match_case_insensitive(comment.user.login.as_str(), login)
    })
    .map(|comment| comment.id)
    .collect()
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
  fn only_the_open_file_hangs_its_comments_in_the_diff() {
    let comments = [
      comment(1, "src/a.rs", Some(9), "here"),
      comment(2, "src/b.rs", Some(4), "elsewhere"),
    ];

    let rows = editor_review_comments(&comments, Path::new("src/a.rs"));

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, 1);
    // GitHub counts from one, the diff from zero.
    assert_eq!(rows[0].line, 8);
    assert_eq!(rows[0].side, ReviewCommentSide::Right);
    assert_eq!(rows[0].line_label.as_deref(), Some("L9"));
  }

  #[test]
  fn a_reply_hangs_where_the_comment_it_answers_hangs() {
    let mut reply = comment(2, "src/a.rs", None, "agreed");
    reply.in_reply_to_id = Some(1);
    reply.side = None;
    let mut parent = comment(1, "src/a.rs", Some(9), "here");
    parent.side = Some("LEFT".to_string());

    let rows = editor_review_comments(&[parent, reply], Path::new("src/a.rs"));

    let reply = rows.iter().find(|row| row.id == 2).expect("reply");
    assert_eq!(reply.line, 8);
    assert_eq!(reply.side, ReviewCommentSide::Left);
  }

  #[test]
  fn a_range_says_both_of_its_ends() {
    let mut ranged = comment(1, "src/a.rs", Some(12), "range");
    ranged.start_line = Some(10);

    let rows = editor_review_comments(&[ranged], Path::new("src/a.rs"));

    assert_eq!(rows[0].line_label.as_deref(), Some("L10-12"));
  }

  #[test]
  fn only_your_own_comments_are_yours_to_change() {
    let mine = comment(1, "src/a.rs", Some(9), "mine");
    let mut theirs = comment(2, "src/a.rs", Some(4), "theirs");
    theirs.user.login = "ada".to_string();
    let comments = [mine, theirs];

    assert_eq!(
      editable_comment_ids(&comments, Some("OctoCat")),
      HashSet::from([1])
    );
    // Signed out of GitHub: nothing is editable, not everything.
    assert!(editable_comment_ids(&comments, None).is_empty());
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
