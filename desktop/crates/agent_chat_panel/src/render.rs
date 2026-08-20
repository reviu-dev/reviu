//! The item renderers and the composer: everything the panel paints.

use super::*;

/// Wall-clock pulse for live indicators. Sampled at render time so it rides
/// the repaints the panel already does (stream commits, ticks); a repeating
/// gpui Animation would request a frame every vsync and redraw the whole
/// window at display rate for the entire turn.
pub(crate) fn pulse_opacity(min: f32, max: f32) -> f32 {
  const PERIOD_MS: f32 = 1400.0;
  let now_ms = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .unwrap_or_default()
    .as_millis() as f64;
  let phase = ((now_ms % PERIOD_MS as f64) / PERIOD_MS as f64) as f32;
  let wave = (phase * std::f32::consts::TAU).sin() * 0.5 + 0.5;
  min + (max - min) * wave
}

pub(crate) fn render_selector_item(
  name: SharedString,
  description: Option<SharedString>,
  is_current: bool,
  cx: &gpui::App,
) -> gpui::AnyElement {
  let theme = cx.theme().clone();
  let has_description = description.is_some();
  h_flex()
    .w_full()
    .max_w(px(360.))
    .gap_2()
    .map(|this| {
      if has_description {
        this.items_start()
      } else {
        this.items_center()
      }
    })
    .child(
      v_flex()
        .flex_1()
        .min_w_0()
        .gap_0p5()
        .child(
          div()
            .text_sm()
            .whitespace_nowrap()
            .overflow_hidden()
            .text_ellipsis()
            .when(is_current, |this| this.font_weight(gpui::FontWeight::BOLD))
            .child(name),
        )
        .when_some(description, |this, d| {
          this.child(
            div()
              .text_xs()
              .whitespace_nowrap()
              .overflow_hidden()
              .text_ellipsis()
              .text_color(theme.muted_foreground)
              .child(d),
          )
        }),
    )
    .when(is_current, |this| {
      this.child(
        gpui_component::Icon::new(UiIconName::Check)
          .small()
          .text_color(theme.foreground),
      )
    })
    .into_any_element()
}

/// The streaming variant: the caller owns the state and appends via push_str.
pub(crate) fn stateful_markdown_view(
  state: &gpui::Entity<gpui_component::text::TextViewState>,
  extensions: &gpui_component::text::MarkdownExtensions,
  cx: &App,
) -> gpui::AnyElement {
  let theme = cx.theme();
  let mut style = TextViewStyle::default().paragraph_gap(gpui::rems(0.5));
  style.highlight_theme = theme.highlight_theme.clone();
  style.is_dark = theme.mode.is_dark();

  TextView::new(state)
    .style(style)
    .markdown_extensions(extensions.clone())
    .selectable(true)
    .text_sm()
    .into_any_element()
}

pub(crate) fn markdown_view(
  id: impl Into<gpui::ElementId>,
  source: &str,
  extensions: &gpui_component::text::MarkdownExtensions,
  cx: &App,
) -> gpui::AnyElement {
  let theme = cx.theme();
  let mut style = TextViewStyle::default().paragraph_gap(gpui::rems(0.5));
  style.highlight_theme = theme.highlight_theme.clone();
  style.is_dark = theme.mode.is_dark();

  TextView::markdown(id, SharedString::from(source.to_string()))
    .style(style)
    .markdown_extensions(extensions.clone())
    .selectable(true)
    // Body text inherits from here; headings scale off `heading_base_font_size`.
    .text_sm()
    .into_any_element()
}

pub(crate) fn timeline_row(
  content: gpui::AnyElement,
  theme: &gpui_component::Theme,
  is_last: bool,
) -> gpui::AnyElement {
  timeline_row_with_color(content, theme, theme.muted_foreground, is_last)
}

pub(crate) fn timeline_row_with_color(
  content: gpui::AnyElement,
  _theme: &gpui_component::Theme,
  _bullet_color: gpui::Hsla,
  _is_last: bool,
) -> gpui::AnyElement {
  // Flat rows: the rail-and-bullet chrome read as noise, not structure.
  div()
    .w_full()
    .min_w_0()
    .pb_3()
    .child(content)
    .into_any_element()
}

/// The bold kind label and the detail line of a tool header. The label hides
/// when the title itself opens with the verb: "Editing files" needs no "Edit".
pub(crate) fn tool_header_parts(t: &ToolCallView) -> (Option<&'static str>, String) {
  let kind = tool_kind_label(&t.kind);
  if let Some((path, line)) = t.locations.first() {
    let name = path
      .file_name()
      .and_then(|s| s.to_str())
      .unwrap_or_else(|| path.to_str().unwrap_or(""));
    let detail = match line {
      Some(l) => format!("{name} (line {l})"),
      None => name.to_string(),
    };
    return (Some(kind), detail);
  }
  match t.title.strip_prefix(kind) {
    Some(rest) if rest.is_empty() || rest.starts_with(char::is_whitespace) => {
      (Some(kind), rest.trim_start().to_string())
    }
    Some(_) => (None, t.title.clone()),
    None => (Some(kind), t.title.clone()),
  }
}

pub(crate) fn strip_markdown_code_fence(text: &str) -> &str {
  let trimmed = text.trim_matches('\n');
  let mut lines = trimmed.lines();
  let Some(first) = lines.next() else {
    return text;
  };
  let first_trim = first.trim();
  if !first_trim.starts_with("```") {
    return text;
  }
  let after_marker = first_trim.trim_start_matches('`');
  if after_marker
    .chars()
    .any(|c| !c.is_alphanumeric() && c != '-' && c != '_' && c != '.')
  {
    return text;
  }
  let last = match trimmed.rsplit_once('\n') {
    Some((_, l)) => l,
    None => return text,
  };
  if last.trim() != "```" {
    return text;
  }
  let body_start = first.len() + 1;
  let body_end = trimmed.len() - last.len();
  let body_end = body_end.saturating_sub(1);
  if body_end < body_start {
    return "";
  }
  &trimmed[body_start..body_end]
}

pub(crate) fn mono_font_for(theme: &gpui_component::Theme) -> Font {
  Font {
    family: theme.mono_font_family.clone(),
    style: FontStyle::Normal,
    weight: FontWeight::NORMAL,
    ..Default::default()
  }
}

pub(crate) fn mini_diff_line_text_for_layout(text: &str) -> SharedString {
  if text.is_empty() {
    SharedString::from(" ")
  } else {
    SharedString::from(text.to_string())
  }
}

pub(crate) fn visible_output_line_ranges(
  text: &str,
  visible: usize,
) -> Vec<std::ops::Range<usize>> {
  let mut ranges = Vec::new();
  let mut start = 0usize;
  while ranges.len() < visible && start < text.len() {
    let Some(newline_offset) = text[start..].find('\n') else {
      ranges.push(start..text.len());
      break;
    };
    let mut end = start + newline_offset;
    if end > start && text.as_bytes()[end - 1] == b'\r' {
      end -= 1;
    }
    ranges.push(start..end);
    start += newline_offset + 1;
  }
  ranges
}

pub(crate) fn syntax_spans_for_range(
  spans: &[HighlightSpan],
  range: std::ops::Range<usize>,
) -> Vec<HighlightSpan> {
  spans
    .iter()
    .filter_map(|span| {
      let start = span.byte_range.start.max(range.start);
      let end = span.byte_range.end.min(range.end);
      if start >= end {
        return None;
      }
      Some(HighlightSpan {
        byte_range: start - range.start..end - range.start,
        token_type: span.token_type,
      })
    })
    .collect()
}

pub(crate) fn build_text_runs(
  text: &str,
  word_spans: &[InlineSpan],
  syntax_spans: &[HighlightSpan],
  syntax_theme: &SyntaxTheme,
  default_color: Hsla,
  word_diff_bg: Option<Hsla>,
  base_font: &Font,
) -> Vec<TextRun> {
  let len = text.len();
  if len == 0 {
    return Vec::new();
  }

  let mut diff_ranges: Vec<std::ops::Range<usize>> = Vec::new();
  if !word_spans.is_empty() && word_diff_bg.is_some() {
    let mut pos = 0usize;
    for span in word_spans {
      let end = (pos + span.text.len()).min(len);
      if span.kind == InlineSpanKind::Diff && end > pos {
        diff_ranges.push(pos..end);
      }
      pos = end;
    }
  }

  let mut boundaries: Vec<usize> = vec![0, len];
  for r in &diff_ranges {
    boundaries.push(r.start);
    boundaries.push(r.end);
  }
  for s in syntax_spans {
    boundaries.push(s.byte_range.start.min(len));
    boundaries.push(s.byte_range.end.min(len));
  }
  boundaries.sort_unstable();
  boundaries.dedup();

  let mut runs = Vec::new();
  for win in boundaries.windows(2) {
    let s = win[0];
    let e = win[1];
    if e <= s {
      continue;
    }
    let fg = syntax_spans
      .iter()
      .find(|h| h.byte_range.start <= s && s < h.byte_range.end)
      .map(|h| syntax_theme.color_for_token(h.token_type))
      .unwrap_or(default_color);
    let bg = diff_ranges
      .iter()
      .find(|r| r.start <= s && s < r.end)
      .and(word_diff_bg);
    runs.push(TextRun {
      len: e - s,
      font: base_font.clone(),
      color: fg,
      background_color: bg,
      underline: None,
      strikethrough: None,
    });
  }
  runs
}

pub(crate) fn tool_kind_label(kind: &ToolKind) -> &'static str {
  match kind {
    ToolKind::Read => "Read",
    ToolKind::Edit => "Edit",
    ToolKind::Delete => "Delete",
    ToolKind::Move => "Move",
    ToolKind::Search => "Search",
    ToolKind::Execute => "Run",
    ToolKind::Think => "Think",
    ToolKind::Fetch => "Fetch",
    _ => "Tool",
  }
}

pub(crate) fn tool_kind_icon(kind: &ToolKind) -> UiIconName {
  match kind {
    ToolKind::Read => UiIconName::BookOpen,
    ToolKind::Edit => UiIconName::SquarePen,
    ToolKind::Delete => UiIconName::Trash,
    ToolKind::Move => UiIconName::RefreshCw,
    ToolKind::Search => UiIconName::Search,
    ToolKind::Execute => UiIconName::SquareTerminal,
    ToolKind::Think => UiIconName::Sparkles,
    ToolKind::Fetch => UiIconName::Globe,
    _ => UiIconName::Puzzle,
  }
}

pub(crate) fn plan_view_from_acp(plan: &Plan) -> PlanView {
  PlanView {
    entries: plan
      .entries
      .iter()
      .map(|e| PlanEntryView {
        content: e.content.clone(),
        priority: match e.priority {
          PlanEntryPriority::High => PlanEntryPriorityView::High,
          PlanEntryPriority::Medium => PlanEntryPriorityView::Medium,
          PlanEntryPriority::Low => PlanEntryPriorityView::Low,
          _ => PlanEntryPriorityView::Medium,
        },
        status: match e.status {
          PlanEntryStatus::Pending => PlanEntryStatusView::Pending,
          PlanEntryStatus::InProgress => PlanEntryStatusView::InProgress,
          PlanEntryStatus::Completed => PlanEntryStatusView::Completed,
          _ => PlanEntryStatusView::Pending,
        },
      })
      .collect(),
  }
}

pub(crate) fn render_plan(plan: &PlanView, theme: &gpui_component::Theme) -> gpui::AnyElement {
  let mut col = v_flex().gap_1().child(
    div()
      .text_sm()
      .font_weight(gpui::FontWeight::BOLD)
      .text_color(theme.foreground)
      .child("Plan"),
  );
  for entry in &plan.entries {
    let (icon, color) = match entry.status {
      PlanEntryStatusView::Completed => (UiIconName::CircleCheck, theme.status_green()),
      PlanEntryStatusView::InProgress => (UiIconName::CircleDot, theme.warning),
      PlanEntryStatusView::Pending => (UiIconName::CircleDot, theme.muted_foreground),
    };
    let strike = entry.status == PlanEntryStatusView::Completed;
    col = col.child(
      h_flex()
        .gap_2()
        .items_start()
        .child(gpui_component::Icon::new(icon).small().text_color(color))
        .child(
          div()
            .flex_1()
            .text_sm()
            .text_color(if strike {
              theme.muted_foreground
            } else {
              theme.foreground
            })
            .when(strike, |this| this.line_through())
            .child(entry.content.clone()),
        ),
    );
  }
  col.into_any_element()
}

