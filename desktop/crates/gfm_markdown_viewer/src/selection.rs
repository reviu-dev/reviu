use std::{ops::Range, sync::Arc};

use gpui::{
  App, Bounds, ClipboardItem, CursorStyle, DispatchPhase, Element, ElementId, FontStyle,
  FontWeight, GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId, LayoutId,
  MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, SharedString,
  StrikethroughStyle, StyledText, TextAlign, TextRun, UnderlineStyle, Window, fill, point, px,
};
use gpui_component::ActiveTheme as _;
use syntax::SyntaxTheme;

use crate::constants::*;
use crate::gfm_markdown_viewer::{LinkHandlerFn, collect_indentation_indicators};
use crate::types::*;

pub(crate) struct SelectableText {
  text: SharedString,
  spans: Vec<InlineSpan>,
  link_ranges: Vec<LinkRange>,
  render_state: MarkdownRenderState,
  on_link: Option<Arc<LinkHandlerFn>>,
  text_id: usize,
  interactive: bool,
  show_indentation_dots: bool,
  show_inline_code_backgrounds: bool,
  indentation_dot_indices: Vec<usize>,
  indentation_tab_indices: Vec<usize>,
  styled_text: StyledText,
  runs_initialized: bool,
  last_selection: Option<Range<usize>>,
}

#[derive(Clone, Copy)]
pub(crate) struct SelectableTextOptions {
  pub interactive: bool,
  pub show_indentation_dots: bool,
  pub show_inline_code_backgrounds: bool,
}

impl SelectableText {
  pub(crate) fn new(
    text: SharedString,
    spans: Vec<InlineSpan>,
    link_ranges: Vec<LinkRange>,
    render_state: MarkdownRenderState,
    on_link: Option<Arc<LinkHandlerFn>>,
    text_id: usize,
    options: SelectableTextOptions,
  ) -> Self {
    let styled_text = StyledText::new(text.clone());
    let indicators = if options.show_indentation_dots {
      collect_indentation_indicators(text.as_ref())
    } else {
      crate::gfm_markdown_viewer::IndentationIndicators {
        dot_indices: Vec::new(),
        tab_indices: Vec::new(),
      }
    };
    Self {
      text,
      spans,
      link_ranges,
      render_state,
      on_link,
      text_id,
      interactive: options.interactive,
      show_indentation_dots: options.show_indentation_dots,
      show_inline_code_backgrounds: options.show_inline_code_backgrounds,
      indentation_dot_indices: indicators.dot_indices,
      indentation_tab_indices: indicators.tab_indices,
      styled_text,
      runs_initialized: false,
      last_selection: None,
    }
  }

  fn ensure_runs_up_to_date(
    &mut self,
    selection_range: Option<Range<usize>>,
    window: &mut Window,
    cx: &mut App,
  ) {
    if self.runs_initialized && selection_range == self.last_selection {
      return;
    }

    let runs = build_runs(&self.spans, selection_range.clone(), window, cx);
    self.styled_text = StyledText::new(self.text.clone()).with_runs(runs);
    self.last_selection = selection_range;
    self.runs_initialized = true;
  }

