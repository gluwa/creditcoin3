use crate::{metrics::BlockCacheMetrics, Client, Error, OrderedBlock};

use std::{
    io::{Error as IoError, Read, Result as IoResult, Write},
    sync::Arc,
};

use alloy::eips::{BlockId, BlockNumberOrTag};
use ccnext_abi_encoding::common::EncodingVersion;
use prometheus::Registry;

use redis::{aio::MultiplexedConnection, AsyncCommands, ExistenceCheck, SetExpiry, SetOptions};
use serde_json::{from_slice, to_vec};
use snap::{read::FrameDecoder, write::FrameEncoder};

const ONE_HOUR_TTL: u64 = 60 * 60;
const DBSIZE_COMMAND: &str = "DBSIZE";

pub struct BlockCacheConfig {
    pub redis_url: String,
    pub metrics_registry: Arc<Registry>,
}

#[derive(Debug, Clone)]
pub(crate) struct Cache {
    redis_conn: MultiplexedConnection,
    metrics: BlockCacheMetrics,
}

fn compress(buff: &[u8]) -> IoResult<Vec<u8>> {
    let mut encoder = FrameEncoder::new(vec![]);
    encoder.write_all(buff)?;
    encoder.into_inner().map_err(IoError::other)
}

fn inflate(buff: &[u8]) -> IoResult<Vec<u8>> {
    let mut decoder = FrameDecoder::new(buff);
    let mut inflate_buff = vec![];
    decoder.read_to_end(&mut inflate_buff)?;
    Ok(inflate_buff)
}

async fn get_total_cached_blocks(
    mut conn: MultiplexedConnection,
) -> Result<u64, redis::RedisError> {
    redis::cmd(DBSIZE_COMMAND)
        .query_async::<u64>(&mut conn)
        .await
}

// Try to get the block from redis cache, returning None if either a cache miss or an error occurs.
// We purposefully dont fail on cache errors so that the client can fallback to direct block fetching.
async fn get_cached_block(
    mut conn: MultiplexedConnection,
    chain_id: u64,
    block_number: u64,
) -> Option<OrderedBlock> {
    let key = format!("block:{chain_id}:{block_number}");

    match conn.get::<_, Option<Vec<u8>>>(&key).await {
        Ok(Some(bytes)) => {
            // If compression is enabled, inflate the cache block before attempting to
            // deserialize it
            let bytes = match inflate(&bytes) {
                Ok(d) => d,
                Err(err) => {
                    tracing::error!(
                        "Failed to inflate cached block for chain_id: {}, block_number: {}: {}",
                        chain_id,
                        block_number,
                        err
                    );
                    return None;
                }
            };

            // Try to deserialize the block
            match from_slice::<OrderedBlock>(&bytes) {
                Ok(block) => {
                    tracing::debug!(
                        "Cache hit for chain_id: {}, block_number: {}",
                        chain_id,
                        block_number
                    );

                    Some(block)
                }
                Err(_) => {
                    tracing::error!(
                        "Failed to decode cached block for chain_id: {}, block_number: {}",
                        chain_id,
                        block_number
                    );
                    None
                }
            }
        }
        Ok(None) => {
            tracing::debug!(
                "Cache miss for chain_id: {}, block_number: {}",
                chain_id,
                block_number
            );

            None
        }
        Err(err) => {
            tracing::error!(
                "Redis error when fetching cached block for chain_id: {}, block_number: {}: {}",
                chain_id,
                block_number,
                err
            );
            None
        }
    }
}

// Cache the block, logging any errors but not returning them
// in order to not impact main flows
async fn cache_block(
    mut conn: MultiplexedConnection,
    chain_id: u64,
    block_number: u64,
    block: &OrderedBlock,
) {
    let key = format!("block:{chain_id}:{block_number}");

    match to_vec(block) {
        Ok(bytes) => {
            // If compression is enabled, compress the bytes before caching
            let bytes = match compress(&bytes) {
                Ok(c) => c,
                Err(err) => {
                    tracing::error!(
                        "Failed to compress block for caching with key {}: {}",
                        key,
                        err
                    );
                    return;
                }
            };

            if let Err(err) = conn.set_ex::<_, _, ()>(&key, bytes, ONE_HOUR_TTL).await {
                tracing::error!("Redis error when caching block with key {}: {}", key, err);
            } else {
                tracing::trace!("Cached block with key {}", key);
            }
        }
        Err(err) => {
            tracing::error!(
                "Failed to encode block for caching with key {}: {}",
                key,
                err
            );
        }
    }
}

