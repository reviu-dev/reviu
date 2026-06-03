use anyhow::Result;
use base64::{Engine as _, engine::general_purpose};
use reqwest::blocking::Client;
use reqwest::{Method, StatusCode};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::{Arc, Mutex};

use crate::AppProfile;
use crate::crash_report::StartupCrashReport;
use crate::github_home_tabs::{
  GithubIssueSearchFilters, GithubIssueSearchSort, GithubPullRequestFilterOptionUser,
  GithubPullRequestFilterOptions, GithubPullRequestSearchFilters, GithubPullRequestSearchSort,
};
use crate::sentry_context;

const DEFAULT_API_BASE_URL: &str = "http://localhost:3000";
const KEYCHAIN_USERNAME: &str = "bearer";

#[derive(Debug)]
pub struct ApiError {
  status: Option<StatusCode>,
  message: String,
}

impl ApiError {
  pub fn status_code_u16(&self) -> Option<u16> {
    self.status.map(|status| status.as_u16())
  }
}

impl fmt::Display for ApiError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "{}", self.message)
  }
}

impl std::error::Error for ApiError {}

#[derive(Debug, Deserialize)]
struct ApiErrorResponse {
  error: String,
}

fn resolve_api_base_url(
  runtime_api_base_url: Option<String>,
  build_api_base_url: Option<&'static str>,
) -> String {
  runtime_api_base_url
    .or_else(|| build_api_base_url.map(str::to_string))
    .unwrap_or_else(|| DEFAULT_API_BASE_URL.to_string())
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubNotificationRepositoryOwner {
  pub login: String,
  #[serde(rename = "avatar_url")]
  pub avatar_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubNotificationRepository {
  pub name: String,
  #[serde(rename = "full_name")]
  pub full_name: String,
  pub owner: Option<GithubNotificationRepositoryOwner>,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubNotificationSubject {
  pub title: String,
  #[serde(rename = "type")]
  pub subject_type: String,
  pub url: Option<String>,
  #[serde(rename = "latest_comment_url")]
  pub latest_comment_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubNotification {
  pub id: String,
  pub repository: GithubNotificationRepository,
  pub subject: GithubNotificationSubject,
  pub reason: String,
  pub unread: bool,
  #[serde(rename = "updated_at")]
  pub updated_at: String,
  #[serde(rename = "last_read_at")]
  pub last_read_at: Option<String>,
  pub url: String,
  #[serde(rename = "subscription_url")]
  pub subscription_url: String,
}

#[derive(Debug, Deserialize)]
struct GithubNotificationsResponse {
  notifications: Vec<GithubNotification>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserRole {
  User,
  Pro,
  Admin,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CustomerStateSubscriptionStatus {
  Active,
  Trialing,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct CustomerStateSubscription {
  pub id: String,
  #[serde(rename = "createdAt")]
  pub created_at: String,
  #[serde(rename = "modifiedAt")]
  pub modified_at: Option<String>,
  pub status: CustomerStateSubscriptionStatus,
  pub amount: i64,
  pub currency: String,
  #[serde(rename = "recurringInterval")]
  pub recurring_interval: String,
  #[serde(rename = "currentPeriodStart")]
  pub current_period_start: String,
  #[serde(rename = "currentPeriodEnd")]
  pub current_period_end: Option<String>,
  #[serde(rename = "trialStart")]
  pub trial_start: Option<String>,
  #[serde(rename = "trialEnd")]
  pub trial_end: Option<String>,
  #[serde(rename = "cancelAtPeriodEnd")]
  pub cancel_at_period_end: bool,
  #[serde(rename = "canceledAt")]
  pub canceled_at: Option<String>,
  #[serde(rename = "startedAt")]
  pub started_at: Option<String>,
  #[serde(rename = "endsAt")]
  pub ends_at: Option<String>,
  #[serde(rename = "productId")]
  pub product_id: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)]
pub struct UserSubscription {
  #[serde(default, rename = "portalUrl")]
  pub portal_url: Option<String>,
  #[serde(default, rename = "activeSubscription")]
  pub active_subscription: Option<CustomerStateSubscription>,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct User {
  pub id: String,
  pub name: String,
  pub email: String,
  #[serde(rename = "emailVerified")]
  pub email_verified: bool,
  pub image: Option<String>,
  #[serde(rename = "githubLogin")]
  pub github_login: Option<String>,
  pub role: UserRole,
  #[serde(default)]
  pub subscription: UserSubscription,
}

impl User {
  pub fn has_pro_access(&self) -> bool {
    matches!(self.role, UserRole::Admin | UserRole::Pro)
      || self.subscription.active_subscription.is_some()
  }

  pub fn should_show_billing_entry(&self) -> bool {
    matches!(self.role, UserRole::Admin) || self.subscription.active_subscription.is_some()
  }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AiProvider {
  Openai,
  Anthropic,
}

impl AiProvider {
  pub fn label(self) -> &'static str {
    match self {
      Self::Openai => "OpenAI",
      Self::Anthropic => "Anthropic",
    }
  }

  pub fn default_model(self) -> &'static str {
    match self {
      Self::Openai => "gpt-5.4-mini",
      Self::Anthropic => "claude-sonnet-4-6",
    }
  }

  pub fn available_models(self) -> &'static [&'static str] {
    match self {
      Self::Openai => &["gpt-5.4", "gpt-5.4-mini", "gpt-5.4-nano", "o4-mini"],
      Self::Anthropic => &["claude-opus-4-7", "claude-sonnet-4-6", "claude-haiku-4-5"],
    }
  }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AiCredentialMode {
  UserKey,
  ReviuManaged,
  Local,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct AiSettings {
  pub configured: bool,
  #[serde(default, rename = "credentialMode")]
  pub credential_mode: Option<AiCredentialMode>,
  #[serde(default)]
  pub provider: Option<AiProvider>,
  #[serde(default)]
  pub model: Option<String>,
  #[serde(default, rename = "apiKeyHint")]
  pub api_key_hint: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AiSettingsResponse {
  pub settings: AiSettings,
}

#[derive(Debug, Serialize)]
struct AiSettingsRequest<'a> {
  provider: AiProvider,
  #[serde(rename = "apiKey")]
  api_key: &'a str,
  #[serde(skip_serializing_if = "Option::is_none")]
  model: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AiPrBriefPriority {
  High,
  Medium,
  Low,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiPrBriefTargetType {
  PrFile,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct AiPrBriefTarget {
  #[serde(rename = "type")]
  pub target_type: AiPrBriefTargetType,
  pub path: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AiPrBriefReviewFirstItem {
  pub path: String,
  pub reason: String,
  pub priority: AiPrBriefPriority,
  #[serde(default)]
  pub target: Option<AiPrBriefTarget>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AiPrBriefRisk {
  pub title: String,
  pub detail: String,
  #[serde(default)]
  pub path: Option<String>,
  #[serde(default)]
  pub target: Option<AiPrBriefTarget>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AiPrBriefBlockerType {
  Check,
  Thread,
  Merge,
  Draft,
  Outdated,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct AiPrBriefBlocker {
  #[serde(rename = "type")]
  pub blocker_type: AiPrBriefBlockerType,
  pub label: String,
  pub detail: String,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct AiPrBrief {
  pub summary: Vec<String>,
  #[serde(rename = "reviewFirst")]
  pub review_first: Vec<AiPrBriefReviewFirstItem>,
  pub risks: Vec<AiPrBriefRisk>,
  pub blockers: Vec<AiPrBriefBlocker>,
  #[serde(rename = "generatedAt")]
  pub generated_at: String,
  pub owner: String,
  pub repo: String,
  #[serde(rename = "pullNumber")]
  pub pull_number: u64,
  #[serde(rename = "headSha")]
  pub head_sha: String,
  #[serde(rename = "contextHash")]
  pub context_hash: String,
  pub provider: AiProvider,
  #[serde(rename = "credentialMode")]
  pub credential_mode: AiCredentialMode,
  pub model: String,
  pub cached: bool,
}

#[derive(Debug, Serialize)]
struct AiPrBriefRequest<'a> {
  owner: &'a str,
  repo: &'a str,
  #[serde(rename = "pullNumber")]
  pull_number: u64,
  #[serde(rename = "forceRefresh")]
  force_refresh: bool,
}

#[derive(Debug, Deserialize)]
struct AiPrBriefResponse {
  brief: AiPrBrief,
}

#[derive(Debug, Deserialize)]
struct AiPrBriefOptionalResponse {
  brief: Option<AiPrBrief>,
}

#[derive(Debug, Serialize)]
struct AiCommitMessageRequest<'a> {
  diff: &'a str,
}

#[derive(Debug, Deserialize)]
struct AiCommitMessageResponse {
  message: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GithubPullRequestLabel {
  pub name: String,
  #[serde(default)]
  pub color: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GithubRepository {
  pub owner: String,
  pub repo: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GithubUserRepository {
  pub owner: String,
  pub repo: String,
  #[serde(rename = "full_name")]
  pub full_name: String,
  pub description: Option<String>,
  pub private: bool,
  #[serde(default, rename = "owner_avatar_url")]
  pub owner_avatar_url: Option<String>,
  #[serde(rename = "updated_at")]
  pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GithubRepositorySearchItem {
  pub owner: String,
  pub name: String,
  #[serde(rename = "full_name")]
  pub full_name: String,
  pub description: Option<String>,
  pub stars: u64,
  pub private: bool,
  #[serde(default, rename = "owner_avatar_url")]
  pub owner_avatar_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubUserOrganization {
  pub login: String,
  #[serde(rename = "avatarUrl")]
  pub avatar_url: Option<String>,
  pub description: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubCreatedRepository {
  pub owner: String,
  pub repo: String,
  #[serde(rename = "full_name")]
  pub full_name: String,
  pub description: Option<String>,
  pub private: bool,
  #[serde(rename = "html_url")]
  pub html_url: String,
}

#[derive(Debug, Clone)]
pub enum CreateRepositoryOwner {
  Viewer,
  Organization(String),
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GithubIssueStateReason {
  Completed,
  Reopened,
  NotPlanned,
  Duplicate,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GithubIssueUser {
  pub login: String,
  pub name: Option<String>,
  #[serde(rename = "avatar_url")]
  pub avatar_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubIssue {
  pub id: u64,
  pub number: u64,
  pub title: String,
  pub state: String,
  pub state_reason: Option<GithubIssueStateReason>,
  #[serde(rename = "created_at")]
  pub created_at: String,
  #[serde(rename = "updated_at")]
  pub updated_at: String,
  #[serde(rename = "closed_at")]
  pub closed_at: Option<String>,
  pub labels: Vec<GithubPullRequestLabel>,
  pub comments_count: u64,
  pub user: Option<GithubIssueUser>,
  pub repository: GithubRepository,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubIssueDetailsComment {
  #[serde(rename = "node_id", default)]
  pub node_id: String,
  #[serde(default)]
  pub reactions: Vec<GithubReactionGroup>,
  pub id: u64,
  pub body: Option<String>,
  #[serde(rename = "created_at")]
  pub created_at: String,
  #[serde(rename = "updated_at")]
  pub updated_at: String,
  pub user: Option<GithubIssueUser>,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubIssueDetails {
  #[serde(rename = "node_id", default)]
  pub node_id: String,
  #[serde(default)]
  pub reactions: Vec<GithubReactionGroup>,
  pub id: u64,
  pub number: u64,
  pub title: String,
  pub body: Option<String>,
  pub state: String,
  pub state_reason: Option<GithubIssueStateReason>,
  #[serde(rename = "created_at")]
  pub created_at: String,
  #[serde(rename = "updated_at")]
  pub updated_at: String,
  #[serde(rename = "closed_at")]
  pub closed_at: Option<String>,
  pub labels: Vec<GithubPullRequestLabel>,
  pub comments: Vec<GithubIssueDetailsComment>,
  pub user: Option<GithubIssueUser>,
  pub repository: GithubRepository,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GithubIssueReferenceTargetKind {
  Issue,
  PullRequest,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct GithubIssueReferenceTarget {
  pub kind: GithubIssueReferenceTargetKind,
  pub number: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubIssueDescriptionUpdate {
  pub id: u64,
  pub number: u64,
  pub body: Option<String>,
  #[serde(rename = "updated_at")]
  pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GithubRepositoryDetailsOwner {
  pub login: String,
  #[serde(rename = "avatar_url")]
  pub avatar_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubRepositoryLicense {
  pub key: String,
  pub name: String,
  #[serde(rename = "spdx_id")]
  pub spdx_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubRepositoryDetails {
  #[serde(rename = "node_id", default)]
  pub node_id: String,
  pub name: String,
  #[serde(rename = "full_name")]
  pub full_name: String,
  #[serde(rename = "viewer_has_starred", default)]
  pub viewer_has_starred: bool,
  pub description: Option<String>,
  pub homepage: Option<String>,
  pub language: Option<String>,
  #[serde(rename = "default_branch")]
  pub default_branch: String,
  #[serde(rename = "stargazers_count")]
  pub stargazers_count: u64,
  #[serde(rename = "forks_count")]
  pub forks_count: u64,
  #[serde(rename = "subscribers_count")]
  pub subscribers_count: u64,
  #[serde(rename = "viewer_subscription_mode", default)]
  pub viewer_subscription_mode: RepoSubscriptionMode,
  pub size: u64,
  #[serde(rename = "pushed_at")]
  pub pushed_at: Option<String>,
  #[serde(rename = "html_url")]
  pub html_url: String,
  pub owner: GithubRepositoryDetailsOwner,
  pub license: Option<GithubRepositoryLicense>,
  #[serde(default)]
  pub languages: Vec<GithubRepositoryLanguage>,
  #[serde(rename = "recent_commits", default)]
  pub recent_commits: Vec<GithubRepositoryCommit>,
  #[serde(default)]
  pub contributors: Vec<GithubRepositoryContributor>,
  #[serde(rename = "contributors_count", default)]
  pub contributors_count: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GithubRepositoryCommit {
  pub sha: String,
  pub message: String,
  #[serde(rename = "committed_at")]
  pub committed_at: String,
  #[serde(rename = "author_login")]
  pub author_login: Option<String>,
  #[serde(rename = "author_avatar_url")]
  pub author_avatar_url: Option<String>,
  #[serde(default)]
  pub authors: Vec<GithubCommitAuthorIdentity>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GithubRepositoryContributor {
  pub login: String,
  #[serde(rename = "avatar_url")]
  pub avatar_url: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GithubStarResult {
  #[serde(rename = "viewer_has_starred")]
  pub viewer_has_starred: bool,
  #[serde(rename = "stargazers_count")]
  pub stargazers_count: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RepoSubscriptionMode {
  #[default]
  Default,
  All,
  Ignore,
}

impl RepoSubscriptionMode {
  pub fn as_str(self) -> &'static str {
    match self {
      Self::Default => "default",
      Self::All => "all",
      Self::Ignore => "ignore",
    }
  }
}

#[derive(Clone, Debug, Deserialize)]
struct RepoSubscriptionStateResponse {
  mode: RepoSubscriptionMode,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubRepositoryLanguage {
  pub name: String,
  pub color: Option<String>,
  pub size: u64,
  pub percentage: f64,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubUserProfileRepository {
  pub owner: String,
  pub repo: String,
  #[serde(rename = "full_name")]
  pub full_name: String,
  pub description: Option<String>,
  pub private: bool,
  pub fork: bool,
  pub archived: bool,
  #[serde(rename = "html_url")]
  pub html_url: String,
  pub language: Option<String>,
  #[serde(rename = "language_color")]
  pub language_color: Option<String>,
  #[serde(rename = "stargazers_count")]
  pub stargazers_count: u64,
  #[serde(rename = "forks_count")]
  pub forks_count: u64,
  #[serde(rename = "updated_at")]
  pub updated_at: String,
  #[serde(rename = "pushed_at")]
  pub pushed_at: Option<String>,
  #[serde(default)]
  pub languages: Vec<GithubRepositoryLanguage>,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubUserProfile {
  pub login: String,
  pub name: Option<String>,
  #[serde(rename = "avatar_url")]
  pub avatar_url: Option<String>,
  pub bio: Option<String>,
  pub company: Option<String>,
  pub location: Option<String>,
  #[serde(rename = "website_url")]
  pub website_url: Option<String>,
  #[serde(rename = "twitter_username")]
  pub twitter_username: Option<String>,
  #[serde(rename = "html_url")]
  pub html_url: String,
  #[serde(rename = "created_at")]
  pub created_at: String,
  #[serde(rename = "followers_count")]
  pub followers_count: u64,
  #[serde(rename = "following_count")]
  pub following_count: u64,
  #[serde(rename = "repositories_count")]
  pub repositories_count: u64,
  #[serde(rename = "repositories_indexed_count")]
  pub repositories_indexed_count: u64,
  #[serde(rename = "repositories_truncated")]
  pub repositories_truncated: bool,
  #[serde(rename = "stargazers_count")]
  pub stargazers_count: u64,
  #[serde(rename = "forks_count")]
  pub forks_count: u64,
  #[serde(default)]
  pub languages: Vec<GithubRepositoryLanguage>,
  #[serde(default)]
  pub repositories: Vec<GithubUserProfileRepository>,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubRepositoryTreeEntry {
  pub path: String,
  pub mode: String,
  #[serde(rename = "type")]
  pub entry_type: String,
  pub sha: String,
  pub size: Option<u64>,
  pub url: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubRepositoryTree {
  pub sha: String,
  pub url: Option<String>,
  pub tree: Vec<GithubRepositoryTreeEntry>,
  pub truncated: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubRepositoryBranchCommit {
  pub sha: String,
  pub url: String,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubRepositoryBranch {
  pub name: String,
  pub commit: GithubRepositoryBranchCommit,
  pub protected: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubRepositoryReadme {
  pub content: Option<String>,
  pub path: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GithubPullRequest {
  pub number: u64,
  pub title: String,
  pub state: GithubPullRequestState,
  #[serde(rename = "created_at", default)]
  pub created_at: String,
  #[serde(rename = "closed_at")]
  pub closed_at: Option<String>,
  #[serde(rename = "merged_at")]
  pub merged_at: Option<String>,
  pub draft: bool,
  #[serde(rename = "updated_at")]
  pub updated_at: String,
  #[serde(rename = "comments_count", default)]
  pub comments_count: u64,
  #[serde(default)]
  pub author: GithubPullRequestAuthor,
  pub labels: Vec<GithubPullRequestLabel>,
  pub repository: GithubRepository,
}

impl GithubPullRequest {
  pub fn status(&self) -> GithubPullRequestStatus {
    crate::github_shared::resolve_pull_request_status(
      &self.state,
      self.draft,
      self.merged_at.is_some(),
    )
  }
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct GithubPullRequestAuthor {
  #[serde(default)]
  pub login: String,
  #[serde(rename = "avatar_url")]
  pub avatar_url: Option<String>,
  #[serde(rename = "is_bot", default)]
  pub is_bot: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubPullRequestCommitUser {
  pub login: String,
  #[serde(rename = "avatar_url")]
  pub avatar_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct GithubCommitAuthorIdentity {
  pub name: Option<String>,
  pub email: Option<String>,
  pub login: Option<String>,
  #[serde(rename = "avatar_url")]
  pub avatar_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GithubPullRequestCommit {
  pub sha: String,
  pub message: String,
  #[serde(rename = "authored_at")]
  pub authored_at: Option<String>,
  #[serde(rename = "committed_at")]
  pub committed_at: Option<String>,
  #[serde(rename = "parent_sha")]
  pub parent_sha: Option<String>,
  pub author: Option<GithubPullRequestCommitUser>,
  pub committer: Option<GithubPullRequestCommitUser>,
  #[serde(default)]
  pub authors: Vec<GithubCommitAuthorIdentity>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GithubCommitUser {
  pub login: String,
  #[serde(rename = "avatar_url")]
  pub avatar_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GithubCommitStats {
  pub additions: u64,
  pub deletions: u64,
  #[allow(dead_code)]
  pub total: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubCommitAssociatedPullRequest {
  pub number: u64,
  pub title: String,
  pub state: String,
  #[serde(rename = "merged_at")]
  pub merged_at: Option<String>,
  #[serde(rename = "html_url")]
  #[allow(dead_code)]
  pub html_url: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GithubCommitDetails {
  pub sha: String,
  pub message: String,
  #[serde(rename = "html_url")]
  pub html_url: String,
  #[serde(rename = "authored_at")]
  pub authored_at: Option<String>,
  #[serde(rename = "committed_at")]
  pub committed_at: Option<String>,
  #[serde(rename = "parent_sha")]
  pub parent_sha: Option<String>,
  pub author: Option<GithubCommitUser>,
  pub committer: Option<GithubCommitUser>,
  #[serde(default)]
  pub authors: Vec<GithubCommitAuthorIdentity>,
  pub stats: Option<GithubCommitStats>,
  pub files: Vec<GithubPullRequestFile>,
  #[serde(rename = "associated_pull_request", default)]
  pub associated_pull_request: Option<GithubCommitAssociatedPullRequest>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GithubPullRequestReviewCommentUser {
  pub login: String,
  #[serde(rename = "avatar_url")]
  pub avatar_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubPullRequestReviewUser {
  pub login: String,
  #[serde(rename = "avatar_url")]
  pub avatar_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubPullRequestIssueCommentUser {
  pub login: String,
  #[serde(rename = "avatar_url")]
  pub avatar_url: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GithubReactionContent {
  Confused,
  Eyes,
  Heart,
  Hooray,
  Laugh,
  Rocket,
  ThumbsDown,
  ThumbsUp,
}

impl GithubReactionContent {
  pub const ALL: [Self; 8] = [
    Self::ThumbsUp,
    Self::Laugh,
    Self::Confused,
    Self::Rocket,
    Self::ThumbsDown,
    Self::Hooray,
    Self::Heart,
    Self::Eyes,
  ];

  pub fn emoji(self) -> &'static str {
    match self {
      Self::ThumbsUp => "👍",
      Self::ThumbsDown => "👎",
      Self::Laugh => "😄",
      Self::Hooray => "🎉",
      Self::Confused => "😕",
      Self::Heart => "❤️",
      Self::Rocket => "🚀",
      Self::Eyes => "👀",
    }
  }

  pub fn label(self) -> &'static str {
    match self {
      Self::ThumbsUp => "Thumbs up",
      Self::ThumbsDown => "Thumbs down",
      Self::Laugh => "Laugh",
      Self::Hooray => "Hooray",
      Self::Confused => "Confused",
      Self::Heart => "Heart",
      Self::Rocket => "Rocket",
      Self::Eyes => "Eyes",
    }
  }
}

#[derive(Clone, Debug, Deserialize)]
pub struct GithubReactionGroup {
  pub content: GithubReactionContent,
  pub count: u64,
  #[serde(rename = "viewer_has_reacted")]
  pub viewer_has_reacted: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubPullRequestIssueComment {
  #[serde(rename = "node_id", default)]
  pub node_id: String,
  #[serde(default)]
  pub reactions: Vec<GithubReactionGroup>,
  pub id: u64,
  pub body: String,
  #[serde(rename = "created_at")]
  pub created_at: String,
  #[serde(rename = "updated_at")]
  pub updated_at: String,
  pub user: Option<GithubPullRequestIssueCommentUser>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GithubPullRequestReviewEvent {
  Comment,
  Approve,
  RequestChanges,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GithubPullRequestReviewState {
  Commented,
  Approved,
  #[serde(rename = "CHANGES_REQUESTED")]
  RequestChanges,
  Dismissed,
  Pending,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubPullRequestReview {
  #[serde(rename = "node_id", default)]
  pub node_id: String,
  #[serde(default)]
  pub reactions: Vec<GithubReactionGroup>,
  pub id: u64,
  pub body: Option<String>,
  pub state: GithubPullRequestReviewState,
  #[serde(rename = "submitted_at")]
  pub submitted_at: Option<String>,
  #[serde(rename = "commit_id")]
  pub commit_id: Option<String>,
  #[serde(rename = "html_url")]
  pub html_url: String,
  pub user: Option<GithubPullRequestReviewUser>,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubPullRequestReviewComment {
  #[serde(rename = "node_id", default)]
  pub node_id: String,
  #[serde(default)]
  pub reactions: Vec<GithubReactionGroup>,
  #[serde(default, rename = "is_outdated")]
  pub is_outdated: bool,
  #[serde(default, rename = "thread_id")]
  pub thread_id: String,
  #[serde(default, rename = "is_resolved")]
  pub is_resolved: bool,
  #[serde(default, rename = "is_collapsed")]
  pub is_collapsed: bool,
  #[serde(default, rename = "viewer_can_resolve")]
  pub viewer_can_resolve: bool,
  #[serde(default, rename = "viewer_can_unresolve")]
  pub viewer_can_unresolve: bool,
  pub id: u64,
  #[serde(rename = "pull_request_review_id")]
  pub pull_request_review_id: Option<u64>,
  #[serde(rename = "diff_hunk")]
  pub diff_hunk: String,
  pub path: String,
  pub position: Option<i64>,
  #[serde(rename = "original_position")]
  pub original_position: Option<i64>,
  #[serde(rename = "commit_id")]
  pub commit_id: String,
  #[serde(rename = "original_commit_id")]
  pub original_commit_id: String,
  #[serde(rename = "in_reply_to_id")]
  pub in_reply_to_id: Option<u64>,
  pub user: GithubPullRequestReviewCommentUser,
  pub body: String,
  #[serde(rename = "created_at")]
  pub created_at: String,
  #[serde(rename = "updated_at")]
  pub updated_at: String,
  #[serde(rename = "start_line")]
  pub start_line: Option<i64>,
  #[serde(rename = "original_start_line")]
  pub original_start_line: Option<i64>,
  #[serde(rename = "start_side")]
  pub start_side: Option<String>,
  pub line: Option<i64>,
  #[serde(rename = "original_line")]
  pub original_line: Option<i64>,
  pub side: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubPullRequestFile {
  pub filename: String,
  pub status: String,
  pub patch: Option<String>,
  #[serde(rename = "previous_filename")]
  pub previous_filename: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum GithubPullRequestState {
  Open,
  Closed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GithubPullRequestStatus {
  Open,
  Closed,
  Merged,
  Draft,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum GithubPullRequestMergeMethod {
  Merge,
  Squash,
  Rebase,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum GithubPullRequestMergeReadinessStatus {
  Checking,
  Ready,
  Blocked,
  Forbidden,
  Draft,
  Closed,
  Merged,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GithubPullRequestDetails {
  #[serde(rename = "node_id")]
  pub node_id: String,
  #[serde(default)]
  pub reactions: Vec<GithubReactionGroup>,
  pub number: u64,
  pub title: String,
  pub state: GithubPullRequestState,
  pub draft: bool,
  #[serde(rename = "created_at")]
  pub created_at: String,
  #[serde(rename = "updated_at")]
  pub updated_at: String,
  #[serde(rename = "merged_at")]
  pub merged_at: Option<String>,
  #[serde(rename = "merge_base_sha")]
  pub merge_base_sha: String,
  #[serde(rename = "base_sha")]
  pub base_sha: String,
  #[serde(rename = "head_sha")]
  pub head_sha: String,
  #[serde(rename = "base_ref_name")]
  pub base_ref_name: String,
  #[serde(rename = "head_ref_name")]
  pub head_ref_name: String,
  pub body: Option<String>,
  pub author: GithubPullRequestAuthor,
  #[serde(default)]
  pub assignees: Vec<GithubPullRequestFilterOptionUser>,
  #[serde(default, rename = "requested_reviewers")]
  pub requested_reviewers: Vec<GithubPullRequestFilterOptionUser>,
  #[allow(dead_code)]
  pub comments: u64,
  #[serde(rename = "review_comments")]
  #[allow(dead_code)]
  pub review_comments: u64,
  #[allow(dead_code)]
  pub commits: u64,
  pub additions: u64,
  pub deletions: u64,
  #[serde(rename = "changed_files")]
  pub changed_files: u64,
  pub labels: Vec<GithubPullRequestLabel>,
  pub repository: GithubRepository,
  #[serde(rename = "head_repository")]
  pub head_repository: Option<GithubRepository>,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubPullRequestAutoMergeEnabledBy {
  pub login: String,
  #[serde(rename = "avatar_url")]
  pub avatar_url: String,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubPullRequestAutoMergeDetails {
  #[serde(rename = "merge_method")]
  pub merge_method: GithubPullRequestMergeMethod,
  #[serde(rename = "commit_headline")]
  pub commit_headline: Option<String>,
  #[serde(rename = "commit_body")]
  pub commit_body: Option<String>,
  #[serde(rename = "enabled_at")]
  pub enabled_at: Option<String>,
  #[serde(rename = "enabled_by")]
  pub enabled_by: Option<GithubPullRequestAutoMergeEnabledBy>,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubPullRequestMergeReadiness {
  pub status: GithubPullRequestMergeReadinessStatus,
  pub message: String,
  #[serde(rename = "current_head_sha")]
  pub current_head_sha: String,
  #[serde(rename = "available_methods")]
  pub available_methods: Vec<GithubPullRequestMergeMethod>,
  #[serde(rename = "default_method")]
  pub default_method: Option<GithubPullRequestMergeMethod>,
  #[serde(rename = "can_merge_now")]
  pub can_merge_now: bool,
  #[serde(rename = "viewer_can_merge")]
  pub viewer_can_merge: bool,
  #[serde(rename = "mergeable_state")]
  pub mergeable_state: Option<String>,
  pub rebaseable: Option<bool>,
  #[serde(rename = "auto_merge_enabled")]
  pub auto_merge_enabled: bool,
  #[serde(default, rename = "auto_merge")]
  pub auto_merge: Option<GithubPullRequestAutoMergeDetails>,
  #[serde(default, rename = "viewer_can_enable_auto_merge")]
  pub viewer_can_enable_auto_merge: bool,
  #[serde(default, rename = "viewer_can_disable_auto_merge")]
  pub viewer_can_disable_auto_merge: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubPullRequestMergeResult {
  pub merged: bool,
  pub sha: String,
  pub message: String,
  pub method: GithubPullRequestMergeMethod,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubSuggestedChangeCommitResult {
  pub sha: String,
  pub url: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum GithubPullRequestChecksRollupState {
  Success,
  Pending,
  Failure,
  Skipped,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubPullRequestWorkflowStep {
  pub number: u64,
  pub name: String,
  pub status: Option<String>,
  pub conclusion: Option<String>,
  pub state: GithubPullRequestChecksRollupState,
  #[serde(rename = "started_at")]
  pub started_at: Option<String>,
  #[serde(rename = "completed_at")]
  pub completed_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubPullRequestWorkflowJob {
  pub id: u64,
  pub name: String,
  pub status: Option<String>,
  pub conclusion: Option<String>,
  pub state: GithubPullRequestChecksRollupState,
  #[serde(rename = "started_at")]
  pub started_at: Option<String>,
  #[serde(rename = "completed_at")]
  pub completed_at: Option<String>,
  #[serde(rename = "html_url")]
  pub html_url: Option<String>,
  pub required: bool,
  #[serde(default, rename = "app_name")]
  pub app_name: Option<String>,
  #[serde(default, rename = "app_slug")]
  pub app_slug: Option<String>,
  #[serde(default, rename = "app_avatar_url")]
  pub app_avatar_url: Option<String>,
  pub steps: Vec<GithubPullRequestWorkflowStep>,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubPullRequestWorkflowRun {
  pub id: u64,
  pub name: Option<String>,
  #[serde(rename = "display_title")]
  pub display_title: Option<String>,
  pub event: String,
  pub status: Option<String>,
  pub conclusion: Option<String>,
  pub state: GithubPullRequestChecksRollupState,
  #[serde(rename = "created_at")]
  pub created_at: String,
  #[serde(rename = "updated_at")]
  pub updated_at: String,
  #[serde(rename = "run_started_at")]
  pub run_started_at: Option<String>,
  #[serde(rename = "run_number")]
  pub run_number: u64,
  #[serde(rename = "run_attempt")]
  pub run_attempt: Option<u64>,
  #[serde(rename = "html_url")]
  pub html_url: Option<String>,
  pub jobs: Vec<GithubPullRequestWorkflowJob>,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubPullRequestCheckRun {
  pub id: u64,
  pub name: String,
  pub status: Option<String>,
  pub conclusion: Option<String>,
  pub state: GithubPullRequestChecksRollupState,
  #[serde(rename = "started_at")]
  pub started_at: Option<String>,
  #[serde(rename = "completed_at")]
  pub completed_at: Option<String>,
  #[serde(rename = "html_url")]
  pub html_url: Option<String>,
  #[serde(rename = "details_url")]
  pub details_url: Option<String>,
  pub required: bool,
  #[serde(rename = "app_name")]
  pub app_name: Option<String>,
  #[serde(rename = "app_slug")]
  pub app_slug: Option<String>,
  #[serde(default, rename = "app_avatar_url")]
  pub app_avatar_url: Option<String>,
  pub title: Option<String>,
  pub summary: Option<String>,
  pub text: Option<String>,
  #[serde(rename = "annotations_count")]
  pub annotations_count: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubPullRequestLegacyStatus {
  pub id: u64,
  pub context: String,
  pub status: String,
  pub state: GithubPullRequestChecksRollupState,
  pub description: Option<String>,
  #[serde(rename = "target_url")]
  pub target_url: Option<String>,
  #[serde(default, rename = "avatar_url")]
  pub avatar_url: Option<String>,
  #[serde(rename = "created_at")]
  pub created_at: String,
  #[serde(rename = "updated_at")]
  pub updated_at: String,
  pub required: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubPullRequestChecksSummary {
  #[serde(rename = "head_sha")]
  pub head_sha: String,
  #[serde(rename = "overall_state")]
  pub overall_state: GithubPullRequestChecksRollupState,
  #[serde(rename = "required_state")]
  pub required_state: GithubPullRequestChecksRollupState,
  #[serde(rename = "total_checks")]
  pub total_checks: u64,
  #[serde(rename = "successful_checks")]
  pub successful_checks: u64,
  #[serde(rename = "failed_checks")]
  pub failed_checks: u64,
  #[serde(rename = "pending_checks")]
  pub pending_checks: u64,
  #[serde(default, rename = "skipped_checks")]
  pub skipped_checks: u64,
  #[serde(rename = "required_checks_total")]
  pub required_checks_total: u64,
  #[serde(rename = "required_checks_passed")]
  pub required_checks_passed: u64,
  #[serde(rename = "required_checks_failed")]
  pub required_checks_failed: u64,
  #[serde(rename = "required_checks_pending")]
  pub required_checks_pending: u64,
  #[serde(default, rename = "required_checks_skipped")]
  pub required_checks_skipped: u64,
  #[serde(rename = "required_contexts")]
  pub required_contexts: Vec<String>,
  #[serde(rename = "missing_required_contexts")]
  pub missing_required_contexts: Vec<String>,
  #[serde(rename = "requires_up_to_date_branch")]
  pub requires_up_to_date_branch: bool,
  #[serde(rename = "actions_runs")]
  pub actions_runs: Vec<GithubPullRequestWorkflowRun>,
  #[serde(rename = "other_checks")]
  pub other_checks: Vec<GithubPullRequestCheckRun>,
  #[serde(rename = "legacy_statuses")]
  pub legacy_statuses: Vec<GithubPullRequestLegacyStatus>,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubPullRequestDescriptionUpdate {
  pub number: u64,
  pub body: Option<String>,
  #[serde(rename = "updated_at")]
  pub updated_at: String,
}

impl GithubPullRequestDetails {
  pub fn status(&self) -> GithubPullRequestStatus {
    crate::github_shared::resolve_pull_request_status(
      &self.state,
      self.draft,
      self.merged_at.is_some(),
    )
  }
}

#[derive(Clone)]
pub struct ApiClient {
  base_url: String,
  client: Client,
  bearer_token: Arc<Mutex<Option<String>>>,
}

#[derive(Debug, Serialize)]
struct ExchangeCodeRequest<'a> {
  code: &'a str,
}

#[derive(Debug, Deserialize)]
struct ExchangeCodeResponse {
  token: String,
}

#[derive(Debug, Serialize)]
struct EmptyRequest {}

#[derive(Debug, Serialize)]
struct CheckoutSubscriptionRequest<'a> {
  slug: &'a str,
  redirect: bool,
}

#[derive(Debug, Deserialize)]
struct CheckoutSubscriptionResponse {
  url: String,
  #[allow(dead_code)]
  redirect: bool,
}

#[derive(Debug, Serialize)]
struct SubmitFeedbackRequest<'a> {
  #[serde(rename = "type")]
  feedback_type: &'a str,
  title: &'a str,
  description: &'a str,
}

#[derive(Debug, Deserialize)]
struct GithubPullRequestsResponse {
  #[serde(rename = "pullRequests")]
  pull_requests: Vec<GithubPullRequest>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubRepositoryPullRequestsResponse {
  #[serde(rename = "pullRequests")]
  pub pull_requests: Vec<GithubPullRequest>,
  #[serde(rename = "pullRequestCount")]
  pub pull_request_count: u64,
  #[serde(default = "default_page")]
  pub page: u64,
  #[serde(rename = "perPage", default = "default_per_page")]
  pub per_page: u64,
  #[serde(rename = "totalPages", default = "default_page")]
  pub total_pages: u64,
}

fn default_page() -> u64 {
  1
}

fn default_per_page() -> u64 {
  30
}

#[derive(Debug, Serialize)]
struct GithubPullRequestSearchRequest<'a> {
  filters: &'a GithubPullRequestSearchFilters,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubRepositoryIssuesResponse {
  pub issues: Vec<GithubIssue>,
  #[serde(rename = "issueCount")]
  pub issue_count: u64,
  #[serde(default = "default_page")]
  pub page: u64,
  #[serde(rename = "perPage", default = "default_per_page")]
  pub per_page: u64,
  #[serde(rename = "totalPages", default = "default_page")]
  pub total_pages: u64,
}

#[derive(Debug, Deserialize)]
struct GithubPullRequestFilterOptionsResponse {
  options: GithubPullRequestFilterOptions,
}

#[derive(Debug, Serialize)]
struct GithubPullRequestFilterOptionsRequest<'a> {
  repos: &'a [String],
}

fn github_pull_request_search_sort_query(sort: GithubPullRequestSearchSort) -> &'static str {
  match sort {
    GithubPullRequestSearchSort::UpdatedDesc => "updated_desc",
    GithubPullRequestSearchSort::CreatedDesc => "created_desc",
    GithubPullRequestSearchSort::CreatedAsc => "created_asc",
    GithubPullRequestSearchSort::CommentsDesc => "comments_desc",
  }
}

fn github_pull_request_review_status_query(
  status: crate::github_home_tabs::GithubPullRequestReviewStatus,
) -> &'static str {
  match status {
    crate::github_home_tabs::GithubPullRequestReviewStatus::Any => "any",
    crate::github_home_tabs::GithubPullRequestReviewStatus::None => "none",
    crate::github_home_tabs::GithubPullRequestReviewStatus::Required => "required",
    crate::github_home_tabs::GithubPullRequestReviewStatus::Approved => "approved",
    crate::github_home_tabs::GithubPullRequestReviewStatus::ChangesRequested => "changes_requested",
  }
}

fn github_repository_pull_request_filter_query(
  filters: &GithubPullRequestSearchFilters,
) -> Vec<(String, String)> {
  let mut query = Vec::new();

  for label in &filters.labels {
    query.push(("label".to_string(), label.clone()));
  }
  for label in &filters.excluded_labels {
    query.push(("exclude_label".to_string(), label.clone()));
  }
  for author in &filters.authors {
    query.push(("author".to_string(), author.clone()));
  }
  for assignee in &filters.assignees {
    query.push(("assignee".to_string(), assignee.clone()));
  }
  for reviewer in &filters.requested_reviewers {
    query.push(("requested_reviewer".to_string(), reviewer.clone()));
  }

  if filters.review_status != crate::github_home_tabs::GithubPullRequestReviewStatus::Any {
    query.push((
      "review_status".to_string(),
      github_pull_request_review_status_query(filters.review_status).to_string(),
    ));
  }

  if !filters.include_drafts {
    query.push(("include_drafts".to_string(), "false".to_string()));
  }

  if let Some(base) = filters.base.as_ref().filter(|base| !base.trim().is_empty()) {
    query.push(("base".to_string(), base.trim().to_string()));
  }

  if filters.sort != GithubPullRequestSearchSort::UpdatedDesc {
    query.push((
      "sort".to_string(),
      github_pull_request_search_sort_query(filters.sort).to_string(),
    ));
  }

  query
}

fn github_issue_search_sort_query(sort: GithubIssueSearchSort) -> &'static str {
  match sort {
    GithubIssueSearchSort::UpdatedDesc => "updated_desc",
    GithubIssueSearchSort::CreatedDesc => "created_desc",
    GithubIssueSearchSort::CreatedAsc => "created_asc",
    GithubIssueSearchSort::CommentsDesc => "comments_desc",
  }
}

fn github_repository_issue_filter_query(
  filters: &GithubIssueSearchFilters,
) -> Vec<(String, String)> {
  let mut query = Vec::new();

  for label in &filters.labels {
    query.push(("label".to_string(), label.clone()));
  }
  for author in &filters.authors {
    query.push(("author".to_string(), author.clone()));
  }
  for assignee in &filters.assignees {
    query.push(("assignee".to_string(), assignee.clone()));
  }

  if filters.sort != GithubIssueSearchSort::UpdatedDesc {
    query.push((
      "sort".to_string(),
      github_issue_search_sort_query(filters.sort).to_string(),
    ));
  }

  query
}

#[derive(Debug, Deserialize)]
struct GithubPullRequestResponse {
  #[serde(rename = "pullRequest")]
  pull_request: Option<GithubPullRequest>,
}

#[derive(Debug, Serialize)]
struct CreateGithubPullRequestRequest<'a> {
  title: &'a str,
  base: &'a str,
  #[serde(skip_serializing_if = "Option::is_none")]
  body: Option<&'a str>,
  #[serde(skip_serializing_if = "Option::is_none")]
  draft: Option<bool>,
}

#[derive(Serialize)]
struct GithubPullRequestStatusMutationRequest<'a> {
  #[serde(rename = "pullRequestId")]
  pull_request_id: &'a str,
}

#[derive(Debug, Serialize)]
struct GithubPullRequestUsersMutationRequest<'a> {
  users: &'a [&'a str],
}

#[derive(Debug, Serialize)]
struct GithubPullRequestLabelsMutationRequest<'a> {
  labels: &'a [&'a str],
}

#[derive(Debug, Deserialize)]
struct GithubUserRepositoriesResponse {
  repositories: Vec<GithubUserRepository>,
}

#[derive(Debug, Deserialize)]
struct GithubRepositorySearchResponse {
  repositories: Vec<GithubRepositorySearchItem>,
}

#[derive(Debug, Deserialize)]
struct GithubUserOrganizationsResponse {
  organizations: Vec<GithubUserOrganization>,
}

#[derive(Debug, Deserialize)]
struct GithubCreatedRepositoryResponse {
  repository: GithubCreatedRepository,
}

#[derive(Debug, Serialize)]
struct CreateRepositoryRequest<'a> {
  name: &'a str,
  #[serde(skip_serializing_if = "Option::is_none")]
  description: Option<&'a str>,
  private: bool,
}

#[derive(Debug, Serialize)]
struct ForkRepositoryRequest<'a> {
  #[serde(skip_serializing_if = "Option::is_none")]
  organization: Option<&'a str>,
  #[serde(skip_serializing_if = "Option::is_none")]
  name: Option<&'a str>,
  #[serde(rename = "defaultBranchOnly")]
  default_branch_only: bool,
}

#[derive(Debug, Deserialize)]
struct GithubIssueDetailsResponse {
  issue: GithubIssueDetails,
}

#[derive(Debug, Deserialize)]
struct GithubIssueReferenceTargetResponse {
  target: GithubIssueReferenceTarget,
}

#[derive(Debug, Deserialize)]
struct GithubIssueCommentResponse {
  comment: GithubIssueDetailsComment,
}

#[derive(Debug, Deserialize)]
struct GithubIssueDescriptionUpdateResponse {
  issue: GithubIssueDescriptionUpdate,
}

#[derive(Debug, Deserialize)]
struct GithubPullRequestDetailsResponse {
  #[serde(rename = "pullRequest")]
  pull_request: GithubPullRequestDetails,
}

#[derive(Debug, Deserialize)]
struct GithubPullRequestDescriptionUpdateResponse {
  #[serde(rename = "pullRequest")]
  pull_request: GithubPullRequestDescriptionUpdate,
}

#[derive(Debug, Deserialize)]
struct GithubPullRequestMergeReadinessResponse {
  #[serde(rename = "mergeReadiness")]
  merge_readiness: GithubPullRequestMergeReadiness,
}

#[derive(Debug, Deserialize)]
struct GithubPullRequestMergeResultResponse {
  #[serde(rename = "mergeResult")]
  merge_result: GithubPullRequestMergeResult,
}

#[derive(Debug, Deserialize)]
struct GithubSuggestedChangeCommitResponse {
  commit: GithubSuggestedChangeCommitResult,
}

#[derive(Debug, Deserialize)]
struct UploadAssetResponse {
  url: String,
}

#[derive(Debug, Deserialize)]
struct GithubReviewThreadResolutionResponse {
  #[allow(dead_code)]
  thread: GithubReviewThreadResolution,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct GithubReviewThreadResolution {
  pub thread_id: String,
  pub is_resolved: bool,
  pub viewer_can_resolve: bool,
  pub viewer_can_unresolve: bool,
}

#[derive(Debug, Deserialize)]
struct GithubAssetResolveResponse {
  url: String,
}

#[derive(Debug, Deserialize)]
struct GithubPullRequestChecksSummaryResponse {
  checks: GithubPullRequestChecksSummary,
}

#[derive(Debug, Deserialize)]
struct GithubPullRequestFilesResponse {
  files: Vec<GithubPullRequestFile>,
}

#[derive(Debug, Deserialize)]
struct GithubPullRequestCommitsResponse {
  commits: Vec<GithubPullRequestCommit>,
}

#[derive(Debug, Deserialize)]
struct GithubCommitDetailsResponse {
  commit: GithubCommitDetails,
}

#[derive(Debug, Deserialize, Default)]
pub struct GithubPullRequestConversationPullRequest {
  #[serde(rename = "node_id")]
  pub node_id: String,
  #[serde(default)]
  pub reactions: Vec<GithubReactionGroup>,
}

#[derive(Debug, Deserialize)]
pub struct GithubPullRequestConversation {
  #[serde(default, rename = "pull_request")]
  pub pull_request: GithubPullRequestConversationPullRequest,
  #[serde(rename = "issue_comments")]
  pub issue_comments: Vec<GithubPullRequestIssueComment>,
  pub reviews: Vec<GithubPullRequestReview>,
  #[serde(rename = "review_comments")]
  pub review_comments: Vec<GithubPullRequestReviewComment>,
}

#[derive(Debug, Deserialize)]
struct GithubPullRequestConversationResponse {
  conversation: GithubPullRequestConversation,
}

#[derive(Debug, Deserialize)]
struct GithubReactionGroupsResponse {
  reactions: Vec<GithubReactionGroup>,
}

#[derive(Debug, Deserialize)]
struct GithubPullRequestCommentResponse {
  comment: GithubPullRequestReviewComment,
}

#[derive(Debug, Deserialize)]
struct GithubPullRequestReviewResponse {
  review: GithubPullRequestReview,
}

#[derive(Debug, Serialize)]
struct UpdateGithubPullRequestCommentRequest<'a> {
  body: &'a str,
}

#[derive(Debug, Serialize)]
struct UpdateGithubIssueCommentRequest<'a> {
  body: &'a str,
}

#[derive(Debug, Serialize)]
struct UpdateGithubDescriptionRequest<'a> {
  body: &'a str,
}

#[derive(Debug, Serialize)]
struct UpdateGithubPullRequestBaseRequest<'a> {
  base: &'a str,
}

#[derive(Debug, Serialize)]
struct GithubReactionMutationRequest<'a> {
  #[serde(rename = "subjectId")]
  subject_id: &'a str,
  content: GithubReactionContent,
}

#[derive(Debug, Serialize)]
struct CreateGithubPullRequestCommentRequest<'a> {
  body: &'a str,
  path: &'a str,
  #[serde(rename = "commitId")]
  commit_id: &'a str,
  line: u64,
  side: &'a str,
  #[serde(rename = "startLine", skip_serializing_if = "Option::is_none")]
  start_line: Option<u64>,
  #[serde(rename = "startSide", skip_serializing_if = "Option::is_none")]
  start_side: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct ReplyGithubPullRequestCommentRequest<'a> {
  body: &'a str,
}

#[derive(Debug, Serialize)]
struct ApplyGithubSuggestedChangeRequest<'a> {
  #[serde(rename = "commitTitle")]
  commit_title: &'a str,
  #[serde(rename = "commitMessage", skip_serializing_if = "Option::is_none")]
  commit_message: Option<&'a str>,
  #[serde(rename = "expectedHeadSha")]
  expected_head_sha: &'a str,
  path: &'a str,
  #[serde(rename = "originalStartLine")]
  original_start_line: usize,
  #[serde(rename = "originalLines")]
  original_lines: &'a [String],
  #[serde(rename = "suggestedLines")]
  suggested_lines: &'a [String],
  #[serde(rename = "includeCoAuthor")]
  include_co_author: bool,
  #[serde(
    rename = "suggestionAuthorLogin",
    skip_serializing_if = "Option::is_none"
  )]
  suggestion_author_login: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct CreateGithubPullRequestReviewRequest<'a> {
  event: GithubPullRequestReviewEvent,
  #[serde(skip_serializing_if = "Option::is_none")]
  body: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct CreateGithubPullRequestMergeRequest<'a> {
  method: GithubPullRequestMergeMethod,
  #[serde(rename = "expectedHeadSha")]
  expected_head_sha: &'a str,
  #[serde(rename = "commitTitle", skip_serializing_if = "Option::is_none")]
  commit_title: Option<&'a str>,
  #[serde(rename = "commitMessage", skip_serializing_if = "Option::is_none")]
  commit_message: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct EnableGithubPullRequestAutoMergeRequest<'a> {
  #[serde(rename = "pullRequestId")]
  pull_request_id: &'a str,
  method: GithubPullRequestMergeMethod,
  #[serde(rename = "commitTitle", skip_serializing_if = "Option::is_none")]
  commit_title: Option<&'a str>,
  #[serde(rename = "commitMessage", skip_serializing_if = "Option::is_none")]
  commit_message: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
struct GithubFileContentResponse {
  content: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GithubFileCommit {
  pub message: Option<String>,
  #[allow(dead_code)]
  pub sha: Option<String>,
  #[allow(dead_code)]
  pub html_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GithubFileAssetResponse {
  #[serde(rename = "contentBase64")]
  content_base64: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct DesktopUpdateCheckResponse {
  #[serde(rename = "updateAvailable")]
  pub update_available: bool,
  #[serde(rename = "forceUpdate")]
  pub force_update: bool,
  #[serde(rename = "currentVersion")]
  pub current_version: String,
  #[serde(rename = "latestVersion")]
  pub latest_version: String,
  #[serde(rename = "minimumSupportedVersion")]
  pub minimum_supported_version: String,
  #[serde(rename = "releaseNotesUrl")]
  pub release_notes_url: String,
  #[serde(default)]
  pub artifact: Option<DesktopUpdateArtifact>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DesktopUpdateArtifact {
  pub url: String,
  pub sha256: String,
  pub size: u64,
}

#[derive(Debug, Serialize)]
struct DesktopUpdateCheckRequest<'a> {
  #[serde(rename = "currentVersion")]
  current_version: &'a str,
  platform: &'a str,
  arch: &'a str,
}

impl ApiClient {
  pub fn new() -> Self {
    let base_url = resolve_api_base_url(
      std::env::var("API_BASE_URL").ok(),
      option_env!("API_BASE_URL"),
    );
    Self::new_with_base_url(base_url)
  }

  pub(crate) fn new_with_base_url(base_url: impl Into<String>) -> Self {
    let base_url = base_url.into();
    let user_agent = format!(
      "Reviu-Desktop/{} ({}; {})",
      crate::app_update::resolved_build_version(env!("CARGO_PKG_VERSION")),
      std::env::consts::OS,
      std::env::consts::ARCH,
    );
    let client = Client::builder()
      .cookie_store(true)
      .user_agent(user_agent)
      .build()
      .expect("failed to build api client");

    Self {
      base_url: base_url.trim_end_matches('/').to_string(),
      client,
      bearer_token: Arc::new(Mutex::new(None)),
    }
  }

  fn get_api_url(&self, path: &str) -> String {
    format!("{}/{}", self.base_url, path.trim_start_matches('/'))
  }

  fn desktop_sign_in_url(&self) -> String {
    self.get_api_url("/auth/desktop/start")
  }

  pub fn keychain_service(&self) -> &str {
    AppProfile::current().keychain_service()
  }

  pub fn keychain_username(&self) -> &'static str {
    KEYCHAIN_USERNAME
  }

  pub fn authed_request(&self, method: Method, path: &str) -> reqwest::blocking::RequestBuilder {
    let mut request = self.client.request(method, self.get_api_url(path));
    if let Some(token) = self.bearer_token() {
      request = request.bearer_auth(token);
    }
    request
  }

  fn record_http_status(method: &str, route: &str, status: StatusCode) {
    sentry_context::record_http_status(method, route, status.as_u16());
  }

  fn api_error_from_response(response: reqwest::blocking::Response) -> anyhow::Error {
    let status = response.status();
    let message = response
      .json::<ApiErrorResponse>()
      .ok()
      .map(|payload| payload.error)
      .unwrap_or_else(|| format!("unexpected status: {}", status));

    ApiError {
      status: Some(status),
      message,
    }
    .into()
  }

  pub fn sign_in_with_github(&self) -> Result<Option<String>> {
    Ok(Some(self.desktop_sign_in_url()))
  }

  pub fn exchange_code_for_token(&self, code: &str) -> Result<String> {
    let url = self.get_api_url("/auth/exchange");
    let response = self
      .client
      .post(url)
      .json(&ExchangeCodeRequest { code })
      .send()?;
    let status = response.status();
    Self::record_http_status("POST", "/auth/exchange", status);
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    let payload = response.json::<ExchangeCodeResponse>()?;
    Ok(payload.token)
  }

  pub fn sign_out(&self) -> Result<()> {
    let response = self
      .authed_request(Method::POST, "/api/auth/sign-out")
      .json(&EmptyRequest {})
      .send()?;
    let status = response.status();
    Self::record_http_status("POST", "/api/auth/sign-out", status);
    if !status.is_success() {
      self.clear_bearer_token();
      anyhow::bail!("unexpected status: {}", status);
    }
    self.clear_bearer_token();
    Ok(())
  }

  pub fn submit_feedback(&self, feedback_type: &str, title: &str, description: &str) -> Result<()> {
    let response = self
      .authed_request(Method::POST, "/feedback")
      .json(&SubmitFeedbackRequest {
        feedback_type,
        title,
        description,
      })
      .send()?;
    let status = response.status();
    Self::record_http_status("POST", "/feedback", status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized");
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    Ok(())
  }

  pub fn submit_crash_report(&self, report: &StartupCrashReport) -> Result<()> {
    let response = self
      .authed_request(Method::POST, "/crash-reports")
      .json(report)
      .send()?;
    let status = response.status();
    Self::record_http_status("POST", "/crash-reports", status);
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    Ok(())
  }

  pub fn set_bearer_token(&self, token: String) {
    if let Ok(mut guard) = self.bearer_token.lock() {
      *guard = Some(token);
    }
  }

  pub fn clear_bearer_token(&self) {
    if let Ok(mut guard) = self.bearer_token.lock() {
      *guard = None;
    }
  }

  fn bearer_token(&self) -> Option<String> {
    self
      .bearer_token
      .lock()
      .ok()
      .and_then(|guard| guard.clone())
  }

  pub fn fetch_me(&self) -> Result<Option<User>> {
    let response = self.authed_request(Method::GET, "/users/me").send()?;
    let status = response.status();
    Self::record_http_status("GET", "/users/me", status);
    if status == StatusCode::UNAUTHORIZED {
      return Ok(None);
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    let user = response.json::<User>()?;
    Ok(Some(user))
  }

  pub fn checkout_subscription(&self, slug: &str) -> Result<String> {
    let response = self
      .authed_request(Method::POST, "/api/auth/checkout")
      .json(&CheckoutSubscriptionRequest {
        slug,
        redirect: false,
      })
      .send()?;
    let status = response.status();
    Self::record_http_status("POST", "/api/auth/checkout", status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    let payload = response.json::<CheckoutSubscriptionResponse>()?;
    if payload.url.trim().is_empty() {
      anyhow::bail!("missing checkout url");
    }
    Ok(payload.url)
  }

  pub fn fetch_ai_settings(&self) -> Result<AiSettings> {
    let response = self.authed_request(Method::GET, "/ai/settings").send()?;
    let status = response.status();
    Self::record_http_status("GET", "/ai/settings", status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    let payload = response.json::<AiSettingsResponse>()?;
    Ok(payload.settings)
  }

  pub fn save_ai_settings(
    &self,
    provider: AiProvider,
    model: Option<&str>,
    api_key: &str,
  ) -> Result<AiSettings> {
    let trimmed_model = model.map(str::trim).filter(|value| !value.is_empty());
    let response = self
      .authed_request(Method::PUT, "/ai/settings")
      .json(&AiSettingsRequest {
        provider,
        api_key: api_key.trim(),
        model: trimmed_model,
      })
      .send()?;
    let status = response.status();
    Self::record_http_status("PUT", "/ai/settings", status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    let payload = response.json::<AiSettingsResponse>()?;
    Ok(payload.settings)
  }

  pub fn delete_ai_settings(&self) -> Result<()> {
    let response = self.authed_request(Method::DELETE, "/ai/settings").send()?;
    let status = response.status();
    Self::record_http_status("DELETE", "/ai/settings", status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    Ok(())
  }

  pub fn fetch_latest_github_pr_brief(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
  ) -> Result<Option<AiPrBrief>> {
    let response = self
      .authed_request(Method::GET, "/ai/github/pr/brief")
      .query(&[
        ("owner", owner.to_string()),
        ("repo", repo.to_string()),
        ("pullNumber", number.to_string()),
      ])
      .send()?;
    let status = response.status();
    Self::record_http_status("GET", "/ai/github/pr/brief", status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    let payload = response.json::<AiPrBriefOptionalResponse>()?;
    Ok(payload.brief)
  }

  pub fn generate_github_pr_brief(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
    force_refresh: bool,
  ) -> Result<AiPrBrief> {
    let response = self
      .authed_request(Method::POST, "/ai/github/pr/brief")
      .json(&AiPrBriefRequest {
        owner,
        repo,
        pull_number: number,
        force_refresh,
      })
      .send()?;
    let status = response.status();
    Self::record_http_status("POST", "/ai/github/pr/brief", status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    let payload = response.json::<AiPrBriefResponse>()?;
    Ok(payload.brief)
  }

  pub fn generate_commit_message(&self, diff: &str) -> Result<String> {
    let response = self
      .authed_request(Method::POST, "/ai/commit-message")
      .json(&AiCommitMessageRequest { diff })
      .send()?;
    let status = response.status();
    Self::record_http_status("POST", "/ai/commit-message", status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    let payload = response.json::<AiCommitMessageResponse>()?;
    Ok(payload.message)
  }

  pub fn fetch_github_pull_requests(
    &self,
    filters: &GithubPullRequestSearchFilters,
  ) -> Result<Vec<GithubPullRequest>> {
    let response = self
      .authed_request(Method::POST, "/github/pr/search")
      .json(&GithubPullRequestSearchRequest { filters })
      .send()?;
    let status = response.status();
    Self::record_http_status("POST", "/github/pr/search", status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    let payload = response.json::<GithubPullRequestsResponse>()?;
    Ok(payload.pull_requests)
  }

  pub fn fetch_github_pull_request_filter_options(
    &self,
    repos: &[String],
  ) -> Result<GithubPullRequestFilterOptions> {
    let response = self
      .authed_request(Method::POST, "/github/pr/filter-options")
      .json(&GithubPullRequestFilterOptionsRequest { repos })
      .send()?;
    let status = response.status();
    Self::record_http_status("POST", "/github/pr/filter-options", status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    let payload = response.json::<GithubPullRequestFilterOptionsResponse>()?;
    Ok(payload.options)
  }

  pub fn fetch_github_user_repositories(&self) -> Result<Vec<GithubUserRepository>> {
    let response = self
      .authed_request(Method::GET, "/github/repos/me")
      .send()?;
    let status = response.status();
    Self::record_http_status("GET", "/github/repos/me", status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    let payload = response.json::<GithubUserRepositoriesResponse>()?;
    Ok(payload.repositories)
  }

  pub fn fetch_github_user_profile(&self, login: &str) -> Result<GithubUserProfile> {
    let route = format!("/github/users/{login}");
    let response = self.authed_request(Method::GET, route.as_str()).send()?;
    let status = response.status();
    Self::record_http_status("GET", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    response.json::<GithubUserProfile>().map_err(Into::into)
  }

  pub fn search_github_repositories(&self, query: &str) -> Result<Vec<GithubRepositorySearchItem>> {
    let response = self
      .authed_request(Method::GET, "/github/search")
      .query(&[("q", query)])
      .send()?;
    let status = response.status();
    Self::record_http_status("GET", "/github/search", status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    let payload = response.json::<GithubRepositorySearchResponse>()?;
    Ok(payload.repositories)
  }

  pub fn fetch_github_user_organizations(&self) -> Result<Vec<GithubUserOrganization>> {
    let response = self
      .authed_request(Method::GET, "/github/user/orgs")
      .send()?;
    let status = response.status();
    Self::record_http_status("GET", "/github/user/orgs", status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    let payload = response.json::<GithubUserOrganizationsResponse>()?;
    Ok(payload.organizations)
  }

  pub fn create_github_repository(
    &self,
    owner: &CreateRepositoryOwner,
    name: &str,
    description: Option<&str>,
    private: bool,
  ) -> Result<GithubCreatedRepository> {
    let route = match owner {
      CreateRepositoryOwner::Viewer => "/github/repos".to_string(),
      CreateRepositoryOwner::Organization(org) => format!("/github/orgs/{org}/repos"),
    };
    let trimmed_description = description.map(str::trim).filter(|v| !v.is_empty());
    let response = self
      .authed_request(Method::POST, route.as_str())
      .json(&CreateRepositoryRequest {
        name: name.trim(),
        description: trimmed_description,
        private,
      })
      .send()?;
    let status = response.status();
    Self::record_http_status("POST", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    let payload = response.json::<GithubCreatedRepositoryResponse>()?;
    Ok(payload.repository)
  }

  pub fn fork_github_repository(
    &self,
    source_owner: &str,
    source_repo: &str,
    target_owner: &CreateRepositoryOwner,
    name: Option<&str>,
    default_branch_only: bool,
  ) -> Result<GithubCreatedRepository> {
    let route = format!("/github/repos/{source_owner}/{source_repo}/forks");
    let organization = match target_owner {
      CreateRepositoryOwner::Viewer => None,
      CreateRepositoryOwner::Organization(org) => Some(org.as_str()),
    };
    let trimmed_name = name.map(str::trim).filter(|v| !v.is_empty());
    let response = self
      .authed_request(Method::POST, route.as_str())
      .json(&ForkRepositoryRequest {
        organization,
        name: trimmed_name,
        default_branch_only,
      })
      .send()?;
    let status = response.status();
    Self::record_http_status("POST", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    let payload = response.json::<GithubCreatedRepositoryResponse>()?;
    Ok(payload.repository)
  }

  pub fn fetch_github_repository_details(
    &self,
    owner: &str,
    repo: &str,
  ) -> Result<GithubRepositoryDetails> {
    let route = format!("/github/repos/{owner}/{repo}");
    let response = self.authed_request(Method::GET, route.as_str()).send()?;
    let status = response.status();
    Self::record_http_status("GET", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    let payload = response.json::<GithubRepositoryDetails>()?;
    Ok(payload)
  }

  pub fn fetch_github_commit(
    &self,
    owner: &str,
    repo: &str,
    sha: &str,
  ) -> Result<GithubCommitDetails> {
    let response = self
      .authed_request(Method::GET, "/github/commit")
      .query(&[("org", owner), ("repo", repo), ("sha", sha)])
      .send()?;
    let status = response.status();
    Self::record_http_status("GET", "/github/commit", status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    let payload = response.json::<GithubCommitDetailsResponse>()?;
    Ok(payload.commit)
  }

  pub fn star_github_repository(
    &self,
    owner: &str,
    repo: &str,
    node_id: &str,
  ) -> Result<GithubStarResult> {
    let route = format!("/github/repos/{owner}/{repo}/star");
    let response = self
      .authed_request(Method::PUT, route.as_str())
      .query(&[("node_id", node_id)])
      .send()?;
    let status = response.status();
    Self::record_http_status("PUT", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    let payload = response.json::<GithubStarResult>()?;
    Ok(payload)
  }

  pub fn unstar_github_repository(
    &self,
    owner: &str,
    repo: &str,
    node_id: &str,
  ) -> Result<GithubStarResult> {
    let route = format!("/github/repos/{owner}/{repo}/star");
    let response = self
      .authed_request(Method::DELETE, route.as_str())
      .query(&[("node_id", node_id)])
      .send()?;
    let status = response.status();
    Self::record_http_status("DELETE", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    let payload = response.json::<GithubStarResult>()?;
    Ok(payload)
  }

  pub fn set_github_repository_subscription(
    &self,
    owner: &str,
    repo: &str,
    mode: RepoSubscriptionMode,
  ) -> Result<RepoSubscriptionMode> {
    let route = format!("/github/repos/{owner}/{repo}/subscription");
    let response = self
      .authed_request(Method::PUT, route.as_str())
      .json(&serde_json::json!({ "mode": mode.as_str() }))
      .send()?;
    let status = response.status();
    Self::record_http_status("PUT", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    let payload = response.json::<RepoSubscriptionStateResponse>()?;
    Ok(payload.mode)
  }

  pub fn fetch_github_repository_tree(
    &self,
    owner: &str,
    repo: &str,
    tree_sha: &str,
  ) -> Result<GithubRepositoryTree> {
    let route = format!("/github/repos/{owner}/{repo}/trees/{tree_sha}");
    let response = self
      .authed_request(Method::GET, route.as_str())
      .query(&[("recursive", "1")])
      .send()?;
    let status = response.status();
    Self::record_http_status("GET", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    let payload = response.json::<GithubRepositoryTree>()?;
    Ok(payload)
  }

  pub fn fetch_github_repository_branches(
    &self,
    owner: &str,
    repo: &str,
  ) -> Result<Vec<GithubRepositoryBranch>> {
    let route = format!("/github/repos/{owner}/{repo}/branches");
    let response = self.authed_request(Method::GET, route.as_str()).send()?;
    let status = response.status();
    Self::record_http_status("GET", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    let payload = response.json::<Vec<GithubRepositoryBranch>>()?;
    Ok(payload)
  }

  pub fn fetch_github_repository_readme(
    &self,
    owner: &str,
    repo: &str,
    reference: Option<&str>,
  ) -> Result<Option<GithubRepositoryReadme>> {
    let route = format!("/github/repos/{owner}/{repo}/readme");
    let request = self.authed_request(Method::GET, route.as_str());
    let request =
      if let Some(reference) = reference.map(str::trim).filter(|value| !value.is_empty()) {
        request.query(&[("ref", reference)])
      } else {
        request
      };
    let response = request.send()?;
    let status = response.status();
    Self::record_http_status("GET", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if status == StatusCode::NOT_FOUND {
      return Ok(None);
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    let payload = response.json::<GithubRepositoryReadme>()?;
    Ok(Some(payload))
  }

  pub fn fetch_github_repository_pull_requests(
    &self,
    owner: &str,
    repo: &str,
    filters: &GithubPullRequestSearchFilters,
    state: &str,
    page: u64,
  ) -> Result<GithubRepositoryPullRequestsResponse> {
    let route = format!("/github/repos/{owner}/{repo}/pr");
    let mut query = github_repository_pull_request_filter_query(filters);
    query.push(("state".to_string(), state.to_string()));
    query.push(("page".to_string(), page.to_string()));
    let response = self
      .authed_request(Method::GET, route.as_str())
      .query(&query)
      .send()?;
    let status = response.status();
    Self::record_http_status("GET", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    response
      .json::<GithubRepositoryPullRequestsResponse>()
      .map_err(Into::into)
  }

  pub fn fetch_github_repository_issues(
    &self,
    owner: &str,
    repo: &str,
    state: &str,
    filters: &GithubIssueSearchFilters,
    page: u64,
  ) -> Result<GithubRepositoryIssuesResponse> {
    let route = format!("/github/repos/{owner}/{repo}/issues");
    let mut query = github_repository_issue_filter_query(filters);
    query.push(("state".to_string(), state.to_string()));
    query.push(("page".to_string(), page.to_string()));
    let response = self
      .authed_request(Method::GET, route.as_str())
      .query(&query)
      .send()?;
    let status = response.status();
    Self::record_http_status("GET", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    response
      .json::<GithubRepositoryIssuesResponse>()
      .map_err(Into::into)
  }

  pub fn fetch_pull_request_for_branch(
    &self,
    owner: &str,
    repo: &str,
    branch: &str,
  ) -> Result<Option<GithubPullRequest>> {
    let route = format!("/github/repos/{owner}/{repo}/pr/branch");
    let response = self
      .authed_request(Method::GET, route.as_str())
      .query(&[("branch", branch)])
      .send()?;
    let status = response.status();
    Self::record_http_status("GET", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    let payload = response.json::<GithubPullRequestResponse>()?;
    Ok(payload.pull_request)
  }

  pub fn create_pull_request(
    &self,
    owner: &str,
    repo: &str,
    branch: &str,
    title: &str,
    base: &str,
    body: Option<&str>,
    draft: bool,
  ) -> Result<GithubPullRequest> {
    let route = format!("/github/repos/{owner}/{repo}/pr");
    let trimmed_body = body.map(str::trim).filter(|value| !value.is_empty());
    let response = self
      .authed_request(Method::POST, route.as_str())
      .query(&[("branch", branch)])
      .json(&CreateGithubPullRequestRequest {
        title: title.trim(),
        base: base.trim(),
        body: trimmed_body,
        draft: draft.then_some(true),
      })
      .send()?;
    let status = response.status();
    Self::record_http_status("POST", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    let payload = response.json::<GithubPullRequestResponse>()?;
    payload
      .pull_request
      .ok_or_else(|| anyhow::anyhow!("Missing pull request in response"))
  }

  pub fn fetch_github_repository_issue_details(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
  ) -> Result<GithubIssueDetails> {
    let route = format!("/github/repos/{owner}/{repo}/issues/{number}");
    let response = self.authed_request(Method::GET, route.as_str()).send()?;
    let status = response.status();
    Self::record_http_status("GET", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    let payload = response.json::<GithubIssueDetailsResponse>()?;
    Ok(payload.issue)
  }

  pub fn resolve_github_issue_reference_target(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
  ) -> Result<GithubIssueReferenceTarget> {
    let route = format!("/github/repos/{owner}/{repo}/issues/{number}/target");
    let response = self.authed_request(Method::GET, route.as_str()).send()?;
    let status = response.status();
    Self::record_http_status("GET", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    let payload = response.json::<GithubIssueReferenceTargetResponse>()?;
    Ok(payload.target)
  }

  pub fn update_issue_description(
    &self,
    owner: &str,
    repo: &str,
    issue_number: u64,
    body: &str,
  ) -> Result<GithubIssueDescriptionUpdate> {
    let route = format!("/github/repos/{owner}/{repo}/issues/{issue_number}");
    let response = self
      .authed_request(Method::PATCH, route.as_str())
      .json(&UpdateGithubDescriptionRequest { body })
      .send()?;
    let status = response.status();
    Self::record_http_status("PATCH", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    let payload = response.json::<GithubIssueDescriptionUpdateResponse>()?;
    Ok(payload.issue)
  }

  pub fn create_issue_comment(
    &self,
    owner: &str,
    repo: &str,
    issue_number: u64,
    body: &str,
  ) -> Result<GithubIssueDetailsComment> {
    let route = format!("/github/repos/{owner}/{repo}/issues/{issue_number}/comments");
    let response = self
      .authed_request(Method::POST, route.as_str())
      .json(&UpdateGithubIssueCommentRequest { body })
      .send()?;
    let status = response.status();
    Self::record_http_status("POST", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    let payload = response.json::<GithubIssueCommentResponse>()?;
    Ok(payload.comment)
  }

  pub fn update_issue_comment(
    &self,
    owner: &str,
    repo: &str,
    issue_number: u64,
    comment_id: u64,
    body: &str,
  ) -> Result<GithubIssueDetailsComment> {
    let route = format!("/github/repos/{owner}/{repo}/issues/{issue_number}/comments/{comment_id}");
    let response = self
      .authed_request(Method::PATCH, route.as_str())
      .json(&UpdateGithubIssueCommentRequest { body })
      .send()?;
    let status = response.status();
    Self::record_http_status("PATCH", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    let payload = response.json::<GithubIssueCommentResponse>()?;
    Ok(payload.comment)
  }

  pub fn delete_issue_comment(
    &self,
    owner: &str,
    repo: &str,
    issue_number: u64,
    comment_id: u64,
  ) -> Result<()> {
    let route = format!("/github/repos/{owner}/{repo}/issues/{issue_number}/comments/{comment_id}");
    let response = self.authed_request(Method::DELETE, route.as_str()).send()?;
    let status = response.status();
    Self::record_http_status("DELETE", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    Ok(())
  }

  pub fn fetch_pull_request_details(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
  ) -> Result<GithubPullRequestDetails> {
    let route = format!("/github/pr/{number}");
    let response = self
      .authed_request(Method::GET, route.as_str())
      .query(&[("org", owner), ("repo", repo)])
      .send()?;
    let status = response.status();
    Self::record_http_status("GET", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    let payload = response.json::<GithubPullRequestDetailsResponse>()?;
    Ok(payload.pull_request)
  }

  pub fn fetch_pull_request_merge_readiness(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
  ) -> Result<GithubPullRequestMergeReadiness> {
    let route = format!("/github/pr/{number}/merge-readiness");
    let response = self
      .authed_request(Method::GET, route.as_str())
      .query(&[("org", owner), ("repo", repo)])
      .send()?;
    let status = response.status();
    Self::record_http_status("GET", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    let payload = response.json::<GithubPullRequestMergeReadinessResponse>()?;
    Ok(payload.merge_readiness)
  }

  pub fn fetch_pull_request_checks(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
  ) -> Result<GithubPullRequestChecksSummary> {
    let route = format!("/github/pr/{number}/checks");
    let response = self
      .authed_request(Method::GET, route.as_str())
      .query(&[("org", owner), ("repo", repo)])
      .send()?;
    let status = response.status();
    Self::record_http_status("GET", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    let payload = response.json::<GithubPullRequestChecksSummaryResponse>()?;
    Ok(payload.checks)
  }

  pub fn update_pull_request_description(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
    body: &str,
  ) -> Result<GithubPullRequestDescriptionUpdate> {
    let route = format!("/github/pr/{number}");
    let response = self
      .authed_request(Method::PATCH, route.as_str())
      .query(&[("org", owner), ("repo", repo)])
      .json(&UpdateGithubDescriptionRequest { body })
      .send()?;
    let status = response.status();
    Self::record_http_status("PATCH", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    let payload = response.json::<GithubPullRequestDescriptionUpdateResponse>()?;
    Ok(payload.pull_request)
  }

  pub fn update_pull_request_base(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
    base: &str,
  ) -> Result<()> {
    let route = format!("/github/pr/{number}/base");
    let response = self
      .authed_request(Method::PATCH, route.as_str())
      .query(&[("org", owner), ("repo", repo)])
      .json(&UpdateGithubPullRequestBaseRequest { base })
      .send()?;
    let status = response.status();
    Self::record_http_status("PATCH", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    Ok(())
  }

  pub fn add_pull_request_assignee(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
    login: &str,
  ) -> Result<()> {
    let route = format!("/github/pr/{number}/assignees");
    let users = [login];
    let response = self
      .authed_request(Method::POST, route.as_str())
      .query(&[("org", owner), ("repo", repo)])
      .json(&GithubPullRequestUsersMutationRequest { users: &users })
      .send()?;
    let status = response.status();
    Self::record_http_status("POST", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    Ok(())
  }

  pub fn remove_pull_request_assignee(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
    login: &str,
  ) -> Result<()> {
    let route = format!("/github/pr/{number}/assignees");
    let users = [login];
    let response = self
      .authed_request(Method::DELETE, route.as_str())
      .query(&[("org", owner), ("repo", repo)])
      .json(&GithubPullRequestUsersMutationRequest { users: &users })
      .send()?;
    let status = response.status();
    Self::record_http_status("DELETE", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    Ok(())
  }

  pub fn add_pull_request_label(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
    label: &str,
  ) -> Result<()> {
    let route = format!("/github/pr/{number}/labels");
    let labels = [label];
    let response = self
      .authed_request(Method::POST, route.as_str())
      .query(&[("org", owner), ("repo", repo)])
      .json(&GithubPullRequestLabelsMutationRequest { labels: &labels })
      .send()?;
    let status = response.status();
    Self::record_http_status("POST", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    Ok(())
  }

  pub fn remove_pull_request_label(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
    label: &str,
  ) -> Result<()> {
    let route = format!("/github/pr/{number}/labels");
    let labels = [label];
    let response = self
      .authed_request(Method::DELETE, route.as_str())
      .query(&[("org", owner), ("repo", repo)])
      .json(&GithubPullRequestLabelsMutationRequest { labels: &labels })
      .send()?;
    let status = response.status();
    Self::record_http_status("DELETE", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    Ok(())
  }

  pub fn request_pull_request_reviewer(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
    login: &str,
  ) -> Result<()> {
    let route = format!("/github/pr/{number}/requested-reviewers");
    let users = [login];
    let response = self
      .authed_request(Method::POST, route.as_str())
      .query(&[("org", owner), ("repo", repo)])
      .json(&GithubPullRequestUsersMutationRequest { users: &users })
      .send()?;
    let status = response.status();
    Self::record_http_status("POST", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    Ok(())
  }

  pub fn remove_pull_request_reviewer(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
    login: &str,
  ) -> Result<()> {
    let route = format!("/github/pr/{number}/requested-reviewers");
    let users = [login];
    let response = self
      .authed_request(Method::DELETE, route.as_str())
      .query(&[("org", owner), ("repo", repo)])
      .json(&GithubPullRequestUsersMutationRequest { users: &users })
      .send()?;
    let status = response.status();
    Self::record_http_status("DELETE", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    Ok(())
  }

  pub fn mark_pull_request_ready_for_review(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
    pull_request_id: &str,
  ) -> Result<()> {
    let route = format!("/github/pr/{number}/ready-for-review");
    let response = self
      .authed_request(Method::POST, route.as_str())
      .query(&[("org", owner), ("repo", repo)])
      .json(&GithubPullRequestStatusMutationRequest { pull_request_id })
      .send()?;
    let status = response.status();
    Self::record_http_status("POST", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    Ok(())
  }

  pub fn convert_pull_request_to_draft(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
    pull_request_id: &str,
  ) -> Result<()> {
    let route = format!("/github/pr/{number}/convert-to-draft");
    let response = self
      .authed_request(Method::POST, route.as_str())
      .query(&[("org", owner), ("repo", repo)])
      .json(&GithubPullRequestStatusMutationRequest { pull_request_id })
      .send()?;
    let status = response.status();
    Self::record_http_status("POST", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    Ok(())
  }

  pub fn enable_pull_request_auto_merge(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
    pull_request_id: &str,
    method: GithubPullRequestMergeMethod,
    commit_title: Option<&str>,
    commit_message: Option<&str>,
  ) -> Result<()> {
    let route = format!("/github/pr/{number}/auto-merge");
    let trimmed_title = commit_title
      .map(str::trim)
      .filter(|value| !value.is_empty());
    let trimmed_message = commit_message
      .map(str::trim)
      .filter(|value| !value.is_empty());
    let response = self
      .authed_request(Method::POST, route.as_str())
      .query(&[("org", owner), ("repo", repo)])
      .json(&EnableGithubPullRequestAutoMergeRequest {
        pull_request_id,
        method,
        commit_title: trimmed_title,
        commit_message: trimmed_message,
      })
      .send()?;
    let status = response.status();
    Self::record_http_status("POST", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    Ok(())
  }

  pub fn disable_pull_request_auto_merge(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
    pull_request_id: &str,
  ) -> Result<()> {
    let route = format!("/github/pr/{number}/auto-merge");
    let response = self
      .authed_request(Method::DELETE, route.as_str())
      .query(&[("org", owner), ("repo", repo)])
      .json(&GithubPullRequestStatusMutationRequest { pull_request_id })
      .send()?;
    let status = response.status();
    Self::record_http_status("DELETE", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    Ok(())
  }

  pub fn merge_pull_request(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
    method: GithubPullRequestMergeMethod,
    expected_head_sha: &str,
    commit_title: Option<&str>,
    commit_message: Option<&str>,
  ) -> Result<GithubPullRequestMergeResult> {
    let route = format!("/github/pr/{number}/merge");
    let trimmed_title = commit_title
      .map(str::trim)
      .filter(|value| !value.is_empty());
    let trimmed_message = commit_message
      .map(str::trim)
      .filter(|value| !value.is_empty());
    let response = self
      .authed_request(Method::PUT, route.as_str())
      .query(&[("org", owner), ("repo", repo)])
      .json(&CreateGithubPullRequestMergeRequest {
        method,
        expected_head_sha,
        commit_title: trimmed_title,
        commit_message: trimmed_message,
      })
      .send()?;
    let status = response.status();
    Self::record_http_status("PUT", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    let payload = response.json::<GithubPullRequestMergeResultResponse>()?;
    Ok(payload.merge_result)
  }

  pub fn update_pull_request_branch(&self, owner: &str, repo: &str, number: u64) -> Result<()> {
    let route = format!("/github/pr/{number}/update-branch");
    let response = self
      .authed_request(Method::PUT, route.as_str())
      .query(&[("org", owner), ("repo", repo)])
      .send()?;
    let status = response.status();
    Self::record_http_status("PUT", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    Ok(())
  }

  pub fn fetch_pull_request_files(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
    commit_sha: Option<&str>,
  ) -> Result<Vec<GithubPullRequestFile>> {
    let route = format!("/github/pr/{number}/files");
    let mut query = vec![("org", owner), ("repo", repo)];
    if let Some(commit_sha) = commit_sha {
      query.push(("commitSha", commit_sha));
    }
    let response = self
      .authed_request(Method::GET, route.as_str())
      .query(&query)
      .send()?;
    let status = response.status();
    Self::record_http_status("GET", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    let payload = response.json::<GithubPullRequestFilesResponse>()?;
    Ok(payload.files)
  }

  pub fn fetch_pull_request_commits(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
  ) -> Result<Vec<GithubPullRequestCommit>> {
    let route = format!("/github/pr/{number}/commits");
    let response = self
      .authed_request(Method::GET, route.as_str())
      .query(&[("org", owner), ("repo", repo)])
      .send()?;
    let status = response.status();
    Self::record_http_status("GET", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    let payload = response.json::<GithubPullRequestCommitsResponse>()?;
    Ok(payload.commits)
  }

  pub fn fetch_github_notifications(&self) -> Result<Vec<GithubNotification>> {
    let response = self
      .authed_request(Method::GET, "/github/notifications")
      .send()?;
    let status = response.status();
    Self::record_http_status("GET", "/github/notifications", status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    let payload = response.json::<GithubNotificationsResponse>()?;
    Ok(payload.notifications)
  }

  pub fn mark_notification_done(&self, thread_id: &str) -> Result<()> {
    let route = format!("/github/notifications/{thread_id}/done");
    let response = self.authed_request(Method::DELETE, route.as_str()).send()?;
    let status = response.status();
    Self::record_http_status("DELETE", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    Ok(())
  }

  pub fn mark_notification_read(&self, thread_id: &str) -> Result<()> {
    let route = format!("/github/notifications/{thread_id}/read");
    let response = self.authed_request(Method::PATCH, route.as_str()).send()?;
    let status = response.status();
    Self::record_http_status("PATCH", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    Ok(())
  }

  pub fn fetch_pull_request_conversation(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
  ) -> Result<GithubPullRequestConversation> {
    let route = format!("/github/pr/{number}/conversation");
    let response = self
      .authed_request(Method::GET, route.as_str())
      .query(&[("org", owner), ("repo", repo)])
      .send()?;
    let status = response.status();
    Self::record_http_status("GET", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    let payload = response.json::<GithubPullRequestConversationResponse>()?;
    Ok(payload.conversation)
  }

  pub fn add_pull_request_reaction(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
    subject_id: &str,
    content: GithubReactionContent,
  ) -> Result<Vec<GithubReactionGroup>> {
    let route = format!("/github/pr/{number}/reactions");
    let response = self
      .authed_request(Method::POST, route.as_str())
      .query(&[("org", owner), ("repo", repo)])
      .json(&GithubReactionMutationRequest {
        subject_id,
        content,
      })
      .send()?;
    let status = response.status();
    Self::record_http_status("POST", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    let payload = response.json::<GithubReactionGroupsResponse>()?;
    Ok(payload.reactions)
  }

  pub fn remove_pull_request_reaction(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
    subject_id: &str,
    content: GithubReactionContent,
  ) -> Result<Vec<GithubReactionGroup>> {
    let route = format!("/github/pr/{number}/reactions");
    let response = self
      .authed_request(Method::DELETE, route.as_str())
      .query(&[("org", owner), ("repo", repo)])
      .json(&GithubReactionMutationRequest {
        subject_id,
        content,
      })
      .send()?;
    let status = response.status();
    Self::record_http_status("DELETE", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    let payload = response.json::<GithubReactionGroupsResponse>()?;
    Ok(payload.reactions)
  }

  pub fn add_issue_reaction(
    &self,
    owner: &str,
    repo: &str,
    issue_number: u64,
    subject_id: &str,
    content: GithubReactionContent,
  ) -> Result<Vec<GithubReactionGroup>> {
    let route = format!("/github/repos/{owner}/{repo}/issues/{issue_number}/reactions");
    let response = self
      .authed_request(Method::POST, route.as_str())
      .json(&GithubReactionMutationRequest {
        subject_id,
        content,
      })
      .send()?;
    let status = response.status();
    Self::record_http_status("POST", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    let payload = response.json::<GithubReactionGroupsResponse>()?;
    Ok(payload.reactions)
  }

  pub fn remove_issue_reaction(
    &self,
    owner: &str,
    repo: &str,
    issue_number: u64,
    subject_id: &str,
    content: GithubReactionContent,
  ) -> Result<Vec<GithubReactionGroup>> {
    let route = format!("/github/repos/{owner}/{repo}/issues/{issue_number}/reactions");
    let response = self
      .authed_request(Method::DELETE, route.as_str())
      .json(&GithubReactionMutationRequest {
        subject_id,
        content,
      })
      .send()?;
    let status = response.status();
    Self::record_http_status("DELETE", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    let payload = response.json::<GithubReactionGroupsResponse>()?;
    Ok(payload.reactions)
  }

  pub fn update_pull_request_review_comment(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
    comment_id: u64,
    body: &str,
  ) -> Result<GithubPullRequestReviewComment> {
    let route = format!("/github/pr/{number}/comments/{comment_id}");
    let response = self
      .authed_request(Method::PATCH, route.as_str())
      .query(&[("org", owner), ("repo", repo)])
      .json(&UpdateGithubPullRequestCommentRequest { body })
      .send()?;
    let status = response.status();
    Self::record_http_status("PATCH", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    let payload = response.json::<GithubPullRequestCommentResponse>()?;
    Ok(payload.comment)
  }

  #[allow(clippy::too_many_arguments)]
  pub fn create_pull_request_review_comment(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
    path: &str,
    commit_id: &str,
    line: u64,
    side: &str,
    start_line: Option<u64>,
    start_side: Option<&str>,
    body: &str,
  ) -> Result<GithubPullRequestReviewComment> {
    let route = format!("/github/pr/{number}/comments");
    let response = self
      .authed_request(Method::POST, route.as_str())
      .query(&[("org", owner), ("repo", repo)])
      .json(&CreateGithubPullRequestCommentRequest {
        body,
        path,
        commit_id,
        line,
        side,
        start_line,
        start_side,
      })
      .send()?;
    let status = response.status();
    Self::record_http_status("POST", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    let payload = response.json::<GithubPullRequestCommentResponse>()?;
    Ok(payload.comment)
  }

  pub fn reply_pull_request_review_comment(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
    comment_id: u64,
    body: &str,
  ) -> Result<GithubPullRequestReviewComment> {
    let route = format!("/github/pr/{number}/comments/{comment_id}/replies");
    let response = self
      .authed_request(Method::POST, route.as_str())
      .query(&[("org", owner), ("repo", repo)])
      .json(&ReplyGithubPullRequestCommentRequest { body })
      .send()?;
    let status = response.status();
    Self::record_http_status("POST", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    let payload = response.json::<GithubPullRequestCommentResponse>()?;
    Ok(payload.comment)
  }

  pub fn delete_pull_request_review_comment(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
    comment_id: u64,
  ) -> Result<()> {
    let route = format!("/github/pr/{number}/comments/{comment_id}");
    let response = self
      .authed_request(Method::DELETE, route.as_str())
      .query(&[("org", owner), ("repo", repo)])
      .send()?;
    let status = response.status();
    Self::record_http_status("DELETE", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    Ok(())
  }

  #[allow(clippy::too_many_arguments)]
  pub fn apply_pull_request_suggested_change(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
    comment_id: u64,
    commit_title: &str,
    commit_message: Option<&str>,
    expected_head_sha: &str,
    path: &str,
    original_start_line: usize,
    original_lines: &[String],
    suggested_lines: &[String],
    include_co_author: bool,
    suggestion_author_login: Option<&str>,
  ) -> Result<GithubSuggestedChangeCommitResult> {
    let route = format!("/github/pr/{number}/comments/{comment_id}/suggested-change");
    let trimmed_message = commit_message
      .map(str::trim)
      .filter(|value| !value.is_empty());
    let response = self
      .authed_request(Method::POST, route.as_str())
      .query(&[("org", owner), ("repo", repo)])
      .json(&ApplyGithubSuggestedChangeRequest {
        commit_title: commit_title.trim(),
        commit_message: trimmed_message,
        expected_head_sha,
        path,
        original_start_line,
        original_lines,
        suggested_lines,
        include_co_author,
        suggestion_author_login,
      })
      .send()?;
    let status = response.status();
    Self::record_http_status("POST", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    let payload = response.json::<GithubSuggestedChangeCommitResponse>()?;
    Ok(payload.commit)
  }

  pub fn resolve_pull_request_review_thread(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
    thread_id: &str,
  ) -> Result<()> {
    self.toggle_pull_request_review_thread_resolution(owner, repo, number, thread_id, true)
  }

  pub fn unresolve_pull_request_review_thread(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
    thread_id: &str,
  ) -> Result<()> {
    self.toggle_pull_request_review_thread_resolution(owner, repo, number, thread_id, false)
  }

  fn toggle_pull_request_review_thread_resolution(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
    thread_id: &str,
    resolve: bool,
  ) -> Result<()> {
    let action = if resolve { "resolve" } else { "unresolve" };
    let route = format!("/github/pr/{number}/review-threads/{thread_id}/{action}");
    let response = self
      .authed_request(Method::POST, route.as_str())
      .query(&[("org", owner), ("repo", repo)])
      .send()?;
    let status = response.status();
    Self::record_http_status("POST", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    let _ = response.json::<GithubReviewThreadResolutionResponse>()?;
    Ok(())
  }

  pub fn upload_asset(
    &self,
    bytes: Vec<u8>,
    content_type: &str,
    file_name: &str,
  ) -> Result<String> {
    let part = reqwest::blocking::multipart::Part::bytes(bytes)
      .file_name(file_name.to_string())
      .mime_str(content_type)?;
    let form = reqwest::blocking::multipart::Form::new().part("file", part);
    let response = self
      .authed_request(Method::POST, "/assets/upload")
      .multipart(form)
      .send()?;
    let status = response.status();
    Self::record_http_status("POST", "/assets/upload", status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    let payload = response.json::<UploadAssetResponse>()?;
    Ok(payload.url)
  }

  pub fn submit_pull_request_review(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
    event: GithubPullRequestReviewEvent,
    body: &str,
  ) -> Result<GithubPullRequestReview> {
    let route = format!("/github/pr/{number}/reviews");
    let trimmed_body = body.trim();
    let payload = CreateGithubPullRequestReviewRequest {
      event,
      body: if matches!(event, GithubPullRequestReviewEvent::Approve) && trimmed_body.is_empty() {
        None
      } else {
        Some(trimmed_body)
      },
    };
    let response = self
      .authed_request(Method::POST, route.as_str())
      .query(&[("org", owner), ("repo", repo)])
      .json(&payload)
      .send()?;
    let status = response.status();
    Self::record_http_status("POST", route.as_str(), status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    let payload = response.json::<GithubPullRequestReviewResponse>()?;
    Ok(payload.review)
  }

  pub fn fetch_github_file_content(
    &self,
    owner: &str,
    repo: &str,
    path: &str,
    reference: &str,
  ) -> Result<Option<String>> {
    let response = self
      .authed_request(Method::GET, "/github/file")
      .query(&[
        ("org", owner),
        ("repo", repo),
        ("path", path),
        ("ref", reference),
      ])
      .send()?;
    let status = response.status();
    Self::record_http_status("GET", "/github/file", status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    let payload = response.json::<GithubFileContentResponse>()?;
    Ok(payload.content)
  }

  pub fn fetch_github_file_commit(
    &self,
    owner: &str,
    repo: &str,
    path: &str,
    reference: &str,
  ) -> Result<GithubFileCommit> {
    let response = self
      .authed_request(Method::GET, "/github/file/commit")
      .query(&[
        ("org", owner),
        ("repo", repo),
        ("path", path),
        ("ref", reference),
      ])
      .send()?;
    let status = response.status();
    Self::record_http_status("GET", "/github/file/commit", status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    Ok(response.json::<GithubFileCommit>()?)
  }

  pub fn fetch_github_file_asset(
    &self,
    owner: &str,
    repo: &str,
    path: &str,
    reference: &str,
  ) -> Result<Option<Vec<u8>>> {
    let response = self
      .authed_request(Method::GET, "/github/file/asset")
      .query(&[
        ("org", owner),
        ("repo", repo),
        ("path", path),
        ("ref", reference),
      ])
      .send()?;
    let status = response.status();
    Self::record_http_status("GET", "/github/file/asset", status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }
    let payload = response.json::<GithubFileAssetResponse>()?;
    payload
      .content_base64
      .map(|content| general_purpose::STANDARD.decode(content))
      .transpose()
      .map_err(Into::into)
  }

  pub fn check_desktop_update(
    &self,
    current_version: &str,
    platform: &str,
    arch: &str,
  ) -> Result<DesktopUpdateCheckResponse> {
    let response = self
      .client
      .post(self.get_api_url("/desktop/update/check"))
      .json(&DesktopUpdateCheckRequest {
        current_version,
        platform,
        arch,
      })
      .send()?;
    let status = response.status();
    Self::record_http_status("POST", "/desktop/update/check", status);
    if !status.is_success() {
      anyhow::bail!("unexpected status: {}", status);
    }

    let payload = response.json::<DesktopUpdateCheckResponse>()?;
    Ok(payload)
  }

  pub fn resolve_github_asset_url(&self, url: &str) -> Result<String> {
    let response = self
      .authed_request(Method::GET, "/github/asset/resolve")
      .query(&[("url", url)])
      .send()?;
    let status = response.status();
    Self::record_http_status("GET", "/github/asset/resolve", status);
    if status == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !status.is_success() {
      return Err(Self::api_error_from_response(response));
    }
    let payload = response.json::<GithubAssetResolveResponse>()?;
    Ok(payload.url)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use reqwest::blocking::Client;
  use reqwest::header::AUTHORIZATION;
  use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    thread,
  };

  fn make_test_api_client(base_url: String) -> ApiClient {
    ApiClient {
      base_url,
      client: Client::builder()
        .cookie_store(true)
        .build()
        .expect("build client"),
      bearer_token: Arc::new(Mutex::new(None)),
    }
  }

  #[test]
  fn resolve_api_base_url_prefers_runtime_override() {
    assert_eq!(
      resolve_api_base_url(
        Some("https://runtime.reviu.dev".to_string()),
        Some("https://build.reviu.dev")
      ),
      "https://runtime.reviu.dev"
    );
  }

  #[test]
  fn resolve_api_base_url_uses_build_time_value_when_runtime_is_missing() {
    assert_eq!(
      resolve_api_base_url(None, Some("https://build.reviu.dev")),
      "https://build.reviu.dev"
    );
  }

  #[test]
  fn resolve_api_base_url_falls_back_to_localhost() {
    assert_eq!(resolve_api_base_url(None, None), DEFAULT_API_BASE_URL);
  }

  #[test]
  fn sign_in_with_github_uses_browser_started_desktop_route() {
    let api = make_test_api_client("https://api.reviu.dev".to_string());

    assert_eq!(
      api.sign_in_with_github().expect("sign in url"),
      Some("https://api.reviu.dev/auth/desktop/start".to_string())
    );
  }

  #[test]
  fn keychain_service_uses_profile_namespace() {
    assert_eq!(AppProfile::Prod.keychain_service(), "reviu_auth");
    assert_eq!(AppProfile::Dev.keychain_service(), "reviu_auth.dev");
  }

  fn start_single_response_server(status: &str, body: &str) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let address = format!("http://{}", listener.local_addr().expect("local addr"));
    let status = status.to_string();
    let body = body.to_string();

    let handle = thread::spawn(move || {
      let (mut stream, _) = listener.accept().expect("accept connection");
      let mut request_buffer = [0u8; 2048];
      let _ = stream.read(&mut request_buffer);

      let response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}",
        body.as_bytes().len(),
        body
      );
      stream
        .write_all(response.as_bytes())
        .expect("write response");
      stream.flush().expect("flush response");
    });

    (address, handle)
  }

  fn start_single_response_server_with_request_line(
    status: &str,
    body: &str,
  ) -> (String, Arc<Mutex<Option<String>>>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let address = format!("http://{}", listener.local_addr().expect("local addr"));
    let status = status.to_string();
    let body = body.to_string();
    let request_line = Arc::new(Mutex::new(None));
    let request_line_for_thread = request_line.clone();

    let handle = thread::spawn(move || {
      let (mut stream, _) = listener.accept().expect("accept connection");
      let mut request_buffer = [0u8; 4096];
      let bytes_read = stream.read(&mut request_buffer).expect("read request");
      let request = String::from_utf8_lossy(&request_buffer[..bytes_read]);
      let first_line = request
        .lines()
        .next()
        .map(str::to_string)
        .unwrap_or_default();
      *request_line_for_thread.lock().expect("lock request line") = Some(first_line);

      let response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}",
        body.as_bytes().len(),
        body
      );
      stream
        .write_all(response.as_bytes())
        .expect("write response");
      stream.flush().expect("flush response");
    });

    (address, request_line, handle)
  }

  fn start_single_response_server_with_request(
    status: &str,
    body: &str,
  ) -> (String, Arc<Mutex<Option<String>>>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let address = format!("http://{}", listener.local_addr().expect("local addr"));
    let status = status.to_string();
    let body = body.to_string();
    let request = Arc::new(Mutex::new(None));
    let request_for_thread = request.clone();

    let handle = thread::spawn(move || {
      let (mut stream, _) = listener.accept().expect("accept connection");
      let mut request_buffer = [0u8; 4096];
      let bytes_read = stream.read(&mut request_buffer).expect("read request");
      let request_body = String::from_utf8_lossy(&request_buffer[..bytes_read]).to_string();
      *request_for_thread.lock().expect("lock request") = Some(request_body);

      let response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}",
        body.as_bytes().len(),
        body
      );
      stream
        .write_all(response.as_bytes())
        .expect("write response");
      stream.flush().expect("flush response");
    });

    (address, request, handle)
  }

  fn make_pull_request(
    state: GithubPullRequestState,
    draft: bool,
    merged_at: Option<&str>,
  ) -> GithubPullRequest {
    GithubPullRequest {
      number: 42,
      title: "Test PR".to_string(),
      state,
      created_at: "2026-02-10T09:00:00Z".to_string(),
      closed_at: None,
      merged_at: merged_at.map(str::to_string),
      draft,
      updated_at: "2026-02-15T12:00:00Z".to_string(),
      comments_count: 0,
      author: GithubPullRequestAuthor {
        login: "octocat".to_string(),
        avatar_url: None,
        is_bot: false,
      },
      labels: vec![GithubPullRequestLabel {
        name: "bug".to_string(),
        color: Some("f29513".to_string()),
      }],
      repository: GithubRepository {
        owner: "acme".to_string(),
        repo: "widget".to_string(),
      },
    }
  }

  fn make_pull_request_details(
    state: GithubPullRequestState,
    draft: bool,
    merged_at: Option<&str>,
  ) -> GithubPullRequestDetails {
    GithubPullRequestDetails {
      node_id: "PR_kwDOExample".to_string(),
      reactions: Vec::new(),
      number: 42,
      title: "Test PR".to_string(),
      state,
      draft,
      created_at: "2026-02-10T09:00:00Z".to_string(),
      updated_at: "2026-02-15T12:00:00Z".to_string(),
      merged_at: merged_at.map(str::to_string),
      merge_base_sha: "abc123".to_string(),
      base_sha: "base123".to_string(),
      head_sha: "head123".to_string(),
      base_ref_name: "main".to_string(),
      head_ref_name: "feature".to_string(),
      body: Some("body".to_string()),
      author: GithubPullRequestAuthor {
        login: "octocat".to_string(),
        avatar_url: None,
        is_bot: false,
      },
      assignees: Vec::new(),
      requested_reviewers: Vec::new(),
      comments: 0,
      review_comments: 0,
      commits: 1,
      additions: 1,
      deletions: 0,
      changed_files: 1,
      labels: vec![GithubPullRequestLabel {
        name: "bug".to_string(),
        color: Some("f29513".to_string()),
      }],
      repository: GithubRepository {
        owner: "acme".to_string(),
        repo: "widget".to_string(),
      },
      head_repository: None,
    }
  }

  #[test]
  fn pull_request_status_prioritizes_merged_then_closed_then_draft() {
    let merged = make_pull_request(
      GithubPullRequestState::Open,
      true,
      Some("2026-02-15T12:00:00Z"),
    );
    assert!(matches!(merged.status(), GithubPullRequestStatus::Merged));

    let closed_draft = make_pull_request(GithubPullRequestState::Closed, true, None);
    assert!(matches!(
      closed_draft.status(),
      GithubPullRequestStatus::Closed
    ));

    let draft = make_pull_request(GithubPullRequestState::Open, true, None);
    assert!(matches!(draft.status(), GithubPullRequestStatus::Draft));

    let closed = make_pull_request(GithubPullRequestState::Closed, false, None);
    assert!(matches!(closed.status(), GithubPullRequestStatus::Closed));
  }

  #[test]
  fn pull_request_details_status_prioritizes_merged_then_closed_then_draft() {
    let merged = make_pull_request_details(
      GithubPullRequestState::Open,
      true,
      Some("2026-02-15T12:00:00Z"),
    );
    assert!(matches!(merged.status(), GithubPullRequestStatus::Merged));

    let closed_draft = make_pull_request_details(GithubPullRequestState::Closed, true, None);
    assert!(matches!(
      closed_draft.status(),
      GithubPullRequestStatus::Closed
    ));

    let draft = make_pull_request_details(GithubPullRequestState::Open, true, None);
    assert!(matches!(draft.status(), GithubPullRequestStatus::Draft));

    let open = make_pull_request_details(GithubPullRequestState::Open, false, None);
    assert!(matches!(open.status(), GithubPullRequestStatus::Open));
  }

  #[test]
  fn authed_request_sets_authorization_header_when_token_present() {
    let api = make_test_api_client("http://localhost:3999".to_string());

    let request_without_token = api
      .authed_request(Method::GET, "/users/me")
      .build()
      .expect("build request without token");
    assert!(request_without_token.headers().get(AUTHORIZATION).is_none());

    api.set_bearer_token("secret-token".to_string());
    let request_with_token = api
      .authed_request(Method::GET, "/users/me")
      .build()
      .expect("build request with token");

    let authorization = request_with_token
      .headers()
      .get(AUTHORIZATION)
      .and_then(|value| value.to_str().ok());
    assert_eq!(authorization, Some("Bearer secret-token"));
  }

  #[test]
  fn submit_crash_report_posts_expected_route_and_payload() {
    let body = r#"{"issueId":"issue-123","url":"https://stowline.example.com/issues/issue-123"}"#;
    let (base_url, request, handle) =
      start_single_response_server_with_request("201 Created", body);
    let api = make_test_api_client(base_url);

    api
      .submit_crash_report(&StartupCrashReport {
        crash_id: "crash-123".to_string(),
        message: "editor panic".to_string(),
        panic_location: Some("desktop/crates/editor/src/editor.rs:42:7".to_string()),
        backtrace: Some("frame 1\nframe 2".to_string()),
        thread_name: Some("main".to_string()),
        app_version: "0.0.11".to_string(),
        release: Some("reviu@0.0.11".to_string()),
        os: "macos".to_string(),
        arch: "aarch64".to_string(),
        app_profile: "prod".to_string(),
        happened_at: "2026-04-03T10:00:00Z".to_string(),
        pathname: Some("/git".to_string()),
        workspace_page: Some("git".to_string()),
        git_context: Some(crate::sentry_context::CrashGitContext {
          repo_name: Some("reviu".to_string()),
          repo_hash: Some("abc123def456".to_string()),
          selected_file: Some("desktop/crates/editor/src/editor.rs".to_string()),
          branch: Some("main".to_string()),
          sidebar_mode: "changes".to_string(),
          diff_view: "unified".to_string(),
        }),
        github_pr_context: None,
      })
      .expect("submit crash report");

    handle.join().expect("join server thread");
    let request = request
      .lock()
      .expect("lock request")
      .clone()
      .unwrap_or_default();
    assert!(
      request.contains("POST /crash-reports "),
      "request: {request}"
    );
    assert!(
      request.contains("\"crashId\":\"crash-123\""),
      "request: {request}"
    );
    assert!(
      request.contains("\"panicLocation\":\"desktop/crates/editor/src/editor.rs:42:7\""),
      "request: {request}"
    );
    assert!(
      request.contains("\"appProfile\":\"prod\""),
      "request: {request}"
    );
    assert!(
      request.contains("\"workspacePage\":\"git\""),
      "request: {request}"
    );
    assert!(
      request.contains("\"gitContext\":{\"repoName\":\"reviu\""),
      "request: {request}"
    );
  }

  #[test]
  fn fetch_github_pull_requests_parses_success_payload() {
    let body = r#"{
      "pullRequests": [
        {
          "number": 7,
          "title": "Fix login issue",
          "state": "open",
          "created_at": "2026-02-12T09:30:00Z",
          "closed_at": null,
          "merged_at": null,
          "draft": false,
          "updated_at": "2026-02-15T12:00:00Z",
          "comments_count": 5,
          "author": {
            "login": "octocat",
            "avatar_url": "https://example.com/octocat.png",
            "is_bot": false
          },
          "labels": [{ "name": "bug", "color": "f29513" }],
          "repository": { "owner": "acme", "repo": "widget" }
        }
      ]
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);
    let filters = crate::github_home_tabs::GithubPullRequestSearchFilters {
      authors: vec!["@me".to_string()],
      ..crate::github_home_tabs::GithubPullRequestSearchFilters::default()
    };

    let prs = api
      .fetch_github_pull_requests(&filters)
      .expect("fetch pull requests");
    assert_eq!(prs.len(), 1);
    assert_eq!(prs[0].number, 7);
    assert_eq!(prs[0].title, "Fix login issue");
    assert!(matches!(prs[0].state, GithubPullRequestState::Open));
    assert_eq!(prs[0].created_at, "2026-02-12T09:30:00Z");
    assert_eq!(prs[0].comments_count, 5);
    assert_eq!(prs[0].author.login, "octocat");
    assert_eq!(
      prs[0].author.avatar_url.as_deref(),
      Some("https://example.com/octocat.png")
    );
    assert!(!prs[0].author.is_bot);
    assert_eq!(prs[0].labels[0].color.as_deref(), Some("f29513"));
    assert_eq!(prs[0].repository.owner, "acme");
    assert_eq!(prs[0].repository.repo, "widget");
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_github_pull_requests_posts_expected_route_and_payload() {
    let body = r#"{"pullRequests":[]}"#;
    let (base_url, request, handle) = start_single_response_server_with_request("200 OK", body);
    let api = make_test_api_client(base_url);
    let filters = crate::github_home_tabs::GithubPullRequestSearchFilters {
      repos: vec!["acme/reviu".to_string()],
      labels: vec!["bug".to_string()],
      excluded_labels: vec!["team/reveal".to_string()],
      authors: vec!["@me".to_string()],
      assignees: vec!["alice".to_string()],
      requested_reviewers: vec!["@me".to_string()],
      review_status: crate::github_home_tabs::GithubPullRequestReviewStatus::Required,
      include_drafts: false,
      base: Some("main".to_string()),
      sort: crate::github_home_tabs::GithubPullRequestSearchSort::CommentsDesc,
    };

    let _ = api
      .fetch_github_pull_requests(&filters)
      .expect("fetch pull requests");

    handle.join().expect("join server thread");
    let request = request
      .lock()
      .expect("lock request")
      .clone()
      .unwrap_or_default();
    assert!(
      request.contains("POST /github/pr/search "),
      "request: {request}"
    );
    assert!(
      request.contains("\"repos\":[\"acme/reviu\"]"),
      "request: {request}"
    );
    assert!(
      request.contains("\"labels\":[\"bug\"]"),
      "request: {request}"
    );
    assert!(
      request.contains("\"excluded_labels\":[\"team/reveal\"]"),
      "request: {request}"
    );
    assert!(
      request.contains("\"authors\":[\"@me\"]"),
      "request: {request}"
    );
    assert!(
      request.contains("\"review_status\":\"required\""),
      "request: {request}"
    );
    assert!(
      request.contains("\"include_drafts\":false"),
      "request: {request}"
    );
    assert!(request.contains("\"base\":\"main\""), "request: {request}");
    assert!(
      request.contains("\"sort\":\"comments_desc\""),
      "request: {request}"
    );
  }

  #[test]
  fn fetch_github_pull_request_filter_options_parses_success_payload() {
    let body = r#"{
      "options": {
        "labels": [{ "name": "bug" }],
        "authors": [{ "login": "octocat", "avatar_url": "https://example.com/octocat.png" }],
        "assignees": [{ "login": "alice", "avatar_url": null }]
      }
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let options = api
      .fetch_github_pull_request_filter_options(&["acme/reviu".to_string()])
      .expect("fetch pull request filter options");
    assert_eq!(options.labels.len(), 1);
    assert_eq!(options.labels[0].name, "bug");
    assert_eq!(options.authors.len(), 1);
    assert_eq!(options.authors[0].login, "octocat");
    assert_eq!(
      options.authors[0].avatar_url.as_deref(),
      Some("https://example.com/octocat.png")
    );
    assert_eq!(options.assignees.len(), 1);
    assert_eq!(options.assignees[0].login, "alice");
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_github_pull_request_filter_options_posts_expected_route_and_payload() {
    let body = r#"{"options":{"labels":[],"authors":[],"assignees":[]}}"#;
    let (base_url, request, handle) = start_single_response_server_with_request("200 OK", body);
    let api = make_test_api_client(base_url);

    let _ = api
      .fetch_github_pull_request_filter_options(&[
        "acme/reviu".to_string(),
        "acme/reviu-api".to_string(),
      ])
      .expect("fetch pull request filter options");

    handle.join().expect("join server thread");
    let request = request
      .lock()
      .expect("lock request")
      .clone()
      .unwrap_or_default();
    assert!(
      request.contains("POST /github/pr/filter-options "),
      "request: {request}"
    );
    assert!(
      request.contains("\"repos\":[\"acme/reviu\",\"acme/reviu-api\"]"),
      "request: {request}"
    );
  }

  #[test]
  fn fetch_github_user_repositories_parses_success_payload() {
    let body = r#"{
      "repositories": [
        {
          "owner": "acme",
          "repo": "portal",
          "full_name": "acme/portal",
          "description": "Main app",
          "private": true,
          "owner_avatar_url": "https://example.com/acme.png",
          "updated_at": "2026-02-20T14:12:00Z"
        }
      ]
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let repositories = api
      .fetch_github_user_repositories()
      .expect("fetch github user repositories");
    assert_eq!(repositories.len(), 1);
    assert_eq!(repositories[0].owner, "acme");
    assert_eq!(repositories[0].repo, "portal");
    assert_eq!(repositories[0].full_name, "acme/portal");
    assert_eq!(repositories[0].description.as_deref(), Some("Main app"));
    assert!(repositories[0].private);
    assert_eq!(
      repositories[0].owner_avatar_url.as_deref(),
      Some("https://example.com/acme.png")
    );
    assert_eq!(repositories[0].updated_at, "2026-02-20T14:12:00Z");
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_github_user_repositories_calls_expected_route_without_query_params() {
    let body = r#"{"repositories":[]}"#;
    let (base_url, request_line, handle) =
      start_single_response_server_with_request_line("200 OK", body);
    let api = make_test_api_client(base_url);

    let _ = api
      .fetch_github_user_repositories()
      .expect("fetch github user repositories");

    handle.join().expect("join server thread");
    let request_line = request_line
      .lock()
      .expect("lock request line")
      .clone()
      .unwrap_or_default();
    assert_eq!(request_line, "GET /github/repos/me HTTP/1.1");
  }

  #[test]
  fn fetch_github_user_profile_parses_success_payload() {
    let body = r##"{
      "login": "octocat",
      "name": "The Octocat",
      "avatar_url": "https://avatars.githubusercontent.com/u/583231?v=4",
      "bio": "GitHub mascot",
      "company": "@github",
      "location": "San Francisco",
      "website_url": "https://github.blog",
      "twitter_username": "octocat",
      "html_url": "https://github.com/octocat",
      "created_at": "2011-01-25T18:44:36Z",
      "followers_count": 99,
      "following_count": 5,
      "repositories_count": 2,
      "repositories_indexed_count": 2,
      "repositories_truncated": false,
      "stargazers_count": 20,
      "forks_count": 4,
      "languages": [
        { "name": "TypeScript", "color": "#3178c6", "size": 1000, "percentage": 50.0 }
      ],
      "repositories": [
        {
          "owner": "octocat",
          "repo": "hello-world",
          "full_name": "octocat/hello-world",
          "description": "Example repo",
          "private": false,
          "fork": false,
          "archived": false,
          "html_url": "https://github.com/octocat/hello-world",
          "language": "TypeScript",
          "language_color": "#3178c6",
          "stargazers_count": 12,
          "forks_count": 3,
          "updated_at": "2026-04-10T10:00:00Z",
          "pushed_at": "2026-04-10T09:00:00Z",
          "languages": [
            { "name": "TypeScript", "color": "#3178c6", "size": 700, "percentage": 70.0 }
          ]
        }
      ]
    }"##;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let profile = api
      .fetch_github_user_profile("octocat")
      .expect("fetch github user profile");

    assert_eq!(profile.login, "octocat");
    assert_eq!(profile.name.as_deref(), Some("The Octocat"));
    assert_eq!(profile.followers_count, 99);
    assert_eq!(profile.stargazers_count, 20);
    assert_eq!(profile.languages.len(), 1);
    assert_eq!(profile.languages[0].name, "TypeScript");
    assert_eq!(profile.repositories.len(), 1);
    assert_eq!(profile.repositories[0].full_name, "octocat/hello-world");
    assert_eq!(
      profile.repositories[0].language.as_deref(),
      Some("TypeScript")
    );
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_github_user_profile_calls_expected_route() {
    let body = r#"{
      "login": "octocat",
      "name": null,
      "avatar_url": null,
      "bio": null,
      "company": null,
      "location": null,
      "website_url": null,
      "twitter_username": null,
      "html_url": "https://github.com/octocat",
      "created_at": "2011-01-25T18:44:36Z",
      "followers_count": 0,
      "following_count": 0,
      "repositories_count": 0,
      "repositories_indexed_count": 0,
      "repositories_truncated": false,
      "stargazers_count": 0,
      "forks_count": 0,
      "languages": [],
      "repositories": []
    }"#;
    let (base_url, request_line, handle) =
      start_single_response_server_with_request_line("200 OK", body);
    let api = make_test_api_client(base_url);

    let _ = api
      .fetch_github_user_profile("octocat")
      .expect("fetch github user profile");

    handle.join().expect("join server thread");
    let request_line = request_line
      .lock()
      .expect("lock request line")
      .clone()
      .unwrap_or_default();
    assert_eq!(request_line, "GET /github/users/octocat HTTP/1.1");
  }

  #[test]
  fn fetch_github_repository_details_parses_success_payload() {
    let body = r#"{
      "name": "widget",
      "full_name": "acme/widget",
      "description": "A sample repository",
      "homepage": "https://acme.dev/widget",
      "language": "Rust",
      "default_branch": "main",
      "stargazers_count": 123,
      "forks_count": 45,
      "subscribers_count": 6,
      "open_issues_count": 7,
      "size": 2048,
      "pushed_at": "2026-02-20T12:00:00Z",
      "html_url": "https://github.com/acme/widget",
      "owner": {
        "login": "acme",
        "avatar_url": "https://example.com/avatar.png"
      },
      "recent_commits": [
        {
          "sha": "abc123",
          "message": "feat: add overview authors",
          "committed_at": "2026-04-20T12:00:00Z",
          "author_login": "octocat",
          "author_avatar_url": "https://example.com/octocat.png",
          "authors": [
            {
              "name": "Octo Cat",
              "email": "octocat@example.com",
              "login": "octocat",
              "avatar_url": "https://example.com/octocat.png"
            },
            {
              "name": "Co Author",
              "email": "coauthor@example.com",
              "login": null,
              "avatar_url": null
            }
          ]
        }
      ],
      "license": {
        "key": "mit",
        "name": "MIT License",
        "spdx_id": "MIT"
      }
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let details = api
      .fetch_github_repository_details("acme", "widget")
      .expect("fetch repository details");
    assert_eq!(details.full_name, "acme/widget");
    assert_eq!(details.owner.login, "acme");
    assert_eq!(details.default_branch, "main");
    assert_eq!(details.stargazers_count, 123);
    assert_eq!(details.recent_commits.len(), 1);
    assert_eq!(details.recent_commits[0].authors.len(), 2);
    assert_eq!(
      details.recent_commits[0].authors[1].email.as_deref(),
      Some("coauthor@example.com")
    );
    assert_eq!(
      details
        .license
        .as_ref()
        .map(|license| license.name.as_str()),
      Some("MIT License")
    );
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_github_commit_parses_success_payload() {
    let body = r#"{
      "commit": {
        "sha": "abc123",
        "message": "feat: add commit page\n\nOpen commits in Reviu.",
        "html_url": "https://github.com/acme/widget/commit/abc123",
        "authored_at": "2026-03-01T10:00:00Z",
        "committed_at": "2026-03-01T10:05:00Z",
        "parent_sha": "parent123",
        "author": { "login": "alice", "avatar_url": "https://example.com/alice.png" },
        "committer": null,
        "stats": { "additions": 12, "deletions": 3, "total": 15 },
        "files": [
          {
            "filename": "src/lib.rs",
            "status": "modified",
            "patch": "@@ -1 +1 @@",
            "previous_filename": null
          }
        ]
      }
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let commit = api
      .fetch_github_commit("acme", "widget", "abc123")
      .expect("fetch commit");

    assert_eq!(commit.sha, "abc123");
    assert_eq!(commit.parent_sha.as_deref(), Some("parent123"));
    assert_eq!(
      commit.author.as_ref().map(|user| user.login.as_str()),
      Some("alice")
    );
    assert_eq!(commit.stats.as_ref().map(|stats| stats.total), Some(15));
    assert_eq!(commit.files.len(), 1);
    assert_eq!(commit.files[0].filename, "src/lib.rs");
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_github_commit_calls_expected_route_with_query_params() {
    let body = r#"{
      "commit": {
        "sha": "abc123",
        "message": "feat: add commit page",
        "html_url": "https://github.com/acme/widget/commit/abc123",
        "authored_at": null,
        "committed_at": null,
        "parent_sha": null,
        "author": null,
        "committer": null,
        "stats": null,
        "files": []
      }
    }"#;
    let (base_url, request_line, handle) =
      start_single_response_server_with_request_line("200 OK", body);
    let api = make_test_api_client(base_url);

    let _ = api
      .fetch_github_commit("acme", "widget", "abc123")
      .expect("fetch commit");

    handle.join().expect("join server thread");
    let request_line = request_line
      .lock()
      .expect("lock request line")
      .clone()
      .unwrap_or_default();
    assert_eq!(
      request_line,
      "GET /github/commit?org=acme&repo=widget&sha=abc123 HTTP/1.1"
    );
  }

  #[test]
  fn fetch_github_repository_details_parses_languages() {
    let body = r##"{
      "name": "widget",
      "full_name": "acme/widget",
      "description": "A sample repository",
      "homepage": null,
      "language": "TypeScript",
      "default_branch": "main",
      "stargazers_count": 100,
      "forks_count": 10,
      "subscribers_count": 5,
      "open_issues_count": 3,
      "size": 1024,
      "pushed_at": "2026-02-20T12:00:00Z",
      "html_url": "https://github.com/acme/widget",
      "owner": {
        "login": "acme",
        "avatar_url": "https://example.com/avatar.png"
      },
      "license": null,
      "languages": [
        { "name": "TypeScript", "color": "#3178c6", "size": 45230, "percentage": 78.5 },
        { "name": "Rust", "color": "#dea584", "size": 12400, "percentage": 21.5 }
      ]
    }"##;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let details = api
      .fetch_github_repository_details("acme", "widget")
      .expect("fetch repository details");
    assert_eq!(details.languages.len(), 2);
    assert_eq!(details.languages[0].name, "TypeScript");
    assert_eq!(details.languages[0].color.as_deref(), Some(r"#3178c6"));
    assert_eq!(details.languages[0].percentage, 78.5);
    assert_eq!(details.languages[1].name, "Rust");
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_github_repository_details_defaults_empty_languages() {
    let body = r#"{
      "name": "widget",
      "full_name": "acme/widget",
      "description": null,
      "homepage": null,
      "language": null,
      "default_branch": "main",
      "stargazers_count": 0,
      "forks_count": 0,
      "subscribers_count": 0,
      "open_issues_count": 0,
      "size": 0,
      "pushed_at": null,
      "html_url": "https://github.com/acme/widget",
      "owner": { "login": "acme", "avatar_url": null },
      "license": null
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let details = api
      .fetch_github_repository_details("acme", "widget")
      .expect("fetch repository details");
    assert!(details.languages.is_empty());
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_github_repository_pull_requests_parses_success_payload() {
    let body = r#"{
      "pullRequests": [
        {
          "number": 11,
          "title": "Improve docs",
          "state": "open",
          "created_at": "2026-02-11T08:00:00Z",
          "closed_at": null,
          "merged_at": null,
          "draft": false,
          "updated_at": "2026-02-15T12:00:00Z",
          "comments_count": 7,
          "author": {
            "login": "docs-bot[bot]",
            "avatar_url": null,
            "is_bot": true
          },
          "labels": [{ "name": "docs", "color": "0075ca" }],
          "repository": { "owner": "acme", "repo": "widget" }
        }
      ],
      "pullRequestCount": 42
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);
    let filters = GithubPullRequestSearchFilters::default();

    let response = api
      .fetch_github_repository_pull_requests("acme", "widget", &filters, "open", 1)
      .expect("fetch repository pull requests");
    assert_eq!(response.pull_request_count, 42);
    assert_eq!(response.pull_requests.len(), 1);
    assert_eq!(response.pull_requests[0].number, 11);
    assert_eq!(response.pull_requests[0].title, "Improve docs");
    assert_eq!(response.pull_requests[0].created_at, "2026-02-11T08:00:00Z");
    assert_eq!(response.pull_requests[0].comments_count, 7);
    assert_eq!(response.pull_requests[0].author.login, "docs-bot[bot]");
    assert!(response.pull_requests[0].author.is_bot);
    assert_eq!(
      response.pull_requests[0].labels[0].color.as_deref(),
      Some("0075ca")
    );
    assert_eq!(response.pull_requests[0].repository.owner, "acme");
    assert_eq!(response.pull_requests[0].repository.repo, "widget");
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_github_repository_pull_requests_uses_filter_query_params() {
    let body = r#"{"pullRequests":[],"pullRequestCount":0}"#;
    let (base_url, request_line, handle) =
      start_single_response_server_with_request_line("200 OK", body);
    let api = make_test_api_client(base_url);
    let filters = GithubPullRequestSearchFilters {
      labels: vec!["needs design".to_string()],
      excluded_labels: vec!["team/reveal".to_string()],
      authors: vec!["@me".to_string()],
      assignees: vec!["alice".to_string()],
      requested_reviewers: vec!["bob".to_string()],
      review_status: crate::github_home_tabs::GithubPullRequestReviewStatus::ChangesRequested,
      include_drafts: false,
      base: Some("feature/ui".to_string()),
      sort: GithubPullRequestSearchSort::CommentsDesc,
      ..GithubPullRequestSearchFilters::default()
    };

    let _ = api
      .fetch_github_repository_pull_requests("acme", "widget", &filters, "open", 1)
      .expect("fetch repository pull requests");

    handle.join().expect("join server thread");
    let request_line = request_line
      .lock()
      .expect("lock request line")
      .clone()
      .unwrap_or_default();
    assert!(
      request_line.starts_with("GET /github/repos/acme/widget/pr?"),
      "request_line: {request_line}"
    );
    assert!(
      request_line.contains("label=needs+design"),
      "request_line: {request_line}"
    );
    assert!(
      request_line.contains("exclude_label=team%2Freveal"),
      "request_line: {request_line}"
    );
    assert!(
      request_line.contains("author=%40me"),
      "request_line: {request_line}"
    );
    assert!(
      request_line.contains("assignee=alice"),
      "request_line: {request_line}"
    );
    assert!(
      request_line.contains("requested_reviewer=bob"),
      "request_line: {request_line}"
    );
    assert!(
      request_line.contains("review_status=changes_requested"),
      "request_line: {request_line}"
    );
    assert!(
      request_line.contains("include_drafts=false"),
      "request_line: {request_line}"
    );
    assert!(
      request_line.contains("base=feature%2Fui"),
      "request_line: {request_line}"
    );
    assert!(
      request_line.contains("sort=comments_desc"),
      "request_line: {request_line}"
    );
    assert!(
      request_line.contains("state=open"),
      "request_line: {request_line}"
    );
  }

  #[test]
  fn fetch_github_repository_issues_parses_success_payload() {
    let body = r#"{
      "issues": [
        {
          "id": 1,
          "number": 42,
          "title": "Fix login bug",
          "state": "open",
          "state_reason": null,
          "created_at": "2026-03-01T10:00:00Z",
          "updated_at": "2026-03-05T14:00:00Z",
          "closed_at": null,
          "labels": [{ "name": "bug", "color": "d73a4a" }],
          "comments_count": 3,
          "user": {
            "login": "alice",
            "avatar_url": "https://example.com/alice.png"
          },
          "repository": { "owner": "acme", "repo": "widget" }
        }
      ],
      "issueCount": 15
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let response = api
      .fetch_github_repository_issues(
        "acme",
        "widget",
        "open",
        &GithubIssueSearchFilters::default(),
        1,
      )
      .expect("fetch repository issues");
    assert_eq!(response.issue_count, 15);
    assert_eq!(response.issues.len(), 1);
    assert_eq!(response.issues[0].number, 42);
    assert_eq!(response.issues[0].title, "Fix login bug");
    assert_eq!(
      response.issues[0].labels[0].color.as_deref(),
      Some("d73a4a")
    );
    assert_eq!(response.issues[0].repository.owner, "acme");
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_github_repository_issues_sends_state_query_param() {
    let body = r#"{"issues":[],"issueCount":0}"#;
    let (base_url, request_line, handle) =
      start_single_response_server_with_request_line("200 OK", body);
    let api = make_test_api_client(base_url);

    let _ = api
      .fetch_github_repository_issues(
        "acme",
        "widget",
        "closed",
        &GithubIssueSearchFilters::default(),
        1,
      )
      .expect("fetch repository issues");

    handle.join().expect("join server thread");
    let request_line = request_line
      .lock()
      .expect("lock request line")
      .clone()
      .unwrap_or_default();
    assert!(
      request_line.starts_with("GET /github/repos/acme/widget/issues?"),
      "request_line: {request_line}"
    );
    assert!(
      request_line.contains("state=closed"),
      "request_line: {request_line}"
    );
  }

  #[test]
  fn fetch_pull_request_for_branch_parses_success_payload() {
    let body = r#"{
      "pullRequest": {
        "number": 11,
        "title": "Improve docs",
        "state": "open",
        "created_at": "2026-02-11T08:00:00Z",
        "closed_at": null,
        "merged_at": null,
        "draft": false,
        "updated_at": "2026-02-15T12:00:00Z",
        "comments_count": 7,
        "author": {
          "login": "docs-bot[bot]",
          "avatar_url": null,
          "is_bot": true
        },
        "labels": [{ "name": "docs" }],
        "repository": { "owner": "acme", "repo": "widget" }
      }
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let pull_request = api
      .fetch_pull_request_for_branch("acme", "widget", "feature/parser")
      .expect("fetch branch pull request")
      .expect("pull request payload");
    assert_eq!(pull_request.number, 11);
    assert_eq!(pull_request.title, "Improve docs");
    assert_eq!(pull_request.repository.owner, "acme");
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_pull_request_for_branch_returns_none_when_branch_has_no_open_pull_request() {
    let body = r#"{
      "pullRequest": null
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let pull_request = api
      .fetch_pull_request_for_branch("acme", "widget", "feature/parser")
      .expect("fetch branch pull request");
    assert!(pull_request.is_none());
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_pull_request_for_branch_uses_expected_route_with_branch_query() {
    let body = r#"{"pullRequest":null}"#;
    let (base_url, request_line, handle) =
      start_single_response_server_with_request_line("200 OK", body);
    let api = make_test_api_client(base_url);

    let _ = api
      .fetch_pull_request_for_branch("acme", "widget", "feature/parser")
      .expect("fetch branch pull request");

    handle.join().expect("join server thread");
    let request_line = request_line
      .lock()
      .expect("lock request line")
      .clone()
      .unwrap_or_default();
    assert_eq!(
      request_line,
      "GET /github/repos/acme/widget/pr/branch?branch=feature%2Fparser HTTP/1.1"
    );
  }

  #[test]
  fn create_pull_request_parses_success_payload() {
    let body = r#"{
      "pullRequest": {
        "number": 11,
        "title": "Improve docs",
        "state": "open",
        "created_at": "2026-02-11T08:00:00Z",
        "closed_at": null,
        "merged_at": null,
        "draft": true,
        "updated_at": "2026-02-15T12:00:00Z",
        "comments_count": 7,
        "author": {
          "login": "docs-bot[bot]",
          "avatar_url": null,
          "is_bot": true
        },
        "labels": [{ "name": "docs" }],
        "repository": { "owner": "acme", "repo": "widget" }
      }
    }"#;
    let (base_url, handle) = start_single_response_server("201 Created", body);
    let api = make_test_api_client(base_url);

    let pull_request = api
      .create_pull_request(
        "acme",
        "widget",
        "feature/parser",
        "Improve docs",
        "main",
        Some("Ready to review"),
        true,
      )
      .expect("create pull request");
    assert_eq!(pull_request.number, 11);
    assert_eq!(pull_request.title, "Improve docs");
    assert!(pull_request.draft);
    handle.join().expect("join server thread");
  }

  #[test]
  fn create_pull_request_uses_expected_route_and_payload() {
    let body = r#"{"pullRequest":{"number":11,"title":"Improve docs","state":"open","created_at":"2026-02-11T08:00:00Z","closed_at":null,"merged_at":null,"draft":false,"updated_at":"2026-02-15T12:00:00Z","comments_count":0,"author":{"login":"octocat","avatar_url":null,"is_bot":false},"labels":[],"repository":{"owner":"acme","repo":"widget"}}}"#;
    let (base_url, request, handle) =
      start_single_response_server_with_request("201 Created", body);
    let api = make_test_api_client(base_url);

    let _ = api
      .create_pull_request(
        "acme",
        "widget",
        "feature/parser",
        "  Improve docs  ",
        "  main  ",
        Some("  Ready to review  "),
        true,
      )
      .expect("create pull request");

    handle.join().expect("join server thread");
    let request = request
      .lock()
      .expect("lock request")
      .clone()
      .unwrap_or_default();
    assert!(
      request.starts_with("POST /github/repos/acme/widget/pr?branch=feature%2Fparser HTTP/1.1")
    );
    assert!(
      request
        .to_lowercase()
        .contains("\r\ncontent-type: application/json")
    );
    assert!(request.contains(r#""title":"Improve docs""#));
    assert!(request.contains(r#""base":"main""#));
    assert!(request.contains(r#""body":"Ready to review""#));
    assert!(request.contains(r#""draft":true"#));
  }

  #[test]
  fn fetch_github_repository_tree_parses_success_payload() {
    let body = r#"{
      "sha": "9fb037999f264ba9a7fc6274d15fa3ae2ab98312",
      "url": "https://api.github.com/repos/octocat/Hello-World/trees/9fb037999f264ba9a7fc6274d15fa3ae2ab98312",
      "tree": [
        {
          "path": "file.rb",
          "mode": "100644",
          "type": "blob",
          "size": 30,
          "sha": "44b4fc6d56897b048c772eb4087f854f46256132",
          "url": "https://api.github.com/repos/octocat/Hello-World/git/blobs/44b4fc6d56897b048c772eb4087f854f46256132"
        },
        {
          "path": "subdir",
          "mode": "040000",
          "type": "tree",
          "sha": "f484d249c660418515fb01c2b9662073663c242e",
          "url": "https://api.github.com/repos/octocat/Hello-World/git/trees/f484d249c660418515fb01c2b9662073663c242e"
        }
      ],
      "truncated": false
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let tree = api
      .fetch_github_repository_tree("acme", "widget", "main")
      .expect("fetch repository tree");
    assert_eq!(tree.sha, "9fb037999f264ba9a7fc6274d15fa3ae2ab98312");
    assert_eq!(tree.tree.len(), 2);
    assert_eq!(tree.tree[0].path, "file.rb");
    assert_eq!(tree.tree[0].entry_type, "blob");
    assert_eq!(tree.tree[1].path, "subdir");
    assert_eq!(tree.tree[1].entry_type, "tree");
    assert!(!tree.truncated);
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_github_repository_tree_uses_recursive_query() {
    let body = r#"{
      "sha": "9fb037999f264ba9a7fc6274d15fa3ae2ab98312",
      "url": "https://api.github.com/repos/octocat/Hello-World/trees/9fb037999f264ba9a7fc6274d15fa3ae2ab98312",
      "tree": [],
      "truncated": false
    }"#;
    let (base_url, request_line, handle) =
      start_single_response_server_with_request_line("200 OK", body);
    let api = make_test_api_client(base_url);

    let _ = api
      .fetch_github_repository_tree("acme", "widget", "main")
      .expect("fetch repository tree");

    handle.join().expect("join server thread");
    let request_line = request_line
      .lock()
      .expect("lock request line")
      .clone()
      .unwrap_or_default();
    assert_eq!(
      request_line,
      "GET /github/repos/acme/widget/trees/main?recursive=1 HTTP/1.1"
    );
  }

  #[test]
  fn fetch_github_repository_branches_parses_success_payload() {
    let body = r#"[
      {
        "name": "main",
        "commit": {
          "sha": "1111111111111111111111111111111111111111",
          "url": "https://api.github.com/repos/acme/widget/commits/1111111111111111111111111111111111111111"
        },
        "protected": true
      },
      {
        "name": "feature/new-ui",
        "commit": {
          "sha": "2222222222222222222222222222222222222222",
          "url": "https://api.github.com/repos/acme/widget/commits/2222222222222222222222222222222222222222"
        },
        "protected": false
      }
    ]"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let branches = api
      .fetch_github_repository_branches("acme", "widget")
      .expect("fetch repository branches");
    assert_eq!(branches.len(), 2);
    assert_eq!(branches[0].name, "main");
    assert_eq!(
      branches[0].commit.sha,
      "1111111111111111111111111111111111111111"
    );
    assert!(branches[0].protected);
    assert_eq!(branches[1].name, "feature/new-ui");
    assert!(!branches[1].protected);
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_github_repository_branches_uses_branches_route() {
    let body = r#"[]"#;
    let (base_url, request_line, handle) =
      start_single_response_server_with_request_line("200 OK", body);
    let api = make_test_api_client(base_url);

    let _ = api
      .fetch_github_repository_branches("acme", "widget")
      .expect("fetch repository branches");

    handle.join().expect("join server thread");
    let request_line = request_line
      .lock()
      .expect("lock request line")
      .clone()
      .unwrap_or_default();
    assert_eq!(
      request_line,
      "GET /github/repos/acme/widget/branches HTTP/1.1"
    );
  }

  #[test]
  fn fetch_github_repository_readme_parses_success_payload() {
    let body = r##"{"content":"# Widget\n\nHello README","path":"docs/README.md"}"##;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let readme = api
      .fetch_github_repository_readme("acme", "widget", Some("main"))
      .expect("fetch repository readme");
    let readme = readme.expect("readme payload");
    assert_eq!(readme.content.as_deref(), Some("# Widget\n\nHello README"));
    assert_eq!(readme.path.as_deref(), Some("docs/README.md"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_github_repository_readme_uses_ref_query_when_provided() {
    let body = r##"{"content":"# Widget"}"##;
    let (base_url, request_line, handle) =
      start_single_response_server_with_request_line("200 OK", body);
    let api = make_test_api_client(base_url);

    let _ = api
      .fetch_github_repository_readme("acme", "widget", Some("main"))
      .expect("fetch repository readme");

    handle.join().expect("join server thread");
    let request_line = request_line
      .lock()
      .expect("lock request line")
      .clone()
      .unwrap_or_default();
    assert_eq!(
      request_line,
      "GET /github/repos/acme/widget/readme?ref=main HTTP/1.1"
    );
  }

  #[test]
  fn fetch_github_repository_readme_returns_none_on_not_found() {
    let body = r#"{"error":"Repository not found"}"#;
    let (base_url, handle) = start_single_response_server("404 Not Found", body);
    let api = make_test_api_client(base_url);

    let readme = api
      .fetch_github_repository_readme("acme", "widget", Some("main"))
      .expect("fetch repository readme");
    assert!(readme.is_none());
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_github_repository_issue_details_parses_success_payload() {
    let body = r#"{
      "issue": {
        "node_id": "I_kwDOExample",
        "reactions": [
          { "content": "THUMBS_UP", "count": 2, "viewer_has_reacted": true }
        ],
        "id": 501,
        "number": 77,
        "title": "Fix auth race condition",
        "body": "Issue body",
        "state": "closed",
        "state_reason": "completed",
        "created_at": "2026-02-20T08:00:00Z",
        "updated_at": "2026-02-21T09:30:00Z",
        "closed_at": "2026-02-21T09:30:00Z",
        "labels": [{ "name": "bug", "color": "f29513" }],
        "comments": [
          {
            "node_id": "IC_kwDOIssue",
            "reactions": [
              { "content": "HEART", "count": 1, "viewer_has_reacted": false }
            ],
            "id": 9001,
            "body": "Looks good",
            "created_at": "2026-02-20T10:00:00Z",
            "updated_at": "2026-02-20T10:05:00Z",
            "user": {
              "login": "octocat",
              "name": "The Octocat",
              "avatar_url": "https://example.com/octocat.png"
            }
          }
        ],
        "user": {
          "login": "octocat",
          "name": "The Octocat",
          "avatar_url": "https://example.com/octocat.png"
        },
        "repository": { "owner": "acme", "repo": "widget" }
      }
    }"#;
    let (base_url, request_line, handle) =
      start_single_response_server_with_request_line("200 OK", body);
    let api = make_test_api_client(base_url);

    let issue = api
      .fetch_github_repository_issue_details("acme", "widget", 77)
      .expect("fetch repository issue details");

    assert_eq!(issue.number, 77);
    assert_eq!(issue.node_id, "I_kwDOExample");
    assert_eq!(issue.reactions[0].content, GithubReactionContent::ThumbsUp);
    assert_eq!(issue.title, "Fix auth race condition");
    assert_eq!(issue.state, "closed");
    assert_eq!(issue.state_reason, Some(GithubIssueStateReason::Completed));
    assert_eq!(issue.labels[0].color.as_deref(), Some("f29513"));
    assert_eq!(issue.comments.len(), 1);
    assert_eq!(issue.comments[0].id, 9001);
    assert_eq!(issue.comments[0].node_id, "IC_kwDOIssue");
    assert_eq!(
      issue.comments[0].reactions[0].content,
      GithubReactionContent::Heart
    );
    assert_eq!(issue.comments[0].body.as_deref(), Some("Looks good"));
    assert_eq!(
      issue.comments[0]
        .user
        .as_ref()
        .map(|user| user.login.as_str()),
      Some("octocat")
    );
    assert_eq!(issue.repository.owner, "acme");
    assert_eq!(issue.repository.repo, "widget");

    handle.join().expect("join server thread");
    let request_line = request_line
      .lock()
      .expect("lock request line")
      .clone()
      .unwrap_or_default();
    assert_eq!(
      request_line,
      "GET /github/repos/acme/widget/issues/77 HTTP/1.1"
    );
  }

  #[test]
  fn resolve_github_issue_reference_target_parses_pull_request_target() {
    let body = r#"{
      "target": {
        "kind": "pull_request",
        "number": 24877
      }
    }"#;
    let (base_url, request_line, handle) =
      start_single_response_server_with_request_line("200 OK", body);
    let api = make_test_api_client(base_url);

    let target = api
      .resolve_github_issue_reference_target("acme", "widget", 24877)
      .expect("resolve issue reference target");

    assert_eq!(target.kind, GithubIssueReferenceTargetKind::PullRequest);
    assert_eq!(target.number, 24877);

    handle.join().expect("join server thread");
    let request_line = request_line
      .lock()
      .expect("lock request line")
      .clone()
      .unwrap_or_default();
    assert_eq!(
      request_line,
      "GET /github/repos/acme/widget/issues/24877/target HTTP/1.1"
    );
  }

  #[test]
  fn update_issue_description_parses_success_payload() {
    let body = r#"{
      "issue": {
        "id": 501,
        "number": 77,
        "body": "Updated issue description",
        "updated_at": "2026-02-22T09:30:00Z"
      }
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let issue = api
      .update_issue_description("acme", "widget", 77, "Updated issue description")
      .expect("update issue description");
    assert_eq!(issue.id, 501);
    assert_eq!(issue.number, 77);
    assert_eq!(issue.body.as_deref(), Some("Updated issue description"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn update_issue_description_uses_expected_route() {
    let body = r#"{
      "issue": {
        "id": 501,
        "number": 77,
        "body": "",
        "updated_at": "2026-02-22T09:30:00Z"
      }
    }"#;
    let (base_url, request_line, handle) =
      start_single_response_server_with_request_line("200 OK", body);
    let api = make_test_api_client(base_url);

    let _ = api
      .update_issue_description("acme", "widget", 77, "")
      .expect("update issue description");

    handle.join().expect("join server thread");
    let request_line = request_line
      .lock()
      .expect("lock request line")
      .clone()
      .unwrap_or_default();
    assert_eq!(
      request_line,
      "PATCH /github/repos/acme/widget/issues/77 HTTP/1.1"
    );
  }

  #[test]
  fn create_issue_comment_parses_success_payload() {
    let body = r#"{
      "comment": {
        "id": 9002,
        "body": "New issue comment",
        "created_at": "2026-02-20T10:00:00Z",
        "updated_at": "2026-02-20T10:05:00Z",
        "user": {
          "login": "octocat",
          "name": "The Octocat",
          "avatar_url": "https://example.com/octocat.png"
        }
      }
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let comment = api
      .create_issue_comment("acme", "widget", 77, "New issue comment")
      .expect("create issue comment");
    assert_eq!(comment.id, 9002);
    assert_eq!(comment.body.as_deref(), Some("New issue comment"));
    assert_eq!(
      comment.user.as_ref().map(|user| user.login.as_str()),
      Some("octocat")
    );
    handle.join().expect("join server thread");
  }

  #[test]
  fn create_issue_comment_uses_expected_route() {
    let body = r#"{
      "comment": {
        "id": 9002,
        "body": "New issue comment",
        "created_at": "2026-02-20T10:00:00Z",
        "updated_at": "2026-02-20T10:05:00Z",
        "user": null
      }
    }"#;
    let (base_url, request_line, handle) =
      start_single_response_server_with_request_line("200 OK", body);
    let api = make_test_api_client(base_url);

    let _ = api
      .create_issue_comment("acme", "widget", 77, "New issue comment")
      .expect("create issue comment");

    handle.join().expect("join server thread");
    let request_line = request_line
      .lock()
      .expect("lock request line")
      .clone()
      .unwrap_or_default();
    assert_eq!(
      request_line,
      "POST /github/repos/acme/widget/issues/77/comments HTTP/1.1"
    );
  }

  #[test]
  fn update_issue_comment_parses_success_payload() {
    let body = r#"{
      "comment": {
        "id": 9002,
        "body": "Updated issue comment",
        "created_at": "2026-02-20T10:00:00Z",
        "updated_at": "2026-02-20T11:00:00Z",
        "user": {
          "login": "octocat",
          "name": "The Octocat",
          "avatar_url": "https://example.com/octocat.png"
        }
      }
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let comment = api
      .update_issue_comment("acme", "widget", 77, 9002, "Updated issue comment")
      .expect("update issue comment");
    assert_eq!(comment.id, 9002);
    assert_eq!(comment.body.as_deref(), Some("Updated issue comment"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn update_issue_comment_uses_expected_route() {
    let body = r#"{
      "comment": {
        "id": 9002,
        "body": "Updated issue comment",
        "created_at": "2026-02-20T10:00:00Z",
        "updated_at": "2026-02-20T11:00:00Z",
        "user": null
      }
    }"#;
    let (base_url, request_line, handle) =
      start_single_response_server_with_request_line("200 OK", body);
    let api = make_test_api_client(base_url);

    let _ = api
      .update_issue_comment("acme", "widget", 77, 9002, "Updated issue comment")
      .expect("update issue comment");

    handle.join().expect("join server thread");
    let request_line = request_line
      .lock()
      .expect("lock request line")
      .clone()
      .unwrap_or_default();
    assert_eq!(
      request_line,
      "PATCH /github/repos/acme/widget/issues/77/comments/9002 HTTP/1.1"
    );
  }

  #[test]
  fn delete_issue_comment_returns_ok_on_success() {
    let (base_url, handle) = start_single_response_server("200 OK", "{}");
    let api = make_test_api_client(base_url);

    api
      .delete_issue_comment("acme", "widget", 77, 9002)
      .expect("delete issue comment");
    handle.join().expect("join server thread");
  }

  #[test]
  fn delete_issue_comment_uses_expected_route() {
    let (base_url, request_line, handle) =
      start_single_response_server_with_request_line("200 OK", "{}");
    let api = make_test_api_client(base_url);

    api
      .delete_issue_comment("acme", "widget", 77, 9002)
      .expect("delete issue comment");

    handle.join().expect("join server thread");
    let request_line = request_line
      .lock()
      .expect("lock request line")
      .clone()
      .unwrap_or_default();
    assert_eq!(
      request_line,
      "DELETE /github/repos/acme/widget/issues/77/comments/9002 HTTP/1.1"
    );
  }

  #[test]
  fn fetch_pull_request_details_parses_success_payload() {
    let body = r#"{
      "pullRequest": {
        "node_id": "PR_kwDOExample",
        "number": 42,
        "title": "Improve parser",
        "state": "open",
        "draft": false,
        "created_at": "2026-02-10T09:00:00Z",
        "updated_at": "2026-02-15T12:00:00Z",
        "merged_at": null,
        "merge_base_sha": "abc123",
        "base_sha": "base123",
        "head_sha": "head123",
        "base_ref_name": "main",
        "head_ref_name": "feature/parser",
        "body": "PR body",
        "author": { "login": "octocat", "avatar_url": null, "is_bot": false },
        "assignees": [{ "login": "alice", "avatar_url": "https://example.com/alice.png" }],
        "requested_reviewers": [{ "login": "bob", "avatar_url": null }],
        "comments": 2,
        "review_comments": 3,
        "commits": 4,
        "additions": 10,
        "deletions": 5,
        "changed_files": 2,
        "labels": [{ "name": "enhancement", "color": "a2eeef" }],
        "repository": { "owner": "acme", "repo": "widget" },
        "head_repository": { "owner": "acme", "repo": "widget-fork" }
      }
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let details = api
      .fetch_pull_request_details("acme", "widget", 42)
      .expect("fetch pull request details");
    assert_eq!(details.node_id, "PR_kwDOExample");
    assert_eq!(details.number, 42);
    assert_eq!(details.head_ref_name, "feature/parser");
    assert_eq!(details.author.login, "octocat");
    assert!(!details.author.is_bot);
    assert_eq!(details.assignees[0].login, "alice");
    assert_eq!(details.requested_reviewers[0].login, "bob");
    assert_eq!(details.comments, 2);
    assert_eq!(details.review_comments, 3);
    assert_eq!(details.labels[0].color.as_deref(), Some("a2eeef"));
    assert_eq!(
      details
        .head_repository
        .as_ref()
        .expect("head repo")
        .repo
        .as_str(),
      "widget-fork"
    );
    handle.join().expect("join server thread");
  }

  #[test]
  fn add_pull_request_assignee_uses_expected_route_and_payload() {
    let (base_url, request, handle) =
      start_single_response_server_with_request("204 NO CONTENT", "");
    let api = make_test_api_client(base_url);

    api
      .add_pull_request_assignee("acme", "widget", 42, "alice")
      .expect("add pull request assignee");

    handle.join().expect("join server thread");
    let request = request
      .lock()
      .expect("lock request")
      .clone()
      .unwrap_or_default();
    assert!(request.contains("POST /github/pr/42/assignees?org=acme&repo=widget HTTP/1.1"));
    assert!(request.contains("\"users\":[\"alice\"]"));
  }

  #[test]
  fn remove_pull_request_assignee_uses_expected_route_and_payload() {
    let (base_url, request, handle) =
      start_single_response_server_with_request("204 NO CONTENT", "");
    let api = make_test_api_client(base_url);

    api
      .remove_pull_request_assignee("acme", "widget", 42, "alice")
      .expect("remove pull request assignee");

    handle.join().expect("join server thread");
    let request = request
      .lock()
      .expect("lock request")
      .clone()
      .unwrap_or_default();
    assert!(request.contains("DELETE /github/pr/42/assignees?org=acme&repo=widget HTTP/1.1"));
    assert!(request.contains("\"users\":[\"alice\"]"));
  }

  #[test]
  fn add_pull_request_label_uses_expected_route_and_payload() {
    let (base_url, request, handle) =
      start_single_response_server_with_request("204 NO CONTENT", "");
    let api = make_test_api_client(base_url);

    api
      .add_pull_request_label("acme", "widget", 42, "bug")
      .expect("add pull request label");

    handle.join().expect("join server thread");
    let request = request
      .lock()
      .expect("lock request")
      .clone()
      .unwrap_or_default();
    assert!(request.contains("POST /github/pr/42/labels?org=acme&repo=widget HTTP/1.1"));
    assert!(request.contains("\"labels\":[\"bug\"]"));
  }

  #[test]
  fn remove_pull_request_label_uses_expected_route_and_payload() {
    let (base_url, request, handle) =
      start_single_response_server_with_request("204 NO CONTENT", "");
    let api = make_test_api_client(base_url);

    api
      .remove_pull_request_label("acme", "widget", 42, "bug")
      .expect("remove pull request label");

    handle.join().expect("join server thread");
    let request = request
      .lock()
      .expect("lock request")
      .clone()
      .unwrap_or_default();
    assert!(request.contains("DELETE /github/pr/42/labels?org=acme&repo=widget HTTP/1.1"));
    assert!(request.contains("\"labels\":[\"bug\"]"));
  }

  #[test]
  fn request_pull_request_reviewer_uses_expected_route_and_payload() {
    let (base_url, request, handle) =
      start_single_response_server_with_request("204 NO CONTENT", "");
    let api = make_test_api_client(base_url);

    api
      .request_pull_request_reviewer("acme", "widget", 42, "bob")
      .expect("request pull request reviewer");

    handle.join().expect("join server thread");
    let request = request
      .lock()
      .expect("lock request")
      .clone()
      .unwrap_or_default();
    assert!(
      request.contains("POST /github/pr/42/requested-reviewers?org=acme&repo=widget HTTP/1.1")
    );
    assert!(request.contains("\"users\":[\"bob\"]"));
  }

  #[test]
  fn remove_pull_request_reviewer_uses_expected_route_and_payload() {
    let (base_url, request, handle) =
      start_single_response_server_with_request("204 NO CONTENT", "");
    let api = make_test_api_client(base_url);

    api
      .remove_pull_request_reviewer("acme", "widget", 42, "bob")
      .expect("remove pull request reviewer");

    handle.join().expect("join server thread");
    let request = request
      .lock()
      .expect("lock request")
      .clone()
      .unwrap_or_default();
    assert!(
      request.contains("DELETE /github/pr/42/requested-reviewers?org=acme&repo=widget HTTP/1.1")
    );
    assert!(request.contains("\"users\":[\"bob\"]"));
  }

  #[test]
  fn mark_pull_request_ready_for_review_uses_expected_route_and_payload() {
    let (base_url, request, handle) =
      start_single_response_server_with_request("204 NO CONTENT", "");
    let api = make_test_api_client(base_url);

    api
      .mark_pull_request_ready_for_review("acme", "widget", 42, "PR_kwDOExample")
      .expect("mark pull request ready for review");

    handle.join().expect("join server thread");
    let request = request
      .lock()
      .expect("lock request")
      .clone()
      .unwrap_or_default();
    assert!(request.contains("POST /github/pr/42/ready-for-review?org=acme&repo=widget HTTP/1.1"));
    assert!(request.contains("\"pullRequestId\":\"PR_kwDOExample\""));
  }

  #[test]
  fn convert_pull_request_to_draft_uses_expected_route_and_payload() {
    let (base_url, request, handle) =
      start_single_response_server_with_request("204 NO CONTENT", "");
    let api = make_test_api_client(base_url);

    api
      .convert_pull_request_to_draft("acme", "widget", 42, "PR_kwDOExample")
      .expect("convert pull request to draft");

    handle.join().expect("join server thread");
    let request = request
      .lock()
      .expect("lock request")
      .clone()
      .unwrap_or_default();
    assert!(request.contains("POST /github/pr/42/convert-to-draft?org=acme&repo=widget HTTP/1.1"));
    assert!(request.contains("\"pullRequestId\":\"PR_kwDOExample\""));
  }

  #[test]
  fn fetch_pull_request_merge_readiness_parses_success_payload() {
    let body = r#"{
      "mergeReadiness": {
        "status": "ready",
        "message": "This pull request is ready to merge.",
        "current_head_sha": "head123",
        "available_methods": ["merge", "squash", "rebase"],
        "default_method": "merge",
        "can_merge_now": true,
        "viewer_can_merge": true,
        "mergeable_state": "clean",
        "rebaseable": true,
        "auto_merge_enabled": false
      }
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let readiness = api
      .fetch_pull_request_merge_readiness("acme", "widget", 42)
      .expect("fetch pull request merge readiness");

    assert_eq!(
      readiness.status,
      GithubPullRequestMergeReadinessStatus::Ready
    );
    assert_eq!(readiness.current_head_sha, "head123");
    assert_eq!(
      readiness.available_methods,
      vec![
        GithubPullRequestMergeMethod::Merge,
        GithubPullRequestMergeMethod::Squash,
        GithubPullRequestMergeMethod::Rebase
      ]
    );
    assert_eq!(
      readiness.default_method,
      Some(GithubPullRequestMergeMethod::Merge)
    );
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_pull_request_merge_readiness_uses_expected_route() {
    let body = r#"{
      "mergeReadiness": {
        "status": "checking",
        "message": "Pending",
        "current_head_sha": "head123",
        "available_methods": [],
        "default_method": null,
        "can_merge_now": false,
        "viewer_can_merge": false,
        "mergeable_state": null,
        "rebaseable": null,
        "auto_merge_enabled": false
      }
    }"#;
    let (base_url, request_line, handle) =
      start_single_response_server_with_request_line("200 OK", body);
    let api = make_test_api_client(base_url);

    let _ = api
      .fetch_pull_request_merge_readiness("acme", "widget", 42)
      .expect("fetch pull request merge readiness");

    handle.join().expect("join server thread");
    let request_line = request_line
      .lock()
      .expect("lock request line")
      .clone()
      .unwrap_or_default();
    assert_eq!(
      request_line,
      "GET /github/pr/42/merge-readiness?org=acme&repo=widget HTTP/1.1"
    );
  }

  #[test]
  fn fetch_pull_request_checks_parses_success_payload() {
    let body = r#"{
      "checks": {
        "head_sha": "head123",
        "overall_state": "failure",
        "required_state": "pending",
        "total_checks": 4,
        "successful_checks": 2,
        "failed_checks": 1,
        "pending_checks": 1,
        "required_checks_total": 3,
        "required_checks_passed": 1,
        "required_checks_failed": 1,
        "required_checks_pending": 1,
        "required_contexts": ["build", "lint", "deploy"],
        "missing_required_contexts": ["deploy"],
        "requires_up_to_date_branch": true,
        "actions_runs": [
          {
            "id": 100,
            "name": "CI",
            "display_title": "CI",
            "event": "pull_request",
            "status": "completed",
            "conclusion": "success",
            "state": "success",
            "created_at": "2026-03-19T10:00:00Z",
            "updated_at": "2026-03-19T10:05:00Z",
            "run_started_at": "2026-03-19T10:00:30Z",
            "run_number": 12,
            "run_attempt": 1,
            "html_url": "https://github.com/acme/widget/actions/runs/100",
            "jobs": [
              {
                "id": 200,
                "name": "build",
                "status": "completed",
                "conclusion": "success",
                "state": "success",
                "started_at": "2026-03-19T10:00:30Z",
                "completed_at": "2026-03-19T10:02:00Z",
                "html_url": "https://github.com/acme/widget/actions/runs/100/job/200",
                "required": true,
                "app_name": "GitHub Actions",
                "app_slug": "github-actions",
                "app_avatar_url": "https://avatars.githubusercontent.com/in/15368?v=4",
                "steps": [
                  {
                    "number": 1,
                    "name": "Install",
                    "status": "completed",
                    "conclusion": "success",
                    "state": "success",
                    "started_at": "2026-03-19T10:00:31Z",
                    "completed_at": "2026-03-19T10:00:50Z"
                  }
                ]
              }
            ]
          }
        ],
        "other_checks": [
          {
            "id": 301,
            "name": "lint",
            "status": "completed",
            "conclusion": "failure",
            "state": "failure",
            "started_at": "2026-03-19T10:01:00Z",
            "completed_at": "2026-03-19T10:03:00Z",
            "html_url": "https://github.com/acme/widget/runs/301",
            "details_url": "https://github.com/acme/widget/runs/301",
            "required": true,
            "app_name": "Reviewdog",
            "app_slug": "reviewdog",
            "app_avatar_url": "https://avatars.githubusercontent.com/u/15138054?v=4",
            "title": "Lint",
            "summary": "Lint failed",
            "text": "unused variable",
            "annotations_count": 2
          }
        ],
        "legacy_statuses": [
          {
            "id": 401,
            "context": "security/brakeman",
            "status": "success",
            "state": "success",
            "description": "Security checks passed",
            "target_url": "https://ci.example.com/401",
            "avatar_url": "https://ci.example.com/avatar.png",
            "created_at": "2026-03-19T10:00:00Z",
            "updated_at": "2026-03-19T10:04:00Z",
            "required": false
          }
        ]
      }
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let checks = api
      .fetch_pull_request_checks("acme", "widget", 42)
      .expect("fetch pull request checks");

    assert_eq!(checks.head_sha, "head123");
    assert_eq!(
      checks.overall_state,
      GithubPullRequestChecksRollupState::Failure
    );
    assert_eq!(
      checks.required_state,
      GithubPullRequestChecksRollupState::Pending
    );
    assert_eq!(checks.missing_required_contexts, vec!["deploy"]);
    assert_eq!(checks.actions_runs.len(), 1);
    assert_eq!(checks.actions_runs[0].jobs.len(), 1);
    assert_eq!(checks.actions_runs[0].jobs[0].steps.len(), 1);
    assert_eq!(
      checks.actions_runs[0].jobs[0].app_avatar_url.as_deref(),
      Some("https://avatars.githubusercontent.com/in/15368?v=4")
    );
    assert_eq!(checks.other_checks.len(), 1);
    assert_eq!(
      checks.other_checks[0].app_avatar_url.as_deref(),
      Some("https://avatars.githubusercontent.com/u/15138054?v=4")
    );
    assert_eq!(checks.legacy_statuses.len(), 1);
    assert_eq!(
      checks.legacy_statuses[0].avatar_url.as_deref(),
      Some("https://ci.example.com/avatar.png")
    );
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_pull_request_checks_uses_expected_route() {
    let body = r#"{
      "checks": {
        "head_sha": "head123",
        "overall_state": "success",
        "required_state": "success",
        "total_checks": 0,
        "successful_checks": 0,
        "failed_checks": 0,
        "pending_checks": 0,
        "required_checks_total": 0,
        "required_checks_passed": 0,
        "required_checks_failed": 0,
        "required_checks_pending": 0,
        "required_contexts": [],
        "missing_required_contexts": [],
        "requires_up_to_date_branch": false,
        "actions_runs": [],
        "other_checks": [],
        "legacy_statuses": []
      }
    }"#;
    let (base_url, request_line, handle) =
      start_single_response_server_with_request_line("200 OK", body);
    let api = make_test_api_client(base_url);

    let _ = api
      .fetch_pull_request_checks("acme", "widget", 42)
      .expect("fetch pull request checks");

    handle.join().expect("join server thread");
    let request_line = request_line
      .lock()
      .expect("lock request line")
      .clone()
      .unwrap_or_default();
    assert_eq!(
      request_line,
      "GET /github/pr/42/checks?org=acme&repo=widget HTTP/1.1"
    );
  }

  #[test]
  fn update_pull_request_description_parses_success_payload() {
    let body = r#"{
      "pullRequest": {
        "number": 42,
        "body": "Updated PR description",
        "updated_at": "2026-02-15T13:00:00Z"
      }
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let pull_request = api
      .update_pull_request_description("acme", "widget", 42, "Updated PR description")
      .expect("update pull request description");
    assert_eq!(pull_request.number, 42);
    assert_eq!(pull_request.body.as_deref(), Some("Updated PR description"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn update_pull_request_description_uses_expected_route() {
    let body = r#"{
      "pullRequest": {
        "number": 42,
        "body": "",
        "updated_at": "2026-02-15T13:00:00Z"
      }
    }"#;
    let (base_url, request_line, handle) =
      start_single_response_server_with_request_line("200 OK", body);
    let api = make_test_api_client(base_url);

    let _ = api
      .update_pull_request_description("acme", "widget", 42, "")
      .expect("update pull request description");

    handle.join().expect("join server thread");
    let request_line = request_line
      .lock()
      .expect("lock request line")
      .clone()
      .unwrap_or_default();
    assert_eq!(
      request_line,
      "PATCH /github/pr/42?org=acme&repo=widget HTTP/1.1"
    );
  }

  #[test]
  fn update_pull_request_base_uses_expected_route_and_body() {
    let (base_url, request, handle) =
      start_single_response_server_with_request("204 No Content", "");
    let api = make_test_api_client(base_url);

    api
      .update_pull_request_base("acme", "widget", 42, "release/next")
      .expect("update pull request base");

    handle.join().expect("join server thread");
    let request = request
      .lock()
      .expect("lock request")
      .clone()
      .unwrap_or_default();
    assert!(request.starts_with("PATCH /github/pr/42/base?org=acme&repo=widget HTTP/1.1"));
    assert!(request.contains(r#""base":"release/next""#));
  }

  #[test]
  fn fetch_pull_request_files_parses_success_payload() {
    let body = r#"{
      "files": [
        {
          "filename": "src/main.rs",
          "status": "renamed",
          "patch": "@@ -1 +1 @@",
          "previous_filename": "src/old_main.rs"
        }
      ]
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let files = api
      .fetch_pull_request_files("acme", "widget", 42, None)
      .expect("fetch pull request files");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].filename, "src/main.rs");
    assert_eq!(files[0].status, "renamed");
    assert_eq!(
      files[0].previous_filename.as_deref(),
      Some("src/old_main.rs")
    );
    handle.join().expect("join server thread");
  }

  #[test]
  fn merge_pull_request_parses_success_payload() {
    let body = r#"{
      "mergeResult": {
        "merged": true,
        "sha": "merged123",
        "message": "Pull Request successfully merged",
        "method": "squash"
      }
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let result = api
      .merge_pull_request(
        "acme",
        "widget",
        42,
        GithubPullRequestMergeMethod::Squash,
        "head123",
        Some("Squash title"),
        Some("Squash message"),
      )
      .expect("merge pull request");

    assert!(result.merged);
    assert_eq!(result.sha, "merged123");
    assert_eq!(result.method, GithubPullRequestMergeMethod::Squash);
    handle.join().expect("join server thread");
  }

  #[test]
  fn merge_pull_request_serializes_method_head_sha_and_optional_message_fields() {
    let body = r#"{
      "mergeResult": {
        "merged": true,
        "sha": "merged123",
        "message": "Pull Request successfully merged",
        "method": "merge"
      }
    }"#;
    let (base_url, request, handle) = start_single_response_server_with_request("200 OK", body);
    let api = make_test_api_client(base_url);

    let _ = api
      .merge_pull_request(
        "acme",
        "widget",
        42,
        GithubPullRequestMergeMethod::Merge,
        "head123",
        Some("  Merge title  "),
        Some("  Merge message  "),
      )
      .expect("merge pull request");

    handle.join().expect("join server thread");
    let request = request
      .lock()
      .expect("lock request")
      .clone()
      .unwrap_or_default();
    assert!(request.contains("\"method\":\"merge\""));
    assert!(request.contains("\"expectedHeadSha\":\"head123\""));
    assert!(request.contains("\"commitTitle\":\"Merge title\""));
    assert!(request.contains("\"commitMessage\":\"Merge message\""));
  }

  #[test]
  fn merge_pull_request_omits_empty_title_and_message_fields() {
    let body = r#"{
      "mergeResult": {
        "merged": true,
        "sha": "merged123",
        "message": "Pull Request successfully merged",
        "method": "rebase"
      }
    }"#;
    let (base_url, request, handle) = start_single_response_server_with_request("200 OK", body);
    let api = make_test_api_client(base_url);

    let _ = api
      .merge_pull_request(
        "acme",
        "widget",
        42,
        GithubPullRequestMergeMethod::Rebase,
        "head123",
        Some("   "),
        Some(""),
      )
      .expect("merge pull request");

    handle.join().expect("join server thread");
    let request = request
      .lock()
      .expect("lock request")
      .clone()
      .unwrap_or_default();
    assert!(request.contains("\"method\":\"rebase\""));
    assert!(!request.contains("\"commitTitle\""));
    assert!(!request.contains("\"commitMessage\""));
  }

  #[test]
  fn merge_pull_request_surfaces_backend_error_message() {
    let body = r#"{"error":"Base branch moved; refresh and try again."}"#;
    let (base_url, handle) = start_single_response_server("409 CONFLICT", body);
    let api = make_test_api_client(base_url);

    let err = api
      .merge_pull_request(
        "acme",
        "widget",
        42,
        GithubPullRequestMergeMethod::Merge,
        "head123",
        None,
        None,
      )
      .expect_err("merge pull request should fail");

    assert_eq!(err.to_string(), "Base branch moved; refresh and try again.");
    let api_error = err.downcast_ref::<ApiError>().expect("api error");
    assert_eq!(api_error.status_code_u16(), Some(409));
    handle.join().expect("join server thread");
  }

  #[test]
  fn add_pull_request_assignee_surfaces_backend_error_message() {
    let body = r#"{"error":"Reviewer must have write access to this repository."}"#;
    let (base_url, handle) = start_single_response_server("422 UNPROCESSABLE ENTITY", body);
    let api = make_test_api_client(base_url);

    let err = api
      .add_pull_request_assignee("acme", "widget", 42, "alice")
      .expect_err("add pull request assignee should fail");

    assert_eq!(
      err.to_string(),
      "Reviewer must have write access to this repository."
    );
    let api_error = err.downcast_ref::<ApiError>().expect("api error");
    assert_eq!(api_error.status_code_u16(), Some(422));
    handle.join().expect("join server thread");
  }

  #[test]
  fn request_pull_request_reviewer_surfaces_backend_error_message() {
    let body = r#"{"error":"Requested reviewer is not a collaborator."}"#;
    let (base_url, handle) = start_single_response_server("422 UNPROCESSABLE ENTITY", body);
    let api = make_test_api_client(base_url);

    let err = api
      .request_pull_request_reviewer("acme", "widget", 42, "bob")
      .expect_err("request pull request reviewer should fail");

    assert_eq!(err.to_string(), "Requested reviewer is not a collaborator.");
    let api_error = err.downcast_ref::<ApiError>().expect("api error");
    assert_eq!(api_error.status_code_u16(), Some(422));
    handle.join().expect("join server thread");
  }

  #[test]
  fn mark_pull_request_ready_for_review_surfaces_backend_error_message() {
    let body = r#"{"error":"Only pull request authors can mark this draft ready for review."}"#;
    let (base_url, handle) = start_single_response_server("403 FORBIDDEN", body);
    let api = make_test_api_client(base_url);

    let err = api
      .mark_pull_request_ready_for_review("acme", "widget", 42, "PR_kwDOExample")
      .expect_err("ready for review should fail");

    assert_eq!(
      err.to_string(),
      "Only pull request authors can mark this draft ready for review."
    );
    let api_error = err.downcast_ref::<ApiError>().expect("api error");
    assert_eq!(api_error.status_code_u16(), Some(403));
    handle.join().expect("join server thread");
  }

  #[test]
  fn convert_pull_request_to_draft_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 UNAUTHORIZED", "{}");
    let api = make_test_api_client(base_url);

    let err = api
      .convert_pull_request_to_draft("acme", "widget", 42, "PR_kwDOExample")
      .expect_err("convert pull request to draft should fail");

    assert_eq!(err.to_string(), "unauthorized");
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_pull_request_files_includes_commit_sha_query_when_provided() {
    let body = r#"{"files":[]}"#;
    let (base_url, request_line, handle) =
      start_single_response_server_with_request_line("200 OK", body);
    let api = make_test_api_client(base_url);

    let _ = api
      .fetch_pull_request_files("acme", "widget", 42, Some("deadbeef"))
      .expect("fetch pull request files");

    handle.join().expect("join server thread");
    let request_line = request_line
      .lock()
      .expect("lock request line")
      .clone()
      .unwrap_or_default();
    assert!(
      request_line.contains("/github/pr/42/files?org=acme&repo=widget&commitSha=deadbeef"),
      "unexpected request line: {request_line}"
    );
  }

  #[test]
  fn fetch_pull_request_commits_parses_success_payload() {
    let body = r#"{
      "commits": [
        {
          "sha": "1111111111111111111111111111111111111111",
          "message": "feat: add commit filter",
          "authored_at": "2026-02-27T10:00:00Z",
          "committed_at": "2026-02-27T10:05:00Z",
          "parent_sha": "0000000000000000000000000000000000000000",
          "author": { "login": "octocat", "avatar_url": null },
          "committer": { "login": "octocat", "avatar_url": null },
          "authors": [
            {
              "name": "Octo Cat",
              "email": "octocat@example.com",
              "login": "octocat",
              "avatar_url": "https://example.com/octocat.png"
            },
            {
              "name": "Co Author",
              "email": "coauthor@example.com",
              "login": null,
              "avatar_url": null
            }
          ]
        }
      ]
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let commits = api
      .fetch_pull_request_commits("acme", "widget", 42)
      .expect("fetch pull request commits");
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].sha, "1111111111111111111111111111111111111111");
    assert_eq!(commits[0].message, "feat: add commit filter");
    assert_eq!(
      commits[0].author.as_ref().map(|user| user.login.as_str()),
      Some("octocat")
    );
    assert_eq!(
      commits[0].parent_sha.as_deref(),
      Some("0000000000000000000000000000000000000000")
    );
    assert_eq!(commits[0].authors.len(), 2);
    assert_eq!(commits[0].authors[0].login.as_deref(), Some("octocat"));
    assert_eq!(
      commits[0].authors[1].email.as_deref(),
      Some("coauthor@example.com")
    );
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_pull_request_commits_calls_expected_route_with_query_params() {
    let body = r#"{"commits":[]}"#;
    let (base_url, request_line, handle) =
      start_single_response_server_with_request_line("200 OK", body);
    let api = make_test_api_client(base_url);

    let _ = api
      .fetch_pull_request_commits("acme", "widget", 42)
      .expect("fetch pull request commits");

    handle.join().expect("join server thread");
    let request_line = request_line
      .lock()
      .expect("lock request line")
      .clone()
      .unwrap_or_default();
    assert_eq!(
      request_line,
      "GET /github/pr/42/commits?org=acme&repo=widget HTTP/1.1"
    );
  }

  #[test]
  fn fetch_pull_request_conversation_parses_success_payload() {
    let body = r#"{
      "conversation": {
        "pull_request": {
          "node_id": "PR_kwDOExample",
          "reactions": [
            {
              "content": "THUMBS_UP",
              "count": 2,
              "viewer_has_reacted": true
            }
          ]
        },
        "issue_comments": [
          {
            "node_id": "IC_kwDOExample",
            "id": 11,
            "body": "Can you add tests?",
            "created_at": "2026-02-28T10:00:00Z",
            "updated_at": "2026-02-28T10:05:00Z",
            "user": { "login": "octocat", "avatar_url": null }
          }
        ],
        "reviews": [
          {
            "node_id": "PRR_kwDOExample",
            "id": 123,
            "body": "Looks good",
            "state": "APPROVED",
            "submitted_at": "2026-02-28T12:00:00Z",
            "commit_id": "1111111111111111111111111111111111111111",
            "html_url": "https://github.com/acme/widget/pull/42#pullrequestreview-123",
            "user": { "login": "reviewer", "avatar_url": null }
          }
        ],
        "review_comments": [
          {
            "node_id": "PRRC_kwDOExample",
            "id": 1,
            "pull_request_review_id": 123,
            "diff_hunk": "@@ -1 +1 @@",
            "path": "src/main.rs",
            "position": 1,
            "original_position": 1,
            "commit_id": "head123",
            "original_commit_id": "base123",
            "in_reply_to_id": null,
            "user": { "login": "octocat", "avatar_url": null },
            "body": "Looks good",
            "created_at": "2026-02-15T12:00:00Z",
            "updated_at": "2026-02-15T12:01:00Z",
            "start_line": null,
            "original_start_line": null,
            "start_side": null,
            "line": 1,
            "original_line": 1,
            "side": "RIGHT"
          }
        ]
      }
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let conversation = api
      .fetch_pull_request_conversation("acme", "widget", 42)
      .expect("fetch pull request conversation");
    assert_eq!(conversation.pull_request.node_id, "PR_kwDOExample");
    assert_eq!(
      conversation.pull_request.reactions[0].content,
      GithubReactionContent::ThumbsUp
    );
    assert_eq!(conversation.issue_comments[0].node_id, "IC_kwDOExample");
    assert_eq!(
      conversation.reviews[0].state,
      GithubPullRequestReviewState::Approved
    );
    assert_eq!(conversation.review_comments[0].path, "src/main.rs");
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_pull_request_conversation_uses_expected_route_with_query_params() {
    let body = r#"{"conversation":{"pull_request":{"node_id":"PR_kwDOExample","reactions":[]},"issue_comments":[],"reviews":[],"review_comments":[]}}"#;
    let (base_url, request_line, handle) =
      start_single_response_server_with_request_line("200 OK", body);
    let api = make_test_api_client(base_url);

    let _ = api
      .fetch_pull_request_conversation("acme", "widget", 42)
      .expect("fetch pull request conversation");

    handle.join().expect("join server thread");
    let request_line = request_line
      .lock()
      .expect("lock request line")
      .clone()
      .unwrap_or_default();
    assert_eq!(
      request_line,
      "GET /github/pr/42/conversation?org=acme&repo=widget HTTP/1.1"
    );
  }

  #[test]
  fn add_pull_request_reaction_serializes_subject_and_content() {
    let body = r#"{
      "reactions": [
        { "content": "THUMBS_UP", "count": 3, "viewer_has_reacted": true }
      ]
    }"#;
    let (base_url, request, handle) = start_single_response_server_with_request("200 OK", body);
    let api = make_test_api_client(base_url);

    let reactions = api
      .add_pull_request_reaction(
        "acme",
        "widget",
        42,
        "IC_kwDOExample",
        GithubReactionContent::ThumbsUp,
      )
      .expect("add pull request reaction");

    assert_eq!(reactions.len(), 1);
    assert_eq!(reactions[0].content, GithubReactionContent::ThumbsUp);
    assert!(reactions[0].viewer_has_reacted);
    handle.join().expect("join server thread");
    let request = request
      .lock()
      .expect("lock request")
      .clone()
      .unwrap_or_default();
    assert!(request.starts_with("POST /github/pr/42/reactions?org=acme&repo=widget HTTP/1.1"));
    assert!(request.contains("\"subjectId\":\"IC_kwDOExample\""));
    assert!(request.contains("\"content\":\"THUMBS_UP\""));
  }

  #[test]
  fn remove_pull_request_reaction_uses_delete_reactions_route() {
    let body = r#"{"reactions":[]}"#;
    let (base_url, request_line, handle) =
      start_single_response_server_with_request_line("200 OK", body);
    let api = make_test_api_client(base_url);

    let reactions = api
      .remove_pull_request_reaction(
        "acme",
        "widget",
        42,
        "IC_kwDOExample",
        GithubReactionContent::Heart,
      )
      .expect("remove pull request reaction");

    assert!(reactions.is_empty());
    handle.join().expect("join server thread");
    let request_line = request_line
      .lock()
      .expect("lock request line")
      .clone()
      .unwrap_or_default();
    assert_eq!(
      request_line,
      "DELETE /github/pr/42/reactions?org=acme&repo=widget HTTP/1.1"
    );
  }

  #[test]
  fn add_pull_request_reaction_surfaces_backend_error_message() {
    let body = r#"{"error":"The openai organization restricts OAuth app access. Ask an organization owner to approve Reviu, then try again."}"#;
    let (base_url, handle) = start_single_response_server("403 FORBIDDEN", body);
    let api = make_test_api_client(base_url);

    let err = api
      .add_pull_request_reaction(
        "openai",
        "reviu",
        42,
        "PR_kwDOExample",
        GithubReactionContent::ThumbsUp,
      )
      .expect_err("add pull request reaction should fail");

    assert_eq!(
      err.to_string(),
      "The openai organization restricts OAuth app access. Ask an organization owner to approve Reviu, then try again."
    );
    let api_error = err.downcast_ref::<ApiError>().expect("api error");
    assert_eq!(api_error.status_code_u16(), Some(403));
    handle.join().expect("join server thread");
  }

  #[test]
  fn add_issue_reaction_serializes_subject_and_content() {
    let body = r#"{
      "reactions": [
        { "content": "THUMBS_UP", "count": 3, "viewer_has_reacted": true }
      ]
    }"#;
    let (base_url, request, handle) = start_single_response_server_with_request("200 OK", body);
    let api = make_test_api_client(base_url);

    let reactions = api
      .add_issue_reaction(
        "acme",
        "widget",
        77,
        "IC_kwDOIssue",
        GithubReactionContent::ThumbsUp,
      )
      .expect("add issue reaction");

    assert_eq!(reactions.len(), 1);
    assert_eq!(reactions[0].content, GithubReactionContent::ThumbsUp);
    assert!(reactions[0].viewer_has_reacted);
    handle.join().expect("join server thread");
    let request = request
      .lock()
      .expect("lock request")
      .clone()
      .unwrap_or_default();
    assert!(request.starts_with("POST /github/repos/acme/widget/issues/77/reactions HTTP/1.1"));
    assert!(request.contains("\"subjectId\":\"IC_kwDOIssue\""));
    assert!(request.contains("\"content\":\"THUMBS_UP\""));
  }

  #[test]
  fn add_issue_reaction_surfaces_backend_error_message() {
    let body = r#"{"error":"The openai organization restricts OAuth app access. Ask an organization owner to approve Reviu, then try again."}"#;
    let (base_url, handle) = start_single_response_server("403 FORBIDDEN", body);
    let api = make_test_api_client(base_url);

    let err = api
      .add_issue_reaction(
        "openai",
        "reviu",
        77,
        "I_kwDOExample",
        GithubReactionContent::Heart,
      )
      .expect_err("add issue reaction should fail");

    assert_eq!(
      err.to_string(),
      "The openai organization restricts OAuth app access. Ask an organization owner to approve Reviu, then try again."
    );
    let api_error = err.downcast_ref::<ApiError>().expect("api error");
    assert_eq!(api_error.status_code_u16(), Some(403));
    handle.join().expect("join server thread");
  }

  #[test]
  fn remove_issue_reaction_uses_delete_reactions_route() {
    let body = r#"{"reactions":[]}"#;
    let (base_url, request_line, handle) =
      start_single_response_server_with_request_line("200 OK", body);
    let api = make_test_api_client(base_url);

    let reactions = api
      .remove_issue_reaction(
        "acme",
        "widget",
        77,
        "I_kwDOExample",
        GithubReactionContent::Heart,
      )
      .expect("remove issue reaction");

    assert!(reactions.is_empty());
    handle.join().expect("join server thread");
    let request_line = request_line
      .lock()
      .expect("lock request line")
      .clone()
      .unwrap_or_default();
    assert_eq!(
      request_line,
      "DELETE /github/repos/acme/widget/issues/77/reactions HTTP/1.1"
    );
  }

  #[test]
  fn update_pull_request_review_comment_parses_success_payload() {
    let body = r#"{
      "comment": {
        "id": 1,
        "pullRequestReviewId": 12,
        "diff_hunk": "@@ -1 +1 @@",
        "path": "src/main.rs",
        "position": 1,
        "original_position": 1,
        "commit_id": "head123",
        "original_commit_id": "base123",
        "in_reply_to_id": null,
        "user": { "login": "octocat", "avatar_url": null },
        "body": "Updated body",
        "created_at": "2026-02-15T12:00:00Z",
        "updated_at": "2026-02-16T12:01:00Z",
        "start_line": null,
        "original_start_line": null,
        "start_side": null,
        "line": 1,
        "original_line": 1,
        "side": "RIGHT"
      }
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let comment = api
      .update_pull_request_review_comment("acme", "widget", 42, 1, "Updated body")
      .expect("update review comment");
    assert_eq!(comment.id, 1);
    assert_eq!(comment.body, "Updated body");
    assert_eq!(comment.path, "src/main.rs");
    handle.join().expect("join server thread");
  }

  #[test]
  fn create_pull_request_review_comment_parses_success_payload() {
    let body = r#"{
      "comment": {
        "id": 2,
        "pull_request_review_id": 12,
        "diff_hunk": "@@ -1 +1 @@",
        "path": "src/main.rs",
        "position": 1,
        "original_position": 1,
        "commit_id": "head123",
        "original_commit_id": "base123",
        "in_reply_to_id": null,
        "user": { "login": "octocat", "avatar_url": null },
        "body": "New comment body",
        "created_at": "2026-02-15T12:00:00Z",
        "updated_at": "2026-02-16T12:01:00Z",
        "start_line": null,
        "original_start_line": null,
        "start_side": null,
        "line": 1,
        "original_line": 1,
        "side": "RIGHT"
      }
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let comment = api
      .create_pull_request_review_comment(
        "acme",
        "widget",
        42,
        "src/main.rs",
        "head123",
        1,
        "RIGHT",
        None,
        None,
        "New comment body",
      )
      .expect("create review comment");
    assert_eq!(comment.id, 2);
    assert_eq!(comment.body, "New comment body");
    assert_eq!(comment.path, "src/main.rs");
    handle.join().expect("join server thread");
  }

  #[test]
  fn create_pull_request_review_comment_reply_parses_success_payload() {
    let body = r#"{
      "comment": {
        "id": 3,
        "pull_request_review_id": 12,
        "diff_hunk": "@@ -1 +1 @@",
        "path": "src/main.rs",
        "position": 1,
        "original_position": 1,
        "commit_id": "head123",
        "original_commit_id": "base123",
        "in_reply_to_id": 2,
        "user": { "login": "octocat", "avatar_url": null },
        "body": "Reply body",
        "created_at": "2026-02-15T12:00:00Z",
        "updated_at": "2026-02-16T12:01:00Z",
        "start_line": null,
        "original_start_line": null,
        "start_side": null,
        "line": 1,
        "original_line": 1,
        "side": "RIGHT"
      }
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let comment = api
      .reply_pull_request_review_comment("acme", "widget", 42, 2, "Reply body")
      .expect("create review comment reply");
    assert_eq!(comment.id, 3);
    assert_eq!(comment.body, "Reply body");
    assert_eq!(comment.in_reply_to_id, Some(2));
    handle.join().expect("join server thread");
  }

  #[test]
  fn reply_pull_request_review_comment_uses_replies_route() {
    let body = r#"{
      "comment": {
        "id": 3,
        "pull_request_review_id": 12,
        "diff_hunk": "@@ -1 +1 @@",
        "path": "src/main.rs",
        "position": 1,
        "original_position": 1,
        "commit_id": "head123",
        "original_commit_id": "base123",
        "in_reply_to_id": 2,
        "user": { "login": "octocat", "avatar_url": null },
        "body": "Reply body",
        "created_at": "2026-02-15T12:00:00Z",
        "updated_at": "2026-02-16T12:01:00Z",
        "start_line": null,
        "original_start_line": null,
        "start_side": null,
        "line": 1,
        "original_line": 1,
        "side": "RIGHT"
      }
    }"#;
    let (base_url, request_line, handle) =
      start_single_response_server_with_request_line("200 OK", body);
    let api = make_test_api_client(base_url);

    let _ = api
      .reply_pull_request_review_comment("acme", "widget", 42, 2, "Reply body")
      .expect("create review comment reply");

    handle.join().expect("join server thread");
    let request_line = request_line
      .lock()
      .expect("lock request line")
      .clone()
      .unwrap_or_default();
    assert!(
      request_line.contains("/github/pr/42/comments/2/replies"),
      "unexpected request line: {request_line}"
    );
  }

  #[test]
  fn apply_pull_request_suggested_change_serializes_commit_payload() {
    let body = r#"{
      "commit": {
        "sha": "abc123",
        "url": "https://github.com/acme/widget/commit/abc123"
      }
    }"#;
    let (base_url, request, handle) = start_single_response_server_with_request("200 OK", body);
    let api = make_test_api_client(base_url);

    let result = api
      .apply_pull_request_suggested_change(
        "acme",
        "widget",
        42,
        2,
        "Apply suggestion from code review",
        Some("Thanks"),
        "head123",
        "src/main.rs",
        10,
        &["old".to_string()],
        &["new".to_string()],
        true,
        Some("octocat"),
      )
      .expect("apply suggested change");

    assert_eq!(result.sha, "abc123");
    handle.join().expect("join server thread");
    let request = request
      .lock()
      .expect("lock request")
      .clone()
      .unwrap_or_default();
    assert!(
      request.starts_with(
        "POST /github/pr/42/comments/2/suggested-change?org=acme&repo=widget HTTP/1.1"
      )
    );
    assert!(request.contains("\"commitTitle\":\"Apply suggestion from code review\""));
    assert!(request.contains("\"expectedHeadSha\":\"head123\""));
    assert!(request.contains("\"originalStartLine\":10"));
    assert!(request.contains("\"includeCoAuthor\":true"));
    assert!(request.contains("\"suggestionAuthorLogin\":\"octocat\""));
  }

  #[test]
  fn delete_pull_request_review_comment_returns_ok_on_success() {
    let (base_url, handle) = start_single_response_server("200 OK", "{}");
    let api = make_test_api_client(base_url);

    api
      .delete_pull_request_review_comment("acme", "widget", 42, 2)
      .expect("delete review comment");

    handle.join().expect("join server thread");
  }

  #[test]
  fn submit_pull_request_review_parses_success_payload() {
    let body = r#"{
      "review": {
        "id": 123,
        "body": "Ship it",
        "state": "APPROVED",
        "submitted_at": "2026-02-28T12:00:00Z",
        "commit_id": "1111111111111111111111111111111111111111",
        "html_url": "https://github.com/acme/widget/pull/42#pullrequestreview-123",
        "user": { "login": "octocat", "avatar_url": null }
      }
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let review = api
      .submit_pull_request_review(
        "acme",
        "widget",
        42,
        GithubPullRequestReviewEvent::Approve,
        "",
      )
      .expect("submit pull request review");

    assert_eq!(review.id, 123);
    assert_eq!(review.state, GithubPullRequestReviewState::Approved);
    assert_eq!(review.body.as_deref(), Some("Ship it"));
    assert_eq!(
      review.user.as_ref().map(|user| user.login.as_str()),
      Some("octocat")
    );
    handle.join().expect("join server thread");
  }

  #[test]
  fn submit_pull_request_review_parses_commented_review_payload() {
    let body = r#"{
      "review": {
        "id": 124,
        "body": "Looks good to me",
        "state": "COMMENTED",
        "submitted_at": "2026-02-28T12:05:00Z",
        "commit_id": "1111111111111111111111111111111111111111",
        "html_url": "https://github.com/acme/widget/pull/42#pullrequestreview-124",
        "user": { "login": "octocat", "avatar_url": null }
      }
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let review = api
      .submit_pull_request_review(
        "acme",
        "widget",
        42,
        GithubPullRequestReviewEvent::Comment,
        "Looks good to me",
      )
      .expect("submit pull request review");

    assert_eq!(review.id, 124);
    assert_eq!(review.state, GithubPullRequestReviewState::Commented);
    assert_eq!(review.body.as_deref(), Some("Looks good to me"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn submit_pull_request_review_uses_reviews_route_with_query_params() {
    let body = r#"{
      "review": {
        "id": 1,
        "body": "Looks good",
        "state": "APPROVED",
        "submitted_at": "2026-02-28T12:00:00Z",
        "commit_id": "1111111111111111111111111111111111111111",
        "html_url": "https://github.com/acme/widget/pull/42#pullrequestreview-1",
        "user": { "login": "octocat", "avatar_url": null }
      }
    }"#;
    let (base_url, request_line, handle) =
      start_single_response_server_with_request_line("200 OK", body);
    let api = make_test_api_client(base_url);

    let _ = api
      .submit_pull_request_review(
        "acme",
        "widget",
        42,
        GithubPullRequestReviewEvent::Comment,
        "Looks good",
      )
      .expect("submit pull request review");

    handle.join().expect("join server thread");
    let request_line = request_line
      .lock()
      .expect("lock request line")
      .clone()
      .unwrap_or_default();
    assert_eq!(
      request_line,
      "POST /github/pr/42/reviews?org=acme&repo=widget HTTP/1.1"
    );
  }

  #[test]
  fn submit_pull_request_review_serializes_event_and_body() {
    let body = r#"{
      "review": {
        "id": 2,
        "body": "Needs tests",
        "state": "CHANGES_REQUESTED",
        "submitted_at": "2026-02-28T12:00:00Z",
        "commit_id": "1111111111111111111111111111111111111111",
        "html_url": "https://github.com/acme/widget/pull/42#pullrequestreview-2",
        "user": { "login": "octocat", "avatar_url": null }
      }
    }"#;
    let (base_url, request, handle) = start_single_response_server_with_request("200 OK", body);
    let api = make_test_api_client(base_url);

    let _ = api
      .submit_pull_request_review(
        "acme",
        "widget",
        42,
        GithubPullRequestReviewEvent::RequestChanges,
        "  Needs tests  ",
      )
      .expect("submit pull request review");

    handle.join().expect("join server thread");
    let request = request
      .lock()
      .expect("lock request")
      .clone()
      .unwrap_or_default();
    assert!(request.contains("\"event\":\"REQUEST_CHANGES\""));
    assert!(request.contains("\"body\":\"Needs tests\""));
  }

  #[test]
  fn submit_pull_request_review_omits_empty_body_for_approve() {
    let body = r#"{
      "review": {
        "id": 3,
        "body": null,
        "state": "APPROVED",
        "submitted_at": "2026-02-28T12:00:00Z",
        "commit_id": "1111111111111111111111111111111111111111",
        "html_url": "https://github.com/acme/widget/pull/42#pullrequestreview-3",
        "user": { "login": "octocat", "avatar_url": null }
      }
    }"#;
    let (base_url, request, handle) = start_single_response_server_with_request("200 OK", body);
    let api = make_test_api_client(base_url);

    let _ = api
      .submit_pull_request_review(
        "acme",
        "widget",
        42,
        GithubPullRequestReviewEvent::Approve,
        "   ",
      )
      .expect("submit pull request review");

    handle.join().expect("join server thread");
    let request = request
      .lock()
      .expect("lock request")
      .clone()
      .unwrap_or_default();
    assert!(request.contains("\"event\":\"APPROVE\""));
    assert!(!request.contains("\"body\""));
  }

  #[test]
  fn fetch_github_notifications_parses_success_payload() {
    let body = r#"{
      "notifications": [
        {
          "id": "1",
          "repository": {
            "name": "widget",
            "full_name": "acme/widget",
            "owner": {
              "login": "acme",
              "avatar_url": "https://example.com/avatar.png"
            }
          },
          "subject": {
            "title": "Review requested",
            "type": "PullRequest",
            "url": "https://api.github.test/subject/1",
            "latest_comment_url": null
          },
          "reason": "review_requested",
          "unread": true,
          "updated_at": "2026-02-15T12:00:00Z",
          "last_read_at": null,
          "url": "https://api.github.test/notif/1",
          "subscription_url": "https://api.github.test/sub/1"
        }
      ]
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let notifications = api
      .fetch_github_notifications()
      .expect("fetch notifications");
    assert_eq!(notifications.len(), 1);
    assert_eq!(notifications[0].repository.full_name, "acme/widget");
    assert_eq!(notifications[0].subject.title, "Review requested");
    assert_eq!(
      notifications[0]
        .repository
        .owner
        .as_ref()
        .expect("owner")
        .login
        .as_str(),
      "acme"
    );
    assert!(notifications[0].unread);
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_me_returns_none_on_unauthorized() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "");
    let api = make_test_api_client(base_url);

    let me = api.fetch_me().expect("fetch_me should not fail on 401");
    assert!(me.is_none());
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_me_returns_error_on_non_success_status() {
    let (base_url, handle) = start_single_response_server("500 Internal Server Error", "{}");
    let api = make_test_api_client(base_url);

    let err = api.fetch_me().err();
    assert!(err.is_some());
    assert!(
      err
        .expect("error")
        .to_string()
        .contains("unexpected status")
    );
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_me_parses_subscription_and_portal_url() {
    let body = r#"{
      "id": "user_123",
      "name": "Joris",
      "email": "joris@example.com",
      "emailVerified": true,
      "image": null,
      "githubLogin": "joris-gallot",
      "role": "user",
      "subscription": {
        "portalUrl": "https://polar.sh/portal/session_123",
        "activeSubscription": {
          "id": "sub_123",
          "createdAt": "2026-02-20T10:00:00Z",
          "modifiedAt": null,
          "status": "active",
          "amount": 2000,
          "currency": "usd",
          "recurringInterval": "month",
          "currentPeriodStart": "2026-02-20T10:00:00Z",
          "currentPeriodEnd": "2026-03-20T10:00:00Z",
          "trialStart": null,
          "trialEnd": null,
          "cancelAtPeriodEnd": false,
          "canceledAt": null,
          "startedAt": "2026-02-20T10:00:00Z",
          "endsAt": null,
          "productId": "prod_123"
        }
      }
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let user = api
      .fetch_me()
      .expect("fetch me")
      .expect("authenticated user");

    assert_eq!(
      user.subscription.portal_url.as_deref(),
      Some("https://polar.sh/portal/session_123")
    );
    assert_eq!(
      user
        .subscription
        .active_subscription
        .as_ref()
        .map(|sub| sub.id.as_str()),
      Some("sub_123")
    );
    assert_eq!(
      user
        .subscription
        .active_subscription
        .as_ref()
        .map(|sub| sub.product_id.as_str()),
      Some("prod_123")
    );
    assert!(matches!(user.role, UserRole::User));
    assert!(user.has_pro_access());

    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_me_parses_admin_role_as_pro_access_without_subscription() {
    let body = r#"{
      "id": "user_admin",
      "name": "Admin",
      "email": "admin@example.com",
      "emailVerified": true,
      "image": null,
      "githubLogin": "admin-user",
      "role": "admin",
      "subscription": {
        "portalUrl": null,
        "activeSubscription": null
      }
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let user = api
      .fetch_me()
      .expect("fetch me")
      .expect("authenticated user");

    assert!(matches!(user.role, UserRole::Admin));
    assert!(user.subscription.active_subscription.is_none());
    assert!(user.has_pro_access());

    handle.join().expect("join server thread");
  }

  #[test]
  fn checkout_subscription_returns_checkout_url_on_success() {
    let body = r#"{"url":"https://polar.sh/checkout/session_123","redirect":false}"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let url = api
      .checkout_subscription("pro-annual")
      .expect("checkout subscription should return a URL");

    assert_eq!(url, "https://polar.sh/checkout/session_123");
    handle.join().expect("join server thread");
  }

  #[test]
  fn checkout_subscription_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "{}");
    let api = make_test_api_client(base_url);

    let err = api.checkout_subscription("pro-monthly").err();

    assert!(err.is_some());
    assert!(err.expect("error").to_string().contains("unauthorized"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn checkout_subscription_posts_expected_route_and_payload() {
    let body = r#"{"url":"https://polar.sh/checkout/session_123","redirect":false}"#;
    let (base_url, request, handle) = start_single_response_server_with_request("200 OK", body);
    let api = make_test_api_client(base_url);

    let _ = api
      .checkout_subscription("pro-annual")
      .expect("checkout request");

    handle.join().expect("join server thread");
    let request = request
      .lock()
      .expect("lock request")
      .clone()
      .unwrap_or_default();

    assert!(
      request.contains("POST /api/auth/checkout "),
      "request: {request}"
    );
    assert!(
      request.contains("\"slug\":\"pro-annual\""),
      "request: {request}"
    );
    assert!(request.contains("\"redirect\":false"), "request: {request}");
  }

  #[test]
  fn fetch_ai_settings_parses_configured_payload() {
    let body = r#"{
      "settings": {
        "configured": true,
        "credentialMode": "user_key",
        "provider": "anthropic",
        "model": "claude-sonnet-4-6",
        "apiKeyHint": "sk-...1234"
      }
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let settings = api.fetch_ai_settings().expect("ai settings");

    assert!(settings.configured);
    assert_eq!(settings.provider, Some(AiProvider::Anthropic));
    assert_eq!(settings.credential_mode, Some(AiCredentialMode::UserKey));
    assert_eq!(settings.model.as_deref(), Some("claude-sonnet-4-6"));
    assert_eq!(settings.api_key_hint.as_deref(), Some("sk-...1234"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn save_ai_settings_puts_expected_payload() {
    let body = r#"{
      "settings": {
        "configured": true,
        "credentialMode": "user_key",
        "provider": "openai",
        "model": "gpt-5.4-mini",
        "apiKeyHint": "sk-...abcd"
      }
    }"#;
    let (base_url, request, handle) = start_single_response_server_with_request("200 OK", body);
    let api = make_test_api_client(base_url);

    let settings = api
      .save_ai_settings(AiProvider::Openai, Some("gpt-5.4-mini"), "sk-test")
      .expect("save ai settings");

    assert!(settings.configured);
    handle.join().expect("join server thread");
    let request = request
      .lock()
      .expect("lock request")
      .clone()
      .unwrap_or_default();
    assert!(request.contains("PUT /ai/settings "), "request: {request}");
    assert!(
      request.contains("\"provider\":\"openai\""),
      "request: {request}"
    );
    assert!(
      request.contains("\"apiKey\":\"sk-test\""),
      "request: {request}"
    );
    assert!(
      request.contains("\"model\":\"gpt-5.4-mini\""),
      "request: {request}"
    );
  }

  #[test]
  fn generate_github_pr_brief_posts_pr_identity_and_parses_brief() {
    let body = r#"{
      "brief": {
        "summary": ["Touches parser and test coverage."],
        "reviewFirst": [{
          "path": "src/parser.rs",
          "reason": "Core behavior changed.",
          "priority": "high",
          "target": { "type": "pr_file", "path": "src/parser.rs" }
        }],
        "risks": [{
          "title": "Parser regression",
          "detail": "Fallback handling changed.",
          "path": "src/parser.rs",
          "target": { "type": "pr_file", "path": "src/parser.rs" }
        }],
        "blockers": [{
          "type": "check",
          "label": "CI failed",
          "detail": "Unit tests are failing."
        }],
        "generatedAt": "2026-05-05T18:00:00.000Z",
        "owner": "acme",
        "repo": "widget",
        "pullNumber": 42,
        "headSha": "deadbeef",
        "contextHash": "hash",
        "provider": "openai",
        "credentialMode": "user_key",
        "model": "gpt-5.4-mini",
        "cached": false
      }
    }"#;
    let (base_url, request, handle) = start_single_response_server_with_request("200 OK", body);
    let api = make_test_api_client(base_url);

    let brief = api
      .generate_github_pr_brief("acme", "widget", 42, false)
      .expect("generate brief");

    assert_eq!(brief.summary, vec!["Touches parser and test coverage."]);
    assert_eq!(brief.review_first[0].priority, AiPrBriefPriority::High);
    assert_eq!(
      brief.review_first[0]
        .target
        .as_ref()
        .map(|target| target.path.as_str()),
      Some("src/parser.rs")
    );
    handle.join().expect("join server thread");
    let request = request
      .lock()
      .expect("lock request")
      .clone()
      .unwrap_or_default();
    assert!(
      request.contains("POST /ai/github/pr/brief "),
      "request: {request}"
    );
    assert!(request.contains("\"owner\":\"acme\""), "request: {request}");
    assert!(
      request.contains("\"repo\":\"widget\""),
      "request: {request}"
    );
    assert!(request.contains("\"pullNumber\":42"), "request: {request}");
    assert!(
      request.contains("\"forceRefresh\":false"),
      "request: {request}"
    );
  }

  #[test]
  fn fetch_github_pull_requests_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "");
    let api = make_test_api_client(base_url);
    let filters = crate::github_home_tabs::GithubPullRequestSearchFilters::default();

    let err = api.fetch_github_pull_requests(&filters).err();
    assert!(err.is_some());
    assert!(err.expect("error").to_string().contains("unauthorized"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_github_pull_request_filter_options_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "");
    let api = make_test_api_client(base_url);

    let err = api
      .fetch_github_pull_request_filter_options(&["acme/reviu".to_string()])
      .err();
    assert!(err.is_some());
    assert!(err.expect("error").to_string().contains("unauthorized"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_github_user_repositories_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "");
    let api = make_test_api_client(base_url);

    let err = api.fetch_github_user_repositories().err();
    assert!(err.is_some());
    assert!(err.expect("error").to_string().contains("unauthorized"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_github_user_profile_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "");
    let api = make_test_api_client(base_url);

    let err = api.fetch_github_user_profile("octocat").err();
    assert!(err.is_some());
    assert!(err.expect("error").to_string().contains("unauthorized"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_github_repository_details_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "");
    let api = make_test_api_client(base_url);

    let err = api.fetch_github_repository_details("acme", "widget").err();
    assert!(err.is_some());
    assert!(err.expect("error").to_string().contains("unauthorized"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_github_repository_pull_requests_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "");
    let api = make_test_api_client(base_url);
    let filters = GithubPullRequestSearchFilters::default();

    let err = api
      .fetch_github_repository_pull_requests("acme", "widget", &filters, "open", 1)
      .err();
    assert!(err.is_some());
    assert!(err.expect("error").to_string().contains("unauthorized"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_pull_request_for_branch_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "");
    let api = make_test_api_client(base_url);

    let err = api
      .fetch_pull_request_for_branch("acme", "widget", "feature/parser")
      .err();
    assert!(err.is_some());
    assert!(err.expect("error").to_string().contains("unauthorized"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn create_pull_request_returns_backend_api_error() {
    let body = r#"{"error":"A pull request already exists for acme:feature/parser."}"#;
    let (base_url, handle) = start_single_response_server("422 Unprocessable Entity", body);
    let api = make_test_api_client(base_url);

    let err = api
      .create_pull_request(
        "acme",
        "widget",
        "feature/parser",
        "Improve docs",
        "main",
        None,
        false,
      )
      .err()
      .expect("error");
    assert_eq!(
      err.to_string(),
      "A pull request already exists for acme:feature/parser."
    );
    handle.join().expect("join server thread");
  }

  #[test]
  fn create_pull_request_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "");
    let api = make_test_api_client(base_url);

    let err = api
      .create_pull_request(
        "acme",
        "widget",
        "feature/parser",
        "Improve docs",
        "main",
        None,
        false,
      )
      .err();
    assert!(err.is_some());
    assert!(err.expect("error").to_string().contains("unauthorized"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_github_repository_tree_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "{}");
    let api = make_test_api_client(base_url);

    let err = api
      .fetch_github_repository_tree("acme", "widget", "main")
      .err();
    assert!(err.is_some());
    assert!(err.expect("error").to_string().contains("unauthorized"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_github_repository_branches_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "{}");
    let api = make_test_api_client(base_url);

    let err = api.fetch_github_repository_branches("acme", "widget").err();
    assert!(err.is_some());
    assert!(err.expect("error").to_string().contains("unauthorized"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_github_repository_readme_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "{}");
    let api = make_test_api_client(base_url);

    let err = api
      .fetch_github_repository_readme("acme", "widget", Some("main"))
      .err();
    assert!(err.is_some());
    assert!(err.expect("error").to_string().contains("unauthorized"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_github_repository_issue_details_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "");
    let api = make_test_api_client(base_url);

    let err = api
      .fetch_github_repository_issue_details("acme", "widget", 77)
      .err();
    assert!(err.is_some());
    assert!(err.expect("error").to_string().contains("unauthorized"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn update_issue_description_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "");
    let api = make_test_api_client(base_url);

    let err = api
      .update_issue_description("acme", "widget", 77, "Body")
      .err();
    assert!(err.is_some());
    assert!(err.expect("error").to_string().contains("unauthorized"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn create_issue_comment_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "");
    let api = make_test_api_client(base_url);

    let err = api
      .create_issue_comment("acme", "widget", 77, "New issue comment")
      .err();
    assert!(err.is_some());
    assert!(err.expect("error").to_string().contains("unauthorized"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn update_issue_comment_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "");
    let api = make_test_api_client(base_url);

    let err = api
      .update_issue_comment("acme", "widget", 77, 9002, "Updated issue comment")
      .err();
    assert!(err.is_some());
    assert!(err.expect("error").to_string().contains("unauthorized"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn delete_issue_comment_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "");
    let api = make_test_api_client(base_url);

    let err = api.delete_issue_comment("acme", "widget", 77, 9002).err();
    assert!(err.is_some());
    assert!(err.expect("error").to_string().contains("unauthorized"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_pull_request_details_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "");
    let api = make_test_api_client(base_url);

    let err = api.fetch_pull_request_details("acme", "widget", 42).err();
    assert!(err.is_some());
    assert!(err.expect("error").to_string().contains("unauthorized"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn update_pull_request_description_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "");
    let api = make_test_api_client(base_url);

    let err = api
      .update_pull_request_description("acme", "widget", 42, "Body")
      .err();
    assert!(err.is_some());
    assert!(err.expect("error").to_string().contains("unauthorized"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn update_pull_request_base_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "");
    let api = make_test_api_client(base_url);

    let err = api
      .update_pull_request_base("acme", "widget", 42, "release/next")
      .err();
    assert!(err.is_some());
    assert!(err.expect("error").to_string().contains("unauthorized"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn add_pull_request_assignee_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "");
    let api = make_test_api_client(base_url);

    let err = api
      .add_pull_request_assignee("acme", "widget", 42, "alice")
      .err();
    assert!(err.is_some());
    assert!(err.expect("error").to_string().contains("unauthorized"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn add_pull_request_label_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "");
    let api = make_test_api_client(base_url);

    let err = api
      .add_pull_request_label("acme", "widget", 42, "bug")
      .err();
    assert!(err.is_some());
    assert!(err.expect("error").to_string().contains("unauthorized"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn remove_pull_request_label_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "");
    let api = make_test_api_client(base_url);

    let err = api
      .remove_pull_request_label("acme", "widget", 42, "bug")
      .err();
    assert!(err.is_some());
    assert!(err.expect("error").to_string().contains("unauthorized"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn request_pull_request_reviewer_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "");
    let api = make_test_api_client(base_url);

    let err = api
      .request_pull_request_reviewer("acme", "widget", 42, "bob")
      .err();
    assert!(err.is_some());
    assert!(err.expect("error").to_string().contains("unauthorized"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_pull_request_files_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "");
    let api = make_test_api_client(base_url);

    let err = api
      .fetch_pull_request_files("acme", "widget", 42, None)
      .err();
    assert!(err.is_some());
    assert!(err.expect("error").to_string().contains("unauthorized"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_pull_request_commits_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "");
    let api = make_test_api_client(base_url);

    let err = api.fetch_pull_request_commits("acme", "widget", 42).err();
    assert!(err.is_some());
    assert!(err.expect("error").to_string().contains("unauthorized"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_pull_request_conversation_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "");
    let api = make_test_api_client(base_url);

    let err = api
      .fetch_pull_request_conversation("acme", "widget", 42)
      .err();
    assert!(err.is_some());
    assert!(err.expect("error").to_string().contains("unauthorized"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn submit_pull_request_review_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "");
    let api = make_test_api_client(base_url);

    let err = api
      .submit_pull_request_review(
        "acme",
        "widget",
        42,
        GithubPullRequestReviewEvent::Comment,
        "Looks good",
      )
      .err();
    assert!(err.is_some());
    assert!(err.expect("error").to_string().contains("unauthorized"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn update_pull_request_review_comment_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "");
    let api = make_test_api_client(base_url);

    let err = api
      .update_pull_request_review_comment("acme", "widget", 42, 1, "Updated body")
      .err();
    assert!(err.is_some());
    assert!(err.expect("error").to_string().contains("unauthorized"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn create_pull_request_review_comment_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "");
    let api = make_test_api_client(base_url);

    let err = api
      .create_pull_request_review_comment(
        "acme",
        "widget",
        42,
        "src/main.rs",
        "head123",
        1,
        "RIGHT",
        None,
        None,
        "New comment body",
      )
      .err();
    assert!(err.is_some());
    assert!(err.expect("error").to_string().contains("unauthorized"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn reply_pull_request_review_comment_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "");
    let api = make_test_api_client(base_url);

    let err = api
      .reply_pull_request_review_comment("acme", "widget", 42, 2, "Reply body")
      .err();
    assert!(err.is_some());
    assert!(err.expect("error").to_string().contains("unauthorized"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn delete_pull_request_review_comment_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "");
    let api = make_test_api_client(base_url);

    let err = api
      .delete_pull_request_review_comment("acme", "widget", 42, 2)
      .err();
    assert!(err.is_some());
    assert!(err.expect("error").to_string().contains("unauthorized"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_github_notifications_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "");
    let api = make_test_api_client(base_url);

    let err = api.fetch_github_notifications().err();
    assert!(err.is_some());
    assert!(err.expect("error").to_string().contains("unauthorized"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_github_file_content_returns_content_on_success() {
    let body = r##"{"content":"# Hello\n"}"##;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let content = api
      .fetch_github_file_content("acme", "widget", "README.md", "main")
      .expect("fetch content");
    assert_eq!(content.as_deref(), Some("# Hello\n"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_github_file_content_returns_none_when_content_is_null() {
    let body = r#"{"content":null}"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let content = api
      .fetch_github_file_content("acme", "widget", "README.md", "main")
      .expect("fetch content");
    assert_eq!(content, None);
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_github_file_content_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "{}");
    let api = make_test_api_client(base_url);

    let err = api
      .fetch_github_file_content("acme", "widget", "README.md", "main")
      .err();
    assert!(err.is_some());
    assert!(err.expect("error").to_string().contains("unauthorized"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_github_file_content_returns_error_on_non_success_status() {
    let (base_url, handle) = start_single_response_server("500 Internal Server Error", "{}");
    let api = make_test_api_client(base_url);

    let err = api
      .fetch_github_file_content("acme", "widget", "README.md", "main")
      .err();
    assert!(err.is_some());
    assert!(
      err
        .expect("error")
        .to_string()
        .contains("unexpected status")
    );
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_github_file_asset_decodes_base64_payload() {
    let body = r##"{"contentBase64":"aGVsbG8="}"##;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let content = api
      .fetch_github_file_asset("acme", "widget", "image.png", "main")
      .expect("fetch asset");
    assert_eq!(content, Some(b"hello".to_vec()));
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_github_file_asset_returns_none_when_content_is_null() {
    let body = r#"{"contentBase64":null}"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let content = api
      .fetch_github_file_asset("acme", "widget", "image.png", "main")
      .expect("fetch asset");
    assert_eq!(content, None);
    handle.join().expect("join server thread");
  }

  #[test]
  fn check_desktop_update_parses_success_payload() {
    let body = r#"{
      "updateAvailable": true,
      "forceUpdate": false,
      "currentVersion": "0.1.0",
      "latestVersion": "0.2.0",
      "minimumSupportedVersion": "0.1.0",
      "releaseNotesUrl": "https://reviu.dev/releases/0.2.0",
      "artifact": {
        "url": "https://api.reviu.dev/desktop/update/download/release/v0.2.0/Reviu-0.2.0-macos-aarch64.dmg",
        "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "size": 1024
      }
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let payload = api
      .check_desktop_update("0.1.0", "macos", "aarch64")
      .expect("check desktop update");
    assert!(payload.update_available);
    assert!(!payload.force_update);
    assert_eq!(payload.current_version, "0.1.0");
    assert_eq!(payload.latest_version, "0.2.0");
    assert_eq!(payload.minimum_supported_version, "0.1.0");
    assert_eq!(
      payload.release_notes_url,
      "https://reviu.dev/releases/0.2.0"
    );
    assert_eq!(
      payload
        .artifact
        .as_ref()
        .map(|artifact| artifact.url.as_str()),
      Some(
        "https://api.reviu.dev/desktop/update/download/release/v0.2.0/Reviu-0.2.0-macos-aarch64.dmg"
      )
    );
    handle.join().expect("join server thread");
  }

  #[test]
  fn check_desktop_update_posts_expected_route_and_payload() {
    let body = r#"{
      "updateAvailable": false,
      "forceUpdate": false,
      "currentVersion": "0.1.0",
      "latestVersion": "0.1.0",
      "minimumSupportedVersion": "0.1.0",
      "releaseNotesUrl": "https://reviu.dev/releases/0.1.0",
      "artifact": null
    }"#;
    let (base_url, request, handle) = start_single_response_server_with_request("200 OK", body);
    let api = make_test_api_client(base_url);

    let _ = api
      .check_desktop_update("0.1.0", "macos", "aarch64")
      .expect("check desktop update");

    handle.join().expect("join server thread");
    let request = request
      .lock()
      .expect("lock request")
      .clone()
      .unwrap_or_default();

    assert!(
      request.contains("POST /desktop/update/check "),
      "request: {request}"
    );
    assert!(
      request.contains("\"currentVersion\":\"0.1.0\""),
      "request: {request}"
    );
    assert!(
      request.contains("\"platform\":\"macos\""),
      "request: {request}"
    );
    assert!(
      request.contains("\"arch\":\"aarch64\""),
      "request: {request}"
    );
  }

  #[test]
  fn check_desktop_update_returns_error_on_non_success_status() {
    let (base_url, handle) = start_single_response_server("502 Bad Gateway", "{}");
    let api = make_test_api_client(base_url);

    let err = api.check_desktop_update("0.1.0", "macos", "aarch64").err();
    assert!(err.is_some());
    assert!(
      err
        .expect("error")
        .to_string()
        .contains("unexpected status")
    );
    handle.join().expect("join server thread");
  }

  #[test]
  fn sign_out_clears_bearer_token_when_request_fails() {
    let (base_url, handle) = start_single_response_server("500 Internal Server Error", "{}");
    let api = make_test_api_client(base_url);
    api.set_bearer_token("token".to_string());

    let err = api.sign_out().err();
    assert!(err.is_some());
    assert_eq!(api.bearer_token(), None);
    handle.join().expect("join server thread");
  }

  #[test]
  fn sign_out_clears_bearer_token_on_success() {
    let (base_url, handle) = start_single_response_server("200 OK", "{}");
    let api = make_test_api_client(base_url);
    api.set_bearer_token("token".to_string());

    api.sign_out().expect("sign out success");

    assert_eq!(api.bearer_token(), None);
    handle.join().expect("join server thread");
  }
}
