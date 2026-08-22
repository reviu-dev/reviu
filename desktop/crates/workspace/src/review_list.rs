//! The Review tab: the batch of local comments waiting to go to the agent.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use gpui::{
  AnyElement, Context, InteractiveElement, IntoElement, MouseButton, ParentElement, Render,
  StatefulInteractiveElement as _, Styled, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
  ActiveTheme as _, Disableable as _, Icon, IconName, Sizable,
  button::{Button, ButtonVariants as _},
  checkbox::Checkbox,
  h_flex, v_flex,
};
use ui::{StatusThemeExt as _, UiIconName};

use crate::agent_review::{
  LocalAgentReviewComment, LocalAgentReviewCommentState, agent_review_line_label,
  agent_review_state_is_copyable,
};
use crate::changes_list::split_path_label;

pub(crate) const REVIEW_LIST_SEND_DEBUG_SELECTOR: &str = "review-list-send";
pub(crate) const REVIEW_LIST_DISCARD_DEBUG_SELECTOR: &str = "review-list-discard";
pub(crate) const REVIEW_LIST_SELECT_ALL_DEBUG_SELECTOR: &str = "review-list-select-all";

/// Longest excerpt shown on a row before it is cut.
const REVIEW_EXCERPT_MAX_CHARS: usize = 120;

pub(crate) enum ReviewListEvent {
  /// Take me to the lines this comment is about.
  OpenComment {
    path: PathBuf,
    line: usize,
  },
  DeleteComment {
    id: u64,
  },
  /// One comment on its own, whatever the selection holds.
  SendComment {
    id: u64,
  },
  SendReview,
  DiscardReview,
}

/// What a row needs, and nothing about where the comment came from: a pull
/// request's pending comments can feed the same panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReviewPanelComment {
  pub id: u64,
  pub path: PathBuf,
  pub line: usize,
  pub line_label: String,
  pub excerpt: String,
  pub state: LocalAgentReviewCommentState,
  /// Addressed and outdated comments have a row, but nothing left to send.
  pub sendable: bool,
}

/// The first line with something on it: a row shows one line, not a paragraph.
pub(crate) fn review_comment_excerpt(body: &str) -> String {
  let line = body
    .lines()
    .map(str::trim)
    .find(|line| !line.is_empty())
    .unwrap_or("");
  if line.chars().count() <= REVIEW_EXCERPT_MAX_CHARS {
    return line.to_string();
  }
  let mut excerpt = line
    .chars()
    .take(REVIEW_EXCERPT_MAX_CHARS)
    .collect::<String>();
  excerpt.push('…');
  excerpt
}

pub(crate) fn review_panel_comments(
  comments: &[LocalAgentReviewComment],
) -> Vec<ReviewPanelComment> {
  let mut rows = comments
    .iter()
    .map(|comment| ReviewPanelComment {
      id: comment.id,
      path: comment.path.clone(),
      line: comment.line,
      line_label: agent_review_line_label(comment),
      excerpt: review_comment_excerpt(comment.body.as_ref()),
      state: comment.state.clone(),
      sendable: agent_review_state_is_copyable(&comment.state),
    })
    .collect::<Vec<_>>();
  rows.sort_by(|a, b| {
    a.path
      .cmp(&b.path)
      .then_with(|| a.line.cmp(&b.line))
      .then_with(|| a.id.cmp(&b.id))
  });
  rows
}

/// Comments keep their order inside a file, and files keep the order the sorted
/// comments gave them.
pub(crate) fn group_review_comments_by_file(
  comments: &[ReviewPanelComment],
) -> Vec<(PathBuf, Vec<ReviewPanelComment>)> {
  let mut groups: Vec<(PathBuf, Vec<ReviewPanelComment>)> = Vec::new();
  for comment in comments {
    match groups.last_mut() {
      Some((path, rows)) if path == &comment.path => rows.push(comment.clone()),
      _ => groups.push((comment.path.clone(), vec![comment.clone()])),
    }
  }
  groups
}

/// What a row says about a comment the batch already dealt with. A draft says
/// nothing: it is the ordinary case.
pub(crate) fn review_state_label(state: &LocalAgentReviewCommentState) -> Option<&'static str> {
  match state {
    LocalAgentReviewCommentState::Draft => None,
    LocalAgentReviewCommentState::Copied => Some("Sent"),
    LocalAgentReviewCommentState::Addressed => Some("Addressed"),
    LocalAgentReviewCommentState::Outdated => Some("Outdated"),
  }
}

