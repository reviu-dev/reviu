use std::sync::Arc;

use gpui::{App, Hsla, IntoElement, ParentElement as _, SharedString, Styled, div, prelude::*};
use gpui_component::{
  ActiveTheme as _, Icon, IconName, Sizable as _, avatar::Avatar, h_flex, label::Label, tag::Tag,
  v_flex,
};
use time::OffsetDateTime;
use ui::{StatusTag, StatusThemeExt as _, UiIconName};

use crate::{
  api::{ApiClient, GithubPullRequest, GithubPullRequestAuthor, GithubPullRequestStatus},
  date_format::format_relative_time_at,
};

pub(crate) fn make_asset_url_resolver(
  api: &ApiClient,
) -> Arc<dyn Fn(&str) -> Option<String> + Send + Sync> {
  let api = api.clone();
  Arc::new(move |url: &str| api.resolve_github_asset_url(url).ok())
}

/// If the URL is a GitHub user-attachment URL, resolve and open the signed S3 URL.
/// Returns true if the URL was handled.
pub(crate) fn try_open_github_asset_url(url: &str, api: &ApiClient, cx: &mut gpui::App) -> bool {
  if !gfm_markdown_viewer::is_github_user_attachment_url(url) {
    return false;
  }
  // Try a quick blocking resolve — the asset URL resolver cache should already
  // have the signed URL if the image was rendered. If not, this will block briefly
  // (single HTTP request to our backend) which is acceptable for a click action.
  match api.resolve_github_asset_url(url) {
    Ok(signed_url) => cx.open_url(&signed_url),
    Err(_) => cx.open_url(url),
  }
  true
}

pub(crate) fn repo_section_header(label: &SharedString, cx: &App) -> impl IntoElement {
  let theme = cx.theme();
  h_flex()
    .items_center()
    .py_1()
    .px_2()
    .gap_2()
    .rounded(theme.radius)
    .text_sm()
    .bg(theme.sidebar_accent.opacity(0.5))
    .text_color(theme.muted_foreground)
    .child(Icon::new(IconName::Folder))
    .child(
      div()
        .min_w_0()
        .flex_1()
        .child(Label::new(label.clone()).truncate()),
    )
}

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

fn pull_request_status_icon(
  status: GithubPullRequestStatus,
  theme: &gpui_component::Theme,
) -> Icon {
  Icon::new(UiIconName::GitPullRequestArrow)
    .size_3()
    .text_color(pull_request_status_color(status, theme))
}

pub(crate) fn pull_request_label_row(labels: impl IntoIterator<Item = Tag>) -> impl IntoElement {
  h_flex()
    .h_6()
    .items_center()
    .min_w_0()
    .overflow_hidden()
    .gap_1()
    .children(labels)
}

pub(crate) fn pull_request_author_is_bot(author: &GithubPullRequestAuthor) -> bool {
  author.is_bot || author.login.trim().to_ascii_lowercase().ends_with("[bot]")
}

pub(crate) fn pull_request_author_display_name(author: &GithubPullRequestAuthor) -> String {
  let login = author.login.trim();
  let display_name = login
    .strip_suffix("[bot]")
    .unwrap_or(login)
    .trim_end_matches(['-', '_', ' ']);

  if display_name.is_empty() {
    if login.is_empty() {
      "unknown".to_string()
    } else {
      login.to_string()
    }
  } else {
    display_name.to_string()
  }
}

fn pull_request_opened_at(pr: &GithubPullRequest) -> &str {
  let created_at = pr.created_at.trim();
  if created_at.is_empty() {
    pr.updated_at.as_str()
  } else {
    pr.created_at.as_str()
  }
}

fn pull_request_closed_at(pr: &GithubPullRequest) -> &str {
  pr.closed_at
    .as_deref()
    .filter(|value| !value.trim().is_empty())
    .unwrap_or(pr.updated_at.as_str())
}

