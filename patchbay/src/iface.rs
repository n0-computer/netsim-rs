//! Unified interface configuration and runtime handle.

use std::sync::Arc;

use anyhow::{anyhow, bail, Result};
use ipnet::{Ipv4Net, Ipv6Net};

use crate::{
    core::NodeId,
    lab::{LabInner, LinkCondition, LinkDirection},
};

/// Unified configuration for a device network interface.
///
/// Describes how an interface should be set up, used identically by
/// [`DeviceBuilder::iface`](crate::DeviceBuilder::iface) and
/// [`Device::add_iface`](crate::Device::add_iface). Methods take and
/// return `self` by value for chaining. The struct is `Copy`.
///
/// # Routed interfaces
///
/// A routed interface connects to a router's downstream bridge via a
/// veth pair. IPv4 and IPv6 addresses are allocated from the router's
/// pool unless overridden with [`addr`](Self::addr) or
/// [`addr_v6`](Self::addr_v6).
///
/// # Isolated interfaces
///
/// An isolated interface uses a Linux dummy device with no bridge
/// attachment. It has no gateway and no pool-allocated addresses.
/// Use [`addr`](Self::addr) and/or [`addr_v6`](Self::addr_v6) to
/// assign addresses explicitly.
#[derive(Clone, Copy, Debug)]
pub struct IfaceConfig {
    /// Router whose downstream bridge this interface connects to.
    /// None for isolated interfaces.
    pub(crate) gateway: Option<NodeId>,
    /// Explicit IPv4 address with prefix length.
    pub(crate) addr: Option<Ipv4Net>,
    /// Explicit IPv6 address with prefix length.
    pub(crate) addr_v6: Option<Ipv6Net>,
    /// Initial egress impairment (device-side veth / dummy device).
    pub(crate) egress: Option<LinkCondition>,
    /// Initial ingress impairment (bridge-side veth). Ignored for isolated.
    pub(crate) ingress: Option<LinkCondition>,
    /// If true, the interface is created in link-down state.
    pub(crate) start_down: bool,
}

impl IfaceConfig {
    /// Routed interface connected to a router's downstream bridge.
    ///
    /// IPv4 allocated from the router's pool. IPv6 allocated if the
    /// router supports dual-stack. Use [`addr`](Self::addr) /
    /// [`addr_v6`](Self::addr_v6) to override with explicit addresses.
    pub fn routed(router: NodeId) -> Self {
        Self {
            gateway: Some(router),
            addr: None,
            addr_v6: None,
            egress: None,
            ingress: None,
            start_down: false,
        }
    }

    /// Isolated interface. Not connected to any bridge. Uses a Linux
    /// dummy device internally. No addresses by default. Use
    /// [`addr`](Self::addr) and/or [`addr_v6`](Self::addr_v6) to
    /// assign addresses.
    pub fn isolated() -> Self {
        Self {
            gateway: None,
            addr: None,
            addr_v6: None,
            egress: None,
            ingress: None,
            start_down: false,
        }
    }

    /// Sets an explicit IPv4 address with prefix length. On routed
    /// interfaces, overrides pool allocation. On isolated interfaces,
    /// configures the address.
    pub fn addr(mut self, addr: Ipv4Net) -> Self {
        self.addr = Some(addr);
        self
    }

    /// Sets an explicit IPv6 address with prefix length.
    pub fn addr_v6(mut self, addr: Ipv6Net) -> Self {
        self.addr_v6 = Some(addr);
        self
    }

    /// Sets a link condition for the given direction.
    ///
    /// `Egress` applies to the device-side veth (or dummy device for
    /// isolated interfaces). `Ingress` applies to the bridge-side veth
    /// (ignored for isolated interfaces). `Both` sets the same condition
    /// on both sides.
    ///
    /// Can be called multiple times. Each call replaces the condition for
    /// the specified direction(s).
    pub fn condition(mut self, condition: LinkCondition, direction: LinkDirection) -> Self {
        match direction {
            LinkDirection::Egress => self.egress = Some(condition),
            LinkDirection::Ingress => self.ingress = Some(condition),
            LinkDirection::Both => {
                self.egress = Some(condition);
                self.ingress = Some(condition);
            }
        }
        self
    }

