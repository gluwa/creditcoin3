//! Mock RPC providers for testing.
//!
//! This module provides mock implementations of [`CcRpcProvider`] and [`EthRpcProvider`]
//! that return deterministic test data without requiring real RPC endpoints.
//!
//! # Usage
//!
//! ```rust
//! use continuity::mocks::make_mock_providers;
//!
//! let chain_key = 2;
//! let (cc_provider, eth_provider) = make_mock_providers(chain_key);
//! // Use with ContinuityBuilder::new_with_providers
//! ```
//!
//! # Mock Data
//!
//! The mock providers return:
//! - **Attestations:** Every 10 blocks (10, 20, 30, ...)
//! - **Checkpoints:** Every 100 blocks (0, 100, 200, ...)
//! - **Attestation interval:** 10 blocks
//! - **Checkpoint interval:** 10 attestations

use crate::rpc::{CcRpcProvider, EthRpcProvider};
use anyhow::Result;
use async_trait::async_trait;
use attestor_primitives::block::Block;
use attestor_primitives::{AttestationCheckpoint, AttestationData, SignedAttestation};
use cc_client::AccountId32;
use sp_core::H256;
use std::sync::Arc;

/// Canonical mock root for a given block number.
///
/// All mock providers share the same `root(n)` convention so that the attestation
/// cache, checkpoint cache, and `build_continuity_blocks` output produce the same
/// digest chain, which lets the continuity-service upper-boundary verification
/// succeed in tests.
fn mock_root(block_number: u64) -> H256 {
    H256::from_low_u64_be(block_number + 1000)
}

/// Canonical mock digest chain: `digest(0) = 0`, `digest(n) = hash_payload(n, root(n), digest(n - 1))`.
///
/// The CC3 mock returns attestations / checkpoints with this digest, and the ETH mock's
/// `build_continuity_blocks` reproduces the same chain when started from the correct
/// lower digest.
fn mock_digest_chain(up_to_block: u64) -> H256 {
    let mut digest = H256::zero();
    for n in 1..=up_to_block {
        let root = mock_root(n);
        digest = Block::hash_payload(&n, &root, &digest);
    }
    digest
}

/// Mock Creditcoin3 RPC provider for testing.
///
/// Returns deterministic fake attestations and checkpoints without requiring
/// a real CC3 node connection.
///
/// # Attestation Schedule
///
/// - Attestations occur every 10 blocks: 10, 20, 30, ...
/// - Checkpoints occur every 100 blocks: 0, 100, 200, ...
/// - Genesis block: 0 (configurable)
///
/// # Examples
///
/// ```rust
/// use continuity::mocks::MockCcRpcProvider;
///
/// let provider = MockCcRpcProvider::new(1);
/// // Use with ContinuityBuilder::new_with_providers
/// ```
pub struct MockCcRpcProvider {
    /// The chain key this mock provider is configured for
    pub chain_key: u64,
    /// Genesis block number for attestation chain (default 0)
    pub genesis_block: u64,
}

impl MockCcRpcProvider {
    /// Create a new mock CC3 provider for the given chain.
    pub fn new(chain_key: u64) -> Self {
        Self {
            chain_key,
            genesis_block: 0,
        }
    }
}

#[async_trait]
impl CcRpcProvider for MockCcRpcProvider {
    async fn get_attestations_for_chain(
        &self,
        chain_key: u64,
    ) -> Result<Vec<SignedAttestation<H256, AccountId32>>> {
        let mk_attestation = |header_number: u64| {
            // Chain the attestation's prev_digest so that AttestationData::digest()
            // returns `mock_digest_chain(header_number)` — i.e. the same digest that
            // the mock ETH provider computes at this block. This keeps the
            // attestation cache aligned with `build_continuity_blocks` output and
            // allows upper-boundary digest verification to succeed in tests.
            let prev_digest = if header_number == 0 {
                None
            } else {
                Some(mock_digest_chain(header_number - 1))
            };
            SignedAttestation {
                attestation: AttestationData {
                    chain_key,
                    header_number,
                    header_hash: H256::from_low_u64_be(header_number),
                    root: mock_root(header_number),
                    prev_digest,
                },
                signature: [0u8; 96],
                attestors: vec![],
                continuity_proof: Default::default(),
            }
        };
        // Attestations every 10 blocks (matching DefaultAttestationInterval = 10)
        // Range covers 10..=1000 to match checkpoint mock coverage
        Ok((1..=100).map(|i| mk_attestation(i * 10)).collect())
    }