fn pull_request_merged_at(pr: &GithubPullRequest) -> &str {
  pr.merged_at
    .as_deref()
    .filter(|value| !value.trim().is_empty())
    .unwrap_or(pr.updated_at.as_str())
}

fn pull_request_activity_text_at(pr: &GithubPullRequest, now: OffsetDateTime) -> SharedString {
  match pr.status() {
    GithubPullRequestStatus::Open | GithubPullRequestStatus::Draft => format!(
      "opened {}",
      format_relative_time_at(pull_request_opened_at(pr), now)
    )
    .into(),
    GithubPullRequestStatus::Merged => format!(
      "was merged {}",
      format_relative_time_at(pull_request_merged_at(pr), now)
    )
    .into(),
    GithubPullRequestStatus::Closed => format!(
      "was closed {}",
      format_relative_time_at(pull_request_closed_at(pr), now)
    )
    .into(),
  }
}

pub(crate) fn pull_request_activity_text(pr: &GithubPullRequest) -> SharedString {
  pull_request_activity_text_at(pr, OffsetDateTime::now_utc())
}

fn pull_request_updated_text_at(pr: &GithubPullRequest, now: OffsetDateTime) -> SharedString {
  format!("updated {}", format_relative_time_at(&pr.updated_at, now)).into()
}

pub(crate) fn pull_request_updated_text(pr: &GithubPullRequest) -> SharedString {
  pull_request_updated_text_at(pr, OffsetDateTime::now_utc())
}

fn pull_request_comments_count_text(pr: &GithubPullRequest) -> Option<SharedString> {
  (pr.comments_count > 0).then(|| pr.comments_count.to_string().into())
}

fn pull_request_author_inline(
  author: &GithubPullRequestAuthor,
  theme: &gpui_component::Theme,
) -> impl IntoElement {
  let display_name = pull_request_author_display_name(author);

  if pull_request_author_is_bot(author) {
    h_flex()
      .items_center()
      .gap_1()
      .child(div().text_color(theme.foreground).child(display_name))
      .child(Tag::secondary().small().rounded_full().child("bot"))
  } else {
    h_flex()
      .items_center()
      .gap_1()
      .child(
        Avatar::new()
          .name(display_name.clone())
          .when_some(author.avatar_url.clone(), |this, url| this.src(url))
          .xsmall(),
      )
      .child(div().text_color(theme.foreground).child(display_name))
  }
}

