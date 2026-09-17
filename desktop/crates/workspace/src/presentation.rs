use std::path::Path;

use gpui::App;

use crate::AppProfile;

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Debug, Default)]
pub struct DriverPresentation {
  pub hide_profile_badge: bool,
  pub project_avatars: std::collections::HashMap<std::path::PathBuf, String>,
  pub user_avatar: Option<String>,
}

#[cfg(any(test, feature = "test-support"))]
impl gpui::Global for DriverPresentation {}

pub(crate) fn profile_badge_label(profile: AppProfile, _cx: &App) -> Option<&'static str> {
  #[cfg(any(test, feature = "test-support"))]
  if _cx
    .try_global::<DriverPresentation>()
    .is_some_and(|presentation| presentation.hide_profile_badge)
  {
    return None;
  }
  profile.header_tag_label()
}

pub(crate) fn project_avatar(_root: &Path, fallback: Option<String>, _cx: &App) -> Option<String> {
  #[cfg(any(test, feature = "test-support"))]
  if let Some(avatar) = _cx
    .try_global::<DriverPresentation>()
    .and_then(|presentation| presentation.project_avatars.get(_root))
  {
    return Some(avatar.clone());
  }
  fallback
}

pub(crate) fn user_avatar(fallback: Option<String>, _cx: &App) -> Option<String> {
  #[cfg(any(test, feature = "test-support"))]
  if let Some(avatar) = _cx
    .try_global::<DriverPresentation>()
    .and_then(|presentation| presentation.user_avatar.as_ref())
  {
    return Some(avatar.clone());
  }
  fallback
}

#[cfg(test)]
mod tests {
  use super::*;
  use gpui::TestAppContext;

  #[gpui::test]
  fn normal_presentation_is_unchanged(cx: &mut TestAppContext) {
    cx.update(|cx| {
      assert_eq!(profile_badge_label(AppProfile::Dev, cx), Some("DEV"));
      assert_eq!(profile_badge_label(AppProfile::Prod, cx), None);
      let avatar = Some("https://example.com/avatar.png".to_string());
      assert_eq!(
        project_avatar(Path::new("/repo"), avatar.clone(), cx),
        avatar
      );
      assert_eq!(user_avatar(avatar.clone(), cx), avatar);
      assert_eq!(project_avatar(Path::new("/repo"), None, cx), None);
      assert_eq!(user_avatar(None, cx), None);
    });
  }

  #[gpui::test]
  fn overrides_are_scoped_to_the_configured_images(cx: &mut TestAppContext) {
    cx.update(|cx| {
      cx.set_global(DriverPresentation {
        hide_profile_badge: true,
        project_avatars: [("/repo".into(), "repo-image".to_string())].into(),
        user_avatar: Some("user-image".into()),
      });
      assert_eq!(profile_badge_label(AppProfile::Dev, cx), None);
      assert_eq!(AppProfile::Dev.storage_dir_name(), "reviu.dev");
      assert_eq!(AppProfile::Dev.keychain_service(), "reviu_auth.dev");
      assert_eq!(
        project_avatar(Path::new("/repo"), None, cx).as_deref(),
        Some("repo-image")
      );
      assert_eq!(project_avatar(Path::new("/other"), None, cx), None);
      let fallback = Some("original".to_string());
      assert_eq!(
        project_avatar(Path::new("/other"), fallback.clone(), cx),
        fallback
      );
      assert_eq!(user_avatar(None, cx).as_deref(), Some("user-image"));
      cx.set_global(DriverPresentation::default());
      assert_eq!(profile_badge_label(AppProfile::Dev, cx), Some("DEV"));
      assert_eq!(project_avatar(Path::new("/repo"), None, cx), None);
      assert_eq!(user_avatar(None, cx), None);
    });
  }
}
