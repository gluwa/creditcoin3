use alloy::transports::{RpcError, TransportErrorKind};
use anyhow::Result;
use thiserror::Error;
use tracing::{debug, error, info};

use attestor_primitives::block::ContinuityProof;
use merkle::TransactionMerkleProof;

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
    #[error("Verification failed")]
    VerificationFailed,
    #[error("Couldn't parse contract address: {0}")]
    AddressParse(#[from] hex::FromHexError),
    #[error("Error: {0}")]
    Other(String),
}

sol! {
    #[sol(rpc)]
    INativeQueryVerifier,
    "contracts/block_prover.json",
}

// Helper function to convert ContinuityProof to Solidity ContinuityProof
fn convert_to_solidity_continuity_proof(
    proof: ContinuityProof,
) -> INativeQueryVerifier::ContinuityProof {
    let continuity_blocks: Vec<INativeQueryVerifier::ContinuityBlock> = proof
        .blocks
        .into_iter()
        .map(|cb| INativeQueryVerifier::ContinuityBlock {
            merkleRoot: FixedBytes::from(cb.merkle_root.to_fixed_bytes()),
            digest: FixedBytes::from(cb.digest.to_fixed_bytes()),
        })
        .collect();

    INativeQueryVerifier::ContinuityProof {
        lowerEndpointDigest: FixedBytes::from(proof.lower_endpoint_digest.to_fixed_bytes()),
        blocks: continuity_blocks,
    }
}

/// Block Prover precompile address (0x0FD2 = 4050)
pub const BLOCK_PROVER_ADDRESS: Address = Address::new([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x0F, 0xD2,
]);

