//! Utility functions for Merkle tree construction and manipulation.
//!
//! This module provides helper functions for calculating tree dimensions,
//! partitioning data, and locating nodes within the prefixed storage structure.

extern crate alloc;
#[cfg(test)]
use alloc::{vec, vec::Vec};

/// Calculates the number of nodes at a specific layer in the tree.
///
/// # Arguments
///
/// * `arity` - The branching factor of the tree
/// * `height` - The total height of the tree
/// * `layer_index` - The layer to calculate the size for (0 = base layer)
#[inline]
pub fn layer_size(height: usize, layer_index: usize) -> usize {
    // Use the crate ARITY constant; the function no longer accepts an arity param.
    let arity = crate::ARITY;
    if height == 0 {
        1
    } else {
        1 << (arity.trailing_zeros() as usize * (height - layer_index - 1))
    }
}

/// Calculates the total number of prefixed blocks needed to store a tree with `n` leaves.
#[inline]
pub fn num_of_prefixed_for_input(n: usize) -> usize {
    let arity = crate::ARITY;
    let h = height(n);
    (arity * layer_size(h, 0) - 1) / (arity - 1)
}

/// Converts a leaf index to its location in the prefixed storage.
///
/// Returns `(prefixed_index, offset)` where `prefixed_index` is the index of the
/// prefixed block and `offset` is the position within that block.
#[inline]
pub(crate) fn location_in_prefixed(input_index: usize) -> (usize, usize) {
    let arity = crate::ARITY;
    let offset = input_index & (arity - 1);
    let index = input_index >> arity.trailing_zeros();
    (index, offset)
}

/// Checks if a number is a power of 2.
#[inline]
fn is_pow2(n: usize) -> bool {
    n.is_power_of_two()
}

/// Returns the smallest power of 2 that is greater than or equal to `n`.
#[inline]
fn ceiling_pow2(n: usize) -> usize {
    // For zero, keep behaviour of returning 1 (smallest meaningful power).
    if n == 0 {
        1
    } else {
        // Use the standard library helper which returns the next power of two
        // (or None on overflow). On overflow fall back to the highest power of two
        // representable for usize.
        n.checked_next_power_of_two()
            .unwrap_or(1 << (usize::BITS as usize - 1))
    }
}

/// Returns the largest power of 2 that is less than or equal to `n`.
#[inline]
fn floor_pow2(n: usize) -> usize {
    if n == 0 {
        1
    } else {
        // Compute next power of two (or fallback on overflow), then adjust if needed.
        let p = n
            .checked_next_power_of_two()
            .unwrap_or(1 << (usize::BITS as usize - 1));
        if p == n {
            n
        } else {
            p >> 1
        }
    }
}

/// Checks if `n` is a power of the given arity.
///
/// This keeps the previous logic but uses `is_pow2`. Defensive check added
/// to avoid a modulo by zero if an invalid arity is ever passed.
#[inline]
fn is_arity_pow(n: usize, arity: usize) -> bool {
    if arity == 0 {
        return false;
    }
    if !is_pow2(n) {
        return false;
    }
    let arity_tz = arity.trailing_zeros();
    if arity_tz == 0 {
        // arity is not a power of two: cannot represent clean alignment in this model
        return false;
    }
    n.trailing_zeros() % arity_tz == 0
}

/// Returns the smallest power of `arity` that is greater than or equal to `n`.
///
/// Since this crate fixes `crate::ARITY == 2` in practice, use the optimized
/// power-of-two path where possible. Otherwise fall back to the generic logic.
#[inline]
pub(crate) fn ceiling_arity_pow(n: usize) -> usize {
    let arity = crate::ARITY;
    if arity == 2 {
        // For binary arity the smallest "arity-power" >= n is just the next power of two.
        ceiling_pow2(n)
    } else if n == 0 {
        1
    } else if is_arity_pow(n, arity) {
        n
    } else {
        arity * floor_arity_pow(n)
    }
}

