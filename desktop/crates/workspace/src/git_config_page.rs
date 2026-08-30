use std::{
  fs,
  path::{Path, PathBuf},
  sync::Arc,
};

use editor::Editor;
use git::find_global_config_path;
use gpui::{
  AnyElement, App, Context, Entity, FocusHandle, Focusable, Global, Render, SharedString, Window,
  div, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Disableable, IconName, Sizable as _,
  button::{Button, ButtonVariants},
  dialog::DialogFooter,
  h_flex,
};

use ui::{
  CommandPalette, CommandPaletteAction, CommandPaletteCommand, CommandPaletteConfig,
  CommandPaletteHandler, CommandPalettePage, PAGE_HEADER_HEIGHT, StatusThemeExt, WindowExt,
};

use crate::{ShowCommandPalette, auth_state::AuthStateStore, file_view::render_file_title};

const GIT_CONFIG_DIALOG_MAX_WIDTH: f32 = 1120.0;
const GIT_CONFIG_DIALOG_MAX_HEIGHT: f32 = 780.0;
const GIT_CONFIG_DIALOG_MARGIN: f32 = 64.0;
const GIT_CONFIG_UNSAVED_SAVE_DEBUG_SELECTOR: &str = "git-config-unsaved-save";
const GIT_CONFIG_UNSAVED_DISCARD_DEBUG_SELECTOR: &str = "git-config-unsaved-discard";
const GIT_CONFIG_UNSAVED_CANCEL_DEBUG_SELECTOR: &str = "git-config-unsaved-cancel";

#[derive(Clone, Default)]
struct GitConfigDialogState {
  view: Option<Entity<GitConfigPage>>,
  is_open: bool,
}

impl Global for GitConfigDialogState {}

pub struct GitConfigPage {
  focus_handle: FocusHandle,
  editor: Option<Entity<Editor>>,
  load_error: Option<SharedString>,
}

