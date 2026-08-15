//! Page chrome: header, sidebar, editor area and the terminal split.

use super::*;

impl Render for GitPage {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let working_directory = self.selected_repo.clone();
    self.terminal_view.update(cx, |view, cx| {
      view.set_working_directory(working_directory, cx);
    });

    let content = if Self::should_render_repository_split(self.selected_repo.as_deref()) {
      ui::h_resizable("git-page-split")
        .child(
          ui::resizable_panel()
            .size(px(SIDEBAR_DEFAULT_WIDTH))
            .size_range(px(SIDEBAR_MIN_WIDTH)..px(SIDEBAR_MAX_WIDTH))
            .child(self.render_sidebar(window, cx)),
        )
        .child(ui::resizable_panel().child(self.render_main_content(window, cx)))
        .into_any_element()
    } else {
      self.render_repository_empty_state(window, cx)
    };

    div()
      .size_full()
      .flex()
      .flex_col()
      .bg(cx.theme().background)
      .track_focus(&self.focus_handle(cx))
      .on_action(cx.listener(GitPage::show_command_palette_action))
      .on_action(cx.listener(GitPage::show_branch_switcher_action))
      .on_action(cx.listener(GitPage::show_file_search_action))
      .on_action(cx.listener(GitPage::find_action))
      .on_action(cx.listener(GitPage::close_find_action))
      .on_action(cx.listener(GitPage::open_repository_action))
      .on_action(cx.listener(GitPage::toggle_terminal_sidebar_action))
      .on_action(cx.listener(GitPage::commit_changes_action))
      .on_action(cx.listener(GitPage::open_git_history_sidebar_action))
      .on_action(cx.listener(GitPage::open_git_changes_sidebar_action))
      .on_action(cx.listener(GitPage::pull_changes_action))
      .on_action(cx.listener(GitPage::push_changes_shortcut_action))
      .on_action(cx.listener(GitPage::force_push_changes_shortcut_action))
      .on_action(cx.listener(GitPage::toggle_diff_view_action))
      .on_action(cx.listener(GitPage::toggle_hide_whitespace_action))
      .on_action(cx.listener(GitPage::previous_annotation_action))
      .on_action(cx.listener(GitPage::next_annotation_action))
      .on_action(cx.listener(GitPage::comment_hunk_action))
      .on_action(cx.listener(GitPage::send_review_comments_to_agent_action))
      .on_action(cx.listener(GitPage::add_selection_to_agent_action))
      .on_action(cx.listener(GitPage::toggle_hunk_stage_action))
      .on_action(cx.listener(GitPage::restore_hunk_action))
      .on_action(cx.listener(GitPage::toggle_file_stage_action))
      .on_action(cx.listener(GitPage::restore_file_shortcut_action))
      .on_action(cx.listener(GitPage::accept_both_conflict_action))
      .child(self.render_header(window, cx))
      .child(div().flex_1().min_h_0().child(content))
  }
}

impl GitPage {
  pub(super) fn should_show_changed_files_tag(changed_files_count: usize) -> bool {
    changed_files_count > 0
  }

