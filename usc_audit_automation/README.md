# USC Audit Automation

This document explains how to set up and run the USC Audit Automation tool locally.

---

## Local Setup

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

### 2. Start the Attestation Zombienet

In a separate terminal, navigato to `attestor_zombienet` and run the following:

```bash
../target/release/attestor_zombienet --cc3-key "//Alice"
```

This starts the attestors for the Creditcoin USC chain.

### 3. Run a Local Ethereum Node (Anvil)

In a separate terminal, start an Ethereum node with Anvil:

```bash
anvil --block-time 6
```

This will run an Ethereum node at `http://localhost:8545`

### 4. Configuration Overview

The USC Audit Automation tool is using a config file (config.toml)

Example config.toml:

```
slack_webhook_url = "SLACK_URL"
slack_alert_group = "<@GROUP_TO_ALERT>"
log_verbose = false

[[target]]
usc_network_name = "Local"
usc_metrics_url = "metrics_url"
ethereum_rpc_url = "rpc_url"
```

### 5. CLI Options

You can view all CLI options at any time using:

```bash
cargo run -- --help
```

### 6. Run the Audit Automation Tool

Navigate to the `usc_audit_automation` and run the program with the following:

```bash
cd usc_audit_automation
cargo run -- --config-file /path/to/config.toml
```

You should see output similar to:

````
2025-09-09T13:30:17.014314Z  INFO Starting attestation network height diff check for: USC Local
2025-09-09T13:30:17.433658Z  INFO Completed attestation height diff check for: USC Local
{"text":"```⬛ USC Local\n✅  block heights diff: 11 (2,211|2,200)```"}
````

- Logs show the attestation height checks and Slack notification payloads.

- If log_verbose=true, additional debugging information will appear in the console.
