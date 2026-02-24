//! TOML configuration structures used by [`crate::Lab::load`].

use crate::NatMode;
use serde::Deserialize;
use std::collections::HashMap;

/// Parsed lab configuration from TOML.
#[derive(Deserialize, Clone, Default)]
pub struct LabConfig {
    /// Optional region-latency map.
    pub region: Option<HashMap<String, RegionConfig>>,
    /// Router entries.
    #[serde(default)]
    pub router: Vec<RouterConfig>,
    /// Raw device tables; post-processed by [`crate::Lab::from_config`].
    #[serde(default)]
    pub device: HashMap<String, toml::Value>,
}

/// Per-region latency configuration.
#[derive(Deserialize, Clone)]
pub struct RegionConfig {
    /// Map of target-region name → one-way latency in ms.
    #[serde(default)]
    pub latencies: HashMap<String, u32>,
}

/// Router configuration entry.
#[derive(Deserialize, Clone)]
pub struct RouterConfig {
    /// Router name.
    pub name: String,
    /// Optional region tag (used for inter-region latency rules).
    pub region: Option<String>,
    /// Name of the upstream router.  If absent the router attaches to the IX switch.
    pub upstream: Option<String>,
    /// NAT mode.  Defaults to `"none"` (public downstream, no NAT).
    #[serde(default)]
    pub nat: NatMode,
}
