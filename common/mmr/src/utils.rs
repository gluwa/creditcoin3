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
pub fn layer_size(_arity: usize, height: usize, layer_index: usize) -> usize {
    // Ignore passed arity and use global ARITY constant
    let arity = crate::ARITY;
    if height == 0 {
        1
    } else {
        1 << (arity.trailing_zeros() as usize * (height - layer_index - 1))
    }
}

/// Calculates the total number of prefixed blocks needed to store a tree with `n` leaves.
#[inline]
pub fn num_of_prefixed_for_input(n: usize, _arity: usize) -> usize {
    // Force use of ARITY constant
    let arity = crate::ARITY;
    let height = height(n, arity);
    (arity * layer_size(arity, height, 0) - 1) / (arity - 1)
}

/// Converts a leaf index to its location in the prefixed storage.
///
/// Returns `(prefixed_index, offset)` where `prefixed_index` is the index of the
/// prefixed block and `offset` is the position within that block.
#[inline]
pub(crate) fn location_in_prefixed(input_index: usize, _arity: usize) -> (usize, usize) {
    // Use fixed ARITY
    let arity = crate::ARITY;
    let offset = input_index & (arity - 1);
    let index = input_index >> arity.trailing_zeros();
    (index, offset)
}

/// Checks if a number is a power of 2.
#[inline]
fn is_pow2(n: usize) -> bool {
    (n.leading_zeros() + n.trailing_zeros() + 1) as usize == usize::BITS as usize
}

/// Returns the smallest power of 2 that is greater than or equal to `n`.
#[inline]
fn ceiling_pow2(n: usize) -> usize {
    if is_pow2(n) {
        n
    } else {
        1 << (usize::BITS as usize - n.leading_zeros() as usize)
    }
}

/// Returns the largest power of 2 that is less than or equal to `n`.
#[inline]
fn floor_pow2(n: usize) -> usize {
    let ceiling_pow2 = ceiling_pow2(n);
    if ceiling_pow2 == n {
        n
    } else {
        ceiling_pow2 >> 1
    }
}

/// Checks if `n` is a power of the given arity.
#[inline]
fn is_arity_pow(n: usize, arity: usize) -> bool {
    is_pow2(n) && n.trailing_zeros() % arity.trailing_zeros() == 0
}

/// Returns the smallest power of `arity` that is greater than or equal to `n`.
#[inline]
pub(crate) fn ceiling_arity_pow(n: usize, _arity: usize) -> usize {
    let arity = crate::ARITY;
    if n == 0 {
        1
    } else if is_arity_pow(n, arity) {
        n
    } else {
        arity * floor_arity_pow(n, arity)
    }
}

/// Returns the largest power of `arity` that is less than or equal to `n`.
#[inline]
pub(crate) fn floor_arity_pow(n: usize, _arity: usize) -> usize {
    let arity = crate::ARITY;
    if n > 0 {
        let arity_alignment = arity.trailing_zeros();
        1 << (arity_alignment * (floor_pow2(n).trailing_zeros() / arity_alignment)) as usize
    } else {
        1
    }
}

/// Calculates the height of a tree needed to hold `n` leaves.
#[inline]
pub(crate) fn height(n: usize, _arity: usize) -> usize {
    let arity = crate::ARITY;
    (ceiling_arity_pow(n, arity).trailing_zeros() / arity.trailing_zeros()) as usize
}

/// Partitions `n` leaves into chunks based on powers of `arity`.
///
/// This is used to divide a large dataset into multiple base trees in the MMR structure.
/// Returns a vector of offsets where each partition begins.
///
/// Note: This function is only used in tests (MMR structure tests) and not in production code.
#[cfg(test)]
pub(crate) fn partition_by_arity(n: usize, _arity: usize) -> Vec<usize> {
    let arity = crate::ARITY;
    let mut k = n;
    let mut offsets = vec![0usize];

    while k >= arity {
        let curr_floor_arity_pow = floor_arity_pow(k, arity);

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
        let a = ARITY;
        assert_eq!(0, height(0, a));
        assert_eq!(0, height(1, a));
        assert_eq!(1, height(2, a));
        assert_eq!(2, height(3, a));
        assert_eq!(3, height(7, a));
        assert_eq!(3, height(8, a));
        assert_eq!(4, height(9, a));
        // Since arity is fixed, additional cases for other arities removed
    }

    #[test]
    fn num_of_prefixed_for_input_test() {
        let a = ARITY;
        assert_eq!(1, num_of_prefixed_for_input(1, a));
        assert_eq!(1, num_of_prefixed_for_input(2, a));
        assert_eq!(3, num_of_prefixed_for_input(3, a));
        assert_eq!(3, num_of_prefixed_for_input(4, a));
        assert_eq!(7, num_of_prefixed_for_input(5, a));
    }

    #[test]
    fn floor_arity_pow_test() {
        let a = ARITY;
        assert_eq!(1, floor_arity_pow(0, a));
        assert_eq!(4, floor_arity_pow(7, a));
        assert_eq!(8, floor_arity_pow(15, a));
    }
    #[test]
    fn ceiling_arity_pow_test() {
        let a = ARITY;
        assert_eq!(1, ceiling_arity_pow(0, a));
        assert_eq!(1, ceiling_arity_pow(1, a));
        assert_eq!(8, ceiling_arity_pow(7, a));
        assert_eq!(16, ceiling_arity_pow(15, a));
        assert_eq!(16, ceiling_arity_pow(16, a));
    }

    #[test]
    fn partition_by_arity_test() {
        let a = ARITY;
        assert_eq!(vec![0, 4], partition_by_arity(4, a));
        assert_eq!(vec![0, 4, 5], partition_by_arity(5, a));
        assert_eq!(vec![0, 4, 6], partition_by_arity(6, a));
        assert_eq!(vec![0, 4, 6, 7], partition_by_arity(7, a));
        assert_eq!(vec![0, 32, 34, 35], partition_by_arity(35, a));
        // Removed multi-arity partition tests since arity is fixed
    }

    #[test]
    fn layer_size_test() {
        let arity = ARITY;
        let height = 1usize;
        assert_eq!(1, layer_size(arity, height, 0));
    }
}
