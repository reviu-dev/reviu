#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use app_root::AppRoot;
use editor::Quit;
use gpui::{App, Bounds, Focusable, WindowBounds, WindowOptions, prelude::*, px, size};
#[cfg(target_os = "macos")]
use gpui::{TitlebarOptions, point};
use gpui_component::Root;
#[cfg(target_os = "macos")]
use gpui_component::TitleBar;
use reqwest_client::ReqwestClient;
use std::borrow::Cow;
use std::sync::{Arc, mpsc};
use std::time::Duration;
#[cfg(target_os = "macos")]
use ui::PAGE_HEADER_HEIGHT;
use ui::{AppAssets, parse_github_url_action};
use workspace::{
  AppProfile, AuthStateStore, WorkspaceView, build_app_menus, github_navigation::open_pr_target,
  install_app_key_bindings, install_crash_reporter, show_startup_crash_report_notification,
  take_pending_startup_crash_report,
};

#[cfg(target_os = "linux")]
mod linux_single_instance {
  use std::io;
  use std::os::unix::net::UnixDatagram;
  use std::path::PathBuf;
  use std::sync::mpsc;
  use std::thread;
  use workspace::AppProfile;

  fn sock_path() -> PathBuf {
    let data_dir = std::env::var_os("XDG_DATA_HOME")
      .map(PathBuf::from)
      .unwrap_or_else(|| {
        let home = std::env::var_os("HOME").unwrap_or_else(|| "/tmp".into());
        PathBuf::from(home).join(".local/share")
      })
      .join(AppProfile::current().storage_dir_name());
    std::fs::create_dir_all(&data_dir).ok();
    data_dir.join("reviu.sock")
  }

  /// If another instance is already listening, forward the URL and return true.
  pub fn try_forward_url(url: &str) -> bool {
    let path = sock_path();
    let Ok(sock) = UnixDatagram::unbound() else {
      return false;
    };
    if sock.connect(&path).is_ok() {
      let _ = sock.send(url.as_bytes());
      return true;
    }
    false
  }

  /// Start listening for URLs from other instances.
  /// Sends received URLs into the provided channel.
  pub fn listen(tx: mpsc::Sender<Vec<String>>) {
    let path = sock_path();
    // Clean up stale socket: try to connect, if refused then the listener is dead.
    if let Ok(probe) = UnixDatagram::unbound() {
      if let Err(e) = probe.connect(&path) {
        if e.kind() == io::ErrorKind::ConnectionRefused {
          std::fs::remove_file(&path).ok();
        }
      } else {
        // Another instance is already listening, should not happen since
        // try_forward_url would have caught this, but be safe.
        return;
      }
    }
    // Remove leftover socket file that has no listener.
    // bind() fails if the file already exists.
    let _ = std::fs::remove_file(&path);
    let Ok(listener) = UnixDatagram::bind(&path) else {
      return;
    };
    thread::spawn(move || {
      let mut buf = [0u8; 2048];
      while let Ok(len) = listener.recv(&mut buf) {
        let url = String::from_utf8_lossy(&buf[..len]).to_string();
        let _ = tx.send(vec![url]);
      }
    });
  }
}

