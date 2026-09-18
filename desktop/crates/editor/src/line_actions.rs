use super::editing::{TextEdit, block_opener, map_offset};
use super::*;
use crate::indentation::leading_whitespace;

#[cfg(test)]
#[path = "line_actions_tests.rs"]
mod tests;

fn selected_rows(document: &Document, selection: &Range<usize>) -> Range<usize> {
  let first = document.char_to_line(selection.start);
  let last = document.char_to_line(selection.end);
  let end = if last > first && selection.end == document.line_to_char(last) {
    last
  } else {
    last + 1
  };
  first..end
}

struct LineBlock {
  range: Range<usize>,
  first_row: usize,
  lines: Vec<String>,
  endings: Vec<String>,
}

impl LineBlock {
  fn new(document: &Document, rows: Range<usize>) -> Self {
    let start = document.line_to_char(rows.start);
    let end = if rows.end < document.len_lines() {
      document.line_to_char(rows.end)
    } else {
      document.len()
    };
    let mut lines = Vec::new();
    let mut endings = Vec::new();
    for row in rows.clone() {
      let text = document.line_content(row).unwrap_or_default().into_owned();
      let content_end = document.line_to_char(row) + text.chars().count();
      let line_end = document
        .line_range(row)
        .map_or(content_end, |range| range.end);
      endings.push(document.slice_to_string(content_end..line_end));
      lines.push(text);
    }
    Self {
      range: start..end,
      first_row: rows.start,
      lines,
      endings,
    }
  }

  fn text(&self) -> String {
    self
      .lines
      .iter()
      .zip(&self.endings)
      .flat_map(|(line, ending)| [line.as_str(), ending.as_str()])
      .collect()
  }

  fn offset_for(&self, row: usize, column: usize) -> usize {
    self.range.start
      + self
        .lines
        .iter()
        .zip(&self.endings)
        .take(row)
        .map(|(line, ending)| line.chars().count() + ending.chars().count())
        .sum::<usize>()
      + self
        .lines
        .get(row)
        .map_or(0, |line| column.min(line.chars().count()))
  }
}

#[derive(Clone, Copy)]
enum CommentStyle {
  Line(&'static str),
  Block(&'static str, &'static str),
}

fn comment_style(language: &str) -> Option<CommentStyle> {
  match language {
    "rust" | "typescript" | "c" | "cpp" | "csharp" | "go" | "java" | "kotlin" | "swift"
    | "dart" | "php" | "scala" | "zig" => Some(CommentStyle::Line("//")),
    "python" | "ruby" | "bash" | "yaml" | "toml" | "make" | "dockerfile" | "cmake" | "r"
    | "julia" | "elixir" | "hcl" => Some(CommentStyle::Line("#")),
    "sql" | "lua" | "haskell" => Some(CommentStyle::Line("--")),
    "clojure" => Some(CommentStyle::Line(";")),
    "css" | "scss" => Some(CommentStyle::Block("/*", "*/")),
    "html" | "xml" | "markdown" => Some(CommentStyle::Block("<!--", "-->")),
    "ocaml" | "ocaml_interface" => Some(CommentStyle::Block("(*", "*)")),
    _ => None,
  }
}

impl Editor {
  pub(crate) fn move_lines(&mut self, down: bool, cx: &mut Context<Self>) {
    if !self.can_edit_text(cx) {
      return;
    }
    let selection = self.selection_snapshot(cx);
    let document = self.document.read(cx);
    let rows = selected_rows(document, &selection.range);
    if (!down && rows.start == 0) || (down && rows.end == document.len_lines()) {
      return;
    }
    let region = if down {
      rows.start..rows.end + 1
    } else {
      rows.start - 1..rows.end
    };
    let mut block = LineBlock::new(document, region);
    if down {
      block.lines.rotate_right(1);
    } else {
      block.lines.rotate_left(1);
    }
    let map = |offset| {
      let row = document.char_to_line(offset);
      let column = offset - document.line_to_char(row);
      let moved = if down {
        row - block.first_row + 1
      } else {
        row - block.first_row - 1
      };
      block.offset_for(moved, column)
    };
    let after = SelectionSnapshot {
      range: map(selection.range.start)..map(selection.range.end),
      reversed: selection.reversed,
    };
    let text = block.text();
    self.apply_text_edits(
      vec![TextEdit {
        range: block.range,
        text,
      }],
      after,
      cx,
    );
  }

