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
use cc_client::{attestation::CcEvent, Client as CcClient};
use pallet_prover_primitives::Query;

pub mod config;

mod attestation;
mod contract;
pub mod postgres;
mod prom;
mod query;

use crate::{attestation::fragment::Error, prom::ProverMetrics};
use config::Config;

/// `AttestationCacheType` cache type
pub type AttestationCacheType = AttestationCache<H256, AccountId32>;

/// `ChainName` type
pub type ChainName = String;

/// Prover server is configured using `Config`
pub struct Server {
    config: Config,
    cc3_eth_client: EthClient,
    cc3_client: CcClient,
    // Attestation cache
    attestations_cache: AttestationCacheType,
    // Queries that are waiting for attestations
    waiting_queries: BTreeMap<u64, Vec<Query>>,
    // Queries that have been queued for light proving
    queued_light_proving_queries: HashSet<H256>,
    // Queries that have been received
    received_query_ids: HashSet<H256>,
    // Prometheus metrics
    metrics: Option<ProverMetrics>,
    // Chain name
    chain_name: ChainName,
}

impl Server {
    /// Create a new server based on `Config`
    pub async fn new(config: Config) -> Result<Self> {
        let db_pool = postgres::db::get_pool(&config.postgres_uri)?;
        postgres::db::run_migrations(config.postgres_uri.clone()).await?;

        // Create attestations cache
        let attestations_cache: AttestationCacheType = AttestationCache::new(db_pool);

        // Deploy the prover contract
        // This will deploy it on ccnext chain
        let cc3_eth_client =
            EthClient::new(&config.cc3_rpc_url, Some(&config.cc3_evm_private_key)).await?;

        let cc3_client = CcClient::new(&config.cc3_rpc_url, &config.cc3_key).await?;
        let eth_client = Arc::new(EthClient::new(&config.eth_rpc_url, None).await?);
        let chain_id = eth_client.chain_id();

        let supported_chain = cc3_client
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
        info!("✅ Deployed prover contract");

        // Register metrics server if configured
        let metrics = if config.enable_prometheus_metrics {
            let address_str = format!("{}:{}", config.prometheus_host, config.prometheus_port);
            info!(
                "📈 Starting Prometheus metrics server on http://{}/metrics",
                address_str
            );
            prom::start_prom_server(&config)
        } else {
            None
        };

        let chain_name = cc3_client
            .get_chain_name()
            .await
            .unwrap_or_else(|_| "unknown-chain".to_string());

        Ok(Server {
            config,
            cc3_eth_client,
            cc3_client,
            attestations_cache,
            waiting_queries: BTreeMap::new(),
            queued_light_proving_queries: HashSet::new(),
            received_query_ids: HashSet::new(),
            metrics,
            chain_name,
        })
    }

