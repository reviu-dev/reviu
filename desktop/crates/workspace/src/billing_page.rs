use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{
  App, Context, FocusHandle, Focusable, Render, SharedString, Window, div, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Disableable as _, IconName, Sizable as _, StyledExt,
  button::{Button, ButtonVariants as _},
  h_flex,
  spinner::Spinner,
  v_flex,
};
use smol::unblock;
use ui::{
  CommandPalette, CommandPaletteAction, CommandPaletteCommand, CommandPaletteConfig,
  CommandPaletteHandler, CommandPalettePage, HEADER_HEIGHT, StatusThemeExt, UiIconName, WindowExt,
};

use crate::{
  AuthCallbackTarget, CloseWorkspacePage, ShowCommandPalette,
  api::{ApiClient, CustomerStateSubscription, CustomerStateSubscriptionStatus},
  auth_state::{AuthState, AuthStateStore},
  github_page::GithubPageHandle,
  github_pr_details_page::GithubPrDetailsPageHandle,
  workspace::{WorkspaceApi, WorkspacePage, WorkspaceRoute},
};

pub struct BillingPage {
  focus_handle: FocusHandle,
  api: ApiClient,
  checkout_loading: bool,
  refresh_loading: bool,
  checkout_task: Option<gpui::Task<()>>,
  refresh_task: Option<gpui::Task<()>>,
  error: Option<SharedString>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BillingSubscriptionState {
  Active,
  Trialing,
  ToBeCanceled,
  Canceled,
}

impl BillingPage {
  pub fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
    let mut view = Self {
      focus_handle: cx.focus_handle(),
      api: WorkspaceApi::global(cx).api.clone(),
      checkout_loading: false,
      refresh_loading: false,
      checkout_task: None,
      refresh_task: None,
      error: None,
    };

    view.refresh_subscription_state(cx);

    view
  }

  fn refresh_action(&mut self, _: &gpui::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
    self.refresh_subscription_state(cx);
  }

  fn subscribe_action(&mut self, _: &gpui::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
    self.start_checkout(cx);
  }

  fn refresh_subscription_state(&mut self, cx: &mut Context<Self>) {
    if self.refresh_loading {
      return;
    }

    self.refresh_loading = true;
    self.error = None;

    let api = self.api.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || api.fetch_me())
        .await
        .map_err(|error| error.to_string());

