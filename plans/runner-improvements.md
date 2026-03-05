# Runner Improvements

## Unify `sim.topology` into `[[extends]]`

**Current state**: Two separate mechanisms for loading external files:
- `sim.topology = "name"` → loads `../topos/<name>.toml` (network layout only)
- `[[extends]]` → loads templates, groups, binaries, prepare specs (ignores topology)

**Proposal**: Make `[[extends]]` also merge topology (router/device/region) from the extended file,
then deprecate `sim.topology`.

Instead of:
```toml
[sim]
topology = "1to1-public"
```

Write:
```toml
[[extends]]
file = "../topos/1to1-public.toml"
```

**Pros**:
- One import concept instead of two
- More flexible: a single file can provide topology + templates + binaries
- `extends` already parses files as `SimFile` which includes topology via `#[serde(flatten)]`

**Cons**:
- `sim.topology = "name"` auto-searches `../topos/<name>.toml` — convenient shorthand
- Breaking change for existing sim files (need migration period)
- Need to decide merge semantics: does extends topology merge or override inline?

**Plan**:
1. Make `load_extends` also collect topology from extended files
2. Merge topology: routers append, devices merge by name (inline wins)
3. Keep `sim.topology` working but emit a deprecation warning
4. Update iroh sims to use `extends` for topology in a follow-up
