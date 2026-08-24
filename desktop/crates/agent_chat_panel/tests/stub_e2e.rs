//! End to end: the mounted panel drives a real ACP agent process (the stub).

use agent_chat_panel::{AgentChatPanel, default_agent_id, set_backend_command_override};
use agent_registry::AgentId;
use gpui::{AppContext as _, TestAppContext};

#[gpui::test]
async fn a_prompt_round_trips_through_a_real_agent_process(cx: &mut TestAppContext) {
  // A real child process answers off the test scheduler.
  cx.executor().allow_parking();
  set_backend_command_override(Some(env!("CARGO_BIN_EXE_stub_agent_panel").to_string()));

  cx.update(gpui_component::init);
  let cwd = std::env::temp_dir();
  let mut mounted = None;
  let (_root, cx) = cx.add_window_view(|window, cx| {
    let panel = cx.new(|cx| AgentChatPanel::new(default_agent_id(), cwd.clone(), None, window, cx));
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
    panel.inject_event_for_test(late_text_chunk("late words"), cx);
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
  set_backend_command_override(Some(env!("CARGO_BIN_EXE_stub_agent_panel").to_string()));

  cx.update(gpui_component::init);
  let cwd = std::env::temp_dir();
  let mut mounted = None;
  let (_root, cx) = cx.add_window_view(|window, cx| {
    let panel = cx.new(|cx| AgentChatPanel::new(default_agent_id(), cwd.clone(), None, window, cx));
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
async fn a_staged_image_reaches_the_agent_as_an_image_block(cx: &mut TestAppContext) {
  cx.executor().allow_parking();
  set_backend_command_override(Some(env!("CARGO_BIN_EXE_stub_agent_panel").to_string()));

  cx.update(gpui_component::init);
  let cwd = std::env::temp_dir();
  let mut mounted = None;
  let (_root, cx) = cx.add_window_view(|window, cx| {
    let panel = cx.new(|cx| AgentChatPanel::new(default_agent_id(), cwd.clone(), None, window, cx));
    mounted = Some(panel.clone());
    gpui_component::Root::new(panel, window, cx)
  });
  let panel = mounted.expect("agent chat panel");

  cx.condition(&panel, |panel, _| panel.backend_ready()).await;

  panel.update(cx, |panel, cx| {
    panel.stage_image_for_test(
      gpui::Image::from_bytes(gpui::ImageFormat::Png, vec![1, 2, 3, 4]),
      cx,
    );
    assert_eq!(panel.staged_image_count(), 1, "the stub advertises images");
    assert!(panel.send_external_prompt("look at this".to_string(), cx));
    assert_eq!(panel.staged_image_count(), 0, "sending drains the staging");
  });

  cx.condition(&panel, |panel, _| {
    !panel.is_turn_in_flight()
      && panel
        .transcript_texts()
        .iter()
        .any(|t| t == "image received")
  })
  .await;
}

#[gpui::test]
async fn cancelling_a_turn_leaves_a_stopped_marker(cx: &mut TestAppContext) {
  cx.executor().allow_parking();
  set_backend_command_override(Some(env!("CARGO_BIN_EXE_stub_agent_panel").to_string()));

  cx.update(gpui_component::init);
  let cwd = std::env::temp_dir();
  let mut mounted = None;
  let (_root, cx) = cx.add_window_view(|window, cx| {
    let panel = cx.new(|cx| AgentChatPanel::new(default_agent_id(), cwd.clone(), None, window, cx));
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
  set_backend_command_override(Some(env!("CARGO_BIN_EXE_stub_agent_panel").to_string()));

  cx.update(gpui_component::init);
  let cwd = std::env::temp_dir();
  let mut mounted = None;
  let (_root, cx) = cx.add_window_view(|window, cx| {
    let panel = cx.new(|cx| AgentChatPanel::new(default_agent_id(), cwd.clone(), None, window, cx));
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
  set_backend_command_override(Some(env!("CARGO_BIN_EXE_stub_agent_panel").to_string()));

  cx.update(gpui_component::init);
  let cwd = std::env::temp_dir();
  let mut mounted = None;
  let (_root, cx) = cx.add_window_view(|window, cx| {
    let panel = cx.new(|cx| AgentChatPanel::new(default_agent_id(), cwd.clone(), None, window, cx));
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
  set_backend_command_override(Some(env!("CARGO_BIN_EXE_stub_agent_panel").to_string()));

  cx.update(gpui_component::init);
  let cwd = std::env::temp_dir();
  let mut mounted = None;
  let (_root, cx) = cx.add_window_view(|window, cx| {
    let panel = cx.new(|cx| AgentChatPanel::new(default_agent_id(), cwd.clone(), None, window, cx));
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
  set_backend_command_override(Some(env!("CARGO_BIN_EXE_stub_agent_panel").to_string()));

  cx.update(gpui_component::init);
  let cwd = std::env::temp_dir();
  let mut mounted = None;
  let (_root, cx) = cx.add_window_view(|window, cx| {
    let panel = cx.new(|cx| AgentChatPanel::new(default_agent_id(), cwd.clone(), None, window, cx));
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
async fn a_session_load_replay_never_duplicates_the_transcript(cx: &mut TestAppContext) {
  cx.executor().allow_parking();
  set_backend_command_override(Some(env!("CARGO_BIN_EXE_stub_agent_panel").to_string()));

  cx.update(gpui_component::init);
  let cwd = std::env::temp_dir();
  let mut mounted = None;
  let (_root, cx) = cx.add_window_view(|window, cx| {
    let panel = cx.new(|cx| AgentChatPanel::new(default_agent_id(), cwd.clone(), None, window, cx));
    mounted = Some(panel.clone());
    gpui_component::Root::new(panel, window, cx)
  });
  let panel = mounted.expect("agent chat panel");

  cx.condition(&panel, |panel, _| panel.backend_ready()).await;

  panel.update(cx, |panel, cx| {
    assert!(panel.send_external_prompt("hello stub".to_string(), cx));
  });
  cx.condition(&panel, |panel, _| {
    !panel.is_turn_in_flight() && panel.transcript_texts().len() >= 2
  })
  .await;

  // A respawn with a stored session id goes through session/load, which the
  // stub answers by replaying the whole history first.
  panel.update(cx, |panel, cx| panel.mark_disconnected_for_test(cx));
  cx.run_until_parked();
  let button = cx
    .debug_bounds("agent-chat-reconnect")
    .expect("reconnect button painted");
  cx.simulate_click(button.center(), gpui::Modifiers::default());
  cx.condition(&panel, |panel, _| panel.backend_ready()).await;

  panel.read_with(cx, |panel, _| {
    let transcript = panel.transcript_texts();
    assert_eq!(
      transcript.iter().filter(|t| *t == "ack").count(),
      1,
      "the replayed reply was dropped, got {transcript:?}"
    );
    assert_eq!(
      panel.thought_texts().len(),
      1,
      "the replayed thought was dropped"
    );
  });
}

#[gpui::test]
async fn reconnect_respawns_the_agent_session(cx: &mut TestAppContext) {
  cx.executor().allow_parking();
  set_backend_command_override(Some(env!("CARGO_BIN_EXE_stub_agent_panel").to_string()));

  cx.update(gpui_component::init);
  let cwd = std::env::temp_dir();
  let mut mounted = None;
  let (_root, cx) = cx.add_window_view(|window, cx| {
    let panel = cx.new(|cx| AgentChatPanel::new(default_agent_id(), cwd.clone(), None, window, cx));
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
  set_backend_command_override(Some(env!("CARGO_BIN_EXE_stub_agent_panel").to_string()));

  cx.update(gpui_component::init);
  let cwd = std::env::temp_dir();
  let mut mounted = None;
  let (_root, cx) = cx.add_window_view(|window, cx| {
    let panel = cx.new(|cx| AgentChatPanel::new(default_agent_id(), cwd.clone(), None, window, cx));
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

#[gpui::test]
async fn a_turn_with_edits_closes_on_an_aggregated_summary_card(cx: &mut TestAppContext) {
  cx.executor().allow_parking();
  set_backend_command_override(Some(env!("CARGO_BIN_EXE_stub_agent_panel").to_string()));

  cx.update(gpui_component::init);
  let cwd = std::env::temp_dir();
  let mut mounted = None;
  let (_root, cx) = cx.add_window_view(|window, cx| {
    let panel = cx.new(|cx| AgentChatPanel::new(default_agent_id(), cwd.clone(), None, window, cx));
    mounted = Some(panel.clone());
    gpui_component::Root::new(panel, window, cx)
  });
  let panel = mounted.expect("agent chat panel");

  cx.condition(&panel, |panel, _| panel.backend_ready()).await;

  panel.update(cx, |panel, cx| {
    assert!(panel.send_external_prompt("please edit the file".to_string(), cx));
  });
  cx.condition(&panel, |panel, _| {
    !panel.is_turn_in_flight() && panel.transcript_texts().len() >= 2
  })
  .await;

  panel.read_with(cx, |panel, _| {
    assert_eq!(
      panel.turn_summary_rows(),
      vec![vec![("src/stub.rs".to_string(), 1, 0)]],
      "the edit diff lands as one summary card row"
    );
    assert!(
      matches!(panel.turn_summary_durations().as_slice(), [Some(_)]),
      "the card records how long the turn ran"
    );
  });

  // A follow-up turn without edits adds no card.
  panel.update(cx, |panel, cx| {
    assert!(panel.send_external_prompt("hello again".to_string(), cx));
  });
  cx.condition(&panel, |panel, _| {
    !panel.is_turn_in_flight() && panel.transcript_texts().len() >= 4
  })
  .await;
  panel.read_with(cx, |panel, _| {
    assert_eq!(panel.turn_summary_rows().len(), 1);
  });
}

#[gpui::test]
async fn auto_approve_answers_permissions_without_a_click(cx: &mut TestAppContext) {
  cx.executor().allow_parking();
  set_backend_command_override(Some(env!("CARGO_BIN_EXE_stub_agent_panel").to_string()));

  cx.update(gpui_component::init);
  let cwd = std::env::temp_dir();
  let mut mounted = None;
  let (_root, cx) = cx.add_window_view(|window, cx| {
    let panel = cx.new(|cx| AgentChatPanel::new(default_agent_id(), cwd.clone(), None, window, cx));
    mounted = Some(panel.clone());
    gpui_component::Root::new(panel, window, cx)
  });
  let panel = mounted.expect("agent chat panel");

  cx.condition(&panel, |panel, _| panel.backend_ready()).await;

  panel.update(cx, |panel, cx| {
    panel.toggle_auto_approve(cx);
    assert!(panel.send_external_prompt("ask permission first".to_string(), cx));
  });

  // The turn completes with no human answer.
  cx.condition(&panel, |panel, _| {
    !panel.is_turn_in_flight() && panel.transcript_texts().len() >= 2
  })
  .await;

  panel.read_with(cx, |panel, _| {
    assert!(panel.pending_permission().is_none());
    assert_eq!(
      panel.permission_answers(),
      vec![(Some("allow".to_string()), true)],
      "the allow option was picked automatically"
    );
    assert_eq!(panel.transcript_texts()[1], "ack");
  });
}

#[gpui::test]
async fn enabling_auto_approve_answers_the_permission_already_waiting(cx: &mut TestAppContext) {
  cx.executor().allow_parking();
  set_backend_command_override(Some(env!("CARGO_BIN_EXE_stub_agent_panel").to_string()));

  cx.update(gpui_component::init);
  let cwd = std::env::temp_dir();
  let mut mounted = None;
  let (_root, cx) = cx.add_window_view(|window, cx| {
    let panel = cx.new(|cx| AgentChatPanel::new(default_agent_id(), cwd.clone(), None, window, cx));
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

  // Flipping the toggle drains the parked prompt and the turn resumes.
  panel.update(cx, |panel, cx| {
    panel.toggle_auto_approve(cx);
  });
  cx.condition(&panel, |panel, _| {
    !panel.is_turn_in_flight() && panel.transcript_texts().len() >= 2
  })
  .await;

  panel.read_with(cx, |panel, _| {
    assert_eq!(
      panel.permission_answers(),
      vec![(Some("allow".to_string()), true)]
    );
  });
}

#[gpui::test]
async fn a_sent_prompt_holds_at_the_viewport_top_through_the_turn(cx: &mut TestAppContext) {
  cx.executor().allow_parking();
  set_backend_command_override(Some(env!("CARGO_BIN_EXE_stub_agent_panel").to_string()));

  cx.update(gpui_component::init);
  let cwd = std::env::temp_dir();
  let mut mounted = None;
  let (_root, cx) = cx.add_window_view(|window, cx| {
    let panel = cx.new(|cx| AgentChatPanel::new(default_agent_id(), cwd.clone(), None, window, cx));
    mounted = Some(panel.clone());
    gpui_component::Root::new(panel, window, cx)
  });
  let panel = mounted.expect("agent chat panel");

  cx.condition(&panel, |panel, _| panel.backend_ready()).await;

  panel.update(cx, |panel, cx| {
    assert!(panel.send_external_prompt("hello stub".to_string(), cx));
  });
  panel.read_with(cx, |panel, _| {
    let (active, following, end_space) = panel.runway_state();
    assert!(active && following, "the send arms the runway");
    assert!(end_space > 0.0, "a provisional reservation is seeded");
  });

  cx.condition(&panel, |panel, _| {
    !panel.is_turn_in_flight() && panel.transcript_texts().len() >= 2
  })
  .await;
  // Settle a couple of frames so the reservation is trued up from bounds.
  panel.update(cx, |_, cx| cx.notify());
  cx.run_until_parked();
  panel.update(cx, |_, cx| cx.notify());
  cx.run_until_parked();

  panel.read_with(cx, |panel, _| {
    let (active, following, _) = panel.runway_state();
    assert!(
      active && following,
      "a short reply leaves the hold in place after the turn"
    );
    let (anchor_top, viewport_top) = panel.runway_anchor_top().expect("anchor painted");
    // A short first transcript clamps at the list start, so the prompt sits
    // between the top and the held margin.
    assert!(
      anchor_top >= viewport_top - 1.0 && anchor_top <= viewport_top + 17.0,
      "the prompt rests near the viewport top: {anchor_top} vs {viewport_top}"
    );
  });
}

#[gpui::test]
async fn a_steer_joins_the_running_turn_without_closing_it(cx: &mut TestAppContext) {
  cx.executor().allow_parking();
  set_backend_command_override(Some(env!("CARGO_BIN_EXE_stub_agent_panel").to_string()));

  cx.update(gpui_component::init);
  let cwd = std::env::temp_dir();
  let mut mounted = None;
  let (_root, cx) = cx.add_window_view(|window, cx| {
    let panel = cx.new(|cx| AgentChatPanel::new(default_agent_id(), cwd.clone(), None, window, cx));
    mounted = Some(panel.clone());
    gpui_component::Root::new(panel, window, cx)
  });
  let panel = mounted.expect("agent chat panel");

  cx.condition(&panel, |panel, _| panel.backend_ready()).await;

  // The stub parks this turn until something releases it.
  panel.update(cx, |panel, cx| {
    assert!(panel.send_external_prompt("wait here".to_string(), cx));
  });
  // "partial reply" is still streaming prose at this point, not an item.
  cx.condition(&panel, |panel, _| {
    panel.is_turn_in_flight() && panel.streaming_prose().contains("partial reply")
  })
  .await;

  panel.update(cx, |panel, cx| {
    assert!(panel.steer_prompt("steer now".to_string(), cx));
    assert!(
      panel.is_turn_in_flight(),
      "the steer joins the turn instead of closing it"
    );
  });

  cx.condition(&panel, |panel, _| !panel.is_turn_in_flight())
    .await;

  panel.read_with(cx, |panel, _| {
    let transcript = panel.transcript_texts();
    let wait = transcript.iter().position(|t| t == "wait here");
    let partial = transcript.iter().position(|t| t == "partial reply");
    let steer = transcript.iter().position(|t| t == "steer now");
    let steered = transcript.iter().position(|t| t == "steered reply");
    assert!(
      wait < partial && partial < steer && steer < steered,
      "the steered message lands inside the turn, got {transcript:?}"
    );
    assert!(
      !transcript.iter().any(|t| t.starts_with("Stopped")),
      "a superseded predecessor is not a stop, got {transcript:?}"
    );
  });
}

#[gpui::test]
async fn a_refused_steer_re_queues_the_message(cx: &mut TestAppContext) {
  cx.executor().allow_parking();
  set_backend_command_override(Some(env!("CARGO_BIN_EXE_stub_agent_panel").to_string()));

  cx.update(gpui_component::init);
  let cwd = std::env::temp_dir();
  let mut mounted = None;
  let (_root, cx) = cx.add_window_view(|window, cx| {
    let panel = cx.new(|cx| AgentChatPanel::new(default_agent_id(), cwd.clone(), None, window, cx));
    mounted = Some(panel.clone());
    gpui_component::Root::new(panel, window, cx)
  });
  let panel = mounted.expect("agent chat panel");

  cx.condition(&panel, |panel, _| panel.backend_ready()).await;

  panel.update(cx, |panel, cx| {
    assert!(panel.send_external_prompt("wait here".to_string(), cx));
  });
  cx.condition(&panel, |panel, _| panel.is_turn_in_flight())
    .await;

  // The stub errors any prompt containing "fail": the steer is refused while
  // the main turn keeps running.
  panel.update(cx, |panel, cx| {
    assert!(panel.steer_prompt("fail this one".to_string(), cx));
  });
  cx.condition(&panel, |panel, _| !panel.queued_prompt_texts().is_empty())
    .await;

  panel.read_with(cx, |panel, _| {
    assert!(
      panel.is_turn_in_flight(),
      "the main turn survives the refusal"
    );
    assert_eq!(panel.queued_prompt_texts(), vec!["fail this one"]);
    let transcript = panel.transcript_texts();
    assert!(
      !transcript.iter().any(|t| t == "fail this one"),
      "the optimistic bubble was retracted, got {transcript:?}"
    );
  });

  panel.update(cx, |panel, cx| panel.cancel_turn(cx));
  cx.condition(&panel, |panel, _| !panel.is_turn_in_flight())
    .await;
}

#[gpui::test]
async fn a_terminal_tool_call_streams_output_into_the_transcript(cx: &mut TestAppContext) {
  cx.executor().allow_parking();
  set_backend_command_override(Some(env!("CARGO_BIN_EXE_stub_agent_panel").to_string()));

  cx.update(gpui_component::init);
  let cwd = std::env::temp_dir();
  let mut mounted = None;
  let (_root, cx) = cx.add_window_view(|window, cx| {
    let panel = cx.new(|cx| AgentChatPanel::new(default_agent_id(), cwd.clone(), None, window, cx));
    mounted = Some(panel.clone());
    gpui_component::Root::new(panel, window, cx)
  });
  let panel = mounted.expect("agent chat panel");

  cx.condition(&panel, |panel, _| panel.backend_ready()).await;

  panel.update(cx, |panel, cx| {
    assert!(panel.send_external_prompt("run a terminal command".to_string(), cx));
  });
  cx.condition(&panel, |panel, _| {
    !panel.is_turn_in_flight() && !panel.tool_terminal_ids().is_empty()
  })
  .await;

  let id = panel.read_with(cx, |panel, _| panel.tool_terminal_ids()[0].clone());
  panel.read_with(cx, |panel, _| {
    let snap = panel
      .terminal_snapshot(&id)
      .expect("the snapshot outlives the release");
    assert!(snap.finished);
    assert_eq!(snap.exit_code, Some(3));
    assert!(
      snap.output.contains("line one") && snap.output.contains("line two"),
      "live output captured, got {:?}",
      snap.output
    );
  });

  // The embedded terminal paints with its command and output.
  panel.update(cx, |_, cx| cx.notify());
  cx.run_until_parked();
  assert!(
    cx.debug_bounds("agent-terminal-block").is_some(),
    "the terminal block is painted in the tool row"
  );
}

#[gpui::test]
async fn the_stop_button_kills_a_running_terminal_command(cx: &mut TestAppContext) {
  cx.executor().allow_parking();
  set_backend_command_override(Some(env!("CARGO_BIN_EXE_stub_agent_panel").to_string()));

  cx.update(gpui_component::init);
  let cwd = std::env::temp_dir();
  let mut mounted = None;
  let (_root, cx) = cx.add_window_view(|window, cx| {
    let panel = cx.new(|cx| AgentChatPanel::new(default_agent_id(), cwd.clone(), None, window, cx));
    mounted = Some(panel.clone());
    gpui_component::Root::new(panel, window, cx)
  });
  let panel = mounted.expect("agent chat panel");

  cx.condition(&panel, |panel, _| panel.backend_ready()).await;

  panel.update(cx, |panel, cx| {
    assert!(panel.send_external_prompt("terminal sleep please".to_string(), cx));
  });
  // Wait for the long-lived command to actually start.
  cx.condition(&panel, |panel, _| {
    panel
      .tool_terminal_ids()
      .first()
      .and_then(|id| panel.terminal_snapshot(id))
      .is_some_and(|snap| snap.output.contains("started") && !snap.finished)
  })
  .await;

  let stop = cx
    .debug_bounds("agent-terminal-stop")
    .expect("a running command offers its stop button");
  cx.simulate_click(stop.center(), gpui::Modifiers::default());

  cx.condition(&panel, |panel, _| !panel.is_turn_in_flight())
    .await;
  panel.read_with(cx, |panel, _| {
    let id = panel.tool_terminal_ids()[0].clone();
    let snap = panel.terminal_snapshot(&id).expect("snapshot kept");
    assert!(snap.finished && snap.killed, "the stop button killed it");
  });
}

#[gpui::test]
async fn cancelling_after_an_accepted_steer_settles_the_turn_cleanly(cx: &mut TestAppContext) {
  cx.executor().allow_parking();
  set_backend_command_override(Some(env!("CARGO_BIN_EXE_stub_agent_panel").to_string()));

  cx.update(gpui_component::init);
  let cwd = std::env::temp_dir();
  let mut mounted = None;
  let (_root, cx) = cx.add_window_view(|window, cx| {
    let panel = cx.new(|cx| AgentChatPanel::new(default_agent_id(), cwd.clone(), None, window, cx));
    mounted = Some(panel.clone());
    gpui_component::Root::new(panel, window, cx)
  });
  let panel = mounted.expect("agent chat panel");

  cx.condition(&panel, |panel, _| panel.backend_ready()).await;

  panel.update(cx, |panel, cx| {
    assert!(panel.send_external_prompt("wait right here".to_string(), cx));
  });
  // Let the pre-steer prose land first, so the flush at steer time is stable.
  cx.condition(&panel, |panel, _| {
    panel.is_turn_in_flight() && panel.streaming_prose().contains("partial reply")
  })
  .await;

  // The stub injects this steer but keeps the turn parked.
  panel.update(cx, |panel, cx| {
    assert!(panel.steer_prompt("hold this steer".to_string(), cx));
  });
  cx.condition(&panel, |panel, _| {
    panel.streaming_prose().contains("steered reply")
  })
  .await;

  panel.update(cx, |panel, cx| panel.cancel_turn(cx));
  cx.condition(&panel, |panel, _| !panel.is_turn_in_flight())
    .await;

  panel.read_with(cx, |panel, _| {
    let transcript = panel.transcript_texts();
    assert!(
      transcript.iter().any(|t| t == "hold this steer"),
      "the accepted steer keeps its bubble, got {transcript:?}"
    );
    assert!(
      transcript.iter().any(|t| t.contains("steered reply")),
      "the streamed reply survives the cancel, got {transcript:?}"
    );
    assert!(
      transcript.last().is_some_and(|t| t.starts_with("Stopped")),
      "the cancel leaves its marker"
    );
    assert!(
      panel.queued_prompt_texts().is_empty(),
      "an accepted steer is not re-queued by the cancel"
    );
  });
}

#[gpui::test]
async fn codex_style_terminal_metadata_streams_into_the_transcript(cx: &mut TestAppContext) {
  cx.executor().allow_parking();
  set_backend_command_override(Some(env!("CARGO_BIN_EXE_stub_agent_panel").to_string()));

  cx.update(gpui_component::init);
  let cwd = std::env::temp_dir();
  let mut mounted = None;
  let (_root, cx) = cx.add_window_view(|window, cx| {
    let panel = cx.new(|cx| AgentChatPanel::new(default_agent_id(), cwd.clone(), None, window, cx));
    mounted = Some(panel.clone());
    gpui_component::Root::new(panel, window, cx)
  });
  let panel = mounted.expect("agent chat panel");

  cx.condition(&panel, |panel, _| panel.backend_ready()).await;

  panel.update(cx, |panel, cx| {
    assert!(panel.send_external_prompt("codex run".to_string(), cx));
  });
  cx.condition(&panel, |panel, _| !panel.is_turn_in_flight())
    .await;

  panel.read_with(cx, |panel, _| {
    assert_eq!(
      panel.tool_terminal_ids(),
      vec!["codex-item-1".to_string()],
      "the agent-owned terminal id reaches the tool item"
    );
    let snap = panel
      .terminal_snapshot("codex-item-1")
      .expect("metadata fed the store");
    assert_eq!(snap.output, "chunk one\nchunk two\n");
    assert!(snap.finished);
    assert_eq!(snap.exit_code, Some(0));
    assert!(!snap.can_kill, "no stop control for agent-owned commands");
  });

  panel.update(cx, |_, cx| cx.notify());
  cx.run_until_parked();
  assert!(
    cx.debug_bounds("agent-terminal-block").is_some(),
    "the streamed output is painted in the tool row"
  );
  assert!(
    cx.debug_bounds("agent-terminal-stop").is_none(),
    "no stop button on a finished agent-owned command"
  );
}

#[gpui::test]
async fn switching_to_pi_respawns_the_session_on_the_new_backend(cx: &mut TestAppContext) {
  cx.executor().allow_parking();
  set_backend_command_override(Some(env!("CARGO_BIN_EXE_stub_agent_panel").to_string()));

  cx.update(gpui_component::init);
  let cwd = std::env::temp_dir();
  let mut mounted = None;
  let (_root, cx) = cx.add_window_view(|window, cx| {
    let panel = cx.new(|cx| AgentChatPanel::new(default_agent_id(), cwd.clone(), None, window, cx));
    mounted = Some(panel.clone());
    gpui_component::Root::new(panel, window, cx)
  });
  let panel = mounted.expect("agent chat panel");

  cx.condition(&panel, |panel, _| panel.backend_ready()).await;

  panel.update(cx, |panel, cx| {
    panel.switch_backend(AgentId::new("pi-acp"), cx);
  });
  cx.condition(&panel, |panel, _| panel.backend_ready()).await;

  panel.read_with(cx, |panel, _| {
    assert_eq!(panel.backend_kind(), &AgentId::new("pi-acp"));
  });

  panel.update(cx, |panel, cx| {
    assert!(panel.send_external_prompt("hello pi".to_string(), cx));
  });
  cx.condition(&panel, |panel, _| {
    !panel.is_turn_in_flight() && panel.transcript_texts().len() >= 2
  })
  .await;

  panel.read_with(cx, |panel, _| {
    let transcript = panel.transcript_texts();
    assert_eq!(transcript[0], "hello pi");
    assert_eq!(transcript[1], "ack");
  });
}
