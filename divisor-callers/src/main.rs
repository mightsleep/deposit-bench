//! Three real loops with a run-time divisor, each as it is today (`op`), with the PR's enum
//! matched once outside the loop (`pr`) and with the opaque divisor called per value (`pv`):
//!
//! - arrow `integer_op` `Op::Div` with a scalar divisor: `try_unary` with `div_checked`, with
//!   and without nulls (with nulls `try_unary` calls the closure per valid index)
//! - DataFusion `date_bin` for nanosecond timestamps: `try_unary` over a closure that calls the
//!   binning function through a function pointer, `rem_euclid` by the stride
//! - Comet `might_contain_longs`: two murmur3 hashes per value, then up to `k` probes, each
//!   `% bit_size`, with an early exit; the divisor is a field of the filter
//!
//! `date_bin` has no `pr`: the binning function is a function pointer, so matching outside
//! would need one function per variant.
//!
//! `pr.rs` is `arrow-buffer/src/divisor.rs` at the PR head (6f639a36c) and `opaque.rs` the same
//! file with the opaque-divisor patch, both without their tests. Arrow itself is 60.0.0.
//!
//! Prints the minimum ns per value over interleaved rounds.

#[allow(dead_code)]
mod opaque;
#[allow(dead_code)]
mod pr;

use arrow_array::types::{Int64Type, TimestampNanosecondType};
use arrow_array::{
    ArrowNativeTypeOp, BooleanArray, Int64Array, PrimitiveArray, TimestampNanosecondArray,
};
use arrow_schema::ArrowError;
use opaque::{DivisorI32, DivisorI64};
use std::hint::black_box;
use std::time::Instant;

const SIZE: usize = 8192;

// arrow integer_op, Op::Div with a scalar on the right

fn overflow(l: i64, r: i64) -> ArrowError {
    ArrowError::ArithmeticOverflow(format!("Overflow happened on: {l:?} / {r:?}"))
}

#[inline(never)]
fn div_op(a: &Int64Array, r: i64) -> Result<Int64Array, ArrowError> {
    a.try_unary(|l| l.div_checked(r))
}

#[inline(never)]
fn div_pr(a: &Int64Array, r: i64, d: pr::DivisorI64) -> Result<Int64Array, ArrowError> {
    match d {
        pr::DivisorI64::PowerOfTwo(d) => a.try_unary(move |l| {
            d.checked_div_rem(l)
                .map(|(q, _)| q)
                .ok_or_else(|| overflow(l, r))
        }),
        pr::DivisorI64::Multiplier(d) => a.try_unary(move |l| Ok(d.wrapping_div_rem(l).0)),
    }
}

#[inline(never)]
fn div_pv(a: &Int64Array, r: i64, d: DivisorI64) -> Result<Int64Array, ArrowError> {
    a.try_unary(move |l| {
        d.checked_div_rem(l)
            .map(|(q, _)| q)
            .ok_or_else(|| overflow(l, r))
    })
}

// DataFusion date_bin, nanosecond timestamps and a nanosecond stride

type Result64 = Result<i64, ArrowError>;

#[derive(Clone, Copy)]
struct BinFunctions {
    narrow: fn(i64, i64, i64) -> Result64,
}

#[derive(Clone, Copy)]
struct Stride {
    nanos: i64,
    divisor: DivisorI64,
}

#[derive(Clone, Copy)]
struct BinFunctionsPv {
    narrow: fn(Stride, i64, i64) -> Result64,
}

fn invalid(msg: String) -> ArrowError {
    ArrowError::InvalidArgumentError(msg)
}

fn date_bin_nanos_interval(stride_nanos: i64, source: i64, origin: i64) -> Result64 {
    let time_diff = source.checked_sub(origin).ok_or_else(|| {
        invalid(format!(
            "date_bin source timestamp {source} - origin {origin} overflows i64"
        ))
    })?;
    let time_delta = compute_distance(time_diff, stride_nanos)?;
    origin.checked_add(time_delta).ok_or_else(|| {
        invalid(format!(
            "date_bin origin {origin} + delta {time_delta} overflows i64"
        ))
    })
}

