//! Confirming a merge: the method is already chosen, this is where the commit
//! title and message are adjusted before the call.

use std::rc::Rc;

use gpui::{
  App, Context, Entity, FocusHandle, Focusable as _, IntoElement, ParentElement, Render, Styled,
  Window, div, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _,
  dialog::{DialogDescription, DialogFooter, DialogHeader, DialogTitle},
  input::{Input, InputState},
  v_flex,
};
use ui::{Button, ButtonVariants as _, Textarea, TextareaState, WindowExt as _};

use crate::api::{GithubMergeCommitDefaults, GithubPullRequestMergeMethod};
use crate::pull_request_merge::{
  merge_method_confirm_label, merge_method_label, merge_method_supports_commit_message,
};

/// Invoked on confirm with the commit title and message to send. `None` means
/// the field was left blank and GitHub generates it.
pub(crate) type MergeConfirmedHandler = Rc<dyn Fn(Option<String>, Option<String>, &mut App)>;

struct MergeDialog {
  number: u64,
  method: GithubPullRequestMergeMethod,
  /// `None` when the method takes no commit inputs (rebase).
  inputs: Option<MergeDialogInputs>,
  on_confirmed: MergeConfirmedHandler,
}

struct MergeDialogInputs {
  title: Entity<InputState>,
  message: Entity<TextareaState>,
}

impl MergeDialog {
  fn new(
    number: u64,
    method: GithubPullRequestMergeMethod,
    defaults: Option<GithubMergeCommitDefaults>,
    on_confirmed: MergeConfirmedHandler,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    let inputs = merge_method_supports_commit_message(method).then(|| {
      let title = cx.new(|cx| InputState::new(window, cx).placeholder("GitHub picks the title"));
      let message = cx.new(|cx| {
        TextareaState::new(window, cx)
          .auto_grow(3, 10)
          .placeholder("GitHub writes the message")
      });
      if let Some(defaults) = defaults {
        if !defaults.title.is_empty() {
          title.update(cx, |input, cx| input.set_value(&defaults.title, window, cx));
        }
        if !defaults.message.is_empty() {
          message.update(cx, |input, cx| {
            input.set_value(&defaults.message, window, cx)
          });
        }
      }
      MergeDialogInputs { title, message }
    });

    Self {
      number,
      method,
      inputs,
      on_confirmed,
    }
  }

  /// Rebase has no input: the dialog's own focus stays put, so Escape and the
  /// close button keep reaching it.
  fn input_focus_handle(&self, cx: &App) -> Option<FocusHandle> {
    self
      .inputs
      .as_ref()
      .map(|inputs| inputs.title.read(cx).focus_handle(cx))
  }

  fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let (title, message) = match &self.inputs {
      Some(inputs) => (
        commit_field(&inputs.title.read(cx).value()),
        commit_field(&inputs.message.read(cx).value()),
      ),
      None => (None, None),
    };
    (self.on_confirmed)(title, message, cx);
    window.close_dialog(cx);
  }

  fn description(&self) -> String {
    let number = self.number;
    match self.method {
      GithubPullRequestMergeMethod::Merge | GithubPullRequestMergeMethod::Squash => {
        format!("#{number} into its base branch. Blank fields let GitHub write the commit.")
      }
      GithubPullRequestMergeMethod::Rebase => {
        format!("The commits of #{number} land on the base branch as they are, messages included.")
      }
    }
  }
}

impl Render for MergeDialog {
  fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    div()
      .id("merge-dialog")
      .flex()
      .flex_col()
      .child(
        DialogHeader::new()
          .p_4()
          .child(DialogTitle::new().child(merge_method_label(self.method)))
          .child(DialogDescription::new().child(self.description())),
      )
      .when_some(self.inputs.as_ref(), |this, inputs| {
        this.child(
          v_flex()
            .px_4()
            .pb_4()
            .gap_3()
            .child(
              v_flex()
                .debug_selector(|| MERGE_DIALOG_TITLE_DEBUG_SELECTOR.to_string())
                .gap_1()
                .child(
                  div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Commit message"),
                )
                .child(Input::new(&inputs.title).w_full()),
            )
            .child(
              v_flex()
                .debug_selector(|| MERGE_DIALOG_MESSAGE_DEBUG_SELECTOR.to_string())
                .gap_1()
                .child(
                  div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Extended description"),
                )
                .child(Textarea::new(&inputs.message).w_full()),
            ),
        )
      })
      .child(
        DialogFooter::new()
          .px_4()
          .pb_4()
          .pt_1()
          .justify_end()
          .child(
            Button::new("cancel-merge")
              .label("Cancel")
              .outline()
              .on_click(|_, window, cx| window.close_dialog(cx)),
          )
          .child(
            Button::new("confirm-merge")
              .debug_selector(|| MERGE_DIALOG_CONFIRM_DEBUG_SELECTOR.to_string())
              .label(merge_method_confirm_label(self.method))
              .primary()
              .on_click(cx.listener(|this, _, window, cx| this.confirm(window, cx))),
          ),
      )
  }
}

pub(crate) const MERGE_DIALOG_CONFIRM_DEBUG_SELECTOR: &str = "merge-dialog-confirm";
pub(crate) const MERGE_DIALOG_TITLE_DEBUG_SELECTOR: &str = "merge-dialog-title";
pub(crate) const MERGE_DIALOG_MESSAGE_DEBUG_SELECTOR: &str = "merge-dialog-message";

