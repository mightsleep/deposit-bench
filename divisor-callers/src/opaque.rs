// Licensed to the Apache Software Foundation (ASF) under one
// or more contributor license agreements.  See the NOTICE file
// distributed with this work for additional information
// regarding copyright ownership.  The ASF licenses this file
// to you under the Apache License, Version 2.0 (the
// "License"); you may not use this file except in compliance
// with the License.  You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing,
// software distributed under the License is distributed on an
// "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.  See the License for the
// specific language governing permissions and limitations
// under the License.

/// How a divisor divides, private so that representations can be added without a breaking change
#[derive(Debug, Clone, Copy)]
enum Repr<P, M> {
    /// A divisor whose magnitude is a power of two
    PowerOfTwo(P),
    /// Any other divisor
    Multiplier(M),
}

macro_rules! divisor_enum {
    ($name:ident, $power_of_two:ident, $multiplier:ident, $t:ty) => {
        #[doc = concat!("A `", stringify!($t), "` divisor, precomputed once to divide many values by it")]
        ///
        /// Division by a divisor that is only known at run time needs a divide instruction.
        /// [`Self::new`] precomputes a shift and a mask when the magnitude of the divisor is a
        /// power of two, and otherwise a multiplier with the method in
        /// [Granlund and Montgomery, "Division by Invariant Integers using Multiplication" (1994)](https://gmplib.org/~tege/divcnst-pldi94.pdf),
        /// so that each division is a high multiply, shifts and additions.
        ///
        /// To divide many values, call the methods for each value in the loop. They branch on
        /// how the divisor divides, and at `opt-level = 3` the compiler moves that branch out of
        /// the loop (loop unswitching). Move the divisor into the loop's closure, so that the
        /// compiler doesn't reload it through a reference for each value.
        ///
        /// # Example
        /// ```
        #[doc = concat!("# use arrow_buffer::", stringify!($name), ";")]
        #[doc = concat!("let values: Vec<", stringify!($t), "> = vec![100, 7, 64];")]
        #[doc = concat!("let divisor = ", stringify!($name), "::new(7).unwrap();")]
        /// let quotients: Vec<_> = values
        ///     .iter()
        ///     .map(move |n| divisor.wrapping_div_rem(*n).0)
        ///     .collect();
        /// assert_eq!(quotients, [14, 1, 9]);
        /// ```
        #[derive(Debug, Clone, Copy)]
        pub struct $name(Repr<$power_of_two, $multiplier>);

        impl $name {
            /// Returns a divisor for `divisor`, or `None` if `divisor` is zero
            ///
            /// # Example
            /// ```
            #[doc = concat!("# use arrow_buffer::", stringify!($name), ";")]
            #[doc = concat!("const SEVEN: ", stringify!($name), " = ", stringify!($name), "::new(7).unwrap();")]
            #[doc = concat!("assert!(", stringify!($name), "::new(0).is_none());")]
            /// ```
            pub const fn new(divisor: $t) -> Option<Self> {
                if divisor == 0 {
                    return None;
                }
                Some(Self(match $power_of_two::new(divisor) {
                    Some(d) => Repr::PowerOfTwo(d),
                    None => Repr::Multiplier($multiplier::new(divisor)),
                }))
            }

            /// Returns `(n.wrapping_div(divisor), n.wrapping_rem(divisor))`
            ///
            /// # Example
            /// ```
            #[doc = concat!("# use arrow_buffer::", stringify!($name), ";")]
            #[doc = concat!("let divisor = ", stringify!($name), "::new(7).unwrap();")]
            /// assert_eq!(divisor.wrapping_div_rem(100), (14, 2));
            /// ```
            #[inline]
            pub const fn wrapping_div_rem(self, n: $t) -> ($t, $t) {
                match self.0 {
                    Repr::PowerOfTwo(d) => d.wrapping_div_rem(n),
                    Repr::Multiplier(d) => d.wrapping_div_rem(n),
                }
            }
        }
    };
}

