use std::backtrace::Backtrace;
#[cfg(test)]
use std::cell::RefCell;
use std::collections::HashSet;
use std::fs;
use std::panic::PanicHookInfo;
use std::path::PathBuf;
use std::sync::{Mutex, Once, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use dirs::config_dir;
use gpui::{AnyWindowHandle, App, ClickEvent, Window, div, prelude::*};
use gpui_component::{Disableable as _, Sizable as _, notification::Notification};
use serde::{Deserialize, Serialize};
use smol::unblock;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use ui::{Button, ButtonVariants as _, WindowExt};

use crate::AppProfile;
use crate::app_update::resolved_build_version;
use crate::sentry_context::{
  CrashGitContext, CrashGithubPrContext, current_crash_context_snapshot,
};
use crate::workspace::WorkspaceApi;

const CRASH_REPORTS_DIR_NAME: &str = "crash-reports";
const PENDING_CRASH_REPORT_FILE_NAME: &str = "pending.json";
const CRASH_DETAILS_COPY_BACKTRACE_LIMIT: usize = 20_000;

static CRASH_REPORTER_INSTALL: Once = Once::new();
static CRASH_REPORT_PERSISTED: std::sync::atomic::AtomicBool =
  std::sync::atomic::AtomicBool::new(false);

fn crash_report_submission_state() -> &'static Mutex<HashSet<String>> {
  static STATE: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
  STATE.get_or_init(|| Mutex::new(HashSet::new()))
}

#[cfg(test)]
thread_local! {
  static TEST_CRASH_REPORTS_DIR: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StartupCrashReport {
  pub crash_id: String,
  pub message: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub panic_location: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub backtrace: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub thread_name: Option<String>,
  pub app_version: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub release: Option<String>,
  pub os: String,
  pub arch: String,
  pub app_profile: String,
  pub happened_at: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub pathname: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub workspace_page: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub(crate) git_context: Option<CrashGitContext>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub(crate) github_pr_context: Option<CrashGithubPrContext>,
}

#[derive(Debug)]
struct StartupCrashReportNotificationId;

impl StartupCrashReport {
  fn from_panic_info(info: &PanicHookInfo<'_>) -> Self {
    let snapshot = current_crash_context_snapshot();
    Self {
      crash_id: build_crash_id(),
      message: panic_message(info),
      panic_location: panic_location(info),
      backtrace: Some(Backtrace::force_capture().to_string()),
      thread_name: std::thread::current().name().map(str::to_string),
      app_version: resolved_build_version(env!("CARGO_PKG_VERSION")),
      release: option_env!("SENTRY_RELEASE").map(str::to_string),
      os: std::env::consts::OS.to_string(),
      arch: std::env::consts::ARCH.to_string(),
      app_profile: app_profile_label(AppProfile::current()).to_string(),
      happened_at: current_timestamp_rfc3339(),
      pathname: snapshot.pathname,
      workspace_page: snapshot.workspace_page,
      git_context: snapshot.git,
      github_pr_context: snapshot.github_pr,
    }
  }

  pub fn details_text(&self) -> String {
    let mut lines = vec![
      "Reviu Desktop Crash Report".to_string(),
      format!("Crash ID: {}", self.crash_id),
      format!("Message: {}", self.message.trim()),
      format!("App version: {}", self.app_version),
      format!(
        "Release: {}",
        self.release.as_deref().unwrap_or("unknown").trim()
      ),
      format!("Profile: {}", self.app_profile.trim()),
      format!("Platform: {}/{}", self.os.trim(), self.arch.trim()),
      format!("Happened at: {}", self.happened_at.trim()),
    ];

    if let Some(thread_name) = self.thread_name.as_deref()
      && !thread_name.trim().is_empty()
    {
      lines.push(format!("Thread: {}", thread_name.trim()));
    }

    if let Some(location) = self.panic_location.as_deref()
      && !location.trim().is_empty()
    {
      lines.push(format!("Panic location: {}", location.trim()));
    }

    if let Some(backtrace) = self.backtrace.as_deref()
      && !backtrace.trim().is_empty()
    {
      lines.push(String::new());
      lines.push("Backtrace:".to_string());
      lines.push(trim_multiline(
        backtrace,
        CRASH_DETAILS_COPY_BACKTRACE_LIMIT,
      ));
    }

    if let Some(workspace_page) = self.workspace_page.as_deref() {
      lines.push(String::new());
      lines.push("UI Context:".to_string());
      lines.push(format!("Workspace page: {}", workspace_page));
      if let Some(pathname) = self.pathname.as_deref() {
        lines.push(format!("Pathname: {}", pathname));
      }
    }

    if let Some(git) = self.git_context.as_ref() {
      lines.push(String::new());
      lines.push("Git Context:".to_string());
      if let Some(repo_name) = git.repo_name.as_deref() {
        lines.push(format!("Repo: {}", repo_name));
      }
      if let Some(repo_hash) = git.repo_hash.as_deref() {
        lines.push(format!("Repo hash: {}", repo_hash));
      }
      if let Some(branch) = git.branch.as_deref() {
        lines.push(format!("Branch: {}", branch));
      }
      if let Some(selected_file) = git.selected_file.as_deref() {
        lines.push(format!("Selected file: {}", selected_file));
      }
      lines.push(format!("Sidebar mode: {}", git.sidebar_mode));
      lines.push(format!("Diff view: {}", git.diff_view));
    }

    if let Some(pr) = self.github_pr_context.as_ref() {
      lines.push(String::new());
      lines.push("GitHub PR Context:".to_string());
      lines.push(format!("Repository: {}/{}", pr.owner, pr.repo));
      lines.push(format!("PR number: {}", pr.number));
      if let Some(selected_file) = pr.selected_file.as_deref() {
        lines.push(format!("Selected file: {}", selected_file));
      }
      if let Some(active_tab) = pr.active_tab {
        lines.push(format!("Active tab: {}", active_tab));
      }
    }

    lines.join("\n")
  }
}

pub fn install_crash_reporter() {
  CRASH_REPORTER_INSTALL.call_once(|| {
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
      // Only persist the first panic report. When a panic crosses an FFI boundary
      // (e.g. GPUI's Objective-C bridge), Rust catches and re-panics with a
      // generic "panic in a function that cannot unwind" message. The first call
      // has the original panic message; the second would overwrite it.
      if !CRASH_REPORT_PERSISTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        if let Err(err) =
          persist_startup_crash_report(&StartupCrashReport::from_panic_info(info))
        {
          eprintln!("Failed to persist crash report: {err}");
        }
      }

      previous_hook(info);
    }));
  });
}

