use std::sync::Arc;

use editor::set_indent_rainbow_enabled;
use gpui::{
  App, Context, FocusHandle, Focusable, Render, SharedString, Window, div, prelude::*, px,
};

use gpui_component::{
  ActiveTheme as _, IconName, Sizable, Size, Theme, ThemeMode,
  button::{Button, ButtonVariants},
  setting::{SettingField, SettingGroup, SettingItem, SettingPage, Settings},
};

use ui::{
  CommandPalette, CommandPaletteAction, CommandPaletteCommand, CommandPaletteConfig,
  CommandPaletteHandler, CommandPalettePage, PAGE_HEADER_HEIGHT, WindowExt,
};

use crate::{
  CloseWorkspacePage, ShowCommandPalette,
  auth_state::{AuthState, AuthStateStore},
  config::{AppSettings as PersistedSettings, ConfigStore},
  github_navigation::{open_pr_target, open_repo_target},
  github_page::GithubPageHandle,
  navigation::NavigationHistory,
};

pub struct SettingsPage {
  focus_handle: FocusHandle,
  auto_switch_theme: bool,
  indent_rainbow: bool,
  size: Size,
}

impl SettingsPage {
  pub fn new(_: &mut Window, cx: &mut Context<Self>, settings: PersistedSettings) -> Self {
    Self {
      focus_handle: cx.focus_handle(),
      auto_switch_theme: settings.auto_switch_theme,
      indent_rainbow: settings.indent_rainbow,
      size: Size::default(),
    }
  }

  pub(crate) fn auto_switch_theme_enabled(&self) -> bool {
    self.auto_switch_theme
  }

  pub(crate) fn indent_rainbow_enabled(&self) -> bool {
    self.indent_rainbow
  }

  fn setting_pages(&self, _: &mut Window, cx: &mut Context<Self>) -> Vec<SettingPage> {
    let view = cx.entity();
    let default_auto = self.auto_switch_theme;
    let default_indent_rainbow = self.indent_rainbow;

    vec![SettingPage::new("General").default_open(true).groups(vec![
      SettingGroup::new().title("Appearance").items(vec![
        SettingItem::new(
          "Dark Mode",
          SettingField::switch(
            |cx: &App| cx.theme().mode.is_dark(),
            {
              let view = view.clone();
              move |val: bool, cx: &mut App| {
                let settings = view.read(cx);
                ConfigStore::persist_app_settings(PersistedSettings {
                  auto_switch_theme: settings.auto_switch_theme,
                  dark_mode: val,
                  indent_rainbow: settings.indent_rainbow,
                });

                let mode = if val {
                  ThemeMode::Dark
                } else {
                  ThemeMode::Light
                };
                Theme::change(mode, None, cx);
                cx.refresh_windows();
              }
            },
          )
          .default_value(false),
        )
        .description("Switch between light and dark themes."),
        SettingItem::new(
          "Auto Switch Theme",
          SettingField::checkbox(
            {
              let view = view.clone();
              move |cx: &App| view.read(cx).auto_switch_theme
            },
            {
              let view = view.clone();
              move |val: bool, cx: &mut App| {
                view.update(cx, |view, _| {
                  view.auto_switch_theme = val;
                });
                if val {
                  Theme::sync_system_appearance(None, cx);
                }

                ConfigStore::persist_app_settings(PersistedSettings {
                  auto_switch_theme: val,
                  dark_mode: cx.theme().mode.is_dark(),
                  indent_rainbow: view.read(cx).indent_rainbow,
                });

                cx.refresh_windows();
              }
            },
          )
          .default_value(default_auto),
        )
        .description("Automatically switch theme based on system settings."),
        SettingItem::new(
          "Indent Rainbow",
          SettingField::checkbox(
            {
              let view = view.clone();
              move |cx: &App| view.read(cx).indent_rainbow
            },
            {
              let view = view.clone();
              move |val: bool, cx: &mut App| {
                view.update(cx, |view, _| {
                  view.indent_rainbow = val;
                });

                set_indent_rainbow_enabled(val);
                ConfigStore::persist_app_settings(PersistedSettings {
                  auto_switch_theme: view.read(cx).auto_switch_theme,
                  dark_mode: cx.theme().mode.is_dark(),
                  indent_rainbow: val,
                });

                cx.refresh_windows();
              }
            },
          )
          .default_value(default_indent_rainbow),
        )
        .description("Color indentation guides by level in the editor."),
      ]),
    ])]
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
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    NavigationHistory::navigate_back(cx);
  }

  fn open_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let include_github = matches!(AuthStateStore::get(cx), AuthState::Authenticated(_));
    let commands =
      CommandPaletteCommand::default_global_commands(CommandPalettePage::Settings, include_github);

    let view = cx.entity();
    let handler: CommandPaletteHandler = Arc::new(move |action, window, cx| {
      view.update(cx, |view, cx| {
        view.handle_command_palette_action(action, window, cx)
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
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    match action {
      CommandPaletteAction::OpenGitPage => {
        NavigationHistory::navigate("/git", cx);
        Ok(())
      }
      CommandPaletteAction::OpenGithubPage => {
        GithubPageHandle::refresh(cx);
        NavigationHistory::navigate("/github", cx);
        Ok(())
      }
      CommandPaletteAction::OpenGithubPrDetails {
        owner,
        repo,
        number,
        open_changes_tab,
        review_comment_id,
      } => {
        open_pr_target(
          owner,
          repo,
          number,
          open_changes_tab,
          review_comment_id,
          None,
          cx,
        );
        Ok(())
      }
      CommandPaletteAction::OpenGithubRepoDetails {
        owner,
        repo,
        tab,
        issue_number,
        issue_comment_id,
      } => {
        open_repo_target(owner, repo, tab, issue_number, issue_comment_id, cx);
        Ok(())
      }
      CommandPaletteAction::OpenSettingsPage => Ok(()),
      CommandPaletteAction::OpenBillingPage => {
        NavigationHistory::navigate("/billing", cx);
        Ok(())
      }
      CommandPaletteAction::OpenAboutPage => {
        NavigationHistory::navigate("/about", cx);
        Ok(())
      }
      CommandPaletteAction::OpenGitConfigPage => {
        NavigationHistory::navigate("/git-config", cx);
        Ok(())
      }
      CommandPaletteAction::SendFeedback => {
        crate::feedback_dialog::open_feedback_dialog(window, cx);
        Ok(())
      }
      _ => Err("Command not available.".into()),
    }
  }
}

impl Render for SettingsPage {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    let header = div()
      .h(px(PAGE_HEADER_HEIGHT))
      .max_h(px(PAGE_HEADER_HEIGHT))
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
          .child("Settings"),
      )
      .child(
        Button::new("close-settings")
          .icon(IconName::Close)
          .ghost()
          .compact()
          .tooltip("Close settings")
          .on_click(|_, _, cx| {
            NavigationHistory::navigate_back(cx);
          }),
      );

    div()
      .size_full()
      .flex()
      .flex_col()
      .bg(theme.background)
      .track_focus(&self.focus_handle(cx))
      .on_action(cx.listener(SettingsPage::show_command_palette_action))
      .on_action(cx.listener(SettingsPage::close_workspace_page_action))
      .child(header)
      .child(
        Settings::new("app-settings")
          .with_size(self.size)
          .pages(self.setting_pages(window, cx)),
      )
  }
}

impl Focusable for SettingsPage {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}