impl GitConfigPage {
  pub fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
    Self::new_for_path(Self::git_config_path(), cx)
  }

  fn new_for_path(config_path: PathBuf, cx: &mut Context<Self>) -> Self {
    let (editor, load_error) = Self::create_editor(&config_path, cx);

    Self {
      focus_handle: cx.focus_handle(),
      editor,
      load_error,
    }
  }

  fn git_config_path() -> PathBuf {
    if let Some(path) = find_global_config_path() {
      return path;
    }

    dirs::home_dir()
      .map(|home| home.join(".gitconfig"))
      .unwrap_or_else(|| PathBuf::from(".gitconfig"))
  }

  fn create_editor(
    config_path: &Path,
    cx: &mut Context<Self>,
  ) -> (Option<Entity<Editor>>, Option<SharedString>) {
    if let Err(err) = Self::ensure_git_config_exists(config_path) {
      return (None, Some(err.into()));
    }

    let repo_root = config_path
      .parent()
      .map(Path::to_path_buf)
      .unwrap_or_else(|| PathBuf::from("."));
    let file_path = config_path.to_path_buf();
    let editor = cx.new(|cx| Editor::new_with_paths(repo_root, file_path, cx));
    (Some(editor), None)
  }

  fn ensure_git_config_exists(config_path: &Path) -> Result<(), String> {
    if config_path.exists() {
      return Ok(());
    }

    if let Some(parent) = config_path.parent() {
      fs::create_dir_all(parent).map_err(|err| {
        format!(
          "Failed to create Git config directory {}: {}",
          parent.display(),
          err
        )
      })?;
    }

    fs::write(config_path, "")
      .map_err(|err| format!("Failed to create {}: {}", config_path.display(), err))
  }

  fn show_command_palette_action(
    &mut self,
    _: &ShowCommandPalette,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.open_command_palette(window, cx);
  }

  fn open_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let include_github = AuthStateStore::has_github_access(cx);
    let signed_in = AuthStateStore::is_signed_in(cx);
    let commands = crate::shortcuts::with_palette_keybindings(
      CommandPaletteCommand::default_global_commands(ui::GlobalCommandsContext {
        current_page: CommandPalettePage::GitConfig,
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

  fn editor_is_dirty(&self, cx: &App) -> bool {
    self
      .editor
      .as_ref()
      .is_some_and(|editor| editor.read(cx).is_dirty)
  }

  fn close_dialog_after_alert(window: &mut Window, cx: &mut App) {
    window.close_dialog(cx);
  }

  fn request_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.editor_is_dirty(cx) {
      self.open_unsaved_changes_dialog(window, cx);
      return;
    }
    window.close_dialog(cx);
  }

  fn close_action(&mut self, _: &editor::CloseFind, window: &mut Window, cx: &mut Context<Self>) {
    self.request_close(window, cx);
    cx.stop_propagation();
  }

  fn open_unsaved_changes_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let view = cx.entity();
    let editor = self.editor.clone();

    window.open_alert_dialog(cx, move |alert, _, _| {
      let save_view = view.clone();
      let discard_view = view.clone();
      let save_editor = editor.clone();
      alert
        .title("Save Git config changes?")
        .description(div().child("Save your edits before closing, or discard them permanently."))
        .close_button(true)
        .footer(
          DialogFooter::new()
            .child(
              Button::new(GIT_CONFIG_UNSAVED_CANCEL_DEBUG_SELECTOR)
                .debug_selector(|| GIT_CONFIG_UNSAVED_CANCEL_DEBUG_SELECTOR.to_string())
                .label("Cancel")
                .ghost()
                .on_click(|_, window, cx| {
                  window.close_dialog(cx);
                }),
            )
            .child(
              Button::new(GIT_CONFIG_UNSAVED_DISCARD_DEBUG_SELECTOR)
                .debug_selector(|| GIT_CONFIG_UNSAVED_DISCARD_DEBUG_SELECTOR.to_string())
                .label("Discard")
                .danger()
                .on_click(move |_, window, cx| {
                  window.close_dialog(cx);
                  if cx.has_global::<GitConfigDialogState>() {
                    cx.global_mut::<GitConfigDialogState>().view = None;
                  }
                  discard_view.update(cx, |view, _| {
                    view.editor = None;
                  });
                  Self::close_dialog_after_alert(window, cx);
                }),
            )
            .child(
              Button::new(GIT_CONFIG_UNSAVED_SAVE_DEBUG_SELECTOR)
                .debug_selector(|| GIT_CONFIG_UNSAVED_SAVE_DEBUG_SELECTOR.to_string())
                .label("Save")
                .primary()
                .on_click(move |_, window, cx| {
                  let window_handle = window.window_handle();
                  window.close_dialog(cx);
                  if let Some(editor) = save_editor.clone() {
                    let save_view = save_view.clone();
                    editor.update(cx, |editor, cx| {
                      editor.save_with_completion(
                        cx,
                        Some(Box::new(move |cx| {
                          let save_view = save_view.clone();
                          let _ = cx.update_window(window_handle, move |_, window, cx| {
                            if cx.has_global::<GitConfigDialogState>() {
                              cx.global_mut::<GitConfigDialogState>().view = None;
                            }
                            save_view.update(cx, |view, _| {
                              view.editor = None;
                            });
                            Self::close_dialog_after_alert(window, cx);
                          });
                        })),
                      );
                    });
                  }
                }),
            ),
        )
    });
  }

  fn render_editor_header(&self, editor: &Entity<Editor>, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let editor_state = editor.read(cx);
    let file_path = editor_state.workdir_path.clone();
    let file_dirty = editor_state.is_dirty;

    let editor_entity = editor.clone();
    let save_button = Button::new("git-config-save")
      .label("Save")
      .xsmall()
      .ghost()
      .disabled(!file_dirty)
      .on_click(move |_, _, cx| {
        editor_entity.update(cx, |editor, cx| editor.save(cx));
      });

    let close_button = Button::new("git-config-close")
      .debug_selector(|| "git-config-close".to_string())
      .icon(IconName::Close)
      .ghost()
      .compact()
      .tooltip("Close")
      .on_click(cx.listener(|this, _, window, cx| this.request_close(window, cx)));

    div()
      .h(px(PAGE_HEADER_HEIGHT))
      .max_h(px(PAGE_HEADER_HEIGHT))
      .px_3()
      .flex()
      .items_center()
      .justify_between()
      .bg(theme.sidebar)
      .border_b_1()
      .border_color(theme.title_bar_border)
      .child(render_file_title(&file_path, file_dirty, cx))
      .child(
        h_flex()
          .items_center()
          .gap_2()
          .child(save_button)
          .child(close_button),
      )
      .into_any_element()
  }
}

