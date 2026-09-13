use std::{
  ops::Range,
  path::{Path, PathBuf},
  sync::{Arc, OnceLock},
};

use alacritty_terminal::term::cell::Flags;
use regex::Regex;

use crate::{ScreenSnapshot, ViewportPoint};

const URL_PATTERN: &str = r#"(?i)(?:https?://|mailto:|ftp://|ssh://|git://)[^\x00-\x20<>\"`']+"#;
const PATH_CANDIDATE_PATTERN: &str = r#"[^(){}\[\]<>\"'`\s]+(?:\([0-9]+(?:[,:][0-9]+)?\))?"#;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TerminalLinkTarget {
  Url(Arc<str>),
  Path {
    path: PathBuf,
    line: Option<u32>,
    column: Option<u32>,
  },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TerminalLink {
  pub(crate) target: TerminalLinkTarget,
  pub(crate) tooltip: Arc<str>,
}

struct SnapshotLine {
  text: String,
  cells: Vec<(Range<usize>, Range<usize>)>,
}

impl SnapshotLine {
  fn columns_for_match(&self, byte_range: Range<usize>) -> Option<Range<usize>> {
    let mut matching_cells = self.cells.iter().filter(|(cell_bytes, _)| {
      cell_bytes.start < byte_range.end && cell_bytes.end > byte_range.start
    });
    let (_, first_columns) = matching_cells.next()?;
    let mut columns = first_columns.clone();
    for (_, cell_columns) in matching_cells {
      columns.end = cell_columns.end;
    }
    Some(columns)
  }
}

pub(crate) fn link_at(
  screen: &ScreenSnapshot,
  point: ViewportPoint,
  working_directory: Option<&Path>,
) -> Option<TerminalLink> {
  if point.row >= screen.rows || point.col >= screen.cols {
    return None;
  }

  if let Some(uri) = screen
    .cells
    .iter()
    .find(|cell| cell.row == point.row && cell.col == point.col)
    .and_then(|cell| cell.hyperlink_uri.clone())
  {
    return Some(TerminalLink {
      target: TerminalLinkTarget::Url(uri.clone()),
      tooltip: uri,
    });
  }

  let line = snapshot_line(screen, point.row)?;
  for found in url_regex().find_iter(&line.text) {
    let text = sanitize_url_match(found.as_str());
    if text.is_empty() {
      continue;
    }
    let byte_range = found.start()..found.start() + text.len();
    let Some(columns) = line.columns_for_match(byte_range) else {
      continue;
    };
    if columns.contains(&point.col) {
      let url = Arc::<str>::from(text);
      return Some(TerminalLink {
        target: TerminalLinkTarget::Url(url.clone()),
        tooltip: url,
      });
    }
  }

  for found in path_candidate_regex().find_iter(&line.text) {
    let text = found
      .as_str()
      .trim_end_matches(['.', ',', ';', ':', '!', '?']);
    if text.is_empty() {
      continue;
    }
    let byte_range = found.start()..found.start() + text.len();
    let Some(columns) = line.columns_for_match(byte_range) else {
      continue;
    };
    if !columns.contains(&point.col) {
      continue;
    }

    let (path, line, column) = parse_path_with_position(text);
    let Some(path) = resolve_terminal_path(path, working_directory) else {
      continue;
    };
    return Some(TerminalLink {
      target: TerminalLinkTarget::Path { path, line, column },
      tooltip: Arc::<str>::from(text),
    });
  }

  None
}

fn url_regex() -> &'static Regex {
  static URL_REGEX: OnceLock<Regex> = OnceLock::new();
  URL_REGEX.get_or_init(|| Regex::new(URL_PATTERN).expect("terminal URL regex should compile"))
}

fn path_candidate_regex() -> &'static Regex {
  static PATH_REGEX: OnceLock<Regex> = OnceLock::new();
  PATH_REGEX
    .get_or_init(|| Regex::new(PATH_CANDIDATE_PATTERN).expect("terminal path regex should compile"))
}

