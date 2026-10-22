// Ablation for the +aes expansion: the same Hacker's Delight 7-4 compress
// (prefix XOR by PMULL against all ones), but every value stays in vector
// registers, so the only GPR <-> vector moves are the arguments and the
// result. aes has 12 of them on its serial chain (6 stages, in and out), so
// aes - vec is what those moves cost. rt6 and pm6 time the two halves of one
// stage: six GPR -> vector -> GPR round trips, six chained PMULLs.
#include <arm_neon.h>
#include <stdint.h>

static inline uint64x1_t pmul(uint64x1_t a, uint64x1_t b) {
  uint64x2_t r;
  __asm__(".arch_extension aes\n\tpmull %0.1q, %1.1d, %2.1d" : "=w"(r) : "w"(a), "w"(b));
  return vget_low_u64(r);
}

#define STAGE(k)                                         \
  do {                                                   \
    uint64x1_t mp = pmul(mk, ones);                      \
    uint64x1_t mv = vand_u64(mp, m);                     \
    m = vorr_u64(veor_u64(m, mv), vshr_n_u64(mv, k));    \
    uint64x1_t t = vand_u64(x, mv);                      \
    x = vorr_u64(veor_u64(x, t), vshr_n_u64(t, k));      \
    mk = vbic_u64(mk, mp);                               \
  } while (0)

uint64_t vec_pext64(uint64_t v, uint64_t mask) {
  uint64x1_t m = vcreate_u64(mask), ones = vcreate_u64(~0ULL);
  uint64x1_t x = vand_u64(vcreate_u64(v), m);
  uint64x1_t mk = vshl_n_u64(veor_u64(m, ones), 1);
  STAGE(1); STAGE(2); STAGE(4); STAGE(8); STAGE(16); STAGE(32);
  return vget_lane_u64(x, 0);
}

uint64_t rt6_pext64(uint64_t v, uint64_t m) {
  for (int i = 0; i < 6; i++) __asm__ volatile("fmov d16, %0\n\tfmov %0, d16" : "+r"(v) : : "v16");
  return v ^ m;
}

uint64_t pm6_pext64(uint64_t v, uint64_t m) {
  uint64x1_t a = vcreate_u64(v), b = vcreate_u64(m);
  for (int i = 0; i < 6; i++) a = pmul(a, b);
  return vget_lane_u64(a, 0);
}

// Hybrid: the mask chain (PMULL, BIC) stays in vector registers, the data
// updates run in GPRs. One vector -> GPR move per stage, off the mask chain.
static inline uint64x1_t bic(uint64x1_t a, uint64x1_t b) {
  uint64x1_t r;
  __asm__("bic %0.8b, %1.8b, %2.8b" : "=w"(r) : "w"(a), "w"(b));
  return r;
}

static inline uint64_t to_gpr(uint64x1_t v) {
  uint64_t r;
  __asm__("fmov %x0, %d1" : "=r"(r) : "w"(v));
  return r;
}

#define HSTAGE(k)                     \
  do {                                \
    uint64x1_t mp = pmul(mk, ones);   \
    mk = bic(mk, mp);                 \
    uint64_t mv = to_gpr(mp) & m;     \
    m = (m ^ mv) | (mv >> k);         \
    uint64_t t = x & mv;              \
    x = (x ^ t) | (t >> k);           \
  } while (0)

uint64_t hyb_pext64(uint64_t v, uint64_t m) {
  uint64x1_t ones = vcreate_u64(~0ULL), mk = vcreate_u64(~m << 1);
  uint64_t x = v & m;
  HSTAGE(1); HSTAGE(2); HSTAGE(4); HSTAGE(8); HSTAGE(16); HSTAGE(32);
  return x;
}
