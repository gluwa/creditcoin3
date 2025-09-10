use anyhow::{anyhow, Result};
use cc_client::AccountId32;
use either::Either;
use eth::Client as EthClient;
use futures::stream::{FuturesUnordered, StreamExt};
use query::external::Error as LightProvingError;
use sp_core::H256;
use std::collections::{BTreeMap, HashSet};
use tokio::{
    sync::mpsc,
    task::{self, JoinError, JoinHandle},
};
use tracing::{debug, error, info, warn};

use attestation::cache::AttestationCache;
use attestor_primitives::{AttestationCheckpoint, SignedAttestation};
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

/// Type for managing light proving queries
type LightProverQueries = FuturesUnordered<JoinHandle<(Query, Result<Vec<u8>, LightProvingError>)>>;

/// `ChainName` type
pub type ChainName = String;

/// Prover server is configured using `Config`
pub struct Server {
    config: Config,
    // Ethereum client for the cc3 chain where the prover contract is deployed
    cc3_eth_client: EthClient,
    // Creditcoin client for the cc3 chain where the prover contract is deployed
    cc3_client: CcClient,
    // Ethereum client for the source chain
    source_chain_eth_client: EthClient,
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
    fn is_light_prover_mode(&self) -> bool {
        self.config.prover_be_socket_addr.is_some()
    }
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

        let supported_chain = cc3_client
            .get_supported_chain(config.chain_key)
            .await?
            .ok_or(Error::FailedToGetChainKey)?;

        // Check that the source chain id matches the configured chain id
        // This is to prevent misconfiguration
        let source_chain_eth_client = EthClient::new(&config.eth_rpc_url, None).await?;
        let chain_id = source_chain_eth_client.chain_id();
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
        info!(
            "✅ Connected to Creditcoin chain: {} with id: {}",
            chain_name, config.chain_key
        );

