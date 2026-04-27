#!/usr/bin/env bash
#
# Verify the prerequisites for running the release pipeline. Checks
# only — never installs anything. Emits a punch list of every missing
# item with the README's remediation command, then exits non-zero if
# anything is missing.
#
# Designed to run twice: standalone before committing to a release
# attempt, and (cheaply) as the first step of release-build.sh so the
# build fails fast on a misconfigured machine instead of mid-toolchain.

set -euo pipefail
IFS=$'\n\t'
trap 'echo "release-doctor: error on line $LINENO" >&2' ERR

# Must stay in sync with release-build.sh's TARGETS array. The native
# host target is included even though rustup typically auto-installs it
# — explicit checks survive a partial rustup setup.
readonly TARGETS=(
  aarch64-apple-darwin
  x86_64-apple-darwin
  aarch64-unknown-linux-gnu
  x86_64-unknown-linux-gnu
)

failed=0

mark_pass() {
  echo "  ✓ $*"
}

mark_fail() {
  echo "  ✗ $*"
  failed=1
}

remediation() {
  echo "      remediation: $*"
}

check_zig() {
  if ! command -v zig >/dev/null 2>&1; then
    mark_fail "zig: not on PATH"
    remediation "brew install zig"
    return
  fi
  local version
  version="$(zig version 2>/dev/null || echo unknown)"
  mark_pass "zig: $version"
}

check_cargo_zigbuild() {
  if cargo zigbuild --help >/dev/null 2>&1; then
    mark_pass "cargo-zigbuild: available"
  else
    mark_fail "cargo-zigbuild: not installed"
    remediation "cargo install cargo-zigbuild"
  fi
}

check_rustup_targets() {
  if ! command -v rustup >/dev/null 2>&1; then
    mark_fail "rustup: not on PATH"
    remediation "install rustup from https://rustup.rs/"
    return
  fi
  local installed
  installed="$(rustup target list --installed)"
  local target
  for target in "${TARGETS[@]}"; do
    if grep -qx "$target" <<<"$installed"; then
      mark_pass "rustup target: $target"
    else
      mark_fail "rustup target: $target not installed"
      remediation "rustup target add $target"
    fi
  done
}

check_gh_auth() {
  if ! command -v gh >/dev/null 2>&1; then
    mark_fail "gh: not installed"
    remediation "brew install gh && gh auth login"
    return
  fi
  if gh auth status >/dev/null 2>&1; then
    mark_pass "gh: authenticated"
  else
    mark_fail "gh: not authenticated"
    remediation "gh auth login"
  fi
}

main() {
  echo "release-doctor: checking release prerequisites"
  echo

  check_zig
  check_cargo_zigbuild
  check_rustup_targets
  check_gh_auth

  echo
  if (( failed == 0 )); then
    echo "release-doctor: all prerequisites met"
    exit 0
  fi
  echo "release-doctor: missing prerequisites — fix the items marked above" >&2
  exit 1
}

main "$@"
