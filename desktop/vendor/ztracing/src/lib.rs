//! Upstream ztracing's API with profiling permanently off: spans are inert,
//! `#[instrument]` is a passthrough.

pub use tracing::{Level, field};
pub use ztracing_macro::instrument;

#[macro_export]
macro_rules! __noop_span {
  ($($tt:tt)*) => {
    $crate::Span
  };
}

pub use __noop_span as debug_span;
pub use __noop_span as error_span;
pub use __noop_span as event;
pub use __noop_span as info_span;
pub use __noop_span as span;
pub use __noop_span as trace_span;
pub use __noop_span as warn_span;

pub struct Span;

impl Span {
  pub fn current() -> Self {
    Self
  }

  pub fn enter(&self) {}

  pub fn record<T, S>(&self, _t: T, _s: S) {}
}

pub fn init() {}
