use gpui::{Hsla, ParentElement as _};
use ui::{StatusTag, StatusThemeExt as _};

use crate::api::GithubPullRequestStatus;

pub(crate) fn short_sha(sha: &str) -> String {
  sha.chars().take(7).collect()
}

pub(crate) fn repo_label(owner: &str, repo: &str) -> String {
  format!("{owner}/{repo}")
}

pub(crate) fn issue_url(owner: &str, repo: &str, issue_number: u64) -> String {
  format!("https://github.com/{owner}/{repo}/issues/{issue_number}")
}

pub(crate) fn pr_url(owner: &str, repo: &str, pr_number: u64) -> String {
  format!("https://github.com/{owner}/{repo}/pull/{pr_number}")
}

pub(crate) fn pull_request_status_label(status: GithubPullRequestStatus) -> &'static str {
  match status {
    GithubPullRequestStatus::Open => "Open",
    GithubPullRequestStatus::Closed => "Closed",
    GithubPullRequestStatus::Merged => "Merged",
    GithubPullRequestStatus::Draft => "Draft",
  }
}

pub(crate) fn pull_request_status_color(
  status: GithubPullRequestStatus,
  theme: &gpui_component::Theme,
) -> Hsla {
  match status {
    GithubPullRequestStatus::Open => theme.status_green(),
    GithubPullRequestStatus::Closed => theme.status_red(),
    GithubPullRequestStatus::Merged => theme.status_violet(),
    GithubPullRequestStatus::Draft => theme.status_gray(),
  }
}

pub(crate) fn pull_request_status_tag(
  status: GithubPullRequestStatus,
  theme: &gpui_component::Theme,
) -> StatusTag {
  StatusTag::new(pull_request_status_color(status, theme)).child(pull_request_status_label(status))
}

pub(crate) fn line_snippets_from_content(
  content: &str,
  start_line: usize,
  end_line: usize,
) -> Option<Vec<String>> {
  if start_line == 0 || end_line == 0 {
    return None;
  }
  let (start_line, end_line) = if start_line <= end_line {
    (start_line, end_line)
  } else {
    (end_line, start_line)
  };

  let lines: Vec<&str> = content.split('\n').collect();
  if start_line > lines.len() {
    return None;
  }

  let end_index = end_line.min(lines.len());
  if end_index < start_line {
    return None;
  }

  Some(
    lines[start_line.saturating_sub(1)..end_index]
      .iter()
      .map(|line| line.trim_end_matches('\r').to_string())
      .collect(),
  )
}

pub(crate) fn is_unauthorized_error_message(error: &str) -> bool {
  error.to_ascii_lowercase().contains("unauthorized")
}

pub(crate) fn logins_match_case_insensitive(left: &str, right: &str) -> bool {
  left.eq_ignore_ascii_case(right)
}

pub(crate) fn normalize_non_empty_text(value: &str) -> Option<String> {
  let trimmed = value.trim();
  if trimmed.is_empty() {
    None
  } else {
    Some(trimmed.to_string())
  }
}

pub(crate) fn next_trimmed_text_update(raw_value: &str, initial_value: &str) -> Option<String> {
  let next_value = raw_value.trim();
  if next_value == initial_value.trim() {
    None
  } else {
    Some(next_value.to_string())
  }
}

#[cfg(test)]
mod tests {
  use super::{
    is_unauthorized_error_message, issue_url, line_snippets_from_content,
    logins_match_case_insensitive, next_trimmed_text_update, normalize_non_empty_text, pr_url,
    pull_request_status_label, repo_label, short_sha,
  };
  use crate::api::GithubPullRequestStatus;

  #[test]
  fn short_sha_truncates_to_seven_characters() {
    assert_eq!(short_sha("1234567890abcdef"), "1234567");
  }

  #[test]
  fn short_sha_keeps_short_values_and_empty_string() {
    assert_eq!(short_sha("abc"), "abc");
    assert_eq!(short_sha(""), "");
  }

  #[test]
  fn repo_label_formats_owner_and_repo() {
    assert_eq!(repo_label("acme", "widget"), "acme/widget");
  }

  #[test]
  fn issue_and_pr_url_format_expected_paths() {
    assert_eq!(
      issue_url("acme", "widget", 42),
      "https://github.com/acme/widget/issues/42"
    );
    assert_eq!(
      pr_url("acme", "widget", 7),
      "https://github.com/acme/widget/pull/7"
    );
  }

  #[test]
  fn pull_request_status_label_covers_all_variants() {
    assert_eq!(
      pull_request_status_label(GithubPullRequestStatus::Open),
      "Open"
    );
    assert_eq!(
      pull_request_status_label(GithubPullRequestStatus::Closed),
      "Closed"
    );
    assert_eq!(
      pull_request_status_label(GithubPullRequestStatus::Merged),
      "Merged"
    );
    assert_eq!(
      pull_request_status_label(GithubPullRequestStatus::Draft),
      "Draft"
    );
  }

  #[test]
  fn line_snippets_from_content_handles_bounds_order_and_crlf() {
    let content = "first\r\nsecond\r\nthird\r\n";
    assert_eq!(
      line_snippets_from_content(content, 2, 3),
      Some(vec!["second".to_string(), "third".to_string()])
    );
    assert_eq!(
      line_snippets_from_content(content, 3, 2),
      Some(vec!["second".to_string(), "third".to_string()])
    );
    assert_eq!(
      line_snippets_from_content(content, 4, 4),
      Some(vec!["".to_string()])
    );
    assert!(line_snippets_from_content(content, 0, 3).is_none());
    assert!(line_snippets_from_content(content, 10, 10).is_none());
  }

  #[test]
  fn unauthorized_error_detection_is_case_insensitive() {
    assert!(is_unauthorized_error_message("unauthorized"));
    assert!(is_unauthorized_error_message("HTTP 401 Unauthorized"));
    assert!(!is_unauthorized_error_message("unexpected status: 500"));
  }

  #[test]
  fn login_comparison_is_case_insensitive() {
    assert!(logins_match_case_insensitive("OctoCat", "octocat"));
    assert!(!logins_match_case_insensitive("octocat", "hubot"));
  }

  #[test]
  fn normalize_non_empty_text_trims_and_rejects_blank_values() {
    assert_eq!(
      normalize_non_empty_text("  hello world  "),
      Some("hello world".to_string())
    );
    assert_eq!(normalize_non_empty_text(" \n\t "), None);
  }

  #[test]
  fn next_trimmed_text_update_returns_none_when_value_is_unchanged_after_trim() {
    assert_eq!(
      next_trimmed_text_update("  hello world  ", "hello world"),
      None
    );
  }

  #[test]
  fn next_trimmed_text_update_returns_trimmed_value_when_changed() {
    assert_eq!(
      next_trimmed_text_update("  hello reviu  ", "hello world"),
      Some("hello reviu".to_string())
    );
  }

  #[test]
  fn next_trimmed_text_update_allows_empty_string_to_clear_value() {
    assert_eq!(
      next_trimmed_text_update("   ", "hello world"),
      Some(String::new())
    );
  }
}
