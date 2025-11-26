use continuity::{ContinuityBuilder, ContinuityConfig};
use proof_gen_api_server::{build_app, ContinuityService};
use std::sync::Arc;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;

/// Starts a typed Postgres container and runs migrations, returning (app, chain_key).
pub async fn start_app_with_postgres(chain_key: u64) -> axum::Router {
    // Start container
    let container = Postgres::default().start().await.expect("start postgres");
    let port = container.get_host_port_ipv4(5432).await.expect("host port");
    // Env vars consumed by DbManager
    std::env::set_var("POSTGRES_HOST", "127.0.0.1");
    std::env::set_var("POSTGRES_PORT", port.to_string());
    std::env::set_var("POSTGRES_USER", "postgres");
    std::env::set_var("POSTGRES_PASSWORD", "postgres");
    std::env::set_var("POSTGRES_DB", "postgres");

    let cfg = ContinuityConfig {
        cc3_rpc_url: "ws://mock".into(),
        eth_rpc_url: "ws://mock".into(),
        chain_key,
    };
    let (cc_provider, eth_provider) =
        proof_gen_api_server::mock_providers::make_mock_providers(chain_key);
    let builder = ContinuityBuilder::new_with_providers(cfg, cc_provider, eth_provider);
    let db = proof_gen_api_server::db::DbManager::new().expect("db manager init");
    db.run_migrations().await.expect("migrations");
    let service = Arc::new(ContinuityService::new(Arc::new(builder), Arc::new(db)));
    // Prevent container from dropping (which would terminate Postgres) before tests complete.
    // We intentionally leak it for the duration of the test process.
    std::mem::forget(container);
    build_app(service)
}
