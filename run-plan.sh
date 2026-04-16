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
# parse_propagation — read propagation.out.yaml on stdin, emit one
# tab-separated line per entry:
#   <kind>\t<target>\t<summary-joined-to-one-line>
# -----------------------------------------------------------------------------

parse_propagation() {
    awk '
        function flush() {
            if (have_entry) {
                gsub(/\t/, " ", summary)
                gsub(/[[:space:]]+$/, "", summary)
                printf "%s\t%s\t%s\n", kind, target, summary
            }
            target = ""; kind = ""; summary = ""
            in_summary = 0; summary_indent = -1
            have_entry = 0
        }
        BEGIN { have_entry = 0; in_summary = 0 }
        /^[[:space:]]*$/ {
            if (in_summary && summary != "") summary = summary " "
            next
        }
        /^propagations:[[:space:]]*$/ { next }
        /^[[:space:]]*-[[:space:]]+target:/ {
            flush()
            sub(/^[[:space:]]*-[[:space:]]+target:[[:space:]]*/, "")
            target = $0
            have_entry = 1
            next
        }
        /^[[:space:]]+kind:/ {
            sub(/^[[:space:]]+kind:[[:space:]]*/, "")
            kind = $0
            next
        }
        /^[[:space:]]+summary:[[:space:]]*\|[[:space:]]*$/ {
            in_summary = 1
            summary_indent = -1
            next
        }
        {
            if (in_summary) {
                line = $0
                if (summary_indent == -1) {
                    match(line, /^[[:space:]]*/)
                    summary_indent = RLENGTH
                }
                if (length(line) >= summary_indent) {
                    line = substr(line, summary_indent + 1)
                }
                if (summary == "") summary = line
                else summary = summary " " line
            }
        }
        END { flush() }
    '
}

# -----------------------------------------------------------------------------
# list_plans_in — discover plan directories under a project's LLM_STATE/ tree.
# -----------------------------------------------------------------------------

list_plans_in() {
    local project_root="$1"
    if [ ! -d "$project_root/LLM_STATE" ]; then
        return 0
    fi
    find "$project_root/LLM_STATE" -type f -name phase.md -print 2>/dev/null | \
        while IFS= read -r phase_file; do
            dirname "$phase_file"
        done
}

# -----------------------------------------------------------------------------
# parse_related_projects — extract entries from related-plans.md sections.
# -----------------------------------------------------------------------------

parse_related_projects() {
    local file="$1"
    local section="$2"
    if [ ! -f "$file" ]; then
        return 0
    fi
    awk -v section="$section" '
        /^## / {
            if (tolower($0) ~ tolower(section)) {
                in_section = 1
            } else {
                in_section = 0
            }
            next
        }
        in_section && /^- / {
            line = $0
            sub(/^- /, "", line)
            sub(/ [—-].*$/, "", line)
            sub(/[[:space:]]+$/, "", line)
            print line
        }
    ' "$file"
}

# -----------------------------------------------------------------------------
# build_related_plans — synthesize the {{RELATED_PLANS}} block for a plan.
# -----------------------------------------------------------------------------

