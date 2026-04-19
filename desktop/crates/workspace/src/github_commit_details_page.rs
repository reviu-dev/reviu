use std::{
  collections::{BTreeMap, HashMap},
  path::{Path, PathBuf},
  rc::Rc,
  sync::Arc,
};

use editor::{DiffViewMode, Editor};
use git::{DiffKind, DiffSet, FileDiff, compute_buffer_diff};
use gpui::{
  AnyElement, App, Context, Entity, FocusHandle, Focusable, ParentElement, Render, SharedString,
  Styled, Task, Window, div, img, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt,
  avatar::Avatar,
  button::{Button, ButtonVariants as _},
  h_flex,
  label::Label,
  skeleton::Skeleton,
  tag::Tag,
  tree::{TreeItem, TreeState, tree},
  v_flex,
};
use smol::unblock;
use ui::{
  CommandPalette, CommandPaletteAction, CommandPaletteCommand, CommandPaletteConfig,
  CommandPaletteHandler, CommandPalettePage, FILE_ICON_SIZE_PX, SelectableRowStyle,
  StatusThemeExt as _, UiIconName, WindowExt, file_icon_path_for_name_with_theme, h_resizable,
  resizable_panel, selectable_list_item,
};

use crate::{
  ShowCommandPalette,
  api::{ApiClient, GithubCommitDetails, GithubPullRequestFile},
  auth_state::AuthStateStore,
  config::AppSettings,
  date_format::format_relative_time,
  file_preview::{FilePreviewKind, file_preview_kind},
  github_navigation::{open_pr_target, open_repo_target},
  github_page::GithubPageHandle,
  github_shared,
  navigation::NavigationHistory,
  workspace::WorkspaceApi,
};

const COMMIT_FILE_SIDEBAR_WIDTH: f32 = 360.0;
const COMMIT_FILE_SIDEBAR_MIN_WIDTH: f32 = 320.0;
const COMMIT_FILE_SIDEBAR_MAX_WIDTH: f32 = 1200.0;
const COMMIT_HEADER_HEIGHT: f32 = 40.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GithubCommitFileStatus {
  Added,
  Modified,
  Deleted,
  Renamed,
}

fn commit_file_status(status: &str) -> GithubCommitFileStatus {
  match status.trim().to_ascii_lowercase().as_str() {
    "added" => GithubCommitFileStatus::Added,
    "removed" | "deleted" => GithubCommitFileStatus::Deleted,
    "renamed" => GithubCommitFileStatus::Renamed,
    _ => GithubCommitFileStatus::Modified,
  }
}

fn commit_file_status_letter(status: GithubCommitFileStatus) -> &'static str {
  match status {
    GithubCommitFileStatus::Added => "A",
    GithubCommitFileStatus::Modified => "M",
    GithubCommitFileStatus::Deleted => "D",
    GithubCommitFileStatus::Renamed => "R",
  }
}

fn commit_file_status_color(
  status: GithubCommitFileStatus,
  theme: &gpui_component::Theme,
) -> gpui::Hsla {
  match status {
    GithubCommitFileStatus::Modified => theme.status_orange(),
    GithubCommitFileStatus::Added => theme.status_green(),
    GithubCommitFileStatus::Deleted => theme.status_red(),
    GithubCommitFileStatus::Renamed => theme.info,
  }
}

#[derive(Clone, Debug)]
struct GithubCommitFileDiff {
  path: SharedString,
  old_path: Option<SharedString>,
  status: GithubCommitFileStatus,
  patch: Option<SharedString>,
}

#[derive(Clone, Debug)]
struct GithubCommitFileContents {
  base: Option<String>,
  head: Option<String>,
}

fn commit_files_from_api(files: Vec<GithubPullRequestFile>) -> Vec<Rc<GithubCommitFileDiff>> {
  files
    .into_iter()
    .map(|file| {
      let status = commit_file_status(file.status.as_str());
      let path = if file.filename.is_empty() {
        "unknown".to_string()
      } else {
        file.filename
      };
      let old_path = if status == GithubCommitFileStatus::Renamed {
        file.previous_filename
      } else {
        None
      };

      Rc::new(GithubCommitFileDiff {
        path: path.into(),
        old_path: old_path.map(Into::into),
        status,
        patch: file.patch.map(Into::into),
      })
    })
    .collect()
}

#[derive(Default)]
struct GithubCommitFileTreeNode {
  name: String,
  path: String,
  children: BTreeMap<String, GithubCommitFileTreeNode>,
  file: Option<()>,
}

impl GithubCommitFileTreeNode {
  fn new(name: String, path: String) -> Self {
    Self {
      name,
      path,
      children: BTreeMap::new(),
      file: None,
    }
  }

  fn is_folder(&self) -> bool {
    !self.children.is_empty()
  }
}

type CommitTreeBuildResult = (
  Vec<TreeItem>,
  HashMap<String, Rc<GithubCommitFileDiff>>,
  Option<usize>,
  Option<String>,
);

