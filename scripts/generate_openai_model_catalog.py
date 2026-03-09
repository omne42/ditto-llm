#!/usr/bin/env python3
from __future__ import annotations

import argparse
import concurrent.futures
import datetime as dt
import html
import re
import sys
import urllib.request
from collections import OrderedDict
from pathlib import Path

from provider_model_catalog_json import write_json_sidecar

MODELS_INDEX_URL = "https://developers.openai.com/api/docs/models"
MODEL_DOC_URL = "https://developers.openai.com/api/docs/models/{slug}"
KNOWN_STAGES = {"default", "recommended", "preview", "legacy", "latest"}
MODALITIES = ("Text", "Image", "Audio", "Video")
SECTION_STOP = {"Endpoints", "Features", "Snapshots", "Rate limits"}
TRUE_FEATURE_STATUSES = {"Supported", "Not supported"}
SKIP_IO_VALUES = {"•", "Input", "Output"}
HEADER_STOP = {
    "Compare",
    "Try in Playground",
    "Intelligence",
    "Performance",
    "Speed",
    "Price",
    "Cost",
    "Input",
    "Output",
    "Pricing",
}


def fetch_text(url: str, timeout: float = 20.0) -> str:
    req = urllib.request.Request(
        url,
        headers={
            "User-Agent": "ditto-llm/openai-model-catalog-generator",
            "Accept": "text/html,application/xhtml+xml",
        },
    )
    with urllib.request.urlopen(req, timeout=timeout) as response:
        return response.read().decode("utf-8", "ignore")


def normalize_lines(raw_html: str) -> list[str]:
    text = raw_html.replace("><", ">\n<")
    text = re.sub(r"<script\b.*?</script>", "", text, flags=re.S)
    text = re.sub(r"<style\b.*?</style>", "", text, flags=re.S)
    text = re.sub(r"<[^>]+>", "\n", text)
    text = html.unescape(text)
    lines = [re.sub(r"\s+", " ", line).strip() for line in text.splitlines()]
    return [line for line in lines if line]


def model_slugs() -> list[str]:
    raw = fetch_text(MODELS_INDEX_URL)
    slugs = sorted(set(re.findall(r'/api/docs/models/([a-zA-Z0-9.-]+)', raw)))
    return slugs


def find_last(lines: list[str], value: str) -> int | None:
    found = None
    for idx, line in enumerate(lines):
        if line == value:
            found = idx
    return found


def normalize_key(value: str) -> str:
    return re.sub(r"[^a-z0-9]+", "_", value.lower()).strip("_")


def normalize_stage(value: str | None) -> str | None:
    if value is None:
        return None
    stage = normalize_key(value)
    return stage if stage in KNOWN_STAGES or "preview" in stage else None


def clean_sentence_lines(lines: list[str]) -> list[str]:
    out: list[str] = []
    for line in lines:
        if not line or line in HEADER_STOP:
            continue
        if re.fullmatch(r"[\d,]+", line):
            continue
        if line.startswith("$"):
            continue
        if out and out[-1] == line:
            continue
        out.append(line)
    return out


