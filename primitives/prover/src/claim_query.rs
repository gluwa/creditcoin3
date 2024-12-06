use crate::claim_query::query_field::{ClaimQueryField, SampleBy};
use core::ops::Range;
use hashbrown::HashSet;
use parity_scale_codec::{Decode, Encode, MaxEncodedLen};
use rlp::Rlp;
use scale_info::TypeInfo;
use serde::{Deserialize, Serialize};
use sp_core::ConstU32;
use sp_runtime::BoundedVec;
use sp_std::vec::Vec;
use thiserror::Error;

mod query_field {
    use super::ClaimQueryFieldError;
    use core::ops::Range;
    use parity_scale_codec::{Decode, Encode, MaxEncodedLen};
    use rlp::{Decodable, Rlp};
    use scale_info::TypeInfo;
    use serde::Serialize;
    use utils::utils::U248_BYTE_COUNT;

    pub trait ClaimQueryField:
        TryFrom<usize> + Serialize + Encode + Decode + TypeInfo + MaxEncodedLen + core::fmt::Debug
    {
        fn as_usize(&self) -> usize;

        fn sample_by(&self) -> SampleBy;

        fn as_felt_offsets(&self, rlp: &Rlp) -> Result<Range<usize>, ClaimQueryFieldError> {
            self.as_byte_offsets(rlp).map(|r| {
                (r.start / U248_BYTE_COUNT)
                    ..(r.end / U248_BYTE_COUNT + usize::from(r.end % U248_BYTE_COUNT != 0))
            })
        }

        fn as_byte_offsets(&self, rlp: &Rlp) -> Result<Range<usize>, ClaimQueryFieldError> {
            use ClaimQueryFieldError::*;

            let n: usize = self.as_usize();
            let rlp_at_n = rlp.at(n).map_err(RlpDecoder)?;

            let pi = rlp_at_n.payload_info().map_err(RlpDecoder)?;

            let payload_len = pi.header_len + pi.value_len;
            let payload_range = 0..payload_len;

            let preceding_range = match n {
                0 => 0..0,
                _ => Self::try_from(n - 1)
                    .map_err(|_| InvalidFieldIndex(n - 1))?
                    .as_byte_offsets(rlp)?,
            };

            let range_to_add = match self.sample_by() {
                SampleBy::Value
                |
                // if range not specified sample the entire range
                SampleBy::Range(None)
                |
                // if index not specified sample the entire array
                SampleBy::Index(None) => payload_range.clone(),

                SampleBy::Range(Some(range)) => range.start as usize..range.end as usize,

                SampleBy::Index(Some(index)) => {
                    let mut accum_range = 0..0;
                    for i in 0..=index {
                        let pi = rlp_at_n
                            .at(i as usize)
                            .map_err(RlpDecoder)?
                            .payload_info()
                            .map_err(RlpDecoder)?;

                        accum_range.start = accum_range.end;
                        accum_range.end += pi.header_len + pi.value_len;
                    }
                    accum_range
                }
            };

            if range_to_add.end > payload_len {
                Err(InvalidPayloadOffset(range_to_add.clone()))
            } else {
                Ok((preceding_range.end + range_to_add.start)
                    ..(preceding_range.end + range_to_add.end))
            }
        }
        // fn try_from_byte_offsets(r: &Range<u64>, rlp: &rlp::Rlp) -> Result<Self, ClaimQueryFieldError> {
        //     for n in 0usize..=Self::last().as_usize() {
        //         let field = Self::try_from(n)
        //             .map_err(|_| ClaimQueryFieldError::InvalidFieldIndex(n))?;
        //         let field_range = field.as_byte_offsets(rlp)?;
        //         if r.start >= field_range.start && r.end <= field_range.end {
        //             return Ok(field);
        //         }
        //     }
        //     Err(ClaimQueryFieldError::InvalidPayloadOffset(r.clone()))
        // }

        fn decode_payload<T: Decodable>(&self, rlp: &rlp::Rlp) -> Result<T, rlp::DecoderError> {
            rlp.val_at::<T>(self.as_usize())
        }
    }

