use std::{
  collections::HashMap,
  hash::{DefaultHasher, Hash, Hasher},
  ops::Range,
  path::Path,
  sync::{Arc, Mutex},
};

use crate::constants::*;
use crate::height_estimation::*;
use crate::image::*;
#[cfg(test)]
use crate::parse::*;
#[cfg(test)]
use crate::parse_html::*;
use crate::parsed_cache::parse_markdown_for_render;
#[cfg(test)]
use crate::parsed_cache::{PARSED_MARKDOWN_CACHE_MAX_ENTRIES, ParsedMarkdownCache};
use crate::preview_segments::{MarkdownRenderSegment, split_markdown_preview_segments};
use crate::selection::*;
use crate::types::*;
pub use crate::types::{
  Block, CodeBlock, Details, GithubCodeReferencePreview, GithubDiffLine, GithubDiffLineKind,
  GithubIssueReferenceContext, Inline, LinkAction, List, MarkdownRenderState, ParsedMarkdown,
  SuggestionActionContext, SuggestionContext, Table,
};
use gpui::{
  AnyElement, App, CursorStyle, Div, MouseButton, SharedString, Window, div, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _, avatar::Avatar,
  clipboard::Clipboard, h_flex, scroll::ScrollableElement, v_flex,
};
#[cfg(test)]
use syntax::TokenType;
use syntax::{HighlightSpan, SyntaxHighlighter, languages};
use ui::{ScrollAxes, restrict_scroll_to_wheel_axis, scrollable_node};
use unicode_segmentation::UnicodeSegmentation;

type BlockRenderFn = dyn Fn(AnyElement, &App) -> AnyElement + Send + Sync;
type HeadingRenderFn = dyn Fn(u8, AnyElement, &App) -> AnyElement + Send + Sync;
type CodeBlockRenderFn = dyn Fn(&CodeBlock, &App) -> AnyElement + Send + Sync;
type ListItemRenderFn = dyn Fn(ListItemView, &App) -> AnyElement + Send + Sync;
type ThematicBreakRenderFn = dyn Fn(&App) -> AnyElement + Send + Sync;
type TableRenderFn = dyn Fn(&Table, &App) -> AnyElement + Send + Sync;
type SuggestionActionRenderFn = dyn Fn(SuggestionActionContext, &App) -> AnyElement + Send + Sync;
pub(crate) type LinkHandlerFn = dyn Fn(&str, &mut Window, &mut App) -> LinkAction + Send + Sync;
const WORD_DIFF_MAX_COMBINED_BYTES: usize = 2_048;

// Data types imported from crate::types

#[derive(Clone, Default)]
pub struct RenderOverrides {
  pub paragraph: Option<Arc<BlockRenderFn>>,
  pub heading: Option<Arc<HeadingRenderFn>>,
  pub code_block: Option<Arc<CodeBlockRenderFn>>,
  pub list: Option<Arc<BlockRenderFn>>,
  pub list_item: Option<Arc<ListItemRenderFn>>,
  pub block_quote: Option<Arc<BlockRenderFn>>,
  pub thematic_break: Option<Arc<ThematicBreakRenderFn>>,
  pub table: Option<Arc<TableRenderFn>>,
}

// MarkdownRenderState, SelectionRange, SelectionState, ActiveSelection, clamp_to_char_boundary
// imported from crate::types

#[derive(Clone, Default)]
pub struct MarkdownRenderOptions {
  pub on_link: Option<Arc<LinkHandlerFn>>,
  pub overrides: RenderOverrides,
  pub state: MarkdownRenderState,
  pub scope_id: Option<usize>,
  pub github_code_reference_previews: Option<Arc<HashMap<Arc<str>, GithubCodeReferencePreview>>>,
  pub github_issue_reference_context: Option<GithubIssueReferenceContext>,
  pub expand_code_blocks: bool,
  pub hardbreaks: bool,
  pub image_base_url: Option<SharedString>,
  pub syntax_cache: Option<Arc<crate::syntax_cache::SyntaxHighlightCache>>,
  pub asset_url_resolver: Option<Arc<dyn Fn(&str) -> Option<String> + Send + Sync>>,
  pub suggestion_context: Option<SuggestionContext>,
  pub suggestion_action: Option<Arc<SuggestionActionRenderFn>>,
}

impl MarkdownRenderOptions {
  pub fn with_on_link(handler: Arc<LinkHandlerFn>) -> Self {
    Self {
      on_link: Some(handler),
      ..Default::default()
    }
  }

  pub fn overrides(mut self, overrides: RenderOverrides) -> Self {
    self.overrides = overrides;
    self
  }

  pub fn with_state(mut self, state: MarkdownRenderState) -> Self {
    self.state = state;
    self
  }

  pub fn with_scope_id(mut self, scope_id: usize) -> Self {
    self.scope_id = Some(scope_id);
    self
  }

  pub fn with_github_code_reference_previews(
    mut self,
    previews: Arc<HashMap<Arc<str>, GithubCodeReferencePreview>>,
  ) -> Self {
    self.github_code_reference_previews = Some(previews);
    self
  }

  pub fn with_github_issue_reference_context(
    mut self,
    owner: impl AsRef<str>,
    repo: impl AsRef<str>,
  ) -> Self {
    let owner = owner.as_ref().trim();
    let repo = repo.as_ref().trim();
    if owner.is_empty() || repo.is_empty() {
      self.github_issue_reference_context = None;
      return self;
    }

    self.github_issue_reference_context = Some(GithubIssueReferenceContext {
      owner: Arc::from(owner.to_string()),
      repo: Arc::from(repo.to_string()),
    });
    self
  }

  pub fn with_expanded_code_blocks(mut self) -> Self {
    self.expand_code_blocks = true;
    self
  }

  pub fn with_image_base_url(mut self, image_base_url: impl Into<SharedString>) -> Self {
    self.image_base_url = Some(image_base_url.into());
    self
  }

  pub fn with_syntax_cache(
    mut self,
    cache: Arc<crate::syntax_cache::SyntaxHighlightCache>,
  ) -> Self {
    self.syntax_cache = Some(cache);
    self
  }

  pub fn with_asset_url_resolver(
    mut self,
    resolver: Arc<dyn Fn(&str) -> Option<String> + Send + Sync>,
  ) -> Self {
    self.asset_url_resolver = Some(resolver);
    self
  }

  pub fn with_hardbreaks(mut self) -> Self {
    self.hardbreaks = true;
    self
  }

  pub fn with_suggestion_context(mut self, ctx: SuggestionContext) -> Self {
    self.suggestion_context = Some(ctx);
    self
  }

  pub fn with_suggestion_action(mut self, render: Arc<SuggestionActionRenderFn>) -> Self {
    self.suggestion_action = Some(render);
    self
  }
}

pub struct ListItemView {
  pub bullet: String,
  pub checked: Option<bool>,
  pub content: AnyElement,
}

// Constants imported from crate::constants
// BadgeImageSource, BadgeResolveState, Segment, ParsedMarkdown imported from crate::types

pub fn render_parsed_markdown(
  parsed: &ParsedMarkdown,
  options: &MarkdownRenderOptions,
  cx: &App,
) -> AnyElement {
  let scope_id = resolve_scope_id_for_parsed(parsed, options);
  let mut ctx = RenderContext::new(scope_id);
  render_blocks(&parsed.blocks, options, 0, cx, &mut ctx).into_any_element()
}

fn markdown_source_scope_id(source: &str, state: &MarkdownRenderState) -> usize {
  let mut hasher = DefaultHasher::new();
  source.hash(&mut hasher);
  scoped_id_for_state(hasher.finish() as usize, state)
}

fn short_github_reference(reference: &str) -> String {
  let trimmed = reference.trim();
  if trimmed.len() > 7 && trimmed.chars().all(|ch| ch.is_ascii_hexdigit()) {
    return trimmed.chars().take(7).collect();
  }
  if trimmed.len() > 24 {
    let mut shortened: String = trimmed.chars().take(24).collect();
    shortened.push_str("...");
    return shortened;
  }
  trimmed.to_string()
}

fn github_diff_line_background(kind: GithubDiffLineKind, cx: &App) -> gpui::Hsla {
  let theme = cx.theme();
  let ui_theme = ui::Theme::new(theme.is_dark());
  match kind {
    GithubDiffLineKind::Removed => ui_theme.diff_removed_background(),
    GithubDiffLineKind::Added => ui_theme.diff_added_background(),
    GithubDiffLineKind::Context => theme.background,
  }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DiffWordHighlight {
  ranges: Vec<Range<usize>>,
  background: InlineBackground,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IdentifierCharKind {
  Lower,
  Upper,
  Digit,
  Underscore,
  Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct WordToken {
  text: String,
  range: Range<usize>,
}

fn identifier_char_kind(ch: char) -> IdentifierCharKind {
  if ch == '_' {
    IdentifierCharKind::Underscore
  } else if ch.is_lowercase() {
    IdentifierCharKind::Lower
  } else if ch.is_uppercase() {
    IdentifierCharKind::Upper
  } else if ch.is_numeric() {
    IdentifierCharKind::Digit
  } else {
    IdentifierCharKind::Other
  }
}

fn split_identifier_token_ranges(segment: &str) -> Vec<Range<usize>> {
  let chars: Vec<_> = segment.char_indices().collect();
  if chars.is_empty() {
    return Vec::new();
  }

  let mut ranges = Vec::new();
  let mut start = 0usize;

  for idx in 1..chars.len() {
    let (byte_offset, current) = chars[idx];
    let (_, previous) = chars[idx - 1];
    let previous_kind = identifier_char_kind(previous);
    let current_kind = identifier_char_kind(current);
    let next_kind = chars
      .get(idx + 1)
      .map(|(_, next)| identifier_char_kind(*next));

    let should_split = match (previous_kind, current_kind) {
      (IdentifierCharKind::Underscore, _) | (_, IdentifierCharKind::Underscore) => true,
      (IdentifierCharKind::Digit, IdentifierCharKind::Digit) => false,
      (IdentifierCharKind::Digit, _) | (_, IdentifierCharKind::Digit) => true,
      (IdentifierCharKind::Lower, IdentifierCharKind::Upper) => true,
      (IdentifierCharKind::Upper, IdentifierCharKind::Upper) => {
        next_kind == Some(IdentifierCharKind::Lower)
      }
      _ => false,
    };

    if should_split {
      ranges.push(start..byte_offset);
      start = byte_offset;
    }
  }

  ranges.push(start..segment.len());
  ranges
}

fn word_tokens(text: &str, include_whitespace: bool) -> Vec<WordToken> {
  let mut tokens = Vec::new();
  for (idx, segment) in text.split_word_bound_indices() {
    if !include_whitespace && segment.trim().is_empty() {
      continue;
    }
    let subranges = split_identifier_token_ranges(segment);
    if subranges.is_empty() {
      tokens.push(WordToken {
        text: segment.to_string(),
        range: idx..idx + segment.len(),
      });
      continue;
    }

    for subrange in subranges {
      tokens.push(WordToken {
        text: segment[subrange.clone()].to_string(),
        range: idx + subrange.start..idx + subrange.end,
      });
    }
  }
  tokens
}

fn merge_ranges(mut ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
  ranges.sort_by_key(|range| range.start);
  let mut merged: Vec<Range<usize>> = Vec::new();
  for range in ranges {
    if let Some(last) = merged.last_mut()
      && range.start <= last.end
    {
      last.end = last.end.max(range.end);
      continue;
    }
    merged.push(range);
  }
  merged
}

fn word_diff_ranges(old_text: &str, new_text: &str) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
  if old_text == new_text {
    return (Vec::new(), Vec::new());
  }

  if old_text.len().saturating_add(new_text.len()) > WORD_DIFF_MAX_COMBINED_BYTES {
    return (Vec::new(), Vec::new());
  }

  let (removed, added) = word_diff_ranges_impl(old_text, new_text, false);
  if removed.is_empty() && added.is_empty() && old_text != new_text {
    return word_diff_ranges_impl(old_text, new_text, true);
  }
  (removed, added)
}

fn word_diff_ranges_impl(
  old_text: &str,
  new_text: &str,
  include_whitespace: bool,
) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
  let old_tokens = word_tokens(old_text, include_whitespace);
  let new_tokens = word_tokens(new_text, include_whitespace);

  let old_len = old_tokens.len();
  let new_len = new_tokens.len();
  if old_len == 0 && new_len == 0 {
    return (Vec::new(), Vec::new());
  }

  let mut dp = vec![vec![0usize; new_len + 1]; old_len + 1];
  for i in 0..old_len {
    for j in 0..new_len {
      if old_tokens[i].text == new_tokens[j].text {
        dp[i + 1][j + 1] = dp[i][j] + 1;
      } else {
        dp[i + 1][j + 1] = dp[i][j + 1].max(dp[i + 1][j]);
      }
    }
  }

  let mut matched_old = vec![false; old_len];
  let mut matched_new = vec![false; new_len];
  let mut i = old_len;
  let mut j = new_len;
  while i > 0 && j > 0 {
    if old_tokens[i - 1].text == new_tokens[j - 1].text {
      matched_old[i - 1] = true;
      matched_new[j - 1] = true;
      i -= 1;
      j -= 1;
    } else if dp[i - 1][j] >= dp[i][j - 1] {
      i -= 1;
    } else {
      j -= 1;
    }
  }

  let removed = old_tokens
    .iter()
    .enumerate()
    .filter_map(|(idx, token)| (!matched_old[idx]).then_some(token.range.clone()))
    .collect();
  let added = new_tokens
    .iter()
    .enumerate()
    .filter_map(|(idx, token)| (!matched_new[idx]).then_some(token.range.clone()))
    .collect();

  (merge_ranges(removed), merge_ranges(added))
}

fn apply_inline_background_ranges(
  spans: Vec<InlineSpan>,
  ranges: &[Range<usize>],
  background: InlineBackground,
) -> Vec<InlineSpan> {
  if ranges.is_empty() {
    return spans;
  }

  let ranges = merge_ranges(ranges.to_vec());
  let mut result = Vec::new();
  let mut range_index = 0usize;
  let mut current_range = ranges.get(range_index);

  for span in spans {
    let span_start = span.range.start;
    let span_end = span.range.end;
    let mut cursor = span_start;

    while let Some(range) = current_range {
      if range.end <= span_start {
        range_index += 1;
        current_range = ranges.get(range_index);
        continue;
      }
      if range.start >= span_end {
        break;
      }

      let overlap_start = range.start.max(span_start);
      let overlap_end = range.end.min(span_end);

      if overlap_start > cursor {
        let mut prefix = span.clone();
        prefix.range = cursor..overlap_start;
        result.push(prefix);
      }

      if overlap_end > overlap_start {
        let mut highlighted = span.clone();
        highlighted.range = overlap_start..overlap_end;
        highlighted.background = Some(background);
        result.push(highlighted);
      }

      cursor = overlap_end;
      if range.end <= span_end {
        range_index += 1;
        current_range = ranges.get(range_index);
      } else {
        break;
      }
    }

    if cursor < span_end {
      let mut suffix = span.clone();
      suffix.range = cursor..span_end;
      result.push(suffix);
    }
  }

  result
}

fn github_diff_word_highlights(lines: &[GithubDiffLine]) -> Vec<Option<DiffWordHighlight>> {
  let mut highlights = vec![None; lines.len()];
  let mut ix = 0usize;

  while ix < lines.len() {
    if lines[ix].kind == GithubDiffLineKind::Context {
      ix += 1;
      continue;
    }

    let block_start = ix;
    while ix < lines.len() && lines[ix].kind != GithubDiffLineKind::Context {
      ix += 1;
    }
    let block_end = ix;

    let removed_indices: Vec<_> = (block_start..block_end)
      .filter(|idx| lines[*idx].kind == GithubDiffLineKind::Removed)
      .collect();
    let added_indices: Vec<_> = (block_start..block_end)
      .filter(|idx| lines[*idx].kind == GithubDiffLineKind::Added)
      .collect();
    let pair_count = removed_indices.len().min(added_indices.len());

    for pair_ix in 0..pair_count {
      let removed_idx = removed_indices[pair_ix];
      let added_idx = added_indices[pair_ix];
      let (removed_ranges, added_ranges) = word_diff_ranges(
        lines[removed_idx].content.as_ref(),
        lines[added_idx].content.as_ref(),
      );

      if !removed_ranges.is_empty() {
        highlights[removed_idx] = Some(DiffWordHighlight {
          ranges: removed_ranges,
          background: InlineBackground::DiffWordRemoved,
        });
      }
      if !added_ranges.is_empty() {
        highlights[added_idx] = Some(DiffWordHighlight {
          ranges: added_ranges,
          background: InlineBackground::DiffWordAdded,
        });
      }
    }
  }

  highlights
}

fn render_github_diff_lines(
  lines: &[GithubDiffLine],
  path: &str,
  text_seed: usize,
  state: MarkdownRenderState,
  min_content_width_px: f32,
  cx: &App,
) -> Div {
  let theme = cx.theme();
  let language_hint = code_block_language_hint_from_path(path);
  let snippets: Vec<Arc<str>> = lines.iter().map(|line| line.content.clone()).collect();
  let per_line_spans =
    build_preview_code_spans_per_line(&snippets, language_hint.as_deref(), None, 1);
  let word_highlights = github_diff_word_highlights(lines);
  let line_number_width = lines
    .iter()
    .filter_map(|line| line.old_line.or(line.new_line))
    .max()
    .map(|line| ((line.to_string().len() as f32) * 8.0).max(28.0))
    .unwrap_or(28.0);
  let gutter_width = line_number_width + 16.0;

  let mut rows = v_flex().w_full().min_w(px(min_content_width_px));

  for (ix, line) in lines.iter().enumerate() {
    let (line_text, mut line_spans) = per_line_spans.get(ix).cloned().unwrap_or_else(|| {
      (
        SharedString::from(line.content.as_ref().to_string()),
        Vec::new(),
      )
    });
    if let Some(highlight) = word_highlights.get(ix).and_then(Option::as_ref) {
      line_spans =
        apply_inline_background_ranges(line_spans, &highlight.ranges, highlight.background);
    }
    let text_id = compose_text_id(text_seed, ix + 1);

    rows = rows.child(
      h_flex()
        .items_center()
        .bg(github_diff_line_background(line.kind, cx))
        .min_w(px(min_content_width_px))
        .child(
          div()
            .w(px(gutter_width))
            .px_2()
            .text_right()
            .text_xs()
            .font_family(theme.mono_font_family.clone())
            .text_color(theme.muted_foreground)
            .child(
              line
                .old_line
                .map(|line| line.to_string())
                .unwrap_or_default(),
            ),
        )
        .child(
          div()
            .w(px(gutter_width))
            .px_2()
            .text_right()
            .text_xs()
            .font_family(theme.mono_font_family.clone())
            .text_color(theme.muted_foreground)
            .child(
              line
                .new_line
                .map(|line| line.to_string())
                .unwrap_or_default(),
            ),
        )
        .child(
          div()
            .flex_1()
            .min_w_0()
            .font_family(theme.mono_font_family.clone())
            .text_sm()
            .whitespace_nowrap()
            .text_color(theme.foreground)
            .child(SelectableText::new(
              line_text,
              line_spans,
              Vec::new(),
              state.clone(),
              None,
              text_id,
              SelectableTextOptions {
                interactive: false,
                show_indentation_dots: true,
                show_inline_code_backgrounds: false,
              },
            )),
        ),
    );
  }

  rows
}

pub fn render_github_diff_code_reference_preview_card(
  preview: &GithubCodeReferencePreview,
  diff_lines: &[GithubDiffLine],
  cx: &App,
) -> Div {
  let theme = cx.theme();
  let link_color = github_link_color(theme.background);
  let mut preview_id_hasher = DefaultHasher::new();
  preview.url.hash(&mut preview_id_hasher);
  preview.start_line.hash(&mut preview_id_hasher);
  preview.end_line.hash(&mut preview_id_hasher);
  diff_lines.len().hash(&mut preview_id_hasher);
  let preview_hash = preview_id_hasher.finish();
  let preview_scroll_id: SharedString = format!(
    "markdown-diff-code-reference-preview-scroll-{}",
    preview_hash
  )
  .into();
  let snippet_text_seed = preview_hash as usize;
  let min_preview_content_width_px =
    (estimate_code_reference_preview_min_content_width_px(preview) + 110.0).max(
      diff_lines
        .iter()
        .map(|line| estimate_code_block_min_content_width_px(line.content.as_ref()))
        .fold(0.0, f32::max)
        + 110.0,
    );
  let url = preview.url.clone();
  let file_label = format!("{}/{}", preview.repo.as_ref(), preview.path.as_ref());
  let line_label = if preview.start_line == preview.end_line {
    format!(
      "Line {} in {}",
      preview.start_line,
      short_github_reference(preview.reference.as_ref())
    )
  } else {
    format!(
      "Lines {}-{} in {}",
      preview.start_line,
      preview.end_line,
      short_github_reference(preview.reference.as_ref())
    )
  };

  let preview_scroll_key = preview_card_scroll_key(preview_hash);
  let preview_scroll_handle = scrollable_handle(preview_scroll_key);
  let preview_content = restrict_scroll_to_wheel_axis(
    div()
      .id(preview_scroll_id)
      .w_full()
      .min_w_0()
      .max_h(px(MARKDOWN_CODE_BLOCK_MAX_HEIGHT_PX))
      .overflow_scroll(),
  )
  .track_scroll(&preview_scroll_handle)
  .child(render_github_diff_lines(
    diff_lines,
    preview.path.as_ref(),
    snippet_text_seed,
    MarkdownRenderState::new(),
    min_preview_content_width_px,
    cx,
  ));
  let preview_scrolled = scrollable_node(
    preview_content,
    &preview_scroll_handle,
    ScrollAxes::both(),
    preview_scroll_key,
  )
  .into_any_element();

  div()
    .flex()
    .flex_col()
    .relative()
    .my(px(MARKDOWN_CODE_REFERENCE_CARD_MARGIN_Y_PX))
    .border_1()
    .border_color(theme.border)
    .rounded_md()
    .overflow_hidden()
    .child(
      div()
        .bg(theme.sidebar)
        .border_b_1()
        .border_color(theme.border)
        .px(px(MARKDOWN_CODE_REFERENCE_CARD_PADDING_X_PX))
        .py(px(MARKDOWN_CODE_REFERENCE_CARD_PADDING_Y_PX))
        .cursor(CursorStyle::PointingHand)
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
          cx.stop_propagation();
          cx.open_url(url.as_ref());
        })
        .child(
          div()
            .flex()
            .flex_col()
            .child(
              div()
                .text_sm()
                .font_medium()
                .text_color(link_color)
                .child(file_label),
            )
            .child(
              div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(line_label),
            ),
        ),
    )
    .child(preview_scrolled)
    .horizontal_scrollbar(&preview_scroll_handle)
    .vertical_scrollbar(&preview_scroll_handle)
}

