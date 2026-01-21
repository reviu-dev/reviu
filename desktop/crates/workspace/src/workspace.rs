use std::{
  fs,
  path::{Path, PathBuf},
  time::{Duration, Instant, SystemTime},
};

use crate::config::{ConfigStore, RecentRepository};
use crate::theme::{AppColors, app_colors};
use editor::{DiffViewMode, Editor};
use git::{FileStatusKind, RepositoryFile, open_repository};
use gpui::{
  App, ClickEvent, Context, Div, DragMoveEvent, Entity, FocusHandle, Focusable, InteractiveElement,
  PathPromptOptions, Pixels, Point, Render, Rgba, Stateful, Task, Window, actions, deferred, div,
  prelude::*, px, rgb, uniform_list,
};
use syntax::Theme;

const SIDEBAR_DEFAULT_WIDTH: Pixels = px(260.0);
const SIDEBAR_MIN_WIDTH: Pixels = px(200.0);
const SIDEBAR_MAX_WIDTH: Pixels = px(600.0);
const SIDEBAR_RESIZE_HANDLE_WIDTH: Pixels = px(6.0);
const HEADER_HEIGHT: f32 = 36.0;
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

pub struct WorkspaceView {
  root_path: Option<PathBuf>,
  files: Vec<FileEntry>,
  selected_file: Option<usize>,
  editor: Option<Entity<Editor>>,
  error: Option<String>,
  current_dirty: bool,
  poll_task: Option<Task<()>>,
  sidebar_width: Pixels,
  previous_sidebar_drag_position: Option<Point<Pixels>>,
  focus_handle: FocusHandle,
  recent_repositories: Vec<RecentRepository>,
  repo_picker_open: bool,
  theme: Theme,
  diff_view_mode: DiffViewMode,
  last_repo_poll: Option<Instant>,
}

#[derive(Clone)]
struct DraggedSidebar;

impl Render for DraggedSidebar {
  fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
    gpui::Empty
  }
}

impl WorkspaceView {
  pub fn new(cx: &mut Context<Self>) -> Self {
    let recent_repositories = ConfigStore::load_recent_repositories();
    let mut view = Self {
      root_path: None,
      files: Vec::new(),
      selected_file: None,
      editor: None,
      error: None,
      current_dirty: false,
      poll_task: None,
      sidebar_width: SIDEBAR_DEFAULT_WIDTH,
      previous_sidebar_drag_position: None,
      focus_handle: cx.focus_handle(),
      recent_repositories,
      repo_picker_open: false,
      theme: Theme::dark(),
      diff_view_mode: DiffViewMode::Inline,
      last_repo_poll: None,
    };
    view.start_file_polling(cx);
    view
  }

  fn resize_sidebar(&mut self, width: Pixels, cx: &mut Context<Self>) {
    let width = width.max(SIDEBAR_MIN_WIDTH).min(SIDEBAR_MAX_WIDTH).round();
    if self.sidebar_width != width {
      self.sidebar_width = width;
      cx.notify();
    }
  }

  fn toggle_repo_picker(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
    self.repo_picker_open = !self.repo_picker_open;
    cx.notify();
  }

