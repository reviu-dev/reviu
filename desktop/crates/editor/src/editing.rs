use super::*;
use crate::indentation::leading_whitespace;
use syntax::{SyntaxHighlighter, TokenType};

#[cfg(test)]
#[path = "editing_tests.rs"]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TextEdit {
  pub range: Range<usize>,
  pub text: String,
}

pub(super) fn map_offset(offset: usize, edits: &[TextEdit]) -> usize {
  let mut removed = 0;
  let mut inserted = 0;
  for edit in edits {
    if offset < edit.range.start {
      break;
    }
    if offset <= edit.range.end {
      return edit.range.start - removed + inserted + edit.text.chars().count();
    }
    removed += edit.range.len();
    inserted += edit.text.chars().count();
  }
  offset - removed + inserted
}

pub(super) fn non_code_at_end(
  source: &str,
  highlights: &[syntax::HighlightSpan],
  language: &str,
) -> bool {
  let block_comments = matches!(
    language,
    "rust"
      | "c"
      | "cpp"
      | "csharp"
      | "go"
      | "typescript"
      | "java"
      | "kotlin"
      | "swift"
      | "dart"
      | "php"
      | "scala"
      | "css"
      | "scss"
      | "hcl"
  );
  let nested = matches!(
    language,
    "rust" | "swift" | "kotlin" | "scala" | "ocaml" | "ocaml_interface" | "haskell"
  );
  let (block_start, block_end) = match language {
    "html" | "xml" | "markdown" => (b"<!--".as_slice(), b"-->".as_slice()),
    "ocaml" | "ocaml_interface" => (b"(*".as_slice(), b"*)".as_slice()),
    "haskell" => (b"{-".as_slice(), b"-}".as_slice()),
    "lua" => (b"--[[".as_slice(), b"]]".as_slice()),
    "julia" => (b"#=".as_slice(), b"=#".as_slice()),
    _ => (b"/*".as_slice(), b"*/".as_slice()),
  };
  let line_comment = match language {
    "python" | "yaml" | "ruby" | "bash" | "hcl" | "toml" | "make" | "dockerfile" | "cmake"
    | "r" | "julia" | "elixir" => Some(b"#".as_slice()),
    "sql" | "lua" | "haskell" => Some(b"--".as_slice()),
    "clojure" => Some(b";".as_slice()),
    "zig" => Some(b"//".as_slice()),
    _ if block_comments => Some(b"//".as_slice()),
    _ => None,
  };
  let mut depth = 0usize;
  let mut offset = 0;
  let mut quote: Option<Vec<u8>> = None;
  let mut raw = false;
  let bytes = source.as_bytes();
  while let Some(rest) = bytes.get(offset..).filter(|rest| !rest.is_empty()) {
    if let Some(delimiter) = &quote {
      if rest.starts_with(delimiter) {
        offset += delimiter.len();
        quote = None;
      } else if !raw && rest.starts_with(b"\\") {
        offset += if rest.starts_with(b"\\\r\n") { 3 } else { 2 };
      } else if matches!(language, "typescript" | "python")
        && matches!(delimiter.as_slice(), b"'" | b"\"")
        && matches!(rest.first(), Some(b'\r' | b'\n'))
      {
        // An invalid single-line string must not swallow code on subsequent lines.
        quote = None;
        offset += 1;
      } else {
        offset += 1;
      }
      continue;
    }
    if depth > 0 {
      if rest.starts_with(block_end) {
        depth -= 1;
        offset += block_end.len();
      } else if nested && rest.starts_with(block_start) {
        depth += 1;
        offset += block_start.len();
      } else {
        offset += 1;
      }
      continue;
    }
    if matches!(rest.first(), Some(b'/' | b'\''))
      && let Some(span) = highlights.iter().find(|span| {
        span.byte_range.contains(&offset)
          && matches!(
            span.token_type,
            TokenType::StringRegex | TokenType::Lifetime
          )
      })
    {
      offset = span.byte_range.end;
      continue;
    }
    if (block_comments || block_start != b"/*" || language == "sql")
      && rest.starts_with(block_start)
    {
      depth = 1;
      offset += block_start.len();
    } else if line_comment.is_some_and(|marker| rest.starts_with(marker)) {
      offset += rest
        .iter()
        .position(|byte| *byte == b'\n')
        .unwrap_or(rest.len());
      if offset == bytes.len() {
        return true;
      }
    } else if matches!(rest.first(), Some(b'\'' | b'"' | b'`')) {
      if rest[0] == b'\''
        && (matches!(
          language,
          "clojure" | "haskell" | "ocaml" | "ocaml_interface"
        ) || (matches!(language, "plain" | "markdown" | "yaml")
          && source[..offset]
            .chars()
            .next_back()
            .is_some_and(|character| character.is_alphanumeric())))
      {
        offset += 1;
        continue;
      }
      let character = rest[0];
      let mut delimiter = vec![character];
      let triple = [character; 3];
      if matches!(
        language,
        "python" | "kotlin" | "swift" | "scala" | "toml" | "julia" | "elixir"
      ) && rest.starts_with(&triple)
      {
        delimiter = triple.to_vec();
      }
      raw = (language == "go" && character == b'`')
        || (matches!(language, "bash" | "toml") && character == b'\'');
      if language == "rust" && character == b'"' {
        let before = &bytes[..offset];
        let hashes = before
          .iter()
          .rev()
          .take_while(|byte| **byte == b'#')
          .count();
        raw = before.get(before.len().saturating_sub(hashes + 1)) == Some(&b'r');
        if raw {
          delimiter.extend(std::iter::repeat_n(b'#', hashes));
        }
      }
      offset += if language == "rust" && raw {
        1
      } else {
        delimiter.len()
      };
      quote = Some(delimiter);
    } else {
      offset += 1;
    }
  }
  depth > 0 || quote.is_some()
}