pub fn render_github_code_reference_preview_card(
  preview: &GithubCodeReferencePreview,
  cx: &App,
) -> Div {
  let theme = cx.theme();
  let link_color = github_link_color(theme.background);
  let mut preview_id_hasher = DefaultHasher::new();
  preview.url.hash(&mut preview_id_hasher);
  preview.start_line.hash(&mut preview_id_hasher);
  preview.end_line.hash(&mut preview_id_hasher);
  let preview_hash = preview_id_hasher.finish();
  let preview_scroll_id: SharedString =
    format!("markdown-code-reference-preview-scroll-{}", preview_hash).into();
  let snippet_text_seed = preview_hash as usize;
  let snippet_language_hint = code_block_language_hint_from_path(preview.path.as_ref());
  let snippet_render_state = MarkdownRenderState::new();
  let min_preview_content_width_px = estimate_code_reference_preview_min_content_width_px(preview);
  let url = preview.url.clone();
  let file_label = format!("{}/{}", preview.repo.as_ref(), preview.path.as_ref());
  let line_label = if preview.start_line == preview.end_line {
    format!(
      "Line {} in {}",
      preview.start_line,
      short_github_reference(preview.reference.as_ref())
    )
  } else {
    format!(
      "Lines {}-{} in {}",
      preview.start_line,
      preview.end_line,
      short_github_reference(preview.reference.as_ref())
    )
  };

  let mut snippet_rows = div()
    .flex()
    .flex_col()
    .gap(px(MARKDOWN_CODE_REFERENCE_SNIPPET_ROW_GAP_PX));
  if preview.snippets.is_empty() {
    snippet_rows = snippet_rows.child(
      h_flex()
        .items_center()
        .gap_2()
        .child(
          div()
            .text_xs()
            .font_medium()
            .text_color(theme.muted_foreground)
            .child(preview.start_line.to_string()),
        )
        .child(
          div()
            .font_family(cx.theme().mono_font_family.clone())
            .text_sm()
            .whitespace_nowrap()
            .text_color(theme.foreground)
            .child(""),
        ),
    );
  } else {
    let per_line_spans = build_preview_code_spans_per_line(
      &preview.snippets,
      snippet_language_hint.as_deref(),
      preview.full_content.as_deref(),
      preview.start_line,
    );

    for (offset, (line_text, line_spans)) in per_line_spans.into_iter().enumerate() {
      let line_number = preview.start_line + offset;
      let text_id = compose_text_id(snippet_text_seed, line_number);
      snippet_rows = snippet_rows.child(
        h_flex()
          .items_center()
          .gap_2()
          .child(
            div()
              .text_xs()
              .font_medium()
              .text_color(theme.muted_foreground)
              .child(line_number.to_string()),
          )
          .child(
            div()
              .font_family(cx.theme().mono_font_family.clone())
              .text_sm()
              .whitespace_nowrap()
              .text_color(theme.foreground)
              .child(SelectableText::new(
                line_text,
                line_spans,
                Vec::new(),
                snippet_render_state.clone(),
                None,
                text_id,
                SelectableTextOptions {
                  interactive: false,
                  show_indentation_dots: true,
                  show_inline_code_backgrounds: false,
                },
              )),
          ),
      );
    }
  }

  let preview_scroll_key = preview_card_scroll_key(preview_hash);
  let preview_scroll_handle = scrollable_handle(preview_scroll_key);
  let preview_content = restrict_scroll_to_wheel_axis(
    div()
      .id(preview_scroll_id)
      .w_full()
      .px(px(MARKDOWN_CODE_REFERENCE_CARD_PADDING_X_PX))
      .py(px(MARKDOWN_CODE_REFERENCE_CARD_PADDING_Y_PX))
      .min_w_0()
      .max_h(px(MARKDOWN_CODE_BLOCK_MAX_HEIGHT_PX))
      .overflow_scroll(),
  )
  .track_scroll(&preview_scroll_handle)
  .child(
    div()
      .min_w(px(min_preview_content_width_px))
      .whitespace_nowrap()
      .text_sm()
      .text_color(theme.foreground)
      .child(
        div()
          .flex()
          .flex_col()
          .gap(px(MARKDOWN_CODE_REFERENCE_CARD_INTERNAL_GAP_PX))
          .child(snippet_rows),
      ),
  );
  let preview_scrolled = scrollable_node(
    preview_content,
    &preview_scroll_handle,
    ScrollAxes::both(),
    preview_scroll_key,
  )
  .into_any_element();

  div()
    .flex()
    .flex_col()
    .relative()
    .my(px(MARKDOWN_CODE_REFERENCE_CARD_MARGIN_Y_PX))
    .border_1()
    .border_color(theme.border)
    .rounded_md()
    .overflow_hidden()
    .child(
      div()
        .bg(theme.accent.opacity(0.3))
        .border_b_1()
        .border_color(theme.border)
        .px(px(MARKDOWN_CODE_REFERENCE_CARD_PADDING_X_PX))
        .py(px(MARKDOWN_CODE_REFERENCE_CARD_PADDING_Y_PX))
        .cursor(CursorStyle::PointingHand)
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
          cx.stop_propagation();
          cx.open_url(url.as_ref());
        })
        .child(
          div()
            .flex()
            .flex_col()
            .child(
              div()
                .text_sm()
                .font_medium()
                .text_color(link_color)
                .child(file_label),
            )
            .child(
              div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(line_label),
            ),
        ),
    )
    .child(preview_scrolled)
    .horizontal_scrollbar(&preview_scroll_handle)
    .vertical_scrollbar(&preview_scroll_handle)
}

fn code_block_language_hint_from_path(path: &str) -> Option<String> {
  let file_name = path.rsplit('/').next().unwrap_or(path).trim();
  if file_name.is_empty() {
    return None;
  }

  if let Some(extension) = Path::new(file_name)
    .extension()
    .and_then(|ext| ext.to_str())
    .filter(|ext| !ext.is_empty())
  {
    return Some(extension.to_string());
  }

  Some(file_name.to_string())
}

/// Max file size for full-file highlighting. Above this we fall back to
/// snippet-only highlighting to avoid blocking the UI thread.
const FULL_FILE_HIGHLIGHT_MAX_BYTES: usize = 150_000;

/// Cache of highlight spans keyed by (language, content_hash).
/// Avoids re-highlighting the same file for multiple code preview cards.
static PREVIEW_HIGHLIGHT_CACHE: Mutex<Option<PreviewHighlightCache>> = Mutex::new(None);

const PREVIEW_HIGHLIGHT_CACHE_MAX_ENTRIES: usize = 16;

struct PreviewHighlightCache {
  entries: Vec<(u64, Arc<Vec<InlineSpan>>)>,
}

impl PreviewHighlightCache {
  fn get(&self, key: u64) -> Option<&Arc<Vec<InlineSpan>>> {
    self.entries.iter().find(|(k, _)| *k == key).map(|(_, v)| v)
  }

  fn insert(&mut self, key: u64, spans: Arc<Vec<InlineSpan>>) {
    if self.entries.len() >= PREVIEW_HIGHLIGHT_CACHE_MAX_ENTRIES {
      self.entries.remove(0);
    }
    self.entries.push((key, spans));
  }
}

fn preview_highlight_cache_key(language_hint: Option<&str>, content: &str) -> u64 {
  let mut hasher = DefaultHasher::new();
  language_hint.hash(&mut hasher);
  content.len().hash(&mut hasher);
  // Hash first + last chunks for speed on large files.
  let prefix = &content[..content.len().min(4096)];
  prefix.hash(&mut hasher);
  if content.len() > 4096 {
    let suffix = &content[content.len().saturating_sub(4096)..];
    suffix.hash(&mut hasher);
  }
  hasher.finish()
}

fn highlight_full_text(text: &str, language_hint: Option<&str>) -> Arc<Vec<InlineSpan>> {
  let base_style = InlineStyle {
    code: true,
    ..InlineStyle::default()
  };

  let cache_key = preview_highlight_cache_key(language_hint, text);

  // Check cache.
  if let Ok(guard) = PREVIEW_HIGHLIGHT_CACHE.lock()
    && let Some(cache) = guard.as_ref()
    && let Some(cached) = cache.get(cache_key)
  {
    return cached.clone();
  }

  let spans = code_block_language_config(language_hint)
    .and_then(|config| {
      let mut highlighter = SyntaxHighlighter::new(config);
      highlighter
        .highlight_text(text)
        .ok()
        .map(|highlights| syntax_highlight_spans_for_code(text, &highlights, base_style))
    })
    .unwrap_or_default();

  let spans = Arc::new(spans);

  if let Ok(mut guard) = PREVIEW_HIGHLIGHT_CACHE.lock() {
    let cache = guard.get_or_insert_with(|| PreviewHighlightCache {
      entries: Vec::new(),
    });
    cache.insert(cache_key, spans.clone());
  }

  spans
}

/// Highlight code with full file context when available, then extract spans for
/// only the visible lines. Results are cached per file so multiple preview cards
/// referencing the same file only trigger one tree-sitter pass.
fn build_preview_code_spans_per_line(
  snippets: &[Arc<str>],
  language_hint: Option<&str>,
  full_content: Option<&str>,
  start_line: usize,
) -> Vec<(SharedString, Vec<InlineSpan>)> {
  let base_style = InlineStyle {
    code: true,
    ..InlineStyle::default()
  };

  // Use full file content if available and within size cap.
  let usable_full_content =
    full_content.filter(|content| content.len() <= FULL_FILE_HIGHLIGHT_MAX_BYTES);

  if let Some(content) = usable_full_content {
    let first_line_ix = start_line.saturating_sub(1);
    let target_line_indices: Vec<usize> = (first_line_ix..first_line_ix + snippets.len()).collect();
    let all_spans = highlight_full_text(content, language_hint);
    return split_spans_per_line(snippets, content, &all_spans, target_line_indices);
  }

  // Fallback: join snippets and highlight as one block.
  let joined: String = snippets
    .iter()
    .map(|s| s.as_ref())
    .collect::<Vec<_>>()
    .join("\n");
  let all_spans = code_block_language_config(language_hint)
    .and_then(|config| {
      let mut highlighter = SyntaxHighlighter::new(config);
      highlighter
        .highlight_text(&joined)
        .ok()
        .map(|highlights| syntax_highlight_spans_for_code(&joined, &highlights, base_style))
    })
    .unwrap_or_default();
  split_spans_per_line(snippets, &joined, &all_spans, (0..snippets.len()).collect())
}

fn split_spans_per_line(
  snippets: &[Arc<str>],
  full_text: &str,
  all_spans: &[InlineSpan],
  target_line_indices: Vec<usize>,
) -> Vec<(SharedString, Vec<InlineSpan>)> {
  let base_style = InlineStyle {
    code: true,
    ..InlineStyle::default()
  };

  let mut all_line_byte_ranges: Vec<Range<usize>> = Vec::new();
  let mut cursor = 0usize;
  for line in full_text.split('\n') {
    let end = cursor + line.len();
    all_line_byte_ranges.push(cursor..end);
    cursor = end + 1;
  }

  target_line_indices
    .iter()
    .enumerate()
    .map(|(snippet_ix, &line_ix)| {
      let line_text = SharedString::from(snippets[snippet_ix].as_ref().to_string());
      let text_len = line_text.len();
      let Some(line_range) = all_line_byte_ranges.get(line_ix) else {
        return (
          line_text,
          vec![InlineSpan {
            range: 0..text_len,
            style: base_style,
            link: None,
            syntax_token: None,
            background: None,
          }],
        );
      };
      let line_start = line_range.start;
      let line_end = line_range.end;

      let mut line_spans: Vec<InlineSpan> = all_spans
        .iter()
        .filter_map(|span| {
          let span_start = span.range.start.max(line_start);
          let span_end = span.range.end.min(line_end);
          if span_start >= span_end {
            return None;
          }
          Some(InlineSpan {
            range: (span_start - line_start)..(span_end - line_start),
            style: span.style,
            link: span.link.clone(),
            syntax_token: span.syntax_token,
            background: span.background,
          })
        })
        .collect();

      // Fill gaps so spans cover the entire line.
      if line_spans.is_empty() && text_len > 0 {
        line_spans.push(InlineSpan {
          range: 0..text_len,
          style: base_style,
          link: None,
          syntax_token: None,
          background: None,
        });
      } else {
        let mut filled = Vec::with_capacity(line_spans.len() * 2);
        let mut pos = 0usize;
        for span in &line_spans {
          if span.range.start > pos {
            filled.push(InlineSpan {
              range: pos..span.range.start,
              style: base_style,
              link: None,
              syntax_token: None,
              background: None,
            });
          }
          filled.push(span.clone());
          pos = span.range.end;
        }
        if pos < text_len {
          filled.push(InlineSpan {
            range: pos..text_len,
            style: base_style,
            link: None,
            syntax_token: None,
            background: None,
          });
        }
        line_spans = filled;
      }

      (line_text, line_spans)
    })
    .collect()
}

fn render_markdown_with_preview_segments(
  source: &str,
  options: &MarkdownRenderOptions,
  previews: &HashMap<Arc<str>, GithubCodeReferencePreview>,
  cx: &App,
) -> AnyElement {
  let segments = split_markdown_preview_segments(source, previews);
  let has_previews = segments
    .iter()
    .any(|segment| matches!(segment, MarkdownRenderSegment::Preview(_)));
  if !has_previews {
    let parsed = parse_markdown_for_render(source);
    return render_parsed_markdown(&parsed, options, cx);
  }

  let base_scope_id = options.scope_id.map_or_else(
    || markdown_source_scope_id(source, &options.state),
    |scope_id| scoped_id_for_state(scope_id, &options.state),
  );

  let mut rendered = div().flex().flex_col();
  for (segment_index, segment) in segments.into_iter().enumerate() {
    match segment {
      MarkdownRenderSegment::Markdown(markdown) => {
        if markdown.is_empty() {
          continue;
        }
        let parsed = parse_markdown_for_render(markdown.as_str());
        let scoped_options = options
          .clone()
          .with_scope_id(compose_text_id(base_scope_id, segment_index + 1));
        rendered = rendered.child(render_parsed_markdown(&parsed, &scoped_options, cx));
      }
      MarkdownRenderSegment::Preview(preview) => {
        rendered = rendered.child(render_github_code_reference_preview_card(&preview, cx));
      }
    }
  }

  rendered.into_any_element()
}

pub fn render_markdown(source: &str, options: &MarkdownRenderOptions, cx: &App) -> AnyElement {
  if let Some(previews) = options.github_code_reference_previews.as_ref()
    && !previews.is_empty()
  {
    return render_markdown_with_preview_segments(source, options, previews.as_ref(), cx);
  }

  let parsed = parse_markdown_for_render(source);
  render_parsed_markdown(&parsed, options, cx)
}

pub(crate) struct RenderContext {
  text_scope_id: usize,
  next_text_id: usize,
  next_details_id: usize,
}

impl RenderContext {
  fn new(text_scope_id: usize) -> Self {
    Self {
      text_scope_id,
      next_text_id: 0,
      next_details_id: 0,
    }
  }

  fn next_text_id(&mut self) -> usize {
    let local_id = self.next_text_id;
    self.next_text_id += 1;
    compose_text_id(self.text_scope_id, local_id)
  }

  fn next_details_id(&mut self) -> usize {
    let local_id = self.next_details_id;
    self.next_details_id += 1;
    compose_text_id(self.text_scope_id, local_id)
  }
}

fn compose_text_id(scope_id: usize, local_id: usize) -> usize {
  scope_id.wrapping_mul(1_000_003usize).wrapping_add(local_id)
}

fn resolve_scope_id_for_parsed(parsed: &ParsedMarkdown, options: &MarkdownRenderOptions) -> usize {
  options.scope_id.map_or_else(
    || parsed_markdown_scope_id(parsed, &options.state),
    |scope_id| scoped_id_for_state(scope_id, &options.state),
  )
}

fn parsed_markdown_scope_id(parsed: &ParsedMarkdown, state: &MarkdownRenderState) -> usize {
  let parsed_seed = Arc::as_ptr(&parsed.blocks) as usize;
  scoped_id_for_state(parsed_seed, state)
}

