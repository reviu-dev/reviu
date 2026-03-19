use app_root::AppRoot;
use editor::*;
use gpui::{
  App, Bounds, Focusable, KeyBinding, TitlebarOptions, WindowBounds, WindowOptions, point,
  prelude::*, px, size,
};
use gpui_component::{Root, TitleBar};
use reqwest_client::ReqwestClient;
use std::borrow::Cow;
use std::sync::{Arc, mpsc};
use std::time::Duration;
use ui::{AppAssets, PAGE_HEADER_HEIGHT};
use workspace::{
  AuthCallbackTarget, CloseWorkspacePage, CommitChanges, OpenRepository, ShowCommandPalette,
  ShowFileSearch, WorkspaceView,
};

mod app_root;
const INITIAL_WINDOW_WIDTH: f32 = 1200.0;
const INITIAL_WINDOW_HEIGHT: f32 = 800.0;
const REVIU_URL_SCHEME: &str = "reviu";
const SENTRY_DSN: &str =
  "https://a816f0ac9d37d42ec72719de4770c538@o1155685.ingest.us.sentry.io/4510920248918016";
const SENTRY_ENABLE_DEV_ENV: &str = "SENTRY_ENABLE_DEV";
const SENTRY_REDACTED: &str = "[REDACTED]";
#[cfg(target_os = "macos")]
const MACOS_TRAFFIC_LIGHT_X: f32 = 9.0;
#[cfg(target_os = "macos")]
const MACOS_TRAFFIC_LIGHT_Y: f32 = PAGE_HEADER_HEIGHT / 2.0 - 12.0;

#[cfg(target_os = "macos")]
fn macos_titlebar_options() -> TitlebarOptions {
  let mut options = TitleBar::title_bar_options();

  options.title = Some("Reviu".into());
  // Align traffic lights vertically with the 50px global top bar.
  options.traffic_light_position =
    Some(point(px(MACOS_TRAFFIC_LIGHT_X), px(MACOS_TRAFFIC_LIGHT_Y)));
  options
}

