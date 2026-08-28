use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, anyhow, bail};
use serde::Serialize;

const DEFAULT_FILE_COUNT: usize = 300;
const DEFAULT_SAMPLE_SECONDS: u64 = 5;
const DEFAULT_OUTPUT_ROOT: &str = "target/perf/reviu-driver";

const SAMPLE_BUCKETS: &[SampleBucket] = &[
  SampleBucket {
    name: "main_thread_draw",
    needles: &["Window::draw", "draw_roots", "window::step"],
  },
  SampleBucket {
    name: "layout_paint",
    needles: &[
      "compute_layout",
      "request_layout",
      "prepaint",
      "paint",
      "Taffy",
    ],
  },
  SampleBucket {
    name: "agent_chat",
    needles: &["AgentChatPanel", "agent_chat_panel"],
  },
  SampleBucket {
    name: "changes_list",
    needles: &[
      "ChangesList",
      "changes_list",
      "DockPanel",
      "dock_panel",
      "VirtualList",
      "ListState",
    ],
  },
  SampleBucket {
    name: "gfm_text",
    needles: &["gfm_markdown_viewer", "TextView", "markdown", "comrak"],
  },
  SampleBucket {
    name: "git_status",
    needles: &[
      "list_repo_status",
      "RepoSnapshot",
      "status_poll",
      "git::status",
    ],
  },
  SampleBucket {
    name: "store_json",
    needles: &["ConversationStore", "kick_writer", "serde_json"],
  },
];

#[derive(Clone, Copy)]
struct SampleBucket {
  name: &'static str,
  needles: &'static [&'static str],
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PerfArgs {
  pub(crate) backend: String,
  pub(crate) output_root: PathBuf,
  pub(crate) file_count: usize,
  pub(crate) sample_seconds: u64,
  pub(crate) skip_sample: bool,
  pub(crate) driver_bin: Option<PathBuf>,
  pub(crate) agent_bin: Option<PathBuf>,
}

impl Default for PerfArgs {
  fn default() -> Self {
    Self {
      backend: "visual".to_string(),
      output_root: PathBuf::from(DEFAULT_OUTPUT_ROOT),
      file_count: DEFAULT_FILE_COUNT,
      sample_seconds: DEFAULT_SAMPLE_SECONDS,
      skip_sample: false,
      driver_bin: None,
      agent_bin: None,
    }
  }
}

#[derive(Serialize)]
struct PerfManifest {
  repo: PathBuf,
  output_root: PathBuf,
  backend: String,
  file_count: usize,
  sample_seconds: u64,
  scenarios: Vec<ScenarioReport>,
}