fn scoped_id_for_state(scope_seed: usize, state: &MarkdownRenderState) -> usize {
  scope_seed ^ state.instance_id.wrapping_mul(0x9E37_79B1usize)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GithubAlertKind {
  Note,
  Tip,
  Important,
  Warning,
  Caution,
}

/// Detects GitHub-style alert syntax in a blockquote: `> [!NOTE]`, `> [!TIP]`, etc.
/// Returns the alert kind and the remaining text after the marker.
fn detect_github_alert(children: &[Block]) -> Option<(GithubAlertKind, String)> {
  let Block::Paragraph(inlines) = children.first()? else {
    return None;
  };
  let Inline::Text(text) = inlines.first()? else {
    return None;
  };
  let trimmed = text.trim_start();
  let (kind, marker_len) = if trimmed.starts_with("[!NOTE]") {
    (GithubAlertKind::Note, "[!NOTE]".len())
  } else if trimmed.starts_with("[!TIP]") {
    (GithubAlertKind::Tip, "[!TIP]".len())
  } else if trimmed.starts_with("[!IMPORTANT]") {
    (GithubAlertKind::Important, "[!IMPORTANT]".len())
  } else if trimmed.starts_with("[!WARNING]") {
    (GithubAlertKind::Warning, "[!WARNING]".len())
  } else if trimmed.starts_with("[!CAUTION]") {
    (GithubAlertKind::Caution, "[!CAUTION]".len())
  } else {
    return None;
  };
  let prefix_offset = text.len() - trimmed.len();
  let remaining = text[prefix_offset + marker_len..].to_string();
  Some((kind, remaining))
}

fn render_blocks(
  blocks: &[Block],
  options: &MarkdownRenderOptions,
  indent: usize,
  cx: &App,
  ctx: &mut RenderContext,
) -> AnyElement {
  let mut container = div().flex().flex_col().gap_2();

  for (ix, block) in blocks.iter().enumerate() {
    let block_element = render_block(block, options, indent, cx, ctx);
    let block_element = if ix > 0 && matches!(block, Block::Heading { .. }) {
      div()
        .mt(px(MARKDOWN_HEADING_EXTRA_TOP_MARGIN_PX))
        .child(block_element)
        .into_any_element()
    } else {
      block_element
    };

    container = container.child(block_element);
  }

  if indent > 0 {
    container = container.pl(px(12.0 * indent as f32));
  }

  container.into_any_element()
}

fn render_block(
  block: &Block,
  options: &MarkdownRenderOptions,
  indent: usize,
  cx: &App,
  ctx: &mut RenderContext,
) -> AnyElement {
  match block {
    Block::Paragraph(inlines) => {
      let theme = cx.theme();
      let content = div()
        .whitespace_normal()
        .overflow_hidden()
        .text_sm()
        .text_color(theme.foreground)
        .child(render_inline_text(inlines, options, cx, ctx))
        .into_any_element();
      if let Some(override_fn) = options.overrides.paragraph.as_ref() {
        override_fn(content, cx)
      } else {
        content
      }
    }
    Block::Heading { level, content } => {
      let content = render_heading_text(*level, content, options, cx, ctx);
      if let Some(override_fn) = options.overrides.heading.as_ref() {
        override_fn(*level, content, cx)
      } else {
        content
      }
    }
    Block::List(list) => render_list(list, options, indent, cx, ctx),
    Block::CodeBlock(code) => {
      if let Some(override_fn) = options.overrides.code_block.as_ref() {
        return override_fn(code, cx);
      }
      if code.lang.as_deref() == Some("suggestion") {
        if let Some(suggestion_ctx) = options.suggestion_context.as_ref() {
          return render_suggestion_block(code, suggestion_ctx, options, cx, ctx);
        }
        let fallback_ctx = SuggestionContext {
          original_start_line: None,
          suggested_start_line: None,
          original_lines: Vec::new(),
          path: Arc::from(""),
        };
        return render_suggestion_block(code, &fallback_ctx, options, cx, ctx);
      }
      render_code_block(code, options, cx, ctx)
    }
    Block::BlockQuote(children) => {
      let alert = detect_github_alert(children);
      let (border_color, alert_header) = if let Some((kind, _remaining)) = &alert {
        let _theme = cx.theme();
        let (color, icon, label) = match kind {
          GithubAlertKind::Note => (gpui::hsla(0.58, 0.8, 0.55, 1.0), IconName::Info, "Note"),
          GithubAlertKind::Tip => (
            gpui::hsla(0.38, 0.7, 0.45, 1.0),
            IconName::CircleCheck,
            "Tip",
          ),
          GithubAlertKind::Important => (
            gpui::hsla(0.75, 0.7, 0.55, 1.0),
            IconName::Info,
            "Important",
          ),
          GithubAlertKind::Warning => (
            gpui::hsla(0.12, 0.8, 0.50, 1.0),
            IconName::TriangleAlert,
            "Warning",
          ),
          GithubAlertKind::Caution => (
            gpui::hsla(0.0, 0.75, 0.55, 1.0),
            IconName::CircleX,
            "Caution",
          ),
        };
        (
          color,
          Some(
            h_flex()
              .items_center()
              .gap_1p5()
              .pb_1()
              .child(Icon::new(icon).size_4().text_color(color))
              .child(
                div()
                  .text_sm()
                  .font_semibold()
                  .text_color(color)
                  .child(label),
              ),
          ),
        )
      } else {
        (cx.theme().muted_foreground, None)
      };

      // Build children, stripping the alert marker from the first paragraph
      let rendered_children = if let Some((_, remaining_first_text)) = &alert {
        let mut modified_children = children.clone();
        if let Some(Block::Paragraph(inlines)) = modified_children.first_mut()
          && let Some(Inline::Text(text)) = inlines.first_mut()
        {
          *text = remaining_first_text.clone();
          if text.is_empty() {
            inlines.remove(0);
            // Also remove leading SoftBreak if present
            if matches!(inlines.first(), Some(Inline::SoftBreak)) {
              inlines.remove(0);
            }
          }
        }
        render_blocks(&modified_children, options, indent + 1, cx, ctx)
      } else {
        render_blocks(children, options, indent + 1, cx, ctx)
      };

      let content = div()
        .border_l_2()
        .border_color(border_color)
        .pl(px(8.0))
        .when_some(alert_header, |this, header| this.child(header))
        .child(rendered_children)
        .into_any_element();
      if let Some(override_fn) = options.overrides.block_quote.as_ref() {
        override_fn(content, cx)
      } else {
        content
      }
    }
    Block::ThematicBreak => {
      if let Some(override_fn) = options.overrides.thematic_break.as_ref() {
        return override_fn(cx);
      }
      div()
        .h(px(1.0))
        .bg(cx.theme().border)
        .rounded_md()
        .into_any_element()
    }
    Block::Table(table) => render_table(table, options, cx, ctx),
    Block::Details(details) => render_details(details, options, indent, cx, ctx),
    Block::Aligned { center, blocks } => {
      if *center {
        let mut aligned = div().flex().flex_col().w_full().min_w_0().gap_2();
        for block in blocks {
          aligned = aligned.child(
            h_flex().w_full().min_w_0().justify_center().child(
              div()
                .text_center()
                .min_w_0()
                .child(render_block(block, options, indent, cx, ctx)),
            ),
          );
        }
        aligned.into_any_element()
      } else {
        render_blocks(blocks, options, indent, cx, ctx)
      }
    }
  }
}

fn render_list(
  list: &List,
  options: &MarkdownRenderOptions,
  _indent: usize,
  cx: &App,
  ctx: &mut RenderContext,
) -> AnyElement {
  let theme = cx.theme();
  let mut container = div()
    .flex()
    .flex_col()
    .w_full()
    .min_w_0()
    .gap_1()
    .pl(px(LIST_LEFT_PADDING_PX));
  let start = list.start.unwrap_or(1);

  for (ix, item) in list.items.iter().enumerate() {
    let bullet = if list.ordered {
      format!("{}.", start + ix as u64)
    } else if item.checked == Some(true) {
      "[x]".to_string()
    } else if item.checked == Some(false) {
      "[ ]".to_string()
    } else {
      "•".to_string()
    };

    let content = render_list_item_blocks(&item.blocks, options, cx, ctx);
    let row = ListItemView {
      bullet: bullet.clone(),
      checked: item.checked,
      content,
    };

    let element = if let Some(override_fn) = options.overrides.list_item.as_ref() {
      override_fn(row, cx)
    } else {
      h_flex()
        .items_start()
        .w_full()
        .min_w_0()
        .child(
          div()
            .flex_none()
            .text_sm()
            .text_color(theme.foreground)
            .pr(px(LIST_MARKER_GAP_PX))
            .child(bullet),
        )
        .child(div().min_w_0().flex_1().child(row.content))
        .into_any_element()
    };
    container = container.child(element);
  }

  let container = container.into_any_element();
  if let Some(override_fn) = options.overrides.list.as_ref() {
    override_fn(container, cx)
  } else {
    container
  }
}

fn render_list_item_blocks(
  blocks: &[Block],
  options: &MarkdownRenderOptions,
  cx: &App,
  ctx: &mut RenderContext,
) -> AnyElement {
  let mut container = div().flex().flex_col().w_full().min_w_0().gap_2();
  for block in blocks {
    container = container.child(render_block(block, options, 0, cx, ctx));
  }
  container.into_any_element()
}

fn render_table(
  table: &Table,
  options: &MarkdownRenderOptions,
  cx: &App,
  ctx: &mut RenderContext,
) -> AnyElement {
  if let Some(override_fn) = options.overrides.table.as_ref() {
    return override_fn(table, cx);
  }

  let theme = cx.theme();
  let column_count = table
    .rows
    .iter()
    .fold(table.headers.len(), |count, row| count.max(row.len()))
    .max(1);
  let column_widths = table_column_widths(table, column_count);
  let total_width: f32 = column_widths.iter().sum();

  let mut header_row = div()
    .flex()
    .flex_row()
    .items_stretch()
    .bg(theme.accent.opacity(0.3));
  for (column, width) in column_widths.iter().enumerate().take(column_count) {
    let cell = table
      .headers
      .get(column)
      .map_or(&[][..], |cell| cell.as_slice());
    let basis = if total_width > 0.0 {
      *width / total_width
    } else {
      1.0 / column_count as f32
    };
    header_row = header_row.child(
      div()
        .flex_basis(gpui::relative(basis))
        .flex_grow(1.)
        .min_w_0()
        .h_full()
        .px_3()
        .py_2()
        .when(column + 1 < column_count, |this| {
          this.border_r_1().border_color(theme.border)
        })
        .child(
          div()
            .text_sm()
            .font_medium()
            .text_color(theme.foreground)
            .child(render_table_cell_inlines(cell, options, cx, ctx)),
        ),
    );
  }

  let mut body = div().flex().flex_col();
  for (row_index, row) in table.rows.iter().enumerate() {
    let mut row_el = div()
      .flex()
      .flex_row()
      .items_stretch()
      .border_t_1()
      .border_color(theme.border)
      .when(row_index % 2 == 1, |this| {
        this.bg(theme.accent.opacity(0.3))
      });
    for (column, width) in column_widths.iter().enumerate().take(column_count) {
      let cell = row.get(column).map_or(&[][..], |cell| cell.as_slice());
      let basis = if total_width > 0.0 {
        *width / total_width
      } else {
        1.0 / column_count as f32
      };
      row_el = row_el.child(
        div()
          .flex_basis(gpui::relative(basis))
          .flex_grow(1.)
          .min_w_0()
          .px_3()
          .py_2()
          .when(column + 1 < column_count, |this| {
            this.border_r_1().border_color(theme.border)
          })
          .child(
            div()
              .text_sm()
              .text_color(theme.foreground)
              .child(render_table_cell_inlines(cell, options, cx, ctx)),
          ),
      );
    }
    body = body.child(row_el);
  }

  let table_scroll_id: SharedString =
    format!("markdown-table-scroll-{:x}", table as *const Table as usize).into();

  div()
    .id(table_scroll_id)
    .w_full()
    .min_w_0()
    .child(
      div()
        .border_1()
        .border_color(theme.border)
        .rounded_md()
        .overflow_hidden()
        .child(div().flex().flex_col().child(header_row).child(body)),
    )
    .overflow_x_scrollbar()
    .into_any_element()
}

fn table_column_widths(table: &Table, column_count: usize) -> Vec<f32> {
  let mut widths = vec![TABLE_CELL_MIN_WIDTH_PX; column_count];

  for (column, width) in widths.iter_mut().enumerate().take(column_count) {
    if let Some(cell) = table.headers.get(column) {
      *width = (*width).max(table_cell_width(cell));
    }

    for row in &table.rows {
      if let Some(cell) = row.get(column) {
        *width = (*width).max(table_cell_width(cell));
      }
    }
  }

  widths
}

fn table_cell_width(inlines: &[Inline]) -> f32 {
  (table_inline_width(inlines) + TABLE_CELL_HORIZONTAL_PADDING_PX).max(TABLE_CELL_MIN_WIDTH_PX)
}

fn table_inline_width(inlines: &[Inline]) -> f32 {
  if inlines.is_empty() {
    return 0.0;
  }

  let mut width = 0.0f32;
  let mut parts = 0usize;
  for inline in inlines {
    let part = table_inline_part_width(inline);
    if part > 0.0 {
      width += part;
      parts += 1;
    }
  }

  if parts > 1 {
    width += (parts as f32 - 1.0) * TABLE_INLINE_GAP_PX;
  }

  width
}

fn table_inline_part_width(inline: &Inline) -> f32 {
  match inline {
    Inline::Text(value) => table_text_width(value),
    Inline::Link { content, .. } => table_inline_width(content),
    Inline::Image { .. } => TABLE_BADGE_WIDTH_PX,
    Inline::Code(value) => table_text_width(value) + 8.0,
    Inline::SoftBreak | Inline::HardBreak => TABLE_INLINE_CHAR_WIDTH_PX,
    Inline::Strong(children) | Inline::Emphasis(children) | Inline::Strikethrough(children) => {
      table_inline_width(children)
    }
  }
}

fn table_text_width(value: &str) -> f32 {
  value.chars().count() as f32 * TABLE_INLINE_CHAR_WIDTH_PX
}

fn render_inline_selectable_text(
  inlines: &[Inline],
  options: &MarkdownRenderOptions,
  interactive: bool,
  _cx: &App,
  ctx: &mut RenderContext,
) -> AnyElement {
  let (text, spans, link_ranges) = build_spans(inlines, options);
  let text_id = ctx.next_text_id();

  SelectableText::new(
    text,
    spans,
    link_ranges,
    options.state.clone(),
    options.on_link.clone(),
    text_id,
    SelectableTextOptions {
      interactive,
      show_indentation_dots: false,
      show_inline_code_backgrounds: true,
    },
  )
  .into_any_element()
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum GithubMentionTextSegment {
  Text(String),
  Mention(String),
}

fn is_github_login_start_byte(byte: u8) -> bool {
  byte.is_ascii_alphanumeric()
}

fn is_github_login_byte(byte: u8) -> bool {
  byte.is_ascii_alphanumeric() || byte == b'-'
}

fn is_github_mention_prefix_char(ch: char) -> bool {
  ch.is_alphanumeric() || matches!(ch, '_' | '-' | '.')
}

fn is_github_mention_suffix_char(ch: char) -> bool {
  ch.is_alphanumeric() || matches!(ch, '_' | '/' | '@')
}

fn find_github_mention(value: &str, mut cursor: usize) -> Option<(usize, usize, String)> {
  let bytes = value.as_bytes();
  while cursor < value.len() {
    let Some(relative_at_ix) = value[cursor..].find('@') else {
      break;
    };
    let at_ix = cursor + relative_at_ix;
    let prefix_char = value[..at_ix].chars().next_back();
    if prefix_char.is_some_and(is_github_mention_prefix_char) {
      cursor = at_ix + 1;
      continue;
    }

    let login_start = at_ix + 1;
    if login_start >= value.len() || !is_github_login_start_byte(bytes[login_start]) {
      cursor = at_ix + 1;
      continue;
    }

    let mut login_end = login_start;
    while login_end < value.len() && is_github_login_byte(bytes[login_end]) {
      login_end += 1;
    }

    let login = &value[login_start..login_end];
    let suffix_char = value[login_end..].chars().next();
    if login.len() > 39
      || login.ends_with('-')
      || suffix_char.is_some_and(is_github_mention_suffix_char)
    {
      cursor = login_end;
      continue;
    }

    return Some((at_ix, login_end, login.to_string()));
  }

  None
}

fn split_github_mention_segments(value: &str) -> Vec<GithubMentionTextSegment> {
  let mut segments = Vec::new();
  let mut cursor = 0usize;

  while let Some((mention_start, mention_end, login)) = find_github_mention(value, cursor) {
    if mention_start > cursor {
      segments.push(GithubMentionTextSegment::Text(
        value[cursor..mention_start].to_string(),
      ));
    }

    segments.push(GithubMentionTextSegment::Mention(login));
    cursor = mention_end;
  }

  if cursor < value.len() {
    segments.push(GithubMentionTextSegment::Text(value[cursor..].to_string()));
  }

  segments
}

fn inline_contains_github_mention(inline: &Inline) -> bool {
  match inline {
    Inline::Text(value) => split_github_mention_segments(value)
      .iter()
      .any(|segment| matches!(segment, GithubMentionTextSegment::Mention(_))),
    Inline::Strong(_) | Inline::Emphasis(_) | Inline::Strikethrough(_) => false,
    Inline::Code(_)
    | Inline::Link { .. }
    | Inline::Image { .. }
    | Inline::SoftBreak
    | Inline::HardBreak => false,
  }
}

fn render_github_mention(
  login: &str,
  options: &MarkdownRenderOptions,
  interactive: bool,
  cx: &App,
  ctx: &mut RenderContext,
) -> AnyElement {
  let theme = cx.theme();
  let link_color = github_link_color(theme.background);
  let url: Arc<str> = format!("https://github.com/{login}").into();
  let avatar_url = format!("{}.png?size=40", url.as_ref());
  let label = format!("@{login}");
  let mention_id = ctx.next_text_id();
  let on_link = options.on_link.clone();

  h_flex()
    .id(format!("markdown-github-mention-{mention_id}"))
    .items_center()
    .gap_1()
    .flex_shrink_0()
    .text_sm()
    .font_medium()
    .text_color(link_color)
    .cursor(CursorStyle::PointingHand)
    .when(interactive, |this| {
      this.on_mouse_down(MouseButton::Left, move |_, window, cx| {
        cx.stop_propagation();
        let handled = on_link
          .as_ref()
          .map(|handler| handler(url.as_ref(), window, cx))
          .unwrap_or(LinkAction::Open);
        if handled == LinkAction::Open {
          cx.open_url(url.as_ref());
        }
      })
    })
    .child(
      Avatar::new()
        .name(login.to_string())
        .src(avatar_url)
        .with_size(px(18.0)),
    )
    .child(label)
    .into_any_element()
}

fn flush_inline_text_chunk(
  row_container: gpui::Div,
  text_chunk: &mut Vec<Inline>,
  options: &MarkdownRenderOptions,
  interactive: bool,
  cx: &App,
  ctx: &mut RenderContext,
) -> (gpui::Div, bool) {
  if text_chunk.is_empty() {
    return (row_container, false);
  }

  let row_container = row_container.child(render_inline_selectable_text(
    text_chunk,
    options,
    interactive,
    cx,
    ctx,
  ));
  text_chunk.clear();
  (row_container, true)
}

fn render_inline_row(
  row: &[Inline],
  options: &MarkdownRenderOptions,
  interactive: bool,
  image_render_context: Option<&MarkdownImageRenderContext<'_>>,
  cx: &App,
  ctx: &mut RenderContext,
) -> (gpui::Div, bool) {
  let mut row_container = h_flex().items_center().gap_1().flex_wrap().min_w_0();
  let mut row_has_content = false;
  let mut text_chunk: Vec<Inline> = Vec::new();

  for inline in row {
    if let Some(image_render_context) = image_render_context
      && let Some(image_data) = inline_image_data(inline)
    {
      let (next_container, _) = flush_inline_text_chunk(
        row_container,
        &mut text_chunk,
        options,
        interactive,
        cx,
        ctx,
      );
      row_container = next_container;
      row_container = row_container.child(render_inline_image(&image_data, image_render_context));
      row_has_content = true;
      continue;
    }

    if let Inline::Text(value) = inline {
      for segment in split_github_mention_segments(value) {
        match segment {
          GithubMentionTextSegment::Text(text) => text_chunk.push(Inline::Text(text)),
          GithubMentionTextSegment::Mention(login) => {
            let (next_container, _) = flush_inline_text_chunk(
              row_container,
              &mut text_chunk,
              options,
              interactive,
              cx,
              ctx,
            );
            row_container = next_container;
            row_container =
              row_container.child(render_github_mention(&login, options, interactive, cx, ctx));
            row_has_content = true;
          }
        }
      }
    } else {
      text_chunk.push(inline.clone());
    }
  }

  let (next_container, flushed) = flush_inline_text_chunk(
    row_container,
    &mut text_chunk,
    options,
    interactive,
    cx,
    ctx,
  );
  row_container = next_container;
  row_has_content |= flushed;

  (row_container, row_has_content)
}

fn render_inline_with_mentions(
  inlines: &[Inline],
  options: &MarkdownRenderOptions,
  interactive: bool,
  cx: &App,
  ctx: &mut RenderContext,
) -> AnyElement {
  let rows = split_inlines_by_hard_breaks(inlines);
  let mut content = div().flex().flex_col().min_w_0();
  let mut has_content = false;

  for (row_ix, row) in rows.iter().enumerate() {
    let (row_container, row_has_content) =
      render_inline_row(row, options, interactive, None, cx, ctx);
    if row_has_content {
      content = content.child(row_container);
      has_content = true;
    }

    if row_ix + 1 < rows.len() {
      content = content.child(div().h(px(MARKDOWN_IMAGE_HARD_BREAK_SPACER_PX)));
      has_content = true;
    }
  }

  if has_content {
    content.into_any_element()
  } else {
    div().into_any_element()
  }
}

fn render_inline_with_images(
  inlines: &[Inline],
  options: &MarkdownRenderOptions,
  interactive: bool,
  cx: &App,
  ctx: &mut RenderContext,
) -> AnyElement {
  let is_dark_mode = cx.theme().mode.is_dark();
  let image_render_context = MarkdownImageRenderContext {
    on_link: options.on_link.clone(),
    interactive,
    is_dark_mode,
    image_base_url: options.image_base_url.as_ref().map(SharedString::as_ref),
    asset_url_resolver: options.asset_url_resolver.as_ref(),
  };
  if let Some(image_data) = single_inline_image_data(inlines) {
    return render_block_image(&image_data, &image_render_context);
  }

  let rows = split_inlines_by_hard_breaks(inlines);
  let mut content = div().flex().flex_col().min_w_0();
  let mut has_content = false;

  for (row_ix, row) in rows.iter().enumerate() {
    let mut row_container = h_flex().items_center().gap_1().flex_wrap().min_w_0();
    let mut row_has_content = false;
    let mut inline_row: Vec<Inline> = Vec::new();

    for inline in row {
      if let Some(image_data) = inline_image_data(inline)
        && image_data.is_block_sized()
      {
        // Flush current row, then render the image as a block on its own line.
        if !inline_row.is_empty() {
          let (flushed_container, flushed_content) = render_inline_row(
            &inline_row,
            options,
            interactive,
            Some(&image_render_context),
            cx,
            ctx,
          );
          row_container = flushed_container;
          row_has_content = flushed_content;
          inline_row.clear();
        }
        if row_has_content {
          content = content.child(row_container);
        }
        content = content.child(render_block_image(&image_data, &image_render_context));
        has_content = true;
        row_container = h_flex().items_center().gap_1().flex_wrap().min_w_0();
        row_has_content = false;
        continue;
      }

      inline_row.push(inline.clone());
    }

    if !inline_row.is_empty() {
      let (flushed_container, flushed_content) = render_inline_row(
        &inline_row,
        options,
        interactive,
        Some(&image_render_context),
        cx,
        ctx,
      );
      row_container = flushed_container;
      row_has_content = flushed_content;
    }

    if row_has_content {
      content = content.child(row_container);
      has_content = true;
    }

    // Keep explicit <br>/<hard break> vertical rhythm around image rows.
    if row_ix + 1 < rows.len() {
      content = content.child(div().h(px(MARKDOWN_IMAGE_HARD_BREAK_SPACER_PX)));
      has_content = true;
    }
  }

  if has_content {
    content.into_any_element()
  } else {
    div().into_any_element()
  }
}

pub(crate) fn render_inline_text(
  inlines: &[Inline],
  options: &MarkdownRenderOptions,
  cx: &App,
  ctx: &mut RenderContext,
) -> AnyElement {
  if inlines.iter().any(inline_contains_image) {
    return render_inline_with_images(inlines, options, true, cx, ctx);
  }

  if inlines.iter().any(inline_contains_github_mention) {
    return render_inline_with_mentions(inlines, options, true, cx, ctx);
  }

  render_inline_selectable_text(inlines, options, true, cx, ctx)
}

fn render_inline_static(
  inlines: &[Inline],
  options: &MarkdownRenderOptions,
  cx: &App,
  ctx: &mut RenderContext,
) -> AnyElement {
  if inlines.iter().any(inline_contains_image) {
    return render_inline_with_images(inlines, options, false, cx, ctx);
  }

  if inlines.iter().any(inline_contains_github_mention) {
    return render_inline_with_mentions(inlines, options, false, cx, ctx);
  }

  render_inline_selectable_text(inlines, options, false, cx, ctx)
}

fn render_heading_text(
  level: u8,
  inlines: &[Inline],
  options: &MarkdownRenderOptions,
  cx: &App,
  ctx: &mut RenderContext,
) -> AnyElement {
  let theme = cx.theme();
  let mut container = div()
    .whitespace_normal()
    .text_color(theme.foreground)
    .child(render_inline_text(inlines, options, cx, ctx));

  container = match level {
    1 => container.text_3xl().font_bold(),
    2 => container.text_2xl().font_bold(),
    3 => container.text_xl().font_semibold(),
    _ => container.text_lg().font_medium(),
  };

  container.into_any_element()
}

// ScrollHandles hold `Rc<RefCell<_>>` and are !Send, so we cache them in a
// thread-local. GPUI renders on the main thread; this keeps the handle
// state persistent across frames without needing Send/Sync on the cache.
thread_local! {
  static SCROLLABLE_HANDLES: std::cell::RefCell<HashMap<u64, gpui::ScrollHandle>> =
    std::cell::RefCell::new(HashMap::new());
}

fn scrollable_handle(key: u64) -> gpui::ScrollHandle {
  SCROLLABLE_HANDLES.with(|map| {
    map
      .borrow_mut()
      .entry(key)
      .or_insert_with(gpui::ScrollHandle::new)
      .clone()
  })
}

fn code_block_scroll_key(instance_id: usize, text_id: usize) -> u64 {
  // Namespace by combining the markdown instance id with the text id so
  // separate markdown views don't share state.
  0x1000_0000_0000_0000_u64 | ((instance_id as u64) << 32) | (text_id as u32 as u64)
}

fn suggestion_block_scroll_key(instance_id: usize, text_id: usize) -> u64 {
  0x2000_0000_0000_0000_u64 | ((instance_id as u64) << 32) | (text_id as u32 as u64)
}

fn preview_card_scroll_key(preview_hash: u64) -> u64 {
  // Keep the high bit clear to avoid collisions with the namespaced keys above.
  preview_hash & 0x0fff_ffff_ffff_ffff
}

fn render_code_block(
  code: &CodeBlock,
  options: &MarkdownRenderOptions,
  cx: &App,
  ctx: &mut RenderContext,
) -> AnyElement {
  let theme = cx.theme();
  let (text, spans, link_ranges) = build_code_block_spans(code, options.syntax_cache.as_ref());
  let min_content_width_px = estimate_code_block_min_content_width_px(text.as_ref());
  let text_id = ctx.next_text_id();
  let content = SelectableText::new(
    text,
    spans,
    link_ranges,
    options.state.clone(),
    options.on_link.clone(),
    text_id,
    code_block_selectable_text_options(),
  );
  let scroll_id: SharedString = format!("markdown-code-block-scroll-{text_id}").into();
  let scroll_content = restrict_scroll_to_wheel_axis(div().id(scroll_id).w_full().min_w_0()).child(
    div()
      .min_w(px(min_content_width_px))
      .px(px(MARKDOWN_CODE_BLOCK_PADDING_X_PX))
      .pt(px(MARKDOWN_CODE_BLOCK_PADDING_TOP_PX))
      .pb(px(MARKDOWN_CODE_BLOCK_PADDING_BOTTOM_PX))
      .whitespace_nowrap()
      .child(
        div()
          .pl(px(MARKDOWN_CODE_BLOCK_TEXT_SHIFT_X_PX))
          .font_family(theme.mono_font_family.clone())
          .text_size(theme.mono_font_size)
          .text_color(theme.foreground)
          .child(content),
      ),
  );

  let scroll_key = code_block_scroll_key(options.state.instance_id, text_id);
  let scroll_handle = scrollable_handle(scroll_key);
  let expanded = options.expand_code_blocks;
  let scroll_container = if expanded {
    let scroll_content = scroll_content
      .overflow_x_scroll()
      .track_scroll(&scroll_handle);
    scrollable_node(
      scroll_content,
      &scroll_handle,
      ScrollAxes::horizontal(),
      scroll_key,
    )
    .into_any_element()
  } else {
    let scroll_content = scroll_content
      .max_h(px(MARKDOWN_CODE_BLOCK_MAX_HEIGHT_PX))
      .overflow_scroll()
      .track_scroll(&scroll_handle);
    scrollable_node(
      scroll_content,
      &scroll_handle,
      ScrollAxes::both(),
      scroll_key,
    )
    .into_any_element()
  };
  let copy_value = code_block_copy_value(code);
  let hover_group_id = code_block_hover_group_id(text_id);

  let mut wrapper: Div = div()
    .w_full()
    .min_w_0()
    .relative()
    .group(hover_group_id.clone())
    .bg(theme.muted)
    .rounded_md()
    .overflow_hidden()
    .child(scroll_container)
    .horizontal_scrollbar(&scroll_handle);
  if !expanded {
    wrapper = wrapper.vertical_scrollbar(&scroll_handle);
  }
  wrapper
    .child(
      div()
        .absolute()
        .top_2()
        .right_2()
        .invisible()
        .group_hover(&hover_group_id, |this| this.visible())
        .child(Clipboard::new(("markdown-code-block-copy", text_id)).value(copy_value)),
    )
    .into_any_element()
}

fn render_suggestion_block(
  code: &CodeBlock,
  suggestion_ctx: &SuggestionContext,
  options: &MarkdownRenderOptions,
  cx: &App,
  ctx: &mut RenderContext,
) -> AnyElement {
  let theme = cx.theme();
  let old_text_id = ctx.next_text_id();
  let original_start_line = suggestion_ctx.original_start_line;
  let suggested_start_line = suggestion_ctx.suggested_start_line;
  let suggested_value = code.value.strip_suffix('\n').unwrap_or(code.value.as_str());
  let suggested_lines: Vec<String> = if suggested_value.is_empty() {
    Vec::new()
  } else {
    suggested_value.split('\n').map(str::to_string).collect()
  };
  let mut diff_lines =
    Vec::with_capacity(suggestion_ctx.original_lines.len() + suggested_lines.len());
  diff_lines.extend(
    suggestion_ctx
      .original_lines
      .iter()
      .enumerate()
      .map(|(ix, line)| GithubDiffLine {
        old_line: original_start_line.map(|start| start + ix),
        new_line: None,
        content: Arc::from(line.as_str()),
        kind: GithubDiffLineKind::Removed,
      }),
  );
  diff_lines.extend(
    suggested_lines
      .iter()
      .enumerate()
      .map(|(ix, line)| GithubDiffLine {
        old_line: None,
        new_line: suggested_start_line.map(|start| start + ix),
        content: Arc::from(line.as_str()),
        kind: GithubDiffLineKind::Added,
      }),
  );

  let min_content_width_px = diff_lines
    .iter()
    .map(|line| estimate_code_block_min_content_width_px(line.content.as_ref()))
    .fold(0.0, f32::max)
    + 120.0;
  let copy_value: SharedString = code.value.clone().into();
  let hover_group_id: SharedString = format!("markdown-suggestion-hover-{old_text_id}").into();
  let scroll_id: SharedString = format!("markdown-suggestion-scroll-{old_text_id}").into();
  let action = options.suggestion_action.as_ref().map(|render| {
    render(
      SuggestionActionContext {
        path: suggestion_ctx.path.clone(),
        original_start_line,
        original_lines: suggestion_ctx.original_lines.clone(),
        suggested_lines: suggested_lines.clone(),
      },
      cx,
    )
  });

  let header = h_flex()
    .items_center()
    .justify_between()
    .gap_3()
    .px_3()
    .py_1p5()
    .border_b_1()
    .border_color(theme.border)
    .bg(theme.sidebar)
    .child(
      div()
        .text_xs()
        .font_medium()
        .text_color(theme.muted_foreground)
        .child("Suggested change"),
    )
    .child(
      h_flex()
        .items_center()
        .gap_1()
        .when_some(action, |this, action| this.child(action))
        .child(Clipboard::new(("markdown-suggestion-copy", old_text_id)).value(copy_value)),
    );

  let suggestion_scroll_key = suggestion_block_scroll_key(options.state.instance_id, old_text_id);
  let suggestion_scroll_handle = scrollable_handle(suggestion_scroll_key);
  let scroll_content =
    restrict_scroll_to_wheel_axis(div().id(scroll_id).w_full().min_w_0().overflow_x_scroll())
      .track_scroll(&suggestion_scroll_handle)
      .child(render_github_diff_lines(
        &diff_lines,
        suggestion_ctx.path.as_ref(),
        old_text_id,
        options.state.clone(),
        min_content_width_px,
        cx,
      ));
  let scroll_content = scrollable_node(
    scroll_content,
    &suggestion_scroll_handle,
    ScrollAxes::horizontal(),
    suggestion_scroll_key,
  )
  .into_any_element();

  div()
    .w_full()
    .min_w_0()
    .relative()
    .group(hover_group_id.clone())
    .border_1()
    .border_color(theme.border)
    .rounded_md()
    .child(header)
    .child(scroll_content)
    .horizontal_scrollbar(&suggestion_scroll_handle)
    .into_any_element()
}

fn code_block_selectable_text_options() -> SelectableTextOptions {
  SelectableTextOptions {
    interactive: true,
    show_indentation_dots: true,
    show_inline_code_backgrounds: false,
  }
}

fn code_block_hover_group_id(text_id: usize) -> SharedString {
  format!("markdown-code-block-hover-{text_id}").into()
}

fn code_block_copy_value(code: &CodeBlock) -> SharedString {
  code.value.clone().into()
}

fn code_block_display_value(code: &CodeBlock) -> String {
  let mut value = code.value.as_str();
  if let Some(stripped) = value.strip_suffix('\n') {
    value = stripped.strip_suffix('\r').unwrap_or(stripped);
  }

  if is_plain_text_code_fence_language(code.lang.as_deref()) {
    value = strip_trailing_orphan_details_line(value);
  }

  let widened = expand_leading_spaces_for_code_block(value);
  expand_tabs_for_code_block(widened.as_ref())
}

fn estimate_code_block_min_content_width_px(text: &str) -> f32 {
  let widest_line_columns = text
    .lines()
    .map(|line| line.chars().count())
    .max()
    .unwrap_or(0) as f32;
  let chrome_width_px =
    MARKDOWN_CODE_BLOCK_PADDING_X_PX * 2.0 + MARKDOWN_CODE_BLOCK_TEXT_SHIFT_X_PX;
  (widest_line_columns * MARKDOWN_CODE_BLOCK_APPROX_CHAR_WIDTH_PX + chrome_width_px).ceil()
}

fn expand_leading_spaces_for_code_block(value: &str) -> String {
  if !value.contains(' ') || MARKDOWN_CODE_BLOCK_LEADING_SPACE_RENDER_MULTIPLIER <= 1 {
    return value.to_string();
  }

  let mut widened =
    String::with_capacity(value.len() * MARKDOWN_CODE_BLOCK_LEADING_SPACE_RENDER_MULTIPLIER);
  let mut in_leading_indent = true;

  for ch in value.chars() {
    match ch {
      ' ' if in_leading_indent => {
        for _ in 0..MARKDOWN_CODE_BLOCK_LEADING_SPACE_RENDER_MULTIPLIER {
          widened.push(' ');
        }
      }
      '\n' => {
        widened.push('\n');
        in_leading_indent = true;
      }
      '\r' => {
        widened.push('\r');
        in_leading_indent = true;
      }
      '\t' => {
        widened.push('\t');
      }
      _ => {
        widened.push(ch);
        in_leading_indent = false;
      }
    }
  }

  widened
}

fn expand_tabs_for_code_block(value: &str) -> String {
  if !value.contains('\t') {
    return value.to_string();
  }

  let mut expanded = String::with_capacity(value.len());
  let mut column = 0usize;
  for ch in value.chars() {
    match ch {
      '\t' => {
        let spaces = MARKDOWN_CODE_BLOCK_TAB_WIDTH - (column % MARKDOWN_CODE_BLOCK_TAB_WIDTH);
        for _ in 0..spaces {
          expanded.push(' ');
        }
        column += spaces;
      }
      '\n' => {
        expanded.push('\n');
        column = 0;
      }
      '\r' => {
        expanded.push('\r');
        column = 0;
      }
      _ => {
        expanded.push(ch);
        column += 1;
      }
    }
  }

  expanded
}

pub(crate) struct IndentationIndicators {
  pub dot_indices: Vec<usize>,
  pub tab_indices: Vec<usize>,
}

pub(crate) fn collect_indentation_indicators(text: &str) -> IndentationIndicators {
  if text.len() > MARKDOWN_CODE_INDENT_DOT_DISABLE_ABOVE_TEXT_LEN {
    return IndentationIndicators {
      dot_indices: Vec::new(),
      tab_indices: Vec::new(),
    };
  }

  let mut dot_indices = Vec::new();
  let mut tab_indices = Vec::new();
  let mut leading_spaces = Vec::new();
  let mut leading_tabs = Vec::new();
  let mut saw_non_whitespace = false;
  let mut in_leading_indent = true;

  for (ix, ch) in text.char_indices() {
    match ch {
      '\n' | '\r' => {
        if saw_non_whitespace {
          dot_indices.extend_from_slice(&leading_spaces);
          tab_indices.extend_from_slice(&leading_tabs);
        }
        leading_spaces.clear();
        leading_tabs.clear();
        saw_non_whitespace = false;
        in_leading_indent = true;
      }
      ' ' if in_leading_indent => {
        leading_spaces.push(ix);
      }
      '\t' if in_leading_indent => {
        leading_tabs.push(ix);
      }
      ' ' | '\t' => {}
      _ => {
        saw_non_whitespace = true;
        in_leading_indent = false;
      }
    }
  }

  if saw_non_whitespace {
    dot_indices.extend_from_slice(&leading_spaces);
    tab_indices.extend_from_slice(&leading_tabs);
  }

  IndentationIndicators {
    dot_indices: limit_indentation_dot_indices(dot_indices),
    tab_indices,
  }
}

fn limit_indentation_dot_indices(indices: Vec<usize>) -> Vec<usize> {
  if indices.len() <= MARKDOWN_CODE_INDENT_DOT_MAX_RENDER_COUNT {
    return indices;
  }

  let step = indices
    .len()
    .div_ceil(MARKDOWN_CODE_INDENT_DOT_MAX_RENDER_COUNT);
  indices.into_iter().step_by(step).collect()
}

fn strip_trailing_orphan_details_line(value: &str) -> &str {
  let trimmed_end = value.trim_end_matches([' ', '\t', '\r', '\n']);
  let line_start = trimmed_end.rfind('\n').map(|idx| idx + 1).unwrap_or(0);
  if line_start == 0 {
    return value;
  }

  let last_line = trimmed_end[line_start..].trim();
  if !last_line.eq_ignore_ascii_case("</details>") {
    return value;
  }

  value[..line_start].trim_end_matches([' ', '\t', '\r', '\n'])
}

fn is_plain_text_code_fence_language(lang: Option<&str>) -> bool {
  let Some(lang) = lang else {
    return true;
  };
  let lang = lang
    .trim()
    .trim_matches(|c| c == '{' || c == '}')
    .trim_start_matches('.')
    .to_ascii_lowercase();
  if lang.is_empty() {
    return true;
  }

  matches!(
    lang.as_str(),
    "text" | "txt" | "plain" | "plaintext" | "log" | "output" | "console"
  )
}

fn build_code_block_spans(
  code: &CodeBlock,
  syntax_cache: Option<&Arc<crate::syntax_cache::SyntaxHighlightCache>>,
) -> (SharedString, Vec<InlineSpan>, Vec<LinkRange>) {
  // Check cache first, returns highlighted spans if previously computed
  if let Some(cache) = syntax_cache
    && let Some(cached) = cache.get(code)
  {
    return cached;
  }

  let display_value = code_block_display_value(code);
  let text = SharedString::from(display_value.clone());
  let text_len = text.len();
  let base_style = InlineStyle {
    code: true,
    ..InlineStyle::default()
  };

  if text_len == 0 {
    return (text, Vec::new(), Vec::new());
  }

  // If we have a cache, schedule async highlight and return plain spans now
  if let Some(cache) = syntax_cache
    && code_block_language_config(code.lang.as_deref()).is_some()
  {
    cache.schedule_highlight(code, &display_value);
    // Return plain (uncolored) spans, will be replaced on next render after background completes
    let plain_spans = vec![InlineSpan {
      range: 0..text_len,
      style: base_style,
      link: None,
      syntax_token: None,
      background: None,
    }];
    return (text, plain_spans, Vec::new());
  }

  // No cache, synchronous highlight (fallback for preview cards etc.)
  let spans = code_block_language_config(code.lang.as_deref())
    .and_then(|config| {
      let mut highlighter = SyntaxHighlighter::new(config);
      highlighter
        .highlight_text(display_value.as_ref())
        .ok()
        .map(|highlights| {
          syntax_highlight_spans_for_code(display_value.as_ref(), &highlights, base_style)
        })
    })
    .filter(|spans| !spans.is_empty())
    .unwrap_or_else(|| {
      vec![InlineSpan {
        range: 0..text_len,
        style: base_style,
        link: None,
        syntax_token: None,
        background: None,
      }]
    });

  (text, spans, Vec::new())
}

fn code_block_language_config(lang: Option<&str>) -> Option<&'static syntax::LanguageConfig> {
  let lang = lang?
    .trim()
    .trim_matches(|c| c == '{' || c == '}')
    .trim_start_matches('.');
  if lang.is_empty() {
    return None;
  }

  languages::language_config_for_name(lang).or_else(|| languages::detect_language_config(lang))
}

pub(crate) fn syntax_highlight_spans_for_code(
  text: &str,
  highlights: &[HighlightSpan],
  base_style: InlineStyle,
) -> Vec<InlineSpan> {
  let text_len = text.len();
  if text_len == 0 {
    return Vec::new();
  }

  let mut syntax_ranges: Vec<_> = highlights
    .iter()
    .filter_map(|highlight| {
      let start = clamp_to_char_boundary(text, highlight.byte_range.start.min(text_len));
      let end = clamp_to_char_boundary(text, highlight.byte_range.end.min(text_len));
      (end > start).then_some((start..end, highlight.token_type))
    })
    .collect();
  syntax_ranges.sort_by_key(|(range, _)| (range.start, range.end));

  let mut spans = Vec::new();
  let mut current_pos = 0usize;
  for (range, token_type) in syntax_ranges {
    let start = range.start.max(current_pos);
    let end = range.end.min(text_len);
    if end <= start {
      continue;
    }

    if start > current_pos {
      spans.push(InlineSpan {
        range: current_pos..start,
        style: base_style,
        link: None,
        syntax_token: None,
        background: None,
      });
    }

    spans.push(InlineSpan {
      range: start..end,
      style: base_style,
      link: None,
      syntax_token: Some(token_type),
      background: None,
    });
    current_pos = end;
  }

  if current_pos < text_len {
    spans.push(InlineSpan {
      range: current_pos..text_len,
      style: base_style,
      link: None,
      syntax_token: None,
      background: None,
    });
  }

  spans
}

fn render_details(
  details: &Details,
  options: &MarkdownRenderOptions,
  indent: usize,
  cx: &App,
  ctx: &mut RenderContext,
) -> AnyElement {
  let theme = cx.theme();
  let details_id = ctx.next_details_id();
  let is_open = {
    let mut map = options.state.details_open.lock().unwrap();
    map.entry(details_id).or_insert(details.open);
    *map.get(&details_id).unwrap_or(&details.open)
  };

  let toggle_icon = if is_open {
    gpui_component::IconName::ChevronDown
  } else {
    gpui_component::IconName::ChevronRight
  };

  let toggle_state = options.state.clone();
  let default_open = details.open;
  let toggle_id = format!(
    "gfm-details-toggle-{}-{}",
    options.state.instance_id, details_id
  );
  let has_links = details.summary.iter().any(inline_contains_link);
  let summary_content = if has_links {
    render_inline_selectable_text(&details.summary, options, true, cx, ctx)
  } else {
    render_inline_static(&details.summary, options, cx, ctx)
  };

  let summary_text = div()
    .whitespace_normal()
    .text_sm()
    .font_medium()
    .text_color(theme.foreground)
    .child(summary_content);

  let summary: AnyElement = if has_links {
    h_flex()
      .items_center()
      .gap_2()
      .child(
        div()
          .id(toggle_id)
          .cursor_pointer()
          .child(gpui_component::Icon::new(toggle_icon).small())
          .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            let mut map = toggle_state.details_open.lock().unwrap();
            let next = !map.get(&details_id).copied().unwrap_or(default_open);
            map.insert(details_id, next);
            window.refresh();
            cx.stop_propagation();
          }),
      )
      .child(summary_text)
      .into_any_element()
  } else {
    h_flex()
      .id(toggle_id)
      .cursor_pointer()
      .items_center()
      .gap_2()
      .child(gpui_component::Icon::new(toggle_icon).small())
      .child(summary_text)
      .on_mouse_down(MouseButton::Left, move |_, window, cx| {
        let mut map = toggle_state.details_open.lock().unwrap();
        let next = !map.get(&details_id).copied().unwrap_or(default_open);
        map.insert(details_id, next);
        window.refresh();
        cx.stop_propagation();
      })
      .into_any_element()
  };

  let mut container = div().flex().flex_col().gap_2().child(summary);
  if is_open {
    let body = render_blocks(&details.blocks, options, indent + 1, cx, ctx);
    container = container.child(body);
  }

  container.into_any_element()
}

