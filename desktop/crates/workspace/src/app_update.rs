use std::sync::{Arc, Mutex};

use gpui::{App, Global};

use crate::config::ConfigStore;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AvailableAppUpdate {
  pub latest_version: String,
  pub download_url: String,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct AppUpdateNotificationId;

#[derive(Clone, Default)]
struct AppUpdateState {
  available_update: Option<AvailableAppUpdate>,
}

#[derive(Clone)]
pub struct AppUpdateStore {
  state: Arc<Mutex<AppUpdateState>>,
}

impl Default for AppUpdateStore {
  fn default() -> Self {
    Self {
      state: Arc::new(Mutex::new(AppUpdateState::default())),
    }
  }
}

impl Global for AppUpdateStore {}

impl AppUpdateStore {
  pub fn try_available_update(cx: &App) -> Option<AvailableAppUpdate> {
    let store = cx.try_global::<Self>()?;
    store
      .state
      .lock()
      .ok()
      .and_then(|state| state.available_update.clone())
  }

  pub fn set_available_update(cx: &mut App, update: Option<AvailableAppUpdate>) {
    let Some(store) = cx.try_global::<Self>() else {
      return;
    };
    if let Ok(mut state) = store.state.lock() {
      state.available_update = update;
    }
    cx.refresh_windows();
  }

  pub fn clear_available_update(cx: &mut App) {
    Self::set_available_update(cx, None);
  }

  pub fn apply_download_action(download_url: &str, latest_version: &str, cx: &mut App) {
    cx.open_url(download_url);
    ConfigStore::persist_simulated_app_version(Some(latest_version));
    Self::clear_available_update(cx);
  }
}

pub fn normalize_semver(value: &str) -> Option<String> {
  let normalized = value.trim().strip_prefix('v').unwrap_or(value.trim());
  let mut parts = normalized.split('.');
  let major: u64 = parts.next()?.parse().ok()?;
  let minor: u64 = parts.next()?.parse().ok()?;
  let patch: u64 = parts.next()?.parse().ok()?;
  if parts.next().is_some() {
    return None;
  }
  Some(format!("{major}.{minor}.{patch}"))
}

fn parse_semver_tuple(value: &str) -> Option<(u64, u64, u64)> {
  let normalized = normalize_semver(value)?;
  let mut parts = normalized.split('.');
  let major: u64 = parts.next()?.parse().ok()?;
  let minor: u64 = parts.next()?.parse().ok()?;
  let patch: u64 = parts.next()?.parse().ok()?;
  Some((major, minor, patch))
}

pub fn effective_current_version(base_version: &str, simulated_version: Option<&str>) -> String {
  let normalized_base = normalize_semver(base_version).unwrap_or_else(|| base_version.to_string());
  let Some(simulated_version) = simulated_version else {
    return normalized_base;
  };

  let Some(base_tuple) = parse_semver_tuple(&normalized_base) else {
    return normalized_base;
  };
  let Some(simulated_tuple) = parse_semver_tuple(simulated_version) else {
    return normalized_base;
  };

  if simulated_tuple > base_tuple {
    normalize_semver(simulated_version).unwrap_or(normalized_base)
  } else {
    normalized_base
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use gpui::TestAppContext;
  use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
  };

  fn unique_test_db_path(label: &str) -> PathBuf {
    static NEXT_DB_ID: AtomicU64 = AtomicU64::new(1);
    let id = NEXT_DB_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
      "reviu-app-update-{label}-{}-{id}.sqlite",
      std::process::id()
    ))
  }

  #[test]
  fn effective_current_version_uses_simulated_only_when_newer() {
    assert_eq!(
      effective_current_version("0.1.0", Some("0.2.0")),
      "0.2.0".to_string()
    );
    assert_eq!(
      effective_current_version("0.2.0", Some("0.1.0")),
      "0.2.0".to_string()
    );
    assert_eq!(
      effective_current_version("v0.1.0", Some("v0.1.0")),
      "0.1.0".to_string()
    );
    assert_eq!(
      effective_current_version("0.1.0", Some("invalid")),
      "0.1.0".to_string()
    );
  }

  #[gpui::test]
  fn apply_download_action_persists_simulated_version_and_clears_state(cx: &mut TestAppContext) {
    let db_path = unique_test_db_path("apply-download");
    let _ = fs::remove_file(&db_path);
    ConfigStore::set_test_db_path(Some(db_path));

    cx.update(|cx| {
      cx.set_global(AppUpdateStore::default());
      AppUpdateStore::set_available_update(
        cx,
        Some(AvailableAppUpdate {
          latest_version: "0.2.0".to_string(),
          download_url: "https://reviu.dev/downloads/latest".to_string(),
        }),
      );

      AppUpdateStore::apply_download_action("https://reviu.dev/downloads/latest", "0.2.0", cx);

      assert!(AppUpdateStore::try_available_update(cx).is_none());
    });

    assert_eq!(
      ConfigStore::load_simulated_app_version(),
      Some("0.2.0".to_string())
    );

    ConfigStore::set_test_db_path(None);
  }
}
