//! One taffy node for a whole mini code/diff body: row bands, the number
//! gutter and its border are painted by hand instead of one div tree per
//! line, which is what made scrolling diff-heavy conversations expensive.

use gpui::{
  App, AvailableSpace, Bounds, ClipboardItem, CursorStyle, DispatchPhase, Element, ElementId,
  GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId, IntoElement, LayoutId,
  MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, SharedString, Size,
  TextRun, Window, WrappedLine, fill, point, px, relative, size,
};
use gpui_component::ActiveTheme as _;
use selectable_text::{
  SelectionMode, SelectionRegistry, apply_selection_to_runs, clamp_to_char_boundary,
  extend_selection, line_range_at, mode_for_click_count, selection_range, word_range_at,
};
use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

const CELL_PADDING_X: Pixels = px(8.);

pub(crate) struct CodeLineRow {
  /// Pre-padded, monospace-aligned line numbers; None when the block has no gutter.
  pub gutter: Option<SharedString>,
  pub text: SharedString,
  /// Styled runs covering `text`; empty means one default run.
  pub runs: Vec<TextRun>,
  /// Full-width row background (diff band / block background).
  pub band: Hsla,
}

/// Wires the block into the app-wide selection registry: `text` is the source
/// the rows were sliced from, `row_ranges[i]` is row i's byte range in it.
pub(crate) struct SelectionSpec {
  pub text: SharedString,
  pub row_ranges: Vec<Range<usize>>,
  pub text_id: u64,
  pub registry: SelectionRegistry,
}

pub(crate) struct CodeLines {
  rows: Rc<Vec<CodeLineRow>>,
  gutter_width: Pixels,
  gutter_color: Hsla,
  border_color: Hsla,
  default_color: Hsla,
  font: gpui::Font,
  selection: Option<Rc<SelectionSpec>>,
  /// false = terminal-style: lines clip at the edge instead of wrapping.
  wrap: bool,
  layout: Rc<RefCell<Option<CodeLinesLayout>>>,
}

struct RowLayout {
  gutter: Option<WrappedLine>,
  lines: Vec<WrappedLine>,
  height: Pixels,
}

struct CodeLinesLayout {
  rows: Vec<RowLayout>,
  line_height: Pixels,
  wrap_width: Option<Pixels>,
  selection: Option<Range<usize>>,
  size: Size<Pixels>,
}

#[cfg(test)]
pub(crate) struct CodeLinesProbe(Rc<RefCell<Option<CodeLinesLayout>>>);

#[cfg(test)]
impl CodeLinesProbe {
  pub(crate) fn row_heights(&self) -> Vec<f32> {
    self
      .0
      .borrow()
      .as_ref()
      .map(|layout| {
        layout
          .rows
          .iter()
          .map(|row| f32::from(row.height))
          .collect()
      })
      .unwrap_or_default()
  }
}

/// Byte index into the selection source for a window-space position.
fn index_for_position(
  layout: &CodeLinesLayout,
  spec: &SelectionSpec,
  bounds: Bounds<Pixels>,
  text_origin_x: Pixels,
  position: Point<Pixels>,
) -> usize {
  let mut y = bounds.origin.y;
  for (row_layout, range) in layout.rows.iter().zip(&spec.row_ranges) {
    let row_bottom = y + row_layout.height;
    if position.y < row_bottom {
      let mut line_origin = point(text_origin_x, y);
      for line in &row_layout.lines {
        let line_bottom = line_origin.y + line.size(layout.line_height).height;
        if position.y < line_bottom {
          let within = position - line_origin;
          let ix = match line.index_for_position(within, layout.line_height) {
            Ok(ix) | Err(ix) => ix,
          };
          return (range.start + ix).min(range.end);
        }
        line_origin.y = line_bottom;
      }
      return range.end;
    }
    y = row_bottom;
  }
  spec.row_ranges.last().map(|r| r.end).unwrap_or(0)
}

