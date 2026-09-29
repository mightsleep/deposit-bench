#![feature(uint_gather_scatter_bits)]
#![crate_type = "staticlib"]
#[unsafe(no_mangle)]
pub fn ext64(v: u64, m: u64) -> u64 { v.extract_bits(m) }
#[unsafe(no_mangle)]
pub fn dep64(v: u64, m: u64) -> u64 { v.deposit_bits(m) }
