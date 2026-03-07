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

MODELS_INTRO_URL = 'https://platform.minimaxi.com/docs/guides/models-intro'
API_OVERVIEW_URL = 'https://platform.minimaxi.com/docs/api-reference/api-overview'
TEXT_ANTHROPIC_URL = 'https://platform.minimaxi.com/docs/api-reference/text-anthropic-api'
TEXT_OPENAI_URL = 'https://platform.minimaxi.com/docs/api-reference/text-openai-api'
TEXT_CHAT_URL = 'https://platform.minimaxi.com/docs/api-reference/text-chat'
PROMPT_CACHE_URL = 'https://platform.minimaxi.com/docs/api-reference/text-prompt-caching'
ANTHROPIC_CACHE_URL = 'https://platform.minimaxi.com/docs/api-reference/anthropic-api-compatible-cache'
MUSIC_INTRO_URL = 'https://platform.minimaxi.com/docs/api-reference/music-intro'
MUSIC_GENERATION_URL = 'https://platform.minimaxi.com/docs/api-reference/music-generation'
DEFAULT_OUTPUT = Path(__file__).resolve().parents[1] / 'catalog' / 'provider_models' / 'minimax.toml'
ZERO_WIDTH = '\u200b'

RAW_TO_CANONICAL = {
    'Speech-2.8-HD': 'speech-2.8-hd',
    'Speech-2.8-Turbo': 'speech-2.8-turbo',
    'Speech-2.6-HD': 'speech-2.6-hd',
    'Speech-2.6-Turbo': 'speech-2.6-turbo',
    'Speech-02-HD': 'speech-02-hd',
    'Speech-02-Turbo': 'speech-02-turbo',
    'MiniMax Hailuo 2.3': 'MiniMax-Hailuo-2.3',
    'MiniMax Hailuo 2.3 Fast': 'MiniMax-Hailuo-2.3-Fast',
    'MiniMax Hailuo 02': 'MiniMax-Hailuo-02',
    'MiniMax M2.5': 'MiniMax-M2.5',
    'MiniMax M2.5-highspeed': 'MiniMax-M2.5-highspeed',
    'MiniMax M2.1': 'MiniMax-M2.1',
    'MiniMax M2.1-highspeed': 'MiniMax-M2.1-highspeed',
    'MiniMax M2': 'MiniMax-M2',
    'MiniMax M2-her': 'M2-her',
    'Music2.5+': 'music-2.5+',
}
TEXT_MODELS = [
    'MiniMax-M2.5',
    'MiniMax-M2.5-highspeed',
    'MiniMax-M2.1',
    'MiniMax-M2.1-highspeed',
    'MiniMax-M2',
]
SPEECH_MODELS = [
    'speech-2.8-hd',
    'speech-2.8-turbo',
    'speech-2.6-hd',
    'speech-2.6-turbo',
    'speech-02-hd',
    'speech-02-turbo',
]
VIDEO_MODELS = ['MiniMax-Hailuo-2.3', 'MiniMax-Hailuo-2.3-Fast', 'MiniMax-Hailuo-02']
IMAGE_MODELS = ['image-01', 'image-01-live']
MUSIC_MODELS = ['music-2.0', 'music-2.5', 'music-2.5+']
EXPECTED_MODELS = set(TEXT_MODELS + ['M2-her'] + SPEECH_MODELS + VIDEO_MODELS + IMAGE_MODELS + MUSIC_MODELS)


