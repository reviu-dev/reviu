//! The one place that reads across per-repo conversation stores. Everything
//! multi-repo goes through here, so a future storage backend (#587) swaps
//! inside this facade without touching the shell.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use agent_chat_panel::{AgentChatPanel, ConversationMeta, ConversationStore};
use gpui::{App, AppContext as _, Entity};

use crate::agent_chat_state::agent_chat_state_dir;
use crate::config::ConfigStore;

/// Bounds the aggregation: recents beyond this never get a store, which is
/// what keeps the per-folder index model (#583) sufficient and #587 closed.
pub(crate) const MAX_TRACKED_REPOS: usize = 8;

pub(crate) struct ConversationHub {
  /// Keyed by canonicalized repo root; insertion order is meaningless.
  stores: Vec<(PathBuf, Entity<ConversationStore>)>,
}

pub(crate) struct ConversationStoreAccess {
  pub store: Entity<ConversationStore>,
  pub evicted_repo: Option<PathBuf>,
}

fn canonical(path: &Path) -> PathBuf {
  std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

impl ConversationHub {
  pub fn new() -> Self {
    Self { stores: Vec::new() }
  }

  /// The store for a repo, created on first sight.
  pub fn store_for(
    &mut self,
    repo_root: &Path,
    protected_repos: &HashSet<PathBuf>,
    cx: &mut App,
  ) -> Option<ConversationStoreAccess> {
    let key = canonical(repo_root);
    if let Some((_, store)) = self.stores.iter().find(|(existing, _)| *existing == key) {
      return Some(ConversationStoreAccess {
        store: store.clone(),
        evicted_repo: None,
      });
    }
    let mut evicted_repo = None;
    if self.stores.len() >= MAX_TRACKED_REPOS
      && let Some(evicted_index) = self.eviction_candidate(protected_repos)
    {
      // Make room instead of silently handing out no store (sessions created
      // without one would never persist). Evicting only drops the repo from
      // the aggregation: live panels hold their own handle to its store.
      let evicted = self.stores.remove(evicted_index);
      evicted.1.update(cx, |store, _| store.flush_on_quit());
      evicted_repo = Some(evicted.0);
    }
    let state_dir =
      agent_chat_state_dir().map(|dir| AgentChatPanel::state_dir_for_repo(&dir, repo_root))?;
    let store = cx.new(|_| ConversationStore::new(state_dir));
    store.update(cx, |store, cx| store.arm_quit_flush(cx));
    self.stores.push((key, store.clone()));
    Some(ConversationStoreAccess {
      store,
      evicted_repo,
    })
  }

  fn eviction_candidate(&self, protected_repos: &HashSet<PathBuf>) -> Option<usize> {
    let recent_positions: HashMap<PathBuf, usize> = ConfigStore::load_recent_repositories()
      .into_iter()
      .enumerate()
      .map(|(index, repo)| (canonical(&repo.path), index))
      .collect();

    self
      .stores
      .iter()
      .enumerate()
      .filter(|(_, (repo, _))| !protected_repos.contains(repo))
      .max_by_key(|(_, (repo, _))| recent_positions.get(repo).copied().unwrap_or(usize::MAX))
      .map(|(index, _)| index)
  }

  pub fn tracked_repositories(&self) -> Vec<PathBuf> {
    self.stores.iter().map(|(repo, _)| repo.clone()).collect()
  }

  pub fn tracked_len(&self) -> usize {
    self.stores.len()
  }

  pub fn reorder(&mut self, ordered_repos: &[PathBuf]) {
    let positions: HashMap<PathBuf, usize> = ordered_repos
      .iter()
      .enumerate()
      .map(|(index, repo)| (canonical(repo), index))
      .collect();
    self
      .stores
      .sort_by_key(|(repo, _)| positions.get(repo).copied().unwrap_or(usize::MAX));
  }

  /// Every tracked repo's conversations, grouped by repo in a STABLE order
  /// (tracking order, never resorted) and newest-created first inside each
  /// group: rows must not dance while sessions stream. Conversation ids are
  /// unique across repos by construction (millis + pid + counter).
  pub fn sections(&self, cx: &App) -> Vec<(PathBuf, Vec<ConversationMeta>)> {
    self
      .stores
      .iter()
      .map(|(repo, store)| {
        let mut metas = store.read(cx).list();
        metas.sort_by_key(|meta| std::cmp::Reverse(meta.started_at_secs));
        (repo.clone(), metas)
      })
      .collect()
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
