use alloy::transports::{RpcError, TransportErrorKind};
use anyhow::Result;
use thiserror::Error;
use tracing::{debug, error, info};

use attestor_primitives::{
    block::Block,
    query::{Query, ResultSegment},
};
use sp_core::H256;
// Removed Felt import - no longer using Felt type

use crate::{Client, Error as ClientError};
use alloy::{
    contract::Error as AlloyContractError,
    primitives::{Address, FixedBytes, U256},
    sol,
};
use attestor_primitives::block::Block as AttestorBlock;

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
    NativeQueryVerifier,
    "contracts/native_query_verifier.json",
}

// Helper function to convert attestor Block to Solidity ContinuityBlock
fn convert_to_sol_blocks(blocks: Vec<AttestorBlock>) -> Vec<NativeQueryVerifier::ContinuityBlock> {
    blocks
        .into_iter()
        .map(|block| NativeQueryVerifier::ContinuityBlock {
            block_number: block.block_number,
            root: FixedBytes::from(block.root.to_fixed_bytes()),
            prev_digest: FixedBytes::from(block.prev_digest.to_fixed_bytes()),
            digest: FixedBytes::from(block.digest.to_fixed_bytes()),
        })
        .collect()
}

/// Native Query Verifier precompile address (0x0FD2 = 4050)
pub const NATIVE_QUERY_VERIFIER_ADDRESS: Address = Address::new([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x0F, 0xD2,
]);

/// Verification status codes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationStatus {
    Success = 0,
    MerkleProofInvalid = 1,
    ContinuityChainInvalid = 2,
    DataExtractionError = 3,
}

impl TryFrom<u8> for VerificationStatus {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(VerificationStatus::Success),
            1 => Ok(VerificationStatus::MerkleProofInvalid),
            2 => Ok(VerificationStatus::ContinuityChainInvalid),
            3 => Ok(VerificationStatus::DataExtractionError),
            _ => Err(Error::Other(format!(
                "Unknown verification status: {}",
                value
            ))),
        }
    }
}

impl std::fmt::Display for VerificationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerificationStatus::Success => write!(f, "Success"),
            VerificationStatus::MerkleProofInvalid => write!(f, "Merkle proof invalid"),
            VerificationStatus::ContinuityChainInvalid => write!(f, "Continuity chain invalid"),
            VerificationStatus::DataExtractionError => write!(f, "Data extraction error"),
        }
    }
}

/// Convert from Solidity ResultSegment to primitive ResultSegment
fn decode_result_segment(
    segment: NativeQueryVerifier::ResultSegment,
) -> Result<ResultSegment, Error> {
    let offset = segment.offset;
    let bytes = H256::from(segment.bytes.0);
    Ok(ResultSegment { offset, bytes })
}

/// Merkle proof for transaction inclusion
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MerkleProof {
    pub root: H256,
    pub siblings: Vec<H256>,
}

impl From<MerkleProof> for NativeQueryVerifier::MerkleProof {
    fn from(proof: MerkleProof) -> Self {
        Self {
            root: FixedBytes::<32>::new(proof.root.0),
            siblings: proof
                .siblings
                .into_iter()
                .map(|h| FixedBytes::<32>::new(h.0))
                .collect(),
        }
    }
}

/// Query verification result
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryVerificationResult {
    pub status: VerificationStatus,
    pub result_segments: Vec<ResultSegment>,
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
    fn to_solidity_query(query: &Query) -> NativeQueryVerifier::Query {
        NativeQueryVerifier::Query {
            chain_id: query.chain_id,
            height: query.height,
            index: query.index,
            layout_segments: query
                .layout_segments
                .iter()
                .map(|seg| NativeQueryVerifier::LayoutSegment {
                    offset: seg.offset,
                    size: seg.size,
                })
                .collect(),
        }
    }

    /// Verify a blockchain query with Merkle proof and continuity chain
    ///
    /// # Arguments
    /// * `query` - The query specification
    /// * `tx_data` - Raw transaction data to verify
    /// * `merkle_proof` - Merkle proof for transaction inclusion
    /// * `continuity_blocks` - Chain of block attestations
    ///
    /// # Returns
    /// `QueryVerificationResult` with status and extracted data segments
    pub async fn verify_query(
        &self,
        query: &Query,
        tx_data: &[u8],
        merkle_proof: MerkleProof,
        continuity_blocks: Vec<Block>,
    ) -> Result<QueryVerificationResult, Error> {
        debug!(
            "Calling native query verifier for query: chain_id={}, height={}, index={}",
            query.chain_id, query.height, query.index
        );

        let sol_query = Self::to_solidity_query(query);
        let sol_proof = merkle_proof.into();

        let provider = self.client.get_wallet_ws_provider().await?;
        let contract = NativeQueryVerifier::new(self.address, &provider);
        let result = contract
            .verifyQuery(
                sol_query,
                tx_data.to_vec().into(),
                sol_proof,
                convert_to_sol_blocks(continuity_blocks),
            )
            .call()
            .await
            .map_err(|e| {
                error!("Native query verifier call failed: {:?}", e);
                Error::AlloyContractError(e)
            })?;

        let status = VerificationStatus::try_from(result.result.status)?;

        if status != VerificationStatus::Success {
            error!("Query verification failed with status: {}", status);
            return Err(Error::VerificationFailed(result.result.status));
        }

        let result_segments: Result<Vec<ResultSegment>, Error> = result
            .result
            .result_segments
            .into_iter()
            .map(decode_result_segment)
            .collect();

        let result_segments = result_segments?;

        info!(
            "Query verification successful. Extracted {} segments",
            result_segments.len()
        );

        Ok(QueryVerificationResult {
            status,
            result_segments,
        })
    }

    /// Estimate gas for a query verification
    pub async fn estimate_gas(
        &self,
        query: &Query,
        tx_data: &[u8],
        merkle_proof: MerkleProof,
        continuity_blocks: Vec<Block>,
    ) -> Result<u64, Error> {
        let sol_query = Self::to_solidity_query(query);
        let sol_proof = merkle_proof.into();

        let provider = self.client.get_wallet_ws_provider().await?;
        let contract = NativeQueryVerifier::new(self.address, &provider);
        let gas = contract
            .verifyQuery(
                sol_query,
                tx_data.to_vec().into(),
                sol_proof,
                convert_to_sol_blocks(continuity_blocks),
            )
            .estimate_gas()
            .await?;

        debug!("Estimated gas for query verification: {}", gas);
        Ok(gas as u64)
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
