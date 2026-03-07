#!/usr/bin/env python3
from __future__ import annotations

import argparse
import datetime as dt
import json
import re
import sys
import urllib.parse
import urllib.request
from collections import OrderedDict
from pathlib import Path

from provider_model_catalog_json import write_json_sidecar

SOURCE_URL = 'https://www.volcengine.com/docs/82379/1330310?lang=zh'
CHAT_API_URL = 'https://www.volcengine.com/docs/82379/1494384?lang=zh'
RESPONSES_API_URL = 'https://www.volcengine.com/docs/82379/1569618?lang=zh'
DEFAULT_BASE_URL = 'https://ark.cn-beijing.volces.com/api/v3'
DEFAULT_OUTPUT = Path(__file__).resolve().parents[1] / 'catalog' / 'provider_models' / 'doubao.toml'
STATUS_ORDER = {'active': 0, 'historical': 1, 'pending_retirement': 2}
API_SURFACE_MAP = {
    'Responses API': 'responses',
    'Response API': 'responses',
    'Chat API': 'chat.completion',
    'Video Generation API': 'video.generation',
    'Image generation API': 'image.generation',
    '3D生成 API': '3d.generation',
    'Embedding API': 'embedding',
    'Embeddings Multimodal API': 'embedding.multimodal',
    'Context Create API': 'context.cache',
    'Context API': 'context.cache',
    'Batch API': 'batch',
}
MODEL_ID_RE = re.compile(r'[A-Za-z0-9./_-]*\d[A-Za-z0-9./_-]*')
TABLE_SEPARATOR_RE = re.compile(r':?-{2,}:?')
MARKDOWN_LINK_RE = re.compile(r'\[([^\]]+)\]\(([^)]+)\)')


def fetch_text(url: str, timeout: float = 30.0) -> str:
    req = urllib.request.Request(
        url,
        headers={
            'User-Agent': 'ditto-llm/doubao-model-catalog-generator',
            'Accept': 'text/html,application/xhtml+xml',
        },
    )
    with urllib.request.urlopen(req, timeout=timeout) as response:
        return response.read().decode('utf-8', 'ignore')


def extract_router_data(html: str) -> dict:
    match = re.search(r'window\._ROUTER_DATA = (\{.*?\})</script>', html, re.S)
    if not match:
        raise RuntimeError('unable to locate window._ROUTER_DATA in source HTML')
    return json.loads(match.group(1))


def load_doc() -> dict:
    html = fetch_text(SOURCE_URL)
    router_data = extract_router_data(html)
    page = router_data['loaderData']['docs/(libid)/(docid$)/page']
    return page['curDoc']


def split_table_row(line: str) -> list[str] | None:
    line = line.rstrip('\\').strip()
    if not line.startswith('|'):
        return None
    return [part.strip() for part in line.split('|')[1:-1]]


def strip_markdown(text: str) -> str:
    text = text.replace('\\-', '-').replace('&nbsp;', ' ').replace('^^', 'same_as_above')
    text = re.sub(r'<[^>]+>', ' ', text)
    text = re.sub(r'!\[[^\]]*\]\([^)]*\)', ' ', text)
    text = MARKDOWN_LINK_RE.sub(r'\1', text)
    text = text.replace('**', '').replace('`', ' ')
    return ' '.join(text.split())


def extract_links(text: str) -> list[tuple[str, str]]:
    return [(title.strip(), url.strip()) for title, url in MARKDOWN_LINK_RE.findall(text)]


def extract_model_ids(cell: str) -> list[str]:
    raw = cell.replace('\\-', '-')
    model_ids: list[str] = []
    for title, _url in extract_links(raw):
        if MODEL_ID_RE.fullmatch(title):
            model_ids.append(title)

    normalized = strip_markdown(raw)
    if not model_ids or '同时支持' in normalized:
        for token in MODEL_ID_RE.findall(normalized):
            if token.startswith('docs/'):
                continue
            if 'volcengine.com' in token or 'byteplus.com' in token:
                continue
            if token not in model_ids:
                model_ids.append(token)
    return model_ids