pub fn take_pending_startup_crash_report() -> Option<StartupCrashReport> {
  let pending_path = pending_crash_report_path()?;
  let bytes = fs::read(&pending_path).ok()?;

  match serde_json::from_slice::<StartupCrashReport>(&bytes) {
    Ok(report) => Some(report),
    Err(err) => {
      eprintln!(
        "Failed to parse pending crash report {}: {}",
        pending_path.display(),
        err
      );
      let _ = fs::remove_file(pending_path);
      None
    }
  }
}

pub fn show_startup_crash_report_notification(
  window: &mut Window,
  report: StartupCrashReport,
  cx: &mut App,
) {
  let sending = is_crash_report_submitting(&report.crash_id);
  show_startup_crash_report_notification_with_state(window, report, sending, cx);
}

fn show_startup_crash_report_notification_with_state(
  window: &mut Window,
  report: StartupCrashReport,
  sending: bool,
  cx: &mut App,
) {
  let report_for_send = report.clone();
  let window_handle = window.window_handle();

  window.push_notification(
    Notification::new()
      .id::<StartupCrashReportNotificationId>()
      .title("Reviu recovered from a crash")
      .message("Send a report to help the team investigate and fix this crash.")
      .autohide(false)
      .content(move |_, _, _cx| {
        let report_crash_id = report.crash_id.clone();
        let report_for_send = report_for_send.clone();
        let send_label = if sending { "Sending..." } else { "Send report" };
        div()
          .flex()
          .mt_3()
          .gap_2()
          .child(
            Button::new("startup-crash-dismiss")
              .ghost()
              .compact()
              .small()
              .label("Dismiss")
              .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                clear_crash_report_submitting(&report_crash_id);
                let _ = clear_pending_startup_crash_report();
                window.remove_notification::<StartupCrashReportNotificationId>(cx);
              }),
          )
          .child(
            Button::new("startup-crash-send")
              .primary()
              .compact()
              .small()
              .loading(sending)
              .disabled(sending)
              .label(send_label)
              .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                if !mark_crash_report_submitting(&report_for_send.crash_id) {
                  return;
                }
                show_startup_crash_report_notification_with_state(
                  window,
                  report_for_send.clone(),
                  true,
                  cx,
                );
                submit_startup_crash_report(report_for_send.clone(), window_handle, cx);
              }),
          )
          .into_any_element()
      }),
    cx,
  );
}

