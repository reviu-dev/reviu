const APP_PROFILE_ENV: &str = "REVIU_PROFILE";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppProfile {
  Prod,
  Dev,
}

impl Default for AppProfile {
  fn default() -> Self {
    if cfg!(debug_assertions) {
      Self::Dev
    } else {
      Self::Prod
    }
  }
}

impl AppProfile {
  pub fn current() -> Self {
    resolve_app_profile(
      std::env::var(APP_PROFILE_ENV).ok().as_deref(),
      cfg!(debug_assertions),
    )
  }

  pub fn is_dev(self) -> bool {
    matches!(self, Self::Dev)
  }

  pub fn storage_dir_name(self) -> &'static str {
    match self {
      Self::Prod => "reviu",
      Self::Dev => "reviu.dev",
    }
  }

  pub fn keychain_service(self) -> &'static str {
    match self {
      Self::Prod => "reviu_auth",
      Self::Dev => "reviu_auth.dev",
    }
  }

  pub fn url_scheme(self) -> &'static str {
    match self {
      Self::Prod => "reviu",
      Self::Dev => "reviu-dev",
    }
  }

  pub fn header_tag_label(self) -> Option<&'static str> {
    match self {
      Self::Prod => None,
      Self::Dev => Some("DEV"),
    }
  }
}

fn resolve_app_profile(env_value: Option<&str>, debug_build: bool) -> AppProfile {
  match env_value.map(|value| value.trim().to_ascii_lowercase()) {
    Some(value) if matches!(value.as_str(), "prod" | "production") => AppProfile::Prod,
    Some(value) if matches!(value.as_str(), "dev" | "development") => AppProfile::Dev,
    _ if debug_build => AppProfile::Dev,
    _ => AppProfile::Prod,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn resolve_app_profile_prefers_explicit_prod_value() {
    assert_eq!(resolve_app_profile(Some("prod"), true), AppProfile::Prod);
    assert_eq!(
      resolve_app_profile(Some("production"), false),
      AppProfile::Prod
    );
  }

  #[test]
  fn resolve_app_profile_prefers_explicit_dev_value() {
    assert_eq!(resolve_app_profile(Some("dev"), false), AppProfile::Dev);
    assert_eq!(
      resolve_app_profile(Some("development"), false),
      AppProfile::Dev
    );
  }

  #[test]
  fn resolve_app_profile_falls_back_to_build_mode() {
    assert_eq!(resolve_app_profile(None, true), AppProfile::Dev);
    assert_eq!(resolve_app_profile(None, false), AppProfile::Prod);
    assert_eq!(resolve_app_profile(Some("unknown"), true), AppProfile::Dev);
    assert_eq!(
      resolve_app_profile(Some("unknown"), false),
      AppProfile::Prod
    );
  }

  #[test]
  fn app_profile_namespaces_match_expected_values() {
    assert_eq!(AppProfile::Prod.storage_dir_name(), "reviu");
    assert_eq!(AppProfile::Dev.storage_dir_name(), "reviu.dev");
    assert_eq!(AppProfile::Prod.keychain_service(), "reviu_auth");
    assert_eq!(AppProfile::Dev.keychain_service(), "reviu_auth.dev");
    assert_eq!(AppProfile::Prod.url_scheme(), "reviu");
    assert_eq!(AppProfile::Dev.url_scheme(), "reviu-dev");
    assert_eq!(AppProfile::Prod.header_tag_label(), None);
    assert_eq!(AppProfile::Dev.header_tag_label(), Some("DEV"));
  }
}
