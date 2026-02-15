use std::{
  collections::{BTreeMap, HashMap},
  path::{Path, PathBuf},
  rc::Rc,
  sync::Arc,
};

use editor::{CloseFind, DiffViewMode, Editor, Find, ReviewComment, ReviewCommentSide};
use gfm_markdown_viewer::{MarkdownRenderOptions, MarkdownRenderState, render_markdown};
use git::{DiffKind, DiffSet, FileDiff, compute_buffer_diff};
use gpui::{
  App, Context, Entity, FocusHandle, Focusable, ParentElement, Render, RenderImage, SharedString,
  Styled, Task, Window, div, img, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Disableable, Icon, IconName, Selectable, Sizable as _, StyledExt,
  avatar::Avatar,
  button::{Button, ButtonVariants as _},
  clipboard::Clipboard,
  h_flex,
  label::Label,
  list::ListItem,
  scroll::ScrollableElement,
  skeleton::Skeleton,
  spinner::Spinner,
  tab::{Tab, TabBar},
  tag::Tag,
  text::TextView,
  tree::{TreeItem, TreeState, tree},
  v_flex,
};
use smol::unblock;

use ui::{
  CommandPalette, CommandPaletteAction, CommandPaletteCommand, CommandPaletteConfig,
  CommandPaletteHandler, CommandPalettePage, FILE_ICON_SIZE_PX, SearchFileEntry, SearchFileHandler,
  SearchFilePalette, SearchFilePaletteConfig, StatusThemeExt, UiIconName, UserMenuConfig,
  UserMenuPage, UserMenuState, UserMenuUser, WindowExt, file_icon_path_for_name_with_theme,
  h_resizable, resizable_panel, user_menu,
};

use crate::{
  AuthCallbackTarget, ShowCommandPalette, ShowFileSearch,
  api::{
    ApiClient, GithubPullRequestDetails, GithubPullRequestFile, GithubPullRequestReviewComment,
  },
  auth_state::{AuthState, AuthStateStore},
  github_page::GithubPageHandle,
  workspace::{WorkspaceApi, WorkspacePage, WorkspaceRoute},
};

const SIDEBAR_DEFAULT_WIDTH: f32 = 350.0;
const SIDEBAR_MIN_WIDTH: f32 = 250.0;
const SIDEBAR_MAX_WIDTH: f32 = 1500.0;
const DIFF_HEADER_HEIGHT: f32 = 40.0;

fn format_datetime(value: &str) -> SharedString {
  let Some((date, time)) = value.split_once('T') else {
    return value.to_string().into();
  };

  let _ = time;
  date.to_string().into()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GithubPrFileStatus {
  Added,
  Modified,
  Deleted,
  Renamed,
}

fn status_letter(status: GithubPrFileStatus) -> &'static str {
  match status {
    GithubPrFileStatus::Added => "A",
    GithubPrFileStatus::Modified => "M",
    GithubPrFileStatus::Deleted => "D",
    GithubPrFileStatus::Renamed => "R",
  }
}

fn status_color(status: GithubPrFileStatus, theme: &gpui_component::Theme) -> gpui::Hsla {
  match status {
    GithubPrFileStatus::Modified => theme.status_orange(),
    GithubPrFileStatus::Added => theme.status_green(),
    GithubPrFileStatus::Deleted => theme.status_red(),
    GithubPrFileStatus::Renamed => theme.info,
  }
}

#[derive(Clone, Debug)]
struct GithubPrFileDiff {
  path: SharedString,
  old_path: Option<SharedString>,
  status: GithubPrFileStatus,
}

fn map_file_status(status: &str) -> GithubPrFileStatus {
  match status.trim().to_ascii_lowercase().as_str() {
    "added" => GithubPrFileStatus::Added,
    "removed" | "deleted" => GithubPrFileStatus::Deleted,
    "renamed" => GithubPrFileStatus::Renamed,
    _ => GithubPrFileStatus::Modified,
  }
}

fn files_from_api(files: Vec<GithubPullRequestFile>) -> Vec<Rc<GithubPrFileDiff>> {
  files
    .into_iter()
    .map(|file| {
      let status = map_file_status(file.status.as_str());
      let path = if file.filename.is_empty() {
        "unknown".to_string()
      } else {
        file.filename
      };
      let old_path = if status == GithubPrFileStatus::Renamed {
        file.previous_filename
      } else {
        None
      };

      Rc::new(GithubPrFileDiff {
        path: path.into(),
        old_path: old_path.map(Into::into),
        status,
      })
    })
    .collect()
}

#[derive(Clone, Debug, Default)]
struct GithubPrFileContents {
  base: Option<String>,
  head: Option<String>,
}

#[derive(Default)]
struct FileTreeNode {
  name: String,
  path: String,
  children: BTreeMap<String, FileTreeNode>,
  file: Option<Rc<GithubPrFileDiff>>,
}

impl FileTreeNode {
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

fn build_tree_items(
  files: &[Rc<GithubPrFileDiff>],
) -> (
  Vec<TreeItem>,
  HashMap<String, Rc<GithubPrFileDiff>>,
  Option<usize>,
  Option<String>,
) {
  fn insert_node(
    map: &mut BTreeMap<String, FileTreeNode>,
    parts: &[&str],
    prefix: &str,
    file: Rc<GithubPrFileDiff>,
  ) {
    let Some((head, tail)) = parts.split_first() else {
      return;
    };

    let path = if prefix.is_empty() {
      head.to_string()
    } else {
      format!("{}/{}", prefix, head)
    };

    let node = map
      .entry(head.to_string())
      .or_insert_with(|| FileTreeNode::new(head.to_string(), path.clone()));

    if tail.is_empty() {
      node.file = Some(file);
      return;
    }

    let node_path = node.path.clone();
    insert_node(&mut node.children, tail, &node_path, file);
  }

  let mut root: BTreeMap<String, FileTreeNode> = BTreeMap::new();
  let mut file_lookup: HashMap<String, Rc<GithubPrFileDiff>> = HashMap::new();

  for file in files {
    let path = file.path.as_ref();
    file_lookup.insert(path.to_string(), file.clone());
    let parts: Vec<&str> = path.split('/').collect();
    insert_node(&mut root, &parts, "", file.clone());
  }

  let mut order = Vec::new();
  let mut first_file_id: Option<String> = None;

  let mut root_nodes: Vec<FileTreeNode> = root.into_values().collect();
  root_nodes.sort_by(|a, b| {
    b.is_folder()
      .cmp(&a.is_folder())
      .then_with(|| a.name.cmp(&b.name))
  });

  let items = root_nodes
    .into_iter()
    .map(|node| build_tree_item(node, &mut order, &mut first_file_id))
    .collect::<Vec<_>>();

  let selected_index = first_file_id
    .as_ref()
    .and_then(|id| order.iter().position(|candidate| candidate == id));

  (items, file_lookup, selected_index, first_file_id)
}

fn build_tree_item(
  node: FileTreeNode,
  order: &mut Vec<String>,
  first_file_id: &mut Option<String>,
) -> TreeItem {
  let mut child_nodes: Vec<FileTreeNode> = node.children.into_values().collect();
  child_nodes.sort_by(|a, b| {
    b.is_folder()
      .cmp(&a.is_folder())
      .then_with(|| a.name.cmp(&b.name))
  });

  let mut item = TreeItem::new(node.path.clone(), node.name.clone());
  if !child_nodes.is_empty() {
    let children = child_nodes
      .into_iter()
      .map(|child| build_tree_item(child, order, first_file_id))
      .collect::<Vec<_>>();
    item = item.children(children).expanded(true);
  }

  order.push(node.path.clone());
  if node.file.is_some() && first_file_id.is_none() {
    *first_file_id = Some(node.path.clone());
  }

  item
}

pub struct GithubPrDetailsPage {
  focus_handle: FocusHandle,
  api: ApiClient,
  details_task: Option<Task<()>>,
  files_task: Option<Task<()>>,
  files_loading: bool,
  files_error: Option<SharedString>,
  review_comments_task: Option<Task<()>>,
  review_comments_loading: bool,
  review_comments_error: Option<SharedString>,
  review_comments: Vec<GithubPullRequestReviewComment>,
  file_loading: bool,
  file_error: Option<SharedString>,
  tree_state: Entity<TreeState>,
  file_lookup: HashMap<String, Rc<GithubPrFileDiff>>,
  file_contents: HashMap<String, GithubPrFileContents>,
  file_content_tasks: HashMap<String, Task<()>>,
  selected_file: Option<Rc<GithubPrFileDiff>>,
  selected_tree_id: Option<String>,
  diff_editor: Entity<Editor>,
  diff_view: DiffViewMode,
  show_markdown_preview: bool,
  description_markdown_state: MarkdownRenderState,
  svg_preview: Option<Result<Arc<RenderImage>, SharedString>>,
  svg_preview_source: Option<SharedString>,
  svg_preview_task: Option<Task<()>>,
  active_tab_ix: usize,
  pull_request: Option<GithubPullRequestDetails>,
  error: Option<SharedString>,
}

#[derive(Clone, Default)]
pub struct GithubPrDetailsPageHandle {
  page: Option<gpui::WeakEntity<GithubPrDetailsPage>>,
}

impl gpui::Global for GithubPrDetailsPageHandle {}

impl GithubPrDetailsPageHandle {
  pub fn register(cx: &mut Context<GithubPrDetailsPage>) {
    cx.set_global(Self {
      page: Some(cx.entity().downgrade()),
    });
  }

