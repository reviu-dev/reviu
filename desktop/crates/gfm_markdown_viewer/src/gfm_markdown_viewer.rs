use std::{
  collections::{HashMap, HashSet},
  fs,
  hash::{DefaultHasher, Hash, Hasher},
  ops::Range,
  path::{Path, PathBuf},
  sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
  },
  time::Duration,
};

use base64::{Engine as _, engine::general_purpose};
use comrak::{
  Arena, ComrakOptions,
  nodes::{AstNode, ListType, NodeValue},
  parse_document,
};
use gpui::{
  AnyElement, App, Bounds, ClipboardItem, CursorStyle, DispatchPhase, Div, Element, ElementId,
  FontStyle, FontWeight, GlobalElementId, Hitbox, HitboxBehavior, Hsla, ImageCacheError,
  ImgResourceLoader, InspectorElementId, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent,
  MouseUpEvent, ObjectFit, Pixels, RenderImage, Resource, SharedString, StrikethroughStyle,
  StyledText, TextRun, UnderlineStyle, Window, div, fill, img, point, prelude::*, px, relative,
};
use gpui_component::{
  ActiveTheme as _, Sizable as _, StyledExt as _, clipboard::Clipboard, h_flex, v_flex,
};
use once_cell::sync::Lazy;
use reqwest::header::CONTENT_TYPE;
use syntax::{HighlightSpan, SyntaxHighlighter, SyntaxTheme, TokenType, languages};
use tree_sitter::{Node as TsNode, Parser as TsParser};

use crate::parsed_cache::parse_markdown_for_render;
use crate::preview_segments::{MarkdownRenderSegment, split_markdown_preview_segments};
#[cfg(test)]
use crate::parsed_cache::{PARSED_MARKDOWN_CACHE_MAX_ENTRIES, ParsedMarkdownCache};

type BlockRenderFn = dyn Fn(AnyElement, &App) -> AnyElement + Send + Sync;
type HeadingRenderFn = dyn Fn(u8, AnyElement, &App) -> AnyElement + Send + Sync;
type CodeBlockRenderFn = dyn Fn(&CodeBlock, &App) -> AnyElement + Send + Sync;
type ListItemRenderFn = dyn Fn(ListItemView, &App) -> AnyElement + Send + Sync;
type ThematicBreakRenderFn = dyn Fn(&App) -> AnyElement + Send + Sync;
type TableRenderFn = dyn Fn(&Table, &App) -> AnyElement + Send + Sync;
type LinkHandlerFn = dyn Fn(&str, &mut Window, &mut App) -> LinkAction + Send + Sync;

