use std::{
  fs,
  path::{Path, PathBuf},
  time::{Duration, Instant, SystemTime},
};

use crate::config::{ConfigStore, RecentRepository};
use crate::workspace::{WorkspacePage, WorkspaceRoute};
use editor::{ChangeDirection, DiffViewMode, Editor};
use git::{
  BranchStatus, FileStatusKind, RepositoryFile, branch_status, can_undo_last_commit,
  commit_repository, discard_change, has_head_commit, open_repository, push_repository, stage_all,
  stage_path, undo_last_commit as git_undo_last_commit, unstage_all, unstage_path,
};
use gpui::{
  App, ClickEvent, Context, Corner, Div, Entity, FocusHandle, Focusable, Hsla, PathPromptOptions,
  Pixels, Render, SharedString, Stateful, Task, Window, actions, div, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Theme as ComponentTheme,
  button::{ButtonGroup, ButtonRounded},
  menu::{DropdownMenu as _, PopupMenuItem},
};
use syntax::Theme as SyntaxTheme;
use ui::{
  Button, ButtonVariants, Collapsible, Disableable, IconName, Input, InputState, ResizableState,
  SearchableVec, Select, SelectEvent, SelectItem, SelectState, Sidebar, SidebarItem, Sizable,
  h_resizable, resizable_panel,
};

const SIDEBAR_DEFAULT_WIDTH: Pixels = px(260.0);
const SIDEBAR_MIN_WIDTH: Pixels = px(200.0);
const SIDEBAR_MAX_WIDTH: Pixels = px(600.0);
const APP_HEADER_HEIGHT: f32 = 42.0;
const FILE_POLL_INTERVAL_MS: u64 = 500;
const REPO_POLL_INTERVAL_MS: u64 = 1500;

actions!(workspace, [OpenRepository, SaveFile]);

#[derive(Clone)]
struct FileEntry {
  path: PathBuf,
  display_name: String,
  status: FileStatusKind,
  base_content: Option<String>,
  current_content: Option<String>,
  saved_content: Option<String>,
  last_modified: Option<SystemTime>,
}

#[derive(Clone)]
struct SidebarFileEntry {
  path: PathBuf,
  display_name: String,
  status: FileStatusKind,
}

impl From<&FileEntry> for SidebarFileEntry {
  fn from(entry: &FileEntry) -> Self {
    Self {
      path: entry.path.clone(),
      display_name: entry.display_name.clone(),
      status: entry.status,
    }
  }
}

#[derive(Clone)]
struct RepoSelectItem {
  label: SharedString,
  path: PathBuf,
}

impl SelectItem for RepoSelectItem {
  type Value = PathBuf;

  fn title(&self) -> SharedString {
    self.label.clone()
  }

  fn value(&self) -> &Self::Value {
    &self.path
  }

