use std::sync::Arc;

use editor::set_indent_rainbow_enabled;
use gpui::{
  App, Context, FocusHandle, Focusable, Keystroke, KeystrokeEvent, Render, SharedString,
  Subscription, Window, div, prelude::*, px,
};

use gpui_component::{
  ActiveTheme as _, IconName, Sizable, Size, Theme, ThemeMode,
  button::{Button, ButtonVariants},
  kbd::Kbd,
  setting::{NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage, Settings},
};

use ui::{
  CommandPalette, CommandPaletteAction, CommandPaletteCommand, CommandPaletteConfig,
  CommandPaletteHandler, CommandPalettePage, PAGE_HEADER_HEIGHT, StatusThemeExt, WindowExt,
};

use crate::{
  CloseWorkspacePage, ShowCommandPalette,
  auth_state::AuthStateStore,
  config::AppSettings as PersistedSettings,
  github_navigation::{open_pr_target, open_repo_target},
  github_page::GithubPageHandle,
  navigation::NavigationHistory,
  shortcuts::{
    self, ShortcutCategory, ShortcutDefinition, ShortcutId, ShortcutOverrides,
    resolved_display_shortcut_keystroke_in, shortcut_definitions,
  },
};

#[derive(Clone)]
struct ShortcutCaptureError {
  shortcut_id: ShortcutId,
  message: SharedString,
}

pub struct SettingsPage {
  focus_handle: FocusHandle,
  auto_switch_theme: bool,
  indent_rainbow: bool,
  git_unified_file_view: bool,
  split_diff_view: bool,
  shortcut_recording: Option<ShortcutId>,
  shortcut_error: Option<ShortcutCaptureError>,
  size: Size,
  _subscriptions: Vec<Subscription>,
}

impl SettingsPage {
  pub fn new(_: &mut Window, cx: &mut Context<Self>, settings: PersistedSettings) -> Self {
    let view = cx.entity();
    let shortcut_capture_subscription = cx.intercept_keystrokes(move |event, window, cx| {
      let _ = view.update(cx, |view, cx| {
        view.handle_shortcut_capture(event, window, cx);
      });
    });

    Self {
      focus_handle: cx.focus_handle(),
      auto_switch_theme: settings.auto_switch_theme,
      indent_rainbow: settings.indent_rainbow,
      git_unified_file_view: settings.git_unified_file_view,
      split_diff_view: settings.split_diff_view,
      shortcut_recording: None,
      shortcut_error: None,
      size: Size::default(),
      _subscriptions: vec![shortcut_capture_subscription],
    }
  }

  pub(crate) fn auto_switch_theme_enabled(&self) -> bool {
    self.auto_switch_theme
  }

