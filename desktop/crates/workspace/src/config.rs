use gpui::{App, Global};
#[cfg(test)]
use std::cell::RefCell;
use std::{
  collections::{HashMap, HashSet},
  fs,
  path::{Path, PathBuf},
  time::{SystemTime, UNIX_EPOCH},
};

use crate::AppProfile;
use crate::api::GithubPullRequestMergeMethod;
use crate::shortcuts::ShortcutId;
use app_log::ResultExt;
use dirs::config_dir;
use gpui::Keystroke;
use rusqlite::{Connection, params};

const CONFIG_DIR_NAME: &str = "reviu";
const CONFIG_DB_NAME: &str = "reviu.sqlite";
const DEV_CONFIG_DB_NAME: &str = "reviu.dev.sqlite";

#[derive(Clone, Copy)]
struct ConfigTable {
  name: &'static str,
  create_sql: &'static str,
}

const RECENT_REPOS_TABLE: ConfigTable = ConfigTable {
  name: "recent_repositories",
  create_sql: "CREATE TABLE IF NOT EXISTS recent_repositories (path TEXT PRIMARY KEY, last_opened INTEGER NOT NULL)",
};

const SESSION_SIDEBAR_REPOS_TABLE: ConfigTable = ConfigTable {
  name: "session_sidebar_repositories",
  create_sql: "CREATE TABLE IF NOT EXISTS session_sidebar_repositories (path TEXT PRIMARY KEY, position INTEGER NOT NULL)",
};

const SETTINGS_TABLE: ConfigTable = ConfigTable {
  name: "settings",
  create_sql: "CREATE TABLE IF NOT EXISTS settings (id INTEGER PRIMARY KEY CHECK (id = 1), auto_switch_theme INTEGER NOT NULL DEFAULT 1, dark_mode INTEGER NOT NULL DEFAULT 0, indent_rainbow INTEGER NOT NULL DEFAULT 0)",
};

const SHORTCUT_OVERRIDES_TABLE: ConfigTable = ConfigTable {
  name: "shortcut_overrides",
  create_sql: "CREATE TABLE IF NOT EXISTS shortcut_overrides (shortcut_id TEXT PRIMARY KEY, keystroke TEXT NOT NULL)",
};

const COMMAND_USAGES_TABLE: ConfigTable = ConfigTable {
  name: "command_usages",
  create_sql: "CREATE TABLE IF NOT EXISTS command_usages (command_id TEXT PRIMARY KEY, recent_timestamps TEXT NOT NULL)",
};

const ANALYTICS_META_TABLE: ConfigTable = ConfigTable {
  name: "analytics_meta",
  create_sql: "CREATE TABLE IF NOT EXISTS analytics_meta (id INTEGER PRIMARY KEY CHECK (id = 1), device_id TEXT NOT NULL)",
};

const MERGE_METHODS_TABLE: ConfigTable = ConfigTable {
  name: "merge_methods",
  create_sql: "CREATE TABLE IF NOT EXISTS merge_methods (repo TEXT PRIMARY KEY, method TEXT NOT NULL)",
};

pub const COMMAND_USAGE_TIMESTAMP_CAP: usize = 30;

const CONFIG_TABLES: [ConfigTable; 6] = [
  RECENT_REPOS_TABLE,
  SESSION_SIDEBAR_REPOS_TABLE,
  SETTINGS_TABLE,
  SHORTCUT_OVERRIDES_TABLE,
  COMMAND_USAGES_TABLE,
  ANALYTICS_META_TABLE,
];

type Migration = fn(&Connection) -> rusqlite::Result<()>;

/// Ordered config-database migrations. The vector index + 1 is the migration's `user_version`.
/// Only append; never reorder or edit a shipped migration. v1 is an idempotent baseline so any
/// pre-existing database (which has `user_version` 0) converges to the current schema.
const MIGRATIONS: &[Migration] = [
  migrate_v1_baseline,
  migrate_v2_merge_methods,
  migrate_v3_session_sidebar_repositories,
]
.as_slice();

fn schema_version(conn: &Connection) -> rusqlite::Result<i64> {
  conn.query_row("PRAGMA user_version", [], |row| row.get(0))
}

fn set_schema_version(conn: &Connection, version: i64) -> rusqlite::Result<()> {
  // PRAGMA cannot be parameterized; `version` is a controlled integer, not user input.
  conn.execute_batch(&format!("PRAGMA user_version = {version}"))
}

fn run_migrations(conn: &Connection) -> rusqlite::Result<()> {
  let mut current = schema_version(conn)?;
  for (index, migration) in MIGRATIONS.iter().enumerate() {
    let version = index as i64 + 1;
    if current >= version {
      continue;
    }
    conn.execute_batch("BEGIN")?;
    let applied = migration(conn).and_then(|()| set_schema_version(conn, version));
    match applied {
      Ok(()) => {
        conn.execute_batch("COMMIT")?;
        current = version;
      }
      Err(err) => {
        conn
          .execute_batch("ROLLBACK")
          .log_err_context("rolling back a failed migration");
        return Err(err);
      }
    }
  }
  Ok(())
}

fn migrate_v1_baseline(conn: &Connection) -> rusqlite::Result<()> {
  create_baseline_tables(conn)?;
  ensure_default_rows(conn)?;
  ensure_settings_columns(conn)?;
  Ok(())
}

fn migrate_v2_merge_methods(conn: &Connection) -> rusqlite::Result<()> {
  conn.execute(MERGE_METHODS_TABLE.create_sql, [])?;
  Ok(())
}

