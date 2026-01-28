use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TelemetryEvent {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl TelemetryEvent {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            data: None,
        }
    }

    pub fn with_data(name: impl Into<String>, data: Value) -> Self {
        Self {
            name: name.into(),
            data: Some(data),
        }
    }
}

pub trait TelemetrySink: Send + Sync {
    fn emit(&self, event: TelemetryEvent);
}

#[derive(Debug, Default)]
pub struct NoopTelemetrySink;

impl TelemetrySink for NoopTelemetrySink {
    fn emit(&self, _event: TelemetryEvent) {}
}

#[derive(Clone)]
pub struct Telemetry {
    sink: Arc<dyn TelemetrySink>,
}

impl Telemetry {
    pub fn new<S>(sink: S) -> Self
    where
        S: TelemetrySink + 'static,
    {
        Self {
            sink: Arc::new(sink),
        }
    }

    pub fn from_arc(sink: Arc<dyn TelemetrySink>) -> Self {
        Self { sink }
    }

    pub fn emit(&self, event: TelemetryEvent) {
        self.sink.emit(event);
    }
}

impl Default for Telemetry {
    fn default() -> Self {
        Self::new(NoopTelemetrySink)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    struct CountingSink {
        counter: Arc<AtomicUsize>,
    }

    impl TelemetrySink for CountingSink {
        fn emit(&self, _event: TelemetryEvent) {
            self.counter.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn telemetry_hook_is_called() {
        let counter = Arc::new(AtomicUsize::new(0));
        let telemetry = Telemetry::new(CountingSink {
            counter: Arc::clone(&counter),
        });
        telemetry.emit(TelemetryEvent::new("test.event"));
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }
}
