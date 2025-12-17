#![doc = include_str!("../../../../mermaid.html")]
//! A [`Worker`] thread responsible for the validation and submission of attestations which have
//! reached [`Quorum`].
//!
//! # Quorum
//!
//! The validation worker receives attestations sent over to it by the [production worker] and the
//! [p2p worker]. Attestations are ordered by the [attestation pool] and are evaluated lazily only
//! once quorum has been reached.
//!
//! Once an attestation has reached quorum, it is validated locally to make sure that all its
//! attestors are eligible and that its digest and continuity proof follow the attestation chain.
//! Once that is done, the BLS signature of each attestation is aggregated into a single succinct
//! proof which can be used by the runtime to validate quorum.
//!
//! # Submission
//!
//! Valid attestations are eagerly submitted, that is to say we commit an attestation as soon as
//! the runtime can make further progress in validation. To avoid idling while the runtime is
//! validating a past attestation, the validation worker will keep validating attestations ahead of
//! finality, checking multiple attestations locally _ahead of time_ so as to be able to submit them
//! once the runtime is done with any previous attestations.
//!
//! # Finalization
//!
//! To avoid DOSing the runtime, attestors are randomly selected for submission via a VRF threshold
//! computation. To handle the edge case of no attestor being selected, a max finalization delay of
//! [`ATTESTATION_TIMEOUT`] is set, after which the attestations will be re-submitted with a
//! different VRF computation.
//!
//! # Attestation submission flow
//!
//! <pre class="mermaid">
//! sequenceDiagram
//!     box Networks
//!         participant CC3
//!     end
//!     box Thread 4
//!         participant Validation Worker
//!     end
//!     box Shared
//!         participant Attestation Pool
//!     end
//!
//!     loop Validation
//!         Validation Worker ->> Attestation Pool: Polls
//!
//!         activate Attestation Pool
//!         Attestation Pool ->> Validation Worker: Quorum
//!         deactivate Attestation Pool
//!
//!         activate Validation Worker
//!         Validation Worker ->> Validation Worker: Validate
//!         Validation Worker ->> Validation Worker: Check eligibility
//!         Validation Worker ->> CC3: Submit Attestation
//!         activate CC3
//!
//!         loop wait on submission
//!             loop validating
//!                 Validation Worker ->> Attestation Pool: Polls
//!
//!                 activate Attestation Pool
//!                 Attestation Pool ->> Validation Worker: Quorum
//!                 deactivate Attestation Pool
//!
//!                 Validation Worker ->> Validation Worker: Validate
//!                 Validation Worker ->> Attestation Pool: Store
//!             end
//!
//!             CC3 ->> Validation Worker: Confirm Finalization
//!             deactivate CC3
//!
//!             Validation Worker ->> CC3: Submit next valid attestation
//!             deactivate Validation Worker
//!         end
//!     end
//! </pre>
//!
//! [`Worker`]: crate::worker::Worker
//! [`Quorum`]: pool::Quorum
//! [production worker]: crate::worker::production
//! [p2p worker]: crate::worker::p2p
//! [attestation pool]: pool
//! [`ATTESTATION_TIMEOUT`]: common::constants::ATTESTATION_TIMEOUT

mod error;
mod future;
pub mod pool;

use crate::prelude::*;
pub use error::*;

// -------------------------------------- [ Configuration ] ------------------------------------ //

#[derive(attestor_macro::Builder)]
pub struct Config {
    cc3: crate::chain_listener::cc3::CC3,
    receiver_validation: pool::AttestationPoolReceiver,
    receiver_attestation_latest: tokio::sync::watch::Receiver<Option<common::types::Height>>,
    api_calls: cc_client::cc3::runtime_apis::RuntimeApi,
    api: subxt::OnlineClient<subxt::SubstrateConfig>,
    keypair: subxt_signer::sr25519::Keypair,
    start_height: common::types::Height,
}

// ----------------------------------------- [ Worker ] ---------------------------------------- //

pub(crate) struct WorkerAttestationValidation {
    // CHAIN LISTENERS
    cc3: crate::chain_listener::cc3::CC3,

    // ATTESTATIONS
    keypair: subxt_signer::sr25519::Keypair,
    watch_submission: future::OptionFuture<(AttestationSubmission, common::types::Height)>,
    attempts: usize,

    // MESSAGE CHANNELS
    receiver_validation: pool::AttestationPoolReceiver,
    receiver_attestation_latest: tokio::sync::watch::Receiver<Option<common::types::Height>>,

    // CHAIN DATA
    api_calls: cc_client::cc3::runtime_apis::RuntimeApi,
    api: subxt::OnlineClient<subxt::SubstrateConfig>,
    start_height: common::types::Height,
}