      let _ = this.update(cx, |this, cx| {
        match result {
          Ok(Some(user)) => AuthStateStore::set(cx, AuthState::Authenticated(user)),
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

  fn start_checkout(&mut self, cx: &mut Context<Self>) {
    if self.checkout_loading {
      return;
    }

    self.checkout_loading = true;
    self.error = None;

    let api = self.api.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || api.checkout_subscription("pro"))
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

  fn show_command_palette_action(
    &mut self,
    _: &ShowCommandPalette,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.open_command_palette(window, cx);
  }

  fn close_workspace_page_action(
    &mut self,
    _: &CloseWorkspacePage,
    _: &mut Window,
    cx: &mut Context<Self>,
  ) {
    WorkspaceRoute::close_billing(cx);
    cx.refresh_windows();
  }

  fn open_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let include_github = matches!(AuthStateStore::get(cx), AuthState::Authenticated(_));
    let commands =
      CommandPaletteCommand::default_global_commands(CommandPalettePage::Settings, include_github);

    let view = cx.entity();
    let handler: CommandPaletteHandler = Arc::new(move |action, _window, cx| {
      view.update(cx, |view, cx| {
        view.handle_command_palette_action(action, cx)
      })
    });

    let config = CommandPaletteConfig::new(Vec::new(), commands, handler);
    let palette = cx.new(|cx| CommandPalette::new(window, cx, config));
    let palette_for_dialog = palette.clone();

    window.open_dialog(cx, move |dialog, _, _| {
      dialog
        .p_0()
        .border_0()
        .min_h_0()
        .overlay_closable(true)
        .keyboard(true)
        .close_button(false)
        .child(palette_for_dialog.clone())
    });
  }

  fn handle_command_palette_action(
    &mut self,
    action: CommandPaletteAction,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    match action {
      CommandPaletteAction::OpenGitPage => {
        WorkspaceRoute::global_mut(cx).page = WorkspacePage::Git;
        cx.refresh_windows();
        Ok(())
      }
      CommandPaletteAction::OpenGithubPage => {
        if AuthStateStore::has_active_subscription(cx) {
          GithubPageHandle::refresh(cx);
          WorkspaceRoute::open_github(cx);
        } else {
          WorkspaceRoute::open_billing(cx);
        }
        cx.refresh_windows();
        Ok(())
      }
      CommandPaletteAction::OpenGithubPrDetails {
        owner,
        repo,
        number,
      } => {
        GithubPrDetailsPageHandle::show(owner.into(), repo.into(), number, cx);
        Ok(())
      }
      CommandPaletteAction::OpenSettingsPage => {
        WorkspaceRoute::open_settings(cx);
        cx.refresh_windows();
        Ok(())
      }
      CommandPaletteAction::OpenGitConfigPage => {
        WorkspaceRoute::open_git_config(cx);
        cx.refresh_windows();
        Ok(())
      }
      _ => Err("Command not available.".into()),
    }
  }

  fn format_datetime(value: Option<&str>) -> SharedString {
    let Some(value) = value else {
      return "—".into();
    };

    let trimmed = value.trim();
    let date_part = trimmed
      .split_once('T')
      .map(|(date, _)| date)
      .unwrap_or(trimmed);

    let mut parts = date_part.split('-');
    let year = parts.next().and_then(|value| value.parse::<u32>().ok());
    let month = parts.next().and_then(|value| value.parse::<u32>().ok());
    let day = parts.next().and_then(|value| value.parse::<u32>().ok());

    if parts.next().is_some() {
      return trimmed.to_string().into();
    }

    let (Some(year), Some(month), Some(day)) = (year, month, day) else {
      return trimmed.to_string().into();
    };

    let month_name = match month {
      1 => "January",
      2 => "February",
      3 => "March",
      4 => "April",
      5 => "May",
      6 => "June",
      7 => "July",
      8 => "August",
      9 => "September",
      10 => "October",
      11 => "November",
      12 => "December",
      _ => return trimmed.to_string().into(),
    };

    format!("{month_name} {day}, {year}").into()
  }

  fn format_amount(amount_cents: i64, currency: &str) -> SharedString {
    let amount = amount_cents as f64 / 100.0;
    let currency = currency.to_uppercase();
    if currency == "USD" {
      return format!("${amount:.2}").into();
    }
    format!("{amount:.2} {currency}").into()
  }

  fn current_unix_seconds() -> Option<i64> {
    let duration = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
    Some(duration.as_secs() as i64)
  }

  fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = year - if month <= 2 { 1 } else { 0 };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let month = month as i32;
    let day = day as i32;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era as i64 * 146_097 + doe as i64 - 719_468
  }

  fn parse_rfc3339_to_unix_seconds(value: &str) -> Option<i64> {
    let (date_part, time_part_with_offset) = value.trim().split_once('T')?;

    let mut date_split = date_part.split('-');
    let year = date_split.next()?.parse::<i32>().ok()?;
    let month = date_split.next()?.parse::<u32>().ok()?;
    let day = date_split.next()?.parse::<u32>().ok()?;
    if date_split.next().is_some() {
      return None;
    }

    let (time_part, offset_seconds) =
      if let Some(time_part) = time_part_with_offset.strip_suffix('Z') {
        (time_part, 0i64)
      } else if let Some((time_part, offset_part)) = time_part_with_offset.rsplit_once('+') {
        let (offset_hours, offset_minutes) = offset_part.split_once(':')?;
        let offset_hours = offset_hours.parse::<i64>().ok()?;
        let offset_minutes = offset_minutes.parse::<i64>().ok()?;
        (time_part, offset_hours * 3600 + offset_minutes * 60)
      } else if let Some((time_part, offset_part)) = time_part_with_offset.rsplit_once('-') {
        let (offset_hours, offset_minutes) = offset_part.split_once(':')?;
        let offset_hours = offset_hours.parse::<i64>().ok()?;
        let offset_minutes = offset_minutes.parse::<i64>().ok()?;
        (time_part, -(offset_hours * 3600 + offset_minutes * 60))
      } else {
        return None;
      };

    let mut time_split = time_part.split(':');
    let hour = time_split.next()?.parse::<i64>().ok()?;
    let minute = time_split.next()?.parse::<i64>().ok()?;
    let second_raw = time_split.next()?;
    if time_split.next().is_some() {
      return None;
    }
    let second = second_raw
      .split('.')
      .next()
      .and_then(|value| value.parse::<i64>().ok())?;

    let days = Self::days_from_civil(year, month, day);
    let unix_seconds = days * 86_400 + hour * 3_600 + minute * 60 + second - offset_seconds;
    Some(unix_seconds)
  }

  fn has_subscription_ended(subscription: &CustomerStateSubscription) -> bool {
    let end_timestamp = subscription
      .current_period_end
      .as_deref()
      .or(subscription.ends_at.as_deref())
      .and_then(Self::parse_rfc3339_to_unix_seconds);
    let now = Self::current_unix_seconds();

    matches!((end_timestamp, now), (Some(end), Some(now)) if end <= now)
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

  fn status_badge(
    subscription: &CustomerStateSubscription,
    theme: &gpui_component::Theme,
  ) -> (SharedString, gpui::Hsla) {
    match Self::display_status(subscription) {
      BillingSubscriptionState::Active => ("Active".into(), theme.status_green()),
      BillingSubscriptionState::Trialing => ("Trialing".into(), theme.status_violet()),
      BillingSubscriptionState::ToBeCanceled => ("To be canceled".into(), theme.status_orange()),
      BillingSubscriptionState::Canceled => ("Canceled".into(), theme.status_red()),
    }
  }

  fn billing_date_label(subscription: &CustomerStateSubscription) -> &'static str {
    match Self::display_status(subscription) {
      BillingSubscriptionState::ToBeCanceled | BillingSubscriptionState::Canceled => "Expiry Date",
      BillingSubscriptionState::Active | BillingSubscriptionState::Trialing => "Renewal Date",
    }
  }

  fn render_active_subscription(
    &self,
    subscription: &CustomerStateSubscription,
    portal_url: Option<&str>,
    theme: &gpui_component::Theme,
  ) -> impl IntoElement {
    let (status_label, status_color) = Self::status_badge(subscription, theme);
    let amount = Self::format_amount(subscription.amount, subscription.currency.as_str());
    let plan = format!(
      "{}/{}",
      amount,
      subscription.recurring_interval.to_lowercase()
    );
    let billing_date_label = Self::billing_date_label(subscription);
    let billing_date_value = if subscription.cancel_at_period_end {
      Self::format_datetime(
        subscription
          .current_period_end
          .as_deref()
          .or(subscription.ends_at.as_deref()),
      )
    } else {
      Self::format_datetime(subscription.current_period_end.as_deref())
    };
    let start_date_value = Self::format_datetime(
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
      .max_w(px(700.))
      .mx_auto()
      .gap_4()
      .p_4()
      .border_1()
      .border_color(theme.border)
      .rounded(theme.radius)
      .bg(theme.sidebar)
      .child(
        h_flex()
          .items_center()
          .justify_between()
          .gap_4()
          .child(
            v_flex()
              .gap_1()
              .child(
                div()
                  .text_lg()
                  .font_semibold()
                  .text_color(theme.foreground)
                  .child("Pro subscription"),
              )
              .child(
                div()
                  .text_sm()
                  .text_color(theme.muted_foreground)
                  .child(plan),
              ),
          )
          .child(
            div()
              .px_2()
              .py_1()
              .rounded_full()
              .border_1()
              .border_color(status_color.opacity(0.5))
              .text_xs()
              .font_medium()
              .text_color(status_color)
              .child(status_label),
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
            Button::new("billing-portal-active")
              .icon(IconName::ExternalLink)
              .label("Open billing portal")
              .small()
              .on_click(move |_, _, cx| {
                cx.open_url(&portal_url);
              }),
          )
        },
      )
  }

  fn render_no_subscription(&self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    v_flex()
      .w_full()
      .max_w(px(700.))
      .mx_auto()
      .gap_4()
      .p_4()
      .border_1()
      .border_color(theme.border)
      .rounded(theme.radius)
      .bg(theme.sidebar)
      .child(
        v_flex()
          .gap_1()
          .child(
            div()
              .text_lg()
              .font_semibold()
              .text_color(theme.foreground)
              .child("Reviu Pro"),
          )
          .child(
            h_flex()
              .items_end()
              .gap_2()
              .child(
                div()
                  .text_xl()
                  .font_semibold()
                  .text_color(theme.foreground)
                  .child("$20"),
              )
              .child(
                div()
                  .text_sm()
                  .text_color(theme.muted_foreground)
                  .child("/ month"),
              ),
          ),
      )
      .child(
        div()
          .text_sm()
          .text_color(theme.muted_foreground)
          .child("No active subscription found for your account."),
      )
      .child(
        Button::new("billing-subscribe")
          .icon(UiIconName::CreditCard)
          .label("Subscribe")
          .small()
          .loading(self.checkout_loading)
          .disabled(self.checkout_loading || self.refresh_loading)
          .on_click(cx.listener(Self::subscribe_action)),
      )
  }

  fn render_unauthenticated(&self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    v_flex()
      .w_full()
      .max_w(px(620.))
      .mx_auto()
      .gap_3()
      .p_4()
      .border_1()
      .border_color(theme.border)
      .rounded(theme.radius)
      .bg(theme.sidebar)
      .child(
        div()
          .text_lg()
          .font_semibold()
          .text_color(theme.foreground)
          .child("Billing"),
      )
      .child(
        div()
          .text_sm()
          .text_color(theme.muted_foreground)
          .child("Sign in with GitHub to manage your subscription."),
      )
      .child(
        Button::new("billing-sign-in")
          .icon(IconName::GitHub)
          .label("Sign in with GitHub")
          .small()
          .on_click(|_, _, cx| {
            AuthCallbackTarget::start_sign_in(cx);
          }),
      )
  }

  fn render_loading(&self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    v_flex()
      .w_full()
      .h_full()
      .items_center()
      .justify_center()
      .gap_2()
      .child(Spinner::new().small())
      .child(
        div()
          .text_sm()
          .text_color(theme.muted_foreground)
          .child("Loading subscription..."),
      )
  }

  fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    let refresh_button = Button::new("billing-refresh")
      .icon(UiIconName::RefreshCcw)
      .ghost()
      .compact()
      .loading(self.refresh_loading)
      .disabled(self.refresh_loading || self.checkout_loading)
      .tooltip("Refresh subscription state")
      .on_click(cx.listener(Self::refresh_action));

    let close_button = Button::new("close-billing")
      .icon(IconName::Close)
      .ghost()
      .compact()
      .tooltip("Close billing")
      .on_click(|_, _, cx| {
        WorkspaceRoute::close_billing(cx);
        cx.refresh_windows();
      });

    div()
      .h(px(HEADER_HEIGHT))
      .max_h(px(HEADER_HEIGHT))
      .px_3()
      .flex()
      .items_center()
      .justify_between()
      .bg(theme.sidebar)
      .border_b_1()
      .border_color(theme.title_bar_border)
      .child(
        div()
          .text_sm()
          .text_color(theme.foreground)
          .child("Billing"),
      )
      .child(
        h_flex()
          .items_center()
          .gap_2()
          .child(refresh_button)
          .child(close_button),
      )
  }
}

