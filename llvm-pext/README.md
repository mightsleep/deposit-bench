# llvm.pext / llvm.pdep expansion

Times the generic expansion of `llvm.pext` and `llvm.pdep` on an AArch64 machine
(macOS or Linux), before and after a change to it. The objects are prebuilt by llc,
so only a C compiler is needed:

    git clone https://github.com/mightsleep/deposit-bench
    sh deposit-bench/llvm-pext/run.sh

It first checks every function against a reference loop, then prints ns per call
for random 64-bit masks: `now` is today's expansion, `now +aes` the same with AES,
which gives CLMUL through PMULL (every Apple core has it, most server cores too),
and `bytewise` and `staged` the two proposed forms. A Rust core row appears
when `rustc` is a nightly.
