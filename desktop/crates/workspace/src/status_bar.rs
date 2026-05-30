use crate::api::GithubNotification;

const MAX_STATUS_BAR_NOTIFICATIONS: usize = 10;

fn unread_notification_count_label(count: usize) -> String {
  if count == 1 {
    "1 unread notification".to_string()
  } else {
    format!("{count} unread notifications")
  }
}

fn status_bar_title(count: usize) -> Option<String> {
  (count > 0).then(|| count.to_string())
}

#[cfg(any(test, target_os = "linux", target_os = "windows"))]
fn status_bar_tooltip(count: usize) -> String {
  if count > 0 {
    format!("Reviu - {}", unread_notification_count_label(count))
  } else {
    "Reviu - no unread notifications".to_string()
  }
}

fn notification_menu_title(notification: &GithubNotification) -> String {
  format!(
    "{} - {}",
    notification.subject.title, notification.repository.name
  )
}

#[cfg(any(test, target_os = "linux"))]
fn gtk_tray_init_error_message(error: impl std::fmt::Display) -> String {
  format!("Unable to initialize GTK for the Reviu system tray: {error}")
}

#[cfg(any(test, target_os = "linux"))]
fn gtk_tray_no_display_message() -> &'static str {
  "Unable to initialize the Reviu system tray: no GDK display available (Wayland/headless session without GTK support)."
}

#[cfg(any(test, target_os = "linux"))]
const LINUX_APPINDICATOR_LIBRARY_NAMES: &[&str] = &[
  "libayatana-appindicator3.so.1",
  "libappindicator3.so.1",
  "libayatana-appindicator3.so",
  "libappindicator3.so",
];

#[cfg(any(test, target_os = "linux"))]
fn linux_appindicator_unavailable_message(errors: &[String]) -> String {
  let mut message = "Unable to initialize the Reviu system tray because neither ayatana-appindicator3 nor appindicator3 is available.".to_string();
  if !errors.is_empty() {
    message.push('\n');
    message.push_str(&errors.join("\n"));
  }
  message
}

#[cfg(target_os = "macos")]
mod macos {
  use std::cell::RefCell;

  use objc2::runtime::AnyObject;
  use objc2::{
    AnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained, sel,
  };
  use objc2_app_kit::{
    NSApplication, NSImage, NSMenu, NSMenuItem, NSStatusBar, NSStatusItem,
    NSVariableStatusItemLength,
  };
  use objc2_foundation::{NSData, NSObject, NSString};

  use crate::api::GithubNotification;
  use crate::status_bar::{
    MAX_STATUS_BAR_NOTIFICATIONS, notification_menu_title, status_bar_title,
    unread_notification_count_label,
  };

  thread_local! {
    static STATUS_ITEM: RefCell<Option<Retained<NSStatusItem>>> = const { RefCell::new(None) };
    static CACHED_NOTIFICATIONS: RefCell<Vec<GithubNotification>> = const { RefCell::new(Vec::new()) };
    static PENDING_NOTIFICATION_INDEX: RefCell<Option<usize>> = const { RefCell::new(None) };
    static MENU_HANDLER: RefCell<Option<Retained<NSObject>>> = const { RefCell::new(None) };
  }

  define_class!(
    #[unsafe(super = NSObject)]
    #[name = "ReviuStatusBarMenuHandler"]
    #[thread_kind = AnyThread]
    struct StatusBarMenuHandler;

    impl StatusBarMenuHandler {
      #[unsafe(method(notificationClicked:))]
      fn notification_clicked(&self, sender: &AnyObject) {
        let index: isize = unsafe { msg_send![sender, tag] };
        PENDING_NOTIFICATION_INDEX.with(|cell| {
          *cell.borrow_mut() = Some(index as usize);
        });
        let mtm = unsafe { MainThreadMarker::new_unchecked() };
        let app = NSApplication::sharedApplication(mtm);
        let _: () = unsafe { msg_send![&app, activateIgnoringOtherApps: true] };
      }
    }
  );

  fn get_or_create_handler() -> Retained<NSObject> {
    MENU_HANDLER.with(|cell| {
      let mut borrow = cell.borrow_mut();
      if let Some(handler) = borrow.as_ref() {
        return handler.clone();
      }
      let handler: Retained<StatusBarMenuHandler> =
        unsafe { msg_send![StatusBarMenuHandler::alloc(), init] };
      let obj: Retained<NSObject> = handler.into_super();
      *borrow = Some(obj.clone());
      obj
    })
  }

