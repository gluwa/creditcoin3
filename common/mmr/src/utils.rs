//use core::mem::size_of;
use crate::Arity;
// #[macro_export]
// macro_rules! max_leaves {
//     ($arity:expr, $height:expr) => {
//         $arity * layer_size!($arity, $height, 0)
//     };
// }
#[inline]
pub fn layer_size(arity: Arity, height: usize, layer_index: usize) -> usize {
    let arity = arity as usize;
    if height == 0 {
        1
    } else {
        1 << (arity.trailing_zeros() as usize * (height - layer_index - 1))
    }
}

#[inline]
pub fn num_of_prefixed_for_input(n: usize, arity: Arity) -> usize {
    let height = height(n, arity);
    (arity as usize * layer_size(arity, height, 0) - 1) / (arity as usize - 1)
}

#[inline]
pub(crate) fn location_in_prefixed(input_index: usize, arity: Arity) -> (usize, usize) {
    let arity = arity as usize;
    let offset = input_index & (arity - 1); // index modulo ARITY
    let index = input_index >> arity.trailing_zeros();
    (index, offset)
}

#[inline]
fn is_pow2(n: usize) -> bool {
    (n.leading_zeros() + n.trailing_zeros() + 1) as usize == usize::BITS as usize
}

#[inline]
fn ceiling_pow2(n: usize) -> usize {
    if is_pow2(n) {
        n
    } else {
        1 << (usize::BITS as usize - n.leading_zeros() as usize)
    }
}

#[inline]
fn floor_pow2(n: usize) -> usize {
    let ceiling_pow2 = ceiling_pow2(n);
    if ceiling_pow2 == n {
        n
    } else {
        ceiling_pow2 >> 1
    }
}

#[inline]
fn is_arity_pow(n: usize, arity: usize) -> bool {
    is_pow2(n) && n.trailing_zeros() % arity.trailing_zeros() == 0
}

#[inline]
pub(crate) fn ceiling_arity_pow(n: usize, arity: Arity) -> usize {
    if n == 0 {
        1
    } else if is_arity_pow(n, arity as usize) {
        n
    } else {
        arity as usize * floor_arity_pow(n, arity)
    }
}

#[inline]
pub(crate) fn floor_arity_pow(n: usize, arity: Arity) -> usize {
    let arity = arity as usize;
    if n > 0 {
        let arity_alignment = arity.trailing_zeros();

        1 << (arity_alignment * (floor_pow2(n).trailing_zeros() / arity_alignment)) as usize
    } else {
        1
    }
}

#[inline]
pub(crate) fn height(n: usize, arity: Arity) -> usize {
    (ceiling_arity_pow(n, arity).trailing_zeros() / (arity as usize).trailing_zeros()) as usize
}

