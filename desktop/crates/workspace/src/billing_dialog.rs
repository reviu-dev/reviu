//! Reviu Pro over whatever you were doing: what it brings, what it costs, and
//! the state of your subscription.

use gpui::{
  AnyElement, App, Context, Global, Render, SharedString, WeakEntity, Window, div, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt,
  button::{Button, ButtonVariants as _},
  dialog::{DialogFooter, DialogHeader, DialogTitle},
  h_flex,
  spinner::Spinner,
  v_flex,
};
use time::OffsetDateTime;
use ui::{StatusTag, StatusThemeExt, UiIconName, WindowExt};

use crate::{
  api::{ApiClient, CustomerStateSubscription, CustomerStateSubscriptionStatus},
  auth_state::{AuthState, AuthStateStore},
  date_format::{format_long_date_opt, parse_rfc3339},
  pricing_copy::{
    PRO_ANNUAL_PERIOD, PRO_ANNUAL_PRICE, PRO_ANNUAL_SAVE_PERCENT, PRO_ANNUAL_SLUG, PRO_BENEFITS,
    PRO_MONTHLY_PERIOD, PRO_MONTHLY_PRICE, PRO_MONTHLY_SLUG, PRO_TRIAL,
  },
  workspace::WorkspaceApi,
};

/// Wide enough for the two price columns to sit side by side.
const BILLING_DIALOG_WIDTH: f32 = 480.0;

/// Names this surface in the checkout and sign-in funnels.
const ANALYTICS_SOURCE: &str = "billing_dialog";

const REFRESH_DEBUG_SELECTOR: &str = "billing-refresh";
const CLOSE_DEBUG_SELECTOR: &str = "billing-close";

pub fn open_billing_dialog(window: &mut Window, _cx: &mut App) {
  // Defer to next frame so the command palette dialog closes first
  window.on_next_frame(|window, cx| {
    open_billing_dialog_inner(window, cx);
  });
}

/// The dialog reads the auth state on every render, so the copy already up
/// shows the fresh subscription: a second one would only stack a duplicate the
/// user has to close twice.
struct OpenBillingDialog(Option<WeakEntity<BillingDialog>>);

impl Global for OpenBillingDialog {}

fn billing_dialog_is_open(window: &mut Window, cx: &mut App) -> bool {
  let tracked = cx
    .try_global::<OpenBillingDialog>()
    .and_then(|open| open.0.as_ref())
    .is_some_and(|billing| billing.upgrade().is_some());

  // A released dialog can outlive its entity for a frame, so the window has the
  // final say on whether anything is still on screen.
  tracked && window.has_active_dialog(cx)
}

fn open_billing_dialog_inner(window: &mut Window, cx: &mut App) {
  if billing_dialog_is_open(window, cx) {
    return;
  }

  let billing = cx.new(BillingDialog::new);
  cx.set_global(OpenBillingDialog(Some(billing.downgrade())));
  window.open_dialog(cx, move |dialog, _, _| {
    dialog
      .p_0()
      .w(px(BILLING_DIALOG_WIDTH))
      .close_button(false)
      .child(billing.clone())
  });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BillingSubscriptionState {
  Active,
  Trialing,
  ToBeCanceled,
  Canceled,
}

/// What the dialog has to say, so the promise-bearing states are named rather
/// than inferred from a chain of `if let`.
#[derive(Debug)]
enum BillingContent<'a> {
  Loading,
  SignIn,
  Subscribe,
  Manage {
    subscription: &'a CustomerStateSubscription,
    portal_url: Option<&'a str>,
  },
}

impl BillingContent<'_> {
  /// Only someone with an account has a subscription state worth asking about
  /// again.
  fn can_refresh(&self) -> bool {
    matches!(self, Self::Subscribe | Self::Manage { .. })
  }
}

