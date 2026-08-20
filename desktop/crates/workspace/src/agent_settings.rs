use std::path::{Path, PathBuf};

use agent_registry::AgentId;
use serde::{Deserialize, Serialize};

use crate::AppProfile;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AgentSettings {
  pub backend: String,
}

impl AgentSettings {
  pub fn load() -> AgentId {
    let registry = agent_registry::global();
    let stored = settings_path()
      .and_then(|path| std::fs::read_to_string(&path).ok())
      .and_then(|raw| serde_json::from_str::<AgentSettings>(&raw).ok())
      .map(|parsed| migrate_backend_key(&parsed.backend))
      .unwrap_or_else(agent_chat_panel::default_agent_id);

    agent_chat_panel::resolve_agent(&registry, &stored)
      .unwrap_or_else(agent_chat_panel::default_agent_id)
  }
}

/// Reviu shipped with its own agent keys before the ACP registry; map those to
/// registry ids so an upgrade keeps the user's agent and model choice.
pub fn migrate_backend_key(stored: &str) -> AgentId {
  match stored {
    "claude" => AgentId::new("claude-acp"),
    "codex" => AgentId::new("codex-acp"),
    "pi" => AgentId::new("pi-acp"),
    other => AgentId::new(other),
  }
}

fn settings_path() -> Option<PathBuf> {
  Some(settings_path_in(
    &dirs::config_dir()?,
    AppProfile::current(),
  ))
}

fn settings_path_in(base: &Path, profile: AppProfile) -> PathBuf {
  base.join(profile.storage_dir_name()).join("agent.json")
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn agent_settings_path_uses_profile_namespace() {
    let base = Path::new("/tmp/reviu-config");

    assert_eq!(
      settings_path_in(base, AppProfile::Prod),
      PathBuf::from("/tmp/reviu-config/reviu/agent.json")
    );
    assert_eq!(
      settings_path_in(base, AppProfile::Dev),
      PathBuf::from("/tmp/reviu-config/reviu.dev/agent.json")
    );
  }

  #[test]
  fn legacy_backend_keys_map_onto_registry_ids() {
    for (stored, expected) in [
      ("claude", "claude-acp"),
      ("codex", "codex-acp"),
      ("pi", "pi-acp"),
    ] {
      assert_eq!(migrate_backend_key(stored), AgentId::new(expected));
    }
  }

  #[test]
  fn a_registry_id_already_stored_is_left_alone() {
    assert_eq!(
      migrate_backend_key("gemini"),
      AgentId::new("gemini"),
      "ids that are already registry ids must not be rewritten"
    );
  }

  #[test]
  fn a_stale_or_unknown_agent_falls_back_to_a_runnable_one() {
    let registry = agent_registry::Registry::embedded();
    let resolved = agent_chat_panel::resolve_agent(&registry, &AgentId::new("withdrawn-agent"))
      .expect("the embedded registry always has a runnable agent");
    assert_eq!(resolved, agent_chat_panel::default_agent_id());

    let kept = agent_chat_panel::resolve_agent(&registry, &AgentId::new("gemini"))
      .expect("gemini is runnable in the snapshot");
    assert_eq!(kept, AgentId::new("gemini"));
  }
}
