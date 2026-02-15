use gpui::{AnyElement, App, ClickEvent, IntoElement, ParentElement, SharedString, Window};
use gpui_component::button::ButtonVariant;
use gpui_component::dialog::{Dialog, DialogButtonProps};

type ConfirmDialogHandler = dyn Fn(&ClickEvent, &mut Window, &mut App) -> bool;

pub struct ConfirmDialog {
  title: SharedString,
  message: AnyElement,
  confirm_text: Option<SharedString>,
  cancel_text: Option<SharedString>,
  confirm_variant: Option<ButtonVariant>,
  cancel_variant: Option<ButtonVariant>,
  on_confirm: Option<Box<ConfirmDialogHandler>>,
  on_cancel: Option<Box<ConfirmDialogHandler>>,
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
    self.on_confirm = Some(Box::new(on_confirm));
    self
  }

  pub fn on_cancel<F>(mut self, on_cancel: F) -> Self
  where
    F: Fn(&ClickEvent, &mut Window, &mut App) -> bool + 'static,
  {
    self.on_cancel = Some(Box::new(on_cancel));
    self
  }

  pub fn build(self, dialog: Dialog) -> Dialog {
    let mut props = DialogButtonProps::default();
    if let Some(text) = self.confirm_text {
      props = props.ok_text(text);
    }
    if let Some(text) = self.cancel_text {
      props = props.cancel_text(text);
    }
    if let Some(variant) = self.confirm_variant {
      props = props.ok_variant(variant);
    }
    if let Some(variant) = self.cancel_variant {
      props = props.cancel_variant(variant);
    }

    let mut dialog = dialog
      .title(self.title)
      .confirm()
      .child(self.message)
      .overlay_closable(true)
      .button_props(props);

    if let Some(on_confirm) = self.on_confirm {
      dialog = dialog.on_ok(on_confirm);
    }
    if let Some(on_cancel) = self.on_cancel {
      dialog = dialog.on_cancel(on_cancel);
    }

    dialog
  }
}
