#!/usr/bin/env python3
from __future__ import annotations

import argparse
import datetime as dt
import html
import re
import sys
import urllib.parse
import urllib.request
from collections import OrderedDict
from pathlib import Path

from provider_model_catalog_json import write_json_sidecar

DOC_INDEX_URL = 'https://docs.bigmodel.cn/llms.txt'
MODEL_OVERVIEW_URL = 'https://docs.bigmodel.cn/cn/guide/start/model-overview.md'
BASE_ORIGIN = 'https://open.bigmodel.cn'
DEFAULT_BASE_URL = 'https://open.bigmodel.cn/api/paas/v4'
DEFAULT_OUTPUT = Path(__file__).resolve().parents[1] / 'catalog' / 'provider_models' / 'zhipu.toml'

MODEL_PAGE_LINK_RE = re.compile(r'\[([^\]]+)\]\((https://docs\.bigmodel\.cn/cn/guide/models/[^)]+\.md)\)')
MARKDOWN_LINK_RE = re.compile(r'\[([^\]]+)\]\(([^)]+)\)')
TABLE_SEPARATOR_RE = re.compile(r':?-{2,}:?')
TAB_RE = re.compile(r'<Tab title="([^"]+)">(.*?)</Tab>', re.S)
CARD_RE = re.compile(r'<Card(?:\s+title="([^"]+)")?[^>]*>(.*?)</Card>', re.S)
EXPANDABLE_RE = re.compile(r'<Expandable title="([^"]+)">(.*?)</Expandable>', re.S)
NOTE_BLOCK_RE = re.compile(r'<(Note|Info|Warning)>\s*(.*?)\s*</\1>', re.S)
CARD_H3_RE = re.compile(r'<h3[^>]*>([^<]+)</h3>\s*<p[^>]*>([^<]+)</p>', re.S)
CODE_MODEL_RE = re.compile(r'(?<![A-Za-z0-9_-])(glm|cogview|cogvideox|embedding|vidu|autoglm|charglm|emohaa|rerank|codegeex)[A-Za-z0-9._/-]*', re.I)

EXTRA_MODEL_PAGES = {
    'https://docs.bigmodel.cn/cn/guide/models/text/glm-4.md',
    'https://docs.bigmodel.cn/cn/guide/models/text/glm-4-long.md',
    'https://docs.bigmodel.cn/cn/guide/models/free/glm-4.5-flash.md',
    'https://docs.bigmodel.cn/cn/guide/models/text/glm-z1.md',
}

API_DOCS = [
    {
        'url': 'https://docs.bigmodel.cn/api-reference/模型-api/对话补全.md',
        'source_page': 'chat_completion',
        'api_surface': 'chat.completion',
    },
    {
        'url': 'https://docs.bigmodel.cn/api-reference/模型-api/对话补全异步.md',
        'source_page': 'chat_completion_async',
        'api_surface': 'chat.completion.async',
    },
    {
        'url': 'https://docs.bigmodel.cn/api-reference/模型-api/图像生成.md',
        'source_page': 'image_generation',
        'api_surface': 'image.generation',
    },
    {
        'url': 'https://docs.bigmodel.cn/api-reference/模型-api/图像生成异步.md',
        'source_page': 'image_generation_async',
        'api_surface': 'image.generation.async',
    },
    {
        'url': 'https://docs.bigmodel.cn/api-reference/模型-api/视频生成异步.md',
        'source_page': 'video_generation_async',
        'api_surface': 'video.generation.async',
    },
    {
        'url': 'https://docs.bigmodel.cn/api-reference/模型-api/文本嵌入.md',
        'source_page': 'embeddings',
        'api_surface': 'embedding',
    },
    {
        'url': 'https://docs.bigmodel.cn/api-reference/模型-api/文本转语音.md',
        'source_page': 'text_to_speech',
        'api_surface': 'audio.speech',
    },
    {
        'url': 'https://docs.bigmodel.cn/api-reference/模型-api/语音转文本.md',
        'source_page': 'speech_to_text',
        'api_surface': 'audio.transcription',
    },
    {
        'url': 'https://docs.bigmodel.cn/api-reference/模型-api/音色复刻.md',
        'source_page': 'voice_clone',
        'api_surface': 'audio.voice_clone',
    },
    {
        'url': 'https://docs.bigmodel.cn/api-reference/模型-api/文本重排序.md',
        'source_page': 'rerank',
        'api_surface': 'rerank',
    },
    {
        'url': 'https://docs.bigmodel.cn/api-reference/模型-api/版面解析.md',
        'source_page': 'ocr',
        'api_surface': 'ocr',
    },
    {
        'url': 'https://docs.bigmodel.cn/cn/asyncapi/realtime.md',
        'source_page': 'realtime',
        'api_surface': 'realtime.websocket',
    },
]