fn billing_content(state: &AuthState) -> BillingContent<'_> {
  match state {
    AuthState::Unknown => BillingContent::Loading,
    AuthState::Unauthenticated => BillingContent::SignIn,
    AuthState::Authenticated(user) => match user.subscription.active_subscription.as_ref() {
      Some(subscription) => BillingContent::Manage {
        subscription,
        portal_url: user.subscription.portal_url.as_deref(),
      },
      None => BillingContent::Subscribe,
    },
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReviuProCheckoutCta {
  SubscribeMonthly,
  SubscribeAnnual,
}

impl ReviuProCheckoutCta {
  fn label(self) -> &'static str {
    match self {
      Self::SubscribeMonthly => "Start free trial",
      Self::SubscribeAnnual => "Start free trial",
    }
  }
}

fn reviu_pro_checkout_button(id: &'static str, cta: ReviuProCheckoutCta) -> Button {
  Button::new(id)
    .icon(UiIconName::CreditCard)
    .label(cta.label())
    .small()
}

/// The promise has to reach the screen where the money changes hands, not stop
/// at the surfaces that are missing the feature. `footer` is what the visitor
/// can do about it: pay, or sign in first.
fn render_pro_offer(theme: &gpui_component::Theme, footer: impl IntoElement) -> AnyElement {
  v_flex()
    .w_full()
    .gap_4()
    .child(render_pro_promise_summary(theme))
    .child(footer)
    .into_any_element()
}

fn render_pro_promise_summary(theme: &gpui_component::Theme) -> AnyElement {
  v_flex()
    .gap_2()
    .child(
      div()
        .text_sm()
        .text_color(theme.muted_foreground)
        .child("Pull requests, reviews and notifications, inside the app."),
    )
    .child(v_flex().gap_1().children(PRO_BENEFITS.map(|benefit| {
      h_flex()
        .gap_2()
        .items_start()
        .child(
          // The tick belongs on the first line of the benefit, not on the
          // optical centre of a benefit that wraps.
          div().mt(px(4.)).child(
            Icon::new(UiIconName::Check)
              .size_3()
              .text_color(theme.status_green()),
          ),
        )
        .child(
          div()
            .flex_1()
            .text_sm()
            .text_color(theme.foreground)
            .child(benefit),
        )
    })))
    .child(
      div()
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(PRO_TRIAL),
    )
    .into_any_element()
}

fn render_pro_pricing_cards(
  annual_button: impl IntoElement,
  monthly_button: impl IntoElement,
  theme: &gpui_component::Theme,
) -> impl IntoElement {
  h_flex()
    .gap_3()
    .child(
      v_flex()
        .flex_1()
        .gap_2()
        .child(
          div()
            .text_sm()
            .font_semibold()
            .text_color(theme.foreground)
            .child("Annual"),
        )
        .child(
          h_flex()
            .items_end()
            .gap_2()
            .child(
              h_flex()
                .items_end()
                .gap_1()
                .child(
                  div()
                    .text_xl()
                    .font_semibold()
                    .text_color(theme.foreground)
                    .child(PRO_ANNUAL_PRICE),
                )
                .child(
                  div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(PRO_ANNUAL_PERIOD),
                ),
            )
            .child(
              div()
                .text_xs()
                .font_semibold()
                .text_color(theme.status_blue())
                .child(PRO_ANNUAL_SAVE_PERCENT),
            ),
        )
        .child(h_flex().justify_start().child(annual_button)),
    )
    .child(
      v_flex()
        .flex_1()
        .gap_2()
        .child(
          div()
            .text_sm()
            .font_semibold()
            .text_color(theme.foreground)
            .child("Monthly"),
        )
        .child(
          h_flex()
            .items_end()
            .gap_1()
            .child(
              div()
                .text_xl()
                .font_semibold()
                .text_color(theme.foreground)
                .child(PRO_MONTHLY_PRICE),
            )
            .child(
              div()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(PRO_MONTHLY_PERIOD),
            ),
        )
        .child(h_flex().justify_start().child(monthly_button)),
    )
}

struct BillingDialog {
  api: ApiClient,
  checkout_loading: bool,
  refresh_loading: bool,
  checkout_task: Option<gpui::Task<()>>,
  refresh_task: Option<gpui::Task<()>>,
  error: Option<SharedString>,
}