  pub fn init_status_bar(icon_png: &[u8]) {
    let mtm = unsafe { MainThreadMarker::new_unchecked() };
    STATUS_ITEM.with(|cell| {
      if cell.borrow().is_some() {
        return;
      }

      let status_bar = NSStatusBar::systemStatusBar();
      let item = status_bar.statusItemWithLength(NSVariableStatusItemLength);

      if let Some(button) = item.button(mtm) {
        if let Some(icon) = load_template_icon(icon_png) {
          button.setImage(Some(&icon));
        }
      }

      let menu = NSMenu::new(mtm);
      let open_item = make_open_reviu_item(mtm);
      menu.addItem(&open_item);
      item.setMenu(Some(&menu));

      *cell.borrow_mut() = Some(item);
    });
  }

  pub fn remove_status_bar() {
    STATUS_ITEM.with(|cell| {
      if let Some(item) = cell.borrow_mut().take() {
        let status_bar = NSStatusBar::systemStatusBar();
        status_bar.removeStatusItem(&item);
      }
    });
  }

  pub fn update_status_bar(count: usize, notifications: &[GithubNotification]) {
    let mtm = unsafe { MainThreadMarker::new_unchecked() };

    CACHED_NOTIFICATIONS.with(|cell| {
      *cell.borrow_mut() = notifications.to_vec();
    });

    STATUS_ITEM.with(|cell| {
      let borrow = cell.borrow();
      let Some(item) = borrow.as_ref() else {
        return;
      };

      if let Some(button) = item.button(mtm) {
        let title = status_bar_title(count)
          .map(|count| format!(" {count}"))
          .unwrap_or_default();
        button.setTitle(&NSString::from_str(&title));
      }

      let menu = NSMenu::new(mtm);

      if count > 0 {
        let label = unread_notification_count_label(count);
        let header = unsafe {
          NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(&label),
            None,
            &NSString::from_str(""),
          )
        };
        header.setEnabled(false);
        menu.addItem(&header);
        menu.addItem(&NSMenuItem::separatorItem(mtm));

        let handler = get_or_create_handler();
        let unread: Vec<_> = notifications
          .iter()
          .enumerate()
          .filter(|(_, n)| n.unread)
          .take(MAX_STATUS_BAR_NOTIFICATIONS)
          .collect();
        for (index, notif) in &unread {
          let title = notification_menu_title(notif);
          let notif_item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
              NSMenuItem::alloc(mtm),
              &NSString::from_str(&title),
              Some(sel!(notificationClicked:)),
              &NSString::from_str(""),
            )
          };
          unsafe { notif_item.setTarget(Some(&handler)) };
          notif_item.setTag(*index as isize);
          menu.addItem(&notif_item);
        }

        if notifications.iter().filter(|n| n.unread).count() > MAX_STATUS_BAR_NOTIFICATIONS {
          let more = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
              NSMenuItem::alloc(mtm),
              &NSString::from_str("...and more"),
              None,
              &NSString::from_str(""),
            )
          };
          more.setEnabled(false);
          menu.addItem(&more);
        }

        menu.addItem(&NSMenuItem::separatorItem(mtm));
      }

      let open_item = make_open_reviu_item(mtm);
      menu.addItem(&open_item);
      item.setMenu(Some(&menu));
    });
  }

  pub fn take_pending_notification() -> Option<GithubNotification> {
    let index = PENDING_NOTIFICATION_INDEX.with(|cell| cell.borrow_mut().take())?;
    CACHED_NOTIFICATIONS.with(|cell| cell.borrow().get(index).cloned())
  }

  fn load_template_icon(png_bytes: &[u8]) -> Option<Retained<NSImage>> {
    let data = NSData::with_bytes(png_bytes);
    let icon = NSImage::initWithData(NSImage::alloc(), &data)?;
    icon.setSize(objc2_foundation::NSSize::new(18.0, 18.0));
    icon.setTemplate(true);
    Some(icon)
  }

  fn make_open_reviu_item(mtm: MainThreadMarker) -> Retained<NSMenuItem> {
    let open_item = unsafe {
      NSMenuItem::initWithTitle_action_keyEquivalent(
        NSMenuItem::alloc(mtm),
        &NSString::from_str("Open Reviu"),
        Some(sel!(activateIgnoringOtherApps:)),
        &NSString::from_str(""),
      )
    };
    let app = NSApplication::sharedApplication(mtm);
    unsafe { open_item.setTarget(Some(&app)) };
    open_item
  }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
