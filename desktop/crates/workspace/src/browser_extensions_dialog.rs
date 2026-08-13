use gpui::{App, Window, div, prelude::*, px};
use gpui_component::{
  ActiveTheme, Icon, button::Button, dialog::DialogButtonProps, h_flex, v_flex,
};
use ui::{IconName, UiIconName, WindowExt};

const FIREFOX_EXTENSION_URL: &str =
  "https://addons.mozilla.org/en-US/firefox/addon/reviu-open-in-app/";
const CHROME_EXTENSION_URL: &str =
  "https://chromewebstore.google.com/detail/ofifncflkbaboknlejdkifijpdkhheac";

pub fn open_browser_extensions_dialog(window: &mut Window, _cx: &mut App) {
  window.on_next_frame(move |window, cx| {
    open_browser_extensions_dialog_inner(window, cx);
  });
}

fn open_browser_extensions_dialog_inner(window: &mut Window, cx: &mut App) {
  window.open_alert_dialog(cx, move |alert, _, cx| {
    let theme = cx.theme().clone();

    alert
      .title("Browser extension")
      .description("Open any GitHub pull request in Reviu directly from your browser.")
      .child(
        v_flex()
          .gap_2()
          .pt_2()
          .child(extension_button(
            "browser-extensions-firefox",
            UiIconName::FirefoxBrowser,
            "Install for Firefox",
            FIREFOX_EXTENSION_URL,
            &theme,
          ))
          .child(extension_button(
            "browser-extensions-chrome",
            UiIconName::GoogleChrome,
            "Install for Chrome",
            CHROME_EXTENSION_URL,
            &theme,
          )),
      )
      .show_cancel(false)
      .button_props(DialogButtonProps::default().ok_text("Close"))
  });
}

fn extension_button(
  id: &'static str,
  icon: UiIconName,
  label: &'static str,
  url: &'static str,
  theme: &gpui_component::Theme,
) -> impl IntoElement {
  Button::new(id)
    .outline()
    .cursor_pointer()
    .child(
      h_flex()
        .w_full()
        .items_center()
        .gap_3()
        .child(Icon::new(icon).size_5().text_color(theme.foreground))
        .child(div().flex_1().text_left().child(label))
        .child(
          Icon::new(IconName::ExternalLink)
            .size_3()
            .text_color(theme.muted_foreground),
        ),
    )
    .h(px(44.0))
    .on_click(move |_, _, cx| {
      cx.open_url(url);
    })
}
