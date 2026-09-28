//! intersect_bit_buffers end to end (self words + rank bits -> output words)
//! for the portable deposit, one pass against two:
//! - vortex: today's loop, stateful RankBitReader, portable deposit with its
//!   two shortcuts
//! - one pass: absolute reader, the byte network with popcount tiers
//! - two pass: first the source bits of every word (popcount, running offset,
//!   absolute read), then the network over independent words in blocks of 8,
//!   a block of trivial words copied; no tiers, so the compiler may vectorize
//! - two pass, shl-add: the same with the byte prefix sums as three
//!   shift-adds instead of a multiply, which NEON lacks for 64-bit lanes
//! - pdep: one pass, absolute reader, BMI2 (x86 builds with bmi2 only)
//!
//! Build the same source three ways to compare: default, with LLVM's loop
//! and SLP vectorizers off (the scalar baseline for identical code), and
//! with `-C target-cpu=native`. LABEL names the build in the output.
use std::hint::black_box;
use std::time::Instant;

const fn bytes(b: u8) -> u64 {
    b as u64 * 0x0101_0101_0101_0101
}

#[inline(always)]
fn expand_bytes<const SHLADD: bool>(source: u64, mask: u64, s8: u32) -> u64 {
    let c = mask - ((mask >> 1) & bytes(0x55));
    let c = (c & bytes(0x33)) + ((c >> 2) & bytes(0x33));
    let kept = (c + (c >> 4)) & bytes(0x0F);
    // byte prefix sums: one multiply, or three shift-adds for targets whose
    // vector units have no 64-bit multiply (NEON)
    // s8 is 8, passed in at run time: with literal shifts LLVM folds the
    // shift-adds straight back into the multiply
    let below = if SHLADD {
        let p = kept + (kept << s8);
        let p = p + (p << (2 * s8));
        (p + (p << (4 * s8))) << s8
    } else {
        kept.wrapping_mul(bytes(1)) << 8
    };
    let mut m = mask;
    let mut zeros = !mask;
    let mut mv = [0u64; 3];
    let mut n = 1;
    let mut i = 0;
    while n < 8 {
        let mut parity = zeros;
        let mut len = n;
        while len < 8 {
            parity ^= (parity << len) & bytes(0xFF << len);
            len <<= 1;
        }
        let q = m & parity;
        m ^= q ^ ((q >> n) & bytes(0xFF >> n));
        mv[i] = (q >> n) & bytes(0xFF >> n);
        zeros &= !parity;
        zeros ^= (zeros >> n) & bytes(0xFF >> n);
        n <<= 1;
        i += 1;
    }
    let mut x = 0;
    for j in 0..8 {
        x |= ((source >> ((below >> (8 * j)) & 0xFF)) & 0xFF) << (8 * j);
    }
    x &= m;
    let mut n = 4;
    let mut i = 3;
    while n > 0 {
        i -= 1;
        let v = mv[i];
        x = (x & !v) | ((x & v) << n);
        n >>= 1;
    }
    x & mask
}

#[inline(always)]
fn deposit_tiers(source: u64, mask: u64, k: u32) -> u64 {
    if k <= 2 {
        let lowest = mask & mask.wrapping_neg();
        let second = (mask ^ lowest) & (mask ^ lowest).wrapping_neg();
        (lowest & (source & 1).wrapping_neg()) | (second & (source >> 1 & 1).wrapping_neg())
    } else if k >= 62 {
        let mut x = source;
        let mut holes = !mask;
        for _ in 0..2 {
            let h = holes & holes.wrapping_neg();
            let below = h.wrapping_sub(1);
            x = (x & below) | ((x << 1) & !below);
            holes &= !h;
        }
        x & mask
    } else {
        expand_bytes::<false>(source, mask, 8)
    }
}

#[inline(always)]
fn low(n: usize) -> u64 {
    if n == 64 {
        u64::MAX
    } else {
        (1 << n) - 1
    }
}

/// Bits `off..off + n` of `rank`, which carries a padding word past its end
#[inline(always)]
fn read_abs(rank: &[u64], off: usize, n: usize) -> u64 {
    let (w, b) = (off / 64, off % 64);
    let lo = rank[w];
    let hi = rank[w + 1];
    ((lo >> b) | ((hi << 1) << (63 - b))) & low(n)
}

