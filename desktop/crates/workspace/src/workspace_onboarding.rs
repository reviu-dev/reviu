use agent_acp::BackendAvailability;
use gpui::{
  AnyElement, App, Context, Entity, FocusHandle, Focusable, InteractiveElement as _, Render,
  ScrollHandle, SharedString, StatefulInteractiveElement as _, Subscription, Window, div, img,
  prelude::*, px, relative,
};
use gpui_component::{
  ActiveTheme as _, Disableable as _, Icon, IndexPath, Sizable as _,
  button::Button,
  checkbox::Checkbox,
  h_flex,
  scroll::ScrollableElement as _,
  select::{Select, SelectEvent, SelectItem, SelectState},
  v_flex,
};
use ui::{REVIU_WORDMARK_WIDTH_PX, StatusThemeExt as _, UiIconName, reviu_logo_path};

use crate::config::AppSettings as PersistedSettings;
use crate::session_page::SessionPage;
use crate::shortcuts;
use crate::{OpenProject, ShowCommandPalette};

#[derive(Clone)]
struct AgentSelectOption {
  id: agent_registry::AgentId,
  title: SharedString,
  description: String,
}

impl SelectItem for AgentSelectOption {
  type Value = agent_registry::AgentId;

  fn title(&self) -> SharedString {
    self.title.clone()
  }

  fn display_title(&self) -> Option<AnyElement> {
    Some(
      h_flex()
        .gap_2()
        .items_center()
        .child(agent_chat_panel::backend_icon(&self.id).size_4())
        .child(self.title.clone())
        .into_any_element(),
    )
  }

  fn render(&self, _: &mut Window, cx: &mut App) -> impl IntoElement {
    let theme = cx.theme().clone();
    h_flex()
      .min_w_0()
      .gap_2()
      .items_center()
      .child(agent_chat_panel::backend_icon(&self.id).size_4())
      .child(
        div()
          .min_w_0()
          .child(div().text_sm().child(self.title.clone()))
          .child(
            div()
              .text_xs()
              .text_color(theme.muted_foreground)
              .truncate()
              .child(self.description.clone()),
          ),
      )
  }

  fn value(&self) -> &Self::Value {
    &self.id
  }

  fn matches(&self, query: &str) -> bool {
    let query = query.to_lowercase();
    self.title.to_lowercase().contains(&query)
      || self.description.to_lowercase().contains(&query)
      || self.id.as_str().contains(&query)
  }
}

pub(crate) struct WorkspaceOnboarding {
  session_page: Entity<SessionPage>,
  focus_handle: FocusHandle,
  split_diff_view: bool,
  default_agent: agent_registry::AgentId,
  enabled_agents: Vec<agent_registry::AgentId>,
  agent_scroll_handle: ScrollHandle,
  default_agent_select: Entity<SelectState<Vec<AgentSelectOption>>>,
  _subscriptions: Vec<Subscription>,
}

impl WorkspaceOnboarding {
  pub(crate) fn new(
    session_page: Entity<SessionPage>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    let agent_settings = crate::agent_settings::AgentSettings::load_preferences();
    let default_agent_options = Self::default_agent_options(&agent_settings.enabled_agents);
    let selected_default_agent = default_agent_options
      .iter()
      .position(|option| option.id == agent_settings.default_agent)
      .map(IndexPath::new);
    let default_agent_select = cx.new(|cx| {
      SelectState::new(default_agent_options, selected_default_agent, window, cx).searchable(true)
    });
    let default_agent_select_subscription = cx.subscribe(
      &default_agent_select,
      move |this, _, event: &SelectEvent<Vec<AgentSelectOption>>, cx| {
        let SelectEvent::Confirm(Some(agent_id)) = event else {
          return;
        };
        let updated = crate::agent_settings::AgentSettings::set_default_agent(agent_id.clone());
        this.default_agent = updated.default_agent;
        this.enabled_agents = updated.enabled_agents;
        cx.notify();
        cx.refresh_windows();
      },
    );

    Self {
      session_page,
      focus_handle: cx.focus_handle(),
      split_diff_view: PersistedSettings::get(cx).split_diff_view,
      default_agent: agent_settings.default_agent,
      enabled_agents: agent_settings.enabled_agents,
      agent_scroll_handle: ScrollHandle::new(),
      default_agent_select,
      _subscriptions: vec![default_agent_select_subscription],
    }
  }

