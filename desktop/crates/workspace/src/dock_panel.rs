//! The right dock of the shell: changes, files, history, pull request, terminal.

#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;

use git::{
  HeadCommitStatus, RepoStage, RepoStatusEntry, commit_changes, current_branch_status,
  current_github_remote_repo, head_commit_status, is_merge_in_progress, is_rebase_in_progress,
  list_repo_status, list_repo_worktree_files, stage_all,
};
use gpui::{
  Anchor, AnyElement, AnyWindowHandle, App, Context, Entity, FocusHandle, Focusable, Render,
  SharedString, Task, Window, div, img, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, h_flex,
  menu::{DropdownMenu as _, PopupMenuItem},
  tree::{TreeItem, TreeState, tree},
  v_flex,
};
use terminal::TerminalView;

use crate::changes_list::{ChangesList, ChangesListEvent};
use crate::history_list::{HistoryList, HistoryListEvent};
use crate::repo_state::{PaletteCommand, RepoState, push_flags, should_publish_branch};

const DOCK_PANEL_TERMINAL_DEBUG_SELECTOR: &str = "dock-panel-terminal";
pub(crate) const DOCK_PANEL_HISTORY_DEBUG_SELECTOR: &str = "dock-panel-history";
const DOCK_PANEL_COMMIT_DEBUG_SELECTOR: &str = "dock-panel-commit";
const DOCK_PANEL_COMMIT_MENU_DEBUG_SELECTOR: &str = "dock-panel-commit-menu";
const DOCK_PANEL_CREATE_PR_DEBUG_SELECTOR: &str = "dock-panel-create-pr";
const DOCK_PANEL_PUBLISH_AND_CREATE_PR_DEBUG_SELECTOR: &str = "dock-panel-publish-and-create-pr";
const DOCK_PANEL_COMPARE_DEBUG_SELECTOR: &str = "dock-panel-compare-on-github";
const DOCK_PANEL_OPERATION_DEBUG_SELECTOR: &str = "dock-panel-operation";
#[cfg(test)]
const DOCK_PANEL_TERMINAL_TAB_DEBUG_SELECTOR: &str = "dock-panel-tab-terminal";
#[cfg(test)]
const DOCK_PANEL_HISTORY_TAB_DEBUG_SELECTOR: &str = "dock-panel-tab-history";
const DOCK_PANEL_REFRESH_DEBUG_SELECTOR: &str = "dock-panel-refresh";
use std::collections::BTreeMap;
use std::rc::Rc;

use crate::api::GithubPullRequest;
use crate::auth_state::AuthStateStore;
use crate::github_navigation::{open_compare_target, open_pr_target};
use crate::github_shared::{pull_request_status_color, pull_request_status_label};
use crate::pull_request_dialog::{
  GithubBranchContext, PullRequestCreatedHandler, open_create_pull_request_dialog,
};
use crate::workspace::WorkspaceApi;
use ui::{
  Button, ButtonVariants as _, CommandPaletteCommand, StatusThemeExt as _, Textarea, TextareaState,
  UiIconName,
};

#[derive(Clone, Debug)]
pub enum DockPanelEvent {
  OpenFile {
    path: PathBuf,
  },
  /// A file as it was in a commit, read-only.
  OpenCommitFile {
    commit_oid: String,
    path: PathBuf,
  },
  /// A commit landed: whoever shows the branch state has to refresh it.
  Committed,
  /// The rebase can move on: the host runs it, it owns the conflict flow.
  ContinueRebase,
  /// A command picked in the commit menu; the host owns running it.
  RunCommand(CommitMenuCommand),
  /// An unpublished branch: push it, then open the pull request form.
  PublishBranchAndCreatePullRequest(GithubBranchContext),
  /// The working tree was re-read: whoever shows a file has to look again.
  StatusRefreshed,
}

impl gpui::EventEmitter<DockPanelEvent> for DockPanel {}

/// What the menu next to the commit button offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitMenuCommand {
  Amend,
  UndoLastCommit,
  Push,
  ForcePush,
}

