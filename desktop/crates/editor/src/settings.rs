use std::sync::atomic::{AtomicBool, Ordering};

static INDENT_RAINBOW_ENABLED: AtomicBool = AtomicBool::new(false);

pub fn indent_rainbow_enabled() -> bool {
  INDENT_RAINBOW_ENABLED.load(Ordering::Relaxed)
}

pub fn set_indent_rainbow_enabled(enabled: bool) {
  INDENT_RAINBOW_ENABLED.store(enabled, Ordering::Relaxed);
}