  fn default_agent_options(enabled_agents: &[agent_registry::AgentId]) -> Vec<AgentSelectOption> {
    agent_registry::global()
      .runnable()
      .into_iter()
      .filter(|agent| enabled_agents.contains(&agent.id))
      .map(|agent| AgentSelectOption {
        id: agent.id.clone(),
        title: agent.display_name().into(),
        description: if agent.description.trim().is_empty() {
          format!("Registry id: {}", agent.id)
        } else {
          agent.description.clone()
        },
      })
      .collect()
  }

  fn apply_agent_settings(
    &mut self,
    updated: crate::agent_settings::AgentSettings,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.default_agent = updated.default_agent;
    self.enabled_agents = updated.enabled_agents;
    self.sync_default_agent_select(window, cx);
    cx.notify();
  }

  fn sync_default_agent_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let options = Self::default_agent_options(&self.enabled_agents);
    let default_agent = self.default_agent.clone();
    self.default_agent_select.update(cx, |select, cx| {
      select.set_items(options, window, cx);
      select.set_selected_value(&default_agent, window, cx);
    });
  }

  fn finish_setup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    PersistedSettings::update(cx, |settings| settings.onboarding_done = true);
    self.session_page.update(cx, |page, cx| {
      let focus_handle = page.focus_handle(cx);
      window.focus(&focus_handle, cx);
    });
    cx.refresh_windows();
  }

  fn open_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self.finish_setup(window, cx);
    self
      .session_page
      .update(cx, |page, cx| page.prompt_open_project(window, cx));
  }

  fn open_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self.session_page.update(cx, |page, cx| {
      page.open_workspace_command_palette(window, cx)
    });
  }

  fn open_project_action(&mut self, _: &OpenProject, window: &mut Window, cx: &mut Context<Self>) {
    self.open_project(window, cx);
    cx.stop_propagation();
  }

  fn show_command_palette_action(
    &mut self,
    _: &ShowCommandPalette,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.open_command_palette(window, cx);
    cx.stop_propagation();
  }

  fn render_section(
    &self,
    title: &'static str,
    description: &'static str,
    child: impl IntoElement,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let theme = cx.theme().clone();
    v_flex()
      .gap_4()
      .px_5()
      .pt_3()
      .pb_4()
      .rounded_md()
      .border_1()
      .border_color(theme.border)
      .bg(theme.background)
      .child(
        v_flex()
          .gap_1()
          .child(
            div()
              .text_lg()
              .font_weight(gpui::FontWeight::MEDIUM)
              .child(title),
          )
          .child(
            div()
              .text_sm()
              .text_color(theme.muted_foreground)
              .child(description),
          ),
      )
      .child(child)
  }

  fn render_agent_row(
    &self,
    agent: &agent_registry::RegistryAgent,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let theme = cx.theme().clone();
    let agent_id = agent.id.clone();
    let label: SharedString = agent.display_name().into();
    let description = if agent.description.trim().is_empty() {
      format!("Registry id: {agent_id}")
    } else {
      agent.description.clone()
    };
    let is_enabled = self.enabled_agents.contains(&agent_id);
    let only_enabled = self.enabled_agents.len() == 1 && is_enabled;
    let is_default = self.default_agent == agent_id;
    let availability = agent_chat_panel::backend_config_for(agent).check_availability();
    let status = match availability {
      BackendAvailability::Ok => h_flex()
        .gap_1()
        .items_center()
        .child(
          Icon::new(UiIconName::Check)
            .size_3()
            .text_color(theme.status_green()),
        )
        .child(
          div()
            .text_xs()
            .text_color(theme.status_green())
            .child("Ready"),
        )
        .into_any_element(),
      BackendAvailability::MissingBinary { command, .. } => div()
        .text_xs()
        .text_color(theme.status_amber())
        .child(format!("Missing `{command}`"))
        .into_any_element(),
    };

    h_flex()
      .id(format!("onboarding-agent-row-{agent_id}"))
      .w_full()
      .items_center()
      .justify_between()
      .gap_3()
      .p_3()
      .rounded_md()
      .border_1()
      .border_color(if is_default {
        theme.primary.opacity(0.45)
      } else {
        theme.border.opacity(0.7)
      })
      .bg(if is_default {
        theme.primary.opacity(0.08)
      } else {
        theme.secondary.opacity(0.35)
      })
      .cursor_pointer()
      .hover(|this| this.bg(theme.secondary_hover))
      .on_click({
        let view = cx.entity();
        let agent_id = agent_id.clone();
        move |_, window, cx| {
          if only_enabled {
            return;
          }
          let updated =
            crate::agent_settings::AgentSettings::set_agent_enabled(agent_id.clone(), !is_enabled);
          view.update(cx, |view, cx| {
            view.apply_agent_settings(updated, window, cx);
          });
          cx.refresh_windows();
        }
      })
      .child(
        h_flex()
          .min_w_0()
          .flex_1()
          .gap_3()
          .items_center()
          .child(
            Checkbox::new(format!("onboarding-agent-enabled-{agent_id}"))
              .checked(is_enabled)
              .disabled(only_enabled),
          )
          .child(agent_chat_panel::backend_icon(&agent_id).size_4())
          .child(
            v_flex()
              .min_w_0()
              .gap_0p5()
              .child(
                h_flex()
                  .gap_2()
                  .items_center()
                  .child(div().text_sm().child(label))
                  .when(is_default, |this| {
                    this.child(
                      div()
                        .px_1p5()
                        .py_0p5()
                        .rounded_md()
                        .bg(theme.primary.opacity(0.12))
                        .text_xs()
                        .text_color(theme.primary)
                        .child("Default"),
                    )
                  }),
              )
              .child(
                div()
                  .text_xs()
                  .text_color(theme.muted_foreground)
                  .truncate()
                  .child(description),
              ),
          ),
      )
      .child(status)
      .into_any_element()
  }

  fn render_agents_section(
    &self,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let registry = agent_registry::global();
    let agents = registry
      .runnable()
      .into_iter()
      .map(|agent| self.render_agent_row(agent, cx))
      .collect::<Vec<_>>();
    let default_agent_select = self.default_agent_select.clone();
    let body = v_flex()
      .gap_4()
      .child(
        h_flex()
          .items_center()
          .justify_between()
          .gap_4()
          .child(
            div()
              .text_sm()
              .text_color(cx.theme().muted_foreground)
              .child("Pick at least one agent. New chats use your default agent."),
          )
          .child(
            h_flex()
              .items_center()
              .gap_2()
              .child(div().text_sm().child("Default"))
              .child(
                Select::new(&default_agent_select)
                  .placeholder("Choose an agent...")
                  .small()
                  .w(px(280.)),
              ),
          ),
      )
      .child(
        div()
          .relative()
          .child(
            v_flex()
              .id("onboarding-agent-list")
              .gap_2()
              .max_h(px(300.))
              .overflow_y_scroll()
              .track_scroll(&self.agent_scroll_handle)
              .children(agents),
          )
          .vertical_scrollbar(&self.agent_scroll_handle),
      );

    self.render_section(
      "Agents",
      "Choose which ACP agents Reviu shows. You can change this later in Settings > Agents.",
      body,
      cx,
    )
  }

  fn render_diff_preview(&self, split: bool, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let line = |color: gpui::Hsla, width: f32| {
      div()
        .h(px(4.))
        .w(px(width))
        .rounded(px(2.))
        .bg(color.opacity(0.72))
    };
    let column = |removed: bool| {
      let color = if removed {
        theme.status_red()
      } else {
        theme.status_green()
      };
      v_flex()
        .flex_1()
        .gap_2()
        .p_3()
        .bg(theme.secondary.opacity(0.5))
        .child(line(theme.muted_foreground, 44.))
        .child(line(color, 84.))
        .child(line(theme.muted_foreground, 64.))
        .child(line(color, 104.))
        .child(line(theme.muted_foreground, 76.))
    };

    if split {
      h_flex()
        .gap_2()
        .child(column(true))
        .child(column(false))
        .into_any_element()
    } else {
      v_flex()
        .gap_2()
        .child(line(theme.muted_foreground, 120.))
        .child(line(theme.status_red(), 92.))
        .child(line(theme.status_green(), 108.))
        .child(line(theme.muted_foreground, 72.))
        .child(line(theme.status_green(), 132.))
        .into_any_element()
    }
  }

  fn render_diff_card(
    &self,
    title: &'static str,
    description: &'static str,
    split: bool,
    cx: &mut Context<Self>,
  ) -> impl IntoElement {
    let theme = cx.theme().clone();
    let selected = self.split_diff_view == split;
    div()
      .id(format!("onboarding-diff-card-{title}"))
      .flex_1()
      .min_w(px(240.))
      .p_4()
      .rounded_md()
      .border_1()
      .border_color(if selected {
        theme.primary
      } else {
        theme.border
      })
      .bg(if selected {
        theme.primary.opacity(0.08)
      } else {
        theme.secondary.opacity(0.35)
      })
      .cursor_pointer()
      .hover(|this| this.bg(theme.secondary_hover))
      .on_click({
        let view = cx.entity();
        move |_, _, cx| {
          PersistedSettings::update(cx, |settings| settings.split_diff_view = split);
          view.update(cx, |view, cx| {
            view.split_diff_view = split;
            cx.notify();
          });
          cx.refresh_windows();
        }
      })
      .child(
        v_flex()
          .gap_3()
          .child(
            h_flex()
              .justify_between()
              .items_center()
              .child(
                div()
                  .text_sm()
                  .font_weight(gpui::FontWeight::MEDIUM)
                  .child(title),
              )
              .when(selected, |this| {
                this.child(
                  Icon::new(UiIconName::Check)
                    .size_4()
                    .text_color(theme.primary),
                )
              }),
          )
          .child(
            div()
              .h(px(110.))
              .rounded_md()
              .border_1()
              .border_color(theme.border.opacity(0.7))
              .overflow_hidden()
              .p_3()
              .child(self.render_diff_preview(split, cx)),
          )
          .child(
            div()
              .text_xs()
              .text_color(theme.muted_foreground)
              .child(description),
          ),
      )
  }

  fn render_diff_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
    let body = h_flex()
      .gap_3()
      .flex_wrap()
      .child(self.render_diff_card(
        "Inline",
        "Changes are stacked in one flow. This is the default.",
        false,
        cx,
      ))
      .child(self.render_diff_card(
        "Split",
        "Old and new code sit side by side for wider reviews.",
        true,
        cx,
      ));

    self.render_section(
      "Diff view",
      "Choose the default review layout. You can change it later in Settings > General > Editor.",
      body,
      cx,
    )
  }
}

