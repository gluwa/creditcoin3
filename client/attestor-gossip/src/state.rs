use anyhow::Result;
use log::{debug, info, warn};
use sp_core::H256;
use std::collections::{BTreeMap, HashMap};

use attestor_primitives::{ChainKey, Round};

use crate::{communication::Attestation, round::RoundConfig, Error, LOG_TARGET};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoteImportResult {
    Ok,
    RoundConcluded,
    DoubleVote,
}

pub type BlockNumber = u64;

pub type EpochIndex = u64;

/// Votes. Maps an account id to an attestation
/// To keep track of who voted for what
pub type Votes<H, AccountId> = HashMap<AccountId, Attestation<H, AccountId>>;

/// Voting round per chain key
pub type ChainVoteRound<H, A> = BTreeMap<ChainKey, VoteRound<H, A>>;

#[derive(Debug, Clone)]
pub struct VoteRound<H, A> {
    // Votes per chain and block number
    pub header_votes: BTreeMap<BlockNumber, Votes<H, A>>,
    // The epoch index when the first vote was cast
    pub epoch: EpochIndex,
}

impl<H, A> Default for VoteRound<H, A> {
    fn default() -> Self {
        Self {
            header_votes: BTreeMap::new(),
            epoch: 0,
        }
    }
}

impl<H, A> VoteRound<H, A>
where
    A: Clone + PartialEq + Eq + std::hash::Hash,
{
    pub fn init(attestation: Attestation<H, A>, epoch: EpochIndex) -> Self {
        let header_number = attestation.attestation_data.header_number;
        let attestor_id = attestation.attestor.clone();

        let mut header_votes = BTreeMap::new();
        let mut votes = HashMap::new();
        votes.insert(attestor_id, attestation);
        header_votes.insert(header_number, votes);

        Self {
            header_votes,
            epoch,
        }
    }

    pub fn add_vote(
        &mut self,
        attestation: Attestation<H, A>,
        epoch_index: u64,
    ) -> Option<Attestation<H, A>> {
        let header_number = attestation.attestation_data.header_number;
        let attestor_id = attestation.attestor.clone();

        let entry = self.header_votes.get_mut(&header_number);

        if let Some(votes) = entry {
            if self.epoch != epoch_index {
                info!(
                    target: LOG_TARGET,
                    "📝 Epoch mismatch, expected: {}, got: {}", self.epoch, epoch_index
                );
                // Should clear votes for the header number
                votes.clear();
                // Update the epoch
                self.epoch = epoch_index;
            }

            // Insert the vote
            return votes.insert(attestor_id, attestation);
        } else {
            let mut votes = HashMap::new();
            votes.insert(attestor_id, attestation);
            self.header_votes.insert(header_number, votes);
            self.epoch = epoch_index;
        }

        None
    }

    pub fn clear_votes(&mut self, header_number: BlockNumber) {
        if let Some(votes) = self.header_votes.get_mut(&header_number) {
            votes.clear();
        }
    }
}

/// Active attestor set per chain / epoch
pub type ActiveAttestorSet<AccountId> = BTreeMap<ChainKey, BTreeMap<EpochIndex, Vec<AccountId>>>;

#[derive(Debug, Clone)]
pub struct State<H, AccountId> {
    /// Maps chain key to a map of block number to votes
    pub chain_head_votes: ChainVoteRound<H, AccountId>,
    /// Active attestor set
    pub active_attestor_set: ActiveAttestorSet<AccountId>,
}

