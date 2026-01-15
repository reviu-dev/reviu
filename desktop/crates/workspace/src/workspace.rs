use std::path::{Path, PathBuf};

use editor::Editor;
use git::{FileStatusKind, open_repository};
use gpui::{
  App, ClickEvent, Context, Div, Entity, Focusable, InteractiveElement, PathPromptOptions, Render,
  Rgba, Stateful, Window, actions, div, prelude::*, px, rgb, uniform_list,
};

const SIDEBAR_WIDTH: f32 = 260.0;
const HEADER_HEIGHT: f32 = 36.0;

actions!(workspace, [OpenRepository]);

#[derive(Clone)]
struct FileEntry {
  path: PathBuf,
  display_name: String,
  status: FileStatusKind,
  base_content: Option<String>,
  current_content: Option<String>,
}

pub struct WorkspaceView {
  root_path: Option<PathBuf>,
  files: Vec<FileEntry>,
  selected_file: Option<usize>,
  editor: Option<Entity<Editor>>,
  error: Option<String>,
}

impl WorkspaceView {
  pub fn new(_cx: &mut Context<Self>) -> Self {
    Self {
      root_path: None,
      files: Vec::new(),
      selected_file: None,
      editor: None,
      error: None,
    }
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

  fn set_root_path(&mut self, path: PathBuf, cx: &mut Context<Self>) {
    match open_repository(&path) {
      Ok(repository) => {
        let repo_root = repository.root;
        self.root_path = Some(repo_root.clone());
        self.files = repository
          .entries
          .into_iter()
          .map(|entry| FileEntry {
            path: repo_root.join(&entry.path),
            display_name: entry.path.to_string_lossy().to_string(),
            status: entry.status,
            base_content: entry.base_content,
            current_content: entry.current_content,
          })
          .collect();
        self.selected_file = None;
        self.editor = None;
        self.error = None;
      }
      Err(err) => {
        self.root_path = Some(path);
        self.files = Vec::new();
        self.selected_file = None;
        self.editor = None;
        self.error = Some(format!("Not a git repository: {err}"));
      }
    }
    cx.notify();
  }

  fn select_file(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
    let Some(entry) = self.files.get(index) else {
      return;
    };

    let Some(content) = entry.current_content.clone() else {
      self.editor = None;
      self.selected_file = Some(index);
      self.error = Some(format!(
        "File content unavailable: {}",
        entry.display_name
      ));
      cx.notify();
      return;
    };

    let base_content = entry.base_content.clone();
    let file_ext = entry.path.extension().and_then(|ext| ext.to_str());
    let editor = cx.new(|cx| Editor::new(&content, base_content.as_deref(), file_ext, cx));
    let focus_handle = editor.read(cx).focus_handle(cx);

    self.editor = Some(editor);
    self.selected_file = Some(index);
    self.error = None;

    window.focus(&focus_handle, cx);
    cx.notify();
  }

  fn render_empty_state(&mut self, cx: &mut Context<Self>) -> Div {
    let (message, color, show_hint) = if let Some(error) = &self.error {
      (error.clone(), rgb(0xcc6666), false)
    } else {
      (
        "Open a repository to get started.".to_string(),
        rgb(0xd0d0d0),
        true,
      )
    };

    div()
      .key_context("Workspace")
      .on_action(cx.listener(Self::open_repository_action))
      .size_full()
      .bg(rgb(0x141414))
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
            .text_color(rgb(0x9a9a9a))
            .child("Press Cmd+O to open a repository."),
        )
      })
      .child(action_button(
        "open-folder-empty",
        "Open Repository",
        cx.listener(Self::open_repository_clicked),
      ))
  }

  fn render_sidebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let root_label = self
      .root_path
      .as_ref()
      .map(|path| root_label(path.as_path()))
      .unwrap_or_else(|| "Repository".to_string());

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
      sidebar_body = div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .text_color(rgb(0x808080))
        .child("No changes found.");
    }

    div()
      .w(px(SIDEBAR_WIDTH))
      .flex()
      .flex_col()
      .h_full()
      .bg(rgb(0x181818))
      .border_r_1()
      .border_color(rgb(0x2a2a2a))
      .child(
        div()
          .h(px(HEADER_HEIGHT))
          .px_3()
          .flex()
          .items_center()
          .justify_between()
          .bg(rgb(0x1d1d1d))
          .child(div().text_sm().text_color(rgb(0xe0e0e0)).child(root_label))
          .child(action_button(
            "open-folder-sidebar",
            "Open Repository",
            cx.listener(Self::open_repository_clicked),
          )),
      )
      .child(sidebar_body)
  }

  fn render_display_line(
    &self,
    idx: usize,
    entry: &FileEntry,
    cx: &mut Context<Self>,
  ) -> Stateful<Div> {
    let is_selected = self.selected_file == Some(idx);
    let (tag, tag_color) = status_tag(entry.status);

    div()
      .id(idx)
      .px_2()
      .py_1()
      .w_full()
      .text_sm()
      .text_color(rgb(0xcfcfcf))
      .cursor_pointer()
      .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
        this.select_file(idx, window, cx);
      }))
      .flex()
      .items_center()
      .gap_2()
      .when_else(
        is_selected,
        |this| this.bg(rgb(0x2a2a2a)),
        |this| {
          this
            .bg(rgb(0x1a1a1a))
            .hover(|style| style.bg(rgb(0x222222)))
        },
      )
      .child(div().flex_none().text_sm().text_color(tag_color).child(tag))
      .child(entry.display_name.clone())
  }
}

impl Render for WorkspaceView {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    if self.root_path.is_none() {
      return self.render_empty_state(cx);
    }

    let mut main = div()
      .flex_1()
      .flex()
      .flex_col()
      .size_full()
      .bg(rgb(0x1b1b1b));

    if let Some(editor) = &self.editor {
      main = main.child(editor.clone());
    } else {
      let (message, color) = if let Some(error) = &self.error {
        (error.clone(), rgb(0xcc6666))
      } else if self.files.is_empty() {
        ("No changes found in this repository.".to_string(), rgb(0x808080))
      } else {
        ("Select a file to view it.".to_string(), rgb(0x808080))
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

    div()
      .size_full()
      .flex()
      .flex_row()
      .bg(rgb(0x141414))
      .child(self.render_sidebar(cx))
      .child(main)
      .key_context("Workspace")
      .on_action(cx.listener(Self::open_repository_action))
  }
}

fn root_label(path: &Path) -> String {
  path
    .file_name()
    .and_then(|name| name.to_str())
    .map(|name| name.to_string())
    .unwrap_or_else(|| path.display().to_string())
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
) -> impl IntoElement {
  div()
    .id(id)
    .px_3()
    .py_1()
    .bg(rgb(0x2a2a2a))
    .text_color(rgb(0xffffff))
    .text_sm()
    .border_1()
    .border_color(rgb(0x3a3a3a))
    .rounded_sm()
    .cursor_pointer()
    .hover(|style| style.opacity(0.9))
    .child(label.to_string())
    .on_click(on_click)
}
