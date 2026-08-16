use gpui::{App, Window};
use ui::CommandPaletteGithubRepoTab;

use crate::github_pr_details_page::GithubPrDetailsPageHandle;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SamePrGfmNavigation {
  ShowOverview { switch_to_overview: bool },
  ScrollComment { switch_to_changes: bool },
}

pub(crate) fn same_pr_gfm_navigation(
  active_tab_ix: usize,
  review_comment_id: Option<u64>,
) -> SamePrGfmNavigation {
  if review_comment_id.is_some() {
    SamePrGfmNavigation::ScrollComment {
      switch_to_changes: active_tab_ix != 1,
    }
  } else {
    SamePrGfmNavigation::ShowOverview {
      switch_to_overview: active_tab_ix != 0,
    }
  }
}

pub(crate) fn should_open_externally(window: &Window) -> bool {
  window.modifiers().secondary()
}

pub(crate) fn github_repo_url(
  owner: &str,
  repo: &str,
  tab: Option<CommandPaletteGithubRepoTab>,
  issue_number: Option<u64>,
  issue_comment_id: Option<u64>,
) -> String {
  let base = format!("https://github.com/{owner}/{repo}");
  match tab {
    Some(CommandPaletteGithubRepoTab::PullRequests) => format!("{base}/pulls"),
    Some(CommandPaletteGithubRepoTab::Issues) => match (issue_number, issue_comment_id) {
      (Some(number), Some(comment_id)) => {
        format!("{base}/issues/{number}#issuecomment-{comment_id}")
      }
      (Some(number), None) => format!("{base}/issues/{number}"),
      (None, _) => format!("{base}/issues"),
    },
    Some(CommandPaletteGithubRepoTab::Overview) | None => base,
  }
}

pub(crate) fn github_profile_url(login: &str) -> String {
  format!("https://github.com/{login}")
}

pub(crate) fn github_commit_url(owner: &str, repo: &str, sha: &str) -> String {
  format!("https://github.com/{owner}/{repo}/commit/{sha}")
}

/// The compare page of a branch with the pull request form open: what our own
/// dialog cannot offer (reviewers, labels, projects) lives there.
pub(crate) fn github_compare_url(owner: &str, repo: &str, branch: &str) -> String {
  let branch = encode_ref(branch);
  format!("https://github.com/{owner}/{repo}/compare/{branch}?expand=1")
}

/// Branch names carry slashes, which stay path separators; the rest is escaped.
fn encode_ref(reference: &str) -> String {
  reference
    .split('/')
    .map(|segment| {
      segment
        .chars()
        .map(|character| match character {
          'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '~' => character.to_string(),
          other => {
            let mut buffer = [0u8; 4];
            other
              .encode_utf8(&mut buffer)
              .as_bytes()
              .iter()
              .map(|byte| format!("%{byte:02X}"))
              .collect::<String>()
          }
        })
        .collect::<String>()
    })
    .collect::<Vec<_>>()
    .join("/")
}

pub fn open_compare_target(owner: String, repo: String, branch: String, cx: &mut App) {
  cx.open_url(&github_compare_url(&owner, &repo, &branch));
}

pub fn open_repo_target(
  owner: String,
  repo: String,
  tab: Option<CommandPaletteGithubRepoTab>,
  issue_number: Option<u64>,
  issue_comment_id: Option<u64>,
  cx: &mut App,
) {
  cx.open_url(&github_repo_url(
    &owner,
    &repo,
    tab,
    issue_number,
    issue_comment_id,
  ));
}

pub fn open_profile_target(login: String, cx: &mut App) {
  cx.open_url(&github_profile_url(&login));
}

pub fn open_pr_target(
  owner: String,
  repo: String,
  number: u64,
  open_changes_tab: bool,
  review_comment_id: Option<u64>,
  cx: &mut App,
) {
  GithubPrDetailsPageHandle::show_with_open_target(
    owner.into(),
    repo.into(),
    number,
    open_changes_tab,
    review_comment_id,
    cx,
  );
}

