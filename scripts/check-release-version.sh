#!/usr/bin/env bash
set -eu

repo_root="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$repo_root"

cargo_version="$(
  awk '
    $0 == "[workspace.package]" { in_section = 1; next }
    /^\[/ && in_section { exit }
    in_section && $1 == "version" {
      gsub(/"/, "", $3)
      print $3
      exit
    }
  ' Cargo.toml
)"

if [ -z "${cargo_version:-}" ]; then
  echo "error: failed to read [workspace.package].version from Cargo.toml" >&2
  exit 1
fi

tag_name="${1:-${GITHUB_REF_NAME:-}}"
if [ -z "${tag_name:-}" ]; then
  echo "usage: $0 vX.Y.Z" >&2
  echo "or set GITHUB_REF_NAME to a release tag name" >&2
  exit 2
fi

tag_name="${tag_name#refs/tags/}"
if [ "${tag_name#v}" = "$tag_name" ]; then
  echo "error: release tag must start with 'v', got '$tag_name'" >&2
  exit 1
fi

tag_version="${tag_name#v}"

if [ "$cargo_version" != "$tag_version" ]; then
  echo "error: release tag '$tag_name' does not match Cargo workspace version '$cargo_version'" >&2
  exit 1
fi

echo "release version check passed: tag '$tag_name' matches Cargo workspace version '$cargo_version'"
