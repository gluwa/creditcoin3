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
    providers::Provider,
    rpc::types::{eth::BlockId, TransactionReceipt},
    sol,
    sol_types::SolEvent,
};

/// Gas every transaction pays before execution.
const TX_BASE_GAS: u64 = 21_000;
/// Allowance per continuity root for verifying the chain of roots.
const GAS_PER_CONTINUITY_ROOT: u64 = 5_000;
/// Flat allowance for the Merkle verification and the call overhead.
const GAS_PROOF_OVERHEAD: u64 = 20_000;
/// `TransactionVerified` is a `LOG3` (signature + two indexed topics) with one 32-byte word of
/// data: 375 + 3 * 375 + 8 * 32.
const GAS_TRANSACTION_VERIFIED_EVENT: u64 = 1_756;
/// The encoded transaction is hashed and copied, so price it like `KECCAK256` (30 + 6 per word)
/// plus a memory copy (3 per word).
const GAS_KECCAK_BASE: u64 = 30;
const GAS_PER_TX_BYTES_WORD: u64 = 9;

/// Intrinsic gas of the call data (EIP-2028): 4 per zero byte, 16 per non-zero byte. The EVM
/// rejects a transaction whose gas limit is below it.
fn calldata_gas(data: &[u8]) -> u64 {
    data.iter()
        .map(|byte| if *byte == 0 { 4u64 } else { 16 })
        .fold(0u64, u64::saturating_add)
}

/// Gas limit for a `verifyAndEmit` transaction when RPC estimation is unavailable. It is sized
/// from the whole encoded call, so a large proof is not given less than its own intrinsic gas.
fn fallback_emit_gas(calldata: &[u8], encoded_tx_len: usize, continuity_roots: usize) -> u64 {
    let tx_bytes_words = (encoded_tx_len as u64).div_ceil(32);
    [
        TX_BASE_GAS,
        calldata_gas(calldata),
        GAS_PROOF_OVERHEAD,
        GAS_PER_CONTINUITY_ROOT.saturating_mul(continuity_roots as u64),
        GAS_KECCAK_BASE.saturating_add(GAS_PER_TX_BYTES_WORD.saturating_mul(tx_bytes_words)),
        GAS_TRANSACTION_VERIFIED_EVENT,
    ]
    .into_iter()
    .fold(0u64, u64::saturating_add)
}

/// A transaction needing more gas than a block holds can never be included.
#[allow(clippy::result_large_err)]
fn ensure_fits_block(required: u64, block_gas_limit: u64) -> Result<(), Error> {
    if required > block_gas_limit {
        return Err(Error::GasLimitExceeded {
            required,
            limit: block_gas_limit,
        });
    }
    Ok(())
}

/// The RPC estimate for a `verifyAndEmit` call, or the fallback when the estimate failed. The
/// fallback is rejected when it exceeds the latest block's gas limit.
async fn resolve_emit_gas<P: Provider>(
    provider: &P,
    estimate: Result<u64, AlloyContractError>,
    calldata: &[u8],
    encoded_tx_len: usize,
    continuity_roots: usize,
) -> Result<u64, Error> {
    let err = match estimate {
        Ok(gas) => {
            debug!("Gas estimation succeeded: {}", gas);
            return Ok(gas);
        }
        Err(err) => err,
    };

    // Estimation can fail in some setups (e.g. pallet-evm issues), so size the limit ourselves.
    let required = fallback_emit_gas(calldata, encoded_tx_len, continuity_roots);
    let block = provider
        .get_block(BlockId::latest())
        .await?
        .ok_or_else(|| Error::Other("latest block not found".to_string()))?;
    ensure_fits_block(required, block.header.gas_limit)?;
    debug!(
        "Gas estimation failed ({}), using calculated gas limit: {} for {} continuity roots",
        err, required, continuity_roots
    );
    Ok(required)
}