    // specifies how a particular field is to be sampled
    pub enum SampleBy {
        // sample a single value
        Value,
        // sample a byte range [a..b). If None, entire sampling range is implied
        Range(Option<Range<u64>>),
        // sample an array element by index. If None, entire array is implied
        Index(Option<u64>),
    }
}
pub trait ClaimQuery {
    fn as_byte_offsets(&self, rlp: &Rlp) -> Result<Vec<Range<usize>>, ClaimQueryFieldError>;
    fn as_felt_offsets(&self, rlp: &Rlp) -> Result<Vec<Range<usize>>, ClaimQueryFieldError>;
}
#[derive(Debug, PartialEq, Clone, Error)]
pub enum ClaimQueryFieldError {
    #[error("RLP decoding error: {0}")]
    RlpDecoder(#[from] rlp::DecoderError),

    #[error("Invalid field index: {0}")]
    InvalidFieldIndex(usize),

    #[error("Invalid payload offset: range {0:?}")]
    InvalidPayloadOffset(Range<usize>),
}

#[derive(
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    Hash,
    Debug,
    Clone,
    Encode,
    Decode,
    TypeInfo,
    MaxEncodedLen,
)]
#[serde(rename_all = "snake_case")]
pub enum LegacyClaimQueryField {
    Nonce,
    GasPrice,
    GasLimit,
    To,
    Value,
    SingleDataRelativeRange(Option<Range<u64>>),
    Signature,
    SignatureHash,

    StateRoot,
    UsedGas,
    LogsBloom,
    SingleLog(Option<u64>),
}

impl ClaimQueryField for LegacyClaimQueryField {
    fn as_usize(&self) -> usize {
        use LegacyClaimQueryField::*;
        match self {
            Nonce => 0,
            GasPrice => 1,
            GasLimit => 2,
            To => 3,
            Value => 4,
            SingleDataRelativeRange(_) => 5,
            Signature => 6,
            SignatureHash => 7,

            StateRoot => 8,
            UsedGas => 9,
            LogsBloom => 10,
            SingleLog(_) => 11,
        }
    }

    fn sample_by(&self) -> SampleBy {
        use LegacyClaimQueryField::*;

        match self {
            SingleDataRelativeRange(range) => SampleBy::Range(range.clone()),
            SingleLog(index) => SampleBy::Index(*index),
            _ => SampleBy::Value,
        }
    }
}

impl TryFrom<usize> for LegacyClaimQueryField {
    type Error = ClaimQueryFieldError;

    fn try_from(n: usize) -> Result<Self, Self::Error> {
        use LegacyClaimQueryField::*;
        match n {
            0 => Ok(Nonce),
            1 => Ok(GasPrice),
            2 => Ok(GasLimit),
            3 => Ok(To),
            4 => Ok(Value),
            5 => Ok(SingleDataRelativeRange(Default::default())),
            6 => Ok(Signature),
            7 => Ok(SignatureHash),

            8 => Ok(StateRoot),
            9 => Ok(UsedGas),
            10 => Ok(LogsBloom),
            11 => Ok(SingleLog(Default::default())),
            n => Err(ClaimQueryFieldError::InvalidFieldIndex(n)),
        }
    }
}

#[derive(
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    Hash,
    Debug,
    Clone,
    Encode,
    Decode,
    TypeInfo,
    MaxEncodedLen,
)]
#[serde(rename_all = "snake_case")]
pub enum Eip2930ClaimQueryField {
    ChainId,
    Nonce,
    GasPrice,
    GasLimit,
    To,
    Value,
    SingleDataRelativeRange(Option<Range<u64>>),
    AccessListItem(Option<u64>),
    Signature,
    SignatureHash,

    StatusCode,
    UsedGas,
    LogsBloom,
    SingleLog(Option<u64>),
}

impl ClaimQueryField for Eip2930ClaimQueryField {
    fn as_usize(&self) -> usize {
        use Eip2930ClaimQueryField::*;
        match self {
            ChainId => 0,
            Nonce => 1,
            GasPrice => 2,
            GasLimit => 3,
            To => 4,
            Value => 5,
            SingleDataRelativeRange(_) => 6,
            AccessListItem(_) => 7,
            Signature => 8,
            SignatureHash => 9,

            StatusCode => 10,
            UsedGas => 11,
            LogsBloom => 12,
            SingleLog(_) => 13,
        }
    }