  fn toggle_theme(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
    self.theme.toggle();
    let theme = self.theme.clone();
    if let Some(editor) = self.editor.as_ref() {
      editor.update(cx, |editor, cx| {
        editor.set_theme(theme.clone(), cx);
      });
    }
    cx.notify();
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

  fn select_recent_repository(&mut self, index: usize, cx: &mut Context<Self>) {
    let Some(path) = self
      .recent_repositories
      .get(index)
      .map(|repo| repo.path.clone())
    else {
      return;
    };
    self.repo_picker_open = false;
    self.set_root_path(path, cx);
  }

  fn repo_picker_label(&self) -> String {
    self
      .root_path
      .as_ref()
      .map(|path| root_label(path.as_path()))
      .unwrap_or_else(|| "Select Repository".to_string())
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
    if self.repo_picker_open {
      self.repo_picker_open = false;
      cx.notify();
    }
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
    let Some(entry) = self.files.get_mut(selected_file) else {
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

    let selected_path = self
      .selected_file
      .and_then(|idx| self.files.get(idx).map(|entry| entry.path.clone()));
    let selected_entry = self
      .selected_file
      .and_then(|idx| self.files.get(idx).cloned());
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
    let mut files = repository_entries_to_files(&repo_root, repository.entries);

    if let Some(path) = selected_path.as_ref() {
      if let Some(index) = files.iter().position(|entry| entry.path == *path) {
        if let Some(existing) = selected_entry.as_ref() {
          let entry = &mut files[index];
          entry.current_content = existing.current_content.clone();
          entry.saved_content = existing.saved_content.clone();
          entry.last_modified = existing.last_modified;
        }
      } else if selected_dirty {
        if let Some(mut entry) = selected_entry {
          entry.status = FileStatusKind::Modified;
          files.push(entry);
        }
      } else {
        self.selected_file = None;
        self.editor = None;
        self.current_dirty = false;
      }
    }

    files.sort_by(|a, b| a.path.cmp(&b.path));
    let next_selected = selected_path
      .as_ref()
      .and_then(|path| files.iter().position(|entry| entry.path == *path));

    self.files = files;
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
    let Some(entry) = self.files.get_mut(selected_file) else {
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
    self.repo_picker_open = false;
    match open_repository(&path) {
      Ok(repository) => {
        let repo_root = repository.root;
        self.root_path = Some(repo_root.clone());
        ConfigStore::persist_recent_repository(&repo_root);
        bump_recent_repository(&mut self.recent_repositories, repo_root.clone());
        self.files = repository_entries_to_files(&repo_root, repository.entries);
        self.selected_file = None;
        self.editor = None;
        self.error = None;
        self.current_dirty = false;
        self.last_repo_poll = Some(Instant::now());
      }
      Err(err) => {
        self.root_path = Some(path);
        self.files = Vec::new();
        self.selected_file = None;
        self.editor = None;
        self.error = Some(format!("Not a git repository: {err}"));
        self.current_dirty = false;
        self.last_repo_poll = Some(Instant::now());
      }
    }
    cx.notify();
  }

  fn select_file(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
    let (content, base_content, file_path) = match self.files.get_mut(index) {
      Some(entry) => {
        refresh_entry_from_disk(entry);
        (
          entry.current_content.clone(),
          entry.base_content.clone(),
          entry.path.clone(),
        )
      }
      None => return,
    };

    let Some(content) = content else {
      self.editor = None;
      self.selected_file = Some(index);
      self.error = Some(format!(
        "File content unavailable: {}",
        self
          .files
          .get(index)
          .map(|entry| entry.display_name.clone())
          .unwrap_or_else(|| "Unknown file".to_string())
      ));
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

    if let Some(entry) = self.files.get_mut(index) {
      if entry.saved_content.is_none() {
        entry.saved_content = Some(content.clone());
      }
      entry.last_modified = read_modified_time(&entry.path);
    }

    self.editor = Some(editor);
    self.selected_file = Some(index);
    self.error = None;
    self.current_dirty = false;

    window.focus(&focus_handle, cx);
    cx.notify();
  }

  fn render_empty_state(&mut self, cx: &mut Context<Self>) -> Div {
    let colors = app_colors(&self.theme);
    let (message, color, show_hint) = if let Some(error) = &self.error {
      (error.clone(), colors.error_text, false)
    } else {
      (
        "Open a repository to get started.".to_string(),
        colors.text,
        true,
      )
    };

    div()
      .key_context("Workspace")
      .track_focus(&self.focus_handle(cx))
      .on_action(cx.listener(Self::open_repository_action))
      .on_action(cx.listener(Self::save_file_action))
      .size_full()
      .bg(colors.app_bg)
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
            .text_color(colors.text_subtle)
            .child("Press Cmd+O to open a repository."),
        )
      })
      .child(action_button(
        "open-folder-empty",
        "Open Repository",
        cx.listener(Self::open_repository_clicked),
        &colors,
      ))
  }

  fn render_app_header(&mut self, cx: &mut Context<Self>) -> Div {
    let colors = app_colors(&self.theme);
    let toggle_label = if self.theme.is_dark {
      "Light Mode"
    } else {
      "Dark Mode"
    };
    let diff_label = match self.diff_view_mode {
      DiffViewMode::Inline => "Split Diff",
      DiffViewMode::Split => "Inline Diff",
    };

    div()
      .h(px(APP_HEADER_HEIGHT))
      .px_3()
      .flex()
      .items_center()
      .justify_between()
      .bg(colors.header_bg)
      .border_b_1()
      .border_color(colors.border)
      .child(div().text_sm().text_color(colors.text).child("Reviu"))
      .child(
        div()
          .flex()
          .items_center()
          .gap_2()
          .child(action_button(
            "diff-toggle",
            diff_label,
            cx.listener(Self::toggle_diff_view),
            &colors,
          ))
          .child(action_button(
            "theme-toggle",
            toggle_label,
            cx.listener(Self::toggle_theme),
            &colors,
          )),
      )
  }

  fn render_repo_picker_header(&mut self, cx: &mut Context<Self>) -> Div {
    let colors = app_colors(&self.theme);
    let label = self.repo_picker_label();
    let icon = if self.repo_picker_open { "^" } else { "v" };

    div().border_b_1().border_color(colors.border).child(
      div()
        .id("repo-picker-toggle")
        .h(px(HEADER_HEIGHT))
        .px_3()
        .flex()
        .items_center()
        .justify_between()
        .bg(colors.header_bg)
        .cursor_pointer()
        .child(div().text_sm().text_color(colors.text).child(label))
        .child(div().text_sm().text_color(colors.text_subtle).child(icon))
        .on_click(cx.listener(Self::toggle_repo_picker)),
    )
  }

  fn render_repo_picker_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
    let colors = app_colors(&self.theme);
    let mut menu = div()
      .flex()
      .flex_col()
      .bg(colors.menu_bg)
      .border_t_1()
      .border_color(colors.border)
      .id("repo-picker-menu")
      .absolute()
      .top(px(HEADER_HEIGHT))
      .left(px(0.0))
      .right(px(0.0))
      .w_full()
      .max_h(px(240.0))
      .overflow_y_scroll()
      .occlude();

    if self.recent_repositories.is_empty() {
      menu = menu.child(
        div()
          .px_3()
          .py_2()
          .text_sm()
          .text_color(colors.text_muted)
          .child("No recent repositories."),
      );
    } else {
      for (index, repo) in self.recent_repositories.iter().enumerate() {
        menu = menu.child(self.render_repo_menu_item(index, repo, cx));
      }
    }

    menu.child(self.render_repo_add_item(cx))
  }

