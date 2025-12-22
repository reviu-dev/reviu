use crate::error::Result;
use crate::git::diff::DiffEngine;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
/// Global application state
#[derive(Debug, Clone)]
pub struct AppState {
  pub auth: AuthState,
  pub workspace: Workspace,
  pub ui: UIState,
  pub config: Config,
}

impl AppState {
  pub fn new() -> Self {
    Self {
      auth: AuthState::default(),
      workspace: Workspace::default(),
      ui: UIState::default(),
      config: Config::default(),
    }
  }
}

impl Default for AppState {
  fn default() -> Self {
    Self::new()
  }
}

/// Authentication state
#[derive(Debug, Clone, Default)]
pub struct AuthState {
  pub token: Option<String>,
  pub user: Option<User>,
  pub premium: bool,
}

/// User information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
  pub id: String,
  pub email: String,
  pub name: Option<String>,
  pub avatar_url: Option<String>,
  pub premium: bool,
}

/// Workspace managing multiple repositories
#[derive(Debug, Clone, Default)]
pub struct Workspace {
  pub repos: HashMap<PathBuf, Repository>,
  pub active_repo: Option<PathBuf>,
  pub recent_repos: Vec<PathBuf>,
}

impl Workspace {
  pub fn get_active_repo(&self) -> Option<&Repository> {
    self
      .active_repo
      .as_ref()
      .and_then(|path| self.repos.get(path))
  }

  pub fn get_active_repo_mut(&mut self) -> Option<&mut Repository> {
    self
      .active_repo
      .as_ref()
      .and_then(|path| self.repos.get_mut(path))
  }

  pub fn add_repository(&mut self, path: PathBuf, repo: Repository) {
    self.repos.insert(path.clone(), repo);
    self.set_active_repo(path);
  }

  pub fn set_active_repo(&mut self, path: PathBuf) {
    self.active_repo = Some(path.clone());

    // Update recent repos
    if let Some(pos) = self.recent_repos.iter().position(|p| p == &path) {
      self.recent_repos.remove(pos);
    }
    self.recent_repos.insert(0, path);

    // Keep only last 10 recent repos
    if self.recent_repos.len() > 10 {
      self.recent_repos.truncate(10);
    }
  }
}

/// Repository state
#[derive(Debug, Clone)]
pub struct Repository {
  pub path: PathBuf,
  pub name: String,
  pub head: Option<String>,
  pub remote: Option<String>,
  pub status: GitStatus,
  pub diff: Option<DiffState>,
  pub selected_files: Vec<PathBuf>,
  pub staged_hunks: HashSet<HunkId>,
}

impl Repository {
  pub fn new(path: PathBuf, name: String) -> Self {
    Self {
      path,
      name,
      head: None,
      remote: None,
      status: GitStatus::default(),
      diff: None,
      selected_files: Vec::new(),
      staged_hunks: HashSet::new(),
    }
  }
}

/// Git status information
#[derive(Debug, Clone, Default)]
pub struct GitStatus {
  pub files: Vec<FileStatus>,
  pub branch: Option<String>,
  pub ahead: usize,
  pub behind: usize,
}

/// File status in the repository
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStatus {
  pub path: PathBuf,
  pub status: FileStatusKind,
  pub staged: bool,
}

/// Kind of file status change
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileStatusKind {
  Untracked,
  Modified,
  Added,
  Deleted,
  Renamed { from: Option<PathBuf> },
  Copied { from: Option<PathBuf> },
}

impl FileStatusKind {
  pub fn as_str(&self) -> &str {
    match self {
      FileStatusKind::Untracked => "Untracked",
      FileStatusKind::Modified => "Modified",
      FileStatusKind::Added => "Added",
      FileStatusKind::Deleted => "Deleted",
      FileStatusKind::Renamed { .. } => "Renamed",
      FileStatusKind::Copied { .. } => "Copied",
    }
  }

  pub fn short_str(&self) -> &str {
    match self {
      FileStatusKind::Untracked => "U",
      FileStatusKind::Modified => "M",
      FileStatusKind::Added => "A",
      FileStatusKind::Deleted => "D",
      FileStatusKind::Renamed { .. } => "R",
      FileStatusKind::Copied { .. } => "C",
    }
  }
}

/// Diff state for a file or set of files
#[derive(Debug, Clone)]
pub struct DiffState {
  pub files: Vec<FileDiff>,
}

/// Diff information for a single file
#[derive(Debug, Clone)]
pub struct FileDiff {
  pub path: PathBuf,
  pub old_path: Option<PathBuf>,
  pub status: FileStatusKind,
  pub hunks: Vec<Hunk>,
  /// Full file content for lazy line loading
  pub old_content: Option<String>,
  pub new_content: Option<String>,
}

/// A hunk of changes in a diff (metadata only, lines loaded on-demand)
#[derive(Debug, Clone)]
pub struct Hunk {
  pub id: HunkId,
  pub old_start: u32,
  pub old_lines: u32,
  pub new_start: u32,
  pub new_lines: u32,
  pub header: String,
  /// Byte range in old_content for this hunk
  pub old_byte_range: std::ops::Range<usize>,
  /// Byte range in new_content for this hunk
  pub new_byte_range: std::ops::Range<usize>,
  /// Parsed lines for this hunk (additions, deletions, context)
  /// Note: Lines are parsed by git2 and stored here for rendering
  pub lines: Vec<Line>,
  pub context_expanded: bool,
}

impl Hunk {
  // Note: Lines are now always parsed by git2 and stored in hunk.lines
  // The byte ranges (old_byte_range, new_byte_range) are kept for potential
  // future optimizations but are not currently used for line extraction
}

