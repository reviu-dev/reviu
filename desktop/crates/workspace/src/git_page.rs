use std::{
  collections::{HashMap, HashSet},
  path::{Path, PathBuf},
  rc::Rc,
  sync::Arc,
  time::{Duration, Instant},
};

use agent_chat_panel::AgentChatPanel;
use editor::{
  CloseFind, ConflictNavigationDirection, ConflictNavigationState, ConflictResolution,
  DiffViewMode, Editor, Find, HunkAction, HunkNavigationDirection, HunkState, ReviewComment,
  ReviewCommentCancelHandler, ReviewCommentCreateHandler, ReviewCommentCreateRequest,
  ReviewCommentDeleteHandler, ReviewCommentDisplayMode, ReviewCommentEditHandler,
  ReviewCommentSide,
};
use gfm_markdown_viewer::{MarkdownRenderOptions, SuggestionContext, render_markdown};
use git::{
  BranchKind, BranchRef, BranchStatus, CommitChangedFile, CommitFileChangeKind, HeadCommitStatus,
  HistoryCommitNode, HistoryRevision, InteractiveRebaseTarget, InteractiveRebaseTodoEntry,
  MergeBranchOutcome, PullOutcome, RebaseBranchOutcome, RepoStage, RepoStatusEntry, RepoStatusKind,
  abort_merge, abort_rebase, amend_commit, apply_stash, branch_has_unpublished_commits,
  checkout_detached_target, cherry_pick_commits, commit_changes, continue_rebase, create_branch,
  create_branch_from, create_stash, current_branch_status, current_branch_upstream,
  current_github_remote_repo, current_head_sha, current_history_revision,
  current_rebase_commit_message, default_remote_branch, default_stash_message, delete_branch,
  delete_untracked_file, detached_head_label, diff_set_from_patch, drop_stash, fetch,
  head_commit_status, is_merge_in_progress, is_rebase_in_progress, list_branches,
  list_commit_changed_files, list_commit_history, list_interactive_rebase_commits,
  list_repo_head_files, list_repo_status, list_stashes, load_commit_file_diff, merge_branch,
  pop_stash, pull, push, rebase_branch, resolve_branch_ref, restore_file, restore_renamed_file,
  skip_rebase, stage_all, stage_file, start_interactive_rebase, switch_branch, undo_last_commit,
  unstage_all, unstage_file,
};
use gpui::{
  AnyElement, AnyWindowHandle, App, ClipboardItem, Context, Corner, Entity, FocusHandle, Focusable,
  Global, Image, InteractiveElement, ObjectFit, ParentElement, PathPromptOptions, Pixels, Render,
  RenderImage, SharedString, Styled, Subscription, Task, WeakEntity, Window, actions, div, img,
  prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Disableable, Icon, IconName, IndexPath, Selectable, Sizable, StyledExt,
  button::{Button, ButtonGroup, ButtonVariant, ButtonVariants as _},
  checkbox::Checkbox,
  dialog::{DialogDescription, DialogFooter, DialogHeader, DialogTitle},
  h_flex,
  input::InputEvent,
  kbd::Kbd,
  list::{List, ListDelegate, ListEvent, ListItem, ListState},
  menu::{DropdownMenu, PopupMenuItem},
  notification::Notification,
  select::{Select, SelectEvent, SelectState},
  spinner::Spinner,
  tag::Tag,
  text::TextView,
  tooltip::Tooltip,
  tree::{TreeItem, TreeState, tree},
  v_flex,
};
use sentry::protocol::{Map, Value};
use smol::unblock;

use crate::agent_settings::AgentSettings;

fn agent_chat_state_dir() -> Option<std::path::PathBuf> {
  Some(dirs::config_dir()?.join("reviu").join("agent-chats"))
}

fn prune_agent_chat_state_once() {
  use std::sync::OnceLock;
  static PRUNED: OnceLock<()> = OnceLock::new();
  PRUNED.get_or_init(|| {
    if let Some(dir) = agent_chat_state_dir() {
      let _ =
        AgentChatPanel::prune_old_state(&dir, std::time::Duration::from_secs(60 * 60 * 24 * 30));
    }
  });
}
use terminal::TerminalView;

use crate::{
  active_local_repo::{ActiveLocalRepo, ActiveLocalRepoStore},
  api::{ApiClient, GithubPullRequest},
  auth_state::{AuthState, AuthStateStore},
  config::{AppSettings, ConfigStore, RecentRepository},
  dock_badge::set_dock_badge,
  file_preview::{
    FilePreviewKind, file_preview_kind, is_markdown_path, is_previewable_path, is_svg_path,
    raster_image_from_bytes, should_show_unsupported_binary_placeholder,
  },
  file_search_palette::open_file_search_palette as open_shared_file_search_palette,
  github_commit_details_page::GithubCommitDetailsPageHandle,
  github_navigation::should_open_externally,
  github_page::GithubPageHandle,
  github_pr_details_page::GithubPrDetailsPageHandle,
  github_profile_page::GithubProfilePageHandle,
  github_repo_page::GithubRepoPageHandle,
  github_shared,
  interactive_rebase_todo_view::{
    InteractiveRebaseTodoView, InteractiveRebaseTodoViewCancelHandler,
    InteractiveRebaseTodoViewConfig, InteractiveRebaseTodoViewHandler,
  },
  navigation::NavigationHistory,
  notification_count::NotificationCountStore,
  sentry_context,
  shortcuts::{self, ShortcutId},
  workspace::WorkspaceApi,
};
use ui::{
  CommandPalette, CommandPaletteAction, CommandPaletteBranch, CommandPaletteBranchKind,
  CommandPaletteCommand, CommandPaletteConfig, CommandPaletteGithubRepoTab, CommandPaletteHandler,
  CommandPaletteInitialScreen, CommandPalettePage, CommandPaletteRepository, CommandPaletteStash,
  ConfirmDialog, DropdownSelectConfig, DropdownSelectItem, FILE_ICON_SIZE_PX, Input, InputState,
  PAGE_HEADER_HEIGHT, SearchFileEntry, SearchFileHandler, SelectableRowStyle, StatusAlert,
  StatusThemeExt, UiIconName, WindowExt, dropdown_select, file_icon_path_for_path_with_theme,
  selectable_list_item,
};

const SIDEBAR_DEFAULT_WIDTH: f32 = 400.0;
const SIDEBAR_MIN_WIDTH: f32 = 250.0;
const SIDEBAR_MAX_WIDTH: f32 = 1500.0;
const STATUS_POLL_INTERVAL_MS: u64 = 3_000;
const INACTIVE_STATUS_POLL_INTERVAL: Duration = Duration::from_secs(60);
const UNPUBLISHED_BRANCH_RECHECK_INTERVAL: Duration = Duration::from_secs(30);
const EDITOR_HEADER_HEIGHT: f32 = 40.0;
const HISTORY_MAX_COMMITS: usize = 200;
const HISTORY_AUTHOR_MAX_WIDTH: f32 = 180.0;
const DETACHED_BRANCH_SELECT_SENTINEL: &str = "__reviu_detached_head__";
const TRIGGER_DROPDOWN_SELECT_WIDTH: f32 = 350.0;
const EMPTY_REPOSITORY_TITLE: &str = "Select a repository";
const EMPTY_REPOSITORY_HINT_PREFIX: &str = "Press";
const EMPTY_REPOSITORY_HINT_SUFFIX: &str = "to add a repository.";
const GIT_MARKDOWN_PREVIEW_EDITOR_DEBUG_SELECTOR: &str = "git-markdown-preview-editor-pane";
const GIT_MARKDOWN_PREVIEW_RENDER_DEBUG_SELECTOR: &str = "git-markdown-preview-render-pane";
const GIT_BINARY_PREVIEW_RENDER_DEBUG_SELECTOR: &str = "git-binary-preview-render-pane";
const GIT_TERMINAL_BUTTON_DEBUG_SELECTOR: &str = "git-terminal-button";
const GIT_TERMINAL_SIDEBAR_DEBUG_SELECTOR: &str = "git-terminal-sidebar";
const TERMINAL_SIDEBAR_DEFAULT_WIDTH: f32 = 420.0;
const TERMINAL_SIDEBAR_MIN_WIDTH: f32 = 280.0;
const TERMINAL_SIDEBAR_MAX_WIDTH: f32 = 1200.0;
const GIT_AGENT_BUTTON_DEBUG_SELECTOR: &str = "git-agent-button";
const GIT_AGENT_SIDEBAR_DEBUG_SELECTOR: &str = "git-agent-sidebar";
const AGENT_SIDEBAR_DEFAULT_WIDTH: f32 = 480.0;
const AGENT_SIDEBAR_MIN_WIDTH: f32 = 320.0;
const AGENT_SIDEBAR_MAX_WIDTH: f32 = 1200.0;

type RepoSelectHandler = Rc<dyn Fn(PathBuf, &mut Window, &mut App)>;
type BranchSelectHandler = Rc<dyn Fn(BranchRef, &mut Window, &mut App)>;

fn interactive_rebase_success_message(target: &InteractiveRebaseTarget) -> String {
  match target {
    InteractiveRebaseTarget::Branch(branch) => {
      format!("Rebased interactively onto {}", branch.name)
    }
    InteractiveRebaseTarget::BranchInPlace(branch) => {
      format!("Edited commits since {}", branch.name)
    }
    InteractiveRebaseTarget::HeadCount(count) => {
      format!("Rebased last {count} commits")
    }
  }
}

fn render_image_preview_status_message(
  message: impl Into<SharedString>,
  color: gpui::Hsla,
) -> AnyElement {
  div()
    .w(px(280.0))
    .max_w_full()
    .px_3()
    .text_sm()
    .text_center()
    .whitespace_normal()
    .text_color(color)
    .child(message.into())
    .into_any_element()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileStageButtonAction {
  Stage,
  Unstage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct GithubBranchContext {
  owner: String,
  repo: String,
  branch: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum GitBranchPullRequestButtonState {
  Hidden,
  Checking,
  PublishAndCreate,
  OpenExisting {
    owner: String,
    repo: String,
    number: u64,
  },
  Create,
}

struct GitBranchSwitchNotificationId;
struct GitActionErrorNotificationId;

#[derive(Clone, Debug, PartialEq, Eq)]
enum GitPageOpenAction {
  MergeBaseBranch { base_branch_name: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ActiveConflictResolutionSnapshot {
  merge_in_progress: bool,
  rebase_in_progress: bool,
  conflicted_path: Option<PathBuf>,
}

enum GitPageOpenActionResult {
  ResumeActiveConflict(ActiveConflictResolutionSnapshot),
  MergeBaseBranchReady(BranchRef),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GitCommitPrimaryButtonState {
  ContinueRebase,
  Commit,
  PublishBranch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AnnotationKind {
  Conflict,
  Change,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AnnotationNavigationState {
  active_index: usize,
  total: usize,
  kind: AnnotationKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AnnotationDirection {
  Previous,
  Next,
}

impl AnnotationDirection {
  fn conflict(self) -> ConflictNavigationDirection {
    match self {
      Self::Previous => ConflictNavigationDirection::Previous,
      Self::Next => ConflictNavigationDirection::Next,
    }
  }

  fn hunk(self) -> HunkNavigationDirection {
    match self {
      Self::Previous => HunkNavigationDirection::Previous,
      Self::Next => HunkNavigationDirection::Next,
    }
  }
}

#[derive(Clone)]
enum GitBinaryPreview {
  RasterImage(Arc<Image>),
  UnsupportedBinary,
}

fn git_refresh_in_progress(status_loading: bool, branch_loading: bool) -> bool {
  status_loading || branch_loading
}

#[derive(Clone, Default)]
pub struct GitPageHandle {
  git_page: Option<WeakEntity<GitPage>>,
}

impl Global for GitPageHandle {}

impl GitPageHandle {
  pub fn register(cx: &mut Context<GitPage>) {
    cx.set_global(Self {
      git_page: Some(cx.entity().downgrade()),
    });
  }

  pub fn is_refreshing(cx: &App) -> bool {
    let Some(weak) = cx
      .try_global::<Self>()
      .and_then(|handle| handle.git_page.clone())
    else {
      return false;
    };

    weak
      .read_with(cx, |this, _cx| {
        git_refresh_in_progress(
          this.status_refresh_in_progress,
          this.branch_refresh_in_progress,
        )
      })
      .unwrap_or(false)
  }

  pub fn refresh_page(cx: &mut App) {
    let Some(weak) = cx.global::<Self>().git_page.clone() else {
      return;
    };
    let _ = weak.update(cx, |this, cx| this.refresh_current_page(cx));
  }

  pub fn show_repository_and_merge_base(
    repo_root: PathBuf,
    base_branch_name: String,
    cx: &mut App,
  ) {
    NavigationHistory::navigate("/git", cx);

    let Some(weak) = cx
      .try_global::<Self>()
      .and_then(|handle| handle.git_page.clone())
    else {
      return;
    };

    let _ = weak.update(cx, |this, cx| {
      this.open_repository_with_action(
        repo_root,
        GitPageOpenAction::MergeBaseBranch { base_branch_name },
        cx,
      );
    });
  }

  pub fn show_repository(repo_root: PathBuf, cx: &mut App) {
    NavigationHistory::navigate("/git", cx);

    let Some(weak) = cx
      .try_global::<Self>()
      .and_then(|handle| handle.git_page.clone())
    else {
      ConfigStore::persist_recent_repository(&repo_root);
      return;
    };

    let _ = weak.update(cx, |this, cx| {
      this.set_selected_repo(repo_root, cx);
    });
  }
}

struct CreatePullRequestDialog {
  api: ApiClient,
  window_handle: AnyWindowHandle,
  git_page: WeakEntity<GitPage>,
  branch_context: GithubBranchContext,
  title_input: Entity<InputState>,
  base_input: Entity<InputState>,
  body_input: Entity<InputState>,
  template_select: Entity<SelectState<Vec<String>>>,
  draft: bool,
  default_branch_loading: bool,
  template_loading: bool,
  template_options_count: usize,
  submit_loading: bool,
  validation_error: Option<SharedString>,
  default_branch_task: Option<Task<()>>,
  template_task: Option<Task<()>>,
  submit_task: Option<Task<()>>,
  _subscriptions: Vec<Subscription>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PullRequestTemplateLoadResult {
  default_branch: Option<String>,
  template_paths: Vec<String>,
  template_body: Option<String>,
}

const PULL_REQUEST_TEMPLATE_SINGLE_PATHS: [&str; 3] = [
  ".github/pull_request_template.md",
  "pull_request_template.md",
  "docs/pull_request_template.md",
];
const PULL_REQUEST_TEMPLATE_DIRECTORY_PATHS: [&str; 3] = [
  ".github/PULL_REQUEST_TEMPLATE/",
  "PULL_REQUEST_TEMPLATE/",
  "docs/PULL_REQUEST_TEMPLATE/",
];

fn resolve_pull_request_template_paths(
  entries: &[crate::api::GithubRepositoryTreeEntry],
) -> Vec<String> {
  let mut paths = Vec::new();

  for candidate in PULL_REQUEST_TEMPLATE_SINGLE_PATHS {
    if entries
      .iter()
      .any(|entry| entry.entry_type == "blob" && entry.path == candidate)
    {
      paths.push(candidate.to_string());
    }
  }

  for directory in PULL_REQUEST_TEMPLATE_DIRECTORY_PATHS {
    let mut directory_paths = entries
      .iter()
      .filter(|entry| entry.entry_type == "blob")
      .filter_map(|entry| {
        let suffix = entry.path.strip_prefix(directory)?;
        if suffix.is_empty() || suffix.contains('/') {
          return None;
        }

        Some(entry.path.clone())
      })
      .collect::<Vec<_>>();
    directory_paths.sort();

    for path in directory_paths {
      if !paths.contains(&path) {
        paths.push(path);
      }
    }
  }

  paths
}

impl CreatePullRequestDialog {
  fn new(
    api: ApiClient,
    window_handle: AnyWindowHandle,
    git_page: WeakEntity<GitPage>,
    branch_context: GithubBranchContext,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    let template_select = cx.new(|cx| SelectState::new(Vec::<String>::new(), None, window, cx));
    let subscription = cx.subscribe(
      &template_select,
      move |this, _, event: &SelectEvent<Vec<String>>, cx| {
        let SelectEvent::Confirm(Some(path)) = event else {
          return;
        };
        this.load_pull_request_template(path.clone(), cx);
      },
    );

    let mut this = Self {
      api,
      window_handle,
      git_page,
      branch_context,
      title_input: cx.new(|cx| InputState::new(window, cx).placeholder("Pull request title")),
      base_input: cx.new(|cx| InputState::new(window, cx).placeholder("Base branch")),
      body_input: cx.new(|cx| {
        InputState::new(window, cx)
          .auto_grow(4, 10)
          .placeholder("Add an optional description...")
      }),
      template_select,
      draft: false,
      default_branch_loading: false,
      template_loading: false,
      template_options_count: 0,
      submit_loading: false,
      validation_error: None,
      default_branch_task: None,
      template_task: None,
      submit_task: None,
      _subscriptions: vec![subscription],
    };
    this.load_repository_defaults(cx);
    this
  }

  fn load_repository_defaults(&mut self, cx: &mut Context<Self>) {
    if self.default_branch_loading {
      return;
    }

    self.default_branch_loading = true;
    self.template_loading = true;

    let api = self.api.clone();
    let owner = self.branch_context.owner.clone();
    let repo = self.branch_context.repo.clone();
    let base_input = self.base_input.clone();
    let body_input = self.body_input.clone();
    let template_select = self.template_select.clone();
    let window_handle = self.window_handle;

    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || {
        let details = api.fetch_github_repository_details(&owner, &repo).ok();
        let default_branch = details
          .as_ref()
          .map(|details| details.default_branch.trim().to_string())
          .filter(|value| !value.is_empty());

        let template_paths = default_branch
          .as_ref()
          .and_then(|default_branch| {
            api
              .fetch_github_repository_tree(&owner, &repo, default_branch)
              .ok()
              .map(|tree| resolve_pull_request_template_paths(&tree.tree))
          })
          .unwrap_or_default();

        let template_body = if template_paths.len() == 1 {
          default_branch.as_ref().and_then(|default_branch| {
            api
              .fetch_github_file_content(&owner, &repo, &template_paths[0], default_branch)
              .ok()
              .flatten()
          })
        } else {
          None
        };

        PullRequestTemplateLoadResult {
          default_branch,
          template_paths,
          template_body,
        }
      })
      .await;

      let _ = cx.update_window(window_handle, |_, window, cx| {
        let _ = this.update(cx, |this, cx| {
          this.default_branch_loading = false;
          this.template_loading = false;
          this.template_options_count = result.template_paths.len();

          if let Some(default_branch) = result.default_branch.clone()
            && base_input.read(cx).value().trim().is_empty()
          {
            base_input.update(cx, |input, cx| {
              input.set_value(default_branch, window, cx);
            });
          }

          template_select.update(cx, |state, cx| {
            state.set_items(result.template_paths.clone(), window, cx);
            state.set_selected_index(None, window, cx);
          });

          if let Some(template_body) = result.template_body.clone() {
            body_input.update(cx, |input, cx| {
              input.set_value(template_body, window, cx);
            });
          }

          cx.notify();
        });
      });
    });

    self.default_branch_task = Some(task);
    cx.notify();
  }

  fn load_pull_request_template(&mut self, template_path: String, cx: &mut Context<Self>) {
    if self.template_loading {
      return;
    }

    let base_branch = self.base_input.read(cx).value().trim().to_string();
    if base_branch.is_empty() {
      return;
    }

    self.template_loading = true;

    let api = self.api.clone();
    let owner = self.branch_context.owner.clone();
    let repo = self.branch_context.repo.clone();
    let body_input = self.body_input.clone();
    let window_handle = self.window_handle;

    let task = cx.spawn(async move |this, cx| {
      let result =
        unblock(move || api.fetch_github_file_content(&owner, &repo, &template_path, &base_branch))
          .await;

      let _ = cx.update_window(window_handle, |_, window, cx| {
        let _ = this.update(cx, |this, cx| {
          this.template_loading = false;

          if let Ok(Some(template_body)) = result {
            body_input.update(cx, |input, cx| {
              input.set_value(template_body, window, cx);
            });
          }

          cx.notify();
        });
      });
    });

    self.template_task = Some(task);
    cx.notify();
  }

  fn submit_action(&mut self, _: &gpui::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
    if self.submit_loading {
      return;
    }

    let title = self.title_input.read(cx).value().trim().to_string();
    let base = self.base_input.read(cx).value().trim().to_string();
    let body = self.body_input.read(cx).value().to_string();

    if title.is_empty() {
      self.validation_error = Some("Pull request title is required.".into());
      cx.notify();
      return;
    }

    if base.is_empty() {
      self.validation_error = Some("Base branch is required.".into());
      cx.notify();
      return;
    }

    self.validation_error = None;
    self.submit_loading = true;

    let api = self.api.clone();
    let owner = self.branch_context.owner.clone();
    let repo = self.branch_context.repo.clone();
    let branch = self.branch_context.branch.clone();
    let branch_context = self.branch_context.clone();
    let git_page = self.git_page.clone();
    let draft = self.draft;
    let window_handle = self.window_handle;

    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || {
        api.create_pull_request(
          &owner,
          &repo,
          &branch,
          &title,
          &base,
          Some(body.as_str()),
          draft,
        )
      })
      .await;

      let _ = cx.update_window(window_handle, |_, window, cx| {
        let _ = this.update(cx, |this, cx| {
          this.submit_loading = false;
          cx.notify();
        });

        match result {
          Ok(pull_request) => {
            let created_pull_request = pull_request.clone();
            let _ = git_page.update(cx, |git_page, cx| {
              git_page.apply_created_pull_request(&branch_context, &created_pull_request, cx);
            });
            window.close_dialog(cx);
            GithubPrDetailsPageHandle::show_with_open_target(
              pull_request.repository.owner.into(),
              pull_request.repository.repo.into(),
              pull_request.number,
              false,
              None,
              cx,
            );
          }
          Err(error) => {
            window.push_notification(
              Notification::error(error.to_string())
                .id::<GitActionErrorNotificationId>()
                .title("Create pull request failed"),
              cx,
            );
          }
        }
      });
    });

    self.submit_task = Some(task);
    cx.notify();
  }

  fn toggle_draft_action(&mut self, checked: bool, _: &mut Window, cx: &mut Context<Self>) {
    self.draft = checked;
    cx.notify();
  }
}

impl Focusable for CreatePullRequestDialog {
  fn focus_handle(&self, cx: &App) -> FocusHandle {
    self.title_input.read(cx).focus_handle(cx)
  }
}

impl Render for CreatePullRequestDialog {
  fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let branch_label = format!(
      "{}:{}",
      self.branch_context.owner, self.branch_context.branch
    );
    let repo_label = github_shared::repo_label(
      self.branch_context.owner.as_str(),
      self.branch_context.repo.as_str(),
    );

    div()
      .id("git-create-pull-request-dialog")
      .flex()
      .flex_col()
      .child(
        DialogHeader::new()
          .p_4()
          .child(DialogTitle::new().child("Create Pull Request"))
          .child(DialogDescription::new().child(format!(
            "Create a pull request from {branch_label} in {repo_label}."
          ))),
      )
      .child(
        v_flex()
          .px_4()
          .pb_4()
          .gap_3()
          .child(
            v_flex()
              .gap_1()
              .child(div().text_sm().child("Title"))
              .child(Input::new(&self.title_input).w_full()),
          )
          .child(
            v_flex()
              .gap_1()
              .child(
                h_flex()
                  .justify_between()
                  .items_center()
                  .child(div().text_sm().child("Base Branch"))
                  .when(self.default_branch_loading, |this| {
                    this.child(Spinner::new().xsmall())
                  }),
              )
              .child(Input::new(&self.base_input).w_full()),
          )
          .child(
            v_flex()
              .gap_1()
              .when(self.template_options_count > 1, |this| {
                this.child(
                  v_flex()
                    .gap_1()
                    .child(div().text_sm().child("Template"))
                    .child(
                      Select::new(&self.template_select)
                        .placeholder("Select a pull request template...")
                        .disabled(self.template_loading || self.submit_loading),
                    ),
                )
              })
              .child(
                h_flex()
                  .justify_between()
                  .items_center()
                  .child(div().text_sm().child("Description"))
                  .when(self.template_loading, |this| {
                    this.child(Spinner::new().xsmall())
                  }),
              )
              .child(
                Input::new(&self.body_input)
                  .w_full()
                  .disabled(self.template_loading),
              ),
          )
          .child(
            Checkbox::new("git-create-pull-request-draft")
              .checked(self.draft)
              .label("Create as draft")
              .on_click(cx.listener(|this, checked, window, cx| {
                this.toggle_draft_action(*checked, window, cx);
              })),
          )
          .when(self.validation_error.is_some(), |this| {
            let error = self.validation_error.clone().unwrap_or_default();
            this.child(div().text_xs().text_color(theme.status_red()).child(error))
          }),
      )
      .child(
        DialogFooter::new()
          .px_4()
          .pb_4()
          .pt_1()
          .justify_end()
          .child(
            Button::new("cancel-create-pull-request")
              .label("Cancel")
              .outline()
              .disabled(self.submit_loading)
              .on_click(|_, window, cx| {
                window.close_dialog(cx);
              }),
          )
          .child(
            Button::new("submit-create-pull-request")
              .label("Create pull request")
              .icon(UiIconName::GitPullRequestArrow)
              .primary()
              .loading(self.submit_loading)
              .disabled(self.submit_loading)
              .on_click(cx.listener(Self::submit_action)),
          ),
      )
  }
}

fn open_create_pull_request_dialog(
  api: ApiClient,
  window_handle: AnyWindowHandle,
  git_page: WeakEntity<GitPage>,
  branch_context: GithubBranchContext,
  window: &mut Window,
  cx: &mut App,
) {
  let dialog = cx.new(|cx| {
    CreatePullRequestDialog::new(
      api.clone(),
      window_handle,
      git_page,
      branch_context,
      window,
      cx,
    )
  });
  let dialog_for_overlay = dialog.clone();
  let dialog_for_focus = dialog.clone();

  window.open_dialog(cx, move |overlay, _, _| {
    overlay.p_0().w(px(600.0)).child(dialog_for_overlay.clone())
  });

  window.on_next_frame(move |window, cx| {
    let focus_handle = dialog_for_focus.read(cx).focus_handle(cx);
    window.focus(&focus_handle, cx);
  });
}

struct GitCommandPaletteContents {
  commands: Vec<CommandPaletteCommand>,
  branches: Vec<CommandPaletteBranch>,
  rebase_branches: Vec<CommandPaletteBranch>,
  delete_branches: Vec<CommandPaletteBranch>,
  stashes: Vec<CommandPaletteStash>,
  default_stash_message: Option<SharedString>,
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

fn format_git_file_name_label(path: &Path) -> SharedString {
  path
    .file_name()
    .and_then(|name| name.to_str())
    .unwrap_or("Untitled")
    .replace(['\n', '\r'], "")
    .into()
}

fn format_git_path_label_parts(path: &Path) -> (SharedString, SharedString) {
  let label = path.to_string_lossy().replace(['\n', '\r'], "");
  let file_name = path
    .file_name()
    .and_then(|name| name.to_str())
    .unwrap_or(label.as_str())
    .replace(['\n', '\r'], "");
  let prefix = label
    .strip_suffix(file_name.as_str())
    .unwrap_or("")
    .to_string();
  (prefix.into(), file_name.into())
}

fn render_git_path_label(
  theme: &gpui_component::Theme,
  path: &Path,
  muted_file: bool,
  line_through: bool,
) -> AnyElement {
  let (prefix_label, file_label) = format_git_path_label_parts(path);

  h_flex()
    .min_w_0()
    .overflow_hidden()
    .gap_0()
    .when(line_through, |this| this.line_through())
    .child(
      div()
        .whitespace_nowrap()
        .overflow_hidden()
        .text_ellipsis_start()
        .text_color(theme.muted_foreground)
        .child(prefix_label),
    )
    .child(
      div()
        .flex_shrink_0()
        .when(muted_file, |this| this.text_color(theme.muted_foreground))
        .child(file_label),
    )
    .into_any_element()
}

fn render_git_status_path_label(
  theme: &gpui_component::Theme,
  status: RepoStatusKind,
  path: &Path,
  old_path: Option<&Path>,
) -> AnyElement {
  if status == RepoStatusKind::Renamed
    && let Some(old_path) = old_path
  {
    return h_flex()
      .min_w_0()
      .flex_1()
      .items_center()
      .gap_1()
      .child(render_git_path_label(theme, old_path, true, true))
      .child(
        Icon::new(IconName::ArrowRight)
          .size_3()
          .text_color(theme.muted_foreground),
      )
      .child(render_git_path_label(theme, path, false, false))
      .into_any_element();
  }

  render_git_path_label(theme, path, false, status == RepoStatusKind::Deleted)
}

fn render_repo_status_label(
  theme: &gpui_component::Theme,
  status: Option<RepoStatusKind>,
  label: SharedString,
  old_label: Option<SharedString>,
) -> AnyElement {
  if status == Some(RepoStatusKind::Renamed)
    && let Some(old_label) = old_label
  {
    return h_flex()
      .min_w_0()
      .flex_1()
      .items_center()
      .gap_1()
      .child(
        div()
          .min_w_0()
          .overflow_hidden()
          .text_ellipsis_start()
          .text_color(theme.muted_foreground)
          .line_through()
          .child(old_label),
      )
      .child(
        Icon::new(IconName::ArrowRight)
          .size_3()
          .text_color(theme.muted_foreground),
      )
      .child(
        div()
          .min_w_0()
          .flex_1()
          .overflow_hidden()
          .text_ellipsis_start()
          .child(label),
      )
      .into_any_element();
  }

  div()
    .min_w_0()
    .flex_1()
    .overflow_hidden()
    .text_ellipsis_start()
    .when(status == Some(RepoStatusKind::Deleted), |this| {
      this.line_through()
    })
    .child(label)
    .into_any_element()
}

#[derive(Clone, Debug)]
struct GitFileRow {
  entry: RepoStatusEntry,
}

impl GitFileRow {
  fn new(entry: RepoStatusEntry) -> Self {
    Self { entry }
  }
}

struct GitFileSection {
  label: SharedString,
  is_staged: bool,
  rows: Vec<Rc<GitFileRow>>,
}

struct GitFileListDelegate {
  rows: Vec<Rc<GitFileRow>>,
  sections: Vec<GitFileSection>,
  split_sections: bool,
  selected_index: Option<IndexPath>,
  opened_path: Option<PathBuf>,
  git_page: WeakEntity<GitPage>,
}

impl GitFileListDelegate {
  fn new(git_page: WeakEntity<GitPage>) -> Self {
    Self {
      rows: Vec::new(),
      sections: Vec::new(),
      split_sections: false,
      selected_index: None,
      opened_path: None,
      git_page,
    }
  }

  fn set_rows(&mut self, entries: Vec<RepoStatusEntry>, split_sections: bool) {
    self.rows = entries
      .into_iter()
      .map(|entry| Rc::new(GitFileRow::new(entry)))
      .collect();
    self.split_sections = split_sections;
    self.rebuild_sections();
  }

  fn rebuild_sections(&mut self) {
    if !self.split_sections {
      self.sections = vec![GitFileSection {
        label: "".into(),
        is_staged: false,
        rows: self.rows.clone(),
      }];
      return;
    }

    let mut staged_rows = Vec::new();
    let mut unstaged_rows = Vec::new();
    for row in &self.rows {
      match row.entry.stage {
        RepoStage::Staged => staged_rows.push(row.clone()),
        RepoStage::Unstaged => unstaged_rows.push(row.clone()),
        RepoStage::PartiallyStaged => {
          staged_rows.push(row.clone());
          unstaged_rows.push(row.clone());
        }
      }
    }

    let mut sections = Vec::new();
    if !staged_rows.is_empty() {
      sections.push(GitFileSection {
        label: format!("Staged Changes ({})", staged_rows.len()).into(),
        is_staged: true,
        rows: staged_rows,
      });
    }
    if !unstaged_rows.is_empty() {
      sections.push(GitFileSection {
        label: format!("Changes ({})", unstaged_rows.len()).into(),
        is_staged: false,
        rows: unstaged_rows,
      });
    }
    self.sections = sections;
  }

  fn row_at(&self, ix: IndexPath) -> Option<Rc<GitFileRow>> {
    self
      .sections
      .get(ix.section)
      .and_then(|s| s.rows.get(ix.row).cloned())
  }

  fn find_index_for_path(&self, path: &Path) -> Option<IndexPath> {
    for (section_ix, section) in self.sections.iter().enumerate() {
      for (row_ix, row) in section.rows.iter().enumerate() {
        if row.entry.path == path {
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

  fn set_opened_path(&mut self, path: Option<PathBuf>) {
    self.opened_path = path;
  }
}

fn file_list_base_item(
  ix: IndexPath,
  selected_index: Option<IndexPath>,
  theme: &gpui_component::Theme,
) -> ListItem {
  selectable_list_item(
    ix,
    selected_index
      .map(|selected| selected.eq_row(ix))
      .unwrap_or(false),
    SelectableRowStyle::Inset,
    theme,
  )
}

impl ListDelegate for GitFileListDelegate {
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
        .text_sm()
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
    let mut base_item = file_list_base_item(ix, self.selected_index, &theme);
    let row = self.row_at(ix)?;
    let is_opened = self
      .opened_path
      .as_ref()
      .map(|path| path == &row.entry.path)
      .unwrap_or(false);

    if is_opened {
      base_item = base_item.bg(theme.sidebar_accent.opacity(0.35));
    }

    let status_kind = row.entry.status;
    let status_letter = status_kind.short_code();
    let status_color = GitPage::status_color(status_kind, &theme);
    let status_tooltip = GitPage::status_tooltip(status_kind);
    let (stage_icon, stage_color, stage_tooltip) = GitPage::stage_style(row.entry.stage, &theme);
    let file_icon = file_icon_path_for_path_with_theme(&row.entry.path, &theme)
      .map(|path| {
        img(path)
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

    let status_element = div()
      .id(format!("git-status-letter-{}", ix.row))
      .w(px(15.))
      .min_w(px(15.))
      .text_xs()
      .text_color(status_color)
      .tooltip(move |window, cx| Tooltip::new(status_tooltip.clone()).build(window, cx))
      .child(status_letter);

    let file_label = render_git_status_path_label(
      &theme,
      row.entry.status,
      &row.entry.path,
      row.entry.old_path.as_deref(),
    );

    let rel_path = row.entry.path.clone();
    let old_path = row.entry.old_path.clone();
    let stage = row.entry.stage;
    let git_page = self.git_page.clone();

    // In split mode, the action is determined by the section the file is in,
    // not by the file's stage status (PartiallyStaged appears in both sections).
    let is_staged_section = self.split_sections
      && self
        .sections
        .get(ix.section)
        .map(|s| s.is_staged)
        .unwrap_or(false);
    let toggle_stage_action =
      GitPage::sidebar_toggle_stage_action(stage, self.split_sections, is_staged_section);
    let (toggle_stage_icon, toggle_stage_tooltip) = match toggle_stage_action {
      FileStageButtonAction::Stage => (IconName::Plus, "Stage file"),
      FileStageButtonAction::Unstage => (IconName::Minus, "Unstage file"),
    };
    let can_restore = GitPage::can_restore_file_stage(stage);

    Some(
      base_item.px_2().py_1().child(
        h_flex()
          .group("file-row")
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
              .child(file_label),
          )
          .child(
            div()
              .absolute()
              .right_0()
              .opacity(0.0)
              .group_hover("file-row", |this| this.opacity(1.0))
              .bg(theme.sidebar)
              .rounded(theme.radius)
              .child(
                ButtonGroup::new(format!("file-actions-{}", ix.row))
                  .outline()
                  .child(
                    Button::new(format!("stage-{}", ix.row))
                      .icon(toggle_stage_icon)
                      .xsmall()
                      .tab_stop(false)
                      .tooltip(toggle_stage_tooltip)
                      .on_click({
                        let rel_path = rel_path.clone();
                        let git_page = git_page.clone();
                        move |_event, window, cx| {
                          let _ = git_page.update(cx, |page, cx| match toggle_stage_action {
                            FileStageButtonAction::Unstage => {
                              page.unstage_file_action(rel_path.clone(), cx);
                            }
                            FileStageButtonAction::Stage => {
                              page.stage_file_click_action(
                                window,
                                rel_path.clone(),
                                status_kind,
                                cx,
                              );
                            }
                          });
                        }
                      }),
                  )
                  .when(can_restore, |this| {
                    this.child(
                      Button::new(format!("restore-{}", ix.row))
                        .icon(IconName::Undo)
                        .xsmall()
                        .tab_stop(false)
                        .tooltip("Discard changes")
                        .on_click({
                          let rel_path = rel_path.clone();
                          let old_path = old_path.clone();
                          let git_page = git_page.clone();
                          move |_event, window, cx| {
                            let _ = git_page.update(cx, |page, cx| {
                              page.restore_file_click_action(
                                window,
                                rel_path.clone(),
                                old_path.clone(),
                                status_kind,
                                cx,
                              );
                            });
                          }
                        }),
                    )
                  }),
              ),
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
      .flex()
      .flex_col()
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GitSidebarMode {
  Changes,
  History,
}

#[derive(Clone, Debug)]
struct HistoryCommitFileRow {
  path: PathBuf,
  kind: CommitFileChangeKind,
  label: SharedString,
}

impl HistoryCommitFileRow {
  fn from_commit_file(file: CommitChangedFile) -> Self {
    let path_label = file.path.to_string_lossy().replace(['\n', '\r'], "");
    let label = file
      .old_path
      .as_ref()
      .map(|old_path| {
        let old_label = old_path.to_string_lossy().replace(['\n', '\r'], "");
        format!("{old_label} -> {path_label}")
      })
      .unwrap_or(path_label);
    Self {
      path: file.path,
      kind: file.kind,
      label: label.into(),
    }
  }
}

#[derive(Clone, Debug)]
struct HistoryRenderRow {
  commit: HistoryCommitNode,
}

impl HistoryRenderRow {
  fn from_commit(commit: HistoryCommitNode) -> Self {
    Self { commit }
  }
}

#[derive(Clone, Debug)]
enum HistoryTreeNode {
  Commit {
    oid: String,
  },
  File {
    commit_oid: String,
    file: HistoryCommitFileRow,
  },
  LoadHint {
    oid: String,
  },
  Placeholder,
}

#[derive(Clone)]
struct RecentRepoItem {
  path: PathBuf,
  name: SharedString,
  prefix: SharedString,
  is_selected: bool,
}

impl RecentRepoItem {
  fn new(repo: &RecentRepository, selected_repo: Option<&Path>) -> Self {
    let label = repo.path.to_string_lossy().replace(['\n', '\r'], "");
    let name = repo
      .path
      .file_name()
      .and_then(|name| name.to_str())
      .unwrap_or(label.as_str())
      .replace(['\n', '\r'], "");
    let prefix = label.strip_suffix(name.as_str()).unwrap_or("").to_string();
    Self {
      path: repo.path.clone(),
      name: name.into(),
      prefix: prefix.into(),
      is_selected: selected_repo.is_some_and(|selected| selected == repo.path.as_path()),
    }
  }
}

impl DropdownSelectItem for RecentRepoItem {
  type Value = PathBuf;

  fn value(&self) -> &Self::Value {
    &self.path
  }

  fn selected(&self) -> bool {
    self.is_selected
  }

  fn matches(&self, query: &str) -> bool {
    let query = query.trim();
    if query.is_empty() {
      return true;
    }

    let lowered_query = query.to_lowercase();
    self.name.to_lowercase().contains(&lowered_query)
      || self.prefix.to_lowercase().contains(&lowered_query)
  }

  fn render_item(&self, _window: &mut Window, cx: &mut App) -> AnyElement {
    h_flex()
      .min_w_0()
      .overflow_hidden()
      .items_center()
      .max_w(px(TRIGGER_DROPDOWN_SELECT_WIDTH - 40.0))
      .gap_0()
      .text_sm()
      .child(
        div()
          .text_ellipsis_start()
          .overflow_hidden()
          .text_color(cx.theme().muted_foreground)
          .child(self.prefix.clone()),
      )
      .child(div().flex_shrink().child(self.name.clone()))
      .into_any_element()
  }

  fn render_selected(&self, _window: &mut Window, cx: &mut App) -> AnyElement {
    h_flex()
      .min_w_0()
      .items_center()
      .text_sm()
      .gap_0()
      .child(
        div()
          .overflow_hidden()
          .text_ellipsis_start()
          .text_color(cx.theme().muted_foreground)
          .child(self.prefix.clone()),
      )
      .child(
        div()
          .flex_shrink()
          .text_color(cx.theme().foreground)
          .child(self.name.clone()),
      )
      .into_any_element()
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

  fn detached(label: SharedString, is_current: bool) -> Self {
    Self {
      branch: GitPage::detached_branch_select_value(),
      label,
      is_current,
    }
  }
}

impl DropdownSelectItem for BranchSelectItem {
  type Value = BranchRef;

  fn value(&self) -> &Self::Value {
    &self.branch
  }

  fn selected(&self) -> bool {
    self.is_current
  }

  fn matches(&self, query: &str) -> bool {
    let query = query.trim();
    if query.is_empty() {
      return true;
    }

    self.label.to_lowercase().contains(&query.to_lowercase())
  }

  fn render_item(&self, _window: &mut Window, _cx: &mut App) -> AnyElement {
    div()
      .min_w_0()
      .max_w(px(TRIGGER_DROPDOWN_SELECT_WIDTH - 40.0))
      .flex_1()
      .overflow_hidden()
      .text_sm()
      .text_ellipsis()
      .child(self.label.clone())
      .into_any_element()
  }

  fn render_selected(&self, _window: &mut Window, cx: &mut App) -> AnyElement {
    div()
      .min_w_0()
      .flex_1()
      .overflow_hidden()
      .text_sm()
      .text_color(cx.theme().foreground)
      .text_ellipsis()
      .child(self.label.clone())
      .into_any_element()
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

  pub fn refresh_me(cx: &mut App) {
    let Some(weak) = cx.global::<Self>().git_page.clone() else {
      return;
    };
    let _ = weak.update(cx, |this, cx| this.refresh_auth_state(cx));
  }

  pub fn handle_subscription_callback(cx: &mut App) {
    let Some(weak) = cx.global::<Self>().git_page.clone() else {
      return;
    };
    let _ = weak.update(cx, |this, cx| this.handle_subscription_callback(cx));
  }
}

pub struct GitPage {
  focus_handle: FocusHandle,
  history_tree_wrapper_focus: FocusHandle,
  api: ApiClient,
  repo_dropdown_items: Vec<RecentRepoItem>,
  branch_dropdown_items: Vec<BranchSelectItem>,
  file_list: Entity<ListState<GitFileListDelegate>>,
  history_tree: Entity<TreeState>,
  window_handle: AnyWindowHandle,
  selected_repo: Option<PathBuf>,
  status_entries: Vec<RepoStatusEntry>,
  branch_status: Option<BranchStatus>,
  has_head_commit: bool,
  can_undo_last_commit: bool,
  can_push: bool,
  can_force_push: bool,
  has_unpublished_branch_commits: bool,
  unpublished_branch_check_key: Option<UnpublishedBranchCheckKey>,
  unpublished_branch_checked_at: Option<Instant>,
  force_push_after_rebase: bool,
  push_pull_in_progress: bool,
  publish_branch_and_create_pr_in_progress: bool,
  fetch_in_progress: bool,
  has_staged_changes: bool,
  merge_in_progress: bool,
  rebase_in_progress: bool,
  sidebar_mode: GitSidebarMode,
  history_commits: Vec<HistoryCommitNode>,
  history_revision: Option<HistoryRevision>,
  history_loading: bool,
  history_expanded_commit_oids: HashSet<String>,
  history_commit_files: HashMap<String, Vec<HistoryCommitFileRow>>,
  history_commit_files_loading: HashSet<String>,
  pending_history_file_loads: HashSet<String>,
  history_opened_commit_file: Option<(String, PathBuf)>,
  history_rows_cache: Vec<HistoryRenderRow>,
  history_tree_nodes: HashMap<String, HistoryTreeNode>,
  selected_file: Option<PathBuf>,
  selected_file_source: Option<SelectedFileSource>,
  selected_file_index_hint: Option<IndexPath>,
  select_first_file_after_restore: bool,
  force_list_selection: bool,
  editor: Option<Entity<Editor>>,
  terminal_view: Entity<TerminalView>,
  agent_chat_view: Option<Entity<AgentChatPanel>>,
  interactive_rebase_todo_view: Option<Entity<InteractiveRebaseTodoView>>,
  diff_view: DiffViewMode,
  hide_whitespace: bool,
  git_unified_file_view: bool,
  show_markdown_preview: bool,
  show_terminal_sidebar: bool,
  show_agent_sidebar: bool,
  agent_review_comments: Vec<LocalAgentReviewComment>,
  next_agent_review_comment_id: u64,
  binary_preview: Option<GitBinaryPreview>,
  svg_preview: Option<Result<Arc<RenderImage>, SharedString>>,
  svg_preview_source: Option<SharedString>,
  svg_preview_task: Option<Task<()>>,
  branch_pr_lookup_context: Option<GithubBranchContext>,
  branch_pr_lookup_result: Option<GithubPullRequest>,
  branch_pr_lookup_loading: bool,
  pending_open_action: Option<GitPageOpenAction>,
  pending_conflict_reveal_path: Option<PathBuf>,
  auth_state: AuthState,
  auth_task: Option<Task<()>>,
  branch_pr_lookup_task: Option<Task<()>>,
  open_file_task: Option<Task<()>>,
  status_task: Option<Task<()>>,
  status_refresh_in_progress: bool,
  history_task: Option<Task<()>>,
  history_files_task: Option<Task<()>>,
  history_open_file_task: Option<Task<()>>,
  branch_task: Option<Task<()>>,
  branch_refresh_in_progress: bool,
  branch_pr_lookup_generation: u64,
  open_file_generation: u64,
  status_refresh_generation: u64,
  branch_refresh_generation: u64,
  poll_task: Option<Task<()>>,
  poll_window_active: bool,
  commit_input: Entity<InputState>,
  operation_error: Option<SharedString>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct SelectedFileUpdate {
  clear_selection: bool,
  sync_diff_view: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectedFileSource {
  StatusEntry,
  ProjectFile,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct UnpublishedBranchCheckKey {
  repo_root: PathBuf,
  branch_name: String,
  ahead: usize,
  behind: usize,
  has_upstream: bool,
  head_sha: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum LocalAgentReviewCommentState {
  Draft,
  Copied,
  Addressed,
  Outdated,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LocalAgentReviewComment {
  id: u64,
  in_reply_to_id: Option<u64>,
  path: PathBuf,
  line: usize,
  side: ReviewCommentSide,
  start_line: Option<usize>,
  start_side: Option<ReviewCommentSide>,
  body: Arc<str>,
  original_start_line: Option<usize>,
  original_lines: Vec<String>,
  state: LocalAgentReviewCommentState,
}

fn agent_review_line_label(comment: &LocalAgentReviewComment) -> String {
  let line = comment.line.saturating_add(1);
  let Some(start_line) = comment.start_line.map(|line| line.saturating_add(1)) else {
    return format!("L{line}");
  };
  if start_line == line {
    format!("L{line}")
  } else {
    let start = start_line.min(line);
    let end = start_line.max(line);
    format!("L{start}-L{end}")
  }
}

fn agent_review_side_label(side: ReviewCommentSide) -> &'static str {
  match side {
    ReviewCommentSide::Left => "old",
    ReviewCommentSide::Right => "new",
  }
}

fn agent_review_comment_is_copyable(comment: &LocalAgentReviewComment) -> bool {
  matches!(
    comment.state,
    LocalAgentReviewCommentState::Draft | LocalAgentReviewCommentState::Copied
  )
}

fn lines_match_at(lines: &[String], start_line: usize, expected: &[String]) -> bool {
  if expected.is_empty() || start_line == 0 {
    return false;
  }
  let start_ix = start_line - 1;
  lines
    .get(start_ix..start_ix.saturating_add(expected.len()))
    .is_some_and(|lines| lines == expected)
}

fn contains_line_sequence(lines: &[String], expected: &[String]) -> bool {
  if expected.is_empty() || expected.len() > lines.len() {
    return false;
  }
  lines
    .windows(expected.len())
    .any(|window| window == expected)
}

fn extract_first_suggestion_lines(body: &str) -> Vec<String> {
  let mut in_suggestion = false;
  let mut lines = Vec::new();

  for line in body.lines() {
    let trimmed = line.trim();
    if !in_suggestion {
      if trimmed.starts_with("```suggestion") {
        in_suggestion = true;
      }
      continue;
    }

    if trimmed == "```" {
      return lines;
    }

    lines.push(line.to_string());
  }

  Vec::new()
}

fn next_agent_review_comment_state(
  comment: &LocalAgentReviewComment,
  current_file_lines: &[String],
) -> LocalAgentReviewCommentState {
  if matches!(comment.state, LocalAgentReviewCommentState::Draft) {
    return LocalAgentReviewCommentState::Draft;
  }

  let suggested_lines = extract_first_suggestion_lines(comment.body.as_ref());
  if contains_line_sequence(current_file_lines, &suggested_lines) {
    return LocalAgentReviewCommentState::Addressed;
  }

  if let Some(original_start_line) = comment.original_start_line
    && !lines_match_at(
      current_file_lines,
      original_start_line,
      &comment.original_lines,
    )
  {
    return LocalAgentReviewCommentState::Outdated;
  }

  LocalAgentReviewCommentState::Copied
}

fn format_agent_review_export(comments: &[LocalAgentReviewComment]) -> String {
  let mut comments = comments
    .iter()
    .filter(|comment| agent_review_comment_is_copyable(comment))
    .collect::<Vec<_>>();
  comments.sort_by(|a, b| {
    a.path
      .cmp(&b.path)
      .then_with(|| a.line.cmp(&b.line))
      .then_with(|| a.id.cmp(&b.id))
  });

  let mut output = String::from("Fix these review comments:\n");
  let mut current_path: Option<&Path> = None;

  for comment in comments {
    if current_path != Some(comment.path.as_path()) {
      current_path = Some(comment.path.as_path());
      if !output.is_empty() {
        output.push('\n');
      }
      output.push_str("## ");
      output.push_str(&comment.path.to_string_lossy().replace(['\n', '\r'], ""));
      output.push('\n');
    }

    output.push_str("\n### ");
    output.push_str(&agent_review_line_label(comment));
    output.push_str(" (");
    output.push_str(agent_review_side_label(comment.side));
    output.push_str(" side)");
    if comment.in_reply_to_id.is_some() {
      output.push_str(" reply");
    }
    output.push_str("\n\n");
    output.push_str(comment.body.trim());
    output.push('\n');
  }

  output
}

impl GitPage {
  fn sidebar_mode_tag(mode: GitSidebarMode) -> &'static str {
    match mode {
      GitSidebarMode::Changes => "changes",
      GitSidebarMode::History => "history",
    }
  }

  fn diff_view_tag(diff_view: DiffViewMode) -> &'static str {
    match diff_view {
      DiffViewMode::Inline => "inline",
      DiffViewMode::Split => "split",
    }
  }

  fn active_diff_view_tag(&self) -> &'static str {
    if self.show_markdown_preview
      && self
        .selected_file
        .as_ref()
        .is_some_and(|path| is_previewable_path(path))
    {
      "markdown_preview"
    } else {
      Self::diff_view_tag(self.diff_view)
    }
  }

  fn github_branch_context_from_active_repo(
    local_repo: &ActiveLocalRepo,
  ) -> Option<GithubBranchContext> {
    let owner = local_repo.github_owner.as_deref()?.trim();
    let repo = local_repo.github_repo.as_deref()?.trim();
    let branch = local_repo.current_branch.as_deref()?.trim();

    if owner.is_empty() || repo.is_empty() || branch.is_empty() || branch == "HEAD" {
      return None;
    }

    Some(GithubBranchContext {
      owner: owner.to_string(),
      repo: repo.to_string(),
      branch: branch.to_string(),
    })
  }

  fn github_branch_context(&self, cx: &App) -> Option<GithubBranchContext> {
    ActiveLocalRepoStore::get(cx)
      .as_ref()
      .and_then(Self::github_branch_context_from_active_repo)
  }

  fn branch_has_github_upstream(branch_status: Option<&BranchStatus>) -> bool {
    matches!(
      branch_status,
      Some(status) if status.has_upstream && !Self::is_detached_head(Some(status))
    )
  }

  fn branch_pr_lookup_context(&self, cx: &App) -> Option<GithubBranchContext> {
    (AuthStateStore::has_github_access(cx)
      && Self::branch_has_github_upstream(self.branch_status.as_ref()))
    .then(|| self.github_branch_context(cx))
    .flatten()
  }

  fn branch_pr_button_state(
    branch_context: Option<&GithubBranchContext>,
    can_open_in_app: bool,
    has_github_upstream: bool,
    can_publish_branch: bool,
    lookup_loading: bool,
    lookup_result: Option<&GithubPullRequest>,
  ) -> GitBranchPullRequestButtonState {
    let Some(_branch_context) = branch_context else {
      return GitBranchPullRequestButtonState::Hidden;
    };

    if !can_open_in_app {
      return GitBranchPullRequestButtonState::Hidden;
    }

    if !has_github_upstream {
      return if can_publish_branch {
        GitBranchPullRequestButtonState::PublishAndCreate
      } else {
        GitBranchPullRequestButtonState::Hidden
      };
    }

    if lookup_loading {
      return GitBranchPullRequestButtonState::Checking;
    }

    if let Some(pull_request) = lookup_result {
      return GitBranchPullRequestButtonState::OpenExisting {
        owner: pull_request.repository.owner.clone(),
        repo: pull_request.repository.repo.clone(),
        number: pull_request.number,
      };
    }

    GitBranchPullRequestButtonState::Create
  }

  fn create_pull_request_branch_context(&self, cx: &App) -> Option<GithubBranchContext> {
    let branch_context = self.github_branch_context(cx)?;

    matches!(
      Self::branch_pr_button_state(
        Some(&branch_context),
        AuthStateStore::has_github_access(cx),
        Self::branch_has_github_upstream(self.branch_status.as_ref()),
        Self::should_publish_branch_and_create_pull_request(
          self.branch_status.as_ref(),
          self.has_unpublished_branch_commits,
        ),
        self.branch_pr_lookup_loading,
        self.branch_pr_lookup_result.as_ref(),
      ),
      GitBranchPullRequestButtonState::Create
    )
    .then_some(branch_context)
  }

  fn current_branch_pr_button_state(&self, cx: &App) -> GitBranchPullRequestButtonState {
    let branch_context = self.github_branch_context(cx);
    Self::branch_pr_button_state(
      branch_context.as_ref(),
      AuthStateStore::has_github_access(cx),
      Self::branch_has_github_upstream(self.branch_status.as_ref()),
      Self::should_publish_branch_and_create_pull_request(
        self.branch_status.as_ref(),
        self.has_unpublished_branch_commits,
      ),
      self.branch_pr_lookup_loading,
      self.branch_pr_lookup_result.as_ref(),
    )
  }

  fn branch_pull_request_palette_command(&self, cx: &App) -> Option<CommandPaletteCommand> {
    match self.current_branch_pr_button_state(cx) {
      GitBranchPullRequestButtonState::Create => Some(CommandPaletteCommand::create_pull_request()),
      GitBranchPullRequestButtonState::OpenExisting { number, .. } => {
        Some(CommandPaletteCommand::open_pull_request(number))
      }
      GitBranchPullRequestButtonState::Checking => Some(
        CommandPaletteCommand::create_pull_request().disabled("Checking for an open pull request"),
      ),
      GitBranchPullRequestButtonState::Hidden
      | GitBranchPullRequestButtonState::PublishAndCreate => None,
    }
  }

  fn should_apply_created_pull_request(
    active_context: Option<&GithubBranchContext>,
    created_context: &GithubBranchContext,
  ) -> bool {
    active_context == Some(created_context)
  }

  fn apply_created_pull_request(
    &mut self,
    created_context: &GithubBranchContext,
    pull_request: &GithubPullRequest,
    cx: &mut Context<Self>,
  ) {
    if !Self::should_apply_created_pull_request(
      self.branch_pr_lookup_context(cx).as_ref(),
      created_context,
    ) {
      return;
    }

    self.branch_pr_lookup_generation = self.branch_pr_lookup_generation.wrapping_add(1);
    self.branch_pr_lookup_task = None;
    self.branch_pr_lookup_context = Some(created_context.clone());
    self.branch_pr_lookup_result = Some(pull_request.clone());
    self.branch_pr_lookup_loading = false;
    cx.notify();
  }

  fn publish_branch_and_create_pull_request_action(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    let Some(branch_context) = self.github_branch_context(cx) else {
      return;
    };
    if !AuthStateStore::has_github_access(cx)
      || !Self::should_publish_branch_and_create_pull_request(
        self.branch_status.as_ref(),
        self.has_unpublished_branch_commits,
      )
      || self.push_pull_in_progress
      || self.publish_branch_and_create_pr_in_progress
    {
      return;
    }

    let api = self.api.clone();
    let window_handle = self.window_handle;
    self.add_git_breadcrumb("Publish branch and create PR started", Map::new());
    self.push_pull_in_progress = true;
    self.publish_branch_and_create_pr_in_progress = true;

    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || push(&repo_root, false)).await;
      let _ = this.update(cx, |this, cx| {
        this.push_pull_in_progress = false;
        this.publish_branch_and_create_pr_in_progress = false;

        match result {
          Ok(()) => {
            this.force_push_after_rebase = false;
            this.add_git_breadcrumb("Publish branch and create PR succeeded", Map::new());
            this.reload_status(cx);
            let git_page = cx.entity().downgrade();
            let _ = cx.update_window(window_handle, |_, window, cx| {
              open_create_pull_request_dialog(
                api.clone(),
                window_handle,
                git_page,
                branch_context.clone(),
                window,
                cx,
              );
            });
          }
          Err(error) => {
            let error_message = error.to_string();
            let mut data = Map::new();
            data.insert("error".into(), error_message.clone().into());
            this.add_git_breadcrumb("Publish branch and create PR failed", data.clone());
            this.record_git_unexpected_error(
              "git.publish_and_create_pr",
              error_message.as_str(),
              data,
            );
            this.push_git_action_error_notification(
              "Publish branch failed",
              error_message.into(),
              cx,
            );
            this.reload_status(cx);
          }
        }
      });
    });

    self.status_task = Some(task);
  }

  fn sentry_git_data(&self) -> Map<String, Value> {
    let mut data = Map::new();
    if let Some(repo_root) = self.selected_repo.as_deref() {
      let (repo_name, repo_hash) = sentry_context::sanitize_repo_path(repo_root);
      data.insert("repo_name".into(), repo_name.into());
      data.insert("repo_hash".into(), repo_hash.into());
    }
    if let Some(selected_file) = self.selected_file.as_deref() {
      let file = selected_file.to_string_lossy().replace(['\n', '\r'], "");
      data.insert("selected_file".into(), file.into());
    }
    if let Some(branch) = self
      .branch_status
      .as_ref()
      .map(|status| status.name.clone())
    {
      data.insert("branch".into(), branch.into());
    }
    data.insert(
      "sidebar_mode".into(),
      Self::sidebar_mode_tag(self.sidebar_mode).into(),
    );
    data.insert("diff_view".into(), self.active_diff_view_tag().into());
    data
  }

  fn add_git_breadcrumb(&self, message: &str, mut data: Map<String, Value>) {
    let base = self.sentry_git_data();
    for (key, value) in base {
      data.entry(key).or_insert(value);
    }
    sentry_context::add_breadcrumb("git.action", message, data);
  }

  fn record_git_unexpected_error(
    &self,
    op: &'static str,
    error: &str,
    mut data: Map<String, Value>,
  ) {
    let base = self.sentry_git_data();
    for (key, value) in base {
      data.entry(key).or_insert(value);
    }
    let io_error = std::io::Error::other(error.to_string());
    sentry_context::capture_unexpected_error(op, &io_error, data);
  }

  fn record_git_expected_error(&self, operation: &str, reason: &str, mut data: Map<String, Value>) {
    let base = self.sentry_git_data();
    for (key, value) in base {
      data.entry(key).or_insert(value);
    }
    sentry_context::record_expected_error(operation, reason, data);
  }

  fn sync_sentry_git_context(&self) {
    sentry_context::sync_git_context(
      self.selected_repo.as_deref(),
      self.selected_file.as_deref(),
      self
        .branch_status
        .as_ref()
        .map(|status| status.name.as_str()),
      Self::sidebar_mode_tag(self.sidebar_mode),
      self.active_diff_view_tag(),
    );
  }

  fn sync_active_local_repo(&self, cx: &mut Context<Self>) {
    let snapshot = self.selected_repo.as_ref().map(|repo_root| {
      let github_remote = current_github_remote_repo(repo_root).ok().flatten();
      ActiveLocalRepo {
        repo_root: repo_root.clone(),
        github_owner: github_remote.as_ref().map(|remote| remote.owner.clone()),
        github_repo: github_remote.as_ref().map(|remote| remote.repo.clone()),
        current_branch: self
          .branch_status
          .as_ref()
          .map(|status| status.name.clone()),
        head_sha: current_head_sha(repo_root).ok().flatten(),
        has_uncommitted_changes: !self.status_entries.is_empty(),
      }
    });
    ActiveLocalRepoStore::set(cx, snapshot);
  }

  fn clear_branch_pr_lookup(&mut self) {
    self.branch_pr_lookup_task = None;
    self.branch_pr_lookup_context = None;
    self.branch_pr_lookup_result = None;
    self.branch_pr_lookup_loading = false;
    self.branch_pr_lookup_generation = self.branch_pr_lookup_generation.wrapping_add(1);
  }

  fn refresh_branch_pr_lookup_if_needed(&mut self, cx: &mut Context<Self>) {
    let next_context = self.branch_pr_lookup_context(cx);
    if self.branch_pr_lookup_context.as_ref() == next_context.as_ref() {
      return;
    }

    self.refresh_branch_pr_lookup(cx);
  }

  fn refresh_branch_pr_lookup(&mut self, cx: &mut Context<Self>) {
    let next_context = self.branch_pr_lookup_context(cx);
    self.branch_pr_lookup_generation = self.branch_pr_lookup_generation.wrapping_add(1);
    let generation = self.branch_pr_lookup_generation;
    self.branch_pr_lookup_task = None;
    self.branch_pr_lookup_context = next_context.clone();
    self.branch_pr_lookup_result = None;
    self.branch_pr_lookup_loading = false;

    let Some(context) = next_context else {
      cx.notify();
      return;
    };

    self.branch_pr_lookup_loading = true;
    let api = self.api.clone();
    let task = cx.spawn(async move |this, cx| {
      let context_for_fetch = context.clone();
      let result = unblock(move || {
        api.fetch_pull_request_for_branch(
          context_for_fetch.owner.as_str(),
          context_for_fetch.repo.as_str(),
          context_for_fetch.branch.as_str(),
        )
      })
      .await;

      let _ = this.update(cx, |this, cx| {
        if this.branch_pr_lookup_generation != generation
          || this.branch_pr_lookup_context.as_ref() != Some(&context)
        {
          return;
        }

        this.branch_pr_lookup_task = None;
        this.branch_pr_lookup_loading = false;
        this.branch_pr_lookup_result = result.ok().flatten();
        cx.notify();
      });
    });

    self.branch_pr_lookup_task = Some(task);
    cx.notify();
  }

  fn should_refresh_file_list(sidebar_mode: GitSidebarMode) -> bool {
    sidebar_mode == GitSidebarMode::Changes
  }

  fn should_refresh_history_for_poll(
    include_history: bool,
    history_empty: bool,
    cached_revision: Option<&HistoryRevision>,
    polled_revision: Option<&HistoryRevision>,
  ) -> bool {
    if !include_history {
      return false;
    }
    if history_empty {
      return true;
    }

    match polled_revision {
      Some(polled_revision) => Some(polled_revision) != cached_revision,
      None => false,
    }
  }

  fn branch_name_changed(previous: Option<&BranchStatus>, next: Option<&BranchStatus>) -> bool {
    previous.map(|status| status.name.as_str()) != next.map(|status| status.name.as_str())
  }

  fn has_staged_changes(entries: &[RepoStatusEntry]) -> bool {
    entries
      .iter()
      .any(|entry| matches!(entry.stage, RepoStage::Staged | RepoStage::PartiallyStaged))
  }

  fn selected_file_update(
    selected_file: Option<&Path>,
    selected_file_source: Option<SelectedFileSource>,
    status_entries: &[RepoStatusEntry],
    has_history_file_selection: bool,
    sync_diff_when_selected_retained: bool,
  ) -> SelectedFileUpdate {
    if has_history_file_selection {
      return SelectedFileUpdate::default();
    }

    let Some(selected_file) = selected_file else {
      return SelectedFileUpdate::default();
    };

    let is_selected_file_present = status_entries
      .iter()
      .any(|entry| entry.path.as_path() == selected_file);
    if !is_selected_file_present {
      if selected_file_source == Some(SelectedFileSource::ProjectFile) {
        return SelectedFileUpdate {
          clear_selection: false,
          sync_diff_view: sync_diff_when_selected_retained,
        };
      }

      return SelectedFileUpdate {
        clear_selection: true,
        sync_diff_view: false,
      };
    }

    SelectedFileUpdate {
      clear_selection: false,
      sync_diff_view: sync_diff_when_selected_retained,
    }
  }

  fn selected_branch_from_status(current: Option<&BranchStatus>) -> Option<BranchRef> {
    current.map(|status| {
      if Self::is_detached_head(Some(status)) {
        Self::detached_branch_select_value()
      } else {
        BranchRef {
          name: status.name.clone(),
          kind: BranchKind::Local,
        }
      }
    })
  }

  fn is_detached_head(branch_status: Option<&BranchStatus>) -> bool {
    branch_status.is_some_and(|status| status.name == "HEAD")
  }

  fn detached_branch_select_value() -> BranchRef {
    BranchRef {
      name: DETACHED_BRANCH_SELECT_SENTINEL.to_string(),
      kind: BranchKind::Local,
    }
  }

  fn is_detached_branch_select_value(branch: &BranchRef) -> bool {
    branch.kind == BranchKind::Local && branch.name == DETACHED_BRANCH_SELECT_SENTINEL
  }

  fn branch_select_items(
    branches: Vec<BranchRef>,
    selected: Option<&BranchRef>,
    detached_label: Option<&str>,
  ) -> Vec<BranchSelectItem> {
    let mut items = branches
      .into_iter()
      .map(|branch| {
        let is_current = selected == Some(&branch);
        BranchSelectItem::new(branch, is_current)
      })
      .collect::<Vec<_>>();

    if selected.is_some_and(Self::is_detached_branch_select_value) {
      let label = detached_label
        .map(|label| format!("HEAD ({label})"))
        .unwrap_or_else(|| "HEAD (detached)".to_string());
      items.insert(0, BranchSelectItem::detached(label.into(), true));
    }

    items
  }

  fn should_apply_branch_refresh(
    selected_repo: Option<&Path>,
    requested_repo: &Path,
    current_generation: u64,
    refresh_generation: u64,
  ) -> bool {
    selected_repo == Some(requested_repo) && current_generation == refresh_generation
  }

  fn should_apply_status_refresh(
    selected_repo: Option<&Path>,
    requested_repo: &Path,
    current_generation: u64,
    refresh_generation: u64,
  ) -> bool {
    selected_repo == Some(requested_repo) && current_generation == refresh_generation
  }

  fn advance_status_refresh_generation(&mut self) -> u64 {
    self.status_refresh_generation = self.status_refresh_generation.wrapping_add(1);
    self.status_refresh_generation
  }

  fn current_status_refresh_generation(&self) -> u64 {
    self.status_refresh_generation
  }

  fn status_poll_interval(window_active: bool) -> Duration {
    if window_active {
      Duration::from_millis(STATUS_POLL_INTERVAL_MS)
    } else {
      INACTIVE_STATUS_POLL_INTERVAL
    }
  }

  fn should_poll_status(
    window_active: bool,
    selected_repo: Option<&Path>,
    status_refresh_in_progress: bool,
  ) -> bool {
    window_active && selected_repo.is_some() && !status_refresh_in_progress
  }

  fn unpublished_branch_check_key(
    repo_root: &Path,
    branch_status: &BranchStatus,
    head_sha: Option<String>,
  ) -> UnpublishedBranchCheckKey {
    UnpublishedBranchCheckKey {
      repo_root: repo_root.to_path_buf(),
      branch_name: branch_status.name.clone(),
      ahead: branch_status.ahead,
      behind: branch_status.behind,
      has_upstream: branch_status.has_upstream,
      head_sha,
    }
  }

  fn should_recheck_unpublished_branch(
    next_key: &UnpublishedBranchCheckKey,
    cached_key: Option<&UnpublishedBranchCheckKey>,
    force_recheck: bool,
  ) -> bool {
    force_recheck || cached_key != Some(next_key)
  }

  fn resolve_polled_unpublished_branch_commits(
    repo_root: &Path,
    branch_status: Option<&BranchStatus>,
    cached_key: Option<&UnpublishedBranchCheckKey>,
    cached_value: bool,
    force_recheck: bool,
  ) -> Option<(bool, Option<UnpublishedBranchCheckKey>, bool)> {
    let Some(branch_status) = branch_status else {
      return Some((false, None, true));
    };

    if Self::is_detached_head(Some(branch_status)) {
      return Some((false, None, true));
    }

    if branch_status.has_upstream {
      let next_key = Self::unpublished_branch_check_key(repo_root, branch_status, None);
      return Some((branch_status.ahead > 0, Some(next_key), true));
    }

    let head_sha = current_head_sha(repo_root).ok().flatten();
    let next_key = Self::unpublished_branch_check_key(repo_root, branch_status, head_sha);
    if !Self::should_recheck_unpublished_branch(&next_key, cached_key, force_recheck) {
      return Some((cached_value, Some(next_key), false));
    }

    let has_unpublished_branch_commits = branch_has_unpublished_commits(repo_root).ok()?;
    Some((has_unpublished_branch_commits, Some(next_key), true))
  }

  fn should_refresh_editor_for_path(selected_file: Option<&Path>, rel_path: &Path) -> bool {
    selected_file == Some(rel_path)
  }

  fn restore_uses_delete(status: RepoStatusKind) -> bool {
    status == RepoStatusKind::Untracked
  }

  fn selected_file_can_stage(stage: RepoStage) -> bool {
    stage == RepoStage::Unstaged
  }

  fn selected_file_can_unstage(stage: RepoStage) -> bool {
    matches!(stage, RepoStage::Staged | RepoStage::PartiallyStaged)
  }

  fn can_restore_file_stage(stage: RepoStage) -> bool {
    matches!(stage, RepoStage::Unstaged | RepoStage::PartiallyStaged)
  }

  fn sidebar_toggle_stage_action(
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
    } else if Self::selected_file_can_unstage(stage) {
      FileStageButtonAction::Unstage
    } else {
      FileStageButtonAction::Stage
    }
  }

  fn stage_requires_confirmation(status: RepoStatusKind) -> bool {
    status == RepoStatusKind::Conflicted
  }

  fn should_confirm_stage_for_status(
    status: Option<RepoStatusKind>,
    has_unresolved_conflict_markers: bool,
  ) -> bool {
    status.is_some_and(Self::stage_requires_confirmation) && has_unresolved_conflict_markers
  }

  fn first_conflicted_path(repo_root: &Path) -> Option<PathBuf> {
    list_repo_status(repo_root)
      .ok()?
      .into_iter()
      .find(|entry| entry.status == RepoStatusKind::Conflicted)
      .map(|entry| entry.path)
  }

  fn open_editor_has_unresolved_conflict_markers(&self, cx: &App) -> bool {
    self.editor.as_ref().is_none_or(|editor| {
      editor.read_with(cx, |editor, cx| editor.has_unresolved_conflict_markers(cx))
    })
  }

  fn all_entries_staged(entries: &[RepoStatusEntry]) -> bool {
    !entries.is_empty() && entries.iter().all(|entry| entry.stage == RepoStage::Staged)
  }

  #[allow(clippy::too_many_arguments)]
  fn apply_status_snapshot(
    &mut self,
    entries: Vec<RepoStatusEntry>,
    branch_status: Option<BranchStatus>,
    head_status: Option<HeadCommitStatus>,
    has_unpublished_branch_commits: bool,
    merge_in_progress: bool,
    rebase_in_progress: bool,
    rebase_commit_message: Option<String>,
    sync_diff_when_selected_retained: bool,
    cx: &mut Context<Self>,
  ) -> bool {
    let was_rebase_in_progress = self.rebase_in_progress;
    self.status_entries = entries;
    let branch_changed =
      Self::branch_name_changed(self.branch_status.as_ref(), branch_status.as_ref());
    self.branch_status = branch_status;
    if branch_changed
      || (self.force_push_after_rebase
        && self
          .branch_status
          .as_ref()
          .is_some_and(|status| status.ahead == 0))
    {
      self.force_push_after_rebase = false;
    }
    self.merge_in_progress = merge_in_progress;
    self.rebase_in_progress = rebase_in_progress;
    if !rebase_in_progress {
      self.operation_error = None;
    }
    self.sync_rebase_commit_input(
      was_rebase_in_progress,
      rebase_in_progress,
      rebase_commit_message,
      cx,
    );
    self.has_staged_changes = Self::has_staged_changes(&self.status_entries);
    let head_status = head_status.unwrap_or(HeadCommitStatus {
      has_head_commit: false,
      can_undo_last_commit: false,
    });
    self.has_head_commit = head_status.has_head_commit;
    self.can_undo_last_commit = head_status.can_undo_last_commit;
    self.has_unpublished_branch_commits = has_unpublished_branch_commits;
    let (can_push, can_force_push) = Self::push_flags(
      self.branch_status.as_ref(),
      self.has_head_commit,
      self.force_push_after_rebase,
    );
    self.can_push = can_push;
    self.can_force_push = can_force_push;
    self.sync_active_local_repo(cx);
    self.refresh_branch_pr_lookup_if_needed(cx);

    let selected_file_update = Self::selected_file_update(
      self.selected_file.as_deref(),
      self.selected_file_source,
      &self.status_entries,
      self.history_opened_commit_file.is_some(),
      sync_diff_when_selected_retained,
    );
    if selected_file_update.clear_selection {
      self.invalidate_open_file_task();
      self.selected_file = None;
      self.selected_file_source = None;
      self.editor = None;
      self.binary_preview = None;
      self.ensure_page_shortcut_focus(cx);
    } else if selected_file_update.sync_diff_view {
      self.sync_diff_view(cx);
    }

    self.sync_editor_unmerged_state(cx);

    if self.select_first_file_after_restore {
      self.select_first_file_after_restore = false;
      if let Some(first_path) = self.status_entries.first().map(|entry| entry.path.clone()) {
        self.open_status_file(first_path, cx);
      }
    }

    self.sync_sentry_git_context();

    branch_changed
  }

  fn history_file_status_kind(&self, commit_oid: &str, rel_path: &Path) -> Option<RepoStatusKind> {
    self
      .history_commit_files
      .get(commit_oid)
      .and_then(|files| files.iter().find(|file| file.path == rel_path))
      .map(|file| Self::history_change_kind_to_repo_status(file.kind))
  }

  fn split_disabled_for_path(&self, rel_path: &Path) -> bool {
    if self.selected_file.as_deref() == Some(rel_path) && self.binary_preview.is_some() {
      return true;
    }

    if let Some((commit_oid, selected_path)) = self.history_opened_commit_file.as_ref()
      && selected_path == rel_path
      && let Some(status) = self.history_file_status_kind(commit_oid, rel_path)
    {
      return matches!(
        status,
        RepoStatusKind::Untracked | RepoStatusKind::Added | RepoStatusKind::Deleted
      );
    }

    self.status_entries.iter().any(|entry| {
      entry.path == rel_path
        && matches!(
          entry.status,
          RepoStatusKind::Untracked | RepoStatusKind::Added | RepoStatusKind::Deleted
        )
    })
  }

  fn selected_file_is_markdown(&self) -> bool {
    self
      .selected_file
      .as_ref()
      .map(|path| is_markdown_path(path))
      .unwrap_or(false)
  }

  fn selected_file_is_svg(&self) -> bool {
    self
      .selected_file
      .as_ref()
      .map(|path| is_svg_path(path))
      .unwrap_or(false)
  }

  fn build_binary_preview(path: &Path, binary_bytes: Option<Vec<u8>>) -> Option<GitBinaryPreview> {
    if let Some(bytes) = binary_bytes {
      if let Some(image) = raster_image_from_bytes(path, bytes.clone()) {
        return Some(GitBinaryPreview::RasterImage(image));
      }
      if should_show_unsupported_binary_placeholder(path, Some(bytes.as_slice())) {
        return Some(GitBinaryPreview::UnsupportedBinary);
      }
      return None;
    }

    if matches!(
      file_preview_kind(path),
      Some(FilePreviewKind::UnsupportedBinary)
    ) {
      Some(GitBinaryPreview::UnsupportedBinary)
    } else {
      None
    }
  }

  fn effective_diff_view_for_path(&self, path: &Path) -> DiffViewMode {
    if self.selected_file.as_deref() == Some(path) && self.binary_preview.is_some() {
      return DiffViewMode::Inline;
    }

    if self.show_markdown_preview && is_previewable_path(path) {
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
    let hide_ws = self.hide_whitespace;
    editor.update(cx, |editor, cx| {
      editor.set_diff_view_mode(diff_view, cx);
      editor.set_ignore_whitespace(hide_ws, cx);
    });
  }

  fn render_binary_preview_content(
    &self,
    preview: &GitBinaryPreview,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let theme = cx.theme().clone();

    match preview {
      GitBinaryPreview::RasterImage(image) => {
        let loading_color = theme.muted_foreground;
        let error_color = theme.status_red();
        let image_el = img(image.clone())
          .max_w_full()
          .max_h_full()
          .object_fit(ObjectFit::Contain)
          .with_loading(move || {
            render_image_preview_status_message("Rendering image preview...", loading_color)
          })
          .with_fallback(move || {
            render_image_preview_status_message("Unable to render image preview", error_color)
          });

        div()
          .flex_1()
          .min_h_0()
          .min_w(px(0.0))
          .overflow_hidden()
          .bg(theme.background)
          .occlude()
          .debug_selector(|| GIT_BINARY_PREVIEW_RENDER_DEBUG_SELECTOR.to_string())
          .child(
            div().relative().size_full().child(
              div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .bottom_0()
                .p_4()
                .flex()
                .items_center()
                .justify_center()
                .child(image_el),
            ),
          )
          .into_any_element()
      }
      GitBinaryPreview::UnsupportedBinary => div()
        .flex_1()
        .min_h_0()
        .min_w(px(0.0))
        .bg(theme.background)
        .occlude()
        .debug_selector(|| GIT_BINARY_PREVIEW_RENDER_DEBUG_SELECTOR.to_string())
        .child(
          v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .child(
              Icon::new(IconName::File)
                .size_6()
                .text_color(theme.muted_foreground),
            )
            .child(
              div()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child("Binary file preview is not available."),
            ),
        )
        .into_any_element(),
    }
  }

  fn selected_file_index(&self, cx: &Context<Self>) -> Option<IndexPath> {
    let selected = self.selected_file.as_ref()?;
    let delegate = self.file_list.read(cx).delegate();
    if let Some(hint) = self.selected_file_index_hint
      && let Some(row) = delegate.row_at(hint)
      && row.entry.path == *selected
    {
      return Some(hint);
    }
    delegate.find_index_for_path(selected)
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
    self.git_unified_file_view = crate::config::AppSettings::get(cx).git_unified_file_view;
    let rows = self.status_entries.clone();
    let split_sections = !self.git_unified_file_view;
    let opened_path = self.selected_file.clone();
    self.file_list.update(cx, |state, cx| {
      state.delegate_mut().set_rows(rows.clone(), split_sections);
      state.delegate_mut().set_opened_path(opened_path);
      cx.notify();
    });

    let selected_index = if self.force_list_selection {
      self.force_list_selection = false;
      self.selected_file_index(cx)
    } else {
      // Try to preserve the selected file by path rather than raw index
      self.selected_file_index(cx)
    };
    self.set_file_list_selected_index(selected_index, cx);
  }

  fn refresh_history_list(&mut self, cx: &mut Context<Self>) {
    self.history_rows_cache = Self::build_history_rows(&self.history_commits);
    self.sync_history_tree_state(cx);
  }

  fn sync_history_tree_state(&mut self, cx: &mut Context<Self>) {
    let selected_id = self
      .history_tree
      .read(cx)
      .selected_entry()
      .map(|entry| entry.item().id.to_string());
    let (items, nodes) = Self::build_history_tree_items(
      &self.history_rows_cache,
      &self.history_commit_files,
      &self.history_commit_files_loading,
      &self.history_expanded_commit_oids,
    );
    self.history_tree_nodes = nodes;
    self.history_tree.update(cx, |state, cx| {
      state.set_items(items, cx);
      if let Some(selected_id) = selected_id.as_ref() {
        let selected_item = TreeItem::new(selected_id.clone(), selected_id.clone());
        state.set_selected_item(Some(&selected_item), cx);
      }
    });
    cx.notify();
  }

  fn build_history_tree_items(
    rows: &[HistoryRenderRow],
    files_by_commit: &HashMap<String, Vec<HistoryCommitFileRow>>,
    loading_commits: &HashSet<String>,
    expanded_commits: &HashSet<String>,
  ) -> (Vec<TreeItem>, HashMap<String, HistoryTreeNode>) {
    let mut items = Vec::with_capacity(rows.len());
    let mut nodes = HashMap::new();

    for row in rows {
      let commit_id = format!("history-commit:{}", row.commit.oid);
      nodes.insert(
        commit_id.clone(),
        HistoryTreeNode::Commit {
          oid: row.commit.oid.clone(),
        },
      );
      let mut children = Vec::new();
      if loading_commits.contains(row.commit.oid.as_str()) {
        let loading_id = format!("history-loading:{}", row.commit.oid);
        nodes.insert(loading_id.clone(), HistoryTreeNode::Placeholder);
        children.push(TreeItem::new(loading_id, "Loading files..."));
      } else if let Some(files) = files_by_commit.get(row.commit.oid.as_str()) {
        if files.is_empty() {
          let empty_id = format!("history-empty:{}", row.commit.oid);
          nodes.insert(empty_id.clone(), HistoryTreeNode::Placeholder);
          children.push(TreeItem::new(empty_id, "No files changed"));
        } else {
          for (file_index, file) in files.iter().enumerate() {
            let file_id = format!("history-file:{}:{}", row.commit.oid, file_index);
            nodes.insert(
              file_id.clone(),
              HistoryTreeNode::File {
                commit_oid: row.commit.oid.clone(),
                file: file.clone(),
              },
            );
            children.push(TreeItem::new(file_id, file.label.clone()));
          }
        }
      } else {
        let hint_id = format!("history-hint:{}", row.commit.oid);
        nodes.insert(
          hint_id.clone(),
          HistoryTreeNode::LoadHint {
            oid: row.commit.oid.clone(),
          },
        );
        children.push(TreeItem::new(hint_id, "Load files..."));
      }

      let is_expanded = expanded_commits.contains(row.commit.oid.as_str());
      items.push(
        TreeItem::new(commit_id, row.commit.summary.clone())
          .children(children)
          .expanded(is_expanded),
      );
    }

    (items, nodes)
  }

  fn sync_history_cache_with_commits(&mut self) {
    let known_oids = self
      .history_commits
      .iter()
      .map(|commit| commit.oid.clone())
      .collect::<HashSet<_>>();
    self
      .history_commit_files
      .retain(|oid, _| known_oids.contains(oid));
    self
      .history_commit_files_loading
      .retain(|oid| known_oids.contains(oid));
    self
      .pending_history_file_loads
      .retain(|oid| known_oids.contains(oid));
    self
      .history_expanded_commit_oids
      .retain(|oid| known_oids.contains(oid));
    if let Some((commit_oid, _)) = self.history_opened_commit_file.as_ref()
      && !known_oids.contains(commit_oid)
    {
      self.history_opened_commit_file = None;
    }
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

  fn handle_subscription_callback(&mut self, cx: &mut Context<Self>) {
    self.refresh_auth_state(cx);

    NavigationHistory::navigate("/billing", cx);
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
        if let Ok(Some((_username, secret))) = read_result
          && let Ok(token) = String::from_utf8(secret)
        {
          this.api.set_bearer_token(token);
          this.refresh_auth_state(cx);
          return;
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
          Ok(Some(user)) => AuthState::Authenticated(Box::new(user)),
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
        cx.update(|cx| cx.open_url(&url));
      }
    });

    self.auth_task = Some(task);
  }

  fn set_auth_state(&mut self, state: AuthState, cx: &mut Context<Self>) {
    let had_github_access = AuthStateStore::has_github_access(cx);
    self.auth_state = state.clone();
    AuthStateStore::set(cx, state);

    if !had_github_access && AuthStateStore::has_github_access(cx) {
      self.fetch_initial_notifications(cx);
    }

    self.refresh_branch_pr_lookup(cx);
    cx.refresh_windows();
    cx.notify();
  }

  fn fetch_initial_notifications(&mut self, cx: &mut Context<Self>) {
    let api = self.api.clone();
    cx.spawn(async move |_, cx| {
      let result = unblock(move || api.fetch_github_notifications()).await;
      let _ = cx.update(|cx| {
        if let Ok(notifications) = result {
          let unread = notifications.iter().filter(|n| n.unread).count();
          NotificationCountStore::set(cx, unread);
          set_dock_badge(unread);
          cx.refresh_windows();
        }
      });
    })
    .detach();
  }

  pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
    let recent = ConfigStore::load_recent_repositories();
    let app_settings = AppSettings::get(cx);
    let selected_repo = recent.first().map(|repo| repo.path.clone());
    let repo_dropdown_items: Vec<RecentRepoItem> = recent
      .iter()
      .map(|repo| RecentRepoItem::new(repo, selected_repo.as_deref()))
      .collect();
    let git_page_weak = cx.entity().downgrade();
    let file_list =
      cx.new(|cx| ListState::new(GitFileListDelegate::new(git_page_weak), window, cx));
    let _ = file_list.read(cx).focus_handle(cx).tab_stop(true);
    let history_tree = cx.new(|cx| TreeState::new(cx));

    let commit_input = cx.new(|cx| {
      InputState::new(window, cx)
        .auto_grow(1, 5)
        .placeholder("Commit message...")
    });

    let terminal_working_directory = selected_repo.clone();

    let mut view = Self {
      focus_handle: cx.focus_handle(),
      history_tree_wrapper_focus: cx.focus_handle().tab_stop(true),
      api: WorkspaceApi::global(cx).api.clone(),
      repo_dropdown_items,
      branch_dropdown_items: Vec::new(),
      file_list,
      history_tree,
      window_handle: window.window_handle(),
      selected_repo,
      status_entries: Vec::new(),
      branch_status: None,
      has_head_commit: false,
      can_undo_last_commit: false,
      can_push: false,
      can_force_push: false,
      has_unpublished_branch_commits: false,
      unpublished_branch_check_key: None,
      unpublished_branch_checked_at: None,
      force_push_after_rebase: false,
      push_pull_in_progress: false,
      publish_branch_and_create_pr_in_progress: false,
      fetch_in_progress: false,
      has_staged_changes: false,
      merge_in_progress: false,
      rebase_in_progress: false,
      sidebar_mode: GitSidebarMode::Changes,
      history_commits: Vec::new(),
      history_revision: None,
      history_loading: false,
      history_expanded_commit_oids: HashSet::new(),
      history_commit_files: HashMap::new(),
      history_commit_files_loading: HashSet::new(),
      pending_history_file_loads: HashSet::new(),
      history_opened_commit_file: None,
      history_rows_cache: Vec::new(),
      history_tree_nodes: HashMap::new(),
      selected_file: None,
      selected_file_source: None,
      selected_file_index_hint: None,
      select_first_file_after_restore: false,
      force_list_selection: false,
      editor: None,
      terminal_view: cx.new(|cx| TerminalView::new(terminal_working_directory.clone(), cx)),
      agent_chat_view: None,
      interactive_rebase_todo_view: None,
      diff_view: if app_settings.split_diff_view {
        DiffViewMode::Split
      } else {
        DiffViewMode::Inline
      },
      hide_whitespace: app_settings.hide_whitespace,
      git_unified_file_view: app_settings.git_unified_file_view,
      show_markdown_preview: false,
      show_terminal_sidebar: false,
      show_agent_sidebar: false,
      agent_review_comments: Vec::new(),
      next_agent_review_comment_id: 1,
      binary_preview: None,
      svg_preview: None,
      svg_preview_source: None,
      svg_preview_task: None,
      branch_pr_lookup_context: None,
      branch_pr_lookup_result: None,
      branch_pr_lookup_loading: false,
      pending_open_action: None,
      pending_conflict_reveal_path: None,
      auth_state: AuthState::Unknown,
      auth_task: None,
      branch_pr_lookup_task: None,
      open_file_task: None,
      status_task: None,
      status_refresh_in_progress: false,
      history_task: None,
      history_files_task: None,
      history_open_file_task: None,
      branch_task: None,
      branch_refresh_in_progress: false,
      branch_pr_lookup_generation: 0,
      open_file_generation: 0,
      status_refresh_generation: 0,
      branch_refresh_generation: 0,
      poll_task: None,
      poll_window_active: true,
      commit_input,
      operation_error: None,
    };

    view.subscribe_to_file_list(cx);
    view.subscribe_to_commit_input(window, cx);
    view.subscribe_to_history_tree_focus(window, cx);
    view.subscribe_to_window_activation(window, cx);
    view.reload_status(cx);
    view.refresh_branches(cx);
    view.start_polling(cx);
    view.load_bearer_from_keychain(cx);
    AuthCallbackTarget::register_git_page(cx);
    GitPageHandle::register(cx);

    view
  }

  #[cfg(test)]
  fn new_for_test(window: &mut Window, cx: &mut Context<Self>) -> Self {
    let git_page_weak = cx.entity().downgrade();
    let file_list =
      cx.new(|cx| ListState::new(GitFileListDelegate::new(git_page_weak), window, cx));
    let _ = file_list.read(cx).focus_handle(cx).tab_stop(true);
    let history_tree = cx.new(|cx| TreeState::new(cx));
    let commit_input = cx.new(|cx| {
      InputState::new(window, cx)
        .auto_grow(1, 5)
        .placeholder("Commit message...")
    });

    let mut view = Self {
      focus_handle: cx.focus_handle(),
      history_tree_wrapper_focus: cx.focus_handle().tab_stop(true),
      api: ApiClient::new(),
      repo_dropdown_items: Vec::new(),
      branch_dropdown_items: Vec::new(),
      file_list,
      history_tree,
      window_handle: window.window_handle(),
      selected_repo: None,
      status_entries: Vec::new(),
      branch_status: None,
      has_head_commit: false,
      can_undo_last_commit: false,
      can_push: false,
      can_force_push: false,
      has_unpublished_branch_commits: false,
      unpublished_branch_check_key: None,
      unpublished_branch_checked_at: None,
      force_push_after_rebase: false,
      push_pull_in_progress: false,
      publish_branch_and_create_pr_in_progress: false,
      fetch_in_progress: false,
      has_staged_changes: false,
      merge_in_progress: false,
      rebase_in_progress: false,
      sidebar_mode: GitSidebarMode::Changes,
      history_commits: Vec::new(),
      history_revision: None,
      history_loading: false,
      history_expanded_commit_oids: HashSet::new(),
      history_commit_files: HashMap::new(),
      history_commit_files_loading: HashSet::new(),
      pending_history_file_loads: HashSet::new(),
      history_opened_commit_file: None,
      history_rows_cache: Vec::new(),
      history_tree_nodes: HashMap::new(),
      selected_file: None,
      selected_file_source: None,
      selected_file_index_hint: None,
      select_first_file_after_restore: false,
      force_list_selection: false,
      editor: None,
      terminal_view: cx.new(|cx| TerminalView::new(None, cx)),
      agent_chat_view: None,
      interactive_rebase_todo_view: None,
      diff_view: DiffViewMode::Inline,
      hide_whitespace: false,
      git_unified_file_view: false,
      show_markdown_preview: false,
      show_terminal_sidebar: false,
      show_agent_sidebar: false,
      agent_review_comments: Vec::new(),
      next_agent_review_comment_id: 1,
      binary_preview: None,
      svg_preview: None,
      svg_preview_source: None,
      svg_preview_task: None,
      branch_pr_lookup_context: None,
      branch_pr_lookup_result: None,
      branch_pr_lookup_loading: false,
      pending_open_action: None,
      pending_conflict_reveal_path: None,
      auth_state: AuthState::Unknown,
      auth_task: None,
      branch_pr_lookup_task: None,
      open_file_task: None,
      status_task: None,
      status_refresh_in_progress: false,
      history_task: None,
      history_files_task: None,
      history_open_file_task: None,
      branch_task: None,
      branch_refresh_in_progress: false,
      branch_pr_lookup_generation: 0,
      open_file_generation: 0,
      status_refresh_generation: 0,
      branch_refresh_generation: 0,
      poll_task: None,
      poll_window_active: true,
      commit_input,
      operation_error: None,
    };

    view.subscribe_to_file_list(cx);
    view.subscribe_to_commit_input(window, cx);
    view.subscribe_to_history_tree_focus(window, cx);
    view.subscribe_to_window_activation(window, cx);
    GitPageHandle::register(cx);
    view
  }

  fn handle_repo_select_confirm(&mut self, repo_root: PathBuf, cx: &mut Context<Self>) {
    self.set_selected_repo(repo_root, cx);
    self.ensure_page_shortcut_focus(cx);
  }

  fn open_repository_with_action(
    &mut self,
    repo_root: PathBuf,
    action: GitPageOpenAction,
    cx: &mut Context<Self>,
  ) {
    let conflict_resolution = Self::active_conflict_resolution_snapshot(&repo_root);
    self.set_selected_repo(repo_root.clone(), cx);
    self.pending_open_action = Some(action.clone());
    if let Some(conflict_resolution) = conflict_resolution {
      self.merge_in_progress = conflict_resolution.merge_in_progress;
      self.rebase_in_progress = conflict_resolution.rebase_in_progress;
    }

    match action {
      GitPageOpenAction::MergeBaseBranch { base_branch_name } => {
        self.start_merge_base_branch_action(repo_root, base_branch_name, cx);
      }
    }
  }

  fn active_conflict_resolution_snapshot(
    repo_root: &Path,
  ) -> Option<ActiveConflictResolutionSnapshot> {
    let merge_in_progress = is_merge_in_progress(repo_root).unwrap_or(false);
    let rebase_in_progress = is_rebase_in_progress(repo_root).unwrap_or(false);
    let conflicted_path = Self::first_conflicted_path(repo_root);

    (merge_in_progress || rebase_in_progress || conflicted_path.is_some()).then_some(
      ActiveConflictResolutionSnapshot {
        merge_in_progress,
        rebase_in_progress,
        conflicted_path,
      },
    )
  }

  fn open_action_loading_message(action: &GitPageOpenAction) -> &'static str {
    match action {
      GitPageOpenAction::MergeBaseBranch { .. } => "Opening conflict resolution...",
    }
  }

  fn start_merge_base_branch_action(
    &mut self,
    repo_root: PathBuf,
    base_branch_name: String,
    cx: &mut Context<Self>,
  ) {
    if self.fetch_in_progress {
      return;
    }

    self.fetch_in_progress = true;
    let editor = self.editor.clone();
    let task = cx.spawn(async move |this, cx| {
      let repo_root_for_action = repo_root.clone();
      let branch_name_for_fetch = base_branch_name.clone();
      let result = unblock(move || {
        if let Some(conflict_resolution) =
          Self::active_conflict_resolution_snapshot(&repo_root_for_action)
        {
          return Ok::<_, anyhow::Error>(GitPageOpenActionResult::ResumeActiveConflict(
            conflict_resolution,
          ));
        }

        fetch(&repo_root_for_action)?;
        let branch_ref = resolve_branch_ref(&repo_root_for_action, &branch_name_for_fetch)?
          .ok_or_else(|| {
            anyhow::anyhow!(
              "branch {:?} was not found locally or on any remote",
              branch_name_for_fetch
            )
          })?;
        Ok::<_, anyhow::Error>(GitPageOpenActionResult::MergeBaseBranchReady(branch_ref))
      })
      .await;

      let _ = this.update(cx, |this, cx| {
        this.fetch_in_progress = false;
        this.pending_open_action = None;

        match result {
          Ok(GitPageOpenActionResult::ResumeActiveConflict(conflict_resolution)) => {
            this.merge_in_progress = conflict_resolution.merge_in_progress;
            this.rebase_in_progress = conflict_resolution.rebase_in_progress;
            if let Some(path) = conflict_resolution.conflicted_path {
              this.open_file_revealing_first_conflict(path, cx);
            }
          }
          Ok(GitPageOpenActionResult::MergeBaseBranchReady(branch_ref)) => {
            if let Err(error) = this.merge_branch_action(branch_ref, None, true, cx) {
              this.push_git_action_error_notification(
                "Update branch failed",
                error.to_string().into(),
                cx,
              );
            }
          }
          Err(error) => {
            this.push_git_action_error_notification(
              "Update branch failed",
              error.to_string().into(),
              cx,
            );
          }
        }

        this.reload_status(cx);
        this.refresh_branches(cx);
        if let Some(editor) = editor.clone() {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
      });
    });

    self.status_task = Some(task);
  }

  fn refocus_page_shortcuts_after_dropdown_select(
    &self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let focus_handle = self.focus_handle.clone();
    window.focus(&focus_handle, cx);
    cx.on_next_frame(window, move |_, window, cx| {
      window.focus(&focus_handle, cx);
    });
  }

  fn repo_select_handler(&self, cx: &Context<Self>) -> RepoSelectHandler {
    let view = cx.entity();
    Rc::new(move |repo_root, window, cx| {
      view.update(cx, |this, cx| {
        this.handle_repo_select_confirm(repo_root, cx);
        this.refocus_page_shortcuts_after_dropdown_select(window, cx);
      });
    })
  }

  fn handle_branch_select_confirm(&mut self, branch: BranchRef, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };

    self.ensure_page_shortcut_focus(cx);

    if Self::is_detached_branch_select_value(&branch) {
      return;
    }
    self.advance_status_refresh_generation();
    let editor = self.editor.clone();
    let branch_name = branch.name.clone();

    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || switch_branch(&repo_root, &branch)).await;
      let _ = this.update(cx, |this, cx| match result {
        Ok(()) => {
          this.reload_status(cx);
          this.refresh_branches(cx);
          if let Some(editor) = editor.clone() {
            editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
          }
        }
        Err(error) => {
          this.push_branch_switch_error_notification(&branch_name, error.to_string().into(), cx);
        }
      });
    });

    self.branch_task = Some(task);
  }

  fn push_branch_switch_error_notification(
    &self,
    branch_name: &str,
    error: SharedString,
    cx: &mut Context<Self>,
  ) {
    let title = format!("Failed to switch to {branch_name}");
    self.push_git_error_notification_with_id::<GitBranchSwitchNotificationId>(title, error, cx);
  }

  fn push_git_action_error_notification(
    &self,
    title: impl Into<SharedString>,
    error: SharedString,
    cx: &mut Context<Self>,
  ) {
    self.push_git_error_notification_with_id::<GitActionErrorNotificationId>(title, error, cx);
  }

  fn push_git_action_error_notification_in_window(
    &self,
    title: impl Into<SharedString>,
    error: SharedString,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    window.push_notification(
      Notification::error(error)
        .id::<GitActionErrorNotificationId>()
        .title(title),
      cx,
    );
  }

  fn push_git_action_success_notification(&self, message: SharedString, cx: &mut Context<Self>) {
    let _ = cx.update_window(self.window_handle, move |_, window, cx| {
      window.push_notification(Notification::success(message), cx);
    });
  }

  fn push_git_error_notification_with_id<T: Sized + 'static>(
    &self,
    title: impl Into<SharedString>,
    error: SharedString,
    cx: &mut Context<Self>,
  ) {
    let title = title.into();
    let _ = cx.update_window(self.window_handle, move |_, window, cx| {
      window.push_notification(Notification::error(error).id::<T>().title(title), cx);
    });
  }

  fn set_commit_input_value(
    &self,
    value: &str,
    window: Option<&mut Window>,
    cx: &mut Context<Self>,
  ) {
    let value = value.to_string();
    if let Some(window) = window {
      self
        .commit_input
        .update(cx, |input, cx| input.set_value(&value, window, cx));
      return;
    }

    let commit_input = self.commit_input.clone();
    let _ = cx.update_window(self.window_handle, move |_, window, cx| {
      commit_input.update(cx, |input, cx| input.set_value(&value, window, cx));
    });
  }

  fn command_palette_error_notification_title(
    action: &CommandPaletteAction,
  ) -> Option<&'static str> {
    match action {
      CommandPaletteAction::CheckoutDetached { .. } => Some("Checkout failed"),
      CommandPaletteAction::SwitchBranch(_) => Some("Switch branch failed"),
      CommandPaletteAction::CreateBranch { .. } => Some("Create branch failed"),
      CommandPaletteAction::CreateBranchFrom { .. } => Some("Create branch failed"),
      CommandPaletteAction::DeleteBranch(_) => Some("Delete branch failed"),
      CommandPaletteAction::MergeBranch { .. } => Some("Merge failed"),
      CommandPaletteAction::AbortMerge => Some("Abort merge failed"),
      CommandPaletteAction::RebaseBranch { .. } => Some("Rebase failed"),
      CommandPaletteAction::AbortRebase => Some("Abort rebase failed"),
      CommandPaletteAction::Stash { .. } => Some("Stash failed"),
      CommandPaletteAction::ApplyStash(_) => Some("Apply stash failed"),
      CommandPaletteAction::DropStash(_) => Some("Drop stash failed"),
      CommandPaletteAction::PopStash(_) => Some("Pop stash failed"),
      CommandPaletteAction::CherryPick { .. } => Some("Cherry-pick failed"),
      _ => None,
    }
  }

  fn handle_command_palette_operation_error(
    &mut self,
    action: &CommandPaletteAction,
    err: anyhow::Error,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let message: SharedString = err.to_string().into();
    let title = Self::command_palette_error_notification_title(action).unwrap_or("Action failed");
    self.push_git_action_error_notification_in_window(title, message, window, cx);
  }

  fn branch_select_handler(&self, cx: &Context<Self>) -> BranchSelectHandler {
    let view = cx.entity();
    Rc::new(move |branch, window, cx| {
      view.update(cx, |this, cx| {
        this.handle_branch_select_confirm(branch, cx);
        this.refocus_page_shortcuts_after_dropdown_select(window, cx);
      });
    })
  }

  fn subscribe_to_file_list(&mut self, cx: &mut Context<Self>) {
    cx.subscribe(
      &self.file_list,
      move |this, state, event: &ListEvent, cx| match event {
        ListEvent::Select(ix) | ListEvent::Confirm(ix) => {
          let row = state.read(cx).delegate().row_at(*ix);
          if let Some(row) = row {
            this.selected_file_index_hint = Some(*ix);
            this.open_status_file(row.entry.path.clone(), cx);
          }
        }
        ListEvent::Cancel => {}
      },
    )
    .detach();
  }

  fn subscribe_to_history_tree_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    cx.on_focus_in(
      &self.history_tree_wrapper_focus.clone(),
      window,
      |this, window, cx| {
        this.history_tree.update(cx, |state, cx| {
          state.focus(window, cx);
        });
      },
    )
    .detach();
  }

  fn subscribe_to_window_activation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    cx.on_focus_in(&self.focus_handle.clone(), window, |this, _window, cx| {
      this.poll_window_active = true;
      if this.selected_repo.is_some() && !this.status_refresh_in_progress {
        this.reload_status(cx);
      }
    })
    .detach();
  }

  fn subscribe_to_commit_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    cx.subscribe_in(
      &self.commit_input,
      window,
      |this, _state, event: &InputEvent, window, cx| {
        if let InputEvent::PressEnter { secondary: true } = event {
          this.commit_changes_inner(window, cx);
        }
      },
    )
    .detach();
  }

  fn invalidate_open_file_task(&mut self) {
    self.open_file_generation = self.open_file_generation.wrapping_add(1);
    self.open_file_task = None;
  }

  fn set_selected_repo(&mut self, repo_root: PathBuf, cx: &mut Context<Self>) {
    if self.selected_repo.as_ref() == Some(&repo_root) {
      return;
    }

    let previous_repo = self.selected_repo.clone();
    self.selected_repo = Some(repo_root.clone());
    self.invalidate_open_file_task();
    self.selected_file = None;
    self.selected_file_source = None;
    self.select_first_file_after_restore = false;
    self.operation_error = None;
    self.editor = None;
    self.agent_review_comments.clear();
    self.next_agent_review_comment_id = 1;
    self.binary_preview = None;
    self.interactive_rebase_todo_view = None;
    self.merge_in_progress = false;
    self.rebase_in_progress = false;
    self.force_push_after_rebase = false;
    self.push_pull_in_progress = false;
    self.publish_branch_and_create_pr_in_progress = false;
    self.history_commits.clear();
    self.history_revision = None;
    self.history_loading = self.sidebar_mode == GitSidebarMode::History;
    self.history_expanded_commit_oids.clear();
    self.history_commit_files.clear();
    self.history_commit_files_loading.clear();
    self.pending_history_file_loads.clear();
    self.history_opened_commit_file = None;
    ActiveLocalRepoStore::set(cx, None);
    self.clear_branch_pr_lookup();
    ConfigStore::persist_recent_repository(&repo_root);

    self.reload_status(cx);
    self.refresh_branches(cx);
    self.refresh_repo_select(cx);
    self.sync_repo_select_with_path(&repo_root, cx);
    self.sync_sentry_git_context();
    let mut data = Map::new();
    if let Some(previous_repo) = previous_repo.as_deref() {
      let (repo_name, repo_hash) = sentry_context::sanitize_repo_path(previous_repo);
      data.insert("from_repo_name".into(), repo_name.into());
      data.insert("from_repo_hash".into(), repo_hash.into());
    }
    let (repo_name, repo_hash) = sentry_context::sanitize_repo_path(&repo_root);
    data.insert("to_repo_name".into(), repo_name.into());
    data.insert("to_repo_hash".into(), repo_hash.into());
    self.add_git_breadcrumb("Selected repository changed", data);
    cx.notify();
  }

  fn clear_selected_repo(&mut self, cx: &mut Context<Self>) {
    if self.selected_repo.is_none() {
      return;
    }
    self.selected_repo = None;
    self.invalidate_open_file_task();
    self.selected_file = None;
    self.selected_file_source = None;
    self.select_first_file_after_restore = false;
    self.operation_error = None;
    self.editor = None;
    self.agent_review_comments.clear();
    self.next_agent_review_comment_id = 1;
    self.binary_preview = None;
    self.interactive_rebase_todo_view = None;
    self.merge_in_progress = false;
    self.rebase_in_progress = false;
    self.force_push_after_rebase = false;
    self.push_pull_in_progress = false;
    self.publish_branch_and_create_pr_in_progress = false;
    self.status_entries.clear();
    self.branch_status = None;
    self.history_commits.clear();
    self.history_revision = None;
    self.history_loading = false;
    self.history_expanded_commit_oids.clear();
    self.history_commit_files.clear();
    self.history_commit_files_loading.clear();
    self.pending_history_file_loads.clear();
    self.history_opened_commit_file = None;
    ActiveLocalRepoStore::set(cx, None);
    self.clear_branch_pr_lookup();
    self.refresh_repo_select(cx);
    cx.notify();
  }

  fn refresh_repo_select(&mut self, _cx: &mut Context<Self>) {
    let selected_repo = self.selected_repo.clone();
    let mut recent = ConfigStore::load_recent_repositories();
    if let Some(selected_repo_path) = selected_repo.as_ref()
      && !recent.iter().any(|repo| &repo.path == selected_repo_path)
    {
      recent.insert(
        0,
        RecentRepository {
          path: selected_repo_path.clone(),
        },
      );
    }

    let items: Vec<RecentRepoItem> = recent
      .iter()
      .map(|repo| RecentRepoItem::new(repo, selected_repo.as_deref()))
      .collect();
    self.repo_dropdown_items = items;
  }

  fn sync_repo_select_with_path(&mut self, repo_root: &Path, _cx: &mut Context<Self>) {
    let repo_root = repo_root.to_path_buf();
    let mut recent = ConfigStore::load_recent_repositories();
    if !recent.iter().any(|repo| repo.path == repo_root) {
      recent.insert(
        0,
        RecentRepository {
          path: repo_root.clone(),
        },
      );
    }
    let items = recent
      .iter()
      .map(|repo| RecentRepoItem::new(repo, Some(repo_root.as_path())))
      .collect::<Vec<_>>();
    self.repo_dropdown_items = items;
  }

  fn clear_branch_select(&mut self, cx: &mut Context<Self>) {
    self.branch_dropdown_items.clear();
    cx.notify();
  }

  fn refresh_branches(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      self.branch_refresh_in_progress = false;
      self.clear_branch_select(cx);
      return;
    };

    self.branch_refresh_in_progress = true;
    self.branch_refresh_generation = self.branch_refresh_generation.wrapping_add(1);
    let refresh_generation = self.branch_refresh_generation;
    let requested_repo = repo_root.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || {
        let branches = list_branches(&repo_root).ok()?;
        let current = current_branch_status(&repo_root).ok();
        let detached_label = if Self::is_detached_head(current.as_ref()) {
          detached_head_label(&repo_root).ok()
        } else {
          None
        };
        Some((branches, current, detached_label))
      })
      .await;
      let _ = this.update(cx, |this, cx| {
        if !Self::should_apply_branch_refresh(
          this.selected_repo.as_deref(),
          requested_repo.as_path(),
          this.branch_refresh_generation,
          refresh_generation,
        ) {
          return;
        }
        this.branch_refresh_in_progress = false;
        this.branch_task = None;
        if let Some((branches, current, detached_label)) = result {
          let selected = Self::selected_branch_from_status(current.as_ref());
          let items =
            Self::branch_select_items(branches, selected.as_ref(), detached_label.as_deref());
          this.branch_dropdown_items = items;
        }
        cx.notify();
      });
    });

    self.branch_task = Some(task);
  }

  fn refresh_history(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      self.history_commits.clear();
      self.history_revision = None;
      self.history_loading = false;
      self.history_expanded_commit_oids.clear();
      self.history_commit_files.clear();
      self.history_commit_files_loading.clear();
      self.pending_history_file_loads.clear();
      self.history_opened_commit_file = None;
      self.interactive_rebase_todo_view = None;
      self.refresh_history_list(cx);
      cx.notify();
      return;
    };

    if self.history_commits.is_empty() {
      self.history_loading = true;
      cx.notify();
    }

    let task = cx.spawn(async move |this, cx| {
      let requested_repo = repo_root.clone();
      let (history, revision) = unblock(move || {
        (
          list_commit_history(&repo_root, HISTORY_MAX_COMMITS),
          current_history_revision(&repo_root).ok(),
        )
      })
      .await;
      let _ = this.update(cx, |this, cx| {
        if this.selected_repo.as_ref() != Some(&requested_repo) {
          return;
        }
        if let Ok(history) = history {
          this.history_commits = history;
          this.sync_history_cache_with_commits();
          if let Some(revision) = revision {
            this.history_revision = Some(revision);
          }
          this.refresh_history_list(cx);
        }
        this.history_loading = false;
        cx.notify();
      });
    });

    self.history_task = Some(task);
  }

  fn refresh_current_page(&mut self, cx: &mut Context<Self>) {
    self.reload_status(cx);
    self.refresh_branches(cx);
  }

  fn reload_status(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      self.invalidate_open_file_task();
      self.status_entries.clear();
      self.select_first_file_after_restore = false;
      if Self::should_refresh_file_list(self.sidebar_mode) {
        self.refresh_file_list(cx);
      }
      self.branch_status = None;
      self.has_head_commit = false;
      self.can_undo_last_commit = false;
      self.can_push = false;
      self.can_force_push = false;
      self.has_unpublished_branch_commits = false;
      self.unpublished_branch_check_key = None;
      self.unpublished_branch_checked_at = None;
      self.force_push_after_rebase = false;
      self.push_pull_in_progress = false;
      self.publish_branch_and_create_pr_in_progress = false;
      self.has_staged_changes = false;
      self.merge_in_progress = false;
      self.rebase_in_progress = false;
      self.operation_error = None;
      self.history_commits.clear();
      self.history_revision = None;
      self.history_loading = false;
      self.history_expanded_commit_oids.clear();
      self.history_commit_files.clear();
      self.history_commit_files_loading.clear();
      self.pending_history_file_loads.clear();
      self.history_opened_commit_file = None;
      self.refresh_history_list(cx);
      self.sync_sentry_git_context();
      ActiveLocalRepoStore::set(cx, None);
      self.clear_branch_pr_lookup();
      self.status_refresh_in_progress = false;
      cx.notify();
      return;
    };
    self.status_refresh_in_progress = true;
    let include_history = self.sidebar_mode == GitSidebarMode::History;
    if include_history && self.history_commits.is_empty() {
      self.history_loading = true;
      cx.notify();
    }
    let refresh_generation = self.advance_status_refresh_generation();

    let task = cx.spawn(async move |this, cx| {
      let requested_repo = repo_root.clone();
      let status = unblock(move || {
        let entries = list_repo_status(&repo_root).ok()?;
        let branch = current_branch_status(&repo_root).ok();
        let head_status = head_commit_status(&repo_root).ok();
        let unpublished_branch_commits = branch_has_unpublished_commits(&repo_root).ok()?;
        let merge_in_progress = is_merge_in_progress(&repo_root).unwrap_or(false);
        let rebase_in_progress = is_rebase_in_progress(&repo_root).unwrap_or(false);
        let rebase_commit_message = if rebase_in_progress {
          current_rebase_commit_message(&repo_root).ok().flatten()
        } else {
          None
        };
        let history = if include_history {
          list_commit_history(&repo_root, HISTORY_MAX_COMMITS).ok()
        } else {
          None
        };
        let history_revision = if include_history {
          current_history_revision(&repo_root).ok()
        } else {
          None
        };
        Some((
          entries,
          branch,
          head_status,
          unpublished_branch_commits,
          merge_in_progress,
          rebase_in_progress,
          rebase_commit_message,
          history,
          history_revision,
        ))
      })
      .await;
      let Some((
        entries,
        branch_status,
        head_status,
        unpublished_branch_commits,
        merge_in_progress,
        rebase_in_progress,
        rebase_commit_message,
        history,
        history_revision,
      )) = status
      else {
        let _ = this.update(cx, |this, cx| {
          if !Self::should_apply_status_refresh(
            this.selected_repo.as_deref(),
            requested_repo.as_path(),
            this.status_refresh_generation,
            refresh_generation,
          ) {
            return;
          }
          this.status_refresh_in_progress = false;
          this.status_task = None;
          this.invalidate_open_file_task();
          this.status_entries.clear();
          this.select_first_file_after_restore = false;
          this.branch_status = None;
          this.has_head_commit = false;
          this.can_undo_last_commit = false;
          this.can_push = false;
          this.can_force_push = false;
          this.has_unpublished_branch_commits = false;
          this.unpublished_branch_check_key = None;
          this.unpublished_branch_checked_at = None;
          this.force_push_after_rebase = false;
          this.push_pull_in_progress = false;
          this.publish_branch_and_create_pr_in_progress = false;
          this.has_staged_changes = false;
          this.merge_in_progress = false;
          this.rebase_in_progress = false;
          this.operation_error = None;
          this.selected_file = None;
          this.selected_file_source = None;
          this.editor = None;
          this.binary_preview = None;
          this.interactive_rebase_todo_view = None;
          this.history_opened_commit_file = None;
          this.clear_branch_select(cx);
          if include_history {
            this.history_commits.clear();
            this.history_revision = None;
            this.history_loading = false;
            this.history_expanded_commit_oids.clear();
            this.history_commit_files.clear();
            this.history_commit_files_loading.clear();
            this.pending_history_file_loads.clear();
            this.refresh_history_list(cx);
          } else if this.history_loading {
            this.history_loading = false;
          }
          if Self::should_refresh_file_list(this.sidebar_mode) {
            this.refresh_file_list(cx);
          }
          ActiveLocalRepoStore::set(cx, None);
          this.clear_branch_pr_lookup();
          cx.notify();
        });
        return;
      };

      let _ = this.update(cx, |this, cx| {
        if !Self::should_apply_status_refresh(
          this.selected_repo.as_deref(),
          requested_repo.as_path(),
          this.status_refresh_generation,
          refresh_generation,
        ) {
          return;
        }
        this.status_refresh_in_progress = false;
        this.status_task = None;
        let branch_changed = this.apply_status_snapshot(
          entries,
          branch_status,
          head_status,
          unpublished_branch_commits,
          merge_in_progress,
          rebase_in_progress,
          rebase_commit_message,
          true,
          cx,
        );
        if include_history {
          if let Some(history) = history {
            this.history_commits = history;
            this.sync_history_cache_with_commits();
            if let Some(history_revision) = history_revision {
              this.history_revision = Some(history_revision);
            }
            this.refresh_history_list(cx);
          }
          this.history_loading = false;
        }
        if branch_changed {
          this.refresh_branches(cx);
        }
        if Self::should_refresh_file_list(this.sidebar_mode) {
          this.refresh_file_list(cx);
        }
        if this.refresh_agent_review_comment_states_for_selected_file(cx) {
          this.sync_agent_review_comments_to_editor(cx);
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
        let poll_window_active = match this.update(cx, |this, _| this.poll_window_active) {
          Ok(window_active) => window_active,
          Err(_) => return,
        };
        cx.background_executor()
          .timer(Self::status_poll_interval(poll_window_active))
          .await;

        let window_handle = match this.update(cx, |this, _| this.window_handle) {
          Ok(window_handle) => window_handle,
          Err(_) => return,
        };
        let window_active = match window_handle.update(cx, |_, window, _| window.is_window_active())
        {
          Ok(window_active) => window_active,
          Err(_) => return,
        };

        let poll_state = match this.update(cx, |this, _| {
          this.poll_window_active = window_active;
          if !Self::should_poll_status(
            window_active,
            this.selected_repo.as_deref(),
            this.status_refresh_in_progress,
          ) {
            return None;
          }
          let repo_root = this
            .selected_repo
            .clone()
            .expect("selected repo is checked before polling");
          let now = Instant::now();
          let force_unpublished_branch_recheck =
            this.unpublished_branch_checked_at.is_none_or(|checked_at| {
              now.duration_since(checked_at) >= UNPUBLISHED_BRANCH_RECHECK_INTERVAL
            });
          Some((
            repo_root,
            this.sidebar_mode == GitSidebarMode::History,
            this.history_revision.clone(),
            this.history_commits.is_empty(),
            // Polling should not supersede an explicit refresh that is already in flight.
            this.current_status_refresh_generation(),
            this.unpublished_branch_check_key.clone(),
            this.has_unpublished_branch_commits,
            force_unpublished_branch_recheck,
            now,
          ))
        }) {
          Ok(Some(value)) => value,
          Ok(None) => continue,
          Err(_) => return,
        };
        let (
          repo_root,
          include_history,
          cached_history_revision,
          history_empty,
          refresh_generation,
          cached_unpublished_branch_key,
          cached_unpublished_branch_commits,
          force_unpublished_branch_recheck,
          unpublished_branch_checked_at,
        ) = poll_state;
        let requested_repo = repo_root.clone();

        let status = unblock(move || {
          let entries = list_repo_status(&repo_root).ok()?;
          let branch = current_branch_status(&repo_root).ok();
          let head_status = head_commit_status(&repo_root).ok();
          let (
            unpublished_branch_commits,
            unpublished_branch_check_key,
            unpublished_branch_checked,
          ) = Self::resolve_polled_unpublished_branch_commits(
            &repo_root,
            branch.as_ref(),
            cached_unpublished_branch_key.as_ref(),
            cached_unpublished_branch_commits,
            force_unpublished_branch_recheck,
          )?;
          let merge_in_progress = is_merge_in_progress(&repo_root).unwrap_or(false);
          let rebase_in_progress = is_rebase_in_progress(&repo_root).unwrap_or(false);
          let rebase_commit_message = if rebase_in_progress {
            current_rebase_commit_message(&repo_root).ok().flatten()
          } else {
            None
          };
          let polled_history_revision = if include_history {
            current_history_revision(&repo_root).ok()
          } else {
            None
          };
          let should_refresh_history = Self::should_refresh_history_for_poll(
            include_history,
            history_empty,
            cached_history_revision.as_ref(),
            polled_history_revision.as_ref(),
          );
          let history = if should_refresh_history {
            list_commit_history(&repo_root, HISTORY_MAX_COMMITS).ok()
          } else {
            None
          };
          Some((
            entries,
            branch,
            head_status,
            unpublished_branch_commits,
            merge_in_progress,
            rebase_in_progress,
            rebase_commit_message,
            polled_history_revision,
            should_refresh_history,
            history,
            unpublished_branch_check_key,
            unpublished_branch_checked,
          ))
        })
        .await;
        let Some((
          entries,
          branch_status,
          head_status,
          unpublished_branch_commits,
          merge_in_progress,
          rebase_in_progress,
          rebase_commit_message,
          polled_history_revision,
          should_refresh_history,
          history,
          unpublished_branch_check_key,
          unpublished_branch_checked,
        )) = status
        else {
          continue;
        };

        let _ = this.update(cx, |this, cx| {
          if !Self::should_apply_status_refresh(
            this.selected_repo.as_deref(),
            requested_repo.as_path(),
            this.status_refresh_generation,
            refresh_generation,
          ) {
            return;
          }
          let branch_changed = this.apply_status_snapshot(
            entries,
            branch_status,
            head_status,
            unpublished_branch_commits,
            merge_in_progress,
            rebase_in_progress,
            rebase_commit_message,
            false,
            cx,
          );
          if unpublished_branch_checked {
            this.unpublished_branch_checked_at = unpublished_branch_check_key
              .as_ref()
              .map(|_| unpublished_branch_checked_at);
          }
          this.unpublished_branch_check_key = unpublished_branch_check_key;
          if include_history {
            if let Some(history) = history {
              this.history_commits = history;
              this.sync_history_cache_with_commits();
              this.history_loading = false;
              if let Some(history_revision) = polled_history_revision {
                this.history_revision = Some(history_revision);
              }
              this.refresh_history_list(cx);
            } else if !should_refresh_history {
              if let Some(history_revision) = polled_history_revision {
                this.history_revision = Some(history_revision);
              }
            } else if this.history_loading {
              // Preserve last known history on transient failures.
              this.history_loading = false;
            }
          }
          if branch_changed {
            this.refresh_branches(cx);
          }
          if Self::should_refresh_file_list(this.sidebar_mode) {
            this.refresh_file_list(cx);
          }
          cx.notify();
        });
      }
    }));
  }

  #[cfg(test)]
  fn poll_once_for_test(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };

    let include_history = self.sidebar_mode == GitSidebarMode::History;
    let cached_history_revision = self.history_revision.clone();
    let history_empty = self.history_commits.is_empty();
    let requested_repo = repo_root.clone();
    let refresh_generation = self.current_status_refresh_generation();
    let cached_unpublished_branch_key = self.unpublished_branch_check_key.clone();
    let cached_unpublished_branch_commits = self.has_unpublished_branch_commits;
    let unpublished_branch_checked_at = Instant::now();
    let force_unpublished_branch_recheck =
      self.unpublished_branch_checked_at.is_none_or(|checked_at| {
        unpublished_branch_checked_at.duration_since(checked_at)
          >= UNPUBLISHED_BRANCH_RECHECK_INTERVAL
      });
    let task = cx.spawn(async move |this, cx| {
      let status = unblock(move || {
        let entries = list_repo_status(&repo_root).ok()?;
        let branch = current_branch_status(&repo_root).ok();
        let head_status = head_commit_status(&repo_root).ok();
        let (unpublished_branch_commits, unpublished_branch_check_key, unpublished_branch_checked) =
          Self::resolve_polled_unpublished_branch_commits(
            &repo_root,
            branch.as_ref(),
            cached_unpublished_branch_key.as_ref(),
            cached_unpublished_branch_commits,
            force_unpublished_branch_recheck,
          )?;
        let merge_in_progress = is_merge_in_progress(&repo_root).unwrap_or(false);
        let rebase_in_progress = is_rebase_in_progress(&repo_root).unwrap_or(false);
        let rebase_commit_message = if rebase_in_progress {
          current_rebase_commit_message(&repo_root).ok().flatten()
        } else {
          None
        };
        let polled_history_revision = if include_history {
          current_history_revision(&repo_root).ok()
        } else {
          None
        };
        let should_refresh_history = Self::should_refresh_history_for_poll(
          include_history,
          history_empty,
          cached_history_revision.as_ref(),
          polled_history_revision.as_ref(),
        );
        let history = if should_refresh_history {
          list_commit_history(&repo_root, HISTORY_MAX_COMMITS).ok()
        } else {
          None
        };
        Some((
          entries,
          branch,
          head_status,
          unpublished_branch_commits,
          merge_in_progress,
          rebase_in_progress,
          rebase_commit_message,
          polled_history_revision,
          should_refresh_history,
          history,
          unpublished_branch_check_key,
          unpublished_branch_checked,
        ))
      })
      .await;

      let Some((
        entries,
        branch_status,
        head_status,
        unpublished_branch_commits,
        merge_in_progress,
        rebase_in_progress,
        rebase_commit_message,
        polled_history_revision,
        should_refresh_history,
        history,
        unpublished_branch_check_key,
        unpublished_branch_checked,
      )) = status
      else {
        return;
      };

      let _ = this.update(cx, |this, cx| {
        if !Self::should_apply_status_refresh(
          this.selected_repo.as_deref(),
          requested_repo.as_path(),
          this.status_refresh_generation,
          refresh_generation,
        ) {
          return;
        }
        let branch_changed = this.apply_status_snapshot(
          entries,
          branch_status,
          head_status,
          unpublished_branch_commits,
          merge_in_progress,
          rebase_in_progress,
          rebase_commit_message,
          false,
          cx,
        );
        if unpublished_branch_checked {
          this.unpublished_branch_checked_at = unpublished_branch_check_key
            .as_ref()
            .map(|_| unpublished_branch_checked_at);
        }
        this.unpublished_branch_check_key = unpublished_branch_check_key;
        if include_history {
          if let Some(history) = history {
            this.history_commits = history;
            this.sync_history_cache_with_commits();
            this.history_loading = false;
            if let Some(history_revision) = polled_history_revision {
              this.history_revision = Some(history_revision);
            }
            this.refresh_history_list(cx);
          } else if !should_refresh_history {
            if let Some(history_revision) = polled_history_revision {
              this.history_revision = Some(history_revision);
            }
          } else if this.history_loading {
            this.history_loading = false;
          }
        }
        if branch_changed {
          this.refresh_branches(cx);
        }
        if Self::should_refresh_file_list(this.sidebar_mode) {
          this.refresh_file_list(cx);
        }
        cx.notify();
      });
    });

    self.status_task = Some(task);
  }

  fn show_command_palette_action(
    &mut self,
    _: &ShowCommandPalette,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.open_command_palette(window, cx, None);
  }

  fn show_branch_switcher_action(
    &mut self,
    _: &crate::ShowBranchSwitcher,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.selected_repo.is_none() {
      return;
    }

    self.open_command_palette(window, cx, Some(CommandPaletteInitialScreen::SwitchBranch));
    cx.stop_propagation();
  }

  fn show_file_search_action(
    &mut self,
    _: &ShowFileSearch,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.open_file_search_palette(window, cx);
  }

  fn find_action(&mut self, action: &Find, window: &mut Window, cx: &mut Context<Self>) {
    let Some(editor) = self.editor.clone() else {
      return;
    };

    editor.update(cx, |editor, cx| {
      editor::find(editor, action, window, cx);
    });
  }

  fn close_find_action(&mut self, action: &CloseFind, window: &mut Window, cx: &mut Context<Self>) {
    let Some(editor) = self.editor.clone() else {
      return;
    };

    editor.update(cx, |editor, cx| {
      editor::close_find(editor, action, window, cx);
    });
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

  fn open_command_palette(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
    initial_screen: Option<CommandPaletteInitialScreen>,
  ) {
    let mut palette_repositories = ConfigStore::load_recent_repositories()
      .into_iter()
      .map(|repo| CommandPaletteRepository {
        path: repo.path.to_string_lossy().replace(['\n', '\r'], "").into(),
      })
      .collect::<Vec<_>>();

    if let Some(selected_repo) = self.selected_repo.as_ref() {
      let selected_repo_path = selected_repo.to_string_lossy().replace(['\n', '\r'], "");
      if !palette_repositories
        .iter()
        .any(|repo| repo.path.as_ref() == selected_repo_path)
      {
        palette_repositories.insert(
          0,
          CommandPaletteRepository {
            path: selected_repo_path.into(),
          },
        );
      }
    }

    let GitCommandPaletteContents {
      commands,
      branches: palette_branches,
      rebase_branches: palette_rebase_branches,
      delete_branches: palette_delete_branches,
      stashes: palette_stashes,
      default_stash_message: palette_default_stash_message,
    } = self.build_command_palette_contents(palette_repositories.len(), cx);

    let view = cx.entity();
    let handler: CommandPaletteHandler = Arc::new(move |action, window, cx| {
      view.update(cx, |view, cx| {
        view.handle_command_palette_action(action, window, cx)
      })
    });

    let mut config = CommandPaletteConfig::new(palette_branches, commands, handler)
      .with_repositories(palette_repositories)
      .with_rebase_branches(palette_rebase_branches)
      .with_delete_branches(palette_delete_branches)
      .with_stashes(palette_stashes);
    if let Some(default_stash_message) = palette_default_stash_message {
      config = config.with_default_stash_message(default_stash_message);
    }
    if let Some(initial_screen) = initial_screen {
      config = config.with_initial_screen(initial_screen);
    }

    let palette = cx.new(|cx| CommandPalette::new(window, cx, config));
    let palette_for_dialog = palette.clone();

    window.open_dialog(cx, move |dialog, _, _| {
      dialog
        .on_ok(|_, _, _| false)
        .p_0()
        .border_0()
        .min_h_0()
        .overlay_closable(true)
        .keyboard(true)
        .close_button(false)
        .child(palette_for_dialog.clone())
    });
  }

  fn command_palette_branch(branch: &BranchRef) -> CommandPaletteBranch {
    CommandPaletteBranch {
      name: branch.name.clone().into(),
      kind: match branch.kind {
        BranchKind::Local => CommandPaletteBranchKind::Local,
        BranchKind::Remote => CommandPaletteBranchKind::Remote,
      },
    }
  }

  fn command_palette_rebase_branches(
    branches: &[BranchRef],
    current_branch_name: Option<&str>,
    upstream_branch: Option<&BranchRef>,
    default_branch: Option<&BranchRef>,
  ) -> Vec<CommandPaletteBranch> {
    let mut candidates = branches
      .iter()
      .enumerate()
      .filter(|(_, branch)| {
        !matches!(
          (branch.kind, current_branch_name),
          (BranchKind::Local, Some(current_branch_name)) if branch.name == current_branch_name
        )
      })
      .map(|(index, branch)| {
        (
          index,
          Self::rebase_branch_priority(branch, upstream_branch, default_branch),
          branch,
        )
      })
      .collect::<Vec<_>>();

    candidates.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    candidates
      .into_iter()
      .map(|(_, _, branch)| Self::command_palette_branch(branch))
      .collect()
  }

  fn rebase_branch_priority(
    branch: &BranchRef,
    upstream_branch: Option<&BranchRef>,
    default_branch: Option<&BranchRef>,
  ) -> usize {
    if upstream_branch.is_some_and(|upstream| upstream == branch) {
      return 0;
    }
    if default_branch.is_some_and(|default| default == branch) {
      return 1;
    }
    match (branch.kind, branch.name.as_str()) {
      (BranchKind::Local, "main" | "master") => 2,
      (
        BranchKind::Remote,
        "origin/main" | "origin/master" | "upstream/main" | "upstream/master",
      ) => 2,
      _ => 3,
    }
  }

  fn build_command_palette_contents(
    &self,
    palette_repositories_len: usize,
    cx: &App,
  ) -> GitCommandPaletteContents {
    let include_github = self.auth_state.has_github_access();
    let mut commands = Vec::new();
    let mut stashes = Vec::new();
    let mut default_stash_message_value = None;
    let mut branches = Vec::new();
    let mut rebase_branches = Vec::new();
    let mut delete_branches = Vec::new();

    if let Some(root_path) = self.selected_repo.clone() {
      let commit_message = self.commit_input.read(cx).value().to_string();
      if self.should_show_commit_palette_command(&commit_message) {
        commands.push(CommandPaletteCommand::commit());
      }
      if self.should_show_continue_rebase_palette_command() {
        commands.push(CommandPaletteCommand::continue_rebase());
      } else if let Some(reason) = self.continue_rebase_disabled_reason() {
        commands.push(CommandPaletteCommand::continue_rebase().disabled(reason));
      }
      if self.should_show_skip_rebase_palette_command() {
        commands.push(CommandPaletteCommand::skip_rebase());
      }
      if self.should_show_push_palette_command() {
        let push_label = Self::push_action_label(self.branch_status.as_ref(), self.has_head_commit);
        commands.push(CommandPaletteCommand::push(push_label));
      }
      if self.should_show_force_push_palette_command() {
        commands.push(CommandPaletteCommand::force_push());
      }
      if self.should_show_undo_last_commit_palette_command() {
        commands.push(CommandPaletteCommand::undo_last_commit());
      }
      if self.should_show_amend_palette_command() {
        commands.push(CommandPaletteCommand::amend());
      }
      if self.should_show_checkout_detached_palette_command() {
        commands.push(CommandPaletteCommand::checkout_detached());
      }

      if Self::should_show_stage_all_command(&self.status_entries) {
        commands.push(CommandPaletteCommand::stage_all());
      }
      if Self::should_show_unstage_all_palette_command(&self.status_entries) {
        commands.push(CommandPaletteCommand::unstage_all());
      }
      if self.should_show_unstage_selected_file_palette_command() {
        commands.push(CommandPaletteCommand::unstage_selected_file());
      } else if self.should_show_stage_selected_file_palette_command() {
        commands.push(CommandPaletteCommand::stage_selected_file());
      }
      if self.should_show_accept_all_conflicts_palette_commands(cx) {
        commands.push(CommandPaletteCommand::accept_all_current_conflicts());
        commands.push(CommandPaletteCommand::accept_all_incoming_conflicts());
      }
      if self.should_show_pull_palette_command() {
        commands.push(CommandPaletteCommand::pull());
      } else if self.selected_repo.is_some()
        && self
          .branch_status
          .as_ref()
          .is_some_and(|status| status.has_upstream)
        && let Some(reason) = self.operation_in_progress_disabled_reason()
      {
        commands.push(CommandPaletteCommand::pull().disabled(reason));
      }
      commands.push(CommandPaletteCommand::fetch());
      if self.should_show_cherry_pick_palette_command() {
        commands.push(CommandPaletteCommand::cherry_pick());
      } else if self.selected_repo.is_some()
        && let Some(reason) = self.operation_in_progress_disabled_reason()
      {
        commands.push(CommandPaletteCommand::cherry_pick().disabled(reason));
      }

      let (show_stash, show_stash_with_untracked) = Self::stash_command_flags(&self.status_entries);

      if show_stash {
        commands.push(CommandPaletteCommand::stash());
      }

      if show_stash_with_untracked {
        commands.push(CommandPaletteCommand::stash_with_untracked());
        default_stash_message_value = default_stash_message(&root_path).ok().map(Into::into);
      }

      if let Ok(repo_stashes) = list_stashes(&root_path) {
        stashes = repo_stashes
          .into_iter()
          .map(|stash| CommandPaletteStash {
            index: stash.index,
            name: stash.name.into(),
            oid: stash.oid.into(),
          })
          .collect();

        if !stashes.is_empty() {
          commands.push(CommandPaletteCommand::apply_stash());
          commands.push(CommandPaletteCommand::drop_stash());
          commands.push(CommandPaletteCommand::pop_stash());
        }
      }

      if let Ok(repo_branches) = list_branches(&root_path) {
        let current_branch_name = self
          .branch_status
          .as_ref()
          .map(|status| status.name.clone())
          .or_else(|| {
            current_branch_status(&root_path)
              .ok()
              .map(|status| status.name)
          });
        delete_branches = repo_branches
          .iter()
          .filter(|branch| match branch.kind {
            BranchKind::Local => current_branch_name
              .as_ref()
              .map_or(true, |current_branch_name| {
                branch.name != *current_branch_name
              }),
            BranchKind::Remote => true,
          })
          .map(|branch| CommandPaletteBranch {
            name: branch.name.clone().into(),
            kind: match branch.kind {
              BranchKind::Local => CommandPaletteBranchKind::Local,
              BranchKind::Remote => CommandPaletteBranchKind::Remote,
            },
          })
          .collect::<Vec<_>>();
        let upstream_branch = current_branch_upstream(&root_path).ok().flatten();
        let default_branch = default_remote_branch(&root_path).ok().flatten();
        rebase_branches = Self::command_palette_rebase_branches(
          &repo_branches,
          current_branch_name.as_deref(),
          upstream_branch.as_ref(),
          default_branch.as_ref(),
        );
        branches = repo_branches
          .iter()
          .map(Self::command_palette_branch)
          .collect::<Vec<_>>();
        commands.push(CommandPaletteCommand::switch_branch());
        if !delete_branches.is_empty() {
          commands.push(CommandPaletteCommand::delete_branch());
        }
        if self.should_show_merge_branch_palette_command() {
          commands.push(CommandPaletteCommand::merge_branch());
        } else if let Some(reason) = self.operation_in_progress_disabled_reason() {
          commands.push(CommandPaletteCommand::merge_branch().disabled(reason));
        }
        if self.merge_in_progress {
          commands.push(CommandPaletteCommand::abort_merge());
        }
        if self.should_show_rebase_branch_palette_command() && !rebase_branches.is_empty() {
          commands.push(CommandPaletteCommand::rebase_branch());
        } else if let Some(reason) = self.operation_in_progress_disabled_reason() {
          commands.push(CommandPaletteCommand::rebase_branch().disabled(reason));
        }
        if self.should_show_interactive_rebase_palette_command() {
          commands.push(CommandPaletteCommand::interactive_rebase());
        } else if let Some(reason) = self.interactive_rebase_disabled_reason() {
          commands.push(CommandPaletteCommand::interactive_rebase().disabled(reason));
        }
        if self.rebase_in_progress {
          commands.push(CommandPaletteCommand::abort_rebase());
        }
      }

      if let Some(command) = self.branch_pull_request_palette_command(cx) {
        commands.push(command);
      }
    }

    if palette_repositories_len > 1 {
      commands.push(CommandPaletteCommand::switch_repository());
    }
    if palette_repositories_len > 0 {
      commands.push(CommandPaletteCommand::forget_repository());
    }
    commands.push(CommandPaletteCommand::open_repository());
    commands.extend(CommandPaletteCommand::default_global_commands(
      CommandPalettePage::Git,
      include_github,
    ));

    GitCommandPaletteContents {
      commands,
      branches,
      rebase_branches,
      delete_branches,
      stashes,
      default_stash_message: default_stash_message_value,
    }
  }

  fn open_file_search_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.selected_repo.is_none() {
      return;
    }

    let entries = self.git_file_search_entries();
    if entries.is_empty() {
      return;
    }

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
    open_shared_file_search_palette(window, cx, entries, handler, false);
  }

  fn git_file_search_entries(&self) -> Vec<SearchFileEntry> {
    let Some(root_path) = self.selected_repo.as_ref() else {
      return Vec::new();
    };

    let changed_entries = self
      .status_entries
      .iter()
      .map(|entry| {
        let file_label = entry.path.to_string_lossy();
        let file_label = file_label.replace(['\n', '\r'], "");
        SearchFileEntry::new(entry.path.clone(), file_label).grouped("Changed")
      })
      .collect::<Vec<_>>();

    let mut changed_paths = HashSet::new();
    for entry in &self.status_entries {
      changed_paths.insert(entry.path.clone());
      if let Some(old_path) = entry.old_path.as_ref() {
        changed_paths.insert(old_path.clone());
      }
    }

    let unchanged_entries = list_repo_head_files(root_path)
      .unwrap_or_default()
      .into_iter()
      .filter(|path| !changed_paths.contains(path))
      .map(|path| {
        let file_label = path.to_string_lossy();
        let file_label = file_label.replace(['\n', '\r'], "");
        SearchFileEntry::new(path, file_label).grouped("Unchanged")
      });

    changed_entries
      .into_iter()
      .chain(unchanged_entries)
      .collect()
  }

  fn merge_branch_action(
    &mut self,
    branch_ref: BranchRef,
    mut window: Option<&mut Window>,
    reveal_first_conflict_on_open: bool,
    cx: &mut Context<Self>,
  ) -> Result<(), anyhow::Error> {
    let Some(root_path) = self.selected_repo.clone() else {
      anyhow::bail!("No repository selected.");
    };
    let target_branch = current_branch_status(&root_path)
      .ok()
      .map(|status| status.name)
      .or_else(|| {
        self
          .branch_status
          .as_ref()
          .map(|status| status.name.clone())
      })
      .unwrap_or_else(|| "HEAD".to_string());

    let mut start_data = Map::new();
    start_data.insert("target_branch".into(), branch_ref.name.clone().into());
    self.add_git_breadcrumb("Merge started", start_data);

    match merge_branch(&root_path, &branch_ref) {
      Ok(outcome) => {
        let mut data = Map::new();
        data.insert("target_branch".into(), branch_ref.name.clone().into());
        let notification = match outcome {
          MergeBranchOutcome::AlreadyUpToDate => {
            self.add_git_breadcrumb("Merge already up to date", data);
            Notification::info(format!("Already up to date with {}", branch_ref.name))
          }
          MergeBranchOutcome::Merged => {
            self.add_git_breadcrumb("Merge succeeded", data);
            Notification::success(format!("Merged {}", branch_ref.name))
          }
        };
        if let Some(window) = window {
          window.push_notification(notification, cx);
        }
        Ok(())
      }
      Err(err) => {
        let err_text = err.to_string();
        if let Some(path) = Self::first_conflicted_path(&root_path) {
          self.merge_in_progress = true;
          self.rebase_in_progress = false;
          let mut data = Map::new();
          data.insert("target_branch".into(), branch_ref.name.clone().into());
          data.insert(
            "file".into(),
            path.to_string_lossy().replace(['\n', '\r'], "").into(),
          );
          data.insert("error".into(), err_text.into());
          self.record_git_expected_error("git.merge", "conflict", data.clone());
          self.add_git_breadcrumb("Merge has conflicts", data);

          let merge_message =
            Self::merge_commit_message(branch_ref.name.as_str(), target_branch.as_str());
          if let Some(w) = window.as_deref_mut() {
            self.set_sidebar_mode(GitSidebarMode::Changes, w, cx);
          }
          self.set_commit_input_value(&merge_message, window, cx);
          if reveal_first_conflict_on_open {
            self.open_file_revealing_first_conflict(path, cx);
          } else {
            self.open_status_file(path, cx);
          }
          Ok(())
        } else {
          let mut data = Map::new();
          data.insert("target_branch".into(), branch_ref.name.clone().into());
          data.insert("error".into(), err_text.clone().into());
          self.add_git_breadcrumb("Merge failed", data.clone());
          self.record_git_unexpected_error("git.merge", err_text.as_str(), data);
          Err(err)
        }
      }
    }
  }

  fn handle_command_palette_action(
    &mut self,
    action: CommandPaletteAction,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    let mut should_post_action_refresh = true;
    let action_for_error = action.clone();
    let result = match action {
      CommandPaletteAction::OpenRepository => {
        self.start_open_repository(window, cx);
        Ok(())
      }
      CommandPaletteAction::OpenGitPage => {
        NavigationHistory::navigate("/git", cx);
        Ok(())
      }
      CommandPaletteAction::OpenGithubPage => {
        GithubPageHandle::refresh(cx);
        NavigationHistory::navigate("/github", cx);
        Ok(())
      }
      CommandPaletteAction::OpenGithubPrDetails {
        owner,
        repo,
        number,
        open_changes_tab,
        review_comment_id,
      } => {
        GithubPrDetailsPageHandle::show_with_open_target(
          owner.into(),
          repo.into(),
          number,
          open_changes_tab,
          review_comment_id,
          cx,
        );
        Ok(())
      }
      CommandPaletteAction::OpenGithubRepoDetails {
        owner,
        repo,
        tab,
        issue_number,
        issue_comment_id,
      } => {
        match tab {
          Some(CommandPaletteGithubRepoTab::PullRequests) => {
            GithubRepoPageHandle::show_pull_requests(owner.into(), repo.into(), cx);
          }
          Some(CommandPaletteGithubRepoTab::Issues) => {
            GithubRepoPageHandle::show_issues(
              owner.into(),
              repo.into(),
              issue_number,
              issue_comment_id,
              cx,
            );
          }
          Some(CommandPaletteGithubRepoTab::Overview) | None => {
            GithubRepoPageHandle::show(owner.into(), repo.into(), cx);
          }
        }
        Ok(())
      }
      CommandPaletteAction::OpenGithubCommitDetails { owner, repo, sha } => {
        GithubCommitDetailsPageHandle::show(owner.into(), repo.into(), sha.into(), cx);
        Ok(())
      }
      CommandPaletteAction::OpenGithubProfile { login } => {
        GithubProfilePageHandle::show(login.into(), cx);
        Ok(())
      }
      CommandPaletteAction::SwitchToPrBranch
      | CommandPaletteAction::CopyPrBranch
      | CommandPaletteAction::ToggleUnchangedFiles
      | CommandPaletteAction::OpenPrMergePopover
      | CommandPaletteAction::OpenPrReviewPopover
      | CommandPaletteAction::TogglePrCommitByCommit => {
        Err(anyhow::anyhow!("Command not available."))
      }
      CommandPaletteAction::CreatePullRequest => {
        let branch_context = self
          .create_pull_request_branch_context(cx)
          .ok_or_else(|| SharedString::from("Command not available."))?;
        let api = self.api.clone();
        let window_handle = self.window_handle;
        let git_page = cx.entity().downgrade();
        window.on_next_frame(move |window, cx| {
          open_create_pull_request_dialog(api, window_handle, git_page, branch_context, window, cx);
        });
        Ok(())
      }
      CommandPaletteAction::OpenPullRequest => {
        let GitBranchPullRequestButtonState::OpenExisting {
          owner,
          repo,
          number,
        } = self.current_branch_pr_button_state(cx)
        else {
          return Err("No open pull request found for this branch.".into());
        };
        GithubPrDetailsPageHandle::show_with_open_target(
          owner.into(),
          repo.into(),
          number,
          false,
          None,
          cx,
        );
        Ok(())
      }
      CommandPaletteAction::OpenSettingsPage => {
        NavigationHistory::navigate("/settings", cx);
        Ok(())
      }
      CommandPaletteAction::OpenBillingPage => {
        NavigationHistory::navigate("/billing", cx);
        Ok(())
      }
      CommandPaletteAction::OpenAboutPage => {
        NavigationHistory::navigate("/about", cx);
        Ok(())
      }
      CommandPaletteAction::OpenGitConfigPage => {
        NavigationHistory::navigate("/git-config", cx);
        Ok(())
      }
      CommandPaletteAction::SendFeedback => {
        should_post_action_refresh = false;
        crate::feedback_dialog::open_feedback_dialog(window, cx);
        Ok(())
      }
      CommandPaletteAction::CreateGithubRepository => {
        should_post_action_refresh = false;
        let api = WorkspaceApi::global(cx).api.clone();
        crate::github_create_repository_dialog::open_create_repository_dialog(api, window, cx);
        Ok(())
      }
      CommandPaletteAction::SearchGithubRepository => {
        should_post_action_refresh = false;
        let api = WorkspaceApi::global(cx).api.clone();
        crate::github_search_dialog::open_github_search_dialog(api, window, cx);
        Ok(())
      }
      CommandPaletteAction::CheckoutDetached { target } => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        if !self.should_show_checkout_detached_palette_command() {
          return Err("Checkout detached is currently disabled.".into());
        }
        self.advance_status_refresh_generation();
        checkout_detached_target(&root_path, &target)
      }
      CommandPaletteAction::Commit => {
        if self.selected_repo.is_none() {
          return Err("No repository selected.".into());
        }
        let commit_message = self.commit_input.read(cx).value().to_string();
        if !self.should_show_commit_palette_command(&commit_message) {
          return Err("Commit command is currently disabled.".into());
        }
        should_post_action_refresh = false;
        self.commit_changes_inner(window, cx);
        Ok(())
      }
      CommandPaletteAction::ContinueRebase => {
        if self.selected_repo.is_none() {
          return Err("No repository selected.".into());
        }
        if !self.should_show_continue_rebase_palette_command() {
          return Err("Rebase continue is currently disabled.".into());
        }
        should_post_action_refresh = false;
        self.continue_rebase_inner(cx);
        Ok(())
      }
      CommandPaletteAction::SkipRebase => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        if !self.should_show_skip_rebase_palette_command() {
          return Err("No rebase in progress.".into());
        }
        self.add_git_breadcrumb("Skip rebase started", Map::new());
        match skip_rebase(&root_path) {
          Ok(()) => {
            if !is_rebase_in_progress(&root_path).unwrap_or(false) {
              self.force_push_after_rebase = true;
            }
            self.add_git_breadcrumb("Skip rebase succeeded", Map::new());
            Ok(())
          }
          Err(err) => {
            let err_text = err.to_string();
            if let Some(path) = Self::first_conflicted_path(&root_path) {
              let mut data = Map::new();
              data.insert("error".into(), err_text.into());
              data.insert(
                "file".into(),
                path.to_string_lossy().replace(['\n', '\r'], "").into(),
              );
              self.record_git_expected_error("git.rebase.skip", "conflict", data.clone());
              self.add_git_breadcrumb("Skip rebase blocked by conflicts", data);
              if let Some(rebase_message) = current_rebase_commit_message(&root_path).ok().flatten()
              {
                self
                  .commit_input
                  .update(cx, |input, cx| input.set_value(&rebase_message, window, cx));
              }
              self.set_sidebar_mode(GitSidebarMode::Changes, window, cx);
              self.open_file_revealing_first_conflict(path, cx);
              Ok(())
            } else {
              let mut data = Map::new();
              data.insert("error".into(), err_text.clone().into());
              self.add_git_breadcrumb("Skip rebase failed", data.clone());
              self.record_git_unexpected_error("git.rebase.skip", err_text.as_str(), data);
              Err(err)
            }
          }
        }
      }
      CommandPaletteAction::Push => {
        if self.selected_repo.is_none() {
          return Err("No repository selected.".into());
        }
        if !self.should_show_push_palette_command() {
          return Err("Push command is currently disabled.".into());
        }
        should_post_action_refresh = false;
        self.push_changes_action(cx);
        Ok(())
      }
      CommandPaletteAction::ForcePush => {
        if self.selected_repo.is_none() {
          return Err("No repository selected.".into());
        }
        if !self.should_show_force_push_palette_command() {
          return Err("Force push command is currently disabled.".into());
        }
        should_post_action_refresh = false;
        self.force_push_changes_action(cx);
        Ok(())
      }
      CommandPaletteAction::UndoLastCommit => {
        if self.selected_repo.is_none() {
          return Err("No repository selected.".into());
        }
        if !self.should_show_undo_last_commit_palette_command() {
          return Err("Undo last commit command is currently disabled.".into());
        }
        should_post_action_refresh = false;
        self.undo_last_commit_action(cx);
        Ok(())
      }
      CommandPaletteAction::Amend => {
        if self.selected_repo.is_none() {
          return Err("No repository selected.".into());
        }
        if !self.should_show_amend_palette_command() {
          return Err("Amend command is currently disabled.".into());
        }
        should_post_action_refresh = false;
        self.commit_amend_changes(window, cx);
        Ok(())
      }
      CommandPaletteAction::StageSelectedFile => {
        if self.selected_repo.is_none() {
          return Err("No repository selected.".into());
        }
        if !self.should_show_stage_selected_file_palette_command() {
          return Err("Stage file command is currently disabled.".into());
        }
        let Some(selected_entry) = self.selected_file_entry().cloned() else {
          return Err("Stage file command is currently disabled.".into());
        };
        should_post_action_refresh = false;
        let has_unresolved_conflict_markers = self.editor.as_ref().is_none_or(|editor| {
          editor.read_with(cx, |editor, cx| editor.has_unresolved_conflict_markers(cx))
        });
        if Self::should_confirm_stage_for_status(
          Some(selected_entry.status),
          has_unresolved_conflict_markers,
        ) {
          self.confirm_stage_conflicted_file_action(window, selected_entry.path.clone(), cx);
        } else {
          self.stage_file_action(selected_entry.path.clone(), cx);
        }
        Ok(())
      }
      CommandPaletteAction::UnstageSelectedFile => {
        if self.selected_repo.is_none() {
          return Err("No repository selected.".into());
        }
        if !self.should_show_unstage_selected_file_palette_command() {
          return Err("Unstage file command is currently disabled.".into());
        }
        let Some(selected_entry) = self.selected_file_entry().cloned() else {
          return Err("Unstage file command is currently disabled.".into());
        };
        should_post_action_refresh = false;
        self.unstage_file_action(selected_entry.path.clone(), cx);
        Ok(())
      }
      CommandPaletteAction::AcceptAllCurrentConflicts => {
        if self.selected_repo.is_none() {
          return Err("No repository selected.".into());
        }
        if !self.should_show_accept_all_conflicts_palette_commands(cx) {
          return Err("Accept all current conflicts is currently disabled.".into());
        }
        should_post_action_refresh = false;
        self.resolve_all_conflicts_in_editor(ConflictResolution::Current, cx);
        Ok(())
      }
      CommandPaletteAction::AcceptAllIncomingConflicts => {
        if self.selected_repo.is_none() {
          return Err("No repository selected.".into());
        }
        if !self.should_show_accept_all_conflicts_palette_commands(cx) {
          return Err("Accept all incoming conflicts is currently disabled.".into());
        }
        should_post_action_refresh = false;
        self.resolve_all_conflicts_in_editor(ConflictResolution::Incoming, cx);
        Ok(())
      }
      CommandPaletteAction::SwitchRepository(repository) => {
        let repo_root = PathBuf::from(repository.path.as_ref());
        if !repo_root.is_dir() {
          let message: SharedString =
            format!("Repository not found: {}", repo_root.display()).into();
          return Err(message);
        }
        self.set_selected_repo(repo_root, cx);
        Ok(())
      }
      CommandPaletteAction::ForgetRepository(repository) => {
        should_post_action_refresh = false;
        let repo_root = PathBuf::from(repository.path.as_ref());
        let forgetting_selected = self.selected_repo.as_deref() == Some(repo_root.as_path());
        ConfigStore::forget_recent_repository(&repo_root);

        if forgetting_selected {
          let next_repo = ConfigStore::load_recent_repositories()
            .into_iter()
            .map(|repo| repo.path)
            .find(|path| path != &repo_root);
          match next_repo {
            Some(next) => self.set_selected_repo(next, cx),
            None => self.clear_selected_repo(cx),
          }
        } else {
          self.refresh_repo_select(cx);
        }
        Ok(())
      }
      CommandPaletteAction::SwitchBranch(branch) => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
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
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        let branch_ref = BranchRef {
          name: name.clone(),
          kind: BranchKind::Local,
        };
        create_branch(&root_path, &name).and_then(|_| switch_branch(&root_path, &branch_ref))
      }
      CommandPaletteAction::CreateBranchFrom { name, base } => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
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
      CommandPaletteAction::DeleteBranch(branch) => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        let branch_ref = BranchRef {
          name: branch.name.to_string(),
          kind: match branch.kind {
            CommandPaletteBranchKind::Local => BranchKind::Local,
            CommandPaletteBranchKind::Remote => BranchKind::Remote,
          },
        };
        let result = delete_branch(&root_path, &branch_ref);
        if result.is_ok() {
          window.push_notification(
            Notification::success(format!("Deleted branch {}", branch_ref.name)),
            cx,
          );
        }
        result
      }
      CommandPaletteAction::MergeBranch { name } => {
        if self.selected_repo.is_none() {
          return Err("No repository selected.".into());
        }
        if !self.should_show_merge_branch_palette_command() {
          return Err("Merge command is currently disabled.".into());
        }
        let branch_ref = BranchRef {
          name: name.name.to_string(),
          kind: match name.kind {
            CommandPaletteBranchKind::Local => BranchKind::Local,
            CommandPaletteBranchKind::Remote => BranchKind::Remote,
          },
        };
        self.merge_branch_action(branch_ref, Some(window), true, cx)
      }
      CommandPaletteAction::AbortMerge => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        let result = abort_merge(&root_path);
        if result.is_ok() {
          self
            .commit_input
            .update(cx, |input, cx| input.set_value("", window, cx));
          window.push_notification(Notification::success("Aborted merge"), cx);
        }
        result
      }
      CommandPaletteAction::RebaseBranch { name } => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        if !self.should_show_rebase_branch_palette_command() {
          return Err("Rebase command is currently disabled.".into());
        }
        let branch_ref = BranchRef {
          name: name.name.to_string(),
          kind: match name.kind {
            CommandPaletteBranchKind::Local => BranchKind::Local,
            CommandPaletteBranchKind::Remote => BranchKind::Remote,
          },
        };
        let mut start_data = Map::new();
        start_data.insert("target_branch".into(), branch_ref.name.clone().into());
        self.add_git_breadcrumb("Rebase started", start_data);
        match rebase_branch(&root_path, &branch_ref) {
          Ok(outcome) => {
            let mut data = Map::new();
            data.insert("target_branch".into(), branch_ref.name.clone().into());
            match outcome {
              RebaseBranchOutcome::AlreadyUpToDate => {
                self.add_git_breadcrumb("Rebase already up to date", data);
                window.push_notification(
                  Notification::info(format!("Already up to date with {}", branch_ref.name)),
                  cx,
                );
              }
              RebaseBranchOutcome::Rebased => {
                self.force_push_after_rebase = true;
                self.add_git_breadcrumb("Rebase succeeded", data);
                window.push_notification(
                  Notification::success(format!("Rebased onto {}", branch_ref.name)),
                  cx,
                );
              }
            }
            Ok(())
          }
          Err(err) => {
            let err_text = err.to_string();
            if let Some(path) = Self::first_conflicted_path(&root_path) {
              let mut data = Map::new();
              data.insert("target_branch".into(), branch_ref.name.clone().into());
              data.insert(
                "file".into(),
                path.to_string_lossy().replace(['\n', '\r'], "").into(),
              );
              data.insert("error".into(), err_text.into());
              self.record_git_expected_error("git.rebase", "conflict", data.clone());
              self.add_git_breadcrumb("Rebase has conflicts", data);
              if let Some(rebase_message) = current_rebase_commit_message(&root_path).ok().flatten()
              {
                self
                  .commit_input
                  .update(cx, |input, cx| input.set_value(&rebase_message, window, cx));
              }
              self.set_sidebar_mode(GitSidebarMode::Changes, window, cx);
              self.open_file_revealing_first_conflict(path, cx);
              Ok(())
            } else {
              let mut data = Map::new();
              data.insert("target_branch".into(), branch_ref.name.clone().into());
              data.insert("error".into(), err_text.clone().into());
              self.add_git_breadcrumb("Rebase failed", data.clone());
              self.record_git_unexpected_error("git.rebase", err_text.as_str(), data);
              Err(err)
            }
          }
        }
      }
      CommandPaletteAction::InteractiveRebaseBranch { ref name }
      | CommandPaletteAction::InteractiveRebaseEditBranch { ref name } => {
        if self.selected_repo.is_none() {
          return Err("No repository selected.".into());
        }
        if !self.should_show_interactive_rebase_palette_command() {
          return Err("Interactive rebase is currently disabled.".into());
        }
        should_post_action_refresh = false;
        let branch_ref = BranchRef {
          name: name.name.to_string(),
          kind: match name.kind {
            CommandPaletteBranchKind::Local => BranchKind::Local,
            CommandPaletteBranchKind::Remote => BranchKind::Remote,
          },
        };
        let target = if matches!(
          action,
          CommandPaletteAction::InteractiveRebaseEditBranch { .. }
        ) {
          InteractiveRebaseTarget::BranchInPlace(branch_ref)
        } else {
          InteractiveRebaseTarget::Branch(branch_ref)
        };
        let preview = self.prepare_interactive_rebase_commits(&target)?;
        self.dispatch_interactive_rebase_target(target, preview, window, cx);
        Ok(())
      }
      CommandPaletteAction::InteractiveRebaseHeadCount { count } => {
        if self.selected_repo.is_none() {
          return Err("No repository selected.".into());
        }
        if !self.should_show_interactive_rebase_palette_command() {
          return Err("Interactive rebase is currently disabled.".into());
        }
        should_post_action_refresh = false;
        let target = InteractiveRebaseTarget::HeadCount(count);
        let preview = self.prepare_interactive_rebase_commits(&target)?;
        self.dispatch_interactive_rebase_target(target, preview, window, cx);
        Ok(())
      }
      CommandPaletteAction::AbortRebase => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        let result = abort_rebase(&root_path);
        if result.is_ok() {
          self.force_push_after_rebase = false;
          self
            .commit_input
            .update(cx, |input, cx| input.set_value("", window, cx));
          window.push_notification(Notification::success("Aborted rebase"), cx);
        }
        result
      }
      CommandPaletteAction::StageAll => {
        if self.selected_repo.is_none() {
          return Err("No repository selected.".into());
        }
        should_post_action_refresh = false;
        if Self::should_confirm_stage_all(self.selected_repo.as_ref(), &self.status_entries) {
          self.confirm_stage_all_conflicted_action(window, cx);
        } else {
          self.stage_all_action(cx);
        }
        Ok(())
      }
      CommandPaletteAction::UnstageAll => {
        if self.selected_repo.is_none() {
          return Err("No repository selected.".into());
        }
        should_post_action_refresh = false;
        self.unstage_all_action(cx);
        Ok(())
      }
      CommandPaletteAction::Pull => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        if !self.should_show_pull_palette_command() {
          return Err("Pull command is currently disabled.".into());
        }
        should_post_action_refresh = false;
        self.pull_repository(root_path, cx);
        Ok(())
      }
      CommandPaletteAction::Fetch => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        should_post_action_refresh = false;
        self.fetch_repository(root_path, cx);
        Ok(())
      }
      CommandPaletteAction::Stash {
        include_untracked,
        message,
      } => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        let result = create_stash(&root_path, include_untracked, message.as_deref());
        if result.is_ok() {
          window.push_notification(Notification::success("Stashed changes"), cx);
        }
        result
      }
      CommandPaletteAction::ApplyStash(stash) => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        let result = apply_stash(&root_path, stash.index);
        if result.is_ok() {
          window.push_notification(
            Notification::success(format!("Applied stash {}", stash.name)),
            cx,
          );
        }
        result
      }
      CommandPaletteAction::DropStash(stash) => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        let result = drop_stash(&root_path, stash.index);
        if result.is_ok() {
          window.push_notification(
            Notification::success(format!("Dropped stash {}", stash.name)),
            cx,
          );
        }
        result
      }
      CommandPaletteAction::PopStash(stash) => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        let result = pop_stash(&root_path, stash.index);
        if result.is_ok() {
          window.push_notification(
            Notification::success(format!("Popped stash {}", stash.name)),
            cx,
          );
        }
        result
      }
      CommandPaletteAction::CherryPick { commit_hashes } => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
        if !self.should_show_cherry_pick_palette_command() {
          return Err("Cherry-pick command is currently disabled.".into());
        }
        let count = commit_hashes.len();
        let result = cherry_pick_commits(&root_path, &commit_hashes);
        if result.is_ok() {
          let label = if count == 1 { "commit" } else { "commits" };
          window.push_notification(
            Notification::success(format!("Cherry-picked {count} {label}")),
            cx,
          );
        }
        result
      }
    };

    if let Err(err) = result {
      self.handle_command_palette_operation_error(&action_for_error, err, window, cx);
    }

    if should_post_action_refresh {
      self.reload_status(cx);
      self.refresh_branches(cx);
      if let Some(editor) = self.editor.clone() {
        editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
      }
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

  fn open_git_history_sidebar_action(
    &mut self,
    _: &crate::OpenGitHistorySidebar,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.set_sidebar_mode(GitSidebarMode::History, window, cx);
    cx.stop_propagation();
  }

  fn open_git_changes_sidebar_action(
    &mut self,
    _: &crate::OpenGitChangesSidebar,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.set_sidebar_mode(GitSidebarMode::Changes, window, cx);
    cx.stop_propagation();
  }

  fn toggle_diff_view_action(
    &mut self,
    _: &crate::ToggleDiffView,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.toggle_diff_view(cx);
    cx.stop_propagation();
  }

  fn toggle_hide_whitespace_action(
    &mut self,
    _: &crate::ToggleHideWhitespace,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.toggle_hide_whitespace(cx);
    cx.stop_propagation();
  }

  fn previous_annotation_action(
    &mut self,
    _: &crate::PreviousAnnotation,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.navigate_annotation_in_editor(AnnotationDirection::Previous, cx);
    cx.stop_propagation();
  }

  fn next_annotation_action(
    &mut self,
    _: &crate::NextAnnotation,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.navigate_annotation_in_editor(AnnotationDirection::Next, cx);
    cx.stop_propagation();
  }

  fn copy_review_comments_for_agent_action(
    &mut self,
    _: &crate::CopyReviewCommentsForAgent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.copy_agent_review_to_clipboard(window, cx);
    cx.stop_propagation();
  }

  fn comment_hunk_action(
    &mut self,
    _: &crate::CommentHunk,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(editor) = self.editor.clone() else {
      return;
    };
    if editor.update(cx, |editor, cx| {
      editor.start_review_comment_for_active_hunk(window, cx)
    }) {
      cx.stop_propagation();
    }
  }

  fn toggle_hunk_stage_action(
    &mut self,
    _: &crate::ToggleHunkStage,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(editor) = self.editor.clone() else {
      return;
    };
    if self.resolve_active_conflict_in_editor(&editor, ConflictResolution::Current, cx) {
      cx.stop_propagation();
      return;
    }
    editor.update(cx, |editor, cx| {
      let Some(group_id) = editor.active_hunk_group_id(cx) else {
        return;
      };
      let Some(state) = editor
        .projection()
        .and_then(|p| p.groups.get(&group_id))
        .map(|g| g.state)
      else {
        return;
      };
      let action = match state {
        HunkState::Unstaged => HunkAction::Stage,
        HunkState::Staged => HunkAction::Unstage,
      };
      editor.enqueue_group_action(group_id, action, cx);
    });
    cx.stop_propagation();
  }

  fn restore_hunk_action(
    &mut self,
    _: &crate::RestoreHunk,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(editor) = self.editor.clone() else {
      return;
    };
    if self.resolve_active_conflict_in_editor(&editor, ConflictResolution::Incoming, cx) {
      cx.stop_propagation();
      return;
    }
    editor.update(cx, |editor, cx| {
      let Some(group_id) = editor.active_hunk_group_id(cx) else {
        return;
      };
      editor.enqueue_group_action(group_id, HunkAction::Restore, cx);
    });
    cx.stop_propagation();
  }

  fn accept_both_conflict_action(
    &mut self,
    _: &crate::AcceptBothConflict,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(editor) = self.editor.clone() else {
      return;
    };
    self.resolve_active_conflict_in_editor(&editor, ConflictResolution::Both, cx);
    cx.stop_propagation();
  }

  fn sync_editor_unmerged_state(&mut self, cx: &mut Context<Self>) {
    let Some(editor) = self.editor.clone() else {
      return;
    };
    if self.history_opened_commit_file.is_some() {
      editor.update(cx, |editor, cx| editor.set_is_unmerged(false, cx));
      return;
    }
    let is_unmerged = self
      .selected_file_entry()
      .is_some_and(|entry| entry.status == RepoStatusKind::Conflicted);
    editor.update(cx, |editor, cx| editor.set_is_unmerged(is_unmerged, cx));
  }

  fn resolve_active_conflict_in_editor(
    &self,
    editor: &Entity<Editor>,
    resolution: ConflictResolution,
    cx: &mut Context<Self>,
  ) -> bool {
    let file_status = if self.history_opened_commit_file.is_some() {
      None
    } else {
      self.selected_file_entry().map(|entry| entry.status)
    };
    if !matches!(file_status, Some(RepoStatusKind::Conflicted)) {
      return false;
    }
    editor.update(cx, |editor, cx| {
      let Some(state) = editor.conflict_navigation_state(cx) else {
        return false;
      };
      editor.resolve_conflict_region(state.active_start_line, resolution, cx);
      editor.save(cx);
      if let Some(next_state) = editor.conflict_navigation_state(cx) {
        editor.reveal_conflict_start_line(next_state.active_start_line, cx);
      }
      true
    })
  }

  fn toggle_file_stage_action(
    &mut self,
    _: &crate::ToggleFileStage,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.selected_repo.is_none() {
      return;
    }
    let Some(entry) = self.selected_file_entry().cloned() else {
      return;
    };
    if Self::selected_file_can_unstage(entry.stage) {
      self.unstage_file_action(entry.path, cx);
    } else {
      self.stage_file_click_action(window, entry.path, entry.status, cx);
    }
    cx.stop_propagation();
  }

  fn restore_file_shortcut_action(
    &mut self,
    _: &crate::RestoreFile,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.selected_repo.is_none() {
      return;
    }
    let Some(entry) = self.selected_file_entry().cloned() else {
      return;
    };
    if !Self::can_restore_file_stage(entry.stage) {
      return;
    }
    self.confirm_restore_file_action(window, entry.path, entry.old_path, entry.status, cx);
    cx.stop_propagation();
  }

  fn pull_changes_action(
    &mut self,
    _: &crate::PullChanges,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };

    self.pull_repository(repo_root, cx);
    cx.stop_propagation();
  }

  fn push_changes_shortcut_action(
    &mut self,
    _: &crate::PushChanges,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.push_changes_action(cx);
    cx.stop_propagation();
  }

  fn force_push_changes_shortcut_action(
    &mut self,
    _: &crate::ForcePushChanges,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.force_push_changes_action(cx);
    cx.stop_propagation();
  }

  fn commit_changes(&mut self, _: &gpui::ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
    self.commit_changes_inner(window, cx);
  }

  fn commit_changes_inner(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.rebase_in_progress {
      let _ = window;
      self.continue_rebase_inner(cx);
      return;
    }

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
    let mut start_data = Map::new();
    start_data.insert("stage_all_needed".into(), stage_all_needed.into());
    self.add_git_breadcrumb("Commit started", start_data);

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
        match result {
          Ok(()) => {
            let _ = cx.update_window(window_handle, |_, window, cx| {
              commit_input.update(cx, |input, cx| input.set_value("", window, cx));
            });
            let mut data = Map::new();
            data.insert("stage_all_needed".into(), stage_all_needed.into());
            this.add_git_breadcrumb("Commit succeeded", data);
          }
          Err(error) => {
            let error_message = error.to_string();
            let mut data = Map::new();
            data.insert("stage_all_needed".into(), stage_all_needed.into());
            data.insert("error".into(), error_message.clone().into());
            this.add_git_breadcrumb("Commit failed", data.clone());
            this.record_git_unexpected_error("git.commit", error_message.as_str(), data);
            this.push_git_action_error_notification("Commit failed", error_message.into(), cx);
          }
        }
        this.reload_status(cx);
        if let Some(editor) = editor.clone() {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
      });
    });

    self.status_task = Some(task);
  }

  fn continue_rebase_inner(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    if !self.rebase_in_progress {
      return;
    }
    self.operation_error = None;
    if Self::has_conflicted_entries(&self.status_entries) {
      self.operation_error = Some("Resolve all conflicts before continuing the rebase.".into());
      let mut data = Map::new();
      data.insert("reason".into(), "conflicts_present".into());
      self.record_git_expected_error("git.continue_rebase", "conflict", data);
      cx.notify();
      return;
    }

    self.add_git_breadcrumb("Continue rebase started", Map::new());
    let commit_input = self.commit_input.clone();
    let window_handle = self.window_handle;
    let editor = self.editor.clone();
    let task = cx.spawn(async move |this, cx| {
      let repo_root_for_continue = repo_root.clone();
      let result = unblock(move || continue_rebase(&repo_root_for_continue)).await;
      let (success, conflicted_path, error_message, failure_message, expected_conflict) =
        match result {
          Ok(()) => (true, None, None, None, false),
          Err(err) => {
            let conflicted_path = Self::first_conflicted_path(&repo_root);
            let err_text = err.to_string();
            let is_conflict_state =
              conflicted_path.is_some() || err_text.contains("rebase has conflicts");
            let error_message = if is_conflict_state {
              None
            } else {
              Some(format!("Continue rebase failed: {err}"))
            };
            (
              false,
              conflicted_path,
              error_message,
              Some(err_text),
              is_conflict_state,
            )
          }
        };
      let _ = this.update(cx, |this, cx| {
        if success {
          this.rebase_in_progress = false;
          this.force_push_after_rebase = true;
          this.operation_error = None;
          this.add_git_breadcrumb("Continue rebase succeeded", Map::new());
          let _ = cx.update_window(window_handle, |_, window, cx| {
            commit_input.update(cx, |input, cx| input.set_value("", window, cx));
          });
        } else if expected_conflict {
          let mut data = Map::new();
          data.insert("has_conflicts".into(), true.into());
          if let Some(message) = failure_message.clone() {
            data.insert("error".into(), message.into());
          }
          this.record_git_expected_error("git.continue_rebase", "conflict", data.clone());
          this.add_git_breadcrumb("Continue rebase blocked by conflicts", data);
        } else if let Some(message) = failure_message.as_deref() {
          let mut data = Map::new();
          data.insert("error".into(), message.to_string().into());
          this.add_git_breadcrumb("Continue rebase failed", data.clone());
          this.record_git_unexpected_error("git.continue_rebase", message, data);
        }
        this.reload_status(cx);
        if let Some(path) = conflicted_path {
          this.open_status_file(path, cx);
        }
        if let Some(error_message) = error_message {
          this.operation_error = Some(error_message.into());
        }
        if let Some(editor) = editor.clone() {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
        cx.notify();
      });
    });
    self.status_task = Some(task);
  }

  fn prepare_interactive_rebase_commits(
    &self,
    target: &InteractiveRebaseTarget,
  ) -> Result<git::InteractiveRebasePreview, SharedString> {
    let Some(repo_root) = self.selected_repo.clone() else {
      return Err("No repository selected.".into());
    };
    if !self.should_show_interactive_rebase_palette_command() {
      return Err("Interactive rebase is currently disabled.".into());
    }

    let preview = list_interactive_rebase_commits(&repo_root, target)
      .map_err(|err| -> SharedString { format!("Action failed: {err}").into() })?;
    if preview.commits.is_empty() {
      return Err("No commits available for interactive rebase.".into());
    }
    Ok(preview)
  }

  fn dispatch_interactive_rebase_target(
    &mut self,
    target: InteractiveRebaseTarget,
    preview: git::InteractiveRebasePreview,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if preview.dropped_merge_count == 0 {
      let view = cx.entity();
      let commits = preview.commits;
      window.on_next_frame(move |window, cx| {
        let target = target.clone();
        let commits = commits.clone();
        view.update(cx, move |view, cx| {
          view.open_interactive_rebase_todo_view_with_commits(target, commits, window, cx);
        });
      });
      return;
    }

    let count = preview.dropped_merge_count;
    let title: SharedString = "Drop merge commits?".into();
    let message: SharedString = if count == 1 {
      "1 merge commit will be dropped from the rebase. Its changes will be re-applied through the picked commits.".into()
    } else {
      format!(
        "{count} merge commits will be dropped from the rebase. Their changes will be re-applied through the picked commits."
      )
      .into()
    };
    let view = cx.entity();
    let commits = preview.commits;

    window.on_next_frame(move |window, cx| {
      let view = view.clone();
      let target = target.clone();
      let commits = commits.clone();
      let title = title.clone();
      let message = message.clone();
      window.open_alert_dialog(cx, move |alert, _, _| {
        let view = view.clone();
        let target = target.clone();
        let commits = commits.clone();
        ConfirmDialog::new(title.clone(), div().child(message.clone()))
          .confirm_text("Continue")
          .cancel_text("Cancel")
          .on_confirm(move |_, window, cx| {
            let target = target.clone();
            let commits = commits.clone();
            view.update(cx, move |view, cx| {
              view.open_interactive_rebase_todo_view_with_commits(target, commits, window, cx);
            });
            true
          })
          .build(alert)
      });
    });
  }

  fn close_interactive_rebase_todo_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self.interactive_rebase_todo_view = None;
    self.focus_editor_or_page(window, cx);
    cx.on_next_frame(window, |this, window, cx| {
      this.focus_editor_or_page(window, cx);
    });
    cx.notify();
  }

  fn open_interactive_rebase_todo_view_with_commits(
    &mut self,
    target: InteractiveRebaseTarget,
    commits: Vec<git::InteractiveRebaseCommit>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if !self.should_show_interactive_rebase_palette_command() {
      self.operation_error = Some("Interactive rebase is currently disabled.".into());
      cx.notify();
      return;
    }

    let view_for_submit = cx.entity();
    let on_submit: InteractiveRebaseTodoViewHandler =
      Arc::new(move |target, todo_entries, window, cx| {
        view_for_submit.update(cx, |view, cx| {
          let result = view.start_interactive_rebase_action(target, todo_entries, window, cx);
          if result.is_ok() {
            view.close_interactive_rebase_todo_view(window, cx);
          }
          result
        })
      });

    let view_for_cancel = cx.entity();
    let on_cancel: InteractiveRebaseTodoViewCancelHandler = Arc::new(move |window, cx| {
      view_for_cancel.update(cx, |view, cx| {
        view.close_interactive_rebase_todo_view(window, cx);
      });
    });

    let todo_config = InteractiveRebaseTodoViewConfig::new(target, commits, on_submit, on_cancel);
    let todo_view = cx.new(|cx| InteractiveRebaseTodoView::new(window, cx, todo_config));
    self.interactive_rebase_todo_view = Some(todo_view.clone());
    cx.on_next_frame(window, move |_, window, cx| {
      todo_view.update(cx, |view, cx| {
        view.focus_rows_list(window, cx);
      });
    });
    cx.notify();
  }

  fn start_interactive_rebase_action(
    &mut self,
    target: InteractiveRebaseTarget,
    todo_entries: Vec<InteractiveRebaseTodoEntry>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    let Some(repo_root) = self.selected_repo.clone() else {
      return Err("No repository selected.".into());
    };
    if !self.should_show_interactive_rebase_palette_command() {
      return Err("Interactive rebase is currently disabled.".into());
    }

    self.operation_error = None;
    let commit_input = self.commit_input.clone();
    let window_handle = window.window_handle();
    let editor = self.editor.clone();
    let success_message = interactive_rebase_success_message(&target);
    let task = cx.spawn(async move |this, cx| {
      let repo_root_for_rebase = repo_root.clone();
      let result =
        unblock(move || start_interactive_rebase(&repo_root_for_rebase, &target, &todo_entries))
          .await;

      let rebase_in_progress = is_rebase_in_progress(&repo_root).unwrap_or(false);
      let conflicted_path = Self::first_conflicted_path(&repo_root);
      let rebase_message = if rebase_in_progress {
        current_rebase_commit_message(&repo_root).ok().flatten()
      } else {
        None
      };
      let (success, error_message) = match result {
        Ok(()) => (!rebase_in_progress, None),
        Err(err) => {
          let is_conflict_state = conflicted_path.is_some() || rebase_in_progress;
          let error_message = if is_conflict_state {
            None
          } else {
            Some(format!("Interactive rebase failed: {err}"))
          };
          (false, error_message)
        }
      };

      let _ = this.update(cx, |this, cx| {
        if success {
          this.force_push_after_rebase = true;
          this.operation_error = None;
          let _ = cx.update_window(window_handle, |_, window, cx| {
            commit_input.update(cx, |input, cx| input.set_value("", window, cx));
            window.push_notification(Notification::success(success_message), cx);
          });
        }
        this.reload_status(cx);
        this.refresh_branches(cx);
        if let Some(path) = conflicted_path {
          this.open_status_file(path, cx);
        }
        if let Some(message) = rebase_message {
          let _ = cx.update_window(window_handle, |_, window, cx| {
            commit_input.update(cx, |input, cx| input.set_value(&message, window, cx));
          });
        }
        if let Some(error_message) = error_message {
          this.operation_error = Some(error_message.into());
        }
        if let Some(editor) = editor.clone() {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
        cx.notify();
      });
    });
    self.status_task = Some(task);
    Ok(())
  }

  fn resolve_all_conflicts_in_editor(
    &mut self,
    resolution: ConflictResolution,
    cx: &mut Context<Self>,
  ) {
    if let Some(editor) = self.editor.clone() {
      editor.update(cx, |editor, cx| {
        editor.resolve_all_conflicts(resolution, cx);
      });
    }
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
        match result {
          Ok(()) => {
            let _ = cx.update_window(window_handle, |_, window, cx| {
              commit_input.update(cx, |input, cx| input.set_value("", window, cx));
            });
          }
          Err(error) => {
            this.push_git_action_error_notification("Amend failed", error.to_string().into(), cx);
          }
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
      let result = unblock(move || undo_last_commit(&repo_root)).await;
      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(()) => {
            this.push_git_action_success_notification("Undid last commit".into(), cx);
          }
          Err(error) => {
            this.push_git_action_error_notification(
              "Undo last commit failed",
              error.to_string().into(),
              cx,
            );
          }
        }
        this.reload_status(cx);
        if let Some(editor) = editor.clone() {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
      });
    });

    self.status_task = Some(task);
  }

  fn fetch_action(&mut self, _: &gpui::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    self.fetch_repository(repo_root, cx);
  }

  fn fetch_repository(&mut self, repo_root: PathBuf, cx: &mut Context<Self>) {
    if self.fetch_in_progress {
      return;
    }
    self.add_git_breadcrumb("Fetch started", Map::new());
    self.fetch_in_progress = true;
    let editor = self.editor.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || fetch(&repo_root)).await;
      let _ = this.update(cx, |this, cx| {
        this.fetch_in_progress = false;
        match result {
          Ok(()) => {
            this.add_git_breadcrumb("Fetch succeeded", Map::new());
            this.push_git_action_success_notification("Fetched from remotes".into(), cx);
          }
          Err(error) => {
            let error_message = error.to_string();
            let mut data = Map::new();
            data.insert("error".into(), error_message.clone().into());
            this.add_git_breadcrumb("Fetch failed", data.clone());
            this.record_git_unexpected_error("git.fetch", error_message.as_str(), data);
            this.push_git_action_error_notification("Fetch failed", error_message.into(), cx);
          }
        }
        this.reload_status(cx);
        this.refresh_branches(cx);
        if let Some(editor) = editor.clone() {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
      });
    });

    self.status_task = Some(task);
  }

  fn pull_repository(&mut self, repo_root: PathBuf, cx: &mut Context<Self>) {
    if self.push_pull_in_progress {
      return;
    }
    if !self.should_show_pull_palette_command() {
      return;
    }
    self.add_git_breadcrumb("Pull started", Map::new());
    self.push_pull_in_progress = true;
    let editor = self.editor.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || pull(&repo_root)).await;
      let _ = this.update(cx, |this, cx| {
        this.push_pull_in_progress = false;
        match result {
          Ok(PullOutcome::AlreadyUpToDate) => {
            this.add_git_breadcrumb("Pull already up to date", Map::new());
            this.push_git_action_success_notification("Already up to date".into(), cx);
          }
          Ok(PullOutcome::Pulled) => {
            this.add_git_breadcrumb("Pull succeeded", Map::new());
            this.push_git_action_success_notification("Pulled".into(), cx);
          }
          Err(error) => {
            let error_message = error.to_string();
            let mut data = Map::new();
            data.insert("error".into(), error_message.clone().into());
            this.add_git_breadcrumb("Pull failed", data.clone());
            this.record_git_unexpected_error("git.pull", error_message.as_str(), data);
            this.push_git_action_error_notification("Pull failed", error_message.into(), cx);
          }
        }
        this.reload_status(cx);
        this.refresh_branches(cx);
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

    self.add_git_breadcrumb("Push started", Map::new());
    self.push_pull_in_progress = true;
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || push(&repo_root, false)).await;
      let _ = this.update(cx, |this, cx| {
        this.push_pull_in_progress = false;
        match result {
          Ok(()) => {
            this.force_push_after_rebase = false;
            this.add_git_breadcrumb("Push succeeded", Map::new());
            this.push_git_action_success_notification("Pushed".into(), cx);
          }
          Err(error) => {
            let error_message = error.to_string();
            let mut data = Map::new();
            data.insert("error".into(), error_message.clone().into());
            this.add_git_breadcrumb("Push failed", data.clone());
            this.record_git_unexpected_error("git.push", error_message.as_str(), data);
            this.push_git_action_error_notification("Push failed", error_message.into(), cx);
          }
        }
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

    self.add_git_breadcrumb("Force push started", Map::new());
    self.push_pull_in_progress = true;
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || push(&repo_root, true)).await;
      let _ = this.update(cx, |this, cx| {
        this.push_pull_in_progress = false;
        match result {
          Ok(()) => {
            this.force_push_after_rebase = false;
            this.add_git_breadcrumb("Force push succeeded", Map::new());
            this.push_git_action_success_notification("Force-pushed".into(), cx);
          }
          Err(error) => {
            let error_message = error.to_string();
            let mut data = Map::new();
            data.insert("error".into(), error_message.clone().into());
            this.add_git_breadcrumb("Force push failed", data.clone());
            this.record_git_unexpected_error("git.force_push", error_message.as_str(), data);
            this.push_git_action_error_notification("Force push failed", error_message.into(), cx);
          }
        }
        this.reload_status(cx);
      });
    });

    self.status_task = Some(task);
  }

  fn reveal_first_conflict_in_editor(&mut self, cx: &mut Context<Self>) {
    let Some(editor) = self.editor.clone() else {
      return;
    };

    editor.update(cx, |editor, cx| editor.reveal_first_conflict(cx));
    self.pending_conflict_reveal_path = None;
  }

  fn open_file_revealing_first_conflict(&mut self, rel_path: PathBuf, cx: &mut Context<Self>) {
    self.open_file_internal(rel_path, true, SelectedFileSource::StatusEntry, cx);
  }

  fn open_file(&mut self, rel_path: PathBuf, cx: &mut Context<Self>) {
    let selected_file_source = self.selected_file_source_for_open_path(&rel_path);
    self.open_file_internal(rel_path, false, selected_file_source, cx);
  }

  fn open_status_file(&mut self, rel_path: PathBuf, cx: &mut Context<Self>) {
    self.open_file_internal(rel_path, false, SelectedFileSource::StatusEntry, cx);
  }

  fn install_agent_review_handlers_for_editor(
    &mut self,
    editor: &Entity<Editor>,
    cx: &mut Context<Self>,
  ) {
    let view = cx.entity().downgrade();
    editor.update(cx, |editor, cx| {
      let create_handler: ReviewCommentCreateHandler = Arc::new({
        let view = view.clone();
        move |request, window, _cx| {
          let view = view.clone();
          window.on_next_frame(move |window, cx| {
            let _ = view.update(cx, |this, cx| {
              this.create_agent_review_comment(request, window, cx);
            });
          });
        }
      });
      editor.set_review_comment_create_handler(Some(create_handler), cx);
      editor.set_review_comment_replies_enabled(false, cx);
      editor.set_review_comment_display_mode(ReviewCommentDisplayMode::LocalNote, cx);

      let edit_handler: ReviewCommentEditHandler = Arc::new({
        let view = view.clone();
        move |comment_id, body, window, _cx| {
          let view = view.clone();
          window.on_next_frame(move |window, cx| {
            let _ = view.update(cx, |this, cx| {
              this.update_agent_review_comment(comment_id, body, window, cx);
            });
          });
        }
      });
      editor.set_review_comment_edit_handler(Some(edit_handler), cx);

      let delete_handler: ReviewCommentDeleteHandler = Arc::new({
        let view = view.clone();
        move |comment_id, window, _cx| {
          let view = view.clone();
          window.on_next_frame(move |_window, cx| {
            let _ = view.update(cx, |this, cx| {
              this.delete_agent_review_comment(comment_id, cx);
            });
          });
        }
      });
      editor.set_review_comment_delete_handler(Some(delete_handler), cx);

      let cancel_handler: ReviewCommentCancelHandler = Arc::new({
        let view = view.clone();
        move |window, _cx| {
          let view = view.clone();
          window.on_next_frame(move |window, cx| {
            let _ = view.update(cx, |this, cx| {
              if this.sidebar_mode == GitSidebarMode::Changes {
                this.focus_changes_sidebar_list(window, cx);
              }
            });
          });
        }
      });
      editor.set_review_comment_cancel_handler(Some(cancel_handler), cx);

      let preview_renderer: editor::ReviewCommentPreviewRenderer = Arc::new(
        |text: &str,
         suggestion_context: Option<SuggestionContext>,
         _window: &mut Window,
         cx: &mut App|
         -> AnyElement {
          let mut options = MarkdownRenderOptions::default();
          if let Some(ctx) = suggestion_context {
            options = options.with_suggestion_context(ctx);
          }
          render_markdown(text, &options, cx)
        },
      );
      editor.set_review_comment_preview_renderer(Some(preview_renderer), cx);
    });
  }

  fn agent_review_original_lines_for_request(
    &self,
    request: &ReviewCommentCreateRequest,
    cx: &App,
  ) -> (Option<usize>, Vec<String>) {
    if request.side != ReviewCommentSide::Right {
      return (None, Vec::new());
    }

    let Some(editor) = self.editor.as_ref() else {
      return (None, Vec::new());
    };

    let start = request.start_line.unwrap_or(request.line).min(request.line);
    let end = request.start_line.unwrap_or(request.line).max(request.line);
    let document = editor.read(cx).document().clone();
    let document = document.read(cx);
    let mut lines = Vec::new();

    for line_ix in start..=end {
      let Some(line) = document.line_content(line_ix) else {
        continue;
      };
      lines.push(
        line
          .trim_end_matches(|ch| ch == '\r' || ch == '\n')
          .to_string(),
      );
    }

    if lines.is_empty() {
      (None, Vec::new())
    } else {
      (Some(start.saturating_add(1)), lines)
    }
  }

  fn root_agent_review_comment_id(&self, comment_id: u64) -> u64 {
    let mut root_id = comment_id;
    let mut current_id = Some(comment_id);
    for _ in 0..32 {
      let Some(id) = current_id else {
        break;
      };
      let Some(comment) = self
        .agent_review_comments
        .iter()
        .find(|comment| comment.id == id)
      else {
        break;
      };
      let Some(parent_id) = comment.in_reply_to_id else {
        root_id = comment.id;
        break;
      };
      root_id = parent_id;
      current_id = Some(parent_id);
    }
    root_id
  }

  fn create_agent_review_comment(
    &mut self,
    request: ReviewCommentCreateRequest,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let parent = request.in_reply_to_id.and_then(|parent_id| {
      self
        .agent_review_comments
        .iter()
        .find(|comment| comment.id == parent_id)
        .cloned()
    });
    let Some(path) = parent
      .as_ref()
      .map(|comment| comment.path.clone())
      .or_else(|| self.selected_file.clone())
    else {
      self.finish_agent_review_create(Some(Arc::from("No selected file")), cx);
      return;
    };

    let (original_start_line, original_lines) = if request.in_reply_to_id.is_some() {
      (None, Vec::new())
    } else {
      self.agent_review_original_lines_for_request(&request, cx)
    };

    let id = self.next_agent_review_comment_id;
    self.next_agent_review_comment_id = self.next_agent_review_comment_id.saturating_add(1);
    self.agent_review_comments.push(LocalAgentReviewComment {
      id,
      in_reply_to_id: request.in_reply_to_id,
      path,
      line: request.line,
      side: request.side,
      start_line: request.start_line,
      start_side: request.start_side,
      body: request.body.clone(),
      original_start_line,
      original_lines,
      state: LocalAgentReviewCommentState::Draft,
    });
    self.sync_agent_review_comments_to_editor(cx);
    self.finish_agent_review_create(None, cx);
    if self.sidebar_mode == GitSidebarMode::Changes {
      self.focus_changes_sidebar_list(window, cx);
    }
    cx.notify();
  }

  fn finish_agent_review_create(&mut self, error: Option<Arc<str>>, cx: &mut Context<Self>) {
    if let Some(editor) = self.editor.clone() {
      editor.update(cx, |editor, cx| {
        editor.finish_review_comment_create_submission(error, cx);
      });
    }
  }

  fn update_agent_review_comment(
    &mut self,
    comment_id: u64,
    body: Arc<str>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if let Some(comment) = self
      .agent_review_comments
      .iter_mut()
      .find(|comment| comment.id == comment_id)
    {
      comment.body = body;
      self.sync_agent_review_comments_to_editor(cx);
      if let Some(editor) = self.editor.clone() {
        editor.update(cx, |editor, cx| {
          editor.finish_review_comment_edit_submission(comment_id, None, cx);
        });
      }
      if self.sidebar_mode == GitSidebarMode::Changes {
        self.focus_changes_sidebar_list(window, cx);
      }
      cx.notify();
    }
  }

  fn delete_agent_review_comment(&mut self, comment_id: u64, cx: &mut Context<Self>) {
    if let Some(editor) = self.editor.clone() {
      editor.update(cx, |editor, cx| {
        editor.start_review_comment_delete_submission(comment_id, cx);
      });
    }

    let removed_root = self.root_agent_review_comment_id(comment_id);
    let removed_ids = self
      .agent_review_comments
      .iter()
      .filter(|comment| {
        comment.id == comment_id
          || comment.in_reply_to_id == Some(comment_id)
          || (comment_id == removed_root
            && self.root_agent_review_comment_id(comment.id) == removed_root)
      })
      .map(|comment| comment.id)
      .collect::<HashSet<_>>();
    self
      .agent_review_comments
      .retain(|comment| !removed_ids.contains(&comment.id));
    self.sync_agent_review_comments_to_editor(cx);
    if let Some(editor) = self.editor.clone() {
      editor.update(cx, |editor, cx| {
        editor.finish_review_comment_delete_submission(comment_id, cx);
      });
    }
    cx.notify();
  }

  fn local_agent_review_comment_to_editor_comment(
    &self,
    comment: &LocalAgentReviewComment,
  ) -> ReviewComment {
    let line_label = Some(Arc::<str>::from(agent_review_line_label(comment)));
    let suggestion_context = if comment.original_lines.is_empty() {
      None
    } else {
      Some(SuggestionContext {
        original_start_line: comment.original_start_line,
        suggested_start_line: comment.original_start_line,
        original_lines: comment.original_lines.clone(),
        path: Arc::from(comment.path.to_string_lossy().as_ref()),
      })
    };

    ReviewComment {
      id: comment.id,
      in_reply_to_id: comment.in_reply_to_id,
      line: comment.line,
      side: comment.side,
      author: Arc::from(""),
      avatar_url: None,
      line_label,
      body: comment.body.clone(),
      suggestion_context,
      created_at: Arc::from(""),
      thread_id: None,
      is_resolved: false,
      is_outdated: matches!(comment.state, LocalAgentReviewCommentState::Outdated),
      viewer_can_resolve: false,
      viewer_can_unresolve: false,
    }
  }

  fn sync_agent_review_comments_to_editor(&mut self, cx: &mut Context<Self>) {
    let Some(editor) = self.editor.clone() else {
      return;
    };
    let Some(selected_file) = self.selected_file.clone() else {
      editor.update(cx, |editor, cx| {
        editor.set_review_comments(Vec::new(), cx);
        editor.set_editable_review_comment_ids(std::iter::empty::<u64>(), cx);
      });
      return;
    };

    self.refresh_agent_review_comment_states_for_selected_file(cx);

    let comments = self
      .agent_review_comments
      .iter()
      .filter(|comment| comment.path == selected_file)
      .filter(|comment| {
        matches!(
          comment.state,
          LocalAgentReviewCommentState::Draft | LocalAgentReviewCommentState::Copied
        )
      })
      .map(|comment| self.local_agent_review_comment_to_editor_comment(comment))
      .collect::<Vec<_>>();
    let editable_ids = comments
      .iter()
      .map(|comment| comment.id)
      .collect::<Vec<_>>();

    editor.update(cx, |editor, cx| {
      editor.set_editable_review_comment_ids(editable_ids, cx);
      editor.set_review_comments(comments, cx);
    });
  }

  fn refresh_agent_review_comment_states_for_selected_file(&mut self, cx: &App) -> bool {
    let Some(editor) = self.editor.clone() else {
      return false;
    };
    let Some(selected_file) = self.selected_file.clone() else {
      return false;
    };

    let current_file_lines = {
      let document = editor.read(cx).document().clone();
      let document = document.read(cx);
      (0..document.len_lines())
        .filter_map(|line_ix| {
          document.line_content(line_ix).map(|line| {
            line
              .trim_end_matches(|ch| ch == '\r' || ch == '\n')
              .to_string()
          })
        })
        .collect::<Vec<_>>()
    };

    let mut changed = false;
    for comment in &mut self.agent_review_comments {
      if comment.path != selected_file {
        continue;
      }
      let next_state = next_agent_review_comment_state(comment, &current_file_lines);
      if comment.state != next_state {
        comment.state = next_state;
        changed = true;
      }
    }

    changed
  }

  fn send_agent_review_to_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self.sync_agent_review_comments_to_editor(cx);
    let copyable_ids = self
      .agent_review_comments
      .iter()
      .filter(|comment| agent_review_comment_is_copyable(comment))
      .map(|comment| comment.id)
      .collect::<HashSet<_>>();

    if copyable_ids.is_empty() {
      window.push_notification(Notification::info("No local review comments to send"), cx);
      return;
    }

    let review = format_agent_review_export(&self.agent_review_comments);
    let count = copyable_ids.len();

    self.ensure_agent_chat_view(window, cx);
    self.show_agent_sidebar = true;
    self.show_terminal_sidebar = false;

    let Some(panel) = self.agent_chat_view.clone() else {
      window.push_notification(Notification::error("Failed to open agent panel"), cx);
      return;
    };

    let dispatched = panel.update(cx, |panel, cx| {
      if !panel.is_ready() {
        return false;
      }
      panel.send_external_prompt(review, cx)
    });

    if !dispatched {
      window.push_notification(
        Notification::info("Agent not ready yet. Try again in a moment."),
        cx,
      );
      cx.notify();
      return;
    }

    for comment in &mut self.agent_review_comments {
      if copyable_ids.contains(&comment.id) {
        comment.state = LocalAgentReviewCommentState::Copied;
      }
    }
    self.sync_agent_review_comments_to_editor(cx);
    window.push_notification(
      Notification::success(format!(
        "Sent {count} review {} to {}",
        if count == 1 { "comment" } else { "comments" },
        AgentSettings::load().label(),
      )),
      cx,
    );
    cx.notify();
  }

  fn copy_agent_review_to_clipboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self.sync_agent_review_comments_to_editor(cx);
    let copyable_ids = self
      .agent_review_comments
      .iter()
      .filter(|comment| agent_review_comment_is_copyable(comment))
      .map(|comment| comment.id)
      .collect::<HashSet<_>>();

    if copyable_ids.is_empty() {
      window.push_notification(Notification::info("No local review comments to copy"), cx);
      return;
    }

    let review = format_agent_review_export(&self.agent_review_comments);
    let count = copyable_ids.len();
    cx.write_to_clipboard(ClipboardItem::new_string(review));
    for comment in &mut self.agent_review_comments {
      if copyable_ids.contains(&comment.id) {
        comment.state = LocalAgentReviewCommentState::Copied;
      }
    }
    self.sync_agent_review_comments_to_editor(cx);
    window.push_notification(
      Notification::success(format!(
        "Copied {count} local review {}",
        if count == 1 { "comment" } else { "comments" }
      )),
      cx,
    );
  }

  fn selected_file_source_for_open_path(&self, rel_path: &Path) -> SelectedFileSource {
    if self
      .status_entries
      .iter()
      .any(|entry| entry.path.as_path() == rel_path)
    {
      SelectedFileSource::StatusEntry
    } else {
      SelectedFileSource::ProjectFile
    }
  }

  fn open_file_internal(
    &mut self,
    rel_path: PathBuf,
    reveal_first_conflict: bool,
    selected_file_source: SelectedFileSource,
    cx: &mut Context<Self>,
  ) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    // Sync diff view from persisted setting
    let app_settings = crate::config::AppSettings::get(cx);
    let saved_mode = if app_settings.split_diff_view {
      DiffViewMode::Split
    } else {
      DiffViewMode::Inline
    };
    if self.diff_view != saved_mode {
      self.diff_view = saved_mode;
    }
    self.hide_whitespace = app_settings.hide_whitespace;
    self.pending_conflict_reveal_path = reveal_first_conflict.then_some(rel_path.clone());
    if self.selected_file.as_ref() == Some(&rel_path) && self.history_opened_commit_file.is_none() {
      self.selected_file_source = Some(selected_file_source);
      if reveal_first_conflict {
        self.reveal_first_conflict_in_editor(cx);
      }
      return;
    }
    let is_markdown = is_markdown_path(&rel_path);
    if !is_markdown && !is_svg_path(&rel_path) {
      self.show_markdown_preview = false;
    }

    self.invalidate_open_file_task();
    let generation = self.open_file_generation;
    let had_history_file_selection = self.history_opened_commit_file.is_some();
    self.history_opened_commit_file = None;
    self.selected_file = Some(rel_path.clone());
    self.selected_file_source = Some(selected_file_source);
    self.sync_sentry_git_context();
    let mut data = Map::new();
    data.insert(
      "file".into(),
      rel_path.to_string_lossy().replace(['\n', '\r'], "").into(),
    );
    self.add_git_breadcrumb("Opened file in git page", data);
    self.editor = None;
    self.binary_preview = None;
    self.svg_preview = None;
    self.svg_preview_source = None;
    self.force_list_selection = true;
    let opened_path = self.selected_file.clone();
    self.file_list.update(cx, |state, cx| {
      state.delegate_mut().set_opened_path(opened_path);
      cx.notify();
    });
    let selected_index = self.selected_file_index(cx);
    self.set_file_list_selected_index(selected_index, cx);
    if had_history_file_selection {
      self.refresh_history_list(cx);
    }

    let diff_view = self.effective_diff_view_for_path(&rel_path);
    let requested_repo = repo_root.clone();
    let requested_path = rel_path.clone();
    let file_path = requested_repo.join(&requested_path);
    let load_repo_root = requested_repo.clone();
    let load_file_path = file_path.clone();
    let task = cx.spawn(async move |this, cx| {
      let loaded =
        unblock(move || Editor::load_file_for_editor(&load_repo_root, &load_file_path)).await;
      let _ = this.update(cx, move |this, cx| {
        if this.open_file_generation != generation {
          return;
        }
        if this.selected_repo.as_ref() != Some(&requested_repo) {
          return;
        }
        if this.selected_file.as_ref() != Some(&requested_path) {
          return;
        }
        if this.history_opened_commit_file.is_some() {
          return;
        }

        let editor_repo_root = requested_repo.clone();
        let editor_file_path = file_path.clone();
        let binary_preview =
          Self::build_binary_preview(requested_path.as_path(), loaded.binary_bytes.clone());
        let should_reveal_first_conflict =
          this.pending_conflict_reveal_path.as_deref() == Some(requested_path.as_path());
        let editor = cx.new(move |cx| {
          Editor::new_with_loaded_file(editor_repo_root, editor_file_path, loaded, cx)
        });
        let hide_ws = this.hide_whitespace;
        let is_unmerged = this
          .selected_file_entry()
          .is_some_and(|entry| entry.status == RepoStatusKind::Conflicted);
        editor.update(cx, |editor, cx| {
          editor.set_diff_view_mode(diff_view, cx);
          editor.set_ignore_whitespace(hide_ws, cx);
          editor.set_is_unmerged(is_unmerged, cx);
          if should_reveal_first_conflict {
            editor.reveal_first_conflict(cx);
          }
        });
        this.binary_preview = binary_preview;
        this.editor = Some(editor.clone());
        this.install_agent_review_handlers_for_editor(&editor, cx);
        this.sync_agent_review_comments_to_editor(cx);
        if should_reveal_first_conflict {
          this.pending_conflict_reveal_path = None;
        }
        cx.notify();
      });
    });
    self.open_file_task = Some(task);
    cx.notify();
  }

  fn queue_history_commit_files_load(
    &mut self,
    commit_oid: String,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.history_commit_files.contains_key(commit_oid.as_str())
      || self
        .history_commit_files_loading
        .contains(commit_oid.as_str())
      || self
        .pending_history_file_loads
        .contains(commit_oid.as_str())
    {
      return;
    }

    self.pending_history_file_loads.insert(commit_oid.clone());
    cx.on_next_frame(window, move |this, _, cx| {
      this.pending_history_file_loads.remove(commit_oid.as_str());
      this.load_history_commit_files(commit_oid.clone(), cx);
    });
  }

  fn load_history_commit_files(&mut self, commit_oid: String, cx: &mut Context<Self>) {
    self.pending_history_file_loads.remove(commit_oid.as_str());
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    self.history_commit_files_loading.insert(commit_oid.clone());
    self.refresh_history_list(cx);
    cx.notify();

    let task = cx.spawn(async move |this, cx| {
      let load_repo_root = repo_root.clone();
      let load_commit_oid = commit_oid.clone();
      let files =
        unblock(move || list_commit_changed_files(&load_repo_root, &load_commit_oid)).await;
      let _ = this.update(cx, |this, cx| {
        if this.selected_repo.as_ref() != Some(&repo_root) {
          return;
        }
        this
          .history_commit_files_loading
          .remove(commit_oid.as_str());
        if let Ok(files) = files {
          let rows = files
            .into_iter()
            .map(HistoryCommitFileRow::from_commit_file)
            .collect::<Vec<_>>();
          this.history_commit_files.insert(commit_oid.clone(), rows);
        } else {
          this.history_commit_files.remove(commit_oid.as_str());
        }
        this.refresh_history_list(cx);
        cx.notify();
      });
    });

    self.history_files_task = Some(task);
  }

  fn open_history_commit_file(
    &mut self,
    commit_oid: String,
    rel_path: PathBuf,
    cx: &mut Context<Self>,
  ) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    self.invalidate_open_file_task();
    self.history_opened_commit_file = Some((commit_oid.clone(), rel_path.clone()));
    self.selected_file = Some(rel_path.clone());
    self.selected_file_source = None;
    self.sync_sentry_git_context();
    let mut data = Map::new();
    data.insert(
      "file".into(),
      rel_path.to_string_lossy().replace(['\n', '\r'], "").into(),
    );
    data.insert("history_commit".into(), commit_oid.clone().into());
    self.add_git_breadcrumb("Opened history file in git page", data);
    self.refresh_history_list(cx);
    cx.notify();

    let task = cx.spawn(async move |this, cx| {
      let load_repo_root = repo_root.clone();
      let load_commit_oid = commit_oid.clone();
      let load_rel_path = rel_path.clone();
      let commit_file =
        unblock(move || load_commit_file_diff(&load_repo_root, &load_commit_oid, &load_rel_path))
          .await;
      let _ = this.update(cx, |this, cx| {
        if this.selected_repo.as_ref() != Some(&repo_root) {
          return;
        }
        let Ok(commit_file) = commit_file else {
          return;
        };

        let file_path = repo_root.join(&rel_path);
        let editor = cx.new(|cx| Editor::new_with_paths(repo_root.clone(), file_path, cx));
        let diff_set = if commit_file.patch.trim().is_empty() {
          None
        } else {
          diff_set_from_patch(&commit_file.patch).ok()
        };
        let diff_view = this.effective_diff_view_for_path(&rel_path);

        let hide_ws = this.hide_whitespace;
        editor.update(cx, |editor, cx| {
          editor.load_readonly_snapshot(commit_file.content, diff_set, cx);
          editor.set_diff_view_mode(diff_view, cx);
          editor.set_ignore_whitespace(hide_ws, cx);
        });

        this.clear_markdown_preview_if_not_previewable(&rel_path);
        this.binary_preview =
          Self::build_binary_preview(rel_path.as_path(), commit_file.binary_bytes.clone());
        this.editor = Some(editor);
        this.selected_file = Some(rel_path.clone());
        this.selected_file_source = None;
        this.history_opened_commit_file = Some((commit_oid.clone(), rel_path.clone()));
        this.sync_sentry_git_context();
        this.svg_preview = None;
        this.svg_preview_source = None;
        this.refresh_history_list(cx);
        cx.notify();
      });
    });

    self.history_open_file_task = Some(task);
  }

  fn clear_markdown_preview_if_not_previewable(&mut self, rel_path: &Path) {
    if !is_previewable_path(rel_path) {
      self.show_markdown_preview = false;
    }
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
    crate::config::AppSettings::update(cx, |s| {
      s.split_diff_view = self.diff_view == DiffViewMode::Split
    });
    self.sync_diff_view(cx);
    self.sync_sentry_git_context();
    let mut data = Map::new();
    data.insert("diff_view".into(), self.active_diff_view_tag().into());
    self.add_git_breadcrumb("Toggled git diff view", data);
    cx.notify();
  }

  fn toggle_hide_whitespace(&mut self, cx: &mut Context<Self>) {
    self.hide_whitespace = !self.hide_whitespace;
    if let Some(editor) = self.editor.as_ref() {
      let value = self.hide_whitespace;
      editor.update(cx, |editor, cx| {
        editor.set_ignore_whitespace(value, cx);
      });
    }
    cx.notify();
  }

  fn toggle_markdown_preview(&mut self, cx: &mut Context<Self>) {
    if !self.selected_file_is_markdown() && !self.selected_file_is_svg() {
      self.show_markdown_preview = false;
      self.sync_diff_view(cx);
      self.sync_sentry_git_context();
      cx.notify();
      return;
    }

    self.show_markdown_preview = !self.show_markdown_preview;
    self.sync_diff_view(cx);
    self.sync_sentry_git_context();
    let mut data = Map::new();
    data.insert("enabled".into(), self.show_markdown_preview.into());
    self.add_git_breadcrumb("Toggled markdown preview", data);
    cx.notify();
  }

  fn update_svg_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if !self.show_markdown_preview || !self.selected_file_is_svg() {
      return;
    }

    let Some(editor) = self.editor.clone() else {
      return;
    };

    let document = editor.read(cx).document().read(cx);
    let svg_source = document.slice_to_string(0..document.len());
    let svg_source: SharedString = svg_source.into();

    if self.svg_preview_source.as_ref() == Some(&svg_source) {
      return;
    }

    self.svg_preview_source = Some(svg_source.clone());
    let renderer = cx.svg_renderer();
    let svg_bytes = svg_source.as_ref().as_bytes().to_vec();
    let background =
      cx.background_spawn(async move { renderer.render_single_frame(svg_bytes.as_slice(), 1.0) });

    let task = cx.spawn_in(window, async move |this, cx| {
      let result = background.await;
      let _ = this.update_in(cx, |this, window, cx| {
        if let Some(Ok(image)) = this.svg_preview.take() {
          let _ = window.drop_image(image);
        }
        this.svg_preview = Some(result.map_err(|err| err.to_string().into()));
        cx.notify();
      });
    });

    self.svg_preview_task = Some(task);
  }

  fn toggle_sidebar_mode_action(
    &mut self,
    _: &gpui::ClickEvent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let next_mode = match self.sidebar_mode {
      GitSidebarMode::Changes => GitSidebarMode::History,
      GitSidebarMode::History => GitSidebarMode::Changes,
    };
    self.set_sidebar_mode(next_mode, window, cx);
  }

  fn focus_changes_sidebar_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.file_list.read(cx).selected_index().is_none() && !self.status_entries.is_empty() {
      self.file_list.update(cx, |state, cx| {
        state.set_selected_index(Some(IndexPath::new(0)), window, cx);
      });
    }

    self.file_list.update(cx, |state, cx| {
      state.focus(window, cx);
    });
  }

  fn focus_history_sidebar_tree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.history_tree.read(cx).selected_index().is_none()
      && let Some(first_row) = self.history_rows_cache.first()
    {
      let first_id = format!("history-commit:{}", first_row.commit.oid);
      self.history_tree.update(cx, |state, cx| {
        let item = TreeItem::new(first_id.clone(), first_id.clone());
        state.set_selected_item(Some(&item), cx);
        if let Some(ix) = state.selected_index() {
          state.scroll_to_item(ix, gpui::ScrollStrategy::Top);
        }
      });
    }

    self.history_tree.update(cx, |state, cx| {
      if let Some(ix) = state.selected_index() {
        state.scroll_to_item(ix, gpui::ScrollStrategy::Top);
      }
      state.focus(window, cx);
    });
  }

  fn focus_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    window.focus(&self.focus_handle, cx);
  }

  fn focus_editor_or_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if let Some(editor) = self.editor.clone() {
      let editor_focus_handle = editor.read(cx).focus_handle(cx);
      window.focus(&editor_focus_handle, cx);
      return;
    }

    self.focus_page(window, cx);
  }

  fn focus_terminal_sidebar_on_next_frame(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
    let terminal_view = self.terminal_view.clone();
    window.on_next_frame(move |window, cx| {
      let focus_handle = terminal_view.read(cx).focus_handle(cx);
      window.focus(&focus_handle, cx);
    });
  }

  fn toggle_terminal_sidebar_visibility(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if !AuthStateStore::is_admin(cx) {
      if self.show_terminal_sidebar {
        self.show_terminal_sidebar = false;
        self.focus_editor_or_page(window, cx);
        cx.notify();
      }
      return;
    }

    self.show_terminal_sidebar = !self.show_terminal_sidebar;
    if self.show_terminal_sidebar {
      self.show_agent_sidebar = false;
      self.focus_terminal_sidebar_on_next_frame(window, cx);
    } else {
      self.focus_editor_or_page(window, cx);
    }
    cx.notify();
  }

  fn toggle_terminal_sidebar_click(
    &mut self,
    _: &gpui::ClickEvent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.toggle_terminal_sidebar_visibility(window, cx);
  }

  fn toggle_terminal_sidebar_action(
    &mut self,
    _: &crate::ToggleTerminalSidebar,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.toggle_terminal_sidebar_visibility(window, cx);
    cx.stop_propagation();
  }

  fn ensure_agent_chat_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if let Some(view) = self.agent_chat_view.as_ref()
      && view.read(cx).needs_reconnect()
    {
      self.agent_chat_view = None;
    }
    if self.agent_chat_view.is_some() {
      return;
    }
    prune_agent_chat_state_once();
    let cwd = self
      .selected_repo
      .clone()
      .unwrap_or_else(|| std::path::PathBuf::from("."));
    let state_dir =
      agent_chat_state_dir().map(|dir| AgentChatPanel::state_dir_for_repo(&dir, &cwd));
    let backend = AgentSettings::load();
    let view = cx.new(|cx| AgentChatPanel::new(backend, cwd, state_dir, window, cx));
    self.agent_chat_view = Some(view);
  }

  fn toggle_agent_sidebar_visibility(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.show_agent_sidebar {
      self.show_agent_sidebar = false;
      self.focus_editor_or_page(window, cx);
      cx.notify();
      return;
    }
    self.ensure_agent_chat_view(window, cx);
    self.show_agent_sidebar = true;
    self.show_terminal_sidebar = false;
    self.focus_agent_sidebar_on_next_frame(window, cx);
    cx.notify();
  }

  fn focus_agent_sidebar_on_next_frame(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
    let Some(view) = self.agent_chat_view.clone() else {
      return;
    };
    window.on_next_frame(move |window, cx| {
      let focus_handle = view.read(cx).input_focus_handle(cx);
      window.focus(&focus_handle, cx);
    });
  }

  fn toggle_agent_sidebar_click(
    &mut self,
    _: &gpui::ClickEvent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.toggle_agent_sidebar_visibility(window, cx);
  }

  fn toggle_agent_sidebar_action(
    &mut self,
    _: &crate::ToggleAgentSidebar,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.toggle_agent_sidebar_visibility(window, cx);
    cx.stop_propagation();
  }

  fn ensure_page_shortcut_focus(&self, cx: &mut Context<Self>) {
    let focus_handle = self.focus_handle.clone();
    let window_handle = self.window_handle;
    let _ = cx.update_window(window_handle, move |_, window, cx| {
      window.focus(&focus_handle, cx);
    });
  }

  fn focus_sidebar_on_next_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.sidebar_mode == GitSidebarMode::Changes {
      cx.on_next_frame(window, |this, window, cx| {
        if this.sidebar_mode == GitSidebarMode::Changes {
          this.focus_changes_sidebar_list(window, cx);
        }
      });
      return;
    }

    self.focus_history_sidebar_tree(window, cx);
    cx.on_next_frame(window, |this, window, cx| {
      if this.sidebar_mode == GitSidebarMode::History {
        this.focus_history_sidebar_tree(window, cx);
      }
    });
  }

  fn set_sidebar_mode(
    &mut self,
    mode: GitSidebarMode,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.sidebar_mode = mode;
    self.sync_sentry_git_context();
    let mut data = Map::new();
    data.insert(
      "sidebar_mode".into(),
      Self::sidebar_mode_tag(self.sidebar_mode).into(),
    );
    self.add_git_breadcrumb("Changed git sidebar mode", data);

    if self.sidebar_mode == GitSidebarMode::History {
      self.refresh_history(cx);
    } else {
      self.refresh_file_list(cx);
      cx.notify();
    }

    self.focus_sidebar_on_next_frame(window, cx);
  }

  fn toggle_stage_all_action(
    &mut self,
    _: &gpui::ClickEvent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.all_changes_staged() {
      self.unstage_all_action(cx);
    } else if Self::should_confirm_stage_all(self.selected_repo.as_ref(), &self.status_entries) {
      self.confirm_stage_all_conflicted_action(window, cx);
    } else {
      self.stage_all_action(cx);
    }
  }

  fn restore_all_click_action(
    &mut self,
    _: &gpui::ClickEvent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.confirm_restore_all_action(window, cx);
  }

  fn abort_merge_action(&mut self, _: &gpui::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    if !self.merge_in_progress {
      return;
    }

    let editor = self.editor.clone();
    let commit_input = self.commit_input.clone();
    let window_handle = self.window_handle;
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || abort_merge(&repo_root)).await;
      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(()) => {
            let _ = cx.update_window(window_handle, |_, window, cx| {
              commit_input.update(cx, |input, cx| input.set_value("", window, cx));
            });
          }
          Err(error) => {
            this.push_git_action_error_notification(
              "Abort merge failed",
              error.to_string().into(),
              cx,
            );
          }
        }
        this.reload_status(cx);
        if let Some(editor) = editor.clone() {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
      });
    });
    self.status_task = Some(task);
  }

  fn abort_rebase_action(&mut self, _: &gpui::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    if !self.rebase_in_progress {
      return;
    }

    let editor = self.editor.clone();
    let commit_input = self.commit_input.clone();
    let window_handle = self.window_handle;
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || abort_rebase(&repo_root)).await;
      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(()) => {
            this.force_push_after_rebase = false;
            let _ = cx.update_window(window_handle, |_, window, cx| {
              commit_input.update(cx, |input, cx| input.set_value("", window, cx));
            });
          }
          Err(error) => {
            this.push_git_action_error_notification(
              "Abort rebase failed",
              error.to_string().into(),
              cx,
            );
          }
        }
        this.reload_status(cx);
        if let Some(editor) = editor.clone() {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
      });
    });
    self.status_task = Some(task);
  }

  fn continue_rebase_action(
    &mut self,
    _: &gpui::ClickEvent,
    _: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.continue_rebase_inner(cx);
  }

  fn stage_all_action(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    self.add_git_breadcrumb("Stage all started", Map::new());
    let editor = self.editor.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || stage_all(&repo_root)).await;
      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(()) => this.add_git_breadcrumb("Stage all succeeded", Map::new()),
          Err(error) => {
            let error_message = error.to_string();
            let mut data = Map::new();
            data.insert("error".into(), error_message.clone().into());
            this.add_git_breadcrumb("Stage all failed", data.clone());
            this.record_git_unexpected_error("git.stage_all", error_message.as_str(), data);
            this.push_git_action_error_notification("Stage all failed", error_message.into(), cx);
          }
        }
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
    self.add_git_breadcrumb("Unstage all started", Map::new());
    let editor = self.editor.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || unstage_all(&repo_root)).await;
      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(()) => this.add_git_breadcrumb("Unstage all succeeded", Map::new()),
          Err(error) => {
            let error_message = error.to_string();
            let mut data = Map::new();
            data.insert("error".into(), error_message.clone().into());
            this.add_git_breadcrumb("Unstage all failed", data.clone());
            this.record_git_unexpected_error("git.unstage_all", error_message.as_str(), data);
            this.push_git_action_error_notification("Unstage all failed", error_message.into(), cx);
          }
        }
        this.reload_status(cx);
        if let Some(editor) = editor.clone() {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
      });
    });
    self.status_task = Some(task);
  }

  fn stage_file_click_action(
    &mut self,
    window: &mut Window,
    rel_path: PathBuf,
    status: RepoStatusKind,
    cx: &mut Context<Self>,
  ) {
    if Self::should_confirm_stage_for_status(
      Some(status),
      self.open_editor_has_unresolved_conflict_markers(cx),
    ) {
      self.confirm_stage_conflicted_file_action(window, rel_path, cx);
    } else {
      self.stage_file_action(rel_path, cx);
    }
  }

  fn stage_file_action(&mut self, rel_path: PathBuf, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    let rel_path_label = rel_path.to_string_lossy().replace(['\n', '\r'], "");
    let mut start_data = Map::new();
    start_data.insert("file".into(), rel_path_label.clone().into());
    self.add_git_breadcrumb("Stage file started", start_data);
    let rel_path_for_job = rel_path.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || stage_file(&repo_root, &rel_path_for_job)).await;
      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(()) => this.add_git_breadcrumb("Stage file succeeded", Map::new()),
          Err(error) => {
            let error_message = error.to_string();
            let mut data = Map::new();
            data.insert("error".into(), error_message.clone().into());
            data.insert("file".into(), rel_path_label.clone().into());
            this.add_git_breadcrumb("Stage file failed", data.clone());
            this.record_git_unexpected_error("git.stage_file", error_message.as_str(), data);
            this.push_git_action_error_notification(
              format!("Failed to stage {rel_path_label}"),
              error_message.into(),
              cx,
            );
          }
        }
        this.reload_status(cx);
        if Self::should_refresh_editor_for_path(this.selected_file.as_deref(), &rel_path)
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
    let rel_path_label = rel_path.to_string_lossy().replace(['\n', '\r'], "");
    let mut start_data = Map::new();
    start_data.insert("file".into(), rel_path_label.clone().into());
    self.add_git_breadcrumb("Unstage file started", start_data);
    let rel_path_for_job = rel_path.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || unstage_file(&repo_root, &rel_path_for_job)).await;
      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(()) => this.add_git_breadcrumb("Unstage file succeeded", Map::new()),
          Err(error) => {
            let error_message = error.to_string();
            let mut data = Map::new();
            data.insert("error".into(), error_message.clone().into());
            data.insert("file".into(), rel_path_label.clone().into());
            this.add_git_breadcrumb("Unstage file failed", data.clone());
            this.record_git_unexpected_error("git.unstage_file", error_message.as_str(), data);
            this.push_git_action_error_notification(
              format!("Failed to unstage {rel_path_label}"),
              error_message.into(),
              cx,
            );
          }
        }
        this.reload_status(cx);
        if Self::should_refresh_editor_for_path(this.selected_file.as_deref(), &rel_path)
          && let Some(editor) = this.editor.clone()
        {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
      });
    });
    self.status_task = Some(task);
  }

  fn restore_file_click_action(
    &mut self,
    window: &mut Window,
    rel_path: PathBuf,
    old_path: Option<PathBuf>,
    status: RepoStatusKind,
    cx: &mut Context<Self>,
  ) {
    self.confirm_restore_file_action(window, rel_path, old_path, status, cx);
  }

  fn restore_file_action(
    &mut self,
    rel_path: PathBuf,
    old_path: Option<PathBuf>,
    status: RepoStatusKind,
    cx: &mut Context<Self>,
  ) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    let rel_path_for_job = rel_path.clone();
    let old_path_for_job = old_path.clone();
    let should_delete = Self::restore_uses_delete(status);
    let is_rename_restore = status == RepoStatusKind::Renamed && old_path.is_some();
    let rel_path_label = rel_path.to_string_lossy().replace(['\n', '\r'], "");
    let mut start_data = Map::new();
    start_data.insert("file".into(), rel_path_label.clone().into());
    start_data.insert("delete".into(), should_delete.into());
    self.add_git_breadcrumb("Restore file started", start_data);
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || {
        if should_delete {
          delete_untracked_file(&repo_root, &rel_path_for_job)
        } else if is_rename_restore {
          let old = old_path_for_job.as_deref().expect("rename has old_path");
          restore_renamed_file(&repo_root, old, &rel_path_for_job)
        } else {
          restore_file(&repo_root, &rel_path_for_job)
        }
      })
      .await;
      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(()) => {
            this.select_first_file_after_restore = true;
            let mut data = Map::new();
            data.insert("delete".into(), should_delete.into());
            this.add_git_breadcrumb("Restore file succeeded", data);
          }
          Err(error) => {
            let error_message = error.to_string();
            let mut data = Map::new();
            data.insert("delete".into(), should_delete.into());
            data.insert("file".into(), rel_path_label.clone().into());
            data.insert("error".into(), error_message.clone().into());
            this.add_git_breadcrumb("Restore file failed", data.clone());
            this.record_git_unexpected_error("git.restore_file", error_message.as_str(), data);
            this.push_git_action_error_notification(
              format!("Failed to restore {rel_path_label}"),
              error_message.into(),
              cx,
            );
          }
        }
        this.reload_status(cx);
        if Self::should_refresh_editor_for_path(this.selected_file.as_deref(), &rel_path)
          && let Some(editor) = this.editor.clone()
        {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
      });
    });
    self.status_task = Some(task);
  }

  fn restore_all_action(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      return;
    };
    if self.status_entries.is_empty() {
      return;
    }
    self.add_git_breadcrumb("Restore all started", Map::new());
    let entries = self.status_entries.clone();
    let editor = self.editor.clone();
    let task = cx.spawn(async move |this, cx| {
      let first_error = unblock(move || {
        let mut first_error = None;
        for entry in entries {
          let result = if Self::restore_uses_delete(entry.status) {
            delete_untracked_file(&repo_root, &entry.path)
          } else if entry.status == RepoStatusKind::Renamed
            && let Some(old_path) = entry.old_path.as_deref()
          {
            restore_renamed_file(&repo_root, old_path, &entry.path)
          } else {
            restore_file(&repo_root, &entry.path)
          };
          if let Err(error) = result
            && first_error.is_none()
          {
            first_error = Some(error.to_string());
          }
        }
        first_error
      })
      .await;
      let _ = this.update(cx, |this, cx| {
        if let Some(error_message) = first_error {
          let mut data = Map::new();
          data.insert("error".into(), error_message.clone().into());
          this.add_git_breadcrumb("Restore all completed with errors", data.clone());
          this.record_git_unexpected_error("git.restore_all", error_message.as_str(), data);
          this.push_git_action_error_notification("Restore all failed", error_message.into(), cx);
        } else {
          this.add_git_breadcrumb("Restore all succeeded", Map::new());
        }
        this.select_first_file_after_restore = true;
        this.reload_status(cx);
        if let Some(editor) = editor.clone() {
          editor.update(cx, |editor, cx| editor.refresh_git_state(cx));
        }
      });
    });
    self.status_task = Some(task);
  }

  fn confirm_stage_conflicted_file_action(
    &mut self,
    window: &mut Window,
    rel_path: PathBuf,
    cx: &mut Context<Self>,
  ) {
    let file_label = rel_path.to_string_lossy().replace(['\n', '\r'], "");
    let title: SharedString = "Mark conflicts as resolved?".into();
    let message: SharedString = format!(
      "Stage {} and mark its merge conflicts as resolved?",
      file_label
    )
    .into();
    let view = cx.entity();
    let rel_path_for_action = rel_path.clone();

    window.open_alert_dialog(cx, move |alert, _, _| {
      let view = view.clone();
      let rel_path_for_action = rel_path_for_action.clone();
      ConfirmDialog::new(title.clone(), div().child(message.clone()))
        .confirm_text("Stage")
        .cancel_text("Cancel")
        .on_confirm(move |_, _, cx| {
          let rel_path_for_action = rel_path_for_action.clone();
          view.update(cx, |view, cx| {
            view.stage_file_action(rel_path_for_action, cx);
          });
          true
        })
        .build(alert)
    });
  }

  fn confirm_stage_all_conflicted_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let title: SharedString = "Mark conflicts as resolved?".into();
    let message: SharedString = "Stage all files and mark merge conflicts as resolved?".into();
    let view = cx.entity();

    window.open_alert_dialog(cx, move |alert, _, _| {
      let view = view.clone();
      ConfirmDialog::new(title.clone(), div().child(message.clone()))
        .confirm_text("Stage all")
        .cancel_text("Cancel")
        .on_confirm(move |_, _, cx| {
          view.update(cx, |view, cx| {
            view.stage_all_action(cx);
          });
          true
        })
        .build(alert)
    });
  }

  fn confirm_restore_file_action(
    &mut self,
    window: &mut Window,
    rel_path: PathBuf,
    old_path: Option<PathBuf>,
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
    let old_path_for_action = old_path.clone();

    window.open_alert_dialog(cx, move |alert, _, _| {
      let view = view.clone();
      let rel_path_for_action = rel_path_for_action.clone();
      let old_path_for_action = old_path_for_action.clone();
      ConfirmDialog::new(title.clone(), div().child(message.clone()))
        .confirm_text(confirm_text.clone())
        .cancel_text("Cancel")
        .destructive()
        .on_confirm(move |_, _, cx| {
          let rel_path_for_action = rel_path_for_action.clone();
          let old_path_for_action = old_path_for_action.clone();
          view.update(cx, |view, cx| {
            view.restore_file_action(rel_path_for_action, old_path_for_action, status, cx);
          });
          true
        })
        .build(alert)
    });
  }

  fn confirm_restore_all_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.selected_repo.is_none() || self.status_entries.is_empty() {
      return;
    }
    let has_untracked = Self::has_untracked_entries(&self.status_entries);
    let title: SharedString = "Restore all files?".into();
    let message: SharedString = if has_untracked {
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
        .on_confirm(move |_, _, cx| {
          view.update(cx, |view, cx| {
            view.restore_all_action(cx);
          });
          true
        })
        .build(alert)
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
      RepoStatusKind::Modified => theme.status_yellow(),
      RepoStatusKind::Added => theme.status_green(),
      RepoStatusKind::Deleted => theme.status_red(),
      RepoStatusKind::Renamed => theme.status_blue(),
      RepoStatusKind::TypeChange => theme.status_blue(),
      RepoStatusKind::Untracked => theme.status_green(),
      RepoStatusKind::Conflicted => theme.status_red(),
    }
  }

  fn status_tooltip(kind: RepoStatusKind) -> SharedString {
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

  fn can_continue_rebase_command(&self) -> bool {
    self.selected_repo.is_some()
      && self.rebase_in_progress
      && !Self::has_conflicted_entries(&self.status_entries)
  }

  fn commit_primary_action_enabled(&self, commit_message: &str) -> bool {
    if self.rebase_in_progress {
      self.can_continue_rebase_command()
    } else {
      self.selected_repo.is_some()
        && !commit_message.trim().is_empty()
        && !self.status_entries.is_empty()
        && !Self::has_conflicted_entries(&self.status_entries)
    }
  }

  fn commit_primary_button_state(
    rebase_in_progress: bool,
    has_uncommitted_changes: bool,
    can_publish_branch: bool,
  ) -> GitCommitPrimaryButtonState {
    if rebase_in_progress {
      GitCommitPrimaryButtonState::ContinueRebase
    } else if can_publish_branch && !has_uncommitted_changes {
      GitCommitPrimaryButtonState::PublishBranch
    } else {
      GitCommitPrimaryButtonState::Commit
    }
  }

  fn should_show_commit_palette_command(&self, commit_message: &str) -> bool {
    !self.rebase_in_progress && self.commit_primary_action_enabled(commit_message)
  }

  fn should_show_continue_rebase_palette_command(&self) -> bool {
    self.rebase_in_progress && self.can_continue_rebase_command()
  }

  fn should_show_skip_rebase_palette_command(&self) -> bool {
    self.rebase_in_progress && self.selected_repo.is_some()
  }

  fn should_show_push_palette_command(&self) -> bool {
    !self.rebase_in_progress && self.selected_repo.is_some() && self.can_push
  }

  fn should_show_force_push_palette_command(&self) -> bool {
    !self.rebase_in_progress && self.selected_repo.is_some() && self.can_force_push
  }

  fn should_show_undo_last_commit_palette_command(&self) -> bool {
    !self.rebase_in_progress && self.selected_repo.is_some() && self.can_undo_last_commit
  }

  fn should_show_amend_palette_command(&self) -> bool {
    !self.rebase_in_progress && self.selected_repo.is_some() && self.has_head_commit
  }

  fn should_show_checkout_detached_palette_command(&self) -> bool {
    self.selected_repo.is_some()
      && self.has_head_commit
      && !self.merge_in_progress
      && !self.rebase_in_progress
      && self.branch_status.is_some()
      && !Self::is_detached_head(self.branch_status.as_ref())
  }

  fn should_show_interactive_rebase_palette_command(&self) -> bool {
    !self.rebase_in_progress
      && !self.merge_in_progress
      && self.selected_repo.is_some()
      && self.has_head_commit
      && self.status_entries.is_empty()
      && !Self::is_detached_head(self.branch_status.as_ref())
  }

  fn continue_rebase_disabled_reason(&self) -> Option<&'static str> {
    (self.selected_repo.is_some() && self.rebase_in_progress && !self.can_continue_rebase_command())
      .then_some("Resolve and stage conflicts first")
  }

  fn operation_in_progress_disabled_reason(&self) -> Option<&'static str> {
    if self.rebase_in_progress {
      Some("Finish or abort the current rebase first")
    } else if self.merge_in_progress {
      Some("Finish or abort the current merge first")
    } else {
      None
    }
  }

  fn interactive_rebase_disabled_reason(&self) -> Option<&'static str> {
    if self.selected_repo.is_none() || self.should_show_interactive_rebase_palette_command() {
      return None;
    }
    if let Some(reason) = self.operation_in_progress_disabled_reason() {
      return Some(reason);
    }
    if !self.has_head_commit {
      return Some("Create a commit first");
    }
    if !self.status_entries.is_empty() {
      return Some("Commit or stash worktree changes first");
    }
    if Self::is_detached_head(self.branch_status.as_ref()) {
      return Some("Checkout a branch first");
    }
    None
  }

  fn should_show_pull_palette_command(&self) -> bool {
    !self.rebase_in_progress
      && !self.merge_in_progress
      && self.selected_repo.is_some()
      && self
        .branch_status
        .as_ref()
        .is_some_and(|status| status.has_upstream)
  }

  fn should_show_merge_branch_palette_command(&self) -> bool {
    !self.rebase_in_progress && !self.merge_in_progress && self.selected_repo.is_some()
  }

  fn should_show_rebase_branch_palette_command(&self) -> bool {
    !self.rebase_in_progress && !self.merge_in_progress && self.selected_repo.is_some()
  }

  fn should_show_cherry_pick_palette_command(&self) -> bool {
    !self.rebase_in_progress && !self.merge_in_progress && self.selected_repo.is_some()
  }

  fn selected_file_entry(&self) -> Option<&RepoStatusEntry> {
    let selected = self.selected_file.as_ref()?;
    self
      .status_entries
      .iter()
      .find(|entry| &entry.path == selected)
  }

  fn conflict_navigation_state_for(
    file_status: Option<RepoStatusKind>,
    editor: &Editor,
    cx: &App,
  ) -> Option<ConflictNavigationState> {
    matches!(file_status, Some(RepoStatusKind::Conflicted))
      .then(|| editor.conflict_navigation_state(cx))
      .flatten()
  }

  fn annotation_navigation_state_for(
    file_status: Option<RepoStatusKind>,
    editor: &Editor,
    cx: &App,
  ) -> Option<AnnotationNavigationState> {
    if let Some(state) = Self::conflict_navigation_state_for(file_status, editor, cx) {
      return Some(AnnotationNavigationState {
        active_index: state.active_index,
        total: state.total,
        kind: AnnotationKind::Conflict,
      });
    }
    editor
      .hunk_navigation_state(cx)
      .map(|state| AnnotationNavigationState {
        active_index: state.active_index,
        total: state.total,
        kind: AnnotationKind::Change,
      })
  }

  #[cfg(test)]
  fn editor_conflict_navigation_state(&self, cx: &App) -> Option<ConflictNavigationState> {
    let file_status = if self.history_opened_commit_file.is_some() {
      None
    } else {
      self.selected_file_entry().map(|entry| entry.status)
    };

    self.editor.as_ref().and_then(|editor| {
      editor.read_with(cx, |editor, cx| {
        Self::conflict_navigation_state_for(file_status, editor, cx)
      })
    })
  }

  #[cfg(test)]
  fn editor_annotation_navigation_state(&self, cx: &App) -> Option<AnnotationNavigationState> {
    let file_status = if self.history_opened_commit_file.is_some() {
      None
    } else {
      self.selected_file_entry().map(|entry| entry.status)
    };

    self.editor.as_ref().and_then(|editor| {
      editor.read_with(cx, |editor, cx| {
        Self::annotation_navigation_state_for(file_status, editor, cx)
      })
    })
  }

  fn can_navigate_annotations(state: Option<AnnotationNavigationState>) -> bool {
    state.is_some_and(|state| state.total > 1)
  }

  fn navigate_annotation_in_editor(
    &mut self,
    direction: AnnotationDirection,
    cx: &mut Context<Self>,
  ) {
    let Some(editor) = self.editor.clone() else {
      return;
    };
    let file_status = if self.history_opened_commit_file.is_some() {
      None
    } else {
      self.selected_file_entry().map(|entry| entry.status)
    };

    editor.update(cx, |editor, cx| {
      let use_conflict_nav = matches!(file_status, Some(RepoStatusKind::Conflicted))
        && editor.conflict_navigation_state(cx).is_some();
      if use_conflict_nav {
        editor.navigate_conflict(direction.conflict(), cx);
      } else {
        editor.navigate_hunk(direction.hunk(), cx);
      }
    });
  }

  fn should_show_stage_selected_file_palette_command(&self) -> bool {
    self.selected_repo.is_some()
      && self
        .selected_file_entry()
        .is_some_and(|entry| Self::selected_file_can_stage(entry.stage))
  }

  fn should_show_unstage_selected_file_palette_command(&self) -> bool {
    self.selected_repo.is_some()
      && self
        .selected_file_entry()
        .is_some_and(|entry| Self::selected_file_can_unstage(entry.stage))
  }

  fn selected_file_status(&self) -> Option<RepoStatusKind> {
    self.selected_file_entry().map(|entry| entry.status)
  }

  fn can_accept_all_conflicts(
    selected_status: Option<RepoStatusKind>,
    is_read_only: bool,
    has_unresolved_conflict_markers: bool,
  ) -> bool {
    matches!(selected_status, Some(RepoStatusKind::Conflicted))
      && !is_read_only
      && has_unresolved_conflict_markers
  }

  fn should_show_accept_all_conflicts_palette_commands(&self, cx: &App) -> bool {
    let selected_status = self.selected_file_status();
    self.editor.as_ref().is_some_and(|editor| {
      editor.read_with(cx, |editor, cx| {
        Self::can_accept_all_conflicts(
          selected_status,
          editor.is_read_only,
          editor.has_unresolved_conflict_markers(cx),
        )
      })
    })
  }

  fn should_publish_branch(branch_status: Option<&BranchStatus>, has_head_commit: bool) -> bool {
    has_head_commit
      && matches!(
        branch_status,
        Some(status) if !status.has_upstream && !Self::is_detached_head(Some(status))
      )
  }

  fn should_publish_branch_and_create_pull_request(
    branch_status: Option<&BranchStatus>,
    has_unpublished_branch_commits: bool,
  ) -> bool {
    Self::should_publish_branch(branch_status, true) && has_unpublished_branch_commits
  }

  fn push_action_label(
    branch_status: Option<&BranchStatus>,
    has_head_commit: bool,
  ) -> &'static str {
    if Self::should_publish_branch(branch_status, has_head_commit) {
      "Push (Publish branch)"
    } else {
      "Push"
    }
  }

  fn push_flags(
    branch_status: Option<&BranchStatus>,
    has_head_commit: bool,
    force_push_after_rebase: bool,
  ) -> (bool, bool) {
    let Some(status) = branch_status else {
      return (false, false);
    };
    if Self::should_publish_branch(Some(status), has_head_commit) {
      return (true, false);
    }
    if !status.has_upstream {
      return (false, false);
    }
    if force_push_after_rebase && status.ahead > 0 {
      return (false, true);
    }
    let can_push = status.ahead > 0 && status.behind == 0;
    let can_force_push = status.ahead > 0 && status.behind > 0;
    (can_push, can_force_push)
  }

  fn all_changes_staged(&self) -> bool {
    Self::should_show_unstage_all_command(&self.status_entries)
  }

  fn changed_files_count(entries: &[RepoStatusEntry]) -> usize {
    entries.len()
  }

  fn has_conflicted_entries(entries: &[RepoStatusEntry]) -> bool {
    entries
      .iter()
      .any(|entry| entry.status == RepoStatusKind::Conflicted)
  }

  fn has_untracked_entries(entries: &[RepoStatusEntry]) -> bool {
    entries
      .iter()
      .any(|entry| entry.status == RepoStatusKind::Untracked)
  }

  fn has_tracked_entries(entries: &[RepoStatusEntry]) -> bool {
    entries
      .iter()
      .any(|entry| entry.status != RepoStatusKind::Untracked)
  }

  fn stash_command_flags(entries: &[RepoStatusEntry]) -> (bool, bool) {
    let show_stash = Self::has_tracked_entries(entries);
    let show_stash_with_untracked = show_stash || Self::has_untracked_entries(entries);
    (show_stash, show_stash_with_untracked)
  }

  fn should_show_stage_all_command(entries: &[RepoStatusEntry]) -> bool {
    Self::changed_files_count(entries) > 0 && !Self::all_entries_staged(entries)
  }

  fn should_show_unstage_all_command(entries: &[RepoStatusEntry]) -> bool {
    Self::all_entries_staged(entries)
  }

  fn should_show_unstage_all_palette_command(entries: &[RepoStatusEntry]) -> bool {
    Self::has_staged_changes(entries)
  }

  fn should_confirm_stage_all(
    selected_repo: Option<&PathBuf>,
    status_entries: &[RepoStatusEntry],
  ) -> bool {
    selected_repo.is_some() && Self::has_conflicted_entries(status_entries)
  }

  fn merge_commit_message(source_branch: &str, target_branch: &str) -> String {
    format!("Merge branch '{source_branch}' into {target_branch}")
  }

  fn sync_rebase_commit_input(
    &mut self,
    was_rebase_in_progress: bool,
    rebase_in_progress: bool,
    rebase_commit_message: Option<String>,
    cx: &mut Context<Self>,
  ) {
    if rebase_in_progress {
      let Some(message) = rebase_commit_message
        .map(|message| message.trim().to_string())
        .filter(|message| !message.is_empty())
      else {
        return;
      };
      if self.commit_input.read(cx).value() == message {
        return;
      }
      let commit_input = self.commit_input.clone();
      let window_handle = self.window_handle;
      let _ = cx.update_window(window_handle, move |_, window, cx| {
        commit_input.update(cx, |input, cx| input.set_value(&message, window, cx));
      });
      return;
    }

    if was_rebase_in_progress {
      let current_value = self.commit_input.read(cx).value();
      if current_value.trim().is_empty() {
        return;
      }
      let commit_input = self.commit_input.clone();
      let window_handle = self.window_handle;
      let _ = cx.update_window(window_handle, move |_, window, cx| {
        commit_input.update(cx, |input, cx| input.set_value("", window, cx));
      });
    }
  }

  fn should_show_changed_files_tag(changed_files_count: usize) -> bool {
    changed_files_count > 0
  }

  fn build_history_rows(commits: &[HistoryCommitNode]) -> Vec<HistoryRenderRow> {
    commits
      .iter()
      .cloned()
      .map(HistoryRenderRow::from_commit)
      .collect()
  }

  fn history_change_kind_to_repo_status(kind: CommitFileChangeKind) -> RepoStatusKind {
    match kind {
      CommitFileChangeKind::Added => RepoStatusKind::Added,
      CommitFileChangeKind::Deleted => RepoStatusKind::Deleted,
      CommitFileChangeKind::Modified => RepoStatusKind::Modified,
      CommitFileChangeKind::Renamed => RepoStatusKind::Renamed,
      // RepoStatusKind does not have "Copied", closest visual semantics is renamed.
      CommitFileChangeKind::Copied => RepoStatusKind::Renamed,
      CommitFileChangeKind::Typechange => RepoStatusKind::TypeChange,
      CommitFileChangeKind::Conflicted => RepoStatusKind::Conflicted,
    }
  }

  fn render_history_sidebar_content(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let theme = cx.theme().clone();
    if self.history_loading {
      return div()
        .id("git-history-loading-container")
        .flex()
        .flex_col()
        .size_full()
        .items_center()
        .justify_center()
        .child(
          div()
            .id("git-history-loading-content")
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .child(Spinner::new().small())
            .child(
              div()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child("Loading history..."),
            ),
        )
        .into_any_element();
    }

    if self.history_commits.is_empty() {
      return div()
        .id("git-history-empty-container")
        .flex()
        .flex_col()
        .size_full()
        .items_center()
        .justify_center()
        .child(
          div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child("No commits to display"),
        )
        .into_any_element();
    }

    if let Some(selected_id) = self
      .history_tree
      .read(cx)
      .selected_entry()
      .map(|entry| entry.item().id.to_string())
      && let Some(HistoryTreeNode::File { commit_oid, file }) =
        self.history_tree_nodes.get(selected_id.as_str()).cloned()
    {
      let already_opened = self
        .history_opened_commit_file
        .as_ref()
        .map(|(opened_oid, opened_path)| opened_oid == &commit_oid && opened_path == &file.path)
        .unwrap_or(false);
      if !already_opened {
        let open_commit_oid = commit_oid.clone();
        let open_path = file.path.clone();
        cx.on_next_frame(window, move |this, _, cx| {
          this.open_history_commit_file(open_commit_oid.clone(), open_path.clone(), cx);
        });
      }
    }

    let view = cx.entity();
    let tree_view = tree(
      &self.history_tree,
      move |ix, entry, selected, window, cx| {
        view.update(cx, |this, cx| {
          let theme = cx.theme().clone();
          let item = entry.item();
          let indent = px(12.) + px(16.) * entry.depth();
          let node = this.history_tree_nodes.get(item.id.as_ref()).cloned();

          match node {
            Some(HistoryTreeNode::Commit { oid }) => {
              let row = this
                .history_rows_cache
                .iter()
                .find(|row| row.commit.oid == oid)
                .cloned();

              let Some(row) = row else {
                return selectable_list_item(ix, selected, SelectableRowStyle::Inset, &theme)
                  .w_full()
                  .px_2()
                  .pl(indent)
                  .child(item.label.clone());
              };

              let summary: SharedString = if row.commit.summary.trim().is_empty() {
                "No commit message".into()
              } else {
                row.commit.summary.clone().into()
              };

              let is_expanded = entry.is_expanded();
              if selected {
                if is_expanded {
                  this
                    .history_expanded_commit_oids
                    .insert(row.commit.oid.clone());
                } else {
                  this
                    .history_expanded_commit_oids
                    .remove(row.commit.oid.as_str());
                }
              }
              if is_expanded
                && !this
                  .history_commit_files
                  .contains_key(row.commit.oid.as_str())
                && !this
                  .history_commit_files_loading
                  .contains(row.commit.oid.as_str())
              {
                this.queue_history_commit_files_load(row.commit.oid.clone(), window, cx);
              }
              let chevron = if is_expanded {
                IconName::ChevronDown
              } else {
                IconName::ChevronRight
              };

              selectable_list_item(ix, selected, SelectableRowStyle::Inset, &theme)
                .w_full()
                .pl_2()
                .pr_3()
                .pl(indent)
                .child(
                  h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .child(
                      h_flex()
                        .min_w_0()
                        .flex_1()
                        .items_center()
                        .gap_2()
                        .child(
                          Icon::new(chevron)
                            .size_3()
                            .text_color(theme.muted_foreground),
                        )
                        .child(
                          div()
                            .min_w_0()
                            .flex_1()
                            .overflow_hidden()
                            .text_sm()
                            .text_ellipsis()
                            .child(summary),
                        ),
                    )
                    .child(
                      div()
                        .max_w(px(HISTORY_AUTHOR_MAX_WIDTH))
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(row.commit.author.clone()),
                    ),
                )
            }
            Some(HistoryTreeNode::File { commit_oid, file }) => {
              let status_kind = Self::history_change_kind_to_repo_status(file.kind);
              let status_color = Self::status_color(status_kind, &theme);
              let file_icon = file_icon_path_for_path_with_theme(&file.path, &theme)
                .map(|path| img(path).size(px(FILE_ICON_SIZE_PX)).into_any_element())
                .unwrap_or_else(|| {
                  Icon::new(IconName::File)
                    .size_3()
                    .text_color(theme.sidebar_foreground)
                    .into_any_element()
                });
              let selected = this
                .history_opened_commit_file
                .as_ref()
                .map(|(selected_oid, selected_path)| {
                  selected_oid == &commit_oid && selected_path == &file.path
                })
                .unwrap_or(false);
              let path = file.path.clone();
              let open_commit_oid = commit_oid.clone();

              selectable_list_item(ix, selected, SelectableRowStyle::Inset, &theme)
                .w_full()
                .px_2()
                .pl(indent)
                .child(
                  h_flex()
                    .w_full()
                    .items_center()
                    .gap_2()
                    .child(
                      div()
                        .w(px(15.))
                        .text_xs()
                        .text_color(status_color)
                        .child(status_kind.short_code()),
                    )
                    .child(file_icon)
                    .child(
                      div()
                        .min_w_0()
                        .flex_1()
                        .overflow_hidden()
                        .text_ellipsis_start()
                        .text_xs()
                        .child(file.label.clone()),
                    ),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                  this.open_history_commit_file(open_commit_oid.clone(), path.clone(), cx);
                }))
            }
            Some(HistoryTreeNode::LoadHint { oid }) => {
              let load_oid = oid.clone();
              selectable_list_item(ix, selected, SelectableRowStyle::Inset, &theme)
                .w_full()
                .px_2()
                .pl(indent)
                .child(
                  div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Load files..."),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                  this.queue_history_commit_files_load(load_oid.clone(), window, cx);
                }))
            }
            _ => selectable_list_item(ix, selected, SelectableRowStyle::Inset, &theme)
              .w_full()
              .px_2()
              .pl(indent)
              .child(
                div()
                  .text_xs()
                  .text_color(theme.muted_foreground)
                  .child(item.label.clone()),
              ),
          }
        })
      },
    );

    let tree_focused = self.history_tree_wrapper_focus.contains_focused(window, cx);
    div()
      .id("git-history-scroll-container")
      .track_focus(&self.history_tree_wrapper_focus)
      .relative()
      .flex_1()
      .min_h_0()
      .key_context(crate::shortcuts::GIT_HISTORY_TREE_CONTEXT)
      .child(tree_view.pb_1().flex_1().w_full())
      .when(tree_focused, |this| {
        this.child(
          div()
            .absolute()
            .top_0()
            .right_0()
            .bottom_0()
            .left_0()
            .border_2()
            .border_color(theme.ring.alpha(0.1)),
        )
      })
      .into_any_element()
  }

  fn render_header(&self, _window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let push_pull_loading = self.push_pull_in_progress;
    let on_repo_select = self.repo_select_handler(cx);
    let on_branch_select = self.branch_select_handler(cx);
    let repo_options = self.repo_dropdown_items.clone();
    let branch_options = self.branch_dropdown_items.clone();
    let branch_context = self.github_branch_context(cx);
    let branch_pr_button_state = self.current_branch_pr_button_state(cx);

    let repo_dropdown = dropdown_select(
      DropdownSelectConfig::new("git-header-repo-select")
        .trigger_label("Repository")
        .trigger_height(px(PAGE_HEADER_HEIGHT - 1.))
        .placeholder("Select repository...")
        .search_placeholder("Search repositories...")
        .options(repo_options)
        .width(px(TRIGGER_DROPDOWN_SELECT_WIDTH))
        .menu_width(px(TRIGGER_DROPDOWN_SELECT_WIDTH))
        .on_select(on_repo_select),
    );
    let repo_dropdown = div().child(repo_dropdown);

    let branch_dropdown = dropdown_select(
      DropdownSelectConfig::new("git-header-branch-select")
        .trigger_label("Branch")
        .trigger_height(px(PAGE_HEADER_HEIGHT - 1.))
        .placeholder("Select branch...")
        .search_placeholder("Search branches...")
        .options(branch_options)
        .width(px(TRIGGER_DROPDOWN_SELECT_WIDTH))
        .menu_width(px(TRIGGER_DROPDOWN_SELECT_WIDTH))
        .disabled(self.selected_repo.is_none())
        .on_select(on_branch_select),
    );
    let branch_dropdown = div().child(branch_dropdown);

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
        .child(
          div()
            .flex()
            .items_center()
            .gap_2()
            .child(
              div()
                .id("branch-ahead-push")
                .flex()
                .items_center()
                .gap_1()
                .cursor_pointer()
                .tooltip(|window, cx| Tooltip::new("Push").build(window, cx))
                .on_click(cx.listener(|this, _event, _window, cx| {
                  this.push_changes_action(cx);
                }))
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
                .id("branch-behind-pull")
                .flex()
                .items_center()
                .gap_1()
                .cursor_pointer()
                .tooltip(|window, cx| Tooltip::new("Pull").build(window, cx))
                .on_click(cx.listener(|this, _event, _window, cx| {
                  if let Some(repo_root) = this.selected_repo.clone() {
                    this.pull_repository(repo_root, cx);
                  }
                }))
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
        .when(push_pull_loading, |this| {
          this.child(
            h_flex()
              .items_center()
              .gap_1()
              .child(Spinner::new().small())
              .child(
                div()
                  .text_xs()
                  .text_color(theme.muted_foreground)
                  .child("Syncing"),
              ),
          )
        })
    });

    let fetch_button = Button::new("git-fetch-button")
      .label("Fetch")
      .icon(UiIconName::RefreshCw)
      .outline()
      .loading_icon(Icon::new(UiIconName::RefreshCw))
      .loading(self.fetch_in_progress)
      .with_variant(ButtonVariant::Secondary)
      .xsmall()
      .p_2()
      .disabled(self.selected_repo.is_none() || self.fetch_in_progress)
      .tooltip("Fetch updates from remotes")
      .on_click(cx.listener(Self::fetch_action));

    let branch_pr_button = match branch_pr_button_state {
      GitBranchPullRequestButtonState::Hidden => None,
      GitBranchPullRequestButtonState::Checking => Some(
        Button::new("git-branch-pr-status")
          .label("Checking PR")
          .icon(UiIconName::GitPullRequestArrow)
          .outline()
          .loading(true)
          .with_variant(ButtonVariant::Secondary)
          .xsmall()
          .p_2()
          .disabled(true)
          .tooltip("Looking for an open pull request for this branch"),
      ),
      GitBranchPullRequestButtonState::PublishAndCreate => {
        branch_context.clone().map(|_branch_context| {
          Button::new("git-publish-and-create-branch-pr")
            .label("Publish and Create PR")
            .icon(UiIconName::GitPullRequestArrow)
            .outline()
            .with_variant(ButtonVariant::Secondary)
            .xsmall()
            .p_2()
            .loading(self.publish_branch_and_create_pr_in_progress)
            .disabled(self.push_pull_in_progress || self.publish_branch_and_create_pr_in_progress)
            .tooltip("Publish this branch to GitHub and create a pull request")
            .on_click(cx.listener(|this, _, _window, cx| {
              this.publish_branch_and_create_pull_request_action(cx);
            }))
        })
      }
      GitBranchPullRequestButtonState::OpenExisting {
        owner,
        repo,
        number,
      } => {
        let pr_url = github_shared::pr_url(owner.as_str(), repo.as_str(), number);
        Some(
          Button::new("git-open-branch-pr")
            .label(format!("Open PR #{number}"))
            .icon(UiIconName::GitPullRequestArrow)
            .outline()
            .with_variant(ButtonVariant::Secondary)
            .xsmall()
            .p_2()
            .on_click(move |_, window, cx| {
              if should_open_externally(window) {
                cx.open_url(&pr_url);
              } else {
                GithubPrDetailsPageHandle::show_with_open_target(
                  owner.clone().into(),
                  repo.clone().into(),
                  number,
                  false,
                  None,
                  cx,
                );
              }
            }),
        )
      }
      GitBranchPullRequestButtonState::Create => branch_context.clone().map(|branch_context| {
        let git_page = cx.entity().downgrade();
        Button::new("git-create-branch-pr")
          .label("Create PR")
          .icon(UiIconName::GitPullRequestArrow)
          .outline()
          .with_variant(ButtonVariant::Secondary)
          .xsmall()
          .p_2()
          .on_click(move |_, window, cx| {
            open_create_pull_request_dialog(
              WorkspaceApi::global(cx).api.clone(),
              window.window_handle(),
              git_page.clone(),
              branch_context.clone(),
              window,
              cx,
            );
          })
      }),
    };

    let header_left = div()
      .flex()
      .flex_1()
      .min_w_0()
      .h_full()
      .items_center()
      .gap_3()
      .child(
        div()
          .flex()
          .items_center()
          .child(
            div()
              .border_r_1()
              .border_color(theme.border)
              .child(repo_dropdown),
          )
          .child(
            div()
              .border_r_1()
              .border_color(theme.border)
              .child(branch_dropdown),
          ),
      )
      .when_some(branch_info, |this, info| this.child(info))
      .child(fetch_button);

    let can_access_terminal_sidebar = AuthStateStore::is_admin(cx);
    let terminal_sidebar_button = Button::new("git-toggle-terminal-sidebar")
      .label("Terminal")
      .icon(UiIconName::SquareTerminal)
      .outline()
      .with_variant(ButtonVariant::Secondary)
      .xsmall()
      .p_2()
      .selected(self.show_terminal_sidebar)
      .disabled(self.selected_repo.is_none())
      .on_click(cx.listener(Self::toggle_terminal_sidebar_click));

    let agent_sidebar_button = Button::new("git-toggle-agent-sidebar")
      .label("Agent")
      .icon(UiIconName::Sparkles)
      .outline()
      .with_variant(ButtonVariant::Secondary)
      .xsmall()
      .p_2()
      .selected(self.show_agent_sidebar)
      .disabled(self.selected_repo.is_none())
      .on_click(cx.listener(Self::toggle_agent_sidebar_click));

    let header_right = h_flex()
      .items_center()
      .gap_2()
      .flex_shrink_0()
      .when_some(branch_pr_button, |this, button| this.child(button))
      .child(
        div()
          .debug_selector(|| GIT_AGENT_BUTTON_DEBUG_SELECTOR.to_string())
          .child(agent_sidebar_button),
      );
    let header_right = if can_access_terminal_sidebar {
      header_right.child(
        div()
          .debug_selector(|| GIT_TERMINAL_BUTTON_DEBUG_SELECTOR.to_string())
          .child(terminal_sidebar_button),
      )
    } else {
      header_right
    };

    div()
      .h(px(PAGE_HEADER_HEIGHT))
      .min_h(px(PAGE_HEADER_HEIGHT))
      .max_h(px(PAGE_HEADER_HEIGHT))
      .pr_3()
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
      .px_2()
      .bg(theme.background)
      .items_center()
      .justify_center()
      .text_color(cx.theme().muted_foreground)
      .child(div().truncate().child(message))
      .into_any_element()
  }

  fn should_render_repository_split(selected_repo: Option<&Path>) -> bool {
    selected_repo.is_some()
  }

  fn render_repository_empty_state(
    &mut self,
    window: &Window,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let theme = cx.theme().clone();
    div()
      .size_full()
      .flex()
      .bg(theme.background)
      .items_center()
      .justify_center()
      .child(
        div()
          .id("git-repository-empty-state")
          .flex()
          .flex_col()
          .items_center()
          .gap_3()
          .child(
            div()
              .text_base()
              .font_medium()
              .text_color(theme.foreground)
              .child(EMPTY_REPOSITORY_TITLE),
          )
          .child(
            h_flex()
              .items_center()
              .gap_2()
              .text_sm()
              .text_color(theme.muted_foreground)
              .child(EMPTY_REPOSITORY_HINT_PREFIX)
              .child(Kbd::new(shortcuts::resolved_display_shortcut_keystroke_in(
                cx,
                window,
                ShortcutId::OpenRepository,
              )))
              .child(EMPTY_REPOSITORY_HINT_SUFFIX),
          )
          .child(
            Button::new("git-empty-state-open-repository")
              .label("Open Repository")
              .icon(IconName::FolderOpen)
              .with_variant(ButtonVariant::Secondary)
              .on_click(cx.listener(move |this, _, window, cx| {
                this.start_open_repository(window, cx);
              })),
          ),
      )
      .into_any_element()
  }

  fn render_loading_state(&self, message: &str, cx: &mut Context<Self>) -> AnyElement {
    let message = message.to_string();
    let theme = cx.theme().clone();
    div()
      .size_full()
      .flex()
      .bg(theme.background)
      .items_center()
      .justify_center()
      .child(
        div()
          .id("git-editor-loading-state")
          .flex()
          .flex_col()
          .items_center()
          .gap_2()
          .child(Spinner::new().small())
          .child(
            div()
              .text_sm()
              .text_color(theme.muted_foreground)
              .child(message),
          ),
      )
      .into_any_element()
  }

  fn should_show_editor_loading_state(selected_file: Option<&Path>, has_editor: bool) -> bool {
    selected_file.is_some() && !has_editor
  }

  fn should_show_open_action_loading_state(
    pending_open_action: Option<&GitPageOpenAction>,
    selected_file: Option<&Path>,
    has_editor: bool,
  ) -> bool {
    pending_open_action.is_some() && selected_file.is_none() && !has_editor
  }

  fn render_editor_header(&self, editor: &Entity<Editor>, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let editor_state = editor.read(cx);
    let is_history_commit_file = self.history_opened_commit_file.is_some();
    let selected_entry = self
      .selected_file
      .as_ref()
      .and_then(|path| self.status_entries.iter().find(|entry| &entry.path == path))
      .cloned();
    let display_path = selected_entry
      .as_ref()
      .map(|entry| entry.path.as_path())
      .or(self.selected_file.as_deref())
      .unwrap_or(editor_state.workdir_path.as_path());
    let file_name = format_git_file_name_label(display_path);
    let old_file_name = selected_entry
      .as_ref()
      .and_then(|entry| entry.old_path.as_ref())
      .map(|path| format_git_file_name_label(path));
    let dir_path = display_path
      .parent()
      .and_then(|parent| parent.to_str())
      .unwrap_or("")
      .to_string();
    let file_dirty = editor_state.is_dirty;
    let editor_entity = editor.clone();
    let status_kind = selected_entry.as_ref().map(|entry| entry.status);
    let status_letter = status_kind.map(|status| status.short_code());
    let status_color = status_kind
      .map(|status| Self::status_color(status, &theme))
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
              .child(render_repo_status_label(
                &theme,
                status_kind,
                file_name,
                old_file_name,
              ))
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

    let show_save_button = !is_history_commit_file && !editor_state.is_read_only;
    let save_button = Button::new("editor-save")
      .label("Save")
      .xsmall()
      .ghost()
      .disabled(!file_dirty)
      .on_click(move |_, _, cx| {
        editor_entity.update(cx, |editor, cx| editor.save(cx));
      });

    let is_markdown = self.selected_file_is_markdown();
    let is_svg = self.selected_file_is_svg();
    let preview_active = (is_markdown || is_svg) && self.show_markdown_preview;
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
    let hide_whitespace = self.hide_whitespace;
    let show_whitespace_button = self.binary_preview.is_none();

    let whitespace_icon = if hide_whitespace {
      IconName::Eye
    } else {
      IconName::EyeOff
    };
    let tooltip = if hide_whitespace {
      "Show whitespace changes"
    } else {
      "Hide whitespace changes"
    };
    let whitespace_button = div()
      .debug_selector(|| "editor-whitespace-toggle".to_string())
      .child(
        Button::new("editor-whitespace-toggle")
          .label("Whitespace")
          .icon(whitespace_icon)
          .tooltip(tooltip)
          .xsmall()
          .ghost()
          .on_click(move |_, _, cx| {
            view.update(cx, |this, cx| {
              this.toggle_hide_whitespace(cx);
            });
          }),
      );

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

    let agent_review_count = self
      .agent_review_comments
      .iter()
      .filter(|comment| agent_review_comment_is_copyable(comment))
      .count();
    let view = cx.entity();
    let copy_agent_review_button = Button::new("editor-copy-agent-review")
      .label(format!("Copy Review ({agent_review_count})"))
      .icon(IconName::Copy)
      .xsmall()
      .ghost()
      .tooltip("Copy local review comments for agent")
      .on_click({
        let view = view.clone();
        move |_, window, cx| {
          view.update(cx, |this, cx| {
            this.copy_agent_review_to_clipboard(window, cx);
          });
        }
      });

    let send_agent_review_button = Button::new("editor-send-agent-review")
      .label("Send to Agent")
      .icon(UiIconName::Sparkles)
      .xsmall()
      .ghost()
      .tooltip("Send local review comments to the agent")
      .on_click({
        let view = view.clone();
        move |_, window, cx| {
          view.update(cx, |this, cx| {
            this.send_agent_review_to_agent(window, cx);
          });
        }
      });

    let file_status = if is_history_commit_file {
      None
    } else {
      selected_entry.as_ref().map(|entry| entry.status)
    };
    let annotation_navigation =
      Self::annotation_navigation_state_for(file_status, &editor_state, cx);
    let can_navigate_annotations = Self::can_navigate_annotations(annotation_navigation);
    let show_accept_all_conflict_actions = matches!(file_status, Some(RepoStatusKind::Conflicted));
    let can_accept_all_conflicts = Self::can_accept_all_conflicts(
      file_status,
      editor_state.is_read_only,
      editor_state.has_unresolved_conflict_markers(cx),
    );

    let editor_entity_accept_current = editor.clone();
    let accept_all_current_button = Button::new("editor-accept-all-current")
      .label("Accept All Current")
      .xsmall()
      .ghost()
      .disabled(!can_accept_all_conflicts)
      .on_click(move |_, _, cx| {
        editor_entity_accept_current.update(cx, |editor, cx| {
          editor.resolve_all_conflicts(ConflictResolution::Current, cx);
        });
      });

    let editor_entity_accept_incoming = editor.clone();
    let accept_all_incoming_button = Button::new("editor-accept-all-incoming")
      .label("Accept All Incoming")
      .xsmall()
      .ghost()
      .disabled(!can_accept_all_conflicts)
      .on_click(move |_, _, cx| {
        editor_entity_accept_incoming.update(cx, |editor, cx| {
          editor.resolve_all_conflicts(ConflictResolution::Incoming, cx);
        });
      });

    let annotation_kind = annotation_navigation.map(|state| state.kind);
    let (previous_tooltip, next_tooltip) = match annotation_kind {
      Some(AnnotationKind::Conflict) => ("Previous conflict", "Next conflict"),
      _ => ("Previous change", "Next change"),
    };

    let view = cx.entity();
    let previous_annotation_button = Button::new("editor-annotation-prev")
      .icon(IconName::ArrowUp)
      .xsmall()
      .ghost()
      .compact()
      .tooltip(previous_tooltip)
      .disabled(!can_navigate_annotations)
      .on_click(move |_, _, cx| {
        view.update(cx, |this, cx| {
          this.navigate_annotation_in_editor(AnnotationDirection::Previous, cx);
        });
      });

    let view = cx.entity();
    let next_annotation_button = Button::new("editor-annotation-next")
      .icon(IconName::ArrowDown)
      .xsmall()
      .ghost()
      .compact()
      .tooltip(next_tooltip)
      .disabled(!can_navigate_annotations)
      .on_click(move |_, _, cx| {
        view.update(cx, |this, cx| {
          this.navigate_annotation_in_editor(AnnotationDirection::Next, cx);
        });
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
          .when_some(
            annotation_navigation.filter(|state| state.total > 1),
            |this, annotation_navigation| {
              this.child(
                h_flex()
                  .items_center()
                  .gap_1()
                  .child(previous_annotation_button)
                  .child(
                    div()
                      .w(px(52.0))
                      .text_xs()
                      .text_center()
                      .text_color(theme.muted_foreground)
                      .child(format!(
                        "{}/{}",
                        annotation_navigation.active_index + 1,
                        annotation_navigation.total
                      )),
                  )
                  .child(next_annotation_button),
              )
            },
          )
          .when(show_accept_all_conflict_actions, |this| {
            this
              .child(accept_all_current_button)
              .child(accept_all_incoming_button)
          })
          .when(show_save_button, |this| this.child(save_button))
          .when(is_markdown || is_svg, |this| this.child(preview_button))
          .when(show_whitespace_button, |this| this.child(whitespace_button))
          .when(!is_history_commit_file && agent_review_count > 0, |this| {
            this
              .child(copy_agent_review_button)
              .child(send_agent_review_button)
          })
          .child(toggle_button),
      )
      .into_any_element()
  }

  fn render_interactive_rebase_todo_header(&self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    h_flex()
      .min_h(px(EDITOR_HEADER_HEIGHT))
      .h(px(EDITOR_HEADER_HEIGHT))
      .px_3()
      .items_center()
      .justify_between()
      .gap_2()
      .bg(theme.sidebar)
      .border_b_1()
      .border_color(theme.title_bar_border)
      .child(
        h_flex()
          .items_center()
          .gap_2()
          .child(Icon::new(UiIconName::GitMerge).size_3())
          .child("Interactive rebase"),
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
      .overflow_hidden()
      .child(editor);

    if let Some(overlay) = overlay {
      wrapper = wrapper.child(overlay);
    }

    wrapper.into_any_element()
  }

  fn render_change_block_actions(
    &mut self,
    editor: &Entity<Editor>,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Option<AnyElement> {
    let theme = cx.theme().clone();
    let editor_state = editor.read(cx);
    if self.history_opened_commit_file.is_some() || editor_state.is_read_only {
      return None;
    }
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

    if matches!(selected_status, Some(RepoStatusKind::Conflicted)) {
      let conflict_start_line = editor_state.hovered_conflict_start_line?;
      let anchor_display_line = editor_state
        .first_display_line_for_conflict(conflict_start_line)
        .unwrap_or(conflict_start_line);
      if editor_state.find_panel_occludes_display_line(anchor_display_line) {
        return None;
      }
      let mut top = Self::hunk_action_top(
        editor_state.measured_editor_line_height(),
        anchor_display_line,
        editor_state.scroll_offset_y,
      );
      if top >= editor_state.viewport_height {
        return None;
      }
      if top < px(0.0) {
        top = px(0.0);
      }

      let editor_entity = editor.clone();
      let mut actions = div().flex().items_center();

      let editor_entity_current = editor_entity.clone();
      actions = actions.child(
        Button::new("accept-current-conflict")
          .label("Accept Current")
          .small()
          .bg(theme.background)
          .rounded_t_none()
          .rounded_br_none()
          .on_click(move |_, _, cx| {
            editor_entity_current.update(cx, |editor, cx| {
              editor.resolve_conflict_region(conflict_start_line, ConflictResolution::Current, cx);
            });
          }),
      );

      let editor_entity_incoming = editor_entity.clone();
      actions = actions.child(
        Button::new("accept-incoming-conflict")
          .label("Accept Incoming")
          .small()
          .bg(theme.background)
          .rounded_none()
          .on_click(move |_, _, cx| {
            editor_entity_incoming.update(cx, |editor, cx| {
              editor.resolve_conflict_region(conflict_start_line, ConflictResolution::Incoming, cx);
            });
          }),
      );

      actions = actions.child(
        Button::new("accept-both-conflict")
          .label("Accept Both")
          .small()
          .bg(theme.background)
          .rounded_t_none()
          .rounded_bl_none()
          .on_click(move |_, _, cx| {
            editor_entity.update(cx, |editor, cx| {
              editor.resolve_conflict_region(conflict_start_line, ConflictResolution::Both, cx);
            });
          }),
      );

      return Some(
        div()
          .absolute()
          .top(top)
          .right(px(30.0))
          .child(actions)
          .into_any_element(),
      );
    }

    let hovered_id = editor_state.hovered_group_id.as_ref()?;
    let overlay = editor_state
      .visible_groups
      .iter()
      .find(|overlay| overlay.id.as_ref() == hovered_id.as_ref())?;

    let anchor_display_line = editor_state
      .first_display_line_for_group(hovered_id)
      .unwrap_or(overlay.display_line);
    if editor_state.find_panel_occludes_display_line(anchor_display_line) {
      return None;
    }
    let mut top = Self::hunk_action_top(
      editor_state.measured_editor_line_height(),
      anchor_display_line,
      editor_state.scroll_offset_y,
    );
    if top >= editor_state.viewport_height {
      return None;
    }
    if top < px(0.0) {
      top = px(0.0);
    }
    let file_dirty = editor_state.is_dirty;

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

  fn hunk_action_top(line_height: Pixels, display_line: usize, scroll_offset: f32) -> Pixels {
    line_height * (display_line as f32 - scroll_offset)
  }

  fn render_commit_button(&mut self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
    let repo_ready = self.selected_repo.is_some();
    let commit_message = self.commit_input.read(cx).value().to_string();
    let commit_enabled = self.commit_primary_action_enabled(&commit_message);
    let can_publish_branch =
      Self::should_publish_branch(self.branch_status.as_ref(), self.has_head_commit);
    let commit_primary_button_state = Self::commit_primary_button_state(
      self.rebase_in_progress,
      !self.status_entries.is_empty(),
      can_publish_branch,
    );
    let amend_enabled = repo_ready && self.has_head_commit;
    let undo_enabled = repo_ready && self.can_undo_last_commit;
    let push_enabled = repo_ready && self.can_push;
    let force_push_enabled = repo_ready && self.can_force_push;
    let menu_enabled = !self.rebase_in_progress
      && (amend_enabled || undo_enabled || push_enabled || force_push_enabled);
    let view = cx.entity();
    let amend_view = view.clone();
    let undo_view = view.clone();
    let push_view = view.clone();
    let force_push_view = view.clone();
    let push_label = Self::push_action_label(self.branch_status.as_ref(), self.has_head_commit);
    let commit_shortcut =
      shortcuts::resolved_display_shortcut_keystroke_in(cx, window, ShortcutId::CommitChanges);

    let main_button = match commit_primary_button_state {
      GitCommitPrimaryButtonState::ContinueRebase => Button::new("commit-button-main")
        .label("Continue")
        .with_variant(ButtonVariant::Secondary)
        .outline()
        .flex_1()
        .rounded_r_none()
        .child(Kbd::new(commit_shortcut.clone()).ml_1())
        .disabled(!commit_enabled)
        .on_click(cx.listener(Self::continue_rebase_action)),
      GitCommitPrimaryButtonState::PublishBranch => Button::new("commit-button-main")
        .label("Publish branch")
        .with_variant(ButtonVariant::Secondary)
        .outline()
        .flex_1()
        .rounded_r_none()
        .loading(self.push_pull_in_progress)
        .disabled(!push_enabled || self.push_pull_in_progress)
        .on_click(cx.listener(|this, _, _, cx| {
          this.push_changes_action(cx);
        })),
      GitCommitPrimaryButtonState::Commit => Button::new("commit-button-main")
        .label("Commit")
        .with_variant(ButtonVariant::Secondary)
        .outline()
        .flex_1()
        .rounded_r_none()
        .child(Kbd::new(commit_shortcut).ml_1())
        .disabled(!commit_enabled)
        .on_click(cx.listener(Self::commit_changes)),
    };

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
                this.focus_page(window, cx);
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
                this.undo_last_commit_action(cx);
                this.focus_page(window, cx);
              });
            }),
        );

        let menu = menu.separator();

        let menu = menu.item(
          PopupMenuItem::new(push_label)
            .icon(IconName::ArrowUp)
            .disabled(!push_enabled)
            .on_click(move |event, window, cx| {
              push_view.update(cx, |this, cx| {
                let _ = event;
                this.push_changes_action(cx);
                this.focus_page(window, cx);
              });
            }),
        );

        menu.item(
          PopupMenuItem::new("Force push (with lease)")
            .icon(IconName::ArrowUp)
            .disabled(!force_push_enabled)
            .on_click(move |event, window, cx| {
              force_push_view.update(cx, |this, cx| {
                let _ = event;
                this.force_push_changes_action(cx);
                this.focus_page(window, cx);
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

  fn render_commit_bar(&mut self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let input = self.commit_input.clone();
    let has_conflicts = Self::has_conflicted_entries(&self.status_entries);
    let detached_head = Self::is_detached_head(self.branch_status.as_ref());
    let operation_error = self.operation_error.clone();

    div()
      .w_full()
      .min_w_0()
      .flex()
      .flex_col()
      .p_2()
      .gap_2()
      .border_t_1()
      .border_color(theme.border)
      .when(detached_head, |this| {
        this.child(
          StatusAlert::new(
            "commit-detached-head-info",
            theme.status_blue(),
            "You are in detached HEAD mode. Commits are not on a branch.",
          )
          .icon(IconName::Info)
          .title("Detached HEAD"),
        )
      })
      .when(has_conflicts, |this| {
        this.child(
          StatusAlert::new(
            "commit-conflicts-warning",
            theme.status_yellow(),
            "Resolve all conflicts before committing.",
          )
          .title("Conflicts detected"),
        )
      })
      .when_some(operation_error, |this, error| {
        this.child(
          StatusAlert::new("commit-operation-error", theme.status_red(), error.clone())
            .icon(IconName::CircleX)
            .title("Operation failed"),
        )
      })
      .child(
        div()
          .w_full()
          .min_w_0()
          .key_context("CommitInput")
          .child(Input::new(&input).w_full()),
      )
      .child(
        div()
          .w_full()
          .min_w_0()
          .child(self.render_commit_button(window, cx)),
      )
  }

  fn render_sidebar_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme();
    let all_staged = self.all_changes_staged();
    let sidebar_enabled = self.selected_repo.is_some() && !self.status_entries.is_empty();
    let restore_all_enabled = sidebar_enabled;
    let merge_abort_enabled = self.selected_repo.is_some() && self.merge_in_progress;
    let rebase_abort_enabled = self.selected_repo.is_some() && self.rebase_in_progress;
    let changed_files_count = Self::changed_files_count(&self.status_entries);
    let (icon, tooltip) = if all_staged {
      (IconName::Minus, "Unstage all files")
    } else {
      (IconName::Plus, "Stage all files")
    };
    let is_history_mode = self.sidebar_mode == GitSidebarMode::History;
    let (mode_label, mode_icon, mode_tooltip) = if is_history_mode {
      ("Changes", UiIconName::FileCode, "Show changes list")
    } else {
      ("History", UiIconName::History, "Show commit history")
    };

    let group_label = if is_history_mode {
      div()
        .text_sm()
        .text_color(theme.sidebar_foreground)
        .child("History")
        .into_any_element()
    } else {
      h_flex()
        .items_center()
        .gap_2()
        .child(
          div()
            .text_sm()
            .text_color(theme.sidebar_foreground)
            .child("Changes"),
        )
        .when(
          Self::should_show_changed_files_tag(changed_files_count),
          |this| {
            this.child(
              Tag::secondary()
                .small()
                .rounded_full()
                .child(changed_files_count.to_string()),
            )
          },
        )
        .into_any_element()
    };

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
        h_flex()
          .items_center()
          .gap_2()
          .when(self.merge_in_progress, |this| {
            this.child(
              Button::new("abort-merge-button")
                .label("Abort merge")
                .icon(IconName::Undo)
                .xsmall()
                .disabled(!merge_abort_enabled)
                .tooltip("Abort current merge")
                .on_click(cx.listener(Self::abort_merge_action)),
            )
          })
          .when(self.rebase_in_progress, |this| {
            this.child(
              Button::new("abort-rebase-button")
                .label("Abort rebase")
                .icon(IconName::Undo)
                .xsmall()
                .disabled(!rebase_abort_enabled)
                .tooltip("Abort current rebase")
                .on_click(cx.listener(Self::abort_rebase_action)),
            )
          })
          .when(!is_history_mode, |this| {
            this.child(
              ButtonGroup::new("button-group")
                .outline()
                .child(
                  Button::new("stage-all-button")
                    .icon(icon)
                    .with_variant(ButtonVariant::Secondary)
                    .xsmall()
                    .disabled(!sidebar_enabled)
                    .tooltip(tooltip)
                    .on_click(cx.listener(Self::toggle_stage_all_action)),
                )
                .child(
                  Button::new("restore-all-button")
                    .icon(IconName::Undo)
                    .with_variant(ButtonVariant::Secondary)
                    .xsmall()
                    .disabled(!restore_all_enabled)
                    .tooltip("Discard all changes")
                    .on_click(cx.listener(Self::restore_all_click_action)),
                ),
            )
          })
          .child(
            Button::new("sidebar-mode-toggle-button")
              .label(mode_label)
              .outline()
              .icon(mode_icon)
              .with_variant(ButtonVariant::Secondary)
              .xsmall()
              .selected(is_history_mode)
              .disabled(self.selected_repo.is_none())
              .tooltip(mode_tooltip)
              .on_click(cx.listener(Self::toggle_sidebar_mode_action)),
          ),
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

    if self.sidebar_mode == GitSidebarMode::History {
      return base_sidebar
        .relative()
        .child(self.render_sidebar_header(cx))
        .child(self.render_history_sidebar_content(window, cx))
        .into_any_element();
    }

    let file_list_focused = self.file_list.read(cx).focus_handle(cx).is_focused(window);
    let list_container = div()
      .id("git-sidebar-file-list-container")
      .relative()
      .flex_1()
      .min_h_0()
      .overflow_hidden()
      .child(
        List::new(&self.file_list)
          .flex_1()
          .w_full()
          .min_h_0()
          .p(px(6.)),
      )
      .when(file_list_focused, |this| {
        this.child(
          div()
            .absolute()
            .top_0()
            .right_0()
            .bottom_0()
            .left_0()
            .border_2()
            .border_color(cx.theme().ring.alpha(0.1)),
        )
      });

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
      .child(self.render_commit_bar(window, cx))
      .into_any_element()
  }

  fn render_editor_area(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
    if self.selected_repo.is_none() {
      return self.render_repository_empty_state(window, cx);
    }

    if let Some(todo_view) = self.interactive_rebase_todo_view.clone() {
      return div()
        .size_full()
        .flex()
        .flex_col()
        .child(self.render_interactive_rebase_todo_header(cx))
        .child(todo_view)
        .into_any_element();
    }

    let theme = cx.theme().clone();
    if let Some(editor) = self.editor.clone() {
      let editor_view = self.render_editor_with_overlay(editor.clone(), window, cx);
      if let Some(binary_preview) = self.binary_preview.as_ref() {
        return div()
          .size_full()
          .overflow_hidden()
          .flex()
          .flex_col()
          .child(self.render_editor_header(&editor, cx))
          .child(self.render_binary_preview_content(binary_preview, cx))
          .into_any_element();
      }

      if self.show_markdown_preview
        && (self.selected_file_is_markdown() || self.selected_file_is_svg())
      {
        let preview_content = if self.selected_file_is_svg() {
          self.update_svg_preview(window, cx);
          let preview = match self.svg_preview.clone() {
            Some(Ok(image)) => img(image).max_w_full().max_h_full().into_any_element(),
            Some(Err(error)) => div()
              .text_sm()
              .text_color(theme.status_red())
              .child(error)
              .into_any_element(),
            None => div()
              .text_sm()
              .text_color(theme.muted_foreground)
              .child("Rendering SVG preview...")
              .into_any_element(),
          };
          div()
            .flex_1()
            .min_h_0()
            .min_w(px(0.0))
            .bg(theme.background)
            .occlude()
            .child(
              div()
                .flex_1()
                .min_h_0()
                .min_w(px(0.0))
                .p_4()
                .items_center()
                .justify_center()
                .child(preview),
            )
            .into_any_element()
        } else {
          let markdown = editor.read(cx).document().read(cx);
          let markdown = markdown.slice_to_string(0..markdown.len());
          div()
            .flex_1()
            .min_h_0()
            .min_w(px(0.0))
            .bg(theme.background)
            .occlude()
            .child(
              div().size_full().pb_4().px_4().child(
                TextView::markdown("git-markdown-preview-text", markdown)
                  .size_full()
                  .selectable(true)
                  .scrollable(true),
              ),
            )
            .into_any_element()
        };

        return div()
          .size_full()
          .flex()
          .flex_col()
          .child(self.render_editor_header(&editor, cx))
          .child(
            div().flex_1().min_h_0().child(
              ui::h_resizable("git-page-markdown-preview")
                .child(
                  ui::resizable_panel().child(
                    div()
                      .size_full()
                      .min_w(px(0.0))
                      .min_h_0()
                      .flex()
                      .flex_col()
                      .debug_selector(|| GIT_MARKDOWN_PREVIEW_EDITOR_DEBUG_SELECTOR.to_string())
                      .child(editor_view),
                  ),
                )
                .child(
                  ui::resizable_panel().child(
                    div()
                      .size_full()
                      .min_w(px(0.0))
                      .min_h_0()
                      .flex()
                      .flex_col()
                      .debug_selector(|| GIT_MARKDOWN_PREVIEW_RENDER_DEBUG_SELECTOR.to_string())
                      .child(preview_content),
                  ),
                ),
            ),
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

    if Self::should_show_editor_loading_state(self.selected_file.as_deref(), self.editor.is_some())
    {
      return self.render_loading_state("Loading file...", cx);
    }

    if Self::should_show_open_action_loading_state(
      self.pending_open_action.as_ref(),
      self.selected_file.as_deref(),
      self.editor.is_some(),
    ) && let Some(action) = self.pending_open_action.as_ref()
    {
      return self.render_loading_state(Self::open_action_loading_message(action), cx);
    }

    self.render_empty_state("Select a file to view diff", cx)
  }

  fn render_terminal_sidebar(&mut self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    div()
      .size_full()
      .p_2()
      .min_w(px(0.0))
      .min_h_0()
      .bg(theme.sidebar)
      .debug_selector(|| GIT_TERMINAL_SIDEBAR_DEBUG_SELECTOR.to_string())
      .child(self.terminal_view.clone())
      .into_any_element()
  }

  fn render_agent_sidebar(&mut self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let mut container = div()
      .size_full()
      .min_w(px(0.0))
      .min_h_0()
      .bg(theme.sidebar)
      .debug_selector(|| GIT_AGENT_SIDEBAR_DEBUG_SELECTOR.to_string());
    if let Some(view) = self.agent_chat_view.clone() {
      container = container.child(view);
    }
    container.into_any_element()
  }

  fn render_main_content(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
    let editor_area = self.render_editor_area(window, cx);
    let show_terminal = self.show_terminal_sidebar && AuthStateStore::is_admin(cx);
    let show_agent = self.show_agent_sidebar;
    if !show_terminal && !show_agent {
      return editor_area;
    }

    let (sidebar_element, default_width, min_width, max_width, split_id) = if show_agent {
      (
        self.render_agent_sidebar(cx),
        AGENT_SIDEBAR_DEFAULT_WIDTH,
        AGENT_SIDEBAR_MIN_WIDTH,
        AGENT_SIDEBAR_MAX_WIDTH,
        "git-page-editor-agent-split",
      )
    } else {
      (
        self.render_terminal_sidebar(cx),
        TERMINAL_SIDEBAR_DEFAULT_WIDTH,
        TERMINAL_SIDEBAR_MIN_WIDTH,
        TERMINAL_SIDEBAR_MAX_WIDTH,
        "git-page-editor-terminal-split",
      )
    };

    ui::h_resizable(split_id)
      .child(ui::resizable_panel().child(editor_area))
      .child(
        ui::resizable_panel()
          .size(px(default_width))
          .size_range(px(min_width)..px(max_width))
          .child(sidebar_element),
      )
      .into_any_element()
  }
}

impl Render for GitPage {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    if !AuthStateStore::is_admin(cx) {
      self.show_terminal_sidebar = false;
    }

    let working_directory = self.selected_repo.clone();
    self.terminal_view.update(cx, |view, cx| {
      view.set_working_directory(working_directory, cx);
    });

    let content = if Self::should_render_repository_split(self.selected_repo.as_deref()) {
      ui::h_resizable("git-page-split")
        .child(
          ui::resizable_panel()
            .size(px(SIDEBAR_DEFAULT_WIDTH))
            .size_range(px(SIDEBAR_MIN_WIDTH)..px(SIDEBAR_MAX_WIDTH))
            .child(self.render_sidebar(window, cx)),
        )
        .child(ui::resizable_panel().child(self.render_main_content(window, cx)))
        .into_any_element()
    } else {
      self.render_repository_empty_state(window, cx)
    };

    div()
      .size_full()
      .flex()
      .flex_col()
      .bg(cx.theme().background)
      .track_focus(&self.focus_handle(cx))
      .on_action(cx.listener(GitPage::show_command_palette_action))
      .on_action(cx.listener(GitPage::show_branch_switcher_action))
      .on_action(cx.listener(GitPage::show_file_search_action))
      .on_action(cx.listener(GitPage::find_action))
      .on_action(cx.listener(GitPage::close_find_action))
      .on_action(cx.listener(GitPage::open_repository_action))
      .on_action(cx.listener(GitPage::toggle_terminal_sidebar_action))
      .on_action(cx.listener(GitPage::toggle_agent_sidebar_action))
      .on_action(cx.listener(GitPage::commit_changes_action))
      .on_action(cx.listener(GitPage::open_git_history_sidebar_action))
      .on_action(cx.listener(GitPage::open_git_changes_sidebar_action))
      .on_action(cx.listener(GitPage::pull_changes_action))
      .on_action(cx.listener(GitPage::push_changes_shortcut_action))
      .on_action(cx.listener(GitPage::force_push_changes_shortcut_action))
      .on_action(cx.listener(GitPage::toggle_diff_view_action))
      .on_action(cx.listener(GitPage::toggle_hide_whitespace_action))
      .on_action(cx.listener(GitPage::previous_annotation_action))
      .on_action(cx.listener(GitPage::next_annotation_action))
      .on_action(cx.listener(GitPage::comment_hunk_action))
      .on_action(cx.listener(GitPage::copy_review_comments_for_agent_action))
      .on_action(cx.listener(GitPage::toggle_hunk_stage_action))
      .on_action(cx.listener(GitPage::restore_hunk_action))
      .on_action(cx.listener(GitPage::toggle_file_stage_action))
      .on_action(cx.listener(GitPage::restore_file_shortcut_action))
      .on_action(cx.listener(GitPage::accept_both_conflict_action))
      .child(self.render_header(window, cx))
      .child(content)
  }
}

impl Focusable for GitPage {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use git2::build::CheckoutBuilder;
  use git2::{BranchType, Cred, PushOptions, RemoteCallbacks, Repository, Signature};
  use gpui::TestAppContext;
  use std::io::{Read, Write};
  use std::net::TcpListener;
  use std::sync::atomic::{AtomicU64, Ordering};
  use std::time::{SystemTime, UNIX_EPOCH};
  use ui::CommandPaletteCommandId;

  use crate::api::{ApiClient, User, UserRole, UserSubscription};

  #[test]
  fn format_git_file_name_label_extracts_file_name() {
    let path = Path::new("src/features/renamed_file.rs");

    assert_eq!(format_git_file_name_label(path).as_ref(), "renamed_file.rs");
  }

  #[test]
  fn format_git_file_name_label_strips_newlines() {
    let path = Path::new("src/renamed\n_file.rs");

    assert_eq!(format_git_file_name_label(path).as_ref(), "renamed_file.rs");
  }

  #[test]
  fn format_git_path_label_parts_splits_prefix_and_name() {
    let path = Path::new("desktop/crates/workspace/src/git_page.rs");
    let (prefix, name) = format_git_path_label_parts(path);

    assert_eq!(prefix.as_ref(), "desktop/crates/workspace/src/");
    assert_eq!(name.as_ref(), "git_page.rs");
  }

  #[test]
  fn agent_review_line_label_formats_ranges() {
    let comment = LocalAgentReviewComment {
      id: 1,
      in_reply_to_id: None,
      path: PathBuf::from("src/main.rs"),
      line: 12,
      side: ReviewCommentSide::Right,
      start_line: Some(10),
      start_side: Some(ReviewCommentSide::Right),
      body: Arc::from("Please simplify this."),
      original_start_line: Some(11),
      original_lines: vec!["let value = true;".to_string()],
      state: LocalAgentReviewCommentState::Draft,
    };

    assert_eq!(agent_review_line_label(&comment), "L11-L13");
  }

  #[test]
  fn format_agent_review_export_groups_and_keeps_suggestions() {
    let comments = vec![
      LocalAgentReviewComment {
        id: 2,
        in_reply_to_id: None,
        path: PathBuf::from("src/lib.rs"),
        line: 4,
        side: ReviewCommentSide::Right,
        start_line: None,
        start_side: None,
        body: Arc::from("Use the shared helper."),
        original_start_line: Some(5),
        original_lines: vec!["let value = custom();".to_string()],
        state: LocalAgentReviewCommentState::Draft,
      },
      LocalAgentReviewComment {
        id: 1,
        in_reply_to_id: None,
        path: PathBuf::from("src/main.rs"),
        line: 1,
        side: ReviewCommentSide::Right,
        start_line: None,
        start_side: None,
        body: Arc::from("Replace with:\n\n```suggestion\nlet value = shared();\n```"),
        original_start_line: Some(2),
        original_lines: vec!["let value = custom();".to_string()],
        state: LocalAgentReviewCommentState::Draft,
      },
    ];

    let export = format_agent_review_export(&comments);

    assert!(export.contains("## src/main.rs\n\n### L2 (new side)"));
    assert!(export.contains("```suggestion\nlet value = shared();\n```"));
    assert!(export.contains("## src/lib.rs\n\n### L5 (new side)"));
    assert!(export.find("src/lib.rs") < export.find("src/main.rs"));
  }

  #[test]
  fn next_agent_review_comment_state_marks_copied_suggestion_as_addressed() {
    let comment = LocalAgentReviewComment {
      id: 1,
      in_reply_to_id: None,
      path: PathBuf::from("src/main.rs"),
      line: 1,
      side: ReviewCommentSide::Right,
      start_line: None,
      start_side: None,
      body: Arc::from("Use this:\n\n```suggestion\nlet value = shared();\n```"),
      original_start_line: Some(2),
      original_lines: vec!["let value = custom();".to_string()],
      state: LocalAgentReviewCommentState::Copied,
    };
    let current_lines = vec![
      "fn main() {".to_string(),
      "let value = shared();".to_string(),
      "}".to_string(),
    ];

    assert_eq!(
      next_agent_review_comment_state(&comment, &current_lines),
      LocalAgentReviewCommentState::Addressed
    );
  }

  #[test]
  fn next_agent_review_comment_state_marks_copied_mismatch_as_outdated() {
    let comment = LocalAgentReviewComment {
      id: 1,
      in_reply_to_id: None,
      path: PathBuf::from("src/main.rs"),
      line: 1,
      side: ReviewCommentSide::Right,
      start_line: None,
      start_side: None,
      body: Arc::from("Please simplify this."),
      original_start_line: Some(2),
      original_lines: vec!["let value = custom();".to_string()],
      state: LocalAgentReviewCommentState::Copied,
    };
    let current_lines = vec![
      "fn main() {".to_string(),
      "let value = changed();".to_string(),
      "}".to_string(),
    ];

    assert_eq!(
      next_agent_review_comment_state(&comment, &current_lines),
      LocalAgentReviewCommentState::Outdated
    );
  }

  #[test]
  fn format_agent_review_export_skips_addressed_and_outdated_comments() {
    let comments = vec![
      LocalAgentReviewComment {
        id: 1,
        in_reply_to_id: None,
        path: PathBuf::from("src/main.rs"),
        line: 1,
        side: ReviewCommentSide::Right,
        start_line: None,
        start_side: None,
        body: Arc::from("Still active."),
        original_start_line: Some(2),
        original_lines: vec!["let value = custom();".to_string()],
        state: LocalAgentReviewCommentState::Copied,
      },
      LocalAgentReviewComment {
        id: 2,
        in_reply_to_id: None,
        path: PathBuf::from("src/main.rs"),
        line: 3,
        side: ReviewCommentSide::Right,
        start_line: None,
        start_side: None,
        body: Arc::from("Already fixed."),
        original_start_line: Some(4),
        original_lines: vec!["let old = value;".to_string()],
        state: LocalAgentReviewCommentState::Addressed,
      },
      LocalAgentReviewComment {
        id: 3,
        in_reply_to_id: None,
        path: PathBuf::from("src/main.rs"),
        line: 5,
        side: ReviewCommentSide::Right,
        start_line: None,
        start_side: None,
        body: Arc::from("Stale."),
        original_start_line: Some(6),
        original_lines: vec!["let stale = value;".to_string()],
        state: LocalAgentReviewCommentState::Outdated,
      },
    ];

    let export = format_agent_review_export(&comments);

    assert!(export.contains("Still active."));
    assert!(!export.contains("Already fixed."));
    assert!(!export.contains("Stale."));
  }

  #[test]
  fn recent_repo_item_splits_prefix_and_name() {
    let repo = RecentRepository {
      path: PathBuf::from("/Users/joris/workspace/reviu"),
    };
    let item = RecentRepoItem::new(&repo, Some(Path::new("/Users/joris/workspace/reviu")));

    assert_eq!(item.path, PathBuf::from("/Users/joris/workspace/reviu"));
    assert_eq!(item.prefix.as_ref(), "/Users/joris/workspace/");
    assert_eq!(item.name.as_ref(), "reviu");
    assert!(item.is_selected);
  }

  #[test]
  fn git_file_row_keeps_entry_paths() {
    let row = GitFileRow::new(RepoStatusEntry {
      path: PathBuf::from("src/features/new_file.rs"),
      old_path: Some(PathBuf::from("src/features/old_file.rs")),
      status: RepoStatusKind::Renamed,
      stage: RepoStage::Unstaged,
    });

    assert_eq!(row.entry.path, PathBuf::from("src/features/new_file.rs"));
    assert_eq!(
      row.entry.old_path.as_deref(),
      Some(Path::new("src/features/old_file.rs"))
    );
  }

  #[test]
  fn git_refresh_helper_reports_loading_when_status_or_branch_refresh_is_running() {
    assert!(!git_refresh_in_progress(false, false));
    assert!(git_refresh_in_progress(true, false));
    assert!(git_refresh_in_progress(false, true));
    assert!(git_refresh_in_progress(true, true));
  }

  #[gpui::test]
  fn git_page_handle_refreshing_ignores_lingering_tasks(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (git_page, cx) = add_git_page_window_with_root(cx);

    cx.update(|_, cx| {
      assert!(!GitPageHandle::is_refreshing(cx));
    });

    git_page.update_in(cx, |this, _window, cx| {
      this.status_task = Some(cx.spawn(async move |_, _| {}));
      this.branch_task = Some(cx.spawn(async move |_, _| {}));
      this.status_refresh_in_progress = false;
      this.branch_refresh_in_progress = false;
    });

    cx.update(|_, cx| {
      assert!(!GitPageHandle::is_refreshing(cx));
    });

    git_page.update_in(cx, |this, _window, _cx| {
      this.status_refresh_in_progress = true;
    });

    cx.update(|_, cx| {
      assert!(GitPageHandle::is_refreshing(cx));
    });
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

  struct TempBareRepo {
    path: PathBuf,
  }

  impl TempBareRepo {
    fn init(prefix: &str) -> Self {
      let mut path = std::env::temp_dir();
      let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
      path.push(format!(
        "reviu-{prefix}-bare-{}-{nanos}",
        std::process::id()
      ));
      std::fs::create_dir_all(&path).expect("create temp dir");
      Repository::init_bare(&path).expect("init bare git repository");
      Self { path }
    }
  }

  impl Drop for TempBareRepo {
    fn drop(&mut self) {
      let _ = std::fs::remove_dir_all(&self.path);
    }
  }

  struct TempDir {
    path: PathBuf,
  }

  impl TempDir {
    fn new(prefix: &str) -> Self {
      let mut path = std::env::temp_dir();
      let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
      path.push(format!("reviu-{prefix}-dir-{}-{nanos}", std::process::id()));
      std::fs::create_dir_all(&path).expect("create temp dir");
      Self { path }
    }
  }

  impl Drop for TempDir {
    fn drop(&mut self) {
      let _ = std::fs::remove_dir_all(&self.path);
    }
  }

  fn commit_text_file(
    repo_root: &Path,
    rel_path: &Path,
    contents: &str,
    message: &str,
  ) -> git2::Oid {
    let repo = Repository::open(repo_root).expect("open repo");
    std::fs::write(repo_root.join(rel_path), contents).expect("write worktree file");

    let mut index = repo.index().expect("open index");
    index.add_path(rel_path).expect("stage file");
    index.write().expect("write index");
    let tree_id = index.write_tree().expect("write tree");
    let tree = repo.find_tree(tree_id).expect("find tree");
    let signature = Signature::now("Reviu Tests", "tests@reviu.local").expect("signature");
    let parent = repo.head().ok().and_then(|head| head.peel_to_commit().ok());

    match parent {
      Some(parent) => repo
        .commit(
          Some("HEAD"),
          &signature,
          &signature,
          message,
          &tree,
          &[&parent],
        )
        .expect("commit with parent"),
      None => repo
        .commit(Some("HEAD"), &signature, &signature, message, &tree, &[])
        .expect("initial commit"),
    }
  }

  fn push_branch_to_remote(repo_root: &Path, branch_name: &str, remote_name: &str) {
    let repo = Repository::open(repo_root).expect("open repo");
    let mut remote = repo.find_remote(remote_name).expect("find remote");
    let refspec = format!("refs/heads/{branch_name}:refs/heads/{branch_name}");
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(|_, _, _| Cred::default());
    let mut options = PushOptions::new();
    options.remote_callbacks(callbacks);
    remote
      .push(&[refspec], Some(&mut options))
      .expect("push branch");
  }

  fn set_upstream(repo_root: &Path, local_branch: &str, upstream_branch: &str) {
    let repo = Repository::open(repo_root).expect("open repo");
    let mut branch = repo
      .find_branch(local_branch, BranchType::Local)
      .expect("find local branch");
    branch
      .set_upstream(Some(upstream_branch))
      .expect("set upstream");
  }

  fn set_remote_head(remote_root: &Path, branch_name: &str) {
    let refname = format!("refs/heads/{branch_name}");
    Repository::open(remote_root)
      .expect("open remote")
      .set_head(&refname)
      .expect("set remote HEAD");
  }

  fn head_oid(repo_root: &Path) -> git2::Oid {
    Repository::open(repo_root)
      .expect("open repo")
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read head")
      .id()
  }

  fn remote_branch_oid(remote_root: &Path, branch_name: &str) -> git2::Oid {
    let refname = format!("refs/heads/{branch_name}");
    Repository::open(remote_root)
      .expect("open remote")
      .refname_to_id(&refname)
      .expect("read remote branch oid")
  }

  fn force_checkout_head(repo_root: &Path) {
    let repo = Repository::open(repo_root).expect("open repo");
    let mut checkout = CheckoutBuilder::new();
    checkout.force();
    repo
      .checkout_head(Some(&mut checkout))
      .expect("force checkout HEAD");
  }

  fn make_commit(oid: &str, parents: &[&str]) -> HistoryCommitNode {
    HistoryCommitNode {
      oid: oid.to_string(),
      short_oid: oid.chars().take(7).collect(),
      summary: format!("commit-{oid}"),
      author: "author".to_string(),
      parent_oids: parents.iter().map(|parent| parent.to_string()).collect(),
      refs: Vec::new(),
    }
  }

  fn make_history_file(path: &str, kind: CommitFileChangeKind) -> HistoryCommitFileRow {
    HistoryCommitFileRow::from_commit_file(CommitChangedFile {
      path: PathBuf::from(path),
      old_path: None,
      kind,
    })
  }

  fn make_history_revision(tag: &str) -> HistoryRevision {
    HistoryRevision {
      head_oid: Some(format!("head-{tag}")),
      head_label: Some(format!("HEAD -> {tag}")),
      refs: vec![format!("{tag}@oid-{tag}")],
    }
  }

  fn make_branch_status(
    name: &str,
    ahead: usize,
    behind: usize,
    has_upstream: bool,
  ) -> BranchStatus {
    BranchStatus {
      name: name.to_string(),
      ahead,
      behind,
      has_upstream,
    }
  }

  fn make_branch_pull_request(number: u64) -> GithubPullRequest {
    GithubPullRequest {
      number,
      title: format!("Pull request {number}"),
      state: crate::api::GithubPullRequestState::Open,
      created_at: "2026-03-20T09:00:00Z".to_string(),
      closed_at: None,
      merged_at: None,
      draft: false,
      updated_at: "2026-03-21T10:00:00Z".to_string(),
      comments_count: 0,
      author: crate::api::GithubPullRequestAuthor {
        login: "octocat".to_string(),
        avatar_url: None,
        is_bot: false,
      },
      labels: Vec::new(),
      repository: crate::api::GithubRepository {
        owner: "acme".to_string(),
        repo: "widget".to_string(),
      },
    }
  }

  fn make_test_api_client(base_url: impl Into<String>) -> ApiClient {
    ApiClient::new_with_base_url(base_url)
  }

  fn start_matching_response_server(
    responses: Vec<(String, String, String)>,
  ) -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let address = format!("http://{}", listener.local_addr().expect("local addr"));

    let handle = std::thread::spawn(move || {
      for _ in 0..responses.len() {
        let (mut stream, _) = listener.accept().expect("accept connection");
        let mut request_buffer = [0u8; 4096];
        let bytes_read = stream.read(&mut request_buffer).expect("read request");
        let request = String::from_utf8_lossy(&request_buffer[..bytes_read]);

        let (_, status, body) = responses
          .iter()
          .find(|(pattern, _, _)| request.contains(pattern))
          .unwrap_or_else(|| panic!("unexpected request: {request}"));

        let response = format!(
          "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}",
          body.as_bytes().len(),
          body,
        );
        stream
          .write_all(response.as_bytes())
          .expect("write response");
        stream.flush().expect("flush response");
      }
    });

    (address, handle)
  }

  fn make_authenticated_test_user(role: UserRole) -> User {
    User {
      id: "user_123".to_string(),
      name: "Joris".to_string(),
      email: "joris@example.com".to_string(),
      email_verified: true,
      image: None,
      github_login: Some("joris-gallot".to_string()),
      role,
      subscription: UserSubscription::default(),
    }
  }

  fn make_repository_tree_entry(path: &str) -> crate::api::GithubRepositoryTreeEntry {
    crate::api::GithubRepositoryTreeEntry {
      path: path.to_string(),
      mode: "100644".to_string(),
      entry_type: "blob".to_string(),
      sha: "deadbeef".to_string(),
      size: Some(128),
      url: None,
    }
  }

  fn isolate_config_store_for_test() {
    static NEXT_DB_ID: AtomicU64 = AtomicU64::new(1);
    let id = NEXT_DB_ID.fetch_add(1, Ordering::Relaxed);
    let db_path = std::env::temp_dir().join(format!(
      "reviu-git-page-test-config-{}-{id}.sqlite",
      std::process::id()
    ));
    let _ = std::fs::remove_file(&db_path);
    ConfigStore::set_test_db_path(Some(db_path));
  }

  fn init_gpui_test(cx: &mut TestAppContext) {
    isolate_config_store_for_test();
    cx.update(|cx| {
      gpui_component::init(cx);
      if !cx.has_global::<WorkspaceApi>() {
        cx.set_global(WorkspaceApi::new());
      }
      if !cx.has_global::<AuthStateStore>() {
        cx.set_global(AuthStateStore::default());
      }
      if !cx.has_global::<ActiveLocalRepoStore>() {
        cx.set_global(ActiveLocalRepoStore::default());
      }
      if !cx.has_global::<crate::config::AppSettings>() {
        cx.set_global(crate::config::AppSettings::default());
      }
      ActiveLocalRepoStore::set(cx, None);
    });
  }

  fn add_git_page_window_with_root(
    cx: &mut TestAppContext,
  ) -> (Entity<GitPage>, &mut gpui::VisualTestContext) {
    let mut mounted_git_page: Option<Entity<GitPage>> = None;
    let (_root, cx) = cx.add_window_view(|window, cx| {
      let git_page = cx.new(|cx| GitPage::new_for_test(window, cx));
      mounted_git_page = Some(git_page.clone());
      gpui_component::Root::new(git_page, window, cx)
    });
    let git_page = mounted_git_page.expect("git page");
    (git_page, cx)
  }

  fn tiny_png_bytes() -> Vec<u8> {
    vec![
      137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 4,
      0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 252, 255, 31, 0, 3, 3, 2,
      0, 239, 154, 63, 71, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ]
  }

  #[gpui::test]
  fn init_gpui_test_registers_required_globals(cx: &mut TestAppContext) {
    init_gpui_test(cx);

    cx.update(|cx| {
      assert!(cx.has_global::<WorkspaceApi>());
      assert!(cx.has_global::<AuthStateStore>());
      assert!(cx.has_global::<ActiveLocalRepoStore>());
      assert_eq!(ActiveLocalRepoStore::get(cx), None);
    });
  }

  #[gpui::test]
  async fn git_page_handle_selects_repo_navigates_and_starts_base_merge(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    cx.executor().allow_parking();
    cx.update(|cx| {
      gpui_router::init(cx);
      NavigationHistory::init(cx);
      NavigationHistory::navigate_replace("/github/acme/widget/pull/42", cx);
    });

    let repo = TempRepo::init("git-page-handle-merge-base");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "base\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");
    let _ = commit_text_file(&repo.path, rel_path, "main change\n", "main change");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(&repo.path, rel_path, "feature change\n", "feature change");

    let (git_page, cx) = add_git_page_window_with_root(cx);

    cx.update(|_, cx| {
      GitPageHandle::show_repository_and_merge_base(repo.path.clone(), base_branch.clone(), cx);
    });
    let (pending_open_action, selected_file, has_editor) = git_page.read_with(cx, |this, _cx| {
      (
        this.pending_open_action.clone(),
        this.selected_file.clone(),
        this.editor.is_some(),
      )
    });
    assert_eq!(
      pending_open_action,
      Some(GitPageOpenAction::MergeBaseBranch {
        base_branch_name: base_branch.clone(),
      })
    );
    assert!(GitPage::should_show_open_action_loading_state(
      pending_open_action.as_ref(),
      selected_file.as_deref(),
      has_editor,
    ));
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (selected_repo, merge_in_progress, selected_file) = git_page.read_with(cx, |this, _cx| {
      (
        this.selected_repo.clone(),
        this.merge_in_progress,
        this.selected_file.clone(),
      )
    });
    assert_eq!(selected_repo, Some(repo.path.clone()));
    assert!(
      merge_in_progress,
      "merge state should stay active on conflicts"
    );
    assert_eq!(selected_file, Some(rel_path.to_path_buf()));
    cx.update(|_, cx| {
      assert_eq!(NavigationHistory::current_pathname(cx).as_ref(), "/git");
    });
  }

  #[gpui::test]
  async fn git_page_handle_reopens_active_merge_conflicts_without_resetting_merge_mode(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    cx.executor().allow_parking();
    cx.update(|cx| {
      gpui_router::init(cx);
      NavigationHistory::init(cx);
      NavigationHistory::navigate_replace("/github/acme/widget/pull/42", cx);
    });

    let repo = TempRepo::init("git-page-handle-resume-merge-conflict");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "base\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");
    let _ = commit_text_file(&repo.path, rel_path, "main change\n", "main change");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(&repo.path, rel_path, "feature change\n", "feature change");
    let merge_result = merge_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    );
    assert!(merge_result.is_err(), "merge should stop on conflicts");
    assert!(
      is_merge_in_progress(&repo.path).expect("read merge state"),
      "repo should already be in merge mode before reopening via handle"
    );

    let (git_page, cx) = add_git_page_window_with_root(cx);

    cx.update(|_, cx| {
      GitPageHandle::show_repository_and_merge_base(repo.path.clone(), base_branch.clone(), cx);
    });

    let (merge_in_progress, rebase_in_progress, pending_open_action, selected_file, has_editor) =
      git_page.read_with(cx, |this, _cx| {
        (
          this.merge_in_progress,
          this.rebase_in_progress,
          this.pending_open_action.clone(),
          this.selected_file.clone(),
          this.editor.is_some(),
        )
      });
    assert!(merge_in_progress);
    assert!(!rebase_in_progress);
    assert_eq!(
      pending_open_action,
      Some(GitPageOpenAction::MergeBaseBranch {
        base_branch_name: base_branch.clone(),
      })
    );
    assert!(GitPage::should_show_open_action_loading_state(
      pending_open_action.as_ref(),
      selected_file.as_deref(),
      has_editor,
    ));

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (selected_repo, merge_in_progress, selected_file) = git_page.read_with(cx, |this, _cx| {
      (
        this.selected_repo.clone(),
        this.merge_in_progress,
        this.selected_file.clone(),
      )
    });
    assert_eq!(selected_repo, Some(repo.path.clone()));
    assert!(merge_in_progress);
    assert_eq!(selected_file, Some(rel_path.to_path_buf()));
    cx.update(|_, cx| {
      assert_eq!(NavigationHistory::current_pathname(cx).as_ref(), "/git");
    });
  }

  #[gpui::test]
  async fn git_page_handle_reveals_first_conflict_after_opening_merge_resolution(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    cx.executor().allow_parking();
    cx.update(|cx| {
      gpui_router::init(cx);
      NavigationHistory::init(cx);
      NavigationHistory::navigate_replace("/github/acme/widget/pull/42", cx);
    });

    let repo = TempRepo::init("git-page-handle-reveal-first-conflict");
    let rel_path = Path::new("README.md");
    let build_contents = |replacement: &str| {
      let mut lines = (0..80)
        .map(|line| format!("line {line}"))
        .collect::<Vec<_>>();
      lines[60] = replacement.to_string();
      format!("{}\n", lines.join("\n"))
    };
    let _ = commit_text_file(
      &repo.path,
      rel_path,
      &build_contents("base line"),
      "initial",
    );
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");
    let _ = commit_text_file(
      &repo.path,
      rel_path,
      &build_contents("main change"),
      "main change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(
      &repo.path,
      rel_path,
      &build_contents("feature change"),
      "feature change",
    );

    let (git_page, cx) = add_git_page_window_with_root(cx);

    cx.update(|_, cx| {
      GitPageHandle::show_repository_and_merge_base(repo.path.clone(), base_branch.clone(), cx);
    });
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (selected_file, conflict_navigation, conflict_top, viewport_height) =
      git_page.read_with(cx, |this, cx| {
        let editor = this.editor.as_ref().expect("editor should exist").read(cx);
        let conflict_navigation = this
          .editor_conflict_navigation_state(cx)
          .expect("conflict navigation state");
        let display_line = editor
          .first_display_line_for_conflict(conflict_navigation.active_start_line)
          .expect("display line for conflict");

        (
          this.selected_file.clone(),
          conflict_navigation,
          GitPage::hunk_action_top(
            editor.measured_editor_line_height(),
            display_line,
            editor.scroll_offset_y,
          ),
          editor.viewport_height,
        )
      });

    assert_eq!(selected_file, Some(rel_path.to_path_buf()));
    assert_eq!(conflict_navigation.active_index, 0);
    assert_eq!(conflict_navigation.total, 1);
    assert!(conflict_navigation.active_start_line >= 60);
    assert!(
      conflict_top < viewport_height,
      "expected first conflict to be visible after opening merge resolution"
    );
  }

  #[gpui::test]
  async fn git_page_handle_reveals_first_conflict_when_file_has_multiple_conflicts(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    cx.executor().allow_parking();
    cx.update(|cx| {
      gpui_router::init(cx);
      NavigationHistory::init(cx);
      NavigationHistory::navigate_replace("/github/acme/widget/pull/42", cx);
    });

    let repo = TempRepo::init("git-page-handle-reveal-multi-conflict");
    let rel_path = Path::new("README.md");
    let build_contents = |replacement_prefix: &str| {
      let mut lines = (0..160)
        .map(|line| format!("line {line}"))
        .collect::<Vec<_>>();
      for target_line in [20usize, 55, 90, 125] {
        lines[target_line] = format!("{replacement_prefix} {target_line}");
      }
      format!("{}\n", lines.join("\n"))
    };
    let _ = commit_text_file(&repo.path, rel_path, &build_contents("base"), "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");
    let _ = commit_text_file(&repo.path, rel_path, &build_contents("main"), "main change");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(
      &repo.path,
      rel_path,
      &build_contents("feature"),
      "feature change",
    );

    let (git_page, cx) = add_git_page_window_with_root(cx);

    cx.update(|_, cx| {
      GitPageHandle::show_repository_and_merge_base(repo.path.clone(), base_branch.clone(), cx);
    });
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (selected_file, first_conflict_top, viewport_height, conflict_navigation) = git_page
      .read_with(cx, |this, cx| {
        let editor = this.editor.as_ref().expect("editor should exist").read(cx);
        let conflict_navigation = this
          .editor_conflict_navigation_state(cx)
          .expect("conflict navigation state");
        let first_display_line = editor
          .first_display_line_for_conflict(20)
          .expect("first conflict display line");

        (
          this.selected_file.clone(),
          GitPage::hunk_action_top(
            editor.measured_editor_line_height(),
            first_display_line,
            editor.scroll_offset_y,
          ),
          editor.viewport_height,
          conflict_navigation,
        )
      });

    assert_eq!(selected_file, Some(rel_path.to_path_buf()));
    assert_eq!(conflict_navigation.active_index, 0);
    assert_eq!(conflict_navigation.total, 4);
    assert_eq!(conflict_navigation.active_start_line, 20);
    assert!(
      first_conflict_top >= px(0.0) && first_conflict_top < viewport_height,
      "expected first conflict to remain visible after diff projection settles"
    );
  }

  #[test]
  fn github_branch_context_from_active_repo_requires_repo_and_named_branch() {
    let ready = ActiveLocalRepo {
      repo_root: PathBuf::from("/tmp/repo"),
      github_owner: Some("acme".to_string()),
      github_repo: Some("widget".to_string()),
      current_branch: Some("feature/parser".to_string()),
      head_sha: Some("head".to_string()),
      has_uncommitted_changes: false,
    };
    assert_eq!(
      GitPage::github_branch_context_from_active_repo(&ready),
      Some(GithubBranchContext {
        owner: "acme".to_string(),
        repo: "widget".to_string(),
        branch: "feature/parser".to_string(),
      })
    );

    let detached = ActiveLocalRepo {
      current_branch: Some("HEAD".to_string()),
      ..ready.clone()
    };
    assert_eq!(
      GitPage::github_branch_context_from_active_repo(&detached),
      None
    );

    let missing_remote = ActiveLocalRepo {
      github_owner: None,
      ..ready
    };
    assert_eq!(
      GitPage::github_branch_context_from_active_repo(&missing_remote),
      None
    );
  }

  #[test]
  fn branch_pr_button_state_prefers_open_existing_pull_request() {
    let context = GithubBranchContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      branch: "feature/parser".to_string(),
    };
    let pull_request = make_branch_pull_request(42);

    assert_eq!(
      GitPage::branch_pr_button_state(
        Some(&context),
        true,
        true,
        false,
        false,
        Some(&pull_request)
      ),
      GitBranchPullRequestButtonState::OpenExisting {
        owner: "acme".to_string(),
        repo: "widget".to_string(),
        number: 42,
      }
    );
  }

  #[test]
  fn branch_pr_button_state_shows_loading_only_when_in_app_lookup_is_available() {
    let context = GithubBranchContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      branch: "feature/parser".to_string(),
    };

    assert_eq!(
      GitPage::branch_pr_button_state(Some(&context), true, true, false, true, None),
      GitBranchPullRequestButtonState::Checking
    );
    assert_eq!(
      GitPage::branch_pr_button_state(Some(&context), false, true, false, true, None),
      GitBranchPullRequestButtonState::Hidden
    );
  }

  #[test]
  fn branch_pr_button_state_hides_branch_pull_request_button_without_github_access() {
    let context = GithubBranchContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      branch: "feature/parser".to_string(),
    };
    let pull_request = make_branch_pull_request(42);

    assert_eq!(
      GitPage::branch_pr_button_state(
        Some(&context),
        false,
        true,
        false,
        false,
        Some(&pull_request),
      ),
      GitBranchPullRequestButtonState::Hidden
    );
  }

  #[test]
  fn branch_pr_button_state_shows_create_when_branch_has_no_open_pull_request() {
    let context = GithubBranchContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      branch: "feature/parser".to_string(),
    };

    assert_eq!(
      GitPage::branch_pr_button_state(Some(&context), true, true, false, false, None),
      GitBranchPullRequestButtonState::Create
    );
  }

  #[test]
  fn should_apply_created_pull_request_only_for_matching_branch_context() {
    let active_context = GithubBranchContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      branch: "feature/parser".to_string(),
    };
    let other_context = GithubBranchContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      branch: "feature/other".to_string(),
    };

    assert!(GitPage::should_apply_created_pull_request(
      Some(&active_context),
      &active_context
    ));
    assert!(!GitPage::should_apply_created_pull_request(
      Some(&active_context),
      &other_context
    ));
    assert!(!GitPage::should_apply_created_pull_request(
      None,
      &active_context
    ));
  }

  #[test]
  fn branch_pr_button_state_shows_publish_and_create_for_unpublished_branch() {
    let context = GithubBranchContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      branch: "feature/parser".to_string(),
    };

    assert_eq!(
      GitPage::branch_pr_button_state(Some(&context), true, false, true, false, None),
      GitBranchPullRequestButtonState::PublishAndCreate
    );
  }

  #[test]
  fn branch_pr_button_state_hides_publish_and_create_without_unique_branch_commits() {
    let context = GithubBranchContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      branch: "feature/parser".to_string(),
    };

    assert_eq!(
      GitPage::branch_pr_button_state(Some(&context), true, false, false, false, None),
      GitBranchPullRequestButtonState::Hidden
    );
  }

  #[test]
  fn branch_has_github_upstream_requires_named_branch_with_upstream() {
    let published = make_branch_status("feature/parser", 0, 0, true);
    let local_only = make_branch_status("feature/parser", 0, 0, false);
    let detached = make_branch_status("HEAD", 0, 0, true);

    assert!(GitPage::branch_has_github_upstream(Some(&published)));
    assert!(!GitPage::branch_has_github_upstream(Some(&local_only)));
    assert!(!GitPage::branch_has_github_upstream(Some(&detached)));
    assert!(!GitPage::branch_has_github_upstream(None));
  }

  #[test]
  fn resolve_pull_request_template_paths_prefers_documented_single_template_locations() {
    let entries = vec![
      make_repository_tree_entry("docs/pull_request_template.md"),
      make_repository_tree_entry("pull_request_template.md"),
      make_repository_tree_entry(".github/pull_request_template.md"),
    ];

    assert_eq!(
      resolve_pull_request_template_paths(&entries),
      vec![
        ".github/pull_request_template.md".to_string(),
        "pull_request_template.md".to_string(),
        "docs/pull_request_template.md".to_string(),
      ]
    );
  }

  #[test]
  fn resolve_pull_request_template_paths_collects_direct_children_from_template_directories() {
    let entries = vec![
      make_repository_tree_entry(".github/PULL_REQUEST_TEMPLATE/bugfix.md"),
      make_repository_tree_entry(".github/PULL_REQUEST_TEMPLATE/feature.md"),
      make_repository_tree_entry(".github/PULL_REQUEST_TEMPLATE/nested/mobile/template.md"),
      make_repository_tree_entry("docs/PULL_REQUEST_TEMPLATE/release.md"),
    ];

    assert_eq!(
      resolve_pull_request_template_paths(&entries),
      vec![
        ".github/PULL_REQUEST_TEMPLATE/bugfix.md".to_string(),
        ".github/PULL_REQUEST_TEMPLATE/feature.md".to_string(),
        "docs/PULL_REQUEST_TEMPLATE/release.md".to_string(),
      ]
    );
  }

  #[gpui::test]
  fn command_palette_create_pull_request_follows_branch_button_visibility_rules(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-command-palette-create-pr");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "initial\n", "initial");

    cx.update(|cx| {
      AuthStateStore::set(
        cx,
        AuthState::Authenticated(Box::new(make_authenticated_test_user(UserRole::Pro))),
      );
      ActiveLocalRepoStore::set(
        cx,
        Some(ActiveLocalRepo {
          repo_root: repo.path.clone(),
          github_owner: Some("acme".to_string()),
          github_repo: Some("widget".to_string()),
          current_branch: Some("main".to_string()),
          head_sha: Some("deadbeef".to_string()),
          has_uncommitted_changes: false,
        }),
      );
    });

    let (git_page, cx) = add_git_page_window_with_root(cx);

    let commands = git_page.update(cx, |this, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.branch_status = Some(make_branch_status("main", 0, 0, true));
      this.branch_pr_lookup_loading = false;
      this.branch_pr_lookup_result = None;
      this.build_command_palette_contents(1, cx).commands
    });
    assert!(
      commands
        .iter()
        .any(|command| command.id == CommandPaletteCommandId::CreatePullRequest)
    );
    assert!(
      !commands
        .iter()
        .any(|command| command.id == CommandPaletteCommandId::OpenPullRequest)
    );

    let commands_with_existing_pr = git_page.update(cx, |this, cx| {
      this.branch_pr_lookup_result = Some(make_branch_pull_request(42));
      this.build_command_palette_contents(1, cx).commands
    });
    assert!(
      !commands_with_existing_pr
        .iter()
        .any(|command| command.id == CommandPaletteCommandId::CreatePullRequest)
    );
    let open_pr_command = commands_with_existing_pr
      .iter()
      .find(|command| command.id == CommandPaletteCommandId::OpenPullRequest)
      .expect("existing branch PR should add an open command");
    assert_eq!(open_pr_command.name.as_ref(), "Open PR #42");
  }

  #[gpui::test]
  async fn refresh_branch_pr_lookup_skips_lookup_without_github_access(cx: &mut TestAppContext) {
    init_gpui_test(cx);

    cx.update(|cx| {
      AuthStateStore::set(
        cx,
        AuthState::Authenticated(Box::new(make_authenticated_test_user(UserRole::User))),
      );
      ActiveLocalRepoStore::set(
        cx,
        Some(ActiveLocalRepo {
          repo_root: PathBuf::from("/tmp/reviu-git-page-branch-pr-no-access"),
          github_owner: Some("acme".to_string()),
          github_repo: Some("widget".to_string()),
          current_branch: Some("feature/parser".to_string()),
          head_sha: Some("deadbeef".to_string()),
          has_uncommitted_changes: false,
        }),
      );
    });

    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update_in(cx, |this, _window, cx| {
      this.refresh_branch_pr_lookup(cx);
    });
    await_git_page_background_tasks(git_page.clone(), cx).await;

    git_page.read_with(cx, |this, cx| {
      assert_eq!(this.branch_pr_lookup_context, None);
      assert!(this.branch_pr_lookup_task.is_none());
      assert!(!this.branch_pr_lookup_loading);
      assert!(this.branch_pr_lookup_result.is_none());
      assert!(!AuthStateStore::has_github_access(cx));
    });
  }

  #[gpui::test]
  async fn refresh_branch_pr_lookup_skips_lookup_for_unpublished_branch(cx: &mut TestAppContext) {
    init_gpui_test(cx);

    cx.update(|cx| {
      AuthStateStore::set(
        cx,
        AuthState::Authenticated(Box::new(make_authenticated_test_user(UserRole::Pro))),
      );
      ActiveLocalRepoStore::set(
        cx,
        Some(ActiveLocalRepo {
          repo_root: PathBuf::from("/tmp/reviu-git-page-branch-pr-unpublished"),
          github_owner: Some("acme".to_string()),
          github_repo: Some("widget".to_string()),
          current_branch: Some("feature/parser".to_string()),
          head_sha: Some("deadbeef".to_string()),
          has_uncommitted_changes: false,
        }),
      );
    });

    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update_in(cx, |this, _window, cx| {
      this.branch_status = Some(make_branch_status("feature/parser", 0, 0, false));
      this.refresh_branch_pr_lookup(cx);
    });
    await_git_page_background_tasks(git_page.clone(), cx).await;

    git_page.read_with(cx, |this, cx| {
      assert_eq!(this.branch_pr_lookup_context, None);
      assert!(this.branch_pr_lookup_task.is_none());
      assert!(!this.branch_pr_lookup_loading);
      assert!(this.branch_pr_lookup_result.is_none());
      assert!(AuthStateStore::has_github_access(cx));
      assert!(!GitPage::branch_has_github_upstream(
        this.branch_status.as_ref()
      ));
    });
  }

  #[gpui::test]
  fn apply_created_pull_request_updates_branch_pr_lookup_for_matching_context(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo_root = PathBuf::from("/tmp/reviu-git-page-created-pr");
    let branch_context = GithubBranchContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      branch: "feature/parser".to_string(),
    };
    let pull_request = make_branch_pull_request(42);

    cx.update(|cx| {
      AuthStateStore::set(
        cx,
        AuthState::Authenticated(Box::new(make_authenticated_test_user(UserRole::Pro))),
      );
      ActiveLocalRepoStore::set(
        cx,
        Some(ActiveLocalRepo {
          repo_root: repo_root.clone(),
          github_owner: Some(branch_context.owner.clone()),
          github_repo: Some(branch_context.repo.clone()),
          current_branch: Some(branch_context.branch.clone()),
          head_sha: Some("deadbeef".to_string()),
          has_uncommitted_changes: false,
        }),
      );
    });

    let (git_page, cx) = add_git_page_window_with_root(cx);

    let create_pr_hidden = git_page.update(cx, |this, cx| {
      this.selected_repo = Some(repo_root.clone());
      this.branch_status = Some(make_branch_status(
        branch_context.branch.as_str(),
        0,
        0,
        true,
      ));
      this.branch_pr_lookup_loading = true;
      this.apply_created_pull_request(&branch_context, &pull_request, cx);
      !this
        .build_command_palette_contents(1, cx)
        .commands
        .into_iter()
        .any(|command| command.id == CommandPaletteCommandId::CreatePullRequest)
    });

    assert!(create_pr_hidden);
    git_page.read_with(cx, |this, _cx| {
      assert_eq!(
        this.branch_pr_lookup_context.as_ref(),
        Some(&branch_context)
      );
      assert_eq!(
        this
          .branch_pr_lookup_result
          .as_ref()
          .map(|pull_request| pull_request.number),
        Some(pull_request.number)
      );
      assert!(!this.branch_pr_lookup_loading);
      assert!(this.branch_pr_lookup_task.is_none());
    });
  }

  #[gpui::test]
  fn apply_created_pull_request_ignores_stale_branch_context(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo_root = PathBuf::from("/tmp/reviu-git-page-created-pr-stale");
    let active_context = GithubBranchContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      branch: "feature/parser".to_string(),
    };
    let stale_context = GithubBranchContext {
      owner: "acme".to_string(),
      repo: "widget".to_string(),
      branch: "feature/other".to_string(),
    };
    let pull_request = make_branch_pull_request(42);

    cx.update(|cx| {
      AuthStateStore::set(
        cx,
        AuthState::Authenticated(Box::new(make_authenticated_test_user(UserRole::Pro))),
      );
      ActiveLocalRepoStore::set(
        cx,
        Some(ActiveLocalRepo {
          repo_root: repo_root.clone(),
          github_owner: Some(active_context.owner.clone()),
          github_repo: Some(active_context.repo.clone()),
          current_branch: Some(active_context.branch.clone()),
          head_sha: Some("deadbeef".to_string()),
          has_uncommitted_changes: false,
        }),
      );
    });

    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update(cx, |this, cx| {
      this.selected_repo = Some(repo_root.clone());
      this.branch_status = Some(make_branch_status(
        active_context.branch.as_str(),
        0,
        0,
        true,
      ));
      this.branch_pr_lookup_loading = true;
      this.apply_created_pull_request(&stale_context, &pull_request, cx);
    });

    git_page.read_with(cx, |this, _cx| {
      assert!(this.branch_pr_lookup_context.is_none());
      assert!(this.branch_pr_lookup_result.is_none());
      assert!(this.branch_pr_lookup_loading);
      assert!(this.branch_pr_lookup_task.is_none());
    });
  }

  fn seed_repo_branch_state(this: &mut GitPage, repo_root: &Path, cx: &mut Context<GitPage>) {
    this.selected_repo = Some(repo_root.to_path_buf());
    let branch_status = current_branch_status(repo_root).expect("read initial branch status");
    let selected = GitPage::selected_branch_from_status(Some(&branch_status));
    let detached_label = if GitPage::is_detached_head(Some(&branch_status)) {
      detached_head_label(repo_root).ok()
    } else {
      None
    };
    let items = GitPage::branch_select_items(
      list_branches(repo_root).expect("list branches"),
      selected.as_ref(),
      detached_label.as_deref(),
    );
    this.branch_status = Some(branch_status);
    this.branch_dropdown_items = items;
    cx.notify();
  }

  fn selected_branch_from_dropdown(this: &GitPage) -> Option<BranchRef> {
    this
      .branch_dropdown_items
      .iter()
      .find(|item| item.is_current)
      .map(|item| item.branch.clone())
  }

  async fn await_git_page_background_tasks(
    git_page: Entity<GitPage>,
    cx: &mut gpui::VisualTestContext,
  ) {
    loop {
      let (
        branch_pr_lookup_task,
        open_file_task,
        status_task,
        branch_task,
        history_task,
        history_files_task,
        history_open_file_task,
      ) = git_page.update_in(cx, |this, _window, _| {
        (
          this.branch_pr_lookup_task.take(),
          this.open_file_task.take(),
          this.status_task.take(),
          this.branch_task.take(),
          this.history_task.take(),
          this.history_files_task.take(),
          this.history_open_file_task.take(),
        )
      });
      let editor = git_page.read_with(cx, |this, _| this.editor.clone());
      let (editor_bases_task, editor_diff_task) = if let Some(editor) = editor {
        editor.update(cx, |editor, _| {
          (editor.bases_task.take(), editor.diff_task.take())
        })
      } else {
        (None, None)
      };

      let mut had_task = false;
      if let Some(task) = branch_pr_lookup_task {
        had_task = true;
        task.await;
      }
      if let Some(task) = open_file_task {
        had_task = true;
        task.await;
      }
      if let Some(task) = status_task {
        had_task = true;
        task.await;
      }
      if let Some(task) = branch_task {
        had_task = true;
        task.await;
      }
      if let Some(task) = history_task {
        had_task = true;
        task.await;
      }
      if let Some(task) = history_files_task {
        had_task = true;
        task.await;
      }
      if let Some(task) = history_open_file_task {
        had_task = true;
        task.await;
      }
      if let Some(task) = editor_bases_task {
        had_task = true;
        task.await;
      }
      if let Some(task) = editor_diff_task {
        had_task = true;
        task.await;
      }

      if !had_task {
        break;
      }
    }
  }

  #[test]
  fn build_history_tree_items_marks_selected_commit_expanded() {
    let commits = vec![
      make_commit("c3", &["c2"]),
      make_commit("c2", &["c1"]),
      make_commit("c1", &[]),
    ];
    let rows = GitPage::build_history_rows(&commits);

    let mut files_by_commit = HashMap::new();
    files_by_commit.insert(
      "c2".to_string(),
      vec![make_history_file(
        "src/c2.rs",
        CommitFileChangeKind::Modified,
      )],
    );
    files_by_commit.insert(
      "c1".to_string(),
      vec![make_history_file("src/c1.rs", CommitFileChangeKind::Added)],
    );

    let loading = HashSet::new();
    let expanded = HashSet::from(["c2".to_string()]);
    let (items, _) =
      GitPage::build_history_tree_items(&rows, &files_by_commit, &loading, &expanded);

    assert!(!items[0].is_expanded());
    assert!(items[1].is_expanded());
    assert!(!items[2].is_expanded());
  }

  #[test]
  fn build_history_tree_items_supports_multiple_expanded_commits() {
    let commits = vec![
      make_commit("c3", &["c2"]),
      make_commit("c2", &["c1"]),
      make_commit("c1", &[]),
    ];
    let rows = GitPage::build_history_rows(&commits);
    let files_by_commit = HashMap::new();
    let loading = HashSet::new();
    let expanded = HashSet::from(["c3".to_string(), "c1".to_string()]);

    let (items, _) =
      GitPage::build_history_tree_items(&rows, &files_by_commit, &loading, &expanded);

    assert!(items[0].is_expanded());
    assert!(!items[1].is_expanded());
    assert!(items[2].is_expanded());
  }

  #[test]
  fn build_history_tree_items_includes_commit_and_file_nodes() {
    let commits = vec![make_commit("c2", &["c1"]), make_commit("c1", &[])];
    let rows = GitPage::build_history_rows(&commits);
    let mut files_by_commit = HashMap::new();
    files_by_commit.insert(
      "c2".to_string(),
      vec![make_history_file(
        "src/main.rs",
        CommitFileChangeKind::Modified,
      )],
    );
    let loading = HashSet::new();
    let expanded = HashSet::from(["c2".to_string()]);

    let (items, nodes) =
      GitPage::build_history_tree_items(&rows, &files_by_commit, &loading, &expanded);
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].children.len(), 1);
    assert_eq!(items[0].children[0].label.as_ref(), "src/main.rs");

    let commit_id = format!("history-commit:{}", rows[0].commit.oid);
    assert!(matches!(
      nodes.get(&commit_id),
      Some(HistoryTreeNode::Commit { oid }) if oid == "c2"
    ));

    let file_id = format!("history-file:{}:{}", rows[0].commit.oid, 0);
    assert!(matches!(
      nodes.get(&file_id),
      Some(HistoryTreeNode::File { commit_oid, .. }) if commit_oid == "c2"
    ));
  }

  #[test]
  fn build_history_tree_items_uses_loading_placeholder() {
    let commits = vec![make_commit("c1", &[])];
    let rows = GitPage::build_history_rows(&commits);
    let files_by_commit = HashMap::new();
    let loading = HashSet::from(["c1".to_string()]);
    let expanded = HashSet::from(["c1".to_string()]);

    let (items, nodes) =
      GitPage::build_history_tree_items(&rows, &files_by_commit, &loading, &expanded);
    assert_eq!(items[0].children.len(), 1);
    assert_eq!(items[0].children[0].label.as_ref(), "Loading files...");
    assert!(matches!(
      nodes.get("history-loading:c1"),
      Some(HistoryTreeNode::Placeholder)
    ));
  }

  #[test]
  fn should_refresh_file_list_only_in_changes_mode() {
    assert!(GitPage::should_refresh_file_list(GitSidebarMode::Changes));
    assert!(!GitPage::should_refresh_file_list(GitSidebarMode::History));
  }

  #[test]
  fn should_refresh_history_for_poll_when_history_empty() {
    assert!(GitPage::should_refresh_history_for_poll(
      true,
      true,
      Some(&make_history_revision("a")),
      Some(&make_history_revision("a"))
    ));
  }

  #[test]
  fn should_not_refresh_history_for_poll_when_revision_unchanged() {
    let revision = make_history_revision("a");
    assert!(!GitPage::should_refresh_history_for_poll(
      true,
      false,
      Some(&revision),
      Some(&revision)
    ));
  }

  #[test]
  fn should_refresh_history_for_poll_when_revision_changed() {
    let cached = make_history_revision("a");
    let current = make_history_revision("b");
    assert!(GitPage::should_refresh_history_for_poll(
      true,
      false,
      Some(&cached),
      Some(&current)
    ));
  }

  #[test]
  fn should_not_refresh_history_for_poll_when_history_not_included() {
    assert!(!GitPage::should_refresh_history_for_poll(
      false,
      true,
      Some(&make_history_revision("a")),
      Some(&make_history_revision("b"))
    ));
  }

  #[test]
  fn should_not_refresh_history_for_poll_when_revision_unavailable() {
    assert!(!GitPage::should_refresh_history_for_poll(
      true,
      false,
      Some(&make_history_revision("a")),
      None
    ));
  }

  #[test]
  fn branch_name_changed_detects_name_transitions() {
    let main = make_branch_status("main", 0, 0, true);
    let feature = make_branch_status("feature", 0, 0, true);

    assert!(GitPage::branch_name_changed(None, Some(&main)));
    assert!(GitPage::branch_name_changed(Some(&main), Some(&feature)));
    assert!(GitPage::branch_name_changed(Some(&main), None));
    assert!(!GitPage::branch_name_changed(None, None));
    assert!(!GitPage::branch_name_changed(Some(&main), Some(&main)));
  }

  #[test]
  fn push_flags_respect_upstream_and_divergence() {
    let no_upstream = make_branch_status("main", 3, 0, false);
    assert_eq!(
      GitPage::push_flags(Some(&no_upstream), false, false),
      (false, false)
    );
    assert_eq!(
      GitPage::push_flags(Some(&no_upstream), true, false),
      (true, false)
    );

    let clean_ahead = make_branch_status("main", 2, 0, true);
    assert_eq!(
      GitPage::push_flags(Some(&clean_ahead), true, false),
      (true, false)
    );

    let diverged = make_branch_status("main", 1, 2, true);
    assert_eq!(
      GitPage::push_flags(Some(&diverged), true, false),
      (false, true)
    );

    let behind_only = make_branch_status("main", 0, 2, true);
    assert_eq!(
      GitPage::push_flags(Some(&behind_only), true, false),
      (false, false)
    );
  }

  #[test]
  fn push_flags_require_force_push_after_rebase_for_tracked_branch() {
    let clean_ahead = make_branch_status("main", 2, 0, true);
    assert_eq!(
      GitPage::push_flags(Some(&clean_ahead), true, true),
      (false, true)
    );
    assert_eq!(
      GitPage::push_flags(Some(&clean_ahead), true, false),
      (true, false)
    );

    let no_ahead = make_branch_status("main", 0, 0, true);
    assert_eq!(
      GitPage::push_flags(Some(&no_ahead), true, true),
      (false, false)
    );
  }

  #[test]
  fn push_action_label_mentions_publish_branch_without_upstream() {
    let no_upstream = make_branch_status("feature", 0, 0, false);
    assert_eq!(
      GitPage::push_action_label(Some(&no_upstream), true),
      "Push (Publish branch)"
    );
    assert_eq!(
      GitPage::push_action_label(Some(&no_upstream), false),
      "Push"
    );

    let tracked = make_branch_status("main", 1, 0, true);
    assert_eq!(GitPage::push_action_label(Some(&tracked), true), "Push");
    let detached = make_branch_status("HEAD", 0, 0, false);
    assert_eq!(GitPage::push_action_label(Some(&detached), true), "Push");
    assert_eq!(GitPage::push_action_label(None, true), "Push");
  }

  #[test]
  fn commit_primary_button_state_prefers_publish_branch_only_for_clean_publishable_branch() {
    assert_eq!(
      GitPage::commit_primary_button_state(false, false, true),
      GitCommitPrimaryButtonState::PublishBranch
    );
    assert_eq!(
      GitPage::commit_primary_button_state(false, true, true),
      GitCommitPrimaryButtonState::Commit
    );
    assert_eq!(
      GitPage::commit_primary_button_state(false, false, false),
      GitCommitPrimaryButtonState::Commit
    );
    assert_eq!(
      GitPage::commit_primary_button_state(true, false, true),
      GitCommitPrimaryButtonState::ContinueRebase
    );
  }

  #[gpui::test]
  fn palette_commit_and_rebase_commands_follow_commit_button_rules(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-palette-commit-rules");
    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update_in(cx, |this, _window, _cx| {
      this.selected_repo = Some(repo.path.clone());
      this.branch_status = Some(make_branch_status("main", 0, 0, true));
      this.rebase_in_progress = false;
      this.status_entries = vec![make_status_entry("README.md", RepoStage::Unstaged)];
      this.selected_file = Some(PathBuf::from("README.md"));
      this.has_head_commit = true;
      this.can_undo_last_commit = true;
      this.can_push = true;
      this.can_force_push = true;
      assert!(this.should_show_commit_palette_command("feat: commit"));
      assert!(!this.should_show_commit_palette_command("   "));
      assert!(!this.should_show_continue_rebase_palette_command());
      assert!(!this.should_show_skip_rebase_palette_command());
      assert!(this.should_show_push_palette_command());
      assert!(this.should_show_force_push_palette_command());
      assert!(this.should_show_undo_last_commit_palette_command());
      assert!(this.should_show_amend_palette_command());
      assert!(this.should_show_checkout_detached_palette_command());
      assert!(!this.should_show_interactive_rebase_palette_command());
      assert!(this.should_show_stage_selected_file_palette_command());
      assert!(!this.should_show_unstage_selected_file_palette_command());

      this.status_entries = vec![make_status_entry("README.md", RepoStage::Staged)];
      assert!(!this.should_show_stage_selected_file_palette_command());
      assert!(this.should_show_unstage_selected_file_palette_command());

      this.status_entries = vec![make_status_entry("README.md", RepoStage::PartiallyStaged)];
      assert!(!this.should_show_stage_selected_file_palette_command());
      assert!(this.should_show_unstage_selected_file_palette_command());

      this.status_entries.clear();
      assert!(this.should_show_checkout_detached_palette_command());
      assert!(this.should_show_interactive_rebase_palette_command());
      this.branch_status = Some(make_branch_status("HEAD", 0, 0, false));
      assert!(!this.should_show_checkout_detached_palette_command());
      assert!(!this.should_show_interactive_rebase_palette_command());
      this.branch_status = Some(make_branch_status("main", 0, 0, true));

      this.selected_file = None;
      assert!(!this.should_show_stage_selected_file_palette_command());
      assert!(!this.should_show_unstage_selected_file_palette_command());

      this.rebase_in_progress = true;
      assert!(!this.should_show_commit_palette_command("feat: commit"));
      assert!(this.should_show_continue_rebase_palette_command());
      assert!(this.should_show_skip_rebase_palette_command());
      assert!(!this.should_show_push_palette_command());
      assert!(!this.should_show_force_push_palette_command());
      assert!(!this.should_show_undo_last_commit_palette_command());
      assert!(!this.should_show_amend_palette_command());
      assert!(!this.should_show_checkout_detached_palette_command());
      assert!(!this.should_show_interactive_rebase_palette_command());
      assert!(!this.should_show_stage_selected_file_palette_command());
      assert!(!this.should_show_unstage_selected_file_palette_command());

      this.status_entries = vec![RepoStatusEntry {
        path: PathBuf::from("README.md"),
        old_path: None,
        status: RepoStatusKind::Conflicted,
        stage: RepoStage::Unstaged,
      }];
      assert!(!this.should_show_continue_rebase_palette_command());
      assert!(this.should_show_skip_rebase_palette_command());

      this.rebase_in_progress = false;
      this.merge_in_progress = true;
      this.status_entries = vec![RepoStatusEntry {
        path: PathBuf::from("README.md"),
        old_path: None,
        status: RepoStatusKind::Conflicted,
        stage: RepoStage::Unstaged,
      }];
      assert!(!this.should_show_commit_palette_command("Merge branch 'feature' into main"));

      this.status_entries = vec![RepoStatusEntry {
        path: PathBuf::from("README.md"),
        old_path: None,
        status: RepoStatusKind::Modified,
        stage: RepoStage::Staged,
      }];
      assert!(this.should_show_commit_palette_command("Merge branch 'feature' into main"));

      this.merge_in_progress = false;
      this.selected_repo = None;
      assert!(!this.should_show_push_palette_command());
      assert!(!this.should_show_force_push_palette_command());
      assert!(!this.should_show_undo_last_commit_palette_command());
      assert!(!this.should_show_amend_palette_command());
      assert!(!this.should_show_checkout_detached_palette_command());
      assert!(!this.should_show_interactive_rebase_palette_command());
      assert!(!this.should_show_stage_selected_file_palette_command());
      assert!(!this.should_show_unstage_selected_file_palette_command());
    });
  }

  #[gpui::test]
  fn command_palette_keeps_temporarily_blocked_commands_visible_with_reasons(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-palette-disabled-reasons");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "initial\n", "initial");
    std::fs::write(repo.path.join(rel_path), "changed\n").expect("modify tracked file");
    let (git_page, cx) = add_git_page_window_with_root(cx);

    let dirty_worktree_reason = git_page.update(cx, |this, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.branch_status = Some(make_branch_status("main", 0, 0, true));
      this.has_head_commit = true;
      this.status_entries = list_repo_status(&repo.path).expect("list status");
      this
        .build_command_palette_contents(1, cx)
        .commands
        .into_iter()
        .find(|command| command.id == CommandPaletteCommandId::InteractiveRebase)
        .and_then(|command| command.disabled_reason)
    });
    assert_eq!(
      dirty_worktree_reason.as_ref().map(|reason| reason.as_ref()),
      Some("Commit or stash worktree changes first")
    );

    let rebase_continue_reason = git_page.update(cx, |this, cx| {
      this.rebase_in_progress = true;
      this.status_entries = vec![RepoStatusEntry {
        path: rel_path.to_path_buf(),
        old_path: None,
        status: RepoStatusKind::Conflicted,
        stage: RepoStage::Unstaged,
      }];
      this
        .build_command_palette_contents(1, cx)
        .commands
        .into_iter()
        .find(|command| command.id == CommandPaletteCommandId::ContinueRebase)
        .and_then(|command| command.disabled_reason)
    });
    assert_eq!(
      rebase_continue_reason
        .as_ref()
        .map(|reason| reason.as_ref()),
      Some("Resolve and stage conflicts first")
    );
  }

  #[test]
  fn accept_all_conflict_command_rules_match_editor_header_rules() {
    assert!(GitPage::can_accept_all_conflicts(
      Some(RepoStatusKind::Conflicted),
      false,
      true,
    ));
    assert!(!GitPage::can_accept_all_conflicts(
      Some(RepoStatusKind::Conflicted),
      true,
      true,
    ));
    assert!(!GitPage::can_accept_all_conflicts(
      Some(RepoStatusKind::Conflicted),
      false,
      false,
    ));
    assert!(!GitPage::can_accept_all_conflicts(
      Some(RepoStatusKind::Modified),
      false,
      true,
    ));
    assert!(!GitPage::can_accept_all_conflicts(None, false, true));
  }

  #[test]
  fn can_navigate_annotations_requires_multiple_annotations() {
    assert!(!GitPage::can_navigate_annotations(None));
    assert!(!GitPage::can_navigate_annotations(Some(
      AnnotationNavigationState {
        active_index: 0,
        total: 1,
        kind: AnnotationKind::Conflict,
      }
    )));
    assert!(GitPage::can_navigate_annotations(Some(
      AnnotationNavigationState {
        active_index: 0,
        total: 2,
        kind: AnnotationKind::Conflict,
      }
    )));
    assert!(GitPage::can_navigate_annotations(Some(
      AnnotationNavigationState {
        active_index: 0,
        total: 3,
        kind: AnnotationKind::Change,
      }
    )));
  }

  fn make_status_entry(path: &str, stage: RepoStage) -> RepoStatusEntry {
    RepoStatusEntry {
      path: PathBuf::from(path),
      old_path: None,
      status: RepoStatusKind::Modified,
      stage,
    }
  }

  #[test]
  fn has_staged_changes_detects_staged_and_partial_entries() {
    assert!(!GitPage::has_staged_changes(&[
      make_status_entry("src/a.rs", RepoStage::Unstaged),
      make_status_entry("src/b.rs", RepoStage::Unstaged),
    ]));
    assert!(GitPage::has_staged_changes(&[
      make_status_entry("src/a.rs", RepoStage::Staged),
      make_status_entry("src/b.rs", RepoStage::Unstaged),
    ]));
    assert!(GitPage::has_staged_changes(&[make_status_entry(
      "src/a.rs",
      RepoStage::PartiallyStaged
    )]));
  }

  #[test]
  fn changed_files_count_matches_status_entries_len() {
    assert_eq!(GitPage::changed_files_count(&[]), 0);

    let entries = vec![
      make_status_entry("src/a.rs", RepoStage::Unstaged),
      make_status_entry("src/b.rs", RepoStage::Staged),
      make_status_entry("src/c.rs", RepoStage::PartiallyStaged),
    ];
    assert_eq!(GitPage::changed_files_count(&entries), 3);
  }

  #[test]
  fn has_conflicted_entries_detects_conflict_status() {
    let clean_entries = vec![
      make_status_entry("src/a.rs", RepoStage::Unstaged),
      make_status_entry("src/b.rs", RepoStage::Staged),
    ];
    assert!(!GitPage::has_conflicted_entries(&clean_entries));

    let mut conflicted_entries = clean_entries;
    conflicted_entries.push(RepoStatusEntry {
      path: PathBuf::from("src/conflict.rs"),
      old_path: None,
      status: RepoStatusKind::Conflicted,
      stage: RepoStage::Unstaged,
    });
    assert!(GitPage::has_conflicted_entries(&conflicted_entries));
  }

  #[test]
  fn has_tracked_entries_excludes_untracked_only_state() {
    let untracked_entries = vec![RepoStatusEntry {
      path: PathBuf::from("notes.txt"),
      old_path: None,
      status: RepoStatusKind::Untracked,
      stage: RepoStage::Unstaged,
    }];
    let tracked_entries = vec![
      RepoStatusEntry {
        path: PathBuf::from("notes.txt"),
        old_path: None,
        status: RepoStatusKind::Untracked,
        stage: RepoStage::Unstaged,
      },
      make_status_entry("src/main.rs", RepoStage::Unstaged),
    ];

    assert!(!GitPage::has_tracked_entries(&untracked_entries));
    assert!(GitPage::has_tracked_entries(&tracked_entries));
  }

  #[test]
  fn stash_command_flags_follow_untracked_only_rule() {
    let untracked_entries = vec![RepoStatusEntry {
      path: PathBuf::from("notes.txt"),
      old_path: None,
      status: RepoStatusKind::Untracked,
      stage: RepoStage::Unstaged,
    }];
    let tracked_entries = vec![make_status_entry("src/main.rs", RepoStage::Unstaged)];

    assert_eq!(GitPage::stash_command_flags(&[]), (false, false));
    assert_eq!(
      GitPage::stash_command_flags(&untracked_entries),
      (false, true)
    );
    assert_eq!(GitPage::stash_command_flags(&tracked_entries), (true, true));
  }

  #[test]
  fn stage_all_command_visibility_requires_at_least_one_entry() {
    let entries = vec![make_status_entry("src/main.rs", RepoStage::Unstaged)];
    let all_staged_entries = vec![make_status_entry("src/lib.rs", RepoStage::Staged)];

    assert!(!GitPage::should_show_stage_all_command(&[]));
    assert!(GitPage::should_show_stage_all_command(&entries));
    assert!(!GitPage::should_show_stage_all_command(&all_staged_entries));
  }

  #[test]
  fn unstage_all_command_visibility_requires_all_entries_staged() {
    let mixed_entries = vec![
      make_status_entry("src/main.rs", RepoStage::Staged),
      make_status_entry("src/lib.rs", RepoStage::Unstaged),
    ];
    let all_staged_entries = vec![make_status_entry("src/editor.rs", RepoStage::Staged)];

    assert!(!GitPage::should_show_unstage_all_command(&[]));
    assert!(!GitPage::should_show_unstage_all_command(&mixed_entries));
    assert!(GitPage::should_show_unstage_all_command(
      &all_staged_entries
    ));
  }

  #[test]
  fn unstage_all_palette_command_visibility_requires_any_staged_entry() {
    let unstaged_only_entries = vec![make_status_entry("src/main.rs", RepoStage::Unstaged)];
    let mixed_entries = vec![
      make_status_entry("src/main.rs", RepoStage::Staged),
      make_status_entry("src/lib.rs", RepoStage::Unstaged),
    ];
    let partial_entries = vec![make_status_entry(
      "src/editor.rs",
      RepoStage::PartiallyStaged,
    )];

    assert!(!GitPage::should_show_unstage_all_palette_command(&[]));
    assert!(!GitPage::should_show_unstage_all_palette_command(
      &unstaged_only_entries
    ));
    assert!(GitPage::should_show_unstage_all_palette_command(
      &mixed_entries
    ));
    assert!(GitPage::should_show_unstage_all_palette_command(
      &partial_entries
    ));
  }

  #[test]
  fn should_confirm_stage_all_when_repo_selected_and_conflicts_present() {
    let repo_path = PathBuf::from("/tmp/reviu-stage-all");
    let conflicted_entries = vec![RepoStatusEntry {
      path: PathBuf::from("README.md"),
      old_path: None,
      status: RepoStatusKind::Conflicted,
      stage: RepoStage::Unstaged,
    }];
    let clean_entries = vec![make_status_entry("src/a.rs", RepoStage::Unstaged)];

    assert!(GitPage::should_confirm_stage_all(
      Some(&repo_path),
      &conflicted_entries
    ));
    assert!(!GitPage::should_confirm_stage_all(
      None,
      &conflicted_entries
    ));
    assert!(!GitPage::should_confirm_stage_all(
      Some(&repo_path),
      &clean_entries
    ));
  }

  #[test]
  fn changed_files_tag_visibility_requires_positive_count() {
    assert!(!GitPage::should_show_changed_files_tag(0));
    assert!(GitPage::should_show_changed_files_tag(1));
    assert!(GitPage::should_show_changed_files_tag(42));
  }

  #[gpui::test]
  fn focus_changes_sidebar_list_selects_first_entry_when_unselected(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update_in(cx, |this, window, cx| {
      this.status_entries = vec![
        make_status_entry("src/main.rs", RepoStage::Unstaged),
        make_status_entry("src/lib.rs", RepoStage::Unstaged),
      ];
      this.refresh_file_list(cx);
      assert_eq!(this.file_list.read(cx).selected_index(), None);

      this.focus_changes_sidebar_list(window, cx);
      assert_eq!(
        this.file_list.read(cx).selected_index(),
        Some(IndexPath::new(0))
      );
    });
  }

  #[gpui::test]
  fn focus_history_sidebar_tree_selects_first_commit_and_takes_focus(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update_in(cx, |this, window, cx| {
      this.history_commits = vec![make_commit("c1", &[]), make_commit("c2", &["c1"])];
      this.refresh_history_list(cx);

      let external_focus = cx.focus_handle();
      let page_focus = this.focus_handle.clone();
      window.focus(&external_focus, cx);

      this.focus_history_sidebar_tree(window, cx);

      let focused = window.focused(cx).expect("history tree should take focus");
      assert_ne!(focused, external_focus);
      assert_ne!(focused, page_focus);
      assert_eq!(
        this
          .history_tree
          .read(cx)
          .selected_entry()
          .map(|entry| entry.item().id.to_string())
          .as_deref(),
        Some("history-commit:c1")
      );
    });
  }

  #[gpui::test]
  async fn set_sidebar_mode_history_focuses_history_tree(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    cx.executor().allow_parking();
    let repo = TempRepo::init("git-page-history-sidebar-focus");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.history_commits = vec![make_commit("c1", &[])];
      this.refresh_history_list(cx);

      let external_focus = cx.focus_handle();
      let page_focus = this.focus_handle.clone();
      window.focus(&external_focus, cx);

      this.set_sidebar_mode(GitSidebarMode::History, window, cx);

      let focused = window.focused(cx).expect("history tree should take focus");
      assert_ne!(focused, external_focus);
      assert_ne!(focused, page_focus);
    });

    let history_task = git_page.update_in(cx, |this, _window, _cx| this.history_task.take());
    if let Some(task) = history_task {
      task.await;
    }
  }

  #[gpui::test]
  fn focus_page_restores_page_shortcut_focus(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update_in(cx, |this, window, cx| {
      let external_focus = cx.focus_handle();
      window.focus(&external_focus, cx);
      assert!(!this.focus_handle.contains_focused(window, cx));

      this.focus_page(window, cx);
      assert!(this.focus_handle.contains_focused(window, cx));
    });
  }

  #[gpui::test]
  async fn repo_select_confirm_refocuses_page_shortcuts(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo_a = TempRepo::init("git-page-focus-after-repo-select-a");
    let repo_b = TempRepo::init("git-page-focus-after-repo-select-b");
    let _ = commit_text_file(&repo_a.path, Path::new("README.md"), "a1\n", "initial");
    let _ = commit_text_file(&repo_b.path, Path::new("README.md"), "b1\n", "initial");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo_a.path.clone());

      let external_focus = cx.focus_handle();
      window.focus(&external_focus, cx);
      assert!(!this.focus_handle.contains_focused(window, cx));
    });

    git_page.update(cx, |this, cx| {
      this.handle_repo_select_confirm(repo_b.path.clone(), cx);
    });

    let (has_page_focus, selected_repo) = git_page.update_in(cx, |this, window, cx| {
      (
        this.focus_handle.contains_focused(window, cx),
        this.selected_repo.clone(),
      )
    });
    assert!(has_page_focus);
    assert_eq!(selected_repo, Some(repo_b.path.clone()));

    await_git_page_background_tasks(git_page.clone(), cx).await;
  }

  #[gpui::test]
  fn branch_select_confirm_refocuses_page_shortcuts(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(PathBuf::from("/tmp/reviu-focus-select-branch"));

      let external_focus = cx.focus_handle();
      window.focus(&external_focus, cx);
      assert!(!this.focus_handle.contains_focused(window, cx));
    });

    git_page.update(cx, |this, cx| {
      this.handle_branch_select_confirm(GitPage::detached_branch_select_value(), cx);
    });

    let (has_page_focus, branch_task_is_none) = git_page.update_in(cx, |this, window, cx| {
      (
        this.focus_handle.contains_focused(window, cx),
        this.branch_task.is_none(),
      )
    });
    assert!(has_page_focus);
    assert!(branch_task_is_none);
  }

  #[gpui::test]
  fn interactive_rebase_todo_view_open_and_cancel_returns_to_editor(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(PathBuf::from("/tmp/repo"));
      this.has_head_commit = true;
      this.branch_status = Some(make_branch_status("main", 0, 0, true));
      this.status_entries.clear();

      let commits = vec![git::InteractiveRebaseCommit {
        oid: "1111111111111111111111111111111111111111".to_string(),
        short_oid: "1111111".to_string(),
        summary: "sample commit".to_string(),
      }];
      this.open_interactive_rebase_todo_view_with_commits(
        InteractiveRebaseTarget::HeadCount(2),
        commits,
        window,
        cx,
      );
      assert!(this.interactive_rebase_todo_view.is_some());

      this.close_interactive_rebase_todo_view(window, cx);
      assert!(this.interactive_rebase_todo_view.is_none());
    });
  }

  #[gpui::test]
  async fn reload_status_refocuses_page_when_selected_file_disappears(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-focus-after-selection-clear");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("modify file");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.selected_file = Some(rel_path.to_path_buf());
      this.status_entries = vec![make_status_entry("README.md", RepoStage::Unstaged)];

      let external_focus = cx.focus_handle();
      window.focus(&external_focus, cx);
      assert!(!this.focus_handle.contains_focused(window, cx));
    });

    restore_file(&repo.path, rel_path).expect("restore file on disk");

    let reload_task = git_page.update_in(cx, |this, _window, cx| {
      this.reload_status(cx);
      this.status_task.take().expect("reload status task")
    });
    reload_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (selected_file, has_page_focus) = git_page.update_in(cx, |this, window, cx| {
      (
        this.selected_file.clone(),
        this.focus_handle.contains_focused(window, cx),
      )
    });
    assert!(selected_file.is_none());
    assert!(has_page_focus);
  }

  #[test]
  fn selected_file_update_clears_missing_selection_without_history_file() {
    let update = GitPage::selected_file_update(
      Some(Path::new("src/missing.rs")),
      Some(SelectedFileSource::StatusEntry),
      &[make_status_entry("src/exists.rs", RepoStage::Unstaged)],
      false,
      true,
    );
    assert_eq!(
      update,
      SelectedFileUpdate {
        clear_selection: true,
        sync_diff_view: false,
      }
    );
  }

  #[test]
  fn selected_file_update_keeps_selection_and_syncs_when_present() {
    let update = GitPage::selected_file_update(
      Some(Path::new("src/main.rs")),
      Some(SelectedFileSource::StatusEntry),
      &[make_status_entry("src/main.rs", RepoStage::Unstaged)],
      false,
      true,
    );
    assert_eq!(
      update,
      SelectedFileUpdate {
        clear_selection: false,
        sync_diff_view: true,
      }
    );
  }

  #[test]
  fn selected_file_update_keeps_project_file_when_missing_from_status() {
    let update = GitPage::selected_file_update(
      Some(Path::new("src/main.rs")),
      Some(SelectedFileSource::ProjectFile),
      &[make_status_entry("src/other.rs", RepoStage::Unstaged)],
      false,
      true,
    );
    assert_eq!(
      update,
      SelectedFileUpdate {
        clear_selection: false,
        sync_diff_view: true,
      }
    );
  }

  #[test]
  fn selected_file_update_never_clears_when_history_file_is_open() {
    let update = GitPage::selected_file_update(
      Some(Path::new("src/main.rs")),
      Some(SelectedFileSource::StatusEntry),
      &[make_status_entry("src/other.rs", RepoStage::Unstaged)],
      true,
      true,
    );
    assert_eq!(update, SelectedFileUpdate::default());
  }

  #[test]
  fn selected_file_update_is_noop_without_selection() {
    let update = GitPage::selected_file_update(
      None,
      Some(SelectedFileSource::StatusEntry),
      &[make_status_entry("src/main.rs", RepoStage::Unstaged)],
      false,
      true,
    );
    assert_eq!(update, SelectedFileUpdate::default());
  }

  #[test]
  fn selected_branch_from_status_maps_detached_head_to_detached_select_value() {
    let detached = make_branch_status("HEAD", 0, 0, false);
    assert_eq!(
      GitPage::selected_branch_from_status(Some(&detached)),
      Some(GitPage::detached_branch_select_value())
    );
  }

  #[test]
  fn selected_branch_from_status_maps_named_head_to_local_branch() {
    let main = make_branch_status("main", 0, 0, true);
    assert_eq!(
      GitPage::selected_branch_from_status(Some(&main)),
      Some(BranchRef {
        name: "main".to_string(),
        kind: BranchKind::Local,
      })
    );
  }

  #[test]
  fn branch_select_items_marks_only_selected_branch() {
    let selected = BranchRef {
      name: "main".to_string(),
      kind: BranchKind::Local,
    };
    let items = GitPage::branch_select_items(
      vec![
        BranchRef {
          name: "main".to_string(),
          kind: BranchKind::Local,
        },
        BranchRef {
          name: "feature".to_string(),
          kind: BranchKind::Local,
        },
      ],
      Some(&selected),
      None,
    );

    assert_eq!(items.len(), 2);
    assert!(items[0].is_current);
    assert!(!items[1].is_current);
  }

  #[test]
  fn branch_select_items_includes_detached_head_entry_when_selected_is_detached() {
    let items = GitPage::branch_select_items(
      vec![BranchRef {
        name: "main".to_string(),
        kind: BranchKind::Local,
      }],
      Some(&GitPage::detached_branch_select_value()),
      Some("v1.0.0"),
    );

    assert_eq!(items.len(), 2);
    assert!(items[0].is_current);
    assert_eq!(items[0].label.as_ref(), "HEAD (v1.0.0)");
    assert!(GitPage::is_detached_branch_select_value(&items[0].branch));
    assert!(!items[1].is_current);
  }

  #[gpui::test]
  async fn command_palette_create_branch_creates_and_switches_to_branch(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-create-branch");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::CreateBranch {
          name: "feature".to_string(),
        },
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let status = current_branch_status(&repo.path).expect("read status");
    assert_eq!(status.name, "feature");
    assert!(
      list_branches(&repo.path)
        .expect("list branches")
        .iter()
        .any(|branch| branch.kind == BranchKind::Local && branch.name == "feature")
    );

    let selected_branch = git_page.read_with(cx, |this, _cx| selected_branch_from_dropdown(this));
    assert_eq!(
      selected_branch,
      Some(BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      })
    );
  }

  #[gpui::test]
  async fn command_palette_create_branch_shows_notification_only_when_branch_exists(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-create-branch-existing");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let base_branch = current_branch_status(&repo.path).expect("base status").name;
    create_branch(&repo.path, "feature").expect("create existing target branch");

    let mut mounted_git_page = None;
    let (root, cx) = cx.add_window_view(|window, cx| {
      let git_page = cx.new(|cx| GitPage::new_for_test(window, cx));
      mounted_git_page = Some(git_page.clone());
      gpui_component::Root::new(git_page, window, cx)
    });
    let git_page = mounted_git_page.expect("git page");
    cx.executor().allow_parking();
    cx.executor().allow_parking();

    let initial_notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(initial_notification_count, 0);

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::CreateBranch {
          name: "feature".to_string(),
        },
        _window,
        cx,
      )
    });
    assert!(
      result.is_ok(),
      "create branch failure should close palette and rely on notification feedback"
    );

    await_git_page_background_tasks(git_page.clone(), cx).await;
    cx.cx.run_until_parked();
    cx.run_until_parked();

    let notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(notification_count, 1);
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after failed create")
        .name,
      base_branch
    );
    let feature_count = list_branches(&repo.path)
      .expect("list branches after failed create")
      .iter()
      .filter(|branch| branch.kind == BranchKind::Local && branch.name == "feature")
      .count();
    assert_eq!(feature_count, 1);
  }

  #[gpui::test]
  async fn command_palette_switch_branch_switches_to_requested_branch(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-switch-branch");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    create_branch(&repo.path, "feature").expect("create feature branch");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::SwitchBranch(CommandPaletteBranch {
          name: "feature".into(),
          kind: CommandPaletteBranchKind::Local,
        }),
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let status = current_branch_status(&repo.path).expect("read status");
    assert_eq!(status.name, "feature");
  }

  #[gpui::test]
  async fn command_palette_delete_branch_deletes_requested_branch(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-delete-branch");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    create_branch(&repo.path, "feature").expect("create feature branch");
    let current_branch = current_branch_status(&repo.path)
      .expect("read current branch")
      .name;
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "v2-feature\n",
      "feature change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: current_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.branch_status = Some(make_branch_status(&current_branch, 0, 0, true));
      this.handle_command_palette_action(
        CommandPaletteAction::DeleteBranch(CommandPaletteBranch {
          name: "feature".into(),
          kind: CommandPaletteBranchKind::Local,
        }),
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert_eq!(
      current_branch_status(&repo.path)
        .expect("read status after delete")
        .name,
      current_branch
    );
    assert!(
      !list_branches(&repo.path)
        .expect("list branches after delete")
        .iter()
        .any(|branch| branch.kind == BranchKind::Local && branch.name == "feature")
    );
  }

  #[gpui::test]
  async fn command_palette_delete_branch_rejects_current_branch_with_notification_only(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-delete-current-branch");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let current_branch = current_branch_status(&repo.path)
      .expect("read current branch")
      .name;

    let mut mounted_git_page = None;
    let (root, cx) = cx.add_window_view(|window, cx| {
      let git_page = cx.new(|cx| GitPage::new_for_test(window, cx));
      mounted_git_page = Some(git_page.clone());
      gpui_component::Root::new(git_page, window, cx)
    });
    let git_page = mounted_git_page.expect("git page");
    cx.executor().allow_parking();

    let initial_notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(initial_notification_count, 0);

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.branch_status = Some(make_branch_status(&current_branch, 0, 0, true));
      this.handle_command_palette_action(
        CommandPaletteAction::DeleteBranch(CommandPaletteBranch {
          name: current_branch.clone().into(),
          kind: CommandPaletteBranchKind::Local,
        }),
        _window,
        cx,
      )
    });
    assert!(
      result.is_ok(),
      "delete current branch failure should close palette and rely on notification feedback"
    );

    await_git_page_background_tasks(git_page.clone(), cx).await;
    cx.cx.run_until_parked();
    cx.run_until_parked();

    let notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(notification_count, 1);
  }

  #[gpui::test]
  async fn command_palette_delete_remote_branch_deletes_requested_branch(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let remote = TempBareRepo::init("git-page-cmd-delete-remote-origin");
    let source = TempRepo::init("git-page-cmd-delete-remote-source");
    let clone_dir = TempDir::new("git-page-cmd-delete-remote-clone");

    let _ = commit_text_file(&source.path, Path::new("README.md"), "v1\n", "initial");
    let source_repo = Repository::open(&source.path).expect("open source repo");
    source_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add source origin");
    let base_branch = current_branch_status(&source.path)
      .expect("read source branch status")
      .name;
    push_branch_to_remote(&source.path, &base_branch, "origin");
    create_branch(&source.path, "feature").expect("create source feature branch");
    push_branch_to_remote(&source.path, "feature", "origin");

    let _clone_repo = Repository::clone(
      remote.path.to_str().expect("remote path utf8"),
      &clone_dir.path,
    )
    .expect("clone remote");
    let clone_branch = current_branch_status(&clone_dir.path)
      .expect("read clone branch status")
      .name;

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(clone_dir.path.clone());
      this.branch_status = Some(make_branch_status(&clone_branch, 0, 0, true));
      this.handle_command_palette_action(
        CommandPaletteAction::DeleteBranch(CommandPaletteBranch {
          name: "origin/feature".into(),
          kind: CommandPaletteBranchKind::Remote,
        }),
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(
      Repository::open(&remote.path)
        .expect("open remote")
        .refname_to_id("refs/heads/feature")
        .is_err()
    );
    assert!(
      !list_branches(&clone_dir.path)
        .expect("list clone branches after delete")
        .iter()
        .any(|branch| branch.kind == BranchKind::Remote && branch.name == "origin/feature")
    );
  }

  #[gpui::test]
  async fn command_palette_checkout_detached_detaches_head(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-checkout-detached");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.has_head_commit = true;
      this.branch_status = Some(make_branch_status("main", 0, 0, true));
      seed_repo_branch_state(this, &repo.path, cx);
      let target = head_oid(&repo.path).to_string();
      this.handle_command_palette_action(
        CommandPaletteAction::CheckoutDetached { target },
        window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let status = current_branch_status(&repo.path).expect("read status");
    assert_eq!(status.name, "HEAD");

    let selected_branch = git_page.read_with(cx, |this, _cx| selected_branch_from_dropdown(this));
    assert_eq!(
      selected_branch,
      Some(GitPage::detached_branch_select_value())
    );
  }

  #[gpui::test]
  async fn command_palette_switch_repository_updates_selected_repo_and_header_select(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo_a = TempRepo::init("git-page-cmd-switch-repo-a");
    let repo_b = TempRepo::init("git-page-cmd-switch-repo-b");
    let _ = commit_text_file(&repo_a.path, Path::new("README.md"), "a1\n", "initial");
    let _ = commit_text_file(&repo_b.path, Path::new("README.md"), "b1\n", "initial");

    ConfigStore::persist_recent_repository(&repo_a.path);
    ConfigStore::persist_recent_repository(&repo_b.path);

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo_a.path.clone());
      this.refresh_repo_select(cx);
      this.handle_command_palette_action(
        CommandPaletteAction::SwitchRepository(CommandPaletteRepository {
          path: repo_b.path.to_string_lossy().to_string().into(),
        }),
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (selected_repo, header_contains_repo) = git_page.read_with(cx, |this, _cx| {
      (
        this.selected_repo.clone(),
        this
          .repo_dropdown_items
          .iter()
          .any(|item| item.path == repo_b.path),
      )
    });
    assert_eq!(selected_repo, Some(repo_b.path.clone()));
    assert!(header_contains_repo);
  }

  #[gpui::test]
  async fn command_palette_forget_repository_removes_it_from_dropdown(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo_a = TempRepo::init("git-page-cmd-forget-repo-a");
    let repo_b = TempRepo::init("git-page-cmd-forget-repo-b");
    let _ = commit_text_file(&repo_a.path, Path::new("README.md"), "a1\n", "initial");
    let _ = commit_text_file(&repo_b.path, Path::new("README.md"), "b1\n", "initial");

    ConfigStore::persist_recent_repository(&repo_a.path);
    ConfigStore::persist_recent_repository(&repo_b.path);

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo_a.path.clone());
      this.refresh_repo_select(cx);
      this.handle_command_palette_action(
        CommandPaletteAction::ForgetRepository(CommandPaletteRepository {
          path: repo_b.path.to_string_lossy().to_string().into(),
        }),
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (selected_repo, still_contains_forgotten) = git_page.read_with(cx, |this, _cx| {
      (
        this.selected_repo.clone(),
        this
          .repo_dropdown_items
          .iter()
          .any(|item| item.path == repo_b.path),
      )
    });
    // The forgotten repo should be gone from the dropdown, but the selection is untouched.
    assert_eq!(selected_repo, Some(repo_a.path.clone()));
    assert!(!still_contains_forgotten);

    let persisted: Vec<PathBuf> = ConfigStore::load_recent_repositories()
      .into_iter()
      .map(|r| r.path)
      .collect();
    assert!(!persisted.contains(&repo_b.path));
    assert!(persisted.contains(&repo_a.path));
  }

  #[gpui::test]
  async fn command_palette_forget_selected_repository_switches_to_next_remaining(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo_a = TempRepo::init("git-page-cmd-forget-selected-a");
    let repo_b = TempRepo::init("git-page-cmd-forget-selected-b");
    let _ = commit_text_file(&repo_a.path, Path::new("README.md"), "a1\n", "initial");
    let _ = commit_text_file(&repo_b.path, Path::new("README.md"), "b1\n", "initial");

    ConfigStore::persist_recent_repository(&repo_a.path);
    ConfigStore::persist_recent_repository(&repo_b.path);

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo_b.path.clone());
      this.refresh_repo_select(cx);
      this.handle_command_palette_action(
        CommandPaletteAction::ForgetRepository(CommandPaletteRepository {
          path: repo_b.path.to_string_lossy().to_string().into(),
        }),
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (selected_repo, dropdown_has_b) = git_page.read_with(cx, |this, _cx| {
      (
        this.selected_repo.clone(),
        this
          .repo_dropdown_items
          .iter()
          .any(|item| item.path == repo_b.path),
      )
    });
    assert_eq!(selected_repo, Some(repo_a.path.clone()));
    assert!(!dropdown_has_b);
  }

  #[gpui::test]
  async fn command_palette_forget_last_repository_clears_selection(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-forget-last");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    ConfigStore::persist_recent_repository(&repo.path);

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.refresh_repo_select(cx);
      this.handle_command_palette_action(
        CommandPaletteAction::ForgetRepository(CommandPaletteRepository {
          path: repo.path.to_string_lossy().to_string().into(),
        }),
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (selected_repo, dropdown_items_len) = git_page.read_with(cx, |this, _cx| {
      (this.selected_repo.clone(), this.repo_dropdown_items.len())
    });
    assert_eq!(selected_repo, None);
    assert_eq!(dropdown_items_len, 0);
  }

  #[gpui::test]
  async fn command_palette_switch_repository_returns_error_for_missing_repository(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-switch-repo-missing");
    let missing_repo = repo.path.join("does-not-exist");

    let (git_page, cx) = add_git_page_window_with_root(cx);

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::SwitchRepository(CommandPaletteRepository {
          path: missing_repo.to_string_lossy().to_string().into(),
        }),
        _window,
        cx,
      )
    });

    let error = result.expect_err("switch repository should fail for a missing path");
    assert!(error.as_ref().starts_with("Repository not found:"));
    let selected_repo = git_page.read_with(cx, |this, _| this.selected_repo.clone());
    assert_eq!(selected_repo, Some(repo.path.clone()));
  }

  #[gpui::test]
  async fn command_palette_dialog_ignores_dialog_confirm_action(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-dialog-confirm");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let mut mounted_git_page = None;
    let (_root, cx) = cx.add_window_view(|window, cx| {
      let git_page = cx.new(|cx| GitPage::new_for_test(window, cx));
      mounted_git_page = Some(git_page.clone());
      gpui_component::Root::new(git_page, window, cx)
    });
    let git_page = mounted_git_page.expect("git page");
    cx.executor().allow_parking();

    git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.open_command_palette(window, cx, Some(CommandPaletteInitialScreen::SwitchBranch));
    });
    cx.cx.run_until_parked();
    cx.run_until_parked();

    let dialog_open_before_confirm =
      git_page.update_in(cx, |_this, window, cx| window.has_active_dialog(cx));
    assert!(dialog_open_before_confirm);

    git_page.update_in(cx, |_this, window, cx| {
      window.dispatch_action(Box::new(gpui_component::dialog::ConfirmDialog), cx);
    });
    cx.cx.run_until_parked();
    cx.run_until_parked();

    let dialog_open_after_confirm =
      git_page.update_in(cx, |_this, window, cx| window.has_active_dialog(cx));
    assert!(dialog_open_after_confirm);
  }

  #[gpui::test]
  async fn command_palette_fetch_updates_remote_tracking_refs(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let remote = TempBareRepo::init("git-page-cmd-fetch-origin");
    let source = TempRepo::init("git-page-cmd-fetch-source");
    let clone_dir = TempDir::new("git-page-cmd-fetch-clone");

    let _ = commit_text_file(&source.path, Path::new("README.md"), "v1\n", "initial");
    let source_repo = Repository::open(&source.path).expect("open source repo");
    source_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add source origin");
    let base_branch = current_branch_status(&source.path)
      .expect("read source branch status")
      .name;
    push_branch_to_remote(&source.path, &base_branch, "origin");

    let _clone_repo = Repository::clone(
      remote.path.to_str().expect("remote path utf8"),
      &clone_dir.path,
    )
    .expect("clone remote");
    let tracking_ref = format!("refs/remotes/origin/{base_branch}");
    let before = Repository::open(&clone_dir.path)
      .expect("open clone")
      .refname_to_id(&tracking_ref)
      .expect("read remote tracking ref before fetch");

    let _ = commit_text_file(
      &source.path,
      Path::new("README.md"),
      "v2\n",
      "source update",
    );
    push_branch_to_remote(&source.path, &base_branch, "origin");
    let expected = remote_branch_oid(&remote.path, &base_branch);
    assert_ne!(
      before, expected,
      "expected remote branch to advance after push"
    );

    let clone_repo = Repository::open(&clone_dir.path).expect("open clone");
    clone_repo
      .reference(
        &tracking_ref,
        before,
        true,
        "force stale remote tracking ref",
      )
      .expect("force stale remote tracking ref");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(clone_dir.path.clone());
      this.handle_command_palette_action(CommandPaletteAction::Fetch, window, cx)
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let after = Repository::open(&clone_dir.path)
      .expect("open clone")
      .refname_to_id(&tracking_ref)
      .expect("read remote tracking ref after fetch");
    assert_eq!(after, expected);
  }

  #[gpui::test]
  async fn command_palette_fetch_toggles_loading_state(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let remote = TempBareRepo::init("git-page-cmd-fetch-loading-origin");
    let source = TempRepo::init("git-page-cmd-fetch-loading-source");
    let clone_dir = TempDir::new("git-page-cmd-fetch-loading-clone");

    let _ = commit_text_file(&source.path, Path::new("README.md"), "v1\n", "initial");
    let source_repo = Repository::open(&source.path).expect("open source repo");
    source_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add source origin");
    let base_branch = current_branch_status(&source.path)
      .expect("read source branch status")
      .name;
    push_branch_to_remote(&source.path, &base_branch, "origin");

    let _clone_repo = Repository::clone(
      remote.path.to_str().expect("remote path utf8"),
      &clone_dir.path,
    )
    .expect("clone remote");
    let _ = commit_text_file(
      &source.path,
      Path::new("README.md"),
      "v2\n",
      "source update",
    );
    push_branch_to_remote(&source.path, &base_branch, "origin");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(clone_dir.path.clone());
      this.handle_command_palette_action(CommandPaletteAction::Fetch, window, cx)
    });
    assert!(result.is_ok());
    assert!(git_page.read_with(cx, |this, _| this.fetch_in_progress));

    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(!git_page.read_with(cx, |this, _| this.fetch_in_progress));
  }

  #[gpui::test]
  async fn fetch_repository_failure_shows_error_notification(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-fetch-failure-notification");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let missing_remote = repo.path.join("missing-remote.git");
    Repository::open(&repo.path)
      .expect("open repo")
      .remote("origin", missing_remote.to_str().expect("remote path utf8"))
      .expect("add origin remote");

    let mut mounted_git_page = None;
    let (root, cx) = cx.add_window_view(|window, cx| {
      let git_page = cx.new(|cx| GitPage::new_for_test(window, cx));
      mounted_git_page = Some(git_page.clone());
      gpui_component::Root::new(git_page, window, cx)
    });
    let git_page = mounted_git_page.expect("git page");
    cx.executor().allow_parking();
    cx.executor().allow_parking();

    let initial_notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(initial_notification_count, 0);

    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.fetch_repository(repo.path.clone(), cx);
    });
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(notification_count, 1);
  }

  #[gpui::test]
  async fn command_palette_create_branch_from_local_creates_and_switches(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-create-from");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    create_branch(&repo.path, "feature").expect("create feature branch");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let feature_head = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "v2-feature\n",
      "feature change",
    );

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::CreateBranchFrom {
          name: "feature-copy".to_string(),
          base: CommandPaletteBranch {
            name: "feature".into(),
            kind: CommandPaletteBranchKind::Local,
          },
        },
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let status = current_branch_status(&repo.path).expect("read status");
    assert_eq!(status.name, "feature-copy");

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let created = repo_handle
      .find_branch("feature-copy", BranchType::Local)
      .expect("find feature-copy branch");
    assert_eq!(created.get().target(), Some(feature_head));
    assert!(created.upstream().is_err());
  }

  #[gpui::test]
  async fn command_palette_create_branch_from_shows_notification_only_when_branch_exists(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-create-from-existing");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let base_branch = current_branch_status(&repo.path).expect("base status").name;
    create_branch(&repo.path, "feature").expect("create feature branch");
    create_branch(&repo.path, "feature-copy").expect("create existing target branch");

    let mut mounted_git_page = None;
    let (root, cx) = cx.add_window_view(|window, cx| {
      let git_page = cx.new(|cx| GitPage::new_for_test(window, cx));
      mounted_git_page = Some(git_page.clone());
      gpui_component::Root::new(git_page, window, cx)
    });
    let git_page = mounted_git_page.expect("git page");
    cx.executor().allow_parking();

    let initial_notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(initial_notification_count, 0);

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::CreateBranchFrom {
          name: "feature-copy".to_string(),
          base: CommandPaletteBranch {
            name: "feature".into(),
            kind: CommandPaletteBranchKind::Local,
          },
        },
        _window,
        cx,
      )
    });
    assert!(
      result.is_ok(),
      "create branch from failure should close palette and rely on notification feedback"
    );

    await_git_page_background_tasks(git_page.clone(), cx).await;
    cx.cx.run_until_parked();
    cx.run_until_parked();

    let notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(notification_count, 1);
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after failed create from")
        .name,
      base_branch
    );
    let feature_copy_count = list_branches(&repo.path)
      .expect("list branches after failed create from")
      .iter()
      .filter(|branch| branch.kind == BranchKind::Local && branch.name == "feature-copy")
      .count();
    assert_eq!(feature_copy_count, 1);
  }

  #[gpui::test]
  async fn command_palette_switch_remote_branch_creates_local_branch_with_upstream(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let remote = TempBareRepo::init("git-page-cmd-switch-remote-origin");
    let source = TempRepo::init("git-page-cmd-switch-remote-source");
    let clone_dir = TempDir::new("git-page-cmd-switch-remote-clone");

    let _ = commit_text_file(&source.path, Path::new("README.md"), "v1\n", "initial");
    let source_repo = Repository::open(&source.path).expect("open source repo");
    source_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add source origin");

    let base_branch = current_branch_status(&source.path)
      .expect("source branch status")
      .name;
    push_branch_to_remote(&source.path, &base_branch, "origin");
    set_remote_head(&remote.path, &base_branch);

    create_branch(&source.path, "feature").expect("create source feature branch");
    switch_branch(
      &source.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch source to feature");
    let _ = commit_text_file(
      &source.path,
      Path::new("README.md"),
      "v2-feature\n",
      "feature change",
    );
    push_branch_to_remote(&source.path, "feature", "origin");

    let _clone_repo = Repository::clone(
      remote.path.to_str().expect("remote path utf8"),
      &clone_dir.path,
    )
    .expect("clone remote");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();
    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(clone_dir.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::SwitchBranch(CommandPaletteBranch {
          name: "origin/feature".into(),
          kind: CommandPaletteBranchKind::Remote,
        }),
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let status = current_branch_status(&clone_dir.path).expect("status after remote switch");
    assert_eq!(status.name, "feature");
    assert!(status.has_upstream);

    let clone_repo = Repository::open(&clone_dir.path).expect("open clone repo");
    let local_feature = clone_repo
      .find_branch("feature", BranchType::Local)
      .expect("find local feature branch");
    let upstream = local_feature
      .upstream()
      .expect("feature upstream")
      .name()
      .expect("upstream name")
      .expect("non-empty upstream")
      .to_string();
    assert_eq!(upstream, "origin/feature");
  }

  #[gpui::test]
  async fn command_palette_switch_remote_branch_shows_notification_only_when_remote_branch_missing(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-switch-remote-missing");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let base_branch = current_branch_status(&repo.path).expect("base status").name;

    let mut mounted_git_page = None;
    let (root, cx) = cx.add_window_view(|window, cx| {
      let git_page = cx.new(|cx| GitPage::new_for_test(window, cx));
      mounted_git_page = Some(git_page.clone());
      gpui_component::Root::new(git_page, window, cx)
    });
    let git_page = mounted_git_page.expect("git page");
    cx.executor().allow_parking();

    let initial_notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(initial_notification_count, 0);

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::SwitchBranch(CommandPaletteBranch {
          name: "origin/missing".into(),
          kind: CommandPaletteBranchKind::Remote,
        }),
        _window,
        cx,
      )
    });
    assert!(
      result.is_ok(),
      "switch branch failure should close palette and rely on notification feedback"
    );

    await_git_page_background_tasks(git_page.clone(), cx).await;
    cx.cx.run_until_parked();
    cx.run_until_parked();

    let notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(notification_count, 1);
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after failed remote switch")
        .name,
      base_branch
    );
  }

  #[gpui::test]
  async fn command_palette_create_branch_from_remote_hides_pr_until_unique_commit(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let remote = TempBareRepo::init("git-page-cmd-create-from-remote-origin");
    let source = TempRepo::init("git-page-cmd-create-from-remote-source");
    let clone_dir = TempDir::new("git-page-cmd-create-from-remote-clone");

    let _ = commit_text_file(&source.path, Path::new("README.md"), "v1\n", "initial");
    let source_repo = Repository::open(&source.path).expect("open source repo");
    source_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add source origin");

    let base_branch = current_branch_status(&source.path)
      .expect("source branch status")
      .name;
    push_branch_to_remote(&source.path, &base_branch, "origin");
    set_remote_head(&remote.path, &base_branch);

    create_branch(&source.path, "feature").expect("create source feature branch");
    push_branch_to_remote(&source.path, "feature", "origin");

    let _clone_repo = Repository::clone(
      remote.path.to_str().expect("remote path utf8"),
      &clone_dir.path,
    )
    .expect("clone remote");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();
    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(clone_dir.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::CreateBranchFrom {
          name: "my-feature".to_string(),
          base: CommandPaletteBranch {
            name: "origin/feature".into(),
            kind: CommandPaletteBranchKind::Remote,
          },
        },
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let status = current_branch_status(&clone_dir.path).expect("status after create from remote");
    assert_eq!(status.name, "my-feature");
    assert!(!status.has_upstream);
    assert!(!branch_has_unpublished_commits(&clone_dir.path).expect("unpublished commit state"));
    assert_eq!(
      GitPage::push_action_label(Some(&status), true),
      "Push (Publish branch)"
    );
    let (can_push, has_unpublished_branch_commits) = git_page.read_with(cx, |this, _| {
      (this.can_push, this.has_unpublished_branch_commits)
    });
    assert!(can_push);
    assert!(!has_unpublished_branch_commits);

    let clone_repo = Repository::open(&clone_dir.path).expect("open clone repo");
    let created = clone_repo
      .find_branch("my-feature", BranchType::Local)
      .expect("find created branch");
    assert!(created.upstream().is_err());

    let _ = commit_text_file(
      &clone_dir.path,
      Path::new("README.md"),
      "v2-feature\n",
      "feature change",
    );
    let reload_task = git_page.update_in(cx, |this, _window, cx| {
      this.reload_status(cx);
      this.status_task.take().expect("reload status task")
    });
    reload_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(branch_has_unpublished_commits(&clone_dir.path).expect("unpublished commit state"));
    let (can_push, has_unpublished_branch_commits) = git_page.read_with(cx, |this, _| {
      (this.can_push, this.has_unpublished_branch_commits)
    });
    assert!(can_push);
    assert!(has_unpublished_branch_commits);
  }

  #[test]
  fn command_palette_create_branch_from_remote_uses_notification_feedback() {
    assert_eq!(
      GitPage::command_palette_error_notification_title(&CommandPaletteAction::CreateBranchFrom {
        name: "my-feature".to_string(),
        base: CommandPaletteBranch {
          name: "origin/missing".into(),
          kind: CommandPaletteBranchKind::Remote,
        },
      }),
      Some("Create branch failed")
    );
  }

  #[test]
  fn command_palette_delete_branch_uses_notification_feedback() {
    assert_eq!(
      GitPage::command_palette_error_notification_title(&CommandPaletteAction::DeleteBranch(
        CommandPaletteBranch {
          name: "feature".into(),
          kind: CommandPaletteBranchKind::Local,
        }
      )),
      Some("Delete branch failed")
    );
  }

  #[cfg(not(target_os = "linux"))]
  #[gpui::test]
  async fn command_palette_commit_stages_all_when_needed(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-commit");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("write unstaged change");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();
    let result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.status_entries = list_repo_status(&repo.path).expect("list status before commit");
      this.commit_input.update(cx, |input, cx| {
        input.set_value("feat: command palette commit", window, cx)
      });
      this.handle_command_palette_action(CommandPaletteAction::Commit, window, cx)
    });
    assert!(result.is_ok());
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(
      list_repo_status(&repo.path)
        .expect("list status after commit")
        .is_empty()
    );
    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let head = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read head commit");
    assert_eq!(head.summary(), Some("feat: command palette commit"));
  }

  #[gpui::test]
  async fn command_palette_commit_returns_error_when_command_is_disabled(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-commit-disabled");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("write unstaged change");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    let result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.status_entries = list_repo_status(&repo.path).expect("list status before commit");
      this
        .commit_input
        .update(cx, |input, cx| input.set_value("   ", window, cx));
      this.handle_command_palette_action(CommandPaletteAction::Commit, window, cx)
    });
    let error = result.expect_err("disabled commit should return an error");
    assert_eq!(error.as_ref(), "Commit command is currently disabled.");
  }

  #[gpui::test]
  async fn command_palette_push_pushes_to_remote_when_allowed(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let source = TempRepo::init("git-page-cmd-push-success-source");
    let remote = TempBareRepo::init("git-page-cmd-push-success-remote");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&source.path, rel_path, "v1\n", "initial");

    let source_repo = Repository::open(&source.path).expect("open source");
    source_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add origin remote");
    let branch_name = current_branch_status(&source.path)
      .expect("source branch status")
      .name;
    push_branch_to_remote(&source.path, &branch_name, "origin");
    set_upstream(&source.path, &branch_name, &format!("origin/{branch_name}"));
    set_remote_head(&remote.path, &branch_name);

    let _ = commit_text_file(&source.path, rel_path, "v2-source\n", "source change");
    let expected_head = head_oid(&source.path);

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let push_result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(source.path.clone());
      this.can_push = true;
      this.handle_command_palette_action(CommandPaletteAction::Push, window, cx)
    });
    assert!(push_result.is_ok());
    assert!(git_page.read_with(cx, |this, _| this.push_pull_in_progress));

    let push_task = git_page.update_in(cx, |this, _window, _| this.status_task.take());
    push_task.expect("push task should exist").await;
    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    assert_eq!(remote_branch_oid(&remote.path, &branch_name), expected_head);
    assert!(!git_page.read_with(cx, |this, _| this.push_pull_in_progress));
  }

  #[gpui::test]
  async fn command_palette_force_push_force_pushes_when_allowed(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let source = TempRepo::init("git-page-cmd-force-push-source");
    let remote = TempBareRepo::init("git-page-cmd-force-push-remote");
    let peer = TempDir::new("git-page-cmd-force-push-peer");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&source.path, rel_path, "v1\n", "initial");

    let source_repo = Repository::open(&source.path).expect("open source");
    source_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add origin remote");
    let branch_name = current_branch_status(&source.path)
      .expect("source branch status")
      .name;
    push_branch_to_remote(&source.path, &branch_name, "origin");
    set_upstream(&source.path, &branch_name, &format!("origin/{branch_name}"));
    set_remote_head(&remote.path, &branch_name);

    let _ = Repository::clone(remote.path.to_str().expect("remote path utf8"), &peer.path)
      .expect("clone remote into peer");

    let _ = commit_text_file(&source.path, rel_path, "v2-source\n", "source change");
    let expected_head = head_oid(&source.path);

    let _ = commit_text_file(&peer.path, rel_path, "v2-peer\n", "peer change");
    push_branch_to_remote(&peer.path, &branch_name, "origin");

    let non_force = push(&source.path, false).err();
    assert!(non_force.is_some(), "non-force push should fail");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let push_result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(source.path.clone());
      this.can_force_push = true;
      this.handle_command_palette_action(CommandPaletteAction::ForcePush, window, cx)
    });
    assert!(push_result.is_ok());
    assert!(git_page.read_with(cx, |this, _| this.push_pull_in_progress));

    let force_task = git_page.update_in(cx, |this, _window, _| this.status_task.take());
    force_task.expect("force push task should exist").await;
    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    assert_eq!(remote_branch_oid(&remote.path, &branch_name), expected_head);
    assert!(!git_page.read_with(cx, |this, _| this.push_pull_in_progress));
  }

  #[gpui::test]
  async fn command_palette_undo_last_commit_moves_head_when_allowed(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-undo-success");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "first");
    let _ = commit_text_file(&repo.path, rel_path, "v2\n", "second");

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let expected_parent = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("head before undo")
      .parent(0)
      .expect("parent")
      .id();

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let undo_result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.can_undo_last_commit = true;
      this.handle_command_palette_action(CommandPaletteAction::UndoLastCommit, window, cx)
    });
    assert!(undo_result.is_ok());

    let undo_task = git_page.update_in(cx, |this, _window, _| this.status_task.take());
    undo_task.expect("undo task should exist").await;
    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    let head_after = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("head after undo")
      .id();
    assert_eq!(head_after, expected_parent);
  }

  #[gpui::test]
  async fn undo_last_commit_action_failure_shows_error_notification(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-undo-failure-notification");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    let head_before = head_oid(&repo.path);

    let mut mounted_git_page = None;
    let (root, cx) = cx.add_window_view(|window, cx| {
      let git_page = cx.new(|cx| GitPage::new_for_test(window, cx));
      mounted_git_page = Some(git_page.clone());
      gpui_component::Root::new(git_page, window, cx)
    });
    let git_page = mounted_git_page.expect("git page");
    cx.executor().allow_parking();

    let initial_notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(initial_notification_count, 0);

    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.can_undo_last_commit = true;
      this.undo_last_commit_action(cx);
    });
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(notification_count, 1);
    assert_eq!(head_oid(&repo.path), head_before);
  }

  #[cfg(not(target_os = "linux"))]
  #[gpui::test]
  async fn command_palette_amend_updates_head_message_when_allowed(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-amend-success");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let amend_result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.has_head_commit = true;
      this
        .commit_input
        .update(cx, |input, cx| input.set_value("feat: amended", window, cx));
      this.handle_command_palette_action(CommandPaletteAction::Amend, window, cx)
    });
    assert!(amend_result.is_ok());

    let amend_task = git_page.update_in(cx, |this, _window, _| this.status_task.take());
    amend_task.expect("amend task should exist").await;
    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let head = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read head after amend");
    assert_eq!(head.summary(), Some("feat: amended"));
  }

  #[gpui::test]
  fn command_palette_commit_menu_actions_return_error_when_disabled(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-commit-menu-disabled");
    let (git_page, cx) = add_git_page_window_with_root(cx);

    for (action, expected_error) in [
      (
        CommandPaletteAction::Push,
        "Push command is currently disabled.",
      ),
      (
        CommandPaletteAction::ForcePush,
        "Force push command is currently disabled.",
      ),
      (
        CommandPaletteAction::UndoLastCommit,
        "Undo last commit command is currently disabled.",
      ),
      (
        CommandPaletteAction::Amend,
        "Amend command is currently disabled.",
      ),
    ] {
      let result = git_page.update_in(cx, |this, window, cx| {
        this.selected_repo = Some(repo.path.clone());
        this.rebase_in_progress = true;
        this.can_push = true;
        this.can_force_push = true;
        this.can_undo_last_commit = true;
        this.has_head_commit = true;
        this.handle_command_palette_action(action.clone(), window, cx)
      });
      let error = result.expect_err("action should be disabled during rebase flow");
      assert_eq!(error.as_ref(), expected_error);
    }
  }

  #[gpui::test]
  fn command_palette_selected_file_stage_toggle_returns_error_when_disabled(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-selected-file-toggle-disabled");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("modify tracked file");

    let (git_page, cx) = add_git_page_window_with_root(cx);

    let stage_without_selection = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.selected_file = None;
      this.status_entries = list_repo_status(&repo.path).expect("list status");
      this.handle_command_palette_action(CommandPaletteAction::StageSelectedFile, window, cx)
    });
    assert_eq!(
      stage_without_selection
        .expect_err("stage selected file should be disabled without selection")
        .as_ref(),
      "Stage file command is currently disabled."
    );

    let unstage_without_staged_file = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.selected_file = Some(rel_path.to_path_buf());
      this.status_entries = list_repo_status(&repo.path).expect("list status");
      this.handle_command_palette_action(CommandPaletteAction::UnstageSelectedFile, window, cx)
    });
    assert_eq!(
      unstage_without_staged_file
        .expect_err("unstage selected file should be disabled when file is unstaged")
        .as_ref(),
      "Unstage file command is currently disabled."
    );
  }

  #[gpui::test]
  async fn command_palette_stage_selected_file_stages_selected_entry(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-stage-selected-file");
    let first = Path::new("a.txt");
    let second = Path::new("b.txt");
    let _ = commit_text_file(&repo.path, first, "a1\n", "first");
    let _ = commit_text_file(&repo.path, second, "b1\n", "second");
    std::fs::write(repo.path.join(first), "a2\n").expect("modify first");
    std::fs::write(repo.path.join(second), "b2\n").expect("modify second");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.status_entries = list_repo_status(&repo.path).expect("list status");
      this.selected_file = Some(first.to_path_buf());
      this.handle_command_palette_action(CommandPaletteAction::StageSelectedFile, window, cx)
    });
    assert!(result.is_ok());

    let stage_task = git_page.update_in(cx, |this, _window, _| this.status_task.take());
    stage_task.expect("stage selected file task").await;
    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    let entries = list_repo_status(&repo.path).expect("list status after stage selected file");
    let first_entry = entries
      .iter()
      .find(|entry| entry.path == first)
      .expect("first entry");
    let second_entry = entries
      .iter()
      .find(|entry| entry.path == second)
      .expect("second entry");
    assert_eq!(first_entry.stage, RepoStage::Staged);
    assert_eq!(second_entry.stage, RepoStage::Unstaged);
  }

  #[gpui::test]
  async fn command_palette_unstage_selected_file_unstages_selected_entry(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-unstage-selected-file");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("modify file");
    stage_file(&repo.path, rel_path).expect("stage file before command");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.status_entries = list_repo_status(&repo.path).expect("list status");
      this.selected_file = Some(rel_path.to_path_buf());
      this.handle_command_palette_action(CommandPaletteAction::UnstageSelectedFile, window, cx)
    });
    assert!(result.is_ok());

    let unstage_task = git_page.update_in(cx, |this, _window, _| this.status_task.take());
    unstage_task.expect("unstage selected file task").await;
    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    let entries = list_repo_status(&repo.path).expect("list status after unstage selected file");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, rel_path);
    assert_eq!(entries[0].stage, RepoStage::Unstaged);
  }

  #[gpui::test]
  async fn command_palette_accept_all_current_conflicts_resolves_editor_markers(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-accept-all-current");
    let rel_path = Path::new("README.md");
    let conflict_text = "before\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> branch\nafter\n";
    std::fs::write(repo.path.join(rel_path), conflict_text).expect("write conflict markers");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.status_entries = vec![RepoStatusEntry {
        path: rel_path.to_path_buf(),
        old_path: None,
        status: RepoStatusKind::Conflicted,
        stage: RepoStage::Unstaged,
      }];
      this.open_file(rel_path.to_path_buf(), cx);
    });
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let can_show_before = git_page.read_with(cx, |this, cx| {
      this.should_show_accept_all_conflicts_palette_commands(cx)
    });
    assert!(
      can_show_before,
      "command should be visible for conflicted file"
    );

    let result = git_page.update_in(cx, |this, window, cx| {
      this.handle_command_palette_action(
        CommandPaletteAction::AcceptAllCurrentConflicts,
        window,
        cx,
      )
    });
    assert!(result.is_ok());

    let (contents, can_show_after) = git_page.read_with(cx, |this, cx| {
      let contents = {
        let editor = this.editor.as_ref().expect("editor should exist").read(cx);
        let document = editor.document().read(cx);
        document.slice_to_string(0..document.len())
      };
      (
        contents,
        this.should_show_accept_all_conflicts_palette_commands(cx),
      )
    });
    assert_eq!(contents, "before\nours\nafter\n");
    assert!(
      !can_show_after,
      "commands should disappear once all conflict markers are resolved"
    );
  }

  #[gpui::test]
  async fn command_palette_accept_all_incoming_conflicts_resolves_editor_markers(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-accept-all-incoming");
    let rel_path = Path::new("README.md");
    let conflict_text = "before\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> branch\nafter\n";
    std::fs::write(repo.path.join(rel_path), conflict_text).expect("write conflict markers");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.status_entries = vec![RepoStatusEntry {
        path: rel_path.to_path_buf(),
        old_path: None,
        status: RepoStatusKind::Conflicted,
        stage: RepoStage::Unstaged,
      }];
      this.open_file(rel_path.to_path_buf(), cx);
    });
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let can_show_before = git_page.read_with(cx, |this, cx| {
      this.should_show_accept_all_conflicts_palette_commands(cx)
    });
    assert!(
      can_show_before,
      "command should be visible for conflicted file"
    );

    let result = git_page.update_in(cx, |this, window, cx| {
      this.handle_command_palette_action(
        CommandPaletteAction::AcceptAllIncomingConflicts,
        window,
        cx,
      )
    });
    assert!(result.is_ok());

    let (contents, can_show_after) = git_page.read_with(cx, |this, cx| {
      let contents = {
        let editor = this.editor.as_ref().expect("editor should exist").read(cx);
        let document = editor.document().read(cx);
        document.slice_to_string(0..document.len())
      };
      (
        contents,
        this.should_show_accept_all_conflicts_palette_commands(cx),
      )
    });
    assert_eq!(contents, "before\ntheirs\nafter\n");
    assert!(
      !can_show_after,
      "commands should disappear once all conflict markers are resolved"
    );
  }

  #[gpui::test]
  async fn editor_conflict_navigation_moves_between_conflicts_in_selected_file(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-editor-conflict-navigation");
    let rel_path = Path::new("README.md");
    let conflict_text = "pre\n<<<<<<< HEAD\nours1\n=======\ntheirs1\n>>>>>>> branch\nmid\n<<<<<<< HEAD\nours2\n=======\ntheirs2\n>>>>>>> branch\npost\n";
    std::fs::write(repo.path.join(rel_path), conflict_text).expect("write conflict markers");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.status_entries = vec![RepoStatusEntry {
        path: rel_path.to_path_buf(),
        old_path: None,
        status: RepoStatusKind::Conflicted,
        stage: RepoStage::Unstaged,
      }];
      this.open_file(rel_path.to_path_buf(), cx);
    });
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let initial_state = git_page.read_with(cx, |this, cx| {
      this
        .editor_conflict_navigation_state(cx)
        .expect("initial conflict navigation state")
    });
    assert_eq!(initial_state.active_index, 0);
    assert_eq!(initial_state.total, 2);

    git_page.update_in(cx, |this, _window, cx| {
      this.navigate_annotation_in_editor(AnnotationDirection::Next, cx);
    });

    let next_state = git_page.read_with(cx, |this, cx| {
      this
        .editor_conflict_navigation_state(cx)
        .expect("next conflict navigation state")
    });
    assert_eq!(next_state.active_index, 1);
    assert_eq!(next_state.total, 2);
    assert_eq!(next_state.active_start_line, 7);
  }

  #[gpui::test]
  async fn annotation_navigation_falls_back_to_hunks_when_no_conflicts(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-annotation-hunk-navigation");
    let rel_path = Path::new("README.md");
    let base_contents = (0..30)
      .map(|line| format!("line {line}"))
      .collect::<Vec<_>>()
      .join("\n");
    let _ = commit_text_file(
      &repo.path,
      rel_path,
      &format!("{base_contents}\n"),
      "initial",
    );

    let mut modified_lines = (0..30)
      .map(|line| format!("line {line}"))
      .collect::<Vec<_>>();
    modified_lines[5] = "line 5 modified".to_string();
    modified_lines[20] = "line 20 modified".to_string();
    let modified_contents = format!("{}\n", modified_lines.join("\n"));
    std::fs::write(repo.path.join(rel_path), modified_contents).expect("write modified file");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.status_entries = vec![RepoStatusEntry {
        path: rel_path.to_path_buf(),
        old_path: None,
        status: RepoStatusKind::Modified,
        stage: RepoStage::Unstaged,
      }];
      this.open_file(rel_path.to_path_buf(), cx);
    });
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let initial_state = git_page.read_with(cx, |this, cx| {
      this
        .editor_annotation_navigation_state(cx)
        .expect("initial annotation navigation state")
    });
    assert_eq!(initial_state.kind, AnnotationKind::Change);
    assert_eq!(initial_state.total, 2);

    git_page.update_in(cx, |this, _window, cx| {
      this.navigate_annotation_in_editor(AnnotationDirection::Next, cx);
    });

    let next_state = git_page.read_with(cx, |this, cx| {
      this
        .editor_annotation_navigation_state(cx)
        .expect("next annotation navigation state")
    });
    assert_eq!(next_state.kind, AnnotationKind::Change);
    assert_eq!(next_state.total, 2);
    assert_ne!(next_state.active_index, initial_state.active_index);
  }

  #[gpui::test]
  async fn command_palette_merge_branch_fast_forwards_current_branch(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-merge");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let base_branch = current_branch_status(&repo.path).expect("base status").name;
    create_branch(&repo.path, "feature").expect("create feature branch");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let feature_head = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "v2-feature\n",
      "feature change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base");
    force_checkout_head(&repo.path);

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::MergeBranch {
          name: CommandPaletteBranch {
            name: "feature".into(),
            kind: CommandPaletteBranchKind::Local,
          },
        },
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let head = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read head");
    assert_eq!(head.id(), feature_head);
    assert_eq!(
      std::fs::read_to_string(repo.path.join("README.md")).expect("read merged file"),
      "v2-feature\n"
    );
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after merge")
        .name,
      base_branch
    );
  }

  #[cfg(not(target_os = "linux"))]
  #[gpui::test]
  async fn command_palette_rebase_branch_fast_forwards_current_branch(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-rebase");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let base_branch = current_branch_status(&repo.path).expect("base status").name;
    create_branch(&repo.path, "feature").expect("create feature branch");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let feature_head = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "v2-feature\n",
      "feature change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base");
    force_checkout_head(&repo.path);

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::RebaseBranch {
          name: CommandPaletteBranch {
            name: "feature".into(),
            kind: CommandPaletteBranchKind::Local,
          },
        },
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let head = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read head");
    assert_eq!(head.id(), feature_head);
    assert_eq!(
      std::fs::read_to_string(repo.path.join("README.md")).expect("read rebased file"),
      "v2-feature\n"
    );
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after rebase")
        .name,
      base_branch
    );
  }

  #[cfg(not(target_os = "linux"))]
  #[gpui::test]
  async fn command_palette_rebase_branch_with_dirty_worktree_shows_notification_only(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-rebase-dirty-worktree");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let base_branch = current_branch_status(&repo.path).expect("base status").name;
    create_branch(&repo.path, "feature").expect("create feature branch");
    std::fs::write(repo.path.join("README.md"), "dirty change\n")
      .expect("write unstaged change before rebase");

    let mut mounted_git_page = None;
    let (root, cx) = cx.add_window_view(|window, cx| {
      let git_page = cx.new(|cx| GitPage::new_for_test(window, cx));
      mounted_git_page = Some(git_page.clone());
      gpui_component::Root::new(git_page, window, cx)
    });
    let git_page = mounted_git_page.expect("git page");
    cx.executor().allow_parking();

    let initial_notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(initial_notification_count, 0);

    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::RebaseBranch {
          name: CommandPaletteBranch {
            name: "feature".into(),
            kind: CommandPaletteBranchKind::Local,
          },
        },
        _window,
        cx,
      )
    });
    assert!(
      result.is_ok(),
      "dirty worktree rebase failure should close palette and rely on notification feedback"
    );

    await_git_page_background_tasks(git_page.clone(), cx).await;
    cx.cx.run_until_parked();
    cx.run_until_parked();

    let notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(notification_count, 1);
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after failed rebase")
        .name,
      base_branch
    );
    assert!(
      !is_rebase_in_progress(&repo.path).expect("read rebase state after failed rebase"),
      "rebase should not start when the worktree is dirty"
    );
  }

  #[cfg(not(target_os = "linux"))]
  #[gpui::test]
  async fn command_palette_cherry_pick_applies_multiple_commits(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-cherry-pick");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let base_branch = current_branch_status(&repo.path).expect("base status").name;
    create_branch(&repo.path, "feature").expect("create feature branch");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");

    let first = commit_text_file(&repo.path, Path::new("README.md"), "v2\n", "feature 1");
    let second = commit_text_file(&repo.path, Path::new("extra.txt"), "extra\n", "feature 2");

    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base");
    force_checkout_head(&repo.path);

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();
    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::CherryPick {
          commit_hashes: vec![first.to_string(), second.to_string()],
        },
        _window,
        cx,
      )
    });
    assert!(result.is_ok());

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let head = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read head");
    assert_eq!(head.message().unwrap_or_default(), "feature 2");
    let parent = head.parent(0).expect("head parent");
    assert_eq!(parent.message().unwrap_or_default(), "feature 1");
    assert_eq!(
      std::fs::read_to_string(repo.path.join("README.md")).expect("read cherry-picked README"),
      "v2\n"
    );
    assert_eq!(
      std::fs::read_to_string(repo.path.join("extra.txt")).expect("read cherry-picked extra file"),
      "extra\n"
    );
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after cherry-pick")
        .name,
      base_branch
    );
  }

  #[test]
  fn command_palette_cherry_pick_uses_notification_feedback() {
    assert_eq!(
      GitPage::command_palette_error_notification_title(&CommandPaletteAction::CherryPick {
        commit_hashes: vec!["deadbeef".to_string()],
      }),
      Some("Cherry-pick failed")
    );
  }

  #[gpui::test]
  async fn command_palette_stash_and_apply_restore_tracked_changes(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-stash-apply");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("write tracked change");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();
    let stash_result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::Stash {
          include_untracked: false,
          message: None,
        },
        window,
        cx,
      )
    });
    assert!(stash_result.is_ok());
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert_eq!(
      std::fs::read_to_string(repo.path.join(rel_path)).expect("read file after stash"),
      "v1\n"
    );
    let stash = list_stashes(&repo.path)
      .expect("list stashes after stash")
      .into_iter()
      .next()
      .expect("stash entry exists");

    let apply_result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::ApplyStash(CommandPaletteStash {
          index: stash.index,
          name: stash.name.clone().into(),
          oid: stash.oid.clone().into(),
        }),
        window,
        cx,
      )
    });
    assert!(apply_result.is_ok());
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert_eq!(
      std::fs::read_to_string(repo.path.join(rel_path)).expect("read file after apply stash"),
      "v2\n"
    );
    assert_eq!(
      list_stashes(&repo.path)
        .expect("list stashes after apply")
        .len(),
      1
    );
  }

  #[gpui::test]
  async fn command_palette_stash_with_untracked_and_pop_restores_untracked_file(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-stash-pop-untracked");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let rel_path = Path::new("notes.txt");
    std::fs::write(repo.path.join(rel_path), "notes\n").expect("write untracked file");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();
    let stash_result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::Stash {
          include_untracked: true,
          message: None,
        },
        window,
        cx,
      )
    });
    assert!(stash_result.is_ok());
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(
      !repo.path.join(rel_path).exists(),
      "untracked file should be removed after stash"
    );
    let stash = list_stashes(&repo.path)
      .expect("list stashes")
      .into_iter()
      .next()
      .expect("stash entry exists");

    let pop_result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::PopStash(CommandPaletteStash {
          index: stash.index,
          name: stash.name.clone().into(),
          oid: stash.oid.clone().into(),
        }),
        window,
        cx,
      )
    });
    assert!(pop_result.is_ok());
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert_eq!(
      std::fs::read_to_string(repo.path.join(rel_path)).expect("read restored untracked file"),
      "notes\n"
    );
    assert!(
      list_stashes(&repo.path)
        .expect("list stashes after pop")
        .is_empty()
    );
  }

  #[gpui::test]
  async fn command_palette_drop_stash_removes_entry(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-stash-drop");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("write tracked change");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();
    let stash_result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::Stash {
          include_untracked: false,
          message: None,
        },
        window,
        cx,
      )
    });
    assert!(stash_result.is_ok());
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let stash = list_stashes(&repo.path)
      .expect("list stashes")
      .into_iter()
      .next()
      .expect("stash entry exists");

    let drop_result = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::DropStash(CommandPaletteStash {
          index: stash.index,
          name: stash.name.clone().into(),
          oid: stash.oid.clone().into(),
        }),
        window,
        cx,
      )
    });
    assert!(drop_result.is_ok());
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(
      list_stashes(&repo.path)
        .expect("list stashes after drop")
        .is_empty()
    );
    assert_eq!(
      std::fs::read_to_string(repo.path.join(rel_path)).expect("read file after drop stash"),
      "v1\n"
    );
  }

  #[gpui::test]
  async fn command_palette_branch_actions_require_selected_repo(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (git_page, cx) = add_git_page_window_with_root(cx);

    let actions = vec![
      CommandPaletteAction::CheckoutDetached {
        target: "deadbeef".to_string(),
      },
      CommandPaletteAction::Commit,
      CommandPaletteAction::ContinueRebase,
      CommandPaletteAction::SkipRebase,
      CommandPaletteAction::Push,
      CommandPaletteAction::ForcePush,
      CommandPaletteAction::UndoLastCommit,
      CommandPaletteAction::Amend,
      CommandPaletteAction::AcceptAllCurrentConflicts,
      CommandPaletteAction::AcceptAllIncomingConflicts,
      CommandPaletteAction::SwitchBranch(CommandPaletteBranch {
        name: "feature".into(),
        kind: CommandPaletteBranchKind::Local,
      }),
      CommandPaletteAction::CreateBranch {
        name: "feature".to_string(),
      },
      CommandPaletteAction::CreateBranchFrom {
        name: "feature-copy".to_string(),
        base: CommandPaletteBranch {
          name: "feature".into(),
          kind: CommandPaletteBranchKind::Local,
        },
      },
      CommandPaletteAction::DeleteBranch(CommandPaletteBranch {
        name: "feature".into(),
        kind: CommandPaletteBranchKind::Local,
      }),
      CommandPaletteAction::MergeBranch {
        name: CommandPaletteBranch {
          name: "feature".into(),
          kind: CommandPaletteBranchKind::Local,
        },
      },
      CommandPaletteAction::AbortMerge,
      CommandPaletteAction::RebaseBranch {
        name: CommandPaletteBranch {
          name: "feature".into(),
          kind: CommandPaletteBranchKind::Local,
        },
      },
      CommandPaletteAction::InteractiveRebaseBranch {
        name: CommandPaletteBranch {
          name: "feature".into(),
          kind: CommandPaletteBranchKind::Local,
        },
      },
      CommandPaletteAction::InteractiveRebaseEditBranch {
        name: CommandPaletteBranch {
          name: "feature".into(),
          kind: CommandPaletteBranchKind::Local,
        },
      },
      CommandPaletteAction::InteractiveRebaseHeadCount { count: 3 },
      CommandPaletteAction::AbortRebase,
      CommandPaletteAction::StageAll,
      CommandPaletteAction::UnstageAll,
      CommandPaletteAction::StageSelectedFile,
      CommandPaletteAction::UnstageSelectedFile,
      CommandPaletteAction::Fetch,
      CommandPaletteAction::Stash {
        include_untracked: false,
        message: None,
      },
      CommandPaletteAction::ApplyStash(CommandPaletteStash {
        index: 0,
        name: "stash@{0}".into(),
        oid: "deadbeef".into(),
      }),
      CommandPaletteAction::DropStash(CommandPaletteStash {
        index: 0,
        name: "stash@{0}".into(),
        oid: "deadbeef".into(),
      }),
      CommandPaletteAction::PopStash(CommandPaletteStash {
        index: 0,
        name: "stash@{0}".into(),
        oid: "deadbeef".into(),
      }),
      CommandPaletteAction::CherryPick {
        commit_hashes: vec!["deadbeef".to_string()],
      },
    ];

    for action in actions {
      let result = git_page.update_in(cx, |this, _window, cx| {
        this.selected_repo = None;
        this.handle_command_palette_action(action.clone(), _window, cx)
      });
      let error = result.expect_err("action should fail without selected repo");
      assert_eq!(error.as_ref(), "No repository selected.");
    }
  }

  #[gpui::test]
  fn command_palette_includes_delete_branch_root_command_when_available(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-command-palette-delete-available");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "initial\n", "initial");
    create_branch(&repo.path, "feature").expect("create feature branch");
    let current_branch = current_branch_status(&repo.path)
      .expect("read current branch")
      .name;

    let (git_page, cx) = add_git_page_window_with_root(cx);

    let (command_ids, delete_branches) = git_page.update(cx, |this, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.branch_status = Some(make_branch_status(&current_branch, 0, 0, true));
      let contents = this.build_command_palette_contents(1, cx);
      (
        contents
          .commands
          .into_iter()
          .map(|command| command.id)
          .collect::<Vec<_>>(),
        contents.delete_branches,
      )
    });

    assert!(command_ids.contains(&CommandPaletteCommandId::DeleteBranch));
    assert_eq!(
      delete_branches,
      vec![CommandPaletteBranch {
        name: "feature".into(),
        kind: CommandPaletteBranchKind::Local,
      }]
    );
  }

  #[test]
  fn command_palette_rebase_branches_exclude_current_branch_and_prioritize_base_branches() {
    let branches = vec![
      BranchRef {
        name: "feature".into(),
        kind: BranchKind::Local,
      },
      BranchRef {
        name: "topic".into(),
        kind: BranchKind::Local,
      },
      BranchRef {
        name: "main".into(),
        kind: BranchKind::Local,
      },
      BranchRef {
        name: "origin/main".into(),
        kind: BranchKind::Remote,
      },
      BranchRef {
        name: "origin/feature".into(),
        kind: BranchKind::Remote,
      },
    ];

    let rebase_branches = GitPage::command_palette_rebase_branches(
      &branches,
      Some("feature"),
      Some(&BranchRef {
        name: "origin/feature".into(),
        kind: BranchKind::Remote,
      }),
      Some(&BranchRef {
        name: "origin/main".into(),
        kind: BranchKind::Remote,
      }),
    );

    assert_eq!(
      rebase_branches,
      vec![
        CommandPaletteBranch {
          name: "origin/feature".into(),
          kind: CommandPaletteBranchKind::Remote,
        },
        CommandPaletteBranch {
          name: "origin/main".into(),
          kind: CommandPaletteBranchKind::Remote,
        },
        CommandPaletteBranch {
          name: "main".into(),
          kind: CommandPaletteBranchKind::Local,
        },
        CommandPaletteBranch {
          name: "topic".into(),
          kind: CommandPaletteBranchKind::Local,
        },
      ]
    );
  }

  #[gpui::test]
  fn command_palette_rebase_targets_do_not_include_current_local_branch(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-command-palette-rebase-targets");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "initial\n", "initial");
    let current_branch = current_branch_status(&repo.path)
      .expect("read current branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");

    let (git_page, cx) = add_git_page_window_with_root(cx);

    let (branches, rebase_branches) = git_page.update(cx, |this, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.branch_status = Some(make_branch_status(&current_branch, 0, 0, true));
      let contents = this.build_command_palette_contents(1, cx);
      (contents.branches, contents.rebase_branches)
    });

    assert!(branches.contains(&CommandPaletteBranch {
      name: current_branch.clone().into(),
      kind: CommandPaletteBranchKind::Local,
    }));
    assert!(!rebase_branches.contains(&CommandPaletteBranch {
      name: current_branch.into(),
      kind: CommandPaletteBranchKind::Local,
    }));
    assert!(rebase_branches.contains(&CommandPaletteBranch {
      name: "feature".into(),
      kind: CommandPaletteBranchKind::Local,
    }));
  }

  #[gpui::test]
  fn git_file_search_entries_group_changed_files_before_unchanged_project_files(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-file-search-groups");
    let _ = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "initial\n",
      "add readme",
    );
    std::fs::create_dir_all(repo.path.join("src")).expect("create src dir");
    let _ = commit_text_file(
      &repo.path,
      Path::new("src/main.rs"),
      "fn main() {}\n",
      "add main",
    );
    std::fs::write(repo.path.join("README.md"), "updated\n").expect("modify readme");

    let (git_page, cx) = add_git_page_window_with_root(cx);

    let entries = git_page.update(cx, |this, _cx| {
      this.selected_repo = Some(repo.path.clone());
      this.status_entries = list_repo_status(&repo.path).expect("list status");
      this.git_file_search_entries()
    });

    let labels = entries
      .iter()
      .map(|entry| {
        (
          entry.group.as_ref().map(|group| group.to_string()),
          entry.label.to_string(),
        )
      })
      .collect::<Vec<_>>();

    assert_eq!(
      labels,
      vec![
        (Some("Changed".to_string()), "README.md".to_string()),
        (Some("Unchanged".to_string()), "src/main.rs".to_string()),
      ]
    );
  }

  #[gpui::test]
  fn command_palette_includes_remote_branches_in_delete_candidates(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let remote = TempBareRepo::init("git-page-command-palette-delete-remote-origin");
    let source = TempRepo::init("git-page-command-palette-delete-remote-source");
    let clone_dir = TempDir::new("git-page-command-palette-delete-remote-clone");

    let _ = commit_text_file(&source.path, Path::new("README.md"), "initial\n", "initial");
    let source_repo = Repository::open(&source.path).expect("open source repo");
    source_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add source origin");
    let base_branch = current_branch_status(&source.path)
      .expect("read source branch status")
      .name;
    push_branch_to_remote(&source.path, &base_branch, "origin");
    create_branch(&source.path, "feature").expect("create source feature branch");
    push_branch_to_remote(&source.path, "feature", "origin");

    let _clone_repo = Repository::clone(
      remote.path.to_str().expect("remote path utf8"),
      &clone_dir.path,
    )
    .expect("clone remote");
    let clone_branch = current_branch_status(&clone_dir.path)
      .expect("read clone branch status")
      .name;

    let (git_page, cx) = add_git_page_window_with_root(cx);

    let (command_ids, delete_branches) = git_page.update(cx, |this, cx| {
      this.selected_repo = Some(clone_dir.path.clone());
      this.branch_status = Some(make_branch_status(&clone_branch, 0, 0, true));
      let contents = this.build_command_palette_contents(1, cx);
      (
        contents
          .commands
          .into_iter()
          .map(|command| command.id)
          .collect::<Vec<_>>(),
        contents.delete_branches,
      )
    });

    assert!(command_ids.contains(&CommandPaletteCommandId::DeleteBranch));
    assert!(delete_branches.contains(&CommandPaletteBranch {
      name: "origin/feature".into(),
      kind: CommandPaletteBranchKind::Remote,
    }));
  }

  #[gpui::test]
  fn command_palette_hides_delete_branch_root_command_without_candidates(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-command-palette-delete-hidden");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "initial\n", "initial");
    let current_branch = current_branch_status(&repo.path)
      .expect("read current branch")
      .name;

    let (git_page, cx) = add_git_page_window_with_root(cx);

    let (command_ids, delete_branches) = git_page.update(cx, |this, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.branch_status = Some(make_branch_status(&current_branch, 0, 0, true));
      let contents = this.build_command_palette_contents(1, cx);
      (
        contents
          .commands
          .into_iter()
          .map(|command| command.id)
          .collect::<Vec<_>>(),
        contents.delete_branches,
      )
    });

    assert!(!command_ids.contains(&CommandPaletteCommandId::DeleteBranch));
    assert!(delete_branches.is_empty());
  }

  #[gpui::test]
  fn command_palette_moves_open_commands_after_git_commands(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-command-palette-order");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "initial\n", "initial");

    let (git_page, cx) = add_git_page_window_with_root(cx);

    let command_ids = git_page.update(cx, |this, cx| {
      this.selected_repo = Some(repo.path.clone());
      this
        .build_command_palette_contents(2, cx)
        .commands
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>()
    });

    let is_open_command = |id: CommandPaletteCommandId| {
      matches!(
        id,
        CommandPaletteCommandId::OpenRepository
          | CommandPaletteCommandId::OpenGitPage
          | CommandPaletteCommandId::OpenGithubPage
          | CommandPaletteCommandId::OpenGithubFromUrl
          | CommandPaletteCommandId::OpenGitConfigPage
          | CommandPaletteCommandId::OpenSettingsPage
          | CommandPaletteCommandId::OpenBillingPage
          | CommandPaletteCommandId::OpenAboutPage
          | CommandPaletteCommandId::SendFeedback
      )
    };

    let first_open_ix = command_ids
      .iter()
      .position(|id| is_open_command(*id))
      .expect("should include open commands");
    let last_non_open_ix = command_ids
      .iter()
      .rposition(|id| !is_open_command(*id))
      .expect("should include git commands");

    assert!(
      first_open_ix > last_non_open_ix,
      "open commands should be listed after git commands: {command_ids:?}"
    );

    let switch_repository_ix = command_ids
      .iter()
      .position(|id| *id == CommandPaletteCommandId::SwitchRepository)
      .expect("should include switch repository");
    let forget_repository_ix = command_ids
      .iter()
      .position(|id| *id == CommandPaletteCommandId::ForgetRepository)
      .expect("should include forget repository");
    let open_repository_ix = command_ids
      .iter()
      .position(|id| *id == CommandPaletteCommandId::OpenRepository)
      .expect("should include open repository");

    assert_eq!(switch_repository_ix + 1, forget_repository_ix);
    assert_eq!(forget_repository_ix + 1, open_repository_ix);
  }

  #[gpui::test]
  async fn command_palette_merge_branch_opens_first_conflicted_file(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-merge-conflict");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "base\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");

    let _ = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "main change\n",
      "main change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "feature change\n",
      "feature change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base");
    force_checkout_head(&repo.path);

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();
    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::MergeBranch {
          name: CommandPaletteBranch {
            name: "feature".into(),
            kind: CommandPaletteBranchKind::Local,
          },
        },
        _window,
        cx,
      )
    });

    assert!(result.is_ok(), "merge conflict should be handled in-editor");
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let selected_file = git_page.read_with(cx, |this, _| this.selected_file.clone());
    assert_eq!(selected_file.as_deref(), Some(Path::new("README.md")));
    let commit_input_value = git_page.read_with(cx, |this, cx| {
      this.commit_input.read(cx).value().to_string()
    });
    assert_eq!(
      commit_input_value,
      format!("Merge branch 'feature' into {base_branch}")
    );
    let editor_text = git_page.read_with(cx, |this, cx| {
      let editor = this.editor.as_ref().expect("editor opened").clone();
      editor.read_with(cx, |editor, cx| {
        let doc = editor.document().read(cx);
        doc.slice_to_string(0..doc.len())
      })
    });
    assert!(
      editor_text.contains("<<<<<<<"),
      "expected conflict markers in opened editor file: {editor_text}"
    );
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after failed merge")
        .name,
      base_branch
    );
  }

  #[gpui::test]
  async fn abort_merge_action_clears_merge_state(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-abort-merge");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "base\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");

    let _ = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "main change\n",
      "main change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "feature change\n",
      "feature change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base");
    force_checkout_head(&repo.path);

    let _ = merge_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect_err("merge should fail with conflicts");
    assert!(
      is_merge_in_progress(&repo.path).expect("read merge state"),
      "merge state should be active after conflict"
    );

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let reload_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.reload_status(cx);
      this.status_task.take().expect("reload status task")
    });
    reload_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(git_page.read_with(cx, |this, _| this.merge_in_progress));
    git_page.update_in(cx, |this, window, cx| {
      this.commit_input.update(cx, |input, cx| {
        input.set_value("Merge branch 'feature' into main", window, cx)
      });
    });

    let abort_task = git_page.update_in(cx, |this, window, cx| {
      this.abort_merge_action(&gpui::ClickEvent::default(), window, cx);
      this.status_task.take().expect("abort merge task")
    });
    abort_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(
      !is_merge_in_progress(&repo.path).expect("read merge state after abort"),
      "merge state should be cleaned after abort"
    );
    assert_eq!(
      std::fs::read_to_string(repo.path.join("README.md")).expect("read README after abort"),
      "main change\n"
    );
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after abort merge")
        .name,
      base_branch
    );
    assert!(!git_page.read_with(cx, |this, _| this.merge_in_progress));
    assert_eq!(
      git_page.read_with(cx, |this, cx| this
        .commit_input
        .read(cx)
        .value()
        .to_string()),
      ""
    );
  }

  #[gpui::test]
  async fn command_palette_abort_merge_clears_merge_state(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-abort-merge");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "base\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");

    let _ = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "main change\n",
      "main change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "feature change\n",
      "feature change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base");
    force_checkout_head(&repo.path);

    let _ = merge_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect_err("merge should fail with conflicts");
    assert!(
      is_merge_in_progress(&repo.path).expect("read merge state"),
      "merge state should be active after conflict"
    );

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let reload_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.reload_status(cx);
      this.status_task.take().expect("reload status task")
    });
    reload_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(git_page.read_with(cx, |this, _| this.merge_in_progress));
    git_page.update_in(cx, |this, window, cx| {
      this.commit_input.update(cx, |input, cx| {
        input.set_value("Merge branch 'feature' into main", window, cx)
      });
    });

    let result = git_page.update_in(cx, |this, window, cx| {
      this.handle_command_palette_action(CommandPaletteAction::AbortMerge, window, cx)
    });
    assert!(
      result.is_ok(),
      "abort merge via command palette should succeed"
    );
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(
      !is_merge_in_progress(&repo.path).expect("read merge state after abort"),
      "merge state should be cleaned after abort"
    );
    assert_eq!(
      std::fs::read_to_string(repo.path.join("README.md")).expect("read README after abort"),
      "main change\n"
    );
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after abort merge")
        .name,
      base_branch
    );
    assert!(!git_page.read_with(cx, |this, _| this.merge_in_progress));
    assert_eq!(
      git_page.read_with(cx, |this, cx| this
        .commit_input
        .read(cx)
        .value()
        .to_string()),
      ""
    );
  }

  #[cfg(not(target_os = "linux"))]
  #[gpui::test]
  async fn commit_action_completes_merge_after_conflict_resolution(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-commit-merge-resolution");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "base\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");

    let _ = commit_text_file(&repo.path, rel_path, "main change\n", "main change");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(&repo.path, rel_path, "feature change\n", "feature change");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base");
    force_checkout_head(&repo.path);

    let _ = merge_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect_err("merge should fail with conflicts");
    assert!(
      is_merge_in_progress(&repo.path).expect("read merge state"),
      "merge state should be active after conflict"
    );

    std::fs::write(repo.path.join(rel_path), "resolved\n").expect("write resolved contents");
    stage_file(&repo.path, rel_path).expect("stage resolved file");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let reload_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.reload_status(cx);
      this.status_task.take().expect("reload status task")
    });
    reload_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(git_page.read_with(cx, |this, _| this.merge_in_progress));
    assert!(
      !git_page.read_with(cx, |this, _| GitPage::has_conflicted_entries(
        &this.status_entries
      )),
      "conflicts should be resolved before commit"
    );

    let commit_task = git_page.update_in(cx, |this, window, cx| {
      this.commit_input.update(cx, |input, cx| {
        input.set_value("Merge branch 'feature' into main", window, cx)
      });
      this.commit_changes(&gpui::ClickEvent::default(), window, cx);
      this.status_task.take().expect("commit task")
    });
    commit_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(
      !is_merge_in_progress(&repo.path).expect("read merge state after commit"),
      "merge state should be cleaned after commit"
    );
    let repo_handle = Repository::open(&repo.path).expect("open repo after merge commit");
    let head = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read head after merge commit");
    assert_eq!(head.parent_count(), 2);
    assert_eq!(head.summary(), Some("Merge branch 'feature' into main"));
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after merge commit")
        .name,
      base_branch
    );
    assert!(!git_page.read_with(cx, |this, _| this.merge_in_progress));
    assert_eq!(
      git_page.read_with(cx, |this, cx| this
        .commit_input
        .read(cx)
        .value()
        .to_string()),
      ""
    );
  }

  #[cfg(not(target_os = "linux"))]
  #[gpui::test]
  async fn abort_rebase_action_clears_rebase_state(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-abort-rebase");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "base\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");

    let _ = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "main change\n",
      "main change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "feature change\n",
      "feature change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base");
    force_checkout_head(&repo.path);

    let _ = rebase_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect_err("rebase should fail with conflicts");
    assert!(
      is_rebase_in_progress(&repo.path).expect("read rebase state"),
      "rebase state should be active after conflict"
    );

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let reload_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.reload_status(cx);
      this.status_task.take().expect("reload status task")
    });
    reload_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(git_page.read_with(cx, |this, _| this.rebase_in_progress));
    git_page.update_in(cx, |this, window, cx| {
      this.commit_input.update(cx, |input, cx| {
        input.set_value("Rebase branch 'main' onto feature", window, cx)
      });
    });

    let abort_task = git_page.update_in(cx, |this, window, cx| {
      this.abort_rebase_action(&gpui::ClickEvent::default(), window, cx);
      this.status_task.take().expect("abort rebase task")
    });
    abort_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(
      !is_rebase_in_progress(&repo.path).expect("read rebase state after abort"),
      "rebase state should be cleaned after abort"
    );
    assert_eq!(
      std::fs::read_to_string(repo.path.join("README.md")).expect("read README after abort"),
      "main change\n"
    );
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after abort rebase")
        .name,
      base_branch
    );
    assert!(!git_page.read_with(cx, |this, _| this.rebase_in_progress));
    assert_eq!(
      git_page.read_with(cx, |this, cx| this
        .commit_input
        .read(cx)
        .value()
        .to_string()),
      ""
    );
  }

  #[cfg(not(target_os = "linux"))]
  #[gpui::test]
  async fn command_palette_abort_rebase_clears_rebase_state(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-abort-rebase");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "base\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");

    let _ = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "main change\n",
      "main change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "feature change\n",
      "feature change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base");
    force_checkout_head(&repo.path);

    let _ = rebase_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect_err("rebase should fail with conflicts");
    assert!(
      is_rebase_in_progress(&repo.path).expect("read rebase state"),
      "rebase state should be active after conflict"
    );

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let reload_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.reload_status(cx);
      this.status_task.take().expect("reload status task")
    });
    reload_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(git_page.read_with(cx, |this, _| this.rebase_in_progress));
    git_page.update_in(cx, |this, window, cx| {
      this
        .commit_input
        .update(cx, |input, cx| input.set_value("main change", window, cx));
    });

    let result = git_page.update_in(cx, |this, window, cx| {
      this.handle_command_palette_action(CommandPaletteAction::AbortRebase, window, cx)
    });
    assert!(
      result.is_ok(),
      "abort rebase via command palette should succeed"
    );
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(
      !is_rebase_in_progress(&repo.path).expect("read rebase state after abort"),
      "rebase state should be cleaned after abort"
    );
    assert_eq!(
      std::fs::read_to_string(repo.path.join("README.md")).expect("read README after abort"),
      "main change\n"
    );
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after abort rebase")
        .name,
      base_branch
    );
    assert!(!git_page.read_with(cx, |this, _| this.rebase_in_progress));
    assert_eq!(
      git_page.read_with(cx, |this, cx| this
        .commit_input
        .read(cx)
        .value()
        .to_string()),
      ""
    );
  }

  #[cfg(not(target_os = "linux"))]
  #[gpui::test]
  async fn continue_rebase_action_completes_rebase_after_conflict_resolution(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-continue-rebase");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "base\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");

    let _ = commit_text_file(&repo.path, rel_path, "main change\n", "main change");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(&repo.path, rel_path, "feature change\n", "feature change");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base");
    force_checkout_head(&repo.path);

    let _ = rebase_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect_err("rebase should fail with conflicts");
    assert!(
      is_rebase_in_progress(&repo.path).expect("read rebase state"),
      "rebase state should be active after conflict"
    );

    std::fs::write(repo.path.join(rel_path), "resolved\n").expect("write resolved contents");
    stage_file(&repo.path, rel_path).expect("stage resolved file");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let reload_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.reload_status(cx);
      this.status_task.take().expect("reload status task")
    });
    reload_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(git_page.read_with(cx, |this, _| this.rebase_in_progress));
    assert!(
      !git_page.read_with(cx, |this, _| {
        GitPage::has_conflicted_entries(&this.status_entries)
      }),
      "conflicts should be resolved before continue"
    );
    assert_eq!(
      git_page.read_with(cx, |this, cx| this
        .commit_input
        .read(cx)
        .value()
        .to_string()),
      "main change"
    );

    let continue_task = git_page.update_in(cx, |this, window, cx| {
      this.continue_rebase_action(&gpui::ClickEvent::default(), window, cx);
      this.status_task.take().expect("continue rebase task")
    });
    continue_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(
      !is_rebase_in_progress(&repo.path).expect("read rebase state after continue"),
      "rebase state should be cleaned after continue"
    );
    assert_eq!(
      std::fs::read_to_string(repo.path.join(rel_path)).expect("read README after continue"),
      "resolved\n"
    );
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after continue rebase")
        .name,
      base_branch
    );
    assert!(!git_page.read_with(cx, |this, _| this.rebase_in_progress));
    assert_eq!(
      git_page.read_with(cx, |this, cx| this
        .commit_input
        .read(cx)
        .value()
        .to_string()),
      ""
    );
  }

  #[cfg(not(target_os = "linux"))]
  #[gpui::test]
  async fn command_palette_continue_rebase_completes_rebase_after_conflict_resolution(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-continue-rebase");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "base\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");

    let _ = commit_text_file(&repo.path, rel_path, "main change\n", "main change");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(&repo.path, rel_path, "feature change\n", "feature change");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base");
    force_checkout_head(&repo.path);

    let _ = rebase_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect_err("rebase should fail with conflicts");

    std::fs::write(repo.path.join(rel_path), "resolved\n").expect("write resolved contents");
    stage_file(&repo.path, rel_path).expect("stage resolved file");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let reload_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.reload_status(cx);
      this.status_task.take().expect("reload status task")
    });
    reload_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let result = git_page.update_in(cx, |this, window, cx| {
      this.handle_command_palette_action(CommandPaletteAction::ContinueRebase, window, cx)
    });
    assert!(
      result.is_ok(),
      "continue rebase via command palette should succeed"
    );
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(
      !is_rebase_in_progress(&repo.path).expect("read rebase state after continue"),
      "rebase state should be cleaned after continue"
    );
    assert_eq!(
      std::fs::read_to_string(repo.path.join(rel_path)).expect("read README after continue"),
      "resolved\n"
    );
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after continue rebase")
        .name,
      base_branch
    );
    assert!(!git_page.read_with(cx, |this, _| this.rebase_in_progress));
  }

  #[cfg(not(target_os = "linux"))]
  #[gpui::test]
  async fn command_palette_skip_rebase_skips_conflicted_commit(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-skip-rebase");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "base\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");

    let _ = commit_text_file(&repo.path, rel_path, "main change\n", "main change");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(&repo.path, rel_path, "feature change\n", "feature change");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base");
    force_checkout_head(&repo.path);

    let _ = rebase_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect_err("rebase should fail with conflicts");
    assert!(
      is_rebase_in_progress(&repo.path).expect("read rebase state"),
      "rebase state should be active after conflict"
    );

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let reload_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.reload_status(cx);
      this.status_task.take().expect("reload status task")
    });
    reload_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let result = git_page.update_in(cx, |this, window, cx| {
      this.handle_command_palette_action(CommandPaletteAction::SkipRebase, window, cx)
    });
    assert!(
      result.is_ok(),
      "skip rebase via command palette should succeed"
    );
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(
      !is_rebase_in_progress(&repo.path).expect("read rebase state after skip"),
      "rebase state should be cleaned after skip"
    );
    assert_eq!(
      std::fs::read_to_string(repo.path.join(rel_path)).expect("read README after skip"),
      "feature change\n"
    );
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("status after skip rebase")
        .name,
      base_branch
    );
  }

  #[cfg(not(target_os = "linux"))]
  #[gpui::test]
  async fn continue_rebase_action_opens_first_conflicted_file_for_next_conflict(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-continue-rebase-next-conflict");
    let readme_path = Path::new("README.md");
    let notes_path = Path::new("NOTES.txt");
    let _ = commit_text_file(&repo.path, readme_path, "base\n", "initial readme");
    let _ = commit_text_file(&repo.path, notes_path, "base\n", "initial notes");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");

    let _ = commit_text_file(
      &repo.path,
      readme_path,
      "main readme change\n",
      "main readme change",
    );
    let _ = commit_text_file(
      &repo.path,
      notes_path,
      "main notes change\n",
      "main notes change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(
      &repo.path,
      readme_path,
      "feature readme change\n",
      "feature readme change",
    );
    let _ = commit_text_file(
      &repo.path,
      notes_path,
      "feature notes change\n",
      "feature notes change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base");
    force_checkout_head(&repo.path);

    let _ = rebase_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect_err("rebase should fail with first conflict");
    assert!(
      is_rebase_in_progress(&repo.path).expect("read rebase state"),
      "rebase state should be active after first conflict"
    );

    std::fs::write(repo.path.join(readme_path), "resolved readme\n")
      .expect("write resolved first conflict");
    stage_file(&repo.path, readme_path).expect("stage resolved first conflict");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let reload_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.reload_status(cx);
      this.status_task.take().expect("reload status task")
    });
    reload_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let continue_task = git_page.update_in(cx, |this, window, cx| {
      this.continue_rebase_action(&gpui::ClickEvent::default(), window, cx);
      this.status_task.take().expect("continue rebase task")
    });
    continue_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(
      is_rebase_in_progress(&repo.path).expect("read rebase state after continue"),
      "rebase state should remain active due to next conflict"
    );
    assert!(git_page.read_with(cx, |this, _| this.rebase_in_progress));
    assert!(
      git_page.read_with(cx, |this, _| {
        GitPage::has_conflicted_entries(&this.status_entries)
      }),
      "expected conflicted entries after next conflict"
    );
    assert_eq!(
      git_page.read_with(cx, |this, _| this.selected_file.clone()),
      Some(notes_path.to_path_buf())
    );
  }

  #[cfg(not(target_os = "linux"))]
  #[gpui::test]
  async fn command_palette_rebase_branch_opens_first_conflicted_file(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-cmd-rebase-conflict");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "base\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");

    let _ = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "main change\n",
      "main change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(
      &repo.path,
      Path::new("README.md"),
      "feature change\n",
      "feature change",
    );
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base");
    force_checkout_head(&repo.path);

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();
    let result = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.handle_command_palette_action(
        CommandPaletteAction::RebaseBranch {
          name: CommandPaletteBranch {
            name: "feature".into(),
            kind: CommandPaletteBranchKind::Local,
          },
        },
        _window,
        cx,
      )
    });

    assert!(
      result.is_ok(),
      "rebase conflict should be handled in-editor"
    );
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let selected_file = git_page.read_with(cx, |this, _| this.selected_file.clone());
    assert_eq!(selected_file.as_deref(), Some(Path::new("README.md")));
    let commit_input_value = git_page.read_with(cx, |this, cx| {
      this.commit_input.read(cx).value().to_string()
    });
    assert_eq!(commit_input_value, "main change");
    let editor_text = git_page.read_with(cx, |this, cx| {
      let editor = this.editor.as_ref().expect("editor opened").clone();
      editor.read_with(cx, |editor, cx| {
        let doc = editor.document().read(cx);
        doc.slice_to_string(0..doc.len())
      })
    });
    assert!(
      editor_text.contains("<<<<<<<"),
      "expected conflict markers in opened editor file: {editor_text}"
    );
    assert!(
      is_rebase_in_progress(&repo.path).expect("read rebase state"),
      "rebase state should be active after conflict"
    );
  }

  #[gpui::test]
  async fn load_history_commit_files_populates_rows_for_commit(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-history-load-files");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    let commit_oid = commit_text_file(&repo.path, rel_path, "v2\n", "update").to_string();

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.load_history_commit_files(commit_oid.clone(), cx);
      this
        .history_files_task
        .take()
        .expect("history files task should exist")
    });
    task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (rows, still_loading) = git_page.read_with(cx, |this, _| {
      (
        this.history_commit_files.get(commit_oid.as_str()).cloned(),
        this
          .history_commit_files_loading
          .contains(commit_oid.as_str()),
      )
    });
    let rows = rows.expect("loaded history rows for commit");
    assert!(!still_loading);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].path, rel_path);
    assert_eq!(rows[0].kind, CommitFileChangeKind::Modified);
  }

  #[gpui::test]
  async fn open_history_commit_file_loads_readonly_snapshot_content(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-history-open-file");
    let rel_path = Path::new("README.md");
    let old_commit_oid = commit_text_file(&repo.path, rel_path, "v1\n", "initial").to_string();
    let _ = commit_text_file(&repo.path, rel_path, "v2\n", "update");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.open_history_commit_file(old_commit_oid.clone(), rel_path.to_path_buf(), cx);
      this
        .history_open_file_task
        .take()
        .expect("history open file task should exist")
    });
    task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (opened, selected, is_read_only, contents) = git_page.read_with(cx, |this, cx| {
      let editor = this.editor.as_ref().expect("history editor should exist");
      let editor = editor.read(cx);
      let document = editor.document().read(cx);
      (
        this.history_opened_commit_file.clone(),
        this.selected_file.clone(),
        editor.is_read_only,
        document.slice_to_string(0..document.len()),
      )
    });

    assert_eq!(
      opened,
      Some((old_commit_oid.clone(), rel_path.to_path_buf()))
    );
    assert_eq!(selected, Some(rel_path.to_path_buf()));
    assert!(is_read_only);
    assert_eq!(contents, "v1\n");
  }

  #[gpui::test]
  async fn open_history_commit_file_readonly_editor_save_does_not_overwrite_worktree(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-history-readonly-save");
    let rel_path = Path::new("README.md");
    let old_commit_oid = commit_text_file(&repo.path, rel_path, "v1\n", "initial").to_string();
    let _ = commit_text_file(&repo.path, rel_path, "v2\n", "update");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let open_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.open_history_commit_file(old_commit_oid, rel_path.to_path_buf(), cx);
      this
        .history_open_file_task
        .take()
        .expect("history open file task should exist")
    });
    open_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let save_task = git_page.update_in(cx, |this, _window, cx| {
      let editor = this.editor.as_ref().expect("history editor").clone();
      editor.update(cx, |editor, cx| {
        assert!(editor.is_read_only, "history editor must stay readonly");
        editor.save(cx);
        editor.save_task.take()
      })
    });

    assert!(
      save_task.is_none(),
      "readonly editor should not schedule save task"
    );
    assert_eq!(
      std::fs::read_to_string(repo.path.join(rel_path)).expect("read worktree file"),
      "v2\n"
    );
  }

  #[gpui::test]
  async fn open_file_loads_editor_asynchronously(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-open-file-async");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("update worktree file");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.open_file(rel_path.to_path_buf(), cx);
      assert!(
        this.open_file_task.is_some(),
        "open file should schedule an async load task"
      );
      assert_eq!(this.selected_file, Some(rel_path.to_path_buf()));
      assert!(
        this.editor.is_none(),
        "editor should be created after async load"
      );
    });

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (selected_file, is_read_only, contents) = git_page.read_with(cx, |this, cx| {
      let editor = this.editor.as_ref().expect("editor should exist").read(cx);
      let document = editor.document().read(cx);
      (
        this.selected_file.clone(),
        editor.is_read_only,
        document.slice_to_string(0..document.len()),
      )
    });

    assert_eq!(selected_file, Some(rel_path.to_path_buf()));
    assert!(!is_read_only);
    assert_eq!(contents, "v2\n");
  }

  #[gpui::test]
  async fn reload_status_keeps_unchanged_project_file_open(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-clean-project-file-stays-open");
    let rel_path = Path::new("src/main.rs");
    std::fs::create_dir_all(repo.path.join("src")).expect("create source dir");
    let _ = commit_text_file(&repo.path, rel_path, "fn main() {}\n", "initial");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.open_file(rel_path.to_path_buf(), cx);
    });
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let reload_task = git_page.update_in(cx, |this, _window, cx| {
      this.reload_status(cx);
      this.status_task.take().expect("reload status task")
    });
    reload_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    git_page.read_with(cx, |this, _cx| {
      assert_eq!(this.selected_file.as_deref(), Some(rel_path));
      assert_eq!(
        this.selected_file_source,
        Some(SelectedFileSource::ProjectFile)
      );
      assert!(this.editor.is_some());
      assert!(this.status_entries.is_empty());
    });
  }

  #[gpui::test]
  async fn project_file_selection_uses_status_entry_after_file_changes(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-project-file-becomes-changed");
    let rel_path = Path::new("src/main.rs");
    std::fs::create_dir_all(repo.path.join("src")).expect("create source dir");
    let _ = commit_text_file(&repo.path, rel_path, "fn main() {}\n", "initial");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.open_file(rel_path.to_path_buf(), cx);
    });
    await_git_page_background_tasks(git_page.clone(), cx).await;

    std::fs::write(
      repo.path.join(rel_path),
      "fn main() { println!(\"changed\"); }\n",
    )
    .expect("modify project file");

    let reload_task = git_page.update_in(cx, |this, _window, cx| {
      this.reload_status(cx);
      this.status_task.take().expect("reload status task")
    });
    reload_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    git_page.read_with(cx, |this, _cx| {
      assert_eq!(this.selected_file.as_deref(), Some(rel_path));
      assert!(this.editor.is_some());
      assert_eq!(
        this.selected_file_entry().map(|entry| entry.status),
        Some(RepoStatusKind::Modified)
      );
    });
  }

  #[gpui::test]
  async fn raster_image_preview_renders_without_source_editor_pane(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-open-image-preview");
    let rel_path = Path::new("fixtures/image.png");
    let absolute_path = repo.path.join(rel_path);
    std::fs::create_dir_all(
      absolute_path
        .parent()
        .expect("image preview path should have parent"),
    )
    .expect("create image preview parent");
    std::fs::write(&absolute_path, tiny_png_bytes()).expect("write png image");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.open_file(rel_path.to_path_buf(), cx);
    });

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let is_raster_preview = git_page.read_with(cx, |this, _cx| {
      matches!(this.binary_preview, Some(GitBinaryPreview::RasterImage(_)))
    });
    let preview_bounds = cx
      .debug_bounds(GIT_BINARY_PREVIEW_RENDER_DEBUG_SELECTOR)
      .expect("binary preview pane bounds")
      .size;

    assert!(is_raster_preview);
    assert!(preview_bounds.width > gpui::px(0.0));
    assert!(preview_bounds.height > gpui::px(0.0));
    assert!(cx.debug_bounds("editor-whitespace-toggle").is_none());
    assert!(
      cx.debug_bounds(GIT_MARKDOWN_PREVIEW_EDITOR_DEBUG_SELECTOR)
        .is_none()
    );
    assert!(
      cx.debug_bounds(GIT_MARKDOWN_PREVIEW_RENDER_DEBUG_SELECTOR)
        .is_none()
    );
  }

  #[gpui::test]
  async fn unsupported_binary_preview_renders_placeholder(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-open-binary-placeholder");
    let rel_path = Path::new("fixtures/slides.pdf");
    let absolute_path = repo.path.join(rel_path);
    std::fs::create_dir_all(
      absolute_path
        .parent()
        .expect("binary placeholder path should have parent"),
    )
    .expect("create binary placeholder parent");
    std::fs::write(&absolute_path, b"%PDF-1.7\n").expect("write pdf file");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.open_file(rel_path.to_path_buf(), cx);
    });

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let is_placeholder = git_page.read_with(cx, |this, _cx| {
      matches!(
        this.binary_preview,
        Some(GitBinaryPreview::UnsupportedBinary)
      )
    });
    let preview_bounds = cx
      .debug_bounds(GIT_BINARY_PREVIEW_RENDER_DEBUG_SELECTOR)
      .expect("binary placeholder pane bounds")
      .size;

    assert!(is_placeholder);
    assert!(preview_bounds.width > gpui::px(0.0));
    assert!(preview_bounds.height > gpui::px(0.0));
    assert!(cx.debug_bounds("editor-whitespace-toggle").is_none());
  }

  #[gpui::test]
  fn markdown_preview_keeps_editor_and_preview_panes_visible(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempDir::new("git-page-markdown-preview-layout");
    let editor_root = TempDir::new("git-page-markdown-preview-editor-root");
    let rel_path = PathBuf::from("README.md");
    let markdown = "# Preview\n\nThe markdown preview pane should stay visible.\n";
    std::fs::write(repo.path.join(&rel_path), markdown).expect("write markdown file");

    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update_in(cx, |this, _window, cx| {
      let editor_root = editor_root.path.clone();
      let file_path = repo.path.join(&rel_path);
      let rel_path = rel_path.clone();
      let loaded = Editor::load_file_for_editor(&editor_root, &file_path);
      let editor =
        cx.new(move |cx| Editor::new_with_loaded_file(editor_root, file_path, loaded, cx));

      this.selected_repo = Some(repo.path.clone());
      this.selected_file = Some(rel_path);
      this.show_markdown_preview = true;
      this.editor = Some(editor);
      cx.notify();
    });

    let editor_bounds = cx
      .debug_bounds(GIT_MARKDOWN_PREVIEW_EDITOR_DEBUG_SELECTOR)
      .expect("editor preview pane bounds")
      .size;
    let preview_bounds = cx
      .debug_bounds(GIT_MARKDOWN_PREVIEW_RENDER_DEBUG_SELECTOR)
      .expect("render preview pane bounds")
      .size;

    assert!(editor_bounds.width > gpui::px(0.0));
    assert!(editor_bounds.height > gpui::px(0.0));
    assert!(preview_bounds.width > gpui::px(0.0));
    assert!(preview_bounds.height > gpui::px(0.0));
    assert!(cx.debug_bounds("editor-whitespace-toggle").is_some());
  }

  #[gpui::test]
  fn terminal_sidebar_renders_as_a_separate_right_panel(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempDir::new("git-page-terminal-sidebar-layout");
    cx.update(|cx| {
      AuthStateStore::set(
        cx,
        AuthState::Authenticated(Box::new(make_authenticated_test_user(UserRole::Admin))),
      );
    });

    let (git_page, cx) = add_git_page_window_with_root(cx);
    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.show_terminal_sidebar = true;
      cx.notify();
    });

    let terminal_bounds = cx
      .debug_bounds(GIT_TERMINAL_SIDEBAR_DEBUG_SELECTOR)
      .expect("terminal sidebar bounds")
      .size;

    assert!(terminal_bounds.width > gpui::px(0.0));
    assert!(terminal_bounds.height > gpui::px(0.0));
  }

  #[gpui::test]
  fn open_terminal_action_toggles_embedded_terminal_sidebar(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempDir::new("git-page-terminal-sidebar-action");
    cx.update(|cx| {
      AuthStateStore::set(
        cx,
        AuthState::Authenticated(Box::new(make_authenticated_test_user(UserRole::Admin))),
      );
    });

    let (git_page, cx) = add_git_page_window_with_root(cx);
    git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.focus_page(window, cx);
    });

    git_page.update_in(cx, |this, window, cx| {
      this.toggle_terminal_sidebar_action(&crate::ToggleTerminalSidebar, window, cx);
      assert!(this.show_terminal_sidebar);
    });

    git_page.update_in(cx, |this, window, cx| {
      this.toggle_terminal_sidebar_action(&crate::ToggleTerminalSidebar, window, cx);
      assert!(!this.show_terminal_sidebar);
    });
  }

  #[gpui::test]
  fn non_admin_users_do_not_render_terminal_button_or_sidebar(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempDir::new("git-page-terminal-sidebar-non-admin");
    cx.update(|cx| {
      AuthStateStore::set(
        cx,
        AuthState::Authenticated(Box::new(make_authenticated_test_user(UserRole::Pro))),
      );
    });

    let (git_page, cx) = add_git_page_window_with_root(cx);
    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.show_terminal_sidebar = true;
      cx.notify();
    });

    assert!(
      cx.debug_bounds(GIT_TERMINAL_BUTTON_DEBUG_SELECTOR)
        .is_none(),
      "non-admin users should not see the terminal button"
    );
    assert!(
      cx.debug_bounds(GIT_TERMINAL_SIDEBAR_DEBUG_SELECTOR)
        .is_none(),
      "non-admin users should not render the terminal sidebar"
    );
  }

  #[gpui::test]
  fn non_admin_terminal_action_does_not_open_embedded_terminal_sidebar(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempDir::new("git-page-terminal-sidebar-non-admin-action");
    cx.update(|cx| {
      AuthStateStore::set(
        cx,
        AuthState::Authenticated(Box::new(make_authenticated_test_user(UserRole::Pro))),
      );
    });

    let (git_page, cx) = add_git_page_window_with_root(cx);
    git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.focus_page(window, cx);
    });

    git_page.update_in(cx, |this, window, cx| {
      this.toggle_terminal_sidebar_action(&crate::ToggleTerminalSidebar, window, cx);
      assert!(!this.show_terminal_sidebar);
    });
  }

  #[gpui::test]
  async fn open_file_replaces_history_snapshot_when_same_path_is_selected(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-open-file-after-history");
    let rel_path = Path::new("README.md");
    let old_commit_oid = commit_text_file(&repo.path, rel_path, "v1\n", "initial").to_string();
    let _ = commit_text_file(&repo.path, rel_path, "v2\n", "update");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let history_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.open_history_commit_file(old_commit_oid.clone(), rel_path.to_path_buf(), cx);
      this
        .history_open_file_task
        .take()
        .expect("history open file task should exist")
    });
    history_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (before_opened, before_read_only, before_contents) = git_page.read_with(cx, |this, cx| {
      let editor = this.editor.as_ref().expect("history editor should exist");
      let editor = editor.read(cx);
      let document = editor.document().read(cx);
      (
        this.history_opened_commit_file.clone(),
        editor.is_read_only,
        document.slice_to_string(0..document.len()),
      )
    });
    assert_eq!(
      before_opened,
      Some((old_commit_oid.clone(), rel_path.to_path_buf()))
    );
    assert!(before_read_only);
    assert_eq!(before_contents, "v1\n");

    git_page.update_in(cx, |this, _window, cx| {
      this.open_file(rel_path.to_path_buf(), cx);
    });
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (opened, is_read_only, contents) = git_page.read_with(cx, |this, cx| {
      let editor = this.editor.as_ref().expect("editor should exist");
      let editor = editor.read(cx);
      let document = editor.document().read(cx);
      (
        this.history_opened_commit_file.clone(),
        editor.is_read_only,
        document.slice_to_string(0..document.len()),
      )
    });

    assert_eq!(opened, None);
    assert!(!is_read_only);
    assert_eq!(contents, "v2\n");
  }

  #[gpui::test]
  async fn queue_history_commit_files_load_skips_cached_loading_and_pending_commits(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update_in(cx, |this, window, cx| {
      let cached_oid = "cached-oid".to_string();
      this.history_commit_files.insert(
        cached_oid.clone(),
        vec![make_history_file(
          "README.md",
          CommitFileChangeKind::Modified,
        )],
      );
      this.queue_history_commit_files_load(cached_oid.clone(), window, cx);
      assert!(
        !this
          .pending_history_file_loads
          .contains(cached_oid.as_str())
      );

      let loading_oid = "loading-oid".to_string();
      this
        .history_commit_files_loading
        .insert(loading_oid.clone());
      this.queue_history_commit_files_load(loading_oid.clone(), window, cx);
      assert!(
        !this
          .pending_history_file_loads
          .contains(loading_oid.as_str())
      );

      let pending_oid = "pending-oid".to_string();
      this.pending_history_file_loads.insert(pending_oid.clone());
      this.queue_history_commit_files_load(pending_oid.clone(), window, cx);
      assert!(
        this
          .pending_history_file_loads
          .contains(pending_oid.as_str())
      );
      assert_eq!(this.pending_history_file_loads.len(), 1);
    });
  }

  #[gpui::test]
  async fn load_history_commit_files_with_invalid_oid_clears_loading_and_stale_rows(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-history-load-invalid");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let invalid_oid = "0123456789012345678901234567890123456789".to_string();

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.history_commit_files.insert(
        invalid_oid.clone(),
        vec![make_history_file(
          "README.md",
          CommitFileChangeKind::Modified,
        )],
      );
      this.load_history_commit_files(invalid_oid.clone(), cx);
      this
        .history_files_task
        .take()
        .expect("history files task should exist")
    });
    task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (rows, loading) = git_page.read_with(cx, |this, _| {
      (
        this.history_commit_files.get(invalid_oid.as_str()).cloned(),
        this
          .history_commit_files_loading
          .contains(invalid_oid.as_str()),
      )
    });
    assert!(rows.is_none());
    assert!(!loading);
  }

  #[test]
  fn external_branch_switch_updates_branch_status_and_branch_select_model() {
    let repo = TempRepo::init("git-page-external-switch");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let initial_status = current_branch_status(&repo.path).expect("read initial branch");
    let initial_branch_name = initial_status.name.clone();
    create_branch(&repo.path, "feature").expect("create feature branch");

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    repo_handle
      .set_head("refs/heads/feature")
      .expect("set HEAD to feature");

    let switched_status = current_branch_status(&repo.path).expect("read switched branch");
    assert_eq!(switched_status.name, "feature");
    assert!(GitPage::branch_name_changed(
      Some(&initial_status),
      Some(&switched_status)
    ));

    let branches = list_branches(&repo.path).expect("list branches");
    let selected = GitPage::selected_branch_from_status(Some(&switched_status));
    let items = GitPage::branch_select_items(branches, selected.as_ref(), None);

    assert_eq!(items.iter().filter(|item| item.is_current).count(), 1);
    assert!(
      items
        .iter()
        .any(|item| item.branch.kind == BranchKind::Local
          && item.branch.name == "feature"
          && item.is_current)
    );
    if initial_branch_name != "feature" {
      assert!(!items.iter().any(|item| {
        item.branch.kind == BranchKind::Local
          && item.branch.name == initial_branch_name
          && item.is_current
      }));
    }
  }

  #[test]
  fn external_detached_head_selects_detached_entry_in_branch_select_model() {
    let repo = TempRepo::init("git-page-external-detach");
    let oid = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let initial_status = current_branch_status(&repo.path).expect("read initial branch");

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    repo_handle.set_head_detached(oid).expect("detach HEAD");

    let detached_status = current_branch_status(&repo.path).expect("read detached status");
    assert_eq!(detached_status.name, "HEAD");
    assert!(GitPage::branch_name_changed(
      Some(&initial_status),
      Some(&detached_status)
    ));

    let branches = list_branches(&repo.path).expect("list branches");
    let selected = GitPage::selected_branch_from_status(Some(&detached_status));
    assert_eq!(selected, Some(GitPage::detached_branch_select_value()));
    let detached_label = detached_head_label(&repo.path).ok();
    let items =
      GitPage::branch_select_items(branches, selected.as_ref(), detached_label.as_deref());
    assert!(
      items
        .iter()
        .any(|item| { GitPage::is_detached_branch_select_value(&item.branch) && item.is_current }),
      "detached HEAD entry should be selected"
    );
  }

  #[gpui::test]
  async fn commit_changes_inner_requires_selected_repo(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update_in(cx, |this, window, cx| {
      this.status_entries = vec![make_status_entry("README.md", RepoStage::Unstaged)];
      this
        .commit_input
        .update(cx, |input, cx| input.set_value("feat: message", window, cx));

      this.commit_changes_inner(window, cx);
      assert!(this.status_task.is_none());
    });
  }

  #[gpui::test]
  async fn commit_changes_inner_requires_non_empty_message_and_changes(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-commit-guards");
    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.status_entries = vec![make_status_entry("README.md", RepoStage::Unstaged)];

      this
        .commit_input
        .update(cx, |input, cx| input.set_value("   ", window, cx));
      this.commit_changes_inner(window, cx);
      assert!(this.status_task.is_none());

      this
        .commit_input
        .update(cx, |input, cx| input.set_value("feat: message", window, cx));
      this.status_entries.clear();
      this.commit_changes_inner(window, cx);
      assert!(this.status_task.is_none());
    });
  }

  #[gpui::test]
  async fn push_changes_action_requires_selected_repo_and_push_capability(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-push-guards");
    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.can_push = false;
      this.push_changes_action(cx);
      assert!(this.status_task.is_none());
      assert!(!this.push_pull_in_progress);

      this.selected_repo = None;
      this.can_push = true;
      this.push_changes_action(cx);
      assert!(this.status_task.is_none());
      assert!(!this.push_pull_in_progress);
    });
  }

  #[gpui::test]
  async fn pull_changes_action_requires_selected_repo_and_respects_existing_sync(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-pull-shortcut-guards");
    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = None;
      this.pull_changes_action(&crate::PullChanges, window, cx);
      assert!(this.status_task.is_none());
      assert!(!this.push_pull_in_progress);

      this.selected_repo = Some(repo.path.clone());
      this.push_pull_in_progress = true;
      this.pull_changes_action(&crate::PullChanges, window, cx);
      assert!(this.status_task.is_none());
      assert!(this.push_pull_in_progress);
    });
  }

  #[gpui::test]
  async fn force_push_changes_action_requires_selected_repo_and_force_capability(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-force-push-guards");
    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.can_force_push = false;
      this.force_push_changes_action(cx);
      assert!(this.status_task.is_none());
      assert!(!this.push_pull_in_progress);

      this.selected_repo = None;
      this.can_force_push = true;
      this.force_push_changes_action(cx);
      assert!(this.status_task.is_none());
      assert!(!this.push_pull_in_progress);
    });
  }

  #[gpui::test]
  async fn undo_last_commit_action_requires_selected_repo_and_undo_capability(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-undo-guards");
    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.can_undo_last_commit = false;
      this.undo_last_commit_action(cx);
      assert!(this.status_task.is_none());

      this.selected_repo = None;
      this.can_undo_last_commit = true;
      this.undo_last_commit_action(cx);
      assert!(this.status_task.is_none());
    });
  }

  #[gpui::test]
  async fn push_changes_action_pushes_to_remote_when_allowed(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let source = TempRepo::init("git-page-push-success-source");
    let remote = TempBareRepo::init("git-page-push-success-remote");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&source.path, rel_path, "v1\n", "initial");

    let source_repo = Repository::open(&source.path).expect("open source");
    source_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add origin remote");
    let branch_name = current_branch_status(&source.path)
      .expect("source branch status")
      .name;
    push_branch_to_remote(&source.path, &branch_name, "origin");
    set_upstream(&source.path, &branch_name, &format!("origin/{branch_name}"));
    set_remote_head(&remote.path, &branch_name);

    let _ = commit_text_file(&source.path, rel_path, "v2-source\n", "source change");
    let expected_head = head_oid(&source.path);
    assert_ne!(
      remote_branch_oid(&remote.path, &branch_name),
      expected_head,
      "remote should be behind before push"
    );

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let push_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(source.path.clone());
      this.force_push_after_rebase = true;
      this.can_push = true;
      this.push_changes_action(cx);
      this.status_task.take().expect("push task")
    });
    assert!(git_page.read_with(cx, |this, _| this.push_pull_in_progress));
    push_task.await;

    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    assert_eq!(remote_branch_oid(&remote.path, &branch_name), expected_head);
    let status = current_branch_status(&source.path).expect("status after push");
    assert_eq!(status.ahead, 0);
    assert!(!git_page.read_with(cx, |this, _| this.force_push_after_rebase));
    assert!(!git_page.read_with(cx, |this, _| this.push_pull_in_progress));
  }

  #[gpui::test]
  async fn publish_branch_and_create_pr_action_publishes_branch_and_opens_dialog(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    cx.executor().allow_parking();
    cx.update(|cx| {
      AuthStateStore::set(
        cx,
        AuthState::Authenticated(Box::new(make_authenticated_test_user(UserRole::Pro))),
      );
    });

    let source = TempRepo::init("git-page-publish-pr-source");
    let remote = TempBareRepo::init("git-page-publish-pr-remote");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&source.path, rel_path, "v1\n", "initial");

    let source_repo = Repository::open(&source.path).expect("open source");
    source_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add origin remote");
    source_repo
      .remote("github", "git@github.com:acme/widget.git")
      .expect("add github remote");

    let branch_name = current_branch_status(&source.path)
      .expect("source branch status")
      .name;

    let details_body = r#"{
      "name": "widget",
      "full_name": "acme/widget",
      "description": "A sample repository",
      "homepage": null,
      "language": "Rust",
      "default_branch": "main",
      "stargazers_count": 0,
      "forks_count": 0,
      "subscribers_count": 0,
      "open_issues_count": 0,
      "size": 1,
      "pushed_at": "2026-03-20T12:00:00Z",
      "html_url": "https://github.com/acme/widget",
      "owner": {
        "login": "acme",
        "avatar_url": "https://example.com/avatar.png"
      },
      "license": null
    }"#;
    let tree_body = r#"{
      "sha": "head123",
      "url": "https://api.github.com/repos/acme/widget/trees/head123",
      "tree": [],
      "truncated": false
    }"#;
    let (base_url, handle) = start_matching_response_server(vec![
      (
        format!("GET /github/repos/acme/widget/pr/branch?branch={branch_name}"),
        "200 OK".to_string(),
        r#"{"pullRequest":null}"#.to_string(),
      ),
      (
        "GET /github/repos/acme/widget HTTP/1.1".to_string(),
        "200 OK".to_string(),
        details_body.to_string(),
      ),
      (
        "GET /github/repos/acme/widget/trees/main?recursive=1 HTTP/1.1".to_string(),
        "200 OK".to_string(),
        tree_body.to_string(),
      ),
    ]);

    let mut mounted_git_page = None;
    let (_root, cx) = cx.add_window_view(|window, cx| {
      let git_page = cx.new(|cx| GitPage::new_for_test(window, cx));
      mounted_git_page = Some(git_page.clone());
      gpui_component::Root::new(git_page, window, cx)
    });
    let git_page = mounted_git_page.expect("git page");

    let publish_task = git_page.update_in(cx, |this, _window, cx| {
      this.api = make_test_api_client(base_url.clone());
      this.selected_repo = Some(source.path.clone());
      this.branch_status = Some(current_branch_status(&source.path).expect("branch status"));
      this.has_unpublished_branch_commits = true;
      this.sync_active_local_repo(cx);
      this.publish_branch_and_create_pull_request_action(cx);
      assert!(this.push_pull_in_progress);
      assert!(this.publish_branch_and_create_pr_in_progress);
      this.status_task.take().expect("publish task")
    });
    publish_task.await;

    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    cx.cx.run_until_parked();
    cx.run_until_parked();
    cx.cx.run_until_parked();
    cx.run_until_parked();
    handle.join().expect("join server thread");

    let status = current_branch_status(&source.path).expect("status after publish");
    assert!(status.has_upstream);
    assert_eq!(
      remote_branch_oid(&remote.path, &branch_name),
      head_oid(&source.path)
    );
    assert!(!git_page.read_with(cx, |this, _| this.push_pull_in_progress));
    assert!(!git_page.read_with(cx, |this, _| this.publish_branch_and_create_pr_in_progress));
  }

  #[gpui::test]
  async fn force_push_changes_action_force_pushes_when_allowed(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let source = TempRepo::init("git-page-force-push-source");
    let remote = TempBareRepo::init("git-page-force-push-remote");
    let peer = TempDir::new("git-page-force-push-peer");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&source.path, rel_path, "v1\n", "initial");

    let source_repo = Repository::open(&source.path).expect("open source");
    source_repo
      .remote("origin", remote.path.to_str().expect("remote path utf8"))
      .expect("add origin remote");
    let branch_name = current_branch_status(&source.path)
      .expect("source branch status")
      .name;
    push_branch_to_remote(&source.path, &branch_name, "origin");
    set_upstream(&source.path, &branch_name, &format!("origin/{branch_name}"));
    set_remote_head(&remote.path, &branch_name);

    let _ = Repository::clone(remote.path.to_str().expect("remote path utf8"), &peer.path)
      .expect("clone remote into peer");

    let _ = commit_text_file(&source.path, rel_path, "v2-source\n", "source change");
    let expected_head = head_oid(&source.path);

    let _ = commit_text_file(&peer.path, rel_path, "v2-peer\n", "peer change");
    push_branch_to_remote(&peer.path, &branch_name, "origin");

    let non_force = push(&source.path, false).err();
    assert!(non_force.is_some(), "non-force push should fail");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let force_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(source.path.clone());
      this.force_push_after_rebase = true;
      this.can_force_push = true;
      this.force_push_changes_action(cx);
      this.status_task.take().expect("force push task")
    });
    assert!(git_page.read_with(cx, |this, _| this.push_pull_in_progress));
    force_task.await;

    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    assert_eq!(remote_branch_oid(&remote.path, &branch_name), expected_head);
    assert!(!git_page.read_with(cx, |this, _| this.force_push_after_rebase));
    assert!(!git_page.read_with(cx, |this, _| this.push_pull_in_progress));
  }

  #[gpui::test]
  async fn stage_restore_actions_require_selected_repo(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = None;

      this.stage_all_action(cx);
      assert!(this.status_task.is_none());

      this.unstage_all_action(cx);
      assert!(this.status_task.is_none());

      this.stage_file_action(PathBuf::from("README.md"), cx);
      assert!(this.status_task.is_none());

      this.unstage_file_action(PathBuf::from("README.md"), cx);
      assert!(this.status_task.is_none());

      this.restore_file_action(
        PathBuf::from("README.md"),
        None,
        RepoStatusKind::Modified,
        cx,
      );
      assert!(this.status_task.is_none());
    });
  }

  #[gpui::test]
  async fn stage_all_action_stages_all_modified_entries(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-stage-all-success");
    let first = Path::new("a.txt");
    let second = Path::new("b.txt");
    let _ = commit_text_file(&repo.path, first, "a1\n", "first");
    let _ = commit_text_file(&repo.path, second, "b1\n", "second");
    std::fs::write(repo.path.join(first), "a2\n").expect("modify first");
    std::fs::write(repo.path.join(second), "b2\n").expect("modify second");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.stage_all_action(cx);
      this.status_task.take().expect("stage all task")
    });
    task.await;
    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    let entries = list_repo_status(&repo.path).expect("list status after stage all");
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().all(|entry| entry.stage == RepoStage::Staged));
    let has_staged = git_page.read_with(cx, |this, _| this.has_staged_changes);
    assert!(has_staged);
  }

  #[gpui::test]
  async fn unstage_all_action_unstages_all_modified_entries(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-unstage-all-success");
    let first = Path::new("a.txt");
    let second = Path::new("b.txt");
    let _ = commit_text_file(&repo.path, first, "a1\n", "first");
    let _ = commit_text_file(&repo.path, second, "b1\n", "second");
    std::fs::write(repo.path.join(first), "a2\n").expect("modify first");
    std::fs::write(repo.path.join(second), "b2\n").expect("modify second");
    stage_all(&repo.path).expect("stage all before ui action");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.unstage_all_action(cx);
      this.status_task.take().expect("unstage all task")
    });
    task.await;
    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    let entries = list_repo_status(&repo.path).expect("list status after unstage all");
    assert_eq!(entries.len(), 2);
    assert!(
      entries
        .iter()
        .all(|entry| entry.stage == RepoStage::Unstaged)
    );
    let has_staged = git_page.read_with(cx, |this, _| this.has_staged_changes);
    assert!(!has_staged);
  }

  #[gpui::test]
  async fn toggle_stage_all_action_unstages_when_all_entries_are_staged(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-toggle-stage-all-to-unstage");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("modify file");
    stage_all(&repo.path).expect("stage all before toggle action");
    let staged_entries = list_repo_status(&repo.path).expect("list staged status");
    assert!(
      staged_entries
        .iter()
        .all(|entry| entry.stage == RepoStage::Staged)
    );

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let task = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.status_entries = staged_entries.clone();
      this.toggle_stage_all_action(&gpui::ClickEvent::default(), window, cx);
      this.status_task.take().expect("toggle stage-all task")
    });
    task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let entries = list_repo_status(&repo.path).expect("list status after toggle unstage");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, rel_path);
    assert_eq!(entries[0].stage, RepoStage::Unstaged);
  }

  #[gpui::test]
  async fn toggle_stage_all_action_stages_when_any_entry_is_unstaged(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-toggle-stage-all-to-stage");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("modify file");
    let unstaged_entries = list_repo_status(&repo.path).expect("list unstaged status");
    assert!(
      unstaged_entries
        .iter()
        .all(|entry| entry.stage == RepoStage::Unstaged)
    );

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let task = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.status_entries = unstaged_entries.clone();
      this.toggle_stage_all_action(&gpui::ClickEvent::default(), window, cx);
      this.status_task.take().expect("toggle stage-all task")
    });
    task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let entries = list_repo_status(&repo.path).expect("list status after toggle stage");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, rel_path);
    assert_eq!(entries[0].stage, RepoStage::Staged);
  }

  #[gpui::test]
  async fn stage_file_action_stages_only_target_file(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-stage-file-success");
    let first = Path::new("a.txt");
    let second = Path::new("b.txt");
    let _ = commit_text_file(&repo.path, first, "a1\n", "first");
    let _ = commit_text_file(&repo.path, second, "b1\n", "second");
    std::fs::write(repo.path.join(first), "a2\n").expect("modify first");
    std::fs::write(repo.path.join(second), "b2\n").expect("modify second");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.stage_file_action(first.to_path_buf(), cx);
      this.status_task.take().expect("stage file task")
    });
    task.await;
    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    let entries = list_repo_status(&repo.path).expect("list status after stage file");
    let first_entry = entries
      .iter()
      .find(|entry| entry.path == first)
      .expect("first entry");
    let second_entry = entries
      .iter()
      .find(|entry| entry.path == second)
      .expect("second entry");
    assert_eq!(first_entry.stage, RepoStage::Staged);
    assert_eq!(second_entry.stage, RepoStage::Unstaged);
  }

  #[gpui::test]
  async fn stage_file_action_with_missing_path_keeps_existing_status(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-stage-file-missing");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("modify tracked file");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();
    let task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.stage_file_action(PathBuf::from("missing.txt"), cx);
      this.status_task.take().expect("stage missing file task")
    });
    task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let entries = list_repo_status(&repo.path).expect("status after stage missing file");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, rel_path);
    assert_eq!(entries[0].stage, RepoStage::Unstaged);
  }

  #[gpui::test]
  async fn stage_file_action_failure_shows_error_notification(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let missing_repo = TempDir::new("git-page-stage-file-failure-notification");
    let missing_repo_path = missing_repo.path.clone();
    std::fs::remove_dir_all(&missing_repo.path).expect("remove temp dir");

    let mut mounted_git_page = None;
    let (root, cx) = cx.add_window_view(|window, cx| {
      let git_page = cx.new(|cx| GitPage::new_for_test(window, cx));
      mounted_git_page = Some(git_page.clone());
      gpui_component::Root::new(git_page, window, cx)
    });
    let git_page = mounted_git_page.expect("git page");
    cx.executor().allow_parking();

    let initial_notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(initial_notification_count, 0);

    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(missing_repo_path.clone());
      this.stage_file_action(PathBuf::from("README.md"), cx);
    });
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(notification_count, 1);
  }

  #[gpui::test]
  async fn unstage_file_action_unstages_target_file(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-unstage-file-success");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("modify file");
    stage_file(&repo.path, rel_path).expect("stage file before ui action");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.unstage_file_action(rel_path.to_path_buf(), cx);
      this.status_task.take().expect("unstage file task")
    });
    task.await;
    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    let entries = list_repo_status(&repo.path).expect("list status after unstage file");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, rel_path);
    assert_eq!(entries[0].stage, RepoStage::Unstaged);
  }

  #[gpui::test]
  async fn unstage_file_action_with_missing_path_keeps_existing_status(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-unstage-file-missing");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("modify tracked file");
    stage_file(&repo.path, rel_path).expect("stage tracked file");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();
    let task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.unstage_file_action(PathBuf::from("missing.txt"), cx);
      this.status_task.take().expect("unstage missing file task")
    });
    task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let entries = list_repo_status(&repo.path).expect("status after unstage missing file");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, rel_path);
    assert_eq!(entries[0].stage, RepoStage::Staged);
  }

  #[gpui::test]
  async fn restore_file_action_reverts_modified_file(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-restore-file-success");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("modify file");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.restore_file_action(rel_path.to_path_buf(), None, RepoStatusKind::Modified, cx);
      this.status_task.take().expect("restore file task")
    });
    task.await;
    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    let contents = std::fs::read_to_string(repo.path.join(rel_path)).expect("read restored file");
    assert_eq!(contents, "v1\n");
    assert!(
      list_repo_status(&repo.path)
        .expect("status after restore")
        .is_empty()
    );
  }

  #[gpui::test]
  async fn restore_file_action_with_missing_path_keeps_existing_changes(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-restore-file-missing");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("modify tracked file");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();
    let task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.restore_file_action(
        PathBuf::from("missing.txt"),
        None,
        RepoStatusKind::Modified,
        cx,
      );
      this.status_task.take().expect("restore missing file task")
    });
    task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let contents = std::fs::read_to_string(repo.path.join(rel_path)).expect("read modified file");
    assert_eq!(contents, "v2\n");
    let entries = list_repo_status(&repo.path).expect("status after restore missing file");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, rel_path);
    assert_eq!(entries[0].stage, RepoStage::Unstaged);
  }

  #[gpui::test]
  async fn restore_file_action_deletes_untracked_file(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-delete-untracked-success");
    let _ = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let rel_path = Path::new("notes.txt");
    let absolute = repo.path.join(rel_path);
    std::fs::write(&absolute, "temporary\n").expect("write untracked file");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.restore_file_action(rel_path.to_path_buf(), None, RepoStatusKind::Untracked, cx);
      this.status_task.take().expect("delete untracked task")
    });
    task.await;
    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    assert!(!absolute.exists());
    assert!(
      list_repo_status(&repo.path)
        .expect("status after delete")
        .is_empty()
    );
  }

  #[gpui::test]
  async fn restore_file_action_restores_deleted_tracked_file(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-restore-deleted-file");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    let absolute = repo.path.join(rel_path);
    std::fs::remove_file(&absolute).expect("delete tracked file in worktree");

    let entries_before = list_repo_status(&repo.path).expect("list status before restore");
    assert_eq!(entries_before.len(), 1);
    assert_eq!(entries_before[0].path, rel_path);
    assert_eq!(entries_before[0].status, RepoStatusKind::Deleted);

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.restore_file_action(rel_path.to_path_buf(), None, RepoStatusKind::Deleted, cx);
      this.status_task.take().expect("restore deleted file task")
    });
    task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(absolute.exists());
    let contents = std::fs::read_to_string(&absolute).expect("read restored tracked file");
    assert_eq!(contents, "v1\n");
    assert!(
      list_repo_status(&repo.path)
        .expect("status after deleted restore")
        .is_empty()
    );
  }

  #[gpui::test]
  async fn restore_file_action_undoes_rename(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-restore-rename");
    let old_path = Path::new("old.txt");
    let new_path = Path::new("new.txt");
    let _ = commit_text_file(&repo.path, old_path, "v1\n", "initial");
    std::fs::rename(repo.path.join(old_path), repo.path.join(new_path))
      .expect("rename file in worktree");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.restore_file_action(
        new_path.to_path_buf(),
        Some(old_path.to_path_buf()),
        RepoStatusKind::Renamed,
        cx,
      );
      this.status_task.take().expect("restore rename task")
    });
    task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(repo.path.join(old_path).exists());
    assert!(!repo.path.join(new_path).exists());
    let contents =
      std::fs::read_to_string(repo.path.join(old_path)).expect("read restored old file");
    assert_eq!(contents, "v1\n");
    assert!(
      list_repo_status(&repo.path)
        .expect("status after rename restore")
        .is_empty()
    );
  }

  #[gpui::test]
  async fn restore_file_action_selects_first_remaining_file(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-restore-select-first-remaining");
    let first_path = Path::new("a-first.txt");
    let second_path = Path::new("b-second.txt");
    let _ = commit_text_file(&repo.path, first_path, "v1\n", "initial first");
    let _ = commit_text_file(&repo.path, second_path, "v1\n", "initial second");
    std::fs::write(repo.path.join(first_path), "first change\n").expect("modify first file");
    std::fs::write(repo.path.join(second_path), "second change\n").expect("modify second file");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let reload_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.reload_status(cx);
      this.status_task.take().expect("reload status task")
    });
    reload_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (restore_path, expected_first_remaining_path) = git_page.read_with(cx, |this, _| {
      assert_eq!(
        this.status_entries.len(),
        2,
        "expected two modified files before restore"
      );
      (
        this.status_entries[1].path.clone(),
        this.status_entries[0].path.clone(),
      )
    });

    git_page.update_in(cx, |this, _window, cx| {
      this.open_file(restore_path.clone(), cx);
    });

    let restore_task = git_page.update_in(cx, |this, _window, cx| {
      this.restore_file_action(restore_path.clone(), None, RepoStatusKind::Modified, cx);
      this.status_task.take().expect("restore file task")
    });
    restore_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (selected_file, entries_len, first_remaining_path) = git_page.read_with(cx, |this, _| {
      (
        this.selected_file.clone(),
        this.status_entries.len(),
        this.status_entries.first().map(|entry| entry.path.clone()),
      )
    });

    assert_eq!(entries_len, 1);
    assert_eq!(first_remaining_path, Some(expected_first_remaining_path));
    assert_eq!(selected_file, first_remaining_path);
  }

  #[gpui::test]
  async fn restore_all_action_restores_tracked_and_deletes_untracked(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-restore-all");
    let tracked_path = Path::new("README.md");
    let untracked_path = Path::new("notes.txt");
    let _ = commit_text_file(&repo.path, tracked_path, "v1\n", "initial");
    std::fs::write(repo.path.join(tracked_path), "v2\n").expect("modify tracked file");
    std::fs::write(repo.path.join(untracked_path), "temporary\n").expect("write untracked file");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let reload_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.reload_status(cx);
      this.status_task.take().expect("reload status task")
    });
    reload_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let restore_all_task = git_page.update_in(cx, |this, _window, cx| {
      this.restore_all_action(cx);
      this.status_task.take().expect("restore all task")
    });
    restore_all_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert_eq!(
      std::fs::read_to_string(repo.path.join(tracked_path)).expect("read tracked file"),
      "v1\n"
    );
    assert!(!repo.path.join(untracked_path).exists());
    assert!(
      list_repo_status(&repo.path)
        .expect("status after restore all")
        .is_empty()
    );
  }

  #[gpui::test]
  async fn restore_all_action_undoes_renamed_files(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-restore-all-rename");
    let old_path = Path::new("old.txt");
    let new_path = Path::new("new.txt");
    let _ = commit_text_file(&repo.path, old_path, "v1\n", "initial");
    // Stage the rename so libgit2 reports it as a single Renamed entry.
    std::fs::rename(repo.path.join(old_path), repo.path.join(new_path))
      .expect("rename file in worktree");
    stage_all(&repo.path).expect("stage rename");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let reload_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.reload_status(cx);
      this.status_task.take().expect("reload status task")
    });
    reload_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let entries_before = git_page.read_with(cx, |this, _| this.status_entries.clone());
    assert_eq!(entries_before.len(), 1);
    assert_eq!(entries_before[0].status, RepoStatusKind::Renamed);
    assert_eq!(entries_before[0].path, new_path);
    assert_eq!(entries_before[0].old_path.as_deref(), Some(old_path));

    let restore_all_task = git_page.update_in(cx, |this, _window, cx| {
      this.restore_all_action(cx);
      this.status_task.take().expect("restore all task")
    });
    restore_all_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    assert!(repo.path.join(old_path).exists());
    assert!(!repo.path.join(new_path).exists());
    assert_eq!(
      std::fs::read_to_string(repo.path.join(old_path)).expect("read restored file"),
      "v1\n"
    );
    assert!(
      list_repo_status(&repo.path)
        .expect("status after restore all rename")
        .is_empty()
    );
  }

  #[cfg(not(target_os = "linux"))]
  #[gpui::test]
  async fn commit_changes_inner_stages_and_commits_when_ready(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-commit-success");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("update file");
    let entries = list_repo_status(&repo.path).expect("list status after edit");
    assert!(!entries.is_empty());

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let commit_task = git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.status_entries = entries.clone();
      this.has_staged_changes = false;
      this.commit_input.update(cx, |input, cx| {
        input.set_value("  feat: update readme  ", window, cx)
      });

      this.commit_changes_inner(window, cx);
      this.status_task.take().expect("commit task")
    });
    commit_task.await;

    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let head = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read head");
    assert_eq!(head.summary(), Some("feat: update readme"));
    assert!(
      list_repo_status(&repo.path)
        .expect("status after commit")
        .is_empty()
    );

    let input_value = git_page.read_with(cx, |this, cx| {
      this.commit_input.read(cx).value().to_string()
    });
    assert!(input_value.is_empty());
  }

  #[cfg(not(target_os = "linux"))]
  #[gpui::test]
  async fn commit_input_secondary_enter_stages_and_commits_when_ready(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-commit-secondary-enter");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    std::fs::write(repo.path.join(rel_path), "v2\n").expect("update file");
    let entries = list_repo_status(&repo.path).expect("list status after edit");
    assert!(!entries.is_empty());

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.status_entries = entries.clone();
      this.has_staged_changes = false;
      this.commit_input.update(cx, |input, cx| {
        input.set_value("feat: secondary enter commit", window, cx)
      });

      this.commit_input.update(cx, |_input, cx| {
        cx.emit(InputEvent::PressEnter { secondary: true })
      });
    });

    cx.cx.run_until_parked();
    cx.run_until_parked();

    let commit_task = git_page.update_in(cx, |this, _window, _cx| {
      this.status_task.take().expect("commit task")
    });
    commit_task.await;

    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let head = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("read head");
    assert_eq!(head.summary(), Some("feat: secondary enter commit"));
    assert!(
      list_repo_status(&repo.path)
        .expect("status after commit")
        .is_empty()
    );

    let input_value = git_page.read_with(cx, |this, cx| {
      this.commit_input.read(cx).value().to_string()
    });
    assert!(input_value.is_empty());
  }

  #[gpui::test]
  async fn undo_last_commit_action_moves_head_when_allowed(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-undo-success");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "first");
    let _ = commit_text_file(&repo.path, rel_path, "v2\n", "second");

    let repo_handle = Repository::open(&repo.path).expect("open repo");
    let expected_parent = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("head before undo")
      .parent(0)
      .expect("parent")
      .id();

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let undo_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.can_undo_last_commit = true;
      this.undo_last_commit_action(cx);
      this.status_task.take().expect("undo task")
    });
    undo_task.await;

    if let Some(reload_task) = git_page.update_in(cx, |this, _window, _| this.status_task.take()) {
      reload_task.await;
    }

    let head_after = repo_handle
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("head after undo")
      .id();
    assert_eq!(head_after, expected_parent);
  }

  #[gpui::test]
  async fn poll_once_updates_branch_status_after_external_branch_switch(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-poll-once-switch");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    create_branch(&repo.path, "feature").expect("create feature branch");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();
    git_page.update_in(cx, |this, _window, cx| {
      seed_repo_branch_state(this, &repo.path, cx);
    });

    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("read branch after switch")
        .name,
      "feature"
    );

    let status_task = git_page.update_in(cx, |this, _window, cx| {
      this.poll_once_for_test(cx);
      this.status_task.take().expect("poll status task")
    });
    status_task.await;
    let branch_name = git_page.read_with(cx, |this, _| {
      this
        .branch_status
        .as_ref()
        .map(|status| status.name.clone())
    });
    assert_eq!(branch_name.as_deref(), Some("feature"));
  }

  #[gpui::test]
  async fn poll_once_does_not_keep_manual_refresh_stuck_loading(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-poll-once-refresh-race");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let reload_task = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.reload_status(cx);
      assert!(this.status_refresh_in_progress);
      this.status_task.take().expect("reload status task")
    });
    let poll_task = git_page.update_in(cx, |this, _window, cx| {
      this.poll_once_for_test(cx);
      this.status_task.take().expect("poll status task")
    });

    reload_task.await;
    poll_task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    cx.update(|_, cx| {
      assert!(
        !GitPageHandle::is_refreshing(cx),
        "manual refresh should not stay stuck after poll runs"
      );
    });
  }

  #[gpui::test]
  async fn branch_select_switch_keeps_status_empty_when_target_branch_is_clean(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-branch-switch-clean-status");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "main\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(&repo.path, rel_path, "feature\n", "feature commit");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base branch");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    let initial_reload = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.reload_status(cx);
      this.status_task.take().expect("initial reload task")
    });
    initial_reload.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let initial_entries = git_page.read_with(cx, |this, _| this.status_entries.clone());
    assert!(
      initial_entries.is_empty(),
      "base branch should start clean, got: {initial_entries:?}"
    );

    git_page.update_in(cx, |this, _window, cx| {
      this.handle_branch_select_confirm(
        BranchRef {
          name: "feature".to_string(),
          kind: BranchKind::Local,
        },
        cx,
      );
    });
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (entries, branch_name, selected_branch) = git_page.read_with(cx, |this, _| {
      (
        this.status_entries.clone(),
        this
          .branch_status
          .as_ref()
          .map(|status| status.name.clone()),
        selected_branch_from_dropdown(this),
      )
    });

    assert!(
      entries.is_empty(),
      "feature branch should stay clean after switch, got: {entries:?}"
    );
    assert_eq!(branch_name.as_deref(), Some("feature"));
    assert_eq!(
      selected_branch,
      Some(BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      })
    );
  }

  #[gpui::test]
  async fn branch_select_switch_failure_shows_error_notification(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-branch-switch-failure-notification");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "main\n", "initial");
    let base_branch = current_branch_status(&repo.path)
      .expect("read base branch")
      .name;
    create_branch(&repo.path, "feature").expect("create feature branch");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    let _ = commit_text_file(&repo.path, rel_path, "feature\n", "feature commit");
    switch_branch(
      &repo.path,
      &BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch back to base branch");
    std::fs::write(repo.path.join(rel_path), "main local change\n").expect("write local change");

    let mut mounted_git_page = None;
    let (root, cx) = cx.add_window_view(|window, cx| {
      let git_page = cx.new(|cx| GitPage::new_for_test(window, cx));
      mounted_git_page = Some(git_page.clone());
      gpui_component::Root::new(git_page, window, cx)
    });
    let git_page = mounted_git_page.expect("git page");
    cx.executor().allow_parking();

    let initial_reload = git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.reload_status(cx);
      this.status_task.take().expect("initial reload task")
    });
    initial_reload.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let initial_notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(initial_notification_count, 0);

    git_page.update_in(cx, |this, _window, cx| {
      this.handle_branch_select_confirm(
        BranchRef {
          name: "feature".to_string(),
          kind: BranchKind::Local,
        },
        cx,
      );
    });
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (branch_name, selected_branch) = git_page.update_in(cx, |this, _window, _cx| {
      (
        this
          .branch_status
          .as_ref()
          .map(|status| status.name.clone()),
        selected_branch_from_dropdown(this),
      )
    });
    let notification_count = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });

    assert_eq!(
      current_branch_status(&repo.path)
        .expect("read branch after failed switch")
        .name,
      base_branch
    );
    assert_eq!(branch_name.as_deref(), Some(base_branch.as_str()));
    assert_eq!(
      selected_branch,
      Some(BranchRef {
        name: base_branch.clone(),
        kind: BranchKind::Local,
      })
    );
    assert_eq!(notification_count, 1);

    cx.executor().advance_clock(Duration::from_secs(5));
    cx.cx.run_until_parked();
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.cx.run_until_parked();
    cx.run_until_parked();

    let notification_count_after_autohide = root.read_with(cx, |root, cx| {
      root.notification.read(cx).notifications().len()
    });
    assert_eq!(notification_count_after_autohide, 0);
  }

  #[gpui::test]
  async fn poll_once_selects_detached_entry_on_external_detached_head(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-poll-once-detached");
    let oid = commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();
    git_page.update_in(cx, |this, _window, cx| {
      seed_repo_branch_state(this, &repo.path, cx);
    });

    Repository::open(&repo.path)
      .expect("open repo")
      .set_head_detached(oid)
      .expect("detach HEAD");

    let status_task = git_page.update_in(cx, |this, _window, cx| {
      this.poll_once_for_test(cx);
      this.status_task.take().expect("poll status task")
    });
    status_task.await;
    if let Some(branch_task) = git_page.update_in(cx, |this, _window, _| this.branch_task.take()) {
      branch_task.await;
    }

    let (branch_name, selected_branch) = git_page.read_with(cx, |this, _cx| {
      (
        this
          .branch_status
          .as_ref()
          .map(|status| status.name.clone()),
        selected_branch_from_dropdown(this),
      )
    });
    assert_eq!(branch_name.as_deref(), Some("HEAD"));
    assert_eq!(
      selected_branch,
      Some(GitPage::detached_branch_select_value())
    );
  }

  #[gpui::test]
  async fn refresh_branches_updates_branch_select_after_external_branch_switch(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-refresh-branches-switch");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    create_branch(&repo.path, "feature").expect("create feature branch");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();
    git_page.update_in(cx, |this, _window, cx| {
      seed_repo_branch_state(this, &repo.path, cx);
    });

    switch_branch(
      &repo.path,
      &BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      },
    )
    .expect("switch to feature");
    assert_eq!(
      current_branch_status(&repo.path)
        .expect("read branch after switch")
        .name,
      "feature"
    );

    let branch_task = git_page.update_in(cx, |this, _window, cx| {
      this.refresh_branches(cx);
      this.branch_task.take().expect("refresh branches task")
    });
    branch_task.await;

    let selected_branch = git_page.read_with(cx, |this, _cx| selected_branch_from_dropdown(this));
    assert_eq!(
      selected_branch,
      Some(BranchRef {
        name: "feature".to_string(),
        kind: BranchKind::Local,
      })
    );
  }

  #[gpui::test]
  async fn reload_status_clears_git_state_when_selected_repo_becomes_unavailable(
    cx: &mut TestAppContext,
  ) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-reload-missing-repo");
    let rel_path = Path::new("README.md");
    let _ = commit_text_file(&repo.path, rel_path, "v1\n", "initial");
    create_branch(&repo.path, "feature").expect("create feature branch");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();
    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      seed_repo_branch_state(this, &repo.path, cx);
      this.status_entries = vec![make_status_entry("README.md", RepoStage::Unstaged)];
      this.branch_status = Some(current_branch_status(&repo.path).expect("read branch status"));
      this.has_head_commit = true;
      this.can_undo_last_commit = true;
      this.can_push = true;
      this.can_force_push = true;
      this.force_push_after_rebase = true;
      this.has_staged_changes = true;
      this.selected_file = Some(rel_path.to_path_buf());
    });

    std::fs::remove_dir_all(&repo.path).expect("remove repo root");

    let task = git_page.update_in(cx, |this, _window, cx| {
      this.reload_status(cx);
      this.status_task.take().expect("reload status task")
    });
    task.await;
    await_git_page_background_tasks(git_page.clone(), cx).await;

    let (
      status_entries_len,
      branch_status,
      has_head_commit,
      can_undo_last_commit,
      can_push,
      can_force_push,
      force_push_after_rebase,
      has_staged_changes,
      selected_file,
      selected_branch,
    ) = git_page.read_with(cx, |this, _cx| {
      (
        this.status_entries.len(),
        this.branch_status.clone(),
        this.has_head_commit,
        this.can_undo_last_commit,
        this.can_push,
        this.can_force_push,
        this.force_push_after_rebase,
        this.has_staged_changes,
        this.selected_file.clone(),
        selected_branch_from_dropdown(this),
      )
    });

    assert_eq!(status_entries_len, 0);
    assert!(branch_status.is_none());
    assert!(!has_head_commit);
    assert!(!can_undo_last_commit);
    assert!(!can_push);
    assert!(!can_force_push);
    assert!(!force_push_after_rebase);
    assert!(!has_staged_changes);
    assert!(selected_file.is_none());
    assert!(selected_branch.is_none());
  }

  #[test]
  fn should_apply_branch_refresh_rejects_stale_generation_or_repo_mismatch() {
    let repo = Path::new("/tmp/repo");
    let other_repo = Path::new("/tmp/other");

    assert!(GitPage::should_apply_branch_refresh(Some(repo), repo, 3, 3));
    assert!(!GitPage::should_apply_branch_refresh(
      Some(repo),
      repo,
      4,
      3
    ));
    assert!(!GitPage::should_apply_branch_refresh(
      Some(other_repo),
      repo,
      3,
      3
    ));
    assert!(!GitPage::should_apply_branch_refresh(None, repo, 3, 3));
  }

  #[test]
  fn should_apply_status_refresh_rejects_stale_generation_or_repo_mismatch() {
    let repo = Path::new("/tmp/repo");
    let other_repo = Path::new("/tmp/other");

    assert!(GitPage::should_apply_status_refresh(Some(repo), repo, 3, 3));
    assert!(!GitPage::should_apply_status_refresh(
      Some(repo),
      repo,
      4,
      3
    ));
    assert!(!GitPage::should_apply_status_refresh(
      Some(other_repo),
      repo,
      3,
      3
    ));
    assert!(!GitPage::should_apply_status_refresh(None, repo, 3, 3));
  }

  #[test]
  fn status_poll_interval_slows_down_for_inactive_window() {
    assert_eq!(
      GitPage::status_poll_interval(true),
      Duration::from_millis(STATUS_POLL_INTERVAL_MS)
    );
    assert_eq!(
      GitPage::status_poll_interval(false),
      INACTIVE_STATUS_POLL_INTERVAL
    );
  }

  #[test]
  fn should_poll_status_requires_active_window_repo_and_idle_refresh() {
    let repo = Path::new("/tmp/repo");

    assert!(GitPage::should_poll_status(true, Some(repo), false));
    assert!(!GitPage::should_poll_status(false, Some(repo), false));
    assert!(!GitPage::should_poll_status(true, None, false));
    assert!(!GitPage::should_poll_status(true, Some(repo), true));
  }

  #[test]
  fn unpublished_branch_check_derives_tracked_branch_from_ahead_count() {
    let repo = Path::new("/tmp/repo");
    let branch = make_branch_status("main", 2, 0, true);
    let (has_unpublished_commits, check_key, checked) =
      GitPage::resolve_polled_unpublished_branch_commits(repo, Some(&branch), None, false, false)
        .expect("tracked branch should not need the expensive unpublished check");

    assert!(has_unpublished_commits);
    assert!(checked);
    assert_eq!(
      check_key,
      Some(UnpublishedBranchCheckKey {
        repo_root: repo.to_path_buf(),
        branch_name: "main".to_string(),
        ahead: 2,
        behind: 0,
        has_upstream: true,
        head_sha: None,
      })
    );
  }

  #[test]
  fn unpublished_branch_check_reuses_cached_local_branch_key_until_forced_or_changed() {
    let repo = Path::new("/tmp/repo");
    let branch = make_branch_status("feature", 0, 0, false);
    let cached_key =
      GitPage::unpublished_branch_check_key(repo, &branch, Some("head-a".to_string()));
    let same_key = GitPage::unpublished_branch_check_key(repo, &branch, Some("head-a".to_string()));
    let changed_head_key =
      GitPage::unpublished_branch_check_key(repo, &branch, Some("head-b".to_string()));
    let changed_repo_key = GitPage::unpublished_branch_check_key(
      Path::new("/tmp/other-repo"),
      &branch,
      Some("head-a".to_string()),
    );

    assert!(!GitPage::should_recheck_unpublished_branch(
      &same_key,
      Some(&cached_key),
      false
    ));
    assert!(GitPage::should_recheck_unpublished_branch(
      &same_key,
      Some(&cached_key),
      true
    ));
    assert!(GitPage::should_recheck_unpublished_branch(
      &changed_head_key,
      Some(&cached_key),
      false
    ));
    assert!(GitPage::should_recheck_unpublished_branch(
      &changed_repo_key,
      Some(&cached_key),
      false
    ));

    let cached_headless_key = GitPage::unpublished_branch_check_key(repo, &branch, None);
    let (has_unpublished_commits, returned_key, checked) =
      GitPage::resolve_polled_unpublished_branch_commits(
        repo,
        Some(&branch),
        Some(&cached_headless_key),
        true,
        false,
      )
      .expect("unchanged key should reuse cached unpublished state");
    assert!(has_unpublished_commits);
    assert_eq!(returned_key, Some(cached_headless_key));
    assert!(!checked);
  }

  #[test]
  fn branch_refresh_guard_ignores_stale_result_after_repo_switch() {
    let repo_a = TempRepo::init("git-page-refresh-stale-a");
    commit_text_file(&repo_a.path, Path::new("README.md"), "a1\n", "initial");
    create_branch(&repo_a.path, "alpha").expect("create alpha branch");
    Repository::open(&repo_a.path)
      .expect("open repo a")
      .set_head("refs/heads/alpha")
      .expect("set HEAD to alpha");

    let repo_b = TempRepo::init("git-page-refresh-stale-b");
    commit_text_file(&repo_b.path, Path::new("README.md"), "b1\n", "initial");

    let repo_a_status = current_branch_status(&repo_a.path).expect("read repo a status");
    let repo_a_items = GitPage::branch_select_items(
      list_branches(&repo_a.path).expect("list repo a branches"),
      GitPage::selected_branch_from_status(Some(&repo_a_status)).as_ref(),
      None,
    );
    assert!(
      repo_a_items
        .iter()
        .any(|item| item.branch.name == "alpha" && item.is_current)
    );

    // Simulate two in-flight refreshes:
    // 1) old refresh requested for repo A at generation 1
    // 2) user switches repo, new refresh requested for repo B at generation 2
    let stale_request_generation = 1;
    let active_generation = 2;
    assert!(!GitPage::should_apply_branch_refresh(
      Some(repo_b.path.as_path()),
      repo_a.path.as_path(),
      active_generation,
      stale_request_generation
    ));
    assert!(GitPage::should_apply_branch_refresh(
      Some(repo_b.path.as_path()),
      repo_b.path.as_path(),
      active_generation,
      active_generation
    ));

    let repo_b_status = current_branch_status(&repo_b.path).expect("read repo b status");
    let repo_b_items = GitPage::branch_select_items(
      list_branches(&repo_b.path).expect("list repo b branches"),
      GitPage::selected_branch_from_status(Some(&repo_b_status)).as_ref(),
      None,
    );
    assert_eq!(
      repo_b_items.iter().filter(|item| item.is_current).count(),
      1
    );
    assert!(
      repo_b_items
        .iter()
        .any(|item| item.branch.name == repo_b_status.name && item.is_current)
    );
    assert!(
      !repo_b_items
        .iter()
        .any(|item| item.branch.name == "alpha" && item.is_current)
    );
  }

  #[test]
  fn should_refresh_editor_for_path_only_when_selected_matches() {
    let selected = Path::new("src/main.rs");
    let other = Path::new("src/lib.rs");

    assert!(GitPage::should_refresh_editor_for_path(
      Some(selected),
      selected
    ));
    assert!(!GitPage::should_refresh_editor_for_path(
      Some(selected),
      other
    ));
    assert!(!GitPage::should_refresh_editor_for_path(None, selected));
  }

  #[test]
  fn should_show_editor_loading_state_only_when_file_selected_without_editor() {
    let selected = Path::new("src/main.rs");
    assert!(GitPage::should_show_editor_loading_state(
      Some(selected),
      false
    ));
    assert!(!GitPage::should_show_editor_loading_state(
      Some(selected),
      true
    ));
    assert!(!GitPage::should_show_editor_loading_state(None, false));
  }

  #[test]
  fn should_show_open_action_loading_state_only_for_pending_repo_open_actions() {
    let action = GitPageOpenAction::MergeBaseBranch {
      base_branch_name: "main".to_string(),
    };
    let selected = Path::new("src/main.rs");

    assert!(GitPage::should_show_open_action_loading_state(
      Some(&action),
      None,
      false,
    ));
    assert!(!GitPage::should_show_open_action_loading_state(
      Some(&action),
      Some(selected),
      false,
    ));
    assert!(!GitPage::should_show_open_action_loading_state(
      Some(&action),
      None,
      true,
    ));
    assert!(!GitPage::should_show_open_action_loading_state(
      None, None, false,
    ));
  }

  #[test]
  fn repository_split_is_hidden_when_no_repo_is_selected() {
    assert!(!GitPage::should_render_repository_split(None));
    assert!(GitPage::should_render_repository_split(Some(Path::new(
      "/tmp/reviu-selected-repo"
    ))));
  }

  #[test]
  fn restore_uses_delete_only_for_untracked_entries() {
    assert!(GitPage::restore_uses_delete(RepoStatusKind::Untracked));
    assert!(!GitPage::restore_uses_delete(RepoStatusKind::Modified));
    assert!(!GitPage::restore_uses_delete(RepoStatusKind::Added));
    assert!(!GitPage::restore_uses_delete(RepoStatusKind::Deleted));
  }

  #[test]
  fn stage_requires_confirmation_only_for_conflicted_entries() {
    assert!(GitPage::stage_requires_confirmation(
      RepoStatusKind::Conflicted
    ));
    assert!(!GitPage::stage_requires_confirmation(
      RepoStatusKind::Modified
    ));
    assert!(!GitPage::stage_requires_confirmation(RepoStatusKind::Added));
  }

  #[test]
  fn should_confirm_stage_for_status_only_when_conflicts_are_unresolved() {
    assert!(GitPage::should_confirm_stage_for_status(
      Some(RepoStatusKind::Conflicted),
      true
    ));
    assert!(!GitPage::should_confirm_stage_for_status(
      Some(RepoStatusKind::Conflicted),
      false
    ));
    assert!(!GitPage::should_confirm_stage_for_status(
      Some(RepoStatusKind::Modified),
      true
    ));
    assert!(!GitPage::should_confirm_stage_for_status(None, true));
  }

  #[test]
  fn sidebar_toggle_stage_action_preserves_partial_split_behavior() {
    assert_eq!(
      GitPage::sidebar_toggle_stage_action(RepoStage::Unstaged, false, false),
      FileStageButtonAction::Stage
    );
    assert_eq!(
      GitPage::sidebar_toggle_stage_action(RepoStage::Staged, false, false),
      FileStageButtonAction::Unstage
    );
    assert_eq!(
      GitPage::sidebar_toggle_stage_action(RepoStage::PartiallyStaged, false, false),
      FileStageButtonAction::Unstage
    );
    assert_eq!(
      GitPage::sidebar_toggle_stage_action(RepoStage::PartiallyStaged, true, false),
      FileStageButtonAction::Stage
    );
    assert_eq!(
      GitPage::sidebar_toggle_stage_action(RepoStage::PartiallyStaged, true, true),
      FileStageButtonAction::Unstage
    );
  }

  #[test]
  fn all_changes_staged_requires_non_empty_and_only_staged_entries() {
    assert!(!GitPage::all_entries_staged(&[]));

    let all_staged = vec![
      make_status_entry("src/a.rs", RepoStage::Staged),
      make_status_entry("src/b.rs", RepoStage::Staged),
    ];
    assert!(GitPage::all_entries_staged(&all_staged));

    let mixed = vec![
      make_status_entry("src/a.rs", RepoStage::Staged),
      make_status_entry("src/b.rs", RepoStage::Unstaged),
    ];
    assert!(!GitPage::all_entries_staged(&mixed));

    let partial = vec![make_status_entry("src/a.rs", RepoStage::PartiallyStaged)];
    assert!(!GitPage::all_entries_staged(&partial));
  }

  #[test]
  fn history_change_kind_mapping_covers_all_variants() {
    assert_eq!(
      GitPage::history_change_kind_to_repo_status(CommitFileChangeKind::Added),
      RepoStatusKind::Added
    );
    assert_eq!(
      GitPage::history_change_kind_to_repo_status(CommitFileChangeKind::Deleted),
      RepoStatusKind::Deleted
    );
    assert_eq!(
      GitPage::history_change_kind_to_repo_status(CommitFileChangeKind::Modified),
      RepoStatusKind::Modified
    );
    assert_eq!(
      GitPage::history_change_kind_to_repo_status(CommitFileChangeKind::Renamed),
      RepoStatusKind::Renamed
    );
    assert_eq!(
      GitPage::history_change_kind_to_repo_status(CommitFileChangeKind::Copied),
      RepoStatusKind::Renamed
    );
    assert_eq!(
      GitPage::history_change_kind_to_repo_status(CommitFileChangeKind::Typechange),
      RepoStatusKind::TypeChange
    );
    assert_eq!(
      GitPage::history_change_kind_to_repo_status(CommitFileChangeKind::Conflicted),
      RepoStatusKind::Conflicted
    );
  }

  #[test]
  fn hunk_action_top_uses_local_display_line_position() {
    let top = GitPage::hunk_action_top(gpui::px(20.0), 110, 109.0);
    assert_eq!(top, gpui::px(20.0));
  }

  #[test]
  fn hunk_action_top_handles_fractional_scroll_offset() {
    let top = GitPage::hunk_action_top(gpui::px(18.0), 10, 9.5);
    assert_eq!(top, gpui::px(9.0));
  }
}
