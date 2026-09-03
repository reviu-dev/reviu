//! The persistence seam: the panel hands over snapshots, this entity owns the
//! throttle, the single background write lane, the meta index and async loads.
//! Swapping the backing storage later means reimplementing this entity only.

use std::path::PathBuf;

use gpui::{App, Context, Task};

use crate::PersistedConversation;
use crate::persistence::{
  ConversationMeta, LoadedConversation, WorktreeBinding, list_conversations_in,
  load_conversation_file, read_drafts, read_index, read_scrolls, read_worktrees, write_drafts,
  write_index, write_scrolls, write_worktrees,
};
use app_log::ResultExt;
use std::collections::HashMap;

const SAVE_THROTTLE: std::time::Duration = std::time::Duration::from_millis(400);
const DRAFT_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(250);

pub(crate) struct SaveRequest {
  pub conversation: PersistedConversation,
  /// Set only by the panel currently shown; background sessions must not
  /// steal the active pointer with their streaming saves.
  pub active_id: Option<String>,
}

enum DeferredOp {
  Delete { id: String },
  SetActive { id: Option<String> },
  WriteDrafts(HashMap<String, String>),
  WriteScrolls(HashMap<String, (usize, f32)>),
  WriteWorktrees(HashMap<String, WorktreeBinding>),
}

pub struct ConversationStore {
  dir: PathBuf,
  /// Listing source of truth at runtime; `index.json` is its cross-launch cache.
  metas: Vec<ConversationMeta>,
  /// Mirror of `active.txt`: the conversation to reopen on the next launch.
  active_id: Option<String>,
  pending_save: Option<SaveRequest>,
  pending_ops: Vec<DeferredOp>,
  /// Composer drafts keyed by conversation id, mirrored in `drafts.json`.
  drafts: HashMap<String, String>,
  drafts_dirty: bool,
  draft_debounce: Option<Task<()>>,
  /// Reading positions keyed by conversation id, mirrored in `scroll.json`.
  /// Absent = the conversation was left following the tail.
  scrolls: HashMap<String, (usize, f32)>,
  scrolls_dirty: bool,
  scroll_debounce: Option<Task<()>>,
  /// Worktree bindings keyed by conversation id, mirrored in `worktrees.json`.
  worktrees: HashMap<String, WorktreeBinding>,
  throttle: Option<Task<()>>,
  writer: Option<Task<()>>,
}

impl ConversationStore {
  pub fn new(dir: PathBuf) -> Self {
    let mut metas = read_index(&dir).unwrap_or_else(|| list_conversations_in(&dir));
    metas.sort_by_key(|m| std::cmp::Reverse(m.updated_at_secs));
    let drafts = read_drafts(&dir);
    let scrolls = read_scrolls(&dir);
    let worktrees = read_worktrees(&dir);
    let active_id = std::fs::read_to_string(dir.join("active.txt"))
      .ok()
      .map(|raw| raw.trim().to_string())
      .filter(|id| !id.is_empty());
    Self {
      dir,
      metas,
      active_id,
      pending_save: None,
      pending_ops: Vec::new(),
      drafts,
      drafts_dirty: false,
      draft_debounce: None,
      scrolls,
      scrolls_dirty: false,
      scroll_debounce: None,
      worktrees,
      throttle: None,
      writer: None,
    }
  }

  pub fn worktree(&self, id: &str) -> Option<WorktreeBinding> {
    self.worktrees.get(id).cloned()
  }

  /// Every binding, for the boot-time orphan sweep.
  pub fn worktree_bindings(&self) -> HashMap<String, WorktreeBinding> {
    self.worktrees.clone()
  }

  /// Branch names by conversation id, for the sidebar's worktree rows.
  pub fn worktree_branches(&self) -> HashMap<String, String> {
    self
      .worktrees
      .iter()
      .map(|(id, binding)| (id.clone(), binding.branch.clone()))
      .collect()
  }

  /// Checkout bindings by conversation id, for sidebar checkout rows.
  pub fn worktree_checkouts(&self) -> HashMap<String, WorktreeBinding> {
    self.worktrees.clone()
  }

  /// Bindings change on discrete gestures (create, delete): written right away.
  pub fn set_worktree(
    &mut self,
    id: &str,
    binding: Option<WorktreeBinding>,
    cx: &mut Context<Self>,
  ) {
    let changed = match binding {
      Some(binding) => self.worktrees.insert(id.to_string(), binding.clone()) != Some(binding),
      None => self.worktrees.remove(id).is_some(),
    };
    if !changed {
      return;
    }
    self
      .pending_ops
      .push(DeferredOp::WriteWorktrees(self.worktrees.clone()));
    self.kick_writer(cx);
  }

  pub fn scroll(&self, id: &str) -> Option<(usize, f32)> {
    self.scrolls.get(id).copied()
  }