impl CommitMenuCommand {
  fn label(self) -> &'static str {
    match self {
      Self::Amend => "Amend",
      Self::UndoLastCommit => "Undo last commit",
      Self::Push => "Push",
      Self::ForcePush => "Force push (with lease)",
    }
  }

  fn icon(self) -> IconName {
    match self {
      Self::Amend => IconName::Replace,
      Self::UndoLastCommit => IconName::Undo,
      Self::Push | Self::ForcePush => IconName::ArrowUp,
    }
  }

  fn rule(self) -> PaletteCommand {
    match self {
      Self::Amend => PaletteCommand::Amend,
      Self::UndoLastCommit => PaletteCommand::UndoLastCommit,
      Self::Push => PaletteCommand::Push,
      Self::ForcePush => PaletteCommand::ForcePush,
    }
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockPanelTab {
  Changes,
  Files,
  History,
  PullRequest,
  Terminal,
}

/// Nested tree items from repo-relative paths. File ids are the relative path;
/// directory ids get a trailing slash so they never collide with file ids.
pub(crate) fn build_worktree_tree_items(files: &[PathBuf]) -> Vec<TreeItem> {
  #[derive(Default)]
  struct Node {
    dirs: BTreeMap<String, Node>,
    files: Vec<String>,
  }

  let mut root = Node::default();
  for file in files {
    let components: Vec<String> = file
      .components()
      .map(|component| component.as_os_str().to_string_lossy().into_owned())
      .collect();
    let Some((file_name, dirs)) = components.split_last() else {
      continue;
    };
    let mut node = &mut root;
    for dir in dirs {
      node = node.dirs.entry(dir.clone()).or_default();
    }
    node.files.push(file_name.clone());
  }

  fn items_for(node: &Node, prefix: &str) -> Vec<TreeItem> {
    let mut items = Vec::new();
    for (name, child) in &node.dirs {
      let child_prefix = format!("{prefix}{name}/");
      items.push(
        TreeItem::new(child_prefix.clone(), name.clone()).children(items_for(child, &child_prefix)),
      );
    }
    let mut files = node.files.clone();
    files.sort();
    for name in files {
      items.push(TreeItem::new(format!("{prefix}{name}"), name));
    }
    items
  }

  items_for(&root, "")
}

#[derive(Clone, Debug)]
pub(crate) enum BranchPrState {
  NoAccess,
  NoRemote,
  Loading,
  Missing(GithubBranchContext),
  Found(GithubBranchContext, Box<GithubPullRequest>),
}

fn branch_pr_state_for_lookup(
  remote: Option<git::GithubRemoteRepo>,
  branch: Option<String>,
  fetch: impl FnOnce(&GithubBranchContext) -> anyhow::Result<Option<GithubPullRequest>>,
) -> BranchPrState {
  let Some(remote) = remote else {
    return BranchPrState::NoRemote;
  };
  let Some(branch) = branch else {
    return BranchPrState::NoRemote;
  };
  let context = GithubBranchContext {
    owner: remote.owner,
    repo: remote.repo,
    branch,
  };
  match fetch(&context) {
    Ok(Some(pull_request)) => BranchPrState::Found(context, Box::new(pull_request)),
    Ok(None) => BranchPrState::Missing(context),
    // Keep the tab usable on transient API errors: offer Create against the context.
    Err(_) => BranchPrState::Missing(context),
  }
}

pub struct DockPanel {
  focus_handle: FocusHandle,
  window_handle: AnyWindowHandle,
  repo_root: Option<PathBuf>,
  status_entries: Vec<RepoStatusEntry>,
  merge_in_progress: bool,
  rebase_in_progress: bool,
  head_status: HeadCommitStatus,
  branch_status: Option<git::BranchStatus>,
  commit_input: Entity<TextareaState>,
  committing: bool,
  last_error: Option<SharedString>,
  active_tab: DockPanelTab,
  changes_list: Entity<ChangesList>,
  pub(crate) history_list: Entity<HistoryList>,
  /// Spawned on the first visit to the tab: a shell per session is too much
  /// for someone who never opens it.
  terminal_view: Option<Entity<TerminalView>>,
  branch_pr: BranchPrState,
  files_tree_state: Entity<TreeState>,
  files_loaded: bool,
  files_loading: bool,
  selected_tree_id: Option<String>,
  pub(crate) _refresh_task: Option<Task<()>>,
  _commit_task: Option<Task<()>>,
  _pr_task: Option<Task<()>>,
  _files_task: Option<Task<()>>,
}

impl DockPanel {
  pub fn new(repo_root: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
    let commit_input = cx.new(|cx| {
      TextareaState::new(window, cx)
        .auto_grow(1, 5)
        .placeholder("Commit message...")
    });
    // cmd-enter from inside the input commits, matching the Git page.
    cx.subscribe_in(
      &commit_input,
      window,
      |this, _state, event: &gpui_component::input::InputEvent, _window, cx| {
        if let gpui_component::input::InputEvent::PressEnter {
          secondary: true, ..
        } = event
        {
          this.commit(cx);
        }
      },
    )
    .detach();

    let split_sections = !crate::config::AppSettings::get(cx).git_unified_file_view;
    let changes_list = cx.new(|cx| ChangesList::new(repo_root.clone(), split_sections, window, cx));
    cx.subscribe_in(
      &changes_list,
      window,
      |this, _list, event: &ChangesListEvent, _window, cx| match event {
        ChangesListEvent::OpenFile { path } => {
          cx.emit(DockPanelEvent::OpenFile { path: path.clone() });
        }
        ChangesListEvent::Changed => this.refresh(cx),
      },
    )
    .detach();

    // The unified/split file list is a setting: follow it without a restart.
    cx.observe_global::<crate::config::AppSettings>(|this, cx| {
      let split_sections = !crate::config::AppSettings::get(cx).git_unified_file_view;
      this
        .changes_list
        .update(cx, |list, cx| list.set_split_sections(split_sections, cx));
    })
    .detach();

    let history_list = cx.new(HistoryList::new);
    cx.subscribe(
      &history_list,
      |_this, _list, event: &HistoryListEvent, cx| match event {
        HistoryListEvent::OpenCommitFile { commit_oid, path } => {
          cx.emit(DockPanelEvent::OpenCommitFile {
            commit_oid: commit_oid.clone(),
            path: path.clone(),
          });
        }
      },
    )
    .detach();

    let mut panel = Self {
      focus_handle: cx.focus_handle(),
      window_handle: window.window_handle(),
      repo_root,
      status_entries: Vec::new(),
      merge_in_progress: false,
      rebase_in_progress: false,
      head_status: HeadCommitStatus::default(),
      branch_status: None,
      commit_input,
      committing: false,
      last_error: None,
      active_tab: DockPanelTab::Changes,
      changes_list,
      history_list,
      terminal_view: None,
      branch_pr: BranchPrState::Loading,
      files_tree_state: cx.new(|cx| TreeState::new(cx)),
      files_loaded: false,
      files_loading: false,
      selected_tree_id: None,
      _refresh_task: None,
      _commit_task: None,
      _pr_task: None,
      _files_task: None,
    };
    panel.refresh(cx);
    panel
  }

  fn load_worktree_files(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.repo_root.clone() else {
      return;
    };
    if self.files_loading {
      return;
    }
    self.files_loading = true;

    let task = cx.spawn(async move |this, cx| {
      let files = cx
        .background_spawn(async move { list_repo_worktree_files(&repo_root) })
        .await;
      let _ = this.update(cx, |this, cx| {
        this.files_loading = false;
        if let Ok(files) = files {
          let items = build_worktree_tree_items(&files);
          this.files_tree_state.update(cx, |state, cx| {
            state.set_items(items, cx);
          });
          this.files_loaded = true;
        }
        cx.notify();
      });
    });
    self._files_task = Some(task);
  }

  pub fn refresh(&mut self, cx: &mut Context<Self>) {
    self.refresh_status(cx);
    self.refresh_branch_pull_request(cx);
    if self.files_loaded {
      self.load_worktree_files(cx);
    }
  }

  /// The working tree alone: what a poll can afford to re-read, with no request
  /// to GitHub and no reload of the file tree.
  pub(crate) fn refresh_status(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.repo_root.clone() else {
      self.status_entries.clear();
      cx.notify();
      return;
    };

    let task = cx.spawn(async move |this, cx| {
      let (result, merge_in_progress, rebase_in_progress, head_status) = cx
        .background_spawn(async move {
          (
            list_repo_status(&repo_root),
            is_merge_in_progress(&repo_root).unwrap_or(false),
            is_rebase_in_progress(&repo_root).unwrap_or(false),
            head_commit_status(&repo_root).unwrap_or_default(),
          )
        })
        .await;
      let _ = this.update(cx, |this, cx| {
        this.merge_in_progress = merge_in_progress;
        this.rebase_in_progress = rebase_in_progress;
        this.head_status = head_status;
        match result {
          Ok(entries) => {
            this.changes_list.update(cx, |list, cx| {
              list.set_entries(entries.clone(), cx);
            });
            this.status_entries = entries;
            this.last_error = None;
            cx.emit(DockPanelEvent::StatusRefreshed);
          }
          Err(error) => this.last_error = Some(format!("{error}").into()),
        }
        cx.notify();
      });
    });
    self._refresh_task = Some(task);
  }

  /// One poll tick: the working tree, plus the history when its tab is open and
  /// the repository actually moved.
  pub(crate) fn poll(&mut self, cx: &mut Context<Self>) {
    self.refresh_status(cx);
    if self.active_tab == DockPanelTab::History {
      self.history_list.update(cx, |list, cx| {
        list.refresh_if_repository_moved(cx);
      });
    }
  }

  pub(crate) fn refresh_branch_pull_request_state(&mut self, cx: &mut Context<Self>) {
    self.refresh_branch_pull_request(cx);
  }

  fn refresh_branch_pull_request(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.repo_root.clone() else {
      return;
    };
    if !AuthStateStore::has_github_access(cx) {
      self.branch_pr = BranchPrState::NoAccess;
      cx.notify();
      return;
    }

    self.branch_pr = BranchPrState::Loading;
    let api = WorkspaceApi::global(cx).api.clone();
    let task = cx.spawn(async move |this, cx| {
      let state = cx
        .background_spawn(async move {
          branch_pr_state_for_lookup(
            current_github_remote_repo(&repo_root).ok().flatten(),
            current_branch_status(&repo_root)
              .ok()
              .map(|status| status.name),
            |context| {
              api.fetch_pull_request_for_branch(&context.owner, &context.repo, &context.branch)
            },
          )
        })
        .await;

      let _ = this.update(cx, |this, cx| {
        this.branch_pr = state;
        cx.notify();
      });
    });
    self._pr_task = Some(task);
  }

  /// Git prepared a message for the operation in progress (merge, rebase).
  pub(crate) fn set_commit_message(
    &mut self,
    message: &str,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self
      .commit_input
      .update(cx, |input, cx| input.set_value(message, window, cx));
  }

  pub(crate) fn commit_message(&self, cx: &App) -> String {
    self.commit_input.read(cx).value().to_string()
  }

  /// The host owns the branch; the panel needs it to know what its menu allows.
  pub(crate) fn set_branch_status(
    &mut self,
    branch_status: Option<git::BranchStatus>,
    cx: &mut Context<Self>,
  ) {
    self.branch_status = branch_status;
    cx.notify();
  }

  /// The palette offers the same thing as the Pull request tab, so the keyboard
  /// reaches the branch's pull request without going through the dock.
  pub(crate) fn branch_pull_request_command(&self) -> Option<CommandPaletteCommand> {
    match &self.branch_pr {
      BranchPrState::NoAccess | BranchPrState::NoRemote => None,
      BranchPrState::Loading => Some(
        CommandPaletteCommand::create_pull_request().disabled("Checking for an open pull request"),
      ),
      // Publishing is a push: it stays a deliberate click in the tab.
      BranchPrState::Missing(_) if self.branch_needs_publishing() => None,
      BranchPrState::Missing(_) => Some(CommandPaletteCommand::create_pull_request()),
      BranchPrState::Found(_, pull_request) => Some(CommandPaletteCommand::open_pull_request(
        pull_request.number,
      )),
    }
  }

  /// What the Pull request tab's own button does, for the palette to reuse.
  pub(crate) fn create_branch_pull_request(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let BranchPrState::Missing(context) = &self.branch_pr else {
      return;
    };
    if self.branch_needs_publishing() {
      return;
    }
    open_create_pull_request_dialog(
      WorkspaceApi::global(cx).api.clone(),
      self.window_handle,
      self.pr_created_handler(cx),
      context.clone(),
      window,
      cx,
    );
  }

  #[cfg(test)]
  pub(crate) fn set_branch_pull_request_state(
    &mut self,
    state: BranchPrState,
    cx: &mut Context<Self>,
  ) {
    self.branch_pr = state;
    cx.notify();
  }

  pub(crate) fn open_branch_pull_request(&self, cx: &mut Context<Self>) {
    let BranchPrState::Found(context, pull_request) = &self.branch_pr else {
      return;
    };
    open_pr_target(
      context.owner.clone(),
      context.repo.clone(),
      pull_request.number,
      false,
      None,
      cx,
    );
  }

  /// GitHub cannot open a pull request for a branch its remote has never seen.
  fn branch_needs_publishing(&self) -> bool {
    should_publish_branch(
      self.branch_status.as_ref(),
      self.head_status.has_head_commit,
    )
  }

  fn repo_state<'a>(&'a self, commit_message: &'a str) -> RepoState<'a> {
    let branch_status = self.branch_status.as_ref();
    let (can_push, can_force_push) =
      push_flags(branch_status, self.head_status.has_head_commit, false);
    RepoState {
      has_repo: self.repo_root.is_some(),
      merge_in_progress: self.merge_in_progress,
      rebase_in_progress: self.rebase_in_progress,
      has_head_commit: self.head_status.has_head_commit,
      can_push,
      can_force_push,
      can_undo_last_commit: self.head_status.can_undo_last_commit,
      branch_status,
      status_entries: &self.status_entries,
      selected_entry: None,
      commit_message,
    }
  }

  pub(crate) fn changes_list(&self) -> Entity<ChangesList> {
    self.changes_list.clone()
  }

  pub(crate) fn head_status(&self) -> HeadCommitStatus {
    self.head_status
  }

  pub(crate) fn merge_in_progress(&self) -> bool {
    self.merge_in_progress
  }

  pub(crate) fn rebase_in_progress(&self) -> bool {
    self.rebase_in_progress
  }

  pub(crate) fn status_entries(&self) -> &[RepoStatusEntry] {
    &self.status_entries
  }

  #[cfg(test)]
  pub(crate) fn repo_root(&self) -> Option<&Path> {
    self.repo_root.as_deref()
  }

  pub(crate) fn set_repo_root(&mut self, repo_root: Option<PathBuf>, cx: &mut Context<Self>) {
    self.repo_root = repo_root.clone();
    self.status_entries.clear();
    self.last_error = None;
    self.changes_list.update(cx, |list, cx| {
      list.set_repo_root(repo_root.clone(), cx);
    });
    self.history_list.update(cx, |list, cx| {
      list.set_repo_root(repo_root.clone(), cx);
    });
    if let Some(terminal) = self.terminal_view.clone() {
      terminal.update(cx, |terminal, cx| {
        terminal.set_working_directory(repo_root, cx);
      });
    }
  }

  /// The history is only worth loading once its tab is opened.
  fn refresh_history(&mut self, cx: &mut Context<Self>) {
    let repo_root = self.repo_root.clone();
    self.history_list.update(cx, |list, cx| {
      list.set_repo_root(repo_root, cx);
      if list.is_empty() {
        list.refresh(cx);
      }
    });
  }

  fn render_history_tab(&self) -> AnyElement {
    div()
      .id("dock-panel-history")
      .debug_selector(|| DOCK_PANEL_HISTORY_DEBUG_SELECTOR.to_string())
      .flex_1()
      .min_h_0()
      .min_w(px(0.0))
      .child(self.history_list.clone())
      .into_any_element()
  }

  fn ensure_terminal(&mut self, cx: &mut Context<Self>) {
    if self.terminal_view.is_some() {
      return;
    }
    let working_directory = self.repo_root.clone();
    self.terminal_view = Some(cx.new(|cx| TerminalView::new(working_directory, cx)));
  }

  /// Shows the shell, never starts it: spawning a process while painting is the
  /// mistake this crate already made with the agent panel.
  fn render_terminal_tab(&self) -> AnyElement {
    let Some(terminal) = self.terminal_view.clone() else {
      return div().into_any_element();
    };

    div()
      .id("dock-panel-terminal")
      .debug_selector(|| DOCK_PANEL_TERMINAL_DEBUG_SELECTOR.to_string())
      .flex_1()
      .min_h_0()
      .min_w(px(0.0))
      .child(terminal)
      .into_any_element()
  }

  fn has_staged_changes(&self) -> bool {
    self
      .status_entries
      .iter()
      .any(|entry| !matches!(entry.stage, RepoStage::Unstaged))
  }

  pub(crate) fn commit(&mut self, cx: &mut Context<Self>) {
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
      let result = cx
        .background_spawn(async move {
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
            cx.emit(DockPanelEvent::Committed);
          }
          Err(error) => this.last_error = Some(format!("{error}").into()),
        }
        this.refresh(cx);
      });
    });
    self._commit_task = Some(task);
  }

  fn render_commit_zone(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    // A rebase ends by continuing it, not by writing another commit message.
    let continuing_rebase = self.rebase_in_progress;
    let can_commit = if continuing_rebase {
      !crate::changes_list::has_conflicted_entries(&self.status_entries)
    } else {
      !self.committing
        && !self.status_entries.is_empty()
        && !self.commit_input.read(cx).value().trim().is_empty()
    };
    let commit_shortcut = crate::shortcuts::resolved_display_shortcut_keystroke_in(
      cx,
      window,
      crate::shortcuts::ShortcutId::CommitChanges,
    );

    v_flex()
      .gap_2()
      .p_2()
      .border_t_1()
      .border_color(theme.border)
      .when_some(self.last_error.clone(), |this, error| {
        this.child(div().text_xs().text_color(theme.status_red()).child(error))
      })
      .child(div().w_full().min_w_0().key_context("CommitInput").child({
        let commit_box = Textarea::new(&self.commit_input).w_full();
        commit_box.into_any_element()
      }))
      .when(self.merge_in_progress || continuing_rebase, |this| {
        let label = if continuing_rebase {
          "Rebase in progress"
        } else {
          "Merge in progress"
        };
        this.child(
          div()
            .debug_selector(|| DOCK_PANEL_OPERATION_DEBUG_SELECTOR.to_string())
            .text_xs()
            .text_color(theme.status_orange())
            .child(label),
        )
      })
      .child(h_flex().w_full().child(self.render_commit_button(
        continuing_rebase,
        can_commit,
        commit_shortcut,
        cx,
      )))
      .into_any_element()
  }

  /// The primary action, plus the menu of what else can be done to the last
  /// commit or the branch.
  fn render_commit_button(
    &self,
    continuing_rebase: bool,
    can_commit: bool,
    commit_shortcut: gpui::Keystroke,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let commit_message = self.commit_input.read(cx).value().to_string();
    let state = self.repo_state(&commit_message);
    let menu_items = [
      CommitMenuCommand::Amend,
      CommitMenuCommand::UndoLastCommit,
      CommitMenuCommand::Push,
      CommitMenuCommand::ForcePush,
    ]
    .map(|command| (command, state.allows(command.rule())));
    let menu_enabled = menu_items.iter().any(|(_, allowed)| *allowed);
    let view = cx.entity();

    h_flex()
      .w_full()
      .child(
        Button::new("dock-panel-commit")
          .label(if continuing_rebase {
            "Continue rebase"
          } else {
            "Commit"
          })
          .debug_selector(|| DOCK_PANEL_COMMIT_DEBUG_SELECTOR.to_string())
          .with_variant(gpui_component::button::ButtonVariant::Secondary)
          .outline()
          .small()
          .w_full()
          .child(gpui_component::kbd::Kbd::new(commit_shortcut).ml_1())
          .loading(self.committing)
          .disabled(!can_commit)
          .on_click(cx.listener(|this, _, _, cx| {
            if this.rebase_in_progress {
              cx.emit(DockPanelEvent::ContinueRebase);
              return;
            }
            this.commit(cx)
          }))
          .flex_1()
          .rounded_r_none(),
      )
      .child(
        Button::new("dock-panel-commit-menu")
          .icon(IconName::ChevronDown)
          .with_variant(gpui_component::button::ButtonVariant::Secondary)
          .outline()
          .small()
          .rounded_l_none()
          .border_l_0()
          .debug_selector(|| DOCK_PANEL_COMMIT_MENU_DEBUG_SELECTOR.to_string())
          .disabled(!menu_enabled)
          .dropdown_menu_with_anchor(Anchor::BottomRight, move |menu, _, _| {
            menu_items.iter().fold(menu, |menu, (command, allowed)| {
              let view = view.clone();
              let command = *command;
              menu.item(
                PopupMenuItem::new(command.label())
                  .icon(command.icon())
                  .disabled(!allowed)
                  .on_click(move |_, _, cx| {
                    view.update(cx, |_, cx| cx.emit(DockPanelEvent::RunCommand(command)));
                  }),
              )
            })
          }),
      )
      .into_any_element()
  }

  /// Opens a tab and gives it focus, loading what that tab needs the first time.
  pub(crate) fn open_tab(
    &mut self,
    target: DockPanelTab,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.active_tab != target {
      self.active_tab = target;
      match target {
        DockPanelTab::PullRequest => self.refresh_branch_pull_request(cx),
        DockPanelTab::Terminal => self.ensure_terminal(cx),
        DockPanelTab::History => self.refresh_history(cx),
        DockPanelTab::Changes | DockPanelTab::Files => {}
      }
    }
    match target {
      DockPanelTab::Changes => {
        let list = self.changes_list.clone();
        list.update(cx, |list, cx| list.focus(window, cx));
      }
      DockPanelTab::History => {
        let history = self.history_list.clone();
        window.focus(&history.read(cx).focus_handle(cx), cx);
      }
      _ => window.focus(&self.focus_handle, cx),
    }
    cx.notify();
  }

  #[cfg(test)]
  pub(crate) fn has_terminal(&self) -> bool {
    self.terminal_view.is_some()
  }

  pub(crate) fn active_tab(&self) -> DockPanelTab {
    self.active_tab
  }

  fn render_tabs(&self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let tab = |id: &'static str, label: &'static str, target: DockPanelTab, active: bool| {
      div()
        .id(id)
        .debug_selector(move || id.to_string())
        .px_2()
        .py_1()
        .rounded(px(5.0))
        .text_xs()
        .cursor_pointer()
        .when(active, |this| {
          this.bg(theme.secondary_active).text_color(theme.foreground)
        })
        .when(!active, |this| {
          this
            .text_color(theme.muted_foreground)
            .hover(|s| s.bg(theme.secondary_hover))
        })
        .child(label)
        .on_click(cx.listener(move |this, _, window, cx| {
          this.open_tab(target, window, cx);
        }))
    };

    h_flex()
      .items_center()
      .gap_1()
      .child(tab(
        "dock-panel-tab-changes",
        "Changes",
        DockPanelTab::Changes,
        self.active_tab == DockPanelTab::Changes,
      ))
      .child(tab(
        "dock-panel-tab-files",
        "Files",
        DockPanelTab::Files,
        self.active_tab == DockPanelTab::Files,
      ))
      .child(tab(
        "dock-panel-tab-history",
        "History",
        DockPanelTab::History,
        self.active_tab == DockPanelTab::History,
      ))
      .child(tab(
        "dock-panel-tab-pr",
        "Pull request",
        DockPanelTab::PullRequest,
        self.active_tab == DockPanelTab::PullRequest,
      ))
      .child(tab(
        "dock-panel-tab-terminal",
        "Terminal",
        DockPanelTab::Terminal,
        self.active_tab == DockPanelTab::Terminal,
      ))
      .into_any_element()
  }

  fn render_files_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();

    // A file row was selected (directories end with '/'): open it in the editor.
    if let Some(selected_id) = self
      .files_tree_state
      .read(cx)
      .selected_entry()
      .map(|entry| entry.item().id.to_string())
      && Some(selected_id.as_str()) != self.selected_tree_id.as_deref()
    {
      self.selected_tree_id = Some(selected_id.clone());
      if !selected_id.ends_with('/') {
        let path = PathBuf::from(selected_id);
        cx.on_next_frame(window, move |_, _, cx| {
          cx.emit(DockPanelEvent::OpenFile { path });
        });
      }
    }

    if !self.files_loaded {
      if !self.files_loading {
        self.load_worktree_files(cx);
      }
      return v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .child(
          div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child("Loading files..."),
        )
        .into_any_element();
    }

    let modified: std::collections::HashSet<PathBuf> = self
      .status_entries
      .iter()
      .map(|entry| entry.path.clone())
      .collect();

    div()
      .flex_1()
      .min_h_0()
      .py_1()
      .child(tree(
        &self.files_tree_state,
        move |ix, entry, selected, _window, cx| {
          let theme = cx.theme().clone();
          let item = entry.item();
          let is_folder = entry.is_folder();
          let icon: AnyElement = if is_folder {
            Icon::new(if entry.is_expanded() {
              IconName::FolderOpen
            } else {
              IconName::Folder
            })
            .size_3()
            .text_color(theme.muted_foreground)
            .into_any_element()
          } else {
            ui::file_icon_path_for_name_with_theme(item.label.as_ref(), &theme)
              .map(|path| img(path).size(px(ui::FILE_ICON_SIZE_PX)).into_any_element())
              .unwrap_or_else(|| {
                Icon::new(IconName::File)
                  .size_3()
                  .text_color(theme.muted_foreground)
                  .into_any_element()
              })
          };
          let is_modified = !is_folder && modified.contains(&PathBuf::from(item.id.as_ref()));

          let indent = px(8.) + px(14.) * entry.depth();
          ui::selectable_list_item(ix, selected, ui::SelectableRowStyle::Inset, &theme)
            .w_full()
            .px_2()
            .pl(indent)
            .child(
              h_flex()
                .items_center()
                .gap_2()
                .child(icon)
                .child(
                  div()
                    .flex_1()
                    .overflow_hidden()
                    .text_ellipsis()
                    .text_sm()
                    .child(item.label.clone()),
                )
                .when(is_modified, |this| {
                  this.child(
                    div()
                      .text_xs()
                      .font_weight(gpui::FontWeight::BOLD)
                      .text_color(theme.status_yellow())
                      .child("M"),
                  )
                }),
            )
        },
      ))
      .into_any_element()
  }

  fn pr_created_handler(&self, cx: &mut Context<Self>) -> PullRequestCreatedHandler {
    let panel = cx.entity().downgrade();
    Rc::new(move |_context, _pull_request, cx| {
      let _ = panel.update(cx, |panel, cx| {
        panel.refresh_branch_pull_request(cx);
      });
    })
  }

  fn render_pr_message(&self, text: &'static str, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    v_flex()
      .flex_1()
      .items_center()
      .justify_center()
      .gap_2()
      .px_4()
      .child(
        Icon::new(UiIconName::GitPullRequest)
          .size_4()
          .text_color(theme.muted_foreground),
      )
      .child(
        div()
          .text_sm()
          .text_center()
          .text_color(theme.muted_foreground)
          .child(text),
      )
      .into_any_element()
  }

  fn render_pr_tab(&self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    match &self.branch_pr {
      BranchPrState::NoAccess => self.render_pr_message(
        "Sign in with GitHub to link this branch to a pull request",
        cx,
      ),
      BranchPrState::NoRemote => self.render_pr_message("No GitHub remote on this repository", cx),
      BranchPrState::Loading => self.render_pr_message("Loading pull request...", cx),
      BranchPrState::Missing(context) if self.branch_needs_publishing() => {
        let context = context.clone();
        v_flex()
          .flex_1()
          .items_center()
          .justify_center()
          .gap_3()
          .px_4()
          .child(
            Icon::new(UiIconName::GitPullRequestArrow)
              .size_4()
              .text_color(theme.muted_foreground),
          )
          .child(
            div()
              .text_sm()
              .text_center()
              .text_color(theme.muted_foreground)
              .child(format!("{} is not on the remote yet", context.branch)),
          )
          .child(
            Button::new("dock-panel-publish-and-create-pr")
              .primary()
              .small()
              .label("Publish and create pull request")
              .debug_selector(|| DOCK_PANEL_PUBLISH_AND_CREATE_PR_DEBUG_SELECTOR.to_string())
              .on_click(cx.listener(move |_, _, _, cx| {
                cx.emit(DockPanelEvent::PublishBranchAndCreatePullRequest(
                  context.clone(),
                ));
              })),
          )
          .into_any_element()
      }
      BranchPrState::Missing(context) => {
        let context = context.clone();
        v_flex()
          .flex_1()
          .items_center()
          .justify_center()
          .gap_3()
          .px_4()
          .child(
            Icon::new(UiIconName::GitPullRequestArrow)
              .size_4()
              .text_color(theme.muted_foreground),
          )
          .child(
            div()
              .text_sm()
              .text_center()
              .text_color(theme.muted_foreground)
              .child(format!("No pull request for {}", context.branch)),
          )
          .child(
            Button::new("dock-panel-create-pr")
              .primary()
              .small()
              .label("Create pull request")
              .debug_selector(|| DOCK_PANEL_CREATE_PR_DEBUG_SELECTOR.to_string())
              .on_click(cx.listener({
                let context = context.clone();
                move |this, _, window, cx| {
                  open_create_pull_request_dialog(
                    WorkspaceApi::global(cx).api.clone(),
                    this.window_handle,
                    this.pr_created_handler(cx),
                    context.clone(),
                    window,
                    cx,
                  );
                }
              })),
          )
          .child(
            // Reviewers, labels and projects live on github.com, not in our dialog.
            Button::new("dock-panel-compare-on-github")
              .ghost()
              .xsmall()
              .label("Open compare on GitHub")
              .debug_selector(|| DOCK_PANEL_COMPARE_DEBUG_SELECTOR.to_string())
              .on_click(move |_, _, cx| {
                open_compare_target(
                  context.owner.clone(),
                  context.repo.clone(),
                  context.branch.clone(),
                  cx,
                );
              }),
          )
          .into_any_element()
      }
      BranchPrState::Found(context, pull_request) => {
        let status = pull_request.status();
        let owner = context.owner.clone();
        let repo = context.repo.clone();
        let number = pull_request.number;

        v_flex()
          .gap_2()
          .p_3()
          .child(
            h_flex()
              .items_center()
              .gap_2()
              .child(
                div()
                  .text_xs()
                  .font_weight(gpui::FontWeight::SEMIBOLD)
                  .text_color(pull_request_status_color(status, &theme))
                  .child(pull_request_status_label(status)),
              )
              .child(
                div()
                  .text_xs()
                  .text_color(theme.muted_foreground)
                  .child(format!("#{number}")),
              ),
          )
          .child(
            div()
              .text_sm()
              .text_color(theme.foreground)
              .child(pull_request.title.clone()),
          )
          .child(
            div()
              .text_xs()
              .text_color(theme.muted_foreground)
              .child(format!(
                "{} comments · {}",
                pull_request.comments_count, context.branch
              )),
          )
          .child(
            Button::new("dock-panel-open-pr")
              .small()
              .w_full()
              .label("Open pull request")
              .on_click(cx.listener(move |_, _, _, cx| {
                open_pr_target(owner.clone(), repo.clone(), number, false, None, cx);
              })),
          )
          .into_any_element()
      }
    }
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
      .child(div().text_sm().text_color(theme.muted_foreground).child(
        if self.repo_root.is_some() {
          "No changes"
        } else {
          "No repository"
        },
      ))
      .into_any_element()
  }
}