  pub(super) fn render_header(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let push_pull_loading = self.push_pull_in_progress;
    let on_repo_select = self.repo_select_handler(cx);
    let on_branch_select = self.branch_select_handler(cx);
    let mut repo_options = self.repo_dropdown_items.clone();
    repo_options.push(RecentRepoItem::open_action());
    let branch_options = self.branch_dropdown_items.clone();
    let branch_context = self.github_branch_context(cx);
    let branch_pr_button_state = self.current_branch_pr_button_state(cx);

    let repo_dropdown = dropdown_select(
      DropdownSelectConfig::new("git-header-repo-select")
        .trigger_label("Repository")
        .trigger_height(px(PAGE_HEADER_HEIGHT - 1.))
        .placeholder("Select repository...")
        .search_placeholder("Search repositories...")
        .options(repo_options)
        .width(px(TRIGGER_DROPDOWN_SELECT_WIDTH))
        .menu_width(px(TRIGGER_DROPDOWN_SELECT_WIDTH))
        .on_select(on_repo_select),
    );
    let repo_dropdown = div().child(repo_dropdown);

    let branch_dropdown = dropdown_select(
      DropdownSelectConfig::new("git-header-branch-select")
        .trigger_label("Branch")
        .trigger_height(px(PAGE_HEADER_HEIGHT - 1.))
        .placeholder("Select branch...")
        .search_placeholder("Search branches...")
        .options(branch_options)
        .width(px(TRIGGER_DROPDOWN_SELECT_WIDTH))
        .menu_width(px(TRIGGER_DROPDOWN_SELECT_WIDTH))
        .disabled(self.selected_repo.is_none())
        .on_select(on_branch_select),
    );
    let branch_dropdown = div().child(branch_dropdown);

    let branch_info = self.branch_status.as_ref().map(|status| {
      let ahead = status.ahead;
      let behind = status.behind;
      let ahead_color = if ahead > 0 {
        theme.status_green()
      } else {
        theme.muted_foreground
      };
      let behind_color = if behind > 0 {
        theme.status_red()
      } else {
        theme.muted_foreground
      };

      div()
        .flex()
        .items_center()
        .gap_2()
        .child(
          div()
            .flex()
            .items_center()
            .gap_2()
            .child(
              div()
                .id("branch-ahead-push")
                .flex()
                .items_center()
                .gap_1()
                .cursor_pointer()
                .tooltip(|window, cx| Tooltip::new("Push").build(window, cx))
                .on_click(cx.listener(|this, _event, _window, cx| {
                  this.push_changes_action(cx);
                }))
                .child(
                  Icon::new(IconName::ArrowUp)
                    .size_3()
                    .text_color(ahead_color),
                )
                .child(
                  div()
                    .text_xs()
                    .text_color(ahead_color)
                    .child(ahead.to_string()),
                ),
            )
            .child(
              div()
                .id("branch-behind-pull")
                .flex()
                .items_center()
                .gap_1()
                .cursor_pointer()
                .tooltip(|window, cx| Tooltip::new("Pull").build(window, cx))
                .on_click(cx.listener(|this, _event, _window, cx| {
                  if let Some(repo_root) = this.selected_repo.clone() {
                    this.pull_repository(repo_root, cx);
                  }
                }))
                .child(
                  Icon::new(IconName::ArrowDown)
                    .size_3()
                    .text_color(behind_color),
                )
                .child(
                  div()
                    .text_xs()
                    .text_color(behind_color)
                    .child(behind.to_string()),
                ),
            ),
        )
        .when(push_pull_loading, |this| {
          this.child(
            h_flex()
              .items_center()
              .gap_1()
              .child(Spinner::new().small())
              .child(
                div()
                  .text_xs()
                  .text_color(theme.muted_foreground)
                  .child("Syncing"),
              ),
          )
        })
    });

    let fetch_button = Button::new("git-fetch-button")
      .label("Fetch")
      .icon(UiIconName::RefreshCw)
      .outline()
      .loading_icon(Icon::new(UiIconName::RefreshCw))
      .loading(self.fetch_in_progress)
      .with_variant(ButtonVariant::Secondary)
      .xsmall()
      .p_2()
      .disabled(self.selected_repo.is_none() || self.fetch_in_progress)
      .tooltip("Fetch updates from remotes")
      .on_click(cx.listener(Self::fetch_action));

    let branch_pr_button = match branch_pr_button_state {
      GitBranchPullRequestButtonState::Hidden => None,
      GitBranchPullRequestButtonState::LockedPro => Some(
        Button::new("git-branch-pr-locked")
          .label("Create PR")
          .icon(UiIconName::GitPullRequestArrow)
          .outline()
          .with_variant(ButtonVariant::Secondary)
          .xsmall()
          .p_2()
          .on_click(|_, _, cx| {
            crate::analytics::track_with(
              cx,
              "pro_teaser_clicked",
              Some(serde_json::json!({ "source": "branch_pr_button" })),
            );
            NavigationHistory::navigate("/billing", cx);
          }),
      ),
      GitBranchPullRequestButtonState::Checking => Some(
        Button::new("git-branch-pr-status")
          .label("Checking PR")
          .icon(UiIconName::GitPullRequestArrow)
          .outline()
          .loading(true)
          .with_variant(ButtonVariant::Secondary)
          .xsmall()
          .p_2()
          .disabled(true)
          .tooltip("Looking for an open pull request for this branch"),
      ),
      GitBranchPullRequestButtonState::PublishAndCreate => {
        branch_context.clone().map(|_branch_context| {
          Button::new("git-publish-and-create-branch-pr")
            .label("Publish and Create PR")
            .icon(UiIconName::GitPullRequestArrow)
            .outline()
            .with_variant(ButtonVariant::Secondary)
            .xsmall()
            .p_2()
            .loading(self.publish_branch_and_create_pr_in_progress)
            .disabled(self.push_pull_in_progress || self.publish_branch_and_create_pr_in_progress)
            .on_click(cx.listener(|this, _, _window, cx| {
              this.publish_branch_and_create_pull_request_action(cx);
            }))
        })
      }
      GitBranchPullRequestButtonState::OpenExisting {
        owner,
        repo,
        number,
      } => {
        let pr_url = github_shared::pr_url(owner.as_str(), repo.as_str(), number);
        Some(
          Button::new("git-open-branch-pr")
            .label(format!("Open PR #{number}"))
            .icon(UiIconName::GitPullRequestArrow)
            .outline()
            .with_variant(ButtonVariant::Secondary)
            .xsmall()
            .p_2()
            .on_click(move |_, window, cx| {
              if should_open_externally(window) {
                cx.open_url(&pr_url);
              } else {
                GithubPrDetailsPageHandle::show_with_open_target(
                  owner.clone().into(),
                  repo.clone().into(),
                  number,
                  false,
                  None,
                  cx,
                );
              }
            }),
        )
      }
      GitBranchPullRequestButtonState::Create => branch_context.clone().map(|branch_context| {
        let git_page = cx.entity().downgrade();
        Button::new("git-create-branch-pr")
          .label("Create PR")
          .icon(UiIconName::GitPullRequestArrow)
          .outline()
          .with_variant(ButtonVariant::Secondary)
          .xsmall()
          .p_2()
          .on_click(move |_, window, cx| {
            open_create_pull_request_dialog(
              WorkspaceApi::global(cx).api.clone(),
              window.window_handle(),
              git_page_created_handler(git_page.clone()),
              branch_context.clone(),
              window,
              cx,
            );
          })
      }),
    };

    let header_left = div()
      .flex()
      .flex_1()
      .min_w_0()
      .h_full()
      .items_center()
      .gap_3()
      .child(
        div()
          .flex()
          .items_center()
          .child(
            div()
              .border_r_1()
              .border_color(theme.border)
              .child(repo_dropdown),
          )
          .child(
            div()
              .border_r_1()
              .border_color(theme.border)
              .child(branch_dropdown),
          ),
      )
      .when_some(branch_info, |this, info| this.child(info))
      .child(fetch_button);

    let terminal_sidebar_button = Button::new("git-toggle-terminal-sidebar")
      .label("Terminal")
      .icon(UiIconName::SquareTerminal)
      .outline()
      .with_variant(ButtonVariant::Secondary)
      .xsmall()
      .p_2()
      .selected(self.show_terminal_sidebar)
      .child(
        div().ml_1().text_color(theme.muted_foreground).child(
          Kbd::new(shortcuts::resolved_display_shortcut_keystroke_in(
            cx,
            window,
            ShortcutId::ToggleTerminalSidebar,
          ))
          .appearance(false),
        ),
      )
      .disabled(self.selected_repo.is_none())
      .on_click(cx.listener(Self::toggle_terminal_sidebar_click));

    let header_right = h_flex()
      .items_center()
      .gap_2()
      .flex_shrink_0()
      .when_some(branch_pr_button, |this, button| this.child(button))
      .child(
        div()
          .debug_selector(|| GIT_TERMINAL_BUTTON_DEBUG_SELECTOR.to_string())
          .child(terminal_sidebar_button),
      );

    div()
      .h(px(PAGE_HEADER_HEIGHT))
      .min_h(px(PAGE_HEADER_HEIGHT))
      .max_h(px(PAGE_HEADER_HEIGHT))
      .pr_3()
      .flex()
      .items_center()
      .justify_between()
      .bg(theme.sidebar)
      .border_b_1()
      .border_color(theme.title_bar_border)
      .child(header_left)
      .child(header_right)
  }

  pub(super) fn render_empty_state(&self, message: &str, cx: &mut Context<Self>) -> AnyElement {
    let message = message.to_string();
    let theme = cx.theme().clone();
    div()
      .size_full()
      .flex()
      .px_2()
      .bg(theme.background)
      .items_center()
      .justify_center()
      .text_color(cx.theme().muted_foreground)
      .child(div().truncate().child(message))
      .into_any_element()
  }

  pub(super) fn should_render_repository_split(selected_repo: Option<&Path>) -> bool {
    selected_repo.is_some()
  }

  pub(super) fn render_repository_empty_state(
    &mut self,
    window: &Window,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let theme = cx.theme().clone();
    div()
      .size_full()
      .flex()
      .bg(theme.background)
      .items_center()
      .justify_center()
      .child(
        div()
          .id("git-repository-empty-state")
          .flex()
          .flex_col()
          .items_center()
          .gap_3()
          .child(
            div()
              .text_base()
              .font_medium()
              .text_color(theme.foreground)
              .child(EMPTY_REPOSITORY_TITLE),
          )
          .child(
            h_flex()
              .items_center()
              .gap_2()
              .text_sm()
              .text_color(theme.muted_foreground)
              .child(EMPTY_REPOSITORY_HINT_PREFIX)
              .child(Kbd::new(shortcuts::resolved_display_shortcut_keystroke_in(
                cx,
                window,
                ShortcutId::OpenRepository,
              )))
              .child(EMPTY_REPOSITORY_HINT_SUFFIX),
          )
          .child(
            Button::new("git-empty-state-open-repository")
              .label("Open Repository")
              .icon(IconName::FolderOpen)
              .with_variant(ButtonVariant::Secondary)
              .on_click(cx.listener(move |this, _, window, cx| {
                this.start_open_repository(window, cx);
              })),
          ),
      )
      .into_any_element()
  }

