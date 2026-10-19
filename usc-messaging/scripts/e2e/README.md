# Writeability full-stack scenarios

This runner starts Creditcoin, Anvil, three attestors, the quoter, proof generator
and the real `message-relayer` binary. It uses governance-based Outbox discovery,
the destination's on-chain attestor set and explicit gossip bootstrap addresses.
Each assertion is tied to the published message ID, contract address, successful
transaction receipt and on-chain state.

Build the Creditcoin binaries with `fast-runtime`, build the chosen
`asc-message-relayer` revision, compile the chosen `asc-contracts` revision and
install Foundry before running. Use the exact revisions pinned by
`.github/workflows/write-ability-e2e.yml` to reproduce CI.

```sh
cd usc-messaging
npm ci
export ASC_CONTRACTS_DIR=/path/to/compiled/asc-contracts
export RELAYER_BIN=/path/to/message-relayer
npm run e2e -- --scenario happy-path,durable-restart --output-dir /tmp/writeability-run-1
```

The output directory must not already contain `state/`. The runner preserves all
state and logs after cleanup. It refuses occupied service ports and never kills
processes by name. SIGINT/SIGTERM stop only the process groups started by this run.
The restart scenario deliberately sends SIGKILL to its own relayer group.
CI uploads the diagnostics and recovery evidence but excludes the bulky native
Creditcoin and proof-generator databases; local runs retain those databases too.

| Scenario          | Required evidence                                                                                                                                                                                                                                                                                                                                                                                                        |
| ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `happy-path`      | Exact `MessageExecuted` payload/emitter/destination and `processedAt`; exact `MessageAcknowledged` and `isAcknowledged`; automatic `DeliveryClaimed` paying the funded relay fee and tip to the expected relayer.                                                                                                                                                                                                        |
| `durable-restart` | Destination RPC gate rejects real relayer calls; the exact publication exists in the durable journal; the marked all-Outbox cursor has advanced beyond the publication plus lookback; execution is still absent; SIGKILL exits the original PID; source finality advances while it is dead; unchanged checkpoint, journal and config restart a new PID; the same execution/ack/fee assertions pass after the gate opens. |

`--lookback-blocks` defaults to 2, so the test reaches a genuine out-of-lookback
publication within a bounded time. The live checkpoint and journal are never
rewritten, deleted or rewound. A source RPC observer verifies that restarted broad
publication scans all begin after the original message block. Historical
message-ID-specific lookups used by acknowledgment routing remain allowed.

Both scenarios run sequentially on one fresh stack. Select either with
`--scenario happy-path` or `--scenario durable-restart`. The fault proxy affects
only the relayer; attestors, chain production and independent assertions continue
using the original destination RPC.

Artifacts include `report.json`, `junit.xml`, per-process logs, `events.jsonl`, the
deployment/config/state files, copies of the crash-boundary checkpoint and journal,
their SHA-256 hashes, process exit signals/PIDs, RPC gate counters and restarted
source scan ranges. Failures return a nonzero exit code; later scenarios are
reported as skipped. Reports contain no claims based solely on log strings or a
shared destination call counter.

Binary overrides are `CREDITCOIN_BIN`, `RELAYER_BIN`, `PROOF_GEN_BIN`,
`E2E_ATTESTOR_BIN`, `E2E_ZOMBIENET_BIN` and `ANVIL_BIN`. Defaults use this checkout's
`target/release/` and `~/.foundry/bin/anvil`. Service ports can be overridden with
`E2E_SOURCE_PORT`, `E2E_DESTINATION_PORT`, `E2E_QUOTER_PORT`, `E2E_PROOF_PORT`,
`E2E_RELAYER_PORT`, `E2E_SOURCE_P2P_PORT`, `E2E_RELAYER_P2P_PORT`,
`E2E_ATTESTOR_P2P_PORT_BASE` and `E2E_ATTESTOR_API_PORT_BASE`.

Run the small controller tests without blockchain binaries:

```sh
npm run e2e:typecheck
npm run e2e:test
npx prettier --check 'scripts/e2e/**' tsconfig.e2e.json
```

These tests validate the controller, fault boundary and evidence rejection. The
full-stack CI scenario is the integration check against the actual binaries.
