//! The event machine: agent stream intake, buffers, prompt dispatch.

use super::*;

/// Commit floor during streaming: one update + notify per window (~8
/// redraws/s) whatever the provider's chunk rate. The first chunk after a
/// quiet spell still paints immediately.
pub(crate) const STREAM_COMMIT_MS: u64 = 120;

/// Whether anything answered the last prompt: any agent output between the
/// tail and the most recent user message. No user message at all counts as
/// answered (nothing was asked).
fn turn_produced_reply(items: &[ChatItem]) -> bool {
  for item in items.iter().rev() {
    match item {
      ChatItem::Message(message) => match message.role {
        ChatRole::User | ChatRole::ReviewExport => return false,
        ChatRole::Agent => return true,
        ChatRole::System => {}
      },
      ChatItem::Tool(_) | ChatItem::Thought(_) | ChatItem::Plan(_) | ChatItem::Permission(_) => {
        return true;
      }
      ChatItem::Checkpoint(_) | ChatItem::TurnSummary(_) => {}
    }
  }
  true
}

/// Folds an adjacent same-message text chunk into `prev` so a burst costs
/// one markdown push_str instead of one per chunk.
fn merge_into_last(prev: Option<&mut AgentEvent>, next: &AgentEvent) -> bool {
  let (prev, next) = match (prev, next) {
    (Some(AgentEvent::AgentMessageChunk(p)), AgentEvent::AgentMessageChunk(n)) => (p, n),
    (Some(AgentEvent::AgentThoughtChunk(p)), AgentEvent::AgentThoughtChunk(n)) => (p, n),
    _ => return false,
  };
  if prev.message_id != next.message_id {
    return false;
  }
  match (&mut prev.content, &next.content) {
    (ContentBlock::Text(prev_text), ContentBlock::Text(next_text)) => {
      prev_text.text.push_str(&next_text.text);
      true
    }
    _ => false,
  }
}

pub(crate) fn fold_adjacent_chunks(events: Vec<AgentEvent>) -> Vec<AgentEvent> {
  let mut out: Vec<AgentEvent> = Vec::with_capacity(events.len());
  for event in events {
    if !merge_into_last(out.last_mut(), &event) {
      out.push(event);
    }
  }
  out
}

impl AgentChatPanel {
  pub(crate) fn start_event_forwarder(
    &mut self,
    rx: async_channel::Receiver<AgentEvent>,
    cx: &mut Context<Self>,
  ) {
    // Boundaries (turn end, new prompt) drain this clone so no chunk can
    // land after the event that should follow it.
    self.events_rx = Some(rx.clone());
    let task = cx.spawn(async move |this, cx| {
      let window = std::time::Duration::from_millis(STREAM_COMMIT_MS);
      let mut last_commit: Option<std::time::Instant> = None;
      loop {
        // The floor runs before recv so nothing is ever held across an
        // await: queued events stay drainable by a boundary.
        if let Some(last) = last_commit {
          let elapsed = last.elapsed();
          if elapsed < window {
            cx.background_executor().timer(window - elapsed).await;
          }
        }
        let Ok(first) = rx.recv().await else {
          break;
        };
        let mut events = vec![first];
        while let Ok(event) = rx.try_recv() {
          events.push(event);
        }
        let events = fold_adjacent_chunks(events);
        let alive = this.update(cx, |panel, cx| {
          for event in events {
            panel.on_event(event, cx);
          }
          panel.schedule_persist(cx);
          panel.sync_list_count();
          cx.notify();
        });
        if alive.is_err() {
          return;
        }
        last_commit = Some(std::time::Instant::now());
      }
      // Channel closed: the agent driver exited (child died or stopped).
      let _ = this.update(cx, |panel, cx| {
        panel.on_agent_disconnected(cx);
        panel.persist_state(cx);
      });
    });
    self._events_task = Some(task);
  }

