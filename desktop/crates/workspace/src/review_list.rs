//! The Review tab: the comments waiting to go out, one section per destination.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use gpui::{
  AnyElement, App, AppContext as _, Context, Entity, Focusable as _, InteractiveElement,
  IntoElement, KeyDownEvent, MouseButton, ParentElement, Render, StatefulInteractiveElement as _,
  Styled, WeakEntity, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
  ActiveTheme as _, Disableable as _, Icon, IconName, IndexPath, Sizable,
  button::{Button, ButtonVariants as _},
  checkbox::Checkbox,
  h_flex,
  list::{List, ListDelegate, ListEvent, ListItem, ListState},
  tag::Tag,
  v_flex,
};
use ui::{SelectableRowStyle, UiIconName, selectable_list_item};

use crate::open_intent::OpenIntent;

use crate::agent_review::{
  LocalAgentReviewComment, LocalAgentReviewCommentState, agent_review_line_label,
  agent_review_state_is_sendable,
};
use crate::changes_list::split_path_label;

pub(crate) const REVIEW_LIST_SEND_DEBUG_SELECTOR: &str = "review-list-send";
pub(crate) const REVIEW_LIST_DISCARD_DEBUG_SELECTOR: &str = "review-list-discard";
pub(crate) const REVIEW_LIST_SELECT_ALL_DEBUG_SELECTOR: &str = "review-list-select-all";
pub(crate) const REVIEW_LIST_SUBMIT_DEBUG_SELECTOR: &str = "review-list-submit";
pub(crate) const REVIEW_LIST_FOOTER_DESTINATION_DEBUG_SELECTOR: &str =
  "review-list-footer-destination";

fn review_list_section_header_debug_selector(section: ReviewSection) -> &'static str {
  match section {
    ReviewSection::Agent => "review-list-section-agent",
    ReviewSection::PullRequest => "review-list-section-pull-request",
  }
}

/// Longest excerpt shown on a row before it is cut.
const REVIEW_EXCERPT_MAX_CHARS: usize = 120;

/// Where the comments of a section go. It is the only thing that separates them,
/// and it is what the section header says instead of a colour code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ReviewSection {
  Agent,
  PullRequest,
}

impl ReviewSection {
  pub(crate) const ALL: [Self; 2] = [Self::Agent, Self::PullRequest];

  fn title(self) -> &'static str {
    match self {
      Self::Agent => "To the agent",
      Self::PullRequest => "To this pull request",
    }
  }

  fn id_prefix(self) -> &'static str {
    match self {
      Self::Agent => "agent",
      Self::PullRequest => "pull-request",
    }
  }
}

/// What a row says about itself beyond its text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReviewRowStatus {
  Draft,
  Sent,
  Pending,
  /// A pull request comment the diff moved under: GitHub keeps it, anchored to
  /// the line it was written against.
  Outdated,
}

pub(crate) fn review_row_status_label(status: ReviewRowStatus) -> Option<&'static str> {
  match status {
    // The ordinary case says nothing, and a pending comment has its section to
    // say it for the whole list.
    ReviewRowStatus::Draft | ReviewRowStatus::Pending => None,
    ReviewRowStatus::Sent => Some("Sent"),
    ReviewRowStatus::Outdated => Some("Outdated"),
  }
}

fn agent_row_status(state: &LocalAgentReviewCommentState) -> ReviewRowStatus {
  match state {
    LocalAgentReviewCommentState::Draft => ReviewRowStatus::Draft,
    LocalAgentReviewCommentState::Sent => ReviewRowStatus::Sent,
  }
}

pub(crate) enum ReviewListEvent {
  /// Take me to the lines this comment is about, on the surface it belongs to.
  OpenComment {
    section: ReviewSection,
    path: PathBuf,
    line: usize,
    intent: OpenIntent,
  },
  DeleteComment {
    section: ReviewSection,
    id: u64,
  },
  /// One comment on its own, whatever the selection holds.
  SendComment {
    id: u64,
  },
  SendReview,
  DiscardReview,
  /// Finish the pull request review: the decision and its message are asked for
  /// where the whole batch is visible.
  SubmitReview,
}

/// What a row needs, and nothing about where the comment came from: a pull
/// request's pending comments feed the same panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReviewPanelComment {
  pub id: u64,
  pub section: ReviewSection,
  pub path: PathBuf,
  pub line: usize,
  pub line_label: String,
  pub excerpt: String,
  pub status: ReviewRowStatus,
  /// Whether this row takes part in a partial send. Only the agent's do: GitHub
  /// submits a review whole.
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

pub(crate) fn sort_review_panel_comments(rows: &mut [ReviewPanelComment]) {
  rows.sort_by(|a, b| {
    a.path
      .cmp(&b.path)
      .then_with(|| a.line.cmp(&b.line))
      .then_with(|| a.id.cmp(&b.id))
  });
}

