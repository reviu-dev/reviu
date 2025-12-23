mod diff_view;
mod main_view;

pub use diff_view::DiffView;
pub use main_view::MainView;

use gpui::{black, rgb, white};

/// Color palette for the application
pub struct Colors;

impl Colors {
  pub fn success() -> gpui::Hsla {
    rgb(0x00c951).into()
  }

  pub fn destructive() -> gpui::Hsla {
    rgb(0xfb2c36).into()
  }

  // Background colors
  pub fn bg_primary() -> gpui::Hsla {
    black()
  }

  pub fn bg_secondary() -> gpui::Hsla {
    let mut color: gpui::Hsla = white();
    color.a = 0.05;
    color
  }

  // Text colors
  pub fn text_primary() -> gpui::Hsla {
    white()
  }

  pub fn text_muted() -> gpui::Hsla {
    let mut color: gpui::Hsla = Colors::text_primary();
    color.a = 0.7;
    color
  }

  // Diff colors
  pub fn diff_addition_bg() -> gpui::Hsla {
    let mut color: gpui::Hsla = Self::success();
    color.a = 0.20;
    color
  }

  pub fn diff_deletion_bg() -> gpui::Hsla {
    let mut color: gpui::Hsla = Self::destructive();
    color.a = 0.20;
    color
  }

  // Status colors
  pub fn status_modified() -> gpui::Hsla {
    rgb(0xfbbf24).into()
  }

  pub fn status_added() -> gpui::Hsla {
    rgb(0x4ade80).into()
  }

  pub fn status_deleted() -> gpui::Hsla {
    rgb(0xf87171).into()
  }

  pub fn status_untracked() -> gpui::Hsla {
    rgb(0x60a5fa).into()
  }

  // Border colors
  pub fn border_primary() -> gpui::Hsla {
    let mut color: gpui::Hsla = white();
    color.a = 0.30;
    color
  }

  pub fn border_focus() -> gpui::Hsla {
    rgb(0x0078d4).into()
  }

  // Interactive colors
  pub fn hover() -> gpui::Hsla {
    let mut color: gpui::Hsla = white();
    color.a = 0.10;
    color
  }

  pub fn active() -> gpui::Hsla {
    let mut color: gpui::Hsla = white();
    color.a = 0.30;
    color
  }

  pub fn selected() -> gpui::Hsla {
    rgb(0x094771).into()
  }
}
