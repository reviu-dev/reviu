//! Everything the shell paints: sidebar, center, diff header, dock.

use super::*;
use crate::annotations::AnnotationKind;
use crate::auth_state::GithubAccessState;
use crate::diff_toolbar::{DiffToolbar, NavigationControl, SplitControl, ToggleControl};
use crate::hunk_actions::render_hunk_actions;
use crate::pro_promise::{ProPromiseSurface, render_pro_promise};
use gpui_component::Selectable as _;

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

    // Without GitHub the inbox has nothing to list, but the column still says
    // what it would be for.
    let github_access = AuthStateStore::github_access_state(cx);
    let github_section = match github_access {
      GithubAccessState::Available => Some(self.inbox.clone().into_any_element()),
      state => render_pro_promise(ProPromiseSurface::Inbox, state, cx),
    };

    let repo_name = self
      .selected_repo
      .as_deref()
      .and_then(|path| path.file_name())
      .map(|name| name.to_string_lossy().into_owned());

    let branch_status = self.repo_snapshot.read(cx).branch_status().cloned();
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
      .child(div().flex_1().min_h_0().child(self.session_list.clone()))
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
        // The conversation stays alongside the diff until the reviewer hides it.
        let view = if self.diff_chat_open {
          self.render_conversation_diff_split(window, cx)
        } else {
          self.render_diff_view(window, cx)
        };
        (
          SharedString::from(format!("session-center-diff-{file}")),
          view,
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

  pub(super) fn render_conversation_diff_split(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let theme = cx.theme().clone();
    h_flex()
      .id("session-conversation-diff-split")
      .size_full()
      .min_w(px(0.0))
      .min_h_0()
      .child(
        div()
          .relative()
          .flex_none()
          .w(px(self.conversation_split_width))
          .h_full()
          .min_w(px(CONVERSATION_SPLIT_MIN_WIDTH))
          .max_w(px(CONVERSATION_SPLIT_MAX_WIDTH))
          .border_r_1()
          .border_color(theme.border)
          .child(self.render_conversation(cx))
          .child(self.render_conversation_split_resize_handle(cx)),
      )
      .child(
        div()
          .flex_1()
          .min_w(px(0.0))
          .h_full()
          .child(self.render_diff_view(window, cx)),
      )
      .into_any_element()
  }

  pub(super) fn render_conversation(&mut self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let mut container = div()
      .debug_selector(|| "session-conversation-pane".to_string())
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
    let file_dirty = self
      .editor
      .as_ref()
      .is_some_and(|editor| editor.read(cx).is_dirty);
    let file_status = self.selected_file_status(cx);
    let old_path = self.selected_file_old_path(cx);
    let previewing = self.show_preview && self.previewable();
    let has_editor = self.editor.is_some();
    // A snapshot of a commit or of a pull request cannot be written back.
    let can_save = self
      .editor
      .as_ref()
      .is_some_and(|editor| !editor.read(cx).is_read_only);

    let mut toolbar = DiffToolbar::new("session-page").before_title(if self.diff_chat_open {
      Button::new("session-page-close-editor")
        .debug_selector(|| "session-page-close-editor".to_string())
        .icon(gpui_component::IconName::Close)
        .ghost()
        .compact()
        .small()
        .tooltip("Close editor (Esc)")
        .on_click(cx.listener(|this, _, window, cx| this.close_diff(window, cx)))
        .into_any_element()
    } else {
      Button::new("session-page-show-chat")
        .debug_selector(|| "session-page-show-chat".to_string())
        .label("Chat")
        .icon(UiIconName::MessageCircle)
        .ghost()
        .compact()
        .small()
        .tooltip("Back to the conversation (Esc)")
        .on_click(cx.listener(|this, _, window, cx| this.close_diff(window, cx)))
        .into_any_element()
    });

    if let Some(path) = self.selected_file.clone() {
      toolbar = toolbar.title(render_file_title_with_status(
        &path,
        old_path.as_deref(),
        file_status,
        file_dirty,
        cx,
      ));
    }

    if self.can_accept_all_conflicts(cx) {
      toolbar = toolbar
        .before_toggles(
          Button::new("session-page-accept-all-current")
            .label("Accept All Current")
            .debug_selector(|| ACCEPT_ALL_CURRENT_DEBUG_SELECTOR.to_string())
            .xsmall()
            .ghost()
            .on_click(cx.listener(|this, _, _, cx| {
              this.resolve_all_conflicts(ConflictResolution::Current, cx)
            }))
            .into_any_element(),
        )
        .before_toggles(
          Button::new("session-page-accept-all-incoming")
            .label("Accept All Incoming")
            .debug_selector(|| ACCEPT_ALL_INCOMING_DEBUG_SELECTOR.to_string())
            .xsmall()
            .ghost()
            .on_click(cx.listener(|this, _, _, cx| {
              this.resolve_all_conflicts(ConflictResolution::Incoming, cx)
            }))
            .into_any_element(),
        );
    }

    if let Some(state) = self.annotation_navigation(cx).filter(|_| !previewing) {
      let (previous_tooltip, next_tooltip) = match state.kind {
        AnnotationKind::Conflict => ("Previous conflict", "Next conflict"),
        AnnotationKind::Change => ("Previous change", "Next change"),
      };
      let view = cx.entity();
      let previous_view = view.clone();
      toolbar = toolbar.navigation(NavigationControl {
        active_index: state.active_index,
        total: state.total,
        enabled: can_navigate_annotations(Some(state)),
        previous_tooltip,
        next_tooltip,
        counter_debug_selector: ANNOTATION_COUNTER_DEBUG_SELECTOR,
        on_previous: Rc::new(move |_, cx| {
          previous_view.update(cx, |this, cx| {
            this.navigate_change(AnnotationDirection::Previous, cx)
          });
        }),
        on_next: Rc::new(move |_, cx| {
          view.update(cx, |this, cx| {
            this.navigate_change(AnnotationDirection::Next, cx)
          });
        }),
      });
    }

    if has_editor && self.previewable() {
      let view = cx.entity();
      toolbar = toolbar.preview(ToggleControl {
        active: self.show_preview,
        disabled: false,
        debug_selector: PREVIEW_TOGGLE_DEBUG_SELECTOR,
        on_toggle: Rc::new(move |_, cx| {
          view.update(cx, |this, cx| this.toggle_preview(cx));
        }),
      });
    }

    if has_editor && self.selected_file_has_changes(cx) && !previewing {
      if self.binary_preview.is_none() {
        let view = cx.entity();
        toolbar = toolbar.whitespace(ToggleControl {
          active: self.hide_whitespace,
          disabled: false,
          debug_selector: WHITESPACE_TOGGLE_DEBUG_SELECTOR,
          on_toggle: Rc::new(move |_, cx| {
            view.update(cx, |this, cx| this.toggle_hide_whitespace(cx));
          }),
        });
      }

      let view = cx.entity();
      toolbar = toolbar.split(SplitControl {
        mode: self.diff_view,
        disabled: self.split_disabled(cx),
        debug_selector: DIFF_VIEW_TOGGLE_DEBUG_SELECTOR,
        on_toggle: Rc::new(move |_, cx| {
          view.update(cx, |this, cx| this.toggle_diff_view(cx));
        }),
      });
    }

    if can_save && !previewing {
      let save_editor = self.editor.clone();
      toolbar = toolbar.after_toggles(
        Button::new("session-page-save-file")
          .debug_selector(|| SAVE_BUTTON_DEBUG_SELECTOR.to_string())
          .label("Save")
          .xsmall()
          .ghost()
          .disabled(!file_dirty)
          .on_click(move |_, _, cx| {
            if let Some(editor) = save_editor.clone() {
              editor.update(cx, |editor, cx| editor.save(cx));
            }
          })
          .into_any_element(),
      );
    }

    toolbar.render(cx)
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
      let hunk_actions = (self.opened_snapshot.is_none())
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

pub(super) const DOCK_RESIZE_HANDLE_DEBUG_SELECTOR: &str = "session-dock-resize-handle";
pub(super) const SIDEBAR_RESIZE_HANDLE_DEBUG_SELECTOR: &str = "session-sidebar-resize-handle";
pub(super) const CONVERSATION_SPLIT_RESIZE_HANDLE_DEBUG_SELECTOR: &str =
  "session-conversation-diff-resize-handle";

/// Width of a collapsed side panel: just enough for its icon rail.
pub(super) const SIDE_RAIL_WIDTH: f32 = 40.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum PanelSide {
  Left,
  Right,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ResizeTarget {
  SidePanel(PanelSide),
  ConversationSplit,
}

/// Payload of a resize drag; the ghost renders nothing.
#[derive(Clone)]
struct DraggedPanel(ResizeTarget);

impl Render for DraggedPanel {
  fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
    gpui::Empty
  }
}

/// A dot on a rail icon: something is waiting behind a panel you cannot see.
/// Exhaustive on purpose, so a new tab has to say whether it has news.
pub(crate) fn dock_rail_tab_has_news(
  tab: DockPanelTab,
  changed_files: usize,
  pending_review: usize,
) -> bool {
  match tab {
    DockPanelTab::Changes => changed_files > 0,
    DockPanelTab::Review => pending_review > 0,
    DockPanelTab::Files
    | DockPanelTab::History
    | DockPanelTab::PullRequest
    | DockPanelTab::Terminal => false,
  }
}

impl SessionPage {
  /// Absolute over the dock's left edge so it costs no layout column: the
  /// header border line stays continuous and the 1px separator comes from the
  /// dock's own border.
  fn render_panel_resize_handle(&self, side: PanelSide, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let (id, selector): (&'static str, &'static str) = match side {
      PanelSide::Left => (
        "session-sidebar-resize-handle",
        SIDEBAR_RESIZE_HANDLE_DEBUG_SELECTOR,
      ),
      PanelSide::Right => (
        "session-dock-resize-handle",
        DOCK_RESIZE_HANDLE_DEBUG_SELECTOR,
      ),
    };
    let handle = div()
      .id(id)
      .debug_selector(move || selector.to_string())
      .absolute()
      .top_0()
      .w(px(5.0))
      .h_full()
      .occlude()
      .cursor_col_resize()
      .hover(|this| this.bg(theme.border))
      .on_drag(
        DraggedPanel(ResizeTarget::SidePanel(side)),
        move |_, _, _, cx| {
          cx.stop_propagation();
          cx.new(|_| DraggedPanel(ResizeTarget::SidePanel(side)))
        },
      )
      .on_mouse_up(
        gpui::MouseButton::Left,
        cx.listener(move |this, event: &gpui::MouseUpEvent, _, cx| {
          if event.click_count == 2 {
            match side {
              PanelSide::Left => this.resize_sidebar(SESSIONS_SIDEBAR_DEFAULT_WIDTH, cx),
              PanelSide::Right => this.resize_dock(DOCK_PANEL_DEFAULT_WIDTH, cx),
            }
            cx.stop_propagation();
          }
        }),
      );
    // Centered on the panel's 1px border, not sitting beside it; deferred so
    // the overhanging half is not painted under the neighbouring column.
    gpui::deferred(match side {
      PanelSide::Left => handle.right(px(-2.0)),
      PanelSide::Right => handle.left(px(-2.0)),
    })
    .into_any_element()
  }

  fn render_conversation_split_resize_handle(&self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let handle = div()
      .id("session-conversation-diff-resize-handle")
      .debug_selector(|| CONVERSATION_SPLIT_RESIZE_HANDLE_DEBUG_SELECTOR.to_string())
      .absolute()
      .top_0()
      .w(px(5.0))
      .h_full()
      .occlude()
      .cursor_col_resize()
      .hover(|this| this.bg(theme.border))
      .on_drag(
        DraggedPanel(ResizeTarget::ConversationSplit),
        move |_, _, _, cx| {
          cx.stop_propagation();
          cx.new(|_| DraggedPanel(ResizeTarget::ConversationSplit))
        },
      )
      .on_mouse_up(
        gpui::MouseButton::Left,
        cx.listener(move |this, event: &gpui::MouseUpEvent, _, cx| {
          if event.click_count == 2 {
            this.resize_conversation_split(CONVERSATION_SPLIT_DEFAULT_WIDTH, cx);
            cx.stop_propagation();
          }
        }),
      );
    gpui::deferred(handle.right(px(-2.0))).into_any_element()
  }

  /// A side panel that slides between its width and its icon rail. The rail
  /// keeps every surface one click away instead of hiding them.
  #[allow(clippy::too_many_arguments)]
  fn render_side_panel(
    &self,
    side: PanelSide,
    open: bool,
    slide_armed: bool,
    width: f32,
    rail: AnyElement,
    content: AnyElement,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let theme = cx.theme().clone();
    // A permanent rail sits beside the panel, so the content slides to zero;
    // a replacing rail keeps its own width on screen.
    let collapsed_width = if side == PanelSide::Right {
      0.0
    } else {
      SIDE_RAIL_WIDTH
    };
    let (from, to) = if open {
      (collapsed_width, width)
    } else {
      (width, collapsed_width)
    };
    let clipped = div()
      .id(match side {
        PanelSide::Left => "session-sidebar-container",
        PanelSide::Right => "session-dock-container",
      })
      .h_full()
      .overflow_hidden();
    let clipped = match side {
      PanelSide::Left => clipped.border_r_1().border_color(theme.border),
      PanelSide::Right => clipped.border_l_1().border_color(theme.border),
    };
    // Right: the rail sits beside the panel for good. Left: it replaces it.
    let (replacing_rail, side_rail) = match side {
      PanelSide::Left => (Some(rail), None),
      PanelSide::Right => (None, Some(rail)),
    };
    let clipped = clipped.child(match replacing_rail {
      Some(rail) if !open => div().w(px(SIDE_RAIL_WIDTH)).h_full().child(rail),
      _ => div().w(px(width)).h_full().child(content),
    });
    let clipped: AnyElement = if slide_armed {
      clipped
        .with_animation(
          (
            match side {
              PanelSide::Left => "session-sidebar-slide",
              PanelSide::Right => "session-dock-slide",
            },
            open as u64,
          ),
          gpui::Animation::new(std::time::Duration::from_millis(CENTER_SWAP_FADE_MS))
            .with_easing(gpui::ease_out_quint()),
          move |this, delta| this.w(px(from + (to - from) * delta)),
        )
        .into_any_element()
    } else {
      clipped.w(px(to)).into_any_element()
    };
    // The grab strip straddles the border, so it lives outside the clip.
    let sliding = div()
      .relative()
      .h_full()
      .flex_shrink_0()
      .child(clipped)
      .when(open, |this| {
        this.child(self.render_panel_resize_handle(side, cx))
      });
    if let Some(rail) = side_rail {
      return h_flex()
        .h_full()
        .flex_shrink_0()
        .child(sliding)
        .child(div().w(px(SIDE_RAIL_WIDTH)).h_full().child(rail))
        .into_any_element();
    }
    sliding.into_any_element()
  }

  fn rail_button(
    id: &'static str,
    icon: impl Into<gpui_component::Icon>,
    tooltip: &'static str,
  ) -> Button {
    Button::new(id)
      .debug_selector(move || id.to_string())
      .icon(icon)
      .ghost()
      .compact()
      .small()
      .tooltip(tooltip)
  }

  /// The permanent tab rail of the right panel: icons never truncate, and the
  /// active icon closes the panel like its shortcut does.
  fn render_dock_rail(&self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    let active_tab = self.dock_panel.read(cx).active_tab();
    let tabs: [(
      &'static str,
      gpui_component::Icon,
      &'static str,
      DockPanelTab,
    ); 6] = [
      (
        "dock-rail-changes",
        gpui_component::Icon::new(UiIconName::FileDiff),
        "Changes",
        DockPanelTab::Changes,
      ),
      (
        "dock-rail-review",
        gpui_component::Icon::new(UiIconName::MessageCircle),
        "Review",
        DockPanelTab::Review,
      ),
      (
        "dock-rail-files",
        gpui_component::Icon::new(gpui_component::IconName::Folder),
        "Files",
        DockPanelTab::Files,
      ),
      (
        "dock-rail-history",
        gpui_component::Icon::new(UiIconName::History),
        "History",
        DockPanelTab::History,
      ),
      (
        "dock-rail-pull-request",
        gpui_component::Icon::new(UiIconName::GitPullRequest),
        "Pull request",
        DockPanelTab::PullRequest,
      ),
      (
        "dock-rail-terminal",
        gpui_component::Icon::new(UiIconName::SquareTerminal),
        "Terminal",
        DockPanelTab::Terminal,
      ),
    ];
    let mut rail = v_flex().items_center().gap_1().pt_2().w_full().child(
      Self::rail_button(
        "dock-rail-toggle",
        gpui_component::Icon::new(if self.dock_open {
          gpui_component::IconName::PanelRightClose
        } else {
          gpui_component::IconName::PanelRightOpen
        }),
        if self.dock_open {
          "Close panel"
        } else {
          "Open panel"
        },
      )
      .on_click(cx.listener(|this, _, window, cx| {
        if this.dock_open {
          this.close_dock(window, cx);
        } else {
          let tab = this.dock_panel.read(cx).active_tab();
          this.open_dock_tab(tab, window, cx);
        }
      })),
    );
    let pending_review = self.draft_review_comment_count();
    let changed_files = self.dock_panel.read(cx).status_entries().len();
    for (id, icon, tooltip, tab) in tabs {
      let button = Self::rail_button(id, icon, tooltip)
        .selected(self.dock_open && active_tab == tab)
        .on_click(cx.listener(move |this, _, window, cx| {
          this.open_dock_tab(tab, window, cx);
        }));
      let badge = dock_rail_tab_has_news(tab, changed_files, pending_review).then(|| {
        div()
          .absolute()
          .top(px(0.0))
          .right(px(2.0))
          .size(px(8.0))
          .rounded_full()
          .bg(theme.primary)
      });
      rail = rail.child(match badge {
        Some(badge) => div()
          .relative()
          .child(button)
          .child(badge)
          .into_any_element(),
        None => button.into_any_element(),
      });
    }
    div()
      .size_full()
      .border_l_1()
      .border_color(theme.border)
      .child(rail)
      .into_any_element()
  }

  fn render_sidebar_rail(&self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme().clone();
    div()
      .size_full()
      .bg(theme.sidebar)
      .child(
        v_flex()
          .items_center()
          .gap_1()
          .pt_2()
          .w_full()
          .child(
            Self::rail_button(
              "sidebar-rail-open",
              gpui_component::Icon::new(gpui_component::IconName::PanelLeftOpen),
              "Open sidebar",
            )
            .on_click(cx.listener(|this, _, _, cx| this.open_sidebar(cx))),
          )
          .child(
            Self::rail_button(
              "sidebar-rail-new-session",
              gpui_component::Icon::new(UiIconName::SquarePen),
              "New session",
            )
            .on_click(cx.listener(|this, _, window, cx| {
              this.open_sidebar(cx);
              this.new_session(window, cx);
            })),
          ),
      )
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
      .on_action(cx.listener(Self::jump_to_latest_message_action))
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
      .on_drag_move(cx.listener(
        |this, event: &gpui::DragMoveEvent<DraggedPanel>, window, cx| match event.drag(cx).0 {
          ResizeTarget::SidePanel(PanelSide::Left) => {
            this.resize_sidebar(f32::from(event.event.position.x), cx);
          }
          ResizeTarget::SidePanel(PanelSide::Right) => {
            // The permanent rail sits between the panel and the window edge.
            let width = window.viewport_size().width - event.event.position.x - px(SIDE_RAIL_WIDTH);
            this.resize_dock(f32::from(width), cx);
          }
          ResizeTarget::ConversationSplit => {
            let center_left = if this.sidebar_open {
              this.sidebar_width
            } else {
              SIDE_RAIL_WIDTH
            };
            this.resize_conversation_split(f32::from(event.event.position.x) - center_left, cx);
          }
        },
      ))
      .child(if self.dock_zoomed {
        div()
          .size_full()
          .min_w(px(0.0))
          .min_h_0()
          .child(self.render_dock_panel(cx))
          .into_any_element()
      } else {
        let sidebar_rail = self.render_sidebar_rail(cx);
        let sidebar_content = self.render_sessions_sidebar(cx);
        let sidebar = self.render_side_panel(
          PanelSide::Left,
          self.sidebar_open,
          self.sidebar_slide_armed,
          self.sidebar_width,
          sidebar_rail,
          sidebar_content,
          cx,
        );
        let dock_rail = self.render_dock_rail(cx);
        let dock_content = self.render_dock_panel(cx);
        let dock = self.render_side_panel(
          PanelSide::Right,
          self.dock_open,
          self.dock_slide_armed,
          self.dock_width,
          dock_rail,
          dock_content,
          cx,
        );
        h_flex()
          .size_full()
          .min_w(px(0.0))
          .min_h_0()
          .child(sidebar)
          .child(
            div()
              .flex_1()
              .min_w(px(0.0))
              .h_full()
              // The split editor can outgrow the column; without a clip its
              // hitboxes keep living under the rail and the dock.
              .overflow_hidden()
              .child(self.render_center(window, cx)),
          )
          .child(dock)
          .into_any_element()
      })
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
    assert_eq!(head.summary().expect("read summary"), Some("second"));

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
  async fn a_snapshot_offers_no_save_and_a_preview_offers_no_diff_controls(
    cx: &mut TestAppContext,
  ) {
    let repo = TempRepo::init("session-page-preview-toolbar");
    commit_text_file(&repo.path, Path::new("README.md"), "# one\n", "initial");
    let commit = commit_text_file(&repo.path, Path::new("README.md"), "# two\n", "second");
    std::fs::write(repo.path.join("README.md"), "# three\n").expect("update file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| panel.refresh(cx))
    });
    cx.run_until_parked();

    // A working-tree file can be written, so it keeps its Save.
    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("README.md"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    await_editor_diff(&page, cx).await;
    assert!(cx.debug_bounds(SAVE_BUTTON_DEBUG_SELECTOR).is_some());

    // The preview renders the document, so there is no diff left to act on.
    page.update(cx, |page, cx| page.toggle_preview(cx));
    cx.run_until_parked();
    assert!(cx.debug_bounds(SAVE_BUTTON_DEBUG_SELECTOR).is_none());
    assert!(cx.debug_bounds(ANNOTATION_COUNTER_DEBUG_SELECTOR).is_none());
    assert!(cx.debug_bounds(WHITESPACE_TOGGLE_DEBUG_SELECTOR).is_none());
    assert!(cx.debug_bounds(DIFF_VIEW_TOGGLE_DEBUG_SELECTOR).is_none());
    // The way back to the code is all that is left.
    assert!(cx.debug_bounds(PREVIEW_TOGGLE_DEBUG_SELECTOR).is_some());

    // A commit snapshot is read-only, in code view too.
    page.update_in(cx, |page, window, cx| {
      page.open_commit_file(commit.to_string(), PathBuf::from("README.md"), window, cx);
    });
    await_open_file(&page, cx).await;
    cx.run_until_parked();
    assert!(cx.debug_bounds(SAVE_BUTTON_DEBUG_SELECTOR).is_none());
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
    std::fs::write(repo.path.join("a.txt"), "v2\n").expect("modify file");

    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| panel.refresh(cx))
    });
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

  #[gpui::test]
  async fn the_active_tab_shortcut_toggles_the_dock_closed(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-dock-toggle");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    assert!(
      cx.debug_bounds("dock-panel-zoom").is_some(),
      "dock starts open"
    );

    // Changes is the active tab: its shortcut closes the dock.
    page.update_in(cx, |page, window, cx| {
      page.open_changes_action(&crate::OpenGitChangesSidebar, window, cx)
    });
    cx.executor()
      .advance_clock(std::time::Duration::from_millis(250));
    cx.run_until_parked();
    page.read_with(cx, |page, _| assert!(!page.dock_open));

    // Any tab shortcut reopens it on that tab.
    page.update_in(cx, |page, window, cx| {
      page.open_history_action(&crate::OpenGitHistorySidebar, window, cx)
    });
    cx.executor()
      .advance_clock(std::time::Duration::from_millis(250));
    cx.run_until_parked();
    page.read_with(cx, |page, cx| {
      assert!(page.dock_open);
      assert_eq!(page.dock_panel.read(cx).active_tab(), DockPanelTab::History);
    });
  }

  #[gpui::test]
  async fn zooming_the_dock_takes_the_whole_shell(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-dock-zoom");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    let zoom = cx.debug_bounds("dock-panel-zoom").expect("zoom button");
    cx.simulate_click(zoom.center(), gpui::Modifiers::default());
    cx.run_until_parked();

    page.read_with(cx, |page, _| assert!(page.dock_zoomed));
    assert!(
      cx.debug_bounds(REPO_CONTEXT_DEBUG_SELECTOR).is_none(),
      "the sidebar is hidden while the dock is zoomed"
    );

    let zoom = cx
      .debug_bounds("dock-panel-zoom")
      .expect("zoom button, zoomed");
    cx.simulate_click(zoom.center(), gpui::Modifiers::default());
    cx.run_until_parked();

    page.read_with(cx, |page, _| assert!(!page.dock_zoomed));
    assert!(
      cx.debug_bounds(REPO_CONTEXT_DEBUG_SELECTOR).is_some(),
      "restoring brings the shell back"
    );
  }

  #[gpui::test]
  async fn dragging_the_handle_resizes_the_dock(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-dock-drag");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    let start_width = page.read_with(cx, |page, _| page.dock_width);
    let handle = cx
      .debug_bounds(DOCK_RESIZE_HANDLE_DEBUG_SELECTOR)
      .expect("resize handle");
    let from = handle.center();
    let to = gpui::point(from.x - px(80.0), from.y);

    cx.simulate_event(gpui::MouseDownEvent {
      position: from,
      button: gpui::MouseButton::Left,
      modifiers: gpui::Modifiers::default(),
      click_count: 1,
      first_mouse: false,
    });
    // The first move only starts the drag; the next ones stream DragMoveEvent.
    cx.simulate_event(gpui::MouseMoveEvent {
      position: gpui::point(from.x - px(10.0), from.y),
      pressed_button: Some(gpui::MouseButton::Left),
      modifiers: gpui::Modifiers::default(),
    });
    cx.simulate_event(gpui::MouseMoveEvent {
      position: to,
      pressed_button: Some(gpui::MouseButton::Left),
      modifiers: gpui::Modifiers::default(),
    });
    cx.simulate_event(gpui::MouseUpEvent {
      position: to,
      button: gpui::MouseButton::Left,
      modifiers: gpui::Modifiers::default(),
      click_count: 1,
    });
    cx.run_until_parked();

    let width = page.read_with(cx, |page, _| page.dock_width);
    assert!(
      (width - (start_width + 80.0)).abs() < 5.0,
      "dragging 80px left widens the dock: {start_width} -> {width}"
    );
  }

  #[gpui::test]
  async fn editor_header_keeps_one_center_pane(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-chat-editor-close-buttons");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("modify file");
    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("README.md"), None, window, cx);
    });
    await_open_file(&page, cx).await;

    assert!(cx.debug_bounds("session-page-close-editor").is_some());
    assert!(cx.debug_bounds("session-page-show-chat").is_none());

    let close_editor = cx
      .debug_bounds("session-page-close-editor")
      .expect("close editor");
    cx.simulate_click(close_editor.center(), gpui::Modifiers::default());
    cx.run_until_parked();

    page.read_with(cx, |page, _| {
      assert_eq!(page.center, CenterView::Conversation);
      assert!(page.diff_chat_open);
    });
    assert!(cx.debug_bounds("session-page-close-editor").is_none());
    assert!(cx.debug_bounds("session-page-show-chat").is_none());

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("README.md"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    page.update_in(cx, |page, window, cx| page.hide_diff_chat(window, cx));
    cx.run_until_parked();

    page.read_with(cx, |page, _| {
      assert_eq!(page.center, CenterView::Diff);
      assert!(!page.diff_chat_open);
    });
    assert!(cx.debug_bounds("session-page-close-editor").is_none());
    assert!(cx.debug_bounds("session-page-show-chat").is_some());

    let show_chat = cx
      .debug_bounds("session-page-show-chat")
      .expect("show chat");
    cx.simulate_click(show_chat.center(), gpui::Modifiers::default());
    cx.run_until_parked();

    page.read_with(cx, |page, _| {
      assert_eq!(page.center, CenterView::Conversation);
      assert!(page.diff_chat_open);
    });
    assert!(cx.debug_bounds("session-page-close-editor").is_none());
    assert!(cx.debug_bounds("session-page-show-chat").is_none());
  }

  #[gpui::test]
  async fn dragging_the_conversation_split_handle_resizes_the_chat(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-conversation-split-drag");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("modify file");
    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("README.md"), None, window, cx);
    });
    await_open_file(&page, cx).await;

    let start_width = page.read_with(cx, |page, _| page.conversation_split_width);
    let handle = cx
      .debug_bounds(CONVERSATION_SPLIT_RESIZE_HANDLE_DEBUG_SELECTOR)
      .expect("conversation split resize handle");
    let from = handle.center();
    let to = gpui::point(from.x + px(80.0), from.y);

    cx.simulate_event(gpui::MouseDownEvent {
      position: from,
      button: gpui::MouseButton::Left,
      modifiers: gpui::Modifiers::default(),
      click_count: 1,
      first_mouse: false,
    });
    cx.simulate_event(gpui::MouseMoveEvent {
      position: gpui::point(from.x + px(10.0), from.y),
      pressed_button: Some(gpui::MouseButton::Left),
      modifiers: gpui::Modifiers::default(),
    });
    cx.simulate_event(gpui::MouseMoveEvent {
      position: to,
      pressed_button: Some(gpui::MouseButton::Left),
      modifiers: gpui::Modifiers::default(),
    });
    cx.simulate_event(gpui::MouseUpEvent {
      position: to,
      button: gpui::MouseButton::Left,
      modifiers: gpui::Modifiers::default(),
      click_count: 1,
    });
    cx.run_until_parked();

    let width = page.read_with(cx, |page, _| page.conversation_split_width);
    assert!(
      (width - (start_width + 80.0)).abs() < 5.0,
      "dragging 80px right widens the chat: {start_width} -> {width}"
    );
  }

  #[test]
  fn only_the_tabs_holding_work_wear_a_dot() {
    assert!(dock_rail_tab_has_news(DockPanelTab::Changes, 2, 0));
    assert!(!dock_rail_tab_has_news(DockPanelTab::Changes, 0, 3));
    assert!(dock_rail_tab_has_news(DockPanelTab::Review, 0, 3));
    assert!(!dock_rail_tab_has_news(DockPanelTab::Review, 2, 0));

    for tab in [
      DockPanelTab::Files,
      DockPanelTab::History,
      DockPanelTab::PullRequest,
      DockPanelTab::Terminal,
    ] {
      assert!(!dock_rail_tab_has_news(tab, 5, 5));
    }
  }

  #[gpui::test]
  async fn closing_the_dock_hands_focus_to_the_center(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-dock-close-focus");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    std::fs::write(repo.path.join("README.md"), "v2\n").expect("modify file");
    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| panel.refresh(cx))
    });
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("README.md"), None, window, cx);
    });
    await_open_file(&page, cx).await;

    page.update_in(cx, |page, window, cx| {
      page.open_changes_action(&crate::OpenGitChangesSidebar, window, cx)
    });
    cx.run_until_parked();

    let editor_handle = page.read_with(cx, |page, cx| {
      page.editor.as_ref().expect("editor").focus_handle(cx)
    });
    let focused = cx.update(|window, cx| {
      let _ = cx;
      window.focused(cx)
    });
    assert_eq!(
      focused.as_ref(),
      Some(&editor_handle),
      "the editor takes the focus back so shortcuts keep working"
    );
  }

  #[gpui::test]
  async fn reopening_changes_on_a_clean_tree_keeps_a_live_focus(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-dock-clean-focus");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    // Close, then reopen Changes on a worktree with nothing in it.
    page.update_in(cx, |page, window, cx| {
      page.open_changes_action(&crate::OpenGitChangesSidebar, window, cx)
    });
    cx.run_until_parked();
    page.update_in(cx, |page, window, cx| {
      page.open_changes_action(&crate::OpenGitChangesSidebar, window, cx)
    });
    cx.run_until_parked();

    page.read_with(cx, |page, _| assert!(page.dock_open));
    let dock_handle = page.read_with(cx, |page, cx| page.dock_panel.read(cx).focus_handle(cx));
    let focused = cx.update(|window, cx| {
      let _ = cx;
      window.focused(cx)
    });
    assert_eq!(
      focused.as_ref(),
      Some(&dock_handle),
      "an empty changes tab must not send the focus to an unmounted list"
    );
  }

  #[gpui::test]
  async fn the_pull_request_tab_keeps_its_place_without_github(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-dock-rail-no-github");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_changes_action(&crate::OpenGitChangesSidebar, window, cx)
    });
    page.update(cx, |page, cx| {
      page.dock_slide_armed = false;
      cx.notify();
    });
    cx.run_until_parked();

    // Icons that come and go with the remote make the rail unlearnable, and the
    // panel behind this one is a promotion surface.
    assert!(cx.debug_bounds("dock-rail-pull-request").is_some());
  }

  #[gpui::test]
  async fn the_dock_rail_reopens_on_the_clicked_tab(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-dock-rail");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    // Close via the active tab shortcut: the rail takes over.
    page.update_in(cx, |page, window, cx| {
      page.open_changes_action(&crate::OpenGitChangesSidebar, window, cx)
    });
    // The test scheduler never finishes animations; pin the collapsed width.
    page.update(cx, |page, cx| {
      page.dock_slide_armed = false;
      cx.notify();
    });
    cx.run_until_parked();
    let history = cx
      .debug_bounds("dock-rail-history")
      .expect("the collapsed dock shows its tab rail");

    cx.simulate_click(history.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    page.read_with(cx, |page, cx| {
      assert!(page.dock_open, "a rail click opens the dock");
      assert_eq!(page.dock_panel.read(cx).active_tab(), DockPanelTab::History);
    });

    // The rail is permanent: clicking the active icon closes the panel.
    page.update(cx, |page, cx| {
      page.dock_slide_armed = false;
      cx.notify();
    });
    cx.run_until_parked();
    let history = cx.debug_bounds("dock-rail-history").expect("rail history");
    cx.simulate_click(history.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    page.read_with(cx, |page, cx| {
      assert!(!page.dock_open, "the active icon toggles the panel shut");
      assert_eq!(
        page.dock_panel.read(cx).active_tab(),
        DockPanelTab::History,
        "the remembered tab survives the close"
      );
    });

    // The rail-top toggle reopens on that remembered tab.
    page.update(cx, |page, cx| {
      page.dock_slide_armed = false;
      cx.notify();
    });
    cx.run_until_parked();
    let toggle = cx.debug_bounds("dock-rail-toggle").expect("rail toggle");
    cx.simulate_click(toggle.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    page.read_with(cx, |page, cx| {
      assert!(page.dock_open);
      assert_eq!(page.dock_panel.read(cx).active_tab(), DockPanelTab::History);
    });
  }

  #[gpui::test]
  async fn the_sidebar_collapses_to_a_rail_and_comes_back(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-sidebar-rail");
    commit_text_file(&repo.path, Path::new("README.md"), "v1\n", "initial");
    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    cx.run_until_parked();

    let collapse = cx
      .debug_bounds("session-sidebar-collapse")
      .expect("collapse button");
    cx.simulate_click(collapse.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    page.read_with(cx, |page, _| assert!(!page.sidebar_open));

    // The test scheduler never finishes animations; pin the collapsed width.
    page.update(cx, |page, cx| {
      page.sidebar_slide_armed = false;
      cx.notify();
    });
    cx.run_until_parked();
    let open = cx
      .debug_bounds("sidebar-rail-open")
      .expect("the collapsed sidebar shows its rail");
    cx.simulate_click(open.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    page.read_with(cx, |page, _| assert!(page.sidebar_open));
  }

  #[gpui::test]
  async fn a_rail_click_never_starts_a_selection_in_the_split_editor(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-split-clickthrough");
    commit_text_file(
      &repo.path,
      Path::new("a.txt"),
      "one\ntwo\nthree\n",
      "initial",
    );
    std::fs::write(repo.path.join("a.txt"), "one\nTWO\nthree\n").expect("modify file");
    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| panel.refresh(cx))
    });
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("a.txt"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    await_editor_diff(&page, cx).await;
    page.update(cx, |page, cx| page.toggle_diff_view(cx));
    cx.run_until_parked();

    // The user's gesture: press on a rail tab, drift the mouse, release.
    let files = cx.debug_bounds("dock-rail-files").expect("rail files");
    let press = files.center();
    cx.simulate_event(gpui::MouseDownEvent {
      position: press,
      button: gpui::MouseButton::Left,
      modifiers: gpui::Modifiers::default(),
      click_count: 1,
      first_mouse: false,
    });
    cx.simulate_event(gpui::MouseMoveEvent {
      position: gpui::point(press.x, press.y + px(120.0)),
      pressed_button: Some(gpui::MouseButton::Left),
      modifiers: gpui::Modifiers::default(),
    });
    cx.simulate_event(gpui::MouseUpEvent {
      position: gpui::point(press.x, press.y + px(120.0)),
      button: gpui::MouseButton::Left,
      modifiers: gpui::Modifiers::default(),
      click_count: 1,
    });
    cx.run_until_parked();

    page.read_with(cx, |page, cx| {
      let selection = page
        .editor
        .as_ref()
        .expect("editor")
        .read(cx)
        .selected_text_for_copy(cx);
      assert!(
        selection.is_none(),
        "a click on the rail must not select text in the editor: {selection:?}"
      );
    });
  }

  #[gpui::test]
  async fn hovering_the_right_split_pane_keeps_its_hunk_hover(cx: &mut TestAppContext) {
    let repo = TempRepo::init("session-split-right-hover");
    commit_text_file(
      &repo.path,
      Path::new("a.txt"),
      "one\ntwo\nthree\n",
      "initial",
    );
    std::fs::write(repo.path.join("a.txt"), "one\nTWO\nthree\n").expect("modify file");
    let (page, cx) = add_session_page_window(repo.path.clone(), cx);
    page.update(cx, |page, cx| {
      page.dock_panel.update(cx, |panel, cx| panel.refresh(cx))
    });
    cx.run_until_parked();

    page.update_in(cx, |page, window, cx| {
      page.open_diff(PathBuf::from("a.txt"), None, window, cx);
    });
    await_open_file(&page, cx).await;
    await_editor_diff(&page, cx).await;
    page.update(cx, |page, cx| page.toggle_diff_view(cx));
    cx.run_until_parked();

    let editor_bounds = cx
      .debug_bounds(DIFF_EDITOR_DEBUG_SELECTOR)
      .expect("editor bounds");
    let hover_pane = |cx: &mut gpui::VisualTestContext, x: gpui::Pixels| -> bool {
      let mut y = editor_bounds.top() + px(4.0);
      while y < editor_bounds.top() + px(200.0) {
        cx.simulate_mouse_move(gpui::point(x, y), None, gpui::Modifiers::default());
        cx.run_until_parked();
        let hovered = page.read_with(cx, |page, cx| {
          page
            .editor
            .as_ref()
            .expect("editor")
            .read(cx)
            .hovered_group_id
            .is_some()
        });
        if hovered {
          return true;
        }
        y += px(10.0);
      }
      false
    };

    // The right pane (the added side) must light up the hunk actions...
    let right_x = editor_bounds.right() - editor_bounds.size.width * 0.25;
    assert!(
      hover_pane(cx, right_x),
      "hovering the changed line on the right pane must set the hunk hover"
    );
    // ...and so must the left pane.
    let left_x = editor_bounds.left() + editor_bounds.size.width * 0.25;
    assert!(
      hover_pane(cx, left_x),
      "hovering the changed line on the left pane must set the hunk hover"
    );
  }
}
