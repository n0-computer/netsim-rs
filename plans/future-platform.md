# Future: Distributed Systems Debug/Test/Bench Platform

Where patchbay can go beyond v1. Ordered roughly by expected impact and
feasibility.

---

## Near-term (post v1)

### qlog comparison
Sum packet and frame counts from per-device qlog files. Show deltas in compare
view as stacked bar charts. Since qlog is already collected per device, this is
mostly parsing + aggregation. Useful for spotting QUIC behavioral changes (more
retransmits, different frame mix) between versions. The compare UI already
prepares for this (code comments mark where to add qlog parsing and delta
display). Implementation: parse qlog JSON, bucket events by type, diff counts
between left/right runs, render as compact delta table in CompareSummary.

### Flaky test detection
Run N iterations of a test suite, track pass/fail rate per test over time. Flag
tests whose failure rate exceeds a threshold. Store history in
`.patchbay/work/flaky-history.jsonl`. Integrate with compare: exclude known-flaky
tests from regression scoring.

### Network fault injection schedules
Programmable chaos beyond static `set-link-condition`. Define schedules:
```toml
[[fault-schedule]]
device = "client"
at = "5s"
condition = { latency_ms = 500, jitter_ms = 100 }
at = "15s"
condition = "reset"  # back to normal
```
Partition events, delay spikes, packet reordering, bandwidth oscillation. Make it
easy to simulate real-world network instability over time.

### Multi-region latency matrices
Define region topologies with realistic inter-region RTT from cloud provider
measurements:
```toml
[regions]
us-east = { router = "r1" }
eu-west = { router = "r2" }
ap-south = { router = "r3" }

[latency-matrix]
us-east.eu-west = { rtt_ms = 80, jitter_ms = 5 }
us-east.ap-south = { rtt_ms = 180, jitter_ms = 15 }
eu-west.ap-south = { rtt_ms = 140, jitter_ms = 10 }
```

### Bisect mode
`patchbay bisect <good-ref> <bad-ref>` — binary search git history for the commit
that introduced a regression (test failure or metric threshold breach). Uses
compare infrastructure internally. Could integrate with `git bisect run`.

---

## Mid-term

### Record & replay
**High impact.** Capture full packet traces (pcap per device via `tcpdump` in
namespace) during a run. Store alongside events. Replay deterministically by
injecting captured packets back into namespaces, without running the original
binaries. Enables:
- Exact reproduction of a failure without rebuilding
- Sharing a failure as a self-contained artifact
- Regression testing against recorded network behavior

### Distributed tracing correlation
Collect OpenTelemetry spans from all devices (via OTLP receiver per namespace),
stitch into a unified trace. Visualize in UI: see a request flow from client
through NAT, relay, to server. Correlate with network events (link condition
changes, NAT rebinds) on the timeline.

### Benchmark suites
Named benchmark profiles tracking key metrics over time:
```toml
[benchmark.relay-throughput]
sim = "iperf-relay.toml"
metric = "iperf.down_bytes"
direction = "higher_is_better"
threshold_regression = "5%"
```
Track like `criterion` but for distributed scenarios. CI posts trend graphs
on PRs.

### CI integration
`patchbay ci` mode:
- Posts compare results as GitHub PR comments (markdown table)
- Blocks merge on regression thresholds
- Stores history in a central patchbay-server instance
- Supports `--push` to send results to remote server

### Resource profiling per device
Collect CPU, memory, fd count, socket buffer usage per namespace. Correlate with
network events in the timeline. Implemented via periodic `/proc` sampling inside
each namespace worker thread.

---

## Long-term vision

### Cluster mode
Distribute devices across multiple machines for large-scale simulations (100+
nodes). Coordination via iroh/QUIC. Each machine runs a patchbay agent that
manages local namespaces. Central orchestrator assigns devices to agents and
manages cross-machine virtual links (via tunnels).

### Protocol conformance testing
Pluggable test harnesses that verify protocol implementations against specs.
Ship reference test suites for QUIC, STUN/TURN, DNS, HTTP/3. Run against any
implementation by pointing at its binary.

### AI-assisted debugging
Feed topology + events + logs + metrics into an LLM to answer:
- "Why did this connection fail?"
- "Why is throughput 10x lower than baseline?"
- "What changed between these two runs that explains the regression?"
Possible via structured context from events.jsonl + metrics.jsonl + qlog.

### Snapshot & restore
Freeze entire lab state: all namespace configurations, iptables rules, tc qdiscs,
routing tables, running process state. Serialize to disk. Restore later for
deterministic debugging. Like VM snapshots but for namespace-based labs.

### Visual topology editor
Drag-and-drop in the UI to build topologies. Export to TOML. Live-edit during
inspect sessions — add/remove devices, change NAT/firewall, apply link conditions,
all from the browser.

### Shared test infrastructure
Hosted patchbay-server where teams push results from CI. Compare across
branches, PRs, releases. Retention policies, alerting on metric regressions,
team dashboards. Like a Grafana for network simulation results.
