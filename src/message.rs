use bevy::ecs::prelude::*;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::container::*;
use crate::lifecycle::{build_container_sm, Pending, ShutdownRequested};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum Message {
    SpawnContainer {
        name: String,
        image: String,
        start_order: u32,
    },
    MarkDone {
        container_name: String,
    },
    RequestShutdown,
}

impl Message {
    pub fn apply(&self, world: &mut World) {
        match self {
            Self::SpawnContainer {
                name,
                image,
                start_order,
            } => {
                world.spawn((
                    ContainerName(name.clone()),
                    ImageRef(image.clone()),
                    StartOrder(*start_order),
                    ContainerPhase::Pending,
                    Pending,
                    build_container_sm(),
                    LogBuffer::default(),
                ));
            }
            Self::MarkDone { container_name } => {
                let entity = world
                    .query::<(Entity, &ContainerName)>()
                    .iter(world)
                    .find(|(_, name)| name.0 == *container_name)
                    .map(|(e, _)| e);
                if let Some(entity) = entity {
                    world
                        .entity_mut(entity)
                        .insert(seldom_state::prelude::Done::Success);
                }
            }
            Self::RequestShutdown => {
                world.resource_mut::<ShutdownRequested>().0 = true;
                if let Some(sys) = world
                    .query_filtered::<Entity, With<SystemEntity>>()
                    .iter(world)
                    .next()
                    && let Some(mut log_buf) = world.get_mut::<LogBuffer>(sys)
                {
                    log_buf.push("Shutting down...");
                }
            }
        }
    }
}

#[derive(Resource, Clone)]
pub struct MessageQueue {
    tx: mpsc::UnboundedSender<Message>,
}

impl MessageQueue {
    pub fn new(tx: mpsc::UnboundedSender<Message>) -> Self {
        Self { tx }
    }

    pub fn send(&self, msg: Message) {
        let _ = self.tx.send(msg);
    }

    pub fn test() -> Self {
        let (tx, _) = mpsc::unbounded_channel();
        Self { tx }
    }
}
