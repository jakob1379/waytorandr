set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

default:
    @just --list

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

lint:
    cargo clippy --all-targets --all-features

test:
    cargo test -q

test-cli:
    ./scripts/test-cli-integration.sh

check: fmt-check lint test build

build:
    nix build

smoke: build
    ./result/bin/waytorandr --help

clean:
    rm -rf target result result-*
