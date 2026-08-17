//! Telling someone about Reviu Pro at the one moment it is relevant: they just
//! pushed a branch to GitHub and cannot review it here.

use gpui::{AnyWindowHandle, App, AppContext as _, div, prelude::*};
use gpui_component::{Sizable as _, notification::Notification};
use ui::{Button, ButtonVariants as _, WindowExt as _};

use crate::analytics;
use crate::navigation::NavigationHistory;

const SOURCE: &str = "post_push_notification";
pub(crate) const PRO_TEASER_BUTTON_DEBUG_SELECTOR: &str = "pro-push-hint-open";

struct ProPushHintNotificationId;

/// Once per session, and only for someone who could actually gain something.
pub(crate) fn should_show_after_push(
  already_shown: bool,
  has_github_access: bool,
  has_github_remote: bool,
) -> bool {
  !already_shown && !has_github_access && has_github_remote
}

pub(crate) fn show_after_push(window_handle: AnyWindowHandle, cx: &mut App) {
  analytics::track_with(
    cx,
    "pro_teaser_shown",
    Some(serde_json::json!({ "source": SOURCE })),
  );
  let _ = cx.update_window(window_handle, move |_, window, cx| {
    window.push_notification(
      Notification::new()
        .id::<ProPushHintNotificationId>()
        .title("Review pull requests in Reviu")
        .message(
          "Reviu Pro brings GitHub pull requests, reviews, and notifications into the app. 14-day free trial.",
        )
        .content(move |_, _, _cx| {
          div()
            .flex()
            .mt_3()
            .child(
              Button::new("pro-push-hint-open")
                .debug_selector(|| PRO_TEASER_BUTTON_DEBUG_SELECTOR.to_string())
                .primary()
                .compact()
                .small()
                .label("See Reviu Pro")
                .on_click(move |_, window, cx| {
                  analytics::track_with(
                    cx,
                    "pro_teaser_clicked",
                    Some(serde_json::json!({ "source": SOURCE })),
                  );
                  NavigationHistory::navigate("/billing", cx);
                  window.on_next_frame(|window, cx| {
                    window.remove_notification::<ProPushHintNotificationId>(cx);
                  });
                }),
            )
            .into_any_element()
        }),
      cx,
    );
  });
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_teaser_waits_for_someone_it_can_help() {
    // Pushed to GitHub without Pro: the one case worth a word.
    assert!(should_show_after_push(false, false, true));

    assert!(
      !should_show_after_push(true, false, true),
      "once per session, not once per push"
    );
    assert!(
      !should_show_after_push(false, true, true),
      "a Pro user has nothing to buy"
    );
    assert!(
      !should_show_after_push(false, false, false),
      "no GitHub remote means Pro would change nothing"
    );
  }
}