  pub fn show(owner: SharedString, repo: SharedString, number: u64, cx: &mut App) {
    let Some(weak) = cx.global::<Self>().page.clone() else {
      return;
    };

    let owner_string = owner.to_string();
    let repo_string = repo.to_string();
    let _ = weak.update(cx, |this, cx| {
      this.load_pull_request(owner_string, repo_string, number, cx);
    });

    WorkspaceRoute::global_mut(cx).page = WorkspacePage::GithubPrDetails;
    cx.refresh_windows();
  }
}

impl GithubPrDetailsPage {
  pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
    GithubPrDetailsPageHandle::register(cx);

    let tree_state = cx.new(|cx| TreeState::new(cx));

    let view = Self {
      focus_handle: cx.focus_handle(),
      api: WorkspaceApi::global(cx).api.clone(),
      details_task: None,
      files_task: None,
      files_loading: false,
      files_error: None,
      review_comments_task: None,
      review_comments_loading: false,
      review_comments_error: None,
      review_comments: Vec::new(),
      file_loading: false,
      file_error: None,
      tree_state,
      file_lookup: HashMap::new(),
      file_contents: HashMap::new(),
      file_content_tasks: HashMap::new(),
      selected_file: None,
      selected_tree_id: None,
      diff_editor: cx.new(|cx| {
        let mut editor = Editor::new_with_paths(PathBuf::from("."), PathBuf::from("pr.diff"), cx);
        editor.is_read_only = true;
        editor
      }),
      diff_view: DiffViewMode::Inline,
      show_markdown_preview: false,
      description_markdown_state: MarkdownRenderState::new(),
      svg_preview: None,
      svg_preview_source: None,
      svg_preview_task: None,
      active_tab_ix: 0,
      pull_request: None,
      error: None,
    };

    view
  }

