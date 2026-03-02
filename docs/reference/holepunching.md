# NAT Hole-Punching

> This is an advanced reference for readers who want to understand how
> patchbay implements NAT traversal at the nftables level. You do not need
> to read this to use patchbay; the [NAT and Firewalls](../guide/nat-and-firewalls.md)
> guide covers the user-facing API.

This document describes how patchbay implements NAT mapping and filtering
using nftables, and what we learned getting UDP hole-punching to work
across different NAT types in Linux network namespaces.

## RFC 4787: mapping and filtering

Two independent axes define NAT behavior for UDP. **Mapping** controls how
external ports are assigned: endpoint-independent mapping (EIM) reuses the
same external port for all destinations, while endpoint-dependent mapping
(EDM) assigns a different port per destination. **Filtering** controls
which inbound packets are forwarded to a mapped port:
endpoint-independent filtering (EIF) accepts packets from any external
host, while endpoint-dependent filtering only forwards replies from hosts
the internal client has already contacted.

Combined, these axes produce the real-world NAT profiles that patchbay
simulates:

| Preset | Mapping | Filtering | Hole-punch? | Real-world examples |
|--------|---------|-----------|-------------|---------------------|
| `Nat::Home` | EIM | APDF | Yes, simultaneous open | FritzBox, Unifi, TP-Link, ASUS RT, OpenWRT |
| `Nat::FullCone` | EIM | EIF | Always | Old FritzBox firmware, some CGNAT |
| `Nat::Corporate` | EDM | APDF | Never (need relay) | Cisco ASA, Palo Alto, Fortinet, Juniper SRX |
| `Nat::CloudNat` | EDM | APDF | Never (need relay) | AWS/Azure/GCP NAT Gateway |
| `Nat::Cgnat` | -- | -- | Varies | ISP-level, stacks with home NAT |

## The fullcone dynamic map

The only reliable way to get endpoint-independent mapping in nftables is to
explicitly track port mappings in a dynamic map. The kernel's built-in
`snat` and `masquerade` statements do not preserve ports across independent
conntrack entries, even when there is no port conflict (see the pitfalls
section below). patchbay works around this with an `@fullcone` map:

```nft
table ip nat {
    map fullcone {
        type inet_service : ipv4_addr . inet_service
        flags dynamic,timeout
        timeout 300s
        size 65536
    }
    chain prerouting {
        type nat hook prerouting priority dstnat; policy accept;
        iif "ix" meta l4proto udp dnat to udp dport map @fullcone
    }
    chain postrouting {
        type nat hook postrouting priority srcnat; policy accept;
        oif "ix" meta l4proto udp update @fullcone {
            udp sport timeout 300s : ip saddr . udp sport
        }
        oif "ix" snat to <wan_ip>
    }
}
```

The postrouting chain records the pre-SNAT source address and port in the
map before the `snat` rule executes. The map key is the UDP source port
and the value is `internal_ip . internal_port`. Even if `snat` later
remaps the port, the map holds the correct mapping keyed by the original
port. On the inbound side, the prerouting chain looks up incoming UDP
packets by destination port in the map and DNATs them to the internal
host, bypassing conntrack reverse-NAT entirely.

The `update` statement must come before `snat` in the postrouting chain.
nftables NAT statements record the transformation, but the conntrack
entry's reply tuple is not yet available during the same chain evaluation.
By recording `udp sport` and `ip saddr` before SNAT, we capture the
original tuple. Map entries time out after 300 seconds and are refreshed
by outbound traffic.

## Filtering modes

### Endpoint-independent filtering (fullcone)

`Nat::FullCone` uses the fullcone map above with no additional filtering.
The prerouting DNAT fires for any inbound packet whose destination port
appears in the map, regardless of source address. Once an internal device
sends one outbound packet, any external host can reach it on the mapped
port.

### Address-and-port-dependent filtering (home NAT)

`Nat::Home` uses the same fullcone map for endpoint-independent mapping,
plus a forward filter that restricts inbound traffic to established
connections:

```nft
table ip filter {
    chain forward {
        type filter hook forward priority 0; policy accept;
        iif "ix" ct state established,related accept
        iif "ix" drop
    }
}
```

This combination is what makes hole-punching work with home NATs. The
sequence is:

1. The internal device sends a UDP packet to the peer. Postrouting SNAT
   creates a conntrack entry and the fullcone map records the port mapping.
2. The peer sends a packet to the device's mapped address. Prerouting DNAT
   via the fullcone map rewrites the destination from the router's WAN IP
   to the device's internal IP.
3. After DNAT, the packet's 5-tuple matches the reply direction of the
   outbound conntrack entry from step 1. Conntrack marks it as
   `ct state established`.
4. The forward filter allows the packet through.

An unsolicited packet from an unknown host also gets DNATed in step 2, but
no matching outbound conntrack entry exists, so the packet arrives with
`ct state new` and the filter drops it.

### Endpoint-dependent mapping (corporate and cloud NAT)

`Nat::Corporate` and `Nat::CloudNat` use plain `masquerade random`
without a fullcone map:

```nft
table ip nat {
    chain postrouting {
        type nat hook postrouting priority 100;
        oif "ix" masquerade random
    }
}
```

