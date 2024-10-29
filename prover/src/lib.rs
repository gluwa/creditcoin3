use anyhow::Result;
use cc_client::AccountId32;
use eth::Client as EthClient;
use sp_core::H256;
use std::sync::Arc;
use tokio::{
    sync::mpsc,
    time::{sleep, Duration},
};
use tracing::{debug, error, info};

use attestation_cache::AttestationCache;

pub mod config;

mod attestation;
mod attestation_cache;
mod cc3;
mod claim;
mod contract;
mod fragment;
mod postgres;

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
            Arc::new(attestation_cache::AttestationCache::new(db_pool));
        info!("Created attestations cache");

        // Deploy the prover contract
        // This will deploy it on ccnext chain
        let cc3_http_url = config
            .cc3_rpc_url
            .clone()
            .replace("ws://", "http://")
            .replace("wss://", "https://");
        let cc3_eth_client = EthClient::new(&cc3_http_url, &config.eth_private_key).await?;
        contract::deploy(&cc3_eth_client).await?;
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

        // Create an eth client
        let eth_client = Arc::new(EthClient::new(&self.config.eth_rpc_url, &String::new()).await?);

        // Get the chain id of the eth chain
        let chain_id = eth_client.get_chain_id().await?;

        let chain_key = cc3_client
            .get_chain_key(chain_id)
            .await?
            .expect("Prover could not find chain key on startup.");

        // Create a channel to synchronize prover DB updates across `sync_cache`
        // and `build_historical_cache_for_chain`
        let (sender, receiver) = mpsc::unbounded_channel();

        // Sync the cache. We start this first to avoid any window where a new attestation could be
        // missed while the historical cache is being built.
        info!("Starting cache live sync");
        let sync_attestations_cache = attestations_cache.clone();
        let sync_cc3_client = cc3_client.clone();
        tokio::spawn(async move {
            attestation_cache::sync_cache(
                chain_key,
                &sync_attestations_cache,
                &sync_cc3_client,
                receiver,
            )
            .await
            .expect("Failed to sync cache");
        });

        // Build historical cache
        info!("Building historical cache for chain with id: {}", chain_id);
        attestation_cache::build_historical_cache_for_chain(
            chain_id,
            attestations_cache.clone(),
            cc3_client.clone(),
            sender,
        )
        .await?;

        let cc_eth_client = Arc::new(self.cc3_client.clone());

        info!("Starting unprocessed claim processing...");
        let unprocessed_queries = contract::get_unprocessed_queries(&self.cc3_client).await?;
        for query in unprocessed_queries {
            let eth_client = eth_client.clone();
            if self.config.test_mode {
                info!("Processing unprocessed query in test mode");
                claim::_dummy_process(eth_client, query, &attestations_cache).await?;
            } else {
                info!("Processing unprocessed query: {:?}", query);
                match claim::process(eth_client, &query, &attestations_cache).await {
                    Ok(proof) => {
                        info!("Submitting proof for query: {:?}", query);
                        contract::submit_proof(&cc_eth_client, query, proof).await?;
                    }
                    Err(e) => {
                        error!("Failed to process query: {:?}", e);
                    }
                }
            }
        }

        // Create a channel for query submission
        let (sender, mut receiver) = mpsc::unbounded_channel();

        info!("Starting query submission subscription...");
        let eth_client_for_query_sub = Arc::new(self.cc3_client.clone());
        tokio::spawn(async move {
            loop {
                match contract::subscribe_query_submission(
                    eth_client_for_query_sub.clone(),
                    sender.clone(),
                )
                .await
                {
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
        while let Some(query) = receiver.recv().await {
            let eth_client = eth_client.clone();

            if self.config.test_mode {
                info!("Processing query in test mode");
                claim::_dummy_process(eth_client, query, &attestations_cache).await?;
            } else {
                info!("Processing query: {:?}", query);
                match claim::process(eth_client, &query, &attestations_cache).await {
                    Ok(proof) => {
                        info!("Submitting proof for query: {:?}", query);
                        contract::submit_proof(&cc_eth_client, query, proof).await?;
                    }
                    Err(e) => {
                        error!("Failed to process query: {:?}", e);
                    }
                }
            }
        }

        Ok(())
    }
}
