use crate::SOME_FRAGMENT_SIZE;
use attestor_primitives::BlockAttestation;
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;

pub struct AttestationFragment {
    pub attestations: [BlockAttestation; SOME_FRAGMENT_SIZE],
    pub len: usize,
}

impl AttestationFragment {
    pub fn blocks(&self) -> &[BlockAttestation] {
        &self.attestations[0..self.len]
    }

    pub fn head(&self) -> Option<&BlockAttestation> {
        if self.len == 0 {
            None
        } else {
            Some(&self.attestations[self.len - 1])
        }
    }

    pub fn tail(&self) -> Option<&BlockAttestation> {
        if self.is_empty() {
            None
        } else {
            Some(&self.attestations[0])
        }
    }

    pub fn is_full(&self) -> bool {
        self.len() == SOME_FRAGMENT_SIZE
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn attestation_slice_for(
        &self,
        block_number: u64,
        upper_bound: Option<u64>,
    ) -> Option<FragmentSlice> {
        let tail = self.tail().map(|att| att.header_number())?;
        let head = self.head().map(|att| att.header_number())?;
        if tail < block_number && head >= block_number {
            Some(FragmentSlice(
                &self.attestations[(block_number - tail - 1) as usize
                    ..(upper_bound.unwrap_or(head) + 1 - tail) as usize],
            ))
        } else {
            None
        }
    }
}

#[derive(Clone)]
pub struct FragmentSlice<'a>(&'a [BlockAttestation]);

impl<'a> FragmentSlice<'a> {
    pub fn start(&self) -> Option<u64> {
        self.0.first().map(BlockAttestation::header_number)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FragmentSliceSerializable<'a> {
    pub blocks: Vec<BlockAttestationSerializable<'a>>,
}

impl<'a> From<FragmentSlice<'a>> for FragmentSliceSerializable<'a> {
    fn from(slice: FragmentSlice<'a>) -> Self {
        Self {
            blocks: slice
                .0
                .iter()
                .map(BlockAttestationSerializable::from)
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockAttestationSerializable<'a> {
    block_number: String,
    tx_root: String,
    rx_root: String,
    prev_digest: String,
    digest: String,
    #[serde(skip_serializing, skip_deserializing)]
    _marker: PhantomData<&'a ()>,
}

impl<'a> From<&'a BlockAttestation> for BlockAttestationSerializable<'a> {
    fn from(b: &'a BlockAttestation) -> Self {
        Self {
            block_number: b.block_number.to_string(),
            tx_root: b.tx_root.to_string(),
            rx_root: b.rx_root.to_string(),
            prev_digest: b.prev_digest.to_string(),
            digest: b.digest.to_string(),
            _marker: PhantomData,
        }
    }
}

impl TryFrom<BlockAttestationSerializable<'_>> for BlockAttestation {
    type Error = ();

    fn try_from(block: BlockAttestationSerializable) -> Result<Self, ()> {
        Ok(Self {
            block_number: block.block_number.parse().map_err(|_| ())?,
            tx_root: starknet_crypto::FieldElement::from_dec_str(block.tx_root.as_ref())
                .map_err(|_| ())?,
            rx_root: starknet_crypto::FieldElement::from_dec_str(block.rx_root.as_ref())
                .map_err(|_| ())?,
            prev_digest: starknet_crypto::FieldElement::from_dec_str(block.prev_digest.as_ref())
                .map_err(|_| ())?,
            digest: starknet_crypto::FieldElement::from_dec_str(block.digest.as_ref())
                .map_err(|_| ())?,
        })
    }
}
