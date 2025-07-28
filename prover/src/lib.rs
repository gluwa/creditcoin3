use anyhow::{anyhow, Result};
use cc_client::AccountId32;
use either::Either;
use eth::Client as EthClient;
use futures::stream::{FuturesUnordered, StreamExt};
use query::external::Error as LightProvingError;
use sp_core::H256;
use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};
use tokio::{
    sync::mpsc,
    task::{self, JoinError, JoinHandle},
};
use tracing::{debug, error, info, warn};

use attestation::cache::AttestationCache;
use pallet_prover_primitives::Query;

pub mod config;

mod attestation;
mod cc3;
mod contract;
pub mod postgres;
mod query;

use crate::attestation::fragment::Error;
use crate::contract::mark_query_as_invalid;
use crate::postgres::from_storage_type;
use config::Config;

/// `AttestationCacheType` cache type
pub type AttestationCacheType = Arc<AttestationCache<H256, AccountId32>>;

/// `CcClientArc` type
pub type CcClientArc = Arc<cc3::Client>;

/// Prover server is configured using `Config`
pub struct Server {
    config: Config,
    cc3_client: EthClient,
    // Attestation cache
    attestations_cache: AttestationCacheType,
    // Queries that are waiting for attestations
    waiting_queries: BTreeMap<u64, Vec<Query>>,
    // Queries that have been queued for light proving
    queued_light_proving_queries: HashSet<H256>,
    // Queries that have been received
    received_query_ids: HashSet<H256>,
}

impl Server {
    /// Create a new server based on `Config`
    pub async fn new(config: Config) -> Result<Self> {
        let db_pool = postgres::db::get_pool(&config.postgres_uri)?;
        postgres::db::run_migrations(config.postgres_uri.clone()).await?;

        // Create attestations cache
        let attestations_cache: AttestationCacheType = Arc::new(AttestationCache::new(db_pool));
        info!("Created attestations cache");

        // Deploy the prover contract
        // This will deploy it on ccnext chain
        let cc3_eth_client =
            EthClient::new(&config.cc3_rpc_url, Some(&config.cc3_evm_private_key)).await?;

        let cc3_client = cc3::Client::new(&config.cc3_rpc_url, &config.cc3_key).await?;
        let eth_client = Arc::new(EthClient::new(&config.eth_rpc_url, None).await?);
        let chain_id = eth_client.chain_id();

        let supported_chain = cc3_client
            .cc_client()
            .get_supported_chain(config.chain_key)
            .await?
            .ok_or(Error::FailedToGetChainKey)?;

        if supported_chain.chain_id != chain_id {
            return Err(Error::WrongChain(chain_id, supported_chain.chain_id).into());
        }

        contract::deploy(
            &cc3_eth_client,
            config.cost_per_byte,
            config.base_fee,
            config.chain_key,
            config.name.clone(),
            config.timeout,
        )
        .await?;
        info!("Deployed prover contract");

        Ok(Server {
            config,
            cc3_client: cc3_eth_client,
            attestations_cache,
            waiting_queries: BTreeMap::new(),
            queued_light_proving_queries: HashSet::new(),
            received_query_ids: HashSet::new(),
        })
    }

