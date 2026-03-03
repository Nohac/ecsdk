use std::future::Future;
use std::pin::Pin;

use bevy_ecs::prelude::*;
use bevy_ecs::schedule::ScheduleLabel;
use bevy_ecs::system::ScheduleSystem;
use futures_util::stream::FuturesUnordered;
use futures_util::StreamExt;
use tokio::sync::mpsc;

use crate::bridge::{AppExit, WorldCmd};
use crate::task::CommandSender;

pub trait Plugin {
    fn build(self, app: &mut App);
}

#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Startup;

#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Update;

pub struct App {
    pub world: World,
    cmd_rx: mpsc::UnboundedReceiver<WorldCmd>,
    startup: Schedule,
    update: Schedule,
    tasks: FuturesUnordered<Pin<Box<dyn Future<Output = ()> + Send>>>,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<WorldCmd>();
        let cmd_sender = CommandSender::new(cmd_tx, tokio::runtime::Handle::current());

        let mut world = World::new();
        world.insert_resource(cmd_sender);
        world.init_resource::<AppExit>();

        Self {
            world,
            cmd_rx,
            startup: Schedule::default(),
            update: Schedule::default(),
            tasks: FuturesUnordered::new(),
        }
    }

    pub fn cmd_sender(&self) -> CommandSender {
        self.world.resource::<CommandSender>().clone()
    }

    pub fn add_plugin(&mut self, plugin: impl Plugin) -> &mut Self {
        plugin.build(self);
        self
    }

    pub fn add_systems<M>(
        &mut self,
        label: impl ScheduleLabel,
        systems: impl IntoScheduleConfigs<ScheduleSystem, M>,
    ) -> &mut Self {
        if label.intern() == Startup.intern() {
            self.startup.add_systems(systems);
        } else {
            self.update.add_systems(systems);
        }
        self
    }

    pub fn insert_resource<R: Resource>(&mut self, resource: R) -> &mut Self {
        self.world.insert_resource(resource);
        self
    }

    pub fn init_resource<R: Resource + FromWorld>(&mut self) -> &mut Self {
        self.world.init_resource::<R>();
        self
    }

    pub fn add_task(&mut self, fut: impl Future<Output = ()> + Send + 'static) -> &mut Self {
        self.tasks.push(Box::pin(fut));
        self
    }

    pub async fn run(&mut self) {
        self.startup.run(&mut self.world);
        self.drain_and_update();

        loop {
            tokio::select! {
                Some(cmd) = self.cmd_rx.recv() => {
                    cmd(&mut self.world);
                }
                Some(()) = self.tasks.next() => {
                    // Task completed (e.g. ctrl_c fired) — may have pushed to channel
                }
            }
            self.drain_and_update();
            if self.world.resource::<AppExit>().0 {
                break;
            }
        }
    }

    fn drain_and_update(&mut self) {
        while let Ok(cmd) = self.cmd_rx.try_recv() {
            cmd(&mut self.world);
        }
        self.update.run(&mut self.world);
    }
}
