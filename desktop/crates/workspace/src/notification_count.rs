use std::sync::{Arc, Mutex};

use gpui::{App, Global};

#[derive(Clone)]
pub struct NotificationCountStore {
  count: Arc<Mutex<usize>>,
}

impl Global for NotificationCountStore {}

impl Default for NotificationCountStore {
  fn default() -> Self {
    Self {
      count: Arc::new(Mutex::new(0)),
    }
  }
}

impl NotificationCountStore {
  pub fn get(cx: &App) -> usize {
    cx.global::<Self>()
      .count
      .lock()
      .map(|c| *c)
      .unwrap_or(0)
  }

  pub fn set(cx: &mut App, count: usize) {
    if let Ok(mut guard) = cx.global::<Self>().count.lock() {
      *guard = count;
    }
  }
}

#[cfg(test)]
mod tests {
  use super::NotificationCountStore;

  #[test]
  fn default_count_is_zero() {
    let store = NotificationCountStore::default();
    assert_eq!(*store.count.lock().unwrap(), 0);
  }
}
