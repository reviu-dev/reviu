//! Signing in and out of GitHub: the OAuth round trip, the keychain, and the
//! state every page reads afterwards.

use gpui::{App, AppContext as _, Global, Task};

use crate::analytics;
use crate::auth_state::{AuthState, AuthStateStore};
use crate::github_notifications::GithubNotificationsStore;
use crate::navigation::NavigationHistory;
use crate::workspace::WorkspaceApi;

/// Holds the task alive: dropping it would cancel a sign-in halfway through.
#[derive(Default)]
struct AuthFlow {
  task: Option<Task<()>>,
}

impl Global for AuthFlow {}

fn set_task(cx: &mut App, task: Task<()>) {
  if !cx.has_global::<AuthFlow>() {
    cx.set_global(AuthFlow::default());
  }
  cx.global_mut::<AuthFlow>().task = Some(task);
}

/// Reads the token stored on this machine, so a restart stays signed in.
pub fn load_stored_token(cx: &mut App) {
  let service = WorkspaceApi::global(cx).api.keychain_service().to_string();
  let task = cx.spawn(async move |cx| {
    let read_result = cx.update(|cx| cx.read_credentials(&service)).await;
    cx.update(|cx| {
      if let Ok(Some((_username, secret))) = read_result
        && let Ok(token) = String::from_utf8(secret)
      {
        WorkspaceApi::global(cx).api.set_bearer_token(token);
        refresh_me(cx);
        return;
      }
      set_auth_state(AuthState::Unauthenticated, cx);
    });
  });
  set_task(cx, task);
}

pub fn start_github_sign_in(cx: &mut App, source: &'static str) {
  analytics::track_with(
    cx,
    "sign_in_started",
    Some(serde_json::json!({ "source": source })),
  );
  let api = WorkspaceApi::global(cx).api.clone();
  let task = cx.spawn(async move |cx| {
    let result = cx
      .background_spawn(async move { api.sign_in_with_github() })
      .await;
    if let Ok(Some(url)) = result {
      cx.update(|cx| cx.open_url(&url));
    }
  });
  set_task(cx, task);
}

/// The other half of the OAuth round trip, handed over by the deep link.
pub fn handle_auth_code(code: String, cx: &mut App) {
  let api = WorkspaceApi::global(cx).api.clone();
  let service = api.keychain_service().to_string();
  let username = api.keychain_username().to_string();
  let task = cx.spawn(async move |cx| {
    let result = cx
      .background_spawn(async move { api.exchange_code_for_token(&code) })
      .await;
    match result {
      Ok(token) => {
        let secret = token.clone().into_bytes();
        let write_task = cx.update(|cx| cx.write_credentials(&service, &username, &secret));
        let _ = write_task.await;
        cx.update(|cx| {
          WorkspaceApi::global(cx).api.set_bearer_token(token);
          analytics::track(cx, "sign_in_completed");
          refresh_me(cx);
        });
      }
      Err(_) => {
        cx.update(|cx| set_auth_state(AuthState::Unauthenticated, cx));
      }
    }
  });
  set_task(cx, task);
}

pub fn sign_out(cx: &mut App) {
  let api = WorkspaceApi::global(cx).api.clone();
  let service = api.keychain_service().to_string();
  let task = cx.spawn(async move |cx| {
    let _ = cx.background_spawn(async move { api.sign_out() }).await;
    let delete_task = cx.update(|cx| cx.delete_credentials(&service));
    let _ = delete_task.await;
    cx.update(|cx| set_auth_state(AuthState::Unauthenticated, cx));
  });
  set_task(cx, task);
}

/// Asks the backend who we are: the answer is what gates Reviu Pro.
pub fn refresh_me(cx: &mut App) {
  let api = WorkspaceApi::global(cx).api.clone();
  let task = cx.spawn(async move |cx| {
    let result = cx.background_spawn(async move { api.fetch_me() }).await;
    cx.update(|cx| {
      let state = match result {
        Ok(Some(user)) => AuthState::Authenticated(Box::new(user)),
        Ok(None) | Err(_) => AuthState::Unauthenticated,
      };
      set_auth_state(state, cx);
    });
  });
  set_task(cx, task);
}

pub fn handle_subscription_callback(cx: &mut App) {
  analytics::track(cx, "subscription_callback_received");
  refresh_me(cx);
  NavigationHistory::navigate("/billing", cx);
}

/// Gaining GitHub access is what fills the inbox and the branch's pull request,
/// so both are asked for as soon as it happens.
fn set_auth_state(state: AuthState, cx: &mut App) {
  let had_github_access = AuthStateStore::has_github_access(cx);
  AuthStateStore::set(cx, state);
  let has_github_access = AuthStateStore::has_github_access(cx);

  if !had_github_access && has_github_access {
    fetch_initial_notifications(cx);
  }
  if had_github_access != has_github_access {
    crate::session_page::SessionPageHandle::refresh_github_state(cx);
  }
  cx.refresh_windows();
}

