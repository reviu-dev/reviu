use gpui::{AnyWindowHandle, App, Window, div, prelude::*};
use gpui_component::IndexPath;
use gpui_component::notification::Notification;
use gpui_component::select::{Select, SelectState};
use smol::unblock;
use ui::{ConfirmDialog, Input, InputState, Textarea, TextareaState, WindowExt};

use crate::{api::ApiClient, workspace::WorkspaceApi};

pub fn open_feedback_dialog(window: &mut Window, _cx: &mut App) {
  // Defer to next frame so the command palette dialog closes first
  window.on_next_frame(move |window, cx| {
    open_feedback_dialog_inner(window, cx);
  });
}

fn open_feedback_dialog_inner(window: &mut Window, cx: &mut App) {
  let api = WorkspaceApi::global(cx).api.clone();
  let window_handle = window.window_handle();

  let type_select = cx.new(|cx| {
    SelectState::new(
      vec!["Bug Report", "Feature Request"],
      Some(IndexPath::default()),
      window,
      cx,
    )
  });

  let title_input = cx.new(|cx| InputState::new(window, cx).placeholder("Brief summary..."));

  let description_input = cx.new(|cx| {
    TextareaState::new(window, cx)
      .auto_grow(3, 8)
      .placeholder("Describe in detail...")
  });

  let title_input_for_dialog = title_input.clone();
  let description_input_for_dialog = description_input.clone();
  let type_select_for_dialog = type_select.clone();

  window.open_alert_dialog(cx, move |alert, _, _| {
    let api = api.clone();
    let title_input = title_input_for_dialog.clone();
    let description_input = description_input_for_dialog.clone();
    let type_select = type_select_for_dialog.clone();

    ConfirmDialog::new(
      "Send Feedback",
      div()
        .w_full()
        .flex()
        .mt_2()
        .flex_col()
        .gap_3()
        .child(Select::new(&type_select).placeholder("Select type..."))
        .child(Input::new(&title_input).w_full())
        .child(Textarea::new(&description_input).w_full()),
    )
    .content_as_body()
    .confirm_text("Submit")
    .on_confirm(move |_, _, cx| {
      let feedback_type_val = type_select
        .read(cx)
        .selected_value()
        .map(|v| match *v {
          "Feature Request" => "feature",
          _ => "bug",
        })
        .unwrap_or("bug")
        .to_string();
      let title_val = title_input.read(cx).value().to_string();
      let description_val = description_input.read(cx).value().to_string();

      if title_val.trim().is_empty() {
        return false;
      }

      submit_feedback(
        api.clone(),
        window_handle,
        feedback_type_val,
        title_val,
        description_val,
        cx,
      );

      true
    })
    .build(alert)
  });
}

fn submit_feedback(
  api: ApiClient,
  window_handle: AnyWindowHandle,
  feedback_type: String,
  title: String,
  description: String,
  cx: &mut App,
) {
  cx.spawn(async move |cx| {
    let result = unblock(move || api.submit_feedback(&feedback_type, &title, &description)).await;

    let _ = cx.update_window(window_handle, |_, window, cx| match result {
      Ok(()) => {
        window.push_notification(
          Notification::success("Feedback submitted successfully. Thank you!"),
          cx,
        );
      }
      Err(_) => {
        window.push_notification(Notification::error("Failed to submit feedback"), cx);
      }
    });
  })
  .detach();
}
