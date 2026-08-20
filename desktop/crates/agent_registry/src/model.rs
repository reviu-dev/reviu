use serde::Deserialize;

/// Identifies an agent in the ACP registry (`claude-acp`, `pi-acp`, ...).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AgentId(pub String);

impl AgentId {
  pub fn new(id: impl Into<String>) -> Self {
    Self(id.into())
  }

  pub fn as_str(&self) -> &str {
    &self.0
  }
}

impl std::fmt::Display for AgentId {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str(&self.0)
  }
}

impl From<&str> for AgentId {
  fn from(value: &str) -> Self {
    Self(value.to_string())
  }
}

/// How an agent is launched. `Binary` entries are parsed but not yet runnable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Distribution {
  /// Run through a package runner: `npx -y <package> <args>` or `uvx <package>`.
  Command {
    runner: Runner,
    package: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
  },
  /// Per-target archives to download, verify and extract.
  Binary { targets: Vec<BinaryTarget> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Runner {
  Npx,
  Uvx,
}

impl Runner {
  pub fn program(self) -> &'static str {
    match self {
      Runner::Npx => "npx",
      Runner::Uvx => "uvx",
    }
  }

  /// Args that precede the package name.
  pub fn runner_args(self) -> &'static [&'static str] {
    match self {
      // Answer the install prompt so a first run is not stuck on stdin.
      Runner::Npx => &["-y"],
      Runner::Uvx => &[],
    }
  }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinaryTarget {
  pub triple: String,
  pub archive: String,
  pub cmd: String,
  pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistryAgent {
  pub id: AgentId,
  pub name: String,
  pub version: String,
  pub description: String,
  pub icon: Option<String>,
  pub repository: Option<String>,
  pub website: Option<String>,
  pub distribution: Distribution,
}

impl RegistryAgent {
  /// Whether Reviu can launch this agent today (binary distribution is not
  /// wired up yet, so those entries stay out of the picker).
  pub fn is_runnable(&self) -> bool {
    matches!(self.distribution, Distribution::Command { .. })
  }

  /// The program and args to spawn, for a runnable agent.
  pub fn command(&self) -> Option<(String, Vec<String>)> {
    let Distribution::Command {
      runner,
      package,
      args,
      ..
    } = &self.distribution
    else {
      return None;
    };
    let mut argv: Vec<String> = runner.runner_args().iter().map(|a| a.to_string()).collect();
    argv.push(package.clone());
    argv.extend(args.iter().cloned());
    Some((runner.program().to_string(), argv))
  }

  /// The third-party CLI the adapter drives, which the registry does not
  /// describe. Adapters that bundle their agent need nothing extra on PATH.
  pub fn required_cli(&self) -> Option<&'static str> {
    REQUIRED_CLIS
      .iter()
      .find(|(id, _)| *id == self.id.as_str())
      .map(|(_, cli)| *cli)
  }

  pub fn install_hint(&self) -> String {
    match self.required_cli() {
      Some(cli) => format!(
        "Requires Node.js and the `{cli}` CLI on PATH. See {}.",
        self.homepage()
      ),
      None => format!(
        "Requires Node.js. The package is fetched on first run. See {}.",
        self.homepage()
      ),
    }
  }

  /// Where to send someone whose agent will not start. The registry carries a
  /// site or a repo for every agent; the protocol page is the last resort.
  pub fn homepage(&self) -> &str {
    self
      .website
      .as_deref()
      .or(self.repository.as_deref())
      .unwrap_or("https://agentclientprotocol.com")
  }

  pub fn env(&self) -> &[(String, String)] {
    match &self.distribution {
      Distribution::Command { env, .. } => env,
      Distribution::Binary { .. } => &[],
    }
  }
}

/// Adapters that shell out to a separate agent CLI rather than bundling it.
/// Verified against each package's dependencies; a wrong entry here turns into
/// a false "not installed", so only add an id after checking.
const REQUIRED_CLIS: &[(&str, &str)] = &[("pi-acp", "pi")];

#[derive(Deserialize)]
struct RawRegistry {
  #[serde(default)]
  agents: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct RawCommand {
  package: String,
  #[serde(default)]
  args: Vec<String>,
  #[serde(default)]
  env: std::collections::BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct RawBinaryTarget {
  archive: String,
  cmd: String,
  sha256: String,
}

/// Parse a registry document, skipping entries we cannot understand rather
/// than failing the whole list: one bad agent must not hide the other 37.
pub fn parse_registry(json: &str) -> anyhow::Result<Vec<RegistryAgent>> {
  let raw: RawRegistry = serde_json::from_str(json)?;
  Ok(raw.agents.iter().filter_map(parse_agent).collect())
}

fn parse_agent(value: &serde_json::Value) -> Option<RegistryAgent> {
  let id = value.get("id")?.as_str()?;
  if !is_safe_id(id) {
    return None;
  }
  let distribution = parse_distribution(value.get("distribution")?)?;
  Some(RegistryAgent {
    id: AgentId::new(id),
    name: string_field(value, "name").unwrap_or_else(|| id.to_string()),
    version: string_field(value, "version").unwrap_or_default(),
    description: string_field(value, "description").unwrap_or_default(),
    icon: string_field(value, "icon"),
    repository: string_field(value, "repository"),
    website: string_field(value, "website"),
    distribution,
  })
}

fn string_field(value: &serde_json::Value, key: &str) -> Option<String> {
  value.get(key)?.as_str().map(str::to_string)
}

fn parse_distribution(value: &serde_json::Value) -> Option<Distribution> {
  for (key, runner) in [("npx", Runner::Npx), ("uvx", Runner::Uvx)] {
    if let Some(raw) = value.get(key)
      && let Ok(command) = serde_json::from_value::<RawCommand>(raw.clone())
    {
      return Some(Distribution::Command {
        runner,
        package: command.package,
        args: command.args,
        env: command.env.into_iter().collect(),
      });
    }
  }

  let binary = value.get("binary")?.as_object()?;
  let targets: Vec<BinaryTarget> = binary
    .iter()
    .filter_map(|(triple, raw)| {
      let target: RawBinaryTarget = serde_json::from_value(raw.clone()).ok()?;
      Some(BinaryTarget {
        triple: triple.clone(),
        archive: target.archive,
        cmd: target.cmd,
        sha256: target.sha256,
      })
    })
    .collect();
  (!targets.is_empty()).then_some(Distribution::Binary { targets })
}

/// Registry ids reach the filesystem (icon cache, state dirs), so anything
/// outside this alphabet is rejected before it can escape a directory.
pub fn is_safe_id(id: &str) -> bool {
  !id.is_empty()
    && id.len() <= 64
    && id
      .chars()
      .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    && !id.starts_with('.')
    && id != ".."
}
