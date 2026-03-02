# Real-World IPv6 Deployments

How IPv6 works in practice and how to simulate each scenario in patchbay.

---

## How ISPs Actually Deploy IPv6

### Residential (FTTH, Cable, DSL)

ISPs assign a **globally routable prefix** (typically /56 or /60) via
DHCPv6-PD (Prefix Delegation). The CE (Customer Edge) router carves /64s
from this prefix for each LAN segment. Devices get **public GUA addresses**
— no NAT involved. The security boundary is a **stateful firewall** on the
CE router that blocks unsolicited inbound connections (RFC 6092).

IPv4 access is provided in parallel via dual-stack (separate IPv4 address
with NAT44) or via DS-Lite / MAP-E / MAP-T (IPv4-in-IPv6 tunneling to the
ISP's AFTR).

**Key properties:**
- Devices have globally routable IPv6 addresses
- No IPv6 NAT — the prefix is public
- Stateful firewall blocks inbound, allows outbound + established
- SLAAC for address assignment (not DHCPv6 address assignment)
- Privacy extensions (RFC 4941) rotate source addresses

**Carriers:** Deutsche Telekom, Comcast, AT&T, Orange, BT, NTT.

### Mobile (4G/5G)

Each device typically gets a **single /64** via RA (Router Advertisement).
The device is the only host on its /64. There is no home router — the
carrier's gateway acts as the first hop.

For IPv4 access, carriers use either:
- **464XLAT** (RFC 6877): CLAT on device + NAT64 on carrier gateway
- **NAT64 + DNS64**: carrier synthesizes AAAA records from A records

Some carriers (T-Mobile US, Jio) are IPv6-only with NAT64. Others
(Verizon, NTT Docomo) do dual-stack.

**Key properties:**
- One /64 per device (not shared)
- NAT64/DNS64 for IPv4 access (no real IPv4 address)
- No firewall — carrier relies on per-device /64 isolation
- 3GPP CGNAT for remaining IPv4 users

### Enterprise / Corporate

Enterprises typically run dual-stack internally with PA (Provider
Aggregatable) or PI (Provider Independent) space. Strict firewalls allow
only TCP 80/443 and UDP 53 outbound. All other ports are blocked —
STUN/TURN on non-standard ports fails, must use TURN-over-TLS on 443.

Some enterprises use ULA (fd00::/8) internally with NAT66 at the border,
though this is discouraged by RFC 4864 and IETF best practices.

### Hotel / Airport / Guest WiFi

After captive portal authentication, these networks typically allow:
- TCP 80, 443 (HTTP/HTTPS)
- TCP/UDP 53 (DNS)
- All other UDP blocked (kills QUIC, STUN, direct P2P)
- TCP to other ports sometimes allowed (unlike corporate)

Many guest networks are still IPv4-only. Those with IPv6 usually provide
GUA addresses with a restrictive firewall.

---

## ULA + NAT66: Mostly a Myth

RFC 4193 ULA (fd00::/8) was designed for stable internal addressing, not
as an IPv6 equivalent of RFC 1918. In practice:

- **No major ISP deploys NAT66** — it defeats the end-to-end principle
- Android **does not support NAT66** (no DHCPv6 client, only SLAAC)
- ULA is used alongside GUA for stable internal addressing, never alone
- RFC 6296 NPTv6 (prefix translation) exists but is niche — mostly
  for multihoming, not general NAT

If you need to simulate "NATted IPv6", use NPTv6 (`NatV6Mode::Nptv6`)
which does stateless 1:1 prefix translation. But understand this is rare
in the real world.

---

## Simulating Real-World Scenarios in Patchbay

### Scenario 1: Residential Dual-Stack (Most Common)

A home router with public IPv6 and NATted IPv4. The CE router firewall
blocks unsolicited inbound on both families.

```rust
let home = lab.add_router("home")
    .nat(Nat::PortRestricted)          // IPv4: NAT44 (EIM/ADF)
    .downstream_pool(DownstreamPool::Public)  // IPv6: public GUA /64
    .firewall(Firewall::BlockInbound)  // RFC 6092 CE router behavior
    .build().await?;
let laptop = lab.add_device("laptop").uplink(home.id()).build().await?;
// laptop.ip()  → 198.18.x.x (public IPv4, NATted)
// laptop.ip6() → 2001:db8:1:x::2 (public GUA, firewalled)
```

IPv4 behaves like today's internet (NAT + port mapping). IPv6 has direct
connectivity but the firewall drops unsolicited inbound — exactly like a
FritzBox or UniFi router.

### Scenario 2: IPv6-Only Mobile with NAT64

A carrier network where devices only have IPv6. IPv4 destinations are
reached via NAT64 (translating IPv6 packets to IPv4).

> **Note:** NAT64 is not yet implemented. See `plans/nat64.md` for the
> implementation plan. Until then, use `IpSupport::V6Only` which provides
> IPv6-only connectivity without IPv4 access.

```rust
let carrier = lab.add_router("carrier")
    .ip_support(IpSupport::V6Only)
    .downstream_pool(DownstreamPool::Public)
    // .nat_v6(NatV6Mode::Nat64)  // TODO: not yet implemented
    .build().await?;
let phone = lab.add_device("phone").uplink(carrier.id()).build().await?;
// phone.ip6() → 2001:db8:1:x::2 (public GUA)
// phone.ip()  → None (no IPv4)
```

### Scenario 3: Corporate Firewall (Restrictive)

Enterprise network that blocks everything except web traffic. STUN/ICE
fails — P2P apps must fall back to TURN-over-TLS on port 443.

```rust
let corp = lab.add_router("corp")
    .nat(Nat::PortRestricted)
    .firewall(Firewall::Corporate)  // TCP 80,443 + UDP 53 only
    .downstream_pool(DownstreamPool::Public)  // GUA v6
    .build().await?;
let workstation = lab.add_device("ws").uplink(corp.id()).build().await?;
```

### Scenario 4: Hotel / Captive Portal

Guest WiFi that allows web traffic but blocks most UDP.

```rust
let hotel = lab.add_router("hotel")
    .nat(Nat::Symmetric)               // Aggressive NAT
    .firewall(Firewall::CaptivePortal) // TCP 80,443,53 + UDP 53
    .build().await?;
let guest = lab.add_device("guest").uplink(hotel.id()).build().await?;
```

### Scenario 5: CGNAT (Carrier-Grade NAT)

Multiple subscribers sharing a single public IPv4 address. Common on
mobile and some fixed-line ISPs.

```rust
let isp = lab.add_router("isp")
    .nat(Nat::Cgnat)
    .downstream_pool(DownstreamPool::Public)  // public GUA v6
    .firewall(Firewall::BlockInbound)
    .build().await?;
let sub = lab.add_device("sub").uplink(isp.id()).build().await?;
```

### Scenario 6: Peer-to-Peer Connectivity Test Matrix

Test how two peers connect across different network types:

```rust
// Home user: easy NAT, public v6
let home = lab.add_router("home")
    .nat(Nat::FullCone)
    .downstream_pool(DownstreamPool::Public)
    .firewall(Firewall::BlockInbound)
    .build().await?;
let alice = lab.add_device("alice").uplink(home.id()).build().await?;

// Mobile user: symmetric NAT, restricted
let mobile = lab.add_router("mobile")
    .nat(Nat::Symmetric)
    .firewall(Firewall::BlockInbound)
    .build().await?;
let bob = lab.add_device("bob").uplink(mobile.id()).build().await?;

// Corporate user: strict firewall
let corp = lab.add_router("corp")
    .nat(Nat::PortRestricted)
    .firewall(Firewall::Corporate)
    .build().await?;
let charlie = lab.add_device("charlie").uplink(corp.id()).build().await?;

// Test: can alice reach bob? bob reach charlie? etc.
```

---

## IPv6 Feature Reference

| Feature | API | Notes |
|---------|-----|-------|
| Dual-stack (default) | `IpSupport::DualStack` | Both v4 and v6 |
| IPv6-only | `IpSupport::V6Only` | No v4 routes |
| IPv4-only | `IpSupport::V4Only` | No v6 routes |
| Private v6 (ULA) | `DownstreamPool::Private` | fd10::/48 pool (default) |
| Public v6 (GUA) | `DownstreamPool::Public` | 2001:db8:1::/48 pool |
| NPTv6 | `NatV6Mode::Nptv6` | Stateless 1:1 prefix translation |
| NAT66 (masquerade) | `NatV6Mode::Masquerade` | Like NAT44 but for v6 |
| Block inbound | `Firewall::BlockInbound` | RFC 6092 CE router |
| Corporate FW | `Firewall::Corporate` | TCP 80,443 + UDP 53 |
| Captive portal FW | `Firewall::CaptivePortal` | TCP 80,443,53 + UDP 53 |
| NAT64 | *planned* | See `plans/nat64.md` |
| DHCPv6-PD | *not planned* | Use static /64 allocation |

---

## Common Pitfalls

### NPTv6 and NDP

NPTv6 `dnat prefix to` rules must include address match clauses (e.g.,
`ip6 daddr <wan_prefix>`) to avoid translating NDP packets. Without this,
neighbor discovery breaks and the router becomes unreachable.

### IPv6 Firewall Is Not Optional

On IPv4, NAT implicitly blocks inbound connections (no port mapping = no
access). On IPv6 with public GUA addresses, there is **no NAT** — devices
are directly addressable. Without `Firewall::BlockInbound`, any host on
the IX can connect to your devices. This matches reality: every CE router
ships with an IPv6 stateful firewall enabled by default.

### On-Link Prefix Confusion

When IX-level routers share a /64 IX prefix, their WAN addresses are
on-link with each other. Routing prefixes carved from the IX range can
cause "on-link" confusion where packets are sent directly (ARP/NDP) rather
than via the gateway. Use distinct prefixes for IX (/64) and downstream
pools (/48 from a different range).