pub(crate) fn pull_request_list_row_body(
  pr: &GithubPullRequest,
  theme: &gpui_component::Theme,
  show_repository: bool,
  show_author: bool,
) -> impl IntoElement {
  let status = pr.status();
  let status_tag = pull_request_status_tag(status, theme);
  let comments_count_text = pull_request_comments_count_text(pr);
  let repo_name = show_repository.then(|| repo_label(&pr.repository.owner, &pr.repository.repo));
  let activity_text = pull_request_activity_text(pr);
  let updated_text = pull_request_updated_text(pr);

  let label_tags = pr.labels.iter().take(4).map(|label| {
    Tag::secondary()
      .small()
      .rounded_full()
      .child(label.name.clone())
  });

  v_flex()
    .gap_1()
    .child(
      h_flex()
        .items_center()
        .gap_2()
        .child(
          h_flex()
            .items_center()
            .gap_2()
            .min_w_0()
            .flex_1()
            .child(pull_request_status_icon(status, theme))
            .child(
              div()
                .min_w_0()
                .flex_1()
                .child(Label::new(pr.title.clone()).truncate()),
            ),
        )
        .when_some(comments_count_text, |this, comments_count_text| {
          this.child(
            h_flex()
              .items_center()
              .gap_1()
              .text_xs()
              .text_color(theme.muted_foreground)
              .child(
                Icon::new(UiIconName::MessageCircle)
                  .size_3()
                  .text_color(theme.muted_foreground),
              )
              .child(comments_count_text),
          )
        })
        .child(status_tag),
    )
    .child(
      h_flex()
        .gap_1()
        .items_center()
        .min_w_0()
        .overflow_hidden()
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(format!("#{}", pr.number))
        .when_some(repo_name, |this, repo_name| this.child(repo_name))
        .when(
          show_author
            && matches!(
              pr.status(),
              GithubPullRequestStatus::Open | GithubPullRequestStatus::Draft
            ),
          |this| {
            this
              .child(activity_text.clone())
              .child("by")
              .child(pull_request_author_inline(&pr.author, theme))
          },
        )
        .when(
          !show_author
            && matches!(
              pr.status(),
              GithubPullRequestStatus::Open | GithubPullRequestStatus::Draft
            ),
          |this| this.child(activity_text.clone()),
        )
        .when(
          show_author
            && matches!(
              pr.status(),
              GithubPullRequestStatus::Closed | GithubPullRequestStatus::Merged
            ),
          |this| {
            this
              .child("by")
              .child(pull_request_author_inline(&pr.author, theme))
              .child(activity_text.clone())
          },
        )
        .when(
          !show_author
            && matches!(
              pr.status(),
              GithubPullRequestStatus::Closed | GithubPullRequestStatus::Merged
            ),
          |this| this.child(activity_text),
        )
        .child("•")
        .child(updated_text),
    )
    .child(pull_request_label_row(label_tags))
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
    pull_request_activity_text_at, pull_request_author_display_name, pull_request_author_is_bot,
    pull_request_comments_count_text, pull_request_list_row_body, pull_request_status_color,
    pull_request_status_label, pull_request_updated_text_at, repo_label, short_sha,
  };
  use crate::api::{
    GithubPullRequest, GithubPullRequestAuthor, GithubPullRequestLabel, GithubPullRequestState,
    GithubPullRequestStatus, GithubRepository,
  };
  use crate::date_format::format_relative_time_at;
  use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, Render, Styled, TestAppContext,
    Window, div,
  };
  use gpui_component::{ActiveTheme as _, v_flex};
  use time::{OffsetDateTime, format_description::well_known::Rfc3339};
  use ui::StatusThemeExt as _;

  fn make_pull_request(labels: &[&str]) -> GithubPullRequest {
    make_pull_request_with_author(
      labels,
      GithubPullRequestAuthor {
        login: "octocat".to_string(),
        avatar_url: None,
        is_bot: false,
      },
    )
  }

  fn make_pull_request_with_author(
    labels: &[&str],
    author: GithubPullRequestAuthor,
  ) -> GithubPullRequest {
    GithubPullRequest {
      number: 7,
      title: "Example PR".to_string(),
      state: GithubPullRequestState::Open,
      created_at: "2026-02-12T12:00:00Z".to_string(),
      closed_at: None,
      merged_at: None,
      draft: false,
      updated_at: "2026-02-14T12:00:00Z".to_string(),
      comments_count: 0,
      author,
      labels: labels
        .iter()
        .map(|label| GithubPullRequestLabel {
          name: (*label).to_string(),
        })
        .collect(),
      repository: GithubRepository {
        owner: "acme".to_string(),
        repo: "portal".to_string(),
      },
    }
  }

  fn init_gpui_test(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
  }

  struct PullRequestRowProbeView {
    labeled: GithubPullRequest,
    unlabeled: GithubPullRequest,
  }

  impl Render for PullRequestRowProbeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
      let theme = cx.theme().clone();

      v_flex()
        .gap_2()
        .child(
          div()
            .debug_selector(|| "labeled".to_string())
            .child(pull_request_list_row_body(
              &self.labeled,
              &theme,
              true,
              true,
            )),
        )
        .child(
          div()
            .debug_selector(|| "unlabeled".to_string())
            .child(pull_request_list_row_body(
              &self.unlabeled,
              &theme,
              true,
              true,
            )),
        )
    }
  }

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

  #[gpui::test]
  fn pull_request_status_color_matches_theme_status_palette(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    cx.update(|cx| {
      let theme = cx.theme().clone();
      assert_eq!(
        pull_request_status_color(GithubPullRequestStatus::Open, &theme),
        theme.status_green()
      );
      assert_eq!(
        pull_request_status_color(GithubPullRequestStatus::Draft, &theme),
        theme.status_gray()
      );
      assert_eq!(
        pull_request_status_color(GithubPullRequestStatus::Merged, &theme),
        theme.status_violet()
      );
      assert_eq!(
        pull_request_status_color(GithubPullRequestStatus::Closed, &theme),
        theme.status_red()
      );
    });
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

  #[test]
  fn pull_request_author_helpers_normalize_bot_login() {
    let author = GithubPullRequestAuthor {
      login: "renovate[bot]".to_string(),
      avatar_url: None,
      is_bot: false,
    };

    assert!(pull_request_author_is_bot(&author));
    assert_eq!(pull_request_author_display_name(&author), "renovate");
  }

  #[test]
  fn pull_request_activity_and_updated_text_follow_status_timestamps() {
    let now = OffsetDateTime::parse("2026-02-15T18:00:00Z", &Rfc3339).expect("parse now");
    assert_eq!(
      format_relative_time_at("2026-02-12T12:00:00Z", now).as_ref(),
      "3 days ago"
    );
    assert_eq!(
      format_relative_time_at("2026-02-14T12:00:00Z", now).as_ref(),
      "yesterday"
    );

    let open = make_pull_request(&[]);
    assert_eq!(
      pull_request_activity_text_at(&open, now).as_ref(),
      "opened 3 days ago"
    );
    assert_eq!(
      pull_request_updated_text_at(&open, now).as_ref(),
      "updated yesterday"
    );

    let merged = GithubPullRequest {
      merged_at: Some("2026-02-12T12:00:00Z".to_string()),
      state: GithubPullRequestState::Closed,
      updated_at: "2026-02-12T12:00:00Z".to_string(),
      ..make_pull_request(&[])
    };
    assert_eq!(
      pull_request_activity_text_at(&merged, now).as_ref(),
      "was merged 3 days ago"
    );
    assert_eq!(
      pull_request_updated_text_at(&merged, now).as_ref(),
      "updated 3 days ago"
    );

    let closed = GithubPullRequest {
      closed_at: Some("2026-02-11T12:00:00Z".to_string()),
      state: GithubPullRequestState::Closed,
      updated_at: "2026-02-11T12:00:00Z".to_string(),
      ..make_pull_request(&[])
    };
    assert_eq!(
      pull_request_activity_text_at(&closed, now).as_ref(),
      "was closed 4 days ago"
    );
    assert_eq!(
      pull_request_updated_text_at(&closed, now).as_ref(),
      "updated 4 days ago"
    );
  }

  #[test]
  fn pull_request_comments_count_text_hides_zero_and_formats_positive_values() {
    assert_eq!(
      pull_request_comments_count_text(&make_pull_request(&[])),
      None
    );

    let with_comments = GithubPullRequest {
      comments_count: 12,
      ..make_pull_request(&[])
    };

    assert_eq!(
      pull_request_comments_count_text(&with_comments)
        .as_ref()
        .map(ToString::to_string),
      Some("12".to_string())
    );
  }

  #[gpui::test]
  fn pull_request_list_row_body_keeps_stable_height_without_labels(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let labeled = make_pull_request(&["bug"]);
    let unlabeled = make_pull_request(&[]);
    let (_view, cx) = cx.add_window_view(|_, _| PullRequestRowProbeView { labeled, unlabeled });

    let labeled_height = cx
      .debug_bounds("labeled")
      .expect("labeled bounds")
      .size
      .height;
    let unlabeled_height = cx
      .debug_bounds("unlabeled")
      .expect("unlabeled bounds")
      .size
      .height;

    assert_eq!(labeled_height, unlabeled_height);
  }
}