impl CodeLines {
  pub(crate) fn new(
    rows: Vec<CodeLineRow>,
    gutter_width: Pixels,
    gutter_color: Hsla,
    border_color: Hsla,
    default_color: Hsla,
    font: gpui::Font,
  ) -> Self {
    Self {
      rows: Rc::new(rows),
      gutter_width,
      gutter_color,
      border_color,
      default_color,
      font,
      selection: None,
      wrap: true,
      layout: Rc::new(RefCell::new(None)),
    }
  }

  pub(crate) fn no_wrap(mut self) -> Self {
    self.wrap = false;
    self
  }

  pub(crate) fn selectable(mut self, spec: SelectionSpec) -> Self {
    self.selection = Some(Rc::new(spec));
    self
  }

  fn has_gutter(&self) -> bool {
    self.gutter_width > px(0.)
  }

  /// Test hook: the computed per-row heights survive the element being moved
  /// into a draw call through the shared layout handle.
  #[cfg(test)]
  pub(crate) fn probe(&self) -> CodeLinesProbe {
    CodeLinesProbe(self.layout.clone())
  }

  fn resolved_selection(&self) -> Option<Range<usize>> {
    let spec = self.selection.as_ref()?;
    let active = spec.registry.active_for(spec.text_id)?;
    selection_range(&active, spec.text.as_ref())
  }

  /// Same drag/word/line/copy behavior as SelectableText, mapped through the
  /// block's per-row layout.
  fn paint_selection_handlers(
    &self,
    hitbox: &Hitbox,
    spec: Rc<SelectionSpec>,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    _cx: &mut App,
  ) {
    if hitbox.is_hovered(window) {
      window.set_cursor_style(CursorStyle::IBeam, hitbox);
    }
    let text_origin_x = bounds.origin.x + self.gutter_width + CELL_PADDING_X;
    let layout = self.layout.clone();

    let index_at = move |layout: &Rc<RefCell<Option<CodeLinesLayout>>>,
                         spec: &SelectionSpec,
                         position: Point<Pixels>| {
      let borrowed = layout.borrow();
      let computed = borrowed.as_ref()?;
      Some(clamp_to_char_boundary(
        spec.text.as_ref(),
        index_for_position(computed, spec, bounds, text_origin_x, position),
      ))
    };

    let hitbox_down = hitbox.clone();
    let spec_down = spec.clone();
    let layout_down = layout.clone();
    let index_down = index_at;
    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
      if phase != DispatchPhase::Bubble
        || event.button != MouseButton::Left
        || !hitbox_down.is_hovered(window)
      {
        return;
      }
      let Some(index) = index_down(&layout_down, &spec_down, event.position) else {
        return;
      };
      let mode = mode_for_click_count(event.click_count);
      let text = spec_down.text.as_ref();
      let anchor_word = match mode {
        SelectionMode::Word => word_range_at(text, index),
        SelectionMode::Line => Some(line_range_at(text, index)),
        SelectionMode::Character => None,
      };
      let (start, end) = extend_selection(text, mode, anchor_word.clone(), index, index);
      spec_down
        .registry
        .set(spec_down.text_id, start, end, true, mode, anchor_word);
      window.refresh();
      cx.stop_propagation();
    });

    let spec_move = spec.clone();
    let layout_move = layout.clone();
    let index_move = index_at;
    window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, _cx| {
      if phase != DispatchPhase::Bubble {
        return;
      }
      let Some(active) = spec_move.registry.active_for(spec_move.text_id) else {
        return;
      };
      if !active.dragging {
        return;
      }
      let Some(index) = index_move(&layout_move, &spec_move, event.position) else {
        return;
      };
      let (start, end) = extend_selection(
        spec_move.text.as_ref(),
        active.mode,
        active.anchor_word.clone(),
        active.anchor,
        index,
      );
      spec_move.registry.set(
        spec_move.text_id,
        start,
        end,
        true,
        active.mode,
        active.anchor_word,
      );
      window.refresh();
    });

    let spec_up = spec.clone();
    let layout_up = layout.clone();
    let index_up = index_at;
    window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
      if phase != DispatchPhase::Bubble {
        return;
      }
      let Some(active) = spec_up.registry.active_for(spec_up.text_id) else {
        return;
      };
      if !active.dragging {
        return;
      }
      let Some(index) = index_up(&layout_up, &spec_up, event.position) else {
        return;
      };
      let text = spec_up.text.as_ref();
      let (start, end) = extend_selection(
        text,
        active.mode,
        active.anchor_word.clone(),
        active.anchor,
        index,
      );
      spec_up.registry.set(
        spec_up.text_id,
        start,
        end,
        false,
        active.mode,
        active.anchor_word,
      );
      if let Some(active) = spec_up.registry.active_for(spec_up.text_id)
        && let Some(range) = selection_range(&active, text)
        && let Some(selected) = text.get(range)
      {
        cx.write_to_clipboard(ClipboardItem::new_string(selected.to_string()));
      }
      window.refresh();
    });
  }
}

