use std::sync::Arc;

use git::{
  InteractiveRebaseAction, InteractiveRebaseCommit, InteractiveRebaseTarget,
  InteractiveRebaseTodoEntry,
};
use gpui::{
  App, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement, KeyDownEvent,
  ParentElement, Render, RenderOnce, SharedString, Styled, Subscription, WeakEntity, Window, div,
  prelude::*, px, white,
};
use gpui_component::{
  ActiveTheme as _, Disableable, IconName, IndexPath, Selectable, Sizable,
  button::{Button, ButtonVariant, ButtonVariants as _},
  h_flex,
  list::{List, ListDelegate, ListEvent, ListState},
  select::{Select, SelectEvent, SelectItem, SelectState},
  v_flex,
};
use ui::StatusThemeExt;

pub type InteractiveRebaseTodoViewHandler = Arc<
  dyn Fn(
      InteractiveRebaseTarget,
      Vec<InteractiveRebaseTodoEntry>,
      &mut Window,
      &mut App,
    ) -> Result<(), SharedString>
    + Send
    + Sync,
>;

pub type InteractiveRebaseTodoViewCancelHandler = Arc<dyn Fn(&mut Window, &mut App) + Send + Sync>;

pub struct InteractiveRebaseTodoViewConfig {
  pub target: InteractiveRebaseTarget,
  pub commits: Vec<InteractiveRebaseCommit>,
  pub on_submit: InteractiveRebaseTodoViewHandler,
  pub on_cancel: InteractiveRebaseTodoViewCancelHandler,
}

impl InteractiveRebaseTodoViewConfig {
  pub fn new(
    target: InteractiveRebaseTarget,
    commits: Vec<InteractiveRebaseCommit>,
    on_submit: InteractiveRebaseTodoViewHandler,
    on_cancel: InteractiveRebaseTodoViewCancelHandler,
  ) -> Self {
    Self {
      target,
      commits,
      on_submit,
      on_cancel,
    }
  }
}

#[derive(Clone)]
struct InteractiveRebaseRow {
  commit: InteractiveRebaseCommit,
  action: InteractiveRebaseAction,
  action_select: Entity<SelectState<Vec<InteractiveRebaseActionOption>>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InteractiveRebaseKeyboardCommand {
  SetAction(InteractiveRebaseAction),
  ToggleMoveMode,
  CancelMoveMode,
}

#[derive(IntoElement)]
struct InteractiveRebaseTodoListItem {
  index: usize,
  total_rows: usize,
  action: InteractiveRebaseAction,
  action_select: Entity<SelectState<Vec<InteractiveRebaseActionOption>>>,
  short_oid: String,
  summary: String,
  selected: bool,
  moving: bool,
  view: WeakEntity<InteractiveRebaseTodoView>,
}

impl InteractiveRebaseTodoListItem {
  fn new(
    index: usize,
    total_rows: usize,
    row: &InteractiveRebaseRow,
    moving: bool,
    view: WeakEntity<InteractiveRebaseTodoView>,
  ) -> Self {
    Self {
      index,
      total_rows,
      action: row.action,
      action_select: row.action_select.clone(),
      short_oid: row.commit.short_oid.clone(),
      summary: row.commit.summary.clone(),
      selected: false,
      moving,
      view,
    }
  }
}

impl Selectable for InteractiveRebaseTodoListItem {
  fn selected(mut self, selected: bool) -> Self {
    self.selected = selected;
    self
  }

  fn is_selected(&self) -> bool {
    self.selected
  }
}

impl RenderOnce for InteractiveRebaseTodoListItem {
  fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
    let theme = cx.theme().clone();
    let action = self.action;
    let is_first = self.index == 0;
    let is_last = self.index + 1 == self.total_rows;
    let row_id = format!("interactive-rebase-row-{}", self.index);
    let action_id = format!("interactive-rebase-row-action-{}", self.index);
    let up_id = format!("interactive-rebase-row-up-{}", self.index);
    let down_id = format!("interactive-rebase-row-down-{}", self.index);
    let summary_id = format!("interactive-rebase-row-summary-{}", self.index);
    let index_for_up = self.index;
    let index_for_down = self.index;
    let view_for_move_up = self.view.clone();
    let view_for_move_down = self.view.clone();
    let border_color = if self.moving {
      theme.status_blue()
    } else if self.selected {
      white()
    } else {
      theme.border.opacity(0.0)
    };

