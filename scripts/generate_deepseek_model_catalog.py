#!/usr/bin/env python3
from __future__ import annotations

import argparse
import datetime as dt
import re
import sys
import urllib.request
from collections import OrderedDict
from pathlib import Path

from bs4 import BeautifulSoup

from provider_model_catalog_json import write_json_sidecar

FIRST_CALL_URL = 'https://api-docs.deepseek.com/'
PRICING_URL = 'https://api-docs.deepseek.com/quick_start/pricing/'
LIST_MODELS_URL = 'https://api-docs.deepseek.com/api/list-models/'
CHAT_API_URL = 'https://api-docs.deepseek.com/api/create-chat-completion'
FIM_API_URL = 'https://api-docs.deepseek.com/api/create-completion'
THINKING_URL = 'https://api-docs.deepseek.com/guides/thinking_mode/'
TOOL_CALLS_URL = 'https://api-docs.deepseek.com/guides/tool_calls/'
PREFIX_URL = 'https://api-docs.deepseek.com/guides/chat_prefix_completion/'
FIM_GUIDE_URL = 'https://api-docs.deepseek.com/guides/fim_completion/'
JSON_URL = 'https://api-docs.deepseek.com/guides/json_mode/'
KV_CACHE_URL = 'https://api-docs.deepseek.com/guides/kv_cache'
UPDATES_URL = 'https://api-docs.deepseek.com/updates/'
DEFAULT_BASE_URL = 'https://api.deepseek.com'
DEFAULT_OUTPUT = Path(__file__).resolve().parents[1] / 'catalog' / 'provider_models' / 'deepseek.toml'

YES_MARKERS = {'✓', '✔', 'supported', 'fully supported', 'auto'}
NO_MARKERS = {'✗', '×', 'not supported'}


def fetch_html(url: str, timeout: float = 30.0) -> str:
    req = urllib.request.Request(
        url,
        headers={
            'User-Agent': 'ditto-llm/deepseek-model-catalog-generator',
            'Accept': 'text/html,application/xhtml+xml',
        },
    )
    with urllib.request.urlopen(req, timeout=timeout) as response:
        return response.read().decode('utf-8', 'ignore')


def article_text(html: str) -> str:
    soup = BeautifulSoup(html, 'html.parser')
    article = soup.find('article')
    if article is None:
        raise RuntimeError('unable to locate article content')
    return ' '.join(article.get_text('\n', strip=True).split())


def normalize_text(value: str) -> str:
    return ' '.join(value.replace('\xa0', ' ').split())


def parse_bool(value: str) -> bool:
    lowered = normalize_text(value).lower()
    if lowered in NO_MARKERS or lowered.endswith('✗'):
        return False
    if lowered in YES_MARKERS or lowered.endswith('✓'):
        return True
    if '✓' in value or '\u2713' in value:
        return True
    if '✗' in value or '\u2717' in value:
        return False
    raise RuntimeError(f'unrecognized support marker: {value!r}')


def parse_k_tokens(value: str) -> int:
    match = re.search(r'(\d+(?:\.\d+)?)\s*K', value, re.I)
    if not match:
        raise RuntimeError(f'unable to parse token count from {value!r}')
    return int(float(match.group(1)) * 1000)


def parse_max_output(value: str) -> tuple[int, int]:
    match = re.search(r'DEFAULT:\s*(\d+)K\s*MAXIMUM:\s*(\d+)K', value, re.I)
    if not match:
        raise RuntimeError(f'unable to parse max output range from {value!r}')
    return int(match.group(1)) * 1000, int(match.group(2)) * 1000


def table_rows(html: str) -> list[list[str]]:
    soup = BeautifulSoup(html, 'html.parser')
    article = soup.find('article')
    if article is None:
        raise RuntimeError('unable to locate article table container')
    table = article.find('table')
    if table is None:
        raise RuntimeError('unable to locate pricing table')
    rows: list[list[str]] = []
    for tr in table.find_all('tr'):
        cells = [normalize_text(cell.get_text(' ', strip=True)) for cell in tr.find_all(['th', 'td'])]
        if cells:
            rows.append(cells)
    return rows