  /// Applies every event still queued (or riding the commit floor) so a
  /// boundary never overtakes the chunks that streamed before it.
  pub(crate) fn drain_pending_events(&mut self, cx: &mut Context<Self>) {
    let Some(rx) = self.events_rx.clone() else {
      return;
    };
    while let Ok(event) = rx.try_recv() {
      self.on_event(event, cx);
    }
  }

  pub(crate) fn start_permission_forwarder(
    &mut self,
    rx: async_channel::Receiver<PermissionPrompt>,
    cx: &mut Context<Self>,
  ) {
    let task = cx.spawn(async move |this, cx| {
      while let Ok(prompt) = rx.recv().await {
        let _ = this.update(cx, |panel, cx| {
          let detail = permission_detail(&prompt.tool_call, &panel.cwd);
          let prompt_id = prompt.id;
          let auto_option = panel
            .auto_approve
            .then(|| auto_approve_option(&prompt.options))
            .flatten();
          let auto = auto_option.is_some();
          panel
            .items
            .push(ChatItem::Permission(Box::new(PermissionItem {
              prompt,
              detail,
              resolved: None,
              auto,
            })));
          panel.sync_list_count();
          match auto_option {
            Some(option_id) => panel.answer_permission(prompt_id, Some(option_id), cx),
            // A prompt without an allow option still needs a human.
            None => cx.emit(AgentChatPanelEvent::PermissionRequested),
          }
          cx.notify();
        });
      }
    });
    self._permission_task = Some(task);
  }

  /// Repaints the tool row embedding a terminal whenever its state changes.
  /// PTY floods (builds, test runs) ride the same commit floor as chunks.
  pub(crate) fn start_terminal_forwarder(
    &mut self,
    rx: async_channel::Receiver<String>,
    cx: &mut Context<Self>,
  ) {
    let task = cx.spawn(async move |this, cx| {
      let window = std::time::Duration::from_millis(STREAM_COMMIT_MS);
      let mut last_commit: Option<std::time::Instant> = None;
      while let Ok(first) = rx.recv().await {
        if let Some(last) = last_commit {
          let elapsed = last.elapsed();
          if elapsed < window {
            cx.background_executor().timer(window - elapsed).await;
          }
        }
        let mut ids = vec![first];
        while let Ok(id) = rx.try_recv() {
          if !ids.contains(&id) {
            ids.push(id);
          }
        }
        let alive = this.update(cx, |panel, cx| {
          for terminal_id in &ids {
            if let Some(item_idx) = panel.items.iter().position(|item| {
              matches!(item, ChatItem::Tool(t) if t.terminals.iter().any(|id| id == terminal_id))
            }) {
              let list_ix = panel.list_ix_for_item(item_idx);
              panel.mark_item_changed_at(list_ix);
            }
          }
          cx.notify();
        });
        if alive.is_err() {
          return;
        }
        last_commit = Some(std::time::Instant::now());
      }
    });
    self._terminal_task = Some(task);
  }

