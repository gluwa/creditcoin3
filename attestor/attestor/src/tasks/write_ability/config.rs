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

use std::path::PathBuf;
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
    /// current head and only signs future messages. Only consulted when there is no persisted scan
    /// cursor yet (see [`state_dir`](Self::state_dir)); once a cursor exists it takes precedence, so
    /// a restart resumes rather than replaying from `start_block`.
    pub start_block: Option<u64>,

    /// Directory for durable write-ability state — currently the Outbox scan cursor (`last_seen`
    /// block), persisted so a restart resumes exactly where it left off instead of skipping messages
    /// published while down or re-signing the whole history. The boot verifies the directory is writable
    /// (see [`super::cursor::ensure_writable`]) so a missing/read-only volume fails loudly rather
    /// than silently degrading. Defaults to [`DEFAULT_STATE_DIR`]; must be a persistent volume for
    /// the cursor to survive pod restarts.
    pub state_dir: PathBuf,

    /// Hard cap on distinct tracked `message_hash` entries (anti-abuse — confluence §5.4).
    pub max_tracked_messages: usize,

    /// Drop partial vote aggregates older than this unless already complete (anti-abuse).
    pub vote_ttl: Duration,

    /// Source of the authorized signer set / quorum size.
    pub attestor_set: AttestorSet,

    /// When a governance factory rotation is detected, resume the `OutboxCreated` discovery scan
    /// from the last factory-scan checkpoint instead of resetting it to genesis — cutting the
    /// rescan down to just the blocks since that checkpoint. Safe as long as the new factory is
    /// itself freshly deployed at rotation time (its `OutboxCreated` can only land at or after the
    /// checkpoint, since the checkpoint already reflects a later block than the new factory's
    /// deployment). Re-pointing to a *pre-existing* factory whose `OutboxCreated` predates the
    /// checkpoint would make discovery skip it and never find that chain key's Outbox — `false`
    /// restores the old unconditional genesis rescan, which has no such assumption.
    ///
    /// That skip is no longer terminal: a resumed scan that reaches the confirmed tip having found
    /// nothing retries once from [`factory_scan_genesis_block`](Self::factory_scan_genesis_block)
    /// before reporting "no Outbox" (see [`super::resolver`]). Leaving this on therefore costs one
    /// extra full scan in the pre-existing-factory case rather than stranding the chain key.
    pub resume_rotation_from_checkpoint: bool,

    /// First Creditcoin L1 block any *from-scratch* `OutboxCreated` discovery scan starts at — the
    /// floor that replaces a literal block 0 everywhere discovery restarts: initial activation, a
    /// rotation with [`resume_rotation_from_checkpoint`](Self::resume_rotation_from_checkpoint)
    /// off, a factory de-registration, and the genesis fallback above.
    ///
    /// Defaults to [`DEFAULT_FACTORY_SCAN_GENESIS_BLOCK`] (0), i.e. a true genesis scan. Raising it
    /// to a height known to precede this chain key's Outbox factory deployment cuts every one of
    /// those scans down to the blocks that could actually contain the event — on a long-lived chain
    /// that is the difference between a multi-minute fleet-wide quorum outage and a brief one.
    ///
    /// **It answers one question: where does a scan with no usable position start?** It never
    /// moves a cursor that has one — a persisted scan resumes from where it left off, and a
    /// rotation that resumes from a checkpoint keeps that checkpoint, in both cases even when the
    /// position sits *below* this value. The single deliberate exception is the genesis fallback,
    /// whose whole job is to rewind to this floor once a resumed scan has proven inconclusive.
    ///
    /// ⚠️ Set it **above** the block a factory emitted its `OutboxCreated` in and that Outbox
    /// becomes undiscoverable — including via the genesis fallback, which floors here too. The safe
    /// value is a height known to precede every factory this chain key will ever be pointed at,
    /// e.g. the block the chain's first Outbox factory was deployed in, minus a margin.
    pub factory_scan_genesis_block: u64,
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
            state_dir: PathBuf::from(DEFAULT_STATE_DIR),
            max_tracked_messages: DEFAULT_MAX_TRACKED_MESSAGES,
            vote_ttl: DEFAULT_VOTE_TTL,
            attestor_set: AttestorSet::default(),
            resume_rotation_from_checkpoint: DEFAULT_RESUME_ROTATION_FROM_CHECKPOINT,
            factory_scan_genesis_block: DEFAULT_FACTORY_SCAN_GENESIS_BLOCK,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::disabled()
    }
}