fn main() {
  let traces_sample_rate = if cfg!(debug_assertions) { 1.0 } else { 0.1 };
  let dsn = if sentry_enabled() {
    Some(SENTRY_DSN.parse().expect("Invalid Sentry DSN"))
  } else {
    None
  };

  println!("Starting Reviu (Sentry enabled: {})", dsn.is_some());
  let _guard = sentry::init(sentry::ClientOptions {
    dsn,
    release: resolved_sentry_release(),
    // Capture user IPs and potentially sensitive headers when using HTTP server integrations
    // see https://docs.sentry.io/platforms/rust/data-management/data-collected for more info
    send_default_pii: true,
    attach_stacktrace: true,
    max_breadcrumbs: 300,
    traces_sample_rate,
    in_app_include: vec!["reviu", "workspace", "editor", "git", "ui"],
    before_send: Some(Arc::new(redact_sensitive_event_data)),
    ..Default::default()
  });

  let (open_url_tx, open_url_rx) = mpsc::channel::<Vec<String>>();
  let app = gpui_platform::application().with_assets(AppAssets);
  app.on_open_urls({
    let open_url_tx = open_url_tx.clone();
    move |urls| {
      let _ = open_url_tx.send(urls);
    }
  });

  app.run(move |cx: &mut App| {
    gpui_component::init(cx);
    let http_client = ReqwestClient::user_agent("reviu").expect("Failed to create HTTP client");
    cx.set_http_client(Arc::new(http_client));

    let bounds = Bounds::centered(
      None,
      size(px(INITIAL_WINDOW_WIDTH), px(INITIAL_WINDOW_HEIGHT)),
      cx,
    );

    cx.bind_keys([
      KeyBinding::new("enter", Enter, None),
      KeyBinding::new("tab", Tab, None),
      KeyBinding::new("backspace", Backspace, None),
      KeyBinding::new("alt-backspace", BackspaceWord, None),
      KeyBinding::new("cmd-backspace", BackspaceAll, None),
      KeyBinding::new("delete", Delete, None),
      KeyBinding::new("up", Up, None),
      KeyBinding::new("down", Down, None),
      KeyBinding::new("left", Left, None),
      KeyBinding::new("alt-left", AltLeft, None),
      KeyBinding::new("cmd-left", CmdLeft, None),
      KeyBinding::new("right", Right, None),
      KeyBinding::new("alt-right", AltRight, None),
      KeyBinding::new("cmd-right", CmdRight, None),
      KeyBinding::new("cmd-up", CmdUp, None),
      KeyBinding::new("cmd-down", CmdDown, None),
      KeyBinding::new("shift-up", SelectUp, None),
      KeyBinding::new("shift-down", SelectDown, None),
      KeyBinding::new("shift-cmd-left", SelectCmdLeft, None),
      KeyBinding::new("shift-cmd-right", SelectCmdRight, None),
      KeyBinding::new("shift-cmd-up", SelectCmdUp, None),
      KeyBinding::new("shift-cmd-down", SelectCmdDown, None),
      KeyBinding::new("shift-left", SelectLeft, None),
      KeyBinding::new("shift-alt-left", SelectWordLeft, None),
      KeyBinding::new("shift-right", SelectRight, None),
      KeyBinding::new("shift-alt-right", SelectWordRight, None),
      KeyBinding::new("cmd-a", SelectAll, None),
      KeyBinding::new("cmd-v", Paste, None),
      KeyBinding::new("cmd-c", Copy, None),
      KeyBinding::new("cmd-x", Cut, None),
      KeyBinding::new("cmd-z", Undo, None),
      KeyBinding::new("cmd-shift-z", Redo, None),
      KeyBinding::new("cmd-s", Save, None),
      KeyBinding::new("cmd-f", Find, None),
      KeyBinding::new("escape", CloseFind, Some("Editor")),
      KeyBinding::new("cmd-enter", CommitChanges, None),
      KeyBinding::new("cmd-o", OpenRepository, None),
      KeyBinding::new("cmd-shift-p", ShowCommandPalette, None),
      KeyBinding::new("cmd-p", ShowFileSearch, None),
      KeyBinding::new("cmd-w", CloseWorkspacePage, None),
      KeyBinding::new("home", Home, None),
      KeyBinding::new("end", End, None),
      KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, None),
    ]);

    let window_options = WindowOptions {
      window_bounds: Some(WindowBounds::Windowed(bounds)),
      #[cfg(target_os = "macos")]
      titlebar: Some(macos_titlebar_options()),
      ..Default::default()
    };

    let window = cx
      .open_window(window_options, |window, cx| {
        let view = cx.new(|cx| WorkspaceView::new(window, cx));
        let app_root = cx.new(|cx| AppRoot::new(view, window, cx));
        window.focus(&app_root.focus_handle(cx), cx);
        cx.new(|cx| Root::new(app_root, window, cx))
      })
      .unwrap();

    window
      .update(cx, |_, window, cx| {
        window.activate_window();
        cx.activate(true);
      })
      .unwrap();

    std::mem::drop(cx.register_url_scheme(REVIU_URL_SCHEME));
    cx.spawn(async move |cx| {
      loop {
        cx.background_executor()
          .timer(Duration::from_millis(200))
          .await;
        while let Ok(urls) = open_url_rx.try_recv() {
          let mut codes = Vec::new();
          let mut should_handle_subscription_callback = false;
          for url in urls {
            if let Some(code) = extract_auth_code(&url) {
              codes.push(code);
            }
            if is_subscription_callback(&url) {
              should_handle_subscription_callback = true;
            }
          }
          if codes.is_empty() && !should_handle_subscription_callback {
            continue;
          }
          cx.update(|cx| {
            for code in codes {
              AuthCallbackTarget::handle_auth_code(code, cx);
            }
            if should_handle_subscription_callback {
              AuthCallbackTarget::handle_subscription_callback(cx);
            }
          });
        }
      }
    })
    .detach();

    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
  });
}

fn resolved_sentry_release_from(
  env_release: Option<&'static str>,
  fallback: Option<Cow<'static, str>>,
) -> Option<Cow<'static, str>> {
  env_release.map(Cow::Borrowed).or(fallback)
}

fn resolved_sentry_release() -> Option<Cow<'static, str>> {
  resolved_sentry_release_from(option_env!("SENTRY_RELEASE"), sentry::release_name!())
}