DISPLAY_NAME_MAP = {
    'GLM-5': 'glm-5',
    'GLM-4.7': 'glm-4.7',
    'GLM-4.7-Flash': 'glm-4.7-flash',
    'GLM-4.7-FlashX': 'glm-4.7-flashx',
    'GLM-4.6': 'glm-4.6',
    'GLM-4.6V': 'glm-4.6v',
    'GLM-4.6V-Flash': 'glm-4.6v-flash',
    'GLM-4.6V-FlashX': 'glm-4.6v-flashx',
    'GLM-4.5': 'glm-4.5',
    'GLM-4.5-Air': 'glm-4.5-air',
    'GLM-4.5-AirX': 'glm-4.5-airx',
    'GLM-4.5-X': 'glm-4.5-x',
    'GLM-4.5-Flash': 'glm-4.5-flash',
    'GLM-4-Long': 'glm-4-long',
    'GLM-4-Plus': 'glm-4-plus',
    'GLM-4-Air-250414': 'glm-4-air-250414',
    'GLM-4-AirX': 'glm-4-airx',
    'GLM-4-Flash-250414': 'glm-4-flash-250414',
    'GLM-4-FlashX-250414': 'glm-4-flashx-250414',
    'GLM-Z1': 'glm-z1',
    'GLM-Z1-Air': 'glm-z1-air',
    'GLM-Z1-AirX': 'glm-z1-airx',
    'GLM-Z1-Flash': 'glm-z1-flash',
    'GLM-Z1-FlashX': 'glm-z1-flashx',
    'GLM-4.1V-Thinking-Flash': 'glm-4.1v-thinking-flash',
    'GLM-4.1V-Thinking-FlashX': 'glm-4.1v-thinking-flashx',
    'GLM-4V-Flash': 'glm-4v-flash',
    'GLM-4-Voice': 'glm-4-voice',
    'GLM-ASR-2512': 'glm-asr-2512',
    'GLM-Realtime': 'glm-realtime',
    'GLM-Realtime-Air': 'glm-realtime-air',
    'GLM-Realtime-Flash': 'glm-realtime-flash',
    'GLM-TTS': 'glm-tts',
    'GLM-TTS-Clone': 'glm-tts-clone',
    'GLM-Image': 'glm-image',
    'GLM-OCR': 'glm-ocr',
    'CharGLM-4': 'charglm-4',
    'AutoGLM-Phone': 'autoglm-phone',
    'CogView-4': 'cogview-4',
    'CogView-3-Flash': 'cogview-3-flash',
    'CogVideoX-3': 'cogvideox-3',
    'CogVideoX-2': 'cogvideox-2',
    'CogVideoX-Flash': 'cogvideox-flash',
    'Embedding-3': 'embedding-3',
    'Embedding-2': 'embedding-2',
    'Emohaa': 'emohaa',
    'Rerank': 'rerank',
    'CodeGeeX-4': 'codegeex-4',
    'Vidu 2': 'vidu2',
    'Vidu Q1': 'viduq1',
    'vidu2-image': 'vidu2-image',
    'vidu2-start-end': 'vidu2-start-end',
    'vidu2-reference': 'vidu2-reference',
    'viduq1-image': 'viduq1-image',
    'viduq1-start-end': 'viduq1-start-end',
    'viduq1-text': 'viduq1-text',
}

FAMILY_PAGE_MODEL_IDS = {
    'https://docs.bigmodel.cn/cn/guide/models/video-generation/vidu2.md': ['vidu2-image', 'vidu2-start-end', 'vidu2-reference'],
    'https://docs.bigmodel.cn/cn/guide/models/video-generation/viduq1.md': ['viduq1-image', 'viduq1-start-end', 'viduq1-text'],
    'https://docs.bigmodel.cn/cn/guide/models/sound-and-video/glm-realtime.md': ['glm-realtime-flash', 'glm-realtime-air'],
}

MODALITY_MAP = {
    '文本': 'text',
    '图像': 'image',
    '图片': 'image',
    '视频': 'video',
    '音频': 'audio',
    '文件': 'file',
    'pdf': 'pdf',
    'PDF': 'pdf',
    '首尾帧': 'start_end_frame',
    '参考': 'reference_image',
    '参考图': 'reference_image',
}

PAGE_CATEGORY_OVERRIDES = {
    'https://docs.bigmodel.cn/cn/guide/models/text/glm-z1.md': ['文本模型', '历史模型'],
    'https://docs.bigmodel.cn/cn/guide/models/text/glm-4.md': ['文本模型', '历史模型'],
}


