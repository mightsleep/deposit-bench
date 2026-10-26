//! Signed division by an invariant divisor that isn't a power of two, against arrow-rs #11467:
//!
//! - `pr`: the PR's `Multiplier` (figure 5.1 of Granlund and Montgomery), from `pr.rs`, which
//!   is `arrow-buffer/src/divisor.rs` at the PR head (6f639a36c) without its tests
//! - `dev`: the PR's quotient with devanbenz's remainder `n - q0 |d|` (remainder only)
//! - `mag`: `|n|` has one bit less than the word, so the rounded-up multiplier
//!   `floor(2^(N + p) / |d|) + 1`, `p = floor(log2 |d|)`, is exact for every dividend: an
//!   unsigned high multiply of `|n|`, then the sign
//! - `f64` (`i32` only): `trunc(n * rcp)` with `rcp = 1 / d` rounded up by one ulp, exact for
//!   every `i32` pair but `MIN / -1`
//! - `floor` (`div_euclid` by a positive divisor): `pr` corrects its truncated quotient by the
//!   sign of the remainder, `mag` divides `n ^ (n >> 31)`, 31 bits, and flips the result back
//!
//! The forms are checked at both ends of the first, last and random steps of many divisors,
//! then timed: ns per value for 8192 values, minimum over interleaved rounds.

#[allow(dead_code)]
mod pr;

use pr::{DivisorI32, DivisorI64};
use std::hint::black_box;
use std::time::Instant;

const SIZE: usize = 8192;

macro_rules! forms {
    ($dev:ident, $mag:ident, $t:ty, $u:ty, $wide:ty) => {
        #[derive(Clone, Copy)]
        pub struct $dev {
            divisor: $t,
            multiplier: $t,
            shift: u32,
        }

        impl $dev {
            pub fn new(divisor: $t) -> Self {
                let magnitude = divisor.unsigned_abs();
                let l = <$t>::BITS - (magnitude - 1).leading_zeros();
                let m = 1 + (1 << (<$t>::BITS + l - 1)) / magnitude as $wide;
                Self {
                    divisor,
                    multiplier: (m - (1 << <$t>::BITS)) as $t,
                    shift: l - 1,
                }
            }

            #[inline]
            pub fn rem(self, n: $t) -> $t {
                let high = ((self.multiplier as $wide * n as $wide) >> <$t>::BITS) as $t;
                let q0 = ((n + high) >> self.shift) - (n >> (<$t>::BITS - 1));
                let sign = self.divisor >> (<$t>::BITS - 1);
                let magnitude = (self.divisor ^ sign) - sign;
                n - q0 * magnitude
            }
        }

        #[derive(Clone, Copy)]
        pub struct $mag {
            m: $u,
            p: u32,
            d: $t,
            sign: $t,
        }

        impl $mag {
            pub fn new(d: $t) -> Self {
                let ad = d.unsigned_abs();
                let p = <$t>::BITS - 1 - ad.leading_zeros();
                Self {
                    m: (((1 as $wide) << (<$t>::BITS + p)) / ad as $wide + 1) as $u,
                    p,
                    d,
                    sign: d >> (<$t>::BITS - 1),
                }
            }

            #[inline]
            fn mulhi(self, a: $u) -> $u {
                (((a as $wide) * (self.m as $wide)) >> <$t>::BITS) as $u >> self.p
            }

            #[inline]
            pub fn div(self, n: $t) -> $t {
                let s = n >> (<$t>::BITS - 1);
                let qm = self.mulhi((n ^ s).wrapping_sub(s) as $u) as $t;
                let qs = s ^ self.sign;
                (qm ^ qs).wrapping_sub(qs)
            }

            #[inline]
            pub fn rem(self, n: $t) -> $t {
                n.wrapping_sub(self.div(n).wrapping_mul(self.d))
            }

            /// `n.div_euclid(d)` for `d > 0`
            #[inline]
            pub fn floor(self, n: $t) -> $t {
                let s = n >> (<$t>::BITS - 1);
                (self.mulhi((n ^ s) as $u) as $t) ^ s
            }
        }
    };
}

forms!(DevI32, MagI32, i32, u32, u64);
forms!(DevI64, MagI64, i64, u64, u128);

#[derive(Clone, Copy)]
pub struct F64I32 {
    rcp: f64,
    d: i32,
}

impl F64I32 {
    /// `|d|` must be at least 2
    pub fn new(d: i32) -> Self {
        let up = f64::from_bits((1.0 / d.unsigned_abs() as f64).to_bits() + 1);
        Self {
            rcp: if d < 0 { -up } else { up },
            d,
        }
    }

