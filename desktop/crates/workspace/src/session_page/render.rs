//! Everything the shell paints: sidebar, center, diff header, dock.

use super::*;
use crate::annotations::AnnotationKind;
use crate::hunk_actions::render_hunk_actions;

impl SessionPage {
  /// Without a repository half the shell has nothing to show, so the row that
  /// normally names it asks for one instead.
  pub(super) fn render_open_repository_row(&self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    h_flex()
      .id("session-open-repository")
      .debug_selector(|| OPEN_REPOSITORY_ROW_DEBUG_SELECTOR.to_string())
      .items_center()
      .gap_2()
      .px_3()
      .py_2()
      .border_t_1()
      .border_color(theme.border)
      .cursor_pointer()
      .hover(|this| this.bg(theme.secondary_hover))
      .tooltip(|window, cx| {
        gpui_component::tooltip::Tooltip::new("Open a repository").build(window, cx)
      })
      .on_click(cx.listener(|this, _, window, cx| {
        this.start_open_repository(window, cx);
      }))
      .child(
        gpui_component::Icon::new(gpui_component::IconName::FolderOpen)
          .size_3()
          .text_color(theme.muted_foreground),
      )
      .child(
        div()
          .text_xs()
          .text_color(theme.foreground)
          .truncate()
          .child("Open repository"),
      )
      .into_any_element()
  }

  /// Ahead/behind counter that runs the matching sync command when clicked.
  pub(super) fn render_sync_counter(
    &self,
    id: &'static str,
    icon: gpui_component::IconName,
    count: usize,
    color: gpui::Hsla,
    tooltip: &'static str,
    in_flight: bool,
    command: RepoCommand,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let color = if in_flight {
      cx.theme().muted_foreground
    } else {
      color
    };

    h_flex()
      .id(id)
      .debug_selector(move || id.to_string())
      .items_center()
      .gap_1()
      .flex_shrink_0()
      .when(!in_flight, |this| {
        this
          .cursor_pointer()
          .tooltip(move |window, cx| {
            gpui_component::tooltip::Tooltip::new(tooltip).build(window, cx)
          })
          .on_click(cx.listener(move |this, _, window, cx| {
            // The row switches repository; the counter runs its command instead.
            cx.stop_propagation();
            if let Err(error) = this.run_repo_command(command.clone(), window, cx) {
              window.push_notification(Notification::warning(error), cx);
            }
          }))
      })
      .child(gpui_component::Icon::new(icon).size_3().text_color(color))
      .child(div().text_xs().text_color(color).child(count.to_string()))
      .into_any_element()
  }

