use anyhow::Result;
use std::path::PathBuf;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::query::QueryId;
use attestor_primitives::ChainKey;
use eth::Client;
use pallet_prover_primitives::Query;

pub mod artifacts;

const CC3_CHAIN_ID: u64 = 42;

/// Client wrapper around the prover contract interactions, backed by an ArtifactStore.
/// This struct encapsulates the Ethereum client and where artifacts are stored,
/// keeping async methods and reducing the need to pass parameters everywhere.
#[derive(Clone)]
pub struct ProverContractClient {
    client: Client,
    store: artifacts::ArtifactStore,
}

impl ProverContractClient {
    /// Create a client with a custom artifact store path. If `path` is None, defaults to the built-in artifact path.
    pub fn new<P: Into<PathBuf>>(client: Client, path: Option<P>) -> Self {
        let store = match path {
            Some(p) => artifacts::ArtifactStore::new(p.into()),
            None => artifacts::ArtifactStore::new_default(),
        };
        Self { client, store }
    }

    pub async fn deploy(
        &self,
        cost_per_byte: u64,
        base_fee: u64,
        chain_key: ChainKey,
        display_name: String,
        timeout: u64,
    ) -> Result<()> {
        let chain_id = self.client.get_chain_id().await.unwrap_or(CC3_CHAIN_ID);

        let artifact = if self.store.has_artifact(chain_id).await? {
            info!("🔍 Found existing deployment artifact, fetching...");
            let artifact = self
                .store
                .get_latest_deployment_artifact_for(chain_id)
                .await?;

            if let Some(artifact_hash) = artifact.bytecode_hash {
                info!("🔑 Artifact bytecode hash: {:?}", artifact_hash);

                let current_hash = eth::evm::prover::compute_current_prover_bytecode_hash();
                info!("🔑 Compiled bytecode hash: {:?}", current_hash);

                if artifact_hash != current_hash {
                    error!(
                        "❌ The artifact's bytecode does not match the compiled contract's bytecode!"
                    );

                    anyhow::bail!("Contract bytecode mismatch, cannot continue.");
                }

                info!("✅ Bytecode verification passed.");
            } else {
                warn!(
                    "⚠️  Existing artifact does not have a bytecode hash stored. Skipping bytecode verification..."
                );
            }

            eth::evm::prover::check_fees_against_existing(
                &self.client,
                cost_per_byte,
                base_fee,
                artifact.contract.address,
            )
            .await?;

            artifact
        } else {
            info!("🚀 Deploying Gluwa Public Prover contract");
            let (contract, bytecode_hash) = eth::evm::prover::deploy(
                &self.client,
                None,
                cost_per_byte,
                base_fee,
                chain_key,
                display_name,
                timeout,
            )
            .await?;

            self.store
                .create_deployment_artifact(chain_id, contract, bytecode_hash)
                .await?
        };

        info!(
            "📜 Creditcoin Public Prover contract address({:?}) on chain {chain_id}",
            artifact.contract.address
        );

        Ok(())
    }

    /// Fetches all unprocessed queries from the contract.
    pub async fn get_initial_unprocessed_queries(&self) -> Result<Vec<Query>> {
        let chain_id = self.client.get_chain_id().await.unwrap_or(CC3_CHAIN_ID);

        let artifact = self
            .store
            .get_latest_deployment_artifact_for(chain_id)
            .await?;

        let queries = artifact
            .contract
            .get_unprocessed_queries(&self.client)
            .await?;

        Ok(queries)
    }

    /// Submit proof by `QueryId` directly
    pub async fn submit_proof_by_id(&self, query_id: QueryId, proof: Vec<u8>) -> Result<String> {
        let chain_id = self.client.get_chain_id().await.unwrap_or(CC3_CHAIN_ID);
        debug!(
            "📝 Submitting proof for query {:?}, chain id {}",
            query_id, chain_id
        );

        let artifact = self
            .store
            .get_latest_deployment_artifact_for(chain_id)
            .await?;

        // Submit the proof
        let tx_hash = artifact
            .contract
            .submit_query_proof(&self.client, query_id.0.into(), proof)
            .await?;

        info!(
            "✅ Proof submitted successfully for query: {:?}, tx_hash: {}",
            query_id, tx_hash
        );
        Ok(tx_hash.to_string())
    }

    /// Subscribes to proof verification events and forwards query ids to the provided channel.
    pub async fn subscribe_proof_verification_events(
        &self,
        proof_channel: mpsc::UnboundedSender<QueryId>,
    ) -> Result<()> {
        let chain_id = self.client.get_chain_id().await.unwrap_or(CC3_CHAIN_ID);

        let artifact = self
            .store
            .get_latest_deployment_artifact_for(chain_id)
            .await?;

        artifact
            .contract
            .subscribe_proof_verification_events(&self.client, proof_channel)
            .await?;

        info!(
            "✅ Subscribed to proof verification events on chain {}",
            chain_id
        );
        Ok(())
    }

    /// Subscribes to query submissions and forwards them to the provided channel.
    pub async fn subscribe_query_submissions(
        &self,
        query_channel: mpsc::UnboundedSender<Query>,
    ) -> Result<()> {
        let chain_id = self.client.get_chain_id().await.unwrap_or(CC3_CHAIN_ID);

        let artifact = self
            .store
            .get_latest_deployment_artifact_for(chain_id)
            .await?;

        artifact
            .contract
            .subscribe_query_submissions(&self.client, query_channel)
            .await?;

        info!("✅ Subscribed to query submissions on chain {}", chain_id);
        Ok(())
    }

    /// Marks a query as invalid on chain with the provided reason.
    pub async fn mark_query_as_invalid(&self, query_id: QueryId, reason: String) -> Result<String> {
        let chain_id = self.client.get_chain_id().await.unwrap_or(CC3_CHAIN_ID);

        let artifact = self
            .store
            .get_latest_deployment_artifact_for(chain_id)
            .await?;

        let tx_hash = artifact
            .contract
            .mark_query_as_invalid(&self.client, query_id, reason)
            .await?;

        info!(
            "📝 Query with id {} marked as invalid, tx_hash: {}",
            query_id, tx_hash
        );

        Ok(tx_hash)
    }
}

pub async fn mark_query_processing_failed(
    eth_client: &Client,
    query_id: QueryId,
    reason: String,
    artifacts_path: &str,
) -> Result<String> {
    let chain_id = eth_client.get_chain_id().await.unwrap_or(CC3_CHAIN_ID);

    let artifact = artifacts::get_latest_deployment_artifact_for(chain_id, artifacts_path).await?;

    let tx_hash = artifact
        .contract
        .mark_query_processing_failed(eth_client, query_id, reason)
        .await?;

    info!(
        "📝 Query with id {} marked as having failed processing, tx_hash: {}",
        query_id, tx_hash
    );

    Ok(tx_hash)
}