#[cfg(target_os = "windows")]
mod windows_single_instance {
  use std::ffi::OsStr;
  use std::os::windows::ffi::OsStrExt;
  use std::ptr;
  use std::sync::mpsc;
  use std::thread;
  use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_PIPE_CONNECTED, GENERIC_WRITE, GetLastError, INVALID_HANDLE_VALUE,
  };
  use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, OPEN_EXISTING, PIPE_ACCESS_INBOUND, ReadFile, WriteFile,
  };
  use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_MESSAGE,
    PIPE_TYPE_MESSAGE, PIPE_WAIT,
  };
  use workspace::AppProfile;

  const PIPE_BUFFER_SIZE: usize = 4096;

  fn pipe_name() -> Vec<u16> {
    let name = format!(r"\\.\pipe\{}-deeplink", AppProfile::current().url_scheme());
    OsStr::new(&name).encode_wide().chain(Some(0)).collect()
  }

  pub fn try_forward_url(url: &str) -> bool {
    let pipe_name = pipe_name();
    let handle = unsafe {
      CreateFileW(
        pipe_name.as_ptr(),
        GENERIC_WRITE,
        0,
        ptr::null(),
        OPEN_EXISTING,
        FILE_ATTRIBUTE_NORMAL,
        ptr::null_mut(),
      )
    };
    if handle == INVALID_HANDLE_VALUE {
      return false;
    }

    let bytes = url.as_bytes();
    let mut bytes_written = 0;
    let write_result = unsafe {
      WriteFile(
        handle,
        bytes.as_ptr(),
        bytes.len() as u32,
        &mut bytes_written,
        ptr::null_mut(),
      )
    };
    unsafe {
      CloseHandle(handle);
    }

    write_result != 0 && bytes_written as usize == bytes.len()
  }

  pub fn listen(tx: mpsc::Sender<Vec<String>>) {
    let _ = thread::Builder::new()
      .name("WindowsDeeplinkListener".to_string())
      .spawn(move || {
        let pipe_name = pipe_name();
        loop {
          let pipe = unsafe {
            CreateNamedPipeW(
              pipe_name.as_ptr(),
              PIPE_ACCESS_INBOUND,
              PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
              1,
              PIPE_BUFFER_SIZE as u32,
              PIPE_BUFFER_SIZE as u32,
              0,
              ptr::null(),
            )
          };
          if pipe == INVALID_HANDLE_VALUE {
            return;
          }

          let connected = unsafe {
            ConnectNamedPipe(pipe, ptr::null_mut()) != 0 || GetLastError() == ERROR_PIPE_CONNECTED
          };
          if connected {
            let mut buffer = [0_u8; PIPE_BUFFER_SIZE];
            let mut bytes_read = 0;
            let read_result = unsafe {
              ReadFile(
                pipe,
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                &mut bytes_read,
                ptr::null_mut(),
              )
            };
            if read_result != 0 && bytes_read > 0 {
              let url = String::from_utf8_lossy(&buffer[..bytes_read as usize]).to_string();
              let _ = tx.send(vec![url]);
            }
          }

          unsafe {
            DisconnectNamedPipe(pipe);
            CloseHandle(pipe);
          }
        }
      });
  }
}

mod app_root;
const INITIAL_WINDOW_WIDTH: f32 = 1200.0;
const INITIAL_WINDOW_HEIGHT: f32 = 800.0;
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

#[cfg(unix)]
mod shell_env;