    #[inline]
    pub fn div(self, n: i32) -> i32 {
        // SAFETY: |n rcp| < 2^31 for |d| >= 2
        unsafe { (n as f64 * self.rcp).to_int_unchecked::<i32>() }
    }

    #[inline]
    pub fn rem(self, n: i32) -> i32 {
        n.wrapping_sub(self.div(n).wrapping_mul(self.d))
    }
}

macro_rules! map {
    ($name:ident, $d:ty, $t:ty, $f:ident) => {
        #[inline(never)]
        fn $name(d: $d, a: &[$t]) -> Vec<$t> {
            a.iter().map(move |x| d.$f(*x)).collect()
        }
    };
}

map!(mag32_div, MagI32, i32, div);
map!(mag32_rem, MagI32, i32, rem);
map!(mag32_floor, MagI32, i32, floor);
map!(f64_div, F64I32, i32, div);
map!(f64_rem, F64I32, i32, rem);
map!(dev32_rem, DevI32, i32, rem);
map!(mag64_div, MagI64, i64, div);
map!(mag64_rem, MagI64, i64, rem);
map!(mag64_floor, MagI64, i64, floor);
map!(dev64_rem, DevI64, i64, rem);

macro_rules! pr_kernels {
    ($div:ident, $rem:ident, $floor:ident, $enum:ident, $t:ty) => {
        #[inline(never)]
        fn $div(d: $enum, a: &[$t]) -> Vec<$t> {
            match d {
                $enum::PowerOfTwo(d) => a.iter().map(move |x| d.wrapping_div_rem(*x).0).collect(),
                $enum::Multiplier(d) => a.iter().map(move |x| d.wrapping_div_rem(*x).0).collect(),
            }
        }
        #[inline(never)]
        fn $rem(d: $enum, a: &[$t]) -> Vec<$t> {
            match d {
                $enum::PowerOfTwo(d) => a.iter().map(move |x| d.wrapping_div_rem(*x).1).collect(),
                $enum::Multiplier(d) => a.iter().map(move |x| d.wrapping_div_rem(*x).1).collect(),
            }
        }
        /// `div_euclid` by a positive divisor: one less where the remainder is negative
        #[inline(never)]
        fn $floor(d: $enum, a: &[$t]) -> Vec<$t> {
            match d {
                $enum::PowerOfTwo(d) => a
                    .iter()
                    .map(move |x| {
                        let (q, r) = d.wrapping_div_rem(*x);
                        q + (r >> (<$t>::BITS - 1))
                    })
                    .collect(),
                $enum::Multiplier(d) => a
                    .iter()
                    .map(move |x| {
                        let (q, r) = d.wrapping_div_rem(*x);
                        q + (r >> (<$t>::BITS - 1))
                    })
                    .collect(),
            }
        }
    };
}

pr_kernels!(pr32_div, pr32_rem, pr32_floor, DivisorI32, i32);
pr_kernels!(pr64_div, pr64_rem, pr64_floor, DivisorI64, i64);

/// aarch64: the PR's figure 5.1 with `sqdmulh`, which returns `(2 a b) >> 32` for 4 lanes, so
/// one more shift right gives `MULSH(m', n)`, folded into `ssra`. LLVM writes `MULSH` as `smull`,
/// `smull2` and `uzp2`. It saturates only for `MIN * MIN`, and `m'` is above `MIN` for every
/// divisor that isn't a power of two. The remainder is devanbenz's, `n - q0 |d|`, with `mls`.
#[cfg(target_arch = "aarch64")]
mod sqd {
    use super::DevI32;
    use core::arch::aarch64::*;

    #[inline(never)]
    pub fn div(d: DevI32, a: &[i32]) -> Vec<i32> {
        kernel::<false>(d, a)
    }

    #[inline(never)]
    pub fn rem(d: DevI32, a: &[i32]) -> Vec<i32> {
        kernel::<true>(d, a)
    }

