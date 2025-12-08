# Creditcoin3 Scripts

Utility scripts for interacting with the Creditcoin3 network and proof generation API.

## Setup

Install dependencies:

```bash
npm install
```

## Scripts

### submit-proof.js

Fetches a proof from the proof-gen-api-server and submits it to the block-prover precompile.

**Prerequisites:**
- Node.js 18+ (for built-in fetch support)
- ethers.js (installed via `npm install`)

**Usage:**

```bash
node submit-proof.js <chainKey> <blockHeight> <txHash> --private-key <key> [options]
```

Or using npm script:

```bash
npm run submit-proof -- <chainKey> <blockHeight> <txHash> --private-key <key> [options]
```

**Example:**

```bash
node submit-proof.js 2 18000000 0x1234...abcd \
  --private-key 0x1234...5678 \
  --api-url http://localhost:3100 \
  --cc3-rpc-url http://localhost:9944
```

**Options:**
- `--private-key <key>` - Private key for signing transactions (required)
- `--api-url <url>` - Proof generation API server URL (default: http://localhost:3100)
- `--cc3-rpc-url <url>` - Creditcoin3 RPC URL (default: http://localhost:9944)
- `--precompile-addr <addr>` - Precompile address (default: 0x0000000000000000000000000000000000000FD2)

**What it does:**

1. Requests a proof from the proof-gen-api-server using the transaction hash
2. Converts the proof format to match the precompile interface
3. Submits the proof to the block-prover precompile using `verifyAndEmit`

