# Architecture Decision: Default Core, Provider Packs, and Capability Boundaries

This document freezes the architectural direction referenced by `todo.md` Phase 0. It is the baseline for future refactors.

Scope note:

- this document defines the long-term packaging and capability-boundary rules,
- the near-term supported surfaces, layering method, L0/L1/L2 repo split, and enterprise-repo boundary are defined in [架构总览：内核、服务、分层与企业仓库边界](./kernel-service-enterprise-boundaries.md).

## Status

Accepted.

## Why This Decision Exists

`ditto-llm` has accumulated several valid capabilities, but the boundaries between protocol, provider, capability, catalog, and runtime implementation are still too loose. That looseness is the main scaling risk.

If we keep adding providers and features without tightening those boundaries, the codebase will continue to drift toward:

- feature combinations that imply support without implementing it,
- catalog entries that do not correspond to runtime behavior,
- provider quirks scattered across adapters and translation code,
- default builds that silently pull in more product surface than intended.

This decision defines the stable direction that later code changes must follow.

## Decision Summary

The default `ditto-llm` core is:

- `provider-openai-compatible`
- `cap-llm`

Everything else is opt-in.

That means:

- the default build provides a minimal, stable text-generation core,
- official OpenAI is not the default provider semantics,
- Anthropic, Google, OpenAI official, and other specific providers are optional provider packs,
- non-LLM capabilities are separate capability packs and must be gated explicitly.

## Core Concepts

These concepts must remain separate.

### Protocol Surface

A protocol surface is the upstream API shape we are talking to.

Examples:

- `openai/chat.completions`
- `openai/responses`
- `anthropic/messages`
- `google/generateContent`

It answers: which endpoint family and request-response contract are we targeting?

### Provider

A provider is the upstream service operator or access channel.

Examples:

- OpenAI
- Anthropic
- Google
- OpenRouter
- DeepSeek
- Bailian
- Qianfan

A provider may expose one or more protocol surfaces. A protocol surface may also be offered by multiple providers.

It answers: who operates the endpoint and authentication domain?

### Capability

A capability is the user-visible operation category.

Examples:

- `llm`
- `embedding`
- `image-generation`
- `image-edit`
- `audio-transcription`
- `audio-speech`
- `moderation`
- `rerank`
- `batch`
- `realtime`

It answers: what can this model or provider do?

### Catalog Source

Catalog source is the canonical runtime metadata that describes provider and model support.

The runtime direction is:

- reference TOML/JSON/CSV/docs remain useful as evidence and generation inputs,
- runtime truth lives in Rust-native registry types,
- generated artifacts are allowed, but runtime should not depend on ad hoc string guessing.

It answers: where does runtime support information come from?

## Default Product Boundary

The default product is intentionally small.

### Included In The Default Core

The default core must provide:

- text generation,
- streaming text output,
- basic tool-calling strategy for LLM flows,
- warnings and explicit compatibility boundaries,
- direct SDK use without requiring the gateway feature.

### Explicitly Not Included In The Default Core

The default core does not automatically include:

- embeddings,
- image generation,
- image editing,
- audio transcription,
- audio speech,
- moderation,
- rerank,
- batch,
- realtime,
- gateway/server runtime,
- GUI applications under `apps/*`.

Those are opt-in capabilities or adjacent products.

## Provider Packaging Rules

All providers must become explicit provider packs.

Rules:

- The generic OpenAI-compatible path is the default base layer.
- Official OpenAI is a separate provider pack, not the default semantic identity.
- Anthropic, Google, DeepSeek, Kimi, OpenRouter, Bailian, Qianfan, MiniMax, Doubao, Zhipu, Hunyuan, and others are separate provider packs.
- A provider pack may depend on a shared protocol family implementation, but that reuse must be explicit.
- A provider may not be treated as "supported" unless its declared capabilities are queryable and implemented in runtime.

## Capability Rules

All providers must declare capabilities by category.

Rules:

- Capabilities are not inferred from provider names.
- Capabilities are not inferred only from model name prefixes.
- Provider-level capability declarations and model-level capability declarations must both exist and must be consistent.
- Runtime must reject unimplemented capability requests explicitly.
- Translation and gateway layers must mount endpoints only for capabilities that are actually implemented.

## Catalog Rules

Catalog and runtime must stop drifting apart.

Rules:

- Provider catalog entries must correspond to runtime provider identities.
- Model capability descriptors must correspond to runtime capability bindings.
- It is acceptable to track future work in docs, but not acceptable to leave provider support permanently implied without implementation.
- If catalog data exists only as evidence and has no runtime binding yet, it must be marked as non-runtime reference data.

## Migration Invariants

Future refactors must preserve these invariants.

### Configuration Compatibility

- Existing configs should continue to load where possible.
- Backward-compatibility aliases are acceptable during migration.
- Old naming may remain as deprecated aliases temporarily, but the new provider/capability naming becomes the documented source of truth.
- Silent semantic changes are not acceptable.

### Feature Migration

The target naming scheme is:

- `provider-*` for provider packs,
- `cap-*` for capability packs.

Legacy names may exist only as migration aliases until callers are updated.

### Default Behavior

The end state changes the default behavior:

- default build means OpenAI-compatible LLM core,
- not official OpenAI by default,
- not non-LLM endpoints by default,
- not gateway/server by default.

These changes must be reflected in docs, examples, tests, and error messages.

## Minimum Experience Required From The Default Core

The default core must remain good enough to be useful on its own.

Minimum bar:

- one stable LLM request type for text generation,
- stream support for incremental text output,
- a basic tool-calling path for LLM workflows,
- explicit warnings when provider behavior is best-effort,
- no hidden dependency on gateway, admin APIs, or GUI apps.

## Non-Goals

This decision does not require immediate implementation of every provider pack or every capability pack.

This decision also does not require:

- removing all legacy names in one change,
- deleting evidence catalogs,
- collapsing SDK and gateway into one runtime mode,
- putting L2 enterprise workflow features into this repository.

## Consequences

Code and docs should now evolve in this order:

1. Freeze naming and boundaries.
2. Rework feature flags around provider packs and capability packs.
3. Introduce stronger runtime registry types.
4. Make catalog truth feed runtime binding instead of drifting beside it.
5. Expand providers and capabilities incrementally under those boundaries.

Any future change that conflicts with this document should be treated as architecture regression unless the decision itself is amended.
