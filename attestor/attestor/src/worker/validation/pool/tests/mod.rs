use crate::common::fixtures::*;
use crate::worker::validation::pool::tests::constants::*;
use crate::worker::validation::pool::tests::fixtures::*;
use crate::worker::validation::pool::*;

mod constants;
mod fixtures;

#[tokio::test]
#[rstest::rstest]
#[timeout(TIMEOUT)]
async fn attestation_pool_sanity_mark_valid(
    _logs: (),
    #[from(attestation)]
    #[with([ATTESTOR_VALID_0], 0, DIGEST_0)]
    attestation_0: AttestationVote,
    #[from(attestation)]
    #[with([ATTESTOR_VALID_1], 0, DIGEST_0)]
    attestation_1: AttestationVote,
    #[from(attestation)]
    #[with([ATTESTOR_VALID_2], 0, DIGEST_1)]
    attestation_2: AttestationVote,
    #[from(quorum)]
    #[with([ATTESTOR_VALID_0, ATTESTOR_VALID_1], 0, DIGEST_0)]
    quorum_expected: Quorum,
    config: Config,
) {
    use futures::stream::StreamExt as _;

    let (sx, mut rx) = attestation_pool(config);

    assert!(sx.send(attestation_0.attestation.clone()).unwrap().is_ok());
    assert!(sx.send(attestation_1.attestation.clone()).unwrap().is_ok());
    assert!(sx.send(attestation_2.attestation.clone()).unwrap().is_ok());

    let (quorum_actual, permit, _digest_local) = rx.next().await.unwrap();

    assert_eq!(quorum_actual, quorum_expected);

    rx.mark_valid(permit);

    let mut pool = rx.common.pool.lock();
    let inner = pool.expect_open();

    assert!(!inner.forks.forks_by_height.contains(&KeyHeight {
        height: 0,
        size: 2,
        digest: DIGEST_0
    }));
    assert_eq!(
        inner.digest_local,
        Some(cc_client::H256(attestation_1.attestation.digest().0))
    );
}

#[tokio::test]
#[rstest::rstest]
#[timeout(TIMEOUT)]
async fn attestation_pool_sanity_mark_invalid(
    _logs: (),
    #[from(attestation)]
    #[with([ATTESTOR_VALID_0])]
    attestation_0: AttestationVote,
    #[from(attestation)]
    #[with([ATTESTOR_VALID_1])]
    attestation_1: AttestationVote,
    #[from(quorum)]
    #[with([ATTESTOR_VALID_0, ATTESTOR_VALID_1])]
    quorum_expected: Quorum,
    config: Config,
) {
    use futures::stream::StreamExt as _;

    let (sx, mut rx) = attestation_pool(config);

    assert!(sx.send(attestation_0.attestation.clone()).unwrap().is_ok());
    assert!(sx.send(attestation_1.attestation.clone()).unwrap().is_ok());

    let (quorum_actual, permit, _digest_local) = rx.next().await.unwrap();

    assert_eq!(quorum_actual, quorum_expected);
    rx.mark_invalid(permit);

    let mut pool = rx.common.pool.lock();
    let inner = pool.expect_open();

    assert!(inner.forks.votes_invalid.contains(&KeyDigest {
        height: attestation_0.attestation.header_number(),
        digest: attestation_0.attestation.digest()
    }));
}