  fn setting_pages(&self, _: &mut Window, cx: &mut Context<Self>) -> Vec<SettingPage> {
    let view = cx.entity();
    let default_auto = self.auto_switch_theme;
    let default_indent_rainbow = self.indent_rainbow;
    let default_git_unified_file_view = self.git_unified_file_view;
    let default_split_diff_view = self.split_diff_view;

    vec![
      SettingPage::new("General").default_open(true).groups(vec![
        SettingGroup::new().title("Appearance").items(vec![
          SettingItem::new(
            "Dark Mode",
            SettingField::switch(|cx: &App| cx.theme().mode.is_dark(), {
              move |val: bool, cx: &mut App| {
                PersistedSettings::update(cx, |s| s.dark_mode = val);

                let mode = if val {
                  ThemeMode::Dark
                } else {
                  ThemeMode::Light
                };
                Theme::change(mode, None, cx);
                cx.refresh_windows();
              }
            })
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

                  PersistedSettings::update(cx, |s| s.auto_switch_theme = val);
                  cx.refresh_windows();
                }
              },
            )
            .default_value(default_auto),
          )
          .description("Automatically switch theme based on system settings."),
          SettingItem::new(
            "Font Size",
            SettingField::number_input(
              NumberFieldOptions {
                min: 12.0,
                max: 24.0,
                step: 1.0,
              },
              |cx: &App| f64::from(cx.theme().font_size),
              {
                move |val: f64, cx: &mut App| {
                  Theme::global_mut(cx).font_size = px(val as f32);
                  PersistedSettings::update(cx, |s| s.font_size = val as f32);
                  cx.refresh_windows();
                }
              },
            )
            .default_value(16.0),
          )
          .description("Base font size for the application (12–24px)."),
        ]),
        SettingGroup::new().title("Editor").items(vec![
          SettingItem::new(
            "Split Diff View",
            SettingField::checkbox(
              {
                let view = view.clone();
                move |cx: &App| view.read(cx).split_diff_view
              },
              {
                let view = view.clone();
                move |val: bool, cx: &mut App| {
                  view.update(cx, |view, _| {
                    view.split_diff_view = val;
                  });

                  PersistedSettings::update(cx, |s| s.split_diff_view = val);
                  cx.refresh_windows();
                }
              },
            )
            .default_value(default_split_diff_view),
          )
          .description("Use side-by-side diff view instead of inline."),
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
                  PersistedSettings::update(cx, |s| s.indent_rainbow = val);
                  cx.refresh_windows();
                }
              },
            )
            .default_value(default_indent_rainbow),
          )
          .description("Color indentation guides by level in the editor."),
        ]),
        SettingGroup::new().title("Git").items(vec![
          SettingItem::new(
            "Unified File View",
            SettingField::checkbox(
              {
                let view = view.clone();
                move |cx: &App| view.read(cx).git_unified_file_view
              },
              {
                let view = view.clone();
                move |val: bool, cx: &mut App| {
                  view.update(cx, |view, _| {
                    view.git_unified_file_view = val;
                  });

                  PersistedSettings::update(cx, |s| s.git_unified_file_view = val);
                  cx.refresh_windows();
                }
              },
            )
            .default_value(default_git_unified_file_view),
          )
          .description(
            "Show all changed files in a single list instead of separate staged/unstaged groups.",
          ),
        ]),
      ]),
      Self::keyboard_shortcuts_page(view.clone()),
    ]
  }

  fn keyboard_shortcuts_page(view: gpui::Entity<Self>) -> SettingPage {
    SettingPage::new("Keyboard Shortcuts")
      .description("Edit current desktop shortcuts. Changes apply immediately in the app.")
      .resettable(false)
      .groups([
        Self::keyboard_shortcuts_group(ShortcutCategory::Workspace, view.clone()),
        Self::keyboard_shortcuts_group(ShortcutCategory::Search, view.clone()),
        Self::keyboard_shortcuts_group(ShortcutCategory::Git, view),
      ])
  }

  fn keyboard_shortcuts_group(
    category: ShortcutCategory,
    view: gpui::Entity<Self>,
  ) -> SettingGroup {
    SettingGroup::new().title(category.title()).items(
      shortcut_definitions()
        .iter()
        .copied()
        .filter(move |definition| definition.category == category)
        .map(move |definition| Self::keyboard_shortcut_item(view.clone(), definition)),
    )
  }

  fn keyboard_shortcut_item(
    view: gpui::Entity<Self>,
    definition: ShortcutDefinition,
  ) -> SettingItem {
    let edit_button_id = format!("settings-shortcut-edit-{}", definition.id.storage_key());
    let cancel_button_id = format!("settings-shortcut-cancel-{}", definition.id.storage_key());
    let reset_button_id = format!("settings-shortcut-reset-{}", definition.id.storage_key());

    SettingItem::new(
      definition.title,
      SettingField::render(move |_, window, cx| {
        let (is_recording, error_message) = {
          let view = view.read(cx);
          (
            view.is_recording_shortcut(definition.id),
            view.shortcut_error_message(definition.id),
          )
        };
        let is_customized = shortcuts::shortcut_is_customized(cx, definition.id);

        let shortcut_actions = div()
          .flex()
          .items_center()
          .justify_end()
          .gap_2()
          .child(Kbd::new(resolved_display_shortcut_keystroke_in(
            cx,
            window,
            definition.id,
          )))
          .child(
            Button::new(if is_recording {
              cancel_button_id.clone()
            } else {
              edit_button_id.clone()
            })
            .small()
            .outline()
            .label(if is_recording { "Recording..." } else { "Edit" })
            .on_click({
              let view = view.clone();
              move |_, window, cx| {
                view.update(cx, |view, cx| {
                  if view.is_recording_shortcut(definition.id) {
                    view.cancel_shortcut_recording(cx);
                  } else {
                    view.start_shortcut_recording(definition.id, window, cx);
                  }
                });
              }
            }),
          )
          .when(is_customized && !is_recording, |this| {
            this.child(
              Button::new(reset_button_id.clone())
                .small()
                .ghost()
                .label("Reset")
                .on_click({
                  let view = view.clone();
                  move |_, _, cx| {
                    view.update(cx, |view, cx| {
                      view.reset_shortcut_override(definition.id, cx);
                    });
                  }
                }),
            )
          });

        div()
          .flex()
          .flex_col()
          .items_end()
          .gap_1()
          .child(shortcut_actions)
          .when(is_recording, |this| {
            this.child(
              div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Press a new shortcut. Esc cancels."),
            )
          })
          .when_some(error_message, |this, message| {
            this.child(
              div()
                .text_xs()
                .text_color(cx.theme().status_red())
                .child(message),
            )
          })
      }),
    )
    .description(Self::keyboard_shortcut_description(definition))
  }

  fn keyboard_shortcut_description(definition: ShortcutDefinition) -> String {
    format!(
      "{} Available on {}.",
      definition.description, definition.scope_label
    )
  }

  fn start_shortcut_recording(
    &mut self,
    shortcut_id: ShortcutId,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.shortcut_recording = Some(shortcut_id);
    self.shortcut_error = None;
    window.focus(&self.focus_handle(cx), cx);
    cx.notify();
  }

  fn cancel_shortcut_recording(&mut self, cx: &mut Context<Self>) {
    self.shortcut_recording = None;
    self.shortcut_error = None;
    cx.notify();
  }

  fn reset_shortcut_override(&mut self, shortcut_id: ShortcutId, cx: &mut Context<Self>) {
    shortcuts::clear_shortcut_override(cx, shortcut_id);
    self.shortcut_recording = self
      .shortcut_recording
      .filter(|current| *current != shortcut_id);
    self.shortcut_error = None;
    self.rebuild_app_key_bindings(cx);
    cx.notify();
  }

  fn handle_shortcut_capture(
    &mut self,
    event: &KeystrokeEvent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(shortcut_id) = self.shortcut_recording else {
      return;
    };
    if !self.focus_handle.contains_focused(window, cx) {
      return;
    }

    window.prevent_default();
    cx.stop_propagation();

    if event.keystroke.key == "escape" {
      self.cancel_shortcut_recording(cx);
      return;
    }

    if Self::is_modifier_only_keystroke(&event.keystroke) {
      return;
    }

    let overrides = ShortcutOverrides::get(cx);
    match shortcuts::validate_shortcut_override(shortcut_id, &event.keystroke, &overrides) {
      Ok(()) => {
        shortcuts::set_shortcut_override(cx, shortcut_id, &event.keystroke);
        self.shortcut_recording = None;
        self.shortcut_error = None;
        self.rebuild_app_key_bindings(cx);
        cx.notify();
      }
      Err(error) => {
        self.shortcut_error = Some(ShortcutCaptureError {
          shortcut_id,
          message: error.message().into(),
        });
        cx.notify();
      }
    }
  }

  fn rebuild_app_key_bindings(&self, cx: &mut Context<Self>) {
    crate::install_app_key_bindings(cx);
    cx.set_menus(crate::build_app_menus(
      AuthStateStore::should_show_billing_entry(cx),
    ));
  }

  fn is_recording_shortcut(&self, shortcut_id: ShortcutId) -> bool {
    self.shortcut_recording == Some(shortcut_id)
  }

  fn shortcut_error_message(&self, shortcut_id: ShortcutId) -> Option<SharedString> {
    self
      .shortcut_error
      .as_ref()
      .filter(|error| error.shortcut_id == shortcut_id)
      .map(|error| error.message.clone())
  }

  fn is_modifier_only_keystroke(keystroke: &Keystroke) -> bool {
    matches!(
      keystroke.key.as_str(),
      "shift" | "control" | "alt" | "platform" | "function"
    )
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
    let include_github = AuthStateStore::has_github_access(cx);
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
        .on_ok(|_, _, _| false)
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
      .when(self.shortcut_recording.is_some(), |this| {
        this.key_context(shortcuts::WORKSPACE_SHORTCUT_RECORDING_CONTEXT)
      })
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

#[cfg(test)]
mod tests {
  use super::*;
  use crate::shortcuts::ShortcutId;

  #[test]
  fn keyboard_shortcut_descriptions_include_scope() {
    let description = SettingsPage::keyboard_shortcut_description(
      *shortcut_definitions()
        .iter()
        .find(|definition| definition.id == ShortcutId::ShowFileSearch)
        .expect("file search shortcut"),
    );

    assert!(description.contains("Open file search"));
    assert!(description.contains("Git, Repo Code, and PR Changes pages"));
  }
}