/// First non-empty line of a thought, cleaned of markdown emphasis markers.
pub(crate) fn thought_preview(text: &str) -> Option<String> {
  let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
  let cleaned = line.replace("**", "").replace('`', "");
  let preview: String = cleaned.trim().chars().take(80).collect();
  (!preview.is_empty()).then_some(preview)
}

pub(crate) fn render_thought(
  idx: usize,
  thought: &ThoughtView,
  theme: &gpui_component::Theme,
  extensions: &gpui_component::text::MarkdownExtensions,
  cx: &mut Context<AgentChatPanel>,
) -> gpui::AnyElement {
  let collapsed = thought.collapsed;
  let preview = thought.text.as_str();
  let header_label: SharedString = match (collapsed, thought_preview(preview)) {
    (true, Some(preview)) => format!("Thought · {preview}").into(),
    _ => "Thought".into(),
  };
  let body_text = thought.text.trim().to_string();
  v_flex()
    .gap_1()
    .child(
      h_flex()
        .id(SharedString::from(format!("agent-chat-thought-{idx}")))
        .gap_1p5()
        .items_center()
        .cursor_pointer()
        .on_click(cx.listener(move |panel, _, _, cx| panel.toggle_thought_collapsed(idx, cx)))
        .child(
          div()
            .text_xs()
            .text_color(theme.muted_foreground)
            .truncate()
            .hover(|s| s.text_color(theme.foreground))
            .child(header_label),
        )
        .into_any_element(),
    )
    .when(!collapsed && !body_text.is_empty(), |this| {
      this.child(
        div()
          .pl_4()
          .text_xs()
          .text_color(theme.muted_foreground)
          .child(markdown_view(
            ("agent-chat-thought-md", idx),
            &body_text,
            extensions,
            cx,
          )),
      )
    })
    .into_any_element()
}

/// The visible tail of a terminal: joined selectable text, one byte range per
/// row, and whether older output was clipped off the top.
pub(crate) fn terminal_tail(
  output: &str,
  max_lines: usize,
  truncated: bool,
) -> (String, Vec<std::ops::Range<usize>>, bool) {
  let lines: Vec<&str> = output.lines().collect();
  let start = lines.len().saturating_sub(max_lines);
  let clipped = start > 0 || truncated;
  let mut text = String::new();
  let mut ranges = Vec::with_capacity(lines.len() - start);
  for line in &lines[start..] {
    if !ranges.is_empty() {
      text.push('\n');
    }
    let line_start = text.len();
    text.push_str(line);
    ranges.push(line_start..text.len());
  }
  (text, ranges, clipped)
}

/// One embedded terminal: command header, live output tail, exit state.
fn render_terminal_block(
  terminal_id: &str,
  item_id_base: u64,
  term_ix: usize,
  terminal_store: Option<&std::sync::Arc<agent_acp::TerminalStore>>,
  registry: &selectable_text::SelectionRegistry,
  theme: &gpui_component::Theme,
  cx: &mut Context<AgentChatPanel>,
) -> gpui::AnyElement {
  const TERMINAL_TAIL_LINES: usize = 24;
  let Some(snap) = terminal_store.and_then(|store| store.snapshot(terminal_id)) else {
    // A reloaded conversation has no live store for old terminals.
    return div()
      .text_xs()
      .italic()
      .text_color(theme.muted_foreground)
      .child("(terminal session ended)")
      .into_any_element();
  };

  let status: gpui::AnyElement = if !snap.finished {
    h_flex()
      .gap_1()
      .items_center()
      .child(
        div()
          .text_xs()
          .text_color(theme.muted_foreground)
          .child("running"),
      )
      .when(snap.can_kill, |this| {
        this.child(
          div()
            .debug_selector(|| "agent-terminal-stop".to_string())
            .child({
              let id = terminal_id.to_string();
              Button::new(("agent-terminal-stop", item_id_base as usize + term_ix))
                .icon(UiIconName::Stop)
                .xsmall()
                .ghost()
                .tooltip("Stop this command")
                .on_click(cx.listener(move |panel, _, _, cx| {
                  if let Some(store) = panel.terminal_store.as_ref() {
                    store.kill(&id);
                  }
                  cx.notify();
                }))
            }),
        )
      })
      .into_any_element()
  } else if snap.killed {
    div()
      .text_xs()
      .text_color(theme.danger)
      .child("killed")
      .into_any_element()
  } else {
    let code = snap.exit_code.unwrap_or_default();
    div()
      .text_xs()
      .text_color(if code == 0 {
        theme.muted_foreground
      } else {
        theme.danger
      })
      .child(match (snap.exit_code, snap.signal.as_deref()) {
        (Some(code), _) => format!("exit {code}"),
        (None, Some(signal)) => format!("signal {signal}"),
        (None, None) => "exited".to_string(),
      })
      .into_any_element()
  };

  // The tail follows the stream: the last lines are always the ones shown.
  let (tail_text, tail_ranges, clipped) =
    terminal_tail(&snap.output, TERMINAL_TAIL_LINES, snap.truncated);

  v_flex()
    .debug_selector(|| "agent-terminal-block".to_string())
    .gap_0p5()
    .rounded(px(6.))
    .border_1()
    .border_color(theme.border.opacity(0.6))
    .overflow_hidden()
    .child(
      h_flex()
        .gap_2()
        .items_center()
        .px_2()
        .py_1()
        .bg(theme.secondary.opacity(0.6))
        .child(
          div()
            .flex_1()
            .min_w_0()
            .truncate()
            .text_xs()
            .font_family("monospace")
            .text_color(theme.muted_foreground)
            // Agent-owned terminals carry no command line; the tool header
            // above already names the command.
            .when(!snap.command.is_empty(), |this| {
              this.child(format!("$ {}", snap.command))
            }),
        )
        .child(status),
    )
    .when(!tail_text.is_empty() || clipped, |this| {
      let mono_font = mono_font_for(theme);
      let band = gpui::transparent_black();
      let mut rows = Vec::with_capacity(tail_ranges.len() + 1);
      let mut row_ranges = Vec::with_capacity(tail_ranges.len() + 1);
      if clipped {
        rows.push(code_lines::CodeLineRow {
          gutter: None,
          text: "…".into(),
          runs: vec![TextRun {
            len: "…".len(),
            font: mono_font.clone(),
            color: theme.muted_foreground,
            background_color: None,
            underline: None,
            strikethrough: None,
          }],
          band,
        });
        // The clip marker is not part of the selectable output.
        row_ranges.push(0..0);
      }
      for range in &tail_ranges {
        rows.push(code_lines::CodeLineRow {
          gutter: None,
          text: mini_diff_line_text_for_layout(&tail_text[range.clone()]),
          runs: Vec::new(),
          band,
        });
      }
      row_ranges.extend(tail_ranges);
      this.child(
        div()
          .py_1()
          .text_xs()
          .font_family("monospace")
          .overflow_hidden()
          .text_color(theme.foreground)
          .child(
            code_lines::CodeLines::new(
              rows,
              px(0.),
              theme.muted_foreground.opacity(0.72),
              theme.border.opacity(0.45),
              theme.foreground,
              mono_font,
            )
            .selectable(code_lines::SelectionSpec {
              text: SharedString::from(tail_text),
              row_ranges,
              text_id: item_id_base | 0x0400_0000 | ((term_ix as u64) << 12),
              registry: registry.clone(),
            }),
          ),
      )
    })
    .into_any_element()
}

fn mini_code_block(theme: &gpui_component::Theme) -> gpui::Div {
  v_flex()
    .font_family("monospace")
    .text_xs()
    .bg(theme.background)
    .border_1()
    .border_color(theme.border)
    .rounded(px(3.))
    .overflow_hidden()
}

fn render_numbered_tool_output(
  output: &ToolOutput,
  start_line: u32,
  visible: usize,
  text_id_base: u64,
  registry: &selectable_text::SelectionRegistry,
  theme: &gpui_component::Theme,
) -> gpui::AnyElement {
  let ui_theme = ui::Theme::new(theme.is_dark());
  let syntax_theme = ui_theme.syntax();
  let mono_font = mono_font_for(theme);

  let ranges = visible_output_line_ranges(&output.text, visible);
  let selection_end = ranges.last().map(|r| r.end).unwrap_or(0);
  let mut rows = Vec::with_capacity(ranges.len());
  for (line_idx, range) in ranges.iter().enumerate() {
    let line_text = &output.text[range.clone()];
    let line_spans = syntax_spans_for_range(&output.syntax_spans, range.clone());
    let runs = highlights_to_text_runs(
      &line_spans,
      line_text,
      theme.foreground,
      mono_font.clone(),
      &syntax_theme,
    );
    rows.push(code_lines::CodeLineRow {
      gutter: Some(SharedString::from(format!(
        "{:>5}",
        start_line + line_idx as u32
      ))),
      text: mini_diff_line_text_for_layout(line_text),
      runs,
      band: theme.background,
    });
  }

  mini_code_block(theme)
    .py_1()
    .child(
      code_lines::CodeLines::new(
        rows,
        px(54.),
        theme.muted_foreground.opacity(0.72),
        theme.border.opacity(0.45),
        theme.foreground,
        mono_font,
      )
      .selectable(code_lines::SelectionSpec {
        text: SharedString::from(output.text[..selection_end].to_string()),
        row_ranges: ranges,
        text_id: text_id_base,
        registry: registry.clone(),
      }),
    )
    .into_any_element()
}

