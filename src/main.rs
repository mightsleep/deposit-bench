//! intersect_bit_buffers end to end (self words + rank bits -> output words)
//! for the portable deposit, one pass against two:
//! - vortex: today's loop, stateful RankBitReader, portable deposit with its
//!   two shortcuts
//! - one pass: absolute reader, the byte network with popcount tiers
//! - two pass: first the source bits of every word (popcount, running offset,
//!   absolute read), then the network over independent words in blocks of 8,
//!   a block of trivial words copied; no tiers, so the compiler may vectorize
//! - one pass + blocks: one pass in blocks of 8 words, a block of trivial
//!   words (source 0 or mask all ones) copied
//! - one pass + blocks, neon: the same with the network in NEON byte lanes
//!   (aarch64 only)
//! - two pass, shl-add (the function remains, no longer timed): the same with the byte prefix sums as three
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

/// One pass in blocks of 8: sources and popcounts of the block first, a block
/// of trivial words (source 0 or mask all ones, result = source) copied,
/// otherwise the network with popcount tiers per word. `DEP` is the deposit
/// used for a non-trivial word
#[inline(always)]
fn one_pass_blocks<F: Fn(u64, u64, u32) -> u64>(
    selfw: &[u64],
    rank: &[u64],
    out: &mut Vec<u64>,
    dep: F,
) {
    out.clear();
    out.resize(selfw.len(), 0);
    let mut off = 0;
    let n8 = selfw.len() / 8 * 8;
    for (o, m) in out[..n8]
        .chunks_exact_mut(8)
        .zip(selfw[..n8].chunks_exact(8))
    {
        let mut s = [0u64; 8];
        let mut k = [0u32; 8];
        let mut triv = true;
        for j in 0..8 {
            k[j] = m[j].count_ones();
            s[j] = read_abs(rank, off, k[j] as usize);
            off += k[j] as usize;
            triv &= (s[j] == 0) | (m[j] == u64::MAX);
        }
        if triv {
            o.copy_from_slice(&s);
        } else {
            for j in 0..8 {
                o[j] = dep(s[j], m[j], k[j]);
            }
        }
    }
    for j in n8..selfw.len() {
        let k = selfw[j].count_ones();
        out[j] = dep(read_abs(rank, off, k as usize), selfw[j], k);
        off += k as usize;
    }
}

fn one_pass_blk(selfw: &[u64], rank: &[u64], out: &mut Vec<u64>) {
    one_pass_blocks(selfw, rank, out, deposit_tiers);
}

