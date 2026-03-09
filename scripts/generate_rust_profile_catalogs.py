#!/usr/bin/env python3
from __future__ import annotations

import json
from pathlib import Path

try:
    import tomllib  # type: ignore[attr-defined]
except ModuleNotFoundError:  # pragma: no cover - compatibility for older Python
    import tomli as tomllib  # type: ignore[no-redef]
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
SOURCE_DIR = ROOT / 'catalog' / 'provider_models'
TARGET_FILE = ROOT / 'src' / 'profile' / 'generated_catalogs.rs'


def rust_string(value: str) -> str:
    return json.dumps(value, ensure_ascii=False)


def owned_string_expr(value: str) -> str:
    return f'{rust_string(value)}.to_string()'


def option_string_expr(value: Any) -> str:
    if value is None:
        return 'None'
    return f'Some({owned_string_expr(str(value))})'


def option_u64_expr(value: Any) -> str:
    if value is None:
        return 'None'
    return f'Some({int(value)})'


def vec_string_expr(values: list[Any]) -> str:
    if not values:
        return 'vec![]'
    rendered = ', '.join(owned_string_expr(str(value)) for value in values)
    return f'vec![{rendered}]'


def btreemap_expr(entries: dict[str, Any], value_expr) -> str:
    if not entries:
        return 'BTreeMap::new()'
    rendered = ', '.join(
        f'({owned_string_expr(str(key))}, {value_expr(value)})'
        for key, value in sorted(entries.items())
    )
    return f'BTreeMap::from([{rendered}])'


def bool_expr(value: Any) -> str:
    return 'true' if bool(value) else 'false'


def string_expr(value: Any) -> str:
    return owned_string_expr(str(value))


def openai_modality_expr(value: str) -> str:
    mapping = {
        'input_only': 'OpenAiModalitySupport::InputOnly',
        'output_only': 'OpenAiModalitySupport::OutputOnly',
        'input_and_output': 'OpenAiModalitySupport::InputAndOutput',
        'not_supported': 'OpenAiModalitySupport::NotSupported',
    }
    return mapping[value]


def openai_availability_status_expr(value: str) -> str:
    mapping = {
        'unverified': 'OpenAiAvailabilityStatus::Unverified',
        'available': 'OpenAiAvailabilityStatus::Available',
        'cache_questionable': 'OpenAiAvailabilityStatus::CacheQuestionable',
        'availability_questionable': 'OpenAiAvailabilityStatus::AvailabilityQuestionable',
    }
    return mapping[value]


def anthropic_status_expr(value: str) -> str:
    mapping = {
        'active': 'AnthropicModelStatus::Active',
        'legacy': 'AnthropicModelStatus::Legacy',
        'deprecated': 'AnthropicModelStatus::Deprecated',
        'retired': 'AnthropicModelStatus::Retired',
    }
    return mapping[value]


def provider_auth_expr(auth: dict[str, Any]) -> str:
    auth_type = str(auth.get('type') or '').strip()
    if auth_type == 'api_key_env':
        return f'ProviderAuth::ApiKeyEnv {{ keys: {vec_string_expr(auth.get("keys") or [])} }}'
    if auth_type == 'query_param_env':
        return (
            'ProviderAuth::QueryParamEnv { '
            f'param: {owned_string_expr(str(auth.get("param") or "key"))}, '
            f'keys: {vec_string_expr(auth.get("keys") or [])}, '
            f'prefix: {option_string_expr(auth.get("prefix"))}, '
            '}'
        )
    raise ValueError(f'unsupported auth type for generated profile catalog: {auth_type}')


def openai_provider_expr(provider: dict[str, Any]) -> str:
    return (
        'OpenAiCatalogProvider { '
        f'id: {owned_string_expr(provider["id"])}, '
        f'display_name: {owned_string_expr(provider["display_name"])}, '
        f'base_url: {owned_string_expr(provider["base_url"])}, '
        f'protocol: {owned_string_expr(provider["protocol"])}, '
        f'source_url: {owned_string_expr(provider["source_url"])}, '
        f'auth: {provider_auth_expr(provider["auth"])}, '
        '}'
    )


def google_provider_expr(provider: dict[str, Any]) -> str:
    return (
        'GoogleCatalogProvider { '
        f'id: {owned_string_expr(provider["id"])}, '
        f'display_name: {owned_string_expr(provider["display_name"])}, '
        f'base_url: {owned_string_expr(provider["base_url"])}, '
        f'protocol: {owned_string_expr(provider["protocol"])}, '
        f'source_url: {owned_string_expr(provider["source_url"])}, '
        f'auth: {provider_auth_expr(provider["auth"])}, '
        '}'
    )