impl Render for DockPanel {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let entry_count = self.status_entries.len();

    let header = h_flex()
      .h(px(40.))
      .min_h(px(40.))
      .max_h(px(40.))
      .flex_shrink_0()
      .items_center()
      .justify_between()
      .px_2()
      .border_b_1()
      .border_color(theme.border)
      .child(self.render_tabs(cx))
      .child(
        h_flex()
          .items_center()
          .gap_1()
          .when(
            self.active_tab == DockPanelTab::Changes && entry_count > 0,
            |this| {
              this.child(
                div()
                  .text_xs()
                  .text_color(theme.muted_foreground)
                  .child(entry_count.to_string()),
              )
            },
          )
          .when(self.active_tab != DockPanelTab::Terminal, |this| {
            this.child(
              Button::new("dock-panel-refresh")
                .debug_selector(|| DOCK_PANEL_REFRESH_DEBUG_SELECTOR.to_string())
                .icon(UiIconName::RefreshCw)
                .ghost()
                .compact()
                .small()
                .tooltip("Refresh")
                .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
            )
          }),
      );

    let body = match self.active_tab {
      DockPanelTab::Files => self.render_files_tab(_window, cx),
      DockPanelTab::Changes => {
        if self.status_entries.is_empty() {
          self.render_empty_state(cx)
        } else {
          div()
            .id("dock-panel-file-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .py_1()
            .child(self.changes_list.clone())
            .into_any_element()
        }
      }
      DockPanelTab::PullRequest => self.render_pr_tab(cx),
      DockPanelTab::History => self.render_history_tab(),
      DockPanelTab::Terminal => self.render_terminal_tab(),
    };

    let mut panel = v_flex()
      .size_full()
      .min_w(px(0.0))
      .min_h_0()
      .bg(theme.sidebar)
      .track_focus(&self.focus_handle)
      .on_action(cx.listener(|this, _: &crate::CommitChanges, _, cx| this.commit(cx)))
      .child(header)
      .child(body);
    if self.active_tab == DockPanelTab::Changes {
      panel = panel.child(self.render_commit_zone(_window, cx));
    }
    panel
  }
}

