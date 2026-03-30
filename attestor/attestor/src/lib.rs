pub mod attestation;
pub mod common;
pub mod stream;
pub mod worker;

mod error;

pub use error::Error;

// ----------------------------------------- [ Exports ] --------------------------------------- //

pub mod prelude {
    pub use crate::common;
    pub use user::prelude::*;
}

use crate::prelude::*;

// -------------------------------------- [ Configuration ] ------------------------------------ //

#[derive(Debug, builder::Builder)]
pub struct Config {
    name: String,
    chain_key: attestor_primitives::ChainKey,

    stream: stream::Config,
    attestation: attestation::Config,

    p2p: worker::p2p::ConfigIncomplete,
    api: worker::api::ConfigIncomplete,
    pool: worker::validation::pool::ConfigIncomplete,
}

// ---------------------------------------- [ Main loop ] -------------------------------------- //

#[derive(Debug)]
pub struct Attestor {
    config: Config,
}

impl Attestor {
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    #[tracing::instrument(
        name = "attestor", 
        skip_all,
        fields(attestor_name = self.config.name, chain_key = self.config.chain_key)
    )]
    pub async fn run(self) -> Result<(), Error> {
        use bls_signatures::Serialize as _;
        use std::str::FromStr as _;

        // --------------------------------------* Identity *--------------------------------------

        let secret_str = self.config.stream.secret.to_secret_uri_string();
        let secret_uri = subxt_signer::SecretUri::from_str(secret_str.as_str())
            .expect("Failed to create secret uri");
        let keypair_cc3 = subxt_signer::sr25519::Keypair::from_uri(&secret_uri)
            .expect("Failed to create secret keypair");
        let account_id = cc_client::AccountId32(keypair_cc3.public_key().0);

        let mut seed = self.config.stream.secret.to_seed_bytes_32();
        let keypair_p2p = libp2p::identity::Keypair::ed25519_from_bytes(&mut *seed)
            .expect("Failed to create ed25519 keypair");
        let peer_id = libp2p::PeerId::from_public_key(&keypair_p2p.public());

        tracing::info!(name = self.config.name, %account_id, chain_key = self.config.chain_key, "🙋‍♀️ Starting attestor");

        let monitor = worker::CancellationMonitor::new();

        // -----------------------------------* Chain endpoints *----------------------------------

        match Self::wait_for_endpoints(&self.config.stream.url_eth, &self.config.stream.url_cc3)
            .await
        {
            Ok(()) => {}
            Err(Interrupt::Stop) => {
                tracing::info!("🔌 Received shutdown signal");
                return Ok(());
            }
            Err(Interrupt::Cont(err)) => {
                tracing::error!(%err, "⛔ Failed to wait for chain endpoints");
                return Err(err);
            }
        }

        let client_cc3 =
            cc_client::Client::new(self.config.stream.url_cc3.as_ref(), secret_str.as_str())
                .await
                .map_err(Error::InitError)?;

        let client_eth = eth::Client::new(self.config.stream.url_eth.as_ref(), None)
            .await
            .map_err(Error::InitError)?;

        // -----------------------------------* Verify attestor balances *-----------------------------

        tracing::info!("🔍 Verifying attestor balances");

        let free_balance = client_cc3
            .get_free_balance(&account_id)
            .await
            .map_err(Error::RpcError)?;
        if free_balance < common::constants::MIN_BALANCE {
            tracing::error!(name = self.config.name, %account_id, chain_key = self.config.chain_key, balance = %free_balance, "⛔ Attestor has insufficient balance");
            return Err(Error::InitError(anyhow::anyhow!(
                "Attestor {} ({}) has insufficient balance: {} < {}",
                self.config.name,
                account_id,
                free_balance,
                common::constants::MIN_BALANCE
            )));
        } else {
            tracing::info!(name = self.config.name, %account_id, chain_key = self.config.chain_key, balance = %free_balance, "🔍 Attestor has sufficient balance");
        }

        // -----------------------------------------* CC3 *----------------------------------------

        let config = stream::cc3::ConfigBuilder::new()
            .with_cc3(client_cc3.clone())
            .with_chain_key(self.config.chain_key)
            .build();
        let stream_cc3_production = stream::cc3::StreamCC3::new(config)
            .await
            .map_err(Error::InitError)?;

        let config = stream::cc3::ConfigBuilder::new()
            .with_cc3(client_cc3.clone())
            .with_chain_key(self.config.chain_key)
            .build();
        let stream_cc3_validation = stream::cc3::StreamCC3::new(config)
            .await
            .map_err(Error::InitError)?;

        let config = stream::cc3::ConfigBuilder::new()
            .with_cc3(client_cc3.clone())
            .with_chain_key(self.config.chain_key)
            .build();
        let mut stream_cc3_genesis = stream::cc3::StreamCC3::new(config)
            .await
            .map_err(Error::InitError)?;

        // ------------------------------------* Registration *------------------------------------

        let bls_seed = self.config.stream.secret.to_bls_seed_bytes();
        let bls_key = bls_signatures::PrivateKey::new(bls_seed.as_slice());

        let bls_public_key_bytes = bls_key.public_key().as_bytes();
        let bls_pubkey_hex = format!("0x{}", hex::encode(&bls_public_key_bytes));
        tracing::info!(
            bls_public_key_hex = %bls_pubkey_hex,
            "🔑 BLS public key (set this in fork genesis Attestors if needed)"
        );

        // ------------------------------------* Start Attesting *------------------------------------

        match Self::register_bls(
            self.config.chain_key,
            &client_cc3,
            &account_id,
            &bls_key,
            &bls_public_key_bytes,
        )
        .await
        {
            Ok(()) => {}
            Err(Interrupt::Stop) => {
                tracing::info!("🔌 Received shutdown signal");
                return Ok(());
            }
            Err(Interrupt::Cont(err)) => {
                tracing::error!(%err, "⛔ Failed to register attestor BLS public key");
                return Err(err);
            }
        }

        // -----------------------------------* Eligibility *----------------------------------- //

        tracing::info!(
            attestor_id = %account_id,
            "⏲️ Waiting for attestor to be made eligible"
        );

        let attestors = match Self::wait_for_eligible(
            self.config.chain_key,
            &client_cc3,
            &account_id,
            &mut stream_cc3_genesis,
        )
        .await
        {
            Ok(attestors) => attestors,
            Err(Interrupt::Stop) => {
                tracing::info!("🔌 Received shutdown signal");
                return Ok(());
            }
            Err(Interrupt::Cont(err)) => {
                tracing::error!(%err, "⛔ Failed to wait on attestor eligibility");
                return Err(err);
            }
        };

        // ---------------------------------* Chain configuration *--------------------------------

        let interval_attestation = match self.config.attestation.attestation_interval {
            Some(attestation_interval) => attestation_interval,
            None => client_cc3
                .chain_attestation_interval(self.config.chain_key)
                .await
                .map_err(Error::RpcError)?
                .map(std::num::NonZero::<common::types::Height>::new)
                .ok_or(Error::MissingAttestationInterval(self.config.chain_key))?
                .unwrap(),
        };

        let strategy_str = client_cc3
            .get_supported_chain(self.config.chain_key)
            .await
            .map_err(Error::RpcError)?
            .ok_or(Error::ChainKeyNotSupported(self.config.chain_key))?
            .maturity_strategy;
        let strategy_enum: supported_chains_primitives::MaturityStrategy = strategy_str
            .as_str()
            .try_into()
            .map_err(|e| Error::InvalidMaturityStrategy(self.config.chain_key, e))?;
        let maturity_delay = strategy_enum
            .maturity_delay()
            .ok_or(Error::NoMaturityDelayForStrategy(strategy_enum))?;

        let genesis = client_cc3
            .get_attestation_chain_genesis_block_number(self.config.chain_key)
            .await
            .map_err(Error::RpcError)?;

        let start_attestation = match client_cc3
            .fetch_last_digest(self.config.chain_key)
            .await
            .map_err(Error::RpcError)?
        {
            Some(last_digest) => match client_cc3
                .get_attestation_by_digest(self.config.chain_key, last_digest)
                .await
                .map_err(Error::RpcError)?
            {
                Some(last_attestation) => Some(common::types::AttestationInfo {
                    digest: last_attestation.digest(),
                    height: last_attestation.header_number(),
                }),
                None => {
                    unreachable!("Invalid last digest, something has gone very wrong!");
                }
            },
            None => client_cc3
                .get_last_checkpoint(self.config.chain_key)
                .await
                .map_err(Error::RpcError)?
                .map(|last_checkpoint| common::types::AttestationInfo {
                    digest: last_checkpoint.digest,
                    height: last_checkpoint.block_number,
                }),
        };

        let start_height = self
            .config
            .attestation
            .start_height
            .or(start_attestation.as_ref().map(|info| info.height + 1))
            .unwrap_or(genesis);

        let target = client_cc3
            .target_sample_size(self.config.chain_key)
            .await
            .map_err(|_| Error::MissingTargetSampleSize(self.config.chain_key))?;
        let quorum =
            std::num::NonZeroUsize::new(attestor_primitives::calculate_threshold(target) as usize)
                .expect("Failed to compute quorum threshold");

        tracing::info!(quorum, ?start_attestation, "🧑‍🤝‍🧑 Retrieved chain data");

        // -------------------------------------* Attestation *------------------------------------

        let config = stream::attestation::ConfigBuilder::new()
            .with_cc3(client_cc3.clone())
            .with_eth(client_eth)
            .with_bls_key(bls_key)
            .with_interval_attestation(interval_attestation)
            .with_chain_key(self.config.chain_key)
            .with_start_height(start_height)
            .with_start_attestation(start_attestation)
            .with_maturity_delay(maturity_delay)
            .build();
        let mut stream_attestation = stream::attestation::StreamAttestation::new(config)
            .await
            .map_err(Error::InitError)?;

        // ---------------------------------------* Metrics *--------------------------------------

        let config = worker::api::metrics::ConfigBuilder::new()
            .with_name(self.config.name)
            .with_address(account_id.clone())
            .with_peer_id(peer_id)
            .with_chain_key(self.config.chain_key)
            .with_start_height(start_height)
            .with_start_attestation(start_attestation)
            .with_genesis(genesis)
            .with_attestation_latest_eth(stream_attestation.block_highest())
            .with_attestation_interval(interval_attestation)
            .build();
        let metrics = std::sync::Arc::new(worker::api::metrics::Metrics::new(config));

        // -------------------------------------* Channels *------------------------------------ //

        // attestation production -> p2p sync
        let (sender_p2p, receiver_p2p) =
            tokio::sync::broadcast::channel(common::constants::CAPACITY_CHANNEL);

        // attestation production / p2p sync -> attestation validation
        let config = self
            .config
            .pool
            .with_attestors(attestors)
            .with_quorum(quorum)
            .with_start_height(start_height)
            .with_start_attestation(start_attestation)
            .with_attestation_interval(interval_attestation)
            .with_metrics(std::sync::Arc::clone(&metrics))
            .build();
        let (mut sender_validation, receiver_validation) =
            worker::validation::pool::attestation_pool(config);

        // ---------------------------------------* API *--------------------------------------- //

        tracing::info!("⏳ [1/4] Starting API worker");

        let config = self
            .config
            .api
            .with_metrics(std::sync::Arc::clone(&metrics))
            .build();
        let api = worker::api::WorkerApi::new(config);

        let mut handle_api = Some(monitor.spawn(api));

        // ------------------------------------* Validation *----------------------------------- //

        tracing::info!("⏳ [2/4] Starting attestation validation worker");

        let config = worker::validation::ConfigBuilder::new()
            .with_stream_cc3(stream_cc3_validation)
            .with_cc3(client_cc3.clone())
            .with_keypair(keypair_cc3)
            .with_validation_receiver(receiver_validation)
            .with_validation_sender(sender_validation.clone())
            .with_api_calls(cc_client::Client::runtime_api())
            .with_start_height(start_height)
            .with_genesis(genesis)
            .with_metrics(std::sync::Arc::clone(&metrics))
            .build();
        let attestation_validation = worker::validation::WorkerAttestationValidation::new(config);
        let mut handle_validation = Some(monitor.spawn(attestation_validation));

        // ---------------------------------------* P2P *--------------------------------------- //

        tracing::info!("⏳ [3/4] Starting P2P worker");

        let config = self
            .config
            .p2p
            .with_keypair(keypair_p2p)
            .with_receiver_p2p(receiver_p2p)
            .with_sender_validation(sender_validation.clone())
            .with_chain_key(self.config.chain_key)
            .with_metrics(std::sync::Arc::clone(&metrics))
            .build();
        let p2p = worker::p2p::WorkerP2P::new(config).map_err(Error::InitError)?;
        let mut handle_p2p = Some(monitor.spawn(p2p));

        // --------------------------------------* Genesis *------------------------------------ //

        let attestation_latest_cc3 = match start_attestation {
            Some(info) => info,
            None => {
                tracing::info!(genesis, "👶 Generating genesis attestation");

                match Self::wait_for_genesis(
                    genesis,
                    &account_id,
                    &mut stream_cc3_genesis,
                    &mut stream_attestation,
                    &mut sender_validation,
                    &sender_p2p,
                )
                .await
                {
                    Ok(info) => info,
                    Err(Interrupt::Stop) => {
                        tracing::info!("🔌 Received shutdown signal");
                        return Ok(());
                    }
                    Err(Interrupt::Cont(err)) => {
                        tracing::error!(%err, "⛔ Failed to submit genesis attestation");
                        return Err(err);
                    }
                }
            }
        };

        // ------------------------------------* Production *----------------------------------- //

        tracing::info!("⏳ [4/4] Starting attestation production worker");

        let config = worker::production::ConfigBuilder::new()
            .with_stream_attestation(stream_attestation)
            .with_stream_cc3(stream_cc3_production)
            .with_sender_p2p(sender_p2p)
            .with_sender_validation(sender_validation)
            .with_interval_attestation(interval_attestation)
            .with_attestation_latest_cc3(attestation_latest_cc3)
            .with_start_height(start_height)
            .with_account_id(account_id)
            .with_metrics(metrics)
            .build();
        let production = worker::production::WorkerAttestationProduction::new(config)
            .map_err(Error::InitError)?;
        let mut handle_production = Some(monitor.spawn(production));

        tracing::info!("✅ All services online!");

        // -----------------------------------* Thread waiting *-----------------------------------

        let mut shutdown = 0;
        let mut res = Ok(());

        // Worker error and shutdown signal
        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    tracing::info!("🔌 Received shutdown signal");
                    monitor.shutdown();
                    break;
                },
                _ = monitor.cancelled() => {
                    tracing::info!("🔌 Received cancellation signal");
                    break;
                }
                _ = monitor.failed() => {
                    tracing::error!("⛔ Worker thread error");
                    res = res.and(
                        Self::wait_for_worker(
                            &mut shutdown,
                            &mut handle_api,
                            &mut handle_production,
                            &mut handle_validation,
                            &mut handle_p2p,
                        ).await,
                    );
                }
            }
        }

        // Wait for remaining workers
        while shutdown < common::constants::WORKER_COUNT {
            res = res.and(
                Self::wait_for_worker(
                    &mut shutdown,
                    &mut handle_api,
                    &mut handle_production,
                    &mut handle_validation,
                    &mut handle_p2p,
                )
                .await,
            );
        }

        res
    }

    async fn wait_for_endpoints(
        url_eth: &url::Url,
        url_cc3: &url::Url,
    ) -> Result<(), Interrupt<Error>> {
        loop {
            tokio::select! {
                Ok(_) = tokio_tungstenite::connect_async(url_eth) => {
                    break;
                }
                _ = tokio::time::sleep(common::constants::RETRY_DELAY) => {}
                _ = tokio::signal::ctrl_c() => {
                    return Err(Interrupt::Stop);
                }
            }
            tracing::info!(
                url = %url_eth,
                "🛜 waiting for Eth WS connection to be made available..."
            );
        }

        loop {
            tokio::select! {
                Ok(_) = tokio_tungstenite::connect_async(url_cc3) => {
                    break;
                }
                _ = tokio::time::sleep(common::constants::RETRY_DELAY) => {}
                _ = tokio::signal::ctrl_c() => {
                    return Err(Interrupt::Stop);
                }
            }
            tracing::info!(
                url = %url_cc3,
                "🛜 waiting for CC3 WS connection to be made available..."
            );
        }

        Ok(())
    }

    async fn register_bls(
        chain_key: attestor_primitives::ChainKey,
        client_cc3: &cc_client::Client,
        account_id: &cc_client::AccountId32,
        bls_key: &bls_signatures::PrivateKey,
        bls_public_key_bytes: &[u8],
    ) -> Result<(), Interrupt<Error>> {
        use anyhow::Context as _;
        use bls_signatures::Serialize as _;

        let status = client_cc3
            .get_attestor_status(chain_key)
            .await
            .map_interrupt(Error::RpcError)?;

        if status == Some(attestor_primitives::AttestorStatus::Idle) {
            tracing::info!(
                attestor_id = %account_id,
                "📝 Submitting attest() extrinsic to transition from Idle to Waiting"
            );

            let bls_public_key = bls_public_key_bytes[..]
                .try_into()
                .context("BLS public key has unexpected length")
                .map_interrupt(Error::InitError)?;

            let proof_of_possession = bls_key.sign(bls_public_key).as_bytes()[..]
                .try_into()
                .context("BLS signature has unexpected length")
                .map_interrupt(Error::InitError)?;

            tokio::select! {
                res = client_cc3.start_attesting(
                    chain_key,
                    bls_public_key,
                    proof_of_possession,
                ) => {
                    res.map_interrupt(Error::RpcError)?;
                }
                _ = tokio::signal::ctrl_c() => {
                    return Err(Interrupt::Stop);
                }
            }

            tracing::info!(
                attestor_id = %account_id,
                "✅ Successfully submitted attest() - now Waiting for election"
            );
        } else {
            tracing::info!(
                attestor_id = %account_id,
                ?status,
                "ℹ️ Attestor status is already {:?}, skipping attest()", status
            );
        }

        Ok(())
    }

    async fn wait_for_eligible(
        chain_key: attestor_primitives::ChainKey,
        client_cc3: &cc_client::Client,
        account_id: &cc_client::AccountId32,
        stream_cc3: &mut stream::cc3::StreamCC3,
    ) -> Result<Vec<cc_client::AccountId32>, Interrupt<Error>> {
        use anyhow::Context as _;
        use futures::StreamExt as _;
        use futures::TryStreamExt as _;

        let mut attestors = client_cc3
            .get_attestor_active_set(chain_key)
            .await
            .map_interrupt(Error::RpcError)?;

        let cc3_block_time_ms = client_cc3
            .api()
            .constants()
            .at(&cc_client::cc3::constants().timestamp().minimum_period())
            .context("Failed to retrieve cc3 block time")
            .map_interrupt(Error::InitError)?
            * 2;

        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        if !attestors.contains(account_id) {
            attestors = 'outer: loop {
                tokio::select! {
                    Some(mut events) = stream_cc3.next() => {
                        while let Some(event) =  events.try_next().await.map_interrupt(Error::CC3Error)? {
                            if let cc_client::attestation::CcEvent::AttestorsElected(attestors) = event {
                                if attestors.contains(account_id) {
                                    break 'outer attestors;
                                }
                            }
                        }
                    }
                    _ = tokio::signal::ctrl_c() => {
                        return Err(Interrupt::Stop);
                    }
                    _ = interval.tick() => {
                        tracing::info!(
                            attestor_id = %account_id,
                            "⏲️  waiting on attestor..."
                        );
                    }
                }
            }
        }

        tracing::info!(attestor_id = %account_id, "☀️ Attestor is eligible for production");

        // Waiting for 2 blocks so other attestors have time to update the attestor set
        let step = cc3_block_time_ms * 2 / 10;

        for i in 1..=10 {
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_millis(step)) => {
                    tracing::info!("⏳ Startup delay {}/{}ms", step * i, cc3_block_time_ms * 2);
                }
                _ = tokio::signal::ctrl_c() => {
                    return Err(Interrupt::Stop);
                }
            }
        }

        Ok(attestors)
    }

    async fn wait_for_genesis(
        genesis: common::types::Height,
        account_id: &cc_client::AccountId32,
        stream_cc3: &mut stream::cc3::StreamCC3,
        stream_attestation: &mut stream::attestation::StreamAttestation,
        sender_validation: &mut worker::validation::pool::AttestationPoolSender,
        sender_p2p: &tokio::sync::broadcast::Sender<common::types::Attestation>,
    ) -> Result<common::types::AttestationInfo, Interrupt<Error>> {
        use anyhow::Context as _;
        use futures::StreamExt as _;
        use futures::TryStreamExt as _;

        let attestation_genesis = stream_attestation
            .generate_attestation_genesis()
            .await
            .map_interrupt(Error::AttestationError)?;

        let height = attestation_genesis.header_number();
        let digest = attestation_genesis.digest();
        // No previous digest means we will log `0x000...000` as the previous digest
        let digest_prev = attestation_genesis
            .prev_digest()
            .unwrap_or_else(sp_core::H256::zero);
        let attestor_id = attestation_genesis.attestor.account_id();

        assert_eq!(height, genesis, "Genesis attestation height mismatch");

        tracing::info!(
            ?digest,
            ?digest_prev,
            height,
            %attestor_id,
            "📡 Generated genesis attestation"
        );

        sender_p2p
            .send(attestation_genesis.clone())
            .context("Failed to send initial attestation over to p2p worker")
            .map_interrupt(Error::InitError)?;
        sender_validation
            .send(attestation_genesis)
            .transpose()
            .expect("Failed to send initial attestation over for validation");

        tracing::info!(genesis, "⏲️ Waiting for genesis attestation to finalize");

        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        let attestation_latest_cc3 = 'outer: loop {
            tokio::select! {
                Some(mut events) = stream_cc3.next() => {
                    while let Some(event) = events.try_next().await.map_interrupt(Error::CC3Error)? {
                        if let cc_client::attestation::CcEvent::BlockAttested(attestation_new) = event {
                            if attestation_new.header_number >= height {
                                break 'outer common::types::AttestationInfo {
                                    digest: attestation_new.digest,
                                    height: attestation_new.header_number,
                                };
                            }
                        }
                    }
                }
                _ = tokio::signal::ctrl_c() => {
                    return Err(Interrupt::Stop);
                }
                _ = interval.tick() => {
                    tracing::info!(
                        height,
                        attestor_id = %account_id,
                        "⏲️  waiting on submission..."
                    );
                }
            }
        };

        stream_attestation.note_attestation_finalization(attestation_latest_cc3);

        if let Err(err) = sender_validation.note_attestation_finalization(attestation_latest_cc3) {
            err.log_error(attestation_latest_cc3.digest);
        }

        Ok(attestation_latest_cc3)
    }

    #[allow(clippy::result_large_err)]
    async fn wait_for_worker(
        shutdown: &mut usize,
        handle_api: &mut Option<
            std::thread::JoinHandle<Result<(), Box<dyn std::error::Error + Sync + Send>>>,
        >,
        handle_production: &mut Option<
            std::thread::JoinHandle<Result<(), Box<dyn std::error::Error + Sync + Send>>>,
        >,
        handle_validation: &mut Option<
            std::thread::JoinHandle<Result<(), Box<dyn std::error::Error + Sync + Send>>>,
        >,
        handle_p2p: &mut Option<
            std::thread::JoinHandle<Result<(), Box<dyn std::error::Error + Sync + Send>>>,
        >,
    ) -> Result<(), Error> {
        loop {
            if let Some(handle) = handle_api.take_if(|handle| handle.is_finished()) {
                *shutdown += 1;
                match handle.join() {
                    Ok(Ok(_)) => {
                        tracing::info!("⏳ [{shutdown}/4] Exited API worker");
                        break Ok(());
                    }
                    Ok(Err(err)) => {
                        tracing::error!(%err, "⛔ [{shutdown}/4] API worker failure");
                        break Err(Error::WorkerError(err));
                    }
                    Err(payload) => std::panic::resume_unwind(payload),
                }
            }

            if let Some(handle) = handle_production.take_if(|handle| handle.is_finished()) {
                *shutdown += 1;
                match handle.join() {
                    Ok(Ok(_)) => {
                        tracing::info!("⏳ [{shutdown}/4] Exited production worker");
                        break Ok(());
                    }
                    Ok(Err(err)) => {
                        tracing::error!(%err, "⛔ [{shutdown}/4] Production worker failure");
                        break Err(Error::WorkerError(err));
                    }
                    Err(payload) => std::panic::resume_unwind(payload),
                };
            }

            if let Some(handle) = handle_validation.take_if(|handle| handle.is_finished()) {
                *shutdown += 1;
                match handle.join() {
                    Ok(Ok(_)) => {
                        tracing::info!("⏳ [{shutdown}/4] Exited validation worker");
                        break Ok(());
                    }
                    Ok(Err(err)) => {
                        tracing::error!(%err, "⛔ [{shutdown}/4] Validation worker failure");
                        break Err(Error::WorkerError(err));
                    }
                    Err(payload) => std::panic::resume_unwind(payload),
                };
            }

            if let Some(handle) = handle_p2p.take_if(|handle| handle.is_finished()) {
                *shutdown += 1;
                match handle.join() {
                    Ok(Ok(_)) => {
                        tracing::info!("⏳ [{shutdown}/4] Exited P2P worker");
                        break Ok(());
                    }
                    Ok(Err(err)) => {
                        tracing::error!(%err, "⛔ [{shutdown}/4] P2P worker failure");
                        break Err(Error::WorkerError(err));
                    }
                    Err(payload) => std::panic::resume_unwind(payload),
                };
            }

            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }
}
