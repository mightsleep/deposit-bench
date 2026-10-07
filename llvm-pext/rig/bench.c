// llvm.pext / llvm.pdep expansions on one AArch64 core, in cycles from the
// core's own counter where the OS lets us read it (perf_event on Linux, kperf
// as root on macOS), in ns always. Variants come from gen.sh; aa is stg again
// at another address, so the stg/aa gap is the noise floor of the table.
#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#ifndef TUNE
#define TUNE "?"
#endif

#define DECL1(p, w) uint##w##_t p##_pext##w(uint##w##_t, uint##w##_t), p##_pdep##w(uint##w##_t, uint##w##_t);
#define DECL(p) DECL1(p, 8) DECL1(p, 16) DECL1(p, 32) DECL1(p, 64)
DECL(now) DECL(aes) DECL(stg) DECL(aa) DECL(byte) DECL(nop)
#ifdef HAVE_BEXT
DECL(bext)
#endif

// counters: cycles and instructions, user mode, this thread
static int ctr_ok;
#if defined(__linux__)
#include <linux/perf_event.h>
#include <sched.h>
#include <sys/auxv.h>
#include <sys/ioctl.h>
#include <sys/syscall.h>
#include <unistd.h>
static int pfd = -1;
static int pe_open(uint64_t cfg, int group) {
  struct perf_event_attr a;
  memset(&a, 0, sizeof a);
  a.type = PERF_TYPE_HARDWARE;
  a.size = sizeof a;
  a.config = cfg;
  a.disabled = group < 0;
  a.exclude_kernel = 1;
  a.exclude_hv = 1;
  a.read_format = PERF_FORMAT_GROUP;
  return (int)syscall(SYS_perf_event_open, &a, 0, -1, group, 0);
}
static void ctr_init(void) {
  cpu_set_t set;
  CPU_ZERO(&set);
  CPU_SET(sched_getcpu(), &set);
  if (sched_setaffinity(0, sizeof set, &set)) perror("sched_setaffinity");
  pfd = pe_open(PERF_COUNT_HW_CPU_CYCLES, -1);
  if (pfd < 0 || pe_open(PERF_COUNT_HW_INSTRUCTIONS, pfd) < 0) {
    perror("perf_event_open (try: sudo sysctl kernel.perf_event_paranoid=2)");
    return;
  }
  ioctl(pfd, PERF_EVENT_IOC_ENABLE, PERF_IOC_FLAG_GROUP);
  ctr_ok = 1;
}
static void ctr_read(uint64_t c[2]) {
  struct { uint64_t nr, v[2]; } r;
  if (!ctr_ok || read(pfd, &r, sizeof r) != sizeof r) { c[0] = c[1] = 0; return; }
  c[0] = r.v[0];
  c[1] = r.v[1];
}
static int have_bext(void) {
#ifdef HWCAP2_SVEBITPERM
  return (getauxval(AT_HWCAP2) & HWCAP2_SVEBITPERM) != 0;
#else
  return (getauxval(AT_HWCAP2) & (1UL << 4)) != 0;
#endif
}
#elif defined(__APPLE__)
#include <dlfcn.h>
#include <pthread.h>
// kperf is private; these four calls and the fixed counters (0 cycles,
// 1 instructions) are what every M1 counter tool uses. Needs root.
static int (*kpc_force_all_ctrs_set)(int);
static int (*kpc_set_counting)(uint32_t);
static int (*kpc_set_thread_counting)(uint32_t);
static int (*kpc_get_thread_counters)(int, unsigned, uint64_t *);
static void ctr_init(void) {
  // P cores; an E core shows up as a low GHz column
  pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE, 0);
  void *h = dlopen("/System/Library/PrivateFrameworks/kperf.framework/kperf", RTLD_LAZY);
  if (!h) { fprintf(stderr, "kperf: %s\n", dlerror()); return; }
  kpc_force_all_ctrs_set = (int (*)(int))dlsym(h, "kpc_force_all_ctrs_set");
  kpc_set_counting = (int (*)(uint32_t))dlsym(h, "kpc_set_counting");
  kpc_set_thread_counting = (int (*)(uint32_t))dlsym(h, "kpc_set_thread_counting");
  kpc_get_thread_counters = (int (*)(int, unsigned, uint64_t *))dlsym(h, "kpc_get_thread_counters");
  if (!kpc_force_all_ctrs_set || !kpc_set_counting || !kpc_set_thread_counting || !kpc_get_thread_counters) {
    fprintf(stderr, "kperf: missing symbols\n");
    return;
  }
  if (kpc_force_all_ctrs_set(1) || kpc_set_counting(1) || kpc_set_thread_counting(1)) {
    fprintf(stderr, "kperf: no access, run with sudo for cycles\n");
    return;
  }
  ctr_ok = 1;
}
static void ctr_read(uint64_t c[2]) {
  uint64_t b[32] = {0};
  if (!ctr_ok || kpc_get_thread_counters(0, 32, b)) { c[0] = c[1] = 0; return; }
  c[0] = b[0];
  c[1] = b[1];
}
static int have_bext(void) { return 0; }
#else
static void ctr_init(void) {}
static void ctr_read(uint64_t c[2]) { c[0] = c[1] = 0; }
static int have_bext(void) { return 0; }
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