pub(crate) struct ReviewList {
  comments: Vec<ReviewPanelComment>,
  collapsed_files: HashSet<PathBuf>,
  /// Empty means the whole batch goes: nobody loses a comment by not ticking it.
  selected: HashSet<u64>,
}

impl gpui::EventEmitter<ReviewListEvent> for ReviewList {}

impl ReviewList {
  pub(crate) fn new() -> Self {
    Self {
      comments: Vec::new(),
      collapsed_files: HashSet::new(),
      selected: HashSet::new(),
    }
  }

  pub(crate) fn set_comments(&mut self, comments: Vec<ReviewPanelComment>, cx: &mut Context<Self>) {
    if self.comments == comments {
      return;
    }
    let paths = comments
      .iter()
      .map(|comment| comment.path.clone())
      .collect::<HashSet<_>>();
    self.collapsed_files.retain(|path| paths.contains(path));
    let sendable_ids = comments
      .iter()
      .filter(|comment| comment.sendable)
      .map(|comment| comment.id)
      .collect::<HashSet<_>>();
    self.selected.retain(|id| sendable_ids.contains(id));
    self.comments = comments;
    cx.notify();
  }

  #[cfg(test)]
  pub(crate) fn comments(&self) -> &[ReviewPanelComment] {
    &self.comments
  }

  pub(crate) fn selected_ids(&self) -> &HashSet<u64> {
    &self.selected
  }

  /// Called once a send went out: what left is marked sent, so its tick has
  /// nothing left to say. Ticks that did not go stay, waiting for their turn.
  pub(crate) fn deselect(&mut self, ids: &HashSet<u64>, cx: &mut Context<Self>) {
    let before = self.selected.len();
    self.selected.retain(|id| !ids.contains(id));
    if self.selected.len() != before {
      cx.notify();
    }
  }

