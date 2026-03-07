#!/usr/bin/env python3
from __future__ import annotations

import argparse
import datetime as dt
import re
import sys
from collections import OrderedDict
from pathlib import Path

import requests
from bs4 import BeautifulSoup

from provider_model_catalog_json import write_json_sidecar

PRODUCT_OVERVIEW_URL = 'https://cloud.tencent.com/document/product/1729/104753'
API_OVERVIEW_URL = 'https://cloud.tencent.com/document/product/1729/101848'
CHAT_COMPLETIONS_URL = 'https://cloud.tencent.com/document/api/1729/105701'
EMBEDDING_URL = 'https://cloud.tencent.com/document/api/1729/102832'
IMAGE_QUESTION_URL = 'https://cloud.tencent.com/document/api/1729/118394'
GROUP_CHAT_URL = 'https://cloud.tencent.com/document/api/1729/116923'
TRANSLATION_URL = 'https://cloud.tencent.com/document/api/1729/113395'
RUN_THREAD_URL = 'https://cloud.tencent.com/document/api/1729/112207'
IMAGE_JOB_URL = 'https://cloud.tencent.com/document/api/1729/105969'
IMAGE_CHAT_JOB_URL = 'https://cloud.tencent.com/document/api/1729/110428'
TEXT_TO_IMAGE_LITE_URL = 'https://cloud.tencent.com/document/api/1729/108738'
OPENAI_COMPAT_URL = 'https://cloud.tencent.com/document/product/1729/111007'
ANTHROPIC_COMPAT_URL = 'https://cloud.tencent.com/document/product/1729/127293'

DEFAULT_OUTPUT = Path(__file__).resolve().parents[1] / 'catalog' / 'provider_models' / 'hunyuan.toml'
NATIVE_ENDPOINT = 'https://hunyuan.tencentcloudapi.com'
OPENAI_COMPAT_BASE_URL = 'https://api.hunyuan.cloud.tencent.com/v1'
ANTHROPIC_COMPAT_BASE_URL = 'https://api.hunyuan.cloud.tencent.com/anthropic'

REALTIME_MODEL_IDS = [
    'hunyuan-2.0-thinking-20251109',
    'hunyuan-2.0-instruct-20251111',
    'hunyuan-t1-latest',
    'hunyuan-a13b',
    'hunyuan-turbos-latest',
    'hunyuan-lite',
    'hunyuan-translation',
    'hunyuan-translation-lite',
    'hunyuan-large-role-latest',
    'hunyuan-vision-1.5-instruct',
    'hunyuan-t1-vision-20250916',
    'hunyuan-turbos-vision-video',
]

IMAGE_PRODUCT_ID_MAP = OrderedDict(
    [
        ('混元生图', 'hunyuan-image'),
        ('混元生图（多轮对话）', 'hunyuan-image-chat'),
        ('文生图轻量版', 'hunyuan-image-lite'),
    ]
)

THREAD_ONLY_MODELS = [
    'hunyuan-standard',
    'hunyuan-standard-256K',
    'hunyuan-pro',
    'hunyuan-code',
    'hunyuan-role',
    'hunyuan-turbo',
]

OPENAI_COMPAT_MODELS = [
    'hunyuan-turbos-latest',
    'hunyuan-functioncall',
    'hunyuan-vision',
    'hunyuan-embedding',
]

OPENAI_COMPAT_ADDITIONAL_ALIASES = ['hunyuan-turbos', 'hunyuan-lite']
ANTHROPIC_COMPAT_MODELS = ['hunyuan-2.0-thinking-20251109', 'hunyuan-2.0-instruct-20251111']

CURRENT_CATEGORY_MAP = {
    '通用文生文': ['文本模型'],
    '翻译': ['翻译模型'],
    '角色扮演': ['角色扮演模型'],
    '混元图生文': ['视觉理解模型'],
    '混元视频生文': ['视频理解模型'],
    '混元生图': ['图片生成模型'],
}


def fetch_html(url: str, timeout: float = 30.0) -> str:
    response = requests.get(
        url,
        timeout=timeout,
        headers={
            'User-Agent': 'ditto-llm/hunyuan-model-catalog-generator',
            'Accept': 'text/html,application/xhtml+xml',
        },
    )
    response.raise_for_status()
    return response.text


def normalize_text(value: str) -> str:
    return ' '.join(value.replace('\xa0', ' ').replace('\ufeff', ' ').split())


def normalize_display_label(value: str) -> str:
    return normalize_text(value).replace(' （', '（').replace('( ', '(').replace(' )', ')')


