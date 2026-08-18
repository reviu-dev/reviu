//! End to end: the mounted panel drives a real ACP agent process (the stub).

use agent_acp::BackendKind;
use agent_chat_panel::{AgentChatPanel, set_backend_command_override};
use gpui::{AppContext as _, TestAppContext};

#[gpui::test]
async fn a_prompt_round_trips_through_a_real_agent_process(cx: &mut TestAppContext) {
  // A real child process answers off the test scheduler.
  cx.executor().allow_parking();
  set_backend_command_override(Some(env!("CARGO_BIN_EXE_stub_agent").to_string()));

  cx.update(gpui_component::init);
  let cwd = std::env::temp_dir();
  let mut mounted = None;
  let (_root, cx) = cx.add_window_view(|window, cx| {
    let panel =
      cx.new(|cx| AgentChatPanel::new(BackendKind::Claude, cwd.clone(), None, window, cx));
    mounted = Some(panel.clone());
    gpui_component::Root::new(panel, window, cx)
  });
  let panel = mounted.expect("agent chat panel");

  cx.condition(&panel, |panel, _| panel.backend_ready()).await;

  cx.condition(&panel, |panel, _| {
    panel.available_command_names() == ["compact", "review"]
  })
  .await;

  panel.update(cx, |panel, cx| {
    assert!(panel.send_external_prompt("hello stub".to_string(), cx));
  });

  cx.condition(&panel, |panel, _| {
    !panel.is_turn_in_flight() && panel.transcript_texts().len() >= 2
  })
  .await;

  panel.read_with(cx, |panel, _| {
    let transcript = panel.transcript_texts();
    assert_eq!(transcript[0], "hello stub");
    assert_eq!(transcript[1], "ack", "the stub acks every prompt");
    assert_eq!(
      panel.thought_texts(),
      vec!["stub thinking"],
      "the thought chunk lands as a Thought item"
    );
  });

  // A chunk trailing in after the turn ended is kept, and surfaces ahead of
  // the next prompt instead of being wiped by it.
  panel.update(cx, |panel, cx| {
    panel.inject_event_for_test(late_text_chunk("late words"));
    assert!(panel.send_external_prompt("second prompt".to_string(), cx));
  });
  cx.condition(&panel, |panel, _| {
    !panel.is_turn_in_flight() && panel.transcript_texts().len() >= 5
  })
  .await;
  panel.read_with(cx, |panel, _| {
    let transcript = panel.transcript_texts();
    let late = transcript.iter().position(|t| t == "late words");
    let second = transcript.iter().position(|t| t == "second prompt");
    assert!(
      late.is_some() && late < second,
      "the late chunk lands before the next prompt, got {transcript:?}"
    );
  });
}

fn late_text_chunk(text: &str) -> agent_acp::AgentEvent {
  agent_acp::AgentEvent::AgentMessageChunk(agent_client_protocol::schema::ContentChunk::new(
    agent_client_protocol::schema::ContentBlock::Text(
      agent_client_protocol::schema::TextContent::new(text),
    ),
  ))
}

#[gpui::test]
async fn a_permission_request_carries_its_command_and_resumes_on_answer(cx: &mut TestAppContext) {
  cx.executor().allow_parking();
  set_backend_command_override(Some(env!("CARGO_BIN_EXE_stub_agent").to_string()));

  cx.update(gpui_component::init);
  let cwd = std::env::temp_dir();
  let mut mounted = None;
  let (_root, cx) = cx.add_window_view(|window, cx| {
    let panel =
      cx.new(|cx| AgentChatPanel::new(BackendKind::Claude, cwd.clone(), None, window, cx));
    mounted = Some(panel.clone());
    gpui_component::Root::new(panel, window, cx)
  });
  let panel = mounted.expect("agent chat panel");

  cx.condition(&panel, |panel, _| panel.backend_ready()).await;

  panel.update(cx, |panel, cx| {
    assert!(panel.send_external_prompt("ask permission first".to_string(), cx));
  });

  cx.condition(&panel, |panel, _| panel.pending_permission().is_some())
    .await;

  let (prompt_id, invocation) = panel.read_with(cx, |panel, _| panel.pending_permission().unwrap());
  assert_eq!(
    invocation.as_deref(),
    Some("cargo test --workspace"),
    "the card carries the real command from the request"
  );

  panel.update(cx, |panel, cx| {
    panel.answer_permission(prompt_id, Some("allow".to_string()), cx);
  });

  cx.condition(&panel, |panel, _| {
    !panel.is_turn_in_flight() && panel.transcript_texts().len() >= 2
  })
  .await;

  panel.read_with(cx, |panel, _| {
    assert_eq!(panel.transcript_texts()[1], "ack");
    assert!(panel.pending_permission().is_none());
  });
}

