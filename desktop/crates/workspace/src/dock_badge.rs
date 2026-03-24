#[cfg(target_os = "macos")]
pub fn set_dock_badge(count: usize) {
  use objc2::MainThreadMarker;
  use objc2_app_kit::NSApplication;
  use objc2_foundation::NSString;

  // Safe: this is always called from the main thread via GPUI's `this.update()`.
  let mtm = unsafe { MainThreadMarker::new_unchecked() };
  let app = NSApplication::sharedApplication(mtm);
  let dock_tile = app.dockTile();
  let label = if count > 0 {
    Some(NSString::from_str(&count.to_string()))
  } else {
    None
  };
  dock_tile.setBadgeLabel(label.as_deref());
}

#[cfg(not(target_os = "macos"))]
pub fn set_dock_badge(_count: usize) {}
