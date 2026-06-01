use alacritty_terminal::{
  term::color::Colors,
  vte::ansi::{Color, NamedColor, Rgb},
};
use gpui::{Hsla, rgb};

#[derive(Clone, Debug)]
pub struct TerminalPalette {
  ansi_colors: [Hsla; 16],
  indexed_colors: [Hsla; 256],
  foreground: Hsla,
  background: Hsla,
  cursor: Hsla,
  selection: Hsla,
}

impl Default for TerminalPalette {
  fn default() -> Self {
    let ansi_colors = [
      hsla_from_rgb(Rgb {
        r: 0x1d,
        g: 0x20,
        b: 0x2f,
      }),
      hsla_from_rgb(Rgb {
        r: 0xf7,
        g: 0x76,
        b: 0x8e,
      }),
      hsla_from_rgb(Rgb {
        r: 0x9e,
        g: 0xce,
        b: 0x6a,
      }),
      hsla_from_rgb(Rgb {
        r: 0xe0,
        g: 0xaf,
        b: 0x68,
      }),
      hsla_from_rgb(Rgb {
        r: 0x7a,
        g: 0xa2,
        b: 0xf7,
      }),
      hsla_from_rgb(Rgb {
        r: 0xbb,
        g: 0x9a,
        b: 0xf7,
      }),
      hsla_from_rgb(Rgb {
        r: 0x7d,
        g: 0xcf,
        b: 0xff,
      }),
      hsla_from_rgb(Rgb {
        r: 0xa9,
        g: 0xb1,
        b: 0xd6,
      }),
      hsla_from_rgb(Rgb {
        r: 0x41,
        g: 0x48,
        b: 0x68,
      }),
      hsla_from_rgb(Rgb {
        r: 0xff,
        g: 0x9e,
        b: 0xb1,
      }),
      hsla_from_rgb(Rgb {
        r: 0xb9,
        g: 0xf2,
        b: 0x7c,
      }),
      hsla_from_rgb(Rgb {
        r: 0xff,
        g: 0xc7,
        b: 0x77,
      }),
      hsla_from_rgb(Rgb {
        r: 0x8a,
        g: 0xb5,
        b: 0xff,
      }),
      hsla_from_rgb(Rgb {
        r: 0xc8,
        g: 0xa6,
        b: 0xff,
      }),
      hsla_from_rgb(Rgb {
        r: 0x9c,
        g: 0xe2,
        b: 0xff,
      }),
      hsla_from_rgb(Rgb {
        r: 0xc0,
        g: 0xca,
        b: 0xf5,
      }),
    ];

    let mut indexed_colors = [Hsla::default(); 256];
    indexed_colors[..16].copy_from_slice(&ansi_colors);

    let mut index = 16usize;
    for r in 0..6 {
      for g in 0..6 {
        for b in 0..6 {
          indexed_colors[index] = hsla_from_rgb(Rgb {
            r: if r == 0 { 0 } else { 55 + r * 40 },
            g: if g == 0 { 0 } else { 55 + g * 40 },
            b: if b == 0 { 0 } else { 55 + b * 40 },
          });
          index += 1;
        }
      }
    }

    for step in 0..24 {
      let value = 8 + step * 10;
      indexed_colors[index] = hsla_from_rgb(Rgb {
        r: value,
        g: value,
        b: value,
      });
      index += 1;
    }

    Self {
      ansi_colors,
      indexed_colors,
      foreground: rgb(0xc0caf5).into(),
      background: rgb(0x1a1b26).into(),
      cursor: rgb(0x7aa2f7).into(),
      selection: rgb(0x33467a).into(),
    }
  }
}

impl TerminalPalette {
  pub fn themed(background: Hsla, foreground: Hsla, cursor: Hsla, selection: Hsla) -> Self {
    Self {
      background,
      foreground,
      cursor,
      selection,
      ..Self::default()
    }
  }

  pub fn background(&self) -> Hsla {
    self.background
  }

  pub fn cursor(&self) -> Hsla {
    self.cursor
  }

  pub fn selection(&self) -> Hsla {
    self.selection
  }

  pub fn resolve(&self, color: Color, colors: &Colors) -> Hsla {
    match color {
      Color::Named(named) => {
        if let Some(override_rgb) = colors[named] {
          return hsla_from_rgb(override_rgb);
        }

        match named as usize {
          index @ 0..=15 => self.ansi_colors[index],
          _ => match named {
            NamedColor::Foreground => self.foreground,
            NamedColor::Background => self.background,
            NamedColor::Cursor => self.cursor,
            NamedColor::BrightForeground => brighten(self.foreground, 1.18),
            NamedColor::DimForeground => dim(self.foreground),
            NamedColor::DimBlack => dim(self.ansi_colors[0]),
            NamedColor::DimRed => dim(self.ansi_colors[1]),
            NamedColor::DimGreen => dim(self.ansi_colors[2]),
            NamedColor::DimYellow => dim(self.ansi_colors[3]),
            NamedColor::DimBlue => dim(self.ansi_colors[4]),
            NamedColor::DimMagenta => dim(self.ansi_colors[5]),
            NamedColor::DimCyan => dim(self.ansi_colors[6]),
            NamedColor::DimWhite => dim(self.ansi_colors[7]),
            _ => self.foreground,
          },
        }
      }
      Color::Indexed(index) => self.indexed_colors[index as usize],
      Color::Spec(rgb) => hsla_from_rgb(rgb),
    }
  }
}

fn hsla_from_rgb(rgb_value: Rgb) -> Hsla {
  gpui::Rgba {
    r: rgb_value.r as f32 / 255.0,
    g: rgb_value.g as f32 / 255.0,
    b: rgb_value.b as f32 / 255.0,
    a: 1.0,
  }
  .into()
}

fn brighten(mut color: Hsla, amount: f32) -> Hsla {
  color.l = (color.l * amount).min(1.0);
  color
}

fn dim(mut color: Hsla) -> Hsla {
  color.l *= 0.75;
  color
}
