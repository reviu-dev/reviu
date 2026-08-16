//! Everything the shell paints: sidebar, center, diff header, dock.

use super::*;

impl SessionPage {
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

    let github_section = AuthStateStore::has_github_access(cx).then(|| self.render_inbox(cx));

    let repo_name = self
      .selected_repo
      .as_deref()
      .and_then(|path| path.file_name())
      .map(|name| name.to_string_lossy().into_owned());

    let branch_status = self.branch_status.clone();
    let sync_in_flight = self.repo_command_in_flight;

    let repo_context = repo_name.map(|name| {
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
    });

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

  pub(super) fn render_inbox(&mut self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let notifications = GithubNotificationsStore::list(cx);
    let unread = GithubNotificationsStore::unread_count(cx);

    let header = h_flex()
      .items_center()
      .gap_2()
      .px_3()
      .py_1()
      .child(
        div()
          .flex_1()
          .text_xs()
          .font_weight(gpui::FontWeight::SEMIBOLD)
          .text_color(theme.muted_foreground)
          .child("GitHub inbox"),
      )
      .when(unread > 0, |this| {
        this.child(
          div()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(unread.to_string()),
        )
      });

    let rows: Vec<_> = notifications
      .into_iter()
      .enumerate()
      .map(|(ix, notification)| {
        let group_name = SharedString::from(format!("inbox-row-{}", notification.id));
        let done_id = notification.id.clone();
        let time = format_relative_time(&notification.updated_at);
        let repo = notification.repository.full_name.clone();
        let title = notification.subject.title.clone();
        let is_unread = notification.unread;

        div()
          .id(("session-page-inbox-row", ix))
          .group(group_name.clone())
          .mx_2()
          .px_2()
          .py_1p5()
          .rounded(px(6.0))
          .cursor_pointer()
          .hover(|s| s.bg(theme.secondary_hover))
          .on_click(cx.listener(move |_, _, _, cx| {
            github_notifications::open_notification(&notification, cx);
          }))
          .child(
            v_flex()
              .gap_0p5()
              .child(
                h_flex()
                  .items_center()
                  .gap_2()
                  .when(is_unread, |this| {
                    this.child(
                      div()
                        .flex_shrink_0()
                        .size(px(6.0))
                        .rounded_full()
                        .bg(theme.primary),
                    )
                  })
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
                    Button::new(("session-page-inbox-done", ix))
                      .icon(UiIconName::Check)
                      .xsmall()
                      .ghost()
                      .opacity(0.0)
                      .group_hover(group_name.clone(), |this| this.opacity(1.0))
                      .tooltip("Mark as done")
                      .on_click(cx.listener(move |_, _, _, cx| {
                        cx.stop_propagation();
                        github_notifications::mark_notification_done(done_id.clone(), cx);
                      })),
                  ),
              )
              .child(
                h_flex()
                  .items_center()
                  .gap_2()
                  .text_xs()
                  .text_color(theme.muted_foreground)
                  .child(div().flex_1().min_w(px(0.0)).truncate().child(repo))
                  .child(div().child(time)),
              ),
          )
      })
      .collect();

    let body = if rows.is_empty() {
      div()
        .px_3()
        .py_2()
        .text_xs()
        .text_color(theme.muted_foreground)
        .child("No notifications")
        .into_any_element()
    } else {
      div()
        .id("session-page-inbox-list")
        .max_h(px(INBOX_MAX_HEIGHT))
        .overflow_y_scroll()
        .pb_1()
        .children(rows)
        .into_any_element()
    };

    v_flex()
      .py_1()
      .border_t_1()
      .border_color(theme.border)
      .child(header)
      .child(body)
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
    let file_title = self
      .selected_file
      .as_deref()
      .map(|path| render_file_title(path, file_dirty, cx));

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
      let editor_pane = div()
        .flex_1()
        .min_h_0()
        .min_w(px(0.0))
        .flex()
        .flex_col()
        .child(editor.clone())
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
      .on_action(cx.listener(Self::show_command_palette_action))
      .on_action(cx.listener(Self::show_file_search_action))
      .on_action(cx.listener(Self::send_review_comments_to_agent_action))
      .on_action(cx.listener(Self::comment_hunk_action))
      .on_action(cx.listener(Self::toggle_diff_view_action))
      .on_action(cx.listener(Self::toggle_hide_whitespace_action))
      .on_action(cx.listener(Self::previous_annotation_action))
      .on_action(cx.listener(Self::next_annotation_action))
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
  #[gpui::test]
  async fn the_repo_line_is_painted_without_connecting_an_agent(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-repo-line");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.executor().allow_parking();
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
    cx.executor().allow_parking();
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
    cx.executor().allow_parking();
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
  async fn the_diff_view_toggle_flips_the_mode_and_persists_it(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-page-diff-toggle");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.executor().allow_parking();
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
      // The Git page reads the same preference.
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
    cx.executor().allow_parking();
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
    cx.executor().allow_parking();
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
    cx.executor().allow_parking();
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
    cx.executor().allow_parking();
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
    cx.executor().allow_parking();
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
    cx.executor().allow_parking();
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
    cx.executor().allow_parking();
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
    cx.executor().allow_parking();
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
    cx.executor().allow_parking();
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
    cx.executor().allow_parking();
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
    cx.executor().allow_parking();
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
    cx.executor().allow_parking();
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
    cx.executor().allow_parking();
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
      page.navigate_change(HunkNavigationDirection::Next, cx);
      assert_eq!(hunk_state(page, cx).active_index, 1);

      // Walking past the last change comes back to the first.
      page.navigate_change(HunkNavigationDirection::Next, cx);
      assert_eq!(hunk_state(page, cx).active_index, 0);

      page.navigate_change(HunkNavigationDirection::Previous, cx);
      assert_eq!(hunk_state(page, cx).active_index, 1);

      // A rendered file has no changes to walk.
      page.toggle_preview(cx);
      page.navigate_change(HunkNavigationDirection::Next, cx);
      assert_eq!(hunk_state(page, cx).active_index, 1);
    });
  }
}
