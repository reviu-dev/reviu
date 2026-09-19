use super::*;
use crate::config_file::TestConfig;
use crate::shortcuts::ShortcutId;
use gpui::{Context, IntoElement, Render, TestAppContext, div};
use std::cell::Cell;
use std::rc::Rc;

fn reload(kind: ConfigKind, cx: &mut App) {
  accept(
    kind,
    std::fs::read_to_string(kind.path()).map_err(|error| error.to_string()),
    cx,
  );
}

#[gpui::test]
fn settings_reload_applies_effects_and_ignores_own_write_echoes(cx: &mut TestAppContext) {
  let _config = TestConfig::new();
  let path = ConfigKind::Settings.path();
  cx.update(|cx| {
    gpui_component::init(cx);
    cx.set_global(AppSettings::default());
  });
  let applications = Rc::new(Cell::new(0));
  let _subscription = cx.update(|cx| {
    cx.observe_global::<AppSettings>({
      let applications = applications.clone();
      move |_| applications.set(applications.get() + 1)
    })
  });
  std::fs::write(
    &path,
    r#"{"font_size":20,"auto_switch_theme":false,"dark_mode":true,"find_regex":true,"soft_wrap":true}"#,
  )
  .expect("write settings");
  cx.update(|cx| reload(ConfigKind::Settings, cx));
  cx.run_until_parked();
  cx.update(|cx| {
    assert_eq!(AppSettings::get(cx).font_size, 20.0);
    assert_eq!(Theme::global(cx).font_size, px(20.0));
    assert!(Theme::global(cx).mode.is_dark());
    assert!(cx.global::<editor::SearchOptions>().regex);
    assert!(AppSettings::get(cx).soft_wrap);
    assert!(editor::EditorSettings::get(cx).soft_wrap);
    AppSettings::update(cx, |settings| settings.font_size = 18.0);
  });
  cx.run_until_parked();
  assert_eq!(applications.get(), 2);
  cx.update(|cx| reload(ConfigKind::Settings, cx));
  cx.run_until_parked();
  assert_eq!(
    applications.get(),
    2,
    "our own filesystem echo must not reapply settings"
  );
}

