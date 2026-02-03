use std::borrow::Cow;

use gpui::{AssetSource, SharedString};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "assets"]
struct UiAssets;

pub struct AppAssets;

impl AssetSource for AppAssets {
  fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
    if let Some(asset) = UiAssets::get(path) {
      return Ok(Some(asset.data));
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