fn migrate_v3_session_sidebar_repositories(conn: &Connection) -> rusqlite::Result<()> {
  conn.execute(SESSION_SIDEBAR_REPOS_TABLE.create_sql, [])?;
  Ok(())
}

fn merge_method_storage_key(method: GithubPullRequestMergeMethod) -> &'static str {
  match method {
    GithubPullRequestMergeMethod::Merge => "merge",
    GithubPullRequestMergeMethod::Squash => "squash",
    GithubPullRequestMergeMethod::Rebase => "rebase",
  }
}

fn merge_method_from_storage(value: &str) -> Option<GithubPullRequestMergeMethod> {
  match value {
    "merge" => Some(GithubPullRequestMergeMethod::Merge),
    "squash" => Some(GithubPullRequestMergeMethod::Squash),
    "rebase" => Some(GithubPullRequestMergeMethod::Rebase),
    _ => None,
  }
}

fn create_baseline_tables(conn: &Connection) -> rusqlite::Result<()> {
  for table in CONFIG_TABLES {
    conn.execute(table.create_sql, [])?;
  }
  Ok(())
}

fn ensure_default_rows(conn: &Connection) -> rusqlite::Result<()> {
  conn.execute(
    &format!(
      "INSERT INTO {} (id) VALUES (1)
       ON CONFLICT(id) DO NOTHING",
      SETTINGS_TABLE.name
    ),
    [],
  )?;
  Ok(())
}

/// FROZEN: only runs below v1. A new setting needs its own migration, never an
/// entry here.
fn ensure_settings_columns(conn: &Connection) -> rusqlite::Result<()> {
  const COLUMNS: &[(&str, &str)] = &[
    ("indent_rainbow", "INTEGER NOT NULL DEFAULT 0"),
    ("font_size", "REAL NOT NULL DEFAULT 16.0"),
    ("git_unified_file_view", "INTEGER NOT NULL DEFAULT 0"),
    ("split_diff_view", "INTEGER NOT NULL DEFAULT 0"),
    ("hide_whitespace", "INTEGER NOT NULL DEFAULT 0"),
    ("menu_bar_icon", "INTEGER NOT NULL DEFAULT 1"),
    ("analytics_enabled", "INTEGER NOT NULL DEFAULT 1"),
  ];

  for (column, definition) in COLUMNS {
    add_settings_column_if_missing(conn, column, definition)?;
  }

  Ok(())
}

fn settings_column_names(conn: &Connection) -> rusqlite::Result<HashSet<String>> {
  let mut stmt = conn.prepare(&format!("PRAGMA table_info({})", SETTINGS_TABLE.name))?;
  let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
  rows.collect()
}

/// Guarded so re-running a migration is a no-op instead of an error.
fn add_settings_column_if_missing(
  conn: &Connection,
  column: &str,
  definition: &str,
) -> rusqlite::Result<()> {
  if settings_column_names(conn)?.contains(column) {
    return Ok(());
  }
  conn.execute(
    &format!(
      "ALTER TABLE {} ADD COLUMN {column} {definition}",
      SETTINGS_TABLE.name
    ),
    [],
  )?;
  Ok(())
}

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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AppSettings {
  pub auto_switch_theme: bool,
  pub dark_mode: bool,
  pub indent_rainbow: bool,
  pub font_size: f32,
  pub git_unified_file_view: bool,
  pub split_diff_view: bool,
  pub hide_whitespace: bool,
  pub menu_bar_icon: bool,
  pub analytics_enabled: bool,
  /// Popup when the agent finishes or asks while the window is inactive.
  pub agent_notifications: bool,
}

impl Global for AppSettings {}

impl AppSettings {
  pub fn get(cx: &App) -> Self {
    *cx.global::<Self>()
  }

  pub fn update(cx: &mut App, f: impl FnOnce(&mut Self)) {
    let mut settings = *cx.global::<Self>();
    f(&mut settings);
    cx.set_global(settings);
    ConfigStore::persist_app_settings(settings);
  }
}

impl Default for AppSettings {
  fn default() -> Self {
    Self {
      auto_switch_theme: true,
      dark_mode: false,
      indent_rainbow: false,
      font_size: 16.0,
      git_unified_file_view: false,
      split_diff_view: false,
      hide_whitespace: false,
      menu_bar_icon: true,
      analytics_enabled: true,
      agent_notifications: true,
    }
  }
}

impl ConfigStore {
  fn db_path() -> PathBuf {
    #[cfg(test)]
    if let Some(path) = Self::test_db_path() {
      return path;
    }

    config_db_path_for_profile(config_dir(), AppProfile::current())
  }

  #[cfg(test)]
  pub(crate) fn test_db_path() -> Option<PathBuf> {
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
      log::warn!(
        "Failed to create config directory {}: {}",
        parent.display(),
        err
      );
      return None;
    }