impl Client {
    pub async fn new_with_cache(
        url: &str,
        private_key: Option<&str>,
        config: BlockCacheConfig,
    ) -> Result<Self, Error> {
        let (url, rpc_provider, chain_id) = Self::init_rpc(url).await?;

        // Obtain redis connection
        let client = redis::Client::open(config.redis_url.as_str())?;
        let redis_conn = client.get_multiplexed_async_connection().await?;

        // Register prometheus metrics
        let metrics = prometheus::register_metrics(&config.metrics_registry)
            .ok_or(Error::FailedToRegisterMetrics)?;

        let cache = Cache {
            redis_conn,
            metrics,
        };

        Ok(Self {
            url,
            private_key: private_key.map(|s| s.to_owned()),
            rpc_provider,
            chain_id,
            cache: Some(cache),
        })
    }

    pub async fn get_block(
        &self,
        number: u64,
        encoding: EncodingVersion,
    ) -> Option<Result<OrderedBlock, Error>> {
        tracing::trace!(
            "Getting block {}",
            BlockId::Number(BlockNumberOrTag::Number(number))
        );

        let Some(Cache {
            ref redis_conn,
            ref metrics,
        }) = self.cache
        else {
            tracing::trace!("Block cache not configured, fetching block directly");
            return self.try_fetch_block(number, encoding).await;
        };

        // Clonning the connection is the same as cloning a handle to the connection pool
        let conn = redis_conn.clone();

        match get_cached_block(conn.clone(), self.chain_id, number).await {
            Some(block) => {
                metrics.observe_cache_hit();

                Some(Ok(block))
            }
            None => {
                metrics.observe_cache_miss();

                let lock_key = format!("lock:block:{}:{}", self.chain_id, number);

                // Thundering herd prevention: try to set a lock key with NX option so
                // that only one process fetches and caches the block
                let set_options = SetOptions::default()
                    .conditional_set(ExistenceCheck::NX) // Only set if not exists
                    .with_expiration(SetExpiry::EX(30)); // Lock expires in 30 seconds

                match conn
                    .clone()
                    .set_options::<_, _, bool>(&lock_key, 1, set_options)
                    .await
                {
                    Err(err) => {
                        tracing::error!("Redis error during locking {}: {}, falling back to fetching block directly", &lock_key, err);

                        self.try_fetch_block(number, encoding).await
                    }
                    Ok(true) => {
                        tracing::trace!("Acquired lock for {}", &lock_key);

                        // We acquired the lock, fetch and cache the block
                        let maybe_block = self.try_fetch_block(number, encoding).await;

                        if let Some(Ok(block)) = maybe_block {
                            cache_block(conn.clone(), self.chain_id, number, &block).await;

                            // Release the lock by deleting the key
                            if let Err(err) = conn.clone().del::<_, ()>(&lock_key).await {
                                tracing::error!(
                                    "Redis error when unlocking {}: {}",
                                    &lock_key,
                                    err
                                );
                            }

                            // Update the total cached blocks count
                            if let Ok(cache_blocks) = get_total_cached_blocks(conn.clone()).await {
                                metrics.set_total_cached_blocks(cache_blocks as i64);
                            }

                            Some(Ok(block))
                        } else {
                            // Release the lock by deleting the key
                            if let Err(err) = conn.clone().del::<_, ()>(&lock_key).await {
                                tracing::error!(
                                    "Redis error when unlocking {}: {}",
                                    &lock_key,
                                    err
                                );
                            }

                            maybe_block
                        }
                    }
                    Ok(false) => {
                        tracing::trace!(
                            "Did not acquire lock for {}, another process is fetching the block",
                            &lock_key
                        );

                        // We did not acquire the lock, another process is fetching the block
                        // Wait briefly and retry fetching from cache
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

                        if let Some(block) =
                            get_cached_block(conn.clone(), self.chain_id, number).await
                        {
                            Some(Ok(block))
                        } else {
                            // As a last resort, fetch the block directly
                            self.try_fetch_block(number, encoding).await
                        }
                    }
                }
            }
        }
    }
}
