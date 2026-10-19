//! Finalized networking identities, loaded independently of gossip and refreshed off the swarm.

use std::{collections::HashSet, sync::Arc, time::Duration};

use anyhow::{ensure, Context as _};
use attestor_primitives::{peer_key_proof_message, AttestorId, ChainKey, PeerRegistry};
use libp2p::{identity, PeerId};
use parity_scale_codec::Encode as _;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::{error::Error, retry::with_retries, secret::AttestorSecret};

const REFRESH_INTERVAL: Duration = Duration::from_secs(15);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registry {
    pub genesis_hash: cc_client::H256,
    pub finalized_hash: cc_client::H256,
    pub finalized_number: u64,
    pub registration_required: bool,
    pub attestors: Vec<(AttestorId, Option<PeerId>)>,
}

impl Registry {
    pub fn from_chain(registry: PeerRegistry<cc_client::AccountId32>) -> anyhow::Result<Self> {
        ensure!(
            registry.attestors.len() <= common::constants::MAX_ATTESTORS,
            "peer registry exceeds the committee bound"
        );
        let mut controllers = HashSet::new();
        let mut peers = HashSet::new();
        let mut attestors = Vec::with_capacity(registry.attestors.len());
        for (controller, key) in registry.attestors {
            ensure!(
                controllers.insert(controller.0),
                "duplicate controller in peer registry"
            );
            let peer = key
                .map(|bytes| {
                    let public = identity::ed25519::PublicKey::try_from_bytes(&bytes)
                        .context("invalid Ed25519 key in finalized peer registry")?;
                    let peer = identity::PublicKey::from(public).to_peer_id();
                    ensure!(
                        peers.insert(peer),
                        "duplicate networking identity in peer registry"
                    );
                    Ok::<_, anyhow::Error>(peer)
                })
                .transpose()?;
            ensure!(
                !registry.registration_required || peer.is_some(),
                "active attestor {controller} has no required networking key"
            );
            attestors.push((AttestorId::from_public(controller.0), peer));
        }
        Ok(Self {
            genesis_hash: Default::default(),
            finalized_hash: Default::default(),
            finalized_number: 0,
            registration_required: registry.registration_required,
            attestors,
        })
    }

    pub fn peer(&self, attestor: &AttestorId) -> Option<PeerId> {
        self.attestors
            .iter()
            .find(|(id, _)| id == attestor)
            .and_then(|(_, peer)| *peer)
    }

    pub fn is_legacy_member(&self, attestor: &AttestorId) -> bool {
        !self.registration_required
            && self
                .attestors
                .iter()
                .any(|(id, peer)| id == attestor && peer.is_none())
    }

    pub fn registered_peers(&self) -> impl Iterator<Item = PeerId> + '_ {
        self.attestors.iter().filter_map(|(_, peer)| *peer)
    }
}

pub fn networking_keypair(secret: &AttestorSecret) -> anyhow::Result<identity::Keypair> {
    let mut seed = secret.to_seed_bytes_32();
    identity::Keypair::ed25519_from_bytes(&mut *seed).map_err(Into::into)
}

/// Publish a missing key automatically. Replacing an existing identity requires explicit
/// operator intent so an old instance restarting cannot undo a completed rotation.
pub async fn ensure_registered(
    cc3: &Arc<cc_client::Client>,
    token: &CancellationToken,
    chain_key: ChainKey,
    controller: &cc_client::AccountId32,
    keypair: &identity::Keypair,
    rotate: bool,
) -> Result<(), Error> {
    let public = keypair
        .public()
        .try_into_ed25519()
        .context("attestor networking identity must be Ed25519")
        .map_err(Error::Init)?
        .to_bytes();
    let existing = finalized_peer_key(cc3, token, chain_key, controller).await?;
    match registration_action(existing, public, rotate).map_err(Error::Init)? {
        RegistrationAction::Unchanged => return Ok(()),
        RegistrationAction::Submit => {}
    }
    let genesis = cc3.api().genesis_hash();
    let message =
        peer_key_proof_message(genesis.as_ref(), chain_key, &controller.encode(), &public);
    let proof: [u8; 64] = keypair
        .sign(&message)
        .context("sign networking-key proof")
        .map_err(Error::Init)?
        .try_into()
        .map_err(|_| Error::Init(anyhow::anyhow!("invalid Ed25519 signature length")))?;
    tracing::info!(peer_id = %keypair.public().to_peer_id(), chain_key, "registering attestor networking identity");
    with_retries(cc3, token, move |cc3| async move {
        cc3.set_attestor_peer_key(chain_key, public, proof, existing)
            .await
    })
    .await?;
    // A finalized reread catches a concurrent operator rotation before this instance starts.
    let observed = finalized_peer_key(cc3, token, chain_key, controller).await?;
    if observed != Some(public) {
        return Err(Error::Init(anyhow::anyhow!("finalized networking key changed during registration; refusing to start with a different identity")));
    }
    Ok(())
}