  pub(super) fn render_loading_state(&self, message: &str, cx: &mut Context<Self>) -> AnyElement {
    let message = message.to_string();
    let theme = cx.theme().clone();
    div()
      .size_full()
      .flex()
      .bg(theme.background)
      .items_center()
      .justify_center()
      .child(
        div()
          .id("git-editor-loading-state")
          .flex()
          .flex_col()
          .items_center()
          .gap_2()
          .child(Spinner::new().small())
          .child(
            div()
              .text_sm()
              .text_color(theme.muted_foreground)
              .child(message),
          ),
      )
      .into_any_element()
  }

  pub(super) fn should_show_editor_loading_state(
    selected_file: Option<&Path>,
    has_editor: bool,
  ) -> bool {
    selected_file.is_some() && !has_editor
  }

  pub(super) fn should_show_open_action_loading_state(
    pending_open_action: Option<&GitPageOpenAction>,
    selected_file: Option<&Path>,
    has_editor: bool,
  ) -> bool {
    pending_open_action.is_some() && selected_file.is_none() && !has_editor
  }

  pub(super) fn render_editor_header(
    &self,
    editor: &Entity<Editor>,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let theme = cx.theme().clone();
    let editor_state = editor.read(cx);
    let is_history_commit_file = self.history_opened_commit_file.is_some();
    let selected_entry = self
      .selected_file
      .as_ref()
      .and_then(|path| self.status_entries.iter().find(|entry| &entry.path == path))
      .cloned();
    let display_path = selected_entry
      .as_ref()
      .map(|entry| entry.path.as_path())
      .or(self.selected_file.as_deref())
      .unwrap_or(editor_state.workdir_path.as_path());
    let file_name = format_git_file_name_label(display_path);
    let old_file_name = selected_entry
      .as_ref()
      .and_then(|entry| entry.old_path.as_ref())
      .map(|path| format_git_file_name_label(path));
    let dir_path = display_path
      .parent()
      .and_then(|parent| parent.to_str())
      .unwrap_or("")
      .to_string();
    let file_dirty = editor_state.is_dirty;
    let editor_entity = editor.clone();
    let status_kind = selected_entry.as_ref().map(|entry| entry.status);
    let status_letter = status_kind.map(|status| status.short_code());
    let status_color = status_kind
      .map(|status| Self::status_color(status, &theme))
      .unwrap_or(theme.muted_foreground);

    let title = h_flex()
      .items_center()
      .gap_2()
      .min_w_0()
      .flex_1()
      .when_some(status_letter, |this, letter| {
        this.child(
          div()
            .w(px(15.))
            .text_xs()
            .text_color(status_color)
            .child(letter),
        )
      })
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
              .child(render_repo_status_label(
                &theme,
                status_kind,
                file_name,
                old_file_name,
              ))
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
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(format!("- {}", dir_path)),
            )
          }),
      );

    let show_save_button = !is_history_commit_file && !editor_state.is_read_only;
    let save_button = Button::new("editor-save")
      .label("Save")
      .xsmall()
      .ghost()
      .disabled(!file_dirty)
      .on_click(move |_, _, cx| {
        editor_entity.update(cx, |editor, cx| editor.save(cx));
      });

    let is_markdown = self.selected_file_is_markdown();
    let is_svg = self.selected_file_is_svg();
    let preview_active = (is_markdown || is_svg) && self.show_markdown_preview;
    let split_disabled = self
      .selected_file
      .as_ref()
      .map(|path| self.split_disabled_for_path(path))
      .unwrap_or(false)
      || preview_active;
    let (toggle_label, toggle_icon) = if split_disabled {
      ("Split", IconName::PanelLeft)
    } else {
      match self.diff_view {
        DiffViewMode::Inline => ("Split", IconName::PanelLeft),
        DiffViewMode::Split => ("Inline", IconName::PanelLeftClose),
      }
    };
    let view = cx.entity();
    let toggle_button = Button::new("editor-diff-toggle")
      .label(toggle_label)
      .icon(toggle_icon)
      .xsmall()
      .ghost()
      .disabled(split_disabled)
      .on_click(move |_, _, cx| {
        view.update(cx, |this, cx| {
          this.toggle_diff_view(cx);
        });
      });

    let view = cx.entity();
    let hide_whitespace = self.hide_whitespace;
    let show_whitespace_button = self.binary_preview.is_none();

    let whitespace_icon = if hide_whitespace {
      IconName::Eye
    } else {
      IconName::EyeOff
    };
    let tooltip = if hide_whitespace {
      "Show whitespace changes"
    } else {
      "Hide whitespace changes"
    };
    let whitespace_button = div()
      .debug_selector(|| "editor-whitespace-toggle".to_string())
      .child(
        Button::new("editor-whitespace-toggle")
          .label("Whitespace")
          .icon(whitespace_icon)
          .tooltip(tooltip)
          .xsmall()
          .ghost()
          .on_click(move |_, _, cx| {
            view.update(cx, |this, cx| {
              this.toggle_hide_whitespace(cx);
            });
          }),
      );

    let view = cx.entity();
    let preview_button = Button::new("editor-markdown-preview")
      .label("Preview")
      .icon(if preview_active {
        IconName::EyeOff
      } else {
        IconName::Eye
      })
      .xsmall()
      .ghost()
      .selected(preview_active)
      .on_click(move |_, _, cx| {
        view.update(cx, |this, cx| {
          this.toggle_markdown_preview(cx);
        });
      });

    let agent_review_count = self.agent_review.copyable_count();
    let view = cx.entity();
    let send_agent_review_button = Button::new("editor-send-agent-review")
      .label(format!("Send to Agent ({agent_review_count})"))
      .icon(UiIconName::Sparkles)
      .xsmall()
      .ghost()
      .tooltip("Send local review comments to the agent")
      .on_click({
        let view = view.clone();
        move |_, window, cx| {
          view.update(cx, |this, cx| {
            this.send_agent_review_to_agent(window, cx);
          });
        }
      });

    let file_status = if is_history_commit_file {
      None
    } else {
      selected_entry.as_ref().map(|entry| entry.status)
    };
    let annotation_navigation =
      Self::annotation_navigation_state_for(file_status, editor_state, cx);
    let can_navigate_annotations = Self::can_navigate_annotations(annotation_navigation);
    let show_accept_all_conflict_actions = matches!(file_status, Some(RepoStatusKind::Conflicted));
    let can_accept_all_conflicts = Self::can_accept_all_conflicts(
      file_status,
      editor_state.is_read_only,
      editor_state.has_unresolved_conflict_markers(cx),
    );

    let editor_entity_accept_current = editor.clone();
    let accept_all_current_button = Button::new("editor-accept-all-current")
      .label("Accept All Current")
      .xsmall()
      .ghost()
      .disabled(!can_accept_all_conflicts)
      .on_click(move |_, _, cx| {
        editor_entity_accept_current.update(cx, |editor, cx| {
          editor.resolve_all_conflicts(ConflictResolution::Current, cx);
        });
      });

    let editor_entity_accept_incoming = editor.clone();
    let accept_all_incoming_button = Button::new("editor-accept-all-incoming")
      .label("Accept All Incoming")
      .xsmall()
      .ghost()
      .disabled(!can_accept_all_conflicts)
      .on_click(move |_, _, cx| {
        editor_entity_accept_incoming.update(cx, |editor, cx| {
          editor.resolve_all_conflicts(ConflictResolution::Incoming, cx);
        });
      });

    let annotation_kind = annotation_navigation.map(|state| state.kind);
    let (previous_tooltip, next_tooltip) = match annotation_kind {
      Some(AnnotationKind::Conflict) => ("Previous conflict", "Next conflict"),
      _ => ("Previous change", "Next change"),
    };

    let view = cx.entity();
    let previous_annotation_button = Button::new("editor-annotation-prev")
      .icon(IconName::ArrowUp)
      .xsmall()
      .ghost()
      .compact()
      .tooltip(previous_tooltip)
      .disabled(!can_navigate_annotations)
      .on_click(move |_, _, cx| {
        view.update(cx, |this, cx| {
          this.navigate_annotation_in_editor(AnnotationDirection::Previous, cx);
        });
      });

    let view = cx.entity();
    let next_annotation_button = Button::new("editor-annotation-next")
      .icon(IconName::ArrowDown)
      .xsmall()
      .ghost()
      .compact()
      .tooltip(next_tooltip)
      .disabled(!can_navigate_annotations)
      .on_click(move |_, _, cx| {
        view.update(cx, |this, cx| {
          this.navigate_annotation_in_editor(AnnotationDirection::Next, cx);
        });
      });

    div()
      .min_h(px(EDITOR_HEADER_HEIGHT))
      .h(px(EDITOR_HEADER_HEIGHT))
      .px_3()
      .flex()
      .items_center()
      .justify_between()
      .gap_2()
      .bg(theme.sidebar)
      .border_b_1()
      .border_color(theme.title_bar_border)
      .child(title)
      .child(
        div()
          .flex()
          .items_center()
          .gap_2()
          .flex_shrink_0()
          .when_some(
            annotation_navigation.filter(|state| state.total > 1),
            |this, annotation_navigation| {
              this.child(
                h_flex()
                  .items_center()
                  .gap_1()
                  .child(previous_annotation_button)
                  .child(
                    div()
                      .w(px(52.0))
                      .text_xs()
                      .text_center()
                      .text_color(theme.muted_foreground)
                      .child(format!(
                        "{}/{}",
                        annotation_navigation.active_index + 1,
                        annotation_navigation.total
                      )),
                  )
                  .child(next_annotation_button),
              )
            },
          )
          .when(show_accept_all_conflict_actions, |this| {
            this
              .child(accept_all_current_button)
              .child(accept_all_incoming_button)
          })
          .when(show_save_button, |this| this.child(save_button))
          .when(is_markdown || is_svg, |this| this.child(preview_button))
          .when(show_whitespace_button, |this| this.child(whitespace_button))
          .when(!is_history_commit_file && agent_review_count > 0, |this| {
            this.child(send_agent_review_button)
          })
          .child(toggle_button),
      )
      .into_any_element()
  }

  pub(super) fn render_interactive_rebase_todo_header(&self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    h_flex()
      .min_h(px(EDITOR_HEADER_HEIGHT))
      .h(px(EDITOR_HEADER_HEIGHT))
      .px_3()
      .items_center()
      .justify_between()
      .gap_2()
      .bg(theme.sidebar)
      .border_b_1()
      .border_color(theme.title_bar_border)
      .child(
        h_flex()
          .items_center()
          .gap_2()
          .child(Icon::new(UiIconName::GitMerge).size_3())
          .child("Interactive rebase"),
      )
      .into_any_element()
  }

  pub(super) fn render_editor_with_overlay(
    &mut self,
    editor: Entity<Editor>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let overlay = self.render_change_block_actions(&editor, window, cx);
    let mut wrapper = div()
      .flex_1()
      .min_w(px(0.0))
      .min_h(px(0.0))
      .relative()
      .overflow_hidden()
      .child(editor);

    if let Some(overlay) = overlay {
      wrapper = wrapper.child(overlay);
    }

    wrapper.into_any_element()
  }

  pub(super) fn render_change_block_actions(
    &mut self,
    editor: &Entity<Editor>,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Option<AnyElement> {
    let theme = cx.theme().clone();
    let editor_state = editor.read(cx);
    if self.history_opened_commit_file.is_some() || editor_state.is_read_only {
      return None;
    }
    let selected_status = self
      .selected_file
      .as_ref()
      .and_then(|selected| {
        self
          .status_entries
          .iter()
          .find(|entry| &entry.path == selected)
      })
      .map(|entry| entry.status);

    if matches!(selected_status, Some(RepoStatusKind::Conflicted)) {
      let conflict_start_line = editor_state.hovered_conflict_start_line?;
      let anchor_display_line = editor_state
        .first_display_line_for_conflict(conflict_start_line)
        .unwrap_or(conflict_start_line);
      if editor_state.find_panel_occludes_display_line(anchor_display_line) {
        return None;
      }
      let mut top = Self::hunk_action_top(
        editor_state.measured_editor_line_height(),
        anchor_display_line,
        editor_state.scroll_offset_y,
      );
      if top >= editor_state.viewport_height {
        return None;
      }
      if top < px(0.0) {
        top = px(0.0);
      }

      let editor_entity = editor.clone();
      let mut actions = div().flex().items_center();

      let editor_entity_current = editor_entity.clone();
      actions = actions.child(
        Button::new("accept-current-conflict")
          .label("Accept Current")
          .small()
          .bg(theme.background)
          .rounded_t_none()
          .rounded_br_none()
          .on_click(move |_, _, cx| {
            editor_entity_current.update(cx, |editor, cx| {
              editor.resolve_conflict_region(conflict_start_line, ConflictResolution::Current, cx);
            });
          }),
      );

      let editor_entity_incoming = editor_entity.clone();
      actions = actions.child(
        Button::new("accept-incoming-conflict")
          .label("Accept Incoming")
          .small()
          .bg(theme.background)
          .rounded_none()
          .on_click(move |_, _, cx| {
            editor_entity_incoming.update(cx, |editor, cx| {
              editor.resolve_conflict_region(conflict_start_line, ConflictResolution::Incoming, cx);
            });
          }),
      );

      actions = actions.child(
        Button::new("accept-both-conflict")
          .label("Accept Both")
          .small()
          .bg(theme.background)
          .rounded_t_none()
          .rounded_bl_none()
          .on_click(move |_, _, cx| {
            editor_entity.update(cx, |editor, cx| {
              editor.resolve_conflict_region(conflict_start_line, ConflictResolution::Both, cx);
            });
          }),
      );

      return Some(
        div()
          .absolute()
          .top(top)
          .right(px(30.0))
          .child(actions)
          .into_any_element(),
      );
    }

    let hovered_id = editor_state.hovered_group_id.as_ref()?;
    let overlay = editor_state
      .visible_groups
      .iter()
      .find(|overlay| overlay.id.as_ref() == hovered_id.as_ref())?;

    let anchor_display_line = editor_state
      .first_display_line_for_group(hovered_id)
      .unwrap_or(overlay.display_line);
    if editor_state.find_panel_occludes_display_line(anchor_display_line) {
      return None;
    }
    let mut top = Self::hunk_action_top(
      editor_state.measured_editor_line_height(),
      anchor_display_line,
      editor_state.scroll_offset_y,
    );
    if top >= editor_state.viewport_height {
      return None;
    }
    if top < px(0.0) {
      top = px(0.0);
    }
    let file_dirty = editor_state.is_dirty;

    if matches!(
      selected_status,
      Some(RepoStatusKind::Untracked | RepoStatusKind::Added)
    ) {
      return None;
    }

    let restore_disabled_by_status = matches!(
      selected_status,
      Some(RepoStatusKind::Untracked | RepoStatusKind::Added)
    );
    let restore_disabled = file_dirty || restore_disabled_by_status;

    let stage_tooltip = if file_dirty {
      "File not saved"
    } else {
      "Stage hunk"
    };
    let unstage_tooltip = if file_dirty {
      "File not saved"
    } else {
      "Unstage hunk"
    };
    let restore_tooltip = if file_dirty {
      "File not saved"
    } else if restore_disabled_by_status {
      "Restore unavailable for added/untracked files"
    } else {
      "Restore hunk"
    };

    let group_id = overlay.id.clone();
    let state = overlay.state;
    let editor_entity = editor.clone();

    let mut actions = div().flex().items_center();

    match state {
      HunkState::Unstaged => {
        let editor_entity = editor_entity.clone();
        let group_id = group_id.clone();
        actions = actions.child(
          Button::new("stage-hunk")
            .icon(IconName::Plus)
            .label("Stage")
            .small()
            .tooltip(stage_tooltip)
            .rounded_t_none()
            .rounded_br_none()
            .bg(theme.background)
            .disabled(file_dirty)
            .on_click(move |_, _, cx| {
              let group_id = group_id.clone();
              editor_entity.update(cx, |editor, cx| {
                editor.enqueue_group_action(group_id, HunkAction::Stage, cx);
              });
            }),
        );
      }
      HunkState::Staged => {
        let editor_entity = editor_entity.clone();
        let group_id = group_id.clone();
        actions = actions.child(
          Button::new("unstage-hunk")
            .icon(IconName::Minus)
            .label("Unstage")
            .tooltip(unstage_tooltip)
            .small()
            .disabled(file_dirty)
            .bg(theme.background)
            .rounded_t_none()
            .on_click(move |_, _, cx| {
              let group_id = group_id.clone();
              editor_entity.update(cx, |editor, cx| {
                editor.enqueue_group_action(group_id, HunkAction::Unstage, cx);
              });
            }),
        );
      }
    }

    if matches!(state, HunkState::Unstaged) {
      let editor_entity = editor_entity.clone();
      let group_id = group_id.clone();
      actions = actions.child(
        Button::new("restore-hunk")
          .icon(IconName::Undo)
          .label("Restore")
          .rounded_t_none()
          .rounded_bl_none()
          .small()
          .bg(theme.background)
          .tooltip(restore_tooltip)
          .disabled(restore_disabled)
          .on_click(move |_, _, cx| {
            let group_id = group_id.clone();
            editor_entity.update(cx, |editor, cx| {
              editor.enqueue_group_action(group_id, HunkAction::Restore, cx);
            });
          }),
      );
    }

    Some(
      div()
        .absolute()
        .top(top)
        .right(px(30.0))
        .child(actions)
        .into_any_element(),
    )
  }

  pub(super) fn hunk_action_top(
    line_height: Pixels,
    display_line: usize,
    scroll_offset: f32,
  ) -> Pixels {
    line_height * (display_line as f32 - scroll_offset)
  }

  pub(super) fn render_commit_button(
    &mut self,
    window: &Window,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let repo_ready = self.selected_repo.is_some();
    let commit_message = self.commit_input.read(cx).value().to_string();
    let commit_enabled = self.commit_primary_action_enabled(&commit_message);
    let can_publish_branch =
      Self::should_publish_branch(self.branch_status.as_ref(), self.has_head_commit);
    let commit_primary_button_state = Self::commit_primary_button_state(
      self.rebase_in_progress,
      !self.status_entries.is_empty(),
      can_publish_branch,
    );
    let amend_enabled = repo_ready && self.has_head_commit;
    let undo_enabled = repo_ready && self.can_undo_last_commit;
    let push_enabled = repo_ready && self.can_push;
    let force_push_enabled = repo_ready && self.can_force_push;
    let menu_enabled = !self.rebase_in_progress
      && (amend_enabled || undo_enabled || push_enabled || force_push_enabled);
    let view = cx.entity();
    let amend_view = view.clone();
    let undo_view = view.clone();
    let push_view = view.clone();
    let force_push_view = view.clone();
    let push_label = Self::push_action_label(self.branch_status.as_ref(), self.has_head_commit);
    let commit_shortcut =
      shortcuts::resolved_display_shortcut_keystroke_in(cx, window, ShortcutId::CommitChanges);

    let main_button = match commit_primary_button_state {
      GitCommitPrimaryButtonState::ContinueRebase => Button::new("commit-button-main")
        .label("Continue")
        .with_variant(ButtonVariant::Secondary)
        .outline()
        .flex_1()
        .rounded_r_none()
        .child(Kbd::new(commit_shortcut.clone()).ml_1())
        .disabled(!commit_enabled)
        .on_click(cx.listener(Self::continue_rebase_action)),
      GitCommitPrimaryButtonState::PublishBranch => Button::new("commit-button-main")
        .label("Publish branch")
        .with_variant(ButtonVariant::Secondary)
        .outline()
        .flex_1()
        .rounded_r_none()
        .loading(self.push_pull_in_progress)
        .disabled(!push_enabled || self.push_pull_in_progress)
        .on_click(cx.listener(|this, _, _, cx| {
          this.push_changes_action(cx);
        })),
      GitCommitPrimaryButtonState::Commit => Button::new("commit-button-main")
        .label("Commit")
        .with_variant(ButtonVariant::Secondary)
        .outline()
        .flex_1()
        .rounded_r_none()
        .child(Kbd::new(commit_shortcut).ml_1())
        .disabled(!commit_enabled)
        .on_click(cx.listener(Self::commit_changes)),
    };

    let menu_button = Button::new("commit-button-menu")
      .icon(IconName::ChevronDown)
      .with_variant(ButtonVariant::Secondary)
      .outline()
      .rounded_l_none()
      .border_l_0()
      .disabled(!menu_enabled)
      .dropdown_menu_with_anchor(Anchor::BottomRight, move |menu, _, _| {
        let amend_view = amend_view.clone();
        let undo_view = undo_view.clone();
        let push_view = push_view.clone();
        let force_push_view = force_push_view.clone();
        let menu = menu.item(
          PopupMenuItem::new("Amend")
            .icon(IconName::Replace)
            .disabled(!amend_enabled)
            .on_click(move |event, window, cx| {
              amend_view.update(cx, |this, cx| {
                let _ = event;
                this.commit_amend_changes(window, cx);
                this.focus_page(window, cx);
              });
            }),
        );

        let menu = menu.item(
          PopupMenuItem::new("Undo last commit")
            .icon(IconName::Undo)
            .disabled(!undo_enabled)
            .on_click(move |event, window, cx| {
              undo_view.update(cx, |this, cx| {
                let _ = event;
                this.undo_last_commit_action(cx);
                this.focus_page(window, cx);
              });
            }),
        );

        let menu = menu.separator();

        let menu = menu.item(
          PopupMenuItem::new(push_label)
            .icon(IconName::ArrowUp)
            .disabled(!push_enabled)
            .on_click(move |event, window, cx| {
              push_view.update(cx, |this, cx| {
                let _ = event;
                this.push_changes_action(cx);
                this.focus_page(window, cx);
              });
            }),
        );

        menu.item(
          PopupMenuItem::new("Force push (with lease)")
            .icon(IconName::ArrowUp)
            .disabled(!force_push_enabled)
            .on_click(move |event, window, cx| {
              force_push_view.update(cx, |this, cx| {
                let _ = event;
                this.force_push_changes_action(cx);
                this.focus_page(window, cx);
              });
            }),
        )
      });

    div()
      .flex()
      .w_full()
      .overflow_hidden()
      .child(main_button)
      .child(menu_button)
  }

  pub(super) fn render_commit_bar(
    &mut self,
    window: &Window,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let theme = cx.theme().clone();
    let input = self.commit_input.clone();
    let has_conflicts = Self::has_conflicted_entries(&self.status_entries);
    let detached_head = Self::is_detached_head(self.branch_status.as_ref());
    let operation_error = self.operation_error.clone();

    div()
      .w_full()
      .min_w_0()
      .flex()
      .flex_col()
      .p_2()
      .gap_2()
      .border_t_1()
      .border_color(theme.border)
      .when(detached_head, |this| {
        this.child(
          StatusAlert::new(
            "commit-detached-head-info",
            theme.status_blue(),
            "You are in detached HEAD mode. Commits are not on a branch.",
          )
          .icon(IconName::Info)
          .title("Detached HEAD"),
        )
      })
      .when(has_conflicts, |this| {
        this.child(
          StatusAlert::new(
            "commit-conflicts-warning",
            theme.status_yellow(),
            "Resolve all conflicts before committing.",
          )
          .title("Conflicts detected"),
        )
      })
      .when_some(operation_error, |this, error| {
        this.child(
          StatusAlert::new("commit-operation-error", theme.status_red(), error.clone())
            .icon(IconName::CircleX)
            .title("Operation failed"),
        )
      })
      .child(div().w_full().min_w_0().key_context("CommitInput").child({
        let commit_box = Textarea::new(&input).w_full();
        commit_box.into_any_element()
      }))
      .child(
        div()
          .w_full()
          .min_w_0()
          .child(self.render_commit_button(window, cx)),
      )
  }

  pub(super) fn render_sidebar_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme();
    let all_staged = self.all_changes_staged();
    let sidebar_enabled = self.selected_repo.is_some() && !self.status_entries.is_empty();
    let restore_all_enabled = sidebar_enabled;
    let merge_abort_enabled = self.selected_repo.is_some() && self.merge_in_progress;
    let rebase_abort_enabled = self.selected_repo.is_some() && self.rebase_in_progress;
    let changed_files_count = Self::changed_files_count(&self.status_entries);
    let (icon, tooltip) = if all_staged {
      (IconName::Minus, "Unstage all files")
    } else {
      (IconName::Plus, "Stage all files")
    };
    let is_history_mode = self.sidebar_mode == GitSidebarMode::History;
    let (mode_label, mode_icon, mode_tooltip) = if is_history_mode {
      ("Changes", UiIconName::FileCode, "Show changes list")
    } else {
      ("History", UiIconName::History, "Show commit history")
    };

    let group_label = if is_history_mode {
      div()
        .text_sm()
        .text_color(theme.sidebar_foreground)
        .child("History")
        .into_any_element()
    } else {
      h_flex()
        .items_center()
        .gap_2()
        .child(
          div()
            .text_sm()
            .text_color(theme.sidebar_foreground)
            .child("Changes"),
        )
        .when(
          Self::should_show_changed_files_tag(changed_files_count),
          |this| {
            this.child(
              Tag::secondary()
                .small()
                .rounded_full()
                .child(changed_files_count.to_string()),
            )
          },
        )
        .into_any_element()
    };

    div()
      .w_full()
      .flex()
      .px_3()
      .min_h(px(EDITOR_HEADER_HEIGHT))
      .border_b_1()
      .border_color(cx.theme().border)
      .items_center()
      .justify_between()
      .child(group_label)
      .child(
        h_flex()
          .items_center()
          .gap_2()
          .when(self.merge_in_progress, |this| {
            this.child(
              Button::new("abort-merge-button")
                .label("Abort merge")
                .icon(IconName::Undo)
                .xsmall()
                .disabled(!merge_abort_enabled)
                .tooltip("Abort current merge")
                .on_click(cx.listener(Self::abort_merge_action)),
            )
          })
          .when(self.rebase_in_progress, |this| {
            this.child(
              Button::new("abort-rebase-button")
                .label("Abort rebase")
                .icon(IconName::Undo)
                .xsmall()
                .disabled(!rebase_abort_enabled)
                .tooltip("Abort current rebase")
                .on_click(cx.listener(Self::abort_rebase_action)),
            )
          })
          .when(!is_history_mode, |this| {
            this.child(
              ButtonGroup::new("button-group")
                .outline()
                .child(
                  Button::new("stage-all-button")
                    .icon(icon)
                    .with_variant(ButtonVariant::Secondary)
                    .xsmall()
                    .disabled(!sidebar_enabled)
                    .tooltip(tooltip)
                    .on_click(cx.listener(Self::toggle_stage_all_action)),
                )
                .child(
                  Button::new("restore-all-button")
                    .icon(IconName::Undo)
                    .with_variant(ButtonVariant::Secondary)
                    .xsmall()
                    .disabled(!restore_all_enabled)
                    .tooltip("Discard all changes")
                    .on_click(cx.listener(Self::restore_all_click_action)),
                ),
            )
          })
          .child(
            Button::new("sidebar-mode-toggle-button")
              .label(mode_label)
              .outline()
              .icon(mode_icon)
              .with_variant(ButtonVariant::Secondary)
              .xsmall()
              .selected(is_history_mode)
              .disabled(self.selected_repo.is_none())
              .tooltip(mode_tooltip)
              .on_click(cx.listener(Self::toggle_sidebar_mode_action)),
          ),
      )
  }

  pub(super) fn render_sidebar(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let theme = cx.theme().clone();
    let base_sidebar = div()
      .id("git-sidebar")
      .w_full()
      .h_full()
      .flex()
      .flex_col()
      .bg(theme.sidebar)
      .text_color(theme.sidebar_foreground);

    if self.selected_repo.is_none() {
      return base_sidebar
        .child(
          div()
            .p_4()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child("Select a repository"),
        )
        .into_any_element();
    }

    if self.sidebar_mode == GitSidebarMode::History {
      return base_sidebar
        .relative()
        .child(self.render_sidebar_header(cx))
        .child(self.render_history_sidebar_content(window, cx))
        .into_any_element();
    }

    let file_list_focused = self.file_list.read(cx).focus_handle(cx).is_focused(window);
    let list_container = div()
      .id("git-sidebar-file-list-container")
      .relative()
      .flex_1()
      .min_h_0()
      .overflow_hidden()
      .child(
        List::new(&self.file_list)
          .flex_1()
          .w_full()
          .min_h_0()
          .p(px(6.)),
      )
      .when(file_list_focused, |this| {
        this.child(
          div()
            .absolute()
            .top_0()
            .right_0()
            .bottom_0()
            .left_0()
            .border_2()
            .border_color(cx.theme().ring.alpha(0.1)),
        )
      });

    base_sidebar
      .relative()
      .child(self.render_sidebar_header(cx))
      .child(
        div()
          .flex()
          .flex_col()
          .flex_1()
          .min_h_0()
          .child(list_container),
      )
      .child(self.render_commit_bar(window, cx))
      .into_any_element()
  }

  pub(super) fn render_editor_area(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    if self.selected_repo.is_none() {
      return self.render_repository_empty_state(window, cx);
    }

    if let Some(todo_view) = self.interactive_rebase_todo_view.clone() {
      return div()
        .size_full()
        .flex()
        .flex_col()
        .child(self.render_interactive_rebase_todo_header(cx))
        .child(todo_view)
        .into_any_element();
    }

    let theme = cx.theme().clone();
    if let Some(editor) = self.editor.clone() {
      let editor_view = self.render_editor_with_overlay(editor.clone(), window, cx);
      if let Some(binary_preview) = self.binary_preview.as_ref() {
        return div()
          .size_full()
          .overflow_hidden()
          .flex()
          .flex_col()
          .child(self.render_editor_header(&editor, cx))
          .child(render_binary_preview(binary_preview, cx))
          .into_any_element();
      }

      if self.show_markdown_preview
        && (self.selected_file_is_markdown() || self.selected_file_is_svg())
      {
        let preview_content = if self.selected_file_is_svg() {
          let editor = editor.clone();
          self.svg_preview.update(cx, |preview, cx| {
            preview.refresh_from_editor(&editor, window, cx);
          });
          self.svg_preview.read(cx).render(cx)
        } else {
          let markdown = editor.read(cx).document().read(cx);
          let markdown = markdown.slice_to_string(0..markdown.len());
          div()
            .flex_1()
            .min_h_0()
            .min_w(px(0.0))
            .bg(theme.background)
            .occlude()
            .child(
              div().size_full().pb_4().px_4().child(
                TextView::markdown("git-markdown-preview-text", markdown)
                  .size_full()
                  .selectable(true)
                  .scrollable(true),
              ),
            )
            .into_any_element()
        };

        return div()
          .size_full()
          .flex()
          .flex_col()
          .child(self.render_editor_header(&editor, cx))
          .child(
            div().flex_1().min_h_0().child(
              ui::h_resizable("git-page-markdown-preview")
                .child(
                  ui::resizable_panel().child(
                    div()
                      .size_full()
                      .min_w(px(0.0))
                      .min_h_0()
                      .flex()
                      .flex_col()
                      .debug_selector(|| GIT_MARKDOWN_PREVIEW_EDITOR_DEBUG_SELECTOR.to_string())
                      .child(editor_view),
                  ),
                )
                .child(
                  ui::resizable_panel().child(
                    div()
                      .size_full()
                      .min_w(px(0.0))
                      .min_h_0()
                      .flex()
                      .flex_col()
                      .debug_selector(|| GIT_MARKDOWN_PREVIEW_RENDER_DEBUG_SELECTOR.to_string())
                      .child(preview_content),
                  ),
                ),
            ),
          )
          .into_any_element();
      }

      return div()
        .size_full()
        .flex()
        .flex_col()
        .child(self.render_editor_header(&editor, cx))
        .child(editor_view)
        .into_any_element();
    }

    if Self::should_show_editor_loading_state(self.selected_file.as_deref(), self.editor.is_some())
    {
      return self.render_loading_state("Loading file...", cx);
    }

    if Self::should_show_open_action_loading_state(
      self.pending_open_action.as_ref(),
      self.selected_file.as_deref(),
      self.editor.is_some(),
    ) && let Some(action) = self.pending_open_action.as_ref()
    {
      return self.render_loading_state(Self::open_action_loading_message(action), cx);
    }

    self.render_empty_state("Select a file to view diff", cx)
  }

  pub(super) fn render_terminal_sidebar(&mut self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    div()
      .size_full()
      .p_2()
      .min_w(px(0.0))
      .min_h_0()
      .bg(theme.sidebar)
      .debug_selector(|| GIT_TERMINAL_SIDEBAR_DEBUG_SELECTOR.to_string())
      .child(self.terminal_view.clone())
      .into_any_element()
  }

  pub(super) fn render_main_content(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let editor_area = self.render_editor_area(window, cx);
    if !self.show_terminal_sidebar {
      return editor_area;
    }

    ui::h_resizable("git-page-editor-terminal-split")
      .child(ui::resizable_panel().child(editor_area))
      .child(
        ui::resizable_panel()
          .size(px(TERMINAL_SIDEBAR_DEFAULT_WIDTH))
          .size_range(px(TERMINAL_SIDEBAR_MIN_WIDTH)..px(TERMINAL_SIDEBAR_MAX_WIDTH))
          .child(self.render_terminal_sidebar(cx)),
      )
      .into_any_element()
  }
}

