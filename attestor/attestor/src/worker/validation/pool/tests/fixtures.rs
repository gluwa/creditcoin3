use crate::worker::validation::pool::tests::constants::*;
use crate::worker::validation::pool::*;

#[rstest::fixture]
pub fn attestation(
    #[default([ATTESTOR_VALID_0])] attestors: impl IntoIterator<Item = attestor_primitives::AttestorId>,
    #[default(0)] header_number: common::types::Height,
    #[default(DIGEST_0)] prev_digest: attestor_primitives::Digest,
) -> AttestationVote {
    let mut iter = attestors.into_iter();

    let attestation =
        move |attestor: attestor_primitives::AttestorId| -> common::types::Attestation {
            common::types::Attestation {
                attestation_data: attestor_primitives::AttestationData {
                    header_number,
                    prev_digest: Some(prev_digest),
                    ..Default::default()
                },
                attestor,
                signature: Default::default(),
                signature_bls: attestor_primitives::bls::WrapEncode(
                    bls_signatures::PrivateKey::new(b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                        .sign(b"0xdeadbeef"),
                ),
                continuity_proof:
                    attestor_primitives::attestation_fragment::AttestationFragmentSerializable {
                        blocks: vec![attestor_primitives::block::BlockSerializable {
                            block_number: header_number,
                            root: attestor_primitives::Digest::default(),
                            prev_digest,
                            digest: attestor_primitives::Digest::default(),
                        }],
                    },
            }
        };

    let attestor = iter.next().unwrap();
    iter.fold(
        AttestationVote {
            votes: vec![attestation(attestor.clone())],
            signers: std::collections::BTreeSet::from([attestor.clone()]),
            attestation: attestation(attestor),
        },
        |mut vote, attestor| {
            vote.votes.push(attestation(attestor.clone()));
            vote.signers.insert(attestor);
            vote
        },
    )
}

#[rstest::fixture]
pub fn attestation_signed(attestation: AttestationVote) -> common::types::AttestationSigned {
    attestor_primitives::SignedAttestation {
        attestation: attestation.attestation.attestation_data,
        signature: [0u8; 96],
        attestors: attestation
            .votes
            .iter()
            .map(|att| att.attestor.clone())
            .collect(),
        continuity_proof: attestation.attestation.continuity_proof,
    }
}

#[rstest::fixture]
pub fn quorum(
    #[default([ATTESTOR_VALID_0])] _attestors: impl IntoIterator<Item = attestor_primitives::AttestorId>
        + Clone,
    #[default(0)] _header_number: common::types::Height,
    #[default(DIGEST_0)] _prev_digest: attestor_primitives::Digest,
    #[with(_attestors.clone(), _header_number, _prev_digest)] attestation: AttestationVote,
) -> Quorum {
    Quorum(attestation.votes)
}

#[rstest::fixture]
pub fn validate_quorum(#[default(2)] vote_count: usize) -> ValidateQuorum {
    ValidateQuorum {
        target_quorum: vote_count.try_into().unwrap(),
    }
}

#[rstest::fixture]
pub fn validate_attestor(
    #[default([ATTESTOR_VALID_0, ATTESTOR_VALID_1, ATTESTOR_VALID_2, ATTESTOR_VALID_3])]
        attestors: impl IntoIterator<Item = attestor_primitives::AttestorId>,
) -> ValidateAttestor {
    ValidateAttestor {
        attestor_set: attestors.into_iter().collect(),
    }
}

#[rstest::fixture]
pub fn attestors(
    #[default([ATTESTOR_VALID_0, ATTESTOR_VALID_1, ATTESTOR_VALID_2, ATTESTOR_VALID_3])]
    attestor_set: impl IntoIterator<Item = attestor_primitives::AttestorId>,
) -> Vec<cc_client::AccountId32> {
    attestor_set
        .into_iter()
        .map(|attestor| cc_client::AccountId32(attestor.public_key()))
        .collect()
}

#[rstest::fixture]
pub fn metrics() -> common::types::Metrics {
    let config = crate::worker::api::metrics::ConfigBuilder::new()
        .with_name("test")
        .with_address(cc_client::AccountId32([0; 32]))
        .with_peer_id(libp2p::PeerId::random())
        .with_chain_key(2u64)
        .with_start_height(common::types::Height::MIN)
        .with_start_attestation(None)
        .with_genesis(common::types::Height::MIN)
        .with_attestation_latest_eth(common::types::Height::MIN)
        .with_attestation_interval(std::num::NonZero::<common::types::Height>::MIN)
        .build();
    std::sync::Arc::new(crate::worker::api::metrics::Metrics::new(config))
}

#[rstest::fixture]
pub fn config(
    validate_quorum: ValidateQuorum,
    #[default(100)] capacity: usize,
    #[default([ATTESTOR_VALID_0, ATTESTOR_VALID_1, ATTESTOR_VALID_2, ATTESTOR_VALID_3])]
    attestors: impl IntoIterator<Item = attestor_primitives::AttestorId>,
    metrics: common::types::Metrics,
) -> Config {
    #[cfg(not(feature = "simulation"))]
    let config = ConfigBuilder::new()
        .with_max_size(std::num::NonZeroUsize::new(capacity).unwrap())
        .with_attestors(attestors.into_iter().collect::<Vec<_>>())
        .with_quorum(validate_quorum.target_quorum)
        .with_attestation_start(Some(stream::util::AttestationInfo {
            digest: DIGEST_0,
            height: common::types::Height::MIN,
        }))
        .with_metrics(metrics)
        .build();

    #[cfg(feature = "simulation")]
    let config = ConfigBuilder::new()
        .with_max_size(std::num::NonZeroUsize::new(capacity).unwrap())
        .with_attestors(attestors.into_iter().collect::<Vec<_>>())
        .with_quorum(validate_quorum.target_quorum)
        .with_attestation_start(Some(common::types::AttestationInfo {
            digest: DIGEST_0,
            height: common::types::Height::MIN,
        }))
        .build();

    config
}

#[rstest::fixture]
pub fn permit(
    #[default([ATTESTOR_VALID_0])] _attestors: impl IntoIterator<Item = attestor_primitives::AttestorId>
        + Clone,
    #[default(0)] _header_number: common::types::Height,
    #[default(DIGEST_0)] _prev_digest: attestor_primitives::Digest,
    #[with(_attestors.clone(), _header_number, _prev_digest)] attestation: AttestationVote,
) -> Permit {
    Permit(stream::util::AttestationInfo {
        height: attestation.attestation.header_number(),
        digest: attestation.attestation.digest(),
    })
}
