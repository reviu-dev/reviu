use std::path::PathBuf;

use crate::AppProfile;

const LOGS_DIR_NAME: &str = "logs";
const LOG_FILE_NAME: &str = "reviu.log";
const OLD_LOG_FILE_NAME: &str = "reviu.log.old";
const LOG_ENV: &str = "REVIU_LOG";

/// Point `app_log` at this profile's log file. Call once, before anything worth logging.
pub fn install(profile: AppProfile) {
  let setting = LogSetting::from_env(std::env::var(LOG_ENV).ok().as_deref(), profile);
  let (path, old_path) = match setting {
    LogSetting::Disabled => (None, None),
    LogSetting::Enabled => match log_paths(profile) {
      Some(paths) => (Some(paths.path), Some(paths.old_path)),
      None => (None, None),
    },
  };
  app_log::init_with_rotation(path, old_path);
}

pub fn active_log_path() -> Option<&'static std::path::Path> {
  app_log::active_log_path()
}

pub fn reveal_active_log(cx: &mut gpui::App) -> bool {
  let Some(path) = active_log_path() else {
    return false;
  };
  cx.reveal_path(path);
  true
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogPaths {
  pub path: PathBuf,
  pub old_path: PathBuf,
}

pub fn log_paths(profile: AppProfile) -> Option<LogPaths> {
  let dir = logs_dir(profile)?;
  Some(LogPaths {
    path: dir.join(LOG_FILE_NAME),
    old_path: dir.join(OLD_LOG_FILE_NAME),
  })
}

fn logs_dir(profile: AppProfile) -> Option<PathBuf> {
  if cfg!(target_os = "macos") {
    return Some(
      dirs::home_dir()?
        .join("Library")
        .join("Logs")
        .join(macos_logs_dir_name(profile)),
    );
  }

  if cfg!(target_os = "windows") {
    return Some(
      dirs::data_local_dir()?
        .join(windows_logs_dir_name(profile))
        .join(LOGS_DIR_NAME),
    );
  }

  Some(
    dirs::state_dir()
      .or_else(dirs::data_local_dir)?
      .join(profile.storage_dir_name())
      .join(LOGS_DIR_NAME),
  )
}

fn macos_logs_dir_name(profile: AppProfile) -> &'static str {
  match profile {
    AppProfile::Prod => "Reviu",
    AppProfile::Dev => "Reviu Dev",
  }
}

fn windows_logs_dir_name(profile: AppProfile) -> &'static str {
  match profile {
    AppProfile::Prod => "Reviu",
    AppProfile::Dev => "Reviu Dev",
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LogSetting {
  Enabled,
  Disabled,
}

impl LogSetting {
  fn from_env(env_value: Option<&str>, _profile: AppProfile) -> Self {
    match env_value.map(|value| value.trim().to_ascii_lowercase()) {
      Some(value) if matches!(value.as_str(), "0" | "false" | "off") => Self::Disabled,
      _ => Self::Enabled,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn logging_is_on_in_every_profile_by_default() {
    assert_eq!(
      LogSetting::from_env(None, AppProfile::Dev),
      LogSetting::Enabled
    );
    assert_eq!(
      LogSetting::from_env(None, AppProfile::Prod),
      LogSetting::Enabled
    );
  }

  #[test]
  fn the_env_var_can_disable_logging() {
    assert_eq!(
      LogSetting::from_env(Some("0"), AppProfile::Prod),
      LogSetting::Disabled
    );
    assert_eq!(
      LogSetting::from_env(Some(" false "), AppProfile::Dev),
      LogSetting::Disabled
    );
    assert_eq!(
      LogSetting::from_env(Some("off"), AppProfile::Dev),
      LogSetting::Disabled
    );
  }

  #[test]
  fn the_env_var_accepts_level_like_values_as_enabled() {
    assert_eq!(
      LogSetting::from_env(Some("debug"), AppProfile::Prod),
      LogSetting::Enabled
    );
    assert_eq!(
      LogSetting::from_env(Some("trace"), AppProfile::Prod),
      LogSetting::Enabled
    );
    assert_eq!(
      LogSetting::from_env(Some("1"), AppProfile::Prod),
      LogSetting::Enabled
    );
  }

  #[test]
  fn the_sink_path_uses_os_log_locations() {
    let paths = log_paths(AppProfile::Dev).expect("log dir should exist");
    assert!(paths.path.ends_with(LOG_FILE_NAME), "got {paths:?}");
    assert!(paths.old_path.ends_with(OLD_LOG_FILE_NAME), "got {paths:?}");
    #[cfg(target_os = "macos")]
    assert!(
      paths
        .path
        .to_string_lossy()
        .contains("Library/Logs/Reviu Dev"),
      "got {paths:?}"
    );
    #[cfg(not(target_os = "macos"))]
    assert!(
      paths.path.to_string_lossy().contains(LOGS_DIR_NAME),
      "got {paths:?}"
    );
  }
}