#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
  Paragraph(Vec<Inline>),
  Heading { level: u8, content: Vec<Inline> },
  List(List),
  CodeBlock(CodeBlock),
  BlockQuote(Vec<Block>),
  ThematicBreak,
  Table(Table),
  Details(Details),
  Aligned { center: bool, blocks: Vec<Block> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeBlock {
  pub lang: Option<String>,
  pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GithubBlobLineReference {
  pub url: String,
  pub owner: String,
  pub repo: String,
  pub reference: String,
  pub path: String,
  pub start_line: usize,
  pub end_line: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GithubCodeReferencePreview {
  pub url: Arc<str>,
  pub repo: Arc<str>,
  pub path: Arc<str>,
  pub reference: Arc<str>,
  pub start_line: usize,
  pub end_line: usize,
  pub snippets: Vec<Arc<str>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
  pub headers: Vec<Vec<Inline>>,
  pub rows: Vec<Vec<Vec<Inline>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Details {
  pub summary: Vec<Inline>,
  pub blocks: Vec<Block>,
  pub open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct List {
  pub ordered: bool,
  pub start: Option<u64>,
  pub items: Vec<ListItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListItem {
  pub blocks: Vec<Block>,
  pub checked: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
  Text(String),
  Link {
    url: String,
    title: Option<String>,
    content: Vec<Inline>,
  },
  Image {
    url: String,
    title: Option<String>,
    alt: String,
    width: Option<String>,
    height: Option<String>,
    dark_url: Option<String>,
    light_url: Option<String>,
  },
  Code(String),
  SoftBreak,
  HardBreak,
  Strong(Vec<Inline>),
  Emphasis(Vec<Inline>),
  Strikethrough(Vec<Inline>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HtmlElement {
  tag: String,
  attrs: Vec<HtmlAttribute>,
  children: Vec<HtmlNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HtmlAttribute {
  name: String,
  value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HtmlNode {
  Element(HtmlElement),
  Text(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct InlineStyle {
  bold: bool,
  italic: bool,
  strike: bool,
  code: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct InlineSpan {
  range: Range<usize>,
  style: InlineStyle,
  link: Option<Arc<str>>,
  syntax_token: Option<TokenType>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LinkRange {
  range: Range<usize>,
  url: Arc<str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GithubIssueReferenceContext {
  pub owner: Arc<str>,
  pub repo: Arc<str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkAction {
  Open,
  Handled,
}

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

#[derive(Clone)]
pub struct MarkdownRenderState {
  instance_id: usize,
  details_open: Arc<Mutex<HashMap<usize, bool>>>,
  selection: Arc<Mutex<Option<ActiveSelection>>>,
}

impl Default for MarkdownRenderState {
  fn default() -> Self {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(1);
    Self {
      instance_id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
      details_open: Arc::new(Mutex::new(HashMap::new())),
      selection: Arc::new(Mutex::new(None)),
    }
  }
}

impl MarkdownRenderState {
  pub fn new() -> Self {
    Self::default()
  }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct SelectionRange {
  start: usize,
  end: usize,
}

impl SelectionRange {
  fn normalized(self) -> Range<usize> {
    if self.start <= self.end {
      self.start..self.end
    } else {
      self.end..self.start
    }
  }

  fn is_empty(self) -> bool {
    self.start == self.end
  }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SelectionState {
  anchor: Option<usize>,
  range: SelectionRange,
  dragging: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ActiveSelection {
  text_id: usize,
  anchor: usize,
  head: usize,
  dragging: bool,
}

fn clamp_to_char_boundary(text: &str, index: usize) -> usize {
  let mut index = index.min(text.len());
  while index > 0 && !text.is_char_boundary(index) {
    index -= 1;
  }
  index
}

#[derive(Clone, Default)]
pub struct MarkdownRenderOptions {
  pub on_link: Option<Arc<LinkHandlerFn>>,
  pub overrides: RenderOverrides,
  pub state: MarkdownRenderState,
  pub scope_id: Option<usize>,
  pub github_code_reference_previews: Option<Arc<HashMap<Arc<str>, GithubCodeReferencePreview>>>,
  pub github_issue_reference_context: Option<GithubIssueReferenceContext>,
  pub expand_code_blocks: bool,
  pub image_base_url: Option<SharedString>,
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
}

pub struct ListItemView {
  pub bullet: String,
  pub checked: Option<bool>,
  pub content: AnyElement,
}

const TABLE_CELL_HORIZONTAL_PADDING_PX: f32 = 24.0;
const TABLE_CELL_MIN_WIDTH_PX: f32 = 64.0;
const TABLE_INLINE_CHAR_WIDTH_PX: f32 = 7.2;
const TABLE_INLINE_GAP_PX: f32 = 4.0;
const TABLE_BADGE_WIDTH_PX: f32 = 56.0;
const LIST_LEFT_PADDING_PX: f32 = 10.0;
const LIST_MARKER_GAP_PX: f32 = 4.0;
const MARKDOWN_BASE_BLOCK_GAP_PX: f32 = 8.0;
const MARKDOWN_HEADING_EXTRA_TOP_MARGIN_PX: f32 = 10.0;
const MARKDOWN_LIST_ITEM_GAP_PX: f32 = 4.0;
const MARKDOWN_INDENT_PER_LEVEL_PX: f32 = 12.0;
const MARKDOWN_CHAR_WIDTH_PX: f32 = 8.8;
const MARKDOWN_MIN_WRAP_COLUMNS: usize = 8;
const MARKDOWN_CODE_LINE_HEIGHT_SCALE: f32 = 0.95;
const MARKDOWN_CODE_BLOCK_PADDING_X_PX: f32 = 12.0;
const MARKDOWN_CODE_BLOCK_PADDING_TOP_PX: f32 = 8.0;
const MARKDOWN_CODE_BLOCK_PADDING_BOTTOM_PX: f32 = 8.0;
const MARKDOWN_CODE_BLOCK_MAX_HEIGHT_PX: f32 = 400.0;
const MARKDOWN_CODE_BLOCK_TEXT_SHIFT_X_PX: f32 = 2.0;
const MARKDOWN_CODE_BLOCK_LEADING_SPACE_RENDER_MULTIPLIER: usize = 2;
const MARKDOWN_CODE_BLOCK_TAB_WIDTH: usize = 4;
const MARKDOWN_CODE_INDENT_DOT_SIZE_PX: f32 = 2.0;
const MARKDOWN_CODE_INDENT_DOT_OPACITY: f32 = 0.45;
const MARKDOWN_CODE_INDENT_DOT_MIN_SPACING_PX: f32 = 5.0;
const MARKDOWN_CODE_INDENT_DOT_MAX_RENDER_COUNT: usize = 600;
const MARKDOWN_CODE_INDENT_DOT_DISABLE_ABOVE_TEXT_LEN: usize = 20_000;
const MARKDOWN_CODE_REFERENCE_CARD_MARGIN_Y_PX: f32 = 8.0;
const MARKDOWN_CODE_REFERENCE_CARD_PADDING_X_PX: f32 = 12.0;
const MARKDOWN_CODE_REFERENCE_CARD_PADDING_Y_PX: f32 = 8.0;
const MARKDOWN_CODE_REFERENCE_CARD_INTERNAL_GAP_PX: f32 = 6.0;
const MARKDOWN_CODE_REFERENCE_SNIPPET_ROW_GAP_PX: f32 = 2.0;
const MARKDOWN_CODE_BLOCK_APPROX_CHAR_WIDTH_PX: f32 = 8.0;
const MARKDOWN_CODE_REFERENCE_ROW_GAP_PX: f32 = 8.0;
const MARKDOWN_INLINE_IMAGE_MAX_HEIGHT_PX: f32 = 420.0;
const MARKDOWN_IMAGE_HARD_BREAK_SPACER_PX: f32 = 14.0;
const MARKDOWN_CODE_BLOCK_VERTICAL_CHROME_PX: f32 =
  MARKDOWN_CODE_BLOCK_PADDING_TOP_PX + MARKDOWN_CODE_BLOCK_PADDING_BOTTOM_PX + 2.0;
static BADGE_IMAGE_SOURCE_CACHE: Lazy<Mutex<HashMap<String, BadgeResolveState>>> =
  Lazy::new(|| Mutex::new(HashMap::new()));

#[derive(Clone, Debug)]
enum BadgeImageSource {
  Remote(String),
  Local(PathBuf),
}

#[derive(Clone, Debug)]
enum BadgeResolveState {
  Pending,
  Ready(BadgeImageSource),
  Failed,
}

enum Segment {
  Markdown(String),
  Details {
    summary: Option<String>,
    body: String,
    open: bool,
  },
}

#[derive(Clone)]
pub struct ParsedMarkdown {
  blocks: Arc<Vec<Block>>,
}

pub fn parse_gfm(source: &str) -> Vec<Block> {
  let mut blocks = Vec::new();
  for segment in split_details_segments(source) {
    match segment {
      Segment::Markdown(markdown) => blocks.extend(parse_comrak(&markdown)),
      Segment::Details {
        summary,
        body,
        open,
      } => {
        let summary_inlines = summary_inlines_from_text(summary.as_deref().unwrap_or("Details"));
        let body_blocks = parse_gfm(&body);
        blocks.push(Block::Details(Details {
          summary: summary_inlines,
          blocks: body_blocks,
          open,
        }));
      }
    }
  }
  blocks
}

pub fn parse_markdown(source: &str) -> ParsedMarkdown {
  ParsedMarkdown {
    blocks: Arc::new(parse_gfm(source)),
  }
}

pub fn render_parsed_markdown(
  parsed: &ParsedMarkdown,
  options: &MarkdownRenderOptions,
  cx: &App,
) -> AnyElement {
  let scope_id = resolve_scope_id_for_parsed(parsed, options);
  let mut ctx = RenderContext::new(scope_id);
  render_blocks(parsed.blocks.as_ref(), options, 0, cx, &mut ctx)
}

pub fn estimate_markdown_height_px(source: &str, wrap_columns: usize, line_height_px: f32) -> f32 {
  let parsed = parse_markdown_for_render(source);
  estimate_parsed_markdown_height_px(&parsed, wrap_columns, line_height_px)
}

pub fn estimate_parsed_markdown_height_px(
  parsed: &ParsedMarkdown,
  wrap_columns: usize,
  line_height_px: f32,
) -> f32 {
  estimate_blocks_height_px(
    parsed.blocks.as_ref(),
    wrap_columns.max(MARKDOWN_MIN_WRAP_COLUMNS),
    line_height_px.max(1.0),
    0,
  )
}

fn estimate_blocks_height_px(
  blocks: &[Block],
  wrap_columns: usize,
  line_height_px: f32,
  indent: usize,
) -> f32 {
  if blocks.is_empty() {
    return line_height_px;
  }

  let mut total = 0.0f32;
  for (ix, block) in blocks.iter().enumerate() {
    if ix > 0 {
      total += MARKDOWN_BASE_BLOCK_GAP_PX;
      if matches!(block, Block::Heading { .. }) {
        total += MARKDOWN_HEADING_EXTRA_TOP_MARGIN_PX;
      }
    }
    total += estimate_block_height_px(block, wrap_columns, line_height_px, indent);
  }

  total.max(line_height_px)
}

fn estimate_block_height_px(
  block: &Block,
  wrap_columns: usize,
  line_height_px: f32,
  indent: usize,
) -> f32 {
  match block {
    Block::Paragraph(inlines) => {
      estimate_inline_lines(inlines, wrap_columns_for_indent(wrap_columns, indent)) as f32
        * line_height_px
    }
    Block::Heading { level, content } => {
      let lines =
        estimate_inline_lines(content, wrap_columns_for_indent(wrap_columns, indent)).max(1);
      let scale = match level {
        1 => 1.35,
        2 => 1.2,
        3 => 1.05,
        _ => 1.0,
      };
      lines as f32 * line_height_px * scale
    }
    Block::List(list) => estimate_list_height_px(list, wrap_columns, line_height_px, indent),
    Block::CodeBlock(code) => {
      let code_lines = code.value.lines().count().max(1) as f32;
      (code_lines * line_height_px * MARKDOWN_CODE_LINE_HEIGHT_SCALE
        + MARKDOWN_CODE_BLOCK_VERTICAL_CHROME_PX)
        .min(MARKDOWN_CODE_BLOCK_MAX_HEIGHT_PX)
    }
    Block::BlockQuote(children) => {
      estimate_blocks_height_px(children, wrap_columns, line_height_px, indent + 1)
    }
    Block::ThematicBreak => 1.0,
    Block::Table(table) => estimate_table_height_px(table, line_height_px),
    Block::Details(details) => {
      estimate_details_height_px(details, wrap_columns, line_height_px, indent)
    }
    Block::Aligned { blocks, .. } => {
      estimate_blocks_height_px(blocks, wrap_columns, line_height_px, indent)
    }
  }
}

fn estimate_list_height_px(
  list: &List,
  wrap_columns: usize,
  line_height_px: f32,
  indent: usize,
) -> f32 {
  if list.items.is_empty() {
    return line_height_px;
  }

  let indent_cols =
    ((LIST_LEFT_PADDING_PX + LIST_MARKER_GAP_PX + 14.0) / MARKDOWN_CHAR_WIDTH_PX).ceil() as usize;
  let item_wrap_columns = wrap_columns_for_indent(wrap_columns, indent)
    .saturating_sub(indent_cols)
    .max(MARKDOWN_MIN_WRAP_COLUMNS);

  let mut total = 0.0f32;
  for (ix, item) in list.items.iter().enumerate() {
    if ix > 0 {
      total += MARKDOWN_LIST_ITEM_GAP_PX;
    }
    total += estimate_blocks_height_px(&item.blocks, item_wrap_columns, line_height_px, indent + 1);
  }

  total.max(line_height_px)
}

fn estimate_details_height_px(
  details: &Details,
  wrap_columns: usize,
  line_height_px: f32,
  indent: usize,
) -> f32 {
  let summary_cols = wrap_columns_for_indent(wrap_columns, indent)
    .saturating_sub(3)
    .max(MARKDOWN_MIN_WRAP_COLUMNS);
  let summary_height =
    estimate_inline_lines(&details.summary, summary_cols).max(1) as f32 * line_height_px;

  if !details.open {
    return summary_height;
  }

  let body = estimate_blocks_height_px(&details.blocks, wrap_columns, line_height_px, indent + 1);
  summary_height + MARKDOWN_BASE_BLOCK_GAP_PX + body
}

fn estimate_table_height_px(table: &Table, line_height_px: f32) -> f32 {
  let header_content_height = estimate_table_row_content_height_px(&table.headers, line_height_px);
  let header_row_height = header_content_height + 16.0;
  let mut total = header_row_height + 2.0;

  for row in &table.rows {
    let content_height = estimate_table_row_content_height_px(row, line_height_px);
    total += content_height + 16.0 + 1.0;
  }

  total.max(line_height_px)
}

fn estimate_table_row_content_height_px(cells: &[Vec<Inline>], line_height_px: f32) -> f32 {
  let mut max_height = line_height_px;
  for cell in cells {
    let has_image = cell.iter().any(inline_contains_image);
    let cell_height = if has_image { 18.0 } else { line_height_px };
    max_height = max_height.max(cell_height);
  }
  max_height
}

fn wrap_columns_for_indent(base_wrap_columns: usize, indent: usize) -> usize {
  let indent_columns =
    ((indent as f32 * MARKDOWN_INDENT_PER_LEVEL_PX) / MARKDOWN_CHAR_WIDTH_PX).ceil() as usize;
  base_wrap_columns
    .saturating_sub(indent_columns)
    .max(MARKDOWN_MIN_WRAP_COLUMNS)
}

fn estimate_inline_lines(inlines: &[Inline], wrap_columns: usize) -> usize {
  let text = inline_to_plain_text(inlines);
  if text.is_empty() {
    return 1;
  }

  let mut lines = 0usize;
  for line in text.split('\n') {
    lines = lines.saturating_add(estimate_wrapped_text_lines(line, wrap_columns));
  }
  lines.max(1)
}

fn estimate_wrapped_text_lines(line: &str, wrap_columns: usize) -> usize {
  let wrap_columns = wrap_columns.max(1);
  if line.is_empty() {
    return 1;
  }

  let mut wrapped_lines = 0usize;
  let mut current_len = 0usize;
  let mut has_word = false;

  for word in line.split_whitespace() {
    has_word = true;
    let mut word_len = word.chars().count();
    if word_len == 0 {
      continue;
    }

    if current_len == 0 {
      if word_len <= wrap_columns {
        current_len = word_len;
      } else {
        wrapped_lines = wrapped_lines.saturating_add(word_len / wrap_columns);
        word_len %= wrap_columns;
        current_len = word_len;
      }
      continue;
    }

    if current_len + 1 + word_len <= wrap_columns {
      current_len += 1 + word_len;
      continue;
    }

    wrapped_lines = wrapped_lines.saturating_add(1);
    if word_len <= wrap_columns {
      current_len = word_len;
    } else {
      wrapped_lines = wrapped_lines.saturating_add(word_len / wrap_columns);
      word_len %= wrap_columns;
      current_len = word_len;
    }
  }

  if !has_word {
    return 1;
  }

  if current_len > 0 {
    wrapped_lines = wrapped_lines.saturating_add(1);
  }

  wrapped_lines.max(1)
}

fn parse_comrak(source: &str) -> Vec<Block> {
  let arena = Arena::new();
  let options = comrak_options();
  let root = parse_document(&arena, source, &options);
  let mut blocks = Vec::new();
  let mut centered_div_depth = 0usize;
  let mut centered_div_blocks = Vec::new();

  for node in root.children() {
    let mut is_centered_open = false;
    let mut is_centered_close = false;

    if let NodeValue::HtmlBlock(html) = &node.data.borrow().value {
      is_centered_open = is_centered_div_open_tag(&html.literal);
      is_centered_close = is_centered_div_close_tag(&html.literal);
    }

    if is_centered_open {
      centered_div_depth = centered_div_depth.saturating_add(1);
      continue;
    }

    if is_centered_close {
      if centered_div_depth > 0 {
        centered_div_depth -= 1;
      }
      if centered_div_depth == 0 && !centered_div_blocks.is_empty() {
        blocks.push(Block::Aligned {
          center: true,
          blocks: std::mem::take(&mut centered_div_blocks),
        });
      }
      continue;
    }

    let node_blocks = blocks_from_node(node);
    if centered_div_depth > 0 {
      centered_div_blocks.extend(node_blocks);
    } else {
      blocks.extend(node_blocks);
    }
  }

  if !centered_div_blocks.is_empty() {
    blocks.push(Block::Aligned {
      center: true,
      blocks: centered_div_blocks,
    });
  }

  blocks
}

fn split_details_segments(source: &str) -> Vec<Segment> {
  let mut segments = Vec::new();
  let mut buffer = String::new();
  let mut fence: Option<(char, usize)> = None;
  let mut lines = source.lines();
  let mut pending_line: Option<String> = None;

  while let Some(line) = pending_line
    .take()
    .or_else(|| lines.next().map(|line| line.to_string()))
  {
    update_fence_state(&line, &mut fence);
    if fence.is_none()
      && let Some(start_idx) = find_details_start(&line)
    {
      let (prefix, rest) = line.split_at(start_idx);
      if !prefix.is_empty() {
        buffer.push_str(prefix);
        buffer.push('\n');
      }

      if !buffer.is_empty() {
        segments.push(Segment::Markdown(buffer));
        buffer = String::new();
      }

      let open = has_open_attribute(rest);
      let mut details_lines = Vec::new();
      let mut details_fence: Option<(char, usize)> = None;
      let mut depth = 0isize;

      let (first_part, trailing, new_depth) = split_details_line(rest, depth);
      depth = new_depth;
      details_lines.push(first_part);
      if depth == 0 {
        if let Some(trailing) = trailing {
          pending_line = Some(trailing);
        }
      } else {
        while depth > 0 {
          let Some(next_line) = lines.next() else {
            break;
          };
          let next_line = next_line.to_string();
          update_fence_state(&next_line, &mut details_fence);
          if details_fence.is_some() {
            details_lines.push(next_line);
            continue;
          }

          let (part, trailing, new_depth) = split_details_line(&next_line, depth);
          depth = new_depth;
          details_lines.push(part);
          if depth == 0 {
            if let Some(trailing) = trailing {
              pending_line = Some(trailing);
            }
            break;
          }
        }
      }

      let details_source = details_lines.join("\n");
      if let Some((summary, body)) = parse_details_block(&details_source) {
        segments.push(Segment::Details {
          summary,
          body,
          open,
        });
      } else {
        segments.push(Segment::Markdown(details_source));
      }
      continue;
    }

    buffer.push_str(&line);
    buffer.push('\n');
  }

  if !buffer.is_empty() {
    segments.push(Segment::Markdown(buffer));
  }

  segments
}

fn update_fence_state(line: &str, fence: &mut Option<(char, usize)>) {
  let trimmed = line.trim_start();
  let mut chars = trimmed.chars();
  let Some(first) = chars.next() else {
    return;
  };
  if first != '`' && first != '~' {
    return;
  }
  let mut count = 1usize;
  for ch in chars {
    if ch == first {
      count += 1;
    } else {
      break;
    }
  }
  if count < 3 {
    return;
  }
  match fence {
    None => {
      *fence = Some((first, count));
    }
    Some((fence_char, fence_len)) if *fence_char == first && count >= *fence_len => {
      *fence = None;
    }
    _ => {}
  }
}

fn find_details_start(line: &str) -> Option<usize> {
  let lower = line.to_ascii_lowercase();
  lower.find("<details")
}

fn has_open_attribute(line: &str) -> bool {
  let lower = line.to_ascii_lowercase();
  if let Some(end) = lower.find('>') {
    let tag = &lower[..end];
    tag
      .split_whitespace()
      .any(|part| part == "open" || part.starts_with("open="))
  } else {
    false
  }
}

fn split_details_line(line: &str, mut depth: isize) -> (String, Option<String>, isize) {
  let lower = line.to_ascii_lowercase();
  let mut idx = 0usize;
  let mut split_at: Option<usize> = None;

  while idx < lower.len() {
    let next_open = lower[idx..].find("<details").map(|pos| idx + pos);
    let next_close = lower[idx..].find("</details").map(|pos| idx + pos);

    let (next_pos, is_open) = match (next_open, next_close) {
      (None, None) => break,
      (Some(pos), None) => (pos, true),
      (None, Some(pos)) => (pos, false),
      (Some(open_pos), Some(close_pos)) => {
        if open_pos <= close_pos {
          (open_pos, true)
        } else {
          (close_pos, false)
        }
      }
    };

    if is_open {
      depth += 1;
      idx = next_pos + "<details".len();
      continue;
    }

    depth -= 1;
    let close_end = match lower[next_pos..].find('>') {
      Some(rel) => next_pos + rel + 1,
      None => next_pos + "</details".len(),
    };
    idx = close_end;
    if depth <= 0 {
      depth = 0;
      split_at = Some(close_end);
      break;
    }
  }

  if let Some(end) = split_at {
    let (part, trailing) = line.split_at(end);
    let trailing = if trailing.is_empty() {
      None
    } else {
      Some(trailing.to_string())
    };
    return (part.to_string(), trailing, depth);
  }

  (line.to_string(), None, depth)
}

fn parse_details_block(source: &str) -> Option<(Option<String>, String)> {
  let lower = source.to_ascii_lowercase();
  let start = lower.find("<details")?;
  let open_tag_end = lower[start..].find('>')? + start;
  let end = find_last_details_close_end(&lower).unwrap_or(source.len());
  if end <= open_tag_end {
    return None;
  }
  let inner = &source[open_tag_end + 1..end];
  let (summary, body) = extract_summary(inner);
  Some((summary, body))
}

fn find_last_details_close_end(lower: &str) -> Option<usize> {
  let mut cursor = 0usize;
  let mut last_end = None;

  while cursor < lower.len() {
    let Some(rel_start) = lower[cursor..].find("</details") else {
      break;
    };
    let start = cursor + rel_start;
    let Some(rel_end) = lower[start..].find('>') else {
      break;
    };
    let end = start + rel_end + 1;
    last_end = Some(end);
    cursor = end;
  }

  last_end
}

fn extract_url_token_candidates(text: &str) -> Vec<String> {
  let mut tokens = Vec::new();
  let mut remaining = text;

  loop {
    let https_idx = remaining.find("https://");
    let http_idx = remaining.find("http://");
    let start = match (https_idx, http_idx) {
      (None, None) => break,
      (Some(https), None) => https,
      (None, Some(http)) => http,
      (Some(https), Some(http)) => https.min(http),
    };
    remaining = &remaining[start..];

    let end = remaining
      .find(char::is_whitespace)
      .unwrap_or(remaining.len());
    let token = remaining[..end]
      .trim_matches(|ch: char| {
        matches!(
          ch,
          '(' | ')' | '[' | ']' | '<' | '>' | '"' | '\'' | ',' | ';'
        )
      })
      .trim_end_matches('.')
      .to_string();
    if !token.is_empty() {
      tokens.push(token);
    }
    remaining = &remaining[end..];
  }

  tokens
}

pub fn parse_github_blob_line_reference(url: &str) -> Option<GithubBlobLineReference> {
  let trimmed = url.trim();
  let rest = trimmed
    .strip_prefix("https://github.com/")
    .or_else(|| trimmed.strip_prefix("http://github.com/"))?;
  let (path_and_blob, fragment) = rest.split_once('#')?;
  let path_and_blob = path_and_blob.split('?').next().unwrap_or(path_and_blob);
  let fragment = fragment.strip_prefix('L')?;
  let start_digits: String = fragment
    .chars()
    .take_while(|ch| ch.is_ascii_digit())
    .collect();
  if start_digits.is_empty() {
    return None;
  }
  let start_line = start_digits.parse::<usize>().ok()?;
  if start_line == 0 {
    return None;
  }
  let fragment_tail = &fragment[start_digits.len()..];
  let end_line = if fragment_tail.is_empty() {
    start_line
  } else {
    let fragment_tail = fragment_tail.strip_prefix('-')?;
    let fragment_tail = fragment_tail.strip_prefix('L').unwrap_or(fragment_tail);
    let end_digits: String = fragment_tail
      .chars()
      .take_while(|ch| ch.is_ascii_digit())
      .collect();
    if end_digits.is_empty() {
      return None;
    }
    let parsed = end_digits.parse::<usize>().ok()?;
    if parsed == 0 {
      return None;
    }
    parsed
  };
  let (start_line, end_line) = if start_line <= end_line {
    (start_line, end_line)
  } else {
    (end_line, start_line)
  };

  let (repo_path, blob_path) = path_and_blob.split_once("/blob/")?;
  let mut repo_parts = repo_path.split('/');
  let owner = repo_parts.next()?.trim();
  let repo = repo_parts.next()?.trim();
  if owner.is_empty() || repo.is_empty() || repo_parts.next().is_some() {
    return None;
  }

  let (reference, file_path) = blob_path.split_once('/')?;
  if reference.is_empty() || file_path.is_empty() {
    return None;
  }

  Some(GithubBlobLineReference {
    url: trimmed.to_string(),
    owner: owner.to_string(),
    repo: repo.to_string(),
    reference: reference.to_string(),
    path: file_path.to_string(),
    start_line,
    end_line,
  })
}

pub fn extract_github_blob_line_references(text: &str) -> Vec<GithubBlobLineReference> {
  let mut references = Vec::new();
  let mut seen = HashSet::new();

  for candidate in extract_url_token_candidates(text) {
    if let Some(reference) = parse_github_blob_line_reference(&candidate)
      && seen.insert(reference.url.clone())
    {
      references.push(reference);
    }
  }

  references
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

  let mut snippet_rows = v_flex().gap(px(MARKDOWN_CODE_REFERENCE_SNIPPET_ROW_GAP_PX));
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
    for (offset, snippet) in preview.snippets.iter().enumerate() {
      let line_number = preview.start_line + offset;
      let (snippet_text, snippet_spans, snippet_links) =
        build_preview_code_spans(snippet.as_ref(), snippet_language_hint.as_deref());
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
                snippet_text,
                snippet_spans,
                snippet_links,
                snippet_render_state.clone(),
                None,
                text_id,
                SelectableTextOptions {
                  interactive: false,
                  show_indentation_dots: true,
                },
              )),
          ),
      );
    }
  }

  v_flex()
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
          v_flex()
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
    .child(
      div()
        .px(px(MARKDOWN_CODE_REFERENCE_CARD_PADDING_X_PX))
        .py(px(MARKDOWN_CODE_REFERENCE_CARD_PADDING_Y_PX))
        .child(
          div()
            .id(preview_scroll_id)
            .w_full()
            .min_w_0()
            .max_h(px(MARKDOWN_CODE_BLOCK_MAX_HEIGHT_PX))
            .overflow_scroll()
            .on_scroll_wheel(|_, _, cx| {
              cx.stop_propagation();
            })
            .child(
              div()
                .min_w(px(min_preview_content_width_px))
                .whitespace_nowrap()
                .text_sm()
                .text_color(theme.foreground)
                .child(
                  v_flex()
                    .gap(px(MARKDOWN_CODE_REFERENCE_CARD_INTERNAL_GAP_PX))
                    .child(snippet_rows),
                ),
            ),
        ),
    )
}

pub fn estimate_github_code_reference_preview_height_px(
  snippet_line_count: usize,
  row_height_px: f32,
) -> f32 {
  let row_height_px = row_height_px.max(1.0);
  let snippet_line_count = snippet_line_count.max(1) as f32;
  let snippet_rows_height_px = row_height_px * snippet_line_count
    + MARKDOWN_CODE_REFERENCE_SNIPPET_ROW_GAP_PX * (snippet_line_count - 1.0).max(0.0);
  let snippet_scroll_height_px = snippet_rows_height_px.min(MARKDOWN_CODE_BLOCK_MAX_HEIGHT_PX);

  row_height_px * 2.0
    + MARKDOWN_CODE_REFERENCE_CARD_PADDING_Y_PX * 4.0
    + MARKDOWN_CODE_REFERENCE_CARD_MARGIN_Y_PX * 2.0
    + 3.0
    + snippet_scroll_height_px
}

fn estimate_code_reference_preview_min_content_width_px(
  preview: &GithubCodeReferencePreview,
) -> f32 {
  let widest_row_columns = if preview.snippets.is_empty() {
    preview.start_line.to_string().chars().count()
  } else {
    preview
      .snippets
      .iter()
      .enumerate()
      .map(|(offset, snippet)| {
        let line_number_columns = (preview.start_line + offset).to_string().chars().count();
        line_number_columns + 2 + snippet.chars().count()
      })
      .max()
      .unwrap_or(0)
  } as f32;

  (widest_row_columns * MARKDOWN_CODE_BLOCK_APPROX_CHAR_WIDTH_PX
    + MARKDOWN_CODE_REFERENCE_ROW_GAP_PX)
    .ceil()
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

fn build_preview_code_spans(
  snippet: &str,
  language_hint: Option<&str>,
) -> (SharedString, Vec<InlineSpan>, Vec<LinkRange>) {
  build_code_block_spans(&CodeBlock {
    lang: language_hint.map(ToOwned::to_owned),
    value: snippet.to_string(),
  })
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

  let mut rendered = v_flex();
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

struct RenderContext {
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
    let id = self.next_details_id;
    self.next_details_id += 1;
    id
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

fn comrak_options() -> ComrakOptions {
  let mut options = ComrakOptions::default();
  options.extension.strikethrough = true;
  options.extension.table = true;
  options.extension.tasklist = true;
  options.extension.autolink = true;
  options.extension.tagfilter = true;
  options.parse.smart = true;
  options
}

fn render_blocks(
  blocks: &[Block],
  options: &MarkdownRenderOptions,
  indent: usize,
  cx: &App,
  ctx: &mut RenderContext,
) -> AnyElement {
  let mut container = v_flex().gap_2();

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
      render_code_block(code, options, cx, ctx)
    }
    Block::BlockQuote(children) => {
      let content = div()
        .border_l_2()
        .border_color(cx.theme().muted_foreground)
        .pl(px(8.0))
        .child(render_blocks(children, options, indent + 1, cx, ctx))
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
        let mut aligned = v_flex().w_full().min_w_0().gap_2();
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
  let mut container = v_flex()
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
  let mut container = v_flex().w_full().min_w_0().gap_2();
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

  let mut header_row = h_flex().bg(theme.accent.opacity(0.3));
  for (column, width) in column_widths.iter().enumerate().take(column_count) {
    let cell = table
      .headers
      .get(column)
      .map_or(&[][..], |cell| cell.as_slice());
    header_row = header_row.child(
      div()
        .w(px(*width))
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
            .whitespace_nowrap()
            .child(render_table_cell_inlines(cell, options, cx, ctx)),
        ),
    );
  }

  let mut body = v_flex();
  for row in &table.rows {
    let mut row_el = h_flex().border_t_1().border_color(theme.border);
    for (column, width) in column_widths.iter().enumerate().take(column_count) {
      let cell = row.get(column).map_or(&[][..], |cell| cell.as_slice());
      row_el = row_el.child(
        div()
          .w(px(*width))
          .px_3()
          .py_2()
          .when(column + 1 < column_count, |this| {
            this.border_r_1().border_color(theme.border)
          })
          .child(
            div()
              .text_sm()
              .text_color(theme.foreground)
              .whitespace_nowrap()
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
    .overflow_x_scroll()
    .child(
      div()
        .border_1()
        .border_color(theme.border)
        .rounded_md()
        .overflow_hidden()
        .child(v_flex().child(header_row).child(body)),
    )
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

fn resolve_badge_image_source_async(url: &str) -> Option<BadgeImageSource> {
  {
    let cache = BADGE_IMAGE_SOURCE_CACHE.lock().unwrap();
    if let Some(state) = cache.get(url) {
      return match state {
        BadgeResolveState::Ready(source) => Some(source.clone()),
        BadgeResolveState::Pending | BadgeResolveState::Failed => None,
      };
    }
  }

  BADGE_IMAGE_SOURCE_CACHE
    .lock()
    .unwrap()
    .insert(url.to_string(), BadgeResolveState::Pending);

  let url = url.to_string();
  std::thread::spawn(move || {
    let source = fetch_badge_image_source(&url);
    let state = if let Some(source) = source {
      BadgeResolveState::Ready(source)
    } else {
      BadgeResolveState::Failed
    };
    BADGE_IMAGE_SOURCE_CACHE.lock().unwrap().insert(url, state);
  });

  None
}

fn load_badge_image_data(
  source: &BadgeImageSource,
  window: &mut Window,
  cx: &mut App,
) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
  let resource = match source {
    BadgeImageSource::Remote(url) => Resource::Uri(url.clone().into()),
    BadgeImageSource::Local(path) => Resource::from(path.clone()),
  };
  window.use_asset::<ImgResourceLoader>(&resource, cx)
}

fn fetch_badge_image_source(url: &str) -> Option<BadgeImageSource> {
  if let Some(state) = BADGE_IMAGE_SOURCE_CACHE.lock().unwrap().get(url)
    && let BadgeResolveState::Ready(source) = state
  {
    return Some(source.clone());
  }

  let source = fetch_badge_image_source_blocking(url)?;
  BADGE_IMAGE_SOURCE_CACHE
    .lock()
    .unwrap()
    .insert(url.to_string(), BadgeResolveState::Ready(source.clone()));
  Some(source)
}

fn fetch_badge_image_source_blocking(url: &str) -> Option<BadgeImageSource> {
  let client = match reqwest::blocking::Client::builder()
    .timeout(Duration::from_secs(4))
    .build()
  {
    Ok(client) => client,
    Err(_) => return Some(BadgeImageSource::Remote(url.to_string())),
  };

  let response = match client.get(url).send() {
    Ok(response) => response,
    Err(_) => return Some(BadgeImageSource::Remote(url.to_string())),
  };

  let content_type = response
    .headers()
    .get(CONTENT_TYPE)
    .and_then(|value| value.to_str().ok())
    .unwrap_or("unknown")
    .to_string();

  let bytes = match response.bytes() {
    Ok(bytes) => bytes,
    Err(_) => return Some(BadgeImageSource::Remote(url.to_string())),
  };

  if (content_type.contains("svg") || bytes.starts_with(b"<svg"))
    && let Ok(svg) = String::from_utf8(bytes.to_vec())
    && should_resolve_svg_embedded_image(&svg)
    && let Some(href) = extract_svg_image_href(&svg)
    && let Some(source) = resolve_badge_href(url, &href)
  {
    return Some(source);
  }

  Some(BadgeImageSource::Remote(url.to_string()))
}

fn should_resolve_svg_embedded_image(svg: &str) -> bool {
  let lower = svg.to_ascii_lowercase();
  if lower.match_indices("<image").count() != 1 {
    return false;
  }

  let has_badge_like_shape_or_text = [
    "<text",
    "<rect",
    "<path",
    "<line",
    "<polyline",
    "<polygon",
    "<circle",
    "<ellipse",
  ]
  .iter()
  .any(|pattern| lower.contains(pattern));

  !has_badge_like_shape_or_text
}

fn extract_svg_image_href(svg: &str) -> Option<String> {
  let lower = svg.to_ascii_lowercase();
  let start = lower.find("<image")?;
  let end = lower[start..].find('>')? + start;
  let tag = &svg[start..=end];
  extract_html_attribute(tag, "xlink:href").or_else(|| extract_html_attribute(tag, "href"))
}

fn resolve_badge_href(base_url: &str, href: &str) -> Option<BadgeImageSource> {
  if href.starts_with("data:") {
    return data_uri_to_temp_file(href).map(BadgeImageSource::Local);
  }

  if href.starts_with("http://") || href.starts_with("https://") {
    return Some(BadgeImageSource::Remote(href.to_string()));
  }

  if href.starts_with("//") {
    return Some(BadgeImageSource::Remote(format!("https:{href}")));
  }

  if let Ok(base) = reqwest::Url::parse(base_url)
    && let Ok(joined) = base.join(href)
  {
    return Some(BadgeImageSource::Remote(joined.to_string()));
  }

  None
}

fn markdown_image_repo_root_url(base: &reqwest::Url) -> Option<reqwest::Url> {
  let root_segments = base
    .path_segments()
    .map(|segments| {
      segments
        .filter(|segment| !segment.is_empty())
        .take(3)
        .collect::<Vec<_>>()
    })
    .unwrap_or_default();
  if root_segments.len() < 3 {
    return None;
  }
  let mut root = base.clone();
  let root_path = format!("/{}/", root_segments.join("/"));
  root.set_path(root_path.as_str());
  Some(root)
}

fn resolve_markdown_image_url(url: &str, image_base_url: Option<&str>) -> String {
  let trimmed = url.trim();
  if trimmed.is_empty() {
    return String::new();
  }

  if trimmed.starts_with("data:")
    || trimmed.starts_with("http://")
    || trimmed.starts_with("https://")
  {
    return trimmed.to_string();
  }

  if trimmed.starts_with("//") {
    return format!("https:{trimmed}");
  }

  let Some(base_url) = image_base_url else {
    return trimmed.to_string();
  };
  let Ok(base) = reqwest::Url::parse(base_url) else {
    return trimmed.to_string();
  };

  if trimmed.starts_with('/')
    && let Some(repo_root) = markdown_image_repo_root_url(&base)
    && let Ok(joined) = repo_root.join(trimmed.trim_start_matches('/'))
  {
    return joined.to_string();
  }

  if let Ok(joined) = base.join(trimmed) {
    return joined.to_string();
  }

  trimmed.to_string()
}

fn data_uri_to_temp_file(data_uri: &str) -> Option<PathBuf> {
  let (meta, payload) = data_uri.split_once(',')?;
  if !meta.contains(";base64") {
    return None;
  }

  let extension = if meta.starts_with("data:image/png") {
    "png"
  } else if meta.starts_with("data:image/jpeg") || meta.starts_with("data:image/jpg") {
    "jpg"
  } else if meta.starts_with("data:image/webp") {
    "webp"
  } else if meta.starts_with("data:image/gif") {
    "gif"
  } else if meta.starts_with("data:image/svg+xml") {
    "svg"
  } else {
    "bin"
  };

  let mut hasher = DefaultHasher::new();
  data_uri.hash(&mut hasher);
  let path = std::env::temp_dir().join(format!("reviu-badge-{:x}.{extension}", hasher.finish()));
  if path.exists() {
    return Some(path);
  }

  let sanitized = payload.replace(char::is_whitespace, "");
  let bytes = general_purpose::STANDARD
    .decode(sanitized.as_bytes())
    .ok()?;
  fs::write(&path, bytes).ok()?;
  Some(path)
}

fn inline_contains_image(inline: &Inline) -> bool {
  match inline {
    Inline::Image { .. } => true,
    Inline::Link { content, .. }
    | Inline::Strong(content)
    | Inline::Emphasis(content)
    | Inline::Strikethrough(content) => content.iter().any(inline_contains_image),
    _ => false,
  }
}

fn inline_image_data(
  inline: &Inline,
) -> Option<(
  String,
  String,
  Option<String>,
  Option<String>,
  Option<String>,
  Option<String>,
  Option<String>,
)> {
  match inline {
    Inline::Image {
      url,
      alt,
      width,
      height,
      dark_url,
      light_url,
      ..
    } => Some((
      url.clone(),
      alt.clone(),
      None,
      width.clone(),
      height.clone(),
      dark_url.clone(),
      light_url.clone(),
    )),
    Inline::Link {
      url: link_url,
      content,
      ..
    } => {
      for child in content {
        if let Some((
          url,
          alt,
          child_link,
          child_width,
          child_height,
          child_dark_url,
          child_light_url,
        )) = inline_image_data(child)
        {
          return Some((
            url,
            alt,
            Some(child_link.unwrap_or_else(|| link_url.clone())),
            child_width,
            child_height,
            child_dark_url,
            child_light_url,
          ));
        }
      }
      None
    }
    Inline::Strong(content) | Inline::Emphasis(content) | Inline::Strikethrough(content) => {
      for child in content {
        if let Some((url, alt, link, width, height, dark_url, light_url)) = inline_image_data(child)
        {
          return Some((url, alt, link, width, height, dark_url, light_url));
        }
      }
      None
    }
    _ => None,
  }
}

fn split_inlines_by_hard_breaks(inlines: &[Inline]) -> Vec<Vec<Inline>> {
  let mut rows = Vec::new();
  let mut current_row = Vec::new();

  for inline in inlines {
    if matches!(inline, Inline::HardBreak) {
      rows.push(current_row);
      current_row = Vec::new();
      continue;
    }
    current_row.push(inline.clone());
  }

  rows.push(current_row);
  rows
}

fn single_inline_image_data(
  inlines: &[Inline],
) -> Option<(
  String,
  String,
  Option<String>,
  Option<String>,
  Option<String>,
  Option<String>,
  Option<String>,
)> {
  if inlines.len() != 1 {
    return None;
  }
  inline_image_data(&inlines[0])
}

fn render_table_cell_inlines(
  inlines: &[Inline],
  options: &MarkdownRenderOptions,
  cx: &App,
  ctx: &mut RenderContext,
) -> AnyElement {
  if !inlines.iter().any(inline_contains_image) {
    return render_inline_text(inlines, options, cx, ctx);
  }

  let mut row = h_flex().items_center().gap_1();
  let mut text_chunk: Vec<Inline> = Vec::new();
  let is_dark_mode = cx.theme().mode.is_dark();

  for inline in inlines {
    if let Some((url, alt, _, _, _, dark_url, light_url)) = inline_image_data(inline) {
      if !text_chunk.is_empty() {
        row = row.child(render_inline_text(&text_chunk, options, cx, ctx));
        text_chunk.clear();
      }

      let badge_label = if alt.is_empty() {
        "image".to_string()
      } else {
        alt
      };
      let themed_url = select_markdown_image_url_for_theme(
        &url,
        dark_url.as_deref(),
        light_url.as_deref(),
        is_dark_mode,
      );
      let badge_url = resolve_markdown_image_url(
        &themed_url,
        options.image_base_url.as_ref().map(SharedString::as_ref),
      );
      row = row.child(
        img(move |window: &mut Window, cx: &mut App| {
          if let Some(source) = resolve_badge_image_source_async(&badge_url) {
            return load_badge_image_data(&source, window, cx);
          }

          window.request_animation_frame();
          None
        })
        .h(px(18.0))
        .object_fit(ObjectFit::Contain)
        .with_loading({
          let badge_label = badge_label.clone();
          move || render_badge_placeholder(&badge_label)
        })
        .with_fallback(move || render_badge_placeholder(&badge_label)),
      );
    } else {
      text_chunk.push(inline.clone());
    }
  }

  if !text_chunk.is_empty() {
    row = row.child(render_inline_text(&text_chunk, options, cx, ctx));
  }

  row.into_any_element()
}

fn render_badge_placeholder(label: &str) -> AnyElement {
  let text = label.trim();
  let text = if text.is_empty() {
    "badge".to_string()
  } else {
    text.to_string()
  };
  div()
    .h(px(18.0))
    .px_2()
    .rounded_sm()
    .bg(Hsla {
      h: 220.0 / 360.0,
      s: 0.18,
      l: 0.58,
      a: 1.0,
    })
    .text_xs()
    .text_color(Hsla {
      h: 0.0,
      s: 0.0,
      l: 1.0,
      a: 1.0,
    })
    .child(text)
    .into_any_element()
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum MarkdownImageDimension {
  Pixels(f32),
  Fraction(f32),
}

fn parse_markdown_image_dimension(dimension_hint: Option<&str>) -> Option<MarkdownImageDimension> {
  let dimension_hint = dimension_hint
    .map(str::trim)
    .filter(|hint| !hint.is_empty())?;
  let lower = dimension_hint.to_ascii_lowercase();

  if let Some(percent) = lower.strip_suffix('%')
    && let Ok(value) = percent.trim().parse::<f32>()
    && value.is_finite()
    && value > 0.0
  {
    return Some(MarkdownImageDimension::Fraction((value / 100.0).min(1.0)));
  }

  let px_value = lower.strip_suffix("px").unwrap_or(lower.as_str()).trim();
  if let Ok(value) = px_value.parse::<f32>()
    && value.is_finite()
    && value > 0.0
  {
    return Some(MarkdownImageDimension::Pixels(value));
  }

  None
}

fn select_markdown_image_url_for_theme(
  url: &str,
  dark_url: Option<&str>,
  light_url: Option<&str>,
  is_dark_mode: bool,
) -> String {
  let fallback = {
    let trimmed = url.trim();
    if trimmed.is_empty() { url } else { trimmed }
  };
  let themed = (if is_dark_mode { dark_url } else { light_url })
    .map(str::trim)
    .filter(|value| !value.is_empty())
    .unwrap_or(fallback);
  themed.to_string()
}

fn render_image_node(
  url: &str,
  alt: &str,
  width_hint: Option<&str>,
  height_hint: Option<&str>,
) -> impl IntoElement {
  let label = if alt.trim().is_empty() {
    "image".to_string()
  } else {
    alt.trim().to_string()
  };
  let image_url = url.to_string();
  let mut image = img(move |window: &mut Window, cx: &mut App| {
    if let Some(source) = resolve_badge_image_source_async(&image_url) {
      return load_badge_image_data(&source, window, cx);
    }

    window.request_animation_frame();
    None
  })
  .max_h(px(MARKDOWN_INLINE_IMAGE_MAX_HEIGHT_PX))
  .object_fit(ObjectFit::Contain);
  if let Some(width) = parse_markdown_image_dimension(width_hint) {
    image = match width {
      MarkdownImageDimension::Pixels(value) => image.w(px(value)),
      MarkdownImageDimension::Fraction(value) => image.w(relative(value)),
    };
  }
  if let Some(height) = parse_markdown_image_dimension(height_hint) {
    image = match height {
      MarkdownImageDimension::Pixels(value) => image.h(px(value)),
      MarkdownImageDimension::Fraction(value) => image.h(relative(value)),
    };
  }

  image
    .with_loading({
      let label = label.clone();
      move || render_badge_placeholder(&label)
    })
    .with_fallback(move || render_badge_placeholder(&label))
}

fn render_block_image_node(
  url: &str,
  alt: &str,
  width_hint: Option<&str>,
  height_hint: Option<&str>,
) -> impl IntoElement {
  let label = if alt.trim().is_empty() {
    "image".to_string()
  } else {
    alt.trim().to_string()
  };
  let image_url = url.to_string();
  let mut image = img(move |window: &mut Window, cx: &mut App| {
    if let Some(source) = resolve_badge_image_source_async(&image_url) {
      return load_badge_image_data(&source, window, cx);
    }

    window.request_animation_frame();
    None
  })
  .max_w_full()
  .h_auto();
  if let Some(width) = parse_markdown_image_dimension(width_hint) {
    image = match width {
      MarkdownImageDimension::Pixels(value) => image.w(px(value)),
      MarkdownImageDimension::Fraction(value) => image.w(relative(value)),
    };
  }
  if let Some(height) = parse_markdown_image_dimension(height_hint) {
    image = match height {
      MarkdownImageDimension::Pixels(value) => image.h(px(value)),
      MarkdownImageDimension::Fraction(value) => image.h(relative(value)),
    };
  }

  image
    .with_loading({
      let label = label.clone();
      move || render_badge_placeholder(&label)
    })
    .with_fallback(move || render_badge_placeholder(&label))
}

fn attach_image_link_handler(
  image: AnyElement,
  url: &str,
  link_url: Option<&str>,
  on_link: Option<Arc<LinkHandlerFn>>,
  interactive: bool,
) -> AnyElement {
  let mut hasher = DefaultHasher::new();
  url.hash(&mut hasher);
  link_url.hash(&mut hasher);
  let image_id: SharedString = format!("markdown-inline-image-{:x}", hasher.finish()).into();

  let mut container = div().id(image_id).child(image);
  if interactive && let Some(link_url) = link_url {
    let link_url = link_url.to_string();
    let on_link = on_link.clone();
    container = container.cursor_pointer().on_click(move |_, window, cx| {
      let handled = on_link
        .as_ref()
        .is_some_and(|handler| matches!(handler(&link_url, window, cx), LinkAction::Handled));
      if !handled {
        cx.open_url(&link_url);
      }
    });
  }

  container.into_any_element()
}

fn render_inline_image(
  url: &str,
  dark_url: Option<&str>,
  light_url: Option<&str>,
  alt: &str,
  width_hint: Option<&str>,
  height_hint: Option<&str>,
  link_url: Option<&str>,
  on_link: Option<Arc<LinkHandlerFn>>,
  interactive: bool,
  is_dark_mode: bool,
  image_base_url: Option<&str>,
) -> AnyElement {
  let themed_url = select_markdown_image_url_for_theme(url, dark_url, light_url, is_dark_mode);
  let resolved_url = resolve_markdown_image_url(&themed_url, image_base_url);
  let image = render_image_node(&resolved_url, alt, width_hint, height_hint).into_any_element();
  attach_image_link_handler(image, &resolved_url, link_url, on_link, interactive)
}

fn render_block_image(
  url: &str,
  dark_url: Option<&str>,
  light_url: Option<&str>,
  alt: &str,
  width_hint: Option<&str>,
  height_hint: Option<&str>,
  link_url: Option<&str>,
  on_link: Option<Arc<LinkHandlerFn>>,
  interactive: bool,
  is_dark_mode: bool,
  image_base_url: Option<&str>,
) -> AnyElement {
  let themed_url = select_markdown_image_url_for_theme(url, dark_url, light_url, is_dark_mode);
  let resolved_url = resolve_markdown_image_url(&themed_url, image_base_url);
  let mut hasher = DefaultHasher::new();
  resolved_url.hash(&mut hasher);
  link_url.hash(&mut hasher);
  let image_scroll_id: SharedString =
    format!("markdown-inline-image-scroll-{:x}", hasher.finish()).into();

  div()
    .id(image_scroll_id)
    .w_full()
    .child(attach_image_link_handler(
      render_block_image_node(&resolved_url, alt, width_hint, height_hint).into_any_element(),
      &resolved_url,
      link_url,
      on_link,
      interactive,
    ))
    .into_any_element()
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
    },
  )
  .into_any_element()
}

fn render_inline_with_images(
  inlines: &[Inline],
  options: &MarkdownRenderOptions,
  interactive: bool,
  cx: &App,
  ctx: &mut RenderContext,
) -> AnyElement {
  let is_dark_mode = cx.theme().mode.is_dark();
  if let Some((url, alt, link, width, height, dark_url, light_url)) =
    single_inline_image_data(inlines)
  {
    return render_block_image(
      &url,
      dark_url.as_deref(),
      light_url.as_deref(),
      &alt,
      width.as_deref(),
      height.as_deref(),
      link.as_deref(),
      options.on_link.clone(),
      interactive,
      is_dark_mode,
      options.image_base_url.as_ref().map(SharedString::as_ref),
    );
  }

  let rows = split_inlines_by_hard_breaks(inlines);
  let mut content = v_flex().min_w_0();
  let mut has_content = false;

  for (row_ix, row) in rows.iter().enumerate() {
    let mut row_container = h_flex().items_center().gap_1().flex_wrap().min_w_0();
    let mut row_has_content = false;
    let mut text_chunk: Vec<Inline> = Vec::new();

    for inline in row {
      if let Some((url, alt, link, width, height, dark_url, light_url)) = inline_image_data(inline)
      {
        if !text_chunk.is_empty() {
          row_container = row_container.child(render_inline_selectable_text(
            &text_chunk,
            options,
            interactive,
            cx,
            ctx,
          ));
          text_chunk.clear();
        }
        row_container = row_container.child(render_inline_image(
          &url,
          dark_url.as_deref(),
          light_url.as_deref(),
          &alt,
          width.as_deref(),
          height.as_deref(),
          link.as_deref(),
          options.on_link.clone(),
          interactive,
          is_dark_mode,
          options.image_base_url.as_ref().map(SharedString::as_ref),
        ));
        row_has_content = true;
      } else {
        text_chunk.push(inline.clone());
      }
    }

    if !text_chunk.is_empty() {
      row_container = row_container.child(render_inline_selectable_text(
        &text_chunk,
        options,
        interactive,
        cx,
        ctx,
      ));
      row_has_content = true;
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

fn render_inline_text(
  inlines: &[Inline],
  options: &MarkdownRenderOptions,
  cx: &App,
  ctx: &mut RenderContext,
) -> AnyElement {
  if inlines.iter().any(inline_contains_image) {
    return render_inline_with_images(inlines, options, true, cx, ctx);
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

fn render_code_block(
  code: &CodeBlock,
  options: &MarkdownRenderOptions,
  cx: &App,
  ctx: &mut RenderContext,
) -> AnyElement {
  let theme = cx.theme();
  let (text, spans, link_ranges) = build_code_block_spans(code);
  let min_content_width_px = estimate_code_block_min_content_width_px(text.as_ref());
  let text_id = ctx.next_text_id();
  let content = SelectableText::new(
    text,
    spans,
    link_ranges,
    options.state.clone(),
    options.on_link.clone(),
    text_id,
    SelectableTextOptions {
      interactive: true,
      show_indentation_dots: true,
    },
  );
  let scroll_id: SharedString = format!("markdown-code-block-scroll-{text_id}").into();
  let scroll_content = div().id(scroll_id).w_full().min_w_0().child(
    div()
      .min_w(px(min_content_width_px))
      .px(px(MARKDOWN_CODE_BLOCK_PADDING_X_PX))
      .pt(px(MARKDOWN_CODE_BLOCK_PADDING_TOP_PX))
      .pb(px(MARKDOWN_CODE_BLOCK_PADDING_BOTTOM_PX))
      .whitespace_nowrap()
      .child(
        div()
          .pl(px(MARKDOWN_CODE_BLOCK_TEXT_SHIFT_X_PX))
          .font_family(cx.theme().mono_font_family.clone())
          .text_sm()
          .text_color(theme.foreground)
          .child(content),
      ),
  );

  let scroll_container = if options.expand_code_blocks {
    scroll_content.overflow_x_scroll().into_any_element()
  } else {
    scroll_content
      .max_h(px(MARKDOWN_CODE_BLOCK_MAX_HEIGHT_PX))
      .overflow_scroll()
      .on_scroll_wheel(|_, _, cx| {
        cx.stop_propagation();
      })
      .into_any_element()
  };
  let copy_value = code_block_copy_value(code);
  let hover_group_id = code_block_hover_group_id(text_id);

  div()
    .w_full()
    .min_w_0()
    .relative()
    .group(hover_group_id.clone())
    .bg(theme.accent.opacity(0.3))
    .border_1()
    .border_color(theme.border)
    .rounded_md()
    .overflow_hidden()
    .child(scroll_container)
    .child(
      div()
        .absolute()
        .top_1()
        .right_1()
        .invisible()
        .group_hover(&hover_group_id, |this| this.visible())
        .child(Clipboard::new(("markdown-code-block-copy", text_id)).value(copy_value)),
    )
    .into_any_element()
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

fn collect_indentation_dot_indices(text: &str) -> Vec<usize> {
  if text.len() > MARKDOWN_CODE_INDENT_DOT_DISABLE_ABOVE_TEXT_LEN || !text.contains(' ') {
    return Vec::new();
  }

  let mut indices = Vec::new();
  let mut leading_spaces = Vec::new();
  let mut saw_non_whitespace = false;
  let mut in_leading_indent = true;

  for (ix, ch) in text.char_indices() {
    match ch {
      '\n' | '\r' => {
        if saw_non_whitespace {
          indices.extend_from_slice(&leading_spaces);
        }
        leading_spaces.clear();
        saw_non_whitespace = false;
        in_leading_indent = true;
      }
      ' ' if in_leading_indent => {
        leading_spaces.push(ix);
      }
      ' ' => {}
      '\t' if in_leading_indent => {
        in_leading_indent = false;
      }
      '\t' => {}
      _ => {
        saw_non_whitespace = true;
        in_leading_indent = false;
      }
    }
  }

  if saw_non_whitespace {
    indices.extend_from_slice(&leading_spaces);
  }

  limit_indentation_dot_indices(indices)
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

fn build_code_block_spans(code: &CodeBlock) -> (SharedString, Vec<InlineSpan>, Vec<LinkRange>) {
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

fn syntax_highlight_spans_for_code(
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
      });
    }

    spans.push(InlineSpan {
      range: start..end,
      style: base_style,
      link: None,
      syntax_token: Some(token_type),
    });
    current_pos = end;
  }

  if current_pos < text_len {
    spans.push(InlineSpan {
      range: current_pos..text_len,
      style: base_style,
      link: None,
      syntax_token: None,
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
  let summary = h_flex()
    .id(toggle_id)
    .items_center()
    .gap_2()
    .child(gpui_component::Icon::new(toggle_icon).small())
    .child(
      div()
        .whitespace_normal()
        .text_sm()
        .font_medium()
        .text_color(theme.foreground)
        .child(render_inline_static(&details.summary, options, cx, ctx)),
    )
    .on_mouse_down(MouseButton::Left, move |_, window, cx| {
      let mut map = toggle_state.details_open.lock().unwrap();
      let next = !map.get(&details_id).copied().unwrap_or(default_open);
      map.insert(details_id, next);
      window.refresh();
      cx.stop_propagation();
    });

  let mut container = v_flex().gap_2().child(summary);
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
}

impl SpanBuilder {
  fn push_inlines(&mut self, inlines: &[Inline], style: InlineStyle, link: Option<Arc<str>>) {
    for inline in inlines {
      match inline {
        Inline::Text(value) => self.push_text(value, style, link.clone()),
        Inline::Code(value) => {
          let mut code_style = style;
          code_style.code = true;
          self.push_text(value, code_style, link.clone());
        }
        Inline::SoftBreak => self.push_text(" ", style, link.clone()),
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

struct SelectableText {
  text: SharedString,
  spans: Vec<InlineSpan>,
  link_ranges: Vec<LinkRange>,
  render_state: MarkdownRenderState,
  on_link: Option<Arc<LinkHandlerFn>>,
  text_id: usize,
  interactive: bool,
  show_indentation_dots: bool,
  indentation_dot_indices: Vec<usize>,
  styled_text: StyledText,
  runs_initialized: bool,
  last_selection: Option<Range<usize>>,
}

#[derive(Clone, Copy)]
struct SelectableTextOptions {
  interactive: bool,
  show_indentation_dots: bool,
}

impl SelectableText {
  fn new(
    text: SharedString,
    spans: Vec<InlineSpan>,
    link_ranges: Vec<LinkRange>,
    render_state: MarkdownRenderState,
    on_link: Option<Arc<LinkHandlerFn>>,
    text_id: usize,
    options: SelectableTextOptions,
  ) -> Self {
    let styled_text = StyledText::new(text.clone());
    let indentation_dot_indices = if options.show_indentation_dots {
      collect_indentation_dot_indices(text.as_ref())
    } else {
      Vec::new()
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
      indentation_dot_indices,
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
      self
        .styled_text
        .paint(None, inspector_id, bounds, &mut (), &mut (), window, cx);
      self.paint_indentation_dots(&text_layout, window, cx);
      return;
    }

    let text_layout = self.styled_text.layout().clone();
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

impl IntoElement for SelectableText {
  type Element = Self;

  fn into_element(self) -> Self::Element {
    self
  }
}

fn selection_state_for(state: &MarkdownRenderState, text_id: usize) -> SelectionState {
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

fn update_selection_state(
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

fn selection_for_text(
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

fn selection_text(
  state: &MarkdownRenderState,
  text_id: usize,
  text: &SharedString,
) -> Option<String> {
  let selection = selection_for_text(state, text_id, text)?;
  text.as_ref().get(selection).map(|value| value.to_string())
}

fn build_runs(
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
  for span in spans {
    let mut font = base_font.clone();
    if span.style.code {
      font.family = SharedString::new_static(".ZedMono");
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
      background_color: None,
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

fn github_link_color(background: Hsla) -> Hsla {
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

fn syntax_theme_for_background(background: Hsla) -> SyntaxTheme {
  if background.l < 0.5 {
    SyntaxTheme::default_dark()
  } else {
    SyntaxTheme::default_light()
  }
}

fn apply_selection_to_runs(
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

fn blocks_from_node<'a>(node: &'a AstNode<'a>) -> Vec<Block> {
  match &node.data.borrow().value {
    NodeValue::Paragraph => vec![Block::Paragraph(inlines_from_nodes(node.children()))],
    NodeValue::Heading(heading) => vec![Block::Heading {
      level: heading.level,
      content: inlines_from_nodes(node.children()),
    }],
    NodeValue::List(list) => vec![Block::List(List {
      ordered: matches!(list.list_type, ListType::Ordered),
      start: if matches!(list.list_type, ListType::Ordered) {
        Some(list.start as u64)
      } else {
        None
      },
      items: node.children().filter_map(list_item_from_node).collect(),
    })],
    NodeValue::CodeBlock(code) => vec![Block::CodeBlock(CodeBlock {
      lang: code
        .info
        .split_whitespace()
        .next()
        .map(|value| value.to_string()),
      value: code.literal.clone(),
    })],
    NodeValue::BlockQuote => vec![Block::BlockQuote(
      node.children().flat_map(blocks_from_node).collect(),
    )],
    NodeValue::ThematicBreak => vec![Block::ThematicBreak],
    NodeValue::Table(_) => vec![Block::Table(table_from_node(node))],
    NodeValue::Item(_) => node.children().flat_map(blocks_from_node).collect(),
    NodeValue::HtmlBlock(html) => {
      if is_html_comment_only_block(&html.literal) || is_details_close_only_block(&html.literal) {
        Vec::new()
      } else if let Some(details) = parse_details_html(&html.literal) {
        vec![Block::Details(details)]
      } else if let Some(centered) = parse_centered_div_html(&html.literal) {
        centered
      } else {
        blocks_from_html_fragment(&html.literal)
      }
    }
    NodeValue::Text(text) => vec![Block::Paragraph(vec![Inline::Text(text.clone())])],
    _ => {
      let text = collect_text(node);
      if text.is_empty() {
        Vec::new()
      } else {
        vec![Block::Paragraph(vec![Inline::Text(text)])]
      }
    }
  }
}

fn parse_details_html(html: &str) -> Option<Details> {
  let trimmed = html.trim();
  let lower = trimmed.to_ascii_lowercase();
  let start = lower.find("<details")?;
  let open_tag_end = lower[start..].find('>')? + start;
  let open = has_open_attribute(&lower[start..=open_tag_end]);
  let end = find_last_details_close_end(&lower).unwrap_or(trimmed.len());
  if end <= open_tag_end {
    return None;
  }

  let inner = &trimmed[open_tag_end + 1..end];
  let (summary_text, body_text) = extract_summary(inner);
  let summary_inlines = summary_inlines_from_text(summary_text.as_deref().unwrap_or("Details"));
  let blocks = if body_text.trim().is_empty() {
    Vec::new()
  } else {
    parse_gfm(&body_text)
  };

  Some(Details {
    summary: summary_inlines,
    blocks,
    open,
  })
}

fn is_centered_div_open_tag(html: &str) -> bool {
  let trimmed = html.trim();
  let lower = trimmed.to_ascii_lowercase();
  if !lower.starts_with("<div")
    || lower.starts_with("</div")
    || lower.contains("</div")
    || trimmed.ends_with("/>")
  {
    return false;
  }

  if let Some(align) = extract_html_attribute(trimmed, "align")
    && align.trim().eq_ignore_ascii_case("center")
  {
    return true;
  }

  if let Some(style) = extract_html_attribute(trimmed, "style") {
    let normalized = style
      .chars()
      .filter(|ch| !ch.is_whitespace())
      .collect::<String>()
      .to_ascii_lowercase();
    if normalized.contains("text-align:center") {
      return true;
    }
  }

  false
}

fn is_centered_div_close_tag(html: &str) -> bool {
  html.trim_start().to_ascii_lowercase().starts_with("</div")
}

fn parse_centered_div_html(html: &str) -> Option<Vec<Block>> {
  let trimmed = html.trim();
  let lower = trimmed.to_ascii_lowercase();
  if !lower.starts_with("<div") || !lower.contains("</div>") {
    return None;
  }

  let open_end = lower.find('>')?;
  let open_tag = &trimmed[..=open_end];
  if !is_centered_div_open_tag(open_tag) {
    return None;
  }

  let close_start = lower.rfind("</div>")?;
  if close_start <= open_end {
    return None;
  }

  let body = &trimmed[open_end + 1..close_start];
  let blocks = parse_gfm(body);
  if blocks.is_empty() {
    return Some(Vec::new());
  }

  Some(vec![Block::Aligned {
    center: true,
    blocks,
  }])
}

fn is_html_comment_only_block(html: &str) -> bool {
  let mut rest = html.trim();
  if rest.is_empty() {
    return false;
  }

  while !rest.is_empty() {
    if !rest.starts_with("<!--") {
      return false;
    }

    let Some(end) = rest.find("-->") else {
      return false;
    };

    rest = rest[end + 3..].trim_start();
  }

  true
}

fn is_details_close_only_block(html: &str) -> bool {
  let mut rest = html.trim();
  if rest.is_empty() {
    return false;
  }

  while !rest.is_empty() {
    let lower = rest.to_ascii_lowercase();
    if !lower.starts_with("</details") {
      return false;
    }

    let Some(close_idx) = rest.find('>') else {
      return false;
    };
    rest = rest[close_idx + 1..].trim_start();
  }

  true
}

fn extract_summary(inner: &str) -> (Option<String>, String) {
  let lower = inner.to_ascii_lowercase();
  let summary_start = match lower.find("<summary") {
    Some(index) => index,
    None => return (None, inner.to_string()),
  };
  let tag_end = match lower[summary_start..].find('>') {
    Some(index) => summary_start + index,
    None => return (None, inner.to_string()),
  };
  let close_start = match lower[tag_end..].find("</summary>") {
    Some(index) => tag_end + index,
    None => return (None, inner.to_string()),
  };
  let summary_inner = inner[tag_end + 1..close_start].to_string();
  let mut body = String::new();
  body.push_str(&inner[..summary_start]);
  body.push_str(&inner[close_start + "</summary>".len()..]);
  (Some(summary_inner), body)
}

fn summary_inlines_from_text(summary_text: &str) -> Vec<Inline> {
  for block in parse_gfm(summary_text) {
    if let Block::Paragraph(inlines) = block {
      if !inline_to_plain_text(&inlines).is_empty() {
        return inlines;
      }
      break;
    }
  }
  vec![Inline::Text(strip_html_tags(summary_text))]
}

fn strip_html_tags(input: &str) -> String {
  let mut out = String::new();
  let mut in_tag = false;
  for ch in input.chars() {
    match ch {
      '<' => in_tag = true,
      '>' => in_tag = false,
      _ if !in_tag => out.push(ch),
      _ => {}
    }
  }
  out
}

fn parse_inline_html_image(html: &str) -> Option<Inline> {
  let lower = html.to_ascii_lowercase();
  let start = lower.find("<img")?;
  let end = lower[start..].find('>')? + start;
  let tag = &html[start..=end];
  let url = extract_html_attribute(tag, "src")?;
  let alt = extract_html_attribute(tag, "alt").unwrap_or_default();
  let title = extract_html_attribute(tag, "title");
  let width = extract_html_attribute(tag, "width")
    .map(|value| value.trim().to_string())
    .filter(|value| !value.is_empty());
  let height = extract_html_attribute(tag, "height")
    .map(|value| value.trim().to_string())
    .filter(|value| !value.is_empty());
  Some(Inline::Image {
    url,
    title,
    alt,
    width,
    height,
    dark_url: None,
    light_url: None,
  })
}

fn is_html_line_break_tag(html: &str) -> bool {
  let trimmed = html.trim();
  if !trimmed.starts_with('<') || !trimmed.ends_with('>') || trimmed.len() < 3 {
    return false;
  }

  let inner = &trimmed[1..trimmed.len() - 1];
  let inner = inner.trim();
  let inner = inner.strip_suffix('/').unwrap_or(inner).trim_end();
  inner.eq_ignore_ascii_case("br")
}

fn decode_basic_html_entities(segment: &str) -> String {
  segment
    .replace("&nbsp;", " ")
    .replace("&amp;", "&")
    .replace("&lt;", "<")
    .replace("&gt;", ">")
    .replace("&quot;", "\"")
    .replace("&#39;", "'")
}

fn push_html_text_segment(inlines: &mut Vec<Inline>, segment: &str, pending_space: &mut bool) {
  let segment = decode_basic_html_entities(segment);
  for ch in segment.chars() {
    if ch.is_whitespace() {
      *pending_space = true;
      continue;
    }

    if *pending_space {
      if let Some(Inline::Text(last)) = inlines.last_mut()
        && !last.is_empty()
        && !last.ends_with('\n')
      {
        last.push(' ');
      }
      *pending_space = false;
    }

    if let Some(Inline::Text(last)) = inlines.last_mut() {
      last.push(ch);
    } else {
      inlines.push(Inline::Text(ch.to_string()));
    }
  }
}

fn push_html_inline(inlines: &mut Vec<Inline>, inline: Inline, pending_space: &mut bool) {
  match inline {
    Inline::Text(text) => {
      push_html_text_segment(inlines, &text, pending_space);
    }
    Inline::HardBreak => {
      inlines.push(Inline::HardBreak);
      *pending_space = false;
    }
    other => {
      if *pending_space {
        if let Some(Inline::Text(last)) = inlines.last_mut()
          && !last.is_empty()
          && !last.ends_with('\n')
        {
          last.push(' ');
        } else if !inlines.is_empty() {
          inlines.push(Inline::Text(" ".to_string()));
        }
        *pending_space = false;
      }
      inlines.push(other);
    }
  }
}

fn push_html_inlines(inlines: &mut Vec<Inline>, items: Vec<Inline>, pending_space: &mut bool) {
  for inline in items {
    push_html_inline(inlines, inline, pending_space);
  }
}

fn html_inlines_have_visible_content(inlines: &[Inline]) -> bool {
  if inlines.is_empty() {
    return false;
  }
  if inlines.iter().any(inline_contains_image) {
    return true;
  }
  if inlines
    .iter()
    .any(|inline| matches!(inline, Inline::HardBreak))
  {
    return true;
  }
  !inline_to_plain_text(inlines).trim().is_empty()
}

fn flush_html_inline_buffer_to_blocks(blocks: &mut Vec<Block>, inlines: &mut Vec<Inline>) {
  let merged = merge_adjacent_text(inlines);
  inlines.clear();
  if html_inlines_have_visible_content(&merged) {
    blocks.push(Block::Paragraph(merged));
  }
}

fn blocks_from_html_fragment(html: &str) -> Vec<Block> {
  if let Some(nodes) = parse_html_fragment_nodes(html) {
    let blocks = html_nodes_to_blocks(&nodes);
    if !blocks.is_empty() {
      return blocks;
    }
  }

  let inlines = legacy_inlines_from_html_fragment(html);
  if inlines.is_empty() {
    Vec::new()
  } else {
    vec![Block::Paragraph(inlines)]
  }
}

fn parse_html_fragment_nodes(html: &str) -> Option<Vec<HtmlNode>> {
  let mut parser = TsParser::new();
  let language: tree_sitter::Language = tree_sitter_html::LANGUAGE.into();
  parser.set_language(&language).ok()?;
  let tree = parser.parse(html, None)?;
  let root = tree.root_node();

  let mut nodes = Vec::new();
  let mut cursor = root.walk();
  let mut previous_end = root.start_byte();
  for child in root.named_children(&mut cursor) {
    append_html_text_node_from_range(&mut nodes, html, previous_end, child.start_byte());
    if let Some(node) = html_node_from_tree_sitter(child, html) {
      nodes.push(node);
    }
    previous_end = child.end_byte();
  }
  append_html_text_node_from_range(&mut nodes, html, previous_end, root.end_byte());

  Some(nodes)
}

fn tree_sitter_node_text<'a>(node: TsNode<'a>, source: &'a str) -> &'a str {
  source.get(node.byte_range()).unwrap_or_default()
}

fn append_html_text_node_from_range(
  nodes: &mut Vec<HtmlNode>,
  source: &str,
  start: usize,
  end: usize,
) {
  if end <= start {
    return;
  }
  if let Some(text) = source.get(start..end)
    && !text.is_empty()
  {
    nodes.push(HtmlNode::Text(text.to_string()));
  }
}

fn html_node_from_tree_sitter(node: TsNode<'_>, source: &str) -> Option<HtmlNode> {
  match node.kind() {
    "element" => html_element_from_tree_sitter_element(node, source).map(HtmlNode::Element),
    "self_closing_tag" => {
      let (tag, attrs) = html_tag_from_tree_sitter(node, source)?;
      Some(HtmlNode::Element(HtmlElement {
        tag,
        attrs,
        children: Vec::new(),
      }))
    }
    "text" | "entity" => {
      let text = tree_sitter_node_text(node, source).to_string();
      if text.is_empty() {
        None
      } else {
        Some(HtmlNode::Text(text))
      }
    }
    _ => None,
  }
}

fn html_element_from_tree_sitter_element(node: TsNode<'_>, source: &str) -> Option<HtmlElement> {
  let mut tag_name = None;
  let mut attrs = Vec::new();
  let mut children = Vec::new();
  let mut content_start = None;

  let mut cursor = node.walk();
  for child in node.named_children(&mut cursor) {
    match child.kind() {
      "start_tag" | "self_closing_tag" => {
        if let Some((tag, parsed_attrs)) = html_tag_from_tree_sitter(child, source) {
          tag_name = Some(tag);
          attrs = parsed_attrs;
          content_start = Some(child.end_byte());
        }
      }
      "end_tag" | "erroneous_end_tag" => {
        if let Some(start) = content_start.take() {
          append_html_text_node_from_range(&mut children, source, start, child.start_byte());
        }
      }
      _ => {
        if let Some(start) = content_start {
          append_html_text_node_from_range(&mut children, source, start, child.start_byte());
        }
        if let Some(mapped) = html_node_from_tree_sitter(child, source) {
          children.push(mapped);
        }
        content_start = Some(child.end_byte());
      }
    }
  }
  if let Some(start) = content_start {
    append_html_text_node_from_range(&mut children, source, start, node.end_byte());
  }

  Some(HtmlElement {
    tag: tag_name?,
    attrs,
    children,
  })
}

fn html_tag_from_tree_sitter(
  node: TsNode<'_>,
  source: &str,
) -> Option<(String, Vec<HtmlAttribute>)> {
  let mut tag_name = None;
  let mut attrs = Vec::new();

  let mut cursor = node.walk();
  for child in node.named_children(&mut cursor) {
    match child.kind() {
      "tag_name" => {
        let name = tree_sitter_node_text(child, source).trim();
        if !name.is_empty() {
          tag_name = Some(name.to_ascii_lowercase());
        }
      }
      "attribute" => {
        if let Some(attr) = html_attribute_from_tree_sitter(child, source) {
          attrs.push(attr);
        }
      }
      _ => {}
    }
  }

  Some((tag_name?, attrs))
}

fn html_attribute_from_tree_sitter(node: TsNode<'_>, source: &str) -> Option<HtmlAttribute> {
  let mut name = None;
  let mut value = None;

  let mut cursor = node.walk();
  for child in node.named_children(&mut cursor) {
    match child.kind() {
      "attribute_name" => {
        let attr = tree_sitter_node_text(child, source).trim();
        if !attr.is_empty() {
          name = Some(attr.to_ascii_lowercase());
        }
      }
      "attribute_value" => {
        value = Some(tree_sitter_node_text(child, source).to_string());
      }
      "quoted_attribute_value" => {
        let mut quoted_cursor = child.walk();
        let mut parsed = None;
        for quoted_child in child.named_children(&mut quoted_cursor) {
          if quoted_child.kind() == "attribute_value" {
            parsed = Some(tree_sitter_node_text(quoted_child, source).to_string());
            break;
          }
        }
        value = Some(parsed.unwrap_or_default());
      }
      _ => {}
    }
  }

  Some(HtmlAttribute { name: name?, value })
}

fn html_attribute_value<'a>(element: &'a HtmlElement, name: &str) -> Option<&'a str> {
  element
    .attrs
    .iter()
    .find(|attr| attr.name.eq_ignore_ascii_case(name))
    .and_then(|attr| attr.value.as_deref())
}

fn parse_html_picture_source_url(srcset: &str) -> Option<String> {
  srcset.split(',').find_map(|candidate| {
    candidate
      .split_whitespace()
      .next()
      .map(str::trim)
      .filter(|value| !value.is_empty())
      .map(ToString::to_string)
  })
}

fn html_picture_theme_urls(element: &HtmlElement) -> (Option<String>, Option<String>) {
  let mut dark_url = None;
  let mut light_url = None;

  for child in &element.children {
    let HtmlNode::Element(source) = child else {
      continue;
    };
    if source.tag != "source" {
      continue;
    }

    let Some(srcset) = html_attribute_value(source, "srcset")
      .map(str::trim)
      .filter(|value| !value.is_empty())
    else {
      continue;
    };
    let Some(source_url) = parse_html_picture_source_url(srcset) else {
      continue;
    };
    let media = html_attribute_value(source, "media")
      .map(str::trim)
      .unwrap_or_default()
      .to_ascii_lowercase();
    if media.contains("prefers-color-scheme") && media.contains("dark") {
      if dark_url.is_none() {
        dark_url = Some(source_url);
      }
      continue;
    }
    if media.contains("prefers-color-scheme") && media.contains("light") && light_url.is_none() {
      light_url = Some(source_url);
    }
  }

  (dark_url, light_url)
}

fn html_element_is_centered(element: &HtmlElement) -> bool {
  if let Some(align) = html_attribute_value(element, "align")
    && align.trim().eq_ignore_ascii_case("center")
  {
    return true;
  }

  if let Some(style) = html_attribute_value(element, "style") {
    let normalized = style
      .chars()
      .filter(|ch| !ch.is_whitespace())
      .collect::<String>()
      .to_ascii_lowercase();
    if normalized.contains("text-align:center") {
      return true;
    }
  }

  false
}

fn is_html_block_level_tag(tag: &str) -> bool {
  matches!(tag, "p" | "div" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
}

fn html_nodes_to_blocks(nodes: &[HtmlNode]) -> Vec<Block> {
  let mut blocks = Vec::new();
  let mut inline_buffer = Vec::new();
  let mut pending_space = false;

  for node in nodes {
    match node {
      HtmlNode::Text(text) => {
        push_html_text_segment(&mut inline_buffer, text, &mut pending_space);
      }
      HtmlNode::Element(element) => {
        if is_html_block_level_tag(element.tag.as_str()) {
          flush_html_inline_buffer_to_blocks(&mut blocks, &mut inline_buffer);
          blocks.extend(html_element_to_blocks(element));
        } else {
          let inlines = html_element_to_inlines(element);
          push_html_inlines(&mut inline_buffer, inlines, &mut pending_space);
        }
      }
    }
  }

  flush_html_inline_buffer_to_blocks(&mut blocks, &mut inline_buffer);
  blocks
}

fn html_nodes_to_inlines(nodes: &[HtmlNode]) -> Vec<Inline> {
  let mut inlines = Vec::new();
  let mut pending_space = false;

  for node in nodes {
    match node {
      HtmlNode::Text(text) => {
        push_html_text_segment(&mut inlines, text, &mut pending_space);
      }
      HtmlNode::Element(element) => {
        let child_inlines = html_element_to_inlines(element);
        push_html_inlines(&mut inlines, child_inlines, &mut pending_space);
      }
    }
  }

  merge_adjacent_text(&inlines)
}

fn html_heading_level(tag: &str) -> Option<u8> {
  match tag {
    "h1" => Some(1),
    "h2" => Some(2),
    "h3" => Some(3),
    "h4" => Some(4),
    "h5" => Some(5),
    "h6" => Some(6),
    _ => None,
  }
}

fn html_element_to_blocks(element: &HtmlElement) -> Vec<Block> {
  if let Some(level) = html_heading_level(element.tag.as_str()) {
    let content = html_nodes_to_inlines(&element.children);
    if !html_inlines_have_visible_content(&content) {
      return Vec::new();
    }

    let heading = Block::Heading { level, content };
    if html_element_is_centered(element) {
      return vec![Block::Aligned {
        center: true,
        blocks: vec![heading],
      }];
    }
    return vec![heading];
  }

  let mut blocks = html_nodes_to_blocks(&element.children);
  if blocks.is_empty() {
    let fallback = html_nodes_to_inlines(&element.children);
    if html_inlines_have_visible_content(&fallback) {
      blocks.push(Block::Paragraph(fallback));
    }
  }

  if blocks.is_empty() {
    return blocks;
  }

  if html_element_is_centered(element) {
    vec![Block::Aligned {
      center: true,
      blocks,
    }]
  } else {
    blocks
  }
}

fn html_element_to_inlines(element: &HtmlElement) -> Vec<Inline> {
  match element.tag.as_str() {
    "br" => vec![Inline::HardBreak],
    "img" => {
      let Some(url) = html_attribute_value(element, "src")
        .map(str::trim)
        .filter(|value| !value.is_empty())
      else {
        return Vec::new();
      };
      vec![Inline::Image {
        url: url.to_string(),
        title: html_attribute_value(element, "title")
          .map(str::trim)
          .filter(|value| !value.is_empty())
          .map(ToString::to_string),
        alt: html_attribute_value(element, "alt")
          .map(str::trim)
          .unwrap_or_default()
          .to_string(),
        width: html_attribute_value(element, "width")
          .map(str::trim)
          .filter(|value| !value.is_empty())
          .map(ToString::to_string),
        height: html_attribute_value(element, "height")
          .map(str::trim)
          .filter(|value| !value.is_empty())
          .map(ToString::to_string),
        dark_url: None,
        light_url: None,
      }]
    }
    "a" => {
      let content = html_nodes_to_inlines(&element.children);
      let Some(url) = html_attribute_value(element, "href")
        .map(str::trim)
        .filter(|value| !value.is_empty())
      else {
        return content;
      };
      if content.is_empty() {
        return vec![Inline::Text(url.to_string())];
      }
      vec![Inline::Link {
        url: url.to_string(),
        title: html_attribute_value(element, "title")
          .map(str::trim)
          .filter(|value| !value.is_empty())
          .map(ToString::to_string),
        content,
      }]
    }
    "picture" => {
      let children = html_nodes_to_inlines(&element.children);
      let (picture_dark_url, picture_light_url) = html_picture_theme_urls(element);
      if let Some((url, alt, _, width, height, dark_url, light_url)) =
        children.iter().find_map(inline_image_data)
      {
        vec![Inline::Image {
          url,
          title: None,
          alt,
          width,
          height,
          dark_url: picture_dark_url.or(dark_url),
          light_url: picture_light_url.or(light_url),
        }]
      } else {
        children
      }
    }
    "source" => Vec::new(),
    "span" | "sub" | "sup" => html_nodes_to_inlines(&element.children),
    _ => html_nodes_to_inlines(&element.children),
  }
}

fn legacy_inlines_from_html_fragment(html: &str) -> Vec<Inline> {
  let mut inlines = Vec::new();
  let mut cursor = 0usize;
  let mut pending_space = false;

  while cursor < html.len() {
    let Some(rel_lt) = html[cursor..].find('<') else {
      push_html_text_segment(&mut inlines, &html[cursor..], &mut pending_space);
      break;
    };

    let lt = cursor + rel_lt;
    push_html_text_segment(&mut inlines, &html[cursor..lt], &mut pending_space);

    let Some(rel_gt) = html[lt..].find('>') else {
      push_html_text_segment(&mut inlines, &html[lt..], &mut pending_space);
      break;
    };

    let gt = lt + rel_gt + 1;
    let tag = &html[lt..gt];
    if let Some(image) = parse_inline_html_image(tag) {
      inlines.push(image);
    } else if is_html_line_break_tag(tag) {
      inlines.push(Inline::HardBreak);
      pending_space = false;
    }

    cursor = gt;
  }

  merge_adjacent_text(&inlines)
}

fn extract_html_attribute(tag: &str, name: &str) -> Option<String> {
  let lower = tag.to_ascii_lowercase();
  let pattern = format!("{name}=");
  let mut cursor = 0usize;

  while cursor < lower.len() {
    let Some(rel_start) = lower[cursor..].find(&pattern) else {
      break;
    };
    let start = cursor + rel_start;

    if start > 0 {
      let prev = lower.as_bytes()[start - 1];
      if prev.is_ascii_alphanumeric() || prev == b'-' || prev == b'_' {
        cursor = start + pattern.len();
        continue;
      }
    }

    let mut value_start = start + pattern.len();
    while value_start < tag.len() && tag.as_bytes()[value_start].is_ascii_whitespace() {
      value_start += 1;
    }
    if value_start >= tag.len() {
      return None;
    }

    let bytes = tag.as_bytes();
    let delimiter = bytes[value_start];
    if delimiter == b'"' || delimiter == b'\'' {
      let content_start = value_start + 1;
      let rest = &tag[content_start..];
      if let Some(close_rel) = rest.find(delimiter as char) {
        return Some(rest[..close_rel].to_string());
      }
      return Some(rest.to_string());
    }

    let mut value_end = value_start;
    while value_end < tag.len() {
      let ch = bytes[value_end];
      if ch.is_ascii_whitespace() || ch == b'>' || ch == b'/' {
        break;
      }
      value_end += 1;
    }
    return Some(tag[value_start..value_end].to_string());
  }

  None
}

fn list_item_from_node<'a>(node: &'a AstNode<'a>) -> Option<ListItem> {
  if !matches!(node.data.borrow().value, NodeValue::Item(_)) {
    return None;
  }
  let checked = node
    .children()
    .find_map(|child| match &child.data.borrow().value {
      NodeValue::TaskItem(marker) => Some(marker.is_some()),
      _ => None,
    });
  Some(ListItem {
    blocks: node.children().flat_map(blocks_from_node).collect(),
    checked,
  })
}

fn table_from_node<'a>(node: &'a AstNode<'a>) -> Table {
  let mut headers: Vec<Vec<Inline>> = Vec::new();
  let mut rows: Vec<Vec<Vec<Inline>>> = Vec::new();

  for row_node in node.children() {
    let is_header = match row_node.data.borrow().value {
      NodeValue::TableRow(is_header) => is_header,
      _ => continue,
    };
    let mut row_cells: Vec<Vec<Inline>> = Vec::new();
    for cell_node in row_node.children() {
      if !matches!(cell_node.data.borrow().value, NodeValue::TableCell) {
        continue;
      }
      row_cells.push(inlines_from_nodes(cell_node.children()));
    }
    if is_header {
      headers = row_cells;
    } else {
      rows.push(row_cells);
    }
  }

  Table { headers, rows }
}

fn inlines_from_nodes<'a>(nodes: impl Iterator<Item = &'a AstNode<'a>>) -> Vec<Inline> {
  let mut inlines = Vec::new();
  for node in nodes {
    match &node.data.borrow().value {
      NodeValue::Text(text) => inlines.push(Inline::Text(text.clone())),
      NodeValue::Code(code) => inlines.push(Inline::Code(code.literal.clone())),
      NodeValue::LineBreak => inlines.push(Inline::HardBreak),
      NodeValue::SoftBreak => inlines.push(Inline::SoftBreak),
      NodeValue::Strong => inlines.push(Inline::Strong(inlines_from_nodes(node.children()))),
      NodeValue::Emph => inlines.push(Inline::Emphasis(inlines_from_nodes(node.children()))),
      NodeValue::Strikethrough => {
        inlines.push(Inline::Strikethrough(inlines_from_nodes(node.children())))
      }
      NodeValue::Link(link) => inlines.push(Inline::Link {
        url: link.url.clone(),
        title: if link.title.is_empty() {
          None
        } else {
          Some(link.title.clone())
        },
        content: inlines_from_nodes(node.children()),
      }),
      NodeValue::Image(image) => {
        let alt = inline_to_plain_text(&inlines_from_nodes(node.children()));
        inlines.push(Inline::Image {
          url: image.url.clone(),
          title: if image.title.is_empty() {
            None
          } else {
            Some(image.title.clone())
          },
          alt,
          width: None,
          height: None,
          dark_url: None,
          light_url: None,
        });
      }
      NodeValue::HtmlInline(html) => {
        if let Some(nodes) = parse_html_fragment_nodes(html) {
          let parsed = html_nodes_to_inlines(&nodes);
          if !parsed.is_empty() {
            inlines.extend(parsed);
            continue;
          }
        }

        if let Some(image) = parse_inline_html_image(html) {
          inlines.push(image);
        } else if is_html_line_break_tag(html) {
          inlines.push(Inline::HardBreak);
        } else {
          let text = strip_html_tags(html);
          if !text.is_empty() {
            inlines.push(Inline::Text(text));
          }
        }
      }
      NodeValue::TaskItem(_) => {}
      _ => {
        let text = collect_text(node);
        if !text.is_empty() {
          inlines.push(Inline::Text(text));
        }
      }
    }
  }

  merge_adjacent_text(&inlines)
}

fn merge_adjacent_text(inlines: &[Inline]) -> Vec<Inline> {
  let mut merged = Vec::new();
  for inline in inlines {
    match (merged.last_mut(), inline) {
      (Some(Inline::Text(existing)), Inline::Text(new_text)) => {
        existing.push_str(new_text);
      }
      _ => merged.push(inline.clone()),
    }
  }
  merged
}

fn inline_to_plain_text(inlines: &[Inline]) -> String {
  let mut text = String::new();
  for inline in inlines {
    match inline {
      Inline::Text(value) => text.push_str(value),
      Inline::Code(value) => text.push_str(value),
      Inline::SoftBreak => text.push(' '),
      Inline::HardBreak => text.push('\n'),
      Inline::Strong(children) | Inline::Emphasis(children) | Inline::Strikethrough(children) => {
        text.push_str(&inline_to_plain_text(children))
      }
      Inline::Link { content, .. } => text.push_str(&inline_to_plain_text(content)),
      Inline::Image { alt, .. } => text.push_str(alt),
    }
  }
  text
}

fn collect_text<'a>(node: &'a AstNode<'a>) -> String {
  match &node.data.borrow().value {
    NodeValue::Text(text) => text.clone(),
    NodeValue::Code(code) => code.literal.clone(),
    NodeValue::Paragraph | NodeValue::Heading(_) => {
      inline_to_plain_text(&inlines_from_nodes(node.children()))
    }
    NodeValue::Link(link) => link.url.clone(),
    _ => String::new(),
  }
}

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
    assert_eq!(rendered, "#11 and #22 #33");
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
      Some((
        "https://img.shields.io/example.svg".to_string(),
        "badge".to_string(),
        Some("https://example.com".to_string()),
        Some("120px".to_string()),
        None,
        Some("https://img.shields.io/example-dark.svg".to_string()),
        Some("https://img.shields.io/example-light.svg".to_string()),
      ))
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

    let height = estimate_blocks_height_px(&blocks, 80, 20.0, 0);
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
      "    fn main() {\n        println!(\"ok\");\n    }"
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
      "    let x = 1 + 2;\nvalue . split_whitespace();"
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
    let (_, spans, _) = build_code_block_spans(&code);

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
    let indices = collect_indentation_dot_indices(text);

    assert_eq!(indices, vec![0, 1, 2, 3, 10, 11]);
  }

  #[test]
  fn collect_indentation_dot_indices_ignores_internal_spaces() {
    let text = "  first second third";
    let indices = collect_indentation_dot_indices(text);

    assert_eq!(indices, vec![0, 1]);
  }

  #[test]
  fn collect_indentation_dot_indices_ignores_blank_or_whitespace_only_lines() {
    let text = "  \n  valid\n    \n end";
    let indices = collect_indentation_dot_indices(text);

    assert_eq!(indices, vec![3, 4, 16]);
  }

  #[test]
  fn collect_indentation_dot_indices_handles_crlf() {
    let text = "  a\r\n    b\r\n  ";
    let indices = collect_indentation_dot_indices(text);

    assert_eq!(indices, vec![0, 1, 5, 6, 7, 8]);
  }

  #[test]
  fn collect_indentation_dot_indices_limits_render_count_for_large_input() {
    let text = (0..200)
      .map(|_| "                              line")
      .collect::<Vec<_>>()
      .join("\n");
    let indices = collect_indentation_dot_indices(text.as_str());

    assert!(!indices.is_empty());
    assert!(indices.len() <= MARKDOWN_CODE_INDENT_DOT_MAX_RENDER_COUNT);
  }

  #[test]
  fn collect_indentation_dot_indices_disables_for_very_large_text() {
    let text = format!(
      "{}code",
      " ".repeat(MARKDOWN_CODE_INDENT_DOT_DISABLE_ABOVE_TEXT_LEN + 1)
    );
    let indices = collect_indentation_dot_indices(text.as_str());

    assert!(indices.is_empty());
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
}