    async fn get_last_checkpoint(&self, _chain_key: u64) -> Result<Option<AttestationCheckpoint>> {
        // Checkpoints happen every 100 blocks (10 attestations * 10 interval)
        // For testing, we use checkpoint at block 0 (genesis)
        Ok(Some(AttestationCheckpoint {
            block_number: 0,
            digest: mock_digest_chain(0),
        }))
    }

    async fn get_checkpoints_for_chain(
        &self,
        _chain_key: u64,
    ) -> Result<Vec<AttestationCheckpoint>> {
        // For testing, provide checkpoints at regular intervals (every 100 blocks).
        // Digests are chained so they match the mock ETH provider's output at the same block.
        Ok((0..=10)
            .map(|i| AttestationCheckpoint {
                block_number: i * 100,
                digest: mock_digest_chain(i * 100),
            })
            .collect())
    }

    async fn get_stable_checkpoints_for_chain(
        &self,
        chain_key: u64,
    ) -> Result<Vec<AttestationCheckpoint>> {
        // Default mock: no reversion in flight, identical to the unfiltered set.
        // Tests that need to model an active revert pruning window should use a
        // bespoke provider that overrides this method.
        self.get_checkpoints_for_chain(chain_key).await
    }

    async fn get_checkpoint_by_height(
        &self,
        _chain_key: u64,
        block_number: u64,
    ) -> Result<Option<AttestationCheckpoint>> {
        // Mock implementation: return checkpoint if it exists in our mock data
        match block_number {
            0 => Ok(Some(AttestationCheckpoint {
                block_number: 0,
                digest: mock_digest_chain(0),
            })),
            _ => Ok(None),
        }
    }

    async fn get_attestation_chain_genesis_block_number(&self, _chain_key: u64) -> Result<u64> {
        Ok(self.genesis_block)
    }

    async fn fetch_last_digest(&self, _chain_key: u64) -> Result<Option<H256>> {
        // Return the chained digest of the highest mock attestation (block 1000) so that
        // `get_attestation_by_digest` can resolve it back via the canonical chain.
        Ok(Some(mock_digest_chain(1000)))
    }

    async fn get_attestation_by_digest(
        &self,
        chain_key: u64,
        digest: H256,
    ) -> Result<Option<SignedAttestation<H256, AccountId32>>> {
        // Mock attestations are at heights 10, 20, ..., 1000 with the canonical chained digest.
        // Scan the schedule and return the matching attestation (if any).
        for i in 1..=100u64 {
            let header_number = i * 10;
            if mock_digest_chain(header_number) == digest {
                return Ok(Some(SignedAttestation {
                    attestation: AttestationData {
                        chain_key,
                        header_number,
                        header_hash: H256::from_low_u64_be(header_number),
                        root: mock_root(header_number),
                        prev_digest: Some(mock_digest_chain(header_number - 1)),
                    },
                    signature: [0u8; 96],
                    attestors: vec![],
                    continuity_proof: Default::default(),
                }));
            }
        }
        Ok(None)
    }

    async fn get_attestation_interval(&self, _chain_key: u64) -> Result<Option<u64>> {
        // Mock returns 10 blocks per attestation (matching mock attestations at 10, 20, 30)
        Ok(Some(10))
    }

    async fn get_checkpoint_interval(&self, _chain_key: u64) -> Result<Option<u64>> {
        // Mock returns 10 attestations per checkpoint (matching comment at line 44)
        Ok(Some(10))
    }
}

/// Mock source chain (ETH/EVM) RPC provider for testing.
///
/// Returns deterministic fake blocks without requiring a real blockchain connection.
///
/// # Behavior
///
/// - Builds continuity blocks with simple deterministic digests
/// - Transaction count: `(block_number % 3) + 1` transactions per block
/// - Last block: Always returns 1000
/// - Chain ID: 31337 (Anvil default)
///
/// # Examples
///
/// ```rust
/// use continuity::mocks::MockEthRpcProvider;
/// use std::sync::Arc;
///
/// let provider = Arc::new(MockEthRpcProvider);
/// // Use with ContinuityBuilder::new_with_providers
/// ```
pub struct MockEthRpcProvider;

