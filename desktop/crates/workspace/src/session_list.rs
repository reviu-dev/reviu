//! The projects sidebar: browse project checkouts and their conversations.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use agent_chat_panel::{ConversationMeta, WorktreeBinding};
use gpui::{
  Anchor, Bounds, Context, DismissEvent, DragMoveEvent, Entity, EventEmitter, Focusable as _,
  IntoElement, MouseExitEvent, Pixels, Point, Render, SharedString, Window, div, prelude::*, px,
};
use gpui_component::menu::{PopupMenu, PopupMenuItem};
use gpui_component::popover::Popover;
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::{ActiveTheme as _, ElementExt as _, Icon, Sizable as _, h_flex, v_flex};
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
    "New chat".into()
  } else {
    trimmed.to_string().into()
  }
}

/// One sidebar row: the conversation and the project it belongs to. Rows arrive
/// grouped by project (stable project order) and render under section headers.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionRow {
  pub meta: ConversationMeta,
  pub project_root: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum CheckoutKind {
  Main,
  Worktree { branch: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CheckoutRow {
  kind: CheckoutKind,
  path: PathBuf,
  title: SharedString,
  subtitle: SharedString,
}

#[derive(Clone)]
struct DraggedProjectSection {
  project_root: PathBuf,
  name: SharedString,
  cursor_offset: Point<Pixels>,
}

impl Render for DraggedProjectSection {
  fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    div()
      .ml(self.cursor_offset.x - px(90.0))
      .mt(self.cursor_offset.y - px(14.0))
      .child(
        div()
          .w(px(180.0))
          .px_2()
          .py_1()
          .text_sm()
          .truncate()
          .bg(cx.theme().accent)
          .text_color(cx.theme().accent_foreground)
          .rounded(cx.theme().radius)
          .shadow_md()
          .child(self.name.clone()),
      )
  }
}

pub enum SessionListEvent {
  /// The section header itself: fold or unfold a project's chats.
  ToggleProjectCollapsed {
    project_root: PathBuf,
  },
  /// The section header's create menu: a session whose agent works in its own
  /// git worktree of THAT project, started from `base`; `None` is the repository's
  /// default branch.
  NewWorktreeSessionIn {
    project_root: PathBuf,
    base: Option<String>,
  },
  SelectedCheckout {
    project_root: PathBuf,
    checkout_root: PathBuf,
  },
  RevealProject {
    project_root: PathBuf,
  },
  CopyProjectPath {
    project_root: PathBuf,
  },
  RemoveProject {
    project_root: PathBuf,
  },
  ProjectOrderChanged {
    project_order: Vec<PathBuf>,
  },
  Selected {
    id: String,
  },
  Deleted {
    id: String,
  },
}

#[derive(Default)]
struct ProjectMenuState {
  menu: Option<Entity<PopupMenu>>,
}

pub struct SessionList {
  conversations: Vec<SessionRow>,
  current_id: String,
  /// Row still hydrating after a click; shows a spinner in its trailing slot.
  loading_id: Option<String>,
  /// Live agent state by conversation id; absent rows are Idle.
  statuses: HashMap<String, SessionStatus>,
  /// Worktree checkout by conversation id, shown under checkout and chat rows.
  worktree_checkouts: HashMap<String, WorktreeBinding>,
  displayed_checkout: Option<PathBuf>,
  /// Folded project sections; folding IS the filter now.
  collapsed_projects: HashSet<PathBuf>,
  /// Every tracked project, in stable order: sections render from this, so an
  /// emptied project keeps its header.
  project_order: Vec<PathBuf>,
  /// Projects that have Git. Only these can create worktree checkouts.
  git_repositories: HashSet<PathBuf>,
  /// Keeps the section highlighted while one of its menus is open.
  open_menu_project: Option<PathBuf>,
  project_header_bounds: HashMap<PathBuf, Bounds<Pixels>>,
  drop_gap: Option<usize>,
}

impl SessionList {
  pub fn new() -> Self {
    Self {
      conversations: Vec::new(),
      current_id: String::new(),
      loading_id: None,
      statuses: HashMap::new(),
      worktree_checkouts: HashMap::new(),
      displayed_checkout: None,
      collapsed_projects: HashSet::new(),
      project_order: Vec::new(),
      git_repositories: HashSet::new(),
      open_menu_project: None,
      project_header_bounds: HashMap::new(),
      drop_gap: None,
    }
  }

  pub fn set_project_order(&mut self, project_order: Vec<PathBuf>, cx: &mut Context<Self>) {
    if self.project_order != project_order {
      self.project_order = project_order;
      cx.notify();
    }
  }

  pub fn set_git_repositories(
    &mut self,
    git_repositories: HashSet<PathBuf>,
    cx: &mut Context<Self>,
  ) {
    if self.git_repositories != git_repositories {
      self.git_repositories = git_repositories;
      cx.notify();
    }
  }

  #[cfg(test)]
  pub(crate) fn project_order_for_test(&self) -> &[PathBuf] {
    &self.project_order
  }

  pub fn toggle_project_collapsed(&mut self, repo_root: &Path, cx: &mut Context<Self>) {
    if !self.collapsed_projects.remove(repo_root) {
      self.collapsed_projects.insert(repo_root.to_path_buf());
    }
    cx.notify();
  }

  #[cfg(test)]
  pub(crate) fn is_project_collapsed(&self, repo_root: &Path) -> bool {
    self.collapsed_projects.contains(repo_root)
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

  pub fn set_worktree_checkouts(
    &mut self,
    worktree_checkouts: HashMap<String, WorktreeBinding>,
    cx: &mut Context<Self>,
  ) {
    if self.worktree_checkouts != worktree_checkouts {
      self.worktree_checkouts = worktree_checkouts;
      cx.notify();
    }
  }

  pub fn set_displayed_checkout(
    &mut self,
    displayed_checkout: Option<PathBuf>,
    cx: &mut Context<Self>,
  ) {
    if self.displayed_checkout != displayed_checkout {
      self.displayed_checkout = displayed_checkout;
      cx.notify();
    }
  }

  pub(crate) fn contains_conversation(&self, id: &str) -> bool {
    self.conversations.iter().any(|row| row.meta.id == id)
  }

  #[cfg(test)]
  pub(crate) fn status_of(&self, id: &str) -> SessionStatus {
    self.statuses.get(id).copied().unwrap_or_default()
  }

  #[cfg(test)]
  pub(crate) fn agent_id_of(&self, id: &str) -> Option<String> {
    self
      .conversations
      .iter()
      .find(|row| row.meta.id == id)
      .map(|row| row.meta.agent_id.to_string())
  }

  #[cfg(test)]
  pub(crate) fn worktree_branch_of(&self, id: &str) -> Option<&str> {
    self
      .worktree_checkouts
      .get(id)
      .map(|binding| binding.branch.as_str())
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
          // A fresh conversation heads its project's section (newest-created
          // first); an unknown project opens a section at the end until the
          // next full refresh settles the order.
          let at = self
            .conversations
            .iter()
            .position(|existing| existing.project_root == row.project_root)
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

  fn rendered_project_order(&self) -> Vec<PathBuf> {
    let mut section_repos = self.project_order.clone();
    for row in &self.conversations {
      if !section_repos.contains(&row.project_root) {
        section_repos.push(row.project_root.clone());
      }
    }
    section_repos
  }

  fn checkout_rows_for_project(&self, repo_root: &Path) -> Vec<CheckoutRow> {
    let mut rows = vec![CheckoutRow {
      kind: CheckoutKind::Main,
      path: repo_root.to_path_buf(),
      title: "Main checkout".into(),
      subtitle: "Default working tree".into(),
    }];
    if !self.git_repositories.contains(repo_root) {
      return rows;
    }
    let mut checkouts = Vec::new();
    for row in self
      .conversations
      .iter()
      .filter(|row| row.project_root == repo_root)
    {
      let Some(binding) = self.worktree_checkouts.get(&row.meta.id) else {
        continue;
      };
      if !checkouts
        .iter()
        .any(|existing: &WorktreeBinding| existing.path == binding.path)
      {
        checkouts.push(binding.clone());
      }
    }
    rows.extend(checkouts.into_iter().map(|binding| CheckoutRow {
      kind: CheckoutKind::Worktree {
        branch: binding.branch.clone(),
      },
      path: binding.path,
      title: binding.branch.into(),
      subtitle: "Worktree checkout".into(),
    }));
    rows
  }

  fn checkout_path_for_session(&self, row: &SessionRow) -> PathBuf {
    if !self.git_repositories.contains(&row.project_root) {
      return row.project_root.clone();
    }
    self
      .worktree_checkouts
      .get(&row.meta.id)
      .map(|binding| binding.path.clone())
      .unwrap_or_else(|| row.project_root.clone())
  }

  #[cfg(test)]
  fn conversation_ids_for_checkout(&self, repo_root: &Path, checkout_root: &Path) -> Vec<String> {
    self
      .conversations
      .iter()
      .filter(|row| {
        row.project_root == repo_root && self.checkout_path_for_session(row) == checkout_root
      })
      .map(|row| row.meta.id.clone())
      .collect()
  }

  fn update_project_header_bounds(&mut self, project_root: PathBuf, bounds: Bounds<Pixels>) {
    self.project_header_bounds.insert(project_root, bounds);
  }

  fn project_drop_gap_at(&self, y: Pixels, dragged_repo: &Path) -> Option<usize> {
    let section_repos = self.rendered_project_order();
    let from = section_repos.iter().position(|repo| repo == dragged_repo)?;
    let mut gap = 0;
    let mut found_bounds = false;

    for (row, repo) in section_repos.iter().enumerate() {
      let Some(bounds) = self.project_header_bounds.get(repo) else {
        continue;
      };
      found_bounds = true;
      if y < bounds.center().y {
        break;
      }
      gap = row + 1;
    }

    if !found_bounds || gap == from || gap == from + 1 {
      None
    } else {
      Some(gap)
    }
  }

  fn update_drop_gap(&mut self, gap: Option<usize>, cx: &mut Context<Self>) {
    if self.drop_gap != gap {
      self.drop_gap = gap;
      cx.notify();
    }
  }

  fn reorder_project_section(&mut self, dragged_repo: &Path, gap: usize, cx: &mut Context<Self>) {
    self.drop_gap = None;
    let mut project_order = self.rendered_project_order();
    let Some(from) = project_order.iter().position(|repo| repo == dragged_repo) else {
      cx.notify();
      return;
    };
    let mut gap = gap.min(project_order.len());
    if gap == from || gap == from + 1 {
      cx.notify();
      return;
    }

    let repo = project_order.remove(from);
    if from < gap {
      gap -= 1;
    }
    project_order.insert(gap, repo);
    self.project_order = project_order.clone();
    cx.emit(SessionListEvent::ProjectOrderChanged { project_order });
    cx.notify();
  }
}

impl SessionList {
  /// A project's section header: fold toggle, name, count when folded, and the
  /// hover actions that target that project.
  fn render_project_header(
    &self,
    repo_root: &Path,
    row: usize,
    is_last: bool,
    count: usize,
    theme: &gpui_component::Theme,
    cx: &mut Context<Self>,
  ) -> gpui::AnyElement {
    let collapsed = self.collapsed_projects.contains(repo_root);
    let name: SharedString = repo_root
      .file_name()
      .map(|name| name.to_string_lossy().into_owned())
      .unwrap_or_else(|| repo_root.to_string_lossy().into_owned())
      .into();
    let toggle_repo = repo_root.to_path_buf();
    let create_repo = repo_root.to_path_buf();
    let options_repo = repo_root.to_path_buf();
    let drag_repo = repo_root.to_path_buf();
    let bounds_repo = repo_root.to_path_buf();
    let group_name = SharedString::from(format!("repo-section-{}", repo_root.display()));
    let menu_open = self.open_menu_project.as_deref() == Some(repo_root);
    let drop_gap = self.drop_gap.filter(|_| cx.has_active_drag());
    let git_backed = self.git_repositories.contains(repo_root);

    h_flex()
      .id(SharedString::from(format!(
        "session-repo-section-{}",
        repo_root.display()
      )))
      .debug_selector(|| format!("session-repo-section-{}", repo_root.display()))
      .group(group_name.clone())
      .relative()
      .items_center()
      .gap_1()
      .mx_2()
      .mt_1()
      .px_2()
      .py_1()
      .rounded(px(6.0))
      .cursor_move()
      .when(menu_open, |this| this.bg(theme.secondary_hover))
      .hover(|this| this.bg(theme.secondary_hover))
      .when_some(drop_gap, |this, gap| {
        let draws_before = gap == row;
        let draws_after = is_last && gap == row + 1;
        if !draws_before && !draws_after {
          return this;
        }
        let line = div()
          .absolute()
          .left_0()
          .right_0()
          .h(px(2.0))
          .rounded_full()
          .bg(theme.status_blue());
        this.child(if draws_after {
          line.bottom(px(-3.0))
        } else {
          line.top(px(-3.0))
        })
      })
      .on_prepaint({
        let view = cx.entity().clone();
        move |bounds, _, cx| {
          view.update(cx, |list, _| {
            list.update_project_header_bounds(bounds_repo.clone(), bounds)
          })
        }
      })
      .on_drag(
        DraggedProjectSection {
          project_root: drag_repo,
          name: name.clone(),
          cursor_offset: Point::default(),
        },
        |drag, cursor_offset, _, cx| {
          let mut drag = drag.clone();
          drag.cursor_offset = cursor_offset;
          cx.new(|_| drag)
        },
      )
      .on_click(cx.listener(move |_, _, _, cx| {
        cx.emit(SessionListEvent::ToggleProjectCollapsed {
          project_root: toggle_repo.clone(),
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
          .when(!menu_open, |this| this.invisible())
          .group_hover(group_name, |this| this.visible())
          .when(git_backed, |this| {
            this.child(
              div()
                .id(SharedString::from(format!(
                  "session-repo-create-wrap-{}",
                  create_repo.display()
                )))
                .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
                .child(Self::render_create_button(create_repo, cx)),
            )
          })
          .child(
            div()
              .id(SharedString::from(format!(
                "session-repo-options-wrap-{}",
                options_repo.display()
              )))
              .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
              .child(Self::render_options_button(options_repo, cx)),
          ),
      )
      .into_any_element()
  }

  fn render_checkout_row(
    &self,
    repo_root: &Path,
    row: &CheckoutRow,
    active: bool,
    theme: &gpui_component::Theme,
    cx: &mut Context<Self>,
  ) -> gpui::AnyElement {
    let selector = match &row.kind {
      CheckoutKind::Main => format!("session-checkout-main-{}", repo_root.display()),
      CheckoutKind::Worktree { branch } => {
        format!("session-checkout-worktree-{}-{branch}", repo_root.display())
      }
    };
    let checkout_repo = repo_root.to_path_buf();
    let checkout_root = row.path.clone();
    let icon = match row.kind {
      CheckoutKind::Main => Icon::new(gpui_component::IconName::FolderOpen)
        .size(px(12.))
        .text_color(theme.muted_foreground)
        .into_any_element(),
      CheckoutKind::Worktree { .. } => Icon::new(UiIconName::GitBranch)
        .size(px(12.))
        .text_color(theme.muted_foreground)
        .into_any_element(),
    };

    div()
      .id(SharedString::from(selector.clone()))
      .debug_selector(move || selector.clone())
      .mx_2()
      .ml_4()
      .px_2()
      .py_1p5()
      .rounded(px(6.0))
      .cursor_pointer()
      .when(active, |this| this.bg(theme.secondary_active))
      .hover(|this| this.bg(theme.secondary_hover))
      .on_click(cx.listener(move |_, _, _, cx| {
        cx.emit(SessionListEvent::SelectedCheckout {
          project_root: checkout_repo.clone(),
          checkout_root: checkout_root.clone(),
        });
      }))
      .child(
        h_flex().items_center().gap_2().child(icon).child(
          v_flex()
            .min_w(px(0.0))
            .gap_0p5()
            .child(
              div()
                .text_xs()
                .truncate()
                .text_color(theme.foreground)
                .child(row.title.clone()),
            )
            .child(
              div()
                .text_xs()
                .truncate()
                .text_color(theme.muted_foreground.opacity(0.75))
                .child(row.subtitle.clone()),
            ),
        ),
      )
      .into_any_element()
  }

  fn render_chat_row(
    &self,
    ix: usize,
    row: &SessionRow,
    now: u64,
    theme: &gpui_component::Theme,
    cx: &mut Context<Self>,
  ) -> gpui::AnyElement {
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
    let id = meta.id.clone();
    let selector_id = meta.id.clone();
    let delete_id = meta.id.clone();
    let title = session_row_title(meta);
    let preview = meta.preview.clone();
    let time = format_relative_secs(meta.updated_at_secs, now);
    let agent_id = meta.agent_id.clone();
    let agent_icon_id = agent_id.clone();
    let group_name = SharedString::from(format!("session-row-{}", meta.id));

    div()
      .id(("session-page-session-row", ix))
      .debug_selector(move || format!("session-chat-row-{selector_id}"))
      .group(group_name.clone())
      .mx_2()
      .ml_8()
      .px_2()
      .py_1()
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
              .flex_shrink_0()
              .debug_selector(move || format!("session-agent-icon-{agent_icon_id}"))
              .child(
                agent_chat_panel::backend_icon(&agent_id)
                  .xsmall()
                  .text_color(theme.muted_foreground),
              ),
          )
          .child(
            div()
              .flex_1()
              .min_w(px(0.0))
              .text_xs()
              .truncate()
              .text_color(theme.foreground)
              .child(title),
          )
          .child(
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
                      .tooltip("Delete chat")
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
      .into_any_element()
  }

  fn render_project_menu_button(
    project_root: PathBuf,
    menu_id: SharedString,
    button: Button,
    cx: &mut Context<Self>,
    build_menu: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
  ) -> impl IntoElement {
    let repo_root = project_root;
    let entity = cx.entity().downgrade();
    let build_menu = Rc::new(build_menu);
    let popover_id = SharedString::from(format!(
      "session-repo-menu-{}-{}",
      menu_id,
      repo_root.display()
    ));
    let menu_state_id = popover_id.clone();
    let open_repo = repo_root.clone();

    Popover::new(popover_id)
      .appearance(false)
      .overlay_closable(false)
      .anchor(Anchor::TopLeft)
      .trigger(button)
      .on_open_change(move |open, _, cx| {
        let repo_root = open_repo.clone();
        let _ = entity.update(cx, |list, cx| {
          if *open {
            list.open_menu_project = Some(repo_root);
          } else if list.open_menu_project.as_deref() == Some(repo_root.as_path()) {
            list.open_menu_project = None;
          }
          cx.notify();
        });
      })
      .content(move |_, window, cx| {
        let menu_state = window.use_keyed_state(menu_state_id.clone(), cx, |_, _| {
          ProjectMenuState::default()
        });
        match menu_state.read(cx).menu.clone() {
          Some(menu) => menu,
          None => {
            let build_menu = build_menu.clone();
            let menu = PopupMenu::build(window, cx, move |menu, window, cx| {
              build_menu(menu, window, cx)
            });
            menu_state.update(cx, |state, _| state.menu = Some(menu.clone()));
            menu.focus_handle(cx).focus(window, cx);

            let popover_state = cx.entity();
            window
              .subscribe(&menu, cx, {
                let menu_state = menu_state.clone();
                move |_, _: &DismissEvent, window, cx| {
                  popover_state.update(cx, |state, cx| state.dismiss(window, cx));
                  menu_state.update(cx, |state, _| state.menu = None);
                }
              })
              .detach();

            menu
          }
        }
      })
  }

  /// The create menu of one Git project section: the worktree base picker reads
  /// branches from that repository at menu-open time.
  fn render_create_button(project_root: PathBuf, cx: &mut Context<Self>) -> impl IntoElement {
    let repo_root = project_root;
    let entity = cx.entity().downgrade();
    let button = Button::new(SharedString::from(format!(
      "session-repo-create-{}",
      repo_root.display()
    )))
    .debug_selector(|| format!("session-repo-create-{}", repo_root.display()))
    .icon(gpui_component::IconName::Plus)
    .ghost()
    .compact()
    .xsmall()
    .tooltip("New worktree");

    Self::render_project_menu_button(
      repo_root.clone(),
      "create".into(),
      button,
      cx,
      move |menu, window, cx| {
        let submenu_repo = repo_root.clone();
        let submenu_entity = entity.clone();
        menu.submenu_with_icon(
          Some(Icon::new(UiIconName::GitBranch)),
          "New worktree",
          window,
          cx,
          move |menu, _, _| {
            let mut menu = menu.max_h(px(360.)).scrollable(true);
            let base_candidates: Vec<SharedString> = git::list_branches(&submenu_repo)
              .ok()
              .unwrap_or_default()
              .into_iter()
              .filter(|branch| branch.kind == git::BranchKind::Local)
              .map(|branch| SharedString::from(branch.name))
              .collect();
            let default_entity = submenu_entity.clone();
            let default_repo = submenu_repo.clone();
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
                    project_root: repo_root,
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
              let entity = submenu_entity.clone();
              let item_repo = submenu_repo.clone();
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
                      project_root: repo_root,
                      base: Some(base),
                    });
                  });
                }),
              );
            }
            menu
          },
        )
      },
    )
  }

  fn render_options_button(project_root: PathBuf, cx: &mut Context<Self>) -> impl IntoElement {
    let repo_root = project_root;
    let entity = cx.entity().downgrade();
    let button = Button::new(SharedString::from(format!(
      "session-repo-options-{}",
      repo_root.display()
    )))
    .debug_selector(|| format!("session-repo-options-{}", repo_root.display()))
    .icon(UiIconName::EllipsisVertical)
    .ghost()
    .compact()
    .xsmall()
    .tooltip("Project options");

    Self::render_project_menu_button(
      repo_root.clone(),
      "options".into(),
      button,
      cx,
      move |menu, _, _| {
        let reveal_repo = repo_root.clone();
        let reveal_entity = entity.clone();
        let copy_repo = repo_root.clone();
        let copy_entity = entity.clone();
        let remove_repo = repo_root.clone();
        let remove_entity = entity.clone();
        let reveal_label = if cfg!(target_os = "macos") {
          "Reveal in Finder"
        } else if cfg!(target_os = "windows") {
          "Reveal in File Explorer"
        } else {
          "Reveal in file manager"
        };
        menu
          .item(
            PopupMenuItem::new(reveal_label)
              .icon(gpui_component::IconName::FolderOpen)
              .on_click(move |_, _, cx| {
                let repo_root = reveal_repo.clone();
                let _ = reveal_entity.update(cx, |_, cx| {
                  cx.emit(SessionListEvent::RevealProject {
                    project_root: repo_root,
                  });
                });
              }),
          )
          .item(
            PopupMenuItem::new("Copy path")
              .icon(gpui_component::IconName::Copy)
              .on_click(move |_, _, cx| {
                let repo_root = copy_repo.clone();
                let _ = copy_entity.update(cx, |_, cx| {
                  cx.emit(SessionListEvent::CopyProjectPath {
                    project_root: repo_root,
                  });
                });
              }),
          )
          .separator()
          .item(
            PopupMenuItem::new("Remove from sidebar")
              .icon(UiIconName::Trash)
              .on_click(move |_, _, cx| {
                let repo_root = remove_repo.clone();
                let _ = remove_entity.update(cx, |_, cx| {
                  cx.emit(SessionListEvent::RemoveProject {
                    project_root: repo_root,
                  });
                });
              }),
          )
      },
    )
  }
}

