use std::{
  ffi::OsString,
  fs::{self, File},
  io::Read,
  path::{Path, PathBuf},
  process::Command,
  sync::{Arc, Mutex},
};

use anyhow::{Context as _, Result, bail};
use dirs::config_dir;
use gpui::{App, Global};
use reqwest::blocking::Client;
use sha2::{Digest, Sha256};

use crate::AppProfile;

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
    store
      .state
      .lock()
      .ok()
      .and_then(|state| state.state.clone())
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

  pub fn set_error(cx: &mut App, update: Option<AvailableAppUpdate>, message: impl Into<String>) {
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
  fs::create_dir_all(&updates_dir).with_context(|| {
    format!(
      "failed to create updates directory {}",
      updates_dir.display()
    )
  })?;

  let file_name = artifact_file_name(update);
  let artifact_path = updates_dir.join(file_name);
  let temp_path = artifact_path.with_extension("part");

  let client = Client::new();
  let mut response = client.get(&update.artifact.url).send().with_context(|| {
    format!(
      "failed to download update artifact from {}",
      update.artifact.url
    )
  })?;

  if !response.status().is_success() {
    bail!(
      "update artifact download failed with status {}",
      response.status()
    );
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

#[cfg(not(target_os = "macos"))]
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

    Ok(())
  }

  #[cfg(not(target_os = "macos"))]
  {
    let _ = artifact_path;
    bail!("installer launch is currently supported on macOS only")
  }
}

pub fn install_update_artifact(ready: &ReadyToInstallAppUpdate) -> Result<()> {
  #[cfg(target_os = "macos")]
  {
    install_update_macos(&ready.artifact_path)
  }

  #[cfg(not(target_os = "macos"))]
  {
    open_installer(&ready.artifact_path)
  }
}

#[cfg(target_os = "macos")]
fn install_update_macos(artifact_path: &Path) -> Result<()> {
  let installed_app_path = current_installed_app_bundle_path()?;
  let mount_point = macos_update_mount_point(&ready_version_label(artifact_path));
  if mount_point.exists() {
    let _ = fs::remove_dir_all(&mount_point);
  }
  fs::create_dir_all(&mount_point).with_context(|| {
    format!(
      "failed to create macOS update mount point {}",
      mount_point.display()
    )
  })?;

  let attach_status = Command::new("hdiutil")
    .arg("attach")
    .arg("-nobrowse")
    .arg("-readonly")
    .arg("-mountpoint")
    .arg(&mount_point)
    .arg(artifact_path)
    .status()
    .with_context(|| {
      format!(
        "failed to mount update artifact {}",
        artifact_path.display()
      )
    })?;

  if !attach_status.success() {
    bail!(
      "failed to mount update artifact {}",
      artifact_path.display()
    );
  }

  let _unmounter = MacOsUnmounter {
    mount_point: mount_point.clone(),
  };
  let mounted_app_path = find_mounted_app_bundle(&mount_point, &installed_app_path)?;
  rsync_app_bundle(&mounted_app_path, &installed_app_path)
}

#[cfg(target_os = "macos")]
struct MacOsUnmounter {
  mount_point: PathBuf,
}

#[cfg(target_os = "macos")]
impl Drop for MacOsUnmounter {
  fn drop(&mut self) {
    let _ = Command::new("hdiutil")
      .arg("detach")
      .arg("-force")
      .arg(&self.mount_point)
      .status();
    let _ = fs::remove_dir_all(&self.mount_point);
  }
}

#[cfg(target_os = "macos")]
fn current_installed_app_bundle_path() -> Result<PathBuf> {
  let current_exe = std::env::current_exe().context("failed to resolve current executable")?;
  app_bundle_path_from_executable_path(&current_exe).with_context(|| {
    format!(
      "auto update requires running from a bundled .app, current executable is {}",
      current_exe.display()
    )
  })
}

#[cfg(target_os = "macos")]
fn app_bundle_path_from_executable_path(executable_path: &Path) -> Option<PathBuf> {
  let macos_dir = executable_path.parent()?;
  if macos_dir.file_name()?.to_str()? != "MacOS" {
    return None;
  }

  let contents_dir = macos_dir.parent()?;
  if contents_dir.file_name()?.to_str()? != "Contents" {
    return None;
  }

  let app_bundle = contents_dir.parent()?;
  (app_bundle.extension()?.to_str()? == "app").then(|| app_bundle.to_path_buf())
}

