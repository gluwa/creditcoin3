//! Admission-control and single-flight regression tests with deterministic in-process
//! providers (artificial delays; a concurrency experiment, not a throughput benchmark).

use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use continuity::rpc::EthRpcProvider;
use proof_gen_api_server::config::AdmissionConfig;
use proof_gen_api_server::networking::build_app_with_admission;
use proof_gen_api_server::prom::ProofGenMetrics;
use proof_gen_api_server::ContinuityService;
use serde_json::Value;
use tower::ServiceExt;

const CHAIN: u64 = 2;

/// Source-chain provider that counts concurrent tip reads and block fills, with a delay on
/// both so concurrency is observable.
struct CountingEth {
    active_tip: AtomicUsize,
    peak_tip: AtomicUsize,
    block_fetches: AtomicUsize,
    tip_delay: Duration,
    /// Delay applied to the first block fetch only (the later ones are fast).
    first_fetch_delay: Duration,
}

impl CountingEth {
    fn new(tip_delay: Duration) -> Self {
        Self {
            active_tip: AtomicUsize::new(0),
            peak_tip: AtomicUsize::new(0),
            block_fetches: AtomicUsize::new(0),
            tip_delay,
            first_fetch_delay: Duration::from_millis(100),
        }
    }
}

#[async_trait::async_trait]
impl EthRpcProvider for CountingEth {
    async fn build_continuity_blocks(
        &self,
        digest: sp_core::H256,
        start: u64,
        end: u64,
    ) -> anyhow::Result<Vec<attestor_primitives::block::Block>> {
        continuity::mocks::MockEthRpcProvider
            .build_continuity_blocks(digest, start, end)
            .await
    }
    async fn get_block_tx_bytes(&self, height: u64) -> anyhow::Result<Vec<Vec<u8>>> {
        continuity::mocks::MockEthRpcProvider
            .get_block_tx_bytes(height)
            .await
    }
    async fn get_tx_hash_by_index(
        &self,
        height: u64,
        index: u64,
    ) -> anyhow::Result<Option<sp_core::H256>> {
        continuity::mocks::MockEthRpcProvider
            .get_tx_hash_by_index(height, index)
            .await
    }
    async fn get_block_tx_data(
        &self,
        height: u64,
    ) -> anyhow::Result<Vec<(sp_core::H256, Vec<u8>)>> {
        let n = self.block_fetches.fetch_add(1, Ordering::SeqCst);
        let delay = if n == 0 {
            self.first_fetch_delay
        } else {
            Duration::from_millis(100)
        };
        tokio::time::sleep(delay).await;
        continuity::mocks::MockEthRpcProvider
            .get_block_tx_data(height)
            .await
    }
    async fn get_tx_position_by_hash(
        &self,
        _hash: sp_core::H256,
    ) -> anyhow::Result<Option<(u64, u64)>> {
        Ok(Some((100, 0)))
    }
    async fn get_last_block(&self) -> anyhow::Result<u64> {
        let active = self.active_tip.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak_tip.fetch_max(active, Ordering::SeqCst);
        tokio::time::sleep(self.tip_delay).await;
        self.active_tip.fetch_sub(1, Ordering::SeqCst);
        Ok(1000)
    }
    async fn get_chain_id(&self) -> anyhow::Result<u64> {
        Ok(31337)
    }
    async fn is_healthy(&self) -> anyhow::Result<bool> {
        Ok(true)
    }
}

async fn app_with(
    provider: Arc<CountingEth>,
    admission: AdmissionConfig,
) -> (axum::Router, String) {
    let (cc, _) = continuity::mocks::make_mock_providers(CHAIN);
    let cfg = continuity::ContinuityConfig::builder()
        .cc3_rpc_url("ws://mock")
        .eth_rpc_url("http://mock")
        .chain_key(CHAIN)
        .attestation_interval(10)
        .checkpoint_interval(10)
        .build();
    let builder = Arc::new(continuity::ContinuityBuilder::new_with_providers(
        cfg,
        cc,
        provider.clone(),
    ));
    let metrics = Arc::new(ProofGenMetrics::new(&[CHAIN]));
    let service = Arc::new(
        ContinuityService::new(vec![builder], metrics.clone(), 2, 1000)
            .await
            .unwrap(),
    );
    let app = build_app_with_admission(service, [CHAIN].into_iter().collect(), metrics, admission);
    let hash = provider
        .get_tx_hash_by_index(100, 0)
        .await
        .unwrap()
        .unwrap();
    (app, format!("/api/v1/proof-by-tx/{CHAIN}/{hash:#x}"))
}

