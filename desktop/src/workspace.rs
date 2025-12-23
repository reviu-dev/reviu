use crate::error::Result;
use crate::state::{Action, AppState};
use crate::storage::Storage;
use crate::ui::MainView;
use gpui::{
  actions, div, prelude::*, App, Context, Entity, EventEmitter, FocusHandle, Focusable, KeyBinding,
  Render, WeakEntity, Window,
};
use std::path::PathBuf;
use std::sync::Arc;

actions!(reviu, [OpenRepository, Quit]);

pub struct WorkspaceCreated(pub WeakEntity<Workspace>);

pub enum Event {
  WorkspaceCreated(WeakEntity<Workspace>),
}

impl EventEmitter<Event> for Workspace {}

/// Main workspace struct - manages state and actions
pub struct Workspace {
  weak_self: WeakEntity<Self>,
  state: AppState,
  storage: Arc<Storage>,
  focus_handle: FocusHandle,
  main_view: Option<Entity<MainView>>,
}

impl Workspace {
  /// Create a new workspace
  pub fn new(cx: &mut Context<Self>) -> Self {
    let focus_handle = cx.focus_handle();

    // Initialize storage
    let data_dir = Self::get_data_dir();
    let storage = Storage::new(&data_dir).expect("Failed to initialize storage");

    // Load config from storage or use defaults
    let config = storage.load_config().ok().flatten().unwrap_or_default();

    // Load recent repos
    let recent_repos = storage
      .get_recent_repos(10)
      .ok()
      .unwrap_or_default()
      .into_iter()
      .map(|(path, _, _)| path)
      .collect();

    // Create initial state
    let mut state = AppState::new();
    state.config = config;
    state.workspace.recent_repos = recent_repos;

    // Try to load cached auth
    if let Ok(Some((token, _expires_at, _user_id))) = storage.get_auth_token() {
      state.auth.token = Some(token);
    }

    if let Ok(Some(user)) = storage.get_user() {
      state.auth.user = Some(user.clone());
      state.auth.premium = user.premium;
    }

    let weak_self = cx.entity().downgrade();

    let mut this = Self {
      weak_self: weak_self.clone(),
      state,
      storage: storage.into(),
      focus_handle,
      main_view: None,
    };

    // Restore recent repositories from storage
    this.restore_recent_repos(cx);

    cx.emit(Event::WorkspaceCreated(cx.entity().downgrade()));

    this
  }

  /// Register all workspace actions and keybindings
  pub fn register(cx: &mut App) {
    cx.bind_keys([
      KeyBinding::new("cmd-o", OpenRepository, None),
      KeyBinding::new("cmd-q", Quit, None),
    ]);
  }

  /// Get the application data directory
  fn get_data_dir() -> PathBuf {
    if let Some(data_dir) = dirs::data_dir() {
      data_dir.join("reviu")
    } else {
      PathBuf::from(".reviu")
    }
  }

  /// Dispatch an action to update the state
  pub fn dispatch(&mut self, action: Action, cx: &mut Context<Self>) -> Result<()> {
    // Check if we're switching repositories to update last_opened_at
    if let Action::SwitchRepository(ref path) = action {
      if let Some(repo) = self.state.workspace.repos.get(path) {
        // Update last_opened_at in storage
        if let Err(e) = self.storage.add_recent_repo(path, &repo.name) {
          log::error!("Failed to update last_opened_at for repo: {}", e);
        }
      }
    }

    crate::state::update(&mut self.state, action)?;
    self.persist_state()?;
    cx.notify();
    Ok(())
  }

  /// Persist state changes to storage
  fn persist_state(&self) -> Result<()> {
    // Save config
    if let Err(e) = self.storage.save_config(&self.state.config) {
      log::error!("Failed to save config: {}", e);
    }

    // Save auth token if present
    if let Some(token) = &self.state.auth.token {
      // Default expiry in 30 days
      let expires_at = chrono::Utc::now().timestamp() + (30 * 24 * 60 * 60);
      let user_id = self
        .state
        .auth
        .user
        .as_ref()
        .map(|u| u.id.as_str())
        .unwrap_or("unknown");

      if let Err(e) = self.storage.save_auth_token(token, expires_at, user_id) {
        log::error!("Failed to save auth token: {}", e);
      }
    }

    // Save user if present
    if let Some(user) = &self.state.auth.user {
      if let Err(e) = self.storage.save_user(user) {
        log::error!("Failed to save user: {}", e);
      }
    }

    Ok(())
  }