#[async_trait]
impl EthRpcProvider for MockEthRpcProvider {
    async fn build_continuity_blocks(
        &self,
        lower_digest: H256,
        start: u64,
        end: u64,
    ) -> Result<Vec<Block>> {
        // Produce the same digest chain as `mock_digest_chain` so that the upper-boundary
        // digest stored in the attestation/checkpoint cache matches what this chain computes
        // (that reconciliation is what the continuity service now verifies).
        let mut prev = lower_digest;
        let mut blocks = Vec::with_capacity((end.saturating_sub(start) as usize) + 1);
        for n in start..=end {
            let block = Block::new_from_prev_digest(n, mock_root(n), prev);
            prev = block.digest();
            blocks.push(block);
        }
        Ok(blocks)
    }

    async fn get_block_tx_bytes(&self, block_number: u64) -> Result<Vec<Vec<u8>>> {
        // Return a deterministic list of transaction payload bytes for the block
        // For testing, create N transactions where N = (block_number % 3) + 1
        let count = (block_number % 3) as usize + 1;
        let mut txs = Vec::with_capacity(count);
        for i in 0..count {
            // Simple deterministic payload: block_number || tx_index
            let mut b = Vec::with_capacity(16);
            b.extend_from_slice(&block_number.to_be_bytes());
            b.extend_from_slice(&(i as u64).to_be_bytes());
            txs.push(b);
        }
        Ok(txs)
    }

    async fn get_tx_hash_by_index(&self, block_number: u64, tx_index: u64) -> Result<Option<H256>> {
        // Generate a deterministic hash for testing purposes
        // Hash is based on block_number and tx_index
        let mut bytes = [0u8; 32];
        bytes[..8].copy_from_slice(&block_number.to_be_bytes());
        bytes[8..16].copy_from_slice(&tx_index.to_be_bytes());
        // Fill rest with deterministic pattern
        for (i, byte) in bytes.iter_mut().enumerate().skip(16) {
            *byte = ((block_number + tx_index + i as u64) % 256) as u8;
        }
        Ok(Some(H256::from(bytes)))
    }

    async fn get_tx_position_by_hash(&self, _tx_hash: H256) -> Result<Option<(u64, u64)>> {
        Ok(None)
    }

    async fn get_last_block(&self) -> Result<u64> {
        // Mock returns a high block number for testing
        Ok(1000)
    }

    async fn get_chain_id(&self) -> Result<u64> {
        // Mock returns test chain ID
        Ok(31337)
    }

    async fn is_healthy(&self) -> Result<bool> {
        // Mock provider is always healthy
        Ok(true)
    }
}

/// Create a pair of mock RPC providers for testing.
///
/// This is a convenience function that creates both mock providers configured
/// for the given chain. Use this in tests to avoid dealing with real RPC endpoints.
///
/// # Arguments
///
/// * `chain_key` - The chain identifier for the mock CC3 provider
///
/// # Returns
///
/// A tuple of `(cc_provider, eth_provider)` ready to be used with
/// [`ContinuityBuilder::new_with_providers`](crate::ContinuityBuilder::new_with_providers).
///
/// # Mock Behavior
///
/// **CC3 Provider:**
/// - Returns attestations at blocks 10, 20, 30
/// - Returns checkpoints at blocks 0, 100, 200
/// - Genesis block: 0
/// - Attestation interval: 10 blocks
/// - Checkpoint interval: 10 attestations
///
/// **ETH Provider:**
/// - Builds continuity blocks with deterministic digests
/// - Last block: 1000
/// - Chain ID: 31337
///
/// # Examples
///
/// ```rust
/// use continuity::{ContinuityBuilder, ContinuityConfig, mocks::make_mock_providers};
/// use std::sync::Arc;
///
/// #[tokio::test]
/// async fn test_with_mocks() -> anyhow::Result<()> {
///     let chain_key = 2;
///     let config = ContinuityConfig::builder()
///         .cc3_rpc_url("http://mock")
///         .eth_rpc_url("http://mock")
///         .chain_key(chain_key)
///         .attestation_interval(10)
///         .checkpoint_interval(10)
///         .build();
///
///     let (cc_provider, eth_provider) = make_mock_providers(chain_key);
///     let builder = ContinuityBuilder::new_with_providers(
///         config,
///         cc_provider,
///         eth_provider,
///     );
///
///     // Use builder in tests
///     let (lower, upper, _) = builder.get_endpoints(&[15], None).await?;
///     assert_eq!(lower.block_number, 10);
///     assert_eq!(upper.block_number, 20);
///
///     Ok(())
/// }
/// ```
pub fn make_mock_providers(chain_key: u64) -> (Arc<MockCcRpcProvider>, Arc<MockEthRpcProvider>) {
    (
        Arc::new(MockCcRpcProvider::new(chain_key)),
        Arc::new(MockEthRpcProvider),
    )
}
