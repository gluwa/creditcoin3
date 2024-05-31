use anyhow::Result;
use sp_core::{H256, U256};
use tracing::{debug, info};

use alloy::{
    primitives::{address, Address},
    providers::{Provider, ProviderBuilder},
    sol,
};

use crate::{cc3, Client};

sol! {
    #[sol(rpc)]
    #[sol(bytecode = "0x1234")] // Replace with actual bytecode
    contract ClaimContract {
        // No arguments in the constructor
        constructor() {}

        // Event emitted when a claim is submitted
        #[derive(Default, PartialEq, Debug)]
        event ClaimSubmitted(
            bytes32 claim_hash,
        );

        // Function to submit a claim
        #[derive(Debug)]
        function submit_claim(
            uint64 chain_id,
            uint64 block_number,
            uint8 tx_index,
            address from,
            address to,
            bool is_tx,
            bool is_rx,
        ) public;

        // Event emitted when a proof is submitted
        #[derive(Default, PartialEq, Debug)]
        event ProofSubmitted(
            bytes32 claim_hash,
        );

        // Function to submit proof for a claim
        #[derive(Debug)]
        function submit_proof(bytes32 claim_hash, bytes memory proof) public;
    }
}

pub const PRECOMPILE_ADDR: Address = address!("0000000000000000000000000000000000003049");
pub const GAS_LIMIT: u64 = 5_000_000;

impl Client {
    pub async fn submit_proof(&self, claim_hash: H256, proof: Vec<u8>) -> Result<()> {
        info!(
            "Submitting proof for claim: {}, proof len: {}",
            claim_hash,
            proof.len()
        );

        let provider = ProviderBuilder::new()
            .with_recommended_fillers()
            .on_builtin(&self.url)
            .await?;

        let contract = ClaimContract::new(PRECOMPILE_ADDR, provider.clone());

        let call_builder = contract.submit_proof(claim_hash.0.into(), proof.into());

        let call_data = call_builder.calldata();

        let api = self.get_substrate_client().await?;

        let gas_price = provider.get_gas_price().await?;
        info!("Gas price: {gas_price}");

        let tx = cc3::tx().evm().call(
            sp_core::H160(self.evm_address.into_array()),
            sp_core::H160(PRECOMPILE_ADDR.into_array()),
            call_data.0.to_vec(),
            subxt::utils::Static(sp_core::U256::zero()),
            GAS_LIMIT,
            subxt::utils::Static(U256::from(gas_price)),
            None,
            None,
            vec![],
        );

        let ext = api
            .tx()
            .sign_and_submit_then_watch_default(&tx, &self.keypair)
            .await?
            .wait_for_finalized_success()
            .await?;

        let hash = ext.extrinsic_hash();
        debug!("Proof submission extrinsic submitted with hash: {:?}", hash);

        Ok(())
    }
}