fn compute_distance(time_diff: i64, stride: i64) -> Result64 {
    let remainder = time_diff.rem_euclid(stride);
    time_diff.checked_sub(remainder).ok_or_else(|| {
        invalid(format!(
            "date_bin compute_distance time_diff {time_diff} - remainder {remainder} overflows i64"
        ))
    })
}

fn date_bin_nanos_interval_pv(stride: Stride, source: i64, origin: i64) -> Result64 {
    let time_diff = source.checked_sub(origin).ok_or_else(|| {
        invalid(format!(
            "date_bin source timestamp {source} - origin {origin} overflows i64"
        ))
    })?;
    let time_delta = compute_distance_pv(time_diff, stride)?;
    origin.checked_add(time_delta).ok_or_else(|| {
        invalid(format!(
            "date_bin origin {origin} + delta {time_delta} overflows i64"
        ))
    })
}

fn compute_distance_pv(time_diff: i64, stride: Stride) -> Result64 {
    // the stride is positive, so a negative remainder is moved up by one stride
    let r = stride.divisor.wrapping_div_rem(time_diff).1;
    let remainder = r + ((r >> 63) & stride.nanos);
    time_diff.checked_sub(remainder).ok_or_else(|| {
        invalid(format!(
            "date_bin compute_distance time_diff {time_diff} - remainder {remainder} overflows i64"
        ))
    })
}

#[inline(never)]
fn date_bin_op(
    a: &TimestampNanosecondArray,
    origin: i64,
    stride: i64,
    f: BinFunctions,
) -> Result<TimestampNanosecondArray, ArrowError> {
    // scale is 1 for nanoseconds, so the wide path is never taken
    a.try_unary(|val| (f.narrow)(stride, val, origin))
}

#[inline(never)]
fn date_bin_pv(
    a: &TimestampNanosecondArray,
    origin: i64,
    stride: Stride,
    f: BinFunctionsPv,
) -> Result<TimestampNanosecondArray, ArrowError> {
    a.try_unary(|val| (f.narrow)(stride, val, origin))
}

// Comet might_contain_longs, V1 (i32) and V2 (i64) scattering

fn murmur3(data: [u8; 8], seed: u32) -> u32 {
    fn mix_k1(k1: i32) -> i32 {
        k1.wrapping_mul(0xcc9e2d51u32 as i32)
            .rotate_left(15)
            .wrapping_mul(0x1b873593u32 as i32)
    }
    fn mix_h1(h1: i32, k1: i32) -> i32 {
        (h1 ^ k1)
            .rotate_left(13)
            .wrapping_mul(5)
            .wrapping_add(0xe6546b64u32 as i32)
    }
    let mut h1 = seed as i32;
    for w in data.chunks_exact(4) {
        h1 = mix_h1(h1, mix_k1(i32::from_le_bytes(w.try_into().unwrap())));
    }
    h1 ^= 8;
    h1 ^= (h1 as u32 >> 16) as i32;
    h1 = h1.wrapping_mul(0x85ebca6bu32 as i32);
    h1 ^= (h1 as u32 >> 13) as i32;
    h1 = h1.wrapping_mul(0xc2b2ae35u32 as i32);
    h1 ^= (h1 as u32 >> 16) as i32;
    h1 as u32
}

struct Bloom {
    data: Vec<u64>,
    k: u32,
    v2: bool,
    div32: DivisorI32,
    div64: DivisorI64,
    pr32: pr::DivisorI32,
    pr64: pr::DivisorI64,
}

impl Bloom {
    fn new(words: usize, k: u32, v2: bool, fill: &mut impl FnMut() -> u64) -> Self {
        let bits = (words * 64) as i64;
        Self {
            data: (0..words).map(|_| fill()).collect(),
            k,
            v2,
            div32: DivisorI32::new(bits as i32).unwrap(),
            div64: DivisorI64::new(bits).unwrap(),
            pr32: pr::DivisorI32::new(bits as i32).unwrap(),
            pr64: pr::DivisorI64::new(bits).unwrap(),
        }
    }

    fn bit_size(&self) -> u64 {
        self.data.len() as u64 * 64
    }

