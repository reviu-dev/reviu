use crate::command_palette::{CommandPaletteAction, CommandPaletteGithubRepoTab};

#[derive(Clone, Debug, PartialEq, Eq)]
enum GithubUrlTarget {
  Profile {
    login: String,
  },
  Repo {
    owner: String,
    repo: String,
    tab: Option<CommandPaletteGithubRepoTab>,
    issue_number: Option<u64>,
    issue_comment_id: Option<u64>,
  },
  PullRequest {
    owner: String,
    repo: String,
    number: u64,
    open_changes_tab: bool,
    review_comment_id: Option<u64>,
  },
  Commit {
    owner: String,
    repo: String,
    sha: String,
  },
}

pub fn parse_github_url_action(url: &str) -> Option<CommandPaletteAction> {
  parse_github_url_target(url).map(github_url_target_to_action)
}

fn github_url_target_to_action(target: GithubUrlTarget) -> CommandPaletteAction {
  match target {
    GithubUrlTarget::Profile { login } => CommandPaletteAction::OpenGithubProfile { login },
    GithubUrlTarget::Repo {
      owner,
      repo,
      tab,
      issue_number,
      issue_comment_id,
    } => CommandPaletteAction::OpenGithubRepoDetails {
      owner,
      repo,
      tab,
      issue_number,
      issue_comment_id,
    },
    GithubUrlTarget::PullRequest {
      owner,
      repo,
      number,
      open_changes_tab,
      review_comment_id,
    } => CommandPaletteAction::OpenGithubPrDetails {
      owner,
      repo,
      number,
      open_changes_tab,
      review_comment_id,
    },
    GithubUrlTarget::Commit { owner, repo, sha } => {
      CommandPaletteAction::OpenGithubCommitDetails { owner, repo, sha }
    }
  }
}

fn parse_github_path_parts(url: &str) -> Option<Vec<String>> {
  let url = url.trim();
  let tail = url
    .strip_prefix("https://github.com/")
    .or_else(|| url.strip_prefix("http://github.com/"))
    .or_else(|| url.strip_prefix("github.com/"))?;
  let tail = tail
    .split('#')
    .next()
    .unwrap_or(tail)
    .split('?')
    .next()
    .unwrap_or(tail);

  let parts = tail
    .split('/')
    .map(str::trim)
    .filter(|part| !part.is_empty())
    .map(ToString::to_string)
    .collect::<Vec<_>>();
  Some(parts)
}

fn parse_github_repository_parts(url: &str) -> Option<(String, String, Vec<String>)> {
  let parts = parse_github_path_parts(url)?;
  if parts.len() < 2 {
    return None;
  }

  let owner = parts[0].clone();
  let repo = parts[1].clone();
  let rest = parts[2..].to_vec();

  Some((owner, repo, rest))
}

fn is_reserved_github_profile_segment(segment: &str) -> bool {
  let segment = segment.to_ascii_lowercase();
  matches!(
    segment.as_str(),
    "about"
      | "apps"
      | "codespaces"
      | "collections"
      | "dashboard"
      | "events"
      | "explore"
      | "features"
      | "issues"
      | "login"
      | "marketplace"
      | "new"
      | "notifications"
      | "orgs"
      | "organizations"
      | "pricing"
      | "pulls"
      | "search"
      | "settings"
      | "sponsors"
      | "topics"
      | "trending"
  )
}

fn parse_github_profile_url(url: &str) -> Option<String> {
  let parts = parse_github_path_parts(url)?;
  let login = parts.first()?.trim();
  if parts.len() != 1 || login.is_empty() || is_reserved_github_profile_segment(login) {
    return None;
  }
  Some(login.to_string())
}

fn parse_github_fragment(url: &str) -> Option<&str> {
  url.split_once('#').map(|(_, fragment)| fragment.trim())
}

fn parse_github_issue_comment_fragment(url: &str) -> Option<u64> {
  let fragment = parse_github_fragment(url)?;
  let fragment = fragment.strip_prefix("issuecomment-")?;
  fragment.parse().ok()
}

fn parse_github_review_comment_fragment(url: &str) -> Option<u64> {
  let fragment = parse_github_fragment(url)?;
  let fragment = fragment
    .strip_prefix("discussion_r")
    .or_else(|| fragment.strip_prefix('r'))?;
  let digits: String = fragment
    .chars()
    .take_while(|ch| ch.is_ascii_digit())
    .collect();
  if digits.is_empty() {
    return None;
  }
  digits.parse().ok()
}