#[derive(Clone, Serialize)]
struct ScenarioReport {
  name: &'static str,
  sample_path: Option<PathBuf>,
  screenshot_path: Option<PathBuf>,
  cpu_samples_path: PathBuf,
  analysis: SampleAnalysis,
  ps_samples: Vec<PsSample>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct SampleAnalysis {
  pub(crate) total_symbol_samples: u64,
  pub(crate) buckets: BTreeMap<String, u64>,
}

#[derive(Clone, Serialize)]
struct PsSample {
  elapsed_ms: u128,
  cpu_percent: Option<f32>,
  rss_kb: Option<u64>,
}

struct TempRunDir {
  path: PathBuf,
}

impl TempRunDir {
  fn new(root: &Path) -> Result<Self> {
    let millis = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .unwrap_or_default()
      .as_millis();
    let path = root.join(format!("run-{millis}-{}", std::process::id()));
    fs::create_dir_all(&path).with_context(|| format!("create {}", path.display()))?;
    Ok(Self { path })
  }
}

pub(crate) fn parse_args(args: impl IntoIterator<Item = String>) -> Result<PerfArgs> {
  let mut parsed = PerfArgs::default();
  let mut args = args.into_iter();
  while let Some(arg) = args.next() {
    match arg.as_str() {
      "--backend" => parsed.backend = required_value(&mut args, "--backend")?,
      "--output" => parsed.output_root = PathBuf::from(required_value(&mut args, "--output")?),
      "--files" => {
        parsed.file_count = required_value(&mut args, "--files")?
          .parse()
          .context("--files must be a positive integer")?;
      }
      "--sample-seconds" => {
        parsed.sample_seconds = required_value(&mut args, "--sample-seconds")?
          .parse()
          .context("--sample-seconds must be a positive integer")?;
      }
      "--driver-bin" => {
        parsed.driver_bin = Some(PathBuf::from(required_value(&mut args, "--driver-bin")?))
      }
      "--agent-bin" => {
        parsed.agent_bin = Some(PathBuf::from(required_value(&mut args, "--agent-bin")?))
      }
      "--skip-sample" => parsed.skip_sample = true,
      "--help" | "-h" => bail!(usage()),
      other if other.starts_with("--backend=") => {
        parsed.backend = other.trim_start_matches("--backend=").to_string();
      }
      other if other.starts_with("--output=") => {
        parsed.output_root = PathBuf::from(other.trim_start_matches("--output="));
      }
      other if other.starts_with("--files=") => {
        parsed.file_count = other
          .trim_start_matches("--files=")
          .parse()
          .context("--files must be a positive integer")?;
      }
      other if other.starts_with("--sample-seconds=") => {
        parsed.sample_seconds = other
          .trim_start_matches("--sample-seconds=")
          .parse()
          .context("--sample-seconds must be a positive integer")?;
      }
      other => bail!("unknown argument: {other}\n{}", usage()),
    }
  }
  if parsed.file_count == 0 {
    bail!("--files must be greater than zero");
  }
  if parsed.sample_seconds == 0 {
    bail!("--sample-seconds must be greater than zero");
  }
  if parsed.backend != "visual" && parsed.backend != "test" {
    bail!("--backend must be visual or test");
  }
  Ok(parsed)
}

fn required_value(args: &mut impl Iterator<Item = String>, name: &str) -> Result<String> {
  args.next().ok_or_else(|| anyhow!("{name} needs a value"))
}

fn usage() -> &'static str {
  "usage: cargo run -p reviu_driver --bin reviu-perf -- [--backend visual|test] [--files 300] [--sample-seconds 5] [--output target/perf/reviu-driver] [--skip-sample]"
}

pub(crate) fn run(args: PerfArgs) -> Result<()> {
  fs::create_dir_all(&args.output_root)
    .with_context(|| format!("create {}", args.output_root.display()))?;
  let run_dir = TempRunDir::new(&args.output_root)?;
  let repo = setup_temp_repo(&run_dir.path.join("repo"), args.file_count)?;
  let artifacts = run_dir.path.join("artifacts");
  fs::create_dir_all(&artifacts).with_context(|| format!("create {}", artifacts.display()))?;

  let driver_bin = resolve_driver_bin(args.driver_bin.as_deref())?;
  let agent_bin = resolve_agent_bin(args.agent_bin.as_deref())?;
  let mut driver = DriverProcess::spawn(&driver_bin, &agent_bin, &args.backend, &run_dir.path)?;
  driver.command(serde_json::json!({ "cmd": "path_prompt", "path": repo }))?;
  driver.command(serde_json::json!({ "cmd": "park" }))?;

  let scenarios = vec![
    run_scenario(
      &mut driver,
      &artifacts,
      "idle",
      args.sample_seconds,
      args.skip_sample,
      false,
      |driver| {
        driver.command(serde_json::json!({ "cmd": "hide_dock" }))?;
        driver.command(serde_json::json!({ "cmd": "park" }))?;
        Ok(())
      },
    )?,
    run_scenario(
      &mut driver,
      &artifacts,
      "chat_stream",
      args.sample_seconds,
      args.skip_sample,
      true,
      |driver| {
        driver.command(serde_json::json!({ "cmd": "hide_dock" }))?;
        driver.command(
          serde_json::json!({ "cmd": "submit_prompt", "text": "perf-stream markdown tools" }),
        )?;
        Ok(())
      },
    )?,
    run_scenario(
      &mut driver,
      &artifacts,
      "chat_stream_changes",
      args.sample_seconds,
      args.skip_sample,
      true,
      |driver| {
        driver.command(serde_json::json!({ "cmd": "show_changes" }))?;
        driver.command(
          serde_json::json!({ "cmd": "submit_prompt", "text": "perf-stream markdown tools changes" }),
        )?;
        Ok(())
      },
    )?,
  ];

  let manifest = PerfManifest {
    repo,
    output_root: run_dir.path.clone(),
    backend: args.backend,
    file_count: args.file_count,
    sample_seconds: args.sample_seconds,
    scenarios,
  };
  let manifest_path = run_dir.path.join("manifest.json");
  let summary_path = run_dir.path.join("summary.md");
  fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?)?;
  fs::write(&summary_path, render_summary(&manifest))?;
  driver.command(serde_json::json!({ "cmd": "quit" })).ok();
  println!("summary: {}", summary_path.display());
  println!("manifest: {}", manifest_path.display());
  Ok(())
}