class CatalogBuilder:
    def __init__(self) -> None:
        self.models: dict[str, OrderedDict[str, object]] = {}
        self.page_model_ids: dict[str, list[str]] = {}
        self.current_api_models: set[str] = set()
        self.overview_models: set[str] = set()

    def get(self, model_id: str, display_name: str | None = None) -> OrderedDict[str, object]:
        if model_id not in self.models:
            self.models[model_id] = OrderedDict(
                display_name=display_name or model_id,
                source_urls=[],
                api_surfaces=[],
                categories=[],
                aliases=[],
                records=[],
                status_hints=[],
                vendor='zhipu',
            )
        entry = self.models[model_id]
        if display_name and not entry.get('display_name'):
            entry['display_name'] = display_name
        return entry

    @staticmethod
    def add_unique(values: list[str], *items: str) -> None:
        for item in items:
            if item and item not in values:
                values.append(item)

    def add_source_url(self, entry: OrderedDict[str, object], url: str) -> None:
        self.add_unique(entry['source_urls'], url)

    def add_api_surface(self, entry: OrderedDict[str, object], *surfaces: str) -> None:
        self.add_unique(entry['api_surfaces'], *surfaces)

    def add_category(self, entry: OrderedDict[str, object], *categories: str) -> None:
        self.add_unique(entry['categories'], *categories)

    def add_alias(self, entry: OrderedDict[str, object], alias: str) -> None:
        if alias and alias != entry['display_name'] and alias != entry.get('model_id'):
            self.add_unique(entry['aliases'], alias)

    def add_record(self, entry: OrderedDict[str, object], record: OrderedDict[str, object]) -> None:
        entry['records'].append(record)

    def add_status_hint(self, entry: OrderedDict[str, object], hint: str) -> None:
        self.add_unique(entry['status_hints'], hint)


def fetch_text(url: str, timeout: float = 30.0) -> str:
    parts = urllib.parse.urlsplit(url)
    safe_url = urllib.parse.urlunsplit(
        (
            parts.scheme,
            parts.netloc,
            urllib.parse.quote(parts.path, safe='/%._-~'),
            urllib.parse.quote(parts.query, safe='=&%._-~'),
            parts.fragment,
        )
    )
    req = urllib.request.Request(
        safe_url,
        headers={
            'User-Agent': 'ditto-llm/zhipu-model-catalog-generator',
            'Accept': 'text/markdown,text/plain,text/html,application/xhtml+xml',
        },
    )
    with urllib.request.urlopen(req, timeout=timeout) as response:
        return response.read().decode('utf-8', 'ignore')


def normalize_docs_url(url: str) -> str:
    if url.startswith('http://') or url.startswith('https://'):
        absolute = url
    else:
        absolute = urllib.parse.urljoin('https://docs.bigmodel.cn', url)
    if absolute.startswith('https://docs.bigmodel.cn') and not absolute.endswith('.md'):
        absolute += '.md'
    return absolute


def strip_markdown(text: str) -> str:
    text = html.unescape(text)
    text = text.replace('\\-', '-')
    text = text.replace('&nbsp;', ' ')
    text = re.sub(r'<br\s*/?>', ' / ', text)
    text = MARKDOWN_LINK_RE.sub(r'\1', text)
    text = re.sub(r'!\[[^\]]*\]\([^)]*\)', ' ', text)
    text = re.sub(r'</?[^>]+>', ' ', text)
    text = text.replace('**', '').replace('`', ' ')
    text = text.replace('（即将下线）', '').replace('(即将下线)', '')
    text = text.replace('（已下线）', '').replace('(已下线)', '')
    return ' '.join(text.split())


def clean_block_text(text: str) -> str:
    previous = None
    while previous != text:
        previous = text
        text = EXPANDABLE_RE.sub(lambda m: f'{m.group(1)}: {strip_markdown(m.group(2))}', text)
    return strip_markdown(text)


def split_table_row(line: str) -> list[str] | None:
    line = line.rstrip('\\').strip()
    if not line.startswith('|'):
        return None
    return [part.strip() for part in line.split('|')[1:-1]]


def row_has_separator_only(row: list[str]) -> bool:
    return all(cell == '' or TABLE_SEPARATOR_RE.fullmatch(cell.replace(' ', '')) for cell in row)


def normalize_headers(header_rows: list[list[str]], width: int) -> list[str]:
    headers: list[str] = []
    for column_index in range(width):
        parts: list[str] = []
        for row in header_rows:
            value = strip_markdown(row[column_index])
            if not value or value in parts:
                continue
            parts.append(value)
        headers.append(' / '.join(parts))
    return headers


def parse_table_blocks(markdown: str) -> list[tuple[str | None, list[str]]]:
    lines = markdown.splitlines()
    current_h3: str | None = None
    blocks: list[tuple[str | None, list[str]]] = []
    i = 0
    while i < len(lines):
        line = lines[i].strip()
        if line.startswith('### '):
            current_h3 = strip_markdown(line[4:])
            i += 1
            continue
        if line.startswith('|'):
            block: list[str] = []
            while i < len(lines) and lines[i].lstrip().startswith('|'):
                block.append(lines[i].rstrip())
                i += 1
            blocks.append((current_h3, block))
            continue
        i += 1
    return blocks