    match Connection::open(&path) {
      Ok(conn) => Some(Self { conn }),
      Err(err) => {
        log::warn!("Failed to open config database {}: {}", path.display(), err);
        None
      }
    }
  }

  fn open_with_tables() -> Option<Self> {
    let store = Self::open()?;
    if let Err(err) = run_migrations(&store.conn) {
      log::warn!("Failed to migrate config database: {}", err);
      return None;
    }
    Some(store)
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
        log::warn!("Failed to load recent repositories: {}", err);
        return Vec::new();
      }
    };

    let rows = match stmt.query_map([], |row| row.get::<_, String>(0)) {
      Ok(rows) => rows,
      Err(err) => {
        log::warn!("Failed to read recent repositories: {}", err);
        return Vec::new();
      }
    };

    let mut repositories = Vec::new();
    let mut missing_paths: Vec<String> = Vec::new();
    for row in rows {
      match row {
        Ok(path_string) => {
          let path = PathBuf::from(&path_string);
          // A folder that stopped being a repository would otherwise come back
          // as the one to open on the next launch.
          if path.is_dir() && path.join(".git").exists() {
            repositories.push(RecentRepository { path });
          } else {
            missing_paths.push(path_string);
          }
        }
        Err(err) => log::warn!("Failed to decode recent repository row: {}", err),
      }
    }

    if !missing_paths.is_empty() {
      for path in &missing_paths {
        self.forget_recent_repository_path(path);
      }
    }

    repositories
  }

  pub fn forget_recent_repository(path: &Path) {
    let Some(store) = Self::open_with_tables() else {
      return;
    };
    store.forget_recent_repository_path(&path.to_string_lossy());
  }

  fn forget_recent_repository_path(&self, path: &str) {
    if let Err(err) = self.conn.execute(
      &format!("DELETE FROM {} WHERE path = ?1", RECENT_REPOS_TABLE.name),
      params![path],
    ) {
      log::warn!("Failed to forget recent repository: {}", err);
    }
    self.forget_session_sidebar_repository_path(path);
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
      log::warn!("Failed to persist recent repository: {}", err);
    }
  }

  pub fn load_session_sidebar_repositories() -> Vec<PathBuf> {
    let Some(store) = Self::open_with_tables() else {
      return Vec::new();
    };
    store.load_session_sidebar_repositories_inner()
  }

  fn load_session_sidebar_repositories_inner(&self) -> Vec<PathBuf> {
    let mut stmt = match self.conn.prepare(&format!(
      "SELECT path FROM {} ORDER BY position ASC",
      SESSION_SIDEBAR_REPOS_TABLE.name
    )) {
      Ok(stmt) => stmt,
      Err(err) => {
        log::warn!("Failed to load session sidebar repositories: {}", err);
        return Vec::new();
      }
    };

    let rows = match stmt.query_map([], |row| row.get::<_, String>(0)) {
      Ok(rows) => rows,
      Err(err) => {
        log::warn!("Failed to read session sidebar repositories: {}", err);
        return Vec::new();
      }
    };

    let mut repositories = Vec::new();
    let mut missing_paths = Vec::new();
    for row in rows {
      match row {
        Ok(path_string) => {
          let path = PathBuf::from(&path_string);
          if path.is_dir() && path.join(".git").exists() {
            repositories.push(path);
          } else {
            missing_paths.push(path_string);
          }
        }
        Err(err) => log::warn!("Failed to decode session sidebar repository row: {}", err),
      }
    }

    if !missing_paths.is_empty() {
      for path in &missing_paths {
        self.forget_session_sidebar_repository_path(path);
      }
    }

    repositories
  }

  pub fn persist_session_sidebar_repositories(paths: &[PathBuf]) {
    let Some(store) = Self::open_with_tables() else {
      return;
    };
    store.persist_session_sidebar_repositories_inner(paths);
  }

  fn persist_session_sidebar_repositories_inner(&self, paths: &[PathBuf]) {
    let applied = self.conn.execute_batch("BEGIN").and_then(|()| {
      self.conn.execute(
        &format!("DELETE FROM {}", SESSION_SIDEBAR_REPOS_TABLE.name),
        [],
      )?;
      for (position, path) in paths.iter().enumerate() {
        self.conn.execute(
          &format!(
            "INSERT INTO {} (path, position) VALUES (?1, ?2)",
            SESSION_SIDEBAR_REPOS_TABLE.name
          ),
          params![path.to_string_lossy().to_string(), position as i64],
        )?;
      }
      self.conn.execute_batch("COMMIT")
    });

    if let Err(err) = applied {
      self
        .conn
        .execute_batch("ROLLBACK")
        .log_err_context("rolling back a failed session sidebar repository write");
      log::warn!("Failed to persist session sidebar repositories: {}", err);
    }
  }

  fn forget_session_sidebar_repository_path(&self, path: &str) {
    if let Err(err) = self.conn.execute(
      &format!(
        "DELETE FROM {} WHERE path = ?1",
        SESSION_SIDEBAR_REPOS_TABLE.name
      ),
      params![path],
    ) {
      log::warn!("Failed to forget session sidebar repository: {}", err);
    }
  }

  /// The merge method last used on this repository, GitHub's own memory being
  /// out of reach.
  pub fn load_merge_method(repo: &str) -> Option<GithubPullRequestMergeMethod> {
    let store = Self::open_with_tables()?;
    let value: Option<String> = store
      .conn
      .query_row(
        &format!(
          "SELECT method FROM {} WHERE repo = ?1",
          MERGE_METHODS_TABLE.name
        ),
        params![repo],
        |row| row.get(0),
      )
      .ok();
    value.as_deref().and_then(merge_method_from_storage)
  }

  pub fn persist_merge_method(repo: &str, method: GithubPullRequestMergeMethod) {
    let Some(store) = Self::open_with_tables() else {
      return;
    };
    if let Err(err) = store.conn.execute(
      &format!(
        "INSERT INTO {} (repo, method) VALUES (?1, ?2)
         ON CONFLICT(repo) DO UPDATE SET method = excluded.method",
        MERGE_METHODS_TABLE.name
      ),
      params![repo, merge_method_storage_key(method)],
    ) {
      log::warn!("Failed to persist merge method: {}", err);
    }
  }

  pub fn load_app_settings() -> AppSettings {
    crate::settings_file::load()
  }

  /// Read of the legacy sqlite columns, kept for the one-time import into the
  /// settings file. The columns are never written any more.
  pub(crate) fn load_app_settings_from_db() -> AppSettings {
    let Some(store) = Self::open_with_tables() else {
      return AppSettings::default();
    };
    store.load_app_settings_inner()
  }

  fn load_app_settings_inner(&self) -> AppSettings {
    let settings = self.conn.query_row(
      &format!(
        "SELECT auto_switch_theme, dark_mode, indent_rainbow, font_size, git_unified_file_view, split_diff_view, hide_whitespace, menu_bar_icon, analytics_enabled FROM {} WHERE id = 1",
        SETTINGS_TABLE.name
      ),
      [],
      |row| {
        let auto_switch_theme: i64 = row.get(0)?;
        let dark_mode: i64 = row.get(1)?;
        let indent_rainbow: i64 = row.get(2)?;
        let font_size: f64 = row.get(3)?;
        let git_unified_file_view: i64 = row.get(4)?;
        let split_diff_view: i64 = row.get(5)?;
        let hide_whitespace: i64 = row.get(6)?;
        let menu_bar_icon: i64 = row.get(7)?;
        let analytics_enabled: i64 = row.get(8)?;
        Ok(AppSettings {
          auto_switch_theme: auto_switch_theme != 0,
          dark_mode: dark_mode != 0,
          indent_rainbow: indent_rainbow != 0,
          font_size: font_size as f32,
          git_unified_file_view: git_unified_file_view != 0,
          split_diff_view: split_diff_view != 0,
          hide_whitespace: hide_whitespace != 0,
          menu_bar_icon: menu_bar_icon != 0,
          analytics_enabled: analytics_enabled != 0,
          agent_notifications: true,
        })
      },
    );

    match settings {
      Ok(settings) => settings,
      Err(err) => {
        log::warn!("Failed to load app settings: {}", err);
        AppSettings::default()
      }
    }
  }

  pub fn persist_app_settings(settings: AppSettings) {
    crate::settings_file::persist(settings);
  }

  pub fn load_or_create_analytics_device_id() -> Option<String> {
    let store = Self::open_with_tables()?;
    store.load_or_create_analytics_device_id_inner()
  }

  fn load_or_create_analytics_device_id_inner(&self) -> Option<String> {
    let existing: rusqlite::Result<String> = self.conn.query_row(
      &format!(
        "SELECT device_id FROM {} WHERE id = 1",
        ANALYTICS_META_TABLE.name
      ),
      [],
      |row| row.get::<_, String>(0),
    );
    if let Ok(id) = existing {
      return Some(id);
    }

    let id = uuid::Uuid::new_v4().to_string();
    if let Err(err) = self.conn.execute(
      &format!(
        "INSERT INTO {} (id, device_id) VALUES (1, ?1)
         ON CONFLICT(id) DO UPDATE SET device_id = excluded.device_id",
        ANALYTICS_META_TABLE.name
      ),
      params![id],
    ) {
      log::warn!("Failed to persist analytics device id: {}", err);
      return None;
    }
    Some(id)
  }

  pub fn load_command_usages() -> HashMap<String, Vec<i64>> {
    let Some(store) = Self::open_with_tables() else {
      return HashMap::default();
    };
    store.load_command_usages_inner()
  }

  fn load_command_usages_inner(&self) -> HashMap<String, Vec<i64>> {
    let mut stmt = match self.conn.prepare(&format!(
      "SELECT command_id, recent_timestamps FROM {}",
      COMMAND_USAGES_TABLE.name
    )) {
      Ok(stmt) => stmt,
      Err(err) => {
        log::warn!("Failed to load command usages: {}", err);
        return HashMap::default();
      }
    };

    let rows = match stmt.query_map([], |row| {
      Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }) {
      Ok(rows) => rows,
      Err(err) => {
        log::warn!("Failed to read command usages: {}", err);
        return HashMap::default();
      }
    };

    let mut usages = HashMap::default();
    for row in rows {
      let Ok((command_id, timestamps_json)) = row else {
        continue;
      };
      let Ok(timestamps) = serde_json::from_str::<Vec<i64>>(&timestamps_json) else {
        continue;
      };
      usages.insert(command_id, timestamps);
    }

    usages
  }

  pub fn persist_command_usage(command_id: &str, timestamps: &[i64]) {
    let Some(store) = Self::open_with_tables() else {
      return;
    };
    store.persist_command_usage_inner(command_id, timestamps);
  }

  fn persist_command_usage_inner(&self, command_id: &str, timestamps: &[i64]) {
    let json = match serde_json::to_string(timestamps) {
      Ok(value) => value,
      Err(err) => {
        log::warn!("Failed to serialize command usage timestamps: {}", err);
        return;
      }
    };

    if let Err(err) = self.conn.execute(
      &format!(
        "INSERT INTO {} (command_id, recent_timestamps) VALUES (?1, ?2)
         ON CONFLICT(command_id) DO UPDATE SET recent_timestamps = excluded.recent_timestamps",
        COMMAND_USAGES_TABLE.name
      ),
      params![command_id, json],
    ) {
      log::warn!("Failed to persist command usage: {}", err);
    }
  }

  pub fn load_shortcut_overrides() -> HashMap<ShortcutId, String> {
    crate::keybindings_file::load()
  }

  /// Read of the legacy sqlite rows, kept for the one-time import into the
  /// keybindings file. The rows are never written any more.
  pub(crate) fn load_shortcut_overrides_from_db() -> HashMap<ShortcutId, String> {
    let Some(store) = Self::open_with_tables() else {
      return HashMap::default();
    };
    store.load_shortcut_overrides_inner()
  }

  fn load_shortcut_overrides_inner(&self) -> HashMap<ShortcutId, String> {
    let mut stmt = match self.conn.prepare(&format!(
      "SELECT shortcut_id, keystroke FROM {} ORDER BY shortcut_id ASC",
      SHORTCUT_OVERRIDES_TABLE.name
    )) {
      Ok(stmt) => stmt,
      Err(err) => {
        log::warn!("Failed to load shortcut overrides: {}", err);
        return HashMap::default();
      }
    };

    let rows = match stmt.query_map([], |row| {
      Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }) {
      Ok(rows) => rows,
      Err(err) => {
        log::warn!("Failed to read shortcut overrides: {}", err);
        return HashMap::default();
      }
    };

    let mut overrides = HashMap::default();
    for row in rows {
      let Ok((shortcut_id, keystroke)) = row else {
        continue;
      };
      let Some(shortcut_id) = ShortcutId::from_storage_key(&shortcut_id) else {
        continue;
      };
      if Keystroke::parse(&keystroke).is_err() {
        continue;
      }
      overrides.insert(shortcut_id, keystroke);
    }

    overrides
  }

  pub fn persist_shortcut_override(shortcut_id: ShortcutId, keystroke: &str) {
    crate::keybindings_file::set(shortcut_id, keystroke);
  }

  pub fn clear_shortcut_override(shortcut_id: ShortcutId) {
    crate::keybindings_file::remove(shortcut_id);
  }
}

