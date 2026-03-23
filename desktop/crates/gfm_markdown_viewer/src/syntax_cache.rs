use std::{
  collections::{HashMap, HashSet},
  hash::{DefaultHasher, Hash, Hasher},
  sync::Mutex,
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
    self.entries.lock().ok()?.get(&key).cloned()
  }

  /// Returns true if there are background highlight tasks still in progress.
  pub fn has_pending(&self) -> bool {
    self.pending.lock().ok().map_or(false, |p| !p.is_empty())
  }

  /// Check if there are new highlights available since last check.
  /// Resets the flag after reading.
  pub fn take_new_highlights(&self) -> bool {
    self.new_highlights.lock().ok().map_or(false, |mut flag| {
      let had = *flag;
      *flag = false;
      had
    })
  }

  /// Schedule async syntax highlighting for a code block.
  /// Returns plain spans immediately. The highlight runs on a background thread.
  /// When complete, the result is inserted into the cache and `has_new_highlights` is set.
  pub(crate) fn schedule_highlight(
    self: &std::sync::Arc<Self>,
    code: &CodeBlock,
    display_value: &str,
  ) {
    let key = Self::cache_key(code);

    // Don't schedule if already pending or cached
    if let Ok(mut pending) = self.pending.lock() {
      if !pending.insert(key) {
        return;
      }
    }

    let cache = self.clone();
    let lang = code.lang.clone();
    let display_value = display_value.to_string();

    std::thread::spawn(move || {
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
        let text = SharedString::from(display_value);
        let result = (text, spans, Vec::new());
        if let Ok(mut entries) = cache.entries.lock() {
          entries.insert(key, result);
        }
        if let Ok(mut flag) = cache.new_highlights.lock() {
          *flag = true;
        }
      }

      // Remove from pending
      if let Ok(mut pending) = cache.pending.lock() {
        pending.remove(&key);
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