def extract_title(markdown: str) -> str:
    for line in markdown.splitlines():
        if line.startswith('# '):
            return strip_markdown(line[2:])
    raise RuntimeError('unable to locate markdown title')


def extract_note_blocks(markdown: str) -> list[str]:
    return [clean_block_text(body) for _kind, body in NOTE_BLOCK_RE.findall(markdown)]


def extract_section(markdown: str, heading_contains: str) -> str:
    lines = markdown.splitlines()
    start = None
    for i, line in enumerate(lines):
        if line.startswith('## ') and heading_contains in strip_markdown(line[3:]):
            start = i + 1
            break
    if start is None:
        return ''
    end = len(lines)
    for j in range(start, len(lines)):
        if lines[j].startswith('## '):
            end = j
            break
    return '\n'.join(lines[start:end])


def extract_overview_summary(markdown: str) -> str | None:
    section = extract_section(markdown, '概览')
    if not section:
        return None
    lines = section.splitlines()
    summary_lines: list[str] = []
    i = 0
    while i < len(lines):
        line = lines[i].strip()
        if not line or line.startswith('> '):
            i += 1
            continue
        if line.startswith('<CardGroup') or line.startswith('<Tabs>') or line.startswith('<Tab ') or line.startswith('<Card ') or line.startswith('```'):
            break
        if line.startswith('<Note>') or line.startswith('<Info>') or line.startswith('<Warning>'):
            break
        if line.startswith('<'):
            i += 1
            continue
        summary_lines.append(strip_markdown(line))
        if len(summary_lines) >= 3:
            break
        i += 1
    if not summary_lines:
        return None
    return ' '.join(summary_lines)


def extract_cards(segment: str) -> list[tuple[str, str]]:
    cards: list[tuple[str, str]] = []
    lines = segment.splitlines()
    i = 0
    while i < len(lines):
        line = lines[i]
        if '<Card' not in line or 'title="' not in line:
            i += 1
            continue
        match = re.search(r'title="([^"]+)"', line)
        if not match:
            i += 1
            continue
        title = strip_markdown(match.group(1))
        body_lines: list[str] = []
        if '</Card>' in line:
            body = line.split('>', 1)[-1].split('</Card>', 1)[0]
            body_lines.append(body)
            i += 1
        else:
            i += 1
            while i < len(lines):
                current = lines[i]
                if '</Card>' in current:
                    before = current.split('</Card>', 1)[0]
                    if before.strip():
                        body_lines.append(before)
                    i += 1
                    break
                body_lines.append(current)
                i += 1
        value = clean_block_text('\n'.join(body_lines))
        if title and value:
            cards.append((title, value))
    return cards


def extract_tab_cards(section: str) -> list[tuple[str, list[tuple[str, str]]]]:
    tabs: list[tuple[str, list[tuple[str, str]]]] = []
    for raw_title, body in TAB_RE.findall(section):
        title = strip_markdown(raw_title)
        cards = extract_cards(body)
        tabs.append((title, cards))
    return tabs


def extract_series_model_cards(markdown: str) -> list[tuple[str, str]]:
    pairs: list[tuple[str, str]] = []
    for raw_title, raw_summary in CARD_H3_RE.findall(markdown):
        title = strip_markdown(raw_title)
        summary = strip_markdown(raw_summary)
        if title and summary:
            pairs.append((title, summary))
    return pairs


def parse_modalities(value: str) -> list[str]:
    result: list[str] = []
    tokens = re.split(r'[、,，/\s]+', value)
    for token in tokens:
        if not token:
            continue
        mapped = MODALITY_MAP.get(token)
        if mapped and mapped not in result:
            result.append(mapped)
    return result


def parse_token_count(value: str) -> int | None:
    match = re.search(r'(\d+(?:\.\d+)?)\s*([KkMm])', value)
    if not match:
        return None
    number = float(match.group(1))
    unit = match.group(2).upper()
    factor = 1000 if unit == 'K' else 1000000
    return int(number * factor)


def canonical_model_id(name: str) -> str:
    cleaned = strip_markdown(name).strip()
    if cleaned in DISPLAY_NAME_MAP:
        return DISPLAY_NAME_MAP[cleaned]
    lowered = cleaned.lower().replace(' ', '-')
    lowered = lowered.replace('–', '-').replace('—', '-')
    lowered = lowered.replace('q1', 'q1')
    lowered = lowered.replace('vidu-2', 'vidu2').replace('vidu-q1', 'viduq1')
    return lowered


