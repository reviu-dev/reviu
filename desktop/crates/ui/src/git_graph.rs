use std::{
  collections::{HashMap, HashSet},
  path::PathBuf,
  sync::Arc,
};

use gpui::{
  AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
  LayoutId, PaintQuad, ParentElement, Path as GpuiPath, PathBuilder, Pixels, RenderOnce,
  SharedString, Style, Window, div, fill, point, prelude::*, px, size,
};
use gpui_component::{ActiveTheme as _, Sizable, Theme, spinner::Spinner};

use crate::StatusThemeExt;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GitGraphLaneSegment {
  pub up: bool,
  pub down: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct GitGraphLanesStyle {
  pub lane_width: f32,
  pub line_width: f32,
  pub commit_row_height: f32,
  pub curve_stub_height: f32,
  pub dot_size: f32,
  pub dot_size_latest: f32,
}

#[derive(Clone, Debug)]
pub struct GitGraphLanesLayout {
  pub segments: Vec<GitGraphLaneSegment>,
  pub lane_branch_ids: Vec<Option<usize>>,
  pub commit_lane: usize,
  pub commit_branch_id: usize,
  pub lane_transitions: Vec<(usize, usize, usize)>,
  pub merge_parent_lanes: Vec<usize>,
  pub merge_parent_lane_branches: Vec<(usize, usize)>,
  pub branch_child_lane_branches: Vec<(usize, usize)>,
  pub branch_pre_stub_lane_branches: Vec<(usize, usize)>,
  pub commit_lane_has_up: bool,
  pub is_latest_commit: bool,
  pub row_height: f32,
}

struct GraphPaintPath {
  path: GpuiPath<Pixels>,
  color: gpui::Hsla,
}

pub struct GitGraphLanesPrepaintState {
  paths: Vec<GraphPaintPath>,
  dot: PaintQuad,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MergeTransitionTarget {
  merged_branch_id: usize,
  curve_branch_id: usize,
}

pub struct GitGraphLanes {
  id: ElementId,
  layout: GitGraphLanesLayout,
  style: GitGraphLanesStyle,
  latest_dot_background: gpui::Hsla,
  branch_palette: Vec<gpui::Hsla>,
  dot_color: gpui::Hsla,
}

impl GitGraphLanes {
  pub fn new(
    id: impl Into<ElementId>,
    layout: GitGraphLanesLayout,
    style: GitGraphLanesStyle,
    theme: &Theme,
  ) -> Self {
    let branch_palette = Self::branch_palette(theme);
    let dot_color = Self::branch_color_for(layout.commit_branch_id, &branch_palette)
      .unwrap_or(theme.sidebar_foreground);

    Self {
      id: id.into(),
      layout,
      style,
      latest_dot_background: theme.sidebar,
      branch_palette,
      dot_color,
    }
  }

  pub fn branch_palette(theme: &Theme) -> Vec<gpui::Hsla> {
    vec![
      theme.status_blue().opacity(0.98),
      theme.status_yellow().opacity(0.98),
      theme.status_green().opacity(0.98),
      theme.status_red().opacity(0.98),
      gpui::Hsla {
        h: 0.08,
        s: 0.58,
        l: 0.40,
        a: 0.98,
      },
      theme.status_violet().opacity(0.98),
      theme.status_orange().opacity(0.98),
      gpui::Hsla {
        h: 0.49,
        s: 0.62,
        l: 0.52,
        a: 0.98,
      },
      gpui::Hsla {
        h: 0.90,
        s: 0.64,
        l: 0.58,
        a: 0.98,
      },
      gpui::Hsla {
        h: 0.17,
        s: 0.68,
        l: 0.50,
        a: 0.98,
      },
      gpui::Hsla {
        h: 0.62,
        s: 0.64,
        l: 0.56,
        a: 0.98,
      },
    ]
  }

  pub fn branch_color_for(branch_id: usize, palette: &[gpui::Hsla]) -> Option<gpui::Hsla> {
    if palette.is_empty() {
      return None;
    }
    Some(palette[branch_id % palette.len()])
  }

  fn lane_color(&self, lane_ix: usize) -> gpui::Hsla {
    self
      .layout
      .lane_branch_ids
      .get(lane_ix)
      .copied()
      .flatten()
      .and_then(|branch_id| Self::branch_color_for(branch_id, &self.branch_palette))
      .unwrap_or(self.dot_color)
  }

  fn lane_center_x(&self, lane_ix: usize, bounds: Bounds<Pixels>) -> Pixels {
    bounds.left() + px(lane_ix as f32 * self.style.lane_width + (self.style.lane_width / 2.0))
  }

  fn merge_transition_targets(
    lane_transitions: &[(usize, usize, usize)],
    merge_parent_lane_branches: &[(usize, usize)],
    lane_branch_ids: &[Option<usize>],
  ) -> HashMap<usize, MergeTransitionTarget> {
    let merge_parent_branch_ids = merge_parent_lane_branches
      .iter()
      .map(|(_, branch_id)| *branch_id)
      .collect::<HashSet<_>>();
    let mut targets = HashMap::<usize, MergeTransitionTarget>::new();
    for (_, to_lane, branch_id) in lane_transitions.iter().copied() {
      if merge_parent_branch_ids.contains(&branch_id) {
        let curve_branch_id = lane_branch_ids
          .get(to_lane)
          .copied()
          .flatten()
          .unwrap_or(branch_id);
        targets.entry(to_lane).or_insert(MergeTransitionTarget {
          merged_branch_id: branch_id,
          curve_branch_id,
        });
      }
    }
    targets
  }

}

impl IntoElement for GitGraphLanes {
  type Element = Self;

  fn into_element(self) -> Self::Element {
    self
  }
}

impl Element for GitGraphLanes {
  type RequestLayoutState = ();
  type PrepaintState = GitGraphLanesPrepaintState;

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
    let lane_count = self.layout.segments.len().max(1);
    let mut style = Style::default();
    style.size.width = px(lane_count as f32 * self.style.lane_width).into();
    style.size.height = px(self.layout.row_height).into();
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
    let mut paths =
      Vec::with_capacity(self.layout.segments.len() + self.layout.merge_parent_lanes.len() + 1);
    let middle_y = bounds.top() + px(self.style.commit_row_height / 2.0);
    let curve_stub_len = px(self.style.curve_stub_height);
    let merge_parent_lane_by_branch = self
      .layout
      .merge_parent_lane_branches
      .iter()
      .copied()
      .map(|(lane, branch_id)| (branch_id, lane))
      .collect::<HashMap<_, _>>();
    let transition_by_branch = self
      .layout
      .lane_transitions
      .iter()
      .copied()
      .map(|(from_lane, to_lane, branch_id)| (branch_id, (from_lane, to_lane)))
      .collect::<HashMap<_, _>>();
    let merge_transition_targets = Self::merge_transition_targets(
      &self.layout.lane_transitions,
      &self.layout.merge_parent_lane_branches,
      &self.layout.lane_branch_ids,
    );

    for (lane_ix, segment) in self.layout.segments.iter().enumerate() {
      if !segment.up && !segment.down {
        continue;
      }
      let is_merge_parent_lane = lane_ix != self.layout.commit_lane
        && self
          .layout
          .merge_parent_lane_branches
          .iter()
          .any(|(lane, _)| *lane == lane_ix);
      let merge_source_without_up = is_merge_parent_lane && !segment.up;
      if merge_source_without_up
        && self
          .layout
          .merge_parent_lane_branches
          .iter()
          .find_map(|(lane, branch_id)| (*lane == lane_ix).then_some(*branch_id))
          .and_then(|branch_id| transition_by_branch.get(&branch_id).copied())
          .is_some_and(|(from_lane, to_lane)| from_lane == lane_ix && to_lane != lane_ix)
      {
        // This merge parent lane transitions on the same row; its continuation in
        // expanded area must be drawn on the transition target lane.
        continue;
      }

      let is_split_target_lane = self
        .layout
        .branch_child_lane_branches
        .iter()
        .any(|(lane, _)| *lane == lane_ix);
      if is_split_target_lane && segment.up && !segment.down {
        continue;
      }
      let is_merge_transition_target_lane =
        merge_transition_targets.contains_key(&lane_ix) && segment.up && !segment.down;
      if is_merge_transition_target_lane {
        continue;
      }

      let lane_has_up = if lane_ix == self.layout.commit_lane {
        segment.up && self.layout.commit_lane_has_up
      } else {
        segment.up
      };
      let top_y = if merge_source_without_up {
        // Keep merge curve angle fixed on header height and extend straight down
        // through expanded children area to avoid visual holes.
        bounds.top() + px(self.style.commit_row_height)
      } else if lane_has_up {
        bounds.top()
      } else {
        middle_y
      };
      let bottom_y = if segment.down {
        bounds.bottom()
      } else {
        middle_y
      };
      if bottom_y <= top_y {
        continue;
      }

      let mut builder = PathBuilder::stroke(px(self.style.line_width));
      let lane_x = self.lane_center_x(lane_ix, bounds);
      builder.move_to(point(lane_x, top_y));
      builder.line_to(point(lane_x, bottom_y));

      if let Ok(path) = builder.build() {
        paths.push(GraphPaintPath {
          path,
          color: self.lane_color(lane_ix),
        });
      }
    }

    let commit_x = self.lane_center_x(self.layout.commit_lane, bounds);
    let split_target_stub_len = px((self.style.curve_stub_height * 0.5).max(2.0));
    for (lane_ix, target) in merge_transition_targets {
      if lane_ix == self.layout.commit_lane {
        continue;
      }
      let Some(segment) = self.layout.segments.get(lane_ix) else {
        continue;
      };
      if !segment.up || segment.down {
        continue;
      }
      let lane_x = self.lane_center_x(lane_ix, bounds);
      let target_y = bounds.top() + split_target_stub_len;
      let curve_start_y = bounds.top() + px(self.style.commit_row_height);
      let mut stub_builder = PathBuilder::stroke(px(self.style.line_width));
      stub_builder.move_to(point(lane_x, bounds.top()));
      stub_builder.line_to(point(lane_x, target_y));
      let Some(stub_path) = stub_builder.build().ok() else {
        continue;
      };
      let curve_color = Self::branch_color_for(target.curve_branch_id, &self.branch_palette)
        .unwrap_or_else(|| self.lane_color(lane_ix));
      paths.push(GraphPaintPath {
        path: stub_path,
        color: curve_color,
      });
      if bounds.bottom() > curve_start_y {
        let continuation_color =
          Self::branch_color_for(target.merged_branch_id, &self.branch_palette)
            .unwrap_or(curve_color);
        let mut extension_builder = PathBuilder::stroke(px(self.style.line_width));
        extension_builder.move_to(point(lane_x, curve_start_y));
        extension_builder.line_to(point(lane_x, bounds.bottom()));
        if let Ok(extension_path) = extension_builder.build() {
          paths.push(GraphPaintPath {
            path: extension_path,
            color: continuation_color,
          });
        }
      }

      let from_x: f32 = commit_x.into();
      let to_x: f32 = lane_x.into();
      let dx = to_x - from_x;
      if dx.abs() < 0.1 {
        continue;
      }

      let mut builder = PathBuilder::stroke(px(self.style.line_width));
      let start_y: f32 = target_y.into();
      let end_y: f32 = middle_y.into();
      let horizontal_gap = dx.abs();
      let vertical_span = (end_y - start_y).abs();
      let depth = (vertical_span * 0.85 + horizontal_gap * 0.10)
        .clamp(4.0, self.style.commit_row_height * 0.65);
      let ctrl_a_x = to_x;
      let ctrl_b_x = from_x + dx * 0.22;
      let ctrl_a_y = start_y + depth;
      let ctrl_b_y = end_y;

      builder.move_to(point(px(to_x), px(start_y)));
      builder.cubic_bezier_to(
        point(px(from_x), px(end_y)),
        point(px(ctrl_a_x), px(ctrl_a_y)),
        point(px(ctrl_b_x), px(ctrl_b_y)),
      );
      if let Ok(path) = builder.build() {
        paths.push(GraphPaintPath {
          path,
          color: curve_color,
        });
      }
    }

    for (from_lane, to_lane, branch_id) in self.layout.lane_transitions.iter().copied() {
      if from_lane == to_lane {
        continue;
      }
      if merge_parent_lane_by_branch
        .get(&branch_id)
        .is_some_and(|merge_lane| *merge_lane == from_lane)
      {
        // If this transition belongs to a merged branch on the same row, merge
        // curve rendering will consume it (drawn from the transitioned lane).
        continue;
      }
      let from_x: f32 = self.lane_center_x(from_lane, bounds).into();
      let to_x: f32 = self.lane_center_x(to_lane, bounds).into();
      let dx = to_x - from_x;
      if dx.abs() < 0.1 {
        continue;
      }

      let mut builder = PathBuilder::stroke(px(self.style.line_width));
      let start_y: f32 = middle_y.into();
      let horizontal_gap = dx.abs();
      let end_y: f32 = middle_y.into();
      let depth = (horizontal_gap * 0.30).clamp(3.0, self.style.commit_row_height * 0.35);
      let ctrl_a_x = from_x + dx * 0.35;
      let ctrl_b_x = to_x - dx * 0.35;
      let ctrl_a_y = start_y - depth;
      let ctrl_b_y = end_y - depth;
      builder.move_to(point(px(from_x), px(start_y)));
      builder.cubic_bezier_to(
        point(px(to_x), px(end_y)),
        point(px(ctrl_a_x), px(ctrl_a_y)),
        point(px(ctrl_b_x), px(ctrl_b_y)),
      );
      if let Ok(path) = builder.build() {
        let transition_color = Self::branch_color_for(branch_id, &self.branch_palette)
          .unwrap_or_else(|| self.lane_color(from_lane));
        paths.push(GraphPaintPath {
          path,
          color: transition_color,
        });
      }
    }

    for (parent_lane, merge_branch_id) in self.layout.merge_parent_lane_branches.iter().copied() {
      if parent_lane == self.layout.commit_lane {
        continue;
      }
      let effective_parent_lane = transition_by_branch
        .get(&merge_branch_id)
        .copied()
        .and_then(|(from_lane, to_lane)| (from_lane == parent_lane).then_some(to_lane))
        .unwrap_or(parent_lane);
      if effective_parent_lane == self.layout.commit_lane {
        continue;
      }

      let from_x: f32 = self.lane_center_x(effective_parent_lane, bounds).into();
      let to_x: f32 = commit_x.into();
      let dx = to_x - from_x;
      if dx.abs() < 0.1 {
        continue;
      }

      let mut builder = PathBuilder::stroke(px(self.style.line_width));
      // Keep merge curve angle stable even when expanded children increase row height.
      let curve_start_y = bounds.top() + px(self.style.commit_row_height);
      let start_y: f32 = curve_start_y.into();
      let end_y: f32 = middle_y.into();
      let horizontal_gap = dx.abs();
      let vertical_span = (start_y - end_y).abs();
      let depth = (vertical_span * 0.85 + horizontal_gap * 0.10)
        .clamp(4.0, self.style.commit_row_height * 0.65);
      let ctrl_a_x = from_x;
      let ctrl_b_x = to_x - dx * 0.22;
      let ctrl_a_y = start_y - depth;
      let ctrl_b_y = end_y;
      let right_lane = effective_parent_lane.max(self.layout.commit_lane);

      builder.move_to(point(px(from_x), px(start_y)));
      builder.cubic_bezier_to(
        point(px(to_x), px(end_y)),
        point(px(ctrl_a_x), px(ctrl_a_y)),
        point(px(ctrl_b_x), px(ctrl_b_y)),
      );

      if let Ok(path) = builder.build() {
        let merge_color = Self::branch_color_for(merge_branch_id, &self.branch_palette)
          .unwrap_or_else(|| self.lane_color(right_lane));
        paths.push(GraphPaintPath {
          path,
          color: merge_color,
        });
      }
    }

    let split_stub_len = curve_stub_len;
    for (lane, branch_id) in self.layout.branch_pre_stub_lane_branches.iter().copied() {
      let lane_x = self.lane_center_x(lane, bounds);
      let mut stub_builder = PathBuilder::stroke(px(self.style.line_width));
      let stub_start = bounds.bottom() - split_stub_len;
      stub_builder.move_to(point(lane_x, stub_start));
      stub_builder.line_to(point(lane_x, bounds.bottom()));
      if let Ok(path) = stub_builder.build() {
        let stub_color = Self::branch_color_for(branch_id, &self.branch_palette)
          .unwrap_or_else(|| self.lane_color(lane));
        paths.push(GraphPaintPath {
          path,
          color: stub_color,
        });
      }
    }

    for (child_lane, child_branch_id) in self
      .layout
      .branch_child_lane_branches
      .iter()
      .copied()
      .filter(|(lane, _)| *lane != self.layout.commit_lane)
    {
      let from_x: f32 = commit_x.into();
      let to_x: f32 = self.lane_center_x(child_lane, bounds).into();
      let dx = to_x - from_x;
      if dx.abs() < 0.1 {
        continue;
      }

      let target_y = bounds.top() + split_target_stub_len;

      let mut builder = PathBuilder::stroke(px(self.style.line_width));
      let start_y: f32 = middle_y.into();
      let end_y: f32 = target_y.into();
      let horizontal_gap = dx.abs();
      let vertical_span = (start_y - end_y).abs();
      let depth = (vertical_span * 0.85 + horizontal_gap * 0.10)
        .clamp(4.0, self.style.commit_row_height * 0.65);
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
        let split_color = Self::branch_color_for(child_branch_id, &self.branch_palette)
          .unwrap_or_else(|| self.lane_color(child_lane));
        let lane_x = self.lane_center_x(child_lane, bounds);
        let mut stub_builder = PathBuilder::stroke(px(self.style.line_width));
        stub_builder.move_to(point(lane_x, bounds.top()));
        stub_builder.line_to(point(lane_x, target_y));
        paths.push(GraphPaintPath {
          path,
          color: split_color,
        });
        if let Ok(stub_path) = stub_builder.build() {
          paths.push(GraphPaintPath {
            path: stub_path,
            color: split_color,
          });
        }
      }
    }

    let dot_center = point(commit_x, middle_y);
    let dot_diameter = if self.layout.is_latest_commit {
      self.style.dot_size_latest
    } else {
      self.style.dot_size
    };
    let dot_radius = px(dot_diameter / 2.0);
    let dot_bounds = Bounds::new(
      point(dot_center.x - dot_radius, dot_center.y - dot_radius),
      size(px(dot_diameter), px(dot_diameter)),
    );
    let dot_background = if self.layout.is_latest_commit {
      self.latest_dot_background
    } else {
      self.dot_color
    };
    let mut dot = fill(dot_bounds, dot_background).corner_radii(dot_radius);
    if self.layout.is_latest_commit {
      dot = dot
        .border_widths(px(self.style.line_width))
        .border_color(self.dot_color);
    }

    GitGraphLanesPrepaintState { paths, dot }
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

pub type GitGraphRowToggleHandler = Arc<dyn Fn(&mut Window, &mut App) + Send + Sync>;
pub type GitGraphRowOpenFileHandler = Arc<dyn Fn(PathBuf, &mut Window, &mut App) + Send + Sync>;

#[derive(Clone, Copy, Debug)]
pub struct GitGraphRowStyle {
  pub expanded_row_height: f32,
  pub branch_badge_max_width: f32,
  pub author_max_width: f32,
}

#[derive(Clone, Debug)]
pub struct GitGraphExpandedFileRow {
  pub path: PathBuf,
  pub label: SharedString,
  pub status_code: SharedString,
  pub status_color: gpui::Hsla,
  pub selected: bool,
}

#[derive(Clone, Debug, Default)]
pub enum GitGraphExpandedState {
  #[default]
  Collapsed,
  Loading,
  Empty,
  Files(Vec<GitGraphExpandedFileRow>),
}

#[derive(IntoElement)]
pub struct GitGraphRow {
  row_index: usize,
  lanes_layout: GitGraphLanesLayout,
  lanes_style: GitGraphLanesStyle,
  row_style: GitGraphRowStyle,
  summary: SharedString,
  author: SharedString,
  branch_label: Option<SharedString>,
  branch_color: gpui::Hsla,
  branch_text_color: gpui::Hsla,
  expanded_state: GitGraphExpandedState,
  on_toggle: Option<GitGraphRowToggleHandler>,
  on_open_file: Option<GitGraphRowOpenFileHandler>,
}

impl GitGraphRow {
  pub fn new(
    row_index: usize,
    lanes_layout: GitGraphLanesLayout,
    lanes_style: GitGraphLanesStyle,
    row_style: GitGraphRowStyle,
    summary: impl Into<SharedString>,
    author: impl Into<SharedString>,
    branch_label: Option<SharedString>,
    branch_color: gpui::Hsla,
    branch_text_color: gpui::Hsla,
    expanded_state: GitGraphExpandedState,
  ) -> Self {
    Self {
      row_index,
      lanes_layout,
      lanes_style,
      row_style,
      summary: summary.into(),
      author: author.into(),
      branch_label,
      branch_color,
      branch_text_color,
      expanded_state,
      on_toggle: None,
      on_open_file: None,
    }
  }

  pub fn on_toggle(mut self, handler: GitGraphRowToggleHandler) -> Self {
    self.on_toggle = Some(handler);
    self
  }

  pub fn on_open_file(mut self, handler: GitGraphRowOpenFileHandler) -> Self {
    self.on_open_file = Some(handler);
    self
  }
}

impl RenderOnce for GitGraphRow {
  fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
    let theme = cx.theme().clone();
    let row_hover_bg = theme.title_bar_border.opacity(0.26);
    let lane_count = self.lanes_layout.segments.len().max(1);
    let graph_width = lane_count as f32 * self.lanes_style.lane_width;
    let total_height = self.lanes_layout.row_height;

    let mut line_hits: Vec<AnyElement> = vec![
      div()
        .id(format!("git-graph-row-hover-hit-{}", self.row_index))
        .absolute()
        .top(px(0.0))
        .left(px(0.0))
        .right(px(0.0))
        .h(px(self.lanes_style.commit_row_height))
        .hover(|this| this.bg(row_hover_bg))
        .when_some(self.on_toggle.clone(), |this, on_toggle| {
          this.on_click(move |_, window, cx| {
            on_toggle(window, cx);
          })
        })
        .into_any_element(),
    ];

    let branch_badge = self.branch_label.clone().map(|label| {
      div()
        .id(format!("git-graph-branch-badge-{}", self.row_index))
        .px_1()
        .py(px(1.))
        .rounded(px(4.))
        .max_w(px(self.row_style.branch_badge_max_width))
        .overflow_hidden()
        .text_ellipsis()
        .text_xs()
        .bg(self.branch_color)
        .text_color(self.branch_text_color)
        .child(label)
        .into_any_element()
    });

    let commit_header = div()
      .id(format!("git-graph-row-header-{}", self.row_index))
      .h(px(self.lanes_style.commit_row_height))
      .min_h(px(self.lanes_style.commit_row_height))
      .flex()
      .items_center()
      .gap_2()
      .overflow_hidden()
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
              .child(self.summary.clone()),
          )
          .child(
            div()
              .min_w_0()
              .max_w(px(self.row_style.author_max_width))
              .overflow_hidden()
              .text_ellipsis()
              .text_xs()
              .text_color(theme.muted_foreground)
              .child(self.author.clone()),
          ),
      )
      .when_some(branch_badge, |this, badge| this.child(badge));

    let mut expanded_children = Vec::<AnyElement>::new();
    let mut expanded_line_ix = 0usize;
    match self.expanded_state {
      GitGraphExpandedState::Collapsed => {}
      GitGraphExpandedState::Loading => {
        let loading_top = self.lanes_style.commit_row_height
          + expanded_line_ix as f32 * self.row_style.expanded_row_height;
        line_hits.push(
          div()
            .id(format!(
              "git-graph-expanded-loading-hover-hit-{}",
              self.row_index
            ))
            .group(SharedString::from(format!(
              "git-graph-line-expanded-{}-{}",
              self.row_index, expanded_line_ix
            )))
            .absolute()
            .top(px(loading_top))
            .left(px(0.0))
            .right(px(0.0))
            .h(px(self.row_style.expanded_row_height))
            .hover(|this| this.bg(row_hover_bg))
            .into_any_element(),
        );
        expanded_children.push(
          div()
            .id(format!("git-graph-expanded-loading-{}", self.row_index))
            .h(px(self.row_style.expanded_row_height))
            .min_h(px(self.row_style.expanded_row_height))
            .pl_1()
            .pr_1()
            .flex()
            .items_center()
            .gap_2()
            .child(Spinner::new().small())
            .child(
              div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child("Loading files..."),
            )
            .into_any_element(),
        );
      }
      GitGraphExpandedState::Empty => {
        let empty_top = self.lanes_style.commit_row_height
          + expanded_line_ix as f32 * self.row_style.expanded_row_height;
        line_hits.push(
          div()
            .id(format!(
              "git-graph-expanded-empty-hover-hit-{}",
              self.row_index
            ))
            .group(SharedString::from(format!(
              "git-graph-line-expanded-{}-{}",
              self.row_index, expanded_line_ix
            )))
            .absolute()
            .top(px(empty_top))
            .left(px(0.0))
            .right(px(0.0))
            .h(px(self.row_style.expanded_row_height))
            .hover(|this| this.bg(row_hover_bg))
            .into_any_element(),
        );
        expanded_children.push(
          div()
            .id(format!("git-graph-expanded-empty-{}", self.row_index))
            .h(px(self.row_style.expanded_row_height))
            .min_h(px(self.row_style.expanded_row_height))
            .pl_2()
            .pr_1()
            .flex()
            .items_center()
            .child(
              div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child("No changed files"),
            )
            .into_any_element(),
        );
      }
      GitGraphExpandedState::Files(files) => {
        for (file_ix, file) in files.iter().enumerate() {
          let file_top = self.lanes_style.commit_row_height
            + expanded_line_ix as f32 * self.row_style.expanded_row_height;
          line_hits.push(
            div()
              .id(format!(
                "git-graph-expanded-file-hover-hit-{}-{}",
                self.row_index, file_ix
              ))
              .group(SharedString::from(format!(
                "git-graph-expanded-file-{}-{}",
                self.row_index, file_ix
              )))
              .absolute()
              .top(px(file_top))
              .left(px(0.0))
              .right(px(0.0))
              .h(px(self.row_style.expanded_row_height))
              .when(file.selected, |this| {
                this.bg(theme.title_bar_border.opacity(0.45))
              })
              .when(!file.selected, |this| {
                this.hover(|this| this.bg(theme.title_bar_border.opacity(0.26)))
              })
              .when_some(self.on_open_file.clone(), |this, on_open_file| {
                let path = file.path.clone();
                this.on_click(move |_, window, cx| {
                  on_open_file(path.clone(), window, cx);
                })
              })
              .into_any_element(),
          );
          expanded_line_ix += 1;

          expanded_children.push(
            div()
              .id(format!(
                "git-graph-expanded-file-{}-{}",
                self.row_index, file_ix
              ))
              .h(px(self.row_style.expanded_row_height))
              .min_h(px(self.row_style.expanded_row_height))
              .pl_1()
              .pr_1()
              .flex()
              .items_center()
              .gap_2()
              .overflow_hidden()
              .child(
                div()
                  .w(px(14.))
                  .text_xs()
                  .text_color(file.status_color)
                  .child(file.status_code.clone()),
              )
              .child(
                div()
                  .min_w_0()
                  .flex_1()
                  .overflow_hidden()
                  .text_ellipsis()
                  .text_xs()
                  .child(file.label.clone()),
              )
              .into_any_element(),
          );
        }
      }
    }

    div()
      .id(format!("git-graph-row-{}", self.row_index))
      .relative()
      .w_full()
      .h(px(total_height))
      .min_h(px(total_height))
      .children(line_hits)
      .child(
        div()
          .id(format!("git-graph-row-content-{}", self.row_index))
          .h(px(total_height))
          .min_h(px(total_height))
          .pl_2()
          .pr_3()
          .flex()
          .items_start()
          .gap_2()
          .child(
            div()
              .id(format!("git-graph-canvas-{}", self.row_index))
              .relative()
              .h(px(total_height))
              .w(px(graph_width))
              .flex_shrink_0()
              .child(GitGraphLanes::new(
                ("git-graph-lanes", self.row_index),
                self.lanes_layout,
                self.lanes_style,
                &theme,
              )),
          )
          .child(
            div()
              .min_w_0()
              .flex_1()
              .flex()
              .flex_col()
              .overflow_hidden()
              .child(commit_header)
              .children(expanded_children),
          ),
      )
  }
}

