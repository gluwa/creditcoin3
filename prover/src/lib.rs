use anyhow::Result;
use cc_client::AccountId32;
use either::Either;
use eth::Client as EthClient;
use sp_core::H256;
use std::sync::Arc;
use tokio::{
    sync::mpsc,
    time::{sleep, Duration},
};
use tracing::{debug, error, info};

use attestation::cache::AttestationCache;

use pallet_prover_primitives::Query;

pub mod config;

mod attestation;
mod cc3;
mod contract;
mod postgres;
mod query;

use crate::contract::remove_query_id;
use config::Config;

/// `AttestationCacheType` cache type
pub type AttestationCacheType = Arc<AttestationCache<H256, AccountId32>>;

/// `CcClientArc` type
pub type CcClientArc = Arc<cc3::Client>;

/// `EthClientArc` type
pub type EthClientArc = Arc<EthClient>;

/// Prover server is configured using `Config`
pub struct Server {
    config: Config,
    cc3_client: EthClient,
    // Attestation cache
    attestations_cache: AttestationCacheType,
}

impl Server {
    /// Create a new server based on `Config`
    pub async fn new(config: Config) -> Result<Self> {
        let db_pool = postgres::db::get_pool(&config.postgres_uri)?;
        postgres::db::run_migrations(config.postgres_uri.clone()).await?;

        // Create attestations cache
        let attestations_cache: AttestationCacheType =
            Arc::new(attestation::cache::AttestationCache::new(db_pool));
        info!("Created attestations cache");

        // Deploy the prover contract
        // This will deploy it on ccnext chain
        let cc3_http_url = config
            .cc3_rpc_url
            .clone()
            .replace("ws://", "http://")
            .replace("wss://", "https://");
        let cc3_eth_client =
            EthClient::new(&cc3_http_url, Some(&config.cc3_evm_private_key)).await?;

        let cc3_client = cc3::Client::new(&config.cc3_rpc_url, &config.cc3_key).await?;
        let eth_client = Arc::new(EthClient::new(&config.eth_rpc_url, None).await?);
        let chain_id = eth_client.get_chain_id().await?;
        let chain_key = cc3_client
            .get_chain_key(chain_id)
            .await?
            .expect("Prover could not find chain key on startup.");

        contract::deploy(
            &cc3_eth_client,
            config.cost_per_byte,
            config.base_fee,
            chain_key,
            config.name.clone(),
        )
        .await?;
        info!("Deployed prover contract");

        Ok(Server {
            config,
            cc3_client: cc3_eth_client,
            attestations_cache,
        })
    }

    /// Runs the server in the background, will start following the configured source chain
    pub async fn run(&mut self) -> Result<()> {
        debug!("Creating cc3 client");
        let cc3_client = cc3::Client::new(&self.config.cc3_rpc_url, &self.config.cc3_key).await?;

        let attestations_cache = self.attestations_cache.clone();
        let cc3_client = Arc::new(cc3_client);
        debug!("Created cc3 client");
        // Create an eth client
        let eth_client = Arc::new(EthClient::new(&self.config.eth_rpc_url, None).await?);

        // Get the chain id of the eth chain
        let chain_id = eth_client.get_chain_id().await?;

        let chain_key = cc3_client
            .get_chain_key(chain_id)
            .await?
            .expect("Prover could not find chain key on startup.");

        let chain_attestation_interval = cc3_client
            .get_attestation_chain_interval(chain_key)
            .await?
            .expect("Prover could not find chain attestation interval on startup.");

        // Create a channel to synchronize prover DB updates across `sync_cache`
        // and `build_historical_cache_for_chain`
        let (sender, receiver) = mpsc::unbounded_channel();

        // Sync the cache. We start this first to avoid any window where a new attestation could be
        // missed while the historical cache is being built.
        info!("Starting cache live sync");
        let sync_attestations_cache = attestations_cache.clone();
        let sync_cc3_client = cc3_client.clone();
        tokio::spawn(async move {
            attestation::cache::sync_cache(
                chain_key,
                &sync_attestations_cache,
                &sync_cc3_client,
                receiver,
            )
            .await
            .expect("Failed to sync cache");
        });

        // Build historical cache
        info!("Building historical cache for chain with id: {}", chain_key);
        attestation::cache::build_historical_cache_for_chain(
            chain_key,
            attestations_cache.clone(),
            cc3_client.clone(),
            sender,
        )
        .await?;

        info!("Starting unprocessed query processing...");
        let unprocessed_queries = contract::get_unprocessed_queries(&self.cc3_client).await?;
        for query in unprocessed_queries {
            info!("Processing unprocessed query: {:?}", query);
            if let Err(e) = self
                .process_query(query.clone(), chain_attestation_interval)
                .await
            {
                error!("Query processing failed, Error: {e:?}");
                remove_query_id(&self.cc3_client, query.id()).await?;
            }
        }

        // Create a channel for query submission
        let (queue, mut queries) = mpsc::unbounded_channel();

        let eth_cc3_client = self.cc3_client.clone();
        tokio::spawn(async move {
            loop {
                info!("Starting query submission subscription...");
                match contract::subscribe_query_submission(&eth_cc3_client, queue.clone()).await {
                    Ok(()) => {}
                    Err(e) => {
                        error!("Failed to subscribe to query submission: {:?}", e);
                        // Optional: Break the loop after a certain number of retries if desired
                        info!("Retrying subscription in one second...");
                        sleep(Duration::from_secs(1)).await; // Delay before retrying
                    }
                }
            }
        });

        info!("Listening for new queries...");
        // Wait for new queries and handle them
        while let Some(query) = queries.recv().await {
            info!("Processing query: {:?}", query);
            if let Err(e) = self
                .process_query(query.clone(), chain_attestation_interval)
                .await
            {
                error!("Query processing failed, Error: {e:?}");
                remove_query_id(&self.cc3_client, query.id()).await?;
            }
        }

        Ok(())
    }

    async fn process_query(&self, query: Query, chain_attestation_interval: u64) -> Result<()> {
        // Create an eth client
        let eth_client = Arc::new(EthClient::new(&self.config.eth_rpc_url, None).await?);

        let r = query::process(
            eth_client.clone(),
            &query,
            &self.attestations_cache,
            self.config.prover_be_socket_addr.is_none(),
            chain_attestation_interval,
        )
        .await?;

        match r {
            Either::Left(proof) => {
                info!("Submitting proof for query: {:?}", query.id());
                contract::submit_proof(&self.cc3_client, query, proof).await?;
            }
            Either::Right(stone_proof_public_input) => {
                info!("Handling external proof for query: {:?}", query.id());
                let proof = query::external::handle_proof_order(
                    query.id(),
                    stone_proof_public_input,
                    self.config
                        .prover_be_socket_addr
                        .as_ref()
                        .expect("Socket addr is Some if we are in light mode"),
                    self.config.be_api_key.as_ref().expect("We check in main() that be_api_key is always Some if prover_be_socket_addr is Some"),
                )
                .await?;
                info!("Submitting proof for query: {:?}", query);
                contract::submit_proof(&self.cc3_client, query, proof).await?;
            }
        }

        Ok(())
    }
}
