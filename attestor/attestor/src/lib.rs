pub mod attestation;
pub mod common;
pub mod stream;
pub mod worker;

mod error;
mod events;

pub use error::Error;

// ----------------------------------------- [ Exports ] --------------------------------------- //

pub mod prelude {
    pub use crate::common;
    pub use common::user::*;
}

use crate::prelude::*;

// -------------------------------------- [ Configuration ] ------------------------------------ //

#[derive(Debug, attestor_macro::Builder)]
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

    #[tracing::instrument(name = "attestor", skip_all)]
    pub async fn run(self) -> Result<(), Error> {
        use std::str::FromStr as _;

        // --------------------------------------* Identity *--------------------------------------

        let secret = self.config.stream.secret.to_string();
        let secret_uri =
            subxt_signer::SecretUri::from_str(&secret).expect("Failed to create secret uri");
        let keypair_cc3 = subxt_signer::sr25519::Keypair::from_uri(&secret_uri)
            .expect("Failed to create secret keypair");
        let account_id = cc_client::AccountId32(keypair_cc3.public_key().0);

        let mut seed = self.config.stream.secret.to_seed_normalized("");
        let keypair_p2p = libp2p::identity::Keypair::ed25519_from_bytes(&mut seed[..32]).unwrap();
        let peer_id = libp2p::PeerId::from_public_key(&keypair_p2p.public());

        tracing::info!(name = self.config.name, %account_id, chain_key = self.config.chain_key, "🙋‍♀️ Starting attestor");

        let monitor = worker::CancellationMonitor::new();

        // -----------------------------------* Chain endpoints *----------------------------------

        self.wait_for_endpoints().await.extract_interrupt()?;

        let client_cc3 = cc_client::Client::new(self.config.stream.url_cc3.as_ref(), &secret)
            .await
            .map_err(Error::InitError)?;

        let client_eth = eth::Client::new(self.config.stream.url_eth.as_ref(), None)
            .await
            .map_err(Error::InitError)?;

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

        tracing::info!("🔑 Making sure attestor bls key is registered...");

        let bls_key = match self.register_bls(&client_cc3).await {
            Ok(bls_key) => bls_key,
            Err(Interrupt::Stop) => {
                tracing::info!("🔌 Received shutdown signal");
                return Ok(());
            }
            Err(Interrupt::Cont(err)) => {
                tracing::error!(%err, "⛔ Failed to register attestor bls public key");
                return Err(err);
            }
        };

        // -----------------------------------* Eligibility *----------------------------------- //

        tracing::info!(
            attestor = %account_id,
            "⏲️ Waiting for attestor to be made eligible"
        );

        let attestors = match self
            .wait_for_eligible(&client_cc3, &account_id, &mut stream_cc3_genesis)
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

        let genesis = client_cc3
            .get_attestation_chain_genesis_block_number(self.config.chain_key)
            .await
            .map_err(Error::RpcError)?;

        let start_info = match client_cc3
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

        let start_height = self.config.attestation.start_height.unwrap_or(
            start_info
                .as_ref()
                .map(|info| info.height)
                .unwrap_or(genesis),
        ) + 1;

        let target = client_cc3
            .target_sample_size(self.config.chain_key)
            .await
            .map_err(|_| Error::MissingTargetSampleSize(self.config.chain_key))?;
        let quorum =
            std::num::NonZeroUsize::new(attestor_primitives::calculate_threshold(target) as usize)
                .expect("Failed to compute quorum threshold");

        tracing::info!(quorum, ?start_info, "🧑‍🤝‍🧑 Retrieved chain data");

        // -------------------------------------* Attestation *------------------------------------

        let config = stream::attestation::ConfigBuilder::new()
            .with_cc3(client_cc3.clone())
            .with_eth(client_eth)
            .with_bls_key(bls_key)
            .with_interval_attestation(interval_attestation)
            .with_chain_key(self.config.chain_key)
            .with_start_height(start_height)
            .with_start_info(start_info)
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
            .with_start_info(start_info)
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
            .with_start_info(start_info)
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

        let api = client_cc3.api().await.map_err(Error::RpcError)?;
        let config = worker::validation::ConfigBuilder::new()
            .with_stream_cc3(stream_cc3_validation)
            .with_cc3(client_cc3.clone())
            .with_keypair(keypair_cc3)
            .with_validation_receiver(receiver_validation)
            .with_validation_sender(sender_validation.clone())
            .with_api_calls(cc_client::Client::runtime_api())
            .with_api(api)
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

        let attestation_latest_cc3 = match start_info {
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
                        tracing::error!(%err, "⛔ Failed to register attestor bls public key");
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

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("🔌 Received shutdown signal");
                monitor.shutdown();
            },
            _ = monitor.cancelled() => {}
        }

        // FIXME: have this reference the number of worker under the monitor
        let mut res = Ok(());
        let mut shutdown = 0;
        while shutdown < common::constants::WORKER_COUNT {
            if let Some(handle) = handle_api.take_if(|handle| handle.is_finished()) {
                shutdown += 1;
                match handle.join() {
                    Ok(res_metrics) => res = res.and(res_metrics.map_err(Error::WorkerError)),
                    Err(payload) => std::panic::resume_unwind(payload),
                }
                tracing::info!("⏳ [{shutdown}/4] Shutting down API worker");
            }

            if let Some(handle) = handle_production.take_if(|handle| handle.is_finished()) {
                shutdown += 1;
                match handle.join() {
                    Ok(res_production) => res = res.and(res_production.map_err(Error::WorkerError)),
                    Err(payload) => std::panic::resume_unwind(payload),
                };
                tracing::info!("⏳ [{shutdown}/4] Shutting down attestation production worker");
            }

            if let Some(handle) = handle_validation.take_if(|handle| handle.is_finished()) {
                shutdown += 1;
                match handle.join() {
                    Ok(res_validation) => res = res.and(res_validation.map_err(Error::WorkerError)),
                    Err(payload) => std::panic::resume_unwind(payload),
                };
                tracing::info!("⏳ [{shutdown}/4] Shutting down attestation validation worker");
            }

            if let Some(handle) = handle_p2p.take_if(|handle| handle.is_finished()) {
                shutdown += 1;
                match handle.join() {
                    Ok(res_p2p) => res = res.and(res_p2p.map_err(Error::WorkerError)),
                    Err(payload) => std::panic::resume_unwind(payload),
                };
                tracing::info!("⏳ [{shutdown}/4] Shutting down p2p worker");
            }

            std::thread::sleep(std::time::Duration::from_millis(200));
        }

        res
    }

    async fn register_bls(
        &self,
        client_cc3: &cc_client::Client,
    ) -> Result<bls_signatures::PrivateKey, Interrupt<Error>> {
        use anyhow::Context as _;
        use bls_signatures::Serialize as _;

        let bls_key =
            bls_signatures::PrivateKey::new(self.config.stream.secret.to_string().as_bytes());

        let is_bls_key_regsitered = client_cc3
            .check_attestor_key_is_registered(self.config.chain_key)
            .await
            .context("Failed to check attestor bls registration")
            .map_interrupt(Error::InitError)?;

        if !is_bls_key_regsitered {
            tracing::info!("🔑  registering attestor bls pubkey...");

            let mut bls_public_key = [0; 48];
            let bytes = &bls_key.public_key().as_bytes();
            bls_public_key.copy_from_slice(bytes);

            let mut proof_of_possession = [0; 96];
            let bytes = &bls_key.sign(bls_public_key).as_bytes()[..96];
            proof_of_possession.copy_from_slice(bytes);

            client_cc3
                .start_attesting(self.config.chain_key, bls_public_key, proof_of_possession)
                .await
                .context("Failed to register attestor bls pubkey")
                .map_interrupt(Error::InitError)?;

            tokio::select! {
                res = client_cc3.start_attesting(
                    self.config.chain_key,
                    bls_public_key,
                    proof_of_possession,
                ) => {
                    res.context("Failed to register attestor bls pubkey")
                        .map_interrupt(Error::InitError)?;
                }
                _ = tokio::signal::ctrl_c() => {
                    return Err(Interrupt::Stop);
                }
            }
        }

        Ok(bls_key)
    }

    async fn wait_for_endpoints(&self) -> Result<(), Interrupt<Error>> {
        loop {
            tokio::select! {
                Ok(_) = tokio_tungstenite::connect_async(&self.config.stream.url_eth) => {
                    break;
                }
                _ = tokio::time::sleep(common::constants::RETRY_DELAY) => {}
                _ = tokio::signal::ctrl_c() => {
                    return Err(Interrupt::Stop);
                }
            }
            tracing::info!(
                url = %self.config.stream.url_eth,
                "🛜 waiting for Eth WS connection to be made available..."
            );
        }

        loop {
            tokio::select! {
                Ok(_) = tokio_tungstenite::connect_async(&self.config.stream.url_cc3) => {
                    break;
                }
                _ = tokio::time::sleep(common::constants::RETRY_DELAY) => {}
                _ = tokio::signal::ctrl_c() => {
                    return Err(Interrupt::Stop);
                }
            }
            tracing::info!(
                url = %self.config.stream.url_cc3,
                "🛜 waiting for CC3 WS connection to be made available..."
            );
        }

        Ok(())
    }

    async fn wait_for_eligible(
        &self,
        client_cc3: &cc_client::Client,
        account_id: &cc_client::AccountId32,
        stream_cc3: &mut stream::cc3::StreamCC3,
    ) -> Result<Vec<cc_client::AccountId32>, Interrupt<Error>> {
        use anyhow::Context as _;
        use futures::StreamExt as _;

        let mut attestors = client_cc3
            .get_attestor_active_set(self.config.chain_key)
            .await
            .map_interrupt(Error::RpcError)?;

        let cc3_block_time_ms = client_cc3
            .api()
            .await
            .context("Failed to initialize cc3 api")
            .map_interrupt(Error::InitError)?
            .constants()
            .at(&cc_client::cc3::constants().timestamp().minimum_period())
            .context("Failed to retrieve cc3 block time")
            .map_interrupt(Error::InitError)?
            * 2;

        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        if !attestors.contains(&account_id) {
            attestors = 'outer: loop {
                tokio::select! {
                    Some(block) = stream_cc3.next() => {
                        let block = block.map_interrupt(Error::CC3Error)?;
                        let events = block.events().await.map_interrupt(Error::CC3Error)?;

                        for event in events {
                            let event = event.map_interrupt(Error::CC3Error)?;
                            if let cc_client::attestation::CcEvent::AttestorsElected(attestors) = event {
                                if attestors.contains(&account_id) {
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
                            attestor = %account_id,
                            "⏲️  waiting on attestor..."
                        );
                    }
                }
            }
        }

        tracing::info!(%account_id, "☀️ Attestor is eligible for production");

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
        use events::EventAttestationFinalization as _;
        use futures::StreamExt as _;

        let attestation_genesis = stream_attestation
            .generate_attestation_genesis()
            .await
            .map_interrupt(Error::AttestationError)?;

        let height = attestation_genesis.header_number();
        let digest = attestation_genesis.digest();
        let digest_prev = attestation_genesis.prev_digest();
        let attestor_id = attestation_genesis.attestor.clone();

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
                Some(block) = stream_cc3.next() => {
                    let block = block.map_interrupt(Error::CC3Error)?;
                    let events = block.events().await.map_interrupt(Error::CC3Error)?;

                    for event in events  {
                        let event = event.map_interrupt(Error::CC3Error)?;
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
                        attestor = %account_id,
                        "⏲️  waiting on submission..."
                    );
                }
            }
        };

        stream_attestation
            .note_attestation_finalization(attestation_latest_cc3)
            .expect("Infallible");
        sender_validation
            .note_attestation_finalization(attestation_latest_cc3)
            .expect("Infallible");

        Ok(attestation_latest_cc3)
    }
}