// --- vortex today ---------------------------------------------------------
struct Reader<'a> {
    it: std::slice::Iter<'a, u64>,
    current: u64,
    next: u64,
    off: usize,
}
impl<'a> Reader<'a> {
    fn new(w: &'a [u64]) -> Self {
        let mut it = w.iter();
        let current = *it.next().unwrap_or(&0);
        let next = *it.next().unwrap_or(&0);
        Self {
            it,
            current,
            next,
            off: 0,
        }
    }
    #[inline(always)]
    fn read(&mut self, n: usize) -> u64 {
        let combined = ((self.next as u128) << 64) | self.current as u128;
        let bits = (combined >> self.off) as u64 & low(n);
        let o = self.off + n;
        if o >= 64 {
            self.current = self.next;
            self.next = *self.it.next().unwrap_or(&0);
            self.off = o - 64;
        } else {
            self.off = o;
        }
        bits
    }
}
fn select_bit_position_portable(word: u64, mut rank: usize) -> usize {
    let mut bit_offset = 0usize;
    for byte in word.to_le_bytes() {
        let count = byte.count_ones() as usize;
        if rank < count {
            let mut bits = byte;
            for _ in 0..rank {
                bits &= bits - 1;
            }
            return bit_offset + bits.trailing_zeros() as usize;
        }
        rank -= count;
        bit_offset += 8;
    }
    0
}
#[inline(always)]
fn deposit_vortex(source: u64, mask: u64, mask_count: usize) -> u64 {
    if source == 0 {
        return 0;
    }
    if mask == u64::MAX {
        return source;
    }
    if mask_count >= 16 && source.count_ones() as usize * 8 < mask_count {
        let mut s = source;
        let mut result = 0u64;
        while s != 0 {
            result |= 1u64 << select_bit_position_portable(mask, s.trailing_zeros() as usize);
            s &= s - 1;
        }
        return result;
    }
    let (mut s, mut m, mut result) = (source, mask, 0u64);
    while m != 0 {
        let bit = m & m.wrapping_neg();
        if s & 1 != 0 {
            result |= bit;
        }
        s >>= 1;
        m &= m - 1;
    }
    result
}
fn vortex(selfw: &[u64], rank: &[u64], out: &mut Vec<u64>) {
    out.clear();
    let mut r = Reader::new(rank);
    for &m in selfw {
        let k = m.count_ones() as usize;
        let s = r.read(k);
        out.push(deposit_vortex(s, m, k));
    }
}

// --- new ------------------------------------------------------------------
fn one_pass(selfw: &[u64], rank: &[u64], out: &mut Vec<u64>) {
    out.clear();
    let mut off = 0;
    for &m in selfw {
        let k = m.count_ones();
        out.push(deposit_tiers(read_abs(rank, off, k as usize), m, k));
        off += k as usize;
    }
}

fn two_pass<const SHLADD: bool>(selfw: &[u64], rank: &[u64], src: &mut Vec<u64>, out: &mut Vec<u64>) {
    let s8 = black_box(8u32);
    src.clear();
    let mut off = 0;
    for &m in selfw {
        let k = m.count_ones() as usize;
        src.push(read_abs(rank, off, k));
        off += k;
    }
    out.clear();
    out.resize(selfw.len(), 0);
    let n8 = selfw.len() / 8 * 8;
    for ((o, s), m) in out[..n8]
        .chunks_exact_mut(8)
        .zip(src[..n8].chunks_exact(8))
        .zip(selfw[..n8].chunks_exact(8))
    {
        let o: &mut [u64; 8] = o.try_into().unwrap();
        let s: &[u64; 8] = s.try_into().unwrap();
        let m: &[u64; 8] = m.try_into().unwrap();
        let mut triv = true;
        for j in 0..8 {
            triv &= (s[j] == 0) | (m[j] == u64::MAX);
        }
        if triv {
            *o = *s;
        } else {
            *o = block8::<SHLADD>(s, m, s8);
        }
    }
    for j in n8..selfw.len() {
        out[j] = expand_bytes::<SHLADD>(src[j], selfw[j], s8);
    }
}

#[inline(always)]
fn block8<const SHLADD: bool>(s: &[u64; 8], m: &[u64; 8], s8: u32) -> [u64; 8] {
    let mut o = [0u64; 8];
    for j in 0..8 {
        o[j] = expand_bytes::<SHLADD>(s[j], m[j], s8);
    }
    o
}

