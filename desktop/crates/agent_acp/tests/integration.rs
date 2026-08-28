use agent_acp::{AgentEvent, AgentSession, BackendConfig};
use agent_client_protocol::schema::{ContentBlock, ModelId, StopReason};

fn stub_backend() -> BackendConfig {
  BackendConfig::new("stub", env!("CARGO_BIN_EXE_stub_agent"), Vec::new())
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
fn set_model_round_trips_against_stub_agent() {
  smol::block_on(async {
    let mut session = spawn_stub_session().await;
    let info = session.init_info().clone();
    assert_eq!(
      info.current_model_id.as_ref().map(|id| id.0.as_ref()),
      Some("stub-small")
    );

    if let Some(events) = session.take_events() {
      smol::spawn(async move { while events.recv().await.is_ok() {} }).detach();
    }

    session
      .set_model(ModelId::new("stub-large"))
      .await
      .expect("model switch");
  });
}

#[test]
fn agent_message_chunk_arrives_on_events_channel() {
  smol::block_on(async {
    let mut session = spawn_stub_session().await;
    let events = session.take_events().expect("events");
    let recv_task = smol::spawn(async move {
      while let Ok(event) = events.recv().await {
        if let AgentEvent::AgentMessageChunk(chunk) = event
          && let ContentBlock::Text(t) = chunk.content
        {
          return Some(t.text);
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

#[test]
fn a_steer_prompt_joins_the_parked_turn() {
  smol::block_on(async {
    let mut session = spawn_stub_session().await;
    if let Some(events) = session.take_events() {
      smol::spawn(async move { while events.recv().await.is_ok() {} }).detach();
    }
    let session = std::sync::Arc::new(session);
    let main = {
      let session = session.clone();
      smol::spawn(async move { session.send_prompt("wait here").await })
    };
    smol::Timer::after(std::time::Duration::from_millis(300)).await;
    let steer =
      session.steer_prompt_blocks(vec![agent_client_protocol::schema::ContentBlock::Text(
        agent_client_protocol::schema::TextContent::new("steer now"),
      )]);
    let steer_outcome = steer.await.expect("steer resolves");
    assert!(matches!(steer_outcome, agent_acp::SteerOutcome::Injected));
    let main_stop = main.await.expect("main resolves");
    assert!(matches!(main_stop, StopReason::EndTurn));
  });
}

#[test]
fn a_terminal_command_streams_output_and_exit_code_into_the_store() {
  smol::block_on(async {
    let mut session = spawn_stub_session().await;
    if let Some(events) = session.take_events() {
      smol::spawn(async move { while events.recv().await.is_ok() {} }).detach();
    }
    let updates = session.take_terminal_updates().expect("updates channel");
    let store = session.terminal_store();

    let stop = session
      .send_prompt("run a terminal command")
      .await
      .expect("prompt");
    assert!(matches!(stop, StopReason::EndTurn));

    // At least one update fired for the terminal's lifecycle.
    let id = updates.recv().await.expect("a terminal update");
    let snapshot = store.snapshot(&id).expect("snapshot survives release");
    assert!(snapshot.finished, "the command ran to completion");
    assert_eq!(snapshot.exit_code, Some(3));
    assert!(
      snapshot.output.contains("line one") && snapshot.output.contains("line two"),
      "output streamed into the store, got {:?}",
      snapshot.output
    );
    assert!(snapshot.command.contains("/bin/sh"));
  });
}

#[test]
fn killing_a_terminal_finishes_its_turn() {
  smol::block_on(async {
    let mut session = spawn_stub_session().await;
    if let Some(events) = session.take_events() {
      smol::spawn(async move { while events.recv().await.is_ok() {} }).detach();
    }
    let updates = session.take_terminal_updates().expect("updates channel");
    let store = std::sync::Arc::new(session.terminal_store());
    let session = std::sync::Arc::new(session);

    let turn = {
      let session = session.clone();
      smol::spawn(async move { session.send_prompt("terminal sleep").await })
    };

    // Wait for the long-lived command to start, then kill it.
    let id = loop {
      let id = updates.recv().await.expect("a terminal update");
      if let Some(snap) = store.snapshot(&id)
        && snap.output.contains("started")
        && !snap.finished
      {
        break id;
      }
    };
    store.kill(&id);

    let stop = turn.await.expect("turn resolves after kill");
    assert!(matches!(stop, StopReason::EndTurn));
    let snapshot = store.snapshot(&id).expect("snapshot kept");
    assert!(snapshot.finished && snapshot.killed);
    assert!(
      snapshot.exit_code != Some(0),
      "a killed command does not exit cleanly"
    );
  });
}