fn setup_temp_repo(path: &Path, file_count: usize) -> Result<PathBuf> {
  fs::create_dir_all(path.join("src"))?;
  run_git(path, &["init", "-b", "main"])?;
  run_git(path, &["config", "user.email", "perf@reviu.local"])?;
  run_git(path, &["config", "user.name", "Reviu Perf"])?;

  for index in 0..file_count {
    let file = path.join("src").join(format!("file-{index:04}.rs"));
    fs::write(
      file,
      format!("pub fn value_{index}() -> usize {{\n  {index}\n}}\n"),
    )?;
  }
  fs::write(path.join("README.md"), "# Reviu perf fixture\n")?;
  run_git(path, &["add", "."])?;
  run_git(path, &["commit", "-m", "initial fixture"])?;

  let staged = file_count / 5;
  let unstaged_start = staged;
  let unstaged_end = (file_count / 5) * 3;
  for index in 0..staged {
    let file = path.join("src").join(format!("file-{index:04}.rs"));
    fs::write(
      &file,
      format!("pub fn value_{index}() -> usize {{\n  {index} * 2\n}}\n"),
    )?;
    let relative_file = file.strip_prefix(path).unwrap_or(file.as_path());
    run_git(path, &["add", relative_file.to_string_lossy().as_ref()])?;
  }
  for index in unstaged_start..unstaged_end {
    let file = path.join("src").join(format!("file-{index:04}.rs"));
    fs::write(
      file,
      format!("pub fn value_{index}() -> usize {{\n  {index} * 3\n}}\n"),
    )?;
  }
  for index in 0..(file_count / 12).max(1) {
    fs::write(
      path.join("src").join(format!("untracked-{index:04}.rs")),
      format!("pub fn untracked_{index}() -> usize {{ {index} }}\n"),
    )?;
  }
  Ok(path.to_path_buf())
}

fn run_git(cwd: &Path, args: &[&str]) -> Result<()> {
  let output = Command::new("git")
    .args(args)
    .current_dir(cwd)
    .output()
    .with_context(|| format!("git {}", args.join(" ")))?;
  if !output.status.success() {
    bail!(
      "git {} failed: {}",
      args.join(" "),
      String::from_utf8_lossy(&output.stderr)
    );
  }
  Ok(())
}

fn resolve_driver_bin(configured: Option<&Path>) -> Result<PathBuf> {
  resolve_sibling_or_build(
    configured,
    "reviu-driver",
    &["-p", "reviu_driver", "--bin", "reviu-driver"],
  )
}

fn resolve_agent_bin(configured: Option<&Path>) -> Result<PathBuf> {
  resolve_sibling_or_build(
    configured,
    "stub_agent",
    &[
      "-p",
      "agent_acp",
      "--features",
      "test-support",
      "--bin",
      "stub_agent",
    ],
  )
}