/// Blank means GitHub generates: an empty string sent as-is would make a
/// commit without a title.
fn commit_field(value: &str) -> Option<String> {
  let trimmed = value.trim();
  (!trimmed.is_empty()).then(|| trimmed.to_string())
}

pub(crate) fn open_merge_dialog(
  number: u64,
  method: GithubPullRequestMergeMethod,
  defaults: Option<GithubMergeCommitDefaults>,
  on_confirmed: MergeConfirmedHandler,
  window: &mut Window,
  cx: &mut App,
) {
  let dialog = cx.new(|cx| MergeDialog::new(number, method, defaults, on_confirmed, window, cx));
  let dialog_for_overlay = dialog.clone();
  let dialog_for_focus = dialog.clone();

  window.open_dialog(cx, move |overlay, _, _| {
    overlay.p_0().w(px(460.0)).child(dialog_for_overlay.clone())
  });

  window.on_next_frame(move |window, cx| {
    if let Some(focus_handle) = dialog_for_focus.read(cx).input_focus_handle(cx) {
      window.focus(&focus_handle, cx);
    }
  });
}

#[cfg(test)]
mod tests {
  use std::cell::RefCell;

  use gpui::{TestAppContext, VisualTestContext};

  use super::*;

  #[test]
  fn a_blank_field_lets_github_generate() {
    assert_eq!(commit_field(""), None);
    assert_eq!(commit_field("   \n  "), None);
    assert_eq!(commit_field("  Ship it  "), Some("Ship it".to_string()));
  }

  struct Page;

  impl Render for Page {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
      div()
        .size_full()
        .children(gpui_component::Root::render_dialog_layer(window, cx))
    }
  }

  fn dialog_page(cx: &mut TestAppContext) -> &mut VisualTestContext {
    cx.update(gpui_component::init);
    let (_root, cx) = cx.add_window_view(|window, cx| {
      let page = cx.new(|_| Page);
      gpui_component::Root::new(page, window, cx)
    });
    cx
  }

  fn defaults() -> GithubMergeCommitDefaults {
    GithubMergeCommitDefaults {
      title: "Add a rate limiter (#42)".to_string(),
      message: "* first\n\n* second".to_string(),
    }
  }

  #[gpui::test]
  async fn the_form_prefills_and_hands_back_what_it_shows(cx: &mut TestAppContext) {
    type ConfirmedFields = std::rc::Rc<RefCell<Option<(Option<String>, Option<String>)>>>;
    let cx = dialog_page(cx);
    let confirmed: ConfirmedFields = std::rc::Rc::new(RefCell::new(None));
    let seen = confirmed.clone();
    cx.update(|window, cx| {
      open_merge_dialog(
        42,
        GithubPullRequestMergeMethod::Squash,
        Some(defaults()),
        std::rc::Rc::new(move |title, message, _| {
          *seen.borrow_mut() = Some((title, message));
        }),
        window,
        cx,
      );
    });
    cx.run_until_parked();

    assert!(cx.debug_bounds(MERGE_DIALOG_TITLE_DEBUG_SELECTOR).is_some());
    assert!(
      cx.debug_bounds(MERGE_DIALOG_MESSAGE_DEBUG_SELECTOR)
        .is_some()
    );

    let confirm = cx
      .debug_bounds(MERGE_DIALOG_CONFIRM_DEBUG_SELECTOR)
      .expect("confirm button bounds");
    cx.simulate_click(confirm.center(), gpui::Modifiers::default());
    cx.run_until_parked();

    // Untouched fields hand back the prefill: what you saw is what is sent.
    assert_eq!(
      confirmed.borrow().clone(),
      Some((
        Some("Add a rate limiter (#42)".to_string()),
        Some("* first\n\n* second".to_string()),
      ))
    );
    assert!(!cx.update(|window, cx| window.has_active_dialog(cx)));
  }

  #[gpui::test]
  async fn escape_closes_the_dialog_even_without_inputs(cx: &mut TestAppContext) {
    let cx = dialog_page(cx);
    cx.update(|window, cx| {
      open_merge_dialog(
        42,
        GithubPullRequestMergeMethod::Rebase,
        None,
        std::rc::Rc::new(|_, _, _| {}),
        window,
        cx,
      );
    });
    cx.run_until_parked();
    assert!(cx.update(|window, cx| window.has_active_dialog(cx)));

    // No input to hand focus to: the dialog's own focus must stay, or Escape
    // and the close button dispatch into the void.
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();

    assert!(!cx.update(|window, cx| window.has_active_dialog(cx)));
  }

  #[gpui::test]
  async fn rebase_offers_no_commit_text_to_edit(cx: &mut TestAppContext) {
    let cx = dialog_page(cx);
    cx.update(|window, cx| {
      open_merge_dialog(
        42,
        GithubPullRequestMergeMethod::Rebase,
        None,
        std::rc::Rc::new(|_, _, _| {}),
        window,
        cx,
      );
    });
    cx.run_until_parked();

    // Rebase keeps the commits as they are: no field pretends otherwise.
    assert!(cx.debug_bounds(MERGE_DIALOG_TITLE_DEBUG_SELECTOR).is_none());
    assert!(
      cx.debug_bounds(MERGE_DIALOG_MESSAGE_DEBUG_SELECTOR)
        .is_none()
    );
    assert!(
      cx.debug_bounds(MERGE_DIALOG_CONFIRM_DEBUG_SELECTOR)
        .is_some()
    );
  }
}