The `random` flag randomizes the source port for each conntrack entry.
Without a fullcone map and without a prerouting chain, hole-punching is
impossible because the peer cannot predict the mapped port from a STUN
probe.

## nftables pitfalls

### Port preservation is unreliable

The single biggest surprise during implementation. Conventional wisdom
says `snat to <ip>` without a port range is "port-preserving". In
practice, Linux conntrack assigns different external ports for different
conntrack entries from the same source socket, even when there is no port
conflict.

For example: a device binds port 40000, sends to a STUN server (port
preserved to 40000), then sends to a peer. Conntrack assigns port 27028
instead of 40000, despite the absence of any conflict on that port.

None of the following fix this:

```nft
oif "ix" snat to 203.0.113.11              # port NOT preserved across entries
oif "ix" snat to 203.0.113.11 persistent   # still remaps
oif "ix" masquerade persistent              # still remaps
```

The `persistent` flag is documented to "give a client the same
source-ip,source-port", but the kernel's NAT tuple uniqueness check still
triggers port reallocation across independent conntrack entries. This is
why the fullcone dynamic map is necessary for endpoint-independent mapping.

### A prerouting nat chain is required even if empty

Without a `type nat hook prerouting` chain registered in the nat table,
the kernel does not perform conntrack reverse-NAT lookup on inbound
packets. Packets destined for the router's WAN IP that should be
reverse-DNATed are delivered to the router's INPUT chain instead of being
forwarded to the internal device.

### Conntrack reverse-NAT depends on port consistency

Even with a prerouting chain, conntrack reverse-NAT only works when the
inbound packet's 5-tuple matches the reply tuple of an existing conntrack
entry. If SNAT changed the port (which it does, as described above), the
peer sends to the wrong port and conntrack cannot match the entry.

## Test helper subtlety

Both sides of a hole-punch test call `holepunch_send_recv`, which sends
UDP probes every 200ms and checks for a response. There is a critical
ordering issue: when one side receives a probe first, it must send a few
more packets before returning. Otherwise, side A receives side B's probe,
returns success, and stops sending. But side B's early probes may have
arrived before side A created its outbound conntrack entry at side B's
NAT, so those probes were dropped by APDF filtering. With side A no
longer sending, side B never receives a packet.

The fix is to send three additional "ack" packets after receiving, to
ensure the peer's NAT has an established conntrack entry in both
directions.

## NatConfig architecture

The `Nat` enum provides named presets. Each preset expands via
`Nat::to_config()` to a `NatConfig` struct that drives rule generation:

```rust
pub struct NatConfig {
    pub mapping: NatMapping,           // EIM or EDM
    pub filtering: NatFiltering,       // EIF or APDF
    pub timeouts: ConntrackTimeouts,   // udp, udp_stream, tcp_established
}
```

The `generate_nat_rules()` function in `core.rs` builds nftables rules
from `NatConfig` alone, without matching on `Nat` variants. This means
users can either use the named presets (`router.nat(Nat::Home)`) or build
custom configurations with arbitrary mapping and filtering combinations.

CGNAT is a special case: `Nat::Cgnat` is applied at the ISP router level
via `apply_isp_cgnat()` rather than through `NatConfig`. It uses plain
`masquerade` (without the `random` flag) on the IX-facing interface and
stacks with the downstream home router's NAT.

## NPTv6 implementation notes

NPTv6 (Network Prefix Translation for IPv6) translates source and
destination prefixes while preserving the host part, using nftables
`snat prefix to` and `dnat prefix to`. Several issues were found during
implementation:

1. **Prefix length mismatch breaks translation.** NPTv6 requires matching
   prefix lengths on LAN and WAN sides. The `nptv6_wan_prefix()` function
   derives a unique /64 from the router's IX address.

2. **Unrestricted `dnat prefix` breaks NDP.** Without an address match
   clause, NDP and ICMPv6 packets get translated, making the router
   unreachable. The rules are restricted to `ip6 saddr/daddr` matching the
   WAN or LAN prefix.

3. **WAN prefix must be outside the IX on-link range.** The IX CIDR was
   changed from /32 to /64 so WAN prefixes are off-link and routed via the
   gateway.

4. **Return routes needed for private v6 downstreams.** IPv6 return routes
   are added for all IX-level routers regardless of downstream pool
   configuration.

See [IPv6 Deployments](ipv6.md) for the full IPv6 deployment reference.

## Limitations

The fullcone map tracks UDP only. TCP hole-punching (simultaneous SYN)
relies on plain conntrack, which matches real-world behavior where TCP
hole-punching is unreliable.

There is also a port preservation assumption in the map: if `snat to <ip>`
remaps the source port, the fullcone map key (the original port) differs
from the actual mapped port. In practice this does not happen in patchbay
simulations because there are few concurrent flows relative to the 64k
port space.

## Future work

- **Address-restricted cone** (EIM + address-dependent filtering): extend
  the fullcone map to track contacted remote IPs.
- **Hairpin NAT**: add a prerouting rule for LAN packets addressed to the
  router's own WAN IP.
- **TCP fullcone**: extend `@fullcone` to TCP for a complete NAT model.
- **Port-conflict-safe fullcone**: two-stage postrouting to read
  `ct reply proto-dst` after conntrack finalizes the mapping.
