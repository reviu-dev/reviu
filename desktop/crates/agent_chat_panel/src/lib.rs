use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use agent_acp::{
  AgentEvent, AgentSession, AuthMethodInfo, BackendAvailability, BackendConfig,
  PermissionOptionKind, PermissionPrompt,
};
use agent_client_protocol::schema::{
  ContentBlock, ToolCall, ToolCallId, ToolCallStatus, ToolCallUpdate, ToolKind,
};
use futures::future::BoxFuture;
use gfm_markdown_viewer::{MarkdownRenderOptions, render_markdown};
use gpui::{
  Context, Entity, FocusHandle, Focusable, IntoElement, ParentElement, Render, ScrollHandle,
  SharedString, Styled, Task, Window, div, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Disableable as _, Sizable as _,
  button::{Button, ButtonVariants as _},
  h_flex,
  input::InputEvent,
  scroll::ScrollableElement as _,
  v_flex,
};
use ui::{Input, InputState};

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

enum Status {
  Connecting,
  Ready,
  Error(String),
  MissingBinary { command: String, hint: String },
}

pub struct AgentChatPanel {
  backend: BackendConfig,
  status: Status,
  items: Vec<ChatItem>,
  tool_index: HashMap<ToolCallId, usize>,
  pending_agent: String,
  session: Option<Arc<AgentSession>>,
  input: Entity<InputState>,
  focus_handle: FocusHandle,
  in_flight: bool,
  scroll_handle: ScrollHandle,
  usage: Option<(u64, u64)>,
  agent_version: Option<String>,
  auth_methods: Vec<AuthMethodInfo>,
  state_path: Option<PathBuf>,
  _connect_task: Option<Task<()>>,
  _events_task: Option<Task<()>>,
  _permission_task: Option<Task<()>>,
  _input_sub: Option<gpui::Subscription>,
}

