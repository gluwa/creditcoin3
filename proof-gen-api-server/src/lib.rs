use anyhow::Result;
use config::Config;
use db::{DbManager, ProofsDbEntry};
use sp_core::H256;
use tokio::time::{sleep, Duration};
use tracing::debug;

pub mod config;
pub mod db;

pub struct Server {
    // proof-gen-api-server is configured using `Config`
    _config: Config,
    // The db manager, which owns a connection thread pool
    db_manager: DbManager,
}

impl Server {
    /// Create a new server based on `Config`
    pub async fn new(config: Config, db_manager: DbManager) -> Result<Self> {
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

        Ok(Server {
            _config: config,
            db_manager,
        })
    }

    /// Runs the server in the background, will start following the configured source chain
    pub async fn run(&mut self) -> Result<()> {
        debug!("Running proof-gen-api-server!");
        self.db_manager.run_migrations().await?;

        // TODO: Remove this
        self.db_manager.create_example_table().await?;

        let mock_entry = ProofsDbEntry {
            chain_key: 1,
            header_number: 1,
            tx_index: Some(1),
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
            self.db_manager.get_proof().await?;
            // Wait a bit to avoid spam
            println!("Waiting...");
            sleep(Duration::from_secs(20)).await;
        }
    }
}
