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
}

impl Default for Theme {
  fn default() -> Self {
    Self::dark()
  }
}