def extract_detail_model_id(detail_url: str | None) -> str | None:
    if not detail_url:
        return None
    query = urllib.parse.parse_qs(urllib.parse.urlparse(detail_url).query)
    return query.get('Id', [None])[0]


def infer_vendor(model_id: str) -> str:
    lowered = model_id.lower()
    mappings = [
        (('doubao', 'seed'), 'bytedance'),
        (('deepseek',), 'deepseek'),
        (('glm',), 'zhipu'),
        (('kimi', 'moonshot'), 'moonshot'),
        (('qwen',), 'alibaba'),
        (('claude',), 'anthropic'),
        (('gpt', 'o1', 'o3', 'o4'), 'openai'),
        (('gemini',), 'google'),
    ]
    for prefixes, vendor in mappings:
        if lowered.startswith(prefixes):
            return vendor
    return lowered.split('-', 1)[0]


def normalize_api_refs(text: str) -> tuple[list[str], list[str], list[str]]:
    titles: list[str] = []
    urls: list[str] = []
    surfaces: list[str] = []
    for title, url in extract_links(text or ''):
        titles.append(title)
        urls.append(url)
        surface = API_SURFACE_MAP.get(title)
        if surface and surface not in surfaces:
            surfaces.append(surface)
    return titles, urls, surfaces


def collect_table_blocks(markdown: str) -> list[tuple[str | None, str | None, str | None, list[str], list[str]]]:
    lines = markdown.splitlines()
    blocks: list[tuple[str | None, str | None, str | None, list[str], list[str]]] = []
    current_h1: str | None = None
    current_h2: str | None = None
    current_h3: str | None = None
    h1_info: list[str] = []
    h2_info: list[str] = []
    h3_info: list[str] = []

    i = 0
    while i < len(lines):
        line = lines[i].strip()
        if line.startswith('# '):
            current_h1 = strip_markdown(line[2:])
            current_h2 = None
            current_h3 = None
            h1_info = []
            h2_info = []
            h3_info = []
            i += 1
            continue
        if line.startswith('## '):
            current_h2 = strip_markdown(line[3:])
            current_h3 = None
            h2_info = []
            h3_info = []
            i += 1
            continue
        if line.startswith('### '):
            current_h3 = strip_markdown(line[4:])
            h3_info = []
            i += 1
            continue
        if line.startswith('|'):
            block: list[str] = []
            while i < len(lines) and lines[i].lstrip().startswith('|'):
                block.append(lines[i].rstrip())
                i += 1
            info_lines = list(h3_info or h2_info or h1_info)
            blocks.append((current_h1, current_h2, current_h3, info_lines, block))
            continue
        if line and not line.startswith('<') and not line.startswith(':::') and line != '&nbsp;':
            target = h3_info if current_h3 else h2_info if current_h2 else h1_info
            target.append(line)
            del target[:-4]
        i += 1
    return blocks


def row_has_separator_only(row: list[str]) -> bool:
    return all(cell == '' or TABLE_SEPARATOR_RE.fullmatch(cell.replace(' ', '')) for cell in row)


def normalize_headers(header_rows: list[list[str]], width: int) -> list[str]:
    headers: list[str] = []
    for column_index in range(width):
        parts: list[str] = []
        for row in header_rows:
            value = strip_markdown(row[column_index])
            if not value or value == 'same_as_above' or value in parts:
                continue
            parts.append(value)
        headers.append(' / '.join(parts))
    return headers


def merge_unique(dest: list[str], values: list[str]) -> None:
    for value in values:
        if value and value not in dest:
            dest.append(value)


