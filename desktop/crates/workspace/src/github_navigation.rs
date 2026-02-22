use gpui::{App, Window};
use ui::CommandPaletteGithubRepoTab;

use crate::{
  github_pr_details_page::GithubPrDetailsPageHandle, github_repo_page::GithubRepoPageHandle,
};

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SameRepoIssueLinkNavigation {
  Noop,
  ScrollComment {
    comment_id: u64,
  },
  ReloadIssue {
    issue_number: u64,
    issue_comment_id: Option<u64>,
  },
}

pub(crate) fn same_repo_issue_link_navigation(
  current_issue_number: u64,
  issue_number: u64,
  issue_comment_id: Option<u64>,
) -> SameRepoIssueLinkNavigation {
  if current_issue_number == issue_number {
    if let Some(comment_id) = issue_comment_id {
      SameRepoIssueLinkNavigation::ScrollComment { comment_id }
    } else {
      SameRepoIssueLinkNavigation::Noop
    }
  } else {
    SameRepoIssueLinkNavigation::ReloadIssue {
      issue_number,
      issue_comment_id,
    }
  }
}

pub(crate) fn should_open_externally(window: &Window) -> bool {
  window.modifiers().secondary()
}

pub(crate) fn open_repo_target(
  owner: String,
  repo: String,
  tab: Option<CommandPaletteGithubRepoTab>,
  issue_number: Option<u64>,
  issue_comment_id: Option<u64>,
  cx: &mut App,
) {
  match tab {
    Some(CommandPaletteGithubRepoTab::PullRequests) => {
      GithubRepoPageHandle::show_pull_requests(owner.into(), repo.into(), cx);
    }
    Some(CommandPaletteGithubRepoTab::Issues) => {
      GithubRepoPageHandle::show_issues(
        owner.into(),
        repo.into(),
        issue_number,
        issue_comment_id,
        cx,
      );
    }
    Some(CommandPaletteGithubRepoTab::Overview) | None => {
      GithubRepoPageHandle::show(owner.into(), repo.into(), cx);
    }
  }
}

pub(crate) fn open_pr_target(
  owner: String,
  repo: String,
  number: u64,
  open_changes_tab: bool,
  review_comment_id: Option<u64>,
  return_repo: Option<(String, String)>,
  cx: &mut App,
) {
  if let Some((return_owner, return_repo)) = return_repo {
    GithubPrDetailsPageHandle::show_with_repo_return_open_target(
      owner.into(),
      repo.into(),
      number,
      return_owner.into(),
      return_repo.into(),
      open_changes_tab,
      review_comment_id,
      cx,
    );
  } else {
    GithubPrDetailsPageHandle::show_with_open_target(
      owner.into(),
      repo.into(),
      number,
      open_changes_tab,
      review_comment_id,
      cx,
    );
  }
}

#[cfg(test)]
mod tests {
  use super::{
    SamePrGfmNavigation, SameRepoIssueLinkNavigation, same_pr_gfm_navigation,
    same_repo_issue_link_navigation,
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
  fn same_repo_issue_link_navigation_noops_for_same_issue_without_fragment() {
    let navigation = same_repo_issue_link_navigation(42, 42, None);
    assert_eq!(navigation, SameRepoIssueLinkNavigation::Noop);
  }

  #[test]
  fn same_repo_issue_link_navigation_scrolls_for_same_issue_comment_fragment() {
    let navigation = same_repo_issue_link_navigation(42, 42, Some(99));
    assert_eq!(
      navigation,
      SameRepoIssueLinkNavigation::ScrollComment { comment_id: 99 }
    );
  }

  #[test]
  fn same_repo_issue_link_navigation_reloads_for_other_issue() {
    let navigation = same_repo_issue_link_navigation(42, 77, Some(101));
    assert_eq!(
      navigation,
      SameRepoIssueLinkNavigation::ReloadIssue {
        issue_number: 77,
        issue_comment_id: Some(101),
      }
    );
  }
}
