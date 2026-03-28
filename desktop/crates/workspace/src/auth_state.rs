use std::sync::{Arc, Mutex};

use gpui::{App, Global};

use crate::{api::User, sentry_context};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GithubAccessState {
  NeedsSignIn,
  NeedsSubscription,
  Available,
}

#[derive(Clone, Debug)]
pub enum AuthState {
  Unknown,
  Authenticated(Box<User>),
  Unauthenticated,
}

#[derive(Clone)]
pub struct AuthStateStore {
  state: Arc<Mutex<AuthState>>,
}

impl Global for AuthStateStore {}

impl AuthStateStore {
  pub fn get(cx: &App) -> AuthState {
    cx.global::<Self>()
      .state
      .lock()
      .map(|state| state.clone())
      .unwrap_or(AuthState::Unknown)
  }

  pub fn has_github_access(cx: &App) -> bool {
    Self::get(cx).has_github_access()
  }

  pub fn github_access_state(cx: &App) -> GithubAccessState {
    Self::get(cx).github_access_state()
  }

  pub fn should_show_billing_entry(cx: &App) -> bool {
    Self::get(cx).should_show_billing_entry()
  }

  pub fn set(cx: &mut App, state: AuthState) {
    sentry_context::sync_auth_state(&state);
    if let Ok(mut guard) = cx.global::<Self>().state.lock() {
      *guard = state;
    }
  }
}

impl Default for AuthStateStore {
  fn default() -> Self {
    Self {
      state: Arc::new(Mutex::new(AuthState::Unknown)),
    }
  }
}

impl AuthState {
  pub fn has_pro_access(&self) -> bool {
    matches!(self, AuthState::Authenticated(user) if user.has_pro_access())
  }

  pub fn github_access_state(&self) -> GithubAccessState {
    match self {
      AuthState::Authenticated(_) if self.has_pro_access() => GithubAccessState::Available,
      AuthState::Authenticated(_) => GithubAccessState::NeedsSubscription,
      AuthState::Unknown | AuthState::Unauthenticated => GithubAccessState::NeedsSignIn,
    }
  }

  pub fn has_github_access(&self) -> bool {
    matches!(self.github_access_state(), GithubAccessState::Available)
  }

  pub fn should_show_billing_entry(&self) -> bool {
    matches!(self, AuthState::Authenticated(user) if user.should_show_billing_entry())
  }
}

#[cfg(test)]
mod tests {
  use super::{AuthState, GithubAccessState};
  use crate::api::{
    CustomerStateSubscription, CustomerStateSubscriptionStatus, User, UserRole, UserSubscription,
  };

  fn make_subscription() -> CustomerStateSubscription {
    CustomerStateSubscription {
      id: "sub_123".to_string(),
      created_at: "2026-01-01T00:00:00Z".to_string(),
      modified_at: None,
      status: CustomerStateSubscriptionStatus::Active,
      amount: 2_000,
      currency: "usd".to_string(),
      recurring_interval: "month".to_string(),
      current_period_start: "2026-01-01T00:00:00Z".to_string(),
      current_period_end: Some("2099-01-01T00:00:00Z".to_string()),
      trial_start: None,
      trial_end: None,
      cancel_at_period_end: false,
      canceled_at: None,
      started_at: Some("2026-01-01T00:00:00Z".to_string()),
      ends_at: None,
      product_id: "prod_123".to_string(),
    }
  }

  fn make_user(role: UserRole, active_subscription: Option<CustomerStateSubscription>) -> User {
    User {
      id: "user_123".to_string(),
      name: "Joris".to_string(),
      email: "joris@example.com".to_string(),
      email_verified: true,
      image: None,
      github_login: Some("joris-gallot".to_string()),
      role,
      subscription: UserSubscription {
        portal_url: None,
        active_subscription,
      },
    }
  }

  #[test]
  fn state_has_pro_access_allows_admin_without_subscription() {
    let state = AuthState::Authenticated(Box::new(make_user(UserRole::Admin, None)));

    assert!(state.has_pro_access());
    assert!(state.has_github_access());
    assert_eq!(state.github_access_state(), GithubAccessState::Available);
    assert!(state.should_show_billing_entry());
  }

  #[test]
  fn state_has_pro_access_allows_pro_role_without_subscription() {
    let state = AuthState::Authenticated(Box::new(make_user(UserRole::Pro, None)));

    assert!(state.has_pro_access());
    assert!(state.has_github_access());
    assert_eq!(state.github_access_state(), GithubAccessState::Available);
    assert!(!state.should_show_billing_entry());
  }

  #[test]
  fn state_has_pro_access_requires_subscription_for_regular_users() {
    let no_subscription = AuthState::Authenticated(Box::new(make_user(UserRole::User, None)));
    let with_subscription = AuthState::Authenticated(Box::new(make_user(
      UserRole::User,
      Some(make_subscription()),
    )));

    assert!(!no_subscription.has_pro_access());
    assert!(!no_subscription.has_github_access());
    assert_eq!(
      no_subscription.github_access_state(),
      GithubAccessState::NeedsSubscription
    );
    assert!(!no_subscription.should_show_billing_entry());
    assert!(with_subscription.has_pro_access());
    assert!(with_subscription.has_github_access());
    assert_eq!(
      with_subscription.github_access_state(),
      GithubAccessState::Available
    );
    assert!(with_subscription.should_show_billing_entry());
  }

  #[test]
  fn state_requires_sign_in_when_unauthenticated() {
    let state = AuthState::Unauthenticated;

    assert_eq!(state.github_access_state(), GithubAccessState::NeedsSignIn);
    assert!(!state.has_github_access());
    assert!(!state.should_show_billing_entry());
  }
}
