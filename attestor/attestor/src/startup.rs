//! Startup-time helpers.
//!
//! Each helper is just an `async fn` that returns `Result<…, Error>`. Cancellation is handled
//! by the caller via `tokio::select!` on the cancellation token — these helpers themselves
//! don't sprinkle `ctrl_c` arms everywhere like v1 did.

use std::sync::Arc;

use anyhow::Context as _;
use bls_signatures::Serialize as _;
use futures::{StreamExt as _, TryStreamExt as _};

use attestor_primitives::{AttestorStatus, ChainKey};
use cc_client::{AccountId32, Client};

use crate::error::Error;
use crate::secret::RpcSecret;

/// Loop until both RPCs accept a WebSocket connection. Returns once both are reachable.
pub async fn wait_for_endpoints(
    url_eth: &RpcSecret,
    url_cc3: &RpcSecret,
) -> Result<(), Error> {
    use common::constants::RETRY_DELAY;

    async fn poke(label: &str, url: &RpcSecret) {
        loop {
            match tokio_tungstenite::connect_async(url.as_ref()).await {
                Ok(_) => return,
                Err(err) => {
                    tracing::info!(%url, %err, "🛜 waiting for {label} ws...");
                    tokio::time::sleep(RETRY_DELAY).await;
                }
            }
        }
    }

    poke("Eth", url_eth).await;
    poke("CC3", url_cc3).await;
    Ok(())
}

/// Register a BLS key with the runtime if our status is `Idle`.
pub async fn register_bls(
    chain_key: ChainKey,
    cc3: &Arc<Client>,
    account_id: &AccountId32,
    bls_key: &bls_signatures::PrivateKey,
) -> Result<(), Error> {
    let status = cc3.get_attestor_status(chain_key).await?;
    if status != Some(AttestorStatus::Idle) {
        tracing::info!(?status, %account_id, "ℹ️ skipping attest() — already registered");
        return Ok(());
    }

    // bls_signatures uses BLS12-381 minimal-pubkey-size: public key is 48 bytes (G1),
    // signature is 96 bytes (G2). The runtime's `start_attesting` extrinsic expects them in
    // that same order (pubkey: [u8; 48], pop: [u8; 96]).
    let public: [u8; 48] = bls_key.public_key().as_bytes()[..]
        .try_into()
        .context("bls public key length")
        .map_err(Error::Init)?;
    let pop: [u8; 96] = bls_key.sign(public).as_bytes()[..]
        .try_into()
        .context("bls signature length")
        .map_err(Error::Init)?;

    tracing::info!(%account_id, "📝 Submitting attest() to transition Idle → Waiting");
    cc3.start_attesting(chain_key, public, pop).await?;
    tracing::info!(%account_id, "✅ attest() submitted");
    Ok(())
}

/// Wait until `account_id` is in the active attestor set. Listens to `AttestorsElected`.
pub async fn wait_for_eligible(
    chain_key: ChainKey,
    cc3: &Arc<Client>,
    account_id: &AccountId32,
) -> Result<Vec<AccountId32>, Error> {
    use cc_client::attestation::CcEvent;

    let mut attestors = cc3.get_attestor_active_set(chain_key).await?;
    if attestors.contains(account_id) {
        tracing::info!(%account_id, "☀️ already eligible");
        return Ok(attestors);
    }

    let config = stream::cc3::ConfigBuilder::new()
        .with_cc3((**cc3).clone())
        .with_chain_key(chain_key)
        .build();
    let mut events = stream::cc3::StreamCC3::new(config)
        .await
        .map_err(Error::Init)?;

    let mut tick = tokio::time::interval(std::time::Duration::from_secs(5));
    loop {
        tokio::select! {
            Some(mut batch) = events.next() => {
                while let Some(event) = batch.try_next().await? {
                    if let CcEvent::AttestorsElected(key, list) = event {
                        if key == chain_key && list.contains(account_id) {
                            attestors = list;
                            tracing::info!(%account_id, "☀️ elected");
                            return Ok(attestors);
                        }
                    }
                }
            }
            _ = tick.tick() => {
                tracing::info!(%account_id, "⏲️ waiting on election...");
            }
        }
    }
}

/// Look up the starting attestation point.
///
/// Returns `(genesis_height, start_attestation)`:
/// - `genesis_height`: the chain's attestation-genesis block (from runtime).
/// - `start_attestation`: `Some` if there's a previously-finalized attestation or checkpoint on
///   chain; `None` if we're genuinely starting from genesis.
pub async fn fetch_start_point(
    chain_key: ChainKey,
    cc3: &Arc<Client>,
) -> Result<
    (
        attestor_primitives::Height,
        Option<crate::shared::AttestationInfo>,
    ),
    Error,
> {
    let genesis = cc3.get_attestation_chain_genesis_block_number(chain_key).await?;

    let start = if let Some(last_digest) = cc3.fetch_last_digest(chain_key).await? {
        let last = cc3.get_attestation_by_digest(chain_key, last_digest).await?
            .expect("last digest must resolve to an attestation");
        Some(crate::shared::AttestationInfo {
            height: last.header_number(),
            digest: last.digest(),
        })
    } else {
        cc3.get_last_checkpoint(chain_key).await?.map(|cp| {
            crate::shared::AttestationInfo {
                height: cp.block_number,
                digest: cp.digest,
            }
        })
    };

    Ok((genesis, start))
}
