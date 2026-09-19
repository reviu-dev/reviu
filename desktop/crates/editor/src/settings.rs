use std::sync::atomic::{AtomicBool, Ordering};

use gpui::{App, Global};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct EditorSettings {
  pub soft_wrap: bool,
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
