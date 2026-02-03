use std::{
  path::{Path, PathBuf},
  sync::Arc,
  time::Duration,
};

use editor::{DiffViewMode, Editor, HunkAction, HunkState};
use git::{
  BranchKind, BranchRef, BranchStatus, HeadCommitStatus, RepoStage, RepoStatusEntry,
  RepoStatusKind, amend_commit, commit_changes, create_branch, create_branch_from,
  current_branch_status, delete_untracked_file, head_commit_status, list_branches,
  list_repo_status, merge_branch, push, restore_file, stage_all, stage_file, switch_branch,
  undo_last_commit, unstage_all, unstage_file,
};
use gpui::{
  AnyElement, App, Context, Corner, Entity, FocusHandle, Focusable, Hsla, InteractiveElement,
  Keystroke, ParentElement, PathPromptOptions, Render, SharedString, Styled, Task, Window, actions,
  div, prelude::*, px, uniform_list,
};
use gpui_component::{
  ActiveTheme as _, Collapsible, Disableable, Icon, IconName, Sizable, StyledExt as _,
  button::{Button, ButtonGroup, ButtonVariant, ButtonVariants as _},
  kbd::Kbd,
  menu::{DropdownMenu, PopupMenuItem},
  select::{Select, SelectEvent, SelectItem, SelectState},
  sidebar::SidebarItem,
  tooltip::Tooltip,
};
use smol::unblock;

use crate::{
  config::{ConfigStore, RecentRepository},
  workspace::{WorkspacePage, WorkspaceRoute},
};
use ui::{
  CommandPalette, CommandPaletteAction, CommandPaletteBranch, CommandPaletteBranchKind,
  CommandPaletteConfig, CommandPaletteHandler, ConfirmDialog, Input, InputState, SearchFileEntry,
  SearchFileHandler, SearchFilePalette, SearchFilePaletteConfig, WindowExt,
};

const HEADER_HEIGHT: f32 = 44.0;
const SIDEBAR_DEFAULT_WIDTH: f32 = 280.0;
const SIDEBAR_MIN_WIDTH: f32 = 220.0;
const SIDEBAR_MAX_WIDTH: f32 = 500.0;
const STATUS_POLL_INTERVAL_MS: u64 = 800;
const EDITOR_HEADER_HEIGHT: f32 = 40.0;

trait StatusThemeExt {
  fn status_orange(&self) -> gpui::Hsla;
  fn status_green(&self) -> gpui::Hsla;
  fn status_red(&self) -> gpui::Hsla;
}

impl StatusThemeExt for gpui_component::Theme {
  fn status_orange(&self) -> Hsla {
    if self.mode.is_dark() {
      Hsla {
        h: 30.0 / 360.0,
        s: 0.85,
        l: 0.58,
        a: 1.0,
      }
    } else {
      self.warning
    }
  }

  fn status_green(&self) -> Hsla {
    if self.mode.is_dark() {
      Hsla {
        h: 135.0 / 360.0,
        s: 0.75,
        l: 0.55,
        a: 1.0,
      }
    } else {
      self.success
    }
  }

  fn status_red(&self) -> Hsla {
    if self.mode.is_dark() {
      Hsla {
        h: 0.0,
        s: 0.75,
        l: 0.58,
        a: 1.0,
      }
    } else {
      self.danger
    }
  }
}

actions!(
  workspace,
  [
    OpenRepository,
    SaveFile,
    ShowCommandPalette,
    ShowFileSearch,
    CommitChanges
  ]
);

#[derive(Clone)]
struct FileSidebarItem {
  label: SharedString,
  status_letter: SharedString,
  status_color: gpui::Hsla,
  stage_icon: IconName,
  stage_color: gpui::Hsla,
  stage_tooltip: Option<SharedString>,
  stage_action: Option<std::rc::Rc<dyn Fn(&gpui::ClickEvent, &mut Window, &mut App)>>,
  unstage_action: Option<std::rc::Rc<dyn Fn(&gpui::ClickEvent, &mut Window, &mut App)>>,
  restore_action: Option<std::rc::Rc<dyn Fn(&gpui::ClickEvent, &mut Window, &mut App)>>,
  active: bool,
  collapsed: bool,
  disabled: bool,
  handler: std::rc::Rc<dyn Fn(&gpui::ClickEvent, &mut Window, &mut App)>,
}

impl FileSidebarItem {
  fn new(
    label: impl Into<SharedString>,
    status_letter: impl Into<SharedString>,
    status_color: gpui::Hsla,
    stage_icon: IconName,
    stage_color: gpui::Hsla,
    stage_tooltip: Option<SharedString>,
  ) -> Self {
    Self {
      label: label.into(),
      status_letter: status_letter.into(),
      status_color,
      stage_icon,
      stage_color,
      stage_tooltip,
      stage_action: None,
      unstage_action: None,
      restore_action: None,
      active: false,
      collapsed: false,
      disabled: false,
      handler: std::rc::Rc::new(|_, _, _| {}),
    }
  }

  fn placeholder(label: impl Into<SharedString>, theme: &gpui_component::Theme) -> Self {
    Self::new(
      label,
      "",
      theme.muted_foreground,
      IconName::Minus,
      theme.muted_foreground,
      None,
    )
    .disabled(true)
  }

  fn active(mut self, active: bool) -> Self {
    self.active = active;
    self
  }

  fn disabled(mut self, disabled: bool) -> Self {
    self.disabled = disabled;
    self
  }

  fn on_stage(
    mut self,
    handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
  ) -> Self {
    self.stage_action = Some(std::rc::Rc::new(handler));
    self
  }

  fn on_unstage(
    mut self,
    handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
  ) -> Self {
    self.unstage_action = Some(std::rc::Rc::new(handler));
    self
  }

  fn on_restore(
    mut self,
    handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
  ) -> Self {
    self.restore_action = Some(std::rc::Rc::new(handler));
    self
  }

  fn on_click(
    mut self,
    handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
  ) -> Self {
    self.handler = std::rc::Rc::new(handler);
    self
  }

  fn status_tooltip(&self) -> SharedString {
    if self.status_letter.is_empty() {
      return "".into();
    }
    match self.status_letter.as_ref() {
      "M" => "Modified",
      "A" => "Added",
      "D" => "Deleted",
      "R" => "Renamed",
      "T" => "Type change",
      "U" => "Untracked",
      _ => "Unknown",
    }
    .into()
  }
}

impl Collapsible for FileSidebarItem {
  fn is_collapsed(&self) -> bool {
    self.collapsed
  }