pub(crate) fn review_panel_comments(
  comments: &[LocalAgentReviewComment],
) -> Vec<ReviewPanelComment> {
  let mut rows = comments
    .iter()
    .map(|comment| ReviewPanelComment {
      id: comment.id,
      section: ReviewSection::Agent,
      path: comment.path.clone(),
      // The line a reader would name, which is what opening the row asks for.
      line: comment.line.saturating_add(1),
      line_label: agent_review_line_label(comment),
      excerpt: review_comment_excerpt(comment.body.as_ref()),
      status: agent_row_status(&comment.state),
      sendable: agent_review_state_is_sendable(&comment.state),
    })
    .collect::<Vec<_>>();
  sort_review_panel_comments(&mut rows);
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

/// The three levels of the panel (destination, file, comment) flattened onto the
/// two the list has: a file is a row of its section, and a collapsed one is the
/// only row its comments leave behind.
#[derive(Clone)]
enum ReviewRow {
  FileHeader { path: PathBuf, count: usize },
  Comment(Box<ReviewPanelComment>),
}

struct ReviewRowsSection {
  section: ReviewSection,
  rows: Vec<ReviewRow>,
}

struct ReviewRowsDelegate {
  owner: WeakEntity<ReviewList>,
  sections: Vec<ReviewRowsSection>,
  /// A single destination has its title pinned above the list instead, where the
  /// Changes tab puts its own.
  show_section_headers: bool,
  selected_index: Option<IndexPath>,
}

impl ReviewRowsDelegate {
  fn new(owner: WeakEntity<ReviewList>) -> Self {
    Self {
      owner,
      sections: Vec::new(),
      show_section_headers: false,
      selected_index: None,
    }
  }

  fn row_at(&self, ix: IndexPath) -> Option<(ReviewSection, ReviewRow)> {
    let section = self.sections.get(ix.section)?;
    Some((section.section, section.rows.get(ix.row)?.clone()))
  }
}

impl ListDelegate for ReviewRowsDelegate {
  type Item = ListItem;

  fn sections_count(&self, _cx: &App) -> usize {
    self.sections.len()
  }

  fn items_count(&self, section: usize, _cx: &App) -> usize {
    self
      .sections
      .get(section)
      .map_or(0, |section| section.rows.len())
  }

  fn render_section_header(
    &mut self,
    section: usize,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> Option<impl IntoElement> {
    if !self.show_section_headers {
      return None;
    }
    let section = self.sections.get(section)?.section;
    let owner = self.owner.upgrade()?;
    Some(owner.update(cx, |list, cx| list.render_section_header(section, cx)))
  }

  fn render_item(
    &mut self,
    ix: IndexPath,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> Option<Self::Item> {
    let theme = cx.theme().clone();
    let (section, row) = self.row_at(ix)?;
    let owner = self.owner.upgrade()?;
    let selected = self
      .selected_index
      .map(|selected| selected.eq_row(ix))
      .unwrap_or(false);
    let content = owner.update(cx, |list, cx| match &row {
      ReviewRow::FileHeader { path, count } => list.render_file_header(section, path, *count, cx),
      ReviewRow::Comment(comment) => list.render_comment_row(comment, cx),
    });
    let mut item = selectable_list_item(ix, selected, SelectableRowStyle::Inset, &theme);
    if let ReviewRow::Comment(comment) = &row {
      let (prefix, id) = (section.id_prefix(), comment.id);
      item = item.debug_selector(move || format!("review-comment-{prefix}-{id}"));
    }
    Some(item.child(content))
  }

  fn set_selected_index(
    &mut self,
    ix: Option<IndexPath>,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) {
    self.selected_index = ix;
    cx.notify();
  }
}

pub(crate) struct ReviewList {
  agent_comments: Vec<ReviewPanelComment>,
  pull_request_comments: Vec<ReviewPanelComment>,
  collapsed_files: HashSet<(ReviewSection, PathBuf)>,
  /// Agent comments only, and empty means the whole batch goes: nobody loses a
  /// comment by not ticking it.
  selected: HashSet<u64>,
  /// Which destination the footer acts on: the section of the row last walked
  /// to. One set of actions at the bottom, and it follows the keyboard.
  active_section: Option<ReviewSection>,
  list: Entity<ListState<ReviewRowsDelegate>>,
}

impl gpui::EventEmitter<ReviewListEvent> for ReviewList {}

impl ReviewList {
  pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
    let owner = cx.entity().downgrade();
    let list = cx
      .new(|cx| ListState::new(ReviewRowsDelegate::new(owner), window, cx).reset_on_cancel(false));

    let _ = list.read(cx).focus_handle(cx).tab_stop(true).tab_index(0);

    // Walking the list shows each comment; a click or Enter hands the editor the
    // keyboard. On a file row there is nothing to read, so both fold it.
    cx.subscribe(&list, |this, state, event: &ListEvent, cx| {
      let (ix, intent) = match event {
        ListEvent::Select(ix) => (*ix, OpenIntent::Browse),
        ListEvent::Confirm(ix) => (*ix, OpenIntent::Open),
        _ => return,
      };
      let Some((section, row)) = state.read(cx).delegate().row_at(ix) else {
        return;
      };
      this.active_section = Some(section);
      cx.notify();
      match row {
        ReviewRow::Comment(comment) => cx.emit(ReviewListEvent::OpenComment {
          section,
          path: comment.path.clone(),
          line: comment.line,
          intent,
        }),
        ReviewRow::FileHeader { path, .. } => {
          if intent.takes_focus() {
            this.toggle_file(section, path, cx);
          }
        }
      }
    })
    .detach();

    Self {
      agent_comments: Vec::new(),
      pull_request_comments: Vec::new(),
      collapsed_files: HashSet::new(),
      selected: HashSet::new(),
      active_section: None,
      list,
    }
  }

  /// Nothing to walk: the panel shows its empty state instead of a list.
  pub(crate) fn is_empty(&self) -> bool {
    self.sections().next().is_none()
  }

  #[cfg(test)]
  pub(crate) fn keyboard_selected_row(&self, cx: &App) -> Option<IndexPath> {
    self.list.read(cx).delegate().selected_index
  }

  pub(crate) fn is_focused(&self, window: &Window, cx: &App) -> bool {
    self
      .list
      .read(cx)
      .focus_handle(cx)
      .contains_focused(window, cx)
  }

  pub(crate) fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
    let handle = self.list.read(cx).focus_handle(cx);
    window.focus(&handle, cx);
  }

  /// The rows the list walks, rebuilt whenever the comments or the folds change.
  fn sync_rows(&mut self, cx: &mut Context<Self>) {
    let sections = self
      .sections()
      .map(|section| {
        let mut rows = Vec::new();
        for (path, comments) in group_review_comments_by_file(self.comments(section)) {
          rows.push(ReviewRow::FileHeader {
            path: path.clone(),
            count: comments.len(),
          });
          if self.collapsed_files.contains(&(section, path)) {
            continue;
          }
          rows.extend(
            comments
              .into_iter()
              .map(|comment| ReviewRow::Comment(Box::new(comment))),
          );
        }
        ReviewRowsSection { section, rows }
      })
      .collect::<Vec<_>>();
    let show_section_headers = sections.len() > 1;

    self.list.update(cx, |state, cx| {
      let delegate = state.delegate_mut();
      delegate.sections = sections;
      delegate.show_section_headers = show_section_headers;
      cx.notify();
    });
  }

  /// Left folds the file the selection sits in, right unfolds it, whether the
  /// row is the file or one of its comments. Same gesture as the trees.
  fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
    let keystroke = &event.keystroke;
    if keystroke.modifiers.modified() {
      return;
    }
    let collapse = match keystroke.key.as_str() {
      "left" => true,
      "right" => false,
      _ => return,
    };
    if !self
      .list
      .read(cx)
      .focus_handle(cx)
      .contains_focused(window, cx)
    {
      return;
    }
    let Some(ix) = self.list.read(cx).delegate().selected_index else {
      return;
    };
    let Some((section, row)) = self.list.read(cx).delegate().row_at(ix) else {
      return;
    };
    let path = match row {
      ReviewRow::FileHeader { path, .. } => path,
      ReviewRow::Comment(comment) => comment.path.clone(),
    };
    let key = (section, path);
    if self.collapsed_files.contains(&key) == collapse {
      return;
    }
    if collapse {
      self.collapsed_files.insert(key);
    } else {
      self.collapsed_files.remove(&key);
    }
    cx.stop_propagation();
    self.sync_rows(cx);
    cx.notify();
  }

  /// A tick or a count changed without moving a row: the list still has to
  /// repaint, and it does not watch its owner.
  fn notify_rows(&mut self, cx: &mut Context<Self>) {
    self.list.update(cx, |_, cx| cx.notify());
    cx.notify();
  }

  pub(crate) fn comments(&self, section: ReviewSection) -> &[ReviewPanelComment] {
    match section {
      ReviewSection::Agent => &self.agent_comments,
      ReviewSection::PullRequest => &self.pull_request_comments,
    }
  }

  fn comments_mut(&mut self, section: ReviewSection) -> &mut Vec<ReviewPanelComment> {
    match section {
      ReviewSection::Agent => &mut self.agent_comments,
      ReviewSection::PullRequest => &mut self.pull_request_comments,
    }
  }

  /// One section at a time: the two destinations have their own lifetime, so a
  /// reload of one must not take the other's rows away.
  pub(crate) fn set_comments(
    &mut self,
    section: ReviewSection,
    comments: Vec<ReviewPanelComment>,
    cx: &mut Context<Self>,
  ) {
    if self.comments(section) == comments.as_slice() {
      return;
    }
    *self.comments_mut(section) = comments;
    let live = self
      .sections()
      .flat_map(|section| self.comments(section))
      .map(|comment| (comment.section, comment.path.clone()))
      .collect::<HashSet<_>>();
    self.collapsed_files.retain(|key| live.contains(key));
    let sendable_ids = self.sendable_ids().collect::<HashSet<_>>();
    self.selected.retain(|id| sendable_ids.contains(id));
    self.sync_rows(cx);
    cx.notify();
  }

  fn sections(&self) -> impl Iterator<Item = ReviewSection> + '_ {
    ReviewSection::ALL
      .into_iter()
      .filter(|section| !self.comments(*section).is_empty())
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
      self.notify_rows(cx);
    }
  }

  fn sendable_ids(&self) -> impl Iterator<Item = u64> + '_ {
    self
      .agent_comments
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
      // A tick promises a partial send, which only the agent's comments can be
      // part of: a pull request review is submitted whole.
      if !self.sendable_ids().any(|id| id == comment_id) {
        return;
      }
      self.selected.insert(comment_id);
    }
    self.notify_rows(cx);
  }

  fn file_sendable_ids(&self, path: &Path) -> Vec<u64> {
    self
      .agent_comments
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
    self.notify_rows(cx);
  }

  pub(crate) fn toggle_select_all(&mut self, cx: &mut Context<Self>) {
    if self.everything_is_selected() {
      self.selected.clear();
    } else {
      self.selected = self.sendable_ids().collect();
    }
    self.notify_rows(cx);
  }

  fn toggle_file(&mut self, section: ReviewSection, path: PathBuf, cx: &mut Context<Self>) {
    let key = (section, path);
    if !self.collapsed_files.remove(&key) {
      self.collapsed_files.insert(key);
    }
    self.sync_rows(cx);
    cx.notify();
  }

  fn render_file_header(
    &self,
    section: ReviewSection,
    path: &Path,
    count: usize,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let theme = cx.theme().clone();
    let collapsed = self
      .collapsed_files
      .contains(&(section, path.to_path_buf()));
    let (dir, file) = split_path_label(path);
    let sendable_ids = if section == ReviewSection::Agent {
      self.file_sendable_ids(path)
    } else {
      Vec::new()
    };
    let selectable = !sendable_ids.is_empty();
    let file_is_selected = selectable && sendable_ids.iter().all(|id| self.selected.contains(id));
    let select_path = path.to_path_buf();

    h_flex()
      .id(gpui::SharedString::from(format!(
        "review-file-{}-{}",
        section.id_prefix(),
        path.to_string_lossy()
      )))
      .w_full()
      .items_center()
      .gap_1()
      .px_1()
      .cursor_pointer()
      .when(section == ReviewSection::Agent, |this| {
        this.child(
          div()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
              Checkbox::new(gpui::SharedString::from(format!(
                "review-file-select-{}",
                path.to_string_lossy()
              )))
              .small()
              .checked(file_is_selected)
              .disabled(!selectable)
              .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.toggle_file_selection(select_path.clone(), cx);
              })),
            ),
        )
      })
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
    let section = comment.section;
    let delete_id = comment.id;
    let send_id = comment.id;
    let select_id = comment.id;
    let sendable = comment.sendable;
    let is_selected = self.selected.contains(&comment.id);

    h_flex()
      .id((
        match section {
          ReviewSection::Agent => "review-comment-agent",
          ReviewSection::PullRequest => "review-comment-pull-request",
        },
        comment.id as usize,
      ))
      .w_full()
      .items_center()
      .gap_2()
      .pl_5()
      .pr_1()
      // Always there in the agent section, so every row's text starts on the
      // same column.
      .when(section == ReviewSection::Agent, |this| {
        this.child(
          div()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
              Checkbox::new(("review-comment-select", select_id as usize))
                .small()
                .checked(is_selected)
                .disabled(!sendable)
                .on_click(cx.listener(move |this, _, _, cx| {
                  cx.stop_propagation();
                  this.toggle_comment(select_id, cx);
                })),
            ),
        )
      })
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
          .truncate()
          .child(comment.excerpt.clone()),
      )
      .when_some(review_row_status_label(comment.status), |this, label| {
        this.child(Tag::secondary().outline().small().child(label))
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
                cx.emit(ReviewListEvent::DeleteComment {
                  section,
                  id: delete_id,
                });
              })),
          ),
      )
      .into_any_element()
  }

  /// The section header says where its comments go. In the agent's, the master
  /// checkbox sits on the column the file checkboxes use, so ticking down the
  /// list reads as one column.
  fn render_section_header(&self, section: ReviewSection, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let sendable = self.sendable_count();
    let selected = self.selected.len();
    let count = self.comments(section).len();

    // The whole header takes the click: a tick box that small is a poor target,
    // and its title says what would be ticked.
    let takes_the_selection = section == ReviewSection::Agent && sendable > 0;

    h_flex()
      .id(gpui::SharedString::from(format!(
        "review-list-section-{}",
        section.id_prefix()
      )))
      .debug_selector(move || review_list_section_header_debug_selector(section).to_string())
      .w_full()
      .items_center()
      .gap_1()
      .px_2()
      .py_1()
      .border_b_1()
      .border_color(theme.border)
      .when(takes_the_selection, |this| {
        this
          .cursor_pointer()
          .on_click(cx.listener(|this, _, _, cx| this.toggle_select_all(cx)))
      })
      .when(section == ReviewSection::Agent, |this| {
        this.child(
          div()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
              Checkbox::new("review-list-select-all")
                .debug_selector(|| REVIEW_LIST_SELECT_ALL_DEBUG_SELECTOR.to_string())
                .small()
                .checked(self.everything_is_selected())
                .disabled(sendable == 0)
                .on_click(cx.listener(|this, _, _, cx| {
                  cx.stop_propagation();
                  this.toggle_select_all(cx);
                })),
            ),
        )
      })
      .child(
        div()
          .flex_1()
          .min_w_0()
          .text_xs()
          .text_color(theme.muted_foreground)
          .truncate()
          .child(section.title()),
      )
      .child(
        div()
          .text_xs()
          .text_color(theme.muted_foreground)
          .child(match section {
            ReviewSection::Agent if selected > 0 => format!("{selected} selected"),
            _ => count.to_string(),
          }),
      )
      .into_any_element()
  }

  /// Where the footer sends what it acts on. The row last walked to decides;
  /// before anything is walked, the first section on screen does.
  fn footer_section(&self) -> Option<ReviewSection> {
    let first = self.sections().next()?;
    match self.active_section {
      Some(active) if self.sections().any(|section| section == active) => Some(active),
      _ => Some(first),
    }
  }

  /// One footer, the actions of one destination. Two destinations in the list
  /// means the footer has to say which one it is talking about.
  fn render_footer(
    &self,
    section: ReviewSection,
    names_its_destination: bool,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let theme = cx.theme().clone();
    let send_count = self.send_count();

    h_flex()
      .w_full()
      .items_center()
      .justify_between()
      .gap_2()
      .p_2()
      .border_t_1()
      .border_color(theme.border)
      .child(
        h_flex()
          .items_center()
          .gap_2()
          .when(names_its_destination, |this| {
            this.child(
              div()
                .debug_selector(|| REVIEW_LIST_FOOTER_DESTINATION_DEBUG_SELECTOR.to_string())
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(section.title()),
            )
          })
          .when(section == ReviewSection::Agent, |this| {
            this.child(
              Button::new("review-list-discard")
                .debug_selector(|| REVIEW_LIST_DISCARD_DEBUG_SELECTOR.to_string())
                .ghost()
                .small()
                .compact()
                .label("Discard")
                .tooltip("Delete every comment you have not sent yet")
                .disabled(self.sendable_count() == 0)
                .on_click(cx.listener(|_, _, _, cx| cx.emit(ReviewListEvent::DiscardReview))),
            )
          }),
      )
      .child(match section {
        ReviewSection::Agent => Button::new("review-list-send")
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
        ReviewSection::PullRequest => Button::new("review-list-submit")
          .debug_selector(|| REVIEW_LIST_SUBMIT_DEBUG_SELECTOR.to_string())
          .primary()
          .small()
          .compact()
          .label("Submit review")
          .tooltip("Send these comments to GitHub with a decision")
          .on_click(cx.listener(|_, _, _, cx| cx.emit(ReviewListEvent::SubmitReview))),
      })
      .into_any_element()
  }
}

