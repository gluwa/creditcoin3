use anyhow::Result;
use sp_core::H256;
use tracing::{debug, info};

use attestation_chain::{
    attestation_fragment::{AttestationFragment, AttestationFragmentSerializable},
    block::Block as FragmentBlock,
};
use attestor_primitives::{AttestorId, ChainId};
use creditcoin3_attestor_gossip::communication::Attestation;
use eth::Client;
use utils::Felt;

use crate::attestation;
use crate::cc3;

pub struct Manager<'a> {
    chain_key: ChainId,
    attestation_interval: u64,
    eth_client: &'a Client,
}

type CreateResult = (AttestationFragmentSerializable, Option<H256>);

impl<'a> Manager<'a> {
    pub fn new(chain_key: ChainId, attestation_interval: u64, eth_client: &'a Client) -> Self {
        Self {
            chain_key,
            attestation_interval,
            eth_client,
        }
    }

    pub async fn async_retry_create(
        &self,
        signed_attestation: &Attestation<H256, AttestorId>,
    ) -> Result<CreateResult, cc3::Error> {
        let fragment: CreateResult = crate::retry::ret(
            || async { self.create(signed_attestation).await },
            10,
            10,
            Some(60),
        )
        .await?;

        Ok(fragment)
    }

    pub async fn create(
        &self,
        signed_attestation: &Attestation<H256, AttestorId>,
    ) -> Result<CreateResult, cc3::Error> {
        let attestation_header_number = signed_attestation.attestation_data.header_number;
        // Only for genesis block we don't need to build a fragment
        if attestation_header_number == 0 {
            info!("No need to build full fragment for genesis block");
            let serializeable_frament =
                AttestationFragmentSerializable::from(&AttestationFragment::new(0));
            return Ok((serializeable_frament, None));
        }

        // Start block is the block number of the attestation header minus the attestation interval
        let start_block = attestation_header_number.saturating_sub(self.attestation_interval);

        // Fragment size is the difference between the attestation header number and the last finalized attestation header number
        // Start and end block are inclusive
        let fragment_size = attestation_header_number.saturating_sub(start_block) + 1;
        let fragment_length = usize::try_from(fragment_size)
            .map_err(|_| cc3::Error::InvalidFragmentLength(fragment_size))?;

        // Create a new fragment with the correct length
        let mut fragment = AttestationFragment::new(fragment_length);

        debug!(
            "Building fragment for interval: {} - {}",
            start_block, attestation_header_number
        );

        // Construct the previous block for the first block in the fragment
        // Because the first block in the fragment needs to reference the previous block's digest
        let previous_block = self
            .eth_client
            .get_block(start_block.saturating_sub(1))
            .await?;
        let attestation = attestation::create(self.chain_key, &previous_block);
        let first_fragment_prev_block = FragmentBlock::new(
            attestation.header_number,
            Felt::from_bytes_be(&attestation.root),
        );

        // Start building the fragment for the interval
        for i in start_block..attestation_header_number {
            let block = self.eth_client.get_block(i).await?;

            let attestation = attestation::create(self.chain_key, &block);
            let fragment_block = FragmentBlock::new(
                attestation.header_number,
                Felt::from_bytes_be(&attestation.root),
            );
            debug!("appending block to fragment: {:?}", fragment_block);

            // If this is the first block in the fragment, we need to construct the block from the previous block
            // In order to set the prev_digest correctly
            // In the other case, the `try_append_block` method on fragment will take care of this if the fragment is not empty
            let fragment_block = if fragment.is_empty() && i != 0 {
                FragmentBlock::try_from_previous(&first_fragment_prev_block, fragment_block)?
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

        Ok((serialized_fragment, prev_digest))
    }
}
