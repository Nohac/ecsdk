use std::collections::HashMap;

use bevy::ecs::prelude::*;

use crate::container::*;

#[derive(Default)]
pub(super) struct PlainRenderState {
    last_phase: HashMap<Entity, ContainerPhase>,
    last_progress: HashMap<Entity, u64>,
    log_line_count: HashMap<Entity, usize>,
}

#[allow(clippy::type_complexity)]
pub(super) fn render_plain(
    query: Query<(
        Entity,
        &ContainerName,
        &StartOrder,
        &ContainerPhase,
        Option<&DownloadProgress>,
        &LogBuffer,
    )>,
    system_query: Query<(Entity, &ContainerName, &LogBuffer), With<SystemEntity>>,
    total_count: Query<Entity, (With<ContainerName>, Without<SystemEntity>)>,
    mut state: Local<PlainRenderState>,
) {
    let total = total_count.iter().count();

    // Collect and sort deterministically so equal-priority containers
    // don't swap positions between frames.
    let mut containers: Vec<_> = query.iter().collect();
    containers.sort_by(|a, b| {
        let (_, name_a, order_a, _, _, _) = a;
        let (_, name_b, order_b, _, _, _) = b;
        order_a
            .0
            .cmp(&order_b.0)
            .then_with(|| name_a.0.cmp(&name_b.0))
    });

    for (idx, (entity, name, _order, phase, progress, log_buf)) in containers.iter().enumerate() {
        let display_idx = idx + 1;
        let entity = *entity;
        let phase = **phase;

        // Check for phase change
        let phase_changed = state.last_phase.get(&entity) != Some(&phase);

        // Check for progress update
        let progress_changed = if phase == ContainerPhase::PullingImage {
            if let Some(prog) = progress {
                let last = state.last_progress.get(&entity).copied().unwrap_or(0);
                prog.downloaded != last
            } else {
                false
            }
        } else {
            state.last_progress.remove(&entity);
            false
        };

        if phase_changed {
            println!("[{display_idx}/{total}] {} — {}", name.0, phase.label());
            state.last_phase.insert(entity, phase);
        } else if progress_changed
            && let Some(prog) = progress
            && prog.total > 0
        {
            let pct = (prog.downloaded * 100) / prog.total;
            println!(
                "[{display_idx}/{total}] {} — Pulling image ({pct}%)",
                name.0
            );
            state.last_progress.insert(entity, prog.downloaded);
        }

        // Print new log lines
        let seen = state.log_line_count.get(&entity).copied().unwrap_or(0);
        for line in log_buf.lines.iter().skip(seen) {
            println!("  {} | {}", name.0, line.text);
        }
        state.log_line_count.insert(entity, log_buf.lines.len());
    }

    // System entity logs
    for (entity, name, log_buf) in &system_query {
        let seen = state.log_line_count.get(&entity).copied().unwrap_or(0);
        for line in log_buf.lines.iter().skip(seen) {
            println!("  {} | {}", name.0, line.text);
        }
        state.log_line_count.insert(entity, log_buf.lines.len());
    }
}
