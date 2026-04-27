#!/usr/bin/env bash
# Render the .adoc sources under docs/reference/ and docs/tutorial/ into
# embedded HTML fragments under website/docs/ and website/tutorials/.
#
# The www.linkuistics.com pipeline reads website/ directly via git archive,
# wraps each fragment in the site shell, and rewrites sibling-relative
# href="<page>.html" links into directory URLs. This script's only job is
# producing the fragments. Requires `asciidoctor` on PATH —
# `brew install asciidoctor` on macOS.

set -euo pipefail
IFS=$'\n\t'

ROOT=$(cd "$(dirname "$0")/.." && pwd)

command -v asciidoctor >/dev/null || {
  echo "asciidoctor not found — try 'brew install asciidoctor'" >&2
  exit 1
}

render_dir() {
  local src_dir="$1"
  local out_dir="$2"

  [ -d "$src_dir" ] || return 0
  mkdir -p "$out_dir"

  local count=0
  local src
  for src in "$src_dir"/*.adoc; do
    [ -f "$src" ] || continue
    local base
    base=$(basename "$src" .adoc)
    asciidoctor --embedded -o "$out_dir/$base.html" "$src"
    echo "rendered $base.html"
    count=$((count + 1))
  done
  echo "wrote $count fragment(s) to $out_dir"
}

render_dir "$ROOT/docs/reference" "$ROOT/website/docs"
render_dir "$ROOT/docs/tutorial"  "$ROOT/website/tutorials"
