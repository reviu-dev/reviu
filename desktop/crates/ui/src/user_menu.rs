use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::{AnyElement, App, IntoElement, ParentElement, SharedString, Styled, Window};
use gpui_component::{
  Icon, IconName, Sizable as _,
  avatar::Avatar,
  button::{Button, ButtonVariants as _},
  menu::{DropdownMenu, PopupMenu, PopupMenuItem},
};

use crate::{UiIconName, file_icon_path_for_name};

fn git_config_icon() -> Icon {
  file_icon_path_for_name(".gitconfig").map_or_else(
    || Icon::new(IconName::File),
    |path| Icon::empty().path(path),
  )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserMenuPage {
  Git,
  Github,
  GithubPrDetails,
  GitConfig,
  Settings,
}

#[derive(Clone, Debug)]
pub struct UserMenuUser {
  pub name: SharedString,
  pub email: SharedString,
  pub image: Option<SharedString>,
}

#[derive(Clone, Debug)]
pub enum UserMenuState {
  Unknown,
  Authenticated(UserMenuUser),
  Unauthenticated,
}

pub struct UserMenuConfig {
  pub id: SharedString,
  pub state: UserMenuState,
  pub current_page: UserMenuPage,
  pub on_open_git: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
  pub on_open_github: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
  pub on_open_git_config: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
  pub on_open_settings: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
  pub on_sign_in: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
  pub on_sign_out: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
}

pub fn user_menu(config: UserMenuConfig) -> Option<AnyElement> {
  match config.state {
    UserMenuState::Unknown => None,
    UserMenuState::Unauthenticated => {
      let handler = config.on_sign_in?;
      Some(
        Button::new(config.id)
          .icon(IconName::GitHub)
          .label("Sign in with GitHub")
          .ghost()
          .gap_2()
          .small()
          .on_click(move |_, window, cx| {
            handler(window, cx);
          })
          .into_any_element(),
      )
    }
    UserMenuState::Authenticated(user) => {
      let avatar = Avatar::new()
        .name(user.name.clone())
        .when_some(user.image.clone(), |this, image| this.src(image))
        .small();
      let user_email = user.email.clone();
      let current_page = config.current_page;
      let on_open_git = config.on_open_git.clone();
      let on_open_github = config.on_open_github.clone();
      let on_open_git_config = config.on_open_git_config.clone();
      let on_open_settings = config.on_open_settings.clone();
      let on_sign_out = config.on_sign_out.clone();

      Some(
        Button::new(config.id)
          .ghost()
          .compact()
          .child(avatar)
          .dropdown_menu_with_anchor(gpui::Corner::TopRight, move |menu: PopupMenu, _, _| {
            let mut menu = menu.item(
              PopupMenuItem::new(user_email.clone())
                .icon(IconName::User)
                .disabled(true),
            );
            menu = menu.separator();

            if current_page != UserMenuPage::Git {
              if let Some(handler) = on_open_git.clone() {
                menu = menu.item(
                  PopupMenuItem::new("Git")
                    .icon(Icon::new(UiIconName::GitBranch))
                    .on_click(move |_, window, cx| {
                      handler(window, cx);
                    }),
                );
              }
            }

            if current_page != UserMenuPage::Github {
              if let Some(handler) = on_open_github.clone() {
                menu = menu.item(
                  PopupMenuItem::new("GitHub")
                    .icon(IconName::GitHub)
                    .on_click(move |_, window, cx| {
                      handler(window, cx);
                    }),
                );
              }
            }

            if current_page != UserMenuPage::Settings {
              if let Some(handler) = on_open_settings.clone() {
                menu = menu.item(
                  PopupMenuItem::new("Settings")
                    .icon(IconName::Settings2)
                    .on_click(move |_, window, cx| {
                      handler(window, cx);
                    }),
                );
              }
            }

            if current_page != UserMenuPage::GitConfig {
              if let Some(handler) = on_open_git_config.clone() {
                menu = menu.item(
                  PopupMenuItem::new("Git Config")
                    .icon(git_config_icon())
                    .on_click(move |_, window, cx| {
                      handler(window, cx);
                    }),
                );
              }
            }

            if let Some(handler) = on_sign_out.clone() {
              menu = menu.separator().item(
                PopupMenuItem::new("Sign out")
                  .icon(IconName::ArrowRight)
                  .on_click(move |_, window, cx| {
                    handler(window, cx);
                  }),
              );
            }

            menu
          })
          .into_any_element(),
      )
    }
  }
}
