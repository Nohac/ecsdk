use std::collections::HashMap;
use std::io::{Write, stdout};

use bevy::ecs::prelude::*;
use crossterm::{
    QueueableCommand,
    cursor::MoveTo,
    style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor},
    terminal::{Clear, ClearType, ScrollUp},
};
use ecsdk::term::{Rect, TerminalSize, reset_scroll_region, set_scroll_region};

use crate::container::*;

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

const SERVICE_COLORS: [Color; 6] = [
    Color::Cyan,
    Color::Green,
    Color::Yellow,
    Color::Magenta,
    Color::Blue,
    Color::Red,
];

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

// ── Render state ──

#[derive(Default)]
pub(super) struct TuiRenderState {
    last_phase: HashMap<Entity, ContainerPhase>,
    last_progress: HashMap<Entity, u64>,
    last_log_sequence: u64,
    initialized: bool,
    last_cols: u16,
    last_rows: u16,
}

// ── Section renderers ──

struct ContainerRow {
    entity: Entity,
    name: String,
    phase: ContainerPhase,
    downloaded: Option<u64>,
    total: Option<u64>,
}

fn render_header(
    out: &mut impl Write,
    rect: Rect,
    containers: &[ContainerRow],
    state: &mut TuiRenderState,
    full: bool,
) {
    let name_width = containers.iter().map(|c| c.name.len()).max().unwrap_or(10);

    for (i, c) in containers.iter().enumerate() {
        if i as u16 >= rect.h {
            break;
        }

        let phase_changed = state.last_phase.get(&c.entity) != Some(&c.phase);
        let progress_changed = c
            .downloaded
            .is_some_and(|d| state.last_progress.get(&c.entity).copied().unwrap_or(0) != d);

        if !full && !phase_changed && !progress_changed {
            continue;
        }

        state.last_phase.insert(c.entity, c.phase);
        if let Some(d) = c.downloaded {
            state.last_progress.insert(c.entity, d);
        }

        let (color, attr) = phase_color(c.phase);

        let _ = out.queue(MoveTo(rect.col, rect.row + i as u16));
        let _ = out.queue(Clear(ClearType::CurrentLine));
        let _ = out.queue(Print(format!("  {:<width$} ", c.name, width = name_width)));

        if let Some(a) = attr {
            let _ = out.queue(SetAttribute(a));
        }
        let _ = out.queue(SetForegroundColor(color));
        let _ = out.queue(Print(format!("{:<14}", c.phase.label())));

        if c.phase == ContainerPhase::PullingImage
            && let (Some(downloaded), Some(total)) = (c.downloaded, c.total)
        {
            let _ = out.queue(ResetColor);
            let _ = out.queue(Print(format!(" {}", progress_bar(downloaded, total, 20))));
        }

        let _ = out.queue(ResetColor);
        let _ = out.queue(SetAttribute(Attribute::Reset));
    }
}

fn render_separator(out: &mut impl Write, rect: Rect) {
    let _ = out.queue(MoveTo(rect.col, rect.row));
    let _ = out.queue(SetForegroundColor(Color::DarkGrey));
    let _ = out.queue(Print("─".repeat(rect.w as usize)));
    let _ = out.queue(ResetColor);
}

fn render_log_line(out: &mut impl Write, entry: &LogEntry, row: u16, cols: usize) {
    let _ = out.queue(MoveTo(0, row));
    let _ = out.queue(Clear(ClearType::CurrentLine));
    let _ = out.queue(Print("  "));
    let color = SERVICE_COLORS[entry.color_idx as usize % SERVICE_COLORS.len()];
    let _ = out.queue(SetForegroundColor(color));
    let label = &entry.label;
    let sep = " | ";
    let reserved = 2 + label.len() + sep.len();
    let max_text = cols.saturating_sub(reserved);
    let text = if entry.message.len() > max_text {
        &entry.message[..max_text]
    } else {
        &entry.message
    };
    let _ = out.queue(Print(label));
    let _ = out.queue(ResetColor);
    let _ = out.queue(Print(sep));
    let _ = out.queue(SetForegroundColor(Color::Grey));
    let _ = out.queue(Print(text));
    let _ = out.queue(ResetColor);
}

