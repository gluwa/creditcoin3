use std::{
    collections::BTreeMap,
    fmt::{Debug, Display},
    time::Duration,
};

use sc_network::PeerId;
use sc_network_gossip::{MessageIntent, ValidationResult, Validator, ValidatorContext};
use sp_core::{Pair, H256};
use sp_runtime::traits::Block as BlockT;

use log::{debug, error};
use parity_scale_codec::Codec;
use parity_scale_codec::Decode;
use parking_lot::{Mutex, RwLock};
use wasm_timer::Instant;

use attestor_primitives::{ChainKey, Round};

use super::{
    gossip::{Action, Consider, Message},
    Attestation,
};
use crate::{worker::votes_topic, HashFor, LOG_TARGET};

// Timeout for rebroadcasting messages.
#[cfg(not(test))]
const REBROADCAST_AFTER: Duration = Duration::from_secs(60);
#[cfg(test)]
const REBROADCAST_AFTER: Duration = Duration::from_secs(5);

pub struct GossipFilter<AccountId> {
    pub round: BTreeMap<ChainKey, (u64, u64)>,
    pub validators: BTreeMap<ChainKey, Vec<AccountId>>,
}

impl<AccountId> GossipFilter<AccountId>
where
    AccountId: Eq + PartialEq,
{
    fn attestor_included(&self, chain_key: ChainKey, attestor: &AccountId) -> bool {
        if let Some(validators) = self.validators.get(&chain_key) {
            validators.contains(attestor)
        } else {
            false
        }
    }

    pub fn consider_vote(&self, round: Round) -> Consider {
        let chain_key = round.0;
        let block_height = round.1;

        if let Some((start, end)) = self.round.get(&chain_key) {
            if block_height < *start {
                return Consider::RejectPast;
            }
            if block_height > *end {
                return Consider::RejectFuture;
            }
            Consider::Accept
        } else {
            Consider::CannotEvaluate
        }
    }
}

pub struct AttestorGossipValidator<B, AccountId>
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
}

impl<B, AccountId> Default for AttestorGossipValidator<B, AccountId>
where
    B: BlockT,
    H256: From<<B as BlockT>::Hash>,
    AccountId:
        Clone + Display + Codec + Send + 'static + Sync + Debug + Into<[u8; 32]> + Eq + PartialEq,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<B, AccountId> AttestorGossipValidator<B, AccountId>
where
    B: BlockT,
    H256: From<<B as BlockT>::Hash>,
    AccountId:
        Clone + Display + Codec + Send + 'static + Sync + Debug + Into<[u8; 32]> + Eq + PartialEq,
{
    pub fn new() -> Self {
        Self {
            votes_topic: votes_topic::<B>(),
            gossip_filter: RwLock::new(GossipFilter {
                round: BTreeMap::new(),
                validators: BTreeMap::new(),
            }),
            next_rebroadcast: Mutex::new(Instant::now() + REBROADCAST_AFTER),
        }
    }

    pub fn update_filter(
        &self,
        chain_key: ChainKey,
        start: u64,
        end: u64,
        validators: Vec<AccountId>,
    ) {
        let mut filter = self.gossip_filter.write();
        filter.round.insert(chain_key, (start, end));
        filter.validators.insert(chain_key, validators);
    }

    fn verify_signature(
        &self,
        attestation: &Attestation<HashFor<B>, AccountId>,
    ) -> Action<B::Hash> {
        let filter = self.gossip_filter.read();
        let round = attestation.round();
        let chain_key = round.0;

        // first check if the round is in the filter
        if filter.consider_vote(round) != Consider::Accept {
            return Action::Discard;
        }

        let attestor = attestation.attestor.clone();

        // first check if the attestor was elected for this epoch
        if !filter.attestor_included(chain_key, &attestor) {
            return Action::Discard;
        }

        // then check the signature
        let public_key = sp_core::sr25519::Public::from_raw(attestor.into());
        let msg = attestation.attestation_data.serialize();
        let sr_valid = sp_core::sr25519::Pair::verify(&attestation.signature, msg, &public_key);
        if !sr_valid {
            return Action::Discard;
        }

        Action::Keep(self.votes_topic)
    }
}

impl<Block, AccountId> Validator<Block> for AttestorGossipValidator<Block, AccountId>
where
    Block: BlockT,
    H256: From<<Block as BlockT>::Hash>,
    AccountId:
        Clone + Display + Codec + Send + 'static + Sync + Debug + Into<[u8; 32]> + Eq + PartialEq,
{
    fn validate(
        &self,
        context: &mut dyn ValidatorContext<Block>,
        _sender: &PeerId,
        data: &[u8],
    ) -> ValidationResult<Block::Hash> {
        let action = match Message::<Block, AccountId>::decode(&mut &data[..]) {
            Ok(Message::Attestation(att)) => {
                debug!(target: LOG_TARGET, "📝 Received attestation by: {:?}, round: {:?}", att.attestor, att.round());
                self.verify_signature(&att)
            }
            Err(err) => {
                error!(target: LOG_TARGET, "📝 Error decoding block hash in message: {:?}", err);
                Action::Discard
            }
        };

        match action {
            Action::Keep(topic) => {
                debug!(target: LOG_TARGET, "📝 Broadcasting message for topic {:?}", topic);
                context.broadcast_message(topic, data.to_vec(), false);
                ValidationResult::ProcessAndKeep(topic)
            }
            Action::Discard => ValidationResult::Discard,
        }
    }

    fn message_expired<'a>(&'a self) -> Box<dyn FnMut(Block::Hash, &[u8]) -> bool + 'a> {
        debug!(target: LOG_TARGET, "📝 Setting up message expiration");
        let filter = self.gossip_filter.read();
        Box::new(
            move |_topic, data| match Message::<Block, AccountId>::decode(&mut &data[..]) {
                Ok(Message::Attestation(msg)) => {
                    let round = msg.round();

                    let expired = filter.consider_vote(round) != Consider::Accept;

                    debug!(target: LOG_TARGET, "📝 Vote for round #{:?} expired: {}", round, expired);
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
                return do_rebroadcast;
            }

            match Message::<Block, AccountId>::decode(&mut &data[..]) {
                Ok(Message::Attestation(msg)) => {
                    let round = msg.round();

                    let allowed = filter.consider_vote(round) == Consider::Accept;
                    debug!(target: LOG_TARGET, "📝 Vote for round #{:?} allowed: {}", round, allowed);
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

    #[test]
    fn should_validate_messages() {
        let _ = env_logger::try_init();

        let attestor = Attestor::new();
        let validator_set = vec![attestor.account_id.clone()];

        let attestation_data = simulate_attestation_data(1, 1);
        let attestation = create_signed_attestation(&attestor, attestation_data.clone());

        let gossip_validator = AttestorGossipValidator::<Block, AccountId32>::new();

        let mut context = TestContext;
        let sender = PeerId::random();

        let encoded = Message::<Block, AccountId32>::Attestation(attestation.clone()).encode();

        let res = gossip_validator.validate(&mut context, &sender, &encoded);
        assert!(matches!(res, ValidationResult::Discard));

        gossip_validator.update_filter(1, 0, 10, validator_set.clone());

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
        gossip_validator.update_filter(1, 10, 200, validator_set);
        let res = gossip_validator.validate(&mut context, &sender, &encoded);
        assert!(matches!(res, ValidationResult::Discard));
    }
}