  fn matches(&self, query: &str) -> bool {
    let query = query.to_lowercase();
    self.label.as_ref().to_lowercase().contains(&query)
      || self.path.to_string_lossy().to_lowercase().contains(&query)
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileListKind {
  Changes,
  Staged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectedFile {
  Changes(usize),
  Staged(usize),
}

#[derive(Clone)]
struct WorkspaceSidebarSection {
  view: Entity<GitPage>,
  kind: FileListKind,
  entries: Vec<SidebarFileEntry>,
  selected_index: Option<usize>,
  has_root: bool,
  collapsed: bool,
}

impl WorkspaceSidebarSection {
  fn new(
    view: Entity<GitPage>,
    kind: FileListKind,
    entries: Vec<SidebarFileEntry>,
    selected_index: Option<usize>,
    has_root: bool,
  ) -> Self {
    Self {
      view,
      kind,
      entries,
      selected_index,
      has_root,
      collapsed: false,
    }
  }
}

impl Collapsible for WorkspaceSidebarSection {
  fn collapsed(mut self, collapsed: bool) -> Self {
    self.collapsed = collapsed;
    self
  }

  fn is_collapsed(&self) -> bool {
    self.collapsed
  }
}

impl SidebarItem for WorkspaceSidebarSection {
  fn render(
    self,
    id: impl Into<gpui::ElementId>,
    window: &mut Window,
    cx: &mut App,
  ) -> impl IntoElement {
    let id = id.into();
    let WorkspaceSidebarSection {
      view,
      kind,
      entries,
      selected_index,
      has_root,
      ..
    } = self;

    render_sidebar_section(
      &view,
      kind,
      &entries,
      selected_index,
      has_root,
      cx.theme(),
      window,
    )
    .id(id)
  }
}

pub struct GitPage {
  root_path: Option<PathBuf>,
  changes: Vec<FileEntry>,
  staged: Vec<FileEntry>,
  selected_file: Option<SelectedFile>,
  editor: Option<Entity<Editor>>,
  error: Option<String>,
  current_dirty: bool,
  poll_task: Option<Task<()>>,
  focus_handle: FocusHandle,
  recent_repositories: Vec<RecentRepository>,
  repo_select: Option<Entity<SelectState<SearchableVec<RepoSelectItem>>>>,
  sidebar_state: Entity<ResizableState>,
  theme: SyntaxTheme,
  diff_view_mode: DiffViewMode,
  commit_input: Entity<InputState>,
  has_head_commit: bool,
  can_undo_last_commit: bool,
  can_push: bool,
  can_force_push: bool,
  branch_status: Option<BranchStatus>,
  last_repo_poll: Option<Instant>,
}

impl GitPage {
  pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
    let recent_repositories = ConfigStore::load_recent_repositories();
    let theme = if cx.theme().is_dark() {
      SyntaxTheme::dark()
    } else {
      SyntaxTheme::light()
    };
    let commit_input = cx.new(|cx| {
      InputState::new(window, cx)
        .auto_grow(1, 5)
        .placeholder("Commit message...")
    });
    let sidebar_state = cx.new(|_| ResizableState::default());
    let mut view = Self {
      root_path: None,
      changes: Vec::new(),
      staged: Vec::new(),
      selected_file: None,
      editor: None,
      error: None,
      current_dirty: false,
      poll_task: None,
      focus_handle: cx.focus_handle(),
      recent_repositories,
      repo_select: None,
      sidebar_state,
      theme,
      diff_view_mode: DiffViewMode::Inline,
      commit_input,
      has_head_commit: false,
      can_undo_last_commit: false,
      can_push: false,
      can_force_push: false,
      branch_status: None,
      last_repo_poll: None,
    };
    if let Some(repo) = view.recent_repositories.first() {
      view.set_root_path(repo.path.clone(), cx);
    }
    view.start_file_polling(cx);
    view
  }

  fn sync_theme_from_app(&mut self, cx: &mut Context<Self>) {
    let is_dark = cx.theme().is_dark();
    if self.theme.is_dark == is_dark {
      return;
    }

    self.theme = if is_dark {
      SyntaxTheme::dark()
    } else {
      SyntaxTheme::light()
    };

    let theme = self.theme.clone();
    if let Some(editor) = self.editor.as_ref() {
      editor.update(cx, |editor, cx| {
        editor.set_theme(theme.clone(), cx);
      });
    }
  }

  fn toggle_diff_view(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
    self.diff_view_mode.toggle();
    let mode = self.diff_view_mode;
    if let Some(editor) = self.editor.as_ref() {
      editor.update(cx, |editor, cx| {
        editor.set_diff_view_mode(mode, cx);
      });
    }
    cx.notify();
  }

  fn commit_changes(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
    self.commit_changes_with_amend(false, window, cx);
  }

  fn commit_amend_changes(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
    self.commit_changes_with_amend(true, window, cx);
  }

  fn undo_last_commit(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
    let Some(root_path) = self.root_path.clone() else {
      return;
    };
    if !can_undo_last_commit(&root_path) {
      self.error =
        Some("Cannot undo last commit: already pushed or no upstream branch.".to_string());
      cx.notify();
      return;
    }
    if let Err(err) = git_undo_last_commit(&root_path) {
      self.error = Some(format!("Failed to undo last commit: {err}"));
      cx.notify();
      return;
    }

    self.error = None;
    self.refresh_repository_statuses(cx);
  }

  fn push_changes(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
    self.push_changes_with_force(false, cx);
  }

  fn force_push_changes(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
    self.push_changes_with_force(true, cx);
  }

  fn push_changes_with_force(&mut self, force: bool, cx: &mut Context<Self>) {
    let Some(root_path) = self.root_path.clone() else {
      return;
    };
    let result = push_repository(&root_path, force);
    if let Err(err) = result {
      let action = if force { "force push" } else { "push" };
      self.error = Some(format!("Failed to {action}: {err}"));
      cx.notify();
      return;
    }

    self.error = None;
    self.refresh_repository_statuses(cx);
  }

  fn commit_changes_with_amend(
    &mut self,
    amend: bool,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let message = self.commit_input.read(cx).value().to_string();
    if self.root_path.is_none() {
      return;
    }
    if !amend && message.trim().is_empty() {
      return;
    }
    if !amend && self.staged.is_empty() {
      return;
    }
    if amend && !self.has_head_commit {
      self.error = Some("Nothing to amend.".to_string());
      cx.notify();
      return;
    }
    let root_path = self.root_path.clone().unwrap();
    if let Err(err) = commit_repository(&root_path, &message, amend) {
      self.error = Some(format!("Failed to commit: {err}"));
      cx.notify();
      return;
    }

    self.error = None;
    self.commit_input.update(cx, |input, cx| {
      input.set_value("", window, cx);
    });
    self.refresh_repository_statuses(cx);
  }

  fn stage_file(&mut self, path: PathBuf, cx: &mut Context<Self>) {
    let Some(root_path) = self.root_path.clone() else {
      return;
    };
    if let Err(err) = stage_path(&root_path, &path) {
      self.error = Some(format!("Failed to stage file: {err}"));
      cx.notify();
      return;
    }
    self.error = None;
    self.refresh_repository_statuses(cx);
  }

  fn unstage_file(&mut self, path: PathBuf, cx: &mut Context<Self>) {
    let Some(root_path) = self.root_path.clone() else {
      return;
    };
    if let Err(err) = unstage_path(&root_path, &path) {
      self.error = Some(format!("Failed to unstage file: {err}"));
      cx.notify();
      return;
    }
    self.error = None;
    self.refresh_repository_statuses(cx);
  }

  fn discard_file_change(&mut self, path: PathBuf, status: FileStatusKind, cx: &mut Context<Self>) {
    let Some(root_path) = self.root_path.clone() else {
      return;
    };
    if let Err(err) = discard_change(&root_path, &path, status) {
      self.error = Some(format!("Failed to discard change: {err}"));
      cx.notify();
      return;
    }
    self.error = None;
    self.refresh_repository_statuses(cx);
  }

  fn stage_all_files(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
    let Some(root_path) = self.root_path.clone() else {
      return;
    };
    if let Err(err) = stage_all(&root_path) {
      self.error = Some(format!("Failed to stage all files: {err}"));
      cx.notify();
      return;
    }
    self.error = None;
    self.refresh_repository_statuses(cx);
  }

  fn unstage_all_files(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
    let Some(root_path) = self.root_path.clone() else {
      return;
    };
    if let Err(err) = unstage_all(&root_path) {
      self.error = Some(format!("Failed to unstage all files: {err}"));
      cx.notify();
      return;
    }
    self.error = None;
    self.refresh_repository_statuses(cx);
  }

  fn jump_to_next_change(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
    let Some(editor) = self.editor.as_ref() else {
      return;
    };
    editor.update(cx, |editor, cx| {
      editor.jump_to_change(ChangeDirection::Next, window, cx);
    });
  }

  fn jump_to_previous_change(
    &mut self,
    _: &ClickEvent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(editor) = self.editor.as_ref() else {
      return;
    };
    editor.update(cx, |editor, cx| {
      editor.jump_to_change(ChangeDirection::Previous, window, cx);
    });
  }

  fn open_repository_clicked(
    &mut self,
    _: &ClickEvent,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.start_open_repository(cx);
  }

  fn open_repository_action(
    &mut self,
    _: &OpenRepository,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.start_open_repository(cx);
  }

  fn start_open_repository(&mut self, cx: &mut Context<Self>) {
    let receiver = cx.prompt_for_paths(PathPromptOptions {
      files: false,
      directories: true,
      multiple: false,
      prompt: Some("Select a repository".into()),
    });

    cx.spawn(async move |this, cx| {
      let Ok(result) = receiver.await else {
        return;
      };

      match result {
        Ok(Some(paths)) => {
          if let Some(path) = paths.into_iter().next() {
            let _ = this.update(cx, |view, cx| {
              view.set_root_path(path, cx);
            });
          }
        }
        Ok(None) => {}
        Err(err) => {
          let message = format!("Failed to open repository: {err}");
          let _ = this.update(cx, |view, cx| {
            view.error = Some(message);
            cx.notify();
          });
        }
      }
    })
    .detach();
  }

  fn repo_select_items(&self) -> SearchableVec<RepoSelectItem> {
    let items = self
      .recent_repositories
      .iter()
      .map(|repo| RepoSelectItem {
        label: SharedString::from(root_label(repo.path.as_path())),
        path: repo.path.clone(),
      })
      .collect::<Vec<_>>();

    SearchableVec::new(items)
  }

  fn on_repo_select_event(
    &mut self,
    _: &Entity<SelectState<SearchableVec<RepoSelectItem>>>,
    event: &SelectEvent<SearchableVec<RepoSelectItem>>,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let SelectEvent::Confirm(Some(path)) = event else {
      return;
    };
    self.set_root_path(path.clone(), cx);
  }

  fn start_file_polling(&mut self, cx: &mut Context<Self>) {
    if self.poll_task.is_some() {
      return;
    }

    let task = cx.spawn(async move |this, cx| {
      loop {
        cx.background_executor()
          .timer(Duration::from_millis(FILE_POLL_INTERVAL_MS))
          .await;
        let _ = this.update(cx, |view, cx| {
          view.poll_current_file(cx);
          view.poll_repository_status(cx);
        });
      }
    });

    self.poll_task = Some(task);
  }

  fn poll_current_file(&mut self, cx: &mut Context<Self>) {
    let Some(selected_file) = self.selected_file else {
      if self.current_dirty {
        self.current_dirty = false;
        cx.notify();
      }
      return;
    };
    let Some(editor) = self.editor.as_ref() else {
      if self.current_dirty {
        self.current_dirty = false;
        cx.notify();
      }
      return;
    };
    let SelectedFile::Changes(selected_index) = selected_file else {
      if self.current_dirty {
        self.current_dirty = false;
        cx.notify();
      }
      return;
    };
    let Some(entry) = self.changes.get_mut(selected_index) else {
      return;
    };

    let current_text = editor_buffer_text(editor, cx);
    let saved_text = entry.saved_content.as_deref().unwrap_or("");
    let is_dirty = current_text != saved_text;
    let mut needs_notify = false;

    if self.current_dirty != is_dirty {
      self.current_dirty = is_dirty;
      needs_notify = true;
    }

    if let Some(modified_time) = read_modified_time(&entry.path) {
      if entry.last_modified != Some(modified_time) && !is_dirty {
        if let Some(disk_text) = read_disk_text(&entry.path) {
          reload_editor_content(editor, &disk_text, cx);
          entry.current_content = Some(disk_text.clone());
          entry.saved_content = Some(disk_text);
          entry.last_modified = Some(modified_time);
          if self.current_dirty {
            self.current_dirty = false;
          }
          needs_notify = true;
        } else {
          entry.last_modified = Some(modified_time);
        }
      }
    }

    if needs_notify {
      cx.notify();
    }
  }

  fn poll_repository_status(&mut self, cx: &mut Context<Self>) {
    if self.root_path.is_none() {
      return;
    }

    let now = Instant::now();
    if let Some(last_poll) = self.last_repo_poll {
      if now.duration_since(last_poll) < Duration::from_millis(REPO_POLL_INTERVAL_MS) {
        return;
      }
    }

    self.last_repo_poll = Some(now);
    self.refresh_repository_statuses(cx);
  }

  fn refresh_repository_statuses(&mut self, cx: &mut Context<Self>) {
    let Some(root_path) = self.root_path.clone() else {
      return;
    };

    let selected_entry = match self.selected_file {
      Some(SelectedFile::Changes(idx)) => self.changes.get(idx).cloned(),
      Some(SelectedFile::Staged(idx)) => self.staged.get(idx).cloned(),
      None => None,
    };
    let selected_path = selected_entry.as_ref().map(|entry| entry.path.clone());
    let selected_list = match self.selected_file {
      Some(SelectedFile::Changes(_)) => Some(FileListKind::Changes),
      Some(SelectedFile::Staged(_)) => Some(FileListKind::Staged),
      None => None,
    };
    let selected_dirty = self.current_dirty;

    let repository = match open_repository(&root_path) {
      Ok(repository) => repository,
      Err(err) => {
        self.error = Some(format!("Not a git repository: {err}"));
        return;
      }
    };

    let repo_root = repository.root;
    self.root_path = Some(repo_root.clone());
    self.has_head_commit = has_head_commit(&repo_root);
    self.can_undo_last_commit = can_undo_last_commit(&repo_root);
    match branch_status(&repo_root) {
      Ok(status) => {
        self.can_push = status.ahead > 0 && status.behind == 0;
        self.can_force_push = status.ahead > 0 && status.behind > 0;
        self.branch_status = Some(status);
      }
      Err(_) => {
        self.can_push = false;
        self.can_force_push = false;
        self.branch_status = None;
      }
    }
    let mut changes = repository_entries_to_files(&repo_root, repository.changes);
    let mut staged = repository_entries_to_files(&repo_root, repository.staged);

    if let Some(path) = selected_path.as_ref() {
      match selected_list {
        Some(FileListKind::Changes) => {
          if let Some(index) = changes.iter().position(|entry| entry.path == *path) {
            if let Some(existing) = selected_entry.as_ref() {
              let entry = &mut changes[index];
              entry.current_content = existing.current_content.clone();
              entry.saved_content = existing.saved_content.clone();
              entry.last_modified = existing.last_modified;
            }
          } else if selected_dirty {
            if let Some(mut entry) = selected_entry {
              entry.status = FileStatusKind::Modified;
              changes.push(entry);
            }
          } else {
            self.selected_file = None;
            self.editor = None;
            self.current_dirty = false;
          }
        }
        Some(FileListKind::Staged) => {
          if let Some(index) = staged.iter().position(|entry| entry.path == *path) {
            if let Some(existing) = selected_entry.as_ref() {
              let entry = &mut staged[index];
              entry.current_content = existing.current_content.clone();
              entry.saved_content = existing.saved_content.clone();
              entry.last_modified = existing.last_modified;
            }
          } else {
            self.selected_file = None;
            self.editor = None;
            self.current_dirty = false;
          }
        }
        None => {}
      }
    }

    changes.sort_by(|a, b| a.path.cmp(&b.path));
    staged.sort_by(|a, b| a.path.cmp(&b.path));
    let next_selected = match selected_list {
      Some(FileListKind::Changes) => selected_path
        .as_ref()
        .and_then(|path| changes.iter().position(|entry| entry.path == *path))
        .map(SelectedFile::Changes),
      Some(FileListKind::Staged) => selected_path
        .as_ref()
        .and_then(|path| staged.iter().position(|entry| entry.path == *path))
        .map(SelectedFile::Staged),
      None => None,
    };

    self.changes = changes;
    self.staged = staged;
    self.selected_file = next_selected;
    self.error = None;
    cx.notify();
  }

  fn save_file_clicked(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
    self.save_current_file(cx);
  }

  fn save_file_action(&mut self, _: &SaveFile, _window: &mut Window, cx: &mut Context<Self>) {
    self.save_current_file(cx);
  }

  fn save_current_file(&mut self, cx: &mut Context<Self>) {
    let Some(selected_file) = self.selected_file else {
      return;
    };
    let Some(editor) = self.editor.as_ref() else {
      return;
    };
    let SelectedFile::Changes(selected_index) = selected_file else {
      return;
    };
    let Some(entry) = self.changes.get_mut(selected_index) else {
      return;
    };

    let text = editor_buffer_text(editor, cx);
    if let Some(parent) = entry.path.parent() {
      if let Err(err) = fs::create_dir_all(parent) {
        eprintln!(
          "Failed to create directories for {}: {}",
          entry.display_name, err
        );
        return;
      }
    }
    if let Err(err) = fs::write(&entry.path, &text) {
      eprintln!("Failed to save {}: {}", entry.display_name, err);
      return;
    }

    entry.current_content = Some(text.clone());
    entry.saved_content = Some(text);
    entry.last_modified = read_modified_time(&entry.path);
    self.current_dirty = false;
    cx.notify();
  }

  fn set_root_path(&mut self, path: PathBuf, cx: &mut Context<Self>) {
    match open_repository(&path) {
      Ok(repository) => {
        let repo_root = repository.root;
        self.root_path = Some(repo_root.clone());
        ConfigStore::persist_recent_repository(&repo_root);
        bump_recent_repository(&mut self.recent_repositories, repo_root.clone());
        self.changes = repository_entries_to_files(&repo_root, repository.changes);
        self.staged = repository_entries_to_files(&repo_root, repository.staged);
        self.selected_file = None;
        self.editor = None;
        self.error = None;
        self.current_dirty = false;
        self.last_repo_poll = Some(Instant::now());
        match branch_status(&repo_root) {
          Ok(status) => {
            self.can_push = status.ahead > 0 && status.behind == 0;
            self.can_force_push = status.ahead > 0 && status.behind > 0;
            self.branch_status = Some(status);
          }
          Err(_) => {
            self.can_push = false;
            self.can_force_push = false;
            self.branch_status = None;
          }
        }
      }
      Err(err) => {
        self.root_path = Some(path);
        self.changes = Vec::new();
        self.staged = Vec::new();
        self.selected_file = None;
        self.editor = None;
        self.error = Some(format!("Not a git repository: {err}"));
        self.current_dirty = false;
        self.last_repo_poll = Some(Instant::now());
        self.can_push = false;
        self.can_force_push = false;
        self.branch_status = None;
      }
    }
    cx.notify();
  }

  fn select_file(
    &mut self,
    list: FileListKind,
    index: usize,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let (content, base_content, file_path, display_name) = match list {
      FileListKind::Changes => match self.changes.get_mut(index) {
        Some(entry) => {
          refresh_entry_from_disk(entry);
          (
            entry.current_content.clone(),
            entry.base_content.clone(),
            entry.path.clone(),
            entry.display_name.clone(),
          )
        }
        None => return,
      },
      FileListKind::Staged => match self.staged.get_mut(index) {
        Some(entry) => (
          entry.current_content.clone(),
          entry.base_content.clone(),
          entry.path.clone(),
          entry.display_name.clone(),
        ),
        None => return,
      },
    };

    let Some(content) = content else {
      self.editor = None;
      self.selected_file = Some(match list {
        FileListKind::Changes => SelectedFile::Changes(index),
        FileListKind::Staged => SelectedFile::Staged(index),
      });
      self.error = Some(format!("File content unavailable: {display_name}"));
      cx.notify();
      return;
    };

    let file_ext = file_path.extension().and_then(|ext| ext.to_str());
    let theme = self.theme.clone();
    let editor = cx.new(|cx| Editor::new(&content, base_content.as_deref(), file_ext, theme, cx));
    let mode = self.diff_view_mode;
    editor.update(cx, |editor, cx| {
      editor.set_diff_view_mode(mode, cx);
    });
    let focus_handle = editor.read(cx).focus_handle(cx);

    match list {
      FileListKind::Changes => {
        if let Some(entry) = self.changes.get_mut(index) {
          if entry.saved_content.is_none() {
            entry.saved_content = Some(content.clone());
          }
          entry.last_modified = read_modified_time(&entry.path);
        }
      }
      FileListKind::Staged => {
        if let Some(entry) = self.staged.get_mut(index) {
          if entry.saved_content.is_none() {
            entry.saved_content = Some(content.clone());
          }
        }
      }
    }

    self.editor = Some(editor);
    self.selected_file = Some(match list {
      FileListKind::Changes => SelectedFile::Changes(index),
      FileListKind::Staged => SelectedFile::Staged(index),
    });
    self.error = None;
    self.current_dirty = false;

    window.focus(&focus_handle, cx);
    cx.notify();
  }

  fn render_empty_state(&mut self, cx: &mut Context<Self>) -> Div {
    let theme = cx.theme().clone();
    let (message, color, show_hint) = if let Some(error) = &self.error {
      (error.clone(), theme.red, false)
    } else {
      (
        "Open a repository to get started.".to_string(),
        theme.foreground,
        true,
      )
    };

    div()
      .key_context("Workspace")
      .track_focus(&self.focus_handle(cx))
      .on_action(cx.listener(Self::open_repository_action))
      .on_action(cx.listener(Self::save_file_action))
      .size_full()
      .bg(theme.background)
      .text_color(color)
      .flex()
      .flex_col()
      .items_center()
      .justify_center()
      .gap_2()
      .child(message)
      .when(show_hint, |this| {
        this.child(
          div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child("Press Cmd+O to open a repository."),
        )
      })
      .child(
        Button::new("open-folder-empty")
          .label("Open Repository")
          .on_click(cx.listener(Self::open_repository_clicked)),
      )
  }

  fn render_app_header(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
    let theme = cx.theme().clone();
    let repo_select = self.render_repo_select(window, cx);

    let header_row = div()
      .h(px(APP_HEADER_HEIGHT))
      .px_3()
      .flex()
      .items_center()
      .justify_between()
      .bg(theme.title_bar)
      .border_b_1()
      .border_color(theme.title_bar_border)
      .child(
        div()
          .flex()
          .items_center()
          .gap_2()
          .child(div().text_sm().text_color(theme.foreground).child("Reviu"))
          .child(repo_select)
          .child(
            Button::new("open-repo")
              .icon(IconName::FolderOpen)
              .ghost()
              .compact()
              .tooltip("Open Repository")
              .on_click(cx.listener(Self::open_repository_clicked)),
          ),
      )
      .child(
        Button::new("open-settings")
          .icon(IconName::Settings)
          .ghost()
          .compact()
          .tooltip("Settings")
          .on_click(|_, _, cx| {
            WorkspaceRoute::global_mut(cx).page = WorkspacePage::Settings;
            cx.refresh_windows();
          }),
      );

    header_row
  }

  fn render_repo_select(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let items = self.repo_select_items();
    let sidebar_width = self
      .sidebar_state
      .read(cx)
      .sizes()
      .first()
      .copied()
      .unwrap_or(SIDEBAR_DEFAULT_WIDTH);
    let repo_select = match self.repo_select.clone() {
      Some(state) => state,
      None => {
        let state = cx.new(|cx| SelectState::new(items.clone(), None, window, cx).searchable(true));
        cx.subscribe_in(&state, window, Self::on_repo_select_event)
          .detach();
        self.repo_select = Some(state.clone());
        state
      }
    };

    repo_select.update(cx, |state, cx| {
      state.set_items(items, window, cx);
      match self.root_path.clone() {
        Some(root_path) => state.set_selected_value(&root_path, window, cx),
        None => state.set_selected_index(None, window, cx),
      }
    });

    Select::new(&repo_select)
      .placeholder("Select Repository")
      .search_placeholder("Search repositories")
      .icon(IconName::Folder)
      .menu_width(sidebar_width)
      .empty(
        div()
          .px_2()
          .py_2()
          .text_sm()
          .child("No recent repositories."),
      )
      .w(px(220.0))
  }

  fn render_sidebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let view = cx.entity();
    let staged_entries = self
      .staged
      .iter()
      .map(SidebarFileEntry::from)
      .collect::<Vec<_>>();
    let changes_entries = self
      .changes
      .iter()
      .map(SidebarFileEntry::from)
      .collect::<Vec<_>>();
    let selected_staged = match self.selected_file {
      Some(SelectedFile::Staged(idx)) => Some(idx),
      _ => None,
    };
    let selected_changes = match self.selected_file {
      Some(SelectedFile::Changes(idx)) => Some(idx),
      _ => None,
    };
    let staged_section: WorkspaceSidebarSection = WorkspaceSidebarSection::new(
      view.clone(),
      FileListKind::Staged,
      staged_entries,
      selected_staged,
      self.root_path.is_some(),
    );
    let changes_section = WorkspaceSidebarSection::new(
      view,
      FileListKind::Changes,
      changes_entries,
      selected_changes,
      self.root_path.is_some(),
    );
    let commit_bar = self.render_commit_bar(cx);

    let sidebar = Sidebar::new("workspace-sidebar")
      .w_full()
      .flex_1()
      .bg(theme.sidebar)
      .border_0()
      .text_color(theme.sidebar_foreground)
      .child(staged_section)
      .child(changes_section);

    div()
      .w_full()
      .flex()
      .flex_col()
      .bg(theme.sidebar)
      .child(sidebar)
      .child(commit_bar)
  }

  fn render_commit_bar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let input = self.commit_input.clone();

    div()
      .flex()
      .p_2()
      .relative()
      .bg(theme.sidebar)
      .border_t_1()
      .border_color(theme.sidebar_border)
      .w_full()
      .child(
        div()
          .w_full()
          .flex()
          .flex_col()
          .gap_2()
          .child(Input::new(&input))
          .child(self.render_commit_button(cx)),
      )
  }