fn main() {
  #[cfg(unix)]
  shell_env::handle_printenv_flag();
  #[cfg(unix)]
  shell_env::load();

  let app_profile = AppProfile::current();
  // Used only on Linux/Windows (deeplink forwarding); unused on macOS.
  #[allow(unused_variables)]
  let startup_deeplink_url = startup_deeplink_url(app_profile);

  // On Linux, the .desktop file launches a new process for deeplinks.
  // If an instance is already running, forward the URL via Unix socket and exit.
  #[cfg(target_os = "linux")]
  {
    if let Some(url) = startup_deeplink_url.as_deref()
      && linux_single_instance::try_forward_url(url)
    {
      std::process::exit(0);
    }
  }

  // On Windows, protocol handlers launch a new process for deeplinks.
  // Forward the URL to the running instance so the existing window handles it.
  #[cfg(target_os = "windows")]
  {
    if let Some(url) = startup_deeplink_url.as_deref()
      && windows_single_instance::try_forward_url(url)
    {
      std::process::exit(0);
    }
  }

  let traces_sample_rate = if cfg!(debug_assertions) { 1.0 } else { 0.1 };
  let dsn = if sentry_enabled() {
    Some(SENTRY_DSN.parse().expect("Invalid Sentry DSN"))
  } else {
    None
  };

  if dsn.is_none() {
    println!("Sentry error reporting is disabled.");
  }

  let _guard = sentry::init(sentry::ClientOptions {
    dsn,
    release: resolved_sentry_release(),
    send_default_pii: false,
    attach_stacktrace: true,
    max_breadcrumbs: 300,
    traces_sample_rate,
    in_app_include: vec!["reviu", "workspace", "editor", "git", "ui"],
    before_send: Some(Arc::new(redact_sensitive_event_data)),
    ..Default::default()
  });
  install_crash_reporter();
  let startup_crash_report = take_pending_startup_crash_report();

  let (open_url_tx, open_url_rx) = mpsc::channel::<Vec<String>>();

  // On Linux, listen for deeplinks forwarded from other instances via Unix socket.
  #[cfg(target_os = "linux")]
  linux_single_instance::listen(open_url_tx.clone());

  // On Windows, listen for deeplinks forwarded from other instances via named pipe.
  #[cfg(target_os = "windows")]
  windows_single_instance::listen(open_url_tx.clone());

  // If this instance was launched with a deeplink URL arg (first launch),
  // inject it into the channel so it gets processed once the app is ready.
  #[cfg(any(target_os = "linux", target_os = "windows"))]
  if let Some(url) = startup_deeplink_url.clone() {
    let _ = open_url_tx.send(vec![url]);
  }

  let app = gpui_platform::application().with_assets(AppAssets);
  app.on_open_urls({
    let open_url_tx = open_url_tx.clone();
    move |urls| {
      let _ = open_url_tx.send(urls);
    }
  });

  app.run(move |cx: &mut App| {
    gpui_component::init(cx);
    ui::init(cx);
    if let Some(dir) = agent_registry::icon_cache_dir() {
      ui::set_runtime_asset_dir(dir);
    }
    // gpui-component defaults the UI font to `.SystemUIFont`, which only
    // resolves on macOS; elsewhere it falls back to a monospace face. Point
    // the theme at the fonts we bundle (see ui::init) so Linux and Windows
    // render with the same proportional/monospace pair as macOS.
    #[cfg(not(target_os = "macos"))]
    {
      let theme = gpui_component::Theme::global_mut(cx);
      theme.font_family = "Inter".into();
      theme.mono_font_family = "Lilex".into();
    }
    let http_client = ReqwestClient::user_agent("reviu").expect("Failed to create HTTP client");
    cx.set_http_client(Arc::new(http_client));

    let bounds = Bounds::centered(
      None,
      size(px(INITIAL_WINDOW_WIDTH), px(INITIAL_WINDOW_HEIGHT)),
      cx,
    );

    install_app_key_bindings(cx);

    cx.set_menus(build_app_menus(false));

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
      .update(cx, move |_, window, cx| {
        window.activate_window();
        cx.activate(true);
        if let Some(report) = startup_crash_report.clone() {
          window.on_next_frame(move |window, cx| {
            show_startup_crash_report_notification(window, report.clone(), cx);
          });
        }
      })
      .unwrap();

    // Refresh the agent registry off the UI thread; the cached or embedded
    // copy already backs the picker, so a slow or failed fetch changes nothing.
    cx.background_executor()
      .spawn(async {
        // Icons only need refetching when the document itself moved: their
        // URLs sit under /latest/, so the same URL can serve new artwork.
        let force_icons = match agent_registry::refresh_global_blocking() {
          Ok(outcome) => {
            eprintln!("[agent-registry] {outcome:?}");
            outcome == agent_registry::RefreshOutcome::Updated
          }
          Err(err) => {
            eprintln!("[agent-registry] refresh failed, keeping the cached list: {err}");
            false
          }
        };
        let fetched =
          agent_registry::download_icons_blocking(&agent_registry::global(), force_icons);
        if fetched > 0 {
          eprintln!("[agent-registry] cached {fetched} agent icons");
        }
      })
      .detach();

    std::mem::drop(cx.register_url_scheme(app_profile.url_scheme()));
    cx.spawn(async move |cx| {
      let mut pending_open_github_urls: Vec<String> = Vec::new();
      loop {
        cx.background_executor()
          .timer(Duration::from_millis(200))
          .await;
        while let Ok(urls) = open_url_rx.try_recv() {
          let mut codes = Vec::new();
          let mut should_handle_subscription_callback = false;
          for url in &urls {
            if let Some(code) = extract_auth_code(url) {
              codes.push(code);
            } else if is_subscription_callback(url) {
              should_handle_subscription_callback = true;
            } else if let Some(github_url) = extract_open_url(url) {
              pending_open_github_urls.push(github_url);
            }
          }
          if codes.is_empty()
            && !should_handle_subscription_callback
            && pending_open_github_urls.is_empty()
          {
            continue;
          }
          cx.update(|cx| {
            for code in codes {
              workspace::auth_flow::handle_auth_code(code, cx);
            }
            if should_handle_subscription_callback {
              workspace::auth_flow::handle_subscription_callback(cx);
            }
          });
        }
        if !pending_open_github_urls.is_empty() {
          let auth_known = cx.update(|cx| AuthStateStore::is_known(cx));
          if auth_known {
            let urls: Vec<String> = std::mem::take(&mut pending_open_github_urls);
            cx.update(|cx| {
              for github_url in urls {
                handle_open_github_url(&github_url, cx);
              }
            });
          }
        }
      }
    })
    .detach();

    cx.on_action(|_: &Quit, cx| cx.quit());
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

fn startup_deeplink_url(app_profile: AppProfile) -> Option<String> {
  startup_deeplink_url_from_args(std::env::args(), app_profile.url_scheme())
}

fn startup_deeplink_url_from_args<I, S>(args: I, scheme: &str) -> Option<String>
where
  I: IntoIterator<Item = S>,
  S: AsRef<str>,
{
  let url = args.into_iter().nth(1)?.as_ref().to_string();
  if url
    .strip_prefix(scheme)
    .and_then(|tail| tail.strip_prefix("://"))
    .is_some()
  {
    Some(url)
  } else {
    None
  }
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

fn current_desktop_url_scheme() -> &'static str {
  AppProfile::current().url_scheme()
}

fn extract_auth_code(url: &str) -> Option<String> {
  extract_auth_code_for_scheme(url, current_desktop_url_scheme())
}

fn extract_auth_code_for_scheme(url: &str, scheme: &str) -> Option<String> {
  let url = url.strip_prefix(scheme)?;
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
  is_subscription_callback_for_scheme(url, current_desktop_url_scheme())
}

fn is_subscription_callback_for_scheme(url: &str, scheme: &str) -> bool {
  let Some(url) = url.strip_prefix(scheme) else {
    return false;
  };
  let Some(url) = url.strip_prefix("://") else {
    return false;
  };

  let path = url.split_once('?').map(|(path, _)| path).unwrap_or(url);
  path == "subscription/callback"
}

fn extract_open_url(url: &str) -> Option<String> {
  extract_open_url_for_scheme(url, current_desktop_url_scheme())
    .or_else(|| extract_open_url_for_scheme(url, workspace::URL_SCHEME_PROD))
}

fn extract_open_url_for_scheme(url: &str, scheme: &str) -> Option<String> {
  let url = url.strip_prefix(scheme)?;
  let url = url.strip_prefix("://")?;
  let (path, query) = url.split_once('?')?;
  if path != "open" {
    return None;
  }
  for pair in query.split('&') {
    if let Some(value) = pair.strip_prefix("url=")
      && !value.is_empty()
    {
      let decoded = percent_encoding::percent_decode_str(value)
        .decode_utf8()
        .ok()?;
      return Some(decoded.into_owned());
    }
  }
  None
}

struct PrDeepLink {
  owner: String,
  repo: String,
  number: u64,
  open_changes_tab: bool,
  review_comment_id: Option<u64>,
}

/// Only pull requests have a page in Reviu; other GitHub links would bounce
/// straight back to the browser they came from.
fn pr_deep_link(url: &str) -> Option<PrDeepLink> {
  match parse_github_url_action(url)? {
    ui::CommandPaletteAction::OpenGithubPrDetails {
      owner,
      repo,
      number,
      open_changes_tab,
      review_comment_id,
    } => Some(PrDeepLink {
      owner,
      repo,
      number,
      open_changes_tab,
      review_comment_id,
    }),
    _ => None,
  }
}

fn handle_open_github_url(url: &str, cx: &mut App) {
  let Some(target) = pr_deep_link(url) else {
    return;
  };
  cx.activate(true);
  open_pr_target(
    target.owner,
    target.repo,
    target.number,
    target.open_changes_tab,
    target.review_comment_id,
    cx,
  );
}

#[cfg(test)]
mod tests {
  use super::{
    contains_sensitive_fragment, extract_auth_code_for_scheme, extract_open_url_for_scheme,
    is_sensitive_header, is_sensitive_key, is_subscription_callback_for_scheme,
    is_truthy_env_value, pr_deep_link, redact_sensitive_event_data, resolved_sentry_release_from,
    sentry_enabled_for, startup_deeplink_url_from_args,
  };
  use sentry::protocol::{Event, Request, Value};
  use std::borrow::Cow;

  #[test]
  fn pr_deep_link_accepts_pull_request_urls() {
    let target = pr_deep_link("https://github.com/acme/widget/pull/42").expect("pr deep link");
    assert_eq!(target.owner, "acme");
    assert_eq!(target.repo, "widget");
    assert_eq!(target.number, 42);

    let with_comment =
      pr_deep_link("https://github.com/acme/widget/pull/42#discussion_r123").expect("pr deep link");
    assert_eq!(with_comment.review_comment_id, Some(123));
  }

  #[test]
  fn pr_deep_link_ignores_links_without_a_reviu_page() {
    for url in [
      "https://github.com/acme/widget",
      "https://github.com/acme/widget/issues/42",
      "https://github.com/acme/widget/issues",
      "https://github.com/acme/widget/pulls",
      "https://github.com/acme/widget/commit/abc123",
      "https://github.com/acme",
      "https://example.com/acme/widget/pull/42",
    ] {
      assert!(
        pr_deep_link(url).is_none(),
        "{url} should not open a Reviu page"
      );
    }
  }

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
  fn startup_deeplink_url_reads_current_profile_url_arg() {
    let url = startup_deeplink_url_from_args(
      [
        "reviu".to_string(),
        "reviu://auth/callback?code=abc123".to_string(),
      ],
      "reviu",
    );

    assert_eq!(url.as_deref(), Some("reviu://auth/callback?code=abc123"));
  }

  #[test]
  fn startup_deeplink_url_rejects_other_args() {
    assert_eq!(
      startup_deeplink_url_from_args(["reviu", "--help"], "reviu"),
      None
    );
    assert_eq!(
      startup_deeplink_url_from_args(["reviu", "reviu-dev://auth/callback?code=abc123"], "reviu"),
      None
    );
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
    let code =
      extract_auth_code_for_scheme("reviu://auth/callback?code=abc123&state=test", "reviu");
    assert_eq!(code.as_deref(), Some("abc123"));
  }

  #[test]
  fn extract_auth_code_rejects_non_auth_callback_url() {
    let code = extract_auth_code_for_scheme("reviu://subscription/callback", "reviu");
    assert_eq!(code, None);
  }

  #[test]
  fn is_subscription_callback_accepts_plain_and_query_urls() {
    assert!(is_subscription_callback_for_scheme(
      "reviu://subscription/callback",
      "reviu"
    ));
    assert!(is_subscription_callback_for_scheme(
      "reviu://subscription/callback?checkout_id=123",
      "reviu"
    ));
  }

  #[test]
  fn is_subscription_callback_rejects_other_urls() {
    assert!(!is_subscription_callback_for_scheme(
      "reviu://auth/callback?code=abc123",
      "reviu"
    ));
    assert!(!is_subscription_callback_for_scheme(
      "https://example.com",
      "reviu"
    ));
  }

  #[test]
  fn extract_auth_code_supports_dev_scheme() {
    let code = extract_auth_code_for_scheme("reviu-dev://auth/callback?code=dev123", "reviu-dev");
    assert_eq!(code.as_deref(), Some("dev123"));
  }

  #[test]
  fn is_subscription_callback_supports_dev_scheme() {
    assert!(is_subscription_callback_for_scheme(
      "reviu-dev://subscription/callback",
      "reviu-dev"
    ));
  }

  #[test]
  fn extract_open_url_decodes_github_pr_url() {
    let url = extract_open_url_for_scheme(
      "reviu://open?url=https%3A%2F%2Fgithub.com%2Fowner%2Frepo%2Fpull%2F42",
      "reviu",
    );
    assert_eq!(
      url.as_deref(),
      Some("https://github.com/owner/repo/pull/42")
    );
  }

  #[test]
  fn extract_open_url_decodes_github_repo_url() {
    let url = extract_open_url_for_scheme(
      "reviu://open?url=https%3A%2F%2Fgithub.com%2Fowner%2Frepo",
      "reviu",
    );
    assert_eq!(url.as_deref(), Some("https://github.com/owner/repo"));
  }

  #[test]
  fn extract_open_url_supports_dev_scheme() {
    let url = extract_open_url_for_scheme(
      "reviu-dev://open?url=https%3A%2F%2Fgithub.com%2Fa%2Fb",
      "reviu-dev",
    );
    assert_eq!(url.as_deref(), Some("https://github.com/a/b"));
  }

  #[test]
  fn extract_open_url_rejects_non_open_path() {
    let url = extract_open_url_for_scheme("reviu://auth/callback?url=https%3A%2F%2Fx", "reviu");
    assert_eq!(url, None);
  }

  #[test]
  fn extract_open_url_rejects_empty_url_param() {
    let url = extract_open_url_for_scheme("reviu://open?url=", "reviu");
    assert_eq!(url, None);
  }

  #[test]
  fn extract_open_url_rejects_missing_query() {
    let url = extract_open_url_for_scheme("reviu://open", "reviu");
    assert_eq!(url, None);
  }

  #[test]
  fn extract_open_url_handles_fragment_in_github_url() {
    let url = extract_open_url_for_scheme(
      "reviu://open?url=https%3A%2F%2Fgithub.com%2Fowner%2Frepo%2Fpull%2F1%23discussion_r123",
      "reviu",
    );
    assert_eq!(
      url.as_deref(),
      Some("https://github.com/owner/repo/pull/1#discussion_r123")
    );
  }
}
