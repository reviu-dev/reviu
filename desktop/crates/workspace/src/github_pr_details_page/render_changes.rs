//! What the Changes tab draws: the file sidebar, the diff and their skeletons.

use super::*;

fn repo_status_for_pr_file(status: GithubPrFileStatus) -> RepoStatusKind {
  match status {
    GithubPrFileStatus::Added => RepoStatusKind::Added,
    GithubPrFileStatus::Modified => RepoStatusKind::Modified,
    GithubPrFileStatus::Deleted => RepoStatusKind::Deleted,
    GithubPrFileStatus::Renamed => RepoStatusKind::Renamed,
  }
}

fn status_letter(status: GithubPrFileStatus) -> &'static str {
  match status {
    GithubPrFileStatus::Added => "A",
    GithubPrFileStatus::Modified => "M",
    GithubPrFileStatus::Deleted => "D",
    GithubPrFileStatus::Renamed => "R",
  }
}

fn status_color(status: GithubPrFileStatus, theme: &gpui_component::Theme) -> gpui::Hsla {
  match status {
    GithubPrFileStatus::Modified => theme.status_orange(),
    GithubPrFileStatus::Added => theme.status_green(),
    GithubPrFileStatus::Deleted => theme.status_red(),
    GithubPrFileStatus::Renamed => theme.info,
  }
}

fn visible_review_comment_counts_by_path(
  file_lookup: &HashMap<String, Rc<GithubPrFileDiff>>,
  review_comments: &[GithubPullRequestReviewComment],
) -> HashMap<String, usize> {
  if file_lookup.is_empty() || review_comments.is_empty() {
    return HashMap::new();
  }

  let comments_by_id: HashMap<u64, &GithubPullRequestReviewComment> = review_comments
    .iter()
    .map(|comment| (comment.id, comment))
    .collect();
  let mut counts = HashMap::new();

  for comment in review_comments {
    let Some(file) = file_for_review_comment_path(file_lookup, comment.path.as_str()) else {
      continue;
    };
    if resolve_review_comment_display_anchor(comment, &comments_by_id).is_none() {
      continue;
    }
    *counts.entry(file.path.to_string()).or_insert(0) += 1;
  }

  counts
}

