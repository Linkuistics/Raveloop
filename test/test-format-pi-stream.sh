#!/usr/bin/env bash
# Test for format_pi_stream() in run-plan.sh.
set -eu

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(dirname "$HERE")"

# shellcheck source=../run-plan.sh
. "$ROOT/run-plan.sh"

pass=0
fail=0

assert_contains() {
    local needle="$1"
    local haystack="$2"
    local label="$3"
    if printf '%s' "$haystack" | grep -qF "$needle"; then
        printf 'PASS %s\n' "$label"
        pass=$((pass + 1))
    else
        printf 'FAIL %s — expected to contain: %s\n' "$label" "$needle"
        fail=$((fail + 1))
    fi
}

out="$(format_pi_stream < "$HERE/fixtures/pi-stream-sample.jsonl")"

assert_contains "→ read" "$out" "read tool line"
assert_contains "backlog.md" "$out" "read path rendered"
assert_contains "→ grep" "$out" "grep tool line"
assert_contains "/TODO/" "$out" "grep pattern rendered"
assert_contains "→ bash" "$out" "bash tool line"
assert_contains "cargo test --workspace" "$out" "bash command rendered"
assert_contains "Triage complete" "$out" "final assistant text rendered"
assert_contains "propagation.out.yaml" "$out" "final text full"

out_empty="$(format_pi_stream < "$HERE/fixtures/pi-stream-empty.jsonl")"
assert_contains "Nothing to do" "$out_empty" "empty fixture assistant text"

printf '\n%d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