impl WorkerAttestationValidation {
    pub(crate) fn new(config: Config) -> Self {
        Self {
            cc3: config.cc3,

            keypair: config.keypair,
            watch_submission: future::OptionFuture::default(),
            attempts: 0,

            receiver_validation: config.receiver_validation,
            receiver_attestation_latest: config.receiver_attestation_latest,

            api_calls: config.api_calls,
            api: config.api,
            start_height: config.start_height,
        }
    }
}

impl super::Worker for WorkerAttestationValidation {
    #[tracing::instrument(name = "validation", skip_all)]
    fn task(
        mut self,
        mut shutdown: std::pin::Pin<Box<impl std::future::Future<Output = ()>>>,
    ) -> impl std::future::Future<Output = common::types::Result<()>> {
        async move {
            use futures::StreamExt as _;

            loop {
                tokio::select! {
                    biased;

                    _ = &mut shutdown => {
                        break self.handle_event_shutdown().await;
                    }
                    event = &mut self.watch_submission => {
                        self.handle_event_submission(event).await?;
                    }
                    event = self.receiver_validation.next() => {
                        self.handle_event_quorum(event).await?;
                    },
                }
            }
        }
    }
}

impl WorkerAttestationValidation {
    async fn handle_event_quorum(
        &mut self,
        quorum: Option<(
            pool::Quorum,
            pool::AttestationPermit,
            Option<cc_client::H256>,
        )>,
    ) -> Result<(), Error> {
        // ---------------------------------* Handle pool closure *--------------------------------

        // WARNING: ERROR HANDLING
        //
        // pool can be closed from another thread during shutdown, this is not a failure case!
        let Some((quorum, permit, digest_local)) = quorum else {
            return Ok(());
        };

        let digest = quorum.digest();
        let height = quorum.header_number();
        let chain_key = quorum.chain_key();

        tracing::info!(
            %digest,
            height,
            "🗳️ An attestation has reached quorum"
        );

        match self
            .quorum_aggregate(quorum, digest_local, digest, height, chain_key)
            .await
        {
            // CASE 1] VALID ATTESTATION - NOT WAITING ON SUBMISSION
            //
            // If the attestor notices a new quorum and it is not waiting on the runtime to
            // validate previous attestations, it will eagerly submit any new valid attestation.
            Some(Ok(attestation)) if self.watch_submission.is_none() => {
                // ---------------------------------* Pool update *--------------------------------

                self.receiver_validation.mark_valid(permit);

                // ---------------------------* Attestation submission *---------------------------

                tracing::info!(
                    %digest,
                    height = attestation.attestation.header_number,
                    "🛫 Submitting attestation"
                );

                self.submit_attestation(attestation.into(), height)
                    .await
                    .transpose()?;
            }
            // CASE 2] VALID ATTESTATION - WAITING ON SUBMISSION
            //
            // If the attestor notices a new quorum but is waiting on the runtime to validate
            // previous attestations, it will optimistically validate new sequential attestations
            // for them to be submitted later.
            Some(Ok(attestation)) => {
                // ---------------------------------* Pool update *--------------------------------

                tracing::info!(
                    %digest,
                    height = attestation.attestation.header_number,
                    "🗃️ Storing attestation for later submission"
                );

                self.receiver_validation.mark_for_later(permit, attestation);
            }
            // CASE 3] INVALID ATTESTATION
            //
            // Remove the attestation from the pool, it will eventually be re-generated
            Some(Err(Error::InvalidAttestation(_))) => {
                self.receiver_validation.mark_invalid(permit);
            }
            // CASE 4] EXTERNAL ERROR
            //
            // Cleanup and close the validation worker thread.
            Some(Err(err)) => {
                // WARNING: ERROR HANDLING
                //
                // Even if this is an irrecoverable error, we still need to restore the attestation
                // pool to a valid state as it can still be referenced to from other worker threads.
                self.receiver_validation.mark_invalid(permit);
                return Err(err);
            }
            // CASE 5] EXTERNAL INTERRUPT
            //
            // User initiated shutdown via SIGINT during a blocking retry operation.
            None => {}
        }

        Ok(())
    }