fn build_spans(
  inlines: &[Inline],
  options: &MarkdownRenderOptions,
) -> (SharedString, Vec<InlineSpan>, Vec<LinkRange>) {
  let mut builder = SpanBuilder {
    github_issue_reference_context: options.github_issue_reference_context.clone(),
    hardbreaks: options.hardbreaks,
    ..Default::default()
  };
  builder.push_inlines(inlines, InlineStyle::default(), None);
  builder.finish()
}

#[derive(Default)]
struct SpanBuilder {
  text: String,
  spans: Vec<InlineSpan>,
  github_issue_reference_context: Option<GithubIssueReferenceContext>,
  hardbreaks: bool,
}

impl SpanBuilder {
  fn push_inlines(&mut self, inlines: &[Inline], style: InlineStyle, link: Option<Arc<str>>) {
    for inline in inlines {
      match inline {
        Inline::Text(value) => self.push_text(value, style, link.clone()),
        Inline::Code(value) => {
          let mut code_style = style;
          code_style.code = true;
          self.push_text("\u{2009}", style, link.clone());
          self.push_text(value, code_style, link.clone());
          self.push_text("\u{2009}", style, link.clone());
        }
        Inline::SoftBreak => {
          if self.hardbreaks {
            self.push_text("\n", style, link.clone());
          } else {
            self.push_text(" ", style, link.clone());
          }
        }
        Inline::HardBreak => self.push_text("\n", style, link.clone()),
        Inline::Strong(children) => {
          let mut strong_style = style;
          strong_style.bold = true;
          self.push_inlines(children, strong_style, link.clone());
        }
        Inline::Emphasis(children) => {
          let mut em_style = style;
          em_style.italic = true;
          self.push_inlines(children, em_style, link.clone());
        }
        Inline::Strikethrough(children) => {
          let mut strike_style = style;
          strike_style.strike = true;
          self.push_inlines(children, strike_style, link.clone());
        }
        Inline::Link { url, content, .. } => {
          let url = Arc::<str>::from(url.as_str());
          self.push_inlines(content, style, Some(url));
        }
        Inline::Image { alt, .. } => self.push_text(alt, style, link.clone()),
      }
    }
  }

