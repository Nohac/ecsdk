use bevy::ecs::prelude::*;
use tokio::runtime::Handle;
use tokio::sync::mpsc::UnboundedSender;

use crate::container::LogBuffer;

/// A named message that can be applied to the world via Commands.
pub trait Msg: Send + 'static {
    fn apply(self: Box<Self>, commands: &mut Commands);
}

/// Resource that bridges async tasks to the ECS world.
/// Carries both the message channel sender and the Tokio runtime handle.
#[derive(Resource, Clone)]
pub struct Queue {
    pub(crate) msg_tx: UnboundedSender<Box<dyn Msg>>,
    pub(crate) handle: Handle,
}

impl Queue {
    pub fn new(msg_tx: UnboundedSender<Box<dyn Msg>>, handle: Handle) -> Self {
        Self { msg_tx, handle }
    }

    pub fn send(&self, msg: impl Msg) {
        let _ = self.msg_tx.send(Box::new(msg));
    }
}

/// Handle passed to async task closures. Provides access to the owning entity
/// and a way to send messages back to the ECS.
#[derive(Clone)]
pub struct TaskQueue {
    entity: Entity,
    queue: Queue,
}

impl TaskQueue {
    pub(crate) fn new(entity: Entity, queue: Queue) -> Self {
        Self { entity, queue }
    }

    pub fn entity(&self) -> Entity {
        self.entity
    }

    pub fn send(&self, msg: impl Msg) {
        self.queue.send(msg);
    }
}

// ── Built-in messages ──

/// Triggers a Bevy event through the message queue.
pub struct TriggerEvent<E>(pub E);

impl<E> Msg for TriggerEvent<E>
where
    E: Event + Send + 'static,
    for<'a> E::Trigger<'a>: Default,
{
    fn apply(self: Box<Self>, commands: &mut Commands) {
        commands.trigger(self.0);
    }
}

/// Sets the AppExit resource to signal the main loop to exit.
pub struct SetAppExit;

impl Msg for SetAppExit {
    fn apply(self: Box<Self>, commands: &mut Commands) {
        commands.insert_resource(AppExit(true));
    }
}

/// Appends a log line to an entity's LogBuffer.
pub struct AppendLog {
    pub entity: Entity,
    pub text: String,
}

impl Msg for AppendLog {
    fn apply(self: Box<Self>, commands: &mut Commands) {
        commands.entity(self.entity).queue(AppendLogCmd(self.text));
    }
}

struct AppendLogCmd(String);

impl EntityCommand for AppendLogCmd {
    fn apply(self, mut ewm: EntityWorldMut<'_>) {
        if let Some(mut log_buf) = ewm.get_mut::<LogBuffer>() {
            log_buf.push(self.0);
        }
    }
}

/// Signals that replicon packets arrived on the transport channels
/// and are ready to be drained by ECS systems in the next update cycle.
pub struct PacketsReady;

impl Msg for PacketsReady {
    fn apply(self: Box<Self>, _commands: &mut Commands) {}
}

/// Checked by main loop after each cycle to know when to exit.
#[derive(Resource, Default)]
pub struct AppExit(pub bool);
