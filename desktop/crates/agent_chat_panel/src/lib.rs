use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use agent_acp::{
  AgentEvent, AgentSession, AuthMethodInfo, BackendAvailability, BackendConfig, BackendKind,
  PermissionOptionKind, PermissionPrompt,
};
use agent_client_protocol::schema::{
  ContentBlock, ToolCall, ToolCallContent, ToolCallId, ToolCallStatus, ToolCallUpdate, ToolKind,
};
use agent_client_protocol::schema::{
  ModelId, ModelInfo, SessionConfigId, SessionConfigKind, SessionConfigOption,
  SessionConfigOptionCategory, SessionConfigOptionValue, SessionConfigSelectOptions,
  SessionConfigValueId, SessionMode, SessionModeId,
};
use futures::future::BoxFuture;
use gfm_markdown_viewer::{
  LinkAction, MarkdownRenderOptions, SyntaxHighlightCache, render_markdown,
};
use gpui::Corner;
use gpui::{
  Context, Entity, FocusHandle, Focusable, IntoElement, ParentElement, Render, ScrollHandle,
  SharedString, Styled, Task, Window, div, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Disableable as _, IconName, Sizable as _,
  button::{Button, ButtonVariants as _},
  h_flex,
  input::InputEvent,
  menu::{DropdownMenu as _, PopupMenuItem},
  scroll::ScrollableElement as _,
  spinner::Spinner,
  v_flex,
};
use ui::{Input, InputState, StatusThemeExt as _, UiIconName};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum ChatRole {
  User,
  Agent,
  System,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct ChatMessage {
  role: ChatRole,
  text: String,
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
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct DiffSummary {
  path: String,
  added: u32,
  removed: u32,
  #[serde(default)]
  lines: Vec<DiffLine>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct DiffLine {
  kind: DiffLineKind,
  text: String,
  #[serde(default)]
  spans: Vec<InlineSpan>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
enum DiffLineKind {
  Added,
  Removed,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
enum InlineSpanKind {
  Same,
  Diff,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct InlineSpan {
  kind: InlineSpanKind,
  text: String,
}

#[derive(Clone, Debug)]
struct PermissionItem {
  prompt: PermissionPrompt,
  resolved: Option<String>,
}

#[derive(Clone, Debug)]
enum ChatItem {
  Message(ChatMessage),
  Tool(ToolCallView),
  Permission(PermissionItem),
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
enum PersistedChatItem {
  Message(ChatMessage),
  Tool(ToolCallView),
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ConversationMeta {
  pub id: String,
  pub started_at_secs: u64,
  pub updated_at_secs: u64,
  pub title: String,
  pub message_count: usize,
  #[serde(default)]
  pub session_id: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PersistedConversation {
  meta: ConversationMeta,
  items: Vec<PersistedChatItem>,
}

enum Status {
  Connecting,
  Ready,
  Error(String),
  MissingBinary { command: String, hint: String },
}

pub struct AgentChatPanel {
  backend_kind: BackendKind,
  backend: BackendConfig,
  cwd: PathBuf,
  status: Status,
  items: Vec<ChatItem>,
  tool_index: HashMap<ToolCallId, usize>,
  pending_agent: String,
  session: Option<Arc<AgentSession>>,
  input: Entity<InputState>,
  in_flight: bool,
  scroll_handle: ScrollHandle,
  usage: Option<(u64, u64)>,
  agent_version: Option<String>,
  auth_methods: Vec<AuthMethodInfo>,
  auth_required: bool,
  state_dir: Option<PathBuf>,
  current_conv: ConversationMeta,
  syntax_cache: Arc<SyntaxHighlightCache>,
  available_modes: Vec<SessionMode>,
  current_mode_id: Option<SessionModeId>,
  available_models: Vec<ModelInfo>,
  current_model_id: Option<ModelId>,
  config_options: Vec<SessionConfigOption>,
  _connect_task: Option<Task<()>>,
  _events_task: Option<Task<()>>,
  _permission_task: Option<Task<()>>,
  _input_sub: Option<gpui::Subscription>,
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
      InputState::new(window, cx)
        .auto_grow(1, 8)
        .placeholder("Message...")
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

    let mut panel = Self {
      backend_kind,
      backend: backend.clone(),
      cwd: cwd.clone(),
      status: Status::Connecting,
      items: loaded_items,
      tool_index: loaded_index,
      pending_agent: String::new(),
      session: None,
      input,
      in_flight: false,
      scroll_handle: ScrollHandle::new(),
      usage: None,
      agent_version: None,
      auth_methods: Vec::new(),
      auth_required: false,
      state_dir,
      current_conv,
      syntax_cache: Arc::new(SyntaxHighlightCache::new()),
      available_modes: Vec::new(),
      current_mode_id: None,
      available_models: Vec::new(),
      current_model_id: None,
      config_options: Vec::new(),
      _connect_task: None,
      _events_task: None,
      _permission_task: None,
      _input_sub: Some(input_sub),
    };

    if let BackendAvailability::MissingBinary {
      command,
      install_hint,
    } = backend.check_availability()
    {
      panel.status = Status::MissingBinary {
        command,
        hint: install_hint,
      };
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
            panel.config_options = info.config_options;
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
            cx.notify();
          });
        }
        Err(e) => {
          let msg = format!("{e}");
          let _ = this.update(cx, |panel, cx| {
            panel.status = Status::Error(msg);
            cx.notify();
          });
        }
      }
    });
    panel._connect_task = Some(task);
    panel
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
          panel.scroll_handle.scroll_to_bottom();
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
      self.status = Status::Error("Agent disconnected".into());
      self.items.push(ChatItem::Message(ChatMessage {
        role: ChatRole::System,
        text: "Agent disconnected. Toggle the panel to reconnect.".into(),
      }));
      self.in_flight = false;
      self.session = None;
      cx.notify();
    }
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
          panel.scroll_handle.scroll_to_bottom();
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
    for item in self.items.iter_mut() {
      if let ChatItem::Permission(p) = item
        && p.prompt.id == prompt_id
      {
        p.resolved = Some(option_id.clone().unwrap_or_else(|| "cancel".into()));
      }
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
      }
      AgentEvent::AgentThoughtChunk(_) => {
        if !self.in_flight {
          return;
        }
      }
      AgentEvent::ToolCall(call) => {
        self.upsert_tool_call(call);
      }
      AgentEvent::ToolCallUpdate(update) => {
        self.apply_tool_call_update(update);
      }
      AgentEvent::UsageUpdate(usage) => {
        self.usage = Some((usage.used, usage.size));
      }
      AgentEvent::CurrentModeUpdate(u) => {
        self.current_mode_id = Some(u.current_mode_id);
      }
      AgentEvent::ConfigOptionUpdate(u) => {
        self.config_options = u.config_options;
      }
      _ => {}
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
    self.current_model_id = Some(model_id.clone());
    cx.notify();
    cx.spawn(async move |_, _| {
      let _ = session.set_model(model_id).await;
    })
    .detach();
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

  fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let text = self.input.read(cx).value().to_string();
    let text = text.trim().to_string();
    if text.is_empty() {
      return;
    }
    self.input.update(cx, |state, cx| {
      state.set_value("", window, cx);
    });
    self.dispatch_prompt(text, cx);
  }

  /// Send a prompt programmatically; false if not ready or already in flight.
  pub fn send_external_prompt(&mut self, text: String, cx: &mut Context<Self>) -> bool {
    let text = text.trim().to_string();
    if text.is_empty() {
      return false;
    }
    self.dispatch_prompt(text, cx)
  }

  fn dispatch_prompt(&mut self, text: String, cx: &mut Context<Self>) -> bool {
    if self.in_flight {
      return false;
    }
    let Some(session) = self.session.clone() else {
      return false;
    };

    self.items.push(ChatItem::Message(ChatMessage {
      role: ChatRole::User,
      text: text.clone(),
    }));
    self.pending_agent.clear();
    self.in_flight = true;
    self.persist_state();
    self.scroll_handle.scroll_to_bottom();
    cx.notify();

    cx.spawn(async move |this, cx| {
      let result = session.send_prompt(text).await;
      let _ = this.update(cx, |panel, cx| {
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
              panel.items.push(ChatItem::Message(ChatMessage {
                role: ChatRole::System,
                text: format!("[error] {e}"),
              }));
            }
          }
        }
        panel.in_flight = false;
        panel.persist_state();
        panel.scroll_handle.scroll_to_bottom();
        cx.notify();
      });
    })
    .detach();

    true
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
    self.in_flight = false;
    self.usage = None;
    self.respawn_session(cx);
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
      self.in_flight = false;
      self.usage = None;
      self.respawn_session(cx);
    }
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
    let _ = std::fs::write(dir.join("active.txt"), &self.current_conv.id);
    self.respawn_session(cx);
    cx.notify();
  }

  fn respawn_session(&mut self, cx: &mut Context<Self>) {
    let load_session_id = self.current_conv.session_id.clone();
    self.respawn_session_with(load_session_id, cx);
  }

  fn respawn_session_with(&mut self, load_session_id: Option<String>, cx: &mut Context<Self>) {
    self.session = None;
    self.in_flight = false;
    self.auth_required = false;
    self.auth_methods.clear();
    self.agent_version = None;
    self.usage = None;
    self.available_modes.clear();
    self.current_mode_id = None;
    self.available_models.clear();
    self.current_model_id = None;
    self.config_options.clear();
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
      return;
    }
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
            panel.config_options = info.config_options;
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
            cx.notify();
          });
        }
        Err(e) => {
          let msg = format!("{e}");
          let _ = this.update(cx, |panel, cx| {
            panel.status = Status::Error(msg);
            cx.notify();
          });
        }
      }
    });
    self._connect_task = Some(task);
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
    if self.current_conv.title.is_empty() {
      if let Some(first_user) = self.items.iter().find_map(|i| match i {
        ChatItem::Message(m) if matches!(m.role, ChatRole::User) => Some(m.text.clone()),
        _ => None,
      }) {
        self.current_conv.title = truncate_title(&first_user);
      }
    }
    let persisted: Vec<PersistedChatItem> = self
      .items
      .iter()
      .filter_map(|item| match item {
        ChatItem::Message(m) => Some(PersistedChatItem::Message(m.clone())),
        ChatItem::Tool(t) => Some(PersistedChatItem::Tool(t.clone())),
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

  /// Per-repo directory hosting one file per conversation + an `active.txt` pointer.
  pub fn state_dir_for_repo(state_dir: &std::path::Path, repo: &std::path::Path) -> PathBuf {
    let canonical = repo.canonicalize().unwrap_or_else(|_| repo.to_path_buf());
    let hash = blake3::hash(canonical.to_string_lossy().as_bytes());
    let hex = hash.to_hex();
    state_dir.join(&hex.as_str()[..16])
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

pub fn persist_choice(kind: BackendKind) {
  let Some(dir) = dirs::config_dir() else {
    return;
  };
  let path = dir.join("reviu").join("agent.json");
  if let Some(parent) = path.parent() {
    let _ = std::fs::create_dir_all(parent);
  }
  let body = serde_json::json!({ "backend": kind.storage_key() });
  let _ = std::fs::write(&path, body.to_string());
}

fn new_conversation_meta() -> ConversationMeta {
  let now = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_secs())
    .unwrap_or(0);
  ConversationMeta {
    id: now.to_string(),
    started_at_secs: now,
    updated_at_secs: now,
    title: String::new(),
    message_count: 0,
    session_id: None,
  }
}

fn now_secs() -> u64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_secs())
    .unwrap_or(0)
}

