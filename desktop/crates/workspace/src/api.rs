use anyhow::Result;
use reqwest::{Method, StatusCode};
use reqwest::blocking::Client;
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

  pub fn authed_request(
    &self,
    method: Method,
    path: &str,
  ) -> reqwest::blocking::RequestBuilder {
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
    let response = self
      .authed_request(Method::GET, "/users/me")
      .send()?;
    if response.status() == StatusCode::UNAUTHORIZED {
      return Ok(None);
    }
    if !response.status().is_success() {
      anyhow::bail!("unexpected status: {}", response.status());
    }
    let user = response.json::<User>()?;
    Ok(Some(user))
  }
}