class ModelCatalogBuilder:
    def __init__(self) -> None:
        self.models: dict[str, dict] = {}

    def get(self, model_id: str) -> dict:
        return self.models.setdefault(
            model_id,
            {
                'source_urls': [],
                'display_name': model_id,
                'status': 'active',
                'vendor': 'minimax',
                'api_surfaces': [],
                'categories': [],
                'records': [],
                'cache_modes': [],
                'aliases': [],
            },
        )

    @staticmethod
    def add_unique(values: list[str], *items: str) -> None:
        for item in items:
            if item and item not in values:
                values.append(item)

    def add_model_table_record(
        self,
        model_id: str,
        *,
        source_url: str,
        source_page: str,
        section: str,
        columns: list[str],
        values: list[str],
        categories: list[str],
        api_surfaces: list[str],
        alias: str | None = None,
    ) -> None:
        entry = self.get(model_id)
        self.add_unique(entry['source_urls'], source_url)
        self.add_unique(entry['categories'], *categories)
        self.add_unique(entry['api_surfaces'], *api_surfaces)
        if alias and alias != model_id:
            self.add_unique(entry['aliases'], alias)
        record = OrderedDict()
        record['table_kind'] = 'source_table'
        record['source_url'] = source_url
        record['source_page'] = source_page
        record['section'] = section
        if api_surfaces:
            record['api_surfaces'] = api_surfaces
        record['columns'] = columns
        record['values'] = values
        entry['records'].append(record)
        if len(values) > 1 and values[1] and 'summary' not in entry:
            entry['summary'] = values[1]
        if len(values) > 1 and len(columns) > 1 and columns[1] in ('输入输出总 token', '上下文窗口'):
            digits = re.sub(r'[^0-9]', '', values[1])
            if digits:
                entry['context_window_tokens'] = int(digits)

    def add_api_reference_record(
        self,
        model_ids: list[str],
        *,
        source_url: str,
        source_page: str,
        section: str,
        api_surface: str,
        endpoint: str | None = None,
        base_url: str | None = None,
        notes: str | None = None,
    ) -> None:
        for model_id in model_ids:
            entry = self.get(model_id)
            self.add_unique(entry['source_urls'], source_url)
            self.add_unique(entry['api_surfaces'], api_surface)
            record = OrderedDict()
            record['table_kind'] = 'api_reference'
            record['source_url'] = source_url
            record['source_page'] = source_page
            record['section'] = section
            record['api_surface'] = api_surface
            if endpoint:
                record['endpoint'] = endpoint
            if base_url:
                record['base_url'] = base_url
            if notes:
                record['notes'] = notes
            entry['records'].append(record)

    def add_cache_support(
        self,
        model_ids: list[str],
        *,
        source_url: str,
        source_page: str,
        section: str,
        cache_mode: str,
        notes: str,
    ) -> None:
        for model_id in model_ids:
            entry = self.get(model_id)
            self.add_unique(entry['source_urls'], source_url)
            self.add_unique(entry['api_surfaces'], 'context.cache')
            self.add_unique(entry['cache_modes'], cache_mode)
            record = OrderedDict()
            record['table_kind'] = 'feature_support'
            record['source_url'] = source_url
            record['source_page'] = source_page
            record['section'] = section
            record['api_surface'] = 'context.cache'
            record['cache_mode'] = cache_mode
            record['notes'] = notes
            entry['records'].append(record)



def fetch_text(url: str, timeout: float = 30.0) -> str:
    req = urllib.request.Request(
        url,
        headers={
            'User-Agent': 'ditto-llm/minimax-model-catalog-generator',
            'Accept': 'text/html,application/xhtml+xml',
        },
    )
    with urllib.request.urlopen(req, timeout=timeout) as response:
        return response.read().decode('utf-8', 'ignore')



def clean_text(text: str) -> str:
    return ' '.join(text.replace('\xa0', ' ').replace(ZERO_WIDTH, ' ').split())



def canonical_model_id(raw_name: str) -> str:
    raw_name = clean_text(raw_name)
    if raw_name in RAW_TO_CANONICAL:
        return RAW_TO_CANONICAL[raw_name]
    if raw_name.startswith(('MiniMax-M', 'speech-', 'music-', 'image-', 'M2-her', 'MiniMax-Hailuo-')):
        return raw_name.split()[0]
    if raw_name.startswith('MiniMax Hailuo '):
        suffix = raw_name[len('MiniMax Hailuo ') :].replace(' Fast', '-Fast').replace(' ', '-')
        return 'MiniMax-Hailuo-' + suffix
    return raw_name



