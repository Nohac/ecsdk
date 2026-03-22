use std::io::{Write, stdout};

use bevy::ecs::prelude::*;
use crossterm::{
    ExecutableCommand,
    cursor::{Hide, Show},
    terminal::{
        DisableLineWrap, EnableLineWrap, EnterAlternateScreen, LeaveAlternateScreen,
        disable_raw_mode, enable_raw_mode,
    },
};

/// Current terminal dimensions, kept up-to-date via resize events.
#[derive(Resource)]
pub struct TerminalSize {
    pub cols: u16,
    pub rows: u16,
}

impl TerminalSize {
    pub fn query_now() -> Self {
        let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
        Self { cols, rows }
    }
}

/// A screen rectangle: col, row, width, height (all in terminal cells).
#[derive(Clone, Copy)]
pub struct Rect {
    pub col: u16,
    pub row: u16,
    pub w: u16,
    pub h: u16,
}

/// Set the terminal scroll region via raw ANSI escape (CSI r).
/// top and bottom are 1-based row numbers.
pub fn set_scroll_region(out: &mut impl Write, top: u16, bottom: u16) {
    let _ = write!(out, "\x1b[{};{}r", top, bottom);
}

/// Reset scroll region to full terminal.
pub fn reset_scroll_region(out: &mut impl Write) {
    let _ = write!(out, "\x1b[r");
}

/// RAII guard that initializes the terminal on creation and restores it on drop.
#[derive(Resource, Default)]
pub struct TerminalGuard;

impl TerminalGuard {
    pub fn new() -> Self {
        let original = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            terminal_teardown();
            original(info);
        }));
        let mut out = stdout();
        let _ = enable_raw_mode();
        let _ = out.execute(EnterAlternateScreen);
        let _ = out.execute(Hide);
        let _ = out.execute(DisableLineWrap);
        Self
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        terminal_teardown();
    }
}

fn terminal_teardown() {
    let mut out = stdout();
    reset_scroll_region(&mut out);
    let _ = out.execute(EnableLineWrap);
    let _ = out.execute(Show);
    let _ = out.execute(LeaveAlternateScreen);
    let _ = disable_raw_mode();
}
