//! Session review panel: working-tree changeset and commit actions.

use std::path::PathBuf;

use git::{
  RepoStage, RepoStatusEntry, RepoStatusKind, commit_changes, list_repo_status, stage_all,
};
use gpui::{
  AnyElement, AnyWindowHandle, App, Context, Entity, FocusHandle, Focusable, Render, SharedString,
  Task, Window, div, prelude::*, px,
};
use gpui_component::{ActiveTheme as _, Disableable as _, Icon, Sizable as _, h_flex, v_flex};
use smol::unblock;

use crate::git_page::read_commit_diff;
use crate::workspace::WorkspaceApi;
use ui::{
  Button, ButtonVariants as _, StatusThemeExt as _, Textarea, TextareaState, UiIconName,
};

fn status_color(kind: RepoStatusKind, theme: &gpui_component::Theme) -> gpui::Hsla {
  match kind {
    RepoStatusKind::Modified => theme.status_yellow(),
    RepoStatusKind::Added => theme.status_green(),
    RepoStatusKind::Deleted => theme.status_red(),
    RepoStatusKind::Renamed => theme.status_blue(),
    RepoStatusKind::TypeChange => theme.status_blue(),
    RepoStatusKind::Untracked => theme.status_green(),
    RepoStatusKind::Conflicted => theme.status_red(),
  }
}

pub(crate) fn split_path_label(path: &std::path::Path) -> (String, String) {
  let file = path
    .file_name()
    .map(|name| name.to_string_lossy().into_owned())
    .unwrap_or_else(|| path.to_string_lossy().into_owned());
  let dir = path
    .parent()
    .filter(|parent| !parent.as_os_str().is_empty())
    .map(|parent| format!("{}/", parent.to_string_lossy()))
    .unwrap_or_default();
  (dir, file)
}

#[derive(Clone, Debug)]
pub enum ReviewPanelEvent {
  OpenFile { path: PathBuf },
}

impl gpui::EventEmitter<ReviewPanelEvent> for ReviewPanel {}

pub struct ReviewPanel {
  focus_handle: FocusHandle,
  window_handle: AnyWindowHandle,
  repo_root: Option<PathBuf>,
  status_entries: Vec<RepoStatusEntry>,
  commit_input: Entity<TextareaState>,
  committing: bool,
  generating_message: bool,
  last_error: Option<SharedString>,
  _refresh_task: Option<Task<()>>,
  _commit_task: Option<Task<()>>,
}

impl ReviewPanel {
  pub fn new(repo_root: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
    let commit_input = cx.new(|cx| {
      TextareaState::new(window, cx)
        .auto_grow(1, 5)
        .placeholder("Commit message...")
    });

    let mut panel = Self {
      focus_handle: cx.focus_handle(),
      window_handle: window.window_handle(),
      repo_root,
      status_entries: Vec::new(),
      commit_input,
      committing: false,
      generating_message: false,
      last_error: None,
      _refresh_task: None,
      _commit_task: None,
    };
    panel.refresh(cx);
    panel
  }

