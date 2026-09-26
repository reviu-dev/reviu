use gpui::{
  App, Bounds, CursorStyle, DispatchPhase, ElementId, Entity, GlobalElementId, Hitbox,
  HitboxBehavior, InspectorElementId, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent,
  PaintQuad, Pixels, ScrollDelta, ScrollWheelEvent, Style, TextAlign, TextRun, Window, fill, point,
  prelude::*, px, relative, size,
};
use gpui_component::ActiveTheme as _;
use std::{collections::HashMap, ops::Range, sync::Arc};

use git::DiffLineKind;

use crate::{
  editor::{ConflictLineKind, DisplayCursor, Editor, ScrollAxis},
  editor_element::DiffElementView,
  git_gutter::{GitGutterLineKind, GitGutterMarkers},
  projection::{
    ChangeKind, DisplayLine, HunkState, Projection, ProjectionBlock, ProjectionBlockMap,
    ReviewCommentBackground, ReviewCommentSide,
  },
};

const PIXEL_SCROLL_DIVISOR: f32 = 20.0;
const LINE_SCROLL_MULTIPLIER: f32 = 3.0;
const SCROLL_AXIS_RATIO: f32 = 1.1;
const SCROLL_AXIS_SWITCH_RATIO: f32 = 1.4;
const SCROLL_AXIS_TIMEOUT_MS: u64 = 150;
const CONFLICT_MARKER_ALPHA_MULTIPLIER: f32 = 1.35;

fn line_y(
  bounds_top: Pixels,
  line_height: Pixels,
  display_line: usize,
  scroll_offset: f32,
) -> Pixels {
  bounds_top + line_height * (display_line as f32 - scroll_offset)
}

fn conflict_doc_line(display_line: &DisplayLine) -> Option<usize> {
  match display_line {
    DisplayLine::Doc { doc_line, .. } | DisplayLine::Modified { doc_line, .. } => Some(*doc_line),
    _ => None,
  }
}

fn conflict_background(theme: &ui::Theme, kind: ConflictLineKind) -> Option<gpui::Hsla> {
  match kind {
    ConflictLineKind::Current => Some(theme.current_conflict_background()),
    ConflictLineKind::CurrentMarker => {
      let mut color = theme.current_conflict_background();
      color.a = (color.a * CONFLICT_MARKER_ALPHA_MULTIPLIER).min(1.0);
      Some(color)
    }
    ConflictLineKind::Base => Some(theme.base_conflict_background()),
    ConflictLineKind::BaseMarker => {
      let mut color = theme.base_conflict_background();
      color.a = (color.a * CONFLICT_MARKER_ALPHA_MULTIPLIER).min(1.0);
      Some(color)
    }
    ConflictLineKind::Divider => None,
    ConflictLineKind::Incoming => Some(theme.incoming_conflict_background()),
    ConflictLineKind::IncomingMarker => {
      let mut color = theme.incoming_conflict_background();
      color.a = (color.a * CONFLICT_MARKER_ALPHA_MULTIPLIER).min(1.0);
      Some(color)
    }
  }
}

fn conflict_stripe_color(theme: &ui::Theme, _kind: ConflictLineKind) -> gpui::Hsla {
  theme.conflict_block_stripe()
}

fn inactive_conflict_border_colors(theme: &ui::Theme) -> (gpui::Hsla, gpui::Hsla) {
  (
    theme.current_conflict_stripe(),
    theme.incoming_conflict_stripe(),
  )
}

fn inactive_conflict_border_edges(
  previous: Option<ConflictLineKind>,
  _current: ConflictLineKind,
  next: Option<ConflictLineKind>,
) -> (bool, bool) {
  (previous.is_none(), next.is_none())
}

fn active_conflict_border_edges(
  previous: Option<usize>,
  current: Option<usize>,
  next: Option<usize>,
  active_range: Option<&Range<usize>>,
) -> Option<(bool, bool)> {
  let active_range = active_range?;
  current.filter(|line| active_range.contains(line))?;
  Some((
    previous.is_none_or(|line| !active_range.contains(&line)),
    next.is_none_or(|line| !active_range.contains(&line)),
  ))
}

fn conflict_doc_line_for_display_line(
  display_line: usize,
  projection: Option<&Projection>,
  doc_line_count: usize,
) -> Option<usize> {
  if let Some(projection) = projection {
    projection
      .lines
      .get(display_line)
      .and_then(conflict_doc_line)
  } else if display_line < doc_line_count {
    Some(display_line)
  } else {
    None
  }
}

fn conflict_kind_for_display_line(
  display_line: usize,
  projection: Option<&Projection>,
  doc_line_count: usize,
  conflict_line_kinds: &HashMap<usize, ConflictLineKind>,
) -> Option<ConflictLineKind> {
  let doc_line = conflict_doc_line_for_display_line(display_line, projection, doc_line_count)?;
  conflict_line_kinds.get(&doc_line).copied()
}

fn group_id_for_gutter_display_line(
  display_line: usize,
  projection: Option<&Projection>,
  block_map: &ProjectionBlockMap,
  include_context_doc_lines: bool,
) -> Option<Arc<str>> {
  block_map
    .block_at_display_line(display_line)
    .and_then(|block| block.group_id.clone())
    .or_else(|| {
      projection
        .and_then(|projection| projection.lines.get(display_line))
        .and_then(|line| match line {
          DisplayLine::Doc { group_id, .. } if include_context_doc_lines => group_id.clone(),
          DisplayLine::Doc {
            change: Some(ChangeKind::Added),
            group_id,
            ..
          } => group_id.clone(),
          DisplayLine::Modified { group_id, .. } => group_id.clone(),
          DisplayLine::Removed { group_id, .. } => group_id.clone(),
          DisplayLine::NoNewline { group_id, .. } => group_id.clone(),
          _ => None,
        })
    })
}