  fn set_active_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
    self.active_tab_ix = ix;
    if ix == 1 {
      self.sync_tree_selection(cx);
      window.focus(&self.focus_handle, cx);
    }
    cx.notify();
  }

  fn set_selected_file(&mut self, selected: Option<Rc<GithubPrFileDiff>>, cx: &mut Context<Self>) {
    let current_id = self.selected_file.as_ref().map(|file| file.path.clone());
    let next_id = selected.as_ref().map(|file| file.path.clone());
    if current_id == next_id {
      return;
    }

    self.selected_file = selected.clone();
    self.selected_tree_id = selected.as_ref().map(|file| file.path.to_string());
    if !self.selected_file_is_markdown() && !self.selected_file_is_svg() {
      self.show_markdown_preview = false;
    }
    self.svg_preview = None;
    self.svg_preview_source = None;

    if let Some(file) = selected {
      self.ensure_diff_editor_for_path(file.path.as_ref(), cx);
      self.sync_diff_view(cx);
      self.sync_tree_selection(cx);
      let key = file.path.to_string();
      let cached = self.file_contents.contains_key(&key);
      let in_flight = self.file_content_tasks.contains_key(&key);
      let _ = (cached, in_flight);
      if let Some(contents) = self.file_contents.get(&key).cloned() {
        if contents.base.is_none() && contents.head.is_none() {
          self.file_loading = false;
          self.file_error = Some("File contents unavailable".into());
          self.clear_diff_editor(cx);
        } else {
          self.file_loading = false;
          self.file_error = None;
          self.apply_full_diff(&file, &contents, cx);
        }
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

    self.sync_review_comments(cx);
    cx.notify();
  }

  fn ensure_diff_editor_for_path(&mut self, path: &str, cx: &mut Context<Self>) {
    let desired_path = PathBuf::from(path);
    let mut current_path = None;
    self.diff_editor.update(cx, |editor, _| {
      current_path = Some(editor.workdir_path.clone());
    });
    if current_path.as_ref() == Some(&desired_path) {
      return;
    }

    let repo_root = PathBuf::from(".");
    let desired_path_for_editor = desired_path.clone();
    self.diff_editor = cx.new(|cx| {
      let mut editor = Editor::new_with_paths(repo_root, desired_path_for_editor, cx);
      editor.is_read_only = true;
      editor
    });
  }

  fn clear_diff_editor(&mut self, cx: &mut Context<Self>) {
    self.diff_editor.update(cx, |editor, cx| {
      editor.document().update(cx, |doc, cx| {
        doc.replace_all("", cx);
      });
      editor.reset_after_replace();
      editor.reset_selection(cx);
      editor.set_diffs(None, cx);
      editor.is_read_only = true;
    });
  }

  fn split_disabled_for_file(&self, file: &GithubPrFileDiff) -> bool {
    matches!(
      file.status,
      GithubPrFileStatus::Added | GithubPrFileStatus::Deleted
    )
  }

  fn split_disabled_for_selected_file(&self) -> bool {
    self
      .selected_file
      .as_ref()
      .is_some_and(|file| self.split_disabled_for_file(file))
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

  fn is_svg_path(path: &Path) -> bool {
    matches!(
      path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref(),
      Some("svg")
    )
  }

  fn selected_file_is_markdown(&self) -> bool {
    self
      .selected_file
      .as_ref()
      .map(|file| Self::is_markdown_path(Path::new(file.path.as_ref())))
      .unwrap_or(false)
  }

  fn selected_file_is_svg(&self) -> bool {
    self
      .selected_file
      .as_ref()
      .map(|file| Self::is_svg_path(Path::new(file.path.as_ref())))
      .unwrap_or(false)
  }

  fn review_comments_for_selected_file(&self) -> Vec<ReviewComment> {
    let Some(file) = self.selected_file.as_ref() else {
      return Vec::new();
    };

    let comments_for_file: Vec<&GithubPullRequestReviewComment> = self
      .review_comments
      .iter()
      .filter(|comment| comment.path == file.path)
      .collect();
    let comments_by_id: HashMap<u64, &GithubPullRequestReviewComment> = comments_for_file
      .iter()
      .map(|comment| (comment.id, *comment))
      .collect();

    self
      .review_comments
      .iter()
      .filter(|comment| comment.path == file.path)
      .filter_map(|comment| {
        let mut resolved_line = comment.line.or(comment.start_line);
        let mut resolved_side = comment.side.as_deref().or(comment.start_side.as_deref());
        let mut current = Some(comment);
        for _ in 0..32 {
          if resolved_line.is_some() && resolved_side.is_some() {
            break;
          }

          let Some(parent_id) = current.and_then(|value| value.in_reply_to_id) else {
            break;
          };

          current = comments_by_id.get(&parent_id).copied();
          let Some(parent) = current else {
            break;
          };

          if resolved_line.is_none() {
            resolved_line = parent.line.or(parent.start_line);
          }
          if resolved_side.is_none() {
            resolved_side = parent.side.as_deref().or(parent.start_side.as_deref());
          }
        }

        let line = resolved_line.and_then(|value| {
          if value > 0 {
            Some(value as usize)
          } else {
            None
          }
        })?;
        let side = match resolved_side {
          Some("LEFT") => ReviewCommentSide::Left,
          _ => ReviewCommentSide::Right,
        };

        let side_label = resolved_side.unwrap_or("");
        let line_label = {
          let line_label = if let Some(start) = comment.start_line
            && let Some(end) = comment.line
            && start != end
          {
            Some(
              format!("L{}-{} {}", start, end, side_label)
                .trim()
                .to_string(),
            )
          } else {
            comment
              .line
              .or(comment.start_line)
              .or(resolved_line)
              .map(|value| format!("L{} {}", value, side_label).trim().to_string())
          };
          line_label.map(|label| Arc::from(label.as_str()))
        };

        Some(ReviewComment {
          id: comment.id,
          in_reply_to_id: comment.in_reply_to_id,
          line: line.saturating_sub(1),
          side,
          author: Arc::from(comment.user.login.as_str()),
          avatar_url: comment
            .user
            .avatar_url
            .as_deref()
            .map(|value| Arc::from(value)),
          line_label,
          body: Arc::from(comment.body.as_str()),
          created_at: Arc::from(format_datetime(&comment.created_at).to_string()),
        })
      })
      .collect()
  }

  fn sync_review_comments(&mut self, cx: &mut Context<Self>) {
    let comments = self.review_comments_for_selected_file();
    let pr_number = self.pull_request.as_ref().map(|pr| pr.number);
    self.diff_editor.update(cx, |editor, cx| {
      editor.set_review_comment_pr_number(pr_number, cx)
    });
    self
      .diff_editor
      .update(cx, |editor, cx| editor.set_review_comments(comments, cx));
  }

  fn effective_diff_view(&self) -> DiffViewMode {
    if self.show_markdown_preview
      && (self.selected_file_is_markdown() || self.selected_file_is_svg())
    {
      return DiffViewMode::Inline;
    }

    if self.split_disabled_for_selected_file() {
      return DiffViewMode::Inline;
    }

    self.diff_view
  }

  fn sync_diff_view(&mut self, cx: &mut Context<Self>) {
    let diff_view = self.effective_diff_view();
    self
      .diff_editor
      .update(cx, |editor, cx| editor.set_diff_view_mode(diff_view, cx));
  }

  fn toggle_diff_view(&mut self, cx: &mut Context<Self>) {
    if self.split_disabled_for_selected_file() {
      return;
    }
    if self.show_markdown_preview
      && (self.selected_file_is_markdown() || self.selected_file_is_svg())
    {
      return;
    }

    self.diff_view = match self.diff_view {
      DiffViewMode::Inline => DiffViewMode::Split,
      DiffViewMode::Split => DiffViewMode::Inline,
    };
    self.sync_diff_view(cx);
    cx.notify();
  }

  fn toggle_markdown_preview(&mut self, cx: &mut Context<Self>) {
    if !self.selected_file_is_markdown() && !self.selected_file_is_svg() {
      return;
    }

    self.show_markdown_preview = !self.show_markdown_preview;
    self.sync_diff_view(cx);
    cx.notify();
  }

  fn update_svg_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if !self.show_markdown_preview || !self.selected_file_is_svg() {
      return;
    }

    let document = self.diff_editor.read(cx).document().read(cx);
    let svg_source = document.slice_to_string(0..document.len());
    let svg_source: SharedString = svg_source.into();

    if self.svg_preview_source.as_ref() == Some(&svg_source) {
      return;
    }

    self.svg_preview_source = Some(svg_source.clone());
    let renderer = cx.svg_renderer();
    let svg_bytes = svg_source.as_ref().as_bytes().to_vec();
    let background =
      cx.background_spawn(
        async move { renderer.render_single_frame(svg_bytes.as_slice(), 1.0, true) },
      );

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

  fn apply_full_diff(
    &mut self,
    file: &GithubPrFileDiff,
    contents: &GithubPrFileContents,
    cx: &mut Context<Self>,
  ) {
    self.file_loading = false;
    self.file_error = None;
    let head = contents.head.as_deref().unwrap_or("");
    let base = contents.base.as_deref();
    let _ = (
      file.path.clone(),
      contents.base.as_ref().map(|value| value.len()),
      head.len(),
    );
    let diff = compute_buffer_diff(
      DiffKind::Uncommitted,
      base,
      head,
      Path::new(file.path.as_ref()),
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
      let _ = (
        editor.document().read(cx).len(),
        editor.document().read(cx).len_lines(),
      );
      editor.document().update(cx, |doc, cx| {
        doc.replace_all(head, cx);
      });
      editor.reset_after_replace();
      let _ = (
        editor.document().read(cx).len(),
        editor.document().read(cx).len_lines(),
      );
      editor.reset_selection(cx);
      editor.set_diffs(diff_set, cx);
      editor.is_read_only = true;
    });
    self.sync_diff_view(cx);
  }

  fn maybe_fetch_selected_file_contents(&mut self, cx: &mut Context<Self>) {
    if let Some(file) = self.selected_file.clone() {
      self.maybe_fetch_file_contents(file, cx);
    }
  }

  fn maybe_fetch_file_contents(&mut self, file: Rc<GithubPrFileDiff>, cx: &mut Context<Self>) {
    let key = file.path.to_string();
    let key_for_task = key.clone();
    if self.file_contents.contains_key(&key) || self.file_content_tasks.contains_key(&key) {
      return;
    }

    let Some(pull_request) = self.pull_request.as_ref() else {
      return;
    };
    let base_sha_for_diff = if !pull_request.merge_base_sha.is_empty() {
      pull_request.merge_base_sha.clone()
    } else {
      pull_request.base_sha.clone()
    };

    if base_sha_for_diff.is_empty() || pull_request.head_sha.is_empty() {
      return;
    }

    let owner = pull_request.repository.owner.clone();
    let repo = pull_request.repository.repo.clone();
    let head_owner = pull_request
      .head_repository
      .as_ref()
      .map(|repo| repo.owner.clone())
      .unwrap_or_else(|| owner.clone());
    let head_repo = pull_request
      .head_repository
      .as_ref()
      .map(|repo| repo.repo.clone())
      .unwrap_or_else(|| repo.clone());
    let base_sha = base_sha_for_diff;
    let head_sha = pull_request.head_sha.clone();

    let base_path = match file.status {
      GithubPrFileStatus::Added => None,
      GithubPrFileStatus::Renamed => file
        .old_path
        .as_ref()
        .map(|path| path.to_string())
        .or_else(|| Some(file.path.to_string())),
      _ => Some(file.path.to_string()),
    };
    let head_path = match file.status {
      GithubPrFileStatus::Deleted => None,
      _ => Some(file.path.to_string()),
    };

    let api = self.api.clone();
    let task = cx.spawn(async move |this, cx| {
      let base_result = if let Some(path) = base_path.clone() {
        let api = api.clone();
        let owner = owner.clone();
        let repo = repo.clone();
        let base_sha = base_sha.clone();
        unblock(move || api.fetch_github_file_content(&owner, &repo, &path, &base_sha)).await
      } else {
        Ok(None)
      };

      let head_result = if let Some(path) = head_path.clone() {
        let api = api.clone();
        let owner = head_owner.clone();
        let repo = head_repo.clone();
        let head_sha = head_sha.clone();
        unblock(move || api.fetch_github_file_content(&owner, &repo, &path, &head_sha)).await
      } else {
        Ok(None)
      };

      let _ = this.update(cx, |this, cx| {
        this.file_content_tasks.remove(&key_for_task);
        let (base, head) = match (base_result, head_result) {
          (Ok(base), Ok(head)) => (base, head),
          _ => {
            if this.selected_tree_id.as_deref() == Some(key_for_task.as_str()) {
              this.file_loading = false;
              this.file_error = Some("Failed to load file contents".into());
            }
            return;
          }
        };

        if base.is_none() && head.is_none() {
          if this.selected_tree_id.as_deref() == Some(key_for_task.as_str()) {
            this.file_loading = false;
            this.file_error = Some("File contents unavailable".into());
          }
          this
            .file_contents
            .insert(key_for_task.clone(), GithubPrFileContents { base, head });
          return;
        }

        this
          .file_contents
          .insert(key_for_task.clone(), GithubPrFileContents { base, head });

        if this.selected_tree_id.as_deref() == Some(key_for_task.as_str()) {
          if let Some(file) = this.file_lookup.get(&key_for_task).cloned() {
            if let Some(contents) = this.file_contents.get(&key_for_task).cloned() {
              this.apply_full_diff(&file, &contents, cx);
              cx.notify();
            }
          }
        }
      });
    });

    self.file_content_tasks.insert(key, task);
  }

  fn load_pull_request(
    &mut self,
    owner: String,
    repo: String,
    number: u64,
    cx: &mut Context<Self>,
  ) {
    self.active_tab_ix = 0;
    self.error = None;
    self.pull_request = None;
    self.files_loading = true;
    self.files_error = None;
    self.review_comments_loading = true;
    self.review_comments_error = None;
    self.review_comments.clear();
    self.file_loading = false;
    self.file_error = None;
    self.tree_state.update(cx, |state, cx| {
      state.set_items(Vec::new(), cx);
    });
    self.file_lookup.clear();
    self.file_contents.clear();
    self.file_content_tasks.clear();
    self.selected_tree_id = None;
    self.set_selected_file(None, cx);
    self.diff_view = DiffViewMode::Inline;
    self.show_markdown_preview = false;
    self.svg_preview = None;
    self.svg_preview_source = None;
    self.diff_editor.update(cx, |editor, cx| {
      editor.document().update(cx, |doc, cx| {
        doc.replace_all("", cx);
      });
      editor.is_read_only = true;
    });

    let details_api = self.api.clone();
    let details_owner = owner.clone();
    let details_repo = repo.clone();
    let details_task = cx.spawn(async move |this, cx| {
      let result = unblock(move || {
        details_api.fetch_pull_request_details(&details_owner, &details_repo, number)
      })
      .await;

      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(pull_request) => {
            this.pull_request = Some(pull_request);
            this.error = None;
            this.sync_review_comments(cx);
            this.maybe_fetch_selected_file_contents(cx);
          }
          Err(error) => {
            this.pull_request = None;
            this.error = Some(error.to_string().into());
            this.sync_review_comments(cx);
          }
        }
        cx.notify();
      });
    });

    let comments_api = self.api.clone();
    let comments_owner = owner.clone();
    let comments_repo = repo.clone();
    let review_comments_task = cx.spawn(async move |this, cx| {
      let result = unblock(move || {
        comments_api.fetch_pull_request_review_comments(&comments_owner, &comments_repo, number)
      })
      .await;

      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(comments) => {
            this.review_comments = comments;
            this.review_comments_loading = false;
            this.review_comments_error = None;
            this.sync_review_comments(cx);
          }
          Err(error) => {
            this.review_comments_loading = false;
            this.review_comments_error = Some(error.to_string().into());
            this.sync_review_comments(cx);
          }
        }
        cx.notify();
      });
    });

    let files_api = self.api.clone();
    let files_task = cx.spawn(async move |this, cx| {
      let result = unblock(move || files_api.fetch_pull_request_files(&owner, &repo, number)).await;

      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(files) => {
            this.files_loading = false;
            this.files_error = None;
            let files = files_from_api(files);
            let (items, lookup, selected_index, selected_id) = build_tree_items(&files);
            this.file_lookup = lookup;
            this.selected_tree_id = selected_id.clone();
            this.tree_state.update(cx, |state, cx| {
              state.set_items(items, cx);
              state.set_selected_index(selected_index, cx);
            });
            let selected = selected_id.and_then(|id| this.file_lookup.get(&id).cloned());
            this.set_selected_file(selected, cx);
          }
          Err(error) => {
            this.files_loading = false;
            this.files_error = Some(error.to_string().into());
            this.tree_state.update(cx, |state, cx| {
              state.set_items(Vec::new(), cx);
            });
            this.file_lookup.clear();
            this.selected_tree_id = None;
            this.set_selected_file(None, cx);
          }
        }
        cx.notify();
      });
    });

    self.details_task = Some(details_task);
    self.review_comments_task = Some(review_comments_task);
    self.files_task = Some(files_task);
  }

  fn render_header(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    let menu_state = match AuthStateStore::get(cx) {
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
          email: user.email.into(),
          image: user.image.map(Into::into),
        })
      }
    };

    let open_git = Rc::new(|_window: &mut Window, cx: &mut App| {
      let cx = &mut *cx;
      WorkspaceRoute::global_mut(cx).page = WorkspacePage::Git;
      cx.refresh_windows();
    });
    let open_github = Rc::new(|_window: &mut Window, cx: &mut App| {
      let cx = &mut *cx;
      WorkspaceRoute::global_mut(cx).page = WorkspacePage::Github;
      cx.refresh_windows();
    });
    let open_settings = Rc::new(|_window: &mut Window, cx: &mut App| {
      let cx = &mut *cx;
      WorkspaceRoute::open_settings(cx);
      cx.refresh_windows();
    });
    let open_git_config = Rc::new(|_window: &mut Window, cx: &mut App| {
      let cx = &mut *cx;
      WorkspaceRoute::open_git_config(cx);
      cx.refresh_windows();
    });
    let sign_in = Rc::new(|_window: &mut Window, cx: &mut App| {
      AuthCallbackTarget::start_sign_in(cx);
    });
    let sign_out = Rc::new(|_window: &mut Window, cx: &mut App| {
      AuthCallbackTarget::sign_out(cx);
    });

    let auth_control = user_menu(UserMenuConfig {
      id: "auth-menu".into(),
      state: menu_state,
      current_page: UserMenuPage::GithubPrDetails,
      on_open_git: Some(open_git),
      on_open_github: Some(open_github),
      on_open_git_config: Some(open_git_config),
      on_open_settings: Some(open_settings),
      on_sign_in: Some(sign_in),
      on_sign_out: Some(sign_out),
    });

    let tab_bar = TabBar::new("pr-details-tabs")
      .w_full()
      .segmented()
      .selected_index(self.active_tab_ix)
      .on_click(cx.listener(|this, ix: &usize, window, cx| {
        this.set_active_tab(*ix, window, cx);
      }))
      .child(Tab::new().label("Overview"))
      .child(Tab::new().label("Changes"));

    let back_button = || {
      Button::new("pr-back")
        .icon(IconName::ArrowLeft)
        .ghost()
        .compact()
        .on_click(|_, _, cx| {
          WorkspaceRoute::global_mut(cx).page = WorkspacePage::Github;
          cx.refresh_windows();
        })
    };

    let left_area = if let Some(pr) = self.pull_request.as_ref() {
      let status_tag = pr.status().tag(&theme);

      let title = div()
        .min_w_0()
        .text_sm()
        .font_medium()
        .text_color(theme.foreground)
        .overflow_hidden()
        .text_ellipsis_start()
        .child(pr.title.clone());

      let meta = h_flex()
        .items_center()
        .gap_2()
        .text_sm()
        .text_color(theme.muted_foreground)
        .child(status_tag)
        .child(format!("#{}", pr.number));

      h_flex()
        .items_center()
        .gap_2()
        .child(back_button())
        .child(div().flex().items_center().gap_3().child(title).child(meta))
    } else {
      let title_skeleton = Skeleton::new().w(px(220.)).h_4().rounded_md();
      let meta_skeleton = Skeleton::new().w(px(110.)).h_4().rounded_md().secondary();
      h_flex().items_center().gap_2().child(back_button()).child(
        div()
          .flex()
          .items_center()
          .gap_3()
          .child(title_skeleton)
          .child(meta_skeleton),
      )
    };

    div()
      .px_3()
      .py_2()
      .flex()
      .flex_col()
      .gap_1()
      .bg(theme.sidebar)
      .border_b_1()
      .border_color(theme.title_bar_border)
      .child(
        div()
          .flex()
          .items_center()
          .justify_between()
          .child(left_area)
          .when_some(auth_control, |this, control| this.child(control)),
      )
      .child(tab_bar)
  }

  fn render_details(
    &self,
    pr: &GithubPullRequestDetails,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let theme = cx.theme().clone();
    let repo_label = format!("{}/{}", pr.repository.owner, pr.repository.repo);
    let pr_url = format!(
      "https://github.com/{}/{}/pull/{}",
      pr.repository.owner, pr.repository.repo, pr.number
    );
    let updated_at = format_datetime(&pr.updated_at);
    let created_at = format_datetime(&pr.created_at);
    let merged_at = pr.merged_at.as_deref().map(format_datetime);

    let body = pr
      .body
      .clone()
      .filter(|value| !value.trim().is_empty())
      .unwrap_or_else(|| "No description provided.".to_string());

    let stats_badges = h_flex().gap_2().flex_wrap().children([
      Tag::info()
        .small()
        .rounded_full()
        .child(format!("Commits {}", pr.commits)),
      Tag::success()
        .small()
        .rounded_full()
        .child(format!("Additions +{}", pr.additions)),
      Tag::danger()
        .small()
        .rounded_full()
        .child(format!("Deletions -{}", pr.deletions)),
      Tag::warning()
        .small()
        .rounded_full()
        .child(format!("Files changed {}", pr.changed_files)),
      Tag::secondary()
        .small()
        .rounded_full()
        .child(format!("Comments {}", pr.comments)),
      Tag::secondary()
        .small()
        .rounded_full()
        .child(format!("Review comments {}", pr.review_comments)),
    ]);

    let labels_row = if pr.labels.is_empty() {
      None
    } else {
      Some(
        h_flex()
          .gap_1()
          .flex_wrap()
          .children(pr.labels.iter().map(|label| {
            Tag::secondary()
              .small()
              .rounded_full()
              .child(label.name.clone())
          })),
      )
    };

    let content = v_flex()
      .w(px(900.0))
      .gap_4()
      .child(
        h_flex()
          .gap_2()
          .items_center()
          .text_sm()
          .text_color(theme.muted_foreground)
          .child(
            h_flex()
              .items_center()
              .gap_2()
              .child(
                Avatar::new()
                  .name(pr.author.login.clone())
                  .when_some(pr.author.avatar_url.clone(), |this, url| this.src(url))
                  .small(),
              )
              .child(
                div()
                  .text_sm()
                  .text_color(theme.foreground)
                  .child(pr.author.login.clone()),
              ),
          )
          .child(repo_label)
          .child(
            Button::new("open-pr-on-github")
              .icon(IconName::ExternalLink)
              .ghost()
              .small()
              .label("View on GitHub")
              .compact()
              .on_click({
                let pr_url = pr_url.clone();
                move |_, _, cx| {
                  let _ = cx.open_url(&pr_url);
                }
              }),
          ),
      )
      .child(
        h_flex()
          .gap_6()
          .flex_wrap()
          .items_center()
          .child(
            h_flex()
              .items_center()
              .gap_2()
              .child(
                div()
                  .text_sm()
                  .text_color(theme.muted_foreground)
                  .child("Created"),
              )
              .child(
                div()
                  .text_sm()
                  .text_color(theme.foreground)
                  .child(created_at),
              ),
          )
          .child(
            h_flex()
              .items_center()
              .gap_2()
              .child(
                div()
                  .text_sm()
                  .text_color(theme.muted_foreground)
                  .child("Updated"),
              )
              .child(
                div()
                  .text_sm()
                  .text_color(theme.foreground)
                  .child(updated_at),
              ),
          )
          .when_some(merged_at.clone(), |this, merged| {
            this.child(
              h_flex()
                .items_center()
                .gap_2()
                .child(
                  div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child("Merged"),
                )
                .child(
                  div()
                    .text_sm()
                    .text_color(theme.foreground)
                    .child(merged.to_string()),
                ),
            )
          }),
      )
      .child(
        h_flex().items_center().gap_2().child(
          h_flex()
            .items_center()
            .gap_2()
            .child(
              h_flex()
                .items_center()
                .gap_1()
                .child(
                  div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Source"),
                )
                .child(
                  div()
                    .text_sm()
                    .text_color(theme.foreground)
                    .child(pr.head_ref_name.clone()),
                )
                .child(Clipboard::new("copy-pr-branch-source").value(pr.head_ref_name.clone())),
            )
            .child(
              Icon::new(IconName::ArrowRight)
                .size_3()
                .text_color(theme.muted_foreground),
            )
            .child(
              h_flex()
                .items_center()
                .gap_1()
                .child(
                  div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Target"),
                )
                .child(
                  div()
                    .text_sm()
                    .text_color(theme.foreground)
                    .child(pr.base_ref_name.clone()),
                )
                .child(Clipboard::new("copy-pr-branch-target").value(pr.base_ref_name.clone())),
            ),
        ),
      )
      .child(stats_badges)
      .when_some(labels_row, |this, labels| this.child(labels))
      .child(
        v_flex()
          .gap_2()
          .child(
            div()
              .text_sm()
              .font_medium()
              .text_color(theme.foreground)
              .child("Description"),
          )
          .child(
            div()
              .border_1()
              .border_color(theme.border)
              .rounded(theme.radius)
              .p_3()
              .child(render_markdown(
                body.as_str(),
                &MarkdownRenderOptions::default()
                  .with_state(self.description_markdown_state.clone()),
                cx,
              )),
          ),
      );

    v_flex()
      .items_center()
      .py_4()
      .id("ad")
      .overflow_y_scrollbar()
      .child(content)
  }

  fn render_files_sidebar(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let theme = cx.theme().clone();
    let count = self.file_lookup.len();

    if let Some(selected_id) = self
      .tree_state
      .read(cx)
      .selected_entry()
      .map(|entry| entry.item().id.to_string())
    {
      if Some(selected_id.as_str()) != self.selected_tree_id.as_deref()
        && let Some(file) = self.file_lookup.get(&selected_id).cloned()
      {
        self.selected_tree_id = Some(selected_id.clone());
        cx.on_next_frame(window, move |this, _, cx| {
          this.set_selected_file(Some(file), cx);
        });
      }
    }

    let header = div()
      .px_3()
      .flex()
      .items_center()
      .h(px(DIFF_HEADER_HEIGHT))
      .border_b_1()
      .border_color(theme.border)
      .child(
        h_flex()
          .items_center()
          .w_full()
          .justify_between()
          .child(div().text_sm().text_color(theme.foreground).child("Files"))
          .child(
            div()
              .text_xs()
              .text_color(theme.muted_foreground)
              .child(count.to_string()),
          ),
      );

    let mut comment_counts = HashMap::new();
    if !self.review_comments.is_empty() {
      for comment in &self.review_comments {
        *comment_counts.entry(comment.path.clone()).or_insert(0) += 1;
      }
    }

    let list = if self.files_loading {
      v_flex()
        .flex_1()
        .h_full()
        .items_center()
        .justify_center()
        .gap_2()
        .child(Spinner::new().small())
        .child(
          div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child("Loading files..."),
        )
        .into_any_element()
    } else if self.files_error.is_some() {
      v_flex()
        .flex_1()
        .h_full()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.status_red())
        .child(self.files_error.clone().unwrap_or_default())
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
      tree(
        &self.tree_state,
        move |ix, entry, _selected, _window, cx| {
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
            let status_letter = status.map(status_letter).unwrap_or("");
            let status_color = status
              .map(|status| status_color(status, &theme))
              .unwrap_or(theme.muted_foreground);
            let comment_count = if is_folder {
              0
            } else {
              comment_counts.get(item.id.as_ref()).copied().unwrap_or(0)
            };
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
            let mut row = ListItem::new(ix)
              .w_full()
              .rounded(theme.radius)
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
                      .text_ellipsis_start()
                      .child(item.label.clone()),
                  )
                  .when(comment_count > 0, |this| {
                    this.child(
                      h_flex()
                        .items_center()
                        .gap_1()
                        .text_xs()
                        .pr_2()
                        .text_color(theme.muted_foreground)
                        .child(
                          Icon::new(UiIconName::MessageCircle)
                            .size_3()
                            .text_color(theme.muted_foreground),
                        )
                        .child(comment_count.to_string()),
                    )
                  }),
              );

            if !is_folder && this.file_lookup.contains_key(item.id.as_ref()) {
              let id = item.id.clone();
              row = row.on_click(cx.listener(move |this, _, _, cx| {
                if let Some(file) = this.file_lookup.get(id.as_ref()).cloned() {
                  this.set_selected_file(Some(file), cx);
                }
              }));
            }

            row
          })
        },
      )
      .flex_1()
      .w_full()
      .into_any_element()
    };

    v_flex()
      .bg(theme.sidebar)
      .size_full()
      .child(header)
      .child(div().flex_1().min_h_0().child(list))
  }

  fn render_diff_header(
    &self,
    file: &GithubPrFileDiff,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
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

    let status_letter = status_letter(file.status);
    let status_color = status_color(file.status, &theme);
    let is_markdown = Self::is_markdown_path(path);
    let is_svg = Self::is_svg_path(path);
    let preview_active = (is_markdown || is_svg) && self.show_markdown_preview;
    let file_loading = self.file_loading;
    let split_disabled = self.split_disabled_for_file(file) || preview_active;
    let (toggle_label, toggle_icon) = if split_disabled {
      ("Split", IconName::PanelLeft)
    } else {
      match self.diff_view {
        DiffViewMode::Inline => ("Split", IconName::PanelLeft),
        DiffViewMode::Split => ("Inline", IconName::PanelLeftClose),
      }
    };
    let view = cx.entity();
    let toggle_button = Button::new("pr-diff-toggle")
      .label(toggle_label)
      .icon(toggle_icon)
      .xsmall()
      .ghost()
      .disabled(split_disabled || file_loading)
      .on_click(move |_, _, cx| {
        view.update(cx, |this, cx| {
          this.toggle_diff_view(cx);
        });
      });
    let view = cx.entity();
    let preview_button = Button::new("pr-markdown-preview")
      .label("Preview")
      .icon(if preview_active {
        IconName::EyeOff
      } else {
        IconName::Eye
      })
      .xsmall()
      .ghost()
      .selected(preview_active)
      .disabled(file_loading)
      .on_click(move |_, _, cx| {
        view.update(cx, |this, cx| {
          this.toggle_markdown_preview(cx);
        });
      });

    div()
      .h(px(DIFF_HEADER_HEIGHT))
      .bg(theme.sidebar)
      .px_3()
      .flex()
      .items_center()
      .justify_between()
      .border_b_1()
      .border_color(theme.border)
      .child(
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
          .child(icon)
          .child({
            let mut label = Label::new(file_name);
            if !dir_path.is_empty() {
              label = label.secondary(format!("- {}", dir_path));
            }
            label.truncate()
          }),
      )
      .child(
        h_flex()
          .items_center()
          .gap_2()
          .child(toggle_button)
          .when(is_markdown || is_svg, |this| this.child(preview_button)),
      )
  }

  fn show_file_search_action(
    &mut self,
    _: &ShowFileSearch,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.active_tab_ix != 1 {
      return;
    }

    self.open_file_search_palette(window, cx);
  }

  fn show_command_palette_action(
    &mut self,
    _: &ShowCommandPalette,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.open_command_palette(window, cx);
  }

  fn find_action(&mut self, action: &Find, window: &mut Window, cx: &mut Context<Self>) {
    if self.active_tab_ix != 1 {
      return;
    }

    self.diff_editor.update(cx, |editor, cx| {
      editor::find(editor, action, window, cx);
    });
  }

  fn close_find_action(&mut self, action: &CloseFind, window: &mut Window, cx: &mut Context<Self>) {
    if self.active_tab_ix != 1 {
      return;
    }

    self.diff_editor.update(cx, |editor, cx| {
      editor::close_find(editor, action, window, cx);
    });
  }

  fn open_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let include_github = matches!(AuthStateStore::get(cx), AuthState::Authenticated(_));
    let commands = CommandPaletteCommand::default_global_commands(
      CommandPalettePage::GithubPrDetails,
      include_github,
    );

    let view = cx.entity();
    let handler: CommandPaletteHandler = Arc::new(move |action, _window, cx| {
      view.update(cx, |view, cx| {
        view.handle_command_palette_action(action, cx)
      })
    });

    let config = CommandPaletteConfig::new(Vec::new(), commands, handler);
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

  fn handle_command_palette_action(
    &mut self,
    action: CommandPaletteAction,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    match action {
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
      CommandPaletteAction::OpenGithubPrDetails {
        owner,
        repo,
        number,
      } => {
        self.load_pull_request(owner.to_string(), repo.to_string(), number, cx);
        WorkspaceRoute::global_mut(cx).page = WorkspacePage::GithubPrDetails;
        cx.refresh_windows();
        Ok(())
      }
      CommandPaletteAction::OpenSettingsPage => {
        WorkspaceRoute::open_settings(cx);
        cx.refresh_windows();
        Ok(())
      }
      CommandPaletteAction::OpenGitConfigPage => {
        WorkspaceRoute::open_git_config(cx);
        cx.refresh_windows();
        Ok(())
      }
      _ => Err("Command not available.".into()),
    }
  }

  fn open_file_search_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.file_lookup.is_empty() {
      return;
    }

    let mut entries = self
      .file_lookup
      .values()
      .map(|file| {
        let path = PathBuf::from(file.path.as_ref());
        let label = file.path.as_ref().replace(['\n', '\r'], "");
        SearchFileEntry::new(path, label)
      })
      .collect::<Vec<_>>();
    entries.sort_by(|a, b| a.label.cmp(&b.label));

    let view = cx.entity();
    let handler: SearchFileHandler = Arc::new(move |path, window, cx| {
      view.update(cx, |view, cx| {
        view.select_file_from_palette(&path, cx);
      });

      let view_for_focus = view.clone();
      window.on_next_frame(move |window, cx| {
        let focus_handle = view_for_focus
          .read(cx)
          .diff_editor
          .read(cx)
          .focus_handle(cx);
        window.focus(&focus_handle, cx);
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

  fn select_file_from_palette(&mut self, path: &Path, cx: &mut Context<Self>) {
    let key = path.to_string_lossy().to_string();
    let Some(file) = self.file_lookup.get(&key).cloned() else {
      return;
    };

    let tree_item = TreeItem::new(key.clone(), key.clone());
    self.tree_state.update(cx, |state, cx| {
      state.set_selected_item(Some(&tree_item), cx);
      if let Some(ix) = state.selected_index() {
        state.scroll_to_item(ix, gpui::ScrollStrategy::Top);
      }
    });

    self.set_selected_file(Some(file), cx);
  }

  fn sync_tree_selection(&mut self, cx: &mut Context<Self>) {
    let Some(file) = self.selected_file.as_ref() else {
      return;
    };

    let key = file.path.as_ref().to_string();
    let tree_item = TreeItem::new(key.clone(), key.clone());
    self.tree_state.update(cx, |state, cx| {
      state.set_selected_item(Some(&tree_item), cx);
      if let Some(ix) = state.selected_index() {
        state.scroll_to_item(ix, gpui::ScrollStrategy::Top);
      }
    });
  }

  fn render_changes_tab(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let theme = cx.theme().clone();
    let is_markdown = self.selected_file_is_markdown();
    let is_svg = self.selected_file_is_svg();
    let preview_active = self.show_markdown_preview && (is_markdown || is_svg);

    let editor_content: gpui::AnyElement = if self.files_loading {
      v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_2()
        .child(Spinner::new().small())
        .child(
          div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child("Loading diff..."),
        )
        .into_any_element()
    } else if self.file_loading {
      v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_2()
        .child(Spinner::new().small())
        .child(
          div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child("Loading file contents..."),
        )
        .into_any_element()
    } else if self.file_error.is_some() {
      v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.status_red())
        .child(self.file_error.clone().unwrap_or_default())
        .into_any_element()
    } else if self.files_error.is_some() {
      v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.status_red())
        .child(self.files_error.clone().unwrap_or_default())
        .into_any_element()
    } else if self.selected_file.is_some() {
      if preview_active {
        let preview_panel = if is_svg {
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
          let markdown = self.diff_editor.read(cx).document().read(cx);
          let markdown = markdown.slice_to_string(0..markdown.len());
          div()
            .flex_1()
            .min_h_0()
            .min_w(px(0.0))
            .bg(theme.background)
            .child(
              div().size_full().pb_4().px_4().child(
                TextView::markdown("github-pr-markdown-preview-text", markdown)
                  .size_full()
                  .selectable(true)
                  .scrollable(true),
              ),
            )
            .into_any_element()
        };
        div()
          .flex_1()
          .min_h_0()
          .child(
            h_resizable("github-pr-markdown-preview")
              .child(resizable_panel().child(self.diff_editor.clone()))
              .child(resizable_panel().child(preview_panel)),
          )
          .into_any_element()
      } else {
        div()
          .flex_1()
          .min_h_0()
          .child(self.diff_editor.clone())
          .into_any_element()
      }
    } else {
      div()
        .flex_1()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.muted_foreground)
        .child("Select a file to view diff")
        .into_any_element()
    };

    let editor_panel = v_flex()
      .size_full()
      .overflow_hidden()
      .when_some(self.selected_file.as_ref(), |this, file| {
        this.child(self.render_diff_header(file, cx))
      })
      .child(editor_content);

    h_resizable("github-pr-changes")
      .child(
        resizable_panel()
          .size(px(SIDEBAR_DEFAULT_WIDTH))
          .size_range(px(SIDEBAR_MIN_WIDTH)..px(SIDEBAR_MAX_WIDTH))
          .child(self.render_files_sidebar(window, cx)),
      )
      .child(resizable_panel().child(editor_panel))
  }
}

