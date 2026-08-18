//! The sidebar's session list: pick, create and delete conversations.

use agent_chat_panel::ConversationMeta;
use gpui::{Context, EventEmitter, IntoElement, Render, SharedString, Window, div, prelude::*, px};
use gpui_component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use ui::{Button, ButtonVariants as _, UiIconName};

pub(crate) fn format_relative_secs(updated_at_secs: u64, now_secs: u64) -> String {
  let delta = now_secs.saturating_sub(updated_at_secs);
  match delta {
    0..=59 => "now".to_string(),
    60..=3_599 => format!("{}m", delta / 60),
    3_600..=86_399 => format!("{}h", delta / 3_600),
    _ => format!("{}d", delta / 86_400),
  }
}

fn now_secs() -> u64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_secs())
    .unwrap_or(0)
}

pub(crate) fn session_row_title(meta: &ConversationMeta) -> SharedString {
  let trimmed = meta.title.trim();
  if trimmed.is_empty() {
    "New session".into()
  } else {
    trimmed.to_string().into()
  }
}

pub enum SessionListEvent {
  NewSession,
  Selected { id: String },
  Deleted { id: String },
}

pub struct SessionList {
  conversations: Vec<ConversationMeta>,
  current_id: String,
}

impl SessionList {
  pub fn new() -> Self {
    Self {
      conversations: Vec::new(),
      current_id: String::new(),
    }
  }

  pub fn set_conversations(
    &mut self,
    conversations: Vec<ConversationMeta>,
    current_id: String,
    cx: &mut Context<Self>,
  ) {
    self.conversations = conversations;
    self.current_id = current_id;
    cx.notify();
  }
}

impl EventEmitter<SessionListEvent> for SessionList {}

impl Render for SessionList {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let now = now_secs();

    let header = h_flex()
      .h(px(40.))
      .min_h(px(40.))
      .max_h(px(40.))
      .flex_shrink_0()
      .items_center()
      .justify_between()
      .px_3()
      .border_b_1()
      .border_color(theme.border)
      .child(
        div()
          .text_xs()
          .font_weight(gpui::FontWeight::SEMIBOLD)
          .text_color(theme.muted_foreground)
          .child("Sessions"),
      )
      .child(
        Button::new("session-page-new-session")
          .icon(UiIconName::SquarePen)
          .ghost()
          .compact()
          .small()
          .tooltip("New session")
          .on_click(cx.listener(|_, _, _, cx| cx.emit(SessionListEvent::NewSession))),
      );

    let rows: Vec<_> = self
      .conversations
      .iter()
      .enumerate()
      .map(|(ix, meta)| {
        let is_current = meta.id == self.current_id;
        let id = meta.id.clone();
        let delete_id = meta.id.clone();
        let title = session_row_title(meta);
        let time = format_relative_secs(meta.updated_at_secs, now);
        let group_name = SharedString::from(format!("session-row-{}", meta.id));

        div()
          .id(("session-page-session-row", ix))
          .group(group_name.clone())
          .mx_2()
          .px_2()
          .py_1p5()
          .rounded(px(6.0))
          .cursor_pointer()
          .when(is_current, |this| this.bg(theme.secondary_active))
          .hover(|s| s.bg(theme.secondary_hover))
          .on_click(cx.listener(move |_, _, _, cx| {
            cx.emit(SessionListEvent::Selected { id: id.clone() });
          }))
          .child(
            h_flex()
              .items_center()
              .gap_2()
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
                div()
                  .text_xs()
                  .text_color(theme.muted_foreground)
                  .group_hover(group_name.clone(), |this| this.opacity(0.0))
                  .child(time),
              )
              .child(
                Button::new(("session-page-session-delete", ix))
                  .icon(UiIconName::Trash)
                  .xsmall()
                  .ghost()
                  .opacity(0.0)
                  .group_hover(group_name.clone(), |this| this.opacity(1.0))
                  .tooltip("Delete session")
                  .on_click(cx.listener(move |_, _, _, cx| {
                    cx.stop_propagation();
                    cx.emit(SessionListEvent::Deleted {
                      id: delete_id.clone(),
                    });
                  })),
              ),
          )
      })
      .collect();

    let body = if rows.is_empty() {
      v_flex()
        .flex_1()
        .min_h_0()
        .items_center()
        .justify_center()
        .gap_2()
        .px_4()
        .child(
          Icon::new(UiIconName::MessageCirclePlus)
            .size_4()
            .text_color(theme.muted_foreground),
        )
        .child(
          div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child("No sessions yet"),
        )
        .child(
          div()
            .text_xs()
            .text_center()
            .text_color(theme.muted_foreground.opacity(0.8))
            .child("Message the agent to start one"),
        )
        .into_any_element()
    } else {
      div()
        .id("session-page-session-list")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .py_1()
        .children(rows)
        .into_any_element()
    };

    v_flex()
      .size_full()
      .min_w(px(0.0))
      .min_h_0()
      .child(header)
      .child(body)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn meta_with_title(title: &str) -> ConversationMeta {
    ConversationMeta {
      id: "1".to_string(),
      started_at_secs: 0,
      updated_at_secs: 0,
      title: title.to_string(),
      message_count: 0,
      session_id: None,
    }
  }

  #[test]
  fn format_relative_secs_buckets() {
    assert_eq!(format_relative_secs(100, 100), "now");
    assert_eq!(format_relative_secs(100, 159), "now");
    assert_eq!(format_relative_secs(100, 160), "1m");
    assert_eq!(format_relative_secs(100, 100 + 3_600), "1h");
    assert_eq!(format_relative_secs(100, 100 + 86_400), "1d");
    assert_eq!(format_relative_secs(100, 100 + 3 * 86_400), "3d");
  }

  #[test]
  fn format_relative_secs_clamps_future_timestamps() {
    assert_eq!(format_relative_secs(200, 100), "now");
  }

  #[test]
  fn session_row_title_falls_back_when_empty() {
    assert_eq!(session_row_title(&meta_with_title("")), "New session");
    assert_eq!(session_row_title(&meta_with_title("   ")), "New session");
    assert_eq!(
      session_row_title(&meta_with_title("Fix scroll")),
      "Fix scroll"
    );
  }
}
