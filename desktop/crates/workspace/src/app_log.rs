use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use dirs::config_dir;

use crate::AppProfile;

const LOGS_DIR_NAME: &str = "logs";
const LOG_FILE_NAME: &str = "reviu.log";
/// Truncate the log file when it exceeds this size (1 MB).
const MAX_LOG_SIZE: u64 = 1_024 * 1_024;

static ENABLED: OnceLock<bool> = OnceLock::new();

fn is_enabled() -> bool {
  *ENABLED.get_or_init(|| {
    if std::env::var("REVIU_LOG").is_ok_and(|v| matches!(v.as_str(), "1" | "true")) {
      return true;
    }
    AppProfile::current().is_dev()
  })
}

fn log_file_path() -> Option<PathBuf> {
  let dir = config_dir()?
    .join(AppProfile::current().storage_dir_name())
    .join(LOGS_DIR_NAME);
  Some(dir.join(LOG_FILE_NAME))
}

/// Returns the path to the active log file, if logging is enabled.
pub fn active_log_path() -> Option<PathBuf> {
  if !is_enabled() {
    return None;
  }
  log_file_path()
}

/// Append a timestamped line to the log file.
/// No-op in prod unless `REVIU_LOG=1` is set.
pub fn log(message: &str) {
  if !is_enabled() {
    return;
  }

  let Some(path) = log_file_path() else {
    return;
  };

  if let Some(parent) = path.parent() {
    let _ = fs::create_dir_all(parent);
  }

  // Truncate if the file is too large.
  if let Ok(meta) = fs::metadata(&path)
    && meta.len() > MAX_LOG_SIZE
  {
    let _ = fs::remove_file(&path);
  }

  let timestamp = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|d| {
      let secs = d.as_secs();
      let millis = d.subsec_millis();
      format!("{secs}.{millis:03}")
    })
    .unwrap_or_default();

  let line = format!("[{timestamp}] {message}\n");

  let file = OpenOptions::new().create(true).append(true).open(&path);
  if let Ok(mut f) = file {
    let _ = f.write_all(line.as_bytes());
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::fs;

  #[test]
  fn log_file_path_uses_profile_storage_dir() {
    let path = log_file_path().expect("config_dir should exist");
    let storage = AppProfile::current().storage_dir_name();
    assert!(path.to_str().unwrap().contains(storage));
    assert!(path.to_str().unwrap().ends_with("logs/reviu.log"));
  }

  #[test]
  fn log_writes_to_file_in_dev() {
    // In test builds (debug_assertions), AppProfile defaults to Dev so logging is enabled.
    let path = log_file_path().expect("config_dir should exist");
    if let Some(parent) = path.parent() {
      let _ = fs::create_dir_all(parent);
    }

    log("test_log_entry");

    let contents = fs::read_to_string(&path).unwrap_or_default();
    assert!(contents.contains("test_log_entry"));
  }
}