MODEL_CELL_RE = re.compile(r'^(?P<display>.+?)（(?P<model>[^）]+)）$')
TOKEN_LIMIT_RE = re.compile(r'最大输入\s*(\d+)k\s*最大输出\s*(\d+)k', re.I)
MODEL_ENUM_RE = re.compile(r'hunyuan[-a-z0-9.]+', re.I)


def table_rows(html: str) -> list[list[list[str]]]:
    soup = BeautifulSoup(html, 'html.parser')
    tables: list[list[list[str]]] = []
    for table in soup.find_all('table'):
        rows: list[list[str]] = []
        for tr in table.find_all('tr'):
            cells = [normalize_text(cell.get_text(' ', strip=True)) for cell in tr.find_all(['th', 'td'])]
            if cells:
                rows.append(cells)
        if rows:
            tables.append(rows)
    return tables


def page_text(html: str) -> str:
    return normalize_text(BeautifulSoup(html, 'html.parser').get_text(' ', strip=True))


def extract_model_ids_from_text(text: str) -> list[str]:
    seen_lower: set[str] = set()
    results: list[str] = []
    for match in MODEL_ENUM_RE.findall(text):
        lowered = match.lower()
        if lowered == 'hunyuan.tencentcloudapi.com' or lowered == 'hunyuan.cloud.tencent.com':
            continue
        if lowered not in seen_lower:
            seen_lower.add(lowered)
            results.append(match)
    return results


def parse_model_cell(cell: str) -> tuple[str, str | None]:
    match = MODEL_CELL_RE.match(cell)
    if match:
        return normalize_text(match.group('display')), normalize_text(match.group('model'))
    value = normalize_text(cell)
    return value, value if value.startswith('hunyuan-') else None


def parse_token_limits(io_text: str) -> tuple[int | None, int | None]:
    match = TOKEN_LIMIT_RE.search(io_text)
    if not match:
        return None, None
    return int(match.group(1)) * 1000, int(match.group(2)) * 1000


class CatalogBuilder:
    def __init__(self) -> None:
        self.models: OrderedDict[str, OrderedDict[str, object]] = OrderedDict()

    def get(self, model_id: str, display_name: str | None = None) -> OrderedDict[str, object]:
        if model_id not in self.models:
            self.models[model_id] = OrderedDict(
                source_url='',
                source_urls=[],
                display_name=display_name or model_id,
                status='active',
                vendor='tencent',
                api_surfaces=[],
                categories=[],
                aliases=[],
                records=[],
            )
        entry = self.models[model_id]
        if display_name:
            entry['display_name'] = display_name
        return entry

    @staticmethod
    def add_unique(values: list[str], *items: str) -> None:
        for item in items:
            if item and item not in values:
                values.append(item)

    def add_source(self, entry: OrderedDict[str, object], *urls: str) -> None:
        self.add_unique(entry['source_urls'], *urls)
        if not entry['source_url'] and urls:
            entry['source_url'] = urls[0]

    def add_api_surface(self, entry: OrderedDict[str, object], *surfaces: str) -> None:
        self.add_unique(entry['api_surfaces'], *surfaces)

    def add_category(self, entry: OrderedDict[str, object], *categories: str) -> None:
        self.add_unique(entry['categories'], *categories)

    def add_alias(self, entry: OrderedDict[str, object], *aliases: str) -> None:
        for alias in aliases:
            if alias and alias != entry['display_name'] and alias not in entry['aliases']:
                entry['aliases'].append(alias)

    def set_if_missing(self, entry: OrderedDict[str, object], key: str, value) -> None:
        if value is None:
            return
        if key not in entry or entry[key] in ('', [], None):
            entry[key] = value

    def add_record(self, entry: OrderedDict[str, object], record: OrderedDict[str, object]) -> None:
        entry['records'].append(record)

    def finalize(self) -> OrderedDict[str, OrderedDict[str, object]]:
        finalized: OrderedDict[str, OrderedDict[str, object]] = OrderedDict()
        for model_id in sorted(self.models):
            entry = self.models[model_id]
            if not entry['source_urls'] and entry['source_url']:
                entry['source_urls'] = [entry['source_url']]
            if entry['source_urls'] and not entry['source_url']:
                entry['source_url'] = entry['source_urls'][0]
            if not entry['aliases']:
                entry.pop('aliases', None)
            if 'context_window_tokens' in entry and entry['context_window_tokens'] is None:
                entry.pop('context_window_tokens', None)
            if 'max_output_tokens' in entry and entry['max_output_tokens'] is None:
                entry.pop('max_output_tokens', None)
            finalized[model_id] = entry
        return finalized