impl Focusable for DockPanel {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::test_support::{TempRepo, commit_text_file};
  use git::RepoStatusKind;
  use git2::Repository;
  use gpui::TestAppContext;
  use std::path::Path;
  use std::sync::Arc;
  use std::sync::atomic::{AtomicBool, Ordering};
  use ui::{CommandPaletteCommandId, WindowExt as _};

  #[gpui::test]
  async fn a_poll_re_reads_the_working_tree_without_calling_github(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_component::init(cx));
    let repo = TempRepo::init("dock-poll-local");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;
    panel.update(cx, |panel, _| {
      // As if the Files tab had been opened once already.
      panel.files_loaded = true;
      panel._files_task.take();
      panel.branch_pr = BranchPrState::Loading;
    });

    std::fs::write(repo.path.join("README.md"), "v2\n").expect("edit outside Reviu");
    panel.update(cx, |panel, cx| panel.poll(cx));
    await_refresh(&panel, cx).await;

    panel.read_with(cx, |panel, _| {
      assert_eq!(panel.status_entries().len(), 1, "the poll saw the edit");
      assert!(
        matches!(panel.branch_pr, BranchPrState::Loading),
        "a poll asks GitHub nothing"
      );
      assert!(
        panel._files_task.is_none(),
        "a poll does not rebuild the file tree"
      );
    });

