use std::path::PathBuf;
use std::time::Duration;

use gpui::{App, AppContext as _, Global, Task, Window, px};
use gpui_component::{Theme, ThemeMode, WindowExt as _, notification::Notification};

use crate::config::AppSettings;
use crate::config_file::ConfigMonitor;
use crate::shortcuts::{self, ShortcutOverrides};
use crate::workspace_window::WorkspaceWindow;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy)]
pub(crate) enum ConfigKind {
  Settings,
  Keybindings,
}

impl ConfigKind {
  const ALL: [Self; 2] = [Self::Settings, Self::Keybindings];

  pub(crate) fn path(self) -> PathBuf {
    match self {
      Self::Settings => crate::settings_file::settings_file_path(),
      Self::Keybindings => crate::keybindings_file::keybindings_file_path(),
    }
  }

  pub(crate) fn name(self) -> &'static str {
    match self {
      Self::Settings => "settings.json",
      Self::Keybindings => "keybindings.json",
    }
  }
}

#[derive(Default)]
struct FileState {
  observed: Option<String>,
  error: Option<String>,
}

#[derive(Default)]
struct ConfigFiles {
  settings: FileState,
  keybindings: FileState,
  revision: u64,
}

impl Global for ConfigFiles {}

impl ConfigFiles {
  fn file_mut(&mut self, kind: ConfigKind) -> &mut FileState {
    match kind {
      ConfigKind::Settings => &mut self.settings,
      ConfigKind::Keybindings => &mut self.keybindings,
    }
  }

  fn revision(cx: &App) -> u64 {
    cx.try_global::<Self>().map_or(0, |files| files.revision)
  }
}

struct ConfigError;

fn notify_error(kind: ConfigKind, error: Option<String>, cx: &mut App) {
  let state = cx.default_global::<ConfigFiles>().file_mut(kind);
  if state.error == error {
    return;
  }
  if let Some(error) = &error {
    log::warn!("{}: {error}", kind.name());
  }
  state.error = error;
  WorkspaceWindow::with_window(cx, move |window, cx| {
    let error = cx
      .default_global::<ConfigFiles>()
      .file_mut(kind)
      .error
      .clone();
    if let Some(error) = error {
      window.push_notification(
        Notification::new()
          .id1::<ConfigError>(kind.name())
          .title(format!("Could not update {}", kind.name()))
          .message(format!("Your current configuration is unchanged. {error}"))
          .autohide(false),
        cx,
      );
    } else {
      window.remove_notification1::<ConfigError>(kind.name(), cx);
    }
  });
}

pub(crate) fn save_failed(kind: ConfigKind, error: anyhow::Error, cx: &mut App) {
  cx.default_global::<ConfigFiles>().file_mut(kind).observed = None;
  notify_error(kind, Some(format!("{error:#}")), cx);
}

pub(crate) fn saved(kind: ConfigKind, text: String, cx: &mut App) {
  let files = cx.default_global::<ConfigFiles>();
  files.revision += 1;
  files.file_mut(kind).observed = None;
  accept(kind, Ok(text), cx);
}

fn accept_snapshot(revision: u64, results: [Result<String, String>; 2], cx: &mut App) -> bool {
  if revision != ConfigFiles::revision(cx) {
    return false;
  }
  for (kind, result) in ConfigKind::ALL.into_iter().zip(results) {
    accept(kind, result, cx);
  }
  true
}

fn accept(kind: ConfigKind, result: Result<String, String>, cx: &mut App) {
  let text = match result {
    Ok(text) => text,
    Err(error) => {
      cx.default_global::<ConfigFiles>().file_mut(kind).observed = None;
      notify_error(kind, Some(error), cx);
      return;
    }
  };
  let state = cx.default_global::<ConfigFiles>().file_mut(kind);
  if state.observed.as_ref() == Some(&text) {
    return;
  }
  state.observed = Some(text.clone());
  let result = match kind {
    ConfigKind::Settings => crate::settings_file::parse(&text).map(|settings| {
      if cx.try_global::<AppSettings>() != Some(&settings) {
        apply_settings(settings, None, cx);
      }
    }),
    ConfigKind::Keybindings => crate::keybindings_file::parse(&text).map(|entries| {
      let overrides = ShortcutOverrides::from_entries(entries);
      if ShortcutOverrides::get(cx) != overrides {
        cx.set_global(overrides);
        shortcuts::install_workspace_shortcuts(cx);
        cx.refresh_windows();
      }
    }),
  };
  notify_error(kind, result.err().map(|error| error.to_string()), cx);
}

