use gpui::Hsla;
use syntax::SyntaxTheme;

#[derive(Debug, Clone)]
pub struct Theme {
  pub is_dark: bool,
}

impl Theme {
  pub fn new(is_dark_mode: bool) -> Self {
    Self {
      is_dark: is_dark_mode,
    }
  }

  pub fn dark() -> Self {
    Self::new(true)
  }

  pub fn light() -> Self {
    Self::new(false)
  }

  pub fn toggle(&mut self) {
    self.is_dark = !self.is_dark;
  }

  pub fn syntax(&self) -> SyntaxTheme {
    if self.is_dark {
      SyntaxTheme::default_dark()
    } else {
      SyntaxTheme::default_light()
    }
  }

  pub fn cursor(&self) -> Hsla {
    Hsla {
      h: 210.0 / 360.0,
      s: 1.0,
      l: 0.5,
      a: 0.7,
    }
  }

  pub fn gutter_background(&self) -> Hsla {
    if self.is_dark {
      Hsla {
        h: 0.0,
        s: 0.0,
        l: 0.0,
        a: 1.0,
      }
    } else {
      Hsla {
        h: 0.0,
        s: 0.0,
        l: 1.0,
        a: 1.0,
      }
    }
  }

  pub fn line_number(&self) -> Hsla {
    if self.is_dark {
      Hsla {
        h: 0.0,
        s: 0.0,
        l: 0.53,
        a: 1.0,
      }
    } else {
      Hsla {
        h: 0.0,
        s: 0.0,
        l: 0.40,
        a: 1.0,
      }
    }
  }

  pub fn selection(&self) -> Hsla {
    if self.is_dark {
      Hsla {
        h: 210.0 / 360.0,
        s: 1.0,
        l: 0.55,
        a: 0.3,
      }
    } else {
      Hsla {
        h: 210.0 / 360.0,
        s: 1.0,
        l: 0.85,
        a: 0.4,
      }
    }
  }

  pub fn indent_rainbow_colors(&self) -> [Hsla; 6] {
    if self.is_dark {
      [
        Hsla {
          h: 212.0 / 360.0, // blue
          s: 0.62,
          l: 0.62,
          a: 0.14,
        },
        Hsla {
          h: 268.0 / 360.0, // violet
          s: 0.60,
          l: 0.64,
          a: 0.14,
        },
        Hsla {
          h: 198.0 / 360.0, // cyan
          s: 0.60,
          l: 0.60,
          a: 0.14,
        },
        Hsla {
          h: 246.0 / 360.0, // indigo
          s: 0.58,
          l: 0.63,
          a: 0.14,
        },
        Hsla {
          h: 48.0 / 360.0, // yellow
          s: 0.66,
          l: 0.64,
          a: 0.14,
        },
        Hsla {
          h: 286.0 / 360.0, // purple
          s: 0.58,
          l: 0.63,
          a: 0.14,
        },
      ]
    } else {
      [
        Hsla {
          h: 212.0 / 360.0, // blue
          s: 0.64,
          l: 0.46,
          a: 0.09,
        },
        Hsla {
          h: 268.0 / 360.0, // violet
          s: 0.62,
          l: 0.45,
          a: 0.09,
        },
        Hsla {
          h: 198.0 / 360.0, // cyan
          s: 0.62,
          l: 0.44,
          a: 0.09,
        },
        Hsla {
          h: 246.0 / 360.0, // indigo
          s: 0.60,
          l: 0.45,
          a: 0.09,
        },
        Hsla {
          h: 48.0 / 360.0, // yellow
          s: 0.72,
          l: 0.44,
          a: 0.09,
        },
        Hsla {
          h: 286.0 / 360.0, // purple
          s: 0.60,
          l: 0.45,
          a: 0.09,
        },
      ]
    }
  }

  pub fn diff_added_background(&self) -> Hsla {
    if self.is_dark {
      Hsla {
        h: 120.0 / 360.0,
        s: 0.35,
        l: 0.22,
        a: 0.6,
      }
    } else {
      Hsla {
        h: 120.0 / 360.0,
        s: 0.35,
        l: 0.9,
        a: 0.7,
      }
    }
  }

