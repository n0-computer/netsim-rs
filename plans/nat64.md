# NAT64 Implementation Plan

## Context

Real-world IPv6-only mobile networks (T-Mobile US/DE, NTT Docomo) use NAT64
to provide IPv4 access. patchbay's `IpSupport::V6Only` mode currently provides
no IPv4 reachability at all — devices simply have no v4 routes. Adding NAT64
would let us simulate the dominant mobile IPv6 deployment model.

## Architecture: userspace SIIT translator (zero external deps)

Inspired by [danderson's siit0 proposal](https://gist.github.com/danderson/664bf95f372acf106982bcc29ff56b53):
a stateless IP/ICMP translation (SIIT) device that converts between IPv6 and
IPv4 packet headers, combined with standard nftables NAT for address mapping.

### Why not external tools?

- **jool** — out-of-tree kernel module, requires DKMS, hard to maintain
- **tayga** — userspace TUN translator, works but adds an external binary dep
- **kernel nftables** — can rewrite addresses but cannot change IP protocol
  version (IPv6 header ↔ IPv4 header)

### Our approach: built-in Rust SIIT translator

Build a minimal SIIT translator as an async task running on the namespace
worker's LocalSet. Uses a TUN device (no new crate deps — just `nix` ioctls
for TUN creation + `tokio::io` for async read/write).

**Packet flow (IPv6 client → IPv4 server):**

```
Device (fd10::2)
    │ dst = 64:ff9b::203.0.113.1 (well-known NAT64 prefix + embedded IPv4)
    ▼
NAT64 Router namespace
    │ route: 64:ff9b::/96 dev tun-nat64
    ▼
TUN device (tun-nat64)
    │ SIIT translator reads IPv6 packet
    │ strips IPv6 header, creates IPv4 header
    │ src = 192.0.2.X (NAT64 pool), dst = 203.0.113.1
    │ writes IPv4 packet back to tun-nat64
    ▼
IPv4 routing table
    │ default via IX gateway
    ▼
IX → destination
```

**Return path (IPv4 reply → IPv6 client):**

```
IX → NAT64 Router
    │ route: 192.0.2.0/24 dev tun-nat64
    ▼
TUN device (tun-nat64)
    │ SIIT reads IPv4 packet
    │ strips IPv4 header, creates IPv6 header
    │ src = 64:ff9b::203.0.113.1, dst = fd10::2
    │ writes IPv6 packet back to tun-nat64
    ▼
IPv6 routing table
    │ fd10::/64 dev br-lan
    ▼
Device
```

### Translation rules (RFC 6145 SIIT)

The translator handles:
- **IPv6 → IPv4**: extract embedded IPv4 from last 32 bits of dst (well-known
  prefix `64:ff9b::/96`), map src from configured pool
- **IPv4 → IPv6**: embed src IPv4 into `64:ff9b::` prefix, map dst back to
  original IPv6 src (via conntrack or stateless 1:1 mapping)
- **ICMPv6 ↔ ICMPv4**: translate type/code values per RFC 6145 §4
- **TCP/UDP**: headers identical, recalculate checksums (IPv6 pseudo-header
  differs from IPv4)

### Stateful vs stateless

For simplicity, use **stateful** NAT64 (like real carrier deployments):
- nftables `masquerade` on the IPv4 side handles port mapping
- The SIIT translator only does header translation
- conntrack handles the return path mapping

### Implementation (~300-400 lines)

**New file: `patchbay/src/nat64.rs`**

```rust
/// Minimal SIIT (Stateless IP/ICMP Translation) for NAT64.
///
/// Creates a TUN device and translates IPv6 ↔ IPv4 headers.
/// Combined with nftables masquerade on the v4 side, this
/// implements stateful NAT64 (RFC 6146).

pub(crate) struct Nat64Translator {
    tun_fd: AsyncFd<OwnedFd>,  // or tokio File on /dev/net/tun
    nat64_prefix: Ipv6Net,     // 64:ff9b::/96
    v4_pool: Ipv4Addr,         // address used on the v4 side
}

impl Nat64Translator {
    pub fn new(tun_name: &str, prefix: Ipv6Net, pool: Ipv4Addr) -> Result<Self>;
    pub async fn run(&self) -> Result<()>;  // main translation loop
    fn translate_v6_to_v4(&self, pkt: &[u8]) -> Option<Vec<u8>>;
    fn translate_v4_to_v6(&self, pkt: &[u8]) -> Option<Vec<u8>>;
}
```

**TUN creation** (using `nix` which we already depend on):
```rust
use nix::sys::stat::Mode;
use nix::fcntl::{open, OFlag};
// open /dev/net/tun, ioctl TUNSETIFF with IFF_TUN | IFF_NO_PI
```

### API

```rust
// New NatV6Mode variant:
pub enum NatV6Mode {
    None,
    Nptv6,
    Masquerade,
    Nat64,  // NEW: IPv6-only with NAT64 for IPv4 access
}

// Usage:
let carrier = lab.add_router("carrier")
    .ip_support(IpSupport::V6Only)
    .nat_v6(NatV6Mode::Nat64)
    .build().await?;
// Devices behind this router:
// - Have IPv6 addresses (ULA or GUA)
// - Can reach IPv4 hosts via 64:ff9b::<ipv4> prefix
// - Cannot be reached from IPv4 (outbound only)
```

### Router setup changes

In `setup_router_async`, when `NatV6Mode::Nat64`:

1. Create TUN `tun-nat64` in the router namespace
2. Assign the NAT64 v4 pool address to the TUN
3. Add route `64:ff9b::/96 dev tun-nat64`
4. Add route `<pool>/32 dev tun-nat64` for return traffic
5. Add nftables masquerade on the IX interface for v4
6. Spawn the SIIT translator task on the namespace worker

### DNS64

For complete NAT64, DNS64 is needed (synthesizes AAAA records from A records).
This can be deferred — applications in the simulator can use `64:ff9b::<ipv4>`
addresses directly. If needed later, a simple DNS64 proxy (~100 lines) can be
added to the namespace's DNS overlay.

## Dependencies

None new. Uses:
- `nix` (already dep) — TUN device creation via ioctl
- `tokio` (already dep) — async I/O on TUN fd
- Standard library — IPv4/IPv6 header parsing and construction

## Testing

```rust
#[tokio::test]
async fn nat64_v6_to_v4() -> Result<()> {
    let lab = Lab::new().await?;
    let dc = lab.add_router("dc").build().await?;  // v4-only server
    let server = lab.add_device("server").uplink(dc.id()).build().await?;
    server.spawn_reflector((server.ip().unwrap(), 3000).into())?;

    let carrier = lab.add_router("carrier")
        .ip_support(IpSupport::V6Only)
        .nat_v6(NatV6Mode::Nat64)
        .build().await?;
    let phone = lab.add_device("phone").uplink(carrier.id()).build().await?;

    // Phone reaches v4 server via NAT64 prefix
    let v4_addr = server.ip().unwrap();
    let nat64_addr: Ipv6Addr = /* embed v4_addr in 64:ff9b:: */;
    let target = SocketAddr::new(IpAddr::V6(nat64_addr), 3000);
    let observed = phone.probe_udp_mapping(target)?;
    assert!(observed.is_ipv4());  // server sees IPv4 source
    Ok(())
}
```

## Estimated effort

- TUN creation + async wrapper: ~50 lines
- SIIT translator (v6↔v4 headers, ICMP, checksum): ~250 lines
- Router setup integration: ~50 lines
- Tests: ~100 lines
- Total: ~450 lines, no new deps
