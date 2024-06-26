use anyhow::Result;
use attestation_cache::AttestationCache;
use attestor_primitives::ChainId;
use cc_client::AccountId32;
use eth::Client;
use sp_core::H256;
use std::sync::Arc;
use tokio::sync::mpsc;
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

type CcClientArc = Arc<cc3::Client>;

/// Prover server is configured using `Config`
pub struct Server {
    #[allow(dead_code)]
    config: Config,
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

        let attestations_cache = Arc::new(self.attestations_cache.clone());
        let cc3_client = Arc::new(cc3_client);

        // Sync the cache
        let config = self.config.clone();
        let cc3_client_clone = Arc::clone(&cc3_client);
        info!("Starting sync cache");
        tokio::spawn(async move {
            let _ = sync_cache(config, attestations_cache, &cc3_client_clone).await;
        });

        // Handle claim subscription
        // This will run in the background
        // The server will continue to run and do other work
        let config = self.config.clone();
        let cc3_client_clone = Arc::clone(&cc3_client);
        info!("Starting claim sub");
        tokio::spawn(async move {
            let _ = handle_claim_sub(&config, &cc3_client_clone).await;
        });

        Ok(())
    }
}

pub async fn handle_claim_sub(config: &Config, cc3_client: &CcClientArc) -> Result<()> {
    let (claim_tx, mut claim_rx) = mpsc::channel(config.claim_buffer.into());
    debug!("Created claim buffer with size: {}", config.claim_buffer);

    // Run sub in background and allow server to continue doing other work
    let client = Arc::clone(cc3_client);
    tokio::spawn(async move {
        let _ = client.start_claim_sub(claim_tx).await;
    });

    debug!("Starting claim processing handler");

    // Handle claims in the main task or another spawned task
    let client = Arc::clone(cc3_client);
    let chain_price_configurations = config.chain_price_configurations.clone();
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

    Ok(())
}

async fn sync_cache(
    config: Config,
    attestations_cache: Arc<AttestationCacheType>,
    cc3_client: &CcClientArc,
) -> Result<()> {
    let chains: Vec<u64> = config
        .chain_price_configurations
        .chain
        .iter()
        .map(|c| c.chain_id)
        .collect();

    // First populate historical attestations
    let futures = chains.clone().into_iter().map(|chain| {
        build_historical_cache_for_chain(chain, attestations_cache.clone(), Arc::clone(cc3_client))
    });

    let _ = futures::future::join_all(futures).await;

    info!("Historical attestations caches built");

    // Start subscription for new attestations
    let (attestation_tx, mut attestation_rx) = mpsc::channel(config.claim_buffer.into());
    debug!(
        "Created attestation buffer with size: {}",
        config.claim_buffer
    );

    // Run sub in background and allow server to continue doing other work
    let client = cc3_client.clone();
    tokio::spawn(async move {
        let _ = client.start_attestation_sub(attestation_tx, chains).await;
    });

    // Handle attestations in the main task or another spawned task
    //
    // This will run in the background
    // The server will continue to run and do other work
    let attestations_cache = attestations_cache.clone();
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

pub async fn build_historical_cache_for_chain(
    chain: ChainId,
    attestations_cache: Arc<AttestationCacheType>,
    cc3_client: CcClientArc,
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

pub async fn process_claim(client: CcClientArc, eth_client: Client, claim: Claim) -> Result<()> {
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