  /// Debounced like drafts; None marks the conversation as tail-following.
  pub fn set_scroll(&mut self, id: &str, position: Option<(usize, f32)>, cx: &mut Context<Self>) {
    let changed = match position {
      Some(position) => self.scrolls.insert(id.to_string(), position) != Some(position),
      None => self.scrolls.remove(id).is_some(),
    };
    if !changed {
      return;
    }
    self.scrolls_dirty = true;
    if self.scroll_debounce.is_some() {
      return;
    }
    self.scroll_debounce = Some(cx.spawn(async move |this, cx| {
      cx.background_executor().timer(DRAFT_DEBOUNCE).await;
      let _ = this.update(cx, |store, cx| {
        store.scroll_debounce = None;
        store.queue_scroll_write(cx);
      });
    }));
  }

  fn queue_scroll_write(&mut self, cx: &mut Context<Self>) {
    if !self.scrolls_dirty {
      return;
    }
    self.scrolls_dirty = false;
    self
      .pending_ops
      .push(DeferredOp::WriteScrolls(self.scrolls.clone()));
    self.kick_writer(cx);
  }

  pub fn draft(&self, id: &str) -> Option<String> {
    self.drafts.get(id).cloned()
  }

  /// Debounced (250ms): typing updates the map immediately, the file follows.
  pub fn set_draft(&mut self, id: &str, text: &str, cx: &mut Context<Self>) {
    let changed = if text.trim().is_empty() {
      self.drafts.remove(id).is_some()
    } else {
      self.drafts.insert(id.to_string(), text.to_string()) != Some(text.to_string())
    };
    if !changed {
      return;
    }
    self.drafts_dirty = true;
    if self.draft_debounce.is_some() {
      return;
    }
    self.draft_debounce = Some(cx.spawn(async move |this, cx| {
      cx.background_executor().timer(DRAFT_DEBOUNCE).await;
      let _ = this.update(cx, |store, cx| {
        store.draft_debounce = None;
        store.queue_draft_write(cx);
      });
    }));
  }

  fn queue_draft_write(&mut self, cx: &mut Context<Self>) {
    if !self.drafts_dirty {
      return;
    }
    self.drafts_dirty = false;
    self
      .pending_ops
      .push(DeferredOp::WriteDrafts(self.drafts.clone()));
    self.kick_writer(cx);
  }

  pub fn list(&self) -> Vec<ConversationMeta> {
    self.metas.clone()
  }

  /// Coalescing throttle: the first call arms the timer, later snapshots
  /// replace the pending one, one write lands per window.
  pub(crate) fn schedule_save(&mut self, request: SaveRequest, cx: &mut Context<Self>) {
    self.pending_save = Some(request);
    if self.throttle.is_some() {
      return;
    }
    self.throttle = Some(cx.spawn(async move |this, cx| {
      cx.background_executor().timer(SAVE_THROTTLE).await;
      let _ = this.update(cx, |store, cx| {
        store.throttle = None;
        store.kick_writer(cx);
      });
    }));
  }

  /// Boundary save: skips the throttle, still writes off the main thread.
  pub(crate) fn save_now(&mut self, request: SaveRequest, cx: &mut Context<Self>) {
    self.pending_save = Some(request);
    self.throttle = None;
    self.kick_writer(cx);
  }

  pub fn delete(&mut self, id: &str, cx: &mut Context<Self>) {
    if self
      .pending_save
      .as_ref()
      .is_some_and(|req| req.conversation.meta.id == id)
    {
      // A queued save must not resurrect the file after the delete.
      self.pending_save = None;
    }
    if self.drafts.remove(id).is_some() {
      self.drafts_dirty = true;
      self.queue_draft_write(cx);
    }
    if self.scrolls.remove(id).is_some() {
      self.scrolls_dirty = true;
      self.queue_scroll_write(cx);
    }
    if self.worktrees.remove(id).is_some() {
      self
        .pending_ops
        .push(DeferredOp::WriteWorktrees(self.worktrees.clone()));
    }
    self.metas.retain(|m| m.id != id);
    self
      .pending_ops
      .push(DeferredOp::Delete { id: id.to_string() });
    self.kick_writer(cx);
  }

  pub fn set_active(&mut self, id: Option<String>, cx: &mut Context<Self>) {
    self.active_id = id.clone();
    self.pending_ops.push(DeferredOp::SetActive { id });
    self.kick_writer(cx);
  }

  pub fn active_id(&self) -> Option<&str> {
    self.active_id.as_deref()
  }

  pub fn active_meta(&self) -> Option<ConversationMeta> {
    let active_id = self.active_id.as_deref()?;
    self.metas.iter().find(|meta| meta.id == active_id).cloned()
  }

  pub(crate) fn load(&self, id: &str, cx: &App) -> Task<Option<LoadedConversation>> {
    let path = self.dir.join(format!("{id}.json"));
    cx.background_executor()
      .spawn(async move { load_conversation_file(&path) })
  }