#[tokio::test]
#[rstest::rstest]
#[timeout(TIMEOUT)]
async fn attestation_pool_mark_for_later(
    _logs: (),
    #[from(attestation)]
    #[with([ATTESTOR_VALID_0])]
    attestation_0: AttestationVote,
    #[from(attestation)]
    #[with([ATTESTOR_VALID_1])]
    attestation_1: AttestationVote,
    #[from(attestation_signed)] attestation_signed: common::types::AttestationSigned,
    #[from(quorum)]
    #[with([ATTESTOR_VALID_0, ATTESTOR_VALID_1])]
    quorum_expected: Quorum,
    config: Config,
) {
    use futures::stream::StreamExt as _;

    let (sx, mut rx) = attestation_pool(config);

    assert_matches::assert_matches!(rx.take_next_validated(), None);

    assert!(sx.send(attestation_0.attestation.clone()).unwrap().is_ok());
    assert!(sx.send(attestation_1.attestation.clone()).unwrap().is_ok());

    let (quorum_actual, permit, _digest_local) = rx.next().await.unwrap();

    assert_eq!(quorum_actual, quorum_expected);
    rx.mark_for_later(
        permit,
        attestation_signed.clone(),
        vec![
            attestation_0.attestation.clone(),
            attestation_1.attestation.clone(),
        ],
    );

    // Such types, much wow... -fuck subxt and the incompatible dependencies which make using
    // our own types an even more royal pain $$%%^#$#
    let attestation_expected: cc_client::cc3::runtime_types::attestor_primitives::SignedAttestation<
            cc_client::H256,
            cc_client::AccountId32,
        > = attestation_signed.clone().into();

    assert_matches::assert_matches!(rx.take_next_validated(), Some((height, digest, attestation, votes)) => {
        assert_eq!(height, attestation_0.attestation.header_number());
        assert_eq!(digest, attestation_0.attestation.digest());
        // Other types in this don't implement PartialEq and Eq...
        assert_eq!(attestation.attestors, attestation_expected.attestors);
        assert_eq!(votes,
            vec![
                attestation_0.attestation,
                attestation_1.attestation,
            ],
        );
    });

    assert_eq!(
        sx.common.pool.lock().expect_open().digest_local,
        Some(cc_client::H256(attestation_signed.digest().0))
    );
}

#[tokio::test]
#[rstest::rstest]
#[timeout(TIMEOUT)]
async fn attestation_pool_sanity_pending(
    _logs: (),
    #[from(attestation)]
    #[with([ATTESTOR_VALID_0], 1, DIGEST_1)]
    attestation_pending: AttestationVote,
    config: Config,
) {
    let (mut sx, rx) = attestation_pool(config);

    assert!(sx
        .send(attestation_pending.attestation.clone())
        .unwrap()
        .is_ok());

    {
        let mut pool = rx.common.pool.lock();
        let inner = pool.expect_open();

        assert_eq!(inner.forks.pending_by_digest.len(), 1);
        assert_eq!(inner.forks.pending_by_prev_digest_tail.len(), 1);
        assert_eq!(inner.forks.pending_by_height.len(), 1);
        assert!(inner
            .forks
            .pending_by_prev_digest_tail
            .contains(&KeyTailPending {
                prev_digest_tail: PrevDigestTail(DIGEST_1),
                height: 1,
                digest: attestation_pending.attestation.digest(),
            }));
    }

    sx.note_attestation_finalization(stream::util::AttestationInfo {
        digest: DIGEST_1,
        height: 0,
    })
    .unwrap();

    {
        let mut pool = rx.common.pool.lock();
        let inner = pool.expect_open();
        let vote = AttestationVote::new(attestation_pending.attestation.clone());

        assert_eq!(inner.forks.forks_best.clone().unwrap(), vote);
    }
}

#[tokio::test]
#[rstest::rstest]
#[timeout(TIMEOUT)]
async fn attestation_pool_sanity_err_invalid_attestor(
    #[with([ATTESTOR_INVALID])] attestation: AttestationVote,
    config: Config,
) {
    let (sx, _rx) = attestation_pool(config);

    assert_matches::assert_matches!(
        sx.send(attestation.attestation.clone()),
        Some(Err(Error::Unauthorized(ATTESTOR_INVALID, 0)))
    );
}

