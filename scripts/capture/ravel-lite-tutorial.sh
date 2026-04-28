#!/usr/bin/env bash
#
# Capture the ravel-lite tutorial scenario inside a TestAnyware macOS VM.
#
# Outputs (under docs/captures/ravel-lite-tutorial/):
#   state/        — pulled LLM_STATE/ tree from the VM after the run
#   screens/      — PNG screenshots of TUI / interactive moments
#                   (chapter 03 create conversation + chapters 04-05 run TUI)
#   transcripts/  — per-command stdout+stderr captures, paste-ready into
#                   the [source,bash] / [source,console] blocks under
#                   docs/tutorial/01..05*.adoc
#
# Prerequisite: ravel-lite formula must be live in Linkuistics/homebrew-taps
# (see scripts/release-build.sh + scripts/release-publish.sh).
#
# Section markers ([STEP-NAME]) on each log line let an LLM driver
# correlate captures and outputs to script phases.
#
# Endpoint plumbing
# -----------------
# testanyware 0.2.0's exec / screenshot / find-text / download / input
# subcommands all accept --vm <id>; the CLI resolves <id> to its own
# per-VM spec at $XDG_STATE_HOME/testanyware/vms/<id>.json. So this
# script generates a deterministic VM id locally, passes it to
# `vm start --id`, and reuses it via --vm thereafter — there is no
# `vm list` / VNC-or-agent-endpoint discovery step. (Earlier drafts
# parsed `vm list --format json`, a flag testanyware does not support.)
#
# Headless vs TUI capture
# -----------------------
# Chapters 01-02 cover headless commands (brew, ravel-lite version,
# init, state projects ...). Their stdout is reachable by the local
# script over `testanyware exec`, so transcript_at writes a verbatim
# transcript per command. Chapters 04-05 cover the TUI run flow whose
# output lives inside the VM's GUI Terminal; those moments are captured
# as screenshots, not text.
#
# Chapter 03 (`ravel-lite create`) is interactive and hybrid:
#   - The four-question scope conversation is paraphrased by claude per
#     run, so byte-accurate text capture is brittle. We drive it with
#     `testanyware input type` against pre-staged response files in
#     `responses/03-*.txt` (mirroring the chapter's illustrative
#     responses verbatim) and screenshot each prompt moment for
#     visual capture.
#   - Once claude exits, the resulting plan files are deterministic
#     enough to capture as text via transcript_at (`ls`, `state
#     backlog list`, `state memory list`).
# A marker `echo` after the create call is the completion signal — it
# only appears in the GUI Terminal once claude has returned control to
# the shell, so the post-create transcripts are guaranteed to read
# real on-disk state.
#
# Flags
# -----
#   --from <step>     Skip every chapter group before <step>. Steps are
#                     `chapter01` … `chapter04`, plus `final`. `--from
#                     chapter04` requires the VM to already have
#                     ravel-lite + claude installed and a created plan
#                     on disk; pair with `--vm-id <id>` to reuse a VM
#                     from a prior `--keep-vm` run.
#   --vm-id <id>      Reuse an existing testanyware VM by id; skips
#                     `vm_lifecycle_start` and disables the auto-
#                     teardown trap. Pair with `--from` for cheap
#                     chapter04-05 iteration.
#   --keep-vm         Skip the auto-teardown trap so the VM survives
#                     for a follow-up `--vm-id` run. Only meaningful
#                     when the script started the VM itself (i.e.
#                     `--vm-id` was NOT provided).

set -euo pipefail
IFS=$'\n\t'

readonly REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly CAPTURE_DIR="$REPO_ROOT/docs/captures/ravel-lite-tutorial"
readonly STATE_DIR="$CAPTURE_DIR/state"
readonly SCREENS_DIR="$CAPTURE_DIR/screens"
readonly TRANSCRIPTS_DIR="$CAPTURE_DIR/transcripts"
readonly RESPONSES_DIR="$REPO_ROOT/scripts/capture/responses"

