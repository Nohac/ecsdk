use std::fmt;

use bevy::app::App;
use bevy::ecs::entity::Entity;
use bevy::ecs::prelude::Resource;
use ecsdk_core::WakeSignal;
use tokio::sync::mpsc;
use tracing::field::{Field, Visit};
use tracing::span;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

// ---------------------------------------------------------------------------
// Log event — captured by the layer, drained by the app
// ---------------------------------------------------------------------------

pub struct LogEvent {
    pub entity: Option<Entity>,
    pub level: tracing::Level,
    pub message: String,
}

// ---------------------------------------------------------------------------
// Span data — stores entity_id extracted from span fields
// ---------------------------------------------------------------------------

struct SpanData {
    entity_bits: Option<u64>,
}

// ---------------------------------------------------------------------------
// Field visitors
// ---------------------------------------------------------------------------

struct EntityVisitor {
    entity_bits: Option<u64>,
}

impl Visit for EntityVisitor {
    fn record_u64(&mut self, field: &Field, value: u64) {
        if field.name() == "entity_id" {
            self.entity_bits = Some(value);
        }
    }

    fn record_debug(&mut self, _field: &Field, _value: &dyn fmt::Debug) {}
}

struct MessageVisitor {
    message: String,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_owned();
        }
    }
}

// ---------------------------------------------------------------------------
// ECS tracing layer + receiver
// ---------------------------------------------------------------------------

pub struct EcsTracingLayer {
    tx: mpsc::UnboundedSender<LogEvent>,
    wake: WakeSignal,
}

/// Resource holding the receiver end of the tracing channel.
/// The app schedules its own drain system to read from `rx` and route
/// events into whatever log storage it uses (e.g. entity LogBuffers).
#[derive(Resource)]
pub struct TracingReceiver {
    pub rx: mpsc::UnboundedReceiver<LogEvent>,
}

/// Creates a matched layer + receiver pair.
///
/// The caller:
/// 1. Installs the layer in a `tracing_subscriber`
/// 2. Adds `ecsdk_tracing::plugin(receiver)` to the app
/// 3. Schedules a drain system that reads from `TracingReceiver::rx`
pub fn setup(wake: WakeSignal) -> (EcsTracingLayer, TracingReceiver) {
    let (tx, rx) = mpsc::unbounded_channel();
    (EcsTracingLayer { tx, wake }, TracingReceiver { rx })
}

/// Bevy plugin that inserts the `TracingReceiver` resource.
/// The app schedules its own drain system.
pub struct TracingPlugin(std::sync::Mutex<Option<TracingReceiver>>);

impl TracingPlugin {
    pub fn new(receiver: TracingReceiver) -> Self {
        Self(std::sync::Mutex::new(Some(receiver)))
    }
}

impl bevy::app::Plugin for TracingPlugin {
    fn build(&self, app: &mut App) {
        if let Some(receiver) = self.0.lock().unwrap().take() {
            app.insert_resource(receiver);
        }
    }
}


// ---------------------------------------------------------------------------
// Layer implementation
// ---------------------------------------------------------------------------

impl<S> tracing_subscriber::Layer<S> for EcsTracingLayer
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(
        &self,
        attrs: &span::Attributes<'_>,
        id: &span::Id,
        ctx: Context<'_, S>,
    ) {
        let mut visitor = EntityVisitor { entity_bits: None };
        attrs.record(&mut visitor);

        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(SpanData {
                entity_bits: visitor.entity_bits,
            });
        }
    }

    fn on_event(&self, event: &tracing::Event<'_>, ctx: Context<'_, S>) {
        let mut msg_visitor = MessageVisitor {
            message: String::new(),
        };
        event.record(&mut msg_visitor);

        if msg_visitor.message.is_empty() {
            return;
        }

        // Walk span stack to find entity_id
        let entity = ctx.event_span(event).and_then(|span| {
            let mut current = Some(span);
            while let Some(s) = current {
                if let Some(data) = s.extensions().get::<SpanData>()
                    && let Some(bits) = data.entity_bits
                {
                    return Entity::try_from_bits(bits);
                }
                current = s.parent();
            }
            None
        });

        let _ = self.tx.send(LogEvent {
            entity,
            level: *event.metadata().level(),
            message: msg_visitor.message,
        });
        self.wake.0.notify_one();
    }
}
