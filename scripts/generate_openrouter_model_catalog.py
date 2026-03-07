#!/usr/bin/env python3
from __future__ import annotations

import argparse
import datetime as dt
import json
import sys
import urllib.request
from collections import OrderedDict, defaultdict
from pathlib import Path

from provider_model_catalog_json import write_json_sidecar

MODELS_DOC_URL = 'https://openrouter.ai/docs/api/api-reference/models/get-models'
MODELS_API_URL = 'https://openrouter.ai/api/v1/models'
RANKINGS_URL = 'https://openrouter.ai/rankings'
CHAT_API_DOC_URL = 'https://openrouter.ai/docs/api/api-reference/chat/send-chat-completion-request'
RESPONSES_API_DOC_URL = 'https://openrouter.ai/docs/api/api-reference/responses/create-responses'
DEFAULT_OUTPUT = Path(__file__).resolve().parents[1] / 'catalog' / 'provider_models' / 'openrouter.toml'
DEFAULT_LIMIT = 300


def fetch_text(url: str, timeout: float = 30.0) -> str:
    request = urllib.request.Request(
        url,
        headers={
            'User-Agent': 'ditto-llm/openrouter-model-catalog-generator',
            'Accept': 'text/html,application/json',
        },
    )
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return response.read().decode('utf-8', 'ignore')


def fetch_json(url: str, timeout: float = 30.0) -> dict:
    return json.loads(fetch_text(url, timeout=timeout))


def extract_ranking_data(page_html: str) -> list[dict]:
    text = page_html.replace('\\"', '"')
    marker = '"rankingData":['
    start = text.find(marker)
    if start == -1:
        raise ValueError('unable to locate rankingData in rankings page')
    array_start = start + len(marker) - 1
    depth = 0
    array_end: int | None = None
    for idx, ch in enumerate(text[array_start:], start=array_start):
        if ch == '[':
            depth += 1
        elif ch == ']':
            depth -= 1
            if depth == 0:
                array_end = idx + 1
                break
    if array_end is None:
        raise ValueError('unterminated rankingData array')
    return json.loads(text[array_start:array_end])


def aggregate_weekly_requests(ranking_data: list[dict]) -> list[tuple[str, int]]:
    counts: defaultdict[str, int] = defaultdict(int)
    for row in ranking_data:
        slug = row.get('model_permaslug')
        if not slug:
            continue
        counts[str(slug)] += int(row.get('count') or 0)
    return sorted(counts.items(), key=lambda item: item[1], reverse=True)


def normalize_brand(model_id: str) -> str:
    if '/' not in model_id:
        return model_id
    return model_id.split('/', 1)[0]


def infer_categories(model: dict) -> list[str]:
    inputs = set(model.get('architecture', {}).get('input_modalities') or [])
    outputs = set(model.get('architecture', {}).get('output_modalities') or [])
    categories: list[str] = []
    if 'text' in inputs or 'text' in outputs:
        categories.append('文本模型')
    if inputs & {'image', 'audio', 'video', 'file'}:
        categories.append('多模态模型')
    if 'image' in outputs:
        categories.append('图像生成模型')
    if 'audio' in inputs or 'audio' in outputs:
        categories.append('音频模型')
    if 'video' in inputs or 'video' in outputs:
        categories.append('视频模型')
    if 'file' in inputs:
        categories.append('文件输入模型')
    return categories


def infer_supported_features(model: dict) -> list[str]:
    params = set(model.get('supported_parameters') or [])
    pricing = model.get('pricing') or {}
    inputs = set(model.get('architecture', {}).get('input_modalities') or [])
    outputs = set(model.get('architecture', {}).get('output_modalities') or [])
    features: list[str] = []
    if 'tools' in params or 'tool_choice' in params:
        features.append('tool_calls')
    if 'response_format' in params or 'structured_outputs' in params:
        features.append('structured_outputs')
    if 'reasoning' in params or 'include_reasoning' in params:
        features.append('reasoning')
    if 'seed' in params:
        features.append('seeded_generation')
    if 'web_search_options' in params or 'web_search' in pricing:
        features.append('web_search')
    if 'input_cache_read' in pricing or 'input_cache_write' in pricing:
        features.append('context_caching')
    if 'image' in inputs:
        features.append('vision')
    if 'audio' in inputs:
        features.append('audio_input')
    if 'video' in inputs:
        features.append('video_input')
    if 'image' in outputs:
        features.append('image_output')
    if 'audio' in outputs:
        features.append('audio_output')
    return features


