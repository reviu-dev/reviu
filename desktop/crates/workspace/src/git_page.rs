use std::{
  collections::{HashMap, HashSet},
  path::{Path, PathBuf},
  rc::Rc,
  sync::Arc,
  time::Duration,
};

use editor::{CloseFind, DiffViewMode, Editor, Find, HunkAction, HunkState};
use gfm_markdown_viewer::{MarkdownRenderOptions, MarkdownRenderState, render_markdown};
use git::{
  BranchKind, BranchRef, BranchStatus, CommitGraphNode, HeadCommitStatus, RepoStage,
  RepoStatusEntry, RepoStatusKind, amend_commit, commit_changes, create_branch, create_branch_from,
  current_branch_status, delete_untracked_file, head_commit_status, list_branches,
  list_commit_graph, list_repo_status, merge_branch, push, restore_file, stage_all, stage_file,
  switch_branch, undo_last_commit, unstage_all, unstage_file,
};
use gpui::{
  AnyElement, AnyWindowHandle, App, Bounds, Context, Corner, Element, ElementId, Entity,
  FocusHandle, Focusable, Global, GlobalElementId, InspectorElementId, InteractiveElement,
  Keystroke, LayoutId, PaintQuad, ParentElement, Path as GpuiPath, PathBuilder, PathPromptOptions,
  Pixels, Render, RenderImage, SharedString, Style, Styled, Task, WeakEntity, Window, actions, div,
  fill, img, point, prelude::*, px, size,
};
use gpui_component::{
  ActiveTheme as _, Disableable, Icon, IconName, IndexPath, Selectable, Sizable,
  button::{Button, ButtonVariant, ButtonVariants as _},
  h_flex,
  kbd::Kbd,
  label::Label,
  list::{List, ListDelegate, ListEvent, ListItem, ListState},
  menu::{DropdownMenu, PopupMenuItem},
  scroll::ScrollableElement,
  select::{SearchableVec, Select, SelectEvent, SelectItem, SelectState},
  spinner::Spinner,
  tooltip::Tooltip,
};
use smol::unblock;

use crate::{
  api::ApiClient,
  auth_state::{AuthState, AuthStateStore},
  config::{ConfigStore, RecentRepository},
  github_page::GithubPageHandle,
  github_pr_details_page::GithubPrDetailsPageHandle,
  workspace::{WorkspaceApi, WorkspacePage, WorkspaceRoute},
};
use ui::{
  CommandPalette, CommandPaletteAction, CommandPaletteBranch, CommandPaletteBranchKind,
  CommandPaletteCommand, CommandPaletteConfig, CommandPaletteHandler, CommandPalettePage,
  ConfirmDialog, FILE_ICON_SIZE_PX, HEADER_HEIGHT, Input, InputState, SearchFileEntry,
  SearchFileHandler, SearchFilePalette, SearchFilePaletteConfig, StatusThemeExt, UserMenuConfig,
  UserMenuPage, UserMenuState, UserMenuUser, WindowExt, file_icon_path_for_path_with_theme,
  user_menu,
};

const SIDEBAR_DEFAULT_WIDTH: f32 = 350.0;
const SIDEBAR_MIN_WIDTH: f32 = 250.0;
const SIDEBAR_MAX_WIDTH: f32 = 1500.0;
const STATUS_POLL_INTERVAL_MS: u64 = 800;
const EDITOR_HEADER_HEIGHT: f32 = 40.0;
const GRAPH_MAX_COMMITS: usize = 200;
const GRAPH_ROW_HEIGHT: f32 = 30.0;
const GRAPH_LANE_WIDTH: f32 = 14.0;
const GRAPH_LINE_WIDTH: f32 = 2.0;
const GRAPH_DOT_SIZE: f32 = 10.0;
const GRAPH_DOT_SIZE_LATEST: f32 = 14.0;
const GRAPH_BRANCH_BADGE_MAX_WIDTH: f32 = 190.0;
const GRAPH_AUTHOR_MAX_WIDTH: f32 = 180.0;
const GRAPH_POLL_EVERY_TICKS: u32 = 6;

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

struct GitGraphListDelegate {
  rows: Vec<Rc<GraphRenderRow>>,
}

impl GitGraphListDelegate {
  fn new() -> Self {
    Self { rows: Vec::new() }
  }

  fn set_rows(&mut self, rows: Vec<GraphRenderRow>) {
    self.rows = rows.into_iter().map(Rc::new).collect();
  }
}