# Default macOS VM user. Override via env if your golden image differs.
# The current testanyware-golden-macos-tahoe image ships with `admin`
# as the default GUI user; a previous draft of this script assumed
# `tester` and silently injected credentials into the wrong keychain.
# Used for the absolute-path argument to `testanyware download` and
# for the keychain account when injecting Claude OAuth credentials.
readonly VM_USER="${VM_USER:-admin}"
readonly EXAMPLE_DIR_ABS="/Users/${VM_USER}/Development/ravel-tutorial-example"
readonly CONFIG_DIR_TILDE="~/.config/ravel-lite"

# ── Flag parsing ────────────────────────────────────────────────────────────
#
# Set before VM_ID resolution because `--vm-id` overrides the auto-
# generated id and toggles VM lifecycle ownership.
FROM=""
VM_ID_OVERRIDE=""
KEEP_VM=0
while (( $# > 0 )); do
  case "$1" in
    --from)
      [[ $# -ge 2 ]] || { echo "capture: --from needs an argument" >&2; exit 1; }
      FROM="$2"; shift 2 ;;
    --vm-id)
      [[ $# -ge 2 ]] || { echo "capture: --vm-id needs an argument" >&2; exit 1; }
      VM_ID_OVERRIDE="$2"; shift 2 ;;
    --keep-vm)
      KEEP_VM=1; shift ;;
    -h|--help)
      cat <<'USAGE'
Usage: ravel-lite-tutorial.sh [--from <step>] [--vm-id <id>] [--keep-vm]

Captures the ravel-lite tutorial scenario in a TestAnyware macOS VM.
Outputs land under docs/captures/ravel-lite-tutorial/.

Flags:
  --from <step>   Skip every chapter group before <step>. Steps:
                  chapter01, chapter02, chapter03, chapter04, final.
                  Pair with --vm-id when starting after chapter03 —
                  the VM needs ravel-lite + claude installed and a
                  created plan on disk.
  --vm-id <id>    Reuse an existing VM; skips `vm start` and disables
                  the auto-teardown trap.
  --keep-vm       Skip auto-teardown so the VM survives for a
                  follow-up --vm-id run. Only meaningful when the
                  script started the VM itself.

Iteration pattern for chapter04-05 captures:

  # First run: complete chapters 01-03, keep the VM alive.
  ./ravel-lite-tutorial.sh --keep-vm

  # Iterate on chapter04-05 against the same VM:
  ./ravel-lite-tutorial.sh --vm-id <id-from-prior-run> --from chapter04

USAGE
      exit 0 ;;
    *)
      echo "capture: unknown flag: $1" >&2; exit 1 ;;
  esac
done

readonly VALID_STEPS=(chapter01 chapter02 chapter03 chapter04 final)
if [[ -n "$FROM" ]]; then
  found=0
  for step in "${VALID_STEPS[@]}"; do
    [[ "$step" == "$FROM" ]] && { found=1; break; }
  done
  if (( ! found )); then
    echo "capture: --from value '$FROM' is not a known step; allowed: $(IFS=,; echo "${VALID_STEPS[*]}" | tr , ' ')" >&2
    exit 1
  fi
  if [[ "$FROM" != "chapter01" && -z "$VM_ID_OVERRIDE" ]]; then
    echo "capture: --from $FROM without --vm-id starts a fresh VM that lacks the prerequisite state" >&2
    echo "        from earlier chapters; pair with --vm-id <id> from a prior --keep-vm run." >&2
    exit 1
  fi
fi

# Deterministic VM id; passed to `vm start --id` and reused via --vm
# for every subsequent testanyware call. UTC timestamp keeps successive
# runs distinguishable in $XDG_STATE_HOME/testanyware/vms/.
if [[ -n "$VM_ID_OVERRIDE" ]]; then
  readonly VM_ID="$VM_ID_OVERRIDE"
  readonly OWN_VM=0
else
  readonly VM_ID="ravel-lite-tutorial-$(date -u +%Y%m%dT%H%M%SZ)"
  readonly OWN_VM=1
fi

log() { echo "[$1] ${*:2}"; }
die() { echo "capture: $*" >&2; exit 1; }

cleanup() {
  if (( OWN_VM == 0 )); then
    log TEARDOWN "VM $VM_ID was supplied via --vm-id; leaving it running"
    return
  fi
  if (( KEEP_VM )); then
    log TEARDOWN "VM $VM_ID kept alive (--keep-vm); resume with: --vm-id $VM_ID --from <step>"
    return
  fi
  log TEARDOWN "stopping VM $VM_ID"
  testanyware vm stop "$VM_ID" || true
}
trap cleanup EXIT

# ── Step gating ─────────────────────────────────────────────────────────────
#
# `maybe_run <step-name> <fn>` only invokes <fn> if we have reached or
# passed the requested `--from` step. The first call whose step matches
# the flag flips STARTED to 1 and every subsequent call runs.
#
# Step names group whole chapters: `chapter01`, `chapter02`, `chapter03`,
# `chapter04`, `final`. Granular per-function gating is over-flexible —
# the iteration story is "skip earlier chapters", not "skip individual
# setup steps".
STARTED=0
maybe_run() {
  local step_name="$1"; shift
  if (( STARTED == 0 )) && [[ -z "$FROM" || "$step_name" == "$FROM" ]]; then
    STARTED=1
  fi
  if (( STARTED )); then
    "$@"
  else
    log SKIP "$step_name: skipped (--from $FROM)"
  fi
}

preflight() {
  log PREFLIGHT "checking dependencies"
  command -v testanyware >/dev/null || die "testanyware not on PATH"
  mkdir -p "$STATE_DIR" "$SCREENS_DIR" "$TRANSCRIPTS_DIR"
}

vm_lifecycle_start() {
  log VM_LIFECYCLE "starting macOS VM $VM_ID (1920x1080)"
  testanyware vm start \
    --platform macos \
    --display 1920x1080 \
    --id "$VM_ID" >/dev/null
  log VM_LIFECYCLE "VM ready; subsequent commands target --vm $VM_ID"
}

# vm_shell_prelude is prepended to every `testanyware exec` command so
# brew-installed binaries (brew itself, ravel-lite once installed) and
# user-local installs (~/.local/bin/claude) are on PATH. testanyware
# exec runs `/bin/bash -c` non-interactively and bypasses ~/.zprofile,
# which is where macOS Homebrew normally hooks PATH; without this
# prelude, "brew: command not found" hits on the very first install
# step.
readonly VM_SHELL_PRELUDE='eval "$(/opt/homebrew/bin/brew shellenv)"; export PATH="$HOME/.local/bin:$PATH";'

# vm_run <command>: fire-and-forget exec, no transcript capture.
# For prep work (mkdir, file writes) where stdout is not part of any
# tutorial chapter.
#
# We tolerate non-zero exit because testanyware exec has a fixed 30s
# response timeout that fires on the agent channel even when the in-VM
# command itself completed quickly. The `|| true` is wrong for
# correctness-sensitive prep, but every prep step we run is also
# observable later (a missing directory or unwritten file will surface
# when capture_chapter01_transcripts or chapter 02 try to operate on
# it). Keep prep commands SHORT so a real failure has less surface.
vm_run() {
  testanyware exec --vm "$VM_ID" "$VM_SHELL_PRELUDE $1" || true
}

# transcript_at <label> <command>
#
# Runs <command> on the VM with RAVEL_LITE_CONFIG pre-set, and captures
# the combined stdout+stderr to $TRANSCRIPTS_DIR/<label>.txt. The
# transcript leads with "$ <command>" so the file drops paste-ready
# into a [source,console] block. The env-var setup is appended to the
# wire-level command but kept out of the displayed line, matching the
# chapter's assumption that the user has set RAVEL_LITE_CONFIG in
# their shell profile.
#
# We tolerate non-zero exit codes because some captured commands are
# expected to fail (e.g. the chapter 01 "init refuses without --force"
# example). The transcript records whatever the command produced.
transcript_at() {
  local label="$1" cmd="$2"
  local out="$TRANSCRIPTS_DIR/${label}.txt"
  log CAPTURE_TRANSCRIPTS "$label: $cmd"
  {
    printf '$ %s\n' "$cmd"
    testanyware exec --vm "$VM_ID" \
      "$VM_SHELL_PRELUDE export RAVEL_LITE_CONFIG=$CONFIG_DIR_TILDE; $cmd" 2>&1 || true
  } | tee "$out"
  # Strip testanyware's own timeout marker — it's an agent-channel
  # response-collection timeout, not an in-VM command failure, and
  # leaving it in the captured transcript would mislead readers into
  # thinking their command timed out.
  if [[ -f "$out" ]]; then
    sed -i.bak '/^Process timed out after/d' "$out"
    rm -f "${out}.bak"
  fi
}

screenshot_at() {
  local label="$1"
  log CAPTURE_SCREENS "screenshot $label"
  testanyware screenshot --vm "$VM_ID" -o "$SCREENS_DIR/${label}.png"
}

# type_lines <response-file>
#
# Types each line of <response-file> via `testanyware input type`,
# pressing return between lines. We do not rely on `input type`
# carrying through embedded newlines — line-then-return is the
# predictable contract claude's prompt sees.
type_lines() {
  local response_file="$1"
  local line
  while IFS= read -r line; do
    testanyware input type --vm "$VM_ID" "$line"
    testanyware input key --vm "$VM_ID" return
  done <"$response_file"
}

# drive_scope_question <label> <wait-text>
#
# Waits for <wait-text> to appear (claude's paraphrased prompt for
# this scope question), screenshots the moment, then types the
# matching response file. Response files at $RESPONSES_DIR/03-<label>.txt
# mirror the illustrative responses in docs/tutorial/03-creating-a-plan.adoc.
#
# Wait substrings are the most stable topic word for each prompt and
# may need tuning on the first live run if claude paraphrases past
# them.
drive_scope_question() {
  local label="$1" wait_text="$2"
  testanyware find-text --vm "$VM_ID" "$wait_text" --timeout 60 >/dev/null
  screenshot_at "03-conversation-${label}-prompt"
  type_lines "$RESPONSES_DIR/03-${label}.txt"
}

install_ravel_lite() {
  log INSTALL "brew install linkuistics/taps/ravel-lite"
  vm_run "brew tap linkuistics/taps && brew install ravel-lite"
  transcript_at "01-version" "ravel-lite version"
}

# install_claude_code installs the claude-code native binary via the
# official installer. The binary lands at ~/.local/bin/claude.
# We append ~/.local/bin to PATH in ~/.zshenv so the interactive
# Terminal.app session in chapter 03 finds it without us having to
# type the absolute path.
install_claude_code() {
  log INSTALL "installing claude-code (official installer)"
  vm_run "curl -fsSL https://claude.ai/install.sh | bash"
  vm_run "grep -q '.local/bin' ~/.zshenv 2>/dev/null \
    || echo 'export PATH=\$HOME/.local/bin:\$PATH' >> ~/.zshenv"
}

# transfer_claude_auth lifts the Claude Code OAuth credential from the
# host's macOS Keychain and injects it into the VM's tester-user
# keychain. HARNESS-ONLY: tutorial readers will use `claude login`
# interactively; this codepath only exists so the capture script can
# drive a fully-non-interactive end-to-end run on a fresh VM.
#
# The credential is a JSON blob containing OAuth tokens. We never log
# its contents; the only filesystem touch is a private tempfile on the
# host (deleted immediately after upload) and /tmp/claude-creds.json
# inside the ephemeral VM (deleted after injection; VM disk is destroyed
# on `vm stop`).
#
# OAuth tokens themselves are not machine-bound — only the at-rest
# Keychain encryption is — so the access/refresh tokens lifted here are
# valid for use by claude inside the VM until the next refresh rotation.
transfer_claude_auth() {
  log AUTH "transferring claude OAuth credentials (HARNESS-ONLY)"
  local creds_file
  creds_file="$(mktemp -t claude-creds.XXXXXX)"
  trap 'rm -f "${creds_file:-}"' RETURN
  security find-generic-password \
    -s "Claude Code-credentials" -a "$USER" -w >"$creds_file"
  testanyware upload --vm "$VM_ID" "$creds_file" "/tmp/claude-creds.json"
  vm_run "security add-generic-password -U \
    -s 'Claude Code-credentials' \
    -a '$VM_USER' \
    -w \"\$(cat /tmp/claude-creds.json)\""
  vm_run "rm -f /tmp/claude-creds.json"
}

# pretrust_project pre-populates ~/.claude.json AND
# ~/.claude/settings.json in the VM so the chapter 03 create session
# does not stop on any of three modals that have historically caused
# TUI hangs:
#
#   1. First-run onboarding modal — bypassed via hasCompletedOnboarding
#   2. Per-project trust modal — bypassed via per-project
#      hasTrustDialogAccepted (also referenced in the still-open
#      diagnose-claude-tui-invisible-after-work-phase-banner task)
#   3. Per-tool permission prompts — bypassed via permissions.defaultMode
#      = bypassPermissions, which lets claude execute Bash/Read/Write
#      without asking for human approval. This is the "dangerously
#      skip permissions" equivalent in settings form, scoped to this
#      throwaway VM only. Tutorial readers will use the standard
#      interactive permission flow on first invocation.
pretrust_project() {
  log AUTH "pre-trusting tutorial project + bypassing permission prompts"
  vm_run "cat > ~/.claude.json <<'JSON'
{
  \"hasCompletedOnboarding\": true,
  \"projects\": {
    \"$EXAMPLE_DIR_ABS\": {
      \"hasTrustDialogAccepted\": true
    }
  }
}
JSON"
  vm_run "mkdir -p ~/.claude"
  vm_run "cat > ~/.claude/settings.json <<'JSON'
{
  \"permissions\": {
    \"defaultMode\": \"bypassPermissions\"
  },
  \"skipDangerousModePermissionPrompt\": true
}
JSON"
}