    // An explicit refresh still does both.
    panel.update(cx, |panel, cx| panel.refresh(cx));
    await_refresh(&panel, cx).await;
    panel.read_with(cx, |panel, _| {
      assert!(!matches!(panel.branch_pr, BranchPrState::Loading));
      assert!(panel._files_task.is_some());
    });
  }

  #[gpui::test]
  async fn a_poll_touches_the_history_only_when_its_tab_is_open(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_component::init(cx));
    let repo = TempRepo::init("dock-poll-history");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "first");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;

    // Open the history once, so it already knows the repository, then leave it.
    panel.update_in(cx, |panel, window, cx| {
      panel.open_tab(DockPanelTab::History, window, cx)
    });
    cx.run_until_parked();
    panel.update_in(cx, |panel, window, cx| {
      panel.open_tab(DockPanelTab::Changes, window, cx)
    });
    cx.run_until_parked();
    panel.update(cx, |panel, cx| {
      panel.history_list.update(cx, |list, _| {
        list._poll_task.take();
        list._history_task.take();
      })
    });

    panel.update(cx, |panel, cx| panel.poll(cx));
    await_refresh(&panel, cx).await;
    panel.read_with(cx, |panel, cx| {
      assert!(
        panel.history_list.read(cx)._poll_task.is_none(),
        "the history costs nothing while its tab is closed"
      );
    });

    panel.update_in(cx, |panel, window, cx| {
      panel.open_tab(DockPanelTab::History, window, cx)
    });
    cx.run_until_parked();

    panel.update(cx, |panel, cx| panel.poll(cx));
    await_refresh(&panel, cx).await;
    panel.read_with(cx, |panel, cx| {
      assert!(
        panel.history_list.read(cx)._poll_task.is_some(),
        "an open history tab follows the repository"
      );
    });
  }

  #[gpui::test]
  async fn an_unpublished_branch_is_offered_a_push_before_the_pull_request(
    cx: &mut TestAppContext,
  ) {
    cx.update(|cx| gpui_component::init(cx));
    let repo = TempRepo::init("dock-publish-and-create-pr");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;

    let context = GithubBranchContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      branch: "feature".to_string(),
    };
    // Opening the tab refreshes the lookup, so the state is set afterwards.
    panel.update_in(cx, |panel, window, cx| {
      panel.open_tab(DockPanelTab::PullRequest, window, cx)
    });
    cx.run_until_parked();
    panel.update(cx, |panel, cx| {
      panel.branch_pr = BranchPrState::Missing(context.clone());
      panel.set_branch_status(
        Some(git::BranchStatus {
          name: "feature".to_string(),
          ahead: 1,
          behind: 0,
          has_upstream: false,
        }),
        cx,
      );
    });
    cx.run_until_parked();

    assert!(
      cx.debug_bounds(DOCK_PANEL_PUBLISH_AND_CREATE_PR_DEBUG_SELECTOR)
        .is_some(),
      "a branch the remote never saw is published first"
    );
    assert!(
      cx.debug_bounds(DOCK_PANEL_CREATE_PR_DEBUG_SELECTOR)
        .is_none(),
      "GitHub cannot open a pull request for it yet"
    );

    // The event carries the branch the form will target.
    let published = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let recorder = published.clone();
    cx.update(|_, cx| {
      cx.subscribe(&panel, move |_panel, event: &DockPanelEvent, _cx| {
        if let DockPanelEvent::PublishBranchAndCreatePullRequest(context) = event {
          recorder.borrow_mut().push(context.branch.clone());
        }
      })
      .detach();
    });
    let button = cx
      .debug_bounds(DOCK_PANEL_PUBLISH_AND_CREATE_PR_DEBUG_SELECTOR)
      .expect("publish button bounds");
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert_eq!(published.borrow().as_slice(), ["feature".to_string()]);

    // Even called directly, the form refuses a branch the remote never saw.
    panel.update_in(cx, |panel, window, cx| {
      panel.create_branch_pull_request(window, cx)
    });
    cx.run_until_parked();
    assert!(!cx.update(|window, cx| window.has_active_dialog(cx)));

    // Once it has an upstream, the plain Create button comes back.
    panel.update(cx, |panel, cx| {
      panel.set_branch_status(
        Some(git::BranchStatus {
          name: "feature".to_string(),
          ahead: 0,
          behind: 0,
          has_upstream: true,
        }),
        cx,
      );
    });
    cx.run_until_parked();
    assert!(
      cx.debug_bounds(DOCK_PANEL_CREATE_PR_DEBUG_SELECTOR)
        .is_some(),
      "a published branch goes straight to the form"
    );
    assert!(
      cx.debug_bounds(DOCK_PANEL_PUBLISH_AND_CREATE_PR_DEBUG_SELECTOR)
        .is_none()
    );
  }

  #[gpui::test]
  async fn the_palette_mirrors_what_the_pull_request_tab_offers(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_component::init(cx));
    let repo = TempRepo::init("dock-pr-palette");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;

    let context = GithubBranchContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      branch: "feature".to_string(),
    };
    let published = git::BranchStatus {
      name: "feature".to_string(),
      ahead: 0,
      behind: 0,
      has_upstream: true,
    };

    let command_id = |panel: &DockPanel| {
      panel
        .branch_pull_request_command()
        .map(|command| (command.id, command.disabled_reason.is_some()))
    };

    // Without GitHub, or without a GitHub remote, the palette says nothing.
    panel.update(cx, |panel, cx| {
      panel.branch_pr = BranchPrState::NoAccess;
      panel.set_branch_status(Some(published.clone()), cx);
    });
    panel.read_with(cx, |panel, _| assert_eq!(command_id(panel), None));
    panel.update(cx, |panel, _| panel.branch_pr = BranchPrState::NoRemote);
    panel.read_with(cx, |panel, _| assert_eq!(command_id(panel), None));

    // While the lookup runs, the command shows up but cannot be run.
    panel.update(cx, |panel, _| panel.branch_pr = BranchPrState::Loading);
    panel.read_with(cx, |panel, _| {
      assert_eq!(
        command_id(panel),
        Some((CommandPaletteCommandId::CreatePullRequest, true))
      );
    });

    // No pull request on a published branch: the form is one keystroke away.
    panel.update(cx, |panel, _| {
      panel.branch_pr = BranchPrState::Missing(context.clone())
    });
    panel.read_with(cx, |panel, _| {
      assert_eq!(
        command_id(panel),
        Some((CommandPaletteCommandId::CreatePullRequest, false))
      );
    });

    // Unpublished: publishing is a push, it stays a deliberate click in the tab.
    panel.update(cx, |panel, cx| {
      panel.set_branch_status(
        Some(git::BranchStatus {
          name: "feature".to_string(),
          ahead: 1,
          behind: 0,
          has_upstream: false,
        }),
        cx,
      );
    });
    panel.read_with(cx, |panel, _| assert_eq!(command_id(panel), None));

    // An existing pull request: the palette opens that one.
    panel.update(cx, |panel, cx| {
      panel.set_branch_status(Some(published), cx);
      panel.branch_pr = BranchPrState::Found(context, Box::new(test_pull_request()));
    });
    panel.read_with(cx, |panel, _| {
      assert_eq!(
        command_id(panel),
        Some((CommandPaletteCommandId::OpenPullRequest, false))
      );
      let label = panel
        .branch_pull_request_command()
        .expect("command")
        .name
        .to_string();
      assert!(label.contains("42"), "the command names the pull request");
    });
  }

  async fn await_refresh(panel: &Entity<DockPanel>, cx: &mut gpui::VisualTestContext) {
    let task = panel.update(cx, |panel, _| panel._refresh_task.take());
    if let Some(task) = task {
      task.await;
    }
    cx.run_until_parked();
  }

  fn add_dock_panel_window(
    repo_root: Option<PathBuf>,
    cx: &mut TestAppContext,
  ) -> (Entity<DockPanel>, &mut gpui::VisualTestContext) {
    cx.update(|cx| {
      if !cx.has_global::<crate::config::AppSettings>() {
        cx.set_global(crate::config::AppSettings::default());
      }
      if !cx.has_global::<AuthStateStore>() {
        cx.set_global(AuthStateStore::default());
      }
      if !cx.has_global::<WorkspaceApi>() {
        cx.set_global(WorkspaceApi::new());
      }
    });
    let mut mounted: Option<Entity<DockPanel>> = None;
    let (_root, cx) = cx.add_window_view(|window, cx| {
      let panel = cx.new(|cx| DockPanel::new(repo_root.clone(), window, cx));
      mounted = Some(panel.clone());
      gpui_component::Root::new(panel, window, cx)
    });
    (mounted.expect("dock panel"), cx)
  }

  fn test_remote() -> git::GithubRemoteRepo {
    git::GithubRemoteRepo {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
    }
  }

  fn test_pull_request() -> GithubPullRequest {
    serde_json::from_value(serde_json::json!({
      "number": 42,
      "title": "Add widgets",
      "state": "open",
      "draft": false,
      "updated_at": "2026-08-13T00:00:00Z",
      "labels": [],
      "repository": { "owner": "acme", "repo": "widget" }
    }))
    .expect("build test pull request")
  }

  fn tree_ids(items: &[TreeItem]) -> Vec<String> {
    let mut ids = Vec::new();
    for item in items {
      ids.push(item.id.to_string());
      ids.extend(tree_ids(&item.children));
    }
    ids
  }

  #[test]
  fn build_worktree_tree_items_nests_dirs_first_then_files_sorted() {
    let files = vec![
      PathBuf::from("src/main.rs"),
      PathBuf::from("README.md"),
      PathBuf::from("src/api/client.rs"),
      PathBuf::from("Cargo.toml"),
    ];

    let items = build_worktree_tree_items(&files);

    // Top level: dirs first (src/), then files alphabetically.
    assert_eq!(
      items
        .iter()
        .map(|item| item.id.to_string())
        .collect::<Vec<_>>(),
      vec!["src/", "Cargo.toml", "README.md"]
    );
    // Depth-first: directory ids end with '/', file ids are the relative path.
    assert_eq!(
      tree_ids(&items),
      vec![
        "src/",
        "src/api/",
        "src/api/client.rs",
        "src/main.rs",
        "Cargo.toml",
        "README.md"
      ]
    );
    let src = &items[0];
    assert!(src.is_folder());
    assert_eq!(src.label.to_string(), "src");
  }

  #[test]
  fn build_worktree_tree_items_handles_empty_input() {
    assert!(build_worktree_tree_items(&[]).is_empty());
  }

  #[test]
  fn branch_pr_state_requires_remote_and_branch() {
    let no_remote = branch_pr_state_for_lookup(None, Some("main".to_string()), |_| {
      panic!("fetch must not run without a remote")
    });
    assert!(matches!(no_remote, BranchPrState::NoRemote));

    let no_branch = branch_pr_state_for_lookup(Some(test_remote()), None, |_| {
      panic!("fetch must not run without a branch")
    });
    assert!(matches!(no_branch, BranchPrState::NoRemote));
  }

  #[test]
  fn branch_pr_state_maps_lookup_results() {
    let found = branch_pr_state_for_lookup(
      Some(test_remote()),
      Some("feature/x".to_string()),
      |context| {
        assert_eq!(context.owner, "acme");
        assert_eq!(context.repo, "widget");
        assert_eq!(context.branch, "feature/x");
        Ok(Some(test_pull_request()))
      },
    );
    match found {
      BranchPrState::Found(context, pull_request) => {
        assert_eq!(context.branch, "feature/x");
        assert_eq!(pull_request.number, 42);
      }
      other => panic!("expected Found, got {other:?}"),
    }

    let missing =
      branch_pr_state_for_lookup(Some(test_remote()), Some("feature/x".to_string()), |_| {
        Ok(None)
      });
    assert!(matches!(missing, BranchPrState::Missing(_)));

    // API errors degrade to Missing so the tab still offers Create against the context.
    let errored =
      branch_pr_state_for_lookup(Some(test_remote()), Some("feature/x".to_string()), |_| {
        Err(anyhow::anyhow!("network down"))
      });
    match errored {
      BranchPrState::Missing(context) => assert_eq!(context.branch, "feature/x"),
      other => panic!("expected Missing, got {other:?}"),
    }
  }

  #[gpui::test]
  async fn branch_pr_requires_github_access(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_component::init(cx));
    let repo = TempRepo::init("dock-panel-pr-gate");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    cx.run_until_parked();

    // Default auth state is Unknown: no GitHub access, no lookup attempted.
    panel.read_with(cx, |panel, _| {
      assert!(matches!(panel.branch_pr, BranchPrState::NoAccess));
    });
  }

  #[gpui::test]
  async fn refresh_lists_working_tree_changes(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_component::init(cx));
    let repo = TempRepo::init("dock-panel-refresh");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;

    panel.read_with(cx, |panel, _| {
      assert_eq!(panel.status_entries.len(), 1);
      assert_eq!(panel.status_entries[0].path, PathBuf::from("README.md"));
      assert_eq!(panel.status_entries[0].status, RepoStatusKind::Modified);
    });
  }

  #[gpui::test]
  async fn a_merge_in_progress_still_ends_with_a_commit(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_component::init(cx));
    let repo = TempRepo::init("dock-panel-merge");
    commit_text_file(&repo.path, Path::new("a.txt"), "base\n", "initial");
    let base = git::BranchRef {
      name: git::current_branch_status(&repo.path)
        .expect("branch status")
        .name,
      kind: git::BranchKind::Local,
    };
    let feature = git::BranchRef {
      name: "feature".to_string(),
      kind: git::BranchKind::Local,
    };
    git::create_branch(&repo.path, &feature.name).expect("create branch");
    git::switch_branch(&repo.path, &feature).expect("switch to feature");
    commit_text_file(&repo.path, Path::new("a.txt"), "feature\n", "feature work");
    git::switch_branch(&repo.path, &base).expect("switch back");
    commit_text_file(&repo.path, Path::new("a.txt"), "main\n", "main work");
    let _ = git::merge_branch(&repo.path, &feature);
    std::fs::write(repo.path.join("a.txt"), "resolved\n").expect("resolve conflict");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;
    panel.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    panel.read_with(cx, |panel, _| {
      assert!(panel.merge_in_progress());
      assert!(!panel.rebase_in_progress(), "a merge is not a rebase");
    });
    assert!(
      cx.debug_bounds(DOCK_PANEL_OPERATION_DEBUG_SELECTOR)
        .is_some(),
      "the panel says a merge is running"
    );

    // Unlike a rebase, the button still commits: that is how a merge ends.
    panel.update_in(cx, |panel, window, cx| {
      panel.set_commit_message("Merge branch 'feature'", window, cx)
    });
    let button = cx
      .debug_bounds(DOCK_PANEL_COMMIT_DEBUG_SELECTOR)
      .expect("commit button bounds");
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    let commit = panel.update(cx, |panel, _| panel._commit_task.take());
    if let Some(task) = commit {
      task.await;
    }
    cx.run_until_parked();

    assert!(!git::is_merge_in_progress(&repo.path).expect("merge state"));
  }

  #[gpui::test]
  async fn a_branch_without_a_pull_request_offers_both_ways_to_open_one(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_component::init(cx));
    let repo = TempRepo::init("dock-panel-create-pr");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;

    panel.update(cx, |panel, cx| {
      panel.branch_pr = BranchPrState::Missing(GithubBranchContext {
        owner: "acme".to_string(),
        repo: "widget".to_string(),
        branch: "feature".to_string(),
      });
      panel.active_tab = DockPanelTab::PullRequest;
      cx.notify();
    });
    cx.run_until_parked();

    assert!(
      cx.debug_bounds(DOCK_PANEL_CREATE_PR_DEBUG_SELECTOR)
        .is_some(),
      "the dialog stays the default path"
    );
    assert!(
      cx.debug_bounds(DOCK_PANEL_COMPARE_DEBUG_SELECTOR).is_some(),
      "github.com covers what the dialog does not"
    );

    // A branch that already has a pull request offers neither.
    panel.update(cx, |panel, cx| {
      panel.branch_pr = BranchPrState::NoRemote;
      cx.notify();
    });
    cx.run_until_parked();
    assert!(
      cx.debug_bounds(DOCK_PANEL_CREATE_PR_DEBUG_SELECTOR)
        .is_none()
    );
    assert!(cx.debug_bounds(DOCK_PANEL_COMPARE_DEBUG_SELECTOR).is_none());
  }

  #[gpui::test]
  async fn the_commit_menu_offers_what_the_repository_allows(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_component::init(cx));
    let repo = TempRepo::init("dock-panel-commit-menu");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");
    commit_text_file(&repo.path, Path::new("b.txt"), "v1\n", "second");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;
    panel.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    assert!(
      cx.debug_bounds(DOCK_PANEL_COMMIT_MENU_DEBUG_SELECTOR)
        .is_some(),
      "the commit button carries its menu"
    );

    // Two commits and no branch handed over yet: amend and undo, no push.
    panel.read_with(cx, |panel, _| {
      let state = panel.repo_state("");
      assert!(state.allows(PaletteCommand::Amend));
      assert!(state.allows(PaletteCommand::UndoLastCommit));
      assert!(
        !state.allows(PaletteCommand::Push),
        "the panel knows no branch yet"
      );
    });

    // The host hands the branch over: publishing becomes possible.
    panel.update(cx, |panel, cx| {
      panel.set_branch_status(
        Some(git::BranchStatus {
          name: "main".to_string(),
          ahead: 0,
          behind: 0,
          has_upstream: false,
        }),
        cx,
      )
    });
    panel.read_with(cx, |panel, _| {
      assert!(
        panel.repo_state("").allows(PaletteCommand::Push),
        "an unpublished branch is pushed to publish it"
      );
    });

    // The menu asks the host to run, it never runs the command itself.
    let asked = Arc::new(AtomicBool::new(false));
    let observer = {
      let asked = asked.clone();
      cx.update(|_, cx| {
        cx.subscribe(&panel, move |_panel, event: &DockPanelEvent, _cx| {
          if matches!(event, DockPanelEvent::RunCommand(CommitMenuCommand::Amend)) {
            asked.store(true, Ordering::SeqCst);
          }
        })
      })
    };
    panel.update(cx, |_, cx| {
      cx.emit(DockPanelEvent::RunCommand(CommitMenuCommand::Amend))
    });
    cx.run_until_parked();
    assert!(asked.load(Ordering::SeqCst));
    drop(observer);
  }

  #[gpui::test]
  async fn a_rebase_in_progress_replaces_the_commit_button(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_component::init(cx));
    let repo = TempRepo::init("dock-panel-rebase");
    commit_text_file(&repo.path, Path::new("a.txt"), "base\n", "initial");
    let base = git::BranchRef {
      name: git::current_branch_status(&repo.path)
        .expect("branch status")
        .name,
      kind: git::BranchKind::Local,
    };
    let feature = git::BranchRef {
      name: "feature".to_string(),
      kind: git::BranchKind::Local,
    };
    git::create_branch(&repo.path, &feature.name).expect("create branch");
    git::switch_branch(&repo.path, &feature).expect("switch to feature");
    commit_text_file(&repo.path, Path::new("a.txt"), "feature\n", "feature work");
    git::switch_branch(&repo.path, &base).expect("switch back");
    commit_text_file(&repo.path, Path::new("a.txt"), "main\n", "main work");
    git::switch_branch(&repo.path, &feature).expect("switch to feature");
    let _ = git::rebase_branch(&repo.path, &base);

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;
    panel.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    panel.read_with(cx, |panel, _| assert!(panel.rebase_in_progress()));
    assert!(
      cx.debug_bounds(DOCK_PANEL_OPERATION_DEBUG_SELECTOR)
        .is_some(),
      "the panel says a rebase is running"
    );

    // The button now continues the rebase; the host runs it.
    let asked = Arc::new(AtomicBool::new(false));
    let observer = {
      let asked = asked.clone();
      cx.update(|_, cx| {
        cx.subscribe(&panel, move |_panel, event: &DockPanelEvent, _cx| {
          if matches!(event, DockPanelEvent::ContinueRebase) {
            asked.store(true, Ordering::SeqCst);
          }
        })
      })
    };

    // Conflicted: the button is there but does nothing yet.
    let button = cx
      .debug_bounds(DOCK_PANEL_COMMIT_DEBUG_SELECTOR)
      .expect("commit button bounds");
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(!asked.load(Ordering::SeqCst));

    // Resolved and staged: the same button asks to continue.
    std::fs::write(repo.path.join("a.txt"), "resolved\n").expect("resolve conflict");
    git::stage_all(&repo.path).expect("stage the resolution");
    await_refresh(&panel, cx).await;
    panel.update(cx, |panel, cx| {
      panel.refresh(cx);
    });
    await_refresh(&panel, cx).await;
    let button = cx
      .debug_bounds(DOCK_PANEL_COMMIT_DEBUG_SELECTOR)
      .expect("commit button bounds");
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(asked.load(Ordering::SeqCst));

    // Nothing was committed behind our back.
    assert!(git::is_rebase_in_progress(&repo.path).expect("rebase state"));
    drop(observer);
  }

  #[gpui::test]
  async fn commit_stages_and_commits_all_changes(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_component::init(cx));
    let repo = TempRepo::init("dock-panel-commit");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;

    panel.update_in(cx, |panel, window, cx| {
      panel.commit_input.update(cx, |input, cx| {
        input.set_value("feat: update readme", window, cx)
      });
    });
    // The shell refreshes its branch counters on this event.
    let committed = Arc::new(AtomicBool::new(false));
    let observer = {
      let committed = committed.clone();
      cx.update(|_, cx| {
        cx.subscribe(&panel, move |_, event: &DockPanelEvent, _| {
          if matches!(event, DockPanelEvent::Committed) {
            committed.store(true, Ordering::Relaxed);
          }
        })
      })
    };

    panel.update(cx, |panel, cx| panel.commit(cx));

    let commit_task = panel.update(cx, |panel, _| {
      panel._commit_task.take().expect("commit task")
    });
    commit_task.await;
    await_refresh(&panel, cx).await;
    drop(observer);
    assert!(
      committed.load(Ordering::Relaxed),
      "expected a Committed event"
    );

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
    let repo = TempRepo::init("dock-panel-commit-guards");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
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

  #[gpui::test]
  async fn the_terminal_starts_only_when_its_tab_is_opened(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_component::init(cx));
    let repo = TempRepo::init("dock-panel-terminal-lazy");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;

    panel.read_with(cx, |panel, _| {
      // No shell for someone who never opens the tab.
      assert!(panel.terminal_view.is_none());
      assert!(panel.active_tab != DockPanelTab::Terminal);
    });

    panel.update(cx, |panel, cx| {
      panel.active_tab = DockPanelTab::Terminal;
      panel.ensure_terminal(cx);
      cx.notify();
    });
    cx.run_until_parked();

    panel.read_with(cx, |panel, cx| {
      let terminal = panel.terminal_view.as_ref().expect("terminal view");
      assert_eq!(
        terminal.read(cx).working_directory(),
        Some(repo.path.as_path())
      );
    });
    assert!(
      cx.debug_bounds(DOCK_PANEL_TERMINAL_DEBUG_SELECTOR)
        .is_some(),
      "the terminal tab should be painted"
    );
  }

  #[gpui::test]
  async fn switching_repository_moves_a_running_terminal(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_component::init(cx));
    let repo = TempRepo::init("dock-panel-terminal-switch");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let other = TempRepo::init("dock-panel-terminal-switch-other");
    commit_text_file(&other.path, Path::new("README.md"), "other\n", "initial");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;

    panel.update(cx, |panel, cx| panel.ensure_terminal(cx));
    panel.update(cx, |panel, cx| {
      panel.set_repo_root(Some(other.path.clone()), cx)
    });
    cx.run_until_parked();

    panel.read_with(cx, |panel, cx| {
      let terminal = panel.terminal_view.as_ref().expect("terminal view");
      assert_eq!(
        terminal.read(cx).working_directory(),
        Some(other.path.as_path())
      );
    });
  }

  #[gpui::test]
  async fn the_history_tab_lists_the_commits_and_opens_one_of_their_files(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_component::init(cx));
    let repo = TempRepo::init("dock-panel-history-tab");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    commit_text_file(&repo.path, Path::new("README.md"), "v2\n", "second");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;
    panel.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    let tab = cx
      .debug_bounds(DOCK_PANEL_HISTORY_TAB_DEBUG_SELECTOR)
      .expect("history tab bounds");
    cx.simulate_click(tab.center(), gpui::Modifiers::default());
    let history = panel.read_with(cx, |panel, _| panel.history_list.clone());
    let load = history.update(cx, |list, _| list._history_task.take());
    if let Some(task) = load {
      task.await;
    }
    cx.run_until_parked();

    panel.read_with(cx, |panel, _| {
      assert_eq!(panel.active_tab, DockPanelTab::History);
    });
    assert!(
      cx.debug_bounds(crate::history_list::HISTORY_LIST_DEBUG_SELECTOR)
        .is_some(),
      "the history tab shows the commit tree"
    );

    // The panel forwards the file of a commit to whoever hosts it.
    let opened = Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = opened.clone();
    cx.update(|_, cx| {
      cx.subscribe(&panel, move |_panel, event: &DockPanelEvent, _cx| {
        if let DockPanelEvent::OpenCommitFile { commit_oid, path } = event {
          seen.borrow_mut().push((commit_oid.clone(), path.clone()));
        }
      })
      .detach();
    });

    let head = git::current_head_sha(&repo.path)
      .expect("head sha")
      .expect("head sha");
    history.update(cx, |list, cx| {
      list.open_commit_file(head.clone(), PathBuf::from("README.md"), cx)
    });
    cx.run_until_parked();

    assert_eq!(
      opened.borrow().as_slice(),
      &[(head, PathBuf::from("README.md"))]
    );
  }

  #[gpui::test]
  async fn clicking_the_terminal_tab_opens_a_shell(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_component::init(cx));
    let repo = TempRepo::init("dock-panel-terminal-click");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;
    panel.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    assert!(
      cx.debug_bounds(DOCK_PANEL_REFRESH_DEBUG_SELECTOR).is_some(),
      "the refresh button belongs to the review tabs"
    );

    let tab = cx
      .debug_bounds(DOCK_PANEL_TERMINAL_TAB_DEBUG_SELECTOR)
      .expect("terminal tab bounds");
    cx.simulate_click(tab.center(), gpui::Modifiers::default());
    cx.run_until_parked();

    panel.read_with(cx, |panel, _| {
      assert_eq!(panel.active_tab, DockPanelTab::Terminal);
      assert!(panel.terminal_view.is_some());
    });
    assert!(
      cx.debug_bounds(DOCK_PANEL_TERMINAL_DEBUG_SELECTOR)
        .is_some()
    );
    assert!(
      cx.debug_bounds(DOCK_PANEL_REFRESH_DEBUG_SELECTOR).is_none(),
      "nothing to refresh on the terminal tab"
    );
  }

  #[gpui::test]
  async fn reopening_the_terminal_tab_keeps_the_same_shell(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_component::init(cx));
    let repo = TempRepo::init("dock-panel-terminal-reuse");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;

    let first = panel.update(cx, |panel, cx| {
      panel.ensure_terminal(cx);
      panel.terminal_view.clone().expect("terminal view")
    });
    let second = panel.update(cx, |panel, cx| {
      panel.ensure_terminal(cx);
      panel.terminal_view.clone().expect("terminal view")
    });

    // A second shell would leak a process on every visit to the tab.
    assert_eq!(first.entity_id(), second.entity_id());
  }

  #[gpui::test]
  async fn staging_from_the_changes_list_refreshes_the_panel(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_component::init(cx));
    let repo = TempRepo::init("dock-panel-changes-list");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");

    let (panel, cx) = add_dock_panel_window(Some(repo.path.clone()), cx);
    await_refresh(&panel, cx).await;
    panel.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    panel.read_with(cx, |panel, cx| {
      assert_eq!(panel.status_entries.len(), 1);
      assert_eq!(panel.status_entries[0].stage, RepoStage::Unstaged);
      // The panel feeds the shared list.
      assert_eq!(panel.changes_list.read(cx).entries().len(), 1);
    });

    let button = cx
      .debug_bounds("changes-stage-0-0")
      .expect("stage button bounds");
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    let action = panel.update(cx, |panel, cx| {
      panel
        .changes_list
        .update(cx, |list, _| list._action_task.take().expect("stage task"))
    });
    action.await;
    cx.run_until_parked();
    await_refresh(&panel, cx).await;

    panel.read_with(cx, |panel, _| {
      assert!(
        panel
          .status_entries
          .iter()
          .all(|entry| entry.stage != RepoStage::Unstaged),
        "the panel should have refreshed after the staging action"
      );
    });
  }
}
