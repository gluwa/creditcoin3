//! Runtime metadata updater.
//!
//! Subxt's `OnlineClient` caches the runtime version + metadata at connect time and would
//! otherwise stay frozen on those bytes forever. Substrate runtime upgrades (`set_code`)
//! shift the cached encodings out from under us — at minimum we'd encode storage reads /
//! extrinsics against stale type info and start getting txpool rejections.
//!
//! This task subscribes to runtime upgrades via `OnlineClient::updater()` and, on every
//! new `Update`, applies it back into the shared client (Layer A) — *except* when the
//! Attestation pallet's metadata hash changes vs. the baseline captured at startup, which
//! means the runtime layout for the call we actually sign (`commit_attestation`) has
//! drifted from our compile-time-generated types (Layer B). In that case we return
//! `Error::RuntimeMetadata` so the supervisor cancels + drains, the process exits nonzero, k8s
//! reschedules, and CI rolls a binary built against the new metadata.
//!
//! Reconnect interaction: `shared.cc3.api()` loads the current `OnlineClient` via
//! `ArcSwap`. The `updater()` we obtain is bound to the snapshot at call time, so after a
//! reconnect we have to rebind to the *new* client's updater — which is what the outer
//! loop does whenever the underlying stream ends.

use std::sync::Arc;
use std::time::Duration;

use crate::error::Error;
use crate::shared::Shared;

const ATTESTATION_PALLET: &str = "Attestation";

pub async fn run(shared: Arc<Shared>) -> Result<(), Error> {
    // The baseline is the Attestation pallet hash from the *compile-time* metadata bundled
    // with this binary — that's what the statically-typed `cc_client::cc3::*` call sites
    // encode against. Drift detection is relative to that, not relative to whatever the
    // chain happened to have at startup.
    let compiled = match cc_client::compiled_metadata() {
        Ok(m) => m,
        Err(err) => {
            tracing::error!(
                ?err,
                "🛑 failed to decode bundled metadata — signalling shutdown"
            );
            return Err(Error::RuntimeMetadata(format!(
                "failed to decode bundled metadata: {err:?}"
            )));
        }
    };
    let baseline = match attestation_hash(&compiled) {
        Some(h) => h,
        None => {
            tracing::error!(
                pallet = ATTESTATION_PALLET,
                "🛑 pallet missing from bundled metadata — refusing to run"
            );
            return Err(Error::RuntimeMetadata(format!(
                "pallet {ATTESTATION_PALLET} missing from bundled metadata"
            )));
        }
    };
    tracing::info!(
        baseline = %hex::encode(baseline),
        "🧭 runtime updater baseline captured (from compiled metadata)"
    );

    loop {
        if shared.token.is_cancelled() {
            return Ok(());
        }

        // Bind the updater to whatever OnlineClient is currently in the ArcSwap. After a
        // reconnect, this snapshots the fresh client; the previous iteration's snapshot
        // (and its stream) is dropped at the end of the inner scope.
        let api = shared.cc3.api();
        let updater = api.updater();

        let mut stream = match updater.runtime_updates().await {
            Ok(s) => s,
            Err(err) => {
                tracing::warn!(
                    ?err,
                    "runtime updates subscription failed — retry after delay"
                );
                tokio::select! {
                    _ = shared.token.cancelled() => return Ok(()),
                    _ = tokio::time::sleep(Duration::from_secs(5)) => continue,
                }
            }
        };

        loop {
            tokio::select! {
                _ = shared.token.cancelled() => return Ok(()),
                next = stream.next() => match next {
                    None => {
                        tracing::info!("runtime updates stream ended — rebinding");
                        break;
                    }
                    Some(Err(err)) => {
                        tracing::warn!(?err, "runtime updates stream error — rebinding");
                        break;
                    }
                    Some(Ok(update)) => {
                        let spec_version = update.runtime_version().spec_version;
                        let live = match attestation_hash(update.metadata()) {
                            Some(h) => h,
                            None => {
                                tracing::error!(
                                    spec_version,
                                    pallet = ATTESTATION_PALLET,
                                    "🛑 pallet missing from new runtime metadata — signalling shutdown"
                                );
                                return Err(Error::RuntimeMetadata(format!(
                                    "pallet {ATTESTATION_PALLET} missing from runtime metadata at spec_version {spec_version}"
                                )));
                            }
                        };

                        if live != baseline {
                            tracing::error!(
                                spec_version,
                                baseline = %hex::encode(baseline),
                                live = %hex::encode(live),
                                "🛑 Attestation pallet metadata changed — binary needs rebuild; signalling shutdown"
                            );
                            return Err(Error::RuntimeMetadata(format!(
                                "Attestation pallet metadata changed at spec_version {spec_version} (baseline {}, live {}) — binary needs rebuild",
                                hex::encode(baseline),
                                hex::encode(live),
                            )));
                        }

                        match updater.apply_update(update) {
                            Ok(()) => {
                                tracing::info!(
                                    spec_version,
                                    "🔄 runtime metadata updated (Attestation pallet unchanged)"
                                );
                            }
                            Err(subxt::client::UpgradeError::SameVersion) => {
                                // Initial subscription replay; not an error.
                            }
                            Err(err) => {
                                tracing::warn!(spec_version, ?err, "apply_update failed");
                            }
                        }
                    }
                }
            }
        }
    }
}

fn attestation_hash(metadata: &subxt::Metadata) -> Option<[u8; 32]> {
    metadata
        .pallet_by_name(ATTESTATION_PALLET)
        .map(|p| p.hash())
}
