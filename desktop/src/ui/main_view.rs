use crate::state::{Action, AppState};
use crate::storage::Storage;
use crate::ui::{Colors, EditorDiffView};
use gpui::{
  div, prelude::*, px, AnyElement, Context, Entity, IntoElement, ParentElement, Styled, WeakEntity,
  Window,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Main application view - now a proper entity with click handlers
pub struct MainView {
  workspace: WeakEntity<crate::workspace::Workspace>,
  #[allow(dead_code)]
  storage: Arc<Storage>,
  /// Cache of EditorDiffView for each file
  editor_diffs: HashMap<PathBuf, Entity<EditorDiffView>>,
}

impl MainView {
  pub fn new(
    workspace: WeakEntity<crate::workspace::Workspace>,
    storage: Arc<Storage>,
    cx: &mut Context<Self>,
  ) -> Self {
    // Observe workspace changes to trigger re-render
    if let Some(ws) = workspace.upgrade() {
      cx.observe(&ws, |_this, _workspace, cx| {
        cx.notify();
      })
      .detach();
    }

    Self {
      workspace,
      storage,
      editor_diffs: HashMap::new(),
    }
  }

  /// Render empty state when no repository is open
  pub fn render_empty_state() -> AnyElement {
    div()
      .flex()
      .flex_col()
      .size_full()
      .bg(Colors::bg_primary())
      .child(Self::render_header_empty())
      .child(
        div().flex().flex_1().child(
          div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .size_full()
            .gap_4()
            .child(
              div()
                .text_2xl()
                .text_color(Colors::text_secondary())
                .child("No repository open"),
            )
            .child(
              div()
                .text_sm()
                .text_color(Colors::text_muted())
                .child("Press Cmd+O to open a repository"),
            ),
        ),
      )
      .child(Self::render_status_bar_empty())
      .into_any_element()
  }

  /// Render the header/toolbar
  fn render_header(state: &AppState) -> impl IntoElement {
    let repo_name = state.workspace.get_active_repo().map(|repo| {
      log::info!("Rendering header with repo: {}", repo.name);
      format!(" • {}", repo.name)
    });

    log::info!(
      "render_header - has_repo: {}",
      state.workspace.active_repo.is_some()
    );

    div()
      .flex()
      .items_center()
      .justify_between()
      .h(px(48.0))
      .px(px(16.0))
      .bg(Colors::bg_secondary())
      .border_b_1()
      .border_color(Colors::border_primary())
      .child(
        div()
          .flex()
          .items_center()
          .gap_2()
          .child(
            div()
              .text_xl()
              .font_weight(gpui::FontWeight::BOLD)
              .text_color(Colors::text_primary())
              .child("Reviu"),
          )
          .children(repo_name.map(|name| {
            div()
              .text_sm()
              .text_color(Colors::text_secondary())
              .child(name)
          })),
      )
      .child(
        div().flex().gap_2().child(
          div()
            .text_sm()
            .text_color(Colors::text_secondary())
            .child("Cmd+O to open repository"),
        ),
      )
  }

  /// Render the header when no repository is open
  fn render_header_empty() -> impl IntoElement {
    div()
      .flex()
      .items_center()
      .justify_between()
      .h(px(48.0))
      .px(px(16.0))
      .bg(Colors::bg_secondary())
      .border_b_1()
      .border_color(Colors::border_primary())
      .child(
        div().flex().items_center().gap_2().child(
          div()
            .text_xl()
            .font_weight(gpui::FontWeight::BOLD)
            .text_color(Colors::text_primary())
            .child("Reviu"),
        ),
      )
      .child(
        div().flex().gap_2().child(
          div()
            .text_sm()
            .text_color(Colors::text_secondary())
            .child("Press Cmd+O to open repository"),
        ),
      )
  }

  /// Render the status bar
  fn render_status_bar(state: &AppState) -> impl IntoElement {
    let branch = state
      .workspace
      .get_active_repo()
      .and_then(|repo| repo.head.clone())
      .unwrap_or_else(|| "No branch".to_string());

    let file_count = state
      .workspace
      .get_active_repo()
      .map(|repo| repo.status.files.len())
      .unwrap_or(0);

    div()
      .flex()
      .items_center()
      .justify_between()
      .h(px(24.0))
      .px(px(16.0))
      .bg(Colors::bg_secondary())
      .border_t_1()
      .border_color(Colors::border_primary())
      .child(
        div()
          .flex()
          .gap_4()
          .child(
            div()
              .text_xs()
              .text_color(Colors::text_secondary())
              .child(format!("Branch: {}", branch)),
          )
          .child(
            div()
              .text_xs()
              .text_color(Colors::text_secondary())
              .child(format!("{} files changed", file_count)),
          ),
      )
      .child(
        div()
          .text_xs()
          .text_color(Colors::text_muted())
          .child("v0.1.0"),
      )
  }

  /// Render the status bar when no repository is open
  fn render_status_bar_empty() -> impl IntoElement {
    div()
      .flex()
      .items_center()
      .justify_between()
      .h(px(24.0))
      .px(px(16.0))
      .bg(Colors::bg_secondary())
      .border_t_1()
      .border_color(Colors::border_primary())
      .child(
        div().flex().gap_4().child(
          div()
            .text_xs()
            .text_color(Colors::text_secondary())
            .child("No repository open"),
        ),
      )
      .child(
        div()
          .text_xs()
          .text_color(Colors::text_muted())
          .child("v0.1.0"),
      )
  }

  /// Render a single file item
  fn render_file_item(
    path: &std::path::Path,
    status: crate::state::FileStatusKind,
    staged: bool,
  ) -> impl IntoElement {
    let status_color = match status {
      crate::state::FileStatusKind::Modified => Colors::status_modified(),
      crate::state::FileStatusKind::Added => Colors::status_added(),
      crate::state::FileStatusKind::Deleted => Colors::status_deleted(),
      crate::state::FileStatusKind::Untracked => Colors::status_untracked(),
      crate::state::FileStatusKind::Renamed { .. } => Colors::status_modified(),
      crate::state::FileStatusKind::Copied { .. } => Colors::status_added(),
    };

    let status_str = status.short_str().to_string();
    let status_badge = if staged { " [S]" } else { "" };
    let file_name = path
      .file_name()
      .and_then(|n| n.to_str())
      .unwrap_or("Unknown");
    let file_path = path.to_string_lossy().to_string();
    let path_clone = path.to_path_buf();

    div()
      .id(path_clone.to_string_lossy().to_string())
      .flex()
      .items_center()
      .gap_2()
      .px(px(12.0))
      .py(px(8.0))
      .cursor_pointer()
      .hover(|this| this.bg(Colors::hover()))
      .active(|this| this.bg(Colors::border_primary()))
      .child(
        div()
          .w(px(24.0))
          .text_xs()
          .font_weight(gpui::FontWeight::BOLD)
          .text_color(status_color)
          .child(status_str),
      )
      .child(
        div()
          .flex()
          .flex_col()
          .gap_1()
          .flex_1()
          .child(
            div()
              .text_sm()
              .text_color(Colors::text_primary())
              .child(format!("{}{}", file_name, status_badge)),
          )
          .child(
            div()
              .text_xs()
              .text_color(Colors::text_muted())
              .child(file_path),
          ),
      )
  }

  /// Render diff panel with scrolling support and virtualization
  fn render_diff_panel(
    &mut self,
    state: &AppState,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    // Check if we have a selected file to display
    if let Some(repo) = state.workspace.get_active_repo() {
      if let Some(selected_file) = repo.selected_files.first() {
        // Check if we have a diff loaded for this file
        if let Some(diff_state) = &repo.diff {
          if let Some(file_diff) = diff_state.files.iter().find(|f| &f.path == selected_file) {
            return self.render_file_diff(file_diff, window, cx);
          }
        }

        // File selected but no diff loaded yet
        return div()
          .flex()
          .flex_col()
          .items_center()
          .justify_center()
          .size_full()
          .gap_4()
          .child(
            div()
              .text_xl()
              .text_color(Colors::text_secondary())
              .child("Loading diff..."),
          )
          .child(
            div()
              .text_sm()
              .text_color(Colors::text_muted())
              .child(format!("File: {}", selected_file.display())),
          )
          .into_any_element();
      }

      // Repository loaded but no file selected
      if !repo.status.files.is_empty() {
        return div()
          .flex()
          .flex_col()
          .items_center()
          .justify_center()
          .size_full()
          .gap_4()
          .child(
            div()
              .text_xl()
              .text_color(Colors::text_secondary())
              .child("No file selected"),
          )
          .child(
            div()
              .text_sm()
              .text_color(Colors::text_muted())
              .child("Click on a file from the list to view changes"),
          )
          .into_any_element();
      }
    }

    // Empty state
    div()
      .flex()
      .flex_col()
      .items_center()
      .justify_center()
      .size_full()
      .gap_4()
      .child(
        div()
          .text_xl()
          .text_color(Colors::text_secondary())
          .child("No changes"),
      )
      .child(
        div()
          .text_sm()
          .text_color(Colors::text_muted())
          .child("Make some changes to see diffs here"),
      )
      .into_any_element()
  }

  /// Render file diff using EditorDiffView for optimal performance
  fn render_file_diff(
    &mut self,
    file_diff: &crate::state::FileDiff,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let file_name = file_diff
      .path
      .file_name()
      .and_then(|n| n.to_str())
      .unwrap_or("Unknown")
      .to_string();

    let status_text = file_diff.status.as_str().to_string();
    let file_path = file_diff.path.clone();

    // Get or create EditorDiffView for this file
    let editor_view = self
      .editor_diffs
      .entry(file_path)
      .or_insert_with(|| cx.new(|cx| EditorDiffView::new(Arc::new(file_diff.clone()), window, cx)))
      .clone();

    div()
      .flex()
      .flex_col()
      .size_full()
      .bg(Colors::bg_primary())
      // File header
      .child(
        div()
          .flex()
          .items_center()
          .gap_2()
          .px(px(16.0))
          .py(px(12.0))
          .bg(Colors::bg_secondary())
          .border_b_1()
          .border_color(Colors::border_primary())
          .child(
            div()
              .text_sm()
              .font_weight(gpui::FontWeight::BOLD)
              .text_color(Colors::text_primary())
              .child(file_name),
          )
          .child(
            div()
              .text_xs()
              .text_color(Colors::text_muted())
              .child(format!("({})", status_text)),
          ),
      )
      // EditorDiffView for high-performance diff display
      .child(div().flex_1().size_full().child(editor_view))
      .into_any_element()
  }

  /// Handle file click - dispatch SelectFile action
  fn handle_file_click(&mut self, path: PathBuf, _window: &mut Window, cx: &mut Context<Self>) {
    self
      .workspace
      .update(cx, |workspace, cx| {
        if let Err(e) = workspace.dispatch(Action::SelectFile(path.clone()), cx) {
          eprintln!("Failed to select file: {}", e);
        }
      })
      .ok();

    cx.notify();
  }
}

impl Render for MainView {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let state = self
      .workspace
      .upgrade()
      .map(|ws| ws.read(cx).state().clone());

    if let Some(state) = state {
      let has_repo = state.workspace.active_repo.is_some();

      if !has_repo {
        return Self::render_empty_state();
      }

      // Get files to render with click handlers
      let staged_files: Vec<_> = state
        .workspace
        .get_active_repo()
        .map(|repo| {
          repo
            .status
            .files
            .iter()
            .filter(|f| f.staged)
            .cloned()
            .collect()
        })
        .unwrap_or_default();

      let unstaged_files: Vec<_> = state
        .workspace
        .get_active_repo()
        .map(|repo| {
          repo
            .status
            .files
            .iter()
            .filter(|f| !f.staged)
            .cloned()
            .collect()
        })
        .unwrap_or_default();

      div()
        .flex()
        .flex_col()
        .size_full()
        .bg(Colors::bg_primary())
        .child(Self::render_header(&state))
        .child(
          div().flex().flex_1().child(
            div()
              .flex()
              .size_full()
              .child(
                // File list panel with clickable items
                div()
                  .w(px(300.0))
                  .h_full()
                  .border_r_1()
                  .border_color(Colors::border_primary())
                  .child(
                    div()
                      .flex()
                      .flex_col()
                      .size_full()
                      .bg(Colors::bg_primary())
                      // Staged section
                      .child(
                        div()
                          .flex()
                          .flex_col()
                          .child(
                            div()
                              .px(px(12.0))
                              .py(px(8.0))
                              .border_b_1()
                              .border_color(Colors::border_primary())
                              .child(
                                div()
                                  .text_xs()
                                  .font_weight(gpui::FontWeight::BOLD)
                                  .text_color(Colors::text_secondary())
                                  .child(format!("STAGED ({})", staged_files.len())),
                              ),
                          )
                          .children(staged_files.iter().map(|file| {
                            let path = file.path.clone();
                            div()
                              .child(Self::render_file_item(
                                &file.path,
                                file.status.clone(),
                                file.staged,
                              ))
                              .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(move |view, _event, window, cx| {
                                  view.handle_file_click(path.clone(), window, cx);
                                }),
                              )
                          }))
                          .children(if staged_files.is_empty() {
                            vec![div()
                              .px(px(12.0))
                              .py(px(8.0))
                              .text_xs()
                              .text_color(Colors::text_muted())
                              .child("No staged changes")]
                          } else {
                            vec![]
                          }),
                      )
                      // Unstaged section
                      .child(
                        div()
                          .flex()
                          .flex_col()
                          .child(
                            div()
                              .px(px(12.0))
                              .py(px(8.0))
                              .border_b_1()
                              .border_color(Colors::border_primary())
                              .child(
                                div()
                                  .text_xs()
                                  .font_weight(gpui::FontWeight::BOLD)
                                  .text_color(Colors::text_secondary())
                                  .child(format!("CHANGES ({})", unstaged_files.len())),
                              ),
                          )
                          .children(unstaged_files.iter().map(|file| {
                            let path = file.path.clone();
                            div()
                              .child(Self::render_file_item(
                                &file.path,
                                file.status.clone(),
                                file.staged,
                              ))
                              .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(move |view, _event, window, cx| {
                                  view.handle_file_click(path.clone(), window, cx);
                                }),
                              )
                          }))
                          .children(if unstaged_files.is_empty() {
                            vec![div()
                              .px(px(12.0))
                              .py(px(8.0))
                              .text_xs()
                              .text_color(Colors::text_muted())
                              .child("No unstaged changes")]
                          } else {
                            vec![]
                          }),
                      ),
                  ),
              )
              .child(
                // Diff view panel
                div()
                  .flex_1()
                  .h_full()
                  .child(self.render_diff_panel(&state, _window, cx)),
              ),
          ),
        )
        .child(Self::render_status_bar(&state))
        .into_any_element()
    } else {
      Self::render_empty_state()
    }
  }
}