/// Count `TransactionVerified` logs emitted by the receipt's transaction.
///
/// We match on the event signature hash (`topic0`) of
/// `TransactionVerified(uint64,uint64,uint64)` so an unrelated log accidentally aliasing the
/// position of `topics[0]` cannot trigger a false positive.
///
/// Filtering by `log.address == precompile_address` is intentionally avoided here: the precompile
/// is invoked via a transaction whose `to` is the precompile itself, so the emitted log's
/// `address` is already constrained by the calldata path we built above. Counting is what catches
/// (a) a reverted/OOG tx that emitted nothing despite a replay-via-eth_call success and (b) a
/// silently dropped emission.
fn count_transaction_verified_logs(receipt: &TransactionReceipt) -> usize {
    let sig = INativeQueryVerifier::TransactionVerified::SIGNATURE_HASH;
    receipt
        .inner
        .logs()
        .iter()
        .filter(|log| log.topics().first() == Some(&sig))
        .count()
}

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
    #[error("Proof needs about {required} gas, above the block gas limit of {limit}")]
    GasLimitExceeded { required: u64, limit: u64 },
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
// The new ContinuityProof structure only has roots (digests computed on-chain)
// Solidity expects (bytes32 lowerEndpointDigest, bytes32[] roots)
fn convert_to_solidity_continuity_proof(
    proof: ContinuityProof,
) -> INativeQueryVerifier::ContinuityProof {
    // Convert roots to FixedBytes array
    let roots: Vec<FixedBytes<32>> = proof
        .roots
        .into_iter()
        .map(|root| FixedBytes::from(root.to_fixed_bytes()))
        .collect();

    INativeQueryVerifier::ContinuityProof {
        lowerEndpointDigest: FixedBytes::from(proof.lower_endpoint_digest.to_fixed_bytes()),
        roots,
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
    /// * `continuity_proof` - Optimized continuity proof (roots[0] is at queryHeight)
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

        Ok(result)
    }

    /// Verify a blockchain query with Merkle proof and continuity chain (transaction that emits events)
    ///
    /// # Arguments
    /// * `chain_key` - The chain key identifier
    /// * `height` - The block height to verify
    /// * `encoded_transaction` - Raw transaction data to verify
    /// * `merkle_proof` - Merkle proof for transaction inclusion
    /// * `continuity_proof` - Optimized continuity proof (roots[0] is at queryHeight)
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

        let gas_limit = resolve_emit_gas(
            contract.provider(),
            tx_builder.estimate_gas().await,
            tx_builder.calldata(),
            encoded_transaction.len(),
            continuity_proof.roots.len(),
        )
        .await?;
        let tx_builder = tx_builder.gas(gas_limit);

        let pending_tx = tx_builder.send().await?;
        let receipt = pending_tx.get_receipt().await.map_err(|e| {
            error!("Failed to get transaction receipt: {:?}", e);
            Error::Other(format!("Failed to get transaction receipt: {e}"))
        })?;

        info!(
            "Query verification transaction sent. Hash: {:?}, Gas used: {:?}",
            receipt.transaction_hash, receipt.gas_used
        );

        // A successful receipt status is a precondition for trusting the emitted-events path:
        // a reverted or out-of-gas transaction won't have emitted `TransactionVerified`, so the
        // previous "replay with eth_call and report success" flow would silently mislabel
        // those as successful verifications. Bail out here instead.
        if !receipt.status() {
            error!(
                tx = ?receipt.transaction_hash,
                gas_used = ?receipt.gas_used,
                "verifyAndEmit transaction reverted on-chain"
            );
            return Err(Error::VerificationFailed);
        }

        // Require exactly one `TransactionVerified` log for the single-query path. Counting the
        // emissions catches the case where the precompile silently returned without emitting
        // (e.g. an upstream contract or proxy ate the receipt) and prevents reporting success
        // when no event was actually published.
        let emitted = count_transaction_verified_logs(&receipt);
        if emitted != 1 {
            error!(
                tx = ?receipt.transaction_hash,
                emitted,
                "expected exactly 1 TransactionVerified log, got {emitted}"
            );
            return Err(Error::VerificationFailed);
        }

        info!("Query verification successful (with events)");

        Ok(true)
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

        Ok(result)
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

        // See `verify_and_emit`: a reverted transaction emits no `TransactionVerified` events,
        // so a replay-via-eth_call success report would lie about what landed on-chain. Bail.
        if !receipt.status() {
            error!(
                tx = ?receipt.transaction_hash,
                gas_used = ?receipt.gas_used,
                "verifyAndEmit batch transaction reverted on-chain"
            );
            return Err(Error::VerificationFailed);
        }

        // Each successfully verified query should produce one `TransactionVerified` log.
        let expected = heights.len();
        let emitted = count_transaction_verified_logs(&receipt);
        if emitted != expected {
            error!(
                tx = ?receipt.transaction_hash,
                expected,
                emitted,
                "batch TransactionVerified log count mismatch"
            );
            return Err(Error::VerificationFailed);
        }

        info!(
            "Batch query verification successful (with events): {} queries",
            expected
        );

        Ok(true)
    }

    /// Estimate the gas for submitting a query verification with [`Self::verify_and_emit`].
    ///
    /// When the RPC estimate fails, falls back to a size-based limit, or fails with
    /// [`Error::GasLimitExceeded`] if even that cannot fit in a block.
    pub async fn estimate_gas(
        &self,
        chain_key: u64,
        height: u64,
        encoded_transaction: &[u8],
        merkle_proof: TransactionMerkleProof,
        continuity_proof: ContinuityProof,
    ) -> Result<u64, Error> {
        let sol_proof: INativeQueryVerifier::MerkleProof = merkle_proof.into();
        let sol_proof_struct = convert_to_solidity_continuity_proof(continuity_proof.clone());

        let provider = self.client.get_wallet_ws_provider().await?;
        let contract = INativeQueryVerifier::new(self.address, provider);
        let tx_builder = contract.verifyAndEmit_0(
            chain_key,
            height,
            encoded_transaction.to_vec().into(),
            sol_proof,
            sol_proof_struct,
        );

        resolve_emit_gas(
            contract.provider(),
            tx_builder.estimate_gas().await,
            tx_builder.calldata(),
            encoded_transaction.len(),
            continuity_proof.roots.len(),
        )
        .await
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

    #[test]
    fn calldata_gas_prices_zero_and_non_zero_bytes() {
        assert_eq!(calldata_gas(&[]), 0);
        assert_eq!(calldata_gas(&[0, 0, 0]), 12);
        assert_eq!(calldata_gas(&[1, 0, 255]), 16 + 4 + 16);
    }

    #[test]
    fn fallback_emit_gas_adds_up_every_term() {
        // 21_000 base + 20_000 overhead + 30 keccak + 1_756 event
        assert_eq!(fallback_emit_gas(&[], 0, 0), 42_786);
        // + calldata 4 + 16, + 2 words of tx bytes (9 each), + 3 roots (5_000 each)
        assert_eq!(
            fallback_emit_gas(&[0, 1], 33, 3),
            21_000 + 20 + 20_000 + 15_000 + (30 + 18) + 1_756
        );
    }

    /// The old formula (`21_000 + 5_000 * roots + 20_000`) fell below the intrinsic gas of a
    /// proof with large call data, so such a transaction could never be accepted.
    #[test]
    fn fallback_emit_gas_covers_the_intrinsic_gas_of_large_calldata() {
        let calldata = vec![0xAB; 200_000];
        let roots = 10;
        let intrinsic = TX_BASE_GAS + calldata_gas(&calldata);
        let old_formula = 21_000u64 + roots as u64 * 5_000 + 20_000;

        assert!(old_formula < intrinsic);
        assert!(fallback_emit_gas(&calldata, 1_000, roots) > intrinsic);
    }

    #[test]
    fn fallback_emit_gas_grows_with_each_input() {
        let base = fallback_emit_gas(&[1; 100], 100, 5);
        assert!(fallback_emit_gas(&[1; 101], 100, 5) > base);
        // Transaction bytes are priced per 32-byte word: 100 bytes is 4 words, 129 is 5.
        assert!(fallback_emit_gas(&[1; 100], 129, 5) > base);
        assert!(fallback_emit_gas(&[1; 100], 100, 6) > base);
    }

    #[test]
    fn fallback_emit_gas_saturates_instead_of_overflowing() {
        assert_eq!(fallback_emit_gas(&[], usize::MAX, usize::MAX), u64::MAX);
    }

    #[test]
    fn ensure_fits_block_rejects_only_above_the_limit() {
        assert!(ensure_fits_block(75_000_000, 75_000_000).is_ok());
        assert!(matches!(
            ensure_fits_block(75_000_001, 75_000_000),
            Err(Error::GasLimitExceeded {
                required: 75_000_001,
                limit: 75_000_000
            })
        ));
    }
}