macro_rules! unsigned_divisor {
    ($name:ident, $power_of_two:ident, $multiplier:ident, $t:ty, $wide:ty) => {
        divisor_enum!($name, $power_of_two, $multiplier, $t);

        #[doc = concat!("A `", stringify!($t), "` divisor that is a power of two, see [`", stringify!($name), "`]")]
        #[derive(Debug, Clone, Copy)]
        struct $power_of_two {
            shift: u32,
            mask: $t,
        }

        impl $power_of_two {
            const fn new(divisor: $t) -> Option<Self> {
                if !divisor.is_power_of_two() {
                    return None;
                }
                Some(Self {
                    shift: divisor.trailing_zeros(),
                    mask: divisor - 1,
                })
            }

            /// Returns `(n / divisor, n % divisor)`
            #[inline]
            pub const fn wrapping_div_rem(self, n: $t) -> ($t, $t) {
                (n >> self.shift, n & self.mask)
            }
        }

        #[doc = concat!("A `", stringify!($t), "` divisor that isn't a power of two, see [`", stringify!($name), "`]")]
        ///
        /// It uses the multiplier from figure 4.1 of
        /// [Granlund and Montgomery, "Division by Invariant Integers using Multiplication" (1994)](https://gmplib.org/~tege/divcnst-pldi94.pdf).
        #[derive(Debug, Clone, Copy)]
        struct $multiplier {
            divisor: $t,
            /// `m'` in figure 4.1
            multiplier: $t,
            /// `sh2` in figure 4.1
            shift: u32,
        }

        impl $multiplier {
            /// `divisor` must be at least 3 and not a power of two
            const fn new(divisor: $t) -> Self {
                // ceil(log2(divisor)), which is at least 2 because divisor >= 3. So figure 4.1's
                // `sh1 = min(l, 1)` is always 1 and `sh2 = max(l - 1, 0)` is `l - 1`
                let l = <$t>::BITS - (divisor - 1).leading_zeros();
                // 2^l - divisor < divisor, so the multiplier fits in the narrow type
                let numerator = ((1 << l) - divisor as $wide) << <$t>::BITS;
                Self {
                    divisor,
                    multiplier: (numerator / divisor as $wide + 1) as $t,
                    shift: l - 1,
                }
            }

            /// Returns `(n / divisor, n % divisor)`
            #[inline]
            pub const fn wrapping_div_rem(self, n: $t) -> ($t, $t) {
                let t = ((self.multiplier as $wide * n as $wide) >> <$t>::BITS) as $t;
                // `(n + t) >> l` could overflow, so this halves `n - t` first, as figure 4.1 does
                let q = (t + ((n - t) >> 1)) >> self.shift;
                (q, n - q * self.divisor)
            }
        }
    };
}