def infer_categories_from_url(url: str) -> list[str]:
    if url in PAGE_CATEGORY_OVERRIDES:
        return PAGE_CATEGORY_OVERRIDES[url]
    if '/models/text/' in url:
        return ['文本模型']
    if '/models/vlm/' in url:
        return ['视觉模型']
    if '/models/image-generation/' in url:
        return ['图像生成模型']
    if '/models/video-generation/' in url:
        return ['视频生成模型']
    if '/models/embedding/' in url:
        return ['向量模型']
    if '/models/humanoid/' in url:
        return ['其他模型']
    if '/models/sound-and-video/' in url:
        return ['音视频模型']
    if '/models/free/' in url:
        if 'cogview' in url:
            return ['图像生成模型', '免费模型']
        if 'cogvideo' in url:
            return ['视频生成模型', '免费模型']
        if 'glm-4.6v' in url or 'glm-4.1v' in url or 'glm-4v' in url:
            return ['视觉模型', '免费模型']
        return ['文本模型', '免费模型']
    return []


def infer_page_api_surface(url: str, markdown: str) -> str | None:
    if '/realtime' in url or 'WebSocket' in markdown or '/realtime' in markdown:
        return 'realtime.websocket'
    if '/embeddings' in markdown or '.embeddings.create' in markdown or '/models/embedding/' in url:
        return 'embedding'
    if '/images/generations' in markdown or '.images.generate' in markdown or '/models/image-generation/' in url:
        return 'image.generation'
    if '/videos/generations' in markdown or '/models/video-generation/' in url:
        return 'video.generation.async'
    if '/audio/speech' in markdown or '/models/sound-and-video/glm-tts' in url:
        return 'audio.speech'
    if '/audio/transcriptions' in markdown or '/models/sound-and-video/glm-asr' in url:
        return 'audio.transcription'
    if '/audio/voice-clone' in markdown or '/models/sound-and-video/glm-tts-clone' in url:
        return 'audio.voice_clone'
    if '/ocr' in markdown or '/models/vlm/glm-ocr' in url:
        return 'ocr'
    if '/rerank' in markdown:
        return 'rerank'
    if '/chat/completions' in markdown or 'chat.completions.create' in markdown:
        return 'chat.completion'
    return None


def extract_named_models_from_note(note: str) -> list[str]:
    seen: list[str] = []
    for token in re.findall(r'(GLM(?:-[A-Za-z0-9.]+)+|CharGLM-4|Emohaa|CodeGeeX-4|CogView(?:-[A-Za-z0-9.]+)+|CogVideoX(?:-[A-Za-z0-9.]+)+)', note):
        model_id = canonical_model_id(token)
        if model_id not in seen:
            seen.append(model_id)
    return seen


def add_model_page_record(
    builder: CatalogBuilder,
    model_id: str,
    display_name: str,
    *,
    source_url: str,
    source_page: str,
    section: str,
    summary: str | None,
    cards: list[tuple[str, str]],
    note: str | None,
    categories: list[str],
) -> None:
    entry = builder.get(model_id, display_name=display_name)
    builder.add_source_url(entry, source_url)
    builder.add_category(entry, *categories)
    if display_name != model_id:
        builder.add_alias(entry, display_name)
    if summary and not entry.get('summary'):
        entry['summary'] = summary
    card_map = OrderedDict(cards)
    input_modalities = parse_modalities(card_map.get('输入模态', ''))
    output_modalities = parse_modalities(card_map.get('输出模态', ''))
    if input_modalities and 'input_modalities' not in entry:
        entry['input_modalities'] = input_modalities
    if output_modalities and 'output_modalities' not in entry:
        entry['output_modalities'] = output_modalities
    context_tokens = parse_token_count(card_map.get('上下文窗口', ''))
    if context_tokens and 'context_window_tokens' not in entry:
        entry['context_window_tokens'] = context_tokens
    max_output_tokens = parse_token_count(card_map.get('最大输出 Tokens', '') or card_map.get('最大输出', ''))
    if max_output_tokens and 'max_output_tokens' not in entry:
        entry['max_output_tokens'] = max_output_tokens
    record = OrderedDict()
    record['table_kind'] = 'model_page'
    record['source_url'] = source_url
    record['source_page'] = source_page
    record['section'] = section
    if summary:
        record['summary'] = summary
    if note:
        record['note'] = note
    if cards:
        record['card_titles'] = [title for title, _value in cards]
        record['card_values'] = [value for _title, value in cards]
    builder.add_record(entry, record)


