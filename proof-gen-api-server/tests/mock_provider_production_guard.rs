use proof_gen_api_server::{config::Config, db::DbManager, Server};

// Ensure server refuses to start in production when mock providers are enabled.
#[tokio::test]
async fn mock_providers_refused_in_production() {
    std::env::set_var("BIND_ADDR", "127.0.0.1:0");
    std::env::set_var("CC3_RPC_URL", "ws://mock");
    std::env::set_var("ETH_RPC_URL", "http://mock");
    std::env::set_var("CC3_KEY", "dummy mnemonic words for testing");
    std::env::set_var("CHAIN_KEY", "2");
    std::env::set_var("RUST_LOG", "production");

    // Postgres vars (builder stops before migrations so connection not used here)
    std::env::set_var("POSTGRES_HOST", "localhost");
    std::env::set_var("POSTGRES_PORT", "5432");
    std::env::set_var("POSTGRES_USER", "test");
    std::env::set_var("POSTGRES_PASSWORD", "test");
    std::env::set_var("POSTGRES_DB", "test");

    let mut cfg = Config::new_mock_config(2);
    cfg.use_mock_providers = true;

    let db = DbManager::new().expect("db manager init");
    let server = Server::new(cfg, db).await.expect("server create");

    let err = server.run().await.expect_err("should refuse startup");
    let msg = format!("{err}");
    assert!(
        msg.contains("Refusing to start"),
        "error message should contain refusal notice: {msg}"
    );
}