fn parse_github_pull_request_url(url: &str) -> Option<(String, String, u64)> {
  let (owner, repo, path_parts) = parse_github_repository_parts(url)?;
  if path_parts.first().map(|part| part.as_str()) != Some("pull") {
    return None;
  }

  let number: u64 = path_parts.get(1)?.parse().ok()?;
  Some((owner, repo, number))
}

fn parse_github_issue_url(url: &str) -> Option<(String, String, u64)> {
  let (owner, repo, path_parts) = parse_github_repository_parts(url)?;
  if path_parts.first().map(|part| part.as_str()) != Some("issues") {
    return None;
  }

  let number: u64 = path_parts.get(1)?.parse().ok()?;
  Some((owner, repo, number))
}

fn parse_github_commit_url(url: &str) -> Option<(String, String, String)> {
  let (owner, repo, path_parts) = parse_github_repository_parts(url)?;
  if path_parts.first().map(|part| part.as_str()) != Some("commit") {
    return None;
  }

  let sha = path_parts.get(1)?.trim();
  if sha.is_empty() {
    return None;
  }

  Some((owner, repo, sha.to_string()))
}

#[cfg(test)]
fn parse_github_repository_url(url: &str) -> Option<(String, String)> {
  let (owner, repo, _) = parse_github_repository_parts(url)?;
  Some((owner, repo))
}

fn parse_github_url_target(url: &str) -> Option<GithubUrlTarget> {
  if let Some(login) = parse_github_profile_url(url) {
    return Some(GithubUrlTarget::Profile { login });
  }

  if let Some((owner, repo, number)) = parse_github_pull_request_url(url) {
    let (_, _, path_parts) = parse_github_repository_parts(url)?;
    let review_comment_id = parse_github_review_comment_fragment(url);
    let path_target = path_parts.get(2).map(|part| part.as_str());
    let managed_pr_path = matches!(path_target, None | Some("changes") | Some("files"));
    if review_comment_id.is_none() && !managed_pr_path {
      return Some(GithubUrlTarget::Repo {
        owner,
        repo,
        tab: None,
        issue_number: None,
        issue_comment_id: None,
      });
    }
    let open_changes_tab =
      review_comment_id.is_some() || matches!(path_target, Some("changes") | Some("files"));
    return Some(GithubUrlTarget::PullRequest {
      owner,
      repo,
      number,
      open_changes_tab,
      review_comment_id,
    });
  }

  if let Some((owner, repo, number)) = parse_github_issue_url(url) {
    return Some(GithubUrlTarget::Repo {
      owner,
      repo,
      tab: Some(CommandPaletteGithubRepoTab::Issues),
      issue_number: Some(number),
      issue_comment_id: parse_github_issue_comment_fragment(url),
    });
  }

  if let Some((owner, repo, sha)) = parse_github_commit_url(url) {
    return Some(GithubUrlTarget::Commit { owner, repo, sha });
  }

  let (owner, repo, path_parts) = parse_github_repository_parts(url)?;
  let tab = match path_parts.first().map(|part| part.as_str()) {
    Some("pulls") => Some(CommandPaletteGithubRepoTab::PullRequests),
    Some("issues") => Some(CommandPaletteGithubRepoTab::Issues),
    _ => None,
  };
  let issue_number = if tab == Some(CommandPaletteGithubRepoTab::Issues) {
    path_parts.get(1).and_then(|value| value.parse().ok())
  } else {
    None
  };

  Some(GithubUrlTarget::Repo {
    owner,
    repo,
    tab,
    issue_number,
    issue_comment_id: None,
  })
}

#[cfg(test)]
mod tests {
  use super::{
    GithubUrlTarget, parse_github_commit_url, parse_github_issue_url, parse_github_profile_url,
    parse_github_pull_request_url, parse_github_repository_url, parse_github_url_action,
    parse_github_url_target,
  };
  use crate::CommandPaletteAction;
  use crate::command_palette::CommandPaletteGithubRepoTab;

  #[test]
  fn parse_github_pull_request_url_accepts_standard_url() {
    let parsed = parse_github_pull_request_url("https://github.com/joris-gallot/guit/pull/23");
    assert_eq!(parsed, Some(("joris-gallot".into(), "guit".into(), 23)));
  }

  #[test]
  fn parse_github_profile_url_accepts_user_home_url() {
    let parsed = parse_github_profile_url("https://github.com/octocat?tab=repositories");
    assert_eq!(parsed, Some("octocat".into()));
  }

  #[test]
  fn parse_github_profile_url_rejects_reserved_github_paths() {
    assert_eq!(parse_github_profile_url("https://github.com/pulls"), None);
    assert_eq!(
      parse_github_profile_url("https://github.com/settings"),
      None
    );
  }

