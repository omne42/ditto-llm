# ditto-gateway-contract-types

Frozen Rust types for Ditto Gateway contract v0.1.

- Contract ID: `gateway-v0.1`
- Contract version: `0.1.0`
- OpenAPI source: `contracts/gateway-contract-v0.1.openapi.yaml`

This crate embeds the OpenAPI YAML as `GATEWAY_OPENAPI_V0_1_YAML` and exports stable
request/response models for:

- health
- proxy envelope (`ProxyJsonEnvelope`)
- admin audit
- admin budgets/costs ledgers
- reservations reap
- shared error shape
- admin query models (`ListAuditLogsQuery`, `ListLedgersQuery`)
