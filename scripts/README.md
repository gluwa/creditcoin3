# Creditcoin3 Scripts

Utility scripts for interacting with the Creditcoin3 network and proof generation API.

## Prerequisites

The following components must be running for these scripts to work:

- **Anvil chain** - Ethereum-compatible local development chain (source chain)
- **Creditcoin3 dev network** - Creditcoin3 blockchain node (RPC endpoint)
- **Attestor zombienet** - Attestation service that monitors and attests to source chain blocks
- **Proof gen API server** - API server that generates and serves proofs for transactions

## Setup

Install dependencies:

```bash
npm install
```

## Development

Format code:

```bash
npm run format
```

Check formatting:

```bash
npm run check-format
```

Lint code:

```bash
npm run lint
```

## Scripts

### TransferWaitAndSubmit.js

Complete end-to-end flow: transfers funds on source chain, waits for attestation, fetches proof, and submits it to the precompile.

**Usage:**

```bash
node TransferWaitAndSubmit.js [options]
```

**Example:**

```bash
# Using the same key for both chains
node TransferWaitAndSubmit.js \
  --source-rpc-url http://127.0.0.1:8545 \
  --cc3-rpc-url ws://localhost:9944 \
  --api-url http://localhost:3100

# Using different keys for Ethereum and Creditcoin3
node TransferWaitAndSubmit.js \
  --source-rpc-url http://127.0.0.1:8545 \
  --private-key 0x1234...5678 \
  --cc3-private-key 0xabcd...ef01 \
  --cc3-rpc-url ws://localhost:9944 \
  --api-url http://localhost:3100
```

**Options:**

- `--chain-key <key>` - Chain key for the source chain (default: auto-detect from chain ID)
- `--source-rpc-url <url>` - Source chain RPC URL (default: http://127.0.0.1:8545)
- `--cc3-rpc-url <url>` - Creditcoin3 RPC URL for both WS and HTTP (default: ws://localhost:9944)
- `--cc3-ws-url <url>` - Creditcoin3 WebSocket RPC URL (default: ws://localhost:9944)
- `--cc3-http-url <url>` - Creditcoin3 HTTP RPC URL (default: http://localhost:9944)
- `--private-key <key>` - Private key for signing source chain transactions (default: Anvil Account #0)
- `--cc3-private-key <key>` - Private key for signing Creditcoin3 transactions (default: same as --private-key)
- `--api-url <url>` - Proof API server URL (default: http://localhost:3100)
- `--precompile-addr <addr>` - Precompile address (default: 0x0000000000000000000000000000000000000FD2)
- `--devnet` - Use devnet provider URL for source chain

**What it does:**

1. Sends a random transfer transaction on the source chain
2. Waits for the transaction to be mined
3. Connects to Creditcoin3 and subscribes to attestation events
4. Waits for a `BlockAttested` event where the attested block number is >= the block containing the transfer
5. Waits for 2 Creditcoin3 blocks to ensure attestation is indexed
6. Fetches the proof from the proof-gen-api-server using the transaction hash
7. Converts the proof format to match the precompile interface
8. Submits the proof to the block-prover precompile using `verifyAndEmit`

### TransferAndWait.js

Sends a transfer transaction on the source chain and waits for it to be attested on Creditcoin3.

**Usage:**

```bash
node TransferAndWait.js [options]
```

**Example:**

```bash
node TransferAndWait.js \
  --source-rpc-url http://127.0.0.1:8545 \
  --cc3-rpc-url ws://localhost:9944
```

**Options:**

- `--chain-key <key>` - Chain key for the source chain (default: auto-detect from chain ID)
- `--source-rpc-url <url>` - Source chain RPC URL (default: http://127.0.0.1:8545)
- `--cc3-rpc-url <url>` - Creditcoin3 RPC URL (default: ws://localhost:9944)
- `--private-key <key>` - Private key for signing transactions (default: Anvil Account #0)
- `--devnet` - Use devnet provider URL for source chain

**What it does:**

1. Sends a random transfer transaction on the source chain
2. Waits for the transaction to be mined
3. Connects to Creditcoin3 and subscribes to attestation events
4. Waits for a `BlockAttested` event where the attested block number is >= the block containing the transfer

### SubmitProof.js

Fetches a proof from the proof-gen-api-server and submits it to the block-prover precompile.

**Usage:**

```bash
node SubmitProof.js <chainKey> <blockHeight> <txHash> --private-key <key> [options]
```

Or using npm script:

```bash
npm run submit-proof -- <chainKey> <blockHeight> <txHash> --private-key <key> [options]
```

**Example:**

```bash
node SubmitProof.js 2 18000000 0x1234...abcd \
  --private-key 0x1234...5678 \
  --api-url http://localhost:3100 \
  --cc3-rpc-url http://localhost:9944
```

**Options:**

- `--private-key <key>` - Private key for signing transactions (required)
- `--api-url <url>` - Proof generation API server URL (default: http://localhost:3100)
- `--cc3-rpc-url <url>` - Creditcoin3 RPC URL (default: http://localhost:9944)
- `--precompile-addr <addr>` - Precompile address (default: 0x0000000000000000000000000000000FD2)
- `-v, --verbose` - Enable verbose logging (shows API response details)

**What it does:**

1. Requests a proof from the proof-gen-api-server using the transaction hash
2. Converts the proof format to match the precompile interface
3. Submits the proof to the block-prover precompile using `verifyAndEmit`

**Verbose Logging:**

When enabled with `-v` or `--verbose`, the script outputs detailed debugging information:

- **API Request Details:**
  - The exact API URL being called
  - HTTP response status code and status text
  - Response headers

- **Full API Response:**
  - Complete JSON response from the proof API, including:
    - `continuityProof` - All blocks in the continuity chain with their digests
    - `merkleProof` - Merkle proof structure with siblings
    - `txBytes` - Raw transaction bytes
    - `chainKey`, `headerNumber`, `txIndex`, `txHash` - Transaction metadata
    - `cached` - Whether the proof was cached
    - `generatedAt` - Timestamp when the proof was generated

- **Error Details:**
  - Full error response bodies if API calls fail

**Use Cases:**

- **Debugging proof structure issues** - Inspect the continuity proof format and block digests
- **Comparing API responses** - See how proofs differ between API versions or requests
- **Understanding proof format** - Learn the structure of continuity proofs and merkle proofs
- **Troubleshooting API connectivity** - Verify API endpoints and response formats

**Example with verbose logging:**

```bash
node SubmitProof.js 3 9986381 0xd93880ebc927784c9ab2605d319a1e4ff78c3d91e7d744012ee2defae273f85f \
  --private-key 0x1234...5678 \
  --api-url https://proof-gen-api.usc-devnet.creditcoin.network \
  --cc3-rpc-url https://rpc.usc-devnet.creditcoin.network \
  -v
```
