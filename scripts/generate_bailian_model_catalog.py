#!/usr/bin/env python3
from __future__ import annotations

import argparse
import datetime as dt
import re
import sys
import urllib.request
from collections import OrderedDict
from pathlib import Path

from provider_model_catalog_json import write_json_sidecar

SOURCE_URL = 'https://help.aliyun.com/zh/model-studio/models'
REGIONS = {'中国内地', '全球', '国际', '美国', '金融云'}
EXCLUDED_SECTION_MARKERS = ('下线', '已下线模型')
MODEL_NAME_RE = re.compile(r'^([^\s（(]+)')
RELATED_MODEL_RE = re.compile(r'[A-Za-z][A-Za-z0-9]*(?:[./_-][A-Za-z0-9]+)+')
GENERIC_MODEL_TOKENS = {
    '模型名称',
    '版本',
    '说明',
    '价格',
    '免费额度',
    'Token',
    'AI',
}
ALIBABA_PREFIXES = (
    'qwen',
    'wan',
    'wanx',
    'tongyi',
    'fun-asr',
    'cosyvoice',
    'sambert',
    'paraformer',
    'gummy',
    'sensevoice',
    'opennlu',
    'farui',
    'gui',
    'wordart',
    'qvq',
)


def fetch_text(url: str, timeout: float = 30.0) -> str:
    req = urllib.request.Request(
        url,
        headers={
            'User-Agent': 'ditto-llm/bailian-model-catalog-generator',
            'Accept': 'text/html,application/xhtml+xml',
        },
    )
    with urllib.request.urlopen(req, timeout=timeout) as response:
        return response.read().decode('utf-8', 'ignore')


def normalize_text(value: str) -> str:
    return ' '.join(value.split())


def first_token(text: str) -> str | None:
    match = MODEL_NAME_RE.match(text.strip())
    if not match:
        return None
    return match.group(1).strip()


def looks_like_model_id(token: str | None) -> bool:
    if not token or token in GENERIC_MODEL_TOKENS:
        return False
    if len(token) < 3:
        return False
    if re.search(r'[\u4e00-\u9fff]', token):
        return False
    if token.isupper() and len(token) <= 4:
        return False
    if token.startswith('参见'):
        return False
    return re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9./_-]*', token) is not None


def extract_related_model_ids(text: str) -> list[str]:
    seen: list[str] = []
    for token in RELATED_MODEL_RE.findall(text):
        if token not in GENERIC_MODEL_TOKENS and token not in seen:
            seen.append(token)
    return seen


def infer_vendor(model_id: str, category: str, series: str | None) -> str:
    lowered = model_id.lower()
    if '/' in model_id:
        return model_id.split('/', 1)[0].lower()
    if lowered.startswith(ALIBABA_PREFIXES):
        return 'alibaba'
    if category.startswith('文本生成-第三方模型') and series:
        return series.split('-', 1)[0].strip().lower()
    if category.startswith('图像生成-第三方模型') and series:
        return series.strip().lower()
    return lowered.split('-', 1)[0]


def infer_api_surface(category: str, series: str | None, model_id: str) -> str:
    text = f'{category} {series or ""} {model_id}'
    lowered = model_id.lower()
    if 'embedding' in lowered or '文本向量' in text or '多模态向量' in text:
        return 'embedding'
    if 'rerank' in lowered or '重排' in text:
        return 'rerank'
    if '图像翻译' in text:
        return 'image.translation'
    if '图像生成' in text:
        if '编辑' in text or '局部重绘' in text or '擦除' in text or '扩展' in text:
            return 'image.edit'
        return 'image.generation'
    if '视频生成' in text or '视频编辑' in text:
        return 'video.generation'
    if '语音合成' in text or 'tts' in lowered:
        return 'audio.speech'
    if '实时语音识别' in text:
        return 'audio.transcription.realtime'
    if '语音识别' in text or '翻译（语音转成指定语种的文本）' in text or 'asr' in lowered:
        return 'audio.transcription'
    if '文本分类' in text or '抽取' in text:
        return 'classification_or_extraction'
    return 'chat.completion'


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
            text = normalize_text(cell.get_text(' ', strip=True))
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


def model_sort_key(model_id: str) -> tuple[str, str]:
    prefix = model_id.split('/', 1)[0].lower()
    return prefix, model_id.lower()


def toml_quote(value: str) -> str:
    return '"' + value.replace('\\', '\\\\').replace('"', '\\"') + '"'


def toml_array(values: list[str]) -> str:
    return '[' + ', '.join(toml_quote(value) for value in values) + ']'


def write_record(lines: list[str], path: str, record: OrderedDict[str, str | list[str]]) -> None:
    lines.append(f'[[{path}]]')
    for key, value in record.items():
        if isinstance(value, list):
            lines.append(f'{key} = {toml_array(value)}')
        else:
            lines.append(f'{key} = {toml_quote(value)}')
    lines.append('')


