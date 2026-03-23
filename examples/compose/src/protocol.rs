use ecsdk::prelude::*;
use serde::{Deserialize, Serialize};

/// Client requests the server to shut down all containers.
#[derive(Event, Serialize, Deserialize)]
pub struct ShutdownRequest;
