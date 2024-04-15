pub mod crypto;
pub mod random;

use std::{collections::HashMap, sync::atomic::AtomicBool};

use arbitrary::Arbitrary;
use crypto::{AggregatableScheme, CryptoScheme, PublicFor};
use parity_scale_codec::{Decode, Encode};
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use sp_core::{
    crypto::{AccountId32, VrfPublic, VrfSecret},
    sr25519::{
        self,
        vrf::{VrfSignData, VrfSignature, VrfTranscript},
    },
    H256,
};

#[derive(Encode, Decode, Arbitrary, Debug, Clone, PartialEq)]
/// Vote data for a given block on the source chain, to be signed by the attestor/voter
pub struct VoteData {
    /// Identifier of the chain that the block belongs to
    pub chain_id: u32,
    /// Height of the source chain block
    pub height: u64,
    /// Root hash of the source chain block
    #[arbitrary(with = random::arbitrary_h256)]
    pub hash: H256,
}

#[derive(Encode, Decode, Debug, Clone)]
/// A signed vote for a source chain block
pub struct Vote<S> {
    /// The signature of the vote data
    pub signature: S,
    /// VRF signature showing that the attestor was selected
    /// to participate in voting
    pub proof_of_inclusion: VrfSignature,
    /// Identifier of the attestor. Used to look up
    /// public keys and verify the attestor is actually an attestor
    pub attestor_id: AccountId32,
}

#[derive(Encode, Decode, Debug, Arbitrary)]
pub struct Randomness([u8; 32]);

#[derive(Encode, Decode, Debug, Arbitrary)]
pub struct Context {
    /// Randomness provided by the execution chain
    randomness: Randomness,
}

// For now everyone is selected
const THRESHOLD: u128 = u128::MAX;

const VRF_CONTEXT: &[u8] = b"testing-vrf";

/// create the proof/signature showing that the attestor
/// was selected to participate in voting
///
/// In other words, a proof that the attestor's keys
/// produce a random number that is <= `THRESHOLD`.
pub fn make_proof_of_inclusion(
    ctx: &Context,
    keys: &sr25519::Pair,
    attestor_id: &AccountId32,
    source_block_height: u64,
) -> VrfSignature {
    let transcript = make_transcript(ctx, attestor_id, source_block_height);
    let random = keys.make_bytes(VRF_CONTEXT, &transcript);
    let random = u128::from_le_bytes(random);
    if random > THRESHOLD {
        panic!("random too big");
    }
    let sign_data = VrfSignData::new(transcript);
    keys.vrf_sign(&sign_data)
}

/// Produce a signed vote for the given vote data
pub fn make_vote<C: CryptoScheme>(
    ctx: &Context,
    pair: &C::KeyPair,
    vrf_pair: &sr25519::Pair,
    data: &VoteData,
    attestor_id: &AccountId32,
) -> Vote<C::Signature> {
    let signature = C::sign(pair, &data.encode());
    let proof_of_inclusion = make_proof_of_inclusion(ctx, vrf_pair, &attestor_id, data.height);
    Vote {
        signature,
        proof_of_inclusion,
        attestor_id: attestor_id.clone(),
    }
}

/// Make the transcript for the VRF used for attestor selection
fn make_transcript(
    ctx: &Context,
    attestor_id: &AccountId32,
    source_block_height: u64,
) -> VrfTranscript {
    VrfTranscript::new(
        b"test",
        &[
            (b"source_block_height", &source_block_height.encode()),
            (b"randomness", &ctx.randomness.encode()),
            (b"id", &attestor_id.encode()),
        ],
    )
}

/// Verify the proof that the attestor was selected to participate
pub fn verify_proof_of_inclusion(
    ctx: &Context,
    vrf_public: &sr25519::Public,
    proof_of_inclusion: &VrfSignature,
    attestor_id: &AccountId32,
    source_block_height: u64,
) -> bool {
    let vrf_input = VrfSignData::new(make_transcript(ctx, attestor_id, source_block_height));

    if !vrf_public.vrf_verify(&vrf_input, &proof_of_inclusion) {
        return false;
    }

    let Ok(random_pub) =
        proof_of_inclusion
            .pre_output
            .make_bytes(VRF_CONTEXT, vrf_input.as_ref(), &vrf_public)
        else {
            return false;
        };

    let random_pub = u128::from_le_bytes(random_pub);

    if random_pub > THRESHOLD {
        return false;
    }

    true
}

pub fn verify_vote_signature<C: CryptoScheme>(
    public: &PublicFor<C>,
    vote: &Vote<C::Signature>,
    vote_data_encoded: &[u8],
) -> bool {
    let Vote { signature, .. } = vote;

    if !C::verify(public, signature, vote_data_encoded) {
        return false;
    }

    true
}

