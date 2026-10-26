//! u32 division by a divisor known only at run time, many values per divisor.
//!
//! - `op`: `/` and `%` with the divisor hidden from the compiler (a divide instruction)
//! - `pr`: arrow-rs PR #11467 (`DivisorU32`): a power-of-two variant and Granlund-Montgomery
//!   figure 4.1, `q = (t + ((n - t) >> 1)) >> sh` with `t = mulhi(m', n)`, matched once outside
//!   the loop
//! - `split`: the same multiplier in two shapes, chosen per divisor. If the rounded-up
//!   multiplier `a = ceil(2^(32+p) / d)` is exact for every u32 without the add (about ln 2, 69%,
//!   of divisors), `q = mulhi(a, n) >> p`. Otherwise round down and bump the dividend,
//!   `q = mulhi(a, n.saturating_add(1)) >> p` (ridiculous_fish, "Labor of Division
//!   (Episode III)"); the saturation only matters for n = u32::MAX, which is never a multiple of
//!   a non-power-of-two d. Both are a plain mulhi, which stays in u32 lanes.
//!
//! 8192 random values per batch, min ns per value over interleaved rounds, after a 2 s
//! warm-up. Every kernel is checked against `/` and `%`.

use std::hint::black_box;
use std::time::Instant;

const SIZE: usize = 8192;

// ---------------------------------------------------------------------------------------------
// PR #11467

#[derive(Clone, Copy)]
enum Pr {
    PowerOfTwo {
        shift: u32,
        mask: u32,
    },
    Multiplier {
        divisor: u32,
        multiplier: u32,
        shift: u32,
    },
}

impl Pr {
    fn new(divisor: u32) -> Self {
        if divisor.is_power_of_two() {
            return Pr::PowerOfTwo {
                shift: divisor.trailing_zeros(),
                mask: divisor - 1,
            };
        }
        let l = u32::BITS - (divisor - 1).leading_zeros();
        let numerator = ((1u64 << l) - divisor as u64) << u32::BITS;
        Pr::Multiplier {
            divisor,
            multiplier: (numerator / divisor as u64 + 1) as u32,
            shift: l - 1,
        }
    }
}

#[inline(always)]
fn pr_mul(divisor: u32, multiplier: u32, shift: u32, n: u32) -> (u32, u32) {
    let t = ((multiplier as u64 * n as u64) >> u32::BITS) as u32;
    let q = (t + ((n - t) >> 1)) >> shift;
    (q, n - q * divisor)
}

#[inline(never)]
fn pr_div(d: Pr, src: &[u32], dst: &mut [u32]) {
    match d {
        Pr::PowerOfTwo { shift, .. } => dst
            .iter_mut()
            .zip(src)
            .for_each(move |(o, n)| *o = n >> shift),
        Pr::Multiplier {
            divisor,
            multiplier,
            shift,
        } => dst
            .iter_mut()
            .zip(src)
            .for_each(move |(o, n)| *o = pr_mul(divisor, multiplier, shift, *n).0),
    }
}

#[inline(never)]
fn pr_rem(d: Pr, src: &[u32], dst: &mut [u32]) {
    match d {
        Pr::PowerOfTwo { mask, .. } => dst
            .iter_mut()
            .zip(src)
            .for_each(move |(o, n)| *o = n & mask),
        Pr::Multiplier {
            divisor,
            multiplier,
            shift,
        } => dst
            .iter_mut()
            .zip(src)
            .for_each(move |(o, n)| *o = pr_mul(divisor, multiplier, shift, *n).1),
    }
}

// ---------------------------------------------------------------------------------------------
// split

#[derive(Clone, Copy)]
enum Split {
    PowerOfTwo { shift: u32, mask: u32 },
    Up { a: u32, p: u32, d: u32 },
    Down { a: u32, p: u32, d: u32 },
}

impl Split {
    fn new(d: u32) -> Self {
        if d.is_power_of_two() {
            return Split::PowerOfTwo {
                shift: d.trailing_zeros(),
                mask: d - 1,
            };
        }
        // s = 32 + p, p = floor(log2 d): the rounded-up multiplier is exact without the add
        // when its error e = a d - 2^s is at most 2^p
        let p = u32::BITS - 1 - d.leading_zeros();
        let pow = 1u64 << (32 + p);
        let down = pow / d as u64;
        if (down + 1) * d as u64 - pow <= 1 << p {
            Split::Up {
                a: (down + 1) as u32,
                p,
                d,
            }
        } else {
            Split::Down {
                a: down as u32,
                p,
                d,
            }
        }
    }
    fn name(self) -> &'static str {
        match self {
            Split::PowerOfTwo { .. } => "power of two",
            Split::Up { .. } => "round up",
            Split::Down { .. } => "round down + 1",
        }
    }
}

#[inline(always)]
fn up(a: u32, p: u32, d: u32, n: u32) -> (u32, u32) {
    let q = (((a as u64 * n as u64) >> 32) as u32) >> p;
    (q, n - q * d)
}

