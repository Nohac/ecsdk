use std::future::Future;
use std::pin::Pin;

use bevy::app::App;
use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use tokio::runtime::Handle;
use tokio::sync::mpsc;

use crate::bridge::{AppExit, WorldCmd};
use crate::task::CommandSender;

pub type TaskQueue = Vec<Pin<Box<dyn Future<Output = ()> + Send>>>;

pub fn setup() -> (App, mpsc::UnboundedReceiver<WorldCmd>, TaskQueue) {
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let mut app = App::new();
    app.insert_resource(CommandSender::new(cmd_tx, Handle::current()));
    app.init_resource::<AppExit>();
    (app, cmd_rx, Vec::new())
}

pub async fn run_async(mut app: App, mut cmd_rx: mpsc::UnboundedReceiver<WorldCmd>, tasks: TaskQueue) {
    let mut tasks: FuturesUnordered<_> = tasks.into_iter().collect();

    app.finish();
    app.cleanup();
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
