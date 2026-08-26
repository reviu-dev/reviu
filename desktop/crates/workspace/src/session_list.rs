//! The sidebar's session list: pick, create and delete conversations.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

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

/// One sidebar row: the conversation and the repo it belongs to. Rows arrive
/// grouped by repo (stable section order) and render under section headers.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionRow {
  pub meta: ConversationMeta,
  pub repo_root: PathBuf,
}

pub enum SessionListEvent {
  /// The section header's compose button: a session in THAT repo.
  NewSessionIn {
    repo_root: PathBuf,
  },
  /// The section header itself: fold or unfold a repo's sessions.
  ToggleRepoCollapsed {
    repo_root: PathBuf,
  },
  /// The section header's worktree button: a session whose agent works in its
  /// own git worktree of THAT repo, started from `base`; `None` is the
  /// repository's default branch.
  NewWorktreeSessionIn {
    repo_root: PathBuf,
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
  /// Folded repo sections; folding IS the filter now.
  collapsed_repos: std::collections::HashSet<PathBuf>,
  /// Every tracked repo, in stable order: sections render from this, so an
  /// emptied repo keeps its header (and its compose button).
  section_order: Vec<PathBuf>,
}

impl SessionList {
  pub fn new() -> Self {
    Self {
      conversations: Vec::new(),
      current_id: String::new(),
      loading_id: None,
      statuses: HashMap::new(),
      worktree_branches: HashMap::new(),
      collapsed_repos: std::collections::HashSet::new(),
      section_order: Vec::new(),
    }
  }

  pub fn set_section_order(&mut self, section_order: Vec<PathBuf>, cx: &mut Context<Self>) {
    if self.section_order != section_order {
      self.section_order = section_order;
      cx.notify();
    }
  }

  #[cfg(test)]
  pub(crate) fn section_order_for_test(&self) -> &[PathBuf] {
    &self.section_order
  }

  pub fn toggle_repo_collapsed(&mut self, repo_root: &Path, cx: &mut Context<Self>) {
    if !self.collapsed_repos.remove(repo_root) {
      self.collapsed_repos.insert(repo_root.to_path_buf());
    }
    cx.notify();
  }

  #[cfg(test)]
  pub(crate) fn is_repo_collapsed(&self, repo_root: &Path) -> bool {
    self.collapsed_repos.contains(repo_root)
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
        // Updated in place: a streaming session must never change position.
        Some(entry) => {
          *entry = row;
          changed = true;
        }
        None => {
          // A fresh conversation heads its repo's section (newest-created
          // first); an unknown repo opens a section at the end until the
          // next full refresh settles the order.
          let at = self
            .conversations
            .iter()
            .position(|existing| existing.repo_root == row.repo_root)
            .unwrap_or(self.conversations.len());
          self.conversations.insert(at, row);
          changed = true;
        }
      }
    }
    if changed {
      cx.notify();
    }
  }
}

