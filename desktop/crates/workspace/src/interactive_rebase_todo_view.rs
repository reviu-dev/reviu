use std::sync::Arc;

use git::{
  InteractiveRebaseAction, InteractiveRebaseCommit, InteractiveRebaseTarget,
  InteractiveRebaseTodoEntry,
};
use gpui::{
  App, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement, KeyDownEvent,
  ParentElement, Render, RenderOnce, SharedString, Styled, WeakEntity, Window, div, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Disableable, IconName, IndexPath, Selectable, Sizable,
  button::{Button, ButtonVariant, ButtonVariants as _},
  h_flex,
  list::{List, ListDelegate, ListState},
  select::{Select, SelectEvent, SelectItem, SelectState},
  v_flex,
};

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

#[derive(IntoElement)]
struct InteractiveRebaseTodoListItem {
  index: usize,
  total_rows: usize,
  action: InteractiveRebaseAction,
  action_select: Entity<SelectState<Vec<InteractiveRebaseActionOption>>>,
  short_oid: String,
  summary: String,
  selected: bool,
  view: WeakEntity<InteractiveRebaseTodoView>,
}

impl InteractiveRebaseTodoListItem {
  fn new(
    index: usize,
    total_rows: usize,
    row: &InteractiveRebaseRow,
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

    h_flex()
      .id(row_id)
      .w_full()
      .items_center()
      .gap_2()
      .when_else(
        self.selected,
        |row| row.border_color(theme.primary),
        |row| row.border_color(theme.border.opacity(0.0)),
      )
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
    Some(InteractiveRebaseTodoListItem::new(
      index,
      total_rows,
      &row,
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
  error: Option<SharedString>,
  on_submit: Option<InteractiveRebaseTodoViewHandler>,
  on_cancel: Option<InteractiveRebaseTodoViewCancelHandler>,
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

    Self {
      focus_handle: cx.focus_handle(),
      target: config.target,
      rows,
      rows_list,
      error: None,
      on_submit: Some(config.on_submit),
      on_cancel: Some(config.on_cancel),
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

  fn shortcut_action(event: &KeyDownEvent) -> Option<InteractiveRebaseAction> {
    let modifiers = event.keystroke.modifiers;
    if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
      return None;
    }

    match event.keystroke.key.to_ascii_lowercase().as_str() {
      "p" => Some(InteractiveRebaseAction::Pick),
      "s" => Some(InteractiveRebaseAction::Squash),
      "f" => Some(InteractiveRebaseAction::Fixup),
      "d" => Some(InteractiveRebaseAction::Drop),
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

  fn move_row_up(&mut self, index: usize, cx: &mut Context<Self>) {
    if index == 0 || index >= self.rows.len() {
      return;
    }
    self.rows.swap(index, index - 1);
    self.error = None;
    cx.notify();
  }

  fn move_row_down(&mut self, index: usize, cx: &mut Context<Self>) {
    if self.rows.is_empty() || index + 1 >= self.rows.len() {
      return;
    }
    self.rows.swap(index, index + 1);
    self.error = None;
    cx.notify();
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
    let Some(action) = Self::shortcut_action(event) else {
      return;
    };

    let list_focus_handle = self.rows_list.read(cx).focus_handle(cx);
    if !list_focus_handle.contains_focused(window, cx) {
      return;
    }

    self.apply_action_to_selected_row(action, window, cx);
    cx.stop_propagation();
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
  fn shortcut_action_matches_expected_keys() {
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
      InteractiveRebaseTodoView::shortcut_action(&event("p", Modifiers::none())),
      Some(InteractiveRebaseAction::Pick)
    );
    assert_eq!(
      InteractiveRebaseTodoView::shortcut_action(&event("s", Modifiers::none())),
      Some(InteractiveRebaseAction::Squash)
    );
    assert_eq!(
      InteractiveRebaseTodoView::shortcut_action(&event("f", Modifiers::none())),
      Some(InteractiveRebaseAction::Fixup)
    );
    assert_eq!(
      InteractiveRebaseTodoView::shortcut_action(&event("d", Modifiers::none())),
      Some(InteractiveRebaseAction::Drop)
    );

    let mut shift = Modifiers::none();
    shift.shift = true;
    assert_eq!(
      InteractiveRebaseTodoView::shortcut_action(&event("p", shift)),
      Some(InteractiveRebaseAction::Pick)
    );

    let mut cmd = Modifiers::none();
    cmd.platform = true;
    assert_eq!(
      InteractiveRebaseTodoView::shortcut_action(&event("p", cmd)),
      None
    );
    assert_eq!(
      InteractiveRebaseTodoView::shortcut_action(&event("x", Modifiers::none())),
      None
    );
  }
}
