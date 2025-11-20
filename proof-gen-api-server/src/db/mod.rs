use anyhow::Result;
use deadpool_postgres::{Config, ManagerConfig, Pool, RecyclingMethod};
use sp_core::H256;
use tokio_postgres::NoTls;
use tracing::debug;

const V1_DOWN_SQL: &str = include_str!("../../migrations/v1/down.sql");
const V1_UP_SQL: &str = include_str!("../../migrations/v1/up.sql");

#[derive(Debug, Clone)]
pub struct ProofsDbEntry {
    pub chain_key: u64,
    pub header_number: u64,
    pub tx_index: Option<u64>,
    pub tx_hash: Option<H256>,
    pub continuity_proof: Option<()>,
    pub merkle_proof: Option<()>,
    pub merkle_root: Option<H256>,
}

pub struct DbManager {
    pool: Pool,
}

/// Creates a new db manager with a pool of DB connections
impl DbManager {
    pub fn new() -> Result<Self> {
        // Get db connection details from env variables
        let postgres_host = std::env::var("POSTGRES_HOST").expect("POSTGRES_HOST must be set");
        let postgres_port = std::env::var("POSTGRES_PORT").expect("POSTGRES_PORT must be set");
        let postgres_user = std::env::var("POSTGRES_USER").expect("POSTGRES_USER must be set");
        let postgres_password =
            std::env::var("POSTGRES_PASSWORD").expect("POSTGRES_PASSWORD must be set");
        let postgres_db = std::env::var("POSTGRES_DB").expect("POSTGRES_DB must be set");

        // Set up DB connection pool
        let mut cfg = Config::new();
        cfg.host = Some(postgres_host);
        cfg.port = Some(postgres_port.parse::<u16>()?);
        cfg.user = Some(postgres_user);
        cfg.password = Some(postgres_password);
        cfg.dbname = Some(postgres_db);
        cfg.manager = Some(ManagerConfig {
            recycling_method: RecyclingMethod::Fast,
        });

        let pool = cfg.create_pool(Some(deadpool_postgres::Runtime::Tokio1), NoTls)?;

        Ok(DbManager { pool })
    }

    pub async fn run_migrations(&self) -> Result<()> {
        let client = self.pool.get().await?;

        client.batch_execute(V1_DOWN_SQL).await?;
        client.batch_execute(V1_UP_SQL).await?;

        Ok(())
    }

    /// Creates all tables necessary for our proofs DB if not already present
    pub async fn create_example_table(&self) -> Result<()> {
        let client = self.pool.get().await?;
        // TODO: Remove this after testing
        client
            .execute(
                "CREATE TABLE IF NOT EXISTS example (
                    id SERIAL PRIMARY KEY,
                    name TEXT NOT NULL
                )",
                &[],
            )
            .await?;
        Ok(())
    }

    // Because we don't need to wait on the insert result, we spawn a task to insert in the background
    pub fn insert_proofs_entry(&self, entry: ProofsDbEntry) {
        // Converting ProofsDbEntry into tokio::postgres friendly items
        let unpacked_data = "I'm a proof!".to_string();
        let pool = self.pool.clone();
        tokio::spawn(async move {
            match pool.get().await {
                Ok(client) => {
                    if let Err(e) = client
                        .execute("INSERT INTO example (name) VALUES ($1)", &[&unpacked_data])
                        .await
                    {
                        eprintln!("Failed to insert proof {entry:?}, error: {e}");
                    }
                }
                Err(e) => {
                    eprintln!("Failed to insert proof: {entry:?}, Couldn't get DB connection from pool. Error: {e}");
                }
            }
        });
    }

    pub async fn get_proof(&self) -> Result<()> {
        let client = self.pool.get().await?;
        let rows = client.query("SELECT id, name FROM example", &[]).await?;
        for row in rows {
            let id: i32 = row.get(0);
            let name: String = row.get(1);
            println!("row: id={id}, name={name}");
        }
        Ok(())
    }

    pub async fn reset_db(&self) -> Result<()> {
        debug!("Connecting to database to reset...");
        let client = self.pool.get().await?;
        tokio::task::spawn_blocking(async move || {
            client
                .query("DROP SCHEMA public CASCADE;", &[])
                .await
                .expect("Failed to drop schema");
            client
                .query("CREATE SCHEMA public;", &[])
                .await
                .expect("Failed to create schema");
        });

        Ok(())
    }
}