def parse_model_pages(builder: CatalogBuilder) -> None:
    index_text = fetch_text(DOC_INDEX_URL)
    page_urls = {url for _title, url in MODEL_PAGE_LINK_RE.findall(index_text)}
    page_urls |= EXTRA_MODEL_PAGES

    for url in sorted(page_urls):
        markdown = fetch_text(url)
        title = extract_title(markdown)
        notes = extract_note_blocks(markdown)
        primary_note = notes[0] if notes else None
        summary = extract_overview_summary(markdown)
        overview_section = extract_section(markdown, '概览')
        tabs = extract_tab_cards(overview_section)
        discovered_ids: list[str] = []
        categories = infer_categories_from_url(url)

        if url in FAMILY_PAGE_MODEL_IDS:
            model_ids = FAMILY_PAGE_MODEL_IDS[url]
            cards = extract_cards(overview_section)
            for model_id in model_ids:
                display_name = next((name for name, mapped in DISPLAY_NAME_MAP.items() if mapped == model_id and not name.endswith(('2', 'Q1'))), model_id)
                add_model_page_record(
                    builder,
                    model_id,
                    display_name,
                    source_url=url,
                    source_page='model_page',
                    section='概览',
                    summary=summary,
                    cards=cards,
                    note=primary_note,
                    categories=categories,
                )
                discovered_ids.append(model_id)
        elif tabs:
            for tab_title, cards in tabs:
                model_id = canonical_model_id(tab_title)
                add_model_page_record(
                    builder,
                    model_id,
                    tab_title,
                    source_url=url,
                    source_page='model_page',
                    section='概览',
                    summary=summary,
                    cards=cards,
                    note=primary_note,
                    categories=categories,
                )
                discovered_ids.append(model_id)
        elif title == 'GLM-4.5':
            cards = extract_cards(overview_section)
            for model_title, model_summary in extract_series_model_cards(markdown):
                model_id = canonical_model_id(model_title)
                add_model_page_record(
                    builder,
                    model_id,
                    model_title,
                    source_url=url,
                    source_page='model_page',
                    section='概览',
                    summary=model_summary,
                    cards=cards,
                    note=primary_note,
                    categories=categories,
                )
                discovered_ids.append(model_id)
        else:
            cards = extract_cards(overview_section)
            model_id = canonical_model_id(title)
            add_model_page_record(
                builder,
                model_id,
                title,
                source_url=url,
                source_page='model_page',
                section='概览',
                summary=summary,
                cards=cards,
                note=primary_note,
                categories=categories,
            )
            discovered_ids.append(model_id)

        inferred_surface = infer_page_api_surface(url, markdown)
        if inferred_surface:
            for model_id in discovered_ids:
                entry = builder.get(model_id)
                if inferred_surface not in entry['api_surfaces']:
                    builder.add_api_surface(entry, inferred_surface)
                    builder.add_record(
                        entry,
                        OrderedDict(
                            table_kind='inference',
                            source_url=url,
                            source_page='model_page',
                            section='调用示例',
                            api_surface=inferred_surface,
                            notes='Inferred from official model-page examples or interface description.',
                        ),
                    )

        if primary_note:
            named_model_ids = extract_named_models_from_note(primary_note)
            page_title_id = canonical_model_id(title)
            if '系列' in primary_note and page_title_id in named_model_ids:
                named_model_ids = [model_id for model_id in named_model_ids if model_id != page_title_id]
            targets = [model_id for model_id in named_model_ids if model_id in discovered_ids] or discovered_ids
            if '已下线' in primary_note:
                for model_id in targets:
                    builder.add_status_hint(builder.get(model_id), 'historical')
            elif '即将下线' in primary_note:
                for model_id in targets:
                    builder.add_status_hint(builder.get(model_id), 'deprecated')

        builder.page_model_ids[url] = discovered_ids


def extract_operation(markdown: str) -> tuple[str, str] | tuple[None, None]:
    match = re.search(r'^````yaml\s+[^\s]+\s+(get|post|put|delete)\s+([^\s]+)$', markdown, re.M)
    if not match:
        return None, None
    method = match.group(1).upper()
    path = match.group(2)
    return method, path


def extract_model_enums(markdown: str) -> list[str]:
    lines = markdown.splitlines()
    values: list[str] = []
    i = 0
    while i < len(lines):
        stripped = lines[i].strip()
        if stripped == 'model:':
            model_indent = len(lines[i]) - len(lines[i].lstrip())
            j = i + 1
            while j < len(lines):
                raw_line = lines[j]
                line = raw_line.strip()
                indent = len(raw_line) - len(raw_line.lstrip())
                if line and indent <= model_indent:
                    break
                if line == 'enum:':
                    enum_indent = indent
                    k = j + 1
                    while k < len(lines):
                        raw_item = lines[k]
                        item = raw_item.strip()
                        item_indent = len(raw_item) - len(raw_item.lstrip())
                        if item and item_indent <= enum_indent:
                            break
                        if item.startswith('- '):
                            value = strip_markdown(item[2:])
                            if value and value != '禁用仅占位' and re.search(r'[A-Za-z]', value):
                                if value not in values:
                                    values.append(value)
                        k += 1
                    j = k
                    continue
                j += 1
            i = j
            continue
        i += 1
    return values