impl IntoElement for CodeLines {
  type Element = Self;

  fn into_element(self) -> Self::Element {
    self
  }
}

impl Element for CodeLines {
  type RequestLayoutState = ();
  type PrepaintState = Option<Hitbox>;

  fn id(&self) -> Option<ElementId> {
    None
  }

  fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
    None
  }

  fn request_layout(
    &mut self,
    _: Option<&GlobalElementId>,
    _: Option<&InspectorElementId>,
    window: &mut Window,
    _cx: &mut App,
  ) -> (LayoutId, Self::RequestLayoutState) {
    let text_style = window.text_style();
    let font_size = text_style.font_size.to_pixels(window.rem_size());
    let line_height = window.pixel_snap(
      text_style
        .line_height
        .to_pixels(font_size.into(), window.rem_size()),
    );

    let mut style = gpui::Style::default();
    style.size.width = relative(1.).into();

    let rows = self.rows.clone();
    let gutter_width = self.gutter_width;
    let gutter_color = self.gutter_color;
    let default_color = self.default_color;
    let font = self.font.clone();
    let layout = self.layout.clone();
    let wrap = self.wrap;
    let selection = self.resolved_selection();
    let selection_bg = _cx.theme().selection;
    let row_ranges = self.selection.as_ref().map(|spec| spec.row_ranges.clone());

    let layout_id = window.request_measured_layout(style, {
      move |known_dimensions, available_space, window, _cx| {
        let width = known_dimensions.width.or(match available_space.width {
          AvailableSpace::Definite(x) => Some(x),
          _ => None,
        });
        let wrap_width = if wrap {
          width.map(|w| (w - gutter_width - CELL_PADDING_X * 2.).max(px(16.)))
        } else {
          None
        };

        if let Some(cached) = layout.borrow().as_ref()
          && cached.wrap_width == wrap_width
          && cached.selection == selection
        {
          return cached.size;
        }

        let mut laid_out_rows = Vec::with_capacity(rows.len());
        let mut total = Size::<Pixels>::default();
        for (row_ix, row) in rows.iter().enumerate() {
          let gutter = row.gutter.as_ref().and_then(|numbers| {
            let run = TextRun {
              len: numbers.len(),
              font: font.clone(),
              color: gutter_color,
              background_color: None,
              underline: None,
              strikethrough: None,
            };
            window
              .text_system()
              .shape_text(numbers.clone(), font_size, &[run], None, None)
              .ok()
              .and_then(|mut lines| (!lines.is_empty()).then(|| lines.remove(0)))
          });
          let mut runs: Vec<TextRun> = if row.runs.is_empty() {
            vec![TextRun {
              len: row.text.len(),
              font: font.clone(),
              color: default_color,
              background_color: None,
              underline: None,
              strikethrough: None,
            }]
          } else {
            row.runs.clone()
          };
          // The active selection lands as background runs, row-local offsets.
          if let (Some(selection), Some(ranges)) = (&selection, &row_ranges)
            && let Some(range) = ranges.get(row_ix)
            && selection.start < range.end.min(range.start + row.text.len())
            && selection.end > range.start
          {
            let local = selection.start.saturating_sub(range.start)
              ..(selection.end - range.start).min(row.text.len());
            if local.start < local.end {
              runs = apply_selection_to_runs(runs, local, selection_bg);
            }
          }
          let lines: Vec<WrappedLine> = window
            .text_system()
            .shape_text(row.text.clone(), font_size, &runs, wrap_width, None)
            .map(|lines| lines.into_vec())
            .unwrap_or_default();
          let mut height = px(0.);
          let mut width = gutter_width + CELL_PADDING_X * 2.;
          for line in &lines {
            let line_size = line.size(line_height);
            height += line_size.height;
            width = (width + line_size.width).max(width);
          }
          height = height.max(line_height);
          total.height += height;
          total.width = total.width.max(width);
          laid_out_rows.push(RowLayout {
            gutter,
            lines,
            height,
          });
        }
        if let Some(w) = width {
          total.width = w;
        }
        let computed = CodeLinesLayout {
          rows: laid_out_rows,
          line_height,
          wrap_width,
          selection: selection.clone(),
          size: total,
        };
        let size = computed.size;
        layout.borrow_mut().replace(computed);
        size
      }
    });
    (layout_id, ())
  }

  fn prepaint(
    &mut self,
    _: Option<&GlobalElementId>,
    _: Option<&InspectorElementId>,
    bounds: Bounds<Pixels>,
    _: &mut Self::RequestLayoutState,
    window: &mut Window,
    _cx: &mut App,
  ) -> Self::PrepaintState {
    self
      .selection
      .is_some()
      .then(|| window.insert_hitbox(bounds, HitboxBehavior::Normal))
  }

  fn paint(
    &mut self,
    _: Option<&GlobalElementId>,
    _: Option<&InspectorElementId>,
    bounds: Bounds<Pixels>,
    _: &mut Self::RequestLayoutState,
    hitbox: &mut Self::PrepaintState,
    window: &mut Window,
    cx: &mut App,
  ) {
    if let (Some(hitbox), Some(spec)) = (hitbox.as_ref(), self.selection.clone()) {
      self.paint_selection_handlers(hitbox, spec, bounds, window, cx);
    }
    let layout = self.layout.borrow();
    let Some(layout) = layout.as_ref() else {
      return;
    };
    let line_height = layout.line_height;
    let text_align = window.text_style().text_align;

    let mut y = bounds.origin.y;
    for (row, row_layout) in self.rows.iter().zip(&layout.rows) {
      let row_bounds = Bounds::new(
        point(bounds.origin.x, y),
        size(bounds.size.width, row_layout.height),
      );
      window.paint_quad(fill(row_bounds, row.band));

      if let Some(gutter) = &row_layout.gutter {
        let _ = gutter.paint(
          point(bounds.origin.x + CELL_PADDING_X, y),
          line_height,
          text_align,
          Some(row_bounds),
          window,
          cx,
        );
      }

      let mut line_origin = point(bounds.origin.x + self.gutter_width + CELL_PADDING_X, y);
      for line in &row_layout.lines {
        let _ = line.paint_background(
          line_origin,
          line_height,
          text_align,
          Some(row_bounds),
          window,
          cx,
        );
        let _ = line.paint(
          line_origin,
          line_height,
          text_align,
          Some(row_bounds),
          window,
          cx,
        );
        line_origin.y += line.size(line_height).height;
      }
      y += row_layout.height;
    }

    if self.has_gutter() {
      let border = Bounds::new(
        point(bounds.origin.x + self.gutter_width, bounds.origin.y),
        size(px(1.), bounds.size.height),
      );
      window.paint_quad(fill(border, self.border_color));
    }
  }
}