  fn render_repo_menu_item(
    &self,
    index: usize,
    repo: &RecentRepository,
    cx: &mut Context<Self>,
  ) -> Stateful<Div> {
    let colors = app_colors(&self.theme);
    let is_selected = self.root_path.as_ref() == Some(&repo.path);
    let label = root_label(repo.path.as_path());

    div()
      .id(("repo-picker-item", index))
      .px_3()
      .py_2()
      .w_full()
      .text_sm()
      .cursor_pointer()
      .text_color(colors.text)
      .when_else(
        is_selected,
        |this| {
          this
            .bg(colors.menu_selected_bg)
            .text_color(colors.menu_selected_text)
        },
        |this| {
          this
            .bg(colors.menu_bg)
            .hover(|style| style.bg(colors.menu_hover_bg))
        },
      )
      .child(label)
      .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
        this.select_recent_repository(index, cx);
      }))
  }

  fn render_repo_add_item(&self, cx: &mut Context<Self>) -> Stateful<Div> {
    let colors = app_colors(&self.theme);
    div()
      .id("repo-picker-add")
      .px_3()
      .py_2()
      .w_full()
      .text_sm()
      .text_color(colors.text)
      .cursor_pointer()
      .border_t_1()
      .border_color(colors.border)
      .bg(colors.menu_bg)
      .hover(|style| style.bg(colors.menu_hover_bg))
      .child("Add Repository...".to_string())
      .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
        this.start_open_repository(cx);
      }))
  }

  fn render_sidebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let colors = app_colors(&self.theme);
    let list = uniform_list(
      "file-list",
      self.files.len(),
      cx.processor(|this, range, _window, cx| {
        let mut items = Vec::new();
        for idx in range {
          let Some(entry) = this.files.get(idx) else {
            continue;
          };
          items.push(this.render_display_line(idx, entry, cx));
        }
        items
      }),
    )
    .h_full();

    let mut sidebar_body = div().flex_1().overflow_hidden().child(list);
    if self.files.is_empty() {
      let message = if self.root_path.is_some() {
        "No changes found."
      } else {
        "No repository selected."
      };
      sidebar_body = div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .text_color(colors.text_muted)
        .child(message);
    }

    let resize_handle = deferred(
      div()
        .id("sidebar-resize-handle")
        .on_drag(DraggedSidebar, |drag, _, _, cx| {
          cx.stop_propagation();
          cx.new(|_| drag.clone())
        })
        .absolute()
        .right(-SIDEBAR_RESIZE_HANDLE_WIDTH / 2.0)
        .top(px(0.0))
        .h_full()
        .w(SIDEBAR_RESIZE_HANDLE_WIDTH)
        .cursor_col_resize()
        .occlude(),
    );

    div()
      .w(self.sidebar_width)
      .flex_none()
      .flex()
      .flex_col()
      .h_full()
      .relative()
      .bg(colors.sidebar_bg)
      .border_r_1()
      .border_color(colors.border)
      .child(self.render_repo_picker_header(cx))
      .child(sidebar_body)
      .when(self.repo_picker_open, |this| {
        this.child(self.render_repo_picker_menu(cx))
      })
      .child(resize_handle)
  }

  fn render_editor_header(&mut self, cx: &mut Context<Self>) -> Div {
    let colors = app_colors(&self.theme);
    let title = self
      .selected_file
      .and_then(|idx| self.files.get(idx).map(|entry| entry.display_name.clone()))
      .unwrap_or_else(|| "File".to_string());

    let mut header = div()
      .h(px(HEADER_HEIGHT))
      .px_3()
      .flex()
      .items_center()
      .justify_between()
      .bg(colors.header_bg)
      .child(div().text_sm().text_color(colors.text).child(title));

    if self.current_dirty {
      header = header.child(action_button(
        "save-file",
        "Save",
        cx.listener(Self::save_file_clicked),
        &colors,
      ));
    }

    header
  }

  fn render_display_line(
    &self,
    idx: usize,
    entry: &FileEntry,
    cx: &mut Context<Self>,
  ) -> Stateful<Div> {
    let colors = app_colors(&self.theme);
    let is_selected = self.selected_file == Some(idx);
    let is_dirty = is_selected && self.current_dirty;
    let (tag, tag_color) = status_tag(entry.status);

    div()
      .id(idx)
      .px_2()
      .py_1()
      .w_full()
      .text_sm()
      .text_color(colors.list_text)
      .cursor_pointer()
      .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
        this.select_file(idx, window, cx);
      }))
      .flex()
      .items_center()
      .gap_2()
      .when_else(
        is_selected,
        |this| {
          this
            .bg(colors.list_selected_bg)
            .text_color(colors.list_selected_text)
        },
        |this| {
          this
            .bg(colors.list_bg)
            .hover(|style| style.bg(colors.list_hover_bg))
        },
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
      .when(is_dirty, |this| {
        this.child(
          div()
            .flex_none()
            .text_sm()
            .text_color(rgb(0xe0b84a))
            .child("*"),
        )
      })
  }
}

