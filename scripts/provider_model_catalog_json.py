#!/usr/bin/env python3
from __future__ import annotations

import json
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover
    import tomli as tomllib  # type: ignore


def write_json_sidecar(toml_path: Path, json_path: Path | None = None) -> Path:
    toml_path = Path(toml_path)
    if json_path is None:
        json_path = toml_path.with_suffix('.json')
    data = tomllib.loads(toml_path.read_text(encoding='utf-8'))
    json_path.write_text(
        json.dumps(data, ensure_ascii=False, indent=2) + '\n',
        encoding='utf-8',
    )
    return json_path
