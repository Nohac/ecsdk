use std::collections::HashMap;

use bevy::ecs::prelude::*;

use crate::container::*;

#[derive(Default)]
pub(super) struct PlainRenderState {
    last_phase: HashMap<Entity, ContainerPhase>,
    last_log_count: usize,
}

#[allow(clippy::type_complexity)]
pub(super) fn render_plain(
    query: Query<(Entity, &ContainerName, &StartOrder, &ContainerPhase)>,
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
        let (_, name_a, order_a, _) = a;
        let (_, name_b, order_b, _) = b;
        order_a
            .0
            .cmp(&order_b.0)
            .then_with(|| name_a.0.cmp(&name_b.0))
    });

    for (idx, (entity, name, _order, phase)) in containers.iter().enumerate() {
        let display_idx = idx + 1;
        let entity = *entity;
        let phase = **phase;

        let phase_changed = state.last_phase.get(&entity) != Some(&phase);

        if phase_changed {
            println!("[{display_idx}/{total}] {} — {}", name.0, phase.label());
            state.last_phase.insert(entity, phase);
        }
    }

    for entry_entity in log_view.iter().skip(state.last_log_count) {
        if let Ok(entry) = log_entries.get(entry_entity) {
            println!("  {} | {}", entry.label, entry.message);
        }
    }
    state.last_log_count = log_view.iter().len();
}
