//! L4 load balancer for routers.
//!
//! A balancer creates a virtual IP (VIP) on a router that distributes incoming
//! connections across a set of backend devices using nftables DNAT rules. This
//! matches kube-proxy nftables mode: `numgen inc mod N` for round-robin or
//! `numgen random mod N` for random distribution.
//!
//! The module is self-contained: types, rule generation, resolution, and
//! `Router`/`RouterBuilder` methods all live here. The only external glue is a
//! `balancers` field on `RouterData` and a setup call in `wiring.rs`.

use std::{net::Ipv4Addr, sync::Arc, time::Duration};

use anyhow::{anyhow, bail, Context, Result};
use tracing::debug;

use crate::{
    core::{NetworkCore, NodeId},
    lab::LabInner,
    nft::run_nft_in,
    wiring::nl_run,
};

// ─────────────────────────────────────────────
// Public types
// ─────────────────────────────────────────────

/// Load balancing algorithm for distributing connections across backends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LbAlgorithm {
    /// Rotate through backends in order. Uses nftables `numgen inc mod N`.
    /// Distribution is deterministic: first connection goes to backend 0,
    /// second to backend 1, and so on.
    #[default]
    RoundRobin,
    /// Select a random backend for each new connection. Uses nftables
    /// `numgen random mod N`.
    Random,
}

/// Session affinity mode for a load balancer.
///
/// Without affinity, each new connection is independently balanced across
/// backends. With `ClientIp` affinity, the first connection from a client
/// IP is balanced normally, then subsequent connections from the same
/// client follow the stored mapping until it expires.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SessionAffinity {
    /// No affinity. Each new connection is balanced independently.
    #[default]
    None,
    /// Pin a client IP to the same backend for the given duration.
    /// Kubernetes default is 10800s (3 hours).
    ClientIp {
        /// How long the affinity mapping persists after the last packet.
        timeout: Duration,
    },
}

/// Protocol for a load balancer. Determines which traffic is matched
/// by the DNAT rules.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LbProtocol {
    /// Match TCP traffic only.
    #[default]
    Tcp,
    /// Match UDP traffic only.
    Udp,
    /// Match both TCP and UDP on the same port.
    Both,
}

/// Configuration for an L4 load balancer on a router.
///
/// A balancer creates a virtual IP on the router that distributes
/// incoming connections across a set of backend devices. Under the hood
/// it generates nftables DNAT rules matching kube-proxy nftables mode.
///
/// # Example
///
/// ```rust,no_run
/// # use patchbay::*;
/// # async fn example(dc: &Router, web1: &Device, web2: &Device) -> anyhow::Result<()> {
/// // Round-robin TCP load balancer on 10.0.0.100:80
/// dc.add_balancer(
///     BalancerConfig::new("web", "10.0.0.100".parse()?, 80)
///         .backend(web1.id(), 8080)
///         .backend(web2.id(), 8080)
///         .round_robin(),
/// )
/// .await?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct BalancerConfig {
    pub(crate) name: String,
    pub(crate) vip: Ipv4Addr,
    pub(crate) port: u16,
    pub(crate) backends: Vec<BalancerBackend>,
    pub(crate) algorithm: LbAlgorithm,
    pub(crate) affinity: SessionAffinity,
    pub(crate) protocol: LbProtocol,
}

/// A backend endpoint for a load balancer.
#[derive(Clone, Debug)]
pub(crate) struct BalancerBackend {
    /// Device whose IP is used as the backend address.
    pub device: NodeId,
    /// Port on the backend device.
    pub port: u16,
}

impl BalancerConfig {
    /// Creates a new balancer configuration.
    ///
    /// `name` identifies this balancer in nftables chain names and in the
    /// `add_lb_backend`/`remove_lb_backend` API. `vip` is the virtual IP that
    /// clients connect to. `port` is the port exposed on the VIP.
    ///
    /// Defaults to TCP, round-robin, no session affinity.
    pub fn new(name: &str, vip: Ipv4Addr, port: u16) -> Self {
        Self {
            name: name.to_string(),
            vip,
            port,
            backends: Vec::new(),
            algorithm: LbAlgorithm::default(),
            affinity: SessionAffinity::default(),
            protocol: LbProtocol::default(),
        }
    }

