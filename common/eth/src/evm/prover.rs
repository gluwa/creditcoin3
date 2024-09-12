use anyhow::Result;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::info;

use prover_primitives::{LayoutSegment, Query};

use alloy::{
    network::EthereumWallet,
    primitives::{Address, FixedBytes, U256},
    providers::ProviderBuilder,
    sol,
};

use crate::Client;

sol! {
    #[sol(rpc)]
    CreditcoinPublicProver,
    "contracts/prover.json",
}

pub const GAS_LIMIT: u64 = 50_000_000;

/// Prover contract proof
pub type Proof = Vec<u8>;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash, Default)]
pub struct GluwaPublicProverContract {
    pub address: Address,
    #[allow(dead_code)]
    gas_limit: u64,
}

pub async fn deploy(
    client: &Client,
    proceeds_address: Option<Address>,
) -> Result<GluwaPublicProverContract> {
    let provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(EthereumWallet::from(client.get_signer()?))
        .on_http(client.get_url());

    // If the proceeds address is not provided, use the cc client keypair derived evm address
    let proceeds_address = if let Some(proceeds_address) = proceeds_address {
        proceeds_address
    } else {
        client.get_signer()?.address()
    };

    info!("Deploying Gluwa Public Prover contract");
    let contract = CreditcoinPublicProver::deploy(provider, proceeds_address).await?;

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
            .with_recommended_fillers()
            .wallet(EthereumWallet::from(client.get_signer()?))
            .on_http(client.get_url());

        let contract = CreditcoinPublicProver::new(self.address, provider.clone());

        let query = CreditcoinPublicProver::Query {
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
            .with_recommended_fillers()
            .wallet(EthereumWallet::from(client.get_signer()?))
            .on_http(client.get_url());

        let contract = CreditcoinPublicProver::new(self.address, provider);

        let builder = contract.submitQueryProof(query_id, proof.into());
        let result = builder.send().await?.get_receipt().await?;

        // Log receipt
        info!("Query proof submission receipt: {:?}", result);

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

        let provider = ProviderBuilder::new()
            .with_recommended_fillers()
            .on_http(client.get_url());

        let contract = CreditcoinPublicProver::new(self.address, provider.clone());

        let sub = contract.QuerySubmitted_filter().watch().await?;
        let mut stream = sub.into_stream();

        info!("Subscribed to query submissions");

        loop {
            if let Some(query) = stream.next().await {
                info!("New query submission");
                let (query_submitted, _log) = query?;

                // TODO: check log

                let query = Query {
                    chain_id: query_submitted.query.chainId,
                    height: query_submitted.query.height,
                    index: query_submitted.query.index,
                    layout_segments: query_submitted
                        .query
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
        }
    }

    pub async fn submit_query(&self, client: &Client, query: Query, cost: u64) -> Result<String> {
        let signer = client.get_signer()?;
        let principal = signer.address();

        let provider = ProviderBuilder::new()
            .with_recommended_fillers()
            .wallet(EthereumWallet::from(signer))
            .on_http(client.get_url());

        let contract = CreditcoinPublicProver::new(self.address, provider);

        let query = CreditcoinPublicProver::Query {
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
            .value(U256::from(cost + 1) * U256::from(1e18 as u64));

        let result = builder.send().await?.watch().await?;

        Ok(result.to_string())
    }

    pub async fn subscribe_proof_verification(
        &self,
        client: &Client,
        query_id: FixedBytes<32>,
        channel: mpsc::Sender<Proof>,
    ) -> Result<()> {
        info!(
            "Subscribing to proof verification for query: {:?}",
            query_id
        );

        let provider = ProviderBuilder::new()
            .with_recommended_fillers()
            .on_http(client.get_url());

        let contract = CreditcoinPublicProver::new(self.address, provider.clone());

        let sub = contract.QueryProofVerified_filter().watch().await?;
        let mut stream = sub.into_stream();

        info!("Subscribed to proof verification");

        loop {
            if let Some(proof) = stream.next().await {
                info!("New proof verification");
                let (proof_verified, _log) = proof?;

                if proof_verified.queryId != query_id {
                    continue;
                }

                channel.send(proof_verified.proof.0.into()).await?;
            }
        }
    }
}
