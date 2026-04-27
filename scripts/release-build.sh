#!/usr/bin/env bash
#
# Build per-target tarballs for the current git tag and render a Homebrew
# formula from scripts/templates/ravel-lite.rb.tmpl.
#
# Output: target/dist/
#   ravel-lite-v<ver>-<target>.tar.xz   (one per target)
#   ravel-lite.rb                       (rendered formula)
#
# After this completes, inspect target/dist/ and run release-publish.sh.

set -euo pipefail
IFS=$'\n\t'

readonly REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly DIST_DIR="$REPO_ROOT/target/dist"
readonly TEMPLATE="$REPO_ROOT/scripts/templates/ravel-lite.rb.tmpl"

readonly TARGETS=(
  aarch64-apple-darwin
  x86_64-apple-darwin
  aarch64-unknown-linux-gnu
  x86_64-unknown-linux-gnu
)

# Glibc floor for Linux targets — wide compatibility, RHEL 7-era.
readonly LINUX_GLIBC=2.17

die() {
  echo "release-build: $*" >&2
  exit 1
}

require_clean_tagged_tree() {
  [[ -z "$(git -C "$REPO_ROOT" status --porcelain)" ]] \
    || die "working tree is dirty; commit or stash before releasing"
  git -C "$REPO_ROOT" describe --tags --exact-match HEAD >/dev/null 2>&1 \
    || die "HEAD is not a tagged commit; run 'cargo release <level> --execute' first"
}

read_version() {
  git -C "$REPO_ROOT" describe --tags --abbrev=0 | sed 's/^v//'
}

build_target() {
  local target="$1" version="$2"
  case "$target" in
    *-apple-darwin)
      cargo build --release --target "$target"
      ;;
    *-unknown-linux-gnu)
      cargo zigbuild --release --target "${target}.${LINUX_GLIBC}"
      ;;
    *)
      die "unknown target: $target"
      ;;
  esac

  local stage="$DIST_DIR/staging/ravel-lite-v${version}-${target}"
  mkdir -p "$stage"
  cp "$REPO_ROOT/target/$target/release/ravel-lite" "$stage/ravel-lite"
  cp "$REPO_ROOT/LICENSE" "$REPO_ROOT/README.md" "$stage/"

  local archive="$DIST_DIR/ravel-lite-v${version}-${target}.tar.xz"
  tar -C "$DIST_DIR/staging" -cJf "$archive" "ravel-lite-v${version}-${target}"
  echo "$archive"
}

sha256_of() {
  shasum -a 256 "$1" | awk '{print $1}'
}

render_formula() {
  local version="$1"
  shift
  local -A shas
  for arg in "$@"; do
    shas["${arg%%=*}"]="${arg#*=}"
  done

  sed \
    -e "s|@VERSION@|${version}|g" \
    -e "s|@SHA_AARCH64_APPLE_DARWIN@|${shas[aarch64-apple-darwin]}|g" \
    -e "s|@SHA_X86_64_APPLE_DARWIN@|${shas[x86_64-apple-darwin]}|g" \
    -e "s|@SHA_AARCH64_UNKNOWN_LINUX_GNU@|${shas[aarch64-unknown-linux-gnu]}|g" \
    -e "s|@SHA_X86_64_UNKNOWN_LINUX_GNU@|${shas[x86_64-unknown-linux-gnu]}|g" \
    "$TEMPLATE" >"$DIST_DIR/ravel-lite.rb"
}

main() {
  cd "$REPO_ROOT"
  "$REPO_ROOT/scripts/release-doctor.sh"
  require_clean_tagged_tree
  local version
  version="$(read_version)"
  echo "release-build: building ravel-lite v${version}"

  rm -rf "$DIST_DIR"
  mkdir -p "$DIST_DIR/staging"

  local sha_args=()
  for target in "${TARGETS[@]}"; do
    echo "release-build: target $target"
    local archive
    archive="$(build_target "$target" "$version")"
    sha_args+=("${target}=$(sha256_of "$archive")")
  done

  render_formula "$version" "${sha_args[@]}"
  rm -rf "$DIST_DIR/staging"

  echo
  echo "release-build: artifacts in $DIST_DIR"
  ls -la "$DIST_DIR"
  echo
  echo "Inspect, then run scripts/release-publish.sh"
}

main "$@"
