use super::*;
use buffer::BufferVersion;
use editing::TextEdit;
use syntax::{SyntaxHighlighter, TokenType};

#[cfg(test)]
#[path = "auto_pairs_tests.rs"]
mod tests;

#[derive(Clone, Debug)]
pub(super) struct AutoPair {
  opening: usize,
  closing: usize,
  opener: char,
  closer: char,
}

#[derive(Default)]
pub(super) struct AutoPairs {
  version: Option<BufferVersion>,
  pairs: Vec<AutoPair>,
  before_edit: Option<Vec<AutoPair>>,
}

impl AutoPairs {
  fn validate(&mut self, version: BufferVersion) {
    if self.version != Some(version) {
      self.pairs.clear();
      self.before_edit = None;
      self.version = Some(version);
    }
  }

  pub(super) fn prepare(&mut self, version: BufferVersion, edits: &[TextEdit]) {
    self.validate(version);
    self.before_edit = Some(self.pairs.clone());
    for edit in edits.iter().rev() {
      self.pairs.retain_mut(|pair| {
        if edit.range.contains(&pair.opening) || edit.range.contains(&pair.closing) {
          return false;
        }
        for offset in [&mut pair.opening, &mut pair.closing] {
          if *offset >= edit.range.end {
            *offset = *offset - edit.range.len() + edit.text.chars().count();
          }
        }
        true
      });
    }
  }

  pub(super) fn finish(&mut self, version: BufferVersion) -> (Vec<AutoPair>, Vec<AutoPair>) {
    let before = self.before_edit.take().unwrap_or_else(|| {
      // An edit outside the input path must not leave stale delimiter positions behind.
      std::mem::take(&mut self.pairs)
    });
    self.version = Some(version);
    (before, self.pairs.clone())
  }

  pub(super) fn restore(&mut self, version: BufferVersion, pairs: Vec<AutoPair>) {
    self.version = Some(version);
    self.pairs = pairs;
    self.before_edit = None;
  }
}

fn closing_character(opener: char, language: Option<&str>) -> Option<char> {
  match opener {
    '(' => Some(')'),
    '[' => Some(']'),
    '{' => Some('}'),
    '"' => Some('"'),
    '\''
      if !matches!(
        language,
        Some("rust" | "json" | "clojure" | "haskell" | "ocaml")
      ) =>
    {
      Some('\'')
    }
    '`'
      if matches!(
        language,
        None | Some("typescript" | "go" | "bash" | "ruby" | "markdown" | "sql")
      ) =>
    {
      Some('`')
    }
    _ => None,
  }
}

fn escaped_at(document: &Document, cursor: usize) -> bool {
  let line = document.char_to_line(cursor);
  document
    .slice_to_string(document.line_to_char(line)..cursor)
    .chars()
    .rev()
    .take_while(|character| *character == '\\')
    .count()
    % 2
    == 1
}

fn allows_pair(document: &Document, cursor: usize, opener: char) -> bool {
  let next = (cursor < document.len())
    .then(|| document.slice_to_string(cursor..cursor + 1))
    .and_then(|text| text.chars().next());
  let language = document.language_config().map(|config| config.name);
  let before = if language == Some("json") {
    ",]}"
  } else {
    ";:.,=}])>"
  };
  if next.is_some_and(|character| !character.is_whitespace() && !before.contains(character)) {
    return false;
  }
  if escaped_at(document, cursor) {
    return false;
  }
  let previous = (cursor > 0)
    .then(|| document.slice_to_string(cursor - 1..cursor))
    .and_then(|text| text.chars().next());
  if matches!(opener, '\'' | '"' | '`') {
    if language == Some("markdown") {
      return false;
    }
    let prefix =
      document.slice_to_string(document.line_to_char(document.char_to_line(cursor))..cursor);
    let word = prefix
      .rsplit(|character: char| !character.is_alphanumeric() && character != '_')
      .next()
      .unwrap_or_default();
    let string_prefix = match language {
      Some("python") => matches!(
        word.to_ascii_lowercase().as_str(),
        "r" | "b" | "u" | "f" | "fr" | "rf" | "br" | "rb"
      ),
      Some("rust") => opener == '"' && matches!(word, "b" | "c" | "r" | "br" | "cr"),
      _ => false,
    };
    if !string_prefix
      && previous.is_some_and(|character| character.is_alphanumeric() || character == '_')
    {
      return false;
    }
    if language == Some("rust")
      && prefix.ends_with('#')
      && prefix.trim_end_matches('#').ends_with('r')
    {
      return false;
    }
  }
  // Keep synchronous context inspection bounded; uncertain contexts use literal input.
  if document.len() > 256_000 {
    return false;
  }
  let source = document.slice_to_string(0..document.len());
  let byte = char_offset_to_byte_offset(&source, cursor);
  let Some(config) = document.language_config() else {
    return !editing::non_code_at_end(&source[..byte], &[], "plain");
  };
  let highlights = match SyntaxHighlighter::new(config).highlight_text(&source) {
    Ok(highlights) => highlights,
    Err(error) => {
      log::warn!("Could not determine automatic pair context: {error}");
      return false;
    }
  };
  if highlights.iter().any(|span| {
    span.byte_range.start < byte
      && byte < span.byte_range.end
      && matches!(
        span.token_type,
        TokenType::String
          | TokenType::StringEscape
          | TokenType::StringRegex
          | TokenType::Comment
          | TokenType::CommentDoc
      )
  }) {
    return false;
  }
  !editing::non_code_at_end(&source[..byte], &highlights, config.name)
}

