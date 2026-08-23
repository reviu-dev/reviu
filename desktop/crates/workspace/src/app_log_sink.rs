use std::path::PathBuf;

use crate::AppProfile;

const LOGS_DIR_NAME: &str = "logs";
const LOG_FILE_NAME: &str = "reviu.log";
const LOG_ENV: &str = "REVIU_LOG";

/// Point `app_log` at this profile's log file. Call once, before anything worth logging.
pub fn install(profile: AppProfile) {
  app_log::init(sink_path(profile, std::env::var(LOG_ENV).ok().as_deref()));
}

fn sink_path(profile: AppProfile, env_value: Option<&str>) -> Option<PathBuf> {
  if !is_enabled(profile, env_value) {
    return None;
  }
  Some(
    profile
      .config_dir()?
      .join(LOGS_DIR_NAME)
      .join(LOG_FILE_NAME),
  )
}

fn is_enabled(profile: AppProfile, env_value: Option<&str>) -> bool {
  match env_value.map(|value| value.trim().to_ascii_lowercase()) {
    Some(value) if matches!(value.as_str(), "1" | "true") => true,
    Some(value) if matches!(value.as_str(), "0" | "false") => false,
    _ => profile.is_dev(),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn logging_is_on_in_dev_by_default() {
    assert!(is_enabled(AppProfile::Dev, None));
    assert!(!is_enabled(AppProfile::Prod, None));
  }

  #[test]
  fn the_env_var_overrides_the_profile() {
    assert!(is_enabled(AppProfile::Prod, Some("1")));
    assert!(is_enabled(AppProfile::Prod, Some(" TRUE ")));
    assert!(!is_enabled(AppProfile::Dev, Some("0")));
    assert!(!is_enabled(AppProfile::Dev, Some("false")));
  }

  #[test]
  fn an_unknown_env_value_falls_back_to_the_profile() {
    assert!(is_enabled(AppProfile::Dev, Some("maybe")));
    assert!(!is_enabled(AppProfile::Prod, Some("maybe")));
  }

  #[test]
  fn the_sink_path_lives_under_the_profile_config_dir() {
    let path = sink_path(AppProfile::Dev, Some("1")).expect("config dir should exist");
    assert!(path.ends_with("logs/reviu.log"), "got {path:?}");
    assert!(path.to_string_lossy().contains("reviu.dev"), "got {path:?}");
  }

  #[test]
  fn there_is_no_sink_when_logging_is_off() {
    assert!(sink_path(AppProfile::Prod, None).is_none());
  }
}