def table_to_grid(table) -> list[list[str]]:
    rows = table.find_all('tr')
    grid: list[list[str]] = []
    spans: dict[int, tuple[str, int]] = {}
    for tr in rows:
        row: list[str] = []
        col = 0
        while col in spans:
            text, left = spans[col]
            row.append(text)
            left -= 1
            if left:
                spans[col] = (text, left)
            else:
                spans.pop(col, None)
            col += 1
        for cell in tr.find_all(['th', 'td'], recursive=False):
            text = clean_text(cell.get_text(' ', strip=True))
            rowspan = int(cell.get('rowspan', 1) or 1)
            colspan = int(cell.get('colspan', 1) or 1)
            for _ in range(colspan):
                row.append(text)
                if rowspan > 1:
                    spans[col] = (text, rowspan - 1)
                col += 1
                while col in spans:
                    text2, left = spans[col]
                    row.append(text2)
                    left -= 1
                    if left:
                        spans[col] = (text2, left)
                    else:
                        spans.pop(col, None)
                    col += 1
        grid.append(row)
    width = max((len(row) for row in grid), default=0)
    return [row + [''] * (width - len(row)) for row in grid]



def iter_tables_with_context(html: str):
    soup = BeautifulSoup(html, 'html.parser')
    h1: str | None = None
    h2: str | None = None
    h3: str | None = None
    for node in soup.find_all(['h1', 'h2', 'h3', 'table']):
        if node.name == 'h1':
            h1 = clean_text(node.get_text(' ', strip=True))
            continue
        if node.name == 'h2':
            h2 = clean_text(node.get_text(' ', strip=True))
            h3 = None
            continue
        if node.name == 'h3':
            h3 = clean_text(node.get_text(' ', strip=True))
            continue
        yield h1, h2, h3, table_to_grid(node)



def parse_models_intro(builder: ModelCatalogBuilder, html: str) -> None:
    for _h1, h2, h3, grid in iter_tables_with_context(html):
        if not grid or h2 != '模型概览' or grid[0][:2] != ['模型名称', '介绍']:
            continue
        category = h3 or '文本模型'
        for row in grid[1:]:
            if not row or not row[0]:
                continue
            builder.add_model_table_record(
                canonical_model_id(row[0]),
                source_url=MODELS_INTRO_URL,
                source_page='models_intro',
                section=' / '.join(part for part in ['旗舰模型', h2, h3] if part),
                columns=grid[0],
                values=row,
                categories=[category],
                api_surfaces=[],
                alias=row[0],
            )



def parse_api_overview(builder: ModelCatalogBuilder, html: str) -> None:
    surface_by_section = {
        '同步语音合成（T2A）': ('语音模型', ['audio.speech']),
        '异步长文本语音生成（T2A Async）': ('语音模型', ['audio.speech.async']),
        '音色快速复刻（Voice Cloning）': ('语音模型', ['audio.voice_cloning']),
        '音色设计（Voice Design）': ('语音模型', ['audio.voice_design']),
        '视频生成（Video Generation）': ('视频模型', ['video.generation']),
        '图像生成（Image Generation）': ('图片模型', ['image.generation']),
        '音乐生成 (Music Generation)': ('音乐模型', ['music.generation']),
    }
    for _h1, h2, h3, grid in iter_tables_with_context(html):
        if not grid or h3 not in ('支持模型', '模型列表'):
            continue
        header = grid[0]
        if header[0] not in ('模型名称', '模型'):
            continue
        if h2 == '文本生成':
            for row in grid[1:]:
                if not row or not row[0]:
                    continue
                builder.add_model_table_record(
                    canonical_model_id(row[0]),
                    source_url=API_OVERVIEW_URL,
                    source_page='api_overview',
                    section=' / '.join(part for part in ['接口概览', h2, h3] if part),
                    columns=header,
                    values=row,
                    categories=['文本模型'],
                    api_surfaces=[],
                    alias=row[0],
                )
            continue
        category, surfaces = surface_by_section.get(h2, (None, None))
        if not category or not surfaces:
            continue
        for row in grid[1:]:
            if not row or not row[0]:
                continue
            builder.add_model_table_record(
                canonical_model_id(row[0]),
                source_url=API_OVERVIEW_URL,
                source_page='api_overview',
                section=' / '.join(part for part in ['接口概览', h2, h3] if part),
                columns=header,
                values=row,
                categories=[category],
                api_surfaces=surfaces,
                alias=row[0],
            )