impl Render for GithubPrDetailsPage {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    let overview_inner: gpui::AnyElement = if let Some(pr) = self.pull_request.as_ref() {
      self.render_details(pr, cx).into_any_element()
    } else if self.error.is_some() {
      v_flex()
        .flex_1()
        .h_full()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.status_red())
        .child(self.error.clone().unwrap_or_default())
        .into_any_element()
    } else {
      v_flex()
        .flex_1()
        .h_full()
        .items_center()
        .justify_center()
        .gap_2()
        .child(Spinner::new().small())
        .child(
          div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child("Loading pull request details..."),
        )
        .into_any_element()
    };

    let overview_content = div()
      .id("overview-tab")
      .flex_1()
      .min_h_0()
      .child(overview_inner)
      .into_any_element();

    let changes_content = div()
      .id("changes-tab")
      .flex_1()
      .min_h_0()
      .child(self.render_changes_tab(window, cx))
      .into_any_element();

    let content = if self.active_tab_ix == 0 {
      overview_content
    } else {
      changes_content
    };

    div()
      .size_full()
      .flex()
      .flex_col()
      .bg(theme.background)
      .track_focus(&self.focus_handle(cx))
      .on_action(cx.listener(GithubPrDetailsPage::show_command_palette_action))
      .on_action(cx.listener(GithubPrDetailsPage::show_file_search_action))
      .on_action(cx.listener(GithubPrDetailsPage::find_action))
      .on_action(cx.listener(GithubPrDetailsPage::close_find_action))
      .child(self.render_header(cx))
      .child(v_flex().flex_1().min_h_0().child(content))
  }
}

