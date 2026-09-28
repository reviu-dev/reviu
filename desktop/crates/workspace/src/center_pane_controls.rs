use std::rc::Rc;

use gpui::{
  AnyElement, App, InteractiveElement as _, IntoElement, ParentElement, Styled as _, Window,
};
use gpui_component::{
  Sizable as _,
  button::{Button, ButtonVariants as _},
  h_flex,
  menu::{DropdownMenu as _, PopupMenuItem},
};

pub(crate) type CenterPaneCommand = Rc<dyn Fn(&mut Window, &mut App)>;
pub(crate) type CenterPaneEnabled = Rc<dyn Fn(&App) -> bool>;

#[derive(Clone)]
pub(crate) struct CenterPaneControlActions {
  pub(crate) visible: CenterPaneEnabled,
  pub(crate) move_left_enabled: CenterPaneEnabled,
  pub(crate) move_right_enabled: CenterPaneEnabled,
  pub(crate) move_up_enabled: CenterPaneEnabled,
  pub(crate) move_down_enabled: CenterPaneEnabled,
  pub(crate) move_left: CenterPaneCommand,
  pub(crate) move_right: CenterPaneCommand,
  pub(crate) move_up: CenterPaneCommand,
  pub(crate) move_down: CenterPaneCommand,
  pub(crate) separate: CenterPaneCommand,
  pub(crate) close: CenterPaneCommand,
}

pub(crate) struct CenterPaneControlIds {
  pub(crate) actions_id: String,
  pub(crate) actions_debug_selector: &'static str,
  pub(crate) close_id: String,
  pub(crate) close_debug_selector: &'static str,
}

pub(crate) fn render_center_pane_controls(
  actions: CenterPaneControlActions,
  ids: CenterPaneControlIds,
) -> AnyElement {
  let menu_actions = actions.clone();
  let close = actions.close.clone();
  h_flex()
    .items_center()
    .gap_1()
    .child(
      Button::new(ids.actions_id)
        .debug_selector(move || ids.actions_debug_selector.to_string())
        .icon(gpui_component::IconName::Ellipsis)
        .xsmall()
        .ghost()
        .dropdown_menu(move |menu, _, cx| {
          let move_left = menu_actions.move_left.clone();
          let move_right = menu_actions.move_right.clone();
          let move_up = menu_actions.move_up.clone();
          let move_down = menu_actions.move_down.clone();
          let separate = menu_actions.separate.clone();
          menu
            .item(
              PopupMenuItem::new("Move Left")
                .disabled(!(menu_actions.move_left_enabled)(cx))
                .on_click(move |_, window, cx| move_left(window, cx)),
            )
            .item(
              PopupMenuItem::new("Move Right")
                .disabled(!(menu_actions.move_right_enabled)(cx))
                .on_click(move |_, window, cx| move_right(window, cx)),
            )
            .item(
              PopupMenuItem::new("Move Up")
                .disabled(!(menu_actions.move_up_enabled)(cx))
                .on_click(move |_, window, cx| move_up(window, cx)),
            )
            .item(
              PopupMenuItem::new("Move Down")
                .disabled(!(menu_actions.move_down_enabled)(cx))
                .on_click(move |_, window, cx| move_down(window, cx)),
            )
            .separator()
            .item(
              PopupMenuItem::new("Separate Tab from Split")
                .on_click(move |_, window, cx| separate(window, cx)),
            )
        }),
    )
    .child(
      Button::new(ids.close_id)
        .debug_selector(move || ids.close_debug_selector.to_string())
        .icon(gpui_component::IconName::Close)
        .xsmall()
        .ghost()
        .on_click(move |_, window, cx| close(window, cx)),
    )
    .into_any_element()
}
