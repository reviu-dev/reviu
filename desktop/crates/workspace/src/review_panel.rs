//! Session review panel: working-tree changeset and commit actions.

#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;

use git::{
  RepoStage, RepoStatusEntry, commit_changes, current_branch_status, current_github_remote_repo,
  list_repo_status, list_repo_worktree_files, stage_all,
};
use gpui::{
  AnyElement, AnyWindowHandle, App, Context, Entity, FocusHandle, Focusable, Render, SharedString,
  Task, Window, div, img, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, h_flex,
  tree::{TreeItem, TreeState, tree},
  v_flex,
};
use smol::unblock;
use terminal::TerminalView;

use crate::changes_list::{ChangesList, ChangesListEvent};

const REVIEW_PANEL_TERMINAL_DEBUG_SELECTOR: &str = "review-panel-terminal";
#[cfg(test)]
const REVIEW_PANEL_TERMINAL_TAB_DEBUG_SELECTOR: &str = "review-panel-tab-terminal";
const REVIEW_PANEL_REFRESH_DEBUG_SELECTOR: &str = "review-panel-refresh";
use std::collections::BTreeMap;
use std::rc::Rc;

use crate::api::GithubPullRequest;
use crate::auth_state::AuthStateStore;
use crate::git_page::{
  GithubBranchContext, PullRequestCreatedHandler, open_create_pull_request_dialog,
};
use crate::github_navigation::open_pr_target;
use crate::github_shared::{pull_request_status_color, pull_request_status_label};
use crate::workspace::WorkspaceApi;
use ui::{Button, ButtonVariants as _, StatusThemeExt as _, Textarea, TextareaState, UiIconName};

#[derive(Clone, Debug)]
pub enum ReviewPanelEvent {
  OpenFile {
    path: PathBuf,
  },
  /// A commit landed: whoever shows the branch state has to refresh it.
  Committed,
}

impl gpui::EventEmitter<ReviewPanelEvent> for ReviewPanel {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReviewPanelTab {
  Changes,
  Files,
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
enum BranchPrState {
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

pub struct ReviewPanel {
  focus_handle: FocusHandle,
  window_handle: AnyWindowHandle,
  repo_root: Option<PathBuf>,
  status_entries: Vec<RepoStatusEntry>,
  commit_input: Entity<TextareaState>,
  committing: bool,
  last_error: Option<SharedString>,
  active_tab: ReviewPanelTab,
  changes_list: Entity<ChangesList>,
  /// Spawned on the first visit to the tab: a shell per session is too much
  /// for someone who never opens it.
  terminal_view: Option<Entity<TerminalView>>,
  branch_pr: BranchPrState,
  files_tree_state: Entity<TreeState>,
  files_loaded: bool,
  files_loading: bool,
  selected_tree_id: Option<String>,
  _refresh_task: Option<Task<()>>,
  _commit_task: Option<Task<()>>,
  _pr_task: Option<Task<()>>,
  _files_task: Option<Task<()>>,
}

impl ReviewPanel {
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
          cx.emit(ReviewPanelEvent::OpenFile { path: path.clone() });
        }
        ChangesListEvent::Changed { .. } => this.refresh(cx),
      },
    )
    .detach();

