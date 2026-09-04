//! The one place that reads across per-project conversation stores. Everything
//! multi-project goes through here, so a future storage backend (#587) swaps
//! inside this facade without touching the shell.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use agent_chat_panel::{AgentChatPanel, ConversationMeta, ConversationStore};
use gpui::{App, AppContext as _, Entity};

use crate::agent_chat_state::agent_chat_state_dir;
use crate::config::ConfigStore;

/// Bounds the aggregation: recents beyond this never get a store, which is
/// what keeps the per-folder index model (#583) sufficient and #587 closed.
pub(crate) const MAX_TRACKED_PROJECTS: usize = 8;

pub(crate) struct ConversationHub {
  /// Keyed by canonicalized project root; insertion order is meaningless.
  stores: Vec<(PathBuf, Entity<ConversationStore>)>,
}

pub(crate) struct ConversationStoreAccess {
  pub store: Entity<ConversationStore>,
  pub evicted_project: Option<PathBuf>,
}

fn canonical(path: &Path) -> PathBuf {
  std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

impl ConversationHub {
  pub fn new() -> Self {
    Self { stores: Vec::new() }
  }

  /// The store for a project, created on first sight.
  pub fn store_for_project(
    &mut self,
    project_root: &Path,
    protected_projects: &HashSet<PathBuf>,
    cx: &mut App,
  ) -> Option<ConversationStoreAccess> {
    let key = canonical(project_root);
    if let Some((_, store)) = self.stores.iter().find(|(existing, _)| *existing == key) {
      return Some(ConversationStoreAccess {
        store: store.clone(),
        evicted_project: None,
      });
    }
    let mut evicted_project = None;
    if self.stores.len() >= MAX_TRACKED_PROJECTS
      && let Some(evicted_index) = self.eviction_candidate(protected_projects)
    {
      // Make room instead of silently handing out no store (sessions created
      // without one would never persist). Evicting only drops the project from
      // the aggregation: live panels hold their own handle to its store.
      let evicted = self.stores.remove(evicted_index);
      evicted.1.update(cx, |store, _| store.flush_on_quit());
      evicted_project = Some(evicted.0);
    }
    let state_dir =
      agent_chat_state_dir().map(|dir| AgentChatPanel::state_dir_for_repo(&dir, project_root))?;
    let store = cx.new(|_| ConversationStore::new(state_dir));
    store.update(cx, |store, cx| store.arm_quit_flush(cx));
    self.stores.push((key, store.clone()));
    Some(ConversationStoreAccess {
      store,
      evicted_project,
    })
  }

  fn eviction_candidate(&self, protected_projects: &HashSet<PathBuf>) -> Option<usize> {
    let mut recent_positions: HashMap<PathBuf, usize> = ConfigStore::load_recent_repositories()
      .into_iter()
      .enumerate()
      .map(|(index, repo)| (canonical(&repo.path), index))
      .collect();
    let offset = recent_positions.len();
    recent_positions.extend(
      ConfigStore::load_recent_projects()
        .into_iter()
        .enumerate()
        .map(|(index, project)| (canonical(&project.path), offset + index)),
    );

    self
      .stores
      .iter()
      .enumerate()
      .filter(|(_, (project, _))| !protected_projects.contains(project))
      .max_by_key(|(_, (project, _))| recent_positions.get(project).copied().unwrap_or(usize::MAX))
      .map(|(index, _)| index)
  }

  pub fn tracked_projects(&self) -> Vec<PathBuf> {
    self
      .stores
      .iter()
      .map(|(project, _)| project.clone())
      .collect()
  }

  pub fn tracked_project_len(&self) -> usize {
    self.stores.len()
  }

  pub fn reorder(&mut self, ordered_projects: &[PathBuf]) {
    let positions: HashMap<PathBuf, usize> = ordered_projects
      .iter()
      .enumerate()
      .map(|(index, project)| (canonical(project), index))
      .collect();
    self
      .stores
      .sort_by_key(|(project, _)| positions.get(project).copied().unwrap_or(usize::MAX));
  }

  /// Every tracked project's conversations, grouped in a STABLE order
  /// (tracking order, never resorted) and newest-created first inside each
  /// group: rows must not dance while sessions stream. Conversation ids are
  /// unique across projects by construction (millis + pid + counter).
  pub fn project_sections(&self, cx: &App) -> Vec<(PathBuf, Vec<ConversationMeta>)> {
    self
      .stores
      .iter()
      .map(|(project, store)| {
        let mut metas = store.read(cx).list();
        metas.sort_by_key(|meta| std::cmp::Reverse(meta.started_at_secs));
        (project.clone(), metas)
      })
      .collect()
  }

  pub fn find_conversation(
    &self,
    conversation_id: &str,
    cx: &App,
  ) -> Option<(PathBuf, Entity<ConversationStore>, ConversationMeta)> {
    self.stores.iter().find_map(|(project, store)| {
      store
        .read(cx)
        .list()
        .into_iter()
        .find(|meta| meta.id == conversation_id)
        .map(|meta| (project.clone(), store.clone(), meta))
    })
  }

  /// Worktree checkout bindings by conversation id, merged across projects.
  pub fn worktree_checkouts(&self, cx: &App) -> HashMap<String, agent_chat_panel::WorktreeBinding> {
    self
      .stores
      .iter()
      .flat_map(|(_, store)| store.read(cx).worktree_checkouts())
      .collect()
  }

  /// Stops tracking a project: queued writes land, the store goes away. The
  /// files on disk stay.
  pub fn drop_project(&mut self, project_root: &Path, cx: &mut App) {
    let key = canonical(project_root);
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
