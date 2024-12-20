use super::QueryId;
use anyhow::{anyhow, Result};
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
struct OrderStatusResponse {
    request_status: String,
    pipeline_id: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
struct PipelineStatusResponse {
    run_id: String,
    status: String,
    message: String,
    start_time: String,
    end_time: String,
    duration_in_ms: String,
}

#[derive(Debug, Error)]
enum Error {
    #[error("The following file has no request field mapping: {0}")]
    BadProofInputFile(String),
    #[error("Sending request failed. Check that `prover-be-socket-addr` argument is provided and correct. Error: {0}")]
    ReqwestSendError(String),
    #[error("BadProofOrderRequest. Error: {0}")]
    BadProofOrderRequest(String),
    #[error("Couldn't parse work order response. Error: {0}")]
    BadProofOrderResponse(String),
    #[error("Timeout reached: Result not available within 60 minutes")]
    ProvingPipelineTimeout,
    #[error("Proof generation failed")]
    ProofGenerationFailed,
    #[error("Bad proof result request. StatusCode: {0}")]
    BadProofResultRequest(String),
    #[error("Bad proof result response. Error: {0}")]
    BadProofResultResponse(String),
}

/// Handle proof order
pub async fn handle_proof_order(
    query_id: QueryId,
    files: Vec<PathBuf>,
    prover_be_socket_addr: &str,
) -> Result<Proof> {
    info!("Handling external proof order");
    let client = Client::new();

    let timeout = Duration::from_secs(1 * 15); // 15 seconds
    let interval = Duration::from_secs(1); // Poll every 1 second
    let start = tokio::time::Instant::now();

    // Repeat proof order if there are connectivity issues.
    let response: WorkOrderResponse;
    loop {
        if start.elapsed() > timeout {
            return Err(anyhow!("Proof order request timeout."));
        }

        // Must do this in loop since reqwest::multipart::Form can't be cloned.
        let form = prepare_proof_order_form(query_id, &files).await?;

        // Attempt to post work order
        if let Some(incoming_response) =
            post_work_order(&client, form, prover_be_socket_addr).await?
        {
            info!(
                "Posted proving work order with request_id: {}",
                incoming_response.request_id
            );
            response = incoming_response;
            break;
        } else {
            // No response yet. We'll try again in a moment
            info!(
                "Sending work order failed... trying again. Elapsed: {:?}",
                start.elapsed().as_secs(),
            );
            sleep(interval).await;
        }
    }

    // Poll for the result
    let proof_bytes = poll_for_result(&client, &response.query_id, prover_be_socket_addr).await?;
    info!("Work order proof len: {}", proof_bytes.len());
    Ok(proof_bytes)
}

async fn post_work_order(
    client: &Client,
    form: reqwest::multipart::Form,
    prover_be_socket_addr: &str,
) -> std::result::Result<Option<WorkOrderResponse>, Error> {
    let url = format!(
        "http://{prover_be_socket_addr}/AzureAppService/QueueLightProverQueryRequest/prove"
    );
    let response = client
        .post(&url)
        .header(ACCEPT, "*/*")
        .multipart(form)
        .send()
        .await
        .map_err(|e| Error::ReqwestSendError(e.to_string()))?;

    match response.status() {
        reqwest::StatusCode::OK => {
            return Ok(Some(
                response
                    .json::<WorkOrderResponse>()
                    .await
                    .map_err(|e| Error::BadProofOrderResponse(e.to_string()))?,
            ));
        }
        other_status if other_status.is_client_error() => {
            return Err(Error::BadProofOrderRequest(other_status.to_string()));
        }
        _ => {
            // The status isn't a client error, but isn't OK. We'll try again in a moment.
            Ok(None)
        }
    }
}

async fn poll_for_result(
    client: &Client,
    query_id: &str,
    prover_be_socket_addr: &str,
) -> std::result::Result<Vec<u8>, Error> {
    let url = format!(
        "http://{prover_be_socket_addr}/AzureAppService/GetProverOutput/prover-output/{query_id}",
    );

    let timeout = Duration::from_secs(60 * 60); // 60 minutes
    let interval = Duration::from_secs(30); // Poll every 30 seconds
    let start = tokio::time::Instant::now();

    while start.elapsed() < timeout {
        // If response is Ok but no proof is supplied, then pipeline is still in progress.
        if let Some(proof) = get_work_order_result(client, &url).await? {
            return Ok(proof);
        }

        info!(
            "Result not yet available... Elapsed: {:?}, Timeout: {:?}",
            start.elapsed().as_secs(),
            timeout.as_secs()
        );
        sleep(interval).await;
    }

    Err(Error::ProvingPipelineTimeout)
}

async fn _get_work_order_status(
    client: &Client,
    request_id: &str,
    prover_be_socket_addr: &str,
) -> Result<OrderStatusResponse> {
    let url = format!("http://{prover_be_socket_addr}/AzureAppService/GetRequestStatusById/request-status/{request_id}");

    let response = client
        .get(&url)
        .send()
        .await?
        .json::<OrderStatusResponse>()
        .await?;

    Ok(response)
}

async fn _get_pipeline_status(
    client: &Client,
    pipeline_id: &str,
    prover_be_socket_addr: &str,
) -> Result<PipelineStatusResponse> {
    let url = format!("http://{prover_be_socket_addr}/AzureAppService/GetPipelineRunStatus/pipeline-status/{pipeline_id}");

    let response = client
        .get(&url)
        .send()
        .await?
        .json::<PipelineStatusResponse>()
        .await?;

    Ok(response)
}

async fn get_work_order_result(
    client: &Client,
    url: &str,
) -> std::result::Result<Option<Vec<u8>>, Error> {
    let response = client
        .get(url)
        .header(ACCEPT, "*/*")
        .send()
        .await
        .map_err(|e| Error::ReqwestSendError(e.to_string()))?;

    match response.status() {
        reqwest::StatusCode::OK => {
            return Ok(Some(
                response
                    .bytes()
                    .await
                    .map_err(|e| Error::BadProofResultResponse(e.to_string()))?
                    .into(),
            ));
        }
        // Result not available yet. Pipeline still in progress.
        reqwest::StatusCode::BAD_REQUEST => Ok(None),
        reqwest::StatusCode::NOT_FOUND => {
            return Err(Error::ProofGenerationFailed);
        }
        other_status => {
            return Err(Error::BadProofResultRequest(other_status.to_string()));
        }
    }
}

async fn prepare_proof_order_form(
    query_id: QueryId,
    files: &Vec<PathBuf>,
) -> Result<reqwest::multipart::Form> {
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
    info!("Posting work order with query_id: {}", uuid_string);

    // Add query id to the form
    form = form.text("queryId", uuid_string);
    Ok(form)
}
