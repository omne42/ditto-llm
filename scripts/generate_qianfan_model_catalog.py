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

MODEL_LIST_URL = 'https://cloud.baidu.com/doc/qianfan/s/rmh4stp0j'
FEATURE_PAGE_URL = 'https://cloud.baidu.com/product/wenxinworkshop.html'
QUICKSTART_URL = 'https://cloud.baidu.com/doc/qianfan/s/rmh4stn9m'
TEXT_DOC_URL = 'https://cloud.baidu.com/doc/qianfan-docs/s/Mm8r1mejk'
TEXT_API_URL = 'https://cloud.baidu.com/doc/qianfan-api/s/3m7of64lb'
VISION_DOC_URL = 'https://cloud.baidu.com/doc/qianfan-docs/s/fm8r1ndsm'
VISION_API_URL = 'https://cloud.baidu.com/doc/qianfan-api/s/rm7u7qdiq'
REASONING_DOC_URL = 'https://cloud.baidu.com/doc/qianfan-docs/s/Wm95lyynv'
IMAGE_DOC_URL = 'https://cloud.baidu.com/doc/qianfan-docs/s/bm8wv3h6f'
IMAGE_GEN_API_URL = 'https://cloud.baidu.com/doc/qianfan-api/s/8m7u6un8a'
IMAGE_EDIT_API_URL = 'https://cloud.baidu.com/doc/qianfan-api/s/Rm9m76ekf'
EMBEDDING_DOC_URL = 'https://cloud.baidu.com/doc/qianfan-docs/s/Um8r1tpwy'
EMBEDDING_API_URL = 'https://cloud.baidu.com/doc/qianfan-api/s/Fm7u3ropn'
RERANK_DOC_URL = 'https://cloud.baidu.com/doc/qianfan-docs/s/em95lyyjw'
RERANK_API_URL = 'https://cloud.baidu.com/doc/qianfan-api/s/2m7u4zt74'
RETIREMENT_URL = 'https://cloud.baidu.com/doc/qianfan/s/zmh4stou3'
DEFAULT_OUTPUT = Path(__file__).resolve().parents[1] / 'catalog' / 'provider_models' / 'qianfan.toml'

BASE_URL = 'https://qianfan.baidubce.com/v2'
CURRENT_DATE = dt.date(2026, 3, 7)