# setup_shell_env writes RAVEL_LITE_CONFIG into ~/.zshenv so the
# interactive Terminal.app session in chapter 03 inherits it. The
# headless transcript_at captures set the env var inline per-command,
# so they don't depend on this; only the GUI Terminal flow does.
setup_shell_env() {
  log SETUP "writing RAVEL_LITE_CONFIG to ~/.zshenv"
  vm_run "grep -q RAVEL_LITE_CONFIG ~/.zshenv 2>/dev/null \
    || echo 'export RAVEL_LITE_CONFIG=$CONFIG_DIR_TILDE' >> ~/.zshenv"
}

capture_chapter01_transcripts() {
  log CAPTURE_TRANSCRIPTS "chapter 01: install-and-config"
  transcript_at "01-init-fresh"          "ravel-lite init $CONFIG_DIR_TILDE"
  transcript_at "01-init-refuse"         "ravel-lite init $CONFIG_DIR_TILDE"
  transcript_at "01-init-force"          "ravel-lite init $CONFIG_DIR_TILDE --force"
  transcript_at "01-ls-phases"           "ls $CONFIG_DIR_TILDE/phases/"
  transcript_at "01-projects-list-empty" "ravel-lite state projects list"
}

capture_chapter02_transcripts() {
  log CAPTURE_TRANSCRIPTS "chapter 02: the-project"
  vm_run "mkdir -p ~/Development/ravel-tutorial-example"
  # Pre-set git identity so the first commit's transcript doesn't
  # include git's "your name was configured automatically" warning,
  # which is one-time first-run noise unrelated to the tutorial.
  vm_run "git config --global user.name 'Tutorial User'"
  vm_run "git config --global user.email 'tutorial@example.com'"
  transcript_at "02-git-init" \
    "cd ~/Development/ravel-tutorial-example && git init"
  # Scaffold README and reading-list.md; no useful stdout to capture.
  # Each prep step is its own vm_run so a 30s agent-channel timeout on
  # one short command doesn't cascade across the whole multi-step
  # scaffold; chained-with-&& we hit the timeout once for the entire
  # block even though each step is sub-second.
  vm_run "mkdir -p ~/Development/ravel-tutorial-example/notes"
  vm_run "printf '%s\\n' '# Reading list' > ~/Development/ravel-tutorial-example/README.md"
  vm_run "touch ~/Development/ravel-tutorial-example/reading-list.md"
  transcript_at "02-git-commit" \
    "cd ~/Development/ravel-tutorial-example && git add . && git commit -m 'Initial scaffold'"
  transcript_at "02-projects-add" \
    "ravel-lite state projects add --path ~/Development/ravel-tutorial-example"
  transcript_at "02-projects-list-populated" \
    "ravel-lite state projects list"
}

