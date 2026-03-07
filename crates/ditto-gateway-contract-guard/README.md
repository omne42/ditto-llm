# ditto-gateway-contract-guard

CI guard for frozen gateway contract files.

Checks:

- OpenAPI shape diff (`--base` vs `--head`) and classifies changes as:
  - `breaking`
  - `feature` (non-breaking shape change)
  - `patch` (textual-only edits)
- Semver gate on `info.version`:
  - `breaking`: requires major bump (or minor bump when major is `0`)
  - `feature`: requires minor/major bump
  - `patch`: requires any version bump
- Version/id sync across artifacts:
  - OpenAPI `info.version` == `GATEWAY_CONTRACT_VERSION` == `package.version`
  - OpenAPI `info.x-ditto-contract-id` == `GATEWAY_CONTRACT_ID`

Example:

```bash
cargo run --manifest-path crates/ditto-gateway-contract-guard/Cargo.toml -- \
  --base /tmp/base.openapi.yaml \
  --head ./contracts/gateway-contract-v0.1.openapi.yaml \
  --contract-lib ./crates/ditto-gateway-contract-types/src/lib.rs \
  --contract-cargo ./crates/ditto-gateway-contract-types/Cargo.toml
```
