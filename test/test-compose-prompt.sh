#!/usr/bin/env bash
# Test for compose_prompt() in run-plan.sh.
set -eu

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(dirname "$HERE")"

# shellcheck source=../run-plan.sh
. "$ROOT/run-plan.sh"

TMP="$HERE/tmp"
rm -rf "$TMP"
mkdir -p "$TMP/project/.git" "$TMP/project/LLM_STATE/testplan"
echo work > "$TMP/project/LLM_STATE/testplan/phase.md"

# Override globals as the main loop would set them
DIR="$TMP/project/LLM_STATE/testplan"
PROJECT="$TMP/project"
DEV_ROOT="$TMP"

out="$(compose_prompt work)"

pass=0; fail=0

assert_contains() {
    if printf '%s' "$out" | grep -qF "$1"; then
        printf 'PASS %s\n' "$2"; pass=$((pass + 1))
    else
        printf 'FAIL %s — expected %s\n' "$2" "$1"; fail=$((fail + 1))
    fi
}

assert_not_contains() {
    if printf '%s' "$out" | grep -qF "$1"; then
        printf 'FAIL %s — should not contain %s\n' "$2" "$1"; fail=$((fail + 1))
    else
        printf 'PASS %s\n' "$2"; pass=$((pass + 1))
    fi
}

assert_contains "$PROJECT/README.md" "project path substituted"
assert_contains "$DIR/backlog.md" "plan path substituted"
assert_contains "$DEV_ROOT/LLM_CONTEXT_PI/fixed-memory" "dev_root substituted"
assert_not_contains "{{PROJECT}}" "no unsubstituted project placeholder"
assert_not_contains "{{DEV_ROOT}}" "no unsubstituted dev_root placeholder"
assert_not_contains "{{PLAN}}" "no unsubstituted plan placeholder"

rm -rf "$TMP"

printf '\n%d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