# capture_chapter03_create_session drives the `ravel-lite create
# LLM_STATE/main` session in the VM's GUI Terminal. The previous
# strict per-question driver (drive_scope_question waiting on hand-
# picked substrings) was brittle for two compounding reasons:
#
#   1. The `defaults/create-plan.md` §1 template asks SIX scope
#      questions, not the four the earlier mechanism assumed; the
#      response files (responses/03-{purpose,project,backlog,
#      memory-seed}.txt) covered only four.
#   2. Claude paraphrases each question, so substring matches like
#      "plan for" / "project" / "backlog" / "memory" hit false
#      positives and false negatives on different runs.
#
# The current driver leans on the create-plan template's escape
# hatch: "If the description is a concrete problem ... treat it as
# the plan's initial task. A single-task plan is a valid plan." We
# type ONE comprehensive description covering purpose + tasks +
# categories + memory hints, hit return, and let claude proceed
# directly to plan-writing without a multi-turn dialogue.
#
# Screenshots are taken at fixed intervals (no find-text dependency
# on prompt characters; macOS zsh defaults to '%', not the '$' an
# earlier draft assumed). The marker `echo CHAPTER_03_CREATE_DONE`
# is the only deterministic completion signal — it appears in the
# Terminal only after claude has exited and the shell prompt is back.
capture_chapter03_create_session() {
  log CAPTURE_SCREENS "chapter 03: creating-a-plan (single-shot description)"
  vm_run "open -a Terminal"
  # Sleep instead of find-text on the prompt char (which differs
  # between bash '$' and zsh '%' and between user-customised
  # prompts); 5s is plenty for Terminal.app to appear with a fresh
  # interactive shell on this golden image.
  sleep 5
  screenshot_at "03-terminal-opened"

  log SCENARIO_RUN "invoking 'ravel-lite create LLM_STATE/main'"
  testanyware input type --vm "$VM_ID" \
    "cd ~/Development/ravel-tutorial-example && ravel-lite create LLM_STATE/main"
  testanyware input key --vm "$VM_ID" return

  # Wait for claude to start and present its initial prompt.
  sleep 20
  screenshot_at "03-conversation-claude-prompt"

  # Type the comprehensive single-shot response. Lines hit return
  # individually (input type does not carry through embedded
  # newlines reliably).
  type_lines "$RESPONSES_DIR/03-comprehensive.txt"

  # Poll for the populated backlog file via the agent channel rather
  # than relying on a screen-side marker. The plan dir state is the
  # actual completion signal we care about — when backlog.yaml has
  # tasks in it, claude has used the state CLI successfully and the
  # plan files are on disk regardless of what the GUI Terminal shows.
  log SCENARIO_RUN "polling for populated backlog (max ~4 minutes)"
  local backlog_path="~/Development/ravel-tutorial-example/LLM_STATE/main/backlog.yaml"
  local i
  for i in $(seq 1 24); do
    sleep 10
    if testanyware exec --vm "$VM_ID" \
        "$VM_SHELL_PRELUDE test -s $backlog_path && grep -q '^- id:' $backlog_path" \
        >/dev/null 2>&1; then
      log SCENARIO_RUN "backlog populated after ~${i}0s; capturing screenshot"
      break
    fi
    if (( i == 24 )); then
      log SCENARIO_RUN "timed out waiting for populated backlog; continuing anyway"
    fi
    if (( i % 6 == 0 )); then
      screenshot_at "03-conversation-poll-${i}"
    fi
  done
  screenshot_at "03-conversation-completion"
}