fn resolve_sibling_or_build(
  configured: Option<&Path>,
  binary_name: &str,
  cargo_args: &[&str],
) -> Result<PathBuf> {
  if let Some(path) = configured {
    return Ok(path.to_path_buf());
  }
  let exe = std::env::current_exe().context("current executable")?;
  let candidate = exe
    .parent()
    .ok_or_else(|| anyhow!("current executable has no parent"))?
    .join(binary_name);
  let status = Command::new("cargo")
    .arg("build")
    .args(cargo_args)
    .current_dir(workspace_root())
    .status()
    .with_context(|| format!("cargo build for {binary_name}"))?;
  if !status.success() {
    bail!("cargo build failed for {binary_name}");
  }
  if candidate.exists() {
    Ok(candidate)
  } else {
    bail!("{} was not built at {}", binary_name, candidate.display())
  }
}

fn workspace_root() -> PathBuf {
  Path::new(env!("CARGO_MANIFEST_DIR"))
    .ancestors()
    .nth(2)
    .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")))
    .to_path_buf()
}

struct DriverProcess {
  child: Child,
  stdin: std::process::ChildStdin,
  stdout: BufReader<std::process::ChildStdout>,
}

impl DriverProcess {
  fn spawn(driver_bin: &Path, agent_bin: &Path, backend: &str, run_dir: &Path) -> Result<Self> {
    let home = run_dir.join("home");
    let config = run_dir.join("config");
    fs::create_dir_all(&home)?;
    fs::create_dir_all(&config)?;
    let mut child = Command::new(driver_bin)
      .arg("--backend")
      .arg(backend)
      .arg("--agent-command")
      .arg(agent_bin)
      .env("HOME", &home)
      .env("XDG_CONFIG_HOME", &config)
      .env("REVIU_PROFILE", "dev")
      .stdin(Stdio::piped())
      .stdout(Stdio::piped())
      .stderr(Stdio::from(fs::File::create(
        run_dir.join("driver.stderr.log"),
      )?))
      .spawn()
      .with_context(|| format!("spawn {}", driver_bin.display()))?;
    let stdin = child.stdin.take().context("driver stdin")?;
    let stdout = BufReader::new(child.stdout.take().context("driver stdout")?);
    let mut process = Self {
      child,
      stdin,
      stdout,
    };
    let ready = process.read_response()?;
    if !ready
      .get("ok")
      .and_then(serde_json::Value::as_bool)
      .unwrap_or(false)
    {
      bail!("driver did not become ready: {ready}");
    }
    Ok(process)
  }

  fn pid(&self) -> u32 {
    self.child.id()
  }

  fn command(&mut self, value: serde_json::Value) -> Result<serde_json::Value> {
    writeln!(self.stdin, "{value}")?;
    self.stdin.flush()?;
    let response = self.read_response()?;
    if !response
      .get("ok")
      .and_then(serde_json::Value::as_bool)
      .unwrap_or(false)
    {
      bail!("driver command failed: {response}");
    }
    Ok(response)
  }

  fn read_response(&mut self) -> Result<serde_json::Value> {
    let mut line = String::new();
    let bytes = self.stdout.read_line(&mut line)?;
    if bytes == 0 {
      bail!("driver closed stdout");
    }
    serde_json::from_str(line.trim()).context("parse driver response")
  }
}