fn build_commit_tree_items(files: &[Rc<GithubCommitFileDiff>]) -> CommitTreeBuildResult {
  fn insert_node(
    map: &mut BTreeMap<String, GithubCommitFileTreeNode>,
    parts: &[&str],
    prefix: &str,
  ) {
    let Some((head, tail)) = parts.split_first() else {
      return;
    };

    let path = if prefix.is_empty() {
      head.to_string()
    } else {
      format!("{prefix}/{head}")
    };

    let node = map
      .entry(head.to_string())
      .or_insert_with(|| GithubCommitFileTreeNode::new(head.to_string(), path.clone()));

    if tail.is_empty() {
      node.file = Some(());
      return;
    }

    let node_path = node.path.clone();
    insert_node(&mut node.children, tail, &node_path);
  }

  let mut root = BTreeMap::new();
  let mut file_lookup = HashMap::new();

  for file in files {
    let path = file.path.as_ref();
    file_lookup.insert(path.to_string(), file.clone());
    let parts = path.split('/').collect::<Vec<_>>();
    insert_node(&mut root, &parts, "");
  }

  let mut order = Vec::new();
  let mut first_file_id = None;
  let mut nodes = root.into_values().collect::<Vec<_>>();
  nodes.sort_by(|a, b| {
    b.is_folder()
      .cmp(&a.is_folder())
      .then_with(|| a.name.cmp(&b.name))
  });

  let items = nodes
    .into_iter()
    .map(|node| build_commit_tree_item(node, &mut order, &mut first_file_id))
    .collect::<Vec<_>>();
  let selected_index = first_file_id
    .as_ref()
    .and_then(|id| order.iter().position(|candidate| candidate == id));

  (items, file_lookup, selected_index, first_file_id)
}

