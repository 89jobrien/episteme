set shell := ["zsh", "-cu"]

default:
  @just --list

generate:
  baml-cli generate

fmt:
  cargo fmt --all

check:
  cargo check --all-targets --all-features

lint:
  cargo clippy --all-targets --all-features -- -D warnings

test:
  cargo nextest run --all-features

test-live-apfel:
  EPISTEME_RUN_LIVE=1 EPISTEME_CLASSIFY_BASE_URL=http://127.0.0.1:18181/v1 EPISTEME_CLASSIFY_MODEL=apple-foundationmodel EPISTEME_DISTILL_BASE_URL=http://127.0.0.1:18181/v1 EPISTEME_DISTILL_MODEL=apple-foundationmodel cargo nextest run --test live_ingestion --run-ignored ignored-only

ci: generate
  cargo fmt --all -- --check
  cargo check --all-targets --all-features
  cargo clippy --all-targets --all-features -- -D warnings
  cargo nextest run --all-features
  cargo test --doc
