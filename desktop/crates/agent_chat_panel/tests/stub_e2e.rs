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
  });

  set_backend_command_override(None);
}