impl Render for WorkspaceOnboarding {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();

    div()
      .id("workspace-onboarding")
      .debug_selector(|| "workspace-onboarding".to_string())
      .key_context(shortcuts::current_workspace_key_context(cx).as_str())
      .track_focus(&self.focus_handle)
      .on_action(cx.listener(Self::open_project_action))
      .on_action(cx.listener(Self::show_command_palette_action))
      .size_full()
      .bg(theme.background)
      .overflow_y_scroll()
      .child(
        h_flex().w_full().justify_center().child(
          v_flex()
            .w_full()
            .max_w(px(920.))
            .p_8()
            .gap_6()
            .child(
              h_flex()
                .justify_between()
                .items_center()
                .gap_6()
                .child(
                  h_flex()
                    .gap_6()
                    .items_center()
                    .child(
                      img(reviu_logo_path(theme.mode.is_dark()))
                        .w(px(REVIU_WORDMARK_WIDTH_PX))
                        .h_auto(),
                    )
                    .child(
                      v_flex()
                        .child(
                          div()
                            .text_2xl()
                            .line_height(relative(1.0))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Welcome to Reviu"),
                        )
                        .child(
                          div()
                            .mt(px(7.))
                            .text_sm()
                            .line_height(relative(1.0))
                            .text_color(theme.muted_foreground)
                            .child("A few choices now make the first review feel familiar."),
                        ),
                    ),
                )
                .child(
                  Button::new("onboarding-finish-top")
                    .outline()
                    .small()
                    .label("Finish setup")
                    .on_click({
                      let view = cx.entity();
                      move |_, window, cx| {
                        view.update(cx, |view, cx| view.finish_setup(window, cx));
                      }
                    }),
                ),
            )
            .child(self.render_agents_section(window, cx))
            .child(self.render_diff_section(cx)),
        ),
      )
  }
}

impl Focusable for WorkspaceOnboarding {
  fn focus_handle(&self, _: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}
