use std::ffi::OsString;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(windows)]
use std::os::windows::net::{UnixListener, UnixStream};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use anyhow::{Context as _, Result};
use gpui::{
  App, AppContext as _, Context, Entity, FocusHandle, Focusable as _, InteractiveElement as _,
  IntoElement, ParentElement as _, Render, Styled as _, Task, Window, div, px,
};
use gpui_component::{
  ActiveTheme as _, Sizable as _, WindowExt as _,
  dialog::{DialogDescription, DialogFooter, DialogHeader, DialogTitle},
  input::{Input, InputEvent, InputState},
  v_flex,
};
use ui::{Button, ButtonVariants as _};
use zeroize::Zeroize;

use crate::workspace_window::WorkspaceWindow;

const ASKPASS_ARG: &str = "--askpass";
const ASKPASS_SOCKET_ENV: &str = "REVIU_ASKPASS_SOCKET";
const ASKPASS_TIMEOUT: Duration = Duration::from_secs(60);
const RESPONSE_OK: u8 = 1;
const RESPONSE_CANCELLED: u8 = 0;
const MAX_PROMPT_BYTES: usize = 64 * 1024;

struct AskpassRequest {
  prompt: String,
  response: mpsc::Sender<Option<String>>,
}

pub(crate) struct GitAskpassServer {
  directory: PathBuf,
  socket_path: PathBuf,
  program_path: PathBuf,
  _task: Task<()>,
}

impl gpui::Global for GitAskpassServer {}

impl Drop for GitAskpassServer {
  fn drop(&mut self) {
    let _ = fs::remove_file(&self.socket_path);
    let _ = fs::remove_dir_all(&self.directory);
  }
}

impl GitAskpassServer {
  fn config(&self) -> git::AskpassConfig {
    git::AskpassConfig {
      program: self.program_path.clone(),
      #[cfg(windows)]
      socket: Some(self.socket_path.clone()),
      #[cfg(not(windows))]
      socket: None,
    }
  }
}

pub fn handle_askpass_invocation() -> bool {
  let Some(invocation) = AskpassInvocation::from_process() else {
    return false;
  };

  let exit_code = match askpass_client(&invocation.socket, invocation.prompt) {
    Ok(()) => 0,
    Err(error) => {
      eprintln!("askpass failed: {error:#}");
      1
    }
  };
  process::exit(exit_code);
}

pub(crate) fn install(cx: &mut App) {
  if cx.has_global::<GitAskpassServer>() {
    return;
  }

  match start_server(cx) {
    Ok(server) => {
      git::configure_askpass(Some(server.config()));
      cx.set_global(server);
    }
    Err(error) => {
      log::warn!("could not start git askpass: {error:#}");
      git::configure_askpass(None);
    }
  }
}

fn start_server(cx: &mut App) -> Result<GitAskpassServer> {
  let directory = askpass_directory()?;
  fs::create_dir_all(&directory).with_context(|| format!("creating {directory:?}"))?;
  let socket_path = directory.join("askpass.sock");
  let _ = fs::remove_file(&socket_path);
  let listener = UnixListener::bind(&socket_path).context("creating askpass socket")?;
  let (request_sender, request_receiver) = async_channel::unbounded::<AskpassRequest>();

  thread::Builder::new()
    .name("GitAskpassSocket".to_string())
    .spawn(move || {
      for stream in listener.incoming() {
        match stream {
          Ok(stream) => {
            let request_sender = request_sender.clone();
            let _ = thread::Builder::new()
              .name("GitAskpassRequest".to_string())
              .spawn(move || handle_stream(stream, request_sender));
          }
          Err(error) => {
            log::warn!("askpass socket accept failed: {error}");
            break;
          }
        }
      }
    })
    .context("starting askpass socket thread")?;

  let task = cx.spawn(async move |cx| {
    while let Ok(request) = request_receiver.recv().await {
      cx.update(|cx| {
        WorkspaceWindow::with_window(cx, move |window, cx| {
          open_dialog(request, window, cx);
        });
      });
    }
  });

  Ok(GitAskpassServer {
    program_path: askpass_program(&directory, &socket_path)?,
    directory,
    socket_path,
    _task: task,
  })
}

fn askpass_directory() -> Result<PathBuf> {
  let timestamp = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .context("system clock is before the Unix epoch")?
    .as_nanos();
  Ok(std::env::temp_dir().join(format!("reviu-askpass-{}-{timestamp}", process::id())))
}

#[cfg(unix)]
fn askpass_program(directory: &Path, socket_path: &Path) -> Result<PathBuf> {
  let current_exe = std::env::current_exe().context("finding current Reviu executable")?;
  let script_path = directory.join("askpass.sh");
  let script = format!(
    "#!/bin/sh\nexec {} {} {} \"$@\" 2>/dev/null\n",
    sh_quote(&current_exe),
    ASKPASS_ARG,
    sh_quote(socket_path)
  );
  fs::write(&script_path, script).with_context(|| format!("writing {script_path:?}"))?;
  let mut permissions = fs::metadata(&script_path)?.permissions();
  permissions.set_mode(0o700);
  fs::set_permissions(&script_path, permissions)
    .with_context(|| format!("marking {script_path:?} executable"))?;
  Ok(script_path)
}