fn build_commit_tree_item(
  node: GithubCommitFileTreeNode,
  order: &mut Vec<String>,
  first_file_id: &mut Option<String>,
) -> TreeItem {
  let mut child_nodes = node.children.into_values().collect::<Vec<_>>();
  child_nodes.sort_by(|a, b| {
    b.is_folder()
      .cmp(&a.is_folder())
      .then_with(|| a.name.cmp(&b.name))
  });

  let mut item = TreeItem::new(node.path.clone(), node.name.clone());
  if !child_nodes.is_empty() {
    let children = child_nodes
      .into_iter()
      .map(|child| build_commit_tree_item(child, order, first_file_id))
      .collect::<Vec<_>>();
    item = item.children(children).expanded(true);
  }

  order.push(node.path.clone());
  if node.file.is_some() && first_file_id.is_none() {
    *first_file_id = Some(node.path);
  }

  item
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum GithubCommitBackTarget {
  Repo {
    owner: SharedString,
    repo: SharedString,
  },
}

#[derive(Clone, Default)]
pub struct GithubCommitDetailsPageHandle {
  page: Option<gpui::WeakEntity<GithubCommitDetailsPage>>,
}

impl gpui::Global for GithubCommitDetailsPageHandle {}

impl GithubCommitDetailsPageHandle {
  pub fn register(cx: &mut Context<GithubCommitDetailsPage>) {
    cx.set_global(Self {
      page: Some(cx.entity().downgrade()),
    });
  }

  pub fn show(owner: SharedString, repo: SharedString, sha: SharedString, cx: &mut App) {
    Self::show_with_back_target(
      owner.clone(),
      repo.clone(),
      sha,
      GithubCommitBackTarget::Repo { owner, repo },
      cx,
    );
  }

  pub fn show_with_repo_return(
    owner: SharedString,
    repo: SharedString,
    sha: SharedString,
    return_owner: SharedString,
    return_repo: SharedString,
    cx: &mut App,
  ) {
    Self::show_with_back_target(
      owner,
      repo,
      sha,
      GithubCommitBackTarget::Repo {
        owner: return_owner,
        repo: return_repo,
      },
      cx,
    );
  }

  fn show_with_back_target(
    owner: SharedString,
    repo: SharedString,
    sha: SharedString,
    back_target: GithubCommitBackTarget,
    cx: &mut App,
  ) {
    let Some(weak) = cx.global::<Self>().page.clone() else {
      return;
    };

    let owner_string = owner.to_string();
    let repo_string = repo.to_string();
    let sha_string = sha.to_string();
    let _ = weak.update(cx, |this, cx| {
      this.load_commit(owner_string, repo_string, sha_string, back_target, cx);
    });

    NavigationHistory::navigate(
      crate::navigation::build_commit_path(&owner, &repo, &sha),
      cx,
    );
  }

  pub fn refresh(cx: &mut App) {
    let Some(weak) = cx.global::<Self>().page.clone() else {
      return;
    };
    let _ = weak.update(cx, |this, cx| this.refresh_current_commit(cx));
  }

  pub fn is_refreshing(cx: &App) -> bool {
    let Some(weak) = cx
      .try_global::<Self>()
      .and_then(|handle| handle.page.clone())
    else {
      return false;
    };

    weak
      .read_with(cx, |this, _| this.commit_loading || this.file_loading)
      .unwrap_or(false)
  }
}

pub struct GithubCommitDetailsPage {
  focus_handle: FocusHandle,
  api: ApiClient,
  owner: SharedString,
  repo: SharedString,
  sha: SharedString,
  back_target: GithubCommitBackTarget,
  commit: Option<GithubCommitDetails>,
  commit_loading: bool,
  commit_error: Option<SharedString>,
  commit_task: Option<Task<()>>,
  tree_state: Entity<TreeState>,
  file_lookup: HashMap<String, Rc<GithubCommitFileDiff>>,
  selected_file: Option<Rc<GithubCommitFileDiff>>,
  selected_tree_id: Option<String>,
  file_contents: HashMap<String, GithubCommitFileContents>,
  file_content_tasks: HashMap<String, Task<()>>,
  file_loading: bool,
  file_error: Option<SharedString>,
  diff_editor: Entity<Editor>,
  diff_view: DiffViewMode,
  hide_whitespace: bool,
}

impl GithubCommitDetailsPage {
  fn build_detached_diff_editor(
    path: impl Into<PathBuf>,
    cx: &mut Context<Self>,
  ) -> Entity<Editor> {
    let editor_path = path.into();
    let load_root = PathBuf::from(".");
    let load_path = PathBuf::from(".reviu-github-commit-preview").join(&editor_path);
    let loaded = Editor::load_file_for_editor(&load_root, &load_path);
    let detached_root = PathBuf::from(".reviu-github-commit-editor-root");

    cx.new(move |cx| {
      let mut editor = Editor::new_with_loaded_file(detached_root, editor_path, loaded, cx);
      editor.is_read_only = true;
      editor
    })
  }

  pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
    GithubCommitDetailsPageHandle::register(cx);
    let tree_state = cx.new(|cx| TreeState::new(cx));
    let app_settings = AppSettings::get(cx);

    Self {
      focus_handle: cx.focus_handle(),
      api: WorkspaceApi::global(cx).api.clone(),
      owner: "".into(),
      repo: "".into(),
      sha: "".into(),
      back_target: GithubCommitBackTarget::Repo {
        owner: "".into(),
        repo: "".into(),
      },
      commit: None,
      commit_loading: false,
      commit_error: None,
      commit_task: None,
      tree_state,
      file_lookup: HashMap::new(),
      selected_file: None,
      selected_tree_id: None,
      file_contents: HashMap::new(),
      file_content_tasks: HashMap::new(),
      file_loading: false,
      file_error: None,
      diff_editor: Self::build_detached_diff_editor("__reviu_github_commit_placeholder__.txt", cx),
      diff_view: if app_settings.split_diff_view {
        DiffViewMode::Split
      } else {
        DiffViewMode::Inline
      },
      hide_whitespace: app_settings.hide_whitespace,
    }
  }

  fn refresh_current_commit(&mut self, cx: &mut Context<Self>) {
    if self.owner.is_empty() || self.repo.is_empty() || self.sha.is_empty() {
      return;
    }
    self.load_commit(
      self.owner.to_string(),
      self.repo.to_string(),
      self.sha.to_string(),
      self.back_target.clone(),
      cx,
    );
  }

  fn reset_file_state(&mut self, cx: &mut Context<Self>) {
    self.file_loading = false;
    self.file_error = None;
    self.file_lookup.clear();
    self.selected_file = None;
    self.selected_tree_id = None;
    self.file_contents.clear();
    self.file_content_tasks.clear();
    self.tree_state.update(cx, |state, cx| {
      state.set_items(Vec::new(), cx);
    });
    self.clear_diff_editor(cx);
  }

  fn load_commit(
    &mut self,
    owner: String,
    repo: String,
    sha: String,
    back_target: GithubCommitBackTarget,
    cx: &mut Context<Self>,
  ) {
    self.owner = owner.clone().into();
    self.repo = repo.clone().into();
    self.sha = sha.clone().into();
    self.back_target = back_target;
    self.commit = None;
    self.commit_loading = true;
    self.commit_error = None;
    self.reset_file_state(cx);

    let api = self.api.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || api.fetch_github_commit(&owner, &repo, &sha)).await;

      let _ = this.update(cx, |this, cx| {
        this.commit_loading = false;
        match result {
          Ok(commit) => {
            let files = commit_files_from_api(commit.files.clone());
            let (items, lookup, selected_index, selected_id) = build_commit_tree_items(&files);
            this.file_lookup = lookup;
            this.tree_state.update(cx, |state, cx| {
              state.set_items(items, cx);
              if let Some(ix) = selected_index {
                state.set_selected_index(Some(ix), cx);
              }
            });
            this.commit = Some(commit);
            this.commit_error = None;
            if let Some(selected_id) = selected_id {
              this.select_file_by_path(&selected_id, cx);
            }
          }
          Err(error) => {
            this.commit = None;
            this.commit_error = Some(error.to_string().into());
          }
        }
        cx.notify();
      });
    });

    self.commit_task = Some(task);
    cx.notify();
  }

  fn select_file_by_path(&mut self, path: &str, cx: &mut Context<Self>) {
    let Some(file) = self.file_lookup.get(path).cloned() else {
      return;
    };
    self.set_selected_file(Some(file), cx);
  }

  fn set_selected_file(
    &mut self,
    selected: Option<Rc<GithubCommitFileDiff>>,
    cx: &mut Context<Self>,
  ) {
    let current_id = self.selected_file.as_ref().map(|file| file.path.clone());
    let next_id = selected.as_ref().map(|file| file.path.clone());
    if current_id == next_id {
      return;
    }

    self.selected_file = selected.clone();
    self.selected_tree_id = selected.as_ref().map(|file| file.path.to_string());

    if let Some(file) = selected {
      self.ensure_diff_editor_for_path(file.path.as_ref(), cx);
      let key = file.path.to_string();
      if let Some(contents) = self.file_contents.get(&key).cloned() {
        self.apply_full_diff(&file, &contents, cx);
      } else {
        self.file_loading = true;
        self.file_error = None;
        self.clear_diff_editor(cx);
        self.maybe_fetch_file_contents(file, cx);
      }
    } else {
      self.file_loading = false;
      self.file_error = None;
      self.clear_diff_editor(cx);
    }

    cx.notify();
  }

  fn ensure_diff_editor_for_path(&mut self, path: &str, cx: &mut Context<Self>) {
    self.diff_editor = Self::build_detached_diff_editor(path, cx);
    self.sync_diff_view(cx);
  }

  fn clear_diff_editor(&mut self, cx: &mut Context<Self>) {
    self.diff_editor.update(cx, |editor, cx| {
      editor.document().update(cx, |doc, cx| {
        doc.replace_all("", cx);
      });
      editor.reset_after_replace();
      editor.set_diffs(None, cx);
      editor.is_read_only = true;
    });
  }

  fn maybe_fetch_file_contents(&mut self, file: Rc<GithubCommitFileDiff>, cx: &mut Context<Self>) {
    match file_preview_kind(Path::new(file.path.as_ref())) {
      Some(FilePreviewKind::RasterImage(_)) | Some(FilePreviewKind::UnsupportedBinary) => {
        self.file_loading = false;
        self.file_error = Some("Binary preview is not available for commit diffs".into());
        return;
      }
      _ => {}
    }

    let key = file.path.to_string();
    if self.file_contents.contains_key(&key) || self.file_content_tasks.contains_key(&key) {
      return;
    }

    let Some(commit) = self.commit.as_ref() else {
      return;
    };
    let owner = self.owner.to_string();
    let repo = self.repo.to_string();
    let head_sha = commit.sha.clone();
    let base_sha = commit.parent_sha.clone();
    let base_path = match file.status {
      GithubCommitFileStatus::Added => None,
      GithubCommitFileStatus::Renamed => file
        .old_path
        .as_ref()
        .map(|path| path.to_string())
        .or_else(|| Some(file.path.to_string())),
      _ => base_sha.as_ref().map(|_| file.path.to_string()),
    };
    let head_path = match file.status {
      GithubCommitFileStatus::Deleted => None,
      _ => Some(file.path.to_string()),
    };

    let api = self.api.clone();
    let key_for_task = key.clone();
    let task = cx.spawn(async move |this, cx| {
      let base_result = if let (Some(base_sha), Some(path)) = (base_sha.clone(), base_path.clone())
      {
        let api = api.clone();
        let owner = owner.clone();
        let repo = repo.clone();
        unblock(move || api.fetch_github_file_content(&owner, &repo, &path, &base_sha)).await
      } else {
        Ok(None)
      };

      let head_result = if let Some(path) = head_path.clone() {
        let api = api.clone();
        let owner = owner.clone();
        let repo = repo.clone();
        let head_sha = head_sha.clone();
        unblock(move || api.fetch_github_file_content(&owner, &repo, &path, &head_sha)).await
      } else {
        Ok(None)
      };

      let _ = this.update(cx, |this, cx| {
        this.file_content_tasks.remove(&key_for_task);
        let is_selected_file = this.selected_tree_id.as_deref() == Some(key_for_task.as_str());
        let (base, head) = match (base_result, head_result) {
          (Ok(base), Ok(head)) => (base, head),
          _ => {
            if is_selected_file {
              if let Some(file) = this.selected_file.clone()
                && file.path.as_ref() == key_for_task.as_str()
                && file.patch.is_some()
              {
                this.apply_patch_preview(&file, cx);
              } else {
                this.file_loading = false;
                this.file_error = Some("Failed to load file contents".into());
              }
            }
            cx.notify();
            return;
          }
        };

        let contents = GithubCommitFileContents { base, head };
        if is_selected_file {
          if let Some(file) = this.selected_file.clone() {
            this.apply_full_diff(&file, &contents, cx);
          }
        }
        this.file_contents.insert(key_for_task, contents);
        cx.notify();
      });
    });

    self.file_content_tasks.insert(key, task);
  }

  fn apply_full_diff(
    &mut self,
    file: &GithubCommitFileDiff,
    contents: &GithubCommitFileContents,
    cx: &mut Context<Self>,
  ) {
    self.file_loading = false;
    self.file_error = None;
    if contents.base.is_none() && contents.head.is_none() {
      if file.patch.is_some() {
        self.apply_patch_preview(file, cx);
      } else {
        self.clear_diff_editor(cx);
        self.file_error = Some("File contents unavailable".into());
      }
      return;
    }

    let head = contents.head.as_deref().unwrap_or("");
    let base = contents.base.as_deref();
    let diff = compute_buffer_diff(
      DiffKind::Uncommitted,
      base,
      head,
      Path::new(file.path.as_ref()),
      self.hide_whitespace,
    )
    .ok();
    let Some(diff) = diff else {
      self.file_error = Some("Unable to compute diff".into());
      self.file_loading = false;
      return;
    };

    let diff_set = Some(DiffSet {
      uncommitted: diff,
      unstaged: FileDiff {
        kind: DiffKind::Unstaged,
        hunks: Vec::new(),
      },
      staged: FileDiff {
        kind: DiffKind::Staged,
        hunks: Vec::new(),
      },
    });

    self.diff_editor.update(cx, |editor, cx| {
      editor.document().update(cx, |doc, cx| {
        doc.replace_all(head, cx);
      });
      editor.reset_after_replace();
      editor.reset_selection(cx);
      editor.set_diffs(diff_set, cx);
      editor.is_read_only = true;
    });
    self.sync_diff_view(cx);
  }

  fn apply_patch_preview(&mut self, file: &GithubCommitFileDiff, cx: &mut Context<Self>) {
    self.file_loading = false;
    self.file_error = None;
    let patch = file
      .patch
      .as_ref()
      .map(|patch| patch.as_ref())
      .unwrap_or("");
    self.diff_editor.update(cx, |editor, cx| {
      editor.document().update(cx, |doc, cx| {
        doc.replace_all(patch, cx);
      });
      editor.reset_after_replace();
      editor.reset_selection(cx);
      editor.set_diffs(None, cx);
      editor.is_read_only = true;
    });
  }

  fn sync_diff_view(&mut self, cx: &mut Context<Self>) {
    self.diff_editor.update(cx, |editor, cx| {
      editor.set_diff_view_mode(self.diff_view, cx)
    });
  }

  fn toggle_diff_view(&mut self, cx: &mut Context<Self>) {
    self.diff_view = match self.diff_view {
      DiffViewMode::Inline => DiffViewMode::Split,
      DiffViewMode::Split => DiffViewMode::Inline,
    };
    self.sync_diff_view(cx);
    cx.notify();
  }

  fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let repo_label = if self.owner.is_empty() || self.repo.is_empty() {
      "Repository".to_string()
    } else {
      github_shared::repo_label(self.owner.as_ref(), self.repo.as_ref())
    };

    let commit_loading = self.commit_loading && self.commit.is_none();
    let summary = if commit_loading {
      self.render_commit_summary_skeleton(cx)
    } else {
      self.render_commit_summary(cx)
    };

    let short_sha = github_shared::short_sha(self.sha.as_ref());
    let back_target = self.back_target.clone();
    let back_button = Button::new("github-commit-back")
      .icon(IconName::ArrowLeft)
      .ghost()
      .compact()
      .on_click(move |_, _, cx| match &back_target {
        GithubCommitBackTarget::Repo { owner, repo } => {
          crate::github_repo_page::GithubRepoPageHandle::show(owner.clone(), repo.clone(), cx);
        }
      });

    div()
      .flex()
      .flex_col()
      .border_b_1()
      .border_color(theme.border)
      .px_3()
      .py_2()
      .gap_1()
      .child(
        h_flex()
          .items_center()
          .justify_between()
          .child(
            h_flex()
              .items_center()
              .gap_2()
              .min_w_0()
              .child(back_button)
              .child(
                Label::new(repo_label)
                  .text_sm()
                  .secondary(short_sha)
                  .truncate(),
              ),
          )
          .child(
            h_flex()
              .items_center()
              .gap_2()
              .when_some(
                self
                  .commit
                  .as_ref()
                  .and_then(|commit| commit.associated_pull_request.clone()),
                |this, pull| {
                  let owner = self.owner.to_string();
                  let repo = self.repo.to_string();
                  let label = format!("PR #{}", pull.number);
                  this.child(
                    Button::new("github-commit-open-pr")
                      .icon(UiIconName::GitPullRequest)
                      .label(label)
                      .ghost()
                      .compact()
                      .small()
                      .on_click(move |_, _, cx| {
                        open_pr_target(
                          owner.clone(),
                          repo.clone(),
                          pull.number,
                          false,
                          None,
                          None,
                          cx,
                        );
                      }),
                  )
                },
              )
              .when_some(
                self.commit.as_ref().map(|commit| commit.html_url.clone()),
                |this, url| {
                  this.child(
                    Button::new("github-commit-open-external")
                      .icon(IconName::ExternalLink)
                      .label("Open on GitHub")
                      .ghost()
                      .compact()
                      .small()
                      .on_click(move |_, _, cx| {
                        cx.open_url(&url);
                      }),
                  )
                },
              ),
          ),
      )
      .child(summary)
  }

  fn render_commit_summary(&self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let Some(commit) = self.commit.as_ref() else {
      return div().into_any_element();
    };
    let subject = github_shared::commit_subject(&commit.message);
    let user = commit.author.as_ref().or(commit.committer.as_ref());
    let committed_at = commit
      .committed_at
      .as_deref()
      .or(commit.authored_at.as_deref())
      .unwrap_or("");

    h_flex()
      .items_center()
      .justify_between()
      .gap_4()
      .child(
        h_flex()
          .items_center()
          .gap_2()
          .min_w_0()
          .when_some(user, |this, user| {
            this.child(
              Avatar::new()
                .name(user.login.clone())
                .when_some(user.avatar_url.clone(), |this, url| this.src(url))
                .small(),
            )
          })
          .child(
            div()
              .text_sm()
              .font_medium()
              .text_color(theme.foreground)
              .truncate()
              .child(subject),
          ),
      )
      .child(
        h_flex()
          .items_center()
          .gap_3()
          .text_sm()
          .text_color(theme.muted_foreground)
          .flex_shrink_0()
          .when_some(commit.stats.as_ref(), |this, stats| {
            this
              .child(
                div()
                  .text_color(theme.status_green())
                  .child(format!("+{}", stats.additions)),
              )
              .child(
                div()
                  .text_color(theme.status_red())
                  .child(format!("-{}", stats.deletions)),
              )
          })
          .when(!committed_at.is_empty(), |this| {
            this.child(format_relative_time(committed_at))
          }),
      )
      .into_any_element()
  }

  fn render_commit_summary_skeleton(&self, cx: &mut Context<Self>) -> AnyElement {
    let theme: gpui_component::Theme = cx.theme().clone();

    h_flex()
      .items_center()
      .justify_between()
      .gap_4()
      .child(
        h_flex()
          .items_center()
          .gap_2()
          .min_w_0()
          .child(Skeleton::new().size(px(24.0)).rounded_full())
          .child(
            Skeleton::new()
              .w(px(280.0))
              .h(px(14.0))
              .rounded(theme.radius),
          ),
      )
      .child(
        h_flex()
          .items_center()
          .gap_3()
          .flex_shrink_0()
          .child(
            Skeleton::new()
              .w(px(34.0))
              .h(px(14.0))
              .rounded(theme.radius),
          )
          .child(
            Skeleton::new()
              .w(px(34.0))
              .h(px(14.0))
              .rounded(theme.radius),
          )
          .child(
            Skeleton::new()
              .w(px(92.0))
              .h(px(14.0))
              .rounded(theme.radius)
              .secondary(),
          ),
      )
      .into_any_element()
  }

  fn render_files_sidebar_skeleton(theme: &gpui_component::Theme) -> AnyElement {
    v_flex()
      .flex_1()
      .p_2()
      .gap_1()
      .children((0..12).map(|ix| {
        let width = match ix % 4 {
          0 => 190.0,
          1 => 145.0,
          2 => 220.0,
          _ => 165.0,
        };

        h_flex()
          .h(px(32.0))
          .items_center()
          .gap_2()
          .px_2()
          .child(
            Skeleton::new()
              .w(px(15.0))
              .h(px(12.0))
              .rounded(theme.radius)
              .secondary(),
          )
          .child(Skeleton::new().size(px(14.0)).rounded(theme.radius))
          .child(
            Skeleton::new()
              .w(px(width))
              .h(px(12.0))
              .rounded(theme.radius),
          )
      }))
      .into_any_element()
  }

  fn render_files_sidebar(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let theme = cx.theme().clone();
    let count = self.file_lookup.len();
    let count_badge = if self.commit_loading {
      Skeleton::new()
        .w(px(22.0))
        .h(px(18.0))
        .rounded_full()
        .secondary()
        .into_any_element()
    } else {
      Tag::secondary()
        .small()
        .rounded_full()
        .child(count.to_string())
        .into_any_element()
    };

    if let Some(selected_id) = self
      .tree_state
      .read(cx)
      .selected_entry()
      .map(|entry| entry.item().id.to_string())
      && Some(selected_id.as_str()) != self.selected_tree_id.as_deref()
      && let Some(file) = self.file_lookup.get(&selected_id).cloned()
    {
      self.selected_tree_id = Some(selected_id.clone());
      cx.on_next_frame(window, move |this, _, cx| {
        this.set_selected_file(Some(file), cx);
      });
    }

    let header = h_flex()
      .pl_3()
      .pr_2()
      .items_center()
      .justify_between()
      .h(px(COMMIT_HEADER_HEIGHT))
      .border_b_1()
      .border_color(theme.border)
      .child(
        h_flex()
          .items_center()
          .gap_2()
          .child(div().text_sm().text_color(theme.foreground).child("Files"))
          .child(count_badge),
      );

    let list = if self.commit_loading {
      Self::render_files_sidebar_skeleton(&theme)
    } else if let Some(error) = self.commit_error.clone() {
      v_flex()
        .flex_1()
        .h_full()
        .items_center()
        .justify_center()
        .px_4()
        .text_sm()
        .text_color(theme.status_red())
        .child(error)
        .into_any_element()
    } else if count == 0 {
      v_flex()
        .flex_1()
        .h_full()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.muted_foreground)
        .child("No files changed")
        .into_any_element()
    } else {
      let view = cx.entity();
      tree(&self.tree_state, move |ix, entry, selected, _window, cx| {
        view.update(cx, |this, cx| {
          let theme = cx.theme().clone();
          let item = entry.item();
          let is_folder = entry.is_folder();
          let status = if is_folder {
            None
          } else {
            this
              .file_lookup
              .get(item.id.as_ref())
              .map(|file| file.status)
          };
          let status_letter = status.map(commit_file_status_letter).unwrap_or("");
          let status_color = status
            .map(|status| commit_file_status_color(status, &theme))
            .unwrap_or(theme.muted_foreground);
          let icon = if is_folder {
            if entry.is_expanded() {
              Icon::new(IconName::FolderOpen)
            } else {
              Icon::new(IconName::Folder)
            }
            .size_3()
            .text_color(theme.muted_foreground)
            .into_any_element()
          } else {
            file_icon_path_for_name_with_theme(item.label.as_ref(), &theme)
              .map(|path| img(path).size(px(FILE_ICON_SIZE_PX)).into_any_element())
              .unwrap_or_else(|| {
                Icon::new(IconName::File)
                  .size_3()
                  .text_color(theme.muted_foreground)
                  .into_any_element()
              })
          };

          let indent = px(12.) + px(15.) * entry.depth();
          let mut row = selectable_list_item(ix, selected, SelectableRowStyle::Inset, &theme)
            .w_full()
            .px_2()
            .pl(indent)
            .child(
              h_flex()
                .items_center()
                .gap_2()
                .when(!is_folder, |this| {
                  this.child(
                    div()
                      .w(px(15.))
                      .text_xs()
                      .text_color(status_color)
                      .child(status_letter),
                  )
                })
                .child(icon)
                .child(
                  div()
                    .flex_1()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(item.label.clone()),
                ),
            );

          if !is_folder {
            let id = item.id.clone();
            row = row.on_click(cx.listener(move |this, _, _, cx| {
              this.select_file_by_path(id.as_ref(), cx);
            }));
          }

          row
        })
      })
      .flex_1()
      .w_full()
      .into_any_element()
    };

    v_flex()
      .bg(theme.sidebar)
      .size_full()
      .child(header)
      .child(div().p_1().flex_1().min_h_0().child(list))
  }

  fn render_diff_header(&self, file: &GithubCommitFileDiff, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let path = Path::new(file.path.as_ref());
    let file_name = path
      .file_name()
      .and_then(|name| name.to_str())
      .unwrap_or(file.path.as_ref())
      .to_string();
    let dir_path = path
      .parent()
      .and_then(|parent| parent.to_str())
      .unwrap_or("")
      .to_string();
    let icon = file_icon_path_for_name_with_theme(&file_name, &theme)
      .map(|path| img(path).size(px(FILE_ICON_SIZE_PX)).into_any_element())
      .unwrap_or_else(|| {
        Icon::new(IconName::File)
          .size_3()
          .text_color(theme.muted_foreground)
          .into_any_element()
      });
    let status_letter = commit_file_status_letter(file.status);
    let status_color = commit_file_status_color(file.status, &theme);
    let (toggle_label, toggle_icon) = match self.diff_view {
      DiffViewMode::Inline => ("Split", IconName::PanelLeft),
      DiffViewMode::Split => ("Inline", IconName::PanelLeftClose),
    };
    let view = cx.entity();

    h_flex()
      .h(px(COMMIT_HEADER_HEIGHT))
      .px_3()
      .items_center()
      .justify_between()
      .border_b_1()
      .border_color(theme.border)
      .child(
        h_flex()
          .items_center()
          .gap_2()
          .min_w_0()
          .child(
            div()
              .text_xs()
              .font_medium()
              .text_color(status_color)
              .child(status_letter),
          )
          .child(icon)
          .child({
            let mut label = Label::new(file_name);
            if !dir_path.is_empty() {
              label = label.secondary(format!("- {dir_path}"));
            }
            label.truncate()
          }),
      )
      .child(
        Button::new("github-commit-toggle-diff-view")
          .label(toggle_label)
          .icon(toggle_icon)
          .xsmall()
          .ghost()
          .on_click(move |_, _, cx| {
            view.update(cx, |this, cx| {
              this.toggle_diff_view(cx);
            });
          }),
      )
      .into_any_element()
  }

  fn render_diff_header_skeleton(&self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();

    h_flex()
      .h(px(COMMIT_HEADER_HEIGHT))
      .px_3()
      .items_center()
      .justify_between()
      .border_b_1()
      .border_color(theme.border)
      .child(
        h_flex()
          .items_center()
          .gap_2()
          .min_w_0()
          .child(
            Skeleton::new()
              .w(px(15.0))
              .h(px(12.0))
              .rounded(theme.radius)
              .secondary(),
          )
          .child(Skeleton::new().size(px(14.0)).rounded(theme.radius))
          .child(
            Skeleton::new()
              .w(px(260.0))
              .h(px(14.0))
              .rounded(theme.radius),
          ),
      )
      .child(
        Skeleton::new()
          .w(px(72.0))
          .h(px(24.0))
          .rounded(theme.radius),
      )
      .into_any_element()
  }

  fn render_diff_content_skeleton(cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();

    v_flex()
      .flex_1()
      .min_h_0()
      .p_4()
      .gap_2()
      .children((0..18).map(|ix| {
        let width = match ix % 6 {
          0 => 72.0,
          1 => 540.0,
          2 => 420.0,
          3 => 610.0,
          4 => 280.0,
          _ => 480.0,
        };

        h_flex()
          .h(px(20.0))
          .items_center()
          .gap_3()
          .child(
            Skeleton::new()
              .w(px(34.0))
              .h(px(12.0))
              .rounded(theme.radius)
              .secondary(),
          )
          .child(
            Skeleton::new()
              .w_full()
              .max_w(px(width))
              .h(px(12.0))
              .rounded(theme.radius),
          )
      }))
      .into_any_element()
  }

  fn render_diff_content(&self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    if self.file_loading {
      return Self::render_diff_content_skeleton(cx);
    }

    if let Some(error) = self.file_error.as_ref() {
      return v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .px_6()
        .text_sm()
        .text_color(theme.status_red())
        .child(error.clone())
        .into_any_element();
    }

    div()
      .flex_1()
      .min_h_0()
      .child(self.diff_editor.clone())
      .into_any_element()
  }

  fn render_commit_body(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let theme = cx.theme().clone();
    if self.commit_error.is_some() && self.commit.is_none() {
      return v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .px_6()
        .text_sm()
        .text_color(theme.status_red())
        .child(self.commit_error.clone().unwrap_or_default())
        .into_any_element();
    }

    let selected_file = self.selected_file.clone();
    let commit_loading = self.commit_loading && self.commit.is_none();
    let mut editor_panel = v_flex()
      .size_full()
      .overflow_hidden()
      .min_w_0()
      .bg(theme.background);

    if commit_loading {
      editor_panel = editor_panel
        .child(self.render_diff_header_skeleton(cx))
        .child(Self::render_diff_content_skeleton(cx));
    } else {
      if let Some(file) = selected_file {
        editor_panel = editor_panel.child(self.render_diff_header(&file, cx));
      }
      editor_panel = editor_panel.child(self.render_diff_content(cx));
    }

    h_resizable("github-commit-details-layout")
      .child(
        resizable_panel()
          .size(px(COMMIT_FILE_SIDEBAR_WIDTH))
          .size_range(px(COMMIT_FILE_SIDEBAR_MIN_WIDTH)..px(COMMIT_FILE_SIDEBAR_MAX_WIDTH))
          .child(self.render_files_sidebar(window, cx)),
      )
      .child(resizable_panel().child(editor_panel))
      .into_any_element()
  }

  fn show_command_palette_action(
    &mut self,
    _: &ShowCommandPalette,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.open_command_palette(window, cx);
  }

  fn open_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let include_github = AuthStateStore::has_github_access(cx);
    let commands = CommandPaletteCommand::default_global_commands(
      CommandPalettePage::GithubRepo,
      include_github,
    );

    let view = cx.entity();
    let handler: CommandPaletteHandler = Arc::new(move |action, window, cx| {
      view.update(cx, |view, cx| {
        view.handle_command_palette_action(action, window, cx)
      })
    });

    let config = CommandPaletteConfig::new(Vec::new(), commands, handler);
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

  fn handle_command_palette_action(
    &mut self,
    action: CommandPaletteAction,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    match action {
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
        open_pr_target(
          owner,
          repo,
          number,
          open_changes_tab,
          review_comment_id,
          None,
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
        open_repo_target(owner, repo, tab, issue_number, issue_comment_id, cx);
        Ok(())
      }
      CommandPaletteAction::OpenGithubCommitDetails { owner, repo, sha } => {
        self.load_commit(
          owner.clone(),
          repo.clone(),
          sha.clone(),
          GithubCommitBackTarget::Repo {
            owner: owner.clone().into(),
            repo: repo.clone().into(),
          },
          cx,
        );
        NavigationHistory::navigate(
          crate::navigation::build_commit_path(&owner, &repo, &sha),
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
        crate::feedback_dialog::open_feedback_dialog(window, cx);
        Ok(())
      }
      _ => Err("Command not available.".into()),
    }
  }
}