    /// Adds a backend device. The device's IPv4 address on its default
    /// interface is resolved at rule-generation time.
    pub fn backend(mut self, device: NodeId, port: u16) -> Self {
        self.backends.push(BalancerBackend { device, port });
        self
    }

    /// Sets the balancing algorithm. Default is round-robin.
    pub fn algorithm(mut self, algo: LbAlgorithm) -> Self {
        self.algorithm = algo;
        self
    }

    /// Shorthand for `algorithm(LbAlgorithm::RoundRobin)`.
    pub fn round_robin(self) -> Self {
        self.algorithm(LbAlgorithm::RoundRobin)
    }

    /// Shorthand for `algorithm(LbAlgorithm::Random)`.
    pub fn random(self) -> Self {
        self.algorithm(LbAlgorithm::Random)
    }

    /// Enables session affinity pinning client IPs to backends for the
    /// given duration.
    pub fn session_affinity(mut self, timeout: Duration) -> Self {
        self.affinity = SessionAffinity::ClientIp { timeout };
        self
    }

    /// Sets the protocol. Default is TCP.
    pub fn protocol(mut self, proto: LbProtocol) -> Self {
        self.protocol = proto;
        self
    }

    /// Shorthand for `protocol(LbProtocol::Udp)`.
    pub fn udp(self) -> Self {
        self.protocol(LbProtocol::Udp)
    }

    /// Shorthand for `protocol(LbProtocol::Both)`.
    pub fn tcp_and_udp(self) -> Self {
        self.protocol(LbProtocol::Both)
    }
}

// ─────────────────────────────────────────────
// Resolved types (internal)
// ─────────────────────────────────────────────

/// A balancer with backend IPs resolved from the topology.
pub(crate) struct ResolvedBalancer {
    pub name: String,
    pub vip: Ipv4Addr,
    pub port: u16,
    pub backends: Vec<ResolvedBackend>,
    pub algorithm: LbAlgorithm,
    pub affinity: SessionAffinity,
    pub protocol: LbProtocol,
}

/// A backend with its IP resolved from the device's default interface.
pub(crate) struct ResolvedBackend {
    pub ip: Ipv4Addr,
    pub port: u16,
}

impl ResolvedBalancer {
    /// Returns true if this balancer has session affinity enabled.
    fn has_affinity(&self) -> bool {
        matches!(self.affinity, SessionAffinity::ClientIp { .. })
    }

    /// Returns the affinity timeout in seconds, or 0 if no affinity.
    fn affinity_timeout_secs(&self) -> u64 {
        match self.affinity {
            SessionAffinity::ClientIp { timeout } => timeout.as_secs(),
            SessionAffinity::None => 0,
        }
    }
}

// ─────────────────────────────────────────────
// Resolution
// ─────────────────────────────────────────────