  pub(crate) fn duplicate_lines(&mut self, down: bool, cx: &mut Context<Self>) {
    if !self.can_edit_text(cx) {
      return;
    }
    let selection = self.selection_snapshot(cx);
    let document = self.document.read(cx);
    let block = LineBlock::new(document, selected_rows(document, &selection.range));
    let mut text = block.text();
    if block.endings.last().is_none_or(|ending| ending.is_empty()) {
      text.push_str(document.line_ending());
    }
    let shift = if down { text.chars().count() } else { 0 };
    let after = SelectionSnapshot {
      range: selection.range.start + shift..selection.range.end + shift,
      reversed: selection.reversed,
    };
    self.apply_text_edits(
      vec![TextEdit {
        range: block.range.start..block.range.start,
        text,
      }],
      after,
      cx,
    );
  }

  pub(crate) fn delete_lines(&mut self, cx: &mut Context<Self>) {
    if !self.can_edit_text(cx) {
      return;
    }
    let selection = self.selection_snapshot(cx);
    let document = self.document.read(cx);
    let rows = selected_rows(document, &selection.range);
    let head = if selection.reversed {
      selection.range.start
    } else {
      selection.range.end
    };
    let prefix = document.slice_to_string(document.line_to_char(document.char_to_line(head))..head);
    let column = prefix.graphemes(true).count();
    let mut range = LineBlock::new(document, rows.clone()).range;
    let (target_start, target_row) = if rows.end < document.len_lines() {
      (range.start, Some(rows.end))
    } else if rows.start > 0 {
      let start = document.line_to_char(rows.start - 1);
      let length = document
        .line_content(rows.start - 1)
        .map_or(0, |line| line.chars().count());
      range.start = start + length;
      (start, Some(rows.start - 1))
    } else {
      (0, None)
    };
    let target_column = target_row
      .and_then(|row| document.line_content(row))
      .map_or(0, |line| {
        line
          .graphemes(true)
          .take(column)
          .map(|grapheme| grapheme.chars().count())
          .sum::<usize>()
      });
    let cursor = target_start + target_column;
    self.apply_text_edits(
      vec![TextEdit {
        range,
        text: String::new(),
      }],
      SelectionSnapshot {
        range: cursor..cursor,
        reversed: false,
      },
      cx,
    );
  }

  pub(crate) fn insert_line(&mut self, below: bool, cx: &mut Context<Self>) {
    if !self.can_edit_text(cx) {
      return;
    }
    let document = self.document.read(cx);
    let row = document.char_to_line(self.cursor_offset());
    let current = document.line_content(row).unwrap_or_default();
    let reference_row = if !below && row > 0 && current.trim_start().starts_with(['}', ']', ')']) {
      row - 1
    } else {
      row
    };
    let reference = document.line_content(reference_row).unwrap_or_default();
    let mut indent = leading_whitespace(&reference).to_string();
    if (below || reference_row != row)
      && block_opener(document, &reference, document.line_to_char(reference_row)).is_some()
    {
      indent.push_str(&document.indentation.unit());
    }
    let newline = document.line_ending();
    let at_end = below && row + 1 == document.len_lines();
    let offset = if at_end {
      document.len()
    } else {
      document.line_to_char(row + usize::from(below))
    };
    let text = if at_end {
      format!("{newline}{indent}")
    } else {
      format!("{indent}{newline}")
    };
    let cursor = offset + indent.chars().count() + if at_end { newline.chars().count() } else { 0 };
    self.apply_text_edits(
      vec![TextEdit {
        range: offset..offset,
        text,
      }],
      SelectionSnapshot {
        range: cursor..cursor,
        reversed: false,
      },
      cx,
    );
  }