    fn sample_by(&self) -> SampleBy {
        use Eip2930ClaimQueryField::*;

        match self {
            SingleDataRelativeRange(range) => SampleBy::Range(range.clone()),
            AccessListItem(index) => SampleBy::Index(*index),
            SingleLog(index) => SampleBy::Index(*index),
            _ => SampleBy::Value,
        }
    }
}

impl TryFrom<usize> for Eip2930ClaimQueryField {
    type Error = ClaimQueryFieldError;

    fn try_from(n: usize) -> Result<Self, Self::Error> {
        use Eip2930ClaimQueryField::*;
        match n {
            0 => Ok(ChainId),
            1 => Ok(Nonce),
            2 => Ok(GasPrice),
            3 => Ok(GasLimit),
            4 => Ok(To),
            5 => Ok(Value),
            6 => Ok(SingleDataRelativeRange(Default::default())),
            7 => Ok(AccessListItem(Default::default())),
            8 => Ok(Signature),
            9 => Ok(SignatureHash),

            10 => Ok(StatusCode),
            11 => Ok(UsedGas),
            12 => Ok(LogsBloom),
            13 => Ok(SingleLog(Default::default())),
            n => Err(ClaimQueryFieldError::InvalidFieldIndex(n)),
        }
    }
}

#[derive(
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    Hash,
    Debug,
    Clone,
    Encode,
    Decode,
    TypeInfo,
    MaxEncodedLen,
)]
#[serde(rename_all = "snake_case")]
pub enum Eip1559ClaimQueryField {
    ChainId,
    Nonce,
    MaxPriorityFeePerGas,
    MaxFeePerGas,
    GasLimit,
    To,
    Value,
    SingleDataRelativeRange(Option<Range<u64>>),
    AccessListItem(Option<u64>),
    Signature,
    SignatureHash,

    StatusCode,
    UsedGas,
    LogsBloom,
    SingleLog(Option<u64>),
}

impl ClaimQueryField for Eip1559ClaimQueryField {
    fn as_usize(&self) -> usize {
        use Eip1559ClaimQueryField::*;
        match self {
            ChainId => 0,
            Nonce => 1,
            MaxPriorityFeePerGas => 2,
            MaxFeePerGas => 3,
            GasLimit => 4,
            To => 5,
            Value => 6,
            SingleDataRelativeRange(_) => 7,
            AccessListItem(_) => 8,
            Signature => 9,
            SignatureHash => 10,

            StatusCode => 11,
            UsedGas => 12,
            LogsBloom => 13,
            SingleLog(_) => 14,
        }
    }

    fn sample_by(&self) -> SampleBy {
        use Eip1559ClaimQueryField::*;

        match self {
            SingleDataRelativeRange(range) => SampleBy::Range(range.clone()),
            AccessListItem(index) => SampleBy::Index(*index),
            SingleLog(index) => SampleBy::Index(*index),
            _ => SampleBy::Value,
        }
    }
}

impl TryFrom<usize> for Eip1559ClaimQueryField {
    type Error = ClaimQueryFieldError;

    fn try_from(n: usize) -> Result<Self, Self::Error> {
        use Eip1559ClaimQueryField::*;
        match n {
            0 => Ok(ChainId),
            1 => Ok(Nonce),
            2 => Ok(MaxPriorityFeePerGas),
            3 => Ok(MaxFeePerGas),
            4 => Ok(GasLimit),
            5 => Ok(To),
            6 => Ok(Value),
            7 => Ok(SingleDataRelativeRange(Default::default())),
            8 => Ok(AccessListItem(Default::default())),
            9 => Ok(Signature),
            10 => Ok(SignatureHash),

            11 => Ok(StatusCode),
            12 => Ok(UsedGas),
            13 => Ok(LogsBloom),
            14 => Ok(SingleLog(Default::default())),
            n => Err(ClaimQueryFieldError::InvalidFieldIndex(n)),
        }
    }
}