  fn push_text(&mut self, value: &str, style: InlineStyle, link: Option<Arc<str>>) {
    if value.is_empty() {
      return;
    }

    if style.code || link.is_some() || self.github_issue_reference_context.is_none() {
      self.push_text_span(value, style, link);
      return;
    }

    self.push_text_with_issue_reference_links(value, style);
  }

  fn push_text_with_issue_reference_links(&mut self, value: &str, style: InlineStyle) {
    let Some((owner, repo)) = self
      .github_issue_reference_context
      .as_ref()
      .map(|context| (context.owner.clone(), context.repo.clone()))
    else {
      self.push_text_span(value, style, None);
      return;
    };

    let mut cursor = 0usize;
    while cursor < value.len() {
      let Some(relative_hash_ix) = value[cursor..].find('#') else {
        break;
      };
      let hash_ix = cursor + relative_hash_ix;
      let prefix_char = value[..hash_ix].chars().next_back();
      if prefix_char.is_some_and(is_issue_reference_prefix_char) {
        cursor = hash_ix + 1;
        continue;
      }

      let digits_start = hash_ix + 1;
      if digits_start >= value.len() || !value.as_bytes()[digits_start].is_ascii_digit() {
        cursor = hash_ix + 1;
        continue;
      }

      let mut digits_end = digits_start;
      while digits_end < value.len() && value.as_bytes()[digits_end].is_ascii_digit() {
        digits_end += 1;
      }
      let suffix_char = value[digits_end..].chars().next();
      if suffix_char.is_some_and(is_issue_reference_suffix_char) {
        cursor = digits_end;
        continue;
      }

      if hash_ix > cursor {
        self.push_text_span(&value[cursor..hash_ix], style, None);
      }

      let issue_number = &value[digits_start..digits_end];
      let issue_url: Arc<str> =
        format!("https://github.com/{owner}/{repo}/issues/{issue_number}").into();
      self.push_text_span(&value[hash_ix..digits_end], style, Some(issue_url));
      cursor = digits_end;
    }

    if cursor < value.len() {
      self.push_text_span(&value[cursor..], style, None);
    }
  }

  fn push_text_span(&mut self, value: &str, style: InlineStyle, link: Option<Arc<str>>) {
    if value.is_empty() {
      return;
    }

    let start = self.text.len();
    self.text.push_str(value);
    let end = self.text.len();

    if let Some(last) = self.spans.last_mut()
      && last.style == style
      && last.link == link
      && last.syntax_token.is_none()
    {
      last.range.end = end;
      return;
    }

    self.spans.push(InlineSpan {
      range: start..end,
      style,
      link,
      syntax_token: None,
      background: None,
    });
  }

  fn finish(self) -> (SharedString, Vec<InlineSpan>, Vec<LinkRange>) {
    let mut link_ranges = Vec::new();
    let mut current: Option<LinkRange> = None;
    for span in &self.spans {
      if let Some(url) = span.link.clone() {
        match current.as_mut() {
          Some(range) if range.url == url && range.range.end == span.range.start => {
            range.range.end = span.range.end;
          }
          _ => {
            if let Some(existing) = current.take() {
              link_ranges.push(existing);
            }
            current = Some(LinkRange {
              range: span.range.clone(),
              url,
            });
          }
        }
      } else if let Some(existing) = current.take() {
        link_ranges.push(existing);
      }
    }
    if let Some(existing) = current.take() {
      link_ranges.push(existing);
    }

    (SharedString::from(self.text), self.spans, link_ranges)
  }
}

fn is_issue_reference_prefix_char(ch: char) -> bool {
  ch.is_alphanumeric() || matches!(ch, '_' | '/' | '-')
}

fn is_issue_reference_suffix_char(ch: char) -> bool {
  ch.is_alphanumeric() || ch == '_'
}

// HTML parsing functions moved to crate::parse_html

#[cfg(test)]
mod tests {
  use super::*;

  fn test_preview_for_url(url: &str) -> GithubCodeReferencePreview {
    GithubCodeReferencePreview {
      url: Arc::from(url),
      repo: Arc::from("acme/widget"),
      path: Arc::from("docker-compose.yml"),
      reference: Arc::from("main"),
      start_line: 7,
      end_line: 9,
      snippets: vec![Arc::from("services:")],
      full_content: None,
    }
  }

  fn test_preview_map(url: &str) -> HashMap<Arc<str>, GithubCodeReferencePreview> {
    HashMap::from([(Arc::from(url), test_preview_for_url(url))])
  }

  #[test]
  fn markdown_render_options_expanded_code_blocks_flag_is_opt_in() {
    let defaults = MarkdownRenderOptions::default();
    assert!(!defaults.expand_code_blocks);

    let expanded = MarkdownRenderOptions::default().with_expanded_code_blocks();
    assert!(expanded.expand_code_blocks);
  }

  #[test]
  fn should_resolve_svg_embedded_image_allows_simple_image_wrapper() {
    let svg = r#"<svg width="16" height="16" xmlns="http://www.w3.org/2000/svg">
  <image href="data:image/png;base64,AAA=" width="16" height="16"/>
</svg>"#;
    assert!(should_resolve_svg_embedded_image(svg));
  }

  #[test]
  fn should_resolve_svg_embedded_image_rejects_badge_like_svg() {
    let svg = r##"<svg width="86" height="20" xmlns="http://www.w3.org/2000/svg">
  <rect width="86" height="20" fill="#555"/>
  <image href="data:image/png;base64,AAA=" x="5" y="3" width="14" height="14"/>
  <text x="25" y="14">Zed</text>
</svg>"##;
    assert!(!should_resolve_svg_embedded_image(svg));
  }

  #[test]
  fn parse_markdown_image_dimension_supports_pixels_and_percent() {
    assert_eq!(
      parse_markdown_image_dimension(Some("200px")),
      Some(MarkdownImageDimension::Pixels(200.0))
    );
    assert_eq!(
      parse_markdown_image_dimension(Some("200")),
      Some(MarkdownImageDimension::Pixels(200.0))
    );
    assert_eq!(
      parse_markdown_image_dimension(Some("85%")),
      Some(MarkdownImageDimension::Fraction(0.85))
    );
  }

  #[test]
  fn parse_markdown_image_dimension_rejects_invalid_values() {
    assert_eq!(parse_markdown_image_dimension(Some("")), None);
    assert_eq!(parse_markdown_image_dimension(Some("abc")), None);
    assert_eq!(parse_markdown_image_dimension(Some("0")), None);
    assert_eq!(parse_markdown_image_dimension(Some("0%")), None);
    assert_eq!(parse_markdown_image_dimension(None), None);
  }

  #[test]
  fn parse_inline_html_image_reads_width_and_height_attributes() {
    let inline = parse_inline_html_image(
      r#"<img src="logo.svg" width="200px" height="80px" align="center" alt="Zod logo" />"#,
    )
    .expect("inline image");

    match inline {
      Inline::Image {
        url,
        alt,
        width,
        height,
        ..
      } => {
        assert_eq!(url, "logo.svg");
        assert_eq!(alt, "Zod logo");
        assert_eq!(width.as_deref(), Some("200px"));
        assert_eq!(height.as_deref(), Some("80px"));
      }
      _ => panic!("expected image"),
    }
  }

  #[test]
  fn resolve_markdown_image_url_joins_relative_path_with_base_url() {
    let resolved = resolve_markdown_image_url(
      "./assets/hero.gif",
      Some("https://raw.githubusercontent.com/acme/widget/main/docs/"),
    );

    assert_eq!(
      resolved,
      "https://raw.githubusercontent.com/acme/widget/main/docs/assets/hero.gif"
    );
  }

  #[test]
  fn resolve_markdown_image_url_treats_leading_slash_as_repo_root() {
    let resolved = resolve_markdown_image_url(
      "/assets/hero.gif",
      Some("https://raw.githubusercontent.com/acme/widget/main/docs/"),
    );

    assert_eq!(
      resolved,
      "https://raw.githubusercontent.com/acme/widget/main/assets/hero.gif"
    );
  }

  #[test]
  fn resolve_markdown_image_url_keeps_absolute_urls() {
    let absolute = "https://images.example.com/hero.gif";
    assert_eq!(resolve_markdown_image_url(absolute, None), absolute);
    assert_eq!(
      resolve_markdown_image_url("//images.example.com/hero.gif", None),
      "https://images.example.com/hero.gif"
    );
  }

  #[test]
  fn parses_links_and_text() {
    let blocks = parse_gfm("See [comment](https://github.com/org/repo/pull/4/changes#r123)");
    assert_eq!(blocks.len(), 1);
    match &blocks[0] {
      Block::Paragraph(inlines) => {
        assert!(matches!(inlines[0], Inline::Text(_)));
        assert!(matches!(inlines[1], Inline::Link { .. }));
      }
      _ => panic!("expected paragraph"),
    }
  }

  #[test]
  fn build_spans_linkifies_issue_references_with_repo_context() {
    let inlines = vec![Inline::Text("Fixes #5320 and closes #81.".to_string())];
    let options =
      MarkdownRenderOptions::default().with_github_issue_reference_context("acme", "widget");

    let (text, _, link_ranges) = build_spans(&inlines, &options);
    let rendered = text.as_ref();

    assert_eq!(rendered, "Fixes #5320 and closes #81.");
    assert_eq!(link_ranges.len(), 2);
    assert_eq!(&rendered[link_ranges[0].range.clone()], "#5320");
    assert_eq!(
      link_ranges[0].url.as_ref(),
      "https://github.com/acme/widget/issues/5320"
    );
    assert_eq!(&rendered[link_ranges[1].range.clone()], "#81");
    assert_eq!(
      link_ranges[1].url.as_ref(),
      "https://github.com/acme/widget/issues/81"
    );
  }

  #[test]
  fn build_spans_does_not_linkify_issue_references_without_repo_context() {
    let inlines = vec![Inline::Text("Fixes #5320.".to_string())];
    let options = MarkdownRenderOptions::default();

    let (_, _, link_ranges) = build_spans(&inlines, &options);
    assert!(link_ranges.is_empty());
  }

  #[test]
  fn build_spans_does_not_linkify_owner_repo_shorthand_before_issue_number() {
    let inlines = vec![Inline::Text(
      "Use owner/repo#5320 and then #81.".to_string(),
    )];
    let options =
      MarkdownRenderOptions::default().with_github_issue_reference_context("acme", "widget");

    let (text, _, link_ranges) = build_spans(&inlines, &options);
    let rendered = text.as_ref();
    assert_eq!(link_ranges.len(), 1);
    assert_eq!(&rendered[link_ranges[0].range.clone()], "#81");
    assert_eq!(
      link_ranges[0].url.as_ref(),
      "https://github.com/acme/widget/issues/81"
    );
  }

