set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

default:
    @just --list

fmt: fmt-rust fmt-nix fmt-shell

fmt-rust:
    cargo fmt --all

fmt-nix:
    alejandra .

fmt-shell:
    shfmt -w scripts

fmt-check: fmt-check-rust fmt-check-nix fmt-check-shell

fmt-check-rust:
    cargo fmt --all -- --check

fmt-check-nix:
    alejandra --check .

fmt-check-shell:
    shfmt -d scripts

lint: lint-rust lint-nix lint-shell

lint-rust:
    cargo clippy --all-targets --all-features

lint-nix:
    deadnix .
    statix check .

lint-shell:
    shellcheck scripts/*.sh

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
