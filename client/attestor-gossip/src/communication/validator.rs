use std::{
    collections::BTreeMap,
    fmt::{Debug, Display},
    sync::Arc,
    time::Duration,
};

use sc_network::{NetworkPeers, PeerId, ReputationChange};
use sc_network_gossip::{MessageIntent, ValidationResult, Validator, ValidatorContext};
use sp_core::{Pair, H256};
use sp_runtime::traits::Block as BlockT;

use log::{debug, error, info};
use parity_scale_codec::Codec;
use parity_scale_codec::Decode;
use parking_lot::{Mutex, RwLock};
use wasm_timer::Instant;

use attestor_primitives::{ChainKey, Round};

use super::{
    cost,
    gossip::{Action, Consider, Message},
    Attestation,
};
use crate::{communication::benefit, worker::votes_topic, HashFor};

const LOG_TARGET: &str = "attestor-gossip-comms";

// Timeout for rebroadcasting messages.
#[cfg(not(test))]
const REBROADCAST_AFTER: Duration = Duration::from_secs(60);
#[cfg(test)]
const REBROADCAST_AFTER: Duration = Duration::from_secs(5);

pub struct GossipFilter<AccountId> {
    pub epoch: u64,
    pub round: BTreeMap<ChainKey, (u64, u64)>,
    pub attestors: BTreeMap<ChainKey, Vec<AccountId>>,
}

impl<AccountId> GossipFilter<AccountId>
where
    AccountId: Eq + PartialEq,
{
    fn attestor_included(&self, chain_key: ChainKey, attestor: &AccountId) -> bool {
        if let Some(attestors) = self.attestors.get(&chain_key) {
            attestors.contains(attestor)
        } else {
            false
        }
    }

    pub fn consider_vote(&self, round: Round, epoch: u64) -> Consider {
        if epoch < self.epoch {
            info!(target: LOG_TARGET, "📝 Vote for round #{:?} expired because epoch has changed. Vote epoch: {}, Expected epoch: {}", round, epoch, self.epoch);
            return Consider::RejectPast;
        }

        let chain_key = round.0;
        let block_height = round.1;

        if let Some((start, window)) = self.round.get(&chain_key) {
            if block_height < *start {
                return Consider::RejectPast;
            }
            if block_height > *start + window {
                return Consider::RejectFuture;
            }
            Consider::Accept
        } else {
            Consider::CannotEvaluate
        }
    }
}

#[derive(Clone, Debug)]
pub struct GossipFilterCfg<'a, AccountId> {
    pub chain_key: ChainKey,
    pub epoch: u64,
    pub start: u64,
    // Window indicates the range of blocks that are considered valid for this round.
    // For example, if start is 100 and window is 100, then we accept attestations for blocks 100 to 200.
    pub window: u64,
    pub attestors: &'a Vec<AccountId>,
}

pub struct AttestorGossipValidator<B, AccountId, N>
where
    B: BlockT,
    AccountId: Clone + Display + Codec + Send + 'static + Sync + Debug + Into<[u8; 32]>,
{
    /// Topic for votes
    votes_topic: B::Hash,

    /// Gossip filter
    gossip_filter: RwLock<GossipFilter<AccountId>>,

    /// Next Rebroadcast time
    next_rebroadcast: Mutex<Instant>,

    /// Network
    network: Arc<N>,
}

