use gpui::{Hsla, black, white};

pub(crate) const TINTED_STATUS_SURFACE_BORDER_OPACITY: f32 = 1.0;
pub(crate) const TINTED_STATUS_SURFACE_BACKGROUND_OPACITY: f32 = 0.12;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StatusSurfacePalette {
  pub(crate) background: Hsla,
  pub(crate) foreground: Hsla,
  pub(crate) border: Hsla,
}

pub(crate) fn filled_status_surface_palette(color: Hsla) -> StatusSurfacePalette {
  StatusSurfacePalette {
    background: color,
    foreground: if color.l > 0.62 { black() } else { white() },
    border: color.opacity(0.0),
  }
}

pub(crate) fn tinted_status_surface_palette(color: Hsla) -> StatusSurfacePalette {
  StatusSurfacePalette {
    background: color.opacity(TINTED_STATUS_SURFACE_BACKGROUND_OPACITY),
    foreground: color,
    border: color.opacity(TINTED_STATUS_SURFACE_BORDER_OPACITY),
  }
}
