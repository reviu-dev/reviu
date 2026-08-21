use std::rc::Rc;

use gpui::{
  AnyElement, App, DefiniteLength, Entity, InteractiveElement, IntoElement, ParentElement,
  RenderOnce, StatefulInteractiveElement, StyleRefinement, Styled, Window, div,
  prelude::FluentBuilder as _,
};
use gpui_component::{
  ActiveTheme as _,
  input::TextareaState,
  tab::{Tab, TabBar},
  v_flex,
};

use crate::GithubEmojiInput;

/// Height of the underline-style tab bar rendered above the input (default Size::Md).
pub const MARKDOWN_COMPOSER_TAB_BAR_HEIGHT_PX: f32 = 36.0;
/// Vertical gap between the tab bar and the body (matches `mt_2`).
pub const MARKDOWN_COMPOSER_TAB_BAR_GAP_PX: f32 = 8.0;
/// Extra vertical space the composer chrome takes on top of the input height.
pub const MARKDOWN_COMPOSER_CHROME_HEIGHT_PX: f32 =
  MARKDOWN_COMPOSER_TAB_BAR_HEIGHT_PX + MARKDOWN_COMPOSER_TAB_BAR_GAP_PX;

pub type MarkdownComposerPreviewFn = dyn Fn(&str, &mut Window, &mut App) -> AnyElement;
pub type MarkdownComposerToggleFn = dyn Fn(&mut Window, &mut App);

#[derive(IntoElement)]
pub struct MarkdownComposer {
  input_state: Entity<TextareaState>,
  style: StyleRefinement,
  height: Option<DefiniteLength>,
  disabled: bool,
  appearance: bool,
  preview_open: bool,
  on_toggle_preview: Option<Rc<MarkdownComposerToggleFn>>,
  render_preview: Option<Rc<MarkdownComposerPreviewFn>>,
}

impl MarkdownComposer {
  pub fn new(input_state: &Entity<TextareaState>) -> Self {
    Self {
      input_state: input_state.clone(),
      style: StyleRefinement::default(),
      height: None,
      disabled: false,
      appearance: true,
      preview_open: false,
      on_toggle_preview: None,
      render_preview: None,
    }
  }

  pub fn h(mut self, height: impl Into<DefiniteLength>) -> Self {
    self.height = Some(height.into());
    self
  }

  pub fn disabled(mut self, disabled: bool) -> Self {
    self.disabled = disabled;
    self
  }

  /// Drops the input's own border so it can sit inside another frame.
  pub fn appearance(mut self, appearance: bool) -> Self {
    self.appearance = appearance;
    self
  }

  pub fn preview_open(mut self, preview_open: bool) -> Self {
    self.preview_open = preview_open;
    self
  }

  pub fn on_toggle_preview<F>(mut self, handler: F) -> Self
  where
    F: Fn(&mut Window, &mut App) + 'static,
  {
    self.on_toggle_preview = Some(Rc::new(handler));
    self
  }

  pub fn preview<F>(mut self, render: F) -> Self
  where
    F: Fn(&str, &mut Window, &mut App) -> AnyElement + 'static,
  {
    self.render_preview = Some(Rc::new(render));
    self
  }

  fn render_input(&self) -> GithubEmojiInput {
    let mut input = GithubEmojiInput::new(&self.input_state)
      .disabled(self.disabled)
      .appearance(self.appearance);
    if let Some(height) = self.height {
      input = input.h(height);
    }
    input
  }
}

impl Styled for MarkdownComposer {
  fn style(&mut self) -> &mut StyleRefinement {
    &mut self.style
  }
}

impl RenderOnce for MarkdownComposer {
  fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
    let theme = cx.theme().clone();
    let input_id = self.input_state.entity_id();

    let Some(render_preview) = self.render_preview.clone() else {
      let mut container = div().w_full().child(self.render_input());
      *container.style() = self.style;
      return container.into_any_element();
    };

    let preview_open = self.preview_open;
    let body: AnyElement = if preview_open {
      let text = self.input_state.read(cx).value().to_string();
      let preview_id = ("markdown-composer-preview", input_id);
      let wrapper = div()
        .id(preview_id)
        .w_full()
        .when_some(self.height, |this, height| {
          this.h(height).overflow_y_scroll()
        });
      if text.trim().is_empty() {
        wrapper
          .text_xs()
          .text_color(theme.muted_foreground)
          .child("Nothing to preview")
          .into_any_element()
      } else {
        wrapper
          .child(render_preview(text.as_str(), window, cx))
          .into_any_element()
      }
    } else {
      self.render_input().into_any_element()
    };

    let mut tab_bar = TabBar::new(("markdown-composer-tabs", input_id))
      .underline()
      .selected_index(if preview_open { 1 } else { 0 })
      .child(Tab::new().label("Write"))
      .child(Tab::new().label("Preview"));

    if let Some(on_toggle) = self.on_toggle_preview {
      tab_bar = tab_bar.on_click(move |ix: &usize, window, cx| {
        let next_open = *ix == 1;
        if next_open != preview_open {
          on_toggle(window, cx);
        }
      });
    }

    let mut container = v_flex()
      .w_full()
      .child(tab_bar)
      .child(div().mt_2().child(body));
    *container.style() = self.style;
    container.into_any_element()
  }
}