async fn get(app: &axum::Router, uri: &str) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    let json = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body).unwrap_or(Value::Null)
    };
    (status, json)
}

#[tokio::test]
async fn a_burst_of_identical_cold_requests_is_capped_and_fills_the_block_once() {
    let provider = Arc::new(CountingEth::new(Duration::from_millis(100)));
    let admission = AdmissionConfig {
        max_in_flight_requests: NonZeroUsize::new(8).unwrap(),
        max_in_flight_per_chain: NonZeroUsize::new(8).unwrap(),
        request_timeout: Duration::from_secs(5),
    };
    let (app, uri) = app_with(provider.clone(), admission).await;

    let responses = futures::future::join_all((0..32).map(|_| get(&app, &uri))).await;
    let ok = responses
        .iter()
        .filter(|(s, _)| *s == StatusCode::OK)
        .count();
    let refused: Vec<&Value> = responses
        .iter()
        .filter(|(s, _)| *s == StatusCode::SERVICE_UNAVAILABLE)
        .map(|(_, b)| b)
        .collect();
    assert_eq!(ok, 8, "exactly the in-flight budget is admitted");
    assert_eq!(refused.len(), 24);
    assert!(refused
        .iter()
        .all(|b| b["code"] == "Overloaded" && b["retriable"] == true));
    assert!(
        provider.peak_tip.load(Ordering::SeqCst) <= 8,
        "concurrent upstream tip reads stay within the budget"
    );
    assert_eq!(
        provider.block_fetches.load(Ordering::SeqCst),
        1,
        "eight concurrent misses for one block share a single fetch"
    );

    // The refused callers retry with back-off (here: one at a time): everything is warm now
    // and nothing is fetched again.
    for _ in 0..24 {
        let (status, body) = get(&app, &uri).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    assert_eq!(provider.block_fetches.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_request_past_the_deadline_is_cut_off_with_504() {
    let provider = Arc::new(CountingEth::new(Duration::from_secs(2)));
    let admission = AdmissionConfig {
        request_timeout: Duration::from_millis(200),
        ..AdmissionConfig::default()
    };
    let (app, uri) = app_with(provider, admission).await;
    let started = std::time::Instant::now();
    let (status, body) = get(&app, &uri).await;
    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT, "{body}");
    assert_eq!(body["code"], "RequestTimeout");
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[tokio::test]
async fn health_and_readiness_are_never_subject_to_admission() {
    let provider = Arc::new(CountingEth::new(Duration::from_secs(1)));
    let admission = AdmissionConfig {
        max_in_flight_requests: NonZeroUsize::new(1).unwrap(),
        max_in_flight_per_chain: NonZeroUsize::new(1).unwrap(),
        request_timeout: Duration::from_secs(5),
    };
    let (app, uri) = app_with(provider, admission).await;

    // Occupy the single slot with a slow proof request.
    let slow_app = app.clone();
    let slow_uri = uri.clone();
    let slow = tokio::spawn(async move { get(&slow_app, &slow_uri).await });
    tokio::time::sleep(Duration::from_millis(50)).await;

    let (status, body) = get(&app, &uri).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");

    for path in ["/api/v1/health", "/livez", "/readyz", "/metrics"] {
        let started = std::time::Instant::now();
        let (status, _) = get(&app, path).await;
        assert_ne!(status, StatusCode::GATEWAY_TIMEOUT, "{path}");
        assert!(
            started.elapsed() < Duration::from_millis(900),
            "{path} must not queue behind proof traffic"
        );
    }
    assert_eq!(slow.await.unwrap().0, StatusCode::OK);
}

/// A fill leader cut off by the request deadline must release the height: the next caller
/// becomes leader and succeeds instead of waiting on a `Notify` that never fires.
#[tokio::test]
async fn a_leader_cancelled_by_the_deadline_does_not_stall_the_height() {
    let mut provider = CountingEth::new(Duration::ZERO);
    provider.first_fetch_delay = Duration::from_secs(2);
    let provider = Arc::new(provider);
    let admission = AdmissionConfig {
        request_timeout: Duration::from_millis(300),
        ..AdmissionConfig::default()
    };
    let (app, uri) = app_with(provider.clone(), admission).await;

    let (status, _) = get(&app, &uri).await;
    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT, "first fetch is slow");
    assert_eq!(provider.block_fetches.load(Ordering::SeqCst), 1);

    let (status, body) = get(&app, &uri).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        provider.block_fetches.load(Ordering::SeqCst),
        2,
        "the second caller took over the fill instead of waiting on the cancelled leader"
    );
}
