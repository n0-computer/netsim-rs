//! Builder for emitting multiple metrics in a single JSONL line.

use serde_json::Map;

/// Builder for batch metric emission. Obtained from [`Device::metrics()`](crate::Device::metrics).
pub struct MetricsBuilder {
    pub(crate) dispatch: tracing::Dispatch,
    pub(crate) values: Map<String, serde_json::Value>,
}

impl MetricsBuilder {
    pub(crate) fn new(dispatch: tracing::Dispatch) -> Self {
        Self {
            dispatch,
            values: Map::new(),
        }
    }

    /// Add a metric key-value pair.
    pub fn record(mut self, key: &str, value: f64) -> Self {
        if let Some(n) = serde_json::Number::from_f64(value) {
            self.values
                .insert(key.to_string(), serde_json::Value::Number(n));
        }
        self
    }

    /// Emit all recorded metrics as a single line in metrics.jsonl.
    pub fn emit(self) {
        if self.values.is_empty() {
            return;
        }
        let _guard = tracing::dispatcher::set_default(&self.dispatch);
        let json = serde_json::to_string(&self.values).unwrap_or_default();
        tracing::event!(
            target: "patchbay::_metrics",
            tracing::Level::INFO,
            metrics_json = %json,
        );
    }
}
