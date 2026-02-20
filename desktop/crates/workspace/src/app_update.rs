use std::{
  fs::{self, File},
  io::Read,
  path::{Path, PathBuf},
  sync::{Arc, Mutex},
};

use anyhow::{Context as _, Result, bail};
use dirs::config_dir;
use gpui::{App, Global};
use reqwest::blocking::Client;
use sha2::{Digest, Sha256};

use crate::config::ConfigStore;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateArtifact {
  pub url: String,
  pub sha256: String,
  pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AvailableAppUpdate {
  pub latest_version: String,
  pub minimum_supported_version: String,
  pub release_notes_url: String,
  pub force_update: bool,
  pub artifact: UpdateArtifact,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadyToInstallAppUpdate {
  pub update: AvailableAppUpdate,
  pub artifact_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppUpdateState {
  Available(AvailableAppUpdate),
  Downloading(AvailableAppUpdate),
  ReadyToInstall(ReadyToInstallAppUpdate),
  Error {
    update: Option<AvailableAppUpdate>,
    message: String,
  },
}

#[derive(Clone, Copy, Debug, Default)]
pub struct AppUpdateNotificationId;

#[derive(Clone, Default)]
struct AppUpdateStoreState {
  state: Option<AppUpdateState>,
}

#[derive(Clone)]
pub struct AppUpdateStore {
  state: Arc<Mutex<AppUpdateStoreState>>,
}

impl Default for AppUpdateStore {
  fn default() -> Self {
    Self {
      state: Arc::new(Mutex::new(AppUpdateStoreState::default())),
    }
  }
}

impl Global for AppUpdateStore {}

impl AppUpdateStore {
  pub fn try_state(cx: &App) -> Option<AppUpdateState> {
    let store = cx.try_global::<Self>()?;
    store.state.lock().ok().and_then(|state| state.state.clone())
  }

  pub fn set_state(cx: &mut App, state: Option<AppUpdateState>) {
    let Some(store) = cx.try_global::<Self>() else {
      return;
    };

    if let Ok(mut inner) = store.state.lock() {
      inner.state = state;
    }

    cx.refresh_windows();
  }

  pub fn set_available_update(cx: &mut App, update: Option<AvailableAppUpdate>) {
    match update {
      Some(update) => Self::set_state(cx, Some(AppUpdateState::Available(update))),
      None => Self::set_state(cx, None),
    }
  }

  pub fn clear_available_update(cx: &mut App) {
    Self::set_state(cx, None);
  }

  pub fn try_available_update(cx: &App) -> Option<AvailableAppUpdate> {
    match Self::try_state(cx) {
      Some(AppUpdateState::Available(update)) => Some(update),
      Some(AppUpdateState::Downloading(update)) => Some(update),
      Some(AppUpdateState::ReadyToInstall(ready)) => Some(ready.update),
      Some(AppUpdateState::Error {
        update: Some(update),
        ..
      }) => Some(update),
      _ => None,
    }
  }

  pub fn try_ready_to_install(cx: &App) -> Option<ReadyToInstallAppUpdate> {
    match Self::try_state(cx) {
      Some(AppUpdateState::ReadyToInstall(ready)) => Some(ready),
      _ => None,
    }
  }

  pub fn is_downloading(cx: &App) -> bool {
    matches!(Self::try_state(cx), Some(AppUpdateState::Downloading(_)))
  }

  pub fn set_downloading(cx: &mut App, update: AvailableAppUpdate) {
    Self::set_state(cx, Some(AppUpdateState::Downloading(update)));
  }

  pub fn set_ready_to_install(cx: &mut App, ready: ReadyToInstallAppUpdate) {
    Self::set_state(cx, Some(AppUpdateState::ReadyToInstall(ready)));
  }

  pub fn set_error(
    cx: &mut App,
    update: Option<AvailableAppUpdate>,
    message: impl Into<String>,
  ) {
    Self::set_state(
      cx,
      Some(AppUpdateState::Error {
        update,
        message: message.into(),
      }),
    );
  }

  pub fn mark_install_started(cx: &mut App, update: &AvailableAppUpdate) {
    ConfigStore::persist_simulated_app_version(Some(&update.latest_version));
    Self::clear_available_update(cx);
  }
}

pub fn normalize_semver(value: &str) -> Option<String> {
  let normalized = value.trim().strip_prefix('v').unwrap_or(value.trim());
  let mut parts = normalized.split('.');
  let major: u64 = parts.next()?.parse().ok()?;
  let minor: u64 = parts.next()?.parse().ok()?;
  let patch: u64 = parts.next()?.parse().ok()?;
  if parts.next().is_some() {
    return None;
  }
  Some(format!("{major}.{minor}.{patch}"))
}

fn parse_semver_tuple(value: &str) -> Option<(u64, u64, u64)> {
  let normalized = normalize_semver(value)?;
  let mut parts = normalized.split('.');
  let major: u64 = parts.next()?.parse().ok()?;
  let minor: u64 = parts.next()?.parse().ok()?;
  let patch: u64 = parts.next()?.parse().ok()?;
  Some((major, minor, patch))
}

pub fn effective_current_version(base_version: &str, simulated_version: Option<&str>) -> String {
  let normalized_base = normalize_semver(base_version).unwrap_or_else(|| base_version.to_string());
  let Some(simulated_version) = simulated_version else {
    return normalized_base;
  };

  let Some(base_tuple) = parse_semver_tuple(&normalized_base) else {
    return normalized_base;
  };
  let Some(simulated_tuple) = parse_semver_tuple(simulated_version) else {
    return normalized_base;
  };

  if simulated_tuple > base_tuple {
    normalize_semver(simulated_version).unwrap_or(normalized_base)
  } else {
    normalized_base
  }
}

pub fn resolve_effective_current_version(base_version: &str) -> String {
  let simulated_version = ConfigStore::load_simulated_app_version();
  let normalized_base = normalize_semver(base_version).unwrap_or_else(|| base_version.to_string());
  let effective = effective_current_version(&normalized_base, simulated_version.as_deref());

  let should_clear_simulated = match simulated_version.as_deref() {
    Some(version) => {
      let base_tuple = parse_semver_tuple(&normalized_base);
      let simulated_tuple = parse_semver_tuple(version);
      match (base_tuple, simulated_tuple) {
        (Some(base), Some(simulated)) => simulated <= base,
        (_, None) => true,
        _ => false,
      }
    }
    None => false,
  };

  if should_clear_simulated {
    ConfigStore::persist_simulated_app_version(None);
  }

  effective
}

pub fn current_platform() -> &'static str {
  match std::env::consts::OS {
    "macos" => "macos",
    "linux" => "linux",
    "windows" => "windows",
    other => other,
  }
}

pub fn current_arch() -> &'static str {
  match std::env::consts::ARCH {
    "aarch64" => "aarch64",
    "x86_64" => "x86_64",
    other => other,
  }
}

pub fn download_update_artifact(update: &AvailableAppUpdate) -> Result<ReadyToInstallAppUpdate> {
  let updates_dir = updates_directory()?;
  fs::create_dir_all(&updates_dir)
    .with_context(|| format!("failed to create updates directory {}", updates_dir.display()))?;

  let file_name = artifact_file_name(update);
  let artifact_path = updates_dir.join(file_name);
  let temp_path = artifact_path.with_extension("part");

  let client = Client::new();
  let mut response = client
    .get(&update.artifact.url)
    .send()
    .with_context(|| format!("failed to download update artifact from {}", update.artifact.url))?;

  if !response.status().is_success() {
    bail!("update artifact download failed with status {}", response.status());
  }

  let mut temp_file = File::create(&temp_path)
    .with_context(|| format!("failed to create temp artifact {}", temp_path.display()))?;
  let downloaded_size = response
    .copy_to(&mut temp_file)
    .with_context(|| format!("failed while writing {}", temp_path.display()))?;

  if downloaded_size != update.artifact.size {
    let _ = fs::remove_file(&temp_path);
    bail!(
      "update artifact size mismatch: expected {} bytes, got {} bytes",
      update.artifact.size,
      downloaded_size,
    );
  }

  let downloaded_sha256 = sha256_hex(&temp_path)?;
  if downloaded_sha256 != update.artifact.sha256.to_lowercase() {
    let _ = fs::remove_file(&temp_path);
    bail!("update artifact checksum mismatch");
  }

  if artifact_path.exists() {
    let _ = fs::remove_file(&artifact_path);
  }

  fs::rename(&temp_path, &artifact_path).with_context(|| {
    format!(
      "failed to move update artifact into place {}",
      artifact_path.display()
    )
  })?;

  Ok(ReadyToInstallAppUpdate {
    update: update.clone(),
    artifact_path,
  })
}

pub fn open_installer(artifact_path: &Path) -> Result<()> {
  #[cfg(target_os = "macos")]
  {
    let status = std::process::Command::new("open")
      .arg(artifact_path)
      .status()
      .with_context(|| format!("failed to launch installer {}", artifact_path.display()))?;

    if !status.success() {
      bail!("failed to launch installer {}", artifact_path.display());
    }

    return Ok(());
  }

  #[cfg(not(target_os = "macos"))]
  {
    let _ = artifact_path;
    bail!("installer launch is currently supported on macOS only")
  }
}

fn updates_directory() -> Result<PathBuf> {
  let base = config_dir().context("missing system config directory")?;
  Ok(base.join("reviu").join("updates"))
}

fn artifact_file_name(update: &AvailableAppUpdate) -> String {
  let from_url = update
    .artifact
    .url
    .rsplit('/')
    .next()
    .and_then(|segment| segment.split('?').next())
    .unwrap_or_default()
    .trim();

  let fallback = format!("reviu-{}-update.bin", update.latest_version);
  let raw = if from_url.is_empty() {
    fallback
  } else {
    from_url.to_string()
  };

  raw
    .chars()
    .map(|character| {
      if character.is_ascii_alphanumeric() || character == '.' || character == '_' || character == '-' {
        character
      } else {
        '_'
      }
    })
    .collect()
}

fn sha256_hex(path: &Path) -> Result<String> {
  let mut file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
  let mut hasher = Sha256::new();
  let mut buffer = [0_u8; 16 * 1024];

  loop {
    let bytes_read = file
      .read(&mut buffer)
      .with_context(|| format!("failed to read {}", path.display()))?;
    if bytes_read == 0 {
      break;
    }
    hasher.update(&buffer[..bytes_read]);
  }

  let digest = hasher.finalize();
  Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(test)]
mod tests {
  use super::*;
  use gpui::TestAppContext;
  use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
  };

  fn unique_test_db_path(label: &str) -> PathBuf {
    static NEXT_DB_ID: AtomicU64 = AtomicU64::new(1);
    let id = NEXT_DB_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
      "reviu-app-update-{label}-{}-{id}.sqlite",
      std::process::id()
    ))
  }

  #[test]
  fn effective_current_version_uses_simulated_only_when_newer() {
    assert_eq!(
      effective_current_version("0.1.0", Some("0.2.0")),
      "0.2.0".to_string()
    );
    assert_eq!(
      effective_current_version("0.2.0", Some("0.1.0")),
      "0.2.0".to_string()
    );
    assert_eq!(
      effective_current_version("v0.1.0", Some("v0.1.0")),
      "0.1.0".to_string()
    );
    assert_eq!(
      effective_current_version("0.1.0", Some("invalid")),
      "0.1.0".to_string()
    );
  }

  #[test]
  fn resolve_effective_current_version_clears_stale_simulated_version() {
    let db_path = unique_test_db_path("resolve-version-stale");
    let _ = fs::remove_file(&db_path);
    ConfigStore::set_test_db_path(Some(db_path));

    ConfigStore::persist_simulated_app_version(Some("0.1.0"));
    let effective = resolve_effective_current_version("0.1.1");

    assert_eq!(effective, "0.1.1".to_string());
    assert_eq!(ConfigStore::load_simulated_app_version(), None);

    ConfigStore::set_test_db_path(None);
  }

  #[test]
  fn resolve_effective_current_version_keeps_newer_simulated_version() {
    let db_path = unique_test_db_path("resolve-version-newer");
    let _ = fs::remove_file(&db_path);
    ConfigStore::set_test_db_path(Some(db_path));

    ConfigStore::persist_simulated_app_version(Some("0.3.0"));
    let effective = resolve_effective_current_version("0.2.0");

    assert_eq!(effective, "0.3.0".to_string());
    assert_eq!(
      ConfigStore::load_simulated_app_version(),
      Some("0.3.0".to_string())
    );

    ConfigStore::set_test_db_path(None);
  }

  #[gpui::test]
  fn app_update_store_state_machine_transitions(cx: &mut TestAppContext) {
    cx.update(|cx| {
      cx.set_global(AppUpdateStore::default());

      let update = AvailableAppUpdate {
        latest_version: "0.2.0".to_string(),
        minimum_supported_version: "0.1.0".to_string(),
        release_notes_url: "https://reviu.dev/releases/0.2.0".to_string(),
        force_update: false,
        artifact: UpdateArtifact {
          url: "https://reviu.dev/downloads/reviu-0.2.0.dmg".to_string(),
          sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            .to_string(),
          size: 123,
        },
      };

      AppUpdateStore::set_available_update(cx, Some(update.clone()));
      assert!(matches!(
        AppUpdateStore::try_state(cx),
        Some(AppUpdateState::Available(_))
      ));

      AppUpdateStore::set_downloading(cx, update.clone());
      assert!(AppUpdateStore::is_downloading(cx));

      AppUpdateStore::set_error(cx, Some(update.clone()), "checksum mismatch");
      assert!(matches!(
        AppUpdateStore::try_state(cx),
        Some(AppUpdateState::Error { .. })
      ));

      AppUpdateStore::clear_available_update(cx);
      assert!(AppUpdateStore::try_state(cx).is_none());
    });
  }
}
