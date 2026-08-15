use std::sync::{Arc, Mutex};

use gpui::{App, Global};
use smol::unblock;

use crate::api::GithubNotification;
use crate::dock_badge::set_dock_badge;
use crate::github_navigation::{open_pr_target, open_repo_target};
use crate::workspace::WorkspaceApi;

/// Notifications last fetched from GitHub, shared by the sessions sidebar inbox,
/// the navigation badge, the dock badge and the menu bar icon.
#[derive(Clone, Default)]
pub struct GithubNotificationsStore {
  notifications: Arc<Mutex<Vec<GithubNotification>>>,
}

impl Global for GithubNotificationsStore {}

impl GithubNotificationsStore {
  pub fn list(cx: &App) -> Vec<GithubNotification> {
    cx.global::<Self>()
      .notifications
      .lock()
      .map(|notifications| notifications.clone())
      .unwrap_or_default()
  }

  pub fn unread_count(cx: &App) -> usize {
    cx.global::<Self>()
      .notifications
      .lock()
      .map(|notifications| unread_count(&notifications))
      .unwrap_or(0)
  }

  /// Replaces the stored notifications and syncs the dock badge.
  pub fn set(cx: &mut App, notifications: Vec<GithubNotification>) {
    let unread = unread_count(&notifications);
    if let Ok(mut guard) = cx.global::<Self>().notifications.lock() {
      *guard = notifications;
    }
    set_dock_badge(unread);
  }

  pub fn clear(cx: &mut App) {
    Self::set(cx, Vec::new());
  }

  pub fn mark_read(cx: &mut App, thread_id: &str) {
    Self::update(cx, |notifications| {
      for notification in notifications.iter_mut() {
        if notification.id == thread_id {
          notification.unread = false;
        }
      }
    });
  }

  pub fn remove(cx: &mut App, thread_id: &str) {
    Self::update(cx, |notifications| {
      notifications.retain(|notification| notification.id != thread_id);
    });
  }

  fn update(cx: &mut App, edit: impl FnOnce(&mut Vec<GithubNotification>)) {
    let unread = {
      let Ok(mut guard) = cx.global::<Self>().notifications.lock() else {
        return;
      };
      edit(&mut guard);
      unread_count(&guard)
    };
    set_dock_badge(unread);
  }
}

fn unread_count(notifications: &[GithubNotification]) -> usize {
  notifications
    .iter()
    .filter(|notification| notification.unread)
    .count()
}

pub(crate) fn notification_owner_repo(notification: &GithubNotification) -> (String, String) {
  let full_name = &notification.repository.full_name;
  let (owner, repo) = full_name.split_once('/').unwrap_or((full_name, ""));
  (owner.to_string(), repo.to_string())
}

pub(crate) fn extract_number_from_api_url(url: &str) -> Option<u64> {
  url.rsplit('/').next()?.parse().ok()
}

/// Converts a GitHub API subject URL to an HTML URL.
/// e.g. `https://api.github.com/repos/o/r/pulls/1` -> `https://github.com/o/r/pull/1`
pub(crate) fn github_html_url_from_notification(
  full_name: &str,
  subject_type: &str,
  api_url: &str,
) -> String {
  let number = api_url.rsplit('/').next().unwrap_or("");
  match subject_type {
    "PullRequest" => format!("https://github.com/{full_name}/pull/{number}"),
    "Issue" => format!("https://github.com/{full_name}/issues/{number}"),
    "Release" => format!("https://github.com/{full_name}/releases"),
    "Discussion" => format!("https://github.com/{full_name}/discussions"),
    _ => format!("https://github.com/{full_name}"),
  }
}

/// Marks the notification read, then opens its subject: pull requests stay in
/// Reviu, everything else opens on github.com.
pub(crate) fn open_notification(notification: &GithubNotification, cx: &mut App) {
  if notification.unread {
    mark_notification_read(notification.id.clone(), cx);
  }

  let (owner, repo) = notification_owner_repo(notification);
  let subject_number = notification
    .subject
    .url
    .as_deref()
    .and_then(extract_number_from_api_url);

  match notification.subject.subject_type.as_str() {
    "PullRequest" => {
      if let Some(number) = subject_number {
        open_pr_target(owner, repo, number, false, None, cx);
      }
    }
    "Issue" => {
      open_repo_target(
        owner,
        repo,
        Some(ui::CommandPaletteGithubRepoTab::Issues),
        subject_number,
        None,
        cx,
      );
    }
    subject_type => {
      let url = notification
        .subject
        .url
        .as_deref()
        .map(|api_url| {
          github_html_url_from_notification(
            &notification.repository.full_name,
            subject_type,
            api_url,
          )
        })
        .unwrap_or_else(|| format!("https://github.com/{}", notification.repository.full_name));
      cx.open_url(&url);
    }
  }
}

