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
            for i in 0..=inner_index {
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
    BlobVersionedHashes(Option<usize>),
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
            BlobVersionedHashes(_) => 10,
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
            AccessListItem(index) 
            |
            BlobVersionedHashes(index) => index.clone(),
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
            10 => Ok(BlobVersionedHashes(Default::default())),
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

