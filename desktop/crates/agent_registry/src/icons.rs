use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

use crate::{Registry, is_safe_id};

/// Asset prefix the app's asset source resolves against the icon cache dir.
pub const ICON_ASSET_PREFIX: &str = "agent-icons";

/// Asset path for an agent's icon, or `None` for an id we refuse to touch the
/// filesystem with.
pub fn icon_asset_path(id: &str) -> Option<String> {
  is_safe_id(id).then(|| format!("{ICON_ASSET_PREFIX}/{id}.svg"))
}

/// Ids whose icon is on disk right now. Checked while rendering, so it must
/// never touch the filesystem.
pub fn cached_icon_ids() -> Arc<HashSet<String>> {
  cell().read().expect("icon cache lock").clone()
}

pub fn has_cached_icon(id: &str) -> bool {
  cached_icon_ids().contains(id)
}

fn cell() -> &'static RwLock<Arc<HashSet<String>>> {
  static CELL: OnceLock<RwLock<Arc<HashSet<String>>>> = OnceLock::new();
  CELL.get_or_init(|| RwLock::new(Arc::new(scan_cached_icons(icon_cache_dir().as_deref()))))
}

fn republish(ids: HashSet<String>) {
  *cell().write().expect("icon cache lock") = Arc::new(ids);
}

pub fn icon_cache_dir() -> Option<PathBuf> {
  Some(crate::profile_dir()?.join(ICON_ASSET_PREFIX))
}

fn scan_cached_icons(dir: Option<&Path>) -> HashSet<String> {
  let Some(dir) = dir else {
    return HashSet::new();
  };
  let Ok(entries) = std::fs::read_dir(dir) else {
    return HashSet::new();
  };
  entries
    .filter_map(|entry| {
      let name = entry.ok()?.file_name().into_string().ok()?;
      let id = name.strip_suffix(".svg")?.to_string();
      is_safe_id(&id).then_some(id)
    })
    .collect()
}

/// A remote body is only written to the cache if it really is an SVG: a CDN
/// serving an HTML error page must not land in the icon directory.
pub fn looks_like_svg(body: &str) -> bool {
  let head = body.trim_start();
  if !head.starts_with('<') || !head.contains("<svg") {
    return false;
  }
  // Reject a document that only mentions svg inside some other markup.
  head.starts_with("<svg") || head.starts_with("<?xml") || head.starts_with("<!DOCTYPE svg")
}

/// Download every missing icon into the cache. Blocking: call off the UI
/// thread. `force` refetches icons already cached, for when the registry
/// document changed under the same `/latest/` URLs.
pub fn download_icons_blocking(registry: &Registry, force: bool) -> usize {
  let Some(dir) = icon_cache_dir() else {
    return 0;
  };
  if std::fs::create_dir_all(&dir).is_err() {
    return 0;
  }

  let client = match reqwest::blocking::Client::builder()
    .timeout(std::time::Duration::from_secs(10))
    .build()
  {
    Ok(client) => client,
    Err(_) => return 0,
  };

  let mut cached = (*cached_icon_ids()).clone();
  let mut written = 0;

  for agent in registry.agents() {
    let id = agent.id.as_str();
    if !is_safe_id(id) {
      continue;
    }
    if !force && cached.contains(id) {
      continue;
    }
    let Some(url) = agent.icon.as_deref() else {
      continue;
    };
    let Ok(response) = client.get(url).send() else {
      continue;
    };
    let Ok(response) = response.error_for_status() else {
      continue;
    };
    let Ok(body) = response.text() else {
      continue;
    };
    if !looks_like_svg(&body) {
      continue;
    }
    if std::fs::write(dir.join(format!("{id}.svg")), body).is_ok() {
      cached.insert(id.to_string());
      written += 1;
    }
  }

  republish(cached);
  written
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn only_safe_ids_produce_an_asset_path() {
    assert_eq!(
      icon_asset_path("gemini").as_deref(),
      Some("agent-icons/gemini.svg")
    );
    for id in ["..", "../evil", "a/b", "a\\b", "", ".hidden"] {
      assert_eq!(icon_asset_path(id), None, "{id:?} must not reach the disk");
    }
  }

  #[test]
  fn only_real_svg_bodies_are_cached() {
    assert!(looks_like_svg(
      "<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>"
    ));
    assert!(looks_like_svg("  \n<svg viewBox=\"0 0 16 16\"/>"));
    assert!(looks_like_svg("<?xml version=\"1.0\"?><svg></svg>"));

    assert!(!looks_like_svg(""));
    assert!(!looks_like_svg("   "));
    assert!(!looks_like_svg(
      "<!DOCTYPE html><html><body>404 <svg/></body></html>"
    ));
    assert!(!looks_like_svg("{\"error\":\"not found\"}"));
    assert!(!looks_like_svg("Not Found"));
  }

  #[test]
  fn scanning_reads_ids_from_svg_files_and_ignores_the_rest() {
    let dir = std::env::temp_dir().join("reviu-icon-scan-test");
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).expect("temp dir");
    std::fs::write(dir.join("gemini.svg"), "<svg/>").expect("write");
    std::fs::write(dir.join("pi-acp.svg"), "<svg/>").expect("write");
    std::fs::write(dir.join("notes.txt"), "x").expect("write");

    let ids = scan_cached_icons(Some(&dir));
    let mut ids: Vec<String> = ids.into_iter().collect();
    ids.sort();
    assert_eq!(ids, vec!["gemini".to_string(), "pi-acp".to_string()]);

    assert!(scan_cached_icons(None).is_empty());
    assert!(scan_cached_icons(Some(&dir.join("missing"))).is_empty());

    std::fs::remove_dir_all(&dir).ok();
  }
}
