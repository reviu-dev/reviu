use std::{
  collections::{HashMap, VecDeque},
  fs,
  hash::{Hash, Hasher, DefaultHasher},
  ops::Range,
  path::PathBuf,
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
  AnyElement, App, Bounds, ClipboardItem, CursorStyle, DispatchPhase, Element, ElementId,
  FontStyle, FontWeight, GlobalElementId, Hitbox, HitboxBehavior, Hsla, ImageCacheError,
  ImgResourceLoader, InspectorElementId, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent,
  MouseUpEvent, Pixels, RenderImage, Resource, SharedString, StrikethroughStyle, StyledText,
  TextRun, UnderlineStyle, Window, div, fill, img, point, prelude::*, px,
};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{ActiveTheme as _, Sizable as _, StyledExt as _, h_flex, v_flex};
use once_cell::sync::Lazy;
use reqwest::header::CONTENT_TYPE;
use syntax::{HighlightSpan, SyntaxHighlighter, SyntaxTheme, TokenType, languages};

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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeBlock {
  pub lang: Option<String>,
  pub value: String,
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
  },
  Code(String),
  SoftBreak,
  HardBreak,
  Strong(Vec<Inline>),
  Emphasis(Vec<Inline>),
  Strikethrough(Vec<Inline>),
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
const MARKDOWN_LIST_ITEM_GAP_PX: f32 = 4.0;
const MARKDOWN_INDENT_PER_LEVEL_PX: f32 = 12.0;
const MARKDOWN_CHAR_WIDTH_PX: f32 = 8.8;
const MARKDOWN_MIN_WRAP_COLUMNS: usize = 8;
const MARKDOWN_CODE_LINE_HEIGHT_SCALE: f32 = 0.95;
const MARKDOWN_CODE_BLOCK_PADDING_X_PX: f32 = 12.0;
const MARKDOWN_CODE_BLOCK_PADDING_TOP_PX: f32 = 8.0;
const MARKDOWN_CODE_BLOCK_PADDING_BOTTOM_PX: f32 = 4.0;
const MARKDOWN_CODE_BLOCK_MAX_HEIGHT_PX: f32 = 400.0;
const MARKDOWN_CODE_BLOCK_TEXT_SHIFT_X_PX: f32 = 2.0;
const MARKDOWN_CODE_BLOCK_LEADING_SPACE_RENDER_MULTIPLIER: usize = 2;
const MARKDOWN_CODE_BLOCK_TAB_WIDTH: usize = 4;
const MARKDOWN_CODE_INDENT_DOT_SIZE_PX: f32 = 2.0;
const MARKDOWN_CODE_INDENT_DOT_OPACITY: f32 = 0.45;
const MARKDOWN_CODE_INDENT_DOT_MIN_SPACING_PX: f32 = 5.0;
const MARKDOWN_CODE_INDENT_DOT_MAX_RENDER_COUNT: usize = 600;
const MARKDOWN_CODE_INDENT_DOT_DISABLE_ABOVE_TEXT_LEN: usize = 20_000;
const PARSED_MARKDOWN_CACHE_MAX_ENTRIES: usize = 256;
const PARSED_MARKDOWN_CACHE_MAX_SOURCE_LEN: usize = 100_000;
const MARKDOWN_CODE_BLOCK_VERTICAL_CHROME_PX: f32 =
  MARKDOWN_CODE_BLOCK_PADDING_TOP_PX + MARKDOWN_CODE_BLOCK_PADDING_BOTTOM_PX + 2.0;
static BADGE_IMAGE_SOURCE_CACHE: Lazy<Mutex<HashMap<String, BadgeResolveState>>> =
  Lazy::new(|| Mutex::new(HashMap::new()));
static PARSED_MARKDOWN_CACHE: Lazy<Mutex<ParsedMarkdownCache>> =
  Lazy::new(|| Mutex::new(ParsedMarkdownCache::default()));

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

#[derive(Default)]
struct ParsedMarkdownCache {
  entries: HashMap<Arc<str>, ParsedMarkdown>,
  lru_keys: VecDeque<Arc<str>>,
}

impl ParsedMarkdownCache {
  fn get(&mut self, source: &str) -> Option<ParsedMarkdown> {
    let parsed = self.entries.get(source).cloned()?;
    self.touch(source);
    Some(parsed)
  }

  fn insert(&mut self, source: Arc<str>, parsed: ParsedMarkdown) {
    if self.entries.contains_key(source.as_ref()) {
      self.touch(source.as_ref());
      return;
    }

    self.entries.insert(source.clone(), parsed);
    self.lru_keys.push_back(source);
    self.evict_excess();
  }

  fn touch(&mut self, source: &str) {
    let Some(ix) = self.lru_keys.iter().position(|key| key.as_ref() == source) else {
      return;
    };
    if let Some(key) = self.lru_keys.remove(ix) {
      self.lru_keys.push_back(key);
    }
  }

