use rlp::{Decodable, Rlp};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fmt::Debug, hash::Hash, ops::Range};
use utils::json_serializable::JsonSerializable;
use utils::utils::U248_BYTE_COUNT;

pub trait ClaimQueryField: TryFrom<usize> + Serialize {    
    fn as_usize(&self) -> usize;
//    fn last() -> Self;
    fn inner_range(&self) -> Option<Range<usize>>;
    fn inner_index(&self) -> Option<usize>;

    fn as_felt_offsets(&self, rlp: &Rlp) -> Result<Range<usize>, ClaimQueryFieldError> {
        self
            .as_byte_offsets(rlp)
            .map(|r| 
                (r.start / U248_BYTE_COUNT)..(r.end / U248_BYTE_COUNT + usize::from(r.end % U248_BYTE_COUNT != 0)))
    }

    fn as_byte_offsets(&self, rlp: &Rlp) -> Result<Range<usize>, ClaimQueryFieldError> {
        use ClaimQueryFieldError::*;

        let n: usize = self.as_usize();
        let rlp_at_n = rlp.at(n).map_err(RlpDecoder)?;
        let pi = rlp_at_n
            .payload_info()
            .map_err(RlpDecoder)?;

        let payload_len = pi.header_len + pi.value_len;
        let payload_range = 0..payload_len;

        let preceding_range = match n {
            0 => 0..0,
            _ => Self::try_from(n - 1)
                    .map_err(|_| InvalidFieldIndex(n - 1))?
                    .as_byte_offsets(rlp)?      
        };

        let range_to_add = if let Some(inner_range) = self.inner_range() {
            inner_range
        } else if let Some(inner_index) = self.inner_index() {
            let mut accum_range = 0..0;
            for i in 0..inner_index {
                let pi = rlp_at_n
                    .at(i)
                    .map_err(RlpDecoder)?
                    .payload_info()
                    .map_err(RlpDecoder)?;

                accum_range.start = accum_range.end;
                accum_range.end += pi.header_len + pi.value_len;
            }
            accum_range.start..accum_range.end
        } else {
            payload_range.clone()
        };

        if range_to_add.end > payload_len {
            Err(InvalidPayloadOffset(range_to_add.clone()))
        }
        else {
            Ok((preceding_range.end + range_to_add.start)..(preceding_range.end + range_to_add.end))
        }
    }

    // fn try_from_byte_offsets(r: &Range<usize>, rlp: &rlp::Rlp) -> Result<Self, ClaimQueryFieldError> {
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

pub trait ClaimQuery {
    fn as_byte_offsets(&self, rlp: &Rlp) -> Result<Vec<Range<usize>>, ClaimQueryFieldError>;
    fn as_felt_offsets(&self, rlp: &Rlp) -> Result<Vec<Range<usize>>, ClaimQueryFieldError>;
}

#[derive(Debug, PartialEq, Clone)]
pub enum ClaimQueryFieldError {
    RlpDecoder(rlp::DecoderError),
    InvalidFieldIndex(usize),
    InvalidPayloadOffset(Range<usize>),
}

// #[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
// pub(crate) struct QueryFeltOffsets(pub Vec<Range<usize>>);

// impl QueryFeltOffsets {
//     pub fn ranges(&self) -> &[Range<usize>] {
//         &self.0
//     }

//     // pub fn as_byte_offsets(&self) -> Vec<Range<usize>> {
//     //     self.0
//     //         .iter()
//     //         .map(|r| r.start * U248_BYTE_COUNT..r.end * U248_BYTE_COUNT)
//     //         .collect()
//     // }
// }

//impl JsonSerializable for QueryFeltOffsets {}

#[derive(Serialize, Deserialize, PartialEq, Eq, Hash,  Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub enum LegacyTxClaimQueryField {
    Nonce,
    GasPrice,
    GasLimit,
    To,
    Value,
    SingleDataRelativeRange(Option<Range<usize>>),
    Signature,
}

impl ClaimQueryField for LegacyTxClaimQueryField {
    fn as_usize(&self) -> usize {
        use LegacyTxClaimQueryField::*;
        match self {
            Nonce => 0,
            GasPrice => 1,
            GasLimit => 2,
            To => 3,
            Value => 4,
            SingleDataRelativeRange(_) => 5,
            Signature => 6,
        }
    }

