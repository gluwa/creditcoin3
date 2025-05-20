use anyhow::Result;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::info;

use pallet_prover_primitives::{LayoutSegment, Query};
use sp_core::H256;

use crate::Client;
use alloy::{
    network::EthereumWallet,
    primitives::{Address, FixedBytes, U256},
    providers::{Provider, ProviderBuilder},
    sol,
};
use attestor_primitives::ChainKey;

sol! {
    #[sol(rpc)]
    CreditcoinPublicProver,
    "contracts/prover.json",
}

pub const GAS_LIMIT: u64 = 50_000_000;

/// Prover contract proof
pub type Proof = Vec<u8>;

/// Result segment
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
pub struct ResultSegment {
    pub offset: U256,
    pub abi_bytes: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash, Default)]
pub struct GluwaPublicProverContract {
    pub address: Address,
    #[allow(dead_code)]
    gas_limit: u64,
}

pub async fn deploy(
    client: &Client,
    proceeds_address: Option<Address>,
    cost_per_byte: u64,
    base_fee: u64,
    chain_key: ChainKey,
    display_name: String,
    timeout: u64,
) -> Result<GluwaPublicProverContract> {
    let provider = ProviderBuilder::new()
        .wallet(EthereumWallet::from(client.get_signer()?))
        .on_http(client.get_url());

    // If the proceeds address is not provided, use the cc client keypair derived evm address
    let proceeds_address = proceeds_address.unwrap_or(client.get_signer()?.address());

    info!("Deploying Gluwa Public Prover contract");
    let contract = CreditcoinPublicProver::deploy(
        provider,
        proceeds_address,
        U256::from(cost_per_byte),
        U256::from(base_fee),
        chain_key,
        display_name,
        timeout,
    )
    .await?;

    info!(
        "Gluwa Public Prover contract deploy at {}",
        contract.address()
    );

    Ok(GluwaPublicProverContract {
        address: *contract.address(),
        gas_limit: GAS_LIMIT,
    })
}

pub fn new(address: String) -> Result<GluwaPublicProverContract> {
    Ok(GluwaPublicProverContract {
        address: address.parse()?,
        gas_limit: GAS_LIMIT,
    })
}

impl GluwaPublicProverContract {
    /// Compute the query cost
    pub async fn compute_query_cost(&self, client: &Client, query: Query) -> Result<u64> {
        info!("Computing query cost");

        let provider = ProviderBuilder::new()
            .wallet(EthereumWallet::from(client.get_signer()?))
            .on_http(client.get_url());

        let contract = CreditcoinPublicProver::new(self.address, provider.clone());

        let query = CreditcoinPublicProver::ChainQuery {
            chainId: query.chain_id,
            height: query.height,
            index: query.index,
            layoutSegments: query
                .layout_segments
                .iter()
                .map(|l| CreditcoinPublicProver::LayoutSegment {
                    offset: l.offset,
                    size: l.size,
                })
                .collect::<Vec<_>>(),
        };

        // probably here we can pass another argument like distance to nearest
        // checkpoint to be included in the cost calculations
        // TODO: add distance to nearest checkpoint to the query
        let builder = contract.computeQueryCost(query);
        let cost = builder.call().await?._0;

        let num: u64 = cost.to::<u64>();

        Ok(num)
    }

    /// Submit query proof
    pub async fn submit_query_proof(
        &self,
        client: &Client,
        query_id: FixedBytes<32>,
        proof: Proof,
    ) -> Result<String> {
        info!("Submitting query proof for query: {:?}", query_id);

        let provider = ProviderBuilder::new()
            .wallet(EthereumWallet::from(client.get_signer()?))
            .on_http(client.get_url());

        let contract = CreditcoinPublicProver::new(self.address, provider.clone());

        let tx_request = contract
            .submitQueryProof(query_id, proof.into())
            .into_transaction_request()
            .gas_limit(self.gas_limit)
            .max_fee_per_gas(5_000_000_000u128)
            .max_priority_fee_per_gas(3_000_000_000u128);

        let result = provider
            .send_transaction(tx_request)
            .await?
            .get_receipt()
            .await?;

        Ok(result.transaction_hash.to_string())
    }

    pub async fn subscribe_query_submissions(
        &self,
        client: &Client,
        query_channel: mpsc::UnboundedSender<Query>,
    ) -> Result<()> {
        info!(
            "Subscribing to query submissions for contract with address: {}",
            self.address
        );

        let provider = ProviderBuilder::new().on_http(client.get_url());

        let contract = CreditcoinPublicProver::new(self.address, provider.clone());

        let sub = contract.QuerySubmitted_filter().watch().await?;
        let mut stream = sub.into_stream();

        info!("Subscribed to query submissions");

        while let Some(query) = stream.next().await {
            info!("New query submission");
            let (query_submitted, _log) = query?;

            // TODO: check log

            let query = Query {
                chain_id: query_submitted.chainQuery.chainId,
                height: query_submitted.chainQuery.height,
                index: query_submitted.chainQuery.index,
                layout_segments: query_submitted
                    .chainQuery
                    .layoutSegments
                    .iter()
                    .map(|l| LayoutSegment {
                        offset: l.offset,
                        size: l.size,
                    })
                    .collect::<Vec<_>>(),
            };

            query_channel.send(query)?;
        }

        Err(anyhow::anyhow!("Query submission stream ended"))
    }

