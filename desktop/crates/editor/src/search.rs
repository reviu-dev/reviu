use std::ops::Range;

use gpui::Global;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SearchDirection {
  Next,
  Previous,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchOptions {
  pub case_sensitive: bool,
  pub whole_word: bool,
  pub regex: bool,
}

impl Global for SearchOptions {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SearchMatch {
  pub display_line: usize,
  pub column_start: usize,
  pub column_end: usize,
  pub doc_range: Range<usize>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SearchState {
  query: String,
  options: SearchOptions,
  matches: Vec<SearchMatch>,
  active_match: Option<usize>,
  error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SearchHighlights {
  pub matches: Vec<SearchMatch>,
  pub active_match: Option<usize>,
  pub active_range: Option<Range<usize>>,
}

pub(crate) struct SearchMatcher {
  regex: regex::Regex,
  options: SearchOptions,
}

impl SearchMatcher {
  pub fn new(query: &str, options: SearchOptions) -> Result<Self, String> {
    let pattern = if options.regex {
      query.to_string()
    } else {
      regex::escape(query)
    };
    let regex = regex::RegexBuilder::new(&pattern)
      .case_insensitive(!options.effective_case_sensitive(query))
      .build()
      .map_err(|error| error.to_string())?;

    Ok(Self { regex, options })
  }

  pub fn matches_for_line(
    &self,
    display_line: usize,
    line_start_offset: usize,
    line_text: &str,
    matches: &mut Vec<SearchMatch>,
  ) {
    for found in self.regex.find_iter(line_text) {
      let byte_start = found.start();
      let byte_end = found.end();
      if byte_start == byte_end {
        continue;
      }
      if self.options.whole_word && !is_whole_word_match(line_text, byte_start, byte_end) {
        continue;
      }
      if !line_text.is_char_boundary(byte_start) || !line_text.is_char_boundary(byte_end) {
        continue;
      }
      let column_start = line_text[..byte_start].chars().count();
      let column_end = line_text[..byte_end].chars().count();
      let range_start = line_start_offset + column_start;
      let range_end = line_start_offset + column_end;
      matches.push(SearchMatch {
        display_line,
        column_start,
        column_end,
        doc_range: range_start..range_end,
      });
    }
  }
}

impl SearchOptions {
  pub fn effective_case_sensitive(&self, query: &str) -> bool {
    self.case_sensitive || query.chars().any(char::is_uppercase)
  }
}

impl SearchState {
  pub fn new(options: SearchOptions) -> Self {
    Self {
      options,
      ..Self::default()
    }
  }

  pub fn query(&self) -> &str {
    &self.query
  }

  pub fn set_query(&mut self, query: String) {
    self.query = query;
  }

  pub fn options(&self) -> SearchOptions {
    self.options
  }

  pub fn matches(&self) -> &[SearchMatch] {
    &self.matches
  }

  pub fn active_match(&self) -> Option<usize> {
    self.active_match
  }

  pub fn highlights(&self) -> SearchHighlights {
    SearchHighlights {
      matches: self.matches.clone(),
      active_match: self.active_match,
      active_range: self
        .active_match
        .and_then(|index| self.matches.get(index))
        .map(|found| found.doc_range.clone()),
    }
  }

  pub fn set_active_match(&mut self, active_match: Option<usize>) {
    self.active_match = active_match.filter(|index| *index < self.matches.len());
  }

  pub fn error(&self) -> Option<&str> {
    self.error.as_deref()
  }

  pub fn clear(&mut self) {
    self.query.clear();
    self.matches.clear();
    self.active_match = None;
    self.error = None;
  }

  pub fn toggle_case_sensitive(&mut self) {
    self.options.case_sensitive = !self.options.case_sensitive;
  }

  pub fn toggle_whole_word(&mut self) {
    self.options.whole_word = !self.options.whole_word;
  }

  pub fn toggle_regex(&mut self) {
    self.options.regex = !self.options.regex;
  }

  pub fn current_match_number(&self) -> usize {
    self
      .active_match
      .map(|index| index + 1)
      .unwrap_or(0)
      .min(self.matches.len())
  }

  pub fn replace_matches(
    &mut self,
    matches: Result<Vec<SearchMatch>, String>,
    preserve_active_match: bool,
    cursor: usize,
  ) {
    if self.query.is_empty() {
      self.matches.clear();
      self.active_match = None;
      self.error = None;
      return;
    }

    let previous_active_match = preserve_active_match.then(|| self.active_match()).flatten();
    let previous_active_match = previous_active_match.and_then(|index| self.matches.get(index));
    let previous_active_match = previous_active_match.cloned();

    match matches {
      Ok(matches) => {
        self.matches = matches;
        self.error = None;
      }
      Err(error) => {
        self.matches.clear();
        self.active_match = None;
        self.error = Some(error);
        return;
      }
    }

    if self.matches.is_empty() {
      self.active_match = None;
      return;
    }

    self.active_match = previous_active_match
      .and_then(|previous| {
        self
          .matches
          .iter()
          .position(|candidate| *candidate == previous)
      })
      .or_else(|| self.match_index_from_cursor(SearchDirection::Next, cursor))
      .or(Some(0));
  }

  pub fn navigation_target(&self, direction: SearchDirection, cursor: usize) -> Option<usize> {
    if self.matches.is_empty() {
      return None;
    }

    if let Some(active_index) = self.active_match {
      return Some(match direction {
        SearchDirection::Next => (active_index + 1) % self.matches.len(),
        SearchDirection::Previous if active_index == 0 => self.matches.len() - 1,
        SearchDirection::Previous => active_index - 1,
      });
    }

    self.match_index_from_cursor(direction, cursor)
  }

  fn match_index_from_cursor(&self, direction: SearchDirection, cursor: usize) -> Option<usize> {
    if self.matches.is_empty() {
      return None;
    }

    match direction {
      SearchDirection::Next => self
        .matches
        .iter()
        .position(|candidate| candidate.doc_range.start >= cursor)
        .or(Some(0)),
      SearchDirection::Previous => self
        .matches
        .iter()
        .rposition(|candidate| candidate.doc_range.end <= cursor)
        .or_else(|| self.matches.len().checked_sub(1)),
    }
  }
}

fn is_whole_word_match(line_text: &str, byte_start: usize, byte_end: usize) -> bool {
  let previous_is_word = line_text[..byte_start]
    .chars()
    .next_back()
    .is_some_and(is_find_word_char);
  let next_is_word = line_text[byte_end..]
    .chars()
    .next()
    .is_some_and(is_find_word_char);
  !previous_is_word && !next_is_word
}

fn is_find_word_char(ch: char) -> bool {
  ch.is_alphanumeric() || ch == '_'
}
