//! SGR-aware ANSI styling for agent output.

pub(crate) use ansi_text::{AnsiColor, AnsiLine, parse_ansi, strip_ansi_escapes};
use gpui::{Font, FontStyle, FontWeight, Hsla, TextRun, UnderlineStyle, px};

/// VS Code's terminal palettes: readable on our dark and light backgrounds.
const DARK_PALETTE: [u32; 16] = [
  0x000000, 0xcd3131, 0x0dbc79, 0xe5e510, 0x2472c8, 0xbc3fbc, 0x11a8cd, 0xe5e5e5, 0x666666,
  0xf14c4c, 0x23d18b, 0xf5f543, 0x3b8eea, 0xd670d6, 0x29b8db, 0xffffff,
];
const LIGHT_PALETTE: [u32; 16] = [
  0x000000, 0xcd3131, 0x00bc00, 0x949800, 0x0451a5, 0xbc05bc, 0x0598bc, 0x555555, 0x666666,
  0xcd3131, 0x14ce14, 0xb5ba00, 0x0451a5, 0xbc05bc, 0x0598bc, 0xa5a5a5,
];

fn ansi_color_to_hsla(color: AnsiColor, is_dark: bool) -> Hsla {
  match color {
    AnsiColor::Rgb(r, g, b) => rgb_to_hsla(r, g, b),
    AnsiColor::Indexed(n @ 0..=15) => {
      let palette = if is_dark { DARK_PALETTE } else { LIGHT_PALETTE };
      gpui::rgb(palette[n as usize]).into()
    }
    AnsiColor::Indexed(n @ 16..=231) => {
      let n = n - 16;
      let level = |c: u8| if c == 0 { 0 } else { 55 + 40 * c };
      rgb_to_hsla(level(n / 36), level((n % 36) / 6), level(n % 6))
    }
    AnsiColor::Indexed(n) => {
      let gray = 8 + 10 * (n - 232);
      rgb_to_hsla(gray, gray, gray)
    }
  }
}

fn rgb_to_hsla(r: u8, g: u8, b: u8) -> Hsla {
  gpui::Rgba {
    r: r as f32 / 255.,
    g: g as f32 / 255.,
    b: b as f32 / 255.,
    a: 1.,
  }
  .into()
}

/// Materializes a parsed line as text runs; empty when the line is unstyled
/// so the caller can fall back to its default run.
pub(crate) fn runs_for_line(
  line: &AnsiLine,
  base_font: &Font,
  default_color: Hsla,
  is_dark: bool,
) -> Vec<TextRun> {
  if line.spans.is_empty() {
    return Vec::new();
  }
  let default_run = |len: usize| TextRun {
    len,
    font: base_font.clone(),
    color: default_color,
    background_color: None,
    underline: None,
    strikethrough: None,
  };
  let mut runs = Vec::new();
  let mut pos = 0usize;
  for span in &line.spans {
    if span.range.start > pos {
      runs.push(default_run(span.range.start - pos));
    }
    let style = span.style;
    let mut font = base_font.clone();
    if style.bold {
      font.weight = FontWeight::BOLD;
    }
    if style.italic {
      font.style = FontStyle::Italic;
    }
    let mut color = style
      .fg
      .map(|fg| ansi_color_to_hsla(fg, is_dark))
      .unwrap_or(default_color);
    if style.dim {
      color = color.opacity(0.65);
    }
    runs.push(TextRun {
      len: span.range.len(),
      font,
      color,
      background_color: style.bg.map(|bg| ansi_color_to_hsla(bg, is_dark)),
      underline: style.underline.then(|| UnderlineStyle {
        thickness: px(1.),
        color: Some(color),
        wavy: false,
      }),
      strikethrough: None,
    });
    pos = span.range.end;
  }
  if pos < line.text.len() {
    runs.push(default_run(line.text.len() - pos));
  }
  runs
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn runs_cover_the_whole_line_with_default_gaps() {
    let line = &parse_ansi("ab\u{1b}[31mcd\u{1b}[0mef")[0];
    let font = Font {
      family: "monospace".into(),
      ..Default::default()
    };
    let runs = runs_for_line(line, &font, gpui::black(), true);
    let total: usize = runs.iter().map(|r| r.len).sum();
    assert_eq!(total, line.text.len());
    assert_eq!(runs.len(), 3);
    assert_ne!(runs[1].color, runs[0].color, "the styled middle differs");
  }
}
