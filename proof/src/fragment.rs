use crate::SOME_FRAGMENT_SIZE;
use attestor_primitives::BlockAttestation;
use serde::Serialize;

pub struct AttestationFragment {
    attestations: [BlockAttestation; SOME_FRAGMENT_SIZE],
    len: usize,
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

#[derive(Serialize)]
pub struct FragmentSlice<'a>(&'a [BlockAttestation]);

impl<'a> FragmentSlice<'a> {
    pub fn start(&self) -> Option<u64> {
        self.0.first().map(BlockAttestation::header_number)
    }
}
