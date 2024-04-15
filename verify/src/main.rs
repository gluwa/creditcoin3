use verify::{
    crypto::{Bls, Sr25519},
    make_aggregated_attestation, make_attestation, make_proof_of_inclusion, make_vote, random,
    Context, VoteData,
};
use parity_scale_codec::Encode;
use sp_core::{sr25519, Pair};

fn main() {
    let data = random::random::<VoteData>();
    let ctx = random::random::<Context>();

    let mut votes = Vec::new();
    for _ in 0..1000 {
        let attestor = random::random::<[u8; 32]>().into();
        let (pair, _) = sr25519::Pair::generate();
        let vote = make_vote::<Sr25519>(&ctx, &pair, &pair, &data, &attestor);
        votes.push(vote);
    }
    let attest = make_attestation::<Sr25519>(votes, data.clone());
    let encoded = attest.encode();

    let mut votes = Vec::new();
    let mut rng = rand::thread_rng();
    for _ in 0..1000 {
        let attestor = random::random::<[u8; 32]>().into();
        let (vrf_pair, _) = sr25519::Pair::generate();
        let pair = bls_signatures::PrivateKey::generate(&mut rng);
        let vote = make_vote::<Bls>(&ctx, &pair, &vrf_pair, &data, &attestor);
        votes.push(vote);
    }
    let aggregated = make_aggregated_attestation::<Bls>(votes, data.clone());
    let aggregated_encoded = aggregated.encode();

    let aggregated_base = aggregated_encoded.len() - aggregated.inclusions.encode().len();

    let non_aggregated_base_size = encoded.len() - attest.votes.encode().len();

    let inclusion_size = {
        let attestor = random::random::<[u8; 32]>().into();
        let (vrf_pair, _) = sr25519::Pair::generate();
        make_proof_of_inclusion(&ctx, &vrf_pair, &attestor, data.height)
            .encode()
            .len()
    };
    println!(
        "{: <45}: {} bytes",
        "Proof of inclusion size", inclusion_size
    );
    println!("{: <45}: {} bytes", "Vote data size", data.encode().len());
    println!(
        "{: <45}: {} bytes",
        "Non-aggregated base size", non_aggregated_base_size
    );
    println!(
        "{: <45}: {} bytes",
        "Non-aggregated size per vote",
        attest.votes[0].encode().len()
    );
    println!("{: <45}: {} bytes", "Aggregated base size", aggregated_base);
    println!(
        "{: <45}: {} bytes",
        "Aggregated size per vote",
        aggregated.inclusions[0].encode().len()
    );
    println!(
        "{: <45}: {} bytes",
        "Non-aggregated sr25519 (1000 votes)",
        encoded.len()
    );
    println!(
        "{: <45}: {} bytes",
        "Aggregated bls (1000 votes)",
        aggregated_encoded.len()
    );
}