fn truncate_title(text: &str) -> String {
  let trimmed = text.trim().lines().next().unwrap_or("").trim();
  let max = 80;
  if trimmed.chars().count() <= max {
    trimmed.to_string()
  } else {
    let head: String = trimmed.chars().take(max).collect();
    format!("{head}...")
  }
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
  metas.sort_by(|a, b| b.updated_at_secs.cmp(&a.updated_at_secs));
  metas
}

fn extract_diffs(content: &[ToolCallContent], cwd: &std::path::Path) -> Vec<DiffSummary> {
  content
    .iter()
    .filter_map(|c| match c {
      ToolCallContent::Diff(d) => {
        let (added, removed) = diff_line_counts(d.old_text.as_deref(), &d.new_text);
        let lines = build_diff_lines(d.old_text.as_deref().unwrap_or(""), &d.new_text);
        Some(DiffSummary {
          path: relativize_path(&d.path, cwd),
          added,
          removed,
          lines,
        })
      }
      _ => None,
    })
    .collect()
}

fn build_diff_lines(old: &str, new: &str) -> Vec<DiffLine> {
  use imara_diff::{Algorithm, Diff, InternedInput};
  let input = InternedInput::new(old, new);
  let diff = Diff::compute(Algorithm::Histogram, &input);
  let old_lines: Vec<&str> = old.lines().collect();
  let new_lines: Vec<&str> = new.lines().collect();
  let mut out = Vec::new();
  for hunk in diff.hunks() {
    let removed: Vec<&str> = hunk
      .before
      .clone()
      .filter_map(|i| old_lines.get(i as usize).copied())
      .collect();
    let added: Vec<&str> = hunk
      .after
      .clone()
      .filter_map(|i| new_lines.get(i as usize).copied())
      .collect();
    let paired = removed.len().min(added.len());
    for k in 0..paired {
      let (old_spans, new_spans) = word_diff_spans(removed[k], added[k]);
      out.push(DiffLine {
        kind: DiffLineKind::Removed,
        text: removed[k].to_string(),
        spans: old_spans,
      });
      out.push(DiffLine {
        kind: DiffLineKind::Added,
        text: added[k].to_string(),
        spans: new_spans,
      });
    }
    for line in removed.iter().skip(paired) {
      out.push(DiffLine {
        kind: DiffLineKind::Removed,
        text: (*line).to_string(),
        spans: Vec::new(),
      });
    }
    for line in added.iter().skip(paired) {
      out.push(DiffLine {
        kind: DiffLineKind::Added,
        text: (*line).to_string(),
        spans: Vec::new(),
      });
    }
  }
  out
}

