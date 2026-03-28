use gpui::Hsla;
use gpui_component::blue_600;

pub trait StatusThemeExt {
  fn status_orange(&self) -> Hsla;
  fn status_yellow(&self) -> Hsla;
  fn status_blue(&self) -> Hsla;
  fn status_green(&self) -> Hsla;
  fn status_red(&self) -> Hsla;
  fn status_violet(&self) -> Hsla;
  fn status_gray(&self) -> Hsla;
}

impl StatusThemeExt for gpui_component::Theme {
  fn status_orange(&self) -> Hsla {
    if self.mode.is_dark() {
      Hsla {
        h: 30.0 / 360.0,
        s: 0.85,
        l: 0.58,
        a: 1.0,
      }
    } else {
      Hsla {
        h: 28.0 / 360.0,
        s: 0.9,
        l: 0.45,
        a: 1.0,
      }
    }
  }

  fn status_yellow(&self) -> Hsla {
    if self.mode.is_dark() {
      Hsla {
        h: 48.0 / 360.0,
        s: 0.9,
        l: 0.62,
        a: 1.0,
      }
    } else {
      Hsla {
        h: 46.0 / 360.0,
        s: 0.9,
        l: 0.42,
        a: 1.0,
      }
    }
  }

  fn status_blue(&self) -> Hsla {
    blue_600()
  }

  fn status_green(&self) -> Hsla {
    if self.mode.is_dark() {
      Hsla {
        h: 135.0 / 360.0,
        s: 0.75,
        l: 0.55,
        a: 1.0,
      }
    } else {
      Hsla {
        h: 140.0 / 360.0,
        s: 0.7,
        l: 0.4,
        a: 1.0,
      }
    }
  }

  fn status_red(&self) -> Hsla {
    if self.mode.is_dark() {
      Hsla {
        h: 0.0,
        s: 0.75,
        l: 0.58,
        a: 1.0,
      }
    } else {
      self.danger
    }
  }

  fn status_violet(&self) -> Hsla {
    if self.mode.is_dark() {
      Hsla {
        h: 270.0 / 360.0,
        s: 0.6,
        l: 0.58,
        a: 1.0,
      }
    } else {
      Hsla {
        h: 265.0 / 360.0,
        s: 0.6,
        l: 0.45,
        a: 1.0,
      }
    }
  }

  fn status_gray(&self) -> Hsla {
    if self.mode.is_dark() {
      Hsla {
        h: 0.0,
        s: 0.0,
        l: 0.55,
        a: 1.0,
      }
    } else {
      Hsla {
        h: 0.0,
        s: 0.0,
        l: 0.4,
        a: 1.0,
      }
    }
  }
}
