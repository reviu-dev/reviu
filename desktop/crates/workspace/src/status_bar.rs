#[cfg(target_os = "macos")]
use crate::api::GithubNotification;

#[cfg(target_os = "macos")]
mod macos {
  use std::cell::RefCell;

  use objc2::AnyThread;
  use objc2::MainThreadMarker;
  use objc2::MainThreadOnly;
  use objc2::rc::Retained;
  use objc2::sel;
  use objc2_app_kit::{
    NSApplication, NSImage, NSMenu, NSMenuItem, NSStatusBar, NSStatusItem,
    NSVariableStatusItemLength,
  };
  use objc2_foundation::{NSData, NSString};

  use crate::api::GithubNotification;

  thread_local! {
    static STATUS_ITEM: RefCell<Option<Retained<NSStatusItem>>> = const { RefCell::new(None) };
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

  pub fn update_status_bar(count: usize, notifications: &[GithubNotification]) {
    let mtm = unsafe { MainThreadMarker::new_unchecked() };

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

        let unread: Vec<_> = notifications.iter().filter(|n| n.unread).take(10).collect();
        for notif in &unread {
          let title = format!("{} — {}", notif.subject.title, notif.repository.name);
          let notif_item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
              NSMenuItem::alloc(mtm),
              &NSString::from_str(&title),
              Some(sel!(activateIgnoringOtherApps:)),
              &NSString::from_str(""),
            )
          };
          let app = NSApplication::sharedApplication(mtm);
          unsafe { notif_item.setTarget(Some(&app)) };
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
pub fn update_status_bar(count: usize, notifications: &[GithubNotification]) {
  macos::update_status_bar(count, notifications);
}

#[cfg(not(target_os = "macos"))]
pub fn update_status_bar(_count: usize, _notifications: &[()]) {}
