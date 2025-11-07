# Query CLI

A command-line tool for executing and verifying cross-chain queries on the Creditcoin3 network. This tool enables querying blockchain data from source chains (like Ethereum) and verifying the results through Creditcoin3's native query precompile.

## Features

- **Native Query Verification**: Query and verify blockchain transaction data through Creditcoin3's native precompile
- **Native Token Transfers**: Execute native token transfers on Ethereum-compatible chains
- **Attestation Monitoring**: Wait for block attestations on Creditcoin3 before querying
- **Batch Operations**: Execute multiple transfers and queries in a single batch for gas optimization
- **Interactive Mode**: User-friendly prompts for query parameter input
- **CI Integration**: Automated mode for continuous integration environments

## Installation

```bash
# From the project root
cd query-cli
cargo build --release

# The binary will be available at target/release/query-cli
```

## Usage

### Basic Query Verification

Verify a transaction from a source chain:

```bash
cargo run --bin query-cli -- \
  --cc3-rpc-url ws://localhost:9944 \
  --cc3-evm-private-key <PRIVATE_KEY> \
  verify \
  --eth-rpc-url http://localhost:8545 \
  --block-height 12345 \
  --txn-hash 0xabc... \
  --data-choice 3 \
  --send-tx
```

### Interactive Mode

Run without all parameters for interactive prompts:

```bash
cargo run --bin query-cli -- \
  --cc3-rpc-url ws://localhost:9944 \
  --cc3-evm-private-key <PRIVATE_KEY> \
  verify
```

### Native Token Transfer with Query

Execute a transfer and automatically query it once attested:

```bash
cargo run --bin query-cli -- \
  --cc3-rpc-url ws://localhost:9944 \
  --cc3-evm-private-key <CC3_PRIVATE_KEY> \
  transfer \
  --eth-rpc-url http://localhost:8545 \
  --eth-private-key <ETH_PRIVATE_KEY> \
  --to-address 0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb8 \
  --amount-wei 1000000000000000 \
  --wait-attestation \
  --auto-query
```

### Batch Transfers with Query

Execute multiple transfers and batch query them:

```bash
cargo run --bin query-cli -- \
  --cc3-rpc-url ws://localhost:9944 \
  --cc3-evm-private-key <CC3_PRIVATE_KEY> \
  batch-transfer \
  --eth-rpc-url http://localhost:8545 \
  --eth-private-key <ETH_PRIVATE_KEY> \
  --count 5 \
  --base-amount 1000000000000000 \
  --wait-attestation \
  --auto-query
```

### CI Mode

For automated testing in CI environments:

```bash
cargo run --bin query-cli -- \
  --cc3-rpc-url $CC3_RPC_URL \
  --cc3-evm-private-key $CC3_PRIVATE_KEY \
  batch-transfer \
  --eth-rpc-url $ETH_RPC_URL \
  --eth-private-key $ETH_PRIVATE_KEY \
  --count 3 \
  --base-amount 1000000000000000 \
  --ci-mode \
  --auto-query
```

## Command Reference

### Global Options