    /// Runs the server in the background, will start following the configured source chain
    pub async fn run(&mut self) -> Result<()> {
        debug!("Created cc3 client");

        let chain_key = self.config.chain_key;

        // Build historical cache
        info!(
            "🛠️  Building historical cache for chain with id: {}, this can take a while...",
            chain_key
        );
        self.attestations_cache
            .build_historical_cache_for_chain(chain_key, &self.cc3_client)
            .await?;
        info!("✅ Built historical cache");

        // Channel for new queries
        let (new_query_sender, new_query_receiver) = mpsc::unbounded_channel::<Query>();

        // Spawn a task to check for queries on contract storage and subscribe to new submissions
        let client_clone = self.cc3_eth_client.clone();
        tokio::spawn(async move {
            contract::provide_unprocessed_queries(&client_clone, new_query_sender.clone()).await
        });

        let (proof_verified_event_sender, proof_verified_event_receiver) =
            mpsc::unbounded_channel::<H256>();

        let proof_client_clone = self.cc3_eth_client.clone();

        // Spawn a task to listen to proof verifications on the contract and report back to the prover operator
        info!("🛰️  Subscribing to proof verification events on the contract");
        tokio::spawn(async move {
            contract::subscribe_proof_verification_events(
                &proof_client_clone,
                proof_verified_event_sender.clone(),
            )
            .await
        });

        self.handle_ongoing_queries_and_fatal_errors(
            new_query_receiver,
            proof_verified_event_receiver,
        )
        .await?;
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    async fn handle_ongoing_queries_and_fatal_errors(
        &mut self,
        mut new_query_receiver: mpsc::UnboundedReceiver<Query>,
        mut proof_event_receiver: mpsc::UnboundedReceiver<H256>,
    ) -> Result<()> {
        let mut light_prover_queries = FuturesUnordered::new();

        let mut subscription = self
            .cc3_client
            .subscribe_events(self.config.chain_key)
            .await?;

        loop {
            tokio::select! {
                Some(event) = subscription.next() => {
                    match event {
                        CcEvent::BlockAttested(attestation) => {
                            // check if the attestation exists in cache
                            if self.attestations_cache
                                .attestation_digest_exists(attestation.digest())
                                .await?
                            {
                                warn!("⚠️ Attestation {:?} already exists in cache, skipping", attestation.digest());
                                continue;
                            }

                            info!(
                                "📝 Received a new attestation: chain: {}, blocknumber: {}, digest({:?})",
                                attestation.chain_key(),
                                attestation.header_number(),
                                attestation.digest()
                            );

                            // Process the attestation
                            let block_height = attestation.header_number();

                            // Update the attestation cache
                            self.attestations_cache.insert_attestation(attestation).await?;

                            debug!("📝 Received notification for new attestation at height {}", block_height);
                            metric_set_labels!(
                                self.metrics,
                                attestation_network_height,
                                self.chain_name,
                                self.config.chain_key,
                                block_height
                            );

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
                                debug!(
                                    "🔍 Found {} waiting queries with height <= {}. Processing now.",
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
                        }
                        CcEvent::CheckpointReached(chain_key, checkpoint) => {
                            // check if exists in cache
                            if self.attestations_cache
                                .checkpoint_digest_exists(checkpoint.digest)
                                .await?
                            {
                                warn!("⚠️ Checkpoint {:?} already exists in cache, skipping", checkpoint.digest);
                                continue;
                            }

                            info!(
                                "📝 Received a new attestation checkpoint: chain: {}, blocknumber: {}, digest({:?})",
                                chain_key,
                                checkpoint.block_number,
                                checkpoint.digest,
                            );

                            self.attestations_cache.insert_checkpoint(checkpoint.clone(), chain_key).await?;
                            self.attestations_cache.mark_cached_up_to(chain_key, checkpoint.digest).await?;
                        }
                        _ => {
                            debug!("⚠️ Received event from Creditcoin client: {:?}", event);
                        }
                    }
                },
                Some(new_query) = new_query_receiver.recv() => {
                    let query_id = new_query.id();

                    if !self.received_query_ids.insert(query_id) {
                        warn!("⚠️ Received duplicate query {:?}, ignoring.", query_id);
                        continue;
                    }

                    info!("📝 Received query {:?}, checking for readiness...", query_id);
                    metric_inc_with_labels!(
                        self.metrics,
                        queries_received,
                        self.chain_name,
                        self.config.chain_key
                    );
                    let maybe_height = self
                        .attestations_cache
                        .last_synced_attestation_block_number(new_query.chain_id)
                        .await?;

                    let Some(last_attestation_height) = maybe_height else {
                        error!(
                        "❌ Failed to get last attestation height from cache. Marking query {:?} as invalid.",
                            query_id
                        );
                        self.mark_query_as_invalid(
                            query_id,
                            "No attestations are synced for this query".to_string(),
                        )
                        .await?;
                        continue;
                    };

                    // Check if the query is ready to be processed
                    if last_attestation_height < new_query.height {
                        info!(
                            "🔄 Query {:?} is not ready. Last attestation: {}, needed: {}. Adding to waiting queue.",
                            query_id, last_attestation_height, new_query.height
                        );

                        // Add the query to the waiting list
                        self.waiting_queries
                            .entry(new_query.height)
                            .or_default()
                            .push(new_query);

                    } else {
                        info!("✅ Query {:?} is ready for immediate processing.", query_id);
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
                },
                Some(query_id) = proof_event_receiver.recv() => {
                    metric_inc_with_labels!(
                        self.metrics,
                        query_proofs_success,
                        self.chain_name,
                        self.config.chain_key
                    );
                    // Log the proof verification event for now. Could also return result segments to the prover if needed
                    info!("🛰️  Proof verification event received for query: {:?}", query_id);
                },
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
            info!("📝 Submitting proof for query: {:?}", query_id);
            contract::submit_proof(&self.cc3_eth_client, query, proof).await?;
            Ok(())
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
                info!("🔄 Handling external proof for query: {:?}", query.id());
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
            "🔍 Found {} new unprocessed queries",
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
                error!("❌ Queuing light proving for queries failed, Error: {e:?}");
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
            info!("🔄 Processing unprocessed query: {:?}", query);
            if let Err(e) = self.stone_proof_query(query.clone()).await {
                error!("❌ Query processing failed, Error: {e:?}");
                // Try to mark a query as invalid, but dont fail if it errors
                let _ = self.mark_query_as_invalid(query.id(), e.to_string()).await;
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
                        info!("📝 Submitting proof for query: {:?}", query);
                        // Prevent unnecessary clone
                        let query_id = query.id();

                        if let Err(e) =
                            contract::submit_proof(&self.cc3_eth_client, query, proof).await
                        {
                            error!(
                                "❌ Failed to submit proof for query: {:?}, Error: {:?}, Most likely verifier failed to verify and reverted",
                                query_id, e
                            );
                            self.mark_query_as_invalid(query_id, e.to_string()).await?;
                        }
                    }
                    Err(e) => {
                        error!("❌ Query processing failed, Error: {e:?}");
                        if let LightProvingError::ProofGenerationFailed = e {
                            panic!("Query processing failed fatally. Prover BE pipeline is likely rejecting proving jobs due to auth/ip. Fix prover BE then restart.");
                        } else {
                            self.mark_query_as_invalid(query.id(), e.to_string())
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

    // Mark a query as invalid on chain with a reason
    // This also increments the failed proofs metric
    async fn mark_query_as_invalid(&self, query_id: H256, reason: String) -> Result<()> {
        metric_inc_with_labels!(
            self.metrics,
            query_proofs_failed,
            self.chain_name,
            self.config.chain_key
        );

        match contract::mark_query_as_invalid(&self.cc3_eth_client, query_id, reason).await {
            Ok(_) => {
                info!("✅ Marked query {:?} as invalid", query_id);
                Ok(())
            }
            Err(e) => {
                error!("❌ Failed to mark query {:?} as invalid: {:?}", query_id, e);
                Err(e)
            }
        }
    }
}
