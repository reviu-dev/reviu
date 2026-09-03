//! Working-tree changes with staging, for the shell's Changes tab.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use git::{
  RepoStage, RepoStatusEntry, RepoStatusKind, delete_untracked_file, restore_file,
  restore_renamed_file, stage_file, unstage_file,
};
use gpui::{
  AnyElement, App, Context, Entity, EventEmitter, Focusable as _, IntoElement, ParentElement,
  SharedString, Styled, Task, WeakEntity, Window, div, img, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Icon, IconName, IndexPath, Sizable as _,
  button::{Button, ButtonGroup},
  h_flex,
  list::{List, ListDelegate, ListEvent, ListItem, ListState},
  notification::Notification,
  tooltip::Tooltip,
};
use ui::{
  ConfirmDialog, FILE_ICON_SIZE_PX, SelectableRowStyle, StatusThemeExt as _, WindowExt as _,
  file_icon_path_for_path_with_theme, selectable_list_item,
};

use crate::open_intent::OpenIntent;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileStageButtonAction {
  Stage,
  Unstage,
}

pub(crate) fn restore_uses_delete(status: RepoStatusKind) -> bool {
  status == RepoStatusKind::Untracked
}

pub(crate) fn can_stage(stage: RepoStage) -> bool {
  stage == RepoStage::Unstaged
}

pub(crate) fn can_unstage(stage: RepoStage) -> bool {
  matches!(stage, RepoStage::Staged | RepoStage::PartiallyStaged)
}

pub(crate) fn stage_requires_confirmation(status: RepoStatusKind) -> bool {
  status == RepoStatusKind::Conflicted
}

/// Only worth asking while the file still carries conflict markers: once they
/// are resolved, staging is the normal way to mark the conflict done.
pub(crate) fn should_confirm_stage(
  status: RepoStatusKind,
  has_unresolved_conflict_markers: bool,
) -> bool {
  stage_requires_confirmation(status) && has_unresolved_conflict_markers
}

pub(crate) fn all_entries_staged(entries: &[RepoStatusEntry]) -> bool {
  !entries.is_empty() && entries.iter().all(|entry| entry.stage == RepoStage::Staged)
}

pub(crate) fn has_conflicted_entries(entries: &[RepoStatusEntry]) -> bool {
  entries
    .iter()
    .any(|entry| entry.status == RepoStatusKind::Conflicted)
}