def clean_nulls(mapping: dict | None) -> OrderedDict[str, object]:
    out: OrderedDict[str, object] = OrderedDict()
    if not mapping:
        return out
    for key, value in mapping.items():
        if value is None:
            continue
        out[str(key)] = value
    return out


def select_models(limit: int) -> list[dict]:
    ranking_rows = extract_ranking_data(fetch_text(RANKINGS_URL))
    ranked_counts = aggregate_weekly_requests(ranking_rows)

    models = fetch_json(MODELS_API_URL).get('data') or []
    by_id = {model['id']: model for model in models}
    by_canonical = {
        model.get('canonical_slug'): model
        for model in models
        if model.get('canonical_slug')
    }

    selected: list[dict] = []
    seen_ids: set[str] = set()
    for ranking_permaslug, weekly_requests in ranked_counts:
        model = by_id.get(ranking_permaslug) or by_canonical.get(ranking_permaslug)
        if model is None:
            continue
        model_id = str(model['id'])
        if model_id in seen_ids:
            continue
        seen_ids.add(model_id)
        selected.append(
            {
                'model': model,
                'ranking_permaslug': ranking_permaslug,
                'weekly_requests': weekly_requests,
            }
        )
        if len(selected) >= limit:
            break

    if len(selected) < limit:
        raise RuntimeError(
            f'only matched {len(selected)} currently available models from rankings, expected at least {limit}'
        )
    return selected


def toml_quote(value: str) -> str:
    escaped = (
        value.replace('\\', '\\\\')
        .replace('"', '\\"')
        .replace('\n', '\\n')
        .replace('\r', '\\r')
        .replace('\t', '\\t')
    )
    return f'"{escaped}"'


def write_inline_array(values: list[object]) -> str:
    rendered: list[str] = []
    for value in values:
        rendered.append(render_scalar(value))
    return '[' + ', '.join(rendered) + ']'


def render_scalar(value: object) -> str:
    if isinstance(value, bool):
        return str(value).lower()
    if isinstance(value, int):
        return str(value)
    if isinstance(value, float):
        return repr(value)
    return toml_quote(str(value))


def write_table_section(lines: list[str], prefix: str, values: OrderedDict[str, object]) -> None:
    if not values:
        return
    lines.append('')
    lines.append(f'[{prefix}]')
    for key, value in values.items():
        if isinstance(value, list):
            lines.append(f'{key} = {write_inline_array(list(value))}')
        else:
            lines.append(f'{key} = {render_scalar(value)}')


