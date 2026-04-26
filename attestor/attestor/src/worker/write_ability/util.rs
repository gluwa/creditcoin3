// TODO: Use this in attestor writability implementation
#[allow(unused)]
// Converts u64 chain key to right padded bytes32 format called for in writability spec
fn right_pad_u64(value: u64) -> sp_core::H256 {
    let mut bytes = [0u8; 32];

    // Convert u64 to bytes (big-endian to match Solidity-style expectations)
    let value_bytes = value.to_be_bytes();

    // Copy into the first 8 bytes (right padding = zeros on the right)
    bytes[0..8].copy_from_slice(&value_bytes);

    sp_core::H256::from(bytes)
}