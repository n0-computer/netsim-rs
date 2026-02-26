use std::{
    collections::HashMap,
    sync::{Arc, Condvar, Mutex, MutexGuard},
    time::{Duration, Instant},
};

use anyhow::{bail, Result};

/// A single named capture slot.
#[derive(Default, Clone)]
pub struct CaptureSlot {
    /// All matched values, oldest first.
    pub history: Vec<String>,
}

impl CaptureSlot {
    /// Latest matched value, or `None` if never matched.
    pub fn value(&self) -> Option<&str> {
        self.history.last().map(|s| s.as_str())
    }
}

#[derive(Default)]
struct CaptureInner {
    slots: HashMap<String, CaptureSlot>,
}

/// Thread-safe capture store shared between the step loop and pump threads.
#[derive(Clone)]
pub struct CaptureStore {
    inner: Arc<(Mutex<CaptureInner>, Condvar)>,
}

impl Default for CaptureStore {
    fn default() -> Self {
        Self::new()
    }
}

impl CaptureStore {
    /// Creates an empty capture store.
    pub fn new() -> Self {
        Self {
            inner: Arc::new((Mutex::new(CaptureInner::default()), Condvar::new())),
        }
    }

    fn lock(&self) -> MutexGuard<'_, CaptureInner> {
        self.inner.0.lock().unwrap()
    }

    fn lock_cvar(&self) -> (&Mutex<CaptureInner>, &Condvar) {
        (&self.inner.0, &self.inner.1)
    }

    /// Record a new value for a capture key `"step_id.capture_name"`.
    /// Appends to `history`, wakes all waiters.
    pub fn record(&self, key: &str, value: String) {
        let (lock, cvar) = self.lock_cvar();
        let mut inner = lock.lock().unwrap();
        inner
            .slots
            .entry(key.to_string())
            .or_default()
            .history
            .push(value);
        cvar.notify_all();
    }

    /// Block until `key` has at least one value, then return the latest.
    /// Returns `Err` on timeout.
    pub fn wait(&self, key: &str, timeout: Duration) -> Result<String> {
        let deadline = Instant::now() + timeout;
        let (lock, cvar) = self.lock_cvar();
        let mut inner = lock.lock().unwrap();
        loop {
            if let Some(v) = inner.slots.get(key).and_then(|s| s.value()) {
                return Ok(v.to_owned());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                bail!("timeout waiting for capture '{}'", key);
            }
            let (guard, result) = cvar.wait_timeout(inner, remaining).unwrap();
            inner = guard;
            if result.timed_out() {
                bail!("timeout waiting for capture '{}'", key);
            }
        }
    }

    /// Non-blocking latest value for interpolation (returns `None` if unset).
    pub fn get(&self, key: &str) -> Option<String> {
        self.lock().slots.get(key)?.value().map(|s| s.to_owned())
    }
}