impl<B, AccountId, N> AttestorGossipValidator<B, AccountId, N>
where
    B: BlockT,
    H256: From<<B as BlockT>::Hash>,
    AccountId:
        Clone + Display + Codec + Send + 'static + Sync + Debug + Into<[u8; 32]> + Eq + PartialEq,
    N: NetworkPeers,
{
    pub fn new(network: Arc<N>) -> Self {
        Self {
            votes_topic: votes_topic::<B>(),
            gossip_filter: RwLock::new(GossipFilter {
                epoch: 0,
                round: BTreeMap::new(),
                attestors: BTreeMap::new(),
            }),
            next_rebroadcast: Mutex::new(Instant::now() + REBROADCAST_AFTER),
            network,
        }
    }

    fn report(&self, who: PeerId, cost_benefit: ReputationChange) {
        self.network.report_peer(who, cost_benefit);
    }

    pub fn update_filter(&self, cfg: GossipFilterCfg<'_, AccountId>) {
        let mut filter = self.gossip_filter.write();
        filter.round.insert(cfg.chain_key, (cfg.start, cfg.window));
        filter
            .attestors
            .insert(cfg.chain_key, cfg.attestors.clone());
        filter.epoch = cfg.epoch;
    }

    fn verify_signature(
        &self,
        attestation: &Attestation<HashFor<B>, AccountId>,
    ) -> Action<B::Hash> {
        let filter = self.gossip_filter.read();
        let round = attestation.round();
        let chain_key = round.0;

        // first check if the round is in the filter
        match filter.consider_vote(round, attestation.epoch) {
            Consider::RejectPast => return Action::Discard(cost::OUTDATED_MESSAGE),
            Consider::RejectFuture => return Action::Discard(cost::FUTURE_MESSAGE),
            Consider::CannotEvaluate => {
                error!(target: LOG_TARGET, "📝 Cannot evaluate vote for round #{:?} in epoch {}. Chain key: {:?}", round, attestation.epoch, chain_key);
                return Action::DiscardNoReport;
            }
            Consider::Accept => {}
        }

        let attestor = attestation.attestor.clone();

        // first check if the attestor was elected for this epoch
        if !filter.attestor_included(chain_key, &attestor) {
            return Action::Discard(cost::UNKNOWN_VOTER);
        }

        // then check the signature
        let public_key = sp_core::sr25519::Public::from_raw(attestor.into());
        let msg = attestation.attestation_data.serialize();
        let sr_valid = sp_core::sr25519::Pair::verify(&attestation.signature, msg, &public_key);
        if !sr_valid {
            return Action::Discard(cost::BAD_SIGNATURE);
        }

        Action::Keep(self.votes_topic, benefit::VOTE_MESSAGE)
    }

    pub fn expire(&self, round: Round) {
        let mut filter = self.gossip_filter.write();

        if let Some((start, _)) = filter.round.get_mut(&round.0) {
            debug!(target: LOG_TARGET, "📝 Setting new start for round #{:?} to {}", round, round.1);
            *start = round.1;
        }
    }
}

