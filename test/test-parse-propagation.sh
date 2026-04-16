#!/usr/bin/env bash
# Test for parse_propagation() in run-plan.sh.
# parse_propagation reads propagation.out.yaml on stdin and prints
# one tab-separated line per entry:
#   <kind>\t<target>\t<summary-single-line>
set -eu

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(dirname "$HERE")"

# shellcheck source=../run-plan.sh
. "$ROOT/run-plan.sh"

pass=0
fail=0

out="$(parse_propagation < "$HERE/fixtures/propagation-sample.yaml")"
line_count="$(printf '%s\n' "$out" | grep -c .)"

if [ "$line_count" -eq 2 ]; then
    printf 'PASS line count (2)\n'
    pass=$((pass + 1))
else
    printf 'FAIL line count — expected 2, got %d\n' "$line_count"
    fail=$((fail + 1))
fi

line1="$(printf '%s\n' "$out" | sed -n 1p)"
line2="$(printf '%s\n' "$out" | sed -n 2p)"

case "$line1" in
    "child"$'\t'"/Users/x/Development/SomeApp/LLM_STATE/core"$'\t'*)
        printf 'PASS line1 kind+target\n'; pass=$((pass + 1)) ;;
    *)
        printf 'FAIL line1 kind+target: %s\n' "$line1"; fail=$((fail + 1)) ;;
esac

case "$line2" in
    "parent"$'\t'"/Users/x/Development/Mnemosyne/LLM_STATE/harness"$'\t'*)
        printf 'PASS line2 kind+target\n'; pass=$((pass + 1)) ;;
    *)
        printf 'FAIL line2 kind+target: %s\n' "$line2"; fail=$((fail + 1)) ;;
esac

case "$line1" in
    *"C function extraction"*)
        printf 'PASS line1 summary content\n'; pass=$((pass + 1)) ;;
    *)
        printf 'FAIL line1 summary content: %s\n' "$line1"; fail=$((fail + 1)) ;;
esac

case "$line2" in
    *"phase-kill semantics"*)
        printf 'PASS line2 summary content\n'; pass=$((pass + 1)) ;;
    *)
        printf 'FAIL line2 summary content: %s\n' "$line2"; fail=$((fail + 1)) ;;
esac

printf '\n%d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