impl Render for GitConfigPage {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    let body = if let Some(editor) = self.editor.clone() {
      div()
        .size_full()
        .flex()
        .flex_col()
        .bg(theme.background)
        .child(self.render_editor_header(&editor, cx))
        .child(
          h_flex()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .child(editor),
        )
        .into_any_element()
    } else {
      let message = self
        .load_error
        .clone()
        .unwrap_or_else(|| "Unable to load ~/.gitconfig".into());
      div()
        .size_full()
        .flex()
        .flex_col()
        .bg(theme.background)
        .child(
          div()
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
                .child("Git Config"),
            )
            .child(
              Button::new("git-config-close")
                .debug_selector(|| "git-config-close".to_string())
                .icon(IconName::Close)
                .ghost()
                .compact()
                .tooltip("Close")
                .on_click(cx.listener(|this, _, window, cx| this.request_close(window, cx))),
            ),
        )
        .child(
          div()
            .flex_1()
            .items_center()
            .justify_center()
            .text_color(theme.status_red())
            .child(message),
        )
        .into_any_element()
    };

    div()
      .size_full()
      .key_context(crate::shortcuts::WORKSPACE_CONTEXT)
      .track_focus(&self.focus_handle(cx))
      .on_action(cx.listener(GitConfigPage::show_command_palette_action))
      .on_action(cx.listener(GitConfigPage::close_action))
      .child(body)
  }
}

impl Focusable for GitConfigPage {
  fn focus_handle(&self, cx: &App) -> FocusHandle {
    if let Some(editor) = self.editor.as_ref() {
      return editor.read(cx).focus_handle(cx);
    }
    self.focus_handle.clone()
  }
}

pub(crate) fn open_git_config_dialog(window: &mut Window, cx: &mut App) {
  let state = cx
    .try_global::<GitConfigDialogState>()
    .cloned()
    .unwrap_or_default();
  if state.is_open && window.has_active_dialog(cx) {
    return;
  }

  let view = state
    .view
    .unwrap_or_else(|| cx.new(|cx| GitConfigPage::new(window, cx)));
  open_git_config_dialog_with_view(view, window, cx);
}

fn open_git_config_dialog_with_view(
  view: Entity<GitConfigPage>,
  window: &mut Window,
  cx: &mut App,
) {
  cx.set_global(GitConfigDialogState {
    view: Some(view.clone()),
    is_open: true,
  });

  let view_for_overlay = view.clone();
  let view_for_focus = view.clone();
  let view_for_cancel = view.clone();
  window.open_dialog(cx, move |dialog, window, _| {
    let viewport = window.viewport_size();
    let width = px(
      (viewport.width.as_f32() - GIT_CONFIG_DIALOG_MARGIN)
        .clamp(700.0, GIT_CONFIG_DIALOG_MAX_WIDTH),
    );
    let height = px(
      (viewport.height.as_f32() - GIT_CONFIG_DIALOG_MARGIN)
        .clamp(500.0, GIT_CONFIG_DIALOG_MAX_HEIGHT),
    );

    dialog
      .p_0()
      .gap_0()
      .w(width)
      .h(height)
      .keyboard(true)
      .close_button(false)
      .on_cancel({
        let view_for_cancel = view_for_cancel.clone();
        move |_, window, cx| {
          view_for_cancel.update(cx, |view, cx| {
            if view.editor_is_dirty(cx) {
              view.open_unsaved_changes_dialog(window, cx);
              false
            } else {
              true
            }
          })
        }
      })
      .on_close(|_, _, cx| {
        if cx.has_global::<GitConfigDialogState>() {
          cx.global_mut::<GitConfigDialogState>().is_open = false;
        }
      })
      .child(view_for_overlay.clone())
  });

  window.on_next_frame(move |window, cx| {
    let focus_handle = view_for_focus.read(cx).focus_handle(cx);
    window.focus(&focus_handle, cx);
  });
}