def parse_text_compat(builder: ModelCatalogBuilder, html: str, *, source_url: str, source_page: str, surface: str, base_url: str) -> None:
    for _h1, h2, _h3, grid in iter_tables_with_context(html):
        if not grid or h2 != '支持的模型' or grid[0][:3] != ['模型名称', '上下文窗口', '模型介绍']:
            continue
        for row in grid[1:]:
            if not row or not row[0]:
                continue
            model_id = canonical_model_id(row[0])
            builder.add_model_table_record(
                model_id,
                source_url=source_url,
                source_page=source_page,
                section=' / '.join(part for part in [source_page.replace('_', ' '), h2] if part),
                columns=grid[0],
                values=row,
                categories=['文本模型'],
                api_surfaces=[surface],
                alias=row[0],
            )
            builder.add_api_reference_record(
                [model_id],
                source_url=source_url,
                source_page=source_page,
                section=source_page.replace('_', ' '),
                api_surface=surface,
                base_url=base_url,
                notes='Official compatibility page exposes this protocol surface for the model.',
            )
        break



def parse_text_chat(builder: ModelCatalogBuilder, html: str) -> None:
    entry = builder.get('M2-her')
    builder.add_unique(entry['categories'], '文本对话模型')
    if 'summary' not in entry:
        entry['summary'] = '文本对话模型，专为角色扮演、多轮对话等场景设计'
    if 'https://api.minimaxi.com/v1/text/chatcompletion_v2' in html:
        builder.add_api_reference_record(
            ['M2-her'],
            source_url=TEXT_CHAT_URL,
            source_page='text_chat',
            section='文本对话',
            api_surface='minimax.chatcompletion_v2',
            endpoint='https://api.minimaxi.com/v1/text/chatcompletion_v2',
            notes='Official native chatcompletion_v2 endpoint for M2-her.',
        )



def parse_prompt_cache(builder: ModelCatalogBuilder, html: str) -> None:
    if 'MiniMax-M2.5 系列' not in html:
        raise RuntimeError('expected MiniMax-M2.5 series support in prompt cache page')
    builder.add_cache_support(
        TEXT_MODELS,
        source_url=PROMPT_CACHE_URL,
        source_page='prompt_cache',
        section='Prompt 缓存 / Cache 对比',
        cache_mode='passive',
        notes='Prompt 缓存（被动缓存）页面将 MiniMax-M2.5 / M2.1 / M2 系列列为支持模型。',
    )



def parse_anthropic_cache(builder: ModelCatalogBuilder, html: str) -> None:
    matched = False
    for _h1, h2, _h3, grid in iter_tables_with_context(html):
        if not grid or h2 != '支持的模型和定价' or grid[0][:2] != ['模型', '输入价格 元/百万 tokens']:
            continue
        matched = True
        for row in grid[1:]:
            if not row or not row[0]:
                continue
            model_id = canonical_model_id(row[0])
            builder.add_cache_support(
                [model_id],
                source_url=ANTHROPIC_CACHE_URL,
                source_page='anthropic_cache',
                section='Anthropic 主动缓存 / 支持的模型和定价',
                cache_mode='anthropic_active',
                notes='Anthropic 主动缓存页面将该模型列为支持模型。',
            )
            entry = builder.get(model_id)
            record = OrderedDict()
            record['table_kind'] = 'pricing_table'
            record['source_url'] = ANTHROPIC_CACHE_URL
            record['source_page'] = 'anthropic_cache'
            record['section'] = 'Anthropic 主动缓存 / 支持的模型和定价'
            record['api_surfaces'] = ['context.cache', 'anthropic.messages']
            record['cache_mode'] = 'anthropic_active'
            record['columns'] = grid[0]
            record['values'] = row
            entry['records'].append(record)
        break
    if not matched:
        raise RuntimeError('failed to find anthropic cache pricing table')



def parse_music_intro(builder: ModelCatalogBuilder, html: str) -> None:
    for _h1, h2, _h3, grid in iter_tables_with_context(html):
        if not grid or h2 != '支持模型' or grid[0][:2] != ['模型名称', '使用方法']:
            continue
        for row in grid[1:]:
            if not row or not row[0]:
                continue
            builder.add_model_table_record(
                canonical_model_id(row[0]),
                source_url=MUSIC_INTRO_URL,
                source_page='music_intro',
                section='音乐生成 / 支持模型',
                columns=grid[0],
                values=row,
                categories=['音乐模型'],
                api_surfaces=['music.generation'],
                alias=row[0],
            )
        break



