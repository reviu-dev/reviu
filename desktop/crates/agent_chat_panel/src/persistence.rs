use std::path::PathBuf;

#[allow(clippy::wildcard_imports)]
use super::*;

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

/// Millis + pid + a process counter: two conversations created in the same
/// second (or by two app instances) never share a file.
pub(crate) fn unique_conversation_id() -> String {
  static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
  let millis = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_millis())
    .unwrap_or(0);
  let count = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
  format!("{millis}-{}-{count}", std::process::id())
}

pub(crate) fn new_conversation_meta() -> ConversationMeta {
  let now = now_secs();
  ConversationMeta {
    id: unique_conversation_id(),
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

pub(crate) type LoadedConversation = (
  ConversationMeta,
  Vec<ChatItem>,
  HashMap<ToolCallId, usize>,
  HashMap<ToolCallId, bool>,
  bool,
);

pub(crate) fn load_active_conversation(dir: &std::path::Path) -> Option<LoadedConversation> {
  let active_path = dir.join("active.txt");
  let active_id = std::fs::read_to_string(&active_path).ok()?;
  let active_id = active_id.trim().to_string();
  if active_id.is_empty() {
    return None;
  }
  let conv_path = dir.join(format!("{active_id}.json"));
  load_conversation_file(&conv_path)
}

pub(crate) fn load_conversation_file(path: &std::path::Path) -> Option<LoadedConversation> {
  let raw = std::fs::read_to_string(path).ok()?;
  let parsed: PersistedConversation = serde_json::from_str(&raw).ok()?;
  let mut items = Vec::with_capacity(parsed.items.len());
  let mut index = HashMap::new();
  for item in parsed.items {
    match item {
      PersistedChatItem::Message(m) => items.push(ChatItem::Message(m)),
      PersistedChatItem::Tool(t) => {
        index.insert(t.id.clone(), items.len());
        items.push(ChatItem::Tool(t));
      }
      PersistedChatItem::Plan(p) => items.push(ChatItem::Plan(p)),
      PersistedChatItem::Thought(t) => items.push(ChatItem::Thought(t)),
      PersistedChatItem::Checkpoint(c) => items.push(ChatItem::Checkpoint(c)),
      PersistedChatItem::Permission(mut p) => {
        // The session that could answer is gone; a pending card must not
        // offer live buttons after a reload.
        if p.resolved.is_none() {
          p.resolved = Some("unanswered".to_string());
        }
        items.push(ChatItem::Permission(p));
      }
      PersistedChatItem::TurnSummary(s) => items.push(ChatItem::TurnSummary(s)),
    }
  }
  let pins = parsed
    .group_pins
    .into_iter()
    .map(|(id, expanded)| (ToolCallId::new(std::sync::Arc::from(id.as_str())), expanded))
    .collect();
  Some((parsed.meta, items, index, pins, parsed.auto_approve))
}

pub(crate) fn list_conversations_in(dir: &std::path::Path) -> Vec<ConversationMeta> {
  let Ok(entries) = std::fs::read_dir(dir) else {
    return Vec::new();
  };
  let mut metas: Vec<ConversationMeta> = entries
    .flatten()
    .filter_map(|entry| {
      let path = entry.path();
      let name = path.file_name()?.to_str()?.to_string();
      if !name.ends_with(".json") {
        return None;
      }
      let raw = std::fs::read_to_string(&path).ok()?;
      let parsed: PersistedConversation = serde_json::from_str(&raw).ok()?;
      Some(parsed.meta)
    })
    .collect();
  metas.sort_by_key(|m| std::cmp::Reverse(m.updated_at_secs));
  metas
}