fn sentry_enabled() -> bool {
  let dev_flag = std::env::var(SENTRY_ENABLE_DEV_ENV).ok();
  sentry_enabled_for(cfg!(debug_assertions), dev_flag.as_deref())
}

fn sentry_enabled_for(debug_build: bool, dev_flag: Option<&str>) -> bool {
  if !debug_build {
    return true;
  }

  dev_flag.is_some_and(is_truthy_env_value)
}

fn is_truthy_env_value(value: &str) -> bool {
  matches!(
    value.trim().to_ascii_lowercase().as_str(),
    "1" | "true" | "yes" | "on"
  )
}

fn redact_sensitive_event_data(
  mut event: sentry::protocol::Event<'static>,
) -> Option<sentry::protocol::Event<'static>> {
  if let Some(request) = event.request.as_mut() {
    if request.query_string.is_some() {
      request.query_string = Some(SENTRY_REDACTED.to_string());
    }
    if request.cookies.is_some() {
      request.cookies = Some(SENTRY_REDACTED.to_string());
    }
    for (name, value) in &mut request.headers {
      if is_sensitive_header(name) {
        *value = SENTRY_REDACTED.to_string();
      }
    }
    if request
      .data
      .as_ref()
      .is_some_and(|value| contains_sensitive_fragment(value))
    {
      request.data = Some(SENTRY_REDACTED.to_string());
    }
  }

  for (key, value) in &mut event.extra {
    if is_sensitive_key(key) {
      *value = sentry::protocol::Value::String(SENTRY_REDACTED.to_string());
    }
  }

  Some(event)
}

fn is_sensitive_header(name: &str) -> bool {
  let name = name.trim().to_ascii_lowercase();
  name == "authorization"
    || name == "cookie"
    || name == "set-cookie"
    || name == "x-api-key"
    || name == "x-auth-token"
}

fn is_sensitive_key(key: &str) -> bool {
  let key = key.trim().to_ascii_lowercase();
  key.contains("token")
    || key.contains("secret")
    || key.contains("password")
    || key.contains("authorization")
}

fn contains_sensitive_fragment(value: &str) -> bool {
  let value = value.to_ascii_lowercase();
  value.contains("token")
    || value.contains("secret")
    || value.contains("password")
    || value.contains("authorization")
}

fn extract_auth_code(url: &str) -> Option<String> {
  let url = url.strip_prefix(REVIU_URL_SCHEME)?;
  let url = url.strip_prefix("://")?;
  let (path, query) = url.split_once('?')?;
  if path != "auth/callback" {
    return None;
  }
  for pair in query.split('&') {
    let (key, value) = pair.split_once('=')?;
    if key == "code" && !value.is_empty() {
      return Some(value.to_string());
    }
  }
  None
}

fn is_subscription_callback(url: &str) -> bool {
  let Some(url) = url.strip_prefix(REVIU_URL_SCHEME) else {
    return false;
  };
  let Some(url) = url.strip_prefix("://") else {
    return false;
  };

  let path = url.split_once('?').map(|(path, _)| path).unwrap_or(url);
  path == "subscription/callback"
}

#[cfg(test)]
mod tests {
  use super::{
    contains_sensitive_fragment, extract_auth_code, is_sensitive_header, is_sensitive_key,
    is_subscription_callback, is_truthy_env_value, redact_sensitive_event_data,
    resolved_sentry_release_from, sentry_enabled_for,
  };
  use sentry::protocol::{Event, Request, Value};
  use std::borrow::Cow;

  #[test]
  fn is_truthy_env_value_accepts_supported_values() {
    assert!(is_truthy_env_value("1"));
    assert!(is_truthy_env_value("true"));
    assert!(is_truthy_env_value("TRUE"));
    assert!(is_truthy_env_value("yes"));
    assert!(is_truthy_env_value("on"));
    assert!(is_truthy_env_value("  true  "));
  }

  #[test]
  fn is_truthy_env_value_rejects_other_values() {
    assert!(!is_truthy_env_value(""));
    assert!(!is_truthy_env_value("0"));
    assert!(!is_truthy_env_value("false"));
    assert!(!is_truthy_env_value("no"));
    assert!(!is_truthy_env_value("off"));
  }

