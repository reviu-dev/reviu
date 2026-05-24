use std::path::PathBuf;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ConversationMeta {
  pub id: String,
  pub started_at_secs: u64,
  pub updated_at_secs: u64,
  pub title: String,
  pub message_count: usize,
  #[serde(default)]
  pub session_id: Option<String>,
}

pub(crate) fn now_secs() -> u64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_secs())
    .unwrap_or(0)
}

pub(crate) fn new_conversation_meta() -> ConversationMeta {
  let now = now_secs();
  ConversationMeta {
    id: now.to_string(),
    started_at_secs: now,
    updated_at_secs: now,
    title: String::new(),
    message_count: 0,
    session_id: None,
  }
}

pub(crate) fn truncate_title(text: &str) -> String {
  let trimmed = text.trim().lines().next().unwrap_or("").trim();
  let max = 80;
  if trimmed.chars().count() <= max {
    trimmed.to_string()
  } else {
    let head: String = trimmed.chars().take(max).collect();
    format!("{head}...")
  }
}

/// Per-repo directory hosting one file per conversation + an `active.txt` pointer.
pub fn state_dir_for_repo(state_dir: &std::path::Path, repo: &std::path::Path) -> PathBuf {
  let canonical = repo.canonicalize().unwrap_or_else(|_| repo.to_path_buf());
  let hash = blake3::hash(canonical.to_string_lossy().as_bytes());
  let hex = hash.to_hex();
  state_dir.join(&hex.as_str()[..16])
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn truncate_title_keeps_first_line_only() {
    assert_eq!(truncate_title("hi\nthere"), "hi");
  }

  #[test]
  fn truncate_title_caps_at_80_chars() {
    let long = "a".repeat(100);
    let out = truncate_title(&long);
    assert!(out.ends_with("..."));
    assert_eq!(out.chars().count(), 83);
  }

  #[test]
  fn new_conversation_meta_initializes_updated_at() {
    let m = new_conversation_meta();
    assert_eq!(m.started_at_secs, m.updated_at_secs);
    assert_eq!(m.message_count, 0);
    assert!(m.title.is_empty());
    assert!(m.session_id.is_none());
  }
}
