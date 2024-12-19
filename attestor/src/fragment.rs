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

pub async fn create(
    chain_key: ChainId,
    start_block: u64,
    signed_attestation: &mut Attestation<H256, AttestorId>,
    eth_client: &Client,
) -> Result<(), cc3::Error> {
    // Only for genesis block we don't need to build a fragment
    if start_block == 0 {
        info!("No need to build full fragment for genesis block");
        let serializeable_frament =
            AttestationFragmentSerializable::from(&AttestationFragment::new(0));
        signed_attestation.continuity_proof = vec![serializeable_frament];
        return Ok(());
    }

    let attestation_header_number = signed_attestation.attestation_data.header_number;
    // Fragment size is the difference between the attestation header number and the last finalized attestation header number
    // Start and end block are inclusive
    let fragment_size = attestation_header_number.saturating_sub(start_block) + 1;
    let fragment_length = usize::try_from(fragment_size)
        .map_err(|_| cc3::Error::InvalidFragmentLength(fragment_size))?;

    // Create a new fragment with the correct length
    let mut fragment = AttestationFragment::new(fragment_length);

    info!(
        "Building fragment for interval: {} - {}",
        start_block, attestation_header_number
    );

    // Construct the previous block for the first block in the fragment
    // Because the first block in the fragment needs to reference the previous block's digest
    let previous_block = eth_client.get_block(start_block - 1).await?;
    let attestation = attestation::create(chain_key, &previous_block);
    let first_fragment_prev_block = FragmentBlock::new(
        attestation.header_number,
        Felt::from_bytes_be(&attestation.root),
    );

    // Start building the fragment for the interval
    for i in start_block..attestation_header_number {
        let block = eth_client.get_block(i).await?;

        let attestation = attestation::create(chain_key, &block);

        // If this is the last block in the fragment, we need to set this digest as the prev_digest for the signed attestation
        // Because the signed attestation is the next attestation in the fragment
        if i == attestation_header_number - 1 {
            signed_attestation.attestation_data.prev_digest = Some(attestation.digest());
        }

        let fragment_block = FragmentBlock::new(
            attestation.header_number,
            Felt::from_bytes_be(&attestation.root),
        );
        debug!("appending block to fragment: {:?}", fragment_block);

        // If this is the first block in the fragment, we need to construct the block from the previous block
        // In order to set the prev_digest correctly
        // In the other case, the `try_append_block` method on fragment will take care of this if the fragment is not empty
        let fragment_block = if fragment.is_empty() {
            FragmentBlock::try_from_previous(&first_fragment_prev_block, fragment_block)?
        } else {
            fragment_block
        };

        fragment.try_append_block(fragment_block)?;
    }

    // Append the attestation that we just signed to complete the fragment
    let fragment_block = FragmentBlock::new(
        attestation_header_number,
        Felt::from_bytes_be(&signed_attestation.attestation_data.root),
    );
    debug!("appending block to fragment: {:?}", fragment_block);
    fragment.try_append_block(fragment_block)?;

    // Serialize the fragment to be sent over the wire
    let serializeable_frament = AttestationFragmentSerializable::from(&fragment);

    // Add the fragment to the signed attestation
    signed_attestation.continuity_proof = vec![serializeable_frament];

    debug!("Fragment created, ready for submission");

    Ok(())
}