#[tokio::test]
#[rstest::rstest]
#[timeout(TIMEOUT)]
async fn attestation_pool_async_wake_receiver(
    _logs: (),
    #[with([ATTESTOR_VALID_0])] attestation: AttestationVote,
    #[with([ATTESTOR_VALID_0])] permit: Permit,
    #[with([ATTESTOR_VALID_0])] quorum: Quorum,
    #[from(validate_quorum)]
    #[with(1)]
    _quorum_validate: ValidateQuorum,
    #[with(_quorum_validate.clone())] config: Config,
) {
    use futures::stream::StreamExt as _;

    let (sx, mut rx) = attestation_pool(config);
    let mut fut = tokio_test::task::spawn(rx.next());

    tokio_test::assert_pending!(fut.poll());
    assert!(sx.send(attestation.attestation.clone()).unwrap().is_ok());
    tokio_test::assert_ready_eq!(fut.poll(), Some((quorum, permit, None)));
}

#[tokio::test]
#[rstest::rstest]
#[timeout(TIMEOUT)]
async fn attestation_pool_async_close(_logs: (), config: Config) {
    use futures::stream::StreamExt as _;

    let (sx, mut rx) = attestation_pool(config);
    let mut fut = tokio_test::task::spawn(rx.next());

    tokio_test::assert_pending!(fut.poll());
    sx.close();
    tokio_test::assert_ready_eq!(fut.poll(), None);
}

#[tokio::test]
#[rstest::rstest]
#[timeout(TIMEOUT)]
async fn attestation_pool_quorum_basic(
    _logs: (),
    #[from(attestation)]
    #[with([ATTESTOR_VALID_0])]
    attestation_0: AttestationVote,
    #[from(attestation)]
    #[with([ATTESTOR_VALID_1])]
    attestation_1: AttestationVote,
    #[from(quorum)]
    #[with([ATTESTOR_VALID_0, ATTESTOR_VALID_1])]
    quorum: Quorum,
    #[from(permit)]
    #[with([ATTESTOR_VALID_0, ATTESTOR_VALID_1])]
    permit: Permit,
    config: Config,
) {
    use futures::stream::StreamExt as _;

    let (sx, mut rx) = attestation_pool(config);

    assert!(sx.send(attestation_0.attestation.clone()).unwrap().is_ok());
    assert!(sx.send(attestation_1.attestation.clone()).unwrap().is_ok());

    let actual = rx.next().await;
    let expected = Some((quorum, permit, None));

    assert_eq!(actual, expected);
}

#[tokio::test]
#[rstest::rstest]
#[timeout(TIMEOUT)]
#[allow(clippy::too_many_arguments)]
async fn attestation_pool_quorum_highest(
    _logs: (),
    #[from(attestation)]
    #[with([ATTESTOR_VALID_0])]
    attestation_0: AttestationVote,
    #[from(attestation)]
    #[with([ATTESTOR_VALID_1])]
    attestation_1: AttestationVote,
    #[from(attestation)]
    #[with([ATTESTOR_VALID_2])]
    attestation_2: AttestationVote,
    #[from(attestation)]
    #[with([ATTESTOR_VALID_0], 100)]
    attestation_3: AttestationVote,
    #[from(attestation)]
    #[with([ATTESTOR_VALID_1], 100)]
    attestation_4: AttestationVote,
    #[from(quorum)]
    #[with([ATTESTOR_VALID_0, ATTESTOR_VALID_1], 100)]
    quorum: Quorum,
    #[from(permit)]
    #[with([ATTESTOR_VALID_0, ATTESTOR_VALID_1], 100)]
    permit: Permit,
    config: Config,
) {
    use futures::stream::StreamExt as _;

    let (sx, mut rx) = attestation_pool(config);

    // Source chain height 0
    assert!(sx.send(attestation_0.attestation.clone()).unwrap().is_ok());
    assert!(sx.send(attestation_1.attestation.clone()).unwrap().is_ok());
    assert!(sx.send(attestation_2.attestation.clone()).unwrap().is_ok());

    // Source chain height 100
    assert!(sx.send(attestation_3.attestation.clone()).unwrap().is_ok());
    assert!(sx.send(attestation_4.attestation.clone()).unwrap().is_ok());

    // NOTE: even though quorum 1 has LESS votes, it still passes the quorum threshold of 2.
    // The attestation pool always favors the HIGHEST quorum so as to improve catchup speed.

    let actual = rx.next().await;
    let expected = Some((quorum, permit, None));

    assert_eq!(actual, expected);
}

