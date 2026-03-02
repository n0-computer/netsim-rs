//! Firewall presets and configuration types.

/// Firewall preset for a router's forward chain.
///
/// Firewall rules restrict which traffic can traverse the router.
/// They are applied as nftables rules in a separate `inet fw` table
/// (priority 10, after NAT filter rules at priority 0). Rules apply
/// to both IPv4 and IPv6 traffic.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Firewall {
    /// No filtering beyond NAT (default).
    #[default]
    None,

    /// Block unsolicited inbound traffic on the WAN interface (RFC 6092).
    ///
    /// Allows all outbound traffic and return traffic for established flows.
    /// Drops new connections arriving from the WAN side. This is the default
    /// security posture of every home router and IPv6 CE router.
    ///
    /// For IPv4 with NAT, this is redundant (NAT + APDF already blocks
    /// inbound). For IPv6 without NAT, this is the primary security boundary
    /// — devices have globally routable addresses but are not reachable from
    /// the internet.
    ///
    /// Observed on: every home router (FritzBox, Unifi, TP-Link, etc.).
    BlockInbound,

    /// Corporate/enterprise firewall.
    ///
    /// Allows TCP 80, 443 and UDP 53 (DNS). Blocks all other TCP and UDP.
    /// STUN/ICE fails, must use TURN-over-TLS on port 443.
    ///
    /// Observed on: Cisco ASA, Palo Alto, Fortinet in enterprise deployments.
    Corporate,

    /// Hotel/airport captive-portal style firewall.
    ///
    /// Allows TCP 80, 443, 53 and UDP 53. Blocks all other UDP.
    /// TCP to other ports is allowed (unlike Corporate).
    ///
    /// Observed on: hotel/airport guest WiFi after captive portal auth.
    CaptivePortal,

    /// Fully custom firewall configuration.
    Custom(FirewallConfig),
}

/// Custom firewall configuration for per-port allow/block rules.
///
/// # Example
/// ```
/// # use patchbay::FirewallConfig;
/// let cfg = FirewallConfig::builder()
///     .allow_tcp(&[80, 443, 8443])
///     .allow_udp(&[53])
///     .block_udp()
///     .build();
/// ```
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FirewallConfig {
    /// Allowed outbound TCP destination ports. Empty + block_tcp = block all TCP.
    pub allow_tcp: Vec<u16>,
    /// Allowed outbound UDP destination ports. Empty + block_udp = block all UDP.
    pub allow_udp: Vec<u16>,
    /// If true, block TCP not in `allow_tcp`.
    pub block_tcp: bool,
    /// If true, block UDP not in `allow_udp`.
    pub block_udp: bool,
}

impl Firewall {
    /// Expands a preset to a [`FirewallConfig`], or returns `None` for [`Firewall::None`].
    pub fn to_config(&self) -> Option<FirewallConfig> {
        match self {
            Firewall::None | Firewall::BlockInbound => None,
            Firewall::Corporate => Some(FirewallConfig {
                allow_tcp: vec![80, 443],
                allow_udp: vec![53],
                block_tcp: true,
                block_udp: true,
            }),
            Firewall::CaptivePortal => Some(FirewallConfig {
                allow_tcp: vec![80, 443, 53],
                allow_udp: vec![53],
                block_tcp: false,
                block_udp: true,
            }),
            Firewall::Custom(cfg) => Some(cfg.clone()),
        }
    }
}

impl FirewallConfig {
    /// Returns a builder for constructing a custom firewall configuration.
    pub fn builder() -> FirewallConfigBuilder {
        FirewallConfigBuilder::default()
    }
}

/// Builder for [`FirewallConfig`].
#[derive(Clone, Debug, Default)]
pub struct FirewallConfigBuilder {
    allow_tcp: Vec<u16>,
    allow_udp: Vec<u16>,
    block_tcp: bool,
    block_udp: bool,
}

impl FirewallConfigBuilder {
    /// Allow outbound TCP to these destination ports.
    pub fn allow_tcp(&mut self, ports: &[u16]) -> &mut Self {
        self.allow_tcp.extend_from_slice(ports);
        self
    }

    /// Allow outbound UDP to these destination ports.
    pub fn allow_udp(&mut self, ports: &[u16]) -> &mut Self {
        self.allow_udp.extend_from_slice(ports);
        self
    }

    /// Block all outbound TCP not in the allow list.
    pub fn block_tcp(&mut self) -> &mut Self {
        self.block_tcp = true;
        self
    }

    /// Block all outbound UDP not in the allow list.
    pub fn block_udp(&mut self) -> &mut Self {
        self.block_udp = true;
        self
    }

    /// Builds the [`FirewallConfig`].
    pub fn build(&self) -> FirewallConfig {
        FirewallConfig {
            allow_tcp: self.allow_tcp.clone(),
            allow_udp: self.allow_udp.clone(),
            block_tcp: self.block_tcp,
            block_udp: self.block_udp,
        }
    }
}
