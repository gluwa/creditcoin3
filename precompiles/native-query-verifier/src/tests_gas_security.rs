// Gas security tests for native-query-verifier precompile
// Ensures gas costs prevent DoS attacks and align with Ethereum standards

use crate::mock::*;
use crate::*;

// ============================================================================
// GAS SECURITY AND DOS PREVENTION TESTS
// ============================================================================

#[test]
fn test_gas_prevents_dos_with_large_tx_data() {
    ExtBuilder::default().build().execute_with(|| {
        // Test various transaction sizes and their gas costs
        let test_cases = vec![
            (1_000, "1KB"),       // Small transaction
            (10_000, "10KB"),     // Medium transaction
            (100_000, "100KB"),   // Large transaction
            (1_000_000, "1MB"),   // Very large transaction
            (10_485_760, "10MB"), // Maximum allowed
        ];

        for (size, label) in test_cases {
            let gas_cost = GAS_BASE_VERIFY + (GAS_PER_TX_BYTE * size);

            // Ensure gas cost scales appropriately
            println!("{} transaction costs {} gas", label, gas_cost);

            // For 10MB (max size), gas should be prohibitively expensive
            if size == 10_485_760 {
                // 21,000 + (16 * 10,485,760) = 167,793,160 gas
                assert_eq!(gas_cost, 167_793_160, "10MB should cost ~168M gas");

                // This exceeds typical block gas limits (30M), preventing DoS
                assert!(gas_cost > 30_000_000, "Should exceed block gas limit");
            }
        }
    });
}

#[test]
fn test_gas_prevents_dos_with_deep_merkle_tree() {
    ExtBuilder::default().build().execute_with(|| {
        // Test Merkle trees of various depths
        let test_cases = vec![
            (1, 2),   // 1 level, 2 siblings
            (5, 10),  // 5 levels, 10 siblings
            (10, 20), // 10 levels, 20 siblings
            (20, 40), // 20 levels, 40 siblings (deep tree)
            (30, 60), // 30 levels, 60 siblings (very deep)
        ];

        for (levels, siblings) in test_cases {
            let gas_cost = GAS_BASE_VERIFY + (GAS_PER_SIBLING * siblings);

            println!("{} level tree costs {} gas", levels, gas_cost);

            // Even very deep trees should have reasonable gas costs
            assert!(gas_cost < 500_000, "Deep trees should still be affordable");

            // But cost should scale to prevent abuse
            if levels > 20 {
                assert!(gas_cost > 30_000, "Very deep trees should be expensive");
            }
        }
    });
}

#[test]
fn test_gas_prevents_dos_with_long_continuity_chain() {
    ExtBuilder::default().build().execute_with(|| {
        // Test continuity chains of various lengths
        let test_cases = vec![
            1,   // Single block
            10,  // Short chain
            50,  // Medium chain
            100, // Long chain
            500, // Very long chain
        ];

        for blocks in test_cases {
            // Each block costs GAS_PER_CONTINUITY_BLOCK + GAS_STORAGE_LOOKUP
            let gas_cost = GAS_BASE_VERIFY
                + (blocks * GAS_PER_CONTINUITY_BLOCK)
                + (blocks * GAS_STORAGE_LOOKUP);

            println!("{} block chain costs {} gas", blocks, gas_cost);

            // Long chains should be expensive but not prohibitive
            if blocks <= 100 {
                assert!(gas_cost < 1_000_000, "Normal chains should be affordable");
            } else {
                assert!(gas_cost > 1_000_000, "Very long chains should be expensive");
            }
        }
    });
}

#[test]
fn test_gas_comparison_with_ethereum_precompiles() {
    ExtBuilder::default().build().execute_with(|| {
        // Compare our gas costs with standard Ethereum precompiles

        // ecrecover costs 3,000 gas
        let ecrecover_gas = 3_000u64;

        // Our base cost should match standard transaction cost
        // (precompile efficiency offsets the complex verification work)
        let tx_base = 21_000u64;
        assert_eq!(
            GAS_BASE_VERIFY, tx_base,
            "Base gas should match standard transaction cost"
        );

        // Per-byte cost should match calldata
        assert_eq!(GAS_PER_TX_BYTE, 16, "Should match EVM calldata cost");

        // Storage lookup should match SLOAD
        assert_eq!(GAS_STORAGE_LOOKUP, 2_600, "Should match cold SLOAD");

        // Sibling verification much more efficient than ecrecover due to native execution
        assert!(
            GAS_PER_SIBLING < ecrecover_gas / 10,
            "Native sibling verification should be much cheaper than ecrecover"
        );
    });
}