  fn sendable_ids(&self) -> impl Iterator<Item = u64> + '_ {
    self
      .comments
      .iter()
      .filter(|comment| comment.sendable)
      .map(|comment| comment.id)
  }

  fn sendable_count(&self) -> usize {
    self.sendable_ids().count()
  }

  /// How many comments the Send button would send right now.
  fn send_count(&self) -> usize {
    if self.selected.is_empty() {
      self.sendable_count()
    } else {
      self.selected.len()
    }
  }

  fn everything_is_selected(&self) -> bool {
    let sendable_count = self.sendable_count();
    sendable_count > 0 && self.selected.len() == sendable_count
  }

  pub(crate) fn toggle_comment(&mut self, comment_id: u64, cx: &mut Context<Self>) {
    if !self.selected.remove(&comment_id) {
      self.selected.insert(comment_id);
    }
    cx.notify();
  }

  fn file_sendable_ids(&self, path: &Path) -> Vec<u64> {
    self
      .comments
      .iter()
      .filter(|comment| comment.path == path && comment.sendable)
      .map(|comment| comment.id)
      .collect()
  }

  pub(crate) fn toggle_file_selection(&mut self, path: PathBuf, cx: &mut Context<Self>) {
    let ids = self.file_sendable_ids(&path);
    if ids.is_empty() {
      return;
    }
    if ids.iter().all(|id| self.selected.contains(id)) {
      self.selected.retain(|id| !ids.contains(id));
    } else {
      self.selected.extend(ids);
    }
    cx.notify();
  }

  pub(crate) fn toggle_select_all(&mut self, cx: &mut Context<Self>) {
    if self.everything_is_selected() {
      self.selected.clear();
    } else {
      self.selected = self.sendable_ids().collect();
    }
    cx.notify();
  }

  fn toggle_file(&mut self, path: PathBuf, cx: &mut Context<Self>) {
    if !self.collapsed_files.remove(&path) {
      self.collapsed_files.insert(path);
    }
    cx.notify();
  }

  fn render_file_header(&self, path: &Path, count: usize, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let collapsed = self.collapsed_files.contains(path);
    let (dir, file) = split_path_label(path);
    let toggle_path = path.to_path_buf();
    let sendable_ids = self.file_sendable_ids(path);
    let file_is_selected =
      !sendable_ids.is_empty() && sendable_ids.iter().all(|id| self.selected.contains(id));
    let select_path = path.to_path_buf();

    h_flex()
      .id(gpui::SharedString::from(format!(
        "review-file-{}",
        path.to_string_lossy()
      )))
      .w_full()
      .items_center()
      .gap_1()
      .px_1()
      .py_1()
      .rounded_sm()
      .hover(|this| this.bg(theme.accent))
      .cursor_pointer()
      .on_click(cx.listener(move |this, _, _, cx| {
        this.toggle_file(toggle_path.clone(), cx);
      }))
      .child(
        div()
          .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
          .child(
            Checkbox::new(gpui::SharedString::from(format!(
              "review-file-select-{}",
              path.to_string_lossy()
            )))
            .small()
            .checked(file_is_selected)
            .disabled(sendable_ids.is_empty())
            .tooltip("Select every comment of this file")
            .on_click(cx.listener(move |this, _, _, cx| {
              cx.stop_propagation();
              this.toggle_file_selection(select_path.clone(), cx);
            })),
          ),
      )
      .child(Icon::new(if collapsed {
        IconName::ChevronRight
      } else {
        IconName::ChevronDown
      }))
      .child(div().text_sm().text_color(theme.foreground).child(file))
      .child(
        div()
          .flex_1()
          .min_w_0()
          .text_xs()
          .text_color(theme.muted_foreground)
          .truncate()
          .child(dir),
      )
      .child(
        div()
          .text_xs()
          .text_color(theme.muted_foreground)
          .child(count.to_string()),
      )
      .into_any_element()
  }

  fn render_comment_row(&self, comment: &ReviewPanelComment, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let open = (comment.path.clone(), comment.line);
    let delete_id = comment.id;
    let send_id = comment.id;
    let select_id = comment.id;
    let sendable = comment.sendable;
    let is_selected = self.selected.contains(&comment.id);

    h_flex()
      .id(("review-comment", comment.id as usize))
      .w_full()
      .items_center()
      .gap_2()
      .pl_5()
      .pr_1()
      .py_1()
      .rounded_sm()
      .hover(|this| this.bg(theme.accent))
      .cursor_pointer()
      .on_click(cx.listener(move |_, _, _, cx| {
        let (path, line) = open.clone();
        cx.emit(ReviewListEvent::OpenComment { path, line });
      }))
      // Always there, so every row's text starts on the same column.
      .child(
        div()
          .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
          .child(
            Checkbox::new(("review-comment-select", select_id as usize))
              .small()
              .checked(is_selected)
              .disabled(!sendable)
              .tooltip("Send only the selected comments")
              .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.toggle_comment(select_id, cx);
              })),
          ),
      )
      .child(
        div()
          .text_xs()
          .text_color(theme.muted_foreground)
          .child(comment.line_label.clone()),
      )
      .child(
        div()
          .flex_1()
          .min_w_0()
          .text_sm()
          .text_color(theme.foreground)
          .child(comment.excerpt.clone()),
      )
      .when_some(review_state_label(&comment.state), |this, label| {
        this.child(
          ui::StatusTag::new(match comment.state {
            LocalAgentReviewCommentState::Outdated => theme.status_orange(),
            _ => theme.muted_foreground,
          })
          .outline()
          .small()
          .child(label),
        )
      })
      .when(sendable, |this| {
        this.child(
          div()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
              Button::new(("review-comment-send", send_id as usize))
                .ghost()
                .xsmall()
                .compact()
                .icon(Icon::new(UiIconName::ArrowUp))
                .tooltip("Send this comment to the agent")
                .on_click(cx.listener(move |_, _, _, cx| {
                  cx.stop_propagation();
                  cx.emit(ReviewListEvent::SendComment { id: send_id });
                })),
            ),
        )
      })
      .child(
        div()
          .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
          .child(
            Button::new(("review-comment-delete", comment.id as usize))
              .ghost()
              .xsmall()
              .compact()
              .icon(Icon::new(UiIconName::Trash))
              .tooltip("Delete comment")
              .on_click(cx.listener(move |_, _, _, cx| {
                cx.stop_propagation();
                cx.emit(ReviewListEvent::DeleteComment { id: delete_id });
              })),
          ),
      )
      .into_any_element()
  }

  fn render_actions(&self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let sendable = self.sendable_count();
    let send_count = self.send_count();
    let everything_is_selected = self.everything_is_selected();

    h_flex()
      .w_full()
      .items_center()
      .justify_between()
      .gap_2()
      .px_1()
      .py_1()
      .border_t_1()
      .border_color(theme.border)
      .child(
        h_flex()
          .items_center()
          .gap_1()
          .child(
            Button::new("review-list-select-all")
              .debug_selector(|| REVIEW_LIST_SELECT_ALL_DEBUG_SELECTOR.to_string())
              .ghost()
              .small()
              .compact()
              .label(if everything_is_selected {
                "Deselect all"
              } else {
                "Select all"
              })
              .disabled(sendable == 0)
              .on_click(cx.listener(|this, _, _, cx| this.toggle_select_all(cx))),
          )
          .child(
            Button::new("review-list-discard")
              .debug_selector(|| REVIEW_LIST_DISCARD_DEBUG_SELECTOR.to_string())
              .ghost()
              .small()
              .compact()
              .label("Discard")
              .tooltip("Delete every comment of this review")
              .on_click(cx.listener(|_, _, _, cx| cx.emit(ReviewListEvent::DiscardReview))),
          ),
      )
      .child(
        Button::new("review-list-send")
          .debug_selector(|| REVIEW_LIST_SEND_DEBUG_SELECTOR.to_string())
          .primary()
          .small()
          .compact()
          .label(if send_count == 1 {
            "Send 1 comment".to_string()
          } else {
            format!("Send {send_count} comments")
          })
          .disabled(send_count == 0)
          .on_click(cx.listener(|_, _, _, cx| cx.emit(ReviewListEvent::SendReview))),
      )
      .into_any_element()
  }
}

