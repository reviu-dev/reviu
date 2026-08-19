//! The event machine: agent stream intake, buffers, prompt dispatch.

use super::*;

impl AgentChatPanel {
  pub(crate) fn start_event_forwarder(
    &mut self,
    rx: async_channel::Receiver<AgentEvent>,
    cx: &mut Context<Self>,
  ) {
    let task = cx.spawn(async move |this, cx| {
      while let Ok(event) = rx.recv().await {
        let _ = this.update(cx, |panel, cx| {
          panel.on_event(event, cx);
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
    self.pending_md_state = None;
    if self.pending_agent.trim().is_empty() {
      self.pending_agent.clear();
      self.pending_md_state = None;
      return;
    }
    let text = std::mem::take(&mut self.pending_agent);
    self.items.push(ChatItem::Message(ChatMessage {
      role: ChatRole::Agent,
      text,
      images: 0,
      image_data: Vec::new(),
    }));
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
    let Some(session) = self.session.clone() else {
      return false;
    };

    // Chunks that trailed in after the previous turn keep their place
    // ahead of the new prompt instead of being wiped.
    self.flush_turn_buffers();
    let images = std::mem::take(&mut self.staged_images);
    self.items.push(ChatItem::Message(ChatMessage {
      role,
      text: text.clone(),
      images: images.len(),
      image_data: images.clone(),
    }));
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
      let blocks = build_prompt_blocks(text, files, selection, images, cwd).await;
      let result = session.send_prompt_blocks(blocks).await;
      let _ = this.update(cx, |panel, cx| {
        panel.flush_turn_buffers();
        panel.append_turn_summary();
        let mut drain_queue = false;
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
                text,
                images: 0,
                image_data: Vec::new(),
              }));
            } else {
              drain_queue = true;
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
                images: 0,
                image_data: Vec::new(),
              }));
            }
          }
        }
        panel.end_turn();
        // Unpinned groups fold when the turn settles; their rows must remeasure.
        let count = panel.messages_list.item_count();
        if count > 0 {
          panel.messages_list.remeasure_items(0..count);
        }
        if drain_queue && !panel.queued_prompts.is_empty() {
          let next = panel.queued_prompts.remove(0);
          panel.dispatch_prompt(next, cx);
        }
        panel.persist_state();
        panel.sync_list_count();
        cx.emit(AgentChatPanelEvent::TurnFinished);
        panel.refresh_repo_files(cx);
        cx.notify();
      });
    })
    .detach();

    true
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
