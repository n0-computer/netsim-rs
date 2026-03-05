# Plan: Virtual Time / Time Acceleration

Status: **not planned** — research complete, no realistic path to faster-than-real-time with the current architecture.

## Problem

Tests run in wall-clock time. A NAT mapping timeout test (5 min) takes 5+ min.
ICE restart timers, DTLS retransmit backoff, keepalive intervals — all require
waiting real seconds/minutes. This limits how many timeout-sensitive scenarios
can be covered in a test suite that needs to finish in reasonable time.

## Core constraint

patchbay uses the real Linux kernel network stack: namespaces, veth pairs,
nftables NAT, and `tc netem` qdiscs. Every kernel-internal timer — TCP
retransmissions, ARP probes, netem delay queues, conntrack timeouts — ticks
against `CLOCK_MONOTONIC` / jiffies. No userspace mechanism can make those
timers run faster.

**Every system that achieves virtual time does so by replacing the kernel
network stack, not by accelerating it.**

## Approaches evaluated

### 1. Shadow-style syscall interception (LD_PRELOAD + seccomp)

Shadow runs real binaries but intercepts every syscall. It re-implements TCP,
UDP, routing, and queueing in a userspace "simulated kernel" driven by a
discrete-event engine with fully virtual time. Deterministic, seeds all
randomness from a PRNG.

**Verdict**: The only way to get true faster-than-real-time. But Shadow achieves
this by not using the kernel network stack at all. Its TCP is a simplified
model, not Linux's. Implementing this would be an enormous effort and would
abandon kernel fidelity — patchbay's core value. This would be a different
project.

### 2. Linux time namespaces (kernel 5.6+)

`CLONE_NEWTIME` allows per-namespace static offsets to `CLOCK_MONOTONIC` and
`CLOCK_BOOTTIME`. Designed for checkpoint/restore (CRIU).

**Verdict**: Not useful. These are fixed offsets, not dilation factors. The
clock still ticks at wall-clock rate.

### 3. tokio::time::pause()

Pauses tokio's internal clock and auto-advances when all tasks block on timers.
Only affects `tokio::time::Instant`, not `std::time`, not kernel timers.
Requires `current_thread` runtime.

**Verdict**: Zero effect on kernel networking, `tc netem` delays, TCP
retransmissions, or ARP resolution. Could test orchestration logic in isolation
but not end-to-end simulation.

### 4. libfaketime (LD_PRELOAD)

Intercepts `clock_gettime()` and related libc calls. Can apply a scaling factor.
Does not affect kernel-internal timers, vDSO calls, or the `tc` qdisc layer.

**Verdict**: Kernel network stack ignores it. A netem delay of 100 ms still
costs 100 ms wall-clock.

### 5. TimeKeeper kernel module (research, U of Illinois)

True per-container time dilation at the kernel level. A TDF of 10 makes the
container see 1 second per 10 wall-clock seconds — the network *appears* 10×
faster, but the test takes 10× *longer*.

**Verdict**: Requires a custom kernel. Only makes simulations slower in
wall-clock terms, not faster. Useful for emulating high-bandwidth on slow
hardware, not for speeding up a test suite. Not practical for stock Linux.

### 6. Turmoil / MadSim (Rust, pure simulation)

Simulate networking in-process with virtual time and deterministic execution.
No namespaces, no veth, no kernel TCP.

**Verdict**: Fundamentally different architecture. Would require building an
alternative simulation backend behind the patchbay API.

### 7. Conntrack timeout override + application test mode

Shorten `nf_conntrack_udp_timeout` to 2-5s via sysctl, configure the
application with matching short keepalive intervals.

**Verdict**: Practical today, already partially supported via `ConntrackTimeouts`
in the NAT config. Doesn't virtualize time but reduces real-time exposure for
timeout-sensitive tests.

### 8. eBPF timer compression

Use `BPF_SOCK_OPS_RTO_CB` to shrink TCP retransmission timers per-namespace.
Requires `CAP_BPF`/`CAP_SYS_ADMIN`. Only affects TCP RTO, not netem or
conntrack.

**Verdict**: Worth noting but fragile, kernel-version-dependent, and not a
general solution.

## What the kernel binds to real time

| Component | Controllable from userspace? |
|---|---|
| `tc netem` delay / jitter | No — kernel qdisc timer |
| `tc` TBF rate limiting | No — kernel token bucket |
| TCP RTO | Partially — initial via BPF (4.13+), min via `ip route` |
| TCP congestion window timing | No |
| ARP / ND probes | Tunable intervals via sysctl, still wall-clock |
| nftables conntrack timeouts | Tunable via sysctl, still wall-clock |
| SLAAC / RA timers | Kernel-managed, wall-clock |

## Conclusion

Faster-than-real-time with the real kernel stack is not achievable. Every
approach that works requires replacing the kernel stack, which defeats patchbay's
purpose.

The pragmatic mitigations already in place or easily added are:

- **ARP/ND pre-warming** (done — `ddc5f79`)
- **Short conntrack timeouts** (supported via `ConntrackTimeouts`)
- **Minimal netem delays** for tests that only care about ordering
- **Test parallelism** across independent lab instances
- **A `time_scale` multiplier** on `LabOpts` that scales netem delay/jitter
  values before passing them to `tc` (not virtual time — just smaller delays)

If true virtual time becomes essential, the path would be an optional
pure-simulation backend (Turmoil-style) behind the same Lab/Router/Device API,
accepting the loss of kernel TCP fidelity for that mode. That is a separate
project decision, not a patchbay enhancement.