#[gpui::test]
async fn cancelling_a_turn_leaves_a_stopped_marker(cx: &mut TestAppContext) {
  cx.executor().allow_parking();
  set_backend_command_override(Some(env!("CARGO_BIN_EXE_stub_agent").to_string()));

  cx.update(gpui_component::init);
  let cwd = std::env::temp_dir();
  let mut mounted = None;
  let (_root, cx) = cx.add_window_view(|window, cx| {
    let panel =
      cx.new(|cx| AgentChatPanel::new(BackendKind::Claude, cwd.clone(), None, window, cx));
    mounted = Some(panel.clone());
    gpui_component::Root::new(panel, window, cx)
  });
  let panel = mounted.expect("agent chat panel");

  cx.condition(&panel, |panel, _| panel.backend_ready()).await;

  panel.update(cx, |panel, cx| {
    // The stub parks a turn whose prompt mentions "wait" until session/cancel.
    assert!(panel.send_external_prompt("wait for me".to_string(), cx));
  });
  cx.condition(&panel, |panel, _| panel.is_turn_in_flight())
    .await;

  panel.update(cx, |panel, cx| panel.cancel_turn(cx));
  cx.condition(&panel, |panel, _| !panel.is_turn_in_flight())
    .await;

  panel.read_with(cx, |panel, _| {
    let transcript = panel.transcript_texts();
    assert!(
      transcript.last().is_some_and(|t| t.starts_with("Stopped")),
      "the cancelled turn leaves a marker, got {transcript:?}"
    );
    assert!(
      transcript.iter().any(|t| t == "partial reply"),
      "prose streamed before the stop survives, got {transcript:?}"
    );
    assert!(
      !transcript.iter().any(|t| t == "ack"),
      "a cancelled turn never acked"
    );
  });
}

#[gpui::test]
async fn a_queued_message_runs_when_the_turn_ends_cleanly(cx: &mut TestAppContext) {
  cx.executor().allow_parking();
  set_backend_command_override(Some(env!("CARGO_BIN_EXE_stub_agent").to_string()));

  cx.update(gpui_component::init);
  let cwd = std::env::temp_dir();
  let mut mounted = None;
  let (_root, cx) = cx.add_window_view(|window, cx| {
    let panel =
      cx.new(|cx| AgentChatPanel::new(BackendKind::Claude, cwd.clone(), None, window, cx));
    mounted = Some(panel.clone());
    gpui_component::Root::new(panel, window, cx)
  });
  let panel = mounted.expect("agent chat panel");

  cx.condition(&panel, |panel, _| panel.backend_ready()).await;

  // The permission gate keeps the first turn open long enough to queue.
  panel.update(cx, |panel, cx| {
    assert!(panel.send_external_prompt("ask permission first".to_string(), cx));
  });
  cx.condition(&panel, |panel, _| panel.pending_permission().is_some())
    .await;

  panel.update(cx, |panel, cx| {
    panel.queue_prompt_for_test("and then this", cx);
    panel.queue_prompt_for_test("and finally that", cx);
  });

  let (prompt_id, _) = panel.read_with(cx, |panel, _| panel.pending_permission().unwrap());
  panel.update(cx, |panel, cx| {
    panel.answer_permission(prompt_id, Some("allow".to_string()), cx);
  });

  cx.condition(&panel, |panel, _| {
    !panel.is_turn_in_flight()
      && panel.queued_prompt_texts().is_empty()
      && panel
        .transcript_texts()
        .iter()
        .filter(|t| *t == "ack")
        .count()
        >= 3
  })
  .await;

  panel.read_with(cx, |panel, _| {
    let transcript = panel.transcript_texts();
    let first = transcript.iter().position(|t| t == "and then this");
    let second = transcript.iter().position(|t| t == "and finally that");
    assert!(
      first.is_some() && second.is_some() && first < second,
      "both queued prompts ran as their own turns, oldest first, got {transcript:?}"
    );
  });
}