  fn collapsed(mut self, collapsed: bool) -> Self {
    self.collapsed = collapsed;
    self
  }
}

impl SidebarItem for FileSidebarItem {
  fn render(
    self,
    id: impl Into<gpui::ElementId>,
    _window: &mut Window,
    cx: &mut App,
  ) -> impl IntoElement {
    let id = id.into();
    let theme = cx.theme().clone();
    let handler = self.handler.clone();
    let is_hoverable = !self.active && !self.disabled;
    let is_collapsed = self.collapsed;
    let has_status = !self.status_letter.is_empty();
    let has_actions =
      self.stage_action.is_some() || self.unstage_action.is_some() || self.restore_action.is_some();
    let stage_action = self.stage_action.clone();
    let unstage_action = self.unstage_action.clone();
    let restore_action = self.restore_action.clone();
    let hover_group: SharedString = format!("sidebar-item-{}", self.label).into();
    let status_tooltip = self.status_tooltip();

    div()
      .id(id)
      .w_full()
      .relative()
      .flex()
      .items_center()
      .gap_2()
      .p_4()
      .group(hover_group.clone())
      .when(is_hoverable, |this| {
        this.hover(|this| {
          this
            .bg(theme.sidebar_accent.opacity(0.8))
            .text_color(theme.sidebar_accent_foreground)
        })
      })
      .when(self.active, |this| {
        this
          .font_medium()
          .bg(theme.sidebar_accent)
          .text_color(theme.sidebar_accent_foreground)
      })
      .when(self.disabled, |this| {
        this.text_color(theme.muted_foreground)
      })
      .when(!is_collapsed, |this| this.h_7())
      .when(is_collapsed, |this| this.justify_center())
      .when(has_status, |this| {
        let status_tooltip = status_tooltip.clone();
        let status_id = format!("status-letter-{}", self.label);
        this.child(
          div()
            .id(status_id)
            .tooltip(move |window, cx| Tooltip::new(status_tooltip.clone()).build(window, cx))
            .child(
              div()
                .w(px(15.))
                .text_xs()
                .text_color(self.status_color)
                .child(self.status_letter.clone()),
            ),
        )
      })
      .child({
        let icon = Icon::new(self.stage_icon)
          .size_3()
          .text_color(self.stage_color);
        let icon_element: AnyElement = if let Some(tooltip) = self.stage_tooltip.clone() {
          let tooltip_id = format!("stage-icon-{}", self.label);
          div()
            .id(tooltip_id)
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            .child(icon)
            .into_any_element()
        } else {
          div().child(icon).into_any_element()
        };
        icon_element
      })
      .when(!is_collapsed, |this| {
        this.child(
          div()
            .w_full()
            .overflow_hidden()
            .text_ellipsis_start()
            .child(self.label.clone()),
        )
      })
      .when(!is_collapsed && !self.disabled && has_actions, |this| {
        let mut actions = ButtonGroup::new(format!("file-actions-{}", self.label))
          .primary()
          .with_variant(ButtonVariant::Secondary)
          .xsmall();

        if let Some(handler) = stage_action {
          actions = actions.child(
            Button::new(format!("stage-file-{}", self.label))
              .icon(IconName::Plus)
              .bg(theme.background)
              .tooltip("Stage file")
              .on_click(move |ev, window, cx| {
                handler(ev, window, cx);
              }),
          );
        }
        if let Some(handler) = unstage_action {
          actions = actions.child(
            Button::new(format!("unstage-file-{}", self.label))
              .icon(IconName::Minus)
              .bg(theme.background)
              .tooltip("Unstage file")
              .on_click(move |ev, window, cx| {
                handler(ev, window, cx);
              }),
          );
        }
        if let Some(handler) = restore_action {
          actions = actions.child(
            Button::new(format!("restore-file-{}", self.label))
              .icon(IconName::Undo)
              .bg(theme.background)
              .tooltip("Restore file")
              .on_click(move |ev, window, cx| {
                handler(ev, window, cx);
              }),
          );
        }

        this.child(
          div()
            .absolute()
            .right(px(5.0))
            .top(px(5.0))
            .opacity(0.0)
            .group_hover(hover_group, |style| style.opacity(1.0))
            .child(actions),
        )
      })
      .when(!self.disabled, |this| {
        this.on_click(move |ev, window, cx| {
          handler(ev, window, cx);
        })
      })
  }
}

#[derive(Clone)]
struct RecentRepoItem {
  path: PathBuf,
  label: SharedString,
}

impl RecentRepoItem {
  fn new(repo: &RecentRepository) -> Self {
    let label = repo.path.to_string_lossy().to_string();
    Self {
      path: repo.path.clone(),
      label: label.into(),
    }
  }
}

impl SelectItem for RecentRepoItem {
  type Value = PathBuf;

  fn title(&self) -> SharedString {
    self.label.clone()
  }

  fn value(&self) -> &Self::Value {
    &self.path
  }
}

pub struct GitPage {
  focus_handle: FocusHandle,
  repo_select: Entity<SelectState<Vec<RecentRepoItem>>>,
  selected_repo: Option<PathBuf>,
  status_entries: Vec<RepoStatusEntry>,
  branch_status: Option<BranchStatus>,
  has_head_commit: bool,
  can_undo_last_commit: bool,
  can_push: bool,
  can_force_push: bool,
  has_staged_changes: bool,
  selected_file: Option<PathBuf>,
  editor: Option<Entity<Editor>>,
  diff_view: DiffViewMode,
  status_task: Option<Task<()>>,
  poll_task: Option<Task<()>>,
  commit_input: Entity<InputState>,
}

impl GitPage {
  fn split_disabled_for_path(&self, rel_path: &Path) -> bool {
    self.status_entries.iter().any(|entry| {
      entry.path == rel_path
        && matches!(
          entry.status,
          RepoStatusKind::Untracked | RepoStatusKind::Added
        )
    })
  }

  pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
    let recent = ConfigStore::load_recent_repositories();
    let items: Vec<RecentRepoItem> = recent.iter().map(RecentRepoItem::new).collect();
    let repo_select = cx.new(|cx| SelectState::new(items, None, window, cx).searchable(true));
    let selected_repo = recent.first().map(|repo| repo.path.clone());

    if let Some(repo) = selected_repo.as_ref() {
      repo_select.update(cx, |state, cx| {
        state.set_selected_value(repo, window, cx);
      });
    }

