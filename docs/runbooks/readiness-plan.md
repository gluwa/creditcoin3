# Creditcoin3 Readiness Test Plan

A plan for testing the readiness of a Creditcoin3 (Substrate + Frontier EVM) deployment on Kubernetes/AKS, including the attestor network, archiver, proof-gen API, and cc3-indexer.

The companion fillable report lives at [`readiness-report.template.md`](./readiness-report.template.md).

## 1. Scope & inputs

Before running the plan, pin these inputs — they go at the top of the report:

| Input | Example | Where to get it |
|---|---|---|
| Environment | `usc-testnet` / `mainnet` / `staging` | K8s namespace |
| Node image tag / git SHA | `creditcoin3-node:v0.x.y@sha` | `kubectl get pod -o jsonpath='{.spec.containers[].image}'` |
| Runtime spec version | `spec_version = NN` | RPC `state_getRuntimeVersion` |
| Chain spec | `usc-testnet.json` | `chainspecs/` + `--chain` flag |
| Source chain(s) under attestation | Ethereum mainnet (`chain_key=2`), … | Attestor `config.yaml` + `pallet-supported-chains` |
| Expected committee size | e.g. 5-of-7 | `pallet-attestation::CommitteeSize` (storage query) |
| Reporter / date / window | — | — |

**Scope covers these pods/workloads** (match this to your actual Helm release):

- `creditcoin3-node` (validators + archive/rpc nodes)
- `attestor` (N replicas, one per committee member)
- `archiver` (per source chain)
- `proof-gen-api-server`
- `cc3-indexer` (SubQuery node + PostgreSQL + query service)
- Supporting infra: Redis (proof-gen cache), Postgres (indexer), optional LB/Ingress for public RPC