def extract_param_row(table_rows_: list[list[str]], param_name: str) -> list[str]:
    for row in table_rows_:
        if row and row[0] == param_name:
            return row
    raise RuntimeError(f'unable to find parameter row for {param_name!r}')


def build_catalog() -> OrderedDict[str, OrderedDict[str, object]]:
    overview_html = fetch_html(PRODUCT_OVERVIEW_URL)
    api_overview_html = fetch_html(API_OVERVIEW_URL)
    chat_html = fetch_html(CHAT_COMPLETIONS_URL)
    embedding_html = fetch_html(EMBEDDING_URL)
    image_question_html = fetch_html(IMAGE_QUESTION_URL)
    group_chat_html = fetch_html(GROUP_CHAT_URL)
    translation_html = fetch_html(TRANSLATION_URL)
    run_thread_html = fetch_html(RUN_THREAD_URL)
    image_job_html = fetch_html(IMAGE_JOB_URL)
    image_chat_job_html = fetch_html(IMAGE_CHAT_JOB_URL)
    text_to_image_lite_html = fetch_html(TEXT_TO_IMAGE_LITE_URL)
    openai_compat_html = fetch_html(OPENAI_COMPAT_URL)
    anthropic_compat_html = fetch_html(ANTHROPIC_COMPAT_URL)

    overview_tables = table_rows(overview_html)
    if len(overview_tables) < 3:
        raise RuntimeError('product overview page no longer exposes the expected model tables')
    text_rows, vision_rows, image_rows = overview_tables[:3]
    if text_rows[0][:3] != ['模型类型', '模型名称（API 调用名）', '版本更新时间']:
        raise RuntimeError('unexpected text model table header in product overview')
    if vision_rows[0][:3] != ['模型类型', '模型名称（API 调用名）', '版本更新时间']:
        raise RuntimeError('unexpected vision model table header in product overview')
    if image_rows[0][:3] != ['模型类型', '模型名称', '版本更新时间']:
        raise RuntimeError('unexpected image model table header in product overview')

    current_rows: OrderedDict[str, dict] = OrderedDict()
    current_image_rows: OrderedDict[str, dict] = OrderedDict()

    for rows, columns in ((text_rows[1:], text_rows[0]), (vision_rows[1:], vision_rows[0])):
        current_category = ''
        for row in rows:
            row_category = row[0]
            if row_category:
                current_category = row_category
            display_name, model_id = parse_model_cell(row[1])
            if not model_id:
                raise RuntimeError(f'expected explicit model id in overview row: {row!r}')
            current_rows[model_id] = {
                'category_label': current_category,
                'display_name': display_name,
                'model_id': model_id,
                'updated': row[2],
                'summary': row[3],
                'io': row[4],
                'columns': columns,
                'values': row,
            }

    current_category = '混元生图'
    for row in image_rows[1:]:
        row_category = row[0]
        if row_category:
            current_category = row_category
        display_name = normalize_display_label(row[1])
        current_image_rows[display_name] = {
            'category_label': current_category,
            'display_name': display_name,
            'updated': row[2],
            'summary': row[3],
            'io': row[4],
            'columns': image_rows[0],
            'values': row,
        }

    missing_realtime = [model_id for model_id in REALTIME_MODEL_IDS if model_id not in current_rows]
    if missing_realtime:
        raise RuntimeError(f'product overview missing expected realtime models: {missing_realtime!r}')
    missing_image_products = [name for name in IMAGE_PRODUCT_ID_MAP if name not in current_image_rows]
    if missing_image_products:
        raise RuntimeError(f'product overview missing expected image products: {missing_image_products!r}')

    chat_tables = table_rows(chat_html)
    if '本接口取值：ChatCompletions。' not in page_text(chat_html):
        raise RuntimeError('ChatCompletions API page missing action marker')
    embedding_text = page_text(embedding_html)
    if '向量维度为1024维' not in embedding_text:
        raise RuntimeError('embedding API page missing 1024-dimension note')
    image_question_model_row = extract_param_row(table_rows(image_question_html)[0], 'Model')
    group_chat_model_row = extract_param_row(table_rows(group_chat_html)[0], 'Model')
    translation_model_row = extract_param_row(table_rows(translation_html)[0], 'Model')
    run_thread_model_row = extract_param_row(table_rows(run_thread_html)[0], 'Model')

    image_question_models = extract_model_ids_from_text(image_question_model_row[3])
    group_chat_models = extract_model_ids_from_text(group_chat_model_row[3])
    translation_models = extract_model_ids_from_text(translation_model_row[3])
    run_thread_models = extract_model_ids_from_text(run_thread_model_row[3])
    openai_compat_models = extract_model_ids_from_text(page_text(openai_compat_html))
    anthropic_compat_models = extract_model_ids_from_text(page_text(anthropic_compat_html))

    if image_question_models != ['hunyuan-vision-image-question']:
        raise RuntimeError(f'unexpected ImageQuestion model enum: {image_question_models!r}')
    if group_chat_models != ['hunyuan-large-role-group']:
        raise RuntimeError(f'unexpected GroupChatCompletions model enum: {group_chat_models!r}')
    if translation_models != ['hunyuan-translation', 'hunyuan-translation-lite']:
        raise RuntimeError(f'unexpected ChatTranslations model enum: {translation_models!r}')
    if any(model_id not in run_thread_models for model_id in THREAD_ONLY_MODELS):
        raise RuntimeError(f'RunThread docs missing expected thread-only models: {THREAD_ONLY_MODELS!r}')
    if any(model_id not in openai_compat_models for model_id in OPENAI_COMPAT_MODELS):
        raise RuntimeError(f'OpenAI compatibility page missing expected models: {OPENAI_COMPAT_MODELS!r}')
    if any(model_id not in anthropic_compat_models for model_id in ANTHROPIC_COMPAT_MODELS):
        raise RuntimeError(f'Anthropic compatibility page missing expected models: {ANTHROPIC_COMPAT_MODELS!r}')

    builder = CatalogBuilder()

    def add_native_chat_record(entry: OrderedDict[str, object], section: str) -> None:
        builder.add_record(
            entry,
            OrderedDict(
                table_kind='api_reference',
                source_url=CHAT_COMPLETIONS_URL,
                source_page='chat_completions',
                section=section,
                api_surface='chat.completion',
                endpoint=NATIVE_ENDPOINT,
                action='ChatCompletions',
                version='2023-09-01',
                notes='Tencent Cloud native chat endpoint. The Model parameter references the current product overview model list.',
            ),
        )

    for model_id in REALTIME_MODEL_IDS:
        row = current_rows[model_id]
        context_window_tokens, max_output_tokens = parse_token_limits(row['io'])
        entry = builder.get(model_id, row['display_name'])
        builder.add_source(entry, PRODUCT_OVERVIEW_URL)
        builder.add_category(entry, *CURRENT_CATEGORY_MAP[row['category_label']])
        builder.set_if_missing(entry, 'summary', row['summary'])
        builder.set_if_missing(entry, 'latest_update', row['updated'])
        builder.set_if_missing(entry, 'input_output', row['io'])
        builder.set_if_missing(entry, 'context_window_tokens', context_window_tokens)
        builder.set_if_missing(entry, 'max_output_tokens', max_output_tokens)
        if row['category_label'] == '翻译':
            builder.add_api_surface(entry, 'chat.translation')
            builder.add_source(entry, TRANSLATION_URL)
            builder.add_record(
                entry,
                OrderedDict(
                    table_kind='source_table',
                    source_url=PRODUCT_OVERVIEW_URL,
                    source_page='product_overview',
                    section=f'{row["category_label"]} / 当前模型',
                    columns=row['columns'],
                    values=row['values'],
                ),
            )
            builder.add_record(
                entry,
                OrderedDict(
                    table_kind='api_reference',
                    source_url=TRANSLATION_URL,
                    source_page='chat_translations',
                    section='输入参数 / Model',
                    api_surface='chat.translation',
                    endpoint=NATIVE_ENDPOINT,
                    action='ChatTranslations',
                    version='2023-09-01',
                    notes='The ChatTranslations API explicitly enumerates hunyuan-translation and hunyuan-translation-lite.',
                ),
            )
            continue
        builder.add_api_surface(entry, 'chat.completion')
        builder.add_source(entry, CHAT_COMPLETIONS_URL)
        builder.add_record(
            entry,
            OrderedDict(
                table_kind='source_table',
                source_url=PRODUCT_OVERVIEW_URL,
                source_page='product_overview',
                section=f'{row["category_label"]} / 当前模型',
                columns=row['columns'],
                values=row['values'],
            ),
        )
        add_native_chat_record(entry, '对话 / 输入参数 / Model')
        if model_id in ANTHROPIC_COMPAT_MODELS:
            builder.add_api_surface(entry, 'anthropic.messages')
            builder.add_source(entry, ANTHROPIC_COMPAT_URL)
            builder.add_record(
                entry,
                OrderedDict(
                    table_kind='compatibility',
                    source_url=ANTHROPIC_COMPAT_URL,
                    source_page='anthropic_compatibility',
                    section='Anthropic 兼容接口 / /v1/messages',
                    api_surface='anthropic.messages',
                    endpoint=f'{ANTHROPIC_COMPAT_BASE_URL}/v1/messages',
                    notes='Official Anthropic-compatible examples enumerate this model with the Hunyuan compatibility endpoint.',
                ),
            )
        if model_id == 'hunyuan-turbos-latest':
            builder.add_source(entry, OPENAI_COMPAT_URL)
            builder.add_record(
                entry,
                OrderedDict(
                    table_kind='compatibility',
                    source_url=OPENAI_COMPAT_URL,
                    source_page='openai_compatibility',
                    section='OpenAI 兼容接口 / chat.completions 示例',
                    api_surface='chat.completion',
                    endpoint=f'{OPENAI_COMPAT_BASE_URL}/chat/completions',
                    notes='Official OpenAI-compatible examples use hunyuan-turbos-latest with the /v1/chat/completions surface.',
                ),
            )
            builder.add_alias(entry, 'hunyuan-turbos')

    for display_name, synthetic_id in IMAGE_PRODUCT_ID_MAP.items():
        row = current_image_rows[display_name]
        entry = builder.get(synthetic_id, display_name)
        builder.add_source(entry, PRODUCT_OVERVIEW_URL)
        builder.add_category(entry, *CURRENT_CATEGORY_MAP[row['category_label']])
        builder.set_if_missing(entry, 'summary', row['summary'])
        builder.set_if_missing(entry, 'latest_update', row['updated'])
        builder.set_if_missing(entry, 'input_output', row['io'])
        builder.add_record(
            entry,
            OrderedDict(
                table_kind='source_table',
                source_url=PRODUCT_OVERVIEW_URL,
                source_page='product_overview',
                section='混元生图 / 当前能力',
                columns=row['columns'],
                values=row['values'],
            ),
        )
        if synthetic_id == 'hunyuan-image':
            builder.add_api_surface(entry, 'image.generation.async')
            builder.add_source(entry, IMAGE_JOB_URL)
            builder.add_record(
                entry,
                OrderedDict(
                    table_kind='api_reference',
                    source_url=IMAGE_JOB_URL,
                    source_page='submit_hunyuan_image_job',
                    section='提交混元生图任务',
                    api_surface='image.generation.async',
                    endpoint=NATIVE_ENDPOINT,
                    action='SubmitHunyuanImageJob',
                    version='2023-09-01',
                    notes='The native image generation API is job-based and does not expose a separate model parameter.',
                ),
            )
        elif synthetic_id == 'hunyuan-image-chat':
            builder.add_api_surface(entry, 'image.generation.async')
            builder.add_source(entry, IMAGE_CHAT_JOB_URL)
            builder.add_record(
                entry,
                OrderedDict(
                    table_kind='api_reference',
                    source_url=IMAGE_CHAT_JOB_URL,
                    source_page='submit_hunyuan_image_chat_job',
                    section='提交混元生图（多轮对话）任务',
                    api_surface='image.generation.async',
                    endpoint=NATIVE_ENDPOINT,
                    action='SubmitHunyuanImageChatJob',
                    version='2023-09-01',
                    notes='The multi-turn image API is job-based and uses ChatId to continue editing the generated image.',
                ),
            )
        else:
            builder.add_api_surface(entry, 'image.generation')
            builder.add_source(entry, TEXT_TO_IMAGE_LITE_URL)
            builder.add_record(
                entry,
                OrderedDict(
                    table_kind='api_reference',
                    source_url=TEXT_TO_IMAGE_LITE_URL,
                    source_page='text_to_image_lite',
                    section='文生图轻量版',
                    api_surface='image.generation',
                    endpoint=NATIVE_ENDPOINT,
                    action='TextToImageLite',
                    version='2023-09-01',
                    notes='The lightweight text-to-image API is synchronous and returns the generated image directly.',
                ),
            )

    image_question_entry = builder.get('hunyuan-vision-image-question', 'hunyuan-vision-image-question')
    builder.add_source(image_question_entry, IMAGE_QUESTION_URL)
    builder.add_api_surface(image_question_entry, 'image.question')
    builder.add_category(image_question_entry, '图像理解模型', '拍照解题模型')
    builder.set_if_missing(image_question_entry, 'summary', '拍照解题专用图像理解模型，来自腾讯云 ImageQuestion 接口的官方枚举值。')
    builder.add_record(
        image_question_entry,
        OrderedDict(
            table_kind='api_enum',
            source_url=IMAGE_QUESTION_URL,
            source_page='image_question',
            section='输入参数 / Model',
            columns=['Model'],
            values=['hunyuan-vision-image-question'],
        ),
    )
    builder.add_record(
        image_question_entry,
        OrderedDict(
            table_kind='api_reference',
            source_url=IMAGE_QUESTION_URL,
            source_page='image_question',
            section='拍照解题',
            api_surface='image.question',
            endpoint=NATIVE_ENDPOINT,
            action='ImageQuestion',
            version='2023-09-01',
            notes='The native ImageQuestion API explicitly enumerates hunyuan-vision-image-question.',
        ),
    )

    group_chat_entry = builder.get('hunyuan-large-role-group', 'hunyuan-large-role-group')
    builder.add_source(group_chat_entry, GROUP_CHAT_URL)
    builder.add_api_surface(group_chat_entry, 'group.chat.completion')
    builder.add_category(group_chat_entry, '角色扮演模型', '群聊模型')
    builder.set_if_missing(group_chat_entry, 'summary', '群聊角色扮演模型，来自腾讯云 GroupChatCompletions 接口的官方枚举值。')
    builder.add_record(
        group_chat_entry,
        OrderedDict(
            table_kind='api_enum',
            source_url=GROUP_CHAT_URL,
            source_page='group_chat_completions',
            section='输入参数 / Model',
            columns=['Model'],
            values=['hunyuan-large-role-group'],
        ),
    )
    builder.add_record(
        group_chat_entry,
        OrderedDict(
            table_kind='api_reference',
            source_url=GROUP_CHAT_URL,
            source_page='group_chat_completions',
            section='群聊',
            api_surface='group.chat.completion',
            endpoint=NATIVE_ENDPOINT,
            action='GroupChatCompletions',
            version='2023-09-01',
            notes='The native GroupChatCompletions API explicitly enumerates hunyuan-large-role-group.',
        ),
    )

    embedding_entry = builder.get('hunyuan-embedding', 'hunyuan-embedding')
    builder.add_source(embedding_entry, EMBEDDING_URL, OPENAI_COMPAT_URL)
    builder.add_api_surface(embedding_entry, 'embedding')
    builder.add_category(embedding_entry, '嵌入模型')
    builder.set_if_missing(embedding_entry, 'summary', '混元文本向量化能力对应的模型标识。原生 GetEmbedding 接口不暴露 model 参数，OpenAI 兼容 /v1/embeddings 固定使用 hunyuan-embedding。')
    builder.set_if_missing(embedding_entry, 'embedding_dimensions', 1024)
    builder.add_record(
        embedding_entry,
        OrderedDict(
            table_kind='api_reference',
            source_url=EMBEDDING_URL,
            source_page='get_embedding',
            section='向量化',
            api_surface='embedding',
            endpoint=NATIVE_ENDPOINT,
            action='GetEmbedding',
            version='2023-09-01',
            notes='The native GetEmbedding API is fixed at 1024 dimensions and does not expose a model parameter.',
        ),
    )
    builder.add_record(
        embedding_entry,
        OrderedDict(
            table_kind='compatibility',
            source_url=OPENAI_COMPAT_URL,
            source_page='openai_compatibility',
            section='OpenAI 兼容接口 / /v1/embeddings',
            api_surface='embedding',
            endpoint=f'{OPENAI_COMPAT_BASE_URL}/embeddings',
            notes='The OpenAI-compatible embedding surface fixes model=hunyuan-embedding and dimensions=1024.',
        ),
    )

    function_call_entry = builder.get('hunyuan-functioncall', 'hunyuan-functioncall')
    builder.add_source(function_call_entry, OPENAI_COMPAT_URL, RUN_THREAD_URL)
    builder.add_api_surface(function_call_entry, 'chat.completion', 'thread.run')
    builder.add_category(function_call_entry, '文本模型', '工具调用模型', 'OpenAI兼容模型')
    builder.set_if_missing(function_call_entry, 'summary', '混元 OpenAI 兼容接口官方 Function Calling 示例使用的模型标识，同时仍出现在 RunThread 文档枚举中。')
    builder.add_record(
        function_call_entry,
        OrderedDict(
            table_kind='compatibility',
            source_url=OPENAI_COMPAT_URL,
            source_page='openai_compatibility',
            section='OpenAI 兼容接口 / Function Calling',
            api_surface='chat.completion',
            endpoint=f'{OPENAI_COMPAT_BASE_URL}/chat/completions',
            notes='Official OpenAI-compatible function-calling examples use hunyuan-functioncall.',
        ),
    )
    builder.add_record(
        function_call_entry,
        OrderedDict(
            table_kind='api_enum',
            source_url=RUN_THREAD_URL,
            source_page='run_thread',
            section='输入参数 / Model',
            columns=['Model'],
            values=['hunyuan-functioncall'],
        ),
    )
    builder.add_record(
        function_call_entry,
        OrderedDict(
            table_kind='api_reference',
            source_url=RUN_THREAD_URL,
            source_page='run_thread',
            section='执行会话',
            api_surface='thread.run',
            endpoint=NATIVE_ENDPOINT,
            action='RunThread',
            version='2023-09-01',
            notes='The RunThread API still enumerates hunyuan-functioncall in the current docs.',
        ),
    )

    compat_vision_entry = builder.get('hunyuan-vision', 'hunyuan-vision')
    builder.add_source(compat_vision_entry, OPENAI_COMPAT_URL, RUN_THREAD_URL)
    builder.add_api_surface(compat_vision_entry, 'chat.completion', 'thread.run')
    builder.add_category(compat_vision_entry, '视觉理解模型', 'OpenAI兼容模型')
    builder.set_if_missing(compat_vision_entry, 'summary', '混元 OpenAI 兼容接口官方图生文示例使用的视觉模型标识，同时仍出现在 RunThread 文档枚举中。')
    builder.add_record(
        compat_vision_entry,
        OrderedDict(
            table_kind='compatibility',
            source_url=OPENAI_COMPAT_URL,
            source_page='openai_compatibility',
            section='OpenAI 兼容接口 / 图生文',
            api_surface='chat.completion',
            endpoint=f'{OPENAI_COMPAT_BASE_URL}/chat/completions',
            notes='Official OpenAI-compatible multimodal examples use hunyuan-vision.',
        ),
    )
    builder.add_record(
        compat_vision_entry,
        OrderedDict(
            table_kind='api_enum',
            source_url=RUN_THREAD_URL,
            source_page='run_thread',
            section='输入参数 / Model',
            columns=['Model'],
            values=['hunyuan-vision'],
        ),
    )
    builder.add_record(
        compat_vision_entry,
        OrderedDict(
            table_kind='api_reference',
            source_url=RUN_THREAD_URL,
            source_page='run_thread',
            section='执行会话',
            api_surface='thread.run',
            endpoint=NATIVE_ENDPOINT,
            action='RunThread',
            version='2023-09-01',
            notes='The RunThread API still enumerates hunyuan-vision in the current docs.',
        ),
    )

    for model_id in THREAD_ONLY_MODELS:
        entry = builder.get(model_id, model_id)
        builder.add_source(entry, RUN_THREAD_URL)
        builder.add_api_surface(entry, 'thread.run')
        builder.add_category(entry, '历史线程模型')
        if model_id == 'hunyuan-code':
            builder.add_category(entry, '代码模型')
        elif model_id == 'hunyuan-role':
            builder.add_category(entry, '角色扮演模型')
        else:
            builder.add_category(entry, '文本模型')
        entry['status'] = 'historical'
        builder.set_if_missing(entry, 'summary', '该模型 ID 未出现在当前产品概述模型总表中，但仍由 RunThread 文档作为官方枚举值列出。')
        builder.add_record(
            entry,
            OrderedDict(
                table_kind='api_enum',
                source_url=RUN_THREAD_URL,
                source_page='run_thread',
                section='输入参数 / Model',
                columns=['Model'],
                values=[model_id],
            ),
        )
        builder.add_record(
            entry,
            OrderedDict(
                table_kind='api_reference',
                source_url=RUN_THREAD_URL,
                source_page='run_thread',
                section='执行会话',
                api_surface='thread.run',
                endpoint=NATIVE_ENDPOINT,
                action='RunThread',
                version='2023-09-01',
                notes='Current Tencent Cloud RunThread docs still enumerate this legacy model id.',
            ),
        )

    for alias in OPENAI_COMPAT_ADDITIONAL_ALIASES:
        if alias == 'hunyuan-lite':
            entry = builder.get('hunyuan-lite')
            builder.add_source(entry, OPENAI_COMPAT_URL)
            builder.add_category(entry, 'OpenAI兼容模型')
            builder.add_record(
                entry,
                OrderedDict(
                    table_kind='compatibility_note',
                    source_url=OPENAI_COMPAT_URL,
                    source_page='openai_compatibility',
                    section='OpenAI 兼容接口 / 混元自定义参数',
                    api_surface='chat.completion',
                    endpoint=f'{OPENAI_COMPAT_BASE_URL}/chat/completions',
                    notes='The OpenAI compatibility docs explicitly note that hunyuan-lite does not support enhancement-related custom parameters.',
                ),
            )
            continue
        entry = builder.get(alias, alias)
        builder.add_source(entry, OPENAI_COMPAT_URL)
        builder.add_api_surface(entry, 'chat.completion')
        builder.add_category(entry, '文本模型', 'OpenAI兼容模型')
        builder.set_if_missing(entry, 'summary', '该模型 ID 出现在混元 OpenAI 兼容接口参数说明中。文档同时提供了 hunyuan-turbos-latest 的直接示例调用。')
        builder.add_record(
            entry,
            OrderedDict(
                table_kind='compatibility_note',
                source_url=OPENAI_COMPAT_URL,
                source_page='openai_compatibility',
                section='OpenAI 兼容接口 / 参数说明',
                api_surface='chat.completion',
                endpoint=f'{OPENAI_COMPAT_BASE_URL}/chat/completions',
                notes='The OpenAI compatibility docs reference this model id in tool-choice parameter notes.',
            ),
        )

    api_overview_tables = table_rows(api_overview_html)
    expected_actions = {
        'ChatCompletions',
        'ImageQuestion',
        'GroupChatCompletions',
        'GetEmbedding',
        'ChatTranslations',
        'CreateThread',
        'RunThread',
        'SubmitHunyuanImageJob',
        'SubmitHunyuanImageChatJob',
        'TextToImageLite',
    }
    action_values = {row[0] for table in api_overview_tables for row in table[1:] if row}
    missing_actions = sorted(expected_actions - action_values)
    if missing_actions:
        raise RuntimeError(f'API overview missing expected actions: {missing_actions!r}')

    return builder.finalize()


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


