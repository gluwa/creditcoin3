use anyhow::Result;
use kameo::spawn;
use tokio::sync::oneshot;

pub mod cc3;
pub mod eth;
pub mod transaction;

#[derive(Debug)]
/// Attestor server is configured using `Config`
pub struct Server {
    #[allow(dead_code)]
    config: Config,
    // Channel to send cancellation to the claim subscription
    // will exit when this is dropped
    _cancel_tx: Option<oneshot::Sender<()>>,
}

#[derive(Debug, Clone)]
/// Server configuration
/// - `eth_rpc_url`: Source chain RPC url
/// - `cc3_rpc_url`: Creditcoin RPC url (must have rpc + websocket features)
/// - `cc3_key`: Mnemonic for a creditcoin3 account
pub struct Config {
    pub eth_rpc_url: String,
    pub cc3_rpc_url: String,
    pub cc3_key: String,
    pub nickname: String,
}

impl Server {
    /// Create a new server based on `Config`
    #[must_use]
    pub fn new(config: Config) -> Self {
        Server {
            config,
            _cancel_tx: None,
        }
    }

    /// Runs the server in the background, will start following the configured source chain
    pub async fn run(&mut self) -> Result<()> {
        let (_cancel_tx, cancel_rx) = oneshot::channel::<()>();
        self._cancel_tx = Some(_cancel_tx);

        let cc3_client = cc3::Client::new(
            &self.config.cc3_rpc_url,
            &self.config.cc3_key,
            &self.config.nickname,
        )?;
        cc3_client.init().await?;

        tokio::spawn(async move {
            let _ = cc3_client.start_claim_sub(cancel_rx).await;
            let _cc3_client = spawn(cc3_client);
        });

        Ok(())
    }
}
