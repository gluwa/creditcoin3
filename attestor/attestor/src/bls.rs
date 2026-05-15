//! BLS public-key store for the elected attestor set.
//!
//! Identical to the v1 store, with two changes:
//! - The inner map is behind a sync `parking_lot::RwLock` (we always do quick lookups, never
//!   any await while holding the lock).
//! - `note_attestors_elected` takes `&Arc<cc_client::Client>` directly. Because the new design
//!   shares one `Arc<Client>` across all tasks, reconnections done from any task become visible
//!   here automatically (the inner `ArcSwap` is shared).

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use tokio_util::sync::CancellationToken;

use attestor_primitives::ChainKey;

use crate::retry::with_retries;

pub struct BlsStore {
    chain_key: ChainKey,
    keys: RwLock<HashMap<[u8; 32], bls_signatures::PublicKey>>,
}

impl BlsStore {
    pub async fn new(
        cc3: &Arc<cc_client::Client>,
        token: &CancellationToken,
        chain_key: ChainKey,
        attestors: &[cc_client::AccountId32],
    ) -> Result<Self, cc_client::Error> {
        let keys = fetch(cc3, token, chain_key, attestors).await?;
        Ok(Self {
            chain_key,
            keys: RwLock::new(keys),
        })
    }

    pub async fn note_attestors_elected(
        &self,
        cc3: &Arc<cc_client::Client>,
        token: &CancellationToken,
        attestors: &[cc_client::AccountId32],
    ) -> Result<(), cc_client::Error> {
        let next = fetch(cc3, token, self.chain_key, attestors).await?;
        *self.keys.write() = next;
        Ok(())
    }

    pub fn pubkey(&self, attestor: impl AsRef<[u8; 32]>) -> Option<bls_signatures::PublicKey> {
        self.keys.read().get(attestor.as_ref()).cloned()
    }
}

async fn fetch(
    cc3: &Arc<cc_client::Client>,
    token: &CancellationToken,
    chain_key: ChainKey,
    attestors: &[cc_client::AccountId32],
) -> Result<HashMap<[u8; 32], bls_signatures::PublicKey>, cc_client::Error> {
    use bls_signatures::Serialize as _;

    let mut out = HashMap::with_capacity(attestors.len());

    for attestor_id in attestors {
        let id_bytes: [u8; 32] = *attestor_id.as_ref();
        let raw = with_retries(cc3, token, |cc3| {
            let q = cc_client::Client::runtime_api()
                .attestor_api()
                .attestor_bls_pubkey(chain_key, id_bytes.into());
            async move {
                let runtime = cc3.api().runtime_api().at_latest().await?;
                let v = runtime.call(q).await.map_err(cc_client::Error::from)?;
                Ok::<_, cc_client::Error>(v)
            }
        })
        .await?;

        let Some(raw) = raw else { continue; };
        let key = bls_signatures::PublicKey::from_bytes(&raw).map_err(|_| {
            cc_client::Error::from(subxt::Error::Other(format!(
                "invalid bls pubkey for {attestor_id}"
            )))
        })?;
        out.insert(id_bytes, key);
    }

    Ok(out)
}
