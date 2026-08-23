use std::sync::Arc;

use gfm_markdown_viewer::AssetUrlResolverFn;
use gpui::Hsla;
use ui::StatusThemeExt as _;

use crate::api::{ApiClient, GithubPullRequestState, GithubPullRequestStatus};

pub(crate) fn make_asset_url_resolver(api: &ApiClient) -> Arc<AssetUrlResolverFn> {
  let api = api.clone();
  Arc::new(move |url: &str| api.resolve_github_asset_url(url).ok())
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

pub(crate) fn repo_label(owner: &str, repo: &str) -> String {
  format!("{owner}/{repo}")
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

pub(crate) fn logins_match_case_insensitive(left: &str, right: &str) -> bool {
  left.eq_ignore_ascii_case(right)
}

#[cfg(test)]
mod tests {
  use super::{
    DiffHunkLineKind, extract_original_line_range_from_diff_hunk,
    extract_original_lines_from_diff_hunk, logins_match_case_insensitive, parse_diff_hunk_lines,
    pull_request_status_color, pull_request_status_label, repo_label,
  };
  use crate::api::GithubPullRequestStatus;
  use gpui::TestAppContext;
  use gpui_component::ActiveTheme as _;
  use ui::StatusThemeExt as _;

  fn init_gpui_test(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
  }

  #[test]
  fn repo_label_formats_owner_and_repo() {
    assert_eq!(repo_label("acme", "widget"), "acme/widget");
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
  fn login_comparison_is_case_insensitive() {
    assert!(logins_match_case_insensitive("OctoCat", "octocat"));
    assert!(!logins_match_case_insensitive("octocat", "hubot"));
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