pub fn open_commit_target(owner: String, repo: String, sha: String, cx: &mut App) {
  cx.open_url(&github_commit_url(&owner, &repo, &sha));
}

#[cfg(test)]
mod tests {
  #[test]
  fn the_compare_url_opens_the_pull_request_form() {
    assert_eq!(
      github_compare_url("acme", "widget", "feature"),
      "https://github.com/acme/widget/compare/feature?expand=1"
    );
    // Slashes are path separators; the rest of a branch name is escaped.
    assert_eq!(
      github_compare_url("acme", "widget", "feat/new thing"),
      "https://github.com/acme/widget/compare/feat/new%20thing?expand=1"
    );
    assert_eq!(
      github_compare_url("acme", "widget", "fix/#42"),
      "https://github.com/acme/widget/compare/fix/%2342?expand=1"
    );
  }

  use super::{
    CommandPaletteGithubRepoTab, SamePrGfmNavigation, github_commit_url, github_compare_url,
    github_profile_url, github_repo_url, same_pr_gfm_navigation,
  };

  #[test]
  fn same_pr_gfm_navigation_routes_comment_links_to_changes_and_scroll() {
    let navigation = same_pr_gfm_navigation(0, Some(42));
    assert_eq!(
      navigation,
      SamePrGfmNavigation::ScrollComment {
        switch_to_changes: true,
      }
    );
  }

  #[test]
  fn same_pr_gfm_navigation_routes_non_comment_links_to_overview_without_reload() {
    let already_overview = same_pr_gfm_navigation(0, None);
    assert_eq!(
      already_overview,
      SamePrGfmNavigation::ShowOverview {
        switch_to_overview: false,
      }
    );

    let from_changes = same_pr_gfm_navigation(1, None);
    assert_eq!(
      from_changes,
      SamePrGfmNavigation::ShowOverview {
        switch_to_overview: true,
      }
    );
  }

  #[test]
  fn github_repo_url_targets_repository_home_without_tab() {
    assert_eq!(
      github_repo_url("acme", "reviu", None, None, None),
      "https://github.com/acme/reviu"
    );
    assert_eq!(
      github_repo_url(
        "acme",
        "reviu",
        Some(CommandPaletteGithubRepoTab::Overview),
        None,
        None
      ),
      "https://github.com/acme/reviu"
    );
  }

  #[test]
  fn github_repo_url_targets_pull_requests_and_issues_lists() {
    assert_eq!(
      github_repo_url(
        "acme",
        "reviu",
        Some(CommandPaletteGithubRepoTab::PullRequests),
        None,
        None
      ),
      "https://github.com/acme/reviu/pulls"
    );
    assert_eq!(
      github_repo_url(
        "acme",
        "reviu",
        Some(CommandPaletteGithubRepoTab::Issues),
        None,
        None
      ),
      "https://github.com/acme/reviu/issues"
    );
  }

  #[test]
  fn github_repo_url_targets_issue_and_issue_comment() {
    assert_eq!(
      github_repo_url(
        "acme",
        "reviu",
        Some(CommandPaletteGithubRepoTab::Issues),
        Some(42),
        None
      ),
      "https://github.com/acme/reviu/issues/42"
    );
    assert_eq!(
      github_repo_url(
        "acme",
        "reviu",
        Some(CommandPaletteGithubRepoTab::Issues),
        Some(42),
        Some(99)
      ),
      "https://github.com/acme/reviu/issues/42#issuecomment-99"
    );
  }

  #[test]
  fn github_profile_and_commit_urls_target_github_com() {
    assert_eq!(
      github_profile_url("joris-gallot"),
      "https://github.com/joris-gallot"
    );
    assert_eq!(
      github_commit_url("acme", "reviu", "abc123"),
      "https://github.com/acme/reviu/commit/abc123"
    );
  }
}