    fn inner_range(&self) -> Option<Range<usize>> {
        use LegacyTxClaimQueryField::*;

        match self {
            SingleDataRelativeRange(inner_range) => inner_range.clone(),
            _ => None,
        }
    }
    fn inner_index(&self) -> Option<usize> {
        None
    }
    // fn last() -> Self {
    //     Self::Signature
    // }
}

impl TryFrom<usize> for LegacyTxClaimQueryField {
    type Error = ClaimQueryFieldError;

    fn try_from(n: usize) -> Result<Self, Self::Error> {
        use LegacyTxClaimQueryField::*;
        match n {
            0 => Ok(Nonce),
            1 => Ok(GasPrice),
            2 => Ok(GasLimit),
            3 => Ok(To),
            4 => Ok(Value),
            5 => Ok(SingleDataRelativeRange(Default::default())),
            6 => Ok(Signature),
            n => Err(ClaimQueryFieldError::InvalidFieldIndex(n))
        }
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Hash,  Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub enum Eip2930TxClaimQueryField {
    ChainId,
    Nonce,
    GasPrice,
    GasLimit,
    To,
    Value,
    SingleDataRelativeRange(Option<Range<usize>>),
    AccessListItem(Option<usize>),
    Signature,
}

impl ClaimQueryField for Eip2930TxClaimQueryField {
    fn as_usize(&self) -> usize {
        use Eip2930TxClaimQueryField::*;
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
        }
    }

    // fn last() -> Self {
    //     Self::Signature
    // }

    fn inner_range(&self) -> Option<Range<usize>> {
        use Eip2930TxClaimQueryField::*;

        match self {
            SingleDataRelativeRange(inner_range) => inner_range.clone(),
            _ => None,
        }
    }
    fn inner_index(&self) -> Option<usize> {
        use Eip2930TxClaimQueryField::*;

        match self {
            AccessListItem(index) => index.clone(),
            _ => None,
        }
    }
}

impl TryFrom<usize> for Eip2930TxClaimQueryField {
    type Error = ClaimQueryFieldError;

    fn try_from(n: usize) -> Result<Self, Self::Error> {
        use Eip2930TxClaimQueryField::*;
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
            n => Err(ClaimQueryFieldError::InvalidFieldIndex(n))
        }
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Hash, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub enum Eip1559TxClaimQueryField {
    ChainId,
    Nonce,
    MaxPriorityFeePerGas,
    MaxFeePerGas,
    GasLimit,
    To,
    Value,
    SingleDataRelativeRange(Option<Range<usize>>),
    AccessListItem(Option<usize>),
    Signature,
}

impl ClaimQueryField for Eip1559TxClaimQueryField {
    fn as_usize(&self) -> usize {
        use Eip1559TxClaimQueryField::*;
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
        }
    }

    // fn last() -> Self {
    //     Self::Signature
    // }

    fn inner_range(&self) -> Option<Range<usize>> {
        use Eip1559TxClaimQueryField::*;

        match self {
            SingleDataRelativeRange(inner_range) => inner_range.clone(),
            _ => None,
        }
    }
    fn inner_index(&self) -> Option<usize> {
        use Eip1559TxClaimQueryField::*;

        match self {
            AccessListItem(index) => index.clone(),
            _ => None,
        }
    }
}

impl TryFrom<usize> for Eip1559TxClaimQueryField {
    type Error = ClaimQueryFieldError;

    fn try_from(n: usize) -> Result<Self, Self::Error> {
        use Eip1559TxClaimQueryField::*;
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
            n => Err(ClaimQueryFieldError::InvalidFieldIndex(n))
        }
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Hash,  Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub enum Eip4844TxClaimQueryField {
    ChainId,
    Nonce,
    MaxPriorityFeePerGas,
    MaxFeePerGas,
    GasLimit,
    To,
    Value,
    SingleDataRelativeRange(Option<Range<usize>>),
    AccessListItem(Option<usize>),
    MaxFeePerBlobGas,
    BlobVersionedHashes,
    Signature,
}

impl ClaimQueryField for Eip4844TxClaimQueryField {
    fn as_usize(&self) -> usize {
        use Eip4844TxClaimQueryField::*;
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
            BlobVersionedHashes => 10,
            Signature => 11,
        }
    }

    // fn last() -> Self {
    //     Self::Signature
    // }