    fn get(&self, index: usize) -> bool {
        (self.data[index >> 6] & (1u64 << (index & 0x3f))) != 0
    }

    fn hashes(x: i64) -> (u32, u32) {
        let h1 = murmur3(x.to_le_bytes(), 0);
        (h1, murmur3(x.to_le_bytes(), h1))
    }

    /// `index(combined)` is `combined % bit_size` for a non-negative `combined`
    fn v1(&self, h1: u32, h2: u32, index: impl Fn(i32) -> i32) -> bool {
        for i in 1..=self.k {
            let mut combined = (h1 as i32).wrapping_add((i as i32).wrapping_mul(h2 as i32));
            if combined < 0 {
                combined = !combined;
            }
            if !self.get(index(combined) as usize) {
                return false;
            }
        }
        true
    }

    fn v2(&self, h1: u32, h2: u32, index: impl Fn(i64) -> i64) -> bool {
        let mut combined = (h1 as i32 as i64).wrapping_mul(i32::MAX as i64);
        let h2 = h2 as i32 as i64;
        for _ in 0..self.k {
            combined = combined.wrapping_add(h2);
            let c = if combined < 0 { !combined } else { combined };
            if !self.get(index(c) as usize) {
                return false;
            }
        }
        true
    }

    fn contains_op(&self, x: i64) -> bool {
        let (h1, h2) = Self::hashes(x);
        if self.v2 {
            let bit_size = self.bit_size() as i64;
            self.v2(h1, h2, |c| c % bit_size)
        } else {
            let bit_size = self.bit_size() as i32;
            self.v1(h1, h2, |c| c % bit_size)
        }
    }

    fn contains_pv(&self, x: i64) -> bool {
        let (h1, h2) = Self::hashes(x);
        if self.v2 {
            self.v2(h1, h2, |c| self.div64.wrapping_div_rem(c).1)
        } else {
            self.v1(h1, h2, |c| self.div32.wrapping_div_rem(c).1)
        }
    }
}

#[inline(never)]
fn bloom_op(f: &Bloom, items: &Int64Array) -> BooleanArray {
    items.iter().map(|v| v.map(|x| f.contains_op(x))).collect()
}

#[inline(never)]
fn bloom_pv(f: &Bloom, items: &Int64Array) -> BooleanArray {
    items.iter().map(|v| v.map(|x| f.contains_pv(x))).collect()
}