pub(super) fn block_opener(document: &Document, prefix: &str, line_start: usize) -> Option<char> {
  let config = document.language_config()?;
  if !prefix.trim_end().ends_with(['{', '[', '(', ':'])
    && !["//", "/*", "#"]
      .iter()
      .any(|marker| prefix.contains(marker))
  {
    return None;
  }
  if matches!(config.name, "markdown" | "html" | "xml" | "make") {
    return None;
  }
  // Bound synchronous syntax work; large files still inherit their existing indentation.
  if document.len() > 256_000 {
    return None;
  }
  let source = document.slice_to_string(0..document.len());
  let line_byte = document.slice_to_string(0..line_start).len();
  let highlights = match SyntaxHighlighter::new(config).highlight_text(&source) {
    Ok(highlights) => highlights,
    Err(error) => {
      log::warn!("Could not determine newline indentation: {error}");
      return None;
    }
  };
  let (byte, character) = prefix.char_indices().rev().find(|(byte, character)| {
    !character.is_whitespace()
      && !highlights.iter().any(|span| {
        span.byte_range.contains(&(line_byte + byte))
          && matches!(span.token_type, TokenType::Comment | TokenType::CommentDoc)
      })
  })?;
  if highlights.iter().any(|span| {
    span.byte_range.contains(&(line_byte + byte))
      && matches!(
        span.token_type,
        TokenType::String | TokenType::StringEscape | TokenType::StringRegex
      )
  }) {
    return None;
  }
  // Incomplete strings and comments can be recovered as code by the syntax parser.
  if non_code_at_end(
    &source[..line_byte + byte + character.len_utf8()],
    &highlights,
    config.name,
  ) {
    return None;
  }
  match character {
    '{' | '[' | '(' => Some(character),
    ':' if matches!(config.name, "python" | "yaml") => Some(character),
    _ => None,
  }
}

