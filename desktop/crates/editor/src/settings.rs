use std::sync::atomic::{AtomicBool, Ordering};

use gpui::{App, Global};

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct EditorSettings {
  pub soft_wrap: bool,
  /// Marks the lines a plain file view changed since the last commit.
  pub git_gutter: bool,
}

impl Default for EditorSettings {
  fn default() -> Self {
    Self {
      soft_wrap: false,
      git_gutter: true,
    }
  }
}

impl Global for EditorSettings {}

impl EditorSettings {
  pub fn get(cx: &App) -> Self {
    cx.try_global::<Self>().copied().unwrap_or_default()
  }
}

static INDENT_RAINBOW_ENABLED: AtomicBool = AtomicBool::new(false);

pub fn indent_rainbow_enabled() -> bool {
  INDENT_RAINBOW_ENABLED.load(Ordering::Relaxed)
}

pub fn set_indent_rainbow_enabled(enabled: bool) {
  INDENT_RAINBOW_ENABLED.store(enabled, Ordering::Relaxed);
}
