use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnsiColor {
  Indexed(u8),
  Rgb(u8, u8, u8),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AnsiStyle {
  pub fg: Option<AnsiColor>,
  pub bg: Option<AnsiColor>,
  pub bold: bool,
  pub dim: bool,
  pub italic: bool,
  pub underline: bool,
}

impl AnsiStyle {
  fn is_default(&self) -> bool {
    *self == Self::default()
  }
}

#[derive(Clone, Debug, Default)]
pub struct AnsiSpan {
  pub range: Range<usize>,
  pub style: AnsiStyle,
}

#[derive(Clone, Debug, Default)]
pub struct AnsiLine {
  pub text: String,
  pub spans: Vec<AnsiSpan>,
}

/// Splits terminal output into stripped lines with styled spans. SGR state
/// carries across newlines like a real terminal; a bare carriage return
/// restarts its line so progress bars show their final state.
pub fn parse_ansi(input: &str) -> Vec<AnsiLine> {
  let mut lines = Vec::new();
  let mut line = AnsiLine::default();
  let mut style = AnsiStyle::default();
  let mut span_start = 0usize;
  let mut chars = input.chars().peekable();

  fn close_span(line: &mut AnsiLine, style: AnsiStyle, span_start: usize) {
    if !style.is_default() && span_start < line.text.len() {
      line.spans.push(AnsiSpan {
        range: span_start..line.text.len(),
        style,
      });
    }
  }

  while let Some(ch) = chars.next() {
    match ch {
      '\u{1b}' => match chars.peek() {
        Some('[') => {
          chars.next();
          let mut params = String::new();
          let mut final_byte = None;
          for c in chars.by_ref() {
            match c {
              '0'..='9' | ';' | ':' | '<' | '=' | '>' | '?' => params.push(c),
              ' '..='/' => {}
              '@'..='~' => {
                final_byte = Some(c);
                break;
              }
              _ => break,
            }
          }
          if final_byte == Some('m') {
            close_span(&mut line, style, span_start);
            apply_sgr(&mut style, &params);
            span_start = line.text.len();
          }
        }
        Some(']') => {
          chars.next();
          while let Some(c) = chars.next() {
            if c == '\u{07}' {
              break;
            }
            if c == '\u{1b}' && chars.peek() == Some(&'\\') {
              chars.next();
              break;
            }
          }
        }
        Some(&c) if (' '..='/').contains(&c) => {
          chars.next();
          chars.next();
        }
        Some(_) => {
          chars.next();
        }
        None => {}
      },
      '\n' => {
        close_span(&mut line, style, span_start);
        lines.push(std::mem::take(&mut line));
        span_start = 0;
      }
      '\r' => {
        if chars.peek() == Some(&'\n') {
          continue;
        }
        line.text.clear();
        line.spans.clear();
        span_start = 0;
      }
      c => line.text.push(c),
    }
  }
  close_span(&mut line, style, span_start);
  if !line.text.is_empty() || !line.spans.is_empty() {
    lines.push(line);
  }
  lines
}

pub fn strip_ansi_escapes(input: &str) -> String {
  parse_ansi(input)
    .into_iter()
    .map(|line| line.text)
    .collect::<Vec<_>>()
    .join("\n")
}

fn apply_sgr(style: &mut AnsiStyle, params: &str) {
  let mut values = params
    .split([';', ':'])
    .map(|p| p.parse::<u16>().unwrap_or(0));
  while let Some(value) = values.next() {
    match value {
      0 => *style = AnsiStyle::default(),
      1 => style.bold = true,
      2 => style.dim = true,
      3 => style.italic = true,
      4 => style.underline = true,
      22 => {
        style.bold = false;
        style.dim = false;
      }
      23 => style.italic = false,
      24 => style.underline = false,
      30..=37 => style.fg = Some(AnsiColor::Indexed(value as u8 - 30)),
      39 => style.fg = None,
      40..=47 => style.bg = Some(AnsiColor::Indexed(value as u8 - 40)),
      49 => style.bg = None,
      90..=97 => style.fg = Some(AnsiColor::Indexed(value as u8 - 90 + 8)),
      100..=107 => style.bg = Some(AnsiColor::Indexed(value as u8 - 100 + 8)),
      38 | 48 => {
        let target_fg = value == 38;
        let color = match values.next() {
          Some(5) => values.next().map(|n| AnsiColor::Indexed(n as u8)),
          Some(2) => {
            let (r, g, b) = (values.next(), values.next(), values.next());
            match (r, g, b) {
              (Some(r), Some(g), Some(b)) => Some(AnsiColor::Rgb(r as u8, g as u8, b as u8)),
              _ => None,
            }
          }
          _ => None,
        };
        if target_fg {
          style.fg = color.or(style.fg);
        } else {
          style.bg = color.or(style.bg);
        }
      }
      _ => {}
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn plain_text_passes_through_unstyled() {
    let lines = parse_ansi("hello\nworld");
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].text, "hello");
    assert!(lines[0].spans.is_empty());
    assert_eq!(lines[1].text, "world");
  }

  #[test]
  fn sgr_colors_become_spans_and_reset_ends_them() {
    let lines = parse_ansi("a \u{1b}[32mgreen\u{1b}[0m tail");
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].text, "a green tail");
    assert_eq!(lines[0].spans.len(), 1);
    let span = &lines[0].spans[0];
    assert_eq!(&lines[0].text[span.range.clone()], "green");
    assert_eq!(span.style.fg, Some(AnsiColor::Indexed(2)));
  }

  #[test]
  fn style_state_carries_across_lines() {
    let lines = parse_ansi("\u{1b}[1;31mboth\nlines\u{1b}[0m ok");
    assert_eq!(lines.len(), 2);
    let first = &lines[0].spans[0];
    assert!(first.style.bold);
    assert_eq!(first.style.fg, Some(AnsiColor::Indexed(1)));
    let second = &lines[1].spans[0];
    assert_eq!(&lines[1].text[second.range.clone()], "lines");
    assert!(second.style.bold);
  }

  #[test]
  fn extended_colors_parse() {
    let lines = parse_ansi("\u{1b}[38;5;208morange\u{1b}[m \u{1b}[38;2;1;2;3mrgb");
    let spans = &lines[0].spans;
    assert_eq!(spans[0].style.fg, Some(AnsiColor::Indexed(208)));
    assert_eq!(spans[1].style.fg, Some(AnsiColor::Rgb(1, 2, 3)));
  }

  #[test]
  fn non_sgr_sequences_are_stripped() {
    let lines = parse_ansi("\u{1b}[2J\u{1b}[1;1Hclean \u{1b}]0;title\u{07}text\u{1b}(B!");
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].text, "clean text!");
    assert!(lines[0].spans.is_empty());
  }

  #[test]
  fn a_bare_carriage_return_keeps_the_last_write() {
    let lines = parse_ansi("progress 10%\rprogress 99%\ndone");
    assert_eq!(lines[0].text, "progress 99%");
    assert_eq!(lines[1].text, "done");
    let crlf = parse_ansi("windows\r\nline");
    assert_eq!(crlf[0].text, "windows");
    assert_eq!(crlf[1].text, "line");
  }

  #[test]
  fn a_truncated_trailing_sequence_is_dropped_without_panic() {
    let lines = parse_ansi("safe\u{1b}[38;5");
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].text, "safe");
  }

  #[test]
  fn strip_ansi_escapes_removes_sequences() {
    assert_eq!(
      strip_ansi_escapes("\u{1b}[31mError\u{1b}[0m: boom"),
      "Error: boom"
    );
  }
}