pub(crate) fn has_untracked_entries(entries: &[RepoStatusEntry]) -> bool {
  entries
    .iter()
    .any(|entry| entry.status == RepoStatusKind::Untracked)
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
    RepoStatusKind::Modified => theme.status_amber(),
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

fn status_uses_warning_icon(kind: RepoStatusKind) -> bool {
  kind == RepoStatusKind::Conflicted
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

pub(crate) enum ChangesListEvent {
  OpenFile {
    path: PathBuf,
    intent: OpenIntent,
  },
  /// A staging action landed: the worktree and the diff moved.
  Changed,
}

struct ChangesSection {
  label: SharedString,
  is_staged: bool,
  rows: Vec<Rc<RepoStatusEntry>>,
}

pub(crate) struct ChangesRowsDelegate {
  rows: Vec<Rc<RepoStatusEntry>>,
  sections: Vec<ChangesSection>,
  split_sections: bool,
  selected_index: Option<IndexPath>,
  hovered_index: Option<IndexPath>,
  opened_path: Option<PathBuf>,
  list: WeakEntity<ChangesList>,
}

impl ChangesRowsDelegate {
  fn new(list: WeakEntity<ChangesList>, split_sections: bool) -> Self {
    Self {
      rows: Vec::new(),
      sections: Vec::new(),
      split_sections,
      selected_index: None,
      hovered_index: None,
      opened_path: None,
      list,
    }
  }

  fn set_rows(&mut self, entries: Vec<RepoStatusEntry>) {
    self.rows = entries.into_iter().map(Rc::new).collect();
    self.rebuild_sections();
  }

  fn index_for_path(&self, path: &Path) -> Option<IndexPath> {
    for (section_ix, section) in self.sections.iter().enumerate() {
      for (row_ix, row) in section.rows.iter().enumerate() {
        if row.path == path {
          return Some(IndexPath {
            section: section_ix,
            row: row_ix,
            column: 0,
          });
        }
      }
    }
    None
  }

  fn rebuild_sections(&mut self) {
    if !self.split_sections {
      self.sections = vec![ChangesSection {
        label: "".into(),
        is_staged: false,
        rows: self.rows.clone(),
      }];
      return;
    }

    let mut staged = Vec::new();
    let mut unstaged = Vec::new();
    for row in &self.rows {
      match row.stage {
        RepoStage::Staged => staged.push(row.clone()),
        RepoStage::Unstaged => unstaged.push(row.clone()),
        RepoStage::PartiallyStaged => {
          staged.push(row.clone());
          unstaged.push(row.clone());
        }
      }
    }

    let mut sections = Vec::new();
    if !staged.is_empty() {
      sections.push(ChangesSection {
        label: format!("Staged Changes ({})", staged.len()).into(),
        is_staged: true,
        rows: staged,
      });
    }
    if !unstaged.is_empty() {
      sections.push(ChangesSection {
        label: format!("Changes ({})", unstaged.len()).into(),
        is_staged: false,
        rows: unstaged,
      });
    }
    self.sections = sections;
  }

  fn row_at(&self, ix: IndexPath) -> Option<Rc<RepoStatusEntry>> {
    self
      .sections
      .get(ix.section)
      .and_then(|section| section.rows.get(ix.row).cloned())
  }

  fn shows_row_actions(&self, ix: IndexPath) -> bool {
    row_actions_visible(self.selected_index, self.hovered_index, ix, cfg!(test))
  }
}

fn row_actions_visible(
  _selected_index: Option<IndexPath>,
  hovered_index: Option<IndexPath>,
  ix: IndexPath,
  force_visible: bool,
) -> bool {
  force_visible || hovered_index == Some(ix)
}

impl ListDelegate for ChangesRowsDelegate {
  type Item = ListItem;

  fn sections_count(&self, _cx: &App) -> usize {
    self.sections.len()
  }

  fn items_count(&self, section: usize, _cx: &App) -> usize {
    self.sections.get(section).map_or(0, |s| s.rows.len())
  }

  fn render_section_header(
    &mut self,
    section: usize,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> Option<impl IntoElement> {
    if !self.split_sections {
      return None;
    }
    let section = self.sections.get(section)?;
    let theme = cx.theme();
    let (icon, icon_color) = if section.is_staged {
      (IconName::CircleCheck, theme.status_green())
    } else {
      (IconName::Minus, theme.muted_foreground)
    };

    Some(
      h_flex()
        .items_center()
        .py_1()
        .px_2()
        .gap_2()
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(Icon::new(icon).size_3().text_color(icon_color))
        .child(div().min_w_0().flex_1().child(section.label.clone())),
    )
  }

  fn render_item(
    &mut self,
    ix: IndexPath,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> Option<Self::Item> {
    let theme = cx.theme().clone();
    let entry = self.row_at(ix)?;
    let mut base = selectable_list_item(
      ix,
      self
        .selected_index
        .map(|selected| selected.eq_row(ix))
        .unwrap_or(false),
      SelectableRowStyle::Inset,
      &theme,
    );
    if self.opened_path.as_deref() == Some(entry.path.as_path()) {
      base = base.bg(theme.sidebar_accent.opacity(0.35));
    }
    // A partially staged file is painted twice, so the section is part of the id.
    let list_state = cx.entity().downgrade();
    base = base
      .debug_selector(move || format!("changes-row-{}-{}", ix.section, ix.row))
      .on_hover(move |is_hovered, _, cx| {
        let _ = list_state.update(cx, |state, cx| {
          let delegate = state.delegate_mut();
          let hovered_index = if *is_hovered {
            Some(ix)
          } else {
            delegate.hovered_index.filter(|hovered| hovered != &ix)
          };
          if delegate.hovered_index != hovered_index {
            delegate.hovered_index = hovered_index;
            cx.notify();
          }
        });
      });

    let status_kind = entry.status;
    let path = entry.path.clone();
    let show_row_actions = self.shows_row_actions(ix);
    let (stage_icon, stage_color, stage_tooltip) = stage_style(entry.stage, &theme);
    let stage_icon = Icon::new(stage_icon).size_3().text_color(stage_color);
    let stage_element: AnyElement = if show_row_actions {
      match stage_tooltip {
        Some(tooltip) => div()
          .id(("changes-stage-icon", ix.row))
          .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
          .child(stage_icon)
          .into_any_element(),
        None => div().child(stage_icon).into_any_element(),
      }
    } else {
      div().child(stage_icon).into_any_element()
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

    let (parent_path, file_name) = split_path_label(&path);
    let status_element: AnyElement = {
      let status_color = status_color(status_kind, &theme);
      let status_content = if status_uses_warning_icon(status_kind) {
        Icon::new(IconName::TriangleAlert)
          .size_3()
          .text_color(status_color)
          .into_any_element()
      } else {
        div()
          .text_xs()
          .text_color(status_color)
          .child(status_kind.short_code())
          .into_any_element()
      };
      let status = div()
        .id(("changes-status-letter", ix.row))
        .w(px(15.))
        .min_w(px(15.))
        .flex()
        .items_center()
        .child(status_content);
      if show_row_actions {
        let status_tip = status_tooltip(status_kind);
        status
          .tooltip(move |window, cx| Tooltip::new(status_tip.clone()).build(window, cx))
          .into_any_element()
      } else {
        status.into_any_element()
      }
    };
    let row_actions = show_row_actions.then(|| {
      let is_staged_section = self
        .sections
        .get(ix.section)
        .map(|section| section.is_staged)
        .unwrap_or(false);
      let toggle = toggle_stage_action(entry.stage, self.split_sections, is_staged_section);
      let (toggle_icon, toggle_tooltip) = match toggle {
        FileStageButtonAction::Stage => (IconName::Plus, "Stage file"),
        FileStageButtonAction::Unstage => (IconName::Minus, "Unstage file"),
      };
      let restorable = can_restore(entry.stage);
      let list = self.list.clone();
      let path = path.clone();
      div()
        .absolute()
        .right_0()
        .bg(theme.sidebar)
        .rounded(theme.radius)
        .child(
          ButtonGroup::new(("changes-row-actions", ix.row))
            .outline()
            .child(
              Button::new(("changes-stage", ix.row))
                .debug_selector(move || format!("changes-stage-{}-{}", ix.section, ix.row))
                .icon(toggle_icon)
                .xsmall()
                .tab_stop(false)
                .tooltip(toggle_tooltip)
                .on_click({
                  let list = list.clone();
                  let path = path.clone();
                  move |_, window, cx| {
                    cx.stop_propagation();
                    let _ = list.update(cx, |list, cx| match toggle {
                      FileStageButtonAction::Stage => {
                        list.stage_file_with_confirmation(path.clone(), status_kind, window, cx)
                      }
                      FileStageButtonAction::Unstage => list.unstage_file(path.clone(), window, cx),
                    });
                  }
                }),
            )
            .when(restorable, |this| {
              this.child(
                Button::new(("changes-restore", ix.row))
                  .debug_selector(move || format!("changes-restore-{}-{}", ix.section, ix.row))
                  .icon(IconName::Undo)
                  .xsmall()
                  .tab_stop(false)
                  .tooltip("Discard changes")
                  .on_click({
                    let list = list.clone();
                    let path = path.clone();
                    move |_, window, cx| {
                      cx.stop_propagation();
                      let _ = list.update(cx, |list, cx| {
                        list.confirm_restore_file(path.clone(), status_kind, window, cx);
                      });
                    }
                  }),
              )
            }),
        )
        .into_any_element()
    });

    Some(
      base.px_2().py_1().child(
        h_flex()
          .group("changes-row")
          .size_full()
          .items_center()
          .relative()
          .gap_2()
          .child(
            h_flex()
              .items_center()
              .min_w_0()
              .gap_2()
              .child(status_element)
              .child(stage_element)
              .child(file_icon)
              .child(
                h_flex()
                  .flex_1()
                  .min_w(px(0.0))
                  .overflow_hidden()
                  .text_sm()
                  .whitespace_nowrap()
                  .gap_1()
                  .child(
                    div()
                      .debug_selector(move || {
                        format!("changes-file-name-{}-{}", ix.section, ix.row)
                      })
                      .flex_shrink_0()
                      .text_color(theme.foreground)
                      .child(file_name),
                  )
                  .when(!parent_path.is_empty(), |this| {
                    this.child(
                      div()
                        .debug_selector(move || {
                          format!("changes-file-path-{}-{}", ix.section, ix.row)
                        })
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis_start()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(parent_path),
                    )
                  }),
              ),
          )
          .children(row_actions),
      ),
    )
  }

  fn set_selected_index(
    &mut self,
    ix: Option<IndexPath>,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) {
    self.selected_index = ix;
    cx.notify();
  }
}

pub(crate) struct ChangesList {
  repo_root: Option<PathBuf>,
  entries: Vec<RepoStatusEntry>,
  list: Entity<ListState<ChangesRowsDelegate>>,
  action_in_flight: bool,
  /// Set by the consumer showing the file: staging a conflict only asks while
  /// markers are still there.
  open_file_has_conflict_markers: bool,
  pub(crate) _action_task: Option<Task<()>>,
}

impl EventEmitter<ChangesListEvent> for ChangesList {}

impl ChangesList {
  pub(crate) fn new(
    repo_root: Option<PathBuf>,
    split_sections: bool,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    let weak = cx.entity().downgrade();
    let list = cx.new(|cx| {
      ListState::new(ChangesRowsDelegate::new(weak, split_sections), window, cx)
        .reset_on_cancel(false)
    });

    // The file list is the first stop of the Changes tab, ahead of the commit box.
    let _ = list.read(cx).focus_handle(cx).tab_stop(true).tab_index(0);

    // Walking the list shows each file; a click or Enter hands the editor the
    // keyboard.
    cx.subscribe(&list, |_, state, event: &ListEvent, cx| {
      let (ix, intent) = match event {
        ListEvent::Select(ix) => (*ix, OpenIntent::Browse),
        ListEvent::Confirm(ix) => (*ix, OpenIntent::Open),
        _ => return,
      };
      if let Some(entry) = state.read(cx).delegate().row_at(ix) {
        cx.emit(ChangesListEvent::OpenFile {
          path: entry.path.clone(),
          intent,
        });
      }
    })
    .detach();

    Self {
      repo_root,
      entries: Vec::new(),
      list,
      action_in_flight: false,
      open_file_has_conflict_markers: false,
      _action_task: None,
    }
  }

  pub(crate) fn set_open_file_has_conflict_markers(&mut self, has_markers: bool) {
    self.open_file_has_conflict_markers = has_markers;
  }

  pub(crate) fn set_repo_root(&mut self, repo_root: Option<PathBuf>, cx: &mut Context<Self>) {
    self.repo_root = repo_root;
    self.set_entries(Vec::new(), cx);
    self.set_opened_path(None, cx);
  }

  pub(crate) fn set_entries(&mut self, entries: Vec<RepoStatusEntry>, cx: &mut Context<Self>) {
    self.entries = entries.clone();
    self.list.update(cx, |state, cx| {
      state.delegate_mut().set_rows(entries);
      cx.notify();
    });
  }

  #[allow(dead_code)] // read by the panel and list tests
  pub(crate) fn entries(&self) -> &[RepoStatusEntry] {
    &self.entries
  }

  pub(crate) fn set_split_sections(&mut self, split_sections: bool, cx: &mut Context<Self>) {
    self.list.update(cx, |state, cx| {
      let delegate = state.delegate_mut();
      delegate.split_sections = split_sections;
      delegate.rebuild_sections();
      cx.notify();
    });
  }

  pub(crate) fn select_path(&mut self, path: Option<&Path>, cx: &mut Context<Self>) {
    let index = path.and_then(|path| self.list.read(cx).delegate().index_for_path(path));
    self.list.update(cx, |state, cx| {
      state.delegate_mut().selected_index = index;
      cx.notify();
    });
  }

  #[cfg(test)]
  pub(crate) fn has_selection(&self, cx: &App) -> bool {
    self.list.read(cx).delegate().selected_index.is_some()
  }

  pub(crate) fn is_focused(&self, window: &Window, cx: &App) -> bool {
    self
      .list
      .read(cx)
      .focus_handle(cx)
      .contains_focused(window, cx)
  }

  pub(crate) fn set_opened_path(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
    self.list.update(cx, |state, cx| {
      state.delegate_mut().opened_path = path;
      cx.notify();
    });
  }

  pub(crate) fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
    let handle = self.list.read(cx).focus_handle(cx);
    window.focus(&handle, cx);
  }

  fn run<F>(&mut self, label: &'static str, job: F, window: &mut Window, cx: &mut Context<Self>)
  where
    F: FnOnce(&Path) -> anyhow::Result<()> + Send + 'static,
  {
    self.run_action(label, false, job, window, cx);
  }

  /// `destructive` actions delete the selected row, so the highlight moves to
  /// the top instead of pointing at nothing.
  fn run_action<F>(
    &mut self,
    label: &'static str,
    destructive: bool,
    job: F,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) where
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
      let result = cx.background_spawn(async move { job(&repo_root) }).await;
      let _ = cx.update_window(window_handle, |_, window, cx| {
        let _ = this.update(cx, |this, cx| {
          this.action_in_flight = false;
          match result {
            Ok(()) => {
              if destructive {
                this.select_first(cx);
              }
              cx.emit(ChangesListEvent::Changed);
            }
            Err(error) => {
              window.push_notification(Notification::error(format!("{label} failed: {error}")), cx)
            }
          }
          cx.notify();
        });
      });
    }));
  }

  /// Staging a file whose conflict markers are still there asks first: it would
  /// mark the conflict resolved.
  pub(crate) fn stage_file_with_confirmation(
    &mut self,
    path: PathBuf,
    status: RepoStatusKind,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if !should_confirm_stage(status, self.open_file_has_conflict_markers) {
      self.stage_file(path, window, cx);
      return;
    }

    let file_label = path.to_string_lossy().replace(['\n', '\r'], "");
    let title: SharedString = "Mark conflicts as resolved?".into();
    let message: SharedString =
      format!("Stage {file_label} and mark its merge conflicts as resolved?").into();
    let view = cx.entity();

    window.open_alert_dialog(cx, move |alert, _, _| {
      let view = view.clone();
      let path = path.clone();
      ConfirmDialog::new(title.clone(), div().child(message.clone()))
        .confirm_text("Stage")
        .cancel_text("Cancel")
        .on_confirm(move |_, window, cx| {
          let path = path.clone();
          view.update(cx, |view, cx| view.stage_file(path, window, cx));
          true
        })
        .build(alert)
    });
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

  /// Discarding destroys work, so it always asks first.
  pub(crate) fn confirm_restore_file(
    &mut self,
    path: PathBuf,
    status: RepoStatusKind,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let file_label = path.to_string_lossy().replace(['\n', '\r'], "");
    let (title, message, confirm_text) = if status == RepoStatusKind::Untracked {
      (
        "Delete file?",
        format!("Delete {file_label} from disk?"),
        "Delete",
      )
    } else {
      (
        "Restore file?",
        format!("Discard changes in {file_label}?"),
        "Restore",
      )
    };

    let title: SharedString = title.into();
    let message: SharedString = message.into();
    let confirm_text: SharedString = confirm_text.into();
    let view = cx.entity();

    window.open_alert_dialog(cx, move |alert, _, _| {
      let view = view.clone();
      let path = path.clone();
      ConfirmDialog::new(title.clone(), div().child(message.clone()))
        .confirm_text(confirm_text.clone())
        .cancel_text("Cancel")
        .destructive()
        .on_confirm(move |_, window, cx| {
          let path = path.clone();
          view.update(cx, |view, cx| {
            view.restore_file(path, status, window, cx);
          });
          true
        })
        .build(alert)
    });
  }

  pub(crate) fn restore_file(
    &mut self,
    path: PathBuf,
    status: RepoStatusKind,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let old_path = self
      .entries
      .iter()
      .find(|entry| entry.path == path)
      .and_then(|entry| entry.old_path.clone());
    self.run_action(
      "Discard changes",
      true,
      move |repo_root| restore_entry(repo_root, &path, old_path.as_deref(), status),
      window,
      cx,
    );
  }

  /// Discards every change in the worktree, so it always asks first.
  pub(crate) fn confirm_restore_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.repo_root.is_none() || self.entries.is_empty() {
      return;
    }

    let title: SharedString = "Restore all files?".into();
    let message: SharedString = if has_untracked_entries(&self.entries) {
      "Discard all tracked changes and delete all untracked files?".into()
    } else {
      "Discard all changes in the repository?".into()
    };
    let view = cx.entity();

    window.open_alert_dialog(cx, move |alert, _, _| {
      let view = view.clone();
      ConfirmDialog::new(title.clone(), div().child(message.clone()))
        .confirm_text("Restore all")
        .cancel_text("Cancel")
        .destructive()
        .on_confirm(move |_, window, cx| {
          view.update(cx, |view, cx| view.restore_all(window, cx));
          true
        })
        .build(alert)
    });
  }

  pub(crate) fn restore_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let entries = self.entries.clone();
    self.run_action(
      "Restore all",
      true,
      move |repo_root| {
        let mut first_error = None;
        for entry in entries {
          let result = restore_entry(
            repo_root,
            &entry.path,
            entry.old_path.as_deref(),
            entry.status,
          );
          if let Err(error) = result
            && first_error.is_none()
          {
            first_error = Some(error);
          }
        }
        match first_error {
          Some(error) => Err(error),
          None => Ok(()),
        }
      },
      window,
      cx,
    );
  }

  /// Keeps the highlight on a row that still exists after a destructive action.
  pub(crate) fn select_first(&mut self, cx: &mut Context<Self>) {
    let index = (!self.entries.is_empty()).then(IndexPath::default);
    self.list.update(cx, |state, cx| {
      state.delegate_mut().selected_index = index;
      cx.notify();
    });
  }
}