#[cfg(target_os = "macos")]
fn find_mounted_app_bundle(mount_point: &Path, installed_app_path: &Path) -> Result<PathBuf> {
  if let Some(app_name) = installed_app_path.file_name() {
    let candidate = mount_point.join(app_name);
    if candidate.is_dir() {
      return Ok(candidate);
    }
  }

  let entries = fs::read_dir(mount_point)
    .with_context(|| format!("failed to inspect mounted update {}", mount_point.display()))?;
  for entry in entries.flatten() {
    let path = entry.path();
    if path.extension().and_then(|ext| ext.to_str()) == Some("app") {
      return Ok(path);
    }
  }

  bail!(
    "failed to find application bundle in mounted update {}",
    mount_point.display()
  )
}

#[cfg(target_os = "macos")]
fn rsync_app_bundle(source_app_path: &Path, installed_app_path: &Path) -> Result<()> {
  let status = Command::new("rsync")
    .arg("-a")
    .arg("--delete")
    .arg("--exclude")
    .arg("Icon?")
    .arg(path_with_trailing_slash(source_app_path))
    .arg(path_with_trailing_slash(installed_app_path))
    .status()
    .with_context(|| {
      format!(
        "failed to copy updated app bundle from {} to {}",
        source_app_path.display(),
        installed_app_path.display()
      )
    })?;

  if !status.success() {
    bail!(
      "failed to copy updated app bundle from {} to {}",
      source_app_path.display(),
      installed_app_path.display()
    );
  }

  Ok(())
}

#[cfg(target_os = "macos")]
fn path_with_trailing_slash(path: &Path) -> OsString {
  let mut value = path.as_os_str().to_os_string();
  value.push("/");
  value
}

#[cfg(target_os = "macos")]
fn macos_update_mount_point(label: &str) -> PathBuf {
  let sanitized_label = label
    .chars()
    .map(|ch| {
      if ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' || ch == '_' {
        ch
      } else {
        '_'
      }
    })
    .collect::<String>();
  std::env::temp_dir().join(format!(
    "reviu-update-mount-{}-{}",
    std::process::id(),
    sanitized_label
  ))
}

#[cfg(target_os = "macos")]
fn ready_version_label(artifact_path: &Path) -> String {
  artifact_path
    .file_stem()
    .and_then(|stem| stem.to_str())
    .filter(|stem| !stem.trim().is_empty())
    .unwrap_or("latest")
    .to_string()
}

fn updates_directory() -> Result<PathBuf> {
  let base = config_dir().context("missing system config directory")?;
  Ok(updates_directory_for_profile(base, AppProfile::current()))
}

fn updates_directory_for_profile(base: PathBuf, profile: AppProfile) -> PathBuf {
  base.join(profile.storage_dir_name()).join("updates")
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
      if character.is_ascii_alphanumeric()
        || character == '.'
        || character == '_'
        || character == '-'
      {
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
          sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
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

  #[test]
  fn updates_directory_uses_profile_namespace() {
    let base = PathBuf::from("/tmp/reviu-updates");

    assert_eq!(
      updates_directory_for_profile(base.clone(), AppProfile::Prod),
      PathBuf::from("/tmp/reviu-updates/reviu/updates")
    );
    assert_eq!(
      updates_directory_for_profile(base, AppProfile::Dev),
      PathBuf::from("/tmp/reviu-updates/reviu.dev/updates")
    );
  }

  #[cfg(target_os = "macos")]
  #[test]
  fn app_bundle_path_from_executable_path_returns_bundle_root() {
    assert_eq!(
      app_bundle_path_from_executable_path(Path::new(
        "/Applications/Reviu.app/Contents/MacOS/reviu"
      )),
      Some(PathBuf::from("/Applications/Reviu.app"))
    );
  }

  #[cfg(target_os = "macos")]
  #[test]
  fn app_bundle_path_from_executable_path_rejects_non_bundle_paths() {
    assert_eq!(
      app_bundle_path_from_executable_path(Path::new("/usr/local/bin/reviu")),
      None
    );
    assert_eq!(
      app_bundle_path_from_executable_path(Path::new(
        "/Applications/Reviu.app/Contents/Helpers/reviu"
      )),
      None
    );
  }

  #[cfg(target_os = "macos")]
  #[test]
  fn ready_version_label_uses_file_stem_and_falls_back_for_empty_names() {
    assert_eq!(
      ready_version_label(Path::new("/tmp/reviu-0.2.0.dmg")),
      "reviu-0.2.0"
    );
    assert_eq!(ready_version_label(Path::new("")), "latest");
  }
}