#[derive(
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    Hash,
    Debug,
    Clone,
    Encode,
    Decode,
    TypeInfo,
    MaxEncodedLen,
)]
#[serde(rename_all = "snake_case")]
pub enum Eip4844ClaimQueryField {
    ChainId,
    Nonce,
    MaxPriorityFeePerGas,
    MaxFeePerGas,
    GasLimit,
    To,
    Value,
    SingleDataRelativeRange(Option<Range<u64>>),
    AccessListItem(Option<u64>),
    MaxFeePerBlobGas,
    BlobVersionedHashes(Option<u64>),
    Signature,
    SignatureHash,

    StatusCode,
    UsedGas,
    LogsBloom,
    SingleLog(Option<u64>),
}

impl ClaimQueryField for Eip4844ClaimQueryField {
    fn as_usize(&self) -> usize {
        use Eip4844ClaimQueryField::*;
        match self {
            ChainId => 0,
            Nonce => 1,
            MaxPriorityFeePerGas => 2,
            MaxFeePerGas => 3,
            GasLimit => 4,
            To => 5,
            Value => 6,
            SingleDataRelativeRange(_) => 7,
            AccessListItem(_) => 8,
            MaxFeePerBlobGas => 9,
            BlobVersionedHashes(_) => 10,
            Signature => 11,
            SignatureHash => 12,

            StatusCode => 13,
            UsedGas => 14,
            LogsBloom => 15,
            SingleLog(_) => 16,
        }
    }

    fn sample_by(&self) -> SampleBy {
        use Eip4844ClaimQueryField::*;

        match self {
            SingleDataRelativeRange(range) => SampleBy::Range(range.clone()),
            AccessListItem(index) => SampleBy::Index(*index),
            SingleLog(index) => SampleBy::Index(*index),
            _ => SampleBy::Value,
        }
    }
}

impl TryFrom<usize> for Eip4844ClaimQueryField {
    type Error = ClaimQueryFieldError;

    fn try_from(n: usize) -> Result<Self, Self::Error> {
        use Eip4844ClaimQueryField::*;
        match n {
            0 => Ok(ChainId),
            1 => Ok(Nonce),
            2 => Ok(MaxPriorityFeePerGas),
            3 => Ok(MaxFeePerGas),
            4 => Ok(GasLimit),
            5 => Ok(To),
            6 => Ok(Value),
            7 => Ok(SingleDataRelativeRange(Default::default())),
            8 => Ok(AccessListItem(Default::default())),
            9 => Ok(MaxFeePerBlobGas),
            10 => Ok(BlobVersionedHashes(Default::default())),
            11 => Ok(Signature),
            12 => Ok(SignatureHash),

            13 => Ok(StatusCode),
            14 => Ok(UsedGas),
            15 => Ok(LogsBloom),
            16 => Ok(SingleLog(Default::default())),
            n => Err(ClaimQueryFieldError::InvalidFieldIndex(n)),
        }
    }
}

#[derive(
    Serialize, Deserialize, PartialEq, Debug, Clone, Encode, Decode, TypeInfo, MaxEncodedLen,
)]
pub enum TxClaimQuery {
    TargetLegacyType(TypedClaimQuery<LegacyClaimQueryField>),
    TargetEip2930Type(TypedClaimQuery<Eip2930ClaimQueryField>),
    TargetEip1559Type(TypedClaimQuery<Eip1559ClaimQueryField>),
    TargetEip4844Type(TypedClaimQuery<Eip4844ClaimQueryField>),
}

impl ClaimQuery for TxClaimQuery {
    fn as_byte_offsets(&self, rlp: &Rlp) -> Result<Vec<Range<usize>>, ClaimQueryFieldError> {
        match self {
            Self::TargetLegacyType(query) => query.as_byte_offsets(rlp),
            Self::TargetEip2930Type(query) => query.as_byte_offsets(rlp),
            Self::TargetEip1559Type(query) => query.as_byte_offsets(rlp),
            Self::TargetEip4844Type(query) => query.as_byte_offsets(rlp),
        }
    }
    fn as_felt_offsets(&self, rlp: &Rlp) -> Result<Vec<Range<usize>>, ClaimQueryFieldError> {
        match self {
            Self::TargetLegacyType(query) => query.as_felt_offsets(rlp),
            Self::TargetEip2930Type(query) => query.as_felt_offsets(rlp),
            Self::TargetEip1559Type(query) => query.as_felt_offsets(rlp),
            Self::TargetEip4844Type(query) => query.as_felt_offsets(rlp),
        }
    }
}

