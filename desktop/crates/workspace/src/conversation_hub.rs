//! The one place that reads across per-repo conversation stores. Everything
//! multi-repo goes through here, so a future storage backend (#587) swaps
//! inside this facade without touching the shell.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use agent_chat_panel::{AgentChatPanel, ConversationMeta, ConversationStore};
use gpui::{App, AppContext as _, Entity};

use crate::agent_chat_state::agent_chat_state_dir;

/// Bounds the aggregation: recents beyond this never get a store, which is
/// what keeps the per-folder index model (#583) sufficient and #587 closed.
pub(crate) const MAX_TRACKED_REPOS: usize = 8;

pub(crate) struct ConversationHub {
  /// Keyed by canonicalized repo root; insertion order is meaningless.
  stores: Vec<(PathBuf, Entity<ConversationStore>)>,
}

fn canonical(path: &Path) -> PathBuf {
  std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

impl ConversationHub {
  pub fn new() -> Self {
    Self { stores: Vec::new() }
  }

  /// The store for a repo, created on first sight. `true` = newly created,
  /// the caller's cue for one-time housekeeping (orphan sweep).
  pub fn store_for(
    &mut self,
    repo_root: &Path,
    cx: &mut App,
  ) -> Option<(Entity<ConversationStore>, bool)> {
    let key = canonical(repo_root);
    if let Some((_, store)) = self.stores.iter().find(|(existing, _)| *existing == key) {
      return Some((store.clone(), false));
    }
    if self.stores.len() >= MAX_TRACKED_REPOS {
      // Make room instead of silently handing out no store (sessions created
      // without one would never persist). Evicting only drops the repo from
      // the aggregation: live panels hold their own handle to its store.
      let evicted = self.stores.remove(0);
      evicted.1.update(cx, |store, _| store.flush_on_quit());
    }
    let state_dir =
      agent_chat_state_dir().map(|dir| AgentChatPanel::state_dir_for_repo(&dir, repo_root))?;
    let store = cx.new(|_| ConversationStore::new(state_dir));
    store.update(cx, |store, cx| store.arm_quit_flush(cx));
    self.stores.push((key, store.clone()));
    Some((store, true))
  }

  /// Every conversation of every tracked repo (or one repo when scoped),
  /// newest first. Conversation ids are unique across repos by construction
  /// (millis + pid + counter).
  pub fn rows(&self, scope: Option<&Path>, cx: &App) -> Vec<(PathBuf, ConversationMeta)> {
    let scope = scope.map(canonical);
    let mut rows: Vec<(PathBuf, ConversationMeta)> = self
      .stores
      .iter()
      .filter(|(repo, _)| scope.as_deref().is_none_or(|scope| scope == repo))
      .flat_map(|(repo, store)| {
        store
          .read(cx)
          .list()
          .into_iter()
          .map(|meta| (repo.clone(), meta))
      })
      .collect();
    rows.sort_by_key(|(_, meta)| std::cmp::Reverse(meta.updated_at_secs));
    rows
  }

  pub fn find_conversation(
    &self,
    conversation_id: &str,
    cx: &App,
  ) -> Option<(PathBuf, Entity<ConversationStore>, ConversationMeta)> {
    self.stores.iter().find_map(|(repo, store)| {
      store
        .read(cx)
        .list()
        .into_iter()
        .find(|meta| meta.id == conversation_id)
        .map(|meta| (repo.clone(), store.clone(), meta))
    })
  }

  /// Worktree branch by conversation id, merged across repos.
  pub fn worktree_branches(&self, cx: &App) -> HashMap<String, String> {
    self
      .stores
      .iter()
      .flat_map(|(_, store)| store.read(cx).worktree_branches())
      .collect()
  }

  /// Stops tracking a repo: queued writes land, the store goes away. The
  /// files on disk stay.
  pub fn drop_store(&mut self, repo_root: &Path, cx: &mut App) {
    let key = canonical(repo_root);
    self.stores.retain(|(existing, store)| {
      if *existing == key {
        store.update(cx, |store, _| store.flush_on_quit());
        false
      } else {
        true
      }
    });
  }
}
