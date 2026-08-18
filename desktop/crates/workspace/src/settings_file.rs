//! `AppSettings` persisted as a plain JSON file; a missing, unknown or
//! mistyped field falls back to its default instead of failing the load.

use std::cell::RefCell;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::AppProfile;
use crate::config::{AppSettings, CloneProtocol, ConfigStore};
use crate::sentry_context::capture_unexpected_error;

const SETTINGS_FILE_NAME: &str = "settings.json";

thread_local! {
  static STARTUP_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Why the settings file could not be read, kept for a startup notification.
pub(crate) fn take_startup_error() -> Option<String> {
  STARTUP_ERROR.with(|slot| slot.borrow_mut().take())
}

fn record_error(op: &'static str, err: &dyn std::error::Error, notify: bool) {
  eprintln!("{op}: {err}");
  capture_unexpected_error(op, err, Default::default());
  if notify {
    STARTUP_ERROR.with(|slot| *slot.borrow_mut() = Some(err.to_string()));
  }
}

fn settings_file_path() -> PathBuf {
  #[cfg(test)]
  {
    return ConfigStore::test_db_path()
      .expect("tests must set a ConfigStore test db path before touching settings")
      .with_extension("settings.json");
  }

  #[cfg(not(test))]
  settings_file_path_for_profile(dirs::config_dir(), AppProfile::current())
}

#[cfg_attr(test, allow(dead_code))]
fn settings_file_path_for_profile(base: Option<PathBuf>, profile: AppProfile) -> PathBuf {
  match base {
    Some(base) => base
      .join(profile.storage_dir_name())
      .join(SETTINGS_FILE_NAME),
    None => PathBuf::from(match profile {
      AppProfile::Prod => "settings.json",
      AppProfile::Dev => "settings.dev.json",
    }),
  }
}

/// On-disk shape: every field optional so a file written by any version resolves.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct SettingsDoc {
  #[serde(deserialize_with = "lenient")]
  auto_switch_theme: Option<bool>,
  #[serde(deserialize_with = "lenient")]
  dark_mode: Option<bool>,
  #[serde(deserialize_with = "lenient")]
  indent_rainbow: Option<bool>,
  #[serde(deserialize_with = "lenient")]
  font_size: Option<f32>,
  #[serde(deserialize_with = "lenient")]
  git_unified_file_view: Option<bool>,
  #[serde(deserialize_with = "lenient")]
  split_diff_view: Option<bool>,
  #[serde(deserialize_with = "lenient")]
  hide_whitespace: Option<bool>,
  #[serde(deserialize_with = "lenient")]
  clone_protocol: Option<CloneProtocol>,
  #[serde(deserialize_with = "lenient")]
  menu_bar_icon: Option<bool>,
  #[serde(deserialize_with = "lenient")]
  analytics_enabled: Option<bool>,
}

/// A field of the wrong type falls back to its default instead of failing the file.
fn lenient<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
  D: serde::Deserializer<'de>,
  T: serde::de::DeserializeOwned,
{
  let value = serde_json::Value::deserialize(deserializer)?;
  Ok(T::deserialize(value).ok())
}

impl SettingsDoc {
  fn resolve(self) -> AppSettings {
    let defaults = AppSettings::default();
    AppSettings {
      auto_switch_theme: self.auto_switch_theme.unwrap_or(defaults.auto_switch_theme),
      dark_mode: self.dark_mode.unwrap_or(defaults.dark_mode),
      indent_rainbow: self.indent_rainbow.unwrap_or(defaults.indent_rainbow),
      font_size: self.font_size.unwrap_or(defaults.font_size),
      git_unified_file_view: self
        .git_unified_file_view
        .unwrap_or(defaults.git_unified_file_view),
      split_diff_view: self.split_diff_view.unwrap_or(defaults.split_diff_view),
      hide_whitespace: self.hide_whitespace.unwrap_or(defaults.hide_whitespace),
      clone_protocol: self.clone_protocol.unwrap_or(defaults.clone_protocol),
      menu_bar_icon: self.menu_bar_icon.unwrap_or(defaults.menu_bar_icon),
      analytics_enabled: self.analytics_enabled.unwrap_or(defaults.analytics_enabled),
    }
  }
}

impl From<AppSettings> for SettingsDoc {
  fn from(settings: AppSettings) -> Self {
    Self {
      auto_switch_theme: Some(settings.auto_switch_theme),
      dark_mode: Some(settings.dark_mode),
      indent_rainbow: Some(settings.indent_rainbow),
      font_size: Some(settings.font_size),
      git_unified_file_view: Some(settings.git_unified_file_view),
      split_diff_view: Some(settings.split_diff_view),
      hide_whitespace: Some(settings.hide_whitespace),
      clone_protocol: Some(settings.clone_protocol),
      menu_bar_icon: Some(settings.menu_bar_icon),
      analytics_enabled: Some(settings.analytics_enabled),
    }
  }
}