impl gpui::Render for ChangesList {
  fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
    List::new(&self.list).w_full().min_h_0()
  }
}

/// Discarding one entry: delete an untracked file, put a rename back, or
/// restore the committed content.
fn restore_entry(
  repo_root: &Path,
  path: &Path,
  old_path: Option<&Path>,
  status: RepoStatusKind,
) -> anyhow::Result<()> {
  if restore_uses_delete(status) {
    return delete_untracked_file(repo_root, path);
  }
  match (status, old_path) {
    (RepoStatusKind::Renamed, Some(old_path)) => restore_renamed_file(repo_root, old_path, path),
    _ => restore_file(repo_root, path),
  }
}

/// Splits `src/app/main.rs` into `src/app` and `main.rs`.
pub(crate) fn split_path_label(path: &Path) -> (String, String) {
  let file_name = path
    .file_name()
    .map(|name| name.to_string_lossy().into_owned())
    .unwrap_or_else(|| path.to_string_lossy().into_owned());
  let parent_path = path
    .parent()
    .filter(|parent| !parent.as_os_str().is_empty())
    .map(|parent| parent.to_string_lossy().into_owned())
    .unwrap_or_default();
  (parent_path, file_name)
}

#[cfg(test)]
mod tests {
  use super::*;
  use git::list_repo_status;
  use git::stage_all;

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
  fn row_actions_only_render_for_the_active_row() {
    let first = IndexPath {
      section: 0,
      row: 0,
      column: 0,
    };
    let second = IndexPath {
      section: 0,
      row: 1,
      column: 0,
    };

    assert!(!row_actions_visible(None, None, first, false));
    assert!(!row_actions_visible(Some(first), None, first, false));
    assert!(row_actions_visible(None, Some(first), first, false));
    assert!(!row_actions_visible(
      Some(second),
      Some(second),
      first,
      false
    ));
    assert!(!row_actions_visible(
      Some(first),
      Some(second),
      first,
      false
    ));
    assert!(row_actions_visible(None, None, first, true));
  }