impl Focusable for GithubPrDetailsPage {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::api::GithubPullRequestFile;

  fn make_api_file(
    filename: &str,
    status: &str,
    previous_filename: Option<&str>,
  ) -> GithubPullRequestFile {
    GithubPullRequestFile {
      filename: filename.to_string(),
      status: status.to_string(),
      patch: None,
      previous_filename: previous_filename.map(str::to_string),
    }
  }

  #[test]
  fn format_datetime_returns_date_component_for_iso_values() {
    assert_eq!(
      format_datetime("2026-02-15T12:34:56Z").as_ref(),
      "2026-02-15"
    );
    assert_eq!(format_datetime("2026-02-15").as_ref(), "2026-02-15");
  }

  #[test]
  fn status_letter_covers_all_file_statuses() {
    assert_eq!(status_letter(GithubPrFileStatus::Added), "A");
    assert_eq!(status_letter(GithubPrFileStatus::Modified), "M");
    assert_eq!(status_letter(GithubPrFileStatus::Deleted), "D");
    assert_eq!(status_letter(GithubPrFileStatus::Renamed), "R");
  }

  #[test]
  fn map_file_status_handles_known_and_unknown_values() {
    assert_eq!(map_file_status("added"), GithubPrFileStatus::Added);
    assert_eq!(map_file_status("removed"), GithubPrFileStatus::Deleted);
    assert_eq!(map_file_status("deleted"), GithubPrFileStatus::Deleted);
    assert_eq!(map_file_status("renamed"), GithubPrFileStatus::Renamed);
    assert_eq!(map_file_status("changed"), GithubPrFileStatus::Modified);
    assert_eq!(map_file_status("ADDED"), GithubPrFileStatus::Added);
    assert_eq!(map_file_status(" deleted "), GithubPrFileStatus::Deleted);
  }