#[cfg(test)]
mod tests {
  use super::*;
  use gpui::{TestAppContext, VisualTestContext};

  struct DialogHost;

  impl Render for DialogHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
      div()
        .size_full()
        .children(gpui_component::Root::render_dialog_layer(window, cx))
    }
  }

  fn dialog_host(cx: &mut TestAppContext) -> &mut VisualTestContext {
    cx.update(gpui_component::init);
    let (_root, cx) = cx.add_window_view(|window, cx| {
      let host = cx.new(|_| DialogHost);
      gpui_component::Root::new(host, window, cx)
    });
    cx
  }

  fn dirty_editor(view: &Entity<GitConfigPage>, cx: &mut VisualTestContext) {
    let editor = view
      .read_with(cx, |view, _| view.editor.clone())
      .expect("git config editor");
    editor.update(cx, |editor, cx| {
      editor.document.update(cx, |document, cx| {
        document.replace_all("changed\n", cx);
      });
      editor.is_dirty = true;
    });
  }

  #[gpui::test]
  async fn escape_closes_git_config_dialog(cx: &mut TestAppContext) {
    let cx = dialog_host(cx);
    let path = std::env::temp_dir().join(format!(
      "reviu-git-config-dialog-{}.gitconfig",
      std::process::id()
    ));
    let _ = std::fs::remove_file(&path);

    cx.update(|window, cx| {
      let view = cx.new(|cx| GitConfigPage::new_for_path(path.clone(), cx));
      open_git_config_dialog_with_view(view, window, cx);
    });
    cx.run_until_parked();
    assert!(cx.update(|window, cx| window.has_active_dialog(cx)));

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();

    assert!(!cx.update(|window, cx| window.has_active_dialog(cx)));
    let _ = std::fs::remove_file(&path);
  }

  #[gpui::test]
  async fn closing_dirty_git_config_asks_before_discarding(cx: &mut TestAppContext) {
    let cx = dialog_host(cx);
    let path = std::env::temp_dir().join(format!(
      "reviu-git-config-dirty-dialog-{}.gitconfig",
      std::process::id()
    ));
    std::fs::write(&path, "original\n").expect("write git config");

    let view = cx.update(|window, cx| {
      let view = cx.new(|cx| GitConfigPage::new_for_path(path.clone(), cx));
      open_git_config_dialog_with_view(view.clone(), window, cx);
      view
    });
    cx.run_until_parked();
    dirty_editor(&view, cx);

    let close = cx.debug_bounds("git-config-close").expect("close button");
    cx.simulate_click(close.center(), gpui::Modifiers::default());
    cx.run_until_parked();

    assert!(
      cx.debug_bounds(GIT_CONFIG_UNSAVED_DISCARD_DEBUG_SELECTOR)
        .is_some()
    );
    assert!(cx.update(|window, cx| window.has_active_dialog(cx)));

    let discard = cx
      .debug_bounds(GIT_CONFIG_UNSAVED_DISCARD_DEBUG_SELECTOR)
      .expect("discard button");
    cx.simulate_click(discard.center(), gpui::Modifiers::default());
    cx.run_until_parked();

    assert!(!cx.update(|window, cx| window.has_active_dialog(cx)));
    assert_eq!(
      std::fs::read_to_string(&path).expect("read git config"),
      "original\n"
    );
    let _ = std::fs::remove_file(&path);
  }
}