TABLE_SPECS = [
    {
        'index': 1,
        'section': '文本生成',
        'subsection': 'ERNIE系列-旗舰模型',
        'api_surfaces': ['chat.completion'],
        'categories': ['文本生成', 'ERNIE系列-旗舰模型'],
        'api_reference': (TEXT_API_URL, 'text_generation', '文本生成', f'{BASE_URL}/chat/completions'),
    },
    {
        'index': 2,
        'section': '文本生成',
        'subsection': 'ERNIE系列-主力模型',
        'api_surfaces': ['chat.completion'],
        'categories': ['文本生成', 'ERNIE系列-主力模型'],
        'api_reference': (TEXT_API_URL, 'text_generation', '文本生成', f'{BASE_URL}/chat/completions'),
    },
    {
        'index': 3,
        'section': '文本生成',
        'subsection': 'ERNIE系列-垂直场景模型',
        'api_surfaces': ['chat.completion'],
        'categories': ['文本生成', 'ERNIE系列-垂直场景模型'],
        'api_reference': (TEXT_API_URL, 'text_generation', '文本生成', f'{BASE_URL}/chat/completions'),
    },
    {
        'index': 4,
        'section': '文本生成',
        'subsection': 'ERNIE系列-开源模型',
        'api_surfaces': ['chat.completion'],
        'categories': ['文本生成', 'ERNIE系列-开源模型'],
        'api_reference': (TEXT_API_URL, 'text_generation', '文本生成', f'{BASE_URL}/chat/completions'),
    },
    {
        'index': 5,
        'section': '文本生成',
        'subsection': 'QianFan系列',
        'api_surfaces': ['chat.completion'],
        'categories': ['文本生成', 'QianFan系列'],
        'api_reference': (TEXT_API_URL, 'text_generation', '文本生成', f'{BASE_URL}/chat/completions'),
    },
    {
        'index': 6,
        'section': '文本生成',
        'subsection': 'DeepSeek系列',
        'api_surfaces': ['chat.completion'],
        'categories': ['文本生成', 'DeepSeek系列'],
        'api_reference': (TEXT_API_URL, 'text_generation', '文本生成', f'{BASE_URL}/chat/completions'),
    },
    {
        'index': 7,
        'section': '文本生成',
        'subsection': '其他',
        'api_surfaces': ['chat.completion'],
        'categories': ['文本生成', '其他'],
        'api_reference': (TEXT_API_URL, 'text_generation', '文本生成', f'{BASE_URL}/chat/completions'),
    },
    {
        'index': 8,
        'section': '视觉理解',
        'subsection': 'ERNIE系列',
        'api_surfaces': ['chat.completion'],
        'categories': ['视觉理解', 'ERNIE系列'],
        'api_reference': (VISION_API_URL, 'vision_understanding', '视觉理解', f'{BASE_URL}/chat/completions'),
    },
    {
        'index': 9,
        'section': '视觉理解',
        'subsection': 'QianFan系列',
        'api_surfaces': ['chat.completion'],
        'categories': ['视觉理解', 'QianFan系列'],
        'api_reference': (VISION_API_URL, 'vision_understanding', '视觉理解', f'{BASE_URL}/chat/completions'),
    },
    {
        'index': 10,
        'section': '视觉理解',
        'subsection': 'InternVL系列',
        'api_surfaces': ['chat.completion'],
        'categories': ['视觉理解', 'InternVL系列'],
        'api_reference': (VISION_API_URL, 'vision_understanding', '视觉理解', f'{BASE_URL}/chat/completions'),
    },
    {
        'index': 11,
        'section': '视觉理解',
        'subsection': 'QwenVL系列',
        'api_surfaces': ['chat.completion'],
        'categories': ['视觉理解', 'QwenVL系列'],
        'api_reference': (VISION_API_URL, 'vision_understanding', '视觉理解', f'{BASE_URL}/chat/completions'),
    },
    {
        'index': 12,
        'section': '深度思考',
        'subsection': 'ERNIE系列',
        'api_surfaces': ['chat.completion'],
        'categories': ['深度思考', 'ERNIE系列'],
        'api_reference': (TEXT_API_URL, 'reasoning_text_generation', '深度思考 / 文本生成', f'{BASE_URL}/chat/completions'),
    },
    {
        'index': 13,
        'section': '深度思考',
        'subsection': 'DeepSeek满血版',
        'api_surfaces': ['chat.completion'],
        'categories': ['深度思考', 'DeepSeek满血版'],
        'api_reference': (TEXT_API_URL, 'reasoning_text_generation', '深度思考 / 文本生成', f'{BASE_URL}/chat/completions'),
    },
    {
        'index': 14,
        'section': '深度思考',
        'subsection': 'DeepSeek蒸馏版',
        'api_surfaces': ['chat.completion'],
        'categories': ['深度思考', 'DeepSeek蒸馏版'],
        'api_reference': (TEXT_API_URL, 'reasoning_text_generation', '深度思考 / 文本生成', f'{BASE_URL}/chat/completions'),
    },
    {
        'index': 15,
        'section': '深度思考',
        'subsection': 'Qwen系列',
        'api_surfaces': ['chat.completion'],
        'categories': ['深度思考', 'Qwen系列'],
        'api_reference': (TEXT_API_URL, 'reasoning_text_generation', '深度思考 / 文本生成', f'{BASE_URL}/chat/completions'),
    },
    {
        'index': 16,
        'section': '深度思考',
        'subsection': '其他',
        'api_surfaces': ['chat.completion'],
        'categories': ['深度思考', '其他'],
        'api_reference': (TEXT_API_URL, 'reasoning_text_generation', '深度思考 / 文本生成', f'{BASE_URL}/chat/completions'),
    },
    {
        'index': 17,
        'section': 'OCR',
        'subsection': None,
        'api_surfaces': ['ocr'],
        'categories': ['OCR'],
        'api_reference': None,
    },
    {
        'index': 18,
        'section': '图像生成',
        'subsection': None,
        'api_surfaces': ['image.generation'],
        'categories': ['图像生成'],
        'api_reference': (IMAGE_GEN_API_URL, 'image_generation', '图片生成', f'{BASE_URL}/images/generations'),
    },
    {
        'index': 19,
        'section': '图像编辑',
        'subsection': None,
        'api_surfaces': ['image.edit'],
        'categories': ['图像编辑'],
        'api_reference': (IMAGE_EDIT_API_URL, 'image_edit', '图片编辑', f'{BASE_URL}/images/edits'),
    },
    {
        'index': 20,
        'section': '视频生成',
        'subsection': None,
        'api_surfaces': ['video.generation'],
        'categories': ['视频生成'],
        'api_reference': None,
    },
    {
        'index': 21,
        'section': '文本向量',
        'subsection': None,
        'api_surfaces': ['embedding'],
        'categories': ['文本向量'],
        'api_reference': (EMBEDDING_API_URL, 'embedding', '向量', f'{BASE_URL}/embeddings'),
    },
    {
        'index': 22,
        'section': '多模态向量',
        'subsection': None,
        'api_surfaces': ['embedding.multimodal'],
        'categories': ['多模态向量'],
        'api_reference': None,
    },
    {
        'index': 23,
        'section': '重排序',
        'subsection': None,
        'api_surfaces': ['rerank'],
        'categories': ['重排序'],
        'api_reference': (RERANK_API_URL, 'rerank', '重排序', f'{BASE_URL}/rerank'),
    },
]