pub(crate) fn render_tool_call(
  t: &ToolCallView,
  theme: &gpui_component::Theme,
  item_id_base: u64,
  registry: &selectable_text::SelectionRegistry,
  terminal_store: Option<&std::sync::Arc<agent_acp::TerminalStore>>,
  cx: &mut Context<AgentChatPanel>,
) -> gpui::AnyElement {
  let title_color = match t.status {
    ToolCallStatus::Failed => theme.danger,
    _ => theme.foreground,
  };
  let (kind_label, detail) = tool_header_parts(t);
  let tool_id = t.id.clone();
  let detail_color = if kind_label.is_some() {
    theme.muted_foreground
  } else {
    title_color
  };
  // One line, truncated: a long command must not push past the edge. The
  // full text lives in the tooltip.
  let detail_first_line: SharedString =
    detail.lines().next().unwrap_or_default().to_string().into();
  let detail_full: SharedString = detail.clone().into();
  let detail_el = (!detail.is_empty()).then(|| match t.locations.first().cloned() {
    Some((path, line)) => div()
      .id(("agent-tool-location", item_id_base as usize))
      .flex_1()
      .min_w_0()
      .truncate()
      .text_sm()
      .text_color(detail_color)
      .cursor_pointer()
      .hover(|this| this.text_color(theme.foreground))
      .child(detail_first_line.clone())
      .tooltip({
        let detail_full = detail_full.clone();
        move |window, cx| {
          gpui_component::tooltip::Tooltip::new(detail_full.clone()).build(window, cx)
        }
      })
      .on_click(cx.listener(move |_panel, _ev, _window, cx| {
        cx.emit(AgentChatPanelEvent::OpenPath {
          path: path.clone(),
          line,
        });
      }))
      .into_any_element(),
    None => div()
      .id(("agent-tool-detail", item_id_base as usize))
      .flex_1()
      .min_w_0()
      .truncate()
      .text_sm()
      .text_color(detail_color)
      .child(detail_first_line.clone())
      .tooltip({
        let detail_full = detail_full.clone();
        move |window, cx| {
          gpui_component::tooltip::Tooltip::new(detail_full.clone()).build(window, cx)
        }
      })
      .into_any_element(),
  });

  v_flex()
    .debug_selector(|| "agent-tool-card".to_string())
    .gap_1()
    .child(
      h_flex()
        .gap_2()
        .items_center()
        .min_w_0()
        .child(
          div()
            .flex_shrink_0()
            .w(px(20.))
            .h(px(20.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(5.))
            .bg(theme.secondary)
            .child(
              gpui_component::Icon::new(tool_kind_icon(&t.kind))
                .size_3()
                .text_color(title_color),
            ),
        )
        .children(kind_label.map(|label| {
          div()
            .text_sm()
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .flex_shrink_0()
            .text_color(title_color)
            .child(label.to_string())
        }))
        .children(detail_el),
    )
    .when(!t.terminals.is_empty(), |mut this| {
      for (term_ix, id) in t.terminals.iter().enumerate() {
        this = this.child(render_terminal_block(
          id,
          item_id_base,
          term_ix,
          terminal_store,
          registry,
          theme,
          cx,
        ));
      }
      this
    })
    .when(!t.diffs.is_empty(), |this| {
      let mut diff_col = v_flex().gap_2();
      for (diff_idx, d) in t.diffs.iter().enumerate() {
        let mut block = v_flex().gap_0p5().child(
          h_flex()
            .gap_2()
            .child(
              div()
                .text_xs()
                .text_color(theme.foreground)
                .child(d.path.clone()),
            )
            .child(
              div()
                .text_xs()
                .text_color(theme.status_green())
                .child(format!("+{}", d.added)),
            )
            .child(
              div()
                .text_xs()
                .text_color(theme.status_red())
                .child(format!("-{}", d.removed)),
            ),
        );
        if !d.lines.is_empty() {
          let total = d.lines.len();
          let visible = if d.expanded {
            total
          } else {
            total.min(MAX_DIFF_LINES_COLLAPSED)
          };
          let ui_theme = ui::Theme::new(theme.is_dark());
          let syntax_theme = ui_theme.syntax();
          let mono_font = mono_font_for(theme);
          // A created-empty file is one blank added line: a bare green band
          // reads as a glitch, so name the emptiness.
          let empty_creation =
            d.lines.len() == 1 && d.removed == 0 && d.lines[0].text.trim().is_empty();
          let show_line_numbers = d
            .lines
            .iter()
            .take(visible)
            .any(|line| line.old_line.is_some() || line.new_line.is_some());
          let diff_gutter = |old_line: Option<u32>, new_line: Option<u32>| {
            SharedString::from(format!(
              "{:>4} {:>4}",
              old_line.map(|n| n.to_string()).unwrap_or_default(),
              new_line.map(|n| n.to_string()).unwrap_or_default()
            ))
          };
          let mut rows = Vec::with_capacity(visible);
          let mut selection_text = String::new();
          let mut row_ranges = Vec::with_capacity(visible);
          for line in d.lines.iter().take(visible) {
            let (bg, fg, hl_bg) = match line.kind {
              DiffLineKind::Added => (
                ui_theme.diff_added_background(),
                theme.foreground,
                ui_theme.diff_word_added_background(),
              ),
              DiffLineKind::Removed => (
                ui_theme.diff_removed_background(),
                theme.foreground,
                ui_theme.diff_word_removed_background(),
              ),
            };
            let gutter = show_line_numbers.then(|| diff_gutter(line.old_line, line.new_line));
            if !selection_text.is_empty() {
              selection_text.push('\n');
            }
            let selection_start = selection_text.len();
            selection_text.push_str(if empty_creation {
              "(empty file)"
            } else {
              &line.text
            });
            row_ranges.push(selection_start..selection_text.len());
            if empty_creation {
              rows.push(code_lines::CodeLineRow {
                gutter,
                text: "(empty file)".into(),
                runs: vec![TextRun {
                  len: "(empty file)".len(),
                  font: Font {
                    style: FontStyle::Italic,
                    ..mono_font.clone()
                  },
                  color: theme.muted_foreground,
                  background_color: None,
                  underline: None,
                  strikethrough: None,
                }],
                band: bg,
              });
              continue;
            }
            let runs = build_text_runs(
              &line.text,
              &line.spans,
              &line.syntax_spans,
              &syntax_theme,
              fg,
              Some(hl_bg),
              &mono_font,
            );
            rows.push(code_lines::CodeLineRow {
              gutter,
              text: mini_diff_line_text_for_layout(&line.text),
              runs,
              band: bg,
            });
          }
          let gutter_width = if show_line_numbers { px(70.) } else { px(0.) };
          let body = mini_code_block(theme).child(
            code_lines::CodeLines::new(
              rows,
              gutter_width,
              theme.muted_foreground.opacity(0.72),
              theme.border.opacity(0.45),
              theme.foreground,
              mono_font,
            )
            .selectable(code_lines::SelectionSpec {
              text: SharedString::from(selection_text),
              row_ranges,
              text_id: item_id_base | 0x0200_0000 | ((diff_idx as u64) << 12),
              registry: registry.clone(),
            }),
          );
          block = block.child(body);
          if total > MAX_DIFF_LINES_COLLAPSED {
            let remaining = total.saturating_sub(visible);
            let label: SharedString = if d.expanded {
              "Show less".into()
            } else {
              format!(
                "Show {remaining} more line{}",
                if remaining == 1 { "" } else { "s" }
              )
              .into()
            };
            let button_id =
              SharedString::from(format!("agent-chat-diff-expand-{}-{diff_idx}", tool_id.0));
            let tool_id = tool_id.clone();
            block = block.child(
              Button::new(button_id)
                .label(label)
                .xsmall()
                .ghost()
                .on_click(cx.listener(move |panel, _, _, cx| {
                  panel.toggle_diff_expanded(tool_id.clone(), diff_idx, cx);
                })),
            );
          }
        }
        diff_col = diff_col.child(block);
      }
      this.child(diff_col)
    })
    .when(!t.outputs.is_empty(), |this| {
      let mut out_col = v_flex().gap_2();
      let ui_theme = ui::Theme::new(theme.is_dark());
      let syntax_theme = ui_theme.syntax();
      let mono_font = mono_font_for(theme);
      for (out_idx, output) in t.outputs.iter().enumerate() {
        let total = output.text.lines().count();
        let visible = if output.expanded {
          total
        } else {
          total.min(MAX_TOOL_OUTPUT_LINES_COLLAPSED)
        };
        let body_text: String = if visible >= total {
          output.text.clone()
        } else {
          let mut count = 0usize;
          let mut end = output.text.len();
          for (i, b) in output.text.as_bytes().iter().enumerate() {
            if *b == b'\n' {
              count += 1;
              if count == visible {
                end = i;
                break;
              }
            }
          }
          output.text[..end].to_string()
        };
        let text_id = item_id_base | 0x100 | (out_idx as u64);
        let output_start_line =
          read_tool_output_start_line(&t.kind, &t.locations).or(output.start_line);
        let content_div = if let Some(start_line) = output_start_line {
          render_numbered_tool_output(
            output,
            start_line,
            visible,
            item_id_base | 0x0100_0000 | ((out_idx as u64) << 20),
            registry,
            theme,
          )
        } else {
          let runs = highlights_to_text_runs(
            &output.syntax_spans,
            &body_text,
            theme.foreground,
            mono_font.clone(),
            &syntax_theme,
          );
          mini_code_block(theme)
            .px_2()
            .py_1()
            .text_color(theme.foreground)
            .whitespace_normal()
            .child(selectable_text::SelectableText::new(
              text_id,
              SharedString::from(body_text),
              runs,
              registry.clone(),
            ))
            .into_any_element()
        };
        let mut block = v_flex().gap_0p5().child(content_div);
        if total > MAX_TOOL_OUTPUT_LINES_COLLAPSED {
          let remaining = total.saturating_sub(visible);
          let label: SharedString = if output.expanded {
            "Show less".into()
          } else {
            format!(
              "Show {remaining} more line{}",
              if remaining == 1 { "" } else { "s" }
            )
            .into()
          };
          let button_id =
            SharedString::from(format!("agent-chat-output-expand-{}-{out_idx}", tool_id.0));
          let tool_id_for_click = tool_id.clone();
          block = block.child(
            Button::new(button_id)
              .label(label)
              .xsmall()
              .ghost()
              .on_click(cx.listener(move |panel, _, _, cx| {
                panel.toggle_output_expanded(tool_id_for_click.clone(), out_idx, cx);
              })),
          );
        }
        out_col = out_col.child(block);
      }
      this.child(out_col)
    })
    .into_any_element()
}

pub(crate) fn permission_option_is_destructive(kind: &PermissionOptionKind) -> bool {
  matches!(
    kind,
    PermissionOptionKind::RejectOnce | PermissionOptionKind::RejectAlways
  )
}

pub(crate) fn format_turn_duration(secs: u64) -> String {
  if secs < 60 {
    format!("{secs}s")
  } else if secs.is_multiple_of(60) {
    format!("{}m", secs / 60)
  } else {
    format!("{}m {}s", secs / 60, secs % 60)
  }
}

/// The option auto-approve picks: allow once first, then allow always.
pub(crate) fn auto_approve_option(options: &[agent_acp::PermissionPromptOption]) -> Option<String> {
  options
    .iter()
    .find(|o| matches!(o.kind, PermissionOptionKind::AllowOnce))
    .or_else(|| {
      options
        .iter()
        .find(|o| matches!(o.kind, PermissionOptionKind::AllowAlways))
    })
    .map(|o| o.option_id.clone())
}

pub(crate) fn render_permission(
  item: &PermissionItem,
  theme: &gpui_component::Theme,
  registry: &selectable_text::SelectionRegistry,
  cx: &mut Context<AgentChatPanel>,
) -> gpui::AnyElement {
  let prompt_id = item.prompt.id;
  let resolved = item.resolved.clone();
  let detail = &item.detail;
  let mut card = v_flex()
    .gap_2()
    .p_3()
    .border_1()
    .border_color(theme.warning)
    .rounded(px(4.))
    .child(
      div()
        .text_xs()
        .text_color(theme.muted_foreground)
        .child("Permission required".to_string()),
    )
    .when(
      item.prompt.tool_call_title != "Permission required",
      |this| {
        this.child(
          div()
            .text_sm()
            .text_color(theme.foreground)
            .child(item.prompt.tool_call_title.clone()),
        )
      },
    );

  if let Some(invocation) = &detail.invocation {
    card = card.child(
      div()
        .id(("perm-invocation", prompt_id as usize))
        .debug_selector(|| "perm-invocation".to_string())
        .max_h(px(92.))
        .overflow_y_scroll()
        .px_2()
        .py_1()
        .bg(theme.background)
        .border_1()
        .border_color(theme.border)
        .rounded(px(3.))
        .font_family("monospace")
        .text_xs()
        .text_color(theme.foreground)
        .whitespace_normal()
        .child(selectable_text::SelectableText::new(
          prompt_id | 0x8000_0000_0000_0000,
          SharedString::from(invocation.clone()),
          Vec::new(),
          registry.clone(),
        )),
    );
  }
  if !detail.diff_stats.is_empty() {
    let mut stats = v_flex()
      .gap_0p5()
      .debug_selector(|| "perm-diff-stats".to_string());
    for (path, added, removed) in &detail.diff_stats {
      stats = stats.child(
        h_flex()
          .gap_2()
          .items_center()
          .text_xs()
          .child(
            div()
              .flex_1()
              .truncate()
              .text_color(theme.muted_foreground)
              .child(path.clone()),
          )
          .child(
            div()
              .text_color(theme.status_green())
              .child(format!("+{added}")),
          )
          .child(div().text_color(theme.danger).child(format!("-{removed}"))),
      );
    }
    card = card.child(stats);
  } else if let Some(path) = &detail.path {
    card = card.child(
      div()
        .debug_selector(|| "perm-path".to_string())
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(path.clone()),
    );
  }

  if let Some(option_id) = &resolved {
    let label = item
      .prompt
      .options
      .iter()
      .find(|option| option.option_id == *option_id)
      .map(|option| option.label.clone())
      .unwrap_or_else(|| option_id.clone());
    let answer = if item.auto {
      format!("Auto-approved: {label}")
    } else {
      format!("Answered: {label}")
    };
    card = card.child(
      div()
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(answer),
    );
    return card.into_any_element();
  }

  let mut buttons = h_flex().gap_2().flex_wrap();
  for option in &item.prompt.options {
    let option_id = option.option_id.clone();
    let destructive = permission_option_is_destructive(&option.kind);
    let button_id = format!("perm-{}-{}", prompt_id, option.option_id);
    let mut button = Button::new(SharedString::from(button_id))
      .label(option.label.clone())
      .small()
      .on_click(cx.listener(move |panel, _, _, cx| {
        panel.answer_permission(prompt_id, Some(option_id.clone()), cx);
      }));
    if destructive {
      button = button.danger();
    } else {
      button = button.primary();
    }
    buttons = buttons.child(button);
  }
  buttons = buttons.child(
    Button::new(SharedString::from(format!("perm-{prompt_id}-cancel")))
      .label("Cancel")
      .small()
      .ghost()
      .on_click(cx.listener(move |panel, _, _, cx| {
        panel.answer_permission(prompt_id, None, cx);
      })),
  );

  card.child(buttons).into_any_element()
}

impl Focusable for AgentChatPanel {
  fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
    self.input.read(cx).focus_handle(cx)
  }
}