    fn inner_range(&self) -> Option<Range<usize>> {
        use Eip4844TxClaimQueryField::*;

        match self {
            SingleDataRelativeRange(inner_range) => inner_range.clone(),
            _ => None,
        }
    }
    fn inner_index(&self) -> Option<usize> {
        use Eip4844TxClaimQueryField::*;

        match self {
            AccessListItem(index) => index.clone(),
            _ => None,
        }
    }
}

impl TryFrom<usize> for Eip4844TxClaimQueryField {
    type Error = ClaimQueryFieldError;

    fn try_from(n: usize) -> Result<Self, Self::Error> {
        use Eip4844TxClaimQueryField::*;
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
            10 => Ok(BlobVersionedHashes),
            11 => Ok(Signature),
            n => Err(ClaimQueryFieldError::InvalidFieldIndex(n))
        }
    }
}

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub enum TxClaimQuery {
    TargetLegacyType(TypedClaimQuery<LegacyTxClaimQueryField>),
    TargetEip2930Type(TypedClaimQuery<Eip2930TxClaimQueryField>),
    TargetEip1559Type(TypedClaimQuery<Eip1559TxClaimQueryField>),
    TargetEip4844Type(TypedClaimQuery<Eip4844TxClaimQueryField>),
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

impl TryFrom<HashSet<LegacyTxClaimQueryField>> for TxClaimQuery {
    type Error = anyhow::Error;

    fn try_from(fields: HashSet<LegacyTxClaimQueryField>) -> Result<Self, Self::Error> {
        Ok(Self::TargetLegacyType(TypedClaimQuery::<LegacyTxClaimQueryField>::try_from(fields)?))
    }
}

impl TryFrom<HashSet<Eip2930TxClaimQueryField>> for TxClaimQuery {
    type Error = anyhow::Error;

    fn try_from(fields: HashSet<Eip2930TxClaimQueryField>) -> Result<Self, Self::Error> {
        Ok(Self::TargetEip2930Type(TypedClaimQuery::<Eip2930TxClaimQueryField>::try_from(fields)?))
    }
}

impl TryFrom<HashSet<Eip1559TxClaimQueryField>> for TxClaimQuery {
    type Error = anyhow::Error;

    fn try_from(fields: HashSet<Eip1559TxClaimQueryField>) -> Result<Self, Self::Error> {
        Ok(Self::TargetEip1559Type(TypedClaimQuery::<Eip1559TxClaimQueryField>::try_from(fields)?))
    }
}

impl TryFrom<HashSet<Eip4844TxClaimQueryField>> for TxClaimQuery {
    type Error = anyhow::Error;

    fn try_from(fields: HashSet<Eip4844TxClaimQueryField>) -> Result<Self, Self::Error> {
        Ok(Self::TargetEip4844Type(TypedClaimQuery::<Eip4844TxClaimQueryField>::try_from(fields)?))
    }
}

impl JsonSerializable for TxClaimQuery {}

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub struct TypedClaimQuery<T: ClaimQueryField>(Vec<T>,);

impl<T: ClaimQueryField> TypedClaimQuery<T> {
    pub fn query(&self) -> &Vec<T> {
        &self.0
    }

    pub fn as_felt_offsets(&self, rlp: &Rlp) -> Result<Vec<Range<usize>>, ClaimQueryFieldError> {
        self
            .query()
            .iter()
            .map(|field| field.as_felt_offsets(&rlp))
            .collect::<Result<_, _>>()
    }

    pub fn as_byte_offsets(&self, rlp: &Rlp) -> Result<Vec<Range<usize>>, ClaimQueryFieldError> {
        self
            .query()
            .iter()
            .map(|field| field.as_byte_offsets(&rlp))
            .collect::<Result<_, _>>()
    }

    // pub fn try_from_byte_offsets(ranges: &[Range<usize>], rlp: &Rlp) -> Result<Self, ClaimQueryFieldError> {
    //     ranges
    //         .into_iter()
    //         .map(|r| T::try_from_byte_offsets(r, rlp))
    //         .collect::<Result<_, _>>()
    //         .map(Self)
    // }

    pub fn for_each_field<'a, F>(&self, rlp: &'a Rlp, f: F) -> Result<(), ClaimQueryFieldError> 
        where F: Fn(&T, &'a Rlp) -> Result<(), ClaimQueryFieldError>
    {
        self
            .query()
            .iter()
            .try_for_each(|field| f(field, rlp))
    }
}

impl<T: ClaimQueryField> TryFrom<HashSet<T>> for TypedClaimQuery<T> {
    type Error = anyhow::Error;

    fn try_from(fields: HashSet<T>) -> Result<Self, Self::Error> {
        (!fields.is_empty())
            .then_some(Self(fields.into_iter().collect()))
            .ok_or(anyhow::anyhow!("no fields in claim query"))
    }
}

impl<T: ClaimQueryField + for <'a> Deserialize<'a>> JsonSerializable for TypedClaimQuery<T> {}

pub type Eip1559RxClaimQueryField = Eip658RxClaimQueryField;
pub type Eip2930RxClaimQueryField = Eip658RxClaimQueryField;
pub type Eip4844RxClaimQueryField = Eip658RxClaimQueryField;

#[derive(Serialize, Deserialize, PartialEq, Eq, Hash, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub enum Eip658RxClaimQueryField {
    StatusCode,
    UsedGas,
    LogsBloom,
    SingleLog(Option<usize>),
}

