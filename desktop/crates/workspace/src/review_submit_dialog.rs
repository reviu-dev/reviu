//! Finishing a review: the decision, its message, and the call that sends both.

use std::rc::Rc;

use gpui::{
  AnyWindowHandle, App, Context, Entity, FocusHandle, Focusable, IntoElement, ParentElement,
  Render, SharedString, Styled, Task, Window, div, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Disableable as _,
  dialog::{DialogDescription, DialogFooter, DialogHeader, DialogTitle},
  notification::Notification,
  radio::{Radio, RadioGroup},
  v_flex,
};
use ui::{
  Button, ButtonVariants as _, StatusThemeExt as _, Textarea, TextareaState, WindowExt as _,
};

use crate::api::ApiClient;
use crate::pull_request_review_submission::{
  ReviewDecision, decision_from_index, decision_index, validate_review_submission,
};

/// Invoked once the review is on GitHub: what showed it as pending has to look
/// again.
pub(crate) type ReviewSubmittedHandler = Rc<dyn Fn(&mut App)>;

/// The pull request a review is being submitted on, and what is waiting on it.
#[derive(Clone, Debug)]
pub(crate) struct ReviewSubmissionTarget {
  pub owner: String,
  pub repo: String,
  pub number: u64,
  /// The viewer's unsubmitted review, when they have one. Without it the
  /// decision goes out on its own.
  pub pending_review_id: Option<String>,
  pub pending_comment_count: usize,
  pub viewer_is_author: bool,
}

struct SubmitReviewDialog {
  api: ApiClient,
  window_handle: AnyWindowHandle,
  target: ReviewSubmissionTarget,
  on_submitted: ReviewSubmittedHandler,
  decision: ReviewDecision,
  body_input: Entity<TextareaState>,
  submitting: bool,
  error: Option<SharedString>,
  submit_task: Option<Task<()>>,
}

impl SubmitReviewDialog {
  fn new(
    api: ApiClient,
    window_handle: AnyWindowHandle,
    target: ReviewSubmissionTarget,
    on_submitted: ReviewSubmittedHandler,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    Self {
      api,
      window_handle,
      target,
      on_submitted,
      decision: ReviewDecision::default(),
      body_input: cx.new(|cx| {
        TextareaState::new(window, cx)
          .auto_grow(3, 10)
          .placeholder("Leave a comment...")
      }),
      submitting: false,
      error: None,
      submit_task: None,
    }
  }

  fn select_decision(&mut self, index: usize, cx: &mut Context<Self>) {
    let decision = decision_from_index(index);
    if self.target.viewer_is_author && !decision.allowed_for_author() {
      return;
    }
    self.decision = decision;
    self.error = None;
    cx.notify();
  }

  fn submit(&mut self, cx: &mut Context<Self>) {
    if self.submitting {
      return;
    }
    let body = self.body_input.read(cx).value().to_string();
    if let Some(error) =
      validate_review_submission(self.decision, body.as_str(), self.target.viewer_is_author)
    {
      self.error = Some(error);
      cx.notify();
      return;
    }

    let api = self.api.clone();
    let owner = self.target.owner.clone();
    let repo = self.target.repo.clone();
    let number = self.target.number;
    let pending_review_id = self.target.pending_review_id.clone();
    let event = self.decision.api_event();
    let on_submitted = self.on_submitted.clone();
    let window_handle = self.window_handle;

    self.error = None;
    self.submitting = true;
    cx.notify();

    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move {
          match pending_review_id {
            Some(review_id) => {
              api.submit_pending_review(&owner, &repo, number, &review_id, event, &body)
            }
            None => api.submit_pull_request_review(&owner, &repo, number, event, &body),
          }
        })
        .await;

