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

pub fn resolved_build_version(build_version: &str) -> String {
  normalize_semver(build_version).unwrap_or_else(|| build_version.to_string())
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
  use std::path::PathBuf;

  #[test]
  fn resolved_build_version_uses_build_version_only() {
    assert_eq!(resolved_build_version("0.1.0"), "0.1.0");
    assert_eq!(resolved_build_version("v0.1.0"), "0.1.0");
    assert_eq!(resolved_build_version("invalid"), "invalid");
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

      let ready = ReadyToInstallAppUpdate {
        update: update.clone(),
        artifact_path: PathBuf::from("/tmp/reviu-installer.dmg"),
      };
      AppUpdateStore::set_ready_to_install(cx, ready.clone());
      assert_eq!(AppUpdateStore::try_ready_to_install(cx), Some(ready));

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
