use gpui::{
  App, Context, Corner, FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement,
  Render, SharedString, Styled, Task, Window, div, prelude::*, px,
};
use gpui_component::{
  ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _,
  avatar::Avatar,
  button::{Button, ButtonVariants as _},
  h_flex,
  menu::{DropdownMenu as _, PopupMenuItem},
  skeleton::Skeleton,
  tag::Tag,
  v_flex,
};
use smol::unblock;

use ui::{
  CommandPalette, CommandPaletteAction, CommandPaletteCommand, CommandPaletteConfig,
  CommandPaletteHandler, CommandPalettePage, StatusThemeExt as _, UiIconName, WindowExt,
};

use crate::{
  ShowCommandPalette,
  api::{ApiClient, GithubUserProfile, GithubUserProfileRepository},
  auth_state::AuthStateStore,
  github_navigation::{open_commit_target, open_pr_target, open_profile_target, open_repo_target},
  github_page::GithubPageHandle,
  github_shared,
  navigation::{NavigationHistory, build_github_profile_path},
  number_format,
  workspace::WorkspaceApi,
};

const PROFILE_CONTAINER_MAX_WIDTH: f32 = 1120.0;
const PROFILE_SIDEBAR_WIDTH: f32 = 280.0;

fn profile_login_from_pathname(pathname: &str) -> Option<String> {
  let segments = pathname
    .trim_start_matches('/')
    .split('/')
    .filter(|segment| !segment.is_empty())
    .collect::<Vec<_>>();

  if segments.len() == 2 && segments[0] == "github" {
    Some(segments[1].to_string())
  } else {
    None
  }
}

fn display_profile_name(profile: &GithubUserProfile) -> SharedString {
  profile
    .name
    .as_ref()
    .filter(|name| !name.trim().is_empty())
    .cloned()
    .unwrap_or_else(|| profile.login.clone())
    .into()
}

fn display_optional_text(value: &Option<String>) -> Option<String> {
  value
    .as_ref()
    .map(|value| value.trim())
    .filter(|value| !value.is_empty())
    .map(ToString::to_string)
}

fn strip_url_scheme(value: &str) -> String {
  value
    .trim()
    .trim_start_matches("https://")
    .trim_start_matches("http://")
    .trim_end_matches('/')
    .to_string()
}

fn profile_github_url(login: &str, html_url: Option<&str>) -> String {
  html_url
    .map(str::trim)
    .filter(|url| !url.is_empty())
    .map(ToString::to_string)
    .unwrap_or_else(|| format!("https://github.com/{}", login.trim()))
}

fn repository_language_color(
  repository: &GithubUserProfileRepository,
  theme: &gpui_component::Theme,
) -> gpui::Hsla {
  github_shared::parse_language_color(repository.language_color.as_deref())
    .unwrap_or(theme.muted_foreground)
}

#[derive(Clone, Default)]
pub struct GithubProfilePageHandle {
  page: Option<gpui::WeakEntity<GithubProfilePage>>,
}

impl gpui::Global for GithubProfilePageHandle {}

impl GithubProfilePageHandle {
  pub fn register(cx: &mut Context<GithubProfilePage>) {
    cx.set_global(Self {
      page: Some(cx.entity().downgrade()),
    });
  }

  pub fn show(login: SharedString, cx: &mut App) {
    let Some(weak) = cx.global::<Self>().page.clone() else {
      return;
    };

    let login_string = login.to_string();
    let _ = weak.update(cx, |this, cx| {
      this.load_profile(login_string, cx);
    });

    NavigationHistory::navigate(build_github_profile_path(&login), cx);
  }

  pub fn refresh(cx: &mut App) {
    let Some(weak) = cx.global::<Self>().page.clone() else {
      return;
    };
    let _ = weak.update(cx, |this, cx| this.refresh_current_profile(cx));
  }

