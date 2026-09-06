use std::path::PathBuf;

#[allow(clippy::wildcard_imports)]
use super::*;
use agent_registry::AgentId;
use app_log::ResultExt;

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
  #[serde(default = "default_meta_agent_id")]
  pub agent_id: AgentId,
  #[serde(default)]
  pub session_id: Option<String>,
  /// First line of the last message, for the sidebar row.
  #[serde(default)]
  pub preview: String,
}

fn default_meta_agent_id() -> AgentId {
  default_agent_id()
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
    agent_id: default_meta_agent_id(),
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

pub(crate) fn chat_items_have_persistable_content(items: &[ChatItem], pending_agent: &str) -> bool {
  !pending_agent.trim().is_empty() || items.iter().any(chat_item_has_persistable_content)
}

fn chat_message_has_persistable_content(message: &ChatMessage) -> bool {
  !matches!(message.role, ChatRole::System)
    && (!message.text.trim().is_empty() || message.images > 0 || !message.image_data.is_empty())
}

fn chat_item_has_persistable_content(item: &ChatItem) -> bool {
  match item {
    ChatItem::Message(message) => chat_message_has_persistable_content(message),
    ChatItem::Tool(_) | ChatItem::Permission(_) => true,
    ChatItem::Plan(plan) => !plan.entries.is_empty(),
    ChatItem::Thought(thought) => !thought.text.trim().is_empty(),
    ChatItem::Compaction(compaction) => {
      !compaction.summary.trim().is_empty() || compaction.error.is_some()
    }
    ChatItem::Checkpoint(_) | ChatItem::TurnSummary(_) => false,
  }
}

fn persisted_chat_items_have_persistable_content(items: &[PersistedChatItem]) -> bool {
  items.iter().any(|item| match item {
    PersistedChatItem::Message(message) => chat_message_has_persistable_content(message),
    PersistedChatItem::Tool(_) | PersistedChatItem::Permission(_) => true,
    PersistedChatItem::Plan(plan) => !plan.entries.is_empty(),
    PersistedChatItem::Thought(thought) => !thought.text.trim().is_empty(),
    PersistedChatItem::Compaction(compaction) => {
      !compaction.summary.trim().is_empty() || compaction.error.is_some()
    }
    PersistedChatItem::Checkpoint(_) | PersistedChatItem::TurnSummary(_) => false,
  })
}

fn conversation_file_has_persistable_content(path: &std::path::Path) -> bool {
  std::fs::read_to_string(path)
    .ok()
    .and_then(|raw| serde_json::from_str::<PersistedConversation>(&raw).ok())
    .is_some_and(|conversation| persisted_chat_items_have_persistable_content(&conversation.items))
}

pub(crate) fn conversation_meta_has_listing_content(
  dir: &std::path::Path,
  meta: &ConversationMeta,
) -> bool {
  let title = meta.title.trim();
  !meta.preview.trim().is_empty()
    || (!title.is_empty() && title != "New chat")
    || conversation_file_has_persistable_content(&dir.join(format!("{}.json", meta.id)))
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

/// Per-project directory hosting one file per conversation + an `active.txt` pointer.
pub fn state_dir_for_project(state_dir: &std::path::Path, project: &std::path::Path) -> PathBuf {
  let canonical = project
    .canonicalize()
    .unwrap_or_else(|_| project.to_path_buf());
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
    assert_eq!(m.agent_id, default_agent_id());
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

pub(crate) fn load_conversation_file(path: &std::path::Path) -> Option<LoadedConversation> {
  let raw = std::fs::read_to_string(path).ok()?;
  let parsed: PersistedConversation = serde_json::from_str(&raw).ok()?;
  let mut items = Vec::with_capacity(parsed.items.len());
  let mut index = HashMap::new();
  for item in parsed.items {
    match item {
      PersistedChatItem::Message(m) => items.push(ChatItem::Message(m)),
      PersistedChatItem::Tool(mut t) => {
        populate_syntax_spans(&mut t);
        index.insert(t.id.clone(), items.len());
        items.push(ChatItem::Tool(t));
      }
      PersistedChatItem::Plan(p) => items.push(ChatItem::Plan(p)),
      PersistedChatItem::Thought(t) => items.push(ChatItem::Thought(t)),
      PersistedChatItem::Compaction(c) => items.push(ChatItem::Compaction(c)),
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
  if std::fs::create_dir_all(dir)
    .log_err_context("creating the conversation dir")
    .is_none()
  {
    return;
  }
  if let Some(json) = serde_json::to_string(scrolls).log_err_context("serializing scroll state") {
    std::fs::write(scrolls_path(dir), json).log_err_context("writing scroll state");
  }
}

/// The worktree a conversation's agent works in; absent = the main checkout.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WorktreeBinding {
  pub path: PathBuf,
  pub branch: String,
}

pub(crate) fn worktrees_path(dir: &std::path::Path) -> PathBuf {
  dir.join("worktrees.json")
}

pub(crate) fn read_worktrees(dir: &std::path::Path) -> HashMap<String, WorktreeBinding> {
  std::fs::read_to_string(worktrees_path(dir))
    .ok()
    .and_then(|raw| serde_json::from_str(&raw).ok())
    .unwrap_or_default()
}

pub(crate) fn write_worktrees(dir: &std::path::Path, worktrees: &HashMap<String, WorktreeBinding>) {
  if std::fs::create_dir_all(dir)
    .log_err_context("creating the conversation dir")
    .is_none()
  {
    return;
  }
  if let Some(json) =
    serde_json::to_string(worktrees).log_err_context("serializing worktree bindings")
  {
    std::fs::write(worktrees_path(dir), json).log_err_context("writing worktree bindings");
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
  if std::fs::create_dir_all(dir)
    .log_err_context("creating the conversation dir")
    .is_none()
  {
    return;
  }
  if let Some(json) = serde_json::to_string(drafts).log_err_context("serializing drafts") {
    std::fs::write(drafts_path(dir), json).log_err_context("writing drafts");
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
  if let Some(json) =
    serde_json::to_string(&file).log_err_context("serializing the conversation index")
  {
    std::fs::write(index_path(dir), json).log_err_context("writing the conversation index");
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
      conversation_meta_has_listing_content(dir, &parsed.meta).then_some(parsed.meta)
    })
    .collect();
  metas.sort_by_key(|m| std::cmp::Reverse(m.updated_at_secs));
  metas
}
