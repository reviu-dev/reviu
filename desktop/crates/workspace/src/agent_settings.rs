use std::path::{Path, PathBuf};

use agent_registry::{AgentId, Registry};
use serde::Deserialize;

use crate::AppProfile;

const DEFAULT_ENABLED_AGENT_IDS: &[&str] = &["claude-acp", "codex-acp", "pi-acp", "gemini"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentSettings {
  pub default_agent: AgentId,
  pub enabled_agents: Vec<AgentId>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct AgentSettingsDoc {
  default_agent: Option<String>,
  backend: Option<String>,
  enabled_agents: Option<Vec<String>>,
}

impl AgentSettings {
  pub fn load() -> AgentId {
    Self::load_preferences().default_agent
  }

  pub fn load_preferences() -> Self {
    let registry = agent_registry::global();
    Self::from_doc(read_settings_doc(), &registry)
  }

  pub fn enabled_agents() -> Vec<AgentId> {
    Self::load_preferences().enabled_agents
  }

  pub fn set_default_agent(agent_id: AgentId) -> Self {
    let registry = agent_registry::global();
    let mut settings = Self::load_preferences();
    if registry
      .get(&agent_id)
      .is_some_and(|agent| agent.is_runnable())
    {
      if !settings.enabled_agents.contains(&agent_id) {
        settings.enabled_agents.push(agent_id.clone());
      }
      settings.default_agent = agent_id;
    }
    persist_preferences(&settings);
    settings
  }

  pub fn set_agent_enabled(agent_id: AgentId, enabled: bool) -> Self {
    let registry = agent_registry::global();
    let mut settings = Self::load_preferences();
    if !registry
      .get(&agent_id)
      .is_some_and(|agent| agent.is_runnable())
    {
      return settings;
    }

    if enabled {
      if !settings.enabled_agents.contains(&agent_id) {
        settings.enabled_agents.push(agent_id.clone());
      }
    } else if settings.enabled_agents.len() > 1 {
      settings.enabled_agents.retain(|id| id != &agent_id);
      if settings.default_agent == agent_id
        && let Some(next) = settings.enabled_agents.first().cloned()
      {
        settings.default_agent = next;
      }
    }

    persist_preferences(&settings);
    settings
  }

  fn from_doc(doc: AgentSettingsDoc, registry: &Registry) -> Self {
    let enabled_was_explicit = doc.enabled_agents.is_some();
    let mut enabled_agents = doc
      .enabled_agents
      .unwrap_or_else(|| {
        DEFAULT_ENABLED_AGENT_IDS
          .iter()
          .map(|id| (*id).to_string())
          .collect()
      })
      .into_iter()
      .map(|id| migrate_backend_key(&id))
      .filter(|id| registry.get(id).is_some_and(|agent| agent.is_runnable()))
      .fold(Vec::new(), |mut agents, id| {
        if !agents.contains(&id) {
          agents.push(id);
        }
        agents
      });

    let stored_default = doc
      .default_agent
      .or(doc.backend)
      .map(|id| migrate_backend_key(&id))
      .unwrap_or_else(agent_chat_panel::default_agent_id);
    let mut default_agent = agent_chat_panel::resolve_agent(registry, &stored_default)
      .or_else(|| enabled_agents.first().cloned())
      .unwrap_or_else(agent_chat_panel::default_agent_id);

    if enabled_agents.is_empty() {
      enabled_agents.push(default_agent.clone());
    } else if !enabled_agents.contains(&default_agent) {
      if enabled_was_explicit {
        default_agent = enabled_agents
          .first()
          .cloned()
          .unwrap_or_else(agent_chat_panel::default_agent_id);
      } else {
        enabled_agents.insert(0, default_agent.clone());
      }
    }

    Self {
      default_agent,
      enabled_agents,
    }
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

pub fn enabled_agent_choices() -> Vec<(AgentId, String)> {
  let registry = agent_registry::global();
  AgentSettings::enabled_agents()
    .into_iter()
    .filter_map(|id| {
      registry
        .get(&id)
        .filter(|agent| agent.is_runnable())
        .map(|agent| (id, agent.display_name().to_string()))
    })
    .collect()
}

fn read_settings_doc() -> AgentSettingsDoc {
  settings_path()
    .and_then(|path| std::fs::read_to_string(&path).ok())
    .and_then(|raw| serde_json::from_str::<AgentSettingsDoc>(&raw).ok())
    .unwrap_or_default()
}

fn read_settings_json() -> serde_json::Value {
  settings_path()
    .and_then(|path| std::fs::read_to_string(path).ok())
    .and_then(|raw| serde_json::from_str(&raw).ok())
    .unwrap_or_else(|| serde_json::json!({}))
}

fn write_settings_json(value: &serde_json::Value) {
  let Some(path) = settings_path() else {
    return;
  };
  if let Some(parent) = path.parent() {
    let _ = std::fs::create_dir_all(parent);
  }
  let _ = std::fs::write(&path, value.to_string());
}

fn persist_preferences(settings: &AgentSettings) {
  let mut json = read_settings_json();
  if !json.is_object() {
    json = serde_json::json!({});
  }
  json["default_agent"] = serde_json::Value::String(settings.default_agent.to_string());
  json["enabled_agents"] = serde_json::Value::Array(
    settings
      .enabled_agents
      .iter()
      .map(|agent_id| serde_json::Value::String(agent_id.to_string()))
      .collect(),
  );
  if let Some(object) = json.as_object_mut() {
    object.remove("backend");
  }
  write_settings_json(&json);
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
  fn default_preferences_enable_the_common_agents() {
    let registry = agent_registry::Registry::embedded();
    let settings = AgentSettings::from_doc(AgentSettingsDoc::default(), &registry);

    assert_eq!(settings.default_agent, agent_chat_panel::default_agent_id());
    assert!(
      settings
        .enabled_agents
        .contains(&AgentId::new("claude-acp"))
    );
    assert!(settings.enabled_agents.contains(&AgentId::new("codex-acp")));
    assert!(settings.enabled_agents.contains(&AgentId::new("pi-acp")));
  }

  #[test]
  fn a_stale_or_unknown_agent_falls_back_to_a_runnable_one() {
    let registry = agent_registry::Registry::embedded();
    let settings = AgentSettings::from_doc(
      AgentSettingsDoc {
        default_agent: Some("withdrawn-agent".to_string()),
        ..Default::default()
      },
      &registry,
    );
    assert_eq!(settings.default_agent, agent_chat_panel::default_agent_id());

    let kept = AgentSettings::from_doc(
      AgentSettingsDoc {
        default_agent: Some("gemini".to_string()),
        ..Default::default()
      },
      &registry,
    );
    assert_eq!(kept.default_agent, AgentId::new("gemini"));
  }

  #[test]
  fn explicit_enabled_agents_keep_the_default_inside_the_selection() {
    let registry = agent_registry::Registry::embedded();
    let settings = AgentSettings::from_doc(
      AgentSettingsDoc {
        default_agent: Some("claude-acp".to_string()),
        enabled_agents: Some(vec!["codex-acp".to_string()]),
        ..Default::default()
      },
      &registry,
    );

    assert_eq!(settings.default_agent, AgentId::new("codex-acp"));
    assert_eq!(settings.enabled_agents, vec![AgentId::new("codex-acp")]);
  }
}
