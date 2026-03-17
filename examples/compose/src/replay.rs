use bevy::app::App;
use ecsdk::core::ApplyMessage;

use crate::lifecycle::LifecycleTestPlugin;
use crate::message::Message;

/// Replays a sequence of state events, running `app.update()` after each one.
/// Returns the final App for inspection.
pub fn replay(events: &[Message]) -> App {
    let mut app = App::new();
    app.add_plugins(LifecycleTestPlugin);
    app.update();

    for event in events {
        event.apply(app.world_mut());
        app.update();
    }

    app
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::*;

    #[test]
    fn replay_full_lifecycle() {
        let events = vec![
            Message::SpawnContainer {
                name: "postgres".into(),
                image: "postgres:16".into(),
                start_order: 0,
            },
            Message::MarkDone {
                container_name: "postgres".into(),
            },
            Message::MarkDone {
                container_name: "postgres".into(),
            },
            Message::RequestShutdown,
            Message::MarkDone {
                container_name: "postgres".into(),
            },
        ];

        let mut app = replay(&events);
        let world = app.world_mut();

        let (_, phase) = world
            .query::<(&ContainerName, &ContainerPhase)>()
            .iter(world)
            .find(|(name, _)| name.0 == "postgres")
            .expect("postgres entity not found");

        assert_eq!(*phase, ContainerPhase::Stopped);
    }

    #[test]
    fn replay_roundtrip_via_json() {
        let events = vec![
            Message::SpawnContainer {
                name: "redis".into(),
                image: "redis:7".into(),
                start_order: 0,
            },
            Message::MarkDone {
                container_name: "redis".into(),
            },
        ];

        // Serialize to JSON lines
        let json: Vec<String> = events
            .iter()
            .map(|e| serde_json::to_string(e).unwrap())
            .collect();

        // Deserialize back
        let deserialized: Vec<Message> = json
            .iter()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();

        let mut app = replay(&deserialized);
        let world = app.world_mut();

        let (_, phase) = world
            .query::<(&ContainerName, &ContainerPhase)>()
            .iter(world)
            .find(|(name, _)| name.0 == "redis")
            .expect("redis entity not found");

        assert_eq!(*phase, ContainerPhase::Starting);
    }
}