impl Editor {
  pub(crate) fn replace_literal_text_in_range(
    &mut self,
    range_utf16: Option<Range<usize>>,
    new_text: &str,
    cx: &mut Context<Self>,
  ) {
    if !self.can_edit_text(cx) {
      return;
    }
    if self.selections.len() > 1 && self.marked_range.is_none() {
      let primary = self.selections.primary().range.clone();
      let explicit = range_utf16
        .as_ref()
        .map(|range| self.range_from_utf16(range, cx));
      if explicit.as_ref().is_none_or(|range| *range == primary) {
        self.edit_selections(cx, |_, selection, _| {
          multicursor::EditPlan::replace(selection.range.clone(), new_text.to_string())
        });
        return;
      }
      self.retain_primary_selection(cx);
    }
    self.cursor_blink.update(cx, |blink, cx| {
      blink.pause_blinking(cx);
    });
    self.display_selection = None;
    let range = range_utf16
      .as_ref()
      .map(|range_utf16| self.range_from_utf16(range_utf16, cx))
      .or(self.marked_range.clone())
      .unwrap_or(self.selections.primary().range.clone());
    let range = self.clamp_range_to_doc_len(range, cx);
    let was_composing = self.marked_range.is_some();
    let standalone = !was_composing
      && (new_text.contains('\n')
        || !self.selections.primary().range.is_empty()
        || new_text.graphemes(true).count() > 1
        || (!range.is_empty() && !new_text.is_empty()));
    if standalone {
      self.finalize_transaction(cx);
    }

    let selection_before = self.selection_snapshot(cx);
    self.auto_pairs.prepare(
      self.document.read(cx).buffer.version(),
      &[TextEdit {
        range: range.clone(),
        text: new_text.to_string(),
      }],
    );
    let start_line = self.document.read(cx).char_to_line(range.start);
    let end_line = self.document.read(cx).char_to_line(range.end);

    let line_height = self.measured_editor_line_height();
    let doc_line_count = self.document.read(cx).len_lines();
    let total_display_lines = self.display_line_count(doc_line_count);
    let display_viewport = self.viewport_range(line_height, total_display_lines);
    let doc_viewports = self.doc_ranges_for_display_viewport(display_viewport);
    let new_line_count = new_text.matches('\n').count();
    let force_end_line = start_line.saturating_add(new_line_count).max(end_line);
    let force_range = start_line..(force_end_line + 1);

    self.maybe_optimistic_unstage_for_edit(start_line, end_line, cx);

    let transaction_id = self.document.update(cx, |doc, cx| {
      let id = if was_composing {
        doc
          .buffer
          .continue_transaction(Instant::now(), |buffer, transaction| {
            buffer.replace(transaction, range.clone(), new_text);
          })
      } else {
        doc
          .buffer
          .transaction(Instant::now(), |buffer, transaction| {
            buffer.replace(transaction, range.clone(), new_text);
          })
      };

      if !doc.should_defer_full_highlight() {
        doc.schedule_recompute_highlights(cx);
      }
      doc.schedule_viewport_highlights_for_ranges(
        &doc_viewports,
        Some(force_range.clone()),
        crate::document::VIEWPORT_HIGHLIGHT_MARGIN_LINES,
        cx,
      );

      cx.notify();
      id
    });
    self.mark_conflict_cache_dirty();

    let has_newline = new_text.contains('\n');

    if has_newline || start_line != end_line {
      self.invalidate_lines_from(start_line);
    } else {
      self.invalidate_line(start_line);
    }

    let new_text_chars = new_text.chars().count();
    let doc_len_after = self.document.read(cx).len();
    let new_cursor = (range.start + new_text_chars).min(doc_len_after);
    self.selections.primary_mut().range = new_cursor..new_cursor;
    self.selections.primary_mut().reversed = false;
    self.marked_range.take();

    self.record_transaction(transaction_id, selection_before, cx);
    if standalone || was_composing {
      self.finalize_transaction(cx);
    }
    self.refresh_dirty(cx);
    self.ensure_cursor_visible_when_hidden(cx);
    self.refresh_find_matches_after_document_edit(cx);
    cx.notify();
    self.schedule_diff_recompute(cx);
  }

  pub(super) fn can_edit_text(&self, cx: &App) -> bool {
    !self.selection_is_read_only()
      && !(self.selections.primary().range.is_empty() && self.is_read_only_display_cursor(cx))
  }

  pub(super) fn apply_text_edits(
    &mut self,
    edits: Vec<TextEdit>,
    selection: SelectionSnapshot,
    cx: &mut Context<Self>,
  ) {
    self.apply_selection_plans(
      vec![(
        self.selections.primary().id,
        multicursor::EditPlan::new(edits, selection),
      )],
      cx,
    );
  }

