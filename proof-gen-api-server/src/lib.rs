use anyhow::Result;
use app::build_app;
use config::Config;
use db::{DbManager, QueryProofs};
use sp_core::H256;
use std::net::SocketAddr;
use tokio::time::{sleep, Duration};
use tracing::debug;
use tracing::{error, info};

pub mod app;
pub mod config;
pub mod db;
pub mod routes;
pub mod services;

pub struct Server {
    // proof-gen-api-server is configured using `Config`
    config: Config,
    // The db manager, which owns a connection thread pool
    db_manager: DbManager,
}

impl Server {
    /// Create a new server based on `Config`
    pub async fn new(config: Config, db_manager: DbManager) -> Result<Self> {
        // TODO: Use these config fields once the networking side of the server is merged

        /*let cc3_client = CcClient::new(&config.cc3_rpc_url, &config.cc3_key).await?;

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
        );*/

        Ok(Server { config, db_manager })
    }

    /// Runs the server in the background, will start following the configured source chain
    pub async fn run(&mut self) -> Result<()> {
        debug!("Running proof-gen-api-server!");
        self.db_manager.run_migrations().await?;

        // Bind address (default in config.rs = 0.0.0.0:3100)
        let addr: SocketAddr = self
            .config
            .bind_addr
            .parse()
            .expect("Invalid BIND_ADDR format");

        // Build app
        let app = build_app();

        // Start server
        info!("🚀 Starting Continuity Proof API Server on http://{addr}");

        // Spawn networking task
        tokio::spawn(async move {
            if let Err(err) =
                axum::serve(tokio::net::TcpListener::bind(addr).await.unwrap(), app).await
            {
                error!("❌ Server error: {}", err);
            }
            // TODO: Set up channel for messages from newtorking or similar
        });

        let mock_entry = QueryProofs {
            chain_key: 1,
            header_number: 1,
            tx_index: None,
            tx_hash: Some(H256::zero()),
            continuity_proof: None,
            merkle_proof: None,
            merkle_root: Some(H256::zero()),
        };

        // This should really be a select! loop
        loop {
            // Test insert
            self.db_manager.insert_proofs_entry(mock_entry.clone());
            // Test read
            info!("Waiting on insert before read...");
            sleep(Duration::from_secs(1)).await;
            let maybe_entry = self.db_manager.get_proofs_entry(1, 1).await?;
            info!("Entry: {maybe_entry:?}");
            // Wait a bit to avoid spam
            info!("Waiting...");
            sleep(Duration::from_secs(20)).await;
        }
    }
}
