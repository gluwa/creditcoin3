//! Write-ability task configuration (confluence §7.3 A1).
//!
//! Message attestation is **opt-in per attestor**: when [`Config::enabled`] is false the task parks
//! until shutdown and the attestor behaves exactly as before. When enabled, the attestor connects
//! to the Creditcoin L1 EVM endpoint, resolves the Outbox for its `chain_key`, and starts signing
//! and gossiping message votes on `{chain_key}/message-votes/v1`.
//!
//! The Outbox is resolved entirely on-chain from the attestor's `chain_key` (factory + chain-info
//! precompile). Addresses are deliberately not configurable: supplying an address separately from
//! the chain key is error prone, because the address might not correspond to that chain key.

use std::time::Duration;

use alloy::primitives::Address;

use attestor_primitives::ChainKey;

/// How the set of authorized message-vote signers (EVM addresses) is determined. Gossip votes from
/// signers outside this set are rejected, and the quorum `N` is derived from its size
/// (confluence §6.6, §5.3).
#[derive(Clone, Debug)]
pub enum AttestorSet {
    /// Static list of EVM attestor addresses (PoC / config fallback).
    Static(Vec<Address>),
    /// Read `IVoteValidator.attestors()` from the on-chain validator at this address.
    OnChainValidator(Address),
}

impl Default for AttestorSet {
    fn default() -> Self {
        AttestorSet::Static(Vec::new())
    }
}

/// Write-ability task configuration.
#[derive(Clone, Debug)]
pub struct Config {
    /// Master switch (confluence A1 `message_attestation_enabled`).
    pub enabled: bool,

    /// Creditcoin L1 EVM JSON-RPC endpoint to watch the Outbox on. Required when `enabled`.
    /// Derived from the top-level `cc3` RPC url at configuration generation.
    pub cc3_eth_rpc_url: Option<url::Url>,

    /// Destination-chain EVM JSON-RPC endpoint — the chain this attestor set attests block heights
    /// for (its `eth` URL), where the Inbox and `EOAValidator` live. Only read when
    /// [`AttestorSet::OnChainValidator`] is configured, to fetch the authorized attestor set.
    pub destination_eth_rpc_url: Option<url::Url>,

    /// Write-ability chain key (`u64`) for this attestor, set from the top-level `chain_key` at
    /// configuration generation. Used as the `u64` key to resolve the Outbox on-chain (chain-info
    /// precompile → factory) and, via [`write_ability::protocol::chain_key_to_bytes32`], as the
    /// `bytes32` key passed to `getOutbox` and bound into each `messageHash`.
    pub write_ability_chain_key: ChainKey,

    /// Confirmation depth below the EVM tip before a `MessagePublished` log is considered final
    /// enough to sign (the probabilistic-finality fallback bound — confluence §6.8).
    pub block_confirmation_depth: u64,

    /// First Creditcoin L1 EVM block to scan on startup. When `None`, the listener starts at the
    /// current head and only signs future messages.
    pub start_block: Option<u64>,

    /// Hard cap on distinct tracked `message_hash` entries (anti-abuse — confluence §5.4).
    pub max_tracked_messages: usize,

    /// Drop partial vote aggregates older than this unless already complete (anti-abuse).
    pub vote_ttl: Duration,

    /// Source of the authorized signer set / quorum size.
    pub attestor_set: AttestorSet,
}

impl Config {
    /// A disabled configuration — the default wired into the attestor so the binary runs unchanged
    /// until message attestation is explicitly turned on.
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            cc3_eth_rpc_url: None,
            destination_eth_rpc_url: None,
            write_ability_chain_key: 0,
            block_confirmation_depth: DEFAULT_BLOCK_CONFIRMATION_DEPTH,
            start_block: None,
            max_tracked_messages: DEFAULT_MAX_TRACKED_MESSAGES,
            vote_ttl: DEFAULT_VOTE_TTL,
            attestor_set: AttestorSet::default(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::disabled()
    }
}

/// Default confirmation depth below the EVM tip before a `MessagePublished` log is signed.
/// Three blocks matches the usual time-to-finality on Creditcoin.
pub const DEFAULT_BLOCK_CONFIRMATION_DEPTH: u64 = 3;

/// Default anti-abuse bound on distinct tracked messages per chain key.
pub const DEFAULT_MAX_TRACKED_MESSAGES: usize = 10_000;

/// Default TTL for incomplete vote aggregates.
pub const DEFAULT_VOTE_TTL: Duration = Duration::from_secs(3600);
