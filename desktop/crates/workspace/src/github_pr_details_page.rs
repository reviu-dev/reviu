use std::{
  collections::{BTreeMap, HashMap},
  path::{Path, PathBuf},
  rc::Rc,
};

use editor::Editor;
use git::{DiffKind, DiffLineKind, DiffSet, FileDiff, compute_buffer_diff, diff_set_from_patch};
use gpui::{
  App, Context, Entity, FocusHandle, Focusable, ParentElement, Render, SharedString, Styled, Task,
  Window, div, img, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt,
  avatar::Avatar,
  button::{Button, ButtonVariants as _},
  clipboard::Clipboard,
  h_flex,
  label::Label,
  list::ListItem,
  spinner::Spinner,
  tab::{Tab, TabBar},
  tag::Tag,
  text::TextView,
  tree::{TreeItem, TreeState, tree},
  v_flex,
};
use smol::unblock;

use ui::{
  FILE_ICON_SIZE_PX, HEADER_HEIGHT, StatusThemeExt, file_icon_path_for_name_with_theme,
  h_resizable, resizable_panel,
};

use crate::{
  api::{ApiClient, GithubPullRequestDetails},
  github_page::GithubPageHandle,
  workspace::{WorkspaceApi, WorkspacePage, WorkspaceRoute},
};

const SIDEBAR_DEFAULT_WIDTH: f32 = 350.0;
const SIDEBAR_MIN_WIDTH: f32 = 250.0;
const SIDEBAR_MAX_WIDTH: f32 = 600.0;
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
  diff: String,
  diff_set: Option<DiffSet>,
  document: String,
}

fn parse_github_diff(diff: &str) -> Vec<Rc<GithubPrFileDiff>> {
  let mut files: Vec<Rc<GithubPrFileDiff>> = Vec::new();
  let mut current: Option<GithubPrFileDiff> = None;

  for line in diff.lines() {
    if line.starts_with("diff --git ") || line.starts_with("diff --cc ") {
      if let Some(file) = current.take() {
        files.push(Rc::new(finalize_diff(file)));
      }

      let parts: Vec<&str> = line.split_whitespace().collect();
      let mut old_path = if parts.len() >= 3 {
        Some(parts[2].trim_start_matches("a/").to_string())
      } else {
        None
      };
      let mut path = if parts.len() >= 4 {
        parts[3].trim_start_matches("b/").to_string()
      } else if parts.len() >= 3 {
        parts[2].trim_start_matches("b/").to_string()
      } else {
        "unknown".to_string()
      };

      if path == "/dev/null" {
        path = parts
          .get(2)
          .map(|value| value.trim_start_matches("a/").to_string())
          .unwrap_or_else(|| "unknown".to_string());
      }
      if old_path.as_deref() == Some("/dev/null") {
        old_path = None;
      }

      current = Some(GithubPrFileDiff {
        path: path.into(),
        old_path: old_path.map(Into::into),
        status: GithubPrFileStatus::Modified,
        diff: String::new(),
        diff_set: None,
        document: String::new(),
      });
    }

    if let Some(file) = current.as_mut() {
      file.diff.push_str(line);
      file.diff.push('\n');

      if line.starts_with("new file mode") || line.starts_with("--- /dev/null") {
        file.status = GithubPrFileStatus::Added;
      } else if line.starts_with("deleted file mode") || line.starts_with("+++ /dev/null") {
        file.status = GithubPrFileStatus::Deleted;
      } else if line.starts_with("rename from ") || line.starts_with("rename to ") {
        file.status = GithubPrFileStatus::Renamed;
        if let Some(rename_to) = line.strip_prefix("rename to ") {
          file.path = rename_to.to_string().into();
        } else if let Some(rename_from) = line.strip_prefix("rename from ") {
          file.old_path = Some(rename_from.to_string().into());
        }
      } else if let Some(path) = line.strip_prefix("+++ b/") {
        if file.path.as_ref() == "unknown" {
          file.path = path.to_string().into();
        }
      } else if let Some(path) = line.strip_prefix("--- a/") {
        if file.path.as_ref() == "unknown" {
          file.path = path.to_string().into();
        }
        if file.old_path.is_none() {
          file.old_path = Some(path.to_string().into());
        }
      }
    }
  }

  if let Some(file) = current.take() {
    files.push(Rc::new(finalize_diff(file)));
  }

  files
}

