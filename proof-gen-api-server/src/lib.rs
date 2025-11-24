use anyhow::{anyhow, Result};
use config::Config;
use db::DbManager;
use networking::run_http_server;
use std::sync::Arc;
use tokio::signal;
use tokio::{select, sync::oneshot::channel};
use tracing::{error, info};

pub mod config;
pub mod db;
pub mod networking;
mod prom;
pub mod services;

// Re-exports for integration tests and external callers
pub use networking::build_app;
pub use services::continuity_service::ContinuityService;
pub use services::mock_providers;

pub struct Server {
    config: Config,
    db_manager: DbManager,
}

impl Server {
    pub async fn new(config: Config, db_manager: DbManager) -> Result<Self> {
        Ok(Server { config, db_manager })
    }

    pub async fn run(&self) -> Result<()> {
        // Production guard: disallow mock providers when running with production log profile BEFORE touching the DB.
        if self.config.use_mock_providers
            && std::env::var("RUST_LOG")
                .map(|v| v.to_ascii_lowercase().contains("production"))
                .unwrap_or(false)
        {
            return Err(anyhow!(
                "Refusing to start with mock providers in production"
            ));
        }

        // Run migrations (only after passing guard)
        self.db_manager.run_migrations().await?;

        // Continuity builder configuration
        let continuity_config = continuity::ContinuityConfig {
            cc3_rpc_url: self.config.cc3_rpc_url.clone(),
            eth_rpc_url: self.config.eth_rpc_url.clone(),
            chain_key: self.config.chain_key,
        };
        let use_mocks = std::env::var("USE_MOCK_PROVIDERS")
            .ok()
            .map(|v| v == "1")
            .unwrap_or(false);
        let builder = if use_mocks {
            let (cc_mock, eth_mock) =
                services::mock_providers::make_mock_providers(self.config.chain_key);
            continuity::ContinuityBuilder::new_with_providers(continuity_config, cc_mock, eth_mock)
        } else {
            continuity::ContinuityBuilder::new(continuity_config).await?
        };

        let service = Arc::new(services::continuity_service::ContinuityService::new(
            Arc::new(builder),
            Arc::new(self.db_manager.clone()),
        ));

        // Build axum application
        let app = build_app(service);
        let (http_shutdown_tx, http_shutdown_rx) = channel::<()>();
        let server_future = run_http_server(app, &self.config.bind_addr, http_shutdown_rx);
        tokio::pin!(server_future);

        info!("Server listening on {}", self.config.bind_addr);

        select! {
            res = &mut server_future => {
                if let Err(err) = res {
                    error!("HTTP server exited with error: {err}");
                    return Err(anyhow!("API HTTP server exited"));
                }
                // Normal exit (unexpected) – treat as error for now
               Err(anyhow!("HTTP server exited unexpectedly"))
            }
            _ = shutdown_signal() => {
                let _ = http_shutdown_tx.send(());
                info!("Shutdown signal received – stopping server");
                Ok(())
            }
        }
    }
}

pub async fn shutdown_signal() {
    // Ctrl+C
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm =
            signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
        sigterm.recv().await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }

    info!("Shutdown signal received");
}
