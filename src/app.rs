use bevy::app::App;
use tokio::runtime::Handle;
use tokio::sync::mpsc;

use crate::bridge::{AppExit, WorldCmd};
use crate::task::CommandSender;

pub fn setup() -> (App, mpsc::UnboundedReceiver<WorldCmd>) {
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let mut app = App::new();
    app.insert_resource(CommandSender::new(cmd_tx, Handle::current()));
    app.init_resource::<AppExit>();
    (app, cmd_rx)
}

pub async fn run_async(mut app: App, mut cmd_rx: mpsc::UnboundedReceiver<WorldCmd>) {
    app.finish();
    app.cleanup();
    app.update();

    loop {
        if let Some(cmd) = cmd_rx.recv().await {
            cmd(app.world_mut());
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
