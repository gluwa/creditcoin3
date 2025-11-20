use anyhow::{bail, Result};
use deadpool_postgres::{Config, ManagerConfig, Pool, RecyclingMethod};
use serde_json::Value;
use sp_core::H256;
use tokio_postgres::NoTls;
use tracing::debug;

use type_conversions::{to_storage_int, ProofsDbEntry};

mod type_conversions;

const V1_UP_SQL: &str = include_str!("../../migrations/v1/up.sql");

#[derive(Debug, Clone)]
pub struct QueryProofs {
    pub chain_key: u64,
    pub header_number: u64,
    pub tx_index: Option<u64>,
    pub tx_hash: Option<H256>,
    pub continuity_proof: Option<Value>,
    pub merkle_proof: Option<Value>,
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
    pub fn insert_proofs_entry(&self, entry: QueryProofs) {
        let pool = self.pool.clone();
        tokio::spawn(async move {
            match pool.get().await {
                Ok(client) => {
                    // Converting QueryProofs contents into postgres friendly items
                    let entry = ProofsDbEntry::from(entry);

                    if let Err(e) = client
                        .execute(
                            r#"
                            INSERT INTO proofs (
                                chain_key,
                                header_number,
                                tx_index,
                                tx_hash,
                                continuity_proof,
                                merkle_proof,
                                merkle_root
                            )
                            VALUES ($1, $2, $3, $4, $5, $6, $7)
                            "#,
                            &[
                                &entry.chain_key,
                                &entry.header_number,
                                &entry.tx_index,
                                &entry.tx_hash,
                                &entry.continuity_proof,
                                &entry.merkle_proof,
                                &entry.merkle_root,
                            ],
                        )
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

    pub async fn get_proofs_entry(
        &self,
        chain_key: u64,
        header_number: u64,
    ) -> Result<Option<ProofsDbEntry>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                r#"
            SELECT id, header_number
            FROM proofs
            WHERE chain_key = $1
              AND header_number = $2
              AND tx_index IS NULL
            LIMIT 2
            "#,
                &[
                    &(to_storage_int(chain_key)),
                    &(to_storage_int(header_number)),
                ],
            )
            .await?;

        if rows.is_empty() {
            return Ok(None);
        }

        if rows.len() > 1 {
            bail!(
                "Expected at most one proof, but found {} for chain_key={} header_number={} with tx_index IS NULL",
                rows.len(),
                chain_key,
                header_number
            );
        }

        let entry = ProofsDbEntry::try_from(&rows[0])?;

        Ok(Some(entry))
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
