use std::fmt;

use bevy::ecs::entity::Entity;
use bevy::ecs::prelude::Resource;
use ecsdk_core::WakeSignal;
use tokio::sync::mpsc;
use tracing::field::{Field, Visit};
use tracing::span;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

pub struct LogEvent {
    pub entity: Option<Entity>,
    pub level: tracing::Level,
    pub message: String,
}

struct SpanData {
    entity_bits: Option<u64>,
}

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

pub struct EcsTracingLayer {
    tx: mpsc::UnboundedSender<LogEvent>,
    wake: WakeSignal,
}

/// Resource holding the receiver end of the tracing channel.
#[derive(Resource)]
pub struct TracingReceiver {
    pub rx: mpsc::UnboundedReceiver<LogEvent>,
}

pub fn setup(wake: WakeSignal) -> (EcsTracingLayer, TracingReceiver) {
    let (tx, rx) = mpsc::unbounded_channel();
    (EcsTracingLayer { tx, wake }, TracingReceiver { rx })
}

impl<S> tracing_subscriber::Layer<S> for EcsTracingLayer
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &span::Attributes<'_>, id: &span::Id, ctx: Context<'_, S>) {
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
