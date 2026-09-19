//! Cross-checks the raw-RLP fetch mode against the JSON mode on a live node.
//!
//! Needs a node that exposes the `debug` namespace (geth/reth/bsc-geth; public providers do not).
//!
//! ```sh
//! ETH_RAW_RLP_TEST_URL=ws://127.0.0.1:8546 ETH_RAW_RLP_TEST_FROM=1 ETH_RAW_RLP_TEST_TO=50 \
//!   cargo test -p eth --test raw_rlp_vs_json -- --ignored --nocapture
//! ```

use eth::{simple_merkle_tree, BlockFetchMode, Client};
use usc_abi_encoding::common::EncodingVersion;

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[tokio::test]
#[ignore = "needs ETH_RAW_RLP_TEST_URL pointing at a node with the debug namespace"]
async fn raw_rlp_and_json_modes_agree() {
    let url = std::env::var("ETH_RAW_RLP_TEST_URL").expect("ETH_RAW_RLP_TEST_URL");
    let json = Client::new(&url, None).await.unwrap();
    let raw = Client::new(&url, None)
        .await
        .unwrap()
        .with_fetch_mode(BlockFetchMode::RawRlp);

    let from = env_u64("ETH_RAW_RLP_TEST_FROM", 1);
    let to = env_u64("ETH_RAW_RLP_TEST_TO", from + 20);
    let mut non_empty = 0usize;
    for number in from..=to {
        let a = json
            .get_block(number, EncodingVersion::V1)
            .await
            .unwrap_or_else(|e| panic!("json fetch of block {number} failed: {e:?}"));
        let b = raw
            .get_block(number, EncodingVersion::V1)
            .await
            .unwrap_or_else(|e| panic!("raw fetch of block {number} failed: {e:?}"));
        assert_eq!(a.hash(), b.hash(), "header hash differs at block {number}");
        assert_eq!(
            a.items().len(),
            b.items().len(),
            "item count differs at block {number}"
        );
        assert_eq!(
            simple_merkle_tree(&a).root(),
            simple_merkle_tree(&b).root(),
            "merkle root differs at block {number}"
        );
        for (x, y) in a.items().iter().zip(b.items()) {
            assert_eq!(x.tx().from, y.tx().from, "sender differs at block {number}");
            assert_eq!(
                x.rx().gas_used,
                y.rx().gas_used,
                "gas_used differs at block {number}"
            );
        }
        non_empty += usize::from(!a.items().is_empty());
    }
    println!("compared blocks {from}..={to}, {non_empty} with transactions");
    assert!(
        non_empty > 0,
        "range had no transactions; the comparison is vacuous"
    );
}
