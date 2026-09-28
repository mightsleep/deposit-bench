# deposit-bench

Portable `pdep` (bit deposit) for vortex-mask's `intersect_by_rank`, measured
end to end against the current portable loop on GitHub's arm64 and x86
runners. See the doc comment at the top of `src/main.rs`.

`vortex`, `deposit_vortex` and `select_bit_position_portable` reproduce code
from vortex-data/vortex (`vortex-mask/src/intersect_by_rank.rs`, Apache-2.0)
for comparison.
