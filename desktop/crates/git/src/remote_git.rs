//! Fetch, pull and push run the user's own git, so whatever signs them in from a terminal
//! signs Reviu in too: Git Credential Manager, credential helpers, ssh-agent and ssh config.

use std::ffi::OsStr;
use std::fmt;
use std::path::Path;
use std::process::{Output, Stdio};

use anyhow::{Context, Result};

/// A remote refused the user, or git found no way to sign them in. That is about their
/// setup, not a bug, and the fix is on their side.
#[derive(Debug)]
pub struct AuthenticationError {
  message: String,
}

impl AuthenticationError {
  pub fn new(message: impl Into<String>) -> Self {
    Self {
      message: message.into(),
    }
  }
}

impl fmt::Display for AuthenticationError {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    formatter.write_str(&self.message)
  }
}

impl std::error::Error for AuthenticationError {}

pub fn is_authentication_error(error: &anyhow::Error) -> bool {
  error
    .chain()
    .any(|cause| cause.downcast_ref::<AuthenticationError>().is_some())
}

/// What git and ssh print when no credential could be found or the remote refused it.
const AUTHENTICATION_FAILURES: &[&str] = &[
  "authentication failed",
  "could not read username",
  "could not read password",
  "terminal prompts disabled",
  "invalid username or password",
  "the requested url returned error: 401",
  "the requested url returned error: 403",
  "permission denied (publickey",
  "host key verification failed",
];

fn is_authentication_failure(stderr: &str) -> bool {
  let stderr = stderr.to_lowercase();
  AUTHENTICATION_FAILURES
    .iter()
    .any(|failure| stderr.contains(failure))
}

pub(crate) fn run_remote_git<I, S>(repo_root: &Path, args: I) -> Result<Output>
where
  I: IntoIterator<Item = S>,
  S: AsRef<OsStr>,
{
  let mut command = gpui_util::new_std_command("git");
  command
    .current_dir(repo_root)
    // Disabled to stop malicious actors from running arbitrary commands via fsmonitor hooks.
    .args(["-c", "core.fsmonitor=false"])
    .arg("--no-optional-locks")
    .arg("--no-pager")
    .args(args)
    // On Windows git runs behind a hidden console: a prompt there would wait forever.
    // Git Credential Manager signs in through its own window, which this does not stop.
    .env("GIT_TERMINAL_PROMPT", "0")
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
  if !has_custom_ssh_command(repo_root) {
    // Same reason for ssh: a passphrase or host key question must fail, not hang.
    command.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
  }

  let output = command.output().context("run git")?;
  if output.status.success() {
    return Ok(output);
  }

  let details = command_output_details(&output);
  let details = if details.is_empty() {
    format!("git {}", output.status)
  } else {
    details
  };
  if is_authentication_failure(&details) {
    return Err(AuthenticationError::new(details).into());
  }
  Err(anyhow::anyhow!(details))
}

fn has_custom_ssh_command(repo_root: &Path) -> bool {
  if std::env::var_os("GIT_SSH_COMMAND").is_some() || std::env::var_os("GIT_SSH").is_some() {
    return true;
  }
  git2::Repository::open(repo_root)
    .and_then(|repo| repo.config())
    .and_then(|mut config| config.snapshot())
    .is_ok_and(|config| config.get_str("core.sshCommand").is_ok())
}

fn command_output_details(output: &Output) -> String {
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

  #[test]
  fn git_and_ssh_sign_in_failures_are_authentication_errors() {
    for stderr in [
      "remote: Invalid username or password.\nfatal: Authentication failed for 'https://github.com/a/b.git/'",
      "fatal: could not read Username for 'https://github.com': terminal prompts disabled",
      "git@github.com: Permission denied (publickey).\nfatal: Could not read from remote repository.",
      "Host key verification failed.",
    ] {
      assert!(is_authentication_failure(stderr), "{stderr}");
    }
    assert!(!is_authentication_failure(
      "! [rejected] main -> main (fetch first)"
    ));
  }

  /// Answers every request with a Basic auth challenge, as a private HTTPS remote does.
  fn start_auth_challenge_server() -> u16 {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind challenge server");
    let port = listener.local_addr().expect("server address").port();
    std::thread::spawn(move || {
      for stream in listener.incoming() {
        let Ok(mut stream) = stream else {
          continue;
        };
        let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
        let mut line = String::new();
        while reader.read_line(&mut line).is_ok_and(|read| read > 2) {
          line.clear();
        }
        let _ = stream.write_all(
          b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"test\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
      }
    });
    port
  }

  #[test]
  fn a_remote_asking_for_credentials_fails_at_once_as_an_authentication_error() {
    let port = start_auth_challenge_server();
    let repo = crate::test_support::TempRepo::init("remote-git-auth");
    crate::test_support::commit_text_file(&repo.path, Path::new("README.md"), "hello", "initial");
    // An empty helper clears the machine's own, so no credential manager answers for the test.
    git2::Repository::open(&repo.path)
      .and_then(|repo| repo.config())
      .and_then(|mut config| config.set_str("credential.helper", ""))
      .expect("clear credential helpers");

    let error = run_remote_git(
      &repo.path,
      [
        "push",
        &format!("http://127.0.0.1:{port}/repo.git"),
        "HEAD:refs/heads/main",
      ],
    )
    .expect_err("push without credentials fails");

    assert!(is_authentication_error(&error), "error: {error:#}");
  }

  #[test]
  fn an_authentication_error_is_recognised_through_context() {
    let error =
      anyhow::Error::new(AuthenticationError::new("Authentication failed")).context("push");
    assert!(is_authentication_error(&error));
    assert!(!is_authentication_error(&anyhow::anyhow!("remote hung up")));
  }
}