**Out of scope** (call out explicitly in the report so it's not mistaken for "tested"):

- `checkpoint-verifier` — dist-only in this repo, no source, no tests here
- `scripts/usc-audit-automation` — unless used as part of the test

## 2. Test phases (ordered by dependency)

Each phase has the same shape: **Purpose → Method → Pass criteria → Evidence to capture**. "Evidence" is what flows into the template report.

### Phase 0 — Cluster preconditions

**Purpose:** The cluster itself must be healthy before we trust any app-level signal.

**Method:**

```bash
kubectl get nodes -o wide
kubectl get pods -A --field-selector=status.phase!=Running
kubectl top nodes
kubectl top pods -n <ns>
kubectl get events -n <ns> --sort-by=.lastTimestamp | tail -50
kubectl get pdb,hpa,pvc -n <ns>
```

AKS-specific: check Azure Monitor "Container Insights" for node pressure (disk, memory) and any recent AKS platform events on the VMSS.

**Pass criteria:**

- All nodes `Ready`, no `MemoryPressure`/`DiskPressure`/`PIDPressure`.
- No pods in `CrashLoopBackOff`, `ImagePullBackOff`, `Pending > 2 min`.
- No `FailedScheduling`, `OOMKilled`, `Evicted` events in the last 24h.
- All PVCs `Bound`, utilization < 80%.

**Evidence:** node table, event tail, top output at T0.

### Phase 1 — Component liveness/readiness

**Purpose:** Each pod is actually up and its own health/readiness probe is green.

The Dockerfile in this repo does not define k8s probes — probes live in the deployment manifests (external). Confirm they exist and use the endpoints below, not just `tcpSocket`.

| Component | Liveness/readiness signal | Command |
|---|---|---|
| `creditcoin3-node` | `system_health` RPC returns `isSyncing=false`, `peers > 0` | `curl -s -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"system_health"}' http://<svc>:9933` |
| `creditcoin3-node` | `/metrics` scrapeable on `:9615` | `curl -s http://<svc>:9615/metrics \| head` |
| `attestor` | API on `:9100` (per `api.port` in `attestor/config.yaml`) | `curl -s http://<svc>:9100/health` (confirm actual path; see `attestor/attestor/src/` for the API router) |
| `archiver` | HTTP `:8080` (via `--api-bind`) | `curl -s http://<svc>:8080/health` |
| `proof-gen-api-server` | HTTP `:3100`, Prom on same port or sibling | `curl -s http://<svc>:3100/health` (verify with `proof-gen-api-server/bin/server.rs`) |
| `cc3-indexer` | SubQuery node + GraphQL `:3000` | `curl -s http://<svc>:3000/.well-known/apollo/server-health` or run a GraphQL `{ _metadata { lastProcessedHeight } }` |

**Pass criteria:** 200 responses across the table; probe success rate ≥ 99% in last 1h (Prometheus `kube_pod_container_status_ready`).

**Evidence:** ready replicas vs desired, restart counts over last 24h, probe failure rate.

### Phase 2 — Node sync & consensus health

**Purpose:** The chain itself is live and the node is caught up.

This is a **standalone Substrate node** (BABE + GRANDPA, Frontier EVM) — not a parachain — so readiness is self-contained: no relay-chain sync to wait on.

**Method (per node pod):**

```bash
# Sync
curl -sH 'Content-Type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"system_syncState"}' http://<svc>:9933
# Finalization
curl -sH 'Content-Type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"chain_getFinalizedHead"}' http://<svc>:9933
# Peers
curl -sH 'Content-Type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"system_peers"}' http://<svc>:9933
# Version
curl -sH 'Content-Type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"state_getRuntimeVersion"}' http://<svc>:9933
```

From Prometheus (`:9615`), record deltas over a 10-minute window:

- `substrate_block_height{status="best"}` rate → block production cadence
- `substrate_block_height{status="finalized"}` lag vs `best` → GRANDPA health (expect ≤ 2–3 blocks behind)
- `substrate_sub_libp2p_peers_count` — peer count per node
- `substrate_sub_txpool_validations_*` — mempool activity
- `substrate_proposer_block_constructed_*` — for validators, authoring frequency

**Pass criteria:**

- `syncState.currentBlock == highestBlock ± 1`.
- `best − finalized ≤ 2` sustained over the window.
- Block time ≈ target (read from runtime constants in `runtime/src/lib.rs`).
- `runtimeVersion.specVersion` matches the expected upgrade target (if a runtime upgrade is part of this release).
- Every validator under test produces at least one block in the window proportional to its BABE slot share.

**Evidence:** best/finalized heights at T0 and T+10m, finality lag plot, peer counts, runtime spec version.

### Phase 3 — Ethereum RPC surface (Frontier)

**Purpose:** EVM RPC is consistent with the substrate side; trace APIs respond if enabled.

**Method:**

```bash
# Tip
curl -sH 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}' \
  http://<svc>:9944
# Chain ID
curl -sH '...' -d '{"method":"eth_chainId",...}'
# Logs round-trip
curl -sH '...' -d '{"method":"eth_getLogs","params":[{"fromBlock":"latest","toBlock":"latest"}],...}'
# If --ethapi debug,trace is enabled:
curl -sH '...' -d '{"method":"debug_traceBlockByNumber","params":["latest",{}],...}'
```

**Pass criteria:**

- `eth_blockNumber` equals (substrate best − frontier offset) within 1 block.
- `eth_chainId` matches runtime `ChainId` constant.
- Trace APIs return valid JSON under concurrent load (respect `--ethapi-max-permits`, default 10).

**Evidence:** sampled RPC latencies (p50/p95/p99) — pull from `substrate_rpc_requests_duration_seconds` histogram if registered, or add a synthetic prober.

### Phase 4 — Attestor network health

**Purpose:** The cross-chain attestation quorum is live, producing, and finalizing on CC3. This is where most operational surprises surface.

**Method — per attestor replica** (metrics from `attestor/metrics/src/lib.rs`):

| Metric | What it tells you | Threshold |
|---|---|---|
| `attestor_peer_count` | Gossipsub peers connected | ≥ committee_size − 1 |
| `attestor_attestation_produced_height` | Last height this node attested to | monotonically rising |
| `attestor_attestation_finalized_height` | Last height quorum finalized | rising, within 1–2 rounds of produced |
| `attestor_attestation_lag_source` | Source-chain head − attested head | ≤ N blocks (product-defined; ~12 for Ethereum safe head is typical) |
| `attestor_attestation_lag_execution` | CC3 head − attestation on CC3 | ≤ small constant |
| `attestor_attestation_delay_production` | Time from source-chain block → local attestation | watch p95 trend |
| `attestor_attestation_delay_quorum` | Time local → quorum | watch p95 trend |
| `attestor_attestation_delay_finalization` | **Recently fixed** (commit `211e8f29`) — time quorum → CC3 finalization. Verify this is now populating non-zero and sensible values. | watch p95 trend |
| `attestor_gossipsub_messages` rate | Gossip throughput | > 0 |
| `attestor_invalid_attestations` | Bad attestations received | 0 sustained |
| `attestor_equivocations` | Double-signs seen | 0 sustained |
| `attestor_invalid_messages` | Bad gossipsub msgs | near-zero |
| `attestor_failed_connections` rate | libp2p dial failures | low and not rising |
| `attestor_hardware_cpu_usage` / `attestor_hardware_memory_usage` | Self-reported load | track vs k8s limits |

**On-chain corroboration (via Substrate RPC to the node):**

- Query `pallet-attestation` storage for the **committee** membership and most recent finalized attestation per supported chain. Confirm every expected attestor's key is present.
- Subscribe to or scan recent `BlockAttested` events; count per chain and compare against source-chain block production rate. Any sustained drop flags a stalled chain.
- Query `pallet-supported-chains` storage — all expected `ChainKey`s should be registered.

**Pass criteria:**

- ≥ `committee_size − f` attestors show a rising `attestation_finalized_height` over the window (where `f` is the tolerated byzantine count per pallet config).
- `attestation_delay_finalization` p95 within SLO (baseline from current testnet).
- Zero `equivocations`. Zero sustained `invalid_attestations`.
- `pallet-attestation` finalized heights on-chain increase at the expected cadence per source chain.

**Evidence:** one row per attestor pod with the table above, plus a cross-chain BlockAttested rate table.

### Phase 5 — Archiver & proof-gen pipeline

**Purpose:** On-demand proofs work end-to-end — source chain → archiver → proof-gen-api → (returned to caller).

**Method:**

1. Pick a finalized source-chain tx hash that falls within the archiver's range. (Query archiver range first: `curl http://<archiver>:8080/...` — confirm the endpoint set; see `archiver/src/main.rs` for the route list.)
2. Call proof-gen:

   ```bash
   curl -s "http://<proof-gen>:3100/proof?chain_key=2&tx_hash=0x…"
   ```
3. Verify the returned proof structure (continuity + merkle + tx bytes).
4. Submit the proof to the `block-prover` precompile (`0x…0FD2`) via an EVM transaction; check for the emitted event.

**Scripts in this repo that already orchestrate this:**

- `scripts/TransferWaitAndSubmit.js` — full flow (source-chain transfer → wait for attestation → submit proof → verify).
- `scripts/SubmitProof.js`, `scripts/SubmitBatchProof.js` — single/batch submission (batch max 10, matches `max_batch_size` in proof-gen config).

Use these as the synthetic probe rather than writing new ones.

**Metrics to capture (from `proof-gen-api-server/src/prom/mod.rs`):**

- `proof_gen_requests` rate and error ratio (`proof_gen_errors` / `proof_gen_requests`)
- `proof_gen_request_duration` p50/p95/p99
- `proof_gen_merkle_generation_duration`
- `proof_gen_last_generation_timestamp` — staleness check (now − ts should be small)
- `proof_gen_block_range` — coverage window
- Redis side: `eth_block_cache_operations` hit/miss ratio, `eth_block_cache_redis_errors` (from `common/eth/src/metrics.rs`)

**Pass criteria:**

- Golden-path submission succeeds and `BlockAttested`/prover-success event is on-chain.
- Error ratio < SLO (pick a number; 0.1% is a common starting point).
- Redis hit ratio > target (e.g., > 80%); zero sustained Redis errors.
- `proof_gen_last_generation_timestamp` is within seconds of now.

**Evidence:** tx hash, block number, proof latency, event log, RPS graph, error counts.

### Phase 6 — End-to-end (golden path + one edge case)

**Purpose:** Catch integration-layer bugs that per-component checks miss.

**Golden path:** run `scripts/TransferWaitAndSubmit.js` against the env. Record the wall-clock latency of each stage (source-chain transfer confirmation, attestor quorum, proof gen, on-chain verify).

**Edge cases worth including** (cheap, high signal):

- **Reorg resilience:** submit proof for a transaction whose source block is at/near `block_confirmation_depth` (default 12 in proof-gen config). Expect either a clean success after confirmation depth is met or a deterministic error — never a stuck request.
- **Batch boundary:** submit a 10-tx batch (at `max_batch_size`) — confirm success and check the precompile event matches all 10.

**Pass criteria:** full flow succeeds within expected latency budget; edge case returns deterministic behaviour.

### Phase 7 — Indexer & auxiliary services

**Purpose:** cc3-indexer is caught up; Postgres/Redis aren't the bottleneck.

**Method:**

- GraphQL: `{ _metadata { lastProcessedHeight, targetHeight, lastProcessedTimestamp, chain } }`
- Compute `targetHeight − lastProcessedHeight`.
- Postgres: connections in use vs max; replication lag if using HA; PVC usage. The cc3-indexer's storage grows with chain history — watch PVC growth rate.
- Redis (for proof-gen): memory usage, evictions, connected clients.

**Pass criteria:**

- Indexer lag ≤ small window (e.g., ≤ 5 blocks) sustained.
- No Postgres error-log spikes; PVC utilization < 80%.
- Redis evictions rate near zero.

### Phase 8 — Resource usage analysis (K8s side)

**Purpose:** Capacity headroom.

**Data sources:**

- `kubectl top pods/nodes` — spot values at the time of the report.
- Prometheus (cAdvisor / kubelet metrics) — trend over the window.
- Azure Monitor / Container Insights — AKS platform view (node-level pressure, VMSS autoscale events).
- Persistent volume metrics — `kubelet_volume_stats_used_bytes` / `kubelet_volume_stats_capacity_bytes`.

**PromQL queries (copy-paste ready):**

```promql
# CPU usage per pod (cores)
sum by (pod) (rate(container_cpu_usage_seconds_total{namespace="<ns>"}[5m]))

# CPU request and limit per pod
sum by (pod) (kube_pod_container_resource_requests{namespace="<ns>",resource="cpu"})
sum by (pod) (kube_pod_container_resource_limits{namespace="<ns>",resource="cpu"})

# Memory working set per pod
sum by (pod) (container_memory_working_set_bytes{namespace="<ns>", container!=""})

# Memory request and limit per pod
sum by (pod) (kube_pod_container_resource_requests{namespace="<ns>",resource="memory"})
sum by (pod) (kube_pod_container_resource_limits{namespace="<ns>",resource="memory"})

# Restart counts (last 24h)
sum by (pod) (increase(kube_pod_container_status_restarts_total{namespace="<ns>"}[24h]))

# OOMKill events (last 24h)
sum by (pod) (increase(kube_pod_container_status_last_terminated_reason{namespace="<ns>",reason="OOMKilled"}[24h]))

# PVC utilization
(kubelet_volume_stats_used_bytes{namespace="<ns>"}
 / kubelet_volume_stats_capacity_bytes{namespace="<ns>"}) * 100

# Network I/O per pod
sum by (pod) (rate(container_network_receive_bytes_total{namespace="<ns>"}[5m]))
sum by (pod) (rate(container_network_transmit_bytes_total{namespace="<ns>"}[5m]))

# Disk I/O — important for RocksDB on the node
sum by (pod) (rate(container_fs_reads_bytes_total{namespace="<ns>",container!=""}[5m]))
sum by (pod) (rate(container_fs_writes_bytes_total{namespace="<ns>",container!=""}[5m]))
```

**What to pay particular attention to for this workload:**

- **Node pod storage growth** — with `--pruning archive` (see `docker-compose.yaml`) the node DB grows without bound. Non-archive nodes should show flat PVC use. Project when PVC fills.
- **RocksDB cache sizing** — `--db-cache` (default 128 MB) is small; if the node hits its memory limit while `db-cache` is configured aggressively, you'll see OOMs. Cross-check request/limit with actual working set.
- **Archiver sled DB growth** — monotonic with source-chain blocks archived.
- **Postgres (cc3-indexer) PVC** — grows with history; typically dominates storage cost.
- **Attestor memory** — libp2p + gossipsub buffers; watch for slow leaks across 24–72h windows.
- **Trace API memory spikes** — `--ethapi-max-permits` gates concurrency; spikes on requests like `debug_traceBlockByNumber` are expected, but should decay.

**Pass criteria (suggested starting thresholds — adjust to your SLOs):**

- CPU usage p95 ≤ 70% of request; ≤ 95% of limit.
- Memory working set p95 ≤ 80% of limit; zero OOMKills in window.
- PVC < 75%, with ≥ 30d runway at current growth rate.
- Restart count = 0 for core pods over the window (minor restarts acceptable for sidecars/jobs).

**Evidence:** one table row per pod with CPU/mem actual-vs-request-vs-limit, restarts, OOMs, PVC %.

### Phase 9 — Log analysis

**Purpose:** Catch latent issues that metrics don't surface (panics, repeated warnings, peer-ban events, DB corruption warnings).

**Log formats:**

- **`creditcoin3-node`:** Substrate default text logs (`Imported #N`, `Idle`, `Starting consensus session`, `👴 Applying authority set change`, `Failed to`, `panicked`, etc.). Configure via `RUST_LOG` env var.
- **`attestor`:** **JSON** structured logs (enabled via `tracing-subscriber` feature `json`, confirmed in `attestor/attestor/Cargo.toml`). Use `jq` or Loki/Azure LogAnalytics structured queries.
- **`archiver`, `proof-gen-api-server`:** `tracing` (text by default; switch to JSON if your aggregator needs it).

**Grep / LogQL patterns to run across the window:**

```bash
# Generic error/panic
kubectl logs -n <ns> <pod> --since=24h | grep -Ei 'panic|fatal|ERROR|segfault|dead lock|database.*corrupt|stalled'

# Node-specific red flags
kubectl logs -n <ns> <node-pod> --since=24h | grep -E \
  'Verification failed|Bad block|Block prepare step failed|GRANDPA.*error|Essential task .* failed|State sync.*failed|Trie lookup error|disconnecting peer'

# Attestor (JSON) — peer churn, quorum misses
kubectl logs -n <ns> <attestor-pod> --since=24h \
  | jq -c 'select(.level=="ERROR" or .level=="WARN")' \
  | jq -r '[.timestamp, .level, .target, .fields.message] | @tsv'

# Proof-gen specific
kubectl logs -n <ns> <proof-gen-pod> --since=24h | grep -Ei \
  'timeout|upstream|rpc.*error|archiver.*unreachable|redis.*error|proof.*mismatch'
```

**Counts to capture:** error count per component, warning rate, top-5 recurring messages, any stack traces, first-seen timestamps for new error types.

**Pass criteria:**

- No panics, no essential-task failures.
- No repeated "Bad block" / "Verification failed" / "GRANDPA error" over the window.
- Attestor error rate below baseline; no new error *types* vs prior release.

### Phase 10 — Alerting, dashboards & runbooks

**Purpose:** Even if everything is green now, verify the paging path works so future regressions are caught.

This repo has **no alert rules, Grafana dashboards, or Prometheus rule files committed** — they live in the ops/SRE repo. For each, confirm:

- Alert rule exists for: finality lag, attestor quorum miss, attestation finalization delay SLO, proof-gen error rate, Redis/Postgres down, PVC > 80%, pod restart loop, OOMKill.
- Dashboards show: all metrics listed in Phase 4, 5, and 8.
- At least one alert can be fired synthetically (e.g., silence + trigger in staging) to confirm paging path.
- Runbooks exist for the top alert → step 1 of each runbook is referenced from the alert annotation.

**Pass criteria:** every metric the report depends on is dashboarded AND alerted; one end-to-end page test completed this cycle.

## 3. Sign-off matrix

The report ends with a simple grid:

| Phase | Pass / Fail / Waived | Owner | Notes |
|---|---|---|---|
| 0 Preconditions | ✅ | SRE | |
| 1 Component liveness | ✅ | SRE | |
| 2 Node sync | ✅ | Chain | |
| 3 Eth RPC | ✅ | Chain | |
| 4 Attestor | ⚠️ | Attestor team | `delay_finalization` p95 trending up |
| 5 Archiver + proof-gen | ✅ | Proof team | |
| 6 E2E | ✅ | QA | |
| 7 Indexer | ✅ | Indexer team | |
| 8 Resource | ⚠️ | SRE | PVC at 71% — plan expand in 30d |
| 9 Logs | ✅ | All | |
| 10 Alerting | ✅ | SRE | |

Release ships only when all rows are ✅ or have an explicit waiver with owner + expiry.

## 4. Notes on what is *not* in this plan

- **No zombienet `.toml`/`.zndsl` configs** — this repo uses a Rust harness (`attestor_zombienet` binary in `attestor/zombienet/`) rather than declarative zombienet. For in-cluster e2e rehearsal that mirrors CI, invoke that binary from a Job pod rather than spinning up new zombienet scenarios.
- **No pre-baked alert rules** — none are checked into the repo, so the plan points to alerts by *name* and leaves existence verification as a phase check.
- **`checkpoint-verifier` is listed as out-of-scope** — only its `dist/` is in the repo, no source or tests, so there is nothing here to exercise.