impl Render for BillingPage {
  fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    let content = match AuthStateStore::get(cx) {
      AuthState::Unknown => self.render_loading(cx).into_any_element(),
      AuthState::Unauthenticated => self.render_unauthenticated(cx).into_any_element(),
      AuthState::Authenticated(user) => {
        let portal_url = user.subscription.portal_url.as_deref();
        if let Some(subscription) = user.subscription.active_subscription.as_ref() {
          self
            .render_active_subscription(subscription, portal_url, &theme)
            .into_any_element()
        } else {
          self.render_no_subscription(cx).into_any_element()
        }
      }
    };

    div()
      .size_full()
      .flex()
      .flex_col()
      .bg(theme.background)
      .track_focus(&self.focus_handle(cx))
      .on_action(cx.listener(BillingPage::show_command_palette_action))
      .on_action(cx.listener(BillingPage::close_workspace_page_action))
      .child(self.render_header(cx))
      .child(
        div().w_full().mx_auto().h_full().min_h_0().p_4().child(
          v_flex()
            .gap_3()
            .child(content)
            .when_some(self.error.clone(), |this, error| {
              this.child(
                div().w_full().flex().justify_center().child(
                  div()
                    .text_sm()
                    .text_color(theme.status_red())
                    .text_center()
                    .child(error),
                ),
              )
            }),
        ),
      )
  }
}