impl BillingDialog {
  fn new(cx: &mut Context<Self>) -> Self {
    Self {
      api: WorkspaceApi::global(cx).api.clone(),
      checkout_loading: false,
      refresh_loading: false,
      checkout_task: None,
      refresh_task: None,
      error: None,
    }
  }

  fn refresh_action(&mut self, _: &gpui::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
    self.refresh_subscription_state(cx);
  }

  fn subscribe_monthly_action(
    &mut self,
    _: &gpui::ClickEvent,
    _: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.start_checkout(PRO_MONTHLY_SLUG, cx);
  }

  fn subscribe_annual_action(
    &mut self,
    _: &gpui::ClickEvent,
    _: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.start_checkout(PRO_ANNUAL_SLUG, cx);
  }

  fn refresh_subscription_state(&mut self, cx: &mut Context<Self>) {
    if self.refresh_loading {
      return;
    }

    self.refresh_loading = true;
    self.error = None;

    let api = self.api.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move { api.fetch_me() })
        .await
        .map_err(|error| error.to_string());

      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(Some(user)) => AuthStateStore::set(cx, AuthState::Authenticated(Box::new(user))),
          Ok(None) => AuthStateStore::set(cx, AuthState::Unauthenticated),
          Err(error) => this.error = Some(error.into()),
        }

        this.refresh_loading = false;
        cx.notify();
        cx.refresh_windows();
      });
    });

    self.refresh_task = Some(task);
    cx.notify();
  }

  fn start_checkout(&mut self, slug: &'static str, cx: &mut Context<Self>) {
    if self.checkout_loading {
      return;
    }

    self.checkout_loading = true;
    self.error = None;
    crate::analytics::track_with(
      cx,
      "subscription_checkout_started",
      Some(serde_json::json!({ "slug": slug, "source": ANALYTICS_SOURCE })),
    );

    let api = self.api.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move { api.checkout_subscription(slug) })
        .await
        .map_err(|error| error.to_string());

      match result {
        Ok(url) => {
          cx.update(|cx| cx.open_url(&url));
          let _ = this.update(cx, |this, cx| {
            this.checkout_loading = false;
            cx.notify();
          });
        }
        Err(error) => {
          let _ = this.update(cx, |this, cx| {
            this.checkout_loading = false;
            this.error = Some(error.into());
            cx.notify();
          });
        }
      }
    });

    self.checkout_task = Some(task);
    cx.notify();
  }

  fn format_amount(amount_cents: i64, currency: &str) -> SharedString {
    let amount = amount_cents as f64 / 100.0;
    let currency = currency.to_uppercase();
    if currency == "USD" {
      return format!("${amount:.2}").into();
    }
    format!("{amount:.2} {currency}").into()
  }

  fn has_subscription_ended(subscription: &CustomerStateSubscription) -> bool {
    let end_timestamp = subscription
      .current_period_end
      .as_deref()
      .or(subscription.ends_at.as_deref())
      .and_then(parse_rfc3339);
    let now = OffsetDateTime::now_utc();

    matches!(end_timestamp, Some(end) if end <= now)
  }

  fn display_status(subscription: &CustomerStateSubscription) -> BillingSubscriptionState {
    if subscription.cancel_at_period_end {
      if Self::has_subscription_ended(subscription) {
        return BillingSubscriptionState::Canceled;
      }
      return BillingSubscriptionState::ToBeCanceled;
    }

    match subscription.status {
      CustomerStateSubscriptionStatus::Active => BillingSubscriptionState::Active,
      CustomerStateSubscriptionStatus::Trialing => BillingSubscriptionState::Trialing,
    }
  }

  fn status_label(state: BillingSubscriptionState) -> &'static str {
    match state {
      BillingSubscriptionState::Active => "Active",
      BillingSubscriptionState::Trialing => "Trialing",
      BillingSubscriptionState::ToBeCanceled => "To be canceled",
      BillingSubscriptionState::Canceled => "Canceled",
    }
  }

  fn status_color(state: BillingSubscriptionState, theme: &gpui_component::Theme) -> gpui::Hsla {
    match state {
      BillingSubscriptionState::Active => theme.status_green(),
      BillingSubscriptionState::Trialing => theme.status_violet(),
      BillingSubscriptionState::ToBeCanceled => theme.status_orange(),
      BillingSubscriptionState::Canceled => theme.status_red(),
    }
  }

  fn billing_date_label(subscription: &CustomerStateSubscription) -> &'static str {
    match Self::display_status(subscription) {
      BillingSubscriptionState::ToBeCanceled | BillingSubscriptionState::Canceled => "Expiry Date",
      BillingSubscriptionState::Active | BillingSubscriptionState::Trialing => "Renewal Date",
    }
  }

  fn render_subscription(
    &self,
    subscription: &CustomerStateSubscription,
    portal_url: Option<&str>,
    theme: &gpui_component::Theme,
  ) -> AnyElement {
    let status = Self::display_status(subscription);
    let amount = Self::format_amount(subscription.amount, subscription.currency.as_str());
    let plan = format!(
      "{}/{}",
      amount,
      subscription.recurring_interval.to_lowercase()
    );
    let billing_date_label = Self::billing_date_label(subscription);
    let billing_date_value = if subscription.cancel_at_period_end {
      format_long_date_opt(
        subscription
          .current_period_end
          .as_deref()
          .or(subscription.ends_at.as_deref()),
      )
    } else {
      format_long_date_opt(subscription.current_period_end.as_deref())
    };
    let start_date_value = format_long_date_opt(
      subscription
        .started_at
        .as_deref()
        .or(Some(subscription.current_period_start.as_str())),
    );

    let row = |label: &'static str, value: SharedString| {
      h_flex()
        .items_center()
        .justify_between()
        .gap_4()
        .child(
          div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child(label),
        )
        .child(
          div()
            .text_sm()
            .text_color(theme.foreground)
            .text_right()
            .child(value),
        )
    };

    v_flex()
      .w_full()
      .gap_3()
      .child(
        h_flex()
          .items_center()
          .justify_between()
          .gap_4()
          .child(
            div()
              .text_sm()
              .font_semibold()
              .text_color(theme.foreground)
              .child(plan),
          )
          .child(
            StatusTag::new(Self::status_color(status, theme))
              .outline()
              .child(Self::status_label(status)),
          ),
      )
      .child(row("Start Date", start_date_value))
      .child(row(billing_date_label, billing_date_value))
      .when_some(
        portal_url.and_then(|url| {
          let trimmed = url.trim();
          if trimmed.is_empty() {
            None
          } else {
            Some(trimmed.to_string())
          }
        }),
        |this, portal_url| {
          this.child(
            h_flex().justify_start().child(
              Button::new("billing-portal-active")
                .icon(IconName::ExternalLink)
                .label("Open billing portal")
                .small()
                .on_click(move |_, _, cx| {
                  cx.open_url(&portal_url);
                }),
            ),
          )
        },
      )
      .into_any_element()
  }

  fn render_subscribe(&self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();

    let annual_button = reviu_pro_checkout_button(
      "billing-subscribe-annual",
      ReviuProCheckoutCta::SubscribeAnnual,
    )
    .loading(self.checkout_loading)
    .disabled(self.checkout_loading || self.refresh_loading)
    .on_click(cx.listener(Self::subscribe_annual_action));

    let monthly_button = reviu_pro_checkout_button(
      "billing-subscribe-monthly",
      ReviuProCheckoutCta::SubscribeMonthly,
    )
    .loading(self.checkout_loading)
    .disabled(self.checkout_loading || self.refresh_loading)
    .on_click(cx.listener(Self::subscribe_monthly_action));

    render_pro_offer(
      &theme,
      render_pro_pricing_cards(annual_button, monthly_button, &theme),
    )
  }

  fn render_sign_in(&self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();

    render_pro_offer(
      &theme,
      h_flex().justify_start().child(
        Button::new("billing-sign-in")
          .icon(IconName::Github)
          .label("Sign in with GitHub")
          .small()
          .on_click(|_, _, cx| {
            crate::auth_flow::start_github_sign_in(cx, ANALYTICS_SOURCE);
          }),
      ),
    )
  }

  fn render_loading(&self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();

    v_flex()
      .w_full()
      .items_center()
      .justify_center()
      .gap_2()
      .py_4()
      .child(Spinner::new().small())
      .child(
        div()
          .text_sm()
          .text_color(theme.muted_foreground)
          .child("Loading subscription..."),
      )
      .into_any_element()
  }
}

