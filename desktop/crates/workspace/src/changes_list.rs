//! Working-tree changes with staging, shared by the Git page and the shell.

use std::path::{Path, PathBuf};

use git::{
  RepoStage, RepoStatusEntry, RepoStatusKind, list_repo_status, restore_file, stage_all,
  stage_file, unstage_all, unstage_file,
};
use gpui::{
  AnyElement, Context, EventEmitter, IntoElement, ParentElement, SharedString, Styled, Task,
  Window, div, img, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Icon, IconName, Sizable as _,
  button::{Button, ButtonGroup},
  h_flex,
  notification::Notification,
  tooltip::Tooltip,
};
use smol::unblock;
use ui::{
  FILE_ICON_SIZE_PX, StatusThemeExt as _, WindowExt as _, file_icon_path_for_path_with_theme,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileStageButtonAction {
  Stage,
  Unstage,
}

pub(crate) fn restore_uses_delete(status: RepoStatusKind) -> bool {
  status == RepoStatusKind::Untracked
}

pub(crate) fn can_unstage(stage: RepoStage) -> bool {
  matches!(stage, RepoStage::Staged | RepoStage::PartiallyStaged)
}

pub(crate) fn can_restore(stage: RepoStage) -> bool {
  matches!(stage, RepoStage::Unstaged | RepoStage::PartiallyStaged)
}

/// In split mode the button follows the section the row sits in, because a
/// partially staged file shows up in both.
pub(crate) fn toggle_stage_action(
  stage: RepoStage,
  split_sections: bool,
  is_staged_section: bool,
) -> FileStageButtonAction {
  if split_sections {
    if is_staged_section {
      FileStageButtonAction::Unstage
    } else {
      FileStageButtonAction::Stage
    }
  } else if can_unstage(stage) {
    FileStageButtonAction::Unstage
  } else {
    FileStageButtonAction::Stage
  }
}

pub(crate) fn status_color(kind: RepoStatusKind, theme: &gpui_component::Theme) -> gpui::Hsla {
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

pub(crate) fn status_tooltip(kind: RepoStatusKind) -> SharedString {
  match kind {
    RepoStatusKind::Modified => "Modified".into(),
    RepoStatusKind::Added => "Added".into(),
    RepoStatusKind::Deleted => "Deleted".into(),
    RepoStatusKind::Renamed => "Renamed".into(),
    RepoStatusKind::TypeChange => "Type changed".into(),
    RepoStatusKind::Untracked => "Untracked".into(),
    RepoStatusKind::Conflicted => "Conflicted".into(),
  }
}

pub(crate) fn stage_style(
  stage: RepoStage,
  theme: &gpui_component::Theme,
) -> (IconName, gpui::Hsla, Option<SharedString>) {
  match stage {
    RepoStage::Staged => (
      IconName::CircleCheck,
      theme.status_green(),
      Some("Staged".into()),
    ),
    RepoStage::PartiallyStaged => (
      IconName::CircleCheck,
      theme.status_orange(),
      Some("Partially staged".into()),
    ),
    RepoStage::Unstaged => (IconName::Minus, theme.muted_foreground, None),
  }
}

/// A staged and an unstaged group; a partially staged file belongs to both.
pub(crate) fn split_entries(
  entries: &[RepoStatusEntry],
) -> (Vec<RepoStatusEntry>, Vec<RepoStatusEntry>) {
  let staged = entries
    .iter()
    .filter(|entry| can_unstage(entry.stage))
    .cloned()
    .collect();
  let unstaged = entries
    .iter()
    .filter(|entry| entry.stage != RepoStage::Staged)
    .cloned()
    .collect();
  (staged, unstaged)
}

pub(crate) enum ChangesListEvent {
  OpenFile {
    path: PathBuf,
  },
  /// A staging action landed: the worktree and the diff moved.
  Changed,
}

pub(crate) struct ChangesList {
  repo_root: Option<PathBuf>,
  entries: Vec<RepoStatusEntry>,
  split_sections: bool,
  opened_path: Option<PathBuf>,
  action_in_flight: bool,
  _action_task: Option<Task<()>>,
}

impl EventEmitter<ChangesListEvent> for ChangesList {}

impl ChangesList {
  pub(crate) fn new(repo_root: Option<PathBuf>, split_sections: bool) -> Self {
    Self {
      repo_root,
      entries: Vec::new(),
      split_sections,
      opened_path: None,
      action_in_flight: false,
      _action_task: None,
    }
  }

  pub(crate) fn set_repo_root(&mut self, repo_root: Option<PathBuf>) {
    self.repo_root = repo_root;
    self.entries.clear();
    self.opened_path = None;
  }

  pub(crate) fn set_entries(&mut self, entries: Vec<RepoStatusEntry>) {
    self.entries = entries;
  }

  #[allow(dead_code)] // consumed when the Git page adopts this list
  pub(crate) fn entries(&self) -> &[RepoStatusEntry] {
    &self.entries
  }

  #[allow(dead_code)] // consumed when the Git page adopts this list
  pub(crate) fn set_split_sections(&mut self, split_sections: bool) {
    self.split_sections = split_sections;
  }

  #[allow(dead_code)] // consumed when the Git page adopts this list
  pub(crate) fn set_opened_path(&mut self, path: Option<PathBuf>) {
    self.opened_path = path;
  }

  fn run<F>(&mut self, label: &'static str, job: F, window: &mut Window, cx: &mut Context<Self>)
  where
    F: FnOnce(&Path) -> anyhow::Result<()> + Send + 'static,
  {
    let Some(repo_root) = self.repo_root.clone() else {
      return;
    };
    if self.action_in_flight {
      return;
    }

    self.action_in_flight = true;
    let window_handle = window.window_handle();
    self._action_task = Some(cx.spawn(async move |this, cx| {
      let result = unblock(move || job(&repo_root)).await;
      let _ = cx.update_window(window_handle, |_, window, cx| {
        let _ = this.update(cx, |this, cx| {
          this.action_in_flight = false;
          match result {
            Ok(()) => cx.emit(ChangesListEvent::Changed),
            Err(error) => {
              window.push_notification(Notification::error(format!("{label} failed: {error}")), cx)
            }
          }
          cx.notify();
        });
      });
    }));
  }

  pub(crate) fn stage_file(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
    self.run(
      "Stage file",
      move |repo_root| stage_file(repo_root, &path),
      window,
      cx,
    );
  }

  pub(crate) fn unstage_file(
    &mut self,
    path: PathBuf,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.run(
      "Unstage file",
      move |repo_root| unstage_file(repo_root, &path),
      window,
      cx,
    );
  }

  pub(crate) fn restore_file(
    &mut self,
    path: PathBuf,
    status: RepoStatusKind,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let delete = restore_uses_delete(status);
    self.run(
      "Discard changes",
      move |repo_root| {
        if delete {
          std::fs::remove_file(repo_root.join(&path)).map_err(anyhow::Error::from)
        } else {
          restore_file(repo_root, &path)
        }
      },
      window,
      cx,
    );
  }

  #[allow(dead_code)] // consumed when the Git page adopts this list
  pub(crate) fn stage_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self.run("Stage all", stage_all, window, cx);
  }

  #[allow(dead_code)] // consumed when the Git page adopts this list
  pub(crate) fn unstage_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self.run("Unstage all", unstage_all, window, cx);
  }

  /// Reloads the worktree status and repaints.
  #[allow(dead_code)] // consumed when the Git page adopts this list
  pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) -> Option<Task<()>> {
    let repo_root = self.repo_root.clone()?;
    Some(cx.spawn(async move |this, cx| {
      let entries = unblock(move || list_repo_status(&repo_root)).await;
      let _ = this.update(cx, |this, cx| {
        if let Ok(entries) = entries {
          this.entries = entries;
        }
        cx.notify();
      });
    }))
  }

  fn render_list(&self, cx: &mut Context<Self>) -> AnyElement {
    if self.entries.is_empty() {
      return div().into_any_element();
    }

    let mut container = div().flex().flex_col().w_full().min_w_0();
    if !self.split_sections {
      let entries = self.entries.clone();
      for (ix, entry) in entries.iter().enumerate() {
        container = container.child(self.render_row(ix, entry, false, cx));
      }
      return container.into_any_element();
    }

    let (staged, unstaged) = split_entries(&self.entries);
    if !staged.is_empty() {
      container = container.child(self.render_section_header("Staged", true, cx));
      for (ix, entry) in staged.iter().enumerate() {
        container = container.child(self.render_row(ix, entry, true, cx));
      }
    }
    if !unstaged.is_empty() {
      container = container.child(self.render_section_header("Changes", false, cx));
      for (ix, entry) in unstaged.iter().enumerate() {
        container = container.child(self.render_row(staged.len() + ix, entry, false, cx));
      }
    }
    container.into_any_element()
  }

  fn render_section_header(
    &self,
    label: &'static str,
    is_staged: bool,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let theme = cx.theme();
    let (icon, icon_color) = if is_staged {
      (IconName::CircleCheck, theme.status_green())
    } else {
      (IconName::Minus, theme.muted_foreground)
    };

    h_flex()
      .items_center()
      .py_1()
      .px_2()
      .gap_2()
      .text_xs()
      .text_color(theme.muted_foreground)
      .child(Icon::new(icon).size_3().text_color(icon_color))
      .child(div().min_w_0().flex_1().child(label))
      .into_any_element()
  }

  fn render_row(
    &self,
    ix: usize,
    entry: &RepoStatusEntry,
    is_staged_section: bool,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let theme = cx.theme().clone();
    let status_kind = entry.status;
    let path = entry.path.clone();
    let is_opened = self.opened_path.as_deref() == Some(path.as_path());

    let (stage_icon, stage_color, stage_tooltip) = stage_style(entry.stage, &theme);
    let stage_element: AnyElement = {
      let icon = Icon::new(stage_icon).size_3().text_color(stage_color);
      match stage_tooltip {
        Some(tooltip) => div()
          .id(("changes-stage-icon", ix))
          .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
          .child(icon)
          .into_any_element(),
        None => div().child(icon).into_any_element(),
      }
    };

    let file_icon = file_icon_path_for_path_with_theme(&path, &theme)
      .map(|icon| {
        img(icon)
          .size(px(FILE_ICON_SIZE_PX))
          .min_size(px(FILE_ICON_SIZE_PX))
          .into_any_element()
      })
      .unwrap_or_else(|| {
        Icon::new(IconName::File)
          .size_3()
          .text_color(theme.sidebar_foreground)
          .into_any_element()
      });

    let (dir, file) = split_path_label(&path);
    let toggle = toggle_stage_action(entry.stage, self.split_sections, is_staged_section);
    let (toggle_icon, toggle_tooltip) = match toggle {
      FileStageButtonAction::Stage => (IconName::Plus, "Stage file"),
      FileStageButtonAction::Unstage => (IconName::Minus, "Unstage file"),
    };
    let restorable = can_restore(entry.stage);
    let status_tip = status_tooltip(status_kind);

    div()
      .id(("changes-row", ix))
      .debug_selector(move || format!("changes-row-{ix}"))
      .group("changes-row")
      .relative()
      .mx_1()
      .px_1()
      .py_1()
      .rounded(px(5.0))
      .cursor_pointer()
      .when(is_opened, |this| {
        this.bg(theme.sidebar_accent.opacity(0.35))
      })
      .hover(|this| this.bg(theme.secondary_hover))
      .on_click(cx.listener({
        let path = path.clone();
        move |_, _, _, cx| {
          cx.emit(ChangesListEvent::OpenFile { path: path.clone() });
        }
      }))
      .child(
        h_flex()
          .items_center()
          .gap_2()
          .child(
            div()
              .id(("changes-status-letter", ix))
              .w(px(15.))
              .min_w(px(15.))
              .text_xs()
              .text_color(status_color(status_kind, &theme))
              .tooltip(move |window, cx| Tooltip::new(status_tip.clone()).build(window, cx))
              .child(status_kind.short_code()),
          )
          .child(stage_element)
          .child(file_icon)
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
      .child(
        div()
          .absolute()
          .right_0()
          .top_1()
          .opacity(0.0)
          .group_hover("changes-row", |this| this.opacity(1.0))
          .bg(theme.sidebar)
          .rounded(theme.radius)
          .child(
            ButtonGroup::new(("changes-row-actions", ix))
              .outline()
              .child(
                Button::new(("changes-stage", ix))
                  .debug_selector(move || format!("changes-stage-{ix}"))
                  .icon(toggle_icon)
                  .xsmall()
                  .tab_stop(false)
                  .tooltip(toggle_tooltip)
                  .on_click(cx.listener({
                    let path = path.clone();
                    move |this, _, window, cx| {
                      cx.stop_propagation();
                      match toggle {
                        FileStageButtonAction::Stage => this.stage_file(path.clone(), window, cx),
                        FileStageButtonAction::Unstage => {
                          this.unstage_file(path.clone(), window, cx)
                        }
                      }
                    }
                  })),
              )
              .when(restorable, |this| {
                this.child(
                  Button::new(("changes-restore", ix))
                    .debug_selector(move || format!("changes-restore-{ix}"))
                    .icon(IconName::Undo)
                    .xsmall()
                    .tab_stop(false)
                    .tooltip("Discard changes")
                    .on_click(cx.listener({
                      let path = path.clone();
                      move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.restore_file(path.clone(), status_kind, window, cx);
                      }
                    })),
                )
              }),
          ),
      )
      .into_any_element()
  }
}