#[inline(always)]
fn down(a: u32, p: u32, d: u32, n: u32) -> (u32, u32) {
    let m = n.saturating_add(1);
    let q = (((a as u64 * m as u64) >> 32) as u32) >> p;
    (q, n - q * d)
}

/// round down + 1 as a n + a in the 64-bit product: no saturation, exact for n = u32::MAX
#[inline(always)]
fn down2(a: u32, p: u32, d: u32, n: u32) -> (u32, u32) {
    let q = (((a as u64 * n as u64 + a as u64) >> 32) as u32) >> p;
    (q, n - q * d)
}

#[inline(never)]
fn down2_div(d: Split, src: &[u32], dst: &mut [u32]) {
    match d {
        Split::Down { a, p, d } => dst.iter_mut().zip(src).for_each(move |(o, n)| *o = down2(a, p, d, *n).0),
        other => split_div(other, src, dst),
    }
}

#[inline(never)]
fn down2_rem(d: Split, src: &[u32], dst: &mut [u32]) {
    match d {
        Split::Down { a, p, d } => dst.iter_mut().zip(src).for_each(move |(o, n)| *o = down2(a, p, d, *n).1),
        other => split_rem(other, src, dst),
    }
}

#[inline(never)]
fn split_div(d: Split, src: &[u32], dst: &mut [u32]) {
    match d {
        Split::PowerOfTwo { shift, .. } => dst
            .iter_mut()
            .zip(src)
            .for_each(move |(o, n)| *o = n >> shift),
        Split::Up { a, p, d } => dst
            .iter_mut()
            .zip(src)
            .for_each(move |(o, n)| *o = up(a, p, d, *n).0),
        Split::Down { a, p, d } => dst
            .iter_mut()
            .zip(src)
            .for_each(move |(o, n)| *o = down(a, p, d, *n).0),
    }
}

#[inline(never)]
fn split_rem(d: Split, src: &[u32], dst: &mut [u32]) {
    match d {
        Split::PowerOfTwo { mask, .. } => dst
            .iter_mut()
            .zip(src)
            .for_each(move |(o, n)| *o = n & mask),
        Split::Up { a, p, d } => dst
            .iter_mut()
            .zip(src)
            .for_each(move |(o, n)| *o = up(a, p, d, *n).1),
        Split::Down { a, p, d } => dst
            .iter_mut()
            .zip(src)
            .for_each(move |(o, n)| *o = down(a, p, d, *n).1),
    }
}

/// round up where it is exact, the PR's add-indicator form for the other divisors: never slower
/// than the PR, also where a saturating add is expensive (SSE2 has none for u32)
#[derive(Clone, Copy)]
enum UpOrPr {
    Up { a: u32, p: u32, d: u32 },
    Pr(Pr),
}

impl UpOrPr {
    fn new(d: u32) -> Self {
        match Split::new(d) {
            Split::Up { a, p, d } => UpOrPr::Up { a, p, d },
            _ => UpOrPr::Pr(Pr::new(d)),
        }
    }
}

#[inline(never)]
fn uop_div(d: UpOrPr, src: &[u32], dst: &mut [u32]) {
    match d {
        UpOrPr::Up { a, p, d } => dst.iter_mut().zip(src).for_each(move |(o, n)| *o = up(a, p, d, *n).0),
        UpOrPr::Pr(p) => pr_div(p, src, dst),
    }
}

#[inline(never)]
fn uop_rem(d: UpOrPr, src: &[u32], dst: &mut [u32]) {
    match d {
        UpOrPr::Up { a, p, d } => dst.iter_mut().zip(src).for_each(move |(o, n)| *o = up(a, p, d, *n).1),
        UpOrPr::Pr(p) => pr_rem(p, src, dst),
    }
}

#[inline(never)]
fn op_div(d: u32, src: &[u32], dst: &mut [u32]) {
    dst.iter_mut().zip(src).for_each(move |(o, n)| *o = n / d);
}

#[inline(never)]
fn op_rem(d: u32, src: &[u32], dst: &mut [u32]) {
    dst.iter_mut().zip(src).for_each(move |(o, n)| *o = n % d);
}

// ---------------------------------------------------------------------------------------------

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

fn warm_up() {
    let t = Instant::now();
    let mut x = 0u64;
    while t.elapsed().as_millis() < 2000 {
        for i in 0..1_000_000u64 {
            x = black_box(x.wrapping_mul(6364136223846793005).wrapping_add(i));
        }
    }
    black_box(x);
}

type Kernel<'a> = Box<dyn Fn(&[u32], &mut [u32]) + 'a>;

