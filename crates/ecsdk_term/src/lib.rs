mod plugin;
mod terminal;

pub use plugin::{TermPlugin, TerminalEvent};
pub use terminal::{Rect, TerminalGuard, TerminalSize, reset_scroll_region, set_scroll_region};
