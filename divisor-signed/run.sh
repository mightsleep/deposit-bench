#!/bin/sh
# Signed division by an invariant divisor: arrow-rs PR #11467 (figure 5.1), its remainder with
# devanbenz's change, the quotient of |n| with an unsigned high multiply, an f64 reciprocal
# (i32) and floor division through the one's complement. Loops aligned to 64 bytes.
set -eu
cd "$(dirname "$0")"
export RUSTFLAGS="-C llvm-args=-align-loops=64"
cargo build --release --quiet
echo "== default build"
./target/release/divisor-signed
RUSTFLAGS="$RUSTFLAGS -C target-cpu=native" CARGO_TARGET_DIR=target/native cargo build --release --quiet
echo "== target-cpu=native"
./target/native/release/divisor-signed
