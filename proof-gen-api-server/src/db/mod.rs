use anyhow::Result;
use tracing::{debug, info};

use diesel::dsl::count;
use diesel::{Connection, QueryDsl};
use diesel_async::{
    async_connection_wrapper::AsyncConnectionWrapper,
    pooled_connection::{deadpool::Pool, AsyncDieselConnectionManager},
    AsyncPgConnection, RunQueryDsl,
};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("./migrations");

pub type PgPool = Pool<AsyncPgConnection>;

pub mod continuity_blocks;
pub mod continuity_proofs;
pub mod models;
pub mod schema;
mod type_conversions;

// Re-export the main types for continuity blocks
pub use models::ContinuityBlockItem;

#[derive(Clone)]
pub struct DbManager {
    pub(crate) pool: Pool<AsyncPgConnection>,
    postgres_uri: String,
}

/// Creates a new db manager with a pool of DB connections
impl DbManager {
    pub fn new(postgres_uri: String) -> Result<Self> {
        // Set up DB connection pool
        let manager = AsyncDieselConnectionManager::<AsyncPgConnection>::new(postgres_uri.clone());
        let pool = Pool::builder(manager).build()?;

        Ok(DbManager { pool, postgres_uri })
    }

    pub async fn run_migrations(&self) -> Result<()> {
        info!("🛠️  Running database migrations...");
        let postgres_uri = self.postgres_uri.clone();
        // Blocking task because diesel_async doesn't support async migrations (yet)
        tokio::task::spawn_blocking(move || {
            let mut async_wrapper: AsyncConnectionWrapper<AsyncPgConnection> =
                AsyncConnectionWrapper::establish(&postgres_uri).expect("Failed to connect to db");
            async_wrapper
                .run_pending_migrations(MIGRATIONS)
                .expect("Failed to run migrations");
        })
        .await?;

        info!("✅ Database migrations completed successfully");
        Ok(())
    }

    pub async fn reset_database(&self) -> Result<()> {
        debug!("🔌 Connecting to database to reset...");
        let uri = self.postgres_uri.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let mut connection: AsyncConnectionWrapper<AsyncPgConnection> =
                AsyncConnectionWrapper::establish(&uri)?;
            diesel::RunQueryDsl::execute(
                diesel::sql_query("DROP SCHEMA public CASCADE;"),
                &mut connection,
            )?;
            diesel::RunQueryDsl::execute(
                diesel::sql_query("CREATE SCHEMA public;"),
                &mut connection,
            )?;
            Ok(())
        })
        .await??;

        // Re-run migrations after reset
        self.run_migrations().await?;
        Ok(())
    }

    pub async fn count_block_level(&self) -> Result<i64> {
        use crate::db::schema::continuity_proofs::dsl::*;

        let mut conn = self.pool.get().await?;
        let count_result: i64 = continuity_proofs.select(count(id)).first(&mut conn).await?;
        Ok(count_result)
    }

    // Expose the pool for direct access by attestation/checkpoint modules
    pub fn pool(&self) -> &Pool<AsyncPgConnection> {
        &self.pool
    }
}
