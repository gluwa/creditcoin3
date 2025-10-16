use alloy::transports::{RpcError, TransportErrorKind};
use anyhow::Result;
use futures_util::{stream_select, Stream, StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};
use sha3::Digest;
use std::pin::Pin;
use thiserror::Error;
use tokio::sync::mpsc;
use tracing::{debug, error, info};

use pallet_prover_primitives::{LayoutSegment, Query, ResultSegment};
use sp_core::H256;

use crate::evm::prover::CreditcoinPublicProver::{
    QueryMarkedInvalid, QueryProcessingFailed, QueryProofVerified,
};
use crate::{Client, Error as ClientError};
use alloy::providers::PendingTransactionError;
use alloy::{
    contract::Error as AlloyContractError,
    primitives::{Address, FixedBytes, U256},
    providers::Provider,
    sol,
};
use attestor_primitives::ChainKey;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Offset too large to fit into u64. Offset: {0}")]
    OffsetOverflow(U256),
    #[error("abiBytes must be exactly 32 bytes. Bytes len: {0}")]
    AbiBytesNot32(usize),
    #[error(transparent)]
    EthClient(#[from] ClientError),
    #[error(transparent)]
    AlloyContractError(#[from] AlloyContractError),
    #[error(transparent)]
    TransactionSendError(#[from] PendingTransactionError),
    #[error(transparent)]
    TransportError(#[from] RpcError<TransportErrorKind>),
    #[error("Query submission stream ended")]
    QueryStreamEnded,
    #[error("Proof verification event stream ended")]
    VerificationEventStreamEnded,
    #[error("Query marked as invalid. QueryId: {0}, Reason: {1}")]
    QueryMarkedInvalid(FixedBytes<32>, String),
    #[error("Query processing failed. QueryId: {0}, Reason: {1}")]
    QueryProcessingFailed(FixedBytes<32>, String),
    #[error("Stream ended without matching proof verification or failure")]
    VerificationOrFailureStreamEnded,
    #[error("Couldn't parse contract address: {0}")]
    AddressParse(#[from] hex::FromHexError),
    #[error("Query channel error: {0}")]
    QueryChannelError(#[from] mpsc::error::SendError<Query>),
    #[error("Proof channel error: {0}")]
    ProofChannelError(#[from] mpsc::error::SendError<H256>),
    #[error("Error: {0}")]
    Other(String),
}

#[allow(clippy::enum_variant_names)]
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

/// Stream of proof verification events, yielding the query ID (H256) of each verified proof
pub type ProofEventStream = Pin<Box<dyn Stream<Item = Result<H256>> + Send>>;

/// Stream of query submissions, yielding each new Query as it is submitted
pub type QuerySubmissionStream = Pin<Box<dyn Stream<Item = Result<Query>> + Send>>;

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
) -> Result<(GluwaPublicProverContract, H256), Error> {
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
) -> Result<(), Error> {
    let provider = eth_client.get_wallet_ws_provider().await?;
    let prover = CreditcoinPublicProver::new(contract_address, &provider);
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

pub fn new(address: String) -> Result<GluwaPublicProverContract, Error> {
    Ok(GluwaPublicProverContract {
        address: address.parse()?,
        gas_limit: GAS_LIMIT,
    })
}

/// Helper function to decode contract's ResultSegment type into the pallet's ResultSegment type
fn decode_result_segment(
    result_segment: CreditcoinPublicProver::ResultSegment,
) -> Result<ResultSegment, Error> {
    let offset = result_segment
        .offset
        .try_into()
        .map_err(|_| Error::OffsetOverflow(result_segment.offset))?;

    let abi_bytes_vec = result_segment.abiBytes.to_vec();
    if abi_bytes_vec.len() != 32 {
        return Err(Error::AbiBytesNot32(abi_bytes_vec.len()));
    }

    let bytes = H256::from_slice(&abi_bytes_vec);
    Ok(ResultSegment { offset, bytes })
}

impl GluwaPublicProverContract {
    /// Compute the query cost
    pub async fn compute_query_cost(&self, client: &Client, query: Query) -> Result<u64, Error> {
        info!("Computing query cost");
        let provider = client.get_wallet_ws_provider().await?;
        let contract = CreditcoinPublicProver::new(self.address, &provider);

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
    ) -> Result<String, Error> {
        debug!("Submitting query proof for query: {:?}", query_id);
        let provider = client.get_wallet_ws_provider().await?;
        let contract = CreditcoinPublicProver::new(self.address, &provider);

        let tx_request = contract
            .submitQueryProof(query_id, proof.into())
            .into_transaction_request()
            .gas_limit(self.gas_limit)
            .max_fee_per_gas(5_000_000_000u128)
            .max_priority_fee_per_gas(3_000_000_000u128);

        let builder = provider
            .send_transaction(tx_request)
            .await
            .map_err(AlloyContractError::from)?;
        let result = builder.get_receipt().await?.transaction_hash;

        Ok(result.to_string())
    }

    pub async fn subscribe_query_submissions_stream(
        &self,
        client: &Client,
    ) -> Result<QuerySubmissionStream, Error> {
        info!(
            "Subscribing to query submissions for contract with address: {}",
            self.address
        );
        let contract = CreditcoinPublicProver::new(self.address, &client.rpc_provider);
        let sub = contract.QuerySubmitted_filter().subscribe().await?;

        let stream = sub
            .into_stream()
            .map_err(anyhow::Error::from) // E -> anyhow::Error
            .map_ok(|(qs, _log)| Query {
                chain_id: qs.chainQuery.chainId,
                height: qs.chainQuery.height,
                index: qs.chainQuery.index,
                layout_segments: qs
                    .chainQuery
                    .layoutSegments
                    .iter()
                    .map(|l| LayoutSegment {
                        offset: l.offset,
                        size: l.size,
                    })
                    .collect(),
            });

        Ok(Box::pin(stream))
    }

    pub async fn get_query_result(
        &self,
        client: &Client,
        query: Query,
    ) -> Result<Option<Vec<ResultSegment>>, Error> {
        let contract = CreditcoinPublicProver::new(self.address, &client.rpc_provider);

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
                .collect::<Result<Vec<_>, Error>>()?;

            Ok(Some(res))
        }
    }

    pub async fn submit_query(
        &self,
        client: &Client,
        query: Query,
        cost: u64,
    ) -> Result<String, Error> {
        let provider = client.get_wallet_ws_provider().await?;
        let contract = CreditcoinPublicProver::new(self.address, &provider);

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

        let signer = client.get_signer()?;
        let principal = signer.address();
        let builder = contract
            .submitQuery(query, principal)
            .value(U256::from(cost));

        let result = builder.send().await?.get_receipt().await?.transaction_hash;

        Ok(result.to_string())
    }

    pub async fn subscribe_proof_verification_events(
        &self,
        client: &Client,
    ) -> Result<ProofEventStream, Error> {
        let contract = CreditcoinPublicProver::new(self.address, &client.rpc_provider);
        let sub = contract.QueryProofVerified_filter().subscribe().await?;

        let s = sub.into_stream().map(|res| {
            res.map(|(ev, _log)| H256::from_slice(&ev.queryId[..]))
                .map_err(anyhow::Error::from)
        });

        Ok(Box::pin(s))
    }

    pub async fn subscribe_proof_verification(
        &self,
        client: &Client,
        query_id: FixedBytes<32>,
    ) -> Result<Vec<ResultSegment>, Error> {
        debug!(
            "Subscribing to proof verification for query: {:?}",
            query_id
        );
        let contract = CreditcoinPublicProver::new(self.address, &client.rpc_provider);

        let verification_filter = contract.QueryProofVerified_filter().topic1(query_id);
        let query_invalid_filter = contract.QueryMarkedInvalid_filter().topic1(query_id);
        let processing_failed_filter = contract.QueryProcessingFailed_filter().topic1(query_id);

        let stream_verified = verification_filter
            .subscribe()
            .await
            .map_err(AlloyContractError::from)?
            .into_stream()
            .map_ok(|(event, _log)| StreamMessage::FromQueryProofVerified(event));

        let invalid_query_stream = query_invalid_filter
            .subscribe()
            .await
            .map_err(AlloyContractError::from)?
            .into_stream()
            .map_ok(|(event, _log)| StreamMessage::FromQueryMarkedInvalid(event));

        let processing_failed_stream = processing_failed_filter
            .subscribe()
            .await
            .map_err(AlloyContractError::from)?
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
                            .collect::<Result<Vec<_>, Error>>()?;

                        return Ok(segments);
                    }
                }
                Ok(StreamMessage::FromQueryMarkedInvalid(event)) => {
                    if event.queryId == query_id {
                        info!(
                            "Query marked invalid. query: {:?} reason: {}",
                            query_id, event.reason
                        );
                        return Err(Error::QueryMarkedInvalid(query_id, event.reason));
                    }
                }
                Ok(StreamMessage::FromQueryProcessingFailed(event)) => {
                    if event.queryId == query_id {
                        info!(
                            "Query processing failed. query: {:?} reason: {}",
                            query_id, event.reason
                        );
                        return Err(Error::QueryProcessingFailed(query_id, event.reason));
                    }
                }
                Err(e) => {
                    error!("event stream error: {e:?}");
                }
            }
        }

        Err(Error::VerificationOrFailureStreamEnded)
    }

    pub async fn get_unprocessed_queries(&self, client: &Client) -> Result<Vec<Query>, Error> {
        info!("Getting unprocessed queries");
        let contract = CreditcoinPublicProver::new(self.address, &client.rpc_provider);
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
    ) -> Result<String, Error> {
        info!("Setting base cost per bytes: {}", new_cost_per_byte);
        let provider = client.get_wallet_ws_provider().await?;
        let contract = CreditcoinPublicProver::new(self.address, &provider);

        let builder = contract.updateCostPerByte(U256::from(new_cost_per_byte));
        let result = builder
            .send()
            .await?
            .watch()
            .await
            .map_err(AlloyContractError::from)?;

        Ok(result.to_string())
    }

    pub async fn update_base_fee(
        &self,
        client: Client,
        new_base_fee: u64,
    ) -> Result<String, Error> {
        info!("Setting base fee: {}", new_base_fee);
        let provider = client.get_wallet_ws_provider().await?;
        let contract = CreditcoinPublicProver::new(self.address, &provider);

        let builder = contract.updateBaseFee(U256::from(new_base_fee));
        let result = builder
            .send()
            .await?
            .watch()
            .await
            .map_err(AlloyContractError::from)?;

        Ok(result.to_string())
    }

    pub async fn mark_query_as_invalid(
        &self,
        client: &Client,
        query_id: H256,
        reason: String,
    ) -> Result<String, Error> {
        info!("Marking query as invalid: {:?}", query_id);
        let provider = client.get_wallet_ws_provider().await?;
        let contract = CreditcoinPublicProver::new(self.address, &provider);

        let builder = contract.markAsInvalid(query_id.0.into(), reason);
        let result = builder.send().await?.get_receipt().await?;

        Ok(result.transaction_hash.to_string())
    }

    pub async fn mark_query_processing_failed(
        &self,
        client: &Client,
        query_id: H256,
        reason: String,
    ) -> Result<String, Error> {
        info!("Marking query as having failed processing: {:?}", query_id);
        let provider = client.get_wallet_ws_provider().await?;
        let contract = CreditcoinPublicProver::new(self.address, &provider);

        let builder = contract.markProcessingFailed(query_id.0.into(), reason);
        let result = builder.send().await?.get_receipt().await?;

        Ok(result.transaction_hash.to_string())
    }
}
