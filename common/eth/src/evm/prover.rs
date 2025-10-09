use anyhow::Result;
use futures_util::{stream_select, StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};
use sha3::Digest;
use tokio::sync::mpsc;
use tracing::{debug, error, info};

use pallet_prover_primitives::{LayoutSegment, Query, ResultSegment};
use sp_core::H256;

use crate::evm::prover::CreditcoinPublicProver::{
    QueryMarkedInvalid, QueryProcessingFailed, QueryProofVerified,
};
use crate::Client;
use alloy::{
    primitives::{Address, FixedBytes, U256},
    providers::Provider,
    sol,
};
use attestor_primitives::ChainKey;

enum StreamMessage {
    FromQueryProofVerified(QueryProofVerified),
    FromQueryMarkedInvalid(QueryMarkedInvalid),
    FromQueryProcessingFailed(QueryProcessingFailed),
}

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
    cost_per_byte: u64,
    base_fee: u64,
    chain_key: ChainKey,
    display_name: String,
    timeout: u64,
) -> Result<(GluwaPublicProverContract, H256)> {
    let provider = client.get_wallet_ws_provider().await?;

    // If the proceeds address is not provided, use the cc client keypair derived evm address
    let proceeds_address = proceeds_address.unwrap_or(client.get_signer()?.address());

    // We compute the bytecode hash here to store it in the artifact
    // This allows us to verify latter if the contracts bytecode has changed
    let bytecode_hash = compute_current_prover_bytecode_hash();

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

    Ok((
        GluwaPublicProverContract {
            address: *contract.address(),
            gas_limit: GAS_LIMIT,
        },
        bytecode_hash,
    ))
}

pub fn compute_current_prover_bytecode_hash() -> H256 {
    let mut hasher = sha3::Sha3_256::new();
    hasher.update(&CreditcoinPublicProver::BYTECODE);
    let result = hasher.finalize();

    // Make sure the hash is exactly 32 bytes
    debug_assert!(result.len() == 32);

    H256::from_slice(result.as_slice())
}

pub async fn check_fees_against_existing(
    eth_client: &Client,
    desired_cost_per_byte: u64,
    desired_base_fee: u64,
    contract_address: Address,
) -> Result<()> {
    let provider = eth_client.get_wallet_ws_provider().await?;
    let prover = CreditcoinPublicProver::new(contract_address, provider.clone());
    let onchain_base_fee = prover.baseFee().call().await?._0;
    let onchain_cost_per_byte_fee = prover.costPerByte().call().await?._0;

    let desired_base_fee = U256::from(desired_base_fee);
    let desired_cost_per_byte = U256::from(desired_cost_per_byte);

    if onchain_base_fee != desired_base_fee {
        info!(
            "🛠️ baseFee mismatch: on-chain={} vs desired={}, updating…",
            onchain_base_fee, desired_base_fee
        );
        let pending = prover.updateBaseFee(desired_base_fee).send().await?;

        let receipt = pending.get_receipt().await?;
        let new_base_fee = prover.baseFee().call().await?._0;
        info!(
            "✅ baseFee updated: {}, tx hash: {}",
            new_base_fee,
            receipt.transaction_hash.to_string()
        );
    } else {
        info!("✅ Existing contract base fee matches desired base fee");
    }

    if onchain_cost_per_byte_fee != desired_cost_per_byte {
        info!(
            "🛠️ costPerByte mismatch: on-chain={} vs desired={}, updating…",
            onchain_cost_per_byte_fee, desired_cost_per_byte
        );

        let pending = prover
            .updateCostPerByte(desired_cost_per_byte)
            .send()
            .await?;

        let receipt = pending.get_receipt().await?;
        let new_cost_per_byte = prover.costPerByte().call().await?._0;
        info!(
            "✅ costPerByte updated: {}, tx hash: {}",
            new_cost_per_byte,
            receipt.transaction_hash.to_string()
        );
    } else {
        info!("✅ Existing contract cost per byte fee matches desired cost per byte fee");
    }

    Ok(())
}

pub fn new(address: String) -> Result<GluwaPublicProverContract> {
    Ok(GluwaPublicProverContract {
        address: address.parse()?,
        gas_limit: GAS_LIMIT,
    })
}