def parse_pricing(rows: list[list[str]]) -> tuple[list[str], dict[str, dict], dict[str, str]]:
    if rows[0][0] != 'MODEL':
        raise RuntimeError('unexpected pricing table header')
    model_ids = rows[0][1:]
    if model_ids != ['deepseek-chat', 'deepseek-reasoner']:
        raise RuntimeError(f'unexpected pricing model ids: {model_ids!r}')

    base_url = rows[1][1]
    context_length = parse_k_tokens(rows[3][1])
    version_cells = rows[2][1:]
    max_output_cells = rows[4][1:]

    features: dict[str, list[bool]] = {}
    pricing: dict[str, str] = {}
    for row in rows[5:]:
        if row[0] == 'FEATURES':
            features[row[1]] = [parse_bool(cell) for cell in row[2:]]
            continue
        if row[0] == 'PRICING':
            pricing[row[1]] = row[2]
            continue
        if row[0].startswith('1M '):
            pricing[row[0]] = row[1]
            continue
        features[row[0]] = [parse_bool(cell) for cell in row[1:]]

    models: dict[str, dict] = {}
    for index, model_id in enumerate(model_ids):
        default_max, max_max = parse_max_output(max_output_cells[index])
        supported_features: list[str] = []
        beta_features: list[str] = []
        if features['Json Output'][index]:
            supported_features.append('json_output')
        if features['Tool Calls'][index]:
            supported_features.append('tool_calls')
        if features['Chat Prefix Completion（Beta）'][index]:
            beta_features.append('chat_prefix_completion')
        if features['FIM Completion（Beta）'][index]:
            beta_features.append('fim_completion')
        models[model_id] = {
            'base_url': base_url,
            'model_version': version_cells[index],
            'context_window_tokens': context_length,
            'default_max_output_tokens': default_max,
            'max_output_tokens': max_max,
            'supported_features': supported_features,
            'beta_features': beta_features,
        }
    return model_ids, models, pricing


def extract_current_models(text: str) -> list[str]:
    matches = []
    for model_id in re.findall(r'deepseek-[a-z0-9.-]+', text):
        if model_id not in matches:
            matches.append(model_id)
    return [model_id for model_id in matches if model_id in {'deepseek-chat', 'deepseek-reasoner'}]


def extract_latest_v32_upgrade_date(text: str) -> str:
    match = re.search(r'Date:\s*(\d{4}-\d{2}-\d{2}).{0,80}?DeepSeek-V3\.2\b', text, re.S)
    if not match:
        raise RuntimeError('unable to find DeepSeek-V3.2 upgrade date in updates page')
    return match.group(1)


def validate_sources(source_texts: dict[str, str], model_ids: list[str]) -> str:
    first_call = source_texts['first_call']
    if 'https://api.deepseek.com/v1 as the base_url' not in first_call:
        raise RuntimeError('first-call page missing /v1 compatibility note')
    if 'deepseek-chat is the non-thinking mode of DeepSeek-V3.2' not in first_call:
        raise RuntimeError('first-call page missing DeepSeek-V3.2 alias description')

    list_models = source_texts['list_models']
    current_models = extract_current_models(list_models)
    if current_models != model_ids:
        raise RuntimeError(f'list-models page mismatch: {current_models!r}')

    chat_api = source_texts['chat_api']
    if 'Possible values: [ deepseek-chat , deepseek-reasoner ]' not in chat_api:
        raise RuntimeError('chat API page missing model enum')
    if 'response_format' not in chat_api or 'json_object' not in chat_api:
        raise RuntimeError('chat API page missing JSON output parameter docs')

    fim_api = source_texts['fim_api']
    if 'Possible values: [ deepseek-chat ]' not in fim_api:
        raise RuntimeError('FIM API page missing deepseek-chat-only enum')
    if 'base_url="https://api.deepseek.com/beta"' not in fim_api:
        raise RuntimeError('FIM API page missing beta base_url requirement')

    thinking = source_texts['thinking']
    if '"model": "deepseek-reasoner"' not in thinking:
        raise RuntimeError('thinking guide missing deepseek-reasoner enable path')
    if 'model = "deepseek-chat"' not in thinking or '"thinking" : { "type" : "enabled" }' not in thinking:
        raise RuntimeError('thinking guide missing deepseek-chat toggle path')
    if 'Not Supported Features ：FIM (Beta)' not in thinking:
        raise RuntimeError('thinking guide missing reasoner FIM limitation')

    tool_calls = source_texts['tool_calls']
    if 'From DeepSeek-V3.2, the API supports tool use in the thinking mode.' not in tool_calls:
        raise RuntimeError('tool-calls guide missing thinking-mode support note')
    if 'base_url="https://api.deepseek.com/beta" to enable Beta features' not in tool_calls:
        raise RuntimeError('tool-calls guide missing strict-mode beta note')

    prefix = source_texts['prefix']
    if 'base_url="https://api.deepseek.com/beta" to enable the Beta feature' not in prefix:
        raise RuntimeError('prefix guide missing beta base_url note')

    kv_cache = source_texts['kv_cache']
    if 'enabled by default for all users' not in kv_cache:
        raise RuntimeError('context-caching guide missing default enablement note')

    updates = source_texts['updates']
    return extract_latest_v32_upgrade_date(updates)


