use std::collections::HashMap;

use bevy_ecs::prelude::*;

use crate::components::*;

#[derive(Default)]
pub struct RenderState {
    last_phase: HashMap<Entity, ContainerPhase>,
    last_progress: HashMap<Entity, u64>,
}

pub fn render_plain(
    query: Query<(
        Entity,
        &ContainerName,
        &StartOrder,
        &ContainerPhase,
        Option<&DownloadProgress>,
    )>,
    total_count: Query<Entity, With<ContainerName>>,
    mut state: Local<RenderState>,
) {
    let total = total_count.iter().count();

    // Collect and sort by StartOrder for consistent display
    let mut containers: Vec<_> = query.iter().collect();
    containers.sort_by_key(|(_, _, order, _, _)| order.0);

    for (idx, (entity, name, _order, phase, progress)) in containers.iter().enumerate() {
        let display_idx = idx + 1;
        let entity = *entity;
        let phase = **phase;

        // Check for phase change
        let phase_changed = state.last_phase.get(&entity) != Some(&phase);

        // Check for progress update
        let progress_changed = if let Some(prog) = progress {
            let last = state.last_progress.get(&entity).copied().unwrap_or(0);
            prog.downloaded != last
        } else {
            false
        };

        if phase_changed {
            let phase_str = match phase {
                ContainerPhase::Pending => "Pending",
                ContainerPhase::PullingImage => "Pulling image",
                ContainerPhase::Starting => "Starting",
                ContainerPhase::Running => "Running",
                ContainerPhase::Stopping => "Stopping",
                ContainerPhase::Stopped => "Stopped",
                ContainerPhase::Failed => "Failed",
            };
            println!("[{display_idx}/{total}] {} — {phase_str}", name.0);
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
}
