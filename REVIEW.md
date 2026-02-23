# Codebase Review

Higher-level suggestions that were not applied directly.

---

## 1. `VmBinarySpec` duplicates `BinarySpec` (sim/build.rs vs crates/netsim-vm/src/vm.rs) ✅

`VmBinarySpec` in `vm.rs` (lines 484–497) is field-for-field identical to `BinarySpec` in
`src/sim/mod.rs`. Likewise, `vm_binary_mode` in `vm.rs` is a character-for-character copy of
`binary_mode` in `src/sim/build.rs`.

**Suggestion**: Expose `BinarySpec` (and `binary_mode` or its logic) from the `netsim` crate and
reuse them in `netsim-vm`. The main barrier is that `BinarySpec` currently lives inside `sim/`
which is a binary-crate module; moving it to `netsim::assets` or a new `netsim::sim_types` module
would allow sharing.

---

## 2. Multi-pass router resolution is a manual topological sort (src/lib.rs `from_config`) ✅

The loop in `from_config` (lines 333–357) repeatedly scans `remaining` routers until all upstream
references resolve. This is O(n²) and conceptually a topological sort.

**Suggestion**: Use `petgraph` or implement a simple Kahn's algorithm. Alternatively, pre-build an
adjacency list and do a single DFS/BFS. The current approach also silently loops forever if the
graph has a cycle not caught by the `!changed` guard (the guard only exits on a stall, which is
correct but subtle).

---

## 3. `artifact_name_kind` allocates unnecessarily (src/sim/build.rs) ✅

Changed to return `(&str, bool)`; the inline temporary at the `"target"` arm was bound to `let
artifact`; `args.push` call-sites use `.to_owned()` where a `String` is needed.

---

## 4. `CaptureStore` accessor pattern is asymmetric (src/sim/capture.rs) ✅

`CaptureStore::get` unwraps the `Arc`/`Mutex` inline (`self.inner.0.lock()`), bypassing the
structured `(Mutex, Condvar)` destructuring used everywhere else. A small private helper
`fn lock(&self) -> MutexGuard<CaptureInner>` would make all three methods uniform.

---

## 5. `write_progress` / `write_run_manifest` are copy-paste twins (src/sim/progress.rs) ✅

Both functions serialize a value with `serde_json::to_string_pretty`, then `tokio::fs::write` it
to a path under `run_root`. The only differences are the filename and the type serialized.

**Suggestion**: Extract a private async helper:
```rust
async fn write_json(path: &Path, value: &impl Serialize) -> Result<()>
```

---

## 6. `stage_build_binary` in netsim-vm/src/util.rs duplicates the example→bin fallback logic

`stage_build_binary` (util.rs lines 66–115) manually tries `--example` then falls back to `--bin`,
mirroring the same fallback in `build_local_binaries_blocking` (src/sim/build.rs lines 167–197).
Both exist because the vm crate needs to cross-compile while the host builder does not.

**Not applied**: The two paths diverge significantly (cross-compile target, blocking single-step
vs batched multi-artifact, different artifact path derivation). A shared helper would need to
replicate all these arguments and would be used only twice with different semantics. Left as-is.

---

## 7. `SimFile` / `LabConfig` topology duplication (src/sim/topology.rs) ✅

`load_topology` returns a `LabConfig` constructed from either a file or from `sim.router` /
`sim.device` / `sim.region` inline fields, which are typed identically to `LabConfig`. `SimFile`
therefore embeds the same three fields that `LabConfig` has, requiring an explicit clone in the
inline branch (`Ok(LabConfig { router: sim.router.clone(), ... })`).

**Suggestion**: Embed `#[serde(flatten)] pub topology: LabConfig` inside `SimFile`, eliminating
the three separate fields and the clone.

---

## 8. `StepTemplateDef` / `StepGroupDef` expansion in runner.rs is deeply ad-hoc

Template/group expansion (`expand_steps` in runner.rs) operates on raw `toml::value::Table` maps
with manual key merging, then re-serializes to TOML text and re-deserializes into `Step`. This
round-trip is fragile and hard to test.

**Not applied**: The review description was inaccurate — the code already uses
`toml::Value::Table(table).try_into::<Step>()` (value → deserializer → type), not a
serialize-to-string round-trip. The `toml::Value` map approach is equivalent to the suggested
`toml::Deserializer` approach. No change needed.

---

## 9. `url_cache_key` could use `hex` crate or iterator collect ✅

Replaced `collect::<String>()` with a `String::with_capacity(32)` buffer written via `write!`
to avoid the 16 intermediate `String` allocations.

---

## 10. `binary_cache.rs` `shared_cache_root` heuristic is fragile ✅

Removed `shared_cache_root` entirely (the heuristic never fired with the current `.assemble/`
work-dir structure). `cached_binary_for_url` now takes `cache_dir: &Path` directly; callers
pass `work_dir.join(".binary-cache")` explicitly.
