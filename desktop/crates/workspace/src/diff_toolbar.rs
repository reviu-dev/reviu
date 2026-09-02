//! The row above a diff: which file, where you are in it, and how it is shown.
//! The shell's diff and the pull request's Changes tab render the same one, with
//! their own buttons on either side of it.

use std::rc::Rc;

use editor::DiffViewMode;
use gpui::{
  AnyElement, App, InteractiveElement as _, IntoElement, ParentElement, Styled, Window, div,
  prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Disableable as _, IconName, Sizable as _,
  button::{Button, ButtonVariants as _},
  h_flex,
};
use ui::UiIconName;

pub(crate) const DIFF_TOOLBAR_HEIGHT: f32 = 40.0;

/// The two consumers are different entities, so each hands over its own closure
/// rather than the toolbar reaching for a view it cannot know.
type ToolbarAction = Rc<dyn Fn(&mut Window, &mut App)>;

/// Stepping through the conflicts or the changes of the open file.
pub(crate) struct NavigationControl {
  pub active_index: usize,
  pub total: usize,
  pub enabled: bool,
  pub label: &'static str,
  pub previous_tooltip: &'static str,
  pub next_tooltip: &'static str,
  /// Each host keeps the name its own tests and the driver already use.
  pub counter_debug_selector: &'static str,
  pub on_previous: ToolbarAction,
  pub on_next: ToolbarAction,
}

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
  before_title: Vec<AnyElement>,
  title: Option<AnyElement>,
  before_toggles: Vec<AnyElement>,
  navigation: Option<NavigationControl>,
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
      before_title: Vec::new(),
      title: None,
      before_toggles: Vec::new(),
      navigation: None,
      preview: None,
      whitespace: None,
      split: None,
      after_toggles: Vec::new(),
      filled: false,
    }
  }

  /// A button of the host, before the file name.
  pub(crate) fn before_title(mut self, element: AnyElement) -> Self {
    self.before_title.push(element);
    self
  }

  /// A button of the host, in the control group ahead of the shared toggles.
  pub(crate) fn before_toggles(mut self, element: AnyElement) -> Self {
    self.before_toggles.push(element);
    self
  }

  pub(crate) fn title(mut self, title: AnyElement) -> Self {
    self.title = Some(title);
    self
  }

  pub(crate) fn navigation(mut self, navigation: NavigationControl) -> Self {
    self.navigation = Some(navigation);
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
    let mut controls = h_flex().flex_shrink_0().items_center().gap_2();

    for element in self.before_toggles {
      controls = controls.child(element);
    }
    if let Some(navigation) = self.navigation {
      controls = controls.child(render_navigation(self.id_prefix, navigation, cx));
    }
    if let Some(preview) = self.preview {
      controls = controls.child(render_preview(self.id_prefix, preview));
    }
    if let Some(whitespace) = self.whitespace {
      controls = controls.child(render_whitespace(self.id_prefix, whitespace));
    }
    if let Some(split) = self.split {
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
      .px_3()
      .border_b_1()
      .border_color(theme.border);
    if self.filled {
      row = row.bg(theme.sidebar);
    }
    for element in self.before_title {
      row = row.child(element);
    }
    if let Some(title) = self.title {
      row = row.child(title);
    }
    row.child(controls).into_any_element()
  }
}

fn render_navigation(
  id_prefix: &'static str,
  navigation: NavigationControl,
  cx: &App,
) -> AnyElement {
  let theme = cx.theme().clone();
  let on_previous = navigation.on_previous.clone();
  let on_next = navigation.on_next.clone();
  let counter_selector = navigation.counter_debug_selector;
  let show_buttons = navigation_buttons_visible(&navigation);

  h_flex()
    .items_center()
    .gap_1()
    .when(show_buttons, |this| {
      this.child(
        Button::new(format!("{id_prefix}-navigate-previous"))
          .icon(IconName::ArrowUp)
          .xsmall()
          .ghost()
          .compact()
          .tooltip(navigation.previous_tooltip)
          .on_click(move |_, window, cx| on_previous(window, cx)),
      )
    })
    .child(
      div()
        .debug_selector(move || counter_selector.to_string())
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(navigation_counter_text(
          navigation.label,
          navigation.active_index,
          navigation.total,
        )),
    )
    .when(show_buttons, |this| {
      this.child(
        Button::new(format!("{id_prefix}-navigate-next"))
          .icon(IconName::ArrowDown)
          .xsmall()
          .ghost()
          .compact()
          .tooltip(navigation.next_tooltip)
          .on_click(move |_, window, cx| on_next(window, cx)),
      )
    })
    .into_any_element()
}

fn navigation_counter_text(label: &str, active_index: usize, total: usize) -> String {
  format!("{label} {}/{}", active_index + 1, total)
}

fn navigation_buttons_visible(navigation: &NavigationControl) -> bool {
  navigation.enabled && navigation.total > 1
}

fn render_whitespace(id_prefix: &'static str, whitespace: ToggleControl) -> AnyElement {
  let hidden = whitespace.active;
  let on_toggle = whitespace.on_toggle.clone();
  let selector = whitespace.debug_selector;

  Button::new(format!("{id_prefix}-whitespace"))
    .debug_selector(move || selector.to_string())
    .label("Whitespace")
    .icon(if hidden {
      IconName::Eye
    } else {
      IconName::EyeOff
    })
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

fn render_split(id_prefix: &'static str, split: SplitControl) -> AnyElement {
  let on_toggle = split.on_toggle.clone();
  let selector = split.debug_selector;
  // A disabled toggle offers the mode it cannot reach, not the one it is in.
  let (label, icon) = if split.disabled || split.mode == DiffViewMode::Inline {
    ("Split", IconName::PanelLeft)
  } else {
    ("Inline", IconName::PanelLeftClose)
  };

  Button::new(format!("{id_prefix}-split"))
    .debug_selector(move || selector.to_string())
    .label(label)
    .icon(icon)
    .xsmall()
    .ghost()
    .disabled(split.disabled)
    .tooltip("Toggle inline and split diff (cmd-/)")
    .on_click(move |_, window, cx| on_toggle(window, cx))
    .into_any_element()
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
    .tooltip("Show the rendered file")
    .on_click(move |_, window, cx| on_toggle(window, cx))
    .into_any_element()
}

#[cfg(test)]
mod tests {
  use super::*;

  fn test_navigation_control(enabled: bool, total: usize) -> NavigationControl {
    NavigationControl {
      active_index: 0,
      total,
      enabled,
      label: "Conflict",
      previous_tooltip: "Previous conflict",
      next_tooltip: "Next conflict",
      counter_debug_selector: "test-counter",
      on_previous: Rc::new(|_, _| {}),
      on_next: Rc::new(|_, _| {}),
    }
  }

  #[test]
  fn navigation_buttons_are_hidden_when_the_counter_cannot_move() {
    assert!(!navigation_buttons_visible(&test_navigation_control(
      false, 1
    )));
    assert!(!navigation_buttons_visible(&test_navigation_control(
      true, 1
    )));
    assert!(navigation_buttons_visible(&test_navigation_control(
      true, 2
    )));
  }

  #[test]
  fn navigation_counter_names_the_walked_annotation() {
    assert_eq!(navigation_counter_text("Hunk", 1, 5), "Hunk 2/5");
    assert_eq!(navigation_counter_text("Conflict", 0, 1), "Conflict 1/1");
  }
}
