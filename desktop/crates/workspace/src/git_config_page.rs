use std::{
  fs,
  path::{Path, PathBuf},
  sync::Arc,
};

use editor::Editor;
use git::find_global_config_path;
use gpui::{
  AnyElement, App, Context, Entity, FocusHandle, Focusable, Render, SharedString, Window, div, img,
  prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Disableable, Icon, IconName, Sizable as _,
  button::{Button, ButtonVariants},
  h_flex,
  label::Label,
};

use ui::{
  CommandPalette, CommandPaletteAction, CommandPaletteCommand, CommandPaletteConfig,
  CommandPaletteHandler, CommandPalettePage, FILE_ICON_SIZE_PX, PAGE_HEADER_HEIGHT, StatusThemeExt,
  file_icon_path_for_path_with_theme,
};

use crate::{
  CloseWorkspacePage, ShowCommandPalette,
  auth_state::AuthStateStore,
  github_navigation::{open_commit_target, open_pr_target, open_profile_target, open_repo_target},
  github_page::GithubPageHandle,
  navigation::NavigationHistory,
  workspace::WorkspaceApi,
};

pub struct GitConfigPage {
  focus_handle: FocusHandle,
  editor: Option<Entity<Editor>>,
  load_error: Option<SharedString>,
}

impl GitConfigPage {
  pub fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
    let config_path = Self::git_config_path();
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
      CommandPaletteCommand::default_global_commands(CommandPalettePage::GitConfig, include_github);

    let view = cx.entity();
    let handler: CommandPaletteHandler = Arc::new(move |action, window, cx| {
      view.update(cx, |view, cx| {
        view.handle_command_palette_action(action, window, cx)
      })
    });

    let config = CommandPaletteConfig::new(Vec::new(), commands, handler);
    let palette = cx.new(|cx| CommandPalette::new(window, cx, config));
    ui::open_command_palette_dialog(palette, window, cx);
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
      CommandPaletteAction::OpenSettingsPage => {
        NavigationHistory::navigate("/settings", cx);
        Ok(())
      }
      CommandPaletteAction::OpenBillingPage => {
        NavigationHistory::navigate("/billing", cx);
        Ok(())
      }
      CommandPaletteAction::OpenAboutPage => {
        NavigationHistory::navigate("/about", cx);
        Ok(())
      }
      CommandPaletteAction::OpenGitConfigPage => Ok(()),
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

  fn render_editor_header(&self, editor: &Entity<Editor>, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let editor_state = editor.read(cx);
    let file_name = editor_state
      .workdir_path
      .file_name()
      .and_then(|name| name.to_str())
      .unwrap_or(".gitconfig")
      .to_string();
    let dir_path = editor_state
      .workdir_path
      .parent()
      .and_then(|parent| parent.to_str())
      .unwrap_or("")
      .to_string();
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
      .icon(IconName::Close)
      .ghost()
      .compact()
      .tooltip("Close")
      .on_click(|_, _, cx| {
        NavigationHistory::navigate_back(cx);
      });

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
        h_flex()
          .items_center()
          .gap_2()
          .min_w_0()
          .flex_1()
          .child(
            file_icon_path_for_path_with_theme(&editor_state.workdir_path, &theme)
              .map(|path| img(path).size(px(FILE_ICON_SIZE_PX)).into_any_element())
              .unwrap_or_else(|| {
                Icon::new(IconName::File)
                  .size_3()
                  .text_color(theme.foreground)
                  .into_any_element()
              }),
          )
          .child(
            h_flex()
              .min_w_0()
              .flex_1()
              .items_center()
              .gap_2()
              .child(
                h_flex()
                  .min_w_0()
                  .items_center()
                  .gap_2()
                  .child(div().min_w_0().child(Label::new(file_name).truncate()))
                  .when(file_dirty, |this| {
                    this.child(
                      div()
                        .size_2()
                        .rounded_full()
                        .bg(theme.foreground)
                        .flex_shrink_0(),
                    )
                  }),
              )
              .when(!dir_path.is_empty(), |this| {
                this.child(
                  div()
                    .min_w_0()
                    .flex_1()
                    .overflow_hidden()
                    .text_ellipsis_start()
                    .text_color(theme.muted_foreground)
                    .child(format!("- {}", dir_path)),
                )
              }),
          ),
      )
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
                .icon(IconName::Close)
                .ghost()
                .compact()
                .tooltip("Close")
                .on_click(|_, _, cx| {
                  NavigationHistory::navigate_back(cx);
                }),
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
      .track_focus(&self.focus_handle(cx))
      .on_action(cx.listener(GitConfigPage::show_command_palette_action))
      .on_action(cx.listener(GitConfigPage::close_workspace_page_action))
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
