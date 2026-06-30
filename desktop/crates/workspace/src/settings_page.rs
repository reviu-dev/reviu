use std::sync::Arc;

use editor::set_indent_rainbow_enabled;
use gpui::{
  AnyWindowHandle, App, Context, FocusHandle, Focusable, Keystroke, KeystrokeEvent, Render,
  SharedString, Subscription, Task, Window, div, prelude::*, px,
};

use gpui_component::{
  ActiveTheme as _, Disableable, IconName, IndexPath, Selectable, Sizable, Size, Theme, ThemeMode,
  button::{Button, ButtonVariants},
  h_flex,
  kbd::Kbd,
  select::{Select, SelectState},
  setting::{NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage, Settings},
  v_flex,
};
use smol::unblock;

use ui::{
  CommandPalette, CommandPaletteAction, CommandPaletteCommand, CommandPaletteConfig,
  CommandPaletteHandler, CommandPalettePage, Input, InputState, PAGE_HEADER_HEIGHT, StatusThemeExt,
  WindowExt,
};

use crate::{
  CloseWorkspacePage, ShowCommandPalette,
  api::{AiProvider, AiSettings, ApiClient},
  auth_state::{AuthState, AuthStateStore},
  config::{AppSettings as PersistedSettings, CloneProtocol},
  github_navigation::{open_commit_target, open_pr_target, open_profile_target, open_repo_target},
  github_page::GithubPageHandle,
  github_shared,
  navigation::NavigationHistory,
  shortcuts::{
    self, ShortcutCategory, ShortcutDefinition, ShortcutId, ShortcutOverrides,
    resolved_display_shortcut_keystroke_in, shortcut_definitions,
  },
  workspace::WorkspaceApi,
};

#[derive(Clone)]
struct ShortcutCaptureError {
  shortcut_id: ShortcutId,
  message: SharedString,
}

pub struct SettingsPage {
  focus_handle: FocusHandle,
  window_handle: AnyWindowHandle,
  auto_switch_theme: bool,
  indent_rainbow: bool,
  git_unified_file_view: bool,
  split_diff_view: bool,
  hide_whitespace: bool,
  clone_protocol: CloneProtocol,
  menu_bar_icon: bool,
  analytics_enabled: bool,
  shortcut_recording: Option<ShortcutId>,
  shortcut_error: Option<ShortcutCaptureError>,
  api: ApiClient,
  ai_provider: AiProvider,
  ai_model_select: gpui::Entity<SelectState<Vec<SharedString>>>,
  ai_api_key_input: gpui::Entity<InputState>,
  ai_configured: bool,
  ai_api_key_hint: Option<SharedString>,
  ai_settings_loading: bool,
  ai_settings_saving: bool,
  ai_settings_deleting: bool,
  ai_settings_error: Option<SharedString>,
  ai_settings_notice: Option<SharedString>,
  ai_settings_task: Option<Task<()>>,
  ai_settings_loaded: bool,
  size: Size,
  _subscriptions: Vec<Subscription>,
}

impl SettingsPage {
  pub fn new(window: &mut Window, cx: &mut Context<Self>, settings: PersistedSettings) -> Self {
    let view = cx.entity();
    let shortcut_capture_subscription = cx.intercept_keystrokes(move |event, window, cx| {
      view.update(cx, |view, cx| {
        view.handle_shortcut_capture(event, window, cx);
      });
    });
    let initial_provider = AiProvider::Openai;
    let ai_model_select = cx.new(|cx| {
      SelectState::new(
        provider_model_items(initial_provider),
        provider_default_model_index(initial_provider),
        window,
        cx,
      )
    });
    let ai_api_key_input = cx.new(|cx| {
      InputState::new(window, cx)
        .placeholder("API key")
        .masked(true)
    });

    let mut this = Self {
      focus_handle: cx.focus_handle(),
      window_handle: window.window_handle(),
      auto_switch_theme: settings.auto_switch_theme,
      indent_rainbow: settings.indent_rainbow,
      git_unified_file_view: settings.git_unified_file_view,
      split_diff_view: settings.split_diff_view,
      hide_whitespace: settings.hide_whitespace,
      clone_protocol: settings.clone_protocol,
      menu_bar_icon: settings.menu_bar_icon,
      analytics_enabled: settings.analytics_enabled,
      shortcut_recording: None,
      shortcut_error: None,
      api: WorkspaceApi::global(cx).api.clone(),
      ai_provider: initial_provider,
      ai_model_select,
      ai_api_key_input,
      ai_configured: false,
      ai_api_key_hint: None,
      ai_settings_loading: false,
      ai_settings_saving: false,
      ai_settings_deleting: false,
      ai_settings_error: None,
      ai_settings_notice: None,
      ai_settings_task: None,
      ai_settings_loaded: false,
      size: Size::default(),
      _subscriptions: vec![shortcut_capture_subscription],
    };
    let auth_subscription = cx.observe_global::<AuthStateStore>(|this, cx| {
      this.load_ai_settings_if_authenticated(cx);
    });
    this._subscriptions.push(auth_subscription);
    this.load_ai_settings_if_authenticated(cx);
    this
  }