static long check(int bext) {
  long bad = 0;
  for (unsigned v = 0; v < 256; v++) for (unsigned m = 0; m < 256; m++) {
    CHECK8(now) CHECK8(aes) CHECK8(stg) CHECK8(aa) CHECK8(byte)
#ifdef HAVE_BEXT
    if (bext) { CHECK8(bext) }
#endif
  }
  for (long i = 0; i < 2000000; i++) {
    uint64_t v = rnd(), m = mask();
    if (i % 7 == 0) m = 0;
    if (i % 11 == 0) m = ~0ULL;
    uint16_t m16 = m; uint32_t m32 = m;
    CHECK(now) CHECK(aes) CHECK(stg) CHECK(aa) CHECK(byte)
#ifdef HAVE_BEXT
    if (bext) { CHECK(bext) }
#endif
  }
  (void)bext;
  return bad;
}

typedef uint64_t (*F64)(uint64_t, uint64_t);
typedef uint32_t (*F32)(uint32_t, uint32_t);
typedef struct { const char *op, *name; F64 f64; F32 f32; int w; } Row;

#define N 4096
#define REP 10
#define ROUNDS 300
#define WARM 10
static uint64_t V[N], M[N];
static double now_ns(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t); return t.tv_sec * 1e9 + t.tv_nsec; }

// one measurement: ns, cycles, instructions per call
typedef struct { double ns, cyc, ins; } Meas;

static Meas run(const Row *r, int chain) {
  uint64_t c0[2], c1[2], acc = 0, x = 0;
  double t0 = now_ns();
  ctr_read(c0);
  if (r->w == 64) {
    if (chain) { for (int k = 0; k < REP; k++) for (int i = 0; i < N; i++) x = r->f64(V[i] ^ (x & 1), M[i]); }
    else { for (int k = 0; k < REP; k++) for (int i = 0; i < N; i++) acc += r->f64(V[i], M[i]); }
  } else {
    if (chain) { for (int k = 0; k < REP; k++) for (int i = 0; i < N; i++) x = r->f32((uint32_t)V[i] ^ (x & 1), (uint32_t)M[i]); }
    else { for (int k = 0; k < REP; k++) for (int i = 0; i < N; i++) acc += r->f32((uint32_t)V[i], (uint32_t)M[i]); }
  }
  ctr_read(c1);
  double t1 = now_ns();
  __asm__ volatile("" :: "r"(acc), "r"(x));
  double n = (double)REP * N;
  Meas m = {(t1 - t0) / n, (double)(c1[0] - c0[0]) / n, (double)(c1[1] - c0[1]) / n};
  return m;
}

static int cmpd(const void *a, const void *b) {
  double x = *(const double *)a, y = *(const double *)b;
  return (x > y) - (x < y);
}
static double minv(double *a, int n) { double m = a[0]; for (int i = 1; i < n; i++) if (a[i] < m) m = a[i]; return m; }
static double med(double *a, int n) { qsort(a, n, sizeof *a, cmpd); return a[n / 2]; }

