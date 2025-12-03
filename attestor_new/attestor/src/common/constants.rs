//! Common constants used throughout the attestor code.

/// Max channel capacity for bounded thread communication using [broadcast] channels.
///
/// [broadcast]: tokio::sync::broadcast
pub const CAPACITY_CHANNEL: usize = 100;

/// Finality timeout before an attestation vote is considered as having failed.
///
/// Since attestation submission leader election takes place on a round-vrf basis, it is
/// possible for no leader to be elected this way. Since no consensus is made on the specific set
/// of leaders being elected, and this election is probabilistic, attestors have no way of knowing
/// when an election fails. As a failcase, attestors will wait for finality to conclude for a max
/// duration of [`ATTESTATION_TIMEOUT`], after which they will consider that no leader has been
/// elected and move on to the next height.
pub const ATTESTATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Currently we only support attesting to Ethereum, which has probabilistic finality. To avoid the
/// risk of attesting to a block which becomes invalidated as part of a reorg, we only attest to
/// data which is at least [`ATTESTATION_FINALIZATION_LAG`] blocks in the past.
pub const ATTESTATION_FINALIZATION_LAG: super::types::Height = 10;

/// General delay used to retry network connections.
pub const RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(2);