/// The byte network in real byte lanes: NEON u8x8 shifts cannot cross into
/// the next byte, so the masking constants of the SWAR form go away, and the
/// byte popcounts are one `cnt`
#[cfg(target_arch = "aarch64")]
#[inline(always)]
fn expand_bytes_neon(source: u64, mask: u64) -> u64 {
    use core::arch::aarch64::*;
    unsafe {
        let m0 = vcreate_u8(mask);
        let mut m = m0;
        let mut zeros = vmvn_u8(m0);
        // round 1: parity = prefix xor of zeros, stride 1
        let mut p = zeros;
        p = veor_u8(p, vshl_n_u8::<1>(p));
        p = veor_u8(p, vshl_n_u8::<2>(p));
        p = veor_u8(p, vshl_n_u8::<4>(p));
        let q = vand_u8(m, p);
        let mv0 = vshr_n_u8::<1>(q);
        m = veor_u8(veor_u8(m, q), mv0);
        zeros = vbic_u8(zeros, p);
        zeros = veor_u8(zeros, vshr_n_u8::<1>(zeros));
        // round 2: stride 2
        let mut p = zeros;
        p = veor_u8(p, vshl_n_u8::<2>(p));
        p = veor_u8(p, vshl_n_u8::<4>(p));
        let q = vand_u8(m, p);
        let mv1 = vshr_n_u8::<2>(q);
        m = veor_u8(veor_u8(m, q), mv1);
        zeros = vbic_u8(zeros, p);
        zeros = veor_u8(zeros, vshr_n_u8::<2>(zeros));
        // round 3: stride 4
        let p = veor_u8(zeros, vshl_n_u8::<4>(zeros));
        let q = vand_u8(m, p);
        let mv2 = vshr_n_u8::<4>(q);
        m = veor_u8(veor_u8(m, q), mv2);
        // byte offsets: counts by `cnt`, prefix by one multiply
        let kept = vget_lane_u64::<0>(vreinterpret_u64_u8(vcnt_u8(m0)));
        let below = kept.wrapping_mul(bytes(1)) << 8;
        let mut x = 0;
        for j in 0..8 {
            x |= ((source >> ((below >> (8 * j)) & 0xFF)) & 0xFF) << (8 * j);
        }
        let mut xv = vand_u8(vcreate_u8(x), m);
        xv = vorr_u8(vbic_u8(xv, mv2), vshl_n_u8::<4>(vand_u8(xv, mv2)));
        xv = vorr_u8(vbic_u8(xv, mv1), vshl_n_u8::<2>(vand_u8(xv, mv1)));
        xv = vorr_u8(vbic_u8(xv, mv0), vshl_n_u8::<1>(vand_u8(xv, mv0)));
        vget_lane_u64::<0>(vreinterpret_u64_u8(vand_u8(xv, m0)))
    }
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
fn deposit_tiers_neon(source: u64, mask: u64, k: u32) -> u64 {
    if k <= 2 || k >= 62 {
        deposit_tiers(source, mask, k)
    } else {
        expand_bytes_neon(source, mask)
    }
}

#[cfg(target_arch = "aarch64")]
fn one_pass_blk_neon(selfw: &[u64], rank: &[u64], out: &mut Vec<u64>) {
    one_pass_blocks(selfw, rank, out, deposit_tiers_neon);
}

/// One pass, the shortcut decided from the masks alone: a block of 8 whose
/// masks are all 0 or all ones takes `s & m`; any other block is exactly the
/// plain one pass, nothing staged
#[inline(always)]
fn one_pass_masks<F: Fn(u64, u64, u32) -> u64>(
    selfw: &[u64],
    rank: &[u64],
    out: &mut Vec<u64>,
    dep: F,
) {
    out.clear();
    out.resize(selfw.len(), 0);
    let mut off = 0;
    let n8 = selfw.len() / 8 * 8;
    for (o, m) in out[..n8]
        .chunks_exact_mut(8)
        .zip(selfw[..n8].chunks_exact(8))
    {
        let mut triv = true;
        for j in 0..8 {
            triv &= (m[j] == 0) | (m[j] == u64::MAX);
        }
        if triv {
            for j in 0..8 {
                let k = m[j].count_ones() as usize;
                o[j] = read_abs(rank, off, k) & m[j];
                off += k;
            }
        } else {
            for j in 0..8 {
                let k = m[j].count_ones();
                o[j] = dep(read_abs(rank, off, k as usize), m[j], k);
                off += k as usize;
            }
        }
    }
    for j in n8..selfw.len() {
        let k = selfw[j].count_ones();
        out[j] = dep(read_abs(rank, off, k as usize), selfw[j], k);
        off += k as usize;
    }
}

fn one_pass_mblk(selfw: &[u64], rank: &[u64], out: &mut Vec<u64>) {
    one_pass_masks(selfw, rank, out, deposit_tiers);
}

#[cfg(target_arch = "aarch64")]
fn one_pass_mblk_neon(selfw: &[u64], rank: &[u64], out: &mut Vec<u64>) {
    one_pass_masks(selfw, rank, out, deposit_tiers_neon);
}

/// One pass word by word; at every 8th word the next 8 masks are checked and a
/// run of 8 masks that are all 0 or all ones takes `s & m`. No fixed-length
/// block for LLVM to unroll and SLP-vectorize, which cost 12 to 17 % on random
/// masks in the blocked form
fn one_pass_look(selfw: &[u64], rank: &[u64], out: &mut Vec<u64>) {
    out.clear();
    out.reserve(selfw.len());
    let mut off = 0;
    let mut i = 0;
    let n = selfw.len();
    while i < n {
        if i % 8 == 0 && i + 8 <= n {
            let b = &selfw[i..i + 8];
            if b.iter().all(|&m| (m == 0) | (m == u64::MAX)) {
                for &m in b {
                    let k = m.count_ones() as usize;
                    out.push(read_abs(rank, off, k) & m);
                    off += k;
                }
                i += 8;
                continue;
            }
        }
        let m = selfw[i];
        let k = m.count_ones();
        out.push(deposit_tiers(read_abs(rank, off, k as usize), m, k));
        off += k as usize;
        i += 1;
    }
}

/// SVE2 BITPERM `bdep`, second pass: a predicated loop over the source words
/// of pass one, VL / 64 words per `bdep`, any vector length
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "sve2,sve2-bitperm")]
unsafe fn bdep_slices(src: &[u64], mask: &[u64], out: &mut [u64]) {
    let n = src.len();
    unsafe {
        core::arch::asm!(
            "mov {i}, #0",
            "whilelo p0.d, {i}, {n}",
            "b.none 2f",
            "1:",
            "ld1d {{ z0.d }}, p0/z, [{s}, {i}, lsl #3]",
            "ld1d {{ z1.d }}, p0/z, [{m}, {i}, lsl #3]",
            "bdep z0.d, z0.d, z1.d",
            "st1d {{ z0.d }}, p0, [{o}, {i}, lsl #3]",
            "incd {i}",
            "whilelo p0.d, {i}, {n}",
            "b.first 1b",
            "2:",
            i = out(reg) _,
            n = in(reg) n,
            s = in(reg) src.as_ptr(),
            m = in(reg) mask.as_ptr(),
            o = in(reg) out.as_mut_ptr(),
            out("v0") _,
            out("v1") _,
            out("p0") _,
            options(nostack),
        );
    }
}