mod desktop_tray {
  use std::cell::RefCell;
  use std::collections::HashMap;

  use image::imageops::FilterType;
  use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem},
  };

  use crate::api::GithubNotification;
  use crate::status_bar::{
    MAX_STATUS_BAR_NOTIFICATIONS, notification_menu_title, status_bar_title, status_bar_tooltip,
    unread_notification_count_label,
  };

  const OPEN_REVIU_MENU_ID: &str = "reviu-open";

  struct TrayState {
    icon: TrayIcon,
    menu: Menu,
    notifications: Vec<GithubNotification>,
    notification_items: HashMap<MenuId, usize>,
  }

  thread_local! {
    static TRAY_STATE: RefCell<Option<TrayState>> = const { RefCell::new(None) };
    static PENDING_NOTIFICATION_INDEX: RefCell<Option<usize>> = const { RefCell::new(None) };
    static PENDING_OPEN_REVIU: RefCell<bool> = const { RefCell::new(false) };
  }

  pub fn init_status_bar(icon_png: &[u8]) {
    if !ensure_platform_tray_runtime() {
      return;
    }

    TRAY_STATE.with(|cell| {
      if cell.borrow().is_some() {
        return;
      }

      let Some(icon) = load_icon(icon_png) else {
        eprintln!("Unable to load Reviu system tray icon.");
        return;
      };

      let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let menu = Menu::new();
        populate_menu(&menu, 0, &[], &mut HashMap::new())
          .map_err(|_| "menu populate".to_string())?;

        let builder = TrayIconBuilder::new()
          .with_icon(icon)
          .with_menu(Box::new(menu.clone()))
          .with_tooltip(status_bar_tooltip(0))
          .with_menu_on_left_click(true)
          .with_menu_on_right_click(true);

        let tray = builder.build().map_err(|_| "tray build".to_string())?;
        Ok::<_, String>((menu, tray))
      }));

      let state = match result {
        Ok(Ok((menu, tray))) => TrayState {
          icon: tray,
          menu,
          notifications: Vec::new(),
          notification_items: HashMap::new(),
        },
        Ok(Err(stage)) => {
          eprintln!("Unable to initialize Reviu system tray ({stage}).");
          return;
        }
        Err(_) => {
          eprintln!("Reviu system tray initialization panicked; continuing without a tray icon.");
          return;
        }
      };

      *cell.borrow_mut() = Some(state);
    });
  }

  pub fn remove_status_bar() {
    TRAY_STATE.with(|cell| {
      *cell.borrow_mut() = None;
    });
  }

  pub fn update_status_bar(count: usize, notifications: &[GithubNotification]) {
    TRAY_STATE.with(|cell| {
      let mut borrow = cell.borrow_mut();
      let Some(state) = borrow.as_mut() else {
        return;
      };

      state.notifications = notifications.to_vec();
      state.notification_items.clear();
      let _ = populate_menu(
        &state.menu,
        count,
        notifications,
        &mut state.notification_items,
      );
      let _ = state.icon.set_tooltip(Some(status_bar_tooltip(count)));
      state.icon.set_title(status_bar_title(count));
    });
  }

  pub fn take_pending_notification() -> Option<GithubNotification> {
    poll_menu_events();
    let index = PENDING_NOTIFICATION_INDEX.with(|cell| cell.borrow_mut().take())?;
    TRAY_STATE.with(|cell| {
      cell
        .borrow()
        .as_ref()
        .and_then(|state| state.notifications.get(index).cloned())
    })
  }

  pub fn take_open_reviu_request() -> bool {
    poll_menu_events();
    PENDING_OPEN_REVIU.with(|cell| {
      let value = *cell.borrow();
      *cell.borrow_mut() = false;
      value
    })
  }

  pub fn has_pending_interaction() -> bool {
    poll_menu_events();
    PENDING_OPEN_REVIU.with(|cell| *cell.borrow())
      || PENDING_NOTIFICATION_INDEX.with(|cell| cell.borrow().is_some())
  }

  fn poll_menu_events() {
    if !has_tray_state() {
      return;
    }

    drain_platform_events();

    while let Ok(event) = MenuEvent::receiver().try_recv() {
      if event.id == OPEN_REVIU_MENU_ID {
        PENDING_OPEN_REVIU.with(|cell| *cell.borrow_mut() = true);
        continue;
      }

      TRAY_STATE.with(|cell| {
        if let Some(state) = cell.borrow().as_ref()
          && let Some(index) = state.notification_items.get(&event.id).copied()
        {
          PENDING_NOTIFICATION_INDEX.with(|cell| *cell.borrow_mut() = Some(index));
          PENDING_OPEN_REVIU.with(|cell| *cell.borrow_mut() = true);
        }
      });
    }
  }

  fn has_tray_state() -> bool {
    TRAY_STATE.with(|cell| cell.borrow().is_some())
  }

  #[cfg(target_os = "linux")]
  fn ensure_platform_tray_runtime() -> bool {
    if let Err(error) = gtk::init() {
      eprintln!("{}", super::gtk_tray_init_error_message(error));
      return false;
    }

    if gtk::gdk::Screen::default().is_none() {
      eprintln!("{}", super::gtk_tray_no_display_message());
      return false;
    }

    match ensure_linux_appindicator_library() {
      Ok(()) => true,
      Err(error) => {
        eprintln!("{error}");
        false
      }
    }
  }

  #[cfg(target_os = "windows")]
  fn ensure_platform_tray_runtime() -> bool {
    true
  }

  #[cfg(target_os = "linux")]
  fn ensure_linux_appindicator_library() -> Result<(), String> {
    let mut errors = Vec::new();
    for name in super::LINUX_APPINDICATOR_LIBRARY_NAMES {
      match unsafe { libloading::Library::new(name) } {
        Ok(_) => return Ok(()),
        Err(error) => errors.push(format!("{name}: {error}")),
      }
    }

    Err(super::linux_appindicator_unavailable_message(&errors))
  }

  #[cfg(target_os = "linux")]
  fn drain_platform_events() {
    while gtk::events_pending() {
      gtk::main_iteration_do(false);
    }
  }

  #[cfg(target_os = "windows")]
  fn drain_platform_events() {}

  fn populate_menu(
    menu: &Menu,
    count: usize,
    notifications: &[GithubNotification],
    notification_items: &mut HashMap<MenuId, usize>,
  ) -> tray_icon::menu::Result<()> {
    while !menu.items().is_empty() {
      menu.remove_at(0);
    }

    if count > 0 {
      let header = MenuItem::new(unread_notification_count_label(count), false, None);
      menu.append(&header)?;
      menu.append(&PredefinedMenuItem::separator())?;

      let unread = notifications
        .iter()
        .enumerate()
        .filter(|(_, notification)| notification.unread)
        .take(MAX_STATUS_BAR_NOTIFICATIONS);

      for (index, notification) in unread {
        let id = MenuId::new(format!("reviu-notification-{index}"));
        let item = MenuItem::with_id(
          id.clone(),
          notification_menu_title(notification),
          true,
          None,
        );
        menu.append(&item)?;
        notification_items.insert(id, index);
      }

      if notifications.iter().filter(|n| n.unread).count() > MAX_STATUS_BAR_NOTIFICATIONS {
        let more = MenuItem::new("...and more", false, None);
        menu.append(&more)?;
      }

      menu.append(&PredefinedMenuItem::separator())?;
    }

    let open_item = MenuItem::with_id(OPEN_REVIU_MENU_ID, "Open Reviu", true, None);
    menu.append(&open_item)?;

    Ok(())
  }

  fn load_icon(icon_png: &[u8]) -> Option<Icon> {
    let image = image::load_from_memory(icon_png).ok()?;
    let image = image.resize_exact(32, 32, FilterType::Lanczos3).to_rgba8();
    Icon::from_rgba(image.into_raw(), 32, 32).ok()
  }
}

