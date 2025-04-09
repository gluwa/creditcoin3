use anyhow::Result;
use sp_core::H256;
use tracing::debug;

use crate::{
    attestation_fragment::{
        AttestationFragment, AttestationFragmentError, AttestationFragmentSerializable,
    },
    block::{Block as FragmentBlock, BlockError},
};

use eth::{Client, Error as EthError};
use mmr::traits::MerkleTreeTrait;
use utils::Felt;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Invalid Fragment Length, {0}")]
    InvalidFragmentLength(u64),
    #[error("Attestation fragment error: {0}")]
    Fragment(#[from] AttestationFragmentError),
    #[error("Attestation fragment block eth error: {0}")]
    Eth(#[from] EthError),
    #[error("Attestation fragment block error: {0}")]
    BlockError(#[from] BlockError),
}

pub struct Manager<'a> {
    start_block: u64,
    end_block: u64,
    eth_client: &'a Client,
}

pub struct CreateResult {
    pub continuity_proof: AttestationFragmentSerializable,
    pub previous_fragment_block: FragmentBlock,
    pub prev_digest: Option<H256>,
}

impl<'a> Manager<'a> {
    pub fn new(start_block: u64, end_block: u64, eth_client: &'a Client) -> Self {
        Self {
            start_block,
            end_block,
            eth_client,
        }
    }

    pub async fn create(&self, prev_digest: H256) -> Result<CreateResult, Error> {
        // Only for genesis block we don't need to build a fragment
        if self.end_block == 0 {
            debug!("No need to build full fragment for genesis block");
            let serializeable_frament =
                AttestationFragmentSerializable::from(&AttestationFragment::new(0));
            return Ok(CreateResult {
                continuity_proof: serializeable_frament,
                previous_fragment_block: Default::default(),
                prev_digest: None,
            });
        }

        // Fragment size is the difference between the attestation header number and the last finalized attestation header number
        // Start and end block are inclusive
        let fragment_size = self.end_block - self.start_block + 1;
        let fragment_length = usize::try_from(fragment_size)
            .map_err(|_| Error::InvalidFragmentLength(fragment_size))?;

        // Create a new fragment with the correct length
        let mut fragment = AttestationFragment::new(fragment_length);

        debug!(
            "Building fragment for interval: {} - {}",
            self.start_block, self.end_block
        );

        // Construct the previous fragment block (in this case it will always be an attestation or checkpoint)
        // This is needed because the prover needs to be able to always have the previous block in the fragment
        let previous_block_number = self.start_block - 1;
        let previous_block = self.eth_client.get_block(previous_block_number).await?;
        let prev_merkle_root = eth::starknet_pedersen_mmr(&previous_block);
        let previous_fragment_block = FragmentBlock::new_with_digest(
            previous_block_number,
            prev_merkle_root.root().0,
            Felt::from_bytes_be(&prev_digest.0),
        );

        // Start building the fragment for the interval
        for i in self.start_block..self.end_block + 1 {
            let block = self.eth_client.get_block(i).await?;

            let merkle_root = eth::starknet_pedersen_mmr(&block);
            let fragment_block = FragmentBlock::new(block.number(), merkle_root.root().0);
            debug!("appending block to fragment: {:?}", fragment_block);

            // If this is the first block in the fragment, we need to construct the block from the previous block
            // In order to set the prev_digest correctly
            // In the other case, the `try_append_block` method on fragment will take care of this if the fragment is not empty
            let fragment_block = if fragment.is_empty() {
                debug!("Constructing first block from previous block");
                FragmentBlock::new_from_prev(
                    block.number(),
                    merkle_root.root().0,
                    Felt::from_bytes_be(&prev_digest.0),
                )
            } else {
                fragment_block
            };

            fragment.try_append_block(fragment_block)?;
        }

        // Construct the prev digest for the fragment
        let mut prev_digest = None;
        if let Some(last_block) = fragment.head() {
            prev_digest = Some(H256::from(last_block.prev_digest().to_bytes_be()));
        }

        // Serialize the fragment to be sent over the wire
        let serialized_fragment = AttestationFragmentSerializable::from(&fragment);

        Ok(CreateResult {
            continuity_proof: serialized_fragment,
            previous_fragment_block,
            prev_digest,
        })
    }
}
