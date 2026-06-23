//! Write-ability task configuration (confluence §7.3 A1).
//!
//! Message attestation is **opt-in per attestor**: when [`Config::enabled`] is false the task parks
//! until shutdown and the attestor behaves exactly as before. When enabled, the attestor connects
//! to the Creditcoin L1 EVM endpoint, resolves the Outbox for its `chain_key`, and starts signing
//! and gossiping message votes on `{chain_key}/message-votes/v1`.
//!
//! Outbox resolution prefers on-chain data (the `outbox_factory_address` shipped in PR #873 plus
//! `IOutboxFactory.getOutbox`), but every input has a config override so an operator can run the
//! PoC before the runtime `SupportedChain` write-ability fields / factory contract exist.

use std::time::Duration;

use alloy::primitives::{Address, B256};

/// How the set of authorized message-vote signers (EVM addresses) is determined. Gossip votes from
/// signers outside this set are rejected, and the quorum `N` is derived from its size
/// (confluence §6.6, §5.3).
#[derive(Clone, Debug)]
pub enum AttesterSet {
    /// Static list of EVM attester addresses (PoC / config fallback).
    Static(Vec<Address>),
    /// Read `IVoteValidator.attesters()` from the on-chain validator at this address.
    OnChainValidator(Address),
}

impl Default for AttesterSet {
    fn default() -> Self {
        AttesterSet::Static(Vec::new())
    }
}

/// Write-ability task configuration.
#[derive(Clone, Debug)]
pub struct Config {
    /// Master switch (confluence A1 `message_attestation_enabled`).
    pub enabled: bool,

    /// Creditcoin L1 EVM JSON-RPC endpoint to watch the Outbox on. Required when `enabled`.
    pub cc3_eth_rpc_url: Option<url::Url>,

    /// Outbox factory address override. When `None`, resolved from runtime (PR #873 precompile /
    /// `SupportedChains::OutboxFactories`).
    pub outbox_factory_address: Option<Address>,

    /// `bytes32` write-ability chain key passed to `getOutbox` and bound into each `messageHash`.
    /// When `None`, derived from the `u64` chain_key via
    /// [`write_ability::protocol::chain_key_to_bytes32`].
    pub write_ability_chain_key: Option<B256>,

    /// Direct Outbox address override — skips the factory lookup entirely (fastest PoC path).
    pub outbox_address: Option<Address>,

    /// Confirmation depth below the EVM tip before a `MessagePublished` log is considered final
    /// enough to sign (the probabilistic-finality fallback bound — confluence §6.8).
    pub block_confirmation_depth: u64,

    /// Hard cap on distinct tracked `message_hash` entries (anti-abuse — confluence §5.4).
    pub max_tracked_messages: usize,

    /// Drop partial vote aggregates older than this unless already complete (anti-abuse).
    pub vote_ttl: Duration,

    /// Source of the authorized signer set / quorum size.
    pub attester_set: AttesterSet,
}

impl Config {
    /// A disabled configuration — the default wired into the attestor so the binary runs unchanged
    /// until message attestation is explicitly turned on.
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            cc3_eth_rpc_url: None,
            outbox_factory_address: None,
            write_ability_chain_key: None,
            outbox_address: None,
            block_confirmation_depth: 0,
            max_tracked_messages: DEFAULT_MAX_TRACKED_MESSAGES,
            vote_ttl: DEFAULT_VOTE_TTL,
            attester_set: AttesterSet::default(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::disabled()
    }
}

/// Default anti-abuse bound on distinct tracked messages per chain key.
pub const DEFAULT_MAX_TRACKED_MESSAGES: usize = 10_000;

/// Default TTL for incomplete vote aggregates.
pub const DEFAULT_VOTE_TTL: Duration = Duration::from_secs(3600);
