use std::sync::{Arc, Mutex};

use gpui::{App, Global};

use crate::{api::User, sentry_context};

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

  pub fn has_active_subscription(cx: &App) -> bool {
    matches!(
      Self::get(cx),
      AuthState::Authenticated(user) if user.subscription.active_subscription.is_some()
    )
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
