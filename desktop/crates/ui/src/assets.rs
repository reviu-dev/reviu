use std::borrow::Cow;

use gpui::{AssetSource, SharedString};

const GIT_BRANCH_ICON: &[u8] = include_bytes!("../assets/icons/git-branch.svg");
const GIT_MERGE_ICON: &[u8] = include_bytes!("../assets/icons/git-merge.svg");

const UI_ICON_PATHS: [&str; 2] = ["icons/git-branch.svg", "icons/git-merge.svg"];

pub struct AppAssets;

impl AssetSource for AppAssets {
  fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
    let data = match path {
      "icons/git-branch.svg" => Some(Cow::Borrowed(GIT_BRANCH_ICON)),
      "icons/git-merge.svg" => Some(Cow::Borrowed(GIT_MERGE_ICON)),
      _ => None,
    };

    if data.is_some() {
      return Ok(data);
    }

    gpui_component_assets::Assets.load(path)
  }

  fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
    let mut items = gpui_component_assets::Assets.list(path)?;

    for asset in UI_ICON_PATHS {
      if asset.starts_with(path) {
        items.push(asset.into());
      }
    }

    items.sort();
    items.dedup();
    Ok(items)
  }
}