#[gpui::test]
async fn a_failed_turn_holds_the_queue(cx: &mut TestAppContext) {
  cx.executor().allow_parking();
  set_backend_command_override(Some(env!("CARGO_BIN_EXE_stub_agent").to_string()));

  cx.update(gpui_component::init);
  let cwd = std::env::temp_dir();
  let mut mounted = None;
  let (_root, cx) = cx.add_window_view(|window, cx| {
    let panel =
      cx.new(|cx| AgentChatPanel::new(BackendKind::Claude, cwd.clone(), None, window, cx));
    mounted = Some(panel.clone());
    gpui_component::Root::new(panel, window, cx)
  });
  let panel = mounted.expect("agent chat panel");

  cx.condition(&panel, |panel, _| panel.backend_ready()).await;

  panel.update(cx, |panel, cx| {
    panel.queue_prompt_for_test("after the failure", cx);
    // The stub errors a turn whose prompt mentions "fail".
    assert!(panel.send_external_prompt("please fail".to_string(), cx));
  });
  cx.condition(&panel, |panel, _| !panel.is_turn_in_flight())
    .await;

  panel.read_with(cx, |panel, _| {
    assert_eq!(
      panel.queued_prompt_texts(),
      vec!["after the failure".to_string()],
      "an errored turn never auto-runs the queue"
    );
    assert!(
      panel
        .transcript_texts()
        .iter()
        .any(|t| t.starts_with("[error]")),
      "the failure is visible in the transcript"
    );
  });
}

#[gpui::test]
async fn stopping_a_turn_keeps_the_queue(cx: &mut TestAppContext) {
  cx.executor().allow_parking();
  set_backend_command_override(Some(env!("CARGO_BIN_EXE_stub_agent").to_string()));

  cx.update(gpui_component::init);
  let cwd = std::env::temp_dir();
  let mut mounted = None;
  let (_root, cx) = cx.add_window_view(|window, cx| {
    let panel =
      cx.new(|cx| AgentChatPanel::new(BackendKind::Claude, cwd.clone(), None, window, cx));
    mounted = Some(panel.clone());
    gpui_component::Root::new(panel, window, cx)
  });
  let panel = mounted.expect("agent chat panel");

  cx.condition(&panel, |panel, _| panel.backend_ready()).await;

  panel.update(cx, |panel, cx| {
    assert!(panel.send_external_prompt("wait for me".to_string(), cx));
  });
  cx.condition(&panel, |panel, _| panel.is_turn_in_flight())
    .await;

  panel.update(cx, |panel, cx| {
    panel.queue_prompt_for_test("held back", cx);
    panel.cancel_turn(cx);
  });
  cx.condition(&panel, |panel, _| !panel.is_turn_in_flight())
    .await;

  panel.read_with(cx, |panel, _| {
    assert_eq!(
      panel.queued_prompt_texts(),
      vec!["held back".to_string()],
      "a user stop never auto-runs the queue"
    );
    assert!(
      !panel.transcript_texts().iter().any(|t| t == "held back"),
      "the queued prompt did not run"
    );
  });
}