impl From<TransactionMerkleProof> for INativeQueryVerifier::MerkleProof {
    fn from(proof: TransactionMerkleProof) -> Self {
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

/// Block Prover contract interface
#[derive(Debug, Clone)]
pub struct BlockProver {
    pub address: Address,
    client: Client,
}

impl BlockProver {
    /// Create a new BlockProver contract instance at the precompile address
    pub fn new(client: &Client) -> Self {
        let address = BLOCK_PROVER_ADDRESS;
        Self {
            address,
            client: client.clone(),
        }
    }

    /// Verify a blockchain query with Merkle proof and continuity chain (view function)
    ///
    /// # Arguments
    /// * `chain_key` - The chain key identifier
    /// * `height` - The block height to verify
    /// * `encoded_transaction` - Raw transaction data to verify
    /// * `merkle_proof` - Merkle proof for transaction inclusion
    /// * `continuity_proof` - Optimized continuity proof (blocks[0] is at queryHeight-1)
    ///
    /// # Returns
    /// `true` on successful verification (reverts on failure)
    pub async fn verify(
        &self,
        chain_key: u64,
        height: u64,
        encoded_transaction: &[u8],
        merkle_proof: TransactionMerkleProof,
        continuity_proof: ContinuityProof,
    ) -> Result<bool, Error> {
        debug!(
            "Calling native query verifier (view): chain_key={}, height={}",
            chain_key, height
        );

        let sol_proof = merkle_proof.into();
        let sol_proof_struct = convert_to_solidity_continuity_proof(continuity_proof);

        let provider = self.client.get_wallet_ws_provider().await?;
        let contract = INativeQueryVerifier::new(self.address, provider);
        // Use verify_1 for single query (second verify function in ABI)
        let result = contract
            .verify_1(
                chain_key,
                height,
                encoded_transaction.to_vec().into(),
                sol_proof,
                sol_proof_struct,
            )
            .call()
            .await
            .map_err(|e| {
                error!("Native query verifier call failed: {:?}", e);
                Error::AlloyContractError(e)
            })?;

        info!("Query verification successful (view)");

        Ok(result._0)
    }

    /// Verify a blockchain query with Merkle proof and continuity chain (transaction that emits events)
    ///
    /// # Arguments
    /// * `chain_key` - The chain key identifier
    /// * `height` - The block height to verify
    /// * `encoded_transaction` - Raw transaction data to verify
    /// * `merkle_proof` - Merkle proof for transaction inclusion
    /// * `continuity_proof` - Optimized continuity proof (blocks[0] is at queryHeight-1)
    ///
    /// # Returns
    /// `true` on successful verification (reverts on failure)
    ///
    /// # Events
    /// Emits `TransactionVerified(uint64 indexed chain_key, uint64 indexed height, uint64 transactionIndex)` event
    pub async fn verify_and_emit(
        &self,
        chain_key: u64,
        height: u64,
        encoded_transaction: &[u8],
        merkle_proof: TransactionMerkleProof,
        continuity_proof: ContinuityProof,
    ) -> Result<bool, Error> {
        debug!(
            "Sending native query verifier transaction: chain_key={}, height={}",
            chain_key, height
        );

        let sol_proof: INativeQueryVerifier::MerkleProof = merkle_proof.into();
        let sol_proof_struct = convert_to_solidity_continuity_proof(continuity_proof.clone());

        let provider = self.client.get_wallet_ws_provider().await?;
        let contract = INativeQueryVerifier::new(self.address, provider);

        // Send as a transaction to emit events
        // Use verifyAndEmit_0 for single query (first verifyAndEmit function in ABI)
        let tx_builder = contract.verifyAndEmit_0(
            chain_key,
            height,
            encoded_transaction.to_vec().into(),
            sol_proof.clone(),
            sol_proof_struct.clone(),
        );

        let pending_tx = tx_builder.send().await?;
        let receipt = pending_tx.get_receipt().await.map_err(|e| {
            error!("Failed to get transaction receipt: {:?}", e);
            Error::Other(format!("Failed to get transaction receipt: {e}"))
        })?;

        info!(
            "Query verification transaction sent. Hash: {:?}, Gas used: {:?}",
            receipt.transaction_hash, receipt.gas_used
        );

        // Now call to get the result
        let result = contract
            .verifyAndEmit_0(
                chain_key,
                height,
                encoded_transaction.to_vec().into(),
                sol_proof,
                sol_proof_struct,
            )
            .call()
            .await
            .map_err(|e| {
                error!("Native query verifier call failed: {:?}", e);
                Error::AlloyContractError(e)
            })?;

        info!("Query verification successful (with events)");

        Ok(result._0)
    }

    /// Verify a batch of queries with shared continuity proof (view function)
    ///
    /// # Arguments
    /// * `chain_key` - The chain key identifier (same for all queries)
    /// * `heights` - Array of block heights to verify
    /// * `encoded_transactions` - Transaction data for each query
    /// * `merkle_proofs` - Merkle proofs for each query
    /// * `shared_continuity_proof` - Shared continuity proof covering all query heights
    ///
    /// # Returns
    /// `true` if all verifications succeed (reverts on any failure)
    pub async fn verify_batch(
        &self,
        chain_key: u64,
        heights: Vec<u64>,
        encoded_transactions: Vec<Vec<u8>>,
        merkle_proofs: Vec<TransactionMerkleProof>,
        shared_continuity_proof: ContinuityProof,
    ) -> Result<bool, Error> {
        debug!(
            "Calling native query verifier batch (view): chain_key={}, {} queries",
            chain_key,
            heights.len()
        );

        let sol_proofs: Vec<INativeQueryVerifier::MerkleProof> =
            merkle_proofs.into_iter().map(|p| p.into()).collect();
        let sol_proof_struct = convert_to_solidity_continuity_proof(shared_continuity_proof);
        let tx_data_bytes: Vec<alloy::primitives::Bytes> =
            encoded_transactions.into_iter().map(|d| d.into()).collect();

        let provider = self.client.get_wallet_ws_provider().await?;
        let contract = INativeQueryVerifier::new(self.address, provider);
        // Use verify_0 for batch query (first verify function in ABI - has arrays)
        let result = contract
            .verify_0(
                chain_key,
                heights,
                tx_data_bytes,
                sol_proofs,
                sol_proof_struct,
            )
            .call()
            .await
            .map_err(|e| {
                error!("Native query verifier batch call failed: {:?}", e);
                Error::AlloyContractError(e)
            })?;

        info!("Batch query verification successful (view)");

        Ok(result._0)
    }

    /// Verify a batch of queries with shared continuity proof (transaction that emits events)
    ///
    /// # Arguments
    /// * `chain_key` - The chain key identifier (same for all queries)
    /// * `heights` - Array of block heights to verify
    /// * `encoded_transactions` - Transaction data for each query
    /// * `merkle_proofs` - Merkle proofs for each query
    /// * `shared_continuity_proof` - Shared continuity proof covering all query heights
    ///
    /// # Returns
    /// `true` if all verifications succeed (reverts on any failure)
    ///
    /// # Events
    /// Emits `TransactionVerified(uint64 indexed chain_key, uint64 indexed height, uint64 transactionIndex)` event for each successfully verified transaction
    pub async fn verify_batch_and_emit(
        &self,
        chain_key: u64,
        heights: Vec<u64>,
        encoded_transactions: Vec<Vec<u8>>,
        merkle_proofs: Vec<TransactionMerkleProof>,
        shared_continuity_proof: ContinuityProof,
    ) -> Result<bool, Error> {
        debug!(
            "Sending native query verifier batch transaction: chain_key={}, {} queries",
            chain_key,
            heights.len()
        );

        let sol_proofs: Vec<INativeQueryVerifier::MerkleProof> =
            merkle_proofs.into_iter().map(|p| p.into()).collect();
        let sol_proof_struct =
            convert_to_solidity_continuity_proof(shared_continuity_proof.clone());
        let tx_data_bytes: Vec<alloy::primitives::Bytes> = encoded_transactions
            .iter()
            .map(|d| d.clone().into())
            .collect();

        let provider = self.client.get_wallet_ws_provider().await?;
        let contract = INativeQueryVerifier::new(self.address, provider);

        // Send as a transaction to emit events
        // Use verifyAndEmit_1 for batch query (second verifyAndEmit function in ABI - overloaded)
        let tx_builder = contract.verifyAndEmit_1(
            chain_key,
            heights.clone(),
            tx_data_bytes.clone(),
            sol_proofs.clone(),
            sol_proof_struct.clone(),
        );

        let pending_tx = tx_builder.send().await?;
        let receipt = pending_tx.get_receipt().await.map_err(|e| {
            error!("Failed to get transaction receipt: {:?}", e);
            Error::Other(format!("Failed to get transaction receipt: {e}"))
        })?;

        info!(
            "Batch query verification transaction sent. Hash: {:?}, Gas used: {:?}",
            receipt.transaction_hash, receipt.gas_used
        );

        // Now call to get the result
        let result = contract
            .verifyAndEmit_1(
                chain_key,
                heights,
                tx_data_bytes,
                sol_proofs,
                sol_proof_struct,
            )
            .call()
            .await
            .map_err(|e| {
                error!("Native query verifier batch call failed: {:?}", e);
                Error::AlloyContractError(e)
            })?;

        info!("Batch query verification successful (with events)");

        Ok(result._0)
    }

    /// Estimate gas for a query verification
    pub async fn estimate_gas(
        &self,
        chain_key: u64,
        height: u64,
        encoded_transaction: &[u8],
        merkle_proof: TransactionMerkleProof,
        continuity_proof: ContinuityProof,
    ) -> Result<u64, Error> {
        let sol_proof = merkle_proof.into();
        let sol_proof_struct = convert_to_solidity_continuity_proof(continuity_proof);

        let provider = self.client.get_wallet_ws_provider().await?;
        let contract = INativeQueryVerifier::new(self.address, provider);
        // Use verify_1 for single query (second verify function in ABI)
        let gas = contract
            .verify_1(
                chain_key,
                height,
                encoded_transaction.to_vec().into(),
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
    fn test_precompile_address() {
        // Verify the address is 0x0FD2 (4050 in decimal)
        let expected = Address::new([
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x0F, 0xD2,
        ]);
        assert_eq!(BLOCK_PROVER_ADDRESS, expected);
    }
}
