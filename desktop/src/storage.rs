use crate::error::Result;
use crate::state::{Config, User};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};

/// Local storage manager using SQLite
pub struct Storage {
  conn: Connection,
}

impl Storage {
  /// Create or open the local storage database
  pub fn new(data_dir: &Path) -> Result<Self> {
    std::fs::create_dir_all(data_dir)?;
    let db_path = data_dir.join("reviu.db");
    let conn = Connection::open(db_path)?;

    let storage = Self { conn };
    storage.initialize_schema()?;
    Ok(storage)
  }

  /// Initialize the database schema
  fn initialize_schema(&self) -> Result<()> {
    self.conn.execute_batch(
      r#"
      -- Auth tokens (encrypted)
      CREATE TABLE IF NOT EXISTS auth (
          id INTEGER PRIMARY KEY,
          token TEXT NOT NULL,
          expires_at INTEGER NOT NULL,
          user_id TEXT NOT NULL,
          created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
      );

      -- User preferences
      CREATE TABLE IF NOT EXISTS preferences (
          key TEXT PRIMARY KEY,
          value TEXT NOT NULL,
          updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
      );

      -- Recent repositories
      CREATE TABLE IF NOT EXISTS recent_repos (
          path TEXT PRIMARY KEY,
          last_opened_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
          name TEXT,
          UNIQUE(path)
      );

      -- Feature flags cache (TTL: 5 minutes)
      CREATE TABLE IF NOT EXISTS feature_flags (
          key TEXT PRIMARY KEY,
          enabled INTEGER NOT NULL,
          cached_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
      );

      -- Window state
      CREATE TABLE IF NOT EXISTS window_state (
          id INTEGER PRIMARY KEY CHECK (id = 1),
          width INTEGER NOT NULL,
          height INTEGER NOT NULL,
          x INTEGER,
          y INTEGER,
          maximized INTEGER NOT NULL DEFAULT 0
      );

      -- User cache
      CREATE TABLE IF NOT EXISTS user_cache (
          id TEXT PRIMARY KEY,
          email TEXT NOT NULL,
          name TEXT,
          avatar_url TEXT,
          premium INTEGER NOT NULL DEFAULT 0,
          updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
      );
      "#,
    )?;
    Ok(())
  }

  // Auth operations

  /// Save authentication token
  pub fn save_auth_token(&self, token: &str, expires_at: i64, user_id: &str) -> Result<()> {
    self.conn.execute(
      "INSERT OR REPLACE INTO auth (id, token, expires_at, user_id) VALUES (1, ?1, ?2, ?3)",
      params![token, expires_at, user_id],
    )?;
    Ok(())
  }

  /// Get authentication token
  pub fn get_auth_token(&self) -> Result<Option<(String, i64, String)>> {
    let result = self
      .conn
      .query_row(
        "SELECT token, expires_at, user_id FROM auth WHERE id = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
      )
      .optional()?;
    Ok(result)
  }

  /// Delete authentication token
  pub fn delete_auth_token(&self) -> Result<()> {
    self.conn.execute("DELETE FROM auth WHERE id = 1", [])?;
    Ok(())
  }

  // User operations

  /// Save user to cache
  pub fn save_user(&self, user: &User) -> Result<()> {
    self.conn.execute(
      "INSERT OR REPLACE INTO user_cache (id, email, name, avatar_url, premium) VALUES (?1, ?2, ?3, ?4, ?5)",
      params![
        user.id,
        user.email,
        user.name,
        user.avatar_url,
        user.premium as i32,
      ],
    )?;
    Ok(())
  }

  /// Get user from cache
  pub fn get_user(&self) -> Result<Option<User>> {
    let result = self
      .conn
      .query_row(
        "SELECT id, email, name, avatar_url, premium FROM user_cache LIMIT 1",
        [],
        |row| {
          Ok(User {
            id: row.get(0)?,
            email: row.get(1)?,
            name: row.get(2)?,
            avatar_url: row.get(3)?,
            premium: row.get::<_, i32>(4)? != 0,
          })
        },
      )
      .optional()?;
    Ok(result)
  }

  /// Delete user from cache
  pub fn delete_user(&self) -> Result<()> {
    self.conn.execute("DELETE FROM user_cache", [])?;
    Ok(())
  }

  // Preferences operations

  /// Save a preference
  pub fn save_preference(&self, key: &str, value: &str) -> Result<()> {
    self.conn.execute(
      "INSERT OR REPLACE INTO preferences (key, value, updated_at) VALUES (?1, ?2, strftime('%s', 'now'))",
      params![key, value],
    )?;
    Ok(())
  }

  /// Get a preference
  pub fn get_preference(&self, key: &str) -> Result<Option<String>> {
    let result = self
      .conn
      .query_row(
        "SELECT value FROM preferences WHERE key = ?1",
        params![key],
        |row| row.get(0),
      )
      .optional()?;
    Ok(result)
  }

  /// Save config to preferences
  pub fn save_config(&self, config: &Config) -> Result<()> {
    let config_json = serde_json::to_string(config)?;
    self.save_preference("config", &config_json)
  }

  /// Load config from preferences
  pub fn load_config(&self) -> Result<Option<Config>> {
    if let Some(config_json) = self.get_preference("config")? {
      let config: Config = serde_json::from_str(&config_json)?;
      Ok(Some(config))
    } else {
      Ok(None)
    }
  }

  // Recent repositories operations