    async fn handle_event_submission(
        &mut self,
        submission: (AttestationSubmission, common::types::Height),
    ) -> Result<(), Error> {
        let (submission, height) = submission;

        match submission {
            // CASE 1] SUBMITTED ATTESTATION
            AttestationSubmission::Elligible(res) => {
                // -----------------------* Attestation runtime validation *---------------------------

                match res {
                    // CASE 1.A] LOST THE ATTESTATION SUBMISSION RACE
                    Err(subxt::Error::Runtime(subxt::error::DispatchError::Module(err))) => {
                        match err
                            .as_root_error::<cc_client::cc3::Error>()
                            .map_err(Error::SubxtError)?
                        {
                            cc_client::cc3::Error::Attestation(
                                cc_client::cc3::attestation::Error::AttestationExists,
                            ) => {
                                // NOTE: Attestation racing
                                //
                                // Since multiple attestors race to submit the same attestation at once and
                                // only one attestor can be selected to win the race, other attestors will
                                // receive a runtime error on submission indicating a duplicate attestation.
                                // This is not a failure case.
                                tracing::info!(height, "✅ Attestation already submitted");
                            }
                            err => {
                                tracing::error!(height, ?err, "⛔ Invalid attestation");
                                // WARNING: PANIC
                                //
                                // Any early return must reset the `watch_submission` future to
                                // avoid double polling!
                                self.watch_submission = future::OptionFuture::default();
                            }
                        }
                    }
                    // CASE 1.B] WON THE ATTESTATION SUBMISSION RACE
                    res => {
                        match res
                            .map_err(Error::SubxtError)?
                            .all_events_in_block()
                            .find_last::<cc_client::cc3::attestation::events::BlockAttested>()
                        {
                            Ok(Some(_attestation)) => {
                                tracing::info!(height, "✅ Attestation submitted on-chain");
                            }
                            _ => {
                                // WARNING: PANIC
                                //
                                // Any early return must reset the `watch_submission` future to
                                // avoid double polling!
                                self.watch_submission = future::OptionFuture::default();
                                return Err(Error::InvalidAttestationEvent);
                            }
                        }
                    }
                }

                // ------------------------* Attestation finalization *----------------------------

                // NOTE: EDGE CASE
                //
                // It is possible (but unlikely) for the attestation finalization event to have
                // been received before submission finalizes (this is because multiple attestors
                // are racing for submission at the same time). To avoid stalling, we must FIRST
                // check the latest attestation and THEN wait for updates if we are not already
                // past finalization.
                while (*self.receiver_attestation_latest.borrow())
                    .is_none_or(|attestation_latest| attestation_latest < height)
                {
                    tokio::select! {
                        biased;

                        _ = tokio::signal::ctrl_c() => {
                            // WARNING: PANIC
                            //
                            // Any early return must reset the `watch_submission` future to
                            // avoid double polling!
                            self.watch_submission = future::OptionFuture::default();
                            return Ok(());
                        }
                        // WARNING: ERROR HANDLING
                        //
                        // From the tokio docs:
                        //
                        // > Returns a RecvError if the channel has been closed AND the current value is
                        // > seen.
                        //
                        // This only errors if the receiving end of this channel has been dropped, which
                        // can happen during shutdown. This is not a failure case!
                        Ok(()) = self.receiver_attestation_latest.changed() => {}
                    }
                }

                // NOTE: EDGE CASE
                //
                // It is possible, but unlikely, that the submission VRF threshold computation does
                // not select ANY attestor for submission. In this case, there is no point in
                // re-computing the VRF threshold at the same height, as it will yield the same
                // result. To avoid this, we keep track of the number of attempts to submit an
                // attestation and take that into account in the VRF computation.
                self.attempts = 0;
            }
            // CASE 2] NOT SELECTED FOR ATTESTATION SUBMISSION
            AttestationSubmission::NotElligible(attestation) => {
                // ------------------------* Attestation finalization *----------------------------

                let mut interval = tokio::time::interval(common::constants::ATTESTATION_TIMEOUT);
                interval.tick().await;

                // NOTE: EDGE CASE
                //
                // It is possible (but unlikely) for the attestation finalization event to have
                // been received before submission finalizes (this is because multiple attestors
                // are racing for submission at the same time). To avoid stalling, we must FIRST
                // check the latest attestation and THEN wait for updates if we are not already
                // past finalization.
                while (*self.receiver_attestation_latest.borrow())
                    .is_none_or(|attestation_latest| attestation_latest < height)
                {
                    tokio::select! {
                        biased;

                        _ = tokio::signal::ctrl_c() => {
                            // WARNING: PANIC
                            //
                            // Any early return must reset the `watch_submission` future to
                            // avoid double polling!
                            self.watch_submission = future::OptionFuture::default();
                            return Ok(());
                        }
                        _ = interval.tick() => {
                            tracing::warn!(
                                threshold = height,
                                "🏃 Attestation finalization timed out, assuming no leader was elected"
                            );

                            // NOTE: EDGE CASE
                            //
                            // It is possible, but unlikely, that the submission VRF threshold
                            // computation does not select ANY attestor for submission. In this
                            // case, there is no point in re-computing the VRF threshold at the same
                            // height, as it will yield the same result. To avoid this, we keep
                            // track of the number of attempts to submit an attestation and take
                            // that into account in the VRF computation.
                            self.attempts += 1;
                            self.submit_attestation(attestation, height).await.transpose()?;

                            return Ok(());
                        }
                        // WARNING: ERROR HANDLING
                        //
                        // From the tokio docs:
                        //
                        // > Returns a RecvError if the channel has been closed AND the current value is
                        // > seen.
                        //
                        // This only errors if the receiving end of this channel has been dropped, which
                        // can happen during shutdown. This is not a failure case!
                        Ok(()) = self.receiver_attestation_latest.changed() => {}
                    }
                }

                // Extra check needed because of potential timeout
                if (*self.receiver_attestation_latest.borrow())
                    .is_some_and(|attestation_latest| attestation_latest >= height)
                {
                    tracing::info!(height, "✅ Attestation submitted externally");

                    // NOTE: EDGE CASE
                    //
                    // It is possible, but unlikely, that the submission VRF threshold computation
                    // does not select ANY attestor for submission. In this case, there is no point
                    // in re-computing the VRF threshold at the same height, as it will yield the
                    // same result. To avoid this, we keep track of the number of attempts to submit
                    // an attestation and take that into account in the VRF computation.
                    self.attempts = 0;
                }
            }
            AttestationSubmission::Finalized => {
                tracing::info!(height, "✅ Attestation submitted externally");
            }
        }

        // -----------------------------* Attestation pre-validation *-----------------------------

        if let Some((height, digest, attestation)) = self.receiver_validation.take_next_validated()
        {
            // CASE 1] AN ATTESTATION IS READY
            //
            // Submit that attestation and wait for it to be validated by the runtime as part of the
            // typical `WorkerAttestationValidation::task` event loop.

            tracing::info!(
                %digest,
                height,
                "🛫 Submitting pre-validated attestation"
            );

            self.submit_attestation(attestation, height)
                .await
                .transpose()?;
        } else {
            // CASE 2] NO ATTESTATION
            //
            // Default to waiting for the next quorum which will be submitted immediately on
            // local validation.
            self.watch_submission = future::OptionFuture::default();
        }

        Ok(())
    }