    pub async fn submit_query(&self, client: &Client, query: Query, cost: u64) -> Result<String> {
        let signer = client.get_signer()?;
        let principal = signer.address();

        let provider = ProviderBuilder::new()
            .wallet(EthereumWallet::from(signer))
            .on_http(client.get_url());

        let contract = CreditcoinPublicProver::new(self.address, provider);

        let query = CreditcoinPublicProver::ChainQuery {
            chainId: query.chain_id,
            height: query.height,
            index: query.index,
            layoutSegments: query
                .layout_segments
                .iter()
                .map(|l| CreditcoinPublicProver::LayoutSegment {
                    offset: l.offset,
                    size: l.size,
                })
                .collect::<Vec<_>>(),
        };

        let builder = contract
            .submitQuery(query, principal)
            .value(U256::from(cost));

        let result = builder.send().await?.watch().await?;

        Ok(result.to_string())
    }

    pub async fn subscribe_proof_verification(
        &self,
        client: &Client,
        query_id: FixedBytes<32>,
    ) -> Result<Vec<ResultSegment>> {
        info!(
            "Subscribing to proof verification for query: {:?}",
            query_id
        );

        let provider = ProviderBuilder::new().on_http(client.get_url());

        let contract = CreditcoinPublicProver::new(self.address, provider.clone());

        let sub = contract.QueryProofVerified_filter().watch().await?;
        let mut stream = sub.into_stream();

        info!("Subscribed to proof verification");

        while let Some(proof) = stream.next().await {
            let (proof_verified, _log) = proof?;

            if proof_verified.queryId == query_id {
                return Ok(proof_verified
                    .resultSegments
                    .into_iter()
                    .map(|r| ResultSegment {
                        offset: r.offset,
                        abi_bytes: r.abiBytes.into(),
                    })
                    .collect());
            }
        }

        Err(anyhow::anyhow!(
            "Stream ended without matching proof verification"
        ))
    }

    pub async fn get_unprocessed_queries(&self, client: &Client) -> Result<Vec<Query>> {
        info!("Getting unprocessed queries");

        let provider = ProviderBuilder::new().on_http(client.get_url());

        let contract = CreditcoinPublicProver::new(self.address, provider);

        let unprocessed = contract.getUnprocessedQueries().call().await?;

        Ok(unprocessed
            ._0
            .into_iter()
            .map(|q| Query {
                chain_id: q.chainId,
                height: q.height,
                index: q.index,
                layout_segments: q
                    .layoutSegments
                    .iter()
                    .map(|l| LayoutSegment {
                        offset: l.offset,
                        size: l.size,
                    })
                    .collect(),
            })
            .collect())
    }

    pub async fn update_base_cost_per_bytes(
        &self,
        client: Client,
        new_cost_per_byte: u64,
    ) -> Result<String> {
        info!("Setting base cost per bytes: {}", new_cost_per_byte);

        let signer = client.get_signer()?;

        let provider = ProviderBuilder::new()
            .wallet(EthereumWallet::from(signer))
            .on_http(client.get_url());

        let contract = CreditcoinPublicProver::new(self.address, provider);

        let builder = contract.updateCostPerByte(U256::from(new_cost_per_byte));

        let result = builder.send().await?.watch().await?;

        Ok(result.to_string())
    }

    pub async fn update_base_fee(&self, client: Client, new_base_fee: u64) -> Result<String> {
        info!("Setting base fee: {}", new_base_fee);

        let signer = client.get_signer()?;

        let provider = ProviderBuilder::new()
            .wallet(EthereumWallet::from(signer))
            .on_http(client.get_url());

        let contract = CreditcoinPublicProver::new(self.address, provider);

        let builder = contract.updateBaseFee(U256::from(new_base_fee));

        let result = builder.send().await?.watch().await?;

        Ok(result.to_string())
    }

    pub async fn remove_query_id(&self, client: &Client, query_id: H256) -> Result<String> {
        info!("Removing query id: {:?}", query_id);
        let signer = client.get_signer()?;

        let provider = ProviderBuilder::new()
            .wallet(EthereumWallet::from(signer))
            .on_http(client.get_url());

        let contract = CreditcoinPublicProver::new(self.address, provider);

        let builder = contract.removeQueryId(query_id.0.into());

        let result = builder.send().await?.get_receipt().await?;

        Ok(result.transaction_hash.to_string())
    }
}
