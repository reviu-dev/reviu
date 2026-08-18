use std::path::{Path, PathBuf};

use agent_acp::BackendKind;
use serde::{Deserialize, Serialize};

use crate::AppProfile;

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
}
