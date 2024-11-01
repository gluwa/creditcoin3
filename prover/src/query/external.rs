use anyhow::Result;
use cc_client::cc3::prover::calls::types::submit_proof::Proof;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::info;

#[derive(Serialize, Deserialize, Debug)]
struct WorkOrderResponse {
    work_order_id: String,
    work_order_status: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct WorkOrderResult {
    work_order_id: String,
    work_order_status: String,
    result: Option<String>,
}

/// Handle proof order
pub async fn handle_proof_order(files: Vec<PathBuf>) -> Result<Proof> {
    // Initialize HTTP client
    let client = Client::new();

    // Read and prepare each file for the multipart form
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

    // Post work order
    let response = post_work_order(&client, form).await?;

    info!("Work order response: {:?}", response);

    Ok(vec![])
}

async fn post_work_order(
    client: &Client,
    form: reqwest::multipart::Form,
) -> Result<WorkOrderResponse> {
    let url = "https://dcac-178-51-4-81.ngrok-free.app/api/prove";

    let response = client
        .post(url)
        .multipart(form)
        .send()
        .await?
        .json::<WorkOrderResponse>()
        .await?;

    Ok(response)
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