  pub fn refresh(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.repo_root.clone() else {
      self.status_entries.clear();
      cx.notify();
      return;
    };

    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || list_repo_status(&repo_root)).await;
      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(entries) => {
            this.status_entries = entries;
            this.last_error = None;
          }
          Err(error) => this.last_error = Some(format!("{error}").into()),
        }
        cx.notify();
      });
    });
    self._refresh_task = Some(task);
  }

  fn has_staged_changes(&self) -> bool {
    self
      .status_entries
      .iter()
      .any(|entry| !matches!(entry.stage, RepoStage::Unstaged))
  }

  fn commit(&mut self, cx: &mut Context<Self>) {
    if self.committing {
      return;
    }
    let Some(repo_root) = self.repo_root.clone() else {
      return;
    };
    let message = self.commit_input.read(cx).value().to_string();
    if message.trim().is_empty() || self.status_entries.is_empty() {
      return;
    }
    let stage_all_needed = !self.has_staged_changes();
    self.committing = true;
    self.last_error = None;
    cx.notify();
    crate::analytics::track(cx, "commit_made");

    let window_handle = self.window_handle;
    let commit_input = self.commit_input.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || {
        if stage_all_needed {
          stage_all(&repo_root)?;
        }
        commit_changes(&repo_root, &message)
      })
      .await;

      let _ = this.update(cx, |this, cx| {
        this.committing = false;
        match result {
          Ok(()) => {
            let _ = cx.update_window(window_handle, |_, window, cx| {
              commit_input.update(cx, |input, cx| input.set_value("", window, cx));
            });
          }
          Err(error) => this.last_error = Some(format!("{error}").into()),
        }
        this.refresh(cx);
      });
    });
    self._commit_task = Some(task);
  }

  fn generate_commit_message(&mut self, cx: &mut Context<Self>) {
    if self.generating_message {
      return;
    }
    let Some(repo_root) = self.repo_root.clone() else {
      return;
    };
    let staged = self.has_staged_changes();
    let api = WorkspaceApi::global(cx).api.clone();
    self.generating_message = true;
    self.last_error = None;
    cx.notify();

    let window_handle = self.window_handle;
    let commit_input = self.commit_input.clone();
    cx.spawn(async move |this, cx| {
      let result = unblock(move || {
        let diff = read_commit_diff(&repo_root, staged)?;
        if diff.trim().is_empty() {
          anyhow::bail!("No changes to summarize");
        }
        api.generate_commit_message(&diff)
      })
      .await;

      let _ = this.update(cx, |this, cx| {
        this.generating_message = false;
        match result {
          Ok(message) => {
            let _ = cx.update_window(window_handle, |_, window, cx| {
              commit_input.update(cx, |input, cx| input.set_value(&message, window, cx));
            });
          }
          Err(error) => this.last_error = Some(format!("{error}").into()),
        }
        cx.notify();
      });
    })
    .detach();
  }

  fn render_file_row(
    &self,
    ix: usize,
    entry: &RepoStatusEntry,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let theme = cx.theme().clone();
    let (dir, file) = split_path_label(&entry.path);
    let path = entry.path.clone();

    div()
      .id(("review-panel-file-row", ix))
      .mx_2()
      .px_2()
      .py_1()
      .rounded(px(5.0))
      .cursor_pointer()
      .hover(|s| s.bg(theme.secondary_hover))
      .on_click(cx.listener(move |_, _, _, cx| {
        cx.emit(ReviewPanelEvent::OpenFile { path: path.clone() });
      }))
      .child(
        h_flex()
          .items_center()
          .gap_2()
          .child(
            div()
              .w(px(12.0))
              .flex_shrink_0()
              .text_xs()
              .font_weight(gpui::FontWeight::BOLD)
              .text_color(status_color(entry.status, &theme))
              .child(entry.status.short_code()),
          )
          .child(
            h_flex()
              .flex_1()
              .min_w(px(0.0))
              .overflow_hidden()
              .text_sm()
              .whitespace_nowrap()
              .when(!dir.is_empty(), |this| {
                this.child(
                  div()
                    .text_color(theme.muted_foreground)
                    .truncate()
                    .child(dir),
                )
              })
              .child(div().text_color(theme.foreground).child(file)),
          ),
      )
      .into_any_element()
  }

  fn render_commit_zone(&self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let can_commit = !self.committing
      && !self.status_entries.is_empty()
      && !self.commit_input.read(cx).value().trim().is_empty();

    v_flex()
      .gap_2()
      .p_3()
      .border_t_1()
      .border_color(theme.border)
      .when_some(self.last_error.clone(), |this, error| {
        this.child(
          div()
            .text_xs()
            .text_color(theme.status_red())
            .child(error),
        )
      })
      .child(
        h_flex()
          .items_end()
          .gap_2()
          .child(div().flex_1().child(Textarea::new(&self.commit_input).w_full()))
          .child(
            Button::new("review-panel-generate-message")
              .icon(UiIconName::Sparkles)
              .ghost()
              .compact()
              .small()
              .loading(self.generating_message)
              .disabled(self.generating_message || self.status_entries.is_empty())
              .tooltip("Generate commit message")
              .on_click(cx.listener(|this, _, _, cx| this.generate_commit_message(cx))),
          ),
      )
      .child(
        Button::new("review-panel-commit")
          .primary()
          .small()
          .w_full()
          .label(if self.committing {
            "Committing..."
          } else {
            "Commit"
          })
          .loading(self.committing)
          .disabled(!can_commit)
          .on_click(cx.listener(|this, _, _, cx| this.commit(cx))),
      )
      .into_any_element()
  }

  fn render_empty_state(&self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    v_flex()
      .flex_1()
      .items_center()
      .justify_center()
      .gap_2()
      .child(
        Icon::new(UiIconName::CircleCheck)
          .size_4()
          .text_color(theme.muted_foreground),
      )
      .child(
        div()
          .text_sm()
          .text_color(theme.muted_foreground)
          .child(if self.repo_root.is_some() {
            "No changes"
          } else {
            "No repository"
          }),
      )
      .into_any_element()
  }
}