pub struct GutterElement {
  editor: Entity<Editor>,
  view: GutterView,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GroupKind {
  Added,
  Removed,
  Mixed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GutterView {
  Inline,
  SplitLeft,
  SplitRight,
}

fn hunk_border_colors_for_kinds(
  theme: &ui::Theme,
  view: GutterView,
  kinds: impl IntoIterator<Item = DiffLineKind>,
) -> Option<(gpui::Hsla, gpui::Hsla)> {
  let mut has_add = false;
  let mut has_remove = false;
  let mut first_kind = None;
  let mut last_kind = None;

  for kind in kinds {
    match kind {
      DiffLineKind::Add => {
        has_add = true;
        first_kind.get_or_insert(kind);
        last_kind = Some(kind);
      }
      DiffLineKind::Remove => {
        has_remove = true;
        first_kind.get_or_insert(kind);
        last_kind = Some(kind);
      }
      DiffLineKind::Context => {}
    }
  }

  if has_add && has_remove {
    let removed = theme.diff_gutter_removed();
    let added = theme.diff_gutter_added();
    return Some(match view {
      GutterView::SplitLeft => (removed, removed),
      GutterView::SplitRight => (added, added),
      GutterView::Inline => (removed, added),
    });
  }

  let color_for_kind = |kind| match kind {
    DiffLineKind::Add => theme.diff_gutter_added(),
    DiffLineKind::Remove => theme.diff_gutter_removed(),
    DiffLineKind::Context => theme.diff_gutter_modified(),
  };

  Some((color_for_kind(first_kind?), color_for_kind(last_kind?)))
}

const GIT_GUTTER_STRIPE_WIDTH: f32 = 4.0;
/// Wider than the stripe: a 4px target is too thin to aim at.
const GIT_GUTTER_CLICK_WIDTH: f32 = 10.0;
const GIT_GUTTER_BORDER: f32 = 1.0;
const GIT_GUTTER_STAGED_FILL_OPACITY: f32 = 0.3;
const GIT_GUTTER_DELETION_HEIGHT: f32 = 2.0;
const GIT_GUTTER_STAGED_DELETION_OPACITY: f32 = 0.5;

fn git_gutter_color(theme: &ui::Theme, kind: GitGutterLineKind) -> gpui::Hsla {
  match kind {
    GitGutterLineKind::Added => theme.diff_gutter_added(),
    GitGutterLineKind::Modified => theme.diff_gutter_modified(),
  }
}

/// The stripe of one display row of the plain file.
#[derive(Clone, Copy, Debug, PartialEq)]
struct GitGutterRowMark {
  color: gpui::Hsla,
  /// Solid as in the diff, or outlined for a staged hunk that is folded: an
  /// expanded one already reads as staged from its outlined, paler lines.
  outlined: bool,
}

fn git_gutter_row_mark(
  markers: &GitGutterMarkers,
  theme: &ui::Theme,
  display_line: Option<DisplayLine>,
) -> Option<GitGutterRowMark> {
  match display_line? {
    // Only the lines of an expanded hunk keep their group in the projection.
    DisplayLine::Doc {
      doc_line, group_id, ..
    } => markers.lines.get(&doc_line).map(|marker| GitGutterRowMark {
      color: git_gutter_color(theme, marker.kind),
      outlined: marker.state == HunkState::Staged && group_id.is_none(),
    }),
    // An expanded hunk's removed lines carry its stripe, as in the diff: the
    // kind of the lines that replaced them, or removed when nothing did.
    DisplayLine::Removed { anchor_line, .. } => Some(GitGutterRowMark {
      color: markers
        .lines
        .get(&anchor_line)
        .map(|marker| git_gutter_color(theme, marker.kind))
        .unwrap_or_else(|| theme.diff_gutter_removed()),
      outlined: false,
    }),
    _ => None,
  }
}

/// Whether a click on the stripe of this row expands or folds a hunk.
fn git_gutter_row_toggles_hunk(
  markers: &GitGutterMarkers,
  display_line: Option<DisplayLine>,
) -> bool {
  match display_line {
    Some(DisplayLine::Doc { doc_line, .. }) => markers
      .hunks
      .iter()
      .any(|hunk| hunk.touches_doc_line(doc_line)),
    Some(DisplayLine::Removed { .. }) => true,
    _ => false,
  }
}

/// Staged changes read as in the diff: outlined over a pale fill, where
/// unstaged ones are solid. Rows are display lines, which an expanded hunk
/// shifts from document lines with the removed lines it brings back.
fn push_git_gutter_quads(
  markers: &GitGutterMarkers,
  theme: &ui::Theme,
  viewport: Range<usize>,
  display_line_at: &dyn Fn(usize) -> Option<DisplayLine>,
  display_line_of: &dyn Fn(usize) -> Option<usize>,
  doc_line_count: usize,
  bounds: Bounds<Pixels>,
  line_height: Pixels,
  scroll_offset: f32,
  quads: &mut Vec<PaintQuad>,
) {
  let left = bounds.left();
  let width = px(GIT_GUTTER_STRIPE_WIDTH);
  let border = px(GIT_GUTTER_BORDER);
  let mark_at = |row: usize| git_gutter_row_mark(markers, theme, display_line_at(row));
  for row in viewport.clone() {
    let Some(mark) = mark_at(row) else {
      continue;
    };
    let color = mark.color;
    let y = line_y(bounds.top(), line_height, row, scroll_offset);
    if !mark.outlined {
      quads.push(fill(
        Bounds::new(point(left, y), size(width, line_height)),
        color,
      ));
      continue;
    }

    let continues_run = |other: Option<usize>| other.and_then(mark_at) == Some(mark);
    quads.push(fill(
      Bounds::new(point(left, y), size(width, line_height)),
      color.opacity(GIT_GUTTER_STAGED_FILL_OPACITY),
    ));
    quads.push(fill(
      Bounds::new(point(left, y), size(border, line_height)),
      color,
    ));
    quads.push(fill(
      Bounds::new(point(left + width - border, y), size(border, line_height)),
      color,
    ));
    if !continues_run(row.checked_sub(1)) {
      quads.push(fill(
        Bounds::new(point(left, y), size(width, border)),
        color,
      ));
    }
    if !continues_run(Some(row + 1)) {
      quads.push(fill(
        Bounds::new(point(left, y + line_height - border), size(width, border)),
        color,
      ));
    }
  }

  let height = px(GIT_GUTTER_DELETION_HEIGHT);
  for deletion in &markers.deletions {
    // Drawn on the edge the removed lines sat on: the top of the line after
    // them, or the bottom of the last line when they ended the file.
    let (doc_line, offset) = if deletion.before_doc_line < doc_line_count {
      (deletion.before_doc_line, px(0.0))
    } else {
      (doc_line_count.saturating_sub(1), line_height - height)
    };
    let Some(row) = display_line_of(doc_line) else {
      continue;
    };
    if !viewport.contains(&row) {
      continue;
    }
    let color = match deletion.state {
      HunkState::Unstaged => theme.diff_gutter_removed(),
      HunkState::Staged => theme
        .diff_gutter_removed()
        .opacity(GIT_GUTTER_STAGED_DELETION_OPACITY),
    };
    let y = line_y(bounds.top(), line_height, row, scroll_offset) + offset;
    quads.push(fill(
      Bounds::new(point(left, y), size(width * 2.0, height)),
      color,
    ));
  }
}

pub struct GutterPrepaintState {
  line_numbers: Vec<(usize, String)>,
  line_height: Pixels,
  scroll_offset: f32,
  line_number_color: gpui::Hsla,
  line_number_right_padding: Pixels,
  line_backgrounds: Vec<PaintQuad>,
  gap_separators: Vec<PaintQuad>,
  stripe_quads: Vec<PaintQuad>,
  conflict_borders: Vec<PaintQuad>,
  group_borders: Vec<PaintQuad>,
  scroll_hitbox: Hitbox,
  /// Stripe rows a click expands or folds.
  hunk_toggle_hitboxes: Vec<Hitbox>,
}

impl GutterElement {
  fn review_comment_side(&self) -> Option<ReviewCommentSide> {
    match self.view {
      GutterView::SplitLeft => Some(ReviewCommentSide::Left),
      GutterView::SplitRight => Some(ReviewCommentSide::Right),
      GutterView::Inline => None,
    }
  }

  pub fn new(editor: Entity<Editor>) -> Self {
    Self {
      editor,
      view: GutterView::Inline,
    }
  }

  pub fn split_left(editor: Entity<Editor>) -> Self {
    Self {
      editor,
      view: GutterView::SplitLeft,
    }
  }

  pub fn split_right(editor: Entity<Editor>) -> Self {
    Self {
      editor,
      view: GutterView::SplitRight,
    }
  }
}

impl IntoElement for GutterElement {
  type Element = Self;

  fn into_element(self) -> Self::Element {
    self
  }
}

impl Element for GutterElement {
  type RequestLayoutState = ();
  type PrepaintState = GutterPrepaintState;

  fn id(&self) -> Option<ElementId> {
    None
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
    let mut style = Style::default();
    style.size.width = relative(1.).into();
    style.size.height = relative(1.).into();

    (window.request_layout(style, [], cx), ())
  }

  fn prepaint(
    &mut self,
    _id: Option<&GlobalElementId>,
    _inspector_id: Option<&InspectorElementId>,
    bounds: Bounds<Pixels>,
    _request_layout: &mut Self::RequestLayoutState,
    window: &mut Window,
    cx: &mut App,
  ) -> Self::PrepaintState {
    let measured_line_height = window.line_height();
    let (
      line_numbers,
      line_height,
      scroll_offset,
      line_number_color,
      line_number_right_padding,
      line_backgrounds,
      gap_separators,
      stripe_quads,
      conflict_borders,
      group_borders,
      scroll_hitbox,
      hunk_toggle_hitboxes,
    ) = {
      let editor = self.editor.read(cx);
      let document = editor.document().read(cx);
      let line_height = measured_line_height;
      let scroll_offset = editor.scroll_offset_y;
      let doc_line_count = document.len_lines();
      let total_lines = editor.display_line_count(doc_line_count);
      let theme = editor.theme.clone();
      let projection = editor.projection.clone();
      // The plain file marks its changes on its own; an expanded hunk only adds
      // its line backgrounds there.
      let show_stripes = editor.shows_git_diff();
      let scroll_hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
      let mut hunk_toggle_hitboxes = Vec::new();
      let line_number_right_padding = editor.gutter_line_number_right_padding();

      let viewport = editor
        .soft_wrap
        .map
        .logical_range(Editor::viewport_range_for_height(
          scroll_offset,
          bounds.size.height,
          line_height,
          editor.soft_wrap.map.count(total_lines),
        ));

      let mut group_kinds = HashMap::new();
      let mut group_border_colors = HashMap::new();
      if let Some(projection) = projection.as_ref() {
        for (group_id, group) in &projection.groups {
          let mut has_add = false;
          let mut has_remove = false;
          for line in &group.hunk.lines {
            match line.kind {
              DiffLineKind::Add => has_add = true,
              DiffLineKind::Remove => has_remove = true,
              DiffLineKind::Context => {}
            }
          }

          let kind = match (has_add, has_remove) {
            (true, false) => Some(GroupKind::Added),
            (false, true) => Some(GroupKind::Removed),
            (true, true) => Some(GroupKind::Mixed),
            _ => None,
          };

          if let Some(kind) = kind {
            group_kinds.insert(group_id.clone(), kind);
          }
          if group.state == HunkState::Staged
            && let Some(colors) = hunk_border_colors_for_kinds(
              &theme,
              self.view,
              group.hunk.lines.iter().map(|line| line.kind),
            )
          {
            group_border_colors.insert(group_id.clone(), colors);
          }
        }
      }

      let added_bg = theme.diff_added_background();
      let added_staged_bg = theme.diff_added_staged_background();
      let removed_bg = theme.diff_removed_background();
      let removed_staged_bg = theme.diff_removed_staged_background();
      let conflict_line_kinds = editor.conflict_line_kinds(cx);
      let stripe_added = theme.diff_gutter_added();
      let stripe_removed = theme.diff_gutter_removed();
      let stripe_modified = theme.diff_gutter_modified();

      let review_comment_side_for_block =
        |block: Option<&ProjectionBlock>| block.and_then(ProjectionBlock::review_comment_side);
      let review_comment_background_for_block = |block: Option<&ProjectionBlock>| {
        let block = block?;
        match block.background? {
          ReviewCommentBackground::Added => Some(if block.secondary {
            added_staged_bg
          } else {
            added_bg
          }),
          ReviewCommentBackground::Removed => Some(if block.secondary {
            removed_staged_bg
          } else {
            removed_bg
          }),
        }
      };

      let is_blank_for_view = |line: Option<&DisplayLine>, block: Option<&ProjectionBlock>| {
        let review_comment_side = review_comment_side_for_block(block);
        match self.view {
          GutterView::SplitLeft => {
            line.is_some_and(|line| {
              matches!(
                line,
                DisplayLine::Doc {
                  change: Some(ChangeKind::Added),
                  ..
                }
              )
            }) || review_comment_side == Some(ReviewCommentSide::Right)
          }
          GutterView::SplitRight => {
            line.is_some_and(|line| matches!(line, DisplayLine::Removed { .. }))
              || review_comment_side == Some(ReviewCommentSide::Left)
          }
          GutterView::Inline => false,
        }
      };

      let active_hunk_group_id = editor.highlighted_hunk_group_id(cx);
      let active_hunk_focus_color = theme.hunk_focused_border();
      let active_conflict_doc_range = editor.highlighted_conflict_doc_range(cx);

      let mut line_numbers = Vec::new();
      let mut line_backgrounds = Vec::new();
      let mut gap_separators = Vec::new();
      let mut stripe_quads = Vec::new();
      let mut conflict_borders = Vec::new();
      let mut group_borders = Vec::new();
      let group_id_for_visible_display_line = |display_idx: usize| {
        let display_line = editor.display_line(display_idx, doc_line_count);
        let block = editor.block_map.block_at_display_line(display_idx);
        if is_blank_for_view(display_line.as_ref(), block) {
          None
        } else {
          group_id_for_gutter_display_line(
            display_idx,
            projection.as_deref(),
            &editor.block_map,
            false,
          )
        }
      };
      for display_idx in viewport.clone() {
        let display_line = editor.display_line(display_idx, doc_line_count);
        let block = editor.block_map.block_at_display_line(display_idx);
        if editor
          .block_map
          .interior_gap_id_for_display_line(display_idx)
          .is_some()
        {
          let y = line_y(bounds.top(), line_height, display_idx, scroll_offset) + line_height * 0.5;
          gap_separators.push(fill(
            Bounds::new(point(bounds.left(), y), size(bounds.size.width, px(1.0))),
            cx.theme().muted_foreground.opacity(0.35),
          ));
        }

        let line_number = if editor.block_map.is_gap_display_line(display_idx) {
          String::new()
        } else {
          match (self.view, &display_line) {
            (GutterView::SplitLeft, Some(DisplayLine::Doc { old_line, .. })) => old_line
              .map(|line| format!("{}", line + 1))
              .unwrap_or_default(),
            (GutterView::SplitLeft, Some(DisplayLine::Modified { old_line, .. })) => {
              format!("{}", old_line + 1)
            }
            (GutterView::SplitLeft, Some(DisplayLine::Removed { old_line, .. })) => {
              format!("{}", old_line + 1)
            }
            (GutterView::SplitRight, Some(DisplayLine::Modified { doc_line, .. })) => {
              format!("{}", doc_line + 1)
            }
            (_, Some(DisplayLine::Doc { doc_line, .. })) => format!("{}", doc_line + 1),
            _ => String::new(),
          }
        };
        line_numbers.push((display_idx, line_number));

        let is_blank = is_blank_for_view(display_line.as_ref(), block);

        let conflict_kind = conflict_kind_for_display_line(
          display_idx,
          projection.as_deref(),
          doc_line_count,
          &conflict_line_kinds,
        );
        let background = if is_blank {
          None
        } else if let Some(conflict_kind) = conflict_kind {
          conflict_background(&theme, conflict_kind)
        } else if let Some(background) = review_comment_background_for_block(block) {
          Some(background)
        } else {
          match &display_line {
            Some(DisplayLine::Doc {
              change: Some(ChangeKind::Added),
              secondary,
              ..
            }) => Some(if *secondary {
              added_staged_bg
            } else {
              added_bg
            }),
            Some(DisplayLine::Removed { secondary, .. }) => Some(if *secondary {
              removed_staged_bg
            } else {
              removed_bg
            }),
            Some(DisplayLine::Modified { secondary, .. }) => match self.view {
              GutterView::SplitLeft => Some(if *secondary {
                removed_staged_bg
              } else {
                removed_bg
              }),
              GutterView::SplitRight => Some(if *secondary {
                added_staged_bg
              } else {
                added_bg
              }),
              GutterView::Inline => None,
            },
            _ => None,
          }
        };

        if let Some(color) = background {
          let y = line_y(bounds.top(), line_height, display_idx, scroll_offset);
          line_backgrounds.push(fill(
            Bounds::new(
              point(bounds.left(), y),
              size(bounds.size.width, line_height),
            ),
            color,
          ));
        }

        let any_group_id = group_id_for_gutter_display_line(
          display_idx,
          projection.as_deref(),
          &editor.block_map,
          true,
        );
        let is_active_hunk_line = match (&active_hunk_group_id, &any_group_id) {
          (Some(active), Some(line_group)) => active.as_ref() == line_group.as_ref(),
          _ => false,
        };

        let conflict_doc_line =
          conflict_doc_line_for_display_line(display_idx, projection.as_deref(), doc_line_count);
        let is_active_conflict_line = conflict_doc_line
          .zip(active_conflict_doc_range.as_ref())
          .map(|(line, range)| range.contains(&line))
          .unwrap_or(false);

        if let Some(conflict_kind) = conflict_kind {
          let previous_doc_line = display_idx.checked_sub(1).and_then(|idx| {
            conflict_doc_line_for_display_line(idx, projection.as_deref(), doc_line_count)
          });
          let next_doc_line = conflict_doc_line_for_display_line(
            display_idx + 1,
            projection.as_deref(),
            doc_line_count,
          );
          let active_edges = active_conflict_border_edges(
            previous_doc_line,
            conflict_doc_line,
            next_doc_line,
            active_conflict_doc_range.as_ref(),
          );
          let border = active_edges
            .map(|edges| ((active_hunk_focus_color, active_hunk_focus_color), edges))
            .or_else(|| {
              let previous_conflict_kind = display_idx.checked_sub(1).and_then(|idx| {
                conflict_kind_for_display_line(
                  idx,
                  projection.as_deref(),
                  doc_line_count,
                  &conflict_line_kinds,
                )
              });
              let next_conflict_kind = conflict_kind_for_display_line(
                display_idx + 1,
                projection.as_deref(),
                doc_line_count,
                &conflict_line_kinds,
              );
              Some((
                inactive_conflict_border_colors(&theme),
                inactive_conflict_border_edges(
                  previous_conflict_kind,
                  conflict_kind,
                  next_conflict_kind,
                ),
              ))
            });

          if let Some(((top_color, bottom_color), (is_top, is_bottom))) = border {
            let border_thickness = px(1.0);
            let y = line_y(bounds.top(), line_height, display_idx, scroll_offset);

            if is_top {
              conflict_borders.push(fill(
                Bounds::new(
                  point(bounds.left(), y),
                  size(bounds.size.width, border_thickness),
                ),
                top_color,
              ));
            }

            if is_bottom {
              conflict_borders.push(fill(
                Bounds::new(
                  point(bounds.left(), y + line_height - border_thickness),
                  size(bounds.size.width, border_thickness),
                ),
                bottom_color,
              ));
            }
          }
        }

        let group_id = group_id_for_gutter_display_line(
          display_idx,
          projection.as_deref(),
          &editor.block_map,
          false,
        );
        if show_stripes {
          let base_stripe_color = if is_blank {
            None
          } else {
            match conflict_kind {
              Some(conflict_kind) => Some(conflict_stripe_color(&theme, conflict_kind)),
              None => group_id.as_ref().and_then(|group_id| {
                group_kinds.get(group_id).map(|kind| match self.view {
                  GutterView::Inline => match kind {
                    GroupKind::Added => stripe_added,
                    GroupKind::Removed => stripe_removed,
                    GroupKind::Mixed => stripe_modified,
                  },
                  GutterView::SplitLeft => stripe_removed,
                  GutterView::SplitRight => stripe_added,
                })
              }),
            }
          };

          let stripe_color = if (is_active_hunk_line && conflict_kind.is_none())
            || (is_active_conflict_line && conflict_kind.is_some())
          {
            Some(active_hunk_focus_color)
          } else {
            base_stripe_color
          };

          if let Some(stripe_color) = stripe_color {
            let y = line_y(bounds.top(), line_height, display_idx, scroll_offset);
            stripe_quads.push(fill(
              Bounds::new(point(bounds.left(), y), size(px(4.0), line_height)),
              stripe_color,
            ));
          }
        }

        if !is_blank
          && conflict_kind.is_none()
          && let Some(group_id) = group_id
          && projection.is_some()
        {
          let border_colors = if is_active_hunk_line {
            Some((active_hunk_focus_color, active_hunk_focus_color))
          } else {
            group_border_colors.get(&group_id).copied()
          };

          if let Some((top_color, bottom_color)) = border_colors {
            let previous_group = display_idx
              .checked_sub(1)
              .and_then(group_id_for_visible_display_line);
            let next_group = group_id_for_visible_display_line(display_idx + 1);

            let is_top = previous_group.as_deref() != Some(group_id.as_ref());
            let is_bottom = next_group.as_deref() != Some(group_id.as_ref());
            let border_thickness = px(1.0);
            let stripe_width = if show_stripes { px(4.0) } else { px(0.0) };
            let width = if bounds.size.width > stripe_width {
              bounds.size.width - stripe_width
            } else {
              px(0.0)
            };
            let x = bounds.left() + stripe_width;
            let y = line_y(bounds.top(), line_height, display_idx, scroll_offset);

            if is_top {
              group_borders.push(fill(
                Bounds::new(point(x, y), size(width, border_thickness)),
                top_color,
              ));
            }

            if is_bottom {
              group_borders.push(fill(
                Bounds::new(
                  point(x, y + line_height - border_thickness),
                  size(width, border_thickness),
                ),
                bottom_color,
              ));
            }
          }
        }
      }

      if !editor.shows_git_diff()
        && let Some(markers) = editor.git_gutter_markers.as_deref()
      {
        let display_line_at =
          |display_line: usize| editor.display_line(display_line, doc_line_count);
        push_git_gutter_quads(
          markers,
          &theme,
          viewport.clone(),
          &display_line_at,
          &|doc_line| editor.doc_to_display_line(doc_line),
          doc_line_count,
          bounds,
          line_height,
          scroll_offset,
          &mut stripe_quads,
        );
        let map = &editor.soft_wrap.map;
        for row in viewport.clone() {
          if !git_gutter_row_toggles_hunk(markers, display_line_at(row)) {
            continue;
          }
          let y = line_y(bounds.top(), line_height, row, scroll_offset);
          let quad = map.block_quad(
            fill(
              Bounds::new(
                point(bounds.left(), y),
                size(px(GIT_GUTTER_CLICK_WIDTH), line_height),
              ),
              gpui::transparent_black(),
            ),
            bounds.top(),
            line_height,
            scroll_offset,
          );
          hunk_toggle_hitboxes.push(window.insert_hitbox(quad.bounds, HitboxBehavior::Normal));
        }
      }

      let line_number_color = editor.theme.line_number();
      let map = &editor.soft_wrap.map;
      let line_numbers = line_numbers
        .into_iter()
        .map(|(line, number)| (map.row(line), number))
        .collect();
      let map_quads = |quads: Vec<PaintQuad>| {
        quads
          .into_iter()
          .map(|quad| map.block_quad(quad, bounds.top(), line_height, scroll_offset))
          .collect()
      };
      let line_backgrounds = map_quads(line_backgrounds);
      let gap_separators = map_quads(gap_separators);
      let stripe_quads = map_quads(stripe_quads);
      let conflict_borders = map_quads(conflict_borders);
      let group_borders = map_quads(group_borders);

      (
        line_numbers,
        line_height,
        scroll_offset,
        line_number_color,
        line_number_right_padding,
        line_backgrounds,
        gap_separators,
        stripe_quads,
        conflict_borders,
        group_borders,
        scroll_hitbox,
        hunk_toggle_hitboxes,
      )
    };

    self.editor.update(cx, |editor, _| {
      editor.editor_line_height = measured_line_height;
    });

    GutterPrepaintState {
      line_numbers,
      line_height,
      scroll_offset,
      line_number_color,
      line_number_right_padding,
      line_backgrounds,
      gap_separators,
      stripe_quads,
      conflict_borders,
      group_borders,
      scroll_hitbox,
      hunk_toggle_hitboxes,
    }
  }

  fn paint(
    &mut self,
    _id: Option<&GlobalElementId>,
    _inspector_id: Option<&InspectorElementId>,
    bounds: Bounds<Pixels>,
    _request_layout: &mut Self::RequestLayoutState,
    prepaint: &mut Self::PrepaintState,
    window: &mut Window,
    cx: &mut App,
  ) {
    let text_style = window.text_style();
    let font_size = text_style.font_size.to_pixels(window.rem_size());
    let text_color = prepaint.line_number_color;

    for quad in &prepaint.line_backgrounds {
      window.paint_quad(quad.clone());
    }

    for quad in &prepaint.gap_separators {
      window.paint_quad(quad.clone());
    }

    for quad in &prepaint.stripe_quads {
      window.paint_quad(quad.clone());
    }

    for quad in &prepaint.conflict_borders {
      window.paint_quad(quad.clone());
    }

    for quad in &prepaint.group_borders {
      window.paint_quad(quad.clone());
    }

    for hitbox in &prepaint.hunk_toggle_hitboxes {
      window.set_cursor_style(CursorStyle::PointingHand, hitbox);
    }

    window.on_mouse_event({
      let editor = self.editor.clone();
      let hitbox = prepaint.scroll_hitbox.clone();
      let line_height = prepaint.line_height;
      let view = match self.view {
        GutterView::Inline => DiffElementView::Inline,
        GutterView::SplitLeft => DiffElementView::SplitLeft,
        GutterView::SplitRight => DiffElementView::SplitRight,
      };
      move |event: &MouseDownEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble
          || event.button != MouseButton::Left
          || !hitbox.is_hovered(window)
        {
          return;
        }
        let toggled_hunk = editor.update(cx, |editor, cx| {
          let line = (editor.scroll_offset_y + (event.position.y - bounds.top()) / line_height)
            .max(0.0)
            .floor() as usize;
          let line = editor.soft_wrap.map.line(line);
          if editor.shows_git_diff()
            || event.position.x > bounds.left() + px(GIT_GUTTER_CLICK_WIDTH)
          {
            return false;
          }
          let doc_line = match editor.display_line(line, editor.document().read(cx).len_lines()) {
            Some(DisplayLine::Doc { doc_line, .. }) => doc_line,
            Some(DisplayLine::Removed { anchor_line, .. }) => anchor_line,
            _ => return false,
          };
          editor.toggle_file_hunk_at_doc_line(doc_line, cx)
        });
        if toggled_hunk {
          cx.stop_propagation();
          return;
        }
        editor.update(cx, |editor, cx| {
          let line = (editor.scroll_offset_y + (event.position.y - bounds.top()) / line_height)
            .max(0.0)
            .floor() as usize;
          let line = editor.soft_wrap.map.line(line);
          let mut event = event.clone();
          event.click_count = 3;
          editor.begin_mouse_selection(DisplayCursor { line, column: 0 }, view, &event, window, cx);
        });
        cx.stop_propagation();
      }
    });

    window.on_mouse_event({
      let editor = self.editor.clone();
      let line_height = prepaint.line_height;
      let review_comment_side = self.review_comment_side();
      // The gutter hover belongs to its pane, like the text hover does.
      let is_primary = !matches!(self.view, GutterView::SplitLeft);
      move |event: &MouseMoveEvent, phase, _window, cx| {
        if phase != DispatchPhase::Bubble {
          return;
        }
        if !bounds.contains(&event.position) {
          return;
        }
        editor.update(cx, |editor, cx| {
          if editor.is_selecting {
            return;
          }
          editor.hovered_from_primary = is_primary;
          let display_line = {
            let y_offset = event.position.y - bounds.top();
            let line_float = editor.scroll_offset_y + (y_offset / line_height);
            if line_float.is_sign_negative() {
              None
            } else {
              Some(editor.soft_wrap.map.line(line_float.floor() as usize))
            }
          };

          let hovered = display_line.and_then(|display_line| {
            let doc_line_count = editor.document().read(cx).len_lines();
            let total_lines = editor.display_line_count(doc_line_count);
            if display_line < total_lines {
              editor.group_id_for_hunk_action_display_line(display_line, review_comment_side)
            } else {
              None
            }
          });

          editor.update_review_comment_create_drag_from_display_line_on_side(
            display_line,
            review_comment_side,
            cx,
          );

          if editor.hovered_group_id.as_deref() != hovered.as_deref() {
            editor.hovered_group_id = hovered;
            cx.notify();
          }
        });
      }
    });
    window.on_mouse_event({
      let editor = self.editor.clone();
      let scroll_hitbox = prepaint.scroll_hitbox.clone();
      let line_height = prepaint.line_height;
      move |event: &ScrollWheelEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble || !scroll_hitbox.should_handle_scroll(window) {
          return;
        }

        editor.update(cx, |editor, cx| {
          let document = editor.document().read(cx);
          let doc_line_count = document.len_lines();
          let total_lines = editor.display_line_count(doc_line_count);
          let now = std::time::Instant::now();
          let reset_lock = editor
            .last_scroll_time
            .map(|last| {
              now.duration_since(last) > std::time::Duration::from_millis(SCROLL_AXIS_TIMEOUT_MS)
            })
            .unwrap_or(true);
          if reset_lock {
            editor.scroll_axis_lock = None;
            editor.last_scroll_x = editor.scroll_handle.offset().x;
          }
          editor.last_scroll_time = Some(now);

          let pixel_delta = event.delta.pixel_delta(line_height);
          let delta_x_px = pixel_delta.x;
          let delta_y_px = -pixel_delta.y;
          let delta_y = match event.delta {
            ScrollDelta::Pixels(point) => -(point.y / px(PIXEL_SCROLL_DIVISOR)),
            ScrollDelta::Lines(point) => -(point.y * LINE_SCROLL_MULTIPLIER),
          };
          let abs_x = delta_x_px.abs();
          let abs_y = delta_y_px.abs();
          let axis = match editor.scroll_axis_lock {
            None => {
              let axis = if abs_x > abs_y * SCROLL_AXIS_RATIO {
                ScrollAxis::Horizontal
              } else {
                ScrollAxis::Vertical
              };
              if matches!(axis, ScrollAxis::Horizontal) {
                editor.last_scroll_x = editor.scroll_handle.offset().x;
              }
              editor.scroll_axis_lock = Some(axis);
              axis
            }
            Some(axis) => {
              if axis == ScrollAxis::Vertical && abs_x > abs_y * SCROLL_AXIS_SWITCH_RATIO {
                editor.last_scroll_x = editor.scroll_handle.offset().x;
                editor.scroll_axis_lock = Some(ScrollAxis::Horizontal);
                ScrollAxis::Horizontal
              } else if axis == ScrollAxis::Horizontal && abs_y > abs_x * SCROLL_AXIS_SWITCH_RATIO {
                editor.scroll_axis_lock = Some(ScrollAxis::Vertical);
                ScrollAxis::Vertical
              } else {
                axis
              }
            }
          };

          if axis == ScrollAxis::Horizontal {
            editor.set_horizontal_scroll_offset(editor.scroll_handle.offset().x + delta_x_px);
            cx.notify();
            return;
          }

          editor.scroll_offset_y = Editor::clamp_vertical_scroll_for_height(
            editor.scroll_offset_y + delta_y,
            bounds.size.height,
            line_height,
            editor.soft_wrap.map.count(total_lines),
          );
          let clamped_scroll_x = editor.clamp_horizontal_scroll_x(editor.last_scroll_x);
          if editor.scroll_handle.offset().x != clamped_scroll_x {
            editor.set_horizontal_scroll_offset(clamped_scroll_x);
          } else {
            editor.last_scroll_x = clamped_scroll_x;
          }
          let viewport = editor.viewport_range(line_height, total_lines);
          let doc_viewports = editor.doc_ranges_for_display_viewport(viewport.clone());
          editor.document.update(cx, |doc, cx| {
            doc.schedule_viewport_highlights_for_ranges(
              &doc_viewports,
              None,
              crate::document::VIEWPORT_HIGHLIGHT_MARGIN_LINES,
              cx,
            );
          });
          cx.notify();
        });

        cx.stop_propagation();
      }
    });

    for (line_idx, line_number) in &prepaint.line_numbers {
      let y = line_y(
        bounds.top(),
        prepaint.line_height,
        *line_idx,
        prepaint.scroll_offset,
      );

      let runs = vec![TextRun {
        len: line_number.len(),
        font: text_style.font(),
        color: text_color,
        background_color: None,
        underline: None,
        strikethrough: None,
      }];

      let shaped =
        window
          .text_system()
          .shape_line(line_number.clone().into(), font_size, &runs, None);

      let text_width = shaped.width;
      let x = bounds.right() - text_width - prepaint.line_number_right_padding;

      let line_origin = point(x, y);
      shaped
        .paint(
          line_origin,
          prepaint.line_height,
          TextAlign::Right,
          None,
          window,
          cx,
        )
        .ok();
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::projection::HunkState;

  #[test]
  fn a_staged_hunk_is_outlined_only_while_folded() {
    use crate::git_gutter::{GitGutterLine, GitGutterLineKind, GitGutterMarkers};
    use std::collections::HashMap;

    let theme = ui::Theme::dark();
    let markers = GitGutterMarkers {
      lines: HashMap::from([(
        2,
        GitGutterLine {
          kind: GitGutterLineKind::Modified,
          state: HunkState::Staged,
        },
      )]),
      ..Default::default()
    };
    let removed = |anchor_line| DisplayLine::Removed {
      text: Arc::from("old"),
      anchor_line,
      old_line: 1,
      hunk: HunkState::Staged,
      group_id: None,
      secondary: true,
    };

    let doc = |group_id: Option<Arc<str>>| DisplayLine::Doc {
      doc_line: 2,
      old_line: None,
      change: Some(ChangeKind::Added),
      hunk: Some(HunkState::Staged),
      group_id,
      secondary: true,
    };
    let solid = |color| {
      Some(GitGutterRowMark {
        color,
        outlined: false,
      })
    };

    assert_eq!(
      git_gutter_row_mark(&markers, &theme, Some(removed(2))),
      solid(theme.diff_gutter_modified())
    );
    assert_eq!(
      git_gutter_row_mark(&markers, &theme, Some(removed(5))),
      solid(theme.diff_gutter_removed()),
      "nothing replaced it: a removal"
    );
    assert_eq!(
      git_gutter_row_mark(&markers, &theme, Some(doc(Some(Arc::from("group"))))),
      solid(theme.diff_gutter_modified()),
      "expanded, it looks like the diff"
    );
    assert_eq!(
      git_gutter_row_mark(&markers, &theme, Some(doc(None))),
      Some(GitGutterRowMark {
        color: theme.diff_gutter_modified(),
        outlined: true,
      }),
      "folded, the outline is the only sign it is staged"
    );
    assert!(git_gutter_row_toggles_hunk(&markers, Some(removed(5))));
  }

  #[test]
  fn conflict_stripe_color_marks_the_whole_conflict_as_one_block() {
    let theme = ui::Theme::dark();
    for kind in [
      ConflictLineKind::CurrentMarker,
      ConflictLineKind::Current,
      ConflictLineKind::BaseMarker,
      ConflictLineKind::Base,
      ConflictLineKind::Divider,
      ConflictLineKind::Incoming,
      ConflictLineKind::IncomingMarker,
    ] {
      assert_eq!(
        conflict_stripe_color(&theme, kind),
        theme.conflict_block_stripe()
      );
    }
  }

  #[test]
  fn conflict_stripe_color_is_fully_opaque() {
    let dark = ui::Theme::dark();
    let light = ui::Theme::light();
    for kind in [
      ConflictLineKind::Current,
      ConflictLineKind::CurrentMarker,
      ConflictLineKind::Incoming,
      ConflictLineKind::IncomingMarker,
    ] {
      assert_eq!(conflict_stripe_color(&dark, kind).a, 1.0);
      assert_eq!(conflict_stripe_color(&light, kind).a, 1.0);
    }
  }

  #[test]
  fn group_id_for_gutter_display_line_uses_block_map_for_review_comment_rows() {
    let group_id: Arc<str> = Arc::from("group-1");
    let lines = vec![
      DisplayLine::Doc {
        doc_line: 0,
        old_line: Some(0),
        change: Some(ChangeKind::Added),
        hunk: Some(HunkState::Unstaged),
        group_id: Some(group_id.clone()),
        secondary: false,
      },
      DisplayLine::ReviewComment {
        id: 1,
        side: ReviewCommentSide::Right,
        group_id: Some(group_id.clone()),
        background: None,
        secondary: false,
        text: Arc::from("comment"),
        is_header: true,
      },
      DisplayLine::ReviewComment {
        id: 1,
        side: ReviewCommentSide::Right,
        group_id: Some(group_id.clone()),
        background: None,
        secondary: false,
        text: Arc::from(""),
        is_header: false,
      },
      DisplayLine::Doc {
        doc_line: 1,
        old_line: Some(1),
        change: Some(ChangeKind::Added),
        hunk: Some(HunkState::Unstaged),
        group_id: Some(group_id.clone()),
        secondary: false,
      },
    ];
    let projection = Projection::from_lines(2, lines, HashMap::new(), None, None);
    let block_map = projection.block_map();

    assert_eq!(
      group_id_for_gutter_display_line(1, Some(&projection), &ProjectionBlockMap::default(), false),
      None
    );
    assert_eq!(
      group_id_for_gutter_display_line(1, Some(&projection), block_map, false).as_deref(),
      Some(group_id.as_ref())
    );
    assert_eq!(
      group_id_for_gutter_display_line(2, Some(&projection), block_map, false).as_deref(),
      Some(group_id.as_ref())
    );
    assert_eq!(
      group_id_for_gutter_display_line(3, Some(&projection), block_map, false).as_deref(),
      Some(group_id.as_ref())
    );
  }

  #[test]
  fn inactive_conflict_border_edges_wrap_the_whole_conflict() {
    assert_eq!(
      inactive_conflict_border_edges(
        None,
        ConflictLineKind::CurrentMarker,
        Some(ConflictLineKind::Current),
      ),
      (true, false)
    );
    assert_eq!(
      inactive_conflict_border_edges(
        Some(ConflictLineKind::Current),
        ConflictLineKind::BaseMarker,
        Some(ConflictLineKind::Base),
      ),
      (false, false)
    );
    assert_eq!(
      inactive_conflict_border_edges(
        Some(ConflictLineKind::Divider),
        ConflictLineKind::Incoming,
        Some(ConflictLineKind::IncomingMarker),
      ),
      (false, false)
    );
    assert_eq!(
      inactive_conflict_border_edges(
        Some(ConflictLineKind::Incoming),
        ConflictLineKind::IncomingMarker,
        None,
      ),
      (false, true)
    );
  }

  #[test]
  fn active_conflict_border_edges_wrap_the_whole_conflict() {
    let active_range = 2..9;
    assert_eq!(
      active_conflict_border_edges(Some(1), Some(2), Some(3), Some(&active_range)),
      Some((true, false))
    );
    assert_eq!(
      active_conflict_border_edges(Some(4), Some(5), Some(6), Some(&active_range)),
      Some((false, false))
    );
    assert_eq!(
      active_conflict_border_edges(Some(7), Some(8), Some(9), Some(&active_range)),
      Some((false, true))
    );
    assert_eq!(
      active_conflict_border_edges(Some(1), Some(2), Some(3), None),
      None
    );
    assert_eq!(
      active_conflict_border_edges(Some(1), Some(9), Some(10), Some(&active_range)),
      None
    );
  }
}
