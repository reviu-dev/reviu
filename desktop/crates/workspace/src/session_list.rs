//! The sidebar's session list: pick, create and delete conversations.

use std::collections::HashMap;
use std::path::PathBuf;

use agent_chat_panel::ConversationMeta;
use gpui::{
  Anchor, Context, EventEmitter, IntoElement, Render, SharedString, Window, div, prelude::*, px,
};
use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use ui::{Button, ButtonVariants as _, StatusThemeExt as _, UiIconName};

/// Live state of a session's agent, derived from its panel; a session with no
/// panel alive is Idle. Deliberately NOT animated: a repeating per-row
/// animation once pinned a whole window at 120Hz (see comet's motion.rs).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SessionStatus {
  #[default]
  Idle,
  Working,
  /// The agent waits on a permission answer.
  Waiting,
  /// The agent process died or its binary is missing.
  Failed,
}

impl SessionStatus {
  fn label(self) -> Option<&'static str> {
    match self {
      SessionStatus::Idle => None,
      SessionStatus::Working => Some("Working"),
      SessionStatus::Waiting => Some("Waiting"),
      SessionStatus::Failed => Some("Failed"),
    }
  }
}

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

/// One sidebar row: the conversation plus, when the list spans several
/// repos, the repo it belongs to.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionRow {
  pub meta: ConversationMeta,
  pub repo_name: Option<SharedString>,
}

pub enum SessionListEvent {
  NewSession,
  /// A session whose agent works in its own git worktree, started from
  /// `base`; `None` is the repository's default branch.
  NewWorktreeSession {
    base: Option<String>,
  },
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
  conversations: Vec<SessionRow>,
  current_id: String,
  /// Row still hydrating after a click; shows a spinner in its trailing slot.
  loading_id: Option<String>,
  /// Live agent state by conversation id; absent rows are Idle.
  statuses: HashMap<String, SessionStatus>,
  /// Worktree branch by conversation id, shown under the row.
  worktree_branches: HashMap<String, String>,
  /// The scope repo: worktree creation targets it, so its base picker reads
  /// branches from it, at menu-open time (always fresh, never polled).
  scope_repo: Option<PathBuf>,
}

impl SessionList {
  pub fn new() -> Self {
    Self {
      conversations: Vec::new(),
      current_id: String::new(),
      loading_id: None,
      statuses: HashMap::new(),
      worktree_branches: HashMap::new(),
      scope_repo: None,
    }
  }

  pub fn set_scope_repo(&mut self, scope_repo: Option<PathBuf>, cx: &mut Context<Self>) {
    if self.scope_repo != scope_repo {
      self.scope_repo = scope_repo;
      cx.notify();
    }
  }

  pub fn set_loading(&mut self, loading_id: Option<String>, cx: &mut Context<Self>) {
    if self.loading_id != loading_id {
      self.loading_id = loading_id;
      cx.notify();
    }
  }

  /// No-op notifies are skipped: statuses re-derive on every panel notify and
  /// must not re-render the sidebar while nothing visible moved.
  pub fn set_statuses(&mut self, statuses: HashMap<String, SessionStatus>, cx: &mut Context<Self>) {
    if self.statuses != statuses {
      self.statuses = statuses;
      cx.notify();
    }
  }

  pub fn set_worktree_branches(
    &mut self,
    worktree_branches: HashMap<String, String>,
    cx: &mut Context<Self>,
  ) {
    if self.worktree_branches != worktree_branches {
      self.worktree_branches = worktree_branches;
      cx.notify();
    }
  }

  #[cfg(test)]
  pub(crate) fn status_of(&self, id: &str) -> SessionStatus {
    self.statuses.get(id).copied().unwrap_or_default()
  }

  #[cfg(test)]
  pub(crate) fn worktree_branch_of(&self, id: &str) -> Option<&str> {
    self.worktree_branches.get(id).map(String::as_str)
  }

  pub fn set_conversations(
    &mut self,
    conversations: Vec<SessionRow>,
    current_id: String,
    cx: &mut Context<Self>,
  ) {
    self.conversations = conversations;
    self.current_id = current_id;
    cx.notify();
  }

  #[cfg(test)]
  pub(crate) fn conversation_ids(&self) -> Vec<String> {
    self
      .conversations
      .iter()
      .map(|row| row.meta.id.clone())
      .collect()
  }

