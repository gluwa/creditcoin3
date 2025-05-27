use anyhow::{anyhow, Result};
use cc_client::AccountId32;
use either::Either;
use eth::Client as EthClient;
use futures::stream::{FuturesUnordered, StreamExt};
use sp_core::H256;
use std::{collections::HashSet, sync::Arc};
use tokio::{
    sync::mpsc,
    task::{self, JoinHandle},
    time::{interval, Duration, Interval, MissedTickBehavior},
};
use tracing::{debug, error, info};
use query::external::Error as LightProvingError;

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

/// Query polling interval
/// Defines how often the prover will poll for unprocessed queries
const QUERY_POLLING_INTERVAL: Duration = Duration::from_secs(60);

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
            config.timeout,
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
        let sync_cache_handle = tokio::spawn(async move {
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

        // Interval for polling unprocessed queries
        let mut polling_interval = interval(QUERY_POLLING_INTERVAL);

        // Set missed tick behavior to delay, so that missed ticks are delayed rather than skipped,
        // preventing them from accumulating.
        polling_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

        self.handle_ongoing_queries_and_fatal_errors(
            polling_interval,
            sync_cache_handle,
            chain_attestation_interval,
        )
        .await?;
        Ok(())
    }

    async fn process_query(&self, query: Query, chain_attestation_interval: u64) -> Result<()> {
        if self.config.prover_be_socket_addr.is_some() {
            return Err(anyhow!(
                "Tried to prove query locally while in light prover mode."
            ));
        };
        // Create an eth client
        let eth_client = EthClient::new(&self.config.eth_rpc_url, None).await?;

        let r = query::process(
            eth_client,
            &query,
            &self.attestations_cache,
            true,
            chain_attestation_interval,
        )
        .await?;

        if let Either::Left(proof) = r {
            info!("Submitting proof for query: {:?}", query.id());
            contract::submit_proof(&self.cc3_client, query, proof).await?;
        }

        Ok(())
    }

    async fn queue_light_proving_jobs(
        &self,
        queries: Vec<Query>,
        chain_attestation_interval: u64,
    ) -> Result<Vec<JoinHandle<(Query, Result<Vec<u8>, LightProvingError>)>>> {
        // Create thread safe versions of config strings
        let prover_be_socket_addr = Arc::new(self.config.prover_be_socket_addr.clone().ok_or(
            anyhow!("Tried to submit light proving jobs while not in light mode!"),
        )?);
        let be_api_key = Arc::new(self.config.be_api_key.clone().ok_or(anyhow!(
            "We check in main() that be_api_key is always Some if prover_be_socket_addr is Some"
        ))?);
        // Create an eth client
        let eth_client = EthClient::new(&self.config.eth_rpc_url, None).await?;
        let mut proving_job_handles: Vec<JoinHandle<(Query, Result<Vec<u8>, LightProvingError>)>> = Vec::new();

        for query in queries {
            let r = query::process(
                eth_client.clone(),
                &query,
                &self.attestations_cache,
                false,
                chain_attestation_interval,
            )
            .await?;

            if let Either::Right(stone_proof_public_input) = r {
                info!("Handling external proof for query: {:?}", query.id());
                // Cloning handles for config strings
                let addr_clone = prover_be_socket_addr.clone();
                let key_clone = be_api_key.clone();
                proving_job_handles.push(task::spawn(async move {
                    let proving_result = query::external::handle_proof_order(
                        query.id(),
                        stone_proof_public_input,
                        addr_clone.as_ref(),
                        key_clone.as_ref(),
                    )
                    .await;
                    (query, proving_result)
                }));
            }
        }
        Ok(proving_job_handles)
    }

    async fn handle_ongoing_queries_and_fatal_errors(
        &mut self,
        mut polling_interval: Interval,
        mut sync_cache_handle: JoinHandle<()>,
        chain_attestation_interval: u64,
    ) -> Result<()> {
        let mut light_prover_queries = FuturesUnordered::new();
        let mut queued_light_proving_queries: HashSet<H256> = HashSet::new();
        loop {
            tokio::select! {
                _ = polling_interval.tick() => {
                    info!("Polling unprocessed queries...");
                    match contract::get_unprocessed_queries(&self.cc3_client).await {
                        Ok(mut unprocessed_queries) => {
                            if self.config.prover_be_socket_addr.is_some() {
                                // We don't want to spam the BE with requests for queries we've already requested.
                                unprocessed_queries.retain(|query| {
                                    !queued_light_proving_queries.contains(&query.id())
                                });
                                info!("Found {} new unprocessed queries", unprocessed_queries.len());
                                // Save these off to use later without cloning queries
                                let query_ids: Vec<H256> = unprocessed_queries.iter().map(|query| {
                                    query.id()
                                }).collect();
                                match self.queue_light_proving_jobs(unprocessed_queries, chain_attestation_interval).await {
                                    Ok(new_query_handles) => {
                                        for query_handle in new_query_handles {
                                            light_prover_queries.push(query_handle);
                                        }
                                    },
                                    Err(e) => {
                                        error!("Queuing light proving for queries failed, Error: {e:?}");
                                    }
                                };
                                // All queries were successfully queued as light proving jobs.
                                for query_id in query_ids {
                                    queued_light_proving_queries.insert(query_id);
                                }
                            } else {
                                // If not light prover, get first one because we won't process all of them at once
                                let query = unprocessed_queries.first().cloned();
                                if let Some(query) = query {
                                    info!("Processing unprocessed query: {:?}", query);
                                    if let Err(e) = self
                                        .process_query(query.clone(), chain_attestation_interval)
                                        .await
                                    {
                                        error!("Query processing failed, Error: {e:?}");
                                        remove_query_id(&self.cc3_client, query.id()).await?;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            error!("Failed to fetch unprocessed queries: {:?}", e);
                        }
                    }
                }
                _ = &mut sync_cache_handle => {
                    panic!("Sync cache thread aborted.")
                },
                Some(result) = light_prover_queries.next() => {
                    match result {
                        Ok((query, result_inner)) => {
                            match result_inner {
                                Ok(proof) => {
                                    info!("Submitting proof for query: {:?}", query);
                                    queued_light_proving_queries.remove(&query.id());
                                    contract::submit_proof(&self.cc3_client, query, proof).await?;
                                },
                                Err(e) => {
                                    error!("Query processing failed, Error: {e:?}");
                                    if let LightProvingError::ProofGenerationFailed = e {
                                        panic!("Query processing failed fatally. Prover BE pipeline is likely rejecting proving jobs due to auth/ip. Fix prover BE then restart.");
                                    } else {
                                        queued_light_proving_queries.remove(&query.id());
                                        remove_query_id(&self.cc3_client, query.id()).await?;
                                    }
                                }
                            }
                        },
                        Err(join_err) => {
                            panic!("Fatal error, couldn't join query worker task, error: {join_err:?}");
                        }
                    }
                }
            }
        }
    }
}
