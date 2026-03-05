# Introduction

patchbay builds realistic network topologies out of Linux network namespaces
and lets you run real code against them. You describe routers, devices, NAT
policies, firewalls, and link conditions through a Rust builder API. The
library creates a namespace per node, wires them with veth pairs, installs
nftables rules for NAT and firewalling, and applies tc netem shaping for
loss, latency, jitter, and rate limits. Each device gets its own kernel
network stack, so code running inside a namespace sees exactly what it would
see on a separate machine. Everything runs unprivileged and cleans up when
the `Lab` is dropped.

## How this book is organized

The **Guide** section walks through patchbay's concepts in the order you
are likely to need them. It starts with the motivation behind the project
and progresses through setting up a lab, building topologies, configuring
NAT and firewalls, running code inside namespaces, and running labs in a
QEMU VM on non-Linux hosts. Each chapter builds on the previous one and
includes runnable examples.

The **Reference** section covers specialized topics in depth. It documents
real-world IPv6 deployment patterns and how to simulate them, recipes for
common network scenarios like WiFi handoff and VPN tunnels, the internals
of NAT traversal and hole-punching as implemented in nftables, and the
TOML simulation file format used by the patchbay runner.

The **Limitations** page documents known boundaries of the current model.
Read it before relying on packet-level control-plane behavior,
OS-specific network-stack quirks, or low-level timing fidelity.

A built-in devtools server (`patchbay serve`) provides an interactive web
UI for inspecting lab runs: topology graphs, event timelines,
per-namespace structured logs, and performance results. Set
`PATCHBAY_OUTDIR` when running tests or simulations to capture output,
then serve it in the browser.
