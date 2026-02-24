//! Integration tests using Anvil local Ethereum node.
//!
//! These tests require Anvil to be installed (`cargo install --git https://github.com/foundry-rs/foundry anvil`).
//! Tests are marked with `#[ignore]` by default. Run with `cargo test -- --ignored` to execute.

use std::collections::BTreeMap;
use std::time::Duration;

use alloy::network::EthereumWallet;
use alloy::providers::ProviderBuilder;
use alloy::signers::local::PrivateKeySigner;

use block_prefetcher::{BlockPrefetcher, PrefetchConfig};
use block_primitives::BlockSink;

/// Mock BlockSink for testing
#[derive(Debug, Default)]
pub struct MockBlockSink {
    pub blocks: BTreeMap<u64, eth::OrderedBlock>,
}

impl MockBlockSink {
    pub fn new() -> Self {
        Self {
            blocks: BTreeMap::new(),
        }
    }
}

impl block_primitives::BlockSink for MockBlockSink {
    type Block = eth::OrderedBlock;

    fn push(&mut self, blocks: impl IntoIterator<Item = Self::Block>) {
        self.blocks
            .extend(blocks.into_iter().map(|b| (b.number(), b)));
    }

    fn next_needed_height(&self) -> <Self::Block as block_primitives::BlockLike>::BlockNumber {
        self.blocks.keys().next_back().map(|h| *h + 1).unwrap_or(0)
    }
}

#[tokio::test]
#[ignore = "requires anvil to be installed"]
async fn test_prefetch_works() {
    // Initialize tracing for test output
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .try_init();

    use alloy::node_bindings::Anvil;

    // Spawn Anvil with 1 second block time to quickly produce blocks for testing
    let anvil = Anvil::new().block_time(1).spawn();

    let signer = PrivateKeySigner::from(anvil.keys()[0].clone());
    let wallet = EthereumWallet::from(signer);

    // Connect provider to Anvil
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .on_ws(alloy::providers::WsConnect::new(anvil.ws_endpoint()))
        .await
        .expect("Failed to connect to Anvil");

    // Wait for a few blocks to be produced before starting the prefetcher
    tokio::time::sleep(Duration::from_secs(3)).await;

    let config = PrefetchConfig {
        chain_id: anvil.chain_id(),
        max_concurrent_fetches: 5,
        finalization_lag: 5,
        max_retries: 3,
        retry_base_delay_ms: 100,
    };

    tracing::info!("Connected to Anvil at {}", anvil.ws_endpoint());

    let shared_sink = std::sync::Arc::new(parking_lot::Mutex::new(MockBlockSink::new()));
    let prefetcher = BlockPrefetcher::new(config, provider, shared_sink.clone());

    // Run the prefetcher in the background
    tokio::spawn(async move {
        let res = prefetcher.run().await;

        match res {
            Ok(_) => tracing::info!("Prefetcher loop exited successfully"),
            Err(err) => tracing::error!("Prefetcher loop exited with error: {err:?}"),
        }
    });

    tokio::time::sleep(Duration::from_secs(5)).await;

    let next_needed_height = shared_sink.lock().next_needed_height();
    assert_eq!(next_needed_height, 4, "Should have prefetched 4 blocks",);

    let expected_heights = shared_sink
        .lock()
        .blocks
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        expected_heights,
        vec![0, 1, 2, 3],
        "Prefetched blocks should be 0, 1, 2, 3"
    );
}