FEATURED_MODEL_MAP = OrderedDict(
    [
        ('ERNIE-5.0', 'ernie-5.0'),
        ('ERNIE-X1.1', 'ernie-x1.1'),
        ('ERNIE-4.5-Turbo-128K', 'ernie-4.5-turbo-128k'),
        ('DeepSeek-R1', 'deepseek-r1'),
    ]
)


def fetch_html(url: str, timeout: float = 30.0) -> str:
    response = requests.get(
        url,
        timeout=timeout,
        headers={
            'User-Agent': 'ditto-llm/qianfan-model-catalog-generator',
            'Accept': 'text/html,application/xhtml+xml',
        },
    )
    response.raise_for_status()
    return response.text


def normalize_text(value: str) -> str:
    return ' '.join(value.replace('\xa0', ' ').replace('\u200b', ' ').replace('\ufeff', ' ').split())


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


def parse_date(value: str) -> dt.date | None:
    match = re.search(r'(\d{4})-(\d{1,2})-(\d{1,2})', value)
    if not match:
        return None
    return dt.date(int(match.group(1)), int(match.group(2)), int(match.group(3)))


def parse_numeric_max(value: str) -> int | None:
    raw = normalize_text(value)
    if raw in {'', '-'}:
        return None
    matches = re.findall(r'(\d+(?:\.\d+)?)\s*([kK])?', raw)
    if not matches:
        return None
    values: list[int] = []
    for number, suffix in matches:
        amount = float(number)
        if suffix:
            amount *= 1000
        values.append(int(amount))
    return max(values) if values else None


def model_column_index(header: list[str]) -> int:
    for index, column in enumerate(header):
        if 'model参数' in column or 'model入参' in column:
            return index
    raise RuntimeError(f'unable to locate model id column in header: {header!r}')


def normalize_table_rows(index: int, rows: list[list[str]]) -> list[list[str]]:
    if index != 20:
        return rows
    header = rows[0]
    normalized = [header]
    current_group_name = ''
    current_input = ''
    current_rate = ''
    for row in rows[1:]:
        if len(row) == len(header):
            current_group_name = row[0]
            current_input = row[3]
            current_rate = row[4]
            normalized.append(row)
            continue
        if len(row) == 2:
            normalized.append([current_group_name, row[0], row[1], current_input, current_rate])
            continue
        normalized.append(row + [''] * (len(header) - len(row)))
    return normalized


def infer_vendor(model_id: str, display_name: str) -> str:
    lowered = model_id.lower()
    name = display_name.lower()
    if lowered.startswith(('ernie', 'qianfan', 'musesteamer')):
        return 'baidu'
    if lowered in {'embedding-v1', 'tao-8k', 'bce-reranker-base', 'paddleocr-vl-0.9b', 'pp-structurev3'}:
        return 'baidu'
    if lowered.startswith('deepseek'):
        return 'deepseek'
    if lowered.startswith(('qwen', 'qwq', 'gme-qwen')):
        return 'alibaba'
    if lowered.startswith('glm') or 'chatglm' in lowered:
        return 'zhipu'
    if lowered.startswith('kimi'):
        return 'moonshot'
    if lowered.startswith('minimax'):
        return 'minimax'
    if lowered.startswith('internvl'):
        return 'opengvlab'
    if lowered.startswith('bge'):
        return 'baai'
    if lowered.startswith('flux'):
        return 'black_forest_labs'
    if lowered.startswith('mistral'):
        return 'mistral'
    if lowered.startswith('gemma'):
        return 'google'
    if lowered.startswith('gpt-oss'):
        return 'openai'
    if lowered.startswith(('llama', 'linly', 'fuyu')):
        return 'meta_like'
    if 'paddle' in lowered or 'paddle' in name:
        return 'baidu'
    return 'external'