impl Render for ReviewList {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let sections = self.sections().collect::<Vec<_>>();

    if sections.is_empty() {
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

    // A single destination pins its title above the rows; two of them carry
    // their titles inside the list, where the rows separate them. Either way one
    // footer sits at the bottom, for the destination the rows point at.
    let mut panel = v_flex()
      .id("review-list")
      .size_full()
      .min_h_0()
      .on_key_down(cx.listener(Self::on_key_down));
    if let [section] = sections.as_slice() {
      panel = panel.child(self.render_section_header(*section, cx));
    }
    panel = panel.child(
      div()
        .id("review-list-rows")
        .flex_1()
        .min_h(px(0.0))
        .px_1()
        .py_1()
        .child(List::new(&self.list).w_full().min_h_0()),
    );
    if let Some(section) = self.footer_section() {
      panel = panel.child(self.render_footer(section, sections.len() > 1, cx));
    }
    panel.into_any_element()
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

  fn pull_request_row(id: u64, path: &str, line: usize) -> ReviewPanelComment {
    ReviewPanelComment {
      id,
      section: ReviewSection::PullRequest,
      path: PathBuf::from(path),
      line,
      line_label: format!("L{line}"),
      excerpt: "pending".to_string(),
      status: ReviewRowStatus::Pending,
      sendable: false,
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
  fn a_pending_pull_request_comment_carries_no_badge() {
    assert_eq!(review_row_status_label(ReviewRowStatus::Draft), None);
    assert_eq!(review_row_status_label(ReviewRowStatus::Pending), None);
    assert_eq!(review_row_status_label(ReviewRowStatus::Sent), Some("Sent"));
    assert_eq!(
      review_row_status_label(ReviewRowStatus::Outdated),
      Some("Outdated")
    );
  }

  #[test]
  fn a_row_carries_the_line_a_reader_would_name() {
    let rows = review_panel_comments(&[comment(
      1,
      "src/a.rs",
      4,
      "here",
      LocalAgentReviewCommentState::Draft,
    )]);

    assert_eq!(rows[0].line_label, "L5");
    assert_eq!(rows[0].line, 5);
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

  fn add_review_list_window(
    cx: &mut gpui::TestAppContext,
  ) -> (gpui::Entity<ReviewList>, &mut gpui::VisualTestContext) {
    use gpui::AppContext as _;

    cx.update(gpui_component::init);
    let mut mounted = None;
    let (_root, cx) = cx.add_window_view(|window, cx| {
      let list = cx.new(|cx| ReviewList::new(window, cx));
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
      comment(4, "src/b.rs", 7, "gone", LocalAgentReviewCommentState::Sent),
    ])
  }

  #[gpui::test]
  async fn the_keyboard_walks_the_rows_and_folds_a_file(cx: &mut gpui::TestAppContext) {
    let (list, cx) = add_review_list_window(cx);
    list.update(cx, |list, cx| {
      list.set_comments(ReviewSection::Agent, batch(), cx)
    });
    cx.run_until_parked();

    let rows = |list: &gpui::Entity<ReviewList>, cx: &mut gpui::VisualTestContext| {
      list.read_with(cx, |list, cx| {
        list.list.read(cx).delegate().items_count(0, cx)
      })
    };
    let selected = |list: &gpui::Entity<ReviewList>, cx: &mut gpui::VisualTestContext| {
      list.read_with(cx, |list, cx| list.list.read(cx).delegate().selected_index)
    };

    // Two files, two comments each: a header row and its comments.
    assert_eq!(rows(&list, cx), 6);

    list.update_in(cx, |list, window, cx| list.focus(window, cx));
    cx.run_until_parked();

    cx.simulate_keystrokes("down");
    let first = selected(&list, cx).expect("the arrow keys reach the rows");
    cx.simulate_keystrokes("down");
    let second = selected(&list, cx).expect("selection stays on the list");
    assert_ne!(first, second, "down walks from one row to the next");

    // Enter on a file row folds it, and its comments leave the list.
    cx.simulate_keystrokes("up");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
      rows(&list, cx),
      4,
      "the folded file keeps its header and drops its comments"
    );

    // Left and right do the same, as they do in the trees.
    cx.simulate_keystrokes("right");
    cx.run_until_parked();
    assert_eq!(rows(&list, cx), 6, "right unfolds the file");
    cx.simulate_keystrokes("left");
    cx.run_until_parked();
    assert_eq!(rows(&list, cx), 4, "left folds it again");
  }

  #[gpui::test]
  async fn walking_the_comments_keeps_the_keyboard_in_the_list(cx: &mut gpui::TestAppContext) {
    let (list, cx) = add_review_list_window(cx);
    list.update(cx, |list, cx| {
      list.set_comments(ReviewSection::Agent, batch(), cx)
    });
    cx.run_until_parked();
    list.update_in(cx, |list, window, cx| list.focus(window, cx));
    cx.run_until_parked();

    let opened = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = opened.clone();
    cx.update(|_, cx| {
      cx.subscribe(&list, move |_, event: &ReviewListEvent, _| {
        if let ReviewListEvent::OpenComment { intent, .. } = event {
          seen.borrow_mut().push(*intent);
        }
      })
      .detach();
    });

    // Down onto the first comment: the row shows, it is not chosen.
    cx.simulate_keystrokes("down");
    cx.simulate_keystrokes("down");
    cx.run_until_parked();
    assert_eq!(opened.borrow().as_slice(), &[OpenIntent::Browse]);

    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
      opened.borrow().as_slice(),
      &[OpenIntent::Browse, OpenIntent::Open],
      "Enter is what hands the editor the keyboard"
    );
  }

  #[gpui::test]
  async fn nothing_ticked_sends_the_whole_batch(cx: &mut gpui::TestAppContext) {
    let (list, cx) = add_review_list_window(cx);

    list.update(cx, |list, cx| {
      list.set_comments(ReviewSection::Agent, batch(), cx)
    });

    list.read_with(cx, |list, _| {
      // Four rows, but the addressed one has nothing left to send.
      assert_eq!(list.comments(ReviewSection::Agent).len(), 4);
      assert_eq!(list.sendable_count(), 3);
      assert_eq!(list.send_count(), 3);
      assert!(list.selected_ids().is_empty());
      assert!(!list.everything_is_selected());
    });
  }

  #[gpui::test]
  async fn ticking_comments_narrows_what_send_sends(cx: &mut gpui::TestAppContext) {
    let (list, cx) = add_review_list_window(cx);
    list.update(cx, |list, cx| {
      list.set_comments(ReviewSection::Agent, batch(), cx)
    });

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
    list.update(cx, |list, cx| {
      list.set_comments(ReviewSection::Agent, batch(), cx)
    });

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
    list.update(cx, |list, cx| {
      list.set_comments(ReviewSection::Agent, batch(), cx)
    });

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
  async fn the_master_checkbox_paints_and_takes_the_whole_batch(cx: &mut gpui::TestAppContext) {
    let (list, cx) = add_review_list_window(cx);
    list.update(cx, |list, cx| {
      list.set_comments(ReviewSection::Agent, batch(), cx)
    });
    cx.run_until_parked();

    let button = cx
      .debug_bounds(REVIEW_LIST_SELECT_ALL_DEBUG_SELECTOR)
      .expect("master checkbox bounds");
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    cx.run_until_parked();

    list.read_with(cx, |list, _| {
      assert_eq!(list.selected_ids(), &HashSet::from([1, 2, 3]));
    });

    // Ticked, the same checkbox gives everything back.
    let button = cx
      .debug_bounds(REVIEW_LIST_SELECT_ALL_DEBUG_SELECTOR)
      .expect("master checkbox bounds");
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    cx.run_until_parked();

    list.read_with(cx, |list, _| assert!(list.selected_ids().is_empty()));
  }

  #[gpui::test]
  async fn the_section_title_ticks_the_whole_batch_too(cx: &mut gpui::TestAppContext) {
    let (list, cx) = add_review_list_window(cx);
    list.update(cx, |list, cx| {
      list.set_comments(ReviewSection::Agent, batch(), cx)
    });
    cx.run_until_parked();

    let header = cx
      .debug_bounds(review_list_section_header_debug_selector(
        ReviewSection::Agent,
      ))
      .expect("section header bounds");
    // The centre of the header is its title, not its tick box.
    cx.simulate_click(header.center(), gpui::Modifiers::default());
    cx.run_until_parked();

    list.read_with(cx, |list, _| {
      assert_eq!(list.selected_ids(), &HashSet::from([1, 2, 3]));
    });

    cx.simulate_click(header.center(), gpui::Modifiers::default());
    cx.run_until_parked();

    list.read_with(cx, |list, _| assert!(list.selected_ids().is_empty()));
  }

  #[gpui::test]
  async fn the_pull_request_section_title_ticks_nothing(cx: &mut gpui::TestAppContext) {
    let (list, cx) = add_review_list_window(cx);
    list.update(cx, |list, cx| {
      list.set_comments(
        ReviewSection::PullRequest,
        vec![pull_request_row(9, "src/a.rs", 3)],
        cx,
      )
    });
    cx.run_until_parked();

    let header = cx
      .debug_bounds(review_list_section_header_debug_selector(
        ReviewSection::PullRequest,
      ))
      .expect("section header bounds");
    cx.simulate_click(header.center(), gpui::Modifiers::default());
    cx.run_until_parked();

    // GitHub submits a review whole: there is nothing to tick here.
    list.read_with(cx, |list, _| assert!(list.selected_ids().is_empty()));
  }

  #[gpui::test]
  async fn a_comment_leaving_the_batch_leaves_the_selection(cx: &mut gpui::TestAppContext) {
    let (list, cx) = add_review_list_window(cx);
    list.update(cx, |list, cx| {
      list.set_comments(ReviewSection::Agent, batch(), cx)
    });
    list.update(cx, |list, cx| list.toggle_select_all(cx));

    // One went out with a turn, another was deleted from the batch.
    list.update(cx, |list, cx| {
      list.set_comments(
        ReviewSection::Agent,
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
            LocalAgentReviewCommentState::Sent,
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
  fn a_sent_comment_keeps_a_row_but_cannot_be_sent_again() {
    let rows = review_panel_comments(&[comment(
      1,
      "src/a.rs",
      2,
      "gone to the agent",
      LocalAgentReviewCommentState::Sent,
    )]);

    // Visible while the agent works on it; the turn that ends takes it away.
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].sendable);
  }
  #[gpui::test]
  async fn each_destination_keeps_its_own_rows(cx: &mut gpui::TestAppContext) {
    let (list, cx) = add_review_list_window(cx);

    list.update(cx, |list, cx| {
      list.set_comments(ReviewSection::Agent, batch(), cx);
      list.set_comments(
        ReviewSection::PullRequest,
        vec![pull_request_row(9, "src/a.rs", 3)],
        cx,
      );
    });

    // Reloading one destination leaves the other alone: they have their own
    // lifetime, the agent's ends with a turn.
    list.update(cx, |list, cx| {
      list.set_comments(ReviewSection::Agent, Vec::new(), cx)
    });

    list.read_with(cx, |list, _| {
      assert!(list.comments(ReviewSection::Agent).is_empty());
      assert_eq!(list.comments(ReviewSection::PullRequest).len(), 1);
    });
  }

  #[gpui::test]
  async fn a_pending_pull_request_comment_takes_no_part_in_a_partial_send(
    cx: &mut gpui::TestAppContext,
  ) {
    let (list, cx) = add_review_list_window(cx);

    list.update(cx, |list, cx| {
      list.set_comments(
        ReviewSection::PullRequest,
        vec![pull_request_row(9, "src/a.rs", 3)],
        cx,
      );
      // GitHub submits a review whole, so ticking one of its comments would
      // promise something the API cannot do.
      list.toggle_comment(9, cx);
      list.toggle_file_selection(PathBuf::from("src/a.rs"), cx);
    });

    list.read_with(cx, |list, _| {
      assert_eq!(list.sendable_count(), 0);
      assert_eq!(list.send_count(), 0);
      assert!(!list.everything_is_selected());
    });
  }

  #[gpui::test]
  async fn the_same_file_collapses_once_per_destination(cx: &mut gpui::TestAppContext) {
    let (list, cx) = add_review_list_window(cx);

    list.update(cx, |list, cx| {
      list.set_comments(ReviewSection::Agent, batch(), cx);
      list.set_comments(
        ReviewSection::PullRequest,
        vec![pull_request_row(9, "src/a.rs", 3)],
        cx,
      );
      list.toggle_file(ReviewSection::Agent, PathBuf::from("src/a.rs"), cx);
    });

    list.read_with(cx, |list, _| {
      assert!(
        list
          .collapsed_files
          .contains(&(ReviewSection::Agent, PathBuf::from("src/a.rs")))
      );
      assert!(
        !list
          .collapsed_files
          .contains(&(ReviewSection::PullRequest, PathBuf::from("src/a.rs")))
      );
    });
  }

  #[gpui::test]
  async fn one_footer_at_a_time_and_it_follows_the_rows(cx: &mut gpui::TestAppContext) {
    let (list, cx) = add_review_list_window(cx);
    list.update(cx, |list, cx| {
      list.set_comments(ReviewSection::Agent, batch(), cx);
      list.set_comments(
        ReviewSection::PullRequest,
        vec![pull_request_row(9, "src/a.rs", 3)],
        cx,
      );
    });
    cx.run_until_parked();

    // Nothing walked yet: the first section of the list owns the footer.
    assert!(cx.debug_bounds(REVIEW_LIST_SEND_DEBUG_SELECTOR).is_some());
    assert!(
      cx.debug_bounds(REVIEW_LIST_DISCARD_DEBUG_SELECTOR)
        .is_some()
    );
    assert!(
      cx.debug_bounds(REVIEW_LIST_SUBMIT_DEBUG_SELECTOR).is_none(),
      "two footers is one too many"
    );

    let row = cx
      .debug_bounds("review-comment-pull-request-9")
      .expect("pull request row bounds");
    cx.simulate_click(row.center(), gpui::Modifiers::default());
    cx.run_until_parked();

    assert!(cx.debug_bounds(REVIEW_LIST_SUBMIT_DEBUG_SELECTOR).is_some());
    assert!(cx.debug_bounds(REVIEW_LIST_SEND_DEBUG_SELECTOR).is_none());
    assert!(
      cx.debug_bounds(REVIEW_LIST_DISCARD_DEBUG_SELECTOR)
        .is_none()
    );
  }

  #[gpui::test]
  async fn the_footer_names_its_destination_only_when_there_are_two(cx: &mut gpui::TestAppContext) {
    let (list, cx) = add_review_list_window(cx);
    list.update(cx, |list, cx| {
      list.set_comments(ReviewSection::Agent, batch(), cx)
    });
    cx.run_until_parked();

    // One destination: its title is already pinned above the rows.
    assert!(
      cx.debug_bounds(REVIEW_LIST_FOOTER_DESTINATION_DEBUG_SELECTOR)
        .is_none()
    );

    list.update(cx, |list, cx| {
      list.set_comments(
        ReviewSection::PullRequest,
        vec![pull_request_row(9, "src/a.rs", 3)],
        cx,
      )
    });
    cx.run_until_parked();

    assert!(
      cx.debug_bounds(REVIEW_LIST_FOOTER_DESTINATION_DEBUG_SELECTOR)
        .is_some(),
      "with two destinations the footer has to say which one it acts on"
    );
  }

  #[gpui::test]
  async fn a_section_that_empties_hands_the_footer_back(cx: &mut gpui::TestAppContext) {
    let (list, cx) = add_review_list_window(cx);
    list.update(cx, |list, cx| {
      list.set_comments(ReviewSection::Agent, batch(), cx);
      list.set_comments(
        ReviewSection::PullRequest,
        vec![pull_request_row(9, "src/a.rs", 3)],
        cx,
      );
    });
    cx.run_until_parked();

    let row = cx
      .debug_bounds("review-comment-pull-request-9")
      .expect("pull request row bounds");
    cx.simulate_click(row.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds(REVIEW_LIST_SUBMIT_DEBUG_SELECTOR).is_some());

    // The review went out: the footer cannot keep acting on a section that has
    // no comments left.
    list.update(cx, |list, cx| {
      list.set_comments(ReviewSection::PullRequest, Vec::new(), cx)
    });
    cx.run_until_parked();

    assert!(cx.debug_bounds(REVIEW_LIST_SEND_DEBUG_SELECTOR).is_some());
    assert!(cx.debug_bounds(REVIEW_LIST_SUBMIT_DEBUG_SELECTOR).is_none());
  }
}
