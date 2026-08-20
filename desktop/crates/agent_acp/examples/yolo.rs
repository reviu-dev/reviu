use agent_acp::{AgentEvent, AgentSession, BackendConfig};
use agent_registry::{AgentId, Registry};
use anyhow::Result;
use std::path::PathBuf;

fn main() -> Result<()> {
  smol::block_on(async {
    let mut args = std::env::args().skip(1);
    let backend_name = args.next().unwrap_or_else(|| "claude-acp".into());
    let prompt = args.next().unwrap_or_else(|| "say hi in one word".into());
    let follow_up = args.next();

    let registry = Registry::load();
    let id = AgentId::new(backend_name.clone());
    let Some(agent) = registry.get(&id).filter(|agent| agent.is_runnable()) else {
      let known: Vec<&str> = registry.runnable().iter().map(|a| a.id.as_str()).collect();
      anyhow::bail!(
        "unknown backend: {backend_name}. Known: {}",
        known.join(", ")
      );
    };
    let (command, command_args) = agent.command().expect("a runnable agent has a command");
    let backend = BackendConfig::new(agent.name.clone(), command, command_args)
      .env(agent.env().to_vec())
      .cli_executable(agent.required_cli());
    eprintln!("[backend] {}", backend.label);
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut session = AgentSession::spawn(backend, cwd, |fut| {
      smol::spawn(fut).detach();
    })
    .await?;

    let events = session.take_events().expect("events channel");
    smol::spawn(async move {
      while let Ok(event) = events.recv().await {
        if let AgentEvent::AgentMessageChunk(chunk) = &event {
          if let agent_client_protocol::schema::ContentBlock::Text(t) = &chunk.content {
            eprint!("{}", t.text);
          }
        }
      }
    })
    .detach();

    eprintln!("[turn 1] sending: {prompt:?}");
    let stop = session.send_prompt(prompt).await?;
    eprintln!("\n[turn 1] stop_reason: {stop:?}");

    if let Some(follow_up) = follow_up {
      eprintln!("[turn 2] sending: {follow_up:?}");
      let stop = session.send_prompt(follow_up).await?;
      eprintln!("\n[turn 2] stop_reason: {stop:?}");
    }

    drop(session);
    Ok(())
  })
}