def build_catalog() -> OrderedDict[str, dict]:
    html_pages = {
        'pricing_html': fetch_html(PRICING_URL),
    }
    source_texts = {
        'first_call': article_text(fetch_html(FIRST_CALL_URL)),
        'list_models': article_text(fetch_html(LIST_MODELS_URL)),
        'chat_api': article_text(fetch_html(CHAT_API_URL)),
        'fim_api': article_text(fetch_html(FIM_API_URL)),
        'thinking': article_text(fetch_html(THINKING_URL)),
        'tool_calls': article_text(fetch_html(TOOL_CALLS_URL)),
        'prefix': article_text(fetch_html(PREFIX_URL)),
        'json': article_text(fetch_html(JSON_URL)),
        'kv_cache': article_text(fetch_html(KV_CACHE_URL)),
        'updates': article_text(fetch_html(UPDATES_URL)),
    }

    model_ids, pricing_models, pricing = parse_pricing(table_rows(html_pages['pricing_html']))
    latest_upgrade_date = validate_sources(source_texts, model_ids)

    common_price_hit = pricing['1M INPUT TOKENS (CACHE HIT)']
    common_price_miss = pricing['1M INPUT TOKENS (CACHE MISS)']
    common_price_output = pricing['1M OUTPUT TOKENS']

    models: OrderedDict[str, dict] = OrderedDict()

    chat_features = pricing_models['deepseek-chat']['supported_features'] + ['context_caching', 'thinking_parameter', 'streaming']
    chat_beta = pricing_models['deepseek-chat']['beta_features'] + ['strict_tool_calls']
    models['deepseek-chat'] = {
        'source_url': PRICING_URL,
        'source_urls': [
            PRICING_URL,
            FIRST_CALL_URL,
            LIST_MODELS_URL,
            CHAT_API_URL,
            THINKING_URL,
            TOOL_CALLS_URL,
            PREFIX_URL,
            FIM_API_URL,
            FIM_GUIDE_URL,
            JSON_URL,
            KV_CACHE_URL,
            UPDATES_URL,
        ],
        'display_name': 'deepseek-chat',
        'status': 'active',
        'vendor': 'deepseek',
        'api_surfaces': ['chat.completion', 'completion.fim.beta', 'context.cache'],
        'categories': ['文本模型'],
        'summary': 'Default DeepSeek API alias for DeepSeek-V3.2 non-thinking mode. It also supports thinking via the thinking parameter, plus JSON output, tool calls, context caching, beta chat prefix completion, and beta FIM completion.',
        'model_version': pricing_models['deepseek-chat']['model_version'],
        'context_window_tokens': pricing_models['deepseek-chat']['context_window_tokens'],
        'default_max_output_tokens': pricing_models['deepseek-chat']['default_max_output_tokens'],
        'max_output_tokens': pricing_models['deepseek-chat']['max_output_tokens'],
        'input_modalities': ['text'],
        'output_modalities': ['text'],
        'supported_features': chat_features,
        'beta_features': chat_beta,
        'latest_upgrade_date': latest_upgrade_date,
        'pricing_input_cache_hit_per_million': common_price_hit,
        'pricing_input_cache_miss_per_million': common_price_miss,
        'pricing_output_per_million': common_price_output,
        'records': [
            OrderedDict(
                table_kind='source_table',
                source_url=PRICING_URL,
                source_page='pricing',
                section='Models & Pricing / Model Details',
                columns=['MODEL', 'BASE URL', 'MODEL VERSION', 'CONTEXT LENGTH', 'MAX OUTPUT', 'FEATURES'],
                values=[
                    'deepseek-chat',
                    pricing_models['deepseek-chat']['base_url'],
                    pricing_models['deepseek-chat']['model_version'],
                    '128K',
                    'DEFAULT: 4K MAXIMUM: 8K',
                    'Json Output / Tool Calls / Chat Prefix Completion (Beta) / FIM Completion (Beta)',
                ],
            ),
            OrderedDict(
                table_kind='api_reference',
                source_url=LIST_MODELS_URL,
                source_page='list_models',
                section='Lists Models',
                api_surface='model.list',
                method='GET',
                endpoint='https://api.deepseek.com/models',
                notes='Listed as a current model in the official /models example.',
            ),
            OrderedDict(
                table_kind='api_reference',
                source_url=CHAT_API_URL,
                source_page='create_chat_completion',
                section='Create Chat Completion',
                api_surface='chat.completion',
                method='POST',
                endpoint='https://api.deepseek.com/chat/completions',
                notes='Primary OpenAI-compatible chat endpoint.',
            ),
            OrderedDict(
                table_kind='api_reference',
                source_url=FIM_API_URL,
                source_page='create_fim_completion',
                section='Create FIM Completion (Beta)',
                api_surface='completion.fim.beta',
                method='POST',
                endpoint='https://api.deepseek.com/completions',
                base_url='https://api.deepseek.com/beta',
                notes='FIM Completion is beta-only and available only for deepseek-chat.',
            ),
            OrderedDict(
                table_kind='feature_support',
                source_url=THINKING_URL,
                source_page='thinking_mode',
                section='Thinking Mode',
                feature='thinking_parameter',
                support='supported',
                notes='deepseek-chat can enter thinking mode via extra_body.thinking.type=enabled.',
            ),
            OrderedDict(
                table_kind='feature_support',
                source_url=PREFIX_URL,
                source_page='chat_prefix_completion',
                section='Chat Prefix Completion (Beta)',
                feature='chat_prefix_completion_beta',
                support='supported',
                base_url='https://api.deepseek.com/beta',
                notes='The last assistant message must set prefix=true.',
            ),
            OrderedDict(
                table_kind='feature_support',
                source_url=KV_CACHE_URL,
                source_page='kv_cache',
                section='Context Caching',
                feature='context_caching',
                support='enabled_by_default',
                notes='Context caching is enabled by default for all users and bills repeated prefixes as cache hits.',
            ),
            OrderedDict(
                table_kind='release_note',
                source_url=UPDATES_URL,
                source_page='updates',
                section='DeepSeek-V3.2',
                release_date=latest_upgrade_date,
                notes='deepseek-chat now maps to DeepSeek-V3.2 non-thinking mode.',
            ),
        ],
    }

    reasoner_features = pricing_models['deepseek-reasoner']['supported_features'] + ['context_caching', 'streaming', 'reasoning_content']
    reasoner_beta = pricing_models['deepseek-reasoner']['beta_features'] + ['strict_tool_calls']
    models['deepseek-reasoner'] = {
        'source_url': PRICING_URL,
        'source_urls': [
            PRICING_URL,
            FIRST_CALL_URL,
            LIST_MODELS_URL,
            CHAT_API_URL,
            THINKING_URL,
            TOOL_CALLS_URL,
            PREFIX_URL,
            JSON_URL,
            KV_CACHE_URL,
            UPDATES_URL,
        ],
        'display_name': 'deepseek-reasoner',
        'status': 'active',
        'vendor': 'deepseek',
        'api_surfaces': ['chat.completion', 'context.cache'],
        'categories': ['文本模型', '推理模型'],
        'summary': 'Default DeepSeek API alias for DeepSeek-V3.2 thinking mode. It exposes reasoning_content, supports JSON output, tool calls, context caching, and beta chat prefix completion, but does not support FIM.',
        'model_version': pricing_models['deepseek-reasoner']['model_version'],
        'context_window_tokens': pricing_models['deepseek-reasoner']['context_window_tokens'],
        'default_max_output_tokens': pricing_models['deepseek-reasoner']['default_max_output_tokens'],
        'max_output_tokens': pricing_models['deepseek-reasoner']['max_output_tokens'],
        'input_modalities': ['text'],
        'output_modalities': ['text'],
        'supported_features': reasoner_features,
        'beta_features': reasoner_beta,
        'not_supported_features': ['fim_completion'],
        'latest_upgrade_date': latest_upgrade_date,
        'pricing_input_cache_hit_per_million': common_price_hit,
        'pricing_input_cache_miss_per_million': common_price_miss,
        'pricing_output_per_million': common_price_output,
        'records': [
            OrderedDict(
                table_kind='source_table',
                source_url=PRICING_URL,
                source_page='pricing',
                section='Models & Pricing / Model Details',
                columns=['MODEL', 'BASE URL', 'MODEL VERSION', 'CONTEXT LENGTH', 'MAX OUTPUT', 'FEATURES'],
                values=[
                    'deepseek-reasoner',
                    pricing_models['deepseek-reasoner']['base_url'],
                    pricing_models['deepseek-reasoner']['model_version'],
                    '128K',
                    'DEFAULT: 32K MAXIMUM: 64K',
                    'Json Output / Tool Calls / Chat Prefix Completion (Beta)',
                ],
            ),
            OrderedDict(
                table_kind='api_reference',
                source_url=LIST_MODELS_URL,
                source_page='list_models',
                section='Lists Models',
                api_surface='model.list',
                method='GET',
                endpoint='https://api.deepseek.com/models',
                notes='Listed as a current model in the official /models example.',
            ),
            OrderedDict(
                table_kind='api_reference',
                source_url=CHAT_API_URL,
                source_page='create_chat_completion',
                section='Create Chat Completion',
                api_surface='chat.completion',
                method='POST',
                endpoint='https://api.deepseek.com/chat/completions',
                notes='Primary OpenAI-compatible chat endpoint for thinking mode.',
            ),
            OrderedDict(
                table_kind='feature_support',
                source_url=THINKING_URL,
                source_page='thinking_mode',
                section='Thinking Mode',
                feature='reasoning_content',
                support='supported',
                notes='Returns reasoning_content and requires it to be passed back correctly during thinking-mode tool loops.',
            ),
            OrderedDict(
                table_kind='feature_support',
                source_url=TOOL_CALLS_URL,
                source_page='tool_calls',
                section='Tool Calls / Thinking Mode',
                feature='tool_calls',
                support='supported',
                notes='From DeepSeek-V3.2, tool use is supported in thinking mode.',
            ),
            OrderedDict(
                table_kind='feature_support',
                source_url=PREFIX_URL,
                source_page='chat_prefix_completion',
                section='Chat Prefix Completion (Beta)',
                feature='chat_prefix_completion_beta',
                support='supported',
                base_url='https://api.deepseek.com/beta',
                notes='Chat prefix completion is supported in thinking mode; FIM remains unsupported.',
            ),
            OrderedDict(
                table_kind='feature_support',
                source_url=KV_CACHE_URL,
                source_page='kv_cache',
                section='Context Caching',
                feature='context_caching',
                support='enabled_by_default',
                notes='Context caching is enabled by default for all users and bills repeated prefixes as cache hits.',
            ),
            OrderedDict(
                table_kind='release_note',
                source_url=UPDATES_URL,
                source_page='updates',
                section='DeepSeek-V3.2',
                release_date=latest_upgrade_date,
                notes='deepseek-reasoner now maps to DeepSeek-V3.2 thinking mode.',
            ),
        ],
    }

    return models


