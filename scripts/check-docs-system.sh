#!/usr/bin/env bash
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"

require_file() {
  local path="$1"
  if [[ ! -f "$path" ]]; then
    echo "missing required file: ${path#$root/}" >&2
    exit 1
  fi
}

require_contains() {
  local path="$1"
  local needle="$2"
  if ! grep -Fq "$needle" "$path"; then
    echo "missing required text in ${path#$root/}: $needle" >&2
    exit 1
  fi
}

require_file "$root/README.md"
require_file "$root/AGENTS.md"
require_file "$root/docs/README.md"
require_file "$root/docs/docs-system-map.md"
require_file "$root/docs/architecture/system-boundaries.md"
require_file "$root/docs/architecture/source-layout.md"
require_file "$root/docs/src/index.md"
require_file "$root/docs/src/SUMMARY.md"

require_contains "$root/README.md" "docs/README.md"
require_contains "$root/README.md" "docs/docs-system-map.md"
require_contains "$root/AGENTS.md" "docs/README.md"
require_contains "$root/AGENTS.md" "docs/docs-system-map.md"

echo "docs system check passed"