def parse_api_docs(builder: CatalogBuilder) -> None:
    for doc in API_DOCS:
        markdown = fetch_text(doc['url'])
        section = extract_title(markdown)
        if doc['api_surface'] == 'realtime.websocket':
            model_ids = extract_model_enums(markdown)
            method = 'WS'
            endpoint = 'wss://open.bigmodel.cn/api/paas/v4/realtime'
        else:
            model_ids = extract_model_enums(markdown)
            method, path = extract_operation(markdown)
            endpoint = f'https://open.bigmodel.cn/api{path}' if path else DEFAULT_BASE_URL
        for model_id in model_ids:
            canonical = canonical_model_id(model_id)
            entry = builder.get(canonical, display_name=model_id)
            builder.add_source_url(entry, doc['url'])
            builder.add_api_surface(entry, doc['api_surface'])
            builder.add_record(
                entry,
                OrderedDict(
                    table_kind='api_reference',
                    source_url=doc['url'],
                    source_page=doc['source_page'],
                    section=section,
                    api_surface=doc['api_surface'],
                    method=method,
                    endpoint=endpoint,
                ),
            )
            builder.current_api_models.add(canonical)


def resolve_overview_model_ids(builder: CatalogBuilder, display_name: str, detail_url: str | None) -> list[str]:
    if detail_url:
        page_url = normalize_docs_url(detail_url)
        if page_url in FAMILY_PAGE_MODEL_IDS:
            return list(FAMILY_PAGE_MODEL_IDS[page_url])
    canonical = canonical_model_id(display_name)
    return [canonical]


def parse_model_overview(builder: CatalogBuilder) -> None:
    markdown = fetch_text(MODEL_OVERVIEW_URL)
    for category, block in parse_table_blocks(markdown):
        if category == '即将弃用模型':
            continue
        rows = [split_table_row(line) for line in block]
        rows = [row for row in rows if row is not None]
        if not rows:
            continue
        separator_index = next((i for i, row in enumerate(rows) if row_has_separator_only(row)), None)
        if separator_index is None:
            continue
        width = max(len(row) for row in rows)
        padded_rows = [row + [''] * (width - len(row)) for row in rows]
        headers = normalize_headers(padded_rows[:separator_index], width)
        data_rows = padded_rows[separator_index + 1 :]
        for row in data_rows:
            if not any(cell.strip() for cell in row):
                continue
            first_cell = row[0]
            text_name = strip_markdown(first_cell)
            detail_links = MARKDOWN_LINK_RE.findall(first_cell)
            detail_url = normalize_docs_url(detail_links[0][1]) if detail_links else None
            status_hint = 'deprecated' if '即将下线' in first_cell else None
            model_ids = resolve_overview_model_ids(builder, text_name, detail_url)
            values = [strip_markdown(cell) for cell in row]
            for model_id in model_ids:
                entry = builder.get(model_id, display_name=text_name)
                builder.add_source_url(entry, MODEL_OVERVIEW_URL)
                if category:
                    builder.add_category(entry, category)
                if len(values) > 2 and values[2] and not entry.get('summary'):
                    entry['summary'] = values[2]
                context_text = next((v for h, v in zip(headers, values) if '上下文' in h), '')
                max_output_text = next((v for h, v in zip(headers, values) if '最大输出' in h), '')
                context_tokens = parse_token_count(context_text)
                max_output_tokens = parse_token_count(max_output_text)
                if context_tokens and 'context_window_tokens' not in entry:
                    entry['context_window_tokens'] = context_tokens
                if max_output_tokens and 'max_output_tokens' not in entry:
                    entry['max_output_tokens'] = max_output_tokens
                if status_hint:
                    builder.add_status_hint(entry, status_hint)
                builder.add_record(
                    entry,
                    OrderedDict(
                        table_kind='source_table',
                        source_url=MODEL_OVERVIEW_URL,
                        source_page='model_overview',
                        section=category or '模型一览',
                        columns=headers,
                        values=values,
                        detail_url=detail_url or '',
                    ),
                )
                builder.overview_models.add(model_id)