impl AgentChatPanel {
  pub fn input_focus_handle(&self, cx: &gpui::App) -> FocusHandle {
    self.input.read(cx).focus_handle(cx)
  }
}

impl Render for AgentChatPanel {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    self.update_runway(window);
    self.update_reader_follow();
    self.update_jump_pill();
    if self.scroll_save_pending {
      self.scroll_save_pending = false;
      self.save_scroll_position(cx);
    }
    let theme = cx.theme().clone();
    let theme = &theme;
    // The composer box owns the focus ring now that the textarea is bare.
    let composer_focused = self.input.focus_handle(cx).is_focused(window);

    let _ = SharedString::from("");

    let usage_text: Option<SharedString> = self.usage.map(|(used, size)| {
      let used_k = used as f64 / 1000.0;
      let size_k = size as f64 / 1000.0;
      format!("{used_k:.1}k / {size_k:.0}k").into()
    });

    let connecting = matches!(self.status, Status::Connecting);
    let show_empty_state = self.items.is_empty()
      && self.extras_before_kinds().is_empty()
      && matches!(self.status, Status::Ready | Status::Connecting);
    let empty_state = if show_empty_state {
      let brand_icon = crate::backend_icon(&self.backend_kind);
      let content = if connecting {
        v_flex()
          .items_center()
          .gap_3()
          .child(
            gpui_component::Icon::new(brand_icon)
              .large()
              .text_color(theme.muted_foreground),
          )
          .child(
            div()
              .text_sm()
              .text_color(theme.muted_foreground)
              .child(format!("Connecting to {}...", self.backend.label)),
          )
          .opacity(pulse_opacity(0.4, 1.0))
          .into_any_element()
      } else {
        v_flex()
          .items_center()
          .gap_2()
          .child(gpui_component::Icon::new(UiIconName::Sparkles).text_color(theme.muted_foreground))
          .child(
            div()
              .text_sm()
              .text_color(theme.muted_foreground)
              .child(format!("Start a conversation with {}", self.backend.label)),
          )
          .child(
            div()
              .text_xs()
              .text_color(theme.muted_foreground)
              .child("Send a message below to begin."),
          )
          .into_any_element()
      };
      Some(
        v_flex()
          .flex_1()
          .min_h_0()
          .items_center()
          .justify_center()
          .child(content),
      )
    } else {
      None
    };

    div()
      .flex()
      .flex_col()
      .size_full()
      .bg(theme.sidebar)
      .child(
        h_flex()
          .h(px(40.))
          .min_h(px(40.))
          .max_h(px(40.))
          .flex_shrink_0()
          .px_3()
          .items_center()
          .justify_between()
          .bg(theme.sidebar)
          .border_b_1()
          .border_color(theme.border)
          .child({
            let current = self.backend_kind.clone();
            let label_suffix = match &self.status {
              Status::Connecting => "",
              Status::Error(_) => " (error)",
              Status::MissingBinary { .. } => " (not installed)",
              Status::Ready => "",
            };
            let label = format!("{}{}", self.backend.label, label_suffix);
            let brand_icon = crate::backend_icon(&current);
            let entity = cx.entity().downgrade();
            Button::new("agent-chat-backend")
              .label(label)
              .icon(brand_icon)
              .dropdown_caret(true)
              .small()
              .ghost()
              .debug_selector(|| "agent-chat-backend".to_string())
              .dropdown_menu_with_anchor(Anchor::TopLeft, move |menu, _, _| {
                // The registry serves more agents than fit on screen.
                let mut menu = menu.max_h(px(360.)).scrollable(true);
                let registry = agent_registry::global();
                for agent in registry.runnable() {
                  let id = agent.id.clone();
                  let entity = entity.clone();
                  let is_current = id == current;
                  let label_text: SharedString = agent.name.clone().into();
                  let selector_id = id.to_string();
                  let icon_id = id.clone();
                  menu = menu.item(
                    PopupMenuItem::element(move |_, cx| {
                      let theme = cx.theme().clone();
                      h_flex()
                        .w_full()
                        .gap_2()
                        .items_center()
                        .debug_selector({
                          let selector_id = selector_id.clone();
                          move || format!("agent-chat-backend-item-{selector_id}")
                        })
                        .child(
                          crate::backend_icon(&icon_id)
                            .small()
                            .text_color(theme.foreground),
                        )
                        .child(
                          div()
                            .flex_1()
                            .text_sm()
                            .when(is_current, |this| this.font_weight(gpui::FontWeight::BOLD))
                            .child(label_text.clone()),
                        )
                        .when(is_current, |this| {
                          this.child(
                            gpui_component::Icon::new(UiIconName::Check)
                              .small()
                              .text_color(theme.foreground),
                          )
                        })
                        .into_any_element()
                    })
                    .on_click(move |_, _, cx| {
                      persist_choice(&id);
                      let id = id.clone();
                      let _ = entity.update(cx, |panel, cx| panel.switch_backend(id, cx));
                    }),
                  );
                }
                menu
              })
          })
          .child(
            h_flex()
              .gap_3()
              .items_center()
              .when_some(usage_text, |this, t| {
                this.child(div().text_xs().text_color(theme.muted_foreground).child(t))
              })
              .when(self.show_conversation_controls, |this| {
                this.child({
                  let entity = cx.entity().downgrade();
                  let conversations = self.list_conversations(cx);
                  let current_id = self.current_conv.id.clone();
                  Button::new("agent-chat-history")
                    .icon(UiIconName::History)
                    .small()
                    .ghost()
                    .disabled(conversations.is_empty())
                    .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                      let mut menu = menu;
                      for meta in &conversations {
                        let id = meta.id.clone();
                        let id_load = meta.id.clone();
                        let id_delete = meta.id.clone();
                        let entity_load = entity.clone();
                        let entity_delete = entity.clone();
                        let title: SharedString = if meta.title.is_empty() {
                          format!("Conversation {}", meta.id).into()
                        } else {
                          meta.title.clone().into()
                        };
                        let group_name = SharedString::from(format!("hist-row-{}", meta.id));
                        let group_for_render = group_name.clone();
                        let button_id = SharedString::from(format!("hist-delete-{}", meta.id));
                        let title_for_render = title.clone();
                        let is_current = id == current_id;
                        menu = menu.item(
                          PopupMenuItem::element(move |_, _| {
                            let entity_delete = entity_delete.clone();
                            let id_delete = id_delete.clone();
                            h_flex()
                              .group(group_for_render.clone())
                              .w_full()
                              .max_w(px(280.))
                              .gap_2()
                              .items_center()
                              .child(
                                div()
                                  .flex_1()
                                  .min_w_0()
                                  .text_sm()
                                  .truncate()
                                  .when(is_current, |this| this.font_weight(gpui::FontWeight::BOLD))
                                  .child(title_for_render.clone()),
                              )
                              .child(
                                Button::new(button_id.clone())
                                  .icon(UiIconName::Trash)
                                  .xsmall()
                                  .ghost()
                                  .opacity(0.0)
                                  .group_hover(group_for_render.clone(), |this| this.opacity(1.0))
                                  .on_click(move |_, window, cx| {
                                    let _ = entity_delete.update(cx, |panel, cx| {
                                      panel.delete_conversation(&id_delete, window, cx)
                                    });
                                  }),
                              )
                              .into_any_element()
                          })
                          .on_click(move |_, window, cx| {
                            let id = id_load.clone();
                            let _ = entity_load
                              .update(cx, |panel, cx| panel.load_conversation(&id, window, cx));
                          }),
                        );
                        let _ = id;
                      }
                      menu
                    })
                })
              })
              .when(self.show_conversation_controls, |this| {
                this.child(
                  Button::new("agent-chat-new")
                    .icon(UiIconName::MessageCirclePlus)
                    .small()
                    .ghost()
                    .on_click(
                      cx.listener(|panel, _, window, cx| panel.new_conversation(window, cx)),
                    ),
                )
              }),
          ),
      )
      .map(|this| {
        if let Some(empty_state) = empty_state {
          this.child(empty_state)
        } else {
          let entity = cx.entity().clone();
          let messages_list = self.messages_list.clone();
          this.child(
            div()
              .flex_1()
              .min_h_0()
              .relative()
              .child(
                gpui::list(messages_list, move |ix, _window, cx| {
                  entity.update(cx, |panel, cx| {
                    let theme = cx.theme().clone();
                    panel.render_list_item(ix, &theme, cx)
                  })
                })
                .size_full(),
              )
              .child(
                div()
                  .absolute()
                  .bottom_0()
                  .left_0()
                  .right_0()
                  .h(px(CONVERSATION_BOTTOM_FADE_PX))
                  .bg(gpui::linear_gradient(
                    180.,
                    gpui::linear_color_stop(theme.sidebar.opacity(0.), 0.),
                    gpui::linear_color_stop(theme.sidebar, 1.),
                  )),
              )
              // Painted after the fade so the pill sits above it.
              .when(self.show_jump_pill, |this| {
                this.child(
                  div()
                    .debug_selector(|| "agent-chat-jump-bottom".to_string())
                    .absolute()
                    .bottom(px(CONVERSATION_BOTTOM_FADE_PX - 24.))
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .child(
                      // Opaque backdrop: the ghost hover tint composes over it
                      // instead of letting the transcript show through.
                      div()
                        .rounded(px(999.))
                        .bg(theme.background)
                        .border_1()
                        .border_color(theme.border)
                        .overflow_hidden()
                        .child(
                          Button::new("agent-chat-jump-bottom")
                            .icon(IconName::ChevronDown)
                            .small()
                            .ghost()
                            .rounded(px(999.))
                            .on_click(cx.listener(|panel, _, _, cx| {
                              panel.jump_to_tail();
                              cx.notify();
                            })),
                        ),
                    ),
                )
              })
              .vertical_scrollbar(&self.messages_list),
          )
        }
      })
      .child(
        div()
          .flex_shrink_0()
          .w_full()
          .flex()
          .justify_center()
          .px_3()
          .pb_3()
          .bg(theme.sidebar)
          .child(
            v_flex()
              .w_full()
              .max_w(px(CONVERSATION_COLUMN_MAX_WIDTH_PX))
              .gap_1()
              .children(self.render_queued_prompts(theme, cx))
              .child(
                v_flex()
                  .debug_selector(|| "agent-chat-composer".to_string())
                  .w_full()
                  .px_2()
                  .py_1p5()
                  .gap_1()
                  .rounded(theme.radius_lg)
                  .border_1()
                  .border_color(if composer_focused {
                    theme.ring
                  } else {
                    theme.border
                  })
                  .bg(theme.background)
                  .on_drop(
                    cx.listener(|panel, paths: &gpui::ExternalPaths, window, cx| {
                      panel.handle_dropped_paths(paths.paths(), window, cx);
                    }),
                  )
                  .drag_over::<gpui::ExternalPaths>(|style, _, _, cx| {
                    let theme = cx.theme();
                    style.border_color(theme.ring).bg(theme.ring.opacity(0.06))
                  })
                  .children(self.render_staged_images(theme, cx))
                  .child(
                    div()
                      .id("agent-mention-input")
                      .relative()
                      .w_full()
                      .capture_action(cx.listener(|panel, _: &input::Paste, _, cx| {
                        panel.intercept_paste(cx);
                      }))
                      .capture_action(cx.listener(|panel, action: &input::Enter, window, cx| {
                        // Slash and mention popups are exclusive by token shape.
                        panel.slash_on_enter(action, window, cx);
                        panel.mention_on_enter(action, window, cx);
                      }))
                      // The textarea propagates a submitting Enter; unstopped, the
                      // keystroke falls through and types "\n" into the input.
                      .on_action(cx.listener(|_, action: &input::Enter, _, cx| {
                        if !action.shift {
                          cx.stop_propagation();
                        }
                      }))
                      .capture_action(cx.listener(|panel, _: &input::MoveUp, _, cx| {
                        panel.slash_on_move(-1, cx);
                        panel.mention_on_move(-1, cx);
                      }))
                      .capture_action(cx.listener(|panel, _: &input::MoveDown, _, cx| {
                        panel.slash_on_move(1, cx);
                        panel.mention_on_move(1, cx);
                      }))
                      .capture_action(cx.listener(|panel, _: &input::Escape, _, cx| {
                        panel.slash_on_escape(cx);
                        panel.mention_on_escape(cx);
                      }))
                      .child(Textarea::new(&self.input).appearance(false).w_full())
                      .child(self.render_mention_overlay(cx))
                      .child(self.render_slash_overlay(cx)),
                  )
                  .child(
                    h_flex()
                      .items_center()
                      .justify_between()
                      .gap_2()
                      .child(
                        h_flex()
                          .gap_1()
                          .flex_wrap()
                          .child(self.render_model_selector(cx))
                          .child(self.render_mode_selector(cx))
                          .children(self.render_config_selector(cx))
                          .child(self.render_auto_approve_toggle(cx)),
                      )
                      .child(if self.in_flight {
                        h_flex()
                          .gap_1()
                          .child(
                            Button::new("agent-chat-queue-send")
                              .icon(UiIconName::ArrowUp)
                              .small()
                              .rounded(px(999.))
                              .tooltip("Queue for the next turn")
                              .on_click(
                                cx.listener(|panel, _, window, cx| panel.submit(window, cx)),
                              ),
                          )
                          .child(
                            Button::new("agent-chat-stop")
                              .icon(UiIconName::Stop)
                              .small()
                              .rounded(px(999.))
                              .danger()
                              .on_click(cx.listener(|panel, _, _, cx| panel.cancel_turn(cx))),
                          )
                          .into_any_element()
                      } else {
                        Button::new("agent-chat-send")
                          .icon(UiIconName::ArrowUp)
                          .small()
                          .rounded(px(999.))
                          .primary()
                          .disabled(!matches!(self.status, Status::Ready))
                          .on_click(cx.listener(|panel, _, window, cx| panel.submit(window, cx)))
                          .into_any_element()
                      }),
                  ),
              ),
          ),
      )
  }
}