  /// Refresh the current conversation's row in place; the rest of the list
  /// only changes through `set_conversations`. No-op notifies are skipped so
  /// streaming commits don't re-render the sidebar.
  pub fn upsert_current(
    &mut self,
    row: Option<SessionRow>,
    current_id: String,
    cx: &mut Context<Self>,
  ) {
    let mut changed = self.current_id != current_id;
    self.current_id = current_id;
    if let Some(row) = row {
      match self
        .conversations
        .iter_mut()
        .find(|existing| existing.meta.id == row.meta.id)
      {
        Some(entry) if *entry == row => {}
        Some(entry) => {
          *entry = row;
          changed = true;
        }
        None => {
          self.conversations.push(row);
          changed = true;
        }
      }
      if changed {
        self
          .conversations
          .sort_by_key(|row| std::cmp::Reverse(row.meta.updated_at_secs));
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
          .child({
            let entity = cx.entity().downgrade();
            let scope_repo = self.scope_repo.clone();
            Button::new("session-page-new-worktree-session")
              .debug_selector(|| "session-page-new-worktree-session".to_string())
              .icon(UiIconName::GitBranch)
              .ghost()
              .compact()
              .small()
              .tooltip("New session in a worktree")
              .dropdown_menu_with_anchor(Anchor::TopLeft, move |menu, _, _| {
                let mut menu = menu.max_h(px(360.)).scrollable(true);
                // Read at menu-open, from the SCOPE repo (worktree creation
                // targets it, not the shown session's checkout).
                let base_candidates: Vec<SharedString> = scope_repo
                  .as_deref()
                  .and_then(|repo| git::list_branches(repo).ok())
                  .unwrap_or_default()
                  .into_iter()
                  .filter(|branch| branch.kind == git::BranchKind::Local)
                  .map(|branch| SharedString::from(branch.name))
                  .collect();
                let default_entity = entity.clone();
                menu = menu.item(
                  PopupMenuItem::element(move |_, cx| {
                    let theme = cx.theme().clone();
                    div()
                      .text_sm()
                      .text_color(theme.foreground)
                      .debug_selector(|| "session-worktree-base-default".to_string())
                      .child("Default branch")
                      .into_any_element()
                  })
                  .on_click(move |_, _, cx| {
                    let _ = default_entity.update(cx, |_, cx| {
                      cx.emit(SessionListEvent::NewWorktreeSession { base: None });
                    });
                  }),
                );
                // Any branch is a valid base: the worktree gets a NEW branch
                // at its commit, nothing is checked out twice.
                for candidate in &base_candidates {
                  let label = candidate.clone();
                  let base = candidate.to_string();
                  let entity = entity.clone();
                  menu = menu.item(
                    PopupMenuItem::element(move |_, cx| {
                      let theme = cx.theme().clone();
                      h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                          Icon::new(UiIconName::GitBranch)
                            .small()
                            .text_color(theme.muted_foreground),
                        )
                        .child(div().text_sm().child(label.clone()))
                        .into_any_element()
                    })
                    .on_click(move |_, _, cx| {
                      let base = base.clone();
                      let _ = entity.update(cx, |_, cx| {
                        cx.emit(SessionListEvent::NewWorktreeSession { base: Some(base) });
                      });
                    }),
                  );
                }
                menu
              })
          })
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
      .map(|(ix, row)| {
        let meta = &row.meta;
        let repo_name = row.repo_name.clone();
        let is_current = meta.id == self.current_id;
        let is_loading = self.loading_id.as_deref() == Some(meta.id.as_str());
        let status = self.statuses.get(&meta.id).copied().unwrap_or_default();
        let status_color = match status {
          SessionStatus::Idle => theme.muted_foreground,
          SessionStatus::Working => theme.status_amber(),
          SessionStatus::Waiting => theme.status_blue(),
          SessionStatus::Failed => theme.status_red(),
        };
        let worktree_branch = self
          .worktree_branches
          .get(&meta.id)
          .map(|branch| SharedString::from(branch.clone()));
        let id = meta.id.clone();
        let delete_id = meta.id.clone();
        let title = session_row_title(meta);
        let preview = meta.preview.clone();
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
                  .child(if is_loading {
                    div()
                      .child(gpui_component::spinner::Spinner::new().xsmall())
                      .into_any_element()
                  } else if let Some(label) = status.label() {
                    // The live state replaces the timestamp: a running or
                    // stuck session matters more than how old it is.
                    div()
                      .text_xs()
                      .text_color(status_color.opacity(0.9))
                      .group_hover(group_name.clone(), |this| this.opacity(0.0))
                      .child(label)
                      .into_any_element()
                  } else {
                    div()
                      .text_xs()
                      .text_color(theme.muted_foreground)
                      .group_hover(group_name.clone(), |this| this.opacity(0.0))
                      .child(time)
                      .into_any_element()
                  })
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
          .when(!preview.is_empty(), |this| {
            this.child(
              div()
                .text_xs()
                .truncate()
                .text_color(theme.muted_foreground)
                .child(preview),
            )
          })
          .when(repo_name.is_some() || worktree_branch.is_some(), |this| {
            this.child(
              h_flex()
                .items_center()
                .gap_2()
                .when_some(repo_name, |this, repo_name| {
                  this.child(
                    h_flex()
                      .items_center()
                      .gap_1()
                      .child(
                        Icon::new(gpui_component::IconName::Folder)
                          .size(px(10.))
                          .text_color(theme.muted_foreground.opacity(0.8)),
                      )
                      .child(
                        div()
                          .text_xs()
                          .truncate()
                          .text_color(theme.muted_foreground.opacity(0.8))
                          .child(repo_name),
                      ),
                  )
                })
                .when_some(worktree_branch, |this, branch| {
                  this.child(
                    h_flex()
                      .items_center()
                      .gap_1()
                      .child(
                        Icon::new(UiIconName::GitBranch)
                          .size(px(10.))
                          .text_color(theme.muted_foreground.opacity(0.8)),
                      )
                      .child(
                        div()
                          .text_xs()
                          .truncate()
                          .text_color(theme.muted_foreground.opacity(0.8))
                          .child(branch),
                      ),
                  )
                }),
            )
          })
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
      preview: String::new(),
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

  fn meta(id: &str, updated: u64) -> SessionRow {
    SessionRow {
      meta: ConversationMeta {
        id: id.to_string(),
        started_at_secs: 0,
        updated_at_secs: updated,
        title: id.to_string(),
        message_count: 1,
        session_id: None,
        preview: String::new(),
      },
      repo_name: None,
    }
  }

  #[gpui::test]
  async fn identical_statuses_never_repaint_the_sidebar(cx: &mut gpui::TestAppContext) {
    use std::cell::Cell;
    use std::rc::Rc;

    let list = cx.new(|_| SessionList::new());
    let repaints = Rc::new(Cell::new(0_usize));
    cx.update(|cx| {
      let repaints = repaints.clone();
      cx.observe(&list, move |_, _| repaints.set(repaints.get() + 1))
        .detach();
    });

    let mut statuses = HashMap::new();
    statuses.insert("a".to_string(), SessionStatus::Working);
    list.update(cx, |list, cx| list.set_statuses(statuses.clone(), cx));
    cx.run_until_parked();
    assert_eq!(repaints.get(), 1, "a real change repaints");

    // Statuses re-derive on every panel notify, streaming included: the same
    // map must cost nothing.
    list.update(cx, |list, cx| list.set_statuses(statuses.clone(), cx));
    list.update(cx, |list, cx| {
      list.set_worktree_branches(HashMap::new(), cx)
    });
    cx.run_until_parked();
    assert_eq!(repaints.get(), 1, "no-op updates never repaint");

    statuses.insert("a".to_string(), SessionStatus::Waiting);
    list.update(cx, |list, cx| list.set_statuses(statuses, cx));
    cx.run_until_parked();
    assert_eq!(repaints.get(), 2);
  }

  #[gpui::test]
  async fn upsert_current_updates_in_place_inserts_and_resorts(cx: &mut gpui::TestAppContext) {
    let list = cx.new(|_| SessionList::new());
    list.update(cx, |list, cx| {
      list.set_conversations(vec![meta("b", 20), meta("a", 10)], "a".into(), cx);

      // Bumping the current row's timestamp moves it to the top, in place.
      list.upsert_current(Some(meta("a", 30)), "a".into(), cx);
      assert_eq!(list.conversations.len(), 2);
      assert_eq!(list.conversations[0].meta.id, "a");

      // A row not yet on disk gets inserted.
      list.upsert_current(Some(meta("c", 40)), "c".into(), cx);
      assert_eq!(list.conversations.len(), 3);
      assert_eq!(list.conversations[0].meta.id, "c");

      // An empty draft only moves the selection.
      list.upsert_current(None, "b".into(), cx);
      assert_eq!(list.current_id, "b");
      assert_eq!(list.conversations.len(), 3);
    });
  }
}
