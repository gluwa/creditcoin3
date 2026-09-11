//! Readiness and consistent-startup regression tests. Deterministic in-process providers only.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use attestor_primitives::{AttestationCheckpoint, SignedAttestation};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use cc_client::AccountId32;
use continuity::mocks::MockCcRpcProvider;
use continuity::rpc::CcRpcProvider;
use proof_gen_api_server::prom::ProofGenMetrics;
use proof_gen_api_server::{build_app, ContinuityService};
use serde_json::Value;
use sp_core::H256;
use tower::ServiceExt;

const CHAIN: u64 = 2;
const SNAPSHOT_HEIGHT: u64 = 42;

fn snapshot_hash() -> H256 {
    H256::from_low_u64_be(0xC0FFEE)
}

/// Mock cc3 provider that can pin reads to a block and records which block each read used.
struct PinningCc {
    inner: MockCcRpcProvider,
    pinned_reads: Mutex<Vec<H256>>,
    fail_checkpoints: bool,
}

impl PinningCc {
    fn new(fail_checkpoints: bool) -> Self {
        Self {
            inner: MockCcRpcProvider::new(CHAIN),
            pinned_reads: Mutex::new(Vec::new()),
            fail_checkpoints,
        }
    }
}

#[async_trait]
impl CcRpcProvider for PinningCc {
    async fn get_attestations_for_chain(
        &self,
        chain_key: u64,
    ) -> anyhow::Result<Vec<SignedAttestation<H256, AccountId32>>> {
        self.inner.get_attestations_for_chain(chain_key).await
    }
    async fn get_last_checkpoint(
        &self,
        chain_key: u64,
    ) -> anyhow::Result<Option<AttestationCheckpoint>> {
        self.inner.get_last_checkpoint(chain_key).await
    }
    async fn get_checkpoints_for_chain(
        &self,
        chain_key: u64,
    ) -> anyhow::Result<Vec<AttestationCheckpoint>> {
        if self.fail_checkpoints {
            anyhow::bail!("cc3 unavailable");
        }
        self.inner.get_checkpoints_for_chain(chain_key).await
    }
    async fn get_checkpoint_by_height(
        &self,
        chain_key: u64,
        block_number: u64,
    ) -> anyhow::Result<Option<AttestationCheckpoint>> {
        self.inner
            .get_checkpoint_by_height(chain_key, block_number)
            .await
    }
    async fn get_attestation_chain_genesis_block_number(
        &self,
        chain_key: u64,
    ) -> anyhow::Result<u64> {
        self.inner
            .get_attestation_chain_genesis_block_number(chain_key)
            .await
    }
    async fn fetch_last_digest(&self, chain_key: u64) -> anyhow::Result<Option<H256>> {
        self.inner.fetch_last_digest(chain_key).await
    }
    async fn get_attestation_by_digest(
        &self,
        chain_key: u64,
        digest: H256,
    ) -> anyhow::Result<Option<SignedAttestation<H256, AccountId32>>> {
        self.inner
            .get_attestation_by_digest(chain_key, digest)
            .await
    }
    async fn get_attestation_interval(&self, chain_key: u64) -> anyhow::Result<Option<u64>> {
        self.inner.get_attestation_interval(chain_key).await
    }
    async fn get_checkpoint_interval(&self, chain_key: u64) -> anyhow::Result<Option<u64>> {
        self.inner.get_checkpoint_interval(chain_key).await
    }

    async fn finalized_head(&self) -> anyhow::Result<Option<(H256, u64)>> {
        Ok(Some((snapshot_hash(), SNAPSHOT_HEIGHT)))
    }
    async fn get_attestations_for_chain_at(
        &self,
        chain_key: u64,
        at: H256,
    ) -> anyhow::Result<Vec<SignedAttestation<H256, AccountId32>>> {
        self.pinned_reads.lock().unwrap().push(at);
        self.get_attestations_for_chain(chain_key).await
    }
    async fn get_checkpoints_for_chain_at(
        &self,
        chain_key: u64,
        at: H256,
    ) -> anyhow::Result<Vec<AttestationCheckpoint>> {
        self.pinned_reads.lock().unwrap().push(at);
        self.get_checkpoints_for_chain(chain_key).await
    }
    async fn get_attestation_chain_genesis_block_number_at(
        &self,
        chain_key: u64,
        at: H256,
    ) -> anyhow::Result<u64> {
        self.pinned_reads.lock().unwrap().push(at);
        self.get_attestation_chain_genesis_block_number(chain_key)
            .await
    }
}