  fn paint_indentation_dots(
    &self,
    text_layout: &gpui::TextLayout,
    window: &mut Window,
    cx: &mut App,
  ) {
    if !self.show_indentation_dots || self.indentation_dot_indices.is_empty() {
      return;
    }

    let text_len = self.text.len();
    let dot_size = px(MARKDOWN_CODE_INDENT_DOT_SIZE_PX);
    let dot_radius = dot_size / 2.;
    let line_height = text_layout.line_height();
    let min_spacing = px(MARKDOWN_CODE_INDENT_DOT_MIN_SPACING_PX);
    let dot_color = cx
      .theme()
      .muted_foreground
      .opacity(MARKDOWN_CODE_INDENT_DOT_OPACITY);
    let mut last_drawn: Option<(usize, Pixels)> = None;

    for &ix in &self.indentation_dot_indices {
      if ix + 1 > text_len {
        continue;
      }
      let Some(start) = text_layout.position_for_index(ix) else {
        continue;
      };
      let Some(end) = text_layout.position_for_index(ix + 1) else {
        continue;
      };
      let cell_width = end.x - start.x;
      if cell_width <= px(0.) {
        continue;
      }

      let dot_center_x = start.x + cell_width / 2.;
      if let Some((last_ix, last_center_x)) = last_drawn
        && ix == last_ix + 1
        && dot_center_x - last_center_x < min_spacing
      {
        continue;
      }

      let dot_x = dot_center_x - dot_size / 2.;
      let dot_y = start.y + (line_height - dot_size) / 2.;
      window.paint_quad(
        fill(
          Bounds::from_corners(
            point(dot_x, dot_y),
            point(dot_x + dot_size, dot_y + dot_size),
          ),
          dot_color,
        )
        .corner_radii(dot_radius),
      );
      last_drawn = Some((ix, dot_center_x));
    }
  }

  fn paint_indentation_tab_arrows(
    &self,
    text_layout: &gpui::TextLayout,
    window: &mut Window,
    cx: &mut App,
  ) {
    if !self.show_indentation_dots || self.indentation_tab_indices.is_empty() {
      return;
    }

    let text_len = self.text.len();
    let line_height = text_layout.line_height();
    let font_size = cx.theme().mono_font_size;
    let arrow_color = cx
      .theme()
      .muted_foreground
      .opacity(MARKDOWN_CODE_INDENT_TAB_ARROW_OPACITY);

    let runs = vec![TextRun {
      len: "→".len(),
      font: gpui::Font {
        family: cx.theme().font_family.clone(),
        style: FontStyle::Normal,
        weight: FontWeight::NORMAL,
        ..Default::default()
      },
      color: arrow_color,
      background_color: None,
      underline: None,
      strikethrough: None,
    }];

    let shaped = window
      .text_system()
      .shape_line("→".into(), font_size, &runs, None);

    for &ix in &self.indentation_tab_indices {
      if ix + 1 > text_len {
        continue;
      }
      let Some(start) = text_layout.position_for_index(ix) else {
        continue;
      };
      let Some(end) = text_layout.position_for_index(ix + 1) else {
        continue;
      };
      let tab_width = end.x - start.x;
      if tab_width <= px(0.) {
        continue;
      }

      let arrow_x = start.x + (tab_width - shaped.width) / 2.;
      let origin = point(arrow_x, start.y);
      let _ = shaped.paint(origin, line_height, TextAlign::Left, None, window, cx);
    }
  }

  fn paint_inline_code_backgrounds(
    &self,
    text_layout: &gpui::TextLayout,
    window: &mut Window,
    cx: &mut App,
  ) {
    if !self.show_inline_code_backgrounds {
      return;
    }
    let line_height = text_layout.line_height();
    let theme = cx.theme();
    let bg_color = if theme.mode.is_dark() {
      theme.muted
    } else {
      theme.muted_foreground.opacity(0.1)
    };
    let h_pad = px(5.);
    let radius = px(6.);
    // Shrink height inside line_height to match GitHub's compact code background
    let v_inset = px(3.);

    for span in &self.spans {
      if !span.style.code || span.range.is_empty() {
        continue;
      }
      let Some(start_pos) = text_layout.position_for_index(span.range.start) else {
        continue;
      };
      let Some(end_pos) = text_layout.position_for_index(span.range.end) else {
        continue;
      };

      // Single-line case only (multi-line inline code is rare)
      if (end_pos.y - start_pos.y).abs() < px(1.) {
        let rect = Bounds::from_corners(
          point(start_pos.x - h_pad, start_pos.y + v_inset),
          point(end_pos.x + h_pad, start_pos.y + line_height - v_inset),
        );
        window.paint_quad(fill(rect, bg_color).corner_radii(radius));
      }
    }
  }
}

