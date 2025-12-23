use crate::state::{Action, AppState};
use crate::storage::Storage;
use crate::ui::{Colors, DiffView};
use gpui::{
  div, prelude::*, px, AnyElement, Context, DragMoveEvent, Entity, IntoElement, MouseButton,
  ParentElement, Pixels, Styled, WeakEntity, Window,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Drag handle for resizing the file list panel
#[derive(Clone, Copy, Debug)]
struct ResizeHandle;

impl Render for ResizeHandle {
  fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
    // Return an invisible element - the visual is handled by the actual resize handle
    div().size_0()
  }
}

/// Main application view - now a proper entity with click handlers
pub struct MainView {
  workspace: WeakEntity<crate::workspace::Workspace>,
  #[allow(dead_code)]
  storage: Arc<Storage>,
  /// Cache of DiffView for each file
  diff_views: HashMap<PathBuf, Entity<DiffView>>,
  /// Width of the file list panel
  file_list_width: Pixels,
  /// State of the repository dropdown menu
  repository_dropdown_open: bool,
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
      diff_views: HashMap::new(),
      file_list_width: px(300.0), // Default width
      repository_dropdown_open: false,
    }
  }

  /// Resize the file list panel
  fn resize_file_list(&mut self, width: Pixels, cx: &mut Context<Self>) {
    // Clamp width between 200px and 600px
    self.file_list_width = width.max(px(200.0)).min(px(600.0));
    cx.notify();
  }

  /// Render empty state when no repository is open
  pub fn render_empty_state() -> AnyElement {
    div()
      .flex()
      .flex_col()
      .size_full()
      .bg(Colors::bg_primary())
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
                .text_3xl()
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(Colors::text_primary())
                .child("Reviu"),
            )
            .child(
              div()
                .text_xl()
                .text_color(Colors::text_primary())
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
      .child(Self::render_status_bar(None))
      .into_any_element()
  }

  /// Render the status bar
  fn render_status_bar(state: Option<&AppState>) -> impl IntoElement {
    let left_content = if let Some(state) = state {
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
        .gap_4()
        .child(
          div()
            .text_xs()
            .text_color(Colors::text_muted())
            .child(format!("Branch: {}", branch)),
        )
        .child(
          div()
            .text_xs()
            .text_color(Colors::text_muted())
            .child(format!("{} files changed", file_count)),
        )
    } else {
      div().flex().gap_4()
    };

    div()
      .flex()
      .items_center()
      .justify_between()
      .h(px(24.0))
      .px(px(16.0))
      .bg(Colors::bg_primary())
      .border_t_1()
      .border_color(Colors::border_primary())
      .child(left_content)
      .child(
        div()
          .text_xs()
          .text_color(Colors::text_muted())
          .child("Reviu v0.1.0"),
      )
  }

  /// Render a single file item
  fn render_file_item(
    path: &std::path::Path,
    status: crate::state::FileStatusKind,
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
      .gap_4()
      .px(px(12.0))
      .py(px(8.0))
      .cursor_pointer()
      .hover(|this| this.bg(Colors::hover()))
      .active(|this| this.bg(Colors::active()))
      .child(
        div()
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
          .min_w_0()
          .child(
            div()
              .text_sm()
              .text_color(Colors::text_primary())
              .overflow_hidden()
              .text_ellipsis_start()
              .child(file_name.to_string()),
          )
          .child(
            div()
              .text_xs()
              .text_color(Colors::text_muted())
              .overflow_hidden()
              .text_ellipsis_start()
              .child(file_path),
          ),
      )
  }

  /// Render the repository selector dropdown
  fn render_repository_selector(&mut self, state: &AppState, cx: &mut Context<Self>) -> AnyElement {
    let repos = &state.workspace.repos;
    let active_repo = &state.workspace.active_repo;

    // No repositories open
    if repos.is_empty() {
      return div()
        .flex()
        .items_center()
        .w_full()
        .px(px(12.0))
        .py(px(8.0))
        .bg(Colors::bg_secondary())
        .border_b_1()
        .border_color(Colors::border_primary())
        .child(
          div()
            .text_sm()
            .text_color(Colors::text_muted())
            .child("No repository open"),
        )
        .into_any_element();
    }

    // One or more repositories - dropdown
    self.render_repository_dropdown(active_repo, repos, cx)
  }

  /// Render repository dropdown for multiple repos
  fn render_repository_dropdown(
    &mut self,
    active_repo: &Option<PathBuf>,
    repos: &HashMap<PathBuf, crate::state::Repository>,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let active_name = active_repo
      .as_ref()
      .and_then(|p| repos.get(p))
      .map(|r| r.name.clone())
      .unwrap_or_else(|| "No repository".to_string());

    div()
      .relative()
      .flex()
      .flex_col()
      .w_full()
      .bg(Colors::bg_secondary())
      .border_b_1()
      .border_color(Colors::border_primary())
      // Trigger button
      .child(self.render_dropdown_trigger(active_name, cx))
      // Dropdown menu (if open)
      .when(self.repository_dropdown_open, |this| {
        this
          .child(gpui::deferred(self.render_dropdown_menu(repos, active_repo, cx)).with_priority(1))
      })
      .into_any_element()
  }

  /// Render the dropdown trigger button
  fn render_dropdown_trigger(&self, active_name: String, cx: &mut Context<Self>) -> AnyElement {
    div()
      .id("repo-dropdown-trigger")
      .flex()
      .items_center()
      .justify_between()
      .w_full()
      .px(px(12.0))
      .py(px(8.0))
      .cursor_pointer()
      .hover(|this| this.bg(Colors::hover()))
      .active(|this| this.bg(Colors::active()))
      .on_mouse_down(
        MouseButton::Left,
        cx.listener(|view, _event, _window, cx| {
          view.repository_dropdown_open = !view.repository_dropdown_open;
          cx.notify();
        }),
      )
      .child(
        div()
          .text_sm()
          .font_weight(gpui::FontWeight::SEMIBOLD)
          .text_color(Colors::text_primary())
          .child(active_name),
      )
      // Chevron icon
      .child(div().text_xs().text_color(Colors::text_muted()).child(
        if self.repository_dropdown_open {
          "▲"
        } else {
          "▼"
        },
      ))
      .into_any_element()
  }

  /// Render the dropdown menu with repository list
  fn render_dropdown_menu(
    &self,
    repos: &HashMap<PathBuf, crate::state::Repository>,
    active_repo: &Option<PathBuf>,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    // Sort repositories by name
    let mut sorted_repos: Vec<_> = repos.iter().collect();
    sorted_repos.sort_by(|(_, a), (_, b)| a.name.cmp(&b.name));

    div()
      .absolute()
      .top_full()
      .left_0()
      .w_full()
      .occlude()
      .flex()
      .flex_col()
      .bg(Colors::bg_primary())
      .border_1()
      .border_color(Colors::border_primary())
      .shadow_lg()
      .on_mouse_down_out(cx.listener(|this, _, _, cx| {
        this.repository_dropdown_open = false;
        cx.notify();
      }))
      .children(
        sorted_repos
          .iter()
          .enumerate()
          .map(|(idx, (path, repo))| {
            let is_active = active_repo.as_ref() == Some(path);
            let path_clone = (*path).clone();

            self.render_dropdown_item(idx, &repo.name, is_active, path_clone, cx)
          })
          .collect::<Vec<_>>(),
      )
      // Separator
      .child(div().h(px(1.0)).w_full().bg(Colors::border_primary()))
      // "Open Repository..." option
      .child(self.render_open_repository_item(cx))
      .into_any_element()
  }

  /// Render a single item in the dropdown menu
  fn render_dropdown_item(
    &self,
    idx: usize,
    repo_name: &str,
    is_active: bool,
    path: PathBuf,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let path_for_handler = path.clone();
    let repo_name_owned = repo_name.to_string();

    div()
      .id(("repo-item", idx))
      .flex()
      .items_center()
      .justify_between()
      .w_full()
      .px(px(12.0))
      .py(px(6.0))
      .bg(Colors::bg_primary())
      .cursor_pointer()
      .when(is_active, |this| this.bg(Colors::bg_secondary()))
      .hover(|this| this.bg(Colors::hover()))
      .active(|this| this.bg(Colors::active()))
      .on_mouse_down(
        MouseButton::Left,
        cx.listener(move |view, _event, _window, cx| {
          view.handle_repository_select(path_for_handler.clone(), cx);
        }),
      )
      .child(
        div()
          .text_sm()
          .text_color(Colors::text_primary())
          .child(repo_name_owned),
      )
      .when(is_active, |this| {
        this.child(div().text_xs().text_color(Colors::success()).child("✓"))
      })
      .into_any_element()
  }

  /// Handle repository selection from dropdown
  fn handle_repository_select(&mut self, path: PathBuf, cx: &mut Context<Self>) {
    // Close the dropdown
    self.repository_dropdown_open = false;

    // Dispatch the action to switch repository
    if let Some(workspace) = self.workspace.upgrade() {
      workspace.update(cx, |ws, cx| {
        if let Err(e) = ws.dispatch(Action::SwitchRepository(path), cx) {
          log::error!("Failed to switch repository: {}", e);
        }
      });
    }

    cx.notify();
  }

  /// Render "Open Repository..." item in dropdown
  fn render_open_repository_item(&self, cx: &mut Context<Self>) -> AnyElement {
    div()
      .id("open-repository-item")
      .flex()
      .items_center()
      .w_full()
      .px(px(12.0))
      .py(px(6.0))
      .bg(Colors::bg_primary())
      .cursor_pointer()
      .hover(|this| this.bg(Colors::hover()))
      .active(|this| this.bg(Colors::active()))
      .on_mouse_down(
        MouseButton::Left,
        cx.listener(|view, _event, window, cx| {
          view.handle_open_repository(window, cx);
        }),
      )
      .child(
        div()
          .text_sm()
          .text_color(Colors::text_primary())
          .child("Open Repository..."),
      )
      .into_any_element()
  }

  /// Handle "Open Repository..." click - trigger file picker
  fn handle_open_repository(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    // Close the dropdown
    self.repository_dropdown_open = false;

    // Trigger the file picker by dispatching the OpenRepository action (same as Cmd+O)
    window.dispatch_action(Box::new(crate::workspace::OpenRepository), cx);

    cx.notify();
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
              .text_color(Colors::text_muted())
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
              .text_color(Colors::text_muted())
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
          .text_color(Colors::text_muted())
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

  /// Render file diff using DiffView with collapsible hunks
  fn render_file_diff(
    &mut self,
    file_diff: &crate::state::FileDiff,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let file_path = file_diff.path.clone();

    // Get or create DiffView for this file
    let diff_view = self
      .diff_views
      .entry(file_path)
      .or_insert_with(|| cx.new(|cx| DiffView::new(Arc::new(file_diff.clone()), window, cx)))
      .clone();

    diff_view.into_any_element()
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

  /// Render the resize handle for the file list panel
  fn render_resize_handle(&self, cx: &Context<Self>) -> impl IntoElement {
    const RESIZE_HANDLE_SIZE: Pixels = px(6.0);

    div()
      .id("file-list-resize-handle")
      .w(RESIZE_HANDLE_SIZE)
      .h_full()
      .cursor_col_resize()
      .bg(Colors::border_primary())
      .hover(|style| style.bg(Colors::hover()))
      .active(|style| style.bg(Colors::active()))
      .on_drag(ResizeHandle, |_, _, _, cx| {
        cx.stop_propagation();
        cx.new(|_| ResizeHandle)
      })
      .on_mouse_down(
        MouseButton::Left,
        cx.listener(|_, _, _, cx| {
          cx.stop_propagation();
        }),
      )
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
        .child(
          div()
            .flex()
            .flex_1()
            .overflow_hidden()
            .on_drag_move(
              cx.listener(|view, e: &DragMoveEvent<ResizeHandle>, _window, cx| {
                // Calculate new width based on mouse position
                let new_width = e.event.position.x;
                view.resize_file_list(new_width, cx);
              }),
            )
            .child(
              div()
                .flex()
                .size_full()
                .child(
                  // File list panel with clickable items
                  div().w(self.file_list_width).h_full().child(
                    div()
                      .flex()
                      .flex_col()
                      .size_full()
                      .bg(Colors::bg_primary())
                      // Repository selector
                      .child(self.render_repository_selector(&state, cx))
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
                                  .text_color(Colors::text_muted())
                                  .child(format!("STAGED ({})", staged_files.len())),
                              ),
                          )
                          .children(staged_files.iter().map(|file| {
                            let path = file.path.clone();
                            div()
                              .child(Self::render_file_item(&file.path, file.status.clone()))
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
                                  .text_color(Colors::text_muted())
                                  .child(format!("CHANGES ({})", unstaged_files.len())),
                              ),
                          )
                          .children(unstaged_files.iter().map(|file| {
                            let path = file.path.clone();
                            div()
                              .child(Self::render_file_item(&file.path, file.status.clone()))
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
                  // Resize handle
                  self.render_resize_handle(cx),
                )
                .child(
                  // Diff view panel
                  div()
                    .flex_1()
                    .h_full()
                    .overflow_hidden()
                    .child(self.render_diff_panel(&state, _window, cx)),
                ),
            ),
        )
        .child(Self::render_status_bar(Some(&state)))
        .into_any_element()
    } else {
      Self::render_empty_state()
    }
  }
}
