use crate::{AsyncCallback, AsyncCallbackWithArg};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tokio::{net::TcpStream, time::timeout};
use tokio_util::sync::CancellationToken;

pub(crate) async fn check_connectivity_task(
    disconnected: Arc<AtomicBool>,
    url: impl tokio::net::ToSocketAddrs + Copy,
    tout: u64,
    check_interval: u64,
    cancellation_token: CancellationToken,

    on_toggle_connection_mode: Option<AsyncCallbackWithArg<bool, ()>>,
    on_checking_connectivity: Option<AsyncCallback<()>>,
) {
    let mut prev = disconnected.load(Ordering::Relaxed);
    loop {
        tokio::select! {
            _ = sleep(Duration::from_millis(check_interval)) => {
                let mut curr = disconnected.load(Ordering::Relaxed);

                if curr {
                    if let Some(ref callback) = on_checking_connectivity {
                        callback().await;
                    }
                    let not_connected = match timeout(Duration::from_millis(tout), TcpStream::connect(url)).await {
                        Ok(inner_res) => inner_res.is_err(),
                        Err(_) => true,
                    };

                    disconnected.store(not_connected, Ordering::Relaxed);
                    curr = not_connected;
                }
                if curr != prev {
                    if let Some(ref callback) = on_toggle_connection_mode {
                        callback(!curr).await;
                    }
                }
                prev = curr;
            },
            _ = cancellation_token.cancelled() => {
                break;
            },
        }
    }
}