build_related_plans() {
    local plan_dir="$1"
    local project_root="$2"
    local dev_root="$3"

    local siblings=()
    local parents=()
    local children=()

    while IFS= read -r p; do
        if [ -n "$p" ] && [ "$p" != "$plan_dir" ]; then
            siblings+=("$p")
        fi
    done < <(list_plans_in "$project_root")

    local related_file="$plan_dir/related-plans.md"
    while IFS= read -r proj_entry; do
        if [ -z "$proj_entry" ]; then continue; fi
        local proj_path
        proj_path="${proj_entry//\{\{DEV_ROOT\}\}/$dev_root}"
        while IFS= read -r p; do
            if [ -n "$p" ]; then
                parents+=("$p")
            fi
        done < <(list_plans_in "$proj_path")
    done < <(parse_related_projects "$related_file" "Parents")

    while IFS= read -r proj_entry; do
        if [ -z "$proj_entry" ]; then continue; fi
        local proj_path
        proj_path="${proj_entry//\{\{DEV_ROOT\}\}/$dev_root}"
        while IFS= read -r p; do
            if [ -n "$p" ]; then
                children+=("$p")
            fi
        done < <(list_plans_in "$proj_path")
    done < <(parse_related_projects "$related_file" "Children")

    if [ ${#siblings[@]} -eq 0 ] && [ ${#parents[@]} -eq 0 ] && [ ${#children[@]} -eq 0 ]; then
        echo "Related plans: (none)"
        return 0
    fi

    echo "Related plans:"
    echo ""
    if [ ${#siblings[@]} -gt 0 ]; then
        echo "Siblings (same project):"
        for p in "${siblings[@]}"; do echo "- $p"; done
        echo ""
    fi
    if [ ${#parents[@]} -gt 0 ]; then
        echo "Parents (from declared peer projects):"
        for p in "${parents[@]}"; do echo "- $p"; done
        echo ""
    fi
    if [ ${#children[@]} -gt 0 ]; then
        echo "Children (from declared peer projects):"
        for p in "${children[@]}"; do echo "- $p"; done
        echo ""
    fi
}

# -----------------------------------------------------------------------------
# compose_prompt — read shared phases/<phase>.md + optional per-plan prompt,
# substitute placeholders, concatenate, print to stdout.
# -----------------------------------------------------------------------------

compose_prompt() {
    local phase="$1"
    local shared="$LLM_CONTEXT_PI_DIR/phases/$phase.md"
    local per_plan="$DIR/prompt-$phase.md"

    if [ ! -f "$shared" ]; then
        echo "Error: no phases/$phase.md in $LLM_CONTEXT_PI_DIR" >&2
        exit 1
    fi

    local related
    related="$(build_related_plans "$DIR" "$PROJECT" "$DEV_ROOT")"

    local shared_sub
    shared_sub="$(
        RELATED_PLANS_VAR="$related" awk -v dev_root="$DEV_ROOT" \
            -v project="$PROJECT" \
            -v plan="$DIR" '
            BEGIN { related = ENVIRON["RELATED_PLANS_VAR"] }
            {
                gsub(/\{\{DEV_ROOT\}\}/, dev_root)
                gsub(/\{\{PROJECT\}\}/, project)
                gsub(/\{\{PLAN\}\}/, plan)
                gsub(/\{\{RELATED_PLANS\}\}/, related)
                print
            }
        ' "$shared"
    )"

    if [ -f "$per_plan" ]; then
        local per_plan_sub
        per_plan_sub="$(
            RELATED_PLANS_VAR="$related" awk -v dev_root="$DEV_ROOT" \
                -v project="$PROJECT" \
                -v plan="$DIR" '
                BEGIN { related = ENVIRON["RELATED_PLANS_VAR"] }
                {
                    gsub(/\{\{DEV_ROOT\}\}/, dev_root)
                    gsub(/\{\{PROJECT\}\}/, project)
                    gsub(/\{\{PLAN\}\}/, plan)
                    gsub(/\{\{RELATED_PLANS\}\}/, related)
                    print
                }
            ' "$per_plan"
        )"
        printf '%s\n\n%s\n' "$shared_sub" "$per_plan_sub"
    else
        printf '%s\n' "$shared_sub"
    fi
}

# -----------------------------------------------------------------------------
# Main loop (guarded — only runs if this script is executed directly)
# -----------------------------------------------------------------------------

if [ "${BASH_SOURCE[0]:-}" = "${0:-}" ]; then
    PLAN_ARG=""

    for arg in "$@"; do
        case "$arg" in
            -*)
                echo "Unknown option: $arg" >&2
                echo "Usage: $0 <plan-dir>" >&2
                exit 1
                ;;
            *)
                if [ -n "$PLAN_ARG" ]; then
                    echo "Unexpected extra argument: $arg" >&2
                    exit 1
                fi
                PLAN_ARG="$arg"
                ;;
        esac
    done

    if [ -z "$PLAN_ARG" ]; then
        echo "Usage: $0 <plan-dir>" >&2
        exit 1
    fi

    if [ ! -d "$PLAN_ARG" ]; then
        echo "Error: $PLAN_ARG is not a directory" >&2
        exit 1
    fi

    DIR="$(cd "$PLAN_ARG" && pwd)"

    # Walk up from the plan dir to find the project root (.git)
    PROJECT="$DIR"
    while [ ! -d "$PROJECT/.git" ] && [ "$PROJECT" != "/" ]; do
        PROJECT="$(dirname "$PROJECT")"
    done
    if [ "$PROJECT" = "/" ]; then
        echo "Error: no git project root found above $DIR" >&2
        exit 1
    fi

    DEV_ROOT="$(dirname "$PROJECT")"
    PLAN_NAME="$(basename "$DIR")"

    SYSTEM_PROMPT_FILE="$LLM_CONTEXT_PI_DIR/prompts/system-prompt.md"
    if [ ! -f "$SYSTEM_PROMPT_FILE" ]; then
        echo "Error: $SYSTEM_PROMPT_FILE missing" >&2
        exit 1
    fi
    SYSTEM_PROMPT_CONTENT="$(cat "$SYSTEM_PROMPT_FILE")"

    # -------------------------------------------------------------------------
    # Auto-memory system: compute project memory directory
    # -------------------------------------------------------------------------
    MEMORY_PROMPT_FILE="$LLM_CONTEXT_PI_DIR/prompts/memory-prompt.md"
    PROJECT_PATH_ENCODED="$(printf '%s' "$PROJECT" | tr '/' '-')"
    MEMORY_DIR="$HOME/.claude-pi/projects/$PROJECT_PATH_ENCODED/memory"
    MEMORY_INDEX="$MEMORY_DIR/MEMORY.md"
    mkdir -p "$MEMORY_DIR"

    MEMORY_PROMPT_CONTENT=""
    if [ -f "$MEMORY_PROMPT_FILE" ]; then
        MEMORY_PROMPT_CONTENT="$(sed "s|{{MEMORY_DIR}}|$MEMORY_DIR|g" "$MEMORY_PROMPT_FILE")"
    fi

    MEMORY_INDEX_CONTENT=""
    if [ -f "$MEMORY_INDEX" ]; then
        MEMORY_INDEX_CONTENT="$(cat "$MEMORY_INDEX")"
    fi

    while true; do
        PHASE=$(cat "$DIR/phase.md" 2>/dev/null || echo work)
        PROMPT="$(compose_prompt "$PHASE")"
        printf '\n=== %s ===\n' "$PHASE"

        case "$PHASE" in
            work)    PHASE_MODEL="$WORK_MODEL";    PHASE_THINKING="$WORK_THINKING"    ;;
            reflect) PHASE_MODEL="$REFLECT_MODEL"; PHASE_THINKING="$REFLECT_THINKING" ;;
            compact) PHASE_MODEL="$COMPACT_MODEL"; PHASE_THINKING="$COMPACT_THINKING" ;;
            triage)  PHASE_MODEL="$TRIAGE_MODEL";  PHASE_THINKING="$TRIAGE_THINKING"  ;;
            *)
                echo "Error: unknown phase '$PHASE' in $DIR/phase.md" >&2
                exit 1
                ;;
        esac

        PI_ARGS=(--no-session --append-system-prompt "$SYSTEM_PROMPT_CONTENT")
        if [ -n "$PROVIDER" ]; then
            PI_ARGS+=(--provider "$PROVIDER")
        fi
        if [ -n "$PHASE_MODEL" ]; then
            PI_ARGS+=(--model "$PHASE_MODEL")
        fi
        if [ -n "$PHASE_THINKING" ]; then
            PI_ARGS+=(--thinking "$PHASE_THINKING")
        fi

        # Auto-memory injection (phase-dependent):
        # Work phase: full read+write (instructions + index)
        # Headless phases: read-only context (index only)
        if [ "$PHASE" = work ] && [ -n "$MEMORY_PROMPT_CONTENT" ]; then
            PI_ARGS+=(--append-system-prompt "$MEMORY_PROMPT_CONTENT")
        fi
        if [ -n "$MEMORY_INDEX_CONTENT" ]; then
            PI_ARGS+=(--append-system-prompt "## Current Memory Index
$MEMORY_INDEX_CONTENT")
        fi

        # Dry-run escape hatch
        if [ -n "${LLM_CONTEXT_PI_DRYRUN:-}" ]; then
            printf '\n--- DRY RUN: would invoke pi with: ---\n'
            printf '  provider=%s model=%s thinking=%s\n' "$PROVIDER" "$PHASE_MODEL" "$PHASE_THINKING"
            printf '  prompt length=%d chars\n' "${#PROMPT}"
            printf '  prompt head:\n'
            printf '%s' "$PROMPT" | head -5
            printf '\n'
            case "$PHASE" in
                work)    echo reflect > "$DIR/phase.md" ;;
                reflect) echo compact > "$DIR/phase.md" ;;
                compact) echo triage  > "$DIR/phase.md" ;;
                triage)  echo work    > "$DIR/phase.md"
                         rm -f "$DIR/propagation.out.yaml"
                         : > "$DIR/_dryrun_triage_done"
                         ;;
            esac
            NEW_PHASE=$(cat "$DIR/phase.md" 2>/dev/null || echo work)
            if [ -f "$DIR/_dryrun_triage_done" ]; then
                rm -f "$DIR/_dryrun_triage_done"
                printf '\n=== dry run complete — all phases cycled ===\n'
                exit 0
            fi
            continue
        fi

        case "$PHASE" in
            work)
                rm -f "$DIR/latest-session.md"
                if [ -x "$DIR/pre-work.sh" ]; then
                    printf '\n=== pre-work hook ===\n'
                    if ! (cd "$PROJECT" && "$DIR/pre-work.sh"); then
                        echo "Error: $DIR/pre-work.sh failed — aborting cycle" >&2
                        exit 1
                    fi
                fi
                (cd "$PROJECT" && pi "${PI_ARGS[@]}" "$PROMPT")
                ;;
            reflect|compact|triage)
                (cd "$PROJECT" && pi "${PI_ARGS[@]}" --mode json -p "$PROMPT" \
                 | format_pi_stream)
                printf '\n'
                ;;
        esac

        NEW_PHASE=$(cat "$DIR/phase.md" 2>/dev/null || echo work)

        if [ "$PHASE" = "$NEW_PHASE" ]; then
            printf '\n=== %s did not advance phase.md — exiting ===\n' "$PHASE"
            exit 0
        fi

        # Session-log append (work only, guarded on advance).
        if [ "$PHASE" = work ] && [ -s "$DIR/latest-session.md" ]; then
            printf '\n' >> "$DIR/session-log.md"
            cat "$DIR/latest-session.md" >> "$DIR/session-log.md"
        fi

        # Compact-baseline update (compact only, guarded on advance).
        if [ "$PHASE" = compact ]; then
            wc -w < "$DIR/memory.md" 2>/dev/null | awk '{print $1}' > "$DIR/compact-baseline"
        fi

        # Reflect-to-compact relative trigger.
        if [ "$PHASE" = reflect ]; then
            BASELINE=$(cat "$DIR/compact-baseline" 2>/dev/null || echo 0)
            WORDS=$(wc -w < "$DIR/memory.md" 2>/dev/null | awk '{print $1}')
            WORDS=${WORDS:-0}
            if [ "$WORDS" -le $((BASELINE + HEADROOM)) ]; then
                echo triage > "$DIR/phase.md"
            fi
        fi

        # After triage: dispatch cross-plan propagations (if any).
        if [ "$PHASE" = triage ] && [ -f "$DIR/propagation.out.yaml" ]; then
            propagation_count=0
            while IFS=$'\t' read -r p_kind p_target p_summary; do
                if [ -z "${p_target:-}" ]; then
                    continue
                fi
                if [ ! -d "$p_target" ]; then
                    printf '\n=== propagation skip: %s does not exist ===\n' "$p_target" >&2
                    continue
                fi
                propagation_count=$((propagation_count + 1))
                printf '\n=== propagation → %s (%s) ===\n' "$p_target" "$p_kind"

                PROPAGATION_PROMPT="You are receiving a cross-plan propagation from the LLM_CONTEXT_PI system.