  /// Add a repository to recent list
  pub fn add_recent_repo(&self, path: &Path, name: &str) -> Result<()> {
    let path_str = path.to_string_lossy().to_string();
    self.conn.execute(
      "INSERT OR REPLACE INTO recent_repos (path, name, last_opened_at) VALUES (?1, ?2, strftime('%s', 'now'))",
      params![path_str, name],
    )?;
    Ok(())
  }

  /// Get recent repositories
  pub fn get_recent_repos(&self, limit: usize) -> Result<Vec<(PathBuf, String, i64)>> {
    let mut stmt = self.conn.prepare(
      "SELECT path, name, last_opened_at FROM recent_repos ORDER BY last_opened_at DESC LIMIT ?1",
    )?;

    let repos = stmt
      .query_map(params![limit], |row| {
        Ok((
          PathBuf::from(row.get::<_, String>(0)?),
          row.get(1)?,
          row.get(2)?,
        ))
      })?
      .collect::<std::result::Result<Vec<_>, _>>()?;

    Ok(repos)
  }

  /// Remove a repository from recent list
  pub fn remove_recent_repo(&self, path: &Path) -> Result<()> {
    let path_str = path.to_string_lossy().to_string();
    self.conn.execute(
      "DELETE FROM recent_repos WHERE path = ?1",
      params![path_str],
    )?;
    Ok(())
  }

  /// Clear all recent repositories
  pub fn clear_recent_repos(&self) -> Result<()> {
    self.conn.execute("DELETE FROM recent_repos", [])?;
    Ok(())
  }

  // Feature flags operations

  /// Save a feature flag
  pub fn save_feature_flag(&self, key: &str, enabled: bool) -> Result<()> {
    self.conn.execute(
      "INSERT OR REPLACE INTO feature_flags (key, enabled, cached_at) VALUES (?1, ?2, strftime('%s', 'now'))",
      params![key, enabled as i32],
    )?;
    Ok(())
  }

  /// Get a feature flag
  pub fn get_feature_flag(&self, key: &str, ttl_seconds: i64) -> Result<Option<bool>> {
    let result = self
      .conn
      .query_row(
        "SELECT enabled FROM feature_flags WHERE key = ?1 AND (strftime('%s', 'now') - cached_at) < ?2",
        params![key, ttl_seconds],
        |row| Ok(row.get::<_, i32>(0)? != 0),
      )
      .optional()?;
    Ok(result)
  }

  /// Clear expired feature flags
  pub fn clear_expired_feature_flags(&self, ttl_seconds: i64) -> Result<()> {
    self.conn.execute(
      "DELETE FROM feature_flags WHERE (strftime('%s', 'now') - cached_at) >= ?1",
      params![ttl_seconds],
    )?;
    Ok(())
  }

  // Window state operations

  /// Save window state
  pub fn save_window_state(
    &self,
    width: i32,
    height: i32,
    x: Option<i32>,
    y: Option<i32>,
    maximized: bool,
  ) -> Result<()> {
    self.conn.execute(
      "INSERT OR REPLACE INTO window_state (id, width, height, x, y, maximized) VALUES (1, ?1, ?2, ?3, ?4, ?5)",
      params![width, height, x, y, maximized as i32],
    )?;
    Ok(())
  }

  /// Get window state
  pub fn get_window_state(&self) -> Result<Option<WindowState>> {
    let result = self
      .conn
      .query_row(
        "SELECT width, height, x, y, maximized FROM window_state WHERE id = 1",
        [],
        |row| {
          Ok(WindowState {
            width: row.get(0)?,
            height: row.get(1)?,
            x: row.get(2)?,
            y: row.get(3)?,
            maximized: row.get::<_, i32>(4)? != 0,
          })
        },
      )
      .optional()?;
    Ok(result)
  }
}

/// Window state information
#[derive(Debug, Clone)]
pub struct WindowState {
  pub width: i32,
  pub height: i32,
  pub x: Option<i32>,
  pub y: Option<i32>,
  pub maximized: bool,
}

#[cfg(test)]
mod tests {
  use super::*;
  use tempfile::tempdir;

  #[test]
  fn test_storage_creation() {
    let dir = tempdir().unwrap();
    let storage = Storage::new(dir.path()).unwrap();
    assert!(dir.path().join("reviu.db").exists());
  }

  #[test]
  fn test_auth_token() {
    let dir = tempdir().unwrap();
    let storage = Storage::new(dir.path()).unwrap();

    storage
      .save_auth_token("test_token", 1234567890, "user_123")
      .unwrap();
    let (token, expires_at, user_id) = storage.get_auth_token().unwrap().unwrap();

    assert_eq!(token, "test_token");
    assert_eq!(expires_at, 1234567890);
    assert_eq!(user_id, "user_123");

    storage.delete_auth_token().unwrap();
    assert!(storage.get_auth_token().unwrap().is_none());
  }

  #[test]
  fn test_preferences() {
    let dir = tempdir().unwrap();
    let storage = Storage::new(dir.path()).unwrap();

    storage.save_preference("theme", "dark").unwrap();
    let value = storage.get_preference("theme").unwrap();

    assert_eq!(value, Some("dark".to_string()));
  }

  #[test]
  fn test_recent_repos() {
    let dir = tempdir().unwrap();
    let storage = Storage::new(dir.path()).unwrap();

    storage
      .add_recent_repo(Path::new("/path/to/repo1"), "repo1")
      .unwrap();
    storage
      .add_recent_repo(Path::new("/path/to/repo2"), "repo2")
      .unwrap();

    let repos = storage.get_recent_repos(10).unwrap();
    assert_eq!(repos.len(), 2);
    assert_eq!(repos[0].1, "repo2"); // Most recent first
    assert_eq!(repos[1].1, "repo1");
  }
}