impl<Block, AccountId, N> Validator<Block> for AttestorGossipValidator<Block, AccountId, N>
where
    Block: BlockT,
    H256: From<<Block as BlockT>::Hash>,
    AccountId:
        Clone + Display + Codec + Send + 'static + Sync + Debug + Into<[u8; 32]> + Eq + PartialEq,
    N: NetworkPeers + Send + Sync,
{
    fn validate(
        &self,
        context: &mut dyn ValidatorContext<Block>,
        sender: &PeerId,
        data: &[u8],
    ) -> ValidationResult<Block::Hash> {
        let raw = data;
        let action = match Message::<Block, AccountId>::decode(&mut &data[..]) {
            Ok(Message::Attestation(att)) => {
                debug!(target: LOG_TARGET, "📝 Received attestation by: {:?}, round: {:?}", att.attestor, att.round());
                self.verify_signature(&att)
            }
            Err(err) => {
                error!(target: LOG_TARGET, "📝 Error decoding block hash in message: {err:?}");
                let bytes = raw.len().min(i32::MAX as usize) as i32;
                let cost = ReputationChange::new(
                    bytes.saturating_mul(cost::PER_UNDECODABLE_BYTE),
                    "ATTESTOR: Bad packet",
                );
                Action::Discard(cost)
            }
        };

        match action {
            Action::Keep(topic, cb) => {
                self.report(*sender, cb);
                debug!(target: LOG_TARGET, "📝 Broadcasting message for topic {topic:?}");
                context.broadcast_message(topic, data.to_vec(), false);
                ValidationResult::ProcessAndKeep(topic)
            }
            Action::Discard(cb) => {
                self.report(*sender, cb);
                ValidationResult::Discard
            }
            Action::DiscardNoReport => ValidationResult::Discard,
        }
    }

    fn message_expired<'a>(&'a self) -> Box<dyn FnMut(Block::Hash, &[u8]) -> bool + 'a> {
        debug!(target: LOG_TARGET, "📝 Setting up message expiration");
        let filter = self.gossip_filter.read();
        Box::new(
            move |_topic, data| match Message::<Block, AccountId>::decode(&mut &data[..]) {
                Ok(Message::Attestation(msg)) => {
                    let round = msg.round();

                    let expired = filter.consider_vote(round, msg.epoch) != Consider::Accept;

                    debug!(target: LOG_TARGET, "📝 Vote for round #{round:?} expired: {expired}");
                    expired
                }
                Err(_) => true,
            },
        )
    }

    fn message_allowed<'a>(
        &'a self,
    ) -> Box<dyn FnMut(&PeerId, MessageIntent, &Block::Hash, &[u8]) -> bool + 'a> {
        debug!(target: LOG_TARGET, "📝 Setting up message allowed");
        let do_rebroadcast = {
            let now = Instant::now();
            let mut next_rebroadcast = self.next_rebroadcast.lock();
            if now >= *next_rebroadcast {
                debug!(target: LOG_TARGET, "📝 Gossip rebroadcast allowed true");
                *next_rebroadcast = now + REBROADCAST_AFTER;
                true
            } else {
                debug!(target: LOG_TARGET, "📝 Gossip rebroadcast not allowed");
                false
            }
        };

        let filter = self.gossip_filter.read();
        Box::new(move |_who, intent, _topic, data| {
            if let MessageIntent::PeriodicRebroadcast = intent {
                // gate on both timer AND filter
                if let Ok(Message::Attestation(msg)) =
                    Message::<Block, AccountId>::decode(&mut &data[..])
                {
                    let round = msg.round();
                    let allowed = filter.consider_vote(round, msg.epoch) == Consider::Accept;
                    return do_rebroadcast && allowed;
                }
                return false;
            }

            match Message::<Block, AccountId>::decode(&mut &data[..]) {
                Ok(Message::Attestation(msg)) => {
                    let round = msg.round();

                    let allowed = filter.consider_vote(round, msg.epoch) == Consider::Accept;
                    debug!(target: LOG_TARGET, "📝 Vote for round #{round:?} allowed: {allowed}");
                    allowed
                }
                Err(_) => false,
            }
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::test_utils::{create_signed_attestation, simulate_attestation_data, Attestor};
    use parity_scale_codec::Encode;
    use sc_network_test::Block;
    use sp_runtime::AccountId32;

    struct TestContext;
    impl<B: sp_runtime::traits::Block> ValidatorContext<B> for TestContext {
        fn broadcast_topic(&mut self, _topic: B::Hash, _force: bool) {
            unimplemented!()
        }

        fn broadcast_message(&mut self, _topic: B::Hash, _message: Vec<u8>, _force: bool) {}

        fn send_message(&mut self, _who: &sc_network::PeerId, _message: Vec<u8>) {
            unimplemented!()
        }

        fn send_topic(&mut self, _who: &sc_network::PeerId, _topic: B::Hash, _force: bool) {
            unimplemented!()
        }
    }

    pub(crate) struct TestNetwork {}

    impl TestNetwork {
        pub fn new() -> Self {
            Self {}
        }
    }

    #[async_trait::async_trait]
    impl NetworkPeers for TestNetwork {
        fn set_authorized_peers(&self, _: std::collections::HashSet<PeerId>) {
            unimplemented!()
        }

        fn set_authorized_only(&self, _: bool) {
            unimplemented!()
        }

        fn add_known_address(&self, _: PeerId, _: sc_network::Multiaddr) {
            unimplemented!()
        }

        fn report_peer(&self, _: PeerId, _: ReputationChange) {
            // let _ = self.report_sender.unbounded_send(PeerReport {
            //     who: peer_id,
            //     cost_benefit,
            // });
        }

        fn peer_reputation(&self, _: &PeerId) -> i32 {
            unimplemented!()
        }

        fn disconnect_peer(&self, _: PeerId, _: sc_network::ProtocolName) {
            unimplemented!()
        }

        fn accept_unreserved_peers(&self) {
            unimplemented!()
        }

        fn deny_unreserved_peers(&self) {
            unimplemented!()
        }

        fn add_reserved_peer(
            &self,
            _: sc_network::config::MultiaddrWithPeerId,
        ) -> Result<(), String> {
            unimplemented!()
        }

        fn remove_reserved_peer(&self, _: PeerId) {
            unimplemented!()
        }

        fn set_reserved_peers(
            &self,
            _: sc_network::ProtocolName,
            _: std::collections::HashSet<sc_network::Multiaddr>,
        ) -> Result<(), String> {
            unimplemented!()
        }

        fn add_peers_to_reserved_set(
            &self,
            _: sc_network::ProtocolName,
            _: std::collections::HashSet<sc_network::Multiaddr>,
        ) -> Result<(), String> {
            unimplemented!()
        }

        fn remove_peers_from_reserved_set(
            &self,
            _: sc_network::ProtocolName,
            _: Vec<PeerId>,
        ) -> Result<(), String> {
            unimplemented!()
        }

        fn sync_num_connected(&self) -> usize {
            unimplemented!()
        }

        fn peer_role(&self, _: PeerId, _: Vec<u8>) -> Option<sc_network::ObservedRole> {
            unimplemented!()
        }

        async fn reserved_peers(&self) -> Result<Vec<PeerId>, ()> {
            unimplemented!();
        }
    }

    #[test]
    fn should_validate_messages() {
        let _ = env_logger::try_init();

        let attestor = Attestor::new();
        let validator_set = vec![attestor.account_id.clone()];

        let chain_key = 1;

        let attestation_data = simulate_attestation_data(chain_key, 1);
        let attestation = create_signed_attestation(&attestor, attestation_data.clone());

        let network = TestNetwork::new();

        let gossip_validator =
            AttestorGossipValidator::<Block, AccountId32, _>::new(Arc::new(network));

        let mut context = TestContext;
        let sender = PeerId::random();

        let encoded = Message::<Block, AccountId32>::Attestation(attestation.clone()).encode();

        let res = gossip_validator.validate(&mut context, &sender, &encoded);
        assert!(matches!(res, ValidationResult::Discard));

        gossip_validator.update_filter(GossipFilterCfg {
            chain_key,
            epoch: 1,
            start: 0,
            window: 1,
            attestors: &validator_set,
        });

        let res = gossip_validator.validate(&mut context, &sender, &encoded);
        assert!(matches!(res, ValidationResult::ProcessAndKeep(_)));

        // test voter not in validator set
        let attestor2 = Attestor::new();
        let attestation = create_signed_attestation(&attestor2, attestation_data.clone());
        let encoded = Message::<Block, AccountId32>::Attestation(attestation.clone()).encode();
        let res = gossip_validator.validate(&mut context, &PeerId::random(), &encoded);
        assert!(matches!(res, ValidationResult::Discard));

        // reject if the round is not in the filter
        let attestation_data = simulate_attestation_data(2, 1);
        let attestation = create_signed_attestation(&attestor, attestation_data);
        let encoded = Message::<Block, AccountId32>::Attestation(attestation.clone()).encode();
        let res = gossip_validator.validate(&mut context, &sender, &encoded);
        assert!(matches!(res, ValidationResult::Discard));

        // reject past votes
        let attestation_data = simulate_attestation_data(1, 0);
        let attestation = create_signed_attestation(&attestor, attestation_data);
        let encoded = Message::<Block, AccountId32>::Attestation(attestation.clone()).encode();

        gossip_validator.update_filter(GossipFilterCfg {
            chain_key,
            epoch: 2,
            start: 100,
            window: 100,
            attestors: &validator_set,
        });

        let res = gossip_validator.validate(&mut context, &sender, &encoded);
        assert!(matches!(res, ValidationResult::Discard));
    }

    #[test]
    fn should_reject_when_epoch_changes() {
        let _ = env_logger::try_init();

        let attestor = Attestor::new();
        let validator_set = vec![attestor.account_id.clone()];

        let chain_key = 1;

        let attestation_data = simulate_attestation_data(chain_key, 1);
        let attestation = create_signed_attestation(&attestor, attestation_data.clone());

        let network = TestNetwork::new();

        let gossip_validator =
            AttestorGossipValidator::<Block, AccountId32, _>::new(Arc::new(network));

        let mut context = TestContext;
        let sender = PeerId::random();

        let encoded = Message::<Block, AccountId32>::Attestation(attestation.clone()).encode();

        let res = gossip_validator.validate(&mut context, &sender, &encoded);
        assert!(matches!(res, ValidationResult::Discard));

        gossip_validator.update_filter(GossipFilterCfg {
            chain_key,
            epoch: 1,
            start: 0,
            window: 10,
            attestors: &validator_set,
        });

        let res = gossip_validator.validate(&mut context, &sender, &encoded);
        assert!(matches!(res, ValidationResult::ProcessAndKeep(_)));

        // Reject votes if epoch changes
        let attestation_data = simulate_attestation_data(1, 0);
        let attestation = create_signed_attestation(&attestor, attestation_data);
        let encoded = Message::<Block, AccountId32>::Attestation(attestation.clone()).encode();

        gossip_validator.update_filter(GossipFilterCfg {
            chain_key,
            epoch: 2,
            start: 0,
            window: 10,
            attestors: &validator_set,
        });

        let res = gossip_validator.validate(&mut context, &sender, &encoded);
        assert!(matches!(res, ValidationResult::Discard));
    }

    #[test]
    fn messages_allowed_and_expired() {
        let _ = env_logger::try_init();

        let attestor = Attestor::new();
        let validator_set = vec![attestor.account_id.clone()];

        let chain_key = 1;

        let attestation_data = simulate_attestation_data(chain_key, 50);
        let attestation = create_signed_attestation(&attestor, attestation_data.clone());

        let network = TestNetwork::new();

        let gv = AttestorGossipValidator::<Block, AccountId32, _>::new(Arc::new(network));

        gv.update_filter(GossipFilterCfg {
            chain_key,
            epoch: 1,
            start: 50,
            window: 50,
            attestors: &validator_set,
        });

        // Check if message is allowed
        let sender = PeerId::random();
        let encoded = Message::<Block, AccountId32>::Attestation(attestation.clone()).encode();
        let intent = MessageIntent::Broadcast;
        let topic = Default::default();

        let mut allowed = gv.message_allowed();
        let mut expired = gv.message_expired();

        // check bad vote format
        assert!(!allowed(&sender, intent, &topic, &mut [0u8; 16]));
        assert!(expired(topic, &mut [0u8; 16]));

        // check good vote format
        assert!(allowed(&sender, intent, &topic, &encoded));
        assert!(!expired(topic, &encoded));

        // future round
        let attestation_data = simulate_attestation_data(chain_key, 110);
        let attestation = create_signed_attestation(&attestor, attestation_data);
        let encoded = Message::<Block, AccountId32>::Attestation(attestation.clone()).encode();
        assert!(!allowed(&sender, intent, &topic, &encoded));
        assert!(expired(topic, &encoded));

        // expired round
        let attestation_data = simulate_attestation_data(1, 10);
        let attestation = create_signed_attestation(&attestor, attestation_data);
        let encoded = Message::<Block, AccountId32>::Attestation(attestation.clone()).encode();
        assert!(!allowed(&sender, intent, &topic, &encoded));
        assert!(expired(topic, &encoded));
    }

    #[test]
    fn messages_rebroadcast() {
        let attestor = Attestor::new();
        let validator_set = vec![attestor.account_id.clone()];

        let chain_key = 1;

        let network = TestNetwork::new();

        let gv = AttestorGossipValidator::<Block, AccountId32, _>::new(Arc::new(network));

        gv.update_filter(GossipFilterCfg {
            chain_key,
            epoch: 1,
            start: 50,
            window: 50,
            attestors: &validator_set,
        });

        let sender = sc_network_types::PeerId::random();
        let topic = Default::default();

        let mut encoded_vote = Message::<Block, AccountId32>::Attestation(
            create_signed_attestation(&attestor, simulate_attestation_data(chain_key, 50)),
        )
        .encode();

        // re-broadcasting only allowed at `REBROADCAST_AFTER` intervals
        let intent = MessageIntent::PeriodicRebroadcast;
        let mut allowed = gv.message_allowed();

        // rebroadcast not allowed so soon after GossipValidator creation
        assert!(!allowed(&sender, intent, &topic, &mut encoded_vote));

        // hack the inner deadline to be `now`
        *gv.next_rebroadcast.lock() = Instant::now();

        // still not allowed on old `allowed` closure result
        assert!(!allowed(&sender, intent, &topic, &mut encoded_vote));

        // renew closure result
        let mut allowed = gv.message_allowed();
        // rebroadcast should be allowed now
        assert!(allowed(&sender, intent, &topic, &mut encoded_vote));
    }
}
