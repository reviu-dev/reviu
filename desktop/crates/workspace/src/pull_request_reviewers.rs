//! Who is reviewing a pull request and where each of them stands. GitHub says
//! it in two places, the requested reviewers and the submitted reviews, and this
//! puts them back together.

use std::collections::HashSet;

use crate::api::{
  GithubPullRequestFilterOptionUser, GithubPullRequestReview, GithubPullRequestReviewState,
};
use crate::github_shared;

/// A reviewer and where they stand, in the order the panel shows them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReviewerRow {
  pub login: String,
  pub avatar_url: Option<String>,
  pub status: ReviewerStatus,
}

/// Everyone whose opinion is expected or given, each with their latest word.
pub(crate) fn reviewer_rows(
  requested_reviewers: &[GithubPullRequestFilterOptionUser],
  reviews: &[GithubPullRequestReview],
  author_login: &str,
) -> Vec<ReviewerRow> {
  merged_reviewers(requested_reviewers, reviews, author_login)
    .into_iter()
    .map(|reviewer| ReviewerRow {
      status: reviewer_status_for_login(reviews, &reviewer.login, requested_reviewers),
      login: reviewer.login,
      avatar_url: reviewer.avatar_url,
    })
    .collect()
}

/// What the collapsed block says about the review: the bad news first.
pub(crate) fn reviewers_summary_title(reviewers: &[ReviewerRow]) -> String {
  let approved = reviewers
    .iter()
    .filter(|reviewer| reviewer.status == ReviewerStatus::Approved)
    .count();
  let changes_requested = reviewers
    .iter()
    .filter(|reviewer| reviewer.status == ReviewerStatus::ChangesRequested)
    .count();

  if changes_requested > 0 {
    return format!("{changes_requested} asked for changes");
  }
  if !reviewers.is_empty() && approved == reviewers.len() {
    return "Approved".to_string();
  }
  format!("{approved} of {} approved", reviewers.len())
}

impl ReviewerStatus {
  pub(crate) fn label(self) -> &'static str {
    match self {
      ReviewerStatus::Awaiting => "Awaiting review",
      ReviewerStatus::Approved => "Approved",
      ReviewerStatus::Commented => "Commented",
      ReviewerStatus::ChangesRequested => "Changes requested",
    }
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReviewerStatus {
  Awaiting,
  Approved,
  Commented,
  ChangesRequested,
}

pub(crate) fn reviewer_status_for_login(
  reviews: &[GithubPullRequestReview],
  login: &str,
  requested_reviewers: &[GithubPullRequestFilterOptionUser],
) -> ReviewerStatus {
  let mut latest_approved: Option<&str> = None;
  let mut latest_changes: Option<&str> = None;
  let mut latest_comment: Option<&str> = None;

  for review in reviews {
    let Some(user) = review.user.as_ref() else {
      continue;
    };
    if !github_shared::logins_match_case_insensitive(user.login.as_str(), login) {
      continue;
    }
    let Some(submitted_at) = review.submitted_at.as_deref() else {
      continue;
    };
    match review.state {
      GithubPullRequestReviewState::Approved => {
        if latest_approved.is_none_or(|ts| submitted_at > ts) {
          latest_approved = Some(submitted_at);
        }
      }
      GithubPullRequestReviewState::RequestChanges => {
        if latest_changes.is_none_or(|ts| submitted_at > ts) {
          latest_changes = Some(submitted_at);
        }
      }
      GithubPullRequestReviewState::Commented => {
        if latest_comment.is_none_or(|ts| submitted_at > ts) {
          latest_comment = Some(submitted_at);
        }
      }
      GithubPullRequestReviewState::Dismissed | GithubPullRequestReviewState::Pending => {}
    }
  }

  let is_requested = requested_reviewers
    .iter()
    .any(|r| r.login.eq_ignore_ascii_case(login));

  let review_status = match (latest_approved, latest_changes, latest_comment) {
    (Some(approved), Some(changes), _) => {
      if approved > changes {
        ReviewerStatus::Approved
      } else {
        ReviewerStatus::ChangesRequested
      }
    }
    (_, Some(_), _) => ReviewerStatus::ChangesRequested,
    (Some(_), None, _) => ReviewerStatus::Approved,
    (None, None, Some(_)) => ReviewerStatus::Commented,
    _ => return ReviewerStatus::Awaiting,
  };

  // If the reviewer was re-requested after their last review, show as awaiting.
  if is_requested {
    return ReviewerStatus::Awaiting;
  }

  review_status
}

pub(crate) fn merged_reviewers(
  requested_reviewers: &[GithubPullRequestFilterOptionUser],
  reviews: &[GithubPullRequestReview],
  author_login: &str,
) -> Vec<GithubPullRequestFilterOptionUser> {
  let mut reviewers = Vec::new();
  let mut seen: HashSet<String> = HashSet::new();

  for reviewer in requested_reviewers {
    let key = reviewer.login.to_lowercase();
    if github_shared::logins_match_case_insensitive(reviewer.login.as_str(), author_login) {
      continue;
    }
    if seen.insert(key) {
      reviewers.push(reviewer.clone());
    }
  }

  for review in reviews {
    let Some(user) = review.user.as_ref() else {
      continue;
    };
    let key = user.login.to_lowercase();
    if github_shared::logins_match_case_insensitive(user.login.as_str(), author_login) {
      continue;
    }
    if seen.insert(key) {
      reviewers.push(GithubPullRequestFilterOptionUser {
        login: user.login.clone(),
        avatar_url: user.avatar_url.clone(),
      });
    }
  }

  reviewers
}

#[cfg(test)]
mod tests {
  use super::*;