impl EventEmitter<SessionListEvent> for SessionList {}

impl Render for SessionList {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let now = now_secs();

    let header = h_flex()
      .debug_selector(|| "session-sidebar-header".to_string())
      .h(px(40.))
      .min_h(px(40.))
      .max_h(px(40.))
      .flex_shrink_0()
      .items_center()
      .px_3()
      .border_b_1()
      .border_color(theme.border)
      .child(
        div()
          .debug_selector(|| "session-sidebar-projects-header".to_string())
          .text_xs()
          .font_weight(gpui::FontWeight::SEMIBOLD)
          .text_color(theme.muted_foreground)
          .child("Projects"),
      );

    // Sections come from the tracked-project order so an empty project keeps its
    // header; rows not yet in that order (fresh upsert) get a section at the
    // end until the next refresh settles it.
    let section_repos = self.rendered_project_order();
    self
      .project_header_bounds
      .retain(|repo, _| section_repos.contains(repo));
    let mut items: Vec<gpui::AnyElement> = Vec::new();
    for (section_ix, section_repo) in section_repos.iter().enumerate() {
      let count = self
        .conversations
        .iter()
        .filter(|row| &row.project_root == section_repo)
        .count();
      items.push(self.render_project_header(
        section_repo,
        section_ix,
        section_ix + 1 == section_repos.len(),
        count,
        &theme,
        cx,
      ));
      if self.collapsed_projects.contains(section_repo) {
        continue;
      }
      for checkout in self.checkout_rows_for_project(section_repo) {
        let active = self.displayed_checkout.as_deref() == Some(checkout.path.as_path());
        items.push(self.render_checkout_row(section_repo, &checkout, active, &theme, cx));
        for (ix, row) in self.conversations.iter().enumerate().filter(|(_, row)| {
          row.project_root == *section_repo && self.checkout_path_for_session(row) == checkout.path
        }) {
          items.push(self.render_chat_row(ix, row, now, &theme, cx));
        }
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
            .child("No projects yet"),
        )
        .child(
          div()
            .text_xs()
            .text_center()
            .text_color(theme.muted_foreground.opacity(0.8))
            .child("Open a project to start working"),
        )
        .into_any_element()
    } else {
      div()
        .id("session-page-session-list")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .py_1()
        .on_drag_move(cx.listener(
          |this, event: &DragMoveEvent<DraggedProjectSection>, _, cx| {
            let drag = event.drag(cx);
            let gap = if event.bounds.contains(&event.event.position) {
              this.project_drop_gap_at(event.event.position.y, &drag.project_root)
            } else {
              None
            };
            this.update_drop_gap(gap, cx);
          },
        ))
        .on_drop(cx.listener(|this, drag: &DraggedProjectSection, _, cx| {
          let Some(gap) = this.drop_gap.take() else {
            cx.notify();
            return;
          };
          this.reorder_project_section(&drag.project_root, gap, cx);
        }))
        .on_mouse_exit(cx.listener(|this, _: &MouseExitEvent, _, cx| {
          this.update_drop_gap(None, cx);
        }))
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
      agent_id: agent_chat_panel::default_agent_id(),
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
    assert_eq!(session_row_title(&meta_with_title("")), "New chat");
    assert_eq!(session_row_title(&meta_with_title("   ")), "New chat");
    assert_eq!(
      session_row_title(&meta_with_title("Fix scroll")),
      "Fix scroll"
    );
  }