  pub fn is_refreshing(cx: &App) -> bool {
    let Some(weak) = cx
      .try_global::<Self>()
      .and_then(|handle| handle.page.clone())
    else {
      return false;
    };

    weak
      .read_with(cx, |this, _| this.profile_loading)
      .unwrap_or(false)
  }
}

pub struct GithubProfilePage {
  focus_handle: FocusHandle,
  api: ApiClient,
  login: SharedString,
  profile: Option<GithubUserProfile>,
  profile_loading: bool,
  profile_error: Option<SharedString>,
  profile_task: Option<Task<()>>,
}

impl GithubProfilePage {
  pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
    GithubProfilePageHandle::register(cx);

    Self {
      focus_handle: cx.focus_handle(),
      api: WorkspaceApi::global(cx).api.clone(),
      login: "".into(),
      profile: None,
      profile_loading: false,
      profile_error: None,
      profile_task: None,
    }
  }

  fn sync_route(&mut self, cx: &mut Context<Self>) {
    let pathname = NavigationHistory::current_pathname(cx);
    let Some(login) = profile_login_from_pathname(&pathname) else {
      return;
    };

    if self.login.as_ref().eq_ignore_ascii_case(&login)
      && (self.profile.is_some() || self.profile_loading)
    {
      return;
    }

    self.load_profile(login, cx);
  }

  fn refresh_current_profile(&mut self, cx: &mut Context<Self>) {
    if self.login.is_empty() {
      return;
    }

    self.load_profile(self.login.to_string(), cx);
  }

  fn load_profile(&mut self, login: String, cx: &mut Context<Self>) {
    if login.trim().is_empty() {
      return;
    }

    self.login = login.clone().into();
    self.profile = None;
    self.profile_loading = true;
    self.profile_error = None;

    let api = self.api.clone();
    let requested_login = login.clone();
    let task = cx.spawn(async move |this, cx| {
      let result = unblock(move || api.fetch_github_user_profile(&requested_login)).await;

      let _ = this.update(cx, |this, cx| {
        if !this.login.as_ref().eq_ignore_ascii_case(&login) {
          return;
        }

        this.profile_loading = false;
        this.profile_task = None;

        match result {
          Ok(profile) => {
            this.profile = Some(profile);
            this.profile_error = None;
          }
          Err(error) => {
            this.profile = None;
            this.profile_error = Some(error.to_string().into());
          }
        }

        cx.notify();
      });
    });

    self.profile_task = Some(task);
    cx.notify();
  }

  fn open_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let include_github = AuthStateStore::has_github_access(cx);
    let commands =
      CommandPaletteCommand::default_global_commands(CommandPalettePage::Github, include_github);
    let view = cx.entity();
    let handler: CommandPaletteHandler = std::sync::Arc::new(move |action, window, cx| {
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

  fn show_command_palette_action(
    &mut self,
    _: &ShowCommandPalette,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.open_command_palette(window, cx);
  }

  fn handle_command_palette_action(
    &mut self,
    action: CommandPaletteAction,
    _window: &mut Window,
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
      CommandPaletteAction::OpenGithubProfile { login } => {
        open_profile_target(login, cx);
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
      CommandPaletteAction::OpenGithubCommitDetails { owner, repo, sha } => {
        open_commit_target(owner, repo, sha, cx);
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
      CommandPaletteAction::OpenGitConfigPage => {
        NavigationHistory::navigate("/git-config", cx);
        Ok(())
      }
      _ => Err("Command not available.".into()),
    }
  }

  fn render_header(&self, cx: &mut Context<Self>) -> gpui::Div {
    let theme = cx.theme().clone();
    let title = if self.login.is_empty() {
      "GitHub profile".to_string()
    } else {
      self.login.to_string()
    };
    let profile_url = profile_github_url(
      &title,
      self
        .profile
        .as_ref()
        .map(|profile| profile.html_url.as_str()),
    );

    h_flex()
      .w_full()
      .items_center()
      .justify_between()
      .px_3()
      .py_2()
      .bg(theme.sidebar)
      .border_b_1()
      .border_color(theme.title_bar_border)
      .child(
        h_flex()
          .items_center()
          .gap_2()
          .child(
            Button::new("github-profile-back")
              .icon(IconName::ArrowLeft)
              .ghost()
              .compact()
              .on_click(|_, _, cx| {
                NavigationHistory::navigate_back(cx);
              }),
          )
          .child(div().text_sm().font_medium().child(title)),
      )
      .child(
        Button::new("github-profile-actions-menu")
          .icon(UiIconName::EllipsisVertical)
          .ghost()
          .small()
          .compact()
          .dropdown_menu_with_anchor(Corner::TopRight, move |menu, _, _| {
            let view_url = profile_url.clone();
            menu.item(
              PopupMenuItem::new("View on GitHub")
                .icon(Icon::new(IconName::ExternalLink))
                .on_click(move |_, _, cx| {
                  cx.open_url(&view_url);
                }),
            )
          }),
      )
  }

  fn render_loading(&self, cx: &mut Context<Self>) -> gpui::Div {
    let theme = cx.theme().clone();

    h_flex()
      .w_full()
      .max_w(px(PROFILE_CONTAINER_MAX_WIDTH))
      .mx_auto()
      .items_start()
      .gap_8()
      .px_6()
      .py_6()
      .child(
        v_flex()
          .w(px(PROFILE_SIDEBAR_WIDTH))
          .gap_3()
          .child(Skeleton::new().size(px(80.0)).rounded_full())
          .child(
            Skeleton::new()
              .w(px(180.0))
              .h(px(20.0))
              .rounded(theme.radius),
          )
          .child(
            Skeleton::new()
              .w(px(120.0))
              .h(px(14.0))
              .rounded(theme.radius)
              .secondary(),
          )
          .child(Skeleton::new().w_full().h(px(8.0)).rounded_full()),
      )
      .child(
        v_flex()
          .flex_1()
          .gap_3()
          .child(h_flex().gap_3().children((0..4).map(|_| {
            v_flex()
              .flex_1()
              .gap_2()
              .p_3()
              .border_1()
              .border_color(theme.border)
              .rounded(theme.radius)
              .child(
                Skeleton::new()
                  .w(px(64.0))
                  .h(px(20.0))
                  .rounded(theme.radius),
              )
              .child(
                Skeleton::new()
                  .w(px(96.0))
                  .h(px(12.0))
                  .rounded(theme.radius)
                  .secondary(),
              )
          })))
          .children((0..5).map(|_| {
            v_flex()
              .gap_2()
              .p_3()
              .border_1()
              .border_color(theme.border)
              .rounded(theme.radius)
              .child(
                Skeleton::new()
                  .w(px(220.0))
                  .h(px(16.0))
                  .rounded(theme.radius),
              )
              .child(
                Skeleton::new()
                  .w(px(420.0))
                  .h(px(12.0))
                  .rounded(theme.radius)
                  .secondary(),
              )
          })),
      )
  }

  fn render_stat(
    &self,
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    icon: Icon,
    cx: &mut Context<Self>,
  ) -> gpui::Div {
    let theme = cx.theme().clone();

    v_flex()
      .flex_1()
      .min_w(px(130.0))
      .gap_2()
      .p_3()
      .border_1()
      .border_color(theme.border)
      .rounded(theme.radius)
      .child(
        h_flex()
          .items_center()
          .justify_between()
          .child(div().text_xl().font_semibold().child(value.into()))
          .child(icon.size_4().text_color(theme.muted_foreground)),
      )
      .child(
        div()
          .text_xs()
          .text_color(theme.muted_foreground)
          .child(label.into()),
      )
  }

  fn render_profile_info_row(
    &self,
    icon: Icon,
    label: impl Into<SharedString>,
    cx: &mut Context<Self>,
  ) -> gpui::Div {
    let theme = cx.theme().clone();

    h_flex()
      .w_full()
      .items_center()
      .gap_2()
      .min_w_0()
      .text_sm()
      .text_color(theme.muted_foreground)
      .child(icon.size_3p5().flex_shrink_0())
      .child(
        div()
          .min_w_0()
          .flex_1()
          .overflow_hidden()
          .text_ellipsis()
          .child(label.into()),
      )
  }

  fn render_sidebar(&self, profile: &GithubUserProfile, cx: &mut Context<Self>) -> gpui::Div {
    let theme = cx.theme().clone();
    let display_name = display_profile_name(profile);
    let bio = display_optional_text(&profile.bio);
    let company = display_optional_text(&profile.company);
    let location = display_optional_text(&profile.location);
    let website = display_optional_text(&profile.website_url);
    let twitter = display_optional_text(&profile.twitter_username);

    v_flex()
      .w(px(PROFILE_SIDEBAR_WIDTH))
      .flex_shrink_0()
      .py_6()
      .gap_4()
      .child(
        Avatar::new()
          .name(display_name.clone())
          .when_some(profile.avatar_url.clone(), |this, url| this.src(url))
          .large(),
      )
      .child(
        v_flex()
          .child(div().text_xl().font_semibold().child(display_name))
          .child(
            div()
              .text_sm()
              .text_color(theme.muted_foreground)
              .child(format!("@{}", profile.login)),
          ),
      )
      .when_some(bio, |this, bio| {
        this.child(div().text_sm().line_height(px(20.0)).child(bio))
      })
      .child(
        h_flex()
          .items_center()
          .gap_2()
          .text_sm()
          .child(
            Icon::new(IconName::User)
              .size_3p5()
              .text_color(theme.muted_foreground),
          )
          .child(format!(
            "{} followers",
            number_format::format_compact_number(profile.followers_count)
          ))
          .child(div().text_color(theme.muted_foreground).child("-"))
          .child(format!(
            "{} following",
            number_format::format_compact_number(profile.following_count)
          )),
      )
      .when_some(company, |this, company| {
        this.child(self.render_profile_info_row(Icon::new(IconName::Building2), company, cx))
      })
      .when_some(location, |this, location| {
        this.child(self.render_profile_info_row(Icon::new(UiIconName::Pin), location, cx))
      })
      .when_some(website, |this, website| {
        let label = strip_url_scheme(&website);
        let url = if website.starts_with("http://") || website.starts_with("https://") {
          website.clone()
        } else {
          format!("https://{website}")
        };
        this.child(
          h_flex()
            .id("github-profile-website")
            .w_full()
            .items_center()
            .gap_2()
            .min_w_0()
            .text_sm()
            .text_color(theme.link)
            .cursor_pointer()
            .hover(|this| this.opacity(0.8))
            .on_click(move |_, _, cx| {
              cx.open_url(&url);
            })
            .child(Icon::new(IconName::ExternalLink).size_3p5().flex_shrink_0())
            .child(
              div()
                .min_w_0()
                .flex_1()
                .overflow_hidden()
                .text_ellipsis()
                .child(label),
            ),
        )
      })
      .when_some(twitter, |this, twitter| {
        let handle = twitter.trim_start_matches('@').to_string();
        let url = format!("https://twitter.com/{handle}");
        this.child(
          h_flex()
            .id("github-profile-twitter")
            .w_full()
            .items_center()
            .gap_2()
            .min_w_0()
            .text_sm()
            .text_color(theme.link)
            .cursor_pointer()
            .hover(|this| this.opacity(0.8))
            .on_click(move |_, _, cx| {
              cx.open_url(&url);
            })
            .child(Icon::new(UiIconName::BrandX).size_3p5().flex_shrink_0())
            .child(
              div()
                .min_w_0()
                .flex_1()
                .overflow_hidden()
                .text_ellipsis()
                .child(format!("@{handle}")),
            ),
        )
      })
      .when(!profile.languages.is_empty(), {
        let languages = profile.languages.clone();
        let theme = theme.clone();
        move |this| this.child(github_shared::render_languages_section(&languages, &theme))
      })
  }

  fn render_repository_row(
    &self,
    repository: &GithubUserProfileRepository,
    cx: &mut Context<Self>,
  ) -> gpui::Stateful<gpui::Div> {
    let theme = cx.theme().clone();
    let owner = repository.owner.clone();
    let repo = repository.repo.clone();
    let description = display_optional_text(&repository.description);
    let updated_at = crate::date_format::format_relative_time(&repository.updated_at);
    let language = repository.language.clone();
    let language_color = repository_language_color(repository, &theme);
    let hover_bg = theme.accent.opacity(0.55);

    h_flex()
      .id(format!("github-profile-repo-{}", repository.full_name))
      .w_full()
      .items_center()
      .gap_3()
      .p_3()
      .cursor_pointer()
      .hover(move |this| this.bg(hover_bg))
      .on_click(move |_, _, cx| {
        open_repo_target(owner.clone(), repo.clone(), None, None, None, cx);
      })
      .child(
        Icon::new(IconName::Folder)
          .size_4()
          .text_color(theme.muted_foreground)
          .flex_shrink_0(),
      )
      .child(
        v_flex()
          .min_w_0()
          .flex_1()
          .gap_1()
          .child(
            h_flex()
              .w_full()
              .items_center()
              .justify_between()
              .gap_2()
              .child(
                div()
                  .min_w_0()
                  .flex_1()
                  .overflow_hidden()
                  .text_ellipsis()
                  .font_medium()
                  .child(repository.full_name.clone()),
              )
              .child(
                h_flex()
                  .items_center()
                  .gap_1()
                  .flex_shrink_0()
                  .when(repository.private, |this| {
                    this.child(Tag::secondary().small().child("Private"))
                  })
                  .when(repository.fork, |this| {
                    this.child(Tag::secondary().small().child("Fork"))
                  })
                  .when(repository.archived, |this| {
                    this.child(Tag::secondary().small().child("Archived"))
                  }),
              ),
          )
          .when_some(description, |this, description| {
            this.child(
              div()
                .text_sm()
                .text_color(theme.muted_foreground)
                .overflow_hidden()
                .text_ellipsis()
                .child(description),
            )
          })
          .child(
            h_flex()
              .items_center()
              .gap_3()
              .text_xs()
              .text_color(theme.muted_foreground)
              .when_some(language, |this, language| {
                this.child(
                  h_flex()
                    .items_center()
                    .gap_1()
                    .child(div().size(px(8.0)).rounded_full().bg(language_color))
                    .child(language),
                )
              })
              .child(
                h_flex()
                  .items_center()
                  .gap_1()
                  .child(Icon::new(UiIconName::Star).size_3())
                  .child(number_format::format_compact_number(
                    repository.stargazers_count,
                  )),
              )
              .child(
                h_flex()
                  .items_center()
                  .gap_1()
                  .child(Icon::new(UiIconName::GitFork).size_3())
                  .child(number_format::format_compact_number(repository.forks_count)),
              )
              .child(format!("Updated {updated_at}")),
          ),
      )
  }

  fn render_profile(&self, profile: &GithubUserProfile, cx: &mut Context<Self>) -> gpui::Div {
    let theme = cx.theme().clone();
    let repo_count = number_format::format_compact_number(profile.repositories_count);
    let stars_label = if profile.repositories_truncated {
      "Stars indexed"
    } else {
      "Stars"
    };
    let forks_label = if profile.repositories_truncated {
      "Forks indexed"
    } else {
      "Forks"
    };
    let repository_summary = if profile.repositories_truncated {
      format!(
        "Showing {} of {} repositories",
        number_format::format_compact_number(profile.repositories_indexed_count),
        repo_count
      )
    } else {
      format!("{repo_count} repositories")
    };

    h_flex()
      .w_full()
      .h_full()
      .min_h_0()
      .max_w(px(PROFILE_CONTAINER_MAX_WIDTH))
      .mx_auto()
      .items_start()
      .gap_8()
      .px_6()
      .child(self.render_sidebar(profile, cx))
      .child(
        v_flex()
          .id("github-profile-content-scroll")
          .min_w_0()
          .flex_1()
          .h_full()
          .min_h_0()
          .overflow_y_scroll()
          .py_6()
          .gap_4()
          .child(
            h_flex()
              .w_full()
              .gap_3()
              .flex_wrap()
              .child(self.render_stat("Repositories", repo_count, Icon::new(IconName::Folder), cx))
              .child(self.render_stat(
                stars_label,
                number_format::format_compact_number(profile.stargazers_count),
                Icon::new(UiIconName::Star),
                cx,
              ))
              .child(self.render_stat(
                forks_label,
                number_format::format_compact_number(profile.forks_count),
                Icon::new(UiIconName::GitFork),
                cx,
              )),
          )
          .child(
            v_flex()
              .w_full()
              .gap_2()
              .child(
                h_flex()
                  .items_center()
                  .justify_between()
                  .child(div().text_sm().font_semibold().child("Repositories"))
                  .child(
                    div()
                      .text_xs()
                      .text_color(theme.muted_foreground)
                      .child(repository_summary),
                  ),
              )
              .child(
                v_flex()
                  .w_full()
                  .border_1()
                  .border_color(theme.border)
                  .rounded(theme.radius)
                  .overflow_hidden()
                  .when(profile.repositories.is_empty(), |this| {
                    this.child(
                      div()
                        .p_4()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child("No repositories found."),
                    )
                  })
                  .children(
                    profile
                      .repositories
                      .iter()
                      .enumerate()
                      .map(|(ix, repository)| {
                        let is_last = ix + 1 == profile.repositories.len();
                        self
                          .render_repository_row(repository, cx)
                          .when(!is_last, |this| {
                            this.border_b_1().border_color(theme.border)
                          })
                      }),
                  ),
              ),
          ),
      )
  }
}

impl Render for GithubProfilePage {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    self.sync_route(cx);
    let theme = cx.theme().clone();

    let content = if self.profile_loading && self.profile.is_none() {
      self.render_loading(cx).into_any_element()
    } else if let Some(error) = self.profile_error.as_ref() {
      v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.status_red())
        .child(error.clone())
        .into_any_element()
    } else if let Some(profile) = self.profile.as_ref() {
      self.render_profile(profile, cx).into_any_element()
    } else {
      v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.muted_foreground)
        .child("No GitHub profile selected.")
        .into_any_element()
    };

    div()
      .size_full()
      .flex()
      .flex_col()
      .bg(theme.background)
      .track_focus(&self.focus_handle(cx))
      .on_action(cx.listener(GithubProfilePage::show_command_palette_action))
      .child(self.render_header(cx))
      .child(div().w_full().flex_1().min_h_0().child(content))
  }
}

impl Focusable for GithubProfilePage {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

#[cfg(test)]
mod tests {
  use super::{profile_github_url, profile_login_from_pathname};

  #[test]
  fn profile_login_from_pathname_matches_single_login_route() {
    assert_eq!(
      profile_login_from_pathname("/github/octocat"),
      Some("octocat".to_string())
    );
    assert_eq!(profile_login_from_pathname("/github/acme/reviu"), None);
    assert_eq!(profile_login_from_pathname("/github"), None);
  }

  #[test]
  fn profile_github_url_prefers_api_html_url() {
    assert_eq!(
      profile_github_url("octocat", Some("https://github.com/the-octocat")),
      "https://github.com/the-octocat"
    );
  }

  #[test]
  fn profile_github_url_falls_back_to_login() {
    assert_eq!(
      profile_github_url("octocat", None),
      "https://github.com/octocat"
    );
    assert_eq!(
      profile_github_url("octocat", Some("  ")),
      "https://github.com/octocat"
    );
  }
}
