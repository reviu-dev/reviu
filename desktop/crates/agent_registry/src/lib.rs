//! The ACP agent registry: which agents exist and how to launch them.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

mod model;
pub use model::{
  AgentId, BinaryTarget, Distribution, RegistryAgent, Runner, is_safe_id, parse_registry,
};

pub const REGISTRY_URL: &str =
  "https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json";

/// Shipped so a first launch, and any offline one, still has the full list.
const SNAPSHOT: &str = include_str!("../assets/registry.json");

const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// The registry moves by a few versions a week, so most launches can skip the
/// request entirely; past the window an ETag keeps the answer to a 304.
const CACHE_TTL: Duration = Duration::from_secs(6 * 60 * 60);

/// Where the freshest known registry came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistrySource {
  Network,
  Cache,
  Snapshot,
}

#[derive(Clone, Debug)]
pub struct Registry {
  agents: Vec<RegistryAgent>,
  pub source: RegistrySource,
}

impl Registry {
  /// The snapshot compiled into the binary. Always succeeds.
  pub fn embedded() -> Self {
    Self {
      agents: parse_registry(SNAPSHOT).unwrap_or_default(),
      source: RegistrySource::Snapshot,
    }
  }

  /// Disk cache if it parses, else the embedded snapshot. Never hits the network.
  pub fn load() -> Self {
    let Some(path) = cache_path() else {
      return Self::embedded();
    };
    Self::from_cache_file(&path).unwrap_or_else(Self::embedded)
  }

  fn from_cache_file(path: &Path) -> Option<Self> {
    let raw = std::fs::read_to_string(path).ok()?;
    let agents = parse_registry(&raw).ok()?;
    (!agents.is_empty()).then_some(Self {
      agents,
      source: RegistrySource::Cache,
    })
  }

  pub fn agents(&self) -> &[RegistryAgent] {
    &self.agents
  }

  /// Agents Reviu can actually launch, in display order.
  pub fn runnable(&self) -> Vec<&RegistryAgent> {
    let mut agents: Vec<&RegistryAgent> = self.agents.iter().filter(|a| a.is_runnable()).collect();
    agents.sort_by_key(|agent| agent.name.to_lowercase());
    agents
  }

  pub fn get(&self, id: &AgentId) -> Option<&RegistryAgent> {
    self.agents.iter().find(|agent| &agent.id == id)
  }
}

/// The registry every caller shares. Loaded from disk once, then swapped in
/// place by a refresh: rendering must never touch the filesystem.
pub fn global() -> Arc<Registry> {
  cell().read().expect("registry lock").clone()
}

fn cell() -> &'static RwLock<Arc<Registry>> {
  static CELL: OnceLock<RwLock<Arc<Registry>>> = OnceLock::new();
  CELL.get_or_init(|| RwLock::new(Arc::new(Registry::load())))
}

fn publish(registry: Registry) -> Arc<Registry> {
  let registry = Arc::new(registry);
  *cell().write().expect("registry lock") = registry.clone();
  registry
}

/// What a refresh attempt did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefreshOutcome {
  /// The cache was young enough that no request was made.
  Skipped,
  /// The registry answered 304: the cache is already current.
  NotModified,
  /// A new registry was fetched and published.
  Updated,
}

/// Refresh from the network and publish the result. Blocking: call off the UI
/// thread. A failure leaves the previously loaded registry in place.
pub fn refresh_global_blocking() -> anyhow::Result<RefreshOutcome> {
  let path = cache_path();

  if path
    .as_deref()
    .is_some_and(|path| cache_is_fresh(path, CACHE_TTL))
  {
    return Ok(RefreshOutcome::Skipped);
  }

  let mut request = reqwest::blocking::Client::builder()
    .timeout(FETCH_TIMEOUT)
    .build()?
    .get(REGISTRY_URL);
  if let Some(etag) = path.as_deref().and_then(read_etag) {
    request = request.header(reqwest::header::IF_NONE_MATCH, etag);
  }

  let response = request.send()?;
  if response.status() == reqwest::StatusCode::NOT_MODIFIED {
    // Touch the cache so the TTL restarts from this confirmation.
    if let Some(path) = path.as_deref() {
      touch(path);
    }
    return Ok(RefreshOutcome::NotModified);
  }

  let response = response.error_for_status()?;
  let etag = response
    .headers()
    .get(reqwest::header::ETAG)
    .and_then(|value| value.to_str().ok())
    .map(str::to_string);
  let body = response.text()?;

  let agents = parse_registry(&body)?;
  if agents.is_empty() {
    anyhow::bail!("registry response held no usable agents");
  }

  if let Some(path) = path.as_deref() {
    if let Some(parent) = path.parent() {
      let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, &body);
    match etag {
      Some(etag) => {
        let _ = std::fs::write(etag_path(path), etag);
      }
      None => {
        let _ = std::fs::remove_file(etag_path(path));
      }
    }
  }

  publish(Registry {
    agents,
    source: RegistrySource::Network,
  });
  Ok(RefreshOutcome::Updated)
}

