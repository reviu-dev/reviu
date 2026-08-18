mod diff;
mod mention;
mod persistence;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::diff::{
  DiffLineKind, DiffSummary, InlineSpan, InlineSpanKind, MAX_DIFF_LINES_COLLAPSED,
  MAX_TOOL_OUTPUT_LINES_COLLAPSED, extract_diffs, extract_outputs,
};
use crate::mention::{DiffMention, MentionCandidate, MentionTrigger, ResolvedMention};
pub use crate::persistence::{ConversationMeta, state_dir_for_repo};
use crate::persistence::{new_conversation_meta, now_secs, truncate_title};
use agent_acp::{
  AgentEvent, AgentSession, AuthMethodInfo, BackendAvailability, BackendConfig, BackendKind,
  PermissionOptionKind, PermissionPrompt,
};
use agent_client_protocol::schema::{
  ContentBlock, EmbeddedResource, EmbeddedResourceResource, Plan, PlanEntryPriority,
  PlanEntryStatus, ResourceLink, SessionInfoUpdate, TextContent, TextResourceContents, ToolCall,
  ToolCallId, ToolCallStatus, ToolCallUpdate, ToolKind,
};
use agent_client_protocol::schema::{
  ModelId, ModelInfo, SessionConfigId, SessionConfigKind, SessionConfigOption,
  SessionConfigOptionCategory, SessionConfigOptionValue, SessionConfigSelectOptions,
  SessionConfigValueId, SessionMode, SessionModeId,
};
use futures::future::BoxFuture;
use gpui::Anchor;
use gpui::AnimationExt as _;
use gpui::{
  AnyElement, App, Context, Empty, Entity, EntityInputHandler as _, FocusHandle, Focusable, Font,
  FontStyle, FontWeight, Hsla, IntoElement, MouseButton, ParentElement, Render, SharedString,
  Styled, StyledText, Task, TextRun, Window, deferred, div, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Disableable as _, IconName, Sizable as _,
  button::{Button, ButtonVariants as _},
  h_flex,
  input::{self, InputEvent, Textarea, TextareaState},
  menu::{DropdownMenu as _, PopupMenuItem},
  scroll::ScrollableElement as _,
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
struct ConfigSelector {
  id: SessionConfigId,
  name: SharedString,
  current_value: SessionConfigValueId,
  current_label: String,
  values: Vec<(SessionConfigValueId, String, Option<String>)>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct ChatMessage {
  role: ChatRole,
  text: String,
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

/// The marker goes right before the prompt that triggered it (the last user-authored
/// message), so a rollback lands on the state that preceded that prompt.
fn checkpoint_insert_index(items: &[ChatItem]) -> usize {
  items
    .iter()
    .rposition(|item| {
      matches!(
        item,
        ChatItem::Message(ChatMessage {
          role: ChatRole::User | ChatRole::ReviewExport,
          ..
        })
      )
    })
    .unwrap_or(items.len())
}

fn tool_index_for_items(items: &[ChatItem]) -> HashMap<ToolCallId, usize> {
  items
    .iter()
    .enumerate()
    .filter_map(|(ix, item)| match item {
      ChatItem::Tool(tool) => Some((tool.id.clone(), ix)),
      _ => None,
    })
    .collect()
}

/// Number of items to keep so the checkpoint marker is the last remaining item.
fn checkpoint_truncate_len(items: &[ChatItem], ref_name: &str) -> Option<usize> {
  items
    .iter()
    .position(|item| matches!(item, ChatItem::Checkpoint(marker) if marker.ref_name == ref_name))
    .map(|marker_ix| marker_ix + 1)
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
  locations: Vec<(PathBuf, Option<u32>)>,
  #[serde(default)]
  diffs: Vec<DiffSummary>,
  #[serde(default)]
  outputs: Vec<ToolOutput>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct ToolOutput {
  pub text: String,
  #[serde(skip)]
  pub expanded: bool,
  #[serde(skip)]
  pub syntax_spans: Vec<HighlightSpan>,
}

#[derive(Clone, Debug)]
struct PermissionItem {
  prompt: PermissionPrompt,
  resolved: Option<String>,
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
  Permission(PermissionItem),
  Plan(PlanView),
  Thought(ThoughtView),
  Checkpoint(CheckpointMarker),
}

/// Working-tree snapshot taken before the prompt that follows it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct CheckpointMarker {
  ref_name: String,
  created_at_secs: u64,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
enum PersistedChatItem {
  Message(ChatMessage),
  Tool(ToolCallView),
  Plan(PlanView),
  Thought(ThoughtView),
  Checkpoint(CheckpointMarker),
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PersistedConversation {
  meta: ConversationMeta,
  items: Vec<PersistedChatItem>,
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

const MENTION_MENU_MAX_ITEMS: usize = 10;
const MAX_REPO_FILES: usize = 20_000;
/// Caps the conversation and the composer to a readable measure on wide windows.
const CONVERSATION_COLUMN_MAX_WIDTH_PX: f32 = 720.0;
const CONVERSATION_BOTTOM_FADE_PX: f32 = 48.0;

/// Code selected in the Git diff view, pushed in to attach as `@selection` context.
#[derive(Clone, Debug)]
struct SelectionContext {
  path: String,
  text: String,
}

/// Emitted so the host can act on panel interactions.
#[derive(Clone, Debug)]
pub enum AgentChatPanelEvent {
  /// User clicked a tool-call file location; open it in the diff view.
  OpenPath { path: PathBuf, line: Option<u32> },
  /// A prompt was dispatched; the host may snapshot the working tree.
  TurnStarted,
  /// The agent finished a turn; the working tree may have changed.
  TurnFinished,
  /// User asked to roll back to a checkpoint marker.
  RollbackRequested { ref_name: String },
}

impl gpui::EventEmitter<AgentChatPanelEvent> for AgentChatPanel {}

pub struct AgentChatPanel {
  backend_kind: BackendKind,
  backend: BackendConfig,
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
  state_dir: Option<PathBuf>,
  current_conv: ConversationMeta,
  selection_registry: selectable_text::SelectionRegistry,
  available_modes: Vec<SessionMode>,
  current_mode_id: Option<SessionModeId>,
  available_models: Vec<ModelInfo>,
  current_model_id: Option<ModelId>,
  config_options: Vec<SessionConfigOption>,
  /// Baseline for the composer trigger's muted state.
  config_defaults: HashMap<SessionConfigId, SessionConfigValueId>,
  _connect_task: Option<Task<()>>,
  _events_task: Option<Task<()>>,
  _permission_task: Option<Task<()>>,
  _input_sub: Option<gpui::Subscription>,
  show_conversation_controls: bool,
}

impl AgentChatPanel {
  pub fn new(
    backend_kind: BackendKind,
    cwd: PathBuf,
    state_dir: Option<PathBuf>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    let backend = backend_kind.config();
    let input = cx.new(|cx| {
      TextareaState::new(window, cx)
        .auto_grow(1, 8)
        .placeholder("Message... (@ to add files or diffs)")
    });
    let input_sub = cx.subscribe_in(
      &input,
      window,
      |this, _state, event: &InputEvent, window, cx| {
        if let InputEvent::PressEnter { .. } = event {
          this.submit(window, cx);
        }
      },
    );

    let (current_conv, loaded_items, loaded_index) = state_dir
      .as_deref()
      .and_then(load_active_conversation)
      .unwrap_or_else(|| (new_conversation_meta(), Vec::new(), HashMap::new()));

    let selection_registry = selectable_text::SelectionRegistry::new();

    let mut panel = Self {
      backend_kind,
      backend: backend.clone(),
      cwd: cwd.clone(),
      status: Status::Connecting,
      items: loaded_items,
      repo_files: Arc::new(Vec::new()),
      active_selection: None,
      mention_selected_ix: 0,
      mention_dismissed: None,
      tool_index: loaded_index,
      pending_agent: String::new(),
      pending_thought: String::new(),
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
      state_dir,
      current_conv,
      selection_registry,
      available_modes: Vec::new(),
      current_mode_id: None,
      available_models: Vec::new(),
      current_model_id: None,
      config_options: Vec::new(),
      config_defaults: HashMap::new(),
      _connect_task: None,
      _events_task: None,
      _permission_task: None,
      _input_sub: Some(input_sub),
      show_conversation_controls: true,
    };

    {
      let files_cwd = cwd.clone();
      cx.spawn(async move |this, cx| {
        let files = list_repo_files(files_cwd).await;
        let _ = this.update(cx, |panel, cx| {
          panel.repo_files = Arc::new(files);
          cx.notify();
        });
      })
      .detach();
    }

    if let BackendAvailability::MissingBinary {
      command,
      install_hint,
    } = backend.check_availability()
    {
      panel.status = Status::MissingBinary {
        command,
        hint: install_hint,
      };
      panel.sync_list_count();
      return panel;
    }

    let executor = cx.background_executor().clone();
    let spawner = move |fut: BoxFuture<'static, ()>| {
      executor.spawn(fut).detach();
    };

    let load_session_id = panel.current_conv.session_id.clone();
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
          let session = Arc::new(session);
          let _ = this.update(cx, |panel, cx| {
            panel.session = Some(session.clone());
            panel.status = Status::Ready;
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
              panel.persist_state();
            }
            if let Some(rx) = events {
              panel.start_event_forwarder(rx, cx);
            }
            if let Some(rx) = permissions {
              panel.start_permission_forwarder(rx, cx);
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
    panel._connect_task = Some(task);
    panel.sync_list_count();
    panel.start_tick_task(cx);
    panel
  }

  /// The shape of `new` without connecting: no agent process, no state loading.
  #[cfg(test)]
  fn new_disconnected(cwd: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
    let backend_kind = BackendKind::Claude;
    let input = cx.new(|cx| {
      TextareaState::new(window, cx)
        .auto_grow(1, 8)
        .placeholder("Message... (@ to add files or diffs)")
    });
    let input_sub = cx.subscribe_in(
      &input,
      window,
      |this, _state, event: &InputEvent, window, cx| {
        if let InputEvent::PressEnter { .. } = event {
          this.submit(window, cx);
        }
      },
    );

    let mut panel = Self {
      backend_kind,
      backend: backend_kind.config(),
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
      state_dir: None,
      current_conv: new_conversation_meta(),
      selection_registry: selectable_text::SelectionRegistry::new(),
      available_modes: Vec::new(),
      current_mode_id: None,
      available_models: Vec::new(),
      current_model_id: None,
      config_options: Vec::new(),
      config_defaults: HashMap::new(),
      _connect_task: None,
      _events_task: None,
      _permission_task: None,
      _input_sub: Some(input_sub),
      show_conversation_controls: true,
    };
    panel.sync_list_count();
    panel
  }

  fn start_turn(&mut self, cx: &mut Context<Self>) {
    self.in_flight = true;
    self.turn_started_at = Some(std::time::Instant::now());
    self.start_tick_task(cx);
  }

  fn end_turn(&mut self) {
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

  fn start_event_forwarder(
    &mut self,
    rx: async_channel::Receiver<AgentEvent>,
    cx: &mut Context<Self>,
  ) {
    let task = cx.spawn(async move |this, cx| {
      while let Ok(event) = rx.recv().await {
        let _ = this.update(cx, |panel, cx| {
          panel.on_event(event);
          panel.persist_state();
          panel.sync_list_count();
          cx.notify();
        });
      }
      // Channel closed: the agent driver exited (child died or stopped).
      let _ = this.update(cx, |panel, cx| {
        panel.on_agent_disconnected(cx);
        panel.persist_state();
      });
    });
    self._events_task = Some(task);
  }

  fn on_agent_disconnected(&mut self, cx: &mut Context<Self>) {
    if matches!(self.status, Status::Ready) {
      self.flush_pending_thought();
      self.status = Status::Error("Agent disconnected".into());
      self.items.push(ChatItem::Message(ChatMessage {
        role: ChatRole::System,
        text: "Agent disconnected. Toggle the panel to reconnect.".into(),
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

  fn total_list_items(&self) -> usize {
    self.extras_before_count() + self.items.len() + self.extras_after_count()
  }

  fn sync_list_count(&mut self) {
    let new_count = self.total_list_items();
    let old_count = self.messages_list.item_count();
    if new_count == old_count {
      return;
    }
    if new_count > old_count {
      self
        .messages_list
        .splice(old_count..old_count, new_count - old_count);
    } else {
      self.messages_list.reset(new_count);
    }
  }

  fn mark_last_item_changed(&mut self) {
    let count = self.messages_list.item_count();
    if count > 0 {
      self.messages_list.remeasure_items(count - 1..count);
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

  fn render_list_item(
    &mut self,
    list_ix: usize,
    theme: &gpui_component::Theme,
    cx: &mut Context<Self>,
  ) -> gpui::AnyElement {
    let extras_before = self.extras_before_kinds();
    let element = if list_ix < extras_before.len() {
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
    // The last item clears the bottom fade so a fully scrolled transcript is not
    // read through it.
    let is_last = list_ix + 1 == self.total_list_items();
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
    let executable = self.backend.command;
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
        let shell_cmd = cmd.to_shell_string(executable);
        let preview = shell_cmd.clone();
        let copy_value = shell_cmd.clone();
        let copy_id = SharedString::from(format!("auth-copy-{}", method.id));
        let open_id = SharedString::from(format!("auth-open-{}", method.id));
        let launch_cmd = cmd.clone();
        let exec_owned = executable.to_string();
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
                  if !launch_cmd.try_launch_terminal(&exec_owned) {
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
    let is_long = elapsed >= 10;
    let label_color = if is_long {
      theme.warning
    } else {
      theme.muted_foreground
    };
    let brand_icon = match self.backend_kind {
      BackendKind::Claude => UiIconName::Claude,
      BackendKind::Codex => UiIconName::OpenAi,
    };
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
          .with_animation(
            "agent-chat-thinking-pulse",
            gpui::Animation::new(std::time::Duration::from_millis(1400))
              .repeat()
              .with_easing(gpui::pulsating_between(0.35, 1.0)),
            |label, delta| label.opacity(delta),
          ),
      );
    if let Some(e) = elapsed_label {
      row = row.child(div().text_xs().text_color(theme.muted_foreground).child(e));
    }

    let mut container = v_flex().px_3().pb_3().gap_1().child(row);
    if !self.pending_agent.is_empty() {
      container = container.child(markdown_view(
        "agent-chat-md-pending",
        &self.pending_agent,
        cx,
      ));
    }
    container.into_any_element()
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
        div()
          .px_3()
          .pb_3()
          .text_sm()
          .text_color(theme.danger)
          .child(e.clone())
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
    let item_id_base = (idx as u64) << 32;
    let element: gpui::AnyElement = match &item {
      ChatItem::Message(m) => match m.role {
        ChatRole::User => div()
          .px_3()
          .py_2()
          .mb_3()
          .rounded(theme.radius)
          .bg(theme.input_background())
          .border_1()
          .border_color(theme.border)
          .text_sm()
          .text_color(theme.foreground)
          .child(selectable_text::SelectableText::new(
            item_id_base,
            SharedString::from(m.text.clone()),
            Vec::new(),
            registry.clone(),
          ))
          .into_any_element(),
        ChatRole::Agent => timeline_row(
          markdown_view(("agent-chat-md", idx), &m.text, cx),
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
        timeline_row_with_color(
          render_tool_call(t, theme, item_id_base, &registry, cx),
          theme,
          bullet,
          is_last_row,
        )
      }
      ChatItem::Permission(p) => timeline_row(render_permission(p, theme, cx), theme, is_last_row),
      ChatItem::Plan(p) => {
        timeline_row_with_color(render_plan(p, theme), theme, theme.primary, is_last_row)
      }
      ChatItem::Thought(t) => timeline_row(render_thought(idx, t, theme, cx), theme, is_last_row),
      ChatItem::Checkpoint(marker) => {
        let ref_name = marker.ref_name.clone();
        // A trailing marker has nothing after it to undo.
        let can_roll_back = idx + 1 < total;
        let hairline = || div().flex_1().h_px().bg(theme.border.opacity(0.5));

        let center: gpui::AnyElement = if can_roll_back {
          div()
            .id(("chat-checkpoint-rollback", idx))
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
    };
    div().px_3().child(element).into_any_element()
  }

  fn start_permission_forwarder(
    &mut self,
    rx: async_channel::Receiver<PermissionPrompt>,
    cx: &mut Context<Self>,
  ) {
    let task = cx.spawn(async move |this, cx| {
      while let Ok(prompt) = rx.recv().await {
        let _ = this.update(cx, |panel, cx| {
          panel.items.push(ChatItem::Permission(PermissionItem {
            prompt,
            resolved: None,
          }));
          panel.sync_list_count();
          cx.notify();
        });
      }
    });
    self._permission_task = Some(task);
  }

  fn answer_permission(
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

  fn on_event(&mut self, event: AgentEvent) {
    match event {
      AgentEvent::AgentMessageChunk(chunk) => {
        if !self.in_flight {
          return;
        }
        if let ContentBlock::Text(t) = chunk.content {
          self.pending_agent.push_str(&t.text);
        }
        self.sync_list_count();
        self.mark_last_item_changed();
      }
      AgentEvent::AgentThoughtChunk(chunk) => {
        if !self.in_flight {
          return;
        }
        if let ContentBlock::Text(t) = chunk.content {
          self.pending_thought.push_str(&t.text);
        }
        self.sync_list_count();
        self.mark_last_item_changed();
      }
      AgentEvent::ToolCall(call) => {
        let new_id = call.tool_call_id.clone();
        let is_new = !self.tool_index.contains_key(&new_id);
        self.upsert_tool_call(call);
        if !is_new && let Some(&item_idx) = self.tool_index.get(&new_id) {
          let list_ix = self.list_ix_for_item(item_idx);
          self.mark_item_changed_at(list_ix);
        }
      }
      AgentEvent::ToolCallUpdate(update) => {
        let id = update.tool_call_id.clone();
        self.apply_tool_call_update(update);
        if let Some(&item_idx) = self.tool_index.get(&id) {
          let list_ix = self.list_ix_for_item(item_idx);
          self.mark_item_changed_at(list_ix);
        }
      }
      AgentEvent::UsageUpdate(usage) => {
        self.usage = Some((usage.used, usage.size));
      }
      AgentEvent::CurrentModeUpdate(u) => {
        self.current_mode_id = Some(u.current_mode_id);
      }
      AgentEvent::ConfigOptionUpdate(u) => {
        self.set_config_options(u.config_options);
      }
      AgentEvent::Plan(plan) => {
        self.apply_plan(plan);
      }
      AgentEvent::SessionInfoUpdate(info) => {
        self.apply_session_info(info);
      }
      _ => {}
    }
  }

  fn apply_session_info(&mut self, info: SessionInfoUpdate) {
    if let Some(title) = info.title.value() {
      self.current_conv.title = title.clone();
    } else if info.title.is_null() {
      self.current_conv.title.clear();
    }
    self.persist_state();
  }

  fn apply_plan(&mut self, plan: Plan) {
    let view = plan_view_from_acp(&plan);
    if let Some(last) = self.items.last_mut()
      && let ChatItem::Plan(existing) = last
    {
      *existing = view;
      let last_idx = self.items.len() - 1;
      let list_ix = self.list_ix_for_item(last_idx);
      self.mark_item_changed_at(list_ix);
      return;
    }
    self.items.push(ChatItem::Plan(view));
  }

  fn flush_pending_thought(&mut self) {
    if self.pending_thought.is_empty() {
      return;
    }
    let text = std::mem::take(&mut self.pending_thought);
    self.items.push(ChatItem::Thought(ThoughtView {
      text,
      collapsed: true,
    }));
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

  fn upsert_tool_call(&mut self, call: ToolCall) {
    upsert_tool_call_pure(&mut self.items, &mut self.tool_index, call, &self.cwd);
  }

  fn apply_tool_call_update(&mut self, update: ToolCallUpdate) {
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
    let Some(session) = self.session.clone() else {
      return;
    };
    persist_model_choice(self.backend_kind, model_id.0.as_ref());
    self.current_model_id = Some(model_id.clone());
    cx.notify();
    cx.spawn(async move |_, _| {
      let _ = session.set_model(model_id).await;
    })
    .detach();
  }

  /// Reapply the last model the user picked for this backend, if the agent still offers it.
  fn apply_saved_model_choice(&mut self, cx: &mut Context<Self>) {
    let Some(saved) = load_model_choice(self.backend_kind) else {
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

  fn cancel(&mut self, cx: &mut Context<Self>) {
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

  fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let text = self.input.read(cx).value().to_string();
    let text = text.trim().to_string();
    if text.is_empty() {
      return;
    }
    // Drain the composer only once the prompt is actually dispatched: while the
    // agent is still connecting or errored, the user keeps what they typed.
    let dispatched = self.dispatch_prompt(text.clone(), cx);
    self.input.update(cx, |state, cx| {
      if dispatched {
        state.set_value("", window, cx);
      } else {
        // Drop the newline the refused Enter just typed into the textarea.
        state.set_value(&text, window, cx);
      }
    });
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
    self.persist_state();
    self.sync_list_count();
    cx.notify();
  }

  /// Drop everything after a checkpoint marker (the marker itself stays) and restart
  /// the agent session: the provider-side context no longer matches the transcript.
  pub fn truncate_at_checkpoint(&mut self, ref_name: &str, cx: &mut Context<Self>) -> bool {
    let Some(keep_len) = checkpoint_truncate_len(&self.items, ref_name) else {
      return false;
    };
    self.items.truncate(keep_len);
    self.rebuild_tool_index();
    self.pending_agent.clear();
    self.pending_thought.clear();
    self.end_turn();
    self.persist_state();
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

  fn dispatch_prompt(&mut self, text: String, cx: &mut Context<Self>) -> bool {
    self.dispatch_prompt_with_role(text, ChatRole::User, cx)
  }

  fn dispatch_prompt_with_role(
    &mut self,
    text: String,
    role: ChatRole,
    cx: &mut Context<Self>,
  ) -> bool {
    if self.in_flight {
      return false;
    }
    let Some(session) = self.session.clone() else {
      return false;
    };

    self.items.push(ChatItem::Message(ChatMessage {
      role,
      text: text.clone(),
    }));
    self.pending_agent.clear();
    self.pending_thought.clear();
    cx.emit(AgentChatPanelEvent::TurnStarted);
    self.start_turn(cx);
    self.persist_state();
    self.sync_list_count();
    self.messages_list.set_follow_mode(gpui::FollowMode::Tail);
    cx.notify();

    let cwd = self.cwd.clone();
    let files = self.repo_files.clone();
    let selection = self.active_selection.take();
    cx.spawn(async move |this, cx| {
      let blocks = build_prompt_blocks(text, files, selection, cwd).await;
      let result = session.send_prompt_blocks(blocks).await;
      let _ = this.update(cx, |panel, cx| {
        panel.flush_pending_thought();
        let pending = std::mem::take(&mut panel.pending_agent);
        if !pending.is_empty() {
          panel.items.push(ChatItem::Message(ChatMessage {
            role: ChatRole::Agent,
            text: pending,
          }));
        }
        match result {
          Ok(_) => {
            panel.auth_required = false;
          }
          Err(e) => {
            let msg = format!("{e}");
            if msg.contains("auth_required") {
              panel.auth_required = true;
              panel.items.push(ChatItem::Message(ChatMessage {
                role: ChatRole::System,
                text: "Authentication required. Sign in below and retry.".into(),
              }));
            } else {
              let raw = format!("{e}");
              let text = match humanize_agent_error(&raw) {
                Some(human) => {
                  // Full payload stays greppable in the app logs.
                  eprintln!("[agent] prompt error: {raw}");
                  format!("[error] {human}")
                }
                None => format!("[error] {raw}"),
              };
              panel.items.push(ChatItem::Message(ChatMessage {
                role: ChatRole::System,
                text,
              }));
            }
          }
        }
        panel.end_turn();
        panel.persist_state();
        panel.sync_list_count();
        cx.emit(AgentChatPanelEvent::TurnFinished);
        cx.notify();
      });
    })
    .detach();

    true
  }

  pub fn is_turn_in_flight(&self) -> bool {
    self.in_flight
  }

  /// Hide the header history/new-conversation buttons when the host provides
  /// its own session list (the sessions shell sidebar).
  pub fn set_conversation_controls_visible(&mut self, visible: bool) {
    self.show_conversation_controls = visible;
  }

  pub fn is_ready(&self) -> bool {
    matches!(self.status, Status::Ready)
  }

  pub fn needs_reconnect(&self) -> bool {
    matches!(self.status, Status::Error(_) | Status::MissingBinary { .. })
  }

  pub fn backend_kind(&self) -> BackendKind {
    self.backend_kind
  }

  pub fn new_conversation(&mut self, cx: &mut Context<Self>) {
    self.persist_state();
    self.current_conv = new_conversation_meta();
    self.items.clear();
    self.tool_index.clear();
    self.pending_agent.clear();
    self.pending_thought.clear();
    self.end_turn();
    self.usage = None;
    self.respawn_session(cx);
    self.sync_list_count();
    cx.notify();
  }

  pub fn delete_conversation(&mut self, id: &str, cx: &mut Context<Self>) {
    let Some(dir) = self.state_dir.clone() else {
      return;
    };
    let path = dir.join(format!("{id}.json"));
    let _ = std::fs::remove_file(&path);
    if self.current_conv.id == id {
      let _ = std::fs::remove_file(dir.join("active.txt"));
      self.current_conv = new_conversation_meta();
      self.items.clear();
      self.tool_index.clear();
      self.pending_agent.clear();
      self.pending_thought.clear();
      self.end_turn();
      self.usage = None;
      self.respawn_session(cx);
    }
    self.sync_list_count();
    cx.notify();
  }

  pub fn load_conversation(&mut self, id: &str, cx: &mut Context<Self>) {
    let Some(dir) = self.state_dir.clone() else {
      return;
    };
    self.persist_state();
    let path = dir.join(format!("{id}.json"));
    let Some((meta, items, index)) = load_conversation_file(&path) else {
      return;
    };
    self.current_conv = meta;
    self.items = items;
    self.tool_index = index;
    self.pending_agent.clear();
    self.pending_thought.clear();
    let _ = std::fs::write(dir.join("active.txt"), &self.current_conv.id);
    self.respawn_session(cx);
    self.sync_list_count();
    cx.notify();
  }

  fn respawn_session(&mut self, cx: &mut Context<Self>) {
    let load_session_id = self.current_conv.session_id.clone();
    self.respawn_session_with(load_session_id, cx);
  }

  fn respawn_session_with(&mut self, load_session_id: Option<String>, cx: &mut Context<Self>) {
    self.session = None;
    self.end_turn();
    self.auth_required = false;
    self.auth_methods.clear();
    self.agent_version = None;
    self.usage = None;
    self.available_modes.clear();
    self.current_mode_id = None;
    self.available_models.clear();
    self.current_model_id = None;
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
          let session = Arc::new(session);
          let _ = this.update(cx, |panel, cx| {
            panel.session = Some(session.clone());
            panel.status = Status::Ready;
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
              panel.persist_state();
            }
            if let Some(rx) = events {
              panel.start_event_forwarder(rx, cx);
            }
            if let Some(rx) = permissions {
              panel.start_permission_forwarder(rx, cx);
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

  pub fn switch_backend(&mut self, kind: BackendKind, cx: &mut Context<Self>) {
    if kind == self.backend_kind {
      return;
    }
    self.backend_kind = kind;
    self.backend = kind.config();
    // Session id is backend-specific; clear it so we don't try to load a
    // claude session on codex (or vice-versa).
    self.current_conv.session_id = None;
    self.respawn_session_with(None, cx);
    cx.notify();
  }

  fn persist_state(&mut self) {
    let Some(dir) = self.state_dir.as_ref() else {
      return;
    };
    // Skip writing while the conversation has no user-visible content yet
    // (avoids polluting disk + History with empty drafts).
    if self.items.is_empty() && self.pending_agent.is_empty() {
      return;
    }
    // Update meta count + title before writing.
    self.current_conv.message_count = self
      .items
      .iter()
      .filter(|i| matches!(i, ChatItem::Message(_)))
      .count();
    self.current_conv.updated_at_secs = now_secs();
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
      .filter_map(|item| match item {
        ChatItem::Message(m) => Some(PersistedChatItem::Message(m.clone())),
        ChatItem::Tool(t) => Some(PersistedChatItem::Tool(t.clone())),
        ChatItem::Plan(p) => Some(PersistedChatItem::Plan(p.clone())),
        ChatItem::Thought(t) => Some(PersistedChatItem::Thought(t.clone())),
        ChatItem::Checkpoint(c) => Some(PersistedChatItem::Checkpoint(c.clone())),
        ChatItem::Permission(_) => None,
      })
      .collect();
    let _ = std::fs::create_dir_all(dir);
    let conv = PersistedConversation {
      meta: self.current_conv.clone(),
      items: persisted,
    };
    let conv_path = dir.join(format!("{}.json", self.current_conv.id));
    if let Ok(json) = serde_json::to_string(&conv) {
      let _ = std::fs::write(&conv_path, json);
    }
    let active_path = dir.join("active.txt");
    let _ = std::fs::write(&active_path, &self.current_conv.id);
  }

  pub fn list_conversations(&self) -> Vec<ConversationMeta> {
    let Some(dir) = self.state_dir.as_ref() else {
      return Vec::new();
    };
    list_conversations_in(dir)
  }

  pub fn current_conversation(&self) -> &ConversationMeta {
    &self.current_conv
  }

  pub fn state_dir_for_repo(state_dir: &std::path::Path, repo: &std::path::Path) -> PathBuf {
    state_dir_for_repo(state_dir, repo)
  }

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

fn agent_settings_path() -> Option<PathBuf> {
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

pub fn persist_choice(kind: BackendKind) {
  let settings = settings_with_backend(read_agent_settings_json(), kind.storage_key());
  write_agent_settings_json(&settings);
}

fn persist_model_choice(kind: BackendKind, model_id: &str) {
  let settings = settings_with_model(read_agent_settings_json(), kind.storage_key(), model_id);
  write_agent_settings_json(&settings);
}

fn load_model_choice(kind: BackendKind) -> Option<String> {
  model_choice_from_settings(&read_agent_settings_json(), kind.storage_key())
}

fn load_active_conversation(
  dir: &std::path::Path,
) -> Option<(ConversationMeta, Vec<ChatItem>, HashMap<ToolCallId, usize>)> {
  let active_path = dir.join("active.txt");
  let active_id = std::fs::read_to_string(&active_path).ok()?;
  let active_id = active_id.trim().to_string();
  if active_id.is_empty() {
    return None;
  }
  let conv_path = dir.join(format!("{active_id}.json"));
  load_conversation_file(&conv_path)
}

fn load_conversation_file(
  path: &std::path::Path,
) -> Option<(ConversationMeta, Vec<ChatItem>, HashMap<ToolCallId, usize>)> {
  let raw = std::fs::read_to_string(path).ok()?;
  let parsed: PersistedConversation = serde_json::from_str(&raw).ok()?;
  let mut items = Vec::with_capacity(parsed.items.len());
  let mut index = HashMap::new();
  for item in parsed.items {
    match item {
      PersistedChatItem::Message(m) => items.push(ChatItem::Message(m)),
      PersistedChatItem::Tool(t) => {
        index.insert(t.id.clone(), items.len());
        items.push(ChatItem::Tool(t));
      }
      PersistedChatItem::Plan(p) => items.push(ChatItem::Plan(p)),
      PersistedChatItem::Thought(t) => items.push(ChatItem::Thought(t)),
      PersistedChatItem::Checkpoint(c) => items.push(ChatItem::Checkpoint(c)),
    }
  }
  Some((parsed.meta, items, index))
}

fn list_conversations_in(dir: &std::path::Path) -> Vec<ConversationMeta> {
  let Ok(entries) = std::fs::read_dir(dir) else {
    return Vec::new();
  };
  let mut metas: Vec<ConversationMeta> = entries
    .flatten()
    .filter_map(|entry| {
      let path = entry.path();
      let name = path.file_name()?.to_str()?.to_string();
      if !name.ends_with(".json") {
        return None;
      }
      let raw = std::fs::read_to_string(&path).ok()?;
      let parsed: PersistedConversation = serde_json::from_str(&raw).ok()?;
      Some(parsed.meta)
    })
    .collect();
  metas.sort_by_key(|m| std::cmp::Reverse(m.updated_at_secs));
  metas
}

fn populate_syntax_spans(view: &mut ToolCallView) {
  let out_lang = view
    .locations
    .first()
    .and_then(|(p, _)| languages::detect_language_config_for_path(p));
  if let Some(cfg) = out_lang {
    for out in &mut view.outputs {
      let mut h = SyntaxHighlighter::new(cfg);
      out.syntax_spans = h.highlight_text(&out.text).unwrap_or_default();
    }
  }
  for d in &mut view.diffs {
    let Some(cfg) = languages::detect_language_config_for_path(std::path::Path::new(&d.path))
    else {
      continue;
    };
    for line in &mut d.lines {
      let mut h = SyntaxHighlighter::new(cfg);
      line.syntax_spans = h.highlight_text(&line.text).unwrap_or_default();
    }
  }
}

fn upsert_tool_call_pure(
  items: &mut Vec<ChatItem>,
  index: &mut HashMap<ToolCallId, usize>,
  call: ToolCall,
  cwd: &std::path::Path,
) {
  let diffs = extract_diffs(&call.content, cwd);
  let outputs = extract_outputs(&call.content);
  let mut view = ToolCallView {
    id: call.tool_call_id.clone(),
    title: call.title,
    kind: call.kind,
    status: call.status,
    locations: call
      .locations
      .into_iter()
      .map(|l| (l.path, l.line))
      .collect(),
    diffs,
    outputs,
  };
  populate_syntax_spans(&mut view);
  if let Some(&idx) = index.get(&call.tool_call_id)
    && let Some(ChatItem::Tool(existing)) = items.get_mut(idx)
  {
    *existing = view;
    return;
  }
  let idx = items.len();
  index.insert(call.tool_call_id, idx);
  items.push(ChatItem::Tool(view));
}

fn apply_tool_call_update_pure(
  items: &mut [ChatItem],
  index: &HashMap<ToolCallId, usize>,
  update: ToolCallUpdate,
  cwd: &std::path::Path,
) {
  let Some(&idx) = index.get(&update.tool_call_id) else {
    return;
  };
  let Some(ChatItem::Tool(view)) = items.get_mut(idx) else {
    return;
  };
  if let Some(kind) = update.fields.kind {
    view.kind = kind;
  }
  if let Some(status) = update.fields.status {
    view.status = status;
  }
  if let Some(title) = update.fields.title {
    view.title = title;
  }
  if let Some(locs) = update.fields.locations {
    view.locations = locs.into_iter().map(|l| (l.path, l.line)).collect();
  }
  if let Some(content) = update.fields.content {
    view.diffs = extract_diffs(&content, cwd);
    view.outputs = extract_outputs(&content);
  }
  populate_syntax_spans(view);
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
fn place_checkpoint_marker(items: &mut Vec<ChatItem>, marker: ChatItem) {
  let insert_ix = checkpoint_insert_index(items);
  if insert_ix > 0 && matches!(items.get(insert_ix - 1), Some(ChatItem::Checkpoint(_))) {
    items[insert_ix - 1] = marker;
  } else {
    items.insert(insert_ix, marker);
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

fn render_selector_item(
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

fn markdown_view(id: impl Into<gpui::ElementId>, source: &str, cx: &App) -> gpui::AnyElement {
  let theme = cx.theme();
  let mut style = TextViewStyle::default().paragraph_gap(gpui::rems(0.5));
  style.highlight_theme = theme.highlight_theme.clone();
  style.is_dark = theme.mode.is_dark();

  TextView::markdown(id, SharedString::from(source.to_string()))
    .style(style)
    .selectable(true)
    // Body text inherits from here; headings scale off `heading_base_font_size`.
    .text_sm()
    .into_any_element()
}

fn timeline_row(
  content: gpui::AnyElement,
  theme: &gpui_component::Theme,
  is_last: bool,
) -> gpui::AnyElement {
  timeline_row_with_color(content, theme, theme.muted_foreground, is_last)
}

fn timeline_row_with_color(
  content: gpui::AnyElement,
  theme: &gpui_component::Theme,
  bullet_color: gpui::Hsla,
  is_last: bool,
) -> gpui::AnyElement {
  h_flex()
    .w_full()
    .gap_3()
    .items_stretch()
    .child(
      div()
        .relative()
        .w(px(8.))
        .flex_shrink_0()
        .when(!is_last, |this| {
          this.child(
            div()
              .absolute()
              .top(px(14.))
              .bottom(px(-8.))
              .left(px(3.5))
              .w(px(1.))
              .bg(theme.border),
          )
        })
        .child(
          div()
            .absolute()
            .top(px(8.))
            .left(px(1.))
            .w(px(6.))
            .h(px(6.))
            .rounded_full()
            .bg(bullet_color),
        ),
    )
    .child(div().flex_1().min_w_0().pb_3().child(content))
    .into_any_element()
}

fn tool_detail_label(t: &ToolCallView) -> String {
  if let Some((path, line)) = t.locations.first() {
    let name = path
      .file_name()
      .and_then(|s| s.to_str())
      .unwrap_or_else(|| path.to_str().unwrap_or(""));
    return match line {
      Some(l) => format!("{name} (line {l})"),
      None => name.to_string(),
    };
  }
  let kind = tool_kind_label(&t.kind);

  t.title
    .strip_prefix(kind)
    .map(|s| s.trim_start().to_string())
    .unwrap_or_else(|| t.title.clone())
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

fn mono_font_for(theme: &gpui_component::Theme) -> Font {
  Font {
    family: theme.mono_font_family.clone(),
    style: FontStyle::Normal,
    weight: FontWeight::NORMAL,
    ..Default::default()
  }
}

fn build_text_runs(
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

fn tool_kind_label(kind: &ToolKind) -> &'static str {
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

fn tool_kind_icon(kind: &ToolKind) -> UiIconName {
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

fn plan_view_from_acp(plan: &Plan) -> PlanView {
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

fn render_plan(plan: &PlanView, theme: &gpui_component::Theme) -> gpui::AnyElement {
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
fn thought_preview(text: &str) -> Option<String> {
  let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
  let cleaned = line.replace("**", "").replace('`', "");
  let preview: String = cleaned.trim().chars().take(80).collect();
  (!preview.is_empty()).then_some(preview)
}

fn render_thought(
  idx: usize,
  thought: &ThoughtView,
  theme: &gpui_component::Theme,
  cx: &mut Context<AgentChatPanel>,
) -> gpui::AnyElement {
  let collapsed = thought.collapsed;
  let icon = if collapsed {
    IconName::ChevronRight
  } else {
    IconName::ChevronDown
  };
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
        .rounded(theme.radius)
        .hover(|s| s.bg(theme.secondary_hover))
        .on_click(cx.listener(move |panel, _, _, cx| panel.toggle_thought_collapsed(idx, cx)))
        .child(
          gpui_component::Icon::new(icon)
            .size_3()
            .text_color(theme.muted_foreground),
        )
        .child(
          div()
            .text_xs()
            .text_color(theme.muted_foreground)
            .truncate()
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
            cx,
          )),
      )
    })
    .into_any_element()
}

fn render_tool_call(
  t: &ToolCallView,
  theme: &gpui_component::Theme,
  item_id_base: u64,
  registry: &selectable_text::SelectionRegistry,
  cx: &mut Context<AgentChatPanel>,
) -> gpui::AnyElement {
  let title_color = match t.status {
    ToolCallStatus::Failed => theme.danger,
    ToolCallStatus::InProgress => theme.warning,
    _ => theme.foreground,
  };
  let detail = tool_detail_label(t);
  let tool_id = t.id.clone();
  let detail_el = (!detail.is_empty()).then(|| match t.locations.first().cloned() {
    Some((path, line)) => div()
      .id(("agent-tool-location", item_id_base as usize))
      .text_sm()
      .text_color(theme.muted_foreground)
      .cursor_pointer()
      .hover(|this| this.text_color(theme.foreground))
      .child(detail.clone())
      .on_click(cx.listener(move |_panel, _ev, _window, cx| {
        cx.emit(AgentChatPanelEvent::OpenPath {
          path: path.clone(),
          line,
        });
      }))
      .into_any_element(),
    None => div()
      .text_sm()
      .text_color(theme.muted_foreground)
      .child(detail.clone())
      .into_any_element(),
  });

  v_flex()
    .gap_1()
    .child(
      h_flex()
        .gap_2()
        .items_center()
        .flex_wrap()
        .child(
          gpui_component::Icon::new(tool_kind_icon(&t.kind))
            .small()
            .text_color(title_color),
        )
        .child(
          div()
            .text_sm()
            .font_weight(gpui::FontWeight::BOLD)
            .text_color(title_color)
            .child(tool_kind_label(&t.kind).to_string()),
        )
        .children(detail_el),
    )
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
          let mut body = v_flex()
            .font_family("monospace")
            .text_xs()
            .bg(theme.background)
            .border_1()
            .border_color(theme.border)
            .rounded(px(3.))
            .overflow_hidden();
          let ui_theme = ui::Theme::new(theme.is_dark());
          let syntax_theme = ui_theme.syntax();
          let mono_font = mono_font_for(theme);
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
            let runs = build_text_runs(
              &line.text,
              &line.spans,
              &line.syntax_spans,
              &syntax_theme,
              fg,
              Some(hl_bg),
              &mono_font,
            );
            let text_col: gpui::AnyElement = if runs.is_empty() {
              div().flex_1().child(line.text.clone()).into_any_element()
            } else {
              div()
                .flex_1()
                .child(StyledText::new(SharedString::from(line.text.clone())).with_runs(runs))
                .into_any_element()
            };
            body = body.child(
              h_flex()
                .w_full()
                .px_2()
                .bg(bg)
                .text_color(fg)
                .child(text_col),
            );
          }
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
        let runs = highlights_to_text_runs(
          &output.syntax_spans,
          &body_text,
          theme.foreground,
          mono_font.clone(),
          &syntax_theme,
        );
        let text_id = item_id_base | 0x100 | (out_idx as u64);
        let content_div = div()
          .font_family("monospace")
          .text_xs()
          .bg(theme.background)
          .border_1()
          .border_color(theme.border)
          .rounded(px(3.))
          .overflow_hidden()
          .px_2()
          .py_1()
          .text_color(theme.foreground)
          .whitespace_normal()
          .child(selectable_text::SelectableText::new(
            text_id,
            SharedString::from(body_text),
            runs,
            registry.clone(),
          ));
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

fn permission_option_is_destructive(kind: &PermissionOptionKind) -> bool {
  matches!(
    kind,
    PermissionOptionKind::RejectOnce | PermissionOptionKind::RejectAlways
  )
}

fn render_permission(
  item: &PermissionItem,
  theme: &gpui_component::Theme,
  cx: &mut Context<AgentChatPanel>,
) -> gpui::AnyElement {
  let prompt_id = item.prompt.id;
  let resolved = item.resolved.clone();
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
    .child(
      div()
        .text_sm()
        .text_color(theme.foreground)
        .child(item.prompt.tool_call_title.clone()),
    );

  if let Some(option_id) = &resolved {
    card = card.child(
      div()
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(format!("Answered: {option_id}")),
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
      let brand_icon = match self.backend_kind {
        BackendKind::Claude => UiIconName::Claude,
        BackendKind::Codex => UiIconName::OpenAi,
      };
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
          .with_animation(
            "agent-chat-connecting-pulse",
            gpui::Animation::new(std::time::Duration::from_millis(1400))
              .repeat()
              .with_easing(gpui::pulsating_between(0.4, 1.0)),
            |content, delta| content.opacity(delta),
          )
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
            let current = self.backend_kind;
            let label_suffix = match &self.status {
              Status::Connecting => "",
              Status::Error(_) => " (error)",
              Status::MissingBinary { .. } => " (not installed)",
              Status::Ready => "",
            };
            let label = format!("{}{}", current.label(), label_suffix);
            let brand_icon = match current {
              BackendKind::Claude => UiIconName::Claude,
              BackendKind::Codex => UiIconName::OpenAi,
            };
            let entity = cx.entity().downgrade();
            Button::new("agent-chat-backend")
              .label(label)
              .icon(brand_icon)
              .dropdown_caret(true)
              .small()
              .ghost()
              .dropdown_menu_with_anchor(Anchor::TopLeft, move |menu, _, _| {
                let mut menu = menu;
                for kind in BackendKind::all() {
                  let kind = *kind;
                  let entity = entity.clone();
                  let is_current = kind == current;
                  let label_text: SharedString = kind.label().into();
                  let brand_icon = match kind {
                    BackendKind::Claude => UiIconName::Claude,
                    BackendKind::Codex => UiIconName::OpenAi,
                  };
                  menu = menu.item(
                    PopupMenuItem::element(move |_, cx| {
                      let theme = cx.theme().clone();
                      h_flex()
                        .w_full()
                        .gap_2()
                        .items_center()
                        .child(
                          gpui_component::Icon::new(brand_icon)
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
                      persist_choice(kind);
                      let _ = entity.update(cx, |panel, cx| panel.switch_backend(kind, cx));
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
                  let conversations = self.list_conversations();
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
                                  .on_click(move |_, _, cx| {
                                    let _ = entity_delete.update(cx, |panel, cx| {
                                      panel.delete_conversation(&id_delete, cx)
                                    });
                                  }),
                              )
                              .into_any_element()
                          })
                          .on_click(move |_, _, cx| {
                            let id = id_load.clone();
                            let _ =
                              entity_load.update(cx, |panel, cx| panel.load_conversation(&id, cx));
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
                    .on_click(cx.listener(|panel, _, _, cx| panel.new_conversation(cx))),
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
              .child(
                div()
                  .id("agent-mention-input")
                  .relative()
                  .w_full()
                  .capture_action(cx.listener(|panel, action: &input::Enter, window, cx| {
                    panel.mention_on_enter(action, window, cx);
                  }))
                  .capture_action(cx.listener(|panel, _: &input::MoveUp, _, cx| {
                    panel.mention_on_move(-1, cx);
                  }))
                  .capture_action(cx.listener(|panel, _: &input::MoveDown, _, cx| {
                    panel.mention_on_move(1, cx);
                  }))
                  .capture_action(cx.listener(|panel, _: &input::Escape, _, cx| {
                    panel.mention_on_escape(cx);
                  }))
                  .child(Textarea::new(&self.input).appearance(false).w_full())
                  .child(self.render_mention_overlay(cx)),
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
                      .children(self.render_config_selector(cx)),
                  )
                  .child(if self.in_flight {
                    Button::new("agent-chat-stop")
                      .icon(UiIconName::Stop)
                      .small()
                      .rounded(px(999.))
                      .danger()
                      .on_click(cx.listener(|panel, _, _, cx| panel.cancel(cx)))
                  } else {
                    Button::new("agent-chat-send")
                      .icon(UiIconName::ArrowUp)
                      .small()
                      .rounded(px(999.))
                      .primary()
                      .disabled(!matches!(self.status, Status::Ready))
                      .on_click(cx.listener(|panel, _, window, cx| panel.submit(window, cx)))
                  }),
              ),
          ),
      )
  }
}

impl AgentChatPanel {
  fn render_model_selector(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
    let models = self.available_models.clone();
    let current_id = self.current_model_id.clone();
    let current_label: SharedString = current_id
      .as_ref()
      .and_then(|id| models.iter().find(|m| m.model_id == *id))
      .map(|m| short_model_label(&m.name, m.description.as_deref()).into())
      .unwrap_or_else(|| "Model".into());
    let entity = cx.entity().downgrade();
    let brand_icon = match self.backend_kind {
      BackendKind::Claude => UiIconName::Claude,
      BackendKind::Codex => UiIconName::OpenAi,
    };
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
fn selectable_config_options(options: &[SessionConfigOption]) -> Vec<ConfigSelector> {
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

fn config_summary(selectors: &[ConfigSelector]) -> String {
  selectors
    .iter()
    .map(|selector| selector.current_label.as_str())
    .collect::<Vec<_>>()
    .join(" · ")
}

/// True once any option left the value the agent first advertised.
fn config_customized(
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
fn selector_trigger(icon: Option<UiIconName>, label: SharedString) -> impl IntoElement {
  h_flex()
    .items_center()
    .gap_1()
    .when_some(icon, |this, icon| {
      this.child(gpui_component::Icon::new(icon).xsmall())
    })
    .child(label)
    .child(gpui_component::Icon::new(IconName::ChevronDown).xsmall())
}

fn mention_labels(candidate: &MentionCandidate) -> (String, String) {
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

async fn list_repo_files(cwd: PathBuf) -> Vec<String> {
  let output = async_process::Command::new("git")
    .args(["ls-files", "--cached", "--others", "--exclude-standard"])
    .current_dir(&cwd)
    .output()
    .await;
  match output {
    Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout)
      .lines()
      .map(|line| line.trim().to_string())
      .filter(|line| !line.is_empty())
      .take(MAX_REPO_FILES)
      .collect(),
    _ => Vec::new(),
  }
}

async fn run_git(cwd: &Path, args: &[&str]) -> anyhow::Result<String> {
  let output = async_process::Command::new("git")
    .args(args)
    .current_dir(cwd)
    .output()
    .await?;
  if !output.status.success() {
    anyhow::bail!(
      "git {} failed: {}",
      args.join(" "),
      String::from_utf8_lossy(&output.stderr).trim()
    );
  }
  Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

async fn detect_base_ref(cwd: &Path) -> Option<String> {
  if let Ok(out) = run_git(cwd, &["rev-parse", "--abbrev-ref", "origin/HEAD"]).await {
    let reference = out.trim();
    if !reference.is_empty() && reference != "origin/HEAD" {
      return Some(reference.to_string());
    }
  }
  for candidate in ["origin/main", "origin/master", "main", "master"] {
    if run_git(cwd, &["rev-parse", "--verify", "--quiet", candidate])
      .await
      .is_ok()
    {
      return Some(candidate.to_string());
    }
  }
  None
}

/// Build the ACP content blocks for a submitted message: the typed text, plus a `ResourceLink` per
/// `@file` mention and an embedded diff `Resource` per `@diff`/`@staged`/`@branch` mention.
async fn build_prompt_blocks(
  text: String,
  files: Arc<Vec<String>>,
  selection: Option<SelectionContext>,
  cwd: PathBuf,
) -> Vec<ContentBlock> {
  let mentions = mention::resolve_mentions(&text, files.as_slice(), selection.is_some());
  let mut blocks = vec![ContentBlock::Text(TextContent::new(text))];

  for mention in mentions {
    match mention {
      ResolvedMention::File(path) => {
        let uri = format!("file://{}", cwd.join(&path).display());
        blocks.push(ContentBlock::ResourceLink(ResourceLink::new(path, uri)));
      }
      ResolvedMention::Diff(diff) => {
        let (kind, content) = resolve_diff(diff, &cwd).await;
        let resource = TextResourceContents::new(content, format!("reviu-diff://{kind}"))
          .mime_type(Some("text/x-diff".to_string()));
        blocks.push(ContentBlock::Resource(EmbeddedResource::new(
          EmbeddedResourceResource::TextResourceContents(resource),
        )));
      }
      ResolvedMention::Selection => {
        if let Some(selection) = selection.as_ref() {
          let uri = format!("reviu-selection://{}", selection.path);
          let resource = TextResourceContents::new(selection.text.clone(), uri);
          blocks.push(ContentBlock::Resource(EmbeddedResource::new(
            EmbeddedResourceResource::TextResourceContents(resource),
          )));
        }
      }
    }
  }

  blocks
}

async fn resolve_diff(diff: DiffMention, cwd: &Path) -> (&'static str, String) {
  match diff {
    DiffMention::Working => ("working", diff_text(run_git(cwd, &["diff", "HEAD"]).await)),
    DiffMention::Staged => (
      "staged",
      diff_text(run_git(cwd, &["diff", "--cached"]).await),
    ),
    DiffMention::Branch => match detect_base_ref(cwd).await {
      Some(base) => (
        "branch",
        diff_text(run_git(cwd, &["diff", &format!("{base}...HEAD")]).await),
      ),
      None => ("branch", "(could not determine base branch)".to_string()),
    },
  }
}

fn diff_text(diff: anyhow::Result<String>) -> String {
  match diff {
    Ok(diff) if diff.trim().is_empty() => "(no changes)".to_string(),
    Ok(diff) => mention::truncate_diff(&diff),
    Err(err) => format!("(error: {err})"),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use agent_client_protocol::schema::{
    SessionConfigSelect, SessionConfigSelectOption, ToolCallLocation, ToolCallUpdateFields,
  };
  use std::sync::atomic::{AtomicU64, Ordering};

  /// Two fixtures created in the same clock tick would otherwise share a directory.
  static TEMP_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

  fn temp_dir(prefix: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .expect("system clock before unix epoch")
      .as_nanos();
    let unique = TEMP_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
      "reviu-{prefix}-{}-{nanos}-{unique}",
      std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
  }

  fn call(id: &str, title: &str, kind: ToolKind) -> ToolCall {
    let arc: std::sync::Arc<str> = std::sync::Arc::from(id);
    let mut c = ToolCall::new(ToolCallId::new(arc), title.to_string());
    c.kind = kind;
    c
  }

  fn test_cwd() -> &'static std::path::Path {
    std::path::Path::new("/")
  }

  fn user_message(text: &str) -> ChatItem {
    ChatItem::Message(ChatMessage {
      role: ChatRole::User,
      text: text.to_string(),
    })
  }

  fn agent_message(text: &str) -> ChatItem {
    ChatItem::Message(ChatMessage {
      role: ChatRole::Agent,
      text: text.to_string(),
    })
  }

  fn checkpoint_marker(ref_name: &str) -> ChatItem {
    ChatItem::Checkpoint(CheckpointMarker {
      ref_name: ref_name.to_string(),
      created_at_secs: 0,
    })
  }

  #[test]
  fn checkpoint_insert_index_lands_before_last_user_prompt() {
    let items = vec![
      user_message("first"),
      agent_message("done"),
      user_message("second"),
    ];
    assert_eq!(checkpoint_insert_index(&items), 2);

    let empty: Vec<ChatItem> = Vec::new();
    assert_eq!(checkpoint_insert_index(&empty), 0);

    let review_only = vec![ChatItem::Message(ChatMessage {
      role: ChatRole::ReviewExport,
      text: "### a.rs:L1 (new side)\nfix\n".to_string(),
    })];
    assert_eq!(checkpoint_insert_index(&review_only), 0);
  }

  #[test]
  fn checkpoint_truncate_len_keeps_marker_and_drops_rest() {
    let items = vec![
      checkpoint_marker("refs/reviu/checkpoints/s/1"),
      user_message("first"),
      agent_message("done"),
      checkpoint_marker("refs/reviu/checkpoints/s/2"),
      user_message("second"),
      agent_message("done again"),
    ];

    assert_eq!(
      checkpoint_truncate_len(&items, "refs/reviu/checkpoints/s/2"),
      Some(4)
    );
    assert_eq!(
      checkpoint_truncate_len(&items, "refs/reviu/checkpoints/s/1"),
      Some(1)
    );
    assert_eq!(checkpoint_truncate_len(&items, "refs/unknown"), None);
  }

  #[test]
  fn tool_index_tracks_positions_after_checkpoint_insertion_and_truncation() {
    let tool_view = |id: &str, title: &str, kind: ToolKind| {
      let arc: std::sync::Arc<str> = std::sync::Arc::from(id);
      ChatItem::Tool(ToolCallView {
        id: ToolCallId::new(arc),
        title: title.to_string(),
        kind,
        status: ToolCallStatus::Completed,
        locations: Vec::new(),
        diffs: Vec::new(),
        outputs: Vec::new(),
      })
    };
    let mut items = vec![
      user_message("prompt"),
      tool_view("tool-1", "Read", ToolKind::Read),
      agent_message("done"),
      tool_view("tool-2", "Edit", ToolKind::Edit),
    ];

    // Marker inserted before the prompt shifts every tool index by one.
    items.insert(
      checkpoint_insert_index(&items),
      checkpoint_marker("refs/reviu/checkpoints/s/1"),
    );
    let index = tool_index_for_items(&items);
    let tool_1: std::sync::Arc<str> = std::sync::Arc::from("tool-1");
    let tool_2: std::sync::Arc<str> = std::sync::Arc::from("tool-2");
    assert_eq!(index.get(&ToolCallId::new(tool_1.clone())), Some(&2));
    assert_eq!(index.get(&ToolCallId::new(tool_2.clone())), Some(&4));

    // Truncating at the marker leaves no tool entries behind.
    let keep_len =
      checkpoint_truncate_len(&items, "refs/reviu/checkpoints/s/1").expect("marker present");
    items.truncate(keep_len);
    let index = tool_index_for_items(&items);
    assert!(index.is_empty());
  }

  #[test]
  fn place_checkpoint_marker_replaces_trailing_marker_instead_of_stacking() {
    // After a rollback the marker is the last item; the next prompt's checkpoint
    // must replace it, not stack a second divider.
    let mut items = vec![
      user_message("first"),
      agent_message("done"),
      checkpoint_marker("refs/reviu/checkpoints/s/1"),
      user_message("second"),
    ];
    // The new prompt "second" was just pushed; its checkpoint lands before it.
    place_checkpoint_marker(&mut items, checkpoint_marker("refs/reviu/checkpoints/s/2"));
    assert_eq!(items.len(), 4);
    assert!(
      matches!(&items[2], ChatItem::Checkpoint(marker) if marker.ref_name == "refs/reviu/checkpoints/s/2")
    );

    // No marker before the prompt: a fresh one is inserted.
    let mut items = vec![user_message("first")];
    place_checkpoint_marker(&mut items, checkpoint_marker("refs/reviu/checkpoints/s/3"));
    assert_eq!(items.len(), 2);
    assert!(matches!(&items[0], ChatItem::Checkpoint(_)));
  }

  #[test]
  fn deduped_model_entries_collapses_effort_variants() {
    let model = |id: &str, name: &str, description: &str| {
      let arc: std::sync::Arc<str> = std::sync::Arc::from(id);
      let mut info = ModelInfo::new(ModelId::new(arc), name.to_string());
      info.description = Some(description.to_string());
      info
    };
    let models = vec![
      model("sol-low", "GPT-5.6-Sol (low)", "Fast"),
      model("sol-high", "GPT-5.6-Sol (high)", "Deep"),
      model("terra-low", "GPT-5.6-Terra (low)", "Balanced"),
    ];
    let current: std::sync::Arc<str> = std::sync::Arc::from("sol-high");
    let current = ModelId::new(current);

    let entries = deduped_model_entries(&models, Some(&current));

    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].0, "GPT-5.6-Sol");
    // Click target is the first variant of the group (the model's default effort).
    assert_eq!(entries[0].1.0.as_ref(), "sol-low");
    // The group is marked current even though the current id is another variant.
    assert!(entries[0].3);
    assert_eq!(entries[1].0, "GPT-5.6-Terra");
    assert!(!entries[1].3);
  }

  #[test]
  fn checkpoint_marker_survives_persistence_roundtrip() {
    let marker = CheckpointMarker {
      ref_name: "refs/reviu/checkpoints/s/1".to_string(),
      created_at_secs: 42,
    };
    let json =
      serde_json::to_string(&PersistedChatItem::Checkpoint(marker.clone())).expect("serialize");
    let restored: PersistedChatItem = serde_json::from_str(&json).expect("deserialize");
    match restored {
      PersistedChatItem::Checkpoint(restored) => assert_eq!(restored, marker),
      _ => panic!("expected checkpoint item"),
    }
  }

  #[test]
  fn strip_effort_suffix_removes_known_levels_only() {
    assert_eq!(strip_effort_suffix("GPT-5.6-Sol (low)"), "GPT-5.6-Sol");
    assert_eq!(strip_effort_suffix("GPT-5.6-Sol (xhigh)"), "GPT-5.6-Sol");
    assert_eq!(strip_effort_suffix("GPT-5.6-Sol"), "GPT-5.6-Sol");
    // Parenthesized content that is not an effort level stays.
    assert_eq!(strip_effort_suffix("Claude (latest)"), "Claude (latest)");
    // A name that is only a suffix stays untouched.
    assert_eq!(strip_effort_suffix(" (low)"), " (low)");
  }

  #[test]
  fn humanize_agent_error_extracts_nested_detail() {
    let raw = r#"acp prompt error: Internal error: {
      "message": "{\"detail\":\"The 'gpt-5.2-codex' model is not supported when using Codex with a ChatGPT account.\"}",
      "codex_error_info": "other"
    }"#;
    assert_eq!(
      humanize_agent_error(raw).as_deref(),
      Some("The 'gpt-5.2-codex' model is not supported when using Codex with a ChatGPT account.")
    );

    let flat = r#"error: {"message": "rate limited"}"#;
    assert_eq!(humanize_agent_error(flat).as_deref(), Some("rate limited"));

    assert_eq!(humanize_agent_error("plain text failure"), None);
    assert_eq!(humanize_agent_error("error: {not json"), None);
  }

  #[test]
  fn agent_settings_json_keeps_backend_and_models_independent() {
    let settings = serde_json::json!({});
    let settings = settings_with_model(settings, "codex", "gpt-5.6-sol");
    let settings = settings_with_backend(settings, "claude");
    let settings = settings_with_model(settings, "claude", "claude-opus-5");

    assert_eq!(settings["backend"], "claude");
    assert_eq!(
      model_choice_from_settings(&settings, "codex").as_deref(),
      Some("gpt-5.6-sol")
    );
    assert_eq!(
      model_choice_from_settings(&settings, "claude").as_deref(),
      Some("claude-opus-5")
    );
    assert_eq!(model_choice_from_settings(&settings, "unknown"), None);
  }

  #[test]
  fn thought_preview_skips_blank_lines_and_strips_emphasis() {
    assert_eq!(
      thought_preview("\n\n**Planning readme inspection strategy**\ndetails"),
      Some("Planning readme inspection strategy".to_string())
    );
    assert_eq!(thought_preview("   \n\n  "), None);
    assert_eq!(thought_preview(""), None);
  }

  #[test]
  fn review_export_label_counts_sections() {
    assert_eq!(review_export_label("no sections here"), "1 review comment");
    assert_eq!(
      review_export_label("### a.rs:L1 (new side)\nfix\n"),
      "1 review comment"
    );
    assert_eq!(
      review_export_label("### a.rs:L1 (new side)\nfix\n\n### b.rs:L2 (new side)\nrename\n"),
      "2 review comments"
    );
  }

  #[test]
  fn review_export_role_survives_persistence_roundtrip() {
    let message = ChatMessage {
      role: ChatRole::ReviewExport,
      text: "### a.rs:L1 (new side)\nfix\n".to_string(),
    };
    let json = serde_json::to_string(&message).expect("serialize");
    let restored: ChatMessage = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(restored.role, ChatRole::ReviewExport);
    assert_eq!(restored.text, message.text);
  }

  #[test]
  fn upsert_inserts_new_tool_call() {
    let mut items: Vec<ChatItem> = Vec::new();
    let mut index = HashMap::new();
    upsert_tool_call_pure(
      &mut items,
      &mut index,
      call("a", "Read foo", ToolKind::Read),
      test_cwd(),
    );
    assert_eq!(items.len(), 1);
    assert_eq!(index.len(), 1);
    let ChatItem::Tool(view) = &items[0] else {
      panic!("expected Tool item")
    };
    assert_eq!(view.title, "Read foo");
  }

  #[test]
  fn upsert_replaces_existing_by_id() {
    let mut items: Vec<ChatItem> = Vec::new();
    let mut index = HashMap::new();
    upsert_tool_call_pure(
      &mut items,
      &mut index,
      call("a", "Old", ToolKind::Read),
      test_cwd(),
    );
    upsert_tool_call_pure(
      &mut items,
      &mut index,
      call("a", "New", ToolKind::Edit),
      test_cwd(),
    );
    assert_eq!(items.len(), 1);
    let ChatItem::Tool(view) = &items[0] else {
      panic!()
    };
    assert_eq!(view.title, "New");
    assert!(matches!(view.kind, ToolKind::Edit));
  }

  #[test]
  fn upsert_appends_distinct_ids_in_order() {
    let mut items: Vec<ChatItem> = Vec::new();
    let mut index = HashMap::new();
    upsert_tool_call_pure(
      &mut items,
      &mut index,
      call("a", "A", ToolKind::Read),
      test_cwd(),
    );
    upsert_tool_call_pure(
      &mut items,
      &mut index,
      call("b", "B", ToolKind::Edit),
      test_cwd(),
    );
    assert_eq!(items.len(), 2);
    assert_eq!(index.get(&ToolCallId::from("a")), Some(&0));
    assert_eq!(index.get(&ToolCallId::from("b")), Some(&1));
  }

  #[test]
  fn apply_update_merges_partial_fields() {
    let mut items: Vec<ChatItem> = Vec::new();
    let mut index = HashMap::new();
    upsert_tool_call_pure(
      &mut items,
      &mut index,
      call("a", "Initial", ToolKind::Read),
      test_cwd(),
    );

    let mut fields = ToolCallUpdateFields::default();
    fields.status = Some(ToolCallStatus::InProgress);
    let update = ToolCallUpdate::new(ToolCallId::from("a"), fields);
    apply_tool_call_update_pure(&mut items, &index, update, test_cwd());

    let ChatItem::Tool(view) = &items[0] else {
      panic!()
    };
    assert_eq!(view.title, "Initial");
    assert!(matches!(view.status, ToolCallStatus::InProgress));
  }

  #[test]
  fn apply_update_unknown_id_is_noop() {
    let mut items: Vec<ChatItem> = Vec::new();
    let index = HashMap::new();
    let mut fields = ToolCallUpdateFields::default();
    fields.status = Some(ToolCallStatus::Completed);
    let update = ToolCallUpdate::new(ToolCallId::from("ghost"), fields);
    apply_tool_call_update_pure(&mut items, &index, update, test_cwd());
    assert!(items.is_empty());
  }

  #[test]
  fn apply_update_replaces_locations() {
    let mut items: Vec<ChatItem> = Vec::new();
    let mut index = HashMap::new();
    upsert_tool_call_pure(
      &mut items,
      &mut index,
      call("a", "Edit", ToolKind::Edit),
      test_cwd(),
    );

    let mut fields = ToolCallUpdateFields::default();
    fields.locations = Some(vec![ToolCallLocation::new("foo.rs").line(42_u32)]);
    let update = ToolCallUpdate::new(ToolCallId::from("a"), fields);
    apply_tool_call_update_pure(&mut items, &index, update, test_cwd());

    let ChatItem::Tool(view) = &items[0] else {
      panic!()
    };
    assert_eq!(view.locations.len(), 1);
    assert_eq!(view.locations[0].1, Some(42));
  }

  #[test]
  fn diff_expansion_defaults_to_collapsed() {
    use agent_client_protocol::schema::{Diff, ToolCallContent};
    let content = vec![ToolCallContent::Diff(
      Diff::new("foo.rs", "new\n").old_text(Some("old\n".to_string())),
    )];
    let diffs = crate::diff::extract_diffs(&content, test_cwd());
    assert!(!diffs[0].expanded);
  }

  #[test]
  fn session_info_update_sets_title_value_and_null_clears() {
    let mut meta = new_conversation_meta();
    meta.title = "hello".into();
    let value_update = SessionInfoUpdate::new().title("renamed");
    if let Some(title) = value_update.title.value() {
      meta.title = title.clone();
    }
    assert_eq!(meta.title, "renamed");

    let null_update = SessionInfoUpdate::new().title(None::<String>);
    if null_update.title.is_null() {
      meta.title.clear();
    }
    assert!(meta.title.is_empty());

    let undefined_update = SessionInfoUpdate::new();
    let snapshot = meta.title.clone();
    if let Some(title) = undefined_update.title.value() {
      meta.title = title.clone();
    } else if undefined_update.title.is_null() {
      meta.title.clear();
    }
    assert_eq!(meta.title, snapshot);
  }

  #[test]
  fn plan_view_from_acp_maps_status_and_priority() {
    use agent_client_protocol::schema::PlanEntry;
    let plan = Plan::new(vec![
      PlanEntry::new(
        "do thing",
        PlanEntryPriority::High,
        PlanEntryStatus::InProgress,
      ),
      PlanEntry::new(
        "done thing",
        PlanEntryPriority::Low,
        PlanEntryStatus::Completed,
      ),
    ]);
    let view = plan_view_from_acp(&plan);
    assert_eq!(view.entries.len(), 2);
    assert_eq!(view.entries[0].content, "do thing");
    assert_eq!(view.entries[0].priority, PlanEntryPriorityView::High);
    assert_eq!(view.entries[0].status, PlanEntryStatusView::InProgress);
    assert_eq!(view.entries[1].status, PlanEntryStatusView::Completed);
    assert_eq!(view.entries[1].priority, PlanEntryPriorityView::Low);
  }

  #[test]
  fn permission_destructive_kinds_match() {
    assert!(permission_option_is_destructive(
      &PermissionOptionKind::RejectOnce
    ));
    assert!(permission_option_is_destructive(
      &PermissionOptionKind::RejectAlways
    ));
    assert!(!permission_option_is_destructive(
      &PermissionOptionKind::AllowOnce
    ));
    assert!(!permission_option_is_destructive(
      &PermissionOptionKind::AllowAlways
    ));
  }

  #[test]
  fn prune_old_state_deletes_files_older_than_threshold() {
    let dir = temp_dir("agent-prune");
    std::fs::write(dir.join("a.json"), "[]").unwrap();
    std::fs::write(dir.join("b.json"), "[]").unwrap();
    // sleep a hair so files have age > 0
    std::thread::sleep(std::time::Duration::from_millis(20));
    let pruned = AgentChatPanel::prune_old_state(&dir, std::time::Duration::from_millis(1));
    assert_eq!(pruned, 2);
    assert!(!dir.join("a.json").exists());
    assert!(!dir.join("b.json").exists());
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn prune_old_state_keeps_recent_files() {
    let dir = temp_dir("agent-prune-keep");
    std::fs::write(dir.join("fresh.json"), "[]").unwrap();
    let pruned = AgentChatPanel::prune_old_state(&dir, std::time::Duration::from_secs(60));
    assert_eq!(pruned, 0);
    assert!(dir.join("fresh.json").exists());
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn list_conversations_sorted_by_updated_at_desc() {
    let dir = temp_dir("agent-sort");
    let mk = |id: &str, started: u64, updated: u64| {
      let conv = PersistedConversation {
        meta: ConversationMeta {
          id: id.to_string(),
          started_at_secs: started,
          updated_at_secs: updated,
          title: id.to_string(),
          message_count: 1,
          session_id: None,
        },
        items: vec![PersistedChatItem::Message(ChatMessage {
          role: ChatRole::User,
          text: "hi".into(),
        })],
      };
      std::fs::write(
        dir.join(format!("{id}.json")),
        serde_json::to_string(&conv).unwrap(),
      )
      .unwrap();
    };
    mk("old", 1000, 1000);
    mk("recent_started_stale", 5000, 5000);
    mk("old_started_recent_updated", 2000, 9000);

    let metas = list_conversations_in(&dir);
    assert_eq!(metas[0].id, "old_started_recent_updated");
    assert_eq!(metas[1].id, "recent_started_stale");
    assert_eq!(metas[2].id, "old");
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn short_model_label_extracts_identifier_from_description() {
    assert_eq!(
      short_model_label(
        "Default (recommended)",
        Some("Opus 4.7 with 1M context · Most capable for complex work"),
      ),
      "Opus 4.7"
    );
    assert_eq!(
      short_model_label("Sonnet", Some("Sonnet 4.6 · Best for everyday tasks")),
      "Sonnet 4.6"
    );
    assert_eq!(
      short_model_label("Haiku", Some("Haiku 4.5 · Fastest for quick answers")),
      "Haiku 4.5"
    );
  }

  #[test]
  fn short_model_label_falls_back_to_name_when_no_description() {
    assert_eq!(short_model_label("Sonnet", None), "Sonnet");
    assert_eq!(short_model_label("Sonnet", Some("")), "Sonnet");
  }

  #[test]
  fn short_model_label_uses_name_when_description_has_no_separator() {
    // The default-effort suffix is dropped: the effort selector shows the applied value.
    assert_eq!(
      short_model_label(
        "gpt-5.2-codex (high)",
        Some("Frontier agentic coding model. Greater reasoning depth for complex problems"),
      ),
      "gpt-5.2-codex"
    );
    assert_eq!(
      short_model_label(
        "GPT-5.5 (high)",
        Some(
          "Frontier model for complex coding, research, and real-world work. Greater reasoning depth for complex problems"
        ),
      ),
      "GPT-5.5"
    );
  }

  fn select_option(
    id: &str,
    name: &str,
    current: &str,
    values: &[&str],
    category: Option<SessionConfigOptionCategory>,
  ) -> SessionConfigOption {
    let mut option = SessionConfigOption::new(
      SessionConfigId::new(std::sync::Arc::from(id)),
      name.to_string(),
      SessionConfigKind::Select(SessionConfigSelect::new(
        SessionConfigValueId::new(std::sync::Arc::from(current)),
        values
          .iter()
          .map(|value| {
            SessionConfigSelectOption::new(
              SessionConfigValueId::new(std::sync::Arc::from(*value)),
              value.to_string(),
            )
          })
          .collect::<Vec<_>>(),
      )),
    );
    option.category = category;
    option
  }

  #[test]
  fn selectable_config_options_skips_model_and_mode_categories() {
    let options = vec![
      select_option(
        "model",
        "Model",
        "gpt-5.5",
        &["gpt-5.5"],
        Some(SessionConfigOptionCategory::Model),
      ),
      select_option(
        "mode",
        "Mode",
        "agent",
        &["agent"],
        Some(SessionConfigOptionCategory::Mode),
      ),
      select_option("effort", "Reasoning effort", "low", &["low", "high"], None),
    ];

    let selectors = selectable_config_options(&options);

    assert_eq!(selectors.len(), 1);
    assert_eq!(selectors[0].name.as_ref(), "Reasoning effort");
    assert_eq!(selectors[0].current_label, "low");
  }

  #[test]
  fn selectable_config_options_falls_back_to_the_option_name_for_unknown_values() {
    let options = vec![select_option(
      "sandbox",
      "Sandbox",
      "gone",
      &["off", "on"],
      None,
    )];

    let selectors = selectable_config_options(&options);

    assert_eq!(selectors[0].current_label, "Sandbox");
  }

  #[test]
  fn config_summary_joins_effective_values() {
    let options = vec![
      select_option("effort", "Effort", "high", &["low", "high"], None),
      select_option("sandbox", "Sandbox", "off", &["off", "on"], None),
    ];

    let selectors = selectable_config_options(&options);

    assert_eq!(config_summary(&selectors), "high · off");
    assert_eq!(config_summary(&[]), "");
  }

  #[test]
  fn config_customized_only_when_a_value_left_its_advertised_default() {
    let options = vec![select_option(
      "effort",
      "Effort",
      "low",
      &["low", "high"],
      None,
    )];
    let selectors = selectable_config_options(&options);
    let mut defaults = HashMap::new();
    defaults.insert(
      selectors[0].id.clone(),
      SessionConfigValueId::new(std::sync::Arc::from("low")),
    );

    assert!(!config_customized(&selectors, &defaults));

    let changed = selectable_config_options(&[select_option(
      "effort",
      "Effort",
      "high",
      &["low", "high"],
      None,
    )]);
    assert!(config_customized(&changed, &defaults));
  }

  #[test]
  fn config_customized_is_false_without_a_recorded_default() {
    let selectors = selectable_config_options(&[select_option(
      "effort",
      "Effort",
      "high",
      &["low", "high"],
      None,
    )]);

    assert!(!config_customized(&selectors, &HashMap::new()));
  }

  #[test]
  fn working_word_holds_for_seven_seconds_then_moves_on() {
    let seed = working_word_seed("conv-1");

    assert_eq!(working_word(seed, 0), working_word(seed, 6));
    assert_ne!(working_word(seed, 6), working_word(seed, 7));
    assert_eq!(working_word(seed, 7), working_word(seed, 13));
  }

  #[test]
  fn working_word_differs_between_conversations_at_the_same_moment() {
    let words: std::collections::HashSet<&str> = (0..8)
      .map(|ix| working_word(working_word_seed(&format!("conv-{ix}")), 0))
      .collect();

    assert!(words.len() > 1, "seeds must spread across the vocabulary");
  }

  #[test]
  fn working_word_cycles_through_the_whole_vocabulary() {
    let seed = working_word_seed("conv-1");
    let cycle: std::collections::HashSet<&str> = (0..WORKING_WORDS.len() as u64)
      .map(|step| working_word(seed, step * WORKING_WORD_ROTATE_SECS))
      .collect();

    assert_eq!(cycle.len(), WORKING_WORDS.len());
  }

  #[test]
  fn tool_kind_labels_cover_main_kinds() {
    assert_eq!(tool_kind_label(&ToolKind::Read), "Read");
    assert_eq!(tool_kind_label(&ToolKind::Edit), "Edit");
    assert_eq!(tool_kind_label(&ToolKind::Execute), "Run");
  }

  fn add_panel_window(
    cx: &mut gpui::TestAppContext,
  ) -> (gpui::Entity<AgentChatPanel>, &mut gpui::VisualTestContext) {
    cx.update(gpui_component::init);
    let mut mounted: Option<gpui::Entity<AgentChatPanel>> = None;
    let (_root, cx) = cx.add_window_view(|window, cx| {
      let panel = cx.new(|cx| AgentChatPanel::new_disconnected(PathBuf::from("."), window, cx));
      mounted = Some(panel.clone());
      gpui_component::Root::new(panel, window, cx)
    });
    (mounted.expect("agent chat panel"), cx)
  }

  #[gpui::test]
  async fn mounting_the_panel_spawns_no_agent_and_paints(cx: &mut gpui::TestAppContext) {
    let (panel, cx) = add_panel_window(cx);
    cx.run_until_parked();

    panel.read_with(cx, |panel, _| {
      assert!(panel.session.is_none(), "no agent process was connected");
      assert!(panel._connect_task.is_none());
      // Connecting shows the generating row and nothing else.
      assert_eq!(panel.messages_list.item_count(), 1);
    });
  }

  #[gpui::test]
  async fn a_missing_binary_paints_its_install_hint(cx: &mut gpui::TestAppContext) {
    let (panel, cx) = add_panel_window(cx);
    panel.update(cx, |panel, cx| {
      panel.status = Status::MissingBinary {
        command: "npx".to_string(),
        hint: "Install Node.js to get npx".to_string(),
      };
      panel.sync_list_count();
      cx.notify();
    });
    cx.run_until_parked();

    assert!(
      cx.debug_bounds("agent-chat-missing-binary").is_some(),
      "the notice is painted"
    );
    panel.read_with(cx, |panel, _| {
      assert_eq!(panel.messages_list.item_count(), 1);
    });
  }

  #[gpui::test]
  async fn a_loaded_conversation_renders_one_row_per_item(cx: &mut gpui::TestAppContext) {
    let (panel, cx) = add_panel_window(cx);
    panel.update(cx, |panel, cx| {
      panel.status = Status::Ready;
      panel.items = vec![user_message("hello"), agent_message("hi there")];
      panel.sync_list_count();
      cx.notify();
    });
    cx.run_until_parked();

    panel.read_with(cx, |panel, _| {
      // Ready and idle: no generating row, one list row per message.
      assert_eq!(panel.messages_list.item_count(), 2);
    });
  }

  #[gpui::test]
  async fn typing_enter_without_a_session_sends_nothing(cx: &mut gpui::TestAppContext) {
    let (panel, cx) = add_panel_window(cx);
    cx.run_until_parked();

    let input_focus = panel.read_with(cx, |panel, cx| panel.input.read(cx).focus_handle(cx));
    cx.update(|window, cx| window.focus(&input_focus, cx));
    cx.simulate_input("do the thing");
    panel.read_with(cx, |panel, cx| {
      assert_eq!(panel.input.read(cx).value(), "do the thing");
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();

    panel.read_with(cx, |panel, cx| {
      // Nothing was dispatched, and the composer keeps the user's text.
      assert_eq!(panel.input.read(cx).value(), "do the thing");
      assert!(panel.items.is_empty(), "no prompt was recorded");
      assert!(!panel.in_flight);
    });
  }
}
