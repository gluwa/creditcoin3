//! Finalized networking identities, loaded independently of gossip and refreshed off the swarm.

use std::{collections::HashSet, future::Future, sync::Arc, time::Duration};

use anyhow::{ensure, Context as _};
use attestor_primitives::{peer_key_proof_message, AttestorId, ChainKey, PeerRegistry};
use libp2p::{identity, PeerId};
use parity_scale_codec::Encode as _;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::{error::Error, retry::with_retries, secret::AttestorSecret};

const REFRESH_INTERVAL: Duration = Duration::from_secs(120);
const LAGGING_RETRY_INTERVAL: Duration = Duration::from_secs(1);

/// Coalesced invalidation from the existing finalized-event stream. The height prevents a
/// lagging RPC from satisfying a refresh with state from before the event that requested it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RefreshRequest {
    finalized_number: u64,
    reconnect_count: u64,
}

pub fn request_refresh(
    sender: &watch::Sender<Option<RefreshRequest>>,
    finalized_number: u64,
    reconnect_count: u64,
    registry_changed: bool,
) {
    sender.send_if_modified(|current| {
        if current
            .is_some_and(|request| !registry_changed && request.reconnect_count == reconnect_count)
        {
            return false;
        }
        let next = Some(RefreshRequest {
            finalized_number: finalized_number
                .max(current.map_or(0, |request| request.finalized_number)),
            reconnect_count,
        });
        if *current == next {
            return false;
        }
        *current = next;
        true
    });
}