impl Render for ReviewList {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    if self.comments.is_empty() {
      return v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .p_4()
        .child(
          div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child("Comment on a diff line to start a review."),
        )
        .into_any_element();
    }

    let mut list = v_flex().w_full().gap_0p5();
    for (path, comments) in group_review_comments_by_file(&self.comments) {
      list = list.child(self.render_file_header(&path, comments.len(), cx));
      if self.collapsed_files.contains(&path) {
        continue;
      }
      for comment in &comments {
        list = list.child(self.render_comment_row(comment, cx));
      }
    }

    v_flex()
      .size_full()
      .min_h_0()
      .child(
        div()
          .id("review-list-scroll")
          .flex_1()
          .min_h(px(0.0))
          .overflow_y_scroll()
          .child(list),
      )
      .child(self.render_actions(cx))
      .into_any_element()
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use editor::ReviewCommentSide;
  use std::sync::Arc;

  fn comment(
    id: u64,
    path: &str,
    line: usize,
    body: &str,
    state: LocalAgentReviewCommentState,
  ) -> LocalAgentReviewComment {
    LocalAgentReviewComment {
      id,
      in_reply_to_id: None,
      path: PathBuf::from(path),
      line,
      side: ReviewCommentSide::Right,
      start_line: None,
      start_side: None,
      body: Arc::from(body),
      original_start_line: Some(line + 1),
      original_lines: Vec::new(),
      state,
    }
  }

  #[test]
  fn a_row_shows_the_first_line_that_says_something() {
    assert_eq!(
      review_comment_excerpt("\n\n  extract this  \nand that"),
      "extract this"
    );
    assert_eq!(review_comment_excerpt(""), "");

    let long = "a".repeat(REVIEW_EXCERPT_MAX_CHARS + 10);
    let excerpt = review_comment_excerpt(&long);
    assert_eq!(excerpt.chars().count(), REVIEW_EXCERPT_MAX_CHARS + 1);
    assert!(excerpt.ends_with('…'));
  }