impl TryFrom<HashSet<LegacyClaimQueryField>> for TxClaimQuery {
    type Error = anyhow::Error;

    fn try_from(fields: HashSet<LegacyClaimQueryField>) -> Result<Self, Self::Error> {
        Ok(Self::TargetLegacyType(TypedClaimQuery::<
            LegacyClaimQueryField,
        >::try_from(fields)?))
    }
}

impl TryFrom<HashSet<Eip2930ClaimQueryField>> for TxClaimQuery {
    type Error = anyhow::Error;

    fn try_from(fields: HashSet<Eip2930ClaimQueryField>) -> Result<Self, Self::Error> {
        Ok(Self::TargetEip2930Type(TypedClaimQuery::<
            Eip2930ClaimQueryField,
        >::try_from(fields)?))
    }
}

impl TryFrom<HashSet<Eip1559ClaimQueryField>> for TxClaimQuery {
    type Error = anyhow::Error;

    fn try_from(fields: HashSet<Eip1559ClaimQueryField>) -> Result<Self, Self::Error> {
        Ok(Self::TargetEip1559Type(TypedClaimQuery::<
            Eip1559ClaimQueryField,
        >::try_from(fields)?))
    }
}

impl TryFrom<HashSet<Eip4844ClaimQueryField>> for TxClaimQuery {
    type Error = anyhow::Error;

    fn try_from(fields: HashSet<Eip4844ClaimQueryField>) -> Result<Self, Self::Error> {
        Ok(Self::TargetEip4844Type(TypedClaimQuery::<
            Eip4844ClaimQueryField,
        >::try_from(fields)?))
    }
}

pub(crate) const MAX_QUERY_LEN: usize = 42;

#[derive(
    Serialize, Deserialize, PartialEq, Debug, Clone, Encode, Decode, TypeInfo, MaxEncodedLen,
)]
pub struct TypedClaimQuery<T: ClaimQueryField>(BoundedVec<T, ConstU32<{ MAX_QUERY_LEN as u32 }>>);

impl<T: ClaimQueryField> TypedClaimQuery<T> {
    pub fn query(&self) -> &Vec<T> {
        &self.0
    }

    pub fn as_felt_offsets(&self, rlp: &Rlp) -> Result<Vec<Range<usize>>, ClaimQueryFieldError> {
        self.query()
            .iter()
            .map(|field| field.as_felt_offsets(rlp))
            .collect::<Result<_, _>>()
    }

    pub fn as_byte_offsets(&self, rlp: &Rlp) -> Result<Vec<Range<usize>>, ClaimQueryFieldError> {
        self.query()
            .iter()
            .map(|field| field.as_byte_offsets(rlp))
            .collect::<Result<_, _>>()
    }

    // pub fn try_from_byte_offsets(ranges: &[Range<u64>], rlp: &Rlp) -> Result<Self, ClaimQueryFieldError> {
    //     ranges
    //         .into_iter()
    //         .map(|r| T::try_from_byte_offsets(r, rlp))
    //         .collect::<Result<_, _>>()
    //         .map(Self)
    // }

    pub fn for_each_field<'a, F>(&self, rlp: &'a Rlp, f: F) -> Result<(), ClaimQueryFieldError>
    where
        F: Fn(&T, &'a Rlp) -> Result<(), ClaimQueryFieldError>,
    {
        self.query().iter().try_for_each(|field| f(field, rlp))
    }
}

impl<T: ClaimQueryField> TryFrom<HashSet<T>> for TypedClaimQuery<T> {
    type Error = anyhow::Error;

    fn try_from(fields: HashSet<T>) -> Result<Self, Self::Error> {
        if (1..MAX_QUERY_LEN).contains(&fields.len()) {
            Ok(Self(
                fields
                    .into_iter()
                    .collect::<Vec<_>>()
                    .try_into()
                    .map_err(|err| anyhow::anyhow!("{:?}", err))?,
            ))
        } else {
            Err(anyhow::anyhow!(
                "query length is expected to be in range [1..{})",
                MAX_QUERY_LEN
            ))
        }
    }
}