#[gpui::test]
async fn an_edited_prompt_rewinds_and_replays_through_a_fresh_session(cx: &mut TestAppContext) {
  cx.executor().allow_parking();
  set_backend_command_override(Some(env!("CARGO_BIN_EXE_stub_agent").to_string()));

  cx.update(gpui_component::init);
  let cwd = std::env::temp_dir();
  let mut mounted = None;
  let (_root, cx) = cx.add_window_view(|window, cx| {
    let panel =
      cx.new(|cx| AgentChatPanel::new(BackendKind::Claude, cwd.clone(), None, window, cx));
    mounted = Some(panel.clone());
    gpui_component::Root::new(panel, window, cx)
  });
  let panel = mounted.expect("agent chat panel");

  cx.condition(&panel, |panel, _| panel.backend_ready()).await;

  panel.update(cx, |panel, cx| {
    assert!(panel.send_external_prompt("old prompt".to_string(), cx));
    // The shell would snapshot the worktree on TurnStarted; stand in for it.
    panel.record_checkpoint("refs/reviu/test-cp".to_string(), cx);
  });
  cx.condition(&panel, |panel, _| {
    !panel.is_turn_in_flight() && panel.transcript_texts().len() >= 2
  })
  .await;

  // Edit the prompt; the panel asks for a rollback, the shell (stood in for
  // here) restores the worktree then truncates, and the fresh session replays.
  panel.update_in(cx, |panel, window, cx| {
    let idx = 1; // [checkpoint, user, thought, agent]
    panel.begin_message_edit(idx, window, cx);
    let input = panel.edit_input_for_test().expect("edit editor");
    input.update(cx, |state, cx| state.set_value("new prompt", window, cx));
    panel.submit_message_edit(cx);
    assert!(panel.truncate_at_checkpoint("refs/reviu/test-cp", cx));
  });

  cx.condition(&panel, |panel, _| {
    !panel.is_turn_in_flight()
      && panel.transcript_texts().iter().any(|t| t == "new prompt")
      && panel.transcript_texts().iter().any(|t| t == "ack")
  })
  .await;

  panel.read_with(cx, |panel, _| {
    let transcript = panel.transcript_texts();
    assert!(
      !transcript.iter().any(|t| t == "old prompt"),
      "the edited turn is gone, got {transcript:?}"
    );
  });
}

#[gpui::test]
async fn reconnect_respawns_the_agent_session(cx: &mut TestAppContext) {
  cx.executor().allow_parking();
  set_backend_command_override(Some(env!("CARGO_BIN_EXE_stub_agent").to_string()));

  cx.update(gpui_component::init);
  let cwd = std::env::temp_dir();
  let mut mounted = None;
  let (_root, cx) = cx.add_window_view(|window, cx| {
    let panel =
      cx.new(|cx| AgentChatPanel::new(BackendKind::Claude, cwd.clone(), None, window, cx));
    mounted = Some(panel.clone());
    gpui_component::Root::new(panel, window, cx)
  });
  let panel = mounted.expect("agent chat panel");

  cx.condition(&panel, |panel, _| panel.backend_ready()).await;

  panel.update(cx, |panel, cx| {
    panel.mark_disconnected_for_test(cx);
  });
  cx.run_until_parked();

  let button = cx
    .debug_bounds("agent-chat-reconnect")
    .expect("reconnect button painted");
  cx.simulate_click(button.center(), gpui::Modifiers::default());

  cx.condition(&panel, |panel, _| panel.backend_ready()).await;

  panel.read_with(cx, |panel, _| {
    assert!(
      panel
        .transcript_texts()
        .iter()
        .any(|t| t == "Agent disconnected."),
      "reconnecting keeps the transcript"
    );
  });
}

#[gpui::test]
async fn enter_sends_the_composer_and_shift_enter_types_a_newline(cx: &mut TestAppContext) {
  cx.executor().allow_parking();
  set_backend_command_override(Some(env!("CARGO_BIN_EXE_stub_agent").to_string()));

  cx.update(gpui_component::init);
  let cwd = std::env::temp_dir();
  let mut mounted = None;
  let (_root, cx) = cx.add_window_view(|window, cx| {
    let panel =
      cx.new(|cx| AgentChatPanel::new(BackendKind::Claude, cwd.clone(), None, window, cx));
    mounted = Some(panel.clone());
    gpui_component::Root::new(panel, window, cx)
  });
  let panel = mounted.expect("agent chat panel");

  cx.condition(&panel, |panel, _| panel.backend_ready()).await;

  let input_focus = panel.read_with(cx, |panel, cx| panel.composer_focus_handle(cx));
  cx.update(|window, cx| window.focus(&input_focus, cx));
  cx.simulate_input("first line");
  cx.simulate_keystrokes("shift-enter");
  cx.simulate_input("second line");
  cx.simulate_keystrokes("enter");

  cx.condition(&panel, |panel, _| {
    !panel.is_turn_in_flight() && panel.transcript_texts().len() >= 2
  })
  .await;

  panel.read_with(cx, |panel, cx| {
    let transcript = panel.transcript_texts();
    assert_eq!(
      transcript[0], "first line\nsecond line",
      "the newline typed with shift-enter reaches the prompt"
    );
    assert_eq!(panel.composer_text(cx), "", "the composer drained on send");
  });
}