def render_toml(models: OrderedDict[str, OrderedDict[str, object]]) -> str:
    now = dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat().replace('+00:00', 'Z')
    lines = [
        '# Generated from official Tencent Hunyuan / Tencent Cloud docs.',
        '# Edit via scripts/generate_hunyuan_model_catalog.py.',
        '# Sources:',
        f'# - {PRODUCT_OVERVIEW_URL}',
        f'# - {API_OVERVIEW_URL}',
        f'# - {CHAT_COMPLETIONS_URL}',
        f'# - {EMBEDDING_URL}',
        f'# - {IMAGE_QUESTION_URL}',
        f'# - {GROUP_CHAT_URL}',
        f'# - {TRANSLATION_URL}',
        f'# - {RUN_THREAD_URL}',
        f'# - {IMAGE_JOB_URL}',
        f'# - {IMAGE_CHAT_JOB_URL}',
        f'# - {TEXT_TO_IMAGE_LITE_URL}',
        f'# - {OPENAI_COMPAT_URL}',
        f'# - {ANTHROPIC_COMPAT_URL}',
        f'# Generated at: {now}',
        '',
        '[provider]',
        'id = "hunyuan"',
        'display_name = "Tencent Hunyuan"',
        f'base_url = {toml_quote(OPENAI_COMPAT_BASE_URL)}',
        'protocol = "hunyuan"',
        f'source_url = {toml_quote(PRODUCT_OVERVIEW_URL)}',
        '',
        '[provider.auth]',
        'type = "api_key_env"',
        'keys = ["HUNYUAN_API_KEY"]',
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
        if 'aliases' in data:
            write_key_value(lines, 'aliases', data['aliases'])
        if 'summary' in data:
            write_key_value(lines, 'summary', data['summary'])
        if 'latest_update' in data:
            write_key_value(lines, 'latest_update', data['latest_update'])
        if 'input_output' in data:
            write_key_value(lines, 'input_output', data['input_output'])
        if 'context_window_tokens' in data:
            write_key_value(lines, 'context_window_tokens', data['context_window_tokens'])
        if 'max_output_tokens' in data:
            write_key_value(lines, 'max_output_tokens', data['max_output_tokens'])
        if 'embedding_dimensions' in data:
            write_key_value(lines, 'embedding_dimensions', data['embedding_dimensions'])
        lines.append('')
        record_path = f'models.{toml_quote(model_id)}.records'
        for record in data['records']:
            write_record(lines, record_path, record)
    return '\n'.join(lines).rstrip() + '\n'


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description='Generate Tencent Hunyuan provider model catalog')
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