impl Element for SelectableText {
  type RequestLayoutState = ();
  type PrepaintState = Hitbox;

  fn id(&self) -> Option<ElementId> {
    None
  }

  fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
    None
  }

  fn request_layout(
    &mut self,
    _: Option<&GlobalElementId>,
    inspector_id: Option<&InspectorElementId>,
    window: &mut Window,
    cx: &mut App,
  ) -> (LayoutId, Self::RequestLayoutState) {
    let selection_range = selection_for_text(&self.render_state, self.text_id, &self.text);
    self.ensure_runs_up_to_date(selection_range, window, cx);
    let (layout_id, _) = self
      .styled_text
      .request_layout(None, inspector_id, window, cx);
    (layout_id, ())
  }

  fn prepaint(
    &mut self,
    _global_id: Option<&GlobalElementId>,
    inspector_id: Option<&InspectorElementId>,
    bounds: Bounds<Pixels>,
    state: &mut Self::RequestLayoutState,
    window: &mut Window,
    cx: &mut App,
  ) -> Self::PrepaintState {
    let selection_range = selection_for_text(&self.render_state, self.text_id, &self.text);
    self.ensure_runs_up_to_date(selection_range, window, cx);
    self
      .styled_text
      .prepaint(None, inspector_id, bounds, state, window, cx);
    window.insert_hitbox(bounds, HitboxBehavior::Normal)
  }

  fn paint(
    &mut self,
    _global_id: Option<&GlobalElementId>,
    inspector_id: Option<&InspectorElementId>,
    bounds: Bounds<Pixels>,
    _: &mut Self::RequestLayoutState,
    hitbox: &mut Hitbox,
    window: &mut Window,
    cx: &mut App,
  ) {
    if !self.interactive {
      let text_layout = self.styled_text.layout().clone();
      self.paint_inline_code_backgrounds(&text_layout, window, cx);
      self
        .styled_text
        .paint(None, inspector_id, bounds, &mut (), &mut (), window, cx);
      self.paint_indentation_dots(&text_layout, window, cx);
      self.paint_indentation_tab_arrows(&text_layout, window, cx);
      return;
    }

    let text_layout = self.styled_text.layout().clone();
    self.paint_inline_code_backgrounds(&text_layout, window, cx);
    let link_ranges = self.link_ranges.clone();
    let on_link = self.on_link.clone();
    let render_state = self.render_state.clone();
    let text_id = self.text_id;
    let text_len = self.text.len();
    let text_for_selection = self.text.clone();
    let text_for_hover = self.text.clone();
    let text_for_down = self.text.clone();
    let text_for_move = self.text.clone();
    let text_for_up = self.text.clone();
    let layout_for_down = text_layout.clone();
    let layout_for_move = text_layout.clone();
    let layout_for_up = text_layout.clone();

    if hitbox.is_hovered(window) && {
      let index = clamp_to_char_boundary(
        text_for_hover.as_ref(),
        text_layout
          .index_for_position(window.mouse_position())
          .unwrap_or_else(|ix| ix)
          .min(text_len),
      );
      link_ranges.iter().any(|range| range.range.contains(&index))
    } {
      window.set_cursor_style(CursorStyle::PointingHand, hitbox);
    }

    window.on_mouse_event({
      let hitbox = hitbox.clone();
      let render_state = render_state.clone();
      move |event: &MouseDownEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble
          || event.button != MouseButton::Left
          || !hitbox.is_hovered(window)
        {
          return;
        }

        let index = clamp_to_char_boundary(
          text_for_down.as_ref(),
          layout_for_down
            .index_for_position(event.position)
            .unwrap_or_else(|ix| ix)
            .min(text_len),
        );
        update_selection_state(&render_state, text_id, index, index, true);
        window.refresh();
        cx.stop_propagation();
      }
    });

    window.on_mouse_event({
      let hitbox = hitbox.clone();
      let render_state = render_state.clone();
      move |event: &MouseMoveEvent, phase, window, _cx| {
        if phase != DispatchPhase::Bubble {
          return;
        }
        let current = selection_state_for(&render_state, text_id);
        if current.dragging {
          let index = clamp_to_char_boundary(
            text_for_move.as_ref(),
            layout_for_move
              .index_for_position(event.position)
              .unwrap_or_else(|ix| ix)
              .min(text_len),
          );
          let anchor = current.anchor.unwrap_or(index);
          update_selection_state(&render_state, text_id, anchor, index, true);
          window.refresh();
          return;
        }

        if !hitbox.is_hovered(window) {
          return;
        }

        window.refresh();
      }
    });

    window.on_mouse_event({
      let hitbox = hitbox.clone();
      let render_state = render_state.clone();
      let link_ranges = link_ranges.clone();
      let on_link = on_link.clone();
      move |event: &MouseUpEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble {
          return;
        }

        let index = clamp_to_char_boundary(
          text_for_up.as_ref(),
          layout_for_up
            .index_for_position(event.position)
            .unwrap_or_else(|ix| ix)
            .min(text_len),
        );
        let current = selection_state_for(&render_state, text_id);
        if !current.dragging {
          return;
        }

        update_selection_state(
          &render_state,
          text_id,
          current.anchor.unwrap_or(index),
          index,
          false,
        );

        let updated = selection_state_for(&render_state, text_id);
        if updated.range.is_empty() {
          if hitbox.is_hovered(window)
            && let Some(link) = link_ranges
              .iter()
              .find(|range| range.range.contains(&index))
              .map(|range| range.url.clone())
          {
            let handled = on_link
              .as_ref()
              .map(|handler| handler(link.as_ref(), window, cx))
              .unwrap_or(LinkAction::Open);
            if handled == LinkAction::Open {
              cx.open_url(link.as_ref());
            }
          }
        } else if let Some(text) = selection_text(&render_state, text_id, &text_for_selection) {
          cx.write_to_clipboard(ClipboardItem::new_string(text));
        }

        window.refresh();
      }
    });

    self
      .styled_text
      .paint(None, inspector_id, bounds, &mut (), &mut (), window, cx);
    self.paint_indentation_dots(&text_layout, window, cx);
  }
}

