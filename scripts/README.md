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
node submit-proof.js <chainKey> <blockHeight> <txHash> <privateKey> [options]
```

Or using npm script:

```bash
npm run submit-proof -- <chainKey> <blockHeight> <txHash> <privateKey> [options]
```

**Example:**

```bash
node submit-proof.js 2 18000000 0x1234...abcd 0x1234...5678 \
  --api-url http://localhost:3100 \
  --rpc-url https://eth.llamarpc.com \
  --precompile-addr 0x0000000000000000000000000000000000000FD2
```

**Options:**
- `--api-url <url>` - Proof generation API server URL (default: http://localhost:3100)
- `--rpc-url <url>` - Ethereum RPC URL (default: http://localhost:8545)
- `--precompile-addr <addr>` - Precompile address (default: 0x0000000000000000000000000000000000000FD2)

**What it does:**

1. Finds the transaction in the specified block by hash
2. Fetches raw transaction bytes from the RPC endpoint
3. Requests a proof from the proof-gen-api-server
4. Converts the proof format to match the precompile interface
5. Submits the proof to the block-prover precompile using `verifyAndEmit`

**Note:** The RPC endpoint should support `eth_getRawTransactionByHash` for best results. If not available, the script will attempt to serialize transactions using ethers.js.

