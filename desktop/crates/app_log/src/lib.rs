use std::fmt::Display;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::panic::Location;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// Truncate the log file when it exceeds this size (1 MB).
const MAX_LOG_SIZE: u64 = 1_024 * 1_024;

static SINK: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Point logging at `path`, or disable it with `None`. Only the first call wins, so
/// `log` stays a no-op in binaries and tests that never opt in.
pub fn init(path: Option<PathBuf>) {
  if SINK.set(path).is_err() {
    log("app_log::init called twice, keeping the first sink");
  }
}

pub fn active_log_path() -> Option<&'static Path> {
  SINK.get()?.as_deref()
}

/// Append a timestamped line to the log file. No-op until `init` opts in.
pub fn log(message: &str) {
  let Some(path) = active_log_path() else {
    return;
  };
  append(path, message);
}

/// Format and append a line, like `eprintln!` but into the app log.
#[macro_export]
macro_rules! log {
  ($($arg:tt)*) => { $crate::log(&format!($($arg)*)) };
}

fn append(path: &Path, message: &str) {
  if let Some(parent) = path.parent()
    && let Err(error) = fs::create_dir_all(parent)
  {
    // Nowhere left to report this, so stderr is the last resort.
    eprintln!("app_log: cannot create {}: {error}", parent.display());
    return;
  }

  if fs::metadata(path).is_ok_and(|meta| meta.len() > MAX_LOG_SIZE)
    && let Err(error) = fs::remove_file(path)
  {
    eprintln!("app_log: cannot truncate {}: {error}", path.display());
  }

  let line = format!("[{}] {message}\n", timestamp());
  match OpenOptions::new().create(true).append(true).open(path) {
    Ok(mut file) => {
      if let Err(error) = file.write_all(line.as_bytes()) {
        eprintln!("app_log: cannot write {}: {error}", path.display());
      }
    }
    Err(error) => eprintln!("app_log: cannot open {}: {error}", path.display()),
  }
}

fn timestamp() -> String {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|elapsed| format!("{}.{:03}", elapsed.as_secs(), elapsed.subsec_millis()))
    .unwrap_or_default()
}

fn error_message(caller: &Location<'_>, context: &str, error: &dyn Display) -> String {
  let file = caller.file();
  let line = caller.line();
  if context.is_empty() {
    format!("{file}:{line}: {error}")
  } else {
    format!("{file}:{line}: {context}: {error}")
  }
}

pub trait ResultExt<T> {
  /// Log the error with its call site and turn the result into an `Option`.
  fn log_err(self) -> Option<T>;

  /// Same as [`ResultExt::log_err`], with extra context on what was being attempted.
  fn log_err_context(self, context: &str) -> Option<T>;
}

impl<T, E: Display> ResultExt<T> for Result<T, E> {
  #[track_caller]
  fn log_err(self) -> Option<T> {
    match self {
      Ok(value) => Some(value),
      Err(error) => {
        log(&error_message(Location::caller(), "", &error));
        None
      }
    }
  }

  #[track_caller]
  fn log_err_context(self, context: &str) -> Option<T> {
    match self {
      Ok(value) => Some(value),
      Err(error) => {
        log(&error_message(Location::caller(), context, &error));
        None
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn scratch_log(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(name).join("reviu.log");
    if let Some(parent) = path.parent() {
      fs::remove_dir_all(parent).ok();
    }
    path
  }

  #[test]
  fn log_is_a_no_op_without_a_sink() {
    log("dropped on the floor");
    assert!(active_log_path().is_none());
  }

  #[test]
  fn append_writes_a_timestamped_line() {
    let path = scratch_log("app_log_append_test");

    append(&path, "hello");

    let contents = fs::read_to_string(&path).expect("log file should exist");
    assert!(contents.contains("hello"), "got {contents:?}");
    assert!(contents.starts_with('['), "got {contents:?}");
  }

  #[test]
  fn append_truncates_an_oversized_file() {
    let path = scratch_log("app_log_truncate_test");
    if let Some(parent) = path.parent() {
      fs::create_dir_all(parent).expect("temp dir should be creatable");
    }
    fs::write(&path, vec![b'x'; (MAX_LOG_SIZE + 1) as usize])
      .expect("seed file should be writable");

    append(&path, "after truncate");

    let contents = fs::read_to_string(&path).expect("log file should exist");
    assert!(contents.contains("after truncate"));
    assert!(!contents.contains('x'), "old contents should be gone");
  }

  #[test]
  fn error_message_carries_the_call_site() {
    let caller = Location::caller();
    let message = error_message(caller, "", &"boom");
    assert!(message.ends_with(": boom"), "got {message:?}");
    assert!(message.contains("lib.rs"), "got {message:?}");
  }

  #[test]
  fn error_message_prefixes_the_context() {
    let message = error_message(Location::caller(), "saving config", &"boom");
    assert!(
      message.ends_with(": saving config: boom"),
      "got {message:?}"
    );
  }

  #[test]
  fn log_err_passes_the_value_through() {
    let success: Result<u8, String> = Ok(7);
    assert_eq!(success.log_err(), Some(7));

    let failure: Result<u8, String> = Err("boom".to_string());
    assert_eq!(failure.log_err(), None);
    let failure: Result<u8, String> = Err("boom".to_string());
    assert_eq!(failure.log_err_context("while testing"), None);
  }
}