impl gpui::IntoElement for SelectableText {
  type Element = Self;

  fn into_element(self) -> Self::Element {
    self
  }
}

pub(crate) fn selection_state_for(state: &MarkdownRenderState, text_id: usize) -> SelectionState {
  let selection = state.selection.lock().unwrap();
  if let Some(active) = selection.as_ref()
    && active.text_id == text_id
  {
    return SelectionState {
      anchor: Some(active.anchor),
      range: SelectionRange {
        start: active.anchor,
        end: active.head,
      },
      dragging: active.dragging,
    };
  }
  SelectionState::default()
}

pub(crate) fn update_selection_state(
  state: &MarkdownRenderState,
  text_id: usize,
  anchor: usize,
  head: usize,
  dragging: bool,
) {
  let mut selection = state.selection.lock().unwrap();
  *selection = Some(ActiveSelection {
    text_id,
    anchor,
    head,
    dragging,
  });
}

pub(crate) fn selection_for_text(
  state: &MarkdownRenderState,
  text_id: usize,
  text: &SharedString,
) -> Option<Range<usize>> {
  let text = text.as_ref();
  let text_len = text.len();
  let selection = state.selection.lock().unwrap();
  let active = selection.as_ref()?;
  if active.text_id != text_id || active.anchor == active.head {
    return None;
  }
  let mut range = SelectionRange {
    start: active.anchor,
    end: active.head,
  }
  .normalized();
  range.start = clamp_to_char_boundary(text, range.start.min(text_len));
  range.end = clamp_to_char_boundary(text, range.end.min(text_len));
  if range.start >= range.end {
    None
  } else {
    Some(range)
  }
}

