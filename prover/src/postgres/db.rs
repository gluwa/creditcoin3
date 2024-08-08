use anyhow::Result;
use diesel::Connection;
use diesel_async::{
    async_connection_wrapper::AsyncConnectionWrapper,
    pooled_connection::{deadpool::Pool, AsyncDieselConnectionManager},
    AsyncPgConnection,
};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use tracing::info;

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!();

pub type PgPool = Pool<AsyncPgConnection>;

pub fn get_pool(postgres_uri: &str) -> Result<Pool<AsyncPgConnection>> {
    let config = AsyncDieselConnectionManager::<diesel_async::AsyncPgConnection>::new(postgres_uri);

    Ok(Pool::builder(config).build()?)
}

pub async fn run_migrations(postgres_uri: String) -> Result<()> {
    info!("Running databse migrations...");
    // Blocking task because diesel_async doesn't support async migrations (yet)
    tokio::task::spawn_blocking(move || {
        let mut async_wrapper: AsyncConnectionWrapper<AsyncPgConnection> =
            AsyncConnectionWrapper::establish(postgres_uri.as_str())
                .expect("Failed to connect to db");
        async_wrapper
            .run_pending_migrations(MIGRATIONS)
            .expect("Failed to run migrations");
    })
    .await?;

    Ok(())
}