impl Render for ReviewPanel {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let entry_count = self.status_entries.len();

    let header = h_flex()
      .items_center()
      .justify_between()
      .px_3()
      .py_2()
      .child(
        h_flex()
          .items_center()
          .gap_2()
          .child(
            div()
              .text_xs()
              .font_weight(gpui::FontWeight::SEMIBOLD)
              .text_color(theme.muted_foreground)
              .child("Changes"),
          )
          .when(entry_count > 0, |this| {
            this.child(
              div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(entry_count.to_string()),
            )
          }),
      )
      .child(
        Button::new("review-panel-refresh")
          .icon(UiIconName::RefreshCw)
          .ghost()
          .compact()
          .small()
          .tooltip("Refresh changes")
          .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
      );

    let body = if self.status_entries.is_empty() {
      self.render_empty_state(cx)
    } else {
      let rows: Vec<AnyElement> = self
        .status_entries
        .clone()
        .iter()
        .enumerate()
        .map(|(ix, entry)| self.render_file_row(ix, entry, cx))
        .collect();
      div()
        .id("review-panel-file-list")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .py_1()
        .children(rows)
        .into_any_element()
    };

    v_flex()
      .size_full()
      .min_w(px(0.0))
      .min_h_0()
      .bg(theme.sidebar)
      .border_l_1()
      .border_color(theme.border)
      .track_focus(&self.focus_handle)
      .child(header)
      .child(body)
      .child(self.render_commit_zone(cx))
  }
}

impl Focusable for ReviewPanel {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use git2::{Repository, Signature};
  use gpui::TestAppContext;
  use std::path::Path;
  use std::time::{SystemTime, UNIX_EPOCH};

  #[test]
  fn split_path_label_separates_dir_and_file() {
    assert_eq!(
      split_path_label(Path::new("crates/workspace/src/git_page.rs")),
      ("crates/workspace/src/".to_string(), "git_page.rs".to_string())
    );
  }

  #[test]
  fn split_path_label_handles_root_files() {
    assert_eq!(
      split_path_label(Path::new("CHANGELOG.md")),
      (String::new(), "CHANGELOG.md".to_string())
    );
  }

  struct TempRepo {
    path: PathBuf,
  }

  impl TempRepo {
    fn init(prefix: &str) -> Self {
      let mut path = std::env::temp_dir();
      let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
      path.push(format!("reviu-{prefix}-{}-{nanos}", std::process::id()));
      std::fs::create_dir_all(&path).expect("create temp dir");
      Repository::init(&path).expect("init git repository");
      Self { path }
    }
  }

  impl Drop for TempRepo {
    fn drop(&mut self) {
      let _ = std::fs::remove_dir_all(&self.path);
    }
  }

  fn commit_text_file(repo_root: &Path, rel_path: &Path, contents: &str, message: &str) {
    let repo = Repository::open(repo_root).expect("open repo");
    std::fs::write(repo_root.join(rel_path), contents).expect("write worktree file");

    let mut index = repo.index().expect("open index");
    index.add_path(rel_path).expect("stage file");
    index.write().expect("write index");
    let tree_id = index.write_tree().expect("write tree");
    let tree = repo.find_tree(tree_id).expect("find tree");
    let signature = Signature::now("Reviu Tests", "tests@reviu.local").expect("signature");
    let parent = repo.head().ok().and_then(|head| head.peel_to_commit().ok());
    let parents: Vec<_> = parent.iter().collect();
    repo
      .commit(
        Some("HEAD"),
        &signature,
        &signature,
        message,
        &tree,
        &parents,
      )
      .expect("commit");
  }