/// Whether the cached registry is young enough to use without asking again.
fn cache_is_fresh(path: &Path, ttl: Duration) -> bool {
  std::fs::metadata(path)
    .and_then(|meta| meta.modified())
    // A modification time in the future means a clock we cannot trust: refetch.
    .is_ok_and(|modified| modified.elapsed().is_ok_and(|age| age < ttl))
}

fn etag_path(cache: &Path) -> PathBuf {
  cache.with_extension("etag")
}

fn read_etag(cache: &Path) -> Option<String> {
  let etag = std::fs::read_to_string(etag_path(cache)).ok()?;
  let etag = etag.trim().to_string();
  (!etag.is_empty()).then_some(etag)
}

fn touch(path: &Path) {
  if let Ok(file) = std::fs::OpenOptions::new().append(true).open(path) {
    let _ = file.set_modified(std::time::SystemTime::now());
  }
}

fn cache_path() -> Option<PathBuf> {
  Some(cache_path_in(&dirs::config_dir()?, storage_dir_name()))
}

fn cache_path_in(base: &Path, profile_dir: &str) -> PathBuf {
  base.join(profile_dir).join("agent-registry.json")
}

/// Mirrors the app profile split so a dev build never shares prod's cache.
fn storage_dir_name() -> &'static str {
  if cfg!(debug_assertions) {
    "reviu.dev"
  } else {
    "reviu"
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_embedded_snapshot_parses_and_carries_the_known_agents() {
    let registry = Registry::embedded();
    assert!(
      registry.agents().len() > 20,
      "snapshot holds the full registry, got {}",
      registry.agents().len()
    );
    for id in ["claude-acp", "codex-acp", "pi-acp"] {
      let agent = registry
        .get(&AgentId::new(id))
        .unwrap_or_else(|| panic!("{id} is in the snapshot"));
      assert!(agent.is_runnable(), "{id} launches through a command");
    }
  }

  #[test]
  fn a_command_agent_spawns_through_its_package_runner() {
    let registry = Registry::embedded();
    let pi = registry.get(&AgentId::new("pi-acp")).expect("pi-acp");
    let (program, args) = pi.command().expect("pi-acp is runnable");
    assert_eq!(program, "npx");
    assert_eq!(args[0], "-y");
    assert!(
      args[1].starts_with("pi-acp@"),
      "the package carries its pinned version, got {}",
      args[1]
    );
  }

  #[test]
  fn binary_only_agents_are_parsed_but_not_offered() {
    let registry = Registry::embedded();
    let binary_only: Vec<&RegistryAgent> = registry
      .agents()
      .iter()
      .filter(|a| matches!(a.distribution, Distribution::Binary { .. }))
      .collect();
    assert!(!binary_only.is_empty(), "the snapshot has binary agents");
    for agent in binary_only {
      assert!(!agent.is_runnable());
      assert!(agent.command().is_none());
      assert!(!registry.runnable().contains(&agent));
    }
  }

  #[test]
  fn a_broken_entry_is_skipped_without_losing_the_rest() {
    let json = r#"{
      "version": "1.0.0",
      "agents": [
        {"id": "good", "name": "Good", "distribution": {"npx": {"package": "good@1.0.0"}}},
        {"name": "No id", "distribution": {"npx": {"package": "x@1"}}},
        {"id": "no-distribution", "name": "Nope"},
        {"id": "../escape", "name": "Escape", "distribution": {"npx": {"package": "x@1"}}},
        {"id": "empty-binary", "name": "Empty", "distribution": {"binary": {}}},
        {"id": "uv", "name": "Uv", "distribution": {"uvx": {"package": "uv-agent@2"}}}
      ]
    }"#;
    let agents = parse_registry(json).expect("the document itself is valid");
    let ids: Vec<&str> = agents.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, vec!["good", "uv"]);

    let uv = &agents[1];
    let (program, args) = uv.command().expect("uvx agents are runnable");
    assert_eq!(program, "uvx");
    assert_eq!(args, vec!["uv-agent@2".to_string()]);
  }

  #[test]
  fn invalid_json_is_an_error_not_an_empty_registry() {
    assert!(parse_registry("not json").is_err());
  }

  #[test]
  fn ids_that_could_escape_a_directory_are_rejected() {
    for id in ["..", "../x", "a/b", "a\\b", "", ".hidden", &"x".repeat(65)] {
      assert!(!is_safe_id(id), "{id:?} must be rejected");
    }
    for id in ["claude-acp", "pi-acp", "codex_acp", "vt.code"] {
      assert!(is_safe_id(id), "{id:?} must be accepted");
    }
  }

  #[test]
  fn the_global_registry_is_shared_and_never_empty() {
    let first = global();
    let second = global();
    assert!(Arc::ptr_eq(&first, &second), "callers share one registry");
    assert!(!first.runnable().is_empty());
  }

  #[test]
  fn only_pi_needs_a_separate_cli_on_path() {
    let registry = Registry::embedded();
    assert_eq!(
      registry
        .get(&AgentId::new("pi-acp"))
        .expect("pi-acp")
        .required_cli(),
      Some("pi")
    );
    for id in ["claude-acp", "codex-acp"] {
      assert_eq!(
        registry.get(&AgentId::new(id)).expect(id).required_cli(),
        None,
        "{id} bundles its agent, requiring nothing else on PATH"
      );
    }
  }

  #[test]
  fn install_hints_point_at_the_agents_own_page() {
    let registry = Registry::embedded();

    let pi = registry.get(&AgentId::new("pi-acp")).expect("pi-acp");
    assert_eq!(pi.homepage(), "https://github.com/svkozak/pi-acp");
    let hint = pi.install_hint();
    assert!(hint.contains("`pi` CLI"), "got {hint}");
    assert!(hint.contains("github.com/svkozak/pi-acp"), "got {hint}");

    for agent in registry.agents() {
      assert_ne!(
        agent.homepage(),
        "https://agentclientprotocol.com",
        "{} should carry its own site or repo",
        agent.id
      );
    }
  }

  #[test]
  fn an_agent_with_no_links_falls_back_to_the_protocol_page() {
    let json =
      r#"{"agents":[{"id":"bare","name":"Bare","distribution":{"npx":{"package":"bare@1"}}}]}"#;
    let agents = parse_registry(json).expect("valid");
    assert_eq!(agents[0].homepage(), "https://agentclientprotocol.com");
  }

  #[test]
  fn a_fresh_cache_skips_the_request_and_a_stale_one_does_not() {
    let dir = std::env::temp_dir().join("reviu-registry-freshness-test");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let cache = dir.join("agent-registry.json");
    std::fs::write(&cache, "{}").expect("write cache");

    assert!(
      cache_is_fresh(&cache, Duration::from_secs(3600)),
      "a cache written just now is fresh"
    );
    assert!(
      !cache_is_fresh(&cache, Duration::from_secs(0)),
      "any cache is stale against a zero TTL"
    );
    assert!(
      !cache_is_fresh(&dir.join("does-not-exist.json"), Duration::from_secs(3600)),
      "a missing cache is never fresh"
    );

    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn the_etag_sits_beside_its_cache_and_blank_values_are_ignored() {
    let dir = std::env::temp_dir().join("reviu-registry-etag-test");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let cache = dir.join("agent-registry.json");
    assert_eq!(etag_path(&cache), dir.join("agent-registry.etag"));

    assert_eq!(read_etag(&cache), None, "no file means no validator");

    std::fs::write(etag_path(&cache), "  \"abc123\"\n").expect("write etag");
    assert_eq!(read_etag(&cache).as_deref(), Some("\"abc123\""));

    std::fs::write(etag_path(&cache), "   ").expect("write blank etag");
    assert_eq!(read_etag(&cache), None, "a blank validator is not sent");

    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn the_cache_lives_under_the_profile_namespace() {
    assert_eq!(
      cache_path_in(Path::new("/tmp/cfg"), "reviu"),
      PathBuf::from("/tmp/cfg/reviu/agent-registry.json")
    );
  }
}