fn fetch_initial_notifications(cx: &mut App) {
  let api = WorkspaceApi::global(cx).api.clone();
  cx.spawn(async move |cx| {
    let result = cx
      .background_spawn(async move { api.fetch_github_notifications() })
      .await;
    cx.update(|cx| {
      if let Ok(notifications) = result {
        GithubNotificationsStore::set(cx, notifications);
        cx.refresh_windows();
      }
    });
  })
  .detach();
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::api::ApiClient;
  use gpui::TestAppContext;
  use std::io::{Read as _, Write as _};
  use std::net::TcpListener;
  use std::sync::{Arc, Mutex};
  use std::thread;

  /// Answers the calls the flow makes, in order, and records what was asked.
  fn start_backend(
    responses: Vec<(&'static str, String)>,
  ) -> (String, Arc<Mutex<Vec<String>>>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test backend");
    let address = format!("http://{}", listener.local_addr().expect("local addr"));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorder = requests.clone();

    let handle = thread::spawn(move || {
      for (status, body) in responses {
        let Ok((mut stream, _)) = listener.accept() else {
          return;
        };
        let mut buffer = [0u8; 4096];
        let read = stream.read(&mut buffer).unwrap_or(0);
        let request = String::from_utf8_lossy(&buffer[..read]).to_string();
        if let Some(first_line) = request.lines().next() {
          recorder
            .lock()
            .expect("record request")
            .push(first_line.to_string());
        }
        let response = format!(
          "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}",
          body.len(),
          body,
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
      }
    });

    (address, requests, handle)
  }

  fn init_test_app(cx: &mut TestAppContext, base_url: String) {
    cx.update(|cx| {
      cx.set_global(WorkspaceApi {
        api: ApiClient::new_with_base_url(base_url),
      });
      cx.set_global(AuthStateStore::default());
      cx.set_global(crate::config::AppSettings::default());
      cx.set_global(GithubNotificationsStore::default());
    });
  }

  fn user_payload() -> String {
    serde_json::json!({
      "id": "user_123",
      "name": "Joris",
      "email": "joris@example.com",
      "emailVerified": true,
      "image": null,
      "githubLogin": "joris-gallot",
      "role": "pro",
    })
    .to_string()
  }

  fn asked(requests: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    requests.lock().expect("read requests").clone()
  }

  #[gpui::test]
  async fn signing_in_exchanges_the_code_then_asks_who_we_are(cx: &mut TestAppContext) {
    let (base_url, requests, _backend) = start_backend(vec![
      (
        "200 OK",
        serde_json::json!({ "token": "tok_abc" }).to_string(),
      ),
      ("200 OK", user_payload()),
      ("200 OK", "[]".to_string()),
    ]);
    init_test_app(cx, base_url);

    cx.update(|cx| handle_auth_code("code_xyz".to_string(), cx));
    cx.run_until_parked();

    let asked = asked(&requests);
    assert!(
      asked
        .first()
        .is_some_and(|line| line.starts_with("POST /auth/exchange")),
      "the code is exchanged first, got {asked:?}"
    );
    assert!(
      asked.iter().any(|line| line.starts_with("GET /users/me")),
      "then the token is used to find out who is signed in, got {asked:?}"
    );
    cx.update(|cx| {
      assert!(AuthStateStore::has_github_access(cx));
    });
  }

  #[gpui::test]
  async fn a_rejected_code_signs_nobody_in(cx: &mut TestAppContext) {
    let (base_url, requests, _backend) = start_backend(vec![("400 Bad Request", "{}".to_string())]);
    init_test_app(cx, base_url);

    cx.update(|cx| handle_auth_code("code_xyz".to_string(), cx));
    cx.run_until_parked();

    assert_eq!(
      asked(&requests).len(),
      1,
      "a rejected code stops there, it never asks who we are"
    );
    cx.update(|cx| {
      assert!(matches!(
        AuthStateStore::get(cx),
        AuthState::Unauthenticated
      ));
    });
  }

  #[gpui::test]
  async fn signing_out_tells_the_backend_and_forgets_the_user(cx: &mut TestAppContext) {
    let (base_url, requests, _backend) = start_backend(vec![
      (
        "200 OK",
        serde_json::json!({ "token": "tok_abc" }).to_string(),
      ),
      ("200 OK", user_payload()),
      ("200 OK", "[]".to_string()),
      ("200 OK", "{}".to_string()),
    ]);
    init_test_app(cx, base_url);

    cx.update(|cx| handle_auth_code("code_xyz".to_string(), cx));
    cx.run_until_parked();
    cx.update(|cx| assert!(AuthStateStore::has_github_access(cx)));

    cx.update(sign_out);
    cx.run_until_parked();

    assert!(
      asked(&requests)
        .iter()
        .any(|line| line.starts_with("POST /api/auth/sign-out")),
      "the session is revoked on the backend, not only locally"
    );
    cx.update(|cx| {
      assert!(!AuthStateStore::has_github_access(cx));
    });
  }

  #[gpui::test]
  async fn gaining_github_access_fills_the_inbox(cx: &mut TestAppContext) {
    let (base_url, requests, _backend) = start_backend(vec![
      (
        "200 OK",
        serde_json::json!({ "token": "tok_abc" }).to_string(),
      ),
      ("200 OK", user_payload()),
      (
        "200 OK",
        serde_json::json!({ "notifications": [{
          "id": "1",
          "repository": { "name": "widget", "full_name": "acme/widget" },
          "subject": {
            "title": "Improve parser",
            "type": "PullRequest",
            "url": "https://api.github.com/repos/acme/widget/pulls/42",
            "latest_comment_url": null,
          },
          "reason": "review_requested",
          "unread": true,
          "updated_at": "2026-08-16T12:00:00Z",
          "last_read_at": null,
          "url": "https://api.github.com/notifications/threads/1",
          "subscription_url": "https://api.github.com/notifications/threads/1/subscription",
        }] })
        .to_string(),
      ),
    ]);
    init_test_app(cx, base_url);

    cx.update(|cx| handle_auth_code("code_xyz".to_string(), cx));
    cx.run_until_parked();

    assert!(
      asked(&requests)
        .iter()
        .any(|line| line.contains("/github/notifications")),
      "signing in is what fills the inbox, without waiting for a page to ask"
    );
    cx.update(|cx| {
      assert_eq!(GithubNotificationsStore::unread_count(cx), 1);
    });
  }

  #[gpui::test]
  async fn starting_a_sign_in_hands_the_browser_the_url(cx: &mut TestAppContext) {
    let (base_url, _requests, _backend) = start_backend(vec![]);
    init_test_app(cx, base_url);

    cx.update(|cx| start_github_sign_in(cx, "test"));
    cx.run_until_parked();

    let opened = cx.opened_url().expect("a browser was sent somewhere");
    assert!(
      opened.contains("/auth/desktop"),
      "the browser starts the OAuth round trip, got {opened}"
    );
  }

  #[gpui::test]
  async fn coming_back_from_the_checkout_refreshes_the_subscription(cx: &mut TestAppContext) {
    let (base_url, requests, _backend) = start_backend(vec![
      ("200 OK", user_payload()),
      ("200 OK", "{}".to_string()),
    ]);
    init_test_app(cx, base_url);
    cx.update(|cx| {
      gpui_router::init(cx);
      NavigationHistory::init(cx);
      NavigationHistory::navigate_replace("/session", cx);
    });

    cx.update(handle_subscription_callback);
    cx.run_until_parked();

    assert!(
      asked(&requests)
        .iter()
        .any(|line| line.starts_with("GET /users/me")),
      "the plan just changed, so the app asks again who we are"
    );
    cx.update(|cx| {
      assert_eq!(
        NavigationHistory::current_pathname(cx).as_ref(),
        "/billing",
        "and lands on the page that shows the subscription"
      );
    });
  }

  #[gpui::test]
  async fn a_token_the_backend_rejects_signs_us_out(cx: &mut TestAppContext) {
    let (base_url, requests, _backend) = start_backend(vec![
      (
        "200 OK",
        serde_json::json!({ "token": "tok_abc" }).to_string(),
      ),
      ("200 OK", user_payload()),
      ("200 OK", "[]".to_string()),
      // The token was revoked server-side since.
      ("401 Unauthorized", "{}".to_string()),
    ]);
    init_test_app(cx, base_url);

    cx.update(|cx| handle_auth_code("code_xyz".to_string(), cx));
    cx.run_until_parked();
    cx.update(|cx| assert!(AuthStateStore::has_github_access(cx)));

    cx.update(refresh_me);
    cx.run_until_parked();

    assert_eq!(
      asked(&requests)
        .iter()
        .filter(|line| line.starts_with("GET /users/me"))
        .count(),
      2
    );
    cx.update(|cx| {
      assert!(
        !AuthStateStore::has_github_access(cx),
        "a rejected token must not keep showing Pro"
      );
    });
  }

  #[gpui::test]
  async fn without_a_stored_token_the_app_starts_signed_out(cx: &mut TestAppContext) {
    // The test platform has no keychain, which is exactly a fresh machine.
    let (base_url, requests, _backend) = start_backend(vec![]);
    init_test_app(cx, base_url);

    cx.update(load_stored_token);
    cx.run_until_parked();

    assert!(
      asked(&requests).is_empty(),
      "nothing is asked of the backend without a token"
    );
    cx.update(|cx| {
      assert!(matches!(
        AuthStateStore::get(cx),
        AuthState::Unauthenticated
      ));
    });
  }
}