#[tokio::test]
#[rstest::rstest]
#[timeout(TIMEOUT)]
async fn attestation_pool_evict_pending(
    _logs: (),
    #[from(attestation)]
    #[with([ATTESTOR_VALID_0])]
    attestation_0: AttestationVote,
    #[from(attestation)]
    #[with([ATTESTOR_VALID_1], 1, DIGEST_1)]
    attestation_1: AttestationVote,
    #[from(attestation)]
    #[with([ATTESTOR_VALID_2])]
    attestation_2: AttestationVote,
    #[from(validate_quorum)]
    #[with(1)]
    _quorum_validate: ValidateQuorum,
    #[from(config)]
    #[with(_quorum_validate.clone(), 2)]
    config: Config,
) {
    let (sx, rx) = attestation_pool(config);

    assert!(sx
        .send(attestation_0.attestation.clone())
        .unwrap()
        .as_ref()
        .is_ok_and(Vec::is_empty));
    assert!(sx
        .send(attestation_1.attestation.clone())
        .unwrap()
        .as_ref()
        .is_ok_and(Vec::is_empty));

    assert_eq!(
        sx.send(attestation_2.attestation.clone()).unwrap().unwrap(),
        vec![attestation_1.attestation.clone()]
    );

    let mut pool = rx.common.pool.lock();
    let inner = pool.expect_open();

    assert!(inner.forks.pending_by_prev_digest_tail.is_empty());
    assert_eq!(inner.forks.pending_by_digest.len(), 0);
    assert_eq!(inner.forks.pending_by_prev_digest_tail.len(), 0);
    assert_eq!(inner.forks.pending_by_height.len(), 0);
    assert_eq!(inner.forks.forks_by_height.len(), 1);
    assert_eq!(inner.forks.forks_by_size.len(), 1);
    assert!(inner.forks.forks_by_height.contains(&KeyHeight {
        height: 0,
        size: 2,
        digest: attestation_0.attestation.digest()
    }));
    assert!(inner.forks.forks_by_size.contains(&KeySize {
        size: 2,
        height: 0,
        digest: attestation_0.attestation.digest()
    }));
}

#[tokio::test]
#[rstest::rstest]
#[timeout(TIMEOUT)]
async fn attestation_pool_evict_fork(
    _logs: (),
    #[from(attestation)]
    #[with([ATTESTOR_VALID_0])]
    attestation_0: AttestationVote,
    #[from(attestation)]
    #[with([ATTESTOR_VALID_1])]
    attestation_1: AttestationVote,
    #[from(attestation)]
    #[with([ATTESTOR_VALID_2], 1)]
    attestation_2: AttestationVote,
    #[from(attestation)]
    #[with([ATTESTOR_VALID_3])]
    attestation_3: AttestationVote,
    #[from(validate_quorum)]
    #[with(1)]
    _quorum_validate: ValidateQuorum,
    #[from(config)]
    #[with(_quorum_validate.clone(), 3)]
    config: Config,
) {
    let (sx, rx) = attestation_pool(config);

    assert!(sx
        .send(attestation_0.attestation.clone())
        .unwrap()
        .as_ref()
        .is_ok_and(Vec::is_empty));
    assert!(sx
        .send(attestation_1.attestation.clone())
        .unwrap()
        .as_ref()
        .is_ok_and(Vec::is_empty));
    assert!(sx
        .send(attestation_2.attestation.clone())
        .unwrap()
        .as_ref()
        .is_ok_and(Vec::is_empty));

    {
        let mut pool = rx.common.pool.lock();
        let inner = pool.expect_open();

        assert_eq!(inner.forks.forks_by_size.len(), 2);
    }

    assert_eq!(
        sx.send(attestation_3.attestation.clone()).unwrap().unwrap(),
        vec![attestation_2.attestation.clone()]
    );

    {
        let mut pool = rx.common.pool.lock();
        let inner = pool.expect_open();

        assert!(!inner
            .forks
            .forks_by_digest
            .contains_key(&attestation_2.attestation.digest()));
        assert!(inner
            .forks
            .forks_by_digest
            .contains_key(&attestation_3.attestation.digest()));
        assert_eq!(inner.forks.forks_by_height.len(), 1);
        assert_eq!(inner.forks.forks_by_size.len(), 1);
        assert!(inner.forks.forks_by_height.contains(&KeyHeight {
            height: 0,
            size: 3,
            digest: attestation_0.attestation.digest()
        }));
        assert!(inner.forks.forks_by_size.contains(&KeySize {
            size: 3,
            height: 0,
            digest: attestation_0.attestation.digest()
        }));
    }
}