#[cfg(target_arch = "aarch64")]
fn two_pass_bdep(selfw: &[u64], rank: &[u64], src: &mut Vec<u64>, out: &mut Vec<u64>) {
    src.clear();
    let mut off = 0;
    for &m in selfw {
        let k = m.count_ones() as usize;
        src.push(read_abs(rank, off, k));
        off += k;
    }
    out.clear();
    out.resize(selfw.len(), 0);
    unsafe { bdep_slices(src, selfw, out) };
}

/// One word through `bdep`: in and out of the vector file per word
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "sve2,sve2-bitperm")]
unsafe fn bdep1(s: u64, m: u64) -> u64 {
    let r: u64;
    unsafe {
        core::arch::asm!(
            "fmov d0, {s}",
            "fmov d1, {m}",
            "bdep z0.d, z0.d, z1.d",
            "fmov {r}, d0",
            s = in(reg) s,
            m = in(reg) m,
            r = lateout(reg) r,
            out("v0") _,
            out("v1") _,
            options(pure, nomem, nostack),
        );
    }
    r
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "sve2,sve2-bitperm")]
unsafe fn one_pass_bdep_inner(selfw: &[u64], rank: &[u64], out: &mut Vec<u64>) {
    out.clear();
    let mut off = 0;
    for &m in selfw {
        let k = m.count_ones() as usize;
        out.push(unsafe { bdep1(read_abs(rank, off, k), m) });
        off += k;
    }
}

#[cfg(target_arch = "aarch64")]
fn one_pass_bdep(selfw: &[u64], rank: &[u64], out: &mut Vec<u64>) {
    unsafe { one_pass_bdep_inner(selfw, rank, out) }
}

