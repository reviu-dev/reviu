use gpui::prelude::*;
use gpui_component::{Sizable as _, tag::Tag};

use crate::StatusThemeExt;

pub fn pr_status_tag(
  theme: &gpui_component::Theme,
  state: &str,
  draft: bool,
  merged_at: Option<&str>,
) -> Tag {
  if merged_at.is_some() {
    Tag::custom(
      theme.status_violet(),
      theme.primary_foreground,
      theme.status_violet(),
    )
    .small()
    .rounded_full()
    .child("Merged")
  } else if draft {
    Tag::custom(
      theme.status_gray(),
      theme.primary_foreground,
      theme.status_gray(),
    )
    .small()
    .rounded_full()
    .child("Draft")
  } else if state == "open" {
    Tag::success().small().rounded_full().child("Open")
  } else {
    Tag::custom(theme.status_red(), theme.primary_foreground, theme.status_red())
      .small()
      .rounded_full()
      .child("Closed")
  }
}
