# Module Boundaries and Dependency Direction

This document turns the current architecture discussion into an explicit module-boundary plan for `ditto-llm`.

It complements [Architecture Decision](./architecture-decision.md) by focusing on **code layout**, **dependency direction**, and **migration order** inside the current Rust workspace.

## Status

Accepted as the directory-convergence target.

## Why This Exists

`ditto-llm` already has the correct macro direction:

- L0: provider adapters, protocol conversion, SDK primitives
- L1: gateway / proxy / control-plane
- L2: enterprise closed-loop platform in a separate repository

The problem is that the source tree does not consistently enforce that design.

The main failure modes today are:

- core abstractions directly depending on concrete providers,
- `profile` becoming a mixed namespace for config, env, auth, catalog bridge, and routing policy,
- `catalog` depending on dynamic configuration concerns,
- gateway translation acting as the only real runtime assembly layer,
- oversized modules that absorb unrelated responsibilities.

If this continues, the result is predictable: circular dependencies, provider-specific branching in the wrong layers, and painful future crate-splitting.

## Target Tree

The long-term internal tree should converge toward:

```text
src/
├── core/
├── capabilities/
├── config/
├── catalog/
├── providers/
├── runtime/
├── sdk/
├── agent/
└── gateway/
```

### `core/`

Pure foundational abstractions.

Examples:

- errors
- stream primitives
- model traits
- middleware/layer traits

`core` must not depend on `providers`, `runtime`, `sdk`, `agent`, or `gateway`.

### `capabilities/`

Capability facades and capability-local types.

Examples:

- `llm`
- `embedding`
- `image`
- `audio`
- `file`
- `batch`

This is where large mixed type files should eventually be split by domain instead of living forever in monolithic `types/mod.rs`.

### `config/`

Dynamic user-controlled configuration.

Examples:

- env loading
- provider config
- auth config
- routing policy config
- config editor / upsert / list / delete flows

This layer is the replacement destination for the current overloaded `profile` namespace.

### `catalog/`

Static provider and model metadata.

Examples:

- provider registry
- capability descriptors
- model behaviors
- generated builtin catalogs

`catalog` describes what is statically known.
It must not become the place that interprets mutable end-user configuration policy.

### `providers/`

Concrete upstream implementations.

Examples:

- OpenAI
- OpenAI-compatible family
- Anthropic
- Google
- Bedrock
- provider shared protocol-family code

`providers` may depend on `core`, `capabilities`, and `config`, but must not depend on `gateway`.

### `runtime/`

The assembly layer.

This is the key missing boundary.

`runtime` joins:

- static catalog truth,
- dynamic config,
- concrete provider builders.

Responsibilities include:

- resolve runtime routes,
- pick the correct provider surface for a model,
- validate config against catalog/runtime capability truth,
- provide gateway-agnostic model/provider factory entry points.

### `sdk/`

Developer-facing helpers and protocols.

Examples:

- telemetry
- devtools
- HTTP stream protocol helpers
- client-side protocol shims

### `agent/`

Higher-level orchestration inside this repository.

`agent` should depend on lower layers, not the reverse.
It is not the L2 enterprise platform boundary described in the roadmap.

### `gateway/`

Top-level HTTP application and control plane.

Internally it should converge toward:

```text
gateway/
├── domain/
├── application/
├── adapters/
└── transport/
```

- `domain/`: budgets, rate limits, policy interfaces, audit abstractions
- `application/`: translation flows, orchestration, request handling use-cases
- `adapters/`: sqlite/redis/postgres/http glue
- `transport/`: axum router + HTTP handlers only

## Dependency Rules

These are the rules that code review should enforce.

1. `core` is bottom-most.
2. `capabilities` depends on `core`.
3. `catalog` depends on `core` and catalog-native identifiers, but should avoid depending on mutable config policy.
4. `config` may depend on core identifiers and catalog enums/ids.
5. `providers` depends on `core + capabilities + config`.
6. `runtime` depends on `catalog + config + providers`.
7. `gateway` depends on `runtime + sdk + domain adapters`.
8. `sdk` must not import `gateway` internals.
9. `agent` must not become a backdoor dependency of lower layers.

## Recent Progress

- `crate::config` and `crate::runtime` now exist as real facades instead of only aspirational names.
- `catalog/resolver.rs` already resolves config types through `crate::config`, not `crate::profile`.
- OpenAI-compatible family quirks inference moved out of the legacy `profile` namespace into a provider-shared module; `profile` remains a compatibility shell.
- `crates/ditto-core/src/capabilities/file.rs` is back to being a pure capability facade; `FileClient` implementations now live in provider code.
- `cargo check --features agent` is green again after repairing shared OpenAI-family gates.

## Immediate Corrections

These are the first concrete corrections already underway.

### 1. Introduce a real `config` facade

The first migration step is to stop teaching new code to depend on `crate::profile::*` directly.

New code should prefer `crate::config::*`.

This allows implementation files to move later without a flag day.

### 2. Introduce a real `runtime` facade

`gateway` must stop being the only practical place where config + catalog + provider assembly happens.

The runtime resolution API should have a gateway-neutral home.

### 3. Move `catalog` off direct `profile` naming

A concrete smell in the current tree is `catalog/resolver.rs` importing `ProviderConfig` from `profile` directly.

That is the wrong direction semantically, even if the type is currently defined there.

The import path should already speak in terms of `config`, and later the implementation can move underneath it.

## Migration Order

To keep the repository stable, migration should happen in this order.

### Phase 1: Break naming-level coupling

- add `config/` facade
- add `runtime/` facade
- switch new call sites from `profile::*` to `config::*`
- switch runtime resolution entry points to `runtime::*`

### Phase 2: Move pure config code out of `profile`

Move these first:

- env
- provider config
- auth config
- routing policy config

Leave compatibility re-exports in `profile` temporarily.

### Phase 3: Split domain type blobs

Split `types/mod.rs` and similar mixed modules by capability domain.

### Phase 4: Move runtime assembly out of gateway-heavy code

Gateway translation should call runtime factories/resolvers, not own the only assembly logic itself.

### Phase 5: Shrink public root exports

After paths are stable, reduce the number of root-level `pub use` re-exports from `lib.rs`.

That forces callers to respect layer boundaries instead of treating the crate root as a flat namespace.

## What This Does Not Require Yet

This plan does **not** require immediately splitting the repository into multiple Cargo crates.

That would be premature while boundaries are still moving.

The right sequence is:

1. clean boundaries inside one crate,
2. stabilize imports and responsibilities,
3. only then evaluate crate extraction.

## Success Criteria

This module-boundary effort is successful when:

- new code no longer grows `profile` as a junk drawer,
- `catalog` is statically descriptive instead of config-aware by accident,
- runtime assembly has a dedicated home,
- gateway becomes a consumer of runtime assembly instead of the owner of it,
- future crate-splitting becomes mechanical instead of conceptual.
