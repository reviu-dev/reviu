//! GUI-launched apps inherit a minimal PATH that misses user-installed tools
//! (nvm, homebrew, asdf, mise). Re-run the user's login shell, capture env,
//! copy into our process.

#![cfg(unix)]

use std::collections::HashMap;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShellKind {
  Zsh,
  Bash,
  Fish,
  Nushell,
  Csh,
  Tcsh,
  PosixOther,
}

impl ShellKind {
  fn from_path(path: &Path) -> Self {
    let name = path
      .file_name()
      .and_then(|s| s.to_str())
      .unwrap_or("sh")
      .trim_start_matches('-');
    match name {
      "zsh" => Self::Zsh,
      "bash" => Self::Bash,
      "fish" => Self::Fish,
      "nu" | "nushell" => Self::Nushell,
      "csh" => Self::Csh,
      "tcsh" => Self::Tcsh,
      _ => Self::PosixOther,
    }
  }
}

/// If invoked with `--printenv`, dump process env as JSON and exit. Used by
/// [`load`] to capture the env from a re-spawned login shell.
pub fn handle_printenv_flag() {
  if std::env::args().any(|a| a == "--printenv") {
    let env: HashMap<String, String> = std::env::vars().collect();
    match serde_json::to_string(&env) {
      Ok(json) => println!("{json}"),
      Err(e) => eprintln!("failed to serialize env: {e}"),
    }
    std::process::exit(0);
  }
}

pub fn load() {
  // Terminal launch already inherits the shell env; skip the re-spawn.
  if std::io::stdout().is_terminal() {
    return;
  }
  if let Err(e) = load_inner() {
    eprintln!("login shell env load failed: {e:#}");
  }
}

fn load_inner() -> Result<()> {
  let shell_path = std::env::var_os("SHELL")
    .map(PathBuf::from)
    .filter(|p| !p.as_os_str().is_empty())
    .context("SHELL not set")?;
  let kind = ShellKind::from_path(&shell_path);
  let home = std::env::var_os("HOME")
    .map(PathBuf::from)
    .unwrap_or_else(|| PathBuf::from("/"));
  let exe = std::env::current_exe().context("current_exe")?;

  let home_q = shell_words::quote(home.to_str().context("HOME not UTF-8")?).into_owned();
  let exe_q = shell_words::quote(exe.to_str().context("exe path not UTF-8")?).into_owned();

  // cd $HOME first so direnv / asdf / mise / nvm hooks fire before capture.
  let command_string = match kind {
    ShellKind::Fish => format!("emit fish_prompt; cd {home_q}; {exe_q} --printenv"),
    ShellKind::Nushell => format!("cd {home_q}; ^{exe_q} --printenv; exit"),
    _ => format!("cd {home_q}; {exe_q} --printenv"),
  };

  let mut cmd = Command::new(&shell_path);
  match kind {
    ShellKind::Csh | ShellKind::Tcsh => {
      use std::os::unix::process::CommandExt;
      cmd.arg0("-");
    }
    ShellKind::Fish | ShellKind::Zsh | ShellKind::Bash | ShellKind::PosixOther => {
      cmd.arg("-l");
    }
    ShellKind::Nushell => {}
  }
  match kind {
    ShellKind::Nushell => {
      cmd.args(["-e", &command_string]);
    }
    _ => {
      cmd.args(["-i", "-c", &command_string]);
    }
  }

  cmd
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());

  let output = cmd.output().context("spawn login shell")?;
  let stdout = String::from_utf8_lossy(&output.stdout);
  let env_map = parse_env(&stdout).with_context(|| {
    format!(
      "parse env (exit {}, stderr: {:?})",
      output.status,
      String::from_utf8_lossy(&output.stderr)
    )
  })?;

  apply_env(env_map);
  Ok(())
}

// rc files may print banners before our JSON; scan for the first `{` that
// parses cleanly as the env map.
fn parse_env(output: &str) -> Result<HashMap<String, String>> {
  for (position, _) in output.match_indices('{') {
    let candidate = &output[position..];
    let mut deser = serde_json::Deserializer::from_str(candidate);
    if let Ok(env) = HashMap::<String, String>::deserialize(&mut deser) {
      return Ok(env);
    }
  }
  anyhow::bail!("no JSON env object found in shell output")
}

fn apply_env(env: HashMap<String, String>) {
  // Shell-local vars that pollute embedded terminals if propagated.
  const SKIP: &[&str] = &["SHLVL", "_"];
  for (name, value) in env {
    if SKIP.contains(&name.as_str()) {
      continue;
    }
    // SAFETY: called from main() before any worker threads are spawned.
    unsafe { std::env::set_var(&name, &value) };
  }
}