  fn user(login: &str) -> GithubPullRequestFilterOptionUser {
    GithubPullRequestFilterOptionUser {
      login: login.to_string(),
      avatar_url: None,
    }
  }

  fn review(login: &str, state: &str, at: &str) -> GithubPullRequestReview {
    serde_json::from_value(serde_json::json!({
      "id": 1,
      "user": { "login": login, "avatar_url": null },
      "state": state,
      "submitted_at": at,
      "body": null,
      "html_url": "https://github.com/acme/widget/pull/1",
    }))
    .expect("build review")
  }

  #[test]
  fn a_requested_reviewer_who_has_not_answered_is_awaiting() {
    let rows = reviewer_rows(&[user("ada")], &[], "author");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].login, "ada");
    assert_eq!(rows[0].status, ReviewerStatus::Awaiting);
    assert_eq!(rows[0].status.label(), "Awaiting review");
  }

  #[test]
  fn the_latest_word_of_a_reviewer_wins() {
    let reviews = vec![
      review("ada", "APPROVED", "2026-08-01T00:00:00Z"),
      review("ada", "CHANGES_REQUESTED", "2026-08-02T00:00:00Z"),
    ];

    let rows = reviewer_rows(&[], &reviews, "author");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, ReviewerStatus::ChangesRequested);
  }

  #[test]
  fn the_author_never_reviews_their_own_pull_request() {
    let reviews = vec![review("author", "APPROVED", "2026-08-01T00:00:00Z")];

    assert!(reviewer_rows(&[user("author")], &reviews, "author").is_empty());
  }

  #[test]
  fn the_summary_leads_with_the_bad_news() {
    let approved = vec![ReviewerRow {
      login: "ada".to_string(),
      avatar_url: None,
      status: ReviewerStatus::Approved,
    }];
    assert_eq!(reviewers_summary_title(&approved), "Approved");

    let mixed = vec![
      approved[0].clone(),
      ReviewerRow {
        login: "linus".to_string(),
        avatar_url: None,
        status: ReviewerStatus::ChangesRequested,
      },
    ];
    assert_eq!(reviewers_summary_title(&mixed), "1 asked for changes");

    let waiting = vec![ReviewerRow {
      login: "ada".to_string(),
      avatar_url: None,
      status: ReviewerStatus::Awaiting,
    }];
    assert_eq!(reviewers_summary_title(&waiting), "0 of 1 approved");
  }
}
