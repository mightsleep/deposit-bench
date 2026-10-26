#!/bin/sh
# u32 division by an invariant divisor: arrow-rs PR #11467 against the round-up / round-down
# split, in the default build and with -C target-cpu=native
set -eu
cd "$(dirname "$0")"
cargo build --release --quiet
echo "== default build"
./target/release/divisor-u32
RUSTFLAGS="-C target-cpu=native" CARGO_TARGET_DIR=target/native cargo build --release --quiet
echo "== target-cpu=native"
./target/native/release/divisor-u32