    h_flex()
      .id(row_id)
      .w_full()
      .items_center()
      .gap_2()
      .border_color(border_color)
      .border_2()
      .rounded(theme.radius)
      .child(
        div().id(action_id).w(px(130.)).child(
          Select::new(&self.action_select)
            .small()
            .menu_width(px(130.))
            .when(action == InteractiveRebaseAction::Drop, |select| {
              select.text_color(theme.red)
            }),
        ),
      )
      .child(
        h_flex()
          .items_center()
          .gap_1()
          .child(
            Button::new(up_id)
              .icon(IconName::ArrowUp)
              .small()
              .ghost()
              .disabled(is_first)
              .on_click(move |_, _, cx| {
                let _ = view_for_move_up.update(cx, |view, cx| {
                  view.move_row_up(index_for_up, cx);
                });
              }),
          )
          .child(
            Button::new(down_id)
              .icon(IconName::ArrowDown)
              .small()
              .ghost()
              .disabled(is_last)
              .on_click(move |_, _, cx| {
                let _ = view_for_move_down.update(cx, |view, cx| {
                  view.move_row_down(index_for_down, cx);
                });
              }),
          ),
      )
      .child(
        div()
          .id(summary_id)
          .min_w_0()
          .flex_1()
          .overflow_hidden()
          .text_ellipsis()
          .child(format!("{}  {}", self.short_oid, self.summary)),
      )
  }
}

struct InteractiveRebaseTodoListDelegate {
  view: WeakEntity<InteractiveRebaseTodoView>,
}

impl InteractiveRebaseTodoListDelegate {
  fn new(view: WeakEntity<InteractiveRebaseTodoView>) -> Self {
    Self { view }
  }

  fn row_count(&self, cx: &App) -> usize {
    let Some(view) = self.view.upgrade() else {
      return 0;
    };
    view.read(cx).rows.len()
  }

  fn row_at(&self, ix: IndexPath, cx: &App) -> Option<InteractiveRebaseRow> {
    let view = self.view.upgrade()?;
    view.read(cx).rows.get(ix.row).cloned()
  }

  fn moving_row_index(&self, cx: &App) -> Option<usize> {
    let view = self.view.upgrade()?;
    view.read(cx).moving_row_index
  }
}

impl ListDelegate for InteractiveRebaseTodoListDelegate {
  type Item = InteractiveRebaseTodoListItem;

  fn items_count(&self, _section: usize, cx: &App) -> usize {
    self.row_count(cx)
  }

