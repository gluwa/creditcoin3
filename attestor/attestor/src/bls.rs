//! BLS public-key store for the elected attestor set.
//!
//! Identical to the v1 store, with two changes:
//! - The inner map is behind a sync `parking_lot::RwLock` (we always do quick lookups, never
//!   any await while holding the lock).
//! - `note_attestors_elected` takes `&Arc<cc_client::Client>` directly. Because the new design
//!   shares one `Arc<Client>` across all tasks, reconnections done from any task become visible
//!   here automatically (the inner `ArcSwap` is shared).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use parking_lot::RwLock;
use tokio_util::sync::CancellationToken;

use attestor_primitives::ChainKey;

use crate::retry::with_retries;

pub struct BlsStore {
    chain_key: ChainKey,
    set: RwLock<ElectedSet>,
}

/// The elected attestor set as of the last election. `members` is every elected attestor, with or
/// without a BLS key on chain; `keys` only has those whose key could be fetched. Kept behind one
/// lock so a reader never sees the keys of one election next to the members of another.
#[derive(Default)]
struct ElectedSet {
    keys: HashMap<[u8; 32], bls_signatures::PublicKey>,
    members: HashSet<[u8; 32]>,
}

impl ElectedSet {
    fn new(
        keys: HashMap<[u8; 32], bls_signatures::PublicKey>,
        attestors: &[cc_client::AccountId32],
    ) -> Self {
        let members = attestors.iter().map(|a| *a.as_ref()).collect();
        Self { keys, members }
    }
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
            set: RwLock::new(ElectedSet::new(keys, attestors)),
        })
    }

    pub async fn note_attestors_elected(
        &self,
        cc3: &Arc<cc_client::Client>,
        token: &CancellationToken,
        attestors: &[cc_client::AccountId32],
    ) -> Result<(), cc_client::Error> {
        let next = fetch(cc3, token, self.chain_key, attestors).await?;
        *self.set.write() = ElectedSet::new(next, attestors);
        Ok(())
    }

    pub fn pubkey(&self, attestor: impl AsRef<[u8; 32]>) -> Option<bls_signatures::PublicKey> {
        self.set.read().keys.get(attestor.as_ref()).cloned()
    }

    /// `true` when `attestor` is the only elected member of the set. Such an attestor reaches
    /// quorum on its own vote and has nobody to gossip to, so a failed publish is expected rather
    /// than a sign of a broken mesh. Counts every elected member, including ones without a BLS
    /// key, so a set with a misconfigured peer is never mistaken for a solo one.
    pub fn is_sole_member(&self, attestor: impl AsRef<[u8; 32]>) -> bool {
        let set = self.set.read();
        set.members.len() == 1 && set.members.contains(attestor.as_ref())
    }

    #[cfg(test)]
    pub(crate) fn for_tests(chain_key: ChainKey, attestors: &[cc_client::AccountId32]) -> Self {
        Self {
            chain_key,
            set: RwLock::new(ElectedSet::new(HashMap::new(), attestors)),
        }
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
    let mut missing: Vec<&cc_client::AccountId32> = Vec::new();

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

        let Some(raw) = raw else {
            missing.push(attestor_id);
            continue;
        };
        let key = bls_signatures::PublicKey::from_bytes(&raw).map_err(|_| {
            cc_client::Error::from(subxt::Error::Other(format!(
                "invalid bls pubkey for {attestor_id}"
            )))
        })?;
        out.insert(id_bytes, key);
    }

    // The runtime requires a registered BLS key before an attestor can activate, so an active
    // attestor without one indicates chain-state drift or a race with deregistration. Their
    // votes cannot be verified locally: gossip from them is rejected (`UnknownAttestor`) and
    // they contribute nothing toward quorum on this node, effectively shrinking the committee.
    // Surface that loudly instead of silently skipping — but keep running: hard-failing here
    // would let a single misconfigured peer take down every other attestor.
    if !missing.is_empty() {
        tracing::error!(
            ?missing,
            fetched = out.len(),
            total = attestors.len(),
            "🕵️ active attestors without an on-chain BLS pubkey — their votes cannot be verified"
        );
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::BlsStore;

    fn id(byte: u8) -> cc_client::AccountId32 {
        cc_client::AccountId32::from([byte; 32])
    }

    #[test]
    fn sole_member_only_when_the_set_is_exactly_this_attestor() {
        let me = id(1);
        assert!(BlsStore::for_tests(0, &[me.clone()]).is_sole_member(&me));
        // Another member, even one we hold no BLS key for, means gossip is needed.
        assert!(!BlsStore::for_tests(0, &[me.clone(), id(2)]).is_sole_member(&me));
        // Elected alone, but it is someone else: we are not even in the set.
        assert!(!BlsStore::for_tests(0, &[id(2)]).is_sole_member(&me));
        assert!(!BlsStore::for_tests(0, &[]).is_sole_member(&me));
    }
}