impl AgentChatPanel {
  pub fn new(
    backend: BackendConfig,
    cwd: PathBuf,
    state_path: Option<PathBuf>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    let input = cx.new(|cx| {
      InputState::new(window, cx)
        .auto_grow(2, 8)
        .placeholder("Message Claude... (Enter to send, Shift+Enter for newline)")
    });
    let focus_handle = cx.focus_handle();

    let input_sub = cx.subscribe_in(
      &input,
      window,
      |this, _state, event: &InputEvent, window, cx| {
        if let InputEvent::PressEnter { .. } = event {
          this.submit(window, cx);
        }
      },
    );

    let (loaded_items, loaded_index) = state_path
      .as_deref()
      .and_then(load_state_from_path)
      .unwrap_or_default();

    let mut panel = Self {
      backend: backend.clone(),
      status: Status::Connecting,
      items: loaded_items,
      tool_index: loaded_index,
      pending_agent: String::new(),
      session: None,
      input,
      focus_handle,
      in_flight: false,
      scroll_handle: ScrollHandle::new(),
      usage: None,
      agent_version: None,
      auth_methods: Vec::new(),
      state_path,
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

    let task = cx.spawn(async move |this, cx| {
      let result = AgentSession::spawn(backend, cwd, spawner).await;
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

  fn answer_permission(&mut self, prompt_id: u64, option_id: Option<String>, cx: &mut Context<Self>) {
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
        if let ContentBlock::Text(t) = chunk.content {
          self.pending_agent.push_str(&t.text);
        }
      }
      AgentEvent::AgentThoughtChunk(_) => {}
      AgentEvent::ToolCall(call) => {
        self.upsert_tool_call(call);
      }
      AgentEvent::ToolCallUpdate(update) => {
        self.apply_tool_call_update(update);
      }
      AgentEvent::UsageUpdate(usage) => {
        self.usage = Some((usage.used, usage.size));
      }
      _ => {}
    }
  }

  fn upsert_tool_call(&mut self, call: ToolCall) {
    upsert_tool_call_pure(&mut self.items, &mut self.tool_index, call);
  }

  fn apply_tool_call_update(&mut self, update: ToolCallUpdate) {
    apply_tool_call_update_pure(&mut self.items, &self.tool_index, update);
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

  /// Send a prompt programmatically (e.g. from another panel). Pushes the text
  /// into chat history as if the user typed it, then waits for the agent reply.
  /// Returns false if the agent isn't ready or another turn is in flight.
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
        if let Err(e) = result {
          panel.items.push(ChatItem::Message(ChatMessage {
            role: ChatRole::System,
            text: format!("[error] {e}"),
          }));
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
    matches!(
      self.status,
      Status::Error(_) | Status::MissingBinary { .. }
    )
  }

  fn clear_chat(&mut self, cx: &mut Context<Self>) {
    self.items.clear();
    self.tool_index.clear();
    self.pending_agent.clear();
    self.persist_state();
    cx.notify();
  }

  fn persist_state(&self) {
    let Some(path) = self.state_path.as_ref() else {
      return;
    };
    let persisted: Vec<PersistedChatItem> = self
      .items
      .iter()
      .filter_map(|item| match item {
        ChatItem::Message(m) => Some(PersistedChatItem::Message(m.clone())),
        ChatItem::Tool(t) => Some(PersistedChatItem::Tool(t.clone())),
        ChatItem::Permission(_) => None,
      })
      .collect();
    if let Some(parent) = path.parent() {
      let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string(&persisted) {
      let _ = std::fs::write(path, json);
    }
  }

  /// Compute the on-disk state path for a given repo. Hash the canonical
  /// repo path so file names are stable but bounded in length.
  pub fn state_path_for_repo(state_dir: &std::path::Path, repo: &std::path::Path) -> PathBuf {
    let canonical = repo.canonicalize().unwrap_or_else(|_| repo.to_path_buf());
    let hash = blake3::hash(canonical.to_string_lossy().as_bytes());
    let hex = hash.to_hex();
    state_dir.join(format!("{}.json", &hex.as_str()[..16]))
  }
}

fn load_state_from_path(path: &std::path::Path) -> Option<(Vec<ChatItem>, HashMap<ToolCallId, usize>)> {
  let raw = std::fs::read_to_string(path).ok()?;
  let parsed: Vec<PersistedChatItem> = serde_json::from_str(&raw).ok()?;
  let mut items = Vec::with_capacity(parsed.len());
  let mut index = HashMap::new();
  for item in parsed {
    match item {
      PersistedChatItem::Message(m) => items.push(ChatItem::Message(m)),
      PersistedChatItem::Tool(t) => {
        index.insert(t.id.clone(), items.len());
        items.push(ChatItem::Tool(t));
      }
    }
  }
  Some((items, index))
}

fn upsert_tool_call_pure(
  items: &mut Vec<ChatItem>,
  index: &mut HashMap<ToolCallId, usize>,
  call: ToolCall,
) {
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

fn tool_status_glyph(status: &ToolCallStatus) -> &'static str {
  match status {
    ToolCallStatus::Pending => "○",
    ToolCallStatus::InProgress => "◐",
    ToolCallStatus::Completed => "●",
    ToolCallStatus::Failed => "✗",
    _ => "·",
  }
}

fn render_tool_call(t: &ToolCallView, theme: &gpui_component::Theme) -> gpui::AnyElement {
  let status_color = match t.status {
    ToolCallStatus::Completed => theme.success,
    ToolCallStatus::Failed => theme.danger,
    ToolCallStatus::InProgress => theme.warning,
    _ => theme.muted_foreground,
  };
  let locations: String = t
    .locations
    .iter()
    .map(|(p, line)| match line {
      Some(l) => format!("{}:{l}", p.display()),
      None => p.display().to_string(),
    })
    .collect::<Vec<_>>()
    .join(", ");

  v_flex()
    .gap_1()
    .p_2()
    .border_1()
    .border_color(theme.border)
    .rounded(px(4.))
    .child(
      h_flex()
        .gap_2()
        .items_center()
        .child(
          div()
            .text_xs()
            .text_color(status_color)
            .child(tool_status_glyph(&t.status).to_string()),
        )
        .child(
          div()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(tool_kind_label(&t.kind).to_string()),
        )
        .child(
          div()
            .text_sm()
            .text_color(theme.foreground)
            .child(t.title.clone()),
        ),
    )
    .when(!locations.is_empty(), |this| {
      this.child(
        div()
          .text_xs()
          .text_color(theme.muted_foreground)
          .child(locations),
      )
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
  fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

impl Render for AgentChatPanel {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let theme = &theme;
    let md_options = MarkdownRenderOptions::default();

    let header_text: SharedString = match &self.status {
      Status::Connecting => format!("Connecting to {}...", self.backend.label).into(),
      Status::Ready => match &self.agent_version {
        Some(v) => format!("{} · v{v}", self.backend.label).into(),
        None => self.backend.label.into(),
      },
      Status::Error(e) => format!("Error: {e}").into(),
      Status::MissingBinary { command, .. } => format!("`{command}` not found").into(),
    };

    let usage_text: Option<SharedString> = self.usage.map(|(used, size)| {
      let used_k = used as f64 / 1000.0;
      let size_k = size as f64 / 1000.0;
      format!("{used_k:.1}k / {size_k:.0}k").into()
    });

    let mut messages = v_flex()
      .id("agent-chat-messages")
      .flex_1()
      .min_h_0()
      .gap_3()
      .p_3()
      .track_scroll(&self.scroll_handle)
      .overflow_y_scrollbar();

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

    if matches!(self.status, Status::Ready) && !self.auth_methods.is_empty() {
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
              .child(if cfg!(target_os = "macos") {
                Button::new(open_id)
                  .label("Open in Terminal")
                  .small()
                  .primary()
                  .on_click(cx.listener(move |_, _, _, cx| {
                    if !launch_cmd.try_launch_terminal(&exec_owned) {
                      cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                        copy_value.clone(),
                      ));
                    }
                  }))
              } else {
                Button::new(open_id)
                  .label("Open in Terminal")
                  .small()
                  .primary()
                  .disabled(true)
                  .tooltip("Not supported on this platform yet")
              })
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

    for item in &self.items {
      match item {
        ChatItem::Message(m) => {
          let (label, color) = match m.role {
            ChatRole::User => ("You", theme.primary),
            ChatRole::Agent => (self.backend.label, theme.foreground),
            ChatRole::System => ("System", theme.muted_foreground),
          };
          let body: gpui::AnyElement = match m.role {
            ChatRole::Agent => render_markdown(&m.text, &md_options, cx),
            _ => div()
              .text_sm()
              .text_color(theme.foreground)
              .child(m.text.clone())
              .into_any_element(),
          };
          messages = messages.child(
            v_flex()
              .gap_1()
              .child(div().text_xs().text_color(color).child(label.to_string()))
              .child(body),
          );
        }
        ChatItem::Tool(t) => {
          messages = messages.child(render_tool_call(t, theme));
        }
        ChatItem::Permission(p) => {
          messages = messages.child(render_permission(p, theme, cx));
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
    }

    v_flex()
      .size_full()
      .bg(theme.background)
      .child(
        h_flex()
          .h(px(36.))
          .px_3()
          .items_center()
          .justify_between()
          .border_b_1()
          .border_color(theme.border)
          .child(div().text_sm().text_color(theme.foreground).child(header_text))
          .child(
            h_flex()
              .gap_3()
              .items_center()
              .when_some(usage_text, |this, t| {
                this.child(
                  div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(t),
                )
              })
              .child(
                Button::new("agent-chat-clear")
                  .label("Clear")
                  .small()
                  .ghost()
                  .disabled(self.items.is_empty() && self.pending_agent.is_empty())
                  .tooltip("Clear chat history for this repo")
                  .on_click(cx.listener(|panel, _, _, cx| panel.clear_chat(cx))),
              ),
          ),
      )
      .child(messages)
      .child(
        h_flex()
          .p_2()
          .gap_2()
          .border_t_1()
          .border_color(theme.border)
          .child(Input::new(&self.input).w_full())
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
      )
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

  #[test]
  fn upsert_inserts_new_tool_call() {
    let mut items: Vec<ChatItem> = Vec::new();
    let mut index = HashMap::new();
    upsert_tool_call_pure(&mut items, &mut index, call("a", "Read foo", ToolKind::Read));
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
    upsert_tool_call_pure(&mut items, &mut index, call("a", "Old", ToolKind::Read));
    upsert_tool_call_pure(&mut items, &mut index, call("a", "New", ToolKind::Edit));
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
    upsert_tool_call_pure(&mut items, &mut index, call("a", "A", ToolKind::Read));
    upsert_tool_call_pure(&mut items, &mut index, call("b", "B", ToolKind::Edit));
    assert_eq!(items.len(), 2);
    assert_eq!(index.get(&ToolCallId::from("a")), Some(&0));
    assert_eq!(index.get(&ToolCallId::from("b")), Some(&1));
  }

  #[test]
  fn apply_update_merges_partial_fields() {
    let mut items: Vec<ChatItem> = Vec::new();
    let mut index = HashMap::new();
    upsert_tool_call_pure(&mut items, &mut index, call("a", "Initial", ToolKind::Read));

    let mut fields = ToolCallUpdateFields::default();
    fields.status = Some(ToolCallStatus::InProgress);
    let update = ToolCallUpdate::new(ToolCallId::from("a"), fields);
    apply_tool_call_update_pure(&mut items, &index, update);

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
    apply_tool_call_update_pure(&mut items, &index, update);
    assert!(items.is_empty());
  }

  #[test]
  fn apply_update_replaces_locations() {
    let mut items: Vec<ChatItem> = Vec::new();
    let mut index = HashMap::new();
    upsert_tool_call_pure(&mut items, &mut index, call("a", "Edit", ToolKind::Edit));

    let mut fields = ToolCallUpdateFields::default();
    fields.locations = Some(vec![ToolCallLocation::new("foo.rs").line(42_u32)]);
    let update = ToolCallUpdate::new(ToolCallId::from("a"), fields);
    apply_tool_call_update_pure(&mut items, &index, update);

    let ChatItem::Tool(view) = &items[0] else {
      panic!()
    };
    assert_eq!(view.locations.len(), 1);
    assert_eq!(view.locations[0].1, Some(42));
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
  fn tool_kind_labels_cover_main_kinds() {
    assert_eq!(tool_kind_label(&ToolKind::Read), "Read");
    assert_eq!(tool_kind_label(&ToolKind::Edit), "Edit");
    assert_eq!(tool_kind_label(&ToolKind::Execute), "Run");
  }
}