impl AgentChatPanel {
  /// Cards for messages queued mid-turn, stacked above the composer.
  fn render_queued_prompts(
    &self,
    theme: &gpui_component::Theme,
    cx: &mut Context<Self>,
  ) -> Option<gpui::AnyElement> {
    if self.queued_prompts.is_empty() {
      return None;
    }
    let mut col = v_flex()
      .debug_selector(|| "agent-chat-queued".to_string())
      .gap_1();
    for (ix, text) in self.queued_prompts.iter().enumerate() {
      col = col.child(
        h_flex()
          .gap_2()
          .items_center()
          .px_2()
          .py_1()
          .rounded(theme.radius)
          .border_1()
          .border_color(theme.border)
          .bg(theme.background)
          .child(
            gpui_component::Icon::new(UiIconName::MessageCirclePlus)
              .small()
              .text_color(theme.muted_foreground),
          )
          .child(
            div()
              .flex_1()
              .min_w_0()
              .truncate()
              .text_sm()
              .text_color(theme.foreground)
              .child(text.clone()),
          )
          .when(self.in_flight && self.supports_steering, |this| {
            this.child(
              Button::new(("agent-chat-queued-steer", ix))
                .icon(UiIconName::ArrowUpFromLine)
                .xsmall()
                .ghost()
                .debug_selector(|| "agent-chat-queued-steer".to_string())
                .tooltip("Send into the current turn")
                .on_click(cx.listener(move |panel, _, _, cx| {
                  panel.steer_queued(ix, cx);
                })),
            )
          })
          .child(
            Button::new(("agent-chat-queued-edit", ix))
              .icon(UiIconName::SquarePen)
              .xsmall()
              .ghost()
              .tooltip("Edit in composer")
              .on_click(cx.listener(move |panel, _, window, cx| {
                panel.pop_queued_to_composer(ix, window, cx);
              })),
          )
          .child(
            Button::new(("agent-chat-queued-delete", ix))
              .icon(UiIconName::Trash)
              .xsmall()
              .ghost()
              .tooltip("Remove")
              .on_click(cx.listener(move |panel, _, _, cx| {
                panel.delete_queued(ix, cx);
              })),
          ),
      );
    }
    Some(col.into_any_element())
  }

  /// Thumbnails of the images staged for the next prompt.
  fn render_staged_images(
    &self,
    theme: &gpui_component::Theme,
    cx: &mut Context<Self>,
  ) -> Option<gpui::AnyElement> {
    if self.staged_images.is_empty() {
      return None;
    }
    let mut strip = h_flex()
      .debug_selector(|| "agent-chat-attachments".to_string())
      .gap_2()
      .pt_1()
      .pb_2()
      .mb_1()
      .border_b_1()
      .border_color(theme.border.opacity(0.5))
      .flex_wrap();
    for (ix, image) in self.staged_images.iter().enumerate() {
      strip = strip.child(
        div()
          .group("chat-attachment")
          .relative()
          .w(px(56.))
          .h(px(56.))
          .rounded(px(8.))
          .overflow_hidden()
          .border_1()
          .border_color(theme.border)
          .child(
            gpui::img(image.clone())
              .size_full()
              .object_fit(gpui::ObjectFit::Cover),
          )
          .child(
            div()
              .absolute()
              .top(px(2.))
              .right(px(2.))
              .opacity(0.)
              .group_hover("chat-attachment", |this| this.opacity(1.))
              .child(
                Button::new(("agent-chat-attachment-remove", ix))
                  .icon(IconName::Close)
                  .xsmall()
                  .rounded(px(999.))
                  .on_click(cx.listener(move |panel, _, _, cx| {
                    panel.remove_staged_image(ix, cx);
                  })),
              ),
          ),
      );
    }
    Some(strip.into_any_element())
  }

  fn render_model_selector(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
    let models = self.available_models.clone();
    let current_id = self.current_model_id.clone();
    let current_label: SharedString = current_id
      .as_ref()
      .and_then(|id| models.iter().find(|m| m.model_id == *id))
      .map(|m| short_model_label(&m.name, m.description.as_deref()).into())
      .unwrap_or_else(|| "Model".into());
    let entity = cx.entity().downgrade();
    let brand_icon = crate::backend_icon(&self.backend_kind);
    Button::new("agent-chat-model")
      .child(selector_trigger(Some(brand_icon), current_label))
      .xsmall()
      .ghost()
      .disabled(models.is_empty())
      .dropdown_menu_with_anchor(Anchor::BottomLeft, move |menu, _, _| {
        let mut menu = menu
          .label("Select a model")
          .max_h(px(360.))
          .scrollable(true);
        for (label, model_id, description, is_current) in
          deduped_model_entries(&models, current_id.as_ref())
        {
          let entity = entity.clone();
          let label_text: SharedString = label.into();
          let description: Option<SharedString> = description.map(Into::into);
          menu = menu.item(
            PopupMenuItem::element(move |_, cx| {
              render_selector_item(label_text.clone(), description.clone(), is_current, cx)
            })
            .on_click(move |_, _, cx| {
              let model_id = model_id.clone();
              let _ = entity.update(cx, |panel, cx| panel.set_model(model_id, cx));
            }),
          );
        }
        menu
      })
      .into_any_element()
  }

  fn render_auto_approve_toggle(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
    let active = self.auto_approve;
    div()
      .debug_selector(|| "agent-chat-auto-approve".to_string())
      .child(
        Button::new("agent-chat-auto-approve")
          .icon(UiIconName::CircleCheck)
          .label("Auto-approve")
          .xsmall()
          .ghost()
          .when(active, |this| this.text_color(cx.theme().primary))
          .tooltip(if active {
            "Permission requests are approved automatically"
          } else {
            "Approve permission requests automatically"
          })
          .on_click(cx.listener(|panel, _, _, cx| panel.toggle_auto_approve(cx))),
      )
      .into_any_element()
  }

  fn render_mode_selector(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
    let modes = self.available_modes.clone();
    let current_id = self.current_mode_id.clone();
    let current_label: SharedString = current_id
      .as_ref()
      .and_then(|id| modes.iter().find(|m| m.id == *id))
      .map(|m| m.name.clone().into())
      .unwrap_or_else(|| "Mode".into());
    let entity = cx.entity().downgrade();
    Button::new("agent-chat-mode")
      .child(selector_trigger(None, current_label))
      .xsmall()
      .ghost()
      .disabled(modes.is_empty())
      .dropdown_menu_with_anchor(Anchor::BottomLeft, move |menu, _, _| {
        let mut menu = menu.label("Select a mode");
        for m in modes.iter() {
          let mode_id = m.id.clone();
          let entity = entity.clone();
          let is_current = current_id.as_ref() == Some(&mode_id);
          let label_text: SharedString = m.name.clone().into();
          let description: Option<SharedString> = m.description.clone().map(Into::into);
          menu = menu.item(
            PopupMenuItem::element(move |_, cx| {
              render_selector_item(label_text.clone(), description.clone(), is_current, cx)
            })
            .on_click(move |_, _, cx| {
              let mode_id = mode_id.clone();
              let _ = entity.update(cx, |panel, cx| panel.set_mode(mode_id, cx));
            }),
          );
        }
        menu
      })
      .into_any_element()
  }

  /// One trigger instead of a row of bare dropdowns.
  fn render_config_selector(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
    let options = self.selectable_config_options();
    if options.is_empty() {
      return None;
    }

    let summary: SharedString = config_summary(&options).into();
    let customized = config_customized(&options, &self.config_defaults);
    let entity = cx.entity().downgrade();

    Some(
      Button::new("agent-chat-config")
        .child(selector_trigger(None, summary))
        .xsmall()
        .ghost()
        .when(!customized, |this| {
          this.text_color(cx.theme().muted_foreground)
        })
        .dropdown_menu_with_anchor(Anchor::BottomLeft, move |menu, _, _| {
          let mut menu = menu.max_h(px(420.)).scrollable(true);
          for (ix, option) in options.iter().enumerate() {
            if ix > 0 {
              menu = menu.separator();
            }
            menu = menu.label(option.name.clone());
            for (value_id, name, description) in option.values.iter() {
              let value_id = value_id.clone();
              let name: SharedString = name.clone().into();
              let description: Option<SharedString> = description.clone().map(Into::into);
              let entity = entity.clone();
              let config_id = option.id.clone();
              let is_current = value_id == option.current_value;
              menu = menu.item(
                PopupMenuItem::element(move |_, cx| {
                  render_selector_item(name.clone(), description.clone(), is_current, cx)
                })
                .on_click(move |_, _, cx| {
                  let value_id = value_id.clone();
                  let config_id = config_id.clone();
                  let _ = entity.update(cx, |panel, cx| {
                    panel.set_config_option(config_id, value_id, cx)
                  });
                }),
              );
            }
          }
          menu
        })
        .into_any_element(),
    )
  }

  fn selectable_config_options(&self) -> Vec<ConfigSelector> {
    selectable_config_options(&self.config_options)
  }
}

/// The non-model, non-mode selects the composer collapses behind one trigger.
pub(crate) fn selectable_config_options(options: &[SessionConfigOption]) -> Vec<ConfigSelector> {
  options
    .iter()
    .filter(|opt| {
      !matches!(
        opt.category,
        Some(SessionConfigOptionCategory::Model) | Some(SessionConfigOptionCategory::Mode)
      )
    })
    .filter_map(|opt| {
      let SessionConfigKind::Select(sel) = &opt.kind else {
        return None;
      };
      let values: Vec<(SessionConfigValueId, String, Option<String>)> = match &sel.options {
        SessionConfigSelectOptions::Ungrouped(opts) => opts
          .iter()
          .map(|o| (o.value.clone(), o.name.clone(), o.description.clone()))
          .collect(),
        SessionConfigSelectOptions::Grouped(groups) => groups
          .iter()
          .flat_map(|g| {
            g.options
              .iter()
              .map(|o| (o.value.clone(), o.name.clone(), o.description.clone()))
          })
          .collect(),
        _ => Vec::new(),
      };
      if values.is_empty() {
        return None;
      }
      let current_label = values
        .iter()
        .find(|(value, _, _)| *value == sel.current_value)
        .map(|(_, name, _)| name.clone())
        .unwrap_or_else(|| opt.name.clone());
      Some(ConfigSelector {
        id: opt.id.clone(),
        name: opt.name.clone().into(),
        current_value: sel.current_value.clone(),
        current_label,
        values,
      })
    })
    .collect()
}