  #[test]
  fn comments_are_grouped_by_file_in_reading_order() {
    let comments = review_panel_comments(&[
      comment(
        3,
        "src/b.rs",
        4,
        "third",
        LocalAgentReviewCommentState::Draft,
      ),
      comment(
        1,
        "src/a.rs",
        9,
        "second",
        LocalAgentReviewCommentState::Draft,
      ),
      comment(
        2,
        "src/a.rs",
        2,
        "first",
        LocalAgentReviewCommentState::Draft,
      ),
    ]);

    let groups = group_review_comments_by_file(&comments);

    assert_eq!(
      groups
        .iter()
        .map(|(path, rows)| (path.to_string_lossy().to_string(), rows.len()))
        .collect::<Vec<_>>(),
      vec![("src/a.rs".to_string(), 2), ("src/b.rs".to_string(), 1)]
    );
    assert_eq!(
      groups[0]
        .1
        .iter()
        .map(|row| row.excerpt.as_str())
        .collect::<Vec<_>>(),
      vec!["first", "second"]
    );
  }

  #[test]
  fn a_row_only_carries_a_tag_when_the_comment_left_the_draft_state() {
    assert_eq!(
      review_state_label(&LocalAgentReviewCommentState::Draft),
      None
    );
    assert_eq!(
      review_state_label(&LocalAgentReviewCommentState::Copied),
      Some("Sent")
    );
    assert_eq!(
      review_state_label(&LocalAgentReviewCommentState::Addressed),
      Some("Addressed")
    );
    assert_eq!(
      review_state_label(&LocalAgentReviewCommentState::Outdated),
      Some("Outdated")
    );
  }

  fn add_review_list_window(
    cx: &mut gpui::TestAppContext,
  ) -> (gpui::Entity<ReviewList>, &mut gpui::VisualTestContext) {
    use gpui::AppContext as _;

    cx.update(gpui_component::init);
    let mut mounted = None;
    let (_root, cx) = cx.add_window_view(|window, cx| {
      let list = cx.new(|_| ReviewList::new());
      mounted = Some(list.clone());
      gpui_component::Root::new(list, window, cx)
    });
    (mounted.expect("review list"), cx)
  }

  fn batch() -> Vec<ReviewPanelComment> {
    review_panel_comments(&[
      comment(
        1,
        "src/a.rs",
        1,
        "first",
        LocalAgentReviewCommentState::Draft,
      ),
      comment(
        2,
        "src/a.rs",
        4,
        "second",
        LocalAgentReviewCommentState::Draft,
      ),
      comment(
        3,
        "src/b.rs",
        2,
        "third",
        LocalAgentReviewCommentState::Draft,
      ),
      comment(
        4,
        "src/b.rs",
        7,
        "done",
        LocalAgentReviewCommentState::Addressed,
      ),
    ])
  }

  #[gpui::test]
  async fn nothing_ticked_sends_the_whole_batch(cx: &mut gpui::TestAppContext) {
    let (list, cx) = add_review_list_window(cx);

    list.update(cx, |list, cx| list.set_comments(batch(), cx));

    list.read_with(cx, |list, _| {
      // Four rows, but the addressed one has nothing left to send.
      assert_eq!(list.comments().len(), 4);
      assert_eq!(list.sendable_count(), 3);
      assert_eq!(list.send_count(), 3);
      assert!(list.selected_ids().is_empty());
      assert!(!list.everything_is_selected());
    });
  }

  #[gpui::test]
  async fn ticking_comments_narrows_what_send_sends(cx: &mut gpui::TestAppContext) {
    let (list, cx) = add_review_list_window(cx);
    list.update(cx, |list, cx| list.set_comments(batch(), cx));

    list.update(cx, |list, cx| {
      list.toggle_comment(2, cx);
      list.toggle_comment(3, cx);
      list.toggle_comment(3, cx);
    });

    list.read_with(cx, |list, _| {
      assert_eq!(list.selected_ids(), &HashSet::from([2]));
      assert_eq!(list.send_count(), 1);
    });

    // A send takes the ticks of what left with it, and leaves the rest.
    list.update(cx, |list, cx| list.deselect(&HashSet::from([3]), cx));
    list.read_with(cx, |list, _| {
      assert_eq!(list.selected_ids(), &HashSet::from([2]));
    });
    list.update(cx, |list, cx| list.deselect(&HashSet::from([2]), cx));
    list.read_with(cx, |list, _| {
      assert!(list.selected_ids().is_empty());
      assert_eq!(list.send_count(), 3);
    });
  }

