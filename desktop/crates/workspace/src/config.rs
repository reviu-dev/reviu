use gpui::{App, Global};
#[cfg(test)]
use std::cell::RefCell;
use std::{
  collections::HashMap,
  fs,
  path::{Path, PathBuf},
  time::{SystemTime, UNIX_EPOCH},
};

use dirs::config_dir;
use gpui::Keystroke;
use rusqlite::{Connection, params};
use serde_json;

use crate::AppProfile;
use crate::github_home_tabs::{
  GithubHomePullRequestTab, normalize_github_home_pull_request_tab,
  seed_github_home_pull_request_tabs,
};
use crate::shortcuts::ShortcutId;

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

const SETTINGS_TABLE: ConfigTable = ConfigTable {
  name: "settings",
  create_sql: "CREATE TABLE IF NOT EXISTS settings (id INTEGER PRIMARY KEY CHECK (id = 1), auto_switch_theme INTEGER NOT NULL DEFAULT 1, dark_mode INTEGER NOT NULL DEFAULT 0, indent_rainbow INTEGER NOT NULL DEFAULT 0)",
};

const PINNED_REPOS_TABLE: ConfigTable = ConfigTable {
  name: "pinned_repos",
  create_sql: "CREATE TABLE IF NOT EXISTS pinned_repos (full_name TEXT PRIMARY KEY, pinned_at INTEGER NOT NULL)",
};

const SHORTCUT_OVERRIDES_TABLE: ConfigTable = ConfigTable {
  name: "shortcut_overrides",
  create_sql: "CREATE TABLE IF NOT EXISTS shortcut_overrides (shortcut_id TEXT PRIMARY KEY, keystroke TEXT NOT NULL)",
};

const GITHUB_HOME_PULL_REQUEST_TABS_TABLE: ConfigTable = ConfigTable {
  name: "github_home_pull_request_tabs",
  create_sql: "CREATE TABLE IF NOT EXISTS github_home_pull_request_tabs (id TEXT PRIMARY KEY, name TEXT NOT NULL, position INTEGER NOT NULL, filters_json TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL)",
};

const COMMAND_USAGES_TABLE: ConfigTable = ConfigTable {
  name: "command_usages",
  create_sql: "CREATE TABLE IF NOT EXISTS command_usages (command_id TEXT PRIMARY KEY, recent_timestamps TEXT NOT NULL)",
};

pub const COMMAND_USAGE_TIMESTAMP_CAP: usize = 30;

const CONFIG_TABLES: [ConfigTable; 6] = [
  RECENT_REPOS_TABLE,
  SETTINGS_TABLE,
  PINNED_REPOS_TABLE,
  SHORTCUT_OVERRIDES_TABLE,
  GITHUB_HOME_PULL_REQUEST_TABS_TABLE,
  COMMAND_USAGES_TABLE,
];

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloneProtocol {
  Https,
  Ssh,
}

impl CloneProtocol {
  pub fn as_str(self) -> &'static str {
    match self {
      Self::Https => "https",
      Self::Ssh => "ssh",
    }
  }

  pub fn from_str(value: &str) -> Self {
    match value {
      "ssh" => Self::Ssh,
      _ => Self::Https,
    }
  }
}

