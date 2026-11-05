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

/// Write-ability task configuration.
#[derive(Clone, Debug)]
pub struct Config {
    /// Master switch (confluence A1 `message_attestation_enabled`).
    pub enabled: bool,

    /// Creditcoin L1 EVM JSON-RPC endpoint to watch the Outbox on. Required when `enabled`.
    /// Derived from the top-level `cc3` RPC url at configuration generation.
    pub cc3_eth_rpc_url: Option<url::Url>,

    /// Destination-chain EVM JSON-RPC endpoint — the chain this attestor set attests block heights
    /// for (its `eth` URL), where the Inbox and `EOAValidator` live. The authorized attestor set,
    /// quorum threshold and message-vote EIP-712 domain are all read from here.
    pub destination_eth_rpc_url: Option<url::Url>,

    /// Write-ability chain key (`u64`) for this attestor, set from the top-level `chain_key` at
    /// configuration generation. Used as the `u64` key to resolve the Outbox on-chain (chain-info
    /// precompile → factory) and, via [`write_ability::protocol::chain_key_to_bytes32`], as the
    /// `bytes32` key passed to `getOutbox` and matched against each indexed message's route.
    pub write_ability_chain_key: ChainKey,

    /// Legacy confirmation-depth setting, retained for configuration compatibility. Production
    /// message signing always requires the finalized head; this setting cannot enable a fallback.
    pub block_confirmation_depth: u64,

    /// First Creditcoin L1 EVM block to scan on startup. When `None`, the listener starts at the
    /// current head and only signs future messages. Only consulted when there is no persisted scan
    /// cursor yet (see [`state_dir`](Self::state_dir)); once a cursor exists it takes precedence, so
    /// a restart resumes rather than replaying from `start_block`.
    pub start_block: Option<u64>,

    /// Base directory for durable Outbox cursors and pending messages. Each public attestor account
    /// gets its own subdirectory; different signers may safely share this base. An exclusive lock
    /// rejects duplicate processes for the same account and chain. Startup verifies writable,
    /// durable storage and migrates legacy shared state by replaying the configured history.
    /// Defaults to [`DEFAULT_STATE_DIR`]; must be a persistent volume to survive pod restarts.
    pub state_dir: PathBuf,

    /// Hard cap on distinct tracked `message_id` entries (anti-abuse — confluence §5.4).
    pub max_tracked_messages: usize,

    /// Drop partial vote aggregates older than this unless already complete (anti-abuse).
    pub vote_ttl: Duration,

    /// Destination `EOAValidator` (`IVoteValidator`). The single source of the authorized signer
    /// set and quorum threshold (`attestors()` / `threshold()`), and the `verifyingContract` of
    /// the message-vote EIP-712 domain. Gossip votes from signers outside its set are rejected.
    /// Required when `enabled`.
    pub vote_validator: Option<Address>,

    /// Destination Inbox that delivers this chain's messages. Message votes are EIP-712 bound to
    /// it (the `inbox` field of `MessageVote`), so a vote bundle cannot be replayed into another
    /// Inbox that accepts the same Outbox. Required when `enabled`; the attestor checks on-chain
    /// that this Inbox hands its votes to [`vote_validator`](Self::vote_validator).
    pub destination_inbox: Option<Address>,
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
            vote_validator: None,
            destination_inbox: None,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::disabled()
    }
}

impl Config {
    /// Reject an invalid startup configuration: out-of-range legacy confirmation depth, zero vote
    /// TTL / tracked-message cap, or a missing / zero validator, Inbox or destination RPC. Only enforced when
    /// `enabled` — a disabled config is always valid.
    pub fn validate(&self) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        if self.cc3_eth_rpc_url.is_none() {
            return Err("message attestation enabled but no Creditcoin EVM RPC URL".to_string());
        }
        if self.block_confirmation_depth == 0 {
            return Err("legacy block_confirmation_depth must be > 0".to_string());
        }
        // Preserve validation of the legacy field for configuration compatibility, even though
        // it no longer authorizes signing beyond the finalized head.
        if self.block_confirmation_depth > MAX_BLOCK_CONFIRMATION_DEPTH {
            return Err(format!(
                "legacy block_confirmation_depth {} exceeds the supported maximum {}",
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
        match self.vote_validator {
            None => {
                return Err(
                    "message attestation enabled but no validator_address (the destination \
                     EOAValidator that holds the attestor set)"
                        .to_string(),
                );
            }
            Some(validator) if validator == Address::ZERO => {
                return Err("validator_address is the zero address".to_string());
            }
            Some(_) => {}
        }
        match self.destination_inbox {
            None => {
                return Err(
                    "message attestation enabled but no inbox_address (message votes are \
                     EIP-712 bound to the destination Inbox)"
                        .to_string(),
                );
            }
            Some(inbox) if inbox == Address::ZERO => {
                return Err("inbox_address is the zero address".to_string());
            }
            Some(_) => {}
        }
        if self.destination_eth_rpc_url.is_none() {
            return Err("message attestation enabled but no destination EVM RPC URL".to_string());
        }
        Ok(())
    }
}

/// Default base directory for durable write-ability state. Matches the
/// persistent volume the attestor StatefulSet mounts (`/data`), so production needs no explicit
/// `state_dir`; local/dev runs override it in config.
pub const DEFAULT_STATE_DIR: &str = "/data";

/// Default for the legacy confirmation-depth field; no longer used as a signing boundary.
pub const DEFAULT_BLOCK_CONFIRMATION_DEPTH: u64 = 3;

/// Historical validation ceiling for the legacy `block_confirmation_depth` setting.
pub const MAX_BLOCK_CONFIRMATION_DEPTH: u64 = 100_000;

/// Reject an RPC URL whose scheme the EVM/substrate clients can't dial (audit P2-7). Accepts only
/// `http(s)` / `ws(s)`; `None` is accepted here (presence is enforced separately). Catches a fat-fingered `htto://…` / `file://…` at boot instead of at first RPC call.
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
            vote_validator: Some(address!("000000000000000000000000000000000000000b")),
            destination_inbox: Some(address!("000000000000000000000000000000000000000c")),
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
    fn requires_a_nonzero_validator() {
        let mut c = enabled_base();
        c.vote_validator = None;
        assert!(c.validate().is_err(), "missing validator");
        c.vote_validator = Some(Address::ZERO);
        assert!(c.validate().is_err(), "zero validator");
    }

    #[test]
    fn requires_a_nonzero_inbox() {
        let mut c = enabled_base();
        c.destination_inbox = None;
        assert!(c.validate().is_err(), "missing inbox");
        c.destination_inbox = Some(Address::ZERO);
        assert!(c.validate().is_err(), "zero inbox");
    }

    #[test]
    fn requires_a_destination_rpc() {
        let mut c = enabled_base();
        c.destination_eth_rpc_url = None;
        assert!(c.validate().is_err());
    }
}