    /// Creates the interface in link-down state.
    pub fn down(mut self) -> Self {
        self.start_down = true;
        self
    }
}

/// [`NodeId`] converts to a simple routed interface with pool-allocated IP.
impl From<NodeId> for IfaceConfig {
    fn from(router: NodeId) -> Self {
        Self::routed(router)
    }
}

// ─────────────────────────────────────────────
// Iface handle
// ─────────────────────────────────────────────

/// Cloneable handle to a single device interface.
///
/// Follows the same pattern as [`Device`](crate::Device) and
/// [`Router`](crate::Router): holds `Arc<LabInner>` plus identifiers,
/// and every method briefly locks the core mutex to read or write.
///
/// Returned by [`Device::iface`](crate::Device::iface),
/// [`Device::add_iface`](crate::Device::add_iface), and
/// [`Device::interfaces`](crate::Device::interfaces).
///
/// If the interface is removed while a handle exists, methods return
/// `Err("interface removed")`.
#[derive(Clone)]
pub struct Iface {
    device: NodeId,
    ifname: Arc<str>,
    lab: Arc<LabInner>,
}

impl std::fmt::Debug for Iface {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Iface")
            .field("device", &self.device)
            .field("ifname", &self.ifname)
            .finish()
    }
}

impl Iface {
    pub(crate) fn new(device: NodeId, ifname: Arc<str>, lab: Arc<LabInner>) -> Self {
        Self {
            device,
            ifname,
            lab,
        }
    }

    /// Returns the interface name (e.g. `"eth0"`).
    pub fn name(&self) -> &str {
        &self.ifname
    }

    /// Returns the device [`NodeId`] this interface belongs to.
    pub fn device_id(&self) -> NodeId {
        self.device
    }

    /// Returns the assigned IPv4 address, if any.
    pub fn ip(&self) -> Option<std::net::Ipv4Addr> {
        let inner = self.lab.core.lock().expect("poisoned");
        inner
            .device(self.device)
            .and_then(|d| d.iface(&self.ifname))
            .and_then(|i| i.ip)
    }

    /// Returns the assigned IPv6 address, if any.
    pub fn ip6(&self) -> Option<std::net::Ipv6Addr> {
        let inner = self.lab.core.lock().expect("poisoned");
        inner
            .device(self.device)
            .and_then(|d| d.iface(&self.ifname))
            .and_then(|i| i.ip_v6)
    }

    /// Returns the assigned IPv6 link-local address, if any.
    pub fn ll6(&self) -> Option<std::net::Ipv6Addr> {
        let inner = self.lab.core.lock().expect("poisoned");
        inner
            .device(self.device)
            .and_then(|d| d.iface(&self.ifname))
            .and_then(|i| i.ll_v6)
    }

    /// Returns the egress link condition, if any.
    pub fn egress(&self) -> Option<LinkCondition> {
        let inner = self.lab.core.lock().expect("poisoned");
        inner
            .device(self.device)
            .and_then(|d| d.iface(&self.ifname))
            .and_then(|i| i.egress)
    }

    /// Returns the ingress link condition, if any.
    pub fn ingress(&self) -> Option<LinkCondition> {
        let inner = self.lab.core.lock().expect("poisoned");
        inner
            .device(self.device)
            .and_then(|d| d.iface(&self.ifname))
            .and_then(|i| i.ingress)
    }

    /// Returns `true` if this interface is routed (connected to a router).
    pub fn is_routed(&self) -> bool {
        let inner = self.lab.core.lock().expect("poisoned");
        inner
            .device(self.device)
            .and_then(|d| d.iface(&self.ifname))
            .map(|i| !i.isolated)
            .unwrap_or(false)
    }

    /// Returns `true` if this interface is isolated (dummy device, no bridge).
    pub fn is_isolated(&self) -> bool {
        let inner = self.lab.core.lock().expect("poisoned");
        inner
            .device(self.device)
            .and_then(|d| d.iface(&self.ifname))
            .map(|i| i.isolated)
            .unwrap_or(false)
    }