  async fn await_refresh(panel: &Entity<ReviewPanel>, cx: &mut gpui::VisualTestContext) {
    let task = panel.update(cx, |panel, _| panel._refresh_task.take());
    if let Some(task) = task {
      task.await;
    }
    cx.run_until_parked();
  }

  fn add_review_panel_window(
    repo_root: Option<PathBuf>,
    cx: &mut TestAppContext,
  ) -> (Entity<ReviewPanel>, &mut gpui::VisualTestContext) {
    let mut mounted: Option<Entity<ReviewPanel>> = None;
    let (_root, cx) = cx.add_window_view(|window, cx| {
      let panel = cx.new(|cx| ReviewPanel::new(repo_root.clone(), window, cx));
      mounted = Some(panel.clone());
      gpui_component::Root::new(panel, window, cx)
    });
    (mounted.expect("review panel"), cx)
  }

  #[gpui::test]
  async fn refresh_lists_working_tree_changes(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_component::init(cx));
    let repo = TempRepo::init("review-panel-refresh");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");

    let (panel, cx) = add_review_panel_window(Some(repo.path.clone()), cx);
    cx.executor().allow_parking();
    await_refresh(&panel, cx).await;

    panel.read_with(cx, |panel, _| {
      assert_eq!(panel.status_entries.len(), 1);
      assert_eq!(panel.status_entries[0].path, PathBuf::from("README.md"));
      assert_eq!(panel.status_entries[0].status, RepoStatusKind::Modified);
    });
  }

  #[gpui::test]
  async fn commit_stages_and_commits_all_changes(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_component::init(cx));
    let repo = TempRepo::init("review-panel-commit");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");

    let (panel, cx) = add_review_panel_window(Some(repo.path.clone()), cx);
    cx.executor().allow_parking();
    await_refresh(&panel, cx).await;

    panel.update_in(cx, |panel, window, cx| {
      panel
        .commit_input
        .update(cx, |input, cx| input.set_value("feat: update readme", window, cx));
    });
    panel.update(cx, |panel, cx| panel.commit(cx));

    let commit_task = panel.update(cx, |panel, _| panel._commit_task.take().expect("commit task"));
    commit_task.await;
    await_refresh(&panel, cx).await;

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let head = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read head");
    assert_eq!(head.summary(), Some("feat: update readme"));
    panel.read_with(cx, |panel, cx| {
      assert!(panel.status_entries.is_empty());
      assert!(panel.commit_input.read(cx).value().is_empty());
    });
  }

  #[gpui::test]
  async fn commit_requires_message_and_changes(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_component::init(cx));
    let repo = TempRepo::init("review-panel-commit-guards");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (panel, cx) = add_review_panel_window(Some(repo.path.clone()), cx);
    cx.executor().allow_parking();
    await_refresh(&panel, cx).await;

    // Clean tree: commit is a no-op even with a message.
    panel.update_in(cx, |panel, window, cx| {
      panel
        .commit_input
        .update(cx, |input, cx| input.set_value("feat: nothing", window, cx));
    });
    panel.update(cx, |panel, cx| panel.commit(cx));
    panel.read_with(cx, |panel, _| assert!(panel._commit_task.is_none()));

    // Dirty tree but empty message: also a no-op.
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");
    panel.update(cx, |panel, cx| panel.refresh(cx));
    await_refresh(&panel, cx).await;
    panel.update_in(cx, |panel, window, cx| {
      panel
        .commit_input
        .update(cx, |input, cx| input.set_value("   ", window, cx));
    });
    panel.update(cx, |panel, cx| panel.commit(cx));
    panel.read_with(cx, |panel, _| assert!(panel._commit_task.is_none()));
  }
}