  fn render_commit_button(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let commit_enabled = self.root_path.is_some() && !self.staged.is_empty();
    let amend_enabled = self.root_path.is_some() && self.has_head_commit;
    let undo_enabled = self.root_path.is_some() && self.can_undo_last_commit;
    let push_enabled = self.root_path.is_some() && self.can_push;
    let force_push_enabled = self.root_path.is_some() && self.can_force_push;
    let menu_enabled = amend_enabled || undo_enabled || push_enabled || force_push_enabled;
    let view = cx.entity();
    let amend_view = view.clone();
    let undo_view = view.clone();
    let push_view = view.clone();
    let force_push_view = view.clone();

    let main_button = Button::new("commit-button-main")
      .label("Commit")
      .primary()
      .flex_1()
      .rounded_r_none()
      .disabled(!commit_enabled)
      .on_click(cx.listener(Self::commit_changes));

    let menu_button = Button::new("commit-button-menu")
      .icon(IconName::ChevronDown)
      .primary()
      .rounded_l_none()
      .border_l_0()
      .disabled(!menu_enabled)
      .dropdown_menu_with_anchor(Corner::BottomRight, move |menu, _, _| {
        let amend_view = amend_view.clone();
        let undo_view = undo_view.clone();
        let push_view = push_view.clone();
        let force_push_view = force_push_view.clone();
        let menu = menu.item(
          PopupMenuItem::new("Amend")
            .icon(IconName::Replace)
            .disabled(!amend_enabled)
            .on_click(move |event, window, cx| {
              amend_view.update(cx, |this, cx| {
                this.commit_amend_changes(event, window, cx);
              });
            }),
        );

        let menu = menu.item(
          PopupMenuItem::new("Undo last commit")
            .icon(IconName::Undo)
            .disabled(!undo_enabled)
            .on_click(move |event, window, cx| {
              undo_view.update(cx, |this, cx| {
                this.undo_last_commit(event, window, cx);
              });
            }),
        );

        let menu = menu.separator();

        let menu = menu.item(
          PopupMenuItem::new("Push")
            .icon(IconName::ArrowUp)
            .disabled(!push_enabled)
            .on_click(move |event, window, cx| {
              push_view.update(cx, |this, cx| {
                this.push_changes(event, window, cx);
              });
            }),
        );

        menu.item(
          PopupMenuItem::new("Force push (with lease)")
            .icon(IconName::TriangleAlert)
            .disabled(!force_push_enabled)
            .on_click(move |event, window, cx| {
              force_push_view.update(cx, |this, cx| {
                this.force_push_changes(event, window, cx);
              });
            }),
        )
      });

    div()
      .flex()
      .w_full()
      .overflow_hidden()
      .child(main_button)
      .child(menu_button)
  }

