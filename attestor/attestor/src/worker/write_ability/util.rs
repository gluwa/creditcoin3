// TODO: Use this in attestor writability implementation
#[allow(unused)]
// Converts u64 chain key to bytes32 format called for in writability spec
// We choose the standard encoding used for u64 -> bytes32, which is left padded
fn right_pad_u64(value: u64) -> sp_core::H256 {
    let mut bytes = [0u8; 32];

    // Convert u64 to bytes (big-endian to match Solidity-style expectations)
    let value_bytes = value.to_be_bytes();

    // Copy into the last 8 bytes (left padding = zeros on the left)
    bytes[24..32].copy_from_slice(&value_bytes);

    sp_core::H256::from(bytes)
}