fn submit_startup_crash_report(
  report: StartupCrashReport,
  window_handle: AnyWindowHandle,
  cx: &mut App,
) {
  let crash_id = report.crash_id.clone();
  let report_for_submission = report.clone();
  let api = WorkspaceApi::global(cx).api.clone();
  cx.spawn(async move |cx| {
    let result = unblock(move || api.submit_crash_report(&report_for_submission)).await;

    let _ = cx.update_window(window_handle, |_, window, cx| match result {
      Ok(()) => {
        clear_crash_report_submitting(&crash_id);
        let _ = clear_pending_startup_crash_report();
        window.remove_notification::<StartupCrashReportNotificationId>(cx);
        window.push_notification(
          Notification::success("Crash report sent successfully. Thank you."),
          cx,
        );
      }
      Err(_) => {
        clear_crash_report_submitting(&crash_id);
        show_startup_crash_report_notification_with_state(window, report.clone(), false, cx);
        window.push_notification(Notification::error("Failed to send crash report"), cx);
      }
    });
  })
  .detach();
}

fn is_crash_report_submitting(crash_id: &str) -> bool {
  crash_report_submission_state()
    .lock()
    .map(|state| state.contains(crash_id))
    .unwrap_or(false)
}

fn mark_crash_report_submitting(crash_id: &str) -> bool {
  let Ok(mut state) = crash_report_submission_state().lock() else {
    return false;
  };

  if state.contains(crash_id) {
    return false;
  }

  state.insert(crash_id.to_string());
  true
}

fn clear_crash_report_submitting(crash_id: &str) {
  if let Ok(mut state) = crash_report_submission_state().lock() {
    state.remove(crash_id);
  }
}

fn persist_startup_crash_report(report: &StartupCrashReport) -> std::io::Result<()> {
  let Some(crash_reports_dir) = crash_reports_dir() else {
    return Ok(());
  };

  fs::create_dir_all(&crash_reports_dir)?;
  let pending_path = crash_reports_dir.join(PENDING_CRASH_REPORT_FILE_NAME);
  let archive_path = crash_reports_dir.join(format!("{}.log", report.crash_id));

  fs::write(&archive_path, report.details_text())?;
  let payload =
    serde_json::to_vec_pretty(report).map_err(|err| std::io::Error::other(err.to_string()))?;
  fs::write(pending_path, payload)?;

  Ok(())
}

fn clear_pending_startup_crash_report() -> std::io::Result<()> {
  let Some(pending_path) = pending_crash_report_path() else {
    return Ok(());
  };

  if pending_path.exists() {
    fs::remove_file(pending_path)?;
  }

  Ok(())
}

fn pending_crash_report_path() -> Option<PathBuf> {
  Some(crash_reports_dir()?.join(PENDING_CRASH_REPORT_FILE_NAME))
}

fn crash_reports_dir() -> Option<PathBuf> {
  #[cfg(test)]
  if let Some(path) = test_crash_reports_dir() {
    return Some(path);
  }

  Some(
    config_dir()?
      .join(AppProfile::current().storage_dir_name())
      .join(CRASH_REPORTS_DIR_NAME),
  )
}

#[cfg(test)]
fn test_crash_reports_dir() -> Option<PathBuf> {
  TEST_CRASH_REPORTS_DIR.with(|slot| slot.borrow().clone())
}

#[cfg(test)]
fn set_test_crash_reports_dir(path: Option<PathBuf>) {
  TEST_CRASH_REPORTS_DIR.with(|slot| {
    *slot.borrow_mut() = path;
  });
}

fn build_crash_id() -> String {
  let millis = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|duration| duration.as_millis())
    .unwrap_or_default();
  format!("crash-{millis}-{}", std::process::id())
}

fn panic_message(info: &PanicHookInfo<'_>) -> String {
  if let Some(message) = info.payload().downcast_ref::<String>() {
    return message.clone();
  }

  if let Some(message) = info.payload().downcast_ref::<&str>() {
    return (*message).to_string();
  }

  "panic payload was not a string".to_string()
}

fn panic_location(info: &PanicHookInfo<'_>) -> Option<String> {
  info.location().map(|location| {
    format!(
      "{}:{}:{}",
      location.file(),
      location.line(),
      location.column()
    )
  })
}