  pub(super) fn render_sessions_sidebar(&mut self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let (conversations, current_id) = match self.agent_chat_view.as_ref() {
      Some(panel) => {
        let panel = panel.read(cx);
        (
          panel.list_conversations(),
          panel.current_conversation().id.clone(),
        )
      }
      None => (Vec::new(), String::new()),
    };
    let now = now_secs();

    let header = h_flex()
      .h(px(40.))
      .min_h(px(40.))
      .max_h(px(40.))
      .flex_shrink_0()
      .items_center()
      .justify_between()
      .px_3()
      .border_b_1()
      .border_color(theme.border)
      .child(
        div()
          .text_xs()
          .font_weight(gpui::FontWeight::SEMIBOLD)
          .text_color(theme.muted_foreground)
          .child("Sessions"),
      )
      .child(
        Button::new("session-page-new-session")
          .icon(UiIconName::SquarePen)
          .ghost()
          .compact()
          .small()
          .tooltip("New session")
          .on_click(cx.listener(|this, _, window, cx| this.new_session(window, cx))),
      );

    let rows: Vec<_> = conversations
      .into_iter()
      .enumerate()
      .map(|(ix, meta)| {
        let is_current = meta.id == current_id;
        let id = meta.id.clone();
        let delete_id = meta.id.clone();
        let title = session_row_title(&meta);
        let time = format_relative_secs(meta.updated_at_secs, now);
        let group_name = SharedString::from(format!("session-row-{}", meta.id));

        div()
          .id(("session-page-session-row", ix))
          .group(group_name.clone())
          .mx_2()
          .px_2()
          .py_1p5()
          .rounded(px(6.0))
          .cursor_pointer()
          .when(is_current, |this| this.bg(theme.secondary_active))
          .hover(|s| s.bg(theme.secondary_hover))
          .on_click(cx.listener(move |this, _, window, cx| {
            this.select_session(&id, window, cx);
          }))
          .child(
            h_flex()
              .items_center()
              .gap_2()
              .child(
                div()
                  .flex_1()
                  .min_w(px(0.0))
                  .text_sm()
                  .truncate()
                  .text_color(theme.foreground)
                  .child(title),
              )
              .child(
                div()
                  .text_xs()
                  .text_color(theme.muted_foreground)
                  .group_hover(group_name.clone(), |this| this.opacity(0.0))
                  .child(time),
              )
              .child(
                Button::new(("session-page-session-delete", ix))
                  .icon(UiIconName::Trash)
                  .xsmall()
                  .ghost()
                  .opacity(0.0)
                  .group_hover(group_name.clone(), |this| this.opacity(1.0))
                  .tooltip("Delete session")
                  .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.delete_session(&delete_id, cx);
                  })),
              ),
          )
      })
      .collect();

    let github_section =
      AuthStateStore::has_github_access(cx).then(|| self.inbox.clone().into_any_element());

    let repo_name = self
      .selected_repo
      .as_deref()
      .and_then(|path| path.file_name())
      .map(|name| name.to_string_lossy().into_owned());

    let branch_status = self.branch_status.clone();
    let sync_in_flight = self.repo_command_in_flight;

    let repo_context = match repo_name {
      None => Some(self.render_open_repository_row(cx).into_any_element()),
      Some(name) => Some(
        h_flex()
          .id("session-repo-context")
          .debug_selector(|| REPO_CONTEXT_DEBUG_SELECTOR.to_string())
          .items_center()
          .gap_2()
          .px_3()
          .py_2()
          .border_t_1()
          .border_color(theme.border)
          .cursor_pointer()
          .hover(|this| this.bg(theme.secondary_hover))
          .tooltip(|window, cx| {
            gpui_component::tooltip::Tooltip::new("Switch repository").build(window, cx)
          })
          .on_click(cx.listener(|this, _, window, cx| {
            this.open_command_palette_with_screen(
              Some(CommandPaletteInitialScreen::SwitchRepository),
              window,
              cx,
            );
          }))
          .child(
            div()
              .text_xs()
              .text_color(theme.foreground)
              .truncate()
              .child(name),
          )
          .when_some(branch_status.clone(), |this, status| {
            this.child(
              h_flex()
                .items_center()
                .gap_1()
                .min_w(px(0.0))
                .flex_1()
                .child(
                  gpui_component::Icon::new(UiIconName::GitBranch)
                    .size_3()
                    .text_color(theme.muted_foreground),
                )
                .child(
                  div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .truncate()
                    .child(SharedString::from(status.name)),
                ),
            )
          })
          .when_some(branch_status, |this, status| {
            this
              .when(status.behind > 0, |this| {
                this.child(self.render_sync_counter(
                  REPO_BEHIND_DEBUG_SELECTOR,
                  gpui_component::IconName::ArrowDown,
                  status.behind,
                  theme.status_red(),
                  "Pull",
                  sync_in_flight,
                  RepoCommand::Pull,
                  cx,
                ))
              })
              .when(status.ahead > 0, |this| {
                this.child(self.render_sync_counter(
                  REPO_AHEAD_DEBUG_SELECTOR,
                  gpui_component::IconName::ArrowUp,
                  status.ahead,
                  theme.status_green(),
                  "Push",
                  sync_in_flight,
                  RepoCommand::Push,
                  cx,
                ))
              })
          })
          .into_any_element(),
      ),
    };

    v_flex()
      .size_full()
      .min_w(px(0.0))
      .min_h_0()
      .bg(theme.sidebar)
      .child(header)
      .child(if rows.is_empty() {
        v_flex()
          .flex_1()
          .min_h_0()
          .items_center()
          .justify_center()
          .gap_2()
          .px_4()
          .child(
            Icon::new(UiIconName::MessageCirclePlus)
              .size_4()
              .text_color(theme.muted_foreground),
          )
          .child(
            div()
              .text_sm()
              .text_color(theme.muted_foreground)
              .child("No sessions yet"),
          )
          .child(
            div()
              .text_xs()
              .text_center()
              .text_color(theme.muted_foreground.opacity(0.8))
              .child("Message the agent to start one"),
          )
          .into_any_element()
      } else {
        div()
          .id("session-page-session-list")
          .flex_1()
          .min_h_0()
          .overflow_y_scroll()
          .py_1()
          .children(rows)
          .into_any_element()
      })
      .children(github_section)
      .children(repo_context)
      .into_any_element()
  }

  /// The todo of an interactive rebase takes the whole center: it is a table to
  /// edit, not a side panel.
  pub(super) fn render_interactive_rebase(&mut self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let Some(todo_view) = self.interactive_rebase_todo_view.clone() else {
      return div().into_any_element();
    };

    v_flex()
      .size_full()
      .min_h_0()
      .min_w(px(0.0))
      .debug_selector(|| INTERACTIVE_REBASE_DEBUG_SELECTOR.to_string())
      .child(
        h_flex()
          .h(px(40.))
          .min_h(px(40.))
          .flex_shrink_0()
          .items_center()
          .gap_2()
          .px_3()
          .border_b_1()
          .border_color(theme.border)
          .child(gpui_component::Icon::new(UiIconName::GitMerge).size_3())
          .child(div().text_sm().child("Interactive rebase")),
      )
      .child(div().flex_1().min_h_0().child(todo_view))
      .into_any_element()
  }

  pub(super) fn render_center(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    // Keyed on what is shown, so every swap remounts and replays the fade.
    let (id, view) = match self.center {
      CenterView::Conversation => (
        SharedString::from("session-center-conversation"),
        self.render_conversation(cx),
      ),
      CenterView::InteractiveRebase => (
        SharedString::from("session-center-interactive-rebase"),
        self.render_interactive_rebase(cx),
      ),
      CenterView::Diff => {
        let file = self
          .selected_file
          .as_deref()
          .map(|path| path.to_string_lossy().into_owned())
          .unwrap_or_default();
        (
          SharedString::from(format!("session-center-diff-{file}")),
          self.render_diff_view(window, cx),
        )
      }
    };
    div()
      .size_full()
      .min_w(px(0.0))
      .min_h_0()
      .child(view)
      .with_animation(
        id,
        gpui::Animation::new(std::time::Duration::from_millis(CENTER_SWAP_FADE_MS))
          .with_easing(gpui::ease_out_quint()),
        |view, delta| view.opacity(delta),
      )
      .into_any_element()
  }

  pub(super) fn render_conversation(&mut self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let mut container = div()
      .size_full()
      .min_w(px(0.0))
      .min_h_0()
      .bg(theme.background);
    if let Some(view) = self.agent_chat_view.clone() {
      container = container.child(view);
    }
    container.into_any_element()
  }

  pub(super) fn render_diff_header(&self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let copyable_count = self.copyable_review_comment_count();
    let file_dirty = self
      .editor
      .as_ref()
      .is_some_and(|editor| editor.read(cx).is_dirty);
    let save_editor = self.editor.clone();
    let file_status = self.selected_file_status(cx);
    let old_path = self.selected_file_old_path(cx);
    let file_title = self.selected_file.clone().map(|path| {
      render_file_title_with_status(&path, old_path.as_deref(), file_status, file_dirty, cx)
    });

    h_flex()
      .h(px(40.))
      .min_h(px(40.))
      .max_h(px(40.))
      .flex_shrink_0()
      .items_center()
      .gap_3()
      .px_3()
      .border_b_1()
      .border_color(theme.border)
      .child(
        Button::new("session-page-diff-back")
          .label("Chat")
          .icon(UiIconName::MessageCircle)
          .ghost()
          .compact()
          .small()
          .tooltip("Back to the conversation (Esc)")
          .on_click(cx.listener(|this, _, window, cx| this.close_diff(window, cx))),
      )
      .children(file_title)
      .when(self.can_accept_all_conflicts(cx), |this| {
        this
          .child(
            Button::new("session-page-accept-all-current")
              .label("Accept All Current")
              .debug_selector(|| ACCEPT_ALL_CURRENT_DEBUG_SELECTOR.to_string())
              .xsmall()
              .ghost()
              .on_click(cx.listener(|this, _, _, cx| {
                this.resolve_all_conflicts(ConflictResolution::Current, cx)
              })),
          )
          .child(
            Button::new("session-page-accept-all-incoming")
              .label("Accept All Incoming")
              .debug_selector(|| ACCEPT_ALL_INCOMING_DEBUG_SELECTOR.to_string())
              .xsmall()
              .ghost()
              .on_click(cx.listener(|this, _, _, cx| {
                this.resolve_all_conflicts(ConflictResolution::Incoming, cx)
              })),
          )
      })
      .when_some(self.annotation_navigation(cx), |this, state| {
        let (previous_tooltip, next_tooltip) = match state.kind {
          AnnotationKind::Conflict => ("Previous conflict", "Next conflict"),
          AnnotationKind::Change => ("Previous change", "Next change"),
        };
        let enabled = can_navigate_annotations(Some(state));
        this
          .child(
            div()
              .text_xs()
              .text_color(theme.muted_foreground)
              .debug_selector(|| ANNOTATION_COUNTER_DEBUG_SELECTOR.to_string())
              .child(format!("{}/{}", state.active_index + 1, state.total)),
          )
          .child(
            Button::new("session-page-annotation-prev")
              .icon(gpui_component::IconName::ArrowUp)
              .xsmall()
              .ghost()
              .compact()
              .tooltip(previous_tooltip)
              .disabled(!enabled)
              .on_click(cx.listener(|this, _, _, cx| {
                this.navigate_change(AnnotationDirection::Previous, cx)
              })),
          )
          .child(
            Button::new("session-page-annotation-next")
              .icon(gpui_component::IconName::ArrowDown)
              .xsmall()
              .ghost()
              .compact()
              .tooltip(next_tooltip)
              .disabled(!enabled)
              .on_click(
                cx.listener(|this, _, _, cx| this.navigate_change(AnnotationDirection::Next, cx)),
              ),
          )
      })
      .when(self.editor.is_some() && self.previewable(), |this| {
        let (label, icon) = if self.show_preview {
          ("Code", UiIconName::FileCode)
        } else {
          ("Preview", UiIconName::Eye)
        };
        this.child(
          Button::new("session-page-preview-toggle")
            .debug_selector(|| PREVIEW_TOGGLE_DEBUG_SELECTOR.to_string())
            .label(label)
            .icon(icon)
            .xsmall()
            .ghost()
            .tooltip("Show the rendered file")
            .on_click(cx.listener(|this, _, _, cx| this.toggle_preview(cx))),
        )
      })
      .when(
        self.editor.is_some()
          && self.binary_preview.is_none()
          && self.selected_file_has_changes(cx)
          && !(self.show_preview && self.previewable()),
        |this| {
          let hide_whitespace = self.hide_whitespace;
          this.child(
            Button::new("session-page-whitespace-toggle")
              .debug_selector(|| WHITESPACE_TOGGLE_DEBUG_SELECTOR.to_string())
              .label("Whitespace")
              .icon(if hide_whitespace {
                gpui_component::IconName::Eye
              } else {
                gpui_component::IconName::EyeOff
              })
              .xsmall()
              .ghost()
              .tooltip(if hide_whitespace {
                "Show whitespace changes"
              } else {
                "Hide whitespace changes"
              })
              .on_click(cx.listener(|this, _, _, cx| this.toggle_hide_whitespace(cx))),
          )
        },
      )
      .when(
        self.editor.is_some()
          && self.selected_file_has_changes(cx)
          && !(self.show_preview && self.previewable()),
        |this| {
          let split_disabled = self.split_disabled(cx);
          let (label, icon) = if split_disabled || self.diff_view == DiffViewMode::Inline {
            ("Split", gpui_component::IconName::PanelLeft)
          } else {
            ("Inline", gpui_component::IconName::PanelLeftClose)
          };
          this.child(
            Button::new("session-page-diff-view-toggle")
              .debug_selector(|| DIFF_VIEW_TOGGLE_DEBUG_SELECTOR.to_string())
              .label(label)
              .icon(icon)
              .xsmall()
              .ghost()
              .disabled(split_disabled)
              .tooltip("Toggle inline and split diff (cmd-/)")
              .on_click(cx.listener(|this, _, _, cx| this.toggle_diff_view(cx))),
          )
        },
      )
      .when(save_editor.is_some(), |this| {
        let save_editor = save_editor.clone();
        this.child(
          Button::new("session-page-save-file")
            .label("Save")
            .xsmall()
            .ghost()
            .disabled(!file_dirty)
            .on_click(move |_, _, cx| {
              if let Some(editor) = save_editor.clone() {
                editor.update(cx, |editor, cx| editor.save(cx));
              }
            }),
        )
      })
      .when(copyable_count > 0, |this| {
        this.child(
          Button::new("session-page-send-review")
            .primary()
            .compact()
            .small()
            .label(if copyable_count == 1 {
              "Send 1 comment to agent".to_string()
            } else {
              format!("Send {copyable_count} comments to agent")
            })
            .tooltip("Send review comments to the agent (cmd-shift-a)")
            .on_click(cx.listener(|this, _, window, cx| {
              this.send_agent_review_to_agent(window, cx);
            })),
        )
      })
      .into_any_element()
  }

  pub(super) fn render_diff_view(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let theme = cx.theme().clone();
    let body: AnyElement = if let Some(preview) = self.binary_preview.as_ref() {
      render_binary_preview(preview, cx)
    } else if let Some(editor) = self.editor.clone() {
      // Actions of the hovered hunk or conflict float over the editor.
      let hunk_actions = (self.opened_commit.is_none())
        .then(|| {
          let file_status = self.selected_file_status(cx);
          render_hunk_actions(&editor, file_status, cx)
        })
        .flatten();
      let editor_pane = div()
        .flex_1()
        .min_h_0()
        .min_w(px(0.0))
        .flex()
        .flex_col()
        .relative()
        .overflow_hidden()
        .debug_selector(|| DIFF_EDITOR_DEBUG_SELECTOR.to_string())
        .child(editor.clone())
        .children(hunk_actions)
        .into_any_element();

      if self.show_preview && self.previewable() {
        // A toggle, not a split: the rendered file takes the pane. Its children
        // size themselves with flex_1, hence the flex column here.
        let preview_pane = crate::file_preview::render_preview_pane(
          "session-preview-text",
          &editor,
          &self.svg_preview,
          self.selected_file_is_svg(),
          window,
          cx,
        );
        div()
          .flex_1()
          .min_h_0()
          .min_w(px(0.0))
          .flex()
          .flex_col()
          .debug_selector(|| PREVIEW_PANE_DEBUG_SELECTOR.to_string())
          .child(preview_pane)
          .into_any_element()
      } else {
        editor_pane
      }
    } else {
      v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .child(
          div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child("Loading diff..."),
        )
        .into_any_element()
    };

    v_flex()
      .size_full()
      .min_w(px(0.0))
      .min_h_0()
      .bg(theme.background)
      .child(self.render_diff_header(cx))
      .child(body)
      .into_any_element()
  }

  pub(super) fn render_dock_panel(&mut self, _cx: &mut Context<Self>) -> AnyElement {
    div()
      .size_full()
      .min_w(px(0.0))
      .min_h_0()
      .child(self.dock_panel.clone())
      .into_any_element()
  }
}

