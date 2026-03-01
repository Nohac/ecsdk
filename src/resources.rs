use bevy_ecs::prelude::*;
use tokio::runtime::Handle;
use tokio::sync::mpsc::UnboundedSender;

use crate::events::AppEvent;

/// Clone of the event channel sender. Systems use this to spawn async tasks
/// that send events back to the main loop. (Elm's `Cmd Msg` equivalent.)
#[derive(Resource, Clone)]
pub struct EventSender(pub UnboundedSender<AppEvent>);

/// Handle to the Tokio runtime. Needed because Bevy's multi-threaded schedule
/// runs systems on its own thread pool, which lacks Tokio context.
#[derive(Resource, Clone)]
pub struct TokioHandle(pub Handle);
