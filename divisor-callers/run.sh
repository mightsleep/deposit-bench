#!/bin/sh
# Real callers of a run-time divisor (arrow integer_op, DataFusion date_bin, Comet bloom
# filter): today's `/` and `%` against arrow-rs PR #11467 matched outside the loop and the
# opaque divisor called per value. Loops aligned to 64 bytes, so that where they land doesn't
# decide the numbers.
set -eu
cd "$(dirname "$0")"
export RUSTFLAGS="-C llvm-args=-align-loops=64"
cargo build --release --quiet
echo "== default build"
./target/release/divisor-callers
RUSTFLAGS="$RUSTFLAGS -C target-cpu=native" CARGO_TARGET_DIR=target/native cargo build --release --quiet
echo "== target-cpu=native"
./target/native/release/divisor-callers
