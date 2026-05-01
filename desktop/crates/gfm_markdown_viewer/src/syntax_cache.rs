use std::{
  collections::{HashMap, HashSet},
  hash::{DefaultHasher, Hash, Hasher},
  sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use gpui::SharedString;
use syntax::{SyntaxHighlighter, languages};

use crate::types::{CodeBlock, InlineSpan, InlineStyle, LinkRange};

pub(crate) type CachedSpans = (SharedString, Vec<InlineSpan>, Vec<LinkRange>);

/// A thread-safe cache for syntax-highlighted code block spans.
///
/// On cache miss, returns plain (uncolored) spans immediately and
/// schedules tree-sitter highlighting on a background thread.
/// When the background work completes, `has_new_highlights()` returns true
/// so the caller can trigger a re-render.
pub struct SyntaxHighlightCache {
  entries: Mutex<HashMap<u64, CachedSpans>>,
  pending: Mutex<HashSet<u64>>,
  new_highlights: Mutex<bool>,
}

/// Lock recovering from poisoning. A poisoned lock means a previous holder
/// panicked while mutating; for this cache the data is safe to keep using,
/// and silently skipping the lock would leak `pending` keys forever and pin
/// the render loop to 60 FPS.
fn lock_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
  mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Drop guard that always removes a key from `pending`, even if the
/// background thread panics. Without this, a single panic in tree-sitter
/// (or in span construction) leaves `has_pending()` permanently true and
/// forces the consumer's render loop to poll forever.
struct PendingGuard {
  cache: Arc<SyntaxHighlightCache>,
  key: u64,
}

impl Drop for PendingGuard {
  fn drop(&mut self) {
    lock_recover(&self.cache.pending).remove(&self.key);
  }
}

impl SyntaxHighlightCache {
  pub fn new() -> Self {
    Self {
      entries: Mutex::new(HashMap::new()),
      pending: Mutex::new(HashSet::new()),
      new_highlights: Mutex::new(false),
    }
  }

  /// Look up cached highlight spans for a code block.
  pub(crate) fn get(&self, code: &CodeBlock) -> Option<CachedSpans> {
    let key = Self::cache_key(code);
    lock_recover(&self.entries).get(&key).cloned()
  }

  /// Returns true if there are background highlight tasks still in progress.
  pub fn has_pending(&self) -> bool {
    !lock_recover(&self.pending).is_empty()
  }

  /// Check if there are new highlights available since last check.
  /// Resets the flag after reading.
  pub fn take_new_highlights(&self) -> bool {
    let mut flag = lock_recover(&self.new_highlights);
    let had = *flag;
    *flag = false;
    had
  }

  /// Schedule async syntax highlighting for a code block.
  /// Returns plain spans immediately. The highlight runs on a background thread.
  /// When complete, the result is inserted into the cache and `has_new_highlights` is set.
  pub(crate) fn schedule_highlight(
    self: &Arc<Self>,
    code: &CodeBlock,
    display_value: &str,
  ) {
    let key = Self::cache_key(code);

    if !lock_recover(&self.pending).insert(key) {
      return;
    }

    let guard = PendingGuard {
      cache: self.clone(),
      key,
    };
    let lang = code.lang.clone();
    let display_value = display_value.to_string();

    std::thread::spawn(move || {
      let _guard = guard;

      let base_style = InlineStyle {
        code: true,
        ..InlineStyle::default()
      };

      let spans = code_block_language_config(lang.as_deref())
        .and_then(|config| {
          let mut highlighter = SyntaxHighlighter::new(config);
          highlighter
            .highlight_text(display_value.as_ref())
            .ok()
            .map(|highlights| {
              crate::gfm_markdown_viewer::syntax_highlight_spans_for_code(
                display_value.as_ref(),
                &highlights,
                base_style,
              )
            })
        })
        .filter(|spans| !spans.is_empty());

      if let Some(spans) = spans {
        let cache = &_guard.cache;
        let text = SharedString::from(display_value);
        let result = (text, spans, Vec::new());
        lock_recover(&cache.entries).insert(key, result);
        *lock_recover(&cache.new_highlights) = true;
      }
    });
  }

  fn cache_key(code: &CodeBlock) -> u64 {
    let mut hasher = DefaultHasher::new();
    code.lang.hash(&mut hasher);
    code.value.hash(&mut hasher);
    hasher.finish()
  }
}

fn code_block_language_config(lang: Option<&str>) -> Option<&'static syntax::LanguageConfig> {
  let lang = lang?
    .trim()
    .trim_matches(|c| c == '{' || c == '}')
    .trim_start_matches('.');
  if lang.is_empty() {
    return None;
  }
  languages::language_config_for_name(lang).or_else(|| languages::detect_language_config(lang))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn pending_guard_clears_key_on_panic() {
    let cache = Arc::new(SyntaxHighlightCache::new());
    let key = 42u64;
    lock_recover(&cache.pending).insert(key);
    assert!(cache.has_pending());

    let guard = PendingGuard {
      cache: cache.clone(),
      key,
    };
    let handle = std::thread::spawn(move || {
      let _guard = guard;
      std::panic::panic_any(());
    });
    let _ = handle.join();

    assert!(
      !cache.has_pending(),
      "pending must drain on panic to avoid pinning the render loop at 60 FPS"
    );
  }
}