/// min ns per value of each kernel, interleaved within each round
fn time(ks: &[Kernel<'_>], src: &[u32]) -> Vec<f64> {
    let mut best = vec![f64::INFINITY; ks.len()];
    let mut out = vec![0u32; src.len()];
    for _ in 0..15 {
        for (b, k) in best.iter_mut().zip(ks) {
            let t = Instant::now();
            for _ in 0..200 {
                k(black_box(src), black_box(&mut out));
            }
            *b = b.min(t.elapsed().as_nanos() as f64 / (200 * src.len()) as f64);
        }
    }
    best
}

fn main() {
    warm_up();
    let mut rng = Rng(42);
    let src: Vec<u32> = (0..SIZE).map(|_| rng.next() as u32).collect();
    let mut check: Vec<u32> = (0..1 << 20).map(|_| rng.next() as u32).collect();
    check.extend([0, 1, 2, u32::MAX, u32::MAX - 1, 1 << 31, (1 << 31) - 1]);

    println!("ns per value; split = round up or round down + 1, up|pr = round up or the PR's form");
    println!("{:<12} {:<15} {:>7} {:>7} {:>7} {:>7} {:>9} {:>9}   {:>7} {:>7} {:>7} {:>7} {:>9} {:>9}", "divisor", "split", "/", "pr", "split", "up|pr", "split/pr", "up|pr/pr", "%", "pr", "split", "up|pr", "split/pr", "up|pr/pr");
    for d in [7u32, 13, 10, 641, 1000, 1_000_000_007, 3, 16] {
        let (p, s) = (Pr::new(d), Split::new(d));
        // correctness against / and %, including multiples of d and their neighbors
        let mut probe = check.clone();
        for k in [1u32, 2, u32::MAX / d - 1, u32::MAX / d] {
            let m = k * d;
            probe.extend([
                m,
                m.wrapping_sub(1),
                m.wrapping_add(1),
                m.wrapping_add(d - 1),
            ]);
        }
        let mut a = vec![0u32; probe.len()];
        let mut b = vec![0u32; probe.len()];
        for (kernel, want) in [(
            pr_div as fn(Pr, &[u32], &mut [u32]),
            op_div as fn(u32, &[u32], &mut [u32]),
        )] {
            kernel(p, &probe, &mut a);
            want(d, &probe, &mut b);
            assert_eq!(a, b, "pr / {d}");
        }
        split_div(s, &probe, &mut a);
        op_div(d, &probe, &mut b);
        assert_eq!(a, b, "split / {d}");
        pr_rem(p, &probe, &mut a);
        op_rem(d, &probe, &mut b);
        assert_eq!(a, b, "pr % {d}");
        split_rem(s, &probe, &mut a);
        assert_eq!(a, b, "split % {d}");
        down2_rem(s, &probe, &mut a);
        op_rem(d, &probe, &mut b);
        assert_eq!(a, b, "down2 % {d}");
        down2_div(s, &probe, &mut a);
        op_div(d, &probe, &mut b);
        assert_eq!(a, b, "down2 / {d}");
        op_rem(d, &probe, &mut b);
        let u = UpOrPr::new(d);
        uop_rem(u, &probe, &mut a);
        assert_eq!(a, b, "up-or-pr % {d}");
        uop_div(u, &probe, &mut a);
        op_div(d, &probe, &mut b);
        assert_eq!(a, b, "up-or-pr / {d}");

        let (db, pb, sb, ub) = (black_box(d), black_box(p), black_box(s), black_box(u));
        if let Split::Down { .. } = s {
            let dv = time(&[Box::new(move |x, o| pr_div(pb, x, o)), Box::new(move |x, o| split_div(sb, x, o)), Box::new(move |x, o| down2_div(sb, x, o))], &src);
            let rm = time(&[Box::new(move |x, o| pr_rem(pb, x, o)), Box::new(move |x, o| split_rem(sb, x, o)), Box::new(move |x, o| down2_rem(sb, x, o))], &src);
            println!("  down {d}: div pr={:.3} sat={:.3} ({:.2}x) a*n+a={:.3} ({:.2}x) | rem pr={:.3} sat={:.3} ({:.2}x) a*n+a={:.3} ({:.2}x)", dv[0], dv[1], dv[0] / dv[1], dv[2], dv[0] / dv[2], rm[0], rm[1], rm[0] / rm[1], rm[2], rm[0] / rm[2]);
        }
        let div = time(
            &[
                Box::new(move |s, o| op_div(db, s, o)),
                Box::new(move |s, o| pr_div(pb, s, o)),
                Box::new(move |s, o| split_div(sb, s, o)),
                Box::new(move |s, o| uop_div(ub, s, o)),
            ],
            &src,
        );
        let rem = time(
            &[
                Box::new(move |s, o| op_rem(db, s, o)),
                Box::new(move |s, o| pr_rem(pb, s, o)),
                Box::new(move |s, o| split_rem(sb, s, o)),
                Box::new(move |s, o| uop_rem(ub, s, o)),
            ],
            &src,
        );
        println!(
            "{d:<12} {:<15} {:>7.3} {:>7.3} {:>7.3} {:>7.3} {:>8.2}x {:>8.2}x   {:>7.3} {:>7.3} {:>7.3} {:>7.3} {:>8.2}x {:>8.2}x",
            s.name(),
            div[0], div[1], div[2], div[3], div[1] / div[2], div[1] / div[3],
            rem[0], rem[1], rem[2], rem[3], rem[1] / rem[2], rem[1] / rem[3]
        );
    }
}