    // ── Mutate: link conditions ──

    /// Sets a link condition on this interface for the given direction.
    ///
    /// For isolated interfaces, `Ingress` returns an error (no bridge-side
    /// veth). `Both` sets egress only and silently skips ingress.
    pub async fn set_condition(
        &self,
        condition: LinkCondition,
        direction: LinkDirection,
    ) -> Result<()> {
        use crate::nft::apply_or_remove_impair;

        let (dev_ns, gw_ns, gw_ifname, isolated, op) = {
            let inner = self.lab.core.lock().expect("poisoned");
            let dev = inner
                .device(self.device)
                .ok_or_else(|| anyhow!("device removed"))?;
            let iface = dev
                .iface(&self.ifname)
                .ok_or_else(|| anyhow!("interface '{}' removed", self.ifname))?;
            let op = Arc::clone(&dev.op);
            let isolated = iface.isolated;

            if isolated && matches!(direction, LinkDirection::Ingress) {
                bail!(
                    "cannot set ingress condition on isolated interface '{}' \
                     (no bridge-side veth)",
                    self.ifname
                );
            }

            let (gw_ns, gw_ifname) = if !isolated {
                let uplink = iface.uplink.expect("routed interface has uplink");
                let gw_router = inner
                    .switch(uplink)
                    .and_then(|sw| sw.owner_router)
                    .and_then(|rid| inner.router(rid))
                    .ok_or_else(|| {
                        anyhow!("gateway router not found for interface '{}'", self.ifname)
                    })?;
                (gw_router.ns.clone(), format!("v{}", iface.idx))
            } else {
                (Arc::from(""), String::new())
            };

            (dev.ns.clone(), gw_ns, gw_ifname, isolated, op)
        };
        let _guard = op.lock().await;

        // Apply egress condition.
        if matches!(direction, LinkDirection::Egress | LinkDirection::Both) {
            apply_or_remove_impair(&self.lab.netns, &dev_ns, &self.ifname, Some(condition)).await;
        }

        // Apply ingress condition (skip silently for isolated + Both).
        if !isolated && matches!(direction, LinkDirection::Ingress | LinkDirection::Both) {
            apply_or_remove_impair(&self.lab.netns, &gw_ns, &gw_ifname, Some(condition)).await;
        }

        // Update stored state.
        {
            let mut inner = self.lab.core.lock().expect("poisoned");
            if let Some(dev) = inner.device_mut(self.device) {
                if let Some(iface) = dev.iface_mut(&self.ifname) {
                    match direction {
                        LinkDirection::Egress => iface.egress = Some(condition),
                        LinkDirection::Ingress => iface.ingress = Some(condition),
                        LinkDirection::Both => {
                            iface.egress = Some(condition);
                            if !isolated {
                                iface.ingress = Some(condition);
                            }
                        }
                    }
                }
            }
        }

        self.lab
            .emit(crate::event::LabEventKind::LinkConditionChanged {
                device: self.device_name(),
                iface: self.ifname.to_string(),
                condition: Some(condition),
                direction,
            });
        Ok(())
    }

