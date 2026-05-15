//! Shared state — held by every task as `Arc<Shared>`.
//!
//! The single most important property of this struct vs. the v1 design: there is **one**
//! `Arc<cc_client::Client>` here. Every task holds the same `Arc`, so when one task calls
//! `cc3.reconnect()` the underlying `ArcSwap` swap is visible to every other task immediately.
//! That's the fix for the reconnect-data-duplication bug.
//!
//! The other notable additions vs. v1 are:
//!
//! - `can_attest`: `watch::Sender<bool>` so toggle events actually wake consumers
//!   (the legacy `Arc<AtomicBool>` couldn't).
//! - `latest_finalized`: `watch::Sender<AttestationInfo>` so the validation task can
//!   `wait_for(|info| info.height >= X)` instead of subscribing to a second CC3 stream.
//! - `gossip_tx`: `mpsc::Sender<Vote>` (not broadcast) — one consumer (p2p), no fan-out.
//! - `token`: a root `CancellationToken`. Tasks derive child tokens; `select!` on
//!   `token.cancelled()` instead of bespoke `Notify` / `Interrupt::Stop` plumbing.

use std::num::NonZero;
use std::sync::Arc;

use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

use attestor_primitives::{ChainKey, Digest, Height};

use crate::bls::BlsStore;
use crate::proof_cache::ProofCache;

/// What every task needs.
pub struct Shared {
    pub name: String,
    pub chain_key: ChainKey,
    pub account_id: cc_client::AccountId32,
    pub attestor_id: attestor_primitives::AttestorId,

    pub signer: cc_client::signer::CC3Signer,
    pub bls_key: bls_signatures::PrivateKey,

    pub cc3: Arc<cc_client::Client>,
    pub eth: eth::Client,

    pub bls_store: Arc<BlsStore>,
    pub metrics: metrics::Metrics,

    pub pool_send: attestor_pool::Sender,
    pub gossip_tx: mpsc::Sender<crate::vote::Vote>,

    pub can_attest_tx: watch::Sender<bool>,
    pub can_attest_rx: watch::Receiver<bool>,

    /// Latest BlockAttested observed on cc3. `None` until the first BlockAttested event is
    /// processed by the production task. Validation reads this to wait for a height to finalize
    /// without subscribing to cc3 itself.
    pub latest_finalized_tx: watch::Sender<Option<AttestationInfo>>,
    pub latest_finalized_rx: watch::Receiver<Option<AttestationInfo>>,

    /// Latest height for which production has cached local AttestationData. `None` until the
    /// first local emit. The p2p task subscribes to this so it can drain any pending votes
    /// (received from peers before we had local data) and retry verification.
    pub local_produced_tx: watch::Sender<Option<Height>>,
    pub local_produced_rx: watch::Receiver<Option<Height>>,

    pub proof_cache: Arc<ProofCache>,

    pub interval_attestation: parking_lot::RwLock<NonZero<Height>>,
    pub maturity_delay: u64,
    pub start_height: Height,
    pub genesis: Height,

    pub token: CancellationToken,
}

/// Stored in the `latest_finalized` watch channel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AttestationInfo {
    pub height: Height,
    pub digest: Digest,
}

impl Shared {
    /// Cheap typed accessor.
    pub fn attestation_interval(&self) -> NonZero<Height> {
        *self.interval_attestation.read()
    }
}
