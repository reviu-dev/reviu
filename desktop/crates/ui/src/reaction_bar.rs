use std::rc::Rc;

use gpui::{
  App, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce, SharedString,
  StyleRefinement, Styled, Window, div, prelude::FluentBuilder as _,
};
use gpui_component::{
  ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, StyledExt as _,
  button::{Button, ButtonVariants as _},
  h_flex,
  popover::Popover,
  v_flex,
};

use crate::{StatusThemeExt as _, UiIconName};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReactionOption<T> {
  pub value: T,
  pub emoji: SharedString,
  pub label: SharedString,
}

impl<T> ReactionOption<T> {
  pub fn new(value: T, emoji: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
    Self {
      value,
      emoji: emoji.into(),
      label: label.into(),
    }
  }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReactionGroup<T> {
  pub value: T,
  pub emoji: SharedString,
  pub label: SharedString,
  pub count: u64,
  pub viewer_has_reacted: bool,
}

impl<T> ReactionGroup<T> {
  pub fn new(
    value: T,
    emoji: impl Into<SharedString>,
    label: impl Into<SharedString>,
    count: u64,
    viewer_has_reacted: bool,
  ) -> Self {
    Self {
      value,
      emoji: emoji.into(),
      label: label.into(),
      count,
      viewer_has_reacted,
    }
  }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReactionToggle<T> {
  pub subject_id: SharedString,
  pub value: T,
  pub viewer_has_reacted: bool,
}

type ReactionToggleHandler<T> = Rc<dyn Fn(ReactionToggle<T>, &mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub struct ReactionBar<T: Clone + PartialEq + 'static> {
  id: ElementId,
  id_prefix: String,
  subject_id: SharedString,
  options: Vec<ReactionOption<T>>,
  reactions: Vec<ReactionGroup<T>>,
  loading: bool,
  error: Option<SharedString>,
  on_toggle: Option<ReactionToggleHandler<T>>,
  style: StyleRefinement,
}

impl<T> ReactionBar<T>
where
  T: Clone + PartialEq + 'static,
{
  pub fn new(id: impl Into<SharedString>) -> Self {
    let id = id.into();

    Self {
      id: id.clone().into(),
      id_prefix: id.to_string(),
      subject_id: SharedString::default(),
      options: Vec::new(),
      reactions: Vec::new(),
      loading: false,
      error: None,
      on_toggle: None,
      style: StyleRefinement::default(),
    }
  }

  pub fn subject_id(mut self, subject_id: impl Into<SharedString>) -> Self {
    self.subject_id = subject_id.into();
    self
  }

  pub fn options(mut self, options: impl IntoIterator<Item = ReactionOption<T>>) -> Self {
    self.options = options.into_iter().collect();
    self
  }

  pub fn reactions(mut self, reactions: impl IntoIterator<Item = ReactionGroup<T>>) -> Self {
    self.reactions = reactions.into_iter().collect();
    self
  }

  pub fn loading(mut self, loading: bool) -> Self {
    self.loading = loading;
    self
  }

  pub fn error(mut self, error: Option<impl Into<SharedString>>) -> Self {
    self.error = error.map(Into::into);
    self
  }

  pub fn on_toggle(
    mut self,
    handler: impl Fn(ReactionToggle<T>, &mut Window, &mut App) + 'static,
  ) -> Self {
    self.on_toggle = Some(Rc::new(handler));
    self
  }
}

impl<T> Styled for ReactionBar<T>
where
  T: Clone + PartialEq + 'static,
{
  fn style(&mut self) -> &mut StyleRefinement {
    &mut self.style
  }
}

impl<T> RenderOnce for ReactionBar<T>
where
  T: Clone + PartialEq + 'static,
{
  fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
    let ReactionBar {
      id,
      id_prefix,
      subject_id,
      options,
      reactions,
      loading,
      error,
      on_toggle,
      style,
    } = self;

    if subject_id.as_ref().is_empty() {
      return div().into_any_element();
    }

    let theme = cx.theme().clone();
    let visible_reactions = options
      .iter()
      .filter_map(|option| {
        reactions
          .iter()
          .find(|reaction| reaction.value == option.value && reaction.count > 0)
          .cloned()
      })
      .collect::<Vec<_>>();

    h_flex()
      .id(id)
      .items_center()
      .gap_1()
      .flex_wrap()
      .refine_style(&style)
      .child(
        Popover::new(format!("{id_prefix}-add-reaction-popover"))
          .anchor(gpui::Corner::TopRight)
          .appearance(false)
          .trigger(
            Button::new(format!("{id_prefix}-add-reaction"))
              .xsmall()
              .compact()
              .ghost()
              .icon(UiIconName::SmilePlus)
              .tooltip("Add reaction")
              .disabled(loading || on_toggle.is_none()),
          )
          .content({
            let options = options.clone();
            let reactions = reactions.clone();
            let on_toggle = on_toggle.clone();
            let subject_id = subject_id.clone();
            let id_prefix = id_prefix.clone();
            move |_, _, cx| {
              let theme = cx.theme().clone();
              let popover = cx.entity().clone();
              let options = options.clone();
              let reactions = reactions.clone();
              let on_toggle = on_toggle.clone();
              let subject_id = subject_id.clone();
              let id_prefix = id_prefix.clone();

              div()
                .id(format!("{id_prefix}-reaction-picker"))
                .bg(theme.popover)
                .text_color(theme.popover_foreground)
                .border_1()
                .border_color(theme.border)
                .rounded(theme.radius)
                .shadow_lg()
                .p_1()
                .child(v_flex().gap_1().children(options.chunks(4).enumerate().map(
                  |(row_ix, row)| {
                    h_flex().gap_1().children(row.iter().enumerate().map({
                      let reactions = reactions.clone();
                      let on_toggle = on_toggle.clone();
                      let subject_id = subject_id.clone();
                      let id_prefix = id_prefix.clone();
                      let popover = popover.clone();
                      move |(option_ix, option)| {
                        let value = option.value.clone();
                        let viewer_has_reacted = reactions
                          .iter()
                          .find(|reaction| reaction.value == option.value)
                          .is_some_and(|reaction| reaction.viewer_has_reacted);
                        let on_toggle = on_toggle.clone();
                        let subject_id = subject_id.clone();
                        let popover = popover.clone();
                        Button::new(format!("{id_prefix}-picker-reaction-{row_ix}-{option_ix}"))
                          .small()
                          .compact()
                          .ghost()
                          .selected(viewer_has_reacted)
                          .disabled(loading || on_toggle.is_none())
                          .label(option.emoji.clone())
                          .tooltip(option.label.clone())
                          .on_click(move |_, window, cx| {
                            let _ = popover.update(cx, |popover, cx| {
                              popover.dismiss(window, cx);
                            });
                            if let Some(on_toggle) = on_toggle.as_ref() {
                              on_toggle(
                                ReactionToggle {
                                  subject_id: subject_id.clone(),
                                  value: value.clone(),
                                  viewer_has_reacted,
                                },
                                window,
                                cx,
                              );
                            }
                          })
                      }
                    }))
                  },
                )))
            }
          }),
      )
      .children(
        visible_reactions
          .into_iter()
          .enumerate()
          .map(|(ix, reaction)| {
            let on_toggle = on_toggle.clone();
            let subject_id = subject_id.clone();
            let value = reaction.value.clone();
            let viewer_has_reacted = reaction.viewer_has_reacted;

            Button::new(format!("{id_prefix}-reaction-{ix}"))
              .xsmall()
              .compact()
              .ghost()
              .selected(viewer_has_reacted)
              .disabled(loading || on_toggle.is_none())
              .label(format!("{} {}", reaction.emoji.as_ref(), reaction.count))
              .tooltip(reaction.label)
              .on_click(move |_, window, cx| {
                if let Some(on_toggle) = on_toggle.as_ref() {
                  on_toggle(
                    ReactionToggle {
                      subject_id: subject_id.clone(),
                      value: value.clone(),
                      viewer_has_reacted,
                    },
                    window,
                    cx,
                  );
                }
              })
          }),
      )
      .when_some(error, |this, error| {
        this.child(div().text_xs().text_color(theme.status_red()).child(error))
      })
      .into_any_element()
  }
}

#[cfg(test)]
mod tests {
  use super::{ReactionBar, ReactionGroup, ReactionOption};

  #[test]
  fn reaction_option_keeps_display_metadata() {
    let option = ReactionOption::new("THUMBS_UP", "👍", "Thumbs up");

    assert_eq!(option.value, "THUMBS_UP");
    assert_eq!(option.emoji.as_ref(), "👍");
    assert_eq!(option.label.as_ref(), "Thumbs up");
  }

  #[test]
  fn reaction_bar_tracks_builder_state() {
    let bar = ReactionBar::new("github-reactions")
      .subject_id("IC_kwDOExample")
      .options([ReactionOption::new("THUMBS_UP", "👍", "Thumbs up")])
      .reactions([ReactionGroup::new("THUMBS_UP", "👍", "Thumbs up", 2, true)])
      .loading(true)
      .error(Some("Failed"));

    assert_eq!(bar.id_prefix, "github-reactions");
    assert_eq!(bar.subject_id.as_ref(), "IC_kwDOExample");
    assert_eq!(bar.options.len(), 1);
    assert_eq!(bar.reactions.len(), 1);
    assert!(bar.loading);
    assert_eq!(
      bar.error.as_ref().map(|error| error.as_ref()),
      Some("Failed")
    );
  }
}
