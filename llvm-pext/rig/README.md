# llvm.pext / llvm.pdep on real cores

The table for llvm/llvm-project#227292: today's AArch64 expansion with and
without AES (PMULL), the bytewise and the staged forms, in cycles from the
core's own counter, at 32 and 64 bits. Nothing to install, the binaries in
`bin/` are static (Linux) or need only the system (macOS):

    git clone https://github.com/mightsleep/deposit-bench
    cd deposit-bench/llvm-pext/rig
    sudo sh run.sh          # macOS: root for the cycle counter (kperf)
    sh run.sh               # Linux: perf_event_paranoid <= 2

Output goes to `results/<host>-<time>/`: `machine.txt`, `log.txt` and
`all.csv` with every round.

Variants, all from llc at main f0f0fe2 with or without the PR:

| var  | what                                          |
|------|-----------------------------------------------|
| nop  | `xor`, the call and loop overhead (not subtracted) |
| now  | main, no AES                                  |
| aes  | main, +aes: CLMUL through PMULL, what macOS gets today |
| byte | bytewise network                              |
| stg  | staged whole-word network, what the PR emits off Apple |
| aa   | `stg` again at another address: the A/A noise floor |
| bext | SVE2 BEXT/BDEP, only where the core has them  |

Each binary is built for one `-mcpu` (macOS: apple-m1 and apple-m4; Linux:
generic and the detected Neoverse); run.sh alternates them, three rounds by
default (`ROUNDS=5 sh run.sh`). Inside a binary the variants run round robin
with a rotating start, 300 rounds, min and median reported; `thr` is
independent calls, `lat` a chain through the result.

A GHz column well below the nominal clock on a Mac means the thread landed on
an efficiency core; rerun. To rebuild the objects and binaries you need the
llc builds and zig: `sh gen.sh`.
