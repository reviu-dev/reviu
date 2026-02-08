use std::{
  path::{Path, PathBuf},
  rc::Rc,
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
  AnyElement, AnyWindowHandle, App, Context, Corner, Entity, FocusHandle, Focusable, Global,
  InteractiveElement, Keystroke, ParentElement, PathPromptOptions, Render, SharedString, Styled,
  Task, WeakEntity, Window, actions, div, img, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Disableable, Icon, IconName, IndexPath, Selectable, Sizable,
  button::{Button, ButtonVariant, ButtonVariants as _},
  h_flex,
  kbd::Kbd,
  label::Label,
  list::{List, ListDelegate, ListEvent, ListItem, ListState},
  menu::{DropdownMenu, PopupMenuItem},
  select::{SearchableVec, Select, SelectEvent, SelectItem, SelectState},
  text::TextView,
  tooltip::Tooltip,
};
use smol::unblock;

use crate::{
  api::ApiClient,
  auth_state::{AuthState, AuthStateStore},
  config::{ConfigStore, RecentRepository},
  github_page::GithubPageHandle,
  workspace::{WorkspaceApi, WorkspacePage, WorkspaceRoute},
};
use ui::{
  CommandPalette, CommandPaletteAction, CommandPaletteBranch, CommandPaletteBranchKind,
  CommandPaletteCommand, CommandPaletteConfig, CommandPaletteHandler, CommandPalettePage,
  ConfirmDialog,
  FILE_ICON_SIZE_PX, HEADER_HEIGHT, Input, InputState, SearchFileEntry, SearchFileHandler,
  SearchFilePalette, SearchFilePaletteConfig, StatusThemeExt, UserMenuConfig, UserMenuPage,
  UserMenuState, UserMenuUser, WindowExt, file_icon_path_for_path_with_theme, user_menu,
};

const SIDEBAR_DEFAULT_WIDTH: f32 = 300.0;
const SIDEBAR_MIN_WIDTH: f32 = 250.0;
const SIDEBAR_MAX_WIDTH: f32 = 700.0;
const STATUS_POLL_INTERVAL_MS: u64 = 800;
const EDITOR_HEADER_HEIGHT: f32 = 40.0;

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

#[derive(Clone, Debug)]
struct GitFileRow {
  entry: RepoStatusEntry,
  label: SharedString,
}

impl GitFileRow {
  fn new(entry: RepoStatusEntry) -> Self {
    let label = entry
      .path
      .to_string_lossy()
      .replace(['\n', '\r'], "")
      .into();
    Self { entry, label }
  }
}

struct GitFileListDelegate {
  rows: Vec<Rc<GitFileRow>>,
  selected_index: Option<IndexPath>,
  opened_path: Option<PathBuf>,
}

impl GitFileListDelegate {
  fn new() -> Self {
    Self {
      rows: Vec::new(),
      selected_index: None,
      opened_path: None,
    }
  }

  fn set_rows(&mut self, entries: Vec<RepoStatusEntry>) {
    self.rows = entries
      .into_iter()
      .map(|entry| Rc::new(GitFileRow::new(entry)))
      .collect();
  }

  fn row_at(&self, ix: IndexPath) -> Option<Rc<GitFileRow>> {
    self.rows.get(ix.row).cloned()
  }

  fn set_opened_path(&mut self, path: Option<PathBuf>) {
    self.opened_path = path;
  }
}

fn file_list_base_item(ix: IndexPath, selected_index: Option<IndexPath>) -> ListItem {
  ListItem::new(ix).selected(
    selected_index
      .map(|selected| selected.eq_row(ix))
      .unwrap_or(false),
  )
}

impl ListDelegate for GitFileListDelegate {
  type Item = ListItem;

  fn items_count(&self, _section: usize, _cx: &App) -> usize {
    self.rows.len()
  }

