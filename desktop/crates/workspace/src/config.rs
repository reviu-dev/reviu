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

const CONFIG_TABLES: [ConfigTable; 1] = [RECENT_REPOS_TABLE];

pub struct ConfigStore {
  conn: Connection,
}

#[derive(Clone)]
pub struct RecentRepository {
  pub path: PathBuf,
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
    Some(store)
  }

  fn ensure_tables(&self) -> rusqlite::Result<()> {
    for table in CONFIG_TABLES {
      self.conn.execute(table.create_sql, [])?;
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
}