  /// Get a reference to the app state
  pub fn state(&self) -> &AppState {
    &self.state
  }

  /// Restore recent repositories from storage
  fn restore_recent_repos(&mut self, cx: &mut Context<Self>) {
    let recent_repos = match self.storage.get_recent_repos(10) {
      Ok(repos) => repos,
      Err(e) => {
        log::error!("Failed to load recent repos: {}", e);
        return;
      }
    };

    let mut first_valid_repo: Option<PathBuf> = None;

    for (path, _name, _timestamp) in recent_repos {
      // Only restore if the path still exists and is a valid git repo
      if path.exists() && crate::git::is_git_repository(&path) {
        if let Err(e) = self.dispatch(Action::LoadRepository(path.clone()), cx) {
          log::error!("Failed to restore repository {:?}: {}", path, e);
        } else {
          log::info!("Restored repository: {:?}", path);
          // Keep track of the first valid repo (most recently opened)
          if first_valid_repo.is_none() {
            first_valid_repo = Some(path);
          }
        }
      } else {
        log::warn!("Skipping invalid or missing repository: {:?}", path);
        // Optionally remove from storage
        let _ = self.storage.remove_recent_repo(&path);
      }
    }

    // Automatically switch to the most recently opened repository
    if let Some(path) = first_valid_repo {
      if let Err(e) = self.dispatch(Action::SwitchRepository(path.clone()), cx) {
        log::error!(
          "Failed to switch to last opened repository {:?}: {}",
          path,
          e
        );
      } else {
        log::info!("Switched to last opened repository: {:?}", path);
      }
    }
  }

  /// Open a repository
  pub fn open_repository(&mut self, path: PathBuf, cx: &mut Context<Self>) -> Result<()> {
    // Get the repository name before dispatching
    let repo_name = path
      .file_name()
      .and_then(|n| n.to_str())
      .unwrap_or("Unknown")
      .to_string();

    // Load the repository into state
    self.dispatch(Action::LoadRepository(path.clone()), cx)?;

    // Save to recent repos in storage
    if let Err(e) = self.storage.add_recent_repo(&path, &repo_name) {
      log::error!("Failed to save recent repo: {}", e);
    }

    Ok(())
  }

  /// Handle the OpenRepository action
  fn handle_open_repository(
    &mut self,
    _: &OpenRepository,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    // Prompt user to select a directory
    let paths = cx.prompt_for_paths(gpui::PathPromptOptions {
      files: false,
      directories: true,
      multiple: false,
      prompt: Some("Select Git Repository".into()),
    });

    let workspace = cx.entity().downgrade();
    window
      .spawn(cx, async move |cx| {
        let paths = paths.await.ok()?.ok()??;
        let path = paths.first()?.clone();

        // Validate that it's a git repository
        if !crate::git::is_git_repository(&path) {
          eprintln!("Not a git repository: {:?}", path);
          return None;
        }

        workspace
          .update(cx, |workspace, cx| {
            if let Err(e) = workspace.open_repository(path.clone(), cx) {
              eprintln!("Failed to open repository: {}", e);
            }
          })
          .ok()?;

        Some(())
      })
      .detach();
  }

  /// Handle the Quit action
  fn handle_quit(&mut self, _: &Quit, _window: &mut Window, cx: &mut Context<Self>) {
    cx.quit();
  }
}

impl Focusable for Workspace {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

impl Render for Workspace {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    // Create or get the main view entity
    if self.main_view.is_none() {
      let weak_self = self.weak_self.clone();
      let storage = self.storage.clone();
      self.main_view = Some(cx.new(|cx| MainView::new(weak_self, storage, cx)));
    }

    div()
      .size_full()
      .track_focus(&self.focus_handle)
      .on_action(cx.listener(Self::handle_open_repository))
      .on_action(cx.listener(Self::handle_quit))
      .child(self.main_view.clone().unwrap())
  }
}
