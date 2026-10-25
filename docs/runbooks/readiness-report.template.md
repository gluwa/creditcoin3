# Creditcoin3 Readiness Report

## 1. Phase 0 — Cluster preconditions

- Nodes Ready: 2/2
- Non-Running pods: `All pods running normally`
- PVCs Bound and &lt; 80%: &lt;yes/no, list exceptions&gt; `All di
- Recent abnormal events: &lt;paste `kubectl get events` tail or "none"&gt;

## 2. Phase 1 — Component liveness

| Component | Ready replicas / desired | Restarts (24h) | Probe success % (1h) | Notes |
|---|---|---|---|---|
| creditcoin3-node | | | | |
| attestor | | | | |
| archiver | | | | |
| proof-gen-api-server | | | | |
| cc3-indexer (node) | | | | |
| cc3-indexer (query) | | | | |
| postgres (indexer) | | | | |
| redis (proof-gen) | | | | |

## 3. Phase 2 — Node sync & consensus

| Pod | Best height T0 | Best height T+10m | Finalized T+10m | best−finalized | Peers | Runtime spec |
|---|---|---|---|---|---|---|
| | | | | | | |

- Block production rate (blocks/min): &lt;observed&gt; vs expected &lt;target&gt;.
- Validators producing blocks in window: &lt;list / count&gt;.

## 4. Phase 3 — Ethereum RPC

| Check | Result |
|---|---|
| eth_blockNumber vs substrate best | Δ = |
| eth_chainId matches runtime | ✓ / ✗ |
| eth_getLogs (latest) roundtrip | ms |
| debug_traceBlockByNumber (if enabled) | ok / err |
| RPC p50 / p95 / p99 latency | / / ms |

## 5. Phase 4 — Attestor network

### 6.1 Per-attestor snapshot

| Attestor | Peers | produced_height | finalized_height | lag_source | lag_execution | delay_production p95 | delay_quorum p95 | delay_finalization p95 | invalid_atts | equivocations | failed_conns rate |
|---|---|---|---|---|---|---|---|---|---|---|---|
| | | | | | | | | | | | |

### 6.2 On-chain corroboration

| Source chain | Expected cadence | BlockAttested events in window | Committee members present | Missing / equivocating |
|---|---|---|---|---|
| Ethereum (2) | | | | |

Note any change vs prior release (especially on `attestation_delay_finalization` — verify the fix from commit `211e8f29` is populating correctly).

## 7. Phase 5 — Archiver & proof-gen

| Check | Result |
|---|---|
| Archiver height / range | |
| proof_gen_request rate | req/s |
| proof_gen_error ratio | % |
| proof_gen_request_duration p50/p95/p99 | / / ms |
| proof_gen_merkle_generation_duration p95 | ms |
| proof_gen_last_generation_timestamp staleness | s |
| Redis hit ratio | % |
| Redis errors in window | count |
| Precompile `BlockAttested` events (sample) | &lt;tx hash / block&gt; |

## 8. Phase 6 — End-to-end golden path

Ran `scripts/TransferWaitAndSubmit.js` at &lt;ISO timestamp&gt;.

| Stage | Duration | Status |
|---|---|---|
| Source-chain transfer confirmed | ms | |
| Attestor quorum reached | ms | |
| Proof generated | ms | |
| On-chain verification event | ms | |
| **Total wall-clock** | **ms** | |

Edge cases tested:

- [ ] `block_confirmation_depth` boundary
- [ ] Batch of 10 (at `max_batch_size`)

## 9. Phase 7 — Indexer & auxiliary

| Check | Result |
|---|---|
| cc3-indexer lag (target − last processed) | blocks |
| GraphQL probe latency | ms |
| Postgres connections / max | / |
| Postgres PVC utilization | % |
| Redis memory / evictions | / |

## 10. Phase 8 — Kubernetes resource usage

### 10.1 Per-pod utilization (window = &lt;start&gt;→&lt;end&gt;)

| Pod | CPU used p95 (cores) | CPU request | CPU limit | Mem used p95 (GiB) | Mem request | Mem limit | Restarts | OOMKills | PVC % | Notes |
|---|---|---|---|---|---|---|---|---|---|---|
| creditcoin3-node-0 | | | | | | | | | | |
| attestor-0 | | | | | | | | | | |
| archiver-0 | | | | | | | | | | |
| proof-gen-0 | | | | | | | | | | |
| cc3-indexer-node-0 | | | | | | | | | | |
| cc3-indexer-query-0 | | | | | | | | | | |
| postgres-0 | | | | | | | | | | |
| redis-0 | | | | | | | | | | |

### 10.2 Storage runway

| PVC | Size | Used | % | Growth rate | Estimated days to 85% |
|---|---|---|---|---|---|
| node-data | | | | GiB/day | |
| archiver-sled | | | | | |
| postgres-indexer | | | | | |

### 10.3 Node-level (AKS VMSS)

| VM / node | CPU avg | Mem avg | Pressure events | Notes |
|---|---|---|---|---|
| | | | | |

## 11. Phase 9 — Log analysis

| Component | Error count (24h) | Warn count (24h) | New error types | Top recurring message |
|---|---|---|---|---|
| creditcoin3-node | | | | |
| attestor | | | | |
| archiver | | | | |
| proof-gen-api-server | | | | |
| cc3-indexer | | | | |

Panics / essential-task failures: &lt;list or "none"&gt;.

Notable log excerpts (first-seen timestamps + short snippet):

```text
<paste>
```

## 12. Phase 10 — Alerting & dashboards

| Alert | Exists | Wired to pager | Fired-for-test this cycle | Runbook |
|---|---|---|---|---|
| Finality lag | | | | |
| Attestor quorum miss | | | | |
| attestation_delay_finalization SLO | | | | |
| proof_gen error rate | | | | |
| PVC &gt; 80% | | | | |
| Pod restart loop | | | | |
| OOMKill | | | | |
| Redis / Postgres down | | | | |

## 13. Issues / anomalies / waivers

| # | Severity | Summary | Owner | Action | Target date |
|---|---|---|---|---|---|
| 1 | | | | | |