impl Config {
    /// Reject a startup configuration that would silently weaken safety or prevent quorum (audit
    /// P2-7): a zero confirmation depth (signing at the chain tip), zero vote TTL / tracked-message
    /// cap, or a zero / empty / duplicated attestor set. Only enforced when `enabled` — a disabled
    /// config is always valid. Returns a human-readable reason so the boot fails loudly rather than
    /// coming up subtly mis-secured.
    pub fn validate(&self) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        if self.cc3_eth_rpc_url.is_none() {
            return Err("message attestation enabled but no Creditcoin EVM RPC URL".to_string());
        }
        if self.block_confirmation_depth == 0 {
            return Err(
                "block_confirmation_depth must be > 0 — signing at the chain tip is unsafe"
                    .to_string(),
            );
        }
        // Guard against a fat-fingered depth (e.g. an extra few zeros). It is only the fallback
        // bound now that we sign the finalized head (P1-2), but an absurd value would make the
        // fallback path saturate to block 0 and silently never sign — a liveness footgun validate()
        // should catch loudly at boot rather than let the attestor come up mute.
        if self.block_confirmation_depth > MAX_BLOCK_CONFIRMATION_DEPTH {
            return Err(format!(
                "block_confirmation_depth {} is implausibly large (> {}) — likely a typo; the \
                 attestor would silently never sign",
                self.block_confirmation_depth, MAX_BLOCK_CONFIRMATION_DEPTH
            ));
        }
        validate_rpc_url("cc3_eth_rpc_url", self.cc3_eth_rpc_url.as_ref())?;
        validate_rpc_url(
            "destination_eth_rpc_url",
            self.destination_eth_rpc_url.as_ref(),
        )?;
        if self.vote_ttl.is_zero() {
            return Err("vote_ttl must be > 0".to_string());
        }
        if self.max_tracked_messages == 0 {
            return Err("max_tracked_messages must be > 0".to_string());
        }
        match &self.attestor_set {
            AttestorSet::Static(addrs) => {
                if addrs.is_empty() {
                    return Err(
                        "message attestation enabled but the static attestor_set is empty"
                            .to_string(),
                    );
                }
                if addrs.contains(&Address::ZERO) {
                    return Err("static attestor_set contains the zero address".to_string());
                }
                let mut seen = std::collections::HashSet::with_capacity(addrs.len());
                if let Some(dup) = addrs.iter().find(|a| !seen.insert(**a)) {
                    return Err(format!(
                        "static attestor_set contains a duplicate address: {dup}"
                    ));
                }
            }
            AttestorSet::OnChainValidator(validator) => {
                if *validator == Address::ZERO {
                    return Err("OnChainValidator address is the zero address".to_string());
                }
                if self.destination_eth_rpc_url.is_none() {
                    return Err(
                        "OnChainValidator attestor set configured but no destination EVM RPC URL"
                            .to_string(),
                    );
                }
            }
        }
        Ok(())
    }
}

/// Default directory for durable write-ability state (the Outbox scan cursor). Matches the
/// persistent volume the attestor StatefulSet mounts (`/data`), so production needs no explicit
/// `state_dir`; local/dev runs override it in config.
pub const DEFAULT_STATE_DIR: &str = "/data";

/// Default confirmation depth below the EVM tip before a `MessagePublished` log is signed.
/// Three blocks matches the usual time-to-finality on Creditcoin.
pub const DEFAULT_BLOCK_CONFIRMATION_DEPTH: u64 = 3;

/// Sanity ceiling for `block_confirmation_depth` (audit P2-7). Far above any plausible finality
/// depth, so a value beyond it is almost certainly a typo that would make the fallback path saturate
/// to block 0 and silently never sign.
pub const MAX_BLOCK_CONFIRMATION_DEPTH: u64 = 100_000;

/// Reject an RPC URL whose scheme the EVM/substrate clients can't dial (audit P2-7). Accepts only
/// `http(s)` / `ws(s)`; `None` is accepted here (presence is enforced separately per attestor-set
/// mode). Catches a fat-fingered `htto://…` / `file://…` at boot instead of at first RPC call.
fn validate_rpc_url(field: &str, url: Option<&url::Url>) -> Result<(), String> {
    if let Some(u) = url {
        match u.scheme() {
            "http" | "https" | "ws" | "wss" => {}
            other => {
                return Err(format!(
                    "{field} has unsupported URL scheme '{other}' (expected http/https/ws/wss)"
                ));
            }
        }
    }
    Ok(())
}

/// Default anti-abuse bound on distinct tracked messages per chain key.
pub const DEFAULT_MAX_TRACKED_MESSAGES: usize = 10_000;

