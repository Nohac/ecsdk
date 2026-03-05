use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;

use bevy_app::App;
use bevy_ecs::prelude::*;
use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use tokio::runtime::Handle;
use tokio::sync::mpsc;

use crate::bridge::{AppExit, WorldCmd};
use crate::task::CommandSender;

#[derive(Resource, Default)]
pub struct TaskQueue(Mutex<Vec<Pin<Box<dyn Future<Output = ()> + Send>>>>);

pub fn setup() -> (App, mpsc::UnboundedReceiver<WorldCmd>) {
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let mut app = App::new();
    app.insert_resource(CommandSender::new(cmd_tx, Handle::current()));
    app.init_resource::<AppExit>();
    app.init_resource::<TaskQueue>();
    (app, cmd_rx)
}

impl TaskQueue {
    pub fn push(&mut self, fut: impl Future<Output = ()> + Send + 'static) {
        self.0.get_mut().unwrap().push(Box::pin(fut));
    }
}

pub async fn run_async(mut app: App, mut cmd_rx: mpsc::UnboundedReceiver<WorldCmd>) {
    let mut tasks: FuturesUnordered<Pin<Box<dyn Future<Output = ()> + Send>>> = app
        .world_mut()
        .resource_mut::<TaskQueue>()
        .0
        .get_mut()
        .unwrap()
        .drain(..)
        .collect();

    app.update();

    loop {
        tokio::select! {
            Some(cmd) = cmd_rx.recv() => {
                cmd(app.world_mut());
            }
            Some(()) = tasks.next() => {}
        }
        while let Ok(cmd) = cmd_rx.try_recv() {
            cmd(app.world_mut());
        }
        app.update();
        if app.world().resource::<AppExit>().0 {
            break;
        }
    }
}