#define ROW(p, op, w) {#op, #p, w == 64 ? (F64)p##_##op##64 : 0, w == 32 ? (F32)p##_##op##32 : 0, w}
#define ROWS(op, w) ROW(nop, op, w), ROW(now, op, w), ROW(aes, op, w), ROW(byte, op, w), ROW(stg, op, w), ROW(aa, op, w)

int main(int argc, char **argv) {
  int bext = have_bext();
  long bad = check(bext);
  printf("tune %s, correctness: %ld bad\n", TUNE, bad);
  if (bad) return 1;
  ctr_init();
  for (int i = 0; i < N; i++) V[i] = rnd(), M[i] = rnd();

  Row all[] = {
    ROWS(pext, 64), ROWS(pdep, 64), ROWS(pext, 32), ROWS(pdep, 32),
#ifdef HAVE_BEXT
    ROW(bext, pext, 64), ROW(bext, pdep, 64), ROW(bext, pext, 32), ROW(bext, pdep, 32),
#endif
  };
  int K = sizeof all / sizeof all[0];
  if (!bext) K = 24; // the bext rows come last
  enum { MAXK = 32 };
  static double tc[MAXK][ROUNDS], lc[MAXK][ROUNDS], tn[MAXK][ROUNDS], ln[MAXK][ROUNDS], ti[MAXK][ROUNDS];
  // round robin with a rotating start, so no variant always runs after the same one
  for (int r = -WARM; r < ROUNDS; r++)
    for (int jj = 0; jj < K; jj++) {
      int j = (jj + (r < 0 ? 0 : r)) % K;
      Meas a = run(&all[j], 0), b = run(&all[j], 1);
      if (r < 0) continue;
      tc[j][r] = a.cyc; tn[j][r] = a.ns; ti[j][r] = a.ins;
      lc[j][r] = b.cyc; ln[j][r] = b.ns;
    }

  FILE *csv = argc > 1 ? fopen(argv[1], "a") : 0;
  printf("%-5s %-4s %2s | %8s %8s | %8s %8s | %7s | %7s %7s | %5s\n", "op", "var", "w", "thr cyc", "(med)",
         "lat cyc", "(med)", "ins", "thr ns", "lat ns", "GHz");
  for (int j = 0; j < K; j++) {
    const Row *r = &all[j];
    double tcm = minv(tc[j], ROUNDS), lcm = minv(lc[j], ROUNDS), tnm = minv(tn[j], ROUNDS), lnm = minv(ln[j], ROUNDS);
    double ins = minv(ti[j], ROUNDS);
    double tcd = med(tc[j], ROUNDS), lcd = med(lc[j], ROUNDS);
    printf("%-5s %-4s %2d | %8.2f %8.2f | %8.2f %8.2f | %7.1f | %7.2f %7.2f | %5.2f\n", r->op, r->name, r->w, tcm, tcd,
           lcm, lcd, ins, tnm, lnm, ctr_ok ? tcm / tnm : 0.0);
    if (csv)
      fprintf(csv, "%s,%s,%s,%d,%.3f,%.3f,%.3f,%.3f,%.2f,%.3f,%.3f\n", TUNE, r->op, r->name, r->w, tcm, tcd, lcm, lcd, ins,
              tnm, lnm);
    if (j % 6 == 5) printf("\n");
  }
  if (csv) fclose(csv);
  if (!ctr_ok) printf("no cycle counter, ns only\n");
  // row 0 is nop: a handful of instructions at a believable clock, else the
  // counters are not what we think they are (kperf indices are undocumented)
  double ghz = minv(tc[0], ROUNDS) / minv(tn[0], ROUNDS), ins0 = minv(ti[0], ROUNDS);
  if (ctr_ok && (ghz < 0.5 || ghz > 6.5 || ins0 < 3 || ins0 > 40))
    printf("WARNING: counters look wrong (nop: %.2f GHz, %.1f ins), trust ns only\n", ghz, ins0);
  printf("stg vs aa (same code, other address) = noise floor; nop = call overhead, not subtracted\n");
  return 0;
}