#[cfg(test)]
mod tests {
  use super::super::test_support::*;
  use super::*;

  use crate::api::UserRole;
  use gpui::TestAppContext;

  #[test]
  fn changed_files_tag_visibility_requires_positive_count() {
    assert!(!GitPage::should_show_changed_files_tag(0));
    assert!(GitPage::should_show_changed_files_tag(1));
    assert!(GitPage::should_show_changed_files_tag(42));
  }

  #[gpui::test]
  async fn raster_image_preview_renders_without_source_editor_pane(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-open-image-preview");
    let rel_path = Path::new("fixtures/image.png");
    let absolute_path = repo.path.join(rel_path);
    std::fs::create_dir_all(
      absolute_path
        .parent()
        .expect("image preview path should have parent"),
    )
    .expect("create image preview parent");
    std::fs::write(&absolute_path, tiny_png_bytes()).expect("write png image");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.open_file(rel_path.to_path_buf(), cx);
    });

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let is_raster_preview = git_page.read_with(cx, |this, _cx| {
      matches!(this.binary_preview, Some(BinaryPreview::RasterImage(_)))
    });
    let preview_bounds = cx
      .debug_bounds(crate::file_view::BINARY_PREVIEW_DEBUG_SELECTOR)
      .expect("binary preview pane bounds")
      .size;

    assert!(is_raster_preview);
    assert!(preview_bounds.width > gpui::px(0.0));
    assert!(preview_bounds.height > gpui::px(0.0));
    assert!(cx.debug_bounds("editor-whitespace-toggle").is_none());
    assert!(
      cx.debug_bounds(GIT_MARKDOWN_PREVIEW_EDITOR_DEBUG_SELECTOR)
        .is_none()
    );
    assert!(
      cx.debug_bounds(GIT_MARKDOWN_PREVIEW_RENDER_DEBUG_SELECTOR)
        .is_none()
    );
  }

  #[gpui::test]
  async fn unsupported_binary_preview_renders_placeholder(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempRepo::init("git-page-open-binary-placeholder");
    let rel_path = Path::new("fixtures/slides.pdf");
    let absolute_path = repo.path.join(rel_path);
    std::fs::create_dir_all(
      absolute_path
        .parent()
        .expect("binary placeholder path should have parent"),
    )
    .expect("create binary placeholder parent");
    std::fs::write(&absolute_path, b"%PDF-1.7\n").expect("write pdf file");

    let (git_page, cx) = add_git_page_window_with_root(cx);
    cx.executor().allow_parking();

    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.open_file(rel_path.to_path_buf(), cx);
    });

    await_git_page_background_tasks(git_page.clone(), cx).await;

    let is_placeholder = git_page.read_with(cx, |this, _cx| {
      matches!(this.binary_preview, Some(BinaryPreview::UnsupportedBinary))
    });
    let preview_bounds = cx
      .debug_bounds(crate::file_view::BINARY_PREVIEW_DEBUG_SELECTOR)
      .expect("binary placeholder pane bounds")
      .size;

    assert!(is_placeholder);
    assert!(preview_bounds.width > gpui::px(0.0));
    assert!(preview_bounds.height > gpui::px(0.0));
    assert!(cx.debug_bounds("editor-whitespace-toggle").is_none());
  }

  #[gpui::test]
  fn markdown_preview_keeps_editor_and_preview_panes_visible(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempDir::new("git-page-markdown-preview-layout");
    let editor_root = TempDir::new("git-page-markdown-preview-editor-root");
    let rel_path = PathBuf::from("README.md");
    let markdown = "# Preview\n\nThe markdown preview pane should stay visible.\n";
    std::fs::write(repo.path.join(&rel_path), markdown).expect("write markdown file");

    let (git_page, cx) = add_git_page_window_with_root(cx);

    git_page.update_in(cx, |this, _window, cx| {
      let editor_root = editor_root.path.clone();
      let file_path = repo.path.join(&rel_path);
      let rel_path = rel_path.clone();
      let loaded = Editor::load_file_for_editor(&editor_root, &file_path);
      let editor =
        cx.new(move |cx| Editor::new_with_loaded_file(editor_root, file_path, loaded, cx));

      this.selected_repo = Some(repo.path.clone());
      this.selected_file = Some(rel_path);
      this.show_markdown_preview = true;
      this.editor = Some(editor);
      cx.notify();
    });

    let editor_bounds = cx
      .debug_bounds(GIT_MARKDOWN_PREVIEW_EDITOR_DEBUG_SELECTOR)
      .expect("editor preview pane bounds")
      .size;
    let preview_bounds = cx
      .debug_bounds(GIT_MARKDOWN_PREVIEW_RENDER_DEBUG_SELECTOR)
      .expect("render preview pane bounds")
      .size;

    assert!(editor_bounds.width > gpui::px(0.0));
    assert!(editor_bounds.height > gpui::px(0.0));
    assert!(preview_bounds.width > gpui::px(0.0));
    assert!(preview_bounds.height > gpui::px(0.0));
    assert!(cx.debug_bounds("editor-whitespace-toggle").is_some());
  }

  #[gpui::test]
  fn terminal_sidebar_renders_as_a_separate_right_panel(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempDir::new("git-page-terminal-sidebar-layout");
    cx.update(|cx| {
      AuthStateStore::set(
        cx,
        AuthState::Authenticated(Box::new(make_authenticated_test_user(UserRole::Admin))),
      );
    });

    let (git_page, cx) = add_git_page_window_with_root(cx);
    git_page.update_in(cx, |this, _window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.show_terminal_sidebar = true;
      cx.notify();
    });

    let terminal_bounds = cx
      .debug_bounds(GIT_TERMINAL_SIDEBAR_DEBUG_SELECTOR)
      .expect("terminal sidebar bounds")
      .size;

    assert!(terminal_bounds.width > gpui::px(0.0));
    assert!(terminal_bounds.height > gpui::px(0.0));
  }

  #[gpui::test]
  fn open_terminal_action_toggles_embedded_terminal_sidebar(cx: &mut TestAppContext) {
    init_gpui_test(cx);
    let repo = TempDir::new("git-page-terminal-sidebar-action");
    cx.update(|cx| {
      AuthStateStore::set(
        cx,
        AuthState::Authenticated(Box::new(make_authenticated_test_user(UserRole::Admin))),
      );
    });

    let (git_page, cx) = add_git_page_window_with_root(cx);
    git_page.update_in(cx, |this, window, cx| {
      this.selected_repo = Some(repo.path.clone());
      this.focus_page(window, cx);
    });

    git_page.update_in(cx, |this, window, cx| {
      this.toggle_terminal_sidebar_action(&crate::ToggleTerminalSidebar, window, cx);
      assert!(this.show_terminal_sidebar);
    });

    git_page.update_in(cx, |this, window, cx| {
      this.toggle_terminal_sidebar_action(&crate::ToggleTerminalSidebar, window, cx);
      assert!(!this.show_terminal_sidebar);
    });
  }

  #[test]
  fn should_show_editor_loading_state_only_when_file_selected_without_editor() {
    let selected = Path::new("src/main.rs");
    assert!(GitPage::should_show_editor_loading_state(
      Some(selected),
      false
    ));
    assert!(!GitPage::should_show_editor_loading_state(
      Some(selected),
      true
    ));
    assert!(!GitPage::should_show_editor_loading_state(None, false));
  }

  #[test]
  fn should_show_open_action_loading_state_only_for_pending_repo_open_actions() {
    let action = GitPageOpenAction::MergeBaseBranch {
      base_branch_name: "main".to_string(),
    };
    let selected = Path::new("src/main.rs");

    assert!(GitPage::should_show_open_action_loading_state(
      Some(&action),
      None,
      false,
    ));
    assert!(!GitPage::should_show_open_action_loading_state(
      Some(&action),
      Some(selected),
      false,
    ));
    assert!(!GitPage::should_show_open_action_loading_state(
      Some(&action),
      None,
      true,
    ));
    assert!(!GitPage::should_show_open_action_loading_state(
      None, None, false,
    ));
  }

  #[test]
  fn repository_split_is_hidden_when_no_repo_is_selected() {
    assert!(!GitPage::should_render_repository_split(None));
    assert!(GitPage::should_render_repository_split(Some(Path::new(
      "/tmp/reviu-selected-repo"
    ))));
  }

  #[test]
  fn hunk_action_top_uses_local_display_line_position() {
    let top = GitPage::hunk_action_top(gpui::px(20.0), 110, 109.0);
    assert_eq!(top, gpui::px(20.0));
  }

  #[test]
  fn hunk_action_top_handles_fractional_scroll_offset() {
    let top = GitPage::hunk_action_top(gpui::px(18.0), 10, 9.5);
    assert_eq!(top, gpui::px(9.0));
  }
}
