//! The batch entry points' `bytes[]` argument.

use crate::{MaxBatchBytes, MaxBatchSize, MaxEncodedTxBytes};
use precompile_utils::prelude::*;
use precompile_utils::solidity::codec::{Reader, Writer};
use sp_core::{Get, U256};
use sp_std::vec::Vec;

/// The `encoded_transactions` argument of the batch `verify` / `verifyAndEmit` overloads.
///
/// ABI-identical to `bytes[]`. At most [`MaxBatchSize`] elements of at most
/// [`MaxEncodedTxBytes`] each, and at most [`MaxBatchBytes`] in total, with each element
/// counted at its length rounded up to a power of two.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchEncodedTxs(Vec<BoundedBytes<MaxEncodedTxBytes>>);

impl From<Vec<BoundedBytes<MaxEncodedTxBytes>>> for BatchEncodedTxs {
    fn from(value: Vec<BoundedBytes<MaxEncodedTxBytes>>) -> Self {
        Self(value)
    }
}

impl From<BatchEncodedTxs> for Vec<BoundedBytes<MaxEncodedTxBytes>> {
    fn from(value: BatchEncodedTxs) -> Self {
        value.0
    }
}

/// Read a 32-byte ABI length word as a `usize`.
fn read_length(reader: &mut Reader) -> MayRevert<usize> {
    reader
        .read::<U256>()
        .map_err(|_| RevertReason::read_out_of_bounds("length"))?
        .try_into()
        .map_err(|_| RevertReason::value_is_too_large("length").into())
}

impl solidity::Codec for BatchEncodedTxs {
    fn read(reader: &mut Reader) -> MayRevert<Self> {
        let mut array_reader = reader.read_pointer()?;

        let array_size = read_length(&mut array_reader)?;
        if array_size > <MaxBatchSize as Get<u32>>::get() as usize {
            return Err(RevertReason::value_is_too_large("length").into());
        }

        // Element offsets are relative to the first word after the array length.
        let mut item_reader = Reader::new(array_reader.read_till_end()?);

        let max_item = <MaxEncodedTxBytes as Get<u32>>::get() as usize;
        let max_total = <MaxBatchBytes as Get<u32>>::get() as usize;
        let mut total: usize = 0;
        let mut array = Vec::with_capacity(array_size);

        for i in 0..array_size {
            let mut bytes_reader = item_reader.read_pointer().in_array(i)?;

            // Check the declared length against both limits.
            let len = read_length(&mut bytes_reader).in_array(i)?;
            if len > max_item {
                return Err(RevertReason::value_is_too_large("length").in_array(i));
            }
            // An empty item counts as 0 (`0.next_power_of_two()` is 1).
            let reserved = if len == 0 { 0 } else { len.next_power_of_two() };
            total = total.saturating_add(reserved);
            if total > max_total {
                return Err(RevertReason::value_is_too_large("batch length").in_array(i));
            }

            let data = bytes_reader.read_raw_bytes(len).in_array(i)?;
            array.push(BoundedBytes::from(data));
        }

        Ok(Self(array))
    }

    fn write(writer: &mut Writer, value: Self) {
        solidity::Codec::write(writer, value.0);
    }

    fn has_static_size() -> bool {
        false
    }

    fn signature() -> solidity::codec::String {
        <Vec<BoundedBytes<MaxEncodedTxBytes>> as solidity::Codec>::signature()
    }
}