impl SessionList {
  /// A repo's section header: fold toggle, name, count when folded, and the
  /// two create buttons that target THAT repo (session, worktree session).
  fn render_repo_header(
    &self,
    repo_root: &Path,
    count: usize,
    theme: &gpui_component::Theme,
    cx: &mut Context<Self>,
  ) -> gpui::AnyElement {
    let collapsed = self.collapsed_repos.contains(repo_root);
    let name: SharedString = repo_root
      .file_name()
      .map(|name| name.to_string_lossy().into_owned())
      .unwrap_or_else(|| repo_root.to_string_lossy().into_owned())
      .into();
    let toggle_repo = repo_root.to_path_buf();
    let compose_repo = repo_root.to_path_buf();
    let worktree_repo = repo_root.to_path_buf();
    let group_name = SharedString::from(format!("repo-section-{}", repo_root.display()));

    h_flex()
      .id(SharedString::from(format!(
        "session-repo-section-{}",
        repo_root.display()
      )))
      .group(group_name.clone())
      .items_center()
      .gap_1()
      .mx_2()
      .mt_1()
      .px_2()
      .py_1()
      .rounded(px(6.0))
      .cursor_pointer()
      .hover(|this| this.bg(theme.secondary_hover))
      .on_click(cx.listener(move |_, _, _, cx| {
        cx.emit(SessionListEvent::ToggleRepoCollapsed {
          repo_root: toggle_repo.clone(),
        });
      }))
      .child(
        Icon::new(if collapsed {
          gpui_component::IconName::ChevronRight
        } else {
          gpui_component::IconName::ChevronDown
        })
        .size(px(12.))
        .text_color(theme.muted_foreground),
      )
      .child(
        div()
          .flex_1()
          .min_w(px(0.0))
          .text_xs()
          .font_weight(gpui::FontWeight::SEMIBOLD)
          .truncate()
          .text_color(theme.muted_foreground)
          .child(name),
      )
      .when(collapsed, |this| {
        this.child(
          div()
            .text_xs()
            .text_color(theme.muted_foreground.opacity(0.8))
            .child(count.to_string()),
        )
      })
      .child(
        h_flex()
          .items_center()
          .gap_1()
          .opacity(0.6)
          .group_hover(group_name, |this| this.opacity(1.0))
          .child(
            Button::new(SharedString::from(format!(
              "session-repo-new-{}",
              repo_root.display()
            )))
            .icon(UiIconName::SquarePen)
            .ghost()
            .compact()
            .xsmall()
            .tooltip("New session in this repository")
            .on_click(cx.listener(move |_, _, _, cx| {
              cx.stop_propagation();
              cx.emit(SessionListEvent::NewSessionIn {
                repo_root: compose_repo.clone(),
              });
            })),
          )
          .child(
            // The wrapper keeps the dropdown's click from folding the section.
            div()
              .id(SharedString::from(format!(
                "session-repo-worktree-wrap-{}",
                worktree_repo.display()
              )))
              .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
              .child(Self::render_worktree_button(worktree_repo, cx)),
          ),
      )
      .into_any_element()
  }