fn run_scenario(
  driver: &mut DriverProcess,
  artifacts: &Path,
  name: &'static str,
  sample_seconds: u64,
  skip_sample: bool,
  pump_driver: bool,
  prepare: impl FnOnce(&mut DriverProcess) -> Result<()>,
) -> Result<ScenarioReport> {
  prepare(driver)?;
  let scenario_dir = artifacts.join(name);
  fs::create_dir_all(&scenario_dir)?;
  let sample_path = scenario_dir.join("sample.txt");
  let cpu_samples_path = scenario_dir.join("ps.json");
  let screenshot_path = scenario_dir.join("screenshot.png");
  let mut sample_child = if skip_sample {
    None
  } else {
    start_sample(driver.pid(), sample_seconds, &sample_path)?
  };
  let ps_samples = collect_ps_while(driver, sample_seconds, pump_driver)?;
  if let Some(child) = sample_child.as_mut() {
    let _ = child.wait();
  }
  let analysis = if sample_path.exists() {
    analyze_sample_text(&fs::read_to_string(&sample_path).unwrap_or_default())
  } else {
    SampleAnalysis::default()
  };
  fs::write(
    &cpu_samples_path,
    serde_json::to_string_pretty(&ps_samples)?,
  )?;
  if driver
    .command(serde_json::json!({ "cmd": "screenshot", "path": screenshot_path }))
    .is_err()
  {
    let _ = fs::remove_file(&screenshot_path);
  }
  driver.command(serde_json::json!({ "cmd": "wait", "ms": 4_000 }))?;
  Ok(ScenarioReport {
    name,
    sample_path: sample_path.exists().then_some(sample_path),
    screenshot_path: screenshot_path.exists().then_some(screenshot_path),
    cpu_samples_path,
    analysis,
    ps_samples,
  })
}

fn start_sample(pid: u32, seconds: u64, path: &Path) -> Result<Option<Child>> {
  if cfg!(target_os = "macos") {
    let child = Command::new("sample")
      .arg(pid.to_string())
      .arg(seconds.to_string())
      .arg("1")
      .arg("-file")
      .arg(path)
      .stdout(Stdio::null())
      .stderr(Stdio::null())
      .spawn()
      .context("start sample")?;
    Ok(Some(child))
  } else {
    Ok(None)
  }
}

fn collect_ps_while(
  driver: &mut DriverProcess,
  seconds: u64,
  pump_driver: bool,
) -> Result<Vec<PsSample>> {
  let start = Instant::now();
  let deadline = start + Duration::from_secs(seconds);
  let mut samples = Vec::new();
  while Instant::now() < deadline {
    samples.push(read_ps(driver.pid(), start));
    if pump_driver {
      driver.command(serde_json::json!({ "cmd": "wait", "ms": 250 }))?;
    } else {
      std::thread::sleep(Duration::from_millis(250));
    }
  }
  Ok(samples)
}

fn read_ps(pid: u32, start: Instant) -> PsSample {
  let output = Command::new("ps")
    .args(["-p", &pid.to_string(), "-o", "%cpu=,rss="])
    .output();
  let (cpu_percent, rss_kb) = output
    .ok()
    .and_then(|output| String::from_utf8(output.stdout).ok())
    .and_then(|stdout| parse_ps_line(&stdout))
    .unwrap_or((None, None));
  PsSample {
    elapsed_ms: start.elapsed().as_millis(),
    cpu_percent,
    rss_kb,
  }
}

fn parse_ps_line(stdout: &str) -> Option<(Option<f32>, Option<u64>)> {
  let line = stdout.lines().find(|line| !line.trim().is_empty())?;
  let mut parts = line.split_whitespace();
  let cpu = parts.next().and_then(|part| part.parse().ok());
  let rss = parts.next().and_then(|part| part.parse().ok());
  Some((cpu, rss))
}

pub(crate) fn analyze_sample_text(text: &str) -> SampleAnalysis {
  let mut analysis = SampleAnalysis::default();
  for bucket in SAMPLE_BUCKETS {
    analysis.buckets.insert(bucket.name.to_string(), 0);
  }
  for line in text.lines() {
    let Some(count) = leading_sample_count(line) else {
      continue;
    };
    analysis.total_symbol_samples += count;
    for bucket in SAMPLE_BUCKETS {
      if bucket.needles.iter().any(|needle| line.contains(needle)) {
        *analysis.buckets.entry(bucket.name.to_string()).or_default() += count;
      }
    }
  }
  analysis
}

