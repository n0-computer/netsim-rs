# Motivation and Scope

## The problem

Networking code is notoriously hard to test. Unit tests can verify
serialization and state machines, but they cannot tell you whether your
connection logic survives a home NAT, whether your hole-punching strategy
works through carrier-grade NAT, or whether your reconnect path handles a
WiFi-to-cellular handoff without dropping state. Those questions require
actual network stacks with actual packet processing, and the only way most
teams answer them today is by deploying to staging and hoping for the best.

Tools like Docker Compose, Mininet, and custom iptables scripts can help,
but each comes with trade-offs around privilege requirements, cleanup
reliability, and how easily you can parameterize topologies from a test
harness. patchbay was built to make this kind of testing ergonomic for Rust
projects: no root, no cleanup, and a builder API that fits naturally into
`#[tokio::test]` functions.

## What patchbay does

patchbay builds realistic network topologies out of Linux network
namespaces and lets you run real code against them. You describe routers,
devices, NAT policies, firewalls, and link conditions through a Rust
builder API. The library creates a namespace per node, wires them together
with veth pairs, installs nftables rules for NAT and firewalling, and
applies tc netem/tbf shaping for loss, latency, jitter, and rate limits.
Each device gets its own kernel network stack, so code running inside a
namespace sees exactly what it would see on a separate machine.

Everything runs unprivileged. The library enters an unprivileged user
namespace at startup, so no root access is needed at any point. When the
`Lab` value is dropped, all namespaces, interfaces, and rules disappear
automatically.

## Where it fits

patchbay is a testing and development tool, designed for three primary
use cases:

**Integration tests.** Write `#[tokio::test]` functions that build a
topology, run your networking code inside it, and assert on outcomes. Each
test gets an isolated lab with its own address space, so tests can run in
parallel without interfering with each other or with the host.

**Performance and regression testing.** Apply link conditions to simulate
constrained networks (3G, satellite, lossy WiFi) and measure throughput,
latency, or reconnection time under controlled impairment. Because tc
netem operates at the kernel level, the shaping is realistic enough for
comparative benchmarks, though absolute numbers will differ from hardware
links due to scheduling overhead and the absence of real radio or cable
physics.

**Interactive experimentation.** Build a topology in a binary or script,
attach to device namespaces with shell commands, and observe how traffic
flows. This is useful for understanding NAT behavior, debugging
connectivity issues, or validating protocol assumptions before writing
tests.

patchbay operates at the kernel namespace level with real TCP/IP stacks,
not at the packet simulation level. This means the fidelity is high (you
are testing against real Linux networking), but the scale is limited to
what a single machine can support (typically dozens of namespaces, not
thousands).
