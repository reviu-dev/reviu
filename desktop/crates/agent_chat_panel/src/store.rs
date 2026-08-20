//! The persistence seam: the panel hands over snapshots, this entity owns the
//! throttle, the single background write lane, the meta index and async loads.
//! Swapping the backing storage later means reimplementing this entity only.

use std::path::PathBuf;

use gpui::{App, Context, Task};

use crate::PersistedConversation;
use crate::persistence::{
  ConversationMeta, LoadedConversation, list_conversations_in, load_active_conversation,
  load_conversation_file, read_drafts, read_index, write_drafts, write_index,
};
use std::collections::HashMap;

const SAVE_THROTTLE: std::time::Duration = std::time::Duration::from_millis(400);
const DRAFT_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(250);

pub(crate) struct SaveRequest {
  pub conversation: PersistedConversation,
  pub active_id: String,
}

enum DeferredOp {
  Delete { id: String },
  SetActive { id: Option<String> },
  WriteDrafts(HashMap<String, String>),
}

pub(crate) struct ConversationStore {
  dir: PathBuf,
  /// Listing source of truth at runtime; `index.json` is its cross-launch cache.
  metas: Vec<ConversationMeta>,
  pending_save: Option<SaveRequest>,
  pending_ops: Vec<DeferredOp>,
  /// Composer drafts keyed by conversation id, mirrored in `drafts.json`.
  drafts: HashMap<String, String>,
  drafts_dirty: bool,
  draft_debounce: Option<Task<()>>,
  throttle: Option<Task<()>>,
  writer: Option<Task<()>>,
}

impl ConversationStore {
  pub fn new(dir: PathBuf) -> Self {
    let mut metas = read_index(&dir).unwrap_or_else(|| list_conversations_in(&dir));
    metas.sort_by_key(|m| std::cmp::Reverse(m.updated_at_secs));
    let drafts = read_drafts(&dir);
    Self {
      dir,
      metas,
      pending_save: None,
      pending_ops: Vec::new(),
      drafts,
      drafts_dirty: false,
      draft_debounce: None,
      throttle: None,
      writer: None,
    }
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
  pub fn schedule_save(&mut self, request: SaveRequest, cx: &mut Context<Self>) {
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
  pub fn save_now(&mut self, request: SaveRequest, cx: &mut Context<Self>) {
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
    self.metas.retain(|m| m.id != id);
    self
      .pending_ops
      .push(DeferredOp::Delete { id: id.to_string() });
    self.kick_writer(cx);
  }

  pub fn set_active(&mut self, id: Option<String>, cx: &mut Context<Self>) {
    self.pending_ops.push(DeferredOp::SetActive { id });
    self.kick_writer(cx);
  }

  pub fn load(&self, id: &str, cx: &App) -> Task<Option<LoadedConversation>> {
    let path = self.dir.join(format!("{id}.json"));
    cx.background_executor()
      .spawn(async move { load_conversation_file(&path) })
  }

  pub fn load_active(&self, cx: &App) -> Task<Option<LoadedConversation>> {
    let dir = self.dir.clone();
    cx.background_executor()
      .spawn(async move { load_active_conversation(&dir) })
  }

  /// Quit path: whatever is still queued lands synchronously.
  pub fn flush_on_quit(&mut self) {
    self.throttle = None;
    self.draft_debounce = None;
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
  let _ = std::fs::create_dir_all(dir);
  let id = request.conversation.meta.id.clone();
  if let Ok(json) = serde_json::to_string(&request.conversation) {
    let _ = std::fs::write(dir.join(format!("{id}.json")), json);
  }
  let _ = std::fs::write(dir.join("active.txt"), &request.active_id);
  write_index(dir, metas);
}

fn apply_op(dir: &std::path::Path, metas: &[ConversationMeta], op: DeferredOp) {
  match op {
    DeferredOp::Delete { id } => {
      let _ = std::fs::remove_file(dir.join(format!("{id}.json")));
      write_index(dir, metas);
    }
    DeferredOp::WriteDrafts(drafts) => write_drafts(dir, &drafts),
    DeferredOp::SetActive { id } => match id {
      Some(id) => {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(dir.join("active.txt"), id);
      }
      None => {
        let _ = std::fs::remove_file(dir.join("active.txt"));
      }
    },
  }
}