pub(crate) fn apply_settings(settings: AppSettings, window: Option<&mut Window>, cx: &mut App) {
  let previous = cx.try_global::<AppSettings>().copied();
  cx.set_global(settings);
  cx.set_global(settings.find_options());
  cx.set_global(editor::EditorSettings {
    soft_wrap: settings.soft_wrap,
    git_gutter: settings.git_gutter,
  });
  editor::set_indent_rainbow_enabled(settings.indent_rainbow);
  if cx.has_global::<Theme>() {
    Theme::global_mut(cx).font_size = px(settings.font_size);
    if settings.auto_switch_theme {
      Theme::sync_system_appearance(window, cx);
    } else {
      let mode = if settings.dark_mode {
        ThemeMode::Dark
      } else {
        ThemeMode::Light
      };
      Theme::change(mode, window, cx);
    }
  }
  if previous.map(|settings| settings.menu_bar_icon) != Some(settings.menu_bar_icon) {
    #[cfg(not(test))]
    {
      crate::status_bar::set_status_bar_enabled(
        settings.menu_bar_icon,
        crate::workspace::STATUS_BAR_ICON_PNG,
      );
      if settings.menu_bar_icon
        && cx.has_global::<crate::github_notifications::GithubNotificationsStore>()
      {
        let notifications = crate::github_notifications::GithubNotificationsStore::list(cx);
        let unread = crate::github_notifications::GithubNotificationsStore::unread_count(cx);
        crate::status_bar::update_status_bar(unread, &notifications);
      }
    }
  }
  cx.refresh_windows();
}

pub(crate) fn watch(cx: &mut App) -> Task<()> {
  let paths = ConfigKind::ALL.map(ConfigKind::path);
  let (sender, receiver) = async_channel::bounded(1);
  let watched_paths = paths.clone();
  let initialize = cx.background_spawn(async move {
    let mut monitor = ConfigMonitor::new(sender)?;
    monitor.refresh(&watched_paths)?;
    anyhow::Ok(monitor)
  });
  watch_changes(paths, receiver, initialize, cx)
}

fn watch_changes(
  paths: [PathBuf; 2],
  receiver: async_channel::Receiver<()>,
  initialize: Task<anyhow::Result<ConfigMonitor>>,
  cx: &mut App,
) -> Task<()> {
  cx.spawn(async move |cx| {
    let mut monitor = match initialize.await {
      Ok(monitor) => monitor,
      Err(error) => {
        cx.update(|cx| save_failed(ConfigKind::Settings, error, cx));
        return;
      }
    };
    let mut initial = true;
    loop {
      if !initial {
        if receiver.recv().await.is_err() {
          break;
        }
        cx.background_executor()
          .timer(Duration::from_millis(100))
          .await;
        while receiver.try_recv().is_ok() {}
      }
      initial = false;
      let revision = cx.update(|cx| ConfigFiles::revision(cx));
      let paths = paths.clone();
      let (returned_monitor, results, watch_error) = cx
        .background_spawn(async move {
          let watch_error = monitor.refresh(&paths).err();
          let results = paths.map(|path| {
            std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))
          });
          (monitor, results, watch_error)
        })
        .await;
      monitor = returned_monitor;
      let stale = cx.update(|cx| {
        if !accept_snapshot(revision, results, cx) {
          return true;
        }
        if let Some(error) = watch_error {
          save_failed(ConfigKind::Settings, error, cx);
        }
        false
      });
      if stale {
        initial = true;
      }
    }
  })
}