pub(crate) fn config_summary(selectors: &[ConfigSelector]) -> String {
  selectors
    .iter()
    .map(|selector| selector.current_label.as_str())
    .collect::<Vec<_>>()
    .join(" · ")
}

/// True once any option left the value the agent first advertised.
pub(crate) fn config_customized(
  selectors: &[ConfigSelector],
  defaults: &HashMap<SessionConfigId, SessionConfigValueId>,
) -> bool {
  selectors.iter().any(|selector| {
    defaults
      .get(&selector.id)
      .is_some_and(|default| *default != selector.current_value)
  })
}

/// Composer selector trigger: optional leading icon, label, trailing chevron.
pub(crate) fn selector_trigger(
  icon: Option<gpui_component::Icon>,
  label: SharedString,
) -> impl IntoElement {
  h_flex()
    .items_center()
    .gap_1()
    .when_some(icon, |this, icon| this.child(icon.xsmall()))
    .child(label)
    .child(gpui_component::Icon::new(IconName::ChevronDown).xsmall())
}

pub(crate) fn mention_labels(candidate: &MentionCandidate) -> (String, String) {
  match candidate {
    MentionCandidate::Diff(diff) => (
      format!("@{}", diff.keyword()),
      diff.description().to_string(),
    ),
    MentionCandidate::Selection => (
      "@selection".to_string(),
      "Selected code in diff".to_string(),
    ),
    MentionCandidate::File(path) => {
      let name = path.rsplit('/').next().unwrap_or(path).to_string();
      (name, path.clone())
    }
  }
}

impl AgentChatPanel {
  fn render_list_item(
    &mut self,
    list_ix: usize,
    theme: &gpui_component::Theme,
    cx: &mut Context<Self>,
  ) -> gpui::AnyElement {
    let extras_before = self.extras_before_kinds();
    let element = if list_ix == self.runway_spacer_ix() {
      let reserved = if self.runway_active {
        self.runway_end_space
      } else {
        0.0
      };
      return div().h(px(reserved)).into_any_element();
    } else if list_ix < extras_before.len() {
      self.render_extra_before(extras_before[list_ix], theme, cx)
    } else {
      let item_ix = list_ix - extras_before.len();
      if item_ix < self.items.len() {
        self.render_item_at(item_ix, theme, cx)
      } else if let Some(kind) = self.extras_after_kind() {
        self.render_extra_after(kind, theme, cx)
      } else {
        div().into_any_element()
      }
    };
    let element = div()
      .w_full()
      .flex()
      .justify_center()
      .child(
        div()
          .w_full()
          .max_w(px(CONVERSATION_COLUMN_MAX_WIDTH_PX))
          .child(element),
      )
      .into_any_element();
    // The last content row (the runway spacer sits after it) clears the
    // bottom fade so a fully scrolled transcript is not read through it.
    let is_last = list_ix + 2 == self.total_list_items();
    div()
      .when(list_ix == 0, |this| this.pt_3())
      .when(is_last, |this| {
        this.pb(px(CONVERSATION_BOTTOM_FADE_PX + 8.0))
      })
      .child(element)
      .into_any_element()
  }

  fn render_extra_before(
    &mut self,
    kind: ExtraBeforeKind,
    theme: &gpui_component::Theme,
    cx: &mut Context<Self>,
  ) -> gpui::AnyElement {
    match kind {
      ExtraBeforeKind::MissingBinary => {
        let Status::MissingBinary { command, hint } = &self.status else {
          return div().into_any_element();
        };
        v_flex()
          .debug_selector(|| "agent-chat-missing-binary".to_string())
          .px_3()
          .gap_2()
          .child(
            div()
              .text_sm()
              .text_color(theme.danger)
              .child(format!("`{command}` not found on PATH")),
          )
          .child(
            div()
              .text_sm()
              .text_color(theme.foreground)
              .child(hint.clone()),
          )
          .into_any_element()
      }
      ExtraBeforeKind::Auth => self.render_auth_card(theme, cx),
    }
  }

