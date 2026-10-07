#!/bin/sh
# Builds the objects run.sh links, on the machine with the llc builds: today's
# expansion (now, now +aes), the staged one from the PR (stg, and the same code
# again as aa, for an A/A pair at another address), the bytewise one (byte),
# SVE2 BEXT/BDEP where the core has them (bext, Linux only), and a call that
# does nothing (nop) for the harness overhead. One set per OS and -mcpu.
set -eu
cd "$(dirname "$0")"
L=${LLC_DIR:-$HOME/work/llvm-ternlog}
base=$L/llc-main-base # main f0f0fe2
pr=$L/llc-main-pext   # main f0f0fe2 + #227292
byte=$L/llc-pext-a64  # bytewise on every AArch64 core

ir() { # prefix: pext/pdep at 8..64 bits, or xor for nop
  for w in 8 16 32 64; do
    for op in pext pdep; do
      if [ "$1" = nop ]; then
        echo "define i$w @nop_$op$w(i$w %v, i$w %m) { %r = xor i$w %v, %m ret i$w %r }"
      else
        echo "declare i$w @llvm.$op.i$w(i$w, i$w)"
        echo "define i$w @$1_$op$w(i$w %v, i$w %m) { %r = call i$w @llvm.$op.i$w(i$w %v, i$w %m) ret i$w %r }"
      fi
    done
  done
}

one() { # os triple cpu
  d=obj/$1-$3 os=$1
  mkdir -p "$d"
  set -- "-O2" "-filetype=obj" "-mtriple=$2" "-mcpu=$3"
  ir now | "$base" "$@" -mattr=-aes,-sve-bitperm -o "$d/now.o"
  ir aes | "$base" "$@" -mattr=+aes,-sve-bitperm -o "$d/aes.o"
  ir stg | "$pr" "$@" -mattr=-aes,-sve-bitperm -o "$d/stg.o"
  ir aa | "$pr" "$@" -mattr=-aes,-sve-bitperm -o "$d/aa.o"
  ir byte | "$byte" "$@" -mattr=-aes,-sve-bitperm -o "$d/byte.o"
  ir nop | "$base" "$@" -o "$d/nop.o"
  case $os in linux) ir bext | "$base" "$@" -mattr=+sve2-bitperm -o "$d/bext.o" ;; esac
}

exe() { # os cpu: prebuilt bench, so the rented machine needs no compiler
  d=obj/$1-$2
  case $1 in
    mac) tgt=aarch64-macos bext="" ;;
    linux) tgt=aarch64-linux-musl bext="-DHAVE_BEXT $d/bext.o" ;;
  esac
  # shellcheck disable=SC2086 # bext is a word list
  zig cc -target "$tgt" -O2 -DTUNE="\"$2\"" bench.c "$d/now.o" "$d/aes.o" "$d/stg.o" \
    "$d/aa.o" "$d/byte.o" "$d/nop.o" $bext -o "bin/$1-$2"
}

rm -rf obj bin
mkdir -p bin
for c in apple-m1 apple-m4; do one mac arm64-apple-macosx "$c"; done
for c in generic neoverse-n1 neoverse-n2 neoverse-v2; do one linux aarch64-linux-gnu "$c"; done
if command -v zig > /dev/null; then
  for c in apple-m1 apple-m4; do exe mac "$c"; done
  for c in generic neoverse-n1 neoverse-n2 neoverse-v2; do exe linux "$c"; done
else
  echo "no zig: no bin/, run.sh builds with cc" >&2
fi
ls obj bin