    async fn handle_event_shutdown(&mut self) -> common::types::Result<()> {
        Ok(())
    }
}

impl WorkerAttestationValidation {
    async fn quorum_aggregate(
        &mut self,
        quorum: pool::Quorum,
        digest_local: Option<cc_client::H256>,
        digest: attestor_primitives::Digest,
        height: common::types::Height,
        chain_key: attestor_primitives::ChainKey,
    ) -> Option<Result<common::types::AttestationSigned, Error>> {
        use bls_signatures::Serialize as _;
        use rand::seq::SliceRandom as _;
        use rand::SeedableRng as _;

        const MAX_ATTEMPTS: usize = 10;
        const DELAY_BASE: u64 = 10;
        const DELAY_MAX: u64 = 90;

        let mut attempt = 0;
        let mut delay = DELAY_BASE;

        let runtime_api = match self.api.runtime_api().at_latest().await {
            Ok(runtime_api) => runtime_api,
            Err(err) => return Some(Err(Error::SubxtError(err))),
        };

        // -----------------------------------* Pre-validation *-----------------------------------

        // STEP 1] PRELIMINARY CHECKS
        //
        // This ensures we are not dealing with a duplicate vote or an invalid source chain.

        let is_chain_supported = loop {
            let call = self
                .api_calls
                .supported_chains_api()
                .is_chain_supported(chain_key);
            match runtime_api.call(call).await {
                Ok(is_chain_supported) => break is_chain_supported,
                Err(err) => {
                    attempt += 1;

                    tracing::debug!(
                        attempt,
                        MAX_ATTEMPTS,
                        "Failed to retrieve supported chain, retrying..."
                    );

                    if attempt >= MAX_ATTEMPTS {
                        return Some(Err(Error::SubxtError(err)));
                    }
                }
            };

            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(delay))=> {},
                _ = tokio::signal::ctrl_c() => return None
            }