def infer_categories_from_entry(model_id: str, entry: OrderedDict[str, object]) -> list[str]:
    surfaces = entry.get('api_surfaces', [])
    if any(surface.startswith('video.') for surface in surfaces):
        return ['视频生成模型']
    if any(surface.startswith('image.') for surface in surfaces):
        return ['图像生成模型']
    if any(surface.startswith('audio.') or surface.startswith('realtime.') for surface in surfaces):
        return ['音视频模型']
    if any(surface == 'embedding' for surface in surfaces):
        return ['向量模型']
    if any(surface == 'ocr' for surface in surfaces):
        return ['视觉模型']
    if any(surface == 'rerank' for surface in surfaces):
        return ['其他模型']
    if any(surface.startswith('chat.') for surface in surfaces):
        if model_id.startswith(('glm-4.6v', 'glm-4.1v', 'glm-4v', 'autoglm')):
            return ['视觉模型']
        if model_id.startswith(('charglm', 'emohaa', 'codegeex')):
            return ['其他模型']
        return ['文本模型']
    return []


def finalize_models(builder: CatalogBuilder) -> OrderedDict[str, dict]:
    ordered: OrderedDict[str, dict] = OrderedDict()
    current_like = builder.current_api_models | builder.overview_models

    for model_id in sorted(builder.models, key=str.lower):
        entry = builder.models[model_id]
        hints = set(entry.pop('status_hints'))
        if 'historical' in hints:
            status = 'historical'
        elif 'deprecated' in hints:
            status = 'deprecated'
        elif model_id in current_like:
            status = 'active'
        else:
            status = 'historical'
        entry['status'] = status
        if model_id == 'codegeex-4' and not entry['api_surfaces']:
            builder.add_api_surface(entry, 'chat.completion')
            builder.add_record(
                entry,
                OrderedDict(
                    table_kind='inference',
                    source_url=MODEL_OVERVIEW_URL,
                    source_page='model_overview',
                    section='其他模型',
                    api_surface='chat.completion',
                    notes='Inferred from the official model overview because no dedicated CodeGeeX-4 API page is currently linked from docs.bigmodel.cn.',
                ),
            )
        if not entry['categories']:
            inferred_categories = infer_categories_from_entry(model_id, entry)
            if inferred_categories:
                entry['categories'] = inferred_categories
        if not entry['source_urls']:
            raise RuntimeError(f'model {model_id} has no source_urls')
        if not entry['records']:
            raise RuntimeError(f'model {model_id} has no records')
        entry.move_to_end('status', last=False)
        entry.move_to_end('display_name', last=False)
        ordered[model_id] = entry

    return ordered


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
        '# Generated from official Zhipu AI / GLM docs.',
        '# Edit via scripts/generate_zhipu_model_catalog.py.',
        '# Sources:',
        f'# - {DOC_INDEX_URL}',
        f'# - {MODEL_OVERVIEW_URL}',
    ]
    for doc in API_DOCS:
        lines.append(f'# - {doc["url"]}')
    lines.extend(
        [
            f'# Generated at: {now}',
            '',
            '[provider]',
            'id = "zhipu"',
            'display_name = "Zhipu AI / GLM"',
            f'base_url = {toml_quote(DEFAULT_BASE_URL)}',
            'protocol = "zhipu"',
            f'source_url = {toml_quote(MODEL_OVERVIEW_URL)}',
            '',
            '[provider.auth]',
            'type = "api_key_env"',
            'keys = ["ZHIPU_API_KEY"]',
            '',
        ]
    )

    for model_id, data in models.items():
        lines.append(f'[models.{toml_quote(model_id)}]')
        write_key_value(lines, 'source_url', data['source_urls'][0])
        if len(data['source_urls']) > 1:
            write_key_value(lines, 'source_urls', data['source_urls'])
        write_key_value(lines, 'display_name', data['display_name'])
        write_key_value(lines, 'status', data['status'])
        write_key_value(lines, 'vendor', data['vendor'])
        if data['api_surfaces']:
            write_key_value(lines, 'api_surfaces', data['api_surfaces'])
        if data['categories']:
            write_key_value(lines, 'categories', data['categories'])
        if data['aliases']:
            write_key_value(lines, 'aliases', data['aliases'])
        if 'summary' in data:
            write_key_value(lines, 'summary', data['summary'])
        if 'input_modalities' in data:
            write_key_value(lines, 'input_modalities', data['input_modalities'])
        if 'output_modalities' in data:
            write_key_value(lines, 'output_modalities', data['output_modalities'])
        if 'context_window_tokens' in data:
            write_key_value(lines, 'context_window_tokens', data['context_window_tokens'])
        if 'max_output_tokens' in data:
            write_key_value(lines, 'max_output_tokens', data['max_output_tokens'])
        lines.append('')
        table_path = f'models.{toml_quote(model_id)}.records'
        for record in data['records']:
            write_record(lines, table_path, record)
    return '\n'.join(lines).rstrip() + '\n'


def generate_catalog() -> OrderedDict[str, dict]:
    builder = CatalogBuilder()
    parse_model_pages(builder)
    parse_api_docs(builder)
    parse_model_overview(builder)
    return finalize_models(builder)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description='Generate Zhipu AI / GLM provider model catalog')
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
