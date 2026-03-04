# Limitations

This page documents known limitations of patchbay. The goal is to help you
decide when patchbay is a good fit, and where you should expect differences
from production systems.

## IPv6 limitations

### RA and RS are modeled, not packet-emulated

In `Ipv6ProvisioningMode::RaDriven`, patchbay models Router Advertisement
(RA) and Router Solicitation (RS) behavior through route updates and
structured tracing events. It does not currently send raw ICMPv6 RA or RS
packets on virtual links.

Impact:

- Application-level routing behavior is usually close to production.
- Packet-capture workflows that expect real RA/RS frames are not covered yet.

### SLAAC behavior is partial

Patchbay models default-route and address behavior needed for routing tests,
but it does not currently implement a full Stateless Address
Autoconfiguration (SLAAC) state machine with all timing transitions.

Impact:

- Connectivity and route-selection tests work well.
- Detailed host autoconfiguration timing studies are out of scope.

### Neighbor Discovery timing is not fully emulated

Neighbor Discovery (ND) address and router behavior is represented in route
and interface state, but exact kernel-level timing of ND probes, retries, and
expiration is not fully emulated.

Impact:

- Most application tests are unaffected.
- Low-level protocol timing analysis should use a dedicated packet-level setup.

### DHCPv6 prefix delegation is not implemented

Patchbay does not implement a DHCPv6 Prefix Delegation server or client flow.
Use static /64 allocation in topologies.

Impact:

- Residential-prefix churn workflows are not fully represented.
- Prefix-based routing and NAT64 scenarios still work with static setup.

## General platform and model limitations

### Linux-only execution model

Patchbay uses Linux network namespaces, nftables, and tc. It models network
behavior through the Linux kernel network stack.

Impact:

- Native execution requires Linux.
- macOS and Windows host stacks are not emulated byte-for-byte.

### Requires kernel features and host tooling

Patchbay depends on user namespaces and network tools such as `nft` and `tc`.
If those capabilities are unavailable or restricted, labs cannot run.

Impact:

- Some CI and container environments need extra setup.
- Missing kernel features can block specific scenarios.

### No wireless or cellular radio-layer simulation

Patchbay models link effects with `tc` parameters such as latency, jitter,
loss, and rate limits. It does not model WiFi or cellular PHY/MAC behavior.

Impact:

- Good for transport and application resilience testing.
- Not suitable for radio scheduling or handover signaling research.

### Dynamic routing protocols are not built in

Patchbay focuses on static topology wiring, NAT, firewalling, and route
management through its API. It does not include built-in BGP, OSPF, or RIP
control-plane implementations.

Impact:

- You can still run routing daemons inside namespaces yourself.
- Protocol orchestration is user-managed, not first-class in patchbay.

### Time and clock behavior are not virtualized

Patchbay uses the host kernel clock and scheduler behavior. It does not
virtualize per-node clocks or provide deterministic virtual time.

Impact:

- Most integration tests work as expected.
- Time-sensitive distributed-system tests may need additional controls.