  #[test]
  fn build_spans_keeps_issue_reference_detection_outside_code_and_existing_links() {
    let inlines = vec![
      Inline::Code("#11".to_string()),
      Inline::Text(" and #22 ".to_string()),
      Inline::Link {
        url: "https://example.com/already-linked".to_string(),
        title: None,
        content: vec![Inline::Text("#33".to_string())],
      },
    ];
    let options =
      MarkdownRenderOptions::default().with_github_issue_reference_context("acme", "widget");

    let (text, _, link_ranges) = build_spans(&inlines, &options);
    let rendered = text.as_ref();
    assert_eq!(rendered, "\u{2009}#11\u{2009} and #22 #33");
    assert_eq!(link_ranges.len(), 2);
    assert_eq!(&rendered[link_ranges[0].range.clone()], "#22");
    assert_eq!(
      link_ranges[0].url.as_ref(),
      "https://github.com/acme/widget/issues/22"
    );
    assert_eq!(&rendered[link_ranges[1].range.clone()], "#33");
    assert_eq!(
      link_ranges[1].url.as_ref(),
      "https://example.com/already-linked"
    );
  }

  #[test]
  fn split_github_mention_segments_detects_user_mentions() {
    let segments = split_github_mention_segments("Ping @test and @foo-bar.");

    assert_eq!(
      segments,
      vec![
        GithubMentionTextSegment::Text("Ping ".to_string()),
        GithubMentionTextSegment::Mention("test".to_string()),
        GithubMentionTextSegment::Text(" and ".to_string()),
        GithubMentionTextSegment::Mention("foo-bar".to_string()),
        GithubMentionTextSegment::Text(".".to_string()),
      ]
    );
  }

  #[test]
  fn split_github_mention_segments_ignores_emails_and_invalid_logins() {
    let segments =
      split_github_mention_segments("Email test@user.com, not @foo_bar or @foo- or @-user.");

    assert_eq!(
      segments,
      vec![GithubMentionTextSegment::Text(
        "Email test@user.com, not @foo_bar or @foo- or @-user.".to_string()
      )]
    );
  }

  #[test]
  fn inline_contains_github_mention_only_checks_plain_text() {
    assert!(inline_contains_github_mention(&Inline::Text(
      "Ping @user".to_string()
    )));
    assert!(!inline_contains_github_mention(&Inline::Code(
      "@user".to_string()
    )));
    assert!(!inline_contains_github_mention(&Inline::Link {
      url: "https://github.com/user".to_string(),
      title: None,
      content: vec![Inline::Text("@user".to_string())],
    }));
  }

  #[test]
  fn parses_html_br_inline_as_hard_break() {
    for br in ["<br>", "<br/>", "<br />", "<BR />"] {
      let source = format!("hello{br}world");
      let blocks = parse_gfm(&source);
      assert_eq!(blocks.len(), 1);
      match &blocks[0] {
        Block::Paragraph(inlines) => {
          assert!(
            inlines
              .iter()
              .any(|inline| matches!(inline, Inline::HardBreak))
          );
          assert_eq!(inline_to_plain_text(inlines), "hello\nworld");
        }
        _ => panic!("expected paragraph"),
      }
    }
  }

