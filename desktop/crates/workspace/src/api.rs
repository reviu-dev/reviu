use anyhow::Result;
use reqwest::StatusCode;
use reqwest::blocking::Client;
use serde::Deserialize;

const DEFAULT_API_BASE_URL: &str = "http://localhost:3000";

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
    }
  }

  fn get_api_url(&self, path: &str) -> String {
    format!("{}/{}", self.base_url, path.trim_start_matches('/'))
  }

  pub fn fetch_me(&self) -> Result<Option<User>> {
    let url = self.get_api_url("/users/me");
    let response = self.client.get(url).send()?;
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
