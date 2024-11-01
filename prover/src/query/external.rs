use anyhow::Result;
use cc_client::cc3::prover::calls::types::submit_proof::Proof;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;
use tokio::time::sleep;
use tracing::info;

use super::QueryId;

#[derive(Serialize, Deserialize, Debug)]
struct WorkOrderResponse {
    query_id: String,
    status: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct WorkOrderResult {
    query_id: String,
    status: String,
    proof: Option<String>,
}

/// Handle proof order
pub async fn handle_proof_order(query_id: QueryId, files: Vec<PathBuf>) -> Result<Proof> {
    info!("Handling external proof order");
    let client = Client::new();

    // Prepare each file for the multipart form
    let mut form = reqwest::multipart::Form::new();
    for file in files {
        let file_content = tokio::fs::read(&file).await?;
        let filename = file
            .file_name()
            .ok_or("Invalid file name")
            .map_err(|_e| anyhow::anyhow!("Failed to parse file"))?;

        let filename_string = filename
            .to_str()
            .ok_or("Invalid file name")
            .map_err(|_e| anyhow::anyhow!("Failed to parse file"))?
            .to_string();

        form = form.part(
            filename_string.clone(),
            reqwest::multipart::Part::bytes(file_content).file_name(filename_string),
        );
    }

    // Add query id to the form
    form = form.text("query_id", query_id.to_string());

    // Post work order
    let response = post_work_order(&client, form).await?;

    // Poll for the result
    let proof_bytes = poll_for_result(&client, &response.query_id).await?;
    info!("Work order proof len: {}", proof_bytes.len());
    Ok(proof_bytes)
}

async fn post_work_order(
    client: &Client,
    form: reqwest::multipart::Form,
) -> Result<WorkOrderResponse> {
    let url = "https://adc4-178-51-4-81.ngrok-free.app/api/prove";
    let response = client
        .post(url)
        .multipart(form)
        .send()
        .await?
        .json::<WorkOrderResponse>()
        .await?;

    Ok(response)
}

async fn poll_for_result(client: &Client, work_order_id: &str) -> Result<Vec<u8>> {
    let url = format!(
        "https://adc4-178-51-4-81.ngrok-free.app/api/prove/{}/result",
        work_order_id
    );

    let timeout = Duration::from_secs(15 * 60); // 15 minutes
    let interval = Duration::from_secs(30); // Poll every 10 seconds
    let start = tokio::time::Instant::now();

    while start.elapsed() < timeout {
        let response = client.get(&url).send().await?;

        let proof_bytes = response.bytes().await?;

        if proof_bytes.len() > 0 {
            return Ok(proof_bytes.into());
        }

        info!("Result not yet available, waiting to retry...");
        sleep(interval).await;
    }

    Err(anyhow::anyhow!(
        "Timeout reached: Result not available within 15 minutes"
    ))
}

async fn _get_work_order_status(client: &Client, work_order_id: &str) -> Result<WorkOrderResponse> {
    let url = format!("http://127.0.0.1:5000/api/prove/{work_order_id}");

    let response = client
        .get(&url)
        .send()
        .await?
        .json::<WorkOrderResponse>()
        .await?;

    Ok(response)
}

async fn _get_work_order_result(client: &Client, work_order_id: &str) -> Result<WorkOrderResult> {
    let url = format!("http://127.0.0.1:5000/api/prove/{work_order_id}/result");

    let response = client
        .get(&url)
        .send()
        .await?
        .json::<WorkOrderResult>()
        .await?;

    Ok(response)
}
