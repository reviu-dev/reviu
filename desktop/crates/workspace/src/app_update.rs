#[cfg(not(target_os = "windows"))]
use std::ffi::OsString;
use std::{
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
  pub restart_binary_path: Option<PathBuf>,
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
  std::env::consts::OS
}

pub fn current_arch() -> &'static str {
  std::env::consts::ARCH
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

  #[cfg(target_os = "linux")]
  {
    let restart_binary_path = current_linux_install_layout()
      .ok()
      .map(|layout| layout.restart_binary_path);
    Ok(ReadyToInstallAppUpdate {
      update: update.clone(),
      artifact_path,
      restart_binary_path,
    })
  }

  #[cfg(not(target_os = "linux"))]
  {
    Ok(ReadyToInstallAppUpdate {
      update: update.clone(),
      artifact_path,
      restart_binary_path: None,
    })
  }
}

pub fn install_update_artifact(ready: &ReadyToInstallAppUpdate) -> Result<()> {
  #[cfg(target_os = "macos")]
  {
    install_update_macos(&ready.artifact_path)
  }

  #[cfg(target_os = "linux")]
  {
    install_update_linux(ready)
  }

  #[cfg(target_os = "windows")]
  {
    install_update_windows(&ready.artifact_path)
  }

  #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
  {
    let _ = ready;
    bail!("installer launch is currently supported on macOS, Linux, and Windows only")
  }
}

pub fn should_install_update_after_download() -> bool {
  !cfg!(target_os = "windows")
}

pub fn ready_update_button_label() -> &'static str {
  if cfg!(target_os = "windows") {
    "Install update"
  } else {
    "Restart to update"
  }
}

pub fn ready_update_status_message() -> &'static str {
  if cfg!(target_os = "windows") {
    "Update ready. Run the installer to finish applying it."
  } else {
    "Update ready. Restart Reviu to finish applying it."
  }
}

#[cfg(target_os = "windows")]
fn install_update_windows(artifact_path: &Path) -> Result<()> {
  Command::new(artifact_path).spawn().with_context(|| {
    format!(
      "failed to launch Windows installer {}",
      artifact_path.display()
    )
  })?;

  Ok(())
}

#[cfg(target_os = "linux")]
fn install_update_linux(ready: &ReadyToInstallAppUpdate) -> Result<()> {
  let layout = current_linux_install_layout()?;
  let extract_root = linux_update_extract_root(&ready.update.latest_version);
  if extract_root.exists() {
    let _ = fs::remove_dir_all(&extract_root);
  }
  fs::create_dir_all(&extract_root).with_context(|| {
    format!(
      "failed to create Linux update extract directory {}",
      extract_root.display()
    )
  })?;

  let _cleanup = ScopedDirRemoval {
    path: extract_root.clone(),
  };

  let status = Command::new("tar")
    .arg("-xzf")
    .arg(&ready.artifact_path)
    .arg("-C")
    .arg(&extract_root)
    .status()
    .with_context(|| {
      format!(
        "failed to extract Linux update archive {}",
        ready.artifact_path.display()
      )
    })?;

  if !status.success() {
    bail!(
      "failed to extract Linux update archive {}",
      ready.artifact_path.display()
    );
  }

  let package_root = find_linux_package_root(&extract_root)?;
  let source_binary = package_root.join("bin").join("reviu");
  let source_icon = package_root
    .join("share")
    .join("icons")
    .join("hicolor")
    .join("512x512")
    .join("apps")
    .join("reviu.png");

  if !source_binary.is_file() {
    bail!(
      "Linux update archive is missing bin/reviu in {}",
      package_root.display()
    );
  }

  if !source_icon.is_file() {
    bail!(
      "Linux update archive is missing share/icons/hicolor/512x512/apps/reviu.png in {}",
      package_root.display()
    );
  }

  let version_dir = layout
    .install_base
    .join("versions")
    .join(&ready.update.latest_version);
  if version_dir.exists() {
    fs::remove_dir_all(&version_dir)
      .with_context(|| format!("failed to remove {}", version_dir.display()))?;
  }
  fs::create_dir_all(layout.install_base.join("versions")).with_context(|| {
    format!(
      "failed to create Linux versions directory {}",
      layout.install_base.join("versions").display()
    )
  })?;

  fs::rename(&package_root, &version_dir).with_context(|| {
    format!(
      "failed to move extracted Linux update into {}",
      version_dir.display()
    )
  })?;

  recreate_symlink(&layout.current_link, &version_dir)?;
  fs::create_dir_all(&layout.icons_dir)
    .with_context(|| format!("failed to create {}", layout.icons_dir.display()))?;
  fs::create_dir_all(&layout.applications_dir)
    .with_context(|| format!("failed to create {}", layout.applications_dir.display()))?;

  let installed_icon = version_dir
    .join("share")
    .join("icons")
    .join("hicolor")
    .join("512x512")
    .join("apps")
    .join("reviu.png");
  fs::copy(&installed_icon, &layout.icon_target).with_context(|| {
    format!(
      "failed to copy Linux app icon from {} to {}",
      installed_icon.display(),
      layout.icon_target.display()
    )
  })?;

  write_linux_desktop_entry(
    &layout.desktop_entry_path,
    &layout.restart_binary_path,
    &layout.icon_target,
  )?;
  refresh_linux_desktop_registration(&layout);

  Ok(())
}