/// Helper function to decode contract's ResultSegment type into the pallet's ResultSegment type
fn decode_result_segment(
    result_segment: CreditcoinPublicProver::ResultSegment,
) -> Result<ResultSegment> {
    let offset = result_segment
        .offset
        .try_into()
        .map_err(|_| anyhow::anyhow!("Offset too large to fit into u64"))?;

    let abi_bytes_vec = result_segment.abiBytes.to_vec();
    if abi_bytes_vec.len() != 32 {
        return Err(anyhow::anyhow!("abiBytes must be exactly 32 bytes"));
    }

    let bytes = H256::from_slice(&abi_bytes_vec);
    Ok(ResultSegment { offset, bytes })
}

impl GluwaPublicProverContract {
    /// Compute the query cost
    pub async fn compute_query_cost(&self, client: &Client, query: Query) -> Result<u64> {
        info!("Computing query cost");

        let provider = client.get_wallet_ws_provider().await?;

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
        debug!("Submitting query proof for query: {:?}", query_id);

        let provider = client.get_wallet_ws_provider().await?;

        let contract = CreditcoinPublicProver::new(self.address, provider.clone());

        let tx_request = contract
            .submitQueryProof(query_id, proof.into())
            .into_transaction_request()
            .gas_limit(self.gas_limit)
            .max_fee_per_gas(5_000_000_000u128)
            .max_priority_fee_per_gas(3_000_000_000u128);

        let builder = provider.send_transaction(tx_request).await?;
        let result = builder.get_receipt().await?.transaction_hash;

        Ok(result.to_string())
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

        let contract = CreditcoinPublicProver::new(self.address, client.rpc_provider.clone());

        let sub = contract.QuerySubmitted_filter().subscribe().await?;
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

    pub async fn get_query_result(
        &self,
        client: &Client,
        query: Query,
    ) -> Result<Option<Vec<ResultSegment>>> {
        let provider = client.get_wallet_ws_provider().await?;
        let contract = CreditcoinPublicProver::new(self.address, provider);

        let chain_query = CreditcoinPublicProver::ChainQuery {
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
                .collect(),
        };

        let result_segments = contract.getQueryResult(chain_query).call().await?;

        if result_segments._0.is_empty() {
            Ok(None)
        } else {
            let res = result_segments
                ._0
                .into_iter()
                .map(decode_result_segment)
                .collect::<Result<Vec<_>>>()?;

            Ok(Some(res))
        }
    }

    pub async fn submit_query(&self, client: &Client, query: Query, cost: u64) -> Result<String> {
        let signer = client.get_signer()?;
        let principal = signer.address();

        let provider = client.get_wallet_ws_provider().await?;

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

        let result = builder.send().await?.get_receipt().await?.transaction_hash;

        Ok(result.to_string())
    }

    pub async fn subscribe_proof_verification_events(
        &self,
        client: &Client,
        proof_channel: mpsc::UnboundedSender<H256>,
    ) -> Result<()> {
        let contract = CreditcoinPublicProver::new(self.address, client.rpc_provider.clone());

        let sub = contract.QueryProofVerified_filter().subscribe().await?;
        let mut stream = sub.into_stream();

        info!("Subscribed to proof verification events");

        while let Some(proof) = stream.next().await {
            let (proof_verified, _log) = proof?;
            let query_id = proof_verified.queryId;

            proof_channel.send(H256::from_slice(&query_id[..]))?;
        }

        Err(anyhow::anyhow!("Proof verification event stream ended"))
    }

    pub async fn subscribe_proof_verification(
        &self,
        client: &Client,
        query_id: FixedBytes<32>,
    ) -> Result<Vec<ResultSegment>> {
        debug!(
            "Subscribing to proof verification for query: {:?}",
            query_id
        );

        let contract = CreditcoinPublicProver::new(self.address, client.rpc_provider.clone());

        let verification_filter = contract.QueryProofVerified_filter().topic1(query_id);

        let query_invalid_filter = contract.QueryMarkedInvalid_filter().topic1(query_id);

        let processing_failed_filter = contract.QueryProcessingFailed_filter().topic1(query_id);

        let stream_verified = verification_filter
            .subscribe()
            .await?
            .into_stream()
            .map_ok(|(event, _log)| StreamMessage::FromQueryProofVerified(event));

        let invalid_query_stream = query_invalid_filter
            .subscribe()
            .await?
            .into_stream()
            .map_ok(|(event, _log)| StreamMessage::FromQueryMarkedInvalid(event));

        let processing_failed_stream = processing_failed_filter
            .subscribe()
            .await?
            .into_stream()
            .map_ok(|(event, _log)| StreamMessage::FromQueryProcessingFailed(event));

        let mut combined = stream_select!(
            stream_verified,
            invalid_query_stream,
            processing_failed_stream
        );

        info!("Subscribed to proof verification");

        while let Some(message) = combined.next().await {
            match message {
                Ok(StreamMessage::FromQueryProofVerified(event)) => {
                    if event.queryId == query_id {
                        let segments = event
                            .resultSegments
                            .into_iter()
                            .map(decode_result_segment)
                            .collect::<Result<Vec<_>>>()?;

                        return Ok(segments);
                    }
                }
                Ok(StreamMessage::FromQueryMarkedInvalid(event)) => {
                    if event.queryId == query_id {
                        info!(
                            "Query marked invalid. query: {:?} reason: {}",
                            query_id, event.reason
                        );
                        return Err(anyhow::anyhow!(
                            "Query marked invalid. {:?} reason: {}",
                            event.queryId,
                            event.reason
                        ));
                    }
                }
                Ok(StreamMessage::FromQueryProcessingFailed(event)) => {
                    if event.queryId == query_id {
                        info!(
                            "Query processing failed. query: {:?} reason: {}",
                            query_id, event.reason
                        );
                        return Err(anyhow::anyhow!(
                            "Query processing failed. {:?} reason: {}",
                            event.queryId,
                            event.reason
                        ));
                    }
                }
                Err(e) => {
                    error!("event stream error: {e:?}");
                }
            }
        }

        Err(anyhow::anyhow!(
            "Stream ended without matching proof verification"
        ))
    }

    pub async fn get_unprocessed_queries(&self, client: &Client) -> Result<Vec<Query>> {
        info!("Getting unprocessed queries");

        let contract = CreditcoinPublicProver::new(self.address, client.rpc_provider.clone());

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

        let provider = client.get_wallet_ws_provider().await?;

        let contract = CreditcoinPublicProver::new(self.address, provider);

        let builder = contract.updateCostPerByte(U256::from(new_cost_per_byte));

        let result = builder.send().await?.watch().await?;

        Ok(result.to_string())
    }

    pub async fn update_base_fee(&self, client: Client, new_base_fee: u64) -> Result<String> {
        info!("Setting base fee: {}", new_base_fee);

        let provider = client.get_wallet_ws_provider().await?;

        let contract = CreditcoinPublicProver::new(self.address, provider);

        let builder = contract.updateBaseFee(U256::from(new_base_fee));

        let result = builder.send().await?.watch().await?;

        Ok(result.to_string())
    }

    pub async fn mark_query_as_invalid(
        &self,
        client: &Client,
        query_id: H256,
        reason: String,
    ) -> Result<String> {
        info!("Marking query as invalid: {:?}", query_id);

        let provider = client.get_wallet_ws_provider().await?;

        let contract = CreditcoinPublicProver::new(self.address, provider);

        let builder = contract.markAsInvalid(query_id.0.into(), reason);

        let result = builder.send().await?.get_receipt().await?;

        Ok(result.transaction_hash.to_string())
    }

    pub async fn mark_query_processing_failed(
        &self,
        client: &Client,
        query_id: H256,
        reason: String,
    ) -> Result<String> {
        info!("Marking query as having failed processing: {:?}", query_id);

        let provider = client.get_wallet_ws_provider().await?;

        let contract = CreditcoinPublicProver::new(self.address, provider);

        let builder = contract.markProcessingFailed(query_id.0.into(), reason);

        let result = builder.send().await?.get_receipt().await?;

        Ok(result.transaction_hash.to_string())
    }
}
