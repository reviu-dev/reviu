use gpui::{
  AnyElement, App, Hsla, IntoElement, ParentElement, Pixels, RenderOnce, StyleRefinement, Styled,
  Window, px,
};
use gpui_component::{Sizable as _, Size, StyledExt as _, tag::Tag};

use crate::status_surface::{
  StatusSurfacePalette, filled_status_surface_palette, tinted_status_surface_palette,
};

const STATUS_TAG_XSMALL_TEXT_SIZE: Pixels = px(10.);
const STATUS_TAG_XSMALL_PADDING_X: Pixels = px(4.);
const STATUS_TAG_XSMALL_PADDING_Y: Pixels = px(1.);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StatusTagVariant {
  Filled,
  Outline,
}

fn status_tag_palette(color: Hsla, variant: StatusTagVariant) -> StatusSurfacePalette {
  match variant {
    StatusTagVariant::Filled => filled_status_surface_palette(color),
    StatusTagVariant::Outline => tinted_status_surface_palette(color),
  }
}

fn status_tag_text_size_override(size: Size) -> Option<Pixels> {
  match size {
    Size::XSmall => Some(STATUS_TAG_XSMALL_TEXT_SIZE),
    _ => None,
  }
}

fn status_tag_padding_x_override(size: Size) -> Option<Pixels> {
  match size {
    Size::XSmall => Some(STATUS_TAG_XSMALL_PADDING_X),
    _ => None,
  }
}

fn status_tag_padding_y_override(size: Size) -> Option<Pixels> {
  match size {
    Size::XSmall => Some(STATUS_TAG_XSMALL_PADDING_Y),
    _ => None,
  }
}

#[derive(IntoElement)]
pub struct StatusTag {
  color: Hsla,
  children: Vec<AnyElement>,
  size: Size,
  variant: StatusTagVariant,
  style: StyleRefinement,
}

impl StatusTag {
  pub fn new(color: Hsla) -> Self {
    Self {
      color,
      children: Vec::new(),
      size: Size::Small,
      variant: StatusTagVariant::Filled,
      style: StyleRefinement::default(),
    }
  }

  pub fn outline(mut self) -> Self {
    self.variant = StatusTagVariant::Outline;
    self
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

impl gpui_component::Sizable for StatusTag {
  fn with_size(mut self, size: impl Into<Size>) -> Self {
    self.size = size.into();
    self
  }
}

impl RenderOnce for StatusTag {
  fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
    let palette = status_tag_palette(self.color, self.variant);
    let mut tag = Tag::custom(palette.background, palette.foreground, palette.border)
      .with_size(self.size)
      .rounded_full();
    if let Some(text_size) = status_tag_text_size_override(self.size) {
      tag = tag.text_size(text_size);
    }
    if let Some(padding_x) = status_tag_padding_x_override(self.size) {
      tag = tag.px(padding_x);
    }
    if let Some(padding_y) = status_tag_padding_y_override(self.size) {
      tag = tag.py(padding_y);
    }
    let mut tag = tag.refine_style(&self.style);
    tag.extend(self.children);
    tag
  }
}

#[cfg(test)]
mod tests {
  use super::{
    STATUS_TAG_XSMALL_PADDING_X, STATUS_TAG_XSMALL_PADDING_Y, STATUS_TAG_XSMALL_TEXT_SIZE,
    StatusTag, StatusTagVariant, status_tag_padding_x_override, status_tag_padding_y_override,
    status_tag_palette, status_tag_text_size_override,
  };
  use crate::status_surface::TINTED_STATUS_SURFACE_BACKGROUND_OPACITY;
  use gpui::{Hsla, black, px, white};
  use gpui_component::{Sizable as _, Size};

  #[test]
  fn status_tag_outline_palette_uses_current_style() {
    let color = Hsla {
      h: 140.0 / 360.0,
      s: 0.7,
      l: 0.4,
      a: 1.0,
    };

    let palette = status_tag_palette(color, StatusTagVariant::Outline);

    assert_eq!(palette.foreground, color);
    assert_eq!(palette.border, color);
    assert_eq!(palette.background.h, color.h);
    assert_eq!(palette.background.s, color.s);
    assert_eq!(palette.background.l, color.l);
    assert_eq!(
      palette.background.a,
      TINTED_STATUS_SURFACE_BACKGROUND_OPACITY
    );
  }

  #[test]
  fn status_tag_defaults_to_small_and_supports_xsmall() {
    let color = Hsla {
      h: 140.0 / 360.0,
      s: 0.7,
      l: 0.4,
      a: 1.0,
    };

    let tag = StatusTag::new(color);
    assert_eq!(tag.size, Size::Small);
    assert_eq!(tag.variant, StatusTagVariant::Filled);

    let compact_tag = StatusTag::new(color).xsmall();
    assert_eq!(compact_tag.size, Size::XSmall);
  }

  #[test]
  fn status_tag_xsmall_uses_compact_metrics() {
    assert_eq!(
      status_tag_text_size_override(Size::XSmall),
      Some(STATUS_TAG_XSMALL_TEXT_SIZE)
    );
    assert_eq!(
      status_tag_padding_x_override(Size::XSmall),
      Some(STATUS_TAG_XSMALL_PADDING_X)
    );
    assert_eq!(
      status_tag_padding_y_override(Size::XSmall),
      Some(STATUS_TAG_XSMALL_PADDING_Y)
    );
    assert_eq!(status_tag_text_size_override(Size::Small), None);
    assert_eq!(status_tag_padding_x_override(Size::Small), None);
    assert_eq!(status_tag_padding_y_override(Size::Small), None);
    assert_eq!(STATUS_TAG_XSMALL_TEXT_SIZE, px(10.));
  }

  #[test]
  fn status_tag_outline_builder_switches_variant() {
    let color = Hsla {
      h: 140.0 / 360.0,
      s: 0.7,
      l: 0.4,
      a: 1.0,
    };

    let tag = StatusTag::new(color).outline();
    assert_eq!(tag.variant, StatusTagVariant::Outline);
  }

  #[test]
  fn status_tag_filled_palette_uses_solid_background() {
    let dark_color = Hsla {
      h: 0.0,
      s: 0.75,
      l: 0.45,
      a: 1.0,
    };
    let light_color = Hsla {
      h: 48.0 / 360.0,
      s: 0.9,
      l: 0.72,
      a: 1.0,
    };

    let dark_palette = status_tag_palette(dark_color, StatusTagVariant::Filled);
    assert_eq!(dark_palette.background, dark_color);
    assert_eq!(dark_palette.border.a, 0.0);
    assert_eq!(dark_palette.foreground, white());

    let light_palette = status_tag_palette(light_color, StatusTagVariant::Filled);
    assert_eq!(light_palette.background, light_color);
    assert_eq!(light_palette.border.a, 0.0);
    assert_eq!(light_palette.foreground, black());
  }
}