impl Render for WorkspaceView {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let colors = app_colors(&self.theme);
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
          .bg(colors.surface_bg)
          .child(
            div()
              .flex()
              .items_center()
              .justify_center()
              .size_full()
              .text_color(colors.text_muted)
              .child("Select a repository to get started."),
          )
      } else {
        let mut main = div()
          .flex_1()
          .flex()
          .flex_col()
          .min_w(px(0.0))
          .size_full()
          .bg(colors.surface_bg);

        if let Some(editor) = self.editor.clone() {
          let header = self.render_editor_header(cx);
          main = main
            .child(header)
            .child(div().flex_1().min_w(px(0.0)).child(editor));
        } else {
          let (message, color) = if let Some(error) = &self.error {
            (error.clone(), colors.error_text)
          } else if self.files.is_empty() {
            (
              "No changes found in this repository.".to_string(),
              colors.text_muted,
            )
          } else {
            ("Select a file to view it.".to_string(), colors.text_muted)
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

      div()
        .flex_1()
        .flex()
        .flex_row()
        .bg(colors.app_bg)
        .child(self.render_sidebar(cx))
        .child(main)
    };

    div()
      .size_full()
      .flex()
      .flex_col()
      .bg(colors.app_bg)
      .child(self.render_app_header(cx))
      .child(content)
      .key_context("Workspace")
      .track_focus(&self.focus_handle(cx))
      .on_action(cx.listener(Self::open_repository_action))
      .on_action(cx.listener(Self::save_file_action))
      .on_drag_move(
        cx.listener(|workspace, e: &DragMoveEvent<DraggedSidebar>, _, cx| {
          if workspace.previous_sidebar_drag_position != Some(e.event.position) {
            workspace.previous_sidebar_drag_position = Some(e.event.position);
            let new_width = e.event.position.x - e.bounds.left();
            workspace.resize_sidebar(new_width, cx);
          }
        }),
      )
  }
}

impl Focusable for WorkspaceView {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
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

fn status_tag(status: FileStatusKind) -> (&'static str, Rgba) {
  match status {
    FileStatusKind::Added => ("A", rgb(0x4fa86b)),
    FileStatusKind::Untracked => ("U", rgb(0x4fa86b)),
    FileStatusKind::Modified => ("M", rgb(0xd08c3f)),
    FileStatusKind::Deleted => ("D", rgb(0xd26666)),
    FileStatusKind::Renamed => ("R", rgb(0xd08c3f)),
    FileStatusKind::Typechange => ("T", rgb(0xd08c3f)),
    FileStatusKind::Conflicted => ("C", rgb(0xd26666)),
  }
}

fn action_button(
  id: &'static str,
  label: &str,
  on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
  colors: &AppColors,
) -> impl IntoElement {
  div()
    .id(id)
    .px_3()
    .py_1()
    .bg(colors.button_bg)
    .text_color(colors.button_text)
    .text_sm()
    .border_1()
    .border_color(colors.button_border)
    .rounded_sm()
    .cursor_pointer()
    .hover(|style| style.opacity(0.9))
    .child(label.to_string())
    .on_click(on_click)
}
