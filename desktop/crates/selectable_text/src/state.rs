use std::{
  ops::Range,
  sync::{Arc, Mutex, MutexGuard},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SelectionMode {
  #[default]
  Character,
  Word,
  Line,
}

#[derive(Clone, Debug)]
pub struct ActiveSelection {
  pub text_id: u64,
  pub anchor: usize,
  pub head: usize,
  pub dragging: bool,
  pub mode: SelectionMode,
  pub anchor_word: Option<Range<usize>>,
}

#[derive(Clone, Default)]
pub struct SelectionRegistry(Arc<Mutex<Option<ActiveSelection>>>);

impl SelectionRegistry {
  pub fn new() -> Self {
    Self::default()
  }

  // A panic while the lock is held must not poison every later selection.
  fn lock(&self) -> MutexGuard<'_, Option<ActiveSelection>> {
    self
      .0
      .lock()
      .unwrap_or_else(|poisoned| poisoned.into_inner())
  }

  pub fn clear(&self) {
    *self.lock() = None;
  }

  pub fn active_for(&self, text_id: u64) -> Option<ActiveSelection> {
    let guard = self.lock();
    match guard.as_ref() {
      Some(active) if active.text_id == text_id => Some(active.clone()),
      _ => None,
    }
  }

  pub fn set(
    &self,
    text_id: u64,
    anchor: usize,
    head: usize,
    dragging: bool,
    mode: SelectionMode,
    anchor_word: Option<Range<usize>>,
  ) {
    *self.lock() = Some(ActiveSelection {
      text_id,
      anchor,
      head,
      dragging,
      mode,
      anchor_word,
    });
  }

  #[allow(dead_code)]
  pub(crate) fn clear_if(&self, text_id: u64) {
    let mut guard = self.lock();
    if guard.as_ref().is_some_and(|a| a.text_id == text_id) {
      *guard = None;
    }
  }
}

pub fn normalize_range(start: usize, end: usize) -> Range<usize> {
  if start <= end { start..end } else { end..start }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn registry_records_active_selection_for_text() {
    let registry = SelectionRegistry::new();
    registry.set(1, 4, 9, true, SelectionMode::Character, None);

    let active = registry.active_for(1).expect("active selection");
    assert_eq!(active.anchor, 4);
    assert_eq!(active.head, 9);
    assert!(active.dragging);
    assert_eq!(active.mode, SelectionMode::Character);
  }

  #[test]
  fn registry_returns_none_for_other_text_id() {
    let registry = SelectionRegistry::new();
    registry.set(1, 0, 3, false, SelectionMode::Character, None);
    assert!(registry.active_for(2).is_none());
  }

  #[test]
  fn registry_survives_a_panic_holding_the_lock() {
    let registry = SelectionRegistry::new();
    registry.set(1, 0, 5, true, SelectionMode::Character, None);

    let poisoner = registry.clone();
    let panicked = std::thread::spawn(move || {
      let _guard = poisoner.lock();
      panic!("poison the selection lock");
    })
    .join();
    assert!(panicked.is_err());

    assert_eq!(registry.active_for(1).expect("active selection").head, 5);
    registry.clear();
    assert!(registry.active_for(1).is_none());
  }

  #[test]
  fn registry_set_overwrites_previous_selection() {
    let registry = SelectionRegistry::new();
    registry.set(1, 0, 5, true, SelectionMode::Character, None);
    registry.set(2, 0, 8, true, SelectionMode::Character, None);
    assert!(registry.active_for(1).is_none());
    assert_eq!(registry.active_for(2).unwrap().head, 8);
  }

  #[test]
  fn registry_clear_removes_active() {
    let registry = SelectionRegistry::new();
    registry.set(1, 0, 5, true, SelectionMode::Character, None);
    registry.clear();
    assert!(registry.active_for(1).is_none());
  }

  #[test]
  fn registry_clear_if_only_clears_matching_id() {
    let registry = SelectionRegistry::new();
    registry.set(1, 0, 5, true, SelectionMode::Character, None);
    registry.clear_if(2);
    assert!(registry.active_for(1).is_some());
    registry.clear_if(1);
    assert!(registry.active_for(1).is_none());
  }

  #[test]
  fn normalize_range_orders_endpoints() {
    assert_eq!(normalize_range(2, 5), 2..5);
    assert_eq!(normalize_range(5, 2), 2..5);
    assert_eq!(normalize_range(3, 3), 3..3);
  }
}
