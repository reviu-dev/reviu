use std::rc::Rc;

use gpui::{AnyElement, App, ClickEvent, IntoElement, ParentElement as _, SharedString, Window};
use gpui_component::Sizable as _;
use gpui_component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_component::dialog::{AlertDialog, Cancel, Confirm, DialogFooter};

type ConfirmDialogHandler = dyn Fn(&ClickEvent, &mut Window, &mut App) -> bool;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConfirmDialogContentPlacement {
  Description,
  Body,
}

#[derive(Clone, Debug, PartialEq)]
struct ResolvedConfirmDialogProps {
  confirm_text: SharedString,
  cancel_text: SharedString,
  confirm_variant: ButtonVariant,
  cancel_variant: ButtonVariant,
}

pub struct ConfirmDialog {
  title: SharedString,
  message: AnyElement,
  confirm_text: Option<SharedString>,
  cancel_text: Option<SharedString>,
  confirm_variant: Option<ButtonVariant>,
  cancel_variant: Option<ButtonVariant>,
  content_placement: ConfirmDialogContentPlacement,
  on_confirm: Option<Rc<ConfirmDialogHandler>>,
  on_cancel: Option<Rc<ConfirmDialogHandler>>,
}

impl ConfirmDialog {
  pub fn new(title: impl Into<SharedString>, message: impl IntoElement) -> Self {
    Self {
      title: title.into(),
      message: message.into_any_element(),
      confirm_text: None,
      cancel_text: None,
      confirm_variant: None,
      cancel_variant: None,
      content_placement: ConfirmDialogContentPlacement::Description,
      on_confirm: None,
      on_cancel: None,
    }
  }

  pub fn confirm_text(mut self, text: impl Into<SharedString>) -> Self {
    self.confirm_text = Some(text.into());
    self
  }

  pub fn cancel_text(mut self, text: impl Into<SharedString>) -> Self {
    self.cancel_text = Some(text.into());
    self
  }

  pub fn confirm_variant(mut self, variant: ButtonVariant) -> Self {
    self.confirm_variant = Some(variant);
    self
  }

  pub fn cancel_variant(mut self, variant: ButtonVariant) -> Self {
    self.cancel_variant = Some(variant);
    self
  }

  pub fn destructive(self) -> Self {
    self.confirm_variant(ButtonVariant::Danger)
  }

  pub fn content_as_body(mut self) -> Self {
    self.content_placement = ConfirmDialogContentPlacement::Body;
    self
  }

  pub fn on_confirm<F>(mut self, on_confirm: F) -> Self
  where
    F: Fn(&ClickEvent, &mut Window, &mut App) -> bool + 'static,
  {
    self.on_confirm = Some(Rc::new(on_confirm));
    self
  }

  pub fn on_cancel<F>(mut self, on_cancel: F) -> Self
  where
    F: Fn(&ClickEvent, &mut Window, &mut App) -> bool + 'static,
  {
    self.on_cancel = Some(Rc::new(on_cancel));
    self
  }

  fn resolved_props(&self) -> ResolvedConfirmDialogProps {
    ResolvedConfirmDialogProps {
      confirm_text: self.confirm_text.clone().unwrap_or_else(|| "OK".into()),
      cancel_text: self.cancel_text.clone().unwrap_or_else(|| "Cancel".into()),
      confirm_variant: self.confirm_variant.unwrap_or(ButtonVariant::Primary),
      cancel_variant: self.cancel_variant.unwrap_or_default(),
    }
  }

  pub fn build(self, alert: AlertDialog) -> AlertDialog {
    let props = self.resolved_props();
    let confirm_text = props.confirm_text.clone();
    let cancel_text = props.cancel_text.clone();
    let confirm_variant = props.confirm_variant;
    let cancel_variant = props.cancel_variant;
    let on_confirm = self.on_confirm.clone();
    let on_cancel = self.on_cancel.clone();

    let alert = alert
      .title(self.title)
      .close_button(true)
      .on_ok(move |event, window, cx| {
        if let Some(on_confirm) = on_confirm.as_ref() {
          on_confirm(event, window, cx)
        } else {
          true
        }
      })
      .on_cancel(move |event, window, cx| {
        if let Some(on_cancel) = on_cancel.as_ref() {
          on_cancel(event, window, cx)
        } else {
          true
        }
      })
      .footer(
        DialogFooter::new()
          .child(
            Button::new("cancel")
              .label(cancel_text)
              .with_variant(cancel_variant)
              .small()
              .on_click(|_, window, cx| window.dispatch_action(Box::new(Cancel), cx)),
          )
          .child(
            Button::new("ok")
              .label(confirm_text)
              .with_variant(confirm_variant)
              .small()
              .on_click(|_, window, cx| {
                window.dispatch_action(Box::new(Confirm { secondary: false }), cx)
              }),
          ),
      );

    match self.content_placement {
      ConfirmDialogContentPlacement::Description => alert.description(self.message),
      ConfirmDialogContentPlacement::Body => alert.child(self.message),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::ConfirmDialog;
  use super::ConfirmDialogContentPlacement;
  use gpui_component::button::ButtonVariant;

  #[test]
  fn resolved_props_default_to_ok_and_cancel() {
    let dialog = ConfirmDialog::new("Confirm", "Message");
    let props = dialog.resolved_props();

    assert_eq!(props.confirm_text.as_ref(), "OK");
    assert_eq!(props.cancel_text.as_ref(), "Cancel");
    assert_eq!(props.confirm_variant, ButtonVariant::Primary);
    assert_eq!(props.cancel_variant, ButtonVariant::default());
  }

  #[test]
  fn destructive_confirm_dialog_uses_danger_variant() {
    let dialog = ConfirmDialog::new("Confirm", "Message").destructive();
    let props = dialog.resolved_props();

    assert_eq!(props.confirm_variant, ButtonVariant::Danger);
  }

  #[test]
  fn content_defaults_to_description_and_can_render_as_body() {
    let dialog = ConfirmDialog::new("Confirm", "Message");
    assert_eq!(
      dialog.content_placement,
      ConfirmDialogContentPlacement::Description
    );

    let dialog = ConfirmDialog::new("Confirm", "Message").content_as_body();
    assert_eq!(
      dialog.content_placement,
      ConfirmDialogContentPlacement::Body
    );
  }
}
