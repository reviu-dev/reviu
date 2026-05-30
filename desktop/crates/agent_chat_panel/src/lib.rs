mod diff;
mod persistence;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::diff::{
  DiffLineKind, DiffSummary, InlineSpan, InlineSpanKind, MAX_DIFF_LINES_COLLAPSED,
  MAX_TOOL_OUTPUT_LINES_COLLAPSED, extract_diffs, extract_outputs,
};
pub use crate::persistence::{ConversationMeta, state_dir_for_repo};
use crate::persistence::{new_conversation_meta, now_secs, truncate_title};
use agent_acp::{
  AgentEvent, AgentSession, AuthMethodInfo, BackendAvailability, BackendConfig, BackendKind,
  PermissionOptionKind, PermissionPrompt,
};
use agent_client_protocol::schema::{
  ContentBlock, Plan, PlanEntryPriority, PlanEntryStatus, SessionInfoUpdate, ToolCall, ToolCallId,
  ToolCallStatus, ToolCallUpdate, ToolKind,
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
  Context, Entity, FocusHandle, Focusable, Font, FontStyle, FontWeight, Hsla, IntoElement,
  ParentElement, Render, SharedString, Styled, StyledText, Task, TextRun, Window, div, prelude::*,
  px,
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
use syntax::{HighlightSpan, SyntaxHighlighter, SyntaxTheme, highlights_to_text_runs, languages};
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
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
enum PersistedChatItem {
  Message(ChatMessage),
  Tool(ToolCallView),
  Plan(PlanView),
  Thought(ThoughtView),
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PersistedConversation {
  meta: ConversationMeta,
  items: Vec<PersistedChatItem>,
}

#[derive(Clone, Copy, Debug)]
enum ExtraBeforeKind {
  MissingBinary,
  Error,
  Auth,
}

#[derive(Clone, Copy, Debug)]
enum ExtraAfterKind {
  Pending,
  Spinner,
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
  pending_thought: String,
  session: Option<Arc<AgentSession>>,
  input: Entity<InputState>,
  in_flight: bool,
  messages_list: gpui::ListState,
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
      pending_thought: String::new(),
      session: None,
      input,
      in_flight: false,
      messages_list: gpui::ListState::new(0, gpui::ListAlignment::Top, px(300.)),
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
          panel.sync_list_count();
          panel.messages_list.scroll_to_end();
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
      self.in_flight = false;
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
    if matches!(self.status, Status::Error(_)) {
      n += 1;
    }
    if matches!(self.status, Status::Ready) && self.auth_required && !self.auth_methods.is_empty() {
      n += 1;
    }
    n
  }

  fn extras_after_count(&self) -> usize {
    if !self.pending_agent.is_empty() {
      1
    } else if self.in_flight {
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
    if matches!(self.status, Status::Error(_)) {
      v.push(ExtraBeforeKind::Error);
    }
    if matches!(self.status, Status::Ready) && self.auth_required && !self.auth_methods.is_empty() {
      v.push(ExtraBeforeKind::Auth);
    }
    v
  }

  fn extras_after_kind(&self) -> Option<ExtraAfterKind> {
    if !self.pending_agent.is_empty() {
      Some(ExtraAfterKind::Pending)
    } else if self.in_flight {
      Some(ExtraAfterKind::Spinner)
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
        let md_options =
          MarkdownRenderOptions::with_on_link(Arc::new(|_url, _window, _cx| LinkAction::Open))
            .with_syntax_cache(self.syntax_cache.clone());
        self.render_item_at(item_ix, theme, &md_options, cx)
      } else if let Some(kind) = self.extras_after_kind() {
        self.render_extra_after(kind, theme, cx)
      } else {
        div().into_any_element()
      }
    };
    if list_ix == 0 {
      div().pt_3().child(element).into_any_element()
    } else {
      element
    }
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
      ExtraBeforeKind::Error => {
        let Status::Error(e) = &self.status else {
          return div().into_any_element();
        };
        div()
          .px_3()
          .text_sm()
          .text_color(theme.danger)
          .child(format!("Failed to start agent: {e}"))
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

  fn render_extra_after(
    &mut self,
    kind: ExtraAfterKind,
    theme: &gpui_component::Theme,
    cx: &mut Context<Self>,
  ) -> gpui::AnyElement {
    match kind {
      ExtraAfterKind::Pending => {
        let md_options =
          MarkdownRenderOptions::with_on_link(Arc::new(|_url, _window, _cx| LinkAction::Open))
            .with_syntax_cache(self.syntax_cache.clone());
        v_flex()
          .px_3()
          .pb_3()
          .gap_1()
          .child(
            div()
              .text_xs()
              .text_color(theme.muted_foreground)
              .child(format!("{} (typing...)", self.backend.label)),
          )
          .child(render_markdown(&self.pending_agent, &md_options, cx))
          .into_any_element()
      }
      ExtraAfterKind::Spinner => h_flex()
        .px_3()
        .pb_3()
        .gap_2()
        .items_center()
        .child(Spinner::new().xsmall().color(theme.muted_foreground))
        .child(
          div()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(format!("{} is thinking...", self.backend.label)),
        )
        .into_any_element(),
    }
  }

  fn render_item_at(
    &mut self,
    idx: usize,
    theme: &gpui_component::Theme,
    md_options: &MarkdownRenderOptions,
    cx: &mut Context<Self>,
  ) -> gpui::AnyElement {
    let has_trailer = self.extras_after_kind().is_some();
    let total = self.items.len();
    let next_is_user = self
      .items
      .get(idx + 1)
      .map(|i| {
        matches!(
          i,
          ChatItem::Message(ChatMessage {
            role: ChatRole::User,
            ..
          })
        )
      })
      .unwrap_or(false);
    let is_end_of_group = if idx + 1 == total {
      !has_trailer
    } else {
      next_is_user
    };
    let is_last_row = is_end_of_group;

    let item = self.items[idx].clone();
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
          .child(m.text.clone())
          .into_any_element(),
        ChatRole::Agent => {
          timeline_row(render_markdown(&m.text, md_options, cx), theme, is_last_row)
        }
        ChatRole::System => timeline_row(
          div()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(m.text.clone())
            .into_any_element(),
          theme,
          is_last_row,
        ),
      },
      ChatItem::Tool(t) => {
        let bullet = match t.status {
          ToolCallStatus::Completed => theme.status_green(),
          ToolCallStatus::Failed => theme.danger,
          ToolCallStatus::InProgress => theme.warning,
          _ => theme.muted_foreground,
        };
        timeline_row_with_color(render_tool_call(t, theme, cx), theme, bullet, is_last_row)
      }
      ChatItem::Permission(p) => timeline_row(render_permission(p, theme, cx), theme, is_last_row),
      ChatItem::Plan(p) => {
        timeline_row_with_color(render_plan(p, theme), theme, theme.primary, is_last_row)
      }
      ChatItem::Thought(t) => timeline_row(render_thought(idx, t, theme, cx), theme, is_last_row),
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
          panel.messages_list.scroll_to_end();
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
        self.config_options = u.config_options;
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
    self.pending_thought.clear();
    self.in_flight = true;
    self.persist_state();
    self.sync_list_count();
    self.messages_list.scroll_to_end();
    cx.notify();

    cx.spawn(async move |this, cx| {
      let result = session.send_prompt(text).await;
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
              panel.items.push(ChatItem::Message(ChatMessage {
                role: ChatRole::System,
                text: format!("[error] {e}"),
              }));
            }
          }
        }
        panel.in_flight = false;
        panel.persist_state();
        panel.sync_list_count();
        panel.messages_list.scroll_to_end();
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
    self.pending_thought.clear();
    self.in_flight = false;
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
      self.in_flight = false;
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
        ChatItem::Plan(p) => Some(PersistedChatItem::Plan(p.clone())),
        ChatItem::Thought(t) => Some(PersistedChatItem::Thought(t.clone())),
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
    view.outputs = extract_outputs(&content);
  }
  populate_syntax_spans(view);
}

fn short_model_label(name: &str, description: Option<&str>) -> String {
  // Claude descriptions follow "<id> [with X] · <blurb>" so the model id can be
  // pulled out. Other backends (Codex) use a free-form blurb with no separator;
  // fall back to the dropdown name there.
  let Some(desc) = description.filter(|d| d.contains(" · ")) else {
    return name.to_string();
  };
  let before_separator = desc.split(" · ").next().unwrap_or(desc);
  let before_qualifier = before_separator
    .split_once(" with ")
    .map(|(head, _)| head)
    .unwrap_or(before_separator);
  let trimmed = before_qualifier.trim();
  if trimmed.is_empty() {
    name.to_string()
  } else {
    trimmed.to_string()
  }
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
  let preview: SharedString = thought
    .text
    .lines()
    .next()
    .unwrap_or("")
    .chars()
    .take(80)
    .collect::<String>()
    .into();
  let toggle_id = SharedString::from(format!("agent-chat-thought-toggle-{idx}"));
  let body_text = thought.text.clone();
  v_flex()
    .gap_1()
    .child(
      h_flex()
        .id(SharedString::from(format!("agent-chat-thought-{idx}")))
        .gap_2()
        .items_center()
        .cursor_pointer()
        .on_click(cx.listener(move |panel, _, _, cx| panel.toggle_thought_collapsed(idx, cx)))
        .child(
          gpui_component::Icon::new(icon)
            .small()
            .text_color(theme.muted_foreground),
        )
        .child(
          div()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(if collapsed {
              SharedString::from(format!("Thought: {preview}"))
            } else {
              SharedString::from("Thought")
            }),
        )
        .child(div().flex_1())
        .child(Button::new(toggle_id).xsmall().ghost().label(if collapsed {
          "Expand"
        } else {
          "Collapse"
        }))
        .into_any_element(),
    )
    .when(!collapsed, |this| {
      this.child(
        div()
          .text_xs()
          .text_color(theme.muted_foreground)
          .child(body_text),
      )
    })
    .into_any_element()
}

fn render_tool_call(
  t: &ToolCallView,
  theme: &gpui_component::Theme,
  cx: &mut Context<AgentChatPanel>,
) -> gpui::AnyElement {
  let title_color = match t.status {
    ToolCallStatus::Failed => theme.danger,
    ToolCallStatus::InProgress => theme.warning,
    _ => theme.foreground,
  };
  let detail = tool_detail_label(t);
  let tool_id = t.id.clone();

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
        let mut content_div = div()
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
          .whitespace_normal();
        if runs.is_empty() {
          content_div = content_div.child(body_text);
        } else {
          content_div =
            content_div.child(StyledText::new(SharedString::from(body_text)).with_runs(runs));
        }
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
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let theme = &theme;

    let _ = SharedString::from("");

    let usage_text: Option<SharedString> = self.usage.map(|(used, size)| {
      let used_k = used as f64 / 1000.0;
      let size_k = size as f64 / 1000.0;
      format!("{used_k:.1}k / {size_k:.0}k").into()
    });

    let show_empty_state =
      self.total_list_items() == 0 && matches!(self.status, Status::Ready | Status::Connecting);
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
              .vertical_scrollbar(&self.messages_list),
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
      .map(|m| short_model_label(&m.name, m.description.as_deref()).into())
      .unwrap_or_else(|| "Model".into());
    let entity = cx.entity().downgrade();
    Button::new("agent-chat-model")
      .label(current_label)
      .icon(IconName::ChevronDown)
      .xsmall()
      .ghost()
      .disabled(models.is_empty())
      .dropdown_menu_with_anchor(Corner::BottomLeft, move |menu, _, _| {
        let mut menu = menu
          .label("Select a model")
          .max_h(px(360.))
          .scrollable(true);
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
    assert_eq!(
      short_model_label(
        "gpt-5.2-codex (high)",
        Some("Frontier agentic coding model. Greater reasoning depth for complex problems"),
      ),
      "gpt-5.2-codex (high)"
    );
    assert_eq!(
      short_model_label(
        "GPT-5.5 (high)",
        Some(
          "Frontier model for complex coding, research, and real-world work. Greater reasoning depth for complex problems"
        ),
      ),
      "GPT-5.5 (high)"
    );
  }

  #[test]
  fn tool_kind_labels_cover_main_kinds() {
    assert_eq!(tool_kind_label(&ToolKind::Read), "Read");
    assert_eq!(tool_kind_label(&ToolKind::Edit), "Edit");
    assert_eq!(tool_kind_label(&ToolKind::Execute), "Run");
  }
}