  pub(crate) fn auto_switch_theme_enabled(&self) -> bool {
    self.auto_switch_theme
  }

  fn apply_ai_settings(&mut self, settings: AiSettings, window: &mut Window, cx: &mut App) {
    self.ai_configured = settings.configured;
    if let Some(provider) = settings.provider {
      self.ai_provider = provider;
    }
    let model: SharedString = settings
      .model
      .unwrap_or_else(|| self.ai_provider.default_model().to_string())
      .into();
    self.ai_model_select.update(cx, |select, cx| {
      select.set_items(provider_model_items(self.ai_provider), window, cx);
      select.set_selected_value(&model, window, cx);
    });
    self
      .ai_api_key_input
      .update(cx, |input, cx| input.set_value("", window, cx));
    self.ai_api_key_hint = settings.api_key_hint.map(Into::into);
  }

  fn load_ai_settings_if_authenticated(&mut self, cx: &mut Context<Self>) {
    if self.ai_settings_loaded || self.ai_settings_loading {
      return;
    }
    if !matches!(AuthStateStore::get(cx), AuthState::Authenticated(_)) {
      return;
    }
    self.load_ai_settings(cx);
  }

  fn load_ai_settings(&mut self, cx: &mut Context<Self>) {
    if self.ai_settings_loading {
      return;
    }

    self.ai_settings_loading = true;
    self.ai_settings_error = None;
    let api = self.api.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || api.fetch_ai_settings()).await;
      let _ = this.update(cx, |this, cx| {
        this.ai_settings_loading = false;
        match result {
          Ok(settings) => {
            this.ai_settings_loaded = true;
            let window_handle = this.window_handle;
            let _ = cx.update_window(window_handle, |_, window, cx| {
              this.apply_ai_settings(settings, window, cx);
            });
          }
          Err(error) => {
            let message = error.to_string();
            if !github_shared::is_unauthorized_error_message(message.as_str()) {
              this.ai_settings_error = Some(message.into());
            }
          }
        }
        cx.notify();
      });
    });
    self.ai_settings_task = Some(task);
  }

  fn set_ai_provider(&mut self, provider: AiProvider, window: &mut Window, cx: &mut Context<Self>) {
    if self.ai_provider == provider {
      return;
    }

    self.ai_provider = provider;
    self.ai_model_select.update(cx, |select, cx| {
      select.set_items(provider_model_items(provider), window, cx);
      select.set_selected_index(provider_default_model_index(provider), window, cx);
    });
    self.ai_settings_notice = None;
    cx.notify();
  }

  fn save_ai_settings(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
    if self.ai_settings_saving {
      return;
    }

    let api_key = self.ai_api_key_input.read(cx).value().trim().to_string();
    if api_key.is_empty() {
      self.ai_settings_error = Some("Enter an API key before saving.".into());
      self.ai_settings_notice = None;
      cx.notify();
      return;
    }

    let model = self
      .ai_model_select
      .read(cx)
      .selected_value()
      .map(|value| value.to_string())
      .unwrap_or_else(|| self.ai_provider.default_model().to_string());

    self.ai_settings_saving = true;
    self.ai_settings_error = None;
    self.ai_settings_notice = None;
    let api = self.api.clone();
    let provider = self.ai_provider;
    let task = cx.spawn(async move |this, cx| {
      let result =
        unblock(move || api.save_ai_settings(provider, Some(model.as_str()), api_key.as_str()))
          .await;
      let _ = this.update(cx, |this, cx| {
        this.ai_settings_saving = false;
        match result {
          Ok(settings) => {
            let window_handle = this.window_handle;
            let _ = cx.update_window(window_handle, |_, window, cx| {
              this.apply_ai_settings(settings, window, cx);
            });
            this.ai_settings_notice = Some("AI settings saved.".into());
          }
          Err(error) => {
            this.ai_settings_error = Some(error.to_string().into());
          }
        }
        cx.notify();
      });
    });
    self.ai_settings_task = Some(task);
  }

  fn delete_ai_settings(&mut self, cx: &mut Context<Self>) {
    if self.ai_settings_deleting {
      return;
    }

    self.ai_settings_deleting = true;
    self.ai_settings_error = None;
    self.ai_settings_notice = None;
    let api = self.api.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || api.delete_ai_settings()).await;
      let _ = this.update(cx, |this, cx| {
        this.ai_settings_deleting = false;
        match result {
          Ok(()) => {
            this.ai_configured = false;
            this.ai_api_key_hint = None;
            this.ai_settings_notice = Some("AI key removed.".into());
          }
          Err(error) => {
            this.ai_settings_error = Some(error.to_string().into());
          }
        }
        cx.notify();
      });
    });
    self.ai_settings_task = Some(task);
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
          SettingItem::new(
            "Clone Protocol",
            SettingField::dropdown(
              vec![
                ("https".into(), "HTTPS".into()),
                ("ssh".into(), "SSH".into()),
              ],
              {
                let view = view.clone();
                move |cx: &App| view.read(cx).clone_protocol.as_str().into()
              },
              {
                let view = view.clone();
                move |val: SharedString, cx: &mut App| {
                  let protocol = CloneProtocol::from_str(val.as_ref());
                  view.update(cx, |view, _| {
                    view.clone_protocol = protocol;
                  });
                  PersistedSettings::update(cx, |s| s.clone_protocol = protocol);
                }
              },
            )
            .default_value(self.clone_protocol.as_str()),
          )
          .description(
            "Protocol used when cloning a GitHub repository. HTTPS uses your credential helper; SSH uses your configured SSH key.",
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
      Self::ai_settings_page(view.clone()),
      Self::keyboard_shortcuts_page(view.clone(), window, cx),
    ]
  }

  fn ai_settings_page(view: gpui::Entity<Self>) -> SettingPage {
    SettingPage::new("AI").default_open(true).groups(vec![
      SettingGroup::new().title("Bring Your Own Key").items(vec![
        SettingItem::new(
          "Provider",
          SettingField::render({
            let view = view.clone();
            move |_, _, cx| {
              let provider = view.read(cx).ai_provider;
              h_flex()
                .items_center()
                .justify_end()
                .gap_2()
                .child(Self::render_ai_provider_button(
                  "settings-ai-provider-openai",
                  AiProvider::Openai,
                  provider,
                  view.clone(),
                ))
                .child(Self::render_ai_provider_button(
                  "settings-ai-provider-anthropic",
                  AiProvider::Anthropic,
                  provider,
                  view.clone(),
                ))
                .into_any_element()
            }
          }),
        )
        .description("Choose which provider Reviu should call for AI features."),
        SettingItem::new(
          "Model",
          SettingField::render({
            let view = view.clone();
            move |_, _, cx| {
              let select = view.read(cx).ai_model_select.clone();
              div()
                .w(px(280.0))
                .child(Select::new(&select).w_full())
                .into_any_element()
            }
          }),
        )
        .description("Pick which model Reviu calls for AI features."),
        SettingItem::new(
          "API Key",
          SettingField::render({
            let view = view.clone();
            move |_, _, cx| {
              let state = view.read(cx);
              let input = state.ai_api_key_input.clone();
              let hint = state.ai_api_key_hint.clone();
              div()
                .w(px(280.0))
                .flex()
                .flex_col()
                .items_start()
                .gap_1()
                .child(Input::new(&input).w_full().mask_toggle())
                .when_some(hint, |this, hint| {
                  this.child(
                    div()
                      .text_xs()
                      .text_color(cx.theme().muted_foreground)
                      .child(format!("Saved key: {hint}")),
                  )
                })
                .into_any_element()
            }
          }),
        )
        .description(
          "Stored encrypted by the Reviu backend. The desktop app does not keep the key.",
        ),
        SettingItem::new(
          "Credentials",
          SettingField::render(move |_, _, cx| {
            let state = view.read(cx);
            let is_busy =
              state.ai_settings_loading || state.ai_settings_saving || state.ai_settings_deleting;
            let configured = state.ai_configured;
            let error = state.ai_settings_error.clone();
            let notice = state.ai_settings_notice.clone();

            v_flex()
              .items_end()
              .gap_2()
              .child(
                h_flex()
                  .items_center()
                  .justify_end()
                  .gap_2()
                  .when(configured, |this| {
                    this.child(
                      Button::new("settings-ai-delete")
                        .small()
                        .ghost()
                        .label("Remove")
                        .disabled(is_busy)
                        .on_click({
                          let view = view.clone();
                          move |_, _, cx| {
                            view.update(cx, |view, cx| {
                              view.delete_ai_settings(cx);
                            });
                          }
                        }),
                    )
                  })
                  .child(
                    Button::new("settings-ai-save")
                      .small()
                      .primary()
                      .label(if state.ai_settings_saving {
                        "Saving..."
                      } else {
                        "Save"
                      })
                      .disabled(is_busy)
                      .on_click({
                        let view = view.clone();
                        move |_, window, cx| {
                          view.update(cx, |view, cx| {
                            view.save_ai_settings(window, cx);
                          });
                        }
                      }),
                  ),
              )
              .when_some(notice, |this, notice| {
                this.child(
                  div()
                    .text_xs()
                    .text_color(cx.theme().status_green())
                    .child(notice),
                )
              })
              .when_some(error, |this, error| {
                this.child(
                  div()
                    .text_xs()
                    .text_color(cx.theme().status_red())
                    .child(error),
                )
              })
              .into_any_element()
          }),
        )
        .description("Required for BYOK AI features in Reviu Pro."),
      ]),
    ])
  }

  fn render_ai_provider_button(
    id: &'static str,
    provider: AiProvider,
    selected_provider: AiProvider,
    view: gpui::Entity<Self>,
  ) -> Button {
    Button::new(id)
      .small()
      .outline()
      .label(provider.label())
      .selected(provider == selected_provider)
      .on_click(move |_, window, cx| {
        view.update(cx, |view, cx| {
          view.set_ai_provider(provider, window, cx);
        });
      })
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
        open_pr_target(owner, repo, number, open_changes_tab, review_comment_id, cx);
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
      CommandPaletteAction::OpenGithubCommitDetails { owner, repo, sha } => {
        open_commit_target(owner, repo, sha, cx);
        Ok(())
      }
      CommandPaletteAction::OpenGithubProfile { login } => {
        open_profile_target(login, cx);
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
      CommandPaletteAction::SearchGithubRepository => {
        let api = WorkspaceApi::global(cx).api.clone();
        crate::github_search_dialog::open_github_search_dialog(api, window, cx);
        Ok(())
      }
      CommandPaletteAction::CreateGithubRepository => {
        let api = WorkspaceApi::global(cx).api.clone();
        crate::github_create_repository_dialog::open_create_repository_dialog(api, window, cx);
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

fn provider_model_items(provider: AiProvider) -> Vec<SharedString> {
  provider
    .available_models()
    .iter()
    .map(|name| SharedString::from(*name))
    .collect()
}

fn provider_default_model_index(provider: AiProvider) -> Option<IndexPath> {
  let default = provider.default_model();
  provider
    .available_models()
    .iter()
    .position(|name| *name == default)
    .map(|row| IndexPath::default().row(row))
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
    assert!(description.contains("Git, Repo Code, and PR Changes pages"));
  }
}
