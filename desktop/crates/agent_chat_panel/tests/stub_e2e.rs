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

  set_backend_command_override(None);
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

  set_backend_command_override(None);
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

  set_backend_command_override(None);
}