pub fn affects_registry(event: &cc_client::attestation::CcEvent) -> bool {
    use cc_client::attestation::CcEvent;
    matches!(
        event,
        CcEvent::PeerRegistryChanged(_)
            | CcEvent::AttestorsElected(..)
            | CcEvent::AttestorActivated(..)
            | CcEvent::AttestorChilled(..)
            | CcEvent::AttestorKicked(..)
            | CcEvent::RandomnessChanged(_)
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registry {
    pub genesis_hash: cc_client::H256,
    pub finalized_hash: cc_client::H256,
    pub finalized_number: u64,
    pub registration_required: bool,
    pub attestors: Vec<(AttestorId, Option<PeerId>)>,
    pub waiting_attestors: Vec<(AttestorId, Option<PeerId>)>,
}

impl Registry {
    /// Freshness still advances on an unchanged snapshot; only reservation rebuilding ignores it.
    pub fn same_reservations(&self, other: &Self) -> bool {
        self.registration_required == other.registration_required
            && self.attestors == other.attestors
            && self.waiting_attestors == other.waiting_attestors
    }

    pub fn from_chain(registry: PeerRegistry<cc_client::AccountId32>) -> anyhow::Result<Self> {
        ensure!(
            registry.attestors.len() + registry.waiting_attestors.len()
                <= common::constants::MAX_ATTESTORS,
            "peer registry exceeds the committee bound"
        );
        let mut controllers = HashSet::new();
        let mut peers = HashSet::new();
        let active_count = registry.attestors.len();
        let mut attestors = Vec::new();
        for (controller, key) in registry
            .attestors
            .into_iter()
            .chain(registry.waiting_attestors)
        {
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
                "active or waiting attestor {controller} has no required networking key"
            );
            attestors.push((AttestorId::from_public(controller.0), peer));
        }
        let waiting_attestors = attestors.split_off(active_count);
        Ok(Self {
            genesis_hash: Default::default(),
            finalized_hash: Default::default(),
            finalized_number: 0,
            registration_required: registry.registration_required,
            attestors,
            waiting_attestors,
        })
    }

    pub fn peer(&self, attestor: &AttestorId) -> Option<PeerId> {
        self.members()
            .find(|(id, _)| id == attestor)
            .and_then(|(_, peer)| *peer)
    }

    pub fn is_legacy_member(&self, attestor: &AttestorId) -> bool {
        !self.registration_required
            && self
                .members()
                .any(|(id, peer)| id == attestor && peer.is_none())
    }

    pub fn registered_peers(&self) -> impl Iterator<Item = PeerId> + '_ {
        self.members().filter_map(|(_, peer)| *peer)
    }

    fn members(&self) -> impl Iterator<Item = &(AttestorId, Option<PeerId>)> {
        self.attestors.iter().chain(&self.waiting_attestors)
    }

    /// A public seed needs a registered remote peer, except when its own registered key is
    /// the entire finalized active/waiting set. A partial larger set still needs remote trust.
    pub fn has_bootstrap_peers(&self, local: PeerId) -> bool {
        self.registered_peers().any(|peer| peer != local)
            || (self.attestors.len() + self.waiting_attestors.len() == 1
                && self.registered_peers().next() == Some(local))
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
    refresh: watch::Receiver<Option<RefreshRequest>>,
) -> Result<(), Error> {
    let load_token = token.clone();
    refresh_loop(token, sender, refresh, move || {
        let cc3 = cc3.clone();
        let token = load_token.clone();
        async move { load(&cc3, &token, chain_key).await }
    })
    .await
}

async fn refresh_loop<F, Fut>(
    token: CancellationToken,
    sender: watch::Sender<Arc<Registry>>,
    mut refresh: watch::Receiver<Option<RefreshRequest>>,
    mut load: F,
) -> Result<(), Error>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Registry, Error>>,
{
    let mut interval = tokio::time::interval(REFRESH_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    interval.tick().await;
    let mut lagging = false;
    loop {
        tokio::select! {
            biased;
            _ = token.cancelled() => return Ok(()),
            _ = interval.tick() => {},
            changed = refresh.changed() => {
                changed.map_err(|_| Error::Init(anyhow::anyhow!("peer registry refresh channel closed")))?;
            },
            _ = tokio::time::sleep(LAGGING_RETRY_INTERVAL), if lagging => {},
        }
        refresh.borrow_and_update();
        let loaded = tokio::select! {
            _ = token.cancelled() => return Ok(()),
            result = load() => result,
        };
        let registry = match loaded {
            Ok(registry) => Arc::new(registry),
            Err(_) if token.is_cancelled() => return Ok(()),
            Err(error) => return Err(error),
        };
        // Requests can arrive while the RPC is in flight. Check the newest required block,
        // not only the request that woke this task, before publishing the result.
        let minimum = refresh
            .borrow()
            .map_or(0, |request| request.finalized_number);
        let current = sender.borrow().clone();
        if !can_advance(&current, &registry).map_err(Error::Init)?
            || registry.finalized_number < minimum
        {
            lagging = true;
            tracing::warn!(
                observed = registry.finalized_number,
                retained = current.finalized_number,
                minimum,
                "ignoring a lagging finalized peer registry"
            );
            continue;
        }
        lagging = false;
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn empty_registry_at(number: u64) -> Registry {
        let mut registry = Registry::from_chain(PeerRegistry {
            registration_required: false,
            attestors: vec![],
            waiting_attestors: vec![],
        })
        .unwrap();
        registry.finalized_number = number;
        registry
    }

    #[test]
    fn only_registry_and_committee_events_request_refreshes() {
        use cc_client::attestation::CcEvent;
        let account = cc_client::AccountId32([1; 32]);
        for event in [
            CcEvent::PeerRegistryChanged(1),
            CcEvent::AttestorsElected(1, vec![]),
            CcEvent::AttestorActivated(1, account.clone()),
            CcEvent::AttestorChilled(1, account.clone()),
            CcEvent::AttestorKicked(account),
            CcEvent::RandomnessChanged((1, [0; 32])),
        ] {
            assert!(affects_registry(&event));
        }
        for event in [
            CcEvent::AttestationIntervalChanged(1, 10),
            CcEvent::TargetSampleSizeChanged(1, 3),
            CcEvent::CheckpointIntervalChanged(1, 10),
            CcEvent::MaxCatchupChanged(1, 100),
        ] {
            assert!(!affects_registry(&event));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn unchanged_registry_uses_two_minute_fallback_without_event_polling() {
        let token = CancellationToken::new();
        let (sender, receiver) = watch::channel(Arc::new(empty_registry_at(10)));
        let (_refresh_tx, refresh_rx) = watch::channel(None);
        let calls = Arc::new(AtomicUsize::new(0));
        let loaded = calls.clone();
        let task = tokio::spawn(refresh_loop(token.clone(), sender, refresh_rx, move || {
            loaded.fetch_add(1, Ordering::SeqCst);
            std::future::ready(Ok(empty_registry_at(11)))
        }));
        tokio::task::yield_now().await;
        tokio::time::advance(REFRESH_INTERVAL - Duration::from_secs(1)).await;
        tokio::task::yield_now().await;
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        tokio::time::advance(Duration::from_secs(1)).await;
        tokio::task::yield_now().await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(receiver.borrow().finalized_number, 11);
        token.cancel();
        task.await.unwrap().unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn events_coalesce_and_reconnect_refreshes_without_waiting_for_fallback() {
        let token = CancellationToken::new();
        let (sender, mut receiver) = watch::channel(Arc::new(empty_registry_at(10)));
        let (refresh_tx, refresh_rx) = watch::channel(None);
        let calls = Arc::new(AtomicUsize::new(0));
        let loaded = calls.clone();
        let next = Arc::new(std::sync::atomic::AtomicU64::new(22));
        let number = next.clone();
        request_refresh(&refresh_tx, 20, 0, true);
        request_refresh(&refresh_tx, 21, 0, true);
        request_refresh(&refresh_tx, 22, 0, true);
        let now = tokio::time::Instant::now();
        let task = tokio::spawn(refresh_loop(token.clone(), sender, refresh_rx, move || {
            loaded.fetch_add(1, Ordering::SeqCst);
            std::future::ready(Ok(empty_registry_at(number.load(Ordering::SeqCst))))
        }));
        receiver.changed().await.unwrap();
        assert_eq!(tokio::time::Instant::now(), now);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        request_refresh(&refresh_tx, 23, 0, false);
        tokio::task::yield_now().await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        next.store(24, Ordering::SeqCst);
        request_refresh(&refresh_tx, 24, 1, false);
        receiver.changed().await.unwrap();
        assert_eq!(tokio::time::Instant::now(), now);
        assert_eq!(receiver.borrow().finalized_number, 24);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        token.cancel();
        task.await.unwrap().unwrap();
    }

    #[test]
    fn initial_batch_refreshes_and_backfilled_requests_cannot_lower_the_required_height() {
        let (sender, mut receiver) = watch::channel(None);
        request_refresh(&sender, 20, 0, false);
        assert!(receiver.has_changed().unwrap());
        assert_eq!(receiver.borrow_and_update().unwrap().finalized_number, 20);
        request_refresh(&sender, 15, 0, true);
        assert!(!receiver.has_changed().unwrap());
        request_refresh(&sender, 21, 0, true);
        assert_eq!(receiver.borrow().unwrap().finalized_number, 21);
    }

    #[tokio::test(start_paused = true)]
    async fn event_during_rpc_rejects_the_pre_event_result_and_retries_promptly() {
        let token = CancellationToken::new();
        let (sender, mut receiver) = watch::channel(Arc::new(empty_registry_at(10)));
        let (refresh_tx, refresh_rx) = watch::channel(None);
        let (release, blocked) = tokio::sync::oneshot::channel();
        let (release_next, blocked_next) = tokio::sync::oneshot::channel();
        let mut blocked = Some(blocked);
        let mut blocked_next = Some(blocked_next);
        let calls = Arc::new(AtomicUsize::new(0));
        let loaded = calls.clone();
        request_refresh(&refresh_tx, 20, 0, true);
        let task = tokio::spawn(refresh_loop(token.clone(), sender, refresh_rx, move || {
            let call = loaded.fetch_add(1, Ordering::SeqCst);
            let blocked = if call == 0 {
                blocked.take()
            } else {
                blocked_next.take()
            };
            async move {
                if let Some(blocked) = blocked {
                    blocked.await.unwrap();
                }
                Ok(empty_registry_at(20 + call as u64))
            }
        }));
        tokio::task::yield_now().await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        request_refresh(&refresh_tx, 21, 0, true);
        release.send(()).unwrap();
        tokio::task::yield_now().await;
        assert_eq!(receiver.borrow().finalized_number, 10);
        release_next.send(()).unwrap();
        receiver.changed().await.unwrap();
        assert_eq!(receiver.borrow().finalized_number, 21);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        token.cancel();
        task.await.unwrap().unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn a_lagging_event_refresh_retries_before_the_slow_fallback() {
        let token = CancellationToken::new();
        let (sender, mut receiver) = watch::channel(Arc::new(empty_registry_at(10)));
        let (refresh_tx, refresh_rx) = watch::channel(None);
        let calls = Arc::new(AtomicUsize::new(0));
        let loaded = calls.clone();
        request_refresh(&refresh_tx, 20, 0, true);
        let task = tokio::spawn(refresh_loop(token.clone(), sender, refresh_rx, move || {
            let call = loaded.fetch_add(1, Ordering::SeqCst);
            std::future::ready(Ok(empty_registry_at(if call == 0 { 19 } else { 20 })))
        }));
        tokio::task::yield_now().await;
        assert_eq!(receiver.borrow().finalized_number, 10);
        let before = tokio::time::Instant::now();
        receiver.changed().await.unwrap();
        assert_eq!(tokio::time::Instant::now() - before, LAGGING_RETRY_INTERVAL);
        assert_eq!(receiver.borrow().finalized_number, 20);
        token.cancel();
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn cancellation_stops_an_in_flight_refresh() {
        let token = CancellationToken::new();
        let (sender, _receiver) = watch::channel(Arc::new(empty_registry_at(10)));
        let (refresh_tx, refresh_rx) = watch::channel(None);
        request_refresh(&refresh_tx, 20, 0, true);
        let task = tokio::spawn(refresh_loop(token.clone(), sender, refresh_rx, || {
            std::future::pending::<Result<Registry, Error>>()
        }));
        tokio::task::yield_now().await;
        token.cancel();
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }

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
            waiting_attestors: vec![],
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
            waiting_attestors: vec![],
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
                attestors,
                waiting_attestors: vec![],
            })
            .is_err());
        }
    }

    #[test]
    fn waiting_candidates_share_the_identity_and_size_checks() {
        let controller = cc_client::AccountId32([9; 32]);
        let input = PeerRegistry {
            registration_required: true,
            attestors: vec![],
            waiting_attestors: vec![(controller.clone(), Some(key(1)))],
        };
        let registry = Registry::from_chain(input.clone()).unwrap();
        assert!(registry.attestors.is_empty());
        assert!(registry
            .peer(&AttestorId::from_public(controller.0))
            .is_some());
        for invalid in [
            PeerRegistry {
                attestors: input.waiting_attestors.clone(),
                ..input.clone()
            },
            PeerRegistry {
                waiting_attestors: vec![(controller.clone(), None)],
                ..input.clone()
            },
            PeerRegistry {
                waiting_attestors: vec![
                    (controller, Some(key(1)));
                    common::constants::MAX_ATTESTORS + 1
                ],
                ..input
            },
        ] {
            assert!(Registry::from_chain(invalid).is_err());
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
            waiting_attestors: vec![],
        })
        .unwrap();
        old.finalized_number = 10;
        let mut current = Registry::from_chain(PeerRegistry {
            registration_required: true,
            attestors: vec![(cc_client::AccountId32([9; 32]), Some(key(2)))],
            waiting_attestors: vec![],
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
