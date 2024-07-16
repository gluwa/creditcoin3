use anyhow::Result;
use attestation_cache::AttestationCache;
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

/// `AttestationCacheType` cache type
pub type AttestationCacheType = Arc<AttestationCache<H256, AccountId32>>;

/// `CcClientArc` type
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
            Arc::new(attestation_cache::AttestationCache::new(db_pool));

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
        )
        .await?;
        debug!("Creating cc3 client");
        cc3_client.init().await?;

        // Sync chain prices configuration
        cc3_client
            .sync_chain_prices_configuration(self.config.chain_price_configurations.chain.clone())
            .await?;

        let attestations_cache = self.attestations_cache.clone();
        let cc3_client = Arc::new(cc3_client);

        // Build historical cache first before starting the subscription for new attestations & claims
        let config = self.config.clone();
        // let attestations_cache = attestations_cache.clone();
        info!("Starting sync cache");
        attestation_cache::build_historical_cache(config, &attestations_cache, &cc3_client).await?;

        // Sync the cache
        info!("Starting cache live sync");
        let sync_attestations_cache = attestations_cache.clone();
        let sync_cc3_client = cc3_client.clone();
        let sync_config = self.config.clone();
        tokio::spawn(async move {
            attestation_cache::sync_cache(&sync_config, &sync_attestations_cache, &sync_cc3_client)
                .await
                .expect("Failed to sync cache");
        });

        // Handle claim subscription
        info!("Starting claim subscription");
        let claim_attestations_cache = attestations_cache.clone();
        let claim_cc3_client = cc3_client.clone();
        let claim_config = self.config.clone();
        tokio::spawn(async move {
            handle_claim_sub(&claim_config, &claim_cc3_client, &claim_attestations_cache)
                .await
                .expect("Failed to handle claim sub");
        });

        Ok(())
    }
}

pub async fn handle_claim_sub(
    config: &Config,
    cc3_client: &CcClientArc,
    _attestation_cache: &AttestationCacheType,
) -> Result<()> {
    let (claim_tx, mut claim_rx) = mpsc::channel(config.claim_buffer.into());
    debug!("Created claim buffer with size: {}", config.claim_buffer);

    // Run sub in background and allow server to continue doing other work
    let client = Arc::clone(cc3_client);
    let claim_sub_handle = tokio::spawn(async move { client.start_claim_sub(claim_tx).await });

    debug!("Starting claim processing handler");

    // Handle claims in the main task or another spawned task
    let client = Arc::clone(cc3_client);
    let chain_price_configurations = config.chain_price_configurations.clone();
    while let Some(claim) = claim_rx.recv().await {
        // Get the rpc url for the chain the claim is from
        let eth_client_rpc_url = chain_price_configurations
            .get_rpc_url(claim.claim.chain_id)
            .ok_or_else(|| anyhow::anyhow!("Chain not found"))
            .unwrap_or_else(|_| panic!("Chain with id {} not found", claim.claim.chain_id));

        // Create an eth client
        let eth_client = eth::Client::new(eth_client_rpc_url).await?;

        // Process the claim
        process_claim(client.clone(), eth_client, claim).await?;
    }

    // Wait for the claim subscription task to finish and handle its result
    claim_sub_handle.await??;

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
