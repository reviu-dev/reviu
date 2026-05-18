use std::path::PathBuf;

use agent_acp::BackendKind;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AgentSettings {
  pub backend: String,
}

impl AgentSettings {
  pub fn load() -> BackendKind {
    let Some(path) = settings_path() else {
      return BackendKind::Claude;
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
      return BackendKind::Claude;
    };
    let parsed: AgentSettings = serde_json::from_str(&raw).unwrap_or_default();
    BackendKind::from_storage_key(&parsed.backend).unwrap_or(BackendKind::Claude)
  }

  pub fn save(backend: BackendKind) {
    let Some(path) = settings_path() else {
      return;
    };
    if let Some(parent) = path.parent() {
      let _ = std::fs::create_dir_all(parent);
    }
    let value = AgentSettings {
      backend: backend.storage_key().to_string(),
    };
    if let Ok(json) = serde_json::to_string(&value) {
      let _ = std::fs::write(&path, json);
    }
  }
}

fn settings_path() -> Option<PathBuf> {
  Some(dirs::config_dir()?.join("reviu").join("agent.json"))
}