impl GithubPrDetailsPage {
  pub(super) fn render_files_sidebar(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let theme = cx.theme().clone();
    let local_project_mode = self.local_project_mode_active(cx);
    let count = self.active_file_count(cx);
    let tree_search_active = self.tree_search_query_normalized().is_some();

    if self.tree_search_reset_pending {
      let tree_search_input = self.tree_search_input.clone();
      cx.on_next_frame(window, move |this, window, cx| {
        if this.tree_search_reset_pending {
          tree_search_input.update(cx, |input, cx| input.set_value("", window, cx));
          this.tree_search_reset_pending = false;
          cx.notify();
        }
      });
    }

    if let Some(selected_id) = self
      .tree_state
      .read(cx)
      .selected_entry()
      .map(|entry| entry.item().id.to_string())
      && Some(selected_id.as_str()) != self.current_selected_tree_path().as_deref()
    {
      cx.on_next_frame(window, move |this, _, cx| {
        this.select_visible_tree_path(selected_id.as_str(), cx);
      });
    }

    let in_commit_by_commit_mode = self.selected_commit_sha.is_some();
    let commits_disabled =
      self.commits.is_empty() || self.commits_loading || self.commits_error.is_some();
    let toggle_view = cx.entity();
    let commit_by_commit_toggle = Button::new("github-pr-commit-by-commit-toggle")
      .ghost()
      .xsmall()
      .compact()
      .icon(UiIconName::GitCommitHorizontal)
      .label(if in_commit_by_commit_mode {
        "All changes"
      } else {
        "Commit by commit"
      })
      .tooltip_with_action(
        if in_commit_by_commit_mode {
          "Show all changes"
        } else {
          "Review commit by commit"
        },
        &crate::ToggleCommitByCommit,
        Some(crate::shortcuts::PR_CHANGES_ONLY_CONTEXT),
      )
      .disabled(commits_disabled)
      .on_click(move |_, _, cx| {
        toggle_view.update(cx, |this, cx| {
          if this.selected_commit_sha.is_some() {
            this.exit_commit_by_commit_review(cx);
          } else {
            this.enter_commit_by_commit_review(cx);
          }
        });
      });

    let header = h_flex()
      .pl_3()
      .pr_2()
      .items_center()
      .justify_between()
      .h(px(DIFF_HEADER_HEIGHT))
      .border_b_1()
      .border_color(theme.border)
      .child(
        h_flex()
          .items_center()
          .gap_2()
          .child(div().text_sm().text_color(theme.foreground).child("Files"))
          .child(
            Tag::secondary()
              .small()
              .rounded_full()
              .child(count.to_string()),
          ),
      )
      .child(commit_by_commit_toggle);

    let commit_by_commit_row = self.render_commit_by_commit_row(&theme, cx);

    let comment_counts = if self.selected_commit_sha.is_none() && !self.review_comments.is_empty() {
      visible_review_comment_counts_by_path(&self.file_lookup, &self.review_comments)
    } else {
      HashMap::new()
    };

    let list = if local_project_mode && self.local_project_tree_loading {
      v_flex()
        .flex_1()
        .h_full()
        .items_center()
        .justify_center()
        .gap_2()
        .child(Spinner::new().small())
        .child(
          div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child("Loading project files..."),
        )
        .into_any_element()
    } else if local_project_mode && self.local_project_tree_error.is_some() {
      v_flex()
        .flex_1()
        .h_full()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.status_red())
        .child(self.local_project_tree_error.clone().unwrap_or_default())
        .into_any_element()
    } else if !local_project_mode && self.files_loading {
      Self::render_changes_files_sidebar_skeleton(&theme)
    } else if !local_project_mode && self.files_error.is_some() {
      v_flex()
        .flex_1()
        .h_full()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.status_red())
        .child(self.files_error.clone().unwrap_or_default())
        .into_any_element()
    } else if count == 0 {
      v_flex()
        .flex_1()
        .h_full()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.muted_foreground)
        .child(if tree_search_active {
          "No matching files"
        } else if local_project_mode {
          "No project files found"
        } else {
          "No files changed"
        })
        .into_any_element()
    } else {
      let view = cx.entity();
      tree(&self.tree_state, move |ix, entry, selected, _window, cx| {
        view.update(cx, |this, cx| {
          let theme = cx.theme().clone();
          let item = entry.item();
          let is_folder = entry.is_folder();
          let status = if is_folder {
            None
          } else {
            this
              .file_lookup
              .get(item.id.as_ref())
              .map(|file| file.status)
          };
          let status_letter = status.map(status_letter).unwrap_or("");
          let status_color = status
            .map(|status| status_color(status, &theme))
            .unwrap_or(theme.muted_foreground);
          let comment_count = if is_folder {
            0
          } else {
            comment_counts.get(item.id.as_ref()).copied().unwrap_or(0)
          };
          let icon = if is_folder {
            if entry.is_expanded() {
              Icon::new(IconName::FolderOpen)
            } else {
              Icon::new(IconName::Folder)
            }
            .size_3()
            .text_color(theme.muted_foreground)
            .into_any_element()
          } else {
            file_icon_path_for_name_with_theme(item.label.as_ref(), &theme)
              .map(|path| img(path).size(px(FILE_ICON_SIZE_PX)).into_any_element())
              .unwrap_or_else(|| {
                Icon::new(IconName::File)
                  .size_3()
                  .text_color(theme.muted_foreground)
                  .into_any_element()
              })
          };

          let indent = px(12.) + px(15.) * entry.depth();
          let mut row = selectable_list_item(ix, selected, SelectableRowStyle::Inset, &theme)
            .w_full()
            .px_2()
            .pl(indent)
            .child(
              h_flex()
                .items_center()
                .gap_2()
                .when(!is_folder, |this| {
                  this.child(
                    div()
                      .w(px(15.))
                      .text_xs()
                      .text_color(status_color)
                      .child(status_letter),
                  )
                })
                .child(icon)
                .child(
                  div()
                    .flex_1()
                    .overflow_hidden()
                    .text_ellipsis()
                    .text_sm()
                    .child(item.label.clone()),
                )
                .when(comment_count > 0, |this| {
                  this.child(
                    h_flex()
                      .items_center()
                      .gap_1()
                      .text_xs()
                      .pr_2()
                      .text_color(theme.muted_foreground)
                      .child(
                        Icon::new(UiIconName::MessageCircle)
                          .size_3()
                          .text_color(theme.muted_foreground),
                      )
                      .child(comment_count.to_string()),
                  )
                }),
            );

          if !is_folder {
            let id = item.id.clone();
            row = row.on_click(cx.listener(move |this, _, _, cx| {
              this.select_visible_tree_path(id.as_ref(), cx);
            }));
          }

          row
        })
      })
      .flex_1()
      .w_full()
      .into_any_element()
    };

    v_flex()
      .bg(theme.sidebar)
      .size_full()
      .child(header)
      .when_some(commit_by_commit_row, |this, row| this.child(row))
      .child(
        div()
          .pb_1()
          .px_1()
          .flex_1()
          .min_h_0()
          .key_context(crate::shortcuts::GITHUB_PR_CHANGES_TREE_CONTEXT)
          .child(list),
      )
  }

  pub(super) fn render_commit_by_commit_row(
    &self,
    theme: &gpui_component::Theme,
    cx: &mut Context<Self>,
  ) -> Option<AnyElement> {
    let current_index = self.selected_commit_index()?;
    let total = self.commits.len();
    let commit = self.commits.get(current_index)?;
    // Commits are newest-first; older commits live at higher indices.
    let has_previous = current_index + 1 < total;
    let has_next = current_index > 0;
    // Display chronological position (oldest = 1, newest = total).
    let position = total.saturating_sub(current_index);
    let subject = github_shared::commit_subject(&commit.message);
    let prev_view = cx.entity();
    let next_view = cx.entity();

    Some(
      h_flex()
        .gap_2()
        .px_3()
        .py_2()
        .items_center()
        .justify_between()
        .border_b_1()
        .border_color(theme.border)
        .child(
          div()
            .min_w_0()
            .flex_1()
            .text_sm()
            .text_color(theme.foreground)
            .overflow_hidden()
            .text_ellipsis()
            .child(subject),
        )
        .child(
          h_flex()
            .items_center()
            .gap_1()
            .child(
              Button::new("github-pr-commit-prev")
                .ghost()
                .xsmall()
                .compact()
                .icon(IconName::ChevronLeft)
                .tooltip_with_action(
                  "Previous commit",
                  &crate::PreviousPrCommit,
                  Some(crate::shortcuts::PR_CHANGES_ONLY_CONTEXT),
                )
                .disabled(!has_previous)
                .on_click(move |_, _, cx| {
                  prev_view.update(cx, |this, cx| {
                    this.navigate_commit_by_commit(CommitNavigationDirection::Previous, cx);
                  });
                }),
            )
            .child(
              div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(format!("{}/{}", position, total)),
            )
            .child(
              Button::new("github-pr-commit-next")
                .ghost()
                .xsmall()
                .compact()
                .icon(IconName::ChevronRight)
                .tooltip_with_action(
                  "Next commit",
                  &crate::NextPrCommit,
                  Some(crate::shortcuts::PR_CHANGES_ONLY_CONTEXT),
                )
                .disabled(!has_next)
                .on_click(move |_, _, cx| {
                  next_view.update(cx, |this, cx| {
                    this.navigate_commit_by_commit(CommitNavigationDirection::Next, cx);
                  });
                }),
            ),
        )
        .into_any_element(),
    )
  }

  pub(super) fn render_diff_header(
    &self,
    file: &GithubPrFileDiff,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let path = Path::new(file.path.as_ref());
    let is_markdown = is_markdown_path(path);
    let is_svg = is_svg_path(path);
    let preview_active = (is_markdown || is_svg) && self.show_markdown_preview;
    let file_loading = self.file_loading;
    let split_disabled = self.split_disabled_for_selected_file() || preview_active;
    let hunk_navigation = self
      .diff_editor
      .read(cx)
      .hunk_navigation_state(cx)
      .filter(|state| state.total > 1);

    let title = render_file_title_with_status(
      path,
      file.old_path.as_ref().map(|old| Path::new(old.as_ref())),
      Some(repo_status_for_pr_file(file.status)),
      false,
      cx,
    );

    let mut toolbar = DiffToolbar::new("pr").filled(true).title(title);

    if let Some(state) = hunk_navigation {
      let previous_view = cx.entity();
      let next_view = cx.entity();
      toolbar = toolbar.navigation(NavigationControl {
        active_index: state.active_index,
        total: state.total,
        enabled: !file_loading,
        previous_tooltip: "Previous change",
        next_tooltip: "Next change",
        counter_debug_selector: PR_CHANGE_COUNTER_DEBUG_SELECTOR,
        on_previous: Rc::new(move |_, cx| {
          previous_view.update(cx, |this, cx| {
            this.diff_editor.update(cx, |editor, cx| {
              editor.navigate_hunk(HunkNavigationDirection::Previous, cx);
            });
          });
        }),
        on_next: Rc::new(move |_, cx| {
          next_view.update(cx, |this, cx| {
            this.diff_editor.update(cx, |editor, cx| {
              editor.navigate_hunk(HunkNavigationDirection::Next, cx);
            });
          });
        }),
      });
    }

    if is_markdown || is_svg {
      let view = cx.entity();
      toolbar = toolbar.preview(ToggleControl {
        active: preview_active,
        disabled: file_loading,
        debug_selector: PR_PREVIEW_TOGGLE_DEBUG_SELECTOR,
        on_toggle: Rc::new(move |_, cx| {
          view.update(cx, |this, cx| this.toggle_markdown_preview(cx));
        }),
      });
    }

    if self.binary_preview.is_none() {
      let view = cx.entity();
      toolbar = toolbar.whitespace(ToggleControl {
        active: self.hide_whitespace,
        disabled: file_loading,
        debug_selector: PR_WHITESPACE_TOGGLE_DEBUG_SELECTOR,
        on_toggle: Rc::new(move |_, cx| {
          view.update(cx, |this, cx| this.toggle_hide_whitespace(cx));
        }),
      });
    }

    let view = cx.entity();
    toolbar = toolbar.split(SplitControl {
      mode: self.diff_view,
      disabled: split_disabled || file_loading,
      debug_selector: PR_DIFF_VIEW_TOGGLE_DEBUG_SELECTOR,
      on_toggle: Rc::new(move |_, cx| {
        view.update(cx, |this, cx| this.toggle_diff_view(cx));
      }),
    });

    toolbar.render(cx)
  }

  pub(super) fn render_selected_editor_content(
    &mut self,
    is_markdown: bool,
    is_svg: bool,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> gpui::AnyElement {
    if let Some(binary_preview) = self.binary_preview.as_ref() {
      return self.render_binary_preview_content(binary_preview, cx);
    }

    let theme = cx.theme().clone();
    let preview_active = self.show_markdown_preview && (is_markdown || is_svg);

    if preview_active {
      let preview_panel = if is_svg {
        let editor = self.diff_editor.clone();
        self.svg_preview.update(cx, |preview, cx| {
          preview.refresh_from_editor(&editor, window, cx);
        });
        self.svg_preview.read(cx).render(cx)
      } else {
        let markdown = self.diff_editor.read(cx).document().read(cx);
        let markdown = markdown.slice_to_string(0..markdown.len());
        div()
          .flex_1()
          .min_h_0()
          .min_w(px(0.0))
          .bg(theme.background)
          .child(
            div().size_full().pb_4().px_4().child(
              TextView::markdown("github-pr-markdown-preview-text", markdown)
                .size_full()
                .selectable(true)
                .scrollable(true),
            ),
          )
          .into_any_element()
      };
      return div()
        .flex_1()
        .min_h_0()
        .child(
          h_resizable("github-pr-markdown-preview")
            .child(
              resizable_panel().child(
                div()
                  .size_full()
                  .min_w(px(0.0))
                  .min_h_0()
                  .flex()
                  .flex_col()
                  .debug_selector(|| GITHUB_PR_MARKDOWN_PREVIEW_EDITOR_DEBUG_SELECTOR.to_string())
                  .child(self.diff_editor.clone()),
              ),
            )
            .child(
              resizable_panel().child(
                div()
                  .size_full()
                  .min_w(px(0.0))
                  .min_h_0()
                  .flex()
                  .flex_col()
                  .debug_selector(|| GITHUB_PR_MARKDOWN_PREVIEW_RENDER_DEBUG_SELECTOR.to_string())
                  .child(preview_panel),
              ),
            ),
        )
        .into_any_element();
    }

    div()
      .flex_1()
      .min_h_0()
      .child(self.diff_editor.clone())
      .into_any_element()
  }

  pub(super) fn render_changes_files_sidebar_skeleton(theme: &gpui_component::Theme) -> AnyElement {
    v_flex()
      .flex_1()
      .p_2()
      .gap_1()
      .children((0..14).map(|ix| {
        let width = match ix % 5 {
          0 => 190.0,
          1 => 145.0,
          2 => 220.0,
          3 => 110.0,
          _ => 165.0,
        };
        let indent = if ix % 3 == 0 { 0.0 } else { 16.0 };

        h_flex()
          .h(px(28.0))
          .items_center()
          .gap_2()
          .pl(px(8.0 + indent))
          .pr_2()
          .child(
            Skeleton::new()
              .w(px(15.0))
              .h(px(12.0))
              .rounded(theme.radius)
              .secondary(),
          )
          .child(Skeleton::new().size(px(14.0)).rounded(theme.radius))
          .child(
            Skeleton::new()
              .w(px(width))
              .h(px(12.0))
              .rounded(theme.radius),
          )
      }))
      .into_any_element()
  }

  pub(super) fn render_changes_diff_header_skeleton(theme: &gpui_component::Theme) -> AnyElement {
    h_flex()
      .h(px(DIFF_HEADER_HEIGHT))
      .bg(theme.sidebar)
      .px_3()
      .items_center()
      .justify_between()
      .border_b_1()
      .border_color(theme.border)
      .child(
        h_flex()
          .items_center()
          .gap_2()
          .min_w_0()
          .child(
            Skeleton::new()
              .w(px(15.0))
              .h(px(12.0))
              .rounded(theme.radius)
              .secondary(),
          )
          .child(Skeleton::new().size(px(14.0)).rounded(theme.radius))
          .child(
            Skeleton::new()
              .w(px(220.0))
              .h(px(14.0))
              .rounded(theme.radius),
          )
          .child(
            Skeleton::new()
              .w(px(120.0))
              .h(px(12.0))
              .rounded(theme.radius)
              .secondary(),
          ),
      )
      .child(
        h_flex()
          .items_center()
          .gap_2()
          .child(Skeleton::new().size(px(20.0)).rounded(theme.radius))
          .child(Skeleton::new().size(px(20.0)).rounded(theme.radius))
          .child(
            Skeleton::new()
              .w(px(86.0))
              .h(px(20.0))
              .rounded(theme.radius),
          )
          .child(
            Skeleton::new()
              .w(px(64.0))
              .h(px(20.0))
              .rounded(theme.radius),
          ),
      )
      .into_any_element()
  }

  pub(super) fn render_changes_diff_body_skeleton(theme: &gpui_component::Theme) -> AnyElement {
    v_flex()
      .flex_1()
      .min_h_0()
      .p_4()
      .gap_2()
      .children((0..22).map(|ix| {
        let width = match ix % 7 {
          0 => 320.0,
          1 => 540.0,
          2 => 220.0,
          3 => 610.0,
          4 => 380.0,
          5 => 460.0,
          _ => 280.0,
        };

        h_flex()
          .h(px(20.0))
          .items_center()
          .gap_3()
          .child(
            Skeleton::new()
              .w(px(28.0))
              .h(px(12.0))
              .rounded(theme.radius)
              .secondary(),
          )
          .child(
            Skeleton::new()
              .w_full()
              .max_w(px(width))
              .h(px(12.0))
              .rounded(theme.radius),
          )
      }))
      .into_any_element()
  }

  pub(super) fn render_changes_diff_skeleton(theme: &gpui_component::Theme) -> AnyElement {
    v_flex()
      .flex_1()
      .min_h_0()
      .child(Self::render_changes_diff_header_skeleton(theme))
      .child(Self::render_changes_diff_body_skeleton(theme))
      .into_any_element()
  }

  pub(super) fn render_changes_tab(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let theme = cx.theme().clone();
    let local_project_mode = self.local_project_mode_active(cx);
    let is_markdown = self.selected_file_is_markdown();
    let is_svg = self.selected_file_is_svg();
    let editor_content: gpui::AnyElement = if self.file_loading {
      // editor_panel already renders the real header for the selected file
      // skip the header skeleton to avoid stacking it on top of the live header.
      let header_already_rendered =
        self.selected_file.is_some() || self.selected_local_project_file.is_some();
      if header_already_rendered {
        Self::render_changes_diff_body_skeleton(&theme)
      } else {
        Self::render_changes_diff_skeleton(&theme)
      }
    } else if self.file_error.is_some() {
      v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.status_red())
        .child(self.file_error.clone().unwrap_or_default())
        .into_any_element()
    } else if self.selected_file.is_some() || self.selected_local_project_file.is_some() {
      self.render_selected_editor_content(is_markdown, is_svg, window, cx)
    } else if local_project_mode && self.local_project_tree_loading {
      v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_2()
        .child(Spinner::new().small())
        .child(
          div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child("Loading project files..."),
        )
        .into_any_element()
    } else if local_project_mode && self.local_project_tree_error.is_some() {
      v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.status_red())
        .child(self.local_project_tree_error.clone().unwrap_or_default())
        .into_any_element()
    } else if !local_project_mode && self.files_loading {
      Self::render_changes_diff_skeleton(&theme)
    } else if !local_project_mode && self.files_error.is_some() {
      v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.status_red())
        .child(self.files_error.clone().unwrap_or_default())
        .into_any_element()
    } else if local_project_mode {
      v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.muted_foreground)
        .child("Select a file to view it")
        .into_any_element()
    } else {
      v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.muted_foreground)
        .child("Select a file to view diff")
        .into_any_element()
    };

    let editor_panel = v_flex()
      .size_full()
      .overflow_hidden()
      .when_some(self.selected_file.as_ref(), |this, file| {
        this.child(self.render_diff_header(file, cx))
      })
      .when(
        self.selected_file.is_none() && self.selected_local_project_file.is_some(),
        |this| {
          this.when_some(self.selected_local_project_file.as_ref(), |this, file| {
            this.child(self.render_local_project_file_header(file, cx))
          })
        },
      )
      .child(editor_content);

    let files_sidebar = self.render_files_sidebar(window, cx);

    h_resizable("github-pr-changes-layout")
      .child(
        resizable_panel()
          .size(px(SIDEBAR_DEFAULT_WIDTH))
          .size_range(px(SIDEBAR_MIN_WIDTH)..px(SIDEBAR_MAX_WIDTH))
          .child(files_sidebar),
      )
      .child(resizable_panel().child(editor_panel))
  }
}