#[cfg(test)]
mod tests {
  use super::{GitGraphLanes, MergeTransitionTarget};

  #[test]
  fn merge_transition_targets_keep_merged_branch_id_on_target_lane() {
    let targets =
      GitGraphLanes::merge_transition_targets(&[(3, 1, 7)], &[(3, 7)], &[Some(0), Some(1)]);
    assert_eq!(
      targets.get(&1),
      Some(&MergeTransitionTarget {
        merged_branch_id: 7,
        curve_branch_id: 1,
      })
    );
  }

  #[test]
  fn merge_transition_targets_ignore_non_merge_transitions() {
    let targets = GitGraphLanes::merge_transition_targets(
      &[(3, 1, 7), (2, 1, 5)],
      &[(3, 7)],
      &[Some(0), Some(1)],
    );
    assert_eq!(targets.len(), 1);
    assert_eq!(
      targets.get(&1),
      Some(&MergeTransitionTarget {
        merged_branch_id: 7,
        curve_branch_id: 1,
      })
    );
  }

  #[test]
  fn merge_transition_targets_fallback_to_merged_branch_when_target_lane_has_no_branch() {
    let targets =
      GitGraphLanes::merge_transition_targets(&[(3, 1, 7)], &[(3, 7)], &[Some(0), None]);
    assert_eq!(
      targets.get(&1),
      Some(&MergeTransitionTarget {
        merged_branch_id: 7,
        curve_branch_id: 7,
      })
    );
  }
}