- `--cc3-rpc-url`: Creditcoin3 RPC URL (default: http://localhost:9944)
- `--cc3-evm-private-key`: Private key for Creditcoin3 EVM operations
- `--verbose`: Enable debug logging

### Commands

#### `verify`

Verify a query through the native precompile.

Options:
- `--eth-rpc-url`: Source chain RPC URL
- `--block-height`: Block number to query
- `--txn-hash`: Transaction hash to verify
- `--data-choice`: Data selection (0=All, 1=Range, 2=ERC20, 3=Native)
- `--send-tx`: Send as transaction (costs gas) instead of call

#### `transfer`

Execute a native token transfer and optionally query it.

Options:
- `--eth-rpc-url`: Source chain RPC URL (required)
- `--eth-private-key`: Private key for transfers (required)
- `--to-address`: Recipient address (required)
- `--amount-wei`: Amount in wei (required)
- `--wait-attestation`: Wait for block attestation (default: true)
- `--auto-query`: Automatically query after attestation (default: true)
- `--send-tx`: Send query as transaction

#### `batch-transfer`

Execute multiple transfers and batch query them.

Options:
- `--eth-rpc-url`: Source chain RPC URL (required)
- `--eth-private-key`: Private key for transfers (required)
- `--count`: Number of transfers (default: 3)
- `--base-amount`: Base amount in wei (default: 1000000000000000)
- `--wait-attestation`: Wait for attestations (default: true)
- `--auto-query`: Automatically batch query (default: true)
- `--send-tx`: Send queries as transactions
- `--ci-mode`: Use test addresses for CI

## Architecture

### Module Structure

- **`main.rs`**: CLI entry point and command parsing
- **`attestation.rs`**: Attestation monitoring and event subscription
- **`native_transfer.rs`**: Native token transfer execution using Alloy
- **`workflow.rs`**: Orchestrates transfer → attestation → query workflows
- **`verification.rs`**: Query verification against the native precompile
- **`batch_verification.rs`**: Batch query execution and optimization
- **`merkle.rs`**: Merkle proof generation for transactions
- **`continuity.rs`**: Continuity chain validation
- **`query_builder.rs`**: Query construction with layout segments
- **`prompt.rs`**: Interactive user prompts

### Query Verification Flow

1. **Fetch Block Data**: Retrieve the block containing the transaction
2. **Generate Merkle Proof**: Create proof that transaction is in the block
3. **Build Continuity Chain**: Establish chain of blocks from attestation to query
4. **Submit to Precompile**: Verify the query through Creditcoin3's native precompile
5. **Extract Results**: Retrieve the verified data segments

### Attestation Workflow

1. **Execute Transfer**: Perform native token transfer on source chain
2. **Monitor Events**: Subscribe to Creditcoin3 attestation events
3. **Wait for Attestation**: Block until the transfer block is attested
4. **Execute Query**: Automatically query the attested transaction

## Configuration

### Network Support

- **Ethereum Mainnet**: Chain ID 1
- **Sepolia Testnet**: Chain ID 11155111
- **Local Networks**: Configurable chain ID
- **Custom Networks**: Any EVM-compatible chain

### Environment Variables

```bash
# Creditcoin3 Configuration
export CC3_RPC_URL="ws://localhost:9944"
export CC3_PRIVATE_KEY="0x..."

# Source Chain Configuration
export ETH_RPC_URL="http://localhost:8545"
export ETH_PRIVATE_KEY="0x..."
```

## Testing

Run the test suite:

```bash
cargo test
```

Run with verbose output:

```bash
cargo test -- --nocapture
```

## Performance

### Gas Optimization

- **Batch Queries**: Save 40-60% gas by sharing continuity chains
- **Merkle Proof Caching**: Reuse proofs for multiple queries
- **Event Subscription**: Efficient attestation monitoring

### Typical Gas Costs

- Single Query: ~35,000 gas
- Batch Query (5 items): ~125,000 gas (25,000 per query)
- With Transaction: +21,000 gas base cost

## Troubleshooting

### Common Issues

#### "Timeout waiting for attestation"
- Ensure Creditcoin3 chain is running
- Verify attestors are active for the source chain
- Check network connectivity

#### "Transaction not found in block"
- Verify the transaction hash is correct
- Ensure the transaction is confirmed
- Check you're querying the correct block

#### "Merkle proof invalid"
- Ensure block data is complete
- Verify transaction index is correct
- Check encoding version matches

## Development

### Building from Source

```bash
# Install dependencies
cargo build

# Run in development mode
cargo run -- --help

# Build optimized binary
cargo build --release
```

### Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Add tests for new functionality
5. Ensure all tests pass
6. Submit a pull request

## License

MIT
