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
  create_sql:
    "CREATE TABLE IF NOT EXISTS settings (id INTEGER PRIMARY KEY CHECK (id = 1), auto_switch_theme INTEGER NOT NULL DEFAULT 1, dark_mode INTEGER NOT NULL DEFAULT 0)",
};

const CONFIG_TABLES: [ConfigTable; 2] = [RECENT_REPOS_TABLE, SETTINGS_TABLE];

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
}

impl Default for AppSettings {
  fn default() -> Self {
    Self {
      auto_switch_theme: true,
      dark_mode: false,
    }
  }
}

impl ConfigStore {
  fn db_path() -> PathBuf {
    if let Some(base) = config_dir() {
      base.join(CONFIG_DIR_NAME).join(CONFIG_DB_NAME)
    } else {
      PathBuf::from(CONFIG_DB_NAME)
    }
  }

  fn open() -> Option<Self> {
    let path = Self::db_path();
    if let Some(parent) = path.parent() {
      if let Err(err) = fs::create_dir_all(parent) {
        eprintln!(
          "Failed to create config directory {}: {}",
          parent.display(),
          err
        );
        return None;
      }
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
        "SELECT auto_switch_theme, dark_mode FROM {} WHERE id = 1",
        SETTINGS_TABLE.name
      ),
      [],
      |row| {
        let auto_switch_theme: i64 = row.get(0)?;
        let dark_mode: i64 = row.get(1)?;
        Ok(AppSettings {
          auto_switch_theme: auto_switch_theme != 0,
          dark_mode: dark_mode != 0,
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
        "INSERT INTO {} (id, auto_switch_theme, dark_mode)
         VALUES (1, ?1, ?2)
         ON CONFLICT(id) DO UPDATE
         SET auto_switch_theme = excluded.auto_switch_theme,
             dark_mode = excluded.dark_mode",
        SETTINGS_TABLE.name
      ),
      params![
        if settings.auto_switch_theme { 1_i64 } else { 0_i64 },
        if settings.dark_mode { 1_i64 } else { 0_i64 }
      ],
    ) {
      eprintln!("Failed to persist app settings: {}", err);
    }
  }
}
