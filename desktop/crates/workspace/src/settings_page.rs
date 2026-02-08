use gpui::{App, Context, FocusHandle, Focusable, Render, Window, div, prelude::*, px};

use gpui_component::{
  ActiveTheme as _, IconName, Sizable, Size, Theme, ThemeMode,
  button::{Button, ButtonVariants},
  setting::{SettingField, SettingGroup, SettingItem, SettingPage, Settings},
};

use ui::HEADER_HEIGHT;

use crate::workspace::{WorkspacePage, WorkspaceRoute};

pub struct SettingsPage {
  focus_handle: FocusHandle,
  auto_switch_theme: bool,
  size: Size,
}

impl SettingsPage {
  pub fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
    Self {
      focus_handle: cx.focus_handle(),
      auto_switch_theme: false,
      size: Size::default(),
    }
  }

  fn setting_pages(&self, _: &mut Window, cx: &mut Context<Self>) -> Vec<SettingPage> {
    let view = cx.entity();
    let default_auto = self.auto_switch_theme;

    vec![SettingPage::new("General").default_open(true).groups(vec![
      SettingGroup::new().title("Appearance").items(vec![
        SettingItem::new(
          "Dark Mode",
          SettingField::switch(
            |cx: &App| cx.theme().mode.is_dark(),
            |val: bool, cx: &mut App| {
              let mode = if val {
                ThemeMode::Dark
              } else {
                ThemeMode::Light
              };
              Theme::change(mode, None, cx);
              cx.refresh_windows();
            },
          )
          .default_value(false),
        )
        .description("Switch between light and dark themes."),
        SettingItem::new(
          "Auto Switch Theme",
          SettingField::checkbox(
            {
              let view = view.clone();
              move |cx: &App| view.read(cx).auto_switch_theme
            },
            {
              let view = view.clone();
              move |val: bool, cx: &mut App| {
                view.update(cx, |view, _| {
                  view.auto_switch_theme = val;
                });
                if val {
                  Theme::sync_system_appearance(None, cx);
                }
                cx.refresh_windows();
              }
            },
          )
          .default_value(default_auto),
        )
        .description("Automatically switch theme based on system settings."),
      ]),
    ])]
  }
}

impl Render for SettingsPage {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    let header = div()
      .h(px(HEADER_HEIGHT))
      .max_h(px(HEADER_HEIGHT))
      .px_4()
      .flex()
      .items_center()
      .justify_between()
      .bg(theme.sidebar)
      .border_b_1()
      .border_color(theme.title_bar_border)
      .child(
        div()
          .text_sm()
          .text_color(theme.foreground)
          .child("Settings"),
      )
      .child(
        Button::new("close-settings")
          .icon(IconName::Close)
          .ghost()
          .compact()
          .tooltip("Close settings")
          .on_click(|_, _, cx| {
            WorkspaceRoute::global_mut(cx).page = WorkspacePage::Git;
            cx.refresh_windows();
          }),
      );

    div()
      .size_full()
      .flex()
      .flex_col()
      .bg(theme.background)
      .track_focus(&self.focus_handle(cx))
      .child(header)
      .child(
        Settings::new("app-settings")
          .with_size(self.size)
          .pages(self.setting_pages(window, cx)),
      )
  }
}

impl Focusable for SettingsPage {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}
