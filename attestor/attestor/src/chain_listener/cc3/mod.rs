//! A [chain listenerd responsible for producing and submitting attestations to the cc3 execution
//! chain, as well as reacting to events in the [production worker].
//!
//! [chain listener]: crate::chain_listener
//! [production worker]: crate::worker::production

mod error;

use crate::prelude::*;
pub use error::*;

// -------------------------------------- [ Configuration ] ------------------------------------ //

#[derive(Debug, attestor_macro::Builder)]
/// Configuration options for the cc3 [chain listener].
///
/// [chain listener]: crate::chain_listener
pub struct Config {
    /// Chain RPC url
    pub cc3_url: url::Url,
    /// Private key of the attestor being used to submit attestations on-chain. Keep in mind that
    /// attestors have to be **registered** before they can start to produce attestations.
    pub cc3_key: bip39::Mnemonic,
    #[specify_later]
    /// Execution chain client responsible for establishing a connection with cc3.
    pub cc3_client: cc_client::Client,
    /// Source chain client responsible for establishing a connection with ethereum.
    pub eth_url: url::Url,
    #[specify_later]
    /// Unique key which identifies the chain being attested to. In the case of ethereum it is `2`.
    pub chain_key: attestor_primitives::ChainKey,
    #[specify_later]
    /// Starting height at which attestation are produced and source chain block fetching begins.
    /// This value is fetched from on-chain storage unless it is overridden in [attestation config].
    ///
    /// [attestation config]: crate::attestation
    pub start_height: common::types::Height,
}

// ------------------------------------- [ Chain Listener ] ------------------------------------ //

/// CC3 [chain listener], responsible for producing and submitting attestations to the execution
/// chain.
///
/// This listener is polled by the [production worker] to generate attestations as new source
/// chain blocks are made available.
///
/// [chain listener]: crate::chain_listener
/// [production worker]: crate::worker::production
pub(crate) struct CC3 {
    cc3: cc_client::Client,
    eth: eth::Client,
    api: subxt::OnlineClient<subxt::SubstrateConfig>,
    stream: common::types::SubxtBlockStream,
    bls_key: bls_signatures::PrivateKey,
    chain_key: attestor_primitives::ChainKey,
    start_height: common::types::Height,
}

impl CC3 {
    /// Creates a new [`CC3`] [chain listener].
    ///
    /// This method is also responsible for registering an attestors bls public key to on-chain
    /// storage if this is not already the case, which is needed for attestation verification by
    /// the runtime.
    ///
    /// Keep in mind that attestors have to be registered before they can submit their bls public
    /// key. If this is not the case, this operation will fail and the attestor will shutdown.
    ///
    /// [chain listener]: crate::chain_listener
    #[tracing::instrument(skip_all, level = "debug")]
    pub async fn new(config: Config) -> anyhow::Result<Self> {
        use anyhow::Context as _;
        use bls_signatures::Serialize as _;

        tracing::info!("🛜 Staring CC3 listener");
        tracing::info!(url = %config.cc3_url, "🛜  with");
        tracing::info!(chain_key = config.chain_key, "🛜  with");

        // --------------------------------* Bls key registration *--------------------------------

        let cc3 = config.cc3_client;

        tracing::info!("🛜 Making sure attestor bls key is registered...");

        let cc3_key = config.cc3_key.to_string();
        let bls_key = bls_signatures::PrivateKey::new(cc3_key.as_bytes());

        let is_bls_key_regsitered = cc3
            .check_attestor_key_is_registered(config.chain_key)
            .await
            .context("Failed to check attestor bls registration")?;

        if !is_bls_key_regsitered {
            tracing::info!("🛜  registering attestor bls pubkey...");

            let mut bls_public_key = [0; 48];
            let bytes = &bls_key.public_key().as_bytes();
            bls_public_key.copy_from_slice(bytes);

            let mut proof_of_possession = [0; 96];
            let bytes = &bls_key.sign(bls_public_key).as_bytes()[..96];
            proof_of_possession.copy_from_slice(bytes);

            cc3.start_attesting(config.chain_key, bls_public_key, proof_of_possession)
                .await
                .context("Failed to register attestor bls pubkey")?;
        }

        // ------------------------------------* Configuration *-----------------------------------

        let eth = eth::Client::new(config.eth_url.as_ref(), None)
            .await
            .context("Failed to initialize ETH client")?;
        let api = cc3
            .api()
            .await
            .context("Failed to initialize CC3 client api")?;
        let stream = api
            .blocks()
            .subscribe_finalized()
            .await
            .context("Failed to initialize CC3 finalized block subscription")?;

        Ok(Self {
            cc3,
            eth,
            api,
            stream,
            bls_key,
            chain_key: config.chain_key,
            start_height: config.start_height,
        })
    }