impl ClaimQueryField for Eip658RxClaimQueryField {
    fn as_usize(&self) -> usize {
        use Eip658RxClaimQueryField::*;
        match self {
            StatusCode => 0,
            UsedGas => 1,
            LogsBloom => 2,
            SingleLog(_) => 3,
        }
    }

    fn inner_range(&self) -> Option<Range<usize>> {
        None
    }

    fn inner_index(&self) -> Option<usize> {
        use Eip658RxClaimQueryField::*;

        match self {
            SingleLog(log_index) => *log_index,
            _ => None,
        }
    }
}

impl TryFrom<usize> for Eip658RxClaimQueryField {
    type Error = ClaimQueryFieldError;

    fn try_from(n: usize) -> Result<Self, Self::Error> {
        use Eip658RxClaimQueryField::*;
        match n {
            0 => Ok(StatusCode),
            1 => Ok(UsedGas),
            2 => Ok(LogsBloom),
            3 => Ok(SingleLog(None)),
            n => Err(ClaimQueryFieldError::InvalidFieldIndex(n))
        }
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Hash, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub enum FrontierClaimQueryField {
    StateRoot,
    UsedGas,
    LogsBloom,
    SingleLog(Option<usize>),
}

impl ClaimQueryField for FrontierClaimQueryField {
    fn as_usize(&self) -> usize {
        use FrontierClaimQueryField::*;
        match self {
            StateRoot => 0,
            UsedGas => 1,
            LogsBloom => 2,
            SingleLog(_) => 3,
        }
    }

    fn inner_range(&self) -> Option<Range<usize>> {
        None
    }

    fn inner_index(&self) -> Option<usize> {
        use FrontierClaimQueryField::*;

        match self {
            SingleLog(log_index) => *log_index,
            _ => None,
        }
    }
}

impl TryFrom<usize> for FrontierClaimQueryField {
    type Error = ClaimQueryFieldError;

    fn try_from(n: usize) -> Result<Self, Self::Error> {
        use FrontierClaimQueryField::*;
        match n {
            0 => Ok(StateRoot),
            1 => Ok(UsedGas),
            2 => Ok(LogsBloom),
            3 => Ok(SingleLog(None)),
            n => Err(ClaimQueryFieldError::InvalidFieldIndex(n))
        }
    }
}

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub enum RxClaimQuery {
    TargetFrontierType(TypedClaimQuery<FrontierClaimQueryField>),
    TargetEip2930Type(TypedClaimQuery<Eip658RxClaimQueryField>),
    TargetEip1559Type(TypedClaimQuery<Eip658RxClaimQueryField>),
    TargetEip4844Type(TypedClaimQuery<Eip658RxClaimQueryField>),
}

impl ClaimQuery for RxClaimQuery {
    fn as_byte_offsets(&self, rlp: &Rlp) -> Result<Vec<Range<usize>>, ClaimQueryFieldError> {
        match self {
            // Self::TargetLegacyType(query) => unimplemented!(),
            Self::TargetFrontierType(query) => query.as_byte_offsets(rlp),
            Self::TargetEip2930Type(query) => query.as_byte_offsets(rlp),
            Self::TargetEip1559Type(query) => query.as_byte_offsets(rlp),
            Self::TargetEip4844Type(query) => query.as_byte_offsets(rlp),
        }  
    }
    fn as_felt_offsets(&self, rlp: &Rlp) -> Result<Vec<Range<usize>>, ClaimQueryFieldError> {
        match self {
            // Self::TargetLegacyType(query) => unimplemented!(),
            Self::TargetFrontierType(query) => query.as_felt_offsets(rlp),
            Self::TargetEip2930Type(query) => query.as_felt_offsets(rlp),
            Self::TargetEip1559Type(query) => query.as_felt_offsets(rlp),
            Self::TargetEip4844Type(query) => query.as_felt_offsets(rlp),
        }  
    }
}

// impl TryFrom<HashSet<LegacyTxClaimQueryField>> for TxClaimQuery {
//     type Error = anyhow::Error;

//     fn try_from(fields: HashSet<LegacyTxClaimQueryField>) -> Result<Self, Self::Error> {
//         Ok(Self::TargetLegacyType(TypedClaimQuery::<LegacyTxClaimQueryField>::try_from(fields)?))
//     }
// }

// impl TryFrom<HashSet<Eip2930TxClaimQueryField>> for TxClaimQuery {
//     type Error = anyhow::Error;

//     fn try_from(fields: HashSet<Eip2930TxClaimQueryField>) -> Result<Self, Self::Error> {
//         Ok(Self::TargetEip2930Type(TypedClaimQuery::<Eip2930TxClaimQueryField>::try_from(fields)?))
//     }
// }

impl TryFrom<HashSet<Eip658RxClaimQueryField>> for RxClaimQuery {
    type Error = anyhow::Error;

    fn try_from(fields: HashSet<Eip658RxClaimQueryField>) -> Result<Self, Self::Error> {
        Ok(Self::TargetEip1559Type(TypedClaimQuery::<Eip658RxClaimQueryField>::try_from(fields)?))
    }
}

impl TryFrom<HashSet<FrontierClaimQueryField>> for RxClaimQuery {
    type Error = anyhow::Error;

    fn try_from(fields: HashSet<FrontierClaimQueryField>) -> Result<Self, Self::Error> {
        Ok(Self::TargetFrontierType(TypedClaimQuery::<FrontierClaimQueryField>::try_from(fields)?))
    }
}

impl JsonSerializable for RxClaimQuery {}

// #[cfg(test)]
// mod tests {
//     use std::collections::HashSet;

//     use crate::{claim_query::{Eip2930TxClaimQueryField, TxClaimQuery, TypedClaimQuery}, U256};
//     use crate::transaction::TypedTransaction;
//     use crate::sorted_block::SortedBlock;
//     use crate::claim_query::{ClaimQueryField, Eip4844TxClaimQueryField};
//     use utils::utils::U248_BYTE_COUNT;
//     use utils::json_serializable::JsonSerializable;

//     #[test]
//     fn claim_query_serialize_test() {
//         let fname = "./claim_query.json";
//         println!("file name: {fname}");
//         let claim_query = TypedClaimQuery::<Eip2930TxClaimQueryField>::try_from(
//             vec![Eip2930TxClaimQueryField::To, Eip2930TxClaimQueryField::Nonce].into_iter().collect::<HashSet<_>>()
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
//             vec![Eip2930TxClaimQueryField::To, Eip2930TxClaimQueryField::Nonce].into_iter().collect::<HashSet<_>>()
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
//                 Eip4844TxClaimQueryField::To, 
//                 Eip4844TxClaimQueryField::SingleDataRelativeRange(Some(2..4)),
//                 Eip4844TxClaimQueryField::Nonce,
//                 Eip4844TxClaimQueryField::SingleDataRelativeRange(Some(3..7)),
//                 Eip4844TxClaimQueryField::SingleDataRelativeRange(None)
//             ]
//             .into_iter()
//             .collect::<HashSet<_>>()
//         )
//         .unwrap();

//         claim_query.to_file(fname).unwrap();
//         assert_eq!(claim_query, TxClaimQuery::try_from_file(fname).unwrap());
//     }

//     // #[tokio::test]
//     // async fn claim_query_to_felts_test() {
//     //     use Eip4844TxClaimQueryField::*;

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
//     //             To, 
//     //             SingleDataRelativeRange(Some(2..4)),
//     //             Nonce,
//     //             SingleDataRelativeRange(Some(3..7)),
//     //             SingleDataRelativeRange(None)
//     //         ]
//     //         .into_iter()
//     //         .collect::<HashSet<_>>()
//     //     )
//     //     .unwrap();

//     //     let claim = NewClaim::new(rlp, claim_query);

//     //     let felt_offsets = claim.felt_offsets().unwrap();
//     //     felt_offsets.to_file(fname).unwrap();
//     //     assert_eq!(felt_offsets, QueryFeltOffsets::try_from_file(fname).unwrap());
//     // }

//     #[tokio::test]
//     async fn query_test_1() {
//         use Eip4844TxClaimQueryField::*;
//         let block = 19543673.into();
//         let tx_cache = &mut <TypedTransaction as crate::FetchFromBlock>::Cache::new(
//             &crate::block_cache_dir(),
//             block,
//         );
//         let sorted_transactions_block =
//             SortedBlock::<TypedTransaction>::try_fetch("no_url_only_cache", Some(tx_cache), block)
//                 .await
//                 .unwrap();
//         let payload_bytes = sorted_transactions_block.iter().nth(95).unwrap().payload_bytes();

//         println!("{:?}", payload_bytes);
//         let rlp = rlp::Rlp::new(&payload_bytes[..]);
//         let x = rlp::decode::<U256>(rlp.at(2).unwrap().as_raw());
//         println!("x: {x:?}", );
//         let x = rlp::decode::<U256>(rlp.at(3).unwrap().as_raw());
//         println!("x: {x:?}", );
//         let x = rlp.val_at::<u64>(0);
//         println!("x: {x:?}", );
//         let x = rlp.val_at::<U256>(2);
//         println!("x: {x:?}", );
//         let x = rlp.val_at::<U256>(3);
//         println!("x: {x:?}", );
//         let x = rlp.val_at::<U256>(4);
//         println!("x: {x:?}", );

//         let x = rlp.val_at::<Vec<u8>>(7).unwrap();
//         println!("x: {:?}", hex::encode(&x));

//         let offsets = ChainId.as_byte_offsets(&rlp::Rlp::new(&payload_bytes[..]));
//         assert_eq!(offsets, Ok(0..1));
//         let offsets = Nonce.as_byte_offsets(&rlp::Rlp::new(&payload_bytes[..]));
//         assert_eq!(offsets, Ok(1..2));


//         // let offsets = Eip4844TxClaimQueryField::SingleDataRelativeRange(42..43).as_byte_offsets(&rlp::Rlp::new(&payload_bytes[..]));
//         // assert_eq!(offsets, 1..x.len());
//         let offsets = MaxPriorityFeePerGas.as_byte_offsets(&rlp::Rlp::new(&payload_bytes[..]));
//         assert_eq!(offsets, Ok(2..7));
//         let offsets = MaxFeePerGas.as_byte_offsets(&rlp::Rlp::new(&payload_bytes[..]));
//         assert_eq!(offsets, Ok(7..13));
//         let offsets = SingleDataRelativeRange(None).as_byte_offsets(&rlp::Rlp::new(&payload_bytes[..]));
//         assert_eq!(offsets, Ok(38..77));
//         let offsets = SingleDataRelativeRange(Some(0..1)).as_byte_offsets(&rlp::Rlp::new(&payload_bytes[..]));
//         assert_eq!(offsets, Ok(38..39));
//         let offsets = AccessListItem(None).as_byte_offsets(&rlp::Rlp::new(&payload_bytes[..]));
//         assert_eq!(offsets, Ok(77..78));
//         let offsets = MaxFeePerBlobGas.as_byte_offsets(&rlp::Rlp::new(&payload_bytes[..]));
//         assert_eq!(offsets, Ok(78..79));

//         let decoded_field = MaxFeePerGas.decode_payload::<U256>(&rlp::Rlp::new(&payload_bytes[..]));
//         assert_eq!(decoded_field, Ok(40000000000u64.into()));

//         assert!(SingleDataRelativeRange(None).decode_payload::<Vec<u8>>(&rlp).is_ok());
//         assert!(rlp.val_at::<Vec<u8>>(SingleDataRelativeRange(None).as_usize()).is_ok());
//     }

//     #[tokio::test]
//     async fn query_test_2() {
//         use Eip4844TxClaimQueryField::*;
//         let block = 19543673.into();
//         let tx_cache = &mut <TypedTransaction as crate::FetchFromBlock>::Cache::new(
//             &crate::block_cache_dir(),
//             block,
//         );
//         let sorted_transactions_block: SortedBlock<TypedTransaction> =
//             SortedBlock::<TypedTransaction>::try_fetch("no_url_only_cache", Some(tx_cache), block)
//                 .await
//                 .unwrap();
//         let payload_bytes = sorted_transactions_block.iter().nth(95).unwrap().payload_bytes();

//         let felts = felts_from_bytes(&payload_bytes[..]);

//         let felt_offsets = ChainId.as_felt_offsets(&rlp::Rlp::new(&payload_bytes[..]));
//         assert_eq!(felt_offsets, Ok(0..1));

//         let felt_offsets = felt_offsets.unwrap();
//         let bytes_chain_id = felts_to_bytes(&felts[felt_offsets], None);

// //        println!("bytes_chain_id: {:?}", &bytes_chain_id[offsets.clone()]);

//         assert_eq!(&payload_bytes[..31], &bytes_chain_id[..]);
//     }

//     #[tokio::test]
//     async fn query_test_3() {
//         use Eip4844TxClaimQueryField::*;
//         let block = 19543673.into();
//         let tx_cache = &mut <TypedTransaction as crate::FetchFromBlock>::Cache::new(
//             &crate::block_cache_dir(),
//             block,
//         );
//         let sorted_transactions_block: SortedBlock<TypedTransaction> =
//             SortedBlock::<TypedTransaction>::try_fetch("no_url_only_cache", Some(tx_cache), block)
//                 .await
//                 .unwrap();
//         let payload_bytes = sorted_transactions_block.iter().nth(95).unwrap().payload_bytes();

//         let felts = felts_from_bytes(&payload_bytes[..]);
//         let felt_offsets = GasLimit.as_felt_offsets(&rlp::Rlp::new(&payload_bytes[..]));
//         assert_eq!(felt_offsets, Ok(0..1));

//         let felt_offsets = felt_offsets.unwrap();
//         let bytes_gas_limit = felts_to_bytes(&felts[felt_offsets], None);

//         assert_eq!(&payload_bytes[..31], &bytes_gas_limit[..]);
//     }

//     #[tokio::test]
//     async fn query_test_4() {
//         use Eip4844TxClaimQueryField::*;
//         let block = 19543673.into();
//         let tx_cache = &mut <TypedTransaction as crate::FetchFromBlock>::Cache::new(
//             &crate::block_cache_dir(),
//             block,
//         );
//         let sorted_transactions_block: SortedBlock<TypedTransaction> =
//             SortedBlock::<TypedTransaction>::try_fetch("no_url_only_cache", Some(tx_cache), block)
//                 .await
//                 .unwrap();
//         let payload_bytes = sorted_transactions_block.iter().nth(95).unwrap().payload_bytes();

//         let felts = felts_from_bytes(&payload_bytes[..]);
//         let felt_offsets = SingleDataRelativeRange(None).as_felt_offsets(&rlp::Rlp::new(&payload_bytes[..]));
//         assert_eq!(felt_offsets, Ok(1..3));

//         let felt_offsets = felt_offsets.unwrap();
//         let bytes_data = felts_to_bytes(&felts[felt_offsets], None);

//         assert_eq!(&payload_bytes[31..31*3], &bytes_data[..]);
//     }

//     #[tokio::test]
//     async fn query_test_5() {
//         use Eip4844TxClaimQueryField::*;
//         let block = 19543673.into();
//         let tx_cache = &mut <TypedTransaction as crate::FetchFromBlock>::Cache::new(
//             &crate::block_cache_dir(),
//             block,
//         );
//         let sorted_transactions_block: SortedBlock<TypedTransaction> =
//             SortedBlock::<TypedTransaction>::try_fetch("no_url_only_cache", Some(tx_cache), block)
//                 .await
//                 .unwrap();
//         let payload_bytes = sorted_transactions_block.iter().nth(95).unwrap().payload_bytes();

//         let offsets = SingleDataRelativeRange(Some(24..30)).as_byte_offsets(&rlp::Rlp::new(&payload_bytes[..]));

//         println!("payload offsets: {:?}", offsets);

//         let felts = felts_from_bytes(&payload_bytes[..]);
//         let felt_offsets = SingleDataRelativeRange(Some(24..30)).as_felt_offsets(&rlp::Rlp::new(&payload_bytes[..]));
//         assert_eq!(felt_offsets, Ok(2..3));

//         let felt_offsets = felt_offsets.unwrap();
//         let bytes_data = felts_to_bytes(&felts[felt_offsets.clone()], None);

//         assert_eq!(
//             &payload_bytes[U248_BYTE_COUNT * felt_offsets.start..U248_BYTE_COUNT * felt_offsets.end], 
//             &bytes_data[..]
//         );
//     }
// }