  pub fn diff_removed_background(&self) -> Hsla {
    if self.is_dark {
      Hsla {
        h: 0.0,
        s: 0.45,
        l: 0.22,
        a: 0.6,
      }
    } else {
      Hsla {
        h: 0.0,
        s: 0.45,
        l: 0.9,
        a: 0.7,
      }
    }
  }

  pub fn diff_word_added_background(&self) -> Hsla {
    if self.is_dark {
      Hsla {
        h: 120.0 / 360.0,
        s: 0.6,
        l: 0.28,
        a: 0.8,
      }
    } else {
      Hsla {
        h: 120.0 / 360.0,
        s: 0.55,
        l: 0.85,
        a: 0.7,
      }
    }
  }

  pub fn diff_word_removed_background(&self) -> Hsla {
    if self.is_dark {
      Hsla {
        h: 0.0,
        s: 0.65,
        l: 0.28,
        a: 0.8,
      }
    } else {
      Hsla {
        h: 0.0,
        s: 0.55,
        l: 0.85,
        a: 0.7,
      }
    }
  }

  pub fn diff_removed_text(&self) -> Hsla {
    if self.is_dark {
      Hsla {
        h: 0.0,
        s: 0.75,
        l: 0.62,
        a: 1.0,
      }
    } else {
      Hsla {
        h: 0.0,
        s: 0.65,
        l: 0.45,
        a: 1.0,
      }
    }
  }

  pub fn diff_added_staged_background(&self) -> Hsla {
    let mut color = self.diff_added_background();
    color.a = (color.a * 0.6).min(1.0);
    color
  }

  pub fn diff_removed_staged_background(&self) -> Hsla {
    let mut color = self.diff_removed_background();
    color.a = (color.a * 0.6).min(1.0);
    color
  }

  pub fn diff_gutter_added(&self) -> Hsla {
    Hsla {
      h: 120.0 / 360.0,
      s: 0.6,
      l: 0.45,
      a: 1.0,
    }
  }

  pub fn diff_gutter_removed(&self) -> Hsla {
    Hsla {
      h: 0.0,
      s: 0.7,
      l: 0.5,
      a: 1.0,
    }
  }

  pub fn diff_gutter_modified(&self) -> Hsla {
    Hsla {
      h: 30.0 / 360.0,
      s: 0.75,
      l: 0.5,
      a: 1.0,
    }
  }

  pub fn hunk_focused_border(&self) -> Hsla {
    Hsla {
      h: 210.0 / 360.0,
      s: 1.0,
      l: 0.5,
      a: 1.0,
    }
  }

  pub fn current_conflict_background(&self) -> Hsla {
    if self.is_dark {
      Hsla {
        h: 45.0 / 360.0,
        s: 0.55,
        l: 0.24,
        a: 0.55,
      }
    } else {
      Hsla {
        h: 45.0 / 360.0,
        s: 0.7,
        l: 0.88,
        a: 0.7,
      }
    }
  }

  pub fn current_conflict_stripe(&self) -> Hsla {
    if self.is_dark {
      Hsla {
        h: 42.0 / 360.0,
        s: 0.95,
        l: 0.55,
        a: 1.0,
      }
    } else {
      Hsla {
        h: 38.0 / 360.0,
        s: 0.95,
        l: 0.5,
        a: 1.0,
      }
    }
  }

  pub fn incoming_conflict_background(&self) -> Hsla {
    if self.is_dark {
      Hsla {
        h: 275.0 / 360.0,
        s: 0.45,
        l: 0.30,
        a: 0.55,
      }
    } else {
      Hsla {
        h: 275.0 / 360.0,
        s: 0.55,
        l: 0.9,
        a: 0.7,
      }
    }
  }

  pub fn incoming_conflict_stripe(&self) -> Hsla {
    if self.is_dark {
      Hsla {
        h: 275.0 / 360.0,
        s: 0.75,
        l: 0.65,
        a: 1.0,
      }
    } else {
      Hsla {
        h: 275.0 / 360.0,
        s: 0.7,
        l: 0.55,
        a: 1.0,
      }
    }
  }
}

impl Default for Theme {
  fn default() -> Self {
    Self::dark()
  }
}