impl Render for BillingDialog {
  fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    let auth_state = AuthStateStore::get(cx);
    let content = billing_content(&auth_state);
    let can_refresh = content.can_refresh();
    let body = match content {
      BillingContent::Loading => self.render_loading(cx),
      BillingContent::SignIn => self.render_sign_in(cx),
      BillingContent::Subscribe => self.render_subscribe(cx),
      BillingContent::Manage {
        subscription,
        portal_url,
      } => self.render_subscription(subscription, portal_url, &theme),
    };

    let refresh = can_refresh.then(|| {
      Button::new("billing-refresh")
        .debug_selector(|| REFRESH_DEBUG_SELECTOR.to_string())
        .icon(UiIconName::RefreshCw)
        .label("Refresh")
        .ghost()
        .small()
        .loading(self.refresh_loading)
        .disabled(self.refresh_loading || self.checkout_loading)
        .on_click(cx.listener(Self::refresh_action))
    });

    div()
      .id("billing-dialog")
      .flex()
      .flex_col()
      .child(
        DialogHeader::new()
          .p_4()
          .child(DialogTitle::new().child("Reviu Pro")),
      )
      .child(
        v_flex()
          .px_4()
          .pb_4()
          .gap_3()
          .child(body)
          .when_some(self.error.clone(), |this, error| {
            this.child(div().text_sm().text_color(theme.status_red()).child(error))
          }),
      )
      .child(
        DialogFooter::new()
          .px_4()
          .pb_4()
          .pt_1()
          // Refresh is plumbing, not part of the offer: it sits opposite Close.
          .when(refresh.is_some(), |this| this.justify_between())
          .children(refresh)
          .child(
            Button::new("billing-close")
              .debug_selector(|| CLOSE_DEBUG_SELECTOR.to_string())
              .label("Close")
              .primary()
              .on_click(|_, window, cx| window.close_dialog(cx)),
          ),
      )
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use gpui::{TestAppContext, VisualTestContext};

