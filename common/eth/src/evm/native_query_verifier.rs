use alloy::transports::{RpcError, TransportErrorKind};
use anyhow::Result;
use thiserror::Error;
use tracing::{debug, error, info};

use attestor_primitives::{
    block::ContinuityProof,
    query::{Query, ResultSegment},
};
use mmr::query_proof::QueryMerkleProof;
use sp_core::H256;
// Removed Felt import - no longer using Felt type

use crate::{Client, Error as ClientError};
use alloy::{
    contract::Error as AlloyContractError,
    primitives::{Address, FixedBytes, U256},
    sol,
};

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
    TransportError(#[from] RpcError<TransportErrorKind>),
    #[error("Verification failed with status: {0}")]
    VerificationFailed(u8),
    #[error("Couldn't parse contract address: {0}")]
    AddressParse(#[from] hex::FromHexError),
    #[error("Error: {0}")]
    Other(String),
}

sol! {
    #[sol(rpc)]
    INativeQueryVerifier,
    "contracts/nativeQueryVerifier.abi.json",
}

// Helper function to convert ContinuityProof to Solidity ContinuityProof
fn convert_to_solidity_continuity_proof(
    proof: ContinuityProof,
) -> INativeQueryVerifier::ContinuityProof {
    let continuity_blocks: Vec<INativeQueryVerifier::ContinuityBlock> = proof
        .blocks
        .into_iter()
        .map(|cb| INativeQueryVerifier::ContinuityBlock {
            root: FixedBytes::from(cb.root.to_fixed_bytes()),
            digest: FixedBytes::from(cb.digest.to_fixed_bytes()),
        })
        .collect();

    INativeQueryVerifier::ContinuityProof {
        lowerEndpointDigest: FixedBytes::from(proof.lower_endpoint_digest.to_fixed_bytes()),
        blocks: continuity_blocks,
    }
}

/// Native Query Verifier precompile address (0x0FD2 = 4050)
pub const NATIVE_QUERY_VERIFIER_ADDRESS: Address = Address::new([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x0F, 0xD2,
]);

/// Convert from Solidity ResultSegment to primitive ResultSegment
fn decode_result_segment(
    segment: INativeQueryVerifier::ResultSegment,
) -> Result<ResultSegment, Error> {
    let offset = segment.offset;
    let bytes = H256::from(segment.data.0);
    Ok(ResultSegment { offset, bytes })
}

impl From<QueryMerkleProof> for INativeQueryVerifier::MerkleProof {
    fn from(proof: QueryMerkleProof) -> Self {
        Self {
            root: FixedBytes::<32>::new(proof.root.0),
            siblings: proof
                .siblings
                .into_iter()
                .map(|entry| INativeQueryVerifier::MerkleProofEntry {
                    hash: FixedBytes::<32>::new(entry.hash.0),
                    isLeft: entry.is_left,
                })
                .collect(),
        }
    }
}

/// Native Query Verifier contract interface
#[derive(Debug, Clone)]
pub struct NativeQueryVerifierContract {
    pub address: Address,
    client: Client,
}

impl NativeQueryVerifierContract {
    /// Create a new NativeQueryVerifier contract instance at the precompile address
    pub fn new(client: &Client) -> Self {
        let address = NATIVE_QUERY_VERIFIER_ADDRESS;
        Self {
            address,
            client: client.clone(),
        }
    }

    /// Convert Query to Solidity Query type
    fn to_solidity_query(query: &Query) -> INativeQueryVerifier::Query {
        INativeQueryVerifier::Query {
            chain_id: query.chain_id,
            height: query.height,
            layout_segments: query
                .layout_segments
                .iter()
                .map(|seg| INativeQueryVerifier::LayoutSegment {
                    offset: seg.offset,
                    size: seg.size,
                })
                .collect(),
        }
    }

    /// Verify a blockchain query with Merkle proof and continuity chain (read-only call)
    ///
    /// # Arguments
    /// * `query` - The query specification
    /// * `tx_data` - Raw transaction data to verify
    /// * `merkle_proof` - Merkle proof for transaction inclusion
    /// * `continuity_proof` - Optimized continuity proof (blocks[0] is at queryHeight-1)
    ///
    /// # Returns
    /// Vector of extracted data segments (reverts on failure)
    pub async fn verify_query(
        &self,
        query: &Query,
        tx_data: &[u8],
        merkle_proof: QueryMerkleProof,
        continuity_proof: ContinuityProof,
    ) -> Result<Vec<ResultSegment>, Error> {
        debug!(
            "Calling native query verifier for query: chain_id={}, height={}",
            query.chain_id, query.height
        );

        let sol_query = Self::to_solidity_query(query);
        let sol_proof = merkle_proof.into();
        let sol_proof_struct = convert_to_solidity_continuity_proof(continuity_proof);

        let provider = self.client.get_wallet_ws_provider().await?;
        let contract = INativeQueryVerifier::new(self.address, provider);
        let result = contract
            .verifyQuery(
                sol_query,
                tx_data.to_vec().into(),
                sol_proof,
                sol_proof_struct,
            )
            .call()
            .await
            .map_err(|e| {
                error!("Native query verifier call failed: {:?}", e);
                Error::AlloyContractError(e)
            })?;

        let result_segments: Result<Vec<ResultSegment>, Error> = result
            .result_segments
            .into_iter()
            .map(decode_result_segment)
            .collect();

        let result_segments = result_segments?;

        info!(
            "Query verification successful. Extracted {} segments",
            result_segments.len()
        );

        Ok(result_segments)
    }

    /// Verify a blockchain query with Merkle proof and continuity chain (transaction that emits events)
    ///
    /// # Arguments
    /// * `query` - The query specification
    /// * `tx_data` - Raw transaction data to verify
    /// * `merkle_proof` - Merkle proof for transaction inclusion
    /// * `continuity_proof` - Optimized continuity proof (blocks[0] is at queryHeight-1)
    ///
    /// # Returns
    /// Vector of extracted data segments (reverts on failure)
    pub async fn verify_query_with_tx(
        &self,
        query: &Query,
        tx_data: &[u8],
        merkle_proof: QueryMerkleProof,
        continuity_proof: ContinuityProof,
    ) -> Result<Vec<ResultSegment>, Error> {
        debug!(
            "Sending native query verifier transaction for query: chain_id={}, height={} id={}",
            query.chain_id,
            query.height,
            query.id()
        );

        let sol_query = Self::to_solidity_query(query);
        let sol_proof: crate::evm::native_query_verifier::INativeQueryVerifier::MerkleProof =
            merkle_proof.into();
        let sol_proof_struct = convert_to_solidity_continuity_proof(continuity_proof.clone());

        let provider = self.client.get_wallet_ws_provider().await?;
        let contract = INativeQueryVerifier::new(self.address, provider);

        // Send as a transaction to emit events
        let tx_builder = contract.verifyQuery(
            sol_query.clone(),
            tx_data.to_vec().into(),
            sol_proof.clone(),
            sol_proof_struct.clone(),
        );

        let pending_tx = tx_builder.send().await?;
        let receipt = pending_tx.get_receipt().await.unwrap();

        info!(
            "Query verification transaction sent. Hash: {:?}, Gas used: {:?}",
            receipt.transaction_hash, receipt.gas_used
        );

        // Now call to get the result
        let result = contract
            .verifyQuery(
                sol_query,
                tx_data.to_vec().into(),
                sol_proof,
                sol_proof_struct,
            )
            .call()
            .await
            .map_err(|e| {
                error!("Native query verifier call failed: {:?}", e);
                Error::AlloyContractError(e)
            })?;

        let result_segments: Result<Vec<ResultSegment>, Error> = result
            .result_segments
            .into_iter()
            .map(decode_result_segment)
            .collect();

        let result_segments = result_segments?;

        info!(
            "Query verification successful. Extracted {} segments",
            result_segments.len()
        );

        Ok(result_segments)
    }

    /// Estimate gas for a query verification
    pub async fn estimate_gas(
        &self,
        query: &Query,
        tx_data: &[u8],
        merkle_proof: QueryMerkleProof,
        continuity_proof: ContinuityProof,
    ) -> Result<u64, Error> {
        let sol_query = Self::to_solidity_query(query);
        let sol_proof = merkle_proof.into();
        let sol_proof_struct = convert_to_solidity_continuity_proof(continuity_proof);

        let provider = self.client.get_wallet_ws_provider().await?;
        let contract = INativeQueryVerifier::new(self.address, provider);
        let gas = contract
            .verifyQuery(
                sol_query,
                tx_data.to_vec().into(),
                sol_proof,
                sol_proof_struct,
            )
            .estimate_gas()
            .await?;

        debug!("Estimated gas for query verification: {}", gas);
        Ok(gas)
    }

    /// Get the precompile address
    pub fn address(&self) -> Address {
        self.address
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verification_status_conversion() {
        assert_eq!(
            VerificationStatus::try_from(0).unwrap(),
            VerificationStatus::Success
        );
        assert_eq!(
            VerificationStatus::try_from(1).unwrap(),
            VerificationStatus::MerkleProofInvalid
        );
        assert_eq!(
            VerificationStatus::try_from(2).unwrap(),
            VerificationStatus::ContinuityChainInvalid
        );
        assert_eq!(
            VerificationStatus::try_from(3).unwrap(),
            VerificationStatus::DataExtractionError
        );
        assert!(VerificationStatus::try_from(4).is_err());
    }

    #[test]
    fn test_precompile_address() {
        // Verify the address is 0x0FD2 (4050 in decimal)
        let expected = Address::new([
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x0F, 0xD2,
        ]);
        assert_eq!(NATIVE_QUERY_VERIFIER_ADDRESS, expected);
    }
}
