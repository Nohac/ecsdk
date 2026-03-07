use bevy::app::App;
use tokio::runtime::Handle;
use tokio::sync::mpsc;

use crate::msg::{AppExit, Msg, Queue};

pub fn setup() -> (App, mpsc::UnboundedReceiver<Box<dyn Msg>>) {
    let (msg_tx, msg_rx) = mpsc::unbounded_channel();
    let mut app = App::new();
    app.insert_resource(Queue::new(msg_tx, Handle::current()));
    app.init_resource::<AppExit>();
    (app, msg_rx)
}

pub async fn run_async(mut app: App, mut msg_rx: mpsc::UnboundedReceiver<Box<dyn Msg>>) {
    app.finish();
    app.cleanup();
    app.update();

    loop {
        if let Some(msg) = msg_rx.recv().await {
            {
                let mut cmds = app.world_mut().commands();
                msg.apply(&mut cmds);
            }
            app.world_mut().flush();
        }
        while let Ok(msg) = msg_rx.try_recv() {
            {
                let mut cmds = app.world_mut().commands();
                msg.apply(&mut cmds);
            }
            app.world_mut().flush();
        }
        app.update();
        if app.world().resource::<AppExit>().0 {
            break;
        }
    }
}