/// Default TTL for incomplete vote aggregates.
pub const DEFAULT_VOTE_TTL: Duration = Duration::from_secs(3600);

/// Default for [`Config::resume_rotation_from_checkpoint`]: resume from the last factory-scan
/// checkpoint rather than genesis on a factory rotation. On by default — a rotation deploying a
/// fresh factory (the normal case) is safe to resume this way, and the multi-minute genesis
/// rescan it replaces is itself an active operational cost (confluence write-ability handoff §3.4,
/// §10.9). Set to `false` if a deployment ever re-points a chain key at a pre-existing factory.
pub const DEFAULT_RESUME_ROTATION_FROM_CHECKPOINT: bool = true;

/// Default for [`Config::factory_scan_genesis_block`]: block 0, i.e. a discovery scan that starts
/// from scratch really does start at genesis. Preserves the historical behaviour for every
/// deployment that does not set it, so raising it is always an explicit decision about a specific
/// chain whose factory deployment height the operator knows.
pub const DEFAULT_FACTORY_SCAN_GENESIS_BLOCK: u64 = 0;

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::address;

    fn enabled_base() -> Config {
        Config {
            enabled: true,
            cc3_eth_rpc_url: Some("ws://localhost:9944".parse().unwrap()),
            destination_eth_rpc_url: Some("ws://localhost:8545".parse().unwrap()),
            write_ability_chain_key: 2,
            block_confirmation_depth: DEFAULT_BLOCK_CONFIRMATION_DEPTH,
            start_block: None,
            state_dir: PathBuf::from(DEFAULT_STATE_DIR),
            max_tracked_messages: DEFAULT_MAX_TRACKED_MESSAGES,
            vote_ttl: DEFAULT_VOTE_TTL,
            attestor_set: AttestorSet::Static(vec![address!(
                "000000000000000000000000000000000000000a"
            )]),
            resume_rotation_from_checkpoint: DEFAULT_RESUME_ROTATION_FROM_CHECKPOINT,
            factory_scan_genesis_block: DEFAULT_FACTORY_SCAN_GENESIS_BLOCK,
        }
    }

    #[test]
    fn disabled_config_is_always_valid() {
        assert!(Config::disabled().validate().is_ok());
    }

    #[test]
    fn valid_enabled_config_passes() {
        assert!(enabled_base().validate().is_ok());
    }

    #[test]
    fn rejects_zero_confirmation_depth() {
        let mut c = enabled_base();
        c.block_confirmation_depth = 0;
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_implausibly_large_confirmation_depth() {
        let mut c = enabled_base();
        c.block_confirmation_depth = MAX_BLOCK_CONFIRMATION_DEPTH + 1;
        assert!(c.validate().is_err());
        // The ceiling itself is accepted.
        c.block_confirmation_depth = MAX_BLOCK_CONFIRMATION_DEPTH;
        assert!(c.validate().is_ok());
    }

    #[test]
    fn rejects_unsupported_rpc_url_scheme() {
        let mut c = enabled_base();
        c.cc3_eth_rpc_url = Some("ftp://localhost:9944".parse().unwrap());
        assert!(c.validate().is_err());
        // A destination URL with a bad scheme is caught too.
        let mut c = enabled_base();
        c.destination_eth_rpc_url = Some("file:///etc/passwd".parse().unwrap());
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_zero_vote_ttl_and_cap() {
        let mut c = enabled_base();
        c.vote_ttl = Duration::ZERO;
        assert!(c.validate().is_err());
        let mut c = enabled_base();
        c.max_tracked_messages = 0;
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_empty_zero_and_duplicate_static_set() {
        let mut c = enabled_base();
        c.attestor_set = AttestorSet::Static(vec![]);
        assert!(c.validate().is_err(), "empty");

        c.attestor_set = AttestorSet::Static(vec![Address::ZERO]);
        assert!(c.validate().is_err(), "zero address");

        let a = address!("000000000000000000000000000000000000000a");
        c.attestor_set = AttestorSet::Static(vec![a, a]);
        assert!(c.validate().is_err(), "duplicate");
    }

    #[test]
    fn rejects_zero_validator_and_missing_dest_rpc() {
        let mut c = enabled_base();
        c.attestor_set = AttestorSet::OnChainValidator(Address::ZERO);
        assert!(c.validate().is_err(), "zero validator");

        c.attestor_set =
            AttestorSet::OnChainValidator(address!("000000000000000000000000000000000000000b"));
        c.destination_eth_rpc_url = None;
        assert!(c.validate().is_err(), "missing dest rpc");
    }
}
