//! A [chain listener] responsible for re-broadcasting past attestations to the [p2p worker] at a
//! set interval to maintain network liveness.
//!
//! [chain listener]: crate::chain_listener
//! [p2p worker]: crate::worker::p2p

use crate::prelude::*;

// -------------------------------------- [ Configuration ] ------------------------------------ //

#[derive(Debug, attestor_macro::Builder)]
/// Configuration options for the rebroadcast [chain listener].
///
/// [chain listener]: crate::chain_listener
pub struct Config {
    /// Interval, in **seconds**, at which past attestations are rebroadcasted.
    rebroadcast_interval: std::num::NonZeroU64,
    attestation_interval: std::num::NonZero<common::types::Height>,
    start_height: common::types::Height,
}

// ------------------------------------- [ Chain Listener ] ------------------------------------ //

/// Rebroadcast [chain listener], responsible for re-broadcasting attestations to the [p2p worker]
/// at a set interval.
///
/// [chain listener]: crate::chain_listener
/// [p2p worker]: crate::worker::p2p
pub(crate) struct Rebroadcast {
    // BROADCAST DATA
    start: common::types::Height,
    catchup: super::Catchup,
    interval: tokio::time::Interval,
    broadcasting: bool,

    // CHAIN DATA
    attestation_interval: std::num::NonZero<common::types::Height>,
    start_height: common::types::Height,
}

impl Rebroadcast {
    /// Creates a new Rebroadcast [chain listener].
    ///
    /// [chain listener]: crate::chain_listener
    #[tracing::instrument(skip_all, level = "debug")]
    pub async fn new(config: Config) -> Self {
        let duration = std::time::Duration::from_secs(config.rebroadcast_interval.get());
        let duration_pretty = util::display::DurationPretty(duration);

        tracing::info!("🛜 Staring Rebroadcast listener");
        tracing::info!(%duration_pretty, "🛜  with");

        let mut interval = tokio::time::interval(duration_pretty.0);
        interval.tick().await;

        Self {
            start: config.start_height,
            catchup: super::Catchup {
                start: config.start_height,
                stop: config.start_height,
            },
            interval,
            broadcasting: false,

            attestation_interval: config.attestation_interval,
            start_height: config.start_height,
        }
    }

    /// Returns the next block to be rebroadcasted after the time to rebroadcast elapses. Returns
    /// [`None`] if no block has been produced yet.
    pub async fn next(&mut self) -> Option<common::types::Height> {
        if !self.broadcasting {
            self.interval.tick().await;
            self.broadcasting = true;
            tracing::info!("🔁 Re-broadcasting attestations");
        }

        // NOTE: RATE LIMIT
        //
        // We cap the number of attestations which can be rebroadcasted at once to avoid DOSing the
        // network.
        let size = self.catchup.start.saturating_sub(self.start);
        let size_max = common::constants::MAX_REBROADCAST * self.attestation_interval.get();

        if self.catchup.start < self.catchup.stop && size < size_max {
            let n = self.catchup.start;
            self.catchup.start += self.attestation_interval.get();
            return Some(n);
        }

        self.catchup.start = self.start;
        self.broadcasting = false;

        None
    }
}

// ----------------------------------------- [ Events ] ---------------------------------------- //

// Handling in response to execution chain events.
impl Rebroadcast {
    /// A new attestation has reached finality on the execution chain.
    ///
    /// If we are re-broadcasting attestations, we need to make sure we do not re-broadcast this
    /// attestation.
    pub fn note_attestation_finalization(&mut self, latest_attestation_cc3: common::types::Height) {
        let start_new = latest_attestation_cc3 + self.attestation_interval.get();
        if self.catchup.start < start_new {
            self.catchup.start = start_new;
        }
        self.start = start_new;
    }

    /// A new attestation has been produced by the [production worker]. Marks it as ready to be
    /// rebroadcasted.
    ///
    /// [production worker]: crate::worker::production
    pub fn note_attestation_production(&mut self, latest_attestation_eth: common::types::Height) {
        if self.catchup.stop < latest_attestation_eth {
            self.catchup.stop = latest_attestation_eth + self.attestation_interval.get();
        }
    }

    pub fn note_attestation_interval_change(
        &mut self,
        interval_new: std::num::NonZero<common::types::Height>,
        attestation_latest_cc3: Option<common::types::Height>,
    ) {
        let start_new = attestation_latest_cc3.unwrap_or(self.start_height);

        self.attestation_interval = interval_new;
        self.catchup = super::Catchup {
            start: start_new,
            stop: start_new,
        };
        self.start = start_new;
    }
}
