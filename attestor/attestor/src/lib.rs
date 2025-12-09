pub mod attestation;
pub mod chain_listener;
pub mod common;
pub mod worker;

mod error;
mod util;

pub use error::Error;

// ----------------------------------------- [ Exports ] --------------------------------------- //

pub mod prelude {
    pub use crate::common;
    pub(crate) use crate::util;

    pub(crate) use crate::ensure;
    pub(crate) use crate::hash_set;

    pub const WORKER_COUNT: usize = 3;
}

// -------------------------------------- [ Configuration ] ------------------------------------ //

#[derive(Debug, attestor_macro::Builder)]
pub struct Config {
    name: String,
    chain_key: attestor_primitives::ChainKey,
    eth: chain_listener::eth::ConfigIncomplete,
    cc3: chain_listener::cc3::ConfigIncomplete,
    p2p: worker::p2p::ConfigIncomplete,
    pool: worker::validation::pool::ConfigIncomplete,
    attestation: attestation::Config,
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

        let secret_uri = subxt_signer::SecretUri::from_str(&self.config.cc3.cc3_key.to_string())
            .expect("Failed to create secret uri");
        let keypair = subxt_signer::sr25519::Keypair::from_uri(&secret_uri)
            .expect("Failed to create secret keypair");
        let account_id = cc_client::AccountId32(keypair.public_key().0);

        tracing::info!(name = self.config.name, %account_id, chain_key = self.config.chain_key, "🙋‍♀️ Starting attestor");

        let monitor = worker::CancellationMonitor::new();

        // -----------------------------------* Chain endpoints *----------------------------------

        loop {
            tokio::select! {
                Ok(_) = tokio_tungstenite::connect_async(&self.config.eth.eth_url) => {
                    break;
                }
                _ = tokio::time::sleep(common::constants::RETRY_DELAY) => {}
            }
            tracing::info!(
                url = %self.config.eth.eth_url,
                "🛜 waiting for Eth WS connection to be made available..."
            );
        }

        loop {
            tokio::select! {
                Ok(_) = tokio_tungstenite::connect_async(&self.config.cc3.cc3_url) => {
                    break;
                }
                _ = tokio::time::sleep(common::constants::RETRY_DELAY) => {}
            }
            tracing::info!(
                url = %self.config.cc3.cc3_url,
                "🛜 waiting for CC3 WS connection to be made available..."
            );
        }

        // ----------------------------------* Connection to CC3 *---------------------------------

        let cc3_key = self.config.cc3.cc3_key.to_string();
        let cc3_client = cc_client::Client::new(&self.config.cc3.cc3_url.to_string(), &cc3_key)
            .await
            .map_err(Error::CC3Error)?;

        // ---------------------------------* Chain configuration *--------------------------------

        let attestation_interval = match self.config.attestation.attestation_interval {
            Some(attestation_interval) => attestation_interval,
            None => cc3_client
                .chain_attestation_interval(self.config.chain_key)
                .await
                .map_err(Error::CC3Error)?
                .map(std::num::NonZeroU64::new)
                .ok_or(Error::MissingAttestationInterval(self.config.chain_key))?
                .unwrap(),
        };

        let attestation_start_cc3 = match cc3_client
            .fetch_last_digest(self.config.chain_key)
            .await
            .map_err(|err| Error::InitError(Box::new(err)))?
        {
            Some(last_digest) => match cc3_client
                .get_attestation_by_digest(self.config.chain_key, last_digest)
                .await
                .map_err(|err| Error::InitError(Box::new(err)))?
            {
                Some(last_attestation) => {
                    Some((last_attestation.digest(), last_attestation.header_number()))
                }
                None => {
                    unreachable!("Invalid last digest, something has gone very wrong!");
                }
            },
            None => cc3_client
                .get_last_checkpoint(self.config.chain_key)
                .await
                .map_err(|err| Error::InitError(Box::new(err)))?
                .map(|last_checkpoint| (last_checkpoint.digest, last_checkpoint.block_number)),
        };

        let attestors = cc3_client
            .get_attestor_active_set(self.config.chain_key)
            .await
            .map_err(|err| Error::InitError(Box::new(err)))?;
        let attestors = worker::validation::pool::AttestorValidatePermissioned::new(
            std::collections::HashSet::from_iter(attestors.into_iter().map(|attestor| {
                attestor_primitives::AttestorId::new(sp_core::crypto::AccountId32::new(attestor.0))
            })),
        );

        let start_height = match self.config.attestation.start_height {
            Some(start_height) => start_height,
            None => match attestation_start_cc3 {
                Some((_digest, height)) => util::next_multiple_of(attestation_interval, height),
                None => cc3_client
                    .get_attestation_chain_genesis_block_number(self.config.chain_key)
                    .await
                    .unwrap_or_default(),
            },
        };

        let target = cc3_client
            .target_sample_size(self.config.chain_key)
            .await
            .map_err(|_| Error::MissingTargetSampleSize(self.config.chain_key))?;
        let quorum =
            std::num::NonZeroUsize::new(attestor_primitives::calculate_threshold(target) as usize)
                .expect("Failed to compute quorum threshold");

        tracing::info!(quorum, "🧑‍🤝‍🧑 Retrieved target sample size");

        // ----------------------------------* Chain listeners *-------------------------------- //

        let config = self
            .config
            .cc3
            .clone()
            .with_chain_key(self.config.chain_key)
            .with_cc3_client(cc3_client.clone())
            .with_start_height(start_height)
            .build();
        let cc3_production = chain_listener::cc3::CC3::new(config)
            .await
            .map_err(Error::CC3Error)?;

