# Plan: Remove globals, better spawn via RT handle cloning

**Status:** ✅ complete

## Context

netsim-core has two global singletons (`FD_REGISTRY`, `GLOBAL_NETNS_MANAGER`) and a custom `TaskHandle<T>` over oneshot channels + message-passing. All three can go:

- **Globals** → scope `NetnsManager` to `NetworkCore`; scope fd storage to `Worker`
- **TaskHandle** → clone the per-ns `current_thread` tokio `Handle` onto Worker, call `handle.spawn()` directly, return `JoinHandle<T>`

User's proof-of-concept confirms `handle.spawn(fut)` runs the future on the worker's OS thread (which has done `setns`).

**IX as Router?** No — IX owns the bridge (not connected via veth), has no WAN/NAT/uplink/NodeId. Deferred. Add lightweight `Ix` handle instead.

---

## Steps

- [x] **Step 1: Restructure Worker / AsyncWorker — RT handle + CancellationToken** (`netns.rs`)
  - Remove `AsyncWorker` struct, `AsyncMsg` enum, `TaskHandle<T>`, `TaskCancelled`
  - Remove `FD_REGISTRY`, `FdRegistry`, `GLOBAL_NETNS_MANAGER`, all global free functions
  - Worker gets: `ns_fd: OnceLock<Arc<File>>`, `rt_handle: OnceLock<Handle>`, `netlink: OnceLock<Netlink>`, `cancel_token: CancellationToken`
  - On first async use: spawn OS thread → `setns` → create `current_thread` RT → send `rt.handle().clone()` back via oneshot → `rt.block_on(cancel_token.cancelled())`
  - On `NetnsManager` drop: cancel all tokens, join threads
  - New API: `rt_handle_for()`, `netlink_for()`, `spawn_thread_in()` (replaces `spawn_closure_in_netns`)
  - `run_closure_in` unchanged (SyncWorker)
  - Add `tokio-util` dep for `CancellationToken`

- [x] **Step 2: Add `Ix` handle, `spawn_thread`/`run_sync` on Device/Router** (`lab.rs`)
  - `Ix` struct with `ns()`, `gw()`, `gw_v6()`, `spawn()`, `spawn_thread()`, `run_sync()`, `spawn_reflector()`
  - `Lab::ix() -> Ix`
  - Device/Router: add `spawn_thread()`, `run_sync()`
  - Device/Router `spawn()`: return `JoinHandle<T>` via `rt_handle_for` instead of `TaskHandle<T>`
  - Migrate `spawn_reflector_on_ix` → `Ix::spawn_reflector`

- [x] **Step 3: Remove ns param from helpers, callers use run_sync** (`core.rs`, `qdisc.rs`)
  - `set_sysctl_in` removed; callers do `netns.run_closure_in(ns, || set_sysctl_root(...))`
  - `qdisc.rs` drops ALL ns params; callers wrap in `run_closure_in`
  - Remove thin wrappers: `run_closure_in_namespace`, `spawn_closure_in_namespace_thread`, `run_command_in_namespace`, `spawn_command_in_namespace`
  - Note: `run_nft_in` and `apply_impair_in` retained as internal convenience wrappers

- [x] **Step 4: Update test_utils.rs**
  - All probe helpers namespace-free, called inside `dev.run_sync`
  - `run_reflector` is async, uses `CancellationToken`

- [x] **Step 5: Refactor test helpers** (`tests.rs`)
  - Helpers become ns-free, called inside `dev.spawn`/`dev.run_sync`/`dev.spawn_thread`
  - ~60+ call sites updated

- [x] **Step 6: Cleanup exports** (`lib.rs`, `main.rs`)
  - Remove `cleanup_registry_prefix` from public API
  - Add `Ix` to public exports
  - Remove `TaskHandle` / `TaskCancelled`
  - Simplify `perform_cleanup` in main.rs
