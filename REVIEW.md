# Codebase Review

Higher-level suggestions that were not applied directly.

---

## Open

*(no open items)*

---

## Completed

1. **`VmBinarySpec` duplicates `BinarySpec`** — unified via shared `netsim` crate dependency; `BinarySpec` exposed from `netsim::assets` ✅
2. **Multi-pass router resolution is a manual topological sort** — identified O(n²) loop in `from_config`; cycle guard correct but subtle; left as-is (acceptable for current topology sizes) ✅
3. **`artifact_name_kind` allocates unnecessarily** — changed to return `(&str, bool)`; call-sites use `.to_owned()` only where needed ✅
4. **`CaptureStore` accessor pattern is asymmetric** — private `fn lock()` helper added for uniform access ✅
5. **`write_progress` / `write_run_manifest` are copy-paste twins** — private `async fn write_json(path, value)` helper extracted ✅
6. **`stage_build_binary` duplicates example→bin fallback logic** — not applied; the two paths diverge significantly (cross-compile target, blocking vs batched, different artifact derivation) ✅
7. **`SimFile` / `LabConfig` topology duplication** — `#[serde(flatten)] pub topology: LabConfig` applied inside `SimFile` ✅
8. **`StepTemplateDef` expansion round-trip is fragile** — not applied; description was inaccurate; code already uses `toml::Value::Table.try_into::<Step>()` correctly ✅
9. **`url_cache_key` uses intermediate `String` allocations** — replaced with `String::with_capacity(32)` buffer written via `write!` ✅
10. **`binary_cache.rs` `shared_cache_root` heuristic is fragile** — `shared_cache_root` removed entirely; callers pass `cache_dir: &Path` explicitly ✅
11. **`netsim-core/src/lib.rs` monolith** — split into `lab.rs` + `config.rs`; `lib.rs` slimmed to ~80 LOC of module declarations and re-exports ✅
12. **Bridge/namespace naming in `Lab`** — moved fully into `NetworkCore` (private `bridge_counter`, `ns_counter`, `next_bridge_name()`, `next_ns_name()`); callers pass no names ✅
13. **Transparent type aliases `RouterId = NodeId` etc.** — removed; all code uses `NodeId`; `router_id_by_name()` / `device_id_by_name()` added to `NetworkCore`; duplicate name maps removed from `Lab` ✅
14. **Duplicate `spawn_reflector_in` + crate-root probe exports** — duplicate removed; `probe_in_ns`, `udp_roundtrip_in_ns`, `udp_rtt_in_ns` moved into `test_utils.rs`; no re-exports at crate root ✅
15. **Dead iperf UI table** — `IperfResult` interface and iperf table JSX removed from `ui/src/types.ts` and `ui/src/components/PerfTab.tsx` ✅
16. **`Lab::init_tracing()` was cfg(test)-only no-op** — replaced by `netsim_utils::init_tracing()` called at startup in both `netsim` and `netsim-vm` binaries ✅
