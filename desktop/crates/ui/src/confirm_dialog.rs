use std::rc::Rc;

use gpui::{AnyElement, App, ClickEvent, IntoElement, SharedString, Window};
use gpui_component::button::ButtonVariant;
use gpui_component::dialog::{AlertDialog, DialogButtonProps};

type ConfirmDialogHandler = dyn Fn(&ClickEvent, &mut Window, &mut App) -> bool;

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

    let mut props = DialogButtonProps::default()
      .show_cancel(true)
      .ok_text(props.confirm_text)
      .cancel_text(props.cancel_text)
      .ok_variant(props.confirm_variant)
      .cancel_variant(props.cancel_variant);

    if let Some(on_confirm) = self.on_confirm {
      props = props.on_ok(move |event, window, cx| on_confirm(event, window, cx));
    }
    if let Some(on_cancel) = self.on_cancel {
      props = props.on_cancel(move |event, window, cx| on_cancel(event, window, cx));
    }

    alert
      .title(self.title)
      .description(self.message)
      .close_button(true)
      .overlay_closable(true)
      .button_props(props)
  }
}

#[cfg(test)]
mod tests {
  use super::ConfirmDialog;
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
}