  #[test]
  fn parse_github_pull_request_url_rejects_non_pull_url() {
    let parsed = parse_github_pull_request_url("https://github.com/joris-gallot/guit/issues/23");
    assert_eq!(parsed, None);
  }

  #[test]
  fn parse_github_pull_request_url_accepts_changes_fragment_url() {
    let parsed = parse_github_pull_request_url(
      "https://github.com/joris-gallot/guit/pull/4/changes#diff-914ffa9e8939125aa8bba06dbe2ac48755c94e58e2e6c24aa81d52cfafea0709",
    );
    assert_eq!(parsed, Some(("joris-gallot".into(), "guit".into(), 4)));
  }

  #[test]
  fn parse_github_pull_request_url_accepts_query_params() {
    let parsed = parse_github_pull_request_url(
      "https://github.com/joris-gallot/guit/pull/4?notification_referrer_id=NT_kwDOAAABBBCCC",
    );
    assert_eq!(parsed, Some(("joris-gallot".into(), "guit".into(), 4)));
  }

  #[test]
  fn parse_github_issue_url_accepts_standard_url() {
    let parsed = parse_github_issue_url("https://github.com/joris-gallot/guit/issues/23");
    assert_eq!(parsed, Some(("joris-gallot".into(), "guit".into(), 23)));
  }

  #[test]
  fn parse_github_commit_url_accepts_standard_url() {
    let parsed = parse_github_commit_url(
      "https://github.com/joris-gallot/guit/commit/abc123def456?diff=split",
    );
    assert_eq!(
      parsed,
      Some(("joris-gallot".into(), "guit".into(), "abc123def456".into()))
    );
  }

  #[test]
  fn parse_github_repository_url_accepts_standard_url() {
    let parsed = parse_github_repository_url("https://github.com/joris-gallot/guit");
    assert_eq!(parsed, Some(("joris-gallot".into(), "guit".into())));
  }

  #[test]
  fn parse_github_repository_url_accepts_nested_path_url() {
    let parsed = parse_github_repository_url(
      "https://github.com/joris-gallot/guit/pull/4?notification_referrer_id=NT_kwDOAAABBBCCC",
    );
    assert_eq!(parsed, Some(("joris-gallot".into(), "guit".into())));
  }

  #[test]
  fn parse_github_repository_url_rejects_non_github_url() {
    let parsed = parse_github_repository_url("https://gitlab.com/acme/widget");
    assert_eq!(parsed, None);
  }

  #[test]
  fn parse_github_url_target_returns_pull_request_for_pull_url() {
    let parsed = parse_github_url_target("https://github.com/joris-gallot/guit/pull/4");
    assert!(matches!(
      parsed,
      Some(GithubUrlTarget::PullRequest {
        owner,
        repo,
        number,
        open_changes_tab: false,
        review_comment_id: None
      }) if owner == "joris-gallot" && repo == "guit" && number == 4
    ));
  }

  #[test]
  fn parse_github_url_target_returns_repo_for_repo_url() {
    let parsed = parse_github_url_target("https://github.com/joris-gallot/guit");
    assert!(matches!(
      parsed,
      Some(GithubUrlTarget::Repo {
        owner,
        repo,
        tab: None,
        issue_number: None,
        issue_comment_id: None
      }) if owner == "joris-gallot" && repo == "guit"
    ));
  }

  #[test]
  fn parse_github_url_target_returns_profile_for_user_url() {
    let parsed = parse_github_url_target("https://github.com/octocat");
    assert!(matches!(
      parsed,
      Some(GithubUrlTarget::Profile { login }) if login == "octocat"
    ));
  }

  #[test]
  fn parse_github_url_target_routes_pulls_url_to_repo_pull_requests_tab() {
    let parsed = parse_github_url_target(
      "https://github.com/joris-gallot/guit/pulls?q=sort%3Aupdated-desc+is%3Apr+is%3Aopen",
    );
    assert!(matches!(
      parsed,
      Some(GithubUrlTarget::Repo {
        owner,
        repo,
        tab: Some(CommandPaletteGithubRepoTab::PullRequests),
        issue_number: None,
        issue_comment_id: None
      }) if owner == "joris-gallot" && repo == "guit"
    ));
  }