  fn render_item(
    &mut self,
    ix: IndexPath,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> Option<Self::Item> {
    let row = self.row_at(ix, cx)?;
    let index = ix.row;
    let total_rows = self.row_count(cx);
    let moving = self
      .moving_row_index(cx)
      .map(|moving_index| moving_index == index)
      .unwrap_or(false);
    Some(InteractiveRebaseTodoListItem::new(
      index,
      total_rows,
      &row,
      moving,
      self.view.clone(),
    ))
  }

  fn render_empty(
    &mut self,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> impl IntoElement {
    div()
      .id("interactive-rebase-rows-empty")
      .size_full()
      .flex()
      .items_center()
      .justify_center()
      .text_sm()
      .text_color(cx.theme().muted_foreground)
      .child("No commits available")
  }

  fn set_selected_index(
    &mut self,
    _ix: Option<IndexPath>,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) {
    cx.notify();
  }
}

#[derive(Clone, Copy)]
struct InteractiveRebaseActionOption {
  action: InteractiveRebaseAction,
}

impl SelectItem for InteractiveRebaseActionOption {
  type Value = InteractiveRebaseAction;

  fn title(&self) -> SharedString {
    InteractiveRebaseTodoView::action_label(self.action).into()
  }

  fn value(&self) -> &Self::Value {
    &self.action
  }
}

pub struct InteractiveRebaseTodoView {
  focus_handle: FocusHandle,
  target: InteractiveRebaseTarget,
  rows: Vec<InteractiveRebaseRow>,
  rows_list: Entity<ListState<InteractiveRebaseTodoListDelegate>>,
  moving_row_index: Option<usize>,
  last_selected_row_index: Option<usize>,
  error: Option<SharedString>,
  on_submit: Option<InteractiveRebaseTodoViewHandler>,
  on_cancel: Option<InteractiveRebaseTodoViewCancelHandler>,
  _subscriptions: Vec<Subscription>,
}

impl InteractiveRebaseTodoView {
  pub fn new(
    window: &mut Window,
    cx: &mut Context<Self>,
    config: InteractiveRebaseTodoViewConfig,
  ) -> Self {
    let mut rows = Vec::with_capacity(config.commits.len());
    for commit in config.commits {
      let action_select = cx.new(|cx| {
        let mut state = SelectState::new(Self::action_options(), None, window, cx);
        state.set_selected_value(&InteractiveRebaseAction::Pick, window, cx);
        state
      });
      let action_select_for_subscription = action_select.clone();
      cx.subscribe(
        &action_select,
        move |this, _, event: &SelectEvent<Vec<InteractiveRebaseActionOption>>, cx| {
          let SelectEvent::Confirm(Some(action)) = event else {
            return;
          };
          this.set_row_action_for_select(&action_select_for_subscription, *action, cx);
        },
      )
      .detach();

      rows.push(InteractiveRebaseRow {
        commit,
        action: InteractiveRebaseAction::Pick,
        action_select,
      });
    }

    let view = cx.entity().downgrade();
    let rows_list = cx.new(|cx| {
      ListState::new(
        InteractiveRebaseTodoListDelegate::new(view.clone()),
        window,
        cx,
      )
    });
    let has_rows = !rows.is_empty();
    rows_list.update(cx, |list, cx| {
      if has_rows {
        list.set_selected_index(Some(IndexPath::new(0)), window, cx);
      }
    });
    let _subscriptions = vec![cx.subscribe_in(
      &rows_list,
      window,
      |this, _, event: &ListEvent, window, cx| {
        if let ListEvent::Select(ix) = event {
          this.on_rows_list_select(*ix, window, cx);
        }
      },
    )];

    Self {
      focus_handle: cx.focus_handle(),
      target: config.target,
      rows,
      rows_list,
      moving_row_index: None,
      last_selected_row_index: has_rows.then_some(0),
      error: None,
      on_submit: Some(config.on_submit),
      on_cancel: Some(config.on_cancel),
      _subscriptions,
    }
  }

  fn action_options() -> Vec<InteractiveRebaseActionOption> {
    vec![
      InteractiveRebaseActionOption {
        action: InteractiveRebaseAction::Pick,
      },
      InteractiveRebaseActionOption {
        action: InteractiveRebaseAction::Squash,
      },
      InteractiveRebaseActionOption {
        action: InteractiveRebaseAction::Fixup,
      },
      InteractiveRebaseActionOption {
        action: InteractiveRebaseAction::Drop,
      },
    ]
  }

  fn action_label(action: InteractiveRebaseAction) -> &'static str {
    match action {
      InteractiveRebaseAction::Pick => "pick",
      InteractiveRebaseAction::Squash => "squash",
      InteractiveRebaseAction::Fixup => "fixup",
      InteractiveRebaseAction::Drop => "drop",
    }
  }

  fn keyboard_command(event: &KeyDownEvent) -> Option<InteractiveRebaseKeyboardCommand> {
    let modifiers = event.keystroke.modifiers;
    if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
      return None;
    }

    match event.keystroke.key.to_ascii_lowercase().as_str() {
      "p" => Some(InteractiveRebaseKeyboardCommand::SetAction(
        InteractiveRebaseAction::Pick,
      )),
      "s" => Some(InteractiveRebaseKeyboardCommand::SetAction(
        InteractiveRebaseAction::Squash,
      )),
      "f" => Some(InteractiveRebaseKeyboardCommand::SetAction(
        InteractiveRebaseAction::Fixup,
      )),
      "d" => Some(InteractiveRebaseKeyboardCommand::SetAction(
        InteractiveRebaseAction::Drop,
      )),
      "space" | " " => Some(InteractiveRebaseKeyboardCommand::ToggleMoveMode),
      "escape" => Some(InteractiveRebaseKeyboardCommand::CancelMoveMode),
      _ => None,
    }
  }