def parse_models(markdown: str) -> OrderedDict[str, dict]:
    models: dict[str, dict] = {}

    for section, subsection, subsubsection, info_lines, block in collect_table_blocks(markdown):
        rows = [split_table_row(line) for line in block]
        rows = [row for row in rows if row is not None]
        if not rows:
            continue

        separator_index = next((i for i, row in enumerate(rows) if row_has_separator_only(row)), None)
        if separator_index is None:
            continue

        width = max(len(row) for row in rows)
        padded_rows = [row + [''] * (width - len(row)) for row in rows]
        header_rows = padded_rows[:separator_index]
        data_rows = padded_rows[separator_index + 1 :]
        headers = normalize_headers(header_rows, width)

        info_text = ' | '.join(info_lines)
        section_api_titles, section_api_urls, section_surfaces = normalize_api_refs(info_text)
        last_nonempty = [''] * width
        current_primary_model_id: str | None = None
        last_detail_model_id: str | None = None
        last_detail_url: str = ''

        for raw_row in data_rows:
            values: list[str] = []
            for index, cell in enumerate(raw_row):
                value = strip_markdown(cell)
                if value == 'same_as_above':
                    value = last_nonempty[index]
                if value:
                    last_nonempty[index] = value
                values.append(value)
            if not any(values):
                continue

            first_cell_raw = raw_row[0]
            model_ids = extract_model_ids(first_cell_raw)
            alias_row = '同时支持' in strip_markdown(first_cell_raw)
            detail_links = extract_links(first_cell_raw)
            detail_url = detail_links[0][1] if detail_links else ''
            detail_model_id = extract_detail_model_id(detail_url)

            if model_ids and not alias_row:
                current_primary_model_id = model_ids[0]
                if detail_model_id:
                    last_detail_model_id = detail_model_id
                if detail_url:
                    last_detail_url = detail_url
            if not model_ids and current_primary_model_id:
                model_ids = [current_primary_model_id]
                detail_model_id = last_detail_model_id
                detail_url = last_detail_url
            if not model_ids:
                continue

            row_text = ' '.join(values)
            if '待下线' in row_text:
                status = 'pending_retirement'
            elif any(part and '往期' in part for part in (subsection, subsubsection)):
                status = 'historical'
            else:
                status = 'active'

            row_api_titles, row_api_urls, row_surfaces = normalize_api_refs(' '.join(raw_row[1:]))
            combined_surfaces: list[str] = []
            merge_unique(combined_surfaces, section_surfaces)
            merge_unique(combined_surfaces, row_surfaces)
            combined_api_titles: list[str] = []
            combined_api_urls: list[str] = []
            merge_unique(combined_api_titles, section_api_titles)
            merge_unique(combined_api_titles, row_api_titles)
            merge_unique(combined_api_urls, section_api_urls)
            merge_unique(combined_api_urls, row_api_urls)

            for model_id in model_ids:
                entry = models.setdefault(
                    model_id,
                    {
                        'source_url': SOURCE_URL,
                        'display_name': model_id,
                        'status': 'active',
                        'vendor': infer_vendor(model_id),
                        'api_surfaces': [],
                        'capability_sections': [],
                        'subsections': [],
                        'console_model_ids': [],
                        'related_model_ids': [],
                        'records': [],
                    },
                )

                if STATUS_ORDER[status] > STATUS_ORDER[entry['status']]:
                    entry['status'] = status
                merge_unique(entry['api_surfaces'], combined_surfaces)
                merge_unique(entry['capability_sections'], [section or ''])
                merge_unique(entry['subsections'], [subsection or '', subsubsection or ''])
                if detail_model_id:
                    merge_unique(entry['console_model_ids'], [detail_model_id])

                if alias_row and current_primary_model_id and current_primary_model_id != model_id:
                    merge_unique(entry['related_model_ids'], [current_primary_model_id])
                    primary_entry = models.setdefault(
                        current_primary_model_id,
                        {
                            'source_url': SOURCE_URL,
                            'display_name': current_primary_model_id,
                            'status': 'active',
                            'vendor': infer_vendor(current_primary_model_id),
                            'api_surfaces': [],
                            'capability_sections': [],
                            'subsections': [],
                            'console_model_ids': [],
                            'related_model_ids': [],
                            'records': [],
                        },
                    )
                    merge_unique(primary_entry['related_model_ids'], [model_id])

                record = OrderedDict()
                record['table_kind'] = 'capability_matrix'
                record['section'] = section or ''
                if subsection:
                    record['subsection'] = subsection
                if subsubsection:
                    record['subsubsection'] = subsubsection
                record['status'] = status
                record['columns'] = headers
                record['values'] = values
                if detail_url:
                    record['detail_url'] = detail_url
                if detail_model_id:
                    record['detail_model_id'] = detail_model_id
                record['api_titles'] = combined_api_titles
                record['api_urls'] = combined_api_urls
                record['api_surfaces'] = combined_surfaces
                if alias_row:
                    record['alias_row'] = True
                entry['records'].append(record)

    ordered = OrderedDict()
    for model_id in sorted(models, key=str.lower):
        model = models[model_id]
        if not model['api_surfaces']:
            raise RuntimeError(f'model {model_id} has no api_surfaces after parsing')
        ordered[model_id] = model
    return ordered


