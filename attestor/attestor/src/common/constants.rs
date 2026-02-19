//! Common constants used throughout the attestor code.

use crate::prelude::*;

/// Max channel capacity for bounded thread communication using [broadcast] channels.
///
/// [broadcast]: tokio::sync::broadcast
pub const CAPACITY_CHANNEL: usize = 100;

/// Finality timeout before an attestation vote is assumed to have failed.
///
/// Since attestation submission leader election takes place on a round-vrf basis, it is
/// possible for no leader to be elected. Since no consensus is made on the specific set of leaders
/// being elected, and this election is probabilistic, attestors have no way of knowing when an
/// election fails. As a failsafe, attestors will wait for finality to conclude for a max duration
/// of [`ATTESTATION_TIMEOUT`], after which they will assume that no leader has been elected and
/// retry their elegibility check with different parameters.
pub const ATTESTATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Currently we only support attesting to Ethereum, which has probabilistic finality. To avoid the
/// risk of attesting to a block which becomes invalidated as part of a reorg, we only attest to
/// data which is at least [`ATTESTATION_FINALIZATION_LAG`] blocks in the past.
pub const ATTESTATION_FINALIZATION_LAG: super::types::Height = 10;

/// General delay used to retry network connections.
pub const RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(2);

/// Default P2P port for libp2p networking.
///
/// This port is used when no explicit P2P port is configured via CLI args, environment variables,
/// or config file. Port 9000 is chosen as it's commonly available and suitable for Kubernetes
/// LoadBalancer services.
pub const DEFAULT_P2P_PORT: u16 = 9000;

/// Default port used to expose the attestor API in the [`api worker`].
///
/// [`api worker`]: crate::worker::api
pub const DEFAULT_API_PORT: u16 = 9100;

/// Header used for the `/metrics` enpoint in the [`api worker`].
///
/// [`api worker`]: crate::worker::api
pub const METRICS_HEADER: &str = "application/openmetrics-text; version=1.0.0; charset=utf-8";

/// Max number of attestations which can be rebroadcasted ahead of chain finality.
pub const MAX_REBROADCAST: common::types::Height = 10;

pub const MAX_CATCHUP: std::num::NonZero<common::types::Height> =
    std::num::NonZero::new(5).unwrap();

pub const MAX_CONCURRENT_RPC_CALLS: usize = 10;

pub const WORKER_COUNT: usize = 4;