    let commit_input = cx.new(|cx| {
      InputState::new(window, cx)
        .auto_grow(1, 5)
        .placeholder("Commit message...")
    });

    let mut view = Self {
      focus_handle: cx.focus_handle(),
      repo_select,
      selected_repo,
      status_entries: Vec::new(),
      branch_status: None,
      has_head_commit: false,
      can_undo_last_commit: false,
      can_push: false,
      can_force_push: false,
      has_staged_changes: false,
      selected_file: None,
      editor: None,
      diff_view: DiffViewMode::Inline,
      status_task: None,
      poll_task: None,
      commit_input,
    };

    view.subscribe_to_repo_select(window, cx);
    view.reload_status(cx);
    view.start_polling(cx);

    view
  }

  fn subscribe_to_repo_select(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
    cx.subscribe(
      &self.repo_select,
      move |this, _state, event: &SelectEvent<Vec<RecentRepoItem>>, cx| {
        if let SelectEvent::Confirm(Some(repo)) = event {
          this.set_selected_repo(repo.clone(), cx);
        }
      },
    )
    .detach();
  }

  fn set_selected_repo(&mut self, repo_root: PathBuf, cx: &mut Context<Self>) {
    if self.selected_repo.as_ref() == Some(&repo_root) {
      return;
    }

    self.selected_repo = Some(repo_root.clone());
    self.selected_file = None;
    self.editor = None;
    ConfigStore::persist_recent_repository(&repo_root);

    self.reload_status(cx);
    cx.notify();
  }

  fn reload_status(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      self.status_entries.clear();
      self.branch_status = None;
      self.has_head_commit = false;
      self.can_undo_last_commit = false;
      self.can_push = false;
      self.can_force_push = false;
      self.has_staged_changes = false;
      return;
    };

    let task = cx.spawn(async move |this, cx| {
      let status = unblock(move || {
        let entries = list_repo_status(&repo_root).ok()?;
        let branch = current_branch_status(&repo_root).ok();
        let head_status = head_commit_status(&repo_root).ok();
        Some((entries, branch, head_status))
      })
      .await;
      let Some((entries, branch_status, head_status)) = status else {
        return;
      };

      let _ = this.update(cx, |this, cx| {
        this.status_entries = entries;
        this.branch_status = branch_status;
        this.has_staged_changes = this
          .status_entries
          .iter()
          .any(|entry| matches!(entry.stage, RepoStage::Staged | RepoStage::PartiallyStaged));
        let head_status = head_status.unwrap_or(HeadCommitStatus {
          has_head_commit: false,
          can_undo_last_commit: false,
        });
        this.has_head_commit = head_status.has_head_commit;
        this.can_undo_last_commit = head_status.can_undo_last_commit;
        let (can_push, can_force_push) = Self::push_flags(this.branch_status.as_ref());
        this.can_push = can_push;
        this.can_force_push = can_force_push;
        if let Some(selected) = this.selected_file.as_ref() {
          let still_present = this
            .status_entries
            .iter()
            .any(|entry| &entry.path == selected);
          if !still_present {
            this.selected_file = None;
            this.editor = None;
          } else if this.split_disabled_for_path(selected) && this.diff_view != DiffViewMode::Inline
          {
            this.diff_view = DiffViewMode::Inline;
            if let Some(editor) = this.editor.clone() {
              editor.update(cx, |editor, cx| {
                editor.set_diff_view_mode(DiffViewMode::Inline, cx)
              });
            }
          }
        }
        cx.notify();
      });
    });

    self.status_task = Some(task);
  }

  fn start_polling(&mut self, cx: &mut Context<Self>) {
    if self.poll_task.is_some() {
      return;
    }

    self.poll_task = Some(cx.spawn(async move |this, cx| {
      loop {
        cx.background_executor()
          .timer(Duration::from_millis(STATUS_POLL_INTERVAL_MS))
          .await;

        let repo_root = match this.update(cx, |this, _| this.selected_repo.clone()) {
          Ok(value) => value,
          Err(_) => return,
        };
        let Some(repo_root) = repo_root else {
          continue;
        };

        let status = unblock(move || {
          let entries = list_repo_status(&repo_root).ok()?;
          let branch = current_branch_status(&repo_root).ok();
          let head_status = head_commit_status(&repo_root).ok();
          Some((entries, branch, head_status))
        })
        .await;
        let Some((entries, branch_status, head_status)) = status else {
          continue;
        };

        let _ = this.update(cx, |this, cx| {
          this.status_entries = entries;
          this.branch_status = branch_status;
          this.has_staged_changes = this
            .status_entries
            .iter()
            .any(|entry| matches!(entry.stage, RepoStage::Staged | RepoStage::PartiallyStaged));
          let head_status = head_status.unwrap_or(HeadCommitStatus {
            has_head_commit: false,
            can_undo_last_commit: false,
          });
          this.has_head_commit = head_status.has_head_commit;
          this.can_undo_last_commit = head_status.can_undo_last_commit;
          let (can_push, can_force_push) = Self::push_flags(this.branch_status.as_ref());
          this.can_push = can_push;
          this.can_force_push = can_force_push;
          if let Some(selected) = this.selected_file.as_ref() {
            let still_present = this
              .status_entries
              .iter()
              .any(|entry| &entry.path == selected);
            if !still_present {
              this.selected_file = None;
              this.editor = None;
            }
          }
          cx.notify();
        });
      }
    }));
  }

  fn show_command_palette_action(
    &mut self,
    _: &ShowCommandPalette,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.open_command_palette(window, cx);
  }

  fn show_file_search_action(
    &mut self,
    _: &ShowFileSearch,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.open_file_search_palette(window, cx);
  }

  fn open_repository_action(
    &mut self,
    _: &OpenRepository,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.start_open_repository(window, cx);
  }

  fn start_open_repository(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let receiver = cx.prompt_for_paths(PathPromptOptions {
      files: false,
      directories: true,
      multiple: false,
      prompt: Some("Select a repository".into()),
    });

    let window_handle = window.window_handle();
    let repo_select = self.repo_select.clone();

    cx.spawn(async move |this, cx| {
      let Ok(result) = receiver.await else {
        return;
      };

      match result {
        Ok(Some(paths)) => {
          if let Some(path) = paths.into_iter().next() {
            ConfigStore::persist_recent_repository(&path);
            let recent = ConfigStore::load_recent_repositories();
            let items: Vec<RecentRepoItem> = recent.iter().map(RecentRepoItem::new).collect();

            let _ = cx.update_window(window_handle, |_, window, cx| {
              repo_select.update(cx, |state, cx| {
                state.set_items(items, window, cx);
                state.set_selected_value(&path, window, cx);
              });
            });

            let _ = this.update(cx, |view, cx| {
              view.set_selected_repo(path, cx);
            });
          }
        }
        Ok(None) => {}
        Err(_) => {}
      }
    })
    .detach();
  }

  fn open_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let Some(root_path) = self.selected_repo.clone() else {
      return;
    };

    let branches = match list_branches(&root_path) {
      Ok(branches) => branches,
      Err(_) => {
        return;
      }
    };

    let palette_branches = branches
      .into_iter()
      .map(|branch| CommandPaletteBranch {
        name: branch.name.into(),
        kind: match branch.kind {
          BranchKind::Local => CommandPaletteBranchKind::Local,
          BranchKind::Remote => CommandPaletteBranchKind::Remote,
        },
      })
      .collect::<Vec<_>>();

    let view = cx.entity();
    let handler: CommandPaletteHandler = Arc::new(move |action, _window, cx| {
      view.update(cx, |view, cx| {
        view.handle_command_palette_action(action, cx)
      })
    });

    let palette = cx.new(|cx| {
      CommandPalette::new(
        window,
        cx,
        CommandPaletteConfig::new(palette_branches, handler),
      )
    });
    let palette_for_dialog = palette.clone();

    window.open_dialog(cx, move |dialog, _, _| {
      dialog
        .p_0()
        .border_0()
        .min_h_0()
        .overlay_closable(true)
        .keyboard(true)
        .close_button(false)
        .child(palette_for_dialog.clone())
    });
  }

  fn open_file_search_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.selected_repo.is_none() || self.status_entries.is_empty() {
      return;
    }

    let entries = self
      .status_entries
      .iter()
      .map(|entry| {
        let file_label = entry.path.to_string_lossy();
        let file_label = file_label.replace(['\n', '\r'], "");
        SearchFileEntry::new(entry.path.clone(), file_label)
      })
      .collect::<Vec<_>>();

    let view = cx.entity();
    let handler: SearchFileHandler = Arc::new(move |path, _window, cx| {
      view.update(cx, |view, cx| {
        view.open_file(path, cx);
      });
      Ok(())
    });

    let palette = cx
      .new(|cx| SearchFilePalette::new(window, cx, SearchFilePaletteConfig::new(entries, handler)));
    let palette_for_dialog = palette.clone();

    window.open_dialog(cx, move |dialog, _, _| {
      dialog
        .p_0()
        .border_0()
        .min_h_0()
        .overlay_closable(true)
        .keyboard(true)
        .close_button(false)
        .child(palette_for_dialog.clone())
    });
  }

  fn handle_command_palette_action(
    &mut self,
    action: CommandPaletteAction,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    let Some(root_path) = self.selected_repo.clone() else {
      return Err("No repository selected.".into());
    };

    let result = match action {
      CommandPaletteAction::SwitchBranch(branch) => {
        let branch_ref = BranchRef {
          name: branch.name.to_string(),
          kind: match branch.kind {
            CommandPaletteBranchKind::Local => BranchKind::Local,
            CommandPaletteBranchKind::Remote => BranchKind::Remote,
          },
        };
        switch_branch(&root_path, &branch_ref)
      }
      CommandPaletteAction::CreateBranch { name } => {
        let branch_ref = BranchRef {
          name: name.clone(),
          kind: BranchKind::Local,
        };
        create_branch(&root_path, &name).and_then(|_| switch_branch(&root_path, &branch_ref))
      }
      CommandPaletteAction::CreateBranchFrom { name, base } => {
        let branch_ref = BranchRef {
          name: base.name.to_string(),
          kind: match base.kind {
            CommandPaletteBranchKind::Local => BranchKind::Local,
            CommandPaletteBranchKind::Remote => BranchKind::Remote,
          },
        };
        let new_branch = BranchRef {
          name: name.clone(),
          kind: BranchKind::Local,
        };
        create_branch_from(&root_path, &name, &branch_ref)
          .and_then(|_| switch_branch(&root_path, &new_branch))
      }
      CommandPaletteAction::MergeBranch { name } => {
        let branch_ref = BranchRef {
          name: name.name.to_string(),
          kind: match name.kind {
            CommandPaletteBranchKind::Local => BranchKind::Local,
            CommandPaletteBranchKind::Remote => BranchKind::Remote,
          },
        };
        merge_branch(&root_path, &branch_ref)
      }
    };

    if let Err(err) = result {
      let message: SharedString = format!("Action failed: {err}").into();
      return Err(message);
    }

    self.reload_status(cx);
    if let Some(editor) = self.editor.clone() {
      editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
    }
    Ok(())
  }

  fn commit_changes_action(
    &mut self,
    _: &CommitChanges,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let focus_handle = self.commit_input.read(cx).focus_handle(cx);
    if !focus_handle.contains_focused(window, cx) {
      return;
    }
    self.commit_changes_inner(window, cx);
  }

  fn commit_changes(&mut self, _: &gpui::ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
    self.commit_changes_inner(window, cx);
  }

  fn commit_changes_inner(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    let message = self.commit_input.read(cx).value().to_string();
    if message.trim().is_empty() {
      return;
    }
    let has_changes = !self.status_entries.is_empty();
    if !has_changes {
      return;
    }
    let stage_all_needed = !self.has_staged_changes;

    let window_handle = window.window_handle();
    let commit_input = self.commit_input.clone();
    let editor = self.editor.clone();

    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || {
        if stage_all_needed {
          stage_all(&repo_root)?;
        }
        commit_changes(&repo_root, &message)
      })
      .await;
      let _ = this.update(cx, |this, cx| {
        if result.is_ok() {
          let _ = cx.update_window(window_handle, |_, window, cx| {
            commit_input.update(cx, |input, cx| input.set_value("", window, cx));
          });
        }
        this.reload_status(cx);
        if let Some(editor) = editor.clone() {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
      });
    });

    self.status_task = Some(task);
  }

  fn commit_amend_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    if !self.has_head_commit {
      return;
    }

    let message = self.commit_input.read(cx).value().to_string();
    let message = message.trim().to_string();
    let message_opt = if message.is_empty() {
      None
    } else {
      Some(message)
    };

    let window_handle = window.window_handle();
    let commit_input = self.commit_input.clone();
    let editor = self.editor.clone();

    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || amend_commit(&repo_root, message_opt.as_deref())).await;
      let _ = this.update(cx, |this, cx| {
        if result.is_ok() {
          let _ = cx.update_window(window_handle, |_, window, cx| {
            commit_input.update(cx, |input, cx| input.set_value("", window, cx));
          });
        }
        this.reload_status(cx);
        if let Some(editor) = editor.clone() {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
      });
    });

    self.status_task = Some(task);
  }

  fn undo_last_commit_action(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    if !self.can_undo_last_commit {
      return;
    }

    let editor = self.editor.clone();
    let task = cx.spawn(async move |this, cx| {
      let _ = unblock(move || undo_last_commit(&repo_root)).await;
      let _ = this.update(cx, |this, cx| {
        this.reload_status(cx);
        if let Some(editor) = editor.clone() {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
      });
    });

    self.status_task = Some(task);
  }

  fn push_changes_action(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    if !self.can_push {
      return;
    }

    let task = cx.spawn(async move |this, cx| {
      let _ = unblock(move || push(&repo_root, false)).await;
      let _ = this.update(cx, |this, cx| {
        this.reload_status(cx);
      });
    });

    self.status_task = Some(task);
  }

  fn force_push_changes_action(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    if !self.can_force_push {
      return;
    }

    let task = cx.spawn(async move |this, cx| {
      let _ = unblock(move || push(&repo_root, true)).await;
      let _ = this.update(cx, |this, cx| {
        this.reload_status(cx);
      });
    });

    self.status_task = Some(task);
  }

  fn open_file(&mut self, rel_path: PathBuf, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    if self.selected_file.as_ref() == Some(&rel_path) {
      return;
    }
    let split_disabled = self.split_disabled_for_path(&rel_path);
    if split_disabled && self.diff_view != DiffViewMode::Inline {
      self.diff_view = DiffViewMode::Inline;
    }
    let file_path = repo_root.join(&rel_path);
    let editor = cx.new(|cx| Editor::new_with_paths(repo_root, file_path, cx));
    let diff_view = if split_disabled {
      DiffViewMode::Inline
    } else {
      self.diff_view
    };
    editor.update(cx, |editor, cx| editor.set_diff_view_mode(diff_view, cx));
    self.editor = Some(editor);
    self.selected_file = Some(rel_path);
    cx.notify();
  }

  fn toggle_diff_view(&mut self, cx: &mut Context<Self>) {
    if let Some(selected) = self.selected_file.as_ref()
      && self.split_disabled_for_path(selected)
    {
      return;
    }
    self.diff_view = match self.diff_view {
      DiffViewMode::Inline => DiffViewMode::Split,
      DiffViewMode::Split => DiffViewMode::Inline,
    };

    if let Some(editor) = self.editor.clone() {
      let diff_view = self.diff_view;
      editor.update(cx, |editor, cx| editor.set_diff_view_mode(diff_view, cx));
    }

    cx.notify();
  }

  fn toggle_stage_all_action(
    &mut self,
    _: &gpui::ClickEvent,
    _: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.all_changes_staged() {
      self.unstage_all_action(cx);
    } else {
      self.stage_all_action(cx);
    }
  }

  fn stage_all_action(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    let editor = self.editor.clone();
    let task = cx.spawn(async move |this, cx| {
      let _ = unblock(move || stage_all(&repo_root)).await;
      let _ = this.update(cx, |this, cx| {
        this.reload_status(cx);
        if let Some(editor) = editor.clone() {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
      });
    });
    self.status_task = Some(task);
  }

  fn unstage_all_action(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    let editor = self.editor.clone();
    let task = cx.spawn(async move |this, cx| {
      let _ = unblock(move || unstage_all(&repo_root)).await;
      let _ = this.update(cx, |this, cx| {
        this.reload_status(cx);
        if let Some(editor) = editor.clone() {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
      });
    });
    self.status_task = Some(task);
  }

  fn stage_file_action(&mut self, rel_path: PathBuf, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    let rel_path_for_job = rel_path.clone();
    let task = cx.spawn(async move |this, cx| {
      let _ = unblock(move || stage_file(&repo_root, &rel_path_for_job)).await;
      let _ = this.update(cx, |this, cx| {
        this.reload_status(cx);
        if this.selected_file.as_ref() == Some(&rel_path)
          && let Some(editor) = this.editor.clone()
        {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
      });
    });
    self.status_task = Some(task);
  }

  fn unstage_file_action(&mut self, rel_path: PathBuf, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    let rel_path_for_job = rel_path.clone();
    let task = cx.spawn(async move |this, cx| {
      let _ = unblock(move || unstage_file(&repo_root, &rel_path_for_job)).await;
      let _ = this.update(cx, |this, cx| {
        this.reload_status(cx);
        if this.selected_file.as_ref() == Some(&rel_path)
          && let Some(editor) = this.editor.clone()
        {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
      });
    });
    self.status_task = Some(task);
  }

  fn restore_file_action(
    &mut self,
    rel_path: PathBuf,
    status: RepoStatusKind,
    cx: &mut Context<Self>,
  ) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    let rel_path_for_job = rel_path.clone();
    let task = cx.spawn(async move |this, cx| {
      let _ = unblock(move || {
        if status == RepoStatusKind::Untracked {
          delete_untracked_file(&repo_root, &rel_path_for_job)
        } else {
          restore_file(&repo_root, &rel_path_for_job)
        }
      })
      .await;
      let _ = this.update(cx, |this, cx| {
        this.reload_status(cx);
        if this.selected_file.as_ref() == Some(&rel_path)
          && let Some(editor) = this.editor.clone()
        {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
      });
    });
    self.status_task = Some(task);
  }

  fn confirm_restore_file_action(
    &mut self,
    window: &mut Window,
    rel_path: PathBuf,
    status: RepoStatusKind,
    cx: &mut Context<Self>,
  ) {
    let file_label = rel_path.to_string_lossy().replace(['\n', '\r'], "");
    let (title, message, confirm_text) = if status == RepoStatusKind::Untracked {
      (
        "Delete file?",
        format!("Delete {} from disk?", file_label),
        "Delete",
      )
    } else {
      (
        "Restore file?",
        format!("Discard changes in {}?", file_label),
        "Restore",
      )
    };

    let title: SharedString = title.into();
    let message: SharedString = message.into();
    let confirm_text: SharedString = confirm_text.into();
    let view = cx.entity();
    let rel_path_for_action = rel_path.clone();

    window.open_dialog(cx, move |dialog, _, _| {
      let view = view.clone();
      let rel_path_for_action = rel_path_for_action.clone();
      ConfirmDialog::new(title.clone(), div().child(message.clone()))
        .confirm_text(confirm_text.clone())
        .cancel_text("Cancel")
        .destructive()
        .on_confirm(move |_, _, cx| {
          let rel_path_for_action = rel_path_for_action.clone();
          view.update(cx, |view, cx| {
            view.restore_file_action(rel_path_for_action, status, cx);
          });
          true
        })
        .build(dialog)
    });
  }

  fn stage_style(
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

  fn status_color(kind: RepoStatusKind, theme: &gpui_component::Theme) -> gpui::Hsla {
    match kind {
      RepoStatusKind::Modified => theme.status_orange(),
      RepoStatusKind::Added => theme.status_green(),
      RepoStatusKind::Deleted => theme.status_red(),
      RepoStatusKind::Renamed => theme.info,
      RepoStatusKind::TypeChange => theme.info,
      RepoStatusKind::Untracked => theme.status_green(),
      RepoStatusKind::Conflicted => theme.status_red(),
    }
  }

  fn push_flags(branch_status: Option<&BranchStatus>) -> (bool, bool) {
    let Some(status) = branch_status else {
      return (false, false);
    };
    if !status.has_upstream {
      return (false, false);
    }
    let can_push = status.ahead > 0 && status.behind == 0;
    let can_force_push = status.ahead > 0 && status.behind > 0;
    (can_push, can_force_push)
  }

  fn all_changes_staged(&self) -> bool {
    !self.status_entries.is_empty()
      && self
        .status_entries
        .iter()
        .all(|entry| entry.stage == RepoStage::Staged)
  }

  fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let select = Select::new(&self.repo_select)
      .placeholder("Select repository...")
      .menu_width(px(360.))
      .w(px(360.));

    let branch_info = self.branch_status.as_ref().map(|status| {
      let ahead = status.ahead;
      let behind = status.behind;
      let ahead_color = if ahead > 0 {
        theme.status_green()
      } else {
        theme.muted_foreground
      };
      let behind_color = if behind > 0 {
        theme.status_red()
      } else {
        theme.muted_foreground
      };

      div()
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .rounded(theme.radius)
        .bg(theme.background)
        .border_1()
        .border_color(theme.title_bar_border)
        .child(
          div()
            .text_sm()
            .text_color(theme.foreground)
            .child(status.name.clone()),
        )
        .child(
          div()
            .flex()
            .items_center()
            .gap_2()
            .child(
              div()
                .flex()
                .items_center()
                .gap_1()
                .child(
                  Icon::new(IconName::ArrowUp)
                    .size_3()
                    .text_color(ahead_color),
                )
                .child(
                  div()
                    .text_xs()
                    .text_color(ahead_color)
                    .child(ahead.to_string()),
                ),
            )
            .child(
              div()
                .flex()
                .items_center()
                .gap_1()
                .child(
                  Icon::new(IconName::ArrowDown)
                    .size_3()
                    .text_color(behind_color),
                )
                .child(
                  div()
                    .text_xs()
                    .text_color(behind_color)
                    .child(behind.to_string()),
                ),
            ),
        )
    });

    let header_left = div()
      .flex()
      .items_center()
      .gap_3()
      .child(
        div()
          .text_sm()
          .text_color(theme.foreground)
          .child("Repository"),
      )
      .child(select)
      .when_some(branch_info, |this, info| this.child(info));

    let settings_button = Button::new("open-settings")
      .icon(IconName::Settings2)
      .ghost()
      .compact()
      .tooltip("Settings")
      .on_click(|_, _, cx| {
        WorkspaceRoute::global_mut(cx).page = WorkspacePage::Settings;
        cx.refresh_windows();
      });

    div()
      .h(px(HEADER_HEIGHT))
      .px_4()
      .flex()
      .items_center()
      .justify_between()
      .bg(theme.sidebar)
      .border_b_1()
      .border_color(theme.title_bar_border)
      .child(header_left)
      .child(settings_button)
  }

  fn render_empty_state(&self, message: &str, cx: &mut Context<Self>) -> AnyElement {
    let message = message.to_string();
    let theme = cx.theme().clone();
    div()
      .size_full()
      .flex()
      .bg(theme.background)
      .items_center()
      .justify_center()
      .text_color(cx.theme().muted_foreground)
      .child(message)
      .into_any_element()
  }

  fn render_editor_header(&self, editor: &Entity<Editor>, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let editor_state = editor.read(cx);
    let file_name = editor_state
      .workdir_path
      .file_name()
      .and_then(|name| name.to_str())
      .unwrap_or("Untitled")
      .to_string();
    let file_dirty = editor_state.is_dirty;
    let editor_entity = editor.clone();

    let title = div()
      .flex()
      .items_center()
      .gap_2()
      .child(
        div()
          .text_sm()
          .font_medium()
          .text_color(theme.foreground)
          .child(file_name),
      )
      .when(file_dirty, |this| {
        this.child(
          div()
            .size_2()
            .rounded_full()
            .bg(theme.foreground)
            .flex_shrink_0(),
        )
      });

    let save_button = Button::new("editor-save")
      .label("Save")
      .xsmall()
      .ghost()
      .disabled(!file_dirty)
      .on_click(move |_, _, cx| {
        editor_entity.update(cx, |editor, cx| editor.save(cx));
      });

    let split_disabled = self
      .selected_file
      .as_ref()
      .map(|path| self.split_disabled_for_path(path))
      .unwrap_or(false);
    let (toggle_label, toggle_icon, toggle_tooltip) = if split_disabled {
      (
        "Split",
        IconName::PanelLeft,
        "Split diff unavailable for new files",
      )
    } else {
      match self.diff_view {
        DiffViewMode::Inline => ("Split", IconName::PanelLeft, "Switch to split diff"),
        DiffViewMode::Split => ("Inline", IconName::PanelLeftClose, "Switch to inline diff"),
      }
    };
    let view = cx.entity();
    let toggle_button = Button::new("editor-diff-toggle")
      .label(toggle_label)
      .icon(toggle_icon)
      .xsmall()
      .ghost()
      .tooltip(toggle_tooltip)
      .disabled(split_disabled)
      .on_click(move |_, _, cx| {
        view.update(cx, |this, cx| {
          this.toggle_diff_view(cx);
        });
      });

    div()
      .h(px(EDITOR_HEADER_HEIGHT))
      .px_3()
      .flex()
      .items_center()
      .justify_between()
      .bg(theme.sidebar)
      .border_b_1()
      .border_color(theme.title_bar_border)
      .child(title)
      .child(
        div()
          .flex()
          .items_center()
          .gap_2()
          .child(save_button)
          .child(toggle_button),
      )
      .into_any_element()
  }

  fn render_editor_with_overlay(
    &mut self,
    editor: Entity<Editor>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let overlay = self.render_change_block_actions(&editor, window, cx);
    let mut wrapper = div()
      .flex_1()
      .min_w(px(0.0))
      .min_h(px(0.0))
      .relative()
      .child(editor);

    if let Some(overlay) = overlay {
      wrapper = wrapper.child(overlay);
    }

    wrapper.into_any_element()
  }

  fn render_change_block_actions(
    &mut self,
    editor: &Entity<Editor>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Option<AnyElement> {
    let theme = cx.theme().clone();
    let editor_state = editor.read(cx);
    let hovered_id = editor_state.hovered_group_id.as_ref()?;
    let overlay = editor_state
      .visible_groups
      .iter()
      .find(|overlay| overlay.id.as_ref() == hovered_id.as_ref())?;

    let viewport_start = editor_state.scroll_offset_y.floor() as usize;
    if overlay.display_line < viewport_start {
      return None;
    }

    let line_height = window.line_height();
    let visible_lines = ((editor_state.viewport_height / line_height).ceil() as usize).max(1);
    let viewport_end = viewport_start + visible_lines;
    if overlay.display_line >= viewport_end {
      return None;
    }

    let top = line_height * (overlay.display_line - viewport_start) as f32;
    let file_dirty = editor_state.is_dirty;
    let restore_disabled_by_status = self
      .selected_file
      .as_ref()
      .and_then(|selected| {
        self
          .status_entries
          .iter()
          .find(|entry| &entry.path == selected)
      })
      .map(|entry| {
        matches!(
          entry.status,
          RepoStatusKind::Untracked | RepoStatusKind::Added
        )
      })
      .unwrap_or(false);
    let restore_disabled = file_dirty || restore_disabled_by_status;

    let stage_tooltip = if file_dirty {
      "File not saved"
    } else {
      "Stage hunk"
    };
    let unstage_tooltip = if file_dirty {
      "File not saved"
    } else {
      "Unstage hunk"
    };
    let restore_tooltip = if file_dirty {
      "File not saved"
    } else if restore_disabled_by_status {
      "Restore unavailable for added/untracked files"
    } else {
      "Restore hunk"
    };

    let group_id = overlay.id.clone();
    let state = overlay.state;
    let editor_entity = editor.clone();

    let mut actions = div().flex().items_center();

    match state {
      HunkState::Unstaged => {
        let editor_entity = editor_entity.clone();
        let group_id = group_id.clone();
        actions = actions.child(
          Button::new("stage-hunk")
            .icon(IconName::Plus)
            .label("Stage")
            .small()
            .tooltip(stage_tooltip)
            .rounded_t_none()
            .rounded_br_none()
            .bg(theme.background)
            .disabled(file_dirty)
            .on_click(move |_, _, cx| {
              let group_id = group_id.clone();
              editor_entity.update(cx, |editor, cx| {
                editor.enqueue_group_action(group_id, HunkAction::Stage, cx);
              });
            }),
        );
      }
      HunkState::Staged => {
        let editor_entity = editor_entity.clone();
        let group_id = group_id.clone();
        actions = actions.child(
          Button::new("unstage-hunk")
            .icon(IconName::Minus)
            .label("Unstage")
            .tooltip(unstage_tooltip)
            .small()
            .disabled(file_dirty)
            .bg(theme.background)
            .rounded_t_none()
            .on_click(move |_, _, cx| {
              let group_id = group_id.clone();
              editor_entity.update(cx, |editor, cx| {
                editor.enqueue_group_action(group_id, HunkAction::Unstage, cx);
              });
            }),
        );
      }
    }

    if matches!(state, HunkState::Unstaged) {
      let editor_entity = editor_entity.clone();
      let group_id = group_id.clone();
      actions = actions.child(
        Button::new("restore-hunk")
          .icon(IconName::Undo)
          .label("Restore")
          .rounded_t_none()
          .rounded_bl_none()
          .small()
          .bg(theme.background)
          .tooltip(restore_tooltip)
          .disabled(restore_disabled)
          .on_click(move |_, _, cx| {
            let group_id = group_id.clone();
            editor_entity.update(cx, |editor, cx| {
              editor.enqueue_group_action(group_id, HunkAction::Restore, cx);
            });
          }),
      );
    }

    Some(
      div()
        .absolute()
        .top(top)
        .right(px(30.0))
        .child(actions)
        .into_any_element(),
    )
  }

  fn render_commit_button(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let repo_ready = self.selected_repo.is_some();
    let commit_message = self.commit_input.read(cx).value();
    let commit_message_ready = !commit_message.trim().is_empty();
    let has_changes = !self.status_entries.is_empty();
    let commit_enabled = repo_ready && commit_message_ready && has_changes;
    let amend_enabled = repo_ready && self.has_head_commit;
    let undo_enabled = repo_ready && self.can_undo_last_commit;
    let push_enabled = repo_ready && self.can_push;
    let force_push_enabled = repo_ready && self.can_force_push;
    let menu_enabled = amend_enabled || undo_enabled || push_enabled || force_push_enabled;
    let view = cx.entity();
    let amend_view = view.clone();
    let undo_view = view.clone();
    let push_view = view.clone();
    let force_push_view = view.clone();

    let main_button = Button::new("commit-button-main")
      .label("Commit")
      .with_variant(ButtonVariant::Secondary)
      .outline()
      .flex_1()
      .rounded_r_none()
      .child(Kbd::new(Keystroke::parse("cmd-enter").unwrap()).ml_1())
      .disabled(!commit_enabled)
      .on_click(cx.listener(Self::commit_changes));

    let menu_button = Button::new("commit-button-menu")
      .icon(IconName::ChevronDown)
      .with_variant(ButtonVariant::Secondary)
      .outline()
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
                let _ = event;
                this.commit_amend_changes(window, cx);
              });
            }),
        );

        let menu = menu.item(
          PopupMenuItem::new("Undo last commit")
            .icon(IconName::Undo)
            .disabled(!undo_enabled)
            .on_click(move |event, window, cx| {
              undo_view.update(cx, |this, cx| {
                let _ = event;
                let _ = window;
                this.undo_last_commit_action(cx);
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
                let _ = event;
                let _ = window;
                this.push_changes_action(cx);
              });
            }),
        );

        menu.item(
          PopupMenuItem::new("Force push (with lease)")
            .icon(IconName::TriangleAlert)
            .disabled(!force_push_enabled)
            .on_click(move |event, window, cx| {
              force_push_view.update(cx, |this, cx| {
                let _ = event;
                let _ = window;
                this.force_push_changes_action(cx);
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

  fn render_commit_bar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let input = self.commit_input.clone();

    div()
      .w_full()
      .flex()
      .flex_col()
      .p_2()
      .gap_2()
      .border_t_1()
      .border_color(theme.border)
      .child(div().w_full().child(Input::new(&input)))
      .child(self.render_commit_button(cx))
  }

  fn render_sidebar_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme();
    let all_staged = self.all_changes_staged();
    let sidebar_enabled = self.selected_repo.is_some() && !self.status_entries.is_empty();
    let (label, icon, tooltip) = if all_staged {
      ("Unstage all", IconName::Minus, "Unstage all files")
    } else {
      ("Stage all", IconName::Plus, "Stage all files")
    };

    let group_label = div()
      .text_sm()
      .text_color(theme.sidebar_foreground)
      .child("Changes");

    div()
      .w_full()
      .flex()
      .px_3()
      .min_h(px(EDITOR_HEADER_HEIGHT))
      .border_b_1()
      .border_color(cx.theme().border)
      .items_center()
      .justify_between()
      .child(group_label)
      .child(
        Button::new("stage-all-button")
          .label(label)
          .icon(icon)
          .with_variant(ButtonVariant::Secondary)
          .xsmall()
          .disabled(!sidebar_enabled)
          .tooltip(tooltip)
          .on_click(cx.listener(Self::toggle_stage_all_action)),
      )
  }

  fn render_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let base_sidebar = div()
      .id("git-sidebar")
      .w_full()
      .h_full()
      .flex()
      .flex_col()
      .bg(theme.sidebar)
      .text_color(theme.sidebar_foreground);

    if self.selected_repo.is_none() {
      let placeholder = FileSidebarItem::placeholder("Select a repository", &cx.theme().clone());
      return base_sidebar
        .child(
          placeholder
            .render("git-sidebar-placeholder", window, cx)
            .into_any_element(),
        )
        .into_any_element();
    } else if self.status_entries.is_empty() {
      let placeholder = FileSidebarItem::placeholder("No changes", &cx.theme().clone());
      return base_sidebar
        .child(
          placeholder
            .render("git-sidebar-placeholder", window, cx)
            .into_any_element(),
        )
        .into_any_element();
    }

    let list = uniform_list(
      "git-sidebar-list",
      self.status_entries.len(),
      cx.processor(|this, range: std::ops::Range<usize>, window, cx| {
        let theme = cx.theme().clone();
        let selected_file = this.selected_file.clone();
        range
          .map(|ix| {
            let entry = &this.status_entries[ix];
            let is_active = selected_file.as_ref() == Some(&entry.path);
            let path = entry.path.clone();
            let path_for_open = path.clone();
            let status = entry.status;
            let status_letter = status.short_code();
            let file_label = entry.path.to_string_lossy();
            let file_label = file_label.replace(['\n', '\r'], "");
            let status_color = Self::status_color(status, &theme);
            let (stage_icon, stage_color, stage_tooltip) = Self::stage_style(entry.stage, &theme);
            let can_stage = matches!(
              entry.stage,
              RepoStage::Unstaged | RepoStage::PartiallyStaged
            );
            let can_unstage = matches!(entry.stage, RepoStage::Staged | RepoStage::PartiallyStaged);
            let can_restore = matches!(entry.stage, RepoStage::Unstaged);

            let mut item = FileSidebarItem::new(
              file_label,
              status_letter,
              status_color,
              stage_icon,
              stage_color,
              stage_tooltip,
            )
            .active(is_active)
            .on_click(cx.listener(move |this, _, _, cx| {
              this.open_file(path_for_open.clone(), cx);
            }));

            if can_stage {
              let path = path.clone();
              item = item.on_stage(cx.listener(move |this, _, _, cx| {
                this.stage_file_action(path.clone(), cx);
              }));
            }

            if can_unstage {
              let path = path.clone();
              item = item.on_unstage(cx.listener(move |this, _, _, cx| {
                this.unstage_file_action(path.clone(), cx);
              }));
            }

            if can_restore {
              let path = path.clone();
              let status = status;
              item = item.on_restore(cx.listener(move |this, _, window, cx| {
                this.confirm_restore_file_action(window, path.clone(), status, cx);
              }));
            }

            item
              .render(format!("git-sidebar-item-{}", ix), window, cx)
              .into_any_element()
          })
          .collect()
      }),
    )
    .size_full();

    let list_container = div()
      .relative()
      .flex_1()
      .min_h_0()
      .overflow_hidden()
      .child(list);

    base_sidebar
      .relative()
      .child(self.render_sidebar_header(cx))
      .child(
        div()
          .flex()
          .flex_col()
          .flex_1()
          .min_h_0()
          .child(list_container),
      )
      .child(self.render_commit_bar(cx))
      .into_any_element()
  }

  fn render_editor_area(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
    if self.selected_repo.is_none() {
      return self.render_empty_state("Select a repository to view changes", cx);
    }

    if let Some(editor) = self.editor.clone() {
      return div()
        .size_full()
        .flex()
        .flex_col()
        .child(self.render_editor_header(&editor, cx))
        .child(self.render_editor_with_overlay(editor, window, cx))
        .into_any_element();
    }

    self.render_empty_state("Select a file to view diff", cx)
  }
}

impl Render for GitPage {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    div()
      .size_full()
      .flex()
      .flex_col()
      .bg(cx.theme().background)
      .track_focus(&self.focus_handle(cx))
      .on_action(cx.listener(GitPage::show_command_palette_action))
      .on_action(cx.listener(GitPage::show_file_search_action))
      .on_action(cx.listener(GitPage::open_repository_action))
      .on_action(cx.listener(GitPage::commit_changes_action))
      .child(self.render_header(cx))
      .child(
        ui::h_resizable("git-page-split")
          .child(
            ui::resizable_panel()
              .size(px(SIDEBAR_DEFAULT_WIDTH))
              .size_range(px(SIDEBAR_MIN_WIDTH)..px(SIDEBAR_MAX_WIDTH))
              .child(self.render_sidebar(window, cx)),
          )
          .child(ui::resizable_panel().child(self.render_editor_area(window, cx))),
      )
  }
}

impl Focusable for GitPage {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}
