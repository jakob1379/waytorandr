#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(git rev-parse --show-toplevel)" && pwd)"
cd "$repo_root"

printf '%s\n' 'Running simulated CLI backend integration tests...'
nix develop -c cargo test -p waytorandr --features test-backend --test full_cli -- --nocapture
