use std::future::Future;
use std::time::Duration;
use tokio::time;
use tracing::info;

pub async fn ret<T, E, Fut, F>(
    mut f: F,
    retries: i32,
    base_delay: u64,
    max_delay: Option<u64>,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let mut count = 0;
    let mut delay = base_delay; // Start with the base delay

    loop {
        match f().await {
            Ok(result) => {
                break Ok(result);
            }
            Err(e) => {
                if count >= retries {
                    break Err(e);
                }

                count += 1;

                // Sleep for the current delay
                time::sleep(Duration::from_secs(delay)).await;

                info!("Retrying in {}... attempt: {}", delay, count);

                // Increase delay exponentially, respecting the max_delay if provided
                delay = (delay * 2).min(max_delay.unwrap_or(u64::MAX));
            }
        }
    }
}
