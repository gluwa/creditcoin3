# Query CLI

This CLI tool allows you to create and verify blockchain queries using the native query verifier precompile.

## Prerequisites

You need to run following components first:

- Creditcoin3 network (dev, testnet, or mainnet)
- Source blockchain RPC endpoint (for fetching block data)

## Installation

```sh
cargo build --release
```

## Usage

For all available options, run:

```sh
../target/release/query-cli --help
```

## Example using the EVM development account Baltathar

### Interactive Mode
```sh
../target/release/query-cli verify \
  --cc3-rpc-url "ws://localhost:9944" \
  --cc3-evm-private-key "0x8075991ce870b93a8870eca0c0f91913d12f47948ca0fd25b49c6fa7cdbeee8b"
```

### Direct Mode (with all parameters)
```sh
../target/release/query-cli verify \
  --cc3-rpc-url "ws://localhost:9944" \
  --cc3-evm-private-key "0x8075991ce870b93a8870eca0c0f91913d12f47948ca0fd25b49c6fa7cdbeee8b" \
  --eth-rpc-url "https://eth-mainnet.g.alchemy.com/v2/YOUR_API_KEY" \
  --block-height 18500000 \
  --txn-hash "0x..." \
  --data-choice 1
```

## How it works

1. **Query Creation**: The tool helps you create a query specifying what data you want to extract from a blockchain transaction
2. **Merkle Proof Generation**: It generates a Merkle proof for the transaction within its block
3. **Continuity Proof**: It fetches attestation data to prove the block is part of the canonical chain
4. **Native Verification**: The query is verified using the native query verifier precompile at address `0x0FD2`
5. **Result Display**: The extracted data segments and gas usage are displayed

## Supported Data Extraction Options

- **All Data**: Extract the entire transaction data
- **Range of Data**: Specify custom offset and size ranges
- **ERC20 Transfer Data**: Automatically extract relevant fields from ERC20 transfer transactions
- **Native Token Transfer Data**: Extract data from native token transfers
```