#[cfg(target_feature = "bmi2")]
fn pdep(selfw: &[u64], rank: &[u64], out: &mut Vec<u64>) {
    out.clear();
    let mut off = 0;
    for &m in selfw {
        let k = m.count_ones() as usize;
        out.push(unsafe { std::arch::x86_64::_pdep_u64(read_abs(rank, off, k), m) });
        off += k;
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn bits(&mut self, p: f64) -> u64 {
        let t = (p * 4294967296.0) as u64;
        (0..64).fold(0, |w, i| w | u64::from((self.next() >> 32) < t) << i)
    }
    fn runs(&mut self, n: usize, run: f64) -> Vec<u64> {
        let flip = (1.0f64 / run * 4294967296.0) as u64;
        let mut set = false;
        (0..n)
            .map(|_| {
                (0..64).fold(0u64, |w, i| {
                    if (self.next() >> 32) < flip {
                        set = !set;
                    }
                    w | u64::from(set) << i
                })
            })
            .collect()
    }
}

fn time(n: usize, mut f: impl FnMut()) -> f64 {
    f();
    let reps = (1 << 24) / n + 1;
    let mut best = f64::MAX;
    for _ in 0..9 {
        let t = Instant::now();
        for _ in 0..reps {
            f();
        }
        best = best.min(t.elapsed().as_secs_f64() / reps as f64);
    }
    best * 1e9 / n as f64
}

fn main() {
    const N: usize = 1 << 16;
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    println!(
        "ns / self word, {N} words, {}",
        std::env::var("LABEL").unwrap_or_default()
    );
    let mut cases: Vec<(String, Vec<u64>, Vec<u64>)> = vec![];
    for (ds, dr) in [(0.5, 0.5), (0.5, 0.1), (0.9, 0.9), (0.1, 0.5), (0.02, 0.5)] {
        let selfw: Vec<u64> = (0..N).map(|_| rng.bits(ds)).collect();
        let total: usize = selfw.iter().map(|w| w.count_ones() as usize).sum();
        let rank: Vec<u64> = (0..total.div_ceil(64)).map(|_| rng.bits(dr)).collect();
        cases.push((format!("random self {ds}, rank {dr}"), selfw, rank));
    }
    for run in [64.0, 4096.0] {
        let selfw = rng.runs(N, run);
        let total: usize = selfw.iter().map(|w| w.count_ones() as usize).sum();
        let rank = rng.runs(total.div_ceil(64), run);
        cases.push((format!("runs of {run}, both"), selfw, rank));
    }
    for (name, selfw, rank) in &mut cases {
        let total: usize = selfw.iter().map(|w| w.count_ones() as usize).sum();
        if total % 64 != 0 {
            let l = rank.len();
            rank[l - 1] &= low(total % 64);
        }
        rank.extend([0, 0]);
        let (mut a, mut b, mut c, mut e, mut src) = (vec![], vec![], vec![], vec![], vec![]);
        vortex(selfw, rank, &mut a);
        one_pass(selfw, rank, &mut b);
        two_pass::<false>(selfw, rank, &mut src, &mut c);
        two_pass::<true>(selfw, rank, &mut src, &mut e);
        assert_eq!(a, b, "{name}");
        assert_eq!(a, c, "{name}");
        assert_eq!(a, e, "{name}");
        let ta = time(N, || vortex(black_box(selfw), black_box(rank), &mut a));
        let tb = time(N, || one_pass(black_box(selfw), black_box(rank), &mut b));
        let tc = time(N, || {
            two_pass::<false>(black_box(selfw), black_box(rank), &mut src, &mut c)
        });
        let te = time(N, || {
            two_pass::<true>(black_box(selfw), black_box(rank), &mut src, &mut e)
        });
        #[cfg(target_feature = "bmi2")]
        let td = {
            let mut d = vec![];
            pdep(selfw, rank, &mut d);
            assert_eq!(a, d);
            time(N, || pdep(black_box(selfw), black_box(rank), &mut d))
        };
        #[cfg(not(target_feature = "bmi2"))]
        let td = f64::NAN;
        println!(
            "  {name:<26} vortex {ta:6.2} | one pass {tb:5.2} (x{:5.2}) | two pass mul {tc:5.2} (x{:5.2}) | two pass shl-add {te:5.2} (x{:5.2}) | pdep {td:5.2}",
            ta / tb,
            ta / tc,
            ta / te
        );
    }
}