  pub(crate) fn on_event(&mut self, event: AgentEvent, cx: &mut Context<Self>) {
    match event {
      AgentEvent::AgentMessageChunk(chunk) => {
        // Prose after a thought closes it, so the two keep their order.
        self.flush_pending_thought();
        if let ContentBlock::Text(t) = chunk.content {
          self.pending_agent.push_str(&t.text);
          let state = self.pending_md_state.get_or_insert_with(|| {
            cx.new(|cx| gpui_component::text::TextViewState::markdown("", cx))
          });
          state.update(cx, |state, cx| state.push_str(&t.text, cx));
        }
        self.sync_list_count();
        self.mark_last_item_changed();
      }
      AgentEvent::AgentThoughtChunk(chunk) => {
        // A thought after prose closes the prose segment.
        self.flush_pending_agent();
        if let ContentBlock::Text(t) = chunk.content {
          self.pending_thought.push_str(&t.text);
        }
        self.sync_list_count();
        self.mark_last_item_changed();
      }
      AgentEvent::ToolCall(call) => {
        let new_id = call.tool_call_id.clone();
        let is_new = !self.tool_index.contains_key(&new_id);
        if is_new {
          // Narration streamed before this call keeps its place in the
          // timeline instead of collapsing at the end of the turn.
          self.flush_pending_thought();
          self.flush_pending_agent();
        }
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
      AgentEvent::AvailableCommandsUpdate(update) => {
        self.available_commands = update.available_commands;
      }
      _ => {}
    }
  }

  pub(crate) fn flush_pending_thought(&mut self) {
    if self.pending_thought.trim().is_empty() {
      self.pending_thought.clear();
      return;
    }
    let text = std::mem::take(&mut self.pending_thought);
    self.items.push(ChatItem::Thought(ThoughtView {
      text,
      collapsed: true,
    }));
  }

  pub(crate) fn flush_pending_agent(&mut self) {
    let markdown_state = self.pending_md_state.take();
    if self.pending_agent.trim().is_empty() {
      self.pending_agent.clear();
      return;
    }
    let raw_text = std::mem::take(&mut self.pending_agent);
    let sanitized_text = sanitize_agent_markdown(&raw_text);
    let text = SharedString::from(sanitized_text);
    let item_idx = self.items.len();
    let can_reuse_markdown_state = raw_text == text.as_str() && agent_message_needs_markdown(&text);
    self.items.push(ChatItem::Message(ChatMessage {
      role: ChatRole::Agent,
      text: text.clone(),
      images: 0,
      image_data: Vec::new(),
    }));
    if can_reuse_markdown_state && let Some(state) = markdown_state {
      self
        .settled_md_states
        .insert(item_idx, SettledMarkdownState { text, state });
    }
  }

  /// Close the streaming buffers at the end of a turn, thought first.
  pub(crate) fn flush_turn_buffers(&mut self) {
    self.flush_pending_thought();
    self.flush_pending_agent();
  }

  pub(crate) fn dispatch_prompt(&mut self, text: String, cx: &mut Context<Self>) -> bool {
    self.dispatch_prompt_with_role(text, ChatRole::User, cx)
  }

  pub(crate) fn dispatch_prompt_with_role(
    &mut self,
    text: String,
    role: ChatRole,
    cx: &mut Context<Self>,
  ) -> bool {
    if self.in_flight {
      return false;
    }
    if !self.turn_gate.can_start(&self.cwd, &self.current_conv.id) {
      self.items.push(ChatItem::Message(ChatMessage {
        role: ChatRole::System,
        text: "Another session is running. Wait for it to finish.".into(),
        images: 0,
        image_data: Vec::new(),
      }));
      self.sync_list_count();
      cx.notify();
      return false;
    }
    let Some(session) = self.session.clone() else {
      return false;
    };

    // Chunks that trailed in after the previous turn keep their place
    // ahead of the new prompt instead of being wiped.
    self.drain_pending_events(cx);
    self.flush_turn_buffers();
    let images = std::mem::take(&mut self.staged_images);
    self.items.push(ChatItem::Message(ChatMessage {
      role,
      text: text.clone().into(),
      images: images.len(),
      image_data: images.clone(),
    }));
    cx.emit(AgentChatPanelEvent::TurnStarted);
    self.start_turn(cx);
    self.persist_state(cx);
    self.sync_list_count();
    self.arm_runway();
    cx.notify();

    let cwd = self.cwd.clone();
    let files = self.repo_files.clone();
    let selection = self.active_selection.take();
    cx.spawn(async move |this, cx| {
      let blocks = build_prompt_blocks(text, files, selection, images, cwd).await;
      let result = session.send_prompt_blocks(blocks).await;
      let _ = this.update(cx, |panel, cx| panel.complete_prompt(result, cx));
    })
    .detach();

    true
  }

  /// Injects a prompt into the running turn (Cmd+Enter). Falls back to a
  /// fresh dispatch when no turn is in flight, and refuses outright when the
  /// agent does not advertise the steering extension.
  pub fn steer_prompt(&mut self, text: String, cx: &mut Context<Self>) -> bool {
    if !self.in_flight {
      return self.dispatch_prompt(text, cx);
    }
    if !self.supports_steering {
      return false;
    }
    let Some(session) = self.session.clone() else {
      return false;
    };
    // The steered message joins the current turn: no checkpoint, no new
    // turn boundary; prose streamed so far settles ahead of it.
    self.drain_pending_events(cx);
    self.flush_turn_buffers();
    self.items.push(ChatItem::Message(ChatMessage {
      role: ChatRole::User,
      text: text.clone().into(),
      images: 0,
      image_data: Vec::new(),
    }));
    self.persist_state(cx);
    self.sync_list_count();
    self.arm_runway();
    cx.notify();

    let cwd = self.cwd.clone();
    let files = self.repo_files.clone();
    cx.spawn(async move |this, cx| {
      let blocks = build_prompt_blocks(text.clone(), files, None, Vec::new(), cwd).await;
      let result = session.steer_prompt_blocks(blocks).await;
      let _ = this.update(cx, |panel, cx| {
        match result {
          // Delivered into the running turn; its prompt still owns the end.
          Ok(agent_acp::SteerOutcome::Injected) => {}
          // The agent started a detached turn itself; its stream lands as
          // late chunks and there is no reply to wait for.
          Ok(agent_acp::SteerOutcome::StartedNewTurn) => {}
          // The turn was already over: run the message as a fresh turn.
          Ok(agent_acp::SteerOutcome::PromptRequired) => {
            panel.retract_steered_message(&text);
            if !panel.in_flight && !panel.dispatch_prompt(text.clone(), cx) {
              panel.queued_prompts.push(text.clone());
            }
          }
          // Steering unsupported or refused: back to the queue, not lost.
          Err(e) => {
            log::warn!("[agent] steer error: {e}");
            panel.retract_steered_message(&text);
            panel.queued_prompts.push(text.clone());
            panel.items.push(ChatItem::Message(ChatMessage {
              role: ChatRole::System,
              text: "Steer refused; message queued for the next turn.".into(),
              images: 0,
              image_data: Vec::new(),
            }));
          }
        }
        panel.persist_state(cx);
        panel.sync_list_count();
        cx.notify();
      });
    })
    .detach();
    true
  }

  /// Removes the optimistic bubble of a steer the session refused.
  fn retract_steered_message(&mut self, text: &str) {
    if let Some(idx) = self.items.iter().rposition(
      |item| matches!(item, ChatItem::Message(m) if m.role == ChatRole::User && m.text == text),
    ) {
      self.items.remove(idx);
      self.settled_md_states.clear();
      self.rebuild_tool_index();
    }
  }

  /// Settles a finished prompt: buffers flush, the summary card lands, the
  /// turn ends and the queue drains.
  pub(crate) fn complete_prompt(
    &mut self,
    result: anyhow::Result<agent_client_protocol::schema::StopReason>,
    cx: &mut Context<Self>,
  ) {
    {
      let panel = self;
      {
        panel.drain_pending_events(cx);
        panel.flush_turn_buffers();
        panel.append_turn_summary();
        // A turn that ran to the end: not stopped by the user, not failed.
        let mut completed = false;
        match result {
          Ok(stop_reason) => {
            panel.auth_required = false;
            if stop_reason == agent_client_protocol::schema::StopReason::Cancelled {
              // The user asked to stop, not to continue: the queue holds.
              let text = match panel.turn_started_at.map(|t| t.elapsed().as_secs()) {
                Some(secs) if secs > 0 => format!("Stopped after {secs}s"),
                _ => "Stopped".to_string(),
              };
              panel.items.push(ChatItem::Message(ChatMessage {
                role: ChatRole::System,
                text: text.into(),
                images: 0,
                image_data: Vec::new(),
              }));
            } else {
              completed = true;
              // The pi-style silent failure: the provider refused (credits,
              // limits) but the adapter ended the turn cleanly with nothing
              // in it. Whatever the adapter hides, an empty turn is loud.
              if !turn_produced_reply(&panel.items) {
                let message = "The agent ended the turn without a reply. \
Its provider may have refused it (credits, usage limit) without reporting an error.";
                panel.last_turn_failed = true;
                cx.emit(AgentChatPanelEvent::TurnFailed {
                  message: message.to_string(),
                });
                panel.items.push(ChatItem::Message(ChatMessage {
                  role: ChatRole::System,
                  text: format!("[error] {message}").into(),
                  images: 0,
                  image_data: Vec::new(),
                }));
              }
            }
          }
          Err(e) => {
            let msg = format!("{e}");
            if msg.contains("auth_required") {
              panel.auth_required = true;
              panel.items.push(ChatItem::Message(ChatMessage {
                role: ChatRole::System,
                text: "Authentication required. Sign in below and retry.".into(),
                images: 0,
                image_data: Vec::new(),
              }));
            } else {
              let raw = format!("{e}");
              // Full payload stays greppable in the app logs.
              log::warn!("[agent] prompt error: {raw}");
              let human = humanize_agent_error(&raw).unwrap_or_else(|| raw.clone());
              let short = agent_error_hint(&human)
                .or_else(|| agent_error_hint(&raw))
                .map(str::to_string)
                .unwrap_or_else(|| truncate_chars(&human, 200));
              let text = if short == human {
                format!("[error] {human}")
              } else {
                format!("[error] {short}\n{human}")
              };
              panel.last_turn_failed = true;
              cx.emit(AgentChatPanelEvent::TurnFailed {
                message: short.clone(),
              });
              // Some adapters (codex) stream the error as an agent bubble AND
              // fail the prompt with the same text: one display is enough.
              let already_shown = panel
                .items
                .iter()
                .rev()
                .find_map(|item| match item {
                  ChatItem::Message(m) if matches!(m.role, ChatRole::Agent) => {
                    Some(m.text.to_string())
                  }
                  ChatItem::Message(m)
                    if matches!(m.role, ChatRole::User | ChatRole::ReviewExport) =>
                  {
                    Some(String::new())
                  }
                  _ => None,
                })
                .is_some_and(|bubble| {
                  !bubble.is_empty()
                    && (bubble.contains(human.trim()) || human.contains(bubble.trim()))
                });
              if !already_shown {
                panel.items.push(ChatItem::Message(ChatMessage {
                  role: ChatRole::System,
                  text: text.into(),
                  images: 0,
                  image_data: Vec::new(),
                }));
              }
            }
          }
        }
        panel.end_turn();
        let apply_model_before_queue = panel.pending_model_selection.take().and_then(|pending| {
          let session = panel.session.clone()?;
          Some((session, pending))
        });
        let dispatch_queued_after_model =
          completed && !panel.queued_prompts.is_empty() && apply_model_before_queue.is_some();
        if let Some((session, pending)) = apply_model_before_queue {
          panel.spawn_set_model_request(
            session,
            pending.model_id,
            pending.previous_model_id,
            pending.generation,
            dispatch_queued_after_model,
            cx,
          );
        }
        // Unpinned groups fold when the turn settles; their rows must remeasure.
        let count = panel.messages_list.item_count();
        if count > 0 {
          panel.messages_list.remeasure_items(0..count);
        }
        if completed && !panel.queued_prompts.is_empty() && !dispatch_queued_after_model {
          panel.dispatch_next_queued_prompt(cx);
        }
        panel.persist_state(cx);
        panel.sync_list_count();
        cx.emit(AgentChatPanelEvent::TurnFinished { completed });
        panel.refresh_repo_files(cx);
        cx.notify();
      }
    }
  }

  /// Closes a turn that edited files with an aggregated summary card.
  pub(crate) fn append_turn_summary(&mut self) {
    let (files, checkpoint_ref) = turn_edit_stats(&self.items);
    if files.is_empty() {
      return;
    }
    self.items.push(ChatItem::TurnSummary(TurnSummaryView {
      files,
      checkpoint_ref,
      undone: false,
      duration_secs: self.turn_started_at.map(|t| t.elapsed().as_secs()),
      expanded: false,
      work_expanded: false,
    }));
  }
}