#[derive(Clone, Copy, Debug)]
pub struct AppSettings {
  pub auto_switch_theme: bool,
  pub dark_mode: bool,
  pub indent_rainbow: bool,
  pub font_size: f32,
  pub git_unified_file_view: bool,
  pub split_diff_view: bool,
  pub hide_whitespace: bool,
  pub clone_protocol: CloneProtocol,
  pub menu_bar_icon: bool,
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
      clone_protocol: CloneProtocol::Https,
      menu_bar_icon: true,
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
    let mut has_font_size = false;
    let mut has_git_unified_file_view = false;
    let mut has_split_diff_view = false;
    let mut has_hide_whitespace = false;
    let mut has_clone_protocol = false;
    let mut has_menu_bar_icon = false;
    let mut stmt = self
      .conn
      .prepare(&format!("PRAGMA table_info({})", SETTINGS_TABLE.name))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
      let column = row?;
      if column == "indent_rainbow" {
        has_indent_rainbow = true;
      }
      if column == "font_size" {
        has_font_size = true;
      }
      if column == "git_unified_file_view" {
        has_git_unified_file_view = true;
      }
      if column == "split_diff_view" {
        has_split_diff_view = true;
      }
      if column == "hide_whitespace" {
        has_hide_whitespace = true;
      }
      if column == "clone_protocol" {
        has_clone_protocol = true;
      }
      if column == "menu_bar_icon" {
        has_menu_bar_icon = true;
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

    if !has_font_size {
      self.conn.execute(
        &format!(
          "ALTER TABLE {} ADD COLUMN font_size REAL NOT NULL DEFAULT 16.0",
          SETTINGS_TABLE.name
        ),
        [],
      )?;
    }

    if !has_git_unified_file_view {
      self.conn.execute(
        &format!(
          "ALTER TABLE {} ADD COLUMN git_unified_file_view INTEGER NOT NULL DEFAULT 0",
          SETTINGS_TABLE.name
        ),
        [],
      )?;
    }

    if !has_split_diff_view {
      self.conn.execute(
        &format!(
          "ALTER TABLE {} ADD COLUMN split_diff_view INTEGER NOT NULL DEFAULT 0",
          SETTINGS_TABLE.name
        ),
        [],
      )?;
    }

    if !has_hide_whitespace {
      self.conn.execute(
        &format!(
          "ALTER TABLE {} ADD COLUMN hide_whitespace INTEGER NOT NULL DEFAULT 0",
          SETTINGS_TABLE.name
        ),
        [],
      )?;
    }

    if !has_clone_protocol {
      self.conn.execute(
        &format!(
          "ALTER TABLE {} ADD COLUMN clone_protocol TEXT NOT NULL DEFAULT 'https'",
          SETTINGS_TABLE.name
        ),
        [],
      )?;
    }

    if !has_menu_bar_icon {
      self.conn.execute(
        &format!(
          "ALTER TABLE {} ADD COLUMN menu_bar_icon INTEGER NOT NULL DEFAULT 1",
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
    let mut missing_paths: Vec<String> = Vec::new();
    for row in rows {
      match row {
        Ok(path_string) => {
          let path = PathBuf::from(&path_string);
          if path.is_dir() {
            repositories.push(RecentRepository { path });
          } else {
            missing_paths.push(path_string);
          }
        }
        Err(err) => eprintln!("Failed to decode recent repository row: {}", err),
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
      eprintln!("Failed to forget recent repository: {}", err);
    }
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
        "SELECT auto_switch_theme, dark_mode, indent_rainbow, font_size, git_unified_file_view, split_diff_view, hide_whitespace, clone_protocol, menu_bar_icon FROM {} WHERE id = 1",
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
        let clone_protocol: String = row.get(7)?;
        let menu_bar_icon: i64 = row.get(8)?;
        Ok(AppSettings {
          auto_switch_theme: auto_switch_theme != 0,
          dark_mode: dark_mode != 0,
          indent_rainbow: indent_rainbow != 0,
          font_size: font_size as f32,
          git_unified_file_view: git_unified_file_view != 0,
          split_diff_view: split_diff_view != 0,
          hide_whitespace: hide_whitespace != 0,
          clone_protocol: CloneProtocol::from_str(&clone_protocol),
          menu_bar_icon: menu_bar_icon != 0,
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
        eprintln!("Failed to load command usages: {}", err);
        return HashMap::default();
      }
    };

    let rows = match stmt.query_map([], |row| {
      Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }) {
      Ok(rows) => rows,
      Err(err) => {
        eprintln!("Failed to read command usages: {}", err);
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
        eprintln!("Failed to serialize command usage timestamps: {}", err);
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
      eprintln!("Failed to persist command usage: {}", err);
    }
  }

  pub fn load_pinned_repos() -> Vec<String> {
    let Some(store) = Self::open_with_tables() else {
      return Vec::new();
    };
    store.load_pinned_repos_inner()
  }

  pub fn load_shortcut_overrides() -> HashMap<ShortcutId, String> {
    let Some(store) = Self::open_with_tables() else {
      return HashMap::default();
    };
    store.load_shortcut_overrides_inner()
  }

  pub fn load_or_seed_github_home_pull_request_tabs() -> Vec<GithubHomePullRequestTab> {
    let Some(store) = Self::open_with_tables() else {
      return seed_github_home_pull_request_tabs();
    };
    let tabs = store.load_github_home_pull_request_tabs_inner();
    if !tabs.is_empty() {
      return tabs;
    }

    let seeded_tabs = seed_github_home_pull_request_tabs();
    store.persist_github_home_pull_request_tabs_inner(&seeded_tabs);
    store.load_github_home_pull_request_tabs_inner()
  }

  pub fn persist_github_home_pull_request_tabs(tabs: &[GithubHomePullRequestTab]) {
    let Some(store) = Self::open_with_tables() else {
      return;
    };
    store.persist_github_home_pull_request_tabs_inner(tabs);
  }

  fn load_shortcut_overrides_inner(&self) -> HashMap<ShortcutId, String> {
    let mut stmt = match self.conn.prepare(&format!(
      "SELECT shortcut_id, keystroke FROM {} ORDER BY shortcut_id ASC",
      SHORTCUT_OVERRIDES_TABLE.name
    )) {
      Ok(stmt) => stmt,
      Err(err) => {
        eprintln!("Failed to load shortcut overrides: {}", err);
        return HashMap::default();
      }
    };

    let rows = match stmt.query_map([], |row| {
      Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }) {
      Ok(rows) => rows,
      Err(err) => {
        eprintln!("Failed to read shortcut overrides: {}", err);
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

  fn load_pinned_repos_inner(&self) -> Vec<String> {
    let mut stmt = match self.conn.prepare(&format!(
      "SELECT full_name FROM {} ORDER BY pinned_at ASC",
      PINNED_REPOS_TABLE.name
    )) {
      Ok(stmt) => stmt,
      Err(err) => {
        eprintln!("Failed to load pinned repos: {}", err);
        return Vec::new();
      }
    };

    let rows = match stmt.query_map([], |row| row.get::<_, String>(0)) {
      Ok(rows) => rows,
      Err(err) => {
        eprintln!("Failed to read pinned repos: {}", err);
        return Vec::new();
      }
    };

    rows.filter_map(|r| r.ok()).collect()
  }

  fn load_github_home_pull_request_tabs_inner(&self) -> Vec<GithubHomePullRequestTab> {
    let mut stmt = match self.conn.prepare(&format!(
      "SELECT id, name, filters_json FROM {} ORDER BY position ASC",
      GITHUB_HOME_PULL_REQUEST_TABS_TABLE.name
    )) {
      Ok(stmt) => stmt,
      Err(err) => {
        eprintln!("Failed to load GitHub home pull request tabs: {}", err);
        return Vec::new();
      }
    };

    let rows = match stmt.query_map([], |row| {
      Ok((
        row.get::<_, String>(0)?,
        row.get::<_, String>(1)?,
        row.get::<_, String>(2)?,
      ))
    }) {
      Ok(rows) => rows,
      Err(err) => {
        eprintln!("Failed to read GitHub home pull request tabs: {}", err);
        return Vec::new();
      }
    };

    let mut tabs = Vec::new();
    for row in rows {
      let Ok((id, name, filters_json)) = row else {
        continue;
      };
      let Ok(filters) = serde_json::from_str(&filters_json) else {
        continue;
      };
      tabs.push(normalize_github_home_pull_request_tab(
        &GithubHomePullRequestTab { id, name, filters },
      ));
    }
    tabs.retain(|tab| !tab.id.is_empty() && !tab.name.is_empty());
    tabs
  }

  pub fn pin_repo(full_name: &str) {
    let Some(store) = Self::open_with_tables() else {
      return;
    };
    store.pin_repo_inner(full_name);
  }

  fn pin_repo_inner(&self, full_name: &str) {
    let pinned_at = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .unwrap_or_default()
      .as_secs() as i64;

    if let Err(err) = self.conn.execute(
      &format!(
        "INSERT INTO {} (full_name, pinned_at) VALUES (?1, ?2)
         ON CONFLICT(full_name) DO NOTHING",
        PINNED_REPOS_TABLE.name
      ),
      params![full_name, pinned_at],
    ) {
      eprintln!("Failed to pin repo: {}", err);
    }
  }

  pub fn unpin_repo(full_name: &str) {
    let Some(store) = Self::open_with_tables() else {
      return;
    };
    store.unpin_repo_inner(full_name);
  }

  pub fn persist_shortcut_override(shortcut_id: ShortcutId, keystroke: &str) {
    let Some(store) = Self::open_with_tables() else {
      return;
    };
    store.persist_shortcut_override_inner(shortcut_id, keystroke);
  }

  fn persist_shortcut_override_inner(&self, shortcut_id: ShortcutId, keystroke: &str) {
    if let Err(err) = self.conn.execute(
      &format!(
        "INSERT INTO {} (shortcut_id, keystroke) VALUES (?1, ?2)
         ON CONFLICT(shortcut_id) DO UPDATE SET keystroke = excluded.keystroke",
        SHORTCUT_OVERRIDES_TABLE.name
      ),
      params![shortcut_id.storage_key(), keystroke],
    ) {
      eprintln!("Failed to persist shortcut override: {}", err);
    }
  }

  pub fn clear_shortcut_override(shortcut_id: ShortcutId) {
    let Some(store) = Self::open_with_tables() else {
      return;
    };
    store.clear_shortcut_override_inner(shortcut_id);
  }

  fn clear_shortcut_override_inner(&self, shortcut_id: ShortcutId) {
    if let Err(err) = self.conn.execute(
      &format!(
        "DELETE FROM {} WHERE shortcut_id = ?1",
        SHORTCUT_OVERRIDES_TABLE.name
      ),
      params![shortcut_id.storage_key()],
    ) {
      eprintln!("Failed to clear shortcut override: {}", err);
    }
  }

  fn unpin_repo_inner(&self, full_name: &str) {
    if let Err(err) = self.conn.execute(
      &format!(
        "DELETE FROM {} WHERE full_name = ?1",
        PINNED_REPOS_TABLE.name
      ),
      params![full_name],
    ) {
      eprintln!("Failed to unpin repo: {}", err);
    }
  }

  fn persist_app_settings_inner(&self, settings: AppSettings) {
    if let Err(err) = self.conn.execute(
      &format!(
        "INSERT INTO {} (id, auto_switch_theme, dark_mode, indent_rainbow, font_size, git_unified_file_view, split_diff_view, hide_whitespace, clone_protocol, menu_bar_icon)
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(id) DO UPDATE
         SET auto_switch_theme = excluded.auto_switch_theme,
             dark_mode = excluded.dark_mode,
             indent_rainbow = excluded.indent_rainbow,
             font_size = excluded.font_size,
             git_unified_file_view = excluded.git_unified_file_view,
             split_diff_view = excluded.split_diff_view,
             hide_whitespace = excluded.hide_whitespace,
             clone_protocol = excluded.clone_protocol,
             menu_bar_icon = excluded.menu_bar_icon",
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
        },
        settings.font_size as f64,
        if settings.git_unified_file_view {
          1_i64
        } else {
          0_i64
        },
        if settings.split_diff_view {
          1_i64
        } else {
          0_i64
        },
        if settings.hide_whitespace {
          1_i64
        } else {
          0_i64
        },
        settings.clone_protocol.as_str(),
        if settings.menu_bar_icon { 1_i64 } else { 0_i64 },
      ],
    ) {
      eprintln!("Failed to persist app settings: {}", err);
    }
  }

  fn persist_github_home_pull_request_tabs_inner(&self, tabs: &[GithubHomePullRequestTab]) {
    if let Err(err) = self.conn.execute(
      &format!("DELETE FROM {}", GITHUB_HOME_PULL_REQUEST_TABS_TABLE.name),
      [],
    ) {
      eprintln!("Failed to clear GitHub home pull request tabs: {}", err);
      return;
    }

    let timestamp = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .unwrap_or_default()
      .as_secs() as i64;

    for (position, tab) in tabs
      .iter()
      .map(normalize_github_home_pull_request_tab)
      .enumerate()
    {
      let Ok(filters_json) = serde_json::to_string(&tab.filters) else {
        continue;
      };

      if let Err(err) = self.conn.execute(
        &format!(
          "INSERT INTO {} (id, name, position, filters_json, created_at, updated_at)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
          GITHUB_HOME_PULL_REQUEST_TABS_TABLE.name
        ),
        params![
          tab.id,
          tab.name,
          position as i64,
          filters_json,
          timestamp,
          timestamp,
        ],
      ) {
        eprintln!("Failed to persist GitHub home pull request tab: {}", err);
        return;
      }
    }
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
      dark_mode: true,
      indent_rainbow: true,
      font_size: 20.0,
      git_unified_file_view: true,
      split_diff_view: true,
      hide_whitespace: true,
      clone_protocol: CloneProtocol::Ssh,
      menu_bar_icon: false,
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
    assert_eq!(loaded.clone_protocol, CloneProtocol::Ssh);
    assert!(!loaded.menu_bar_icon);

    ConfigStore::set_test_db_path(None);
  }

  #[test]
  fn pinned_repos_round_trip() {
    let db_path = unique_test_db_path("pinned");
    let _ = fs::remove_file(&db_path);
    ConfigStore::set_test_db_path(Some(db_path));

    assert!(ConfigStore::load_pinned_repos().is_empty());

    ConfigStore::pin_repo("owner/repo-a");
    ConfigStore::pin_repo("owner/repo-b");

    let pinned = ConfigStore::load_pinned_repos();
    assert_eq!(pinned.len(), 2);
    assert!(pinned.contains(&"owner/repo-a".to_string()));
    assert!(pinned.contains(&"owner/repo-b".to_string()));

    // Pin again should not duplicate
    ConfigStore::pin_repo("owner/repo-a");
    assert_eq!(ConfigStore::load_pinned_repos().len(), 2);

    ConfigStore::unpin_repo("owner/repo-a");
    let pinned = ConfigStore::load_pinned_repos();
    assert_eq!(pinned, vec!["owner/repo-b".to_string()]);

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
  fn github_home_pull_request_tabs_seed_and_round_trip() {
    let db_path = unique_test_db_path("github-home-tabs");
    let _ = fs::remove_file(&db_path);
    ConfigStore::set_test_db_path(Some(db_path));

    let seeded_tabs = ConfigStore::load_or_seed_github_home_pull_request_tabs();
    assert_eq!(seeded_tabs.len(), 2);
    assert_eq!(seeded_tabs[0].name, "My Open PRs");
    assert!(seeded_tabs[0].filters.include_drafts);
    assert_eq!(seeded_tabs[1].name, "Need Review");

    let custom_tabs = vec![GithubHomePullRequestTab {
      id: "custom-1".to_string(),
      name: "Frontend".to_string(),
      filters: crate::github_home_tabs::GithubPullRequestSearchFilters {
        repos: vec!["acme/reviu".to_string()],
        labels: vec!["frontend".to_string()],
        excluded_labels: vec!["dependencies".to_string()],
        authors: vec!["alice".to_string()],
        assignees: vec!["bob".to_string()],
        requested_reviewers: vec!["@me".to_string()],
        review_status: crate::github_home_tabs::GithubPullRequestReviewStatus::Required,
        include_drafts: false,
        base: None,
        sort: Default::default(),
      },
    }];

    ConfigStore::persist_github_home_pull_request_tabs(&custom_tabs);
    let loaded_tabs = ConfigStore::load_or_seed_github_home_pull_request_tabs();
    assert_eq!(loaded_tabs, custom_tabs);

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
}