fn render_logs(
    out: &mut impl Write,
    rect: Rect,
    logs: &[&LogEntry],
    state: &mut TuiRenderState,
    full: bool,
) {
    if rect.h == 0 {
        return;
    }

    // Set scroll region to log area (ANSI rows are 1-based)
    set_scroll_region(out, rect.row + 1, rect.row + rect.h);

    let cols = rect.w as usize;

    if full {
        // Bottom-align: draw the last N entries at the bottom of the rect
        let visible = (rect.h as usize).min(logs.len());
        let start = logs.len().saturating_sub(visible);
        let offset = rect.h - visible as u16;

        for (i, entry) in logs[start..].iter().enumerate() {
            let row = rect.row + offset + i as u16;
            render_log_line(out, entry, row, cols);
        }
        state.last_log_sequence = logs.last().map(|e| e.sequence).unwrap_or(0);
    } else {
        let bottom_row = rect.row + rect.h - 1;
        let last_seen = state.last_log_sequence;
        for entry in logs.iter().filter(|entry| entry.sequence > last_seen) {
            let _ = out.queue(MoveTo(0, bottom_row));
            let _ = out.queue(ScrollUp(1));
            render_log_line(out, entry, bottom_row, cols);
            state.last_log_sequence = entry.sequence;
        }
    }
}

// ── Main render system ──

#[allow(clippy::type_complexity)]
pub(super) fn render_tui(
    query: Query<(
        Entity,
        &ContainerName,
        &StartOrder,
        &ContainerPhase,
        Option<&DownloadProgress>,
    )>,
    log_view: Single<&LogView>,
    log_entries: Query<&LogEntry>,
    term_size: Res<TerminalSize>,
    mut state: Local<TuiRenderState>,
) {
    let cols = term_size.cols;
    let rows = term_size.rows;
    if rows < 4 || cols < 20 {
        return;
    }

    let mut out = stdout();

    // Collect and sort containers
    let mut containers: Vec<_> = query
        .iter()
        .map(|(entity, name, order, phase, progress)| {
            (
                order.0,
                ContainerRow {
                    entity,
                    name: name.0.clone(),
                    phase: *phase,
                    downloaded: progress.map(|p| p.downloaded),
                    total: progress.map(|p| p.total),
                },
            )
        })
        .collect();
    containers.sort_by(|(order_a, container_a), (order_b, container_b)| {
        order_a
            .cmp(order_b)
            .then_with(|| container_a.name.cmp(&container_b.name))
    });
    let containers: Vec<_> = containers.into_iter().map(|(_, c)| c).collect();

    let logs: Vec<_> = log_view
        .iter()
        .filter_map(|entry_entity| log_entries.get(entry_entity).ok())
        .collect();

    // Layout: header | separator (1 row) | logs
    let header_h = containers.len() as u16;
    let separator_row = header_h;
    let log_start = separator_row + 1;
    let log_h = rows.saturating_sub(log_start);

    let size_changed = cols != state.last_cols || rows != state.last_rows;
    let full = !state.initialized || size_changed;

    if full {
        reset_scroll_region(&mut out);
        let _ = out.queue(Clear(ClearType::All));
        state.initialized = true;
        state.last_cols = cols;
        state.last_rows = rows;
        state.last_phase.clear();
        state.last_progress.clear();
    }

    render_header(
        &mut out,
        Rect {
            col: 0,
            row: 0,
            w: cols,
            h: header_h,
        },
        &containers,
        &mut state,
        full,
    );

    render_separator(
        &mut out,
        Rect {
            col: 0,
            row: separator_row,
            w: cols,
            h: 1,
        },
    );

    render_logs(
        &mut out,
        Rect {
            col: 0,
            row: log_start,
            w: cols,
            h: log_h,
        },
        &logs,
        &mut state,
        full,
    );

    let _ = out.flush();
}