#[cfg(any(target_os = "linux", test))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinuxInstallLayout {
  pub install_base: PathBuf,
  pub data_home: PathBuf,
  pub current_link: PathBuf,
  pub restart_binary_path: PathBuf,
  pub applications_dir: PathBuf,
  pub desktop_entry_path: PathBuf,
  pub icons_dir: PathBuf,
  pub icon_target: PathBuf,
}

#[cfg(target_os = "linux")]
fn current_linux_install_layout() -> Result<LinuxInstallLayout> {
  let current_exe = std::env::current_exe().context("failed to resolve current executable")?;
  let resolved = fs::canonicalize(&current_exe).unwrap_or(current_exe.clone());
  linux_install_layout_from_executable_path(&resolved).with_context(|| {
    format!(
      "auto update requires running from the installed Linux bundle, current executable is {}",
      resolved.display()
    )
  })
}

#[cfg(any(target_os = "linux", test))]
fn linux_install_layout_from_executable_path(executable_path: &Path) -> Option<LinuxInstallLayout> {
  if executable_path.file_name()?.to_str()? != "reviu" {
    return None;
  }

  let bin_dir = executable_path.parent()?;
  if bin_dir.file_name()?.to_str()? != "bin" {
    return None;
  }

  let bundle_root = bin_dir.parent()?;
  let install_base = match bundle_root.file_name()?.to_str()? {
    "current" => bundle_root.parent()?.to_path_buf(),
    _ => {
      let versions_dir = bundle_root.parent()?;
      if versions_dir.file_name()?.to_str()? != "versions" {
        return None;
      }
      versions_dir.parent()?.to_path_buf()
    }
  };

  let data_home = install_base.parent()?.to_path_buf();
  let current_link = install_base.join("current");
  let restart_binary_path = current_link.join("bin").join("reviu");
  let applications_dir = data_home.join("applications");
  let desktop_entry_path = applications_dir.join("reviu.desktop");
  let icons_dir = data_home
    .join("icons")
    .join("hicolor")
    .join("512x512")
    .join("apps");
  let icon_target = icons_dir.join("reviu.png");

  Some(LinuxInstallLayout {
    install_base,
    data_home,
    current_link,
    restart_binary_path,
    applications_dir,
    desktop_entry_path,
    icons_dir,
    icon_target,
  })
}

#[cfg(any(target_os = "linux", test))]
fn linux_update_extract_root(version: &str) -> PathBuf {
  std::env::temp_dir().join(format!(
    "reviu-linux-update-{}-{}",
    std::process::id(),
    version
      .chars()
      .map(|ch| {
        if ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' || ch == '_' {
          ch
        } else {
          '_'
        }
      })
      .collect::<String>()
  ))
}

