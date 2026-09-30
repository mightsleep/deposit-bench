#!/bin/sh
# vortex-mask intersect_by_rank in situ: upstream develop (main) against the
# port with each deposit forced through VX_DEPOSIT (measure.patch), twice.
# bench.patch adds random_rotating and its extra densities to both trees.
set -eu
SHA=4224390c2b1c9dfa6d7376c5d1760763a3d6d6da
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
git -C main apply "$here/bench.patch"
git -C port apply "$here/bench.patch" "$here/port.patch" "$here/measure.patch"

if command -v rustup > /dev/null; then
  (cd main && rustup toolchain install --profile minimal > /dev/null)
fi
rustc --version

# the new deposits against a reference, on this CPU
(cd port && cargo test -q -p vortex-mask --lib intersect_by_rank)

for t in main port; do
  (cd $t && cargo bench -q -p vortex-mask --bench intersect_by_rank --no-run)
done

variants="main portable"
pin=""
if [ -r /proc/cpuinfo ]; then
  grep -qw pmull /proc/cpuinfo && variants="$variants pmull"
  grep -qw svebitperm /proc/cpuinfo && variants="$variants bdep"
  command -v taskset > /dev/null && pin="taskset -c 1"
else
  variants="$variants pmull"
fi
echo "variants: $variants"

for round in 1 2; do
  for v in $variants; do
    if [ "$v" = main ]; then dir=main; dep=; else dir=port; dep=$v; fi
    (cd $dir && VX_DEPOSIT=$dep $pin cargo bench -q -p vortex-mask --bench intersect_by_rank) \
      > "$out/$v-r$round.txt" 2>&1
  done
  echo "== round $round, median µs, main / each"
  sh "$here/table.sh" $(for v in $variants; do echo "$out/$v-r$round.txt"; done)
done
