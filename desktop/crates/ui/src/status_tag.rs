use gpui::{
  AnyElement, App, Hsla, IntoElement, ParentElement, RenderOnce, StyleRefinement, Styled, Window,
};
use gpui_component::{Sizable as _, StyledExt as _, tag::Tag};

const STATUS_TAG_BACKGROUND_OPACITY: f32 = 0.12;

#[derive(Clone, Copy, Debug, PartialEq)]
struct StatusTagPalette {
  background: Hsla,
  foreground: Hsla,
  border: Hsla,
}

fn status_tag_palette(color: Hsla) -> StatusTagPalette {
  StatusTagPalette {
    background: color.opacity(STATUS_TAG_BACKGROUND_OPACITY),
    foreground: color,
    border: color,
  }
}

#[derive(IntoElement)]
pub struct StatusTag {
  color: Hsla,
  children: Vec<AnyElement>,
  style: StyleRefinement,
}

impl StatusTag {
  pub fn new(color: Hsla) -> Self {
    Self {
      color,
      children: Vec::new(),
      style: StyleRefinement::default(),
    }
  }
}

impl ParentElement for StatusTag {
  fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
    self.children.extend(elements);
  }
}

impl Styled for StatusTag {
  fn style(&mut self) -> &mut StyleRefinement {
    &mut self.style
  }
}

impl RenderOnce for StatusTag {
  fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
    let palette = status_tag_palette(self.color);
    let mut tag = Tag::custom(palette.background, palette.foreground, palette.border)
      .small()
      .rounded_full()
      .refine_style(&self.style);
    tag.extend(self.children);
    tag
  }
}

#[cfg(test)]
mod tests {
  use super::{STATUS_TAG_BACKGROUND_OPACITY, status_tag_palette};
  use gpui::Hsla;

  #[test]
  fn status_tag_palette_uses_full_color_for_text_and_border() {
    let color = Hsla {
      h: 140.0 / 360.0,
      s: 0.7,
      l: 0.4,
      a: 1.0,
    };

    let palette = status_tag_palette(color);

    assert_eq!(palette.foreground, color);
    assert_eq!(palette.border, color);
    assert_eq!(palette.background.h, color.h);
    assert_eq!(palette.background.s, color.s);
    assert_eq!(palette.background.l, color.l);
    assert_eq!(palette.background.a, STATUS_TAG_BACKGROUND_OPACITY);
  }
}
