use anyhow::Result;
use reqwest::blocking::Client;
use reqwest::{Method, StatusCode};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

const DEFAULT_API_BASE_URL: &str = "http://localhost:3000";
const KEYCHAIN_SERVICE: &str = "reviu_auth";
const KEYCHAIN_USERNAME: &str = "bearer";

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserRole {
  User,
  Admin,
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
  pub role: UserRole,
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
  pub state: String,
  #[serde(rename = "mergedAt")]
  pub merged_at: Option<String>,
  pub draft: bool,
  #[serde(rename = "updatedAt")]
  pub updated_at: String,
  pub labels: Vec<GithubPullRequestLabel>,
  pub repository: GithubRepository,
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
pub struct GithubPullRequestDetails {
  pub number: u64,
  pub title: String,
  pub state: String,
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
struct GithubPullRequestDiffResponse {
  diff: String,
}

#[derive(Debug, Deserialize)]
struct GithubPullRequestCommentsResponse {
  comments: Vec<GithubPullRequestReviewComment>,
}

#[derive(Debug, Deserialize)]
struct GithubFileContentResponse {
  content: Option<String>,
}

impl ApiClient {
  pub fn new() -> Self {
    let base_url =
      std::env::var("API_BASE_URL").unwrap_or_else(|_| DEFAULT_API_BASE_URL.to_string());
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

  pub fn fetch_pull_request_diff(&self, owner: &str, repo: &str, number: u64) -> Result<String> {
    let response = self
      .authed_request(Method::GET, &format!("/github/pr/{number}/diff"))
      .query(&[("org", owner), ("repo", repo)])
      .send()?;
    if response.status() == StatusCode::UNAUTHORIZED {
      anyhow::bail!("unauthorized")
    }
    if !response.status().is_success() {
      anyhow::bail!("unexpected status: {}", response.status());
    }
    let payload = response.json::<GithubPullRequestDiffResponse>()?;
    Ok(payload.diff)
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
}