impl<H, AccountId> State<H, AccountId>
where
    H: Clone
        + AsRef<[u8]>
        + Into<H256>
        + From<H256>
        + PartialEq
        + Eq
        + std::hash::Hash
        + Default
        + std::fmt::Debug
        + Copy,
    AccountId: Clone + PartialEq + Eq + std::hash::Hash + Into<[u8; 32]> + std::fmt::Debug,
{
    pub fn new_chain(&mut self, attestation: Attestation<H, AccountId>, epoch_index: u64) {
        let round = attestation.round();
        let chain_key = round.0;

        self.chain_head_votes
            .insert(chain_key, VoteRound::init(attestation, epoch_index));
    }

    pub fn clear_votes(&mut self, chain_key: ChainKey, header_number: u64) {
        if let Some(vote_round) = self.chain_head_votes.get_mut(&chain_key) {
            vote_round.clear_votes(header_number);
        }
    }

    pub fn get_attestations_by_chain_and_header(
        &self,
        chain: ChainKey,
        header_number: u64,
    ) -> Result<&Votes<H, AccountId>, Error> {
        let vote_round = self
            .chain_head_votes
            .get(&chain)
            .ok_or(Error::Other(format!(
                "Error fetching attestations for chain, Chain key: {}",
                chain
            )))?;

        let votes = vote_round
            .header_votes
            .get(&header_number)
            .ok_or(Error::Other(
                "Error fetching attestation for block".to_string(),
            ))?;

        Ok(votes)
    }

    pub fn note_vote(
        &mut self,
        attestation: Attestation<H, AccountId>,
        round_config: &RoundConfig,
        epoch_index: u64,
    ) -> Result<VoteImportResult, Error> {
        let round = attestation.round();
        let chain_key = round.0;
        let header_number = round.1;
        let attestor_id = attestation.attestor.clone();

        // Check if the chain_key exists in the block_attestations
        if let Some(vote_round) = self.chain_head_votes.get_mut(&chain_key) {
            let old_vote = vote_round.add_vote(attestation, epoch_index);
            if old_vote.is_some() {
                warn!(target: LOG_TARGET, "📝 Attestor({:?}) voted for round {:?} again", attestor_id, (chain_key, header_number));
                return Ok(VoteImportResult::DoubleVote);
            }
        } else {
            // Insert new attestation if it doesn't exist
            debug!(target: LOG_TARGET, "📝 First time a vote comes in for new chain: {}, round: {:?}", chain_key, round);
            self.new_chain(attestation, epoch_index);
        }

        if self.check_round_state(round, round_config)? {
            return Ok(VoteImportResult::RoundConcluded);
        }

        Ok(VoteImportResult::Ok)
    }

    pub fn check_round_state(
        &mut self,
        round: Round,
        round_config: &RoundConfig,
    ) -> Result<bool, Error> {
        let chain_key = round.0;
        let header_number = round.1;

        let block_attestations =
            self.get_attestations_by_chain_and_header(chain_key, header_number)?;

        let (major_digest, _) = find_major_digest::<H, AccountId>(block_attestations);

        // Filter attestations by major digest
        // TODO: Can we do this in a more efficient way / place?
        let attestations = block_attestations
            .iter()
            .filter(|(_, attestation)| attestation.digest() == major_digest.into())
            .collect::<Vec<_>>();

        // Get calculated threshold for the round
        let threshold = round_config.threshold;

        info!(
            target: LOG_TARGET,
            "📝 Checking if we can finalize round{:?}, digest: {:?}, Votes: {:?}/{:?}",
            round,
            major_digest,
            attestations.len(),
            threshold
        );
        // If we can't find a majority voting on the same digest, we can't continue
        // Also check if the target attestation to be submitted is the same as the last attestation + interval
        // Only then we can submit the attestation
        Ok(attestations.len() >= threshold.try_into().unwrap())
    }

    pub fn update_active_attestors(
        &mut self,
        chain_key: ChainKey,
        epoch_index: u64,
        active_attestors: Vec<AccountId>,
    ) {
        // Update active attestor set
        let chain_active_attestors = self.active_attestor_set.entry(chain_key).or_default();

        // Insert new active attestor set
        chain_active_attestors.insert(epoch_index, active_attestors);
    }

    pub fn check_chain_attestor_epoch_inclusion(
        &self,
        chain_key: ChainKey,
        epoch_index: u64,
        attestor: AccountId,
    ) -> Result<bool, Error> {
        let epoch_active_attestors = self
            .active_attestor_set
            .get(&chain_key)
            .ok_or(Error::RoundConfigNotSet)?;

        let attestors = epoch_active_attestors
            .get(&epoch_index)
            .ok_or(Error::RoundConfigNotSet)?;

        Ok(attestors.contains(&attestor))
    }
}

impl<H, AccountId> Default for State<H, AccountId> {
    fn default() -> Self {
        State {
            chain_head_votes: BTreeMap::new(),
            active_attestor_set: BTreeMap::new(),
        }
    }
}