  /// Flush queued writes when the app quits; without this a quit mid-stream
  /// loses the last throttle window of transcript.
  pub fn arm_quit_flush(&mut self, cx: &mut Context<Self>) {
    cx.on_app_quit(|store: &mut Self, _| {
      store.flush_on_quit();
      async {}
    })
    .detach();
  }

  /// Quit path: whatever is still queued lands synchronously.
  pub fn flush_on_quit(&mut self) {
    self.throttle = None;
    self.draft_debounce = None;
    self.scroll_debounce = None;
    for op in std::mem::take(&mut self.pending_ops) {
      apply_op(&self.dir, &self.metas, op);
    }
    if let Some(request) = self.pending_save.take() {
      apply_meta(&mut self.metas, &request.conversation.meta);
      write_save(&self.dir, &self.metas, request);
    }
    if std::mem::take(&mut self.drafts_dirty) {
      write_drafts(&self.dir, &self.drafts);
    }
    if std::mem::take(&mut self.scrolls_dirty) {
      write_scrolls(&self.dir, &self.scrolls);
    }
  }

  /// One write lane: ops and saves land in order, one at a time.
  fn kick_writer(&mut self, cx: &mut Context<Self>) {
    if self.writer.is_some() {
      return;
    }
    self.writer = Some(cx.spawn(async move |this, cx| {
      loop {
        let step = this.update(cx, |store, _| {
          if let Some(op) = (!store.pending_ops.is_empty()).then(|| store.pending_ops.remove(0)) {
            return Some(WriterStep::Op(store.dir.clone(), store.metas.clone(), op));
          }
          if let Some(request) = store.pending_save.take() {
            apply_meta(&mut store.metas, &request.conversation.meta);
            return Some(WriterStep::Save(
              store.dir.clone(),
              store.metas.clone(),
              request,
            ));
          }
          store.writer = None;
          None
        });
        match step {
          Ok(Some(WriterStep::Op(dir, metas, op))) => {
            cx.background_executor()
              .spawn(async move { apply_op(&dir, &metas, op) })
              .await;
          }
          Ok(Some(WriterStep::Save(dir, metas, request))) => {
            cx.background_executor()
              .spawn(async move { write_save(&dir, &metas, request) })
              .await;
          }
          Ok(None) | Err(_) => break,
        }
      }
    }));
  }

  #[cfg(test)]
  pub(crate) fn has_pending_save(&self) -> bool {
    self.pending_save.is_some()
  }
}

enum WriterStep {
  Save(PathBuf, Vec<ConversationMeta>, SaveRequest),
  Op(PathBuf, Vec<ConversationMeta>, DeferredOp),
}

fn apply_meta(metas: &mut Vec<ConversationMeta>, meta: &ConversationMeta) {
  match metas.iter_mut().find(|m| m.id == meta.id) {
    Some(entry) => *entry = meta.clone(),
    None => metas.push(meta.clone()),
  }
  metas.sort_by_key(|m| std::cmp::Reverse(m.updated_at_secs));
}

fn write_save(dir: &std::path::Path, metas: &[ConversationMeta], request: SaveRequest) {
  if std::fs::create_dir_all(dir)
    .log_err_context("creating the conversation dir")
    .is_none()
  {
    return;
  }
  let id = request.conversation.meta.id.clone();
  if let Some(json) =
    serde_json::to_string(&request.conversation).log_err_context("serializing the conversation")
  {
    std::fs::write(dir.join(format!("{id}.json")), json)
      .log_err_context("writing the conversation");
  }
  if let Some(active_id) = &request.active_id {
    std::fs::write(dir.join("active.txt"), active_id)
      .log_err_context("writing the active conversation id");
  }
  write_index(dir, metas);
}

fn apply_op(dir: &std::path::Path, metas: &[ConversationMeta], op: DeferredOp) {
  match op {
    DeferredOp::Delete { id } => {
      std::fs::remove_file(dir.join(format!("{id}.json")))
        .log_err_context("deleting the conversation");
      write_index(dir, metas);
    }
    DeferredOp::WriteDrafts(drafts) => write_drafts(dir, &drafts),
    DeferredOp::WriteScrolls(scrolls) => write_scrolls(dir, &scrolls),
    DeferredOp::WriteWorktrees(worktrees) => write_worktrees(dir, &worktrees),
    DeferredOp::SetActive { id } => match id {
      Some(id) => {
        if std::fs::create_dir_all(dir)
          .log_err_context("creating the conversation dir")
          .is_some()
        {
          std::fs::write(dir.join("active.txt"), id)
            .log_err_context("writing the active conversation id");
        }
      }
      None => {
        std::fs::remove_file(dir.join("active.txt"))
          .log_err_context("clearing the active conversation id");
      }
    },
  }
}
