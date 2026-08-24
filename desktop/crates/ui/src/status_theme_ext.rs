use gpui::Hsla;
use gpui_component::ColorName;

// `Tag` puts its foreground on a tint of its own colour and can afford scale 300;
// these land on the app background, where that shade reads washed out.
const STATUS_DARK_SCALE: usize = 400;
const STATUS_LIGHT_SCALE: usize = 600;

pub trait StatusThemeExt {
  fn status_color(&self, color: ColorName) -> Hsla;
  fn status_orange(&self) -> Hsla;
  fn status_amber(&self) -> Hsla;
  fn status_blue(&self) -> Hsla;
  fn status_green(&self) -> Hsla;
  fn status_red(&self) -> Hsla;
  fn status_violet(&self) -> Hsla;
  fn status_gray(&self) -> Hsla;
}

impl StatusThemeExt for gpui_component::Theme {
  fn status_color(&self, color: ColorName) -> Hsla {
    color.scale(if self.mode.is_dark() {
      STATUS_DARK_SCALE
    } else {
      STATUS_LIGHT_SCALE
    })
  }

  fn status_orange(&self) -> Hsla {
    self.status_color(ColorName::Orange)
  }

  fn status_amber(&self) -> Hsla {
    self.status_color(ColorName::Amber)
  }

  fn status_blue(&self) -> Hsla {
    self.status_color(ColorName::Blue)
  }

  fn status_green(&self) -> Hsla {
    self.status_color(ColorName::Green)
  }

  fn status_red(&self) -> Hsla {
    self.status_color(ColorName::Red)
  }

  fn status_violet(&self) -> Hsla {
    self.status_color(ColorName::Violet)
  }

  fn status_gray(&self) -> Hsla {
    self.status_color(ColorName::Gray)
  }
}

#[cfg(test)]
mod tests {
  use super::{STATUS_DARK_SCALE, STATUS_LIGHT_SCALE, StatusThemeExt as _};
  use gpui::TestAppContext;
  use gpui_component::{ActiveTheme as _, ColorName, Theme, ThemeMode};

  #[gpui::test]
  fn status_colors_follow_the_shared_color_scales(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);

    for (mode, scale) in [
      (ThemeMode::Dark, STATUS_DARK_SCALE),
      (ThemeMode::Light, STATUS_LIGHT_SCALE),
    ] {
      cx.update(|cx| {
        Theme::change(mode, None, cx);
        let theme = cx.theme();

        assert_eq!(theme.status_orange(), ColorName::Orange.scale(scale));
        assert_eq!(theme.status_amber(), ColorName::Amber.scale(scale));
        assert_eq!(theme.status_blue(), ColorName::Blue.scale(scale));
        assert_eq!(theme.status_green(), ColorName::Green.scale(scale));
        assert_eq!(theme.status_red(), ColorName::Red.scale(scale));
        assert_eq!(theme.status_violet(), ColorName::Violet.scale(scale));
        assert_eq!(theme.status_gray(), ColorName::Gray.scale(scale));
      });
    }
  }

  #[gpui::test]
  fn dark_and_light_pick_different_shades(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);

    let dark = cx.update(|cx| {
      Theme::change(ThemeMode::Dark, None, cx);
      cx.theme().status_violet()
    });
    let light = cx.update(|cx| {
      Theme::change(ThemeMode::Light, None, cx);
      cx.theme().status_violet()
    });

    assert_ne!(dark, light);
    assert!(dark.l > light.l);
  }
}
