use std::{
  collections::HashMap,
  ops::Range,
  path::PathBuf,
  sync::{
    Arc, Mutex, MutexGuard,
    atomic::{AtomicUsize, Ordering},
  },
};

use syntax::TokenType;

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

/// Context used by the viewer to render `` ```suggestion `` code fences as a
/// GitHub-style diff (the lines currently on the anchored side of the comment
/// on top, the suggestion content underneath).
#[derive(Clone, Debug)]
pub struct SuggestionContext {
  pub original_start_line: Option<usize>,
  pub suggested_start_line: Option<usize>,
  pub original_lines: Vec<String>,
  pub path: Arc<str>,
}

#[derive(Clone, Debug)]
pub struct SuggestionActionContext {
  pub path: Arc<str>,
  pub original_start_line: Option<usize>,
  pub original_lines: Vec<String>,
  pub suggested_lines: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GithubDiffLineKind {
  Context,
  Added,
  Removed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GithubDiffLine {
  pub old_line: Option<usize>,
  pub new_line: Option<usize>,
  pub content: Arc<str>,
  pub kind: GithubDiffLineKind,
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
  /// Full file content for accurate syntax highlighting.
  /// When provided, the highlighter runs on the entire file and extracts
  /// spans for the visible lines, giving tree-sitter proper context.
  pub full_content: Option<Arc<str>>,
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
pub(crate) struct HtmlElement {
  pub(crate) tag: String,
  pub(crate) attrs: Vec<HtmlAttribute>,
  pub(crate) children: Vec<HtmlNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HtmlAttribute {
  pub(crate) name: String,
  pub(crate) value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HtmlNode {
  Element(HtmlElement),
  Text(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct InlineStyle {
  pub(crate) bold: bool,
  pub(crate) italic: bool,
  pub(crate) strike: bool,
  pub(crate) code: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InlineBackground {
  DiffWordAdded,
  DiffWordRemoved,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InlineSpan {
  pub(crate) range: Range<usize>,
  pub(crate) style: InlineStyle,
  pub(crate) link: Option<Arc<str>>,
  pub(crate) syntax_token: Option<TokenType>,
  pub(crate) background: Option<InlineBackground>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LinkRange {
  pub(crate) range: Range<usize>,
  pub(crate) url: Arc<str>,
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

#[derive(Clone)]
pub struct MarkdownRenderState {
  pub(crate) instance_id: usize,
  pub(crate) details_open: Arc<Mutex<HashMap<usize, bool>>>,
  pub(crate) selection_registry: selectable_text::SelectionRegistry,
}

impl Default for MarkdownRenderState {
  fn default() -> Self {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(1);
    Self {
      instance_id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
      details_open: Arc::new(Mutex::new(HashMap::new())),
      selection_registry: selectable_text::SelectionRegistry::new(),
    }
  }
}

impl MarkdownRenderState {
  pub fn new() -> Self {
    Self::default()
  }

  pub fn with_selection_registry(registry: selectable_text::SelectionRegistry) -> Self {
    Self {
      selection_registry: registry,
      ..Self::default()
    }
  }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SelectionRange {
  pub(crate) start: usize,
  pub(crate) end: usize,
}

impl SelectionRange {
  pub(crate) fn is_empty(self) -> bool {
    self.start == self.end
  }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SelectionState {
  pub(crate) anchor: Option<usize>,
  pub(crate) range: SelectionRange,
  pub(crate) dragging: bool,
}

pub(crate) fn clamp_to_char_boundary(text: &str, index: usize) -> usize {
  let mut index = index.min(text.len());
  while index > 0 && !text.is_char_boundary(index) {
    index -= 1;
  }
  index
}

#[derive(Clone, Debug)]
pub(crate) enum BadgeImageSource {
  Remote(String),
  Local(PathBuf),
}

#[derive(Clone, Debug)]
pub(crate) enum BadgeResolveState {
  Pending,
  Ready(BadgeImageSource),
  Failed,
}

#[derive(Debug)]
pub(crate) enum Segment {
  Markdown(String),
  Details {
    summary: Option<String>,
    body: String,
    open: bool,
  },
}

#[derive(Clone)]
pub struct ParsedMarkdown {
  pub(crate) blocks: Arc<Vec<Block>>,
}

// A panic while a cache lock is held must not poison every later render.
pub(crate) fn lock_ignoring_poison<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
  mutex
    .lock()
    .unwrap_or_else(|poisoned| poisoned.into_inner())
}