  #[test]
  fn parses_html_block_with_multiple_elements_and_br_tags() {
    let source = r#"<p align="center">
TypeScript-first schema validation with static type inference
<br/>
by <a href="https://x.com/colinhacks">@colinhacks</a>
</p>"#;

    let blocks = parse_gfm(source);
    assert_eq!(blocks.len(), 1);
    match &blocks[0] {
      Block::Aligned { center, blocks } => {
        assert!(*center);
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
          Block::Paragraph(inlines) => {
            assert!(
              inlines
                .iter()
                .any(|inline| matches!(inline, Inline::HardBreak))
            );
            assert_eq!(
              inline_to_plain_text(inlines),
              "TypeScript-first schema validation with static type inference\nby @colinhacks"
            );
          }
          _ => panic!("expected paragraph"),
        }
      }
      _ => panic!("expected aligned block"),
    }
  }

  #[test]
  fn parses_html_block_with_multiple_images() {
    let source = r#"<p>
<a href="https://example.com/a"><img src="https://img.shields.io/a.svg" alt="A" /></a>
<a href="https://example.com/b"><img src="https://img.shields.io/b.svg" alt="B" /></a>
</p>"#;

    let blocks = parse_gfm(source);
    assert_eq!(blocks.len(), 1);
    match &blocks[0] {
      Block::Paragraph(inlines) => {
        let linked_images = inlines
          .iter()
          .filter(|inline| {
            matches!(
              inline,
              Inline::Link { content, .. } if content.iter().any(inline_contains_image)
            )
          })
          .count();
        assert_eq!(linked_images, 2);
      }
      _ => panic!("expected paragraph"),
    }
  }

  #[test]
  fn parses_html_badge_row_as_centered_links_with_images() {
    let source = r#"<p align="center">
<a href="https://a.example"><img src="https://img.shields.io/a.svg" alt="A" /></a>
<a href="https://b.example"><img src="https://img.shields.io/b.svg" alt="B" /></a>
<a href="https://c.example"><img src="https://img.shields.io/c.svg" alt="C" /></a>
<a href="https://d.example"><img src="https://img.shields.io/d.svg" alt="D" /></a>
<a href="https://e.example"><img src="https://img.shields.io/e.svg" alt="E" /></a>
</p>"#;

    let blocks = parse_gfm(source);
    assert_eq!(blocks.len(), 1);
    match &blocks[0] {
      Block::Aligned { center, blocks } => {
        assert!(*center);
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
          Block::Paragraph(inlines) => {
            let linked_images = inlines
              .iter()
              .filter(|inline| {
                matches!(
                  inline,
                  Inline::Link { content, .. } if content.iter().any(inline_contains_image)
                )
              })
              .count();
            assert_eq!(linked_images, 5);
          }
          _ => panic!("expected paragraph"),
        }
      }
      _ => panic!("expected aligned block"),
    }
  }

  #[test]
  fn parses_markdown_badge_rows_inside_centered_div() {
    let source = r#"<div align="center">

[![A](https://img.shields.io/a.svg)](https://a.example)
[![B](https://img.shields.io/b.svg)](https://b.example)

</div>"#;

    let blocks = parse_gfm(source);
    assert_eq!(blocks.len(), 1);
    match &blocks[0] {
      Block::Aligned { center, blocks } => {
        assert!(*center);
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
          Block::Paragraph(inlines) => {
            let linked_images = inlines
              .iter()
              .filter(|inline| {
                matches!(
                  inline,
                  Inline::Link { content, .. } if content.iter().any(inline_contains_image)
                )
              })
              .count();
            assert_eq!(linked_images, 2);
          }
          _ => panic!("expected paragraph"),
        }
      }
      _ => panic!("expected aligned block"),
    }
  }

  #[test]
  fn ignores_orphan_centered_div_close_tag() {
    let blocks = parse_gfm("Before\n\n</div>\n\nAfter");
    assert_eq!(blocks.len(), 2);
    assert!(matches!(
      &blocks[0],
      Block::Paragraph(inlines) if inline_to_plain_text(inlines) == "Before"
    ));
    assert!(matches!(
      &blocks[1],
      Block::Paragraph(inlines) if inline_to_plain_text(inlines) == "After"
    ));
  }

  #[test]
  fn parses_centered_picture_with_br_padding() {
    let source = r#"<p align="center">
  <br>
  <br>
  <a href="https://oxc.rs" target="_blank" rel="noopener noreferrer">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://oxc.rs/oxc-light.svg">
      <source media="(prefers-color-scheme: light)" srcset="https://oxc.rs/oxc-dark.svg">
      <img alt="Oxc logo" src="https://oxc.rs/oxc-dark.svg" height="60">
    </picture>
  </a>
  <br>
  <br>
  <br>
</p>"#;

    let blocks = parse_gfm(source);
    assert_eq!(blocks.len(), 1);
    match &blocks[0] {
      Block::Aligned { center, blocks } => {
        assert!(*center);
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
          Block::Paragraph(inlines) => {
            let hard_breaks = inlines
              .iter()
              .filter(|inline| matches!(inline, Inline::HardBreak))
              .count();
            assert_eq!(hard_breaks, 5);

            let image = inlines.iter().find_map(|inline| match inline {
              Inline::Image {
                url,
                alt,
                height,
                dark_url,
                light_url,
                ..
              } => Some((
                url.as_str(),
                alt.as_str(),
                height.as_deref(),
                dark_url.as_deref(),
                light_url.as_deref(),
              )),
              Inline::Link { content, .. } => content.iter().find_map(|child| match child {
                Inline::Image {
                  url,
                  alt,
                  height,
                  dark_url,
                  light_url,
                  ..
                } => Some((
                  url.as_str(),
                  alt.as_str(),
                  height.as_deref(),
                  dark_url.as_deref(),
                  light_url.as_deref(),
                )),
                _ => None,
              }),
              _ => None,
            });

            assert_eq!(
              image,
              Some((
                "https://oxc.rs/oxc-dark.svg",
                "Oxc logo",
                Some("60"),
                Some("https://oxc.rs/oxc-light.svg"),
                Some("https://oxc.rs/oxc-dark.svg"),
              ))
            );
          }
          _ => panic!("expected paragraph"),
        }
      }
      _ => panic!("expected aligned block"),
    }
  }

  #[test]
  fn parses_left_aligned_image_paragraph_without_extra_blocks() {
    let source = r#"To give you an idea of its capabilities, here is an example from the [vscode] repository, which finishes linting 4800+ files in 0.7 seconds:

<p float="left" align="left">
  <img src="https://cdn.jsdelivr.net/gh/oxc-project/oxc-assets/linter-screenshot.png" width="60%">
</p>

→ [oxlint documentation](https://oxc.rs/docs/guide/usage/linter/cli.html)"#;

    let blocks = parse_gfm(source);
    assert_eq!(blocks.len(), 3);

    match &blocks[1] {
      Block::Paragraph(inlines) => {
        assert_eq!(inlines.len(), 1);
        match &inlines[0] {
          Inline::Image {
            url,
            width,
            dark_url,
            light_url,
            ..
          } => {
            assert_eq!(
              url,
              "https://cdn.jsdelivr.net/gh/oxc-project/oxc-assets/linter-screenshot.png"
            );
            assert_eq!(width.as_deref(), Some("60%"));
            assert_eq!(dark_url, &None);
            assert_eq!(light_url, &None);
          }
          _ => panic!("expected image inline"),
        }
      }
      _ => panic!("expected paragraph"),
    }
  }

  #[test]
  fn parses_html_heading_with_center_alignment_wrapper() {
    let source = r#"<h2 align="center">Featured sponsor: Jazz</h2>"#;
    let blocks = parse_gfm(source);
    assert_eq!(blocks.len(), 1);
    match &blocks[0] {
      Block::Aligned { center, blocks } => {
        assert!(*center);
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
          Block::Heading { level, content } => {
            assert_eq!(*level, 2);
            assert_eq!(inline_to_plain_text(content), "Featured sponsor: Jazz");
          }
          _ => panic!("expected heading"),
        }
      }
      _ => panic!("expected aligned block"),
    }
  }

  #[test]
  fn parses_html_picture_uses_img_descendant() {
    let source = r#"<div>
  <picture width="85%">
    <source media="(prefers-color-scheme: dark)" srcset="https://example.com/dark.png">
    <source media="(prefers-color-scheme: light)" srcset="https://example.com/light.png">
    <img alt="jazz logo" src="https://example.com/fallback.png" width="85%">
  </picture>
</div>"#;

    let blocks = parse_gfm(source);
    assert_eq!(blocks.len(), 1);
    match &blocks[0] {
      Block::Paragraph(inlines) => {
        let image = inlines.iter().find_map(|inline| match inline {
          Inline::Image {
            url,
            alt,
            width,
            dark_url,
            light_url,
            ..
          } => Some((
            url.as_str(),
            alt.as_str(),
            width.as_deref(),
            dark_url.as_deref(),
            light_url.as_deref(),
          )),
          _ => None,
        });
        assert_eq!(
          image,
          Some((
            "https://example.com/fallback.png",
            "jazz logo",
            Some("85%"),
            Some("https://example.com/dark.png"),
            Some("https://example.com/light.png"),
          ))
        );
      }
      _ => panic!("expected paragraph"),
    }
  }

  #[test]
  fn parses_unknown_html_tags_using_children_text_fallback() {
    let source = r#"<custom-tag>Hello <strong-tag>world</strong-tag></custom-tag>"#;
    let blocks = parse_gfm(source);
    assert_eq!(blocks.len(), 1);
    match &blocks[0] {
      Block::Paragraph(inlines) => {
        assert_eq!(inline_to_plain_text(inlines), "Hello world");
      }
      _ => panic!("expected paragraph"),
    }
  }

  #[test]
  fn split_inlines_by_hard_breaks_creates_rows() {
    let rows = split_inlines_by_hard_breaks(&[
      Inline::Text("a".to_string()),
      Inline::HardBreak,
      Inline::Text("b".to_string()),
      Inline::HardBreak,
      Inline::Text("c".to_string()),
    ]);
    assert_eq!(rows.len(), 3);
    assert_eq!(inline_to_plain_text(&rows[0]), "a");
    assert_eq!(inline_to_plain_text(&rows[1]), "b");
    assert_eq!(inline_to_plain_text(&rows[2]), "c");
  }

  #[test]
  fn inline_image_data_keeps_parent_link_context() {
    let inline = Inline::Link {
      url: "https://example.com".to_string(),
      title: None,
      content: vec![Inline::Image {
        url: "https://img.shields.io/example.svg".to_string(),
        title: None,
        alt: "badge".to_string(),
        width: Some("120px".to_string()),
        height: None,
        dark_url: Some("https://img.shields.io/example-dark.svg".to_string()),
        light_url: Some("https://img.shields.io/example-light.svg".to_string()),
      }],
    };

    let image = inline_image_data(&inline);
    assert_eq!(
      image,
      Some(InlineImageData {
        url: "https://img.shields.io/example.svg".to_string(),
        alt: "badge".to_string(),
        link_url: Some("https://example.com".to_string()),
        width_hint: Some("120px".to_string()),
        height_hint: None,
        dark_url: Some("https://img.shields.io/example-dark.svg".to_string()),
        light_url: Some("https://img.shields.io/example-light.svg".to_string()),
      })
    );
  }

  #[test]
  fn single_inline_image_data_only_matches_single_image_rows() {
    let single = vec![Inline::Image {
      url: "https://img.shields.io/a.svg".to_string(),
      title: None,
      alt: "A".to_string(),
      width: None,
      height: None,
      dark_url: None,
      light_url: None,
    }];
    assert!(single_inline_image_data(&single).is_some());

    let multiple = vec![
      Inline::Image {
        url: "https://img.shields.io/a.svg".to_string(),
        title: None,
        alt: "A".to_string(),
        width: None,
        height: None,
        dark_url: None,
        light_url: None,
      },
      Inline::Image {
        url: "https://img.shields.io/b.svg".to_string(),
        title: None,
        alt: "B".to_string(),
        width: None,
        height: None,
        dark_url: None,
        light_url: None,
      },
    ];
    assert!(single_inline_image_data(&multiple).is_none());
  }

  #[test]
  fn select_markdown_image_url_for_theme_prefers_dark_variant() {
    let selected = select_markdown_image_url_for_theme(
      "https://example.com/fallback.png",
      Some("https://example.com/dark.png"),
      Some("https://example.com/light.png"),
      true,
    );
    assert_eq!(selected, "https://example.com/dark.png");
  }

  #[test]
  fn select_markdown_image_url_for_theme_prefers_light_variant() {
    let selected = select_markdown_image_url_for_theme(
      "https://example.com/fallback.png",
      Some("https://example.com/dark.png"),
      Some("https://example.com/light.png"),
      false,
    );
    assert_eq!(selected, "https://example.com/light.png");
  }

  #[test]
  fn parse_github_blob_line_reference_parses_standard_blob_link() {
    let parsed = parse_github_blob_line_reference(
      "https://github.com/joris-gallot/guit/blob/0a25a8d0816a770ec75edb442dc3e533c78343a3/docker-compose.yml#L11",
    )
    .expect("valid blob line reference");

    assert_eq!(parsed.owner, "joris-gallot");
    assert_eq!(parsed.repo, "guit");
    assert_eq!(parsed.reference, "0a25a8d0816a770ec75edb442dc3e533c78343a3");
    assert_eq!(parsed.path, "docker-compose.yml");
    assert_eq!(parsed.start_line, 11);
    assert_eq!(parsed.end_line, 11);
  }

  #[test]
  fn parse_github_blob_line_reference_parses_line_range_variants() {
    let parsed = parse_github_blob_line_reference(
      "https://github.com/joris-gallot/guit/blob/main/docker-compose.yml#L03-L11",
    )
    .expect("valid blob line range reference");
    assert_eq!(parsed.start_line, 3);
    assert_eq!(parsed.end_line, 11);

    let parsed = parse_github_blob_line_reference(
      "https://github.com/joris-gallot/guit/blob/main/docker-compose.yml#L3-L11",
    )
    .expect("valid blob line range reference");
    assert_eq!(parsed.start_line, 3);
    assert_eq!(parsed.end_line, 11);

    let parsed = parse_github_blob_line_reference(
      "https://github.com/joris-gallot/guit/blob/main/docker-compose.yml#L3-11",
    )
    .expect("valid blob line range reference");
    assert_eq!(parsed.start_line, 3);
    assert_eq!(parsed.end_line, 11);
  }

  #[test]
  fn parse_github_blob_line_reference_rejects_invalid_inputs() {
    assert!(
      parse_github_blob_line_reference("https://example.com/repo/blob/main/file.rs#L7").is_none()
    );
    assert!(
      parse_github_blob_line_reference("https://github.com/acme/widget/blob/main/file.rs")
        .is_none()
    );
    assert!(
      parse_github_blob_line_reference("https://github.com/acme/widget/blob/main/file.rs#L0")
        .is_none()
    );
    assert!(
      parse_github_blob_line_reference("https://github.com/acme/widget/blob/main/file.rs#L7-L0")
        .is_none()
    );
  }

  #[test]
  fn extract_github_blob_line_references_reads_markdown_link_syntax() {
    let body = "[compose](https://github.com/acme/widget/blob/main/docker-compose.yml#L7)";
    let references = extract_github_blob_line_references(body);
    assert_eq!(references.len(), 1);
    assert_eq!(references[0].start_line, 7);
    assert_eq!(references[0].end_line, 7);
    assert_eq!(references[0].path, "docker-compose.yml");
  }

  #[test]
  fn split_markdown_preview_segments_replaces_standalone_raw_url_line() {
    let url = "https://github.com/acme/widget/blob/main/docker-compose.yml#L7-L9";
    let source = format!("Before\n{url}\nAfter");
    let previews = test_preview_map(url);

    let segments = split_markdown_preview_segments(&source, &previews);
    assert_eq!(segments.len(), 3);
    assert!(
      matches!(&segments[0], MarkdownRenderSegment::Markdown(markdown) if markdown == "Before\n")
    );
    assert!(
      matches!(&segments[1], MarkdownRenderSegment::Preview(preview) if preview.url.as_ref() == url)
    );
    assert!(
      matches!(&segments[2], MarkdownRenderSegment::Markdown(markdown) if markdown == "After")
    );
  }

  #[test]
  fn split_markdown_preview_segments_replaces_standalone_markdown_link_line() {
    let url = "https://github.com/acme/widget/blob/main/docker-compose.yml#L7-L9";
    let source = format!("Before\n[compose]({url})\nAfter");
    let previews = test_preview_map(url);

    let segments = split_markdown_preview_segments(&source, &previews);
    assert_eq!(segments.len(), 3);
    assert!(
      matches!(&segments[1], MarkdownRenderSegment::Preview(preview) if preview.url.as_ref() == url)
    );
  }

  #[test]
  fn split_markdown_preview_segments_keeps_inline_link_as_markdown() {
    let url = "https://github.com/acme/widget/blob/main/docker-compose.yml#L7-L9";
    let source = format!("Inline link {url} should stay markdown");
    let previews = test_preview_map(url);

    let segments = split_markdown_preview_segments(&source, &previews);
    assert_eq!(segments, vec![MarkdownRenderSegment::Markdown(source)]);
  }

  #[test]
  fn parses_lists() {
    let blocks = parse_gfm("- a\n- b");
    assert_eq!(blocks.len(), 1);
    match &blocks[0] {
      Block::List(list) => {
        assert!(!list.ordered);
        assert_eq!(list.items.len(), 2);
      }
      _ => panic!("expected list"),
    }
  }

  #[test]
  fn parses_code_blocks() {
    let blocks = parse_gfm("```rust\nfn main() {}\n```");
    assert_eq!(blocks.len(), 1);
    match &blocks[0] {
      Block::CodeBlock(code) => {
        assert_eq!(code.lang.as_deref(), Some("rust"));
        assert!(code.value.contains("fn main"));
      }
      _ => panic!("expected code block"),
    }
  }

  #[test]
  fn parses_blockquote() {
    let blocks = parse_gfm("> hello");
    assert_eq!(blocks.len(), 1);
    match &blocks[0] {
      Block::BlockQuote(children) => {
        assert!(!children.is_empty());
      }
      _ => panic!("expected blockquote"),
    }
  }

  #[test]
  fn detects_github_alert_note() {
    let blocks = parse_gfm("> [!NOTE]\n> This is a note.");
    assert_eq!(blocks.len(), 1);
    let Block::BlockQuote(children) = &blocks[0] else {
      panic!("expected blockquote");
    };
    let alert = detect_github_alert(children);
    assert!(alert.is_some());
    let (kind, remaining) = alert.unwrap();
    assert_eq!(kind, GithubAlertKind::Note);
    assert!(remaining.trim().is_empty() || remaining.trim() == "");
  }

  #[test]
  fn detects_all_github_alert_kinds() {
    for (syntax, expected) in [
      ("[!NOTE]", GithubAlertKind::Note),
      ("[!TIP]", GithubAlertKind::Tip),
      ("[!IMPORTANT]", GithubAlertKind::Important),
      ("[!WARNING]", GithubAlertKind::Warning),
      ("[!CAUTION]", GithubAlertKind::Caution),
    ] {
      let source = format!("> {syntax}\n> Content here.");
      let blocks = parse_gfm(&source);
      let Block::BlockQuote(children) = &blocks[0] else {
        panic!("expected blockquote for {syntax}");
      };
      let (kind, _) = detect_github_alert(children)
        .unwrap_or_else(|| panic!("expected alert detection for {syntax}"));
      assert_eq!(kind, expected, "mismatch for {syntax}");
    }
  }

  #[test]
  fn regular_blockquote_not_detected_as_alert() {
    let blocks = parse_gfm("> Just a normal quote");
    let Block::BlockQuote(children) = &blocks[0] else {
      panic!("expected blockquote");
    };
    assert!(detect_github_alert(children).is_none());
  }

  #[test]
  fn parses_table() {
    let blocks = parse_gfm("| a | b |\n| - | - |\n| c | d |");
    assert_eq!(blocks.len(), 1);
    match &blocks[0] {
      Block::Table(table) => {
        assert_eq!(table.headers.len(), 2);
        assert_eq!(table.rows.len(), 1);
      }
      _ => panic!("expected table"),
    }
  }

  #[test]
  fn parses_table_followed_by_paragraph_without_empty_block() {
    let source = r#"## 📦 Tools & Packages

| Tool        | npm                                                     | crates.io                                                   |
| ----------- | ------------------------------------------------------- | ----------------------------------------------------------- |
| Linter      | [oxlint](https://npmx.dev/package/oxlint)               | -                                                           |
| Formatter   | [oxfmt](https://npmx.dev/package/oxfmt)                 | -                                                           |
| Parser      | [oxc-parser](https://npmx.dev/package/oxc-parser)       | [oxc_parser](https://crates.io/crates/oxc_parser)           |
| Transformer | [oxc-transform](https://npmx.dev/package/oxc-transform) | [oxc_transformer](https://crates.io/crates/oxc_transformer) |
| Minifier    | [oxc-minify](https://npmx.dev/package/oxc-minify)       | [oxc_minifier](https://crates.io/crates/oxc_minifier)       |
| Resolver    | [oxc-resolver](https://npmx.dev/package/oxc-resolver)   | [oxc_resolver](https://crates.io/crates/oxc_resolver)       |

See [documentation](https://oxc.rs/) for detailed usage guides for each tool."#;

    let blocks = parse_gfm(source);
    assert_eq!(blocks.len(), 3);

    match &blocks[0] {
      Block::Heading { level, content } => {
        assert_eq!(*level, 2);
        assert_eq!(inline_to_plain_text(content), "📦 Tools & Packages");
      }
      _ => panic!("expected heading"),
    }

    match &blocks[1] {
      Block::Table(table) => {
        assert_eq!(table.headers.len(), 3);
        assert_eq!(table.rows.len(), 6);
      }
      _ => panic!("expected table"),
    }

    match &blocks[2] {
      Block::Paragraph(inlines) => {
        assert_eq!(
          inline_to_plain_text(inlines),
          "See documentation for detailed usage guides for each tool."
        );
      }
      _ => panic!("expected paragraph"),
    }
  }

  #[test]
  fn parses_table_with_image_cells() {
    let blocks = parse_gfm("| Age |\n| --- |\n| ![age](https://example.com/age.svg) |");
    assert_eq!(blocks.len(), 1);
    match &blocks[0] {
      Block::Table(table) => {
        assert_eq!(table.headers.len(), 1);
        assert_eq!(table.rows.len(), 1);
        assert_eq!(table.rows[0].len(), 1);
        match &table.rows[0][0][0] {
          Inline::Image { url, alt, .. } => {
            assert_eq!(url, "https://example.com/age.svg");
            assert_eq!(alt, "age");
          }
          _ => panic!("expected image inline"),
        }
      }
      _ => panic!("expected table"),
    }
  }

  #[test]
  fn parses_table_with_html_image_cells() {
    let blocks =
      parse_gfm("| Age |\n| --- |\n| <img src=\"https://example.com/age.svg\" alt=\"age\" /> |");
    assert_eq!(blocks.len(), 1);
    match &blocks[0] {
      Block::Table(table) => {
        assert_eq!(table.rows.len(), 1);
        assert_eq!(table.rows[0].len(), 1);
        match &table.rows[0][0][0] {
          Inline::Image { url, alt, .. } => {
            assert_eq!(url, "https://example.com/age.svg");
            assert_eq!(alt, "age");
          }
          _ => panic!("expected image inline"),
        }
      }
      _ => panic!("expected table"),
    }
  }

  #[test]
  fn parses_text_followed_by_html_img_keeps_text_and_image() {
    let source = "Tilt up (manual sync) on e2e-nsp and run the tests:\n<img width=\"346\" height=\"341\" alt=\"image\" src=\"https://github.com/user-attachments/assets/558c25e0-68bd-4c1e-84c7-49863c99c532\" />";
    let blocks = parse_gfm(source);
    assert_eq!(blocks.len(), 1);
    match &blocks[0] {
      Block::Paragraph(inlines) => {
        assert!(inlines.len() >= 2);
        assert!(
          matches!(&inlines[0], Inline::Text(t) if t.contains("Tilt up")),
          "first inline should be the text"
        );
        assert!(
          inlines.iter().any(|i| matches!(i, Inline::Image { .. })),
          "should contain an Image inline"
        );
      }
      _ => panic!("expected paragraph"),
    }
  }

  #[test]
  fn parses_html_block_image_as_inline_image_paragraph() {
    let blocks = parse_gfm(
      "<img width=\"1159\" height=\"272\" alt=\"Image\" src=\"https://github.com/user-attachments/assets/525e1fe3-1159-47ea-a1ac-8926a03c9cd1\" />",
    );
    assert_eq!(blocks.len(), 1);
    match &blocks[0] {
      Block::Paragraph(inlines) => {
        assert_eq!(inlines.len(), 1);
        match &inlines[0] {
          Inline::Image { url, alt, .. } => {
            assert_eq!(
              url,
              "https://github.com/user-attachments/assets/525e1fe3-1159-47ea-a1ac-8926a03c9cd1"
            );
            assert_eq!(alt, "Image");
          }
          _ => panic!("expected image inline"),
        }
      }
      _ => panic!("expected paragraph"),
    }
  }

  #[test]
  fn is_github_user_attachment_url_matches_valid_urls() {
    assert!(is_github_user_attachment_url(
      "https://github.com/user-attachments/assets/2d42dac5-357f-45d0-a4cb-a4ebd849304b"
    ));
    assert!(!is_github_user_attachment_url(
      "https://github.com/octocat/repo"
    ));
    assert!(!is_github_user_attachment_url(
      "https://example.com/user-attachments/assets/abc"
    ));
  }

  #[test]
  fn bare_user_attachment_link_detected_as_image() {
    let url = "https://github.com/user-attachments/assets/4aa12d28-968a-490d-81ee-32bbbb595fc4";
    let blocks = parse_gfm(url);
    assert_eq!(blocks.len(), 1);
    match &blocks[0] {
      Block::Paragraph(inlines) => {
        // comrak autolink may produce the link as the only inline (possibly with
        // a trailing soft break). Find the Link inline.
        let link = inlines
          .iter()
          .find(|i| matches!(i, Inline::Link { .. }))
          .expect("should contain a Link inline");
        assert!(
          inline_contains_image(link),
          "bare user-attachment link should be treated as image-containing"
        );
        let data = inline_image_data(link).expect("should produce InlineImageData");
        assert_eq!(data.url, url);
        assert_eq!(data.link_url, Some(url.to_string()));
      }
      _ => panic!("expected paragraph"),
    }
  }

  #[test]
  fn table_column_widths_use_largest_cell_per_column() {
    let table = Table {
      headers: vec![
        vec![Inline::Text("Package".to_string())],
        vec![Inline::Text("Type".to_string())],
      ],
      rows: vec![
        vec![
          vec![Inline::Text("tauri-build (source)".to_string())],
          vec![Inline::Text("minor".to_string())],
        ],
        vec![
          vec![Inline::Text("@tauri-apps/api".to_string())],
          vec![Inline::Text("build-dependencies".to_string())],
        ],
      ],
    };

    let widths = table_column_widths(&table, 2);
    assert_eq!(widths.len(), 2);
    assert!(widths[0] >= table_cell_width(&[Inline::Text("tauri-build (source)".to_string())]));
    assert!(widths[1] >= table_cell_width(&[Inline::Text("build-dependencies".to_string())]));
  }

  #[test]
  fn parses_details_block() {
    let source = r#"<details open>
<summary>Summary</summary>

Body **text**
</details>"#;
    let blocks = parse_gfm(source);
    assert_eq!(blocks.len(), 1);
    match &blocks[0] {
      Block::Details(details) => {
        assert!(details.open);
        assert_eq!(inline_to_plain_text(&details.summary), "Summary");
        assert!(!details.blocks.is_empty());
      }
      _ => panic!("expected details"),
    }
  }

  #[test]
  fn parses_details_summary_with_html_link() {
    let source = r#"<details>
<summary><a href="https://github.com/badlogic/pi-mono">badlogic/pi-mono</a></summary>

Body content
</details>"#;
    let blocks = parse_gfm(source);
    assert_eq!(blocks.len(), 1);
    match &blocks[0] {
      Block::Details(details) => {
        assert!(!details.open);
        assert_eq!(details.summary.len(), 1);
        match &details.summary[0] {
          Inline::Link { url, content, .. } => {
            assert_eq!(url, "https://github.com/badlogic/pi-mono");
            assert_eq!(inline_to_plain_text(content), "badlogic/pi-mono");
          }
          other => panic!("expected link inline, got {:?}", other),
        }
      }
      _ => panic!("expected details"),
    }
  }

  #[test]
  fn parses_sibling_nested_details_blocks() {
    let source = r#"Release Notes

<details open>
<summary>tauri-apps/tauri (@tauri-apps/api)</summary>

[v2.10.1](https://github.com/tauri-apps/tauri/releases/tag/tauri-v2.10.1): tauri v2.10.1

[Compare Source](https://github.com/tauri-apps/tauri/compare/tauri-v2.10.0...tauri-v2.10.1)

<details>
<summary><em><h4>Cargo Audit</h4></em></summary>

[2.10.1]

Dependencies

- [ce8fddb46](https://github.com/tauri-apps/tauri/commit/ce8fddb46) ([#14873](https://github.com/tauri-apps/tauri/pull/14873)) Unlocked version range for webkit2gtk-rs dependency.

</details>
<details>
<summary><em><h4>Cargo Publish</h4></em></summary>

```text
Updating crates.io index
```

</details>
</details>"#;

    let blocks = parse_gfm(source);
    assert_eq!(blocks.len(), 2);

    let parent_details = match &blocks[1] {
      Block::Details(details) => details,
      _ => panic!("expected top-level details"),
    };

    let nested: Vec<&Details> = parent_details
      .blocks
      .iter()
      .filter_map(|block| match block {
        Block::Details(details) => Some(details),
        _ => None,
      })
      .collect();
    assert_eq!(nested.len(), 2);
    assert!(
      !nested[0].blocks.is_empty(),
      "first nested details body should stay inside details"
    );
    assert!(
      !nested[1].blocks.is_empty(),
      "second nested details body should stay inside details"
    );

    let non_details_blocks_after_nested = parent_details
      .blocks
      .iter()
      .filter(|block| !matches!(block, Block::Details(_)))
      .count();
    assert!(
      non_details_blocks_after_nested >= 2,
      "expected non-details release notes blocks (links/title)"
    );
  }

  #[test]
  fn parses_unclosed_nested_details_like_github() {
    let source = r#"<details open>
<summary>Root</summary>

<details>
<summary><em><h4>Cargo Audit</h4></em></summary>

- audit body
</details>
<details>
<summary><em><h4>Cargo Publish</h4></em></summary>

```text
publish body
```
</details>"#;

    let blocks = parse_gfm(source);
    assert_eq!(blocks.len(), 1);

    let root = match &blocks[0] {
      Block::Details(details) => details,
      _ => panic!("expected root details"),
    };
    let nested: Vec<&Details> = root
      .blocks
      .iter()
      .filter_map(|block| match block {
        Block::Details(details) => Some(details),
        _ => None,
      })
      .collect();

    assert_eq!(nested.len(), 2);
    assert_eq!(inline_to_plain_text(&nested[0].summary), "Cargo Audit");
    assert_eq!(inline_to_plain_text(&nested[1].summary), "Cargo Publish");
    assert!(!nested[0].blocks.is_empty());
    assert!(!nested[1].blocks.is_empty());
  }

  #[test]
  fn ignores_orphan_details_closing_tag_after_valid_block() {
    let source = r#"<details>
<summary>Summary</summary>

Body
</details>
</details>"#;

    let blocks = parse_gfm(source);
    assert_eq!(blocks.len(), 1);
    match &blocks[0] {
      Block::Details(details) => {
        assert_eq!(inline_to_plain_text(&details.summary), "Summary");
        assert!(!details.blocks.is_empty());
      }
      _ => panic!("expected details"),
    }
  }

  #[test]
  fn ignores_comment_only_html_blocks() {
    let blocks = parse_gfm(
      r#"<!--
Ceci est un commentaire je devrais pas le voir, il faut pas le rendre
-->"#,
    );
    assert!(blocks.is_empty());
  }

  #[test]
  fn ignores_comment_block_between_paragraphs() {
    let blocks = parse_gfm(
      r#"Avant

<!--
Commentaire cache
-->

Apres"#,
    );

    assert_eq!(blocks.len(), 2);
    match &blocks[0] {
      Block::Paragraph(inlines) => assert_eq!(inline_to_plain_text(inlines), "Avant"),
      _ => panic!("expected paragraph"),
    }
    match &blocks[1] {
      Block::Paragraph(inlines) => assert_eq!(inline_to_plain_text(inlines), "Apres"),
      _ => panic!("expected paragraph"),
    }
  }

  #[test]
  fn estimates_markdown_height_grows_with_longer_content() {
    let short = "Short paragraph.";
    let long = "Short paragraph. ".repeat(40);
    let short_height = estimate_markdown_height_px(short, 72, 20.0);
    let long_height = estimate_markdown_height_px(&long, 72, 20.0);
    assert!(long_height > short_height);
  }

  #[test]
  fn estimates_markdown_height_respects_wrap_columns() {
    let source = "This is a long markdown line that should wrap to more visual lines when the available width is smaller.";
    let wide = estimate_markdown_height_px(source, 96, 20.0);
    let narrow = estimate_markdown_height_px(source, 32, 20.0);
    assert!(narrow > wide);
  }

  #[test]
  fn estimate_blocks_height_adds_extra_spacing_before_headings() {
    let blocks = vec![
      Block::Paragraph(vec![Inline::Text("Alpha".to_string())]),
      Block::Heading {
        level: 2,
        content: vec![Inline::Text("Beta".to_string())],
      },
    ];

    let height = estimate_blocks_height_px(&blocks, 80, 20.0, 0, None);
    let expected = 20.0 + MARKDOWN_BASE_BLOCK_GAP_PX + MARKDOWN_HEADING_EXTRA_TOP_MARGIN_PX + 24.0;
    assert_eq!(height, expected);
  }

  #[test]
  fn estimate_github_code_reference_preview_height_grows_with_more_lines() {
    let single = estimate_github_code_reference_preview_height_px(1, 20.0);
    let many = estimate_github_code_reference_preview_height_px(12, 20.0);
    assert!(many > single);
  }

  #[test]
  fn estimate_github_code_reference_preview_height_caps_scroll_content() {
    let capped = estimate_github_code_reference_preview_height_px(1_000, 20.0);
    let expected = 20.0 * 2.0
      + MARKDOWN_CODE_REFERENCE_CARD_PADDING_Y_PX * 4.0
      + MARKDOWN_CODE_REFERENCE_CARD_MARGIN_Y_PX * 2.0
      + 3.0
      + MARKDOWN_CODE_BLOCK_MAX_HEIGHT_PX;
    assert_eq!(capped, expected);
  }

  #[test]
  fn estimate_code_reference_preview_min_content_width_px_uses_widest_row() {
    let mut preview =
      test_preview_for_url("https://github.com/acme/widget/blob/main/docker-compose.yml#L7-L9");
    preview.start_line = 7;
    preview.snippets = vec![Arc::from("abc"), Arc::from("abcdefgh")];

    let width = estimate_code_reference_preview_min_content_width_px(&preview);
    let expected_columns = 1 + 2 + 8;
    let expected = (expected_columns as f32 * MARKDOWN_CODE_BLOCK_APPROX_CHAR_WIDTH_PX
      + MARKDOWN_CODE_REFERENCE_ROW_GAP_PX)
      .ceil();

    assert_eq!(width, expected);
  }

  #[test]
  fn estimate_code_reference_preview_min_content_width_px_handles_empty_snippets() {
    let mut preview =
      test_preview_for_url("https://github.com/acme/widget/blob/main/docker-compose.yml#L7-L9");
    preview.start_line = 1234;
    preview.snippets.clear();

    let width = estimate_code_reference_preview_min_content_width_px(&preview);
    let expected =
      (4.0 * MARKDOWN_CODE_BLOCK_APPROX_CHAR_WIDTH_PX + MARKDOWN_CODE_REFERENCE_ROW_GAP_PX).ceil();

    assert_eq!(width, expected);
  }

  #[test]
  fn normalizes_code_block_display_value_trailing_newline() {
    let rust = CodeBlock {
      lang: Some("rust".to_string()),
      value: "fn main() {}\n".to_string(),
    };
    let text = CodeBlock {
      lang: Some("text".to_string()),
      value: "line\r\n".to_string(),
    };
    let multiline = CodeBlock {
      lang: Some("text".to_string()),
      value: "line\n\n".to_string(),
    };

    assert_eq!(code_block_display_value(&rust), "fn main() {}");
    assert_eq!(code_block_display_value(&text), "line");
    assert_eq!(code_block_display_value(&multiline), "line\n");
  }

  #[test]
  fn code_block_copy_value_keeps_original_source() {
    let code = CodeBlock {
      lang: Some("text".to_string()),
      value: "\tline one\n\n</details>\n".to_string(),
    };

    assert_eq!(
      code_block_copy_value(&code).as_ref(),
      "\tline one\n\n</details>\n"
    );
  }

  #[test]
  fn code_block_hover_group_id_is_stable_and_unique() {
    assert_eq!(
      code_block_hover_group_id(42).as_ref(),
      "markdown-code-block-hover-42"
    );
    assert_ne!(code_block_hover_group_id(1), code_block_hover_group_id(2));
  }

  #[test]
  fn code_block_selectable_text_options_match_gfm_preview_behavior() {
    let options = code_block_selectable_text_options();

    assert!(options.interactive);
    assert!(options.show_indentation_dots);
    assert!(!options.show_inline_code_backgrounds);
  }

  #[test]
  fn code_block_display_value_preserves_box_drawing_table_spacing() {
    let table = concat!(
      "┌──────────────────────────────────────┬──────────────┬───────────────────────────┐\n",
      "│                                      │ Figma Plugin │ Manual MCP server config  │\n",
      "├──────────────────────────────────────┼──────────────┼───────────────────────────┤\n",
      "│ MCP tools (get_design_context, etc.) │ Included     │ You configure it yourself │\n",
      "├──────────────────────────────────────┼──────────────┼───────────────────────────┤\n",
      "│ Skills (/implement-design, etc.)     │ Included     │ Not available             │\n",
      "├──────────────────────────────────────┼──────────────┼───────────────────────────┤\n",
      "│ Steering / best practices            │ Included     │ Not available             │\n",
      "└──────────────────────────────────────┴──────────────┴───────────────────────────┘",
    );
    let code = CodeBlock {
      lang: None,
      value: format!("{table}\n"),
    };

    assert_eq!(code_block_display_value(&code), table);
  }

  #[test]
  fn expands_tabs_for_code_block_for_consistent_indentation() {
    let code = CodeBlock {
      lang: Some("rust".to_string()),
      value: "\tfn main() {\n\t\tprintln!(\"hi\");\n\t}\n".to_string(),
    };

    assert_eq!(
      code_block_display_value(&code),
      "    fn main() {\n        println!(\"hi\");\n    }"
    );
  }

  #[test]
  fn expands_leading_spaces_for_code_block_for_tab_like_indentation() {
    let code = CodeBlock {
      lang: Some("rust".to_string()),
      value: "  fn main() {\n    println!(\"ok\");\n  }\n".to_string(),
    };

    assert_eq!(
      code_block_display_value(&code),
      "  fn main() {\n    println!(\"ok\");\n  }"
    );
  }

  #[test]
  fn expands_only_leading_spaces_for_code_block() {
    let code = CodeBlock {
      lang: Some("rust".to_string()),
      value: "  let x = 1 + 2;\nvalue . split_whitespace();\n".to_string(),
    };

    assert_eq!(
      code_block_display_value(&code),
      "  let x = 1 + 2;\nvalue . split_whitespace();"
    );
  }

  #[test]
  fn strips_orphan_details_closing_line_for_plain_text_code_blocks() {
    let code = CodeBlock {
      lang: Some("text".to_string()),
      value: "Downloaded crate-a\nDownloaded crate-b\n\n</details>\n".to_string(),
    };

    assert_eq!(
      code_block_display_value(&code),
      "Downloaded crate-a\nDownloaded crate-b"
    );
  }

  #[test]
  fn keeps_details_closing_line_for_non_plain_text_code_blocks() {
    let code = CodeBlock {
      lang: Some("html".to_string()),
      value: "<details>\n</details>\n".to_string(),
    };

    assert_eq!(code_block_display_value(&code), "<details>\n</details>");
  }

  #[test]
  fn resolves_code_block_language_from_fence_name_or_extension() {
    let rust = code_block_language_config(Some("rust")).expect("rust language");
    assert_eq!(rust.name, "rust");

    let rs = code_block_language_config(Some("rs")).expect("rs extension");
    assert_eq!(rs.name, "rust");

    let dotted = code_block_language_config(Some(".py")).expect(".py extension");
    assert_eq!(dotted.name, "python");
  }

  #[test]
  fn code_block_language_hint_from_path_prefers_extension() {
    assert_eq!(
      code_block_language_hint_from_path("src/lib.rs").as_deref(),
      Some("rs")
    );
    assert_eq!(
      code_block_language_hint_from_path("Dockerfile").as_deref(),
      Some("Dockerfile")
    );
    assert_eq!(code_block_language_hint_from_path(""), None);
  }

  #[test]
  fn builds_syntax_highlighted_spans_for_rust_code_blocks() {
    let code = CodeBlock {
      lang: Some("rust".to_string()),
      value: "fn main() {}\n".to_string(),
    };
    let (_, spans, _) = build_code_block_spans(&code, None);

    assert!(!spans.is_empty());
    assert!(spans.iter().all(|span| span.style.code));
    assert!(
      spans
        .iter()
        .any(|span| span.syntax_token == Some(TokenType::Keyword))
    );
  }

  #[test]
  fn estimate_code_block_min_content_width_px_uses_widest_line() {
    let shorter = estimate_code_block_min_content_width_px("abc\ndef");
    let wider = estimate_code_block_min_content_width_px("abc\ndefghijkl");

    assert!(wider > shorter);
  }

  #[test]
  fn estimate_code_block_min_content_width_px_keeps_code_block_chrome_width() {
    let width = estimate_code_block_min_content_width_px("");
    let expected =
      (MARKDOWN_CODE_BLOCK_PADDING_X_PX * 2.0 + MARKDOWN_CODE_BLOCK_TEXT_SHIFT_X_PX).ceil();

    assert_eq!(width, expected);
  }

  #[test]
  fn selection_for_text_clamps_non_char_boundary_indices() {
    let state = MarkdownRenderState::new();
    let text = SharedString::from("✅ **Branches parallèles** (API + notifications)");

    update_selection_state(&state, 42, 1, text.len(), false);
    let selection = selection_for_text(&state, 42, &text).expect("selection should exist");

    assert_eq!(selection.start, 0);
    assert!(text.is_char_boundary(selection.start));
    assert!(text.is_char_boundary(selection.end));
  }

  #[test]
  fn collect_indentation_dot_indices_marks_leading_spaces_only() {
    let text = "    first\n  second";
    let indices = collect_indentation_indicators(text).dot_indices;

    assert_eq!(indices, vec![0, 1, 2, 3, 10, 11]);
  }

  #[test]
  fn collect_indentation_dot_indices_ignores_internal_spaces() {
    let text = "  first second third";
    let indices = collect_indentation_indicators(text).dot_indices;

    assert_eq!(indices, vec![0, 1]);
  }

  #[test]
  fn collect_indentation_dot_indices_ignores_blank_or_whitespace_only_lines() {
    let text = "  \n  valid\n    \n end";
    let indices = collect_indentation_indicators(text).dot_indices;

    assert_eq!(indices, vec![3, 4, 16]);
  }

  #[test]
  fn collect_indentation_dot_indices_handles_crlf() {
    let text = "  a\r\n    b\r\n  ";
    let indices = collect_indentation_indicators(text).dot_indices;

    assert_eq!(indices, vec![0, 1, 5, 6, 7, 8]);
  }

  #[test]
  fn collect_indentation_dot_indices_limits_render_count_for_large_input() {
    let text = (0..200)
      .map(|_| "                              line")
      .collect::<Vec<_>>()
      .join("\n");
    let indices = collect_indentation_indicators(text.as_str()).dot_indices;

    assert!(!indices.is_empty());
    assert!(indices.len() <= MARKDOWN_CODE_INDENT_DOT_MAX_RENDER_COUNT);
  }

  #[test]
  fn collect_indentation_dot_indices_disables_for_very_large_text() {
    let text = format!(
      "{}code",
      " ".repeat(MARKDOWN_CODE_INDENT_DOT_DISABLE_ABOVE_TEXT_LEN + 1)
    );
    let indices = collect_indentation_indicators(text.as_str()).dot_indices;

    assert!(indices.is_empty());
  }

  #[test]
  fn collect_indentation_indicators_collects_tab_indices() {
    let text = "\t\tfirst\n\tsecond";
    let indicators = collect_indentation_indicators(text);

    assert!(indicators.dot_indices.is_empty());
    assert_eq!(indicators.tab_indices, vec![0, 1, 8]);
  }

  #[test]
  fn collect_indentation_indicators_collects_mixed_spaces_and_tabs() {
    let text = "  \tfirst\n\t  second";
    let indicators = collect_indentation_indicators(text);

    assert_eq!(indicators.dot_indices, vec![0, 1, 10, 11]);
    assert_eq!(indicators.tab_indices, vec![2, 9]);
  }

  #[test]
  fn parsed_markdown_cache_returns_cached_entry_for_same_source() {
    let mut cache = ParsedMarkdownCache::default();
    let source: Arc<str> = Arc::from("**hello**");
    let parsed = parse_markdown(source.as_ref());
    let original_ptr = Arc::as_ptr(&parsed.blocks);

    cache.insert(source.clone(), parsed);
    let cached = cache
      .get(source.as_ref())
      .expect("cached markdown should be present");

    assert_eq!(Arc::as_ptr(&cached.blocks), original_ptr);
  }

  #[test]
  fn parsed_markdown_cache_evicts_oldest_entry_when_full() {
    let mut cache = ParsedMarkdownCache::default();
    for ix in 0..=PARSED_MARKDOWN_CACHE_MAX_ENTRIES {
      let source = format!("source-{ix}");
      cache.insert(Arc::from(source.as_str()), parse_markdown(source.as_str()));
    }

    assert!(cache.len() <= PARSED_MARKDOWN_CACHE_MAX_ENTRIES);
    assert!(cache.get("source-0").is_none());
    assert!(
      cache
        .get(format!("source-{PARSED_MARKDOWN_CACHE_MAX_ENTRIES}").as_str())
        .is_some()
    );
  }

  #[test]
  fn parsed_markdown_cache_get_refreshes_lru_order() {
    let mut cache = ParsedMarkdownCache::default();
    for ix in 0..PARSED_MARKDOWN_CACHE_MAX_ENTRIES {
      let source = format!("source-{ix}");
      cache.insert(Arc::from(source.as_str()), parse_markdown(source.as_str()));
    }

    let first_key = "source-0";
    assert!(cache.get(first_key).is_some());

    let overflow_source = format!("source-{}", PARSED_MARKDOWN_CACHE_MAX_ENTRIES);
    cache.insert(
      Arc::from(overflow_source.as_str()),
      parse_markdown(overflow_source.as_str()),
    );

    assert!(cache.get(first_key).is_some());
    assert!(cache.get("source-1").is_none());
  }

  #[test]
  fn selection_text_keeps_raw_spaces_without_visual_markers() {
    let state = MarkdownRenderState::new();
    let text = SharedString::from("    let x = 1;");
    update_selection_state(&state, 7, 0, text.len(), false);

    let selected = selection_text(&state, 7, &text).expect("selection should exist");
    assert_eq!(selected, "    let x = 1;");
  }

  #[test]
  fn render_context_text_ids_are_scoped_and_stable_per_local_index() {
    let mut first = RenderContext::new(42);
    let mut second = RenderContext::new(43);

    let first_id = first.next_text_id();
    let second_id = second.next_text_id();
    let first_next_id = first.next_text_id();

    assert_ne!(first_id, second_id);
    assert_eq!(first_next_id, first_id + 1);
  }

  #[test]
  fn build_spans_sets_code_style_for_inline_code() {
    let inlines = vec![
      Inline::Text("prefix ".to_string()),
      Inline::Code("code_value".to_string()),
      Inline::Text(" suffix".to_string()),
    ];
    let options = MarkdownRenderOptions::default();
    let (text, spans, _) = build_spans(&inlines, &options);

    // Thin spaces (\u{2009}) are inserted around inline code for visual padding
    assert_eq!(text.as_ref(), "prefix \u{2009}code_value\u{2009} suffix");

    let code_spans: Vec<_> = spans.iter().filter(|s| s.style.code).collect();
    assert_eq!(code_spans.len(), 1);
    assert_eq!(&text[code_spans[0].range.clone()], "code_value");
  }

  #[test]
  fn build_spans_code_spans_do_not_overlap_with_text_spans() {
    let inlines = vec![
      Inline::Text("a".to_string()),
      Inline::Code("b".to_string()),
      Inline::Text("c".to_string()),
    ];
    let options = MarkdownRenderOptions::default();
    let (text, spans, _) = build_spans(&inlines, &options);

    assert_eq!(text.as_ref(), "a\u{2009}b\u{2009}c");
    let code_spans: Vec<_> = spans.iter().filter(|s| s.style.code).collect();
    assert_eq!(code_spans.len(), 1);
    assert_eq!(&text[code_spans[0].range.clone()], "b");
  }

  #[test]
  fn build_spans_inline_code_inside_bold_preserves_both_styles() {
    let inlines = vec![Inline::Strong(vec![Inline::Code("bold_code".to_string())])];
    let options = MarkdownRenderOptions::default();
    let (text, spans, _) = build_spans(&inlines, &options);

    assert_eq!(text.as_ref(), "\u{2009}bold_code\u{2009}");
    let code_spans: Vec<_> = spans
      .iter()
      .filter(|s| s.style.code && s.style.bold)
      .collect();
    assert_eq!(code_spans.len(), 1);
    assert_eq!(&text[code_spans[0].range.clone()], "bold_code");
  }

  #[test]
  fn build_spans_adjacent_inline_code_produces_separate_spans() {
    let inlines = vec![
      Inline::Code("first".to_string()),
      Inline::Text(" ".to_string()),
      Inline::Code("second".to_string()),
    ];
    let options = MarkdownRenderOptions::default();
    let (text, spans, _) = build_spans(&inlines, &options);

    assert_eq!(
      text.as_ref(),
      "\u{2009}first\u{2009} \u{2009}second\u{2009}"
    );
    let code_spans: Vec<_> = spans.iter().filter(|s| s.style.code).collect();
    assert_eq!(code_spans.len(), 2);
    assert_eq!(&text[code_spans[0].range.clone()], "first");
    assert_eq!(&text[code_spans[1].range.clone()], "second");
  }

  #[test]
  fn build_spans_softbreak_renders_as_space_by_default() {
    let inlines = vec![
      Inline::Text("line one".to_string()),
      Inline::SoftBreak,
      Inline::Text("line two".to_string()),
    ];
    let options = MarkdownRenderOptions::default();
    let (text, _, _) = build_spans(&inlines, &options);
    assert_eq!(text.as_ref(), "line one line two");
  }

  #[test]
  fn build_spans_softbreak_renders_as_newline_with_hardbreaks() {
    let inlines = vec![
      Inline::Text("line one".to_string()),
      Inline::SoftBreak,
      Inline::Text("line two".to_string()),
    ];
    let options = MarkdownRenderOptions::default().with_hardbreaks();
    let (text, _, _) = build_spans(&inlines, &options);
    assert_eq!(text.as_ref(), "line one\nline two");
  }

  #[test]
  fn build_preview_code_spans_per_line_covers_every_byte() {
    let snippets: Vec<Arc<str>> = vec![
      Arc::from("  container_name: postgres"),
      Arc::from("  image: postgres:17.7"),
      Arc::from("  restart: always"),
    ];
    let result = build_preview_code_spans_per_line(&snippets, Some("yaml"), None, 1);

    assert_eq!(result.len(), 3);
    for (i, (text, spans)) in result.iter().enumerate() {
      assert_eq!(text.as_ref(), snippets[i].as_ref());
      let text_len = text.len();
      // Verify spans cover 0..text_len without gaps
      let mut covered = 0;
      for span in spans {
        assert_eq!(
          span.range.start, covered,
          "gap at byte {} in line {i}: {text:?}",
          covered
        );
        covered = span.range.end;
      }
      assert_eq!(
        covered, text_len,
        "spans don't reach end of line {i}: {text:?}"
      );
    }
  }

  #[test]
  fn build_preview_code_spans_per_line_produces_syntax_tokens_for_typescript() {
    let snippets: Vec<Arc<str>> = vec![
      Arc::from("const x = 42;"),
      Arc::from("let name = \"hello\";"),
    ];
    let result = build_preview_code_spans_per_line(&snippets, Some("typescript"), None, 1);

    assert_eq!(result.len(), 2);
    let has_token = result
      .iter()
      .any(|(_, spans)| spans.iter().any(|s| s.syntax_token.is_some()));
    assert!(has_token, "expected at least one syntax token");
  }

  #[test]
  fn word_diff_ranges_highlight_added_camel_case_segment() {
    let old_text = "const getLastNotification = () => true;";
    let new_text = "const getLastDataNotification = () => true;";

    let (removed, added) = word_diff_ranges(old_text, new_text);

    assert!(removed.is_empty());
    assert_eq!(added.len(), 1);
    assert_eq!(&new_text[added[0].clone()], "Data");
  }

  #[test]
  fn apply_inline_background_ranges_preserves_syntax_tokens() {
    let spans = vec![InlineSpan {
      range: 0..10,
      style: InlineStyle {
        code: true,
        ..InlineStyle::default()
      },
      link: None,
      syntax_token: Some(TokenType::Keyword),
      background: None,
    }];

    let spans = apply_inline_background_ranges(spans, &[2..5], InlineBackground::DiffWordAdded);

    assert_eq!(spans.len(), 3);
    assert_eq!(spans[0].range, 0..2);
    assert_eq!(spans[0].background, None);
    assert_eq!(spans[1].range, 2..5);
    assert_eq!(spans[1].syntax_token, Some(TokenType::Keyword));
    assert_eq!(spans[1].background, Some(InlineBackground::DiffWordAdded));
    assert_eq!(spans[2].range, 5..10);
  }

  #[test]
  fn github_diff_word_highlights_pairs_removed_and_added_lines() {
    let lines = vec![
      GithubDiffLine {
        old_line: Some(12),
        new_line: None,
        content: Arc::from("type ConfigWithOptionalSecrets = Registry &"),
        kind: GithubDiffLineKind::Removed,
      },
      GithubDiffLine {
        old_line: None,
        new_line: Some(12),
        content: Arc::from("// Some configurations might be missing without `admin_configuration`"),
        kind: GithubDiffLineKind::Added,
      },
      GithubDiffLine {
        old_line: None,
        new_line: Some(13),
        content: Arc::from("type ConfigWithOptionalSecrets = Registry &"),
        kind: GithubDiffLineKind::Added,
      },
    ];

    let highlights = github_diff_word_highlights(&lines);

    assert_eq!(highlights.len(), 3);
    assert!(highlights[0].is_some());
    assert_eq!(
      highlights[0].as_ref().unwrap().background,
      InlineBackground::DiffWordRemoved
    );
    assert!(highlights[1].is_some());
    assert_eq!(
      highlights[1].as_ref().unwrap().background,
      InlineBackground::DiffWordAdded
    );
    assert!(highlights[2].is_none());
  }
}