        let config = self
            .config
            .cc3
            .with_chain_key(self.config.chain_key)
            .with_cc3_client(cc3_client)
            .with_start_height(start_height)
            .build();
        let cc3_validation = chain_listener::cc3::CC3::new(config)
            .await
            .map_err(Error::CC3Error)?;

        let config = self
            .config
            .eth
            .with_attestation_interval(attestation_interval)
            .with_start_height(start_height)
            .build();
        let eth = chain_listener::eth::Ethereum::new(config)
            .await
            .map_err(Error::EthError)?;

        let config = chain_listener::rebroadcast::ConfigBuilder::new()
            .with_rebroadcast_interval(self.config.attestation.rebroadcast_interval)
            .build();
        let rebroadcast = chain_listener::rebroadcast::Rebroadcast::new(config).await;

        // ----------------------------------* Chain constants *--------------------------------- //

        let api = cc3_production.api();
        let call = cc_client::cc3::constants()
            .attestation()
            .max_attestations_per_block();
        let max_attestations_per_block = api.constants().at(&call).unwrap();

        // ----------------------------------* Message passing *-------------------------------- //

        // P2P Subscriber changes
        let can_broadcast_production =
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let can_broadcast_p2p = std::sync::Arc::clone(&can_broadcast_production);

        // attestation production -> p2p sync
        let (p2p_sender, p2p_receiver) =
            tokio::sync::broadcast::channel(common::constants::CAPACITY_CHANNEL);

        // attestation production / p2p sync -> attestation validation
        let epoch = cc3_production
            .get_current_epoch()
            .await
            .map_err(|err| Error::InitError(Box::new(err)))?;

        let config = self
            .config
            .pool
            .with_attestors(attestors)
            .with_quorum(quorum)
            .with_start_height(start_height)
            .with_attestation_interval(attestation_interval)
            .with_max_attestations_per_block(max_attestations_per_block)
            .build();
        let (validation_sender, validation_receiver) =
            worker::validation::pool::attestation_pool(config);

        // attestation production -> attestation validation
        let (attestation_latest_sender, attestation_latest_receiver) =
            tokio::sync::watch::channel(None);

        // attestation validation -> attestation production
        let (attestation_invalidation_sender, attestation_invalidation_receiver) =
            tokio::sync::watch::channel(None);

        // ------------------------------* Attestation Production *----------------------------- //

        tracing::info!("⏳ [1/3] Starting attestation production worker");

        let config = worker::production::ConfigBuilder::new()
            .with_eth(eth)
            .with_cc3(cc3_production)
            .with_account_id(account_id)
            .with_rebroadcast(rebroadcast)
            .with_sender_p2p(p2p_sender)
            .with_sender_validation(validation_sender.clone())
            .with_sender_attestation_latest(attestation_latest_sender)
            .with_receiver_attestation_invalidation(attestation_invalidation_receiver)
            .with_can_broadcast(can_broadcast_production)
            .with_attestation_start_cc3(attestation_start_cc3)
            .with_max_attestations_per_block(max_attestations_per_block)
            .with_epoch(epoch)
            .build();
        let attestation_production = worker::production::WorkerAttestationProduction::new(config)
            .await
            .map_err(Error::WorkerError)?;
        let handle_production = monitor.spawn(attestation_production);

        // ------------------------------* Attestation Validation *----------------------------- //

        tracing::info!("⏳ [2/3] Starting attestation validation worker");

        let config = worker::validation::ConfigBuilder::new()
            .with_cc3(cc3_validation)
            .with_sender_attestation_invalidation(attestation_invalidation_sender)
            .with_receiver_validation(validation_receiver)
            .with_receiver_attestation_latest(attestation_latest_receiver)
            .with_api_calls(cc_client::Client::runtime_api())
            .with_api(api)
            .with_keypair(keypair)
            .build();
        let attestation_validation = worker::validation::WorkerAttestationValidation::new(config);
        let handle_validation = monitor.spawn(attestation_validation);

        // -------------------------------------* P2P Sync *------------------------------------ //

        tracing::info!("⏳ [3/3] Starting P2P worker");

        let config = self
            .config
            .p2p
            .with_keypair(libp2p::identity::Keypair::generate_ed25519())
            .with_receiver_p2p(p2p_receiver)
            .with_sender_validation(validation_sender)
            .with_can_broadcast(can_broadcast_p2p)
            .with_chain_key(self.config.chain_key)
            .build();
        let p2p = worker::p2p::WorkerP2P::new(config).map_err(Error::WorkerError)?;
        let handle_p2p = monitor.spawn(p2p);

        tracing::info!("✅ All services online!");

        // ----------------------------------* Thread waiting *--------------------------------- //

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("🔌 Received shutdown signal");
                monitor.shutdown();
            },
            _ = monitor.cancelled() => {}
        }

        let mut res = Ok(());

        match handle_production.join() {
            Ok(res_production) => res = res.and(res_production.map_err(Error::WorkerError)),
            Err(payload) => std::panic::resume_unwind(payload),
        };

        tracing::info!("⏳ [1/3] Shutting down attestation production worker");

        match handle_validation.join() {
            Ok(res_validation) => res = res.and(res_validation.map_err(Error::WorkerError)),
            Err(payload) => std::panic::resume_unwind(payload),
        };

        tracing::info!("⏳ [2/3] Shutting down attestation validation worker");

        match handle_p2p.join() {
            Ok(res_p2p) => res = res.and(res_p2p.map_err(Error::WorkerError)),
            Err(payload) => std::panic::resume_unwind(payload),
        };

        tracing::info!("⏳ [3/3] Shutting down p2p worker");

        res
    }
}