#[cfg(any(target_os = "linux", test))]
fn find_linux_package_root(extract_root: &Path) -> Result<PathBuf> {
  let entries = fs::read_dir(extract_root).with_context(|| {
    format!(
      "failed to inspect extracted update {}",
      extract_root.display()
    )
  })?;
  for entry in entries.flatten() {
    let path = entry.path();
    if path.is_dir() {
      return Ok(path);
    }
  }

  bail!(
    "failed to find top-level extracted Linux update directory in {}",
    extract_root.display()
  )
}

#[cfg(target_os = "linux")]
fn recreate_symlink(link_path: &Path, target_path: &Path) -> Result<()> {
  if link_path.exists() || link_path.symlink_metadata().is_ok() {
    if link_path.is_dir() && !link_path.symlink_metadata()?.file_type().is_symlink() {
      fs::remove_dir_all(link_path)
        .with_context(|| format!("failed to remove {}", link_path.display()))?;
    } else {
      fs::remove_file(link_path)
        .with_context(|| format!("failed to remove {}", link_path.display()))?;
    }
  }

  std::os::unix::fs::symlink(target_path, link_path).with_context(|| {
    format!(
      "failed to create symlink {} -> {}",
      link_path.display(),
      target_path.display()
    )
  })?;

  Ok(())
}

#[cfg(any(target_os = "linux", test))]
fn linux_desktop_entry_contents(binary_path: &Path, icon_path: &Path) -> String {
  format!(
    "[Desktop Entry]\nType=Application\nName=Reviu\nComment=Keyboard-first Git client\nExec={} %U\nIcon={}\nTerminal=false\nCategories=Development;VersionControl;\nMimeType=x-scheme-handler/reviu;\nStartupWMClass=Reviu\n",
    binary_path.display(),
    icon_path.display()
  )
}

#[cfg(target_os = "linux")]
fn write_linux_desktop_entry(
  desktop_entry_path: &Path,
  binary_path: &Path,
  icon_path: &Path,
) -> Result<()> {
  fs::write(
    desktop_entry_path,
    linux_desktop_entry_contents(binary_path, icon_path),
  )
  .with_context(|| format!("failed to write {}", desktop_entry_path.display()))
}

#[cfg(target_os = "linux")]
fn refresh_linux_desktop_registration(layout: &LinuxInstallLayout) {
  let xdg_mime_default = OsString::from("default");
  let xdg_mime_desktop = OsString::from("reviu.desktop");
  let xdg_mime_scheme = OsString::from("x-scheme-handler/reviu");
  try_run_optional_linux_command(
    "update-desktop-database",
    &[layout.applications_dir.as_os_str()],
  );
  try_run_optional_linux_command(
    "xdg-mime",
    &[
      xdg_mime_default.as_os_str(),
      xdg_mime_desktop.as_os_str(),
      xdg_mime_scheme.as_os_str(),
    ],
  );
}

#[cfg(target_os = "linux")]
fn try_run_optional_linux_command(command: &str, args: &[&std::ffi::OsStr]) {
  let Ok(status) = Command::new(command).args(args).status() else {
    return;
  };
  let _ = status.success();
}

#[cfg(target_os = "linux")]
struct ScopedDirRemoval {
  path: PathBuf,
}

