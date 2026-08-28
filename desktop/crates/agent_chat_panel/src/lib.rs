mod ansi;
mod code_block;
mod code_lines;
mod store;
mod transcript;
use transcript::*;
mod tool_normalization;
use tool_normalization::*;
mod prompt;
use prompt::*;
mod render;
use render::*;
mod session_controls;
use session_controls::*;
mod diff;
mod events;
mod mention;
mod message_sanitizer;
mod persistence;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::diff::{
  DiffLineKind, DiffSummary, InlineSpan, InlineSpanKind, MAX_DIFF_LINES_COLLAPSED,
  MAX_TOOL_OUTPUT_LINES_COLLAPSED, backfill_legacy_line_numbers, extract_diffs,
  extract_outputs_with_fallback, extract_terminals,
};
use crate::mention::{DiffMention, MentionCandidate, MentionTrigger, ResolvedMention};
use crate::message_sanitizer::sanitize_agent_markdown;
use crate::persistence::{
  CONVERSATION_FORMAT_VERSION, LoadedConversation, new_conversation_meta, now_secs, preview_of,
  truncate_title,
};
pub use crate::persistence::{ConversationMeta, WorktreeBinding, state_dir_for_repo};
pub use crate::store::ConversationStore;
use crate::store::SaveRequest;
use agent_acp::{
  AgentEvent, AgentSession, AuthMethodInfo, BackendAvailability, BackendConfig,
  PermissionOptionKind, PermissionPrompt,
};
use agent_client_protocol::schema::{
  ContentBlock, EmbeddedResource, EmbeddedResourceResource, Plan, PlanEntryPriority,
  PlanEntryStatus, ResourceLink, SessionInfoUpdate, TextContent, TextResourceContents, ToolCall,
  ToolCallContent, ToolCallId, ToolCallStatus, ToolCallUpdate, ToolKind,
};
use agent_client_protocol::schema::{
  ModelId, ModelInfo, SessionConfigId, SessionConfigKind, SessionConfigOption,
  SessionConfigOptionCategory, SessionConfigOptionValue, SessionConfigSelectOptions,
  SessionConfigValueId, SessionMode, SessionModeId,
};
use agent_registry::{AgentId, Registry, RegistryAgent};
use futures::future::BoxFuture;
use gpui::Anchor;
use gpui::{
  AnyElement, App, Context, Empty, Entity, EntityInputHandler as _, FocusHandle, Focusable, Font,
  FontStyle, FontWeight, Hsla, IntoElement, MouseButton, ParentElement, Render, SharedString,
  Styled, Task, TextRun, Window, deferred, div, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, ColorName, Disableable as _, IconName, Selectable as _, Sizable as _,
  button::{Button, ButtonVariants as _},
  clipboard::Clipboard,
  h_flex,
  input::{self, InputEvent, Textarea, TextareaState},
  menu::{DropdownMenu as _, PopupMenuItem},
  scroll::ScrollableElement as _,
  skeleton::Skeleton,
  tag::Tag,
  text::{TextView, TextViewStyle},
  v_flex,
};
use syntax::{HighlightSpan, SyntaxHighlighter, SyntaxTheme, highlights_to_text_runs, languages};
use ui::{StatusThemeExt as _, UiIconName};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum ChatRole {
  User,
  Agent,
  System,
  /// Local review comments sent as a batch; rendered as a structured card.
  ReviewExport,
}

/// A session config select flattened for the composer trigger.
#[derive(Clone)]
struct ConfigSelector {
  id: SessionConfigId,
  name: SharedString,
  current_value: SessionConfigValueId,
  current_label: String,
  values: Vec<(SessionConfigValueId, String, Option<String>)>,
}

#[derive(Clone, Debug)]
struct PendingModelSelection {
  model_id: ModelId,
  previous_model_id: Option<ModelId>,
  generation: u64,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct ChatMessage {
  role: ChatRole,
  text: String,
  /// Number of images attached when the message was sent.
  #[serde(default)]
  images: usize,
  /// The attached images themselves, for this process only: conversations
  /// reload with the count badge instead of megabytes of pixels.
  #[serde(skip)]
  image_data: Vec<std::sync::Arc<gpui::Image>>,
}

/// Seeded per conversation so two sessions never show the same word at once.
const WORKING_WORDS: [&str; 20] = [
  "Thinking",
  "Working",
  "Digging",
  "Reading",
  "Tracing",
  "Weighing",
  "Drafting",
  "Wiring",
  "Shaping",
  "Threading",
  "Piecing",
  "Combing",
  "Chewing",
  "Wrangling",
  "Mulling",
  "Parsing",
  "Charting",
  "Hunting",
  "Stitching",
  "Rummaging",
];
const WORKING_WORD_ROTATE_SECS: u64 = 7;

fn working_word(seed: u64, elapsed_secs: u64) -> &'static str {
  let step = elapsed_secs / WORKING_WORD_ROTATE_SECS;
  WORKING_WORDS[(seed.wrapping_add(step) % WORKING_WORDS.len() as u64) as usize]
}

fn working_word_seed(conversation_id: &str) -> u64 {
  let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
  for byte in conversation_id.as_bytes() {
    hash ^= u64::from(*byte);
    hash = hash.wrapping_mul(0x1000_0000_01b3);
  }
  hash
}