impl Focusable for BillingPage {
  fn focus_handle(&self, _: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

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

  #[test]
  fn display_status_matches_active_and_trialing() {
    let mut active = make_subscription();
    active.cancel_at_period_end = false;
    active.status = CustomerStateSubscriptionStatus::Active;
    assert_eq!(
      BillingPage::display_status(&active),
      BillingSubscriptionState::Active
    );

    let mut trialing = make_subscription();
    trialing.cancel_at_period_end = false;
    trialing.status = CustomerStateSubscriptionStatus::Trialing;
    assert_eq!(
      BillingPage::display_status(&trialing),
      BillingSubscriptionState::Trialing
    );
  }

  #[test]
  fn display_status_is_to_be_canceled_when_end_date_is_in_future() {
    let mut subscription = make_subscription();
    subscription.cancel_at_period_end = true;
    subscription.current_period_end = Some("2099-01-01T00:00:00Z".to_string());

    assert_eq!(
      BillingPage::display_status(&subscription),
      BillingSubscriptionState::ToBeCanceled
    );
  }

  #[test]
  fn display_status_is_canceled_when_end_date_is_in_past() {
    let mut subscription = make_subscription();
    subscription.cancel_at_period_end = true;
    subscription.current_period_end = Some("2000-01-01T00:00:00Z".to_string());

    assert_eq!(
      BillingPage::display_status(&subscription),
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
      BillingPage::display_status(&subscription),
      BillingSubscriptionState::Canceled
    );
  }

  #[test]
  fn billing_date_label_is_renewal_for_active_and_trialing() {
    let mut active = make_subscription();
    active.status = CustomerStateSubscriptionStatus::Active;
    active.cancel_at_period_end = false;
    assert_eq!(BillingPage::billing_date_label(&active), "Renewal Date");

    let mut trialing = make_subscription();
    trialing.status = CustomerStateSubscriptionStatus::Trialing;
    trialing.cancel_at_period_end = false;
    assert_eq!(BillingPage::billing_date_label(&trialing), "Renewal Date");
  }

  #[test]
  fn billing_date_label_is_expiry_for_cancellation_states() {
    let mut to_be_canceled = make_subscription();
    to_be_canceled.cancel_at_period_end = true;
    to_be_canceled.current_period_end = Some("2099-01-01T00:00:00Z".to_string());
    assert_eq!(
      BillingPage::billing_date_label(&to_be_canceled),
      "Expiry Date"
    );

    let mut canceled = make_subscription();
    canceled.cancel_at_period_end = true;
    canceled.current_period_end = Some("2000-01-01T00:00:00Z".to_string());
    assert_eq!(BillingPage::billing_date_label(&canceled), "Expiry Date");
  }

  #[test]
  fn format_datetime_uses_long_month_date_format() {
    assert_eq!(
      BillingPage::format_datetime(Some("2026-02-20T15:42:30Z")).to_string(),
      "February 20, 2026"
    );
    assert_eq!(
      BillingPage::format_datetime(Some("2026-02-20")).to_string(),
      "February 20, 2026"
    );
  }
}