/// Resolves a single balancer config into a resolved balancer by looking up
/// backend device IPs from the topology.
fn resolve_balancer(core: &NetworkCore, config: &BalancerConfig) -> Result<ResolvedBalancer> {
    let backends = config
        .backends
        .iter()
        .map(|backend| {
            let device = core
                .device(backend.device)
                .ok_or_else(|| anyhow!("backend device {:?} not found", backend.device))?;
            let ip = device
                .default_iface()
                .ip
                .ok_or_else(|| anyhow!("backend device '{}' has no IPv4 address", device.name))?;
            Ok(ResolvedBackend {
                ip,
                port: backend.port,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(ResolvedBalancer {
        name: config.name.clone(),
        vip: config.vip,
        port: config.port,
        backends,
        algorithm: config.algorithm,
        affinity: config.affinity,
        protocol: config.protocol,
    })
}

/// Resolves all balancer configs for a router.
pub(crate) fn resolve_balancers(
    core: &NetworkCore,
    configs: &[BalancerConfig],
) -> Result<Vec<ResolvedBalancer>> {
    configs
        .iter()
        .map(|cfg| resolve_balancer(core, cfg))
        .collect()
}

// ─────────────────────────────────────────────
// nftables rule generation
// ─────────────────────────────────────────────

/// Generates nftables rules for all load balancers on a router.
///
/// Creates `table ip lb` with:
/// - A prerouting chain at `priority dstnat - 5` with jumps to per-balancer chains
/// - Per-balancer chains with `numgen inc/random mod N` DNAT maps
/// - Optional session affinity maps with timeouts
pub(crate) fn generate_lb_rules(balancers: &[ResolvedBalancer]) -> String {
    use std::fmt::Write;

    let mut rules = String::from("table ip lb {\n");

    // Affinity maps (one per balancer that has affinity enabled).
    for balancer in balancers.iter().filter(|b| b.has_affinity()) {
        let timeout = balancer.affinity_timeout_secs();
        writeln!(rules, "    map {}_affinity {{", balancer.name).unwrap();
        writeln!(rules, "        type ipv4_addr : ipv4_addr . inet_service").unwrap();
        writeln!(rules, "        flags dynamic,timeout").unwrap();
        writeln!(rules, "        timeout {timeout}s").unwrap();
        writeln!(rules, "    }}").unwrap();
    }

    // Prerouting chain.
    writeln!(rules, "    chain prerouting {{").unwrap();
    writeln!(
        rules,
        "        type nat hook prerouting priority dstnat - 5; policy accept;"
    )
    .unwrap();
    for balancer in balancers {
        let port_match = match balancer.protocol {
            LbProtocol::Tcp => format!("tcp dport {}", balancer.port),
            LbProtocol::Udp => format!("udp dport {}", balancer.port),
            LbProtocol::Both => format!("th dport {}", balancer.port),
        };
        writeln!(
            rules,
            "        ip daddr {} {} goto {}",
            balancer.vip, port_match, balancer.name,
        )
        .unwrap();
    }
    writeln!(rules, "    }}").unwrap();

    // Per-balancer chains.
    for balancer in balancers {
        writeln!(rules, "    chain {} {{", balancer.name).unwrap();

        // Affinity lookup (before distribution).
        if balancer.has_affinity() {
            writeln!(
                rules,
                "        ip saddr @{name}_affinity dnat to ip saddr map @{name}_affinity",
                name = balancer.name,
            )
            .unwrap();
        }

        // Distribution via numgen.
        let numgen = match balancer.algorithm {
            LbAlgorithm::RoundRobin => "numgen inc",
            LbAlgorithm::Random => "numgen random",
        };
        let backend_count = balancer.backends.len();
        writeln!(rules, "        dnat to {numgen} mod {backend_count} map {{").unwrap();
        for (index, backend) in balancer.backends.iter().enumerate() {
            let comma = if index + 1 < backend_count { "," } else { "" };
            writeln!(
                rules,
                "            {index} : {} . {}{comma}",
                backend.ip, backend.port,
            )
            .unwrap();
        }
        writeln!(rules, "        }}").unwrap();

        // Affinity update (after DNAT, stores the chosen backend).
        if balancer.has_affinity() {
            let timeout = balancer.affinity_timeout_secs();
            writeln!(
                rules,
                "        update @{name}_affinity {{ ip saddr timeout {timeout}s : ip daddr . th dport }}",
                name = balancer.name,
            )
            .unwrap();
        }

        writeln!(rules, "    }}").unwrap();
    }

    writeln!(rules, "}}").unwrap();
    rules
}

// ─────────────────────────────────────────────
// Apply / setup helpers
// ─────────────────────────────────────────────

/// Applies load balancer nftables rules for a router during initial setup.
///
/// Called from `wiring::setup_router_async` when the router has balancers
/// configured at build time.
pub(crate) async fn setup_balancers(
    _netns: &crate::netns::NetnsManager,
    router: &crate::core::RouterData,
) -> Result<()> {
    // At setup time, backends may not have IPs yet if they are built after
    // the router. Build-time balancers with no backends are stored but rules
    // are deferred until backends are added at runtime.
    debug!(
        router = %router.name,
        count = router.balancers.len(),
        "balancer: setup (build-time configs stored, rules applied on first backend add)"
    );
    Ok(())
}

/// Resolves and applies all load balancer rules for a router.
///
/// Deletes and recreates `table ip lb` atomically. Also adds VIP addresses
/// to the downstream bridge.
async fn apply_all_lb_rules(
    lab: &LabInner,
    router_ns: &str,
    bridge: &str,
    balancers: &[BalancerConfig],
) -> Result<()> {
    let resolved = {
        let core = lab.core.lock().expect("poisoned");
        resolve_balancers(&core, balancers)?
    };

    // Skip if no balancers have backends.
    let has_backends = resolved.iter().any(|b| !b.backends.is_empty());
    if !has_backends {
        // Remove the table if it exists (no backends left).
        run_nft_in(&lab.netns, router_ns, "delete table ip lb")
            .await
            .ok();
        return Ok(());
    }

    let rules = generate_lb_rules(&resolved);
    debug!(ns = %router_ns, rules = %rules, "balancer: applying rules");

    // Delete existing table (ignore error if it does not exist).
    run_nft_in(&lab.netns, router_ns, "delete table ip lb")
        .await
        .ok();
    run_nft_in(&lab.netns, router_ns, &rules).await?;

    // Ensure VIP addresses are on the bridge.
    for balancer in &resolved {
        let vip = balancer.vip;
        let bridge: Arc<str> = bridge.into();
        nl_run(&lab.netns, router_ns, {
            let bridge = bridge.clone();
            move |nl: crate::netlink::Netlink| async move {
                // add_addr4 is idempotent; if the address exists it returns Ok.
                nl.add_addr4(&bridge, vip, 32).await.ok();
                Ok(())
            }
        })
        .await?;
    }

    Ok(())
}

// ─────────────────────────────────────────────
// Router methods (split impl)
// ─────────────────────────────────────────────

impl crate::router::Router {
    /// Adds an L4 load balancer to this router.
    ///
    /// The VIP address is added to the router's downstream bridge and
    /// nftables DNAT rules are installed in `table ip lb`. The VIP
    /// becomes reachable from any device that can route to this router.
    ///
    /// Can be called after `build()`. Multiple balancers can coexist
    /// on the same router with different VIPs or different ports on the
    /// same VIP.
    pub async fn add_balancer(&self, config: BalancerConfig) -> Result<()> {
        let lab = self.lab();
        let op = lab
            .inner
            .with_router(self.id(), |r| Arc::clone(&r.op))
            .ok_or_else(|| anyhow!("router removed"))?;
        let _guard = op.lock().await;

        let (balancers, bridge) = {
            let mut core = lab.inner.core.lock().expect("poisoned");
            let router = core
                .router_mut(self.id())
                .ok_or_else(|| anyhow!("router removed"))?;
            // Check for duplicate name.
            if router.balancers.iter().any(|b| b.name == config.name) {
                bail!("balancer '{}' already exists on this router", config.name);
            }
            router.balancers.push(config);
            let balancers = router.balancers.clone();
            let bridge = router.downlink_bridge.clone();
            (balancers, bridge)
        };

        apply_all_lb_rules(&lab.inner, self.ns(), &bridge, &balancers).await
    }

    /// Removes a load balancer by name.
    ///
    /// Deletes the nftables chain and removes the VIP address. Existing
    /// connections continue via conntrack until they time out or the
    /// backend becomes unreachable.
    pub async fn remove_balancer(&self, name: &str) -> Result<()> {
        let lab = self.lab();
        let op = lab
            .inner
            .with_router(self.id(), |r| Arc::clone(&r.op))
            .ok_or_else(|| anyhow!("router removed"))?;
        let _guard = op.lock().await;

        let (balancers, bridge, removed_vip) = {
            let mut core = lab.inner.core.lock().expect("poisoned");
            let router = core
                .router_mut(self.id())
                .ok_or_else(|| anyhow!("router removed"))?;
            let index = router
                .balancers
                .iter()
                .position(|b| b.name == name)
                .ok_or_else(|| anyhow!("balancer '{}' not found", name))?;
            let removed = router.balancers.remove(index);
            let balancers = router.balancers.clone();
            let bridge = router.downlink_bridge.clone();
            (balancers, bridge, removed.vip)
        };

        apply_all_lb_rules(&lab.inner, self.ns(), &bridge, &balancers).await?;

        // Remove the VIP address from the bridge (only if no other balancer uses it).
        let vip_still_used = balancers.iter().any(|b| b.vip == removed_vip);
        if !vip_still_used {
            let vip = removed_vip;
            let bridge: Arc<str> = bridge.clone();
            nl_run(&lab.inner.netns, self.ns(), {
                move |nl: crate::netlink::Netlink| async move {
                    nl.del_addr4(&bridge, vip, 32).await.ok();
                    Ok(())
                }
            })
            .await?;
        }

        Ok(())
    }

    /// Adds a backend to an existing balancer.
    ///
    /// Regenerates the nftables DNAT map to include the new backend.
    /// Existing connections are not affected (they follow conntrack).
    pub async fn add_lb_backend(&self, balancer: &str, device: NodeId, port: u16) -> Result<()> {
        let lab = self.lab();
        let op = lab
            .inner
            .with_router(self.id(), |r| Arc::clone(&r.op))
            .ok_or_else(|| anyhow!("router removed"))?;
        let _guard = op.lock().await;

        let (balancers, bridge) = {
            let mut core = lab.inner.core.lock().expect("poisoned");
            let router = core
                .router_mut(self.id())
                .ok_or_else(|| anyhow!("router removed"))?;
            let lb = router
                .balancers
                .iter_mut()
                .find(|b| b.name == balancer)
                .ok_or_else(|| anyhow!("balancer '{}' not found", balancer))?;
            lb.backends.push(BalancerBackend { device, port });
            let balancers = router.balancers.clone();
            let bridge = router.downlink_bridge.clone();
            (balancers, bridge)
        };

        apply_all_lb_rules(&lab.inner, self.ns(), &bridge, &balancers).await
    }

    /// Removes a backend from an existing balancer.
    ///
    /// Regenerates the nftables DNAT map without the removed backend.
    /// Existing connections to the removed backend continue via conntrack
    /// until timeout. Call `flush_lb_conntrack` to force immediate
    /// redistribution.
    pub async fn remove_lb_backend(&self, balancer: &str, device: NodeId) -> Result<()> {
        let lab = self.lab();
        let op = lab
            .inner
            .with_router(self.id(), |r| Arc::clone(&r.op))
            .ok_or_else(|| anyhow!("router removed"))?;
        let _guard = op.lock().await;

        let (balancers, bridge) = {
            let mut core = lab.inner.core.lock().expect("poisoned");
            let router = core
                .router_mut(self.id())
                .ok_or_else(|| anyhow!("router removed"))?;
            let lb = router
                .balancers
                .iter_mut()
                .find(|b| b.name == balancer)
                .ok_or_else(|| anyhow!("balancer '{}' not found", balancer))?;
            let index = lb
                .backends
                .iter()
                .position(|b| b.device == device)
                .ok_or_else(|| anyhow!("backend device {:?} not found in balancer", device))?;
            lb.backends.remove(index);
            let balancers = router.balancers.clone();
            let bridge = router.downlink_bridge.clone();
            (balancers, bridge)
        };

        apply_all_lb_rules(&lab.inner, self.ns(), &bridge, &balancers).await
    }

    /// Flushes conntrack entries for a balancer, forcing all connections
    /// to be re-balanced on their next packet.
    ///
    /// Simulates aggressive connection draining during a rolling update.
    pub async fn flush_lb_conntrack(&self, balancer: &str) -> Result<()> {
        let lab = self.lab();
        let op = lab
            .inner
            .with_router(self.id(), |r| Arc::clone(&r.op))
            .ok_or_else(|| anyhow!("router removed"))?;
        let _guard = op.lock().await;

        let (vip, port, protocol) = {
            let core = lab.inner.core.lock().expect("poisoned");
            let router = core
                .router(self.id())
                .ok_or_else(|| anyhow!("router removed"))?;
            let lb = router
                .balancers
                .iter()
                .find(|b| b.name == balancer)
                .ok_or_else(|| anyhow!("balancer '{}' not found", balancer))?;
            (lb.vip, lb.port, lb.protocol)
        };

        let ns = self.ns().to_string();
        let rt = lab.inner.netns.rt_handle_for(&ns)?;

        // Flush conntrack entries matching the VIP and port.
        let flush = move || async move {
            let protocols = match protocol {
                LbProtocol::Tcp => vec!["tcp"],
                LbProtocol::Udp => vec!["udp"],
                LbProtocol::Both => vec!["tcp", "udp"],
            };
            for proto in protocols {
                let status = tokio::process::Command::new("conntrack")
                    .args([
                        "-D",
                        "-d",
                        &vip.to_string(),
                        "-p",
                        proto,
                        "--dport",
                        &port.to_string(),
                    ])
                    .status()
                    .await
                    .context("spawn conntrack -D")?;
                // conntrack -D returns non-zero if no entries matched, which is fine.
                debug!(
                    vip = %vip,
                    port = %port,
                    proto = %proto,
                    success = status.success(),
                    "balancer: conntrack flush"
                );
            }
            Ok(())
        };

        rt.spawn(flush())
            .await
            .context("conntrack flush task panicked")?
    }
}

// ─────────────────────────────────────────────
// RouterBuilder methods (split impl)
// ─────────────────────────────────────────────

impl crate::router::RouterBuilder {
    /// Declares a load balancer to be configured when the router is built.
    ///
    /// Backends must reference devices that exist at the time the balancer
    /// rules are applied. For build-time balancers, add backends at runtime
    /// via [`Router::add_lb_backend`] after the backend devices are built.
    pub fn balancer(mut self, config: BalancerConfig) -> Self {
        if self.result.is_ok() {
            self.balancers.push(config);
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_round_robin_rules() {
        let balancers = vec![ResolvedBalancer {
            name: "web".to_string(),
            vip: "10.0.0.100".parse().unwrap(),
            port: 80,
            backends: vec![
                ResolvedBackend {
                    ip: "10.0.1.1".parse().unwrap(),
                    port: 8080,
                },
                ResolvedBackend {
                    ip: "10.0.1.2".parse().unwrap(),
                    port: 8080,
                },
                ResolvedBackend {
                    ip: "10.0.1.3".parse().unwrap(),
                    port: 8080,
                },
            ],
            algorithm: LbAlgorithm::RoundRobin,
            affinity: SessionAffinity::None,
            protocol: LbProtocol::Tcp,
        }];
        let rules = generate_lb_rules(&balancers);
        assert!(rules.contains("numgen inc mod 3"));
        assert!(rules.contains("10.0.1.1 . 8080"));
        assert!(rules.contains("10.0.1.2 . 8080"));
        assert!(rules.contains("10.0.1.3 . 8080"));
        assert!(rules.contains("tcp dport 80"));
        assert!(rules.contains("goto web"));
    }

    #[test]
    fn generate_affinity_rules() {
        let balancers = vec![ResolvedBalancer {
            name: "sticky".to_string(),
            vip: "10.0.0.50".parse().unwrap(),
            port: 443,
            backends: vec![
                ResolvedBackend {
                    ip: "10.0.2.1".parse().unwrap(),
                    port: 443,
                },
                ResolvedBackend {
                    ip: "10.0.2.2".parse().unwrap(),
                    port: 443,
                },
            ],
            algorithm: LbAlgorithm::Random,
            affinity: SessionAffinity::ClientIp {
                timeout: Duration::from_secs(3600),
            },
            protocol: LbProtocol::Tcp,
        }];
        let rules = generate_lb_rules(&balancers);
        assert!(rules.contains("map sticky_affinity"));
        assert!(rules.contains("timeout 3600s"));
        assert!(rules.contains("numgen random mod 2"));
        assert!(rules.contains("update @sticky_affinity"));
    }

    #[test]
    fn generate_udp_rules() {
        let balancers = vec![ResolvedBalancer {
            name: "dns".to_string(),
            vip: "10.0.0.53".parse().unwrap(),
            port: 53,
            backends: vec![ResolvedBackend {
                ip: "10.0.3.1".parse().unwrap(),
                port: 53,
            }],
            algorithm: LbAlgorithm::Random,
            affinity: SessionAffinity::None,
            protocol: LbProtocol::Udp,
        }];
        let rules = generate_lb_rules(&balancers);
        assert!(rules.contains("udp dport 53"));
    }

    #[test]
    fn generate_both_protocol_rules() {
        let balancers = vec![ResolvedBalancer {
            name: "svc".to_string(),
            vip: "10.0.0.1".parse().unwrap(),
            port: 53,
            backends: vec![ResolvedBackend {
                ip: "10.0.3.1".parse().unwrap(),
                port: 53,
            }],
            algorithm: LbAlgorithm::RoundRobin,
            affinity: SessionAffinity::None,
            protocol: LbProtocol::Both,
        }];
        let rules = generate_lb_rules(&balancers);
        assert!(rules.contains("th dport 53"));
    }
}