      let _ = cx.update_window(window_handle, |_, window, cx| {
        let _ = this.update(cx, |this, cx| {
          this.submitting = false;
          cx.notify();
        });

        match result {
          Ok(_) => {
            on_submitted(cx);
            window.close_dialog(cx);
            window.push_notification(
              Notification::info(format!("Review submitted on #{number}")),
              cx,
            );
          }
          Err(error) => {
            let _ = this.update(cx, |this, cx| {
              this.error = Some(error.to_string().into());
              cx.notify();
            });
          }
        }
      });
    });

    self.submit_task = Some(task);
  }

  /// What goes out with the decision, so nobody submits four comments thinking
  /// they were only approving.
  fn comments_description(&self) -> String {
    let number = self.target.number;
    match self.target.pending_comment_count {
      0 => format!("Your decision on #{number}, with no line comments waiting."),
      1 => format!("1 comment goes out on #{number} with this review."),
      count => format!("{count} comments go out on #{number} with this review."),
    }
  }
}

impl Focusable for SubmitReviewDialog {
  fn focus_handle(&self, cx: &App) -> FocusHandle {
    self.body_input.read(cx).focus_handle(cx)
  }
}

impl Render for SubmitReviewDialog {
  fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let viewer_is_author = self.target.viewer_is_author;
    let author_tooltip: SharedString =
      "You cannot approve or request changes on your own pull request.".into();
    let selected = decision_index(self.decision);

    let mut choices = RadioGroup::vertical("submit-review-decision")
      .selected_index(Some(selected))
      .disabled(self.submitting)
      .on_click(cx.listener(|this, index: &usize, _, cx| this.select_decision(*index, cx)));
    for decision in ReviewDecision::ALL {
      let blocked = viewer_is_author && !decision.allowed_for_author();
      choices = choices.child(
        Radio::new(SharedString::from(format!(
          "submit-review-decision-{}",
          decision_index(decision)
        )))
        .label(decision.label())
        .disabled(blocked)
        .when(blocked, |this| this.tooltip(author_tooltip.clone())),
      );
    }

    div()
      .id("submit-review-dialog")
      .flex()
      .flex_col()
      .child(
        DialogHeader::new()
          .p_4()
          .child(DialogTitle::new().child("Submit review"))
          .child(DialogDescription::new().child(self.comments_description())),
      )
      .child(
        v_flex()
          .px_4()
          .pb_4()
          .gap_3()
          .child(choices)
          .child(
            Textarea::new(&self.body_input)
              .w_full()
              .disabled(self.submitting),
          )
          .when_some(self.error.clone(), |this, error| {
            this.child(div().text_xs().text_color(theme.status_red()).child(error))
          }),
      )
      .child(
        DialogFooter::new()
          .px_4()
          .pb_4()
          .pt_1()
          .justify_end()
          .child(
            Button::new("cancel-submit-review")
              .label("Cancel")
              .outline()
              .disabled(self.submitting)
              .on_click(|_, window, cx| window.close_dialog(cx)),
          )
          .child(
            Button::new("confirm-submit-review")
              .label("Submit review")
              .primary()
              .loading(self.submitting)
              .disabled(self.submitting)
              .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
          ),
      )
  }
}

pub(crate) fn open_submit_review_dialog(
  api: ApiClient,
  window_handle: AnyWindowHandle,
  target: ReviewSubmissionTarget,
  on_submitted: ReviewSubmittedHandler,
  window: &mut Window,
  cx: &mut App,
) {
  let dialog =
    cx.new(|cx| SubmitReviewDialog::new(api, window_handle, target, on_submitted, window, cx));
  let dialog_for_overlay = dialog.clone();
  let dialog_for_focus = dialog.clone();

  window.open_dialog(cx, move |overlay, _, _| {
    overlay.p_0().w(px(460.0)).child(dialog_for_overlay.clone())
  });

  window.on_next_frame(move |window, cx| {
    let focus_handle = dialog_for_focus.read(cx).focus_handle(cx);
    window.focus(&focus_handle, cx);
  });
}