class CatalogBuilder:
    def __init__(self) -> None:
        self.models: OrderedDict[str, OrderedDict[str, object]] = OrderedDict()
        self.canonical_ids: dict[str, str] = {}

    def get(self, model_id: str, display_name: str | None = None) -> OrderedDict[str, object]:
        canonical_id = self.canonical_ids.get(model_id.lower(), model_id)
        if canonical_id not in self.models:
            self.models[canonical_id] = OrderedDict(
                source_url='',
                source_urls=[],
                display_name=display_name or canonical_id,
                status='active',
                vendor=infer_vendor(canonical_id, display_name or canonical_id),
                api_surfaces=[],
                categories=[],
                records=[],
            )
            self.canonical_ids[canonical_id.lower()] = canonical_id
        entry = self.models[canonical_id]
        if display_name and (not entry.get('display_name') or entry['display_name'] == canonical_id):
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

    def set_once(self, entry: OrderedDict[str, object], key: str, value) -> None:
        if value in (None, '', [], {}):
            return
        if key not in entry or entry[key] in ('', None, []):
            entry[key] = value

    def set_status(self, entry: OrderedDict[str, object], status: str) -> None:
        priority = {'active': 0, 'historical': 1, 'pending_retirement': 2, 'retired': 3}
        current = entry.get('status', 'active')
        if priority.get(status, 0) >= priority.get(current, 0):
            entry['status'] = status

    def add_record(self, entry: OrderedDict[str, object], record: OrderedDict[str, object]) -> None:
        entry['records'].append(record)

    def finalize(self) -> OrderedDict[str, OrderedDict[str, object]]:
        finalized: OrderedDict[str, OrderedDict[str, object]] = OrderedDict()
        for model_id in sorted(self.models, key=str.lower):
            entry = self.models[model_id]
            if entry['source_urls'] and not entry['source_url']:
                entry['source_url'] = entry['source_urls'][0]
            if 'summary' in entry and not entry['summary']:
                entry.pop('summary', None)
            finalized[model_id] = entry
        return finalized