pub(crate) fn mark_notification_read(thread_id: String, cx: &mut App) {
  GithubNotificationsStore::mark_read(cx, &thread_id);
  let api = WorkspaceApi::global(cx).api.clone();
  cx.background_executor()
    .spawn(async move {
      let _ = unblock(move || api.mark_notification_read(&thread_id)).await;
    })
    .detach();
}

pub(crate) fn mark_notification_done(thread_id: String, cx: &mut App) {
  GithubNotificationsStore::remove(cx, &thread_id);
  let api = WorkspaceApi::global(cx).api.clone();
  cx.background_executor()
    .spawn(async move {
      let _ = unblock(move || api.mark_notification_done(&thread_id)).await;
    })
    .detach();
}

#[cfg(test)]
mod tests {
  use super::{
    extract_number_from_api_url, github_html_url_from_notification, notification_owner_repo,
    unread_count,
  };
  use crate::api::{GithubNotification, GithubNotificationRepository, GithubNotificationSubject};

  fn make_notification(id: &str, full_name: &str, unread: bool) -> GithubNotification {
    GithubNotification {
      id: id.to_string(),
      repository: GithubNotificationRepository {
        name: full_name
          .split_once('/')
          .map_or(full_name, |(_, r)| r)
          .to_string(),
        full_name: full_name.to_string(),
        owner: None,
      },
      subject: GithubNotificationSubject {
        title: "Improve parser".to_string(),
        subject_type: "PullRequest".to_string(),
        url: Some("https://api.github.com/repos/acme/widget/pulls/42".to_string()),
        latest_comment_url: None,
      },
      reason: "review_requested".to_string(),
      unread,
      updated_at: "2026-08-15T12:00:00Z".to_string(),
      last_read_at: None,
      url: "https://api.github.com/notifications/threads/1".to_string(),
      subscription_url: "https://api.github.com/notifications/threads/1/subscription".to_string(),
    }
  }

  #[test]
  fn unread_count_only_counts_unread_notifications() {
    let notifications = vec![
      make_notification("1", "acme/widget", true),
      make_notification("2", "acme/widget", false),
      make_notification("3", "acme/portal", true),
    ];

    assert_eq!(unread_count(&notifications), 2);
    assert_eq!(unread_count(&[]), 0);
  }

  #[test]
  fn notification_owner_repo_splits_full_name() {
    let notification = make_notification("1", "acme/widget", true);
    assert_eq!(
      notification_owner_repo(&notification),
      ("acme".to_string(), "widget".to_string())
    );

    let ownerless = make_notification("2", "widget", true);
    assert_eq!(
      notification_owner_repo(&ownerless),
      ("widget".to_string(), String::new())
    );
  }

  #[test]
  fn extract_number_from_api_url_reads_trailing_segment() {
    assert_eq!(
      extract_number_from_api_url("https://api.github.com/repos/acme/widget/pulls/42"),
      Some(42)
    );
    assert_eq!(
      extract_number_from_api_url("https://api.github.com/repos/acme/widget/pulls"),
      None
    );
  }

  #[test]
  fn github_html_url_from_notification_maps_subject_types() {
    let api_url = "https://api.github.com/repos/acme/widget/pulls/42";
    assert_eq!(
      github_html_url_from_notification("acme/widget", "PullRequest", api_url),
      "https://github.com/acme/widget/pull/42"
    );
    assert_eq!(
      github_html_url_from_notification("acme/widget", "Issue", api_url),
      "https://github.com/acme/widget/issues/42"
    );
    assert_eq!(
      github_html_url_from_notification("acme/widget", "Release", api_url),
      "https://github.com/acme/widget/releases"
    );
    assert_eq!(
      github_html_url_from_notification("acme/widget", "Discussion", api_url),
      "https://github.com/acme/widget/discussions"
    );
    assert_eq!(
      github_html_url_from_notification("acme/widget", "CheckSuite", api_url),
      "https://github.com/acme/widget"
    );
  }
}
