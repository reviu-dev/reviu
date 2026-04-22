use crate::api::GithubNotification;

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
        let title = if count > 0 {
          format!(" {count}")
        } else {
          String::new()
        };
        button.setTitle(&NSString::from_str(&title));
      }

      let menu = NSMenu::new(mtm);

      if count > 0 {
        let label = if count == 1 {
          "1 unread notification".to_string()
        } else {
          format!("{count} unread notifications")
        };
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
          .take(10)
          .collect();
        for (index, notif) in &unread {
          let title = format!("{} - {}", notif.subject.title, notif.repository.name);
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

        if notifications.iter().filter(|n| n.unread).count() > 10 {
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

#[cfg(target_os = "macos")]
pub fn init_status_bar(icon_png: &[u8]) {
  macos::init_status_bar(icon_png);
}

#[cfg(not(target_os = "macos"))]
pub fn init_status_bar(_icon_png: &[u8]) {}

#[cfg(target_os = "macos")]
pub fn remove_status_bar() {
  macos::remove_status_bar();
}

#[cfg(not(target_os = "macos"))]
pub fn remove_status_bar() {}

pub fn set_status_bar_enabled(enabled: bool, icon_png: &[u8]) {
  println!("Setting status bar enabled: {enabled}");
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

#[cfg(not(target_os = "macos"))]
pub fn update_status_bar(_count: usize, _notifications: &[GithubNotification]) {}

#[cfg(target_os = "macos")]
pub fn take_pending_notification() -> Option<GithubNotification> {
  macos::take_pending_notification()
}

#[cfg(not(target_os = "macos"))]
pub fn take_pending_notification() -> Option<GithubNotification> {
  None
}
