#!/usr/bin/env bash
# Usage: run-plan.sh <plan-dir>
#
# Drives the four-phase work cycle for a backlog plan, using pi
# (@mariozechner/pi-coding-agent) as the LLM harness.

set -eu

# -----------------------------------------------------------------------------
# Self-location and configuration
# -----------------------------------------------------------------------------

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LLM_CONTEXT_PI_DIR="$SCRIPT_DIR"

# Defensive defaults — overridden by config.sh below if present.
HEADROOM=1500
PROVIDER="anthropic"
WORK_MODEL=""
REFLECT_MODEL=""
COMPACT_MODEL=""
TRIAGE_MODEL=""
WORK_THINKING=""
REFLECT_THINKING=""
COMPACT_THINKING=""
TRIAGE_THINKING=""

if [ -f "$LLM_CONTEXT_PI_DIR/config.sh" ]; then
    # shellcheck source=/dev/null
    . "$LLM_CONTEXT_PI_DIR/config.sh"
fi

# -----------------------------------------------------------------------------
# format_pi_stream — turn pi's --mode json JSONL into a readable trace
# showing tool calls and final assistant text. Requires jq.
# -----------------------------------------------------------------------------

format_pi_stream() {
    jq -j --unbuffered '
        def tool_summary:
          .toolName as $n |
          (.args // {}) as $a |
          "\n→ " + $n +
          (if $n == "read" or $n == "write" or $n == "edit" then
             (if $a.path then " " + $a.path
              elif $a.file_path then " " + $a.file_path
              else "" end)
           elif $n == "find" then
             (if $a.pattern then " " + $a.pattern else "" end)
             + (if $a.path then " (in " + $a.path + ")" else "" end)
           elif $n == "grep" then
             (if $a.pattern then " /" + $a.pattern + "/" else "" end)
             + (if $a.path then " in " + $a.path else "" end)
           elif $n == "ls" then
             (if $a.path then " " + $a.path else "" end)
           elif $n == "bash" then
             (if $a.command then
                " " + ($a.command | gsub("\n"; " ⏎ ")
                                  | if length > 120 then .[0:117] + "…" else . end)
              else "" end)
           else "" end)
          + "\n";

        if .type == "tool_execution_start" then
            tool_summary
        elif .type == "message_end" then
            (.message.content // []
             | map(select(.type == "text") | .text)
             | join(""))
        elif .type == "tool_execution_end" and (.isError == true) then
            "\n[tool error: " + (.toolName // "?") + "]\n"
        else empty end
    '
}

# -----------------------------------------------------------------------------
# Main loop placeholder (guarded — only runs when executed directly)
# -----------------------------------------------------------------------------

if [ "${BASH_SOURCE[0]:-}" = "${0:-}" ]; then
    echo "run-plan.sh: main loop not yet implemented" >&2
    exit 1
fi
