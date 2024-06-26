use anyhow::Result;
use attestation_cache::AttestationCache;
use attestor_primitives::ChainId;
use cc_client::AccountId32;
use eth::Client;
use sp_core::H256;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use tokio::{fs::File, io::AsyncReadExt};
use tracing::{debug, error, info, warn};

pub mod attestation_cache;
pub mod cc3;
pub mod claim;
pub mod config;
pub mod postgres;

use cc3::Claim;
use config::Config;

pub type AttestationCacheType = AttestationCache<H256, AccountId32>;

/// Prover server is configured using `Config`
pub struct Server {
    #[allow(dead_code)]
    config: Config,
    // Channel to send cancellation to the claim subscription
    // will exit when this is dropped
    cancel_tx: Option<oneshot::Sender<()>>,
    // Channel to send cancellation to the attestation subscription
    cancel_attestation_tx: Option<oneshot::Sender<()>>,
    // Attestation cache
    attestations_cache: AttestationCacheType,
}

impl Server {
    /// Create a new server based on `Config`
    pub fn new(config: Config) -> Result<Self> {
        let db_pool = postgres::db::get_pool(&config.postgres_uri)?;
        let attestations_cache: AttestationCacheType =
            attestation_cache::AttestationCache::new(db_pool);

        Ok(Server {
            config,
            cancel_tx: None,
            cancel_attestation_tx: None,
            attestations_cache,
        })
    }

    /// Runs the server in the background, will start following the configured source chain
    pub async fn run(&mut self) -> Result<()> {
        let cc3_client = cc3::Client::new(
            &self.config.cc3_rpc_url,
            &self.config.cc3_key,
            &self.config.nickname,
        )?;
        debug!("Creating cc3 client");
        cc3_client.init().await?;

        // Sync chain prices configuration
        cc3_client
            .sync_chain_prices_configuration(self.config.chain_price_configurations.chain.clone())
            .await?;

        // Sync the cache
        self.sync_cache(&cc3_client).await?;

        // Handle claim subscription
        // This will run in the background
        // The server will continue to run and do other work
        self.handle_claim_sub(&cc3_client)?;

        Ok(())
    }

    pub fn handle_claim_sub(&mut self, cc3_client: &cc3::Client) -> Result<()> {
        let (claim_tx, mut claim_rx) = mpsc::channel(self.config.claim_buffer.into());
        debug!(
            "Created claim buffer with size: {}",
            self.config.claim_buffer
        );

        debug!("Starting claim sub on cc3");
        // Create a channel to cancel the claim subscription
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        // Store the cancel_tx so we can cancel the subscription later
        self.cancel_tx = Some(cancel_tx);
        // Run sub in background and allow server to continue doing other work
        let client = cc3_client.clone();
        tokio::spawn(async move {
            let _ = client.start_claim_sub(cancel_rx, claim_tx).await;
        });

        debug!("Starting claim processing handler");
        let chain_price_configurations = self.config.chain_price_configurations.clone();
        // Handle claims in the main task or another spawned task
        let client = cc3_client.clone();
        tokio::spawn(async move {
            while let Some(claim) = claim_rx.recv().await {
                // Get the rpc url for the chain the claim is from
                let eth_client_rpc_url = chain_price_configurations
                    .get_rpc_url(claim.claim.chain_id)
                    .ok_or(anyhow::anyhow!("Chain not found"))
                    .unwrap_or_else(|_| panic!("Chain with id {} not found", claim.claim.chain_id));

                // Create an eth client
                let eth_client = eth::Client::new(eth_client_rpc_url)
                    .await
                    .expect("Error creating eth client");

                // Process the claim
                match process_claim(client.clone(), eth_client, claim).await {
                    Ok(()) => {
                        info!("Claim processed");
                    }
                    Err(e) => {
                        panic!("Error processing claim: {e}, unwinding server..")
                    }
                }
            }
        });

        Ok(())
    }

    async fn sync_cache(&mut self, cc3_client: &cc3::Client) -> Result<()> {
        let chains: Vec<u64> = self
            .config
            .chain_price_configurations
            .chain
            .iter()
            .map(|c| c.chain_id)
            .collect();

        // First populate historical attestations
        let futures = chains.clone().into_iter().map(|chain| {
            let attestation_cache = std::sync::Arc::new(self.attestations_cache.clone());
            let cc3_client = cc3_client.clone();
            build_historical_cache_for_chain(chain, attestation_cache, cc3_client)
        });

        let _ = futures::future::join_all(futures).await;

        info!("Historical attestations caches built");

        // Start subscription for new attestations
        let (attestation_tx, mut attestation_rx) = mpsc::channel(self.config.claim_buffer.into());
        debug!(
            "Created attestation buffer with size: {}",
            self.config.claim_buffer
        );

        info!("Subscribing to attestations on cc3 now...");
        // Create a channel to cancel the claim subscription
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        // Store the cancel_tx so we can cancel the subscription later
        self.cancel_attestation_tx = Some(cancel_tx);
        // Run sub in background and allow server to continue doing other work
        let client = cc3_client.clone();
        tokio::spawn(async move {
            let _ = client
                .start_attestation_sub(cancel_rx, attestation_tx, chains)
                .await;
        });

        // Handle attestations in the main task or another spawned task
        //
        // This will run in the background
        // The server will continue to run and do other work
        let attestations_cache = self.attestations_cache.clone();
        tokio::spawn(async move {
            while let Some(attestation) = attestation_rx.recv().await {
                // check if exists in cache
                if attestations_cache
                    .digest_exists(attestation.digest())
                    .await
                    .expect("Error checking if attestation exists in cache")
                {
                    warn!("Attestation already exists in cache, skipping");
                    continue;
                }

                attestations_cache
                    .insert(attestation)
                    .await
                    .expect("Error inserting attestation");
            }
        });

        Ok(())
    }
}