    #[inline(always)]
    fn kernel<const REM: bool>(d: DevI32, a: &[i32]) -> Vec<i32> {
        let sign = d.divisor >> 31;
        let magnitude = (d.divisor ^ sign) - sign;
        let mut out: Vec<i32> = Vec::with_capacity(a.len());
        let o = out.as_mut_ptr();
        let full = a.len() / 4 * 4;
        // SAFETY: NEON is part of aarch64, the loads read a[..full] and the stores write the
        // first a.len() slots of the capacity
        unsafe {
            let m = vdupq_n_s32(d.multiplier);
            let sh = vdupq_n_s32(-(d.shift as i32));
            let s = vdupq_n_s32(sign);
            let mag = vdupq_n_s32(magnitude);
            for i in (0..full).step_by(4) {
                let n = vld1q_s32(a.as_ptr().add(i));
                let q = vshlq_s32(vsraq_n_s32::<1>(n, vqdmulhq_s32(n, m)), sh);
                // q0 = q - (n >> 31), with a logical shift that adds 1 for a negative n
                let q0 = vreinterpretq_s32_u32(vsraq_n_u32::<31>(
                    vreinterpretq_u32_s32(q),
                    vreinterpretq_u32_s32(n),
                ));
                let r = if REM {
                    vmlsq_s32(n, q0, mag)
                } else {
                    vsubq_s32(veorq_s32(q0, s), s)
                };
                vst1q_s32(o.add(i), r);
            }
            for i in full..a.len() {
                let n = a[i];
                let high = ((d.multiplier as i64 * n as i64) >> 32) as i32;
                let q0 = ((n + high) >> d.shift) - (n >> 31);
                *o.add(i) = if REM {
                    n - q0 * magnitude
                } else {
                    (q0 ^ sign) - sign
                };
            }
            out.set_len(a.len());
        }
        out
    }

    /// Both kernels against `wrapping_div` / `wrapping_rem` at the ends of steps
    pub fn check(rng: &mut super::Rng) -> u64 {
        let mut ds: Vec<i32> = (3..3000).collect();
        ds.extend([i32::MAX, i32::MAX - 1, 641, 6700417]);
        ds.extend((0..5000).map(|_| (rng.next() >> (33 + rng.next() % 30)) as i32));
        let mut checked = 0;
        for d in ds {
            for d in [d, d.wrapping_neg()] {
                let ad = d.unsigned_abs();
                if ad < 3 || ad.is_power_of_two() {
                    continue;
                }
                let ad = ad as i64;
                let last = (1i64 << 31) / ad;
                let mut ks: Vec<i64> = (0..8).chain(last - 8..=last).collect();
                ks.extend((0..16).map(|_| (rng.next() % (last as u64 + 1)) as i64));
                let ns: Vec<i32> = ks
                    .iter()
                    .flat_map(|k| [k * ad, k * ad + ad - 1, -(k * ad), -(k * ad + ad - 1)])
                    .filter(|n| (i32::MIN as i64..=i32::MAX as i64).contains(n))
                    .map(|n| n as i32)
                    .collect();
                let dev = DevI32::new(d);
                let (q, r) = (div(dev, &ns), rem(dev, &ns));
                for (i, &n) in ns.iter().enumerate() {
                    assert_eq!(
                        (q[i], r[i]),
                        (n.wrapping_div(d), n.wrapping_rem(d)),
                        "sqd {n} / {d}"
                    );
                }
                checked += ns.len() as u64;
            }
        }
        checked
    }
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }
}

/// Every form is monotone in `n` between sign changes, so the ends of the steps decide it
macro_rules! check {
    ($rng:expr, $t:ty, $mag:ident, $dev:ident, $f64:expr) => {{
        let bits = <$t>::BITS;
        let mut ds: Vec<$t> = (3..3000).collect();
        ds.extend([<$t>::MAX, <$t>::MAX - 1, 641, 6700417]);
        ds.extend((0..5000).map(|_| {
            ($rng.next() >> (64 - bits + 1 + ($rng.next() % (bits as u64 - 2)) as u32)) as $t
        }));
        let mut checked = 0u64;
        for d in ds {
            for d in [d, d.wrapping_neg()] {
                let ad = d.unsigned_abs();
                if ad < 3 || ad.is_power_of_two() {
                    continue;
                }
                let (m, v) = ($mag::new(d), $dev::new(d));
                let ad = ad as i128;
                let last = (1i128 << (bits - 1)) / ad;
                let mut ks: Vec<i128> = (0..8).chain(last - 8..=last).collect();
                ks.extend((0..16).map(|_| ($rng.next() as u128 % (last as u128 + 1)) as i128));
                for k in ks {
                    for n in [k * ad, k * ad + ad - 1, -(k * ad), -(k * ad + ad - 1)] {
                        if n < <$t>::MIN as i128 || n > <$t>::MAX as i128 {
                            continue;
                        }
                        let n = n as $t;
                        let want = (n.wrapping_div(d), n.wrapping_rem(d));
                        assert_eq!((m.div(n), m.rem(n)), want, "mag {n} / {d}");
                        assert_eq!(v.rem(n), want.1, "dev {n} % {d}");
                        if d > 0 {
                            assert_eq!(m.floor(n), n.div_euclid(d), "floor {n} / {d}");
                        }
                        $f64(d, n, want);
                        checked += 1;
                    }
                }
            }
        }
        checked
    }};
}

