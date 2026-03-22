use bevy::ecs::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Component, Serialize, Deserialize)]
pub struct ConnectionStateEntity;

#[derive(Component, Serialize, Deserialize)]
pub struct InitialConnection;

#[derive(Component, Serialize, Deserialize)]
pub struct Connected;

pub(crate) fn spawn_connection_state_entity(mut commands: Commands) {
    commands.spawn((ConnectionStateEntity, bevy_replicon::prelude::Replicated));
}

pub(crate) fn sync_connection_markers(
    clients: Query<Entity, With<bevy_replicon::prelude::ConnectedClient>>,
    connection_state: Single<Entity, With<ConnectionStateEntity>>,
    connected: Query<(), With<Connected>>,
    initial: Query<(), With<InitialConnection>>,
    mut commands: Commands,
) {
    let entity = *connection_state;
    let has_clients = !clients.is_empty();
    let is_connected = connected.get(entity).is_ok();
    let has_initial = initial.get(entity).is_ok();

    if has_clients {
        if !has_initial {
            commands.entity(entity).insert(InitialConnection);
        }
        if !is_connected {
            commands.entity(entity).insert(Connected);
        }
    } else if is_connected {
        commands.entity(entity).remove::<Connected>();
    }
}
