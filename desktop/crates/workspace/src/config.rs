#[cfg(test)]
use std::cell::RefCell;
use std::{
  fs,
  path::{Path, PathBuf},
  time::{SystemTime, UNIX_EPOCH},
};

use dirs::config_dir;
use rusqlite::{Connection, params};

const CONFIG_DIR_NAME: &str = "reviu";
const CONFIG_DB_NAME: &str = "reviu.sqlite";

#[derive(Clone, Copy)]
struct ConfigTable {
  name: &'static str,
  create_sql: &'static str,
}

const RECENT_REPOS_TABLE: ConfigTable = ConfigTable {
  name: "recent_repositories",
  create_sql: "CREATE TABLE IF NOT EXISTS recent_repositories (path TEXT PRIMARY KEY, last_opened INTEGER NOT NULL)",
};

const SETTINGS_TABLE: ConfigTable = ConfigTable {
  name: "settings",
  create_sql: "CREATE TABLE IF NOT EXISTS settings (id INTEGER PRIMARY KEY CHECK (id = 1), auto_switch_theme INTEGER NOT NULL DEFAULT 1, dark_mode INTEGER NOT NULL DEFAULT 0, indent_rainbow INTEGER NOT NULL DEFAULT 0)",
};

const CONFIG_TABLES: [ConfigTable; 2] = [RECENT_REPOS_TABLE, SETTINGS_TABLE];