/// Returns the largest power of `arity` that is less than or equal to `n`.
///
/// Use crate ARITY internally for consistent behavior.
#[inline]
pub(crate) fn floor_arity_pow(n: usize) -> usize {
    let arity = crate::ARITY;
    if arity == 2 {
        // For binary arity the largest "arity-power" <= n is just the floor power of two.
        floor_pow2(n)
    } else if n > 0 {
        let arity_alignment = arity.trailing_zeros();
        // compute shift amount as usize
        let shift = (arity_alignment * (floor_pow2(n).trailing_zeros() / arity_alignment)) as usize;
        1usize << shift
    } else {
        1
    }
}

/// Calculates the height of a tree needed to hold `n` leaves.
#[inline]
pub(crate) fn height(n: usize) -> usize {
    let arity = crate::ARITY;
    if arity == 2 {
        // For binary arity the height is simply log2(ceiling_pow2(n))
        ceiling_arity_pow(n).trailing_zeros() as usize
    } else {
        (ceiling_arity_pow(n).trailing_zeros() / arity.trailing_zeros()) as usize
    }
}

/// Partitions `n` leaves into chunks based on powers of `arity`.
///
/// This is used to divide a large dataset into multiple base trees in the MMR structure.
/// Returns a vector of offsets where each partition begins.
///
/// Note: This function is only used in tests (MMR structure tests) and not in production code.
#[cfg(test)]
pub(crate) fn partition_by_arity(n: usize) -> Vec<usize> {
    let arity = crate::ARITY;
    let mut k = n;
    let mut offsets = vec![0usize];

    while k >= arity {
        let curr_floor_arity_pow = floor_arity_pow(k);

        offsets.push(curr_floor_arity_pow + offsets.last().expect("not empty"));

        k -= curr_floor_arity_pow;
    }
    if k > 0 {
        offsets.push(k + offsets.last().expect("not empty"));
    }
    offsets
}

#[cfg(test)]
mod tests {
    use crate::height;
    use crate::layer_size;
    use crate::num_of_prefixed_for_input;
    use crate::utils::{ceiling_arity_pow, floor_arity_pow, partition_by_arity};
    use crate::ARITY;
    extern crate alloc;
    use alloc::vec;

    #[test]
    fn height_only_test() {
        let _a = ARITY;
        assert_eq!(0, height(0));
        assert_eq!(0, height(1));
        assert_eq!(1, height(2));
        assert_eq!(2, height(3));
        assert_eq!(3, height(7));
        assert_eq!(3, height(8));
        assert_eq!(4, height(9));
        // Since arity is fixed, additional cases for other arities removed
    }

    #[test]
    fn num_of_prefixed_for_input_test() {
        assert_eq!(1, num_of_prefixed_for_input(1));
        assert_eq!(1, num_of_prefixed_for_input(2));
        assert_eq!(3, num_of_prefixed_for_input(3));
        assert_eq!(3, num_of_prefixed_for_input(4));
        assert_eq!(7, num_of_prefixed_for_input(5));
    }

    #[test]
    fn floor_arity_pow_test() {
        assert_eq!(1, floor_arity_pow(0));
        assert_eq!(4, floor_arity_pow(7));
        assert_eq!(8, floor_arity_pow(15));
    }
    #[test]
    fn ceiling_arity_pow_test() {
        assert_eq!(1, ceiling_arity_pow(0));
        assert_eq!(1, ceiling_arity_pow(1));
        assert_eq!(8, ceiling_arity_pow(7));
        assert_eq!(16, ceiling_arity_pow(15));
        assert_eq!(16, ceiling_arity_pow(16));
    }

    #[test]
    fn partition_by_arity_test() {
        let _a = ARITY;
        assert_eq!(vec![0, 4], partition_by_arity(4));
        assert_eq!(vec![0, 4, 5], partition_by_arity(5));
        assert_eq!(vec![0, 4, 6], partition_by_arity(6));
        assert_eq!(vec![0, 4, 6, 7], partition_by_arity(7));
        assert_eq!(vec![0, 32, 34, 35], partition_by_arity(35));
        // Removed multi-arity partition tests since arity is fixed
    }

    #[test]
    fn layer_size_test() {
        let height = 1usize;
        assert_eq!(1, layer_size(height, 0));
    }
}