  /// The worktree button of one repo section: its base picker reads branches
  /// from THAT repo, at menu-open time (always fresh, never polled).
  fn render_worktree_button(repo_root: PathBuf, cx: &mut Context<Self>) -> impl IntoElement {
    let entity = cx.entity().downgrade();
    Button::new(SharedString::from(format!(
      "session-repo-worktree-{}",
      repo_root.display()
    )))
    .debug_selector(|| format!("session-repo-worktree-{}", repo_root.display()))
    .icon(UiIconName::GitBranch)
    .ghost()
    .compact()
    .xsmall()
    .tooltip("New worktree session in this repository")
    .dropdown_menu_with_anchor(Anchor::TopLeft, move |menu, _, _| {
      let mut menu = menu.max_h(px(360.)).scrollable(true);
      let base_candidates: Vec<SharedString> = git::list_branches(&repo_root)
        .ok()
        .unwrap_or_default()
        .into_iter()
        .filter(|branch| branch.kind == git::BranchKind::Local)
        .map(|branch| SharedString::from(branch.name))
        .collect();
      let default_entity = entity.clone();
      let default_repo = repo_root.clone();
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
          let repo_root = default_repo.clone();
          let _ = default_entity.update(cx, |_, cx| {
            cx.emit(SessionListEvent::NewWorktreeSessionIn {
              repo_root,
              base: None,
            });
          });
        }),
      );
      // Any branch is a valid base: the worktree gets a NEW branch at its
      // commit, nothing is checked out twice.
      for candidate in &base_candidates {
        let label = candidate.clone();
        let base = candidate.to_string();
        let entity = entity.clone();
        let item_repo = repo_root.clone();
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
            let repo_root = item_repo.clone();
            let base = base.clone();
            let _ = entity.update(cx, |_, cx| {
              cx.emit(SessionListEvent::NewWorktreeSessionIn {
                repo_root,
                base: Some(base),
              });
            });
          }),
        );
      }
      menu
    })
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
        Button::new("session-sidebar-collapse")
          .debug_selector(|| "session-sidebar-collapse".to_string())
          .icon(gpui_component::IconName::PanelLeftClose)
          .ghost()
          .compact()
          .small()
          .tooltip("Collapse sidebar")
          .on_click(cx.listener(|_, _, _, cx| cx.emit(SessionListEvent::Collapse))),
      );

    // Sections come from the tracked-repo order so an empty repo keeps its
    // header; rows not yet in that order (fresh upsert) get a section at the
    // end until the next refresh settles it.
    let mut section_repos: Vec<PathBuf> = self.section_order.clone();
    for row in &self.conversations {
      if !section_repos.contains(&row.repo_root) {
        section_repos.push(row.repo_root.clone());
      }
    }
    let mut items: Vec<gpui::AnyElement> = Vec::new();
    for section_repo in &section_repos {
      let count = self
        .conversations
        .iter()
        .filter(|row| &row.repo_root == section_repo)
        .count();
      items.push(self.render_repo_header(section_repo, count, &theme, cx));
      if self.collapsed_repos.contains(section_repo) {
        continue;
      }
      for (ix, row) in self
        .conversations
        .iter()
        .enumerate()
        .filter(|(_, row)| &row.repo_root == section_repo)
      {
        items.push({
          let meta = &row.meta;
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
                    } else {
                      // A dot says the live state without eating the title or
                      // the timestamp; the word lives in its tooltip.
                      h_flex()
                        .items_center()
                        .gap_1p5()
                        .group_hover(group_name.clone(), |this| this.opacity(0.0))
                        .when_some(status.label(), |this, label| {
                          this.child(
                            div()
                              .id(("session-status-dot", ix))
                              .size(px(7.))
                              .rounded_full()
                              .bg(status_color.opacity(0.9))
                              .tooltip(move |window, cx| {
                                gpui_component::tooltip::Tooltip::new(label).build(window, cx)
                              }),
                          )
                        })
                        .child(
                          div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(time),
                        )
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
            })
            .into_any_element()
        });
      }
    }
    let rows = items;

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
      repo_root: PathBuf::from("/repo"),
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
  async fn upsert_current_updates_in_place_and_never_moves_a_streaming_row(
    cx: &mut gpui::TestAppContext,
  ) {
    let list = cx.new(|_| SessionList::new());
    list.update(cx, |list, cx| {
      list.set_conversations(vec![meta("b", 20), meta("a", 10)], "a".into(), cx);

      // A streaming session bumps its timestamp: the row must NOT move.
      list.upsert_current(Some(meta("a", 30)), "a".into(), cx);
      assert_eq!(
        list.conversation_ids(),
        vec!["b".to_string(), "a".to_string()],
        "positions are stable while sessions stream"
      );
      assert_eq!(list.conversations[1].meta.updated_at_secs, 30);

      // A row not yet on disk heads its repo's section.
      list.upsert_current(Some(meta("c", 40)), "c".into(), cx);
      assert_eq!(
        list.conversation_ids(),
        vec!["c".to_string(), "b".to_string(), "a".to_string()]
      );

      // An empty draft only moves the selection.
      list.upsert_current(None, "b".into(), cx);
      assert_eq!(list.current_id, "b");
      assert_eq!(list.conversations.len(), 3);

      // Folding a repo is list state, not data: rows stay.
      list.toggle_repo_collapsed(Path::new("/repo"), cx);
      assert!(list.is_repo_collapsed(Path::new("/repo")));
      assert_eq!(list.conversations.len(), 3);
      list.toggle_repo_collapsed(Path::new("/repo"), cx);
      assert!(!list.is_repo_collapsed(Path::new("/repo")));
    });
  }
}
