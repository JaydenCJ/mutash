#!/usr/bin/env bash
# deploy.sh — small release-promotion helper used by the mutash examples.
#
# Everything here is offline: "deploying" means promoting an artifact
# between two local directories after verification. It is exactly the kind
# of script mutash is built for — short, load-bearing, and full of
# comparisons that a lazy test suite never exercises at the boundary.
set -u

MAX_ATTEMPTS="${MAX_ATTEMPTS:-3}"

# MAJOR.MINOR.PATCH, digits and dots only, no empty components.
valid_version() {
  case "$1" in
    '' | *[!0-9.]*) return 1 ;;
    *.*.*) ;;
    *) return 1 ;;
  esac
  case "$1" in
    .* | *. | *..*) return 1 ;;
  esac
  return 0
}

# An HTTP-style status code counts as healthy in the 2xx range.
healthy() {
  [ "$1" -ge 200 ] && [ "$1" -lt 300 ]
}

# Run a command up to MAX_ATTEMPTS times; succeed on the first pass.
retry() {
  local attempt=1
  while [ "$attempt" -le "$MAX_ATTEMPTS" ]; do
    if "$@"; then
      return 0
    fi
    attempt=$((attempt + 1))
  done
  return 1
}

# Promote an artifact into the release directory if it exists and is non-empty.
promote() {
  local artifact="$1" release_dir="$2"
  if [ ! -f "$artifact" ]; then
    echo "missing artifact: $artifact" >&2
    return 1
  fi
  if [ ! -s "$artifact" ]; then
    echo "artifact is empty: $artifact" >&2
    return 1
  fi
  mkdir -p "$release_dir" && cp "$artifact" "$release_dir/"
}

# CLI entry: deploy.sh <version> <artifact> <release-dir>
main() {
  if [ "$#" -ne 3 ]; then
    echo "usage: deploy.sh <version> <artifact> <release-dir>" >&2
    return 2
  fi
  if ! valid_version "$1"; then
    echo "bad version: $1" >&2
    return 2
  fi
  if ! retry promote "$2" "$3"; then
    echo "deploy failed" >&2
    return 1
  fi
  echo "deployed $1"
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  main "$@"
fi