async fn service_with(cc: Arc<dyn CcRpcProvider>) -> anyhow::Result<Arc<ContinuityService>> {
    let (_, eth) = continuity::mocks::make_mock_providers(CHAIN);
    let cfg = continuity::ContinuityConfig::builder()
        .cc3_rpc_url("ws://mock")
        .eth_rpc_url("http://mock")
        .chain_key(CHAIN)
        .attestation_interval(10)
        .checkpoint_interval(10)
        .build();
    let builder = Arc::new(continuity::ContinuityBuilder::new_with_providers(
        cfg, cc, eth,
    ));
    let metrics = Arc::new(ProofGenMetrics::new(&[CHAIN]));
    Ok(Arc::new(
        ContinuityService::new(vec![builder], metrics, 10, 1000).await?,
    ))
}

async fn get(app: &axum::Router, uri: &str) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 1 << 16)
        .await
        .unwrap();
    let json = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body).unwrap()
    };
    (status, json)
}

#[tokio::test]
async fn startup_reads_are_pinned_to_one_finalized_block_and_readiness_waits_for_catch_up() {
    let cc = Arc::new(PinningCc::new(false));
    let service = service_with(cc.clone()).await.unwrap();

    let reads = cc.pinned_reads.lock().unwrap().clone();
    assert_eq!(reads.len(), 3, "genesis, checkpoints, attestations");
    assert!(reads.iter().all(|h| *h == snapshot_hash()), "{reads:?}");
    assert_eq!(service.cc3_snapshot_height(), Some(SNAPSHOT_HEIGHT));

    let app = build_app(
        service.clone(),
        [CHAIN].into_iter().collect(),
        Arc::new(ProofGenMetrics::new(&[CHAIN])),
    );

    let (status, body) = get(&app, "/livez").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "alive");

    // Nothing processed yet: not ready, and /health keeps answering 200 with the verdict.
    let (status, body) = get(&app, "/readyz").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["ready"], false);
    assert!(body["reasons"][0]
        .as_str()
        .unwrap()
        .contains("not processed a finalized block"));
    assert_eq!(body["cc3_snapshot_height"], SNAPSHOT_HEIGHT);
    let (status, body) = get(&app, "/api/v1/health").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ready"], false);
    assert!(
        !body["not_ready_reasons"].as_array().unwrap().is_empty(),
        "ready=false always comes with at least one reason: {body}"
    );
    assert_eq!(
        body["status"], "healthy",
        "legacy status is unchanged by readiness"
    );

    // Behind the snapshot: still catching up.
    service.cc3_progress().note(SNAPSHOT_HEIGHT - 1);
    let (status, body) = get(&app, "/readyz").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(body["reasons"][0].as_str().unwrap().contains("catching up"));

    // Caught up: ready.
    service.cc3_progress().note(SNAPSHOT_HEIGHT);
    let (status, body) = get(&app, "/readyz").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ready"], true);
    assert_eq!(body["cc3_finalized_height"], SNAPSHOT_HEIGHT);

    // A dead event stream withdraws readiness again.
    service.mark_event_stream_dead("End of unbounded event stream");
    let (status, body) = get(&app, "/readyz").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(body["reasons"][0]
        .as_str()
        .unwrap()
        .contains("cc3 event stream ended"));
}

#[tokio::test]
async fn a_failing_startup_read_is_retried_then_fatal_instead_of_an_empty_cache() {
    let cc = Arc::new(PinningCc::new(true));
    let started = std::time::Instant::now();
    let err = match service_with(cc).await {
        Ok(_) => panic!("a persistent snapshot failure must fail startup"),
        Err(err) => err,
    };
    let text = format!("{err:#}");
    assert!(text.contains("checkpoints"), "{text}");
    assert!(text.contains("failed after 5 attempts"), "{text}");
    assert!(
        started.elapsed() >= Duration::from_secs(3),
        "retries with backoff must have happened"
    );
}

#[tokio::test]
async fn providers_that_cannot_pin_still_start_and_become_ready_on_first_progress() {
    let (cc, _) = continuity::mocks::make_mock_providers(CHAIN);
    let service = service_with(cc).await.unwrap();
    assert_eq!(service.cc3_snapshot_height(), None);
    assert!(!service.readiness().ready);
    service.cc3_progress().note(1);
    assert!(
        service.readiness().ready,
        "{:?}",
        service.readiness().reasons
    );
}