    /// Runs the server in the background, will start following the configured source chain
    pub async fn run(&mut self) -> Result<()> {
        debug!("Creating cc3 client");
        let cc3_client = cc3::Client::new(&self.config.cc3_rpc_url, &self.config.cc3_key).await?;

        let attestations_cache = self.attestations_cache.clone();
        let cc3_client = Arc::new(cc3_client);
        debug!("Created cc3 client");

        let chain_key = self.config.chain_key;

        let (sender, receiver) = mpsc::unbounded_channel();
        let (attestation_notifier, new_attestation_receiver) = mpsc::unbounded_channel::<u64>();

        let sync_attestations_cache = attestations_cache.clone();
        let sync_cc3_client = cc3_client.clone();
        let sync_cache_handle = tokio::spawn(async move {
            attestation::cache::sync_cache(
                chain_key,
                &sync_attestations_cache,
                &sync_cc3_client,
                receiver,
                attestation_notifier,
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

        // Channel for new queries
        let (new_query_sender, new_query_receiver) = mpsc::unbounded_channel::<Query>();

        let client_clone = self.cc3_client.clone();

        // Spawn a task to check for queries on contract storage and subscribe to new submissions
        tokio::spawn(async move {
            let retry_delay = std::time::Duration::from_secs(10);

            loop {
                if let Err(e) =
                    contract::provide_unprocessed_queries(&client_clone, new_query_sender.clone())
                        .await
                {
                    error!(
                        "🔴 Query provider failed: {:?}. Retrying after {:?}...",
                        e, retry_delay
                    );
                }
                tokio::time::sleep(retry_delay).await;
            }
        });

        self.handle_ongoing_queries_and_fatal_errors(
            sync_cache_handle,
            new_attestation_receiver,
            new_query_receiver,
        )
        .await?;
        Ok(())
    }

    async fn handle_ongoing_queries_and_fatal_errors(
        &mut self,
        mut sync_cache_handle: JoinHandle<()>,
        mut new_attestation_receiver: mpsc::UnboundedReceiver<u64>,
        mut new_query_receiver: mpsc::UnboundedReceiver<Query>,
    ) -> Result<()> {
        let mut light_prover_queries = FuturesUnordered::new();

        loop {
            tokio::select! {
                _ = &mut sync_cache_handle => {
                    panic!("Sync cache thread aborted.")
                },
                Some(new_query) = new_query_receiver.recv() => {
                    let query_id = new_query.id();

                    if !self.received_query_ids.insert(query_id) {
                        warn!("Received duplicate query {:?}, ignoring.", query_id);
                        continue;
                    }

                    info!("Received query {:?}, checking for readiness...", query_id);

                    let last_attestation_height = from_storage_type(self.attestations_cache.last_synced_attestation(new_query.chain_id).await
                        .map_err(|e| Error::ProverDBError(e.to_string()))?
                        .ok_or(Error::NoAttestationsSynced)?
                        .header_number);

                    // Check if the query is ready to be processed
                    if last_attestation_height < new_query.height {
                        info!(
                            "Query {:?} is not ready. Last attestation: {}, needed: {}. Adding to waiting queue.",
                            query_id, last_attestation_height, new_query.height
                        );

                        // Add the query to the waiting list
                        self.waiting_queries
                            .entry(new_query.height)
                            .or_default()
                            .push(new_query);

                    } else {
                        info!("Query {:?} is ready for immediate processing.", query_id);
                        let queries_to_process = vec![new_query];

                        if self.config.prover_be_socket_addr.is_some() {
                            self.light_prove_unprocessed_queries(
                                queries_to_process,
                                &mut light_prover_queries
                            ).await;
                        } else {
                            self.stand_alone_prove_unprocessed_query(
                                queries_to_process,
                            ).await?;
                        }
                    }
                },
                Some(block_height) = new_attestation_receiver.recv() => {
                    info!("Received notification for new attestation at height {}", block_height);
                    let mut processable_queries = Vec::new();

                    let keys_to_process: Vec<u64> = self.waiting_queries
                        .keys()
                        .filter(|&&key| key <= block_height)
                        .copied()
                        .collect();

                    for key in keys_to_process {
                        if let Some(queries) = self.waiting_queries.remove(&key) {
                            processable_queries.extend(queries);
                        }
                    }

                    if !processable_queries.is_empty() {
                        info!(
                            "Found {} waiting queries with height <= {}. Processing now.",
                            processable_queries.len(),
                            block_height
                        );

                        if self.config.prover_be_socket_addr.is_some() {
                            self.light_prove_unprocessed_queries(
                                processable_queries,
                                &mut light_prover_queries
                            ).await;
                        } else {
                            self.stand_alone_prove_unprocessed_query(
                                processable_queries,
                            ).await?;
                        }
                    }
                },
                Some(result) = light_prover_queries.next() => {
                    let query_id = match &result {
                        Ok((query, _)) => Some(query.id()),
                        Err(_) => None,
                    };

                    self.handle_finished_light_proving_job(result).await?;

                    // Clean up the query after handling, regardless of the outcome
                    if let Some(id) = query_id {
                        self.cleanup_query(id);
                    }
                }
            }
        }
    }

    async fn stone_proof_query(&self, query: Query) -> Result<()> {
        if self.config.prover_be_socket_addr.is_some() {
            return Err(anyhow!(
                "Tried to prove query locally while in light prover mode."
            ));
        };
        // Create an eth client
        let eth_client = EthClient::new(&self.config.eth_rpc_url, None).await?;

        let r = query::process(eth_client, &query, &self.attestations_cache, true).await?;

        let query_id = query.id();

        if let Either::Left(proof) = r {
            info!("Submitting proof for query: {:?}", query_id);
            match contract::submit_proof(&self.cc3_client, query, proof).await {
                Ok(_) => {
                    info!("Proof submitted successfully for query: {:?}", query_id);
                    Ok(())
                }
                Err(e) => {
                    error!(
                        "Failed to submit proof for query: {:?}, Error: {:?}",
                        query_id, e
                    );
                    Err(e)
                }
            }
        } else {
            Err(anyhow!(
                "Received external proof for query: {:?}, but this is not handled in stone proving mode.",
                query.id()
            ))
        }
    }

    async fn queue_light_proving_jobs(
        &self,
        queries: Vec<Query>,
    ) -> Result<Vec<JoinHandle<(Query, Result<Vec<u8>, LightProvingError>)>>> {
        // Create thread safe versions of config strings
        let prover_be_socket_addr = self.config.prover_be_socket_addr.clone().ok_or(anyhow!(
            "Tried to submit light proving jobs while not in light mode!"
        ))?;
        let be_api_key = self.config.be_api_key.clone().ok_or(anyhow!(
            "We check in main() that be_api_key is always Some if prover_be_socket_addr is Some"
        ))?;
        // Create an eth client
        let eth_client = EthClient::new(&self.config.eth_rpc_url, None).await?;
        let mut proving_job_handles: Vec<JoinHandle<(Query, Result<Vec<u8>, LightProvingError>)>> =
            Vec::new();

        for query in queries {
            let r =
                query::process(eth_client.clone(), &query, &self.attestations_cache, false).await?;

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

    async fn light_prove_unprocessed_queries(
        &mut self,
        mut unprocessed_queries: Vec<Query>,
        light_prover_queries: &mut FuturesUnordered<
            JoinHandle<(Query, Result<Vec<u8>, LightProvingError>)>,
        >,
    ) {
        // We don't want to spam the BE with requests for queries we've already requested.
        unprocessed_queries
            .retain(|query| !self.queued_light_proving_queries.contains(&query.id()));

        info!(
            "Found {} new unprocessed queries",
            unprocessed_queries.len()
        );
        // Save these off to use later without cloning queries
        let query_ids: Vec<H256> = unprocessed_queries.iter().map(Query::id).collect();
        match self.queue_light_proving_jobs(unprocessed_queries).await {
            Ok(new_query_handles) => {
                for query_handle in new_query_handles {
                    light_prover_queries.push(query_handle);
                }
            }
            Err(e) => {
                error!("Queuing light proving for queries failed, Error: {e:?}");
            }
        };
        // All queries were successfully queued as light proving jobs.

        for query_id in query_ids {
            self.queued_light_proving_queries.insert(query_id);
        }
    }

    async fn stand_alone_prove_unprocessed_query(
        &mut self,
        unprocessed_queries: Vec<Query>,
    ) -> Result<()> {
        for query in unprocessed_queries {
            info!("Processing unprocessed query: {:?}", query);
            if let Err(e) = self.stone_proof_query(query.clone()).await {
                error!("Query processing failed, Error: {e:?}");
                // Try to mark a query as invalid, but dont fail if it errors
                if let Err(mark_err) =
                    mark_query_as_invalid(&self.cc3_client, query.id(), e.to_string()).await
                {
                    error!(
                        "Failed to mark query {:?} as invalid: {:?}",
                        query.id(),
                        mark_err
                    );
                }
            }
            // Cleanup the query from the received queries
            self.received_query_ids.remove(&query.id());
        }
        Ok(())
    }

    /// Cleans up the query from the internal memory after processing
    fn cleanup_query(&mut self, query_id: H256) {
        self.queued_light_proving_queries.remove(&query_id);

        self.received_query_ids.remove(&query_id);
    }

    async fn handle_finished_light_proving_job(
        &self,
        result: Result<(Query, Result<Vec<u8>, LightProvingError>), JoinError>,
    ) -> Result<()> {
        match result {
            Ok((query, result_inner)) => {
                match result_inner {
                    Ok(proof) => {
                        info!("Submitting proof for query: {:?}", query);
                        // Prevent unnecessary clone
                        let query_id = query.id();
                        if let Err(e) = contract::submit_proof(&self.cc3_client, query, proof).await
                        {
                            error!(
                                "Failed to submit proof for query: {:?}, Error: {:?}, Most likely verifier failed to verify and reverted",
                                query_id, e
                            );
                            mark_query_as_invalid(&self.cc3_client, query_id, e.to_string())
                                .await?;
                        }
                    }
                    Err(e) => {
                        error!("Query processing failed, Error: {e:?}");
                        if let LightProvingError::ProofGenerationFailed = e {
                            panic!("Query processing failed fatally. Prover BE pipeline is likely rejecting proving jobs due to auth/ip. Fix prover BE then restart.");
                        } else {
                            // Prevent unnecessary clone
                            let query_id = query.id();
                            mark_query_as_invalid(&self.cc3_client, query_id, e.to_string())
                                .await?;
                        }
                    }
                }
                Ok(())
            }
            Err(join_err) => {
                panic!("Fatal error, couldn't join query worker task, error: {join_err:?}");
            }
        }
    }
}