  fn render_editor_header(&mut self, cx: &mut Context<Self>) -> Div {
    let theme = cx.theme().clone();
    let title = self
      .selected_file
      .and_then(|selected| match selected {
        SelectedFile::Changes(idx) => self.changes.get(idx),
        SelectedFile::Staged(idx) => self.staged.get(idx),
      })
      .map(|entry| entry.display_name.clone())
      .unwrap_or_else(|| "File".to_string());

    let mut title_row = div()
      .flex()
      .items_center()
      .gap_1()
      .child(div().text_sm().text_color(theme.foreground).child(title));

    if self.current_dirty {
      title_row = title_row.child(div().text_sm().text_color(theme.warning).child("*"));
    }

    let diff_label = match self.diff_view_mode {
      DiffViewMode::Inline => "Split Diff",
      DiffViewMode::Split => "Inline Diff",
    };
    let change_position = self
      .editor
      .as_ref()
      .and_then(|editor| editor.read(cx).change_position(cx));

    let actions = div()
      .flex()
      .items_center()
      .gap_2()
      .when(self.current_dirty, |this| {
        this.child(
          Button::new("save-file")
            .label("Save")
            .small()
            .primary()
            .on_click(cx.listener(Self::save_file_clicked)),
        )
      })
      .child(
        Button::new("prev-change")
          .label("Prev Change")
          .small()
          .on_click(cx.listener(Self::jump_to_previous_change)),
      )
      .child(
        Button::new("next-change")
          .label("Next Change")
          .small()
          .on_click(cx.listener(Self::jump_to_next_change)),
      )
      .when_some(change_position, |this, (current, total)| {
        this.child(
          div()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(format!("{}/{}", current, total)),
        )
      })
      .child(
        Button::new("diff-toggle")
          .label(diff_label)
          .small()
          .on_click(cx.listener(Self::toggle_diff_view)),
      );

    div()
      .h_10()
      .px_3()
      .flex()
      .items_center()
      .justify_between()
      .border_b_1()
      .border_color(theme.border)
      .bg(theme.tab_bar)
      .child(title_row)
      .child(actions)
  }

