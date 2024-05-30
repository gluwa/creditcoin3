use crate::SOME_FRAGMENT_SIZE;
use pallet_attestation_poc::types::Attestation;

pub struct AttestationFragment<H, A>
where
    H: Clone,
{
    attestations: [Attestation<H, A>; SOME_FRAGMENT_SIZE],
    len: usize,
}

impl<H, A> AttestationFragment<H, A>
where
    H: Clone,
{
    // fn new() -> Self {
    //     Self {
    //         attestations: [, SOME_FRAGMENT_SIZE],
    //         len: 0,
    //     }
    // }

    pub fn blocks(&self) -> &[Attestation<H, A>] {
        &self.attestations[0..self.len]
    }

    pub fn head(&self) -> Option<&Attestation<H, A>> {
        if self.len == 0 {
            None
        } else {
            Some(&self.attestations[self.len - 1])
        }
    }

    pub fn tail(&self) -> Option<&Attestation<H, A>> {
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
    ) -> Option<FragmentSlice<H, A>> {
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

pub struct FragmentSlice<'a, H: Clone, A>(&'a [Attestation<H, A>]);

impl<'a, H: Clone, A> FragmentSlice<'a, H, A> {
    pub fn start(&self) -> Option<u64> {
        self.0.first().map(Attestation::header_number)
    }
}