def parse_models(html: str) -> OrderedDict[str, dict]:
    from bs4 import BeautifulSoup

    soup = BeautifulSoup(html, 'html.parser')
    current_category: str | None = None
    current_series: str | None = None
    current_region: str | None = None
    models: dict[str, dict] = {}

    for node in soup.find_all(['h2', 'h3', 'h4', 'table']):
        if node.name == 'h2':
            heading = normalize_text(node.get_text(' ', strip=True))
            if heading in REGIONS:
                current_region = heading
            else:
                current_category = heading
                current_series = None
                current_region = None
            continue
        if node.name == 'h3':
            current_series = normalize_text(node.get_text(' ', strip=True))
            continue
        if node.name == 'h4':
            heading = normalize_text(node.get_text(' ', strip=True))
            if heading in REGIONS:
                current_region = heading
            continue

        if not current_category or any(marker in current_category for marker in EXCLUDED_SECTION_MARKERS):
            continue

        grid = table_to_grid(node)
        if not grid:
            continue
        header = grid[0]
        if '模型名称' not in header:
            continue

        model_col = header.index('模型名称')
        data_rows = [row for row in grid[1:] if any(cell.strip() for cell in row)]
        data_rows = [row for row in data_rows if not row[model_col].startswith('（')]
        if not data_rows:
            continue

        record_columns = [normalize_text(cell) for idx, cell in enumerate(header) if idx != model_col and cell]
        if not record_columns:
            continue

        table_kind = 'pricing_tier' if any('单次请求的输入 Token 数' in cell for cell in header) else 'spec'

        for row in data_rows:
            model_cell = row[model_col].strip()
            token = first_token(model_cell)
            if not looks_like_model_id(token):
                continue
            model_id = token
            related_model_ids = extract_related_model_ids(model_cell)
            entry = models.setdefault(
                model_id,
                {
                    'source_url': SOURCE_URL,
                    'display_name': model_id,
                    'status': 'active',
                    'vendor': infer_vendor(model_id, current_category, current_series),
                    'api_surfaces': OrderedDict(),
                    'categories': OrderedDict(),
                    'series': OrderedDict(),
                    'regions': OrderedDict(),
                    'related_model_ids': OrderedDict(),
                    'records': [],
                },
            )
            entry['api_surfaces'][infer_api_surface(current_category, current_series, model_id)] = None
            entry['categories'][current_category] = None
            if current_series:
                entry['series'][current_series] = None
            if current_region:
                entry['regions'][current_region] = None
            for related in related_model_ids:
                if related != model_id:
                    entry['related_model_ids'][related] = None

            values = [normalize_text(cell) for idx, cell in enumerate(row) if idx != model_col]
            values = values[: len(record_columns)] + [''] * max(0, len(record_columns) - len(values))
            record = OrderedDict([
                ('table_kind', table_kind),
                ('category', current_category),
            ])
            if current_series:
                record['series'] = current_series
            if current_region:
                record['region'] = current_region
            record['model_cell'] = model_cell
            record['columns'] = [column for column in record_columns if column]
            record['values'] = [value for value in values[: len(record_columns)]]
            entry['records'].append(record)

    ordered = OrderedDict()
    for model_id in sorted(models, key=model_sort_key):
        entry = models[model_id]
        ordered[model_id] = OrderedDict([
            ('source_url', entry['source_url']),
            ('display_name', entry['display_name']),
            ('status', entry['status']),
            ('vendor', entry['vendor']),
            ('api_surfaces', list(entry['api_surfaces'].keys())),
            ('categories', list(entry['categories'].keys())),
            ('series', list(entry['series'].keys())),
            ('regions', list(entry['regions'].keys())),
            ('related_model_ids', list(entry['related_model_ids'].keys())),
            ('records', entry['records']),
        ])
    return ordered


def write_catalog(models: OrderedDict[str, dict], output_path: Path) -> None:
    generated_at = dt.datetime.utcnow().replace(microsecond=0).isoformat() + 'Z'
    lines: list[str] = [
        '# Generated from official Alibaba Cloud Model Studio (Bailian) model docs.',
        '# Edit via scripts/generate_bailian_model_catalog.py.',
        f'# Source: {SOURCE_URL}',
        f'# Generated at: {generated_at}',
        '',
        '[provider]',
        'id = "bailian"',
        'display_name = "Alibaba Cloud Model Studio (Bailian)"',
        'base_url = "https://dashscope.aliyuncs.com"',
        'protocol = "dashscope"',
        f'source_url = {toml_quote(SOURCE_URL)}',
        '',
        '[provider.auth]',
        'type = "api_key_env"',
        'keys = ["DASHSCOPE_API_KEY"]',
        '',
    ]

    for model_id, model in models.items():
        lines.append(f'[models.{toml_quote(model_id)}]')
        lines.append(f'source_url = {toml_quote(model["source_url"])}')
        lines.append(f'display_name = {toml_quote(model["display_name"])}')
        lines.append(f'status = {toml_quote(model["status"])}')
        lines.append(f'vendor = {toml_quote(model["vendor"])}')
        if model['api_surfaces']:
            lines.append(f'api_surfaces = {toml_array(model["api_surfaces"])}')
        if model['categories']:
            lines.append(f'categories = {toml_array(model["categories"])}')
        if model['series']:
            lines.append(f'series = {toml_array(model["series"])}')
        if model['regions']:
            lines.append(f'regions = {toml_array(model["regions"])}')
        if model['related_model_ids']:
            lines.append(f'related_model_ids = {toml_array(model["related_model_ids"])}')
        lines.append('')
        for record in model['records']:
            write_record(lines, f'models.{toml_quote(model_id)}.records', record)

    output_path.write_text('\n'.join(lines).rstrip() + '\n', encoding='utf-8')


def main() -> int:
    parser = argparse.ArgumentParser(description='Generate Bailian model catalog from official docs.')
    parser.add_argument(
        '--output',
        type=Path,
        default=Path(__file__).resolve().parents[1] / 'catalog' / 'provider_models' / 'bailian.toml',
    )
    args = parser.parse_args()

    try:
        html = fetch_text(SOURCE_URL)
        models = parse_models(html)
        write_catalog(models, args.output)
        write_json_sidecar(args.output)
    except Exception as exc:  # pragma: no cover
        print(f'error: {exc}', file=sys.stderr)
        return 1

    print(args.output)
    print(args.output.with_suffix('.json'))
    print(f'model_count={len(models)}')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