  pub(crate) fn toggle_line_comments(
    &mut self,
    cx: &mut Context<Self>,
  ) -> Result<(), &'static str> {
    if !self.can_edit_text(cx) {
      return Ok(());
    }
    let selection = self.selection_snapshot(cx);
    let document = self.document.read(cx);
    let style = document
      .language_config()
      .and_then(|config| comment_style(config.name))
      .ok_or("Comment toggling is not available for this file's language.")?;
    let rows = selected_rows(document, &selection.range);
    let block = LineBlock::new(document, rows.clone());
    let mut edits = Vec::new();
    let mut inserted_suffix = None;
    match style {
      CommentStyle::Line(marker) => {
        let nonblank = block
          .lines
          .iter()
          .filter(|line| !line.trim().is_empty())
          .collect::<Vec<_>>();
        let uncomment = !nonblank.is_empty()
          && nonblank
            .iter()
            .all(|line| line.trim_start_matches([' ', '\t']).starts_with(marker));
        let mut common = nonblank
          .first()
          .map_or(String::new(), |line| leading_whitespace(line).to_string());
        for line in &nonblank {
          let count = common
            .chars()
            .zip(leading_whitespace(line).chars())
            .take_while(|(left, right)| left == right)
            .count();
          common.truncate(count);
        }
        for (row, line) in rows.zip(&block.lines) {
          if line.trim().is_empty() && !nonblank.is_empty() {
            continue;
          }
          let start = document.line_to_char(row);
          let indent = leading_whitespace(line).len();
          if uncomment {
            let suffix = &line[indent + marker.len()..];
            let length = marker.len() + usize::from(suffix.starts_with(' '));
            edits.push(TextEdit {
              range: start + indent..start + indent + length,
              text: String::new(),
            });
          } else {
            let offset = start
              + if nonblank.is_empty() {
                indent
              } else {
                common.len()
              };
            edits.push(TextEdit {
              range: offset..offset,
              text: format!("{marker} "),
            });
          }
        }
      }
      CommentStyle::Block(open, close) => {
        let first = block.lines.first().map_or("", String::as_str);
        let last = block.lines.last().map_or("", String::as_str);
        let start = block.range.start + leading_whitespace(first).len();
        let last_row = document.line_to_char(rows.end - 1);
        let end = last_row + last.trim_end_matches([' ', '\t']).chars().count();
        let end = end.max(start);
        let contents = document.slice_to_string(start..end);
        if contents.starts_with(open)
          && contents.ends_with(close)
          && contents.len() >= open.len() + close.len()
        {
          let inner = &contents[open.len()..contents.len() - close.len()];
          if inner.contains(close) {
            return Err("Select one block comment at a time to uncomment it safely.");
          }
          let prefix = open.len() + usize::from(inner.starts_with(' '));
          let suffix = close.len()
            + usize::from(
              inner.ends_with(' ') && inner.len() > usize::from(inner.starts_with(' ')),
            );
          edits.push(TextEdit {
            range: start..start + prefix,
            text: String::new(),
          });
          edits.push(TextEdit {
            range: end - suffix..end,
            text: String::new(),
          });
        } else {
          if contents.contains(close) || (open == "<!--" && contents.contains("--")) {
            return Err(
              "The selection contains a comment delimiter and cannot be safely wrapped in a block comment.",
            );
          }
          inserted_suffix = Some((end, close.len() + 1));
          if start == end {
            edits.push(TextEdit {
              range: start..start,
              text: format!("{open}  {close}"),
            });
          } else {
            edits.push(TextEdit {
              range: start..start,
              text: format!("{open} "),
            });
            edits.push(TextEdit {
              range: end..end,
              text: format!(" {close}"),
            });
          }
        }
      }
    }
    let mut after = SelectionSnapshot {
      range: map_offset(selection.range.start, &edits)..map_offset(selection.range.end, &edits),
      reversed: selection.reversed,
    };
    if let Some((end, length)) = inserted_suffix
      && selection.range.end == end
    {
      after.range.end = after.range.end.saturating_sub(length);
      if selection.range.is_empty() {
        after.range.start = after.range.end;
      }
    }
    self.apply_text_edits(edits, after, cx);
    Ok(())
  }
}