#[cfg(windows)]
fn askpass_program(_: &Path, _: &Path) -> Result<PathBuf> {
  let current_exe = std::env::current_exe().context("finding current Reviu executable")?;
  let helper = current_exe.with_file_name("reviu-askpass.exe");
  if helper.is_file() {
    Ok(helper)
  } else {
    Ok(current_exe)
  }
}

#[cfg(unix)]
fn sh_quote(path: &Path) -> String {
  format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

fn handle_stream(mut stream: UnixStream, request_sender: async_channel::Sender<AskpassRequest>) {
  let response =
    receive_prompt(&mut stream).and_then(|prompt| request_password(prompt, request_sender));
  match response {
    Ok(Some(mut answer)) => {
      let _ = stream.write_all(&[RESPONSE_OK]);
      let _ = stream.write_all(answer.as_bytes());
      answer.zeroize();
    }
    Ok(None) => {
      let _ = stream.write_all(&[RESPONSE_CANCELLED]);
    }
    Err(error) => {
      log::warn!("askpass request failed: {error:#}");
      let _ = stream.write_all(&[RESPONSE_CANCELLED]);
    }
  }
}

fn receive_prompt(stream: &mut UnixStream) -> Result<String> {
  let mut prompt = Vec::new();
  let mut byte = [0_u8; 1];
  while prompt.len() < MAX_PROMPT_BYTES {
    let read = stream.read(&mut byte).context("reading askpass prompt")?;
    if read == 0 || byte[0] == 0 {
      break;
    }
    prompt.push(byte[0]);
  }
  String::from_utf8(prompt).context("askpass prompt was not UTF-8")
}

fn request_password(
  prompt: String,
  request_sender: async_channel::Sender<AskpassRequest>,
) -> Result<Option<String>> {
  let (response_sender, response_receiver) = mpsc::channel();
  request_sender
    .try_send(AskpassRequest {
      prompt,
      response: response_sender,
    })
    .context("sending askpass request to the app")?;
  response_receiver
    .recv_timeout(ASKPASS_TIMEOUT + Duration::from_secs(5))
    .context("waiting for askpass response")
}

struct AskpassInvocation {
  socket: PathBuf,
  prompt: String,
}

impl AskpassInvocation {
  fn from_process() -> Option<Self> {
    Self::from_args_and_env(
      std::env::args_os().skip(1),
      std::env::var_os(ASKPASS_SOCKET_ENV),
    )
  }

  fn from_args_and_env<I>(args: I, socket_env: Option<OsString>) -> Option<Self>
  where
    I: IntoIterator<Item = OsString>,
  {
    let mut args = args.into_iter();
    let first = args.next()?;
    if first == ASKPASS_ARG {
      let socket = args.next().map(PathBuf::from)?;
      return Some(Self {
        socket,
        prompt: join_prompt(args),
      });
    }

    socket_env.map(|socket| {
      let prompt = std::iter::once(first).chain(args).collect::<Vec<_>>();
      Self {
        socket: PathBuf::from(socket),
        prompt: join_prompt(prompt),
      }
    })
  }
}

fn join_prompt<I>(parts: I) -> String
where
  I: IntoIterator<Item = OsString>,
{
  parts
    .into_iter()
    .map(|part| part.to_string_lossy().into_owned())
    .collect::<Vec<_>>()
    .join(" ")
}

fn askpass_client(socket: &Path, prompt: String) -> Result<()> {
  let mut stream =
    UnixStream::connect(socket).with_context(|| format!("connecting to {socket:?}"))?;
  stream
    .write_all(prompt.as_bytes())
    .context("writing askpass prompt")?;
  stream.write_all(&[0]).context("finishing askpass prompt")?;

  let mut response = Vec::new();
  stream
    .read_to_end(&mut response)
    .context("reading askpass response")?;
  match response.split_first() {
    Some((&RESPONSE_OK, answer)) => {
      std::io::stdout()
        .write_all(answer)
        .context("writing askpass answer")?;
      Ok(())
    }
    _ => anyhow::bail!("askpass was cancelled"),
  }
}

struct GitAskpassDialog {
  prompt: String,
  input: Entity<InputState>,
  response: Option<mpsc::Sender<Option<String>>>,
  _timeout_task: Task<()>,
}

impl Drop for GitAskpassDialog {
  fn drop(&mut self) {
    if let Some(response) = self.response.take() {
      let _ = response.send(None);
    }
  }
}

impl GitAskpassDialog {
  fn new(request: AskpassRequest, window: &mut Window, cx: &mut Context<Self>) -> Self {
    let masked = prompt_is_secret(&request.prompt);
    let input = cx.new(|cx| {
      let mut input = InputState::new(window, cx).placeholder("Answer");
      input.set_masked(masked, window, cx);
      input
    });
    cx.subscribe_in(
      &input,
      window,
      |this, _input, event: &InputEvent, window, cx| {
        if matches!(event, InputEvent::PressEnter { .. }) {
          this.confirm(window, cx);
        }
      },
    )
    .detach();

    let window_handle = window.window_handle();
    let timeout_task = cx.spawn(async move |this, cx| {
      cx.background_executor().timer(ASKPASS_TIMEOUT).await;
      let _ = cx.update_window(window_handle, |_, window, cx| {
        let should_close = this
          .update(cx, |this, _| this.respond(None))
          .unwrap_or(false);
        if should_close {
          window.close_dialog(cx);
        }
      });
    });

    Self {
      prompt: request.prompt,
      input,
      response: Some(request.response),
      _timeout_task: timeout_task,
    }
  }

  fn input_focus_handle(&self, cx: &App) -> FocusHandle {
    self.input.read(cx).focus_handle(cx)
  }

  fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let mut value = self.input.update(cx, |input, cx| {
      let value = input.value().to_string();
      input.set_value("", window, cx);
      value
    });
    let response = Some(std::mem::take(&mut value));
    value.zeroize();
    if self.respond(response) {
      window.close_dialog(cx);
    }
  }

  fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.respond(None) {
      window.close_dialog(cx);
    }
  }

  fn respond(&mut self, response: Option<String>) -> bool {
    let Some(sender) = self.response.take() else {
      return false;
    };
    let _ = sender.send(response);
    true
  }
}