def anthropic_provider_expr(provider: dict[str, Any]) -> str:
    return (
        'AnthropicCatalogProvider { '
        f'id: {owned_string_expr(provider["id"])}, '
        f'display_name: {owned_string_expr(provider["display_name"])}, '
        f'base_url: {owned_string_expr(provider["base_url"])}, '
        f'protocol: {owned_string_expr(provider["protocol"])}, '
        f'source_url: {owned_string_expr(provider["source_url"])}, '
        f'auth: {provider_auth_expr(provider["auth"])}, '
        '}'
    )


def openai_entry_expr(entry: dict[str, Any]) -> str:
    return (
        'OpenAiModelCatalogEntry { '
        f'source_url: {owned_string_expr(entry["source_url"])}, '
        f'availability_status: {openai_availability_status_expr(entry.get("availability_status") or "unverified")}, '
        f'display_name: {owned_string_expr(entry["display_name"])}, '
        f'stage: {option_string_expr(entry.get("stage"))}, '
        f'tagline: {option_string_expr(entry.get("tagline"))}, '
        f'summary: {option_string_expr(entry.get("summary"))}, '
        f'performance: {option_string_expr(entry.get("performance"))}, '
        f'speed: {option_string_expr(entry.get("speed"))}, '
        f'input: {option_string_expr(entry.get("input"))}, '
        f'output: {option_string_expr(entry.get("output"))}, '
        f'context_window: {option_u64_expr(entry.get("context_window"))}, '
        f'max_output_tokens: {option_u64_expr(entry.get("max_output_tokens"))}, '
        f'knowledge_cutoff: {option_string_expr(entry.get("knowledge_cutoff"))}, '
        f'modalities: {btreemap_expr(entry.get("modalities") or {}, openai_modality_expr)}, '
        f'features: {btreemap_expr(entry.get("features") or {}, bool_expr)}, '
        f'tools: {btreemap_expr(entry.get("tools") or {}, bool_expr)}, '
        'revisions: OpenAiModelRevisions { '
        f'snapshots: {vec_string_expr((entry.get("revisions") or {}).get("snapshots") or [])}, '
        '}, '
        '}'
    )


def google_entry_expr(entry: dict[str, Any]) -> str:
    versions = entry.get('versions') or []
    version_expr = 'vec![]'
    if versions:
        version_expr = 'vec![' + ', '.join(
            'GoogleModelVersion { '
            f'channel: {owned_string_expr(version["channel"])}, '
            f'model: {owned_string_expr(version["model"])}, '
            '}'
            for version in versions
        ) + ']'
    supported = entry.get('supported_data_types') or {}
    return (
        'GoogleModelCatalogEntry { '
        f'source_url: {owned_string_expr(entry["source_url"])}, '
        f'display_name: {owned_string_expr(entry["display_name"])}, '
        f'model_code: {owned_string_expr(entry["model_code"])}, '
        f'summary: {option_string_expr(entry.get("summary"))}, '
        f'latest_update: {option_string_expr(entry.get("latest_update"))}, '
        'supported_data_types: GoogleSupportedDataTypes { '
        f'input: {vec_string_expr(supported.get("input") or [])}, '
        f'output: {vec_string_expr(supported.get("output") or [])}, '
        '}, '
        f'limits: {btreemap_expr(entry.get("limits") or {}, string_expr)}, '
        f'capabilities: {btreemap_expr(entry.get("capabilities") or {}, string_expr)}, '
        f'versions: {version_expr}, '
        '}'
    )


def anthropic_pricing_expr(pricing: dict[str, Any] | None) -> str:
    if not pricing:
        return 'None'
    return (
        'Some(AnthropicModelPricing { '
        f'input_usd_per_mtok: {owned_string_expr(pricing["input_usd_per_mtok"])}, '
        f'output_usd_per_mtok: {owned_string_expr(pricing["output_usd_per_mtok"])}, '
        '})'
    )