impl ListDelegate for GitGraphListDelegate {
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
    let row = self.rows.get(ix.row)?;
    let theme = cx.theme().clone();
    Some(
      ListItem::new(ix)
        .pl_0()
        .pr_1()
        .py_0()
        .child(GitPage::render_graph_row(ix.row, row.as_ref(), &theme)),
    )
  }

  fn render_empty(
    &mut self,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> impl IntoElement {
    div()
      .id("git-graph-empty-state")
      .size_full()
      .items_center()
      .justify_center()
      .text_sm()
      .text_color(cx.theme().muted_foreground)
      .child("No commits to display")
  }

  fn set_selected_index(
    &mut self,
    _: Option<IndexPath>,
    _window: &mut Window,
    _cx: &mut Context<ListState<Self>>,
  ) {
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GitSidebarMode {
  Changes,
  Graph,
}

#[derive(Clone, Copy, Debug, Default)]
struct GraphLaneSegment {
  up: bool,
  down: bool,
}

#[derive(Clone, Debug)]
struct GraphRenderRow {
  commit: CommitGraphNode,
  segments: Vec<GraphLaneSegment>,
  commit_lane: usize,
  merge_parent_lanes: Vec<usize>,
  branch_child_lanes: Vec<usize>,
  branch_pre_stubs: Vec<usize>,
  commit_lane_has_up: bool,
}

struct GraphPaintPath {
  path: GpuiPath<Pixels>,
  color: gpui::Hsla,
}

struct GraphLanesPrepaintState {
  paths: Vec<GraphPaintPath>,
  dot: PaintQuad,
}

struct GraphLanesElement {
  id: ElementId,
  segments: Vec<GraphLaneSegment>,
  commit_lane: usize,
  merge_parent_lanes: Vec<usize>,
  branch_child_lanes: Vec<usize>,
  branch_pre_stubs: Vec<usize>,
  commit_lane_has_up: bool,
  is_latest_commit: bool,
  latest_dot_background: gpui::Hsla,
  lane_colors: Vec<gpui::Hsla>,
  dot_color: gpui::Hsla,
}

impl GraphLanesElement {
  fn new(row_index: usize, row: &GraphRenderRow, theme: &gpui_component::Theme) -> Self {
    let lane_count = row.segments.len().max(1);
    let lane_colors = (0..lane_count)
      .map(|lane_ix| Self::lane_color_for(lane_ix, theme))
      .collect::<Vec<_>>();
    let dot_color = lane_colors
      .get(row.commit_lane % lane_count)
      .cloned()
      .unwrap_or(theme.sidebar_foreground);

    Self {
      id: ("git-graph-lanes", row_index).into(),
      segments: row.segments.clone(),
      commit_lane: row.commit_lane,
      merge_parent_lanes: row.merge_parent_lanes.clone(),
      branch_child_lanes: row.branch_child_lanes.clone(),
      branch_pre_stubs: row.branch_pre_stubs.clone(),
      commit_lane_has_up: row.commit_lane_has_up,
      is_latest_commit: row_index == 0,
      latest_dot_background: theme.sidebar,
      lane_colors,
      dot_color,
    }
  }

  fn lane_color_for(lane_ix: usize, theme: &gpui_component::Theme) -> gpui::Hsla {
    let palette = [
      theme.status_blue().opacity(0.95),
      theme.status_yellow().opacity(0.95),
      theme.status_green().opacity(0.95),
      theme.status_red().opacity(0.95),
      theme.status_violet().opacity(0.92),
      theme.status_orange().opacity(0.90),
    ];
    palette[lane_ix % palette.len()]
  }

  fn lane_color(&self, lane_ix: usize) -> gpui::Hsla {
    self
      .lane_colors
      .get(lane_ix)
      .cloned()
      .unwrap_or(self.dot_color)
  }

  fn lane_center_x(lane_ix: usize, bounds: Bounds<Pixels>) -> Pixels {
    bounds.left() + px(lane_ix as f32 * GRAPH_LANE_WIDTH + (GRAPH_LANE_WIDTH / 2.0))
  }

  fn lane_center_x_for(&self, lane_ix: usize, bounds: Bounds<Pixels>) -> Pixels {
    Self::lane_center_x(lane_ix, bounds)
  }
}

impl IntoElement for GraphLanesElement {
  type Element = Self;

  fn into_element(self) -> Self::Element {
    self
  }
}

impl Element for GraphLanesElement {
  type RequestLayoutState = ();
  type PrepaintState = GraphLanesPrepaintState;

  fn id(&self) -> Option<ElementId> {
    Some(self.id.clone())
  }

  fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
    None
  }

  fn request_layout(
    &mut self,
    _id: Option<&GlobalElementId>,
    _inspector_id: Option<&InspectorElementId>,
    window: &mut Window,
    cx: &mut App,
  ) -> (LayoutId, Self::RequestLayoutState) {
    let lane_count = self.segments.len().max(1);
    let mut style = Style::default();
    style.size.width = px(lane_count as f32 * GRAPH_LANE_WIDTH).into();
    style.size.height = px(GRAPH_ROW_HEIGHT).into();
    (window.request_layout(style, [], cx), ())
  }

  fn prepaint(
    &mut self,
    _id: Option<&GlobalElementId>,
    _inspector_id: Option<&InspectorElementId>,
    bounds: Bounds<Pixels>,
    _request_layout: &mut Self::RequestLayoutState,
    _window: &mut Window,
    _cx: &mut App,
  ) -> Self::PrepaintState {
    let mut paths = Vec::with_capacity(self.segments.len() + self.merge_parent_lanes.len() + 1);
    let middle_y = bounds.top() + bounds.size.height / 2.0;

    for (lane_ix, segment) in self.segments.iter().enumerate() {
      if !segment.up && !segment.down {
        continue;
      }
      if lane_ix != self.commit_lane && self.merge_parent_lanes.contains(&lane_ix) && !segment.up {
        // Merge parent lane will be represented by the merge curve itself.
        continue;
      }

      let lane_has_up = if lane_ix == self.commit_lane {
        segment.up && self.commit_lane_has_up
      } else {
        segment.up
      };
      let top_y = if lane_has_up { bounds.top() } else { middle_y };
      let bottom_y = if segment.down {
        bounds.bottom()
      } else {
        middle_y
      };
      if bottom_y <= top_y {
        continue;
      }

      let mut builder = PathBuilder::stroke(px(GRAPH_LINE_WIDTH));
      let lane_x = self.lane_center_x_for(lane_ix, bounds);
      builder.move_to(point(lane_x, top_y));
      builder.line_to(point(lane_x, bottom_y));

      if let Ok(path) = builder.build() {
        paths.push(GraphPaintPath {
          path,
          color: self.lane_color(lane_ix),
        });
      }
    }

    let commit_x = self.lane_center_x_for(self.commit_lane, bounds);
    for parent_lane in self.merge_parent_lanes.iter().copied() {
      if parent_lane == self.commit_lane {
        continue;
      }

      let from_x: f32 = self.lane_center_x_for(parent_lane, bounds).into();
      let to_x: f32 = commit_x.into();
      let dx = to_x - from_x;
      if dx.abs() < 0.1 {
        continue;
      }

      let mut builder = PathBuilder::stroke(px(GRAPH_LINE_WIDTH));
      let start_y: f32 = bounds.bottom().into();
      let end_y: f32 = middle_y.into();
      let horizontal_gap = dx.abs();
      let vertical_span = (start_y - end_y).abs();
      let depth =
        (vertical_span * 0.85 + horizontal_gap * 0.10).clamp(4.0, GRAPH_ROW_HEIGHT * 0.65);
      let ctrl_a_x = from_x;
      let ctrl_b_x = to_x - dx * 0.22;
      let ctrl_a_y = start_y - depth;
      let ctrl_b_y = end_y;
      let right_lane = parent_lane.max(self.commit_lane);

      builder.move_to(point(px(from_x), px(start_y)));
      builder.cubic_bezier_to(
        point(px(to_x), px(end_y)),
        point(px(ctrl_a_x), px(ctrl_a_y)),
        point(px(ctrl_b_x), px(ctrl_b_y)),
      );

      if let Ok(path) = builder.build() {
        paths.push(GraphPaintPath {
          path,
          color: self.lane_color(right_lane),
        });
      }
    }

    let split_stub_len = bounds.size.height / 2.0;
    for lane in self.branch_pre_stubs.iter().copied() {
      let lane_x = self.lane_center_x_for(lane, bounds);
      let mut stub_builder = PathBuilder::stroke(px(GRAPH_LINE_WIDTH));
      let stub_start = bounds.bottom() - split_stub_len;
      stub_builder.move_to(point(lane_x, stub_start));
      stub_builder.line_to(point(lane_x, bounds.bottom()));
      if let Ok(path) = stub_builder.build() {
        paths.push(GraphPaintPath {
          path,
          color: self.lane_color(lane),
        });
      }
    }

    for child_lane in self
      .branch_child_lanes
      .iter()
      .copied()
      .filter(|lane| *lane != self.commit_lane)
    {
      let from_x: f32 = commit_x.into();
      let to_x: f32 = self.lane_center_x_for(child_lane, bounds).into();
      let dx = to_x - from_x;
      if dx.abs() < 0.1 {
        continue;
      }

      // Branch creation is the inverse of merge: start from this commit (middle)
      // and connect to the upper part of the child lane on the right.
      let target_y = bounds.top();

      let mut builder = PathBuilder::stroke(px(GRAPH_LINE_WIDTH));
      let start_y: f32 = middle_y.into();
      let end_y: f32 = target_y.into();
      let horizontal_gap = dx.abs();
      let vertical_span = (start_y - end_y).abs();
      let depth =
        (vertical_span * 0.85 + horizontal_gap * 0.10).clamp(4.0, GRAPH_ROW_HEIGHT * 0.65);
      // Mirror of merge geometry:
      // - start tangent mostly horizontal to the right
      // - end tangent vertical upward into the right lane
      let ctrl_a_x = from_x + dx * 0.22;
      let ctrl_b_x = to_x;
      let ctrl_a_y = start_y;
      let ctrl_b_y = end_y + depth;

      builder.move_to(point(px(from_x), px(start_y)));
      builder.cubic_bezier_to(
        point(px(to_x), target_y),
        point(px(ctrl_a_x), px(ctrl_a_y)),
        point(px(ctrl_b_x), px(ctrl_b_y)),
      );

      if let Ok(path) = builder.build() {
        paths.push(GraphPaintPath {
          path,
          color: self.lane_color(child_lane),
        });
      }
    }

    let dot_center = point(commit_x, middle_y);
    let dot_diameter = if self.is_latest_commit {
      GRAPH_DOT_SIZE_LATEST
    } else {
      GRAPH_DOT_SIZE
    };
    let dot_radius = px(dot_diameter / 2.0);
    let dot_bounds = Bounds::new(
      point(dot_center.x - dot_radius, dot_center.y - dot_radius),
      size(px(dot_diameter), px(dot_diameter)),
    );
    let dot_background = if self.is_latest_commit {
      self.latest_dot_background
    } else {
      self.dot_color
    };
    let mut dot = fill(dot_bounds, dot_background).corner_radii(dot_radius);
    if self.is_latest_commit {
      dot = dot
        .border_widths(px(GRAPH_LINE_WIDTH))
        .border_color(self.dot_color);
    }

    GraphLanesPrepaintState { paths, dot }
  }

  fn paint(
    &mut self,
    _id: Option<&GlobalElementId>,
    _inspector_id: Option<&InspectorElementId>,
    _bounds: Bounds<Pixels>,
    _request_layout: &mut Self::RequestLayoutState,
    prepaint: &mut Self::PrepaintState,
    window: &mut Window,
    _cx: &mut App,
  ) {
    for path in &prepaint.paths {
      window.paint_path(path.path.clone(), path.color);
    }
    window.paint_quad(prepaint.dot.clone());
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
  graph_list: Entity<ListState<GitGraphListDelegate>>,
  window_handle: AnyWindowHandle,
  selected_repo: Option<PathBuf>,
  status_entries: Vec<RepoStatusEntry>,
  branch_status: Option<BranchStatus>,
  has_head_commit: bool,
  can_undo_last_commit: bool,
  can_push: bool,
  can_force_push: bool,
  has_staged_changes: bool,
  sidebar_mode: GitSidebarMode,
  graph_commits: Vec<CommitGraphNode>,
  graph_loading: bool,
  selected_file: Option<PathBuf>,
  force_list_selection: bool,
  editor: Option<Entity<Editor>>,
  diff_view: DiffViewMode,
  show_markdown_preview: bool,
  markdown_preview_state: MarkdownRenderState,
  svg_preview: Option<Result<Arc<RenderImage>, SharedString>>,
  svg_preview_source: Option<SharedString>,
  svg_preview_task: Option<Task<()>>,
  auth_state: AuthState,
  auth_task: Option<Task<()>>,
  status_task: Option<Task<()>>,
  graph_task: Option<Task<()>>,
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
      .map(|path| Self::is_markdown_path(path))
      .unwrap_or(false)
  }

  fn selected_file_is_svg(&self) -> bool {
    self
      .selected_file
      .as_ref()
      .map(|path| Self::is_svg_path(path))
      .unwrap_or(false)
  }

  fn effective_diff_view_for_path(&self, path: &Path) -> DiffViewMode {
    if self.show_markdown_preview && (Self::is_markdown_path(path) || Self::is_svg_path(path)) {
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

  fn refresh_graph_list(&mut self, cx: &mut Context<Self>) {
    let rows = Self::build_graph_rows(&self.graph_commits);
    self.graph_list.update(cx, move |state, cx| {
      state.delegate_mut().set_rows(rows);
      cx.notify();
    });
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
    let graph_list =
      cx.new(|cx| ListState::new(GitGraphListDelegate::new(), window, cx).selectable(false));

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
      graph_list,
      window_handle: window.window_handle(),
      selected_repo,
      status_entries: Vec::new(),
      branch_status: None,
      has_head_commit: false,
      can_undo_last_commit: false,
      can_push: false,
      can_force_push: false,
      has_staged_changes: false,
      sidebar_mode: GitSidebarMode::Changes,
      graph_commits: Vec::new(),
      graph_loading: false,
      selected_file: None,
      force_list_selection: false,
      editor: None,
      diff_view: DiffViewMode::Inline,
      show_markdown_preview: false,
      markdown_preview_state: MarkdownRenderState::new(),
      svg_preview: None,
      svg_preview_source: None,
      svg_preview_task: None,
      auth_state: AuthState::Unknown,
      auth_task: None,
      status_task: None,
      graph_task: None,
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
    self.graph_commits.clear();
    self.graph_loading = self.sidebar_mode == GitSidebarMode::Graph;
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

  fn refresh_graph(&mut self, cx: &mut Context<Self>) {
    let Some(repo_root) = self.selected_repo.clone() else {
      self.graph_commits.clear();
      self.graph_loading = false;
      self.refresh_graph_list(cx);
      cx.notify();
      return;
    };

    if self.graph_commits.is_empty() {
      self.graph_loading = true;
      cx.notify();
    }

    let task = cx.spawn(async move |this, cx| {
      let requested_repo = repo_root.clone();
      let graph = unblock(move || list_commit_graph(&repo_root, GRAPH_MAX_COMMITS)).await;
      let _ = this.update(cx, |this, cx| {
        if this.selected_repo.as_ref() != Some(&requested_repo) {
          return;
        }
        this.graph_commits = graph.unwrap_or_default();
        this.graph_loading = false;
        this.refresh_graph_list(cx);
        cx.notify();
      });
    });

    self.graph_task = Some(task);
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
      self.graph_commits.clear();
      self.graph_loading = false;
      self.refresh_graph_list(cx);
      cx.notify();
      return;
    };
    let include_graph = self.sidebar_mode == GitSidebarMode::Graph;
    if include_graph && self.graph_commits.is_empty() {
      self.graph_loading = true;
      cx.notify();
    }

    let task = cx.spawn(async move |this, cx| {
      let requested_repo = repo_root.clone();
      let status = unblock(move || {
        let entries = list_repo_status(&repo_root).ok()?;
        let branch = current_branch_status(&repo_root).ok();
        let head_status = head_commit_status(&repo_root).ok();
        let graph = if include_graph {
          list_commit_graph(&repo_root, GRAPH_MAX_COMMITS).ok()
        } else {
          None
        };
        Some((entries, branch, head_status, graph))
      })
      .await;
      let Some((entries, branch_status, head_status, graph)) = status else {
        let _ = this.update(cx, |this, cx| {
          if this.selected_repo.as_ref() != Some(&requested_repo) {
            return;
          }
          if include_graph && this.graph_loading {
            this.graph_loading = false;
            cx.notify();
          }
        });
        return;
      };

      let _ = this.update(cx, |this, cx| {
        if this.selected_repo.as_ref() != Some(&requested_repo) {
          return;
        }
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
        if include_graph {
          this.graph_commits = graph.unwrap_or_default();
          this.graph_loading = false;
          this.refresh_graph_list(cx);
        }
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
      let mut graph_poll_tick: u32 = 0;
      loop {
        cx.background_executor()
          .timer(Duration::from_millis(STATUS_POLL_INTERVAL_MS))
          .await;

        let repo_root = match this.update(cx, |this, _| this.selected_repo.clone()) {
          Ok(value) => value,
          Err(_) => return,
        };
        let include_graph =
          match this.update(cx, |this, _| this.sidebar_mode == GitSidebarMode::Graph) {
            Ok(value) => value,
            Err(_) => return,
          };
        if include_graph {
          graph_poll_tick = graph_poll_tick.saturating_add(1);
        } else {
          graph_poll_tick = 0;
        }
        let poll_graph_now = include_graph && graph_poll_tick >= GRAPH_POLL_EVERY_TICKS;
        if poll_graph_now {
          graph_poll_tick = 0;
        }
        let Some(repo_root) = repo_root else {
          continue;
        };
        let requested_repo = repo_root.clone();

        let status = unblock(move || {
          let entries = list_repo_status(&repo_root).ok()?;
          let branch = current_branch_status(&repo_root).ok();
          let head_status = head_commit_status(&repo_root).ok();
          let graph = if poll_graph_now {
            list_commit_graph(&repo_root, GRAPH_MAX_COMMITS).ok()
          } else {
            None
          };
          Some((entries, branch, head_status, graph))
        })
        .await;
        let Some((entries, branch_status, head_status, graph)) = status else {
          continue;
        };

        let _ = this.update(cx, |this, cx| {
          if this.selected_repo.as_ref() != Some(&requested_repo) {
            return;
          }
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
          if poll_graph_now {
            this.graph_commits = graph.unwrap_or_default();
            this.graph_loading = false;
            this.refresh_graph_list(cx);
          }
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
          if this.sidebar_mode == GitSidebarMode::Changes {
            this.refresh_file_list(cx);
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

  fn open_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let mut palette_branches = Vec::new();

    let view = cx.entity();
    let handler: CommandPaletteHandler = Arc::new(move |action, _window, cx| {
      view.update(cx, |view, cx| {
        view.handle_command_palette_action(action, cx)
      })
    });

    let include_github = matches!(self.auth_state, AuthState::Authenticated(_));
    let mut commands =
      CommandPaletteCommand::default_global_commands(CommandPalettePage::Git, include_github);

    if let Some(root_path) = self.selected_repo.clone()
      && let Ok(branches) = list_branches(&root_path)
    {
      palette_branches = branches
        .into_iter()
        .map(|branch| CommandPaletteBranch {
          name: branch.name.into(),
          kind: match branch.kind {
            BranchKind::Local => CommandPaletteBranchKind::Local,
            BranchKind::Remote => CommandPaletteBranchKind::Remote,
          },
        })
        .collect::<Vec<_>>();
      commands.push(CommandPaletteCommand::switch_branch());
      commands.push(CommandPaletteCommand::merge_branch());
    }

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
      CommandPaletteAction::OpenGithubPrDetails {
        owner,
        repo,
        number,
      } => {
        GithubPrDetailsPageHandle::show(owner.into(), repo.into(), number, cx);
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
        selected_branch = Some(branch_ref.clone());
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
        selected_branch = Some(branch_ref.clone());
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
        selected_branch = Some(new_branch.clone());
        create_branch_from(&root_path, &name, &branch_ref)
          .and_then(|_| switch_branch(&root_path, &new_branch))
      }
      CommandPaletteAction::MergeBranch { name } => {
        let Some(root_path) = self.selected_repo.clone() else {
          return Err("No repository selected.".into());
        };
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
    if !is_markdown && !Self::is_svg_path(&rel_path) {
      self.show_markdown_preview = false;
    }
    let file_path = repo_root.join(&rel_path);
    let editor = cx.new(|cx| Editor::new_with_paths(repo_root, file_path, cx));
    let diff_view = self.effective_diff_view_for_path(&rel_path);
    editor.update(cx, |editor, cx| editor.set_diff_view_mode(diff_view, cx));
    self.editor = Some(editor);
    self.selected_file = Some(rel_path);
    self.svg_preview = None;
    self.svg_preview_source = None;
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
    if !self.selected_file_is_markdown() && !self.selected_file_is_svg() {
      self.show_markdown_preview = false;
      self.sync_diff_view(cx);
      cx.notify();
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

  fn toggle_sidebar_mode_action(
    &mut self,
    _: &gpui::ClickEvent,
    _: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.sidebar_mode = match self.sidebar_mode {
      GitSidebarMode::Changes => GitSidebarMode::Graph,
      GitSidebarMode::Graph => GitSidebarMode::Changes,
    };

    if self.sidebar_mode == GitSidebarMode::Graph {
      self.refresh_graph(cx);
    } else {
      self.refresh_file_list(cx);
      cx.notify();
    }
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
      RepoStatusKind::Modified => theme.status_yellow(),
      RepoStatusKind::Added => theme.status_green(),
      RepoStatusKind::Deleted => theme.status_red(),
      RepoStatusKind::Renamed => theme.status_blue(),
      RepoStatusKind::TypeChange => theme.status_blue(),
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

  fn build_graph_rows(commits: &[CommitGraphNode]) -> Vec<GraphRenderRow> {
    let commits_by_oid = commits
      .iter()
      .map(|commit| (commit.oid.as_str(), commit))
      .collect::<HashMap<_, _>>();
    let mut mainline_oids = HashSet::new();
    if let Some(head) = commits.first() {
      let mut cursor = Some(head.oid.as_str());
      while let Some(oid) = cursor {
        if !mainline_oids.insert(oid.to_string()) {
          break;
        }
        cursor = commits_by_oid
          .get(oid)
          .and_then(|commit| commit.parent_oids.first())
          .map(String::as_str);
      }
    }

    let mut rows = Vec::with_capacity(commits.len());
    let mut active_lanes: Vec<Option<String>> = Vec::new();

    let trim_trailing_empty = |lanes: &mut Vec<Option<String>>| {
      while lanes.last().map(|lane| lane.is_none()).unwrap_or(false) {
        lanes.pop();
      }
    };

    for commit in commits {
      let is_mainline_commit = mainline_oids.contains(commit.oid.as_str());
      let mut commit_lane_has_up = true;
      let mut commit_lane = if let Some(ix) = active_lanes
        .iter()
        .position(|oid| oid.as_deref() == Some(commit.oid.as_str()))
      {
        ix
      } else {
        commit_lane_has_up = false;
        active_lanes.push(Some(commit.oid.clone()));
        active_lanes.len() - 1
      };
      if is_mainline_commit && commit_lane != 0 {
        active_lanes.swap(0, commit_lane);
        commit_lane = 0;
      }

      let lanes_before = active_lanes.clone();
      let mut lanes_after = lanes_before.clone();
      let mut parent_lanes = Vec::new();
      let mut first_parent_lane = None;
      lanes_after[commit_lane] = None;
      let mut lock_commit_lane_empty = false;

      if let Some(first_parent) = commit.parent_oids.first() {
        if let Some(existing_idx) = lanes_after
          .iter()
          .position(|oid| oid.as_deref() == Some(first_parent.as_str()))
        {
          if existing_idx == commit_lane || is_mainline_commit {
            lanes_after[commit_lane] = Some(first_parent.clone());
            if existing_idx != commit_lane {
              lanes_after[existing_idx] = None;
            }
            if !parent_lanes.contains(&commit_lane) {
              parent_lanes.push(commit_lane);
            }
            first_parent_lane = Some(commit_lane);
          } else {
            if !parent_lanes.contains(&existing_idx) {
              parent_lanes.push(existing_idx);
            }
            first_parent_lane = Some(existing_idx);
            if existing_idx != commit_lane {
              lock_commit_lane_empty = true;
            }
          }
        } else {
          lanes_after[commit_lane] = Some(first_parent.clone());
          parent_lanes.push(commit_lane);
          first_parent_lane = Some(commit_lane);
        }
      } else {
        lock_commit_lane_empty = true;
      }

      let mut preferred_lane = commit_lane + 1;
      for parent in commit.parent_oids.iter().skip(1) {
        if let Some(existing_idx) = lanes_after
          .iter()
          .position(|oid| oid.as_deref() == Some(parent.as_str()))
        {
          if !parent_lanes.contains(&existing_idx) {
            parent_lanes.push(existing_idx);
          }
          preferred_lane = existing_idx + 1;
          continue;
        }

        let insert_at = lanes_after
          .iter()
          .enumerate()
          .skip(preferred_lane)
          .find_map(|(ix, lane)| {
            if lane.is_none() && (!lock_commit_lane_empty || ix != commit_lane) {
              Some(ix)
            } else {
              None
            }
          })
          .unwrap_or(lanes_after.len());

        if insert_at == lanes_after.len() {
          lanes_after.push(Some(parent.clone()));
        } else {
          lanes_after[insert_at] = Some(parent.clone());
        }
        parent_lanes.push(insert_at);
        preferred_lane = insert_at + 1;
      }

      trim_trailing_empty(&mut lanes_after);

      let lane_count = lanes_before
        .len()
        .max(lanes_after.len())
        .max(commit_lane + 1);

      let segments = (0..lane_count)
        .map(|lane| GraphLaneSegment {
          up: lanes_before
            .get(lane)
            .and_then(|lane| lane.as_ref())
            .is_some(),
          down: lanes_after
            .get(lane)
            .and_then(|lane| lane.as_ref())
            .is_some(),
        })
        .collect::<Vec<_>>();

      let merge_parent_lanes = if commit.parent_oids.len() > 1 {
        parent_lanes
          .iter()
          .copied()
          .filter(|lane| Some(*lane) != first_parent_lane)
          .collect::<Vec<_>>()
      } else {
        Vec::new()
      };
      rows.push(GraphRenderRow {
        commit: commit.clone(),
        segments,
        commit_lane,
        merge_parent_lanes,
        branch_child_lanes: Vec::new(),
        branch_pre_stubs: Vec::new(),
        commit_lane_has_up,
      });
      active_lanes = lanes_after;
      trim_trailing_empty(&mut active_lanes);
    }

    let max_lane_count = rows.iter().map(|row| row.segments.len()).max().unwrap_or(0);
    if max_lane_count > 0 {
      for row in &mut rows {
        row
          .segments
          .resize(max_lane_count, GraphLaneSegment::default());
      }
    }

    let row_index_by_oid = rows
      .iter()
      .enumerate()
      .map(|(index, row)| (row.commit.oid.clone(), index))
      .collect::<HashMap<_, _>>();
    let mut branch_child_lanes_by_row = HashMap::<usize, Vec<usize>>::new();
    let mut branch_pre_stubs_by_row = HashMap::<usize, Vec<usize>>::new();

    for child_row in &rows {
      if child_row.commit.parent_oids.len() != 1 {
        continue;
      }
      let Some(parent_row_index) = child_row
        .commit
        .parent_oids
        .first()
        .and_then(|parent_oid| row_index_by_oid.get(parent_oid))
        .copied()
      else {
        continue;
      };

      let parent_lane = rows[parent_row_index].commit_lane;
      let child_lane = child_row.commit_lane;
      if child_lane > parent_lane {
        branch_child_lanes_by_row
          .entry(parent_row_index)
          .or_default()
          .push(child_lane);

        let parent_has_up = rows[parent_row_index]
          .segments
          .get(child_lane)
          .map(|segment| segment.up)
          .unwrap_or(false);
        if !parent_has_up && parent_row_index > 0 {
          branch_pre_stubs_by_row
            .entry(parent_row_index - 1)
            .or_default()
            .push(child_lane);
        }
      }
    }

    for (row_index, mut lanes) in branch_child_lanes_by_row {
      lanes.sort_unstable();
      lanes.dedup();
      if let Some(row) = rows.get_mut(row_index) {
        row.branch_child_lanes = lanes;
      }
    }

    for (row_index, mut lanes) in branch_pre_stubs_by_row {
      lanes.sort_unstable();
      lanes.dedup();
      if let Some(row) = rows.get_mut(row_index) {
        row.branch_pre_stubs = lanes;
      }
    }

    rows
  }

  fn primary_branch_label(refs: &[String]) -> Option<(String, bool)> {
    if let Some(head_ref) = refs.iter().find(|label| label.starts_with("HEAD")) {
      let label = head_ref
        .strip_prefix("HEAD -> ")
        .filter(|name| !name.is_empty())
        .unwrap_or(head_ref.as_str())
        .to_string();
      return Some((label, true));
    }

    refs.first().cloned().map(|label| (label, false))
  }

  fn render_graph_row(
    row_index: usize,
    row: &GraphRenderRow,
    theme: &gpui_component::Theme,
  ) -> AnyElement {
    let lane_count = row.segments.len().max(1);
    let graph_width = lane_count as f32 * GRAPH_LANE_WIDTH;

    let summary = if row.commit.summary.trim().is_empty() {
      "No commit message".to_string()
    } else {
      row.commit.summary.clone()
    };
    let branch_badge = Self::primary_branch_label(&row.commit.refs).map(|(label, is_head)| {
      let bg_color = if is_head {
        theme.accent
      } else {
        theme.title_bar_border.opacity(0.9)
      };
      let text_color = if is_head {
        theme.accent_foreground
      } else {
        theme.sidebar_foreground
      };
      div()
        .id(format!("git-graph-branch-badge-{row_index}"))
        .px_1()
        .py(px(1.))
        .rounded(px(4.))
        .max_w(px(GRAPH_BRANCH_BADGE_MAX_WIDTH))
        .overflow_hidden()
        .text_ellipsis()
        .text_xs()
        .bg(bg_color)
        .text_color(text_color)
        .child(label)
        .into_any_element()
    });

    div()
      .id(format!("git-graph-row-{row_index}"))
      .w_full()
      .h(px(GRAPH_ROW_HEIGHT))
      .min_h(px(GRAPH_ROW_HEIGHT))
      .px_2()
      .flex()
      .items_center()
      .gap_2()
      .child(
        div()
          .id(format!("git-graph-canvas-{row_index}"))
          .relative()
          .h(px(GRAPH_ROW_HEIGHT))
          .w(px(graph_width))
          .flex_shrink_0()
          .child(GraphLanesElement::new(row_index, row, theme)),
      )
      .child(
        div()
          .min_w_0()
          .flex_1()
          .flex()
          .items_center()
          .gap_2()
          .overflow_hidden()
          .child(
            div()
              .min_w_0()
              .flex_1()
              .overflow_hidden()
              .text_ellipsis()
              .text_xs()
              .child(summary),
          )
          .child(
            div()
              .min_w_0()
              .max_w(px(GRAPH_AUTHOR_MAX_WIDTH))
              .overflow_hidden()
              .text_ellipsis()
              .text_xs()
              .text_color(theme.muted_foreground)
              .child(row.commit.author.clone()),
          ),
      )
      .when_some(branch_badge, |this, badge| this.child(badge))
      .into_any_element()
  }

  fn render_graph_sidebar_content(&self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    // self.graph_loading && self.graph_commits.is_empty()
    if self.graph_loading {
      return div()
        .id("git-graph-loading-container")
        .flex()
        .flex_col()
        .size_full()
        .items_center()
        .justify_center()
        .child(
          div()
            .id("git-graph-loading-content")
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .child(Spinner::new().small())
            .child(
              div()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child("Loading graph..."),
            ),
        )
        .into_any_element();
    }

    if self.graph_commits.is_empty() {
      return div()
        .id("git-graph-empty-container")
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

    div()
      .id("git-graph-scroll-container")
      .flex_1()
      .min_h_0()
      .child(
        div()
          .id("git-graph-scroll")
          .size_full()
          .overflow_y_scrollbar()
          .child(
            List::new(&self.graph_list)
              .w_full()
              .min_h_0()
              .h_full()
              .p_0(),
          ),
      )
      .into_any_element()
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
    let open_git_config = Rc::new(|_window: &mut Window, cx: &mut App| {
      let cx = &mut *cx;
      WorkspaceRoute::open_git_config(cx);
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
      on_open_git_config: Some(open_git_config),
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
      .px_2()
      .bg(theme.background)
      .items_center()
      .justify_center()
      .text_color(cx.theme().muted_foreground)
      .child(div().truncate().child(message))
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
          .when(is_markdown || is_svg, |this| this.child(preview_button))
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

    let line_height = window.line_height();
    let anchor_display_line = editor_state
      .first_display_line_for_group(hovered_id)
      .unwrap_or(overlay.display_line);
    if editor_state.find_panel_occludes_display_line(anchor_display_line) {
      return None;
    }
    let mut top = line_height * (anchor_display_line as f32 - editor_state.scroll_offset_y);
    if top >= editor_state.viewport_height {
      return None;
    }
    if top < px(0.0) {
      top = px(0.0);
    }
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
    let changed_files_count = self.status_entries.len();
    let (label, icon, tooltip) = if all_staged {
      ("Unstage all", IconName::Minus, "Unstage all files")
    } else {
      ("Stage all", IconName::Plus, "Stage all files")
    };
    let is_graph_mode = self.sidebar_mode == GitSidebarMode::Graph;
    let (mode_label, mode_icon, mode_tooltip) = if is_graph_mode {
      ("Changes", IconName::File, "Show changes list")
    } else {
      ("Graph", IconName::ArrowUp, "Show commit graph")
    };

    let group_label = if is_graph_mode {
      div()
        .text_sm()
        .text_color(theme.sidebar_foreground)
        .child("Graph")
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
        .when(changed_files_count > 0, |this| {
          this.child(
            div()
              .px_1()
              .py(px(1.))
              .rounded(theme.radius)
              .text_xs()
              .bg(theme.status_red().opacity(0.40))
              .text_color(theme.status_red())
              .child(changed_files_count.to_string()),
          )
        })
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
          .when(!is_graph_mode, |this| {
            this.child(
              Button::new("stage-all-button")
                .label(label)
                .icon(icon)
                .with_variant(ButtonVariant::Secondary)
                .xsmall()
                .disabled(!sidebar_enabled)
                .tooltip(tooltip)
                .on_click(cx.listener(Self::toggle_stage_all_action)),
            )
          })
          .child(
            Button::new("sidebar-mode-toggle-button")
              .label(mode_label)
              .icon(mode_icon)
              .with_variant(ButtonVariant::Secondary)
              .xsmall()
              .selected(is_graph_mode)
              .disabled(self.selected_repo.is_none())
              .tooltip(mode_tooltip)
              .on_click(cx.listener(Self::toggle_sidebar_mode_action)),
          ),
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

    if self.sidebar_mode == GitSidebarMode::Graph {
      return base_sidebar
        .relative()
        .child(self.render_sidebar_header(cx))
        .child(self.render_graph_sidebar_content(cx))
        .into_any_element();
    }

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
            .child(div().size_full().pb_4().px_4().child(render_markdown(
              &markdown,
              &MarkdownRenderOptions::default().with_state(self.markdown_preview_state.clone()),
              cx,
            )))
            .into_any_element()
        };

        return div()
          .size_full()
          .flex()
          .flex_col()
          .child(self.render_editor_header(&editor, cx))
          .child(
            ui::h_resizable("git-page-markdown-preview")
              .child(ui::resizable_panel().child(editor_view))
              .child(ui::resizable_panel().child(preview_content)),
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
      .on_action(cx.listener(GitPage::find_action))
      .on_action(cx.listener(GitPage::close_find_action))
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

#[cfg(test)]
mod tests {
  use super::*;

  fn make_commit(oid: &str, parents: &[&str]) -> CommitGraphNode {
    CommitGraphNode {
      oid: oid.to_string(),
      short_oid: oid.chars().take(7).collect(),
      summary: format!("commit-{oid}"),
      author: "author".to_string(),
      parent_oids: parents.iter().map(|parent| parent.to_string()).collect(),
      refs: Vec::new(),
    }
  }

  fn make_commit_with_refs(oid: &str, parents: &[&str], refs: &[&str]) -> CommitGraphNode {
    CommitGraphNode {
      oid: oid.to_string(),
      short_oid: oid.chars().take(7).collect(),
      summary: format!("commit-{oid}"),
      author: "author".to_string(),
      parent_oids: parents.iter().map(|parent| parent.to_string()).collect(),
      refs: refs.iter().map(|label| label.to_string()).collect(),
    }
  }

  #[test]
  fn build_graph_rows_linear_history_stays_on_single_lane() {
    let commits = vec![
      make_commit("c3", &["c2"]),
      make_commit("c2", &["c1"]),
      make_commit("c1", &[]),
    ];

    let rows = GitPage::build_graph_rows(&commits);
    assert_eq!(rows.len(), 3);

    assert_eq!(rows[0].commit_lane, 0);
    assert!(rows[0].branch_child_lanes.is_empty());
    assert_eq!(rows[0].segments.len(), 1);
    assert!(rows[0].segments[0].up);
    assert!(rows[0].segments[0].down);

    assert_eq!(rows[2].commit_lane, 0);
    assert!(rows[2].segments[0].up);
    assert!(!rows[2].segments[0].down);
  }

  #[test]
  fn build_graph_rows_merge_keeps_cross_lane_connection() {
    let commits = vec![
      make_commit("m", &["a", "b"]),
      make_commit("a", &["p"]),
      make_commit("b", &["p"]),
      make_commit("p", &[]),
    ];

    let rows = GitPage::build_graph_rows(&commits);
    assert_eq!(rows.len(), 4);

    assert_eq!(rows[0].commit_lane, 0);
    assert_eq!(rows[0].segments.len(), 2);
    assert_eq!(rows[0].merge_parent_lanes, vec![1]);

    assert_eq!(rows[2].commit_lane, 1);
    assert!(rows[2].branch_child_lanes.is_empty());
  }

  #[test]
  fn build_graph_rows_new_lane_starts_without_up_segment() {
    let commits = vec![
      make_commit("a", &["p"]),
      make_commit("x", &["y"]),
      make_commit("p", &[]),
      make_commit("y", &[]),
    ];

    let rows = GitPage::build_graph_rows(&commits);
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[1].commit_lane, 1);
    assert!(!rows[1].commit_lane_has_up);
    assert!(rows[1].segments[1].up);
    assert!(rows[1].segments[1].down);
  }

  #[test]
  fn build_graph_rows_keeps_lane_order_stable_when_gap_exists() {
    let commits = vec![
      make_commit("m", &["a", "b", "c"]),
      make_commit("a", &["p"]),
      make_commit("b", &["p"]),
      make_commit("d", &["q"]),
      make_commit("c", &["p"]),
      make_commit("p", &[]),
      make_commit("q", &[]),
    ];

    let rows = GitPage::build_graph_rows(&commits);
    assert_eq!(rows.len(), 7);

    assert_eq!(rows[2].commit_lane, 1);
    assert_eq!(rows[3].commit_lane, 3);
    assert!(!rows[3].commit_lane_has_up);
    assert_eq!(rows[4].commit_lane, 2);
  }

  #[test]
  fn build_graph_rows_is_deterministic_for_same_input() {
    let commits = vec![
      make_commit("m", &["a", "b"]),
      make_commit("a", &["p"]),
      make_commit("b", &["p"]),
      make_commit("p", &[]),
    ];

    let rows_a = GitPage::build_graph_rows(&commits);
    let rows_b = GitPage::build_graph_rows(&commits);

    let lanes_a = rows_a
      .iter()
      .map(|row| {
        let mut merge_parent_lanes = row.merge_parent_lanes.clone();
        merge_parent_lanes.sort_unstable();
        let mut branch_child_lanes = row.branch_child_lanes.clone();
        branch_child_lanes.sort_unstable();
        (
          row.commit_lane,
          merge_parent_lanes,
          branch_child_lanes,
          row.commit_lane_has_up,
        )
      })
      .collect::<Vec<_>>();
    let lanes_b = rows_b
      .iter()
      .map(|row| {
        let mut merge_parent_lanes = row.merge_parent_lanes.clone();
        merge_parent_lanes.sort_unstable();
        let mut branch_child_lanes = row.branch_child_lanes.clone();
        branch_child_lanes.sort_unstable();
        (
          row.commit_lane,
          merge_parent_lanes,
          branch_child_lanes,
          row.commit_lane_has_up,
        )
      })
      .collect::<Vec<_>>();

    assert_eq!(lanes_a, lanes_b);
  }

  #[test]
  fn build_graph_rows_prefers_head_first_parent_chain_as_anchor_lane() {
    let commits = vec![
      make_commit("merge", &["f2", "m2"]),
      make_commit("f2", &["f1"]),
      make_commit_with_refs("m2", &["m1"], &["origin/trunk"]),
      make_commit("f1", &["base"]),
      make_commit("m1", &["base"]),
      make_commit("base", &[]),
    ];

    let rows = GitPage::build_graph_rows(&commits);
    let lane_by_oid = rows
      .iter()
      .map(|row| (row.commit.oid.as_str(), row.commit_lane))
      .collect::<std::collections::HashMap<_, _>>();

    assert_eq!(lane_by_oid.get("f2"), Some(&0));
    assert_eq!(lane_by_oid.get("f1"), Some(&0));
  }

  #[test]
  fn build_graph_rows_marks_branch_creation_from_left_parent_to_right_lane() {
    let commits = vec![
      make_commit("tip", &["left"]),
      make_commit("right", &["left"]),
      make_commit("left", &[]),
    ];

    let rows = GitPage::build_graph_rows(&commits);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[2].commit_lane, 0);
    assert_eq!(rows[2].branch_child_lanes, vec![1]);
  }

  #[test]
  fn build_graph_rows_branch_creation_adds_pre_stub_on_previous_row() {
    let commits = vec![
      make_commit("tip", &["left"]),
      make_commit("right", &["left"]),
      make_commit("left", &[]),
    ];

    let rows = GitPage::build_graph_rows(&commits);
    assert_eq!(rows.len(), 3);
    // Split curve metadata lives on parent row.
    assert_eq!(rows[2].branch_child_lanes, vec![1]);
    // The pre-stub is painted on the row just before that parent row.
    assert_eq!(rows[1].branch_pre_stubs, vec![1]);
    assert!(rows[2].branch_pre_stubs.is_empty());
  }

  #[test]
  fn build_graph_rows_merge_row_has_merge_curve_without_split_metadata() {
    let commits = vec![
      make_commit("m", &["a", "b"]),
      make_commit("a", &["p"]),
      make_commit("b", &["p"]),
      make_commit("p", &[]),
    ];

    let rows = GitPage::build_graph_rows(&commits);
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0].merge_parent_lanes, vec![1]);
    assert!(rows[0].branch_child_lanes.is_empty());
    assert!(rows[0].branch_pre_stubs.is_empty());
  }
}