impl Editor {
  pub(super) fn try_auto_pair(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
    let mut characters = text.chars();
    let Some(character) = characters.next() else {
      return false;
    };
    if characters.next().is_some() {
      return false;
    }
    let document = self.document.read(cx);
    self.auto_pairs.validate(document.buffer.version());
    let before_pairs = self.auto_pairs.pairs.clone();
    let before_version = document.buffer.version();
    let closer = closing_character(
      character,
      document.language_config().map(|config| config.name),
    );
    let mut special = false;
    let mut additions = Vec::new();
    let mut skipped = Vec::new();
    let plans = self
      .selections
      .iter()
      .map(|selection| {
        let cursor = selection.range.start;
        let plan = if selection.range.is_empty()
          && !escaped_at(document, cursor)
          && let Some(index) = self.auto_pairs.pairs.iter().rposition(|pair| {
            pair.closing == cursor
              && pair.closer == character
              && document.slice_to_string(cursor..cursor + 1) == text
          }) {
          special = true;
          skipped.push(index);
          multicursor::EditPlan::new(
            Vec::new(),
            SelectionSnapshot {
              range: cursor + 1..cursor + 1,
              reversed: false,
            },
          )
        } else if let Some(closer) =
          closer.filter(|_| !selection.range.is_empty() || allows_pair(document, cursor, character))
        {
          special = true;
          if selection.range.is_empty() {
            additions.push((selection.id, character, closer));
            multicursor::EditPlan::new(
              vec![TextEdit {
                range: cursor..cursor,
                text: format!("{character}{closer}"),
              }],
              SelectionSnapshot {
                range: cursor + 1..cursor + 1,
                reversed: false,
              },
            )
          } else {
            let end = selection.range.end;
            multicursor::EditPlan::new(
              vec![TextEdit {
                range: selection.range.clone(),
                text: format!(
                  "{character}{}{closer}",
                  document.slice_to_string(selection.range.clone())
                ),
              }],
              SelectionSnapshot {
                range: cursor + 1..end + 1,
                reversed: selection.reversed,
              },
            )
          }
        } else {
          multicursor::EditPlan::replace(selection.range.clone(), text.to_string())
        };
        (selection.id, plan)
      })
      .collect();
    if !special {
      return false;
    }
    skipped.sort_unstable();
    for index in skipped.into_iter().rev() {
      self.auto_pairs.pairs.remove(index);
    }
    self.apply_selection_plans(plans, cx);
    for (id, opener, closer) in additions {
      if let Some(selection) = self.selections.iter().find(|selection| selection.id == id) {
        let closing = selection.head();
        self.auto_pairs.pairs.push(AutoPair {
          opening: closing.saturating_sub(1),
          closing,
          opener,
          closer,
        });
      }
    }
    if self.document.read(cx).buffer.version() != before_version
      && let Some(transaction) = self.undo_stack.back_mut()
    {
      transaction.pairs_before = before_pairs;
      transaction.pairs_after = self.auto_pairs.pairs.clone();
    }
    true
  }

  pub(super) fn auto_pair_deletion(&self, cursor: usize, cx: &App) -> Option<Range<usize>> {
    let document = self.document.read(cx);
    if self.auto_pairs.version != Some(document.buffer.version()) {
      return None;
    }
    self
      .auto_pairs
      .pairs
      .iter()
      .find(|pair| {
        pair.opening + 1 == cursor
          && pair.closing == cursor
          && document.slice_to_string(pair.opening..pair.closing + 1)
            == format!("{}{}", pair.opener, pair.closer)
      })
      .map(|pair| pair.opening..pair.closing + 1)
  }

  pub(crate) fn backspace_auto_pair(&mut self, cx: &mut Context<Self>) -> bool {
    if !self.can_edit_text(cx)
      || !self.selections.primary().range.is_empty()
      || self.marked_range.is_some()
    {
      return false;
    }
    let document = self.document.read(cx);
    self.auto_pairs.validate(document.buffer.version());
    let cursor = self.cursor_offset();
    let Some(pair) = self
      .auto_pairs
      .pairs
      .iter()
      .find(|pair| {
        pair.opening + 1 == cursor
          && pair.closing == cursor
          && document.slice_to_string(pair.opening..pair.closing + 1)
            == format!("{}{}", pair.opener, pair.closer)
      })
      .cloned()
    else {
      return false;
    };
    self.apply_text_edits(
      vec![TextEdit {
        range: pair.opening..pair.closing + 1,
        text: String::new(),
      }],
      SelectionSnapshot {
        range: pair.opening..pair.opening,
        reversed: false,
      },
      cx,
    );
    true
  }
}