/// Unique identifier for a hunk
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HunkId(pub u64);

/// A line in a diff
#[derive(Debug, Clone)]
pub struct Line {
  pub origin: LineOrigin,
  pub content: String,
  pub old_lineno: Option<u32>,
  pub new_lineno: Option<u32>,
}

/// Origin/type of a diff line
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineOrigin {
  Context,
  Addition,
  Deletion,
}

/// UI state
#[derive(Debug, Clone)]
pub struct UIState {
  pub command_palette_open: bool,
  pub active_panel: Panel,
}

impl Default for UIState {
  fn default() -> Self {
    Self {
      command_palette_open: false,
      active_panel: Panel::FileList,
    }
  }
}

/// Active panel in the UI
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
  FileList,
}

/// Scroll state for virtualized lists
#[derive(Debug, Clone, Default)]
pub struct ScrollState {
  pub file_list: f32,
}

/// Application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
  pub theme: Theme,
  pub editor: EditorConfig,
  pub keybindings: HashMap<String, String>,
}

impl Default for Config {
  fn default() -> Self {
    Self {
      theme: Theme::System,
      editor: EditorConfig::default(),
      keybindings: HashMap::new(),
    }
  }
}

/// Theme setting
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
  Light,
  Dark,
  System,
}

/// Editor configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorConfig {
  pub tab_size: usize,
  pub font_family: String,
  pub font_size: f32,
  pub line_numbers: bool,
}

impl Default for EditorConfig {
  fn default() -> Self {
    Self {
      tab_size: 2,
      font_family: "Monaco".to_string(),
      font_size: 13.0,
      line_numbers: true,
    }
  }
}

/// Actions that can be dispatched to update state
#[derive(Debug, Clone)]
pub enum Action {
  // Repository operations
  LoadRepository(PathBuf),
  SelectFile(PathBuf),
}

/// Update the application state based on an action
pub fn update(state: &mut AppState, action: Action) -> Result<()> {
  match action {
    Action::LoadRepository(path) => {
      log::info!("Loading repository: {:?}", path);

      // Open the git repository
      let git_repo = crate::git::open_repository(&path)?;
      let git_status = crate::git::get_repository_status(git_repo.repo())?;

      // Get repository info
      let name = git_repo.info().name.clone();
      let head = git_repo.info().head.clone();
      let remote = git_repo.info().remote.clone();

      // Create Repository state
      let repo = Repository {
        path: path.clone(),
        name,
        head,
        remote,
        status: git_status,
        diff: None,
        selected_files: Vec::new(),
        staged_hunks: std::collections::HashSet::new(),
      };

      // Add to workspace
      state.workspace.add_repository(path, repo);

      log::info!("Successfully loaded repository");
    }

    Action::SelectFile(path) => {
      println!("SelectFile: {:?}", path);
      if let Some(repo) = state.workspace.get_active_repo_mut() {
        // Update selected files
        repo.selected_files.clear();
        repo.selected_files.push(path.clone());

        // Load the diff for this file
        let git_repo = crate::git::open_repository(&repo.path)?;
        let diff_engine = DiffEngine::new(git_repo.repo());

        // Check if file is staged or unstaged
        let file_status = repo.status.files.iter().find(|f| f.path == path);
        let staged = file_status.map(|f| f.staged).unwrap_or(false);
        let file_status_kind = file_status.map(|f| &f.status);

        // Check if file is untracked - if so, create a special diff showing full content
        if let Some(FileStatusKind::Untracked) = file_status_kind {
          // Read the file content
          let file_path = repo.path.join(&path);
          match std::fs::read_to_string(&file_path) {
            Ok(content) => {
              // Create lines from content
              let mut lines = Vec::new();
              for (line_num, line_content) in content.lines().enumerate() {
                lines.push(Line {
                  origin: LineOrigin::Addition,
                  content: line_content.to_string() + "\n",
                  old_lineno: None,
                  new_lineno: Some((line_num + 1) as u32),
                });
              }

              // Create a single hunk with all lines
              use std::collections::hash_map::DefaultHasher;
              use std::hash::{Hash, Hasher};

              let mut hasher = DefaultHasher::new();
              path.hash(&mut hasher);
              let hunk_id = HunkId(hasher.finish());

              let hunk = Hunk {
                id: hunk_id,
                old_start: 0,
                old_lines: 0,
                new_start: 1,
                new_lines: lines.len() as u32,
                header: format!("@@ -0,0 +1,{} @@", lines.len()),
                old_byte_range: 0..0,
                new_byte_range: 0..content.len(),
                lines,
                context_expanded: false,
              };

              // Create FileDiff
              let file_diff = FileDiff {
                path: path.clone(),
                old_path: None,
                status: FileStatusKind::Untracked,
                hunks: vec![hunk],
                old_content: None,
                new_content: Some(content.clone()),
              };

              // Create diff state
              let diff_state = DiffState {
                files: vec![file_diff],
              };
              repo.diff = Some(diff_state);
            }
            Err(e) => {
              eprintln!("ERROR: Failed to read untracked file: {}", e);
              repo.diff = None;
            }
          }
        } else {
          // Get diff with context for tracked files
          match diff_engine.diff_file_with_context(&path, 3, staged) {
            Ok(Some(file_diff)) => {
              // Create or update diff state
              let diff_state = crate::state::DiffState {
                files: vec![file_diff],
              };
              repo.diff = Some(diff_state);
            }
            Ok(None) => {
              repo.diff = None;
            }
            Err(e) => {
              eprintln!("Failed to load diff for {:?}: {}", path, e);
              repo.diff = None;
            }
          }
        }
      }
    }
  }

  Ok(())
}