#[cfg(target_os = "linux")]
impl Drop for ScopedDirRemoval {
  fn drop(&mut self) {
    let _ = fs::remove_dir_all(&self.path);
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
        restart_binary_path: None,
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

  #[test]
  fn update_install_timing_matches_current_platform() {
    assert_eq!(
      should_install_update_after_download(),
      !cfg!(target_os = "windows")
    );
  }

  #[test]
  fn ready_update_copy_matches_current_platform() {
    if cfg!(target_os = "windows") {
      assert_eq!(ready_update_button_label(), "Install update");
      assert_eq!(
        ready_update_status_message(),
        "Update ready. Run the installer to finish applying it."
      );
    } else {
      assert_eq!(ready_update_button_label(), "Restart to update");
      assert_eq!(
        ready_update_status_message(),
        "Update ready. Restart Reviu to finish applying it."
      );
    }
  }

  #[test]
  fn linux_install_layout_accepts_current_and_versioned_paths() {
    assert_eq!(
      linux_install_layout_from_executable_path(Path::new(
        "/home/test/.local/share/reviu/current/bin/reviu"
      )),
      Some(LinuxInstallLayout {
        install_base: PathBuf::from("/home/test/.local/share/reviu"),
        data_home: PathBuf::from("/home/test/.local/share"),
        current_link: PathBuf::from("/home/test/.local/share/reviu/current"),
        restart_binary_path: PathBuf::from("/home/test/.local/share/reviu/current/bin/reviu"),
        applications_dir: PathBuf::from("/home/test/.local/share/applications"),
        desktop_entry_path: PathBuf::from("/home/test/.local/share/applications/reviu.desktop"),
        icons_dir: PathBuf::from("/home/test/.local/share/icons/hicolor/512x512/apps"),
        icon_target: PathBuf::from("/home/test/.local/share/icons/hicolor/512x512/apps/reviu.png"),
      })
    );

    assert_eq!(
      linux_install_layout_from_executable_path(Path::new(
        "/home/test/.local/share/reviu/versions/0.0.11/bin/reviu"
      )),
      Some(LinuxInstallLayout {
        install_base: PathBuf::from("/home/test/.local/share/reviu"),
        data_home: PathBuf::from("/home/test/.local/share"),
        current_link: PathBuf::from("/home/test/.local/share/reviu/current"),
        restart_binary_path: PathBuf::from("/home/test/.local/share/reviu/current/bin/reviu"),
        applications_dir: PathBuf::from("/home/test/.local/share/applications"),
        desktop_entry_path: PathBuf::from("/home/test/.local/share/applications/reviu.desktop"),
        icons_dir: PathBuf::from("/home/test/.local/share/icons/hicolor/512x512/apps"),
        icon_target: PathBuf::from("/home/test/.local/share/icons/hicolor/512x512/apps/reviu.png"),
      })
    );
  }

  #[test]
  fn linux_install_layout_rejects_non_bundle_paths() {
    assert_eq!(
      linux_install_layout_from_executable_path(Path::new("/usr/local/bin/reviu")),
      None
    );
    assert_eq!(
      linux_install_layout_from_executable_path(Path::new(
        "/home/test/.local/share/reviu/versions/0.0.11/helpers/reviu"
      )),
      None
    );
  }

  #[test]
  fn linux_desktop_entry_uses_absolute_binary_and_icon_paths() {
    let desktop_entry = linux_desktop_entry_contents(
      Path::new("/home/test/.local/share/reviu/current/bin/reviu"),
      Path::new("/home/test/.local/share/icons/hicolor/512x512/apps/reviu.png"),
    );

    assert!(desktop_entry.contains("Exec=/home/test/.local/share/reviu/current/bin/reviu %U"));
    assert!(
      desktop_entry.contains("Icon=/home/test/.local/share/icons/hicolor/512x512/apps/reviu.png")
    );
    assert!(desktop_entry.contains("MimeType=x-scheme-handler/reviu;"));
  }

  #[test]
  fn linux_update_extract_root_sanitizes_version_label() {
    let path = linux_update_extract_root("0.2.0-beta+linux build");
    let file_name = path.file_name().and_then(|value| value.to_str());

    assert!(matches!(file_name, Some(name) if name.contains("0.2.0-beta_linux_build")));
  }

  #[test]
  fn find_linux_package_root_returns_first_top_level_directory() {
    let temp_root =
      std::env::temp_dir().join(format!("reviu-app-update-test-{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp_root);
    fs::create_dir_all(temp_root.join("Reviu-0.0.11-linux-x86_64")).expect("create package root");
    fs::write(temp_root.join("ignored.txt"), "fixture").expect("create file fixture");

    let package_root = find_linux_package_root(&temp_root).expect("package root");

    assert_eq!(package_root, temp_root.join("Reviu-0.0.11-linux-x86_64"));

    let _ = fs::remove_dir_all(&temp_root);
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