def anthropic_entry_expr(entry: dict[str, Any]) -> str:
    return (
        'AnthropicModelCatalogEntry { '
        f'source_url: {owned_string_expr(entry["source_url"])}, '
        f'lifecycle_source_url: {option_string_expr(entry.get("lifecycle_source_url"))}, '
        f'display_name: {owned_string_expr(entry["display_name"])}, '
        f'api_model_id: {owned_string_expr(entry["api_model_id"])}, '
        f'api_alias: {option_string_expr(entry.get("api_alias"))}, '
        f'bedrock_model_id: {option_string_expr(entry.get("bedrock_model_id"))}, '
        f'vertex_model_id: {option_string_expr(entry.get("vertex_model_id"))}, '
        f'description: {option_string_expr(entry.get("description"))}, '
        f'input_modalities: {vec_string_expr(entry.get("input_modalities") or [])}, '
        f'output_modalities: {vec_string_expr(entry.get("output_modalities") or [])}, '
        f'features: {btreemap_expr(entry.get("features") or {}, bool_expr)}, '
        f'comparative_latency: {option_string_expr(entry.get("comparative_latency"))}, '
        f'context_window_tokens: {option_u64_expr(entry.get("context_window_tokens"))}, '
        f'beta_context_window_tokens: {option_u64_expr(entry.get("beta_context_window_tokens"))}, '
        f'max_output_tokens: {option_u64_expr(entry.get("max_output_tokens"))}, '
        f'reliable_knowledge_cutoff: {option_string_expr(entry.get("reliable_knowledge_cutoff"))}, '
        f'training_data_cutoff: {option_string_expr(entry.get("training_data_cutoff"))}, '
        f'beta_headers: {btreemap_expr(entry.get("beta_headers") or {}, string_expr)}, '
        f'pricing: {anthropic_pricing_expr(entry.get("pricing"))}, '
        f'status: {anthropic_status_expr(entry["status"])}, '
        f'deprecated_on: {option_string_expr(entry.get("deprecated_on"))}, '
        f'retirement_date: {option_string_expr(entry.get("retirement_date"))}, '
        f'not_retired_before: {option_string_expr(entry.get("not_retired_before"))}, '
        f'recommended_replacement: {option_string_expr(entry.get("recommended_replacement"))}, '
        '}'
    )


def render_models_map(models: dict[str, Any], entry_expr) -> str:
    if not models:
        return 'BTreeMap::new()'
    rendered = ',\n            '.join(
        f'({owned_string_expr(model_id)}, {entry_expr(models[model_id])})'
        for model_id in sorted(models)
    )
    return 'BTreeMap::from([\n            ' + rendered + '\n        ])'


def load_toml(name: str) -> dict[str, Any]:
    with (SOURCE_DIR / name).open('rb') as fh:
        return tomllib.load(fh)


def render_file() -> str:
    openai = load_toml('openai.toml')
    google = load_toml('google.toml')
    anthropic = load_toml('anthropic.toml')

    return '\n'.join([
        '// Generated by scripts/generate_rust_profile_catalogs.py. Do not edit by hand.',
        'use std::collections::BTreeMap;',
        '',
        'use super::anthropic_model_catalog::{',
        '    AnthropicCatalogProvider, AnthropicModelCatalog, AnthropicModelCatalogEntry,',
        '    AnthropicModelPricing, AnthropicModelStatus,',
        '};',
        'use super::config::ProviderAuth;',
        'use super::google_model_catalog::{',
        '    GoogleCatalogProvider, GoogleModelCatalog, GoogleModelCatalogEntry,',
        '    GoogleModelVersion, GoogleSupportedDataTypes,',
        '};',
        'use super::openai_model_catalog::{',
        '    OpenAiAvailabilityStatus, OpenAiCatalogProvider, OpenAiModalitySupport,',
        '    OpenAiModelCatalog, OpenAiModelCatalogEntry, OpenAiModelRevisions,',
        '};',
        '',
        'pub(crate) fn generated_openai_model_catalog() -> OpenAiModelCatalog {',
        '    OpenAiModelCatalog {',
        f'        provider: {openai_provider_expr(openai["provider"] )},',
        f'        models: {render_models_map(openai.get("models") or {}, openai_entry_expr)},',
        '    }',
        '}',
        '',
        'pub(crate) fn generated_google_model_catalog() -> GoogleModelCatalog {',
        '    GoogleModelCatalog {',
        f'        provider: {google_provider_expr(google["provider"])},',
        f'        models: {render_models_map(google.get("models") or {}, google_entry_expr)},',
        '    }',
        '}',
        '',
        'pub(crate) fn generated_anthropic_model_catalog() -> AnthropicModelCatalog {',
        '    AnthropicModelCatalog {',
        f'        provider: {anthropic_provider_expr(anthropic["provider"])},',
        f'        models: {render_models_map(anthropic.get("models") or {}, anthropic_entry_expr)},',
        '    }',
        '}',
        '',
    ])


def main() -> int:
    TARGET_FILE.write_text(render_file(), encoding='utf-8')
    print(TARGET_FILE.relative_to(ROOT))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