  fn validate_rows(&self) -> Result<(), SharedString> {
    let Some(first_kept) = self
      .rows
      .iter()
      .find(|row| row.action != InteractiveRebaseAction::Drop)
    else {
      return Err("At least one commit must stay in the interactive rebase todo.".into());
    };
    if first_kept.action != InteractiveRebaseAction::Pick {
      return Err("The first non-dropped commit must use pick.".into());
    }

    let mut has_previous_kept = false;
    for row in &self.rows {
      match row.action {
        InteractiveRebaseAction::Pick => {
          has_previous_kept = true;
        }
        InteractiveRebaseAction::Drop => {}
        InteractiveRebaseAction::Squash | InteractiveRebaseAction::Fixup => {
          if !has_previous_kept {
            return Err("squash/fixup require a previous picked commit.".into());
          }
          has_previous_kept = true;
        }
      }
    }

    Ok(())
  }

  fn set_row_action_for_select(
    &mut self,
    action_select: &Entity<SelectState<Vec<InteractiveRebaseActionOption>>>,
    action: InteractiveRebaseAction,
    cx: &mut Context<Self>,
  ) {
    let Some(row) = self
      .rows
      .iter_mut()
      .find(|row| row.action_select == *action_select)
    else {
      return;
    };
    row.action = action;
    self.error = None;
    cx.notify();
  }

  fn move_row_up(&mut self, index: usize, cx: &mut Context<Self>) -> usize {
    if index == 0 || index >= self.rows.len() {
      return index;
    }
    self.rows.swap(index, index - 1);
    if let Some(moving_index) = self.moving_row_index {
      if moving_index == index {
        self.moving_row_index = Some(index - 1);
      } else if moving_index + 1 == index {
        self.moving_row_index = Some(index);
      }
    }
    self.error = None;
    cx.notify();
    index - 1
  }

  fn move_row_down(&mut self, index: usize, cx: &mut Context<Self>) -> usize {
    if self.rows.is_empty() || index + 1 >= self.rows.len() {
      return index;
    }
    self.rows.swap(index, index + 1);
    if let Some(moving_index) = self.moving_row_index {
      if moving_index == index {
        self.moving_row_index = Some(index + 1);
      } else if moving_index == index + 1 {
        self.moving_row_index = Some(index);
      }
    }
    self.error = None;
    cx.notify();
    index + 1
  }

  fn selected_row_index(&self, cx: &App) -> Option<usize> {
    self.rows_list.read(cx).selected_index().map(|ix| ix.row)
  }

  fn set_selected_row_index(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
    self.last_selected_row_index = Some(index);
    self.rows_list.update(cx, |list, cx| {
      list.set_selected_index(Some(IndexPath::new(index)), window, cx);
    });
  }

  fn toggle_move_mode(&mut self, cx: &mut Context<Self>) {
    if self.moving_row_index.is_some() {
      self.moving_row_index = None;
      cx.notify();
      return;
    }

    self.moving_row_index = self.selected_row_index(cx);
    cx.notify();
  }

  fn cancel_move_mode(&mut self, cx: &mut Context<Self>) {
    if self.moving_row_index.take().is_some() {
      cx.notify();
    }
  }

  fn on_rows_list_select(&mut self, index: IndexPath, window: &mut Window, cx: &mut Context<Self>) {
    let next_selected = index.row;
    let previous_selected = self.last_selected_row_index.replace(next_selected);
    let Some(moving_index) = self.moving_row_index else {
      return;
    };

    let Some(previous_selected) = previous_selected else {
      self.moving_row_index = Some(next_selected);
      cx.notify();
      return;
    };

    if next_selected == previous_selected {
      return;
    }

    let moved_index = if next_selected + 1 == previous_selected {
      self.move_row_up(moving_index, cx)
    } else if next_selected == previous_selected + 1 {
      self.move_row_down(moving_index, cx)
    } else {
      self.moving_row_index = Some(next_selected);
      cx.notify();
      return;
    };

    self.moving_row_index = Some(moved_index);
    self.set_selected_row_index(moved_index, window, cx);
  }

