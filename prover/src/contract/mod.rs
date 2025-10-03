use anyhow::Result;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::query::QueryId;
use attestor_primitives::ChainKey;
use eth::Client;
use pallet_prover_primitives::Query;

pub mod artifacts;

const CC3_CHAIN_ID: u64 = 42;

// Deploy the contract
// This function will deploy the contract to the chain
// If the contract is already deployed, it will fetch the artifact
pub async fn deploy(
    eth_client: &Client,
    cost_per_byte: u64,
    base_fee: u64,
    chain_key: ChainKey,
    display_name: String,
    timeout: u64,
) -> Result<()> {
    let chain_id = eth_client.get_chain_id().await.unwrap_or(CC3_CHAIN_ID);

    let artifact = if artifacts::has_artifact(chain_id).await? {
        info!("🔍 Found existing deployment artifact, fetching...");
        let artifact = artifacts::get_latest_deployment_artifact_for(chain_id).await?;

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
            warn!("⚠️  Existing artifact does not have a bytecode hash stored. Skipping bytecode verification...");
        }

        eth::evm::prover::check_fees_against_existing(
            eth_client,
            cost_per_byte,
            base_fee,
            artifact.contract.address,
        )
        .await?;

        artifact
    } else {
        info!("🚀 Deploying Gluwa Public Prover contract");
        let (contract, bytecode_hash) = eth::evm::prover::deploy(
            eth_client,
            None,
            cost_per_byte,
            base_fee,
            chain_key,
            display_name,
            timeout,
        )
        .await?;

        artifacts::create_deployment_artifact(chain_id, contract, bytecode_hash).await?
    };

    info!(
        "📜 Creditcoin Public Prover contract address({:?}) on chain {chain_id}",
        artifact.contract.address
    );

    Ok(())
}

// Get unprocessed queries
// This function will fetch all unprocessed queries from the chain
pub async fn get_initial_unprocessed_queries(eth_client: &Client) -> Result<Vec<Query>> {
    let chain_id = eth_client.get_chain_id().await.unwrap_or(CC3_CHAIN_ID);

    let artifact = artifacts::get_latest_deployment_artifact_for(chain_id).await?;

    let queries = artifact
        .contract
        .get_unprocessed_queries(eth_client)
        .await?;

    Ok(queries)
}

/// Submit proof by `QueryId` directly
pub async fn submit_proof_by_id(
    eth_client: &Client,
    query_id: QueryId,
    proof: Vec<u8>,
) -> Result<String> {
    let chain_id = eth_client.get_chain_id().await.unwrap_or(CC3_CHAIN_ID);
    debug!(
        "📝 Submitting proof for query {:?}, chain id {}",
        query_id, chain_id
    );

    let artifact = artifacts::get_latest_deployment_artifact_for(chain_id).await?;
    let tx_hash = artifact
        .contract
        .submit_query_proof(eth_client, query_id.0.into(), proof)
        .await?;

    info!(
        "✅ Proof submitted successfully for query: {:?}, tx_hash: {}",
        query_id, tx_hash
    );
    Ok(tx_hash.to_string())
}

pub async fn subscribe_proof_verification_events(
    eth_client: &Client,
    proof_channel: mpsc::UnboundedSender<QueryId>,
) -> Result<()> {
    let chain_id = eth_client.get_chain_id().await.unwrap_or(CC3_CHAIN_ID);

    let artifact = artifacts::get_latest_deployment_artifact_for(chain_id).await?;

    artifact
        .contract
        .subscribe_proof_verification_events(eth_client, proof_channel)
        .await?;

    info!(
        "✅ Subscribed to proof verification events on chain {}",
        chain_id
    );
    Ok(())
}

pub async fn subscribe_query_submissions(
    eth_client: &Client,
    query_channel: mpsc::UnboundedSender<Query>,
) -> Result<()> {
    let chain_id = eth_client.get_chain_id().await.unwrap_or(CC3_CHAIN_ID);

    let artifact = artifacts::get_latest_deployment_artifact_for(chain_id).await?;

    artifact
        .contract
        .subscribe_query_submissions(eth_client, query_channel)
        .await?;

    info!("✅ Subscribed to query submissions on chain {}", chain_id);
    Ok(())
}

pub async fn mark_query_as_invalid(
    eth_client: &Client,
    query_id: QueryId,
    reason: String,
) -> Result<String> {
    let chain_id = eth_client.get_chain_id().await.unwrap_or(CC3_CHAIN_ID);

    let artifact = artifacts::get_latest_deployment_artifact_for(chain_id).await?;

    let tx_hash = artifact
        .contract
        .mark_query_as_invalid(eth_client, query_id, reason)
        .await?;

    info!(
        "📝 Query with id {} marked as invalid, tx_hash: {}",
        query_id, tx_hash
    );

    Ok(tx_hash)
}
