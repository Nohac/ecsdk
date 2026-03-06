use std::collections::HashMap;
use std::io::{Write, stdout};

use bevy::ecs::prelude::*;
use crossterm::{
    ExecutableCommand, QueueableCommand,
    cursor::{Hide, MoveTo, Show},
    style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor},
    terminal::{
        Clear, ClearType, DisableLineWrap, EnableLineWrap, EnterAlternateScreen,
        LeaveAlternateScreen, ScrollUp, disable_raw_mode, enable_raw_mode,
    },
};

use super::TerminalSize;
use crate::container::*;

const SERVICE_COLORS: [Color; 6] = [
    Color::Cyan,
    Color::Green,
    Color::Yellow,
    Color::Magenta,
    Color::Blue,
    Color::Red,
];

/// Set the terminal scroll region via raw ANSI escape (CSI r).
/// top and bottom are 1-based row numbers.
fn set_scroll_region(out: &mut impl Write, top: u16, bottom: u16) {
    let _ = write!(out, "\x1b[{};{}r", top, bottom);
}

/// Reset scroll region to full terminal.
fn reset_scroll_region(out: &mut impl Write) {
    let _ = write!(out, "\x1b[r");
}

fn phase_color(phase: ContainerPhase) -> (Color, Option<Attribute>) {
    match phase {
        ContainerPhase::Pending => (Color::DarkGrey, None),
        ContainerPhase::PullingImage => (Color::Yellow, None),
        ContainerPhase::Starting => (Color::Cyan, None),
        ContainerPhase::Running => (Color::Green, Some(Attribute::Bold)),
        ContainerPhase::Stopping => (Color::Yellow, None),
        ContainerPhase::Stopped => (Color::DarkGrey, None),
        ContainerPhase::Failed => (Color::Red, Some(Attribute::Bold)),
    }
}

fn progress_bar(downloaded: u64, total: u64, width: usize) -> String {
    if total == 0 {
        return format!("[{:>width$}]", "", width = width);
    }
    let ratio = (downloaded as f64) / (total as f64);
    let filled = (ratio * width as f64) as usize;
    let pct = (ratio * 100.0) as u64;
    format!(
        "[{}{}>{}] {:>3}%",
        "=".repeat(filled.min(width.saturating_sub(1))),
        if filled < width { "" } else { "=" },
        " ".repeat(width.saturating_sub(filled + 1)),
        pct
    )
}

#[derive(Default)]
pub(super) struct TuiRenderState {
    last_phase: HashMap<Entity, ContainerPhase>,
    last_progress: HashMap<Entity, u64>,
    total_log_lines_rendered: usize,
    initialized: bool,
    service_color_map: HashMap<Entity, Color>,
    next_color_idx: usize,
}

impl TuiRenderState {
    fn color_for(&mut self, entity: Entity) -> Color {
        if let Some(&c) = self.service_color_map.get(&entity) {
            return c;
        }
        let c = SERVICE_COLORS[self.next_color_idx % SERVICE_COLORS.len()];
        self.next_color_idx += 1;
        self.service_color_map.insert(entity, c);
        c
    }
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

#[allow(clippy::type_complexity)]
pub(super) fn render_tui(
    query: Query<(
        Entity,
        &ContainerName,
        &StartOrder,
        &ContainerPhase,
        Option<&DownloadProgress>,
    )>,
    merged_logs: Res<MergedLogView>,
    term_size: Res<TerminalSize>,
    mut state: Local<TuiRenderState>,
) {
    let cols = term_size.cols as usize;
    let rows = term_size.rows as usize;
    if rows < 4 || cols < 20 {
        return;
    }

    let mut out = stdout();

    // Collect and sort containers
    let mut containers: Vec<_> = query.iter().collect();
    containers.sort_by_key(|(_, _, order, _, _)| order.0);

    let header_rows = containers.len();
    let separator_row = header_rows; // 0-indexed row for separator
    let log_start_row = separator_row + 1;

    // First frame: clear screen and draw everything
    if !state.initialized {
        let _ = out.queue(Clear(ClearType::All));
        state.initialized = true;
    }

    // Set scroll region to log area (ANSI rows are 1-based)
    if log_start_row < rows {
        set_scroll_region(&mut out, (log_start_row + 1) as u16, rows as u16);
    }

    // ── Progress section (top, pinned) ──

    let name_width = containers
        .iter()
        .map(|(_, n, _, _, _)| n.0.len())
        .max()
        .unwrap_or(10);
    let bar_width = 20;

    for (idx, (entity, name, _order, phase, progress)) in containers.iter().enumerate() {
        let entity = *entity;
        let phase = **phase;

        let phase_changed = state.last_phase.get(&entity) != Some(&phase);
        let progress_changed = if let Some(prog) = progress {
            let last = state.last_progress.get(&entity).copied().unwrap_or(0);
            prog.downloaded != last
        } else {
            false
        };

        if !phase_changed && !progress_changed {
            continue;
        }

        state.last_phase.insert(entity, phase);
        if let Some(prog) = progress {
            state.last_progress.insert(entity, prog.downloaded);
        }

        let (color, attr) = phase_color(phase);
        let label = phase.label();

        let _ = out.queue(MoveTo(0, idx as u16));
        let _ = out.queue(Clear(ClearType::CurrentLine));
        let _ = out.queue(Print(format!("  {:<width$} ", name.0, width = name_width)));

        if let Some(a) = attr {
            let _ = out.queue(SetAttribute(a));
        }
        let _ = out.queue(SetForegroundColor(color));
        let _ = out.queue(Print(format!("{:<14}", label)));

        // Progress bar for PullingImage phase
        if phase == ContainerPhase::PullingImage
            && let Some(prog) = progress
        {
            let _ = out.queue(ResetColor);
            let _ = out.queue(Print(format!(
                " {}",
                progress_bar(prog.downloaded, prog.total, bar_width)
            )));
        }

        let _ = out.queue(ResetColor);
        let _ = out.queue(SetAttribute(Attribute::Reset));
    }

    // ── Separator ──
    let _ = out.queue(MoveTo(0, separator_row as u16));
    let _ = out.queue(SetForegroundColor(Color::DarkGrey));
    let _ = out.queue(Print("─".repeat(cols)));
    let _ = out.queue(ResetColor);

    // ── Log section (scrolling) ──

    let new_entries = &merged_logs.entries[state.total_log_lines_rendered..];
    if !new_entries.is_empty() {
        for entry in new_entries {
            let entity = &entry.entity;
            let name = &entry.name;
            let line = &entry.line;
            // Scroll up within the scroll region, then write at bottom
            let _ = out.queue(MoveTo(0, rows as u16 - 1));
            let _ = out.queue(ScrollUp(1));
            let _ = out.queue(MoveTo(0, rows as u16 - 1));
            let _ = out.queue(Clear(ClearType::CurrentLine));

            let svc_color = state.color_for(*entity);
            let _ = out.queue(SetForegroundColor(svc_color));
            let _ = out.queue(Print(format!("  {:<width$}", name, width = name_width)));
            let _ = out.queue(ResetColor);
            let _ = out.queue(Print(" | "));

            // Truncate log text to fit terminal width
            let prefix_len = 2 + name_width + 3; // "  " + name + " | "
            let max_text = cols.saturating_sub(prefix_len);
            let text = if line.text.len() > max_text {
                &line.text[..max_text]
            } else {
                &line.text
            };
            let _ = out.queue(Print(text));
        }

        state.total_log_lines_rendered = merged_logs.entries.len();
    }

    let _ = out.flush();
}
