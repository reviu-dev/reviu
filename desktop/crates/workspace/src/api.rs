use anyhow::Result;
use reqwest::blocking::Client;
use reqwest::{Method, StatusCode};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

const DEFAULT_API_BASE_URL: &str = "http://localhost:3000";
const KEYCHAIN_SERVICE: &str = "reviu_auth";
const KEYCHAIN_USERNAME: &str = "bearer";

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubNotificationRepositoryOwner {
  pub login: String,
  #[serde(rename = "avatarUrl")]
  pub avatar_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubNotificationRepository {
  pub name: String,
  #[serde(rename = "fullName")]
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
  #[serde(rename = "latestCommentUrl")]
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
  #[serde(rename = "updatedAt")]
  pub updated_at: String,
  #[serde(rename = "lastReadAt")]
  pub last_read_at: Option<String>,
  pub url: String,
  #[serde(rename = "subscriptionUrl")]
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

#[derive(Clone, Debug, Deserialize)]
pub struct GithubPullRequestLabel {
  pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GithubRepository {
  pub owner: String,
  pub repo: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GithubPullRequest {
  pub number: u64,
  pub title: String,
  pub state: GithubPullRequestState,
  #[serde(rename = "mergedAt")]
  pub merged_at: Option<String>,
  pub draft: bool,
  #[serde(rename = "updatedAt")]
  pub updated_at: String,
  pub labels: Vec<GithubPullRequestLabel>,
  pub repository: GithubRepository,
}

impl GithubPullRequest {
  pub fn status(&self) -> GithubPullRequestStatus {
    if self.merged_at.is_some() {
      GithubPullRequestStatus::Merged
    } else if self.draft {
      GithubPullRequestStatus::Draft
    } else {
      self.state.as_status()
    }
  }
}

#[derive(Clone, Debug, Deserialize)]
pub struct GithubPullRequestAuthor {
  pub login: String,
  #[serde(rename = "avatarUrl")]
  pub avatar_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GithubPullRequestReviewCommentUser {
  pub login: String,
  #[serde(rename = "avatarUrl")]
  pub avatar_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct GithubPullRequestReviewComment {
  pub id: u64,
  #[serde(rename = "pullRequestReviewId")]
  pub pull_request_review_id: Option<u64>,
  #[serde(rename = "diffHunk")]
  pub diff_hunk: String,
  pub path: String,
  pub position: Option<i64>,
  #[serde(rename = "originalPosition")]
  pub original_position: Option<i64>,
  #[serde(rename = "commitId")]
  pub commit_id: String,
  #[serde(rename = "originalCommitId")]
  pub original_commit_id: String,
  #[serde(rename = "inReplyToId")]
  pub in_reply_to_id: Option<u64>,
  pub user: GithubPullRequestReviewCommentUser,
  pub body: String,
  #[serde(rename = "createdAt")]
  pub created_at: String,
  #[serde(rename = "updatedAt")]
  pub updated_at: String,
  #[serde(rename = "startLine")]
  pub start_line: Option<i64>,
  #[serde(rename = "originalStartLine")]
  pub original_start_line: Option<i64>,
  #[serde(rename = "startSide")]
  pub start_side: Option<String>,
  pub line: Option<i64>,
  #[serde(rename = "originalLine")]
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

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GithubPullRequestState {
  Open,
  Closed,
}

impl GithubPullRequestState {
  pub fn as_status(&self) -> GithubPullRequestStatus {
    match self {
      GithubPullRequestState::Open => GithubPullRequestStatus::Open,
      GithubPullRequestState::Closed => GithubPullRequestStatus::Closed,
    }
  }
}

pub enum GithubPullRequestStatus {
  Open,
  Closed,
  Merged,
  Draft,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GithubPullRequestDetails {
  pub number: u64,
  pub title: String,
  pub state: GithubPullRequestState,
  pub draft: bool,
  #[serde(rename = "createdAt")]
  pub created_at: String,
  #[serde(rename = "updatedAt")]
  pub updated_at: String,
  #[serde(rename = "mergedAt")]
  pub merged_at: Option<String>,
  #[serde(rename = "mergeBaseSha")]
  pub merge_base_sha: String,
  #[serde(rename = "baseSha")]
  pub base_sha: String,
  #[serde(rename = "headSha")]
  pub head_sha: String,
  #[serde(rename = "baseRefName")]
  pub base_ref_name: String,
  #[serde(rename = "headRefName")]
  pub head_ref_name: String,
  pub body: Option<String>,
  pub author: GithubPullRequestAuthor,
  pub comments: u64,
  #[serde(rename = "reviewComments")]
  pub review_comments: u64,
  pub commits: u64,
  pub additions: u64,
  pub deletions: u64,
  #[serde(rename = "changedFiles")]
  pub changed_files: u64,
  pub labels: Vec<GithubPullRequestLabel>,
  pub repository: GithubRepository,
  #[serde(rename = "headRepository")]
  pub head_repository: Option<GithubRepository>,
}

impl GithubPullRequestDetails {
  pub fn status(&self) -> GithubPullRequestStatus {
    if self.merged_at.is_some() {
      GithubPullRequestStatus::Merged
    } else if self.draft {
      GithubPullRequestStatus::Draft
    } else {
      self.state.as_status()
    }
  }
}

#[derive(Clone)]
pub struct ApiClient {
  base_url: String,
  client: Client,
  bearer_token: Arc<Mutex<Option<String>>>,
}

#[derive(Debug, Serialize)]
struct SocialSignInRequest<'a> {
  provider: &'a str,
  #[serde(rename = "disableRedirect")]
  disable_redirect: bool,
  #[serde(rename = "callbackURL")]
  callback_url: &'a str,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct SocialSignInResponse {
  #[serde(default)]
  url: Option<String>,
  #[serde(default)]
  redirect: Option<bool>,
  #[serde(default)]
  token: Option<String>,
  #[serde(default)]
  user: Option<User>,
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

#[derive(Debug, Deserialize)]
struct GithubPullRequestsResponse {
  #[serde(rename = "pullRequests")]
  pull_requests: Vec<GithubPullRequest>,
}

#[derive(Debug, Deserialize)]
struct GithubPullRequestDetailsResponse {
  #[serde(rename = "pullRequest")]
  pull_request: GithubPullRequestDetails,
}

#[derive(Debug, Deserialize)]
struct GithubPullRequestFilesResponse {
  files: Vec<GithubPullRequestFile>,
}

#[derive(Debug, Deserialize)]
struct GithubPullRequestCommentsResponse {
  comments: Vec<GithubPullRequestReviewComment>,
}

#[derive(Debug, Deserialize)]
struct GithubPullRequestCommentResponse {
  comment: GithubPullRequestReviewComment,
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

#[derive(Debug, Deserialize)]
struct GithubFileContentResponse {
  content: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct DesktopUpdateCheckResponse {
  #[serde(rename = "updateAvailable")]
  pub update_available: bool,
  #[serde(rename = "currentVersion")]
  pub current_version: String,
  #[serde(rename = "latestVersion")]
  pub latest_version: String,
  #[serde(rename = "downloadUrl")]
  pub download_url: String,
}

#[derive(Debug, Serialize)]
struct DesktopUpdateCheckRequest<'a> {
  #[serde(rename = "currentVersion")]
  current_version: &'a str,
}

impl ApiClient {
  pub fn new() -> Self {
    let base_url =
      std::env::var("API_BASE_URL").unwrap_or_else(|_| DEFAULT_API_BASE_URL.to_string());
    Self::new_with_base_url(base_url)
  }

  pub(crate) fn new_with_base_url(base_url: impl Into<String>) -> Self {
    let base_url = base_url.into();
    let client = Client::builder()
      .cookie_store(true)
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

  pub fn keychain_service(&self) -> &str {
    KEYCHAIN_SERVICE
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

  pub fn sign_in_with_github(&self) -> Result<Option<String>> {
    let request = SocialSignInRequest {
      provider: "github",
      disable_redirect: true,
      callback_url: "/auth/callback",
    };
    let url = self.get_api_url("/api/auth/sign-in/social");
    let response = self.client.post(url).json(&request).send()?;
    if !response.status().is_success() {
      anyhow::bail!("unexpected status: {}", response.status());
    }
    let payload = response.json::<SocialSignInResponse>()?;
    Ok(payload.url)
  }

  pub fn exchange_code_for_token(&self, code: &str) -> Result<String> {
    let url = self.get_api_url("/auth/exchange");
    let response = self
      .client
      .post(url)
      .json(&ExchangeCodeRequest { code })
      .send()?;
    if !response.status().is_success() {
      anyhow::bail!("unexpected status: {}", response.status());
    }
    let payload = response.json::<ExchangeCodeResponse>()?;
    Ok(payload.token)
  }

  pub fn sign_out(&self) -> Result<()> {
    let response = self
      .authed_request(Method::POST, "/api/auth/sign-out")
      .json(&EmptyRequest {})
      .send()?;
    if !response.status().is_success() {
      self.clear_bearer_token();
      anyhow::bail!("unexpected status: {}", response.status());
    }
    self.clear_bearer_token();
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
    if response.status() == StatusCode::UNAUTHORIZED {
      return Ok(None);
    }
    if !response.status().is_success() {
      anyhow::bail!("unexpected status: {}", response.status());
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
    if response.status() == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !response.status().is_success() {
      anyhow::bail!("unexpected status: {}", response.status());
    }
    let payload = response.json::<CheckoutSubscriptionResponse>()?;
    if payload.url.trim().is_empty() {
      anyhow::bail!("missing checkout url");
    }
    Ok(payload.url)
  }

  pub fn fetch_latest_pull_requests(
    &self,
    owner: &str,
    repo: &str,
  ) -> Result<Vec<GithubPullRequest>> {
    let response = self
      .authed_request(Method::GET, "/github/pr/latest")
      .query(&[("org", owner), ("repo", repo)])
      .send()?;
    if response.status() == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !response.status().is_success() {
      anyhow::bail!("unexpected status: {}", response.status());
    }
    let payload = response.json::<GithubPullRequestsResponse>()?;
    Ok(payload.pull_requests)
  }

  pub fn fetch_pull_request_details(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
  ) -> Result<GithubPullRequestDetails> {
    let response = self
      .authed_request(Method::GET, &format!("/github/pr/{number}"))
      .query(&[("org", owner), ("repo", repo)])
      .send()?;
    if response.status() == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !response.status().is_success() {
      anyhow::bail!("unexpected status: {}", response.status());
    }
    let payload = response.json::<GithubPullRequestDetailsResponse>()?;
    Ok(payload.pull_request)
  }

  pub fn fetch_pull_request_files(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
  ) -> Result<Vec<GithubPullRequestFile>> {
    let response = self
      .authed_request(Method::GET, &format!("/github/pr/{number}/files"))
      .query(&[("org", owner), ("repo", repo)])
      .send()?;
    if response.status() == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !response.status().is_success() {
      anyhow::bail!("unexpected status: {}", response.status());
    }
    let payload = response.json::<GithubPullRequestFilesResponse>()?;
    Ok(payload.files)
  }

  pub fn fetch_github_notifications(&self) -> Result<Vec<GithubNotification>> {
    let response = self
      .authed_request(Method::GET, "/github/notifications")
      .send()?;
    if response.status() == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !response.status().is_success() {
      anyhow::bail!("unexpected status: {}", response.status());
    }
    let payload = response.json::<GithubNotificationsResponse>()?;
    Ok(payload.notifications)
  }

  pub fn fetch_pull_request_review_comments(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
  ) -> Result<Vec<GithubPullRequestReviewComment>> {
    let response = self
      .authed_request(Method::GET, &format!("/github/pr/{number}/comments"))
      .query(&[("org", owner), ("repo", repo)])
      .send()?;
    if response.status() == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !response.status().is_success() {
      anyhow::bail!("unexpected status: {}", response.status());
    }
    let payload = response.json::<GithubPullRequestCommentsResponse>()?;
    Ok(payload.comments)
  }

  pub fn update_pull_request_review_comment(
    &self,
    owner: &str,
    repo: &str,
    number: u64,
    comment_id: u64,
    body: &str,
  ) -> Result<GithubPullRequestReviewComment> {
    let response = self
      .authed_request(
        Method::PATCH,
        &format!("/github/pr/{number}/comments/{comment_id}"),
      )
      .query(&[("org", owner), ("repo", repo)])
      .json(&UpdateGithubPullRequestCommentRequest { body })
      .send()?;
    if response.status() == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !response.status().is_success() {
      anyhow::bail!("unexpected status: {}", response.status());
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
    let response = self
      .authed_request(Method::POST, &format!("/github/pr/{number}/comments"))
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
    if response.status() == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !response.status().is_success() {
      anyhow::bail!("unexpected status: {}", response.status());
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
    let response = self
      .authed_request(
        Method::POST,
        &format!("/github/pr/{number}/comments/{comment_id}/replies"),
      )
      .query(&[("org", owner), ("repo", repo)])
      .json(&ReplyGithubPullRequestCommentRequest { body })
      .send()?;
    if response.status() == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !response.status().is_success() {
      anyhow::bail!("unexpected status: {}", response.status());
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
    let response = self
      .authed_request(
        Method::DELETE,
        &format!("/github/pr/{number}/comments/{comment_id}"),
      )
      .query(&[("org", owner), ("repo", repo)])
      .send()?;
    if response.status() == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !response.status().is_success() {
      anyhow::bail!("unexpected status: {}", response.status());
    }
    Ok(())
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
    if response.status() == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !response.status().is_success() {
      anyhow::bail!("unexpected status: {}", response.status());
    }
    let payload = response.json::<GithubFileContentResponse>()?;
    Ok(payload.content)
  }

  pub fn check_desktop_update(&self, current_version: &str) -> Result<DesktopUpdateCheckResponse> {
    let response = self
      .client
      .post(self.get_api_url("/desktop/update/check"))
      .json(&DesktopUpdateCheckRequest { current_version })
      .send()?;
    if !response.status().is_success() {
      anyhow::bail!("unexpected status: {}", response.status());
    }

    let payload = response.json::<DesktopUpdateCheckResponse>()?;
    Ok(payload)
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
      merged_at: merged_at.map(str::to_string),
      draft,
      updated_at: "2026-02-15T12:00:00Z".to_string(),
      labels: vec![GithubPullRequestLabel {
        name: "bug".to_string(),
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
      },
      comments: 0,
      review_comments: 0,
      commits: 1,
      additions: 1,
      deletions: 0,
      changed_files: 1,
      labels: vec![GithubPullRequestLabel {
        name: "bug".to_string(),
      }],
      repository: GithubRepository {
        owner: "acme".to_string(),
        repo: "widget".to_string(),
      },
      head_repository: None,
    }
  }

  #[test]
  fn pull_request_status_prioritizes_merged_then_draft_then_state() {
    let merged = make_pull_request(
      GithubPullRequestState::Open,
      true,
      Some("2026-02-15T12:00:00Z"),
    );
    assert!(matches!(merged.status(), GithubPullRequestStatus::Merged));

    let draft = make_pull_request(GithubPullRequestState::Closed, true, None);
    assert!(matches!(draft.status(), GithubPullRequestStatus::Draft));

    let closed = make_pull_request(GithubPullRequestState::Closed, false, None);
    assert!(matches!(closed.status(), GithubPullRequestStatus::Closed));
  }

  #[test]
  fn pull_request_details_status_prioritizes_merged_then_draft_then_state() {
    let merged = make_pull_request_details(
      GithubPullRequestState::Open,
      true,
      Some("2026-02-15T12:00:00Z"),
    );
    assert!(matches!(merged.status(), GithubPullRequestStatus::Merged));

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
  fn fetch_latest_pull_requests_parses_success_payload() {
    let body = r#"{
      "pullRequests": [
        {
          "number": 7,
          "title": "Fix login issue",
          "state": "open",
          "mergedAt": null,
          "draft": false,
          "updatedAt": "2026-02-15T12:00:00Z",
          "labels": [{ "name": "bug" }],
          "repository": { "owner": "acme", "repo": "widget" }
        }
      ]
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let prs = api
      .fetch_latest_pull_requests("acme", "widget")
      .expect("fetch pull requests");
    assert_eq!(prs.len(), 1);
    assert_eq!(prs[0].number, 7);
    assert_eq!(prs[0].title, "Fix login issue");
    assert!(matches!(prs[0].state, GithubPullRequestState::Open));
    assert_eq!(prs[0].repository.owner, "acme");
    assert_eq!(prs[0].repository.repo, "widget");
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_pull_request_details_parses_success_payload() {
    let body = r#"{
      "pullRequest": {
        "number": 42,
        "title": "Improve parser",
        "state": "open",
        "draft": false,
        "createdAt": "2026-02-10T09:00:00Z",
        "updatedAt": "2026-02-15T12:00:00Z",
        "mergedAt": null,
        "mergeBaseSha": "abc123",
        "baseSha": "base123",
        "headSha": "head123",
        "baseRefName": "main",
        "headRefName": "feature/parser",
        "body": "PR body",
        "author": { "login": "octocat", "avatarUrl": null },
        "comments": 2,
        "reviewComments": 3,
        "commits": 4,
        "additions": 10,
        "deletions": 5,
        "changedFiles": 2,
        "labels": [{ "name": "enhancement" }],
        "repository": { "owner": "acme", "repo": "widget" },
        "headRepository": { "owner": "acme", "repo": "widget-fork" }
      }
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let details = api
      .fetch_pull_request_details("acme", "widget", 42)
      .expect("fetch pull request details");
    assert_eq!(details.number, 42);
    assert_eq!(details.head_ref_name, "feature/parser");
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
      .fetch_pull_request_files("acme", "widget", 42)
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
  fn fetch_pull_request_review_comments_parses_success_payload() {
    let body = r#"{
      "comments": [
        {
          "id": 1,
          "pullRequestReviewId": 12,
          "diffHunk": "@@ -1 +1 @@",
          "path": "src/main.rs",
          "position": 1,
          "originalPosition": 1,
          "commitId": "head123",
          "originalCommitId": "base123",
          "inReplyToId": null,
          "user": { "login": "octocat", "avatarUrl": null },
          "body": "Looks good",
          "createdAt": "2026-02-15T12:00:00Z",
          "updatedAt": "2026-02-15T12:01:00Z",
          "startLine": null,
          "originalStartLine": null,
          "startSide": null,
          "line": 1,
          "originalLine": 1,
          "side": "RIGHT"
        }
      ]
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let comments = api
      .fetch_pull_request_review_comments("acme", "widget", 42)
      .expect("fetch review comments");
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].id, 1);
    assert_eq!(comments[0].path, "src/main.rs");
    assert_eq!(comments[0].user.login, "octocat");
    assert_eq!(comments[0].side.as_deref(), Some("RIGHT"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn update_pull_request_review_comment_parses_success_payload() {
    let body = r#"{
      "comment": {
        "id": 1,
        "pullRequestReviewId": 12,
        "diffHunk": "@@ -1 +1 @@",
        "path": "src/main.rs",
        "position": 1,
        "originalPosition": 1,
        "commitId": "head123",
        "originalCommitId": "base123",
        "inReplyToId": null,
        "user": { "login": "octocat", "avatarUrl": null },
        "body": "Updated body",
        "createdAt": "2026-02-15T12:00:00Z",
        "updatedAt": "2026-02-16T12:01:00Z",
        "startLine": null,
        "originalStartLine": null,
        "startSide": null,
        "line": 1,
        "originalLine": 1,
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
        "pullRequestReviewId": 12,
        "diffHunk": "@@ -1 +1 @@",
        "path": "src/main.rs",
        "position": 1,
        "originalPosition": 1,
        "commitId": "head123",
        "originalCommitId": "base123",
        "inReplyToId": null,
        "user": { "login": "octocat", "avatarUrl": null },
        "body": "New comment body",
        "createdAt": "2026-02-15T12:00:00Z",
        "updatedAt": "2026-02-16T12:01:00Z",
        "startLine": null,
        "originalStartLine": null,
        "startSide": null,
        "line": 1,
        "originalLine": 1,
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
        "pullRequestReviewId": 12,
        "diffHunk": "@@ -1 +1 @@",
        "path": "src/main.rs",
        "position": 1,
        "originalPosition": 1,
        "commitId": "head123",
        "originalCommitId": "base123",
        "inReplyToId": 2,
        "user": { "login": "octocat", "avatarUrl": null },
        "body": "Reply body",
        "createdAt": "2026-02-15T12:00:00Z",
        "updatedAt": "2026-02-16T12:01:00Z",
        "startLine": null,
        "originalStartLine": null,
        "startSide": null,
        "line": 1,
        "originalLine": 1,
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
        "pullRequestReviewId": 12,
        "diffHunk": "@@ -1 +1 @@",
        "path": "src/main.rs",
        "position": 1,
        "originalPosition": 1,
        "commitId": "head123",
        "originalCommitId": "base123",
        "inReplyToId": 2,
        "user": { "login": "octocat", "avatarUrl": null },
        "body": "Reply body",
        "createdAt": "2026-02-15T12:00:00Z",
        "updatedAt": "2026-02-16T12:01:00Z",
        "startLine": null,
        "originalStartLine": null,
        "startSide": null,
        "line": 1,
        "originalLine": 1,
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
  fn fetch_github_notifications_parses_success_payload() {
    let body = r#"{
      "notifications": [
        {
          "id": "1",
          "repository": {
            "name": "widget",
            "fullName": "acme/widget",
            "owner": {
              "login": "acme",
              "avatarUrl": "https://example.com/avatar.png"
            }
          },
          "subject": {
            "title": "Review requested",
            "type": "PullRequest",
            "url": "https://api.github.test/subject/1",
            "latestCommentUrl": null
          },
          "reason": "review_requested",
          "unread": true,
          "updatedAt": "2026-02-15T12:00:00Z",
          "lastReadAt": null,
          "url": "https://api.github.test/notif/1",
          "subscriptionUrl": "https://api.github.test/sub/1"
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

    handle.join().expect("join server thread");
  }

  #[test]
  fn checkout_subscription_returns_checkout_url_on_success() {
    let body = r#"{"url":"https://polar.sh/checkout/session_123","redirect":false}"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let url = api
      .checkout_subscription("pro")
      .expect("checkout subscription should return a URL");

    assert_eq!(url, "https://polar.sh/checkout/session_123");
    handle.join().expect("join server thread");
  }

  #[test]
  fn checkout_subscription_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "{}");
    let api = make_test_api_client(base_url);

    let err = api.checkout_subscription("pro").err();

    assert!(err.is_some());
    assert!(err.expect("error").to_string().contains("unauthorized"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn checkout_subscription_posts_expected_route_and_payload() {
    let body = r#"{"url":"https://polar.sh/checkout/session_123","redirect":false}"#;
    let (base_url, request, handle) = start_single_response_server_with_request("200 OK", body);
    let api = make_test_api_client(base_url);

    let _ = api.checkout_subscription("pro").expect("checkout request");

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
    assert!(request.contains("\"slug\":\"pro\""), "request: {request}");
    assert!(request.contains("\"redirect\":false"), "request: {request}");
  }

  #[test]
  fn fetch_latest_pull_requests_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "");
    let api = make_test_api_client(base_url);

    let err = api.fetch_latest_pull_requests("acme", "widget").err();
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
  fn fetch_pull_request_files_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "");
    let api = make_test_api_client(base_url);

    let err = api.fetch_pull_request_files("acme", "widget", 42).err();
    assert!(err.is_some());
    assert!(err.expect("error").to_string().contains("unauthorized"));
    handle.join().expect("join server thread");
  }

  #[test]
  fn fetch_pull_request_review_comments_returns_unauthorized_error() {
    let (base_url, handle) = start_single_response_server("401 Unauthorized", "");
    let api = make_test_api_client(base_url);

    let err = api
      .fetch_pull_request_review_comments("acme", "widget", 42)
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
      "currentVersion": "0.1.0",
      "latestVersion": "0.2.0",
      "downloadUrl": "https://reviu.dev/downloads/latest"
    }"#;
    let (base_url, handle) = start_single_response_server("200 OK", body);
    let api = make_test_api_client(base_url);

    let payload = api
      .check_desktop_update("0.1.0")
      .expect("check desktop update");
    assert!(payload.update_available);
    assert_eq!(payload.current_version, "0.1.0");
    assert_eq!(payload.latest_version, "0.2.0");
    assert_eq!(payload.download_url, "https://reviu.dev/downloads/latest");
    handle.join().expect("join server thread");
  }

  #[test]
  fn check_desktop_update_posts_expected_route_and_payload() {
    let body = r#"{
      "updateAvailable": false,
      "currentVersion": "0.1.0",
      "latestVersion": "0.1.0",
      "downloadUrl": "https://reviu.dev/downloads/latest"
    }"#;
    let (base_url, request, handle) = start_single_response_server_with_request("200 OK", body);
    let api = make_test_api_client(base_url);

    let _ = api
      .check_desktop_update("0.1.0")
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
  }

  #[test]
  fn check_desktop_update_returns_error_on_non_success_status() {
    let (base_url, handle) = start_single_response_server("502 Bad Gateway", "{}");
    let api = make_test_api_client(base_url);

    let err = api.check_desktop_update("0.1.0").err();
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