  pub(crate) fn insert_indented_newline(&mut self, cx: &mut Context<Self>) {
    self.edit_selections(cx, |editor, selection, cx| {
      editor.newline_plan(selection, cx)
    });
  }

  fn newline_plan(&self, selection: &Selection, cx: &App) -> multicursor::EditPlan {
    let document = self.document.read(cx);
    let mut range = selection.range.clone();
    let line = document.char_to_line(range.start);
    let line_start = document.line_to_char(line);
    let prefix = document.slice_to_string(line_start..range.start);
    let indent = leading_whitespace(&prefix);
    let end_line = document.char_to_line(range.end);
    let suffix_end = (document.line_to_char(end_line)
      + document
        .line_content(end_line)
        .map_or(0, |line| line.chars().count()))
    .max(range.end);
    let suffix = document.slice_to_string(range.end..suffix_end);
    let newline = document.line_ending();
    let opener = block_opener(document, &prefix, line_start);
    let extra_indent = opener
      .map(|_| document.indentation.unit())
      .unwrap_or_default();
    let mut text = format!("{newline}{indent}{extra_indent}");
    if range.is_empty()
      && !indent.is_empty()
      && prefix == indent
      && document
        .line_content(line)
        .is_some_and(|line| leading_whitespace(&line).len() == prefix.len())
    {
      range.start = line_start;
    }
    let cursor = range.start + text.chars().count();
    let suffix_indent = leading_whitespace(&suffix);
    range.end += suffix_indent.chars().count();
    let paired = matches!(
      (
        opener,
        suffix.trim_start_matches([' ', '\t']).chars().next()
      ),
      (Some('{'), Some('}')) | (Some('['), Some(']')) | (Some('('), Some(')'))
    );
    if paired {
      text.push_str(&format!("{newline}{indent}"));
    }
    multicursor::EditPlan::new(
      vec![TextEdit { range, text }],
      SelectionSnapshot {
        range: cursor..cursor,
        reversed: false,
      },
    )
  }

  pub(crate) fn indent_selection(&mut self, outdent: bool, cx: &mut Context<Self>) {
    self.edit_selections(cx, |editor, selection, cx| {
      editor.indent_plan(selection, outdent, cx)
    });
  }

  fn indent_plan(&self, selection: &Selection, outdent: bool, cx: &App) -> multicursor::EditPlan {
    let document = self.document.read(cx);
    let indentation = document.indentation;
    let first_line = document.char_to_line(selection.range.start);
    if selection.range.is_empty() && !outdent {
      let prefix =
        document.slice_to_string(document.line_to_char(first_line)..selection.range.start);
      let text = indentation.tab_at(indentation.columns(&prefix));
      let cursor = selection.range.start + text.chars().count();
      return multicursor::EditPlan::new(
        vec![TextEdit {
          range: selection.range.clone(),
          text,
        }],
        SelectionSnapshot {
          range: cursor..cursor,
          reversed: false,
        },
      );
    }
    let mut last_line = document.char_to_line(selection.range.end);
    if last_line > first_line && selection.range.end == document.line_to_char(last_line) {
      last_line -= 1;
    }
    let mut edits = Vec::new();
    for line in first_line..=last_line {
      let Some(text) = document.line_content(line) else {
        continue;
      };
      let prefix = leading_whitespace(&text);
      let columns = indentation.columns(prefix);
      let start = document.line_to_char(line);
      if outdent {
        if columns == 0 {
          continue;
        }
        let target = ((columns - 1) / indentation.width) * indentation.width;
        let mut column = 0;
        let kept = prefix
          .chars()
          .take_while(|character| {
            column += if *character == '\t' {
              indentation.width - column % indentation.width
            } else {
              1
            };
            column <= target
          })
          .count();
        edits.push(TextEdit {
          range: start + kept..start + prefix.chars().count(),
          text: String::new(),
        });
      } else {
        let offset = start + prefix.chars().count();
        edits.push(TextEdit {
          range: offset..offset,
          text: indentation.tab_at(columns),
        });
      }
    }
    let after = SelectionSnapshot {
      range: map_offset(selection.range.start, &edits)..map_offset(selection.range.end, &edits),
      reversed: selection.reversed,
    };
    multicursor::EditPlan::new(edits, after)
  }
}
