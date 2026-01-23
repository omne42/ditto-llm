# ditto-llm

Standalone Rust crate extracted from CodePM.

Current scope:

- Provider profile config (`base_url` / auth / model whitelist)
- OpenAI-compatible `GET /models` discovery + capability flags
- Model-level `thinking` config (mapped by consumers to `reasoning.effort`)
