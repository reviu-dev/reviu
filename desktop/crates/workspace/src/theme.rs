use gpui::{Rgba, rgb};
use syntax::Theme;

#[derive(Clone, Copy)]
pub struct AppColors {
  pub app_bg: Rgba,
  pub surface_bg: Rgba,
  pub sidebar_bg: Rgba,
  pub header_bg: Rgba,
  pub border: Rgba,
  pub text: Rgba,
  pub text_muted: Rgba,
  pub text_subtle: Rgba,
  pub button_bg: Rgba,
  pub button_border: Rgba,
  pub button_text: Rgba,
  pub menu_bg: Rgba,
  pub menu_hover_bg: Rgba,
  pub menu_selected_bg: Rgba,
  pub menu_selected_text: Rgba,
  pub list_bg: Rgba,
  pub list_hover_bg: Rgba,
  pub list_selected_bg: Rgba,
  pub list_text: Rgba,
  pub list_selected_text: Rgba,
  pub error_text: Rgba,
}

pub fn app_colors(theme: &Theme) -> AppColors {
  if theme.is_dark {
    AppColors {
      app_bg: rgb(0x141414),
      surface_bg: rgb(0x1b1b1b),
      sidebar_bg: rgb(0x181818),
      header_bg: rgb(0x1d1d1d),
      border: rgb(0x2a2a2a),
      text: rgb(0xe0e0e0),
      text_muted: rgb(0x808080),
      text_subtle: rgb(0x9a9a9a),
      button_bg: rgb(0x2a2a2a),
      button_border: rgb(0x3a3a3a),
      button_text: rgb(0xffffff),
      menu_bg: rgb(0x1c1c1c),
      menu_hover_bg: rgb(0x222222),
      menu_selected_bg: rgb(0x2a2a2a),
      menu_selected_text: rgb(0xffffff),
      list_bg: rgb(0x1a1a1a),
      list_hover_bg: rgb(0x222222),
      list_selected_bg: rgb(0x2a2a2a),
      list_text: rgb(0xcfcfcf),
      list_selected_text: rgb(0xffffff),
      error_text: rgb(0xcc6666),
    }
  } else {
    AppColors {
      app_bg: rgb(0xf2f2f2),
      surface_bg: rgb(0xffffff),
      sidebar_bg: rgb(0xf0f0f0),
      header_bg: rgb(0xe6e6e6),
      border: rgb(0xd0d0d0),
      text: rgb(0x222222),
      text_muted: rgb(0x666666),
      text_subtle: rgb(0x7a7a7a),
      button_bg: rgb(0xe0e0e0),
      button_border: rgb(0xc8c8c8),
      button_text: rgb(0x1a1a1a),
      menu_bg: rgb(0xf5f5f5),
      menu_hover_bg: rgb(0xe9e9e9),
      menu_selected_bg: rgb(0xdedede),
      menu_selected_text: rgb(0x111111),
      list_bg: rgb(0xf3f3f3),
      list_hover_bg: rgb(0xe9e9e9),
      list_selected_bg: rgb(0xdedede),
      list_text: rgb(0x333333),
      list_selected_text: rgb(0x111111),
      error_text: rgb(0xb24b4b),
    }
  }
}
