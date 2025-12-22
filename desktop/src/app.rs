// Compatibility wrapper - the real workspace logic is in workspace.rs
// This file exists for backward compatibility during the migration

pub use crate::workspace::Workspace;

use crate::error::Result;
use crate::state::{Action, AppState};
use gpui::{App, AppContext, Context, Window};
use std::path::PathBuf;

/// AppView - compatibility wrapper around Workspace
pub struct AppView {
  workspace: gpui::Entity<Workspace>,
}

impl AppView {
  /// Register global keybindings
  pub fn register_keybindings(cx: &mut App) {
    Workspace::register(cx);
  }

  /// Create a new application instance
  pub fn new(cx: &mut Context<Self>) -> Self {
    let workspace = cx.new(|cx| Workspace::new(cx));
    Self { workspace }
  }

  /// Dispatch an action to update the state
  pub fn dispatch(&mut self, action: Action, cx: &mut Context<Self>) -> Result<()> {
    self
      .workspace
      .update(cx, |workspace, cx| workspace.dispatch(action, cx))
  }

  /// Get a reference to the app state
  pub fn state(&self, cx: &App) -> AppState {
    self.workspace.read(cx).state().clone()
  }

  /// Open a repository
  pub fn open_repository(&mut self, path: PathBuf, cx: &mut Context<Self>) -> Result<()> {
    self
      .workspace
      .update(cx, |workspace, cx| workspace.open_repository(path, cx))
  }
}

impl gpui::Render for AppView {
  fn render(
    &mut self,
    _window: &mut Window,
    _cx: &mut Context<Self>,
  ) -> impl gpui::prelude::IntoElement {
    self.workspace.clone()
  }
}