def parse_header(lines: list[str]) -> tuple[dict, int]:
    anchors = [idx for idx, line in enumerate(lines) if line in {"Modalities", "Pricing"}]
    if not anchors:
        raise ValueError("missing Pricing/Modalities anchor")
    anchor = anchors[-1]
    model_markers = [idx for idx, line in enumerate(lines[:anchor]) if line == "Models"]
    if not model_markers:
        raise ValueError("missing Models header")
    start = model_markers[-1]
    block = lines[start:anchor]
    if len(block) < 2:
        raise ValueError("model header block is incomplete")

    out: dict[str, object] = {
        "display_name": block[1],
    }

    stage = normalize_stage(block[2]) if len(block) > 2 else None
    if stage is not None:
        out["stage"] = stage

    tagline_lines: list[str] = []
    idx = 2 + (1 if stage is not None else 0)
    while idx < len(block) and block[idx] not in HEADER_STOP:
        tagline_lines.append(block[idx])
        idx += 1
    tagline_lines = clean_sentence_lines(tagline_lines)
    if tagline_lines:
        out["tagline"] = " ".join(tagline_lines)

    metric_stop = len(block)
    for pair_idx, line in enumerate(block):
        if line in {"Pricing", "context window", "max output tokens"} or line.endswith(" knowledge cutoff"):
            metric_stop = pair_idx
            break

    summary_pairs: OrderedDict[str, str] = OrderedDict()
    last_pair_end = idx
    pre_metric_block = block[:metric_stop]
    for pair_idx in range(len(pre_metric_block) - 1):
        key = pre_metric_block[pair_idx]
        value = pre_metric_block[pair_idx + 1]
        if key in {"Input", "Output"} and value not in SKIP_IO_VALUES and not value.startswith("$"):
            summary_pairs[key.lower()] = value
            last_pair_end = pair_idx + 2
    if "input" in summary_pairs:
        out["input"] = summary_pairs["input"]
    if "output" in summary_pairs:
        out["output"] = summary_pairs["output"]

    summary_lines = clean_sentence_lines(block[last_pair_end:metric_stop])
    if summary_lines:
        out["summary"] = " ".join(summary_lines)

    for pair_idx in range(1, len(block)):
        key = block[pair_idx]
        prev = block[pair_idx - 1]
        if key in {"Intelligence", "Performance"} and pair_idx + 1 < len(block):
            out["performance"] = normalize_key(block[pair_idx + 1])
        elif key == "Speed" and pair_idx + 1 < len(block):
            out["speed"] = normalize_key(block[pair_idx + 1])
        elif key == "context window" and re.fullmatch(r"[\d,]+", prev):
            out["context_window"] = int(prev.replace(",", ""))
        elif key == "max output tokens" and re.fullmatch(r"[\d,]+", prev):
            out["max_output_tokens"] = int(prev.replace(",", ""))
        elif key.endswith(" knowledge cutoff"):
            out["knowledge_cutoff"] = key.replace(" knowledge cutoff", "")

    return out, anchor


def parse_modalities(lines: list[str]) -> OrderedDict[str, str]:
    start = find_last(lines, "Modalities")
    if start is None:
        return OrderedDict()
    out: OrderedDict[str, str] = OrderedDict()
    idx = start + 1
    while idx + 1 < len(lines) and lines[idx] not in SECTION_STOP:
        if lines[idx] in MODALITIES:
            out[normalize_key(lines[idx])] = normalize_key(lines[idx + 1])
            idx += 2
        else:
            idx += 1
    return out


def parse_features(lines: list[str], modalities_idx: int | None) -> tuple[OrderedDict[str, bool], OrderedDict[str, bool]]:
    feature_indices = [idx for idx, line in enumerate(lines) if line == "Features"]
    if not feature_indices:
        return OrderedDict(), OrderedDict()
    start = feature_indices[-1]
    if modalities_idx is not None and start < modalities_idx:
        return OrderedDict(), OrderedDict()

    features: OrderedDict[str, bool] = OrderedDict()
    tools: OrderedDict[str, bool] = OrderedDict()
    section = "features"
    idx = start + 1
    while idx + 1 < len(lines):
        line = lines[idx]
        if line in {"Snapshots", "Rate limits"}:
            break
        if line == "Tools":
            section = "tools"
            idx += 2
            continue
        if lines[idx + 1] in TRUE_FEATURE_STATUSES:
            target = tools if section == "tools" else features
            target[normalize_key(line)] = lines[idx + 1] == "Supported"
            idx += 2
            continue
        idx += 1
    return features, tools


def parse_snapshots(lines: list[str], modalities_idx: int | None) -> list[str]:
    snapshot_indices = [idx for idx, line in enumerate(lines) if line == "Snapshots"]
    if not snapshot_indices:
        return []
    start = snapshot_indices[-1]
    if modalities_idx is not None and start < modalities_idx:
        return []
    out: list[str] = []
    for line in lines[start + 1 :]:
        if line == "Rate limits":
            break
        if re.fullmatch(r"[a-z0-9][a-z0-9.-]*", line) and line not in out:
            out.append(line)
    return out


