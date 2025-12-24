mod error;
pub(crate) mod metrics;

use sysinfo;

use crate::prelude::*;
pub use error::*;

#[derive(attestor_macro::Builder)]
pub struct Config {
    #[specify_later]
    metrics: common::types::Metrics,
    port: u16,
}

struct AppState {
    metrics: common::types::Metrics,
    monitor: std::sync::Mutex<HwMonitor>,
}

struct HwMonitor {
    system: sysinfo::System,
    specifics: sysinfo::RefreshKind,
}

pub(crate) struct WorkerApi {
    metrics: common::types::Metrics,
    port: u16,
}

impl WorkerApi {
    pub(crate) fn new(config: Config) -> Self {
        Self {
            metrics: config.metrics,
            port: config.port,
        }
    }
}

impl super::Worker for WorkerApi {
    fn task(
        self,
        shutdown: std::pin::Pin<Box<impl std::future::Future<Output = ()>>>,
    ) -> impl std::future::Future<Output = common::types::Result<()>> {
        async move {
            let monitor = {
                let cpu_refresh = sysinfo::CpuRefreshKind::nothing().with_cpu_usage();
                let mem_refresh = sysinfo::MemoryRefreshKind::nothing().with_ram();

                let specifics = sysinfo::RefreshKind::nothing()
                    .with_cpu(cpu_refresh)
                    .with_memory(mem_refresh);

                let system = sysinfo::System::new_with_specifics(specifics);

                HwMonitor { system, specifics }
            };

            let state = AppState {
                metrics: self.metrics,
                monitor: std::sync::Mutex::new(monitor),
            };

            let router = axum::Router::new()
                .route("/metrics", axum::routing::get(handle_metrics))
                .with_state(std::sync::Arc::new(state));
            let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", self.port)).await?;
            let address = listener.local_addr().unwrap();

            tracing::info!(?address, "📌 Staring metrics server");

            tokio::select! {
                res = axum::serve(listener, router) => res?,
                _ = shutdown => {}
            }

            Ok(())
        }
    }
}

fn collect_hw_metrics(monitor: &mut HwMonitor) -> (u64, u64) {
    monitor.system.refresh_specifics(monitor.specifics);

    let cpu_usage = monitor.system.global_cpu_usage();
    let cpu_count = monitor.system.cpus().len() as f32;
    let avg_cpu_usage = cpu_usage / cpu_count;

    let total_memory = monitor.system.total_memory();
    let used_memory = monitor.system.used_memory();
    let memory_usage = (used_memory as f32 / total_memory as f32) * 100.0;

    (avg_cpu_usage as u64, memory_usage as u64)
}

async fn handle_metrics(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<AppState>>,
) -> impl axum::response::IntoResponse {
    // Update hardware metrics before encoding

    if let Ok(mut monitor) = state.monitor.lock() {
        let (cpu_usage, memory_usage) = collect_hw_metrics(&mut monitor);
        state.metrics.set_cpu_usage(cpu_usage);
        state.metrics.set_memory_usage(memory_usage);
    }

    let mut buffer = String::new();
    prometheus_client::encoding::text::encode(&mut buffer, &state.metrics.registry).unwrap();

    axum::response::Response::builder()
        .status(axum::http::StatusCode::OK)
        .header(
            axum::http::header::CONTENT_TYPE,
            "application/openmetrics-text; version=1.0.0; charset=utf-8",
        )
        .body(axum::body::Body::from(buffer))
        .unwrap()
}
