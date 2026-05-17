use agent_acp::{AgentSession, BackendConfig};
use agent_client_protocol::schema::StopReason;

#[test]
fn init_and_prompt_round_trip_against_stub_agent() {
  smol::block_on(async {
    let backend = BackendConfig {
      label: "stub",
      command: env!("CARGO_BIN_EXE_stub_agent"),
      args: Vec::new(),
      install_hint: "test only",
    };
    let cwd = std::env::current_dir().expect("cwd");
    let mut session = AgentSession::spawn(backend, cwd, |fut| {
      smol::spawn(fut).detach();
    })
    .await
    .expect("agent session spawn");

    let info = session.init_info().clone();
    assert_eq!(info.name.as_deref(), Some("stub-agent"));

    // Drain events so they don't backpressure the driver.
    if let Some(events) = session.take_events() {
      smol::spawn(async move {
        while events.recv().await.is_ok() {}
      })
      .detach();
    }

    let stop = session.send_prompt("hi").await.expect("prompt");
    assert!(matches!(stop, StopReason::EndTurn));
  });
}