# capture_chapter03_post_state captures the deterministic post-create
# state via transcript_at. These three commands feed the chapter's
# "Inspecting what create produced" section.
capture_chapter03_post_state() {
  log CAPTURE_TRANSCRIPTS "chapter 03: post-create state"
  local plan_dir="~/Development/ravel-tutorial-example/LLM_STATE/main"
  transcript_at "03-ls-plan-dir" "ls $plan_dir/"
  transcript_at "03-backlog-list" \
    "ravel-lite state backlog list $plan_dir --format markdown"
  transcript_at "03-memory-list" \
    "ravel-lite state memory list $plan_dir"
}

# scenario_run drives one full ravel-lite cycle (chapters 04-05)
# against the plan created in chapter 03. The TUI lives in the VM's
# GUI Terminal — the work-phase prompt, phase banners, and per-phase
# progress indicators are all screen-only — so most capture is via
# screenshots. Deterministic post-cycle state (git log, session log,
# memory list, backlog) is captured as text via transcript_at.
#
# Find-text targets:
#   "WORK", "ANALYSE", "REFLECT", "TRIAGE" — phase banners
#       (per src/format.rs::phase_info; uppercase, NOT "phase: work"
#       which only appears in YAML files).
#   "Proceed to next work phase?" — inter-cycle prompt
#       (per src/phase_loop.rs:463; verbatim string).
#
# Completion signal: phase.md returns to `work` AND a new
# `save-work-baseline` commit appears in `git log`. Polled via the
# agent channel rather than a screen marker so a stuck TUI doesn't
# fool us into thinking the cycle finished.
scenario_run() {
  local plan_dir="${EXAMPLE_DIR_ABS}/LLM_STATE/main"
  local project_dir="${EXAMPLE_DIR_ABS}"

  log SCENARIO_RUN "starting one full ravel-lite cycle (chapters 04-05)"

  # Snapshot the pre-cycle phase.md so the chapter prose can show the
  # starting state (`work`) the first cycle assumes.
  transcript_at "04-phase-md-pre" \
    "cat $plan_dir/phase.md"

  # Capture the pre-run HEAD so the completion poll can detect when HEAD
  # has actually advanced. Necessary on iterative `--from chapter04` re-
  # runs: the prior cycle's tail is already a save-work-baseline commit,
  # so a string-only match would short-circuit before the new cycle
  # starts.
  # The 30s testanyware-exec agent-channel timeout fires here on a cold
  # agent (verified empirically). Tolerate failure: an empty pre_run_sha
  # is still useful — the polling check requires save-work-baseline
  # at HEAD, which won't be true until the cycle actually completes,
  # so we cannot short-circuit even with the SHA-difference predicate
  # vacuously satisfied.
  local pre_run_sha=""
  pre_run_sha=$(testanyware exec --vm "$VM_ID" \
    "$VM_SHELL_PRELUDE cd $project_dir && git rev-parse HEAD" 2>/dev/null \
    | tr -d '[:space:]') || pre_run_sha=""
  log SCENARIO_RUN "pre-run HEAD: ${pre_run_sha:-<unknown>}"

  log SCENARIO_RUN "invoking 'ravel-lite run' in the GUI Terminal"
  testanyware input type --vm "$VM_ID" \
    "ravel-lite run $plan_dir"
  testanyware input key --vm "$VM_ID" return

  # The work-phase banner appears once the phase loop has read phase.md
  # and dispatched. 60s gives ample headroom on a cold VM where the
  # ravel-lite binary may still be paging in.
  testanyware find-text --vm "$VM_ID" "WORK" --timeout 60 >/dev/null
  screenshot_at "04-work-banner"

  # The work agent (claude) takes a moment to spin up and emit its
  # initial prompt asking which task to pick. No deterministic find-
  # text target — claude's wording shifts run-to-run — so a fixed
  # delay screenshot is the pragmatic capture point.
  sleep 25
  screenshot_at "04-work-prompt"

  # Drive the work phase with a single-line task choice. The response
  # file mirrors the chapter's illustrative selection ("the
  # distributed-systems entries"); edit responses/04-task-choice.txt
  # to tune for the seeded backlog claude actually wrote in chapter 03.
  type_lines "$RESPONSES_DIR/04-task-choice.txt"

  # The cycle now proceeds unattended through analyse-work, git-commit-
  # work, reflect, git-commit-reflect, (dream skipped on first cycle),
  # triage, git-commit-triage. Poll for completion every 10s; cap at
  # ~20 minutes which covers a leisurely cycle on a cold VM.
  log SCENARIO_RUN "polling for cycle completion (max ~20 minutes)"
  local i
  for i in $(seq 1 120); do
    sleep 10
    if testanyware exec --vm "$VM_ID" \
        "$VM_SHELL_PRELUDE cd $project_dir \
          && [ \"\$(cat $plan_dir/phase.md)\" = work ] \
          && [ \"\$(git rev-parse HEAD)\" != \"$pre_run_sha\" ] \
          && git log --oneline | head -1 | grep -q save-work-baseline" \
        >/dev/null 2>&1; then
      log SCENARIO_RUN "cycle complete after ~${i}0s"
      break
    fi
    # Periodic mid-cycle screenshots. Six iterations = ~1 minute apart;
    # enough to land at least one screenshot per phase without flooding
    # the captures directory. Labelled by elapsed deciseconds so the
    # filename ordering matches real time.
    if (( i % 6 == 0 )); then
      screenshot_at "04-tui-cycle-${i}0s"
    fi
    if (( i == 120 )); then
      log SCENARIO_RUN "timed out waiting for cycle completion; capturing anyway"
    fi
  done

  # The TUI is now showing 'Proceed to next work phase?'. Screenshot,
  # then answer 'n' to exit cleanly so the next chapter prose can
  # describe the close-the-loop story.
  testanyware find-text --vm "$VM_ID" "Proceed to next work phase" --timeout 30 >/dev/null || true
  screenshot_at "04-proceed-prompt"
  testanyware input type --vm "$VM_ID" "n"
  testanyware input key --vm "$VM_ID" return
  sleep 3
  screenshot_at "04-loop-exited"

  # Deterministic post-cycle state for the chapter prose. Each
  # transcript drops paste-ready into a [source,console] block.
  log CAPTURE_TRANSCRIPTS "chapter 04-05: post-cycle deterministic state"
  transcript_at "04-git-log-post-cycle" \
    "cd $project_dir && git log --oneline -10"
  transcript_at "04-session-log-latest" \
    "ravel-lite state session-log show-latest $plan_dir"
  transcript_at "05-memory-after-reflect" \
    "ravel-lite state memory list $plan_dir"
  transcript_at "05-backlog-after-triage" \
    "ravel-lite state backlog list $plan_dir --format markdown"
  transcript_at "05-phase-md-post" \
    "cat $plan_dir/phase.md"
}

capture_state() {
  log CAPTURE_STATE "pulling LLM_STATE/ from VM"
  testanyware download --vm "$VM_ID" \
    "${EXAMPLE_DIR_ABS}/LLM_STATE" "$STATE_DIR"
}

main() {
  preflight
  if (( OWN_VM )); then
    vm_lifecycle_start
  else
    log VM_LIFECYCLE "reusing supplied VM $VM_ID (skipping vm start)"
  fi

  maybe_run chapter01 install_ravel_lite
  maybe_run chapter01 capture_chapter01_transcripts
  maybe_run chapter02 capture_chapter02_transcripts
  maybe_run chapter03 install_claude_code
  maybe_run chapter03 transfer_claude_auth
  maybe_run chapter03 pretrust_project
  maybe_run chapter03 setup_shell_env
  maybe_run chapter03 capture_chapter03_create_session
  maybe_run chapter03 capture_chapter03_post_state
  maybe_run chapter04 scenario_run
  maybe_run final     capture_state

  log MAIN "capture complete; outputs in $CAPTURE_DIR"
}

main "$@"