fn review_export_label(text: &str) -> String {
  let count = text
    .lines()
    .filter(|line| line.starts_with("### "))
    .count()
    .max(1);
  if count == 1 {
    "1 review comment".to_string()
  } else {
    format!("{count} review comments")
  }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct ToolCallView {
  #[allow(dead_code)]
  id: ToolCallId,
  title: String,
  kind: ToolKind,
  status: ToolCallStatus,
  #[serde(default)]
  tool_name: Option<String>,
  locations: Vec<(PathBuf, Option<u32>)>,
  #[serde(default)]
  diffs: Vec<DiffSummary>,
  #[serde(default)]
  outputs: Vec<ToolOutput>,
  /// Ids of terminals embedded in this call; their live state is in the store.
  #[serde(default)]
  terminals: Vec<String>,
  /// First file line of a Read's output, resolved from the location line or
  /// the tool's raw input offset; drives the number gutter.
  #[serde(default)]
  read_start_line: Option<u32>,
  /// Fingerprint of the content that produced diffs/outputs/spans; a re-sent
  /// call with identical content skips the diff and highlight recompute.
  #[serde(skip)]
  content_fp: u64,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct ToolOutput {
  pub text: String,
  #[serde(default)]
  pub start_line: Option<u32>,
  #[serde(skip)]
  pub expanded: bool,
  #[serde(skip)]
  pub syntax_spans: Vec<HighlightSpan>,
}

const READ_SNAPSHOT_MAX_BYTES: u64 = 256 * 1024;
const READ_SNAPSHOT_MAX_LINES: usize = 400;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct PermissionItem {
  prompt: PermissionPrompt,
  detail: PermissionDetail,
  resolved: Option<String>,
  /// Answered by the auto-approve toggle, not by a click.
  #[serde(default)]
  auto: bool,
}

/// What the user is being asked to approve, extracted once when the prompt
/// arrives. The full diff already lives on the tool item above the card, so
/// edits show per-file counts only.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
struct PermissionDetail {
  invocation: Option<String>,
  path: Option<String>,
  diff_stats: Vec<(String, u32, u32)>,
}

fn path_inside_root(path: &Path, root: &Path) -> Option<PathBuf> {
  let candidate = if path.is_absolute() {
    path.to_path_buf()
  } else {
    root.join(path)
  };
  let root = root.canonicalize().ok()?;
  let candidate = candidate.canonicalize().ok()?;
  candidate.starts_with(&root).then_some(candidate)
}

fn byte_offset_for_line(text: &str, line_number: u32) -> Option<usize> {
  if line_number <= 1 {
    return Some(0);
  }
  let mut current_line = 1_u32;
  for (byte_offset, byte) in text.bytes().enumerate() {
    if byte == b'\n' {
      current_line = current_line.saturating_add(1);
      if current_line == line_number {
        return Some(byte_offset + 1);
      }
    }
  }
  None
}

fn local_read_snapshot(cwd: &Path, path: &Path, start_line: Option<u32>) -> Option<String> {
  let path = path_inside_root(path, cwd)?;
  let metadata = std::fs::metadata(&path).ok()?;
  if !metadata.is_file() || metadata.len() > READ_SNAPSHOT_MAX_BYTES {
    return None;
  }
  let text = std::fs::read_to_string(path).ok()?;
  if text.contains('\0') {
    return None;
  }
  let start = byte_offset_for_line(&text, start_line.unwrap_or(1))?;
  let snapshot = text[start..].to_string();
  if snapshot.is_empty() || snapshot.lines().count() > READ_SNAPSHOT_MAX_LINES {
    return None;
  }
  Some(snapshot)
}

fn read_snapshot_content(text: String) -> ToolCallContent {
  ToolCallContent::from(ContentBlock::Text(TextContent::new(text)))
}

fn read_locations_for_call(call: &ToolCall) -> Vec<(PathBuf, Option<u32>)> {
  call
    .locations
    .iter()
    .map(|location| (location.path.clone(), location.line))
    .collect()
}

fn fill_tool_call_read_snapshot(call: &mut ToolCall, cwd: &Path) {
  if !matches!(call.kind, ToolKind::Read) || !call.content.is_empty() || call.raw_output.is_some() {
    return;
  }
  let locations = read_locations_for_call(call);
  let Some((path, _)) = locations.first() else {
    return;
  };
  let start_line = read_tool_start_line(&ToolKind::Read, &locations, call.raw_input.as_ref());
  if let Some(snapshot) = local_read_snapshot(cwd, path, start_line) {
    call.content = vec![read_snapshot_content(snapshot)];
  }
}

fn fill_tool_update_read_snapshot(
  update: &mut ToolCallUpdate,
  existing: Option<&ToolCallView>,
  cwd: &Path,
) {
  let content_is_empty = update
    .fields
    .content
    .as_ref()
    .is_none_or(|content| content.is_empty());
  if !content_is_empty || update.fields.raw_output.is_some() {
    return;
  }
  if existing.is_some_and(|view| !view.outputs.is_empty()) {
    return;
  }
  let kind = update
    .fields
    .kind
    .or_else(|| existing.map(|view| view.kind))
    .unwrap_or(ToolKind::Other);
  if !matches!(kind, ToolKind::Read) {
    return;
  }
  let locations = update
    .fields
    .locations
    .as_ref()
    .map(|locations| {
      locations
        .iter()
        .map(|location| (location.path.clone(), location.line))
        .collect::<Vec<_>>()
    })
    .or_else(|| existing.map(|view| view.locations.clone()))
    .unwrap_or_default();
  let Some((path, _)) = locations.first() else {
    return;
  };
  let start_line = read_tool_start_line(&kind, &locations, update.fields.raw_input.as_ref())
    .or_else(|| existing.and_then(|view| view.read_start_line));
  if let Some(snapshot) = local_read_snapshot(cwd, path, start_line) {
    update.fields.content = Some(vec![read_snapshot_content(snapshot)]);
  }
}

fn permission_detail(
  update: &agent_client_protocol::schema::ToolCallUpdate,
  cwd: &std::path::Path,
) -> PermissionDetail {
  let fields = &update.fields;
  let raw = fields.raw_input.as_ref();
  let raw_str = |key: &str| {
    raw
      .and_then(|v| v.get(key))
      .and_then(|v| v.as_str())
      .map(str::to_string)
  };
  let pretty_raw = || {
    raw
      .filter(|v| !v.is_null() && *v != &serde_json::json!({}))
      .and_then(|v| serde_json::to_string_pretty(v).ok())
      .map(|s| truncate_chars(&s, 1000))
  };
  let invocation = match fields.kind {
    Some(ToolKind::Execute) => raw_str("command").or_else(pretty_raw),
    Some(ToolKind::Fetch) => raw_str("url").or_else(pretty_raw),
    Some(
      ToolKind::Read | ToolKind::Edit | ToolKind::Delete | ToolKind::Move | ToolKind::Search,
    ) => None,
    _ => pretty_raw(),
  };
  let path = fields
    .locations
    .as_ref()
    .and_then(|locs| locs.first())
    .map(|loc| match loc.line {
      Some(line) => format!("{} (line {line})", loc.path.display()),
      None => loc.path.display().to_string(),
    });
  let diff_stats = fields
    .content
    .as_ref()
    .map(|content| {
      extract_diffs(content, cwd)
        .into_iter()
        .map(|d| (d.path, d.added, d.removed))
        .collect()
    })
    .unwrap_or_default();
  PermissionDetail {
    invocation,
    path,
    diff_stats,
  }
}

fn truncate_chars(text: &str, max: usize) -> String {
  if text.chars().count() <= max {
    return text.to_string();
  }
  let cut: String = text.chars().take(max).collect();
  format!("{cut}…")
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct PlanView {
  entries: Vec<PlanEntryView>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct PlanEntryView {
  content: String,
  priority: PlanEntryPriorityView,
  status: PlanEntryStatusView,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum PlanEntryPriorityView {
  Low,
  Medium,
  High,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum PlanEntryStatusView {
  Pending,
  InProgress,
  Completed,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct ThoughtView {
  text: String,
  #[serde(default = "default_thought_collapsed")]
  collapsed: bool,
}

fn default_thought_collapsed() -> bool {
  true
}

#[derive(Clone, Debug)]
enum ChatItem {
  Message(ChatMessage),
  Tool(ToolCallView),
  Permission(Box<PermissionItem>),
  Plan(PlanView),
  Thought(ThoughtView),
  Checkpoint(CheckpointMarker),
  TurnSummary(TurnSummaryView),
}

/// Working-tree snapshot taken before the prompt that follows it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct CheckpointMarker {
  ref_name: String,
  created_at_secs: u64,
}

/// Aggregated edits of one turn: one row per file, totals in the header.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct TurnSummaryView {
  files: Vec<TurnFileStat>,
  /// Checkpoint guarding the turn; enables Undo.
  checkpoint_ref: Option<String>,
  /// The turn's file changes were reverted; the transcript stays.
  #[serde(default)]
  undone: bool,
  /// How long the turn ran, stamped when the card is appended.
  #[serde(default)]
  duration_secs: Option<u64>,
  #[serde(skip)]
  expanded: bool,
  /// The turn's folded work (thoughts, tools, interim prose) is shown.
  #[serde(skip)]
  work_expanded: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct TurnFileStat {
  path: String,
  added: u32,
  removed: u32,
}

/// File rows shown on a collapsed turn summary; beyond this, an expander.
const TURN_SUMMARY_COLLAPSED_FILES: usize = 3;

/// Gap kept above the runway-held prompt so it never glues to the header.
const RUNWAY_TOP_MARGIN_PX: f32 = 16.0;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
enum PersistedChatItem {
  Message(ChatMessage),
  Tool(ToolCallView),
  Plan(PlanView),
  Thought(ThoughtView),
  Checkpoint(CheckpointMarker),
  Permission(Box<PermissionItem>),
  TurnSummary(TurnSummaryView),
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PersistedConversation {
  /// 0 = legacy tag-less files; readers dispatch on this before parsing moves.
  #[serde(default)]
  version: u32,
  meta: ConversationMeta,
  items: Vec<PersistedChatItem>,
  /// Tool-group expand/collapse pins, keyed by the group's first tool id.
  #[serde(default)]
  group_pins: HashMap<String, bool>,
  #[serde(default)]
  auto_approve: bool,
}

#[derive(Clone, Copy, Debug)]
enum ExtraBeforeKind {
  MissingBinary,
  Auth,
}

#[derive(Clone, Copy, Debug)]
enum ExtraAfterKind {
  Generating,
  Error,
}

enum Status {
  Connecting,
  Ready,
  Error(String),
  MissingBinary { command: String, hint: String },
}

#[cfg(any(test, feature = "test-support"))]
static BACKEND_COMMAND_OVERRIDE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Points every new panel at a custom ACP agent binary (tests and the driver).
#[cfg(any(test, feature = "test-support"))]
pub fn set_backend_command_override(command: Option<String>) {
  *BACKEND_COMMAND_OVERRIDE
    .lock()
    .expect("lock backend command override") = command;
}

/// Registry ids we ship a brand icon for. These stay embedded so the agents
/// people use most keep their mark with no cache and no network.
fn embedded_backend_icon(id: &AgentId) -> Option<UiIconName> {
  match id.as_str() {
    "claude-acp" => Some(UiIconName::Claude),
    "codex-acp" => Some(UiIconName::OpenAi),
    "pi-acp" => Some(UiIconName::Pi),
    _ => None,
  }
}

/// Where an agent's icon comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BackendIconSource {
  /// A brand mark compiled into the binary.
  Embedded(UiIconName),
  /// An asset path fetched from the registry into the icon cache.
  Registry(String),
  /// Nothing better available.
  Generic,
}

pub(crate) fn backend_icon_source(id: &AgentId, has_cached_icon: bool) -> BackendIconSource {
  if let Some(embedded) = embedded_backend_icon(id) {
    return BackendIconSource::Embedded(embedded);
  }
  match agent_registry::icon_asset_path(id.as_str()) {
    Some(path) if has_cached_icon => BackendIconSource::Registry(path),
    _ => BackendIconSource::Generic,
  }
}

/// The agent's icon: the one fetched from the registry when it is cached, the
/// embedded brand mark otherwise, and a generic mark as a last resort.
pub fn backend_icon(id: &AgentId) -> gpui_component::Icon {
  match backend_icon_source(id, agent_registry::has_cached_icon(id.as_str())) {
    BackendIconSource::Embedded(icon) => gpui_component::Icon::new(icon),
    BackendIconSource::Registry(path) => gpui_component::Icon::empty().path(path),
    BackendIconSource::Generic => gpui_component::Icon::new(UiIconName::Sparkles),
  }
}

/// The registry entry an id resolves to, or the default agent when the id is
/// gone from the registry (renamed, withdrawn, or a stale saved choice).
pub fn resolve_agent(registry: &Registry, id: &AgentId) -> Option<AgentId> {
  if registry.get(id).is_some_and(|agent| agent.is_runnable()) {
    return Some(id.clone());
  }
  registry
    .get(&default_agent_id())
    .filter(|agent| agent.is_runnable())
    .map(|agent| agent.id.clone())
    .or_else(|| registry.runnable().first().map(|agent| agent.id.clone()))
}

pub fn default_agent_id() -> AgentId {
  AgentId::new("claude-acp")
}

/// Build the launch descriptor for a registry agent.
pub fn backend_config_for(agent: &RegistryAgent) -> BackendConfig {
  let (command, args) = agent
    .command()
    .unwrap_or_else(|| (String::new(), Vec::new()));
  BackendConfig::new(agent.name.clone(), command, args)
    .env(agent.env().to_vec())
    .cli_executable(agent.required_cli())
    .install_hint(agent.install_hint())
}

fn resolve_backend_config(id: &AgentId) -> BackendConfig {
  let registry = agent_registry::global();
  let config = registry.get(id).map(backend_config_for).unwrap_or_else(|| {
    // The saved agent is gone from the registry: name the problem rather than
    // failing on spawn with an empty command.
    BackendConfig::new(id.to_string(), id.to_string(), Vec::new()).install_hint(format!(
      "`{id}` is no longer in the agent registry. Pick another agent."
    ))
  });
  #[cfg(any(test, feature = "test-support"))]
  if let Some(command) = BACKEND_COMMAND_OVERRIDE
    .lock()
    .expect("lock backend command override")
    .clone()
  {
    return config.with_command(command);
  }
  config
}

const MENTION_MENU_MAX_ITEMS: usize = 10;
const SLASH_MENU_MAX_ITEMS: usize = 10;

/// The "/command" token under the cursor: only a token opening the message
/// counts (a "/" after any text is a path, not a command), running to the
/// first whitespace.
fn slash_token_at_cursor(text: &str, cursor: usize) -> Option<String> {
  if !text.starts_with('/') || cursor == 0 || cursor > text.len() || !text.is_char_boundary(cursor)
  {
    return None;
  }
  let token_end = text.find(char::is_whitespace).unwrap_or(text.len());
  if cursor > token_end {
    return None;
  }
  Some(text[1..token_end].to_string())
}
const MAX_REPO_FILES: usize = 20_000;
/// Caps the conversation and the composer to a readable measure on wide windows.
const CONVERSATION_COLUMN_MAX_WIDTH_PX: f32 = 720.0;
const CONVERSATION_BOTTOM_FADE_PX: f32 = 48.0;
const THINKING_PEEK_MAX_HEIGHT_PX: f32 = 180.0;
const THINKING_PEEK_TAIL_BYTES: usize = 4096;
const THINKING_PEEK_TAIL_LINES: usize = 12;

/// Tail of the streaming thought bounded in bytes then lines, plus whether
/// older content was dropped. Bounding keeps the per-chunk markdown re-parse
/// cost flat however long the think runs.
fn thought_peek_tail(text: &str) -> (String, bool) {
  let text = text.trim_end();
  let mut start = text.len().saturating_sub(THINKING_PEEK_TAIL_BYTES);
  while !text.is_char_boundary(start) {
    start += 1;
  }
  let mut slice = &text[start..];
  let mut truncated = start > 0;
  let lines = slice.lines().count();
  if lines > THINKING_PEEK_TAIL_LINES {
    let mut to_drop = lines - THINKING_PEEK_TAIL_LINES;
    for (i, b) in slice.bytes().enumerate() {
      if b == b'\n' {
        to_drop -= 1;
        if to_drop == 0 {
          slice = &slice[i + 1..];
          truncated = true;
          break;
        }
      }
    }
  } else if truncated {
    // Snap the byte cut to a line start so the peek opens on a whole line.
    if let Some(nl) = slice.find('\n')
      && nl + 1 < slice.len()
    {
      slice = &slice[nl + 1..];
    }
  }
  // Plain-text peek: markdown markers show literally, and runs of blank
  // lines open holes in a box meant to be a glimpse.
  let mut cleaned = String::with_capacity(slice.len());
  let mut last_blank = true;
  for line in slice.lines() {
    let line = line.replace("**", "").replace('`', "");
    let blank = line.trim().is_empty();
    if blank && last_blank {
      continue;
    }
    if !cleaned.is_empty() {
      cleaned.push('\n');
    }
    cleaned.push_str(line.trim_end());
    last_blank = blank;
  }
  (cleaned.trim().to_string(), truncated)
}

/// Code selected in the Git diff view, pushed in to attach as `@selection` context.
#[derive(Clone, Debug)]
struct SelectionContext {
  path: String,
  text: String,
}

/// One turn at a time per CHECKOUT: two agents editing the same working tree
/// would corrupt each other's work. Sessions in their own worktrees hold
/// different keys, so they run in parallel.
#[derive(Clone, Default)]
pub struct TurnGate(std::rc::Rc<std::cell::RefCell<HashMap<PathBuf, String>>>);

impl TurnGate {
  pub fn new() -> Self {
    Self::default()
  }

  /// Keys are canonicalized so a worktree reached through a symlink still
  /// collides with itself.
  fn key(checkout: &Path) -> PathBuf {
    std::fs::canonicalize(checkout).unwrap_or_else(|_| checkout.to_path_buf())
  }

  fn can_start(&self, checkout: &Path, conversation_id: &str) -> bool {
    match self.0.borrow().get(&Self::key(checkout)) {
      None => true,
      Some(holder) => holder == conversation_id,
    }
  }

  fn acquire(&self, checkout: &Path, conversation_id: &str) {
    self
      .0
      .borrow_mut()
      .insert(Self::key(checkout), conversation_id.to_string());
  }

  fn release(&self, checkout: &Path, conversation_id: &str) {
    let mut holders = self.0.borrow_mut();
    let key = Self::key(checkout);
    if holders
      .get(&key)
      .is_some_and(|holder| holder == conversation_id)
    {
      holders.remove(&key);
    }
  }
}

/// Emitted so the host can act on panel interactions.
#[derive(Clone, Debug)]
pub enum AgentChatPanelEvent {
  /// User clicked a tool-call file location; open it in the diff view.
  OpenPath { path: PathBuf, line: Option<u32> },
  /// A prompt was dispatched; the host may snapshot the working tree.
  TurnStarted,
  /// The agent finished a turn; the working tree may have changed. `completed`
  /// is false when the user stopped it or it failed, so nothing was done.
  TurnFinished { completed: bool },
  /// User asked to roll back to a checkpoint marker.
  RollbackRequested { ref_name: String },
  /// User asked to revert one turn's file changes, keeping the transcript.
  UndoTurnRequested { ref_name: String },
  /// The agent is waiting on a permission answer.
  PermissionRequested,
  /// The conversation earned its title (first user message), once.
  TitleSettled { title: String },
  /// The turn died on an error; `message` is the short human line.
  TurnFailed { message: String },
  /// User asked the host to hide the chat pane.
  CloseRequested,
}

impl gpui::EventEmitter<AgentChatPanelEvent> for AgentChatPanel {}

impl Drop for AgentChatPanel {
  fn drop(&mut self) {
    // A panel deleted mid-turn must not leave the shared gate held forever.
    self.turn_gate.release(&self.cwd, &self.current_conv.id);
  }
}

pub struct AgentChatPanel {
  backend_kind: AgentId,
  backend: BackendConfig,
  /// The repository this session belongs to; `cwd` may be one of its worktrees.
  repo_root: PathBuf,
  cwd: PathBuf,
  status: Status,
  items: Vec<ChatItem>,
  repo_files: Arc<Vec<String>>,
  active_selection: Option<SelectionContext>,
  mention_selected_ix: usize,
  mention_dismissed: Option<MentionTrigger>,
  tool_index: HashMap<ToolCallId, usize>,
  pending_agent: String,
  pending_thought: String,
  /// Messages typed during a turn, drained oldest-first when it ends cleanly.
  queued_prompts: Vec<String>,
  composer_history_index: Option<usize>,
  composer_history_draft: Option<String>,
  /// Whether the connected agent accepts image blocks in prompts.
  supports_images: bool,
  /// Whether the connected agent can take a message mid-turn.
  supports_steering: bool,
  /// Images staged for the next prompt (pasted or dropped).
  staged_images: Vec<std::sync::Arc<gpui::Image>>,
  /// Incremental markdown state for the streaming reply: chunks append via
  /// push_str so a chunk costs O(delta), not a full document re-parse.
  pending_md_state: Option<gpui::Entity<gpui_component::text::TextViewState>>,
  /// Expand/collapse pins per tool group, keyed by the group's first tool id.
  tool_group_pins: HashMap<ToolCallId, bool>,
  /// Permission requests are answered with their allow option automatically.
  auto_approve: bool,
  /// Live terminals of the connected session, for rendering command output.
  terminal_store: Option<Arc<agent_acp::TerminalStore>>,
  _terminal_task: Option<Task<()>>,
  /// Runway: the sent prompt holds at the viewport top while the reply
  /// streams into reserved space below, instead of tail-scrolling.
  runway_active: bool,
  runway_end_space: f32,
  runway_following: bool,
  /// Derived each frame; held through unmeasured frames to avoid blinking.
  show_jump_pill: bool,
  runway_measure_pending: bool,
  /// A reader scroll landed since the last frame; ListState can't be
  /// re-borrowed inside its own scroll handler, so the check runs at render.
  reader_scrolled: bool,
  /// Same deferral for persisting the reading position.
  scroll_save_pending: bool,
  /// Slash commands advertised by the agent, latest update wins.
  available_commands: Vec<agent_client_protocol::schema::AvailableCommand>,
  /// Item index of the user message being edited, with its inline editor.
  editing_message: Option<usize>,
  edit_input: Option<Entity<TextareaState>>,
  edit_input_sub: Option<gpui::Subscription>,
  /// Armed on Send of an edit: (checkpoint ref, new text). Consumed only by
  /// the truncate for that ref, so a failed rollback never resubmits.
  pending_edit_resubmit: Option<(String, String)>,
  /// Set by the matching truncate; dispatched once the session reconnects.
  resubmit_after_connect: Option<String>,
  slash_selected_ix: usize,
  slash_dismissed: Option<String>,
  session: Option<Arc<AgentSession>>,
  input: Entity<TextareaState>,
  in_flight: bool,
  turn_started_at: Option<std::time::Instant>,
  _tick_task: Option<Task<()>>,
  messages_list: gpui::ListState,
  usage: Option<(u64, u64)>,
  agent_version: Option<String>,
  auth_methods: Vec<AuthMethodInfo>,
  auth_required: bool,
  store: Option<Entity<ConversationStore>>,
  /// Conversation still hydrating from disk after this panel was created.
  loading_conversation: Option<String>,
  current_conv: ConversationMeta,
  /// The shown panel writes the active pointer; background ones must not.
  is_active: bool,
  /// TitleSettled fired; one title announcement per panel.
  title_announced: bool,
  /// The last turn ended in an error; cleared when the next one starts.
  last_turn_failed: bool,
  turn_gate: TurnGate,
  selection_registry: selectable_text::SelectionRegistry,
  connection_error_scroll: gpui::ScrollHandle,
  /// Built once: rebuilding extensions busts TextView's parse cache.
  markdown_extensions: gpui_component::text::MarkdownExtensions,
  available_modes: Vec<SessionMode>,
  current_mode_id: Option<SessionModeId>,
  available_models: Vec<ModelInfo>,
  current_model_id: Option<ModelId>,
  pending_model_selection: Option<PendingModelSelection>,
  model_change_generation: u64,
  config_options: Vec<SessionConfigOption>,
  /// Baseline for the composer trigger's muted state.
  config_defaults: HashMap<SessionConfigId, SessionConfigValueId>,
  _connect_task: Option<Task<()>>,
  events_rx: Option<async_channel::Receiver<AgentEvent>>,
  _events_task: Option<Task<()>>,
  _permission_task: Option<Task<()>>,
  _input_sub: Option<gpui::Subscription>,
  show_close_control: bool,
}

impl AgentChatPanel {
  /// A panel bound to one conversation for its whole life: `resume` hydrates
  /// an existing one from the shared store, `None` starts fresh.
  #[allow(clippy::too_many_arguments)]
  pub fn new(
    backend_kind: AgentId,
    repo_root: PathBuf,
    cwd: PathBuf,
    store: Option<Entity<ConversationStore>>,
    resume: Option<ConversationMeta>,
    turn_gate: TurnGate,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    let backend = resolve_backend_config(&backend_kind);
    let (input, input_sub) = Self::build_composer_input(window, cx);

    let selection_registry = selectable_text::SelectionRegistry::new();
    let markdown_extensions = code_block::extensions(selection_registry.clone());
    let mut current_conv = resume.clone().unwrap_or_else(new_conversation_meta);
    current_conv.agent_id = backend_kind.clone();

    let mut panel = Self {
      backend_kind,
      backend: backend.clone(),
      repo_root,
      cwd: cwd.clone(),
      status: Status::Connecting,
      items: Vec::new(),
      repo_files: Arc::new(Vec::new()),
      active_selection: None,
      mention_selected_ix: 0,
      mention_dismissed: None,
      tool_index: HashMap::new(),
      pending_agent: String::new(),
      pending_thought: String::new(),
      queued_prompts: Vec::new(),
      composer_history_index: None,
      composer_history_draft: None,
      supports_images: false,
      supports_steering: false,
      staged_images: Vec::new(),
      pending_md_state: None,
      tool_group_pins: HashMap::new(),
      auto_approve: false,
      terminal_store: None,
      _terminal_task: None,
      runway_active: false,
      runway_end_space: 0.0,
      runway_following: false,
      show_jump_pill: false,
      runway_measure_pending: false,
      reader_scrolled: false,
      scroll_save_pending: false,
      available_commands: Vec::new(),
      editing_message: None,
      edit_input: None,
      edit_input_sub: None,
      pending_edit_resubmit: None,
      resubmit_after_connect: None,
      slash_selected_ix: 0,
      slash_dismissed: None,
      session: None,
      input,
      in_flight: false,
      turn_started_at: None,
      _tick_task: None,
      messages_list: {
        let list = gpui::ListState::new(0, gpui::ListAlignment::Top, px(300.));
        list.set_follow_mode(gpui::FollowMode::Tail);
        list
      },
      usage: None,
      agent_version: None,
      auth_methods: Vec::new(),
      auth_required: false,
      store,
      loading_conversation: None,
      current_conv,
      is_active: false,
      title_announced: false,
      last_turn_failed: false,
      turn_gate,
      selection_registry,
      connection_error_scroll: gpui::ScrollHandle::new(),
      markdown_extensions,
      available_modes: Vec::new(),
      current_mode_id: None,
      available_models: Vec::new(),
      current_model_id: None,
      pending_model_selection: None,
      model_change_generation: 0,
      config_options: Vec::new(),
      config_defaults: HashMap::new(),
      _connect_task: None,
      events_rx: None,
      _events_task: None,
      _permission_task: None,
      _input_sub: Some(input_sub),
      show_close_control: false,
    };

    panel.refresh_repo_files(cx);
    panel.install_runway_release(cx);
    panel.sync_list_count();

    match resume.zip(panel.store.clone()) {
      // The transcript hydrates off the main thread, and the session connects
      // after it so a saved session id can resume.
      Some((meta, store)) => {
        panel.loading_conversation = Some(meta.id.clone());
        let load = store.read(cx).load(&meta.id, cx);
        cx.spawn_in(window, async move |this, cx| {
          let loaded = load.await;
          let _ = this.update_in(cx, |panel, window, cx| {
            panel.loading_conversation = None;
            if let Some(loaded) = loaded {
              panel.apply_loaded_conversation(loaded);
              panel.restore_scroll(cx);
            }
            panel.restore_draft(window, cx);
            panel.respawn_session(cx);
            cx.notify();
          });
        })
        .detach();
      }
      None => {
        panel.restore_draft(window, cx);
        panel.respawn_session(cx);
      }
    }
    panel
  }

  /// A panel with no shared store and its own gate, for the driver and tests.
  pub fn standalone(
    backend_kind: AgentId,
    cwd: PathBuf,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    Self::new(
      backend_kind,
      cwd.clone(),
      cwd,
      None,
      None,
      TurnGate::default(),
      window,
      cx,
    )
  }

  /// Pushes the composer's text into the store as the draft of the current
  /// conversation; the store debounces the disk write.
  fn schedule_draft_save(&mut self, cx: &mut Context<Self>) {
    let Some(store) = self.store.clone() else {
      return;
    };
    let text = self.input.read(cx).value();
    let id = self.current_conv.id.clone();
    store.update(cx, |store, cx| store.set_draft(&id, &text, cx));
  }

  /// Persists the reading position of the current conversation; tail-following
  /// is stored as absence so a conversation left at the bottom stays live.
  fn save_scroll_position(&mut self, cx: &mut Context<Self>) {
    let Some(store) = self.store.clone() else {
      return;
    };
    if self.loading_conversation.is_some() {
      return;
    }
    let position = if self.messages_list.is_following_tail() {
      None
    } else {
      let offset = self.messages_list.logical_scroll_top();
      Some((offset.item_ix, f32::from(offset.offset_in_item)))
    };
    let id = self.current_conv.id.clone();
    store.update(cx, |store, cx| store.set_scroll(&id, position, cx));
  }

  /// Puts the list back where the reader left this conversation: a stored
  /// offset scrolls there, otherwise the list follows the tail as before.
  fn restore_scroll(&mut self, cx: &App) {
    let saved = self
      .store
      .as_ref()
      .and_then(|store| store.read(cx).scroll(&self.current_conv.id));
    match saved {
      // scroll_to pauses tail-following by itself; the existing bottom
      // re-engage logic resumes it when the reader returns to the end.
      Some((item_ix, offset_px)) => {
        self.messages_list.scroll_to(gpui::ListOffset {
          item_ix: item_ix.min(self.messages_list.item_count().saturating_sub(1)),
          offset_in_item: px(offset_px),
        });
      }
      // Re-engaging Tail also jumps to the end.
      None => self.messages_list.set_follow_mode(gpui::FollowMode::Tail),
    }
  }

  /// Fills the composer with the stored draft of the current conversation.
  fn restore_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self.reset_composer_history();
    let draft = self
      .store
      .as_ref()
      .and_then(|store| store.read(cx).draft(&self.current_conv.id))
      .unwrap_or_default();
    self.set_composer_value(&draft, window, cx);
  }

  fn reset_composer_history(&mut self) {
    self.composer_history_index = None;
    self.composer_history_draft = None;
  }

  fn sent_prompt_history(&self) -> Vec<String> {
    self
      .items
      .iter()
      .filter_map(|item| match item {
        ChatItem::Message(message) if message.role == ChatRole::User => Some(message.text.clone()),
        _ => None,
      })
      .collect()
  }

  fn set_composer_value(&self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
    let line = text.matches('\n').count() as u32;
    let character = text.rsplit('\n').next().unwrap_or_default().chars().count() as u32;
    self.input.update(cx, |state, cx| {
      let base = state.base_state().clone();
      state.set_value(text, window, cx);
      base.update(cx, |base, cx| {
        base.set_cursor_position(input::Position::new(line, character), window, cx);
      });
    });
  }

  #[cfg(any(test, feature = "test-support"))]
  #[doc(hidden)]
  pub fn submit_prompt_for_driver(
    &mut self,
    text: &str,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.set_composer_value(text, window, cx);
    self.submit(window, cx);
  }

  fn browse_composer_history(&mut self, delta: i32, window: &mut Window, cx: &mut Context<Self>) {
    let (value, at_history_edge) = {
      let input = self.input.read(cx);
      if !input.base_state().read(cx).selected_value().is_empty() {
        return;
      }
      let value = input.value().to_string();
      let cursor_line = input.cursor_position(cx).line;
      let at_history_edge = if delta < 0 {
        value.is_empty() || cursor_line == 0
      } else {
        let last_line = value.matches('\n').count() as u32;
        self.composer_history_index.is_some() && cursor_line >= last_line
      };
      (value, at_history_edge)
    };

    if !at_history_edge {
      return;
    }

    let history = self.sent_prompt_history();
    if history.is_empty() {
      return;
    }

    let next_index = if delta < 0 {
      match self.composer_history_index {
        Some(0) => 0,
        Some(index) => index - 1,
        None => {
          self.composer_history_draft = Some(value);
          history.len() - 1
        }
      }
    } else {
      let Some(index) = self.composer_history_index else {
        return;
      };
      if index + 1 >= history.len() {
        let draft = self.composer_history_draft.take().unwrap_or_default();
        self.composer_history_index = None;
        self.set_composer_value(&draft, window, cx);
        self.schedule_draft_save(cx);
        cx.stop_propagation();
        return;
      }
      index + 1
    };

    self.composer_history_index = Some(next_index);
    self.set_composer_value(&history[next_index], window, cx);
    self.schedule_draft_save(cx);
    cx.stop_propagation();
  }

  /// Replaces the in-memory conversation with a hydrated one; buffers and
  /// runway state reset like any conversation switch.
  fn apply_loaded_conversation(&mut self, loaded: LoadedConversation) {
    let (meta, items, index, pins, auto_approve) = loaded;
    self.current_conv = meta;
    self.current_conv.agent_id = self.backend_kind.clone();
    self.items = items;
    self.tool_index = index;
    self.tool_group_pins = pins;
    self.auto_approve = auto_approve;
    self.pending_agent.clear();
    self.pending_md_state = None;
    self.pending_thought.clear();
    self.reset_composer_history();
    self.clear_runway();
    self.sync_list_count();
    // The splice above may keep heights measured on the previous
    // conversation's rows; they must not stick to the new transcript.
    let count = self.messages_list.item_count();
    self.messages_list.remeasure_items(0..count);
  }

  /// A wheel scroll is the reader taking over: release the runway hold.
  fn install_runway_release(&mut self, cx: &mut Context<Self>) {
    let weak = cx.weak_entity();
    self.messages_list.set_scroll_handler(move |_, _, cx| {
      let _ = weak.update(cx, |panel, _| {
        panel.runway_following = false;
        panel.reader_scrolled = true;
        panel.scroll_save_pending = true;
      });
    });
  }

  /// Enter submits, Shift+Enter inserts a newline, Cmd/Ctrl+Enter submits.
  fn build_composer_input(
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> (Entity<TextareaState>, gpui::Subscription) {
    let input = cx.new(|cx| {
      TextareaState::new(window, cx)
        .auto_grow(1, 8)
        .submit_on_enter(true)
        .placeholder("Message... (@ to add files or diffs)")
    });
    let input_sub = cx.subscribe_in(
      &input,
      window,
      |this, _state, event: &InputEvent, window, cx| match event {
        InputEvent::PressEnter { shift, secondary } if !shift => {
          if *secondary {
            this.submit_steer(window, cx);
          } else {
            this.submit(window, cx);
          }
        }
        InputEvent::Change => {
          this.reset_composer_history();
          this.schedule_draft_save(cx);
        }
        _ => {}
      },
    );
    (input, input_sub)
  }

  /// Whether the agent process is connected and ready for prompts.
  #[cfg(any(test, feature = "test-support"))]
  pub fn backend_ready(&self) -> bool {
    matches!(self.status, Status::Ready)
  }

  /// Simulate the agent process dying, as `on_agent_disconnected` would.
  #[cfg(any(test, feature = "test-support"))]
  pub fn mark_disconnected_for_test(&mut self, cx: &mut Context<Self>) {
    self.on_agent_disconnected(cx);
  }

  /// Stage an image as a paste or drop would.
  #[cfg(any(test, feature = "test-support"))]
  pub fn stage_image_for_test(&mut self, image: gpui::Image, cx: &mut Context<Self>) {
    self.stage_image(image, cx);
  }

  /// Number of images staged for the next prompt.
  #[cfg(any(test, feature = "test-support"))]
  pub fn staged_image_count(&self) -> usize {
    self.staged_images.len()
  }

  /// The inline editor of the message being edited.
  #[cfg(any(test, feature = "test-support"))]
  pub fn edit_input_for_test(&self) -> Option<Entity<TextareaState>> {
    self.edit_input.clone()
  }

  /// Feed an event as the forwarder would, e.g. one trailing in late.
  #[cfg(any(test, feature = "test-support"))]
  pub fn inject_event_for_test(&mut self, event: AgentEvent, cx: &mut Context<Self>) {
    self.on_event(event, cx);
  }

  /// Names of the slash commands the agent advertised.
  #[cfg(any(test, feature = "test-support"))]
  pub fn available_command_names(&self) -> Vec<String> {
    self
      .available_commands
      .iter()
      .map(|c| c.name.clone())
      .collect()
  }

  /// Messages currently queued for the next turns.
  #[cfg(any(test, feature = "test-support"))]
  pub fn queued_prompt_texts(&self) -> Vec<String> {
    self.queued_prompts.clone()
  }

  /// Queue a message as the composer would mid-turn.
  #[cfg(any(test, feature = "test-support"))]
  pub fn queue_prompt_for_test(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
    self.queued_prompts.push(text.into());
    cx.notify();
  }

  /// Give the conversation persistable content without an agent connected.
  #[cfg(any(test, feature = "test-support"))]
  pub fn seed_user_message_for_test(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
    self.items.push(ChatItem::Message(ChatMessage {
      role: ChatRole::User,
      text: text.into(),
      images: 0,
      image_data: Vec::new(),
    }));
    self.persist_state(cx);
    self.sync_list_count();
    cx.notify();
  }

  /// Pretend a turn is running, as `start_turn` would.
  #[cfg(any(test, feature = "test-support"))]
  pub fn pretend_turn_in_flight_for_test(&mut self, cx: &mut Context<Self>) {
    self.start_turn(cx);
  }

  /// Park the conversation on an unanswered permission card.
  #[cfg(any(test, feature = "test-support"))]
  pub fn seed_unresolved_permission_for_test(&mut self, cx: &mut Context<Self>) {
    let update = agent_client_protocol::schema::ToolCallUpdate::new(
      ToolCallId::new(std::sync::Arc::from("test-permission")),
      agent_client_protocol::schema::ToolCallUpdateFields::new().kind(ToolKind::Execute),
    );
    self
      .items
      .push(ChatItem::Permission(Box::new(PermissionItem {
        prompt: PermissionPrompt {
          id: 1,
          tool_call_title: "Run something".into(),
          tool_call: update.clone(),
          options: Vec::new(),
        },
        detail: permission_detail(&update, &self.cwd),
        resolved: None,
        auto: false,
      })));
    self.sync_list_count();
    cx.notify();
  }

  /// The first unanswered permission: its id and extracted invocation.
  #[cfg(any(test, feature = "test-support"))]
  pub fn pending_permission(&self) -> Option<(u64, Option<String>)> {
    self.items.iter().find_map(|item| match item {
      ChatItem::Permission(p) if p.resolved.is_none() => {
        Some((p.prompt.id, p.detail.invocation.clone()))
      }
      _ => None,
    })
  }

  /// Terminal ids embedded in each tool item, oldest first.
  #[cfg(any(test, feature = "test-support"))]
  pub fn tool_terminal_ids(&self) -> Vec<String> {
    self
      .items
      .iter()
      .filter_map(|item| match item {
        ChatItem::Tool(t) => Some(t.terminals.clone()),
        _ => None,
      })
      .flatten()
      .collect()
  }

  /// Live snapshot of one of the session's terminals.
  #[cfg(any(test, feature = "test-support"))]
  pub fn terminal_snapshot(&self, id: &str) -> Option<agent_acp::TerminalSnapshot> {
    self.terminal_store.as_ref()?.snapshot(id)
  }

  /// Kill one of the session's terminals, as the stop button would.
  #[cfg(any(test, feature = "test-support"))]
  pub fn kill_terminal(&self, id: &str) {
    if let Some(store) = self.terminal_store.as_ref() {
      store.kill(id);
    }
  }

  /// The prose still streaming into the current turn's buffer.
  #[cfg(any(test, feature = "test-support"))]
  pub fn streaming_prose(&self) -> String {
    self.pending_agent.clone()
  }

  /// Runway state: (active, following, reserved space below the reply).
  #[cfg(any(test, feature = "test-support"))]
  pub fn runway_state(&self) -> (bool, bool, f32) {
    (
      self.runway_active,
      self.runway_following,
      self.runway_end_space,
    )
  }

  /// Window-coordinate top of the runway anchor row and of the list viewport.
  #[cfg(any(test, feature = "test-support"))]
  pub fn runway_anchor_top(&self) -> Option<(f32, f32)> {
    let anchor_ix = self.list_ix_for_item(self.runway_anchor_item()?);
    let bounds = self.messages_list.bounds_for_item(anchor_ix)?;
    Some((
      f32::from(bounds.top()),
      f32::from(self.messages_list.viewport_bounds().top()),
    ))
  }

  /// Every permission card's (resolved answer, answered automatically).
  #[cfg(any(test, feature = "test-support"))]
  pub fn permission_answers(&self) -> Vec<(Option<String>, bool)> {
    self
      .items
      .iter()
      .filter_map(|item| match item {
        ChatItem::Permission(p) => Some((p.resolved.clone(), p.auto)),
        _ => None,
      })
      .collect()
  }

  /// Each turn summary card's recorded duration, oldest first.
  #[cfg(any(test, feature = "test-support"))]
  pub fn turn_summary_durations(&self) -> Vec<Option<u64>> {
    self
      .items
      .iter()
      .filter_map(|item| match item {
        ChatItem::TurnSummary(s) => Some(s.duration_secs),
        _ => None,
      })
      .collect()
  }

  /// The turn summary cards, oldest first: (path, added, removed) per file.
  #[cfg(any(test, feature = "test-support"))]
  pub fn turn_summary_rows(&self) -> Vec<Vec<(String, u32, u32)>> {
    self
      .items
      .iter()
      .filter_map(|item| match item {
        ChatItem::TurnSummary(s) => Some(
          s.files
            .iter()
            .map(|f| (f.path.clone(), f.added, f.removed))
            .collect(),
        ),
        _ => None,
      })
      .collect()
  }

  /// The thought texts of the conversation, oldest first.
  #[cfg(any(test, feature = "test-support"))]
  pub fn thought_texts(&self) -> Vec<String> {
    self
      .items
      .iter()
      .filter_map(|item| match item {
        ChatItem::Thought(t) => Some(t.text.clone()),
        _ => None,
      })
      .collect()
  }

  /// Focus handle of the composer input.
  #[cfg(any(test, feature = "test-support"))]
  pub fn composer_focus_handle(&self, cx: &App) -> FocusHandle {
    self.input.read(cx).focus_handle(cx)
  }

  /// Current composer text.
  #[cfg(any(test, feature = "test-support"))]
  pub fn composer_text(&self, cx: &App) -> String {
    self.input.read(cx).value().to_string()
  }

  /// The plain message texts of the conversation, oldest first.
  #[cfg(any(test, feature = "test-support"))]
  pub fn transcript_texts(&self) -> Vec<String> {
    self
      .items
      .iter()
      .filter_map(|item| match item {
        ChatItem::Message(message) => Some(message.text.clone()),
        _ => None,
      })
      .collect()
  }

  /// The shape of `new` without connecting: no agent process, no state loading.
  #[cfg(test)]
  fn new_disconnected(cwd: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
    let backend_kind = default_agent_id();
    let (input, input_sub) = Self::build_composer_input(window, cx);
    let selection_registry = selectable_text::SelectionRegistry::new();
    let markdown_extensions = code_block::extensions(selection_registry.clone());

    let mut panel = Self {
      backend: resolve_backend_config(&backend_kind),
      backend_kind,
      repo_root: cwd.clone(),
      cwd,
      status: Status::Connecting,
      items: Vec::new(),
      repo_files: Arc::new(Vec::new()),
      active_selection: None,
      mention_selected_ix: 0,
      mention_dismissed: None,
      tool_index: HashMap::new(),
      pending_agent: String::new(),
      pending_thought: String::new(),
      queued_prompts: Vec::new(),
      composer_history_index: None,
      composer_history_draft: None,
      supports_images: false,
      supports_steering: false,
      staged_images: Vec::new(),
      pending_md_state: None,
      tool_group_pins: HashMap::new(),
      auto_approve: false,
      terminal_store: None,
      _terminal_task: None,
      runway_active: false,
      runway_end_space: 0.0,
      runway_following: false,
      show_jump_pill: false,
      runway_measure_pending: false,
      reader_scrolled: false,
      scroll_save_pending: false,
      available_commands: Vec::new(),
      editing_message: None,
      edit_input: None,
      edit_input_sub: None,
      pending_edit_resubmit: None,
      resubmit_after_connect: None,
      slash_selected_ix: 0,
      slash_dismissed: None,
      session: None,
      input,
      in_flight: false,
      turn_started_at: None,
      _tick_task: None,
      messages_list: {
        let list = gpui::ListState::new(0, gpui::ListAlignment::Top, px(300.));
        list.set_follow_mode(gpui::FollowMode::Tail);
        list
      },
      usage: None,
      agent_version: None,
      auth_methods: Vec::new(),
      auth_required: false,
      store: None,
      loading_conversation: None,
      current_conv: new_conversation_meta(),
      is_active: false,
      title_announced: false,
      last_turn_failed: false,
      turn_gate: TurnGate::default(),
      selection_registry,
      connection_error_scroll: gpui::ScrollHandle::new(),
      markdown_extensions,
      available_modes: Vec::new(),
      current_mode_id: None,
      available_models: Vec::new(),
      current_model_id: None,
      pending_model_selection: None,
      model_change_generation: 0,
      config_options: Vec::new(),
      config_defaults: HashMap::new(),
      _connect_task: None,
      events_rx: None,
      _events_task: None,
      _permission_task: None,
      _input_sub: Some(input_sub),
      show_close_control: false,
    };
    panel.install_runway_release(cx);
    panel.sync_list_count();
    panel
  }

  fn start_turn(&mut self, cx: &mut Context<Self>) {
    self.turn_gate.acquire(&self.cwd, &self.current_conv.id);
    self.last_turn_failed = false;
    self.in_flight = true;
    self.turn_started_at = Some(std::time::Instant::now());
    self.start_tick_task(cx);
  }

  fn end_turn(&mut self) {
    self.turn_gate.release(&self.cwd, &self.current_conv.id);
    self.in_flight = false;
    self.turn_started_at = None;
    self._tick_task = None;
  }

  fn start_tick_task(&mut self, cx: &mut Context<Self>) {
    if self._tick_task.is_some() {
      return;
    }
    let task = cx.spawn(async move |this, cx| {
      loop {
        cx.background_executor()
          .timer(std::time::Duration::from_millis(500))
          .await;
        let active = this
          .update(cx, |panel, cx| {
            let active = panel.in_flight || matches!(panel.status, Status::Connecting);
            if active {
              cx.notify();
            }
            active
          })
          .unwrap_or(false);
        if !active {
          break;
        }
      }
    });
    self._tick_task = Some(task);
  }

  fn on_agent_disconnected(&mut self, cx: &mut Context<Self>) {
    if matches!(self.status, Status::Ready) {
      self.flush_turn_buffers();
      self.status = Status::Error("Agent disconnected".into());
      self.items.push(ChatItem::Message(ChatMessage {
        role: ChatRole::System,
        text: "Agent disconnected.".into(),
        images: 0,
        image_data: Vec::new(),
      }));
      self.end_turn();
      self.session = None;
      self.sync_list_count();
      cx.notify();
    }
  }

  fn extras_before_count(&self) -> usize {
    let mut n = 0;
    if matches!(self.status, Status::MissingBinary { .. }) {
      n += 1;
    }
    if matches!(self.status, Status::Ready) && self.auth_required && !self.auth_methods.is_empty() {
      n += 1;
    }
    n
  }

  fn extras_after_count(&self) -> usize {
    if self.extras_after_kind().is_some() {
      1
    } else {
      0
    }
  }

  /// One extra trailing row: the runway spacer, zero-height when inactive.
  fn total_list_items(&self) -> usize {
    self.extras_before_count() + self.items.len() + self.extras_after_count() + 1
  }

  fn runway_spacer_ix(&self) -> usize {
    self.total_list_items() - 1
  }

  /// The prompt row the runway holds at the viewport top.
  fn runway_anchor_item(&self) -> Option<usize> {
    self.items.iter().rposition(|item| {
      matches!(
        item,
        ChatItem::Message(ChatMessage {
          role: ChatRole::User | ChatRole::ReviewExport,
          ..
        })
      )
    })
  }

  fn arm_runway(&mut self) {
    let Some(anchor_item) = self.runway_anchor_item() else {
      return;
    };
    self.runway_active = true;
    self.runway_following = true;
    self.runway_measure_pending = false;
    self.reader_scrolled = false;
    // Provisional full-viewport reservation: the anchored rows have no
    // measured bounds yet, and without scroll room past the tail the list
    // clamps to its end for a frame. The first measured frame trues it up.
    let measured = f32::from(self.messages_list.viewport_bounds().size.height);
    self.runway_end_space = if measured > 0.0 { measured } else { 600.0 };
    self.messages_list.set_follow_mode(gpui::FollowMode::Normal);
    self.hold_runway_anchor(anchor_item);
    let spacer = self.runway_spacer_ix();
    self.mark_item_changed_at(spacer);
  }

  /// Pins the anchor a small margin below the viewport top when the list has content above it.
  fn hold_runway_anchor(&mut self, anchor_item: usize) {
    let anchor_ix = self.list_ix_for_item(anchor_item);
    self.messages_list.scroll_to(gpui::ListOffset {
      item_ix: anchor_ix,
      offset_in_item: px(0.),
    });
    if anchor_ix > 0 {
      self.messages_list.scroll_by(px(-RUNWAY_TOP_MARGIN_PX));
    }
  }

  fn clear_runway(&mut self) {
    if !self.runway_active {
      return;
    }
    self.runway_active = false;
    self.runway_following = false;
    self.runway_measure_pending = false;
    self.runway_end_space = 0.0;
    let spacer = self.runway_spacer_ix();
    self.mark_item_changed_at(spacer);
  }

  fn schedule_runway_measure(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.runway_measure_pending {
      return;
    }
    self.runway_measure_pending = true;
    cx.on_next_frame(window, |panel, _window, cx| {
      panel.runway_measure_pending = false;
      if panel.runway_active {
        cx.notify();
      }
    });
  }

  /// Per-frame runway upkeep: size the reservation to what the turn has not
  /// filled yet, and keep the anchor pinned while following.
  fn update_runway(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if !self.runway_active {
      return;
    }
    let Some(anchor_item) = self.runway_anchor_item() else {
      self.clear_runway();
      return;
    };
    let anchor_ix = self.list_ix_for_item(anchor_item);
    let spacer_ix = self.runway_spacer_ix();
    let viewport = self.messages_list.viewport_bounds();
    let viewport_height = if viewport.size.height > px(0.) {
      viewport.size.height
    } else {
      window.viewport_size().height
    };
    let anchor_bounds = self.messages_list.bounds_for_item(anchor_ix);
    let last_content_ix = spacer_ix.checked_sub(1);
    let last_content_bounds = last_content_ix.and_then(|ix| self.messages_list.bounds_for_item(ix));
    if anchor_bounds.is_none()
      && self
        .messages_list
        .item_is_above_viewport(anchor_ix)
        .unwrap_or(false)
    {
      if self.runway_following {
        self.hold_runway_anchor(anchor_item);
        self.schedule_runway_measure(window, cx);
        return;
      }
      let tail_below_viewport = last_content_bounds
        .map(|last| last.bottom() > viewport.bottom() + px(1.))
        .unwrap_or(false);
      self.clear_runway();
      if tail_below_viewport {
        self.show_jump_pill = true;
      }
      return;
    }
    // Missing bounds mean unknown, not zero: every stream commit remeasures
    // the tail rows, and a zero end space would snap the list to its end for
    // one frame. Let the previous reservation stand through those frames.
    let tail_height = anchor_bounds.and_then(|a| {
      let last = last_content_bounds?;
      Some((last.bottom() - a.top()).max(px(0.)))
    });
    if tail_height.is_none() {
      self.schedule_runway_measure(window, cx);
    }
    let raw_end_space =
      tail_height.map(|height| viewport_height - px(RUNWAY_TOP_MARGIN_PX) - height);
    let end_space = match raw_end_space {
      Some(space) => space.max(px(0.)),
      None => px(self.runway_end_space),
    };
    if end_space <= px(0.) {
      let tail_below_viewport = raw_end_space.is_some_and(|space| space < px(-1.));
      self.clear_runway();
      if tail_below_viewport {
        self.show_jump_pill = true;
      }
      return;
    }
    if (f32::from(end_space) - self.runway_end_space).abs() > 0.5 {
      self.runway_end_space = end_space.into();
      self.mark_item_changed_at(spacer_ix);
    }
    if self.runway_following {
      self.hold_runway_anchor(anchor_item);
    }
  }

  /// Engage sticky tail-follow until the reader scrolls up.
  pub fn jump_to_tail(&mut self) {
    self.clear_runway();
    self.messages_list.set_follow_mode(gpui::FollowMode::Tail);
    self.show_jump_pill = false;
  }

  /// A reader scroll that lands on the very end opts into following the tail.
  /// The spacer's bottom includes active runway space, so reaching it means
  /// the reader intentionally scrolled to the end of the surface.
  fn update_reader_follow(&mut self) {
    if !self.reader_scrolled {
      return;
    }
    let viewport = self.messages_list.viewport_bounds();
    if viewport.size.height <= px(0.) {
      return;
    }
    let Some(spacer) = self.messages_list.bounds_for_item(self.runway_spacer_ix()) else {
      return;
    };
    self.reader_scrolled = false;
    if spacer.bottom() <= viewport.bottom() + px(1.) && !self.messages_list.is_following_tail() {
      self.jump_to_tail();
    }
  }

  /// The jump pill shows only when real content continues below the viewport.
  /// Unmeasured tail bounds mean unknown, not "away": hold the previous answer
  /// instead of blinking at commit cadence.
  fn update_jump_pill(&mut self) {
    let visible = if self.messages_list.is_following_tail()
      || self.messages_list.viewport_bounds().size.height <= px(0.)
    {
      Some(false)
    } else {
      let viewport_bottom = self.messages_list.viewport_bounds().bottom();
      self
        .runway_spacer_ix()
        .checked_sub(1)
        .and_then(|last_content_ix| self.messages_list.bounds_for_item(last_content_ix))
        .map(|b| b.bottom() > viewport_bottom + px(1.))
    };
    if let Some(visible) = visible {
      self.show_jump_pill = visible;
    }
  }

  fn sync_list_count(&mut self) {
    let new_count = self.total_list_items();
    let old_count = self.messages_list.item_count();
    if new_count == old_count {
      return;
    }
    if new_count > old_count {
      // New rows go before the trailing runway spacer, so its measured
      // height travels with it instead of landing on a content row.
      let at = old_count.saturating_sub(1);
      self.messages_list.splice(at..at, new_count - old_count);
    } else {
      // Shrinks are tail-side (the Generating row settling, truncation);
      // a splice keeps the reading position where reset() jumped to top.
      self.messages_list.splice(new_count..old_count, 0);
    }
  }

  /// Remeasures the last content row (and the runway spacer behind it).
  fn mark_last_item_changed(&mut self) {
    let count = self.messages_list.item_count();
    if count > 0 {
      self
        .messages_list
        .remeasure_items(count.saturating_sub(2)..count);
    }
  }

  fn mark_item_changed_at(&mut self, list_ix: usize) {
    if list_ix < self.messages_list.item_count() {
      self.messages_list.remeasure_items(list_ix..list_ix + 1);
    }
  }

  fn list_ix_for_item(&self, item_idx: usize) -> usize {
    self.extras_before_count() + item_idx
  }

  fn extras_before_kinds(&self) -> Vec<ExtraBeforeKind> {
    let mut v = Vec::new();
    if matches!(self.status, Status::MissingBinary { .. }) {
      v.push(ExtraBeforeKind::MissingBinary);
    }
    if matches!(self.status, Status::Ready) && self.auth_required && !self.auth_methods.is_empty() {
      v.push(ExtraBeforeKind::Auth);
    }
    v
  }

  fn extras_after_kind(&self) -> Option<ExtraAfterKind> {
    if matches!(self.status, Status::Error(_)) {
      Some(ExtraAfterKind::Error)
    } else if matches!(self.status, Status::Connecting) || self.in_flight {
      Some(ExtraAfterKind::Generating)
    } else {
      None
    }
  }

  /// Flips auto-approve; enabling it also answers any permission already
  /// waiting, so a parked turn resumes immediately.
  pub fn toggle_auto_approve(&mut self, cx: &mut Context<Self>) {
    self.auto_approve = !self.auto_approve;
    if self.auto_approve {
      let pending: Vec<(u64, Option<String>)> = self
        .items
        .iter()
        .filter_map(|item| match item {
          ChatItem::Permission(p) if p.resolved.is_none() => {
            auto_approve_option(&p.prompt.options).map(|option| (p.prompt.id, Some(option)))
          }
          _ => None,
        })
        .collect();
      for (prompt_id, option_id) in pending {
        for item in self.items.iter_mut() {
          if let ChatItem::Permission(p) = item
            && p.prompt.id == prompt_id
          {
            p.auto = true;
          }
        }
        self.answer_permission(prompt_id, option_id, cx);
      }
    }
    self.persist_state(cx);
    cx.notify();
  }

  pub fn answer_permission(
    &mut self,
    prompt_id: u64,
    option_id: Option<String>,
    cx: &mut Context<Self>,
  ) {
    if let Some(session) = self.session.as_ref() {
      session.answer_permission(prompt_id, option_id.clone());
    }
    let mut hit: Option<usize> = None;
    for (i, item) in self.items.iter_mut().enumerate() {
      if let ChatItem::Permission(p) = item
        && p.prompt.id == prompt_id
      {
        p.resolved = Some(option_id.clone().unwrap_or_else(|| "cancel".into()));
        hit = Some(i);
      }
    }
    if let Some(i) = hit {
      let list_ix = self.list_ix_for_item(i);
      self.mark_item_changed_at(list_ix);
    }
    cx.notify();
  }

  fn apply_session_info(&mut self, info: SessionInfoUpdate) {
    if let Some(title) = info.title.value() {
      self.current_conv.title = title.clone();
    } else if info.title.is_null() {
      self.current_conv.title.clear();
    }
    // The event forwarder schedules a persist after every on_event batch.
  }

  fn apply_plan(&mut self, plan: Plan) {
    let view = plan_view_from_acp(&plan);
    // The live plan is the last one of the current turn: an interleaved tool
    // call must update it in place, not append a duplicate block.
    for (ix, item) in self.items.iter_mut().enumerate().rev() {
      match item {
        ChatItem::Plan(existing) => {
          *existing = view;
          let list_ix = self.list_ix_for_item(ix);
          self.mark_item_changed_at(list_ix);
          return;
        }
        ChatItem::Message(m) if m.role == ChatRole::User => break,
        _ => {}
      }
    }
    self.items.push(ChatItem::Plan(view));
  }

  /// Reload the mention candidates; the agent creates files as it works.
  fn refresh_repo_files(&mut self, cx: &mut Context<Self>) {
    let files_cwd = self.cwd.clone();
    let listing = cx.background_spawn(async move { list_repo_files(files_cwd) });
    cx.spawn(async move |this, cx| {
      let files = listing.await;
      let _ = this.update(cx, |panel, cx| {
        panel.repo_files = Arc::new(files);
        cx.notify();
      });
    })
    .detach();
  }

  fn toggle_diff_expanded(&mut self, tool_id: ToolCallId, diff_idx: usize, cx: &mut Context<Self>) {
    let mut hit: Option<usize> = None;
    for (i, item) in self.items.iter_mut().enumerate() {
      if let ChatItem::Tool(t) = item
        && t.id == tool_id
      {
        if let Some(d) = t.diffs.get_mut(diff_idx) {
          d.expanded = !d.expanded;
          hit = Some(i);
        }
        break;
      }
    }
    if let Some(i) = hit {
      let list_ix = self.list_ix_for_item(i);
      self.mark_item_changed_at(list_ix);
      cx.notify();
    }
  }

  fn toggle_output_expanded(
    &mut self,
    tool_id: ToolCallId,
    output_idx: usize,
    cx: &mut Context<Self>,
  ) {
    let mut hit: Option<usize> = None;
    for (i, item) in self.items.iter_mut().enumerate() {
      if let ChatItem::Tool(t) = item
        && t.id == tool_id
      {
        if let Some(o) = t.outputs.get_mut(output_idx) {
          o.expanded = !o.expanded;
          hit = Some(i);
        }
        break;
      }
    }
    if let Some(i) = hit {
      let list_ix = self.list_ix_for_item(i);
      self.mark_item_changed_at(list_ix);
      cx.notify();
    }
  }

  fn toggle_thought_collapsed(&mut self, idx: usize, cx: &mut Context<Self>) {
    if let Some(ChatItem::Thought(t)) = self.items.get_mut(idx) {
      t.collapsed = !t.collapsed;
      let list_ix = self.list_ix_for_item(idx);
      self.mark_item_changed_at(list_ix);
      cx.notify();
    }
  }

  fn upsert_tool_call(&mut self, mut call: ToolCall) {
    fill_tool_call_read_snapshot(&mut call, &self.cwd);
    upsert_tool_call_pure(&mut self.items, &mut self.tool_index, call, &self.cwd);
  }

  fn apply_tool_call_update(&mut self, mut update: ToolCallUpdate) {
    let existing = self
      .tool_index
      .get(&update.tool_call_id)
      .and_then(|index| self.items.get(*index))
      .and_then(|item| match item {
        ChatItem::Tool(view) => Some(view),
        _ => None,
      });
    fill_tool_update_read_snapshot(&mut update, existing, &self.cwd);
    apply_tool_call_update_pure(&mut self.items, &self.tool_index, update, &self.cwd);
  }

  fn set_mode(&mut self, mode_id: SessionModeId, cx: &mut Context<Self>) {
    let Some(session) = self.session.clone() else {
      return;
    };
    self.current_mode_id = Some(mode_id.clone());
    cx.notify();
    cx.spawn(async move |_, _| {
      let _ = session.set_mode(mode_id).await;
    })
    .detach();
  }

  fn set_model(&mut self, model_id: ModelId, cx: &mut Context<Self>) {
    let same_pending = self
      .pending_model_selection
      .as_ref()
      .map(|pending| pending.model_id == model_id)
      .unwrap_or(true);
    if self.current_model_id.as_ref() == Some(&model_id) && same_pending {
      return;
    }
    let previous_model_id = self
      .pending_model_selection
      .as_ref()
      .map(|pending| pending.previous_model_id.clone())
      .unwrap_or_else(|| self.current_model_id.clone());
    persist_model_choice(&self.backend_kind, model_id.0.as_ref());
    self.current_model_id = Some(model_id.clone());
    self.model_change_generation = self.model_change_generation.wrapping_add(1);
    let generation = self.model_change_generation;
    if self.in_flight {
      self.pending_model_selection = Some(PendingModelSelection {
        model_id,
        previous_model_id,
        generation,
      });
      cx.notify();
      return;
    }
    let Some(session) = self.session.clone() else {
      cx.notify();
      return;
    };
    self.pending_model_selection = None;
    let dispatch_queued_after = !self.queued_prompts.is_empty();
    self.spawn_set_model_request(
      session,
      model_id,
      previous_model_id,
      generation,
      dispatch_queued_after,
      cx,
    );
    cx.notify();
  }

  fn spawn_set_model_request(
    &mut self,
    session: Arc<AgentSession>,
    model_id: ModelId,
    previous_model_id: Option<ModelId>,
    generation: u64,
    dispatch_queued_after: bool,
    cx: &mut Context<Self>,
  ) {
    cx.spawn(async move |this, cx| {
      let result = session.set_model(model_id.clone()).await;
      let _ = this.update(cx, |panel, cx| {
        if panel.model_change_generation != generation {
          return;
        }
        if let Err(error) = result {
          let raw = format!("{error}");
          log::warn!("[agent] set model error: {raw}");
          panel.current_model_id = previous_model_id.clone();
          if let Some(previous) = previous_model_id.as_ref() {
            persist_model_choice(&panel.backend_kind, previous.0.as_ref());
          }
          panel.items.push(ChatItem::Message(ChatMessage {
            role: ChatRole::System,
            text: format!(
              "[error] Could not switch model: {}",
              truncate_chars(&raw, 200)
            ),
            images: 0,
            image_data: Vec::new(),
          }));
          panel.persist_state(cx);
          panel.sync_list_count();
          cx.notify();
          return;
        }
        if dispatch_queued_after {
          panel.dispatch_next_queued_prompt(cx);
        }
        cx.notify();
      });
    })
    .detach();
  }

  fn dispatch_next_queued_prompt(&mut self, cx: &mut Context<Self>) {
    if self.queued_prompts.is_empty() {
      return;
    }
    let next = self.queued_prompts.remove(0);
    if !self.dispatch_prompt(next.clone(), cx) {
      self.queued_prompts.insert(0, next);
    }
  }

  /// Reapply the last model the user picked for this backend, if the agent still offers it.
  fn apply_saved_model_choice(&mut self, cx: &mut Context<Self>) {
    let Some(saved) = load_model_choice(&self.backend_kind) else {
      return;
    };
    if self
      .current_model_id
      .as_ref()
      .is_some_and(|current| current.0.as_ref() == saved)
    {
      return;
    }
    let Some(model) = self
      .available_models
      .iter()
      .find(|model| model.model_id.0.as_ref() == saved)
    else {
      return;
    };
    self.set_model(model.model_id.clone(), cx);
  }

  fn set_config_options(&mut self, options: Vec<SessionConfigOption>) {
    for option in &options {
      let SessionConfigKind::Select(sel) = &option.kind else {
        continue;
      };
      self
        .config_defaults
        .entry(option.id.clone())
        .or_insert_with(|| sel.current_value.clone());
    }
    self.config_options = options;
  }

  fn set_config_option(
    &mut self,
    config_id: SessionConfigId,
    value_id: SessionConfigValueId,
    cx: &mut Context<Self>,
  ) {
    let Some(session) = self.session.clone() else {
      return;
    };
    for opt in self.config_options.iter_mut() {
      if opt.id == config_id
        && let SessionConfigKind::Select(sel) = &mut opt.kind
      {
        sel.current_value = value_id.clone();
      }
    }
    cx.notify();
    cx.spawn(async move |_, _| {
      let _ = session
        .set_config_option(config_id, SessionConfigOptionValue::from(value_id))
        .await;
    })
    .detach();
  }

  pub fn cancel_turn(&mut self, cx: &mut Context<Self>) {
    if let Some(session) = self.session.as_ref() {
      session.cancel();
    }
    cx.notify();
  }

  fn mention_snapshot(&self, cx: &App) -> Option<(MentionTrigger, Vec<MentionCandidate>)> {
    let input = self.input.read(cx);
    let cursor = input.base_state().read(cx).cursor();
    let trigger = mention::mention_trigger_at_cursor(input.value().as_ref(), cursor)?;
    if self
      .mention_dismissed
      .as_ref()
      .is_some_and(|dismissed| dismissed == &trigger)
    {
      return None;
    }
    let candidates = mention::matching_mentions(
      &trigger.query,
      self.repo_files.as_slice(),
      self.active_selection.is_some(),
      MENTION_MENU_MAX_ITEMS,
    );
    if candidates.is_empty() {
      return None;
    }
    Some((trigger, candidates))
  }

  fn mention_on_enter(
    &mut self,
    action: &input::Enter,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if action.secondary {
      return;
    }
    let Some((trigger, candidates)) = self.mention_snapshot(cx) else {
      return;
    };
    let ix = self.mention_selected_ix.min(candidates.len() - 1);
    self.insert_mention(&trigger, &candidates[ix], window, cx);
    cx.stop_propagation();
  }

  fn mention_on_move(&mut self, delta: i32, cx: &mut Context<Self>) {
    let Some((_, candidates)) = self.mention_snapshot(cx) else {
      return;
    };
    let len = candidates.len();
    self.mention_selected_ix = if delta < 0 {
      if self.mention_selected_ix == 0 {
        len - 1
      } else {
        self.mention_selected_ix - 1
      }
    } else {
      (self.mention_selected_ix + 1) % len
    };
    cx.stop_propagation();
    cx.notify();
  }

  fn mention_on_escape(&mut self, cx: &mut Context<Self>) {
    let Some((trigger, _)) = self.mention_snapshot(cx) else {
      return;
    };
    self.mention_dismissed = Some(trigger);
    cx.stop_propagation();
    cx.notify();
  }

  fn slash_snapshot(
    &self,
    cx: &App,
  ) -> Option<(String, Vec<agent_client_protocol::schema::AvailableCommand>)> {
    if self.available_commands.is_empty() {
      return None;
    }
    let input = self.input.read(cx);
    let cursor = input.base_state().read(cx).cursor();
    let token = slash_token_at_cursor(input.value().as_ref(), cursor)?;
    if self.slash_dismissed.as_deref() == Some(token.as_str()) {
      return None;
    }
    let filter = token.to_lowercase();
    let matches: Vec<_> = self
      .available_commands
      .iter()
      .filter(|c| {
        c.name.to_lowercase().contains(&filter) || c.description.to_lowercase().contains(&filter)
      })
      .take(SLASH_MENU_MAX_ITEMS)
      .cloned()
      .collect();
    if matches.is_empty() {
      return None;
    }
    Some((token, matches))
  }

  fn slash_on_enter(&mut self, action: &input::Enter, window: &mut Window, cx: &mut Context<Self>) {
    if action.secondary {
      return;
    }
    let Some((_, candidates)) = self.slash_snapshot(cx) else {
      return;
    };
    let ix = self.slash_selected_ix.min(candidates.len() - 1);
    self.insert_slash_command(&candidates[ix].name.clone(), window, cx);
    cx.stop_propagation();
  }

  fn slash_on_move(&mut self, delta: i32, cx: &mut Context<Self>) {
    let Some((_, candidates)) = self.slash_snapshot(cx) else {
      return;
    };
    let len = candidates.len();
    self.slash_selected_ix = if delta < 0 {
      if self.slash_selected_ix == 0 {
        len - 1
      } else {
        self.slash_selected_ix - 1
      }
    } else {
      (self.slash_selected_ix + 1) % len
    };
    cx.stop_propagation();
    cx.notify();
  }

  fn slash_on_escape(&mut self, cx: &mut Context<Self>) {
    let Some((token, _)) = self.slash_snapshot(cx) else {
      return;
    };
    self.slash_dismissed = Some(token);
    cx.stop_propagation();
    cx.notify();
  }

  /// Replace the leading token with the chosen command, keeping any arguments.
  fn insert_slash_command(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
    let text = self.input.read(cx).value();
    let token_end = text.find(char::is_whitespace).unwrap_or(text.len());
    let replacement = if token_end == text.len() {
      format!("/{name} ")
    } else {
      format!("/{name}")
    };
    let replace_range = mention::byte_range_to_utf16_range(text.as_ref(), 0..token_end);
    self.input.update(cx, |input, cx| {
      input.base_state().clone().update(cx, |base, cx| {
        base.replace_text_in_range(Some(replace_range), &replacement, window, cx);
      });
      input.focus(window, cx);
    });
    self.slash_selected_ix = 0;
    // Keep the menu closed on the inserted name, or Enter would re-insert
    // instead of sending; it reopens as soon as the token changes.
    self.slash_dismissed = Some(name.to_string());
    cx.notify();
  }

  fn insert_mention(
    &mut self,
    trigger: &MentionTrigger,
    candidate: &MentionCandidate,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let token = candidate.token();
    let text = self.input.read(cx).value();
    let replace_range = mention::byte_range_to_utf16_range(text.as_ref(), trigger.range.clone());
    self.input.update(cx, |input, cx| {
      input.base_state().clone().update(cx, |base, cx| {
        base.replace_text_in_range(Some(replace_range), &token, window, cx);
      });
      input.focus(window, cx);
    });
    self.mention_selected_ix = 0;
    self.mention_dismissed = None;
    cx.notify();
  }

  fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let text = self.input.read(cx).value().trim().to_string();
    if text.is_empty() {
      return;
    }
    self.reset_composer_history();
    // Mid-turn, the message queues instead of being refused.
    if self.in_flight {
      self.queued_prompts.push(text);
      self
        .input
        .update(cx, |state, cx| state.set_value("", window, cx));
      self.schedule_draft_save(cx);
      cx.notify();
      return;
    }
    // Drain the composer only once the prompt is actually dispatched: while the
    // agent is still connecting or errored, the user keeps what they typed.
    if self.dispatch_prompt(text, cx) {
      self
        .input
        .update(cx, |state, cx| state.set_value("", window, cx));
      self.schedule_draft_save(cx);
    }
  }

  /// Cmd/Ctrl+Enter: steer the running turn instead of queueing.
  fn submit_steer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if !self.in_flight || !self.supports_steering {
      self.submit(window, cx);
      return;
    }
    let text = self.input.read(cx).value().trim().to_string();
    if text.is_empty() {
      return;
    }
    self.reset_composer_history();
    if self.steer_prompt(text, cx) {
      self
        .input
        .update(cx, |state, cx| state.set_value("", window, cx));
      self.schedule_draft_save(cx);
    }
  }

  /// A pinned group keeps the user's choice; otherwise it is open only while
  /// the turn streams into it (trailing), and folds once the turn settles.
  fn tool_group_expanded(&self, start: usize, end: usize) -> bool {
    if let Some(id) = first_tool_id_in(&self.items, start, end)
      && let Some(&pinned) = self.tool_group_pins.get(&id)
    {
      return pinned;
    }
    self.in_flight && end + 1 == self.items.len()
  }

  fn toggle_tool_group(&mut self, idx: usize, cx: &mut Context<Self>) {
    let Some((start, end, _)) = tool_group_span(&self.items, idx) else {
      return;
    };
    let expanded = self.tool_group_expanded(start, end);
    let Some(id) = first_tool_id_in(&self.items, start, end) else {
      return;
    };
    self.tool_group_pins.insert(id, !expanded);
    for item_ix in start..=end {
      let list_ix = self.list_ix_for_item(item_ix);
      self.mark_item_changed_at(list_ix);
    }
    self.persist_state(cx);
    cx.notify();
  }

  pub fn begin_message_edit(&mut self, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
    if self.in_flight {
      return;
    }
    let Some(ChatItem::Message(m)) = self.items.get(idx) else {
      return;
    };
    if m.role != ChatRole::User || checkpoint_ref_before(&self.items, idx).is_none() {
      return;
    }
    let text = m.text.clone();
    // auto_grow only measures the wrap after the first layout: started at one
    // row, a tall bubble collapses for a frame then jumps back. Seeding the
    // minimum with an estimate keeps the bubble's height through the switch.
    let estimated_rows = estimate_wrapped_rows(&text).clamp(1, 8);
    let input = cx.new(|cx| {
      TextareaState::new(window, cx)
        .auto_grow(estimated_rows, 8)
        .submit_on_enter(true)
        .default_value(text)
    });
    window.focus(&input.read(cx).focus_handle(cx), cx);
    self.edit_input_sub = Some(cx.subscribe_in(
      &input,
      window,
      |this, _state, event: &InputEvent, _window, cx| {
        if let InputEvent::PressEnter { shift: false, .. } = event {
          this.submit_message_edit(cx);
        }
      },
    ));
    self.edit_input = Some(input);
    self.editing_message = Some(idx);
    self.mark_item_changed_at(self.list_ix_for_item(idx));
    cx.notify();
  }

  pub fn cancel_message_edit(&mut self, cx: &mut Context<Self>) {
    if let Some(idx) = self.editing_message.take() {
      self.edit_input = None;
      self.edit_input_sub = None;
      self.mark_item_changed_at(self.list_ix_for_item(idx));
      cx.notify();
    }
  }

  /// Rewind: restore the checkpoint guarding this prompt, truncate, and
  /// resubmit the edited text once the fresh session connects.
  pub fn submit_message_edit(&mut self, cx: &mut Context<Self>) {
    let Some(idx) = self.editing_message else {
      return;
    };
    let Some(text) = self
      .edit_input
      .as_ref()
      .map(|input| input.read(cx).value().trim().to_string())
    else {
      return;
    };
    if text.is_empty() {
      return;
    }
    let Some(ref_name) = checkpoint_ref_before(&self.items, idx) else {
      return;
    };
    self.editing_message = None;
    self.edit_input = None;
    self.edit_input_sub = None;
    self.pending_edit_resubmit = Some((ref_name.clone(), text));
    cx.emit(AgentChatPanelEvent::RollbackRequested { ref_name });
    cx.notify();
  }

  fn pop_queued_to_composer(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
    if ix >= self.queued_prompts.len() {
      return;
    }
    // A non-empty draft swaps into the queue slot so nothing is lost.
    let draft = self.input.read(cx).value().trim().to_string();
    let text = if draft.is_empty() {
      self.queued_prompts.remove(ix)
    } else {
      std::mem::replace(&mut self.queued_prompts[ix], draft)
    };
    self.reset_composer_history();
    self.set_composer_value(&text, window, cx);
    self.schedule_draft_save(cx);
    window.focus(&self.input.read(cx).focus_handle(cx), cx);
    cx.notify();
  }

  fn delete_queued(&mut self, ix: usize, cx: &mut Context<Self>) {
    if ix < self.queued_prompts.len() {
      self.queued_prompts.remove(ix);
      cx.notify();
    }
  }

  /// Sends a queued message into the current turn instead of waiting.
  fn steer_queued(&mut self, ix: usize, cx: &mut Context<Self>) {
    if ix >= self.queued_prompts.len() {
      return;
    }
    let text = self.queued_prompts.remove(ix);
    if !self.steer_prompt(text.clone(), cx) {
      self.queued_prompts.insert(ix, text);
    }
    cx.notify();
  }

  fn stage_image(&mut self, image: gpui::Image, cx: &mut Context<Self>) {
    if !self.supports_images {
      return;
    }
    self.staged_images.push(std::sync::Arc::new(image));
    cx.notify();
  }

  fn remove_staged_image(&mut self, ix: usize, cx: &mut Context<Self>) {
    if ix < self.staged_images.len() {
      self.staged_images.remove(ix);
      cx.notify();
    }
  }

  /// Paste with an image on the clipboard stages it; text keeps the input's
  /// own paste behavior.
  fn intercept_paste(&mut self, cx: &mut Context<Self>) {
    if !self.supports_images {
      return;
    }
    let Some(item) = cx.read_from_clipboard() else {
      return;
    };
    let mut staged_any = false;
    for entry in item.into_entries() {
      if let gpui::ClipboardEntry::Image(image) = entry {
        self.stage_image(image, cx);
        staged_any = true;
      }
    }
    if staged_any {
      cx.stop_propagation();
    }
  }

  fn handle_dropped_paths(
    &mut self,
    paths: &[PathBuf],
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    for path in paths {
      match image_format_for_path(path) {
        Some(format) if self.supports_images => {
          if let Ok(bytes) = std::fs::read(path) {
            self.stage_image(gpui::Image::from_bytes(format, bytes), cx);
          }
        }
        _ => {
          // Non-image files land as a mention token the prompt builder resolves.
          let token = match path.strip_prefix(&self.cwd) {
            Ok(rel) => format!("@{} ", rel.display()),
            Err(_) => format!("{} ", path.display()),
          };
          self.input.update(cx, |state, cx| {
            let mut text = state.value().to_string();
            if !text.is_empty() && !text.ends_with(char::is_whitespace) {
              text.push(' ');
            }
            text.push_str(&token);
            state.set_value(&text, window, cx);
          });
          self.schedule_draft_save(cx);
        }
      }
    }
    window.focus(&self.input.read(cx).focus_handle(cx), cx);
    cx.notify();
  }

  /// Send a prompt programmatically; false if not ready or already in flight.
  pub fn send_external_prompt(&mut self, text: String, cx: &mut Context<Self>) -> bool {
    let text = text.trim().to_string();
    if text.is_empty() {
      return false;
    }
    self.dispatch_prompt_with_role(text, ChatRole::User, cx)
  }

  /// Send a review-comment batch; displayed as a structured review card.
  pub fn send_external_review(&mut self, text: String, cx: &mut Context<Self>) -> bool {
    let text = text.trim().to_string();
    if text.is_empty() {
      return false;
    }
    self.dispatch_prompt_with_role(text, ChatRole::ReviewExport, cx)
  }

  /// Record a working-tree checkpoint taken for the in-flight prompt. The marker is
  /// inserted before the prompt so rolling back lands on the state that preceded it.
  pub fn record_checkpoint(&mut self, ref_name: String, cx: &mut Context<Self>) {
    let marker = ChatItem::Checkpoint(CheckpointMarker {
      ref_name,
      created_at_secs: now_secs(),
    });
    place_checkpoint_marker(&mut self.items, marker);
    self.rebuild_tool_index();
    self.persist_state(cx);
    self.sync_list_count();
    cx.notify();
  }

  /// Drop everything after a checkpoint marker (the marker itself stays) and restart
  /// the agent session: the provider-side context no longer matches the transcript.
  pub fn truncate_at_checkpoint(&mut self, ref_name: &str, cx: &mut Context<Self>) -> bool {
    let Some(keep_len) = checkpoint_truncate_len(&self.items, ref_name) else {
      return false;
    };
    self.editing_message = None;
    self.edit_input = None;
    self.edit_input_sub = None;
    if let Some((armed_ref, text)) = self.pending_edit_resubmit.take()
      && armed_ref == ref_name
    {
      self.resubmit_after_connect = Some(text);
    }
    self.items.truncate(keep_len);
    self.rebuild_tool_index();
    self.pending_agent.clear();
    self.pending_md_state = None;
    self.pending_thought.clear();
    self.end_turn();
    self.clear_runway();
    self.persist_state(cx);
    self.respawn_session(cx);
    self.sync_list_count();
    cx.notify();
    true
  }

  fn rebuild_tool_index(&mut self) {
    self.tool_index = tool_index_for_items(&self.items);
  }

  /// Stash a diff-view selection and drop an `@selection` token into the input so the next message
  /// attaches the selected lines as context.
  pub fn add_selection_context(
    &mut self,
    path: String,
    text: String,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.active_selection = Some(SelectionContext { path, text });

    let value = self.input.read(cx).value().to_string();
    let cursor = self
      .input
      .read(cx)
      .base_state()
      .read(cx)
      .cursor()
      .min(value.len());
    let needs_space = value[..cursor]
      .chars()
      .next_back()
      .is_some_and(|ch| !ch.is_whitespace());
    let insert = if needs_space {
      format!(" {}", mention::SELECTION_TOKEN)
    } else {
      mention::SELECTION_TOKEN.to_string()
    };
    let utf16_range = mention::byte_range_to_utf16_range(&value, cursor..cursor);
    self.input.update(cx, |input, cx| {
      input.base_state().clone().update(cx, |base, cx| {
        base.replace_text_in_range(Some(utf16_range), &insert, window, cx);
      });
      input.focus(window, cx);
    });

    self.mention_dismissed = None;
    cx.notify();
  }

  pub fn is_turn_in_flight(&self) -> bool {
    self.in_flight
  }

  /// The last turn ended in an error and nothing ran since.
  pub fn last_turn_failed(&self) -> bool {
    self.last_turn_failed
  }

  /// The turn is parked on a permission card waiting for a human answer.
  pub fn awaiting_permission(&self) -> bool {
    if !self.in_flight {
      return false;
    }
    self
      .items
      .iter()
      .rev()
      .find_map(|item| match item {
        ChatItem::Permission(permission) => Some(permission.resolved.is_none()),
        _ => None,
      })
      .unwrap_or(false)
  }

  /// Shows or hides the folded work above this turn's summary card.
  fn toggle_turn_work(&mut self, summary_idx: usize, cx: &mut Context<Self>) {
    if let Some(ChatItem::TurnSummary(s)) = self.items.get_mut(summary_idx) {
      s.work_expanded = !s.work_expanded;
    }
    let count = self.messages_list.item_count();
    if count > 0 {
      self.messages_list.remeasure_items(0..count);
    }
    cx.notify();
  }

  /// Marks the summary card guarded by this checkpoint as undone.
  pub fn mark_turn_undone(&mut self, ref_name: &str, cx: &mut Context<Self>) {
    let Some(idx) = self.items.iter().rposition(|item| {
      matches!(item, ChatItem::TurnSummary(s) if s.checkpoint_ref.as_deref() == Some(ref_name))
    }) else {
      return;
    };
    if let Some(ChatItem::TurnSummary(s)) = self.items.get_mut(idx) {
      s.undone = true;
    }
    let list_ix = self.list_ix_for_item(idx);
    self.mark_item_changed_at(list_ix);
    self.persist_state(cx);
    cx.notify();
  }

  pub fn set_close_control_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
    if self.show_close_control == visible {
      return;
    }
    self.show_close_control = visible;
    cx.notify();
  }

  pub fn is_ready(&self) -> bool {
    matches!(self.status, Status::Ready)
  }

  pub fn needs_reconnect(&self) -> bool {
    matches!(self.status, Status::Error(_) | Status::MissingBinary { .. })
  }

  /// Respawn a dead connection in place, resuming the saved agent session.
  pub fn reconnect(&mut self, cx: &mut Context<Self>) {
    self.respawn_session(cx);
  }

  pub fn backend_kind(&self) -> &AgentId {
    &self.backend_kind
  }

  pub fn supports_steering(&self) -> bool {
    self.supports_steering
  }

  /// Whether the shown panel is this one; only it writes the active pointer.
  pub fn set_active_conversation(&mut self, is_active: bool) {
    self.is_active = is_active;
  }

  /// A panel is parked when nothing live would be lost by dropping it.
  pub fn is_parked(&self) -> bool {
    !self.in_flight && self.queued_prompts.is_empty() && self.loading_conversation.is_none()
  }

  /// Boundary persist before a park or an eviction.
  pub fn persist_now(&mut self, cx: &mut Context<Self>) {
    self.save_scroll_position(cx);
    self.persist_state(cx);
  }

  /// The conversation this panel is still hydrating, if any.
  pub fn loading_conversation_id(&self) -> Option<&str> {
    self.loading_conversation.as_deref()
  }

  fn respawn_session(&mut self, cx: &mut Context<Self>) {
    let load_session_id = self.current_conv.session_id.clone();
    self.respawn_session_with(load_session_id, cx);
  }

  fn respawn_session_with(&mut self, load_session_id: Option<String>, cx: &mut Context<Self>) {
    self.session = None;
    self._connect_task = None;
    self.events_rx = None;
    self._events_task = None;
    self._permission_task = None;
    self._terminal_task = None;
    self.terminal_store = None;
    self.pending_agent.clear();
    self.pending_md_state = None;
    self.pending_thought.clear();
    self.end_turn();
    self.auth_required = false;
    self.auth_methods.clear();
    self.agent_version = None;
    self.usage = None;
    self.available_modes.clear();
    self.current_mode_id = None;
    self.available_models.clear();
    self.current_model_id = None;
    self.pending_model_selection = None;
    self.model_change_generation = self.model_change_generation.wrapping_add(1);
    self.config_options.clear();
    self.config_defaults.clear();
    self.status = Status::Connecting;
    if let BackendAvailability::MissingBinary {
      command,
      install_hint,
    } = self.backend.check_availability()
    {
      self.status = Status::MissingBinary {
        command,
        hint: install_hint,
      };
      self.sync_list_count();
      return;
    }
    self.sync_list_count();
    let backend = self.backend.clone();
    let cwd = self.cwd.clone();
    let executor = cx.background_executor().clone();
    let spawner = move |fut: BoxFuture<'static, ()>| {
      executor.spawn(fut).detach();
    };
    let task = cx.spawn(async move |this, cx| {
      let result = match load_session_id {
        Some(id) => AgentSession::spawn_with_load(backend, cwd, id, spawner).await,
        None => AgentSession::spawn(backend, cwd, spawner).await,
      };
      match result {
        Ok(mut session) => {
          let info = session.init_info().clone();
          let events = session.take_events();
          let permissions = session.take_permission_prompts();
          let terminal_updates = session.take_terminal_updates();
          let terminal_store = session.terminal_store();
          let session = Arc::new(session);
          let _ = this.update(cx, |panel, cx| {
            panel.session = Some(session.clone());
            panel.status = Status::Ready;
            panel.supports_images = info.supports_images;
            panel.supports_steering = info.supports_steering;
            panel.agent_version = info.version;
            panel.auth_methods = info.auth_methods;
            panel.available_modes = info.available_modes;
            panel.current_mode_id = info.current_mode_id;
            panel.available_models = info.available_models;
            panel.current_model_id = info.current_model_id;
            panel.set_config_options(info.config_options);
            panel.apply_saved_model_choice(cx);
            if let Some(sid) = info.session_id {
              panel.current_conv.session_id = Some(sid);
              panel.persist_state(cx);
            }
            if let Some(rx) = events {
              panel.start_event_forwarder(rx, cx);
            }
            if let Some(rx) = permissions {
              panel.start_permission_forwarder(rx, cx);
            }
            panel.terminal_store = Some(terminal_store.clone());
            if let Some(rx) = terminal_updates {
              panel.start_terminal_forwarder(rx, cx);
            }
            if let Some(text) = panel.resubmit_after_connect.take() {
              panel.dispatch_prompt(text, cx);
            }
            panel.sync_list_count();
            cx.notify();
          });
        }
        Err(e) => {
          let msg = format!("{e}");
          let _ = this.update(cx, |panel, cx| {
            panel.status = Status::Error(msg);
            panel.sync_list_count();
            cx.notify();
          });
        }
      }
    });
    self._connect_task = Some(task);
    self.start_tick_task(cx);
  }

  pub fn can_switch_backend(&self) -> bool {
    !self.conversation_started() && self.loading_conversation.is_none() && !self.in_flight
  }

  pub fn switch_backend(&mut self, id: AgentId, cx: &mut Context<Self>) {
    if id == self.backend_kind || !self.can_switch_backend() {
      return;
    }
    self.backend = resolve_backend_config(&id);
    self.backend_kind = id.clone();
    self.current_conv.agent_id = id;
    // Session id is backend-specific; clear it so we don't try to load a
    // claude session on codex (or vice-versa).
    self.current_conv.session_id = None;
    self.respawn_session_with(None, cx);
    cx.notify();
  }

  fn conversation_started(&self) -> bool {
    self.items.iter().any(|item| {
      matches!(
        item,
        ChatItem::Message(message)
          if matches!(message.role, ChatRole::User | ChatRole::ReviewExport)
      )
    })
  }

  fn can_switch_model(&self) -> bool {
    self.loading_conversation.is_none()
  }

  /// Whether the conversation has user-visible content worth writing to disk.
  pub fn has_persistable_content(&self) -> bool {
    !self.items.is_empty() || !self.pending_agent.is_empty()
  }

  /// Throttled persist for the streaming path: the store coalesces snapshots
  /// and writes every ~400ms. Boundaries (turn end, disconnect, switch) go
  /// through `persist_state` and skip the throttle.
  fn schedule_persist(&mut self, cx: &mut Context<Self>) {
    let Some((store, request)) = self.store.clone().zip(self.build_save_request()) else {
      return;
    };
    store.update(cx, |store, cx| store.schedule_save(request, cx));
    self.announce_title_if_settled(cx);
  }

  fn persist_state(&mut self, cx: &mut Context<Self>) {
    let Some((store, request)) = self.store.clone().zip(self.build_save_request()) else {
      return;
    };
    store.update(cx, |store, cx| store.save_now(request, cx));
    self.announce_title_if_settled(cx);
  }

  /// Fires once per panel: also on the first persist of a resumed, already
  /// titled conversation, so a rename lost to a crash heals on the next run
  /// (the host's guards make a second rename a no-op).
  fn announce_title_if_settled(&mut self, cx: &mut Context<Self>) {
    if self.title_announced || self.current_conv.title.is_empty() {
      return;
    }
    self.title_announced = true;
    cx.emit(AgentChatPanelEvent::TitleSettled {
      title: self.current_conv.title.clone(),
    });
  }

  /// Snapshot of the conversation for the store; refreshes the meta (count,
  /// title, preview, timestamp) as a side effect so the sidebar stays true.
  fn build_save_request(&mut self) -> Option<SaveRequest> {
    // Skip writing while the conversation has no user-visible content yet
    // (avoids polluting disk + History with empty drafts).
    if !self.has_persistable_content() {
      return None;
    }
    self.current_conv.agent_id = self.backend_kind.clone();
    self.current_conv.message_count = self
      .items
      .iter()
      .filter(|i| matches!(i, ChatItem::Message(_)))
      .count();
    self.current_conv.updated_at_secs = now_secs();
    self.current_conv.preview = preview_of(&self.items);
    if self.current_conv.title.is_empty()
      && let Some(first_user) = self.items.iter().find_map(|i| match i {
        ChatItem::Message(m) if matches!(m.role, ChatRole::User) => Some(m.text.clone()),
        _ => None,
      })
    {
      self.current_conv.title = truncate_title(&first_user);
    }
    let persisted: Vec<PersistedChatItem> = self
      .items
      .iter()
      .map(|item| match item {
        ChatItem::Message(m) => PersistedChatItem::Message(m.clone()),
        ChatItem::Tool(t) => PersistedChatItem::Tool(t.clone()),
        ChatItem::Plan(p) => PersistedChatItem::Plan(p.clone()),
        ChatItem::Thought(t) => PersistedChatItem::Thought(t.clone()),
        ChatItem::Checkpoint(c) => PersistedChatItem::Checkpoint(c.clone()),
        ChatItem::Permission(p) => PersistedChatItem::Permission(p.clone()),
        ChatItem::TurnSummary(s) => PersistedChatItem::TurnSummary(s.clone()),
      })
      .collect();
    Some(SaveRequest {
      conversation: PersistedConversation {
        version: CONVERSATION_FORMAT_VERSION,
        meta: self.current_conv.clone(),
        items: persisted,
        group_pins: self
          .tool_group_pins
          .iter()
          .map(|(id, expanded)| (id.0.to_string(), *expanded))
          .collect(),
        auto_approve: self.auto_approve,
      },
      active_id: self.is_active.then(|| self.current_conv.id.clone()),
    })
  }

  pub fn current_conversation(&self) -> &ConversationMeta {
    &self.current_conv
  }

  /// The checkout this session's agent works in: a worktree or the main repo.
  pub fn cwd(&self) -> &Path {
    &self.cwd
  }

  /// The repository this session belongs to, whatever checkout it works in.
  pub fn repo_root(&self) -> &Path {
    &self.repo_root
  }

  /// The per-repo store this session persists into.
  pub fn store(&self) -> Option<Entity<ConversationStore>> {
    self.store.clone()
  }

  pub fn state_dir_for_repo(state_dir: &std::path::Path, repo: &std::path::Path) -> PathBuf {
    state_dir_for_repo(state_dir, repo)
  }

  /// The per-repo files that hold state ABOUT conversations rather than a
  /// conversation itself. Their mtime says nothing about staleness: an old
  /// `worktrees.json` may still bind live checkouts, and pruning it would
  /// orphan them.
  const STATE_METADATA_FILES: [&'static str; 5] = [
    "index.json",
    "active.txt",
    "drafts.json",
    "scroll.json",
    "worktrees.json",
  ];

  /// Delete conversation files older than `max_age`; best-effort, errors ignored.
  pub fn prune_old_state(state_dir: &std::path::Path, max_age: std::time::Duration) -> usize {
    let now = std::time::SystemTime::now();
    let mut pruned = 0;
    let Ok(entries) = std::fs::read_dir(state_dir) else {
      return 0;
    };
    for entry in entries.flatten() {
      let path = entry.path();
      let Ok(meta) = entry.metadata() else { continue };
      if meta.is_dir() {
        pruned += Self::prune_old_state(&path, max_age);
        let _ = std::fs::remove_dir(&path);
        continue;
      }
      if !meta.is_file() {
        continue;
      }
      if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| Self::STATE_METADATA_FILES.contains(&name))
      {
        continue;
      }
      let Ok(modified) = meta.modified() else {
        continue;
      };
      let Ok(age) = now.duration_since(modified) else {
        continue;
      };
      if age > max_age && std::fs::remove_file(&path).is_ok() {
        pruned += 1;
      }
    }
    pruned
  }
}

static AGENT_SETTINGS_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// The host app points agent settings at its profile-specific config dir (prod vs dev).
pub fn set_settings_dir(dir: PathBuf) {
  let _ = AGENT_SETTINGS_DIR.set(dir);
}

fn agent_settings_path() -> Option<PathBuf> {
  if let Some(dir) = AGENT_SETTINGS_DIR.get() {
    return Some(dir.join("agent.json"));
  }
  Some(dirs::config_dir()?.join("reviu").join("agent.json"))
}

fn read_agent_settings_json() -> serde_json::Value {
  agent_settings_path()
    .and_then(|path| std::fs::read_to_string(path).ok())
    .and_then(|raw| serde_json::from_str(&raw).ok())
    .unwrap_or_else(|| serde_json::json!({}))
}

fn write_agent_settings_json(value: &serde_json::Value) {
  let Some(path) = agent_settings_path() else {
    return;
  };
  if let Some(parent) = path.parent() {
    let _ = std::fs::create_dir_all(parent);
  }
  let _ = std::fs::write(&path, value.to_string());
}

fn settings_with_backend(mut settings: serde_json::Value, key: &str) -> serde_json::Value {
  settings["backend"] = serde_json::Value::String(key.to_string());
  settings
}

fn settings_with_model(
  mut settings: serde_json::Value,
  backend_key: &str,
  model_id: &str,
) -> serde_json::Value {
  if !settings["models"].is_object() {
    settings["models"] = serde_json::json!({});
  }
  settings["models"][backend_key] = serde_json::Value::String(model_id.to_string());
  settings
}

fn model_choice_from_settings(settings: &serde_json::Value, backend_key: &str) -> Option<String> {
  settings
    .get("models")?
    .get(backend_key)?
    .as_str()
    .map(str::to_string)
}

pub fn persist_choice(id: &AgentId) {
  let settings = settings_with_backend(read_agent_settings_json(), id.as_str());
  write_agent_settings_json(&settings);
}

fn persist_model_choice(id: &AgentId, model_id: &str) {
  let settings = settings_with_model(read_agent_settings_json(), id.as_str(), model_id);
  write_agent_settings_json(&settings);
}

fn load_model_choice(id: &AgentId) -> Option<String> {
  model_choice_from_settings(&read_agent_settings_json(), id.as_str())
}

/// Codex names embed the model's DEFAULT reasoning level ("GPT-5.6-Sol (low)"),
/// which contradicts the separate effort selector showing the applied value.
fn strip_effort_suffix(name: &str) -> &str {
  const EFFORT_LEVELS: [&str; 8] = [
    "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
  ];
  let trimmed = name.trim_end();
  let Some(open) = trimmed.rfind(" (") else {
    return name;
  };
  let Some(inner) = trimmed[open + 2..].strip_suffix(')') else {
    return name;
  };
  if open > 0 && EFFORT_LEVELS.contains(&inner.to_ascii_lowercase().as_str()) {
    &trimmed[..open]
  } else {
    name
  }
}

/// One menu entry per base model: adapters list every model x effort combo
/// ("GPT-5.6-Sol (low)", "(medium)", ...) while the effort has its own selector.
/// Selecting a group picks its first variant (the model's default effort).
fn deduped_model_entries(
  models: &[ModelInfo],
  current_id: Option<&ModelId>,
) -> Vec<(String, ModelId, Option<String>, bool)> {
  let mut entries: Vec<(String, ModelId, Option<String>, bool)> = Vec::new();
  for model in models {
    let label = strip_effort_suffix(&model.name).to_string();
    let is_current = current_id == Some(&model.model_id);
    if let Some(existing) = entries.iter_mut().find(|(existing, ..)| *existing == label) {
      existing.3 |= is_current;
    } else {
      entries.push((
        label,
        model.model_id.clone(),
        model.description.clone(),
        is_current,
      ));
    }
  }
  entries
}

/// Insert the marker before the prompt it snapshots; a marker already sitting
/// there (e.g. right after a rollback) is replaced instead of stacked.
fn image_format_for_path(path: &std::path::Path) -> Option<gpui::ImageFormat> {
  let ext = path.extension()?.to_str()?.to_lowercase();
  match ext.as_str() {
    "png" => Some(gpui::ImageFormat::Png),
    "jpg" | "jpeg" => Some(gpui::ImageFormat::Jpeg),
    "webp" => Some(gpui::ImageFormat::Webp),
    "gif" => Some(gpui::ImageFormat::Gif),
    "bmp" => Some(gpui::ImageFormat::Bmp),
    _ => None,
  }
}

fn short_model_label(name: &str, description: Option<&str>) -> String {
  // Claude descriptions follow "<id> [with X] · <blurb>" so the model id can be
  // pulled out. Other backends (Codex) use a free-form blurb with no separator;
  // fall back to the dropdown name there.
  let Some(desc) = description.filter(|d| d.contains(" · ")) else {
    return strip_effort_suffix(name).to_string();
  };
  let before_separator = desc.split(" · ").next().unwrap_or(desc);
  let before_qualifier = before_separator
    .split_once(" with ")
    .map(|(head, _)| head)
    .unwrap_or(before_separator);
  let trimmed = before_qualifier.trim();
  if trimmed.is_empty() {
    strip_effort_suffix(name).to_string()
  } else {
    trimmed.to_string()
  }
}

/// How many visual rows `text` takes in the 560px edit bubble, estimated at
/// ~75 characters per wrapped line. One row of error either way settles in a
/// single frame; collapsing to one row was the visible blink.
fn estimate_wrapped_rows(text: &str) -> usize {
  const CHARS_PER_ROW: usize = 75;
  text
    .lines()
    .map(|line| line.chars().count().div_ceil(CHARS_PER_ROW).max(1))
    .sum::<usize>()
    .max(1)
}

/// A short, actionable line for the error classes users actually hit.
/// `None` falls back to the raw provider text.
fn agent_error_hint(raw: &str) -> Option<&'static str> {
  let lowered = raw.to_ascii_lowercase();
  let has = |needles: &[&str]| needles.iter().any(|needle| lowered.contains(needle));
  if has(&[
    "usage limit",
    "quota",
    "credit",
    "billing",
    "insufficient",
    "payment",
  ]) {
    return Some("The provider refused this turn: usage limit or credits exhausted.");
  }
  if has(&["rate limit", "too many requests", "429"]) {
    return Some("Rate limited by the provider. Wait a moment and retry.");
  }
  if has(&[
    "overloaded",
    "unavailable",
    "timed out",
    "timeout",
    "connection",
    "network",
    "dns",
  ]) {
    return Some("The provider looks unreachable. Check your connection and retry.");
  }
  None
}

/// Pull the human-readable message out of a structured agent error, e.g. Codex's
/// `Internal error: {"message": "{\"detail\":\"...\"}", ...}` envelopes.
fn humanize_agent_error(raw: &str) -> Option<String> {
  fn extract(value: &serde_json::Value) -> Option<String> {
    if let Some(detail) = value.get("detail").and_then(|detail| detail.as_str()) {
      return Some(detail.to_string());
    }
    let message = value.get("message").and_then(|message| message.as_str())?;
    if let Ok(inner) = serde_json::from_str::<serde_json::Value>(message)
      && let Some(found) = extract(&inner)
    {
      return Some(found);
    }
    Some(message.to_string())
  }

  let start = raw.find('{')?;
  let value: serde_json::Value = serde_json::from_str(raw[start..].trim()).ok()?;
  extract(&value)
    .map(|message| message.trim().to_string())
    .filter(|message| !message.is_empty())
}
#[cfg(test)]
mod tests;