fn snapshot_line(screen: &ScreenSnapshot, row: usize) -> Option<SnapshotLine> {
  if row >= screen.rows || screen.cols == 0 {
    return None;
  }

  let mut text = String::new();
  let mut cells = Vec::with_capacity(screen.cols);
  let mut row_cells = screen
    .cells
    .iter()
    .filter(|cell| cell.row == row)
    .collect::<Vec<_>>();
  row_cells.sort_unstable_by_key(|cell| cell.col);
  let mut column = 0;

  for cell in row_cells {
    while column < cell.col.min(screen.cols) {
      let byte_start = text.len();
      text.push(' ');
      cells.push((byte_start..text.len(), column..column + 1));
      column += 1;
    }
    if column >= screen.cols {
      break;
    }
    if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
      column += 1;
      continue;
    }

    let byte_start = text.len();
    text.push(cell.c);
    text.extend(cell.zerowidth.iter());
    let column_end = column + usize::from(cell.flags.contains(Flags::WIDE_CHAR)) + 1;
    cells.push((byte_start..text.len(), column..column_end.min(screen.cols)));
    column += 1;
  }

  while column < screen.cols {
    let byte_start = text.len();
    text.push(' ');
    cells.push((byte_start..text.len(), column..column + 1));
    column += 1;
  }

  Some(SnapshotLine { text, cells })
}

fn sanitize_url_match(text: &str) -> &str {
  text.trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '}'])
}

fn parse_path_with_position(text: &str) -> (&str, Option<u32>, Option<u32>) {
  if let Some(without_closing) = text.strip_suffix(')')
    && let Some(opening) = without_closing.rfind('(')
  {
    let position = &without_closing[opening + 1..];
    let mut parts = position.split([',', ':']);
    if let Some(line) = parts.next().and_then(parse_position_number) {
      let column = parts.next().and_then(parse_position_number);
      if parts.next().is_none() {
        return (&text[..opening], Some(line), column);
      }
    }
  }

  let Some(last_colon) = text.rfind(':') else {
    return (text, None, None);
  };
  let Some(last_number) = parse_position_number(&text[last_colon + 1..]) else {
    return (text, None, None);
  };
  let before_last = &text[..last_colon];
  if let Some(previous_colon) = before_last.rfind(':')
    && let Some(line) = parse_position_number(&before_last[previous_colon + 1..])
  {
    return (
      &before_last[..previous_colon],
      Some(line),
      Some(last_number),
    );
  }

  (before_last, Some(last_number), None)
}

fn parse_position_number(text: &str) -> Option<u32> {
  text.parse::<u32>().ok().filter(|number| *number > 0)
}

fn resolve_terminal_path(path: &str, working_directory: Option<&Path>) -> Option<PathBuf> {
  let expanded = if let Some(relative) = path.strip_prefix("~/") {
    std::env::var_os("HOME").map(PathBuf::from)?.join(relative)
  } else {
    let path = Path::new(path);
    if path.is_absolute() {
      path.to_path_buf()
    } else {
      working_directory?.join(path)
    }
  };
  let resolved = expanded.canonicalize().ok()?;
  resolved.is_file().then_some(resolved)
}

#[cfg(test)]
mod tests {
  use std::sync::Arc;

  use alacritty_terminal::{
    term::cell::Flags,
    vte::ansi::{Color, NamedColor},
  };

  use super::{TerminalLinkTarget, link_at, parse_path_with_position};
  use crate::{ScreenSnapshot, TerminalCellSnapshot, ViewportPoint};

  fn screen_from_line(line: &str) -> ScreenSnapshot {
    let cells = line
      .chars()
      .enumerate()
      .map(|(col, character)| TerminalCellSnapshot {
        row: 0,
        col,
        c: character,
        zerowidth: Arc::default(),
        fg: Color::Named(NamedColor::Foreground),
        bg: Color::Named(NamedColor::Background),
        flags: Flags::empty(),
        underline_color: None,
        hyperlink_uri: None,
      })
      .collect::<Vec<_>>();
    ScreenSnapshot {
      rows: 1,
      cols: cells.len(),
      cells,
      ..ScreenSnapshot::default()
    }
  }

  #[test]
  fn path_positions_support_colons_and_parentheses() {
    assert_eq!(
      parse_path_with_position("src/main.rs:42:8"),
      ("src/main.rs", Some(42), Some(8))
    );
    assert_eq!(
      parse_path_with_position("src/main.rs:42"),
      ("src/main.rs", Some(42), None)
    );
    assert_eq!(
      parse_path_with_position("src/main.rs(42,8)"),
      ("src/main.rs", Some(42), Some(8))
    );
    assert_eq!(
      parse_path_with_position("src/main.rs"),
      ("src/main.rs", None, None)
    );
  }

  #[test]
  fn plain_urls_are_detected_without_osc_hyperlinks() {
    let screen = screen_from_line("See https://example.com/docs.");
    let link =
      link_at(&screen, ViewportPoint { row: 0, col: 8 }, None).expect("URL should be detected");

    assert_eq!(
      link.target,
      TerminalLinkTarget::Url(Arc::<str>::from("https://example.com/docs"))
    );
  }
}