#[gpui::test]
fn saving_merges_external_changes_before_the_watcher_and_refuses_invalid_files(cx: &mut App) {
  let _config = TestConfig::new();
  cx.set_global(AppSettings::default());
  let path = ConfigKind::Settings.path();
  std::fs::write(&path, r#"{"font_size":21,"future_field":42}"#).expect("external edit");
  AppSettings::update(cx, |settings| settings.hide_whitespace = true);
  assert_eq!(AppSettings::get(cx).font_size, 21.0);
  assert!(AppSettings::get(cx).hide_whitespace);
  assert!(
    std::fs::read_to_string(&path)
      .expect("read")
      .contains("future_field")
  );
  std::fs::write(&path, "{").expect("invalid edit");
  AppSettings::update(cx, |settings| settings.font_size = 12.0);
  assert_eq!(AppSettings::get(cx).font_size, 21.0);
  assert_eq!(std::fs::read_to_string(&path).expect("read"), "{");
  assert!(cx.global::<ConfigFiles>().settings.error.is_some());
}

#[gpui::test]
fn keybindings_reload_changes_active_bindings_and_preserves_them_on_errors(
  cx: &mut TestAppContext,
) {
  let _config = TestConfig::new();
  let path = ConfigKind::Keybindings.path();
  cx.update(|cx| {
    cx.set_global(ShortcutOverrides::default());
    shortcuts::install_workspace_shortcuts(cx);
  });
  let cx = cx.add_empty_window();
  for key in ["ctrl-alt-8", "ctrl-alt-9"] {
    std::fs::write(&path, format!(r#"{{"show_command_palette":"{key}"}}"#)).expect("edit bindings");
    cx.update(|window, cx| {
      reload(ConfigKind::Keybindings, cx);
      let context =
        gpui::KeyContext::parse(&shortcuts::current_workspace_key_context(cx)).expect("context");
      let binding = window
        .highest_precedence_binding_for_action_in_context(&crate::ShowCommandPalette, context)
        .expect("binding");
      assert_eq!(
        binding.keystrokes().first().expect("keystroke").inner(),
        &gpui::Keystroke::parse(key).expect("key")
      );
    });
  }
  let generation = cx.update(|_, cx| shortcuts::current_workspace_key_context(cx));
  for content in ["{", "[]"] {
    std::fs::write(&path, content).expect("invalid edit");
    cx.update(|_, cx| {
      reload(ConfigKind::Keybindings, cx);
      assert_eq!(shortcuts::current_workspace_key_context(cx), generation);
      assert!(ShortcutOverrides::get(cx).contains(ShortcutId::ShowCommandPalette));
    });
  }
  std::fs::remove_file(&path).expect("remove");
  cx.update(|_, cx| {
    reload(ConfigKind::Keybindings, cx);
    assert_eq!(shortcuts::current_workspace_key_context(cx), generation);
  });
  std::fs::write(&path, "{}").expect("recreate");
  cx.update(|_, cx| {
    reload(ConfigKind::Keybindings, cx);
    assert!(!ShortcutOverrides::get(cx).contains(ShortcutId::ShowCommandPalette));
    assert!(cx.global::<ConfigFiles>().keybindings.error.is_none());
  });
}

#[gpui::test]
fn keybinding_save_merges_the_disk_and_does_not_rebind_on_echo(cx: &mut App) {
  let _config = TestConfig::new();
  cx.set_global(ShortcutOverrides::default());
  let path = ConfigKind::Keybindings.path();
  std::fs::write(&path, r#"{"commit_changes":"ctrl-alt-8"}"#).expect("external edit");
  shortcuts::set_shortcut_override(
    cx,
    ShortcutId::ShowCommandPalette,
    &gpui::Keystroke::parse("ctrl-alt-9").expect("key"),
  );
  assert!(ShortcutOverrides::get(cx).contains(ShortcutId::CommitChanges));
  let generation = shortcuts::current_workspace_key_context(cx);
  reload(ConfigKind::Keybindings, cx);
  assert_eq!(shortcuts::current_workspace_key_context(cx), generation);
  std::fs::write(&path, "{").expect("invalid edit");
  shortcuts::clear_shortcut_override(cx, ShortcutId::ShowCommandPalette);
  assert!(ShortcutOverrides::get(cx).contains(ShortcutId::ShowCommandPalette));
  assert_eq!(shortcuts::current_workspace_key_context(cx), generation);
}

#[gpui::test]
fn background_reload_uses_the_same_path_for_initial_load_and_changes(cx: &mut TestAppContext) {
  let _config = TestConfig::new();
  let paths = ConfigKind::ALL.map(ConfigKind::path);
  for path in &paths {
    std::fs::write(path, "{}").expect("initial file");
  }
  let (sender, receiver) = async_channel::bounded(1);
  let (native_sender, _native_receiver) = async_channel::bounded(1);
  let monitor = ConfigMonitor::new(native_sender).expect("monitor");
  let _task = cx.update(|cx| watch_changes(paths, receiver, Task::ready(Ok(monitor)), cx));
  cx.run_until_parked();
  cx.update(|cx| assert_eq!(AppSettings::get(cx), AppSettings::default()));
  for (text, expected) in [
    (r#"{"font_size":20}"#, 20.0),
    ("{", 20.0),
    (r#"{"font_size":22}"#, 22.0),
  ] {
    std::fs::write(ConfigKind::Settings.path(), text).expect("external edit");
    std::fs::write(
      ConfigKind::Keybindings.path(),
      r#"{"show_command_palette":"ctrl-alt-8"}"#,
    )
    .expect("external binding edit");
    sender.try_send(()).expect("signal change");
    cx.run_until_parked();
    cx.background_executor
      .advance_clock(Duration::from_millis(100));
    cx.run_until_parked();
    cx.update(|cx| {
      assert_eq!(AppSettings::get(cx).font_size, expected);
      assert!(ShortcutOverrides::get(cx).contains(ShortcutId::ShowCommandPalette));
    });
  }
}

#[gpui::test]
fn a_background_snapshot_cannot_undo_a_more_recent_ui_save(cx: &mut App) {
  let _config = TestConfig::new();
  cx.set_global(AppSettings::default());
  std::fs::write(ConfigKind::Settings.path(), "{}").expect("initial settings");
  let revision = ConfigFiles::revision(cx);
  let stale = [Ok("{}".to_string()), Ok("{}".to_string())];
  AppSettings::update(cx, |settings| settings.font_size = 23.0);
  assert!(!accept_snapshot(revision, stale, cx));
  assert_eq!(AppSettings::get(cx).font_size, 23.0);
}

struct Host;

impl Render for Host {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    use gpui::ParentElement as _;
    div().children(gpui_component::Root::render_notification_layer(window, cx))
  }
}

#[gpui::test]
fn invalid_then_same_valid_content_clears_the_notification_without_reapplying(
  cx: &mut TestAppContext,
) {
  let _config = TestConfig::new();
  cx.update(gpui_component::init);
  let (_, cx) = cx.add_window_view(|window, cx| {
    let host = cx.new(|_| Host);
    gpui_component::Root::new(host, window, cx)
  });
  cx.update(|window, cx| WorkspaceWindow::register(window.window_handle(), cx));
  let path = ConfigKind::Settings.path();
  let valid = r#"{"font_size":19}"#;
  std::fs::write(&path, valid).expect("write");
  cx.update(|_, cx| reload(ConfigKind::Settings, cx));
  let applications = Rc::new(Cell::new(0));
  let _subscription = cx.update(|_, cx| {
    cx.observe_global::<AppSettings>({
      let applications = applications.clone();
      move |_| applications.set(applications.get() + 1)
    })
  });
  for invalid in ["{", "{", "[]"] {
    std::fs::write(&path, invalid).expect("invalid edit");
    cx.update(|_, cx| reload(ConfigKind::Settings, cx));
    cx.run_until_parked();
    cx.update(|window, cx| {
      assert_eq!(AppSettings::get(cx).font_size, 19.0);
      assert_eq!(window.notifications(cx).len(), 1);
    });
  }
  std::fs::write(&path, valid).expect("repair");
  cx.update(|_, cx| reload(ConfigKind::Settings, cx));
  cx.run_until_parked();
  cx.background_executor.advance_clock(Duration::from_secs(1));
  cx.run_until_parked();
  cx.update(|window, cx| {
    assert!(window.notifications(cx).is_empty());
    assert!(cx.global::<ConfigFiles>().settings.error.is_none());
  });
  assert_eq!(applications.get(), 0);
}