#[cfg(test)]
mod tests {
  use super::super::test_support::*;
  use super::super::*;
  use super::*;

  #[test]
  fn status_letter_covers_all_file_statuses() {
    assert_eq!(status_letter(GithubPrFileStatus::Added), "A");
    assert_eq!(status_letter(GithubPrFileStatus::Modified), "M");
    assert_eq!(status_letter(GithubPrFileStatus::Deleted), "D");
    assert_eq!(status_letter(GithubPrFileStatus::Renamed), "R");
  }

  #[test]
  fn visible_review_comment_counts_by_path_ignores_unanchored_comments_and_maps_renames() {
    let files = files_from_api(vec![
      make_api_file("src/main.rs", "modified", None),
      make_api_file("src/new.rs", "renamed", Some("src/old.rs")),
    ]);
    let lookup: HashMap<String, Rc<GithubPrFileDiff>> = files
      .into_iter()
      .map(|file| (file.path.as_ref().to_string(), file))
      .collect();

    let mut renamed_comment = make_review_comment(1, "2026-02-28T10:00:00Z", None);
    renamed_comment.path = "src/old.rs".to_string();
    renamed_comment.line = Some(3);

    let mut outdated_comment = make_review_comment(2, "2026-02-28T10:01:00Z", None);
    outdated_comment.path = "src/main.rs".to_string();
    outdated_comment.line = None;
    outdated_comment.start_line = None;
    outdated_comment.original_line = Some(7);
    outdated_comment.original_start_line = Some(7);

    let counts =
      visible_review_comment_counts_by_path(&lookup, &[renamed_comment, outdated_comment]);

    assert_eq!(counts.get("src/new.rs"), Some(&1));
    assert!(counts.get("src/main.rs").is_none());
  }
}