macro_rules! signed_divisor {
    ($name:ident, $power_of_two:ident, $multiplier:ident, $t:ty, $wide:ty) => {
        divisor_enum!($name, $power_of_two, $multiplier, $t);

        impl $name {
            #[doc = concat!("Returns `(n / divisor, n % divisor)`, or `None` for `", stringify!($t), "::MIN / -1`")]
            ///
            /// # Example
            /// ```
            #[doc = concat!("# use arrow_buffer::", stringify!($name), ";")]
            #[doc = concat!("let divisor = ", stringify!($name), "::new(-7).unwrap();")]
            /// assert_eq!(divisor.checked_div_rem(-100), Some((14, -2)));
            ///
            #[doc = concat!("let minus_one = ", stringify!($name), "::new(-1).unwrap();")]
            #[doc = concat!("assert_eq!(minus_one.checked_div_rem(", stringify!($t), "::MIN), None);")]
            /// ```
            #[inline]
            pub const fn checked_div_rem(self, n: $t) -> Option<($t, $t)> {
                match self.0 {
                    Repr::PowerOfTwo(d) => d.checked_div_rem(n),
                    Repr::Multiplier(d) => Some(d.wrapping_div_rem(n)),
                }
            }
        }

        #[doc = concat!("A `", stringify!($t), "` divisor whose magnitude is a power of two, see [`", stringify!($name), "`]")]
        ///
        /// The quotient and remainder follow sections 10-1 and 10-2 of Warren, "Hacker's Delight",
        /// 2nd edition. Those sections compute the addend `2^k - 1` for a negative `n` with two
        /// shifts. This type computes it with one shift that spreads the sign bit of `n` and an AND
        /// with `2^k - 1`, which it computes once for the divisor.
        #[derive(Debug, Clone, Copy)]
        struct $power_of_two {
            shift: u32,
            /// `|divisor| - 1`
            mask: $t,
            /// -1 if the divisor is negative, otherwise 0
            sign: $t,
        }

        impl $power_of_two {
            const fn new(divisor: $t) -> Option<Self> {
                let magnitude = divisor.unsigned_abs();
                if !magnitude.is_power_of_two() {
                    return None;
                }
                Some(Self {
                    shift: magnitude.trailing_zeros(),
                    mask: (magnitude - 1) as $t,
                    sign: divisor >> (<$t>::BITS - 1),
                })
            }

            #[doc = concat!("Returns `(n.wrapping_div(divisor), n.wrapping_rem(divisor))`, so `", stringify!($t), "::MIN / -1` returns `(", stringify!($t), "::MIN, 0)`")]
            #[inline]
            pub const fn wrapping_div_rem(self, n: $t) -> ($t, $t) {
                // Adding `|divisor| - 1` to a negative `n` makes the shift round toward zero
                let t = n + ((n >> (<$t>::BITS - 1)) & self.mask);
                let q = t >> self.shift;
                // `(x ^ sign) - sign` negates `x` when `sign` is -1, and wraps only for `MIN / -1`
                // `t & !mask` is `n` rounded toward zero to a multiple of `|divisor|`
                ((q ^ self.sign).wrapping_sub(self.sign), n - (t & !self.mask))
            }

            #[doc = concat!("Returns `(n / divisor, n % divisor)`, or `None` for `", stringify!($t), "::MIN / -1`")]
            #[inline]
            pub const fn checked_div_rem(self, n: $t) -> Option<($t, $t)> {
                // -1 is the only divisor with a negative sign and a zero mask
                if n == <$t>::MIN && self.sign == -1 && self.mask == 0 {
                    None
                } else {
                    Some(self.wrapping_div_rem(n))
                }
            }
        }

        #[doc = concat!("A `", stringify!($t), "` divisor whose magnitude isn't a power of two, see [`", stringify!($name), "`]")]
        ///
        /// It uses the signed multiplier from figure 5.1 of
        /// [Granlund and Montgomery, "Division by Invariant Integers using Multiplication" (1994)](https://gmplib.org/~tege/divcnst-pldi94.pdf).
        #[derive(Debug, Clone, Copy)]
        struct $multiplier {
            divisor: $t,
            /// `m'` in figure 5.1
            multiplier: $t,
            /// `sh_post` in figure 5.1
            shift: u32,
        }

        impl $multiplier {
            /// `|divisor|` must be at least 3 and not a power of two
            const fn new(divisor: $t) -> Self {
                let magnitude = divisor.unsigned_abs();
                // ceil(log2(|divisor|)), which is at least 2 because |divisor| >= 3, so figure
                // 5.1's `max(..., 1)` isn't needed
                let l = <$t>::BITS - (magnitude - 1).leading_zeros();
                // m = 1 + floor(2^(N + l - 1) / |divisor|) is between 2^(N - 1) and 2^N, so
                // m - 2^N fits in the narrow type
                let m = 1 + (1 << (<$t>::BITS + l - 1)) / magnitude as $wide;
                Self {
                    divisor,
                    multiplier: (m - (1 << <$t>::BITS)) as $t,
                    shift: l - 1,
                }
            }

            /// Returns `(n / divisor, n % divisor)`, which can't overflow
            #[inline]
            pub const fn wrapping_div_rem(self, n: $t) -> ($t, $t) {
                // `MULSH(m', n)` in figure 5.1
                let high = ((self.multiplier as $wide * n as $wide) >> <$t>::BITS) as $t;
                // The shift rounds down, and subtracting the sign of `n` rounds toward zero instead
                let q = ((n + high) >> self.shift) - (n >> (<$t>::BITS - 1));
                let sign = self.divisor >> (<$t>::BITS - 1);
                let q = (q ^ sign) - sign;
                (q, n - q * self.divisor)
            }
        }
    };
}

unsigned_divisor!(
    DivisorU32,
    PowerOfTwoDivisorU32,
    MultiplierDivisorU32,
    u32,
    u64
);
unsigned_divisor!(
    DivisorU64,
    PowerOfTwoDivisorU64,
    MultiplierDivisorU64,
    u64,
    u128
);
signed_divisor!(
    DivisorI32,
    PowerOfTwoDivisorI32,
    MultiplierDivisorI32,
    i32,
    i64
);
signed_divisor!(
    DivisorI64,
    PowerOfTwoDivisorI64,
    MultiplierDivisorI64,
    i64,
    i128
);
