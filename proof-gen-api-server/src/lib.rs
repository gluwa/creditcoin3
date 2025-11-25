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
        // Production guard: Only trigger when RUST_LOG explicitly set to "production" / "prod" (case-insensitive).
        // Avoid substring matches that could falsely trigger (e.g. "reproduction_steps=trace").
        let is_prod_log = std::env::var("RUST_LOG")
            .ok()
            .map(|v| {
                let v = v.trim().to_ascii_lowercase();
                matches!(v.as_str(), "production" | "prod")
            })
            .unwrap_or(false);
        if self.config.use_mock_providers && is_prod_log {
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
        // Use the normalized config flag (accepts 1/true/yes) to decide mock vs real providers.
        let builder = if self.config.use_mock_providers {
            let (cc_provider, eth_provider) =
                services::mock_providers::make_mock_providers(self.config.chain_key);
            continuity::ContinuityBuilder::new_with_providers(
                continuity_config,
                cc_provider,
                eth_provider,
            )
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