def build_catalog() -> OrderedDict[str, OrderedDict[str, object]]:
    model_list_html = fetch_html(MODEL_LIST_URL)
    feature_html = fetch_html(FEATURE_PAGE_URL)
    retirement_html = fetch_html(RETIREMENT_URL)
    quickstart_html = fetch_html(QUICKSTART_URL)
    text_api_html = fetch_html(TEXT_API_URL)
    vision_api_html = fetch_html(VISION_API_URL)
    embedding_api_html = fetch_html(EMBEDDING_API_URL)
    rerank_api_html = fetch_html(RERANK_API_URL)
    image_doc_html = fetch_html(IMAGE_DOC_URL)

    model_tables = table_rows(model_list_html)
    if len(model_tables) < 24:
        raise RuntimeError(f'unexpected qianfan model list table count: {len(model_tables)!r}')
    retirement_tables = table_rows(retirement_html)

    if '/v2/chat/completions' not in normalize_text(BeautifulSoup(quickstart_html, 'html.parser').get_text(' ', strip=True)):
        raise RuntimeError('quickstart page missing /v2/chat/completions endpoint')
    if '/v2/chat/completions' not in normalize_text(BeautifulSoup(text_api_html, 'html.parser').get_text(' ', strip=True)):
        raise RuntimeError('text api page missing /v2/chat/completions endpoint')
    if '/v2/chat/completions' not in normalize_text(BeautifulSoup(vision_api_html, 'html.parser').get_text(' ', strip=True)):
        raise RuntimeError('vision api page missing /v2/chat/completions endpoint')
    if '/v2/embeddings' not in normalize_text(BeautifulSoup(embedding_api_html, 'html.parser').get_text(' ', strip=True)):
        raise RuntimeError('embedding api page missing /v2/embeddings endpoint')
    if '/v2/rerank' not in normalize_text(BeautifulSoup(rerank_api_html, 'html.parser').get_text(' ', strip=True)):
        raise RuntimeError('rerank api page missing /v2/rerank endpoint')
    if '/v2/images/generations' not in normalize_text(BeautifulSoup(image_doc_html, 'html.parser').get_text(' ', strip=True)):
        raise RuntimeError('image doc page missing /v2/images/generations endpoint')

    builder = CatalogBuilder()

    for spec in TABLE_SPECS:
        rows = normalize_table_rows(spec['index'], model_tables[spec['index']])
        header = rows[0]
        model_index = model_column_index(header)
        for row in rows[1:]:
            row = row + [''] * (len(header) - len(row))
            model_id = normalize_text(row[model_index])
            if not model_id or model_id == '-':
                continue
            display_name = normalize_text(row[1] if '版本' in header and len(row) > 1 else row[0])
            entry = builder.get(model_id, display_name)
            entry['vendor'] = infer_vendor(model_id, display_name)
            builder.add_source(entry, MODEL_LIST_URL)
            builder.add_api_surface(entry, *spec['api_surfaces'])
            builder.add_category(entry, *spec['categories'])
            if '版本' in header and row[1] and row[1] != display_name:
                builder.set_once(entry, 'version_label', row[1])
            builder.set_once(entry, 'context_window_tokens', parse_numeric_max(row[2]) if len(row) > 2 and '上下文长度' in header[2] else None)
            if len(row) > 3 and ('最大输入' in header[3] or '输入限制' in header[3]):
                builder.set_once(entry, 'max_input_tokens', parse_numeric_max(row[3]))
                builder.set_once(entry, 'max_input_raw', row[3] if parse_numeric_max(row[3]) is None else None)
            if len(row) > 4 and ('最大输出' in header[4] or '向量维度' in header[3] or '每个文本上下文长度' in header[-1] or '默认流控' in header[-1]):
                if '向量维度' in header:
                    dim_index = header.index('向量维度')
                    builder.set_once(entry, 'embedding_dimensions', parse_numeric_max(row[dim_index]))
                output_candidates = [i for i, column in enumerate(header) if '最大输出' in column]
                if output_candidates:
                    output_index = output_candidates[0]
                    builder.set_once(entry, 'max_output_tokens', parse_numeric_max(row[output_index]))
                    builder.set_once(entry, 'max_output_raw', row[output_index] if parse_numeric_max(row[output_index]) is None else None)
            cot_candidates = [i for i, column in enumerate(header) if '思维链长度' in column]
            if cot_candidates:
                cot_index = cot_candidates[0]
                builder.set_once(entry, 'thinking_tokens', parse_numeric_max(row[cot_index]))
                builder.set_once(entry, 'thinking_tokens_raw', row[cot_index] if parse_numeric_max(row[cot_index]) is None else None)
            rate_candidates = [i for i, column in enumerate(header) if '流控' in column or '速率限制' in column]
            if rate_candidates:
                builder.set_once(entry, 'rate_limit', row[rate_candidates[0]])
            builder.add_record(
                entry,
                OrderedDict(
                    table_kind='source_table',
                    source_url=MODEL_LIST_URL,
                    source_page='model_list',
                    section=spec['section'],
                    subsection=spec['subsection'] or '',
                    columns=header,
                    values=row[:len(header)],
                ),
            )
            if spec['api_reference'] is not None:
                ref_url, ref_page, ref_section, endpoint = spec['api_reference']
                builder.add_source(entry, ref_url)
                builder.add_record(
                    entry,
                    OrderedDict(
                        table_kind='api_reference',
                        source_url=ref_url,
                        source_page=ref_page,
                        section=ref_section,
                        api_surface=spec['api_surfaces'][0],
                        endpoint=endpoint,
                        notes='Mapped from the official Qianfan API category docs for this model family.',
                    ),
                )

    featured_rows = model_tables[0]
    if featured_rows and featured_rows[0][0] == '旗舰模型':
        header = featured_rows[0]
        summary_row = featured_rows[1]
        context_row = featured_rows[2]
        output_row = featured_rows[3]
        for column_index, column_name in enumerate(header[1:], start=1):
            model_id = FEATURED_MODEL_MAP.get(column_name)
            if not model_id or model_id not in builder.models:
                continue
            entry = builder.models[model_id]
            builder.add_source(entry, MODEL_LIST_URL, FEATURE_PAGE_URL)
            builder.set_once(entry, 'summary', summary_row[column_index])
            builder.set_once(entry, 'context_window_tokens', parse_numeric_max(context_row[column_index]))
            builder.set_once(entry, 'max_output_tokens', parse_numeric_max(output_row[column_index]))
            builder.add_record(
                entry,
                OrderedDict(
                    table_kind='featured_table',
                    source_url=MODEL_LIST_URL,
                    source_page='model_list',
                    section='推荐模型',
                    columns=header,
                    values=[row[column_index] if column_index < len(row) else '' for row in featured_rows],
                ),
            )

    quickstart_text = normalize_text(BeautifulSoup(quickstart_html, 'html.parser').get_text(' ', strip=True))
    for legacy_model in ['ernie-3.5-8k', 'ernie-4.0-turbo-8k']:
        if legacy_model in quickstart_text.lower():
            entry = builder.get(legacy_model, legacy_model)
            builder.add_source(entry, QUICKSTART_URL)
            builder.add_api_surface(entry, 'chat.completion')
            builder.add_category(entry, '历史模型', '文本生成')
            builder.set_status(entry, 'historical')
            builder.set_once(entry, 'summary', 'Official Qianfan quickstart examples still reference this legacy model id.')
            builder.add_record(
                entry,
                OrderedDict(
                    table_kind='compatibility_example',
                    source_url=QUICKSTART_URL,
                    source_page='quickstart_model_call',
                    section='快速开始-模型服务调用',
                    api_surface='chat.completion',
                    endpoint=f'{BASE_URL}/chat/completions',
                    notes='Legacy example model id still present in the official quickstart examples.',
                ),
            )

    for table in retirement_tables:
        header = table[0]
        if header != ['登记日期', '退役模型版本', '模型退役日期', '推荐替换模型']:
            continue
        for row in table[1:]:
            if '示意' in ''.join(row):
                continue
            model_id = normalize_text(row[1])
            retirement_date = parse_date(row[2])
            entry = builder.get(model_id, model_id)
            builder.add_source(entry, RETIREMENT_URL)
            builder.add_category(entry, '历史模型')
            if retirement_date is not None and retirement_date > CURRENT_DATE:
                builder.set_status(entry, 'pending_retirement')
            else:
                builder.set_status(entry, 'retired')
            builder.set_once(entry, 'summary', 'Listed in the official Qianfan retirement schedule.')
            builder.add_record(
                entry,
                OrderedDict(
                    table_kind='retirement_notice',
                    source_url=RETIREMENT_URL,
                    source_page='model_retirement',
                    section='模型版本升级及退役机制',
                    registration_date=row[0],
                    retirement_date=row[2],
                    recommended_replacements=row[3],
                ),
            )

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
        '# Generated from official Baidu Qianfan / Wenxin docs.',
        '# Edit via scripts/generate_qianfan_model_catalog.py.',
        '# Sources:',
        f'# - {MODEL_LIST_URL}',
        f'# - {FEATURE_PAGE_URL}',
        f'# - {QUICKSTART_URL}',
        f'# - {TEXT_DOC_URL}',
        f'# - {TEXT_API_URL}',
        f'# - {VISION_DOC_URL}',
        f'# - {VISION_API_URL}',
        f'# - {REASONING_DOC_URL}',
        f'# - {IMAGE_DOC_URL}',
        f'# - {IMAGE_GEN_API_URL}',
        f'# - {IMAGE_EDIT_API_URL}',
        f'# - {EMBEDDING_DOC_URL}',
        f'# - {EMBEDDING_API_URL}',
        f'# - {RERANK_DOC_URL}',
        f'# - {RERANK_API_URL}',
        f'# - {RETIREMENT_URL}',
        f'# Generated at: {now}',
        '',
        '[provider]',
        'id = "qianfan"',
        'display_name = "Baidu Qianfan / Wenxin"',
        f'base_url = {toml_quote(BASE_URL)}',
        'protocol = "qianfan"',
        f'source_url = {toml_quote(MODEL_LIST_URL)}',
        '',
        '[provider.auth]',
        'type = "api_key_env"',
        'keys = ["QIANFAN_BEARER_TOKEN", "QIANFAN_API_KEY"]',
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
        for key in [
            'summary',
            'version_label',
            'context_window_tokens',
            'max_input_tokens',
            'max_input_raw',
            'max_output_tokens',
            'max_output_raw',
            'thinking_tokens',
            'thinking_tokens_raw',
            'embedding_dimensions',
            'rate_limit',
        ]:
            if key in data:
                write_key_value(lines, key, data[key])
        lines.append('')
        record_path = f'models.{toml_quote(model_id)}.records'
        for record in data['records']:
            write_record(lines, record_path, record)
    return '\n'.join(lines).rstrip() + '\n'


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description='Generate Baidu Qianfan provider model catalog')
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
