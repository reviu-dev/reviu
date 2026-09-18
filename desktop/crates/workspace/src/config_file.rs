use std::collections::HashSet;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result, bail};
use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};
use serde_json::{Map, Value};

pub(crate) fn edit_json(
  path: &Path,
  edit: impl FnOnce(&mut Map<String, Value>) -> Result<()>,
) -> Result<String> {
  let original = match std::fs::read_to_string(path) {
    Ok(text) => Some(text),
    Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
    Err(error) => return Err(error.into()),
  };
  let mut document = serde_json::from_str(original.as_deref().unwrap_or("{}"))?;
  edit(&mut document)?;
  let text = format!("{}\n", serde_json::to_string_pretty(&document)?);
  write_atomic(path, original.as_deref(), &text)?;
  Ok(text)
}

fn write_atomic(path: &Path, original: Option<&str>, text: &str) -> Result<()> {
  let target = match std::fs::canonicalize(path) {
    Ok(target) => target,
    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
      // A dangling dotfiles link must not be replaced with a regular file.
      if std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_symlink()) {
        bail!(
          "The configuration symlink has no target: {}",
          path.display()
        );
      }
      path.to_path_buf()
    }
    Err(error) => return Err(error.into()),
  };
  let parent = target
    .parent()
    .context("Configuration file has no parent")?;
  std::fs::create_dir_all(parent)?;
  let temporary = parent.join(format!(".reviu-{}.tmp", uuid::Uuid::new_v4()));
  let result = (|| {
    let mut file = std::fs::OpenOptions::new()
      .write(true)
      .create_new(true)
      .open(&temporary)?;
    if let Ok(metadata) = std::fs::metadata(&target) {
      file.set_permissions(metadata.permissions())?;
    }
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    drop(file);
    let current = match std::fs::read_to_string(path) {
      Ok(current) => Some(current),
      Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
      Err(error) => return Err(error.into()),
    };
    if current.as_deref() != original {
      bail!("Configuration changed while saving; please try again");
    }
    if original.is_some() && std::fs::canonicalize(path)? != target {
      bail!("Configuration symlink changed while saving; please try again");
    }
    std::fs::rename(&temporary, &target)?;
    Ok(())
  })();
  if result.is_err()
    && let Err(error) = std::fs::remove_file(&temporary)
    && error.kind() != std::io::ErrorKind::NotFound
  {
    log::warn!("Could not remove temporary configuration file: {error}");
  }
  result
}

pub(crate) struct ConfigMonitor {
  watcher: RecommendedWatcher,
  directories: HashSet<PathBuf>,
  paths: Arc<Mutex<HashSet<PathBuf>>>,
}