  fn render_auth_card(
    &mut self,
    theme: &gpui_component::Theme,
    cx: &mut Context<Self>,
  ) -> gpui::AnyElement {
    let executable = self.backend.command.clone();
    let base_args = self.backend.args.clone();
    let mut card = v_flex()
      .px_3()
      .gap_2()
      .p_2()
      .border_1()
      .border_color(theme.border)
      .rounded(px(4.))
      .child(
        div()
          .text_xs()
          .text_color(theme.muted_foreground)
          .child("Sign-in options offered by the agent:".to_string()),
      );
    for method in self.auth_methods.clone() {
      let mut row = v_flex().gap_1();
      row = row.child(
        div()
          .text_sm()
          .text_color(theme.foreground)
          .child(method.name.clone()),
      );
      if let Some(desc) = method.description.clone() {
        row = row.child(
          div()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(desc),
        );
      }
      if let Some(cmd) = method.terminal_command.clone() {
        let shell_cmd = cmd.to_shell_string(&executable, &base_args);
        let preview = shell_cmd.clone();
        let copy_value = shell_cmd.clone();
        let copy_id = SharedString::from(format!("auth-copy-{}", method.id));
        let open_id = SharedString::from(format!("auth-open-{}", method.id));
        let launch_cmd = cmd.clone();
        let exec_owned = executable.to_string();
        let launch_args = base_args.clone();
        row = row.child(
          div()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(format!("`{preview}`")),
        );
        row = row.child(
          h_flex()
            .gap_2()
            .child(
              Button::new(open_id)
                .label("Open in Terminal")
                .small()
                .primary()
                .on_click(cx.listener(move |_, _, _, cx| {
                  if !launch_cmd.try_launch_terminal(&exec_owned, &launch_args) {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(copy_value.clone()));
                  }
                })),
            )
            .child(
              Button::new(copy_id)
                .label("Copy command")
                .small()
                .ghost()
                .on_click(cx.listener(move |_, _, _, cx| {
                  cx.write_to_clipboard(gpui::ClipboardItem::new_string(shell_cmd.clone()));
                })),
            ),
        );
      }
      card = card.child(row);
    }
    card.into_any_element()
  }

  fn render_generating(
    &self,
    theme: &gpui_component::Theme,
    cx: &mut Context<Self>,
  ) -> gpui::AnyElement {
    let connecting = matches!(self.status, Status::Connecting);
    let elapsed = self
      .turn_started_at
      .map(|t| t.elapsed().as_secs())
      .unwrap_or(0);
    let label_color = theme.muted_foreground;
    let brand_icon = crate::backend_icon(&self.backend_kind);
    let verb: SharedString = if connecting {
      format!("Connecting to {}...", self.backend.label).into()
    } else {
      let seed = working_word_seed(&self.current_conv.id);
      format!("{}...", working_word(seed, elapsed)).into()
    };
    let elapsed_label: Option<SharedString> =
      (!connecting && elapsed >= 2).then(|| format!("{elapsed}s").into());

    let mut row = h_flex()
      .gap_2()
      .items_center()
      .child(
        gpui_component::Icon::new(brand_icon)
          .small()
          .text_color(label_color),
      )
      .child(
        div()
          .text_xs()
          .text_color(label_color)
          .child(verb)
          .opacity(pulse_opacity(0.35, 1.0)),
      );
    if let Some(e) = elapsed_label {
      row = row.child(div().text_xs().text_color(theme.muted_foreground).child(e));
    }

    let mut container = v_flex()
      .debug_selector(|| "agent-chat-generating".to_string())
      .px_3()
      .pb_3()
      .gap_1()
      .child(row);
    if !self.pending_thought.is_empty() {
      container = container.child(self.render_thinking_peek(theme, cx));
    }
    if let Some(state) = self.pending_md_state.clone() {
      container = container.child(stateful_markdown_view(
        &state,
        &self.markdown_extensions,
        cx,
      ));
    } else if !self.pending_agent.is_empty() {
      container = container.child(markdown_view(
        "agent-chat-md-pending",
        &self.pending_agent,
        &self.markdown_extensions,
        cx,
      ));
    }
    container.into_any_element()
  }

  /// Live peek at the streaming thought: the tail of the buffer, dimmed and
  /// clipped to the bottom so the newest reasoning stays in view. It settles
  /// into the collapsed Thought item when the segment closes.
  fn render_thinking_peek(
    &self,
    theme: &gpui_component::Theme,
    _cx: &mut Context<Self>,
  ) -> gpui::AnyElement {
    let (tail, truncated) = thought_peek_tail(&self.pending_thought);
    v_flex()
      .debug_selector(|| "agent-chat-thinking-peek".to_string())
      .gap_1()
      .child(
        div()
          .text_xs()
          .text_color(theme.muted_foreground)
          .child("Thinking"),
      )
      .child(
        div()
          .relative()
          .max_h(px(THINKING_PEEK_MAX_HEIGHT_PX))
          .overflow_hidden()
          .flex()
          .flex_col()
          .justify_end()
          .child(
            // Plain dimmed text: rendered markdown would blast bold headings
            // through the peek's quiet intent.
            div()
              .pl_4()
              .text_xs()
              .text_color(theme.muted_foreground)
              .whitespace_normal()
              .child(SharedString::from(tail)),
          )
          .when(truncated, |this| {
            this.child(
              div()
                .debug_selector(|| "agent-chat-thinking-fade".to_string())
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(px(24.))
                .bg(gpui::linear_gradient(
                  180.,
                  gpui::linear_color_stop(theme.sidebar, 0.),
                  gpui::linear_color_stop(theme.sidebar.opacity(0.), 1.),
                )),
            )
          }),
      )
      .into_any_element()
  }

  fn render_extra_after(
    &mut self,
    kind: ExtraAfterKind,
    theme: &gpui_component::Theme,
    cx: &mut Context<Self>,
  ) -> gpui::AnyElement {
    match kind {
      ExtraAfterKind::Generating => self.render_generating(theme, cx),
      ExtraAfterKind::Error => {
        let Status::Error(e) = &self.status else {
          return div().into_any_element();
        };
        v_flex()
          .px_3()
          .pb_3()
          .gap_2()
          .items_start()
          .child(div().text_sm().text_color(theme.danger).child(e.clone()))
          .child(
            div()
              .debug_selector(|| "agent-chat-reconnect".to_string())
              .child(
                Button::new("agent-chat-reconnect")
                  .label("Reconnect")
                  .small()
                  .primary()
                  .on_click(cx.listener(|panel, _, _, cx| panel.respawn_session(cx))),
              ),
          )
          .into_any_element()
      }
    }
  }

  fn render_item_at(
    &mut self,
    idx: usize,
    theme: &gpui_component::Theme,
    cx: &mut Context<Self>,
  ) -> gpui::AnyElement {
    // A settled turn's work lives folded behind its summary card.
    if let Some(summary_idx) = hiding_turn_summary(&self.items, idx)
      && !matches!(
        self.items.get(summary_idx),
        Some(ChatItem::TurnSummary(s)) if s.work_expanded
      )
    {
      return Empty.into_any_element();
    }
    let has_continuation_trailer =
      matches!(self.extras_after_kind(), Some(ExtraAfterKind::Generating));
    let total = self.items.len();
    // The timeline rail stops when the next item starts a new visual group:
    // a user-authored message or a checkpoint divider.
    let next_starts_new_group = self
      .items
      .get(idx + 1)
      .map(|i| {
        matches!(
          i,
          ChatItem::Message(ChatMessage {
            role: ChatRole::User | ChatRole::ReviewExport,
            ..
          }) | ChatItem::Checkpoint(_)
        )
      })
      .unwrap_or(false);
    let is_end_of_group = if idx + 1 == total {
      !has_continuation_trailer
    } else {
      next_starts_new_group
    };
    let is_last_row = is_end_of_group;

    let item = self.items[idx].clone();
    let registry = self.selection_registry.clone();
    let terminal_store = self.terminal_store.clone();
    let item_id_base = (idx as u64) << 32;
    let element: gpui::AnyElement = match &item {
      ChatItem::Message(m) => match m.role {
        ChatRole::User if self.editing_message == Some(idx) => {
          let editor = self.edit_input.clone();
          v_flex()
            .debug_selector(|| "agent-chat-edit-message".to_string())
            .mb_3()
            .gap_1()
            .px_2()
            .py_1p5()
            .rounded(theme.radius)
            .bg(theme.input_background())
            .border_1()
            .border_color(theme.ring)
            .children(editor.map(|input| Textarea::new(&input).appearance(false).w_full()))
            .child(
              h_flex()
                .gap_1()
                .justify_end()
                .child(
                  Button::new(("agent-chat-edit-cancel", idx))
                    .label("Cancel")
                    .xsmall()
                    .ghost()
                    .on_click(cx.listener(|panel, _, _, cx| panel.cancel_message_edit(cx))),
                )
                .child(
                  Button::new(("agent-chat-edit-send", idx))
                    .label("Send")
                    .xsmall()
                    .primary()
                    .tooltip("Restores the checkpoint and restarts from this message")
                    .on_click(cx.listener(|panel, _, _, cx| panel.submit_message_edit(cx))),
                ),
            )
            .into_any_element()
        }
        ChatRole::User => {
          let can_edit = !self.in_flight && checkpoint_ref_before(&self.items, idx).is_some();
          v_flex()
            .group("chat-user-msg")
            .mb_3()
            .gap_0p5()
            .items_end()
            .child(
              v_flex()
                .items_end()
                .gap_1()
                .max_w(px(560.))
                .when(!m.image_data.is_empty(), |this| {
                  this.child(h_flex().gap_1().flex_wrap().justify_end().children(
                    m.image_data.iter().map(|image| {
                      div()
                        .rounded(px(8.))
                        .overflow_hidden()
                        .border_1()
                        .border_color(theme.border)
                        .child(
                          gpui::img(image.clone())
                            .max_w(px(200.))
                            .max_h(px(160.))
                            .object_fit(gpui::ObjectFit::ScaleDown),
                        )
                    }),
                  ))
                })
                .when(m.image_data.is_empty() && m.images > 0, |this| {
                  this.child(div().text_xs().text_color(theme.muted_foreground).child(
                    if m.images == 1 {
                      "1 image attached".to_string()
                    } else {
                      format!("{} images attached", m.images)
                    },
                  ))
                })
                .child(
                  div()
                    .px_3()
                    .py_2()
                    .rounded(px(10.))
                    .bg(theme.secondary)
                    .text_sm()
                    .text_color(theme.foreground)
                    .child(selectable_text::SelectableText::new(
                      item_id_base,
                      SharedString::from(m.text.clone()),
                      Vec::new(),
                      registry.clone(),
                    )),
                ),
            )
            .child(
              // Capped so the hover-only actions don't reserve a visible gap;
              // the buttons overflow into the block's bottom margin instead.
              h_flex()
                .h(px(10.))
                .items_start()
                .justify_end()
                .gap_0p5()
                .opacity(0.)
                .group_hover("chat-user-msg", |this| this.opacity(1.))
                .child(
                  div().debug_selector(|| "chat-msg-copy".to_string()).child(
                    Clipboard::new(SharedString::from(format!("chat-msg-copy-{idx}")))
                      .value(SharedString::from(m.text.clone()))
                      .tooltip("Copy message"),
                  ),
                )
                .when(can_edit, |this| {
                  this.child(
                    div().debug_selector(|| "chat-msg-edit".to_string()).child(
                      Button::new(("chat-msg-edit", idx))
                        .icon(UiIconName::SquarePen)
                        .xsmall()
                        .ghost()
                        .tooltip("Edit and restart from here")
                        .on_click(cx.listener(move |panel, _, window, cx| {
                          panel.begin_message_edit(idx, window, cx);
                        })),
                    ),
                  )
                }),
            )
            .into_any_element()
        }
        ChatRole::Agent => timeline_row(
          v_flex()
            .group("chat-agent-msg")
            .gap_0p5()
            .child(markdown_view(
              ("agent-chat-md", idx),
              &m.text,
              &self.markdown_extensions,
              cx,
            ))
            .child(
              // Capped so the hover-only copy doesn't reserve a visible gap;
              // the button overflows into the row's bottom padding instead.
              h_flex()
                .h(px(10.))
                .items_start()
                .gap_0p5()
                .opacity(0.)
                .group_hover("chat-agent-msg", |this| this.opacity(1.))
                .child(
                  div()
                    .debug_selector(|| "chat-msg-copy-agent".to_string())
                    .child(
                      Clipboard::new(SharedString::from(format!("chat-msg-copy-agent-{idx}")))
                        .value(SharedString::from(m.text.clone()))
                        .tooltip("Copy message"),
                    ),
                ),
            )
            .into_any_element(),
          theme,
          is_last_row,
        ),
        ChatRole::System => timeline_row(
          div()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(selectable_text::SelectableText::new(
              item_id_base | 0x1,
              SharedString::from(m.text.clone()),
              Vec::new(),
              registry.clone(),
            ))
            .into_any_element(),
          theme,
          is_last_row,
        ),
        ChatRole::ReviewExport => {
          let label = review_export_label(&m.text);
          div()
            .mb_3()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .overflow_hidden()
            .child(
              gpui::div()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_1p5()
                .border_b_1()
                .border_color(theme.border)
                .child(
                  gpui_component::Icon::new(UiIconName::MessageCircleReply)
                    .size_4()
                    .text_color(theme.warning),
                )
                .child(
                  div()
                    .text_xs()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.warning)
                    .child(label),
                ),
            )
            .child(div().px_3().py_2().text_sm().child(markdown_view(
              ("agent-chat-review-md", idx),
              &m.text,
              &self.markdown_extensions,
              cx,
            )))
            .into_any_element()
        }
      },
      ChatItem::Tool(t) => {
        let bullet = match t.status {
          ToolCallStatus::Completed => theme.status_green(),
          ToolCallStatus::Failed => theme.danger,
          ToolCallStatus::InProgress => theme.warning,
          _ => theme.muted_foreground,
        };
        if let Some((start, end, _)) = tool_group_span(&self.items, idx) {
          let expanded = self.tool_group_expanded(start, end);
          if idx == start {
            let group_bullet = if self.items[start..=end].iter().any(|item| {
              matches!(item, ChatItem::Tool(t) if matches!(t.status, ToolCallStatus::InProgress))
            }) {
              theme.warning
            } else {
              theme.muted_foreground
            };
            let header = self.render_tool_group_header(start, end, expanded, theme, cx);
            let content = if expanded {
              v_flex()
                .gap_1()
                .child(header)
                .child(render_tool_call(
                  t,
                  theme,
                  item_id_base,
                  &registry,
                  terminal_store.as_ref(),
                  cx,
                ))
                .into_any_element()
            } else {
              header
            };
            let group_is_last = end + 1 == total && !has_continuation_trailer;
            timeline_row_with_color(content, theme, group_bullet, !expanded && group_is_last)
          } else if expanded {
            timeline_row_with_color(
              render_tool_call(
                t,
                theme,
                item_id_base,
                &registry,
                terminal_store.as_ref(),
                cx,
              ),
              theme,
              bullet,
              is_last_row,
            )
          } else {
            return Empty.into_any_element();
          }
        } else {
          timeline_row_with_color(
            render_tool_call(
              t,
              theme,
              item_id_base,
              &registry,
              terminal_store.as_ref(),
              cx,
            ),
            theme,
            bullet,
            is_last_row,
          )
        }
      }
      ChatItem::Permission(p) => timeline_row(
        render_permission(p, theme, &registry, cx),
        theme,
        is_last_row,
      ),
      ChatItem::Plan(p) => {
        timeline_row_with_color(render_plan(p, theme), theme, theme.primary, is_last_row)
      }
      ChatItem::Thought(t) => {
        let span = tool_group_span(&self.items, idx);
        match span {
          Some((start, end, _)) => {
            let expanded = self.tool_group_expanded(start, end);
            if idx == start {
              // A span can open on a thought; the header lives on its first row.
              let header = self.render_tool_group_header(start, end, expanded, theme, cx);
              let content = if expanded {
                v_flex()
                  .gap_1()
                  .child(header)
                  .child(render_thought(idx, t, theme, &self.markdown_extensions, cx))
                  .into_any_element()
              } else {
                header
              };
              let group_is_last = end + 1 == total && !has_continuation_trailer;
              timeline_row_with_color(
                content,
                theme,
                theme.muted_foreground,
                !expanded && group_is_last,
              )
            } else if expanded {
              timeline_row(
                render_thought(idx, t, theme, &self.markdown_extensions, cx),
                theme,
                is_last_row,
              )
            } else {
              return Empty.into_any_element();
            }
          }
          None => timeline_row(
            render_thought(idx, t, theme, &self.markdown_extensions, cx),
            theme,
            is_last_row,
          ),
        }
      }
      ChatItem::Checkpoint(marker) => {
        let ref_name = marker.ref_name.clone();
        // A trailing marker has nothing after it to undo, and a running turn
        // must finish before history can move.
        let can_roll_back = idx + 1 < total && !self.in_flight;
        if !can_roll_back {
          // An inert divider is noise; the marker returns when actionable.
          return Empty.into_any_element();
        }
        let hairline = || div().flex_1().h_px().bg(theme.border.opacity(0.5));

        let center: gpui::AnyElement = if can_roll_back {
          div()
            .id(("chat-checkpoint-rollback", idx))
            .debug_selector(|| "chat-checkpoint-rollback".to_string())
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .py(px(2.))
            .rounded_full()
            .border_1()
            .border_color(theme.border.opacity(0.7))
            .text_xs()
            .text_color(theme.muted_foreground)
            .cursor_pointer()
            .hover(|s| {
              s.text_color(theme.foreground)
                .border_color(theme.muted_foreground)
                .bg(theme.secondary_hover)
            })
            .child(
              gpui_component::Icon::new(UiIconName::History)
                .size_3()
                .text_color(theme.muted_foreground),
            )
            .child("Roll back")
            .tooltip(|window, cx| {
              gpui_component::tooltip::Tooltip::new(
                "Restore files and conversation to this checkpoint",
              )
              .build(window, cx)
            })
            .on_click(cx.listener(move |_, _, _, cx| {
              cx.emit(AgentChatPanelEvent::RollbackRequested {
                ref_name: ref_name.clone(),
              });
            }))
            .into_any_element()
        } else {
          div()
            .flex()
            .items_center()
            .gap_1()
            .text_xs()
            .text_color(theme.muted_foreground.opacity(0.8))
            .child(
              gpui_component::Icon::new(UiIconName::History)
                .size_3()
                .text_color(theme.muted_foreground.opacity(0.8)),
            )
            .child("Checkpoint")
            .into_any_element()
        };

        div()
          .id(("chat-checkpoint", idx))
          .my_2()
          .flex()
          .items_center()
          .gap_3()
          .child(hairline())
          .child(center)
          .child(hairline())
          .into_any_element()
      }
      ChatItem::TurnSummary(s) => self.render_turn_summary(idx, s, theme, cx),
    };
    div().px_3().child(element).into_any_element()
  }

  fn render_turn_summary(
    &self,
    idx: usize,
    view: &TurnSummaryView,
    theme: &gpui_component::Theme,
    cx: &mut Context<Self>,
  ) -> gpui::AnyElement {
    let added: u32 = view.files.iter().map(|f| f.added).sum();
    let removed: u32 = view.files.iter().map(|f| f.removed).sum();
    let file_count = view.files.len();
    let visible = if view.expanded {
      file_count
    } else {
      file_count.min(TURN_SUMMARY_COLLAPSED_FILES)
    };
    let hidden = file_count - visible;
    let undone = view.undone;
    let can_undo = view.checkpoint_ref.is_some()
      && !self.in_flight
      && !undone
      && is_trailing_turn_summary(&self.items, idx);
    let undo_ref = view.checkpoint_ref.clone();
    let review_path = (!undone)
      .then(|| view.files.first().map(|f| PathBuf::from(&f.path)))
      .flatten();
    let stat_green = if undone {
      theme.muted_foreground
    } else {
      theme.status_green()
    };
    let stat_red = if undone {
      theme.muted_foreground
    } else {
      theme.status_red()
    };
    let row_text = if undone {
      theme.muted_foreground
    } else {
      theme.foreground
    };

    let title = if file_count == 1 {
      "Edited 1 file".to_string()
    } else {
      format!("Edited {file_count} files")
    };

    let header = h_flex()
      .items_center()
      .gap_2()
      .px_3()
      .py_2()
      .child(
        div()
          .flex_shrink_0()
          .size(px(20.))
          .rounded(px(5.))
          .bg(theme.secondary)
          .flex()
          .items_center()
          .justify_center()
          .child(
            gpui_component::Icon::new(UiIconName::FileDiff)
              .size_3()
              .text_color(theme.muted_foreground),
          ),
      )
      .child(
        h_flex()
          .flex_1()
          .min_w(px(0.))
          .items_baseline()
          .gap_2()
          .child(
            div()
              .text_sm()
              .font_weight(FontWeight::MEDIUM)
              .text_color(theme.foreground)
              .child(title),
          )
          .child(
            h_flex()
              .gap_1()
              .text_xs()
              .child(div().text_color(stat_green).child(format!("+{added}")))
              .child(div().text_color(stat_red).child(format!("-{removed}"))),
          ),
      )
      .when(undone, |this| {
        this.child(
          div()
            .debug_selector(|| "turn-summary-undone".to_string())
            .text_xs()
            .text_color(theme.muted_foreground)
            .child("Undone"),
        )
      })
      .when(can_undo, |this| {
        this.child(
          div()
            .debug_selector(|| "turn-summary-undo".to_string())
            .child(
              Button::new(("turn-summary-undo", idx))
                .label("Undo")
                .xsmall()
                .ghost()
                .tooltip("Revert this turn's file changes, keep the conversation")
                .on_click(cx.listener(move |_, _, _, cx| {
                  if let Some(ref_name) = undo_ref.clone() {
                    cx.emit(AgentChatPanelEvent::UndoTurnRequested { ref_name });
                  }
                })),
            ),
        )
      })
      .when_some(review_path, |this, path| {
        this.child(
          div()
            .debug_selector(|| "turn-summary-review".to_string())
            .child(
              Button::new(("turn-summary-review", idx))
                .label("Review")
                .xsmall()
                .outline()
                .tooltip("Open the changes in the diff view")
                .on_click(cx.listener(move |_, _, _, cx| {
                  cx.emit(AgentChatPanelEvent::OpenPath {
                    path: path.clone(),
                    line: None,
                  });
                })),
            ),
        )
      });

    let mut card = v_flex()
      .debug_selector(|| "chat-turn-summary".to_string())
      .my_2()
      .border_1()
      .border_color(theme.border)
      .rounded(px(8.))
      .overflow_hidden()
      .child(header);

    let steps = folded_step_count(&self.items, idx);
    if steps > 0 {
      let work_expanded = view.work_expanded;
      let mut label = String::new();
      if let Some(secs) = view.duration_secs {
        label.push_str(&format!("Worked for {}", format_turn_duration(secs)));
        label.push_str(" · ");
      }
      label.push_str(&format!(
        "{steps} {}",
        if steps == 1 { "step" } else { "steps" }
      ));
      card = card.child(
        h_flex()
          .id(("turn-summary-work", idx))
          .debug_selector(|| "turn-summary-work".to_string())
          .items_center()
          .gap_1()
          .px_3()
          .py_1p5()
          .border_t_1()
          .border_color(theme.border.opacity(0.6))
          .cursor_pointer()
          .text_xs()
          .text_color(theme.muted_foreground)
          .hover(|s| s.bg(theme.secondary_hover))
          .on_click(cx.listener(move |panel, _, _, cx| {
            panel.toggle_turn_work(idx, cx);
          }))
          .child(label)
          .child(
            gpui_component::Icon::new(if work_expanded {
              IconName::ChevronDown
            } else {
              IconName::ChevronRight
            })
            .size_3(),
          ),
      );
    }

    for (row_ix, file) in view.files.iter().take(visible).enumerate() {
      let path = PathBuf::from(&file.path);
      card = card.child(
        h_flex()
          .id(("turn-summary-file", (idx << 16) | row_ix))
          .items_center()
          .gap_2()
          .px_3()
          .py_1p5()
          .border_t_1()
          .border_color(theme.border.opacity(0.6))
          .cursor_pointer()
          .hover(|s| s.bg(theme.secondary_hover))
          .on_click(cx.listener(move |_, _, _, cx| {
            cx.emit(AgentChatPanelEvent::OpenPath {
              path: path.clone(),
              line: None,
            });
          }))
          .child(
            div()
              .flex_1()
              .min_w(px(0.))
              .text_xs()
              .truncate()
              .text_color(row_text)
              .child(file.path.clone()),
          )
          .child(
            h_flex()
              .flex_shrink_0()
              .gap_1()
              .text_xs()
              .child(
                div()
                  .text_color(stat_green)
                  .child(format!("+{}", file.added)),
              )
              .child(
                div()
                  .text_color(stat_red)
                  .child(format!("-{}", file.removed)),
              ),
          ),
      );
    }

    if hidden > 0 || (view.expanded && file_count > TURN_SUMMARY_COLLAPSED_FILES) {
      let expanded = view.expanded;
      let label = if expanded {
        "Show fewer files".to_string()
      } else {
        format!(
          "Show {hidden} more {}",
          if hidden == 1 { "file" } else { "files" }
        )
      };
      card = card.child(
        h_flex()
          .id(("turn-summary-toggle", idx))
          .items_center()
          .gap_1()
          .px_3()
          .py_1p5()
          .border_t_1()
          .border_color(theme.border.opacity(0.6))
          .cursor_pointer()
          .text_xs()
          .text_color(theme.muted_foreground)
          .hover(|s| s.bg(theme.secondary_hover))
          .on_click(cx.listener(move |panel, _, _, cx| {
            if let Some(ChatItem::TurnSummary(s)) = panel.items.get_mut(idx) {
              s.expanded = !expanded;
              let list_ix = panel.list_ix_for_item(idx);
              panel.mark_item_changed_at(list_ix);
              cx.notify();
            }
          }))
          .child(label)
          .child(
            gpui_component::Icon::new(if expanded {
              IconName::ChevronUp
            } else {
              IconName::ChevronDown
            })
            .size_3(),
          ),
      );
    }

    card.into_any_element()
  }

  fn render_slash_overlay(&mut self, cx: &mut Context<Self>) -> AnyElement {
    let Some((_, candidates)) = self.slash_snapshot(cx) else {
      return Empty.into_any_element();
    };
    let selected_ix = self.slash_selected_ix.min(candidates.len() - 1);
    let theme = cx.theme().clone();
    let entity = cx.entity();

    deferred(
      div()
        .id("agent-slash-menu")
        .debug_selector(|| "agent-slash-menu".to_string())
        .absolute()
        .left_0()
        .bottom(gpui::relative(1.0))
        .mb_1()
        .w(px(360.))
        .max_h(px(240.))
        .overflow_hidden()
        .occlude()
        .bg(theme.popover)
        .text_color(theme.popover_foreground)
        .border_1()
        .border_color(theme.border)
        .rounded(theme.radius)
        .shadow_lg()
        .p_1()
        .children(candidates.into_iter().enumerate().map(|(ix, command)| {
          let selected = ix == selected_ix;
          let entity_click = entity.clone();
          let entity_hover = entity.clone();
          h_flex()
            .id(("agent-slash-item", ix))
            .w_full()
            .items_center()
            .justify_between()
            .gap_2()
            .px_2()
            .py_1()
            .rounded(theme.radius)
            .text_xs()
            .line_height(gpui::relative(1.2))
            .cursor_pointer()
            .when(selected, |this| {
              this.bg(theme.accent).text_color(theme.accent_foreground)
            })
            .hover(|this| this.bg(theme.accent.opacity(0.8)))
            .on_mouse_move(move |_, _, cx| {
              entity_hover.update(cx, |panel, cx| {
                if panel.slash_selected_ix != ix {
                  panel.slash_selected_ix = ix;
                  cx.notify();
                }
              });
            })
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
              entity_click.update(cx, |panel, cx| {
                if let Some((_, candidates)) = panel.slash_snapshot(cx)
                  && let Some(command) = candidates.get(ix)
                {
                  panel.insert_slash_command(&command.name.clone(), window, cx);
                }
                cx.stop_propagation();
              });
            })
            .child(SharedString::from(format!("/{}", command.name)))
            .child(
              div()
                .text_color(theme.muted_foreground)
                .truncate()
                .child(SharedString::from(command.description.clone())),
            )
        })),
    )
    .with_priority(2)
    .into_any_element()
  }

  fn render_mention_overlay(&mut self, cx: &mut Context<Self>) -> AnyElement {
    let Some((_, candidates)) = self.mention_snapshot(cx) else {
      return Empty.into_any_element();
    };
    let selected_ix = self.mention_selected_ix.min(candidates.len() - 1);
    let theme = cx.theme().clone();
    let entity = cx.entity();

    deferred(
      div()
        .id("agent-mention-menu")
        .debug_selector(|| "agent-mention-menu".to_string())
        .absolute()
        .left_0()
        .bottom(gpui::relative(1.0))
        .mb_1()
        .w(px(360.))
        .max_h(px(240.))
        .overflow_hidden()
        .occlude()
        .bg(theme.popover)
        .text_color(theme.popover_foreground)
        .border_1()
        .border_color(theme.border)
        .rounded(theme.radius)
        .shadow_lg()
        .p_1()
        .children(candidates.into_iter().enumerate().map(|(ix, candidate)| {
          let selected = ix == selected_ix;
          let (primary, secondary) = mention_labels(&candidate);
          let entity_click = entity.clone();
          let entity_hover = entity.clone();
          h_flex()
            .id(("agent-mention-item", ix))
            .w_full()
            .items_center()
            .justify_between()
            .gap_2()
            .px_2()
            .py_1()
            .rounded(theme.radius)
            .text_xs()
            .line_height(gpui::relative(1.2))
            .cursor_pointer()
            .when(selected, |this| {
              this.bg(theme.accent).text_color(theme.accent_foreground)
            })
            .hover(|this| this.bg(theme.accent.opacity(0.8)))
            .on_mouse_move(move |_, _, cx| {
              entity_hover.update(cx, |panel, cx| {
                if panel.mention_selected_ix != ix {
                  panel.mention_selected_ix = ix;
                  cx.notify();
                }
              });
            })
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
              entity_click.update(cx, |panel, cx| {
                if let Some((trigger, candidates)) = panel.mention_snapshot(cx)
                  && let Some(candidate) = candidates.get(ix)
                {
                  panel.insert_mention(&trigger, &candidate.clone(), window, cx);
                }
                cx.stop_propagation();
              });
            })
            .child(SharedString::from(primary))
            .child(
              div()
                .text_color(theme.muted_foreground)
                .truncate()
                .child(SharedString::from(secondary)),
            )
        })),
    )
    .with_priority(2)
    .into_any_element()
  }

  fn render_tool_group_header(
    &self,
    start: usize,
    end: usize,
    expanded: bool,
    theme: &gpui_component::Theme,
    cx: &mut Context<Self>,
  ) -> gpui::AnyElement {
    let tools: Vec<&ToolCallView> = self.items[start..=end]
      .iter()
      .filter_map(|item| match item {
        ChatItem::Tool(t) => Some(t),
        _ => None,
      })
      .collect();
    let summary = tool_group_summary(&tools);
    let chevron = if expanded {
      IconName::ChevronDown
    } else {
      IconName::ChevronRight
    };
    h_flex()
      .id(("agent-tool-group", start))
      .debug_selector(|| "agent-tool-group".to_string())
      .gap_1p5()
      .items_center()
      .cursor_pointer()
      .hover(|s| s.text_color(theme.foreground))
      .on_click(cx.listener(move |panel, _, _, cx| panel.toggle_tool_group(start, cx)))
      .child(
        gpui_component::Icon::new(chevron)
          .size_3()
          .text_color(theme.muted_foreground),
      )
      .child(
        div()
          .text_xs()
          .text_color(theme.muted_foreground)
          .child(SharedString::from(summary)),
      )
      .into_any_element()
  }
}
