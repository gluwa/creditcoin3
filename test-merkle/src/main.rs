use hex;
use starknet_crypto::Felt;
use utils::block_item_traits::{BlockItem, BlockItemIdentifier};
use utils::{pedersen_hash::pedersen_array, StarknetPedersenMerkleTree};

fn main() {
    println!("Testing Merkle tree verification with exact scenario from logs");
    println!("======================================================\n");

    // Recreate the exact scenario from the logs
    // We know from logs that transactions are 991 bytes each (31 byte identifier + 960 byte payload)
    // Transaction 0 starts with: 0x00000000... (all zeros for first 32 bytes)
    // Transaction 1 starts with: 0x00000100... (0x01 at position 31)

    // Create the exact transaction data as seen in logs
    // Transaction 0: identifier (31 bytes) + payload (960 bytes)
    let mut tx0_full = vec![0u8; 31]; // identifier: all zeros for index 0
    tx0_full.extend(vec![0u8; 960]); // payload: all zeros

    // Transaction 1: identifier (31 bytes) + payload (960 bytes)
    let mut tx1_full = vec![0u8; 23]; // 23 bytes of padding
    tx1_full.extend(&1u64.to_be_bytes()); // 8 bytes for index 1 in big-endian
    let payload1 = vec![0u8; 960]; // payload is all zeros (the 0x01 we saw was from the identifier)
    tx1_full.extend(payload1);

    println!("Transaction 0 size: {} bytes", tx0_full.len());
    println!("Transaction 1 size: {} bytes", tx1_full.len());

    // Build tree with exact data
    let transactions = vec![tx0_full.clone(), tx1_full.clone()];
    let tree = StarknetPedersenMerkleTree::from(&transactions[..]);

    println!("\nTree with exact log data:");
    println!("  Root: 0x{}", hex::encode(tree.root().to_bytes_be()));
    println!("  Height: {}", tree.height());

    // Generate proof for transaction 0
    let proof = tree.generate_proof(0);
    println!("\nProof for transaction 0:");
    for (i, item) in proof.path().iter().enumerate() {
        println!("  Level {}: offset={}", i, item.offset());
        for (j, hash) in item.hashes().iter().enumerate() {
            println!("    Hash[{}]: 0x{}", j, hex::encode(hash.to_bytes_be()));
        }
    }

    // Verify the proof
    let is_valid = proof.validate(&transactions[0]);
    println!(
        "  Validation: {}",
        if is_valid { "PASSED" } else { "FAILED" }
    );

    // Compare with expected root from logs
    let expected_root_from_logs =
        "0x0313c8f056ee437e9f3c23d27966c9e5486a61bab236d6c8c0a57d36a69ddc94";
    println!("\nExpected root from logs: {}", expected_root_from_logs);
    println!(
        "Computed root: 0x{}",
        hex::encode(tree.root().to_bytes_be())
    );
    println!(
        "Match: {}",
        hex::encode(tree.root().to_bytes_be()) == expected_root_from_logs[2..]
    ); // Skip "0x" prefix

    // Test with simpler cases to understand the pattern
    println!("\n=== Testing Simple Cases ===");
    test_simple_cases();

    // Test the known problematic values
    println!("\n=== Testing Known Problematic Case ===");
    test_known_values();
}

fn test_simple_cases() {
    // Test with just payload bytes (960 bytes) - what the verifier receives
    let payload_only = vec![0u8; 960];

    // Hash the payload with leaf prefix
    let mut leaf_data = vec![0x00]; // LEAF_PREFIX
    leaf_data.extend(&payload_only);
    let leaf_hash = hash_bytes(&leaf_data);
    println!(
        "Hash of 960-byte payload: 0x{}",
        hex::encode(leaf_hash.to_bytes_be())
    );

    // Test with full data (991 bytes) - what the tree uses
    let mut full_data = vec![0u8; 31]; // identifier
    full_data.extend(&payload_only);

    let mut leaf_data_full = vec![0x00]; // LEAF_PREFIX
    leaf_data_full.extend(&full_data);
    let leaf_hash_full = hash_bytes(&leaf_data_full);
    println!(
        "Hash of 991-byte full data: 0x{}",
        hex::encode(leaf_hash_full.to_bytes_be())
    );

    // These should match the sibling[0] from logs
    let expected_placeholder = "0x02bb7ab2331f4c2ae269ab25289ddeaf2d76dbe0559c5e1783099bc0bdccccec";
    println!("Expected placeholder from logs: {}", expected_placeholder);
    println!(
        "Match with 991-byte hash: {}",
        hex::encode(leaf_hash_full.to_bytes_be()) == expected_placeholder[2..]
    );
}
fn test_known_values() {
    // Try to reproduce the exact hashes from the logs
    // When verifier receives 960 bytes, it computes: 0x0733...
    // When verifier receives 991 bytes, it computes: 0x02bb... (matches sibling[0])
    let computed_from_960 =
        Felt::from_hex("0x07332b7752fab360fefb6c3ea78faf1f8e37542e9b63cc331fafbdb09e9fa34d")
            .unwrap();
    let computed_from_991 =
        Felt::from_hex("0x02bb7ab2331f4c2ae269ab25289ddeaf2d76dbe0559c5e1783099bc0bdccccec")
            .unwrap();
    let known_sibling =
        Felt::from_hex("0x00e5a725e06f43f8b9864aef56e44cfeeb01a032aec8282e93ba63308f377f69")
            .unwrap();
    let expected_root =
        Felt::from_hex("0x0313c8f056ee437e9f3c23d27966c9e5486a61bab236d6c8c0a57d36a69ddc94")
            .unwrap();

    println!("Known values from logs:");
    println!(
        "  Computed from 960 bytes: 0x{}",
        hex::encode(computed_from_960.to_bytes_be())
    );
    println!(
        "  Computed from 991 bytes (sibling[0]): 0x{}",
        hex::encode(computed_from_991.to_bytes_be())
    );
    println!(
        "  Sibling[1]: 0x{}",
        hex::encode(known_sibling.to_bytes_be())
    );
    println!(
        "  Expected root: 0x{}",
        hex::encode(expected_root.to_bytes_be())
    );

    // Test 1: What the verifier computes with correct 991-byte data
    println!("\n  Test 1: Verifier computation with 991 bytes");
    let mut data = vec![];
    data.extend_from_slice(&Felt::from(1u8).to_bytes_be()); // prefix
    data.extend_from_slice(&computed_from_991.to_bytes_be()); // Our computed leaf (replaces sibling[0])
    data.extend_from_slice(&known_sibling.to_bytes_be()); // Sibling[1]

    let computed_root_991 = hash_bytes(&data);
    println!(
        "    Computed root: 0x{}",
        hex::encode(computed_root_991.to_bytes_be())
    );
    println!(
        "    Match with expected: {}",
        computed_root_991 == expected_root
    );

    // Test 2: Try the tree's actual structure (sibling[0] is placeholder, sibling[1] is real sibling)
    println!("\n  Test 2: Using exact siblings from proof");
    let mut data2 = vec![];
    data2.extend_from_slice(&Felt::from(1u8).to_bytes_be()); // prefix
    data2.extend_from_slice(&computed_from_991.to_bytes_be()); // sibling[0] - the placeholder
    data2.extend_from_slice(&known_sibling.to_bytes_be()); // sibling[1] - actual sibling

    let computed_with_siblings = hash_bytes(&data2);
    println!(
        "    Computed root: 0x{}",
        hex::encode(computed_with_siblings.to_bytes_be())
    );
    println!(
        "    Match with expected: {}",
        computed_with_siblings == expected_root
    );

    // The computed root 0x0673... we get suggests maybe we need the right sibling order or tree structure
    // Let's check what root we'd need to get 0x03... prefix
    println!("\n  Exploring tree structures for 0x03 prefix:");

    // Maybe there's padding or additional tree levels?
    test_with_padding(computed_from_991, known_sibling, expected_root);
}