fn leading_sample_count(line: &str) -> Option<u64> {
  let trimmed =
    line.trim_start_matches(|c: char| c.is_whitespace() || matches!(c, '+' | '!' | ':' | '|'));
  let digits: String = trimmed.chars().take_while(|c| c.is_ascii_digit()).collect();
  if digits.is_empty() {
    None
  } else {
    digits.parse().ok()
  }
}

fn render_summary(manifest: &PerfManifest) -> String {
  let mut out = String::new();
  out.push_str("# Reviu driver perf run\n\n");
  out.push_str(&format!("- repo: `{}`\n", manifest.repo.display()));
  out.push_str(&format!("- backend: `{}`\n", manifest.backend));
  out.push_str(&format!("- files: `{}`\n", manifest.file_count));
  out.push_str(&format!(
    "- sample seconds: `{}`\n\n",
    manifest.sample_seconds
  ));
  out.push_str(
    "| scenario | avg CPU | max RSS MB | changes_list | agent_chat | layout_paint | sample |\n",
  );
  out.push_str("| --- | ---: | ---: | ---: | ---: | ---: | --- |\n");
  for scenario in &manifest.scenarios {
    let avg_cpu = average_cpu(&scenario.ps_samples)
      .map(|value| format!("{value:.1}%"))
      .unwrap_or_else(|| "n/a".to_string());
    let max_rss = scenario
      .ps_samples
      .iter()
      .filter_map(|sample| sample.rss_kb)
      .max()
      .map(|kb| format!("{:.1}", kb as f64 / 1024.0))
      .unwrap_or_else(|| "n/a".to_string());
    let bucket = |name: &str| scenario.analysis.buckets.get(name).copied().unwrap_or(0);
    let sample_path = scenario
      .sample_path
      .as_ref()
      .map(|path| format!("`{}`", path.display()))
      .unwrap_or_else(|| "n/a".to_string());
    out.push_str(&format!(
      "| {} | {} | {} | {} | {} | {} | {} |\n",
      scenario.name,
      avg_cpu,
      max_rss,
      bucket("changes_list"),
      bucket("agent_chat"),
      bucket("layout_paint"),
      sample_path,
    ));
  }
  out
}

fn average_cpu(samples: &[PsSample]) -> Option<f32> {
  let mut total = 0.0;
  let mut count = 0;
  for value in samples.iter().filter_map(|sample| sample.cpu_percent) {
    total += value;
    count += 1;
  }
  (count > 0).then_some(total / count as f32)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn perf_args_parse_defaults_and_overrides() {
    let args = parse_args([
      "--backend=test".to_string(),
      "--files".to_string(),
      "42".to_string(),
      "--sample-seconds=2".to_string(),
      "--skip-sample".to_string(),
    ])
    .expect("args");
    assert_eq!(args.backend, "test");
    assert_eq!(args.file_count, 42);
    assert_eq!(args.sample_seconds, 2);
    assert!(args.skip_sample);
  }

  #[test]
  fn sample_analysis_buckets_weighted_symbols() {
    let text = r#"
    + ! 12 _RNvMs0_WorkspaceView  (in reviu) + 4
    + ! 7 _RNvMs0_ChangesList_render_item  (in reviu) + 8
    + ! 3 _RNvMs0_AgentChatPanel_render  (in reviu) + 12
    + ! 2 taffy::compute_layout  (in reviu) + 16
"#;
    let analysis = analyze_sample_text(text);
    assert_eq!(analysis.total_symbol_samples, 24);
    assert_eq!(analysis.buckets.get("changes_list"), Some(&7));
    assert_eq!(analysis.buckets.get("agent_chat"), Some(&3));
    assert_eq!(analysis.buckets.get("layout_paint"), Some(&2));
  }

  #[test]
  fn ps_line_parses_cpu_and_rss() {
    assert_eq!(
      parse_ps_line(" 12.5  34816\n"),
      Some((Some(12.5), Some(34816)))
    );
  }
}
