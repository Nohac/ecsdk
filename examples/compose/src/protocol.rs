use std::time::SystemTime;

use bevy::ecs::entity::MapEntities;
use bevy::ecs::prelude::*;
use serde::{Deserialize, Serialize};

/// A log line from a container or system entity, sent as a server event.
/// Uses mapped entity so the client receives a remapped entity ID.
#[derive(Event, Serialize, Deserialize)]
pub struct LogEvent {
    pub container_entity: Entity,
    pub text: String,
}

impl MapEntities for LogEvent {
    fn map_entities<M: bevy::ecs::entity::EntityMapper>(&mut self, entity_mapper: &mut M) {
        self.container_entity = entity_mapper.get_mapped(self.container_entity);
    }
}

/// Client requests the server to shut down all containers.
#[derive(Event, Serialize, Deserialize)]
pub struct StatusRequest;

/// Client requests the server to shut down all containers.
#[derive(Event, Serialize, Deserialize)]
pub struct StatusResponse {
    pub time: SystemTime,
    pub note: String,
}

/// Client requests the server to shut down all containers.
#[derive(Event, Serialize, Deserialize)]
pub struct ShutdownRequest;

/// Server notifies clients that it is exiting.
#[derive(Event, Serialize, Deserialize)]
pub struct ServerExitNotice;
