//! Hook invoked after a successful `commit_attestation` with the **eligible** signer set
//! (active attestors counted toward the BLS majority). Used by the runtime to credit
//! attest-coin rewards per committed vote.

use attestor_primitives::ChainKey;

/// Observes successful attestation commits. Default: [`NoopCommittedAttestationObserver`].
pub trait CommittedAttestationObserver<AccountId> {
    /// `eligible_signers` are attestor **operator** `AccountId`s (not stashes) that were both
    /// listed on the attestation and in the active set; they are the set that passed the
    /// majority threshold before BLS verification.
    fn on_committed_eligible(chain_key: ChainKey, eligible_signers: &[AccountId]);
}

/// No-op implementation (tests / runtimes without reward wiring).
pub struct NoopCommittedAttestationObserver;
impl<AccountId> CommittedAttestationObserver<AccountId> for NoopCommittedAttestationObserver {
    fn on_committed_eligible(_chain_key: ChainKey, _eligible_signers: &[AccountId]) {}
}