#[tokio::test]
#[rstest::rstest]
#[timeout(TIMEOUT)]
async fn attestation_pool_evict_fail(
    _logs: (),
    #[from(attestation)]
    #[with([ATTESTOR_VALID_0])]
    attestation_0: AttestationVote,
    #[from(attestation)]
    #[with([ATTESTOR_VALID_1])]
    attestation_1: AttestationVote,
    #[from(attestation)]
    #[with([ATTESTOR_VALID_2], 1)]
    attestation_2: AttestationVote,
    #[from(validate_quorum)]
    #[with(1)]
    _quorum_validate: ValidateQuorum,
    #[from(config)]
    #[with(_quorum_validate.clone(), 2)]
    config: Config,
) {
    let (sx, rx) = attestation_pool(config);

    assert!(sx
        .send(attestation_0.attestation.clone())
        .unwrap()
        .as_ref()
        .is_ok_and(Vec::is_empty));
    assert!(sx
        .send(attestation_1.attestation.clone())
        .unwrap()
        .as_ref()
        .is_ok_and(Vec::is_empty));

    assert_matches::assert_matches!(
        sx.send(attestation_2.attestation.clone()).unwrap(),
        Err(Error::NoSpaceLeft(address, height)) => {
            assert_eq!(&attestation_2.attestation.attestor, &address);
            assert_eq!(attestation_2.attestation.header_number(), height);
        }
    );

    let mut pool = rx.common.pool.lock();
    let inner = pool.expect_open();

    assert_eq!(inner.forks.forks_by_size.len(), 1);
    assert_eq!(inner.forks.forks_by_digest.len(), 1);
    assert_eq!(inner.forks.votes.len(), 2);
    assert!(inner.forks.forks_by_height.contains(&KeyHeight {
        height: 0,
        size: 2,
        digest: attestation_0.attestation.digest()
    }));
    assert!(inner.forks.forks_by_size.contains(&KeySize {
        size: 2,
        height: 0,
        digest: attestation_0.attestation.digest()
    }));
}

#[tokio::test]
#[rstest::rstest]
#[timeout(TIMEOUT)]
async fn attestation_pool_close_sender(
    _logs: (),
    #[with([ATTESTOR_VALID_1])] attestation: AttestationVote,
    config: Config,
) {
    let (sx, rx) = attestation_pool(config);
    rx.close();
    assert_matches::assert_matches!(sx.send(attestation.attestation.clone()), None);
}

#[tokio::test]
#[rstest::rstest]
#[timeout(TIMEOUT)]
async fn attestation_pool_close_receiver(
    _logs: (),
    #[with([ATTESTOR_VALID_1])] attestation: AttestationVote,
    config: Config,
) {
    use futures::stream::StreamExt as _;

    let (sx, mut rx) = attestation_pool(config);
    assert!(sx.send(attestation.attestation.clone()).unwrap().is_ok());

    sx.close();
    assert!(rx.next().await.is_none());
}

