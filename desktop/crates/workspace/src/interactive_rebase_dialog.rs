use std::sync::Arc;

use git::{
  InteractiveRebaseAction, InteractiveRebaseCommit, InteractiveRebaseTarget,
  InteractiveRebaseTodoEntry,
};
use gpui::{
  App, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement,
  Render, SharedString, Styled, Window, div, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Disableable, IconName, Sizable, WindowExt,
  button::{Button, ButtonVariant, ButtonVariants as _},
  h_flex,
  scroll::ScrollableElement,
  select::{Select, SelectEvent, SelectItem, SelectState},
  v_flex,
};

pub type InteractiveRebaseDialogHandler = Arc<
  dyn Fn(
      InteractiveRebaseTarget,
      Vec<InteractiveRebaseTodoEntry>,
      &mut Window,
      &mut App,
    ) -> Result<(), SharedString>
    + Send
    + Sync,
>;

pub struct InteractiveRebaseDialogConfig {
  pub target: InteractiveRebaseTarget,
  pub commits: Vec<InteractiveRebaseCommit>,
  pub on_submit: InteractiveRebaseDialogHandler,
}

impl InteractiveRebaseDialogConfig {
  pub fn new(
    target: InteractiveRebaseTarget,
    commits: Vec<InteractiveRebaseCommit>,
    on_submit: InteractiveRebaseDialogHandler,
  ) -> Self {
    Self {
      target,
      commits,
      on_submit,
    }
  }
}

#[derive(Clone)]
struct InteractiveRebaseRow {
  commit: InteractiveRebaseCommit,
  action: InteractiveRebaseAction,
  action_select: Entity<SelectState<Vec<InteractiveRebaseActionOption>>>,
}

#[derive(Clone, Copy)]
struct InteractiveRebaseActionOption {
  action: InteractiveRebaseAction,
}

impl SelectItem for InteractiveRebaseActionOption {
  type Value = InteractiveRebaseAction;

  fn title(&self) -> SharedString {
    InteractiveRebaseDialog::action_label(self.action).into()
  }

  fn value(&self) -> &Self::Value {
    &self.action
  }
}

pub struct InteractiveRebaseDialog {
  focus_handle: FocusHandle,
  target: InteractiveRebaseTarget,
  rows: Vec<InteractiveRebaseRow>,
  error: Option<SharedString>,
  on_submit: Option<InteractiveRebaseDialogHandler>,
}

impl InteractiveRebaseDialog {
  pub fn new(
    window: &mut Window,
    cx: &mut Context<Self>,
    config: InteractiveRebaseDialogConfig,
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

    Self {
      focus_handle: cx.focus_handle(),
      target: config.target,
      rows,
      error: None,
      on_submit: Some(config.on_submit),
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
      Ok(()) => window.close_dialog(cx),
      Err(error) => {
        self.error = Some(error);
        cx.notify();
      }
    }
  }

  fn render_row(
    &self,
    index: usize,
    row: &InteractiveRebaseRow,
    theme: &gpui_component::Theme,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let action = row.action;
    let is_first = index == 0;
    let is_last = index + 1 == self.rows.len();
    let row_id = format!("interactive-rebase-row-{index}");
    let action_id = format!("interactive-rebase-row-action-{index}");
    let up_id = format!("interactive-rebase-row-up-{index}");
    let down_id = format!("interactive-rebase-row-down-{index}");
    let summary_id = format!("interactive-rebase-row-summary-{index}");
    let short_oid = row.commit.short_oid.clone();
    let summary = row.commit.summary.clone();

    h_flex()
      .id(row_id)
      .w_full()
      .items_center()
      .gap_2()
      .p_2()
      .border_1()
      .border_color(theme.border)
      .rounded(theme.radius)
      .child(
        div().id(action_id).w(px(130.)).child(
          Select::new(&row.action_select)
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
              .on_click(cx.listener(move |this, _, _, cx| {
                this.move_row_up(index, cx);
              })),
          )
          .child(
            Button::new(down_id)
              .icon(IconName::ArrowDown)
              .small()
              .ghost()
              .disabled(is_last)
              .on_click(cx.listener(move |this, _, _, cx| {
                this.move_row_down(index, cx);
              })),
          ),
      )
      .child(
        div()
          .id(summary_id)
          .min_w_0()
          .flex_1()
          .overflow_hidden()
          .text_ellipsis()
          .child(format!("{short_oid}  {summary}")),
      )
  }
}

impl Focusable for InteractiveRebaseDialog {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

impl Render for InteractiveRebaseDialog {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let target_label: SharedString = match &self.target {
      InteractiveRebaseTarget::Branch(branch) => format!("Target: {}", branch.name).into(),
      InteractiveRebaseTarget::HeadCount(count) => format!("Target: HEAD~{count}").into(),
    };
    let validation_error = self.validate_rows().err();
    let can_submit = validation_error.is_none();
    let error = self.error.clone().or(validation_error);
    let rows = self
      .rows
      .iter()
      .enumerate()
      .fold(v_flex().w_full().gap_2(), |container, (index, row)| {
        container.child(self.render_row(index, row, &theme, cx))
      });

    v_flex()
      .track_focus(&self.focus_handle)
      .id("interactive-rebase-dialog")
      .w(px(780.0))
      .max_w_full()
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
              .child("Choose action/order for each commit before starting the rebase."),
          ),
      )
      .child(
        div()
          .id("interactive-rebase-rows")
          .max_h(px(360.0))
          .overflow_y_scrollbar()
          .child(rows),
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
              .on_click(|_, window, cx| {
                window.close_dialog(cx);
              }),
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

  #[test]
  fn action_options_use_expected_order_and_labels() {
    let options = InteractiveRebaseDialog::action_options();

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
}
