#!/usr/bin/env bash
#
# Capture the ravel-lite tutorial scenario inside a TestAnyware macOS VM.
#
# Outputs (under docs/captures/ravel-lite-tutorial/):
#   state/        — pulled LLM_STATE/ tree from the VM after the run
#   screens/      — PNG screenshots of TUI moments (chapters 04-05)
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
# as screenshots, not text. Chapter 03 (`ravel-lite create`) is also
# interactive and currently uncaptured — extending the script to handle
# it is the next iteration's job.

set -euo pipefail
IFS=$'\n\t'

readonly REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly CAPTURE_DIR="$REPO_ROOT/docs/captures/ravel-lite-tutorial"
readonly STATE_DIR="$CAPTURE_DIR/state"
readonly SCREENS_DIR="$CAPTURE_DIR/screens"
readonly TRANSCRIPTS_DIR="$CAPTURE_DIR/transcripts"

# Default macOS VM user. Override via env if your golden image differs.
# Used only for the absolute-path argument to `testanyware download`;
# all in-VM commands use ~/... so they read naturally in transcripts.
readonly VM_USER="${VM_USER:-tester}"
readonly EXAMPLE_DIR_ABS="/Users/${VM_USER}/Development/ravel-tutorial-example"
readonly CONFIG_DIR_TILDE="~/.config/ravel-lite"

# Deterministic VM id; passed to `vm start --id` and reused via --vm
# for every subsequent testanyware call. UTC timestamp keeps successive
# runs distinguishable in $XDG_STATE_HOME/testanyware/vms/.
readonly VM_ID="ravel-lite-tutorial-$(date -u +%Y%m%dT%H%M%SZ)"

log() { echo "[$1] ${*:2}"; }
die() { echo "capture: $*" >&2; exit 1; }

cleanup() {
  log TEARDOWN "stopping VM $VM_ID"
  testanyware vm stop "$VM_ID" || true
}
trap cleanup EXIT

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

# vm_run <command>: fire-and-forget exec, no transcript capture.
# For prep work (mkdir, file writes) where stdout is not part of any
# tutorial chapter.
vm_run() {
  testanyware exec --vm "$VM_ID" "$1"
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
      "export RAVEL_LITE_CONFIG=$CONFIG_DIR_TILDE; $cmd" 2>&1 || true
  } | tee "$out"
}

screenshot_at() {
  local label="$1"
  log CAPTURE_SCREENS "screenshot $label"
  testanyware screenshot --vm "$VM_ID" -o "$SCREENS_DIR/${label}.png"
}

install_ravel_lite() {
  log INSTALL "brew install linkuistics/taps/ravel-lite"
  vm_run "brew tap linkuistics/taps && brew install ravel-lite"
  transcript_at "01-version" "ravel-lite version"
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
  transcript_at "02-git-init" \
    "cd ~/Development/ravel-tutorial-example && git init"
  # Scaffold README and reading-list.md; no useful stdout to capture.
  vm_run "cd ~/Development/ravel-tutorial-example \
    && printf '%s\n' '# Reading list' > README.md \
    && mkdir -p notes \
    && touch reading-list.md"
  transcript_at "02-git-commit" \
    "cd ~/Development/ravel-tutorial-example && git add . && git commit -m 'Initial scaffold'"
  transcript_at "02-projects-add" \
    "ravel-lite state projects add --path ~/Development/ravel-tutorial-example"
  transcript_at "02-projects-list-populated" \
    "ravel-lite state projects list"
}

# scenario_run drives the chapter 04-05 TUI flow. TUI stdout lives
# inside the VM's GUI Terminal, so capture is via screenshots (and the
# downloaded LLM_STATE tree at the end), not transcript_at.
scenario_run() {
  log SCENARIO_RUN "opening Terminal in VM"
  vm_run "open -a Terminal"
  testanyware find-text --vm "$VM_ID" "\$" --timeout 15 >/dev/null

  log SCENARIO_RUN "driving 'ravel-lite create'"
  testanyware input type --vm "$VM_ID" \
    "cd ~/Development/ravel-tutorial-example && ravel-lite create"
  testanyware input key --vm "$VM_ID" return
  testanyware find-text --vm "$VM_ID" "plan name" --timeout 15 >/dev/null
  screenshot_at "01-create-plan-name-prompt"
  testanyware input type --vm "$VM_ID" "main"
  testanyware input key --vm "$VM_ID" return

  log SCENARIO_RUN "driving 'ravel-lite run'"
  testanyware input type --vm "$VM_ID" "ravel-lite run main"
  testanyware input key --vm "$VM_ID" return
  testanyware find-text --vm "$VM_ID" "phase: work" --timeout 30 >/dev/null
  screenshot_at "02-tui-phase-work"
}

capture_state() {
  log CAPTURE_STATE "pulling LLM_STATE/ from VM"
  testanyware download --vm "$VM_ID" \
    "${EXAMPLE_DIR_ABS}/LLM_STATE" "$STATE_DIR"
}

main() {
  preflight
  vm_lifecycle_start
  install_ravel_lite
  capture_chapter01_transcripts
  capture_chapter02_transcripts
  scenario_run
  capture_state
  log MAIN "capture complete; outputs in $CAPTURE_DIR"
}

main "$@"