fn time<T>(f: &mut dyn FnMut() -> Vec<T>) -> (f64, Vec<T>) {
    let t = Instant::now();
    let mut out = Vec::new();
    for _ in 0..200 {
        out = black_box(f());
    }
    (t.elapsed().as_nanos() as f64 / (200 * SIZE) as f64, out)
}

type Kernel<'a, T> = (&'static str, Box<dyn FnMut() -> Vec<T> + 'a>);

fn run<T: PartialEq + std::fmt::Debug>(label: &str, ks: &mut [Kernel<'_, T>]) {
    let mut best = vec![f64::INFINITY; ks.len()];
    let mut outs: Vec<Vec<T>> = ks.iter().map(|_| Vec::new()).collect();
    for _ in 0..15 {
        for (i, (_, k)) in ks.iter_mut().enumerate() {
            let (t, o) = time(k.as_mut());
            best[i] = best[i].min(t);
            outs[i] = o;
        }
    }
    for o in &outs[1..] {
        assert!(o == &outs[0], "{label}: forms disagree");
    }
    print!("{label:<16}");
    for ((name, _), b) in ks.iter().zip(&best) {
        print!(" {name}={b:.3}");
    }
    println!();
}

fn main() {
    let mut rng = Rng(7);
    let c32 = check!(
        rng,
        i32,
        MagI32,
        DevI32,
        |d: i32, n: i32, want: (i32, i32)| {
            let f = F64I32::new(d);
            assert_eq!((f.div(n), f.rem(n)), want, "f64 {n} / {d}");
        }
    );
    let c64 = check!(rng, i64, MagI64, DevI64, |_: i64, _: i64, _: (i64, i64)| {});
    println!("checked {c32} i32 and {c64} i64 dividends");
    #[cfg(target_arch = "aarch64")]
    println!(
        "checked {} i32 dividends with sqdmulh",
        sqd::check(&mut rng)
    );

    let a32: Vec<i32> = (0..SIZE).map(|_| rng.next() as i32).collect();
    let a64: Vec<i64> = (0..SIZE).map(|_| rng.next() as i64).collect();
    for dv in [13i32, -1000] {
        let dv = black_box(dv);
        let (p, v, m, f) = (
            DivisorI32::new(dv).unwrap(),
            black_box(DevI32::new(dv)),
            black_box(MagI32::new(dv)),
            black_box(F64I32::new(dv)),
        );
        let mut div: Vec<Kernel<'_, i32>> = vec![
            ("pr", Box::new(|| pr32_div(p, &a32))),
            ("mag", Box::new(|| mag32_div(m, &a32))),
            ("f64", Box::new(|| f64_div(f, &a32))),
        ];
        #[cfg(target_arch = "aarch64")]
        div.push(("sqd", Box::new(|| sqd::div(v, &a32))));
        run(&format!("i32 / {dv}"), &mut div);
        let mut rem: Vec<Kernel<'_, i32>> = vec![
            ("pr", Box::new(|| pr32_rem(p, &a32))),
            ("dev", Box::new(|| dev32_rem(v, &a32))),
            ("mag", Box::new(|| mag32_rem(m, &a32))),
            ("f64", Box::new(|| f64_rem(f, &a32))),
        ];
        #[cfg(target_arch = "aarch64")]
        rem.push(("sqd", Box::new(|| sqd::rem(v, &a32))));
        run(&format!("i32 % {dv}"), &mut rem);
        if dv > 0 {
            run(
                &format!("i32 floor {dv}"),
                &mut [
                    ("pr", Box::new(|| pr32_floor(p, &a32))),
                    ("mag", Box::new(|| mag32_floor(m, &a32))),
                ],
            );
        }
    }
    for dv in [13i64, -1000] {
        let dv = black_box(dv);
        let (p, v, m) = (
            DivisorI64::new(dv).unwrap(),
            black_box(DevI64::new(dv)),
            black_box(MagI64::new(dv)),
        );
        run(
            &format!("i64 / {dv}"),
            &mut [
                ("pr", Box::new(|| pr64_div(p, &a64))),
                ("mag", Box::new(|| mag64_div(m, &a64))),
            ],
        );
        run(
            &format!("i64 % {dv}"),
            &mut [
                ("pr", Box::new(|| pr64_rem(p, &a64))),
                ("dev", Box::new(|| dev64_rem(v, &a64))),
                ("mag", Box::new(|| mag64_rem(m, &a64))),
            ],
        );
        if dv > 0 {
            run(
                &format!("i64 floor {dv}"),
                &mut [
                    ("pr", Box::new(|| pr64_floor(p, &a64))),
                    ("mag", Box::new(|| mag64_floor(m, &a64))),
                ],
            );
        }
    }
}
