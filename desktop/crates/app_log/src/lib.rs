use std::fmt::{self, Display};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::panic::Location;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

pub use log::{Level, LevelFilter};

/// Rotate the log file when it exceeds this size (1 MB).
const MAX_LOG_SIZE: u64 = 1_024 * 1_024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SinkConfig {
  path: PathBuf,
  old_path: Option<PathBuf>,
  level: LevelFilter,
}

impl SinkConfig {
  pub fn new(path: PathBuf, old_path: Option<PathBuf>, level: LevelFilter) -> Self {
    Self {
      path,
      old_path,
      level,
    }
  }
}

static SINK: OnceLock<Option<SinkConfig>> = OnceLock::new();
static LOGGER: FileLogger = FileLogger;

struct FileLogger;

impl log::Log for FileLogger {
  fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
    let Some(sink) = SINK.get().and_then(Option::as_ref) else {
      return false;
    };
    metadata.level() <= sink.level
  }

  fn log(&self, record: &log::Record<'_>) {
    if !self.enabled(record.metadata()) {
      return;
    }
    let Some(sink) = SINK.get().and_then(Option::as_ref) else {
      return;
    };
    append_record(sink, record.level(), record.target(), record.args());
  }

  fn flush(&self) {}
}

/// Point logging at `path`, or disable it with `None`. Only the first call wins, so
/// logging stays a no-op in binaries and tests that never opt in.
pub fn init(path: Option<PathBuf>) {
  init_with_config(path.map(|path| SinkConfig::new(path, None, LevelFilter::Info)));
}

/// Point logging at a rotated file sink, or disable it with `None`.
pub fn init_with_rotation(path: Option<PathBuf>, old_path: Option<PathBuf>) {
  init_with_rotation_and_level(path, old_path, LevelFilter::Info);
}

pub fn init_with_rotation_and_level(
  path: Option<PathBuf>,
  old_path: Option<PathBuf>,
  level: LevelFilter,
) {
  init_with_config(path.map(|path| SinkConfig::new(path, old_path, level)));
}

fn init_with_config(config: Option<SinkConfig>) {
  let max_level = config
    .as_ref()
    .map(|config| config.level)
    .unwrap_or(LevelFilter::Off);
  if SINK.set(config).is_err() {
    log::warn!("app_log::init called twice, keeping the first sink");
    return;
  }
  if log::set_logger(&LOGGER).is_err() {
    return;
  }
  log::set_max_level(max_level);
}

pub fn active_log_path() -> Option<&'static Path> {
  SINK.get()?.as_ref().map(|sink| sink.path.as_path())
}

pub fn active_old_log_path() -> Option<&'static Path> {
  SINK
    .get()?
    .as_ref()
    .and_then(|sink| sink.old_path.as_deref())
}

pub fn active_level() -> Option<LevelFilter> {
  SINK.get()?.as_ref().map(|sink| sink.level)
}

/// Append an info line to the app log. No-op until `init` opts in.
pub fn log(message: &str) {
  log_target("app_log", message);
}

#[doc(hidden)]
pub fn log_target(target: &str, message: &str) {
  log::info!(target: target, "{message}");
}

/// Format and append an info line to the app log.
#[macro_export]
macro_rules! log {
  ($($arg:tt)*) => { $crate::log_target(module_path!(), &format!($($arg)*)) };
}

fn append_record(sink: &SinkConfig, level: Level, target: &str, message: &dyn Display) {
  append_line(sink, format_args!("{level:<5} {target}: {message}"));
}

fn append_line(sink: &SinkConfig, message: fmt::Arguments<'_>) {
  if let Some(parent) = sink.path.parent()
    && let Err(error) = fs::create_dir_all(parent)
  {
    // Nowhere left to report this, so stderr is the last resort.
    eprintln!("app_log: cannot create {}: {error}", parent.display());
    return;
  }

  if fs::metadata(&sink.path).is_ok_and(|meta| meta.len() > MAX_LOG_SIZE)
    && let Err(error) = rotate(sink)
  {
    eprintln!("app_log: cannot rotate {}: {error}", sink.path.display());
  }

  match OpenOptions::new()
    .create(true)
    .append(true)
    .open(&sink.path)
  {
    Ok(mut file) => {
      if let Err(error) = writeln!(file, "[{}] {message}", timestamp()) {
        eprintln!("app_log: cannot write {}: {error}", sink.path.display());
      }
    }
    Err(error) => eprintln!("app_log: cannot open {}: {error}", sink.path.display()),
  }
}

