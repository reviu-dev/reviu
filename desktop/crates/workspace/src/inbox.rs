//! GitHub inbox in the sessions sidebar: open a notification, mark it done.

use gpui::{Context, IntoElement, Render, SharedString, Window, div, prelude::*, px};
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use ui::{Button, ButtonVariants as _, UiIconName};

use crate::date_format::format_relative_time;
use crate::github_notifications::{self, GithubNotificationsStore};

const INBOX_MAX_HEIGHT: f32 = 220.0;

pub struct Inbox {
  scroll_handle: gpui::ScrollHandle,
}

impl Inbox {
  pub fn new() -> Self {
    Self {
      scroll_handle: gpui::ScrollHandle::new(),
    }
  }
}

impl Render for Inbox {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let notifications = GithubNotificationsStore::list(cx);
    let unread = GithubNotificationsStore::unread_count(cx);

    let header = h_flex()
      .items_center()
      .gap_2()
      .px_3()
      .py_1()
      .child(
        div()
          .flex_1()
          .text_xs()
          .font_weight(gpui::FontWeight::SEMIBOLD)
          .text_color(theme.muted_foreground)
          .child("GitHub inbox"),
      )
      .when(unread > 0, |this| {
        this.child(
          div()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(unread.to_string()),
        )
      });

    let rows: Vec<_> = notifications
      .into_iter()
      .enumerate()
      .map(|(ix, notification)| {
        let group_name = SharedString::from(format!("inbox-row-{}", notification.id));
        let done_id = notification.id.clone();
        let time = format_relative_time(&notification.updated_at);
        let repo = notification.repository.full_name.clone();
        let title = notification.subject.title.clone();
        let is_unread = notification.unread;

        div()
          .id(("session-page-inbox-row", ix))
          .group(group_name.clone())
          .mx_2()
          .px_2()
          .py_1p5()
          .rounded(px(6.0))
          .cursor_pointer()
          .hover(|s| s.bg(theme.secondary_hover))
          .on_click(cx.listener(move |_, _, _, cx| {
            github_notifications::open_notification(&notification, cx);
          }))
          .child(
            v_flex()
              .gap_0p5()
              .child(
                h_flex()
                  .items_center()
                  .gap_2()
                  .when(is_unread, |this| {
                    this.child(
                      div()
                        .flex_shrink_0()
                        .size(px(6.0))
                        .rounded_full()
                        .bg(theme.primary),
                    )
                  })
                  .child(
                    div()
                      .flex_1()
                      .min_w(px(0.0))
                      .text_sm()
                      .truncate()
                      .text_color(theme.foreground)
                      .child(title),
                  )
                  .child(
                    Button::new(("session-page-inbox-done", ix))
                      .icon(UiIconName::Check)
                      .xsmall()
                      .ghost()
                      .opacity(0.0)
                      .group_hover(group_name.clone(), |this| this.opacity(1.0))
                      .tooltip("Mark as done")
                      .on_click(cx.listener(move |_, _, _, cx| {
                        cx.stop_propagation();
                        github_notifications::mark_notification_done(done_id.clone(), cx);
                      })),
                  ),
              )
              .child(
                h_flex()
                  .items_center()
                  .gap_2()
                  .text_xs()
                  .text_color(theme.muted_foreground)
                  .child(div().flex_1().min_w(px(0.0)).truncate().child(repo))
                  .child(div().child(time)),
              ),
          )
      })
      .collect();

    let body = if rows.is_empty() {
      div()
        .px_3()
        .py_2()
        .text_xs()
        .text_color(theme.muted_foreground)
        .child("No notifications")
        .into_any_element()
    } else {
      div()
        .relative()
        .child(
          div()
            .id("session-page-inbox-list")
            .max_h(px(INBOX_MAX_HEIGHT))
            .overflow_y_scroll()
            .track_scroll(&self.scroll_handle)
            .pb_1()
            .children(rows),
        )
        .vertical_scrollbar(&self.scroll_handle)
        .into_any_element()
    };

    v_flex()
      .py_1()
      .border_t_1()
      .border_color(theme.border)
      .child(header)
      .child(body)
  }
}