pub async fn build_historical_cache_for_chain(
    chain: ChainId,
    attestations_cache: Arc<AttestationCacheType>,
    cc3_client: cc3::Client,
) -> Result<()> {
    info!("Building historical cache for chain: {}", chain);
    let last_digest = cc3_client.fetch_last_digest(chain).await?;

    if last_digest.is_none() {
        error!("No historical attestations found for chain: {}", chain);
        return Ok(());
    }

    // Get the last attestation
    let mut last_chain_attestation = cc3_client
        .get_attestation_by_digest(chain, last_digest.unwrap())
        .await
        .map_err(|e| anyhow::anyhow!("Error fetching last attestation: {:?}", e))?
        .ok_or_else(|| anyhow::anyhow!("Last attestation not found"))?;

    // Check if the first digest exists (one with prev_digest = Null) (meaning the front of the chain)
    let head_of_chain_exists = attestations_cache.first_digest_exists(chain).await?;

    // Fetch the last synced attestation from the cache
    let last_attestation_synced_in_cache =
        attestations_cache.last_synced_attestation(chain).await?;

    if !head_of_chain_exists && last_attestation_synced_in_cache.is_some() {
        let digest = H256::from_slice(
            &hex::decode(
                last_attestation_synced_in_cache
                    .unwrap()
                    .prev_digest
                    .unwrap(),
            )
            .expect("Error decoding digest"),
        );
        info!("Head of chain not found in cache, but last attestation found in cache, starting to sync from: {}", digest);

        // fetch last attestation from cache
        last_chain_attestation = cc3_client
            .get_attestation_by_digest(chain, digest)
            .await
            .map_err(|e| anyhow::anyhow!("Error fetching last attestation: {:?}", e))?
            .ok_or_else(|| anyhow::anyhow!("Last attestation not found"))?;
    }

    let mut fetch_more = true;
    // Fetch more historical attestations
    while fetch_more {
        let digest = last_chain_attestation.attestation.digest();

        // Check if the digest already exists in the cache
        let exists_in_cache = attestations_cache.digest_exists(digest).await?;
        info!(
            "Checking if digest {} exists in cache: {}",
            digest, exists_in_cache
        );

        // Check if the digest already exists in the cache and the first digest exists
        // If this digest exists in the cache and the first digest exists, we can stop fetching more
        // because it means we have fetched all the historical attestations
        if exists_in_cache && head_of_chain_exists {
            info!(
                "Digest {} already exists in cache, skipping insertion",
                digest
            );
            fetch_more = false;
        };

        if !exists_in_cache {
            // Insert the attestation into the cache
            info!(
                "Inserting attestation with digest({}) for chain: {}, blocknumber: {} into cache",
                digest,
                last_chain_attestation.chain_id(),
                last_chain_attestation.header_number(),
            );
            attestations_cache
                .insert(last_chain_attestation.clone())
                .await?;
        }

        // Fetch the next attestation
        if let Some(prev_digest) = last_chain_attestation.attestation.prev_digest {
            info!("Fetching attestation with prev_digest: {}", prev_digest);
            last_chain_attestation = cc3_client
                .get_attestation_by_digest(chain, prev_digest)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Last attestation not found"))?;
        } else {
            info!("Reached the front of the chain, stopping fetching more historical attestations");
            fetch_more = false;
        }
    }

    info!("Finished building historical cache for chain: {}", chain);

    Ok(())
}

pub async fn process_claim(
    client: crate::cc3::Client,
    eth_client: Client,
    claim: Claim,
) -> Result<()> {
    info!("Processing claim with hash: {:?}", claim.hash);

    // Check if claim exists on source chain
    match claim::check_claim_inclusion(eth_client, claim.claim).await {
        Ok(true) => {
            info!("Claim included on source chain");
        }
        Ok(false) => {
            warn!("Claim not included on source chain");
        }
        Err(e) => {
            error!("Error checking claim inclusion: {:?}", e);
        }
    };

    // Create proof (TODO: hook up prover)
    let mut proof_example = File::open("proof_example.json").await?;

    // Create a buffer to read the file
    let mut proof = Vec::new();

    // read the whole proof file into the buffer
    proof_example.read_to_end(&mut proof).await?;

    // Submit result to cc3
    client.submit_proof(claim.hash, proof).await?;

    Ok(())
}