Source plan: $DIR
This plan: $p_target
Relationship: the source is your $p_kind

Learning from the source plan:
$p_summary

Read this plan's backlog.md and memory.md at $p_target, decide what (if anything) should be added to backlog.md or updated in memory.md as a result of the learning above, apply the changes using the edit and write tools, and return a one-line summary of what you did (or 'no changes needed' if you determined no update was warranted). Do not commit."

                (cd "$PROJECT" && pi --no-session \
                    --append-system-prompt "$SYSTEM_PROMPT_CONTENT" \
                    ${PROVIDER:+--provider "$PROVIDER"} \
                    ${TRIAGE_MODEL:+--model "$TRIAGE_MODEL"} \
                    --mode json -p "$PROPAGATION_PROMPT" \
                  | format_pi_stream)
                printf '\n'
            done < <(parse_propagation < "$DIR/propagation.out.yaml")

            if [ "$propagation_count" -gt 0 ]; then
                rm -f "$DIR/propagation.out.yaml"
            fi
        fi

        # Post-phase auto-commit. Work phase commits the entire project;
        # headless phases commit only the plan directory.
        if [ "$PHASE" = work ]; then
            (cd "$PROJECT" && git add -A)
        else
            (cd "$PROJECT" && git add "$DIR")
        fi
        if ! (cd "$PROJECT" && git diff --cached --quiet 2>/dev/null); then
            (cd "$PROJECT" && git commit -m "run-plan: $PHASE ($PLAN_NAME)")
        fi
    done
fi
