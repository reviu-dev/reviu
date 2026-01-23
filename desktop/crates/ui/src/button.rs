use gpui::{Div, Rgba, SharedString, div, prelude::*};

#[derive(Clone, Copy)]
pub struct ButtonColors {
  pub text: Rgba,
  pub disabled_text: Rgba,
}

impl ButtonColors {
  pub fn new(text: Rgba, disabled_text: Rgba) -> Self {
    Self { text, disabled_text }
  }
}

pub fn button(
  label: impl Into<SharedString>,
  colors: ButtonColors,
  disabled: bool,
) -> Div {
  let mut element = div()
    .px_3()
    .py_1()
    .flex()
    .items_center()
    .justify_center()
    .text_sm()
    .child(label.into());

  if disabled {
    element = element.text_color(colors.disabled_text);
  } else {
    element = element.text_color(colors.text).cursor_pointer();
  }

  element
}