def parse_model(slug: str) -> tuple[str, dict]:
    lines = normalize_lines(fetch_text(MODEL_DOC_URL.format(slug=slug)))
    header, anchor = parse_header(lines)
    modalities_idx = find_last(lines, "Modalities")
    features, tools = parse_features(lines, modalities_idx)
    snapshots = parse_snapshots(lines, modalities_idx)

    model: dict[str, object] = {
        "source_url": MODEL_DOC_URL.format(slug=slug),
        "availability_status": "unverified",
        **header,
    }
    modalities = parse_modalities(lines)
    if modalities:
        model["modalities"] = modalities
    if features:
        model["features"] = features
    if tools:
        model["tools"] = tools
    if snapshots:
        model["snapshots"] = snapshots
    return slug, model


def toml_quote(value: str) -> str:
    return '"' + value.replace('\\', '\\\\').replace('"', '\\"') + '"'


def write_inline_array(values: list[str]) -> str:
    return '[' + ', '.join(toml_quote(value) for value in values) + ']'


def write_catalog(models: dict[str, dict], output_path: Path) -> None:
    generated_at = dt.datetime.utcnow().replace(microsecond=0).isoformat() + 'Z'
    lines = [
        '# Generated from official OpenAI docs. Edit via scripts/generate_openai_model_catalog.py.',
        f'# Source index: {MODELS_INDEX_URL}',
        f'# Generated at: {generated_at}',
        '',
        '[provider]',
        'id = "openai"',
        'display_name = "OpenAI"',
        'base_url = "https://api.openai.com/v1"',
        'protocol = "openai"',
        f'source_url = {toml_quote(MODELS_INDEX_URL)}',
        '',
        '[provider.auth]',
        'type = "api_key_env"',
        'keys = ["OPENAI_API_KEY"]',
        '',
    ]

    for slug in sorted(models):
        model = models[slug]
        lines.append(f'[models.{toml_quote(slug)}]')
        for key in (
            'source_url',
            'availability_status',
            'display_name',
            'stage',
            'tagline',
            'summary',
            'performance',
            'speed',
            'input',
            'output',
            'context_window',
            'max_output_tokens',
            'knowledge_cutoff',
        ):
            value = model.get(key)
            if value is None:
                continue
            if isinstance(value, int):
                lines.append(f'{key} = {value}')
            else:
                lines.append(f'{key} = {toml_quote(str(value))}')

        for section in ('modalities', 'features', 'tools'):
            values = model.get(section)
            if not values:
                continue
            lines.append('')
            lines.append(f'[models.{toml_quote(slug)}.{section}]')
            for section_key, section_value in values.items():
                if isinstance(section_value, bool):
                    lines.append(f'{section_key} = {str(section_value).lower()}')
                else:
                    lines.append(f'{section_key} = {toml_quote(str(section_value))}')

        snapshots = model.get('snapshots') or []
        if snapshots:
            lines.append('')
            lines.append(f'[models.{toml_quote(slug)}.revisions]')
            lines.append(f'snapshots = {write_inline_array(list(snapshots))}')
        lines.append('')

    output_path.write_text('\n'.join(lines), encoding='utf-8')


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        '--output',
        default='catalog/provider_models/openai.toml',
        help='Output path relative to repo root',
    )
    parser.add_argument('--workers', type=int, default=6)
    args = parser.parse_args()

    slugs = model_slugs()
    models: dict[str, dict] = {}
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, args.workers)) as executor:
        futures = {executor.submit(parse_model, slug): slug for slug in slugs}
        for future in concurrent.futures.as_completed(futures):
            slug = futures[future]
            try:
                model_slug, model = future.result()
            except Exception as exc:  # noqa: BLE001
                print(f'failed to parse {slug}: {exc}', file=sys.stderr)
                return 1
            models[model_slug] = model
            print(f'parsed {model_slug}', file=sys.stderr)

    output_path = Path(args.output)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    write_catalog(models, output_path)
    json_output_path = write_json_sidecar(output_path)
    print(f'wrote {output_path} ({len(models)} models)', file=sys.stderr)
    print(f'wrote {json_output_path}', file=sys.stderr)
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