pub(crate) fn partition_by_arity(n: usize, arity: Arity) -> Vec<usize> {
    let mut k = n;
    let mut offsets = vec![0usize];

    while k >= arity as usize {
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
    use crate::Arity;

    #[test]
    fn height_only_test() {
        let a = Arity::Two;
        assert_eq!(0, height(0, a));
        assert_eq!(0, height(1, a));
        assert_eq!(1, height(2, a));
        assert_eq!(2, height(3, a));
        assert_eq!(3, height(7, a));
        assert_eq!(3, height(8, a));
        assert_eq!(4, height(9, a));

        let a = Arity::Four;
        assert_eq!(0, height(1, a));
        assert_eq!(0, height(1, a));
        assert_eq!(1, height(2, a));
        assert_eq!(1, height(3, a));
        assert_eq!(1, height(4, a));
        assert_eq!(2, height(5, a));
        assert_eq!(2, height(15, a));
        assert_eq!(2, height(16, a));
        assert_eq!(3, height(17, a));
        assert_eq!(3, height(64, a));
        assert_eq!(4, height(65, a));
    }

    #[test]
    fn num_of_prefixed_for_input_test() {
        let a = Arity::Two;
        assert_eq!(1, num_of_prefixed_for_input(1, a));
        assert_eq!(1, num_of_prefixed_for_input(2, a));
        assert_eq!(3, num_of_prefixed_for_input(3, a));
        assert_eq!(3, num_of_prefixed_for_input(4, a));
        assert_eq!(7, num_of_prefixed_for_input(5, a));
    }

    #[test]
    fn floor_arity_pow_test() {
        let a = Arity::Two;
        assert_eq!(1, floor_arity_pow(0, a));
        assert_eq!(4, floor_arity_pow(7, a));
        assert_eq!(8, floor_arity_pow(15, a));

        let a = Arity::Four;
        assert_eq!(4, floor_arity_pow(7, a));
        assert_eq!(4, floor_arity_pow(15, a));
        assert_eq!(1, floor_arity_pow(3, a));
        assert_eq!(16, floor_arity_pow(16, a));
        assert_eq!(16, floor_arity_pow(63, a));
    }
    #[test]
    fn ceiling_arity_pow_test() {
        let a = Arity::Two;
        assert_eq!(1, ceiling_arity_pow(0, a));
        assert_eq!(1, ceiling_arity_pow(1, a));
        assert_eq!(8, ceiling_arity_pow(7, a));
        assert_eq!(16, ceiling_arity_pow(15, a));
        assert_eq!(16, ceiling_arity_pow(16, a));
        let a = Arity::Four;
        assert_eq!(16, ceiling_arity_pow(7, a));
        assert_eq!(16, ceiling_arity_pow(15, a));
        assert_eq!(4, ceiling_arity_pow(3, a));
        assert_eq!(16, ceiling_arity_pow(16, a));
        assert_eq!(64, ceiling_arity_pow(63, a));
        assert_eq!(256, ceiling_arity_pow(256, a));

        let a = Arity::Eight;
        assert_eq!(1, ceiling_arity_pow(1, a));
        assert_eq!(8, ceiling_arity_pow(2, a));
        assert_eq!(8, ceiling_arity_pow(8, a));
        assert_eq!(64, ceiling_arity_pow(15, a));
        assert_eq!(64, ceiling_arity_pow(16, a));
        assert_eq!(64, ceiling_arity_pow(63, a));
    }

    #[test]
    fn partition_by_arity_test() {
        let a = Arity::Two;
        assert_eq!(vec![0, 4], partition_by_arity(4, a));
        assert_eq!(vec![0, 4, 5], partition_by_arity(5, a));
        assert_eq!(vec![0, 4, 6], partition_by_arity(6, a));
        assert_eq!(vec![0, 4, 6, 7], partition_by_arity(7, a));
        let a = Arity::Four;
        assert_eq!(vec![0, 4], partition_by_arity(4, a));
        assert_eq!(vec![0, 4, 5], partition_by_arity(5, a));
        assert_eq!(vec![0, 4, 6], partition_by_arity(6, a));
        assert_eq!(vec![0, 4, 7], partition_by_arity(7, a));
        assert_eq!(vec![0], partition_by_arity(0, a));
        assert_eq!(vec![0, 1], partition_by_arity(1, a));
        assert_eq!(vec![0, 2], partition_by_arity(2, a));
        assert_eq!(vec![0, 3], partition_by_arity(3, a));

        assert_eq!(vec![0, 4, 8], partition_by_arity(8, a));

        assert_eq!(vec![0, 4, 8, 12, 15], partition_by_arity(15, a));

        assert_eq!(
            vec![0, 16, 32, 48, 52, 56, 60, 63],
            partition_by_arity(63, a)
        );
        assert_eq!(vec![0, 64], partition_by_arity(64, a));
    }

    #[test]
    fn layer_size_test() {
        let arity = Arity::Two;
        let height = 1usize;
        assert_eq!(1, layer_size(arity, height, 0));

        let arity = Arity::Four;
        let height = 1usize;
        assert_eq!(1, layer_size(arity, height, 0));
    }
}
