//! One taffy node for a whole mini code/diff body: row bands, the number
//! gutter and its border are painted by hand instead of one div tree per
//! line, which is what made scrolling diff-heavy conversations expensive.

use gpui::{
  App, AvailableSpace, Bounds, Element, ElementId, GlobalElementId, Hsla, InspectorElementId,
  IntoElement, LayoutId, Pixels, SharedString, Size, TextRun, Window, WrappedLine, fill, point, px,
  relative, size,
};
use std::cell::RefCell;
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

pub(crate) struct CodeLines {
  rows: Rc<Vec<CodeLineRow>>,
  gutter_width: Pixels,
  gutter_color: Hsla,
  border_color: Hsla,
  default_color: Hsla,
  font: gpui::Font,
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
  size: Size<Pixels>,
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
      layout: Rc::new(RefCell::new(None)),
    }
  }

  fn has_gutter(&self) -> bool {
    self.gutter_width > px(0.)
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
  type PrepaintState = ();

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

    let layout_id = window.request_measured_layout(style, {
      move |known_dimensions, available_space, window, _cx| {
        let width = known_dimensions.width.or(match available_space.width {
          AvailableSpace::Definite(x) => Some(x),
          _ => None,
        });
        let wrap_width = width.map(|w| (w - gutter_width - CELL_PADDING_X * 2.).max(px(16.)));

        if let Some(cached) = layout.borrow().as_ref()
          && cached.wrap_width == wrap_width
        {
          return cached.size;
        }

        let mut laid_out_rows = Vec::with_capacity(rows.len());
        let mut total = Size::<Pixels>::default();
        for row in rows.iter() {
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
          let runs: Vec<TextRun> = if row.runs.is_empty() {
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
    _bounds: Bounds<Pixels>,
    _: &mut Self::RequestLayoutState,
    _window: &mut Window,
    _cx: &mut App,
  ) -> Self::PrepaintState {
  }

  fn paint(
    &mut self,
    _: Option<&GlobalElementId>,
    _: Option<&InspectorElementId>,
    bounds: Bounds<Pixels>,
    _: &mut Self::RequestLayoutState,
    _: &mut Self::PrepaintState,
    window: &mut Window,
    cx: &mut App,
  ) {
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