            delay = (delay * 2).min(DELAY_MAX);
        };

        if !is_chain_supported {
            tracing::error!(
                %digest,
                height,
                chain_key,
                "⛔ Unsupported source chain"
            );
            return Some(Err(Error::InvalidAttestation(InvalidCause::Unsupported(
                chain_key,
            ))));
        }

        let is_duplicate = loop {
            let call = self.api_calls.attestor_api().contains_digest(
                chain_key,
                cc_client::H256(digest.0),
                height,
            );
            match runtime_api.call(call).await {
                Ok(is_duplicate) => break is_duplicate,
                Err(err) => {
                    attempt += 1;

                    tracing::debug!(
                        attempt,
                        MAX_ATTEMPTS,
                        "Failed to retrieve attestation digest, retrying..."
                    );

                    if attempt >= MAX_ATTEMPTS {
                        return Some(Err(Error::SubxtError(err)));
                    }
                }
            };

            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(delay))=> {},
                _ = tokio::signal::ctrl_c() => return None
            }

            delay = (delay * 2).min(DELAY_MAX);
        };

        if is_duplicate {
            tracing::debug!(
                %digest,
                height,
                "Attestation already exists"
            );
            return Some(Err(Error::InvalidAttestation(InvalidCause::Duplicate)));
        }

        // Uses ChaCha under the hood
        let mut rng = rand::rngs::StdRng::from_os_rng();

        // WARNING: OPTIMIZATION
        //
        // Ss an optimization, we assume that each attestation in the quorum attests to the same
        // vote (this guarantee is upheld by the attestation pool). In later stages of attestation
        // validation, we use this to pick only one attestation to validate (after attestor
        // eligibility has been checked). Still, it is probably not a good idea to have the
        // attestation we select for further validation be deterministic, so we make this
        // unpredictable by shuffling the votes (just in case something DOES go wrong and an
        // attacker manages to find a way to insert a malicious attestation in a valid quorum).
        let mut votes = quorum.votes();
        votes.shuffle(&mut rng);

        // ---------------------------------* Attestor validation *--------------------------------

        for attestation in votes.iter() {
            let attestor_id = attestation.attestor.clone();

            tracing::debug!(
                %digest,
                height,
                %attestor_id,
                "Checking attestor eligibility"
            );

            // STEP 2] VERIFY THE ATTESTATION BLS SIGNATURE
            //
            // This checks the BLS signature with the public key the attestor provided when it
            // registered on chain, which also enforces that the vote should come from a registered
            // attestor.

            tracing::debug!(
                %digest,
                height,
                %attestor_id,
                "Checking attestion bls signature"
            );

            let pubkey = loop {
                let attestor: &[u8; 32] = attestor_id.account_id().as_ref();
                let call = self
                    .api_calls
                    .attestor_api()
                    .attestor_bls_pubkey(attestation.chain_key(), (*attestor).into());
                match runtime_api.call(call).await {
                    Ok(pubkey) => {
                        break pubkey.map(|pubkey| bls_signatures::PublicKey::from_bytes(&pubkey))
                    }
                    Err(err) => {
                        attempt += 1;

                        tracing::debug!(
                            attempt,
                            MAX_ATTEMPTS,
                            "Failed to retrieve attestor bls pubkey, retrying..."
                        );

                        if attempt >= MAX_ATTEMPTS {
                            return Some(Err(Error::SubxtError(err)));
                        }
                    }
                }

                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(delay))=> {},
                    _ = tokio::signal::ctrl_c() => return None
                }

                delay = (delay * 2).min(DELAY_MAX);
            };

            match pubkey {
                Some(Ok(pubkey)) => {
                    let msg = attestation.attestation_data.serialize();
                    if pubkey.verify(attestation.signature_bls.0, &msg) {
                        tracing::debug!(
                            %digest,
                            height,
                            %attestor_id,
                            "Valid attestion bls signature"
                        )
                    } else {
                        tracing::error!(
                            %digest,
                            height,
                            %attestor_id,
                            "⛔ Invalid Attestor bls signature"
                        );
                        return Some(Err(Error::InvalidAttestation(InvalidCause::InvalidBls)));
                    }
                }
                Some(Err(..)) => {
                    tracing::error!(
                        %digest,
                        height,
                        %attestor_id,
                        "⛔ Attestor is registered with an invalid bls public key"
                    );
                    return Some(Err(Error::InvalidAttestation(InvalidCause::InvalidBls)));
                }
                None => {
                    tracing::error!(
                        %digest,
                        height,
                        %attestor_id,
                        "⛔ Attestor is not registered on-chain"
                    );
                    return Some(Err(Error::InvalidAttestation(InvalidCause::InvalidBls)));
                }
            }
        }

        tracing::debug!(
            %digest,
            height,
            "All attestors are eligible to vote"
        );

        // -------------------------------* Attestation validation *-------------------------------

        // STEP 3] VERIFY THE ATTESTATION CONTINUITY CHAIN
        //
        // This ensures that new votes follow the established continuity of the source chain as
        // previously attested.

        let attestation = votes
            .first()
            .expect("Invariant violated: quorum must always contain at least one vote");

        // Every attestation must have a continuity proof except for the first attestation in the
        // chain
        if attestation.continuity_proof.is_empty() && height != self.start_height {
            tracing::error!(
                %digest,
                height,
                "⛔ Empty continuity proof"
            );
            return Some(Err(Error::InvalidAttestation(
                InvalidCause::EmptyContinuityProof,
            )));
        }

        let digest_last_finalized = loop {
            let call = self.api_calls.attestor_api().last_digest(chain_key);
            match runtime_api.call(call).await {
                Ok(digest_last_finalized) => break digest_last_finalized,
                Err(err) => {
                    attempt += 1;

                    tracing::debug!(
                        attempt,
                        MAX_ATTEMPTS,
                        "Failed to retrieve last finalized digest, retrying..."
                    );

                    if attempt >= MAX_ATTEMPTS {
                        return Some(Err(Error::SubxtError(err)));
                    }
                }
            };

            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(delay))=> {},
                _ = tokio::signal::ctrl_c() => return None
            }

            delay = (delay * 2).min(DELAY_MAX);
        };

        let digest_last_finalized = digest_last_finalized.unwrap_or_else(|| {
            tracing::debug!(
                %digest,
                height,
                "No last digest or checkpoint, assuming genesis"
            );
            cc_client::H256::zero()
        });

        // -------------------------------------* Prev digest *------------------------------------

        tracing::debug!(
            %digest,
            height,
            "Checking attestion prev digest"
        );

        match attestation.prev_digest() {
            // NOTE: we don't need to check against `self.digest_local` here since it can only ever
            // be ahead of `digest_last_finalized`.
            Some(digest_prev) if digest_prev.is_zero() && !digest_last_finalized.is_zero() => {
                tracing::error!(
                    %digest,
                    digest_prev = ?Some(digest_prev),
                    height,
                    "⛔ Empty prev digest despite already having finalized attestations on-chain"
                );
                return Some(Err(Error::InvalidAttestation(
                    InvalidCause::EmptyPrevDigest,
                )));
            }
            None if !digest_last_finalized.is_zero() => {
                tracing::error!(
                    %digest,
                    height,
                    "⛔ No prev digest despite already having finalized attestations on-chain"
                );
                return Some(Err(Error::InvalidAttestation(
                    InvalidCause::EmptyPrevDigest,
                )));
            }
            _ => {
                tracing::debug!(
                    %digest,
                    height,
                    "Valid attestation prev digest"
                )
            }
        }

        tracing::debug!(
            %digest,
            height,
            "Checking attestion continuity proof"
        );

        // -------------------------------------* Head digest *------------------------------------

        // The head digest of an attestation's continuity chain must match its prev digest
        if let Some(head) = attestation.continuity_proof.head() {
            let digest_head = head.digest;
            let digest_prev = attestation.prev_digest().unwrap_or_default();

            if digest_head != digest_prev {
                tracing::error!(
                    %digest,
                    digest_prev = ?Some(digest_prev),
                    height,
                    actual = %digest_head,
                    expected = %digest_prev,
                    "⛔ Invalid attestation continuity chain head digest"
                );
                return Some(Err(Error::InvalidAttestation(
                    InvalidCause::InvalidContinuityHeadDigest {
                        actual: digest_head,
                        expected: digest_prev,
                    },
                )));
            } else {
                tracing::debug!(
                    %digest,
                    digest_prev = ?Some(digest_prev),
                    height,
                    "Valid attestation prev head digest"
                )
            }
        }

        // -------------------------------------* Tail digest *------------------------------------

        // The tail prev digest of an attestation's continuity chain must match the digest of the
        // last finalized attestation.
        //
        // In previous versions of the attestor software, it was possible for attestation to lag
        // behind block production, which would lead to the prev digest not matching the last
        // finalized digest.
        //
        // Importantly, strict ordering was not being enforced on attestations, such that the range
        // of source chain blocks being attested to between attestations could overlap. This lead
        // to a situations where attestations which attested to past source chain blocks could be
        // received for validation AFTER a future attestation had been finalized, which would have
        // led to the tail prev digest and latest finalized digest not matching anymore.
        //
        // With the new p2p attestation aggregation and attestation pool implementation,
        // attestations follow a strict ordering in their production. This has the advantage of
        // cutting down on duplicate work (since attestations at different heights no longer attest
        // to overlapping block ranges) but it also makes it so that each attestation chain follows
        // a predictable prev digest. This prev digest is either the latest finalized attestation
        // or the latest local attestation, whichever is highest.
        //
        // Since we enforce strict ordering in attestation production and validation, AND we no
        // longer generate attestations with overlapping block ranges, this means that if an
        // attestation's tail prev digest does not match the latest finalized digest, then this
        // attestation is either:
        //
        // - Invalid.
        // - Already committed on chain.
        //
        // In both cases this can only happen if other attestors have already reached quorum on an
        // attestation at the same height and submitted it on chain. In practice, this will happen
        // often if we race multiple attestors to submission. However, unlike previously where an
        // attestation might contain new overlapping data, no new data can be committed to this way
        // and we can drop the attestation quorum.
        if let Some(tail) = attestation.continuity_proof.tail() {
            let digest_prev_tail = cc_client::H256(tail.prev_digest.0);

            if digest_prev_tail != digest_last_finalized
                && digest_local.is_none_or(|digest_local| digest_prev_tail != digest_local)
            {
                tracing::error!(
                    %digest,
                    %digest_prev_tail,
                    %digest_last_finalized,
                    ?digest_local,
                    height,
                    "⛔ Invalid attestation continuity chain tail digest"
                );
                return Some(Err(Error::InvalidAttestation(
                    InvalidCause::InvalidContinuityTailDigest {
                        actual: digest_prev_tail,
                        expected: digest_last_finalized,
                    },
                )));
            } else {
                tracing::debug!(
                    %digest,
                    %digest_prev_tail,
                    height,
                    "Valid attestation tail digest"
                )
            }
        }

        // ----------------------------------* Continuity proof *----------------------------------

        // Checks that each block in the continuity proof follows a matching chain of digests,
        // starting from the latest finalized digest
        let mut digest_prev_continuity = digest_last_finalized;
        for block in attestation.continuity_proof.iter() {
            let digest_prev_block = cc_client::H256(block.prev_digest.0);

            if digest_prev_block != digest_prev_continuity
                && digest_local.is_none_or(|digest_local| digest_prev_block != digest_local)
            {
                tracing::error!(
                    %digest,
                    height,
                    %digest_prev_block,
                    %digest_prev_continuity,
                    ?digest_local,
                    block_height = block.block_number,
                    "⛔ Invalid attestation continuity chain"
                );
                return Some(Err(Error::InvalidAttestation(
                    InvalidCause::InvalidContinuityProof {
                        block: block.clone(),
                        expected: digest_prev_continuity,
                    },
                )));
            }

            digest_prev_continuity = cc_client::H256(block.digest.0);
        }

        tracing::debug!(
            %digest,
            height,
            "Valid attestation continuity proof"
        );

        // ------------------------------* BLS signature aggregation *-----------------------------

        let sigs = votes
            .iter()
            .map(|att| att.signature_bls.0)
            .collect::<Vec<_>>();
        let bls_aggregate = match bls_signatures::aggregate(&sigs) {
            Ok(bls_aggregate) => match bls_aggregate.as_bytes().try_into() {
                Ok(bls_aggregate) => bls_aggregate,
                Err(err) => return Some(Err(Error::InvalidBls(err))),
            },
            Err(err) => return Some(Err(Error::BlsError(err))),
        };

        tracing::debug!(
            %digest,
            height,
            sigs = sigs.len(),
            bls = alloy::hex::encode_upper(bls_aggregate),
            "Aggregated all attestation BLS signatures"
        );

        let attestors = votes.iter().map(|att| att.attestor.clone()).collect();
        let attestation = votes
            .pop()
            .expect("Invariant violated: quorum must always contain at least one vote");

        Some(Ok(attestor_primitives::SignedAttestation {
            attestation: attestation.attestation_data,
            signature: bls_aggregate,
            attestors,
            continuity_proof: attestation.continuity_proof,
        }))
    }
}