  fn render_footer(&self, cx: &mut Context<Self>) -> Div {
    if self.root_path.is_none() {
      return div();
    }

    let theme = cx.theme().clone();
    let (branch_name, ahead, behind) = match self.branch_status.as_ref() {
      Some(status) => (status.name.as_str(), status.ahead, status.behind),
      None => ("No branch", 0, 0),
    };

    let branch = div()
      .text_sm()
      .text_color(theme.foreground)
      .child(branch_name.to_string());

    let pull = div()
      .flex()
      .items_center()
      .child(IconName::ArrowDown)
      .child(format!("{behind}"));

    let push = div()
      .flex()
      .items_center()
      .child(IconName::ArrowUp)
      .child(format!("{ahead}"));

    div()
      .px_3()
      .py_1()
      .flex()
      .items_center()
      .gap_2()
      .border_t_1()
      .border_color(theme.border)
      .bg(theme.tab_bar)
      .text_xs()
      .text_color(theme.muted_foreground)
      .child(branch)
      .child(div().flex().items_center().gap_1().child(pull).child(push))
  }
}

impl Render for GitPage {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    self.sync_theme_from_app(cx);
    let theme = cx.theme().clone();
    let show_empty_only = self.root_path.is_none() && self.recent_repositories.is_empty();

    let content = if show_empty_only {
      div().flex_1().child(self.render_empty_state(cx))
    } else {
      let main = if self.root_path.is_none() {
        div()
          .flex_1()
          .flex()
          .flex_col()
          .min_w(px(0.0))
          .size_full()
          .bg(theme.background)
          .child(
            div()
              .flex()
              .items_center()
              .justify_center()
              .size_full()
              .text_color(theme.muted_foreground)
              .child("Select a repository to get started."),
          )
      } else {
        let mut main = div()
          .flex_1()
          .flex()
          .flex_col()
          .min_w(px(0.0))
          .min_h(px(0.0))
          .size_full()
          .bg(theme.background);

        if let Some(editor) = self.editor.clone() {
          let header = self.render_editor_header(cx);
          main = main
            .child(header)
            .child(div().flex_1().min_w(px(0.0)).child(editor));
        } else {
          let (message, color) = if let Some(error) = &self.error {
            (error.clone(), theme.red)
          } else if self.changes.is_empty() && self.staged.is_empty() {
            (
              "No changes found in this repository.".to_string(),
              theme.muted_foreground,
            )
          } else {
            (
              "Select a file to view it.".to_string(),
              theme.muted_foreground,
            )
          };

          main = main.child(
            div()
              .flex()
              .items_center()
              .justify_center()
              .size_full()
              .text_color(color)
              .child(message),
          );
        }

        main
      };

      div().flex_1().min_h(px(0.0)).bg(theme.background).child(
        h_resizable("workspace-layout")
          .with_state(&self.sidebar_state)
          .child(
            resizable_panel()
              .size(SIDEBAR_DEFAULT_WIDTH)
              .size_range(SIDEBAR_MIN_WIDTH..SIDEBAR_MAX_WIDTH)
              .child(self.render_sidebar(cx)),
          )
          .child(resizable_panel().child(main)),
      )
    };

