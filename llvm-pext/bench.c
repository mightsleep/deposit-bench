// llvm.pext / llvm.pdep expansion: today's without AES (old_*) and with it, so
// with PMULL for CLMUL (aes_*), the bytewise one (new_*) and the staged one
// (stg_*), and Rust core's extract_bits / deposit_bits unless built with
// -DNO_RUST; objects from llc before and after the change.
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>

#define DECL1(p, w) uint##w##_t p##_pext##w(uint##w##_t, uint##w##_t), p##_pdep##w(uint##w##_t, uint##w##_t);
#define DECL(p) DECL1(p, 8) DECL1(p, 16) DECL1(p, 32) DECL1(p, 64)
DECL(old) DECL(aes) DECL(new) DECL(stg)
#ifndef NO_RUST
uint64_t ext64(uint64_t, uint64_t), dep64(uint64_t, uint64_t);
#endif

static uint64_t ref_pext(uint64_t v, uint64_t m) {
  uint64_t r = 0; int k = 0;
  for (int i = 0; i < 64; i++) if (m >> i & 1) r |= (v >> i & 1) << k++;
  return r;
}
static uint64_t ref_pdep(uint64_t v, uint64_t m) {
  uint64_t r = 0; int k = 0;
  for (int i = 0; i < 64; i++) if (m >> i & 1) r |= (v >> k++ & 1) << i;
  return r;
}
static uint64_t s = 0x9E3779B97F4A7C15ULL;
static uint64_t rnd(void) { s ^= s << 13; s ^= s >> 7; s ^= s << 17; return s; }
static uint64_t mask(void) {
  uint64_t m = rnd();
  switch (rnd() % 5) { case 1: m &= rnd() & rnd(); break; case 2: m |= rnd() | rnd(); break;
    case 3: m = ~(1ULL << (rnd() & 63)); break; case 4: m = 1ULL << (rnd() & 63); break; }
  return m;
}

#define CHECK8(p) \
  bad += p##_pext8(v, m) != ref_pext(v, m) || p##_pdep8(v, m) != (uint8_t)ref_pdep(v, m);
#define CHECK(p) \
  bad += p##_pext16(v, m16) != ref_pext((uint16_t)v, m16) || p##_pdep16(v, m16) != (uint16_t)ref_pdep(v, m16); \
  bad += p##_pext32(v, m32) != ref_pext((uint32_t)v, m32) || p##_pdep32(v, m32) != (uint32_t)ref_pdep(v, m32); \
  bad += p##_pext64(v, m) != ref_pext(v, m) || p##_pdep64(v, m) != ref_pdep(v, m);

static long check(void) {
  long bad = 0;
  for (unsigned v = 0; v < 256; v++) for (unsigned m = 0; m < 256; m++) {
    CHECK8(old) CHECK8(aes) CHECK8(new) CHECK8(stg)
  }
  for (long i = 0; i < 2000000; i++) {
    uint64_t v = rnd(), m = mask();
    if (i % 7 == 0) m = 0;
    if (i % 11 == 0) m = ~0ULL;
    uint16_t m16 = m; uint32_t m32 = m;
    CHECK(old) CHECK(aes) CHECK(new) CHECK(stg)
#ifndef NO_RUST
    bad += ext64(v, m) != ref_pext(v, m) || dep64(v, m) != ref_pdep(v, m);
#endif
  }
  return bad;
}

typedef uint64_t (*F)(uint64_t, uint64_t);
#define N 4096
static uint64_t V[N], M[N];
static double now(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t); return t.tv_sec * 1e9 + t.tv_nsec; }

int main(void) {
  long bad = check();
  printf("correctness: %ld bad\n", bad);
  if (bad) return 1;
  for (int i = 0; i < N; i++) V[i] = rnd(), M[i] = rnd();
  F f[] = {old_pext64, aes_pext64, new_pext64, stg_pext64,
#ifndef NO_RUST
           ext64,
#endif
           old_pdep64, aes_pdep64, new_pdep64, stg_pdep64,
#ifndef NO_RUST
           dep64,
#endif
  };
  const char *n[] = {"pext now", "pext now +aes", "pext bytewise", "pext staged",
#ifndef NO_RUST
                     "pext rust core",
#endif
                     "pdep now", "pdep now +aes", "pdep bytewise", "pdep staged",
#ifndef NO_RUST
                     "pdep rust core",
#endif
  };
  enum { K = sizeof f / sizeof f[0] };
  double bt[K], bl[K];
  for (int j = 0; j < K; j++) bt[j] = bl[j] = 1e18;
  // interleaved rounds, fastest of each: independent calls, then a chain
  // through the value
  for (int r = 0; r < 200; r++) for (int j = 0; j < K; j++) {
    uint64_t acc = 0, x = 0;
    double t0 = now();
    for (int rep = 0; rep < 10; rep++) for (int i = 0; i < N; i++) acc += f[j](V[i], M[i]);
    double t1 = now();
    for (int rep = 0; rep < 10; rep++) for (int i = 0; i < N; i++) x = f[j](V[i] ^ (x & 1), M[i]);
    double t2 = now();
    __asm__ volatile("" :: "r"(acc), "r"(x));
    if (t1 - t0 < bt[j]) bt[j] = t1 - t0;
    if (t2 - t1 < bl[j]) bl[j] = t2 - t1;
  }
  for (int j = 0; j < K; j++)
    printf("%-16s thr %6.2f ns  lat %6.2f ns\n", n[j], bt[j] / (10.0 * N), bl[j] / (10.0 * N));
  return 0;
}