  fn render_item(
    &mut self,
    ix: IndexPath,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> Option<Self::Item> {
    let theme = cx.theme().clone();
    let mut base_item = file_list_base_item(ix, self.selected_index);
    let row = self.rows.get(ix.row)?;
    let is_opened = self
      .opened_path
      .as_ref()
      .map(|path| path == &row.entry.path)
      .unwrap_or(false);

    if is_opened {
      base_item = base_item.bg(theme.sidebar_accent.opacity(0.35));
    }

    let status_letter = row.entry.status.short_code();
    let status_color = GitPage::status_color(row.entry.status, &theme);
    let (stage_icon, stage_color, stage_tooltip) = GitPage::stage_style(row.entry.stage, &theme);
    let file_icon = file_icon_path_for_path_with_theme(&row.entry.path, &theme)
      .map(|path| img(path).size(px(FILE_ICON_SIZE_PX)).into_any_element())
      .unwrap_or_else(|| {
        Icon::new(IconName::File)
          .size_3()
          .text_color(theme.sidebar_foreground)
          .into_any_element()
      });

    let stage_icon = Icon::new(stage_icon).size_3().text_color(stage_color);
    let stage_element: AnyElement = if let Some(tooltip) = stage_tooltip {
      let tooltip_id = format!("git-stage-icon-{}", ix.row);
      div()
        .id(tooltip_id)
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .child(stage_icon)
        .into_any_element()
    } else {
      div().child(stage_icon).into_any_element()
    };

    Some(
      base_item.px_2().py_1().child(
        h_flex()
          .items_center()
          .gap_2()
          .child(
            div()
              .w(px(15.))
              .text_xs()
              .text_color(status_color)
              .child(status_letter),
          )
          .child(stage_element)
          .child(file_icon)
          .child(
            div()
              .flex_1()
              .overflow_hidden()
              .text_ellipsis_start()
              .when(row.entry.status == RepoStatusKind::Deleted, |this| {
                this.line_through()
              })
              .child(row.label.clone()),
          ),
      ),
    )
  }

  fn render_empty(
    &mut self,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> impl IntoElement {
    div()
      .size_full()
      .items_center()
      .justify_center()
      .text_sm()
      .text_color(cx.theme().muted_foreground)
      .child("No changes")
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

#[derive(Clone)]
struct RecentRepoItem {
  path: PathBuf,
  label: SharedString,
  is_selected: bool,
}

impl RecentRepoItem {
  fn new(repo: &RecentRepository, selected_repo: Option<&PathBuf>) -> Self {
    let label = repo.path.to_string_lossy().to_string();
    Self {
      path: repo.path.clone(),
      label: label.into(),
      is_selected: selected_repo.map_or(false, |selected| selected == &repo.path),
    }
  }
}

impl SelectItem for RecentRepoItem {
  type Value = PathBuf;

  fn title(&self) -> SharedString {
    self.label.clone()
  }

  fn render(&self, _: &mut Window, cx: &mut App) -> impl IntoElement {
    let icon = Icon::new(IconName::Check)
      .size_3()
      .text_color(cx.theme().accent_foreground)
      .when(!self.is_selected, |this| this.opacity(0.0));

    h_flex()
      .items_center()
      .gap_2()
      .child(
        div()
          .flex_1()
          .overflow_hidden()
          .text_ellipsis_start()
          .child(self.label.clone()),
      )
      .child(icon)
  }

  fn value(&self) -> &Self::Value {
    &self.path
  }

  fn matches(&self, query: &str) -> bool {
    self.label.to_lowercase().contains(&query.to_lowercase())
  }
}

#[derive(Clone)]
struct BranchSelectItem {
  branch: BranchRef,
  label: SharedString,
  is_current: bool,
}

impl BranchSelectItem {
  fn new(branch: BranchRef, is_current: bool) -> Self {
    let label: SharedString = branch.name.clone().into();
    Self {
      branch,
      label,
      is_current,
    }
  }
}

impl SelectItem for BranchSelectItem {
  type Value = BranchRef;

  fn title(&self) -> SharedString {
    self.label.clone()
  }

  fn render(&self, _: &mut Window, cx: &mut App) -> impl IntoElement {
    let icon = Icon::new(IconName::Check)
      .size_3()
      .text_color(cx.theme().accent_foreground)
      .when(!self.is_current, |this| this.opacity(0.0));

    h_flex()
      .items_center()
      .gap_2()
      .child(
        div()
          .flex_1()
          .overflow_hidden()
          .text_ellipsis()
          .child(self.label.clone()),
      )
      .child(icon)
  }

  fn value(&self) -> &Self::Value {
    &self.branch
  }

  fn matches(&self, query: &str) -> bool {
    self.label.to_lowercase().contains(&query.to_lowercase())
  }
}

#[derive(Clone, Default)]
pub struct AuthCallbackTarget {
  git_page: Option<WeakEntity<GitPage>>,
}

impl Global for AuthCallbackTarget {}

impl AuthCallbackTarget {
  pub fn register_git_page(cx: &mut Context<GitPage>) {
    cx.set_global(Self {
      git_page: Some(cx.entity().downgrade()),
    });
  }

  pub fn handle_auth_code(code: String, cx: &mut App) {
    let Some(weak) = cx.global::<Self>().git_page.clone() else {
      return;
    };
    let _ = weak.update(cx, |this, cx| this.handle_auth_code(code, cx));
  }

  pub fn start_sign_in(cx: &mut App) {
    let Some(weak) = cx.global::<Self>().git_page.clone() else {
      return;
    };
    let _ = weak.update(cx, |this, cx| this.start_github_sign_in(cx));
  }

  pub fn sign_out(cx: &mut App) {
    let Some(weak) = cx.global::<Self>().git_page.clone() else {
      return;
    };
    let _ = weak.update(cx, |this, cx| this.logout(cx));
  }
}

pub struct GitPage {
  focus_handle: FocusHandle,
  api: ApiClient,
  repo_select: Entity<SelectState<SearchableVec<RecentRepoItem>>>,
  branch_select: Entity<SelectState<SearchableVec<BranchSelectItem>>>,
  file_list: Entity<ListState<GitFileListDelegate>>,
  window_handle: AnyWindowHandle,
  selected_repo: Option<PathBuf>,
  status_entries: Vec<RepoStatusEntry>,
  branch_status: Option<BranchStatus>,
  has_head_commit: bool,
  can_undo_last_commit: bool,
  can_push: bool,
  can_force_push: bool,
  has_staged_changes: bool,
  selected_file: Option<PathBuf>,
  force_list_selection: bool,
  editor: Option<Entity<Editor>>,
  diff_view: DiffViewMode,
  show_markdown_preview: bool,
  auth_state: AuthState,
  auth_task: Option<Task<()>>,
  status_task: Option<Task<()>>,
  branch_task: Option<Task<()>>,
  poll_task: Option<Task<()>>,
  commit_input: Entity<InputState>,
}

impl GitPage {
  fn split_disabled_for_path(&self, rel_path: &Path) -> bool {
    self.status_entries.iter().any(|entry| {
      entry.path == rel_path
        && matches!(
          entry.status,
          RepoStatusKind::Untracked | RepoStatusKind::Added | RepoStatusKind::Deleted
        )
    })
  }

  fn is_markdown_path(path: &Path) -> bool {
    matches!(
      path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref(),
      Some("md" | "markdown" | "mdx")
    )
  }

  fn selected_file_is_markdown(&self) -> bool {
    self
      .selected_file
      .as_ref()
      .map(|path| Self::is_markdown_path(path))
      .unwrap_or(false)
  }

  fn effective_diff_view_for_path(&self, path: &Path) -> DiffViewMode {
    if self.show_markdown_preview && Self::is_markdown_path(path) {
      return DiffViewMode::Inline;
    }

    if self.split_disabled_for_path(path) {
      return DiffViewMode::Inline;
    }

    self.diff_view
  }

  fn sync_diff_view(&mut self, cx: &mut Context<Self>) {
    let Some(editor) = self.editor.clone() else {
      return;
    };
    let diff_view = if let Some(path) = self.selected_file.as_ref() {
      self.effective_diff_view_for_path(path)
    } else {
      self.diff_view
    };
    editor.update(cx, |editor, cx| editor.set_diff_view_mode(diff_view, cx));
  }

  fn selected_file_index(&self) -> Option<IndexPath> {
    let selected = self.selected_file.as_ref()?;
    let index = self
      .status_entries
      .iter()
      .position(|entry| &entry.path == selected)?;
    Some(IndexPath::new(index))
  }

  fn set_file_list_selected_index(&self, index: Option<IndexPath>, cx: &mut Context<Self>) {
    let file_list = self.file_list.clone();
    let window_handle = self.window_handle;
    let _ = cx.update_window(window_handle, move |_, window, cx| {
      file_list.update(cx, |state, cx| {
        state.set_selected_index(index, window, cx);
      });
    });
  }

  fn refresh_file_list(&mut self, cx: &mut Context<Self>) {
    let rows = self.status_entries.clone();
    let current_index = self.file_list.read(cx).selected_index();
    let opened_path = self.selected_file.clone();
    self.file_list.update(cx, |state, cx| {
      state.delegate_mut().set_rows(rows.clone());
      state.delegate_mut().set_opened_path(opened_path);
      cx.notify();
    });

    let selected_index = if self.force_list_selection {
      self.force_list_selection = false;
      self.selected_file_index()
    } else {
      current_index
        .and_then(|ix| (ix.row < rows.len()).then_some(IndexPath::new(ix.row)))
        .or_else(|| self.selected_file_index())
    };
    self.set_file_list_selected_index(selected_index, cx);
  }

  fn handle_auth_code(&mut self, code: String, cx: &mut Context<Self>) {
    let api = self.api.clone();
    let service = self.api.keychain_service().to_string();
    let username = self.api.keychain_username().to_string();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || api.exchange_code_for_token(&code)).await;
      match result {
        Ok(token) => {
          let secret = token.clone().into_bytes();
          let write_task = cx.update(|cx| cx.write_credentials(&service, &username, &secret));
          let _ = write_task.await;
          let _ = this.update(cx, |this, cx| {
            this.api.set_bearer_token(token);
            this.refresh_auth_state(cx);
          });
        }
        Err(_) => {
          let _ = this.update(cx, |this, cx| {
            this.set_auth_state(AuthState::Unauthenticated, cx);
          });
        }
      }
    });

    self.auth_task = Some(task);
  }

  fn logout(&mut self, cx: &mut Context<Self>) {
    let api = self.api.clone();
    let service = self.api.keychain_service().to_string();
    let task = cx.spawn(async move |this, cx| {
      let _ = unblock(move || api.sign_out()).await;
      let delete_task = cx.update(|cx| cx.delete_credentials(&service));
      let _ = delete_task.await;
      let _ = this.update(cx, |this, cx| {
        this.set_auth_state(AuthState::Unauthenticated, cx);
      });
    });

    self.auth_task = Some(task);
  }

  fn load_bearer_from_keychain(&mut self, cx: &mut Context<Self>) {
    let service = self.api.keychain_service().to_string();
    let task = cx.spawn(async move |this, cx| {
      let read_result = cx.update(|cx| cx.read_credentials(&service)).await;
      let _ = this.update(cx, |this, cx| {
        if let Ok(Some((_username, secret))) = read_result {
          if let Ok(token) = String::from_utf8(secret) {
            this.api.set_bearer_token(token);
            this.refresh_auth_state(cx);
            return;
          }
        }
        this.set_auth_state(AuthState::Unauthenticated, cx);
      });
    });

    self.auth_task = Some(task);
  }

  fn refresh_auth_state(&mut self, cx: &mut Context<Self>) {
    let api = self.api.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || api.fetch_me()).await;
      let _ = this.update(cx, |this, cx| {
        let state = match result {
          Ok(Some(user)) => AuthState::Authenticated(user),
          Ok(None) => AuthState::Unauthenticated,
          Err(_) => AuthState::Unauthenticated,
        };
        this.set_auth_state(state, cx);
      });
    });

    self.auth_task = Some(task);
  }

  fn start_github_sign_in(&mut self, cx: &mut Context<Self>) {
    let api = self.api.clone();
    let task = cx.spawn(async move |_, cx| {
      let result = unblock(move || api.sign_in_with_github()).await;
      if let Ok(Some(url)) = result {
        let _ = cx.update(|cx| cx.open_url(&url));
      }
    });

    self.auth_task = Some(task);
  }

  fn set_auth_state(&mut self, state: AuthState, cx: &mut Context<Self>) {
    self.auth_state = state.clone();
    AuthStateStore::set(cx, state);
    cx.refresh_windows();
    cx.notify();
  }

  pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
    let recent = ConfigStore::load_recent_repositories();
    let selected_repo = recent.first().map(|repo| repo.path.clone());
    let items: Vec<RecentRepoItem> = recent
      .iter()
      .map(|repo| RecentRepoItem::new(repo, selected_repo.as_ref()))
      .collect();
    let repo_select =
      cx.new(|cx| SelectState::new(SearchableVec::new(items), None, window, cx).searchable(true));
    let branch_select = cx.new(|cx| {
      SelectState::new(
        SearchableVec::new(Vec::<BranchSelectItem>::new()),
        None,
        window,
        cx,
      )
      .searchable(true)
    });
    let file_list = cx.new(|cx| ListState::new(GitFileListDelegate::new(), window, cx));

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
      api: WorkspaceApi::global(cx).api.clone(),
      repo_select,
      branch_select,
      file_list,
      window_handle: window.window_handle(),
      selected_repo,
      status_entries: Vec::new(),
      branch_status: None,
      has_head_commit: false,
      can_undo_last_commit: false,
      can_push: false,
      can_force_push: false,
      has_staged_changes: false,
      selected_file: None,
      force_list_selection: false,
      editor: None,
      diff_view: DiffViewMode::Inline,
      show_markdown_preview: false,
      auth_state: AuthState::Unknown,
      auth_task: None,
      status_task: None,
      branch_task: None,
      poll_task: None,
      commit_input,
    };

    view.subscribe_to_repo_select(cx);
    view.subscribe_to_branch_select(cx);
    view.subscribe_to_file_list(cx);
    view.reload_status(cx);
    view.refresh_branches(cx);
    view.start_polling(cx);
    view.load_bearer_from_keychain(cx);
    AuthCallbackTarget::register_git_page(cx);

    view
  }

  fn subscribe_to_repo_select(&mut self, cx: &mut Context<Self>) {
    cx.subscribe(
      &self.repo_select,
      move |this, _state, event: &SelectEvent<SearchableVec<RecentRepoItem>>, cx| {
        if let SelectEvent::Confirm(Some(repo)) = event {
          this.set_selected_repo(repo.clone(), cx);
        }
      },
    )
    .detach();
  }

  fn subscribe_to_branch_select(&mut self, cx: &mut Context<Self>) {
    cx.subscribe(
      &self.branch_select,
      move |this, _state, event: &SelectEvent<SearchableVec<BranchSelectItem>>, cx| {
        let SelectEvent::Confirm(Some(branch)) = event else {
          return;
        };
        let Some(repo_root) = this.selected_repo.clone() else {
          return;
        };
        let branch = branch.clone();
        let editor = this.editor.clone();

        let task = cx.spawn(async move |this, cx| {
          let result = unblock(move || switch_branch(&repo_root, &branch)).await;
          let _ = this.update(cx, |this, cx| {
            if result.is_ok() {
              this.reload_status(cx);
              this.refresh_branches(cx);
              if let Some(editor) = editor.clone() {
                editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
              }
            }
          });
        });

        this.branch_task = Some(task);
      },
    )
    .detach();
  }

  fn subscribe_to_file_list(&mut self, cx: &mut Context<Self>) {
    cx.subscribe(
      &self.file_list,
      move |this, state, event: &ListEvent, cx| match event {
        ListEvent::Select(ix) | ListEvent::Confirm(ix) => {
          let row = state.read(cx).delegate().row_at(*ix);
          if let Some(row) = row {
            this.open_file(row.entry.path.clone(), cx);
          }
        }
        ListEvent::Cancel => {}
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
    self.refresh_branches(cx);
    self.refresh_repo_select(cx);
    cx.notify();
  }

  fn refresh_repo_select(&mut self, cx: &mut Context<Self>) {
    let recent = ConfigStore::load_recent_repositories();
    let selected_repo = self.selected_repo.clone();
    let items: Vec<RecentRepoItem> = recent
      .iter()
      .map(|repo| RecentRepoItem::new(repo, selected_repo.as_ref()))
      .collect();
    let window_handle = self.window_handle;

    let select = cx.update_window(window_handle, |_, window, cx| {
      let select =
        cx.new(|cx| SelectState::new(SearchableVec::new(items), None, window, cx).searchable(true));
      if let Some(repo) = selected_repo.as_ref() {
        select.update(cx, |state, cx| {
          state.set_selected_value(repo, window, cx);
        });
      }
      select
    });

    if let Ok(select) = select {
      self.repo_select = select;
      self.subscribe_to_repo_select(cx);
    }
  }

  fn refresh_branches(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      let branch_select = self.branch_select.clone();
      let window_handle = self.window_handle;
      let _ = cx.update_window(window_handle, |_, window, cx| {
        branch_select.update(cx, |state, cx| {
          state.set_items(
            SearchableVec::new(Vec::<BranchSelectItem>::new()),
            window,
            cx,
          );
          state.set_selected_index(None, window, cx);
        });
      });
      return;
    };

    let window_handle = self.window_handle;
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || {
        let branches = list_branches(&repo_root).ok()?;
        let current = current_branch_status(&repo_root).ok();
        Some((branches, current))
      })
      .await;
      let Some((branches, current)) = result else {
        return;
      };

      let selected = current.and_then(|status| {
        if status.name == "HEAD" {
          None
        } else {
          Some(BranchRef {
            name: status.name,
            kind: BranchKind::Local,
          })
        }
      });

      let items = branches
        .into_iter()
        .map(|branch| {
          let is_current = selected
            .as_ref()
            .map_or(false, |current| current == &branch);
          BranchSelectItem::new(branch, is_current)
        })
        .collect::<Vec<_>>();

      let select = cx.update_window(window_handle, |_, window, cx| {
        let select = cx
          .new(|cx| SelectState::new(SearchableVec::new(items), None, window, cx).searchable(true));
        if let Some(selected) = selected.as_ref() {
          select.update(cx, |state, cx| {
            state.set_selected_value(selected, window, cx);
          });
        }
        select
      });

      let _ = this.update(cx, |this, cx| {
        if let Ok(select) = select {
          this.branch_select = select;
          this.subscribe_to_branch_select(cx);
        }
      });
    });

    self.branch_task = Some(task);
  }

  fn reload_status(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      self.status_entries.clear();
      self.refresh_file_list(cx);
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
          } else {
            this.sync_diff_view(cx);
          }
        }
        this.refresh_file_list(cx);
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
          this.refresh_file_list(cx);
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

  fn start_open_repository(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
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
            ConfigStore::persist_recent_repository(&path);
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

    let include_github = matches!(self.auth_state, AuthState::Authenticated(_));
    let mut commands =
      CommandPaletteCommand::default_global_commands(CommandPalettePage::Git, include_github);
    commands.push(CommandPaletteCommand::switch_branch());
    commands.push(CommandPaletteCommand::merge_branch());
    let config = CommandPaletteConfig::new(palette_branches, commands, handler);

    let palette = cx.new(|cx| CommandPalette::new(window, cx, config));
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
    let handler: SearchFileHandler = Arc::new(move |path, window, cx| {
      view.update(cx, |view, cx| {
        view.open_file(path, cx);
      });

      let view_for_focus = view.clone();
      window.on_next_frame(move |window, cx| {
        if let Some(editor) = view_for_focus.read(cx).editor.clone() {
          let focus_handle: FocusHandle = editor.read(cx).focus_handle(cx);
          window.focus(&focus_handle, cx);
        } else {
          let focus_handle = view_for_focus.read(cx).focus_handle(cx);
          window.focus(&focus_handle, cx);
        }
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

    let mut selected_branch: Option<BranchRef> = None;
    let result = match action {
      CommandPaletteAction::OpenGitPage => {
        WorkspaceRoute::global_mut(cx).page = WorkspacePage::Git;
        cx.refresh_windows();
        Ok(())
      }
      CommandPaletteAction::OpenGithubPage => {
        GithubPageHandle::refresh(cx);
        WorkspaceRoute::global_mut(cx).page = WorkspacePage::Github;
        cx.refresh_windows();
        Ok(())
      }
      CommandPaletteAction::OpenSettingsPage => {
        WorkspaceRoute::open_settings(cx);
        cx.refresh_windows();
        Ok(())
      }
      CommandPaletteAction::SwitchBranch(branch) => {
        let branch_ref = BranchRef {
          name: branch.name.to_string(),
          kind: match branch.kind {
            CommandPaletteBranchKind::Local => BranchKind::Local,
            CommandPaletteBranchKind::Remote => BranchKind::Remote,
          },
        };
        selected_branch = Some(branch_ref.clone());
        switch_branch(&root_path, &branch_ref)
      }
      CommandPaletteAction::CreateBranch { name } => {
        let branch_ref = BranchRef {
          name: name.clone(),
          kind: BranchKind::Local,
        };
        selected_branch = Some(branch_ref.clone());
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
        selected_branch = Some(new_branch.clone());
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

    if let Some(selected_branch) = selected_branch {
      let branch_select = self.branch_select.clone();
      let window_handle = self.window_handle;
      let _ = cx.update_window(window_handle, |_, window, cx| {
        branch_select.update(cx, |state, cx| {
          state.set_selected_value(&selected_branch, window, cx);
        });
      });
    }

    self.reload_status(cx);
    self.refresh_branches(cx);
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
    let is_markdown = Self::is_markdown_path(&rel_path);
    if !is_markdown {
      self.show_markdown_preview = false;
    }
    let file_path = repo_root.join(&rel_path);
    let editor = cx.new(|cx| Editor::new_with_paths(repo_root, file_path, cx));
    let diff_view = self.effective_diff_view_for_path(&rel_path);
    editor.update(cx, |editor, cx| editor.set_diff_view_mode(diff_view, cx));
    self.editor = Some(editor);
    self.selected_file = Some(rel_path);
    self.force_list_selection = true;
    let opened_path = self.selected_file.clone();
    self.file_list.update(cx, |state, cx| {
      state.delegate_mut().set_opened_path(opened_path);
      cx.notify();
    });
    let selected_index = self.selected_file_index();
    self.set_file_list_selected_index(selected_index, cx);
    cx.notify();
  }

  fn toggle_diff_view(&mut self, cx: &mut Context<Self>) {
    if let Some(selected) = self.selected_file.as_ref()
      && self.split_disabled_for_path(selected)
    {
      return;
    }
    if self.show_markdown_preview {
      self.show_markdown_preview = false;
    }
    self.diff_view = match self.diff_view {
      DiffViewMode::Inline => DiffViewMode::Split,
      DiffViewMode::Split => DiffViewMode::Inline,
    };
    self.sync_diff_view(cx);
    cx.notify();
  }

  fn toggle_markdown_preview(&mut self, cx: &mut Context<Self>) {
    if !self.selected_file_is_markdown() {
      self.show_markdown_preview = false;
      self.sync_diff_view(cx);
      cx.notify();
      return;
    }

    self.show_markdown_preview = !self.show_markdown_preview;
    self.sync_diff_view(cx);
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
      .search_placeholder("Search repositories...")
      .menu_width(px(360.))
      .w(px(360.));

    let branch_select = Select::new(&self.branch_select)
      .placeholder("Select branch...")
      .search_placeholder("Search branches...")
      .menu_width(px(300.))
      .w(px(240.))
      .disabled(self.selected_repo.is_none());

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
      .child(div().text_sm().text_color(theme.foreground).child("Branch"))
      .child(branch_select)
      .when_some(branch_info, |this, info| this.child(info));

    let settings_button = Button::new("open-settings")
      .icon(IconName::Settings2)
      .ghost()
      .compact()
      .tooltip("Settings")
      .on_click(|_, _, cx| {
        WorkspaceRoute::open_settings(cx);
        cx.refresh_windows();
      });

    let menu_state = match &self.auth_state {
      AuthState::Unknown => UserMenuState::Unknown,
      AuthState::Unauthenticated => UserMenuState::Unauthenticated,
      AuthState::Authenticated(user) => {
        let display_name = if user.name.trim().is_empty() {
          user.email.clone()
        } else {
          user.name.clone()
        };
        UserMenuState::Authenticated(UserMenuUser {
          name: display_name.into(),
          email: user.email.clone().into(),
          image: user.image.clone().map(Into::into),
        })
      }
    };

    let view = cx.entity();
    let open_git = Rc::new(|_window: &mut Window, cx: &mut App| {
      let cx = &mut *cx;
      WorkspaceRoute::global_mut(cx).page = WorkspacePage::Git;
      cx.refresh_windows();
    });
    let open_github = Rc::new(|_window: &mut Window, cx: &mut App| {
      let cx = &mut *cx;
      GithubPageHandle::refresh(cx);
      WorkspaceRoute::global_mut(cx).page = WorkspacePage::Github;
      cx.refresh_windows();
    });
    let open_settings = Rc::new(|_window: &mut Window, cx: &mut App| {
      let cx = &mut *cx;
      WorkspaceRoute::open_settings(cx);
      cx.refresh_windows();
    });
    let sign_in = Rc::new(move |_window: &mut Window, cx: &mut App| {
      let _ = view.update(cx, |this, cx| this.start_github_sign_in(cx));
    });
    let view = cx.entity();
    let sign_out = Rc::new(move |_window: &mut Window, cx: &mut App| {
      let _ = view.update(cx, |this, cx| this.logout(cx));
    });

    let auth_control = user_menu(UserMenuConfig {
      id: "auth-menu".into(),
      state: menu_state,
      current_page: UserMenuPage::Git,
      on_open_git: Some(open_git),
      on_open_github: Some(open_github),
      on_open_settings: Some(open_settings),
      on_sign_in: Some(sign_in),
      on_sign_out: Some(sign_out),
    });

    let header_right = h_flex()
      .items_center()
      .gap_2()
      .when_some(auth_control, |this, control| this.child(control))
      .when(
        !matches!(self.auth_state, AuthState::Authenticated(_)),
        |this| this.child(settings_button),
      );

    div()
      .h(px(HEADER_HEIGHT))
      .max_h(px(HEADER_HEIGHT))
      .px_3()
      .flex()
      .items_center()
      .justify_between()
      .bg(theme.sidebar)
      .border_b_1()
      .border_color(theme.title_bar_border)
      .child(header_left)
      .child(header_right)
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
    let (file_name, dir_path) = if let Some(rel_path) = self.selected_file.as_ref() {
      let file_name = rel_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Untitled")
        .to_string();
      let dir_path = rel_path
        .parent()
        .and_then(|parent| parent.to_str())
        .unwrap_or("")
        .to_string();
      (file_name, dir_path)
    } else {
      let file_name = editor_state
        .workdir_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Untitled")
        .to_string();
      let dir_path = editor_state
        .workdir_path
        .parent()
        .and_then(|parent| parent.to_str())
        .unwrap_or("")
        .to_string();
      (file_name, dir_path)
    };
    let file_dirty = editor_state.is_dirty;
    let editor_entity = editor.clone();
    let selected_entry = self
      .selected_file
      .as_ref()
      .and_then(|path| self.status_entries.iter().find(|entry| &entry.path == path))
      .cloned();
    let status_letter = selected_entry
      .as_ref()
      .map(|entry| entry.status.short_code());
    let status_color = selected_entry
      .as_ref()
      .map(|entry| Self::status_color(entry.status, &theme))
      .unwrap_or(theme.muted_foreground);

    let title = h_flex()
      .items_center()
      .gap_2()
      .min_w_0()
      .flex_1()
      .when_some(status_letter, |this, letter| {
        this.child(
          div()
            .w(px(15.))
            .text_xs()
            .text_color(status_color)
            .child(letter),
        )
      })
      .child(
        file_icon_path_for_path_with_theme(&editor_state.workdir_path, &theme)
          .map(|path| img(path).size(px(FILE_ICON_SIZE_PX)).into_any_element())
          .unwrap_or_else(|| {
            Icon::new(IconName::File)
              .size_3()
              .text_color(theme.foreground)
              .into_any_element()
          }),
      )
      .child(
        h_flex()
          .min_w_0()
          .flex_1()
          .items_center()
          .gap_2()
          .child(
            h_flex()
              .min_w_0()
              .items_center()
              .gap_2()
              .child(div().min_w_0().child(Label::new(file_name).truncate()))
              .when(file_dirty, |this| {
                this.child(
                  div()
                    .size_2()
                    .rounded_full()
                    .bg(theme.foreground)
                    .flex_shrink_0(),
                )
              }),
          )
          .when(!dir_path.is_empty(), |this| {
            this.child(
              div()
                .min_w_0()
                .flex_1()
                .overflow_hidden()
                .text_ellipsis_start()
                .text_color(theme.muted_foreground)
                .child(format!("- {}", dir_path)),
            )
          }),
      );

    let save_button = Button::new("editor-save")
      .label("Save")
      .xsmall()
      .ghost()
      .disabled(!file_dirty)
      .on_click(move |_, _, cx| {
        editor_entity.update(cx, |editor, cx| editor.save(cx));
      });

    let is_markdown = self.selected_file_is_markdown();
    let preview_active = is_markdown && self.show_markdown_preview;
    let split_disabled = self
      .selected_file
      .as_ref()
      .map(|path| self.split_disabled_for_path(path))
      .unwrap_or(false)
      || preview_active;
    let (toggle_label, toggle_icon) = if split_disabled {
      ("Split", IconName::PanelLeft)
    } else {
      match self.diff_view {
        DiffViewMode::Inline => ("Split", IconName::PanelLeft),
        DiffViewMode::Split => ("Inline", IconName::PanelLeftClose),
      }
    };
    let view = cx.entity();
    let toggle_button = Button::new("editor-diff-toggle")
      .label(toggle_label)
      .icon(toggle_icon)
      .xsmall()
      .ghost()
      .disabled(split_disabled)
      .on_click(move |_, _, cx| {
        view.update(cx, |this, cx| {
          this.toggle_diff_view(cx);
        });
      });

    let view = cx.entity();
    let preview_button = Button::new("editor-markdown-preview")
      .label("Preview")
      .icon(if preview_active {
        IconName::EyeOff
      } else {
        IconName::Eye
      })
      .xsmall()
      .ghost()
      .selected(preview_active)
      .on_click(move |_, _, cx| {
        view.update(cx, |this, cx| {
          this.toggle_markdown_preview(cx);
        });
      });

    let (can_stage, can_unstage, can_restore, file_path, file_status) =
      if let Some(entry) = selected_entry {
        (
          matches!(
            entry.stage,
            RepoStage::Unstaged | RepoStage::PartiallyStaged
          ),
          matches!(entry.stage, RepoStage::Staged | RepoStage::PartiallyStaged),
          matches!(entry.stage, RepoStage::Unstaged),
          Some(entry.path.clone()),
          Some(entry.status),
        )
      } else {
        (false, false, false, None, None)
      };

    let file_path_stage = file_path.clone();
    let file_path_unstage = file_path.clone();
    let file_path_restore = file_path.clone();

    let view = cx.entity();
    let stage_button = Button::new("editor-stage-file")
      .label("Stage")
      .icon(IconName::Plus)
      .xsmall()
      .ghost()
      .disabled(!can_stage)
      .on_click(move |_, _, cx| {
        if let Some(path) = file_path_stage.clone() {
          view.update(cx, |this, cx| {
            this.stage_file_action(path.clone(), cx);
          });
        }
      });

    let view = cx.entity();
    let unstage_button = Button::new("editor-unstage-file")
      .label("Unstage")
      .icon(IconName::Minus)
      .xsmall()
      .ghost()
      .disabled(!can_unstage)
      .on_click(move |_, _, cx| {
        if let Some(path) = file_path_unstage.clone() {
          view.update(cx, |this, cx| {
            this.unstage_file_action(path.clone(), cx);
          });
        }
      });

    let view = cx.entity();
    let file_status_for_restore = file_status;
    let restore_button = Button::new("editor-restore-file")
      .label("Restore")
      .icon(IconName::Undo)
      .xsmall()
      .ghost()
      .disabled(!can_restore)
      .on_click(move |_, window, cx| {
        if let (Some(path), Some(status)) = (file_path_restore.clone(), file_status_for_restore) {
          view.update(cx, |this, cx| {
            this.confirm_restore_file_action(window, path.clone(), status, cx);
          });
        }
      });

    div()
      .min_h(px(EDITOR_HEADER_HEIGHT))
      .h(px(EDITOR_HEADER_HEIGHT))
      .px_3()
      .flex()
      .items_center()
      .justify_between()
      .gap_2()
      .bg(theme.sidebar)
      .border_b_1()
      .border_color(theme.title_bar_border)
      .child(title)
      .child(
        div()
          .flex()
          .items_center()
          .gap_2()
          .flex_shrink_0()
          .child(stage_button)
          .child(unstage_button)
          .child(restore_button)
          .child(save_button)
          .when(is_markdown, |this| this.child(preview_button))
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
    let selected_status = self
      .selected_file
      .as_ref()
      .and_then(|selected| {
        self
          .status_entries
          .iter()
          .find(|entry| &entry.path == selected)
      })
      .map(|entry| entry.status);

    if matches!(
      selected_status,
      Some(RepoStatusKind::Untracked | RepoStatusKind::Added)
    ) {
      return None;
    }

    let restore_disabled_by_status = matches!(
      selected_status,
      Some(RepoStatusKind::Untracked | RepoStatusKind::Added)
    );
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

  fn render_sidebar(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
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
      return base_sidebar
        .child(
          div()
            .p_4()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child("Select a repository"),
        )
        .into_any_element();
    }

    let list_container = div().relative().flex_1().min_h_0().overflow_hidden().child(
      List::new(&self.file_list)
        .flex_1()
        .w_full()
        .min_h_0()
        .p(px(6.)),
    );

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

    let theme = cx.theme().clone();
    if let Some(editor) = self.editor.clone() {
      let editor_view = self.render_editor_with_overlay(editor.clone(), window, cx);
      if self.show_markdown_preview && self.selected_file_is_markdown() {
        let markdown = editor.read(cx).document().read(cx);
        let markdown = markdown.slice_to_string(0..markdown.len());
        let preview = div()
          .flex_1()
          .min_h_0()
          .min_w(px(0.0))
          .bg(theme.background)
          .occlude()
          .child(
            TextView::markdown("markdown-preview", markdown)
              .selectable(true)
              .scrollable(true)
              .pb_4()
              .px_4(),
          );

        return div()
          .size_full()
          .flex()
          .flex_col()
          .child(self.render_editor_header(&editor, cx))
          .child(
            ui::h_resizable("git-page-markdown-preview")
              .child(ui::resizable_panel().child(editor_view))
              .child(ui::resizable_panel().child(preview)),
          )
          .into_any_element();
      }

      return div()
        .size_full()
        .flex()
        .flex_col()
        .child(self.render_editor_header(&editor, cx))
        .child(editor_view)
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