  #[test]
  fn parse_github_url_target_routes_issues_url_to_repo_issues_tab() {
    let parsed = parse_github_url_target(
      "https://github.com/joris-gallot/guit/issues?q=sort%3Aupdated-desc+is%3Aissue+is%3Aopen",
    );
    assert!(matches!(
      parsed,
      Some(GithubUrlTarget::Repo {
        owner,
        repo,
        tab: Some(CommandPaletteGithubRepoTab::Issues),
        issue_number: None,
        issue_comment_id: None
      }) if owner == "joris-gallot" && repo == "guit"
    ));
  }

  #[test]
  fn parse_github_url_target_routes_issue_details_url_to_repo_issue_sheet_target() {
    let parsed = parse_github_url_target("https://github.com/joris-gallot/guit/issues/23");
    assert!(matches!(
      parsed,
      Some(GithubUrlTarget::Repo {
        owner,
        repo,
        tab: Some(CommandPaletteGithubRepoTab::Issues),
        issue_number: Some(23),
        issue_comment_id: None
      }) if owner == "joris-gallot" && repo == "guit"
    ));
  }

  #[test]
  fn parse_github_url_target_routes_issue_comment_link_to_issue_sheet_comment_target() {
    let parsed = parse_github_url_target(
      "https://github.com/colinhacks/zod/issues/5561#issuecomment-3820640388",
    );
    assert!(matches!(
      parsed,
      Some(GithubUrlTarget::Repo {
        owner,
        repo,
        tab: Some(CommandPaletteGithubRepoTab::Issues),
        issue_number: Some(5561),
        issue_comment_id: Some(3820640388)
      }) if owner == "colinhacks" && repo == "zod"
    ));
  }

  #[test]
  fn parse_github_url_target_routes_commit_url_to_commit_details() {
    let parsed = parse_github_url_target("https://github.com/colinhacks/zod/commit/abc123");
    assert!(matches!(
      parsed,
      Some(GithubUrlTarget::Commit {
        owner,
        repo,
        sha,
      }) if owner == "colinhacks" && repo == "zod" && sha == "abc123"
    ));
  }

  #[test]
  fn parse_github_url_target_routes_pr_review_comment_link_to_changes_tab_target() {
    let parsed =
      parse_github_url_target("https://github.com/colinhacks/zod/pull/5533/changes#r2616576383");
    assert!(matches!(
      parsed,
      Some(GithubUrlTarget::PullRequest {
        owner,
        repo,
        number: 5533,
        open_changes_tab: true,
        review_comment_id: Some(2616576383)
      }) if owner == "colinhacks" && repo == "zod"
    ));
  }

  #[test]
  fn parse_github_url_target_routes_pr_discussion_comment_link_to_changes_tab_target() {
    let parsed =
      parse_github_url_target("https://github.com/colinhacks/zod/pull/5533#discussion_r2616576383");
    assert!(matches!(
      parsed,
      Some(GithubUrlTarget::PullRequest {
        owner,
        repo,
        number: 5533,
        open_changes_tab: true,
        review_comment_id: Some(2616576383)
      }) if owner == "colinhacks" && repo == "zod"
    ));
  }

  #[test]
  fn parse_github_url_target_routes_unmanaged_pr_subpage_to_repo() {
    let parsed = parse_github_url_target("https://github.com/colinhacks/zod/pull/5533/checks");
    assert!(matches!(
      parsed,
      Some(GithubUrlTarget::Repo {
        owner,
        repo,
        tab: None,
        issue_number: None,
        issue_comment_id: None
      }) if owner == "colinhacks" && repo == "zod"
    ));
  }

  #[test]
  fn parse_github_url_action_routes_repo_actions_url_to_repo_details() {
    let action = parse_github_url_action("https://github.com/colinhacks/zod/actions");
    assert!(matches!(
      action,
      Some(CommandPaletteAction::OpenGithubRepoDetails {
        owner,
        repo,
        tab: None,
        issue_number: None,
        issue_comment_id: None
      }) if owner == "colinhacks" && repo == "zod"
    ));
  }

  #[test]
  fn parse_github_url_action_routes_profile_url_to_profile() {
    let action = parse_github_url_action("https://github.com/octocat");
    assert!(matches!(
      action,
      Some(CommandPaletteAction::OpenGithubProfile { login }) if login == "octocat"
    ));
  }

  #[test]
  fn parse_github_url_action_routes_commit_url_to_commit_details() {
    let action = parse_github_url_action("https://github.com/colinhacks/zod/commit/abc123");
    assert!(matches!(
      action,
      Some(CommandPaletteAction::OpenGithubCommitDetails {
        owner,
        repo,
        sha,
      }) if owner == "colinhacks" && repo == "zod" && sha == "abc123"
    ));
  }
}