def parse_music_generation(builder: ModelCatalogBuilder, html: str) -> None:
    found = []
    for model_id in ['music-2.5+', 'music-2.5']:
        if model_id in html:
            found.append(model_id)
    for model_id in found:
        builder.add_api_reference_record(
            [model_id],
            source_url=MUSIC_GENERATION_URL,
            source_page='music_generation',
            section='音乐生成',
            api_surface='music.generation',
            endpoint='https://api.minimaxi.com/v1/music_generation',
            notes='Official music generation examples use this model on the native music_generation endpoint.',
        )
    if not found:
        raise RuntimeError('expected music-2.5+/music-2.5 examples in music generation page')



def add_category_endpoints(builder: ModelCatalogBuilder) -> None:
    builder.add_api_reference_record(
        VIDEO_MODELS,
        source_url='https://platform.minimaxi.com/docs/api-reference/video-generation-t2v',
        source_page='video_generation',
        section='视频生成',
        api_surface='video.generation',
        endpoint='https://api.minimaxi.com/v1/video_generation',
        notes='Official T2V/I2V docs use the shared video_generation endpoint.',
    )
    builder.add_api_reference_record(
        IMAGE_MODELS,
        source_url='https://platform.minimaxi.com/docs/api-reference/image-generation-t2i',
        source_page='image_generation',
        section='图像生成',
        api_surface='image.generation',
        endpoint='https://api.minimaxi.com/v1/image_generation',
        notes='Official T2I/I2I docs use the shared image_generation endpoint.',
    )
    builder.add_api_reference_record(
        SPEECH_MODELS,
        source_url='https://platform.minimaxi.com/docs/api-reference/speech-t2a-http',
        source_page='speech_t2a_http',
        section='同步语音合成 HTTP',
        api_surface='audio.speech',
        endpoint='https://api.minimaxi.com/v1/t2a_v2',
        notes='Speech docs navigation exposes POST /v1/t2a_v2 for synchronous TTS.',
    )
    builder.add_api_reference_record(
        SPEECH_MODELS,
        source_url='https://platform.minimaxi.com/docs/api-reference/speech-t2a-async-create',
        source_page='speech_t2a_async',
        section='异步长文本语音合成',
        api_surface='audio.speech.async',
        endpoint='https://api.minimaxi.com/v1/t2a_async_v2',
        notes='Speech docs navigation exposes POST /v1/t2a_async_v2 for async TTS.',
    )
    builder.add_api_reference_record(
        SPEECH_MODELS,
        source_url='https://platform.minimaxi.com/docs/api-reference/voice-cloning-clone',
        source_page='voice_cloning',
        section='音色快速复刻',
        api_surface='audio.voice_cloning',
        endpoint='https://api.minimaxi.com/v1/voice_clone',
        notes='Speech docs navigation exposes POST /v1/voice_clone for voice cloning.',
    )
    builder.add_api_reference_record(
        SPEECH_MODELS,
        source_url='https://platform.minimaxi.com/docs/api-reference/voice-design-design',
        source_page='voice_design',
        section='音色设计',
        api_surface='audio.voice_design',
        endpoint='https://api.minimaxi.com/v1/voice_design',
        notes='Speech docs navigation exposes POST /v1/voice_design for voice design.',
    )
    builder.add_api_reference_record(
        MUSIC_MODELS,
        source_url=MUSIC_GENERATION_URL,
        source_page='music_generation',
        section='音乐生成',
        api_surface='music.generation',
        endpoint='https://api.minimaxi.com/v1/music_generation',
        notes='Official music generation API uses the music_generation endpoint.',
    )



def validate_models(models: dict[str, dict]) -> OrderedDict[str, dict]:
    model_ids = set(models)
    missing = EXPECTED_MODELS - model_ids
    extra = model_ids - EXPECTED_MODELS
    if missing or extra:
        raise RuntimeError(f'model set mismatch: missing={sorted(missing)} extra={sorted(extra)}')
    for model_id, data in models.items():
        if not data['api_surfaces']:
            raise RuntimeError(f'model {model_id} has no api_surfaces')
        if not data['categories']:
            raise RuntimeError(f'model {model_id} has no categories')
        if not data['records']:
            raise RuntimeError(f'model {model_id} has no records')
    return OrderedDict((model_id, models[model_id]) for model_id in sorted(models, key=str.lower))



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



def write_record(lines: list[str], table_path: str, record: OrderedDict[str, object]) -> None:
    lines.append(f'[[{table_path}]]')
    for key, value in record.items():
        write_key_value(lines, key, value)
    lines.append('')