fn rotate(sink: &SinkConfig) -> std::io::Result<()> {
  match sink.old_path.as_ref() {
    Some(old_path) => {
      if let Some(parent) = old_path.parent() {
        fs::create_dir_all(parent)?;
      }
      if old_path.exists() {
        fs::remove_file(old_path)?;
      }
      if sink.path.exists() {
        fs::rename(&sink.path, old_path)?;
      }
      Ok(())
    }
    None => {
      if sink.path.exists() {
        fs::remove_file(&sink.path)?;
      }
      Ok(())
    }
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
        log::error!(target: "app_log", "{}", error_message(Location::caller(), "", &error));
        None
      }
    }
  }

  #[track_caller]
  fn log_err_context(self, context: &str) -> Option<T> {
    match self {
      Ok(value) => Some(value),
      Err(error) => {
        log::error!(target: "app_log", "{}", error_message(Location::caller(), context, &error));
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
  fn append_writes_a_timestamped_line() {
    let path = scratch_log("app_log_append_test");
    let sink = SinkConfig::new(path.clone(), None, LevelFilter::Info);

    append_line(&sink, format_args!("hello"));

    let contents = fs::read_to_string(&path).expect("log file should exist");
    assert!(contents.contains("hello"), "got {contents:?}");
    assert!(contents.starts_with('['), "got {contents:?}");
  }

  #[test]
  fn append_rotates_an_oversized_file() {
    let path = scratch_log("app_log_rotate_test");
    let old_path = path.with_extension("log.old");
    if let Some(parent) = path.parent() {
      fs::create_dir_all(parent).expect("temp dir should be creatable");
    }
    fs::write(&path, vec![b'x'; (MAX_LOG_SIZE + 1) as usize])
      .expect("seed file should be writable");
    let sink = SinkConfig::new(path.clone(), Some(old_path.clone()), LevelFilter::Info);

    append_line(&sink, format_args!("after rotate"));

    let contents = fs::read_to_string(&path).expect("log file should exist");
    assert!(contents.contains("after rotate"));
    assert!(
      !contents.contains('x'),
      "old contents should be rotated out"
    );
    assert!(old_path.exists(), "old log should keep the rotated file");
  }

  #[test]
  fn log_records_include_level_and_target() {
    let path = scratch_log("app_log_record_test");
    let sink = SinkConfig::new(path.clone(), None, LevelFilter::Info);

    append_record(&sink, Level::Warn, "test_target", &format_args!("careful"));

    let contents = fs::read_to_string(&path).expect("log file should exist");
    assert!(
      contents.contains("WARN  test_target: careful"),
      "got {contents:?}"
    );
  }

  #[test]
  fn error_message_carries_the_call_site() {
    let message = error_message(Location::caller(), "", &"boom");
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

  /// `init` only fires once per process, so everything that needs a live sink shares one test.
  #[test]
  fn the_installed_sink_receives_macro_and_log_err_output() {
    let path = scratch_log("app_log_sink_test");
    init_with_rotation_and_level(Some(path.clone()), None, LevelFilter::Debug);
    assert_eq!(active_log_path(), Some(path.as_path()));
    assert_eq!(active_level(), Some(LevelFilter::Debug));

    let count = 3;
    log!("macro wrote {count} things");
    log::debug!(target: "app_log_test", "debug wrote too");

    let failure: Result<(), String> = Err("disk full".to_string());
    assert!(failure.log_err_context("saving config").is_none());

    let contents = fs::read_to_string(&path).expect("log file should exist");
    assert!(
      contents.contains("INFO  app_log::tests: macro wrote 3 things"),
      "got {contents:?}"
    );
    assert!(
      contents.contains("DEBUG app_log_test: debug wrote too"),
      "got {contents:?}"
    );
    assert!(
      contents.contains("ERROR app_log: ") && contents.contains("saving config: disk full"),
      "got {contents:?}"
    );
    assert!(contents.contains("lib.rs:"), "got {contents:?}");
  }
}