impl ConfigMonitor {
  pub(crate) fn new(sender: async_channel::Sender<()>) -> Result<Self> {
    let paths = Arc::new(Mutex::new(HashSet::new()));
    let watched_paths = paths.clone();
    let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
      match event {
        Ok(event) => {
          if matches!(event.kind, notify::EventKind::Access(_)) {
            return;
          }
          if !event.need_rescan()
            && let Ok(paths) = watched_paths.lock()
            && !event.paths.iter().any(|path| paths.contains(path))
          {
            return;
          }
        }
        Err(error) => log::warn!("Configuration watcher: {error}"),
      }
      match sender.try_send(()) {
        Ok(())
        | Err(async_channel::TrySendError::Full(_) | async_channel::TrySendError::Closed(_)) => {}
      }
    })?;
    Ok(Self {
      watcher,
      directories: HashSet::new(),
      paths,
    })
  }

  pub(crate) fn refresh(&mut self, paths: &[PathBuf]) -> Result<()> {
    let mut directories = HashSet::new();
    let mut watched_paths = HashSet::new();
    let mut pending = paths.to_vec();
    let mut visited = HashSet::new();
    while let Some(path) = pending.pop() {
      if !visited.insert(path.clone()) {
        continue;
      }
      if visited.len() > 64 {
        bail!("Too many configuration symlinks");
      }
      let path = &path;
      watched_paths.extend(path.ancestors().map(Path::to_path_buf));
      if let Some(parent) = path.ancestors().skip(1).find(|parent| parent.is_dir()) {
        let relative = path.strip_prefix(parent)?;
        let parent = std::fs::canonicalize(parent)?;
        let resolved = parent.join(relative);
        watched_paths.extend(resolved.ancestors().map(Path::to_path_buf));
        watched_paths.extend(parent.ancestors().map(Path::to_path_buf));
        if let Some(ancestor) = parent.parent() {
          directories.insert(ancestor.to_path_buf());
        }
        directories.insert(parent);
      }
      if let Ok(target) = std::fs::canonicalize(path)
        && let Some(parent) = target.parent()
      {
        watched_paths.extend(target.ancestors().map(Path::to_path_buf));
        directories.insert(parent.to_path_buf());
      }
      for ancestor in path.ancestors() {
        if std::fs::symlink_metadata(ancestor).is_ok_and(|metadata| metadata.is_symlink())
          && let Some(parent) = ancestor.parent()
        {
          let parent = std::fs::canonicalize(parent)?;
          if let Some(name) = ancestor.file_name() {
            watched_paths.insert(parent.join(name));
          }
          let target = parent.join(std::fs::read_link(ancestor)?);
          pending.push(target.join(path.strip_prefix(ancestor)?));
          directories.insert(parent);
        }
      }
    }
    *self
      .paths
      .lock()
      .map_err(|_| anyhow::anyhow!("Configuration watcher lock poisoned"))? = watched_paths;
    for directory in directories.difference(&self.directories) {
      self
        .watcher
        .watch(directory, RecursiveMode::NonRecursive)
        .with_context(|| format!("Could not watch {}", directory.display()))?;
    }
    for directory in self.directories.difference(&directories) {
      if let Err(error) = self.watcher.unwatch(directory)
        && !matches!(
          error.kind,
          notify::ErrorKind::WatchNotFound | notify::ErrorKind::PathNotFound
        )
      {
        return Err(error.into());
      }
    }
    self.directories = directories;
    Ok(())
  }
}

#[cfg(test)]
pub(crate) struct TestConfig(pub(crate) PathBuf);

