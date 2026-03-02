use std::{
  collections::{HashMap, VecDeque},
  sync::{Arc, Mutex},
};

use once_cell::sync::Lazy;

use crate::gfm_markdown_viewer::{ParsedMarkdown, parse_markdown};

pub(crate) const PARSED_MARKDOWN_CACHE_MAX_ENTRIES: usize = 512;
const PARSED_MARKDOWN_CACHE_MAX_SOURCE_LEN: usize = 100_000;

static PARSED_MARKDOWN_CACHE: Lazy<Mutex<ParsedMarkdownCache>> =
  Lazy::new(|| Mutex::new(ParsedMarkdownCache::default()));

#[derive(Default)]
pub(crate) struct ParsedMarkdownCache {
  entries: HashMap<Arc<str>, ParsedMarkdown>,
  lru_keys: VecDeque<Arc<str>>,
}

impl ParsedMarkdownCache {
  pub(crate) fn get(&mut self, source: &str) -> Option<ParsedMarkdown> {
    let parsed = self.entries.get(source).cloned()?;
    self.touch(source);
    Some(parsed)
  }

  pub(crate) fn insert(&mut self, source: Arc<str>, parsed: ParsedMarkdown) {
    if self.entries.contains_key(source.as_ref()) {
      self.touch(source.as_ref());
      return;
    }

    self.entries.insert(source.clone(), parsed);
    self.lru_keys.push_back(source);
    self.evict_excess();
  }

  fn touch(&mut self, source: &str) {
    let Some(ix) = self.lru_keys.iter().position(|key| key.as_ref() == source) else {
      return;
    };
    if let Some(key) = self.lru_keys.remove(ix) {
      self.lru_keys.push_back(key);
    }
  }

  fn evict_excess(&mut self) {
    while self.entries.len() > PARSED_MARKDOWN_CACHE_MAX_ENTRIES {
      let Some(oldest_key) = self.lru_keys.pop_front() else {
        break;
      };
      self.entries.remove(oldest_key.as_ref());
    }
  }

  #[cfg(test)]
  pub(crate) fn len(&self) -> usize {
    self.entries.len()
  }
}

pub(crate) fn parse_markdown_for_render(source: &str) -> ParsedMarkdown {
  if source.len() > PARSED_MARKDOWN_CACHE_MAX_SOURCE_LEN {
    return parse_markdown(source);
  }

  if let Ok(mut cache) = PARSED_MARKDOWN_CACHE.lock()
    && let Some(parsed) = cache.get(source)
  {
    return parsed;
  }

  let parsed = parse_markdown(source);
  let cache_key: Arc<str> = Arc::from(source);

  if let Ok(mut cache) = PARSED_MARKDOWN_CACHE.lock() {
    if let Some(existing) = cache.get(source) {
      return existing;
    }
    cache.insert(cache_key, parsed.clone());
  }

  parsed
}