  fn make_subscription() -> CustomerStateSubscription {
    CustomerStateSubscription {
      id: "sub_123".to_string(),
      created_at: "2026-01-01T00:00:00Z".to_string(),
      modified_at: None,
      status: CustomerStateSubscriptionStatus::Active,
      amount: 2_000,
      currency: "usd".to_string(),
      recurring_interval: "month".to_string(),
      current_period_start: "2026-01-01T00:00:00Z".to_string(),
      current_period_end: Some("2099-01-01T00:00:00Z".to_string()),
      trial_start: None,
      trial_end: None,
      cancel_at_period_end: false,
      canceled_at: None,
      started_at: Some("2026-01-01T00:00:00Z".to_string()),
      ends_at: None,
      product_id: "prod_123".to_string(),
    }
  }

  fn signed_in_with(subscription: Option<CustomerStateSubscription>) -> AuthState {
    let AuthState::Authenticated(mut user) = crate::auth_state::signed_in_without_subscription()
    else {
      unreachable!("the fixture is authenticated");
    };
    user.subscription.active_subscription = subscription;
    AuthState::Authenticated(user)
  }

  struct Page;

  impl Render for Page {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
      div()
        .size_full()
        .children(gpui_component::Root::render_dialog_layer(window, cx))
    }
  }

  fn open_dialog_over_a_page(state: AuthState, cx: &mut TestAppContext) -> &mut VisualTestContext {
    cx.update(|cx| {
      gpui_component::init(cx);
      cx.set_global(AuthStateStore::default());
      cx.set_global(WorkspaceApi::new());
      AuthStateStore::set(cx, state);
    });

    let (_root, cx) = cx.add_window_view(|window, cx| {
      let page = cx.new(|_| Page);
      gpui_component::Root::new(page, window, cx)
    });

    cx.update(open_billing_dialog_inner);
    cx.run_until_parked();
    cx
  }

  #[gpui::test]
  fn the_dialog_opens_over_whatever_page_is_up(cx: &mut TestAppContext) {
    cx.update(|cx| {
      gpui_component::init(cx);
      cx.set_global(AuthStateStore::default());
      cx.set_global(WorkspaceApi::new());
    });

    let (_root, cx) = cx.add_window_view(|window, cx| {
      let page = cx.new(|_| Page);
      gpui_component::Root::new(page, window, cx)
    });

    cx.update(|window, cx| {
      assert!(!window.has_active_dialog(cx));
      open_billing_dialog_inner(window, cx);
    });
    cx.run_until_parked();

    cx.update(|window, cx| assert!(window.has_active_dialog(cx)));
  }

  #[gpui::test]
  fn coming_back_to_an_open_dialog_leaves_a_single_one_to_close(cx: &mut TestAppContext) {
    let cx = open_dialog_over_a_page(signed_in_with(None), cx);

    cx.update(open_billing_dialog_inner);
    cx.run_until_parked();

    cx.update(|window, cx| window.close_dialog(cx));
    cx.run_until_parked();

    assert!(
      !cx.update(|window, cx| window.has_active_dialog(cx)),
      "the checkout callback reused the dialog already up instead of stacking one"
    );
  }

  #[gpui::test]
  fn the_dialog_opens_again_once_the_previous_one_is_closed(cx: &mut TestAppContext) {
    let cx = open_dialog_over_a_page(signed_in_with(None), cx);

    cx.update(|window, cx| window.close_dialog(cx));
    cx.run_until_parked();

    cx.update(open_billing_dialog_inner);
    cx.run_until_parked();

    assert!(cx.update(|window, cx| window.has_active_dialog(cx)));
  }

  #[gpui::test]
  fn refresh_sits_in_the_footer_opposite_close(cx: &mut TestAppContext) {
    let cx = open_dialog_over_a_page(signed_in_with(None), cx);

    let refresh = cx
      .debug_bounds(REFRESH_DEBUG_SELECTOR)
      .expect("refresh bounds");
    let close = cx.debug_bounds(CLOSE_DEBUG_SELECTOR).expect("close bounds");

    assert!(
      refresh.right() < close.left(),
      "refresh sits on the left of the footer, close on the right"
    );
    assert!(
      (refresh.center().y - close.center().y).abs() < px(2.),
      "refresh shares the footer row with close, it no longer floats in the body"
    );
  }

  #[gpui::test]
  fn a_visitor_without_an_account_only_gets_close(cx: &mut TestAppContext) {
    let cx = open_dialog_over_a_page(AuthState::Unauthenticated, cx);

    assert!(cx.debug_bounds(CLOSE_DEBUG_SELECTOR).is_some());
    assert!(
      cx.debug_bounds(REFRESH_DEBUG_SELECTOR).is_none(),
      "there is no subscription state to ask about again"
    );
  }

  #[test]
  fn every_auth_state_maps_to_what_the_dialog_says() {
    assert!(matches!(
      billing_content(&AuthState::Unknown),
      BillingContent::Loading
    ));
    assert!(matches!(
      billing_content(&AuthState::Unauthenticated),
      BillingContent::SignIn
    ));
    assert!(matches!(
      billing_content(&signed_in_with(None)),
      BillingContent::Subscribe
    ));

    let state = signed_in_with(Some(make_subscription()));
    assert!(matches!(
      billing_content(&state),
      BillingContent::Manage { .. }
    ));
  }

  #[test]
  fn only_someone_with_an_account_can_ask_again() {
    assert!(billing_content(&signed_in_with(None)).can_refresh());
    assert!(
      billing_content(&signed_in_with(Some(make_subscription()))).can_refresh(),
      "an answer that came back stale is exactly what the button is for"
    );

    assert!(!billing_content(&AuthState::Unauthenticated).can_refresh());
    assert!(
      !billing_content(&AuthState::Unknown).can_refresh(),
      "an answer is already on its way"
    );
  }

  #[test]
  fn the_promise_names_what_pro_brings() {
    assert!(!PRO_BENEFITS.is_empty());
    assert!(
      PRO_BENEFITS
        .iter()
        .any(|line| line.contains("Pull requests"))
    );
    assert!(
      PRO_BENEFITS
        .iter()
        .any(|line| line.contains("notifications"))
    );
    assert!(PRO_TRIAL.contains("free trial"));
  }

  #[test]
  fn display_status_matches_active_and_trialing() {
    let mut active = make_subscription();
    active.cancel_at_period_end = false;
    active.status = CustomerStateSubscriptionStatus::Active;
    assert_eq!(
      BillingDialog::display_status(&active),
      BillingSubscriptionState::Active
    );

    let mut trialing = make_subscription();
    trialing.cancel_at_period_end = false;
    trialing.status = CustomerStateSubscriptionStatus::Trialing;
    assert_eq!(
      BillingDialog::display_status(&trialing),
      BillingSubscriptionState::Trialing
    );
  }

  #[test]
  fn display_status_is_to_be_canceled_when_end_date_is_in_future() {
    let mut subscription = make_subscription();
    subscription.cancel_at_period_end = true;
    subscription.current_period_end = Some("2099-01-01T00:00:00Z".to_string());

    assert_eq!(
      BillingDialog::display_status(&subscription),
      BillingSubscriptionState::ToBeCanceled
    );
  }

  #[test]
  fn display_status_is_canceled_when_end_date_is_in_past() {
    let mut subscription = make_subscription();
    subscription.cancel_at_period_end = true;
    subscription.current_period_end = Some("2000-01-01T00:00:00Z".to_string());

    assert_eq!(
      BillingDialog::display_status(&subscription),
      BillingSubscriptionState::Canceled
    );
  }

  #[test]
  fn display_status_uses_ends_at_when_current_period_end_is_missing() {
    let mut subscription = make_subscription();
    subscription.cancel_at_period_end = true;
    subscription.current_period_end = None;
    subscription.ends_at = Some("2000-01-01T00:00:00Z".to_string());

    assert_eq!(
      BillingDialog::display_status(&subscription),
      BillingSubscriptionState::Canceled
    );
  }

  #[test]
  fn billing_date_label_is_renewal_for_active_and_trialing() {
    let mut active = make_subscription();
    active.status = CustomerStateSubscriptionStatus::Active;
    active.cancel_at_period_end = false;
    assert_eq!(BillingDialog::billing_date_label(&active), "Renewal Date");

    let mut trialing = make_subscription();
    trialing.status = CustomerStateSubscriptionStatus::Trialing;
    trialing.cancel_at_period_end = false;
    assert_eq!(BillingDialog::billing_date_label(&trialing), "Renewal Date");
  }

  #[test]
  fn billing_date_label_is_expiry_for_cancellation_states() {
    let mut to_be_canceled = make_subscription();
    to_be_canceled.cancel_at_period_end = true;
    to_be_canceled.current_period_end = Some("2099-01-01T00:00:00Z".to_string());
    assert_eq!(
      BillingDialog::billing_date_label(&to_be_canceled),
      "Expiry Date"
    );

    let mut canceled = make_subscription();
    canceled.cancel_at_period_end = true;
    canceled.current_period_end = Some("2000-01-01T00:00:00Z".to_string());
    assert_eq!(BillingDialog::billing_date_label(&canceled), "Expiry Date");
  }

  #[test]
  fn status_label_covers_all_billing_states() {
    assert_eq!(
      BillingDialog::status_label(BillingSubscriptionState::Active),
      "Active"
    );
    assert_eq!(
      BillingDialog::status_label(BillingSubscriptionState::Trialing),
      "Trialing"
    );
    assert_eq!(
      BillingDialog::status_label(BillingSubscriptionState::ToBeCanceled),
      "To be canceled"
    );
    assert_eq!(
      BillingDialog::status_label(BillingSubscriptionState::Canceled),
      "Canceled"
    );
  }
}