fn config_db_path_for_profile(base: Option<PathBuf>, profile: AppProfile) -> PathBuf {
  if let Some(base) = base {
    return base
      .join(match profile {
        AppProfile::Prod => CONFIG_DIR_NAME,
        AppProfile::Dev => profile.storage_dir_name(),
      })
      .join(CONFIG_DB_NAME);
  }

  PathBuf::from(match profile {
    AppProfile::Prod => CONFIG_DB_NAME,
    AppProfile::Dev => DEV_CONFIG_DB_NAME,
  })
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

  fn unique_test_repo_dir(label: &str) -> PathBuf {
    static NEXT_REPO_ID: AtomicU64 = AtomicU64::new(1);
    let id = NEXT_REPO_ID.fetch_add(1, Ordering::Relaxed);
    let path =
      std::env::temp_dir().join(format!("reviu-config-{label}-{}-{id}", std::process::id()));
    fs::create_dir_all(&path).expect("create temp repo dir");
    git2::Repository::init(&path).expect("init temp repo");
    path
  }

  #[test]
  fn recent_repositories_use_test_db_override() {
    let db_path = unique_test_db_path("recent");
    let _ = fs::remove_file(&db_path);
    ConfigStore::set_test_db_path(Some(db_path));

    let repo_a = unique_test_repo_dir("recent-a");
    let repo_b = unique_test_repo_dir("recent-b");
    ConfigStore::persist_recent_repository(&repo_a);
    ConfigStore::persist_recent_repository(&repo_b);

    let paths: Vec<PathBuf> = ConfigStore::load_recent_repositories()
      .into_iter()
      .map(|repo| repo.path)
      .collect();
    assert!(paths.contains(&repo_a));
    assert!(paths.contains(&repo_b));

    let _ = fs::remove_dir_all(&repo_a);
    let _ = fs::remove_dir_all(&repo_b);
    ConfigStore::set_test_db_path(None);
  }

  #[test]
  fn merge_method_round_trips_per_repository() {
    let db_path = unique_test_db_path("merge-method");
    let _ = fs::remove_file(&db_path);
    ConfigStore::set_test_db_path(Some(db_path));

    assert_eq!(ConfigStore::load_merge_method("acme/widget"), None);

    ConfigStore::persist_merge_method("acme/widget", GithubPullRequestMergeMethod::Squash);
    ConfigStore::persist_merge_method("acme/gadget", GithubPullRequestMergeMethod::Rebase);
    assert_eq!(
      ConfigStore::load_merge_method("acme/widget"),
      Some(GithubPullRequestMergeMethod::Squash)
    );
    assert_eq!(
      ConfigStore::load_merge_method("acme/gadget"),
      Some(GithubPullRequestMergeMethod::Rebase)
    );

    // The last choice wins, one row per repository.
    ConfigStore::persist_merge_method("acme/widget", GithubPullRequestMergeMethod::Merge);
    assert_eq!(
      ConfigStore::load_merge_method("acme/widget"),
      Some(GithubPullRequestMergeMethod::Merge)
    );

    ConfigStore::set_test_db_path(None);
  }

  #[test]
  fn a_merge_method_this_build_does_not_know_reads_as_nothing() {
    assert_eq!(merge_method_from_storage("rocket"), None);
    assert_eq!(
      merge_method_from_storage("squash"),
      Some(GithubPullRequestMergeMethod::Squash)
    );
  }

  #[test]
  fn forget_recent_repository_removes_entry() {
    let db_path = unique_test_db_path("forget");
    let _ = fs::remove_file(&db_path);
    ConfigStore::set_test_db_path(Some(db_path));

    let repo_a = unique_test_repo_dir("forget-a");
    let repo_b = unique_test_repo_dir("forget-b");
    ConfigStore::persist_recent_repository(&repo_a);
    ConfigStore::persist_recent_repository(&repo_b);
    ConfigStore::forget_recent_repository(&repo_a);

    let paths: Vec<PathBuf> = ConfigStore::load_recent_repositories()
      .into_iter()
      .map(|repo| repo.path)
      .collect();
    assert!(!paths.contains(&repo_a));
    assert!(paths.contains(&repo_b));

    let _ = fs::remove_dir_all(&repo_a);
    let _ = fs::remove_dir_all(&repo_b);
    ConfigStore::set_test_db_path(None);
  }

  #[test]
  fn session_sidebar_repository_order_round_trips_and_forget_removes_it() {
    let db_path = unique_test_db_path("session-sidebar-order");
    let _ = fs::remove_file(&db_path);
    ConfigStore::set_test_db_path(Some(db_path));

    let repo_a = unique_test_repo_dir("session-sidebar-order-a");
    let repo_b = unique_test_repo_dir("session-sidebar-order-b");
    ConfigStore::persist_session_sidebar_repositories(&[repo_b.clone(), repo_a.clone()]);

    assert_eq!(
      ConfigStore::load_session_sidebar_repositories(),
      vec![repo_b.clone(), repo_a.clone()]
    );

    ConfigStore::forget_recent_repository(&repo_b);
    assert_eq!(
      ConfigStore::load_session_sidebar_repositories(),
      vec![repo_a.clone()]
    );

    let _ = fs::remove_dir_all(&repo_a);
    let _ = fs::remove_dir_all(&repo_b);
    ConfigStore::set_test_db_path(None);
  }

  #[test]
  fn load_recent_repositories_drops_folders_that_stopped_being_repositories() {
    let db_path = unique_test_db_path("stale-repo");
    let _ = fs::remove_file(&db_path);
    ConfigStore::set_test_db_path(Some(db_path));

    let repo = unique_test_repo_dir("stale-repo-kept");
    let stale = unique_test_repo_dir("stale-repo-dropped");
    ConfigStore::persist_recent_repository(&repo);
    ConfigStore::persist_recent_repository(&stale);

    // The folder is still there, its repository is not.
    fs::remove_dir_all(stale.join(".git")).expect("remove .git");

    let paths: Vec<PathBuf> = ConfigStore::load_recent_repositories()
      .into_iter()
      .map(|repo| repo.path)
      .collect();
    assert_eq!(paths, vec![repo.clone()]);

    // Forgotten for good, not just filtered out of this read.
    fs::create_dir_all(stale.join(".git")).expect("recreate .git");
    let paths_again: Vec<PathBuf> = ConfigStore::load_recent_repositories()
      .into_iter()
      .map(|repo| repo.path)
      .collect();
    assert_eq!(paths_again, vec![repo.clone()]);

    let _ = fs::remove_dir_all(&repo);
    let _ = fs::remove_dir_all(&stale);
    ConfigStore::set_test_db_path(None);
  }

  #[test]
  fn load_recent_repositories_drops_paths_that_no_longer_exist() {
    let db_path = unique_test_db_path("autoclean");
    let _ = fs::remove_file(&db_path);
    ConfigStore::set_test_db_path(Some(db_path));

    let present = unique_test_repo_dir("autoclean-present");
    let missing = unique_test_repo_dir("autoclean-missing");
    ConfigStore::persist_recent_repository(&present);
    ConfigStore::persist_recent_repository(&missing);

    // Simulate the repo being deleted from disk after being persisted.
    fs::remove_dir_all(&missing).expect("remove missing repo");

    let paths: Vec<PathBuf> = ConfigStore::load_recent_repositories()
      .into_iter()
      .map(|repo| repo.path)
      .collect();
    assert_eq!(paths, vec![present.clone()]);

    // A second load should also not resurrect the missing entry.
    let paths_again: Vec<PathBuf> = ConfigStore::load_recent_repositories()
      .into_iter()
      .map(|repo| repo.path)
      .collect();
    assert_eq!(paths_again, vec![present.clone()]);

    let _ = fs::remove_dir_all(&present);
    ConfigStore::set_test_db_path(None);
  }

  #[test]
  fn app_settings_use_test_db_override() {
    let db_path = unique_test_db_path("settings");
    let _ = fs::remove_file(&db_path);
    ConfigStore::set_test_db_path(Some(db_path));

    let settings = AppSettings {
      auto_switch_theme: false,
      analytics_enabled: false,
      dark_mode: true,
      indent_rainbow: true,
      font_size: 20.0,
      git_unified_file_view: true,
      split_diff_view: true,
      hide_whitespace: true,
      menu_bar_icon: false,
      agent_notifications: false,
    };
    ConfigStore::persist_app_settings(settings);

    let loaded = ConfigStore::load_app_settings();
    assert!(!loaded.auto_switch_theme);
    assert!(loaded.dark_mode);
    assert!(loaded.indent_rainbow);
    assert_eq!(loaded.font_size, 20.0);
    assert!(loaded.git_unified_file_view);
    assert!(loaded.split_diff_view);
    assert!(loaded.hide_whitespace);
    assert!(!loaded.menu_bar_icon);

    ConfigStore::set_test_db_path(None);
  }

  #[test]
  fn shortcut_overrides_round_trip() {
    let db_path = unique_test_db_path("shortcut-overrides");
    let _ = fs::remove_file(&db_path);
    ConfigStore::set_test_db_path(Some(db_path));

    assert!(ConfigStore::load_shortcut_overrides().is_empty());

    ConfigStore::persist_shortcut_override(ShortcutId::ShowCommandPalette, "cmd-shift-k");
    ConfigStore::persist_shortcut_override(ShortcutId::CommitChanges, "cmd-j");

    let overrides = ConfigStore::load_shortcut_overrides();
    assert_eq!(
      overrides.get(&ShortcutId::ShowCommandPalette),
      Some(&"cmd-shift-k".to_string())
    );
    assert_eq!(
      overrides.get(&ShortcutId::CommitChanges),
      Some(&"cmd-j".to_string())
    );

    ConfigStore::clear_shortcut_override(ShortcutId::ShowCommandPalette);
    let overrides = ConfigStore::load_shortcut_overrides();
    assert!(!overrides.contains_key(&ShortcutId::ShowCommandPalette));
    assert_eq!(
      overrides.get(&ShortcutId::CommitChanges),
      Some(&"cmd-j".to_string())
    );

    ConfigStore::set_test_db_path(None);
  }

  #[test]
  fn config_db_path_uses_profile_namespace() {
    let base = PathBuf::from("/tmp/reviu-config");

    assert_eq!(
      config_db_path_for_profile(Some(base.clone()), AppProfile::Prod),
      PathBuf::from("/tmp/reviu-config/reviu/reviu.sqlite")
    );
    assert_eq!(
      config_db_path_for_profile(Some(base), AppProfile::Dev),
      PathBuf::from("/tmp/reviu-config/reviu.dev/reviu.sqlite")
    );
  }

  #[test]
  fn config_db_path_uses_profile_specific_fallback_file_names() {
    assert_eq!(
      config_db_path_for_profile(None, AppProfile::Prod),
      PathBuf::from("reviu.sqlite")
    );
    assert_eq!(
      config_db_path_for_profile(None, AppProfile::Dev),
      PathBuf::from("reviu.dev.sqlite")
    );
  }

  #[test]
  fn command_usages_round_trip() {
    let db_path = unique_test_db_path("command-usages");
    let _ = fs::remove_file(&db_path);
    ConfigStore::set_test_db_path(Some(db_path));

    ConfigStore::persist_command_usage("commit", &[1_000, 2_000, 3_000]);
    ConfigStore::persist_command_usage("push", &[5_000]);

    let usages = ConfigStore::load_command_usages();
    assert_eq!(usages.get("commit"), Some(&vec![1_000, 2_000, 3_000]));
    assert_eq!(usages.get("push"), Some(&vec![5_000]));

    ConfigStore::persist_command_usage("commit", &[1_000, 2_000, 3_000, 4_000]);
    let usages = ConfigStore::load_command_usages();
    assert_eq!(
      usages.get("commit"),
      Some(&vec![1_000, 2_000, 3_000, 4_000])
    );

    ConfigStore::set_test_db_path(None);
  }

  fn settings_columns(db_path: &Path) -> std::collections::HashSet<String> {
    let conn = Connection::open(db_path).expect("open db");
    let mut stmt = conn
      .prepare(&format!("PRAGMA table_info({})", SETTINGS_TABLE.name))
      .expect("table_info");
    let rows = stmt
      .query_map([], |row| row.get::<_, String>(1))
      .expect("query columns");
    rows.map(|row| row.expect("column name")).collect()
  }

  fn db_user_version(db_path: &Path) -> i64 {
    Connection::open(db_path)
      .expect("open db")
      .query_row("PRAGMA user_version", [], |row| row.get(0))
      .expect("user_version")
  }

  const ALL_SETTINGS_COLUMNS: &[&str] = &[
    "auto_switch_theme",
    "dark_mode",
    "indent_rainbow",
    "font_size",
    "git_unified_file_view",
    "split_diff_view",
    "hide_whitespace",
    "menu_bar_icon",
    "analytics_enabled",
  ];

  #[test]
  fn migrations_stamp_version_and_full_schema_on_fresh_db() {
    let db_path = unique_test_db_path("migrate-fresh");
    let _ = fs::remove_file(&db_path);
    ConfigStore::set_test_db_path(Some(db_path.clone()));

    // Triggers open_with_tables -> run_migrations on an empty database.
    let _ = ConfigStore::load_app_settings();

    assert_eq!(db_user_version(&db_path), MIGRATIONS.len() as i64);
    let columns = settings_columns(&db_path);
    for column in ALL_SETTINGS_COLUMNS {
      assert!(columns.contains(*column), "missing column {column}");
    }

    ConfigStore::set_test_db_path(None);
  }

  #[test]
  fn migrations_are_idempotent_across_runs() {
    let db_path = unique_test_db_path("migrate-idempotent");
    let _ = fs::remove_file(&db_path);
    ConfigStore::set_test_db_path(Some(db_path.clone()));

    let _ = ConfigStore::load_app_settings();
    let mut settings = ConfigStore::load_app_settings();
    settings.dark_mode = true;
    ConfigStore::persist_app_settings(settings);
    // Second open must not re-run v1 or disturb stored data.
    let reloaded = ConfigStore::load_app_settings();

    assert_eq!(db_user_version(&db_path), MIGRATIONS.len() as i64);
    assert!(reloaded.dark_mode);

    ConfigStore::set_test_db_path(None);
  }

  /// A database already stamped at the current version keeps its values and is
  /// not migrated again.
  #[test]
  fn an_already_versioned_db_is_left_alone() {
    let db_path = unique_test_db_path("migrate-versioned");
    let _ = fs::remove_file(&db_path);

    {
      let conn = Connection::open(&db_path).expect("open db");
      conn.execute(SETTINGS_TABLE.create_sql, []).expect("create");
      ensure_default_rows(&conn).expect("default row");
      ensure_settings_columns(&conn).expect("v1 columns");
      set_schema_version(&conn, MIGRATIONS.len() as i64).expect("stamp current version");
      conn
        .execute("UPDATE settings SET dark_mode = 1 WHERE id = 1", [])
        .expect("seed value");
    }

    ConfigStore::set_test_db_path(Some(db_path.clone()));
    let settings = ConfigStore::load_app_settings();

    assert!(settings.dark_mode, "stored value must survive the open");
    assert_eq!(db_user_version(&db_path), MIGRATIONS.len() as i64);

    ConfigStore::set_test_db_path(None);
  }

  /// A retired setting leaves its column behind on existing installs; reads and
  /// writes name their columns, so the extra one is simply ignored.
  #[test]
  fn a_db_carrying_a_retired_setting_column_still_loads_and_saves() {
    let db_path = unique_test_db_path("retired-column");
    let _ = fs::remove_file(&db_path);

    {
      let conn = Connection::open(&db_path).expect("open db");
      conn.execute(SETTINGS_TABLE.create_sql, []).expect("create");
      ensure_default_rows(&conn).expect("default row");
      ensure_settings_columns(&conn).expect("v1 columns");
      add_settings_column_if_missing(&conn, "home_page", "TEXT NOT NULL DEFAULT 'session'")
        .expect("retired column");
      set_schema_version(&conn, MIGRATIONS.len() as i64).expect("stamp current version");
    }

    ConfigStore::set_test_db_path(Some(db_path.clone()));
    let mut settings = ConfigStore::load_app_settings();
    settings.font_size = 18.0;
    ConfigStore::persist_app_settings(settings);

    assert_eq!(ConfigStore::load_app_settings().font_size, 18.0);
    assert!(settings_columns(&db_path).contains("home_page"));

    ConfigStore::set_test_db_path(None);
  }

  #[test]
  fn migrations_upgrade_legacy_db_without_data_loss() {
    let db_path = unique_test_db_path("migrate-legacy");
    let _ = fs::remove_file(&db_path);

    // Simulate an old install: only the original columns, user_version left at 0.
    {
      let conn = Connection::open(&db_path).expect("open legacy db");
      conn
        .execute_batch(
          "CREATE TABLE settings (id INTEGER PRIMARY KEY CHECK (id = 1), \
           auto_switch_theme INTEGER NOT NULL DEFAULT 1, \
           dark_mode INTEGER NOT NULL DEFAULT 0); \
           INSERT INTO settings (id, dark_mode) VALUES (1, 1);",
        )
        .expect("seed legacy schema");
      assert_eq!(db_user_version(&db_path), 0);
    }

    ConfigStore::set_test_db_path(Some(db_path.clone()));
    let settings = ConfigStore::load_app_settings();

    // Pre-existing value preserved, schema brought up to date, version stamped.
    assert!(settings.dark_mode, "legacy value must survive migration");
    assert_eq!(db_user_version(&db_path), MIGRATIONS.len() as i64);
    let columns = settings_columns(&db_path);
    for column in ALL_SETTINGS_COLUMNS {
      assert!(columns.contains(*column), "missing column {column}");
    }

    // The load imported the sqlite values into the settings file.
    assert!(db_path.with_extension("settings.json").exists());
    assert!(ConfigStore::load_app_settings().dark_mode);

    ConfigStore::set_test_db_path(None);
  }

  #[test]
  fn shortcut_rows_are_imported_into_the_keybindings_file() {
    let db_path = unique_test_db_path("keybindings-import");
    let _ = fs::remove_file(&db_path);
    let file_path = db_path.with_extension("keybindings.json");
    let _ = fs::remove_file(&file_path);

    {
      let conn = Connection::open(&db_path).expect("open db");
      conn
        .execute(SHORTCUT_OVERRIDES_TABLE.create_sql, [])
        .expect("create");
      conn
        .execute(
          "INSERT INTO shortcut_overrides (shortcut_id, keystroke) VALUES ('commit_changes', 'cmd-j')",
          [],
        )
        .expect("seed row");
      conn
        .execute(
          "INSERT INTO shortcut_overrides (shortcut_id, keystroke) VALUES ('close_workspace_page', 'cmd-w')",
          [],
        )
        .expect("seed retired row");
    }

    ConfigStore::set_test_db_path(Some(db_path));
    let overrides = ConfigStore::load_shortcut_overrides();

    assert_eq!(
      overrides.get(&ShortcutId::CommitChanges),
      Some(&"cmd-j".to_string())
    );
    assert!(file_path.exists(), "import must stamp the keybindings file");
    let raw = fs::read_to_string(&file_path).expect("read keybindings file");
    assert!(!raw.contains("close_workspace_page"));
    assert_eq!(
      ConfigStore::load_shortcut_overrides().get(&ShortcutId::CommitChanges),
      Some(&"cmd-j".to_string())
    );

    ConfigStore::set_test_db_path(None);
  }
}
