//! Submitter-side continuity-proof cache, shared between production and validation.
//!
//! Production builds a continuity proof for each height it attests to, and stashes it here keyed
//! by `(height, digest)`. When a quorum is reached on `(height, digest)`, the validation task
//! looks up the proof here. If the digest matches the local one, lookup succeeds; if it doesn't
//! (i.e. someone else's fork won the vote), the validation task must rebuild a proof to suit the
//! remote digest — which today means it can't submit and should drop that quorum.
//!
//! The cache trims itself: old entries are dropped on `note_finalized(height)`.

use std::collections::BTreeMap;
use std::sync::Arc;

use parking_lot::Mutex;

use attestor_primitives::{Digest, Height};

use crate::vote::LocalAttestationData;

#[derive(Clone, Debug)]
pub struct CachedProof {
    pub attestation_data: attestor_primitives::AttestationData<Digest>,
    pub continuity_proof: attestor_primitives::block::ContinuityProof,
}

#[derive(Default)]
pub struct ProofCache {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    by_height: BTreeMap<Height, BTreeMap<Digest, CachedProof>>,
    local_data_by_height: BTreeMap<Height, LocalAttestationData>,
}

impl ProofCache {
    pub fn new() -> Arc<Self> { Arc::new(Self::default()) }

    /// Insert a freshly produced (digest, proof) pair. Also caches the serialized AttestationData
    /// for use by the vote verifier.
    pub fn insert(
        &self,
        attestation_data: attestor_primitives::AttestationData<Digest>,
        continuity_proof: attestor_primitives::block::ContinuityProof,
    ) {
        let height = attestation_data.header_number;
        let digest = attestation_data.digest();
        let serialized = attestation_data.serialize();

        let mut inner = self.inner.lock();
        inner.local_data_by_height.insert(
            height,
            LocalAttestationData { serialized, digest },
        );
        inner
            .by_height
            .entry(height)
            .or_default()
            .insert(digest, CachedProof {
                attestation_data,
                continuity_proof,
            });
    }

    pub fn get(&self, height: Height, digest: Digest) -> Option<CachedProof> {
        self.inner.lock().by_height.get(&height)?.get(&digest).cloned()
    }

    /// Returns the locally signed AttestationData at `height`, if production has produced one
    /// already. Used by the p2p task to verify incoming votes against our own data.
    pub fn local_data(&self, height: Height) -> Option<LocalAttestationData> {
        self.inner.lock().local_data_by_height.get(&height).cloned()
    }

    /// Drop everything at or below `height`.
    pub fn note_finalized(&self, height: Height) {
        let mut inner = self.inner.lock();
        inner.by_height = inner.by_height.split_off(&(height.saturating_add(1)));
        inner.local_data_by_height = inner.local_data_by_height.split_off(&(height.saturating_add(1)));
    }

    pub fn clear(&self) {
        let mut inner = self.inner.lock();
        inner.by_height.clear();
        inner.local_data_by_height.clear();
    }
}