pub(crate) fn selection_text(
  state: &MarkdownRenderState,
  text_id: usize,
  text: &SharedString,
) -> Option<String> {
  let selection = selection_for_text(state, text_id, text)?;
  text.as_ref().get(selection).map(|value| value.to_string())
}

pub(crate) fn build_runs(
  spans: &[InlineSpan],
  selection: Option<Range<usize>>,
  window: &mut Window,
  cx: &mut App,
) -> Vec<TextRun> {
  let base_style = window.text_style();
  let base_font = base_style.font().clone();
  let base_color = base_style.color;
  let theme = cx.theme();
  let link_color = github_link_color(theme.background);
  let syntax_theme = syntax_theme_for_background(theme.background);

  let mut runs = Vec::new();
  let code_font_family = theme.mono_font_family.clone();
  for span in spans {
    let mut font = base_font.clone();
    if span.style.code {
      font.family = code_font_family.clone();
    }
    if span.style.bold {
      font.weight = FontWeight::BOLD;
    }
    if span.style.italic {
      font.style = FontStyle::Italic;
    }

    let mut color = base_color;
    let mut underline = None;
    if let Some(token_type) = span.syntax_token {
      color = syntax_theme.color_for_token(token_type);
    }
    if span.link.is_some() {
      color = link_color;
      underline = Some(UnderlineStyle {
        thickness: px(1.0),
        color: Some(link_color),
        wavy: false,
      });
    }

    let strikethrough = if span.style.strike {
      Some(StrikethroughStyle {
        thickness: px(1.0),
        color: Some(color),
      })
    } else {
      None
    };

    runs.push(TextRun {
      len: span.range.end.saturating_sub(span.range.start),
      font,
      color,
      background_color: inline_span_background(span.background, cx),
      underline,
      strikethrough,
    });
  }

  if let Some(selection) = selection {
    apply_selection_to_runs(runs, selection, theme.selection)
  } else {
    runs
  }
}

fn inline_span_background(background: Option<InlineBackground>, cx: &App) -> Option<Hsla> {
  let ui_theme = ui::Theme::new(cx.theme().is_dark());
  match background {
    Some(InlineBackground::DiffWordAdded) => Some(ui_theme.diff_word_added_background()),
    Some(InlineBackground::DiffWordRemoved) => Some(ui_theme.diff_word_removed_background()),
    None => None,
  }
}

pub(crate) fn github_link_color(background: Hsla) -> Hsla {
  if background.l < 0.5 {
    Hsla {
      h: 212.0 / 360.0,
      s: 1.0,
      l: 0.67,
      a: 1.0,
    }
  } else {
    Hsla {
      h: 212.0 / 360.0,
      s: 0.92,
      l: 0.45,
      a: 1.0,
    }
  }
}

pub(crate) fn syntax_theme_for_background(background: Hsla) -> SyntaxTheme {
  if background.l < 0.5 {
    SyntaxTheme::default_dark()
  } else {
    SyntaxTheme::default_light()
  }
}

pub(crate) fn apply_selection_to_runs(
  runs: Vec<TextRun>,
  selection: Range<usize>,
  selection_color: Hsla,
) -> Vec<TextRun> {
  let mut updated = Vec::new();
  let mut offset = 0usize;
  for run in runs {
    let run_start = offset;
    let run_end = offset + run.len;
    offset = run_end;

    if selection.end <= run_start || selection.start >= run_end {
      updated.push(run);
      continue;
    }

    let overlap_start = selection.start.max(run_start);
    let overlap_end = selection.end.min(run_end);

    if overlap_start > run_start {
      let mut prefix = run.clone();
      prefix.len = overlap_start - run_start;
      updated.push(prefix);
    }

    let mut selected = run.clone();
    selected.len = overlap_end - overlap_start;
    selected.background_color = Some(selection_color);
    updated.push(selected);

    if overlap_end < run_end {
      let mut suffix = run.clone();
      suffix.len = run_end - overlap_end;
      updated.push(suffix);
    }
  }
  updated
}