const WORD_DIFF_MAX_COMBINED_BYTES: usize = 2_048;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IdentifierCharKind {
  Lower,
  Upper,
  Digit,
  Underscore,
  Other,
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

fn split_identifier_token_ranges(segment: &str) -> Vec<std::ops::Range<usize>> {
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

fn word_tokens(text: &str, include_whitespace: bool) -> Vec<&str> {
  use unicode_segmentation::UnicodeSegmentation;
  let mut tokens = Vec::new();
  for (_idx, segment) in text.split_word_bound_indices() {
    if !include_whitespace && segment.trim().is_empty() {
      continue;
    }
    let subranges = split_identifier_token_ranges(segment);
    if subranges.is_empty() {
      tokens.push(segment);
      continue;
    }
    for subrange in subranges {
      tokens.push(&segment[subrange]);
    }
  }
  tokens
}

fn word_diff_spans(old: &str, new: &str) -> (Vec<InlineSpan>, Vec<InlineSpan>) {
  if old == new {
    let same_old = vec![InlineSpan {
      kind: InlineSpanKind::Same,
      text: old.to_string(),
    }];
    let same_new = vec![InlineSpan {
      kind: InlineSpanKind::Same,
      text: new.to_string(),
    }];
    return (same_old, same_new);
  }
  if old.len().saturating_add(new.len()) > WORD_DIFF_MAX_COMBINED_BYTES {
    return (Vec::new(), Vec::new());
  }
  word_diff_spans_impl(old, new)
}

fn word_diff_spans_impl(old: &str, new: &str) -> (Vec<InlineSpan>, Vec<InlineSpan>) {
  let a = word_tokens(old, true);
  let b = word_tokens(new, true);
  let n = a.len();
  let m = b.len();
  let mut dp = vec![vec![0u32; m + 1]; n + 1];
  for i in 0..n {
    for j in 0..m {
      dp[i + 1][j + 1] = if a[i] == b[j] {
        dp[i][j] + 1
      } else {
        dp[i + 1][j].max(dp[i][j + 1])
      };
    }
  }
  let mut old_spans: Vec<InlineSpan> = Vec::new();
  let mut new_spans: Vec<InlineSpan> = Vec::new();
  let mut i = n;
  let mut j = m;
  while i > 0 || j > 0 {
    if i > 0 && j > 0 && a[i - 1] == b[j - 1] {
      old_spans.push(InlineSpan {
        kind: InlineSpanKind::Same,
        text: a[i - 1].to_string(),
      });
      new_spans.push(InlineSpan {
        kind: InlineSpanKind::Same,
        text: b[j - 1].to_string(),
      });
      i -= 1;
      j -= 1;
    } else if j > 0 && (i == 0 || dp[i][j - 1] >= dp[i - 1][j]) {
      new_spans.push(InlineSpan {
        kind: InlineSpanKind::Diff,
        text: b[j - 1].to_string(),
      });
      j -= 1;
    } else {
      old_spans.push(InlineSpan {
        kind: InlineSpanKind::Diff,
        text: a[i - 1].to_string(),
      });
      i -= 1;
    }
  }
  old_spans.reverse();
  new_spans.reverse();
  (
    merge_adjacent_spans(old_spans),
    merge_adjacent_spans(new_spans),
  )
}

fn merge_adjacent_spans(spans: Vec<InlineSpan>) -> Vec<InlineSpan> {
  let mut out: Vec<InlineSpan> = Vec::new();
  for s in spans {
    if let Some(last) = out.last_mut()
      && last.kind == s.kind
    {
      last.text.push_str(&s.text);
      continue;
    }
    out.push(s);
  }
  out
}

fn diff_line_counts(old_text: Option<&str>, new_text: &str) -> (u32, u32) {
  use imara_diff::{Algorithm, Diff, InternedInput};
  let old = old_text.unwrap_or("");
  let input = InternedInput::new(old, new_text);
  let diff = Diff::compute(Algorithm::Histogram, &input);
  let mut added = 0u32;
  let mut removed = 0u32;
  for hunk in diff.hunks() {
    added += hunk.after.end - hunk.after.start;
    removed += hunk.before.end - hunk.before.start;
  }
  (added, removed)
}

fn upsert_tool_call_pure(
  items: &mut Vec<ChatItem>,
  index: &mut HashMap<ToolCallId, usize>,
  call: ToolCall,
  cwd: &std::path::Path,
) {
  let diffs = extract_diffs(&call.content, cwd);
  let view = ToolCallView {
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
  };
  if let Some(&idx) = index.get(&call.tool_call_id) {
    if let Some(ChatItem::Tool(existing)) = items.get_mut(idx) {
      *existing = view;
      return;
    }
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
  }
}

fn relativize_path(path: &std::path::Path, cwd: &std::path::Path) -> String {
  if let Ok(stripped) = path.strip_prefix(cwd) {
    return stripped.display().to_string();
  }
  if let (Ok(canon_path), Ok(canon_cwd)) = (path.canonicalize(), cwd.canonicalize())
    && let Ok(stripped) = canon_path.strip_prefix(&canon_cwd)
  {
    return stripped.display().to_string();
  }
  path.display().to_string()
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
            .when(is_current, |this| this.font_weight(gpui::FontWeight::BOLD))
            .child(name),
        )
        .when_some(description, |this, d| {
          this.child(div().text_xs().text_color(theme.muted_foreground).child(d))
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
  let stripped = t
    .title
    .strip_prefix(kind)
    .map(|s| s.trim_start().to_string())
    .unwrap_or_else(|| t.title.clone());
  stripped
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

fn render_tool_call(t: &ToolCallView, theme: &gpui_component::Theme) -> gpui::AnyElement {
  let title_color = match t.status {
    ToolCallStatus::Failed => theme.danger,
    ToolCallStatus::InProgress => theme.warning,
    _ => theme.foreground,
  };
  let detail = tool_detail_label(t);

  v_flex()
    .gap_1()
    .child(
      h_flex()
        .gap_2()
        .items_center()
        .flex_wrap()
        .child(
          div()
            .text_sm()
            .font_weight(gpui::FontWeight::BOLD)
            .text_color(title_color)
            .child(tool_kind_label(&t.kind).to_string()),
        )
        .when(!detail.is_empty(), |this| {
          this.child(
            div()
              .text_sm()
              .text_color(theme.muted_foreground)
              .child(detail.clone()),
          )
        }),
    )
    .when(!t.diffs.is_empty(), |this| {
      let mut diff_col = v_flex().gap_2();
      for d in &t.diffs {
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
          let mut body = v_flex()
            .font_family("monospace")
            .text_xs()
            .bg(theme.background)
            .border_1()
            .border_color(theme.border)
            .rounded(px(3.))
            .overflow_hidden();
          let ui_theme = ui::Theme::new(theme.is_dark());
          for line in &d.lines {
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
            let text_col: gpui::AnyElement = if line.spans.is_empty() {
              div().flex_1().child(line.text.clone()).into_any_element()
            } else {
              let mut row = h_flex().flex_1().flex_wrap();
              for span in &line.spans {
                let mut chunk = div().child(span.text.clone());
                if span.kind == InlineSpanKind::Diff {
                  chunk = chunk.bg(hl_bg);
                }
                row = row.child(chunk);
              }
              row.into_any_element()
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
        }
        diff_col = diff_col.child(block);
      }
      this.child(diff_col)
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
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let theme = &theme;
    let md_options =
      MarkdownRenderOptions::with_on_link(Arc::new(|_url, _window, _cx| LinkAction::Open))
        .with_syntax_cache(self.syntax_cache.clone());

    let _ = SharedString::from("");

    let usage_text: Option<SharedString> = self.usage.map(|(used, size)| {
      let used_k = used as f64 / 1000.0;
      let size_k = size as f64 / 1000.0;
      format!("{used_k:.1}k / {size_k:.0}k").into()
    });

    let mut messages = v_flex().p_3();

    if let Status::MissingBinary { command, hint } = &self.status {
      messages = messages.child(
        v_flex()
          .gap_2()
          .p_3()
          .border_1()
          .border_color(theme.border)
          .rounded(px(4.))
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
          ),
      );
    }

    if let Status::Error(e) = &self.status {
      messages = messages.child(
        div()
          .text_sm()
          .text_color(theme.danger)
          .child(format!("Failed to start agent: {e}")),
      );
    }

    if matches!(self.status, Status::Ready) && self.auth_required && !self.auth_methods.is_empty() {
      let executable = self.backend.command;
      let mut card = v_flex()
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
      messages = messages.child(card);
    }

    let has_trailer = !self.pending_agent.is_empty() || self.in_flight;
    let total_items = self.items.len();
    let is_user_message = |item: &ChatItem| {
      matches!(
        item,
        ChatItem::Message(ChatMessage {
          role: ChatRole::User,
          ..
        })
      )
    };
    for (idx, item) in self.items.iter().enumerate() {
      let next_is_user = self
        .items
        .get(idx + 1)
        .map(is_user_message)
        .unwrap_or(false);
      let is_end_of_group = if idx + 1 == total_items {
        !has_trailer
      } else {
        next_is_user
      };
      let is_last_row = is_end_of_group;
      match item {
        ChatItem::Message(m) => match m.role {
          ChatRole::User => {
            messages = messages.child(
              div()
                .px_3()
                .py_2()
                .mb_3()
                .rounded(theme.radius)
                .bg(theme.input_background())
                .border_1()
                .border_color(theme.border)
                .text_sm()
                .text_color(theme.foreground)
                .child(m.text.clone()),
            );
          }
          ChatRole::Agent => {
            messages = messages.child(timeline_row(
              render_markdown(&m.text, &md_options, cx),
              theme,
              is_last_row,
            ));
          }
          ChatRole::System => {
            messages = messages.child(timeline_row(
              div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(m.text.clone())
                .into_any_element(),
              theme,
              is_last_row,
            ));
          }
        },
        ChatItem::Tool(t) => {
          let bullet = match t.status {
            ToolCallStatus::Completed => theme.status_green(),
            ToolCallStatus::Failed => theme.danger,
            ToolCallStatus::InProgress => theme.warning,
            _ => theme.muted_foreground,
          };
          messages = messages.child(timeline_row_with_color(
            render_tool_call(t, theme),
            theme,
            bullet,
            is_last_row,
          ));
        }
        ChatItem::Permission(p) => {
          messages = messages.child(timeline_row(
            render_permission(p, theme, cx),
            theme,
            is_last_row,
          ));
        }
      }
    }

    if !self.pending_agent.is_empty() {
      messages = messages.child(
        v_flex()
          .gap_1()
          .child(
            div()
              .text_xs()
              .text_color(theme.muted_foreground)
              .child(format!("{} (typing...)", self.backend.label)),
          )
          .child(render_markdown(&self.pending_agent, &md_options, cx)),
      );
    } else if self.in_flight {
      messages = messages.child(
        h_flex()
          .gap_2()
          .items_center()
          .child(Spinner::new().xsmall().color(theme.muted_foreground))
          .child(
            div()
              .text_xs()
              .text_color(theme.muted_foreground)
              .child(format!("{} is thinking...", self.backend.label)),
          ),
      );
    }

    let show_empty_state = self.items.is_empty()
      && self.pending_agent.is_empty()
      && !self.in_flight
      && matches!(self.status, Status::Ready | Status::Connecting);
    let empty_state = if show_empty_state {
      Some(
        v_flex()
          .flex_1()
          .min_h_0()
          .items_center()
          .justify_center()
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
          ),
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
              Status::Connecting => " (connecting...)",
              Status::Error(_) => " (error)",
              Status::MissingBinary { .. } => " (not installed)",
              Status::Ready => "",
            };
            let label = format!("{}{}", current.label(), label_suffix);
            let entity = cx.entity().downgrade();
            Button::new("agent-chat-backend")
              .label(label)
              .icon(IconName::ChevronDown)
              .small()
              .ghost()
              .dropdown_menu_with_anchor(Corner::TopLeft, move |menu, _, _| {
                let mut menu = menu;
                for kind in BackendKind::all() {
                  let kind = *kind;
                  let entity = entity.clone();
                  let is_current = kind == current;
                  let label_text: SharedString = kind.label().into();
                  menu = menu.item(
                    PopupMenuItem::element(move |_, cx| {
                      let theme = cx.theme().clone();
                      h_flex()
                        .w_full()
                        .gap_2()
                        .items_center()
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
              .child({
                let entity = cx.entity().downgrade();
                let conversations = self.list_conversations();
                let current_id = self.current_conv.id.clone();
                Button::new("agent-chat-history")
                  .icon(UiIconName::History)
                  .small()
                  .ghost()
                  .disabled(conversations.is_empty())
                  .dropdown_menu_with_anchor(Corner::TopRight, move |menu, _, _| {
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
              .child(
                Button::new("agent-chat-new")
                  .icon(UiIconName::MessageCirclePlus)
                  .small()
                  .ghost()
                  .on_click(cx.listener(|panel, _, _, cx| panel.new_conversation(cx))),
              ),
          ),
      )
      .map(|this| {
        if let Some(empty_state) = empty_state {
          this.child(empty_state)
        } else {
          this.child(
            div()
              .flex_1()
              .min_h_0()
              .relative()
              .child(
                div()
                  .id("agent-chat-messages")
                  .size_full()
                  .overflow_y_scroll()
                  .track_scroll(&self.scroll_handle)
                  .child(messages),
              )
              .vertical_scrollbar(&self.scroll_handle),
          )
        }
      })
      .child(
        v_flex()
          .flex_shrink_0()
          .p_2()
          .gap_2()
          .bg(theme.sidebar)
          .border_t_1()
          .border_color(theme.border)
          .child(Input::new(&self.input).w_full())
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
                  .children(self.render_config_option_selectors(cx)),
              )
              .child(if self.in_flight {
                Button::new("agent-chat-stop")
                  .label("Stop")
                  .small()
                  .danger()
                  .on_click(cx.listener(|panel, _, _, cx| panel.cancel(cx)))
              } else {
                Button::new("agent-chat-send")
                  .label("Send")
                  .small()
                  .primary()
                  .disabled(!matches!(self.status, Status::Ready))
                  .on_click(cx.listener(|panel, _, window, cx| panel.submit(window, cx)))
              }),
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
      .map(|m| m.name.clone().into())
      .unwrap_or_else(|| "Model".into());
    let entity = cx.entity().downgrade();
    Button::new("agent-chat-model")
      .label(current_label)
      .icon(IconName::ChevronDown)
      .xsmall()
      .ghost()
      .disabled(models.is_empty())
      .dropdown_menu_with_anchor(Corner::BottomLeft, move |menu, _, _| {
        let mut menu = menu.label("Select a model");
        for m in models.iter() {
          let model_id = m.model_id.clone();
          let entity = entity.clone();
          let is_current = current_id.as_ref() == Some(&model_id);
          let label_text: SharedString = m.name.clone().into();
          let description: Option<SharedString> = m.description.clone().map(Into::into);
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
      .label(current_label)
      .icon(IconName::ChevronDown)
      .xsmall()
      .ghost()
      .disabled(modes.is_empty())
      .dropdown_menu_with_anchor(Corner::BottomLeft, move |menu, _, _| {
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

  fn render_config_option_selectors(&self, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
    let mut out: Vec<gpui::AnyElement> = Vec::new();
    for opt in &self.config_options {
      if matches!(
        opt.category,
        Some(SessionConfigOptionCategory::Model) | Some(SessionConfigOptionCategory::Mode)
      ) {
        continue;
      }
      let SessionConfigKind::Select(sel) = &opt.kind else {
        continue;
      };
      let config_id = opt.id.clone();
      let current_value = sel.current_value.clone();
      let flat_options: Vec<(SessionConfigValueId, String, Option<String>)> = match &sel.options {
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
      let current_label: SharedString = flat_options
        .iter()
        .find(|(v, _, _)| v == &current_value)
        .map(|(_, n, _)| n.clone().into())
        .unwrap_or_else(|| opt.name.clone().into());
      let button_id = SharedString::from(format!("agent-chat-cfg-{}", opt.id.0));
      let opt_label: SharedString = format!("Select {}", opt.name.to_lowercase()).into();
      let entity = cx.entity().downgrade();
      let is_empty = flat_options.is_empty();
      let button = Button::new(button_id)
        .label(current_label)
        .icon(IconName::ChevronDown)
        .xsmall()
        .ghost()
        .disabled(is_empty)
        .dropdown_menu_with_anchor(Corner::BottomLeft, move |menu, _, _| {
          let mut menu = menu.label(opt_label.clone());
          for (value_id, name, description) in flat_options.iter() {
            let value_id = value_id.clone();
            let name: SharedString = name.clone().into();
            let description: Option<SharedString> = description.clone().map(Into::into);
            let entity = entity.clone();
            let config_id = config_id.clone();
            let is_current = value_id == current_value;
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
          menu
        });
      out.push(button.into_any_element());
    }
    out
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use agent_client_protocol::schema::{ToolCallLocation, ToolCallUpdateFields};

  fn call(id: &str, title: &str, kind: ToolKind) -> ToolCall {
    let arc: std::sync::Arc<str> = std::sync::Arc::from(id);
    let mut c = ToolCall::new(ToolCallId::new(arc), title.to_string());
    c.kind = kind;
    c
  }

  fn test_cwd() -> &'static std::path::Path {
    std::path::Path::new("/")
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
  fn word_diff_highlights_only_changed_words() {
    let (old, new) = word_diff_spans(
      "export const VERSION = '3.0.0';",
      "export const VERSION = '2.0.0';",
    );
    let old_diff: String = old
      .iter()
      .filter(|s| s.kind == InlineSpanKind::Diff)
      .map(|s| s.text.clone())
      .collect();
    let new_diff: String = new
      .iter()
      .filter(|s| s.kind == InlineSpanKind::Diff)
      .map(|s| s.text.clone())
      .collect();
    assert_eq!(old_diff, "3");
    assert_eq!(new_diff, "2");
  }

  #[test]
  fn word_diff_splits_camel_case_identifier() {
    let (old, new) = word_diff_spans("foo getLastValue bar", "foo getFirstValue bar");
    let old_diff: String = old
      .iter()
      .filter(|s| s.kind == InlineSpanKind::Diff)
      .map(|s| s.text.clone())
      .collect();
    let new_diff: String = new
      .iter()
      .filter(|s| s.kind == InlineSpanKind::Diff)
      .map(|s| s.text.clone())
      .collect();
    assert_eq!(old_diff, "Last");
    assert_eq!(new_diff, "First");
  }

  #[test]
  fn word_diff_falls_back_to_whitespace_when_only_indent_changes() {
    let (old, new) = word_diff_spans("  let x = 1;", "    let x = 1;");
    let has_diff = old
      .iter()
      .chain(new.iter())
      .any(|s| s.kind == InlineSpanKind::Diff);
    assert!(
      has_diff,
      "indent-only change should still produce highlight"
    );
  }

  #[test]
  fn word_diff_skipped_above_byte_cap() {
    let old = "x".repeat(WORD_DIFF_MAX_COMBINED_BYTES);
    let new = "y".repeat(WORD_DIFF_MAX_COMBINED_BYTES);
    let (old_spans, new_spans) = word_diff_spans(&old, &new);
    assert!(old_spans.is_empty());
    assert!(new_spans.is_empty());
  }

  #[test]
  fn word_diff_empty_for_identical() {
    let (old, new) = word_diff_spans("same line", "same line");
    assert!(old.iter().all(|s| s.kind == InlineSpanKind::Same));
    assert!(new.iter().all(|s| s.kind == InlineSpanKind::Same));
  }

  #[test]
  fn build_diff_lines_pairs_and_carries_spans() {
    let lines = build_diff_lines(
      "export const VERSION = '3.0.0';\n",
      "export const VERSION = '2.0.0';\n",
    );
    let removed = lines
      .iter()
      .filter(|l| l.kind == DiffLineKind::Removed)
      .count();
    let added = lines
      .iter()
      .filter(|l| l.kind == DiffLineKind::Added)
      .count();
    assert_eq!(removed, 1);
    assert_eq!(added, 1);
    for l in &lines {
      assert!(!l.spans.is_empty(), "paired lines should have spans");
    }
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
  fn diff_line_counts_full_replacement() {
    let (added, removed) = diff_line_counts(Some("a\nb\nc\n"), "x\ny\nz\n");
    assert_eq!(added, 3);
    assert_eq!(removed, 3);
  }

  #[test]
  fn diff_line_counts_pure_addition() {
    let (added, removed) = diff_line_counts(None, "new line\n");
    assert_eq!(added, 1);
    assert_eq!(removed, 0);
  }

  #[test]
  fn diff_line_counts_identical_is_zero() {
    let (added, removed) = diff_line_counts(Some("same\nlines\n"), "same\nlines\n");
    assert_eq!(added, 0);
    assert_eq!(removed, 0);
  }

  #[test]
  fn extract_diffs_collects_per_file() {
    use agent_client_protocol::schema::Diff;
    let content = vec![
      ToolCallContent::Diff(Diff::new("foo.rs", "new\n")),
      ToolCallContent::Diff(Diff::new("bar.rs", "after\n").old_text(Some("before\n".to_string()))),
    ];
    let diffs = extract_diffs(&content, test_cwd());
    assert_eq!(diffs.len(), 2);
    assert_eq!(diffs[0].path, "foo.rs");
    assert_eq!(diffs[0].added, 1);
    assert_eq!(diffs[1].added, 1);
    assert_eq!(diffs[1].removed, 1);
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
    let dir = std::env::temp_dir().join(format!(
      "reviu-agent-prune-{}",
      std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
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
    let dir = std::env::temp_dir().join(format!(
      "reviu-agent-prune-keep-{}",
      std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("fresh.json"), "[]").unwrap();
    let pruned = AgentChatPanel::prune_old_state(&dir, std::time::Duration::from_secs(60));
    assert_eq!(pruned, 0);
    assert!(dir.join("fresh.json").exists());
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn list_conversations_sorted_by_updated_at_desc() {
    let dir = std::env::temp_dir().join(format!(
      "reviu-agent-sort-{}",
      std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
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
  fn tool_kind_labels_cover_main_kinds() {
    assert_eq!(tool_kind_label(&ToolKind::Read), "Read");
    assert_eq!(tool_kind_label(&ToolKind::Edit), "Edit");
    assert_eq!(tool_kind_label(&ToolKind::Execute), "Run");
  }
}
