//! The PTY behind a terminal: shell process, grid, search and process
//! tracking, without any UI so non-GPUI crates can run commands in it.

mod session;

pub use session::*;