fn current_timestamp_rfc3339() -> String {
  OffsetDateTime::now_utc()
    .format(&Rfc3339)
    .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn trim_multiline(value: &str, max_chars: usize) -> String {
  let normalized = value.trim();
  if normalized.chars().count() <= max_chars {
    return normalized.to_string();
  }

  let mut trimmed = String::new();
  for ch in normalized.chars().take(max_chars.saturating_sub(3)) {
    trimmed.push(ch);
  }
  trimmed.push_str("...");
  trimmed
}

fn app_profile_label(profile: AppProfile) -> &'static str {
  match profile {
    AppProfile::Prod => "prod",
    AppProfile::Dev => "dev",
  }
}

#[cfg(test)]
mod tests {
  use super::{
    StartupCrashReport, clear_pending_startup_crash_report, set_test_crash_reports_dir,
    take_pending_startup_crash_report, trim_multiline,
  };
  use crate::sentry_context::CrashGitContext;
  use std::fs;
  use std::path::PathBuf;
  use std::time::{SystemTime, UNIX_EPOCH};

  fn unique_test_dir() -> PathBuf {
    let suffix = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map(|duration| duration.as_nanos())
      .unwrap_or_default();
    std::env::temp_dir().join(format!("reviu-crash-report-tests-{suffix}"))
  }

  fn sample_report() -> StartupCrashReport {
    StartupCrashReport {
      crash_id: "crash-123".to_string(),
      message: "editor panic".to_string(),
      panic_location: Some("desktop/crates/editor/src/editor.rs:42:7".to_string()),
      backtrace: Some("frame 1\nframe 2".to_string()),
      thread_name: Some("main".to_string()),
      app_version: "0.0.11".to_string(),
      release: Some("reviu@0.0.11".to_string()),
      os: "macos".to_string(),
      arch: "aarch64".to_string(),
      app_profile: "prod".to_string(),
      happened_at: "2026-04-03T10:00:00Z".to_string(),
      pathname: Some("/git".to_string()),
      workspace_page: Some("git".to_string()),
      git_context: Some(CrashGitContext {
        repo_name: Some("reviu".to_string()),
        repo_hash: Some("abc123def456".to_string()),
        selected_file: Some("desktop/crates/editor/src/editor.rs".to_string()),
        branch: Some("main".to_string()),
        sidebar_mode: "changes".to_string(),
        diff_view: "unified".to_string(),
      }),
      github_pr_context: None,
    }
  }

  #[test]
  fn take_pending_startup_crash_report_reads_serialized_payload() {
    let dir = unique_test_dir();
    fs::create_dir_all(&dir).expect("create test crash report dir");
    set_test_crash_reports_dir(Some(dir.clone()));

    let pending_path = dir.join("pending.json");
    fs::write(
      &pending_path,
      serde_json::to_vec_pretty(&sample_report()).expect("serialize crash report"),
    )
    .expect("write pending crash report");

    let report = take_pending_startup_crash_report().expect("pending report");
    assert_eq!(report, sample_report());

    set_test_crash_reports_dir(None);
    let _ = fs::remove_dir_all(dir);
  }

  #[test]
  fn clear_pending_startup_crash_report_removes_marker() {
    let dir = unique_test_dir();
    fs::create_dir_all(&dir).expect("create test crash report dir");
    set_test_crash_reports_dir(Some(dir.clone()));

    let pending_path = dir.join("pending.json");
    fs::write(&pending_path, b"{}").expect("write pending crash report");

    clear_pending_startup_crash_report().expect("clear pending crash report");
    assert!(!pending_path.exists());

    set_test_crash_reports_dir(None);
    let _ = fs::remove_dir_all(dir);
  }

  #[test]
  fn details_text_includes_core_context() {
    let details = sample_report().details_text();

    assert!(details.contains("Reviu Desktop Crash Report"));
    assert!(details.contains("Crash ID: crash-123"));
    assert!(details.contains("Panic location: desktop/crates/editor/src/editor.rs:42:7"));
    assert!(details.contains("UI Context:"));
    assert!(details.contains("Git Context:"));
    assert!(details.contains("Backtrace:"));
  }

  #[test]
  fn trim_multiline_appends_ellipsis_when_limit_is_hit() {
    let trimmed = trim_multiline(&"x".repeat(32), 12);

    assert_eq!(trimmed, "xxxxxxxxx...");
  }
}