/// Function to find the most frequently occurring digest
fn find_major_digest<H, AccountId>(attestations: &Votes<H, AccountId>) -> (H, usize)
where
    H: Clone + PartialEq + Eq + std::hash::Hash + Default + AsRef<[u8]> + From<H256> + Copy,
    AccountId: Into<[u8; 32]> + Clone,
{
    let mut digest_count: HashMap<H, usize> = HashMap::new();
    for attestation in attestations.values() {
        let digest = attestation.digest();
        *digest_count.entry(H::from(digest)).or_insert(0) += 1;
    }

    digest_count
        .into_iter()
        .max_by_key(|&(_, count)| count)
        .unwrap_or((H::default(), 0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use attestor_primitives::{Attestation as AttestationPrimitive, BlsPublicKey};
    use bls_signatures::Signature;
    use bls_signatures::{key::Serialize, PrivateKey};
    use sp_core::{H256, H512};
    use sp_runtime::AccountId32;

    struct Attestor {
        account_id: AccountId32,
        private_key: PrivateKey,
        _public_key: BlsPublicKey,
    }

    impl Attestor {
        pub fn new(account_id: AccountId32) -> Self {
            let rng = sp_core::H256::random().0;
            let private_key = PrivateKey::new(rng);
            let _public_key = private_key.public_key().as_bytes()[..].try_into().unwrap();

            Self {
                account_id,
                private_key,
                _public_key,
            }
        }

        pub fn sign_bls_attestation(&self, attestation: &AttestationPrimitive<H256>) -> Signature {
            self.private_key.sign(attestation.serialize())
        }
    }

    fn simulate_attestation_data(chain_key: u64, header_number: u64) -> AttestationPrimitive<H256> {
        AttestationPrimitive {
            chain_key,
            header_hash: H256::random(),
            header_number,
            prev_digest: None,
            root: H256::random().0,
        }
    }

    fn create_signed_attestation(
        attestor: &Attestor,
        attestation_data: AttestationPrimitive<H256>,
    ) -> Attestation<H256, AccountId32> {
        let signature = attestor.sign_bls_attestation(&attestation_data);

        Attestation {
            attestation_data,
            attestor: attestor.account_id.clone(),
            continuity_proof: vec![],
            proof_of_inclusion: Default::default(),
            signature: sp_core::sr25519::Signature(H512::random().0),
            signature_bls: attestor_primitives::bls::WrapEncode(signature),
        }
    }

    #[test]
    fn test_single_vote() {
        let mut state = State::default();

        let account_id = AccountId32::new(H256::random().0);
        let attestor = Attestor::new(account_id);

        let attestation_data = simulate_attestation_data(1, 1);
        let attestation = create_signed_attestation(&attestor, attestation_data);

        // With this config, we immediately conclude the round
        let round_config = RoundConfig {
            committee_set_size: 1,
            target_sample_size: 1,
            threshold: 1,
        };

        let result = state
            .note_vote(attestation.clone(), &round_config, 1)
            .unwrap();
        assert_eq!(result, VoteImportResult::RoundConcluded);

        let votes = state.get_attestations_by_chain_and_header(1, 1).unwrap();

        assert_eq!(votes.len(), 1);
        assert_eq!(votes.get(&attestor.account_id), Some(&attestation));
    }

    #[test]
    fn test_single_vote_not_concluding_round() {
        let mut state = State::default();

        let account_id = AccountId32::new(H256::random().0);
        let attestor = Attestor::new(account_id);

        let attestation_data = simulate_attestation_data(1, 1);
        let attestation = create_signed_attestation(&attestor, attestation_data);

        // With this config, we can't conclude the round if we only have one vote
        let round_config = RoundConfig {
            committee_set_size: 3,
            target_sample_size: 3,
            threshold: 2,
        };

        let result = state
            .note_vote(attestation.clone(), &round_config, 1)
            .unwrap();
        assert_eq!(result, VoteImportResult::Ok);

        let votes = state.get_attestations_by_chain_and_header(1, 1).unwrap();

        assert_eq!(votes.len(), 1);
        assert_eq!(votes.get(&attestor.account_id), Some(&attestation));
    }

    #[test]
    fn test_double_vote() {
        let mut state = State::default();

        let account_id = AccountId32::new(H256::random().0);
        let attestor = Attestor::new(account_id);

        let attestation_data = simulate_attestation_data(1, 1);
        let attestation = create_signed_attestation(&attestor, attestation_data);

        let round_config = RoundConfig {
            committee_set_size: 1,
            target_sample_size: 1,
            threshold: 1,
        };

        state
            .note_vote(attestation.clone(), &round_config, 1)
            .unwrap();
        let result = state
            .note_vote(attestation.clone(), &round_config, 1)
            .unwrap();

        assert_eq!(result, VoteImportResult::DoubleVote);
    }

    #[test]
    fn test_round_conclusion() {
        let mut state = State::default();

        let account_id_1 = AccountId32::new(H256::random().0);
        let account_id_2 = AccountId32::new(H256::random().0);
        let attestor_1 = Attestor::new(account_id_1);
        let attestor_2 = Attestor::new(account_id_2);

        let attestation_data = simulate_attestation_data(1, 1);
        let attestation_1 = create_signed_attestation(&attestor_1, attestation_data.clone());
        let attestation_2 = create_signed_attestation(&attestor_2, attestation_data);

        let round_config = RoundConfig {
            committee_set_size: 2,
            target_sample_size: 2,
            threshold: 2,
        };

        state
            .note_vote(attestation_1.clone(), &round_config, 1)
            .unwrap();
        let result = state
            .note_vote(attestation_2.clone(), &round_config, 1)
            .unwrap();

        assert_eq!(result, VoteImportResult::RoundConcluded);
    }

    #[test]
    fn test_multiple_votes_different_headers() {
        let mut state = State::default();

        let account_id_1 = AccountId32::new(H256::random().0);
        let account_id_2 = AccountId32::new(H256::random().0);
        let attestor_1 = Attestor::new(account_id_1);
        let attestor_2 = Attestor::new(account_id_2);

        let attestation_data = simulate_attestation_data(1, 1);
        let attestation_1 = create_signed_attestation(&attestor_1, attestation_data.clone());

        let attestation_data = simulate_attestation_data(1, 2);
        let attestation_2 = create_signed_attestation(&attestor_2, attestation_data);

        let round_config = RoundConfig {
            committee_set_size: 2,
            target_sample_size: 2,
            threshold: 1,
        };

        state
            .note_vote(attestation_1.clone(), &round_config, 1)
            .unwrap();
        state
            .note_vote(attestation_2.clone(), &round_config, 1)
            .unwrap();

        let votes_header_1 = state.get_attestations_by_chain_and_header(1, 1).unwrap();
        let votes_header_2 = state.get_attestations_by_chain_and_header(1, 2).unwrap();

        assert_eq!(votes_header_1.len(), 1);
        assert_eq!(votes_header_2.len(), 1);
    }

    #[test]
    fn test_major_digest() {
        let mut state = State::default();

        let account_id_1 = AccountId32::new(H256::random().0);
        let account_id_2 = AccountId32::new(H256::random().0);
        let attestor_1 = Attestor::new(account_id_1);
        let attestor_2 = Attestor::new(account_id_2);

        let attestation_data = simulate_attestation_data(1, 1);
        let attestation_1 = create_signed_attestation(&attestor_1, attestation_data.clone());

        let attestation_data_2 = simulate_attestation_data(1, 1);
        let attestation_2 = create_signed_attestation(&attestor_2, attestation_data_2);

        let round_config = RoundConfig {
            committee_set_size: 2,
            target_sample_size: 2,
            threshold: 2,
        };

        state
            .note_vote(attestation_1.clone(), &round_config, 1)
            .unwrap();
        state
            .note_vote(attestation_2.clone(), &round_config, 1)
            .unwrap();

        let votes = state.get_attestations_by_chain_and_header(1, 1).unwrap();
        let (_major_digest, count) = find_major_digest(votes);

        assert!(
            count < round_config.threshold as usize,
            "Round should not conclude with differing votes"
        );
    }

    #[test]
    fn test_epoch_changes_clear_votes() {
        let mut state = State::default();

        let account_id = AccountId32::new(H256::random().0);
        let attestor = Attestor::new(account_id);

        let attestation_data = simulate_attestation_data(1, 1);
        let attestation = create_signed_attestation(&attestor, attestation_data);

        let round_config = RoundConfig {
            committee_set_size: 1,
            target_sample_size: 1,
            threshold: 1,
        };

        state
            .note_vote(attestation.clone(), &round_config, 1)
            .unwrap();

        assert!(
            !state.chain_head_votes.is_empty(),
            "Votes should exist before epoch change"
        );

        state
            .note_vote(attestation.clone(), &round_config, 2)
            .unwrap();

        let votes = state.get_attestations_by_chain_and_header(1, 1).unwrap();
        assert!(
            !votes.is_empty(),
            "Votes should be reset after epoch change"
        );
        assert_eq!(votes.len(), 1);
    }

    #[test]
    fn test_clear_votes() {
        let mut state = State::default();

        let account_id = AccountId32::new(H256::random().0);
        let attestor = Attestor::new(account_id);

        let attestation_data = simulate_attestation_data(1, 1);
        let attestation = create_signed_attestation(&attestor, attestation_data);

        let round_config = RoundConfig {
            committee_set_size: 1,
            target_sample_size: 1,
            threshold: 1,
        };

        state
            .note_vote(attestation.clone(), &round_config, 1)
            .unwrap();

        assert!(
            !state.chain_head_votes.is_empty(),
            "Votes should exist before clearing"
        );

        state.clear_votes(1, 1);

        let votes = state.get_attestations_by_chain_and_header(1, 1).unwrap();
        assert!(votes.is_empty(), "Votes should be cleared");
    }
}