  #[test]
  fn markdown_path_detection_is_case_insensitive_and_extension_based() {
    assert!(GithubPrDetailsPage::is_markdown_path(Path::new(
      "README.md"
    )));
    assert!(GithubPrDetailsPage::is_markdown_path(Path::new(
      "docs/GUIDE.MD"
    )));
    assert!(GithubPrDetailsPage::is_markdown_path(Path::new(
      "notes.markdown"
    )));
    assert!(GithubPrDetailsPage::is_markdown_path(Path::new("post.MdX")));

    assert!(!GithubPrDetailsPage::is_markdown_path(Path::new("README")));
    assert!(!GithubPrDetailsPage::is_markdown_path(Path::new(
      "icon.svg"
    )));
    assert!(!GithubPrDetailsPage::is_markdown_path(Path::new(
      "note.md.txt"
    )));
  }

  #[test]
  fn svg_path_detection_is_case_insensitive_and_extension_based() {
    assert!(GithubPrDetailsPage::is_svg_path(Path::new("icon.svg")));
    assert!(GithubPrDetailsPage::is_svg_path(Path::new("ICON.SVG")));

    assert!(!GithubPrDetailsPage::is_svg_path(Path::new("icon.svgz")));
    assert!(!GithubPrDetailsPage::is_svg_path(Path::new("README.md")));
    assert!(!GithubPrDetailsPage::is_svg_path(Path::new("icon")));
  }