#[cfg(test)]
impl TestConfig {
  pub(crate) fn new() -> Self {
    let directory =
      std::env::temp_dir().join(format!("reviu-config-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).expect("create test config directory");
    crate::config::ConfigStore::set_test_db_path(Some(directory.join("config.sqlite")));
    Self(directory)
  }
}

#[cfg(test)]
impl Drop for TestConfig {
  fn drop(&mut self) {
    crate::config::ConfigStore::set_test_db_path(None);
    std::fs::remove_dir_all(&self.0).expect("remove test config directory");
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn directory() -> PathBuf {
    let directory = std::env::temp_dir().join(format!("reviu-config-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).expect("create directory");
    directory
  }

  fn monitor(
    paths: &[PathBuf],
  ) -> (
    ConfigMonitor,
    std::sync::mpsc::Receiver<()>,
    std::thread::JoinHandle<()>,
  ) {
    let (sender, receiver) = async_channel::bounded(1);
    let mut monitor = ConfigMonitor::new(sender).expect("monitor");
    monitor.refresh(paths).expect("watch paths");
    let (events, received) = std::sync::mpsc::channel();
    let bridge = std::thread::spawn(move || {
      while receiver.recv_blocking().is_ok() {
        if events.send(()).is_err() {
          break;
        }
      }
    });
    (monitor, received, bridge)
  }

  fn settle(events: &std::sync::mpsc::Receiver<()>) {
    while events
      .recv_timeout(std::time::Duration::from_millis(200))
      .is_ok()
    {}
  }

  fn changed(events: &std::sync::mpsc::Receiver<()>) {
    events
      .recv_timeout(std::time::Duration::from_secs(5))
      .expect("filesystem change delivered");
  }

  #[test]
  fn watcher_survives_atomic_replacements_deletion_and_recreation() {
    let directory = directory();
    let path = directory.join("settings.json");
    let paths = [path.clone()];
    let (mut monitor, events, bridge) = monitor(&paths);
    for value in ["{}", r#"{"dark_mode":true}"#, r#"{"dark_mode":false}"#] {
      settle(&events);
      let temporary = directory.join("editor-save");
      std::fs::write(&temporary, value).expect("write");
      std::fs::rename(&temporary, &path).expect("atomic replace");
      changed(&events);
      monitor.refresh(&paths).expect("refresh");
      assert_eq!(std::fs::read_to_string(&path).expect("read"), value);
    }
    settle(&events);
    std::fs::remove_file(&path).expect("remove");
    changed(&events);
    monitor.refresh(&paths).expect("refresh missing file");
    settle(&events);
    std::fs::write(&path, "{}").expect("recreate");
    changed(&events);
    drop(monitor);
    bridge.join().expect("bridge");
    std::fs::remove_dir_all(directory).expect("cleanup");
  }

  #[cfg(unix)]
  #[test]
  fn watcher_follows_file_and_parent_symlinks_and_retargets() {
    let directory = directory();
    let first = directory.join("first");
    let second = directory.join("second");
    std::fs::create_dir(&first).expect("mkdir");
    std::fs::create_dir(&second).expect("mkdir");
    for target in [&first, &second] {
      std::fs::write(target.join("settings.json"), "{}").expect("write");
    }
    let link = directory.join("config");
    std::os::unix::fs::symlink(&first, &link).expect("parent link");
    let file_link = directory.join("settings.json");
    std::os::unix::fs::symlink(link.join("settings.json"), &file_link).expect("file link");
    let paths = [file_link.clone()];
    let (mut monitor, events, bridge) = monitor(&paths);
    settle(&events);
    std::fs::write(first.join("settings.json"), r#"{"font_size":18}"#).expect("external edit");
    changed(&events);
    assert!(
      std::fs::read_to_string(&file_link)
        .expect("read")
        .contains("18")
    );
    settle(&events);
    std::fs::remove_file(&link).expect("unlink");
    std::os::unix::fs::symlink(&second, &link).expect("retarget");
    changed(&events);
    monitor.refresh(&paths).expect("follow new target");
    settle(&events);
    std::fs::write(second.join("settings.json"), r#"{"font_size":22}"#).expect("external edit");
    changed(&events);
    assert!(
      std::fs::read_to_string(&file_link)
        .expect("read")
        .contains("22")
    );
    drop(monitor);
    bridge.join().expect("bridge");
    std::fs::remove_dir_all(directory).expect("cleanup");
  }

  #[test]
  fn editing_preserves_other_fields_and_refuses_invalid_json() {
    let directory = directory();
    let path = directory.join("settings.json");
    std::fs::write(&path, r#"{"external": true}"#).expect("write");
    edit_json(&path, |document| {
      document.insert("local".into(), true.into());
      Ok(())
    })
    .expect("edit");
    let document: Value =
      serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("parse");
    assert_eq!(document["external"], true);
    std::fs::write(&path, "{").expect("write");
    assert!(edit_json(&path, |_| Ok(())).is_err());
    assert_eq!(std::fs::read_to_string(&path).expect("read"), "{");
    std::fs::remove_dir_all(directory).expect("cleanup");
  }

  #[test]
  fn concurrent_changes_are_not_overwritten() {
    let directory = directory();
    let path = directory.join("settings.json");
    std::fs::write(&path, "{}").expect("write");
    assert!(
      edit_json(&path, |_| {
        std::fs::write(&path, r#"{"external":true}"#)?;
        Ok(())
      })
      .is_err()
    );
    assert_eq!(
      std::fs::read_to_string(&path).expect("read"),
      r#"{"external":true}"#
    );
    std::fs::remove_dir_all(directory).expect("cleanup");
  }

  #[cfg(unix)]
  #[test]
  fn saving_preserves_file_and_directory_symlinks() {
    let directory = directory();
    let target = directory.join("dotfiles");
    std::fs::create_dir(&target).expect("mkdir");
    let target_file = target.join("settings.json");
    std::fs::write(&target_file, "{}").expect("write");
    let link = directory.join("settings.json");
    std::os::unix::fs::symlink("dotfiles/settings.json", &link).expect("link");
    std::os::unix::fs::symlink("dotfiles", directory.join("config")).expect("link dir");
    for path in [&link, &directory.join("config/settings.json")] {
      edit_json(path, |document| {
        document.insert("enabled".into(), true.into());
        Ok(())
      })
      .expect("save");
    }
    assert!(
      std::fs::symlink_metadata(link)
        .expect("metadata")
        .is_symlink()
    );
    assert!(
      std::fs::read_to_string(target_file)
        .expect("read")
        .contains("enabled")
    );
    std::fs::remove_dir_all(directory).expect("cleanup");
  }
}
