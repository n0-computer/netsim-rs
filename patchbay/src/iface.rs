//! Unified interface configuration and runtime handle.

use ipnet::{Ipv4Net, Ipv6Net};

use crate::{
    core::NodeId,
    lab::{LinkCondition, LinkDirection},
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