fn two_pass<const SHLADD: bool>(
    selfw: &[u64],
    rank: &[u64],
    src: &mut Vec<u64>,
    out: &mut Vec<u64>,
) {
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

/// Whole-word expand, Hacker's Delight 7-5: the compress move masks of six
/// rounds, applied backwards with left shifts. Each round needs the prefix
/// xor of `mk`, which is a carry-less multiply by all ones; `prefix` does it
#[inline(always)]
fn expand_hd<P: Fn(u64) -> u64>(x: u64, mask: u64, prefix: P) -> u64 {
    let mut m = mask;
    let mut mk = !mask << 1;
    let mut mv = [0u64; 6];
    for (i, v) in mv.iter_mut().enumerate() {
        let mp = prefix(mk);
        *v = mp & m;
        m = (m ^ *v) | (*v >> (1 << i));
        mk &= !mp;
    }
    let mut x = x;
    for i in (0..6).rev() {
        let v = mv[i];
        x = (x & !v) | ((x << (1 << i)) & v);
    }
    x & mask
}

#[inline(always)]
fn prefix_xor_soft(x: u64) -> u64 {
    let mut p = x ^ (x << 1);
    p ^= p << 2;
    p ^= p << 4;
    p ^= p << 8;
    p ^= p << 16;
    p ^ (p << 32)
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
unsafe fn prefix_xor_clmul(x: u64) -> u64 {
    // low half of x * (all ones) over GF(2)
    unsafe { core::arch::aarch64::vmull_p64(x, u64::MAX) as u64 }
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
unsafe fn prefix_xor_clmul(x: u64) -> u64 {
    use core::arch::x86_64::*;
    unsafe {
        let p = _mm_clmulepi64_si128(_mm_cvtsi64_si128(x as i64), _mm_set1_epi64x(-1), 0);
        _mm_cvtsi128_si64(p) as u64
    }
}

#[inline(always)]
fn tiers_with<E: Fn(u64, u64) -> u64>(source: u64, mask: u64, k: u32, e: E) -> u64 {
    if k <= 2 || k >= 62 {
        deposit_tiers(source, mask, k)
    } else {
        e(source, mask)
    }
}

fn one_pass_hd_soft(selfw: &[u64], rank: &[u64], out: &mut Vec<u64>) {
    out.clear();
    let mut off = 0;
    for &m in selfw {
        let k = m.count_ones();
        let s = read_abs(rank, off, k as usize);
        out.push(tiers_with(s, m, k, |s, m| expand_hd(s, m, prefix_xor_soft)));
        off += k as usize;
    }
}

#[cfg_attr(target_arch = "aarch64", target_feature(enable = "aes"))]
#[cfg_attr(target_arch = "x86_64", target_feature(enable = "pclmulqdq,popcnt"))]
unsafe fn one_pass_hd_clmul_inner(selfw: &[u64], rank: &[u64], out: &mut Vec<u64>) {
    out.clear();
    let mut off = 0;
    for &m in selfw {
        let k = m.count_ones();
        let s = read_abs(rank, off, k as usize);
        out.push(tiers_with(s, m, k, |s, m| {
            expand_hd(s, m, |x| unsafe { prefix_xor_clmul(x) })
        }));
        off += k as usize;
    }
}

fn one_pass_hd_clmul(selfw: &[u64], rank: &[u64], out: &mut Vec<u64>) {
    unsafe { one_pass_hd_clmul_inner(selfw, rank, out) }
}

fn has_clmul() -> bool {
    #[cfg(target_arch = "aarch64")]
    return std::arch::is_aarch64_feature_detected!("pmull");
    #[cfg(target_arch = "x86_64")]
    return is_x86_feature_detected!("pclmulqdq") && is_x86_feature_detected!("popcnt");
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
        let mut a = vec![];
        vortex(selfw, rank, &mut a);
        let ta = time(N, || vortex(black_box(selfw), black_box(rank), &mut a));
        let mut row = format!("  {name:<26} vortex {ta:6.2}");
        let mut run = |label: &str, f: &dyn Fn(&[u64], &[u64], &mut Vec<u64>)| {
            let mut o = vec![];
            f(selfw, rank, &mut o);
            assert_eq!(a, o, "{label} {name}");
            let t = time(N, || f(black_box(selfw), black_box(rank), &mut o));
            row += &format!(" | {label} {t:5.2} (x{:5.2})", ta / t);
        };
        run("one pass", &one_pass);
        run("+blocks", &one_pass_blk);
        run("+mask blocks", &one_pass_mblk);
        run("+lookahead", &one_pass_look);
        run("hd soft", &one_pass_hd_soft);
        if has_clmul() {
            run("hd clmul", &one_pass_hd_clmul);
        }
        #[cfg(target_arch = "aarch64")]
        run("+mask blocks neon", &one_pass_mblk_neon);
        let src_buf = std::cell::RefCell::new(Vec::new());
        run("two pass", &|s, r, o| {
            two_pass::<false>(s, r, &mut src_buf.borrow_mut(), o)
        });
        #[cfg(target_arch = "aarch64")]
        if std::arch::is_aarch64_feature_detected!("sve2-bitperm") {
            run("bdep 1 pass", &one_pass_bdep);
            run("bdep 2 pass", &|s, r, o| {
                two_pass_bdep(s, r, &mut src_buf.borrow_mut(), o)
            });
        }
        #[cfg(target_feature = "bmi2")]
        run("pdep", &pdep);
        println!("{row}");
    }
}
