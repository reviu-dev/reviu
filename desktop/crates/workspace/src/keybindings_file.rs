//! Shortcut overrides persisted as a flat JSON map `shortcut_id -> keystroke`.
//! Unknown ids and invalid keystrokes are skipped on read but preserved on
//! write, so a file written by another version survives round-trips.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use gpui::Keystroke;

use crate::AppProfile;
use crate::config::ConfigStore;
use crate::sentry_context::capture_unexpected_error;
use crate::shortcuts::ShortcutId;

const KEYBINDINGS_FILE_NAME: &str = "keybindings.json";

type Document = BTreeMap<String, serde_json::Value>;

thread_local! {
  static STARTUP_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Why the keybindings file could not be read, kept for a startup notification.
pub(crate) fn take_startup_error() -> Option<String> {
  STARTUP_ERROR.with(|slot| slot.borrow_mut().take())
}

fn record_error(op: &'static str, err: &dyn std::error::Error, notify: bool) {
  log::warn!("{op}: {err}");
  capture_unexpected_error(op, err, Default::default());
  if notify {
    STARTUP_ERROR.with(|slot| *slot.borrow_mut() = Some(err.to_string()));
  }
}

fn keybindings_file_path() -> PathBuf {
  #[cfg(test)]
  {
    ConfigStore::test_db_path()
      .expect("tests must set a ConfigStore test db path before touching keybindings")
      .with_extension("keybindings.json")
  }

  #[cfg(not(test))]
  keybindings_file_path_for_profile(dirs::config_dir(), AppProfile::current())
}

#[cfg_attr(test, allow(dead_code))]
fn keybindings_file_path_for_profile(base: Option<PathBuf>, profile: AppProfile) -> PathBuf {
  match base {
    Some(base) => base
      .join(profile.storage_dir_name())
      .join(KEYBINDINGS_FILE_NAME),
    None => PathBuf::from(match profile {
      AppProfile::Prod => "keybindings.json",
      AppProfile::Dev => "keybindings.dev.json",
    }),
  }
}

fn overrides_from_document(document: &Document) -> HashMap<ShortcutId, String> {
  let mut overrides = HashMap::new();
  for (key, value) in document {
    let Some(shortcut_id) = ShortcutId::from_storage_key(key) else {
      continue;
    };
    let Some(keystroke) = value.as_str() else {
      continue;
    };
    if Keystroke::parse(keystroke).is_err() {
      continue;
    }
    overrides.insert(shortcut_id, keystroke.to_string());
  }
  overrides
}

fn document_from_overrides(overrides: &HashMap<ShortcutId, String>) -> Document {
  overrides
    .iter()
    .map(|(id, keystroke)| (id.storage_key().to_string(), keystroke.clone().into()))
    .collect()
}

pub(crate) fn load() -> HashMap<ShortcutId, String> {
  let path = keybindings_file_path();
  match std::fs::read_to_string(&path) {
    Ok(raw) => match serde_json::from_str::<Document>(&raw) {
      Ok(document) => overrides_from_document(&document),
      Err(err) => {
        record_error("keybindings.parse", &err, true);
        HashMap::new()
      }
    },
    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
      // One-time import: the file takes over from the sqlite rows.
      let overrides = ConfigStore::load_shortcut_overrides_from_db();
      write_document(&document_from_overrides(&overrides));
      overrides
    }
    Err(err) => {
      record_error("keybindings.read", &err, true);
      HashMap::new()
    }
  }
}

pub(crate) fn set(shortcut_id: ShortcutId, keystroke: &str) {
  edit_document(|document| {
    document.insert(shortcut_id.storage_key().to_string(), keystroke.into());
  });
}

pub(crate) fn remove(shortcut_id: ShortcutId) {
  edit_document(|document| {
    document.remove(shortcut_id.storage_key());
  });
}

/// Edits only the touched key so entries this build does not know survive.
/// A file that no longer parses is left alone rather than clobbered.
fn edit_document(edit: impl FnOnce(&mut Document)) {
  let path = keybindings_file_path();
  let mut document = match std::fs::read_to_string(&path) {
    Ok(raw) => match serde_json::from_str::<Document>(&raw) {
      Ok(document) => document,
      Err(err) => {
        record_error("keybindings.parse", &err, false);
        return;
      }
    },
    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
      document_from_overrides(&ConfigStore::load_shortcut_overrides_from_db())
    }
    Err(err) => {
      record_error("keybindings.read", &err, false);
      return;
    }
  };

  edit(&mut document);
  write_document(&document);
}