  fn apply_action_to_selected_row(
    &mut self,
    action: InteractiveRebaseAction,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let selected_index = self.rows_list.read(cx).selected_index();
    let Some(index) = selected_index.map(|ix| ix.row) else {
      return;
    };
    let Some(row) = self.rows.get_mut(index) else {
      return;
    };

    row.action = action;
    row.action_select.update(cx, |select, cx| {
      select.set_selected_value(&action, window, cx)
    });
    self.error = None;
    cx.notify();
  }

  fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
    let Some(command) = Self::keyboard_command(event) else {
      return;
    };

    let list_focus_handle = self.rows_list.read(cx).focus_handle(cx);
    if !list_focus_handle.contains_focused(window, cx) {
      return;
    }

    match command {
      InteractiveRebaseKeyboardCommand::SetAction(action) => {
        self.apply_action_to_selected_row(action, window, cx);
        cx.stop_propagation();
      }
      InteractiveRebaseKeyboardCommand::ToggleMoveMode => {
        self.toggle_move_mode(cx);
        cx.stop_propagation();
      }
      InteractiveRebaseKeyboardCommand::CancelMoveMode => {
        self.cancel_move_mode(cx);
        cx.stop_propagation();
      }
    }
  }

  pub fn focus_rows_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let has_rows = !self.rows.is_empty();
    self.rows_list.update(cx, |list, cx| {
      if has_rows && list.selected_index().is_none() {
        list.set_selected_index(Some(IndexPath::new(0)), window, cx);
      }
      list.focus(window, cx);
    });
  }

  fn submit(&mut self, _: &gpui::ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
    if let Err(error) = self.validate_rows() {
      self.error = Some(error);
      cx.notify();
      return;
    }
    let Some(on_submit) = self.on_submit.clone() else {
      return;
    };

    let todo_entries = self
      .rows
      .iter()
      .map(|row| InteractiveRebaseTodoEntry {
        oid: row.commit.oid.clone(),
        action: row.action,
      })
      .collect::<Vec<_>>();

    match on_submit(self.target.clone(), todo_entries, window, cx) {
      Ok(()) => {}
      Err(error) => {
        self.error = Some(error);
        cx.notify();
      }
    }
  }

  fn cancel(&mut self, _: &gpui::ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
    if let Some(on_cancel) = self.on_cancel.clone() {
      on_cancel(window, cx);
    }
  }
}

impl Focusable for InteractiveRebaseTodoView {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

impl Render for InteractiveRebaseTodoView {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let target_label: SharedString = match &self.target {
      InteractiveRebaseTarget::Branch(branch) => format!("Target: {}", branch.name).into(),
      InteractiveRebaseTarget::BranchInPlace(branch) => {
        format!("Edit commits since {}", branch.name).into()
      }
      InteractiveRebaseTarget::HeadCount(count) => format!("Target: HEAD~{count}").into(),
    };
    let validation_error = self.validate_rows().err();
    let can_submit = validation_error.is_none();
    let error = self.error.clone().or(validation_error);

    v_flex()
      .track_focus(&self.focus_handle)
      .id("interactive-rebase-todo-view")
      .on_key_down(cx.listener(Self::on_key_down))
      .size_full()
      .min_h_0()
      .gap_3()
      .p_4()
      .child(
        v_flex()
          .gap_1()
          .child(
            div()
              .text_sm()
              .text_color(theme.foreground)
              .child(target_label),
          )
          .child(
            div()
              .text_sm()
              .text_color(theme.muted_foreground)
              .child("Choose action/order for each commit before starting interactive rebase."),
          )
          .child(
            div()
              .text_sm()
              .text_color(theme.muted_foreground)
              .child("Shortcuts: p pick, s squash, f fixup, d drop, space toggle move mode (use up/down while moving)."),
          ),
      )
      .child(
        div()
          .id("interactive-rebase-rows")
          .flex_1()
          .min_h_0()
          .overflow_hidden()
          .border_1()
          .border_color(theme.border)
          .rounded(theme.radius)
          .child(
            List::new(&self.rows_list)
              .w_full()
              .flex_1()
              .min_h_0()
              .p(px(6.)),
          ),
      )
      .when_some(error, |parent, error| {
        parent.child(div().text_sm().text_color(theme.red).child(error))
      })
      .child(
        h_flex()
          .w_full()
          .justify_end()
          .gap_2()
          .child(
            Button::new("interactive-rebase-cancel")
              .label("Cancel")
              .with_variant(ButtonVariant::Secondary)
              .on_click(cx.listener(Self::cancel)),
          )
          .child(
            Button::new("interactive-rebase-submit")
              .label("Start interactive rebase")
              .with_variant(ButtonVariant::Secondary)
              .disabled(!can_submit)
              .on_click(cx.listener(Self::submit)),
          ),
      )
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use gpui::{Keystroke, Modifiers};