  #[test]
  fn files_from_api_sets_unknown_filename_and_rename_old_path() {
    let files = files_from_api(vec![
      make_api_file("", "added", None),
      make_api_file("src/new.rs", "renamed", Some("src/old.rs")),
      make_api_file("src/main.rs", "modified", Some("src/very_old.rs")),
    ]);

    assert_eq!(files[0].path.as_ref(), "unknown");
    assert_eq!(files[0].old_path, None);

    assert_eq!(files[1].status, GithubPrFileStatus::Renamed);
    assert_eq!(files[1].path.as_ref(), "src/new.rs");
    assert_eq!(
      files[1].old_path.as_ref().map(|v| v.as_ref()),
      Some("src/old.rs")
    );

    assert_eq!(files[2].status, GithubPrFileStatus::Modified);
    assert_eq!(files[2].old_path, None);
  }

  #[test]
  fn build_tree_items_prefers_folder_and_selects_first_file() {
    let files = files_from_api(vec![
      make_api_file("README.md", "modified", None),
      make_api_file("src/lib.rs", "modified", None),
      make_api_file("src/main.rs", "modified", None),
    ]);

    let (items, lookup, selected_index, selected_id) = build_tree_items(&files);

    assert_eq!(items.len(), 2);
    assert_eq!(items[0].label.as_ref(), "src");
    assert_eq!(items[0].children.len(), 2);
    assert_eq!(items[0].children[0].label.as_ref(), "lib.rs");
    assert_eq!(items[0].children[1].label.as_ref(), "main.rs");
    assert_eq!(items[1].label.as_ref(), "README.md");

    assert_eq!(selected_id.as_deref(), Some("src/lib.rs"));
    assert_eq!(selected_index, Some(0));
    assert!(lookup.contains_key("src/lib.rs"));
    assert!(lookup.contains_key("README.md"));
  }

  #[test]
  fn build_tree_items_empty_input_has_no_selection() {
    let (items, lookup, selected_index, selected_id) = build_tree_items(&[]);
    assert!(items.is_empty());
    assert!(lookup.is_empty());
    assert_eq!(selected_index, None);
    assert_eq!(selected_id, None);
  }
}
