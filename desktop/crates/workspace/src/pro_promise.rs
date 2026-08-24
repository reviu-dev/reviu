//! The one place that says what Reviu Pro brings. Surfaces that cannot do their
//! job without it show this instead of vanishing.

use std::collections::HashSet;

use gpui::{AnyElement, App, Global, SharedString, div, prelude::*};
use gpui_component::{ActiveTheme as _, Icon, IconName, Sizable as _, v_flex};
use ui::{Button, ButtonVariants as _, UiIconName};

use crate::analytics;
use crate::auth_state::GithubAccessState;

/// Where the promise is being made, which is what tells us later which surface
/// converts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProPromiseSurface {
  PullRequestPanel,
  Inbox,
}

impl ProPromiseSurface {
  fn source(self) -> &'static str {
    match self {
      Self::PullRequestPanel => "pull_request_panel",
      Self::Inbox => "inbox",
    }
  }
}

/// What is missing, and what closes the gap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProPromiseStep {
  SignIn,
  Subscribe,
}

pub(crate) struct ProPromiseCopy {
  pub headline: &'static str,
  pub body: &'static str,
  pub action: &'static str,
  pub step: ProPromiseStep,
}

/// Nothing to promise once GitHub works: the surface does its own job then. The
/// copy names what *this* surface would show, not the whole integration.
pub(crate) fn pro_promise_copy(
  surface: ProPromiseSurface,
  state: GithubAccessState,
) -> Option<ProPromiseCopy> {
  match (surface, state) {
    (_, GithubAccessState::Available) => None,
    (ProPromiseSurface::PullRequestPanel, GithubAccessState::NeedsSignIn) => Some(ProPromiseCopy {
      headline: "Review pull requests in Reviu",
      body: "Sign in with GitHub to bring pull requests, reviews and notifications into the app.",
      action: "Sign in with GitHub",
      step: ProPromiseStep::SignIn,
    }),
    (ProPromiseSurface::PullRequestPanel, GithubAccessState::NeedsSubscription) => {
      Some(ProPromiseCopy {
        headline: "Review pull requests in Reviu",
        body: "Reviu Pro brings GitHub pull requests, reviews and notifications into the app. 14-day free trial.",
        action: "See Reviu Pro",
        step: ProPromiseStep::Subscribe,
      })
    }
    (ProPromiseSurface::Inbox, GithubAccessState::NeedsSignIn) => Some(ProPromiseCopy {
      headline: "Your GitHub notifications, here",
      body: "Sign in with GitHub to follow reviews and mentions without leaving Reviu.",
      action: "Sign in with GitHub",
      step: ProPromiseStep::SignIn,
    }),
    (ProPromiseSurface::Inbox, GithubAccessState::NeedsSubscription) => Some(ProPromiseCopy {
      headline: "Your GitHub notifications, here",
      body: "Reviu Pro brings reviews and mentions into this inbox. 14-day free trial.",
      action: "See Reviu Pro",
      step: ProPromiseStep::Subscribe,
    }),
  }
}