fn write_document(document: &Document) {
  let path = keybindings_file_path();
  if let Some(parent) = path.parent()
    && let Err(err) = std::fs::create_dir_all(parent)
  {
    record_error("keybindings.write", &err, false);
    return;
  }

  let json = match serde_json::to_string_pretty(document) {
    Ok(json) => json,
    Err(err) => {
      record_error("keybindings.serialize", &err, false);
      return;
    }
  };

  // A crash mid-write must not leave a truncated keybindings file.
  let tmp = path.with_extension("json.tmp");
  let written =
    std::fs::write(&tmp, format!("{json}\n")).and_then(|()| std::fs::rename(&tmp, &path));
  if let Err(err) = written {
    record_error("keybindings.write", &err, false);
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
      "reviu-keybindings-file-{label}-{}-{id}.sqlite",
      std::process::id()
    ));
    let _ = fs::remove_file(&db_path);
    ConfigStore::set_test_db_path(Some(db_path));
    let path = keybindings_file_path();
    let _ = fs::remove_file(&path);
    path
  }

  fn teardown() {
    ConfigStore::set_test_db_path(None);
  }

  #[test]
  fn a_fresh_install_creates_the_file_empty() {
    let path = setup("fresh-install");

    assert!(load().is_empty());
    assert!(path.exists(), "first load must stamp the keybindings file");

    teardown();
  }

  #[test]
  fn overrides_round_trip_through_the_file() {
    let _path = setup("round-trip");

    set(ShortcutId::ShowCommandPalette, "cmd-shift-k");
    set(ShortcutId::CommitChanges, "cmd-j");

    let overrides = load();
    assert_eq!(
      overrides.get(&ShortcutId::ShowCommandPalette),
      Some(&"cmd-shift-k".to_string())
    );
    assert_eq!(
      overrides.get(&ShortcutId::CommitChanges),
      Some(&"cmd-j".to_string())
    );
    assert!(take_startup_error().is_none());

    teardown();
  }

  #[test]
  fn removing_an_override_drops_its_key_from_the_file() {
    let path = setup("remove");

    set(ShortcutId::ShowCommandPalette, "cmd-shift-k");
    set(ShortcutId::CommitChanges, "cmd-j");
    remove(ShortcutId::ShowCommandPalette);

    let raw = fs::read_to_string(&path).expect("read file");
    assert!(!raw.contains("show_command_palette"));
    let overrides = load();
    assert!(!overrides.contains_key(&ShortcutId::ShowCommandPalette));
    assert_eq!(
      overrides.get(&ShortcutId::CommitChanges),
      Some(&"cmd-j".to_string())
    );

    teardown();
  }

  #[test]
  fn an_unknown_id_is_ignored_on_read_but_survives_a_rewrite() {
    let path = setup("unknown-id");
    fs::write(
      &path,
      r#"{ "from_the_future": "cmd-x", "commit_changes": "cmd-j" }"#,
    )
    .expect("write file");

    let overrides = load();
    assert_eq!(overrides.len(), 1);
    assert_eq!(
      overrides.get(&ShortcutId::CommitChanges),
      Some(&"cmd-j".to_string())
    );

    // Editing another key must not drop the entry this build does not know.
    set(ShortcutId::ShowCommandPalette, "cmd-shift-k");
    remove(ShortcutId::CommitChanges);
    let raw = fs::read_to_string(&path).expect("read file");
    assert!(raw.contains("from_the_future"));
    assert!(raw.contains("cmd-x"));
    assert!(take_startup_error().is_none());

    teardown();
  }

  #[test]
  fn an_invalid_keystroke_is_ignored_and_the_others_survive() {
    let path = setup("invalid-keystroke");
    fs::write(
      &path,
      r#"{ "show_command_palette": "not-a-keystroke", "pull_changes": 3, "commit_changes": "cmd-j" }"#,
    )
    .expect("write file");

    let overrides = load();
    assert_eq!(overrides.len(), 1);
    assert_eq!(
      overrides.get(&ShortcutId::CommitChanges),
      Some(&"cmd-j".to_string())
    );
    assert!(take_startup_error().is_none());

    teardown();
  }

  #[test]
  fn a_corrupted_file_loads_empty_is_reported_and_is_never_clobbered() {
    let path = setup("corrupted");
    fs::write(&path, "{ not json").expect("write file");

    assert!(load().is_empty());
    assert!(take_startup_error().is_some());

    // A write must refuse to replace a file the user may be mid-editing.
    set(ShortcutId::CommitChanges, "cmd-j");
    assert_eq!(fs::read_to_string(&path).expect("read file"), "{ not json");

    teardown();
  }

  #[test]
  fn keybindings_file_path_uses_profile_namespace() {
    let base = PathBuf::from("/tmp/reviu-config");

    assert_eq!(
      keybindings_file_path_for_profile(Some(base.clone()), AppProfile::Prod),
      PathBuf::from("/tmp/reviu-config/reviu/keybindings.json")
    );
    assert_eq!(
      keybindings_file_path_for_profile(Some(base), AppProfile::Dev),
      PathBuf::from("/tmp/reviu-config/reviu.dev/keybindings.json")
    );
    assert_eq!(
      keybindings_file_path_for_profile(None, AppProfile::Prod),
      PathBuf::from("keybindings.json")
    );
    assert_eq!(
      keybindings_file_path_for_profile(None, AppProfile::Dev),
      PathBuf::from("keybindings.dev.json")
    );
  }
}