    div()
      .size_full()
      .flex()
      .flex_col()
      .relative()
      .bg(theme.background)
      .child(self.render_app_header(window, cx))
      .child(content)
      .child(self.render_footer(cx))
      .key_context("Workspace")
      .track_focus(&self.focus_handle(cx))
      .on_action(cx.listener(Self::open_repository_action))
      .on_action(cx.listener(Self::save_file_action))
  }
}

impl Focusable for GitPage {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

fn render_sidebar_section(
  view: &Entity<GitPage>,
  list: FileListKind,
  entries: &[SidebarFileEntry],
  selected_index: Option<usize>,
  has_root: bool,
  theme: &ComponentTheme,
  window: &mut Window,
) -> Div {
  let (title, entries_len, staged) = match list {
    FileListKind::Changes => ("Changes", entries.len(), false),
    FileListKind::Staged => ("Staged", entries.len(), true),
  };
  let header = div()
    .px_2()
    .h_8()
    .flex()
    .items_center()
    .justify_between()
    .when_else(staged, |this| this.mb_2(), |this| this.my_2())
    .bg(theme.sidebar_border)
    .rounded_md()
    .child(
      div()
        .text_sm()
        .text_color(theme.sidebar_foreground)
        .child(format!("{title} ({entries_len})")),
    )
    .when(entries_len > 0, |this| match list {
      FileListKind::Changes => this.child(
        Button::new("stage-all")
          .label("Stage All")
          .small()
          .compact()
          .on_click(window.listener_for(view, GitPage::stage_all_files)),
      ),
      FileListKind::Staged => this.child(
        Button::new("unstage-all")
          .label("Unstage All")
          .small()
          .compact()
          .on_click(window.listener_for(view, GitPage::unstage_all_files)),
      ),
    });

  let mut items = div().flex().flex_col();
  for (idx, entry) in entries.iter().enumerate() {
    let is_selected = selected_index == Some(idx);
    items = items.child(render_sidebar_row(
      view,
      list,
      idx,
      entry,
      is_selected,
      theme,
      window,
    ));
  }

  let empty_message = match list {
    FileListKind::Changes => {
      if has_root {
        "No changes."
      } else {
        "No repository selected."
      }
    }
    FileListKind::Staged => "No staged changes.",
  };

  let body = if entries_len == 0 {
    div()
      .flex_1()
      .flex()
      .items_center()
      .justify_center()
      .pb_2()
      .text_sm()
      .text_color(theme.muted_foreground)
      .child(empty_message)
  } else {
    div().flex().flex_col().child(items)
  };

  div()
    .flex()
    .flex_col()
    .min_h(px(0.0))
    .child(header)
    .child(body)
}

fn render_sidebar_row(
  view: &Entity<GitPage>,
  list: FileListKind,
  idx: usize,
  entry: &SidebarFileEntry,
  is_selected: bool,
  theme: &ComponentTheme,
  window: &mut Window,
) -> Stateful<Div> {
  let (tag, tag_color) = status_tag(entry.status, theme);
  let row_group = format!(
    "file-row-{}-{}",
    match list {
      FileListKind::Changes => "changes",
      FileListKind::Staged => "staged",
    },
    idx
  );

  let actions_bg = theme.sidebar_accent;

  let mut actions_wrap = ButtonGroup::new(format!("file-actions-{}", row_group))
    .ghost()
    .compact()
    .xsmall()
    .bg(actions_bg)
    .rounded_md();

  let mut actions = div()
    .absolute()
    .right_1()
    .top_0()
    .bottom_0()
    .flex()
    .items_center()
    .opacity(0.0)
    .group_hover(row_group.clone(), |style| style.opacity(1.0));

  match list {
    FileListKind::Changes => {
      let path = entry.path.clone();
      let discard_path = entry.path.clone();
      let status = entry.status;
      actions_wrap = actions_wrap
        .child(
          Button::new(format!("stage-file-{}", &row_group))
            .icon(IconName::Plus)
            .tooltip("Stage file")
            .on_click(
              window.listener_for(view, move |this, _: &ClickEvent, _window, cx| {
                cx.stop_propagation();
                this.stage_file(path.clone(), cx);
              }),
            ),
        )
        .child(
          Button::new(format!("discard-file-{}", &row_group))
            .icon(IconName::Delete)
            .tooltip("Discard change")
            .on_click(
              window.listener_for(view, move |this, _: &ClickEvent, _window, cx| {
                cx.stop_propagation();
                this.discard_file_change(discard_path.clone(), status, cx);
              }),
            ),
        );
    }
    FileListKind::Staged => {
      let path = entry.path.clone();
      actions_wrap = actions_wrap.child(
        Button::new(format!("unstage-file-{}", &row_group))
          .icon(IconName::Minus)
          .tooltip("Unstage file")
          .on_click(
            window.listener_for(view, move |this, _: &ClickEvent, _window, cx| {
              cx.stop_propagation();
              this.unstage_file(path.clone(), cx);
            }),
          ),
      );
    }
  }
  actions = actions.child(actions_wrap);

  div()
    .id(idx)
    .px_2()
    .py_1()
    .w_full()
    .text_sm()
    .text_color(theme.sidebar_foreground)
    .cursor_pointer()
    .on_click(
      window.listener_for(view, move |this, _: &ClickEvent, window, cx| {
        this.select_file(list, idx, window, cx);
      }),
    )
    .relative()
    .group(row_group.clone())
    .rounded_md()
    .flex()
    .items_center()
    .gap_2()
    .when_else(
      is_selected,
      |this| {
        this
          .bg(theme.list_active)
          .text_color(theme.sidebar_foreground)
      },
      |this| this.hover(|style| style.bg(theme.list_hover)),
    )
    .child(div().flex_none().text_sm().text_color(tag_color).child(tag))
    .child(
      div()
        .flex_1()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis_start()
        .child(entry.display_name.clone()),
    )
    .child(actions)
}

fn editor_buffer_text(editor: &Entity<Editor>, cx: &App) -> String {
  let document = editor.read(cx).document().read(cx);
  let len = document.buffer.len();
  document.buffer.slice_to_string(0..len)
}

fn read_modified_time(path: &Path) -> Option<SystemTime> {
  fs::metadata(path)
    .ok()
    .and_then(|metadata| metadata.modified().ok())
}

fn read_disk_text(path: &Path) -> Option<String> {
  let bytes = fs::read(path).ok()?;
  Some(String::from_utf8_lossy(&bytes).to_string())
}

fn reload_editor_content(editor: &Entity<Editor>, text: &str, cx: &mut App) {
  editor.update(cx, |editor, cx| {
    editor.reload_from_disk(text, cx);
  });
}

fn refresh_entry_from_disk(entry: &mut FileEntry) {
  if let Some(modified_time) = read_modified_time(&entry.path) {
    if entry.last_modified != Some(modified_time) {
      if let Some(disk_text) = read_disk_text(&entry.path) {
        entry.current_content = Some(disk_text.clone());
        entry.saved_content = Some(disk_text);
        entry.last_modified = Some(modified_time);
      } else {
        entry.last_modified = Some(modified_time);
      }
    }
  }
}

fn root_label(path: &Path) -> String {
  path
    .file_name()
    .and_then(|name| name.to_str())
    .map(|name| name.to_string())
    .unwrap_or_else(|| path.display().to_string())
}

fn repository_entries_to_files(repo_root: &Path, entries: Vec<RepositoryFile>) -> Vec<FileEntry> {
  entries
    .into_iter()
    .map(|entry| {
      let path = repo_root.join(&entry.path);
      let current_content = entry.current_content;
      let saved_content = current_content.clone();
      let last_modified = read_modified_time(&path);
      FileEntry {
        path,
        display_name: entry.path.to_string_lossy().to_string(),
        status: entry.status,
        base_content: entry.base_content,
        current_content,
        saved_content,
        last_modified,
      }
    })
    .collect()
}

fn bump_recent_repository(repositories: &mut Vec<RecentRepository>, path: PathBuf) {
  if let Some(index) = repositories.iter().position(|repo| repo.path == path) {
    let repo = repositories.remove(index);
    repositories.insert(0, repo);
  } else {
    repositories.insert(0, RecentRepository { path });
  }
}

fn status_tag(status: FileStatusKind, theme: &ComponentTheme) -> (&'static str, Hsla) {
  match status {
    FileStatusKind::Added => ("A", theme.green),
    FileStatusKind::Untracked => ("U", theme.green),
    FileStatusKind::Modified => ("M", theme.yellow),
    FileStatusKind::Deleted => ("D", theme.red),
    FileStatusKind::Renamed => ("R", theme.yellow),
    FileStatusKind::Typechange => ("T", theme.yellow),
    FileStatusKind::Conflicted => ("C", theme.red),
  }
}