async fn finalized_peer_key(
    cc3: &Arc<cc_client::Client>,
    token: &CancellationToken,
    chain_key: ChainKey,
    controller: &cc_client::AccountId32,
) -> Result<Option<[u8; 32]>, Error> {
    let controller = controller.clone();
    with_retries(cc3, token, move |cc3| {
        let controller = controller.clone();
        async move {
            let (at, _) = cc3.finalized_head().await?;
            cc3.attestor_peer_key_at(chain_key, &controller, at).await
        }
    })
    .await
    .map_err(Into::into)
}

#[derive(Debug, PartialEq, Eq)]
enum RegistrationAction {
    Unchanged,
    Submit,
}

fn registration_action(
    existing: Option<[u8; 32]>,
    public: [u8; 32],
    rotate: bool,
) -> anyhow::Result<RegistrationAction> {
    match existing {
        Some(key) if key == public => Ok(RegistrationAction::Unchanged),
        Some(_) if !rotate => anyhow::bail!("on-chain networking key differs from this instance; use the matching --p2p-secret, or explicitly authorize rotation with --rotate-peer-key"),
        _ => Ok(RegistrationAction::Submit),
    }
}

pub async fn load(
    cc3: &Arc<cc_client::Client>,
    token: &CancellationToken,
    chain_key: ChainKey,
) -> Result<Registry, Error> {
    let (genesis, at, number, registry) = with_retries(cc3, token, move |cc3| async move {
        let genesis = cc3.api().genesis_hash();
        let (at, number) = cc3.finalized_head().await?;
        let registry = cc3.attestor_peer_registry_at(chain_key, at).await?;
        Ok((genesis, at, number, registry))
    })
    .await?;
    let mut registry = Registry::from_chain(registry).map_err(Error::Init)?;
    registry.genesis_hash = genesis;
    registry.finalized_hash = at;
    registry.finalized_number = number;
    Ok(registry)
}

