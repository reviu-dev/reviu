use crate::error::{Error, Result};
use crate::state::User;
use reqwest::{Client, header};
use serde::{Deserialize, Serialize};

/// API client for communicating with the Reviu backend
pub struct ApiClient {
  client: Client,
  base_url: String,
  token: Option<String>,
}

impl ApiClient {
  /// Create a new API client
  pub fn new(base_url: impl Into<String>) -> Result<Self> {
    let client = Client::builder()
      .user_agent("Reviu Desktop/0.1.0")
      .timeout(std::time::Duration::from_secs(30))
      .build()?;

    Ok(Self {
      client,
      base_url: base_url.into(),
      token: None,
    })
  }

  /// Set the authentication token
  pub fn set_token(&mut self, token: impl Into<String>) {
    self.token = Some(token.into());
  }

  /// Clear the authentication token
  pub fn clear_token(&mut self) {
    self.token = None;
  }

  /// Build request headers with authentication
  fn build_headers(&self) -> header::HeaderMap {
    let mut headers = header::HeaderMap::new();
    headers.insert(
      header::CONTENT_TYPE,
      header::HeaderValue::from_static("application/json"),
    );

    if let Some(token) = &self.token {
      if let Ok(auth_value) = header::HeaderValue::from_str(&format!("Bearer {}", token)) {
        headers.insert(header::AUTHORIZATION, auth_value);
      }
    }

    headers
  }

  /// Get current user information
  pub async fn get_me(&self) -> Result<MeResponse> {
    let url = format!("{}/api/auth/me", self.base_url);
    let response = self
      .client
      .get(&url)
      .headers(self.build_headers())
      .send()
      .await?;

    if !response.status().is_success() {
      return Err(Error::Unauthorized(format!(
        "Failed to get user info: {}",
        response.status()
      )));
    }

    let me_response: MeResponse = response.json().await?;
    Ok(me_response)
  }

  /// Check authentication status
  pub async fn check_auth(&self) -> Result<bool> {
    match self.get_me().await {
      Ok(_) => Ok(true),
      Err(Error::Unauthorized(_)) => Ok(false),
      Err(e) => Err(e),
    }
  }

  /// Refresh premium status
  pub async fn refresh_premium_status(&self) -> Result<bool> {
    let me = self.get_me().await?;
    Ok(me.premium)
  }

  /// Sign out (revoke token)
  pub async fn sign_out(&self) -> Result<()> {
    let url = format!("{}/api/auth/sign-out", self.base_url);
    let response = self
      .client
      .post(&url)
      .headers(self.build_headers())
      .send()
      .await?;

    if !response.status().is_success() {
      return Err(Error::Network(format!(
        "Failed to sign out: {}",
        response.status()
      )));
    }

    Ok(())
  }

  /// Health check
  pub async fn health_check(&self) -> Result<()> {
    let url = format!("{}/api/health", self.base_url);
    let response = self.client.get(&url).send().await?;

    if !response.status().is_success() {
      return Err(Error::Network(format!(
        "Health check failed: {}",
        response.status()
      )));
    }

    Ok(())
  }
}

/// Response from /api/auth/me endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeResponse {
  pub id: String,
  pub email: String,
  pub name: Option<String>,
  pub premium: bool,
  pub subscription: Option<Subscription>,
}

impl From<MeResponse> for User {
  fn from(me: MeResponse) -> Self {
    Self {
      id: me.id,
      email: me.email,
      name: me.name,
      avatar_url: None,
      premium: me.premium,
    }
  }
}

/// Subscription information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
  pub status: SubscriptionStatus,
  pub plan: String,
  #[serde(rename = "currentPeriodEnd")]
  pub current_period_end: i64,
  #[serde(rename = "cancelAtPeriodEnd")]
  pub cancel_at_period_end: bool,
}

/// Subscription status
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionStatus {
  Active,
  Canceled,
  Incomplete,
  IncompleteExpired,
  PastDue,
  Trialing,
  Unpaid,
}

/// Error response from the API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
  pub error: ApiError,
}

/// API error details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
  pub code: String,
  pub message: String,
}

/// Error codes from the API
pub mod error_codes {
  pub const UNAUTHORIZED: &str = "UNAUTHORIZED";
  pub const FORBIDDEN: &str = "FORBIDDEN";
  pub const NOT_FOUND: &str = "NOT_FOUND";
  pub const INVALID_REQUEST: &str = "INVALID_REQUEST";
  pub const RATE_LIMITED: &str = "RATE_LIMITED";
  pub const INTERNAL_ERROR: &str = "INTERNAL_ERROR";
  pub const GITHUB_ERROR: &str = "GITHUB_ERROR";
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_api_client_creation() {
    let client = ApiClient::new("http://localhost:3000").unwrap();
    assert_eq!(client.base_url, "http://localhost:3000");
    assert!(client.token.is_none());
  }

  #[test]
  fn test_set_token() {
    let mut client = ApiClient::new("http://localhost:3000").unwrap();
    client.set_token("test_token");
    assert_eq!(client.token, Some("test_token".to_string()));
  }

  #[test]
  fn test_clear_token() {
    let mut client = ApiClient::new("http://localhost:3000").unwrap();
    client.set_token("test_token");
    client.clear_token();
    assert!(client.token.is_none());
  }
}
