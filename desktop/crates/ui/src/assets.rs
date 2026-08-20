use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::OnceLock;

use gpui::{AssetSource, SharedString};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "assets"]
struct UiAssets;

/// Assets under this prefix are read from a directory the app fills at runtime
/// (agent icons fetched from the ACP registry) rather than from the binary.
const RUNTIME_PREFIX: &str = "agent-icons/";

static RUNTIME_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Point the runtime asset prefix at a directory. Call once, before any window.
pub fn set_runtime_asset_dir(dir: PathBuf) {
  let _ = RUNTIME_DIR.set(dir);
}

/// Resolve a runtime asset path to a file, refusing anything that is not a
/// plain file name: these paths carry ids fetched from a remote registry.
fn runtime_asset_file(path: &str) -> Option<PathBuf> {
  let name = path.strip_prefix(RUNTIME_PREFIX)?;
  if name.is_empty()
    || name.starts_with('.')
    || name.contains(['/', '\\'])
    || std::path::Path::new(name).components().count() != 1
  {
    return None;
  }
  Some(RUNTIME_DIR.get()?.join(name))
}

pub struct AppAssets;

impl AssetSource for AppAssets {
  fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
    if let Some(asset) = UiAssets::get(path) {
      return Ok(Some(asset.data));
    }

    if path.starts_with(RUNTIME_PREFIX) {
      // A missing runtime asset is normal (not fetched yet): report absence so
      // the caller can fall back, and try again on the next paint.
      return Ok(
        runtime_asset_file(path)
          .and_then(|file| std::fs::read(file).ok())
          .map(Cow::Owned),
      );
    }

    gpui_component_assets::Assets.load(path)
  }

  fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
    let mut items = gpui_component_assets::Assets.list(path)?;

    for asset in UiAssets::iter() {
      if asset.starts_with(path) {
        items.push(asset.as_ref().to_string().into());
      }
    }

    items.sort();
    items.dedup();
    Ok(items)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn runtime_asset_paths_cannot_escape_their_directory() {
    set_runtime_asset_dir(PathBuf::from("/tmp/reviu-runtime-assets"));

    assert_eq!(
      runtime_asset_file("agent-icons/gemini.svg"),
      Some(PathBuf::from("/tmp/reviu-runtime-assets/gemini.svg"))
    );

    for path in [
      "agent-icons/../../etc/passwd",
      "agent-icons/../secret",
      "agent-icons/nested/file.svg",
      "agent-icons/.hidden",
      "agent-icons/",
      "icons/other.svg",
    ] {
      assert_eq!(
        runtime_asset_file(path),
        None,
        "{path:?} must not resolve to a file"
      );
    }
  }

  #[test]
  fn a_missing_runtime_asset_is_absent_rather_than_an_error() {
    set_runtime_asset_dir(PathBuf::from("/tmp/reviu-runtime-assets"));
    let loaded = AppAssets
      .load("agent-icons/definitely-not-there.svg")
      .expect("a missing runtime asset is not an error");
    assert!(loaded.is_none());
  }

  #[test]
  fn a_cached_runtime_asset_is_served_from_disk() {
    let dir = PathBuf::from("/tmp/reviu-runtime-assets");
    std::fs::create_dir_all(&dir).expect("temp dir");
    std::fs::write(dir.join("served.svg"), b"<svg/>").expect("write");
    set_runtime_asset_dir(dir.clone());

    let loaded = AppAssets
      .load("agent-icons/served.svg")
      .expect("load")
      .expect("the cached file is served");
    assert_eq!(loaded.as_ref(), b"<svg/>");

    // Embedded assets still win over the runtime directory.
    assert!(
      AppAssets
        .load("icons/brands/claude.svg")
        .expect("load")
        .is_some()
    );

    std::fs::remove_file(dir.join("served.svg")).ok();
  }
}
