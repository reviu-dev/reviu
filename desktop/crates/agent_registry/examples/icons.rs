fn main() {
  let registry = agent_registry::Registry::load();
  let dir = agent_registry::icon_cache_dir().expect("a cache dir");
  println!("cache: {}", dir.display());
  let written = agent_registry::download_icons_blocking(&registry, false);
  println!("fetched {written} icons");
  let cached = agent_registry::cached_icon_ids();
  println!("cached now: {}", cached.len());
  for id in ["gemini", "cline", "goose", "claude-acp"] {
    println!("  {id}: cached={}", agent_registry::has_cached_icon(id));
  }
}