        Ok(Server {
            config,
            cc3_eth_client,
            cc3_client,
            source_chain_eth_client,
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
        let (new_query_sender, mut new_query_receiver) = mpsc::unbounded_channel::<Query>();

        // Spawn a task to check for queries on contract storage and subscribe to new submissions
        let client_clone = self.cc3_eth_client.clone();
        tokio::spawn(async move {
            contract::provide_unprocessed_queries(&client_clone, new_query_sender.clone()).await
        });

        let (proof_verified_event_sender, mut proof_verified_event_receiver) =
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

        let mut subscription = self
            .cc3_client
            .subscribe_events(self.config.chain_key)
            .await?;

        let mut light_prover_queries = FuturesUnordered::new();
        let (queries_to_process_sender, mut queries_to_process_receiver) =
            mpsc::unbounded_channel::<Query>();

        loop {
            tokio::select! {
                Some(event) = subscription.next() => {
                    match event {
                        CcEvent::BlockAttested(attestation) => {
                            self.handle_block_attested(attestation, queries_to_process_sender.clone()).await?;
                        }
                        CcEvent::CheckpointReached(chain_key, checkpoint) => {
                            self.handle_checkpoint_reached(chain_key, checkpoint).await?;
                        }
                        _ => {
                            debug!("❕Received event from Creditcoin client: {:?}", event);
                        }
                    }
                },
                Some(new_query) = new_query_receiver.recv() => {
                    self.handle_new_query(new_query, queries_to_process_sender.clone()).await?;
                },
                Some(query) = queries_to_process_receiver.recv() => {
                    let query_id = query.id();
                    if let Err(e) = self.handle_query_to_process(query, &mut light_prover_queries).await {
                        error!("❌ Failed to process query: {:?}, Error: {:?}", query_id, e);
                        self.mark_query_as_invalid(query_id, e.to_string()).await?;
                    }
                },
                Some(result) = light_prover_queries.next() => {
                    self.handle_light_prover_result(result).await?;
                }
                Some(query_id) = proof_verified_event_receiver.recv() => {
                    // Only increment success metric if we have seen this query before
                    // This event can fire multiple times for the same query
                    if self.received_query_ids.contains(&query_id) {
                        metric_inc_with_labels!(
                            self.metrics,
                            query_proofs_success,
                            self.chain_name,
                            self.config.chain_key
                        );
                    }

                    // Log the proof verification event for now. Could also return result segments to the prover if needed
                    info!("🛰️ Proof verification event received for query: {:?}", query_id);
                    self.cleanup_query(query_id);
                },
            }
        }
    }

    /// Handles a new attested block, updating the cache and sending any waiting queries for processing
    /// to the processing channel
    async fn handle_block_attested(
        &mut self,
        attestation: SignedAttestation<H256, AccountId32>,
        query_sender: mpsc::UnboundedSender<Query>,
    ) -> Result<()> {
        // check if the attestation exists in cache
        if self
            .attestations_cache
            .attestation_digest_exists(attestation.digest())
            .await?
        {
            warn!(
                "⚠️ Attestation {:?} already exists in cache, skipping",
                attestation.digest()
            );
            return Ok(());
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
        self.attestations_cache
            .insert_attestation(attestation)
            .await?;

        debug!(
            "📝 Received notification for new attestation at height {}",
            block_height
        );
        metric_set_labels!(
            self.metrics,
            attestation_network_height,
            self.chain_name,
            self.config.chain_key,
            block_height
        );

        let mut processable_queries = Vec::new();

        let keys_to_process: Vec<u64> = self
            .waiting_queries
            .keys()
            .filter(|&&key| key <= block_height)
            .copied()
            .collect();

        for key in keys_to_process {
            if let Some(queries) = self.waiting_queries.remove(&key) {
                processable_queries.extend(queries);
            }
        }

        if processable_queries.is_empty() {
            debug!(
                "🔍 No waiting queries with height <= {} found.",
                block_height
            );
            return Ok(());
        }

        debug!(
            "🔍 Found {} waiting queries with height <= {}. Processing now.",
            processable_queries.len(),
            block_height
        );

        for processable_query in processable_queries {
            debug!("🔍 Waiting query to process: {:?}", processable_query);
            query_sender.send(processable_query)?;
        }

        Ok(())
    }

    async fn handle_checkpoint_reached(
        &mut self,
        chain_key: u64,
        checkpoint: AttestationCheckpoint,
    ) -> Result<()> {
        // check if exists in cache
        if self
            .attestations_cache
            .checkpoint_digest_exists(checkpoint.digest)
            .await?
        {
            warn!(
                "⚠️ Checkpoint {:?} already exists in cache, skipping",
                checkpoint.digest
            );
            return Ok(());
        }

        info!(
            "📝 Received a new attestation checkpoint: chain: {}, blocknumber: {}, digest({:?})",
            chain_key, checkpoint.block_number, checkpoint.digest,
        );

        self.attestations_cache
            .insert_checkpoint(checkpoint.clone(), chain_key)
            .await?;
        self.attestations_cache
            .mark_cached_up_to(chain_key, checkpoint.digest)
            .await?;

        Ok(())
    }

    /// Handles a new query, checking if it's ready to be processed or needs to wait
    /// If ready, sends it to the processing channel
    /// If not, adds it to the waiting list
    async fn handle_new_query(
        &mut self,
        query: Query,
        query_sender: mpsc::UnboundedSender<Query>,
    ) -> Result<()> {
        let query_id = query.id();

        if !self.received_query_ids.insert(query_id) {
            warn!("⚠️ Received duplicate query {:?}, ignoring.", query_id);
            return Ok(());
        }

        info!(
            "📝 Received a new query {:?}, checking if we can process it now.",
            query_id
        );
        metric_inc_with_labels!(
            self.metrics,
            queries_received,
            self.chain_name,
            self.config.chain_key
        );

        // Check if the query is ready to be processed
        // This is a synchronous check, the async version is in handle_block_attested
        // and is used when we receive a new attestation
        let maybe_height = self
            .attestations_cache
            .last_synced_attestation_block_number(query.chain_id)
            .await?;

        let Some(last_attestation_height) = maybe_height else {
            error!(
                "❌ 0 Attestations synced for query chain id: {:?}. This shouldn't happen except with a brand new chain. Marking query {:?} as invalid.",
                query.chain_id,
                query_id
            );
            return self
                .mark_query_as_invalid(
                    query_id,
                    "No attestations are synced for this query".to_string(),
                )
                .await;
        };

        if last_attestation_height < query.height {
            info!(
                "🔄 Query {:?} is not ready. Last attestation: {}, needed: {}. Adding to waiting queue.",
                query_id, last_attestation_height, query.height
            );

            // Add the query to the waiting list
            self.waiting_queries
                .entry(query.height)
                .or_default()
                .push(query);
        } else {
            info!("🕛 Query {:?} is ready for immediate processing.", query_id);
            query_sender.send(query)?;
        }

        Ok(())
    }

    /// Handles a query that is ready to be processed
    /// If in light prover mode, sends it to the light prover job queue
    /// If not, processes it immediately and submits the proof on chain
    async fn handle_query_to_process(
        &mut self,
        query: Query,
        light_prover_queries: &mut LightProverQueries,
    ) -> Result<()> {
        info!("🏗️ Processing query: {:?}", query.id());
        let r = query::process(
            &self.source_chain_eth_client,
            &query,
            &self.attestations_cache,
            !self.is_light_prover_mode(),
        )
        .await?;

        let query_id = query.id();

        if let Either::Left(proof) = r {
            info!("📝 Submitting proof for query: {:?}", query_id);
            let _ = contract::submit_proof(&self.cc3_eth_client, query, proof).await?;
            return Ok(());
        } else if let Either::Right(stone_proof_public_input) = r {
            info!("🔄 Handling external proof for query: {:?}", query_id);
            // Cloning handles for config strings
            let addr = self.config.prover_be_socket_addr.clone().ok_or(anyhow!(
                "Tried to submit light proving jobs while not in light mode!"
            ))?;
            let key = self.config.be_api_key.clone().ok_or(anyhow!(
                "We check in main() that be_api_key is always Some if prover_be_socket_addr is Some"
            ))?;
            light_prover_queries.push(task::spawn(async move {
                let proving_result = query::external::handle_proof_order(
                    query_id,
                    stone_proof_public_input,
                    addr.as_ref(),
                    key.as_ref(),
                )
                .await;
                (query, proving_result)
            }));
        }

        Ok(())
    }

    /// Handles the result of a light prover job
    /// If successful, submits the proof on chain
    /// If failed, marks the query as invalid on chain
    /// If the task failed, panics as this is a fatal error
    async fn handle_light_prover_result(
        &mut self,
        result: Result<(Query, Result<Vec<u8>, LightProvingError>), JoinError>,
    ) -> Result<()> {
        let (query, inner_result) = result?;
        let query_id = query.id();

        match inner_result {
            Ok(proof) => {
                info!("📝 Submitting proof for query: {:?}", query_id);
                if let Err(e) = contract::submit_proof(&self.cc3_eth_client, query, proof).await {
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
                    self.mark_query_as_invalid(query_id, e.to_string()).await?;
                }
            }
        }

        Ok(())
    }

    /// Cleans up the query from the internal memory after processing
    fn cleanup_query(&mut self, query_id: H256) {
        self.queued_light_proving_queries.remove(&query_id);
        self.received_query_ids.remove(&query_id);
    }

    /// Marks a query as invalid on chain with the given reason
    /// Increments the failed proofs metric
    /// Cleans up the query from internal memory
    async fn mark_query_as_invalid(&mut self, query_id: H256, reason: String) -> Result<()> {
        metric_inc_with_labels!(
            self.metrics,
            query_proofs_failed,
            self.chain_name,
            self.config.chain_key
        );

        // Clean up the query from internal memory
        self.cleanup_query(query_id);

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