fn finalize_diff(mut file: GithubPrFileDiff) -> GithubPrFileDiff {
  if let Ok(diff_set) = diff_set_from_patch(&file.diff) {
    if !diff_set.uncommitted.hunks.is_empty() {
      file.document = build_document_from_diff(&diff_set.uncommitted);
      file.diff_set = Some(diff_set);
      return file;
    }
  }

  file.document = file.diff.clone();
  file.diff_set = None;
  file
}

fn build_document_from_diff(file_diff: &git::FileDiff) -> String {
  let mut max_line = 0usize;
  for hunk in &file_diff.hunks {
    if hunk.new_lines > 0 {
      let end = hunk
        .new_start
        .saturating_add(hunk.new_lines)
        .saturating_sub(1);
      if end > max_line {
        max_line = end;
      }
    }
  }

  if max_line == 0 {
    return String::new();
  }

  let mut lines = vec![String::new(); max_line];
  for hunk in &file_diff.hunks {
    if hunk.new_lines == 0 {
      continue;
    }

    let mut line_index = hunk.new_start.saturating_sub(1);
    for line in &hunk.lines {
      match line.kind {
        DiffLineKind::Context | DiffLineKind::Add => {
          if line_index < lines.len() {
            lines[line_index] = line.content.clone();
          }
          line_index = line_index.saturating_add(1);
        }
        DiffLineKind::Remove => {}
      }
    }
  }

  lines.join("\n")
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
  diff_task: Option<Task<()>>,
  diff_loading: bool,
  diff_error: Option<SharedString>,
  file_loading: bool,
  file_error: Option<SharedString>,
  tree_state: Entity<TreeState>,
  file_lookup: HashMap<String, Rc<GithubPrFileDiff>>,
  file_contents: HashMap<String, GithubPrFileContents>,
  file_content_tasks: HashMap<String, Task<()>>,
  selected_file: Option<Rc<GithubPrFileDiff>>,
  selected_tree_id: Option<String>,
  diff_editor: Entity<Editor>,
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
      diff_task: None,
      diff_loading: false,
      diff_error: None,
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
      active_tab_ix: 0,
      pull_request: None,
      error: None,
    };

    view
  }

  fn set_active_tab(&mut self, ix: usize, _: &mut Window, cx: &mut Context<Self>) {
    self.active_tab_ix = ix;
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

    if let Some(file) = selected {
      self.ensure_diff_editor_for_path(file.path.as_ref(), cx);
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
    if pull_request.base_sha.is_empty() || pull_request.head_sha.is_empty() {
      return;
    }

    let owner = pull_request.repository.owner.clone();
    let repo = pull_request.repository.repo.clone();
    let base_sha = pull_request.base_sha.clone();
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
        let owner = owner.clone();
        let repo = repo.clone();
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
    self.diff_loading = true;
    self.diff_error = None;
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
            this.maybe_fetch_selected_file_contents(cx);
          }
          Err(error) => {
            this.pull_request = None;
            this.error = Some(error.to_string().into());
          }
        }
        cx.notify();
      });
    });

    let diff_api = self.api.clone();
    let diff_task = cx.spawn(async move |this, cx| {
      let result = unblock(move || diff_api.fetch_pull_request_diff(&owner, &repo, number)).await;

      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(diff) => {
            this.diff_loading = false;
            this.diff_error = None;
            let files = parse_github_diff(&diff);
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
            this.diff_loading = false;
            this.diff_error = Some(error.to_string().into());
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
    self.diff_task = Some(diff_task);
  }

  fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    let back_button = Button::new("back-github")
      .icon(IconName::ArrowRight)
      .label("Back")
      .ghost()
      .compact()
      .on_click(|_, _, cx| {
        GithubPageHandle::request_focus(cx);
        WorkspaceRoute::global_mut(cx).page = WorkspacePage::Github;
        cx.refresh_windows();
      });

    div()
      .h(px(HEADER_HEIGHT))
      .max_h(px(HEADER_HEIGHT))
      .px_4()
      .flex()
      .items_center()
      .justify_between()
      .bg(theme.sidebar)
      .border_b_1()
      .border_color(theme.title_bar_border)
      .child(
        div()
          .text_sm()
          .text_color(theme.foreground)
          .child("Pull Request"),
      )
      .child(back_button)
  }

  fn render_loading(&self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    v_flex()
      .flex_1()
      .items_center()
      .justify_center()
      .mt_12()
      .gap_2()
      .child(Spinner::new().small())
      .child(
        div()
          .text_sm()
          .text_color(theme.muted_foreground)
          .child("Loading pull request details..."),
      )
  }

  fn render_details(
    &self,
    pr: &GithubPullRequestDetails,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let theme = cx.theme().clone();
    let status_tag = if pr.merged_at.is_some() {
      Tag::custom(
        theme.status_violet(),
        theme.primary_foreground,
        theme.status_violet(),
      )
      .small()
      .rounded_full()
      .child("Merged")
    } else if pr.state == "open" {
      Tag::success().small().rounded_full().child("Open")
    } else {
      Tag::secondary().small().rounded_full().child("Closed")
    };

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
      .max_w(px(900.0))
      .gap_4()
      .child(
        v_flex()
          .gap_2()
          .child(
            h_flex().items_center().gap_2().child(
              div()
                .flex_1()
                .text_lg()
                .font_medium()
                .text_color(theme.foreground)
                .child(pr.title.clone()),
            ),
          )
          .child(
            h_flex()
              .gap_2()
              .items_center()
              .text_sm()
              .text_color(theme.muted_foreground)
              .child(status_tag)
              .child(format!("#{}", pr.number))
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
              .child(TextView::markdown("pr-body", body).selectable(true)),
          ),
      );

    v_flex().items_center().py_4().child(content)
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

    let list = if count == 0 {
      div()
        .flex_1()
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
                  ),
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

    div()
      .h(px(DIFF_HEADER_HEIGHT))
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
  }

  fn render_changes_tab(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let theme = cx.theme().clone();

    let editor_content: gpui::AnyElement = if self.diff_loading {
      div()
        .flex_1()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.muted_foreground)
        .child("Loading diff...")
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
      div()
        .flex_1()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.danger)
        .child(self.file_error.clone().unwrap_or_default())
        .into_any_element()
    } else if self.diff_error.is_some() {
      div()
        .flex_1()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.danger)
        .child(self.diff_error.clone().unwrap_or_default())
        .into_any_element()
    } else if self.selected_file.is_some() {
      div()
        .flex_1()
        .min_h_0()
        .child(self.diff_editor.clone())
        .into_any_element()
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

    let overview_content: gpui::AnyElement = if let Some(pr) = self.pull_request.as_ref() {
      self.render_details(pr, cx).into_any_element()
    } else if self.error.is_some() {
      div()
        .flex_1()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.danger)
        .child(self.error.clone().unwrap_or_default())
        .into_any_element()
    } else {
      self.render_loading(cx).into_any_element()
    };

    let changes_content = self.render_changes_tab(window, cx).into_any_element();

    let tab_bar = TabBar::new("pr-details-tabs")
      .w_full()
      .px_6()
      .underline()
      .selected_index(self.active_tab_ix)
      .on_click(cx.listener(|this, ix: &usize, window, cx| {
        this.set_active_tab(*ix, window, cx);
      }))
      .child(Tab::new().label("Overview"))
      .child(Tab::new().label("Changes"));

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
      .child(self.render_header(cx))
      .child(
        v_flex()
          .flex_1()
          .child(tab_bar)
          .child(div().flex_1().child(content)),
      )
  }
}

impl Focusable for GithubPrDetailsPage {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}