    /// Creates a vrf proof of an attestors eligibility to **submit** an attestation at a given
    /// height.
    ///
    /// Eligibility is based on:
    ///
    /// - The execution chain babe randomness from two epochs ago.
    /// - The current epoch - 2
    /// - The target source chain height
    ///
    /// Contrary to [`sign_vrf_production`], this is only meant for DOS purposes by limiting the
    /// number of attestors which can submit attestations to the runtime at once. Similarly to that
    /// method though, it is possible for no attestor to be elected for submission, in which case
    /// the next attestation will attest to twice the attestation interval number of blocks.
    ///
    /// [`sign_vrf_production`]: Self::sign_vrf_production
    pub async fn sign_vrf_submission(
        &self,
        height: common::types::Height,
    ) -> Result<Option<vrf::ProofOfInclusion>, Error> {
        // TODO: we can avoid this call once the attestor has been listening to CC3 long enough
        let (randomness, epoch_index) = self
            .cc3
            .fetch_babe_randomness_two_epoch_ego()
            .await
            .map_err(Error::Cc3Client)?;

        match self
            .cc3
            .sign_vrf_submission(self.chain_key, height, randomness, epoch_index)
            .await
        {
            Ok(vrf) => Ok(Some(vrf)),
            Err(cc_client::Error::FailedToCreateProofOfInclusion(_)) => Ok(None),
            Err(err) => Err(Error::Cc3Client(err)),
        }
    }

    /// Signs an attestation with the attestor's bls secret key so that the runtime can validate
    /// that any attestation reached finality amongst a valid set of attestors.
    pub async fn sign_attestation(
        &self,
        attestation_data: attestor_primitives::AttestationData<attestor_primitives::Digest>,
        continuity_proof: attestor_primitives::attestation_fragment::AttestationFragmentSerializable,
        epoch: u64,
    ) -> common::types::Attestation {
        let attestor = self.cc3.get_attestor_id();
        let message = attestation_data.serialize();
        let signature = sp_core::sr25519::Signature::from_raw(self.cc3.sign(&message).0);
        let signature_bls = attestor_primitives::bls::WrapEncode(self.bls_key.sign(message));

        common::types::Attestation {
            attestation_data,
            attestor,
            signature,
            signature_bls,
            continuity_proof,
            epoch,
        }
    }