/// Matching outside the values loop means one loop per version and variant
#[inline(never)]
fn bloom_pr(f: &Bloom, items: &Int64Array) -> BooleanArray {
    fn all<I: Fn(i64, u32, u32) -> bool>(items: &Int64Array, probe: I) -> BooleanArray {
        items
            .iter()
            .map(|v| {
                v.map(|x| {
                    let (h1, h2) = Bloom::hashes(x);
                    probe(x, h1, h2)
                })
            })
            .collect()
    }
    match (f.v2, f.pr32, f.pr64) {
        (false, pr::DivisorI32::PowerOfTwo(d), _) => {
            all(items, |_, h1, h2| f.v1(h1, h2, |c| d.wrapping_div_rem(c).1))
        }
        (false, pr::DivisorI32::Multiplier(d), _) => {
            all(items, |_, h1, h2| f.v1(h1, h2, |c| d.wrapping_div_rem(c).1))
        }
        (true, _, pr::DivisorI64::PowerOfTwo(d)) => {
            all(items, |_, h1, h2| f.v2(h1, h2, |c| d.wrapping_div_rem(c).1))
        }
        (true, _, pr::DivisorI64::Multiplier(d)) => {
            all(items, |_, h1, h2| f.v2(h1, h2, |c| d.wrapping_div_rem(c).1))
        }
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

type Kernel<'a, T> = (&'static str, Box<dyn FnMut() -> T + 'a>);

/// min ns per value over the rounds, variants interleaved within each round
fn run<T: PartialEq + std::fmt::Debug>(label: &str, ks: &mut [Kernel<'_, T>]) {
    const ITERS: usize = 100;
    let rounds: usize = std::env::var("ROUNDS")
        .ok()
        .map_or(15, |v| v.parse().unwrap());
    let mut best = vec![f64::INFINITY; ks.len()];
    let mut outs: Vec<Option<T>> = ks.iter().map(|_| None).collect();
    for _ in 0..rounds {
        for (i, (_, k)) in ks.iter_mut().enumerate() {
            let t = Instant::now();
            for _ in 0..ITERS {
                outs[i] = Some(black_box(k()));
            }
            let ns = t.elapsed().as_nanos() as f64 / (ITERS * SIZE) as f64;
            best[i] = best[i].min(ns);
        }
    }
    for o in &outs[1..] {
        assert_eq!(o, &outs[0], "{label}: variants disagree");
    }
    print!("{label:<26}");
    for ((name, _), b) in ks.iter().zip(&best) {
        print!(" {name}={b:.3}");
    }
    println!();
}

fn with_nulls(rng: &mut Rng, values: Vec<i64>, nulls: bool) -> Int64Array {
    if nulls {
        values
            .into_iter()
            .map(|v| (rng.next() % 10 != 0).then_some(v))
            .collect()
    } else {
        PrimitiveArray::<Int64Type>::from(values)
    }
}

fn main() {
    let mut rng = Rng(7);

    for nulls in [false, true] {
        let values: Vec<i64> = (0..SIZE).map(|_| rng.next() as i64).collect();
        let a = with_nulls(&mut rng, values, nulls);
        for r in [13i64, 16, 1000] {
            let r = black_box(r);
            let (p, d) = (pr::DivisorI64::new(r).unwrap(), DivisorI64::new(r).unwrap());
            let label = format!("div i64 / {r} nulls={nulls}");
            run(
                &label,
                &mut [
                    ("op", Box::new(|| div_op(&a, r).unwrap())),
                    ("pr", Box::new(|| div_pr(&a, r, p).unwrap())),
                    ("pv", Box::new(|| div_pv(&a, r, d).unwrap())),
                ],
            );
        }
    }

    // timestamps within a few years of 2020, like a real column
    let base = 1_577_836_800_000_000_000i64;
    let ts: TimestampNanosecondArray = PrimitiveArray::<TimestampNanosecondType>::from(
        (0..SIZE)
            .map(|_| base + (rng.next() % (4 * 365 * 86_400_000_000_000)) as i64)
            .collect::<Vec<_>>(),
    );
    let f = black_box(BinFunctions {
        narrow: date_bin_nanos_interval,
    });
    let fpv = black_box(BinFunctionsPv {
        narrow: date_bin_nanos_interval_pv,
    });
    for (name, stride) in [("15 min", 900_000_000_000i64), ("1 h", 3_600_000_000_000)] {
        let stride = black_box(stride);
        let s = Stride {
            nanos: stride,
            divisor: DivisorI64::new(stride).unwrap(),
        };
        let origin = black_box(0);
        run(
            &format!("date_bin {name}"),
            &mut [
                (
                    "op",
                    Box::new(|| date_bin_op(&ts, origin, stride, f).unwrap()),
                ),
                ("pv", Box::new(|| date_bin_pv(&ts, origin, s, fpv).unwrap())),
            ],
        );
    }

    for v2 in [false, true] {
        // a filter of about 0.9 MB with half of its bits set and one of 128 KB, the size
        // Spark picks for 1M and about 140k items at 3% false positives
        for words in [114_039usize, 16_384] {
            let mut fill = || rng.next() & rng.next() | rng.next() & rng.next();
            let filter = black_box(Bloom::new(words, 5, v2, &mut fill));
            let values = (0..SIZE).map(|_| rng.next() as i64).collect();
            let items = with_nulls(&mut rng, values, true);
            let label = format!("bloom v{} {} bits", 1 + v2 as u8, words * 64);
            run(
                &label,
                &mut [
                    ("op", Box::new(|| bloom_op(&filter, &items))),
                    ("pr", Box::new(|| bloom_pr(&filter, &items))),
                    ("pv", Box::new(|| bloom_pv(&filter, &items))),
                ],
            );
        }
    }
}