/// RPC retries happen in their own supervised task; an outage cannot block swarm polling.
/// Keep the last qualified finalized view until a newer query succeeds.
pub async fn run(
    cc3: Arc<cc_client::Client>,
    token: CancellationToken,
    chain_key: ChainKey,
    sender: watch::Sender<Arc<Registry>>,
) -> Result<(), Error> {
    let mut interval = tokio::time::interval(REFRESH_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    interval.tick().await;
    loop {
        tokio::select! {
            _ = token.cancelled() => return Ok(()),
            _ = interval.tick() => {}
        }
        let registry = match load(&cc3, &token, chain_key).await {
            Ok(registry) => Arc::new(registry),
            Err(_) if token.is_cancelled() => return Ok(()),
            Err(error) => return Err(error),
        };
        let current = sender.borrow().clone();
        if !can_advance(&current, &registry).map_err(Error::Init)? {
            tracing::warn!(
                observed = registry.finalized_number,
                retained = current.finalized_number,
                "ignoring a lagging finalized peer registry"
            );
            continue;
        }
        sender.send_if_modified(|current| {
            if *current == registry {
                return false;
            }
            *current = registry;
            true
        });
    }
}

fn can_advance(current: &Registry, next: &Registry) -> anyhow::Result<bool> {
    ensure!(
        current.genesis_hash == next.genesis_hash,
        "network genesis changed while refreshing peer registry"
    );
    if next.finalized_number < current.finalized_number {
        return Ok(false);
    }
    ensure!(
        next.finalized_number != current.finalized_number || next == current,
        "inconsistent networking registry at the same finalized height"
    );
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(seed: u8) -> [u8; 32] {
        networking_keypair(&AttestorSecret::RawSeed([seed; 32]))
            .unwrap()
            .public()
            .try_into_ed25519()
            .unwrap()
            .to_bytes()
    }

    #[test]
    fn networking_identity_is_stable_and_matches_the_registered_public_key() {
        let pair = networking_keypair(&AttestorSecret::RawSeed([1; 32])).unwrap();
        let registry = Registry::from_chain(PeerRegistry {
            registration_required: true,
            attestors: vec![(cc_client::AccountId32([9; 32]), Some(key(1)))],
        })
        .unwrap();
        assert_eq!(
            registry.peer(&AttestorId::from_public([9; 32])),
            Some(pair.public().to_peer_id())
        );
    }

    #[test]
    fn missing_keys_are_explicit_during_rollout_and_rejected_when_required() {
        let input = PeerRegistry {
            registration_required: false,
            attestors: vec![(cc_client::AccountId32([9; 32]), None)],
        };
        let registry = Registry::from_chain(input.clone()).unwrap();
        assert!(registry.is_legacy_member(&AttestorId::from_public([9; 32])));
        assert!(Registry::from_chain(PeerRegistry {
            registration_required: true,
            ..input
        })
        .is_err());
    }

    #[test]
    fn duplicate_controllers_and_peer_ids_are_rejected() {
        let a = cc_client::AccountId32([1; 32]);
        let b = cc_client::AccountId32([2; 32]);
        for attestors in [
            vec![(a.clone(), Some(key(1))), (a, Some(key(2)))],
            vec![
                (b.clone(), Some(key(1))),
                (cc_client::AccountId32([3; 32]), Some(key(1))),
            ],
        ] {
            assert!(Registry::from_chain(PeerRegistry {
                registration_required: true,
                attestors
            })
            .is_err());
        }
    }

    #[test]
    fn an_old_instance_cannot_silently_undo_a_networking_key_rotation() {
        assert_eq!(
            registration_action(None, key(1), false).unwrap(),
            RegistrationAction::Submit
        );
        assert_eq!(
            registration_action(Some(key(1)), key(1), false).unwrap(),
            RegistrationAction::Unchanged
        );
        assert!(registration_action(Some(key(2)), key(1), false).is_err());
        assert_eq!(
            registration_action(Some(key(2)), key(1), true).unwrap(),
            RegistrationAction::Submit
        );
    }

    #[test]
    fn lagging_rpc_cannot_restore_a_rotated_identity() {
        let mut old = Registry::from_chain(PeerRegistry {
            registration_required: true,
            attestors: vec![(cc_client::AccountId32([9; 32]), Some(key(1)))],
        })
        .unwrap();
        old.finalized_number = 10;
        let mut current = Registry::from_chain(PeerRegistry {
            registration_required: true,
            attestors: vec![(cc_client::AccountId32([9; 32]), Some(key(2)))],
        })
        .unwrap();
        current.finalized_number = 20;
        assert!(!can_advance(&current, &old).unwrap());
        old.finalized_number = 20;
        assert!(can_advance(&current, &old).is_err());
        assert!(can_advance(&current, &current).unwrap());
    }

    #[test]
    fn libp2p_proof_verifies_with_the_runtimes_ed25519_implementation() {
        use sp_core::Pair as _;
        let pair = networking_keypair(&AttestorSecret::RawSeed([7; 32])).unwrap();
        let public = pair.public().try_into_ed25519().unwrap().to_bytes();
        let message = peer_key_proof_message(&[8; 32], 1, &[9; 32], &public);
        let signature: [u8; 64] = pair.sign(&message).unwrap().try_into().unwrap();
        assert!(sp_core::ed25519::Pair::verify(
            &sp_core::ed25519::Signature::from_raw(signature),
            &message,
            &sp_core::ed25519::Public::from_raw(public)
        ));
        let wrong_domain = peer_key_proof_message(&[8; 32], 2, &[9; 32], &public);
        assert!(!sp_core::ed25519::Pair::verify(
            &sp_core::ed25519::Signature::from_raw(signature),
            &wrong_domain,
            &sp_core::ed25519::Public::from_raw(public)
        ));
    }
}