/// One impression per surface per session: a render runs on every frame, and a
/// sighting is not a stream of events.
#[derive(Default)]
struct ReportedProPromiseImpressions(HashSet<&'static str>);

impl Global for ReportedProPromiseImpressions {}

/// True the first time a surface is seen this session, false afterwards.
fn take_impression(surface: ProPromiseSurface, cx: &mut App) -> bool {
  cx.default_global::<ReportedProPromiseImpressions>()
    .0
    .insert(surface.source())
}

fn report_impression(surface: ProPromiseSurface, cx: &mut App) {
  if !take_impression(surface, cx) {
    return;
  }

  analytics::track_with(
    cx,
    "pro_teaser_shown",
    Some(serde_json::json!({ "source": surface.source() })),
  );
}

fn take_step(step: ProPromiseStep, surface: ProPromiseSurface, cx: &mut App) {
  analytics::track_with(
    cx,
    "pro_teaser_clicked",
    Some(serde_json::json!({ "source": surface.source() })),
  );
  match step {
    ProPromiseStep::SignIn => crate::auth_flow::start_github_sign_in(cx, surface.source()),
    ProPromiseStep::Subscribe => {
      crate::workspace_window::WorkspaceWindow::with_window(cx, |window, cx| {
        crate::billing_dialog::open_billing_dialog(window, cx);
      })
    }
  }
}

pub(crate) fn render_pro_promise(
  surface: ProPromiseSurface,
  state: GithubAccessState,
  cx: &mut App,
) -> Option<AnyElement> {
  let copy = pro_promise_copy(surface, state)?;
  report_impression(surface, cx);
  let theme = cx.theme().clone();
  let step = copy.step;
  let button_id: SharedString = format!("pro-promise-{}", surface.source()).into();
  let button = Button::new(button_id)
    .debug_selector(move || format!("pro-promise-{}", surface.source()))
    .small()
    .primary()
    .when(step == ProPromiseStep::SignIn, |this| {
      this.icon(IconName::Github)
    })
    .label(copy.action)
    .on_click(move |_, _, cx| take_step(step, surface, cx));

  let element = match surface {
    // The panel is empty behind it: the promise takes the room.
    ProPromiseSurface::PullRequestPanel => v_flex()
      .flex_1()
      .items_center()
      .justify_center()
      .gap_2()
      .px_4()
      .child(
        Icon::new(UiIconName::GitPullRequestArrow)
          .size_4()
          .text_color(theme.muted_foreground),
      )
      .child(
        div()
          .text_sm()
          .text_center()
          .text_color(theme.foreground)
          .child(copy.headline),
      )
      .child(
        div()
          .text_xs()
          .text_center()
          .text_color(theme.muted_foreground)
          .child(copy.body),
      )
      .child(div().mt_1().child(button))
      .into_any_element(),
    // The body of the inbox section, which already carries the header and the
    // rule above it: no chrome of its own, or the borders double up.
    ProPromiseSurface::Inbox => v_flex()
      .w_full()
      .gap_1()
      .px_3()
      .py_2()
      .child(
        div()
          .text_xs()
          .text_color(theme.foreground)
          .child(copy.headline),
      )
      .child(
        div()
          .text_xs()
          .text_color(theme.muted_foreground)
          .child(copy.body),
      )
      .child(div().mt_1().child(button))
      .into_any_element(),
  };
  Some(element)
}

#[cfg(test)]
mod tests {
  use super::*;
  use gpui::TestAppContext;

  const SURFACES: [ProPromiseSurface; 2] = [
    ProPromiseSurface::PullRequestPanel,
    ProPromiseSurface::Inbox,
  ];

  #[test]
  fn a_working_github_has_nothing_to_promise() {
    for surface in SURFACES {
      assert!(pro_promise_copy(surface, GithubAccessState::Available).is_none());
    }
  }

  #[test]
  fn each_missing_piece_asks_for_itself() {
    for surface in SURFACES {
      let sign_in = pro_promise_copy(surface, GithubAccessState::NeedsSignIn).expect("copy");
      assert_eq!(sign_in.step, ProPromiseStep::SignIn);
      assert_eq!(sign_in.action, "Sign in with GitHub");

      let subscribe =
        pro_promise_copy(surface, GithubAccessState::NeedsSubscription).expect("copy");
      assert_eq!(subscribe.step, ProPromiseStep::Subscribe);
      // Someone already signed in is asked to subscribe, not to sign in again.
      assert!(subscribe.body.contains("free trial"));
    }
  }

  #[test]
  fn each_surface_promises_what_it_would_itself_show() {
    // The inbox slot said "Review pull requests in Reviu", which is the panel's
    // job, not its own.
    for state in [
      GithubAccessState::NeedsSignIn,
      GithubAccessState::NeedsSubscription,
    ] {
      let panel = pro_promise_copy(ProPromiseSurface::PullRequestPanel, state).expect("copy");
      assert!(panel.headline.contains("pull request"));

      let inbox = pro_promise_copy(ProPromiseSurface::Inbox, state).expect("copy");
      assert!(inbox.headline.contains("notifications"));
      assert!(!inbox.headline.contains("pull request"));
    }
  }

  #[gpui::test]
  fn a_surface_is_reported_once_however_often_it_is_rendered(cx: &mut TestAppContext) {
    cx.update(|cx| {
      assert!(take_impression(ProPromiseSurface::Inbox, cx));
      assert!(
        !take_impression(ProPromiseSurface::Inbox, cx),
        "a render runs every frame; the sighting is still one"
      );

      assert!(
        take_impression(ProPromiseSurface::PullRequestPanel, cx),
        "each surface is counted on its own, that is the point of the source"
      );
    });
  }

  #[gpui::test]
  fn a_working_github_is_never_counted_as_an_impression(cx: &mut TestAppContext) {
    cx.update(|cx| {
      assert!(
        render_pro_promise(ProPromiseSurface::Inbox, GithubAccessState::Available, cx).is_none()
      );
      assert!(
        take_impression(ProPromiseSurface::Inbox, cx),
        "nothing was promised, so nothing was seen"
      );
    });
  }

  #[test]
  fn every_surface_names_itself_for_the_analytics() {
    assert_eq!(
      ProPromiseSurface::PullRequestPanel.source(),
      "pull_request_panel"
    );
    assert_eq!(ProPromiseSurface::Inbox.source(), "inbox");
  }
}
