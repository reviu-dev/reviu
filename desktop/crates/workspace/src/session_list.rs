//! The sidebar's session list: pick, create and delete conversations.

use agent_chat_panel::ConversationMeta;
use gpui::{Context, EventEmitter, IntoElement, Render, SharedString, Window, div, prelude::*, px};
use gpui_component::scroll::ScrollableElement as _;
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
  /// The collapse button in the header; the page owns the sidebar width.
  Collapse,
  Selected {
    id: String,
  },
  Deleted {
    id: String,
  },
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

  /// Refresh the current conversation's row in place; the rest of the list
  /// only changes through `set_conversations`. No-op notifies are skipped so
  /// streaming commits don't re-render the sidebar.
  pub fn upsert_current(
    &mut self,
    meta: Option<ConversationMeta>,
    current_id: String,
    cx: &mut Context<Self>,
  ) {
    let mut changed = self.current_id != current_id;
    self.current_id = current_id;
    if let Some(meta) = meta {
      match self.conversations.iter_mut().find(|c| c.id == meta.id) {
        Some(entry) if *entry == meta => {}
        Some(entry) => {
          *entry = meta;
          changed = true;
        }
        None => {
          self.conversations.push(meta);
          changed = true;
        }
      }
      if changed {
        self
          .conversations
          .sort_by_key(|m| std::cmp::Reverse(m.updated_at_secs));
      }
    }
    if changed {
      cx.notify();
    }
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
        h_flex()
          .items_center()
          .gap_1()
          .child(
            Button::new("session-page-new-session")
              .icon(UiIconName::SquarePen)
              .ghost()
              .compact()
              .small()
              .tooltip("New session")
              .on_click(cx.listener(|_, _, _, cx| cx.emit(SessionListEvent::NewSession))),
          )
          .child(
            Button::new("session-sidebar-collapse")
              .debug_selector(|| "session-sidebar-collapse".to_string())
              .icon(gpui_component::IconName::PanelLeftClose)
              .ghost()
              .compact()
              .small()
              .tooltip("Collapse sidebar")
              .on_click(cx.listener(|_, _, _, cx| cx.emit(SessionListEvent::Collapse))),
          ),
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
                // One trailing slot: the time sits flush right, the delete
                // button takes its place on hover instead of reserving width.
                div()
                  .relative()
                  .flex_shrink_0()
                  .min_w(px(22.))
                  .flex()
                  .justify_end()
                  .items_center()
                  .child(
                    div()
                      .text_xs()
                      .text_color(theme.muted_foreground)
                      .group_hover(group_name.clone(), |this| this.opacity(0.0))
                      .child(time),
                  )
                  .child(
                    div()
                      .absolute()
                      .right(px(-2.))
                      .top(px(-3.))
                      .opacity(0.0)
                      .group_hover(group_name.clone(), |this| this.opacity(1.0))
                      .child(
                        Button::new(("session-page-session-delete", ix))
                          .icon(UiIconName::Trash)
                          .xsmall()
                          .ghost()
                          .tooltip("Delete session")
                          .on_click(cx.listener(move |_, _, _, cx| {
                            cx.stop_propagation();
                            cx.emit(SessionListEvent::Deleted {
                              id: delete_id.clone(),
                            });
                          })),
                      ),
                  ),
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
        .overflow_y_scrollbar()
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

  fn meta(id: &str, updated: u64) -> ConversationMeta {
    ConversationMeta {
      id: id.to_string(),
      started_at_secs: 0,
      updated_at_secs: updated,
      title: id.to_string(),
      message_count: 1,
      session_id: None,
    }
  }

  #[gpui::test]
  async fn upsert_current_updates_in_place_inserts_and_resorts(cx: &mut gpui::TestAppContext) {
    let list = cx.new(|_| SessionList::new());
    list.update(cx, |list, cx| {
      list.set_conversations(vec![meta("b", 20), meta("a", 10)], "a".into(), cx);

      // Bumping the current row's timestamp moves it to the top, in place.
      list.upsert_current(Some(meta("a", 30)), "a".into(), cx);
      assert_eq!(list.conversations.len(), 2);
      assert_eq!(list.conversations[0].id, "a");

      // A row not yet on disk gets inserted.
      list.upsert_current(Some(meta("c", 40)), "c".into(), cx);
      assert_eq!(list.conversations.len(), 3);
      assert_eq!(list.conversations[0].id, "c");

      // An empty draft only moves the selection.
      list.upsert_current(None, "b".into(), cx);
      assert_eq!(list.current_id, "b");
      assert_eq!(list.conversations.len(), 3);
    });
  }
}