  #[test]
  fn only_conflicted_status_uses_a_warning_icon() {
    assert!(status_uses_warning_icon(RepoStatusKind::Conflicted));
    assert!(!status_uses_warning_icon(RepoStatusKind::Untracked));
  }

  #[test]
  fn split_path_label_separates_the_parent_path_from_the_file() {
    assert_eq!(
      split_path_label(Path::new("crates/workspace/src/session_page.rs")),
      (
        "crates/workspace/src".to_string(),
        "session_page.rs".to_string()
      )
    );
    assert_eq!(
      split_path_label(Path::new("CHANGELOG.md")),
      (String::new(), "CHANGELOG.md".to_string())
    );
  }

  #[gpui::test]
  async fn rows_show_file_name_before_parent_path(cx: &mut gpui::TestAppContext) {
    let (list, cx) = add_changes_list_window(std::env::temp_dir(), cx);
    list.update(cx, |list, cx| {
      list.set_entries(
        vec![RepoStatusEntry {
          path: PathBuf::from("src/components/Avatar.vue"),
          old_path: None,
          status: RepoStatusKind::Modified,
          stage: RepoStage::Unstaged,
        }],
        cx,
      );
    });
    cx.run_until_parked();

    let file_name = cx
      .debug_bounds("changes-file-name-0-0")
      .expect("file name label bounds");
    let parent_path = cx
      .debug_bounds("changes-file-path-0-0")
      .expect("parent path label bounds");
    assert!(
      f32::from(file_name.left()) < f32::from(parent_path.left()),
      "the file name should be shown before its muted parent path"
    );
  }

