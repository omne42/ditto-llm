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

MODELS_INDEX_URL = "https://ai.google.dev/gemini-api/docs/models?hl=en"
MODEL_DOC_URL = "https://ai.google.dev/gemini-api/docs/models/{slug}?hl=en"
KNOWN_EXTRA_SLUGS = {
    "gemini-3.1-pro",
    "gemini-3.1-flash",
}
SECTION_HEADERS = {
    "Supported data types",
    "Token limits",
    "Capabilities",
    "Versions",
    "Versions Read the model version patterns for more details.",
    "Latest update",
}
STOP_HEADERS = SECTION_HEADERS | {
    "Pricing",
    "Get code",
    "Try it",
    "Rate limits",
}
PAIR_TERMINATORS = {
    "Capabilities",
    "Versions",
    "Versions Read the model version patterns for more details.",
    "Latest update",
}


def fetch_text(url: str, timeout: float = 30.0) -> str:
    req = urllib.request.Request(
        url,
        headers={
            "User-Agent": "ditto-llm/google-model-catalog-generator",
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


def normalize_key(value: str) -> str:
    value = value.lower().strip()
    value = value.replace("+", " plus ")
    value = re.sub(r"[^a-z0-9]+", "_", value)
    return value.strip("_")


def split_types(value: str) -> list[str]:
    parts = [part.strip().lower() for part in value.split(",")]
    return [part.replace(" ", "_") for part in parts if part]


def model_slugs() -> list[str]:
    raw = fetch_text(MODELS_INDEX_URL)
    slugs = set(re.findall(r"/gemini-api/docs/models/([a-zA-Z0-9._-]+)", raw))
    slugs |= KNOWN_EXTRA_SLUGS
    return sorted(slugs)


def find_last(lines: list[str], value: str) -> int | None:
    for idx in range(len(lines) - 1, -1, -1):
        if lines[idx] == value:
            return idx
    return None


def parse_header(lines: list[str]) -> tuple[str, str, str | None, int]:
    model_code_idx = find_last(lines, "Model code")
    if model_code_idx is None or model_code_idx + 1 >= len(lines):
        raise ValueError("missing model code block")
    display_name = lines[model_code_idx - 1]
    model_code = lines[model_code_idx + 1]
    next_headers = [idx for idx, line in enumerate(lines[model_code_idx + 2 :], start=model_code_idx + 2) if line in STOP_HEADERS]
    summary_end = next_headers[0] if next_headers else len(lines)
    summary_lines = lines[model_code_idx + 2 : summary_end]
    summary = " ".join(summary_lines).strip() or None
    return display_name, model_code, summary, summary_end


def parse_supported_data_types(lines: list[str]) -> OrderedDict[str, list[str]]:
    start = find_last(lines, "Supported data types")
    if start is None:
        return OrderedDict()
    out: OrderedDict[str, list[str]] = OrderedDict()
    idx = start + 1
    while idx + 1 < len(lines) and lines[idx] not in PAIR_TERMINATORS:
        key = lines[idx]
        value = lines[idx + 1]
        if key in {"Input", "Output"}:
            out[normalize_key(key)] = split_types(value)
            idx += 2
            continue
        idx += 1
    return out


def parse_limits(lines: list[str]) -> OrderedDict[str, str]:
    start = find_last(lines, "Token limits")
    if start is None:
        return OrderedDict()
    out: OrderedDict[str, str] = OrderedDict()
    idx = start + 1
    while idx + 1 < len(lines) and lines[idx] not in PAIR_TERMINATORS:
        key = normalize_key(lines[idx])
        value = normalize_key(lines[idx + 1]) if "images" in lines[idx].lower() else lines[idx + 1].replace(",", "")
        if key:
            out[key] = value
        idx += 2
    return out


def parse_capabilities(lines: list[str]) -> OrderedDict[str, str]:
    start = find_last(lines, "Capabilities")
    if start is None:
        return OrderedDict()
    out: OrderedDict[str, str] = OrderedDict()
    idx = start + 1
    while idx + 1 < len(lines) and lines[idx] not in {"Versions", "Versions Read the model version patterns for more details.", "Latest update"}:
        out[normalize_key(lines[idx])] = normalize_key(lines[idx + 1])
        idx += 2
    return out


def parse_versions(lines: list[str]) -> list[dict[str, str]]:
    starts = ["Versions", "Versions Read the model version patterns for more details."]
    start = next((find_last(lines, label) for label in starts if find_last(lines, label) is not None), None)
    if start is None:
        return []
    out: list[dict[str, str]] = []
    idx = start + 1
    while idx + 1 < len(lines) and lines[idx] != "Latest update":
        channel = normalize_key(lines[idx])
        model = lines[idx + 1]
        if channel and re.fullmatch(r"[a-z0-9][a-z0-9._-]*", model):
            out.append({"channel": channel, "model": model})
        idx += 2
    return out


def parse_latest_update(lines: list[str]) -> str | None:
    start = find_last(lines, "Latest update")
    if start is None or start + 1 >= len(lines):
        return None
    return lines[start + 1]


def parse_model(slug: str) -> tuple[str, dict[str, object]]:
    lines = normalize_lines(fetch_text(MODEL_DOC_URL.format(slug=slug)))
    display_name, model_code, summary, _ = parse_header(lines)
    model: dict[str, object] = {
        "source_url": MODEL_DOC_URL.format(slug=slug).replace("?hl=en", ""),
        "display_name": display_name,
        "model_code": model_code,
    }
    if summary:
        model["summary"] = summary
    supported = parse_supported_data_types(lines)
    if supported:
        model["supported_data_types"] = supported
    limits = parse_limits(lines)
    if limits:
        model["limits"] = limits
    capabilities = parse_capabilities(lines)
    if capabilities:
        model["capabilities"] = capabilities
    versions = parse_versions(lines)
    if versions:
        model["versions"] = versions
    latest = parse_latest_update(lines)
    if latest:
        model["latest_update"] = latest
    return slug, model


def toml_quote(value: str) -> str:
    return '"' + value.replace('\\', '\\\\').replace('"', '\\"') + '"'


def write_string_array(values: list[str]) -> str:
    return "[" + ", ".join(toml_quote(value) for value in values) + "]"


def write_catalog(models: OrderedDict[str, dict[str, object]], output_path: Path) -> None:
    generated_at = dt.datetime.utcnow().replace(microsecond=0).isoformat() + "Z"
    lines: list[str] = [
        "# Generated from official Google AI for Developers model docs. Edit via scripts/generate_google_model_catalog.py.",
        f"# Source index: {MODELS_INDEX_URL.replace('?hl=en', '')}",
        f"# Generated at: {generated_at}",
        "",
        "[provider]",
        'id = "google"',
        'display_name = "Google AI for Developers"',
        'base_url = "https://generativelanguage.googleapis.com/v1beta"',
        'protocol = "gemini_generate_content"',
        f"source_url = {toml_quote(MODELS_INDEX_URL.replace('?hl=en', ''))}",
        "",
        "[provider.auth]",
        'type = "query_param_env"',
        'param = "key"',
        'keys = ["GOOGLE_API_KEY"]',
    ]
    for slug, model in models.items():
        lines.extend([
            "",
            f"[models.{toml_quote(slug)}]",
            f"source_url = {toml_quote(str(model['source_url']))}",
            f"display_name = {toml_quote(str(model['display_name']))}",
            f"model_code = {toml_quote(str(model['model_code']))}",
        ])
        if summary := model.get("summary"):
            lines.append(f"summary = {toml_quote(str(summary))}")
        if latest := model.get("latest_update"):
            lines.append(f"latest_update = {toml_quote(str(latest))}")
        if supported := model.get("supported_data_types"):
            lines.extend([
                "",
                f"[models.{toml_quote(slug)}.supported_data_types]",
            ])
            if input_types := supported.get("input"):
                lines.append(f"input = {write_string_array(input_types)}")
            if output_types := supported.get("output"):
                lines.append(f"output = {write_string_array(output_types)}")
        if limits := model.get("limits"):
            lines.extend([
                "",
                f"[models.{toml_quote(slug)}.limits]",
            ])
            for key, value in limits.items():
                lines.append(f"{key} = {toml_quote(value)}")
        if capabilities := model.get("capabilities"):
            lines.extend([
                "",
                f"[models.{toml_quote(slug)}.capabilities]",
            ])
            for key, value in capabilities.items():
                lines.append(f"{key} = {toml_quote(value)}")
        for version in model.get("versions", []):
            lines.extend([
                "",
                f"[[models.{toml_quote(slug)}.versions]]",
                f"channel = {toml_quote(version['channel'])}",
                f"model = {toml_quote(version['model'])}",
            ])
    output_path.write_text("\n".join(lines) + "\n")


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--output",
        type=Path,
        default=Path(__file__).resolve().parents[1] / "catalog" / "provider_models" / "google.toml",
    )
    args = parser.parse_args(argv)

    slugs = model_slugs()
    models: OrderedDict[str, dict[str, object]] = OrderedDict()
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as executor:
        for slug, model in sorted(executor.map(parse_model, slugs), key=lambda item: item[0]):
            models[slug] = model
    write_catalog(models, args.output)
    json_output_path = write_json_sidecar(args.output)
    print(f"wrote {len(models)} google models to {args.output}")
    print(f"wrote {json_output_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