impl Render for SessionPage {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    div()
      .size_full()
      .min_h_0()
      .track_focus(&self.focus_handle)
      .on_action(cx.listener(Self::close_workspace_page_action))
      .on_action(cx.listener(Self::close_file_view_action))
      .on_action(cx.listener(Self::find_action))
      .on_action(cx.listener(Self::add_selection_to_agent_action))
      .on_action(cx.listener(Self::show_command_palette_action))
      .on_action(cx.listener(Self::show_file_search_action))
      .on_action(cx.listener(Self::send_review_comments_to_agent_action))
      .on_action(cx.listener(Self::comment_hunk_action))
      .on_action(cx.listener(Self::toggle_diff_view_action))
      .on_action(cx.listener(Self::toggle_hide_whitespace_action))
      .on_action(cx.listener(Self::previous_annotation_action))
      .on_action(cx.listener(Self::next_annotation_action))
      .on_action(cx.listener(Self::toggle_hunk_stage_action))
      .on_action(cx.listener(Self::restore_hunk_action))
      .on_action(cx.listener(Self::accept_both_conflict_action))
      .on_action(cx.listener(Self::open_repository_action))
      .on_action(cx.listener(Self::pull_changes_action))
      .on_action(cx.listener(Self::push_changes_action))
      .on_action(cx.listener(Self::force_push_changes_action))
      .on_action(cx.listener(Self::show_branch_switcher_action))
      .on_action(cx.listener(Self::toggle_terminal_action))
      .on_action(cx.listener(Self::open_history_action))
      .on_action(cx.listener(Self::open_changes_action))
      .on_action(cx.listener(Self::toggle_file_stage_action))
      .on_action(cx.listener(Self::restore_file_action))
      .child(
        ui::h_resizable("session-page-shell")
          .child(
            ui::resizable_panel()
              .size(px(SESSIONS_SIDEBAR_DEFAULT_WIDTH))
              .size_range(px(SESSIONS_SIDEBAR_MIN_WIDTH)..px(SESSIONS_SIDEBAR_MAX_WIDTH))
              .child(self.render_sessions_sidebar(cx)),
          )
          .child(ui::resizable_panel().child(self.render_center(window, cx)))
          .child(
            ui::resizable_panel()
              .size(px(DOCK_PANEL_DEFAULT_WIDTH))
              .size_range(px(DOCK_PANEL_MIN_WIDTH)..px(DOCK_PANEL_MAX_WIDTH))
              .child(self.render_dock_panel(cx)),
          ),
      )
  }
}