def toml_quote(value: str) -> str:
    return '"' + value.replace('\\', '\\\\').replace('"', '\\"') + '"'


def toml_array(values: list[str]) -> str:
    return '[' + ', '.join(toml_quote(value) for value in values) + ']'


def toml_bool(value: bool) -> str:
    return 'true' if value else 'false'


def write_key_value(lines: list[str], key: str, value) -> None:
    if isinstance(value, list):
        lines.append(f'{key} = {toml_array(value)}')
    elif isinstance(value, bool):
        lines.append(f'{key} = {toml_bool(value)}')
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
        '# Generated from official Volcengine Ark / Doubao model docs.',
        '# Edit via scripts/generate_doubao_model_catalog.py.',
        '# Sources:',
        f'# - {SOURCE_URL}',
        f'# - {CHAT_API_URL}',
        f'# - {RESPONSES_API_URL}',
        f'# Generated at: {now}',
        '',
        '[provider]',
        'id = "doubao"',
        'display_name = "Volcengine Ark / Doubao"',
        f'base_url = {toml_quote(DEFAULT_BASE_URL)}',
        'protocol = "ark"',
        f'source_url = {toml_quote(SOURCE_URL)}',
        '',
        '[provider.auth]',
        'type = "api_key_env"',
        'keys = ["ARK_API_KEY"]',
        '',
    ]

    for model_id, model in models.items():
        prefix = f'[models.{toml_quote(model_id)}]'
        lines.append(prefix)
        write_key_value(lines, 'source_url', model['source_url'])
        write_key_value(lines, 'display_name', model['display_name'])
        write_key_value(lines, 'status', model['status'])
        write_key_value(lines, 'vendor', model['vendor'])
        write_key_value(lines, 'api_surfaces', model['api_surfaces'])
        write_key_value(lines, 'capability_sections', model['capability_sections'])
        if model['subsections']:
            write_key_value(lines, 'subsections', model['subsections'])
        if model['console_model_ids']:
            write_key_value(lines, 'console_model_ids', model['console_model_ids'])
        if model['related_model_ids']:
            write_key_value(lines, 'related_model_ids', model['related_model_ids'])
        lines.append('')
        table_path = f'models.{toml_quote(model_id)}.records'
        for record in model['records']:
            write_record(lines, table_path, record)
    return '\n'.join(lines).rstrip() + '\n'


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description='Generate Doubao / Volcengine Ark provider model catalog')
    parser.add_argument('--output', type=Path, default=DEFAULT_OUTPUT, help='Output TOML path')
    args = parser.parse_args(argv)

    doc = load_doc()
    models = parse_models(doc['MDContent'])
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(render_toml(models), encoding='utf-8')
    write_json_sidecar(args.output)
    print(f'wrote {len(models)} models to {args.output}')
    return 0


if __name__ == '__main__':
    sys.exit(main())
