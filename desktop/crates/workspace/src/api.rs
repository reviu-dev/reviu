use anyhow::Result;
use reqwest::blocking::Client;
use reqwest::{Method, StatusCode};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::{Arc, Mutex};

use crate::AppProfile;
use crate::crash_report::StartupCrashReport;
use crate::sentry_context;

const DEFAULT_API_BASE_URL: &str = "http://localhost:3001";
const KEYCHAIN_USERNAME: &str = "bearer";

#[derive(Debug)]
pub struct ApiError {
  message: String,
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

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubRepository {
  pub owner: String,
  pub repo: String,
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
  pub comments_count: u64,
  pub repository: GithubRepository,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubIssueDetailsComment {
  #[serde(rename = "node_id", default)]
  pub node_id: String,
  pub id: u64,
  pub body: Option<String>,
  #[serde(rename = "created_at")]
  pub created_at: String,
  #[serde(rename = "updated_at")]
  pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubIssueDetails {
  #[serde(rename = "node_id", default)]
  pub node_id: String,
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
  pub comments: Vec<GithubIssueDetailsComment>,
  pub repository: GithubRepository,
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
pub struct GithubRepositoryDetails {
  #[serde(rename = "default_branch")]
  pub default_branch: String,
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

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct GithubPullRequestFilterOptionUser {
  pub login: String,
  #[serde(rename = "avatar_url")]
  pub avatar_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubPullRequest {
  pub number: u64,
  pub title: String,
  pub state: GithubPullRequestState,
  #[serde(rename = "merged_at")]
  pub merged_at: Option<String>,
  pub draft: bool,
  #[serde(rename = "comments_count", default)]
  pub comments_count: u64,
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
#[allow(dead_code)]
pub struct GithubPullRequestAuthor {
  #[serde(default)]
  pub login: String,
  #[serde(rename = "avatar_url")]
  pub avatar_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubPullRequestCommitUser {
  pub login: String,
  #[serde(rename = "avatar_url")]
  pub avatar_url: Option<String>,
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

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubPullRequestIssueComment {
  #[serde(rename = "node_id", default)]
  pub node_id: String,
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
  // Backend marks these for comments belonging to the viewer's unsubmitted pending review.
  #[serde(default)]
  pub is_pending: bool,
  #[serde(default, rename = "pull_request_review_node_id")]
  pub pull_request_review_node_id: Option<String>,
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
#[allow(dead_code)]
pub struct GithubPullRequestDetails {
  #[serde(rename = "node_id")]
  pub node_id: String,
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

#[derive(Debug, Deserialize)]
struct GithubPullRequestDetailsResponse {
  #[serde(rename = "pullRequest")]
  pull_request: GithubPullRequestDetails,
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

#[derive(Debug, Deserialize, Default)]
pub struct GithubPullRequestConversationPullRequest {
  #[serde(rename = "node_id")]
  pub node_id: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
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
struct GithubPullRequestCommentResponse {
  comment: GithubPullRequestReviewComment,
}

#[derive(Debug, Deserialize)]
struct GithubPullRequestReviewResponse {
  review: GithubPullRequestReview,
}

#[derive(Debug, Deserialize)]
struct GithubPendingReviewReplyResponse {
  #[serde(rename = "node_id")]
  node_id: String,
}

#[derive(Debug, Serialize)]
struct UpdateGithubPullRequestCommentRequest<'a> {
  body: &'a str,
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
struct CreateGithubPullRequestReviewRequest<'a> {
  event: GithubPullRequestReviewEvent,
  #[serde(skip_serializing_if = "Option::is_none")]
  body: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct StartGithubPendingReviewRequest<'a> {
  #[serde(rename = "pullRequestId")]
  pull_request_id: &'a str,
}

#[derive(Debug, Serialize)]
struct AddGithubPendingReviewThreadRequest<'a> {
  #[serde(rename = "pullRequestId")]
  pull_request_id: &'a str,
  #[serde(rename = "pullRequestReviewId")]
  pull_request_review_id: &'a str,
  path: &'a str,
  body: &'a str,
  #[serde(rename = "subjectType")]
  subject_type: &'a str,
  #[serde(skip_serializing_if = "Option::is_none")]
  line: Option<u64>,
  #[serde(skip_serializing_if = "Option::is_none")]
  side: Option<&'a str>,
  #[serde(rename = "startLine", skip_serializing_if = "Option::is_none")]
  start_line: Option<u64>,
  #[serde(rename = "startSide", skip_serializing_if = "Option::is_none")]
  start_side: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct ReplyGithubPendingReviewThreadRequest<'a> {
  #[serde(rename = "pullRequestReviewId")]
  pull_request_review_id: &'a str,
  #[serde(rename = "pullRequestReviewThreadId")]
  pull_request_review_thread_id: &'a str,
  body: &'a str,
}

#[derive(Debug, Serialize)]
struct SubmitGithubPendingReviewRequest<'a> {
  #[serde(rename = "pullRequestReviewId")]
  pull_request_review_id: &'a str,
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

#[derive(Debug, Deserialize)]
struct GithubFileContentResponse {
  content: Option<String>,
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

    ApiError { message }.into()
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

  pub fn start_pending_review(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
    pull_request_id: &str,
  ) -> Result<GithubPullRequestReview> {
    let route = format!("/github/pr/{number}/pending-review");
    let response = self
      .authed_request(Method::POST, route.as_str())
      .query(&[("org", owner), ("repo", repo)])
      .json(&StartGithubPendingReviewRequest { pull_request_id })
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

  #[allow(clippy::too_many_arguments)]
  pub fn add_pending_review_thread(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
    pull_request_id: &str,
    pull_request_review_id: &str,
    path: &str,
    body: &str,
    subject_type: &str,
    line: Option<u64>,
    side: Option<&str>,
    start_line: Option<u64>,
    start_side: Option<&str>,
  ) -> Result<GithubPullRequestReviewComment> {
    let route = format!("/github/pr/{number}/pending-review/threads");
    let response = self
      .authed_request(Method::POST, route.as_str())
      .query(&[("org", owner), ("repo", repo)])
      .json(&AddGithubPendingReviewThreadRequest {
        pull_request_id,
        pull_request_review_id,
        path,
        body,
        subject_type,
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

  pub fn reply_pending_review_thread(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
    pull_request_review_id: &str,
    pull_request_review_thread_id: &str,
    body: &str,
  ) -> Result<String> {
    let route = format!("/github/pr/{number}/pending-review/replies");
    let response = self
      .authed_request(Method::POST, route.as_str())
      .query(&[("org", owner), ("repo", repo)])
      .json(&ReplyGithubPendingReviewThreadRequest {
        pull_request_review_id,
        pull_request_review_thread_id,
        body,
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
    let payload = response.json::<GithubPendingReviewReplyResponse>()?;
    Ok(payload.node_id)
  }

  pub fn update_pending_review_comment(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
    comment_node_id: &str,
    body: &str,
  ) -> Result<()> {
    let route = format!("/github/pr/{number}/pending-review/comments/{comment_node_id}");
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
    Ok(())
  }

  pub fn delete_pending_review_comment(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
    comment_node_id: &str,
  ) -> Result<()> {
    let route = format!("/github/pr/{number}/pending-review/comments/{comment_node_id}");
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

  pub fn submit_pending_review(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
    pull_request_review_id: &str,
    event: GithubPullRequestReviewEvent,
    body: &str,
  ) -> Result<GithubPullRequestReview> {
    let route = format!("/github/pr/{number}/pending-review/submit");
    let trimmed_body = body.trim();
    let payload = SubmitGithubPendingReviewRequest {
      pull_request_review_id,
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
        body.len(),
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
        body.len(),
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
        body.len(),
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
      merged_at: merged_at.map(str::to_string),
      draft,
      comments_count: 0,
      repository: GithubRepository {
        owner: "acme".to_string(),
        repo: "widget".to_string(),
      },
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
        pathname: Some("/session".to_string()),
        workspace_page: Some("session".to_string()),
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
      request.contains("\"workspacePage\":\"session\""),
      "request: {request}"
    );
    assert!(
      request.contains("\"gitContext\":{\"repoName\":\"reviu\""),
      "request: {request}"
    );
  }

  #[test]
  fn fetch_github_repository_details_parses_default_branch() {
    let body = r#"{
      "name": "widget",
      "full_name": "acme/widget",
      "default_branch": "main",
      "html_url": "https://github.com/acme/widget"
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let details = api
      .fetch_github_repository_details("acme", "widget")
      .expect("fetch repository details");
    assert_eq!(details.default_branch, "main");
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_pull_request_for_branch_parses_success_payload() {
    let body = r#"{
      "pullRequest": {
        "number": 11,
        "title": "Improve docs",
        "state": "open",
        "merged_at": null,
        "draft": false,
        "comments_count": 7,
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
    assert_eq!(pull_request.comments_count, 7);
    assert!(!pull_request.draft);
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
        "merged_at": null,
        "draft": true,
        "comments_count": 7,
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
    let body = r#"{"pullRequest":{"number":11,"title":"Improve docs","state":"open","merged_at":null,"draft":false,"comments_count":0,"repository":{"owner":"acme","repo":"widget"}}}"#;
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
        "author": { "login": "octocat", "avatar_url": null },
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
    assert_eq!(details.assignees[0].login, "alice");
    assert_eq!(details.requested_reviewers[0].login, "bob");
    assert_eq!(details.comments, 2);
    assert_eq!(details.review_comments, 3);
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
        "rebaseable": true
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
        "rebaseable": null
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

    // The message is what the app shows; nothing reads the code.
    assert_eq!(err.to_string(), "Base branch moved; refresh and try again.");
    assert!(err.downcast_ref::<ApiError>().is_some());
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_pull_request_conversation_parses_success_payload() {
    let body = r#"{
      "conversation": {
        "pull_request": { "node_id": "PR_kwDOExample" },
        "issue_comments": [
          {
            "node_id": "IC_kwDOExample",
            "id": 11,
            "body": "Can you add tests?",
            "created_at": "2026-02-28T10:00:00Z",
            "updated_at": "2026-02-28T10:05:00Z"
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
  fn add_pending_review_thread_uses_expected_route_and_payload() {
    let body = r#"{
      "comment": {
        "id": 5,
        "pull_request_review_id": 12,
        "diff_hunk": "@@ -1 +1 @@",
        "path": "src/main.rs",
        "position": 1,
        "original_position": 1,
        "commit_id": "head123",
        "original_commit_id": "base123",
        "in_reply_to_id": null,
        "user": { "login": "octocat", "avatar_url": null },
        "body": "Draft comment",
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
    let (base_url, request, handle) = start_single_response_server_with_request("200 OK", body);
    let api = make_test_api_client(base_url);

    let comment = api
      .add_pending_review_thread(
        "acme",
        "widget",
        42,
        "PR_node",
        "PRR_node",
        "src/main.rs",
        "Draft comment",
        "LINE",
        Some(1),
        Some("RIGHT"),
        None,
        None,
      )
      .expect("add pending review thread");
    assert_eq!(comment.id, 5);
    assert_eq!(comment.body, "Draft comment");

    handle.join().expect("join server thread");
    let request = request
      .lock()
      .expect("lock request")
      .clone()
      .unwrap_or_default();
    assert!(request.contains("POST /github/pr/42/pending-review/threads"));
    assert!(request.contains("\"pullRequestId\":\"PR_node\""));
    assert!(request.contains("\"pullRequestReviewId\":\"PRR_node\""));
    assert!(request.contains("\"subjectType\":\"LINE\""));
    assert!(request.contains("\"line\":1"));
  }

  #[test]
  fn submit_pending_review_uses_expected_route_and_payload() {
    let body = r#"{
      "review": {
        "id": 12,
        "state": "APPROVED",
        "html_url": "https://github.com/acme/widget/pull/42",
        "body": "LGTM",
        "user": { "login": "octocat", "avatar_url": null }
      }
    }"#;
    let (base_url, request, handle) = start_single_response_server_with_request("200 OK", body);
    let api = make_test_api_client(base_url);

    let review = api
      .submit_pending_review(
        "acme",
        "widget",
        42,
        "PRR_node",
        GithubPullRequestReviewEvent::Approve,
        "LGTM",
      )
      .expect("submit pending review");
    assert_eq!(review.id, 12);

    handle.join().expect("join server thread");
    let request = request
      .lock()
      .expect("lock request")
      .clone()
      .unwrap_or_default();
    assert!(request.contains("POST /github/pr/42/pending-review/submit"));
    assert!(request.contains("\"pullRequestReviewId\":\"PRR_node\""));
    assert!(request.contains("\"event\":\"APPROVE\""));
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

  const PENDING_REVIEW_BODY: &str = r#"{
    "review": {
      "id": 500,
      "node_id": "PRR_kwDOPending",
      "body": null,
      "state": "PENDING",
      "submitted_at": null,
      "commit_id": null,
      "html_url": "https://github.com/acme/widget/pull/42#pullrequestreview-500",
      "user": { "login": "octocat", "avatar_url": null }
    }
  }"#;

  const PENDING_THREAD_COMMENT_BODY: &str = r#"{
    "comment": {
      "id": 7,
      "node_id": "PRRC_kwDOThread",
      "pull_request_review_id": 500,
      "diff_hunk": "@@ -1 +1 @@",
      "path": "src/main.rs",
      "position": 1,
      "original_position": 1,
      "commit_id": "head123",
      "original_commit_id": "base123",
      "in_reply_to_id": null,
      "user": { "login": "octocat", "avatar_url": null },
      "body": "Needs a guard",
      "created_at": "2026-02-15T12:00:00Z",
      "updated_at": "2026-02-15T12:00:00Z",
      "start_line": null,
      "original_start_line": null,
      "start_side": null,
      "line": 5,
      "original_line": 5,
      "side": "RIGHT"
    }
  }"#;

  #[test]
  fn start_pending_review_posts_the_pull_request_node_id() {
    let (base_url, request, handle) =
      start_single_response_server_with_request("200 OK", PENDING_REVIEW_BODY);
    let api = make_test_api_client(base_url);

    let review = api
      .start_pending_review("acme", "widget", 42, "PR_kwDOExample")
      .expect("start pending review");

    assert_eq!(review.id, 500);
    assert_eq!(review.state, GithubPullRequestReviewState::Pending);
    assert_eq!(review.node_id, "PRR_kwDOPending");
    handle.join().expect("join server thread");
    let request = request
      .lock()
      .expect("lock request")
      .clone()
      .unwrap_or_default();
    assert!(
      request.contains("POST /github/pr/42/pending-review?org=acme&repo=widget "),
      "request: {request}"
    );
    assert!(request.contains("\"pullRequestId\":\"PR_kwDOExample\""));
  }

  #[test]
  fn start_pending_review_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 UNAUTHORIZED", "{}");
    let api = make_test_api_client(base_url);

    let err = api
      .start_pending_review("acme", "widget", 42, "PR_kwDOExample")
      .expect_err("start pending review should fail");

    assert_eq!(err.to_string(), "unauthorized");
    handle.join().expect("join server thread");
  }

  #[test]
  fn add_pending_review_thread_serializes_a_single_line_thread() {
    let (base_url, request, handle) =
      start_single_response_server_with_request("200 OK", PENDING_THREAD_COMMENT_BODY);
    let api = make_test_api_client(base_url);

    let comment = api
      .add_pending_review_thread(
        "acme",
        "widget",
        42,
        "PR_kwDOExample",
        "PRR_kwDOPending",
        "src/main.rs",
        "Needs a guard",
        "LINE",
        Some(5),
        Some("RIGHT"),
        None,
        None,
      )
      .expect("add pending review thread");

    assert_eq!(comment.id, 7);
    assert_eq!(comment.body, "Needs a guard");
    handle.join().expect("join server thread");
    let request = request
      .lock()
      .expect("lock request")
      .clone()
      .unwrap_or_default();
    assert!(
      request.contains("POST /github/pr/42/pending-review/threads?org=acme&repo=widget "),
      "request: {request}"
    );
    assert!(request.contains("\"pullRequestId\":\"PR_kwDOExample\""));
    assert!(request.contains("\"pullRequestReviewId\":\"PRR_kwDOPending\""));
    assert!(request.contains("\"subjectType\":\"LINE\""));
    assert!(request.contains("\"line\":5"));
    assert!(request.contains("\"side\":\"RIGHT\""));
    assert!(!request.contains("startLine"), "request: {request}");
    assert!(!request.contains("startSide"), "request: {request}");
  }

  #[test]
  fn add_pending_review_thread_serializes_a_multi_line_range() {
    let (base_url, request, handle) =
      start_single_response_server_with_request("200 OK", PENDING_THREAD_COMMENT_BODY);
    let api = make_test_api_client(base_url);

    let _ = api
      .add_pending_review_thread(
        "acme",
        "widget",
        42,
        "PR_kwDOExample",
        "PRR_kwDOPending",
        "src/main.rs",
        "Needs a guard",
        "LINE",
        Some(5),
        Some("RIGHT"),
        Some(3),
        Some("RIGHT"),
      )
      .expect("add pending review thread");

    handle.join().expect("join server thread");
    let request = request
      .lock()
      .expect("lock request")
      .clone()
      .unwrap_or_default();
    assert!(request.contains("\"startLine\":3"));
    assert!(request.contains("\"startSide\":\"RIGHT\""));
  }

  #[test]
  fn reply_pending_review_thread_returns_the_new_comment_node() {
    let (base_url, request, handle) =
      start_single_response_server_with_request("200 OK", r#"{"node_id":"PRRC_kwDOReply"}"#);
    let api = make_test_api_client(base_url);

    let node_id = api
      .reply_pending_review_thread(
        "acme",
        "widget",
        42,
        "PRR_kwDOPending",
        "PRRT_kwDOThread",
        "Same remark here",
      )
      .expect("reply pending review thread");

    assert_eq!(node_id, "PRRC_kwDOReply");
    handle.join().expect("join server thread");
    let request = request
      .lock()
      .expect("lock request")
      .clone()
      .unwrap_or_default();
    assert!(
      request.contains("POST /github/pr/42/pending-review/replies?org=acme&repo=widget "),
      "request: {request}"
    );
    assert!(request.contains("\"pullRequestReviewId\":\"PRR_kwDOPending\""));
    assert!(request.contains("\"pullRequestReviewThreadId\":\"PRRT_kwDOThread\""));
    assert!(request.contains("\"body\":\"Same remark here\""));
  }

  #[test]
  fn reply_pending_review_thread_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 UNAUTHORIZED", "{}");
    let api = make_test_api_client(base_url);

    let err = api
      .reply_pending_review_thread("acme", "widget", 42, "PRR_x", "PRRT_x", "text")
      .expect_err("reply pending review thread should fail");

    assert_eq!(err.to_string(), "unauthorized");
    handle.join().expect("join server thread");
  }

  #[test]
  fn update_pending_review_comment_patches_the_comment_node() {
    let (base_url, request, handle) = start_single_response_server_with_request("200 OK", "{}");
    let api = make_test_api_client(base_url);

    api
      .update_pending_review_comment("acme", "widget", 42, "PRRC_kwDOThread", "Sharper wording")
      .expect("update pending review comment");

    handle.join().expect("join server thread");
    let request = request
      .lock()
      .expect("lock request")
      .clone()
      .unwrap_or_default();
    assert!(
      request.contains(
        "PATCH /github/pr/42/pending-review/comments/PRRC_kwDOThread?org=acme&repo=widget "
      ),
      "request: {request}"
    );
    assert!(request.contains("\"body\":\"Sharper wording\""));
  }

  #[test]
  fn delete_pending_review_comment_uses_the_delete_route() {
    let (base_url, request_line, handle) =
      start_single_response_server_with_request_line("200 OK", "{}");
    let api = make_test_api_client(base_url);

    api
      .delete_pending_review_comment("acme", "widget", 42, "PRRC_kwDOThread")
      .expect("delete pending review comment");

    handle.join().expect("join server thread");
    let request_line = request_line
      .lock()
      .expect("lock request line")
      .clone()
      .unwrap_or_default();
    assert_eq!(
      request_line,
      "DELETE /github/pr/42/pending-review/comments/PRRC_kwDOThread?org=acme&repo=widget HTTP/1.1"
    );
  }

  #[test]
  fn delete_pending_review_comment_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 UNAUTHORIZED", "{}");
    let api = make_test_api_client(base_url);

    let err = api
      .delete_pending_review_comment("acme", "widget", 42, "PRRC_x")
      .expect_err("delete pending review comment should fail");

    assert_eq!(err.to_string(), "unauthorized");
    handle.join().expect("join server thread");
  }

  #[test]
  fn submit_pending_review_serializes_review_id_event_and_trimmed_body() {
    let (base_url, request, handle) =
      start_single_response_server_with_request("200 OK", PENDING_REVIEW_BODY);
    let api = make_test_api_client(base_url);

    let _ = api
      .submit_pending_review(
        "acme",
        "widget",
        42,
        "PRR_kwDOPending",
        GithubPullRequestReviewEvent::RequestChanges,
        "  Needs tests  ",
      )
      .expect("submit pending review");

    handle.join().expect("join server thread");
    let request = request
      .lock()
      .expect("lock request")
      .clone()
      .unwrap_or_default();
    assert!(
      request.contains("POST /github/pr/42/pending-review/submit?org=acme&repo=widget "),
      "request: {request}"
    );
    assert!(request.contains("\"pullRequestReviewId\":\"PRR_kwDOPending\""));
    assert!(request.contains("\"event\":\"REQUEST_CHANGES\""));
    assert!(request.contains("\"body\":\"Needs tests\""));
  }

  #[test]
  fn submit_pending_review_omits_the_empty_body_when_approving() {
    let (base_url, request, handle) =
      start_single_response_server_with_request("200 OK", PENDING_REVIEW_BODY);
    let api = make_test_api_client(base_url);

    let _ = api
      .submit_pending_review(
        "acme",
        "widget",
        42,
        "PRR_kwDOPending",
        GithubPullRequestReviewEvent::Approve,
        "   ",
      )
      .expect("submit pending review");

    handle.join().expect("join server thread");
    let request = request
      .lock()
      .expect("lock request")
      .clone()
      .unwrap_or_default();
    assert!(request.contains("\"event\":\"APPROVE\""));
    assert!(!request.contains("\"body\""), "request: {request}");
  }

  #[test]
  fn submit_pending_review_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 UNAUTHORIZED", "{}");
    let api = make_test_api_client(base_url);

    let err = api
      .submit_pending_review(
        "acme",
        "widget",
        42,
        "PRR_x",
        GithubPullRequestReviewEvent::Approve,
        "",
      )
      .expect_err("submit pending review should fail");

    assert_eq!(err.to_string(), "unauthorized");
    handle.join().expect("join server thread");
  }

  const RESOLVED_THREAD_BODY: &str = r#"{
    "thread": {
      "thread_id": "PRRT_kwDOThread",
      "is_resolved": true,
      "viewer_can_resolve": false,
      "viewer_can_unresolve": true
    }
  }"#;

  #[test]
  fn resolve_review_thread_posts_the_resolve_route() {
    let (base_url, request_line, handle) =
      start_single_response_server_with_request_line("200 OK", RESOLVED_THREAD_BODY);
    let api = make_test_api_client(base_url);

    api
      .resolve_pull_request_review_thread("acme", "widget", 42, "PRRT_kwDOThread")
      .expect("resolve review thread");

    handle.join().expect("join server thread");
    let request_line = request_line
      .lock()
      .expect("lock request line")
      .clone()
      .unwrap_or_default();
    assert_eq!(
      request_line,
      "POST /github/pr/42/review-threads/PRRT_kwDOThread/resolve?org=acme&repo=widget HTTP/1.1"
    );
  }

  #[test]
  fn unresolve_review_thread_posts_the_unresolve_route() {
    let body = r#"{
      "thread": {
        "thread_id": "PRRT_kwDOThread",
        "is_resolved": false,
        "viewer_can_resolve": true,
        "viewer_can_unresolve": false
      }
    }"#;
    let (base_url, request_line, handle) =
      start_single_response_server_with_request_line("200 OK", body);
    let api = make_test_api_client(base_url);

    api
      .unresolve_pull_request_review_thread("acme", "widget", 42, "PRRT_kwDOThread")
      .expect("unresolve review thread");

    handle.join().expect("join server thread");
    let request_line = request_line
      .lock()
      .expect("lock request line")
      .clone()
      .unwrap_or_default();
    assert_eq!(
      request_line,
      "POST /github/pr/42/review-threads/PRRT_kwDOThread/unresolve?org=acme&repo=widget HTTP/1.1"
    );
  }

  #[test]
  fn mark_notification_read_patches_the_thread() {
    let (base_url, request_line, handle) =
      start_single_response_server_with_request_line("200 OK", "{}");
    let api = make_test_api_client(base_url);

    api
      .mark_notification_read("thread-1")
      .expect("mark notification read");

    handle.join().expect("join server thread");
    let request_line = request_line
      .lock()
      .expect("lock request line")
      .clone()
      .unwrap_or_default();
    assert_eq!(
      request_line,
      "PATCH /github/notifications/thread-1/read HTTP/1.1"
    );
  }

  #[test]
  fn mark_notification_done_deletes_the_thread() {
    let (base_url, request_line, handle) =
      start_single_response_server_with_request_line("200 OK", "{}");
    let api = make_test_api_client(base_url);

    api
      .mark_notification_done("thread-1")
      .expect("mark notification done");

    handle.join().expect("join server thread");
    let request_line = request_line
      .lock()
      .expect("lock request line")
      .clone()
      .unwrap_or_default();
    assert_eq!(
      request_line,
      "DELETE /github/notifications/thread-1/done HTTP/1.1"
    );
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
  fn fetch_github_repository_details_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "");
    let api = make_test_api_client(base_url);

    let err = api.fetch_github_repository_details("acme", "widget").err();
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
      .expect_err("error");
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
  fn fetch_pull_request_details_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "");
    let api = make_test_api_client(base_url);

    let err = api.fetch_pull_request_details("acme", "widget", 42).err();
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