def write_catalog(selected: list[dict], output_path: Path) -> None:
    generated_at = dt.datetime.utcnow().replace(microsecond=0).isoformat() + 'Z'
    lines = [
        '# Generated from official OpenRouter APIs and rankings. Edit via scripts/generate_openrouter_model_catalog.py.',
        '# Sources:',
        f'# - {MODELS_DOC_URL}',
        f'# - {MODELS_API_URL}',
        f'# - {RANKINGS_URL}',
        f'# - {CHAT_API_DOC_URL}',
        f'# - {RESPONSES_API_DOC_URL}',
        f'# Generated at: {generated_at}',
        '',
        '[provider]',
        'id = "openrouter"',
        'display_name = "OpenRouter"',
        'base_url = "https://openrouter.ai/api/v1"',
        'protocol = "openai"',
        f'source_url = {toml_quote(MODELS_DOC_URL)}',
        f'source_urls = {write_inline_array([MODELS_DOC_URL, MODELS_API_URL, RANKINGS_URL, CHAT_API_DOC_URL, RESPONSES_API_DOC_URL])}',
        f'supported_api_surfaces = {write_inline_array(["model.list", "chat.completion", "response.create.beta"])}',
        f'selection_method = {toml_quote("Aggregate official weekly rankingData.count by model_permaslug from /rankings, map to the current /api/v1/models list via id or canonical_slug, and keep the top 300 distinct available model ids.")}',
        f'selection_limit = {len(selected)}',
        '',
        '[provider.auth]',
        'type = "api_key_env"',
        'keys = ["OPENROUTER_API_KEY"]',
        '',
        '[provider.endpoints]',
        'model_list = "https://openrouter.ai/api/v1/models"',
        'chat_completion = "https://openrouter.ai/api/v1/chat/completions"',
        'responses_beta = "https://openrouter.ai/api/v1/responses"',
        '',
    ]

    for rank, item in enumerate(selected, start=1):
        model = item['model']
        model_id = str(model['id'])
        architecture = model.get('architecture') or {}
        pricing = clean_nulls(model.get('pricing'))
        top_provider = clean_nulls(model.get('top_provider'))
        default_parameters = clean_nulls(model.get('default_parameters'))
        per_request_limits = clean_nulls(model.get('per_request_limits'))
        categories = infer_categories(model)
        supported_features = infer_supported_features(model)

        lines.append(f'[models.{toml_quote(model_id)}]')
        top_level = OrderedDict(
            [
                ('source_url', MODELS_API_URL),
                ('source_urls', [MODELS_API_URL, RANKINGS_URL, MODELS_DOC_URL]),
                ('docs_source_url', MODELS_DOC_URL),
                ('ranking_source_url', RANKINGS_URL),
                ('chat_api_source_url', CHAT_API_DOC_URL),
                ('display_name', str(model.get('name') or model_id)),
                ('status', 'active'),
                ('vendor', 'openrouter'),
                ('brand', normalize_brand(model_id)),
                ('canonical_slug', str(model.get('canonical_slug') or model_id)),
                ('hugging_face_id', model.get('hugging_face_id')),
                ('ranking_permaslug', item['ranking_permaslug']),
                ('weekly_rank', rank),
                ('weekly_requests', int(item['weekly_requests'])),
                ('created_unix', int(model.get('created') or 0)),
                ('api_surfaces', ['chat.completion']),
                ('categories', categories),
                ('summary', str(model.get('description') or '').strip()),
                ('context_window_tokens', model.get('context_length')),
                ('max_output_tokens', top_provider.get('max_completion_tokens')),
                ('input_modalities', list(architecture.get('input_modalities') or [])),
                ('output_modalities', list(architecture.get('output_modalities') or [])),
                ('supported_parameters', list(model.get('supported_parameters') or [])),
                ('supported_features', supported_features),
                ('expiration_date', model.get('expiration_date')),
            ]
        )
        for key, value in top_level.items():
            if value is None:
                continue
            if isinstance(value, list) and not value:
                continue
            lines.append(f'{key} = {write_inline_array(value) if isinstance(value, list) else render_scalar(value)}')

        write_table_section(
            lines,
            f'models.{toml_quote(model_id)}.architecture',
            OrderedDict(
                (
                    (key, value)
                    for key, value in OrderedDict(
                        [
                            ('modality', architecture.get('modality')),
                            ('tokenizer', architecture.get('tokenizer')),
                            ('instruct_type', architecture.get('instruct_type')),
                        ]
                    ).items()
                    if value is not None
                )
            ),
        )
        write_table_section(lines, f'models.{toml_quote(model_id)}.pricing', pricing)
        write_table_section(lines, f'models.{toml_quote(model_id)}.top_provider', top_provider)
        write_table_section(lines, f'models.{toml_quote(model_id)}.default_parameters', default_parameters)
        write_table_section(lines, f'models.{toml_quote(model_id)}.per_request_limits', per_request_limits)
        lines.append('')

    output_path.write_text('\n'.join(lines), encoding='utf-8')


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        '--output',
        default=str(DEFAULT_OUTPUT),
        help='Output TOML path',
    )
    parser.add_argument(
        '--limit',
        type=int,
        default=DEFAULT_LIMIT,
        help='Number of currently available models to keep by weekly OpenRouter usage',
    )
    args = parser.parse_args()

    output_path = Path(args.output)
    output_path.parent.mkdir(parents=True, exist_ok=True)

    try:
        selected = select_models(max(1, args.limit))
        write_catalog(selected, output_path)
        json_output_path = write_json_sidecar(output_path)
    except Exception as exc:  # noqa: BLE001
        print(f'failed to generate OpenRouter catalog: {exc}', file=sys.stderr)
        return 1

    print(f'wrote {output_path} ({len(selected)} models)', file=sys.stderr)
    print(f'wrote {json_output_path}', file=sys.stderr)
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