  fn worktree_binding(path: &str, branch: &str) -> WorktreeBinding {
    WorktreeBinding {
      path: PathBuf::from(path),
      branch: branch.to_string(),
    }
  }

  fn meta(id: &str, updated: u64) -> SessionRow {
    SessionRow {
      meta: ConversationMeta {
        id: id.to_string(),
        started_at_secs: 0,
        updated_at_secs: updated,
        title: id.to_string(),
        message_count: 1,
        agent_id: agent_chat_panel::default_agent_id(),
        session_id: None,
        preview: String::new(),
      },
      project_root: PathBuf::from("/repo"),
    }
  }

  #[gpui::test]
  async fn project_checkout_rows_render_above_existing_chats(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let list = cx.new(|_| SessionList::new());
    let mounted = list.clone();
    let (_root, cx) =
      cx.add_window_view(move |window, cx| gpui_component::Root::new(mounted.clone(), window, cx));
    let repo = PathBuf::from("/repo");
    let worktree_path = PathBuf::from("/repo/.worktrees/feature-sidebar");
    let mut worktree_checkouts = HashMap::new();
    worktree_checkouts.insert(
      "worktree-chat".to_string(),
      WorktreeBinding {
        path: worktree_path.clone(),
        branch: "feature/sidebar".to_string(),
      },
    );

    list.update(cx, |list, cx| {
      let mut main = meta("main-chat", 2);
      main.project_root = repo.clone();
      let mut worktree = meta("worktree-chat", 1);
      worktree.project_root = repo.clone();
      list.set_project_order(vec![repo.clone()], cx);
      list.set_git_repositories(HashSet::from([repo.clone()]), cx);
      list.set_conversations(vec![main, worktree], "worktree-chat".into(), cx);
      list.set_worktree_checkouts(worktree_checkouts, cx);
      list.set_displayed_checkout(Some(worktree_path.clone()), cx);
    });
    cx.run_until_parked();

    assert!(cx.debug_bounds("session-sidebar-projects-header").is_some());
    assert!(cx.debug_bounds("session-checkout-main-/repo").is_some());
    assert!(
      cx.debug_bounds("session-checkout-worktree-/repo-feature/sidebar")
        .is_some()
    );
    assert!(cx.debug_bounds("session-chat-row-main-chat").is_some());
    assert!(cx.debug_bounds("session-chat-row-worktree-chat").is_some());

    let selected = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let selected_checkouts = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = selected.clone();
    let seen_checkouts = selected_checkouts.clone();
    cx.update(|_, cx| {
      cx.subscribe(&list, move |_, event: &SessionListEvent, _| match event {
        SessionListEvent::Selected { id } => seen.borrow_mut().push(id.clone()),
        SessionListEvent::SelectedCheckout { checkout_root, .. } => {
          seen_checkouts.borrow_mut().push(checkout_root.clone())
        }
        _ => {}
      })
      .detach();
    });

    let worktree_checkout = cx
      .debug_bounds("session-checkout-worktree-/repo-feature/sidebar")
      .expect("worktree checkout row");
    cx.simulate_click(worktree_checkout.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert_eq!(selected_checkouts.borrow().as_slice(), &[worktree_path]);

    let main_chat = cx
      .debug_bounds("session-chat-row-main-chat")
      .expect("main chat row");
    cx.simulate_click(main_chat.center(), gpui::Modifiers::default());
    cx.run_until_parked();

    assert_eq!(selected.borrow().as_slice(), &["main-chat".to_string()]);
  }

  #[gpui::test]
  async fn plain_project_sections_keep_the_main_checkout_without_worktree_actions(
    cx: &mut gpui::TestAppContext,
  ) {
    cx.update(gpui_component::init);
    let list = cx.new(|_| SessionList::new());
    let mounted = list.clone();
    let (_root, cx) =
      cx.add_window_view(move |window, cx| gpui_component::Root::new(mounted.clone(), window, cx));
    let project = PathBuf::from("/plain-project");

    list.update(cx, |list, cx| {
      let mut row = meta("plain-chat", 1);
      row.project_root = project.clone();
      list.set_project_order(vec![project.clone()], cx);
      list.set_conversations(vec![row], "plain-chat".into(), cx);
      list.worktree_checkouts.insert(
        "plain-chat".to_string(),
        worktree_binding("/plain-project/.worktrees/ignored", "ignored"),
      );
      list.set_displayed_checkout(Some(project.clone()), cx);
    });
    cx.run_until_parked();

    assert!(
      cx.debug_bounds("session-repo-section-/plain-project")
        .is_some()
    );
    assert!(
      cx.debug_bounds("session-checkout-main-/plain-project")
        .is_some()
    );
    assert!(cx.debug_bounds("session-chat-row-plain-chat").is_some());
    assert!(
      cx.debug_bounds("session-checkout-worktree-/plain-project-ignored")
        .is_none()
    );

    let header = cx
      .debug_bounds("session-repo-section-/plain-project")
      .expect("plain project section header");
    cx.simulate_mouse_move(header.center(), None, gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(
      cx.debug_bounds("session-repo-create-/plain-project")
        .is_none(),
      "plain projects cannot create worktrees"
    );
    assert!(
      cx.debug_bounds("session-repo-options-/plain-project")
        .is_some(),
      "plain projects still have project options"
    );
  }

  #[test]
  fn plain_project_checkout_rows_ignore_worktree_bindings() {
    let mut list = SessionList::new();
    list.conversations = vec![meta("plain-chat", 1)];
    list.worktree_checkouts.insert(
      "plain-chat".to_string(),
      worktree_binding("/repo/.worktrees/ignored", "ignored"),
    );
    assert_eq!(
      list.checkout_rows_for_project(Path::new("/repo")),
      vec![CheckoutRow {
        kind: CheckoutKind::Main,
        path: PathBuf::from("/repo"),
        title: "Main checkout".into(),
        subtitle: "Default working tree".into(),
      }]
    );
  }

  #[test]
  fn checkout_rows_follow_known_worktree_branches() {
    let mut list = SessionList::new();
    list.git_repositories.insert(PathBuf::from("/repo"));
    list.conversations = vec![meta("main-chat", 2), meta("worktree-chat", 1)];
    list.worktree_checkouts.insert(
      "worktree-chat".to_string(),
      worktree_binding("/repo/.worktrees/feature-sidebar", "feature/sidebar"),
    );
    assert_eq!(
      list.checkout_rows_for_project(Path::new("/repo")),
      vec![
        CheckoutRow {
          kind: CheckoutKind::Main,
          path: PathBuf::from("/repo"),
          title: "Main checkout".into(),
          subtitle: "Default working tree".into(),
        },
        CheckoutRow {
          kind: CheckoutKind::Worktree {
            branch: "feature/sidebar".to_string(),
          },
          path: PathBuf::from("/repo/.worktrees/feature-sidebar"),
          title: "feature/sidebar".into(),
          subtitle: "Worktree checkout".into(),
        },
      ]
    );
    assert_eq!(
      list.conversation_ids_for_checkout(Path::new("/repo"), Path::new("/repo")),
      vec!["main-chat".to_string()]
    );
    assert_eq!(
      list.conversation_ids_for_checkout(
        Path::new("/repo"),
        Path::new("/repo/.worktrees/feature-sidebar")
      ),
      vec!["worktree-chat".to_string()]
    );
  }

  #[gpui::test]
  async fn session_rows_show_their_agent_icon(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let list = cx.new(|_| SessionList::new());
    let mounted = list.clone();
    let (_root, cx) =
      cx.add_window_view(move |window, cx| gpui_component::Root::new(mounted.clone(), window, cx));

    list.update(cx, |list, cx| {
      let mut row = meta("pi-session", 1);
      row.meta.agent_id = agent_registry::AgentId::new("pi-acp");
      list.set_conversations(vec![row], "pi-session".into(), cx);
    });
    cx.run_until_parked();

    assert!(cx.debug_bounds("session-agent-icon-pi-acp").is_some());
  }

  #[gpui::test]
  async fn project_section_reveals_create_and_options_actions_on_hover(
    cx: &mut gpui::TestAppContext,
  ) {
    cx.update(gpui_component::init);
    let list = cx.new(|_| SessionList::new());
    let mounted = list.clone();
    let (_root, cx) =
      cx.add_window_view(move |window, cx| gpui_component::Root::new(mounted.clone(), window, cx));
    let repo = PathBuf::from("/repo");

    list.update(cx, |list, cx| {
      list.set_project_order(vec![repo.clone()], cx);
      list.set_git_repositories(HashSet::from([repo.clone()]), cx);
    });
    cx.run_until_parked();

    let header = cx
      .debug_bounds("session-repo-section-/repo")
      .expect("project section header");
    cx.simulate_mouse_move(header.center(), None, gpui::Modifiers::default());
    cx.run_until_parked();

    assert!(cx.debug_bounds("session-repo-create-/repo").is_some());
    let options = cx
      .debug_bounds("session-repo-options-/repo")
      .expect("repo options button");

    cx.simulate_click(options.center(), gpui::Modifiers::default());
    cx.run_until_parked();

    list.read_with(cx, |list, _| {
      assert_eq!(list.open_menu_project.as_deref(), Some(repo.as_path()));
    });
  }

  #[gpui::test]
  async fn dropping_a_project_section_reorders_projects(cx: &mut gpui::TestAppContext) {
    let list = cx.new(|_| SessionList::new());
    let repo_a = PathBuf::from("/repo-a");
    let repo_b = PathBuf::from("/repo-b");
    let repo_c = PathBuf::from("/repo-c");

    list.update(cx, |list, cx| {
      list.set_project_order(vec![repo_a.clone(), repo_b.clone(), repo_c.clone()], cx);
      list.reorder_project_section(&repo_c, 0, cx);
      assert_eq!(
        list.project_order,
        vec![repo_c.clone(), repo_a.clone(), repo_b.clone()]
      );

      list.reorder_project_section(&repo_c, 3, cx);
      assert_eq!(list.project_order, vec![repo_a, repo_b, repo_c]);
    });
  }

  #[gpui::test]
  async fn dragging_a_project_section_updates_the_order(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let list = cx.new(|_| SessionList::new());
    let mounted = list.clone();
    let (_root, cx) =
      cx.add_window_view(move |window, cx| gpui_component::Root::new(mounted.clone(), window, cx));
    let repo_a = PathBuf::from("/repo-a");
    let repo_b = PathBuf::from("/repo-b");
    let repo_c = PathBuf::from("/repo-c");

    list.update(cx, |list, cx| {
      list.set_project_order(vec![repo_a.clone(), repo_b.clone(), repo_c.clone()], cx)
    });
    cx.run_until_parked();

    let from = cx
      .debug_bounds("session-repo-section-/repo-c")
      .expect("source project section")
      .center();
    let to = cx
      .debug_bounds("session-repo-section-/repo-a")
      .expect("target project section")
      .center();

    cx.simulate_mouse_down(from, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_move(
      from + Point::new(px(0.0), px(-10.0)),
      Some(gpui::MouseButton::Left),
      gpui::Modifiers::default(),
    );
    cx.simulate_mouse_move(
      to + Point::new(px(0.0), px(-8.0)),
      Some(gpui::MouseButton::Left),
      gpui::Modifiers::default(),
    );
    cx.simulate_mouse_up(
      to + Point::new(px(0.0), px(-8.0)),
      gpui::MouseButton::Left,
      gpui::Modifiers::default(),
    );
    cx.run_until_parked();

    list.read_with(cx, |list, _| {
      assert_eq!(
        list.project_order,
        vec![repo_c.clone(), repo_a.clone(), repo_b.clone()]
      );
    });
  }

  #[test]
  fn project_drop_gap_uses_header_centers_and_skips_noops() {
    let repo_a = PathBuf::from("/repo-a");
    let repo_b = PathBuf::from("/repo-b");
    let repo_c = PathBuf::from("/repo-c");
    let mut list = SessionList::new();
    list.project_order = vec![repo_a.clone(), repo_b.clone(), repo_c.clone()];
    for (ix, repo) in list.project_order.clone().into_iter().enumerate() {
      list.update_project_header_bounds(
        repo,
        Bounds::new(
          Point::new(px(0.0), px((ix * 20) as f32)),
          gpui::Size::new(px(100.0), px(20.0)),
        ),
      );
    }

    assert_eq!(list.project_drop_gap_at(px(1.0), &repo_c), Some(0));
    assert_eq!(list.project_drop_gap_at(px(31.0), &repo_a), Some(2));
    assert_eq!(list.project_drop_gap_at(px(31.0), &repo_b), None);
    assert_eq!(list.project_drop_gap_at(px(45.0), &repo_c), None);
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
      list.set_worktree_checkouts(HashMap::new(), cx)
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

      // A row not yet on disk heads its project's section.
      list.upsert_current(Some(meta("c", 40)), "c".into(), cx);
      assert_eq!(
        list.conversation_ids(),
        vec!["c".to_string(), "b".to_string(), "a".to_string()]
      );

      // An empty draft only moves the selection.
      list.upsert_current(None, "b".into(), cx);
      assert_eq!(list.current_id, "b");
      assert_eq!(list.conversations.len(), 3);

      // Folding a project is list state, not data: rows stay.
      list.toggle_project_collapsed(Path::new("/repo"), cx);
      assert!(list.is_project_collapsed(Path::new("/repo")));
      assert_eq!(list.conversations.len(), 3);
      list.toggle_project_collapsed(Path::new("/repo"), cx);
      assert!(!list.is_project_collapsed(Path::new("/repo")));
    });
  }
}