// ----------------------------------------- [ HELPERS ] --------------------------------------- //

enum AttestationSubmission {
    Elligible(Result<subxt::blocks::ExtrinsicEvents<subxt::SubstrateConfig>, subxt::Error>),
    NotElligible(
        cc_client::cc3::runtime_types::attestor_primitives::SignedAttestation<
            cc_client::H256,
            cc_client::AccountId32,
        >,
    ),
    Finalized,
}

impl WorkerAttestationValidation {
    async fn submit_attestation(
        &mut self,
        attestation: cc_client::cc3::runtime_types::attestor_primitives::SignedAttestation<
            cc_client::H256,
            cc_client::AccountId32,
        >,
        height: common::types::Height,
    ) -> Option<Result<(), Error>> {
        use futures::FutureExt as _;

        match self
            .cc3
            .sign_vrf_submission(height + self.attempts as common::types::Height)
            .await
        {
            // TODO: have the runtime validate the submission vrf
            //
            // Note that this will require being able to retrieve the randomness of past epochs so
            // the runtime can use the same epoch in validating the vrf as used during generation.
            Ok(Some(vrf)) => {
                // -------------------------* Deterministic Rank Backoff *-------------------------

                // STEP 1]
                //
                // We stagger attestation submissions based on the election vrf to avoid multiple
                // attestors racing the runtime for submission at the same time. We do this in an
                // effort to save block space.

                let mut rank_input = Vec::with_capacity(vrf.output.len() + 8);
                rank_input.extend_from_slice(&vrf.output);
                rank_input.extend_from_slice(&height.to_be_bytes());
                let rank_hash = sp_io::hashing::keccak_256(&rank_input);

                // Given a set S of 0..n-1 distinct elements, we pick at random 3 elements in S to
                // form an ordered tuple. This tuple represents the ranks of each attestor during
                // submission. We choose 3 as the size of the tuple as that is the target number of
                // attestors for submission as per the round vrf. We call this tupple R.
                //
                // The probability of the minimum element in R appearing more than once is defined
                // as:
                //
                //                              P(n) = n(3n - 1) / 2
                //
                // Conversely, the probability of the minimum element in R appearing EXACTLY once
                // is:
                //
                //                        1 - P(n) = (2n - 1)(n - 1) / 2n^2
                //
                // This represents the probability of ONLY 1 attestor racing for submission at
                // once, while other attestors can act as backup. Solving for 1 - P(n) > 0.8 we
                // obtain 8, with diminishing returns beyond that point (see below).
                const RANKS: u64 = 8;
                let bytes = [
                    rank_hash[0],
                    rank_hash[1],
                    rank_hash[2],
                    rank_hash[3],
                    rank_hash[4],
                    rank_hash[5],
                    rank_hash[6],
                    rank_hash[7],
                ];
                let rank = u64::from_be_bytes(bytes) % RANKS;

                tracing::debug!(rank, "attestation submission race mitigation");

                // Determined experimentally
                //
                //                m := average time to submission finalization (17s)
                //
                //                                 delay = rank * m
                //
                // This guarantees that on average the amount of time between submissions should
                // approximate the time to finalization.
                //
                // Note that while 1 - P(n) grows roughly O(1 - 1/n) of the rank, the average
                // finalization delay for any rank size grows roughly linearly. For a rank size of
                // n, the min submission latency remains 0, while the max submission latency is
                // defined as:
                //
                //                                 Δt = n(1 - P(n))
                //
                // Therefore, and assuming an uniform distribution between 0 and Δt (as should be
                // guaranteed by the use of the round vrf as underlying randomness), we have an
                // average submission latency of:
                //
                //                             μ = (2n - 1)(n - 1) / 4n
                //
                // For a rank size of 8, the average submission latency is of roughly 3.3x the
                // average time to finalization.
                let delay = std::time::Duration::from_secs(rank * 17);
                let deadline = tokio::time::Instant::now()
                    .checked_add(delay)
                    .unwrap_or(tokio::time::Instant::now());

                // Attestation should finalize before the deadline. If this is not the case then an
                // attestor is most likely down.
                while (*self.receiver_attestation_latest.borrow())
                    .is_none_or(|attestation_latest| attestation_latest < height)
                {
                    tokio::select! {
                        _ = tokio::time::sleep_until(deadline) => {
                            break;
                        }
                        _ = self.receiver_attestation_latest.changed() => {
                            if self.receiver_attestation_latest.borrow()
                                    .is_some_and(|attestation_latest| attestation_latest >= height)
                            {
                                self.watch_submission = Some(std::future::ready((
                                    AttestationSubmission::Finalized, height
                                )))
                                .into();

                                return Some(Ok(()));
                            }
                        }
                    }
                }

                // ---------------------------------* Submission *---------------------------------

                // STEP 2]
                //
                // If the attestation has not finalized in time, then we submit it anyway. This
                // happens on average either if the attestor is first in line for submission or if
                // another attestor went down.

                let call = cc_client::cc3::tx()
                    .attestation()
                    .commit_attestation(attestation);

                const MAX_ATTEMPTS: usize = 5;
                const DELAY_BASE: u64 = 10;
                const DELAY_MAX: u64 = 60;

                let mut attempt = 0;
                let mut delay = DELAY_BASE;

                let submit = loop {
                    match self
                        .api
                        .tx()
                        .sign_and_submit_then_watch_default(&call, &self.keypair)
                        .await
                    {
                        Ok(submit) => break submit,
                        Err(err) => {
                            attempt += 1;

                            tracing::debug!(
                                attempt,
                                MAX_ATTEMPTS,
                                height,
                                "Failed to submit attestation, retrying..."
                            );

                            if attempt >= MAX_ATTEMPTS {
                                return Some(Err(Error::SubxtError(err)));
                            }
                        }
                    }

                    tokio::select! {
                        _ = tokio::time::sleep(std::time::Duration::from_secs(delay))=> {},
                        _ = tokio::signal::ctrl_c() => return None
                    }

                    delay = (delay * 2).min(DELAY_MAX);
                };

                // --------------------------------* Finalization *--------------------------------

                // STEP 3]
                //
                // Once an attestation has been submitted, we wait for the runtime to validate it.
                // Note that currently the code does not handle well the edge case of submitting
                // invalid attestations to the runtime. This can happen either in the case of a bug
                // in the attestor code or of a super-majority of malicious attestors, however in
                // both cases we currently do not offer good recovery methods.

                let watch = submit
                    .wait_for_finalized_success()
                    .map(move |res| (AttestationSubmission::Elligible(res), height));

                self.watch_submission = Some(watch).into();
            }
            Ok(None) => {
                tracing::info!(
                    height,
                    "🚦 Attestor was not selected for attestation submission"
                );

                self.watch_submission = Some(std::future::ready((
                    AttestationSubmission::NotElligible(attestation),
                    height,
                )))
                .into();
            }
            Err(err) => return Some(Err(Error::CC3Error(err))),
        }

        Some(Ok(()))
    }
}