impl gpui::Render for ChangesList {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    self.render_list(cx)
  }
}

/// Splits `src/app/main.rs` into `src/app/` and `main.rs`.
pub(crate) fn split_path_label(path: &Path) -> (String, String) {
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

#[cfg(test)]
mod tests {
  use super::*;

  fn entry(path: &str, stage: RepoStage) -> RepoStatusEntry {
    RepoStatusEntry {
      path: PathBuf::from(path),
      old_path: None,
      status: RepoStatusKind::Modified,
      stage,
    }
  }

  #[test]
  fn a_partially_staged_file_shows_up_in_both_sections() {
    let entries = vec![
      entry("staged.rs", RepoStage::Staged),
      entry("partial.rs", RepoStage::PartiallyStaged),
      entry("unstaged.rs", RepoStage::Unstaged),
    ];

    let (staged, unstaged) = split_entries(&entries);

    assert_eq!(staged.len(), 2);
    assert_eq!(unstaged.len(), 2);
    assert!(staged.iter().any(|e| e.path.ends_with("partial.rs")));
    assert!(unstaged.iter().any(|e| e.path.ends_with("partial.rs")));
  }

  #[test]
  fn the_stage_button_follows_the_section_when_sections_are_split() {
    // Split: the section decides, so a partially staged file can go both ways.
    assert_eq!(
      toggle_stage_action(RepoStage::PartiallyStaged, true, true),
      FileStageButtonAction::Unstage
    );
    assert_eq!(
      toggle_stage_action(RepoStage::PartiallyStaged, true, false),
      FileStageButtonAction::Stage
    );

    // Unified: the file's own stage decides.
    assert_eq!(
      toggle_stage_action(RepoStage::PartiallyStaged, false, false),
      FileStageButtonAction::Unstage
    );
    assert_eq!(
      toggle_stage_action(RepoStage::Unstaged, false, false),
      FileStageButtonAction::Stage
    );
  }

  #[test]
  fn only_untracked_files_are_discarded_by_deleting_them() {
    assert!(restore_uses_delete(RepoStatusKind::Untracked));
    for status in [
      RepoStatusKind::Modified,
      RepoStatusKind::Added,
      RepoStatusKind::Deleted,
      RepoStatusKind::Renamed,
      RepoStatusKind::Conflicted,
    ] {
      assert!(!restore_uses_delete(status));
    }
  }

  #[test]
  fn split_path_label_separates_the_folder_from_the_file() {
    assert_eq!(
      split_path_label(Path::new("crates/workspace/src/git_page.rs")),
      (
        "crates/workspace/src/".to_string(),
        "git_page.rs".to_string()
      )
    );
    assert_eq!(
      split_path_label(Path::new("CHANGELOG.md")),
      (String::new(), "CHANGELOG.md".to_string())
    );
  }

  fn temp_repo(prefix: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .expect("system clock before unix epoch")
      .as_nanos();
    let path = std::env::temp_dir().join(format!("reviu-{prefix}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&path).expect("create temp dir");
    let repo = git2::Repository::init(&path).expect("init repo");
    std::fs::write(path.join("README.md"), "v1\n").expect("write file");
    let mut index = repo.index().expect("index");
    index.add_path(Path::new("README.md")).expect("add");
    index.write().expect("write index");
    let tree = repo
      .find_tree(index.write_tree().expect("write tree"))
      .expect("tree");
    let signature = git2::Signature::now("Test", "test@example.com").expect("signature");
    repo
      .commit(Some("HEAD"), &signature, &signature, "initial", &tree, &[])
      .expect("commit");
    std::fs::write(path.join("README.md"), "v2\n").expect("update file");
    path
  }

  fn add_changes_list_window(
    repo_root: PathBuf,
    cx: &mut gpui::TestAppContext,
  ) -> (gpui::Entity<ChangesList>, &mut gpui::VisualTestContext) {
    cx.update(|cx| gpui_component::init(cx));
    let mut mounted = None;
    let (_root, cx) = cx.add_window_view(|window, cx| {
      let list = cx.new(|_| ChangesList::new(Some(repo_root.clone()), true));
      mounted = Some(list.clone());
      gpui_component::Root::new(list, window, cx)
    });
    (mounted.expect("changes list"), cx)
  }

  #[gpui::test]
  async fn staging_a_file_from_the_row_button_stages_it_in_git(cx: &mut gpui::TestAppContext) {
    let repo_root = temp_repo("changes-list-stage");
    let (list, cx) = add_changes_list_window(repo_root.clone(), cx);
    cx.executor().allow_parking();

    let entries = list_repo_status(&repo_root).expect("status");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].stage, RepoStage::Unstaged);
    list.update(cx, |list, cx| {
      list.set_entries(entries);
      cx.notify();
    });
    cx.run_until_parked();

    let changed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let observer = {
      let changed = changed.clone();
      cx.update(|_, cx| {
        cx.subscribe(&list, move |_, event: &ChangesListEvent, _| {
          if matches!(event, ChangesListEvent::Changed) {
            changed.store(true, std::sync::atomic::Ordering::Relaxed);
          }
        })
      })
    };

    let button = cx
      .debug_bounds("changes-stage-0")
      .expect("stage button bounds");
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    let task = list.update(cx, |list, _| list._action_task.take().expect("stage task"));
    task.await;
    cx.run_until_parked();
    drop(observer);

    let staged = list_repo_status(&repo_root)
      .expect("status")
      .into_iter()
      .filter(|entry| can_unstage(entry.stage))
      .count();
    assert_eq!(staged, 1, "the file should be staged");
    assert!(changed.load(std::sync::atomic::Ordering::Relaxed));

    let _ = std::fs::remove_dir_all(&repo_root);
  }

  #[gpui::test]
  async fn a_row_click_asks_to_open_the_file(cx: &mut gpui::TestAppContext) {
    let repo_root = temp_repo("changes-list-open");
    let (list, cx) = add_changes_list_window(repo_root.clone(), cx);
    cx.executor().allow_parking();

    list.update(cx, |list, cx| {
      list.set_entries(list_repo_status(&repo_root).expect("status"));
      cx.notify();
    });
    cx.run_until_parked();

    let opened = std::sync::Arc::new(std::sync::Mutex::new(None::<PathBuf>));
    let observer = {
      let opened = opened.clone();
      cx.update(|_, cx| {
        cx.subscribe(&list, move |_, event: &ChangesListEvent, _| {
          if let ChangesListEvent::OpenFile { path } = event {
            *opened.lock().unwrap() = Some(path.clone());
          }
        })
      })
    };

    let row = cx.debug_bounds("changes-row-0").expect("row bounds");
    cx.simulate_click(row.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    drop(observer);

    assert_eq!(
      opened.lock().unwrap().clone(),
      Some(PathBuf::from("README.md"))
    );

    let _ = std::fs::remove_dir_all(&repo_root);
  }
}
