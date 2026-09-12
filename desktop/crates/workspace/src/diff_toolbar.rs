//! The row above a diff: which file, where you are in it, and how it is shown.
//! The shell's diff and the pull request's Changes tab render the same one, with
//! host-specific controls after the shared toggles.

use std::rc::Rc;

use editor::DiffViewMode;
use gpui::{
  AnyElement, App, InteractiveElement as _, IntoElement, ParentElement, Styled, Window, prelude::*,
  px,
};
use gpui_component::{
  ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _,
  button::{Button, ButtonGroup, ButtonVariants as _},
  h_flex,
};
use ui::UiIconName;

pub(crate) const DIFF_TOOLBAR_HEIGHT: f32 = 36.0;

/// The two consumers are different entities, so each hands over its own closure
/// rather than the toolbar reaching for a view it cannot know.
type ToolbarAction = Rc<dyn Fn(&mut Window, &mut App)>;

pub(crate) struct ToggleControl {
  pub active: bool,
  pub disabled: bool,
  pub debug_selector: &'static str,
  pub on_toggle: ToolbarAction,
}

pub(crate) struct SplitControl {
  pub mode: DiffViewMode,
  /// A file with only one side to show cannot be split.
  pub disabled: bool,
  pub debug_selector: &'static str,
  pub on_toggle: ToolbarAction,
}

/// Absent controls are absent buttons: a binary preview has no whitespace to
/// hide, a file without changes has nothing to step through.
pub(crate) struct DiffToolbar {
  /// Element ids are namespaced per host: both toolbars can be mounted at once.
  id_prefix: &'static str,
  title: Option<AnyElement>,
  preview: Option<ToggleControl>,
  whitespace: Option<ToggleControl>,
  split: Option<SplitControl>,
  after_toggles: Vec<AnyElement>,
  filled: bool,
}

impl DiffToolbar {
  pub(crate) fn new(id_prefix: &'static str) -> Self {
    Self {
      id_prefix,
      title: None,
      preview: None,
      whitespace: None,
      split: None,
      after_toggles: Vec::new(),
      filled: false,
    }
  }

  pub(crate) fn title(mut self, title: AnyElement) -> Self {
    self.title = Some(title);
    self
  }

  pub(crate) fn whitespace(mut self, whitespace: ToggleControl) -> Self {
    self.whitespace = Some(whitespace);
    self
  }

  pub(crate) fn split(mut self, split: SplitControl) -> Self {
    self.split = Some(split);
    self
  }

  pub(crate) fn preview(mut self, preview: ToggleControl) -> Self {
    self.preview = Some(preview);
    self
  }

  /// A button of the host, after the shared toggles.
  pub(crate) fn after_toggles(mut self, element: AnyElement) -> Self {
    self.after_toggles.push(element);
    self
  }

  pub(crate) fn render(self, cx: &App) -> AnyElement {
    let theme = cx.theme().clone();
    let mut controls = h_flex().flex_shrink_0().items_center().gap_2().text_xs();

    if let Some(preview) = self.preview {
      controls = controls.child(render_preview(self.id_prefix, preview));
    }
    if let Some(whitespace) = self.whitespace {
      controls = controls.child(render_whitespace(self.id_prefix, whitespace));
    }
    if let Some(split) = self.split
      && !split.disabled
    {
      controls = controls.child(render_split(self.id_prefix, split));
    }
    for element in self.after_toggles {
      controls = controls.child(element);
    }

    let mut row = h_flex()
      .h(px(DIFF_TOOLBAR_HEIGHT))
      .min_h(px(DIFF_TOOLBAR_HEIGHT))
      .max_h(px(DIFF_TOOLBAR_HEIGHT))
      .flex_shrink_0()
      .items_center()
      .gap_3()
      .text_xs()
      .px_3()
      .border_b_1()
      .border_color(theme.border);
    if self.filled {
      row = row.bg(theme.sidebar);
    }
    if let Some(title) = self.title {
      row = row.child(title);
    }
    row.child(controls).into_any_element()
  }
}

fn render_whitespace(id_prefix: &'static str, whitespace: ToggleControl) -> AnyElement {
  let hidden = whitespace.active;
  let on_toggle = whitespace.on_toggle.clone();
  let selector = whitespace.debug_selector;

  Button::new(format!("{id_prefix}-whitespace"))
    .debug_selector(move || selector.to_string())
    .icon(whitespace_icon())
    .selected(hidden)
    .xsmall()
    .ghost()
    .disabled(whitespace.disabled)
    .tooltip(if hidden {
      "Show whitespace changes"
    } else {
      "Hide whitespace changes"
    })
    .on_click(move |_, window, cx| on_toggle(window, cx))
    .into_any_element()
}

fn whitespace_icon() -> UiIconName {
  UiIconName::Pilcrow
}

fn render_split(id_prefix: &'static str, split: SplitControl) -> AnyElement {
  let inline_toggle = split.on_toggle.clone();
  let split_toggle = split.on_toggle.clone();
  let mode = split.mode;
  let selector = split.debug_selector;

  ButtonGroup::new(format!("{id_prefix}-diff-view"))
    .outline()
    .compact()
    .xsmall()
    .disabled(split.disabled)
    .child(
      Button::new(format!("{id_prefix}-diff-view-inline"))
        .icon(UiIconName::DiffInline)
        .selected(mode == DiffViewMode::Inline)
        .tooltip("Show inline diff (cmd-/)")
        .when_some(
          split_button_debug_selector(mode, DiffViewMode::Inline, selector),
          |this, selector| this.debug_selector(move || selector.to_string()),
        )
        .on_click(move |_, window, cx| {
          if mode != DiffViewMode::Inline {
            inline_toggle(window, cx);
          }
        }),
    )
    .child(
      Button::new(format!("{id_prefix}-diff-view-split"))
        .icon(UiIconName::DiffSplit)
        .selected(mode == DiffViewMode::Split)
        .tooltip("Show split diff (cmd-/)")
        .when_some(
          split_button_debug_selector(mode, DiffViewMode::Split, selector),
          |this, selector| this.debug_selector(move || selector.to_string()),
        )
        .on_click(move |_, window, cx| {
          if mode != DiffViewMode::Split {
            split_toggle(window, cx);
          }
        }),
    )
    .into_any_element()
}

fn split_button_debug_selector(
  current_mode: DiffViewMode,
  button_mode: DiffViewMode,
  selector: &'static str,
) -> Option<&'static str> {
  (current_mode != button_mode).then_some(selector)
}

fn render_preview(id_prefix: &'static str, preview: ToggleControl) -> AnyElement {
  let on_toggle = preview.on_toggle.clone();
  let selector = preview.debug_selector;
  // The button names where it takes you, not where you are.
  let (label, icon) = if preview.active {
    ("Code", UiIconName::FileCode)
  } else {
    ("Preview", UiIconName::Eye)
  };

  Button::new(format!("{id_prefix}-preview"))
    .debug_selector(move || selector.to_string())
    .label(label)
    .icon(icon)
    .xsmall()
    .ghost()
    .disabled(preview.disabled)
    .on_click(move |_, window, cx| on_toggle(window, cx))
    .into_any_element()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn whitespace_icon_uses_a_whitespace_symbol() {
    use gpui_component::IconNamed as _;

    assert_eq!(whitespace_icon().path().as_ref(), "icons/pilcrow.svg");
  }
}
