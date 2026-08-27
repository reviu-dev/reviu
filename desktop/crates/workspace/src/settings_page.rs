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
  CommandPaletteHandler, CommandPalettePage, PAGE_HEADER_HEIGHT, StatusThemeExt,
};

use crate::{
  CloseWorkspacePage, ShowCommandPalette,
  auth_state::AuthStateStore,
  config::AppSettings as PersistedSettings,
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
  hide_whitespace: bool,
  menu_bar_icon: bool,
  analytics_enabled: bool,
  shortcut_recording: Option<ShortcutId>,
  shortcut_error: Option<ShortcutCaptureError>,
  size: Size,
  _subscriptions: Vec<Subscription>,
}

impl SettingsPage {
  pub fn new(_window: &mut Window, cx: &mut Context<Self>, settings: PersistedSettings) -> Self {
    let view = cx.entity();
    let shortcut_capture_subscription = cx.intercept_keystrokes(move |event, window, cx| {
      view.update(cx, |view, cx| {
        view.handle_shortcut_capture(event, window, cx);
      });
    });
    Self {
      focus_handle: cx.focus_handle(),
      auto_switch_theme: settings.auto_switch_theme,
      indent_rainbow: settings.indent_rainbow,
      git_unified_file_view: settings.git_unified_file_view,
      split_diff_view: settings.split_diff_view,
      hide_whitespace: settings.hide_whitespace,
      menu_bar_icon: settings.menu_bar_icon,
      analytics_enabled: settings.analytics_enabled,
      shortcut_recording: None,
      shortcut_error: None,
      size: Size::default(),
      _subscriptions: vec![shortcut_capture_subscription],
    }
  }

  pub(crate) fn auto_switch_theme_enabled(&self) -> bool {
    self.auto_switch_theme
  }