  fn temp_repo(prefix: &str) -> PathBuf {
    let path = crate::test_support::temp_path(prefix);
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
    cx.update(gpui_component::init);
    let mut mounted = None;
    let (_root, cx) = cx.add_window_view(|window, cx| {
      let list = cx.new(|cx| ChangesList::new(Some(repo_root.clone()), true, window, cx));
      mounted = Some(list.clone());
      gpui_component::Root::new(list, window, cx)
    });
    (mounted.expect("changes list"), cx)
  }

  #[gpui::test]
  async fn staging_a_file_from_the_row_button_stages_it_in_git(cx: &mut gpui::TestAppContext) {
    let repo_root = temp_repo("changes-list-stage");
    let (list, cx) = add_changes_list_window(repo_root.clone(), cx);

    let entries = list_repo_status(&repo_root).expect("status");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].stage, RepoStage::Unstaged);
    list.update(cx, |list, cx| list.set_entries(entries, cx));
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
      .debug_bounds("changes-stage-0-0")
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

    list.update(cx, |list, cx| {
      list.set_entries(list_repo_status(&repo_root).expect("status"), cx)
    });
    cx.run_until_parked();

    let opened = std::sync::Arc::new(std::sync::Mutex::new(None::<PathBuf>));
    let observer = {
      let opened = opened.clone();
      cx.update(|_, cx| {
        cx.subscribe(&list, move |_, event: &ChangesListEvent, _| {
          if let ChangesListEvent::OpenFile { path, .. } = event {
            *opened.lock().unwrap() = Some(path.clone());
          }
        })
      })
    };

    let row = cx.debug_bounds("changes-row-0-0").expect("row bounds");
    cx.simulate_click(row.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    drop(observer);

    assert_eq!(
      opened.lock().unwrap().clone(),
      Some(PathBuf::from("README.md"))
    );

    let _ = std::fs::remove_dir_all(&repo_root);
  }

  async fn run_action(
    list: &gpui::Entity<ChangesList>,
    cx: &mut gpui::VisualTestContext,
    selector: &'static str,
  ) {
    let button = cx.debug_bounds(selector).expect("button bounds");
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    let task = list.update(cx, |list, _| list._action_task.take().expect("action task"));
    task.await;
    cx.run_until_parked();
  }

  fn set_entries_from_disk(
    list: &gpui::Entity<ChangesList>,
    cx: &mut gpui::VisualTestContext,
    repo_root: &Path,
  ) {
    let entries = list_repo_status(repo_root).expect("status");
    list.update(cx, |list, cx| list.set_entries(entries, cx));
    cx.run_until_parked();
  }

  #[gpui::test]
  async fn row_stage_button_does_not_open_the_file(cx: &mut gpui::TestAppContext) {
    let repo_root = temp_repo("changes-list-stage-no-open");
    let (list, cx) = add_changes_list_window(repo_root.clone(), cx);
    set_entries_from_disk(&list, cx, &repo_root);

    let opened = std::sync::Arc::new(std::sync::Mutex::new(None::<PathBuf>));
    let observer = {
      let opened = opened.clone();
      cx.update(|_, cx| {
        cx.subscribe(&list, move |_, event: &ChangesListEvent, _| {
          if let ChangesListEvent::OpenFile { path, .. } = event {
            *opened.lock().unwrap() = Some(path.clone());
          }
        })
      })
    };

    run_action(&list, cx, "changes-stage-0-0").await;
    drop(observer);

    assert_eq!(opened.lock().unwrap().clone(), None);
    let _ = std::fs::remove_dir_all(&repo_root);
  }

  #[gpui::test]
  async fn row_restore_button_does_not_open_the_file(cx: &mut gpui::TestAppContext) {
    let repo_root = temp_repo("changes-list-restore-no-open");
    let (list, cx) = add_changes_list_window(repo_root.clone(), cx);
    set_entries_from_disk(&list, cx, &repo_root);

    let opened = std::sync::Arc::new(std::sync::Mutex::new(None::<PathBuf>));
    let observer = {
      let opened = opened.clone();
      cx.update(|_, cx| {
        cx.subscribe(&list, move |_, event: &ChangesListEvent, _| {
          if let ChangesListEvent::OpenFile { path, .. } = event {
            *opened.lock().unwrap() = Some(path.clone());
          }
        })
      })
    };

    let button = cx
      .debug_bounds("changes-restore-0-0")
      .expect("restore button bounds");
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    drop(observer);

    assert_eq!(opened.lock().unwrap().clone(), None);
    assert!(cx.update(|window, cx| window.has_active_dialog(cx)));
    cx.update(|window, cx| window.close_dialog(cx));
    let _ = std::fs::remove_dir_all(&repo_root);
  }

  #[gpui::test]
  async fn unstaging_a_file_from_the_row_button_unstages_it(cx: &mut gpui::TestAppContext) {
    let repo_root = temp_repo("changes-list-unstage");
    stage_all(&repo_root).expect("stage all");

    let (list, cx) = add_changes_list_window(repo_root.clone(), cx);
    set_entries_from_disk(&list, cx, &repo_root);

    run_action(&list, cx, "changes-stage-0-0").await;

    let entries = list_repo_status(&repo_root).expect("status");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].stage, RepoStage::Unstaged);

    let _ = std::fs::remove_dir_all(&repo_root);
  }

  #[gpui::test]
  async fn discarding_asks_before_touching_the_file(cx: &mut gpui::TestAppContext) {
    let repo_root = temp_repo("changes-list-discard-confirm");

    let (list, cx) = add_changes_list_window(repo_root.clone(), cx);
    set_entries_from_disk(&list, cx, &repo_root);

    let button = cx
      .debug_bounds("changes-restore-0-0")
      .expect("discard button bounds");
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    cx.run_until_parked();

    assert!(
      cx.update(|window, cx| window.has_active_dialog(cx)),
      "discarding must ask first"
    );
    list.read_with(cx, |list, _| assert!(list._action_task.is_none()));
    assert_eq!(
      std::fs::read_to_string(repo_root.join("README.md")).expect("read file"),
      "v2\n",
      "nothing should change until the dialog is confirmed"
    );

    cx.update(|window, cx| window.close_dialog(cx));
    cx.run_until_parked();
    assert_eq!(
      std::fs::read_to_string(repo_root.join("README.md")).expect("read file"),
      "v2\n",
      "closing the confirmation must not discard the file"
    );

    let _ = std::fs::remove_dir_all(&repo_root);
  }

  #[gpui::test]
  async fn discarding_a_modified_file_restores_its_content(cx: &mut gpui::TestAppContext) {
    let repo_root = temp_repo("changes-list-discard-modified");

    let (list, cx) = add_changes_list_window(repo_root.clone(), cx);
    set_entries_from_disk(&list, cx, &repo_root);

    // What the confirmation dialog runs.
    let task = list.update_in(cx, |list, window, cx| {
      list.restore_file(
        PathBuf::from("README.md"),
        RepoStatusKind::Modified,
        window,
        cx,
      );
      list._action_task.take().expect("restore task")
    });
    task.await;
    cx.run_until_parked();

    assert_eq!(
      std::fs::read_to_string(repo_root.join("README.md")).expect("read file"),
      "v1\n",
      "the committed content should be back"
    );
    assert!(list_repo_status(&repo_root).expect("status").is_empty());

    let _ = std::fs::remove_dir_all(&repo_root);
  }

  #[gpui::test]
  async fn discarding_an_untracked_file_deletes_it(cx: &mut gpui::TestAppContext) {
    let repo_root = temp_repo("changes-list-discard-untracked");
    std::fs::write(repo_root.join("README.md"), "v1\n").expect("reset tracked file");
    std::fs::write(repo_root.join("scratch.txt"), "temp\n").expect("write untracked file");

    let (list, cx) = add_changes_list_window(repo_root.clone(), cx);
    set_entries_from_disk(&list, cx, &repo_root);

    let task = list.update_in(cx, |list, window, cx| {
      list.restore_file(
        PathBuf::from("scratch.txt"),
        RepoStatusKind::Untracked,
        window,
        cx,
      );
      list._action_task.take().expect("restore task")
    });
    task.await;
    cx.run_until_parked();

    assert!(
      !repo_root.join("scratch.txt").exists(),
      "an untracked file is discarded by deleting it"
    );

    let _ = std::fs::remove_dir_all(&repo_root);
  }

  #[gpui::test]
  async fn a_partially_staged_file_is_painted_in_both_sections(cx: &mut gpui::TestAppContext) {
    let repo_root = temp_repo("changes-list-partial");
    // Stage the current content, then modify it again: staged and unstaged at once.
    stage_all(&repo_root).expect("stage all");
    std::fs::write(repo_root.join("README.md"), "v3\n").expect("update file");

    let (list, cx) = add_changes_list_window(repo_root.clone(), cx);
    set_entries_from_disk(&list, cx, &repo_root);

    list.read_with(cx, |list, _| {
      assert_eq!(list.entries().len(), 1);
      assert_eq!(list.entries()[0].stage, RepoStage::PartiallyStaged);
    });

    assert!(cx.debug_bounds("changes-row-0-0").is_some());
    assert!(
      cx.debug_bounds("changes-row-1-0").is_some(),
      "the same file should be painted in the staged and the unstaged section"
    );

    let _ = std::fs::remove_dir_all(&repo_root);
  }

  #[gpui::test]
  async fn the_keyboard_walks_the_list_and_opens_a_file(cx: &mut gpui::TestAppContext) {
    let repo_root = temp_repo("changes-list-keyboard");
    std::fs::write(repo_root.join("other.txt"), "new\n").expect("write second file");

    let (list, cx) = add_changes_list_window(repo_root.clone(), cx);
    set_entries_from_disk(&list, cx, &repo_root);

    let opened = std::sync::Arc::new(std::sync::Mutex::new(Vec::<PathBuf>::new()));
    let observer = {
      let opened = opened.clone();
      cx.update(|_, cx| {
        cx.subscribe(&list, move |_, event: &ChangesListEvent, _| {
          if let ChangesListEvent::OpenFile { path, .. } = event {
            opened.lock().unwrap().push(path.clone());
          }
        })
      })
    };

    list.update_in(cx, |list, window, cx| list.focus(window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("down");
    cx.run_until_parked();
    drop(observer);

    // Walking with the keyboard selects a row, which opens it like a click does.
    assert_eq!(
      opened.lock().unwrap().len(),
      1,
      "moving the selection should open the highlighted file"
    );

    let _ = std::fs::remove_dir_all(&repo_root);
  }

  #[gpui::test]
  async fn pressing_enter_opens_the_selected_file(cx: &mut gpui::TestAppContext) {
    let repo_root = temp_repo("changes-list-enter");

    let (list, cx) = add_changes_list_window(repo_root.clone(), cx);
    set_entries_from_disk(&list, cx, &repo_root);

    let opened = std::sync::Arc::new(std::sync::Mutex::new(Vec::<PathBuf>::new()));
    let observer = {
      let opened = opened.clone();
      cx.update(|_, cx| {
        cx.subscribe(&list, move |_, event: &ChangesListEvent, _| {
          if let ChangesListEvent::OpenFile { path, .. } = event {
            opened.lock().unwrap().push(path.clone());
          }
        })
      })
    };

    list.update_in(cx, |list, window, cx| list.focus(window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("down");
    cx.run_until_parked();
    opened.lock().unwrap().clear();

    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    drop(observer);

    assert_eq!(
      opened.lock().unwrap().clone(),
      vec![PathBuf::from("README.md")],
      "enter should confirm the selected row"
    );

    let _ = std::fs::remove_dir_all(&repo_root);
  }

  #[gpui::test]
  async fn staged_and_unstaged_files_land_in_their_own_section(cx: &mut gpui::TestAppContext) {
    let repo_root = temp_repo("changes-list-two-sections");
    std::fs::write(repo_root.join("other.txt"), "new\n").expect("write second file");
    stage_file(&repo_root, Path::new("other.txt")).expect("stage second file");

    let (list, cx) = add_changes_list_window(repo_root.clone(), cx);
    set_entries_from_disk(&list, cx, &repo_root);

    list.read_with(cx, |list, _| assert_eq!(list.entries().len(), 2));
    // One row per section, and nothing beyond.
    assert!(cx.debug_bounds("changes-row-0-0").is_some());
    assert!(cx.debug_bounds("changes-row-1-0").is_some());
    assert!(cx.debug_bounds("changes-row-0-1").is_none());
    assert!(cx.debug_bounds("changes-row-1-1").is_none());

    let _ = std::fs::remove_dir_all(&repo_root);
  }

  #[gpui::test]
  async fn switching_repository_drops_the_previous_rows(cx: &mut gpui::TestAppContext) {
    let repo_root = temp_repo("changes-list-switch-from");
    let other_root = temp_repo("changes-list-switch-to");

    let (list, cx) = add_changes_list_window(repo_root.clone(), cx);
    set_entries_from_disk(&list, cx, &repo_root);
    assert!(cx.debug_bounds("changes-row-0-0").is_some());

    list.update(cx, |list, cx| {
      list.set_repo_root(Some(other_root.clone()), cx)
    });
    cx.run_until_parked();

    list.read_with(cx, |list, _| assert!(list.entries().is_empty()));
    assert!(
      cx.debug_bounds("changes-row-0-0").is_none(),
      "the previous repository's files must not stay on screen"
    );

    let _ = std::fs::remove_dir_all(&repo_root);
    let _ = std::fs::remove_dir_all(&other_root);
  }

  #[test]
  fn staging_only_asks_while_the_conflict_is_unresolved() {
    assert!(should_confirm_stage(RepoStatusKind::Conflicted, true));
    // Markers resolved: staging is how the conflict gets marked done.
    assert!(!should_confirm_stage(RepoStatusKind::Conflicted, false));
    assert!(!should_confirm_stage(RepoStatusKind::Modified, true));
  }

  #[gpui::test]
  async fn staging_a_conflicted_file_asks_before_marking_it_resolved(
    cx: &mut gpui::TestAppContext,
  ) {
    let repo_root = temp_repo("changes-list-conflict-stage");

    let (list, cx) = add_changes_list_window(repo_root.clone(), cx);
    // A conflicted entry with the markers still in the open file.
    list.update(cx, |list, cx| {
      list.set_open_file_has_conflict_markers(true);
      list.set_entries(
        vec![RepoStatusEntry {
          path: PathBuf::from("README.md"),
          old_path: None,
          status: RepoStatusKind::Conflicted,
          stage: RepoStage::Unstaged,
        }],
        cx,
      );
    });
    cx.run_until_parked();

    let button = cx
      .debug_bounds("changes-stage-0-0")
      .expect("stage button bounds");
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    cx.run_until_parked();

    assert!(
      cx.update(|window, cx| window.has_active_dialog(cx)),
      "staging an unresolved conflict must ask first"
    );
    list.read_with(cx, |list, _| assert!(list._action_task.is_none()));

    let _ = std::fs::remove_dir_all(&repo_root);
  }

  #[gpui::test]
  async fn discarding_a_renamed_file_puts_the_old_name_back(cx: &mut gpui::TestAppContext) {
    let repo_root = temp_repo("changes-list-rename-restore");
    std::fs::write(repo_root.join("README.md"), "v1\n").expect("reset content");
    std::fs::rename(repo_root.join("README.md"), repo_root.join("RENAMED.md"))
      .expect("rename file");
    stage_all(&repo_root).expect("stage the rename");

    let (list, cx) = add_changes_list_window(repo_root.clone(), cx);
    set_entries_from_disk(&list, cx, &repo_root);

    let entry = list.read_with(cx, |list, _| list.entries()[0].clone());
    assert_eq!(entry.status, RepoStatusKind::Renamed);

    let task = list.update_in(cx, |list, window, cx| {
      list.restore_file(entry.path.clone(), entry.status, window, cx);
      list._action_task.take().expect("restore task")
    });
    task.await;
    cx.run_until_parked();

    assert!(
      repo_root.join("README.md").exists(),
      "the original name should be back"
    );
    assert!(!repo_root.join("RENAMED.md").exists());

    let _ = std::fs::remove_dir_all(&repo_root);
  }

  #[gpui::test]
  async fn restore_all_handles_modified_untracked_and_renamed_files(cx: &mut gpui::TestAppContext) {
    let repo_root = temp_repo("changes-list-restore-all");
    // A second tracked file to rename, on top of the modified README.
    std::fs::write(repo_root.join("keep.txt"), "kept\n").expect("write file");
    stage_file(&repo_root, Path::new("keep.txt")).expect("stage");
    {
      let repo = git2::Repository::open(&repo_root).expect("open repo");
      let mut index = repo.index().expect("index");
      index.add_path(Path::new("keep.txt")).expect("add");
      index.write().expect("write index");
      let tree = repo
        .find_tree(index.write_tree().expect("write tree"))
        .expect("tree");
      let parent = repo
        .head()
        .and_then(|head| head.peel_to_commit())
        .expect("head");
      let signature = git2::Signature::now("Test", "test@example.com").expect("signature");
      repo
        .commit(
          Some("HEAD"),
          &signature,
          &signature,
          "second",
          &tree,
          &[&parent],
        )
        .expect("commit");
    }

    std::fs::rename(repo_root.join("keep.txt"), repo_root.join("moved.txt")).expect("rename");
    std::fs::write(repo_root.join("scratch.txt"), "temp\n").expect("write untracked");
    stage_all(&repo_root).expect("stage everything");

    let (list, cx) = add_changes_list_window(repo_root.clone(), cx);
    set_entries_from_disk(&list, cx, &repo_root);

    let task = list.update_in(cx, |list, window, cx| {
      list.restore_all(window, cx);
      list._action_task.take().expect("restore all task")
    });
    task.await;
    cx.run_until_parked();

    assert_eq!(
      std::fs::read_to_string(repo_root.join("README.md")).expect("read modified file"),
      "v1\n",
      "a modified file goes back to its committed content"
    );
    assert!(
      repo_root.join("keep.txt").exists() && !repo_root.join("moved.txt").exists(),
      "a rename is undone"
    );
    assert!(
      !repo_root.join("scratch.txt").exists(),
      "an untracked file is deleted"
    );

    let _ = std::fs::remove_dir_all(&repo_root);
  }

  #[gpui::test]
  async fn restore_all_asks_before_discarding_everything(cx: &mut gpui::TestAppContext) {
    let repo_root = temp_repo("changes-list-restore-all-confirm");

    let (list, cx) = add_changes_list_window(repo_root.clone(), cx);
    set_entries_from_disk(&list, cx, &repo_root);

    list.update_in(cx, |list, window, cx| list.confirm_restore_all(window, cx));
    cx.run_until_parked();

    assert!(cx.update(|window, cx| window.has_active_dialog(cx)));
    list.read_with(cx, |list, _| assert!(list._action_task.is_none()));
    assert_eq!(
      std::fs::read_to_string(repo_root.join("README.md")).expect("read file"),
      "v2\n",
      "nothing is discarded until the dialog is confirmed"
    );

    let _ = std::fs::remove_dir_all(&repo_root);
  }

  #[gpui::test]
  async fn nothing_runs_without_a_repository(cx: &mut gpui::TestAppContext) {
    let (list, cx) = add_changes_list_window(std::env::temp_dir(), cx);
    list.update(cx, |list, cx| list.set_repo_root(None, cx));

    list.update_in(cx, |list, window, cx| {
      list.stage_file(PathBuf::from("README.md"), window, cx);
      list.unstage_file(PathBuf::from("README.md"), window, cx);
      list.restore_file(
        PathBuf::from("README.md"),
        RepoStatusKind::Modified,
        window,
        cx,
      );
    });

    list.read_with(cx, |list, _| assert!(list._action_task.is_none()));
  }

  #[gpui::test]
  async fn discarding_moves_the_selection_off_the_deleted_row(cx: &mut gpui::TestAppContext) {
    let repo_root = temp_repo("changes-list-selection-after-discard");
    std::fs::write(repo_root.join("other.txt"), "new\n").expect("write second file");

    let (list, cx) = add_changes_list_window(repo_root.clone(), cx);
    set_entries_from_disk(&list, cx, &repo_root);

    list.update(cx, |list, cx| {
      list.select_path(Some(Path::new("other.txt")), cx)
    });
    let task = list.update_in(cx, |list, window, cx| {
      list.restore_file(
        PathBuf::from("other.txt"),
        RepoStatusKind::Untracked,
        window,
        cx,
      );
      list._action_task.take().expect("discard task")
    });
    task.await;
    cx.run_until_parked();

    // The selected row is gone: the highlight must not point at nothing.
    list.read_with(cx, |list, cx| assert!(list.has_selection(cx)));

    let _ = std::fs::remove_dir_all(&repo_root);
  }

  #[gpui::test]
  async fn discarding_a_deleted_file_brings_it_back(cx: &mut gpui::TestAppContext) {
    let repo_root = temp_repo("changes-list-restore-deleted");
    std::fs::remove_file(repo_root.join("README.md")).expect("delete tracked file");

    let (list, cx) = add_changes_list_window(repo_root.clone(), cx);
    set_entries_from_disk(&list, cx, &repo_root);

    let entry = list.read_with(cx, |list, _| list.entries()[0].clone());
    assert_eq!(entry.status, RepoStatusKind::Deleted);

    let task = list.update_in(cx, |list, window, cx| {
      list.restore_file(entry.path.clone(), entry.status, window, cx);
      list._action_task.take().expect("restore task")
    });
    task.await;
    cx.run_until_parked();

    assert_eq!(
      std::fs::read_to_string(repo_root.join("README.md")).expect("read restored file"),
      "v1\n"
    );

    let _ = std::fs::remove_dir_all(&repo_root);
  }

  #[gpui::test]
  async fn a_failing_action_tells_the_user(cx: &mut gpui::TestAppContext) {
    let repo_root = temp_repo("changes-list-action-error");

    let (list, cx) = add_changes_list_window(repo_root.clone(), cx);
    set_entries_from_disk(&list, cx, &repo_root);

    // Nothing to delete under this name: the job fails.
    let task = list.update_in(cx, |list, window, cx| {
      list.restore_file(
        PathBuf::from("ghost.txt"),
        RepoStatusKind::Untracked,
        window,
        cx,
      );
      list._action_task.take().expect("restore task")
    });
    task.await;
    cx.run_until_parked();

    let reported = cx.update(|window, cx| {
      !gpui_component::Root::read(window, cx)
        .notification
        .read(cx)
        .notifications()
        .is_empty()
    });
    assert!(reported, "a failed git action must be reported");

    let _ = std::fs::remove_dir_all(&repo_root);
  }
}
