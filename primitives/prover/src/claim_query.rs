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
                Err(InvalidPayloadOffset(
                    range_to_add.start as u64..range_to_add.end as u64,
                ))
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
#[derive(Debug, PartialEq, Clone)]
pub enum ClaimQueryFieldError {
    RlpDecoder(rlp::DecoderError),
    InvalidFieldIndex(usize),
    InvalidPayloadOffset(Range<u64>),
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

// impl<T: ClaimQueryField + for<'a> Deserialize<'a>> JsonSerializable for TypedClaimQuery<T> {}

// impl JsonSerializable for RxClaimQuery {}

// #[cfg(test)]
// mod tests {
//     use hashbrown::HashSet;

//     use utils::block_item_traits::BlockItem;
//     use crate::claim_query::{ClaimQueryField, Eip4844ClaimQueryField};
//     use crate::sorted_block::SortedBlock;
//     use crate::transaction::TypedTransaction;
//     use crate::utils::U248_BYTE_COUNT;
//     use crate::{
//         claim_query::{Eip2930ClaimQueryField, TxClaimQuery, TypedClaimQuery},
//         // json_serializable::JsonSerializable,
//         utils::{felts_from_bytes, felts_to_bytes},
//         U256,
//     };

//     #[test]
//     fn claim_query_serialize_test() {
//         let fname = "./claim_query.json";
//         println!("file name: {fname}");
//         let claim_query = TypedClaimQuery::<Eip2930ClaimQueryField>::try_from(
//             vec![
//                 Eip2930ClaimQueryField::To,
//                 Eip2930ClaimQueryField::Nonce,
//             ]
//             .into_iter()
//             .collect::<HashSet<_>>(),
//         )
//         .unwrap();

//         claim_query.to_file(fname).unwrap();
//         assert_eq!(claim_query, TypedClaimQuery::try_from_file(fname).unwrap());
//     }

//     #[test]
//     fn claim_query_serialize_eip2930_test() {
//         let fname = "./claim_query_eip2930.json";
//         println!("file name: {fname}");
//         let claim_query = TxClaimQuery::try_from(
//             vec![
//                 Eip2930ClaimQueryField::To,
//                 Eip2930ClaimQueryField::Nonce,
//             ]
//             .into_iter()
//             .collect::<HashSet<_>>(),
//         )
//         .unwrap();

//         claim_query.to_file(fname).unwrap();
//         assert_eq!(claim_query, TxClaimQuery::try_from_file(fname).unwrap());
//     }

//     #[test]
//     fn claim_query_serialize_eip4844_test() {
//         let fname = "./claim_query_eip4844.json";
//         println!("file name: {fname}");
//         let claim_query = TxClaimQuery::try_from(
//             vec![
//                 Eip4844ClaimQueryField::To,
//                 Eip4844ClaimQueryField::SingleDataRelativeRange(Some(2..4)),
//                 Eip4844ClaimQueryField::Nonce,
//                 Eip4844ClaimQueryField::SingleDataRelativeRange(Some(3..7)),
//                 Eip4844ClaimQueryField::SingleDataRelativeRange(None),
//             ]
//             .into_iter()
//             .collect::<HashSet<_>>(),
//         )
//         .unwrap();

//         claim_query.to_file(fname).unwrap();
//         assert_eq!(claim_query, TxClaimQuery::try_from_file(fname).unwrap());
//     }

//     #[tokio::test]
//     async fn query_test_1() {
//         use Eip4844ClaimQueryField::*;
//         let block = 19543673.into();
//         let tx_cache = &mut <TypedTransaction as crate::FetchFromBlock>::Cache::new(
//             &crate::block_cache_dir(),
//             block,
//         );
//         let sorted_transactions_block =
//             SortedBlock::<TypedTransaction>::try_fetch("no_url_only_cache", Some(tx_cache), block)
//                 .await
//                 .unwrap();
//         let payload_bytes = sorted_transactions_block
//             .iter()
//             .nth(95)
//             .unwrap()
//             .payload_bytes();

//         println!("{:?}", payload_bytes);
//         let rlp = rlp::Rlp::new(&payload_bytes[..]);
//         let x = rlp::decode::<U256>(rlp.at(2).unwrap().as_raw());
//         println!("x: {x:?}",);
//         let x = rlp::decode::<U256>(rlp.at(3).unwrap().as_raw());
//         println!("x: {x:?}",);
//         let x = rlp.val_at::<u64>(0);
//         println!("x: {x:?}",);
//         let x = rlp.val_at::<U256>(2);
//         println!("x: {x:?}",);
//         let x = rlp.val_at::<U256>(3);
//         println!("x: {x:?}",);
//         let x = rlp.val_at::<U256>(4);
//         println!("x: {x:?}",);

//         let x = rlp.val_at::<Vec<u8>>(7).unwrap();
//         println!("x: {:?}", hex::encode(&x));

//         let offsets = ChainId.as_byte_offsets(&rlp::Rlp::new(&payload_bytes[..]));
//         assert_eq!(offsets, Ok(0..1));
//         let offsets = Nonce.as_byte_offsets(&rlp::Rlp::new(&payload_bytes[..]));
//         assert_eq!(offsets, Ok(1..2));

//         // let offsets = Eip4844ClaimQueryField::SingleDataRelativeRange(42..43).as_byte_offsets(&rlp::Rlp::new(&payload_bytes[..]));
//         // assert_eq!(offsets, 1..x.len());
//         let offsets = MaxPriorityFeePerGas.as_byte_offsets(&rlp::Rlp::new(&payload_bytes[..]));
//         assert_eq!(offsets, Ok(2..7));
//         let offsets = MaxFeePerGas.as_byte_offsets(&rlp::Rlp::new(&payload_bytes[..]));
//         assert_eq!(offsets, Ok(7..13));
//         let offsets =
//             SingleDataRelativeRange(None).as_byte_offsets(&rlp::Rlp::new(&payload_bytes[..]));
//         assert_eq!(offsets, Ok(38..77));
//         let offsets =
//             SingleDataRelativeRange(Some(0..1)).as_byte_offsets(&rlp::Rlp::new(&payload_bytes[..]));
//         assert_eq!(offsets, Ok(38..39));
//         let offsets = AccessListItem(None).as_byte_offsets(&rlp::Rlp::new(&payload_bytes[..]));
//         assert_eq!(offsets, Ok(77..78));

//         let decoded_field = MaxFeePerGas.decode_payload::<U256>(&rlp::Rlp::new(&payload_bytes[..]));
//         assert_eq!(decoded_field, Ok(40000000000u64.into()));

//         let decoded_field =
//             MaxFeePerBlobGas.decode_payload::<U256>(&rlp::Rlp::new(&payload_bytes[..]));
//         assert_eq!(decoded_field, Ok(14200000000u64.into()));

//         assert!(SingleDataRelativeRange(None)
//             .decode_payload::<Vec<u8>>(&rlp)
//             .is_ok());
//         assert!(rlp
//             .val_at::<Vec<u8>>(SingleDataRelativeRange(None).as_usize())
//             .is_ok());
//     }

//     #[tokio::test]
//     async fn query_test_2() {
//         use Eip4844ClaimQueryField::*;
//         let block = 19543673.into();
//         let tx_cache = &mut <TypedTransaction as crate::FetchFromBlock>::Cache::new(
//             &crate::block_cache_dir(),
//             block,
//         );
//         let sorted_transactions_block: SortedBlock<TypedTransaction> =
//             SortedBlock::<TypedTransaction>::try_fetch("no_url_only_cache", Some(tx_cache), block)
//                 .await
//                 .unwrap();
//         let payload_bytes = sorted_transactions_block
//             .iter()
//             .nth(95)
//             .unwrap()
//             .payload_bytes();

//         let felts = felts_from_bytes(&payload_bytes[..]);
//         let rlp = rlp::Rlp::new(&payload_bytes[..]);

//         let felt_offsets = ChainId.as_felt_offsets(&rlp);
//         assert_eq!(felt_offsets, Ok(0..1));

//         println!(
//             "--- {:?}",
//             BlobVersionedHashes(Some(0)).as_byte_offsets(&rlp)
//         );
//         println!(
//             "--- {:?}",
//             BlobVersionedHashes(Some(1)).as_byte_offsets(&rlp)
//         );
//         println!(
//             "--- {:?}",
//             BlobVersionedHashes(Some(2)).as_byte_offsets(&rlp)
//         );

//         let felt_offsets = felt_offsets.unwrap();
//         let bytes_chain_id = felts_to_bytes(&felts[felt_offsets], None);

//         //        println!("bytes_chain_id: {:?}", &bytes_chain_id[offsets.clone()]);

//         assert_eq!(&payload_bytes[..31], &bytes_chain_id[..]);
//     }

//     #[tokio::test]
//     async fn query_test_3() {
//         use Eip4844ClaimQueryField::*;
//         let block = 19543673.into();
//         let tx_cache = &mut <TypedTransaction as crate::FetchFromBlock>::Cache::new(
//             &crate::block_cache_dir(),
//             block,
//         );
//         let sorted_transactions_block: SortedBlock<TypedTransaction> =
//             SortedBlock::<TypedTransaction>::try_fetch("no_url_only_cache", Some(tx_cache), block)
//                 .await
//                 .unwrap();
//         let payload_bytes = sorted_transactions_block
//             .iter()
//             .nth(95)
//             .unwrap()
//             .payload_bytes();

//         let felts = felts_from_bytes(&payload_bytes[..]);
//         let felt_offsets = GasLimit.as_felt_offsets(&rlp::Rlp::new(&payload_bytes[..]));
//         assert_eq!(felt_offsets, Ok(0..1));

//         let felt_offsets = felt_offsets.unwrap();
//         let bytes_gas_limit = felts_to_bytes(&felts[felt_offsets], None);

//         assert_eq!(&payload_bytes[..31], &bytes_gas_limit[..]);
//     }

//     #[tokio::test]
//     async fn query_test_4() {
//         use Eip4844ClaimQueryField::*;
//         let block = 19543673.into();
//         let tx_cache = &mut <TypedTransaction as crate::FetchFromBlock>::Cache::new(
//             &crate::block_cache_dir(),
//             block,
//         );
//         let sorted_transactions_block: SortedBlock<TypedTransaction> =
//             SortedBlock::<TypedTransaction>::try_fetch("no_url_only_cache", Some(tx_cache), block)
//                 .await
//                 .unwrap();
//         let payload_bytes = sorted_transactions_block
//             .iter()
//             .nth(95)
//             .unwrap()
//             .payload_bytes();

//         let felts = felts_from_bytes(&payload_bytes[..]);
//         let felt_offsets =
//             SingleDataRelativeRange(None).as_felt_offsets(&rlp::Rlp::new(&payload_bytes[..]));
//         assert_eq!(felt_offsets, Ok(1..3));

//         let felt_offsets = felt_offsets.unwrap();
//         let bytes_data = felts_to_bytes(&felts[felt_offsets], None);

//         assert_eq!(&payload_bytes[31..31 * 3], &bytes_data[..]);
//     }

//     #[tokio::test]
//     async fn query_test_5() {
//         use Eip4844ClaimQueryField::*;
//         let block = 19543673.into();
//         let tx_cache = &mut <TypedTransaction as crate::FetchFromBlock>::Cache::new(
//             &crate::block_cache_dir(),
//             block,
//         );
//         let sorted_transactions_block: SortedBlock<TypedTransaction> =
//             SortedBlock::<TypedTransaction>::try_fetch("no_url_only_cache", Some(tx_cache), block)
//                 .await
//                 .unwrap();
//         let payload_bytes = sorted_transactions_block
//             .iter()
//             .nth(95)
//             .unwrap()
//             .payload_bytes();

//         let offsets = SingleDataRelativeRange(Some(24..30))
//             .as_byte_offsets(&rlp::Rlp::new(&payload_bytes[..]));

//         println!("payload offsets: {:?}", offsets);

//         let felts = felts_from_bytes(&payload_bytes[..]);
//         let felt_offsets = SingleDataRelativeRange(Some(24..30))
//             .as_felt_offsets(&rlp::Rlp::new(&payload_bytes[..]));
//         assert_eq!(felt_offsets, Ok(2..3));

//         let felt_offsets = felt_offsets.unwrap();
//         let bytes_data = felts_to_bytes(&felts[felt_offsets.clone()], None);

//         assert_eq!(
//             &payload_bytes
//                 [U248_BYTE_COUNT * felt_offsets.start..U248_BYTE_COUNT * felt_offsets.end],
//             &bytes_data[..]
//         );
//     }

//     // #[tokio::test]
//     // async fn claim_query_compacted_to_felts_test() {
//     //     use Eip4844ClaimQueryField::*;

//     //     let block = 19543673.into();
//     //     let tx_cache = &mut <TypedTransaction as crate::FetchFromBlock>::Cache::new(
//     //         &crate::block_cache_dir(),
//     //         block,
//     //     );
//     //     let sorted_transactions_block =
//     //         SortedBlock::<TypedTransaction>::try_fetch("no_url_only_cache", Some(tx_cache), block)
//     //             .await
//     //             .unwrap();
//     //     let payload_bytes = sorted_transactions_block.iter().nth(95).unwrap().payload_bytes();
//     //     let rlp = rlp::Rlp::new(&payload_bytes[..]);

//     //     let fname = "./claim_query_eip4844_felts.json";
//     //     println!("file name: {fname}");
//     //     let claim_query = TxClaimQuery::try_from(
//     //         vec![
//     //             SingleDataRelativeRange(Some(1..18)),
//     //             SingleDataRelativeRange(Some(7..39)),
//     //         ]
//     //         .into_iter()
//     //         .collect::<HashSet<_>>()
//     //     )
//     //     .unwrap();

//     //     let claim = NewClaim::new(rlp, claim_query);

//     //     let felt_offsets = claim.felt_offsets().unwrap();
//     //     felt_offsets.to_file(fname).unwrap();

//     //     let expected_offsets = claim.felt_offsets().unwrap();

//     //     assert_eq!(expected_offsets, QueryFeltOffsets::try_from_file(fname).unwrap());
//     // }
// }