    /// Removes any link condition for the given direction.
    ///
    /// For isolated interfaces, `Ingress` returns an error. `Both` clears
    /// egress only and silently skips ingress.
    pub async fn clear_condition(&self, direction: LinkDirection) -> Result<()> {
        use crate::nft::apply_or_remove_impair;

        let (dev_ns, gw_ns, gw_ifname, isolated, op) = {
            let inner = self.lab.core.lock().expect("poisoned");
            let dev = inner
                .device(self.device)
                .ok_or_else(|| anyhow!("device removed"))?;
            let iface = dev
                .iface(&self.ifname)
                .ok_or_else(|| anyhow!("interface '{}' removed", self.ifname))?;
            let op = Arc::clone(&dev.op);
            let isolated = iface.isolated;

            if isolated && matches!(direction, LinkDirection::Ingress) {
                bail!(
                    "cannot clear ingress condition on isolated interface '{}' \
                     (no bridge-side veth)",
                    self.ifname
                );
            }

            let (gw_ns, gw_ifname) = if !isolated {
                let uplink = iface.uplink.expect("routed interface has uplink");
                let gw_router = inner
                    .switch(uplink)
                    .and_then(|sw| sw.owner_router)
                    .and_then(|rid| inner.router(rid))
                    .ok_or_else(|| {
                        anyhow!("gateway router not found for interface '{}'", self.ifname)
                    })?;
                (gw_router.ns.clone(), format!("v{}", iface.idx))
            } else {
                (Arc::from(""), String::new())
            };

            (dev.ns.clone(), gw_ns, gw_ifname, isolated, op)
        };
        let _guard = op.lock().await;

        if matches!(direction, LinkDirection::Egress | LinkDirection::Both) {
            apply_or_remove_impair(&self.lab.netns, &dev_ns, &self.ifname, None).await;
        }
        if !isolated && matches!(direction, LinkDirection::Ingress | LinkDirection::Both) {
            apply_or_remove_impair(&self.lab.netns, &gw_ns, &gw_ifname, None).await;
        }

        {
            let mut inner = self.lab.core.lock().expect("poisoned");
            if let Some(dev) = inner.device_mut(self.device) {
                if let Some(iface) = dev.iface_mut(&self.ifname) {
                    match direction {
                        LinkDirection::Egress => iface.egress = None,
                        LinkDirection::Ingress => iface.ingress = None,
                        LinkDirection::Both => {
                            iface.egress = None;
                            if !isolated {
                                iface.ingress = None;
                            }
                        }
                    }
                }
            }
        }

        self.lab
            .emit(crate::event::LabEventKind::LinkConditionChanged {
                device: self.device_name(),
                iface: self.ifname.to_string(),
                condition: None,
                direction,
            });
        Ok(())
    }

    // ── Mutate: link state ──

    /// Brings this interface administratively down.
    pub async fn link_down(&self) -> Result<()> {
        use crate::{netlink::Netlink, wiring};

        let ns = {
            let inner = self.lab.core.lock().expect("poisoned");
            let dev = inner
                .device(self.device)
                .ok_or_else(|| anyhow!("device removed"))?;
            let _ = dev
                .iface(&self.ifname)
                .ok_or_else(|| anyhow!("interface '{}' removed", self.ifname))?;
            dev.ns.clone()
        };
        let ifname = self.ifname.to_string();
        wiring::nl_run(&self.lab.netns, &ns, move |nl: Netlink| async move {
            nl.set_link_down(&ifname).await
        })
        .await?;
        self.lab.emit(crate::event::LabEventKind::LinkDown {
            device: self.device_name(),
            iface: self.ifname.to_string(),
        });
        Ok(())
    }