fn test_with_padding(leaf: Felt, sibling: Felt, expected_root: Felt) {
    // Test if the tree has padding that creates additional levels

    // First compute the basic root
    let mut data = vec![];
    data.extend_from_slice(&Felt::from(1u8).to_bytes_be());
    data.extend_from_slice(&leaf.to_bytes_be());
    data.extend_from_slice(&sibling.to_bytes_be());
    let level0_hash = hash_bytes(&data);

    println!(
        "    Level 0 hash: 0x{}",
        hex::encode(level0_hash.to_bytes_be())
    );

    // Try with various padding scenarios
    let paddings = [
        Felt::ZERO,
        Felt::from(1u8),
        leaf,    // Maybe it duplicates?
        sibling, // Maybe it uses sibling as padding?
    ];

    for (i, padding) in paddings.iter().enumerate() {
        // Try padding on right
        let mut data_padded = vec![];
        data_padded.extend_from_slice(&Felt::from(1u8).to_bytes_be());
        data_padded.extend_from_slice(&level0_hash.to_bytes_be());
        data_padded.extend_from_slice(&padding.to_bytes_be());
        let padded_root = hash_bytes(&data_padded);

        if padded_root == expected_root {
            println!("    MATCH FOUND! Padding variant {} (right)", i);
            return;
        }

        // Try padding on left
        let mut data_padded_left = vec![];
        data_padded_left.extend_from_slice(&Felt::from(1u8).to_bytes_be());
        data_padded_left.extend_from_slice(&padding.to_bytes_be());
        data_padded_left.extend_from_slice(&level0_hash.to_bytes_be());
        let padded_root_left = hash_bytes(&data_padded_left);

        if padded_root_left == expected_root {
            println!("    MATCH FOUND! Padding variant {} (left)", i);
            return;
        }
    }

    println!("    No match found with padding scenarios");
}

fn test_tree_generation_patterns() {
    // Test what happens when we build a tree that should produce the expected root
    println!("Testing tree structures to find one that produces expected root...");

    // The expected root starts with 0x03, which suggests multiple levels
    // Let's try to work backwards
    let expected_root =
        Felt::from_hex("0x0313c8f056ee437e9f3c23d27966c9e5486a61bab236d6c8c0a57d36a69ddc94")
            .unwrap();

    // Try different block sizes to see which produces a root starting with 0x03
    for num_txs in 2..=16 {
        let mut txs = vec![];
        for i in 0..num_txs {
            txs.push(vec![i as u8; 960]);
        }

        let tree = StarknetPedersenMerkleTree::from(&txs[..]);
        let root = tree.root();
        let root_bytes = root.to_bytes_be();

        if root_bytes[0] == 0x03 {
            println!("  Found matching prefix with {} transactions:", num_txs);
            println!("    Root: 0x{}", hex::encode(root_bytes));
            println!("    Height: {}", tree.height());

            // Check if this is our target
            if root == expected_root {
                println!("    *** EXACT MATCH FOUND! ***");
            }
        }
    }
}

fn hash_bytes(data: &[u8]) -> Felt {
    if data.is_empty() {
        return Felt::ZERO;
    }

    let mut felts = vec![];
    let mut i = 0;

    while i < data.len() {
        let end = std::cmp::min(i + 31, data.len());
        let chunk = &data[i..end];
        let felt = Felt::from_bytes_be_slice(chunk);
        felts.push(felt);
        i = end;
    }

    pedersen_array(&felts)
}
