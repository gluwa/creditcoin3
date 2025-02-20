use anyhow::Result;
use tokio::sync::mpsc;
use tracing::info;

use crate::query::QueryId;
use artifacts::ChainDeploymentArtifact;
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
    cost_per_byte: Option<u64>,
    base_fee: Option<u64>,
    chain_key: ChainKey,
) -> Result<()> {
    let chain_id = eth_client.get_chain_id().await.unwrap_or(CC3_CHAIN_ID);

    let artifact = if artifacts::has_artifact(chain_id).await? {
        info!("Found existing deployment artifact, fetching...");
        artifacts::get_deployment_artifact(chain_id).await?
    } else {
        info!("Deploying Gluwa Public Prover contract");
        let contract = eth::evm::prover::deploy(eth_client, None, cost_per_byte, base_fee, chain_key).await?;
        artifacts::create_deployment_artifact(chain_id, contract.clone()).await?;

        ChainDeploymentArtifact { chain_id, contract }
    };

    info!(
        "Creditcoin Public Prover contract address({:?}) on chain {chain_id}",
        artifact.contract.address
    );

    Ok(())
}

// Get unprocessed queries
// This function will fetch all unprocessed queries from the chain
pub async fn get_unprocessed_queries(eth_client: &Client) -> Result<Vec<Query>> {
    let chain_id = eth_client.get_chain_id().await.unwrap_or(CC3_CHAIN_ID);

    let artifact = artifacts::get_deployment_artifact(chain_id).await?;

    let queries = artifact
        .contract
        .get_unprocessed_queries(eth_client)
        .await?;

    Ok(queries)
}

pub async fn submit_proof(eth_client: &Client, query: Query, proof: Vec<u8>) -> Result<String> {
    let chain_id = eth_client.get_chain_id().await.unwrap_or(CC3_CHAIN_ID);
    info!(
        "Submitting proof for query {:?}, chain id {}",
        query.id(),
        chain_id
    );

    // Get the deployment artifact
    let artifact = artifacts::get_deployment_artifact(chain_id).await?;

    // Submit the proof
    let tx_hash = artifact
        .contract
        .submit_query_proof(eth_client, query.id().0.into(), proof)
        .await?;

    info!("Proof submitted tx_hash: {}", tx_hash);

    Ok(tx_hash)
}

pub async fn subscribe_query_submission(
    eth_client: &Client,
    query_channel: mpsc::UnboundedSender<Query>,
) -> Result<()> {
    let chain_id = eth_client.get_chain_id().await?;

    // Get the deployment artifact
    let artifact = artifacts::get_deployment_artifact(chain_id).await?;

    artifact
        .contract
        .subscribe_query_submissions(eth_client, query_channel)
        .await
}

pub async fn remove_query_id(eth_client: &Client, query_id: QueryId) -> Result<String> {
    let chain_id = eth_client.get_chain_id().await.unwrap_or(CC3_CHAIN_ID);

    let artifact = artifacts::get_deployment_artifact(chain_id).await?;

    let tx_hash = artifact
        .contract
        .remove_query_id(eth_client, query_id)
        .await?;

    info!("Query with id {} removed, tx_hash: {}", query_id, tx_hash);

    Ok(tx_hash)
}