    let mut panel = Self {
      focus_handle: cx.focus_handle(),
      window_handle: window.window_handle(),
      repo_root,
      status_entries: Vec::new(),
      commit_input,
      committing: false,
      last_error: None,
      active_tab: ReviewPanelTab::Changes,
      changes_list,
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
      let files = unblock(move || list_repo_worktree_files(&repo_root)).await;
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
            this.changes_list.update(cx, |list, cx| {
              list.set_entries(entries.clone(), cx);
            });
            this.status_entries = entries;
            this.last_error = None;
          }
          Err(error) => this.last_error = Some(format!("{error}").into()),
        }
        cx.notify();
      });
    });
    self._refresh_task = Some(task);
    self.refresh_branch_pull_request(cx);
    if self.files_loaded {
      self.load_worktree_files(cx);
    }
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
      let state = unblock(move || {
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
    if let Some(terminal) = self.terminal_view.clone() {
      terminal.update(cx, |terminal, cx| {
        terminal.set_working_directory(repo_root, cx);
      });
    }
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
      .id("review-panel-terminal")
      .debug_selector(|| REVIEW_PANEL_TERMINAL_DEBUG_SELECTOR.to_string())
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
            cx.emit(ReviewPanelEvent::Committed);
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
    let can_commit = !self.committing
      && !self.status_entries.is_empty()
      && !self.commit_input.read(cx).value().trim().is_empty();
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
      .child(
        Button::new("review-panel-commit")
          .label("Commit")
          .with_variant(gpui_component::button::ButtonVariant::Secondary)
          .outline()
          .small()
          .w_full()
          .child(gpui_component::kbd::Kbd::new(commit_shortcut).ml_1())
          .loading(self.committing)
          .disabled(!can_commit)
          .on_click(cx.listener(|this, _, _, cx| this.commit(cx))),
      )
      .into_any_element()
  }

  fn render_tabs(&self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let tab = |id: &'static str, label: &'static str, target: ReviewPanelTab, active: bool| {
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
        .on_click(cx.listener(move |this, _, _, cx| {
          if this.active_tab != target {
            this.active_tab = target;
            match target {
              ReviewPanelTab::PullRequest => this.refresh_branch_pull_request(cx),
              ReviewPanelTab::Terminal => this.ensure_terminal(cx),
              ReviewPanelTab::Changes | ReviewPanelTab::Files => {}
            }
            cx.notify();
          }
        }))
    };

    h_flex()
      .items_center()
      .gap_1()
      .child(tab(
        "review-panel-tab-changes",
        "Changes",
        ReviewPanelTab::Changes,
        self.active_tab == ReviewPanelTab::Changes,
      ))
      .child(tab(
        "review-panel-tab-files",
        "Files",
        ReviewPanelTab::Files,
        self.active_tab == ReviewPanelTab::Files,
      ))
      .child(tab(
        "review-panel-tab-pr",
        "Pull request",
        ReviewPanelTab::PullRequest,
        self.active_tab == ReviewPanelTab::PullRequest,
      ))
      .child(tab(
        "review-panel-tab-terminal",
        "Terminal",
        ReviewPanelTab::Terminal,
        self.active_tab == ReviewPanelTab::Terminal,
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
          cx.emit(ReviewPanelEvent::OpenFile { path });
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
            Button::new("review-panel-create-pr")
              .primary()
              .small()
              .label("Create pull request")
              .on_click(cx.listener(move |this, _, window, cx| {
                open_create_pull_request_dialog(
                  WorkspaceApi::global(cx).api.clone(),
                  this.window_handle,
                  this.pr_created_handler(cx),
                  context.clone(),
                  window,
                  cx,
                );
              })),
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
            Button::new("review-panel-open-pr")
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

impl Render for ReviewPanel {
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
            self.active_tab == ReviewPanelTab::Changes && entry_count > 0,
            |this| {
              this.child(
                div()
                  .text_xs()
                  .text_color(theme.muted_foreground)
                  .child(entry_count.to_string()),
              )
            },
          )
          .when(self.active_tab != ReviewPanelTab::Terminal, |this| {
            this.child(
              Button::new("review-panel-refresh")
                .debug_selector(|| REVIEW_PANEL_REFRESH_DEBUG_SELECTOR.to_string())
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
      ReviewPanelTab::Files => self.render_files_tab(_window, cx),
      ReviewPanelTab::Changes => {
        if self.status_entries.is_empty() {
          self.render_empty_state(cx)
        } else {
          div()
            .id("review-panel-file-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .py_1()
            .child(self.changes_list.clone())
            .into_any_element()
        }
      }
      ReviewPanelTab::PullRequest => self.render_pr_tab(cx),
      ReviewPanelTab::Terminal => self.render_terminal_tab(),
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
    if self.active_tab == ReviewPanelTab::Changes {
      panel = panel.child(self.render_commit_zone(_window, cx));
    }
    panel
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
  use git::RepoStatusKind;
  use git2::{Repository, Signature};
  use gpui::TestAppContext;
  use std::path::Path;
  use std::sync::Arc;
  use std::sync::atomic::{AtomicBool, Ordering};
  use std::time::{SystemTime, UNIX_EPOCH};

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
    let mut mounted: Option<Entity<ReviewPanel>> = None;
    let (_root, cx) = cx.add_window_view(|window, cx| {
      let panel = cx.new(|cx| ReviewPanel::new(repo_root.clone(), window, cx));
      mounted = Some(panel.clone());
      gpui_component::Root::new(panel, window, cx)
    });
    (mounted.expect("review panel"), cx)
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
    let repo = TempRepo::init("review-panel-pr-gate");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (panel, cx) = add_review_panel_window(Some(repo.path.clone()), cx);
    cx.executor().allow_parking();
    cx.run_until_parked();

    // Default auth state is Unknown: no GitHub access, no lookup attempted.
    panel.read_with(cx, |panel, _| {
      assert!(matches!(panel.branch_pr, BranchPrState::NoAccess));
    });
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
      panel.commit_input.update(cx, |input, cx| {
        input.set_value("feat: update readme", window, cx)
      });
    });
    // The shell refreshes its branch counters on this event.
    let committed = Arc::new(AtomicBool::new(false));
    let observer = {
      let committed = committed.clone();
      cx.update(|_, cx| {
        cx.subscribe(&panel, move |_, event: &ReviewPanelEvent, _| {
          if matches!(event, ReviewPanelEvent::Committed) {
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

  #[gpui::test]
  async fn the_terminal_starts_only_when_its_tab_is_opened(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_component::init(cx));
    let repo = TempRepo::init("review-panel-terminal-lazy");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (panel, cx) = add_review_panel_window(Some(repo.path.clone()), cx);
    cx.executor().allow_parking();
    await_refresh(&panel, cx).await;

    panel.read_with(cx, |panel, _| {
      // No shell for someone who never opens the tab.
      assert!(panel.terminal_view.is_none());
      assert!(panel.active_tab != ReviewPanelTab::Terminal);
    });

    panel.update(cx, |panel, cx| {
      panel.active_tab = ReviewPanelTab::Terminal;
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
      cx.debug_bounds(REVIEW_PANEL_TERMINAL_DEBUG_SELECTOR)
        .is_some(),
      "the terminal tab should be painted"
    );
  }

  #[gpui::test]
  async fn switching_repository_moves_a_running_terminal(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_component::init(cx));
    let repo = TempRepo::init("review-panel-terminal-switch");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let other = TempRepo::init("review-panel-terminal-switch-other");
    commit_text_file(&other.path, Path::new("README.md"), "other\n", "initial");

    let (panel, cx) = add_review_panel_window(Some(repo.path.clone()), cx);
    cx.executor().allow_parking();
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
  async fn clicking_the_terminal_tab_opens_a_shell(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_component::init(cx));
    let repo = TempRepo::init("review-panel-terminal-click");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (panel, cx) = add_review_panel_window(Some(repo.path.clone()), cx);
    cx.executor().allow_parking();
    await_refresh(&panel, cx).await;
    panel.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    assert!(
      cx.debug_bounds(REVIEW_PANEL_REFRESH_DEBUG_SELECTOR)
        .is_some(),
      "the refresh button belongs to the review tabs"
    );

    let tab = cx
      .debug_bounds(REVIEW_PANEL_TERMINAL_TAB_DEBUG_SELECTOR)
      .expect("terminal tab bounds");
    cx.simulate_click(tab.center(), gpui::Modifiers::default());
    cx.run_until_parked();

    panel.read_with(cx, |panel, _| {
      assert_eq!(panel.active_tab, ReviewPanelTab::Terminal);
      assert!(panel.terminal_view.is_some());
    });
    assert!(
      cx.debug_bounds(REVIEW_PANEL_TERMINAL_DEBUG_SELECTOR)
        .is_some()
    );
    assert!(
      cx.debug_bounds(REVIEW_PANEL_REFRESH_DEBUG_SELECTOR)
        .is_none(),
      "nothing to refresh on the terminal tab"
    );
  }

  #[gpui::test]
  async fn reopening_the_terminal_tab_keeps_the_same_shell(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_component::init(cx));
    let repo = TempRepo::init("review-panel-terminal-reuse");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (panel, cx) = add_review_panel_window(Some(repo.path.clone()), cx);
    cx.executor().allow_parking();
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
    let repo = TempRepo::init("review-panel-changes-list");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");

    let (panel, cx) = add_review_panel_window(Some(repo.path.clone()), cx);
    cx.executor().allow_parking();
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