    /// Brings this interface administratively up.
    ///
    /// If this is the device's default route interface (and it is routed),
    /// the default route is re-added (Linux removes routes when a link
    /// goes down).
    pub async fn link_up(&self) -> Result<()> {
        use crate::{
            device::select_default_v6_gateway, netlink::Netlink, wiring, Ipv6ProvisioningMode,
        };

        let (ns, uplink, is_default_via, isolated) = {
            let inner = self.lab.core.lock().expect("poisoned");
            let dev = inner
                .device(self.device)
                .ok_or_else(|| anyhow!("device removed"))?;
            let iface = dev
                .iface(&self.ifname)
                .ok_or_else(|| anyhow!("interface '{}' removed", self.ifname))?;
            (
                dev.ns.clone(),
                iface.uplink,
                *dev.default_via == *self.ifname,
                iface.isolated,
            )
        };

        let ifname = self.ifname.to_string();
        wiring::nl_run(&self.lab.netns, &ns, {
            let ifname = ifname.clone();
            move |nl: Netlink| async move { nl.set_link_up(&ifname).await }
        })
        .await?;

        if is_default_via && !isolated {
            let uplink = uplink.expect("routed default-via interface has uplink");
            let provisioning = {
                let inner = self.lab.core.lock().expect("poisoned");
                let dev = inner
                    .device(self.device)
                    .ok_or_else(|| anyhow!("device removed"))?;
                dev.provisioning_mode
                    .unwrap_or(self.lab.ipv6_provisioning_mode)
            };
            let (gw_ip, gw_v6, gw_ll_v6, ra_default_enabled) = {
                let inner = self.lab.core.lock().expect("poisoned");
                let gw_ip = inner.router_downlink_gw_for_switch(uplink)?;
                let gw_v6 = inner.router_downlink_gw6_for_switch(uplink)?;
                let ra_default_enabled = inner.ra_default_enabled_for_switch(uplink)?;
                (
                    gw_ip,
                    gw_v6.global_v6,
                    gw_v6.link_local_v6,
                    ra_default_enabled,
                )
            };
            let primary_v6 =
                select_default_v6_gateway(provisioning, ra_default_enabled, gw_v6, gw_ll_v6);
            let ifname_route = ifname.clone();
            wiring::nl_run(&self.lab.netns, &ns, move |nl: Netlink| async move {
                nl.replace_default_route_v4(&ifname_route, gw_ip).await?;
                nl.set_default_route_v6(&ifname_route, primary_v6).await
            })
            .await?;
            if provisioning == Ipv6ProvisioningMode::RaDriven {
                let rs_router_ll = if ra_default_enabled { gw_ll_v6 } else { None };
                wiring::emit_router_solicitation(
                    &self.lab.netns,
                    ns.to_string(),
                    self.device_name(),
                    ifname.clone(),
                    rs_router_ll,
                )
                .await?;
            }
        }

        self.lab.emit(crate::event::LabEventKind::LinkUp {
            device: self.device_name(),
            iface: self.ifname.to_string(),
        });
        Ok(())
    }

    // ── Mutate: addressing ──

    /// Adds a secondary IPv4 address to this interface.
    pub async fn add_ip(&self, ip: std::net::Ipv4Addr, prefix_len: u8) -> Result<()> {
        use crate::{netlink::Netlink, wiring};

        let ns = {
            let inner = self.lab.core.lock().expect("poisoned");
            let dev = inner
                .device(self.device)
                .ok_or_else(|| anyhow!("device removed"))?;
            let _ = dev
                .iface(&self.ifname)
                .ok_or_else(|| anyhow!("interface '{}' removed", self.ifname))?;
            dev.ns.clone()
        };
        let ifname = self.ifname.to_string();
        wiring::nl_run(&self.lab.netns, &ns, move |nl: Netlink| async move {
            nl.add_addr4(&ifname, ip, prefix_len).await?;
            Ok(())
        })
        .await
    }

    /// Simulates DHCP renewal: allocates a new IP from the current
    /// router's pool, replaces the old address, and returns the new
    /// address.
    ///
    /// Returns an error on isolated interfaces (no pool to allocate from).
    pub async fn renew_ip(&self) -> Result<std::net::Ipv4Addr> {
        use crate::{netlink::Netlink, wiring};

        if self.is_isolated() {
            bail!("cannot renew IP on isolated interface '{}'", self.ifname);
        }

        let (ns, old_ip, new_ip, prefix_len) = self
            .lab
            .core
            .lock()
            .expect("poisoned")
            .renew_device_ip(self.device, &self.ifname)?;

        let ifname = self.ifname.to_string();
        wiring::nl_run(&self.lab.netns, &ns, move |nl: Netlink| async move {
            nl.del_addr4(&ifname, old_ip, prefix_len).await?;
            nl.add_addr4(&ifname, new_ip, prefix_len).await?;
            Ok(())
        })
        .await?;

        self.lab.emit(crate::event::LabEventKind::DeviceIpChanged {
            device: self.device_name(),
            iface_name: self.ifname.to_string(),
            new_ip: Some(new_ip),
            new_ip_v6: None,
        });

        Ok(new_ip)
    }

    // ── Mutate: topology ──