  fn evict_excess(&mut self) {
    while self.entries.len() > PARSED_MARKDOWN_CACHE_MAX_ENTRIES {
      let Some(oldest_key) = self.lru_keys.pop_front() else {
        break;
      };
      self.entries.remove(oldest_key.as_ref());
    }
  }
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

fn parse_markdown_for_render(source: &str) -> ParsedMarkdown {
  if source.len() > PARSED_MARKDOWN_CACHE_MAX_SOURCE_LEN {
    return parse_markdown(source);
  }

  if let Ok(mut cache) = PARSED_MARKDOWN_CACHE.lock()
    && let Some(parsed) = cache.get(source)
  {
    return parsed;
  }

  let parsed = parse_markdown(source);
  let cache_key: Arc<str> = Arc::from(source);

  if let Ok(mut cache) = PARSED_MARKDOWN_CACHE.lock() {
    if let Some(existing) = cache.get(source) {
      return existing;
    }
    cache.insert(cache_key, parsed.clone());
  }

  parsed
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
  let parsed = parse_markdown(source);
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
  root.children().flat_map(blocks_from_node).collect()
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

pub fn render_markdown(source: &str, options: &MarkdownRenderOptions, cx: &App) -> AnyElement {
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

  for block in blocks {
    container = container.child(render_block(block, options, indent, cx, ctx));
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

  let mut header_row = h_flex().bg(theme.accent);
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

  div()
    .overflow_x_scrollbar()
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
    && let Some(href) = extract_svg_image_href(&svg)
    && let Some(source) = resolve_badge_href(url, &href)
  {
    return Some(source);
  }

  Some(BadgeImageSource::Remote(url.to_string()))
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

fn inline_image_data(inline: &Inline) -> Option<(String, String)> {
  match inline {
    Inline::Image { url, alt, .. } => Some((url.clone(), alt.clone())),
    Inline::Link { content, .. }
    | Inline::Strong(content)
    | Inline::Emphasis(content)
    | Inline::Strikethrough(content) => {
      for child in content {
        if let Some(data) = inline_image_data(child) {
          return Some(data);
        }
      }
      None
    }
    _ => None,
  }
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

  for inline in inlines {
    if let Some((url, alt)) = inline_image_data(inline) {
      if !text_chunk.is_empty() {
        row = row.child(render_inline_text(&text_chunk, options, cx, ctx));
        text_chunk.clear();
      }

      let badge_label = if alt.is_empty() {
        "image".to_string()
      } else {
        alt
      };
      let badge_url = url.clone();
      row = row.child(
        img(move |window: &mut Window, cx: &mut App| {
          if let Some(source) = resolve_badge_image_source_async(&badge_url) {
            return load_badge_image_data(&source, window, cx);
          }

          window.request_animation_frame();
          None
        })
        .h(px(18.0))
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

fn render_inline_text(
  inlines: &[Inline],
  options: &MarkdownRenderOptions,
  _cx: &App,
  ctx: &mut RenderContext,
) -> AnyElement {
  let (text, spans, link_ranges) = build_spans(inlines);
  let text_id = ctx.next_text_id();

  SelectableText::new(
    text,
    spans,
    link_ranges,
    options.state.clone(),
    options.on_link.clone(),
    text_id,
    SelectableTextOptions {
      interactive: true,
      show_indentation_dots: false,
    },
  )
  .into_any_element()
}

fn render_inline_static(
  inlines: &[Inline],
  options: &MarkdownRenderOptions,
  _cx: &App,
  ctx: &mut RenderContext,
) -> AnyElement {
  let (text, spans, link_ranges) = build_spans(inlines);
  let text_id = ctx.next_text_id();

  SelectableText::new(
    text,
    spans,
    link_ranges,
    options.state.clone(),
    options.on_link.clone(),
    text_id,
    SelectableTextOptions {
      interactive: false,
      show_indentation_dots: false,
    },
  )
  .into_any_element()
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
    1 => container.text_xl().font_semibold(),
    2 => container.text_lg().font_semibold(),
    3 => container.text_base().font_medium(),
    _ => container.text_sm().font_medium(),
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

  div()
    .bg(theme.accent)
    .border_1()
    .border_color(theme.border)
    .rounded_md()
    .overflow_hidden()
    .child(
      div()
        .id(scroll_id)
        .max_h(px(MARKDOWN_CODE_BLOCK_MAX_HEIGHT_PX))
        .overflow_scroll()
        .on_scroll_wheel(|_, _, cx| {
          cx.stop_propagation();
        })
        .child(
          div()
            .px(px(MARKDOWN_CODE_BLOCK_PADDING_X_PX))
            .pt(px(MARKDOWN_CODE_BLOCK_PADDING_TOP_PX))
            .pb(px(MARKDOWN_CODE_BLOCK_PADDING_BOTTOM_PX))
            .whitespace_nowrap()
            .child(
              div()
                .pl(px(MARKDOWN_CODE_BLOCK_TEXT_SHIFT_X_PX))
                .text_sm()
                .text_color(theme.foreground)
                .child(content),
            ),
        ),
    )
    .into_any_element()
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

fn build_spans(inlines: &[Inline]) -> (SharedString, Vec<InlineSpan>, Vec<LinkRange>) {
  let mut builder = SpanBuilder::default();
  builder.push_inlines(inlines, InlineStyle::default(), None);
  builder.finish()
}

#[derive(Default)]
struct SpanBuilder {
  text: String,
  spans: Vec<InlineSpan>,
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

    let background_color = if span.style.code {
      Some(theme.accent)
    } else {
      None
    };

    runs.push(TextRun {
      len: span.range.end.saturating_sub(span.range.start),
      font,
      color,
      background_color,
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
      } else {
        vec![Block::Paragraph(vec![Inline::Text(html.literal.clone())])]
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
  Some(Inline::Image { url, title, alt })
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
        });
      }
      NodeValue::HtmlInline(html) => {
        if let Some(image) = parse_inline_html_image(html) {
          inlines.push(image);
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

    assert!(cache.entries.len() <= PARSED_MARKDOWN_CACHE_MAX_ENTRIES);
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
