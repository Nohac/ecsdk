use bevy::app::{App, Plugin};

use crate::TracingReceiver;

/// Bevy plugin that inserts the `TracingReceiver` resource.
pub struct TracingPlugin(std::sync::Mutex<Option<TracingReceiver>>);

impl TracingPlugin {
    pub fn new(receiver: TracingReceiver) -> Self {
        Self(std::sync::Mutex::new(Some(receiver)))
    }
}

impl Plugin for TracingPlugin {
    fn build(&self, app: &mut App) {
        if let Some(receiver) = self.0.lock().unwrap().take() {
            app.insert_resource(receiver);
        }
    }
}
