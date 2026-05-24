//! `MinHash` sketch implementation backed by `XxHash64`.
//!
//! We simulate K independent hash functions by using `XxHash64::with_seed(k)`
//! for k in `0..NUM_PERM`. Each hash function maps a shingle (byte slice) to a
//! u64; we keep the minimum across all shingles for each hash function.
//!
//! The Jaccard estimate of two `MinHash` values is:
//!   |{k : a.mins[k] == b.mins[k]}| / `NUM_PERM`

use std::hash::Hasher as _;

use twox_hash::XxHash64;

/// Number of hash permutations — mirrors `_NUM_PERM = 128` in Python.
pub const NUM_PERM: usize = 128;

/// A `MinHash` sketch consisting of `NUM_PERM` minimum values.
#[derive(Debug, Clone)]
pub struct MinHash {
    mins: [u64; NUM_PERM],
}

impl MinHash {
    /// Create a new, empty (all-max) sketch.
    #[must_use]
    pub fn new() -> Self {
        Self {
            mins: [u64::MAX; NUM_PERM],
        }
    }

    /// Hash one shingle (any byte slice) and update the mins array.
    pub fn update(&mut self, data: &[u8]) {
        for (k, min) in self.mins.iter_mut().enumerate() {
            let mut h = XxHash64::with_seed(k as u64);
            h.write(data);
            let v = h.finish();
            if v < *min {
                *min = v;
            }
        }
    }

    /// Estimate Jaccard similarity with another sketch.
    #[must_use]
    pub fn jaccard(&self, other: &MinHash) -> f64 {
        let equal = self
            .mins
            .iter()
            .zip(other.mins.iter())
            .filter(|(a, b)| a == b)
            .count();
        // NUM_PERM = 128 always fits in f64 without precision loss.
        #[allow(clippy::cast_precision_loss)] // NUM_PERM ≤ 128; fits in f64 exactly.
        {
            equal as f64 / NUM_PERM as f64
        }
    }
}

impl Default for MinHash {
    fn default() -> Self {
        Self::new()
    }
}