pub(crate) fn load() -> AppSettings {
  let path = settings_file_path();
  match std::fs::read_to_string(&path) {
    Ok(raw) => match serde_json::from_str::<SettingsDoc>(&raw) {
      Ok(doc) => doc.resolve(),
      Err(err) => {
        record_error("settings.parse", &err, true);
        AppSettings::default()
      }
    },
    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
      // One-time import: the file takes over from the sqlite columns.
      let settings = ConfigStore::load_app_settings_from_db();
      persist(settings);
      settings
    }
    Err(err) => {
      record_error("settings.read", &err, true);
      AppSettings::default()
    }
  }
}

pub(crate) fn persist(settings: AppSettings) {
  let path = settings_file_path();
  if let Some(parent) = path.parent()
    && let Err(err) = std::fs::create_dir_all(parent)
  {
    record_error("settings.write", &err, false);
    return;
  }

  let json = match serde_json::to_string_pretty(&SettingsDoc::from(settings)) {
    Ok(json) => json,
    Err(err) => {
      record_error("settings.serialize", &err, false);
      return;
    }
  };

  // A crash mid-write must not leave a truncated settings file.
  let tmp = path.with_extension("json.tmp");
  let written =
    std::fs::write(&tmp, format!("{json}\n")).and_then(|()| std::fs::rename(&tmp, &path));
  if let Err(err) = written {
    record_error("settings.write", &err, false);
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::fs;
  use std::sync::atomic::{AtomicU64, Ordering};

  fn setup(label: &str) -> PathBuf {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let db_path = std::env::temp_dir().join(format!(
      "reviu-settings-file-{label}-{}-{id}.sqlite",
      std::process::id()
    ));
    let _ = fs::remove_file(&db_path);
    ConfigStore::set_test_db_path(Some(db_path));
    let path = settings_file_path();
    let _ = fs::remove_file(&path);
    path
  }

  fn teardown() {
    ConfigStore::set_test_db_path(None);
  }

  #[test]
  fn settings_round_trip_through_the_file() {
    let path = setup("round-trip");

    let settings = AppSettings {
      auto_switch_theme: false,
      dark_mode: true,
      indent_rainbow: true,
      font_size: 20.0,
      git_unified_file_view: true,
      split_diff_view: true,
      hide_whitespace: true,
      clone_protocol: CloneProtocol::Ssh,
      menu_bar_icon: false,
      analytics_enabled: false,
    };
    persist(settings);

    assert!(path.exists());
    assert_eq!(load(), settings);
    assert!(take_startup_error().is_none());

    teardown();
  }

  #[test]
  fn a_missing_field_takes_its_default_and_the_others_survive() {
    let path = setup("missing-field");
    fs::write(&path, r#"{ "dark_mode": true, "font_size": 20.0 }"#).expect("write file");

    let loaded = load();
    assert!(loaded.dark_mode);
    assert_eq!(loaded.font_size, 20.0);
    assert!(
      loaded.auto_switch_theme,
      "absent field must take its default"
    );
    assert!(loaded.menu_bar_icon);
    assert!(take_startup_error().is_none());

    teardown();
  }

  #[test]
  fn an_unknown_field_is_ignored() {
    let path = setup("unknown-field");
    fs::write(&path, r#"{ "dark_mode": true, "from_the_future": 5 }"#).expect("write file");

    let loaded = load();
    assert!(loaded.dark_mode);
    assert!(take_startup_error().is_none());

    teardown();
  }

  #[test]
  fn a_mistyped_field_falls_back_alone() {
    let path = setup("mistyped-field");
    fs::write(
      &path,
      r#"{ "font_size": "big", "clone_protocol": "gopher", "hide_whitespace": true }"#,
    )
    .expect("write file");

    let loaded = load();
    assert_eq!(loaded.font_size, 16.0);
    assert_eq!(loaded.clone_protocol, CloneProtocol::Https);
    assert!(loaded.hide_whitespace, "valid fields must survive");
    assert!(take_startup_error().is_none());

    teardown();
  }

  #[test]
  fn a_corrupted_file_falls_back_to_defaults_and_is_reported() {
    let path = setup("corrupted");
    fs::write(&path, "{ not json").expect("write file");

    assert_eq!(load(), AppSettings::default());
    assert!(take_startup_error().is_some());

    teardown();
  }

  #[test]
  fn settings_file_path_uses_profile_namespace() {
    let base = PathBuf::from("/tmp/reviu-config");

    assert_eq!(
      settings_file_path_for_profile(Some(base.clone()), AppProfile::Prod),
      PathBuf::from("/tmp/reviu-config/reviu/settings.json")
    );
    assert_eq!(
      settings_file_path_for_profile(Some(base), AppProfile::Dev),
      PathBuf::from("/tmp/reviu-config/reviu.dev/settings.json")
    );
    assert_eq!(
      settings_file_path_for_profile(None, AppProfile::Prod),
      PathBuf::from("settings.json")
    );
    assert_eq!(
      settings_file_path_for_profile(None, AppProfile::Dev),
      PathBuf::from("settings.dev.json")
    );
  }
}