#[cfg(test)]
mod tests {
  use super::super::test_support::*;
  use super::*;
  use crate::test_support::{TempRepo, commit_text_file};
  use gpui::TestAppContext;
  use std::path::Path;
  use ui::CommandPaletteCommandId;
  #[gpui::test]
  async fn the_repo_line_is_painted_without_connecting_an_agent(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-repo-line");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    page.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    assert!(
      cx.debug_bounds(REPO_CONTEXT_DEBUG_SELECTOR).is_some(),
      "the repository line should be painted"
    );
    page.read_with(cx, |page, _| {
      // Rendering must not spawn the agent; the workspace calls activate.
      assert!(page.agent_chat_view.is_none());
    });
  }

  #[gpui::test]
  async fn sync_counters_are_painted_only_when_there_is_something_to_sync(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-counter-paint");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let _remote = publish_to_new_remote(&repo.path, "session-page-counter-paint");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    page.update(cx, |page, cx| page.refresh_branch(cx));
    await_branch_refresh(&page, cx).await;
    page.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    assert!(
      cx.debug_bounds(REPO_AHEAD_DEBUG_SELECTOR).is_none(),
      "nothing to push, no counter"
    );
    assert!(cx.debug_bounds(REPO_BEHIND_DEBUG_SELECTOR).is_none());

    commit_text_file(&repo.path, Path::new("README.md"), "v2\n", "second");
    page.update(cx, |page, cx| page.refresh_branch(cx));
    await_branch_refresh(&page, cx).await;
    page.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    assert!(
      cx.debug_bounds(REPO_AHEAD_DEBUG_SELECTOR).is_some(),
      "one commit to push, the counter shows up"
    );
    assert!(cx.debug_bounds(REPO_BEHIND_DEBUG_SELECTOR).is_none());
  }

  #[gpui::test]
  async fn clicking_the_ahead_counter_pushes_instead_of_switching_repository(
    cx: &mut TestAppContext,
  ) {
    let repo = TempRepo::init("session-page-counter-click");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let remote = publish_to_new_remote(&repo.path, "session-page-counter-click");
    commit_text_file(&repo.path, Path::new("README.md"), "v2\n", "second");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    page.update(cx, |page, cx| page.refresh_branch(cx));
    await_branch_refresh(&page, cx).await;
    page.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    let counter = cx
      .debug_bounds(REPO_AHEAD_DEBUG_SELECTOR)
      .expect("ahead counter bounds");
    cx.simulate_click(counter.center(), gpui::Modifiers::default());

    let command_task = page.update(cx, |page, _| {
      page._repo_command_task.take().expect("push task")
    });
    command_task.await;
    cx.run_until_parked();

    // The click ran the push and did not open the repository switcher.
    let remote_repo = git2::Repository::open(&remote).expect("open remote");
    let head = remote_repo
      .head()
      .and_then(|head| head.peel_to_commit())
      .expect("remote head");
    assert_eq!(head.summary(), Some("second"));

    // The row under the counter opens the repository switcher: it must not fire.
    let switcher_open = cx.update(|window, cx| window.has_active_dialog(cx));
    assert!(!switcher_open, "the repository switcher should stay closed");
  }

  #[gpui::test]
  async fn a_renamed_file_names_both_sides_in_the_header(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-rename-header");
    commit_text_file(&repo.path, Path::new("old_name.rs"), "v1\n", "initial");
    std::fs::rename(repo.path.join("old_name.rs"), repo.path.join("new_name.rs"))
      .expect("rename file");
    git::stage_all(&repo.path).expect("stage the rename");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| panel.refresh(cx))
    });
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("new_name.rs"), None, window, cx);
    });
    await_open_file(&page, cx).await;

    assert!(
      cx.debug_bounds(crate::file_view::FILE_TITLE_OLD_NAME_DEBUG_SELECTOR)
        .is_some(),
      "reading the diff of a moved file has to say where it came from"
    );

    // A plain modification names one file only.
    std::fs::write(repo.path.join("plain.rs"), "v1\n").expect("write file");
    git::stage_all(&repo.path).expect("stage the new file");
    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| panel.refresh(cx))
    });
    cx.run_until_parked();
    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("plain.rs"), None, window, cx);
    });
    await_open_file(&page, cx).await;

    assert!(
      cx.debug_bounds(crate::file_view::FILE_TITLE_OLD_NAME_DEBUG_SELECTOR)
        .is_none()
    );
  }

  #[gpui::test]
  async fn the_diff_view_toggle_flips_the_mode_and_persists_it(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-diff-toggle");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| panel.refresh(cx))
    });
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("README.md"), None, window, cx);
    });
    await_open_file(&page, cx).await;

    page.read_with(cx, |page, _| {
      assert_eq!(page.diff_view, DiffViewMode::Inline);
    });

    // The user's path: the button in the diff header.
    page.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    let toggle = cx
      .debug_bounds(DIFF_VIEW_TOGGLE_DEBUG_SELECTOR)
      .expect("diff view toggle bounds");
    cx.simulate_click(toggle.center(), gpui::Modifiers::default());
    cx.run_until_parked();

    page.read_with(cx, |page, cx| {
      assert_eq!(page.diff_view, DiffViewMode::Split);
      // The PR Changes tab reads the same preference.
      assert!(crate::config::AppSettings::get(cx).split_diff_view);
      assert_eq!(
        page
          .editor
          .as_ref()
          .expect("editor")
          .read(cx)
          .diff_view_mode(),
        DiffViewMode::Split
      );
    });

    page.update(cx, |page, cx| page.toggle_diff_view(cx));
    page.read_with(cx, |page, cx| {
      assert_eq!(page.diff_view, DiffViewMode::Inline);
      assert!(!crate::config::AppSettings::get(cx).split_diff_view);
    });
  }

  #[gpui::test]
  async fn a_file_with_only_one_side_stays_inline(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-diff-toggle-untracked");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("new.txt"), "brand new\n").expect("write untracked file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| panel.refresh(cx))
    });
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("new.txt"), None, window, cx);
    });
    await_open_file(&page, cx).await;

    page.read_with(cx, |page, cx| {
      // An untracked file has nothing to show on the left.
      assert!(page.split_disabled(cx));
      assert_eq!(
        page
          .editor
          .as_ref()
          .expect("editor")
          .read(cx)
          .diff_view_mode(),
        DiffViewMode::Inline,
        "it should open inline whatever the preference says"
      );
    });

    // The button is painted but inert.
    page.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    let toggle = cx
      .debug_bounds(DIFF_VIEW_TOGGLE_DEBUG_SELECTOR)
      .expect("diff view toggle bounds");
    cx.simulate_click(toggle.center(), gpui::Modifiers::default());
    cx.run_until_parked();

    page.read_with(cx, |page, _| {
      assert_eq!(page.diff_view, DiffViewMode::Inline);
    });
  }

  #[gpui::test]
  async fn a_file_without_changes_has_no_diff_controls(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-diff-toggle-clean");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| panel.refresh(cx))
    });
    cx.run_until_parked();

    // Opened from the Files tab: committed content, nothing to compare.
    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("README.md"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    page.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    page.read_with(cx, |page, cx| assert!(!page.selected_file_has_changes(cx)));
    assert!(
      cx.debug_bounds(DIFF_VIEW_TOGGLE_DEBUG_SELECTOR).is_none(),
      "showing a split of a file against itself helps nobody"
    );
    assert!(
      cx.debug_bounds(WHITESPACE_TOGGLE_DEBUG_SELECTOR).is_none(),
      "there are no whitespace changes to hide either"
    );

    // The shortcut is inert too.
    page.update(cx, |page, cx| {
      page.toggle_hide_whitespace(cx);
      assert!(!page.hide_whitespace);
    });
  }

  #[gpui::test]
  async fn a_markdown_file_can_be_read_rendered(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-md-preview");
    commit_text_file(&repo.path, Path::new("README.md"), "# Title\n", "initial");
    std::fs::write(repo.path.join("README.md"), "# Title\n\nBody\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("README.md"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    page.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    assert!(
      cx.debug_bounds(PREVIEW_PANE_DEBUG_SELECTOR).is_none(),
      "the file opens as text"
    );

    let toggle = cx
      .debug_bounds(PREVIEW_TOGGLE_DEBUG_SELECTOR)
      .expect("preview toggle bounds");
    cx.simulate_click(toggle.center(), gpui::Modifiers::default());
    cx.run_until_parked();

    page.read_with(cx, |page, _| assert!(page.show_preview));
    let content = cx
      .debug_bounds(crate::file_preview::PREVIEW_CONTENT_DEBUG_SELECTOR)
      .expect("rendered markdown bounds");
    assert!(
      content.size.width > gpui::px(0.0) && content.size.height > gpui::px(0.0),
      "an empty pane means the rendered file is nowhere to be seen"
    );
    // The rendered file replaces the editor instead of sitting beside it.
    let pane = cx
      .debug_bounds(PREVIEW_PANE_DEBUG_SELECTOR)
      .expect("preview pane bounds");
    assert!(
      content.size.width >= pane.size.width - gpui::px(1.0),
      "the preview should take the whole pane, not half of it"
    );
  }

  #[gpui::test]
  async fn a_plain_file_has_no_preview_toggle(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-no-preview");
    commit_text_file(
      &repo.path,
      Path::new("main.rs"),
      "fn main() {}\n",
      "initial",
    );
    std::fs::write(repo.path.join("main.rs"), "fn main() { }\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("main.rs"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    page.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    page.read_with(cx, |page, _| assert!(!page.previewable()));
    assert!(cx.debug_bounds(PREVIEW_TOGGLE_DEBUG_SELECTOR).is_none());
  }

  #[gpui::test]
  async fn the_preview_takes_the_pane_and_hides_the_diff_controls(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-preview-vs-split");
    commit_text_file(&repo.path, Path::new("README.md"), "# Title\n", "initial");
    std::fs::write(repo.path.join("README.md"), "# Title\n\nBody\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| panel.refresh(cx))
    });
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("README.md"), None, window, cx);
    });
    await_open_file(&page, cx).await;

    page.update(cx, |page, cx| {
      page.toggle_preview(cx);
      assert!(page.show_preview);
    });
    cx.run_until_parked();

    // The rendered file takes the whole pane: no diff mode left to choose.
    assert!(cx.debug_bounds(PREVIEW_PANE_DEBUG_SELECTOR).is_some());
    assert!(
      cx.debug_bounds(DIFF_VIEW_TOGGLE_DEBUG_SELECTOR).is_none(),
      "the split toggle has nothing to act on while previewing"
    );

    assert!(
      cx.debug_bounds(WHITESPACE_TOGGLE_DEBUG_SELECTOR).is_none(),
      "there is no diff on screen to hide whitespace in"
    );

    // The shortcuts are inert too.
    page.update(cx, |page, cx| {
      page.toggle_diff_view(cx);
      assert_eq!(page.diff_view, DiffViewMode::Inline);
      page.toggle_hide_whitespace(cx);
      assert!(!page.hide_whitespace);
    });

    // Back to the code, the toggle is there again.
    page.update(cx, |page, cx| page.toggle_preview(cx));
    cx.run_until_parked();
    assert!(cx.debug_bounds(PREVIEW_PANE_DEBUG_SELECTOR).is_none());
    assert!(cx.debug_bounds(DIFF_VIEW_TOGGLE_DEBUG_SELECTOR).is_some());
  }

  #[gpui::test]
  async fn an_svg_file_renders_as_an_image(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-svg-preview");
    let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"8\" height=\"8\"></svg>\n";
    commit_text_file(&repo.path, Path::new("logo.svg"), svg, "initial");
    std::fs::write(
      repo.path.join("logo.svg"),
      svg.replace("width=\"8\"", "width=\"16\""),
    )
    .expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("logo.svg"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    page.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    page.read_with(cx, |page, _| assert!(page.selected_file_is_svg()));

    let toggle = cx
      .debug_bounds(PREVIEW_TOGGLE_DEBUG_SELECTOR)
      .expect("preview toggle bounds");
    cx.simulate_click(toggle.center(), gpui::Modifiers::default());
    cx.run_until_parked();

    let pane = cx
      .debug_bounds(PREVIEW_PANE_DEBUG_SELECTOR)
      .expect("preview pane bounds");
    assert!(pane.size.width > gpui::px(0.0) && pane.size.height > gpui::px(0.0));
  }

  #[gpui::test]
  async fn the_preview_does_not_follow_onto_the_next_file(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-preview-then-code");
    commit_text_file(&repo.path, Path::new("README.md"), "# Title\n", "initial");
    commit_text_file(&repo.path, Path::new("main.rs"), "fn main() {}\n", "code");
    std::fs::write(repo.path.join("README.md"), "# Title\n\nBody\n").expect("update md");
    std::fs::write(repo.path.join("main.rs"), "fn main() { }\n").expect("update rs");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("README.md"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    page.update(cx, |page, cx| page.toggle_preview(cx));
    cx.run_until_parked();
    assert!(cx.debug_bounds(PREVIEW_PANE_DEBUG_SELECTOR).is_some());

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("main.rs"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    page.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    // Opening a file shows its code, whatever the previous file was showing.
    page.read_with(cx, |page, _| assert!(!page.show_preview));
    assert!(cx.debug_bounds(PREVIEW_PANE_DEBUG_SELECTOR).is_none());
    assert!(cx.debug_bounds(PREVIEW_TOGGLE_DEBUG_SELECTOR).is_none());
  }

  #[gpui::test]
  async fn coming_back_to_a_previewed_file_shows_its_code(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-preview-reset");
    commit_text_file(&repo.path, Path::new("README.md"), "# Title\n", "initial");
    commit_text_file(&repo.path, Path::new("GUIDE.md"), "# Guide\n", "guide");
    std::fs::write(repo.path.join("README.md"), "# Title\n\nBody\n").expect("update readme");
    std::fs::write(repo.path.join("GUIDE.md"), "# Guide\n\nSteps\n").expect("update guide");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("README.md"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    page.update(cx, |page, cx| page.toggle_preview(cx));
    cx.run_until_parked();
    assert!(cx.debug_bounds(PREVIEW_PANE_DEBUG_SELECTOR).is_some());

    // Another markdown file: the preview does not carry over.
    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("GUIDE.md"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    page.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    page.read_with(cx, |page, _| assert!(!page.show_preview));
    assert!(cx.debug_bounds(PREVIEW_PANE_DEBUG_SELECTOR).is_none());
    assert!(
      cx.debug_bounds(PREVIEW_TOGGLE_DEBUG_SELECTOR).is_some(),
      "the button is still offered, it just starts off"
    );
  }

  #[gpui::test]
  async fn revealing_a_line_in_the_previewed_file_shows_the_code(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-preview-reveal");
    commit_text_file(&repo.path, Path::new("README.md"), "# Title\n", "initial");
    std::fs::write(repo.path.join("README.md"), "# Title\n\nBody\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("README.md"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    page.update(cx, |page, cx| page.toggle_preview(cx));
    cx.run_until_parked();
    assert!(cx.debug_bounds(PREVIEW_PANE_DEBUG_SELECTOR).is_some());

    // The agent points at a line of the file already on screen: a rendered
    // document has no line to jump to.
    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("README.md"), Some(3), window, cx);
    });
    cx.run_until_parked();

    page.read_with(cx, |page, _| assert!(!page.show_preview));
    assert!(cx.debug_bounds(PREVIEW_PANE_DEBUG_SELECTOR).is_none());
  }

  #[gpui::test]
  async fn the_whitespace_toggle_reaches_the_editor_and_survives_the_next_file(
    cx: &mut TestAppContext,
  ) {
    let repo = TempRepo::init("session-page-whitespace");
    commit_text_file(&repo.path, Path::new("a.rs"), "fn main() {}\n", "initial");
    commit_text_file(&repo.path, Path::new("b.rs"), "fn other() {}\n", "second");
    std::fs::write(repo.path.join("a.rs"), "fn main() { }\n").expect("update a");
    std::fs::write(repo.path.join("b.rs"), "fn other() { }\n").expect("update b");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| panel.refresh(cx))
    });
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("a.rs"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    page.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    page.read_with(cx, |page, cx| {
      assert!(!page.hide_whitespace);
      assert!(
        !page
          .editor
          .as_ref()
          .expect("editor")
          .read(cx)
          .ignore_whitespace()
      );
    });

    let toggle = cx
      .debug_bounds(WHITESPACE_TOGGLE_DEBUG_SELECTOR)
      .expect("whitespace toggle bounds");
    cx.simulate_click(toggle.center(), gpui::Modifiers::default());
    cx.run_until_parked();

    page.read_with(cx, |page, cx| {
      assert!(page.hide_whitespace);
      assert!(
        page
          .editor
          .as_ref()
          .expect("editor")
          .read(cx)
          .ignore_whitespace()
      );
    });

    // A reading preference for the session, not a per-file one.
    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("b.rs"), None, window, cx);
    });
    await_open_file(&page, cx).await;

    page.read_with(cx, |page, cx| {
      assert!(page.hide_whitespace);
      assert!(
        page
          .editor
          .as_ref()
          .expect("editor")
          .read(cx)
          .ignore_whitespace()
      );
    });
  }

  #[gpui::test]
  async fn the_first_file_of_the_session_follows_the_whitespace_setting(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-whitespace-setting");
    commit_text_file(&repo.path, Path::new("a.rs"), "fn main() {}\n", "initial");
    std::fs::write(repo.path.join("a.rs"), "fn main() { }\n").expect("update a");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    page.update(cx, |page, cx| {
      crate::config::AppSettings::update(cx, |settings| settings.hide_whitespace = true);
      page.dock_panel.update(cx, |panel, cx| panel.refresh(cx))
    });
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("a.rs"), None, window, cx);
    });
    await_open_file(&page, cx).await;

    page.read_with(cx, |page, cx| {
      assert!(page.hide_whitespace);
      assert!(
        page
          .editor
          .as_ref()
          .expect("editor")
          .read(cx)
          .ignore_whitespace()
      );
    });
  }

  #[gpui::test]
  async fn the_diff_shortcuts_reach_the_shell(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-diff-actions");
    let original = (1..=60)
      .map(|line| format!("line {line}\n"))
      .collect::<String>();
    commit_text_file(&repo.path, Path::new("a.rs"), &original, "initial");
    let modified = original
      .replace("line 5\n", "line 5 changed\n")
      .replace("line 50\n", "line 50 changed\n");
    std::fs::write(repo.path.join("a.rs"), modified).expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| panel.refresh(cx))
    });
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("a.rs"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    await_editor_diff(&page, cx).await;

    // The actions the keybindings dispatch, not the methods behind them.
    cx.dispatch_action(crate::NextAnnotation);
    cx.run_until_parked();
    page.read_with(cx, |page, cx| {
      let state = page
        .editor
        .as_ref()
        .expect("editor")
        .read(cx)
        .hunk_navigation_state(cx)
        .expect("hunk navigation state");
      assert_eq!(state.active_index, 1);
    });

    cx.dispatch_action(crate::PreviousAnnotation);
    cx.run_until_parked();
    page.read_with(cx, |page, cx| {
      let state = page
        .editor
        .as_ref()
        .expect("editor")
        .read(cx)
        .hunk_navigation_state(cx)
        .expect("hunk navigation state");
      assert_eq!(state.active_index, 0);
    });

    cx.dispatch_action(crate::ToggleHideWhitespace);
    cx.run_until_parked();
    page.read_with(cx, |page, cx| {
      assert!(page.hide_whitespace);
      assert!(
        page
          .editor
          .as_ref()
          .expect("editor")
          .read(cx)
          .ignore_whitespace()
      );
    });

    cx.dispatch_action(crate::ToggleDiffView);
    cx.run_until_parked();
    page.read_with(cx, |page, _| {
      assert_eq!(page.diff_view, DiffViewMode::Split);
    });
  }

  #[gpui::test]
  async fn the_dock_shortcuts_open_their_tab(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-render-dock-shortcuts");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_history_action(&crate::OpenGitHistorySidebar, window, cx)
    });
    cx.run_until_parked();
    page.read_with(cx, |page, cx| {
      assert_eq!(
        page.dock_panel.read(cx).active_tab(),
        crate::dock_panel::DockPanelTab::History
      );
    });

    page.update_in(cx, |page, window, cx| {
      page.toggle_terminal_action(&crate::ToggleTerminalSidebar, window, cx)
    });
    cx.run_until_parked();
    page.read_with(cx, |page, cx| {
      let panel = page.dock_panel.read(cx);
      assert_eq!(
        panel.active_tab(),
        crate::dock_panel::DockPanelTab::Terminal
      );
      assert!(
        panel.has_terminal(),
        "opening the tab starts the shell, as clicking it does"
      );
    });

    page.update_in(cx, |page, window, cx| {
      page.open_changes_action(&crate::OpenGitChangesSidebar, window, cx)
    });
    cx.run_until_parked();
    page.update_in(cx, |page, window, cx| {
      let panel = page.dock_panel.read(cx);
      assert_eq!(panel.active_tab(), crate::dock_panel::DockPanelTab::Changes);
      assert!(
        panel.changes_list().read(cx).is_focused(window, cx),
        "the keyboard keeps going in the list the shortcut opened"
      );
    });
  }

  #[gpui::test]
  async fn a_sync_shortcut_runs_nothing_when_its_command_cannot(cx: &mut TestAppContext) {
    let (repo, remote, branch) = {
      let repo = TempRepo::init("session-render-sync-shortcut");
      let remote = crate::test_support::TempBareRepo::init("session-render-sync-remote");
      commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");
      git2::Repository::open(&repo.path)
        .expect("open repo")
        .remote("origin", remote.path.to_str().expect("remote path utf8"))
        .expect("add origin");
      let branch = git::current_branch_status(&repo.path)
        .expect("branch status")
        .name;
      crate::test_support::push_branch_to_remote(&repo.path, &branch, "origin");
      crate::test_support::set_upstream(&repo.path, &branch, &format!("origin/{branch}"));
      (repo, remote, branch)
    };

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    page.update(cx, |page, cx| page.refresh_branch(cx));
    await_branch_refresh(&page, cx).await;

    // Nothing ahead: the shortcut is inert.
    page.update_in(cx, |page, window, cx| {
      page.push_changes_action(&crate::PushChanges, window, cx)
    });
    page.read_with(cx, |page, _| {
      assert!(
        page._repo_command_task.is_none(),
        "pressing push with nothing to push starts no command"
      );
    });

    // One commit ahead: the same key pushes.
    commit_text_file(&repo.path, Path::new("a.txt"), "v2\n", "ahead");
    page.update(cx, |page, cx| page.refresh_branch(cx));
    await_branch_refresh(&page, cx).await;
    page.update_in(cx, |page, window, cx| {
      page.push_changes_action(&crate::PushChanges, window, cx)
    });
    let command = page.update(cx, |page, _| {
      page._repo_command_task.take().expect("push task")
    });
    command.await;
    cx.run_until_parked();

    assert_eq!(
      crate::test_support::remote_branch_oid(&remote.path, &branch),
      crate::test_support::head_oid(&repo.path)
    );
  }

  #[gpui::test]
  async fn the_branch_switcher_needs_branches(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-render-branch-switcher");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    // Branches are not loaded yet: the shortcut opens nothing.
    page.update_in(cx, |page, window, cx| {
      page.show_branch_switcher_action(&crate::ShowBranchSwitcher, window, cx)
    });
    cx.run_until_parked();
    assert!(!cx.update(|window, cx| window.has_active_dialog(cx)));

    page.update(cx, |page, cx| page.refresh_branch(cx));
    await_branch_refresh(&page, cx).await;
    page.update_in(cx, |page, window, cx| {
      page.show_branch_switcher_action(&crate::ShowBranchSwitcher, window, cx)
    });
    cx.run_until_parked();
    assert!(
      cx.update(|window, cx| window.has_active_dialog(cx)),
      "the palette opens on the branch screen"
    );
  }

  #[gpui::test]
  async fn the_file_shortcuts_stage_and_restore_what_is_open(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-render-file-shortcuts");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");
    std::fs::write(repo.path.join("a.txt"), "v2\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let refresh = page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("a.txt"), None, window, cx);
    });
    await_open_file(&page, cx).await;

    let changes_task = |page: &Entity<SessionPage>, cx: &mut gpui::VisualTestContext| {
      page.update(cx, |page, cx| {
        page
          .dock_panel
          .read(cx)
          .changes_list()
          .update(cx, |list, _| list._action_task.take())
      })
    };

    // `cmd-enter` stages the open file, and stages it back off.
    page.update_in(cx, |page, window, cx| {
      page.toggle_file_stage_action(&crate::ToggleFileStage, window, cx)
    });
    changes_task(&page, cx).expect("staging task").await;
    cx.run_until_parked();
    assert_eq!(
      git::list_repo_status(&repo.path).expect("status")[0].stage,
      git::RepoStage::Staged
    );

    let refresh = page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    page.update_in(cx, |page, window, cx| {
      page.toggle_file_stage_action(&crate::ToggleFileStage, window, cx)
    });
    changes_task(&page, cx).expect("unstaging task").await;
    cx.run_until_parked();
    assert_eq!(
      git::list_repo_status(&repo.path).expect("status")[0].stage,
      git::RepoStage::Unstaged
    );

    // `cmd-backspace` throws the change away.
    let refresh = page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    page.update_in(cx, |page, window, cx| {
      page.restore_file_action(&crate::RestoreFile, window, cx)
    });
    changes_task(&page, cx).expect("restore task").await;
    cx.run_until_parked();
    assert_eq!(
      std::fs::read_to_string(repo.path.join("a.txt")).expect("read file"),
      "v1\n"
    );
  }

  #[gpui::test]
  async fn find_opens_on_the_file_and_escape_closes_the_search_first(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-render-find");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");
    std::fs::write(repo.path.join("a.txt"), "v2\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("a.txt"), None, window, cx);
    });
    await_open_file(&page, cx).await;

    let find_open = |page: &Entity<SessionPage>, cx: &mut gpui::VisualTestContext| {
      page.read_with(cx, |page, cx| {
        page
          .editor
          .as_ref()
          .expect("editor")
          .read(cx)
          .is_find_panel_open()
      })
    };
    assert!(!find_open(&page, cx));

    page.update_in(cx, |page, window, cx| {
      page.find_action(&editor::Find, window, cx)
    });
    cx.run_until_parked();
    assert!(find_open(&page, cx), "cmd-f opens the search on the diff");

    // Escape closes the search, and the file stays open.
    page.update_in(cx, |page, window, cx| {
      let editor = page.editor.clone().expect("editor");
      editor.update(cx, |editor, cx| {
        editor::close_find(editor, &editor::CloseFind, window, cx)
      });
    });
    cx.run_until_parked();
    assert!(!find_open(&page, cx));
    page.read_with(cx, |page, _| {
      assert_eq!(page.center, CenterView::Diff);
      assert!(page.selected_file.is_some());
    });

    // With no search left to close, escape closes the file.
    page.update_in(cx, |page, window, cx| {
      page.close_file_view_action(&editor::CloseFind, window, cx)
    });
    page.read_with(cx, |page, _| {
      assert_eq!(page.center, CenterView::Conversation);
    });
  }

  #[gpui::test]
  async fn sending_a_selection_needs_a_file_and_a_selection(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-render-selection");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");
    std::fs::write(repo.path.join("a.txt"), "v2\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    // No diff on screen: the shell says so instead of spawning an agent.
    page.update_in(cx, |page, window, cx| {
      page.add_selection_to_agent_action(&crate::AddSelectionToAgent, window, cx)
    });
    cx.run_until_parked();
    page.read_with(cx, |page, _| {
      assert!(page.agent_chat_view.is_none(), "no agent was started");
    });

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("a.txt"), None, window, cx);
    });
    await_open_file(&page, cx).await;

    // A diff, but nothing selected: same, nothing is delivered.
    page.update_in(cx, |page, window, cx| {
      page.add_selection_to_agent_action(&crate::AddSelectionToAgent, window, cx)
    });
    cx.run_until_parked();
    page.read_with(cx, |page, cx| {
      assert_eq!(
        page.selection_context(cx),
        Err("Select code in the diff first")
      );
      assert!(page.agent_chat_view.is_none());
    });

    // With a selection, the file and its text are what travels to the agent.
    page.update_in(cx, |page, window, cx| {
      let editor = page.editor.clone().expect("editor");
      editor.update(cx, |editor, cx| {
        editor::select_all(editor, &editor::SelectAll, window, cx)
      });
    });
    page.read_with(cx, |page, cx| {
      let (path, text) = page.selection_context(cx).expect("a selection to send");
      assert_eq!(path, "a.txt");
      assert!(text.contains("v2"), "the selected lines travel as text");
    });
  }

  #[gpui::test]
  async fn a_hunk_is_staged_and_unstaged_from_the_diff(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-render-hunk-staging");
    let original = (1..=40)
      .map(|line| format!("line {line}\n"))
      .collect::<String>();
    commit_text_file(&repo.path, Path::new("a.txt"), &original, "initial");
    // Two hunks far apart: staging one must leave the other alone.
    let modified = original
      .replace("line 3\n", "line 3 changed\n")
      .replace("line 30\n", "line 30 changed\n");
    std::fs::write(repo.path.join("a.txt"), modified).expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let refresh = page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("a.txt"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    await_editor_diff(&page, cx).await;

    // Hovering a hunk brings up its actions.
    page.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    let editor_bounds = cx
      .debug_bounds(crate::hunk_actions::HUNK_ACTIONS_DEBUG_SELECTOR)
      .is_some();
    assert!(
      !editor_bounds,
      "nothing floats over the diff until a hunk is hovered"
    );
    let hunk_line = page.read_with(cx, |page, cx| {
      page
        .editor
        .as_ref()
        .expect("editor")
        .read(cx)
        .hunk_navigation_state(cx)
        .expect("hunk navigation state")
        .active_display_line
    });
    let line_height = page.read_with(cx, |page, cx| {
      page
        .editor
        .as_ref()
        .expect("editor")
        .read(cx)
        .measured_editor_line_height()
    });
    let editor_bounds = cx
      .debug_bounds(DIFF_EDITOR_DEBUG_SELECTOR)
      .expect("editor pane bounds");
    cx.simulate_mouse_move(
      gpui::point(
        editor_bounds.origin.x + gpui::px(200.0),
        editor_bounds.origin.y + line_height * (hunk_line as f32 + 0.5),
      ),
      None,
      gpui::Modifiers::default(),
    );
    cx.run_until_parked();
    assert!(
      cx.debug_bounds(crate::hunk_actions::HUNK_ACTIONS_DEBUG_SELECTOR)
        .is_some(),
      "the hovered hunk carries its own actions"
    );

    // The button of the hovered hunk stages that hunk.
    let stage = cx
      .debug_bounds(crate::hunk_actions::STAGE_HUNK_DEBUG_SELECTOR)
      .expect("stage hunk button bounds");
    cx.simulate_click(stage.center(), gpui::Modifiers::default());
    await_editor_diff(&page, cx).await;

    let staged_lines = |repo_root: &Path| {
      let entries = git::list_repo_status(repo_root).expect("status");
      entries
        .iter()
        .find(|entry| entry.path == PathBuf::from("a.txt"))
        .map(|entry| entry.stage)
    };
    assert_eq!(
      staged_lines(&repo.path),
      Some(git::RepoStage::PartiallyStaged),
      "one hunk staged, the other not"
    );

    // The dock heard about it: the file shows up as staged there too.
    cx.run_until_parked();
    page.read_with(cx, |page, cx| {
      let entry = page
        .dock_panel
        .read(cx)
        .status_entries()
        .iter()
        .find(|entry| entry.path == PathBuf::from("a.txt"))
        .cloned()
        .expect("the file is still in the changes list");
      assert_eq!(entry.stage, git::RepoStage::PartiallyStaged);
    });

    // The keyboard on the same hunk puts it back.
    page.update_in(cx, |page, window, cx| {
      page.toggle_hunk_stage_action(&crate::ToggleHunkStage, window, cx)
    });
    await_editor_diff(&page, cx).await;
    assert_eq!(staged_lines(&repo.path), Some(git::RepoStage::Unstaged));
  }

  #[gpui::test]
  async fn an_untracked_file_has_no_hunk_actions(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-render-hunk-untracked");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");
    let new_file = (1..=20)
      .map(|line| format!("line {line}\n"))
      .collect::<String>();
    std::fs::write(repo.path.join("new.txt"), new_file).expect("write untracked file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let refresh = page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("new.txt"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    await_editor_diff(&page, cx).await;
    page.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    let editor_bounds = cx
      .debug_bounds(DIFF_EDITOR_DEBUG_SELECTOR)
      .expect("editor pane bounds");
    cx.simulate_mouse_move(
      gpui::point(
        editor_bounds.origin.x + gpui::px(200.0),
        editor_bounds.origin.y + gpui::px(30.0),
      ),
      None,
      gpui::Modifiers::default(),
    );
    cx.run_until_parked();

    assert!(
      cx.debug_bounds(crate::hunk_actions::HUNK_ACTIONS_DEBUG_SELECTOR)
        .is_none(),
      "a file git does not track yet has nothing to stage hunk by hunk"
    );
  }

  #[gpui::test]
  async fn a_commit_snapshot_has_no_hunk_actions(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-render-hunk-snapshot");
    let original = (1..=20)
      .map(|line| format!("line {line}\n"))
      .collect::<String>();
    commit_text_file(&repo.path, Path::new("a.txt"), &original, "initial");
    commit_text_file(
      &repo.path,
      Path::new("a.txt"),
      &original.replace("line 3\n", "line 3 changed\n"),
      "second",
    );
    let head = git::current_head_sha(&repo.path)
      .expect("head sha")
      .expect("head sha");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    let history = page.read_with(cx, |page, cx| page.dock_panel.read(cx).history_list.clone());
    history.update(cx, |list, cx| {
      list.open_commit_file(head, PathBuf::from("a.txt"), cx)
    });
    await_open_file(&page, cx).await;
    await_editor_diff(&page, cx).await;
    page.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    let editor_bounds = cx
      .debug_bounds(DIFF_EDITOR_DEBUG_SELECTOR)
      .expect("editor pane bounds");
    cx.simulate_mouse_move(
      gpui::point(
        editor_bounds.origin.x + gpui::px(200.0),
        editor_bounds.origin.y + gpui::px(30.0),
      ),
      None,
      gpui::Modifiers::default(),
    );
    cx.run_until_parked();

    assert!(
      cx.debug_bounds(crate::hunk_actions::HUNK_ACTIONS_DEBUG_SELECTOR)
        .is_none(),
      "a commit snapshot is read-only: nothing to stage"
    );
  }

  #[gpui::test]
  async fn restoring_a_hunk_puts_its_lines_back(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-render-hunk-restore");
    let original = (1..=40)
      .map(|line| format!("line {line}\n"))
      .collect::<String>();
    commit_text_file(&repo.path, Path::new("a.txt"), &original, "initial");
    let modified = original
      .replace("line 3\n", "line 3 changed\n")
      .replace("line 30\n", "line 30 changed\n");
    std::fs::write(repo.path.join("a.txt"), modified).expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let refresh = page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("a.txt"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    await_editor_diff(&page, cx).await;

    page.update_in(cx, |page, window, cx| {
      page.restore_hunk_action(&crate::RestoreHunk, window, cx)
    });
    await_editor_diff(&page, cx).await;
    cx.run_until_parked();

    let contents = std::fs::read_to_string(repo.path.join("a.txt")).expect("read file");
    assert!(
      contents.contains("line 3\n"),
      "the restored hunk is back to its committed lines"
    );
    assert!(
      contents.contains("line 30 changed\n"),
      "the other hunk is untouched"
    );
  }

  #[gpui::test]
  async fn accepting_one_conflict_block_leaves_the_others(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-render-conflict-block");
    let base_contents = (1..=12)
      .map(|line| format!("line {line}\n"))
      .collect::<String>();
    commit_text_file(&repo.path, Path::new("a.txt"), &base_contents, "initial");
    let base = git::BranchRef {
      name: git::current_branch_status(&repo.path)
        .expect("branch status")
        .name,
      kind: git::BranchKind::Local,
    };
    let feature = git::BranchRef {
      name: "feature".to_string(),
      kind: git::BranchKind::Local,
    };
    git::create_branch(&repo.path, &feature.name).expect("create branch");
    git::switch_branch(&repo.path, &feature).expect("switch to feature");
    let feature_contents = base_contents
      .replace("line 2\n", "line 2 feature\n")
      .replace("line 11\n", "line 11 feature\n");
    commit_text_file(
      &repo.path,
      Path::new("a.txt"),
      &feature_contents,
      "feature work",
    );
    git::switch_branch(&repo.path, &base).expect("switch back");
    let main_contents = base_contents
      .replace("line 2\n", "line 2 main\n")
      .replace("line 11\n", "line 11 main\n");
    commit_text_file(&repo.path, Path::new("a.txt"), &main_contents, "main work");
    let _ = git::merge_branch(&repo.path, &feature);

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let refresh = page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("a.txt"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    await_editor_diff(&page, cx).await;

    // Hovering a conflict block brings up its three sides.
    page.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    let conflict_line = page.read_with(cx, |page, cx| {
      page
        .editor
        .as_ref()
        .expect("editor")
        .read(cx)
        .conflict_navigation_state(cx)
        .expect("conflict navigation state")
        .active_start_line
    });
    let line_height = page.read_with(cx, |page, cx| {
      page
        .editor
        .as_ref()
        .expect("editor")
        .read(cx)
        .measured_editor_line_height()
    });
    let editor_bounds = cx
      .debug_bounds(DIFF_EDITOR_DEBUG_SELECTOR)
      .expect("editor pane bounds");
    cx.simulate_mouse_move(
      gpui::point(
        editor_bounds.origin.x + gpui::px(200.0),
        editor_bounds.origin.y + line_height * (conflict_line as f32 + 0.5),
      ),
      None,
      gpui::Modifiers::default(),
    );
    cx.run_until_parked();
    assert!(
      cx.debug_bounds(crate::hunk_actions::CONFLICT_ACTIONS_DEBUG_SELECTOR)
        .is_some(),
      "the hovered conflict offers current, incoming and both"
    );

    // `shift-enter` on the first conflict keeps the current side, and only it.
    page.update_in(cx, |page, window, cx| {
      page.toggle_hunk_stage_action(&crate::ToggleHunkStage, window, cx)
    });
    await_editor_diff(&page, cx).await;

    page.read_with(cx, |page, cx| {
      let editor = page.editor.as_ref().expect("editor").read(cx);
      assert!(
        editor.has_unresolved_conflict_markers(cx),
        "the second conflict is still waiting"
      );
      let navigation = page.annotation_navigation(cx).expect("navigation state");
      assert_eq!(navigation.total, 1);
    });

    // `cmd-shift-enter` on the one left keeps both sides.
    page.update_in(cx, |page, window, cx| {
      page.accept_both_conflict_action(&crate::AcceptBothConflict, window, cx)
    });
    await_editor_diff(&page, cx).await;

    let contents = std::fs::read_to_string(repo.path.join("a.txt")).expect("read file");
    assert!(!contents.contains("<<<<<<<"), "no conflict marker is left");
    assert!(contents.contains("line 2 main\n"), "current side kept");
    assert!(contents.contains("line 11 main\n") && contents.contains("line 11 feature\n"));
  }

  #[gpui::test]
  async fn a_conflicted_file_offers_to_accept_a_side_and_walks_conflicts(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-render-conflicts");
    let base_contents = (1..=12)
      .map(|line| format!("line {line}\n"))
      .collect::<String>();
    commit_text_file(&repo.path, Path::new("a.txt"), &base_contents, "initial");
    let base = git::BranchRef {
      name: git::current_branch_status(&repo.path)
        .expect("branch status")
        .name,
      kind: git::BranchKind::Local,
    };
    let feature = git::BranchRef {
      name: "feature".to_string(),
      kind: git::BranchKind::Local,
    };
    git::create_branch(&repo.path, &feature.name).expect("create branch");
    git::switch_branch(&repo.path, &feature).expect("switch to feature");
    // Two conflicting areas, so navigating between conflicts has somewhere to go.
    let feature_contents = base_contents
      .replace("line 2\n", "line 2 feature\n")
      .replace("line 11\n", "line 11 feature\n");
    commit_text_file(
      &repo.path,
      Path::new("a.txt"),
      &feature_contents,
      "feature work",
    );
    git::switch_branch(&repo.path, &base).expect("switch back");
    let main_contents = base_contents
      .replace("line 2\n", "line 2 main\n")
      .replace("line 11\n", "line 11 main\n");
    commit_text_file(&repo.path, Path::new("a.txt"), &main_contents, "main work");
    let _ = git::merge_branch(&repo.path, &feature);

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let refresh = page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("a.txt"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    page.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    page.read_with(cx, |page, cx| {
      assert_eq!(
        page.selected_file_status(cx),
        Some(git::RepoStatusKind::Conflicted)
      );
      assert!(page.can_accept_all_conflicts(cx));
      // The counter walks conflicts, not hunks.
      let navigation = page.annotation_navigation(cx).expect("navigation state");
      assert_eq!(navigation.kind, AnnotationKind::Conflict);
      assert_eq!(navigation.total, 2);
      assert_eq!(navigation.active_index, 0);
    });

    // The shortcut walks conflicts here, not hunks.
    page.update(cx, |page, cx| {
      page.navigate_change(AnnotationDirection::Next, cx);
      let navigation = page.annotation_navigation(cx).expect("navigation state");
      assert_eq!(navigation.active_index, 1);
      assert_eq!(navigation.kind, AnnotationKind::Conflict);
    });

    let accept = cx
      .debug_bounds(ACCEPT_ALL_CURRENT_DEBUG_SELECTOR)
      .expect("accept all current bounds");
    assert!(
      cx.debug_bounds(ACCEPT_ALL_INCOMING_DEBUG_SELECTOR)
        .is_some(),
      "both sides are offered"
    );
    assert!(cx.debug_bounds(ANNOTATION_COUNTER_DEBUG_SELECTOR).is_some());
    cx.simulate_click(accept.center(), gpui::Modifiers::default());
    cx.run_until_parked();

    page.read_with(cx, |page, cx| {
      let editor = page.editor.as_ref().expect("editor").read(cx);
      assert!(
        !editor.has_unresolved_conflict_markers(cx),
        "accepting a side resolves every conflict of the file"
      );
      assert!(
        !page.can_accept_all_conflicts(cx),
        "nothing left to accept once the markers are gone"
      );
    });
    page.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    assert!(
      cx.debug_bounds(ACCEPT_ALL_CURRENT_DEBUG_SELECTOR).is_none(),
      "a resolved file carries no accept-all controls"
    );
    assert!(
      cx.debug_bounds(ACCEPT_ALL_INCOMING_DEBUG_SELECTOR)
        .is_none()
    );
    // Current side of a merge is what HEAD held.
    page.read_with(cx, |page, cx| {
      let first_line = page
        .editor
        .as_ref()
        .expect("editor")
        .read(cx)
        .document()
        .read(cx)
        .line_content(0)
        .expect("first line")
        .to_string();
      assert_eq!(first_line.trim_end(), "line 1");
    });
  }

  #[gpui::test]
  async fn the_palette_accepts_the_incoming_side(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-render-accept-incoming");
    commit_text_file(&repo.path, Path::new("a.txt"), "base\n", "initial");
    let base = git::BranchRef {
      name: git::current_branch_status(&repo.path)
        .expect("branch status")
        .name,
      kind: git::BranchKind::Local,
    };
    let feature = git::BranchRef {
      name: "feature".to_string(),
      kind: git::BranchKind::Local,
    };
    git::create_branch(&repo.path, &feature.name).expect("create branch");
    git::switch_branch(&repo.path, &feature).expect("switch to feature");
    commit_text_file(&repo.path, Path::new("a.txt"), "feature\n", "feature work");
    git::switch_branch(&repo.path, &base).expect("switch back");
    commit_text_file(&repo.path, Path::new("a.txt"), "main\n", "main work");
    let _ = git::merge_branch(&repo.path, &feature);

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let refresh = page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("a.txt"), None, window, cx);
    });
    await_open_file(&page, cx).await;

    page.read_with(cx, |page, cx| {
      let ids = page
        .palette_commands(1, cx)
        .into_iter()
        .map(|command| command.id)
        .collect::<Vec<_>>();
      assert!(ids.contains(&CommandPaletteCommandId::AcceptAllCurrentConflicts));
      assert!(ids.contains(&CommandPaletteCommandId::AcceptAllIncomingConflicts));
    });

    page.update_in(cx, |page, window, cx| {
      page
        .handle_command_palette_action(CommandPaletteAction::AcceptAllIncomingConflicts, window, cx)
        .expect("accept the incoming side")
    });
    cx.run_until_parked();

    // Incoming is what the merged branch held.
    page.read_with(cx, |page, cx| {
      let editor = page.editor.as_ref().expect("editor").read(cx);
      assert!(!editor.has_unresolved_conflict_markers(cx));
      let first_line = editor
        .document()
        .read(cx)
        .line_content(0)
        .expect("first line")
        .to_string();
      assert_eq!(first_line.trim_end(), "feature");
    });
  }

  #[gpui::test]
  async fn accepting_a_side_does_nothing_without_a_conflict(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-render-accept-guard");
    commit_text_file(&repo.path, Path::new("a.txt"), "v1\n", "initial");
    std::fs::write(repo.path.join("a.txt"), "v2\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    let refresh = page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| {
        panel.refresh(cx);
        panel._refresh_task.take().expect("refresh task")
      })
    });
    refresh.await;
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("a.txt"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    page.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    assert!(
      cx.debug_bounds(ACCEPT_ALL_CURRENT_DEBUG_SELECTOR).is_none(),
      "a plain modified file has no side to accept"
    );

    // Dispatched anyway: the file must stay as it is.
    page.update_in(cx, |page, window, cx| {
      page
        .handle_command_palette_action(CommandPaletteAction::AcceptAllCurrentConflicts, window, cx)
        .expect("the action is a no-op")
    });
    cx.run_until_parked();

    page.read_with(cx, |page, cx| {
      let editor = page.editor.as_ref().expect("editor").read(cx);
      assert!(!editor.is_dirty, "nothing was rewritten");
      let first_line = editor
        .document()
        .read(cx)
        .line_content(0)
        .expect("first line")
        .to_string();
      assert_eq!(first_line.trim_end(), "v2");
    });
  }

  #[gpui::test]
  async fn walking_the_changes_moves_through_the_hunks(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-walk-changes");
    let original = (1..=60)
      .map(|line| format!("line {line}\n"))
      .collect::<String>();
    commit_text_file(&repo.path, Path::new("README.md"), &original, "initial");
    // Two changes far apart, so navigating has somewhere to go.
    let modified = original
      .replace("line 5\n", "line 5 changed\n")
      .replace("line 50\n", "line 50 changed\n");
    std::fs::write(repo.path.join("README.md"), modified).expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("README.md"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    await_editor_diff(&page, cx).await;

    let hunk_state = |page: &SessionPage, cx: &App| {
      page
        .editor
        .as_ref()
        .expect("editor")
        .read(cx)
        .hunk_navigation_state(cx)
        .expect("hunk navigation state")
    };

    page.read_with(cx, |page, cx| {
      let state = hunk_state(page, cx);
      assert_eq!(state.total, 2);
      assert_eq!(state.active_index, 0);
    });

    page.update(cx, |page, cx| {
      page.navigate_change(AnnotationDirection::Next, cx);
      assert_eq!(hunk_state(page, cx).active_index, 1);

      // Walking past the last change comes back to the first.
      page.navigate_change(AnnotationDirection::Next, cx);
      assert_eq!(hunk_state(page, cx).active_index, 0);

      page.navigate_change(AnnotationDirection::Previous, cx);
      assert_eq!(hunk_state(page, cx).active_index, 1);

      // A rendered file has no changes to walk.
      page.toggle_preview(cx);
      page.navigate_change(AnnotationDirection::Next, cx);
      assert_eq!(hunk_state(page, cx).active_index, 1);
    });
  }
}
