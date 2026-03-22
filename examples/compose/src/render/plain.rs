use std::collections::HashMap;

use bevy::ecs::prelude::*;

use crate::container::*;

#[derive(Default)]
pub(super) struct PlainRenderState {
    last_phase: HashMap<Entity, ContainerPhase>,
    last_progress: HashMap<Entity, u64>,
    last_log_sequence: u64,
}

#[allow(clippy::type_complexity)]
pub(super) fn render_plain(
    query: Query<(
        Entity,
        &ContainerName,
        &StartOrder,
        &ContainerPhase,
        Option<&DownloadProgress>,
    )>,
    total_count: Query<Entity, (With<ContainerName>, Without<SystemEntity>)>,
    log_view: Single<&LogView>,
    log_entries: Query<&LogEntry>,
    mut state: Local<PlainRenderState>,
) {
    let total = total_count.iter().count();

    // Collect and sort deterministically so equal-priority containers
    // don't swap positions between frames.
    let mut containers: Vec<_> = query.iter().collect();
    containers.sort_by(|a, b| {
        let (_, name_a, order_a, _, _) = a;
        let (_, name_b, order_b, _, _) = b;
        order_a
            .0
            .cmp(&order_b.0)
            .then_with(|| name_a.0.cmp(&name_b.0))
    });

    for (idx, (entity, name, _order, phase, progress)) in containers.iter().enumerate() {
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
    }

    for entry_entity in log_view.iter() {
        if let Ok(entry) = log_entries.get(entry_entity)
            && entry.sequence > state.last_log_sequence
        {
            println!("  {} | {}", entry.label, entry.message);
            state.last_log_sequence = entry.sequence;
        }
    }
}