def render_toml(models: OrderedDict[str, dict]) -> str:
    now = dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat().replace('+00:00', 'Z')
    lines = [
        '# Generated from official MiniMax docs.',
        '# Edit via scripts/generate_minimax_model_catalog.py.',
        '# Sources:',
        f'# - {MODELS_INTRO_URL}',
        f'# - {API_OVERVIEW_URL}',
        f'# - {TEXT_ANTHROPIC_URL}',
        f'# - {TEXT_OPENAI_URL}',
        f'# - {TEXT_CHAT_URL}',
        f'# - {PROMPT_CACHE_URL}',
        f'# - {ANTHROPIC_CACHE_URL}',
        f'# - {MUSIC_INTRO_URL}',
        f'# - {MUSIC_GENERATION_URL}',
        f'# Generated at: {now}',
        '',
        '[provider]',
        'id = "minimax"',
        'display_name = "MiniMax"',
        'base_url = "https://api.minimaxi.com"',
        'protocol = "minimax"',
        f'source_url = {toml_quote(MODELS_INTRO_URL)}',
        '',
        '[provider.auth]',
        'type = "api_key_env"',
        'keys = ["MINIMAX_API_KEY"]',
        '',
    ]
    for model_id, data in models.items():
        table = f'[models.{toml_quote(model_id)}]'
        lines.append(table)
        primary_source = data['source_urls'][0]
        write_key_value(lines, 'source_url', primary_source)
        if len(data['source_urls']) > 1:
            write_key_value(lines, 'source_urls', data['source_urls'])
        write_key_value(lines, 'display_name', data['display_name'])
        write_key_value(lines, 'status', data['status'])
        write_key_value(lines, 'vendor', data['vendor'])
        write_key_value(lines, 'api_surfaces', data['api_surfaces'])
        write_key_value(lines, 'categories', data['categories'])
        if data.get('aliases'):
            write_key_value(lines, 'aliases', data['aliases'])
        if data.get('cache_modes'):
            write_key_value(lines, 'cache_modes', data['cache_modes'])
        if 'summary' in data:
            write_key_value(lines, 'summary', data['summary'])
        if 'context_window_tokens' in data:
            write_key_value(lines, 'context_window_tokens', data['context_window_tokens'])
        lines.append('')
        table_path = f'models.{toml_quote(model_id)}.records'
        for record in data['records']:
            write_record(lines, table_path, record)
    return '\n'.join(lines).rstrip() + '\n'



def generate_catalog() -> OrderedDict[str, dict]:
    builder = ModelCatalogBuilder()
    parse_models_intro(builder, fetch_text(MODELS_INTRO_URL))
    parse_api_overview(builder, fetch_text(API_OVERVIEW_URL))
    parse_text_compat(
        builder,
        fetch_text(TEXT_ANTHROPIC_URL),
        source_url=TEXT_ANTHROPIC_URL,
        source_page='text_anthropic',
        surface='anthropic.messages',
        base_url='https://api.minimaxi.com/anthropic',
    )
    parse_text_compat(
        builder,
        fetch_text(TEXT_OPENAI_URL),
        source_url=TEXT_OPENAI_URL,
        source_page='text_openai',
        surface='chat.completion',
        base_url='https://api.minimaxi.com/v1',
    )
    parse_text_chat(builder, fetch_text(TEXT_CHAT_URL))
    parse_prompt_cache(builder, fetch_text(PROMPT_CACHE_URL))
    parse_anthropic_cache(builder, fetch_text(ANTHROPIC_CACHE_URL))
    parse_music_intro(builder, fetch_text(MUSIC_INTRO_URL))
    parse_music_generation(builder, fetch_text(MUSIC_GENERATION_URL))
    add_category_endpoints(builder)
    return validate_models(builder.models)



def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description='Generate MiniMax provider model catalog')
    parser.add_argument('--output', type=Path, default=DEFAULT_OUTPUT, help='Output TOML path')
    args = parser.parse_args(argv)

    models = generate_catalog()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(render_toml(models), encoding='utf-8')
    write_json_sidecar(args.output)
    print(f'wrote {len(models)} models to {args.output}')
    return 0


if __name__ == '__main__':
    sys.exit(main())
