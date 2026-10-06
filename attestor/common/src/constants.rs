//! Common constants used throughout the attestor code.

/// Max channel capacity for bounded thread communication using [broadcast] channels.
///
/// [broadcast]: tokio::sync::broadcast
pub const CAPACITY_CHANNEL: usize = 100;

/// Finality timeout before an attestation vote is assumed to have failed.
///
/// Every attestor submits its proof on quorum (the `PrevalidateAttestationCommit` runtime
/// extension dedups the race at txpool admission), so there is no leader election to wait on.
/// A submitted height can still fail to finalize — a stuck or slow chain, a dropped tx — and
/// attestors have no positive signal for that. As a failsafe they wait for finality for a max
/// duration of [`ATTESTATION_TIMEOUT`], after which they assume the height stalled and retry.
///
/// Sized for usc-devnet observed worst-case: cc3 BlockAttested arrival is typically 30–100 s
/// after quorum is reached. 30 s was too tight — it produced spurious "🏃 finalization timed
/// out" WARN logs on every slow-finalizing height even though the chain caught up on its own.
/// 120 s covers the long-tail without giving up on a genuinely stuck height.
pub const ATTESTATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// General delay used to retry network connections.
pub const RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(2);

/// Warmup pause after observing our own election before attestation tasks start.
///
/// Peers process the same `AttestorsElected` event on their own schedule: they must refresh
/// their BLS key store and admit our peer id before our gossip verifies. Starting to produce
/// and broadcast immediately gets those first votes rejected as `UnknownAttestor` (and, with
/// peer scoring active, penalizes us for them). Two CC3 block times (~15 s each in production)
/// gives the committee a comfortable window to catch up.
pub const POST_ELECTION_WARMUP: std::time::Duration = std::time::Duration::from_secs(30);

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
pub const MAX_REBROADCAST: attestor_primitives::Height = 10;

pub const MAX_CATCHUP: std::num::NonZero<attestor_primitives::Height> =
    std::num::NonZero::new(500).unwrap();

pub const MAX_CONCURRENT_RPC_CALLS: std::num::NonZeroUsize = std::num::NonZero::new(10).unwrap();

/// Upper bound on the on-chain attestor committee, mirroring the runtime's
/// `Attestation::MaxAttestors`. The p2p pending-vote buffer sizes its per-height cap from this so
/// it can hold one early (pre-local-data) vote per possible attestor and never drop one that is
/// later needed to reach quorum — a dropped vote is `Ignore`d under gossipsub Strict validation,
/// which marks it seen and never redelivers it. Keep in sync with the runtime constant.
pub const MAX_ATTESTORS: usize = 100;

/// Minimum balance required for an attestor to operate.
/// This is equivalent to 1 CTC.
pub const MIN_BALANCE: u128 = 1_000_000_000_000_000_000;