def toml_quote(value: str) -> str:
    return '"' + value.replace('\\', '\\\\').replace('"', '\\"') + '"'


def toml_array(values: list[str]) -> str:
    return '[' + ', '.join(toml_quote(value) for value in values) + ']'


def write_key_value(lines: list[str], key: str, value) -> None:
    if isinstance(value, list):
        lines.append(f'{key} = {toml_array(value)}')
    elif isinstance(value, bool):
        lines.append(f"{key} = {'true' if value else 'false'}")
    elif isinstance(value, int):
        lines.append(f'{key} = {value}')
    else:
        lines.append(f'{key} = {toml_quote(str(value))}')


def write_record(lines: list[str], path: str, record: OrderedDict[str, object]) -> None:
    lines.append(f'[[{path}]]')
    for key, value in record.items():
        write_key_value(lines, key, value)
    lines.append('')


def render_toml(models: OrderedDict[str, dict]) -> str:
    now = dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat().replace('+00:00', 'Z')
    lines = [
        '# Generated from official DeepSeek API docs.',
        '# Edit via scripts/generate_deepseek_model_catalog.py.',
        '# Sources:',
        f'# - {FIRST_CALL_URL}',
        f'# - {PRICING_URL}',
        f'# - {LIST_MODELS_URL}',
        f'# - {CHAT_API_URL}',
        f'# - {FIM_API_URL}',
        f'# - {THINKING_URL}',
        f'# - {TOOL_CALLS_URL}',
        f'# - {PREFIX_URL}',
        f'# - {FIM_GUIDE_URL}',
        f'# - {JSON_URL}',
        f'# - {KV_CACHE_URL}',
        f'# - {UPDATES_URL}',
        f'# Generated at: {now}',
        '',
        '[provider]',
        'id = "deepseek"',
        'display_name = "DeepSeek API"',
        f'base_url = {toml_quote(DEFAULT_BASE_URL)}',
        'protocol = "openai"',
        f'source_url = {toml_quote(FIRST_CALL_URL)}',
        '',
        '[provider.auth]',
        'type = "api_key_env"',
        'keys = ["DEEPSEEK_API_KEY"]',
        '',
    ]

    for model_id, data in models.items():
        lines.append(f'[models.{toml_quote(model_id)}]')
        write_key_value(lines, 'source_url', data['source_url'])
        if len(data['source_urls']) > 1:
            write_key_value(lines, 'source_urls', data['source_urls'])
        write_key_value(lines, 'display_name', data['display_name'])
        write_key_value(lines, 'status', data['status'])
        write_key_value(lines, 'vendor', data['vendor'])
        write_key_value(lines, 'api_surfaces', data['api_surfaces'])
        write_key_value(lines, 'categories', data['categories'])
        write_key_value(lines, 'summary', data['summary'])
        write_key_value(lines, 'model_version', data['model_version'])
        write_key_value(lines, 'context_window_tokens', data['context_window_tokens'])
        write_key_value(lines, 'default_max_output_tokens', data['default_max_output_tokens'])
        write_key_value(lines, 'max_output_tokens', data['max_output_tokens'])
        write_key_value(lines, 'input_modalities', data['input_modalities'])
        write_key_value(lines, 'output_modalities', data['output_modalities'])
        write_key_value(lines, 'supported_features', data['supported_features'])
        write_key_value(lines, 'beta_features', data['beta_features'])
        if 'not_supported_features' in data:
            write_key_value(lines, 'not_supported_features', data['not_supported_features'])
        write_key_value(lines, 'latest_upgrade_date', data['latest_upgrade_date'])
        write_key_value(lines, 'pricing_input_cache_hit_per_million', data['pricing_input_cache_hit_per_million'])
        write_key_value(lines, 'pricing_input_cache_miss_per_million', data['pricing_input_cache_miss_per_million'])
        write_key_value(lines, 'pricing_output_per_million', data['pricing_output_per_million'])
        lines.append('')
        record_path = f'models.{toml_quote(model_id)}.records'
        for record in data['records']:
            write_record(lines, record_path, record)
    return '\n'.join(lines).rstrip() + '\n'


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description='Generate DeepSeek provider model catalog')
    parser.add_argument('--output', type=Path, default=DEFAULT_OUTPUT, help='Output TOML path')
    args = parser.parse_args(argv)

    models = build_catalog()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(render_toml(models), encoding='utf-8')
    write_json_sidecar(args.output)
    print(f'wrote {len(models)} models to {args.output}')
    return 0


if __name__ == '__main__':
    sys.exit(main())
