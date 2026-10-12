#!/bin/sh
# vortex-array bool filter in situ: upstream develop (main, the 64 KB byte table on any CPU
# without fast BMI2) against the port (port.patch: runs + byte network, SVE2 BEXT, Zen 1/2
# check). measure.patch adds FORCE_FALLBACK, which makes the port take its portable path even
# where the CPU has BEXT. Two rounds.
set -eu
SHA=d42e7c96fd5ffe7df67e99165eba59194984feaf
here=$(cd "$(dirname "$0")" && pwd)
out=$PWD/results
mkdir -p "$out"

fetch() {
  git init -q "$1"
  git -C "$1" fetch -q --depth 1 https://github.com/vortex-data/vortex "$SHA"
  git -C "$1" checkout -q FETCH_HEAD
}
fetch main
fetch port
git -C port apply "$here/port.patch"
git -C port apply "$here/measure.patch"

if command -v rustup > /dev/null; then
  (cd main && rustup toolchain install --profile minimal > /dev/null)
fi
rustc --version

# the portable compress and BEXT (where the CPU has it) against a reference, on this CPU
(cd port && cargo test -q -p vortex-array --lib arrays::bool::compute::filter)

for t in main port; do
  (cd $t && cargo bench -q -p vortex-array --bench filter_bool --no-run)
done

groups="density_sweep_random density_sweep_dense_runs filter_random_by_random filter_random_by_mostly_true filter_random_by_mostly_false filter_random_by_correlated_runs filter_random_by_power_law"
variants="main portable"
pin=""
if [ -r /proc/cpuinfo ]; then
  grep -qw svebitperm /proc/cpuinfo && variants="$variants bext"
  command -v taskset > /dev/null && pin="taskset -c 1"
fi
echo "variants: $variants"

for round in 1 2; do
  for v in $variants; do
    case $v in
      main) dir=main; force= ;;
      portable) dir=port; force=FORCE_FALLBACK=1 ;;
      bext) dir=port; force= ;;
    esac
    : > "$out/$v-r$round.txt"
    for g in $groups; do
      (cd $dir && env $force $pin cargo bench -q -p vortex-array --bench filter_bool -- "$g") \
        >> "$out/$v-r$round.txt" 2>&1
    done
  done
  echo "== round $round, median µs, main / each"
  sh "$here/table.sh" $(for v in $variants; do echo "$out/$v-r$round.txt"; done)
done