#[cfg(test)]
thread_local! {
  static TEST_DB_PATH: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

pub struct ConfigStore {
  conn: Connection,
}

#[derive(Clone)]
pub struct RecentRepository {
  pub path: PathBuf,
}

#[derive(Clone, Copy, Debug)]
pub struct AppSettings {
  pub auto_switch_theme: bool,
  pub dark_mode: bool,
  pub indent_rainbow: bool,
}

impl Default for AppSettings {
  fn default() -> Self {
    Self {
      auto_switch_theme: true,
      dark_mode: false,
      indent_rainbow: false,
    }
  }
}

impl ConfigStore {
  fn db_path() -> PathBuf {
    #[cfg(test)]
    if let Some(path) = Self::test_db_path() {
      return path;
    }

    if let Some(base) = config_dir() {
      base.join(CONFIG_DIR_NAME).join(CONFIG_DB_NAME)
    } else {
      PathBuf::from(CONFIG_DB_NAME)
    }
  }

  #[cfg(test)]
  fn test_db_path() -> Option<PathBuf> {
    TEST_DB_PATH.with(|path| path.borrow().clone())
  }

  #[cfg(test)]
  pub(crate) fn set_test_db_path(path: Option<PathBuf>) {
    TEST_DB_PATH.with(|slot| {
      *slot.borrow_mut() = path;
    });
  }

  fn open() -> Option<Self> {
    let path = Self::db_path();
    if let Some(parent) = path.parent()
      && let Err(err) = fs::create_dir_all(parent)
    {
      eprintln!(
        "Failed to create config directory {}: {}",
        parent.display(),
        err
      );
      return None;
    }

    match Connection::open(&path) {
      Ok(conn) => Some(Self { conn }),
      Err(err) => {
        eprintln!("Failed to open config database {}: {}", path.display(), err);
        None
      }
    }
  }

  fn open_with_tables() -> Option<Self> {
    let store = Self::open()?;
    if let Err(err) = store.ensure_tables() {
      eprintln!("Failed to initialize config tables: {}", err);
      return None;
    }
    if let Err(err) = store.ensure_default_rows() {
      eprintln!("Failed to initialize default config values: {}", err);
      return None;
    }
    if let Err(err) = store.ensure_settings_columns() {
      eprintln!("Failed to initialize settings schema: {}", err);
      return None;
    }
    Some(store)
  }

  fn ensure_tables(&self) -> rusqlite::Result<()> {
    for table in CONFIG_TABLES {
      self.conn.execute(table.create_sql, [])?;
    }
    Ok(())
  }

  fn ensure_default_rows(&self) -> rusqlite::Result<()> {
    self.conn.execute(
      &format!(
        "INSERT INTO {} (id) VALUES (1)
         ON CONFLICT(id) DO NOTHING",
        SETTINGS_TABLE.name
      ),
      [],
    )?;
    Ok(())
  }

  fn ensure_settings_columns(&self) -> rusqlite::Result<()> {
    let mut has_indent_rainbow = false;
    let mut stmt = self
      .conn
      .prepare(&format!("PRAGMA table_info({})", SETTINGS_TABLE.name))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
      let column = row?;
      if column == "indent_rainbow" {
        has_indent_rainbow = true;
        break;
      }
    }

    if !has_indent_rainbow {
      self.conn.execute(
        &format!(
          "ALTER TABLE {} ADD COLUMN indent_rainbow INTEGER NOT NULL DEFAULT 0",
          SETTINGS_TABLE.name
        ),
        [],
      )?;
    }

    Ok(())
  }

  pub fn load_recent_repositories() -> Vec<RecentRepository> {
    let Some(store) = Self::open_with_tables() else {
      return Vec::new();
    };
    store.load_recent_repositories_inner()
  }

  fn load_recent_repositories_inner(&self) -> Vec<RecentRepository> {
    let mut stmt = match self.conn.prepare(&format!(
      "SELECT path FROM {} ORDER BY last_opened DESC",
      RECENT_REPOS_TABLE.name
    )) {
      Ok(stmt) => stmt,
      Err(err) => {
        eprintln!("Failed to load recent repositories: {}", err);
        return Vec::new();
      }
    };

    let rows = match stmt.query_map([], |row| row.get::<_, String>(0)) {
      Ok(rows) => rows,
      Err(err) => {
        eprintln!("Failed to read recent repositories: {}", err);
        return Vec::new();
      }
    };

    let mut repositories = Vec::new();
    for row in rows {
      match row {
        Ok(path) => repositories.push(RecentRepository {
          path: PathBuf::from(path),
        }),
        Err(err) => eprintln!("Failed to decode recent repository row: {}", err),
      }
    }

    repositories
  }

  pub fn persist_recent_repository(path: &Path) {
    let Some(store) = Self::open_with_tables() else {
      return;
    };
    store.persist_recent_repository_inner(path);
  }

  fn persist_recent_repository_inner(&self, path: &Path) {
    let last_opened = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .unwrap_or_default()
      .as_secs() as i64;
    let path_string = path.to_string_lossy().to_string();

    if let Err(err) = self.conn.execute(
      &format!(
        "INSERT INTO {} (path, last_opened) VALUES (?1, ?2)
         ON CONFLICT(path) DO UPDATE SET last_opened = excluded.last_opened",
        RECENT_REPOS_TABLE.name
      ),
      params![path_string, last_opened],
    ) {
      eprintln!("Failed to persist recent repository: {}", err);
    }
  }

  pub fn load_app_settings() -> AppSettings {
    let Some(store) = Self::open_with_tables() else {
      return AppSettings::default();
    };
    store.load_app_settings_inner()
  }

  fn load_app_settings_inner(&self) -> AppSettings {
    let settings = self.conn.query_row(
      &format!(
        "SELECT auto_switch_theme, dark_mode, indent_rainbow FROM {} WHERE id = 1",
        SETTINGS_TABLE.name
      ),
      [],
      |row| {
        let auto_switch_theme: i64 = row.get(0)?;
        let dark_mode: i64 = row.get(1)?;
        let indent_rainbow: i64 = row.get(2)?;
        Ok(AppSettings {
          auto_switch_theme: auto_switch_theme != 0,
          dark_mode: dark_mode != 0,
          indent_rainbow: indent_rainbow != 0,
        })
      },
    );

    match settings {
      Ok(settings) => settings,
      Err(err) => {
        eprintln!("Failed to load app settings: {}", err);
        AppSettings::default()
      }
    }
  }

  pub fn persist_app_settings(settings: AppSettings) {
    let Some(store) = Self::open_with_tables() else {
      return;
    };
    store.persist_app_settings_inner(settings);
  }

  fn persist_app_settings_inner(&self, settings: AppSettings) {
    if let Err(err) = self.conn.execute(
      &format!(
        "INSERT INTO {} (id, auto_switch_theme, dark_mode, indent_rainbow)
         VALUES (1, ?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE
         SET auto_switch_theme = excluded.auto_switch_theme,
             dark_mode = excluded.dark_mode,
             indent_rainbow = excluded.indent_rainbow",
        SETTINGS_TABLE.name
      ),
      params![
        if settings.auto_switch_theme {
          1_i64
        } else {
          0_i64
        },
        if settings.dark_mode { 1_i64 } else { 0_i64 },
        if settings.indent_rainbow {
          1_i64
        } else {
          0_i64
        }
      ],
    ) {
      eprintln!("Failed to persist app settings: {}", err);
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::sync::atomic::{AtomicU64, Ordering};

  fn unique_test_db_path(label: &str) -> PathBuf {
    static NEXT_DB_ID: AtomicU64 = AtomicU64::new(1);
    let id = NEXT_DB_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
      "reviu-config-{label}-{}-{id}.sqlite",
      std::process::id()
    ))
  }

  #[test]
  fn recent_repositories_use_test_db_override() {
    let db_path = unique_test_db_path("recent");
    let _ = fs::remove_file(&db_path);
    ConfigStore::set_test_db_path(Some(db_path));

    let repo_a = Path::new("/tmp/reviu-config-test-repo-a");
    let repo_b = Path::new("/tmp/reviu-config-test-repo-b");
    ConfigStore::persist_recent_repository(repo_a);
    ConfigStore::persist_recent_repository(repo_b);

    let paths: Vec<PathBuf> = ConfigStore::load_recent_repositories()
      .into_iter()
      .map(|repo| repo.path)
      .collect();
    assert!(paths.contains(&repo_a.to_path_buf()));
    assert!(paths.contains(&repo_b.to_path_buf()));

    ConfigStore::set_test_db_path(None);
  }

  #[test]
  fn app_settings_use_test_db_override() {
    let db_path = unique_test_db_path("settings");
    let _ = fs::remove_file(&db_path);
    ConfigStore::set_test_db_path(Some(db_path));

    let settings = AppSettings {
      auto_switch_theme: false,
      dark_mode: true,
      indent_rainbow: true,
    };
    ConfigStore::persist_app_settings(settings);

    let loaded = ConfigStore::load_app_settings();
    assert!(!loaded.auto_switch_theme);
    assert!(loaded.dark_mode);
    assert!(loaded.indent_rainbow);

    ConfigStore::set_test_db_path(None);
  }
}
