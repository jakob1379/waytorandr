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

lint: lint-rust lint-rust-dead-code lint-nix lint-shell

lint-rust:
    cargo clippy --locked --workspace --all-targets --all-features -- -D warnings

lint-rust-dead-code:
    env RUSTFLAGS='-Ddead_code' cargo check --locked --workspace --all-targets

lint-nix:
    deadnix .
    statix check .

lint-shell:
    shellcheck scripts/*.sh

test:
    cargo test --locked --workspace -q

test-cli:
    ./scripts/test-cli-integration.sh

check: fmt-check lint test test-cli build

build:
    nix build

smoke: build
    ./result/bin/waytorandr --help

clean:
    rm -rf target result result-*