  #[gpui::test]
  async fn a_file_checkbox_takes_its_whole_file(cx: &mut gpui::TestAppContext) {
    let (list, cx) = add_review_list_window(cx);
    list.update(cx, |list, cx| list.set_comments(batch(), cx));

    list.update(cx, |list, cx| {
      list.toggle_file_selection(PathBuf::from("src/a.rs"), cx)
    });
    list.read_with(cx, |list, _| {
      assert_eq!(list.selected_ids(), &HashSet::from([1, 2]));
    });

    // The addressed comment of the other file stays out of it.
    list.update(cx, |list, cx| {
      list.toggle_file_selection(PathBuf::from("src/b.rs"), cx)
    });
    list.read_with(cx, |list, _| {
      assert_eq!(list.selected_ids(), &HashSet::from([1, 2, 3]));
      assert!(list.everything_is_selected());
    });

    // Ticked again, the file lets go.
    list.update(cx, |list, cx| {
      list.toggle_file_selection(PathBuf::from("src/a.rs"), cx)
    });
    list.read_with(cx, |list, _| {
      assert_eq!(list.selected_ids(), &HashSet::from([3]));
    });
  }

  #[gpui::test]
  async fn select_all_starts_from_everything_and_gives_it_back(cx: &mut gpui::TestAppContext) {
    let (list, cx) = add_review_list_window(cx);
    list.update(cx, |list, cx| list.set_comments(batch(), cx));

    list.update(cx, |list, cx| list.toggle_select_all(cx));
    list.read_with(cx, |list, _| {
      assert_eq!(list.selected_ids(), &HashSet::from([1, 2, 3]));
      assert_eq!(list.send_count(), 3);
    });

    list.update(cx, |list, cx| list.toggle_select_all(cx));
    list.read_with(cx, |list, _| {
      assert!(list.selected_ids().is_empty());
      // Back to the whole batch, not to nothing.
      assert_eq!(list.send_count(), 3);
    });
  }

  #[gpui::test]
  async fn the_select_all_button_paints_and_takes_the_whole_batch(cx: &mut gpui::TestAppContext) {
    let (list, cx) = add_review_list_window(cx);
    list.update(cx, |list, cx| list.set_comments(batch(), cx));
    cx.run_until_parked();

    let button = cx
      .debug_bounds(REVIEW_LIST_SELECT_ALL_DEBUG_SELECTOR)
      .expect("select all bounds");
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    cx.run_until_parked();

    list.read_with(cx, |list, _| {
      assert_eq!(list.selected_ids(), &HashSet::from([1, 2, 3]));
    });

    // The same button gives everything back once all of it is ticked.
    let button = cx
      .debug_bounds(REVIEW_LIST_SELECT_ALL_DEBUG_SELECTOR)
      .expect("deselect all bounds");
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    cx.run_until_parked();

    list.read_with(cx, |list, _| assert!(list.selected_ids().is_empty()));
  }

  #[gpui::test]
  async fn a_comment_leaving_the_batch_leaves_the_selection(cx: &mut gpui::TestAppContext) {
    let (list, cx) = add_review_list_window(cx);
    list.update(cx, |list, cx| list.set_comments(batch(), cx));
    list.update(cx, |list, cx| list.toggle_select_all(cx));

    // The agent addressed one and another was deleted from the batch.
    list.update(cx, |list, cx| {
      list.set_comments(
        review_panel_comments(&[
          comment(
            1,
            "src/a.rs",
            1,
            "first",
            LocalAgentReviewCommentState::Draft,
          ),
          comment(
            2,
            "src/a.rs",
            4,
            "second",
            LocalAgentReviewCommentState::Addressed,
          ),
        ]),
        cx,
      )
    });

    list.read_with(cx, |list, _| {
      assert_eq!(list.selected_ids(), &HashSet::from([1]));
      assert_eq!(list.send_count(), 1);
    });
  }

  #[test]
  fn a_comment_the_agent_addressed_still_has_a_row() {
    let rows = review_panel_comments(&[comment(
      1,
      "src/a.rs",
      2,
      "done",
      LocalAgentReviewCommentState::Addressed,
    )]);

    // It is out of the diff and out of the batch, but the panel is where you
    // see that the agent dealt with it.
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].state, LocalAgentReviewCommentState::Addressed);
  }
}
