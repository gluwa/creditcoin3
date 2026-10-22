# Writeability full-stack scenarios

This runner starts Creditcoin, Anvil, four attestors, the quoter, proof generator
and the real `message-relayer` binary. It uses governance-based Outbox discovery,
the destination's on-chain attestor set and explicit gossip bootstrap addresses.
Each assertion is tied to the published message ID, contract address, successful
transaction receipt and on-chain state.

Creditcoin PR CI runs this workflow after its existing testing build and passes
that build's immutable artifact ID directly. The E2E workflow does not compile
Creditcoin again. Relayer CI pins a successful Creditcoin CI run, its artifact ID
and the exact source checkout SHA; it downloads `binary-for-testing-ci` across
repositories and builds the candidate relayer.

The optional `runner-ref` pins the scenario controller independently of the native
source SHA. This lets controller-only changes exercise an existing testing build.
Both revisions appear in `sources.json`; native artifact provenance remains bound
to `creditcoin-ref`, its producer run and its immutable artifact ID.

The artifact includes `native-build.json`. Before executing a binary, the workflow
checks the producer run/repository/workflow, archive digest, source revision,
`fast-runtime`/optimization/platform settings and every binary's SHA-256. A PR
producer's API head SHA can differ from the actual merge checkout SHA. Rerunning
only failed jobs may also retain a build uploaded in an earlier attempt; both
identities are recorded. Cached binaries retain their original compilation
revision and source/configuration key separately from the current artifact
producer. These records are uploaded under `native-provenance/` with the scenario
evidence.

To refresh a dependency pin, dispatch `.github/workflows/ci.yml` with
`writeability-e2e=true` on the desired Creditcoin revision, wait for CI success,
then update the relayer caller's source SHA, run ID and artifact ID together.
Testing artifacts are retained for 90 days. Missing, expired or incompatible
artifacts fail with an instruction to refresh the pin; there is no automatic
native build or fallback to a different revision.

Build the Creditcoin binaries with `fast-runtime`, build the chosen
`asc-message-relayer` revision, compile the chosen `asc-contracts` revision and
install Foundry before running. Use the exact revisions pinned by
`.github/workflows/write-ability-e2e.yml` to reproduce CI.

```sh
cd usc-messaging
npm ci
export ASC_CONTRACTS_DIR=/path/to/compiled/asc-contracts
export RELAYER_BIN=/path/to/message-relayer
npm run e2e -- --scenario happy-path,durable-restart,attestor-rotation --output-dir /tmp/writeability-run-1
```

The output directory must not already contain `state/`. The runner preserves all
state and logs after cleanup. It refuses occupied service ports and never kills
processes by name. SIGINT/SIGTERM stop only the process groups started by this run.
The restart scenario deliberately sends SIGKILL to its own relayer group.
CI uploads the diagnostics and recovery evidence but excludes the bulky native
Creditcoin and proof-generator databases; local runs retain those databases too.

- `happy-path` verifies the exact message's execution payload, emitter, destination
  and processed state, then its acknowledgment event and state. The automatic fee
  claim must pay the funded relay fee and tip to the expected relayer.
- `durable-restart` gates destination RPC and proves the unexecuted publication is
  journaled with the source checkpoint beyond its lookback. It kills the relayer,
  waits for source finality to advance, then starts a new PID with unchanged
  checkpoint, journal and config. Reopening RPC must produce the same execution,
  acknowledgment and fee evidence.

- `attestor-rotation` collects a valid signed replacement, revokes one of four live
  attestors, and checks the old approval fails with `InvalidAttestor` in simulation
  and in a mined transaction. With voted replacements temporarily disabled, the
  remaining three attestors must still deliver, acknowledge and settle a message.
  Re-enabling replacements lets the actual attestors propose and the actual relayer
  submit a fresh update restoring the elected set. The runner verifies its mined
  transaction signatures against the new epoch and surviving committee, rejects the
  old approval again, and verifies another message after rotation.
  These checks pass public BIP39 test identities through the zombienet launcher's
  `--seeds-file` option and validate them against the live registry before collecting
  approvals. The fixture avoids the sudo account, which cannot register as an attestor.

`--lookback-blocks` defaults to 2, so the test reaches a genuine out-of-lookback
publication within a bounded time. The live checkpoint and journal are never
rewritten, deleted or rewound. A source RPC observer verifies that restarted broad
publication scans all begin after the original message block. Historical
message-ID-specific lookups used by acknowledgment routing remain allowed.

All scenarios run sequentially on one fresh stack. Select one with
`--scenario happy-path`, `--scenario durable-restart` or `--scenario attestor-rotation`. The fault proxy affects
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
