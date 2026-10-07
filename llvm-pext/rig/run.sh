#!/bin/sh
# Runs the llvm.pext / llvm.pdep table on this machine and keeps the output in
# results/. macOS needs root for the cycle counter: sudo sh run.sh. Uses the
# prebuilt bin/ when it matches the OS, else builds with cc.
#   ROUNDS=3 sh run.sh   alternating binaries, three times each (default)
set -eu
cd "$(dirname "$0")"
case "$(uname -m)" in
  arm64 | aarch64) ;;
  *) echo "AArch64 only" >&2; exit 1 ;;
esac
case "$(uname -s)" in
  Darwin)
    os=mac
    tunes="apple-m1 apple-m4"
    [ "$(id -u)" = 0 ] || echo "not root: ns only, no cycles (sudo sh run.sh)" >&2
    ;;
  Linux)
    os=linux
    impl=$(awk -F: '/CPU implementer/ {gsub(/ /, "", $2); print $2; exit}' /proc/cpuinfo)
    part=$(awk -F: '/CPU part/ {gsub(/ /, "", $2); print $2; exit}' /proc/cpuinfo)
    tunes=generic
    if [ "$impl" = 0x41 ]; then
      case $part in
        0xd0c) tunes="generic neoverse-n1" ;;
        0xd49) tunes="generic neoverse-n2" ;;
        0xd4f) tunes="generic neoverse-v2" ;;
      esac
    fi
    p=$(cat /proc/sys/kernel/perf_event_paranoid 2> /dev/null || echo 0)
    [ "$p" -le 2 ] || echo "perf_event_paranoid=$p: no cycles (sudo sysctl kernel.perf_event_paranoid=2)" >&2
    ;;
  *) echo "macOS or Linux only" >&2; exit 1 ;;
esac

out=results/$(hostname -s)-$(date +%Y%m%d-%H%M%S)
mkdir -p "$out" build
{
  echo "date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  uname -a
  if [ $os = mac ]; then
    sysctl -n machdep.cpu.brand_string hw.model hw.perflevel0.physicalcpu hw.perflevel1.physicalcpu 2> /dev/null || true
    sw_vers 2> /dev/null || true
  else
    lscpu 2> /dev/null | grep -E "Model name|Vendor|Architecture|^CPU\(s\)|MHz|Flags" || grep -m4 -E "implementer|part|Features" /proc/cpuinfo
    echo "implementer $impl part $part"
  fi
} > "$out/machine.txt"
cat "$out/machine.txt"

for t in $tunes; do
  b=bin/$os-$t
  if [ ! -x "$b" ]; then
    b=build/$os-$t
    bext=""
    [ -f "obj/$os-$t/bext.o" ] && bext="-DHAVE_BEXT obj/$os-$t/bext.o"
    # shellcheck disable=SC2086 # bext is a word list
    cc -O2 -DTUNE="\"$t\"" bench.c obj/$os-$t/now.o obj/$os-$t/aes.o obj/$os-$t/stg.o \
      obj/$os-$t/aa.o obj/$os-$t/byte.o obj/$os-$t/nop.o $bext -o "$b"
  fi
done

echo "tune,op,var,w,thr_cyc_min,thr_cyc_med,lat_cyc_min,lat_cyc_med,ins,thr_ns,lat_ns" > "$out/all.csv"
i=1
while [ "$i" -le "${ROUNDS:-3}" ]; do
  for t in $tunes; do
    b=bin/$os-$t
    [ -x "$b" ] || b=build/$os-$t
    echo "== round $i, -mcpu=$t" | tee -a "$out/log.txt"
    "$b" "$out/all.csv" | tee -a "$out/log.txt"
  done
  i=$((i + 1))
done
echo "results in $out (machine.txt, log.txt, all.csv)"
