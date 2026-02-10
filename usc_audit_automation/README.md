# USC Audit Automation

A monitoring tool that performs automated sanity checks on USC (Universal Smart Contract) attestations, validating blockchain integrity and alerting via Slack.

## Features

- Validates attestation continuity and merkle root correctness
- Checks attestation checkpoint timeliness
- Monitors multiple Ethereum chains via Infura/Alchemy RPC providers
- Sends alerts to Slack with detailed failure information

---

## Setup

### Local Development Setup

### 1. Build & Run the Creditcoin USC Chain

From the project root, build the Creditcoin USC node:

```bash
cargo build --release
```

Then, run the USC node locally:

```bash
./target/release/creditcoin3-node --dev --tmp
```

This starts the node at `http://localhost:9944`

### 2. Run a Local Ethereum Node (Anvil)

In a separate terminal, start an Ethereum node with Anvil:

```bash
anvil --block-time 6
```

This will run an Ethereum node at `http://localhost:8545`

### 3. Start the Attestation Zombienet

In a separate terminal, from the project root, run the following:

```bash
./target/release/attestor_zombienet \
    -n 3 \
    --bin=./target/release/attestor \
    --eth-url=ws://localhost:8545 \
    --cc3-url=ws://localhost:9944 \
    --funding-address='//Alice' \
    --config=./attestor/config.yaml \
    --chain-key=2 &
```

This starts 3 attestor nodes for the Creditcoin USC chain, connected to both the local Ethereum (Anvil) and USC nodes.

### 4. Configuration Overview

The USC Audit Automation tool uses a two-part configuration system:

1. **TOML Config File** - Contains non-secret configuration
2. **Environment Variables** - Contains sensitive secrets

#### Config File Format

Example `config.toml`:

```toml
# Public configuration
log_verbose = false
usc_network_name = "Creditcoin USC Devnet"
usc_rpc_url = "wss://rpc.usc-devnet.creditcoin.network"
usc_attestations_graphql_url = "https://attestations-graphql.usc-devnet.creditcoin.network"

# Ethereum RPC providers (API keys loaded from environment variables)
[[rpc_providers]]
name = "infura"

[[rpc_providers]]
name = "alchemy"
```

Pre-configured files are available:
- `config-devnet.toml` - Creditcoin USC Devnet
- `config-testnet.toml` - Creditcoin USC Testnet

#### Required Environment Variables

Secrets are loaded from environment variables. Copy `.env.example` to `.env` and fill in your values:

```bash
# Slack webhook URL for alerts
USC_SLACK_WEBHOOK_URL=https://hooks.slack.com/services/YOUR/WEBHOOK/URL

# Slack channel/group ID to mention in critical alerts (optional)
USC_SLACK_ALERT_GROUP=YOUR_SLACK_GROUP_ID

# USC network account mnemonic (12 or 24 words)
USC_ACCOUNT_MNEMONIC="your twelve word mnemonic phrase here"

# Infura API key for Ethereum RPC access
USC_INFURA_API_KEY=your_infura_api_key_here

# Alchemy API key for Ethereum RPC access
USC_ALCHEMY_API_KEY=your_alchemy_api_key_here
```

**Security Note:** Secrets are automatically redacted from logs (you'll see `[REDACTED]` in place of actual keys).

### 5. CLI Options

You can view all CLI options at any time using:

```bash
cargo run -- --help
```

### 6. Run the Audit Automation Tool

Navigate to the `usc_audit_automation` directory and run the program:

**For local development:**
```bash
cd usc_audit_automation
cargo run -- --config-file config.toml
```

**For devnet:**
```bash
cargo run -- --config-file config-devnet-v2.toml
```

**For testnet v2:**
```bash
cargo run -- --config-file config-testnet-v2.toml
```

You should see output similar to:

```
2025-12-22T13:30:17.014314Z  INFO usc_audit_automation: Connecting to USC RPC at wss://rpc.usc-devnet.creditcoin.network
2025-12-22T13:30:17.433658Z  INFO usc_audit_automation::ethereum_rpc: Testing RPC connection for infura...
2025-12-22T13:30:18.123456Z  INFO usc_audit_automation::ethereum_rpc: RPC Healthcheck successful for infura at https://sepolia.infura.io/v3/[REDACTED]
```

**Notes:**
- Logs show the attestation sanity checks and Slack notification payloads
- If `log_verbose=true` in your config, additional debugging information will appear
