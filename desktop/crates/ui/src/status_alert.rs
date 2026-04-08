use gpui::{
  AnyElement, App, ElementId, Hsla, InteractiveElement, IntoElement, ParentElement, RenderOnce,
  SharedString, StyleRefinement, Styled, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{ActiveTheme as _, Icon, IconName, StyledExt as _, h_flex, v_flex};

use crate::status_surface::tinted_status_surface_palette;

#[derive(IntoElement)]
pub struct StatusAlert {
  id: ElementId,
  color: Hsla,
  icon: Icon,
  title: Option<SharedString>,
  message: AnyElement,
  action: Option<AnyElement>,
  style: StyleRefinement,
}

impl StatusAlert {
  pub fn new(id: impl Into<ElementId>, color: Hsla, message: impl IntoElement) -> Self {
    Self {
      id: id.into(),
      color,
      icon: Icon::new(IconName::TriangleAlert),
      title: None,
      message: message.into_any_element(),
      action: None,
      style: StyleRefinement::default(),
    }
  }

  pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
    self.icon = icon.into();
    self
  }

  pub fn title(mut self, title: impl Into<SharedString>) -> Self {
    self.title = Some(title.into());
    self
  }

  pub fn action(mut self, action: impl IntoElement) -> Self {
    self.action = Some(action.into_any_element());
    self
  }
}

impl Styled for StatusAlert {
  fn style(&mut self) -> &mut StyleRefinement {
    &mut self.style
  }
}

impl RenderOnce for StatusAlert {
  fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
    let palette = tinted_status_surface_palette(self.color);
    let title = self.title;
    let action = self.action;

    div()
      .id(self.id)
      .w_full()
      .flex()
      .items_start()
      .gap_3()
      .px_4()
      .py_3()
      .rounded(cx.theme().radius)
      .border_1()
      .border_color(palette.border)
      .bg(palette.background)
      .text_color(palette.foreground)
      .text_sm()
      .refine_style(&self.style)
      .child(
        h_flex()
          .w_full()
          .items_start()
          .gap_3()
          .child(div().mt(px(3.)).child(self.icon))
          .child(
            v_flex()
              .flex_1()
              .min_w_0()
              .gap_1()
              .when(title.is_some() || action.is_some(), |this| {
                this.child(
                  h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .when_some(title, |this, title| {
                      this.child(div().font_semibold().child(title))
                    })
                    .when_some(action, |this, action| this.child(action)),
                )
              })
              .child(div().min_w_0().whitespace_normal().child(self.message)),
          ),
      )
  }
}

#[cfg(test)]
mod tests {
  use super::StatusAlert;
  use crate::status_surface::{
    TINTED_STATUS_SURFACE_BACKGROUND_OPACITY, TINTED_STATUS_SURFACE_BORDER_OPACITY,
    tinted_status_surface_palette,
  };
  use gpui::Hsla;

  #[test]
  fn status_alert_uses_shared_tinted_surface_palette() {
    let color = Hsla {
      h: 30.0 / 360.0,
      s: 0.85,
      l: 0.58,
      a: 1.0,
    };

    let palette = tinted_status_surface_palette(color);

    assert_eq!(palette.foreground, color);
    assert_eq!(palette.border.a, TINTED_STATUS_SURFACE_BORDER_OPACITY);
    assert_eq!(
      palette.background.a,
      TINTED_STATUS_SURFACE_BACKGROUND_OPACITY
    );
  }

  #[test]
  fn status_alert_builder_tracks_optional_title_and_action() {
    let color = Hsla {
      h: 0.0,
      s: 0.75,
      l: 0.58,
      a: 1.0,
    };

    let alert = StatusAlert::new("status-alert", color, "Message")
      .title("Title")
      .action("Action");

    assert_eq!(
      alert.title.as_ref().map(|value| value.as_ref()),
      Some("Title")
    );
    assert!(alert.action.is_some());
  }
}
