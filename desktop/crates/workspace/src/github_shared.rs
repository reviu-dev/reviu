use std::path::Path;
use std::sync::Arc;

use gpui::{AppContext as _, Context, Entity, ExternalPaths, Hsla, ParentElement as _, Window};
use gpui_component::{Colorize, Sizable as _, input::TextareaState, tag::Tag};
use ui::{StatusTag, StatusThemeExt as _};

use crate::api::{
  ApiClient, GithubPullRequestLabel, GithubPullRequestState, GithubPullRequestStatus,
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
  // Try a quick blocking resolve, the asset URL resolver cache should already
  // have the signed URL if the image was rendered. If not, this will block briefly
  // (single HTTP request to our backend) which is acceptable for a click action.
  match api.resolve_github_asset_url(url) {
    Ok(signed_url) => cx.open_url(&signed_url),
    Err(_) => cx.open_url(url),
  }
  true
}

pub(crate) fn image_content_type_and_name_for_path(path: &Path) -> Option<(String, String)> {
  let extension = path
    .extension()
    .and_then(|ext| ext.to_str())
    .map(|ext| ext.to_ascii_lowercase())?;
  let content_type = match extension.as_str() {
    "png" => "image/png",
    "jpg" | "jpeg" => "image/jpeg",
    "gif" => "image/gif",
    "webp" => "image/webp",
    _ => return None,
  };
  let file_name = path
    .file_name()
    .and_then(|name| name.to_str())
    .unwrap_or("image")
    .to_string();
  Some((content_type.to_string(), file_name))
}

pub(crate) fn upload_placeholder_id() -> String {
  use std::sync::atomic::{AtomicU64, Ordering};
  static NEXT_ID: AtomicU64 = AtomicU64::new(1);
  let n = NEXT_ID.fetch_add(1, Ordering::Relaxed);
  format!("reviu-upload-{n}")
}

pub(crate) fn replace_placeholder_in_input(
  input: &mut TextareaState,
  placeholder: &str,
  replacement: &str,
  window: &mut Window,
  cx: &mut Context<TextareaState>,
) {
  let current = input.value().to_string();
  let Some(index) = current.find(placeholder) else {
    return;
  };
  let mut next = String::with_capacity(current.len() + replacement.len());
  next.push_str(&current[..index]);
  next.push_str(replacement);
  next.push_str(&current[index + placeholder.len()..]);
  input.set_value(next, window, cx);
}

