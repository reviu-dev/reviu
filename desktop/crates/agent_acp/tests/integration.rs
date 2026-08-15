use agent_acp::{AgentEvent, AgentSession, BackendConfig};
use agent_client_protocol::schema::{ContentBlock, StopReason};

fn stub_backend() -> BackendConfig {
  BackendConfig {
    label: "stub",
    command: env!("CARGO_BIN_EXE_stub_agent"),
    args: Vec::new(),
    install_hint: "test only",
  }
}

async fn spawn_stub_session() -> AgentSession {
  let cwd = std::env::current_dir().expect("cwd");
  AgentSession::spawn(stub_backend(), cwd, |fut| {
    smol::spawn(fut).detach();
  })
  .await
  .expect("agent session spawn")
}

#[test]
fn failed_session_load_falls_back_to_a_fresh_session() {
  smol::block_on(async {
    let cwd = std::env::current_dir().expect("cwd");
    // The stub advertises load_session but errors on session/load: the spawn must
    // still succeed with a brand-new session instead of failing the connection.
    let mut session =
      AgentSession::spawn_with_load(stub_backend(), cwd, "stale-session-id".to_string(), |fut| {
        smol::spawn(fut).detach();
      })
      .await
      .expect("spawn with stale session id");

    let info = session.init_info().clone();
    assert_eq!(info.session_id.as_deref(), Some("stub-session"));

    if let Some(events) = session.take_events() {
      smol::spawn(async move { while events.recv().await.is_ok() {} }).detach();
    }
    let stop = session.send_prompt("hi").await.expect("prompt");
    assert!(matches!(stop, StopReason::EndTurn));
  });
}

#[test]
fn init_and_prompt_round_trip_against_stub_agent() {
  smol::block_on(async {
    let mut session = spawn_stub_session().await;
    let info = session.init_info().clone();
    assert_eq!(info.name.as_deref(), Some("stub-agent"));

    if let Some(events) = session.take_events() {
      smol::spawn(async move { while events.recv().await.is_ok() {} }).detach();
    }

    let stop = session.send_prompt("hi").await.expect("prompt");
    assert!(matches!(stop, StopReason::EndTurn));
  });
}

#[test]
fn multi_turn_against_stub_agent() {
  smol::block_on(async {
    let mut session = spawn_stub_session().await;
    if let Some(events) = session.take_events() {
      smol::spawn(async move { while events.recv().await.is_ok() {} }).detach();
    }

    for _ in 0..3 {
      let stop = session.send_prompt("hi").await.expect("prompt");
      assert!(matches!(stop, StopReason::EndTurn));
    }
  });
}

#[test]
fn agent_message_chunk_arrives_on_events_channel() {
  smol::block_on(async {
    let mut session = spawn_stub_session().await;
    let events = session.take_events().expect("events");
    let recv_task = smol::spawn(async move {
      while let Ok(event) = events.recv().await {
        if let AgentEvent::AgentMessageChunk(chunk) = event {
          if let ContentBlock::Text(t) = chunk.content {
            return Some(t.text);
          }
        }
      }
      None
    });

    session.send_prompt("hi").await.expect("prompt");
    drop(session);
    let collected = recv_task.await;
    assert_eq!(collected.as_deref(), Some("ack"));
  });
}