#[test]
fn test_gas_for_typical_use_cases() {
    ExtBuilder::default().build().execute_with(|| {
        // Test gas costs for typical real-world scenarios

        // Scenario 1: Simple ERC20 transfer verification
        // - 200 byte transaction
        // - 4 siblings (2 levels)
        // - 3 continuity blocks
        let erc20_gas = GAS_BASE_VERIFY
            + (200 * GAS_PER_TX_BYTE)
            + (4 * GAS_PER_SIBLING)
            + (3 * GAS_PER_CONTINUITY_BLOCK)
            + (3 * GAS_STORAGE_LOOKUP);

        println!("ERC20 transfer verification: {} gas", erc20_gas);
        assert!(erc20_gas < 50_000, "Simple transfers should be < 50k gas");

        // Scenario 2: Complex DeFi transaction
        // - 1000 byte transaction
        // - 20 siblings (10 levels)
        // - 10 continuity blocks
        let defi_gas = GAS_BASE_VERIFY
            + (1000 * GAS_PER_TX_BYTE)
            + (20 * GAS_PER_SIBLING)
            + (10 * GAS_PER_CONTINUITY_BLOCK)
            + (10 * GAS_STORAGE_LOOKUP);

        println!("DeFi transaction verification: {} gas", defi_gas);
        assert!(defi_gas < 100_000, "Complex DeFi should be < 100k gas");

        // Scenario 3: NFT mint verification
        // - 500 byte transaction
        // - 8 siblings (4 levels)
        // - 5 continuity blocks
        let nft_gas = GAS_BASE_VERIFY
            + (500 * GAS_PER_TX_BYTE)
            + (8 * GAS_PER_SIBLING)
            + (5 * GAS_PER_CONTINUITY_BLOCK)
            + (5 * GAS_STORAGE_LOOKUP);

        println!("NFT mint verification: {} gas", nft_gas);
        assert!(nft_gas < 60_000, "NFT operations should be < 60k gas");
    });
}

#[test]
fn test_gas_cost_boundaries() {
    ExtBuilder::default().build().execute_with(|| {
        // Test minimum and maximum gas costs

        // Minimum: smallest possible query
        let min_gas = GAS_BASE_VERIFY +
                     GAS_PER_TX_BYTE +  // 1 byte tx
                     0 +                 // No siblings (single tx block)
                     GAS_PER_CONTINUITY_BLOCK + // 1 block
                     GAS_STORAGE_LOOKUP; // 1 lookup

        println!("Minimum gas cost: {}", min_gas);
        assert_eq!(min_gas, 21_000 + 16 + 0 + 3_000 + 2_600);
        assert_eq!(min_gas, 26_616, "Minimum should be ~27k gas");

        // Reasonable maximum: large but valid query
        let reasonable_max = GAS_BASE_VERIFY +
                           (100_000 * GAS_PER_TX_BYTE) +  // 100KB tx
                           (40 * GAS_PER_SIBLING) +        // 20 level tree
                           (50 * GAS_PER_CONTINUITY_BLOCK) + // 50 blocks
                           (50 * GAS_STORAGE_LOOKUP); // 50 lookups

        println!("Reasonable maximum gas cost: {}", reasonable_max);
        assert!(reasonable_max < 10_000_000, "Should be under 10M gas");

        // Theoretical maximum would exceed block limits, providing natural protection
    });
}

#[test]
fn test_gas_incentivizes_efficient_queries() {
    ExtBuilder::default().build().execute_with(|| {
        // Verify that gas costs incentivize efficient query design

        // Inefficient: Extracting many small segments
        let _inefficient_segments = 20; // 20 segments to extract
        let inefficient_gas = GAS_BASE_VERIFY
            + (1000 * GAS_PER_TX_BYTE)
            + (10 * GAS_PER_SIBLING)
            + (10 * GAS_PER_CONTINUITY_BLOCK)
            + (10 * GAS_STORAGE_LOOKUP);

        // Efficient: Extracting fewer, well-designed segments
        let _efficient_segments = 3; // Only 3 segments
        let efficient_gas = GAS_BASE_VERIFY +
                          (1000 * GAS_PER_TX_BYTE) +
                          (10 * GAS_PER_SIBLING) +
                          (5 * GAS_PER_CONTINUITY_BLOCK) + // Shorter chain
                          (5 * GAS_STORAGE_LOOKUP);

        // Efficient queries should cost less
        assert!(
            efficient_gas < inefficient_gas,
            "Efficient queries should be cheaper"
        );

        // The difference should be significant enough to incentivize optimization
        let savings = inefficient_gas - efficient_gas;
        assert!(
            savings > 20_000,
            "Should save significant gas with optimization"
        );

        println!("Inefficient query: {} gas", inefficient_gas);
        println!("Efficient query: {} gas", efficient_gas);
        println!("Savings from optimization: {} gas", savings);
    });
}