/// Spawn detached upload tasks for each image path in `paths`. For each image:
/// - inserts an `![Uploading …]()` placeholder at the input's cursor immediately
/// - uploads the bytes via the backend
/// - replaces the placeholder with the final `![image](url)` markdown on success
/// - strips the placeholder and invokes `on_error` on failure
///
/// Non-image paths are skipped silently.
pub(crate) fn upload_dropped_images<V: 'static>(
  paths: &ExternalPaths,
  input: Entity<TextareaState>,
  api: ApiClient,
  on_error: impl Fn(&mut V, String, &mut Context<V>) + Send + 'static + Clone,
  window: &mut Window,
  cx: &mut Context<V>,
) {
  for path in paths.paths() {
    let Some((content_type, file_name)) = image_content_type_and_name_for_path(path) else {
      continue;
    };
    let placeholder_id = upload_placeholder_id();
    let placeholder_markdown = format!(
      "![Uploading {}…](uploading://{})\n",
      file_name, placeholder_id
    );
    input.update(cx, |input, cx| {
      input.insert(placeholder_markdown.clone(), window, cx);
    });
    let path = path.clone();
    let api = api.clone();
    let input = input.clone();
    let window_handle = window.window_handle();
    let on_error = on_error.clone();
    cx.spawn(async move |this, cx| {
      let upload = cx
        .background_spawn(async move {
          let bytes = std::fs::read(&path).map_err(anyhow::Error::from)?;
          api.upload_asset(bytes, &content_type, &file_name)
        })
        .await;
      let _ = this.update(cx, |this, cx| match upload {
        Ok(url) => {
          let markdown = format!("![image]({url})\n");
          let _ = window_handle.update(cx, |_, window, cx| {
            input.update(cx, |input, cx| {
              replace_placeholder_in_input(input, &placeholder_markdown, &markdown, window, cx);
            });
          });
        }
        Err(error) => {
          let _ = window_handle.update(cx, |_, window, cx| {
            input.update(cx, |input, cx| {
              replace_placeholder_in_input(input, &placeholder_markdown, "", window, cx);
            });
          });
          on_error(this, error.to_string(), cx);
          cx.notify();
        }
      });
    })
    .detach();
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DiffHunkLineKind {
  Context,
  Added,
  Removed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DiffHunkLine {
  pub old_line: Option<usize>,
  pub new_line: Option<usize>,
  pub content: String,
  pub kind: DiffHunkLineKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DiffHunkLineRange {
  pub start_line: usize,
  pub lines: Vec<String>,
}

/// Returns the lines on the RIGHT (new) side of a GitHub review comment's
/// `diff_hunk` that span the comment's anchor range (`start_line..=line`).
/// Used to build the "before" side of a `Suggested change` block.
#[cfg(test)]
pub(crate) fn extract_original_lines_from_diff_hunk(
  diff_hunk: &str,
  start_line: Option<i64>,
  line: i64,
) -> Vec<String> {
  extract_original_line_range_from_diff_hunk(diff_hunk, start_line, line)
    .map(|range| range.lines)
    .unwrap_or_default()
}

pub(crate) fn extract_original_line_range_from_diff_hunk(
  diff_hunk: &str,
  start_line: Option<i64>,
  line: i64,
) -> Option<DiffHunkLineRange> {
  let target_start = start_line.unwrap_or(line);
  let target_end = line;
  if target_start <= 0 || target_end < target_start {
    return None;
  }

  let lines = parse_diff_hunk_lines(diff_hunk);
  extract_diff_hunk_line_range_by_side(&lines, target_start, target_end, true)
    .or_else(|| extract_diff_hunk_line_range_by_side(&lines, target_start, target_end, false))
}

fn extract_diff_hunk_line_range_by_side(
  lines: &[DiffHunkLine],
  target_start: i64,
  target_end: i64,
  use_new_side: bool,
) -> Option<DiffHunkLineRange> {
  let mut output = Vec::new();
  let mut first_line = None;
  for line in lines {
    let line_number = if use_new_side {
      line.new_line
    } else {
      line.old_line
    };
    let Some(line_number) = line_number else {
      continue;
    };
    let line_number = line_number as i64;
    if line_number >= target_start && line_number <= target_end {
      first_line.get_or_insert(line_number as usize);
      output.push(line.content.clone());
    }
    if line_number > target_end {
      break;
    }
  }

  Some(DiffHunkLineRange {
    start_line: first_line?,
    lines: output,
  })
}

pub(crate) fn parse_diff_hunk_lines(diff_hunk: &str) -> Vec<DiffHunkLine> {
  let mut iter = diff_hunk.lines();
  let header = iter.next().unwrap_or("");
  let Some((mut old_line, mut new_line)) = parse_diff_hunk_starts(header) else {
    return Vec::new();
  };

  let mut lines = Vec::new();
  for raw in iter {
    if raw.starts_with("\\ No newline") {
      continue;
    }

    if let Some(content) = raw.strip_prefix('-') {
      lines.push(DiffHunkLine {
        old_line: Some(old_line),
        new_line: None,
        content: content.to_string(),
        kind: DiffHunkLineKind::Removed,
      });
      old_line += 1;
    } else if let Some(content) = raw.strip_prefix('+') {
      lines.push(DiffHunkLine {
        old_line: None,
        new_line: Some(new_line),
        content: content.to_string(),
        kind: DiffHunkLineKind::Added,
      });
      new_line += 1;
    } else {
      let content = raw.strip_prefix(' ').unwrap_or(raw);
      lines.push(DiffHunkLine {
        old_line: Some(old_line),
        new_line: Some(new_line),
        content: content.to_string(),
        kind: DiffHunkLineKind::Context,
      });
      old_line += 1;
      new_line += 1;
    }
  }

  lines
}

fn parse_diff_hunk_starts(header: &str) -> Option<(usize, usize)> {
  let old_start = parse_diff_hunk_start(header, '-')?;
  let new_start = parse_diff_hunk_start(header, '+')?;
  Some((old_start, new_start))
}

fn parse_diff_hunk_start(header: &str, marker: char) -> Option<usize> {
  let marker_ix = header.find(marker)?;
  let after_marker = &header[marker_ix + marker.len_utf8()..];
  let end = after_marker.find([' ', ',']).unwrap_or(after_marker.len());
  after_marker[..end].parse::<usize>().ok()
}

pub(crate) fn commit_subject(message: &str) -> String {
  message
    .lines()
    .map(str::trim)
    .find(|line| !line.is_empty())
    .unwrap_or("No commit message")
    .to_string()
}

pub(crate) fn repo_label(owner: &str, repo: &str) -> String {
  format!("{owner}/{repo}")
}

pub(crate) fn pr_url(owner: &str, repo: &str, pr_number: u64) -> String {
  format!("https://github.com/{owner}/{repo}/pull/{pr_number}")
}

fn github_label_color(label: &GithubPullRequestLabel) -> Option<Hsla> {
  let color = label
    .color
    .as_deref()
    .map(str::trim)
    .filter(|color| !color.is_empty())?;
  let hex = if color.starts_with('#') {
    color.to_string()
  } else {
    format!("#{color}")
  };

  <Hsla as gpui_component::Colorize>::parse_hex(&hex).ok()
}

pub(crate) fn github_label_tag(
  label: &GithubPullRequestLabel,
  theme: &gpui_component::Theme,
) -> Tag {
  let tag = if let Some(color) = github_label_color(label) {
    let background = if theme.is_dark() {
      color.mix_oklab(theme.background, 0.22)
    } else {
      color.mix_oklab(theme.background, 0.14)
    };
    let border = if theme.is_dark() {
      color.mix_oklab(theme.border, 0.45)
    } else {
      color.mix_oklab(theme.border, 0.6)
    };
    let foreground = if theme.is_dark() {
      color.mix_oklab(theme.foreground, 0.55)
    } else if color.l > 0.62 {
      color.mix_oklab(theme.foreground, 0.2)
    } else {
      color.mix_oklab(theme.foreground, 0.65)
    };

    Tag::custom(background, foreground, border)
  } else {
    Tag::secondary()
  };

  tag.small().rounded_full().child(label.name.clone())
}

pub(crate) fn resolve_pull_request_status(
  state: &GithubPullRequestState,
  draft: bool,
  merged: bool,
) -> GithubPullRequestStatus {
  if merged {
    GithubPullRequestStatus::Merged
  } else if matches!(state, GithubPullRequestState::Closed) {
    GithubPullRequestStatus::Closed
  } else if draft {
    GithubPullRequestStatus::Draft
  } else {
    GithubPullRequestStatus::Open
  }
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
  StatusTag::new(pull_request_status_color(status, theme))
    .outline()
    .child(pull_request_status_label(status))
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

#[cfg(test)]
mod tests {
  use super::{
    DiffHunkLineKind, extract_original_line_range_from_diff_hunk,
    extract_original_lines_from_diff_hunk, github_label_color, is_unauthorized_error_message,
    line_snippets_from_content, logins_match_case_insensitive, normalize_non_empty_text,
    parse_diff_hunk_lines, pr_url, pull_request_status_color, pull_request_status_label,
    repo_label,
  };
  use crate::api::{GithubPullRequestLabel, GithubPullRequestStatus};
  use gpui::TestAppContext;
  use gpui_component::{ActiveTheme as _, Colorize as _};
  use ui::StatusThemeExt as _;

  fn init_gpui_test(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
  }

  #[test]
  fn repo_label_formats_owner_and_repo() {
    assert_eq!(repo_label("acme", "widget"), "acme/widget");
  }

  #[test]
  fn github_label_color_parses_hex_without_hash() {
    let color = github_label_color(&GithubPullRequestLabel {
      name: "bug".to_string(),
      color: Some("f29513".to_string()),
    })
    .expect("parsed color");

    assert!(color.to_hex().starts_with("#F2951"));
  }

  #[test]
  fn github_label_color_returns_none_for_missing_color() {
    assert!(
      github_label_color(&GithubPullRequestLabel {
        name: "bug".to_string(),
        color: None,
      })
      .is_none()
    );
  }

  #[test]
  fn pr_url_formats_expected_path() {
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
  fn extract_original_lines_from_diff_hunk_single_line() {
    let hunk = "@@ -18,7 +18,7 @@\n context0\n context1\n context2\n-removed\n+changed line\n context3\n context4";
    assert_eq!(
      extract_original_lines_from_diff_hunk(hunk, None, 21),
      vec!["changed line".to_string()]
    );
  }

  #[test]
  fn extract_original_lines_from_diff_hunk_multi_line_range() {
    let hunk = "@@ -10,6 +10,6 @@\n keep\n line-a\n line-b\n line-c\n keep\n keep";
    assert_eq!(
      extract_original_lines_from_diff_hunk(hunk, Some(11), 13),
      vec![
        "line-a".to_string(),
        "line-b".to_string(),
        "line-c".to_string()
      ]
    );
  }

  #[test]
  fn extract_original_lines_from_diff_hunk_returns_empty_on_bad_header() {
    assert_eq!(
      extract_original_lines_from_diff_hunk("no header here", None, 5),
      Vec::<String>::new()
    );
  }

  #[test]
  fn extract_original_lines_from_diff_hunk_skips_old_side() {
    let hunk = "@@ -5,4 +5,3 @@\n kept1\n-dropped\n kept2\n kept3";
    assert_eq!(
      extract_original_lines_from_diff_hunk(hunk, None, 6),
      vec!["kept2".to_string()]
    );
  }

  #[test]
  fn extract_original_line_range_from_diff_hunk_reports_start_line() {
    let hunk = "@@ -10,6 +10,6 @@\n keep\n line-a\n line-b\n line-c\n keep\n keep";
    let range = extract_original_line_range_from_diff_hunk(hunk, Some(11), 13).expect("line range");
    assert_eq!(range.start_line, 11);
    assert_eq!(
      range.lines,
      vec![
        "line-a".to_string(),
        "line-b".to_string(),
        "line-c".to_string()
      ]
    );
  }

  #[test]
  fn extract_original_line_range_from_diff_hunk_falls_back_to_old_side() {
    let hunk = "@@ -12,2 +20,2 @@\n-old current\n+new current\n keep";
    let range = extract_original_line_range_from_diff_hunk(hunk, None, 12).expect("line range");
    assert_eq!(range.start_line, 12);
    assert_eq!(range.lines, vec!["old current".to_string()]);
  }

  #[test]
  fn parse_diff_hunk_lines_tracks_old_and_new_line_numbers() {
    let hunk = "@@ -10,3 +10,4 @@\n context\n-removed\n+added\n+added 2\n context 2";
    let lines = parse_diff_hunk_lines(hunk);

    assert_eq!(lines.len(), 5);
    assert_eq!(lines[0].kind, DiffHunkLineKind::Context);
    assert_eq!(lines[0].old_line, Some(10));
    assert_eq!(lines[0].new_line, Some(10));
    assert_eq!(lines[1].kind, DiffHunkLineKind::Removed);
    assert_eq!(lines[1].old_line, Some(11));
    assert_eq!(lines[1].new_line, None);
    assert_eq!(lines[2].kind, DiffHunkLineKind::Added);
    assert_eq!(lines[2].old_line, None);
    assert_eq!(lines[2].new_line, Some(11));
    assert_eq!(lines[3].new_line, Some(12));
    assert_eq!(lines[4].old_line, Some(12));
    assert_eq!(lines[4].new_line, Some(13));
  }
}
