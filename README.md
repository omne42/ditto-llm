# ditto-llm

Standalone Rust crate extracted from CodePM.

Current scope:

- Provider profile config (`base_url` / auth / model whitelist / capability flags)
- OpenAI-compatible `GET /models` discovery
- Model-level `thinking` config (mapped by consumers to `reasoning.effort`)