  #[test]
  fn action_options_use_expected_order_and_labels() {
    let options = InteractiveRebaseTodoView::action_options();

    assert_eq!(
      options
        .iter()
        .map(|option| option.action)
        .collect::<Vec<_>>(),
      vec![
        InteractiveRebaseAction::Pick,
        InteractiveRebaseAction::Squash,
        InteractiveRebaseAction::Fixup,
        InteractiveRebaseAction::Drop,
      ]
    );
    assert_eq!(
      options
        .iter()
        .map(|option| option.title().to_string())
        .collect::<Vec<_>>(),
      vec!["pick", "squash", "fixup", "drop"]
    );
  }

  #[test]
  fn keyboard_command_matches_expected_keys() {
    let event = |key: &str, modifiers: Modifiers| KeyDownEvent {
      keystroke: Keystroke {
        modifiers,
        key: key.to_string(),
        key_char: Some(key.to_string()),
      },
      is_held: false,
      prefer_character_input: false,
    };

    assert_eq!(
      InteractiveRebaseTodoView::keyboard_command(&event("p", Modifiers::none())),
      Some(InteractiveRebaseKeyboardCommand::SetAction(
        InteractiveRebaseAction::Pick
      ))
    );
    assert_eq!(
      InteractiveRebaseTodoView::keyboard_command(&event("s", Modifiers::none())),
      Some(InteractiveRebaseKeyboardCommand::SetAction(
        InteractiveRebaseAction::Squash
      ))
    );
    assert_eq!(
      InteractiveRebaseTodoView::keyboard_command(&event("f", Modifiers::none())),
      Some(InteractiveRebaseKeyboardCommand::SetAction(
        InteractiveRebaseAction::Fixup
      ))
    );
    assert_eq!(
      InteractiveRebaseTodoView::keyboard_command(&event("d", Modifiers::none())),
      Some(InteractiveRebaseKeyboardCommand::SetAction(
        InteractiveRebaseAction::Drop
      ))
    );
    assert_eq!(
      InteractiveRebaseTodoView::keyboard_command(&event("space", Modifiers::none())),
      Some(InteractiveRebaseKeyboardCommand::ToggleMoveMode)
    );
    assert_eq!(
      InteractiveRebaseTodoView::keyboard_command(&event("up", Modifiers::none())),
      None
    );
    assert_eq!(
      InteractiveRebaseTodoView::keyboard_command(&event("down", Modifiers::none())),
      None
    );
    assert_eq!(
      InteractiveRebaseTodoView::keyboard_command(&event("escape", Modifiers::none())),
      Some(InteractiveRebaseKeyboardCommand::CancelMoveMode)
    );

    let mut shift = Modifiers::none();
    shift.shift = true;
    assert_eq!(
      InteractiveRebaseTodoView::keyboard_command(&event("p", shift)),
      Some(InteractiveRebaseKeyboardCommand::SetAction(
        InteractiveRebaseAction::Pick
      ))
    );

    let mut cmd = Modifiers::none();
    cmd.platform = true;
    assert_eq!(
      InteractiveRebaseTodoView::keyboard_command(&event("p", cmd)),
      None
    );
    assert_eq!(
      InteractiveRebaseTodoView::keyboard_command(&event("x", Modifiers::none())),
      None
    );
  }
}
