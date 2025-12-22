mod diff_view;
mod main_view;

pub use diff_view::DiffView;
pub use main_view::MainView;

use gpui::rgb;

/// Color palette for the application
pub struct Colors;

impl Colors {
  // Background colors
  pub fn bg_primary() -> gpui::Hsla {
    rgb(0x1e1e1e).into()
  }

  pub fn bg_secondary() -> gpui::Hsla {
    rgb(0x252525).into()
  }

  pub fn bg_tertiary() -> gpui::Hsla {
    rgb(0x2d2d2d).into()
  }

  // Text colors
  pub fn text_primary() -> gpui::Hsla {
    rgb(0xcccccc).into()
  }

  pub fn text_secondary() -> gpui::Hsla {
    rgb(0x999999).into()
  }

  pub fn text_muted() -> gpui::Hsla {
    rgb(0x666666).into()
  }

  // Diff colors
  pub fn diff_addition_bg() -> gpui::Hsla {
    rgb(0x1a3a1a).into()
  }

  pub fn diff_addition_fg() -> gpui::Hsla {
    rgb(0x4ade80).into()
  }

  pub fn diff_deletion_bg() -> gpui::Hsla {
    rgb(0x3a1a1a).into()
  }

  pub fn diff_deletion_fg() -> gpui::Hsla {
    rgb(0xf87171).into()
  }

  pub fn diff_context_bg() -> gpui::Hsla {
    rgb(0x2a2a2a).into()
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
    rgb(0x3e3e3e).into()
  }

  pub fn border_focus() -> gpui::Hsla {
    rgb(0x0078d4).into()
  }

  // Interactive colors
  pub fn hover() -> gpui::Hsla {
    rgb(0x2a2a2a).into()
  }

  pub fn active() -> gpui::Hsla {
    rgb(0x37373d).into()
  }

  pub fn selected() -> gpui::Hsla {
    rgb(0x094771).into()
  }
}