    /// Moves this interface to a different router's downstream network.
    ///
    /// Returns an error on isolated interfaces (nothing to replug to).
    pub async fn replug(&self, to_router: NodeId) -> Result<()> {
        use crate::{event::LabEventKind, netlink::Netlink, wiring, Ipv6ProvisioningMode};

        if self.is_isolated() {
            bail!("cannot replug isolated interface '{}'", self.ifname);
        }

        let op = self
            .lab
            .with_device(self.device, |d| Arc::clone(&d.op))
            .ok_or_else(|| anyhow!("device removed"))?;
        let _guard = op.lock().await;

        let provisioning = {
            let inner = self.lab.core.lock().expect("poisoned");
            let dev = inner
                .device(self.device)
                .ok_or_else(|| anyhow!("device removed"))?;
            dev.provisioning_mode
                .unwrap_or(self.lab.ipv6_provisioning_mode)
        };

        let mut setup = self
            .lab
            .core
            .lock()
            .expect("poisoned")
            .prepare_replug_iface(self.device, &self.ifname, to_router)?;
        if provisioning == Ipv6ProvisioningMode::RaDriven {
            setup.iface_build.gw_ip_v6 = None;
        }

        // Delete old veth pair.
        let dev_ns = setup.iface_build.dev_ns.clone();
        let ifname = self.ifname.to_string();
        let netns = &self.lab.netns;
        wiring::nl_run(netns, &dev_ns, move |h: Netlink| async move {
            h.ensure_link_deleted(&ifname).await.ok();
            Ok(())
        })
        .await?;

        // Wire new interface.
        let new_ip = setup.iface_build.dev_ip;
        let new_ip_v6 = setup.iface_build.dev_ip_v6;
        wiring::wire_iface_async(netns, &setup.prefix, &setup.root_ns, setup.iface_build).await?;

        // Get old router name.
        let from_router_name = {
            let inner = self.lab.core.lock().expect("poisoned");
            let dev = inner.device(self.device);
            let old_uplink = dev
                .and_then(|d| d.iface(&self.ifname))
                .and_then(|i| i.uplink);
            old_uplink
                .and_then(|sw| inner.switch(sw))
                .and_then(|sw| sw.owner_router)
                .and_then(|r| inner.router(r))
                .map(|r| r.name.to_string())
                .unwrap_or_default()
        };

        self.lab
            .core
            .lock()
            .expect("poisoned")
            .finish_replug_iface(self.device, &self.ifname, to_router, new_ip, new_ip_v6)?;

        let to_router_name = self
            .lab
            .core
            .lock()
            .expect("poisoned")
            .router(to_router)
            .map(|r| r.name.to_string())
            .unwrap_or_default();

        self.lab.emit(LabEventKind::InterfaceReplugged {
            device: self.device_name(),
            iface_name: self.ifname.to_string(),
            from_router: from_router_name,
            to_router: to_router_name,
            new_ip,
            new_ip_v6,
        });

        Ok(())
    }

    /// Removes this interface from its device.
    ///
    /// If this was the default route interface, the default switches to
    /// the first remaining interface.
    pub async fn remove(&self) -> Result<()> {
        use crate::{netlink::Netlink, wiring};

        let op = self
            .lab
            .with_device(self.device, |d| Arc::clone(&d.op))
            .ok_or_else(|| anyhow!("device removed"))?;
        let _guard = op.lock().await;

        let dev_ns = self
            .lab
            .core
            .lock()
            .expect("poisoned")
            .remove_device_iface(self.device, &self.ifname)?;

        let ifname = self.ifname.to_string();
        wiring::nl_run(&self.lab.netns, &dev_ns, move |h: Netlink| async move {
            h.ensure_link_deleted(&ifname).await.ok();
            Ok(())
        })
        .await?;

        self.lab.emit(crate::event::LabEventKind::InterfaceRemoved {
            device: self.device_name(),
            iface_name: self.ifname.to_string(),
        });
        Ok(())
    }

    // ── Internal helpers ──

    fn device_name(&self) -> String {
        let inner = self.lab.core.lock().expect("poisoned");
        inner
            .device(self.device)
            .map(|d| d.name.to_string())
            .unwrap_or_default()
    }
}