    /// Computes the continuity proof for an attestation.
    ///
    /// The continuity proof of an attestation consists of a merkle chain of blocks provably linking
    /// the new attestation to the last finalized attestation in order to establish continuity.
    ///
    /// Since the actual root computation can be quite expensive, especially amongst a large number
    /// of blocks/large attestation interval, the underlying implementation will automatically
    /// parallelize it across a [`rayon`] thread pool. If the thread pool if greater than or equal
    /// to the attestation interval, it can be considered that the time complexity of computing the
    /// continuity proof is constant so that an attestor's ability to compute larger and larger
    /// continuity proofs grows roughly linearly with the number of treads it has available.
    ///
    /// In reality, this is counterbalanced by the fact that a larger attestation interval means
    /// an attestors will be waiting on the source chain for longer periods of time, during which
    /// it is acceptable to make even slow progress in continuity proof computation.
    ///
    /// Still, optimizing continuity proof computation this way unlocks interesting options around
    /// the batching of future attestations in advance of the runtime, which can be seen in action
    /// as part of the [validation worker].
    ///
    /// [validation worker]: crate::worker::validation
    #[tracing::instrument(skip(self), level = "debug")]
    pub async fn create_continuity_proof(
        &mut self,
        height: common::types::Height,
        latest_attestation_eth: Option<(attestor_primitives::Digest, common::types::Height)>,
        latest_attestation_cc3: Option<(attestor_primitives::Digest, common::types::Height)>,
    ) -> Option<
        Result<attestor_primitives::attestation_fragment::AttestationFragmentSerializable, Error>,
    > {
        use futures::FutureExt as _;
        use rayon::iter::IndexedParallelIterator as _;
        use rayon::iter::IntoParallelIterator as _;
        use rayon::iter::ParallelIterator as _;

        // ------------------------------------* Range checks *------------------------------------

        if height == self.start_height {
            tracing::debug!("Creating default continuity proof for header number 0");
            return Some(Ok(Default::default()));
        }

        let (from_digest, from_block) = match (latest_attestation_cc3, latest_attestation_eth) {
            (Some((digest, height)), None) | (None, Some((digest, height))) => {
                (digest, height.saturating_add(1))
            }
            (Some((digest_cc3, height_cc3)), Some((digest_eth, height_eth))) => {
                if height_eth > height_cc3 {
                    (digest_eth, height_eth.saturating_add(1))
                } else {
                    (digest_cc3, height_cc3.saturating_add(1))
                }
            }
            (None, None) => (attestor_primitives::Digest::zero(), self.start_height),
        };

        let until_block = if height == from_block {
            // Meaning it's the first attestation in the chain
            height
        } else {
            // We don't need to include the attestation itself inside the continuity proof
            height.saturating_sub(1)
        };

        tracing::debug!(
            from_block,
            until_block,
            %from_digest,
            "Generating continuity proof"
        );

        // WARNING: RACE CONDITION
        //
        // There are plenty of nice network conditions which can cause the latest source chain
        // block height we are aware of to fall behind chain finalization. These mainly revolve
        // around a future attestation being finalized after a reset at an epoch boundary, or
        // other attestation regeneration scenarios. Rather than treat this as an error and try and
        // handle every possible edge case, we simply let this fly and move on to the next source
        // chain attestation.
        if from_block > height {
            return None;
        }

        // ----------------------------------* Continuity proof *----------------------------------

        let fragment_size = (until_block - from_block + 1) as usize;

        // STEP 1] SOURCE BLOCKS
        //
        // Tries to fetch each source chain block CONCURRENTLY, but NOT IN PARALLEL
        let encoding = ccnext_abi_encoding::common::EncodingVersion::V1;
        let blocks = (from_block..=until_block).map(|height| {
            self.eth
                .get_block(height, encoding)
                .map(|opt| opt.transpose().map_err(Error::EthClient))
        });
        let blocks = match futures::future::try_join_all(blocks).await {
            Ok(blocks) => blocks,
            Err(err) => return Some(Err(err)),
        };

        // STEP 2] MERKLE ROOT
        //
        // Computes each block's MMR root CONCURRENTLY and IN PARALLEL
        let mut blocks_with_roots = Vec::with_capacity(fragment_size);
        blocks
            .into_par_iter()
            .map(|opt| opt.map(|block| (eth::simple_merkle_tree(&block).root(), block)))
            .collect_into_vec(&mut blocks_with_roots);

        // STEP 3] FRAGMENT AGGREGATION
        //
        // Aggregate each block into a single fragment
        let mut fragment_blocks =
            Vec::<attestor_primitives::block::BlockSerializable>::with_capacity(fragment_size);
        for opt in blocks_with_roots {
            let Some((root, block)) = opt else {
                // NOTE: INTERRUPT
                //
                // User-initiated shutdown. See the implementation of `self.eth.get_block` to
                // understand why this is here.
                return None;
            };

            let block = if let Some(head) = fragment_blocks.last() {
                attestor_primitives::block::Block::new_from_prev_digest(
                    block.number(),
                    root,
                    head.digest,
                )
            } else {
                attestor_primitives::block::Block::new_from_prev_digest(
                    block.number(),
                    root,
                    from_digest,
                )
            };

            fragment_blocks.push(attestor_primitives::block::BlockSerializable::from(block));
        }

        let fragment = attestor_primitives::attestation_fragment::AttestationFragmentSerializable {
            blocks: fragment_blocks,
        };

        Some(Ok(fragment))
    }