impl Focusable for GithubCommitDetailsPage {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

impl Render for GithubCommitDetailsPage {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    div()
      .size_full()
      .flex()
      .flex_col()
      .track_focus(&self.focus_handle(cx))
      .on_action(cx.listener(Self::show_command_palette_action))
      .child(self.render_header(cx))
      .child(
        div()
          .flex_1()
          .min_h_0()
          .w_full()
          .child(self.render_commit_body(window, cx)),
      )
  }
}

#[cfg(test)]
mod tests {
  use super::{
    GithubCommitFileDiff, GithubCommitFileStatus, build_commit_tree_items, commit_file_status,
    commit_file_status_letter,
  };
  use gpui::SharedString;
  use std::rc::Rc;

  fn make_file(path: &str, status: GithubCommitFileStatus) -> Rc<GithubCommitFileDiff> {
    Rc::new(GithubCommitFileDiff {
      path: SharedString::from(path.to_string()),
      old_path: None,
      status,
      patch: None,
    })
  }

  #[test]
  fn commit_file_status_maps_github_statuses() {
    assert_eq!(commit_file_status("added"), GithubCommitFileStatus::Added);
    assert_eq!(
      commit_file_status("removed"),
      GithubCommitFileStatus::Deleted
    );
    assert_eq!(
      commit_file_status("deleted"),
      GithubCommitFileStatus::Deleted
    );
    assert_eq!(
      commit_file_status("renamed"),
      GithubCommitFileStatus::Renamed
    );
    assert_eq!(
      commit_file_status("modified"),
      GithubCommitFileStatus::Modified
    );
    assert_eq!(
      commit_file_status("changed"),
      GithubCommitFileStatus::Modified
    );
  }

  #[test]
  fn commit_file_status_letter_matches_status() {
    assert_eq!(
      commit_file_status_letter(GithubCommitFileStatus::Added),
      "A"
    );
    assert_eq!(
      commit_file_status_letter(GithubCommitFileStatus::Modified),
      "M"
    );
    assert_eq!(
      commit_file_status_letter(GithubCommitFileStatus::Deleted),
      "D"
    );
    assert_eq!(
      commit_file_status_letter(GithubCommitFileStatus::Renamed),
      "R"
    );
  }

  #[test]
  fn build_commit_tree_items_prefers_folders_and_selects_first_file() {
    let files = vec![
      make_file("README.md", GithubCommitFileStatus::Modified),
      make_file("src/lib.rs", GithubCommitFileStatus::Added),
    ];

    let (items, lookup, selected_index, selected_id) = build_commit_tree_items(&files);

    assert_eq!(items.len(), 2);
    assert_eq!(items[0].label.as_ref(), "src");
    assert!(items[0].is_expanded());
    assert!(lookup.contains_key("README.md"));
    assert!(lookup.contains_key("src/lib.rs"));
    assert_eq!(selected_index, Some(0));
    assert_eq!(selected_id.as_deref(), Some("src/lib.rs"));
  }
}