impl Render for GitAskpassDialog {
  fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme().clone();
    div()
      .id("git-askpass-dialog")
      .flex()
      .flex_col()
      .child(
        DialogHeader::new()
          .p_4()
          .child(DialogTitle::new().child("Git needs a response"))
          .child(
            DialogDescription::new()
              .child("Reviu will send this answer to git or ssh for the current operation."),
          ),
      )
      .child(
        v_flex()
          .px_4()
          .pb_4()
          .gap_2()
          .child(
            div()
              .text_xs()
              .text_color(theme.muted_foreground)
              .child(self.prompt.clone()),
          )
          .child(Input::new(&self.input).w_full()),
      )
      .child(
        DialogFooter::new()
          .px_4()
          .pb_4()
          .pt_1()
          .justify_end()
          .child(
            Button::new("cancel-git-askpass")
              .label("Cancel")
              .outline()
              .small()
              .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx))),
          )
          .child(
            Button::new("confirm-git-askpass")
              .label("Continue")
              .primary()
              .small()
              .on_click(cx.listener(|this, _, window, cx| this.confirm(window, cx))),
          ),
      )
  }
}

fn open_dialog(request: AskpassRequest, window: &mut Window, cx: &mut App) {
  let dialog = cx.new(|cx| GitAskpassDialog::new(request, window, cx));
  let dialog_for_overlay = dialog.clone();
  let dialog_for_focus = dialog.clone();
  window.open_dialog(cx, move |overlay, _, _| {
    overlay.p_0().w(px(420.0)).child(dialog_for_overlay.clone())
  });
  window.on_next_frame(move |window, cx| {
    window.focus(&dialog_for_focus.read(cx).input_focus_handle(cx), cx);
  });
}

fn prompt_is_secret(prompt: &str) -> bool {
  let prompt = prompt.to_ascii_lowercase();
  !(prompt.contains("username") || prompt.contains("yes/no"))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn parses_unix_wrapper_invocation() {
    let invocation = AskpassInvocation::from_args_and_env(
      [
        OsString::from("--askpass"),
        OsString::from("/tmp/reviu.sock"),
        OsString::from("Password for 'https://github.com':"),
      ],
      None,
    )
    .expect("invocation");

    assert_eq!(invocation.socket, PathBuf::from("/tmp/reviu.sock"));
    assert_eq!(invocation.prompt, "Password for 'https://github.com':");
  }

  #[test]
  fn parses_windows_ssh_askpass_invocation() {
    let invocation = AskpassInvocation::from_args_and_env(
      [OsString::from(
        "Enter passphrase for key '/home/me/.ssh/id_ed25519':",
      )],
      Some(OsString::from("C:/Temp/reviu.sock")),
    )
    .expect("invocation");

    assert_eq!(invocation.socket, PathBuf::from("C:/Temp/reviu.sock"));
    assert!(invocation.prompt.contains("Enter passphrase"));
  }

  #[test]
  fn masks_secret_prompts_but_not_usernames_or_host_key_questions() {
    assert!(prompt_is_secret("Password for 'https://github.com':"));
    assert!(prompt_is_secret("Enter passphrase for key '/a/b':"));
    assert!(!prompt_is_secret("Username for 'https://github.com':"));
    assert!(!prompt_is_secret(
      "Are you sure you want to continue connecting (yes/no/[fingerprint])?"
    ));
  }
}
