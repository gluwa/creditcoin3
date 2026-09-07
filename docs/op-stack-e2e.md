# Base / OP-Stack end-to-end run

Build every source-block consumer from the same revision of creditcoin3 PR #1319. For a Solidity
application that reads deposit receipts, also rebuild it with
[asc-contracts PR #47](https://github.com/gluwa/asc-contracts/pull/47). The decoder is inlined:
updating a source package does not update an already deployed contract.

## 1. Check the source RPC and build

Base Sepolia is chain ID **84532**; its Creditcoin `chain_key` is assigned during registration and
is a different identifier. Use endpoints that serve full transactions, `eth_getBlockReceipts`,
and WebSocket `newHeads`. Configure each fallback with the same chain ID.

```sh
export BASE_HTTP='https://base-sepolia-rpc.publicnode.com'
export BASE_WS='wss://base-sepolia-rpc.publicnode.com'

# Read-only: verifies both header roots, every transaction's Merkle inclusion proof,
# and (for a WebSocket URL) receipt of a new block header.
OP_STACK_RPC_URL="$BASE_WS" cargo test --locked -p eth --test op_stack_live -- --ignored --nocapture

cargo build --locked --release -p attestor -p attestor_zombienet -p proof-gen-api-server -p query-cli -p archiver
```

The smoke test defaults to 1,000 blocks behind the source tip. Set `OP_STACK_BLOCK_NUMBER` to a
decimal height to check a specific historical block. It does not register a chain or submit votes.

## 2. Register the source and prepare attestors

On the destination Creditcoin network, an account in Operators membership must register the
source using `supportedChains.registerChain`. Set these parameters together at registration:

| Parameter | Base Sepolia test setup |
| --- | --- |
| `chain_id` | `84532` |
| `chain_name` | `Base Sepolia` |
| `target_sample_size` | The intended attestor sample size; for example, `3` with three eligible attestors |
| `chain_attestation_interval` | For example, `10` source blocks |
| `attestation_checkpoint_interval` | For example, `10` attestations |
| `max_attestors`, `max_invulnerables` | Values suitable for your attestor set, or `None` for runtime defaults |
| `attestation_chain_genesis_block_number` | A recent source height aligned with the interval; record it as `SOURCE_GENESIS` |
| `encoding` | `V1` |
| `maturity_strategy` | An explicit `FixedDelay:<blocks>` appropriate for the test's reorg tolerance |

Record the `chain_key` from `supportedChains.ChainRegistered` as `CHAIN_KEY`. For an already
registered source, check its stored chain ID, encoding, maturity, genesis height, and intervals.
Genesis cannot be changed after attestations or checkpoints exist. A fixed block delay does not
mean OP's L1 finality; do not interpret `EvmSafe` / `EvmFinalized` as OP RPC safe/finalized tags.

Fund and make the intended attestor accounts eligible under that chain's bond/election policy.
One process alone cannot satisfy a three-attestor quorum. Each process needs its own secret,
ports, log directory, and working P2P discovery. The attestor registers its BLS key during startup.

Run each attestor using the usual network configuration, with the source fields set consistently:

```sh
target/release/attestor --config /path/to/attestor.yaml \
  --chain-key "$CHAIN_KEY" --eth-url "$BASE_WS" --eth-chain-family op-stack \
  --cc3-url "$CC3_RPC_URL"
```

The equivalent YAML field is `eth.chain_family: op-stack`; the environment variable is
`ATTESTOR_ETH_CHAIN_FAMILY`. Start from the registered genesis height, or a correctly coordinated
resume height. Do not independently override intervals on just one component.

For a development/test network, the existing helper can fund, register, and launch a local set.
Use a funding account with the required registration permissions and balance; the helper submits
funding/registration transactions and generates the attestor secrets. For a three-attestor setup:

```sh
target/release/attestor_zombienet --number 3 --chain-key "$CHAIN_KEY" \
  --bin "$(pwd)/target/release/attestor" --config "$(pwd)/attestor/config.yaml" \
  --eth-url "$BASE_WS" --cc3-url "$CC3_RPC_URL" \
  --funding-address "$CC3_FUNDING_SECRET" -- --eth-chain-family op-stack
```

This uses the genesis height already registered above. The helper allocates distinct local ports;
check that all intended accounts become active before requesting a proof.

## 3. Start the proof API (and optionally the archiver)

Write a proof-server configuration using the **registered Creditcoin chain key**:

```sh
cat > /tmp/base-proof.yaml <<EOF
bind_host: "127.0.0.1"
bind_port: 3100
chains:
  - chain_key: ${CHAIN_KEY}
    eth_rpc_url: "${BASE_HTTP}"
    eth_chain_family: op-stack
max_batch_size: 10
max_batch_span: 1000
EOF

CC3_RPC_URL="$CC3_RPC_URL" target/release/proof-gen-api-server --config /tmp/base-proof.yaml
```

Omitting `block_confirmation_depth` makes the API use the registered maturity strategy. Add
`eth_rpc_fallback_urls` under the chain if available. In YAML mode the family is per chain; do not
also set the single-chain `--eth-chain-family` / `ETH_CHAIN_FAMILY` override.

An archiver is optional. If used, start with a fresh database for this source/encoding revision:

```sh
target/release/archiver \
  --rpc-http "$BASE_HTTP" --rpc-ws "$BASE_WS" --eth-chain-family op-stack \
  --cc3-rpc-url "$CC3_RPC_URL" --chain-key "$CHAIN_KEY" \
  --start-height "$SOURCE_GENESIS" --sled-db-path ./data/base-sepolia-roots.sled \
  --api-bind 127.0.0.1:8080
```

Then add `archiver_url: "http://127.0.0.1:8080"` to that proof-server chain entry. Changing a chain
or leaf-encoding policy requires rebuilding affected archived roots, not reusing incompatible data.

## 4. Fetch and verify a deposit proof

Wait until the API reports finalized attestations covering the chosen source height:

```sh
curl --fail-with-body "http://127.0.0.1:3100/api/v1/health"
curl --fail-with-body "http://127.0.0.1:3100/api/v1/attested-height/${CHAIN_KEY}"

# Index 0 is the L1-attributes deposit in an ordinary OP-Stack block.
curl --fail-with-body \
  "http://127.0.0.1:3100/api/v1/proof/${CHAIN_KEY}/${SOURCE_HEIGHT}/0" \
  --output /tmp/base-deposit-proof.json
```

Choose `SOURCE_HEIGHT` after the registered genesis and covered by the finalized attestation
range. `BlockNotReady` means the required attestation/endpoints are not available yet. HTTP 200
alone is not an end-to-end verification result: verify inclusion and continuity on Creditcoin.

Set `DEPOSIT_TX_HASH` to the transaction hash at that index and verify through the native
block-prover precompile. The command below uses an `eth_call`, without `--send-tx`:

```sh
target/release/query-cli \
  --cc3-rpc-url "$CC3_RPC_URL" --cc3-evm-private-key "$CC3_EVM_PRIVATE_KEY" \
  --eth-chain-family op-stack verify \
  --eth-rpc-url "$BASE_HTTP" --chain-key "$CHAIN_KEY" \
  --block-height "$SOURCE_HEIGHT" --txn-hash "$DEPOSIT_TX_HASH"
```

Require `Verification successful!`. For the application leg, pass a verified deposit leaf to
`EvmV1Decoder.decodeReceiptFields` or `decodeTransactionType126` from asc-contracts PR #47 and
compare status, gas usage, and event logs to the source receipt. Use a user-deposit transaction
with the application's event when testing event handling; the L1-attributes deposit may have no
logs. Deposit leaves contain no chain ID, so bind decoded data to the chain verified by the proof.
The chain-ID-dependent bridge/delivery paths remain separate from this readability flow.

## Family and nonce rules

Base, Base Sepolia, OP Mainnet, and OP Sepolia infer `op-stack` automatically. Other rollup IDs
need the same explicit override on every component:

| Component | Configuration |
| --- | --- |
| Attestor | `eth.chain_family`, `--eth-chain-family`, or `ATTESTOR_ETH_CHAIN_FAMILY` |
| Proof API | Per-chain `eth_chain_family` in YAML; `--eth-chain-family` / `ETH_CHAIN_FAMILY` in single-chain mode |
| Query CLI | Global `--eth-chain-family` / `ETH_CHAIN_FAMILY`, including transfer and batch modes |
| Archiver | `--eth-chain-family` / `ETH_CHAIN_FAMILY` for HTTP, WS, backfill, and reconnects |
| Continuity library | `ContinuityConfig::builder().eth_chain_family(Some(ChainFamily::OpStack))` |

Since Canyon, deposit leaves use the receipt-root-authenticated nonce. Missing transaction nonce
metadata is accepted; a contradictory transaction nonce causes provider fallback. Before Canyon,
the leaf nonce is zero (unavailable), because neither header root commits to the RPC nonce. Do not
use that zero to calculate historical contract-creation addresses. All attestors and proof/archiver
processes must use this same rule; pre-fix historical roots need regeneration.
