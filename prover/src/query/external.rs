use anyhow::Result;
use cc_client::cc3::prover::calls::types::submit_proof::Proof;
use hex::ToHex;
use reqwest::header::ACCEPT;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;
use thiserror::Error;
use tokio::time::sleep;
use tracing::{info, warn};

use super::QueryId;

const PROVER_BE_SOCKET_ADDR: &str = "0.0.0.0:58313";
// Maps proving input file names to corresponding proving request field names
const FILE_NAME_TO_FIELD_MAP: &[(&str, &str)] = &[
    ("trace.json", "TraceFile"),
    ("memory.json", "MemoryFile"),
    ("private_input.json", "PrivateInputFile"),
    ("public_input.json", "PublicInputFile"),
];

fn get_request_field(file_name: &str) -> Result<String> {
    Ok(FILE_NAME_TO_FIELD_MAP
        .iter()
        .filter(|(file, _)| *file == file_name)
        .next()
        .ok_or(Error::BadProofInputFile(file_name.to_string()))?
        .1
        .to_string())
}

#[derive(Serialize, Deserialize, Debug)]
struct WorkOrderResponse {
    request_id: String,
    query_id: String,
    status: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct WorkOrderResult {
    query_id: String,
    status: String,
    proof: Option<String>,
}

#[derive(Debug, Error)]
enum Error {
    #[error("The following file has no request field mapping: {0}")]
    BadProofInputFile(String),
    #[error("Sending request failed. Error: {0}")]
    ReqwestSendError(String),
    #[error("BadProofOrderRequest. Error: {0}")]
    BadProofOrderRequest(String),
    #[error("Couldn't parse work order response. Error: {0}")]
    BadProofOrderResponse(String),
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

        let request_field = match get_request_field(&filename_string) {
            Ok(field_name) => field_name,
            Err(e) => {
                warn!("Unexpected file in proof inputs dir. Error: {:?}", e);
                continue;
            }
        };

        form = form.part(
            request_field,
            reqwest::multipart::Part::bytes(file_content).file_name(filename_string),
        );
    }

    // Convert query_id into UUID expected by StoneProverBE
    let uuid_string: String = sp_core::twox_128(query_id.as_bytes()).encode_hex();

    // Add query id to the form
    form = form.text("queryId", uuid_string);

    // Post work order
    let response = post_work_order(&client, form).await?;

    // Poll for the result
    let proof_bytes = poll_for_result(&client, &response.request_id).await?;
    info!("Work order proof len: {}", proof_bytes.len());
    Ok(proof_bytes)
}

async fn post_work_order(
    client: &Client,
    form: reqwest::multipart::Form,
) -> std::result::Result<WorkOrderResponse, Error> {
    let url = format!(
        "http://{PROVER_BE_SOCKET_ADDR}/AzureAppService/QueueLightProverQueryRequest/prove"
    );
    let response = client
        .post(url)
        .header(ACCEPT, "*/*")
        .multipart(form)
        .send()
        .await
        .map_err(|e| Error::ReqwestSendError(e.to_string()))?;

    match response.status() {
        reqwest::StatusCode::OK => Ok(response
            .json::<WorkOrderResponse>()
            .await
            .map_err(|e| Error::BadProofOrderResponse(e.to_string()))?),
        other_status => Err(Error::BadProofOrderRequest(other_status.to_string())),
    }
}

async fn poll_for_result(client: &Client, query_id: &str) -> Result<Vec<u8>> {
    let url = format!(
        "http://{PROVER_BE_SOCKET_ADDR}/AzureAppService/GetProverOutput/prover-output/{query_id}",
    );

    let timeout = Duration::from_secs(15 * 60); // 15 minutes
    let interval = Duration::from_secs(30); // Poll every 10 seconds
    let start = tokio::time::Instant::now();

    while start.elapsed() < timeout {
        let response = client.get(&url).send().await?;

        let proof_bytes = response.bytes().await?;

        if !proof_bytes.is_empty() {
            return Ok(proof_bytes.into());
        }

        info!("Result not yet available, waiting to retry...");
        sleep(interval).await;
    }

    Err(anyhow::anyhow!(
        "Timeout reached: Result not available within 15 minutes"
    ))
}

async fn _get_work_order_status(client: &Client, request_id: &str) -> Result<WorkOrderResponse> {
    let url = format!("http://{PROVER_BE_SOCKET_ADDR}/AzureAppService/GetRequestStatusById/request-status/{request_id}");

    let response = client
        .get(&url)
        .send()
        .await?
        .json::<WorkOrderResponse>()
        .await?;

    Ok(response)
}

async fn _get_work_order_result(client: &Client, query_id: &str) -> Result<WorkOrderResult> {
    let url = format!(
        "http://{PROVER_BE_SOCKET_ADDR}/AzureAppService/GetProverOutput/prover-output/{query_id}"
    );

    let response = client
        .get(&url)
        .send()
        .await?
        .json::<WorkOrderResult>()
        .await?;

    Ok(response)
}
