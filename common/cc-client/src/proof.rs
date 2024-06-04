use anyhow::Result;
use sp_core::{H256, U256};
use tracing::{debug, info};

use alloy::{
    primitives::{address, Address},
    providers::{Provider, ProviderBuilder},
    sol,
};

use crate::{cc3, ChainPriceConfig, Client};

sol! {
    #[sol(rpc)]
    #[sol(bytecode = "0x1234")] // Replace with actual bytecode
    contract ClaimContract {
        // No arguments in the constructor
        constructor() {}

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

        // Event emitted when a claim is submitted
        #[derive(Default, PartialEq, Debug)]
        event ClaimSubmitted(
            bytes32 claim_hash,
        );

        #[derive(Debug)]
        function register_prover(string memory nickname) public;

        struct ChainPriceConfig {
            uint64 chain_id;
            uint64 price;
        }

        function set_chain_price_config(ChainPriceConfig[] memory chain_price_configs) public;

        // Function to submit proof for a claim
        #[derive(Debug)]
        function submit_proof(bytes32 claim_hash, uint8[] memory proof) external;

        // Event emitted when a proof is submitted
        #[derive(Default, PartialEq, Debug)]
        event ProofSubmitted(
            bytes32 claim_hash,
        );
    }
}

pub const PRECOMPILE_ADDR: Address = address!("0000000000000000000000000000000000000be9");
pub const GAS_LIMIT: u64 = 50_000_000;

impl Client {
    /// Register the prover with the given nickname
    /// - `nickname`: nickname of the prover
    pub async fn register(&self, nickname: String) -> Result<()> {
        info!("Registering prover with nickname: {}", nickname);

        let provider = ProviderBuilder::new()
            .with_recommended_fillers()
            .on_builtin(&self.url)
            .await?;

        let contract = ClaimContract::new(PRECOMPILE_ADDR, provider.clone());

        let call_builder = contract.register_prover(nickname);

        let call_data = call_builder.calldata();

        info!("Register prover call data: {:?}", call_data);

        let gas_price = provider.get_gas_price().await?;
        info!("Gas price: {gas_price}");

        self.submit_call(call_data.0.to_vec(), gas_price).await?;

        Ok(())
    }

    /// Set the chain price configurations
    /// - `chain_price_configs`: chain price configurations (`chain_id`, price)
    pub async fn set_chain_price_config(
        &self,
        chain_price_configs: Vec<ChainPriceConfig>,
    ) -> Result<()> {
        info!(
            "Setting chain price configurations: {:?}",
            chain_price_configs
        );

        let provider = ProviderBuilder::new()
            .with_recommended_fillers()
            .on_builtin(&self.url)
            .await?;

        let contract = ClaimContract::new(PRECOMPILE_ADDR, provider.clone());

        // Convert ChainPriceConfig to ClaimContract::ChainPriceConfig
        let chain_price_configs: Vec<_> = chain_price_configs
            .into_iter()
            .map(|config| ClaimContract::ChainPriceConfig {
                chain_id: config.chain_id,
                price: config.price,
            })
            .collect();

        let call_builder = contract
            .set_chain_price_config(chain_price_configs)
            .from(self.evm_address);

        let call_data = call_builder.calldata();

        info!("Set chain price config call data: {:?}", call_data);

        let gas_price = provider.get_gas_price().await?;
        info!("Gas price: {gas_price}");

        self.submit_call(call_data.0.to_vec(), gas_price).await?;

        Ok(())
    }

    /// Submit proof for a claim
    /// - `claim_hash`: hash of the claim
    /// - `proof`: proof data
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

        let call_builder = contract
            .submit_proof(claim_hash.0.into(), proof)
            .from(self.evm_address);

        let call_data = call_builder.calldata();
        info!("Proof submission call data: {:?}", call_data);

        let gas_price = provider.get_gas_price().await?;
        info!("Gas price: {gas_price}");

        self.submit_call(call_data.0.to_vec(), gas_price).await?;

        Ok(())
    }

    // Submit a call to the precompile contract
    // - `call_data`: call data
    // - `gas_price`: gas price
    async fn submit_call(&self, call_data: Vec<u8>, gas_price: u128) -> Result<()> {
        info!("Submitting call with data len: {}", call_data.len());

        let from = sp_core::H160(self.evm_address.into_array());
        info!("Submitting call from address:{:?} ", from);

        let tx = cc3::tx().evm().call(
            from,
            sp_core::H160(PRECOMPILE_ADDR.into_array()),
            call_data,
            subxt::utils::Static(sp_core::U256::zero()),
            GAS_LIMIT,
            subxt::utils::Static(U256::from(gas_price)),
            None,
            None,
            vec![],
        );

        let api = self.get_substrate_client().await?;
        let ext = api
            .tx()
            .sign_and_submit_then_watch_default(&tx, &self.keypair)
            .await?
            .wait_for_finalized_success()
            .await?;

        let hash = ext.extrinsic_hash();
        debug!("Call extrinsic submitted with hash: {:?}", hash);

        Ok(())
    }
}
