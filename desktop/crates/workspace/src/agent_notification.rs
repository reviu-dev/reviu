//! Borderless always-on-top toast for agent attention while the main window
//! is inactive; a native OS notification would need app signing on macOS.

use gpui::{
  Context, EventEmitter, InteractiveElement as _, IntoElement, ParentElement as _, PlatformDisplay,
  Render, SharedString, Size, StatefulInteractiveElement as _, Styled as _, Window,
  WindowBackgroundAppearance, WindowBounds, WindowDecorations, WindowKind, WindowOptions, div,
  point, px,
};
use gpui_component::{ActiveTheme as _, Sizable as _, StyledExt as _, h_flex, v_flex};
use std::rc::Rc;

pub const AGENT_NOTIFICATION_SIZE: Size<gpui::Pixels> = Size {
  width: px(380.),
  height: px(64.),
};

pub struct AgentNotification {
  icon: gpui_component::Icon,
  title: SharedString,
  caption: SharedString,
}

pub enum AgentNotificationEvent {
  Accepted,
  Dismissed,
}

impl EventEmitter<AgentNotificationEvent> for AgentNotification {}

impl AgentNotification {
  pub fn new(
    icon: gpui_component::Icon,
    title: impl Into<SharedString>,
    caption: impl Into<SharedString>,
  ) -> Self {
    Self {
      icon,
      title: title.into(),
      caption: caption.into(),
    }
  }

  pub fn window_options(screen: Rc<dyn PlatformDisplay>) -> WindowOptions {
    let margin = px(16.);
    let bounds = gpui::Bounds {
      origin: screen.bounds().top_right() - point(AGENT_NOTIFICATION_SIZE.width + margin, -margin),
      size: AGENT_NOTIFICATION_SIZE,
    };
    WindowOptions {
      window_bounds: Some(WindowBounds::Windowed(bounds)),
      titlebar: None,
      focus: false,
      show: true,
      kind: WindowKind::PopUp,
      is_movable: false,
      display_id: Some(screen.id()),
      window_background: WindowBackgroundAppearance::Transparent,
      window_decorations: Some(WindowDecorations::Client),
      ..Default::default()
    }
  }
}

impl Render for AgentNotification {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme();
    h_flex()
      .id("agent-notification")
      .size_full()
      .gap_2()
      .items_center()
      .px_3()
      .rounded(theme.radius_lg)
      .border_1()
      .border_color(theme.border)
      .bg(theme.popover)
      .shadow_lg()
      .cursor_pointer()
      .on_click(cx.listener(|_, _, _, cx| cx.emit(AgentNotificationEvent::Accepted)))
      .child(self.icon.clone().small().text_color(theme.foreground))
      .child(
        v_flex()
          .flex_1()
          .min_w_0()
          .child(
            div()
              .text_sm()
              .font_semibold()
              .text_color(theme.popover_foreground)
              .truncate()
              .child(self.title.clone()),
          )
          .child(
            div()
              .text_xs()
              .text_color(theme.muted_foreground)
              .truncate()
              .child(self.caption.clone()),
          ),
      )
      .child(
        div()
          .id("agent-notification-dismiss")
          .p_1()
          .rounded(theme.radius)
          .text_color(theme.muted_foreground)
          .hover(|s| s.bg(theme.secondary_hover))
          .on_click(cx.listener(|_, _, _, cx| {
            cx.stop_propagation();
            cx.emit(AgentNotificationEvent::Dismissed);
          }))
          .child(gpui_component::Icon::new(gpui_component::IconName::Close).small()),
      )
  }
}
