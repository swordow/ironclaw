#!/usr/bin/env bash
set -euo pipefail

echo "==> fmt check"
cargo fmt --all -- --check

echo "==> clippy (all warnings)"
cargo clippy --locked --all --benches --tests --examples --all-features -- -D warnings

echo "==> cargo deny"
if command -v cargo-deny &>/dev/null; then
    cargo deny check
else
    echo "WARN: cargo-deny not installed, skipping (install with: cargo install cargo-deny)"
fi

echo "==> tests"
cargo test --locked
