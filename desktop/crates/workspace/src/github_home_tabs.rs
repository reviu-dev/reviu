use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GithubPullRequestReviewStatus {
  #[default]
  Any,
  None,
  Required,
  Approved,
  ChangesRequested,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubPullRequestSearchFilters {
  #[serde(default)]
  pub repos: Vec<String>,
  #[serde(default)]
  pub labels: Vec<String>,
  #[serde(default)]
  pub authors: Vec<String>,
  #[serde(default)]
  pub assignees: Vec<String>,
  #[serde(default)]
  pub requested_reviewers: Vec<String>,
  #[serde(default)]
  pub review_status: GithubPullRequestReviewStatus,
  #[serde(default = "default_true")]
  pub include_drafts: bool,
}

impl Default for GithubPullRequestSearchFilters {
  fn default() -> Self {
    Self {
      repos: Vec::new(),
      labels: Vec::new(),
      authors: Vec::new(),
      assignees: Vec::new(),
      requested_reviewers: Vec::new(),
      review_status: GithubPullRequestReviewStatus::Any,
      include_drafts: true,
    }
  }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubHomePullRequestTab {
  pub id: String,
  pub name: String,
  pub filters: GithubPullRequestSearchFilters,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct GithubPullRequestFilterOptionLabel {
  pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct GithubPullRequestFilterOptionUser {
  pub login: String,
  #[serde(rename = "avatar_url")]
  pub avatar_url: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct GithubPullRequestFilterOptions {
  #[serde(default)]
  pub labels: Vec<GithubPullRequestFilterOptionLabel>,
  #[serde(default)]
  pub authors: Vec<GithubPullRequestFilterOptionUser>,
  #[serde(default)]
  pub assignees: Vec<GithubPullRequestFilterOptionUser>,
}

fn default_true() -> bool {
  true
}

fn normalize_non_empty_list(values: &[&str]) -> Vec<String> {
  let mut normalized = Vec::new();
  for value in values {
    let trimmed = value.trim();
    if trimmed.is_empty() {
      continue;
    }
    if normalized
      .iter()
      .any(|existing: &String| existing.eq_ignore_ascii_case(trimmed))
    {
      continue;
    }
    normalized.push(trimmed.to_string());
  }
  normalized
}

pub fn seed_github_home_pull_request_tabs() -> Vec<GithubHomePullRequestTab> {
  vec![
    GithubHomePullRequestTab {
      id: "github-home-my-open-prs".to_string(),
      name: "My Open PRs".to_string(),
      filters: GithubPullRequestSearchFilters {
        authors: vec!["@me".to_string()],
        ..GithubPullRequestSearchFilters::default()
      },
    },
    GithubHomePullRequestTab {
      id: "github-home-need-review".to_string(),
      name: "Need Review".to_string(),
      filters: GithubPullRequestSearchFilters {
        requested_reviewers: vec!["@me".to_string()],
        ..GithubPullRequestSearchFilters::default()
      },
    },
  ]
}

pub fn normalize_github_pull_request_filters(
  filters: &GithubPullRequestSearchFilters,
) -> GithubPullRequestSearchFilters {
  GithubPullRequestSearchFilters {
    repos: normalize_non_empty_list(&filters.repos.iter().map(String::as_str).collect::<Vec<_>>()),
    labels: normalize_non_empty_list(
      &filters
        .labels
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>(),
    ),
    authors: normalize_non_empty_list(
      &filters
        .authors
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>(),
    ),
    assignees: normalize_non_empty_list(
      &filters
        .assignees
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>(),
    ),
    requested_reviewers: normalize_non_empty_list(
      &filters
        .requested_reviewers
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>(),
    ),
    review_status: filters.review_status,
    include_drafts: filters.include_drafts,
  }
}

pub fn normalize_github_home_pull_request_tab(
  tab: &GithubHomePullRequestTab,
) -> GithubHomePullRequestTab {
  GithubHomePullRequestTab {
    id: tab.id.trim().to_string(),
    name: tab.name.trim().to_string(),
    filters: normalize_github_pull_request_filters(&tab.filters),
  }
}

pub fn generate_github_home_pull_request_tab_id() -> String {
  let timestamp = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_millis();
  format!("github-home-tab-{timestamp}")
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn seed_github_home_pull_request_tabs_match_expected_defaults() {
    let tabs = seed_github_home_pull_request_tabs();
    assert_eq!(tabs.len(), 2);
    assert_eq!(tabs[0].name, "My Open PRs");
    assert_eq!(tabs[0].filters.authors, vec!["@me".to_string()]);
    assert!(tabs[0].filters.include_drafts);
    assert_eq!(tabs[1].name, "Need Review");
    assert_eq!(tabs[1].filters.requested_reviewers, vec!["@me".to_string()]);
  }

  #[test]
  fn github_pull_request_search_filters_default_includes_drafts() {
    let filters = GithubPullRequestSearchFilters::default();

    assert!(filters.include_drafts);
  }

  #[test]
  fn normalize_github_pull_request_filters_dedupes_and_trims_lists() {
    let filters = normalize_github_pull_request_filters(&GithubPullRequestSearchFilters {
      repos: vec![" acme/reviu ".to_string(), "acme/reviu".to_string()],
      labels: vec![" bug ".to_string(), "bug".to_string()],
      authors: vec![" @me ".to_string(), "@me".to_string()],
      assignees: vec![" alice ".to_string(), "Alice".to_string()],
      requested_reviewers: vec![" bob ".to_string(), "bob".to_string()],
      review_status: GithubPullRequestReviewStatus::Approved,
      include_drafts: false,
    });

    assert_eq!(filters.repos, vec!["acme/reviu".to_string()]);
    assert_eq!(filters.labels, vec!["bug".to_string()]);
    assert_eq!(filters.authors, vec!["@me".to_string()]);
    assert_eq!(filters.assignees, vec!["alice".to_string()]);
    assert_eq!(filters.requested_reviewers, vec!["bob".to_string()]);
    assert_eq!(
      filters.review_status,
      GithubPullRequestReviewStatus::Approved
    );
    assert!(!filters.include_drafts);
  }
}
