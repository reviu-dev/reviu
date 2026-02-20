use std::sync::Arc;

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
  api::{ApiClient, CustomerStateSubscription},
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

    let Some((date, time)) = value.split_once('T') else {
      return value.to_string().into();
    };

    let time = time.split('Z').next().unwrap_or(time);
    let time = if time.len() >= 5 { &time[..5] } else { time };

    if time.is_empty() {
      date.to_string().into()
    } else {
      format!("{date} {time}").into()
    }
  }

  fn title_case(value: &str) -> SharedString {
    if value.is_empty() {
      return "—".into();
    }

    let mut chars = value.chars();
    let Some(first) = chars.next() else {
      return "—".into();
    };
    format!("{}{}", first.to_ascii_uppercase(), chars.as_str()).into()
  }

  fn format_amount(amount_cents: i64, currency: &str) -> SharedString {
    let amount = amount_cents as f64 / 100.0;
    let currency = currency.to_uppercase();
    if currency == "USD" {
      return format!("${amount:.2}").into();
    }
    format!("{amount:.2} {currency}").into()
  }

  fn status_color(status: &str, theme: &gpui_component::Theme) -> gpui::Hsla {
    match status.to_ascii_lowercase().as_str() {
      "active" => theme.status_green(),
      "trialing" => theme.status_violet(),
      "canceled" => theme.status_red(),
      "past_due" | "unpaid" | "incomplete" => theme.status_orange(),
      _ => theme.muted_foreground,
    }
  }

  fn render_active_subscription(
    &self,
    subscription: &CustomerStateSubscription,
    portal_url: Option<&str>,
    theme: &gpui_component::Theme,
  ) -> impl IntoElement {
    let status_color = Self::status_color(subscription.status.as_str(), theme);
    let amount = Self::format_amount(subscription.amount, subscription.currency.as_str());
    let plan = format!(
      "{}/{}",
      amount,
      subscription.recurring_interval.to_lowercase()
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
              .child(Self::title_case(subscription.status.as_str())),
          ),
      )
      .child(row("Subscription ID", subscription.id.clone().into()))
      .child(row("Product ID", subscription.product_id.clone().into()))
      .child(row(
        "Current period start",
        Self::format_datetime(Some(subscription.current_period_start.as_str())),
      ))
      .child(row(
        "Current period end",
        Self::format_datetime(subscription.current_period_end.as_deref()),
      ))
      .child(row(
        "Trial start",
        Self::format_datetime(subscription.trial_start.as_deref()),
      ))
      .child(row(
        "Trial end",
        Self::format_datetime(subscription.trial_end.as_deref()),
      ))
      .child(row(
        "Cancel at period end",
        if subscription.cancel_at_period_end {
          "Yes".into()
        } else {
          "No".into()
        },
      ))
      .child(row(
        "Canceled at",
        Self::format_datetime(subscription.canceled_at.as_deref()),
      ))
      .child(row(
        "Started at",
        Self::format_datetime(subscription.started_at.as_deref()),
      ))
      .child(row(
        "Ends at",
        Self::format_datetime(subscription.ends_at.as_deref()),
      ))
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