#[rstest::rstest]
fn quorum_parameters_validate(
    _logs: (),
    #[from(attestation)]
    #[with([ATTESTOR_VALID_0, ATTESTOR_VALID_1])]
    attestation_0: AttestationVote,
    #[from(attestation)]
    #[with([ATTESTOR_VALID_0])]
    attestation_1: AttestationVote,
    validate_quorum: ValidateQuorum,
) {
    assert!(validate_quorum.validate(&attestation_0));
    assert!(!validate_quorum.validate(&attestation_1));
}

#[rstest::rstest]
fn validator_parameters_validate(
    _logs: (),
    #[from(attestation)]
    #[with([ATTESTOR_VALID_0])]
    attestation_0: AttestationVote,
    #[from(attestation)]
    #[with([ATTESTOR_INVALID])]
    attestation_2: AttestationVote,
    validate_attestor: ValidateAttestor,
) {
    assert!(validate_attestor
        .validate(&attestation_0.attestation)
        .is_ok());
    assert_matches::assert_matches!(
        validate_attestor.validate(&attestation_2.attestation),
        Err(Error::Unauthorized(ATTESTOR_INVALID, 0))
    );
}

#[tokio::test]
#[rstest::rstest]
#[timeout(TIMEOUT)]
#[allow(clippy::too_many_arguments)]
async fn chain_reversion_resets_validation_pool(
    _logs: (),
    // Attestations that will be marked valid via mark_for_later
    #[from(attestation)]
    #[with([ATTESTOR_VALID_0], 0, DIGEST_0)]
    attestation_0: AttestationVote,
    #[from(attestation)]
    #[with([ATTESTOR_VALID_1], 0, DIGEST_0)]
    attestation_1: AttestationVote,
    // Attestations that will be marked invalid
    #[from(attestation)]
    #[with([ATTESTOR_VALID_0], 1, DIGEST_0)]
    attestation_2: AttestationVote,
    #[from(attestation)]
    #[with([ATTESTOR_VALID_1], 1, DIGEST_0)]
    attestation_3: AttestationVote,
    // Attestations that will remain in forks after removals
    #[from(attestation)]
    #[with([ATTESTOR_VALID_2], 2, DIGEST_0)]
    attestation_4: AttestationVote,
    #[from(attestation)]
    #[with([ATTESTOR_VALID_3], 2, DIGEST_0)]
    attestation_5: AttestationVote,
    // Attestation that will be entered into pending
    #[from(attestation)]
    #[with([ATTESTOR_VALID_2], 1, DIGEST_1)]
    attestation_pending: AttestationVote,
    #[from(validate_quorum)]
    #[with(2)]
    _quorum_validate: ValidateQuorum,
    #[from(config)]
    #[with(_quorum_validate.clone(), 5)]
    config: Config,
) {
    use futures::stream::StreamExt as _;

    let (mut sx, mut rx) = attestation_pool(config);

    // ------------------------------------------------------------------------
    // 1) Create a quorum and mark it for later.
    //    This populates:
    //      - valid.quorums_valid
    //      - digest_local
    // ------------------------------------------------------------------------
    assert!(sx
        .send(attestation_0.attestation.clone())
        .unwrap()
        .as_ref()
        .is_ok_and(Vec::is_empty));
    assert!(sx
        .send(attestation_1.attestation.clone())
        .unwrap()
        .as_ref()
        .is_ok_and(Vec::is_empty));

    let (_quorum_high, permit_0, _digest_local) = rx.next().await.unwrap();

    let attestation_signed_0 = common::types::AttestationSigned {
        attestation: attestation_0.attestation.attestation_data.clone(),
        signature: [0u8; 96],
        attestors: vec![
            attestation_0.attestation.attestor.clone(),
            attestation_1.attestation.attestor.clone(),
        ],
        continuity_proof: attestation_0.attestation.continuity_proof.clone(),
    };

    rx.mark_for_later(
        permit_0,
        attestation_signed_0,
        vec![
            attestation_0.attestation.clone(),
            attestation_1.attestation.clone(),
        ],
    );

    // ------------------------------------------------------------------------
    // 2) Create another quorum and mark it invalid.
    //    This populates votes_invalid.
    // ------------------------------------------------------------------------
    assert!(sx
        .send(attestation_2.attestation.clone())
        .unwrap()
        .as_ref()
        .is_ok_and(Vec::is_empty));
    assert!(sx
        .send(attestation_3.attestation.clone())
        .unwrap()
        .as_ref()
        .is_ok_and(Vec::is_empty));

    let (_quorum_low, permit_1, _digest_local) = rx.next().await.unwrap();
    rx.mark_invalid(permit_1);

    // ------------------------------------------------------------------------
    // 3) Create another quorum and leave it in forks.
    // This populates forks_by_digest / forks_by_height / forks_by_size / quorums_by_height / votes
    // ------------------------------------------------------------------------
    assert!(sx
        .send(attestation_4.attestation.clone())
        .unwrap()
        .as_ref()
        .is_ok_and(Vec::is_empty));
    assert!(sx
        .send(attestation_5.attestation.clone())
        .unwrap()
        .as_ref()
        .is_ok_and(Vec::is_empty));

    // ------------------------------------------------------------------------
    // 4) Add a pending attestation.
    //    This populates:
    //      - pending_by_digest / pending_by_prev_digest_tail / pending_by_height
    //      - attestation_delay.time
    // ------------------------------------------------------------------------
    assert!(sx
        .send(attestation_pending.attestation.clone())
        .unwrap()
        .is_ok());

    // Sanity-check that we actually populated the structures before reversion.
    {
        let mut pool = rx.common.pool.lock();
        let inner = pool.expect_open();

        assert!(inner.digest_local.is_some());

        assert!(!inner.forks.forks_by_digest.is_empty());
        assert!(!inner.forks.forks_by_height.is_empty());
        assert!(!inner.forks.forks_by_size.is_empty());
        assert!(inner.forks.forks_best.is_some());

        assert!(!inner.forks.pending_by_digest.is_empty());
        assert!(!inner.forks.pending_by_prev_digest_tail.is_empty());
        assert!(!inner.forks.pending_by_height.is_empty());

        assert!(!inner.forks.votes.is_empty());
        assert!(!inner.forks.votes_invalid.is_empty());
        assert!(!inner.forks.quorums_by_height.is_empty());

        assert!(!inner.valid.quorums_valid.is_empty());
        assert!(!inner.attestation_delay.time.is_empty());
    }

    // ------------------------------------------------------------------------
    // 5) Revert the chain and verify everything is cleared/reset.
    // ------------------------------------------------------------------------
    let reversion_info = stream::util::AttestationInfo {
        height: 50,
        digest: DIGEST_1,
    };

    sx.note_attestation_chain_reversion(reversion_info);

    {
        let mut pool = rx.common.pool.lock();
        let inner = pool.expect_open();

        // Digest local reset
        assert_eq!(inner.digest_local, None);

        // Forks reset
        assert!(inner.forks.forks_by_digest.is_empty());
        assert!(inner.forks.forks_by_height.is_empty());
        assert!(inner.forks.forks_by_size.is_empty());
        assert_eq!(inner.forks.forks_best, None);

        assert!(inner.forks.pending_by_digest.is_empty());
        assert!(inner.forks.pending_by_prev_digest_tail.is_empty());
        assert!(inner.forks.pending_by_height.is_empty());

        assert!(inner.forks.votes.is_empty());
        assert!(inner.forks.votes_invalid.is_empty());
        assert!(inner.forks.quorums_by_height.is_empty());

        // Reversion should set the new finalized digest
        assert_eq!(inner.forks.last_finalized_digest, Some(DIGEST_1));

        // Valid queue reset
        assert!(inner.valid.quorums_valid.is_empty());

        // Delay tracking reset
        assert!(inner.attestation_delay.time.is_empty());
    }
}