#[cfg(target_os = "macos")]
pub fn init_status_bar(icon_png: &[u8]) {
  macos::init_status_bar(icon_png);
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
pub fn init_status_bar(icon_png: &[u8]) {
  desktop_tray::init_status_bar(icon_png);
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
pub fn init_status_bar(_icon_png: &[u8]) {}

#[cfg(target_os = "macos")]
pub fn remove_status_bar() {
  macos::remove_status_bar();
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
pub fn remove_status_bar() {
  desktop_tray::remove_status_bar();
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
pub fn remove_status_bar() {}

pub fn set_status_bar_enabled(enabled: bool, icon_png: &[u8]) {
  if enabled {
    init_status_bar(icon_png);
  } else {
    remove_status_bar();
  }
}

#[cfg(target_os = "macos")]
pub fn update_status_bar(count: usize, notifications: &[GithubNotification]) {
  macos::update_status_bar(count, notifications);
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
pub fn update_status_bar(count: usize, notifications: &[GithubNotification]) {
  desktop_tray::update_status_bar(count, notifications);
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
pub fn update_status_bar(_count: usize, _notifications: &[GithubNotification]) {}

#[cfg(target_os = "macos")]
pub fn take_pending_notification() -> Option<GithubNotification> {
  macos::take_pending_notification()
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
pub fn take_pending_notification() -> Option<GithubNotification> {
  desktop_tray::take_pending_notification()
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
pub fn take_pending_notification() -> Option<GithubNotification> {
  None
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
pub fn take_open_reviu_request() -> bool {
  desktop_tray::take_open_reviu_request()
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub fn take_open_reviu_request() -> bool {
  false
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
pub fn has_pending_interaction() -> bool {
  desktop_tray::has_pending_interaction()
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub fn has_pending_interaction() -> bool {
  false
}

#[cfg(test)]
mod tests {
  use super::{
    LINUX_APPINDICATOR_LIBRARY_NAMES, gtk_tray_init_error_message,
    linux_appindicator_unavailable_message, notification_menu_title, status_bar_title,
    status_bar_tooltip, unread_notification_count_label,
  };
  use crate::api::{GithubNotification, GithubNotificationRepository, GithubNotificationSubject};

  #[test]
  fn unread_notification_count_label_handles_singular_and_plural() {
    assert_eq!(unread_notification_count_label(1), "1 unread notification");
    assert_eq!(unread_notification_count_label(3), "3 unread notifications");
  }

  #[test]
  fn status_bar_title_hides_zero_count() {
    assert_eq!(status_bar_title(0), None);
    assert_eq!(status_bar_title(4), Some("4".to_string()));
  }

  #[test]
  fn status_bar_tooltip_describes_empty_and_unread_states() {
    assert_eq!(
      status_bar_tooltip(0),
      "Reviu - no unread notifications".to_string()
    );
    assert_eq!(
      status_bar_tooltip(2),
      "Reviu - 2 unread notifications".to_string()
    );
  }

  #[test]
  fn gtk_tray_init_error_message_names_tray_context() {
    assert_eq!(
      gtk_tray_init_error_message("Failed to initialize GTK"),
      "Unable to initialize GTK for the Reviu system tray: Failed to initialize GTK"
    );
  }

  #[test]
  fn linux_appindicator_unavailable_message_includes_attempted_libraries() {
    let errors: Vec<String> = LINUX_APPINDICATOR_LIBRARY_NAMES
      .iter()
      .map(|name| format!("{name}: missing"))
      .collect();

    let message = linux_appindicator_unavailable_message(&errors);

    assert!(message.contains("neither ayatana-appindicator3 nor appindicator3 is available"));
    assert!(message.contains("libayatana-appindicator3.so.1: missing"));
    assert!(message.contains("libappindicator3.so: missing"));
  }

  #[test]
  fn notification_menu_title_includes_subject_and_repo() {
    let notification = GithubNotification {
      id: "1".to_string(),
      repository: GithubNotificationRepository {
        name: "widget".to_string(),
        full_name: "acme/widget".to_string(),
        owner: None,
      },
      subject: GithubNotificationSubject {
        title: "Review requested".to_string(),
        subject_type: "PullRequest".to_string(),
        url: None,
        latest_comment_url: None,
      },
      reason: "review_requested".to_string(),
      unread: true,
      updated_at: "2026-04-22T00:00:00Z".to_string(),
      last_read_at: None,
      url: "https://api.github.com/notifications/threads/1".to_string(),
      subscription_url: "https://api.github.com/notifications/threads/1/subscription".to_string(),
    };

    assert_eq!(
      notification_menu_title(&notification),
      "Review requested - widget"
    );
  }
}
