use gpui::Context;
use smol::Timer;
use std::time::Duration;

const CURSOR_BLINK_INTERVAL: Duration = Duration::from_millis(500);

pub struct CursorBlink {
  blink_epoch: usize,
  blinking_paused: bool,
  visible: bool,
  enabled: bool,
}

impl CursorBlink {
  pub fn new(cx: &mut Context<Self>) -> Self {
    let mut blink = Self {
      blink_epoch: 0,
      blinking_paused: false,
      visible: true,
      enabled: false,
    };

    // Start blinking immediately
    blink.enable(cx);
    blink
  }

  fn next_blink_epoch(&mut self) -> usize {
    self.blink_epoch += 1;
    self.blink_epoch
  }

  /// Pause blinking temporarily (e.g., when typing)
  /// After the interval, blinking will resume
  pub fn pause_blinking(&mut self, cx: &mut Context<Self>) {
    self.show_cursor(cx);
    self.blinking_paused = true;

    let epoch = self.next_blink_epoch();
    cx.spawn(async move |this, cx| {
      Timer::after(CURSOR_BLINK_INTERVAL).await;
      this.update(cx, |this, cx| this.resume_cursor_blinking(epoch, cx))
    })
    .detach();
  }

  fn resume_cursor_blinking(&mut self, epoch: usize, cx: &mut Context<Self>) {
    if epoch == self.blink_epoch {
      self.blinking_paused = false;
      self.blink_cursors(epoch, cx);
    }
  }

  fn blink_cursors(&mut self, epoch: usize, cx: &mut Context<Self>) {
    if epoch == self.blink_epoch && self.enabled && !self.blinking_paused {
      self.visible = !self.visible;
      cx.notify();

      let epoch = self.next_blink_epoch();
      cx.spawn(async move |this, cx| {
        Timer::after(CURSOR_BLINK_INTERVAL).await;
        if let Some(this) = this.upgrade() {
          this
            .update(cx, |this, cx| this.blink_cursors(epoch, cx))
            .ok();
        }
      })
      .detach();
    }
  }

  pub fn show_cursor(&mut self, cx: &mut Context<Self>) {
    if !self.visible {
      self.visible = true;
      cx.notify();
    }
  }

  /// Enable cursor blinking
  pub fn enable(&mut self, cx: &mut Context<Self>) {
    if self.enabled {
      return;
    }

    self.enabled = true;
    // Start with cursor invisible, will become visible on first blink
    self.visible = false;
    self.blink_cursors(self.blink_epoch, cx);
  }

  /// Disable cursor blinking (keeps cursor visible)
  pub fn disable(&mut self, cx: &mut Context<Self>) {
    self.enabled = false;
    self.show_cursor(cx);
  }

  /// Check if cursor should be rendered
  pub fn visible(&self) -> bool {
    self.visible
  }
}
