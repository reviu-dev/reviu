use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use git2::{Config, Cred, CredentialType, RemoteCallbacks, Repository};

pub(crate) fn remote_callbacks(repo: &Repository) -> Result<RemoteCallbacks<'static>> {
  let config = repo.config().context("read git config")?;
  let mut callbacks = RemoteCallbacks::new();
  callbacks.credentials(move |url, username_from_url, allowed_types| {
    remote_credentials(&config, url, username_from_url, allowed_types)
  });
  Ok(callbacks)
}

fn remote_credentials(
  config: &Config,
  url: &str,
  username_from_url: Option<&str>,
  allowed_types: CredentialType,
) -> std::result::Result<Cred, git2::Error> {
  let userpass_error = if allowed_types.is_user_pass_plaintext() {
    if let Ok(credential) = Cred::credential_helper(config, url, username_from_url) {
      return Ok(credential);
    }
    match credential_from_git_credential_fill(url, username_from_url) {
      Ok(credential) => return Ok(credential),
      Err(error) => Some(error),
    }
  } else {
    None
  };

  if allowed_types.is_ssh_key()
    && let Some(username) = username_from_url
    && let Ok(credential) = Cred::ssh_key_from_agent(username)
  {
    return Ok(credential);
  }

  if allowed_types.is_username()
    && let Some(username) = username_from_url
  {
    return Cred::username(username);
  }

  if allowed_types.is_default() {
    return Cred::default();
  }

  Err(
    userpass_error
      .unwrap_or_else(|| git2::Error::from_str("no supported git credentials available")),
  )
}

fn credential_from_git_credential_fill(
  url: &str,
  username_from_url: Option<&str>,
) -> std::result::Result<Cred, git2::Error> {
  let mut last_error = None;
  for command in git_credential_fill_commands() {
    match credential_from_git_credential_fill_command(Path::new(command), url, username_from_url) {
      Ok(credential) => return Ok(credential),
      Err(error) => last_error = Some(error),
    }
  }

  Err(last_error.unwrap_or_else(|| git2::Error::from_str("git credential fill is not available")))
}

#[cfg(target_os = "macos")]
fn git_credential_fill_commands() -> &'static [&'static str] {
  &["git", "/usr/bin/git"]
}

#[cfg(not(target_os = "macos"))]
fn git_credential_fill_commands() -> &'static [&'static str] {
  &["git"]
}

fn credential_from_git_credential_fill_command(
  command: &Path,
  url: &str,
  username_from_url: Option<&str>,
) -> std::result::Result<Cred, git2::Error> {
  let mut child = Command::new(command)
    .args(["credential", "fill"])
    .env("GIT_TERMINAL_PROMPT", "0")
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .map_err(|error| git2::Error::from_str(&format!("run git credential fill: {error}")))?;

  {
    let Some(stdin) = child.stdin.as_mut() else {
      return Err(git2::Error::from_str("open git credential fill stdin"));
    };
    writeln!(stdin, "url={url}")
      .map_err(|error| git2::Error::from_str(&format!("write git credential url: {error}")))?;
    if let Some(username) = username_from_url {
      writeln!(stdin, "username={username}").map_err(|error| {
        git2::Error::from_str(&format!("write git credential username: {error}"))
      })?;
    }
  }

  let output = child
    .wait_with_output()
    .map_err(|error| git2::Error::from_str(&format!("wait for git credential fill: {error}")))?;
  if !output.status.success() {
    return Err(git2::Error::from_str(&format!(
      "git credential fill failed: {}",
      command_output_details(&output)
    )));
  }

  let Some((username, password)) = parse_git_credential_output(&output.stdout) else {
    return Err(git2::Error::from_str(
      "git credential fill did not return username and password",
    ));
  };

  Cred::userpass_plaintext(&username, &password)
}

fn parse_git_credential_output(output: &[u8]) -> Option<(String, String)> {
  let mut username = None;
  let mut password = None;
  for line in output.split(|byte| *byte == b'\n') {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    let Some(separator) = line.iter().position(|byte| *byte == b'=') else {
      continue;
    };
    let key = &line[..separator];
    let value = &line[separator + 1..];
    let Ok(value) = String::from_utf8(value.to_vec()) else {
      continue;
    };
    match key {
      b"username" => username = Some(value),
      b"password" => password = Some(value),
      _ => {}
    }
  }

  Some((username?, password?))
}

fn command_output_details(output: &std::process::Output) -> String {
  let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
  let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
  [stderr, stdout]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join("\n")
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::test_support::TempDir;
  use git2::{ConfigLevel, CredentialType};

  fn test_config(prefix: &str) -> (TempDir, Config) {
    let temp_dir = TempDir::new(prefix);
    let mut config = Config::new().expect("create config");
    config
      .add_file(&temp_dir.path.join("config"), ConfigLevel::App, false)
      .expect("add config file");
    (temp_dir, config)
  }

  #[cfg(not(windows))]
  #[test]
  fn remote_credentials_use_configured_http_helper() {
    let (_temp_dir, mut config) = test_config("remote-auth-helper");
    config
      .set_str(
        "credential.helper",
        "!f() { echo username=joris; echo password=token; }; f",
      )
      .expect("set credential helper");

    remote_credentials(
      &config,
      "https://github.com/reviu-dev/reviu.git",
      None,
      CredentialType::USER_PASS_PLAINTEXT,
    )
    .expect("read credentials from helper");
  }

  #[cfg(unix)]
  #[test]
  fn remote_credentials_fall_back_to_git_credential_fill() {
    let temp_dir = TempDir::new("remote-auth-git-fill");
    let script_path = temp_dir.path.join("git");
    let input_path = temp_dir.path.join("input");
    std::fs::write(
      &script_path,
      format!(
        "#!/bin/sh\nif [ \"$1\" = credential ] && [ \"$2\" = fill ]; then\n  cat > '{}'\n  echo username=cli-user\n  echo password=cli-token\n  exit 0\nfi\nexit 1\n",
        input_path.display()
      ),
    )
    .expect("write fake git");
    make_executable(&script_path);

    credential_from_git_credential_fill_command(
      &script_path,
      "https://github.com/reviu-dev/reviu.git",
      Some("url-user"),
    )
    .expect("read credentials from git credential fill");

    let input = std::fs::read_to_string(&input_path).expect("read credential input");
    assert!(input.contains("url=https://github.com/reviu-dev/reviu.git"));
    assert!(input.contains("username=url-user"));
  }

  #[test]
  fn remote_credentials_fail_when_no_allowed_type_is_supported() {
    let (_temp_dir, config) = test_config("remote-auth-no-type");

    let error = match remote_credentials(
      &config,
      "https://github.com/reviu-dev/reviu.git",
      None,
      CredentialType::empty(),
    ) {
      Ok(_) => panic!("unsupported credential type should fail"),
      Err(error) => error,
    };

    assert!(
      error
        .message()
        .contains("no supported git credentials available")
    );
  }

  #[test]
  fn parse_git_credential_output_reads_username_and_password() {
    assert_eq!(
      parse_git_credential_output(b"protocol=https\nusername=joris\npassword=token\n"),
      Some(("joris".to_string(), "token".to_string()))
    );
  }

  #[cfg(unix)]
  fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = std::fs::metadata(path)
      .expect("read script metadata")
      .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).expect("chmod script");
  }
}
