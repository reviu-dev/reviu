use std::path::PathBuf;

#[allow(clippy::wildcard_imports)]
use super::*;

/// Bump when the on-disk shape changes; readers dispatch on it. Version 0 is
/// the tag-less legacy format.
pub(crate) const CONVERSATION_FORMAT_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ConversationMeta {
  pub id: String,
  pub started_at_secs: u64,
  pub updated_at_secs: u64,
  pub title: String,
  pub message_count: usize,
  #[serde(default)]
  pub session_id: Option<String>,
  /// First line of the last message, for the sidebar row.
  #[serde(default)]
  pub preview: String,
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
    preview: String::new(),
  }
}

/// First line of the last message, truncated for a sidebar row.
pub(crate) fn preview_of(items: &[ChatItem]) -> String {
  items
    .iter()
    .rev()
    .find_map(|item| match item {
      ChatItem::Message(m) if !matches!(m.role, ChatRole::System) => Some(truncate_title(&m.text)),
      _ => None,
    })
    .unwrap_or_default()
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
      PersistedChatItem::Tool(mut t) => {
        let default_start_line = t.locations.first().and_then(|(_, line)| *line);
        for diff in &mut t.diffs {
          let start_line = t
            .locations
            .iter()
            .find(|(path, _)| location_matches_diff_path(path, &diff.path))
            .and_then(|(_, line)| *line)
            .or(default_start_line);
          backfill_legacy_line_numbers(diff, start_line);
        }
        populate_syntax_spans(&mut t);
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

fn location_matches_diff_path(location: &std::path::Path, diff_path: &str) -> bool {
  let location = location.to_string_lossy();
  location == diff_path || location.ends_with(diff_path)
}

/// Listing only needs `meta`; skipping `items` avoids materializing whole
/// transcripts just to fill the sidebar.
#[derive(serde::Deserialize)]
struct PersistedConversationMetaOnly {
  meta: ConversationMeta,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct IndexFile {
  #[serde(default)]
  version: u32,
  conversations: Vec<ConversationMeta>,
}

pub(crate) fn index_path(dir: &std::path::Path) -> PathBuf {
  dir.join("index.json")
}

pub(crate) fn scrolls_path(dir: &std::path::Path) -> PathBuf {
  dir.join("scroll.json")
}

pub(crate) fn read_scrolls(dir: &std::path::Path) -> HashMap<String, (usize, f32)> {
  std::fs::read_to_string(scrolls_path(dir))
    .ok()
    .and_then(|raw| serde_json::from_str(&raw).ok())
    .unwrap_or_default()
}

pub(crate) fn write_scrolls(dir: &std::path::Path, scrolls: &HashMap<String, (usize, f32)>) {
  let _ = std::fs::create_dir_all(dir);
  if let Ok(json) = serde_json::to_string(scrolls) {
    let _ = std::fs::write(scrolls_path(dir), json);
  }
}

pub(crate) fn drafts_path(dir: &std::path::Path) -> PathBuf {
  dir.join("drafts.json")
}

pub(crate) fn read_drafts(dir: &std::path::Path) -> HashMap<String, String> {
  std::fs::read_to_string(drafts_path(dir))
    .ok()
    .and_then(|raw| serde_json::from_str(&raw).ok())
    .unwrap_or_default()
}

pub(crate) fn write_drafts(dir: &std::path::Path, drafts: &HashMap<String, String>) {
  let _ = std::fs::create_dir_all(dir);
  if let Ok(json) = serde_json::to_string(drafts) {
    let _ = std::fs::write(drafts_path(dir), json);
  }
}

pub(crate) fn read_index(dir: &std::path::Path) -> Option<Vec<ConversationMeta>> {
  let raw = std::fs::read_to_string(index_path(dir)).ok()?;
  let parsed: IndexFile = serde_json::from_str(&raw).ok()?;
  Some(parsed.conversations)
}

pub(crate) fn write_index(dir: &std::path::Path, conversations: &[ConversationMeta]) {
  let file = IndexFile {
    version: CONVERSATION_FORMAT_VERSION,
    conversations: conversations.to_vec(),
  };
  if let Ok(json) = serde_json::to_string(&file) {
    let _ = std::fs::write(index_path(dir), json);
  }
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
      let parsed: PersistedConversationMetaOnly = serde_json::from_str(&raw).ok()?;
      Some(parsed.meta)
    })
    .collect();
  metas.sort_by_key(|m| std::cmp::Reverse(m.updated_at_secs));
  metas
}
