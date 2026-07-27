use super::GatewayEventSource;
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};
use tokio::sync::Notify;
use twilight_model::gateway::event::Event;

type MockEventQueue = Arc<Mutex<Vec<Result<Option<Event>, String>>>>;

/// Shared mock source for offline tests and downstream consumers.
#[derive(Clone, Default)]
pub struct MockGatewaySource {
    events: MockEventQueue,
    start_gate: Option<Arc<Notify>>,
    started: bool,
}

impl MockGatewaySource {
    #[must_use]
    pub fn new(events: Vec<Result<Option<Event>, String>>) -> Self {
        Self::with_optional_start_gate(events, None)
    }

    /// Creates a source that waits until the caller has attached its subscriber.
    #[must_use]
    pub fn with_start_gate(events: Vec<Result<Option<Event>, String>>, start_gate: Arc<Notify>) -> Self {
        Self::with_optional_start_gate(events, Some(start_gate))
    }

    fn with_optional_start_gate(events: Vec<Result<Option<Event>, String>>, start_gate: Option<Arc<Notify>>) -> Self {
        let mut reversed = events;
        reversed.reverse();
        Self {
            events: Arc::new(Mutex::new(reversed)),
            start_gate,
            started: false,
        }
    }
}

impl GatewayEventSource for MockGatewaySource {
    fn next_event<'a>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Event>, String>> + Send + 'a>> {
        Box::pin(async move {
            if !self.started {
                self.started = true;
                if let Some(start_gate) = &self.start_gate {
                    start_gate.notified().await;
                }
            }
            self.events
                .lock()
                .expect("mock source")
                .pop()
                .unwrap_or(Ok(None))
        })
    }
}
