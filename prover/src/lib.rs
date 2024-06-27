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
            attestation_cache::sync_cache(config, attestations_cache, &cc3_client_clone)
                .await
                .expect("Failed to sync cache");
        });

        // Handle claim subscription
        let config = self.config.clone();
        let cc3_client_clone = Arc::clone(&cc3_client);
        info!("Starting claim sub");
        tokio::spawn(async move {
            handle_claim_sub(&config, &cc3_client_clone)
                .await
                .expect("Failed to handle claim sub");
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