  fn setting_pages(&self, window: &mut Window, cx: &mut Context<Self>) -> Vec<SettingPage> {
    let view = cx.entity();
    let default_auto = self.auto_switch_theme;
    let default_indent_rainbow = self.indent_rainbow;
    let default_git_unified_file_view = self.git_unified_file_view;
    let default_split_diff_view = self.split_diff_view;
    let default_menu_bar_icon = self.menu_bar_icon;
    let default_analytics_enabled = self.analytics_enabled;

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
          .description("Use side-by-side diff view instead of inline by default."),
          SettingItem::new(
            "Hide Whitespace",
            SettingField::checkbox(
              {
                let view = view.clone();
                move |cx: &App| view.read(cx).hide_whitespace
              },
              {
                let view = view.clone();
                move |val: bool, cx: &mut App| {
                  view.update(cx, |view, _| {
                    view.hide_whitespace = val;
                  });

                  PersistedSettings::update(cx, |s| s.hide_whitespace = val);
                  cx.refresh_windows();
                }
              },
            )
            .default_value(self.hide_whitespace),
          )
          .description("Hide whitespace changes in diffs by default."),
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
        SettingGroup::new().title("Agent").items(vec![
          SettingItem::new(
            "Notify When The Agent Needs You",
            SettingField::checkbox(
              move |cx: &App| PersistedSettings::get(cx).agent_notifications,
              move |val: bool, cx: &mut App| {
                PersistedSettings::update(cx, |s| s.agent_notifications = val);
              },
            )
            .default_value(true),
          )
          .description(
            "Show a popup when a turn finishes or a permission is asked while the window is inactive.",
          ),
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
        SettingGroup::new().title("Privacy").items(vec![
          SettingItem::new(
            "Send Anonymous Usage Data",
            SettingField::checkbox(
              {
                let view = view.clone();
                move |cx: &App| view.read(cx).analytics_enabled
              },
              {
                let view = view.clone();
                move |val: bool, cx: &mut App| {
                  view.update(cx, |view, _| {
                    view.analytics_enabled = val;
                  });

                  PersistedSettings::update(cx, |s| s.analytics_enabled = val);
                }
              },
            )
            .default_value(default_analytics_enabled),
          )
          .description(
            "Help improve Reviu by sending anonymous feature usage events (no repository, file, or account data). See the privacy policy on reviu.dev for details.",
          ),
        ]),
      ].into_iter().chain(self.menu_bar_settings_groups(view.clone(), default_menu_bar_icon))),
      Self::keyboard_shortcuts_page(view.clone(), window, cx),
    ]
  }

  fn menu_bar_settings_groups(
    &self,
    view: gpui::Entity<Self>,
    default_menu_bar_icon: bool,
  ) -> Vec<SettingGroup> {
    #[cfg(target_os = "macos")]
    let title = "Menu Bar";
    #[cfg(not(target_os = "macos"))]
    let title = "System Tray";
    #[cfg(target_os = "macos")]
    let label = "Show in Menu Bar";
    #[cfg(not(target_os = "macos"))]
    let label = "Show in System Tray";
    #[cfg(target_os = "macos")]
    let description =
      "Show the Reviu icon in the macOS menu bar with unread GitHub notification counts.";
    #[cfg(not(target_os = "macos"))]
    let description =
      "Show the Reviu icon in the system tray with unread GitHub notification counts.";

    vec![SettingGroup::new().title(title).items(vec![
        SettingItem::new(
          label,
          SettingField::checkbox(
            {
              let view = view.clone();
              move |cx: &App| view.read(cx).menu_bar_icon
            },
            {
              let view = view.clone();
              move |val: bool, cx: &mut App| {
                view.update(cx, |view, _| {
                  view.menu_bar_icon = val;
                });

                PersistedSettings::update(cx, |s| s.menu_bar_icon = val);
                crate::status_bar::set_status_bar_enabled(
                  val,
                  crate::workspace::STATUS_BAR_ICON_PNG,
                );
              }
            },
          )
          .default_value(default_menu_bar_icon),
        )
        .description(description),
      ])]
  }

  fn keyboard_shortcuts_page(view: gpui::Entity<Self>, window: &Window, cx: &App) -> SettingPage {
    SettingPage::new("Keyboard Shortcuts")
      .description(
        "Edit desktop shortcuts grouped by workflow. Use the search field to filter by action or key combo. Changes apply immediately in the app.",
      )
      .resettable(false)
      .groups([
        Self::keyboard_shortcuts_group(ShortcutCategory::Core, view.clone(), window, cx),
        Self::keyboard_shortcuts_group(ShortcutCategory::Review, view.clone(), window, cx),
        Self::keyboard_shortcuts_group(ShortcutCategory::LocalGit, view.clone(), window, cx),
        Self::keyboard_shortcuts_group(ShortcutCategory::App, view, window, cx),
      ])
  }

  fn keyboard_shortcuts_group(
    category: ShortcutCategory,
    view: gpui::Entity<Self>,
    window: &Window,
    cx: &App,
  ) -> SettingGroup {
    SettingGroup::new().title(category.title()).items(
      shortcut_definitions()
        .iter()
        .copied()
        .filter(move |definition| definition.category == category)
        .map(move |definition| Self::keyboard_shortcut_item(view.clone(), definition, window, cx)),
    )
  }

  fn keyboard_shortcut_item(
    view: gpui::Entity<Self>,
    definition: ShortcutDefinition,
    window: &Window,
    cx: &App,
  ) -> SettingItem {
    let edit_button_id = format!("settings-shortcut-edit-{}", definition.id.storage_key());
    let cancel_button_id = format!("settings-shortcut-cancel-{}", definition.id.storage_key());
    let reset_button_id = format!("settings-shortcut-reset-{}", definition.id.storage_key());
    let shortcut_keystroke = Self::searchable_shortcut_keystroke(definition.id, window, cx);

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
    .description(Self::keyboard_shortcut_description(
      definition,
      &shortcut_keystroke,
    ))
  }

  fn keyboard_shortcut_description(
    definition: ShortcutDefinition,
    shortcut_keystroke: &str,
  ) -> String {
    format!(
      "{} Shortcut: {}. Available on {}.",
      definition.description, shortcut_keystroke, definition.scope_label
    )
  }

  fn searchable_shortcut_keystroke(shortcut_id: ShortcutId, window: &Window, cx: &App) -> String {
    let keystroke = resolved_display_shortcut_keystroke_in(cx, window, shortcut_id);
    let mut parts = Vec::new();

    if keystroke.modifiers.control {
      parts.push("ctrl".to_string());
    }
    if keystroke.modifiers.alt {
      parts.push("alt".to_string());
    }
    if keystroke.modifiers.shift {
      parts.push("shift".to_string());
    }
    if keystroke.modifiers.platform {
      parts.push("cmd".to_string());
    }
    if keystroke.modifiers.function {
      parts.push("fn".to_string());
    }

    parts.push(keystroke.key);
    parts.join("-")
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
    cx.set_menus(crate::build_app_menus());
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
    let signed_in = AuthStateStore::is_signed_in(cx);
    let commands = crate::shortcuts::with_palette_keybindings(
      CommandPaletteCommand::default_global_commands(ui::GlobalCommandsContext {
        current_page: CommandPalettePage::Settings,
        include_github,
        signed_in,
        has_subscription: AuthStateStore::has_subscription(cx),
      }),
      window,
      cx,
    );

    let view = cx.entity();
    let handler: CommandPaletteHandler = Arc::new(move |action, window, cx| {
      view.update(cx, |view, cx| {
        view.handle_command_palette_action(action, window, cx)
      })
    });

    let config = CommandPaletteConfig::new(Vec::new(), commands, handler);
    let palette = cx.new(|cx| CommandPalette::new(window, cx, config));
    ui::open_palette_dialog(palette, window, cx);
  }

  fn handle_command_palette_action(
    &mut self,
    action: CommandPaletteAction,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Result<(), SharedString> {
    crate::palette_actions::handle_global_command_palette_action(action, window, cx)
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
  fn keyboard_shortcut_descriptions_include_scope_and_keystroke() {
    let description = SettingsPage::keyboard_shortcut_description(
      *shortcut_definitions()
        .iter()
        .find(|definition| definition.id == ShortcutId::ShowFileSearch)
        .expect("file search shortcut"),
      "cmd-p",
    );

    assert!(description.contains("Open file search"));
    assert!(description.contains("Shortcut: cmd-p."));
    assert!(description.contains("PR Changes and Sessions"));
  }
}