pub fn verify_vote<C: CryptoScheme>(
    ctx: &Context,
    public: &PublicFor<C>,
    vrf_public: &sr25519::Public,
    vote: &Vote<C::Signature>,
    vote_data: &VoteData,
) -> bool {
    let vote_data_encoded = vote_data.encode();
    if !verify_vote_signature::<C>(public, vote, &vote_data_encoded) {
        return false;
    }

    if !verify_proof_of_inclusion(
        ctx,
        vrf_public,
        &vote.proof_of_inclusion,
        &vote.attestor_id,
        vote_data.height,
    ) {
        return false;
    }

    true
}

#[derive(Encode, Decode, Debug)]
pub struct Attestation<S> {
    pub data: VoteData,
    pub votes: Vec<Vote<S>>,
}

/// a little helper struct for `AggregatedAttestation`, as it doesn't need
/// to duplicate the vote data for every attestor - it just needs to verify the
/// proof of their inclusion
#[derive(Encode, Debug)]
pub struct AttestorInclusion {
    pub attestor_id: AccountId32,
    pub proof_of_inclusion: VrfSignature,
}

#[derive(Encode, Debug)]
pub struct AggregatedAttestation<S> {
    pub data: VoteData,
    pub aggregate_signature: S,
    pub inclusions: Vec<AttestorInclusion>,
    #[allow(dead_code)]
    pub continuity_proof: Vec<u8>,
}

pub fn make_attestation<C: CryptoScheme>(
    votes: Vec<Vote<C::Signature>>,
    data: VoteData,
) -> Attestation<C::Signature> {
    Attestation { data, votes }
}

pub fn make_aggregated_attestation<C: AggregatableScheme>(
    votes: Vec<Vote<C::Signature>>,
    data: VoteData,
) -> AggregatedAttestation<C::Signature> {
    let continuity_proof = vec![];

    let signatures = votes
        .iter()
        .map(|vote| vote.signature.clone())
        .collect::<Vec<_>>();

    let aggregate_signature = C::make_aggregate(&signatures);

    let inclusions = votes
        .iter()
        .map(|vote| AttestorInclusion {
            attestor_id: vote.attestor_id.clone(),
            proof_of_inclusion: vote.proof_of_inclusion.clone(),
        })
        .collect::<Vec<_>>();

    AggregatedAttestation {
        data,
        aggregate_signature,
        inclusions,
        continuity_proof,
    }
}

#[derive(Clone)]
pub struct PubKeys<P> {
    pub vrf_public: sr25519::Public,
    pub public: P,
}

pub fn verify_attestation<C: CryptoScheme>(
    ctx: &Context,
    attest: &Attestation<C::Signature>,
    pubkeys: &HashMap<AccountId32, PubKeys<PublicFor<C>>>,
) -> bool {
    let Attestation { votes, data } = attest;

    votes.into_par_iter().all(|vote| {
        let pubs = &pubkeys[&vote.attestor_id];
        // the votes need to all be for the same data (chain, header number and hash)!
        verify_vote::<C>(ctx, &pubs.public, &pubs.vrf_public, vote, &data)
    })
}

pub fn verify_attestation_serial<C: CryptoScheme>(
    ctx: &Context,
    attest: &Attestation<C::Signature>,
    pubkeys: &HashMap<AccountId32, PubKeys<PublicFor<C>>>,
) -> bool {
    let Attestation { votes, data } = attest;

    let votes: &[Vote<C::Signature>] = votes;

    votes.iter().all(|vote| {
        let pubs = &pubkeys[&vote.attestor_id];
        // the votes need to all be for the same data (chain, header number and hash)!
        verify_vote::<C>(ctx, &pubs.public, &pubs.vrf_public, vote, &data)
    })
}

pub fn verify_aggregated_attestation<C: AggregatableScheme>(
    ctx: &Context,
    attest: &AggregatedAttestation<C::Signature>,
    pubkeys: &HashMap<AccountId32, PubKeys<PublicFor<C>>>,
) -> bool {
    let AggregatedAttestation {
        data,
        aggregate_signature,
        inclusions,
        continuity_proof: _,
    } = attest;

    let is_valid = AtomicBool::new(true);
    let pubs: Vec<_> = inclusions
        .par_iter()
        .map(
            |AttestorInclusion {
                 attestor_id,
                 proof_of_inclusion,
             }| {
                let PubKeys { vrf_public, public } = pubkeys[attestor_id].clone();

                if !verify_proof_of_inclusion(
                    ctx,
                    &vrf_public,
                    proof_of_inclusion,
                    attestor_id,
                    data.height,
                ) {
                    is_valid.store(false, std::sync::atomic::Ordering::Relaxed);
                }

                public
            },
        )
        .collect();
    if !is_valid.load(std::sync::atomic::Ordering::Relaxed) {
        return false;
    }

    if !C::aggregate_verify(&pubs, aggregate_signature, data.encode().as_ref()) {
        return false;
    }

    true
}