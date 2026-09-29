#!/bin/sh
# Times today's llvm.pext / llvm.pdep expansion against the two proposed ones
# on an AArch64 machine (macOS or Linux). Needs a C compiler; Rust nightly is
# used for a Rust core column when present.
set -eu
cd "$(dirname "$0")"
case "$(uname -s)" in
  Darwin) obj=obj/mac; libs="" ;;
  Linux) obj=obj/linux; libs="-lpthread -ldl" ;;
  *) echo "macOS or Linux only" >&2; exit 1 ;;
esac
case "$(uname -m)" in
  arm64 | aarch64) ;;
  *) echo "the objects are AArch64" >&2; exit 1 ;;
esac
rust=""
if rustc -O -C panic=abort core.rs -o libcore_bits.a 2> /dev/null; then
  rust=libcore_bits.a
fi
# shellcheck disable=SC2086 # libs is a word list
if [ -n "$rust" ]; then
  cc -O2 bench.c "$obj/old.o" "$obj/new.o" "$obj/stg.o" "$rust" $libs -o bench
else
  cc -O2 -DNO_RUST bench.c "$obj/old.o" "$obj/new.o" "$obj/stg.o" $libs -o bench
fi
if [ "$(uname -s)" = Darwin ]; then sysctl -n machdep.cpu.brand_string; else grep -m1 -i "model name\|CPU part" /proc/cpuinfo || true; fi
./bench