  #[test]
  fn sentry_enabled_for_enables_release_without_flag() {
    assert!(sentry_enabled_for(false, None));
    assert!(sentry_enabled_for(false, Some("0")));
  }

  #[test]
  fn sentry_enabled_for_requires_flag_in_debug() {
    assert!(!sentry_enabled_for(true, None));
    assert!(!sentry_enabled_for(true, Some("0")));
    assert!(sentry_enabled_for(true, Some("1")));
  }

  #[test]
  fn resolved_sentry_release_prefers_env_value() {
    let fallback = Some(Cow::Borrowed("reviu@0.0.1"));
    let release = resolved_sentry_release_from(Some("reviu@1.2.3"), fallback);
    assert_eq!(release.as_deref(), Some("reviu@1.2.3"));
  }

  #[test]
  fn resolved_sentry_release_falls_back_when_env_missing() {
    let fallback = Some(Cow::Borrowed("reviu@0.0.1"));
    let release = resolved_sentry_release_from(None, fallback);
    assert_eq!(release.as_deref(), Some("reviu@0.0.1"));
  }

  #[test]
  fn redact_sensitive_event_data_masks_request_fields() {
    let mut request = Request {
      query_string: Some("token=abc123".into()),
      cookies: Some("session=foo".into()),
      data: Some(r#"{"password":"abc"}"#.into()),
      ..Default::default()
    };
    request
      .headers
      .insert("Authorization".into(), "Bearer secret".into());
    request.headers.insert("X-Trace-Id".into(), "safe".into());

    let mut event = Event::default();
    event.request = Some(request);
    event
      .extra
      .insert("api_token".into(), Value::String("token".into()));
    event
      .extra
      .insert("safe_key".into(), Value::String("value".into()));

    let redacted = redact_sensitive_event_data(event).expect("event");
    let request = redacted.request.expect("request");
    assert_eq!(request.query_string.as_deref(), Some("[REDACTED]"));
    assert_eq!(request.cookies.as_deref(), Some("[REDACTED]"));
    assert_eq!(request.data.as_deref(), Some("[REDACTED]"));
    assert_eq!(
      request.headers.get("Authorization").map(String::as_str),
      Some("[REDACTED]")
    );
    assert_eq!(
      request.headers.get("X-Trace-Id").map(String::as_str),
      Some("safe")
    );
    assert_eq!(
      redacted.extra.get("api_token"),
      Some(&Value::String("[REDACTED]".into()))
    );
    assert_eq!(
      redacted.extra.get("safe_key"),
      Some(&Value::String("value".into()))
    );
  }

  #[test]
  fn sensitive_key_helpers_detect_expected_patterns() {
    assert!(is_sensitive_header("authorization"));
    assert!(is_sensitive_header("X-AUTH-TOKEN"));
    assert!(!is_sensitive_header("x-trace-id"));

    assert!(is_sensitive_key("api_token"));
    assert!(is_sensitive_key("db_password"));
    assert!(!is_sensitive_key("git_branch"));

    assert!(contains_sensitive_fragment("Authorization: Bearer token"));
    assert!(!contains_sensitive_fragment("plain text"));
  }

  #[test]
  fn extract_auth_code_reads_code_from_auth_callback_url() {
    let code = extract_auth_code("reviu://auth/callback?code=abc123&state=test");
    assert_eq!(code.as_deref(), Some("abc123"));
  }

  #[test]
  fn extract_auth_code_rejects_non_auth_callback_url() {
    let code = extract_auth_code("reviu://subscription/callback");
    assert_eq!(code, None);
  }

  #[test]
  fn is_subscription_callback_accepts_plain_and_query_urls() {
    assert!(is_subscription_callback("reviu://subscription/callback"));
    assert!(is_subscription_callback(
      "reviu://subscription/callback?checkout_id=123"
    ));
  }

  #[test]
  fn is_subscription_callback_rejects_other_urls() {
    assert!(!is_subscription_callback(
      "reviu://auth/callback?code=abc123"
    ));
    assert!(!is_subscription_callback("https://example.com"));
  }
}