    // TODO: remove this
    pub fn get_chain_key(&self) -> attestor_primitives::ChainKey {
        self.chain_key
    }

    // TODO: remove this
    pub async fn get_current_epoch(&self) -> Result<u64, Error> {
        self.cc3.get_current_epoch().await.map_err(Error::Cc3Client)
    }

    pub fn api(&self) -> subxt::OnlineClient<subxt::SubstrateConfig> {
        self.api.clone()
    }

    /// Determines if an attestor is currently able to attest to the source chain.
    ///
    /// Attestor eligibility is preconditioned on an attestor having been registered and activated
    /// in the runtime. Once that is the case, an attestor will have to wait until the next epoch
    /// rotation for a new set of authorities to be elected: if it is part of this new set, it will
    /// be able to start attesting.
    pub async fn can_attest(&self) -> Result<bool, Error> {
        Ok(matches!(
            self.cc3
                .get_attestor_status(self.chain_key)
                .await
                .map_err(Error::Cc3Client)?,
            Some(attestor_primitives::AttestorStatus::Active)
        ))
    }

    /// Returns a continuous stream of execution chain events.
    ///
    /// These events are handled by the [production worker] in response to changes in execution
    /// chain state.
    ///
    /// [production worker]: crate::worker::production
    pub async fn next(&mut self) -> Option<Result<CC3Events, Error>> {
        match self.next_block().await {
            Some(Ok(block)) => Some(Ok(CC3Events {
                block,
                chain_key: self.chain_key,
            })),
            Some(Err(err)) => Some(Err(err)),
            None => None,
        }
    }
}

// ----------------------------------------- [ Events ] ---------------------------------------- //

pub(crate) struct CC3Events {
    block: common::types::SubxtBlock,
    chain_key: attestor_primitives::ChainKey,
}

impl CC3Events {
    pub async fn events(
        &mut self,
    ) -> Result<impl Iterator<Item = Result<cc_client::attestation::CcEvent, Error>>, Error> {
        let events = self.block.events().await.map_err(Error::SubxtError)?;
        let iter = cc_client::Client::extract_events(self.chain_key, events)
            .map(|event| event.map_err(|err| Error::SubxtError(err.into())));

        Ok(iter)
    }
}

// ----------------------------------------- [ HELPERS ] --------------------------------------- //

impl CC3 {
    async fn next_block(&mut self) -> Option<Result<common::types::SubxtBlock, Error>> {
        const MAX_ATTEMPTS: usize = 5;
        const DELAY_BASE: u64 = 10;
        const DELAY_MAX: u64 = 60;

        let mut attempt = 0;
        let mut delay = DELAY_BASE;

        loop {
            match self.stream.next().await {
                Some(Ok(block)) => break Some(Ok(block)),
                Some(Err(err)) => {
                    attempt += 1;

                    tracing::debug!(
                        attempt,
                        MAX_ATTEMPTS,
                        "Failed to retrieve cc3 block, retrying..."
                    );

                    if attempt >= MAX_ATTEMPTS {
                        break Some(Err(Error::SubxtError(err)));
                    }
                }
                None => match self.api.blocks().subscribe_finalized().await {
                    Ok(stream) => self.stream = stream,
                    Err(err) => {
                        attempt += 1;

                        tracing::debug!(
                            attempt,
                            MAX_ATTEMPTS,
                            "Failed to reconnect to cc3, retrying..."
                        );

                        if attempt >= MAX_ATTEMPTS {
                            break Some(Err(Error::SubxtError(err)));
                        }
                    }
                },
            }

            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(delay))=> {},
                _ = tokio::signal::ctrl_c() => break None
            }

            delay = (delay * 2).min(DELAY_MAX);
        }
    }
}
